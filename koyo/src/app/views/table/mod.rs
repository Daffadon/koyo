use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme, Disableable, IconName, Sizable, Size};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use koyo_core::models::TablePage;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::widgets::{button_tooltip, empty_state};
use crate::app::{KoyoApp, Palette, StagedChange, TabBody, TableMode};

fn loading_state(palette: &Palette) -> AnyElement {
    v_flex()
        .flex_1()
        .w_full()
        .min_h(px(0.0))
        .items_center()
        .justify_center()
        .child(Spinner::new().with_size(Size::Large).color(palette.muted))
        .into_any_element()
}

mod diagram;
mod filter;
mod grid;

const CELL_WIDTH: f32 = crate::app::DEFAULT_COLUMN_WIDTH;
/// Narrow gutter that carries the global row number. It scrolls with the data
/// columns and must be part of every width calculation in the grid.
pub(super) const ROW_RAIL_WIDTH: f32 = 56.0;
/// Every grid row is forced to this height so one scroll gesture maps
/// cleanly on both axes. It matches the natural row height: `text_sm`
/// line box (0.875rem x 1.618 ~= 23px) + `py_1` (8px) + the 1px bottom border.
pub(super) const ROW_HEIGHT: f32 = 32.0;

thread_local! {
    /// Two-axis scroll handles keyed by tab id, so the handle (and with it
    /// the scroll position) survives the re-render GPUI runs every frame.
    /// Kept here instead of in `TableTab` to avoid growing the tab state.
    static TABLE_SCROLL_HANDLES: RefCell<HashMap<u64, ScrollHandle>> =
        RefCell::new(HashMap::new());
}

fn table_scroll_handle(tab_id: u64) -> ScrollHandle {
    TABLE_SCROLL_HANDLES.with(|handles| {
        handles
            .borrow_mut()
            .entry(tab_id)
            .or_insert_with(ScrollHandle::new)
            .clone()
    })
}

