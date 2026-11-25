//! Cheap SQL safety checks for destructive statements.
//!
//! This is intentionally not a full parser. It catches the common direct
//! forms and known executable-comment form, while the confirmation dialog is
//! the safety net.

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
pub(crate) fn transaction_state_after(current: bool, sql: &str) -> bool {
    let cleaned = strip_comments_and_literals(sql);
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
                let compact = statement.replace(' ', "");
                if compact.contains("autocommit=0") || compact.contains("autocommit0") {
                    open = true;
                } else if compact.contains("autocommit=1") || compact.contains("autocommit1") {
                    open = false;
                }
            }
            _ => {}
        }
    }
    open
}

/// Returns the first destructive statement that needs confirmation.
pub(crate) fn sql_risk(sql: &str) -> Option<SqlRisk> {
    if let Some(risk) = executable_comment_risk(sql) {
        return Some(risk);
    }
    let cleaned = strip_comments_and_literals(sql);
    for statement in cleaned.split(';') {
        let mut depth = 0usize;
        let mut word = String::new();
        let mut deletes: Vec<DeleteState> = Vec::new();
        let mut updates: Vec<UpdateState> = Vec::new();
        let mut saw_insert = false;

        for ch in statement.chars().chain(std::iter::once(' ')) {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                word.push(ch.to_ascii_lowercase());
                continue;
            }
            if !word.is_empty() {
                match word.as_str() {
                    "delete" => deletes.push(DeleteState {
                        depth,
                        saw_from: false,
                        saw_where: false,
                    }),
                    "insert" if depth == 0 => saw_insert = true,
                    "update" if !saw_insert => updates.push(UpdateState {
                        depth,
                        saw_where: false,
                    }),
                    "from" => {
                        for delete in deletes.iter_mut().filter(|delete| delete.depth == depth) {
                            delete.saw_from = true;
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
                    "do" if depth == 0 && !saw_insert => {
                        return Some(SqlRisk::DynamicSql);
                    }
                    _ => {}
                }
                word.clear();
            }
            match ch {
                '(' => depth += 1,
                ')' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }

        if deletes
            .iter()
            .any(|delete| delete.saw_from && !delete.saw_where)
        {
            return Some(SqlRisk::UnfilteredDelete);
        }
        if updates.iter().any(|update| !update.saw_where) {
            return Some(SqlRisk::UnfilteredUpdate);
        }
    }
    None
}

#[derive(Clone, Copy)]
struct DeleteState {
    depth: usize,
    saw_from: bool,
    saw_where: bool,
}

#[derive(Clone, Copy)]
struct UpdateState {
    depth: usize,
    saw_where: bool,
}

/// MySQL executes `/*! ... */` as code, so inspect those bodies separately
/// instead of treating them as comments.
fn executable_comment_risk(sql: &str) -> Option<SqlRisk> {
    let mut rest = sql;
    while let Some(start) = rest.find("/*!") {
        let body_start = start + 3;
        let body = &rest[body_start..];
        let end = body.find("*/")?;
        if let Some(risk) = sql_risk(&body[..end]) {
            return Some(risk);
        }
        rest = &body[end + 2..];
    }
    None
}

/// Keep ASCII words and statement structure; replace strings, identifiers, and
/// ordinary comments with spaces so keywords inside them cannot trigger the
/// guard.
fn strip_comments_and_literals(sql: &str) -> String {
    let bytes = sql.as_bytes();
    let mut out = String::with_capacity(sql.len());
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                out.push('\n');
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                let mut depth = 1usize;
                while i < bytes.len() && depth > 0 {
                    if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
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
            b'#' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                out.push('\n');
            }
            quote @ (b'\'' | b'"' | b'`') => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i = (i + 2).min(bytes.len());
                    } else if bytes[i] == quote {
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
            b'[' => {
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
            b'$' => {
                if let Some((tag, after_tag)) = dollar_quote_tag(bytes, i) {
                    let closing = format!("${}$", tag);
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
    let tag = std::str::from_utf8(&bytes[start + 1..end]).ok()?;
    Some((tag, end + 1))
}

#[cfg(test)]
mod tests {
    use super::{SqlRisk, sql_risk};

    #[test]
    fn detects_plain_delete() {
        assert_eq!(
            sql_risk("DELETE FROM users;"),
            Some(SqlRisk::UnfilteredDelete)
        );
        assert_eq!(
            sql_risk("delete from users"),
            Some(SqlRisk::UnfilteredDelete)
        );
        assert_eq!(
            sql_risk("SELECT 1; DELETE FROM users;"),
            Some(SqlRisk::UnfilteredDelete)
        );
        assert_eq!(
            sql_risk("WITH x AS (SELECT 1) DELETE FROM users;"),
            Some(SqlRisk::UnfilteredDelete)
        );
        assert_eq!(
            sql_risk("DELETE FROM users RETURNING *;"),
            Some(SqlRisk::UnfilteredDelete)
        );
    }

    #[test]
    fn detects_cte_and_executable_comment_deletes() {
        assert_eq!(
            sql_risk("WITH x AS (DELETE FROM users RETURNING *) SELECT * FROM x;"),
            Some(SqlRisk::UnfilteredDelete)
        );
        assert_eq!(
            sql_risk("/*!40101 DELETE FROM users */"),
            Some(SqlRisk::UnfilteredDelete)
        );
    }

    #[test]
    fn detects_unfiltered_update_and_ddl() {
        assert_eq!(
            sql_risk("UPDATE users SET active = false;"),
            Some(SqlRisk::UnfilteredUpdate)
        );
        assert_eq!(
            sql_risk("UPDATE users SET active = false WHERE id = 1;"),
            None
        );
        assert_eq!(
            sql_risk("TRUNCATE TABLE users;"),
            Some(SqlRisk::DestructiveDdl)
        );
        assert_eq!(sql_risk("DROP TABLE users;"), Some(SqlRisk::DestructiveDdl));
        assert_eq!(
            sql_risk("EXEC('DELETE FROM users')"),
            Some(SqlRisk::DynamicSql)
        );
        assert_eq!(
            sql_risk("DO $$ BEGIN DELETE FROM users; END $$;"),
            Some(SqlRisk::DynamicSql)
        );
    }

    #[test]
    fn ignores_filtered_deletes_and_non_commands() {
        assert_eq!(sql_risk("DELETE FROM users WHERE id = 1;"), None);
        assert_eq!(
            sql_risk("DELETE FROM users WHERE id IN (SELECT id FROM x);"),
            None
        );
        assert_eq!(
            sql_risk("INSERT INTO users(id) VALUES (1) ON CONFLICT DO NOTHING;"),
            None
        );
        assert_eq!(
            sql_risk("INSERT INTO users(id) VALUES (1) ON CONFLICT (id) DO UPDATE SET id = 1;"),
            None
        );
    }

    #[test]
    fn ignores_keywords_in_strings_comments_and_identifiers() {
        assert_eq!(sql_risk("-- DELETE FROM users"), None);
        assert_eq!(sql_risk("/* DELETE FROM users */ SELECT 1"), None);
        assert_eq!(sql_risk("SELECT 'DELETE FROM users'"), None);
        assert_eq!(sql_risk("SELECT \"delete\" FROM users"), None);
        assert_eq!(sql_risk("SELECT [delete] FROM users"), None);
    }

    #[test]
    fn comments_cannot_hide_a_missing_where() {
        assert_eq!(
            sql_risk("DELETE FROM users -- WHERE id = 1"),
            Some(SqlRisk::UnfilteredDelete)
        );
        assert_eq!(
            sql_risk("DELETE /* WHERE id = 1 */ FROM users;"),
            Some(SqlRisk::UnfilteredDelete)
        );
    }

    #[test]
    fn where_inside_using_subquery_does_not_count() {
        assert_eq!(
            sql_risk("DELETE FROM users USING (SELECT id FROM admins WHERE active) a;"),
            Some(SqlRisk::UnfilteredDelete)
        );
    }

    #[test]
    fn tracks_transaction_boundaries() {
        use super::transaction_state_after;
        assert!(transaction_state_after(false, "BEGIN;"));
        assert!(transaction_state_after(false, "START TRANSACTION;"));
        assert!(!transaction_state_after(true, "COMMIT;"));
        assert!(!transaction_state_after(true, "ROLLBACK;"));
        assert!(transaction_state_after(true, "ROLLBACK TO SAVEPOINT s1;"));
        assert!(!transaction_state_after(true, "END;"));
        assert!(transaction_state_after(true, "SELECT 1;"));
        assert!(transaction_state_after(false, "SET autocommit=0;"));
        assert!(!transaction_state_after(true, "SET autocommit = 1;"));
    }

    #[test]
    fn other_statements_are_ignored() {
        assert_eq!(sql_risk("SELECT 1;"), None);
        assert_eq!(sql_risk(""), None);
    }
}
