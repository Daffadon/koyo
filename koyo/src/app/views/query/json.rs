use gpui_kit::component::v_flex;
use gpui_kit::prelude::*;
use gpui_kit::*;
use gpui_kit::{InteractiveElement, StatefulInteractiveElement};
use koyo_core::models::QueryResultSet;
use std::rc::Rc;

use super::result_grid_scroll_handle;
use crate::app::*;

const JSON_LINE_HEIGHT: f32 = 22.0;

pub(super) fn result_json(
    result: &QueryResultSet,
    tab_id: u64,
    result_index: usize,
    palette: &Palette,
    mono: &SharedString,
    _cx: &mut Context<KoyoApp>,
) -> AnyElement {
    let lines = Rc::new(json_result_lines(result));
    let scroll_handle = result_grid_scroll_handle(tab_id, result_index);
    let palette = *palette;
    let widest = lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(2);
    // monospace 14px text is roughly 8px/character; keep a comfortable gutter.
    let width = ((widest as f32) * 8.0 + 48.0).max(420.0);
    let rows = v_flex()
        .w(px(width))
        .min_w(px(width))
        .children(lines.iter().enumerate().map(|(line_index, line)| {
            div()
                .id(("result-json-line", line_index))
                .flex_shrink_0()
                .h(px(JSON_LINE_HEIGHT))
                .px_4()
                .flex()
                .items_center()
                .whitespace_nowrap()
                .text_sm()
                .text_color(palette.foreground)
                .child(line.clone())
                .into_any_element()
        }))
        .into_any_element();

    div()
        .id(("result-json-body", tab_id))
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.0))
        .overflow_scroll()
        .restrict_scroll_to_axis()
        .track_scroll(&scroll_handle)
        .child(rows)
        .font_family(mono.clone())
        .into_any_element()
}

fn json_result_lines(result: &QueryResultSet) -> Vec<String> {
    let mut lines = Vec::with_capacity(result.rows.len() + 2);
    lines.push("[".to_string());
    for (row_index, row) in result.rows.iter().enumerate() {
        let mut line = String::from("  {");
        for (column_index, column) in result.columns.iter().enumerate() {
            if column_index > 0 {
                line.push_str(", ");
            }
            push_json_string(&mut line, column);
            line.push_str(": ");
            match row.get(column_index).and_then(Option::as_deref) {
                Some(value) => push_json_string(&mut line, value),
                None => line.push_str("null"),
            }
        }
        line.push('}');
        if row_index + 1 < result.rows.len() {
            line.push(',');
        }
        lines.push(line);
    }
    lines.push("]".to_string());
    lines
}

fn push_json_string(output: &mut String, value: &str) {
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\u{08}' => output.push_str("\\b"),
            '\u{0c}' => output.push_str("\\f"),
            character if character < ' ' => {
                use std::fmt::Write as _;
                let _ = write!(output, "\\u{:04x}", character as u32);
            }
            character => output.push(character),
        }
    }
    output.push('"');
}

#[cfg(test)]
mod tests {
    use super::json_result_lines;
    use koyo_core::models::QueryResultSet;

    fn result(columns: &[&str], rows: Vec<Vec<Option<&str>>>) -> QueryResultSet {
        QueryResultSet {
            columns: columns.iter().map(|column| column.to_string()).collect(),
            rows: rows
                .into_iter()
                .map(|row| {
                    row.into_iter()
                        .map(|value| value.map(str::to_string))
                        .collect()
                })
                .collect(),
        }
    }

    #[test]
    fn json_view_is_an_array_of_row_objects() {
        let result = result(
            &["id", "note", "missing"],
            vec![
                vec![Some("1"), Some("a\"b\nline"), None],
                vec![Some("2"), Some("back\\slash"), Some("ok")],
            ],
        );
        let text = json_result_lines(&result).join("\n");
        let parsed: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
        let rows = parsed.as_array().expect("top-level array");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["id"], "1");
        assert_eq!(rows[0]["note"], "a\"b\nline");
        assert!(rows[0]["missing"].is_null());
        assert_eq!(rows[1]["note"], "back\\slash");
        assert_eq!(rows[1]["missing"], "ok");
    }

    #[test]
    fn json_view_has_no_rows_when_the_result_is_empty() {
        assert_eq!(
            json_result_lines(&result(&["id"], Vec::new())),
            vec!["[", "]"]
        );
    }
}
