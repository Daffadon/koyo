use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use gpui_kit::{CursorStyle, InteractiveElement, StatefulInteractiveElement};
use koyo_core::models::QueryResultSet;
use std::rc::Rc;

use super::super::data_grid::{self, ROW_HEIGHT, ROW_RAIL_WIDTH};
use super::super::widgets::{control_focus, keyboard_row, value_text};
use super::result_grid_scroll_handle;
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
    let palette = *palette;
    let scroll_handle = result_grid_scroll_handle(tab_id, result_index);
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
                        .aria_label(format!("Resize column {column}"))
                        .track_focus(&control_focus(
                            tab_id,
                            format!("result-column-resize-{result_index}-{index}"),
                            cx,
                        ))
                        .tab_stop(true)
                        .focus(|style| style.bg(palette.accent_soft))
                        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                            match event.keystroke.key.as_str() {
                                "left" => this.nudge_result_column_width(
                                    tab_id,
                                    result_index,
                                    index,
                                    -16.0,
                                    cx,
                                ),
                                "right" => this.nudge_result_column_width(
                                    tab_id,
                                    result_index,
                                    index,
                                    16.0,
                                    cx,
                                ),
                                _ => {}
                            }
                        }))
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
                        .on_click(cx.listener(|_, _, _, cx| {
                            cx.stop_propagation();
                        })),
                )
        }))
        .into_any_element();

    let row_count = result.rows.len();
    let empty = v_flex()
        .p_4()
        .text_xs()
        .text_color(palette.muted)
        .child("No rows returned.")
        .into_any_element();

    data_grid::render(
        ("result-header-clip", tab_id),
        ("result-body", tab_id),
        columns_width,
        header,
        empty,
        row_count,
        &scroll_handle,
        mono.clone(),
        palette,
        |row_index, frozen_left| {
            render_result_row(
                tab_id,
                result_index,
                result,
                row_index,
                widths.as_slice(),
                palette,
                frozen_left,
                cx,
            )
        },
    )
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
    let row_text_for_keyboard = row_text.clone();
    h_flex()
        .relative()
        .id(SharedString::from(format!(
            "result-row-{tab_id}-{result_index}-{row_index}"
        )))
        .flex_shrink_0()
        .h(px(ROW_HEIGHT))
        .border_b_1()
        .border_color(palette.border)
        .hover(|style| style.bg(palette.panel))
        .child(div().w(px(ROW_RAIL_WIDTH)).flex_shrink_0())
        .children((0..result.columns.len()).map(|column_index| {
            let value = row.get(column_index).cloned().unwrap_or(None);
            let copy_text = value.clone().unwrap_or_default();
            let column_width = widths
                .get(column_index)
                .copied()
                .unwrap_or(RESULT_COLUMN_WIDTH);
            keyboard_row(
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
                    .aria_label(format!(
                        "Copy cell row {} column {}",
                        row_index + 1,
                        column_index + 1
                    ))
                    .cursor_pointer()
                    .hover(|style| style.bg(palette.panel_alt))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.copy_to_clipboard(copy_text.clone(), "Copied cell.", cx);
                    }))
                    .child(value_text(&value, &palette)),
                tab_id,
                format!("result-cell-{result_index}-{row_index}-{column_index}"),
                &palette,
                cx,
                move |this, _, cx| {
                    let copy_text = value.clone().unwrap_or_default();
                    this.copy_to_clipboard(copy_text, "Copied cell.", cx);
                },
            )
        }))
        .child(keyboard_row(
            div()
                .id(SharedString::from(format!(
                    "result-row-rail-{tab_id}-{result_index}-{row_index}"
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
                .aria_label(format!("Copy row {}", row_index + 1))
                .cursor_pointer()
                .hover(|style| style.bg(palette.panel_alt))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.copy_to_clipboard(row_text.clone(), "Copied row.", cx);
                }))
                .child((row_index + 1).to_string()),
            tab_id,
            format!("result-row-rail-{result_index}-{row_index}"),
            &palette,
            cx,
            move |this, _, cx| {
                this.copy_to_clipboard(row_text_for_keyboard.clone(), "Copied row.", cx);
            },
        ))
        .into_any_element()
}
