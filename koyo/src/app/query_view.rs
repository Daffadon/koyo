use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Editor;
use gpui_kit::component::tab::{Tab as BarTab, TabBar};
use gpui_kit::component::{ActiveTheme, Disableable, IconName, Sizable, Size};
use gpui_kit::component::{h_flex, resizable_panel, v_flex, v_resizable};
use gpui_kit::prelude::*;
use gpui_kit::*;
use gpui_kit::{InteractiveElement, StatefulInteractiveElement};
use koyo_core::models::{Engine, QueryResultSet};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use super::widgets::{empty_state, value_text};
use super::{KoyoApp, Palette, ResultView, TabBody};

/// Height of the results header bar when the results area is collapsed.
const COLLAPSED_RESULTS_HEIGHT: f32 = 38.0;

/// Fixed result row height so the virtual list's uniform scroll math is exact.
/// Matches `table_view::ROW_HEIGHT`: `text_sm` line box (0.875rem x 1.618
/// ~= 23px) + `py_1` (8px) + the 1px bottom border.
const ROW_HEIGHT: f32 = 32.0;

thread_local! {
    /// Virtual list scroll handles keyed by (tab id, result set index) so the
    /// scroll position survives the re-render GPUI runs every frame.
    static RESULT_SCROLL_HANDLES: RefCell<HashMap<(u64, usize), UniformListScrollHandle>> =
        RefCell::new(HashMap::new());
}

fn result_scroll_handle(tab_id: u64, result_index: usize) -> UniformListScrollHandle {
    RESULT_SCROLL_HANDLES.with(|handles| {
        handles
            .borrow_mut()
            .entry((tab_id, result_index))
            .or_insert_with(UniformListScrollHandle::new)
            .clone()
    })
}

/// Drop handles for closed tabs and for result sets the query no longer has.
fn prune_result_scroll_handles(app: &KoyoApp, tab_id: u64, result_count: usize) {
    RESULT_SCROLL_HANDLES.with(|handles| {
        let mut handles = handles.borrow_mut();
        handles.retain(|(handle_tab_id, result_index), _| {
            if *handle_tab_id == tab_id {
                *result_index < result_count
            } else {
                app.tabs.iter().any(|tab| tab.id == *handle_tab_id)
            }
        });
    });
}

