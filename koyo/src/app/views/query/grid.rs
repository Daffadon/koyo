use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use gpui_kit::{InteractiveElement, StatefulInteractiveElement};
use koyo_core::models::QueryResultSet;
use std::ops::Range;

use super::super::widgets::value_text;
use super::{ROW_HEIGHT, result_scroll_handle};
use crate::app::*;

pub(super) fn result_grid(
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