/// Drop handles for tabs that no longer exist so closing tables does not leak.
pub(super) fn prune_scroll_handles(app: &KoyoApp) {
    TABLE_SCROLL_HANDLES.with(|handles| {
        let mut handles = handles.borrow_mut();
        handles.retain(|tab_id, _| {
            app.tabs
                .iter()
                .any(|tab| tab.id == *tab_id && matches!(tab.body, TabBody::Table(_)))
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
    let TabBody::Table(table) = &tab.body else {
        return div().into_any_element();
    };
    let tab_id = tab.id;
    let connected = tab.connection.is_some();
    let loading = table.loading;
    let cancelling = table.cancelling;
    let staged: Rc<Vec<StagedChange>> = Rc::new(table.staged.clone());
    let page: Option<Rc<TablePage>> = table.page.clone().map(Rc::new);
    let sort = table.sort.clone();
    let engine = tab.engine();
    let cancel_supported = engine != Some(koyo_core::models::Engine::Oracle);
    let has_staged = !staged.is_empty();
    let mode = table.mode;

    let title = format!("{}.{}", table.schema, table.table);
    let subtitle = if !connected {
        if tab.connecting {
            "Connecting to this table..."
        } else {
            "Reconnect to edit or filter this table."
        }
    } else if page.as_ref().is_some_and(|page| page.editable) {
        "Edits are staged until you apply them."
    } else {
        "Read-only table without a primary key."
    };

    let page_ref = page.as_deref();
    let editable = page_ref.is_some_and(|page| page.editable) && connected;
    let columns = page_ref
        .map(|page| page.columns.clone())
        .unwrap_or_default();
    let offset = page_ref.map(|page| page.offset).unwrap_or(0);
    let limit = page_ref
        .map(|page| page.limit)
        .unwrap_or(crate::app::PAGE_LIMIT);
    let has_more = page_ref.is_some_and(|page| page.has_more);
    let total_rows = page_ref.map(|page| page.total_rows).unwrap_or(0);
    let total_pages = total_rows.div_ceil(u64::from(limit.max(1))).max(1);
    let page_number = offset / limit.max(1) + 1;

    let mode_switch = h_flex()
        .items_center()
        .gap_1()
        .p_1()
        .rounded_md()
        .border_1()
        .border_color(palette.border)
        .bg(palette.panel)
        .child(
            Button::new("table-mode-data")
                .when(mode == TableMode::Data, |this| this.primary())
                .when(mode != TableMode::Data, |this| this.ghost())
                .compact()
                .icon(IconName::LayoutDashboard)
                .label("Data")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_table_mode(tab_id, TableMode::Data, cx);
                })),
        )
        .child(
            Button::new("table-mode-diagram")
                .when(mode == TableMode::Diagram, |this| this.primary())
                .when(mode != TableMode::Diagram, |this| this.ghost())
                .compact()
                .icon(IconName::Frame)
                .label("Diagram")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_table_mode(tab_id, TableMode::Diagram, cx);
                })),
        );

    let mut actions = h_flex().flex_wrap().items_center().gap_2().child(
        div()
            .w(px(88.0))
            .flex()
            .justify_end()
            .when(has_staged, |this| {
                this.child(
                    div()
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .bg(palette.panel_alt)
                        .text_xs()
                        .text_color(palette.warning)
                        .whitespace_nowrap()
                        .child(format!("{} pending", staged.len())),
                )
            }),
    );

    // Keep Cancel mounted when idle. Loading is often shorter than one
    // animation frame; toggling the button in and out is what flickers.
    let cancel_disabled = !loading || cancelling || !cancel_supported;
    actions = actions
        .child(
            Button::new("cancel-table")
                .secondary()
                .compact()
                .icon(IconName::Square)
                .label("Cancel")
                .disabled(cancel_disabled)
                .when(cancel_disabled, |this| this.text_color(palette.foreground))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.cancel_task(tab_id, true, cx);
                })),
        )
        .child(
            Button::new("discard-changes")
                .secondary()
                .compact()
                .icon(IconName::Close)
                .label("Discard")
                .disabled(!has_staged || loading || !connected)
                .when(!has_staged || loading || !connected, |this| {
                    this.text_color(palette.foreground)
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.discard_changes(tab_id, cx);
                })),
        )
        .child(
            Button::new("apply-changes")
                .primary()
                .compact()
                .icon(IconName::Check)
                .label("Apply changes")
                .disabled(!has_staged || loading || !connected)
                .when(!has_staged || loading || !connected, |this| {
                    this.text_color(palette.foreground)
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.apply_changes(tab_id, cx);
                })),
        )
        .child(button_tooltip(
            "tooltip-refresh-table",
            Button::new("refresh-table")
                .ghost()
                .compact()
                .icon(IconName::RefreshCw)
                .disabled(loading || !connected)
                .when(loading || !connected, |this| {
                    this.text_color(palette.foreground)
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.refresh_current_table(tab_id, window, cx);
                })),
            "Refresh table",
        ))
        .when(editable, |this| {
            this.child(
                Button::new("add-row")
                    .secondary()
                    .compact()
                    .icon(IconName::Plus)
                    .label("Add row")
                    .disabled(loading)
                    .when(loading, |this| this.text_color(palette.foreground))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.add_row(tab_id, cx);
                    })),
            )
        });

    let toolbar = h_flex()
        .flex_wrap()
        .items_center()
        .gap_4()
        .px_5()
        .py_3()
        .border_b_1()
        .border_color(palette.border)
        .child(
            v_flex()
                .min_w(px(180.0))
                .max_w(px(420.0))
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(div().text_xs().text_color(palette.muted).child(subtitle)),
        )
        .child(mode_switch)
        .child(div().flex_1().min_w(px(0.0)))
        .child(actions);

    if mode == TableMode::Diagram {
        let mono = cx.theme().mono_font_family.clone();
        return v_flex()
            .size_full()
            .w_full()
            .min_h(px(0.0))
            .child(toolbar)
            .child(diagram::render_diagram(
                tab_id, table, page_ref, palette, &mono,
            ))
            .into_any_element();
    }

    let filter_bar = filter::render_filter_bar(tab_id, table, palette, cx);
    let column_widths = columns
        .iter()
        .enumerate()
        .map(|(index, _)| {
            table
                .column_widths
                .get(index)
                .copied()
                .unwrap_or(CELL_WIDTH)
        })
        .collect::<Vec<_>>();

    let grid = if page.is_none() && loading {
        loading_state(palette)
    } else if let Some(page) = &page {
        grid::render_grid(
            tab_id,
            Rc::clone(page),
            Rc::clone(&staged),
            editable,
            connected,
            &sort,
            &column_widths,
            palette,
            cx,
        )
    } else {
        empty_state(
            "No table data",
            "Choose a table from the explorer.",
            palette,
        )
        .into_any_element()
    };
    let grid = if loading && page.is_some() {
        div()
            .relative()
            .flex_1()
            .w_full()
            .min_h(px(0.0))
            .child(grid)
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(palette.background)
                    .child(Spinner::new().with_size(Size::Large).color(palette.muted)),
            )
            .into_any_element()
    } else {
        grid
    };

    let footer_mono = cx.theme().mono_font_family.clone();

    v_flex()
        .flex_1()
        .w_full()
        .min_w(px(0.0))
        .h_full()
        .overflow_hidden()
        .child(toolbar)
        .child(filter_bar)
        .child(grid)
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .px_5()
                .py_2()
                .border_t_1()
                .border_color(palette.border)
                .text_xs()
                .text_color(palette.muted)
                .child(div().font_family(footer_mono.clone()).child(format!(
                    "{total_rows} rows · {} on this page · {} columns",
                    page_ref.map(|page| page.rows.len()).unwrap_or(0),
                    columns.len()
                )))
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .font_family(footer_mono)
                                .child(format!("Page {page_number} of {total_pages}")),
                        )
                        .child(
                            Button::new("prev-page")
                                .ghost()
                                .compact()
                                .icon(IconName::ChevronLeft)
                                .disabled(loading || !connected || offset == 0)
                                .when(loading || !connected || offset == 0, |this| {
                                    this.text_color(palette.foreground)
                                })
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.request_page(
                                        tab_id,
                                        offset.saturating_sub(limit),
                                        window,
                                        cx,
                                    );
                                })),
                        )
                        .child(
                            Button::new("next-page")
                                .ghost()
                                .compact()
                                .icon(IconName::ChevronRight)
                                .disabled(
                                    loading
                                        || !connected
                                        || !has_more
                                        || u64::from(page_number) >= total_pages,
                                )
                                .when(
                                    loading
                                        || !connected
                                        || !has_more
                                        || u64::from(page_number) >= total_pages,
                                    |this| this.text_color(palette.foreground),
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.request_page(tab_id, offset + limit, window, cx);
                                })),
                        ),
                ),
        )
        .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
            if event.dragging() {
                this.resize_active_column(tab_id, f32::from(event.position.x), cx);
            }
        }))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.end_column_resize(tab_id, cx);
            }),
        )
        .into_any_element()
}
