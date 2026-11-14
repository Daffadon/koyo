use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::{ActiveTheme, Disableable, Icon, IconName, Sizable, Size};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use gpui_kit::{CursorStyle, InteractiveElement, StatefulInteractiveElement};
use koyo_core::models::{
    ColumnInfo, FilterKind, FilterOperator, GridRow, TableFilter, TablePage, TableSort,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use super::widgets::{empty_state, value_text};
use super::{
    KoyoApp, Palette, StagedChange, StagedInsert, TabBody, TableMode, filter_needs_value,
    operator_options,
};

const CELL_WIDTH: f32 = super::DEFAULT_COLUMN_WIDTH;
const ACTION_WIDTH: f32 = 44.0;
/// Narrow gutter that carries the global row number. It scrolls with the data
/// columns and must be part of every width calculation in the grid.
const ROW_RAIL_WIDTH: f32 = 56.0;
/// Every grid row is forced to this height so the virtual list's uniform
/// scroll math is exact. It matches the natural row height: `text_sm`
/// line box (0.875rem x 1.618 ~= 23px) + `py_1` (8px) + the 1px bottom border.
const ROW_HEIGHT: f32 = 32.0;

thread_local! {
    /// Virtual list scroll handles keyed by tab id, so the handle (and with it
    /// the scroll position) survives the re-render GPUI runs every frame.
    /// Kept here instead of in `TableTab` to avoid growing the tab state.
    static TABLE_SCROLL_HANDLES: RefCell<HashMap<u64, UniformListScrollHandle>> =
        RefCell::new(HashMap::new());
}

fn table_scroll_handle(tab_id: u64) -> UniformListScrollHandle {
    TABLE_SCROLL_HANDLES.with(|handles| {
        handles
            .borrow_mut()
            .entry(tab_id)
            .or_insert_with(UniformListScrollHandle::new)
            .clone()
    })
}

/// Drop handles for tabs that no longer exist so closing tables does not leak.
fn prune_table_scroll_handles(app: &KoyoApp) {
    TABLE_SCROLL_HANDLES.with(|handles| {
        let mut handles = handles.borrow_mut();
        if handles.len() > app.tabs.len() {
            handles.retain(|tab_id, _| app.tabs.iter().any(|tab| tab.id == *tab_id));
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
    prune_table_scroll_handles(app);
    let staged: Rc<Vec<StagedChange>> = Rc::new(table.staged.clone());
    let page: Option<Rc<TablePage>> = table.page.clone().map(Rc::new);
    let sort = table.sort.clone();
    let selected_column = table.filter_column;
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
    let limit = page_ref.map(|page| page.limit).unwrap_or(super::PAGE_LIMIT);
    let has_more = page_ref.is_some_and(|page| page.has_more);
    let page_number = offset / limit.max(1) + 1;

    let toolbar = h_flex()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap_4()
        .px_5()
        .py_3()
        .border_b_1()
        .border_color(palette.border)
        .child(
            v_flex()
                .flex_1()
                .min_w(px(180.0))
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(div().text_xs().text_color(palette.muted).child(subtitle)),
        )
        .child(
            h_flex()
                .flex_wrap()
                .items_center()
                .gap_2()
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
                )
                .child(
                    // Reserved slot: mounting the pending badge on the first
                    // edit would reflow every button after it.
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
                )
                .child(
                    // Keep the cancel control mounted while idle. Table sorts
                    // and page loads toggle `loading`; inserting/removing this
                    // button made the toolbar visibly jump on every header
                    // click. `invisible` preserves its slot and button geometry.
                    Button::new("cancel-table")
                        .secondary()
                        .compact()
                        .icon(IconName::Square)
                        .label("Cancel")
                        .disabled(cancelling)
                        .when(!loading || !cancel_supported, |this| this.invisible())
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
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.apply_changes(tab_id, cx);
                        })),
                )
                .child(
                    Button::new("refresh-table")
                        .ghost()
                        .compact()
                        .icon(IconName::RefreshCw)
                        .tooltip("Refresh table")
                        .disabled(loading || !connected)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.refresh_current_table(tab_id, window, cx);
                        })),
                )
                .when(editable, |this| {
                    this.child(
                        Button::new("add-row")
                            .secondary()
                            .compact()
                            .icon(IconName::Plus)
                            .label("Add row")
                            .disabled(loading)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.add_row(tab_id, cx);
                            })),
                    )
                }),
        );

    if mode == TableMode::Diagram {
        return v_flex()
            .size_full()
            .w_full()
            .min_h(px(0.0))
            .child(toolbar)
            .child(render_diagram(tab_id, table, page_ref, palette))
            .into_any_element();
    }

    let filter_bar = render_filter_bar(tab_id, table, &columns, selected_column, palette, cx);
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

    let grid = if let Some(page) = &page {
        render_grid(
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
            if loading {
                "Loading table"
            } else {
                "No table data"
            },
            if loading {
                "Fetching the first page..."
            } else {
                "Choose a table from the explorer."
            },
            palette,
        )
        .into_any_element()
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
                    "{} rows on this page · {} columns",
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
                                .child(format!("Page {page_number}")),
                        )
                        .child(
                            Button::new("prev-page")
                                .ghost()
                                .compact()
                                .icon(IconName::ChevronLeft)
                                .disabled(loading || !connected || offset == 0)
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
                                .disabled(loading || !connected || !has_more)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.request_page(tab_id, offset + limit, window, cx);
                                })),
                        ),
                ),
        )
        .into_any_element()
}