pub(super) fn render(
    app: &KoyoApp,
    index: usize,
    palette: &Palette,
    cx: &mut Context<KoyoApp>,
) -> AnyElement {
    let Some(tab) = app.tabs.get(index) else {
        return div().into_any_element();
    };
    let TabBody::Query(query) = &tab.body else {
        return div().into_any_element();
    };
    let tab_id = tab.id;
    let connected = tab.connection.is_some();
    let engine = tab.engine();
    let mono = cx.theme().mono_font_family.clone();
    let running = query.running;
    let cancelling = query.cancelling;
    let cancel_supported = engine != Some(Engine::Oracle);
    let error = query.error.clone();
    let duration = query.duration_ms;
    let results_collapsed = query.results_collapsed;
    let result_view = query.result_view;
    let output = query.output.as_ref();
    let active_result = output
        .map(|output| {
            query
                .active_result
                .min(output.result_sets.len().saturating_sub(1))
        })
        .unwrap_or(0);
    prune_result_scroll_handles(
        app,
        tab_id,
        output.map(|output| output.result_sets.len()).unwrap_or(0),
    );

    let run_button = Button::new("run-query")
        .primary()
        // Constant label: changing it between "Run query" and "Running"
        // resized the toolbar on every execution.
        .label("Run query")
        .icon(IconName::Play)
        .loading(running)
        .disabled(!connected || running)
        .on_click(cx.listener(move |this, _, _, cx| {
            this.run_query(tab_id, cx);
        }));

    let result_tabs = output
        .filter(|output| output.result_sets.len() > 1)
        .map(|output| {
            let tabs = output
                .result_sets
                .iter()
                .enumerate()
                .map(|(index, result)| {
                    BarTab::new()
                        .label(format!("Result {} · {}", index + 1, result.rows.len()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_active_result(tab_id, index, cx);
                        }))
                })
                .collect::<Vec<_>>();
            TabBar::new(("result-tabs", tab_id))
                .segmented()
                .selected_index(active_result)
                .children(tabs)
        });

    let result_body = match (error.is_empty(), output) {
        (false, _) => v_flex()
            .id(("query-error", tab_id))
            .flex_1()
            .min_h(px(0.0))
            .w_full()
            .p_5()
            .overflow_y_scroll()
            .child(
                h_flex()
                    .items_stretch()
                    .w_full()
                    .max_w(px(900.0))
                    .bg(palette.panel)
                    .border_1()
                    .border_color(palette.border)
                    .child(div().w(px(2.0)).flex_shrink_0().bg(palette.danger))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w(px(0.0))
                            .gap_2()
                            .p_4()
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        gpui_kit::component::Icon::new(IconName::CircleAlert)
                                            .with_size(Size::Small)
                                            .text_color(palette.danger),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(palette.danger)
                                            .child("Query failed"),
                                    ),
                            )
                            .child(
                                div()
                                    .font_family(mono)
                                    .text_sm()
                                    .text_color(palette.foreground)
                                    .whitespace_normal()
                                    .child(error.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(palette.muted)
                                    .whitespace_normal()
                                    .child("Fix the statement and run it again."),
                            ),
                    ),
            )
            .into_any_element(),
        (true, Some(output)) => {
            if output.result_sets.is_empty() {
                let affected = output.affected_rows.iter().copied().sum::<u64>();
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Statement complete"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.muted)
                            .child(format!("{affected} rows affected")),
                    )
                    .into_any_element()
            } else if let Some(result) = output.result_sets.get(active_result) {
                if result_view == ResultView::Json {
                    result_json(result, tab_id, active_result, palette, &mono, cx)
                } else {
                    result_grid(result, tab_id, active_result, palette, &mono, cx)
                }
            } else {
                empty_state("No results", "Run a statement to see rows.", palette)
                    .into_any_element()
            }
        }
        (true, None) => empty_state(
            "Results",
            if connected {
                "Results from your next statement appear here."
            } else {
                "Connect to run a statement and see results."
            },
            palette,
        )
        .into_any_element(),
    };

    let results_header = h_flex()
        .items_center()
        .justify_between()
        .px_5()
        .py_2()
        .border_b_1()
        .border_color(palette.border)
        .child(
            h_flex()
                .id(("results-toggle", tab_id))
                .items_center()
                .gap_2()
                .rounded_md()
                .cursor_pointer()
                .hover(|style| style.bg(palette.panel))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.toggle_results(tab_id, cx);
                }))
                .child(
                    gpui_kit::component::Icon::new(if results_collapsed {
                        IconName::ChevronRight
                    } else {
                        IconName::ChevronDown
                    })
                    .with_size(Size::Small),
                )
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Results"),
                ),
        )
        .child(
            h_flex()
                .items_center()
                .gap_3()
                .text_xs()
                .text_color(palette.muted)
                .child(
                    h_flex()
                        .items_center()
                        .gap_1()
                        .child(
                            Button::new(("result-view-table", tab_id))
                                .when(result_view == ResultView::Table, |this| this.secondary())
                                .when(result_view != ResultView::Table, |this| this.ghost())
                                .compact()
                                .label("Table")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.set_result_view(tab_id, ResultView::Table, cx);
                                })),
                        )
                        .child(
                            Button::new(("result-view-json", tab_id))
                                .when(result_view == ResultView::Json, |this| this.secondary())
                                .when(result_view != ResultView::Json, |this| this.ghost())
                                .compact()
                                .label("JSON")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.set_result_view(tab_id, ResultView::Json, cx);
                                })),
                        ),
                )
                .when_some(duration, |this, duration| {
                    this.child(format!("{duration} ms"))
                })
                .when(output.is_some_and(|output| output.truncated), |this| {
                    this.child("Each result capped at 1,000 rows")
                }),
        );

    let mut results_panel = v_flex()
        .flex_1()
        .min_h(px(0.0))
        .overflow_hidden()
        .child(results_header);
    if !results_collapsed {
        results_panel = results_panel
            .when_some(result_tabs, |this, tabs| this.child(tabs))
            .child(
                v_flex()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .child(result_body),
            );
    }

    v_flex()
        .flex_1()
        .w_full()
        .min_w(px(0.0))
        .h_full()
        .overflow_hidden()
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .px_5()
                .py_2()
                .border_b_1()
                .border_color(palette.border)
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(div().w(px(6.0)).h(px(6.0)).rounded_full().bg(if connected {
                            palette.success
                        } else {
                            palette.muted
                        }))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("SQL editor"),
                        ),
                )
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(if cancel_supported {
                            Button::new("cancel-query")
                                .secondary()
                                .compact()
                                .icon(IconName::Square)
                                .label("Cancel")
                                .disabled(cancelling)
                                .when(!running, |this| this.invisible())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.cancel_task(tab_id, false, cx);
                                }))
                                .into_any_element()
                        } else {
                            div()
                                .text_xs()
                                .text_color(palette.warning)
                                .when(!running, |this| this.invisible())
                                .child("Oracle beta cannot cancel")
                                .into_any_element()
                        })
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette.muted)
                                .child("Ctrl / ⌘ + Enter to run"),
                        )
                        .child(run_button),
                ),
        )
        .when(!connected, |this| {
            this.child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .px_5()
                    .py_2()
                    .bg(palette.panel)
                    .border_b_1()
                    .border_color(palette.border)
                    .text_xs()
                    .text_color(palette.muted)
                    .child(
                        gpui_kit::component::Icon::new(IconName::CircleAlert)
                            .with_size(Size::Small),
                    )
                    .child(if tab.profile.is_some() {
                        "Session disconnected."
                    } else {
                        "Connect a database to run SQL."
                    })
                    .child(
                        Button::new("query-reconnect")
                            .link()
                            .compact()
                            .label(if tab.profile.is_some() {
                                "Reconnect"
                            } else {
                                "Connect"
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_connection_dialog(Some(tab_id), window, cx);
                            })),
                    ),
            )
        })
        .child(
            div().flex_1().min_h(px(0.0)).overflow_hidden().child(
                v_resizable(("query-split", tab_id))
                    .child(
                        resizable_panel()
                            .size(px(320.0))
                            .size_range(px(100.0)..Pixels::MAX)
                            .when(!results_collapsed, |panel| panel.flex_none())
                            .child(
                                div()
                                    .size_full()
                                    .min_h(px(0.0))
                                    .child(Editor::new(&query.editor).h_full().bordered(false)),
                            ),
                    )
                    .child(
                        resizable_panel()
                            .size_range(px(120.0)..Pixels::MAX)
                            .when(results_collapsed, |panel| {
                                panel
                                    .size(px(COLLAPSED_RESULTS_HEIGHT))
                                    .size_range(
                                        px(COLLAPSED_RESULTS_HEIGHT)..px(COLLAPSED_RESULTS_HEIGHT),
                                    )
                                    .flex_none()
                            })
                            .child(results_panel),
                    ),
            ),
        )
        .into_any_element()
}

