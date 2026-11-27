use std::ops::Range;

pub(crate) struct EditorEdit {
    pub(crate) text: String,
    pub(crate) selection: Range<usize>,
}

fn line_index_at(text: &str, offset: usize) -> usize {
    text.as_bytes()[..offset.min(text.len())]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count()
}

fn selected_lines(text: &str, selection: &Range<usize>) -> (usize, usize) {
    let first = line_index_at(text, selection.start);
    let probe = if selection.end > selection.start {
        selection.end.saturating_sub(1)
    } else {
        selection.start
    };
    (first, line_index_at(text, probe) + 1)
}

fn lines_of(text: &str) -> Vec<String> {
    text.split('\n').map(str::to_string).collect()
}

fn line_start(lines: &[String], index: usize) -> usize {
    lines[..index.min(lines.len())]
        .iter()
        .enumerate()
        .map(|(index, line)| line.len() + usize::from(index + 1 < lines.len()))
        .sum()
}

fn block_span(lines: &[String], first: usize, last: usize) -> Range<usize> {
    let start = line_start(lines, first);
    let content = lines[first..last].iter().map(String::len).sum::<usize>();
    let separators = last.saturating_sub(first).saturating_sub(1);
    start..start + content + separators
}

fn caret_in_line(text: &str, offset: usize) -> usize {
    let offset = text.floor_char_boundary(offset.min(text.len()));
    let start = text[..offset]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    offset.saturating_sub(start)
}

fn cursor_after(lines: &[String], first: usize, column: usize) -> usize {
    let start = line_start(lines, first);
    let clamped = lines
        .get(first)
        .map(|line| line.floor_char_boundary(column.min(line.len())))
        .unwrap_or(0);
    start + clamped
}

fn commented(line: &str) -> bool {
    line.trim_start().starts_with("--")
}

pub(crate) fn toggle_comment(text: &str, selection: Range<usize>) -> EditorEdit {
    let mut lines = lines_of(text);
    let (first, last) = selected_lines(text, &selection);
    if first >= last || first >= lines.len() {
        return EditorEdit {
            text: text.into(),
            selection,
        };
    }
    let selected = &lines[first..last];
    let non_empty = selected
        .iter()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();
    // An empty line must get a comment marker; vacuous `all` would pick
    // uncomment and leave the line unchanged.
    let uncomment = !non_empty.is_empty() && non_empty.iter().all(|line| commented(line));
    let first_line = lines[first].clone();
    let first_indent = first_line.len() - first_line.trim_start().len();
    let first_commented = commented(&first_line);
    let removed_prefix = if uncomment && first_commented {
        if first_line[first_indent + 2..].starts_with(' ') {
            3
        } else {
            2
        }
    } else {
        0
    };
    for line in &mut lines[first..last] {
        let indent = line.len() - line.trim_start().len();
        if uncomment {
            if line[indent..].starts_with("--") {
                let mut rest = &line[indent + 2..];
                if let Some(stripped) = rest.strip_prefix(' ') {
                    rest = stripped;
                }
                *line = format!("{}{}", &line[..indent], rest);
            }
        } else {
            line.insert_str(indent, "-- ");
        }
    }
    let block = block_span(&lines, first, last);
    let selection = if selection.is_empty() {
        let column = caret_in_line(text, selection.start);
        let column = if uncomment {
            if removed_prefix > 0 && column >= first_indent + removed_prefix {
                column - removed_prefix
            } else {
                column.min(first_indent)
            }
        } else if column >= first_indent {
            column + 3
        } else {
            column
        };
        let cursor = cursor_after(&lines, first, column);
        cursor..cursor
    } else {
        block
    };
    EditorEdit {
        text: lines.join("\n"),
        selection,
    }
}

fn move_lines(text: &str, selection: Range<usize>, up: bool) -> EditorEdit {
    let mut lines = lines_of(text);
    let (first, last) = selected_lines(text, &selection);
    if first >= last || first >= lines.len() {
        return EditorEdit {
            text: text.into(),
            selection,
        };
    }
    let column = caret_in_line(text, selection.start);
    let block = lines[first..last].to_vec();
    let new_first = if up {
        if first == 0 {
            return EditorEdit {
                text: text.into(),
                selection,
            };
        }
        lines.drain(first..last);
        lines.splice(first - 1..first - 1, block);
        first - 1
    } else {
        if last >= lines.len() {
            return EditorEdit {
                text: text.into(),
                selection,
            };
        }
        lines.drain(first..last);
        lines.splice(first + 1..first + 1, block);
        first + 1
    };
    let selection = if selection.is_empty() {
        let cursor = cursor_after(&lines, new_first, column);
        cursor..cursor
    } else {
        block_span(&lines, new_first, new_first + last - first)
    };
    EditorEdit {
        text: lines.join("\n"),
        selection,
    }
}

