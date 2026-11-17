use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::{ActiveTheme, IconName, Sizable, Size};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use gpui_kit::{CursorStyle, InteractiveElement, StatefulInteractiveElement};
use koyo_core::models::{GridRow, TablePage, TableSort};
use std::rc::Rc;

use super::super::widgets::value_text;
use super::{CELL_WIDTH, ROW_HEIGHT, ROW_RAIL_WIDTH, table_scroll_handle};
use crate::app::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn render_grid(
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
    let column_widths = Rc::new(column_widths.to_vec());
    let columns_width = column_widths.iter().sum::<f32>();
    let width = ROW_RAIL_WIDTH + columns_width;
    // Header affordances must not dim while a sort/page load is in flight;
    // only staged edits and missing connections make a column unsortable.
    let header_locked = !staged.is_empty();
    // `Palette` is `Copy`; the virtual list row closure must be `'static`.
    let palette = *palette;
    // Monospace data is part of the read of the grid; take it once here so the
    // virtual row closure and the header share the same face.
    let mono = cx.theme().mono_font_family.clone();
    let scroll_handle = table_scroll_handle(tab_id);
    // Use the same clamped range as the body scroller. Raw wheel deltas can
    // overshoot the real scroll range, which otherwise makes the header drift
    // out of alignment. Clamp both edges.
    let max_left = -f32::from(scroll_handle.max_offset().x);
    let visible_x = f32::from(scroll_handle.offset().x).clamp(max_left, 0.0);
    let header_left = px(visible_x);
    let frozen_left = px(-visible_x);
    let header = h_flex()
        .w(px(columns_width))
        .min_w(px(columns_width))
        .flex_shrink_0()
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
                                .absolute()
                                .top_0()
                                .bottom_0()
                                .right_0()
                                .w(px(6.0))
                                .occlude()
                                .cursor(CursorStyle::ResizeLeftRight)
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                        cx.stop_propagation();
                                        if event.click_count >= 2 {
                                            this.autosize_column(tab_id, column_index, cx);
                                        } else {
                                            this.begin_column_resize(
                                                tab_id,
                                                column_index,
                                                f32::from(event.position.x),
                                                cx,
                                            );
                                        }
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

    // A table page is capped at 200 rows, so one two-axis scroller can own the
    // rows without a nested axis-locked list.
    let body: AnyElement = if row_count == 0 {
        v_flex()
            .w(px(width))
            .min_w(px(width))
            .py_4()
            .pr_4()
            .pl(px(ROW_RAIL_WIDTH + 16.0))
            .text_xs()
            .text_color(palette.muted)
            .child("No rows in this table.")
            .into_any_element()
    } else {
        v_flex()
            .w(px(width))
            .min_w(px(width))
            .children((0..row_count).map(|row_index| {
                render_virtual_row(
                    tab_id,
                    &page,
                    &staged,
                    row_index,
                    editable,
                    palette,
                    &mono,
                    column_widths.as_slice(),
                    frozen_left,
                    cx,
                )
            }))
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
                .flex()
                .w_full()
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
                .child(
                    div()
                        .id(("table-header-clip", tab_id))
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .child(
                            div()
                                .relative()
                                .left(header_left)
                                .w(px(columns_width))
                                .child(header),
                        ),
                ),
        )
        .child(
            div()
                .id(("table-body", tab_id))
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.0))
                .overflow_scroll()
                .restrict_scroll_to_axis()
                .track_scroll(&scroll_handle)
                .child(body),
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
    frozen_left: Pixels,
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
            frozen_left,
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
            palette,
            mono,
            column_widths,
            frozen_left,
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
    frozen_left: Pixels,
    cx: &mut Context<KoyoApp>,
) -> AnyElement {
    let key = &row.key;
    let update_values = staged.iter().find_map(|change| match change {
        StagedChange::Update {
            key: staged_key,
            values,
            ..
        } if staged_key == key => Some(values),
        _ => None,
    });
    let copy_row_text = Rc::new(row_copy_text(page, staged, row_index));

    let row_text_for_rail = Rc::clone(&copy_row_text);
    let mut row_view = h_flex()
        .relative()
        .id(SharedString::from(format!(
            "table-row-{tab_id}-{row_index}"
        )))
        .flex_shrink_0()
        .h(px(ROW_HEIGHT))
        .border_b_1()
        .border_color(palette.border)
        .font_family(mono.clone())
        .when(update_values.is_some(), |this| this.bg(palette.accent_soft))
        .when(update_values.is_none(), |this| {
            // Quiet row wash; the stronger cell hover still wins over it.
            this.hover(|style| style.bg(palette.panel))
        })
        .child(div().w(px(ROW_RAIL_WIDTH)).flex_shrink_0());

    for (column_index, column) in page.columns.iter().enumerate() {
        let value = update_values
            .and_then(|values| values.get(&column.name).cloned())
            .unwrap_or_else(|| row.values[column_index].clone());
        let can_edit = editable
            && !column.generated
            && !column.identity
            && !row.truncated.get(column_index).copied().unwrap_or(false);
        let copy_value = value.clone().unwrap_or_default();
        let copy_row = Rc::clone(&copy_row_text);
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
                .context_menu(move |menu, _, _| {
                    let cell = copy_value.clone();
                    let row = copy_row.clone();
                    menu.item(PopupMenuItem::new("Copy value").on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(cell.clone()));
                    }))
                    .item(PopupMenuItem::new("Copy row").on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(row.to_string()));
                    }))
                })
                .child(value_text(&value, &palette)),
        );
    }

    row_view = row_view.child(
        div()
            .id(SharedString::from(format!(
                "table-row-rail-{tab_id}-{row_index}"
            )))
            .absolute()
            .top_0()
            .bottom_0()
            .left(frozen_left)
            .w(px(ROW_RAIL_WIDTH))
            .flex()
            .items_center()
            .justify_end()
            .px_1p5()
            .bg(palette.background)
            .text_xs()
            .text_color(palette.muted)
            .border_r_1()
            .border_color(palette.border)
            .cursor_pointer()
            .hover(|style| style.bg(palette.panel_alt))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.copy_to_clipboard(row_text_for_rail.to_string(), "Copied row.", cx);
            }))
            .child((page.offset as usize + row_index + 1).to_string()),
    );

    row_view.into_any_element()
}

