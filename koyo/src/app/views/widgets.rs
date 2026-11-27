use gpui_kit::InteractiveElement;
use gpui_kit::StatefulInteractiveElement;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputGroup, InputState, Textarea};
use gpui_kit::component::{Disableable, Icon, IconName, Sizable, Size};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use crate::app::{CellEdit, CellTarget, ConfirmState, KoyoApp, Palette, QUICK_OPEN_RESULT_LIMIT};

thread_local! {
    /// Stable focus handles for custom, non-Button controls. Creating a handle
    /// during each render would lose focus as soon as the view repaints.
    static CONTROL_FOCUS: RefCell<HashMap<(u64, SharedString), (FocusHandle, u64)>> =
        RefCell::new(HashMap::new());
    static CONTROL_FOCUS_EPOCH: Cell<u64> = const { Cell::new(0) };
}

pub(super) const SETTINGS_FOCUS_SCOPE: u64 = u64::MAX;

pub(super) fn is_activation_key(event: &KeyDownEvent) -> bool {
    matches!(event.keystroke.key.as_str(), "enter" | "space")
}

pub(super) fn control_focus(tab_id: u64, key: impl Into<SharedString>, cx: &App) -> FocusHandle {
    let key = key.into();
    let epoch = CONTROL_FOCUS_EPOCH.with(Cell::get);
    CONTROL_FOCUS.with(|handles| {
        let mut handles = handles.borrow_mut();
        let entry = handles
            .entry((tab_id, key))
            .or_insert_with(|| (cx.focus_handle(), epoch));
        entry.1 = epoch;
        entry.0.clone().tab_stop(true)
    })
}

pub(super) fn prune_control_focus(app: &KoyoApp, window: &Window) {
    let previous = CONTROL_FOCUS_EPOCH.with(|epoch| {
        let current = epoch.get();
        epoch.set(current.wrapping_add(1));
        current
    });
    CONTROL_FOCUS.with(|handles| {
        let mut handles = handles.borrow_mut();
        handles.retain(|(tab_id, _), (handle, last_seen)| {
            let tab_alive =
                *tab_id == SETTINGS_FOCUS_SCOPE || app.tabs.iter().any(|tab| tab.id == *tab_id);
            tab_alive && (*last_seen == previous || handle.is_focused(window))
        });
        if handles.capacity() > handles.len().saturating_mul(4).max(16) {
            handles.shrink_to_fit();
        }
    });
}

/// Make a custom clickable row reachable by Tab, and activate it with Enter or
/// Space. GPUI has no native button element for table rows and grid cells.
pub(super) fn keyboard_row(
    row: gpui_kit::Stateful<gpui_kit::Div>,
    tab_id: u64,
    key: impl Into<SharedString>,
    palette: &Palette,
    cx: &mut Context<KoyoApp>,
    activate: impl Fn(&mut KoyoApp, &mut Window, &mut Context<KoyoApp>) + 'static,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let focus = control_focus(tab_id, key, cx);
    row.track_focus(&focus)
        .tab_stop(true)
        .focus(|style| style.bg(palette.accent_soft).border_color(palette.ring))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                cx.stop_propagation();
                activate(this, window, cx);
            }
        }))
}

/// Per-button tooltip with a short delay. Anchored to the button itself so
/// the popup follows the control when the toolbar reflows.
fn tooltip_button(
    button: Button,
    label: impl Into<SharedString>,
    placement: Option<gpui_kit::base::Placement>,
) -> AnyElement {
    let label = label.into();
    let button = button
        .accessibility_label(label.clone())
        .tooltip(label)
        .tooltip_show_delay(std::time::Duration::from_millis(120));
    match placement {
        Some(placement) => button.tooltip_placement(placement),
        None => button,
    }
    .into_any_element()
}

pub(super) fn button_tooltip(button: Button, label: impl Into<SharedString>) -> AnyElement {
    tooltip_button(button, label, None)
}

/// Top-bar controls use an above placement so their tooltip does not cover
/// the Run query button in the row below.
pub(super) fn top_bar_tooltip(button: Button, label: impl Into<SharedString>) -> AnyElement {
    tooltip_button(button, label, Some(gpui_kit::base::Placement::Top))
}

