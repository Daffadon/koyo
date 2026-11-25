//! Cheap SQL safety checks for destructive statements.
//!
//! This is intentionally not a parser. It only has to avoid false negatives
//! for plain `DELETE` statements; the confirmation dialog is the safety net,
//! not the scanner.

/// Returns true when a statement deletes without a top-level `WHERE`.
pub(crate) fn delete_without_where(sql: &str) -> bool {
    let cleaned = strip_comments_and_literals(sql);
    for statement in cleaned.split(';') {
        let mut depth = 0usize;
        let mut word = String::new();
        let mut saw_delete = false;
        let mut saw_from = false;
        let mut saw_where = false;

        for ch in statement.chars().chain(std::iter::once(' ')) {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                word.push(ch.to_ascii_lowercase());
                continue;
            }
            if !word.is_empty() {
                if depth == 0 {
                    match word.as_str() {
                        "delete" if !saw_delete => saw_delete = true,
                        "from" if saw_delete => saw_from = true,
                        "where" if saw_delete => saw_where = true,
                        _ => {}
                    }
                }
                word.clear();
            }
            match ch {
                '(' => depth += 1,
                ')' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }

        if saw_delete && saw_from && !saw_where {
            return true;
        }
    }
    false
}

/// Keep ASCII words and statement structure; replace strings, identifiers, and
/// comments with spaces so keywords inside them cannot trigger the guard.
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
    use super::delete_without_where;

    #[test]
    fn detects_plain_delete() {
        assert!(delete_without_where("DELETE FROM users;"));
        assert!(delete_without_where("delete from users"));
        assert!(delete_without_where("SELECT 1; DELETE FROM users;"));
        assert!(delete_without_where(
            "WITH x AS (SELECT 1) DELETE FROM users;"
        ));
        assert!(delete_without_where("DELETE FROM users RETURNING *;"));
    }

    #[test]
    fn ignores_filtered_deletes() {
        assert!(!delete_without_where("DELETE FROM users WHERE id = 1;"));
        assert!(!delete_without_where(
            "DELETE FROM users WHERE id IN (SELECT id FROM x);"
        ));
        assert!(!delete_without_where("delete from users\nwhere id = 1"));
    }

    #[test]
    fn ignores_keywords_in_strings_comments_and_identifiers() {
        assert!(!delete_without_where("-- DELETE FROM users"));
        assert!(!delete_without_where("/* DELETE FROM users */ SELECT 1"));
        assert!(!delete_without_where("SELECT 'DELETE FROM users'"));
        assert!(!delete_without_where("SELECT \"delete\" FROM users"));
        assert!(!delete_without_where("SELECT [delete] FROM users"));
        assert!(!delete_without_where(
            "DO $$ BEGIN DELETE FROM users; END $$;"
        ));
    }

    #[test]
    fn comments_cannot_hide_a_missing_where() {
        assert!(delete_without_where("DELETE FROM users -- WHERE id = 1"));
        assert!(delete_without_where(
            "DELETE /* WHERE id = 1 */ FROM users;"
        ));
    }

    #[test]
    fn where_inside_using_subquery_does_not_count() {
        assert!(delete_without_where(
            "DELETE FROM users USING (SELECT id FROM admins WHERE active) a;"
        ));
    }

    #[test]
    fn other_statements_are_ignored() {
        assert!(!delete_without_where("UPDATE users SET active = false;"));
        assert!(!delete_without_where("TRUNCATE TABLE users;"));
        assert!(!delete_without_where(""));
    }
}
