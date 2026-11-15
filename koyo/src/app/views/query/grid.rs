use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use gpui_kit::{CursorStyle, InteractiveElement, StatefulInteractiveElement};
use koyo_core::models::QueryResultSet;
use std::ops::Range;
use std::rc::Rc;

use super::super::widgets::value_text;
use super::{ROW_HEIGHT, result_scroll_handle};
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
    let width = RESULT_ROW_RAIL_WIDTH + widths.iter().sum::<f32>();
    let palette = *palette;

    let header = h_flex()
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
        let widths_for_rows = Rc::clone(&widths);
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
                      cx: &mut Context<KoyoApp>| {
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
                        .map(|row_index| {
                            render_result_row(
                                tab_id,
                                result_index,
                                result,
                                row_index,
                                widths_for_rows.as_slice(),
                                palette,
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
                .id(("result-hscroll", tab_id))
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
                        .child(header)
                        .child(rows),
                ),
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
    cx: &mut Context<KoyoApp>,
) -> AnyElement {
    let Some(row) = result.rows.get(row_index) else {
        return div().h(px(ROW_HEIGHT)).into_any_element();
    };
    let row_text = (0..result.columns.len())
        .map(|index| row.get(index).and_then(Option::as_deref).unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\t");
    h_flex()
        .id(SharedString::from(format!(
            "result-row-{tab_id}-{result_index}-{row_index}"
        )))
        .flex_shrink_0()
        .h(px(ROW_HEIGHT))
        .border_b_1()
        .border_color(palette.border)
        .when(row_index % 2 == 1, |this| this.bg(palette.panel))
        .child(
            div()
                .id(SharedString::from(format!(
                    "result-row-rail-{tab_id}-{result_index}-{row_index}"
                )))
                .w(px(RESULT_ROW_RAIL_WIDTH))
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
                    this.copy_to_clipboard(row_text.clone(), "Copied row.", cx);
                }))
                .child((row_index + 1).to_string()),
        )
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
        .into_any_element()
}