fn row_copy_text(page: &TablePage, staged: &[StagedChange], row_index: usize) -> String {
    let Some(row) = page.rows.get(row_index) else {
        return String::new();
    };
    let update_values = staged.iter().find_map(|change| match change {
        StagedChange::Update {
            key: staged_key,
            values,
            ..
        } if staged_key == &row.key => Some(values),
        _ => None,
    });
    page.columns
        .iter()
        .enumerate()
        .map(|(column_index, column)| {
            update_values
                .and_then(|values| values.get(&column.name).cloned())
                .unwrap_or_else(|| row.values[column_index].clone())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join("\t")
}

fn render_insert_row(
    tab_id: u64,
    page: &TablePage,
    insert: &StagedInsert,
    palette: Palette,
    mono: &SharedString,
    column_widths: &[f32],
    frozen_left: Pixels,
    cx: &mut Context<KoyoApp>,
) -> AnyElement {
    let local_id = insert.local_id;
    let mut row_view = h_flex()
        .relative()
        .flex_shrink_0()
        .h(px(ROW_HEIGHT))
        .border_b_1()
        .border_color(palette.border)
        .bg(palette.accent_soft)
        .font_family(mono.clone())
        .child(div().w(px(ROW_RAIL_WIDTH)).flex_shrink_0());

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

    row_view = row_view.child(
        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(frozen_left)
            .w(px(ROW_RAIL_WIDTH))
            .flex()
            .items_center()
            .justify_end()
            .px_1p5()
            .bg(palette.accent_soft)
            .text_xs()
            .text_color(palette.accent)
            .border_r_1()
            .border_color(palette.border)
            .child("new"),
    );

    row_view.into_any_element()
}
