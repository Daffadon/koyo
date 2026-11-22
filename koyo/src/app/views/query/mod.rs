use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Editor;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::tab::{Tab as BarTab, TabBar};
use gpui_kit::component::{ActiveTheme, Disableable, IconName, Sizable, Size};
use gpui_kit::component::{h_flex, resizable_panel, v_flex, v_resizable};
use gpui_kit::prelude::*;
use gpui_kit::*;
use gpui_kit::{InteractiveElement, StatefulInteractiveElement};
use koyo_core::models::Engine;
use std::cell::RefCell;
use std::collections::HashMap;

use super::widgets::{button_tooltip, empty_state};
use crate::app::{KoyoApp, Palette, ResultView, TabBody};

mod grid;
mod json;

const COLLAPSED_RESULTS_HEIGHT: f32 = 38.0;

thread_local! {
    /// Two-axis result-grid handles. The JSON view keeps its uniform list
    /// handle above; the table view uses one scroller for both axes.
    static RESULT_GRID_SCROLL_HANDLES: RefCell<HashMap<(u64, usize), ScrollHandle>> =
        RefCell::new(HashMap::new());
}

pub(super) fn result_grid_scroll_handle(tab_id: u64, result_index: usize) -> ScrollHandle {
    RESULT_GRID_SCROLL_HANDLES.with(|handles| {
        handles
            .borrow_mut()
            .entry((tab_id, result_index))
            .or_insert_with(ScrollHandle::new)
            .clone()
    })
}