fn render_filter_bar(
    tab_id: u64,
    table: &super::TableTab,
    columns: &[ColumnInfo],
    selected_column: Option<usize>,
    palette: &Palette,
    cx: &mut Context<KoyoApp>,
) -> AnyElement {
    if columns.is_empty() {
        return div().into_any_element();
    }
    let locked = table.loading || !table.staged.is_empty();
    let selected = selected_column.and_then(|index| columns.get(index));
    let kind = selected
        .map(|column| column.filter_kind)
        .unwrap_or(FilterKind::Text);
    let operators = operator_options(kind);
    let needs_value = filter_needs_value(table.filter_operator);
    let operator = table.filter_operator;

    let mut bar = v_flex()
        .px_5()
        .py_2()
        .gap_2()
        .border_b_1()
        .border_color(palette.border)
        .bg(palette.sidebar);

    bar = bar.child(
        h_flex()
            .items_center()
            .gap_2()
            .flex_wrap()
            .child(div().text_xs().text_color(palette.muted).child("Filter"))
            .children(columns.iter().enumerate().map(|(index, column)| {
                Button::new(("filter-column", index))
                    .compact()
                    .ghost()
                    .label(column.name.clone())
                    .when(selected_column == Some(index), |this| this.primary())
                    .disabled(locked)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_filter_column(tab_id, index, window, cx);
                    }))
            })),
    );

    if selected.is_some() {
        bar = bar.child(
            h_flex()
                .items_center()
                .gap_2()
                .flex_wrap()
                .children(operators.iter().map(|(op, label)| {
                    let op = *op;
                    Button::new(SharedString::from(format!("filter-op-{op:?}")))
                        .compact()
                        .ghost()
                        .label(*label)
                        .when(op == operator, |this| this.primary())
                        .disabled(locked)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_filter_operator(tab_id, op, cx);
                        }))
                }))
                .child(
                    div().w(px(220.0)).child(
                        Input::new(&table.filter_value)
                            .w_full()
                            .disabled(locked || !needs_value),
                    ),
                )
                .child(
                    Button::new("apply-filter")
                        .primary()
                        .compact()
                        .icon(IconName::Search)
                        .label("Apply filter")
                        .disabled(locked)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.apply_filter(tab_id, window, cx);
                        })),
                )
                .child(
                    Button::new("clear-filters")
                        .secondary()
                        .compact()
                        .label("Clear")
                        .disabled(locked || table.filters.is_empty())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.clear_filters(tab_id, cx);
                        })),
                ),
        );
    }

    if !table.filters.is_empty() {
        bar = bar.child(h_flex().items_center().gap_1().flex_wrap().children(
            table.filters.iter().map(|filter| {
                let column = filter.column.clone();
                h_flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .bg(palette.panel)
                    .text_xs()
                    .child(filter_label(filter))
                    .child(
                        Button::new(SharedString::from(format!(
                            "remove-filter-{}",
                            filter.column
                        )))
                        .ghost()
                        .compact()
                        .icon(IconName::Close)
                        .tooltip("Remove filter")
                        .disabled(locked)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.remove_filter(tab_id, column.clone(), cx);
                        })),
                    )
            }),
        ));
    }

    bar.into_any_element()
}