/// Brand monogram used in the title bar and sidebar. Deep indigo and comet
/// cyan keep the mark constant across the Dark, Light, and Kimi themes.
pub(super) fn logo_mark(size: f32) -> impl IntoElement {
    let night: gpui_kit::Hsla = gpui_kit::rgb(0x0B1238).into();
    let violet: gpui_kit::Hsla = gpui_kit::rgb(0x2A1A5E).into();
    let cyan: gpui_kit::Hsla = gpui_kit::rgb(0x5FE7FF).into();
    let edge: gpui_kit::Hsla = gpui_kit::rgb(0x3B4C9E).into();
    div()
        .size(px(size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .border_1()
        .border_color(edge)
        .bg(linear_gradient(
            135.0,
            linear_color_stop(night, 0.0),
            linear_color_stop(violet, 1.0),
        ))
        .text_color(cyan)
        .text_size(px(size * 0.46))
        .font_weight(FontWeight::BOLD)
        .child("KY")
}

pub(super) fn button_tooltip_if(
    enabled: bool,
    button: Button,
    label: impl Into<SharedString>,
) -> AnyElement {
    if enabled {
        button_tooltip(button, label)
    } else {
        button.accessibility_label(label).into_any_element()
    }
}

pub(super) fn render_cell_editor(
    edit: &CellEdit,
    palette: &Palette,
    cx: &mut Context<KoyoApp>,
) -> AnyElement {
    let insert = matches!(edit.target, CellTarget::Insert { .. });
    let can_be_null = edit.nullable;
    let value_mode = !edit.insert_is_default && !edit.is_null;
    let apply_disabled = !can_be_null && edit.is_null && !edit.insert_is_default;

    div()
        .id("cell-overlay")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
            if event.keystroke.key == "escape" {
                this.close_cell_editor(window, cx);
            }
        }))
        .child(
            div()
                .id("cell-backdrop")
                .absolute()
                .inset_0()
                .bg(gpui_kit::hsla(0.0, 0.0, 0.0, 0.35))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.close_cell_editor(window, cx);
                })),
        )
        .child(
            v_flex()
                .relative()
                .occlude()
                .w(px(720.0))
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
                                .accessibility_label("Close cell editor")
                                .tooltip("Close")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.close_cell_editor(window, cx);
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
                    InputGroup::new("cell-value-group")
                        .w_full()
                        .min_h(px(120.0))
                        .input(Textarea::new(&edit.input).aria_label("Cell value"))
                        .disabled(!value_mode),
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
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.close_cell_editor(window, cx);
                                })),
                        )
                        .child(
                            Button::new("cell-apply")
                                .primary()
                                .label("Apply")
                                .disabled(apply_disabled)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.apply_cell_editor(window, cx);
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
            .text_ellipsis()
            .child(value.replace('\n', " ").replace('\r', " "))
            .into_any_element(),
    }
}

pub(super) fn empty_state(
    title: &'static str,
    detail: &'static str,
    palette: &Palette,
) -> impl IntoElement {
    v_flex().flex_1().items_center().justify_center().child(
        v_flex()
            .items_center()
            .gap_2()
            .px_8()
            .py_6()
            .rounded_lg()
            .bg(palette.background.opacity(0.36))
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
            .child(div().text_xs().text_color(palette.muted).child(detail)),
    )
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
        .child(
            Input::new(input)
                .aria_label(label)
                .w_full()
                .disabled(disabled),
        )
}

