use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::{ActiveTheme, IconName, Sizable, Size};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use gpui_kit::{CursorStyle, InteractiveElement, StatefulInteractiveElement};
use koyo_core::models::{GridRow, TablePage, TableSort};
use std::ops::Range;
use std::rc::Rc;

use super::super::widgets::{button_tooltip, value_text};
use super::{ACTION_WIDTH, CELL_WIDTH, ROW_HEIGHT, ROW_RAIL_WIDTH, table_scroll_handle};
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
    let copy_row_text = Rc::new(
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
            .join("\t"),
    );

    let row_text_for_rail = Rc::clone(&copy_row_text);
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
                .id(SharedString::from(format!(
                    "table-row-rail-{tab_id}-{row_index}"
                )))
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
                .cursor_pointer()
                .hover(|style| style.bg(palette.panel_alt))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.copy_to_clipboard(row_text_for_rail.to_string(), "Copied row.", cx);
                }))
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
                .child(button_tooltip(
                    ("tooltip-delete-row", row_index),
                    Button::new(SharedString::from(format!("delete-row-{row_index}")))
                        .ghost()
                        .compact()
                        .icon(if deleted {
                            IconName::Undo2
                        } else {
                            IconName::Delete
                        })
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.toggle_row_delete(tab_id, row_index, window, cx);
                        })),
                    if deleted {
                        "Undo delete"
                    } else {
                        "Stage delete"
                    },
                )),
        );
    }

    for (column_index, column) in page.columns.iter().enumerate() {
        let value = update_values
            .and_then(|values| values.get(&column.name).cloned())
            .unwrap_or_else(|| row.values[column_index].clone());
        let can_edit = editable && !deleted && !column.generated && !column.identity;
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
                .child(button_tooltip(
                    ("tooltip-discard-insert", local_id),
                    Button::new(SharedString::from(format!("discard-insert-{local_id}")))
                        .ghost()
                        .compact()
                        .icon(IconName::Delete)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.discard_insert(tab_id, local_id, cx);
                        })),
                    "Discard new row",
                )),
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
