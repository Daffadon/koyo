//! Cheap SQL safety checks for destructive statements.
//!
//! This is intentionally not a full parser. It catches the common direct
//! forms and engine-specific comment/quoting edge cases, while the
//! confirmation dialog is the safety net.

use koyo_core::models::Engine;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SqlRisk {
    UnfilteredDelete,
    UnfilteredUpdate,
    DestructiveDdl,
    DynamicSql,
}

/// Advance the app's transaction warning from the SQL text. The database
/// engine remains authoritative, but this catches BEGIN/COMMIT/ROLLBACK and
/// MySQL autocommit toggles without an extra round trip.
pub(crate) fn transaction_state_after(current: bool, sql: &str, engine: Engine) -> bool {
    let cleaned = strip_comments_and_literals(sql, engine);
    let mut open = current;
    for statement in cleaned.split(';') {
        let words = statement.split_ascii_whitespace().collect::<Vec<_>>();
        let Some(first) = words.first().copied() else {
            continue;
        };
        match first {
            "begin" => open = true,
            "start" if words.get(1) == Some(&"transaction") => open = true,
            "rollback" if words.get(1) == Some(&"to") => {}
            "commit" | "rollback" | "end" | "abort" => open = false,
            "set" => {
                if words.contains(&"global") {
                    continue;
                }
                if let Some(position) = words.iter().position(|word| *word == "autocommit") {
                    match words.get(position + 1).copied() {
                        Some("0" | "off" | "false") => open = true,
                        Some("1" | "on" | "true") => open = false,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    open
}

/// Returns the first destructive statement that needs confirmation.
pub(crate) fn sql_risk(sql: &str, engine: Engine) -> Option<SqlRisk> {
    let cleaned = strip_comments_and_literals(sql, engine);
    for statement in cleaned.split(';') {
        let mut depth = 0usize;
        let mut word = String::new();
        let mut prev_word = String::new();
        let mut deletes: Vec<DeleteState> = Vec::new();
        let mut updates: Vec<UpdateState> = Vec::new();
        let mut in_insert = false;
        let mut allow_upsert_update = false;

        for ch in statement.chars().chain(std::iter::once(' ')) {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                word.push(ch.to_ascii_lowercase());
                continue;
            }
            if !word.is_empty() {
                // `GRANT SELECT, DELETE ON ...` / `GRANT SELECT, UPDATE ON ...`
                if word == "on" {
                    if prev_word == "delete" {
                        deletes.pop();
                    } else if prev_word == "update" {
                        updates.pop();
                    }
                }
                match word.as_str() {
                    "delete" => {
                        let non_dml = matches!(
                            prev_word.as_str(),
                            "on" | "before" | "after" | "of" | "grant" | "revoke" | "or"
                        );
                        if !non_dml {
                            if depth == 0 {
                                in_insert = false;
                                allow_upsert_update = false;
                            }
                            deletes.push(DeleteState {
                                depth,
                                saw_where: false,
                            });
                        }
                    }
                    "insert" if depth == 0 => {
                        in_insert = true;
                        allow_upsert_update = false;
                    }
                    "conflict" | "duplicate" if in_insert => {
                        allow_upsert_update = true;
                    }
                    "update" => {
                        let non_dml = matches!(
                            prev_word.as_str(),
                            "on" | "before" | "after" | "of" | "grant" | "revoke" | "or" | "for"
                        );
                        if allow_upsert_update {
                            allow_upsert_update = false;
                            in_insert = false;
                        } else if !non_dml {
                            if depth == 0 {
                                in_insert = false;
                            }
                            updates.push(UpdateState {
                                depth,
                                saw_where: false,
                            });
                        }
                    }
                    "where" => {
                        for delete in deletes.iter_mut().filter(|delete| delete.depth == depth) {
                            delete.saw_where = true;
                        }
                        for update in updates.iter_mut().filter(|update| update.depth == depth) {
                            update.saw_where = true;
                        }
                    }
                    "truncate" | "drop" => return Some(SqlRisk::DestructiveDdl),
                    "exec" | "execute" if depth == 0 => {
                        return Some(SqlRisk::DynamicSql);
                    }
                    "do" if depth == 0 && !in_insert => {
                        return Some(SqlRisk::DynamicSql);
                    }
                    _ => {}
                }
                prev_word.clear();
                prev_word.push_str(&word);
                word.clear();
            }
            match ch {
                '(' => depth += 1,
                ')' => {
                    if let Some(risk) = evaluate_depth(&mut deletes, &mut updates, depth) {
                        return Some(risk);
                    }
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
        }

        if deletes.iter().any(|delete| !delete.saw_where) {
            return Some(SqlRisk::UnfilteredDelete);
        }
        if updates.iter().any(|update| !update.saw_where) {
            return Some(SqlRisk::UnfilteredUpdate);
        }
    }
    None
}

fn evaluate_depth(
    deletes: &mut Vec<DeleteState>,
    updates: &mut Vec<UpdateState>,
    depth: usize,
) -> Option<SqlRisk> {
    let mut risk = None;
    deletes.retain(|delete| {
        if delete.depth == depth {
            if !delete.saw_where && risk.is_none() {
                risk = Some(SqlRisk::UnfilteredDelete);
            }
            false
        } else {
            true
        }
    });
    if risk.is_some() {
        return risk;
    }
    updates.retain(|update| {
        if update.depth == depth {
            if !update.saw_where && risk.is_none() {
                risk = Some(SqlRisk::UnfilteredUpdate);
            }
            false
        } else {
            true
        }
    });
    risk
}

#[derive(Clone, Copy)]
struct DeleteState {
    depth: usize,
    saw_where: bool,
}

#[derive(Clone, Copy)]
struct UpdateState {
    depth: usize,
    saw_where: bool,
}

/// Keep ASCII words and statement structure; replace strings, identifiers, and
/// ordinary comments with spaces so keywords inside them cannot trigger the
/// guard. Dialect rules prevent comment or escape mismatches across engines.
fn strip_comments_and_literals(sql: &str, engine: Engine) -> String {
    let bytes = sql.as_bytes();
    let mut out = String::with_capacity(sql.len());
    let mut i = 0;
    let mut exec_comment_depth = 0usize;
    let is_mysql = matches!(engine, Engine::Mysql | Engine::Mariadb);
    let is_postgres = engine == Engine::Postgres;
    let bracket_idents = matches!(engine, Engine::SqlServer | Engine::Sqlite);
    let backtick_idents = matches!(engine, Engine::Mysql | Engine::Mariadb | Engine::Sqlite);

    while i < bytes.len() {
        match bytes[i] {
            b'-' if bytes.get(i + 1) == Some(&b'-')
                && (!is_mysql || bytes.get(i + 2).is_some_and(u8::is_ascii_whitespace)) =>
            {
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                out.push('\n');
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                let mariadb_exec_comment = engine == Engine::Mariadb
                    && bytes.get(i + 2) == Some(&b'M')
                    && bytes.get(i + 3) == Some(&b'!');
                if is_mysql && (bytes.get(i + 2) == Some(&b'!') || mariadb_exec_comment) {
                    // MySQL executes `/*!40101 ... */`; MariaDB also executes `/*M!100100 ... */`.
                    i += if mariadb_exec_comment { 4 } else { 3 };
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                    exec_comment_depth += 1;
                    out.push(' ');
                } else {
                    i += 2;
                    let mut depth = 1usize;
                    while i < bytes.len() && depth > 0 {
                        if is_postgres && bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
                            depth += 1;
                            i += 2;
                        } else if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                            depth -= 1;
                            i += 2;
                        } else {
                            i += 1;
                        }
                    }
                    out.push(' ');
                }
            }
            b'*' if exec_comment_depth > 0 && bytes.get(i + 1) == Some(&b'/') => {
                exec_comment_depth -= 1;
                i += 2;
                out.push(' ');
            }
            b'#' if is_mysql => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                out.push('\n');
            }
            b'e' | b'E'
                if is_postgres
                    && bytes.get(i + 1) == Some(&b'\'')
                    && (i == 0
                        || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_')) =>
            {
                i += 2;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i = (i + 2).min(bytes.len());
                    } else if bytes[i] == b'\'' {
                        if bytes.get(i + 1) == Some(&b'\'') {
                            i += 2;
                        } else {
                            i += 1;
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
                out.push(' ');
            }
            quote @ (b'\'' | b'"') => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == quote {
                        if bytes.get(i + 1) == Some(&quote) {
                            i += 2;
                        } else {
                            i += 1;
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
                out.push(' ');
            }
            b'`' if backtick_idents => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'`' {
                        if bytes.get(i + 1) == Some(&b'`') {
                            i += 2;
                        } else {
                            i += 1;
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
                out.push(' ');
            }
            b'[' if bracket_idents => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b']' {
                        if bytes.get(i + 1) == Some(&b']') {
                            i += 2;
                        } else {
                            i += 1;
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
                out.push(' ');
            }
            b'$' if is_postgres
                && (i == 0
                    || (bytes[i - 1].is_ascii()
                        && !bytes[i - 1].is_ascii_alphanumeric()
                        && bytes[i - 1] != b'_'
                        && bytes[i - 1] != b'$')) =>
            {
                if let Some((tag, after_tag)) = dollar_quote_tag(bytes, i) {
                    let closing = format!("${tag}$");
                    if let Some(relative_end) = sql[after_tag..].find(&closing) {
                        i = after_tag + relative_end + closing.len();
                        out.push(' ');
                    } else {
                        i += 1;
                        out.push(' ');
                    }
                } else {
                    i += 1;
                    out.push(' ');
                }
            }
            b';' | b'(' | b')' | b'\n' | b'\r' | b'\t' | b' ' => {
                out.push(bytes[i] as char);
                i += 1;
            }
            byte if byte.is_ascii_alphanumeric() || byte == b'_' => {
                out.push(byte.to_ascii_lowercase() as char);
                i += 1;
            }
            _ => {
                i += 1;
                out.push(' ');
            }
        }
    }

    out
}

fn dollar_quote_tag(bytes: &[u8], start: usize) -> Option<(&str, usize)> {
    let mut end = start + 1;
    while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
        end += 1;
    }
    if bytes.get(end) != Some(&b'$') {
        return None;
    }
    if end > start + 1
        && !(bytes[start + 1].is_ascii_alphabetic()
            || bytes[start + 1] == b'_'
            || !bytes[start + 1].is_ascii())
    {
        return None;
    }
    let tag = std::str::from_utf8(&bytes[start + 1..end]).ok()?;
    Some((tag, end + 1))
}

#[cfg(test)]
mod tests {
    use super::{SqlRisk, sql_risk, transaction_state_after};
    use koyo_core::models::Engine;

    #[test]
    fn detects_plain_delete() {
        assert_eq!(
            sql_risk("DELETE FROM users;", Engine::Postgres),
            Some(SqlRisk::UnfilteredDelete)
        );
        assert_eq!(
            sql_risk("delete from users", Engine::Postgres),
            Some(SqlRisk::UnfilteredDelete)
        );
        assert_eq!(
            sql_risk("SELECT 1; DELETE FROM users;", Engine::Postgres),
            Some(SqlRisk::UnfilteredDelete)
        );
        assert_eq!(
            sql_risk("WITH x AS (SELECT 1) DELETE FROM users;", Engine::Postgres),
            Some(SqlRisk::UnfilteredDelete)
        );
        assert_eq!(
            sql_risk("DELETE FROM users RETURNING *;", Engine::Postgres),
            Some(SqlRisk::UnfilteredDelete)
        );
        assert_eq!(
            sql_risk("DELETE users;", Engine::SqlServer),
            Some(SqlRisk::UnfilteredDelete)
        );
    }

    #[test]
    fn detects_cte_and_executable_comment_deletes() {
        assert_eq!(
            sql_risk(
                "WITH x AS (DELETE FROM users RETURNING *) SELECT * FROM x;",
                Engine::Postgres
            ),
            Some(SqlRisk::UnfilteredDelete)
        );
        assert_eq!(
            sql_risk("/*!40101 DELETE FROM users */", Engine::Mysql),
            Some(SqlRisk::UnfilteredDelete)
        );
        assert_eq!(
            sql_risk("DELETE /*!40101 FROM users */", Engine::Mysql),
            Some(SqlRisk::UnfilteredDelete)
        );
        assert_eq!(
            sql_risk(
                "WITH a AS (DELETE FROM users RETURNING *), b AS (SELECT 1 FROM t WHERE true) SELECT * FROM a;",
                Engine::Postgres
            ),
            Some(SqlRisk::UnfilteredDelete)
        );
    }

    #[test]
    fn detects_unfiltered_update_and_ddl() {
        assert_eq!(
            sql_risk("UPDATE users SET active = false;", Engine::Postgres),
            Some(SqlRisk::UnfilteredUpdate)
        );
        assert_eq!(
            sql_risk(
                "UPDATE users SET active = false WHERE id = 1;",
                Engine::Postgres
            ),
            None
        );
        assert_eq!(
            sql_risk("TRUNCATE TABLE users;", Engine::Postgres),
            Some(SqlRisk::DestructiveDdl)
        );
        assert_eq!(
            sql_risk("DROP TABLE users;", Engine::Postgres),
            Some(SqlRisk::DestructiveDdl)
        );
        assert_eq!(
            sql_risk("EXEC('DELETE FROM users')", Engine::SqlServer),
            Some(SqlRisk::DynamicSql)
        );
        assert_eq!(
            sql_risk("DO $$ BEGIN DELETE FROM users; END $$;", Engine::Postgres),
            Some(SqlRisk::DynamicSql)
        );
        assert_eq!(
            sql_risk(
                "INSERT INTO a VALUES (1)\nUPDATE users SET active = 0",
                Engine::SqlServer
            ),
            Some(SqlRisk::UnfilteredUpdate)
        );
    }

    #[test]
    fn rejects_dialect_comment_and_escape_bypasses() {
        // Backslash is not an escape in PostgreSQL standard strings.
        assert_eq!(
            sql_risk("SELECT '\\'; DELETE FROM users; --'", Engine::Postgres),
            Some(SqlRisk::UnfilteredDelete)
        );
        // `#` is not a line comment in PostgreSQL.
        assert_eq!(
            sql_risk("SELECT 1 # 2; DELETE FROM users;", Engine::Postgres),
            Some(SqlRisk::UnfilteredDelete)
        );
        // Non-nested block comments in MySQL close on the first `*/`.
        assert_eq!(
            sql_risk("/* /* */ DELETE FROM users;", Engine::Mysql),
            Some(SqlRisk::UnfilteredDelete)
        );
        // MySQL requires whitespace after `--`; otherwise the dashes are operators.
        assert_eq!(
            sql_risk("SELECT 1--1; DELETE FROM users;", Engine::Mysql),
            Some(SqlRisk::UnfilteredDelete)
        );
        assert_eq!(
            sql_risk("SELECT 1-- DELETE FROM users;", Engine::Mysql),
            None
        );
        // NO_BACKSLASH_ESCAPES is configurable per connection, so do not assume
        // MySQL backslashes keep the next quote inside a string.
        assert_eq!(
            sql_risk("SELECT 'x\\'; DELETE FROM users;", Engine::Mysql),
            Some(SqlRisk::UnfilteredDelete)
        );
        assert_eq!(
            sql_risk("/*M!100100 DELETE FROM users */", Engine::Mariadb),
            Some(SqlRisk::UnfilteredDelete)
        );
        assert_eq!(
            sql_risk("SELECT arr[1]; DELETE FROM users;", Engine::Postgres),
            Some(SqlRisk::UnfilteredDelete)
        );
        assert_eq!(
            sql_risk(
                "SELECT foo$tag$bar FROM t; DELETE FROM users; SELECT $tag$abc$tag$;",
                Engine::Postgres
            ),
            Some(SqlRisk::UnfilteredDelete)
        );
        assert_eq!(
            sql_risk("SELECT $$DELETE FROM users$$", Engine::Postgres),
            None
        );
        // PostgreSQL E'...' strings do honor backslash escapes.
        assert_eq!(
            sql_risk("SELECT E'\\'; DELETE FROM users;'", Engine::Postgres),
            None
        );
    }

    #[test]
    fn ignores_filtered_deletes_and_non_commands() {
        assert_eq!(
            sql_risk("DELETE FROM users WHERE id = 1;", Engine::Postgres),
            None
        );
        assert_eq!(
            sql_risk(
                "DELETE FROM users WHERE id IN (SELECT id FROM x);",
                Engine::Postgres
            ),
            None
        );
        assert_eq!(
            sql_risk(
                "INSERT INTO users(id) VALUES (1) ON CONFLICT DO NOTHING;",
                Engine::Postgres
            ),
            None
        );
        assert_eq!(
            sql_risk(
                "INSERT INTO users(id) VALUES (1) ON CONFLICT (id) DO UPDATE SET id = 1;",
                Engine::Postgres
            ),
            None
        );
        assert_eq!(
            sql_risk(
                "INSERT INTO users(id) VALUES (1) ON DUPLICATE KEY UPDATE id = 1;",
                Engine::Mysql
            ),
            None
        );
        assert_eq!(
            sql_risk(
                "ALTER TABLE c ADD FOREIGN KEY (u) REFERENCES users(id) ON DELETE CASCADE ON UPDATE CASCADE;",
                Engine::Postgres
            ),
            None
        );
        assert_eq!(
            sql_risk("SELECT * FROM users FOR UPDATE;", Engine::Postgres),
            None
        );
        assert_eq!(
            sql_risk("GRANT SELECT, DELETE ON users TO bob;", Engine::Postgres),
            None
        );
    }

    #[test]
    fn ignores_keywords_in_strings_comments_and_identifiers() {
        assert_eq!(sql_risk("-- DELETE FROM users", Engine::Postgres), None);
        assert_eq!(
            sql_risk("/* DELETE FROM users */ SELECT 1", Engine::Postgres),
            None
        );
        assert_eq!(
            sql_risk("SELECT 'DELETE FROM users'", Engine::Postgres),
            None
        );
        assert_eq!(
            sql_risk("SELECT \"delete\" FROM users", Engine::Postgres),
            None
        );
        assert_eq!(
            sql_risk("SELECT [delete] FROM users", Engine::SqlServer),
            None
        );
    }

    #[test]
    fn comments_cannot_hide_a_missing_where() {
        assert_eq!(
            sql_risk("DELETE FROM users -- WHERE id = 1", Engine::Postgres),
            Some(SqlRisk::UnfilteredDelete)
        );
        assert_eq!(
            sql_risk("DELETE /* WHERE id = 1 */ FROM users;", Engine::Postgres),
            Some(SqlRisk::UnfilteredDelete)
        );
    }

    #[test]
    fn where_inside_using_subquery_does_not_count() {
        assert_eq!(
            sql_risk(
                "DELETE FROM users USING (SELECT id FROM admins WHERE active) a;",
                Engine::Postgres
            ),
            Some(SqlRisk::UnfilteredDelete)
        );
    }

    #[test]
    fn tracks_transaction_boundaries() {
        assert!(transaction_state_after(false, "BEGIN;", Engine::Postgres));
        assert!(transaction_state_after(
            false,
            "START TRANSACTION;",
            Engine::Mysql
        ));
        assert!(!transaction_state_after(true, "COMMIT;", Engine::Postgres));
        assert!(!transaction_state_after(
            true,
            "ROLLBACK;",
            Engine::Postgres
        ));
        assert!(transaction_state_after(
            true,
            "ROLLBACK TO SAVEPOINT s1;",
            Engine::Postgres
        ));
        assert!(!transaction_state_after(true, "END;", Engine::Postgres));
        assert!(transaction_state_after(true, "SELECT 1;", Engine::Postgres));
        assert!(transaction_state_after(
            false,
            "SET autocommit=0;",
            Engine::Mysql
        ));
        assert!(!transaction_state_after(
            true,
            "SET autocommit = 1;",
            Engine::Mysql
        ));
        assert!(transaction_state_after(
            false,
            "SET SESSION autocommit\n=\tOFF;",
            Engine::Mysql
        ));
        assert!(!transaction_state_after(
            true,
            "SET autocommit = ON;",
            Engine::Mysql
        ));
    }

    #[test]
    fn other_statements_are_ignored() {
        assert_eq!(sql_risk("SELECT 1;", Engine::Postgres), None);
        assert_eq!(sql_risk("", Engine::Postgres), None);
    }
}