const JSON_LINE_HEIGHT: f32 = 22.0;

fn result_json(
    result: &QueryResultSet,
    tab_id: u64,
    result_index: usize,
    palette: &Palette,
    mono: &SharedString,
    cx: &mut Context<KoyoApp>,
) -> AnyElement {
    let lines = Rc::new(json_result_lines(result));
    let scroll_handle = result_scroll_handle(tab_id, result_index);
    let palette = *palette;
    let widest = lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(2);
    // monospace 14px text is roughly 8px/character; keep a comfortable gutter.
    let width = ((widest as f32) * 8.0 + 48.0).max(420.0);
    let lines_for_rows = Rc::clone(&lines);
    let rows = uniform_list(
        (
            SharedString::from(format!("result-json-rows-{tab_id}")),
            result_index,
        ),
        lines.len(),
        cx.processor(
            move |_app: &mut KoyoApp,
                  range: Range<usize>,
                  _window: &mut Window,
                  _cx: &mut Context<KoyoApp>| {
                range
                    .map(|line_index| {
                        let line = lines_for_rows.get(line_index).cloned().unwrap_or_default();
                        div()
                            .flex_shrink_0()
                            .h(px(JSON_LINE_HEIGHT))
                            .px_4()
                            .flex()
                            .items_center()
                            .whitespace_nowrap()
                            .text_sm()
                            .text_color(palette.foreground)
                            .child(line)
                            .into_any_element()
                    })
                    .collect::<Vec<_>>()
            },
        ),
    )
    .flex_1()
    .min_h(px(0.0))
    .track_scroll(&scroll_handle);

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.0))
        .overflow_hidden()
        .child(
            div()
                .id(("result-json-hscroll", tab_id))
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.0))
                .overflow_x_scroll()
                .child(
                    div()
                        .w(px(width))
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h(px(0.0))
                        .child(rows),
                ),
        )
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

