use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use gpui_kit::{CursorStyle, InteractiveElement, StatefulInteractiveElement};
use koyo_core::models::QueryResultSet;
use std::rc::Rc;

use super::super::widgets::value_text;
use super::{ROW_HEIGHT, result_grid_scroll_handle};
use crate::app::*;

pub(super) fn result_grid(
    result: &QueryResultSet,
    tab_id: u64,
    result_index: usize,
    column_widths: &[f32],
    palette: &Palette,
    mono: &SharedString,
    cx: &mut Context<KoyoApp>,
) -> AnyElement {
    let widths = Rc::new(
        (0..result.columns.len())
            .map(|index| {
                column_widths
                    .get(index)
                    .copied()
                    .unwrap_or(RESULT_COLUMN_WIDTH)
            })
            .collect::<Vec<_>>(),
    );
    let columns_width = widths.iter().sum::<f32>();
    let width = RESULT_ROW_RAIL_WIDTH + columns_width;
    let palette = *palette;
    let scroll_handle = result_grid_scroll_handle(tab_id, result_index);
    let max_left = -f32::from(scroll_handle.max_offset().x);
    let visible_x = f32::from(scroll_handle.offset().x).clamp(max_left, 0.0);
    let header_left = px(visible_x);
    let frozen_left = px(-visible_x);

    let header = h_flex()
        .w(px(columns_width))
        .min_w(px(columns_width))
        .flex_shrink_0()
        .children(result.columns.iter().enumerate().map(|(index, column)| {
            let column_width = widths.get(index).copied().unwrap_or(RESULT_COLUMN_WIDTH);
            h_flex()
                .id(SharedString::from(format!("result-head-{index}")))
                .relative()
                .w(px(column_width))
                .flex_shrink_0()
                .items_center()
                .px_2()
                .py_2()
                .border_l_1()
                .border_r_1()
                .border_color(palette.muted.opacity(0.35))
                .overflow_hidden()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(palette.muted)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(column.clone()),
                )
                .child(
                    div()
                        .id(SharedString::from(format!(
                            "result-column-resize-{tab_id}-{result_index}-{index}"
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
                                this.begin_result_column_resize(
                                    tab_id,
                                    result_index,
                                    index,
                                    f32::from(event.position.x),
                                    cx,
                                );
                            }),
                        )
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.end_result_column_resize(tab_id, cx);
                            }),
                        )
                        .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                            if event.dragging() {
                                let pointer_x = f32::from(event.position.x);
                                this.resize_result_column(
                                    tab_id,
                                    result_index,
                                    index,
                                    pointer_x,
                                    pointer_x,
                                    cx,
                                );
                            }
                        }))
                        .on_click(cx.listener(|_, _, _, cx| {
                            cx.stop_propagation();
                        })),
                )
        }));

    let row_count = result.rows.len();

    // Result sets are capped at 1,000 rows. Render only a window around the
    // current scroll offset so a wide result does not build tens of thousands
    // of cell elements on every frame.
    let rows: AnyElement = if row_count == 0 {
        v_flex()
            .w(px(width))
            .min_w(px(width))
            .p_4()
            .text_xs()
            .text_color(palette.muted)
            .child("No rows returned.")
            .into_any_element()
    } else {
        let offset_y = f32::from(scroll_handle.offset().y);
        let first = ((-offset_y / ROW_HEIGHT).floor() as usize)
            .saturating_sub(8)
            .min(row_count);
        let last = (first + 160).min(row_count);
        div()
            .relative()
            .w(px(width))
            .min_w(px(width))
            .h(px(row_count as f32 * ROW_HEIGHT))
            .children((first..last).map(|row_index| {
                div()
                    .absolute()
                    .top(px(row_index as f32 * ROW_HEIGHT))
                    .left_0()
                    .w_full()
                    .child(render_result_row(
                        tab_id,
                        result_index,
                        result,
                        row_index,
                        widths.as_slice(),
                        palette,
                        frozen_left,
                        cx,
                    ))
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
            h_flex()
                .w_full()
                .flex_shrink_0()
                .bg(palette.panel)
                .border_b_1()
                .border_color(palette.border)
                .child(
                    div()
                        .w(px(RESULT_ROW_RAIL_WIDTH))
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
                        .id(("result-header-clip", tab_id))
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
                .id(("result-body", tab_id))
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.0))
                .overflow_scroll()
                .restrict_scroll_to_axis()
                .track_scroll(&scroll_handle)
                .child(rows),
        )
        .font_family(mono.clone())
        .into_any_element()
}

fn render_result_row(
    tab_id: u64,
    result_index: usize,
    result: &QueryResultSet,
    row_index: usize,
    widths: &[f32],
    palette: Palette,
    frozen_left: Pixels,
    cx: &mut Context<KoyoApp>,
) -> AnyElement {
    let Some(row) = result.rows.get(row_index) else {
        return div().h(px(ROW_HEIGHT)).into_any_element();
    };
    let row_text = (0..result.columns.len())
        .map(|index| row.get(index).and_then(Option::as_deref).unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\t");
    let row_bg = if row_index % 2 == 1 {
        palette.panel
    } else {
        palette.background
    };
    h_flex()
        .relative()
        .id(SharedString::from(format!(
            "result-row-{tab_id}-{result_index}-{row_index}"
        )))
        .flex_shrink_0()
        .h(px(ROW_HEIGHT))
        .border_b_1()
        .border_color(palette.border)
        .when(row_index % 2 == 1, |this| this.bg(palette.panel))
        .child(div().w(px(RESULT_ROW_RAIL_WIDTH)).flex_shrink_0())
        .children((0..result.columns.len()).map(|column_index| {
            let value = row.get(column_index).cloned().unwrap_or(None);
            let copy_text = value.clone().unwrap_or_default();
            let column_width = widths
                .get(column_index)
                .copied()
                .unwrap_or(RESULT_COLUMN_WIDTH);
            div()
                .id(SharedString::from(format!(
                    "result-cell-{tab_id}-{result_index}-{row_index}-{column_index}"
                )))
                .w(px(column_width))
                .flex_shrink_0()
                .px_2()
                .py_1()
                .border_l_1()
                .border_r_1()
                .border_color(palette.border)
                .overflow_hidden()
                .cursor_pointer()
                .hover(|style| style.bg(palette.panel_alt))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.copy_to_clipboard(copy_text.clone(), "Copied cell.", cx);
                }))
                .child(value_text(&value, &palette))
        }))
        .child(
            div()
                .id(SharedString::from(format!(
                    "result-row-rail-{tab_id}-{result_index}-{row_index}"
                )))
                .absolute()
                .top_0()
                .bottom_0()
                .left(frozen_left)
                .w(px(RESULT_ROW_RAIL_WIDTH))
                .flex()
                .items_center()
                .justify_end()
                .px_1p5()
                .bg(row_bg)
                .text_xs()
                .text_color(palette.muted)
                .border_r_1()
                .border_color(palette.border)
                .cursor_pointer()
                .hover(|style| style.bg(palette.panel_alt))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.copy_to_clipboard(row_text.clone(), "Copied row.", cx);
                }))
                .child((row_index + 1).to_string()),
        )
        .into_any_element()
}