fn filter_label(filter: &TableFilter) -> String {
    let operator = match filter.operator {
        FilterOperator::Contains => "contains",
        FilterOperator::Equals => "=",
        FilterOperator::NotEquals => "≠",
        FilterOperator::LessThan => "<",
        FilterOperator::LessThanOrEqual => "≤",
        FilterOperator::GreaterThan => ">",
        FilterOperator::GreaterThanOrEqual => "≥",
        FilterOperator::IsNull => "is null",
        FilterOperator::IsNotNull => "is not null",
        FilterOperator::IsTrue => "is true",
        FilterOperator::IsFalse => "is false",
    };
    match &filter.value {
        Some(value) => format!("{} {} {}", filter.column, operator, value),
        None => format!("{} {}", filter.column, operator),
    }
}

#[allow(clippy::too_many_arguments)]
fn render_grid(
    tab_id: u64,
    page: Rc<TablePage>,
    staged: Rc<Vec<StagedChange>>,
    editable: bool,
    connected: bool,
    sort: &Option<TableSort>,
    column_widths: &[f32],
    palette: &Palette,
    cx: &mut Context<KoyoApp>,
) -> AnyElement {
    let action_width = if editable { ACTION_WIDTH } else { 0.0 };
    let column_widths = Rc::new(column_widths.to_vec());
    let width = ROW_RAIL_WIDTH + column_widths.iter().sum::<f32>() + action_width;
    // Header affordances must not dim while a sort/page load is in flight;
    // only staged edits and missing connections make a column unsortable.
    let header_locked = !staged.is_empty();
    // `Palette` is `Copy`; the virtual list row closure must be `'static`.
    let palette = *palette;
    // Monospace data is part of the read of the grid; take it once here so the
    // virtual row closure and the header share the same face.
    let mono = cx.theme().mono_font_family.clone();

    let header = h_flex()
        .flex_shrink_0()
        .bg(palette.panel)
        .border_b_1()
        .border_color(palette.border)
        .child(
            div()
                .w(px(ROW_RAIL_WIDTH))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_end()
                .px_1p5()
                .py_2()
                .text_xs()
                .text_color(palette.muted)
                .border_r_1()
                .border_color(palette.border)
                .child("Row"),
        )
        .when(editable, |this| {
            this.child(
                div()
                    .w(px(ACTION_WIDTH))
                    .flex_shrink_0()
                    .border_r_1()
                    .border_color(palette.border),
            )
        })
        .children(
            page.columns
                .iter()
                .enumerate()
                .map(|(column_index, column)| {
                    let name = column.name.clone();
                    let data_type = column.data_type.clone();
                    let sortable = column.sortable && !header_locked && connected;
                    let active_sort = sort.as_ref().filter(|sort| sort.column == column.name);
                    h_flex()
                        .id(SharedString::from(format!("thead-{column_index}")))
                        .relative()
                        .w(px(column_widths
                            .get(column_index)
                            .copied()
                            .unwrap_or(CELL_WIDTH)))
                        .flex_shrink_0()
                        .items_center()
                        .gap_1()
                        .px_2()
                        .py_2()
                        .border_l_1()
                        .border_r_1()
                        .border_color(palette.muted.opacity(0.35))
                        .overflow_hidden()
                        .when(sortable, |this| this.cursor_pointer())
                        .when(!sortable, |this| this.opacity(0.65))
                        .hover(|style| style.bg(palette.panel_alt))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let resized = this.take_column_resized(tab_id);
                            if sortable && !resized {
                                this.request_sort(tab_id, column_index, cx);
                            }
                        }))
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.end_column_resize(tab_id, cx);
                            }),
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                this.begin_column_resize(
                                    tab_id,
                                    column_index,
                                    f32::from(event.position.x),
                                    cx,
                                );
                            }),
                        )
                        .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                            if event.dragging() {
                                let pointer_x = f32::from(event.position.x);
                                this.resize_column(tab_id, column_index, pointer_x, pointer_x, cx);
                            }
                        }))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w(px(0.0))
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(palette.foreground)
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .child(name),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .font_family(mono.clone())
                                        .text_color(palette.muted)
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .child(data_type),
                                ),
                        )
                        .when_some(active_sort, |this, sort| {
                            this.child(
                                gpui_kit::component::Icon::new(if sort.descending {
                                    IconName::ArrowDown
                                } else {
                                    IconName::ArrowUp
                                })
                                .with_size(Size::Small)
                                .text_color(palette.accent),
                            )
                        })
                        .child(
                            div()
                                .id(SharedString::from(format!(
                                    "column-resize-{tab_id}-{column_index}"
                                )))
                                .self_stretch()
                                .w(px(9.0))
                                .flex_shrink_0()
                                .border_l_1()
                                .border_color(palette.muted.opacity(0.35))
                                .occlude()
                                .cursor(CursorStyle::ResizeLeftRight)
                                .hover(|style| style.bg(palette.accent_soft))
                                .child(div().w(px(1.0)).self_stretch().bg(palette.border))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                        cx.stop_propagation();
                                        this.begin_column_resize(
                                            tab_id,
                                            column_index,
                                            f32::from(event.position.x),
                                            cx,
                                        );
                                    }),
                                )
                                .on_click(cx.listener(|_, _, _, cx| {
                                    cx.stop_propagation();
                                }))
                                .on_mouse_up(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        this.end_column_resize(tab_id, cx);
                                    }),
                                ),
                        )
                }),
        );

    let insert_count = staged
        .iter()
        .filter(|change| matches!(change, StagedChange::Insert(_)))
        .count();
    let row_count = page.rows.len() + insert_count;

    // Only the rows inside the viewport are built; the uniform list measures
    // `ROW_HEIGHT` from the first row and computes the scroll range from it.
    let body: AnyElement = if row_count == 0 {
        v_flex()
            .flex_1()
            .min_h(px(0.0))
            .py_4()
            .pr_4()
            .pl(px(ROW_RAIL_WIDTH + 16.0))
            .text_xs()
            .text_color(palette.muted)
            .child("No rows in this table.")
            .into_any_element()
    } else {
        let scroll_handle = table_scroll_handle(tab_id);
        uniform_list(
            ("table-rows", tab_id),
            row_count,
            cx.processor(
                move |_app,
                      range: Range<usize>,
                      _window: &mut Window,
                      cx: &mut Context<KoyoApp>| {
                    range
                        .map(|row_index| {
                            render_virtual_row(
                                tab_id,
                                &page,
                                &staged,
                                row_index,
                                editable,
                                palette,
                                &mono,
                                column_widths.as_slice(),
                                cx,
                            )
                        })
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
                .id(("table-hscroll", tab_id))
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.0))
                .overflow_x_scroll()
                .child(
                    v_flex()
                        .w(px(width))
                        .min_w(px(width))
                        .flex_1()
                        .min_h(px(0.0))
                        .child(header)
                        .child(body),
                ),
        )
        .into_any_element()
}

