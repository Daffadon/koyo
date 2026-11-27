use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::{Disableable, IconName, Sizable};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::super::widgets::button_tooltip;
use crate::app::*;

pub(super) fn render_filter_bar(
    tab_id: u64,
    table: &TableTab,
    connected: bool,
    palette: &Palette,
    cx: &mut Context<KoyoApp>,
) -> AnyElement {
    let locked = table.loading || !table.staged.is_empty() || !connected;
    let clear_disabled = !connected || table.loading;
    let (filter_value, filter_cursor) = {
        let input = table.filter_input.read(cx);
        (input.value().to_string(), input.cursor())
    };
    let suggestions = table
        .page
        .as_ref()
        .map(|page| filter_suggestions(&page.columns, &filter_value, filter_cursor))
        .unwrap_or_default();
    let show_suggestions = !locked && !filter_value.trim().is_empty() && !suggestions.is_empty();
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
            .child(div().text_xs().text_color(palette.muted).child("Filter"))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .key_context("TableFilter")
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                        if event.keystroke.key == "enter" && !event.keystroke.modifiers.shift {
                            cx.stop_propagation();
                            this.apply_filter(tab_id, cx);
                        }
                    }))
                    .child(
                        Input::new(&table.filter_input)
                            .aria_label("Filter table rows")
                            .w_full()
                            .disabled(locked)
                            .cleanable(true)
                            .prefix(
                                gpui_kit::component::Icon::new(IconName::Search)
                                    .with_size(gpui_kit::component::Size::Small)
                                    .text_color(palette.muted),
                            ),
                    ),
            )
            .child(
                Button::new(("apply-filter", tab_id))
                    .primary()
                    .compact()
                    .label("Apply")
                    .disabled(locked)
                    .when(locked, |this| this.text_color(palette.foreground))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.apply_filter(tab_id, cx);
                    })),
            )
            .child(button_tooltip(
                Button::new(("clear-filters", tab_id))
                    .ghost()
                    .compact()
                    .icon(IconName::Close)
                    .disabled(clear_disabled)
                    .when(clear_disabled, |this| this.text_color(palette.foreground))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.request_clear_filters(tab_id, window, cx);
                    })),
                "Clear filter",
            )),
    );

    if show_suggestions {
        bar = bar.child(
            h_flex().flex_wrap().items_center().gap_1().children(
                suggestions
                    .into_iter()
                    .enumerate()
                    .map(|(index, suggestion)| {
                        let insert = suggestion.insert;
                        Button::new(SharedString::from(format!(
                            "filter-suggestion-{tab_id}-{index}"
                        )))
                        .secondary()
                        .compact()
                        .label(suggestion.label)
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.complete_filter_input(tab_id, insert.clone(), window, cx);
                            },
                        ))
                    }),
            ),
        );
    }

    let hint = if !table.filter_error.is_empty() {
        Some((table.filter_error.clone(), palette.danger))
    } else if !table.staged.is_empty() {
        Some((
            "Apply or discard staged edits to change filters.".to_string(),
            palette.muted,
        ))
    } else if !table.filters.is_empty() {
        Some((
            format!("{} condition(s) applied", table.filters.len()),
            palette.muted,
        ))
    } else {
        None
    };
    if let Some((message, color)) = hint {
        bar = bar.child(div().text_xs().text_color(color).child(message));
    }

    bar.into_any_element()
}