pub(crate) fn move_line_up(text: &str, selection: Range<usize>) -> EditorEdit {
    move_lines(text, selection, true)
}

pub(crate) fn move_line_down(text: &str, selection: Range<usize>) -> EditorEdit {
    move_lines(text, selection, false)
}

pub(crate) fn duplicate_line(text: &str, selection: Range<usize>) -> EditorEdit {
    let mut lines = lines_of(text);
    let (first, last) = selected_lines(text, &selection);
    if first >= last || first >= lines.len() {
        return EditorEdit {
            text: text.into(),
            selection,
        };
    }
    let column = caret_in_line(text, selection.start);
    let count = last - first;
    let block = lines[first..last].to_vec();
    lines.splice(last..last, block);
    let selection = if selection.is_empty() {
        let cursor = cursor_after(&lines, last, column);
        cursor..cursor
    } else {
        block_span(&lines, last, last + count)
    };
    EditorEdit {
        text: lines.join("\n"),
        selection,
    }
}

pub(crate) fn delete_line(text: &str, selection: Range<usize>) -> EditorEdit {
    let mut lines = lines_of(text);
    let (first, last) = selected_lines(text, &selection);
    if first >= last || first >= lines.len() {
        return EditorEdit {
            text: text.into(),
            selection,
        };
    }
    lines.drain(first..last);
    let cursor = line_start(&lines, first);
    EditorEdit {
        text: lines.join("\n"),
        selection: cursor..cursor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_and_uncomments_selected_lines() {
        let text = "SELECT 1;\n  SELECT 2;\n";
        let edit = toggle_comment(text, 0..text.len());
        assert_eq!(edit.text, "-- SELECT 1;\n  -- SELECT 2;\n");
        let edit = toggle_comment(&edit.text, 0..edit.text.len());
        assert_eq!(edit.text, "SELECT 1;\n  SELECT 2;\n");
    }

    #[test]
    fn comments_an_empty_line_or_empty_document() {
        let edit = toggle_comment("", 0..0);
        assert_eq!(edit.text, "-- ");
        assert_eq!(edit.selection, 3..3);

        let text = "one\n\ntwo";
        let edit = toggle_comment(text, 4..4);
        assert_eq!(edit.text, "one\n-- \ntwo");
        assert_eq!(edit.selection, 7..7);
    }

    #[test]
    fn moves_selected_block_up_and_down() {
        let text = "one\ntwo\nthree\n";
        let edit = move_line_up(text, 4..7);
        assert_eq!(edit.text, "two\none\nthree\n");
        assert_eq!(edit.selection, 0..3);
        let edit = move_line_down(text, 0..3);
        assert_eq!(edit.text, "two\none\nthree\n");
        assert_eq!(edit.selection, 4..7);
    }

    #[test]
    fn duplicates_and_deletes_lines() {
        let text = "one\ntwo\n";
        let edit = duplicate_line(text, 0..3);
        assert_eq!(edit.text, "one\none\ntwo\n");
        assert_eq!(edit.selection, 4..7);
        let edit = delete_line(&edit.text, 4..7);
        assert_eq!(edit.text, "one\ntwo\n");
        assert_eq!(edit.selection, 4..4);
    }

    #[test]
    fn keeps_cursor_column_when_moving_last_line() {
        let text = "one\ntwo";
        let edit = move_line_up(text, 6..6);
        assert_eq!(edit.text, "two\none");
        assert_eq!(edit.selection, 2..2);
    }

    #[test]
    fn handles_multibyte_utf8_selections_without_panicking() {
        let text = "SELECT 'café'\nSELECT '日本語'";
        let first_end = "SELECT 'café'".len();
        let edit = toggle_comment(text, 0..first_end);
        assert_eq!(edit.text, "-- SELECT 'café'\nSELECT '日本語'");
        assert!(edit.text.is_char_boundary(edit.selection.start));
        assert!(edit.text.is_char_boundary(edit.selection.end));

        let second_start = first_end + 1;
        let moved = move_line_up(text, second_start..text.len());
        assert_eq!(moved.text, "SELECT '日本語'\nSELECT 'café'");

        let dup = duplicate_line(text, 0..first_end);
        assert_eq!(dup.text, "SELECT 'café'\nSELECT 'café'\nSELECT '日本語'");

        let del = delete_line(text, second_start..text.len());
        assert_eq!(del.text, "SELECT 'café'");
    }
}