/// Render one virtual row. Database rows come first and staged inserts follow
/// them, matching the order the grid used before it was virtualized.
fn render_virtual_row(
    tab_id: u64,
    page: &TablePage,
    staged: &[StagedChange],
    row_index: usize,
    editable: bool,
    palette: Palette,
    mono: &SharedString,
    column_widths: &[f32],
    cx: &mut Context<KoyoApp>,
) -> AnyElement {
    if let Some(row) = page.rows.get(row_index) {
        return render_data_row(
            tab_id,
            page,
            staged,
            row_index,
            row,
            editable,
            palette,
            mono,
            column_widths,
            cx,
        );
    }

    let insert_index = row_index.saturating_sub(page.rows.len());
    let insert = staged
        .iter()
        .filter_map(|change| match change {
            StagedChange::Insert(insert) => Some(insert),
            _ => None,
        })
        .nth(insert_index);
    match insert {
        Some(insert) => render_insert_row(
            tab_id,
            page,
            insert,
            editable,
            palette,
            mono,
            column_widths,
            cx,
        ),
        None => div().h(px(ROW_HEIGHT)).into_any_element(),
    }
}

fn render_data_row(
    tab_id: u64,
    page: &TablePage,
    staged: &[StagedChange],
    row_index: usize,
    row: &GridRow,
    editable: bool,
    palette: Palette,
    mono: &SharedString,
    column_widths: &[f32],
    cx: &mut Context<KoyoApp>,
) -> AnyElement {
    let key = &row.key;
    let pending = staged.iter().find(|change| match change {
        StagedChange::Update {
            key: staged_key, ..
        }
        | StagedChange::Delete {
            key: staged_key, ..
        } => staged_key == key,
        StagedChange::Insert(_) => false,
    });
    let deleted = matches!(pending, Some(StagedChange::Delete { .. }));
    let update_values = pending.and_then(|change| match change {
        StagedChange::Update { values, .. } => Some(values),
        _ => None,
    });

    let mut row_view = h_flex()
        .id(SharedString::from(format!(
            "table-row-{tab_id}-{row_index}"
        )))
        .flex_shrink_0()
        .h(px(ROW_HEIGHT))
        .border_b_1()
        .border_color(palette.border)
        .font_family(mono.clone())
        .when(update_values.is_some(), |this| this.bg(palette.accent_soft))
        .when(deleted, |this| this.opacity(0.45))
        .when(update_values.is_none() && !deleted, |this| {
            // Quiet row wash; the stronger cell hover still wins over it.
            this.hover(|style| style.bg(palette.panel))
        })
        .child(
            div()
                .w(px(ROW_RAIL_WIDTH))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_end()
                .px_1p5()
                .text_xs()
                .text_color(palette.muted)
                .border_r_1()
                .border_color(palette.border)
                .child((page.offset as usize + row_index + 1).to_string()),
        );

    if editable {
        row_view = row_view.child(
            div()
                .w(px(ACTION_WIDTH))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .border_r_1()
                .border_color(palette.border)
                .child(
                    Button::new(SharedString::from(format!("delete-row-{row_index}")))
                        .ghost()
                        .compact()
                        .icon(if deleted {
                            IconName::Undo2
                        } else {
                            IconName::Delete
                        })
                        .tooltip(if deleted {
                            "Undo delete"
                        } else {
                            "Stage delete"
                        })
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.toggle_row_delete(tab_id, row_index, window, cx);
                        })),
                ),
        );
    }

    for (column_index, column) in page.columns.iter().enumerate() {
        let value = update_values
            .and_then(|values| values.get(&column.name).cloned())
            .unwrap_or_else(|| row.values[column_index].clone());
        let can_edit = editable && !deleted && !column.generated && !column.identity;
        row_view = row_view.child(
            div()
                .id(SharedString::from(format!(
                    "cell-{row_index}-{column_index}"
                )))
                .w(px(column_widths
                    .get(column_index)
                    .copied()
                    .unwrap_or(CELL_WIDTH)))
                .flex_shrink_0()
                .px_2()
                .py_1()
                .border_l_1()
                .border_r_1()
                .border_color(palette.border)
                .overflow_hidden()
                .text_ellipsis()
                .when(can_edit, |this| {
                    this.cursor_pointer()
                        .hover(|style| style.bg(palette.panel_alt))
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    if can_edit {
                        this.open_cell_editor(tab_id, row_index, column_index, window, cx);
                    }
                }))
                .child(value_text(&value, &palette)),
        );
    }

    row_view.into_any_element()
}

