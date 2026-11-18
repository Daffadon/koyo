//! Shared geometry and scroll shell for read-only and editable data grids.
//!
//! Both the table explorer and query results use the same two-axis scroller,
//! header offset, row rail, and row window. Only the cell/row content differs.

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::ops::Range;

use crate::app::Palette;

/// Every grid row is forced to this height so one scroll gesture maps cleanly
/// on both axes. It matches `text_sm` line box + `py_1` + the 1px border.
pub(super) const ROW_HEIGHT: f32 = 32.0;
/// Narrow gutter that carries the global row number. It is pinned horizontally
/// but scrolls vertically with the rows.
pub(super) const ROW_RAIL_WIDTH: f32 = 56.0;

const OVERSCAN_ROWS: usize = 8;
const FALLBACK_VIEWPORT_ROWS: usize = 80;

pub(super) fn horizontal_offsets(scroll_handle: &ScrollHandle) -> (Pixels, Pixels) {
    // Raw wheel deltas can overshoot the real scroll range. Clamp so the
    // header and frozen rail do not drift away from the body.
    let max_left = -f32::from(scroll_handle.max_offset().x);
    let visible_x = f32::from(scroll_handle.offset().x).clamp(max_left, 0.0);
    (px(visible_x), px(-visible_x))
}

fn clamp_vertical_offset(scroll_handle: &ScrollHandle, row_count: usize) {
    let viewport_height = f32::from(scroll_handle.bounds().size.height);
    let max_scroll = (row_count as f32 * ROW_HEIGHT - viewport_height).max(0.0);
    let current = scroll_handle.offset();
    if f32::from(current.y) < -max_scroll {
        scroll_handle.set_offset(point(current.x, px(-max_scroll)));
    }
}

fn visible_rows(scroll_handle: &ScrollHandle, row_count: usize) -> Range<usize> {
    visible_range(
        f32::from(scroll_handle.offset().y),
        f32::from(scroll_handle.bounds().size.height),
        row_count,
    )
}

fn visible_range(offset_y: f32, viewport_height: f32, row_count: usize) -> Range<usize> {
    if row_count == 0 {
        return 0..0;
    }
    let visible = if viewport_height > 0.0 {
        (viewport_height / ROW_HEIGHT).ceil() as usize
    } else {
        FALLBACK_VIEWPORT_ROWS
    };
    let first = ((-offset_y / ROW_HEIGHT).floor().max(0.0) as usize)
        .saturating_sub(OVERSCAN_ROWS)
        .min(row_count.saturating_sub(1));
    let last = (first + visible + OVERSCAN_ROWS * 2).min(row_count);
    first..last
}

/// Render the shared grid shell. Rows are rendered as normal flow with spacer
/// rows above and below, so the scroll extent stays stable while only a small
/// window around the viewport owns elements.
#[allow(clippy::too_many_arguments)]
pub(super) fn render(
    clip_id: impl Into<ElementId>,
    body_id: impl Into<ElementId>,
    columns_width: f32,
    header: AnyElement,
    empty: AnyElement,
    row_count: usize,
    scroll_handle: &ScrollHandle,
    mono: SharedString,
    palette: Palette,
    mut render_row: impl FnMut(usize, Pixels) -> AnyElement,
) -> AnyElement {
    let width = ROW_RAIL_WIDTH + columns_width;
    // A page, filter, or result refresh can replace many rows with only a few.
    // The old scroll offset then points past the new content and leaves blank
    // space above the remaining rows, so clamp it before computing the window.
    clamp_vertical_offset(scroll_handle, row_count);
    let (visible_left, frozen_left) = horizontal_offsets(scroll_handle);
    let range = visible_rows(scroll_handle, row_count);
    let first = range.start;
    let last = range.end;

    let body: AnyElement = if row_count == 0 {
        v_flex()
            .w(px(width))
            .min_w(px(width))
            .child(empty)
            .into_any_element()
    } else {
        v_flex()
            .w(px(width))
            .min_w(px(width))
            .h(px(row_count as f32 * ROW_HEIGHT))
            .flex_shrink_0()
            .child(div().h(px(first as f32 * ROW_HEIGHT)).flex_shrink_0())
            .children((first..last).map(|row_index| render_row(row_index, frozen_left)))
            .child(
                div()
                    .h(px((row_count - last) as f32 * ROW_HEIGHT))
                    .flex_shrink_0(),
            )
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
                        .id(clip_id)
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .child(
                            div()
                                .relative()
                                .left(visible_left)
                                .w(px(columns_width))
                                .child(header),
                        ),
                ),
        )
        .child(
            div()
                .id(body_id)
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.0))
                .overflow_scroll()
                .restrict_scroll_to_axis()
                .track_scroll(scroll_handle)
                .font_family(mono)
                .child(body),
        )
        .into_any_element()
}
