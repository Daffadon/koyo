use gpui_kit::prelude::*;
use gpui_kit::*;
use koyo_core::models::QueryResultSet;
use std::rc::Rc;

use super::result_json_scroll_handle;
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
    let scroll_handle = result_json_scroll_handle(tab_id, result_index);
    let palette = *palette;
    let widest = lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(2);
    // monospace 14px text is roughly 8px/character; keep a comfortable gutter.
    let width = ((widest as f32) * 8.0 + 48.0).max(420.0);
    let lines_for_render = Rc::clone(&lines);
    let list = uniform_list(
        SharedString::from(format!("result-json-lines-{tab_id}-{result_index}")),
        lines.len(),
        move |range, _, _| {
            range
                .map(|line_index| {
                    div()
                        .id(SharedString::from(format!(
                            "result-json-line-{tab_id}-{result_index}-{line_index}"
                        )))
                        .flex_shrink_0()
                        .h(px(JSON_LINE_HEIGHT))
                        .px_4()
                        .flex()
                        .items_center()
                        .whitespace_nowrap()
                        .text_sm()
                        .text_color(palette.foreground)
                        .child(lines_for_render[line_index].clone())
                })
                .collect::<Vec<_>>()
        },
    )
    .track_scroll(&scroll_handle)
    .w(px(width))
    .min_w(px(width))
    .with_horizontal_sizing_behavior(gpui_kit::ListHorizontalSizingBehavior::Unconstrained);

    div()
        .id(SharedString::from(format!(
            "result-json-body-{tab_id}-{result_index}"
        )))
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.0))
        .overflow_hidden()
        .bg(palette.background)
        .child(list)
        .font_family(mono.clone())
        .into_any_element()
}

fn json_result_lines(result: &QueryResultSet) -> Vec<String> {
    let mut lines = Vec::with_capacity(result.rows.len() * (result.columns.len() + 2) + 2);
    lines.push("[".to_string());
    for (row_index, row) in result.rows.iter().enumerate() {
        lines.push("  {".to_string());
        for (column_index, column) in result.columns.iter().enumerate() {
            let mut line = String::from("    ");
            push_json_string(&mut line, column);
            line.push_str(": ");
            match row.get(column_index).and_then(Option::as_deref) {
                Some(value) => push_json_string(&mut line, value),
                None => line.push_str("null"),
            }
            if column_index + 1 < result.columns.len() {
                line.push(',');
            }
            lines.push(line);
        }
        lines.push(if row_index + 1 < result.rows.len() {
            "  },".to_string()
        } else {
            "  }".to_string()
        });
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
        let lines = json_result_lines(&result);
        assert_eq!(lines.first().map(String::as_str), Some("["));
        assert_eq!(lines.get(1).map(String::as_str), Some("  {"));
        assert!(
            lines.iter().any(|line| line == "    \"missing\": null"),
            "NULL cells must render as JSON null"
        );
        assert_eq!(lines.last().map(String::as_str), Some("]"));
        let text = lines.join("\n");
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
    fn json_view_keeps_empty_strings_distinct_from_null() {
        let result = result(&["empty", "missing"], vec![vec![Some(""), None]]);
        let lines = json_result_lines(&result);
        assert!(lines.iter().any(|line| line == "    \"empty\": \"\","));
        assert!(lines.iter().any(|line| line == "    \"missing\": null"));
    }

    #[test]
    fn json_view_has_no_rows_when_the_result_is_empty() {
        assert_eq!(
            json_result_lines(&result(&["id"], Vec::new())),
            vec!["[", "]"]
        );
    }
}