fn render_insert_row(
    tab_id: u64,
    page: &TablePage,
    insert: &StagedInsert,
    editable: bool,
    palette: Palette,
    mono: &SharedString,
    column_widths: &[f32],
    cx: &mut Context<KoyoApp>,
) -> AnyElement {
    let local_id = insert.local_id;
    let mut row_view = h_flex()
        .flex_shrink_0()
        .h(px(ROW_HEIGHT))
        .border_b_1()
        .border_color(palette.border)
        .bg(palette.accent_soft)
        .font_family(mono.clone())
        .child(
            // Inserted rows follow the page, so a number would be a lie; the
            // rail marks them as new instead.
            div()
                .w(px(ROW_RAIL_WIDTH))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_end()
                .px_1p5()
                .text_xs()
                .text_color(palette.accent)
                .border_r_1()
                .border_color(palette.border)
                .child("new"),
        );

    if editable {
        row_view = row_view.child(
            div()
                .w(px(ACTION_WIDTH))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .border_r_1()
                .border_color(palette.border)
                .child(
                    Button::new(SharedString::from(format!("discard-insert-{local_id}")))
                        .ghost()
                        .compact()
                        .icon(IconName::Delete)
                        .tooltip("Discard new row")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.discard_insert(tab_id, local_id, cx);
                        })),
                ),
        );
    }

    for (column_index, column) in page.columns.iter().enumerate() {
        let generated = column.generated || column.identity;
        let value = insert.values.get(&column.name).cloned();
        let (label, color) = if generated {
            ("Generated".to_string(), palette.muted)
        } else {
            match &value {
                None => ("Default".to_string(), palette.muted),
                Some(None) => ("NULL".to_string(), palette.muted),
                Some(Some(value)) if value.is_empty() => ("empty".to_string(), palette.foreground),
                Some(Some(value)) => (value.clone(), palette.foreground),
            }
        };
        row_view = row_view.child(
            div()
                .id(SharedString::from(format!(
                    "insert-cell-{local_id}-{column_index}"
                )))
                .w(px(column_widths
                    .get(column_index)
                    .copied()
                    .unwrap_or(CELL_WIDTH)))
                .flex_shrink_0()
                .px_2()
                .py_1()
                .border_l_1()
                .border_r_1()
                .border_color(palette.border)
                .overflow_hidden()
                .when(!generated, |this| {
                    this.cursor_pointer()
                        .hover(|style| style.bg(palette.panel_alt))
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    if !generated {
                        this.open_insert_cell_editor(tab_id, local_id, column_index, window, cx);
                    }
                }))
                .child(
                    div()
                        .text_sm()
                        .text_color(color)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(label),
                ),
        );
    }

    row_view.into_any_element()
}