fn result_grid(
    result: &QueryResultSet,
    tab_id: u64,
    result_index: usize,
    palette: &Palette,
    mono: &SharedString,
    cx: &mut Context<KoyoApp>,
) -> AnyElement {
    let width = (result.columns.len().max(1) * 180) as f32;
    let palette = *palette;
    let header = h_flex()
        .flex_shrink_0()
        .bg(palette.panel)
        .border_b_1()
        .border_color(palette.border)
        .children(result.columns.iter().enumerate().map(|(index, column)| {
            div()
                .id(SharedString::from(format!("result-head-{index}")))
                .w(px(180.0))
                .flex_shrink_0()
                .px_2()
                .py_2()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(palette.muted)
                .overflow_hidden()
                .whitespace_nowrap()
                .child(column.clone())
        }));

    let row_count = result.rows.len();

    // Only the rows inside the viewport are built; the uniform list measures
    // `ROW_HEIGHT` from the first row and computes the scroll range from it.
    let rows: AnyElement = if row_count == 0 {
        v_flex()
            .flex_1()
            .min_h(px(0.0))
            .p_4()
            .text_xs()
            .text_color(palette.muted)
            .child("No rows returned.")
            .into_any_element()
    } else {
        let scroll_handle = result_scroll_handle(tab_id, result_index);
        uniform_list(
            (
                SharedString::from(format!("result-rows-{tab_id}")),
                result_index,
            ),
            row_count,
            cx.processor(
                move |app: &mut KoyoApp,
                      range: Range<usize>,
                      _window: &mut Window,
                      _cx: &mut Context<KoyoApp>| {
                    let Some(result) = app
                        .tabs
                        .iter()
                        .find(|tab| tab.id == tab_id)
                        .and_then(|tab| match &tab.body {
                            TabBody::Query(query) => query.output.as_ref(),
                            TabBody::Table(_) => None,
                        })
                        .and_then(|output| output.result_sets.get(result_index))
                    else {
                        return Vec::new();
                    };
                    range
                        .map(|row_index| render_result_row(result, row_index, palette))
                        .collect::<Vec<_>>()
                },
            ),
        )
        .flex_1()
        .min_h(px(0.0))
        .track_scroll(&scroll_handle)
        .into_any_element()
    };

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.0))
        .overflow_hidden()
        .child(
            div()
                .id(("result-hscroll", tab_id))
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.0))
                .overflow_x_scroll()
                .child(div().w(px(width)).flex().flex_col().child(header))
                .child(
                    div()
                        .w(px(width))
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h(px(0.0))
                        .child(rows),
                ),
        )
        .font_family(mono.clone())
        .into_any_element()
}

fn render_result_row(result: &QueryResultSet, row_index: usize, palette: Palette) -> AnyElement {
    let Some(row) = result.rows.get(row_index) else {
        return div().h(px(ROW_HEIGHT)).into_any_element();
    };
    h_flex()
        .flex_shrink_0()
        .h(px(ROW_HEIGHT))
        .border_b_1()
        .border_color(palette.border)
        .when(row_index % 2 == 1, |this| this.bg(palette.panel))
        .children((0..result.columns.len()).map(|column_index| {
            let value = row.get(column_index).cloned().unwrap_or(None);
            div()
                .w(px(180.0))
                .flex_shrink_0()
                .px_2()
                .py_1()
                .overflow_hidden()
                .child(value_text(&value, &palette))
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{QueryResultSet, json_result_lines};

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
