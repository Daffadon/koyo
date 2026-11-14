use gpui_kit::StatefulInteractiveElement;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Disableable, IconName, Sizable, Size};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::{CellEdit, CellTarget, ConfirmState, KoyoApp, Palette};

pub(super) fn render_cell_editor(
    edit: &CellEdit,
    palette: &Palette,
    cx: &mut Context<KoyoApp>,
) -> AnyElement {
    let insert = matches!(edit.target, CellTarget::Insert { .. });
    let can_be_null = edit.nullable;
    let value_mode = !edit.insert_is_default && !edit.is_null;
    let apply_disabled = (!can_be_null && edit.is_null && !edit.insert_is_default)
        || (insert && edit.insert_is_default);

    div()
        .id("cell-overlay")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
            if event.keystroke.key == "escape" {
                this.close_cell_editor(cx);
            }
        }))
        .child(
            div()
                .id("cell-backdrop")
                .absolute()
                .inset_0()
                .bg(gpui_kit::hsla(0.0, 0.0, 0.0, 0.35))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.close_cell_editor(cx);
                })),
        )
        .child(
            v_flex()
                .relative()
                .occlude()
                .w(px(520.0))
                .rounded_lg()
                .border_1()
                .border_color(palette.border)
                .bg(palette.background)
                .shadow_lg()
                .p_4()
                .gap_3()
                .child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            v_flex()
                                .child(
                                    div()
                                        .text_lg()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(edit.title.clone()),
                                )
                                .child(div().text_xs().text_color(palette.muted).child(format!(
                                    "Column {} · {}",
                                    edit.column_name,
                                    if edit.nullable {
                                        "nullable"
                                    } else {
                                        "required"
                                    }
                                ))),
                        )
                        .child(
                            Button::new("cell-close")
                                .ghost()
                                .compact()
                                .icon(IconName::Close)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.close_cell_editor(cx);
                                })),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(palette.muted)
                        .child(if edit.insert_is_default {
                            "The column default will be used."
                        } else if edit.is_null {
                            "NULL will be stored."
                        } else {
                            "Enter a value:"
                        }),
                )
                .child(
                    div()
                        .w_full()
                        .child(Input::new(&edit.input).w_full().disabled(!value_mode)),
                )
                .when(insert, |this| {
                    this.child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("cell-mode-default")
                                    .compact()
                                    .when(edit.insert_is_default, |this| this.primary())
                                    .when(!edit.insert_is_default, |this| this.secondary())
                                    .label("Default")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_cell_default(true, cx);
                                    })),
                            )
                            .child(
                                Button::new("cell-mode-value")
                                    .compact()
                                    .when(value_mode, |this| this.primary())
                                    .when(!value_mode, |this| this.secondary())
                                    .label("Value")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_cell_default(false, cx);
                                        this.set_cell_null(false, cx);
                                    })),
                            )
                            .child(
                                Button::new("cell-mode-null")
                                    .compact()
                                    .when(edit.is_null, |this| this.primary())
                                    .when(!edit.is_null, |this| this.secondary())
                                    .label("NULL")
                                    .disabled(!can_be_null)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_cell_default(false, cx);
                                        this.set_cell_null(true, cx);
                                    })),
                            ),
                    )
                })
                .when(!insert, |this| {
                    this.child(
                        h_flex().gap_2().child(
                            Button::new("cell-toggle-null")
                                .compact()
                                .when(edit.is_null, |this| this.primary())
                                .when(!edit.is_null, |this| this.secondary())
                                .label(if edit.is_null {
                                    "Set value"
                                } else {
                                    "Set NULL"
                                })
                                .disabled(!can_be_null && !edit.is_null)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let next =
                                        !this.cell_edit.as_ref().is_some_and(|edit| edit.is_null);
                                    this.set_cell_null(next, cx);
                                })),
                        ),
                    )
                })
                .child(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("cell-cancel")
                                .secondary()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.close_cell_editor(cx);
                                })),
                        )
                        .child(
                            Button::new("cell-apply")
                                .primary()
                                .label("Apply")
                                .disabled(apply_disabled)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.apply_cell_editor(cx);
                                })),
                        ),
                ),
        )
        .into_any_element()
}

pub(super) fn value_text(value: &Option<String>, palette: &Palette) -> AnyElement {
    match value {
        None => div()
            .text_xs()
            .italic()
            .text_color(palette.muted)
            .child("NULL")
            .into_any_element(),
        Some(value) if value.is_empty() => div()
            .text_xs()
            .text_color(palette.muted)
            .child("empty")
            .into_any_element(),
        Some(value) => div()
            .text_sm()
            .overflow_hidden()
            .whitespace_nowrap()
            .child(value.clone())
            .into_any_element(),
    }
}

pub(super) fn empty_state(
    title: &'static str,
    detail: &'static str,
    palette: &Palette,
) -> impl IntoElement {
    v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_2()
        .child(
            gpui_kit::component::Icon::new(IconName::SquareTerminal)
                .with_size(Size::Large)
                .text_color(palette.muted),
        )
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(div().text_xs().text_color(palette.muted).child(detail))
}

pub(super) fn field_label(label: &'static str, palette: &Palette) -> impl IntoElement {
    div().text_xs().text_color(palette.muted).child(label)
}

pub(super) fn input_field(
    label: &'static str,
    input: &Entity<InputState>,
    disabled: bool,
    palette: &Palette,
) -> Div {
    v_flex()
        .w_full()
        .min_w(px(0.0))
        .gap_1()
        .child(field_label(label, palette))
        .child(Input::new(input).w_full().disabled(disabled))
}

pub(super) fn render_confirm(
    pending: &ConfirmState,
    palette: &Palette,
    cx: &mut Context<KoyoApp>,
) -> AnyElement {
    div()
        .id("confirm-overlay")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
            if event.keystroke.key == "escape" {
                this.resolve_confirm(false, cx);
            }
        }))
        .child(
            div()
                .id("confirm-backdrop")
                .absolute()
                .inset_0()
                .bg(gpui_kit::hsla(0.0, 0.0, 0.0, 0.35))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.resolve_confirm(false, cx);
                })),
        )
        .child(
            v_flex()
                .relative()
                .occlude()
                .w(px(460.0))
                .rounded_lg()
                .border_1()
                .border_color(palette.border)
                .bg(palette.background)
                .shadow_lg()
                .p_4()
                .gap_3()
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(pending.title.clone()),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(palette.muted)
                        .child(pending.description.clone()),
                )
                .child(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("confirm-cancel")
                                .secondary()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.resolve_confirm(false, cx);
                                })),
                        )
                        .child(
                            Button::new("confirm-continue")
                                .primary()
                                .label("Continue")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.resolve_confirm(true, cx);
                                })),
                        ),
                ),
        )
        .into_any_element()
}