/// Drop handles for closed tabs and for result sets the query no longer has.
pub(super) fn prune_scroll_handles(app: &KoyoApp) {
    RESULT_GRID_SCROLL_HANDLES.with(|handles| {
        let mut handles = handles.borrow_mut();
        handles.retain(|(tab_id, result_index), _| {
            app.tabs
                .iter()
                .find(|tab| tab.id == *tab_id)
                .and_then(|tab| match &tab.body {
                    TabBody::Query(query) => query.output.as_ref(),
                    TabBody::Table(_) => None,
                })
                .is_some_and(|output| *result_index < output.result_sets.len())
        });
        if handles.capacity() > handles.len().saturating_mul(4).max(16) {
            handles.shrink_to_fit();
        }
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
    let reconnect_profile = tab.profile.as_ref().map(|profile| profile.id);
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
    let result_widths = query
        .result_column_widths
        .get(active_result)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let run_button = Button::new(("run-query", tab_id))
        .primary()
        // Constant label: changing it between "Run query" and "Running"
        // resized the toolbar on every execution.
        .label("Run query")
        .icon(IconName::Play)
        .loading(running)
        .disabled(!connected || running)
        .tooltip("Run query (Ctrl/⌘ + Enter); selected SQL runs alone")
        .tooltip_show_delay(std::time::Duration::from_millis(120))
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
                    json::result_json(result, tab_id, active_result, palette, &mono, cx)
                } else {
                    grid::result_grid(
                        result,
                        tab_id,
                        active_result,
                        result_widths,
                        palette,
                        &mono,
                        cx,
                    )
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

    let weak_app = cx.entity().downgrade();
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
                .when(
                    output.is_some_and(|output| !output.result_sets.is_empty()),
                    |this| {
                        this.child(button_tooltip(
                            ("tooltip-download-csv", tab_id),
                            Button::new(("download-result-csv", tab_id))
                                .ghost()
                                .compact()
                                .icon(IconName::ArrowDown)
                                .label("CSV")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.download_active_result(tab_id, window, cx);
                                })),
                            "Download active result as CSV",
                        ))
                        .child(button_tooltip(
                            ("tooltip-copy-csv", tab_id),
                            Button::new(("copy-result-csv", tab_id))
                                .ghost()
                                .compact()
                                .icon(IconName::Copy)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.copy_active_result(tab_id, cx);
                                })),
                            "Copy active result as CSV",
                        ))
                        .child({
                            let csv_app = weak_app.clone();
                            let json_app = weak_app.clone();
                            let download_app = weak_app.clone();
                            let collapse_app = weak_app.clone();
                            Button::new(("result-options", tab_id))
                                .ghost()
                                .compact()
                                .icon(IconName::Ellipsis)
                                .accessibility_label("All result options")
                                .tooltip("All result options")
                                .dropdown_menu(move |menu, _, _| {
                                    let csv_app = csv_app.clone();
                                    let json_app = json_app.clone();
                                    let download_app = download_app.clone();
                                    let collapse_app = collapse_app.clone();
                                    menu.item(
                                        PopupMenuItem::new("Copy as CSV").on_click(move |_, _, cx| {
                                            let _ = csv_app.update(cx, |this, cx| {
                                                this.copy_active_result(tab_id, cx);
                                            });
                                        }),
                                    )
                                    .item(
                                        PopupMenuItem::new("Copy as JSON").on_click(move |_, _, cx| {
                                            let _ = json_app.update(cx, |this, cx| {
                                                this.copy_active_result_json(tab_id, cx);
                                            });
                                        }),
                                    )
                                    .item(
                                        PopupMenuItem::new("Download as CSV...").on_click(
                                            move |_, window, cx| {
                                                let _ = download_app.update(cx, |this, cx| {
                                                    this.download_active_result(tab_id, window, cx);
                                                });
                                            },
                                        ),
                                    )
                                    .separator()
                                    .item(
                                        PopupMenuItem::new(if results_collapsed {
                                            "Expand results"
                                        } else {
                                            "Collapse results"
                                        })
                                        .on_click(move |_, _, cx| {
                                            let _ = collapse_app.update(cx, |this, cx| {
                                                this.toggle_results(tab_id, cx);
                                            });
                                        }),
                                    )
                                })
                        })
                    },
                )
                .when(output.is_some_and(|output| output.truncated), |this| {
                    this.child(
                        Button::new(("result-truncated", tab_id))
                            .ghost()
                            .compact()
                            .icon(IconName::Info)
                            .label("Truncated")
                            .tooltip(
                                "The result hit Koyo's row or memory cap. Copy includes the rows that were returned.",
                            ),
                    )
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
                            Button::new(("cancel-query", tab_id))
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
                        .child(
                            Button::new(("format-sql", tab_id))
                                .secondary()
                                .compact()
                                .label("Format")
                                .tooltip("Format SQL; selection formats alone")
                                .tooltip_show_delay(std::time::Duration::from_millis(120))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.format_query(tab_id, window, cx);
                                })),
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
                        Button::new(("query-reconnect", tab_id))
                            .link()
                            .compact()
                            .label(if tab.profile.is_some() {
                                "Reconnect"
                            } else {
                                "Connect"
                            })
                            .on_click(cx.listener(
                                move |this, _, window, cx| match reconnect_profile {
                                    Some(profile_id) => this.connect_profile(
                                        profile_id,
                                        Some(tab_id),
                                        false,
                                        window,
                                        cx,
                                    ),
                                    None => this.open_connection_dialog(Some(tab_id), window, cx),
                                },
                            )),
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
                                    .bg(palette.editor)
                                    .capture_key_down(cx.listener(
                                        move |this, event: &KeyDownEvent, _, cx| {
                                            let modifiers = event.keystroke.modifiers;
                                            if event.keystroke.key == "enter"
                                                && (modifiers.control || modifiers.platform)
                                            {
                                                cx.stop_propagation();
                                                this.run_query(tab_id, cx);
                                            }
                                        },
                                    ))
                                    .child(
                                        Editor::new(&query.editor)
                                            .aria_label("SQL editor")
                                            .h_full()
                                            .bordered(false),
                                    ),
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
        .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
            if event.dragging() {
                this.resize_active_result_column(tab_id, f32::from(event.position.x), cx);
            }
        }))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.end_result_column_resize(tab_id, cx);
            }),
        )
        .into_any_element()
}