fn relation_card(
    schema: &str,
    table: &str,
    column: &str,
    palette: &Palette,
    highlight: bool,
) -> AnyElement {
    v_flex()
        .w(px(210.0))
        .flex_shrink_0()
        .gap_1()
        .p_3()
        .rounded_lg()
        .border_1()
        .border_color(if highlight {
            palette.accent
        } else {
            palette.border
        })
        .bg(if highlight {
            palette.accent_soft
        } else {
            palette.panel
        })
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(format!("{schema}.{table}")),
        )
        .child(
            div()
                .text_xs()
                .text_color(palette.muted)
                .child(column.to_string()),
        )
        .into_any_element()
}

fn render_diagram(
    tab_id: u64,
    table: &super::TableTab,
    page: Option<&TablePage>,
    palette: &Palette,
) -> AnyElement {
    let mut content = v_flex()
        .id(("table-diagram", tab_id))
        .flex_1()
        .min_h(px(0.0))
        .overflow_y_scroll()
        .p_5()
        .gap_4();
    if let Some(page) = page {
        let mut columns = v_flex().gap_1();
        for column in page.columns.iter().take(12) {
            columns = columns.child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(180.0))
                            .text_sm()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(column.name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.muted)
                            .child(column.data_type.clone()),
                    )
                    .when(column.primary_key, |this| {
                        this.child(
                            div()
                                .px_1()
                                .rounded_sm()
                                .bg(palette.panel_alt)
                                .text_xs()
                                .text_color(palette.warning)
                                .child("PK"),
                        )
                    }),
            );
        }
        content = content.child(
            v_flex()
                .gap_2()
                .p_4()
                .rounded_lg()
                .border_1()
                .border_color(palette.accent)
                .bg(palette.panel)
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("{}.{}", table.schema, table.table)),
                )
                .child(columns),
        );
    }

    let title = h_flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child("Relationships"),
        )
        .child(
            div()
                .text_xs()
                .text_color(palette.muted)
                .child(format!("{}", table.foreign_keys.len())),
        );
    content = content.child(title);

    if table.foreign_keys_loading {
        content = content.child(
            div()
                .text_xs()
                .text_color(palette.muted)
                .child("Loading relationships..."),
        );
    } else if table.foreign_keys.is_empty() {
        content = content.child(
            div()
                .p_4()
                .rounded_lg()
                .border_1()
                .border_color(palette.border)
                .bg(palette.panel)
                .text_xs()
                .text_color(palette.muted)
                .child("No foreign keys point at or from this table."),
        );
    } else {
        for fk in &table.foreign_keys {
            let outgoing = fk.schema == table.schema && fk.table == table.table;
            let (left_schema, left_table, left_column) = if outgoing {
                (fk.schema.as_str(), fk.table.as_str(), fk.column.as_str())
            } else {
                (
                    fk.referenced_schema.as_str(),
                    fk.referenced_table.as_str(),
                    fk.referenced_column.as_str(),
                )
            };
            let (right_schema, right_table, right_column) = if outgoing {
                (
                    fk.referenced_schema.as_str(),
                    fk.referenced_table.as_str(),
                    fk.referenced_column.as_str(),
                )
            } else {
                (fk.schema.as_str(), fk.table.as_str(), fk.column.as_str())
            };
            content = content.child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(relation_card(
                        left_schema,
                        left_table,
                        left_column,
                        palette,
                        outgoing,
                    ))
                    .child(
                        v_flex()
                            .w(px(90.0))
                            .flex_shrink_0()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .w_full()
                                    .text_xs()
                                    .text_color(palette.muted)
                                    .text_center()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(fk.constraint_name.clone()),
                            )
                            .child(
                                h_flex()
                                    .items_center()
                                    .w_full()
                                    .child(div().flex_1().h(px(1.0)).bg(palette.border))
                                    .child(
                                        Icon::new(IconName::ChevronRight)
                                            .with_size(Size::Small)
                                            .text_color(palette.muted),
                                    ),
                            ),
                    )
                    .child(relation_card(
                        right_schema,
                        right_table,
                        right_column,
                        palette,
                        !outgoing,
                    )),
            );
        }
    }

    content
        .child(
            div()
                .text_xs()
                .text_color(palette.muted)
                .child("Drag the Results divider or this pane split to resize."),
        )
        .into_any_element()
}