pub(super) fn render_quick_open(
    app: &KoyoApp,
    palette: &Palette,
    cx: &mut Context<KoyoApp>,
) -> AnyElement {
    let Some(open) = app.quick_open.as_ref() else {
        return div().into_any_element();
    };
    let query = open.input.read(cx).value().to_string();
    let matches = app.quick_open_matches(&query);
    let selected = open
        .selected
        .min(matches.len().saturating_sub(1))
        .min(QUICK_OPEN_RESULT_LIMIT.saturating_sub(1));
    let count = matches.len();

    let results = if matches.is_empty() {
        vec![
            div()
                .px_3()
                .py_6()
                .text_sm()
                .text_color(palette.muted)
                .child(if query.trim().is_empty() {
                    "No tables loaded for this connection."
                } else {
                    "No tables match this search."
                })
                .into_any_element(),
        ]
    } else {
        matches
            .into_iter()
            .take(QUICK_OPEN_RESULT_LIMIT)
            .enumerate()
            .map(|(index, table)| {
                let schema = table.schema.clone();
                let name = table.name.clone();
                div()
                    .id(SharedString::from(format!(
                        "quick-open-{schema}-{name}-{index}"
                    )))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .cursor_pointer()
                    .when(index == selected, |this| this.bg(palette.selection))
                    .when(index != selected, |this| {
                        this.hover(|style| style.bg(palette.panel_alt))
                    })
                    .child(Icon::new(IconName::LayoutDashboard).with_size(Size::Small))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_sm()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(format!("{schema}.{name}")),
                    )
                    .child(div().text_xs().text_color(palette.muted).child("table"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.close_quick_open(window, cx);
                        this.open_table(schema.clone(), name.clone(), window, cx);
                    }))
                    .into_any_element()
            })
            .collect()
    };

    div()
        .id("quick-open-overlay")
        .key_context("QuickOpen")
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
            match event.keystroke.key.as_str() {
                "escape" => {
                    cx.stop_propagation();
                    this.close_quick_open(window, cx);
                }
                "enter" => {
                    cx.stop_propagation();
                    this.accept_quick_open(window, cx);
                }
                "down" => {
                    cx.stop_propagation();
                    this.move_quick_open(1, cx);
                }
                "up" => {
                    cx.stop_propagation();
                    this.move_quick_open(-1, cx);
                }
                _ => {}
            }
        }))
        .absolute()
        .inset_0()
        .flex()
        .items_start()
        .justify_center()
        .pt(px(110.0))
        .child(
            div()
                .id("quick-open-backdrop")
                .absolute()
                .inset_0()
                .bg(gpui_kit::hsla(0.0, 0.0, 0.0, 0.35))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.close_quick_open(window, cx);
                })),
        )
        .child(
            v_flex()
                .relative()
                .occlude()
                .w(px(600.0))
                .max_h(px(460.0))
                .rounded_lg()
                .border_1()
                .border_color(palette.border)
                .bg(palette.background)
                .shadow_lg()
                .overflow_hidden()
                .child(
                    div()
                        .px_4()
                        .pt_3()
                        .pb_2()
                        .text_xs()
                        .text_color(palette.muted)
                        .child(format!("Quick open · {count} table(s)")),
                )
                .child(
                    div().px_3().pb_3().child(
                        Input::new(&open.input)
                            .aria_label("Quick open table")
                            .w_full()
                            .cleanable(true)
                            .with_size(Size::Small)
                            .prefix(
                                Icon::new(IconName::Search)
                                    .with_size(Size::Small)
                                    .text_color(palette.muted),
                            ),
                    ),
                )
                .child(
                    v_flex()
                        .id("quick-open-results")
                        .max_h(px(330.0))
                        .overflow_y_scroll()
                        .track_scroll(&open.scroll)
                        .gap_1()
                        .px_2()
                        .pb_2()
                        .children(results),
                ),
        )
        .into_any_element()
}

pub(super) fn render_confirm(
    pending: &ConfirmState,
    focus_handle: &FocusHandle,
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
        .track_focus(focus_handle)
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
            if event.keystroke.key == "escape" {
                cx.stop_propagation();
                this.resolve_confirm(false, window, cx);
            }
        }))
        .child(
            div()
                .id("confirm-backdrop")
                .absolute()
                .inset_0()
                .bg(gpui_kit::hsla(0.0, 0.0, 0.0, 0.35))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.resolve_confirm(false, window, cx);
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
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.resolve_confirm(false, window, cx);
                                })),
                        )
                        .child(
                            Button::new("confirm-continue")
                                .primary()
                                .label("Continue")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.resolve_confirm(true, window, cx);
                                })),
                        ),
                ),
        )
        .into_any_element()
}
