use gpui_kit::InteractiveElement;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{Disableable, FocusTrapElement as _, Icon, IconName, Size};
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::widgets::{SETTINGS_FOCUS_SCOPE, control_focus, is_activation_key};
use crate::app::actions::settings::NotificationPreference;
use crate::app::{KoyoApp, Palette, SettingsSection, ThemeChoice};

fn preference_row(
    id: &'static str,
    title: &'static str,
    detail: &'static str,
    checked: bool,
    enabled: bool,
    preference: NotificationPreference,
    palette: &Palette,
    cx: &mut Context<KoyoApp>,
) -> AnyElement {
    v_flex()
        .gap_1()
        .child(
            Checkbox::new(id)
                .checked(checked)
                .label(title)
                .accessibility_label(title)
                .disabled(!enabled)
                .on_change(cx.listener(move |this, checked, _, cx| {
                    this.set_notification_preference(preference, *checked, cx);
                })),
        )
        .child(div().text_xs().text_color(palette.muted).child(detail))
        .into_any_element()
}

/// Small palette sample for a theme row: the background fills the card, the
/// sidebar marks the rail, and the strip previews the action gradient.
fn theme_preview(choice: ThemeChoice, palette: &Palette) -> AnyElement {
    let [background, sidebar, _] = choice.preview_colors();
    let [from, to] = choice.preview_gradient();
    v_flex()
        .w(px(44.0))
        .h(px(28.0))
        .flex_none()
        .overflow_hidden()
        .rounded_md()
        .border_1()
        .border_color(palette.border)
        .bg(background)
        .child(
            h_flex()
                .flex_1()
                .min_h(px(0.0))
                .child(div().w(px(10.0)).h_full().bg(sidebar)),
        )
        .child(div().w_full().h(px(3.0)).bg(linear_gradient(
            135.0,
            linear_color_stop(from, 0.0),
            linear_color_stop(to, 1.0),
        )))
        .into_any_element()
}

fn category_row(
    app: &KoyoApp,
    section: SettingsSection,
    index: usize,
    palette: &Palette,
    cx: &mut Context<KoyoApp>,
) -> AnyElement {
    let selected = app.settings_section == section;
    div()
        .id(("settings-category", index))
        .w_full()
        .h(px(34.0))
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .px_2()
        .rounded_md()
        .border_1()
        .border_color(if selected {
            palette.ring
        } else {
            palette.sidebar
        })
        .when(selected, |this| this.bg(palette.selection))
        .when(!selected, |this| {
            this.hover(|style| style.bg(palette.panel_alt))
        })
        .cursor_pointer()
        .text_sm()
        .text_color(if selected {
            palette.accent
        } else {
            palette.foreground
        })
        .when(selected, |this| this.font_weight(FontWeight::SEMIBOLD))
        .track_focus(&control_focus(
            SETTINGS_FOCUS_SCOPE,
            format!("settings-section-{index}"),
            cx,
        ))
        .tab_stop(true)
        .focus(|style| style.bg(palette.accent_soft).border_color(palette.ring))
        .aria_label(format!("{} settings", section.label()))
        .aria_selected(selected)
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
            if is_activation_key(event) {
                cx.stop_propagation();
                this.set_settings_section(section, cx);
            }
        }))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.set_settings_section(section, cx);
        }))
        .child(div().flex_1().child(section.label()))
        .when(selected, |this| {
            this.child(Icon::new(IconName::Check).with_size(Size::Small))
        })
        .into_any_element()
}

fn theme_row(
    app: &KoyoApp,
    choice: ThemeChoice,
    index: usize,
    palette: &Palette,
    cx: &mut Context<KoyoApp>,
) -> AnyElement {
    let selected = app.theme_choice == choice;
    div()
        .id(("settings-theme", index))
        .w_full()
        .flex()
        .items_center()
        .gap_3()
        .p_2()
        .rounded_md()
        .border_1()
        .border_color(if selected {
            palette.ring
        } else {
            palette.background
        })
        .when(selected, |this| this.bg(palette.selection))
        .when(!selected, |this| {
            this.hover(|style| style.bg(palette.panel_alt))
        })
        .cursor_pointer()
        .track_focus(&control_focus(
            SETTINGS_FOCUS_SCOPE,
            format!("settings-theme-{index}"),
            cx,
        ))
        .tab_stop(true)
        .focus(|style| style.bg(palette.accent_soft).border_color(palette.ring))
        .aria_label(format!("{} theme", choice.label()))
        .aria_selected(selected)
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
            if is_activation_key(event) {
                cx.stop_propagation();
                this.set_theme(choice, cx);
            }
        }))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.set_theme(choice, cx);
        }))
        .child(theme_preview(choice, palette))
        .child(
            v_flex()
                .flex_1()
                .min_w(px(0.0))
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .text_color(palette.foreground)
                        .child(choice.label()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(palette.muted)
                        .child(choice.description()),
                ),
        )
        .when(selected, |this| {
            this.child(
                Icon::new(IconName::Check)
                    .with_size(Size::Small)
                    .text_color(palette.accent),
            )
        })
        .into_any_element()
}

fn appearance_body(app: &KoyoApp, palette: &Palette, cx: &mut Context<KoyoApp>) -> AnyElement {
    v_flex()
        .gap_3()
        .child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Theme"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(palette.muted)
                        .child("Applies immediately. Koyo remembers your pick on this device."),
                ),
        )
        .children(
            ThemeChoice::ALL
                .into_iter()
                .enumerate()
                .map(|(index, choice)| theme_row(app, choice, index, palette, cx)),
        )
        .into_any_element()
}

fn notifications_body(app: &KoyoApp, palette: &Palette, cx: &mut Context<KoyoApp>) -> AnyElement {
    let notifications = &app.notifications;
    let categories = notifications.enabled;
    v_flex()
        .gap_3()
        .child(preference_row(
            "notify-enabled",
            "Show in-app notifications",
            "A short toast in the corner. Turn this off to silence every event.",
            notifications.enabled,
            true,
            NotificationPreference::Enabled,
            palette,
            cx,
        ))
        .child(
            v_flex()
                .gap_3()
                .when(!categories, |this| this.opacity(0.55))
                .child(preference_row(
                    "notify-connections",
                    "Connection changes",
                    "Connected, disconnected, connection lost, and reconnects.",
                    notifications.connections,
                    categories,
                    NotificationPreference::Connections,
                    palette,
                    cx,
                ))
                .child(preference_row(
                    "notify-queries",
                    "Query completion",
                    "Finished queries and query failures. Off by default to avoid noise.",
                    notifications.queries,
                    categories,
                    NotificationPreference::Queries,
                    palette,
                    cx,
                ))
                .child(preference_row(
                    "notify-data",
                    "Table edits",
                    "Applied row changes and failed saves.",
                    notifications.data,
                    categories,
                    NotificationPreference::Data,
                    palette,
                    cx,
                ))
                .child(preference_row(
                    "notify-exports",
                    "CSV exports",
                    "Export started, export ready, and export failures.",
                    notifications.exports,
                    categories,
                    NotificationPreference::Exports,
                    palette,
                    cx,
                )),
        )
        .into_any_element()
}

fn connections_body(app: &KoyoApp, palette: &Palette, cx: &mut Context<KoyoApp>) -> AnyElement {
    v_flex()
        .gap_3()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child("Idle connections"),
        )
        .child(
            div()
                .text_xs()
                .text_color(palette.muted)
                .child(format!(
                    "{} of {} slots in use. Pooled sessions are reused by new tabs and checked every 15 seconds.",
                    app.parked_connections.len(),
                    app.idle_pool_size
                )),
        )
        .child(
            h_flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .w(px(86.0))
                        .text_xs()
                        .text_color(palette.muted)
                        .child("Pool size"),
                )
                .children([0usize, 2, 4, 8].into_iter().map(|size| {
                    Button::new(("pool-size", size))
                        .compact()
                        .label(if size == 0 {
                            "Off".to_string()
                        } else {
                            size.to_string()
                        })
                        .when(app.idle_pool_size == size, |this| this.primary())
                        .when(app.idle_pool_size != size, |this| this.ghost())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_idle_pool_size(size, cx);
                        }))
                })),
        )
        .child(
            h_flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .w(px(86.0))
                        .text_xs()
                        .text_color(palette.muted)
                        .child("Expire after"),
                )
                .children(
                    [
                        (60u64, "1 min"),
                        (300, "5 min"),
                        (900, "15 min"),
                        (3600, "1 hour"),
                    ]
                    .into_iter()
                    .map(|(seconds, label)| {
                        Button::new(format!("pool-ttl-{seconds}"))
                            .compact()
                            .label(label)
                            .when(app.idle_pool_ttl_secs == seconds, |this| this.primary())
                            .when(app.idle_pool_ttl_secs != seconds, |this| this.ghost())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_idle_pool_ttl_secs(seconds, cx);
                            }))
                    }),
                ),
        )
        .child(
            Button::new("disconnect-idle")
                .secondary()
                .compact()
                .label("Disconnect idle")
                .disabled(app.parked_connections.is_empty())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.disconnect_idle_connections(cx);
                })),
        )
        .into_any_element()
}

pub(super) fn render_settings(
    app: &KoyoApp,
    palette: &Palette,
    cx: &mut Context<KoyoApp>,
) -> AnyElement {
    let section = app.settings_section;
    let body = match section {
        SettingsSection::Appearance => appearance_body(app, palette, cx),
        SettingsSection::Notifications => notifications_body(app, palette, cx),
        SettingsSection::Connections => connections_body(app, palette, cx),
    };

    let sidebar = v_flex()
        .w(px(190.0))
        .h_full()
        .flex_none()
        .gap_1()
        .p_3()
        .bg(palette.sidebar_gradient)
        .border_r_1()
        .border_color(palette.sidebar_border)
        .child(
            div()
                .pb_2()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child("Settings"),
        )
        .children(
            SettingsSection::ALL
                .into_iter()
                .enumerate()
                .map(|(index, entry)| category_row(app, entry, index, palette, cx)),
        );

    let content = v_flex()
        .flex_1()
        .min_w(px(0.0))
        .h_full()
        .child(
            h_flex()
                .items_start()
                .justify_between()
                .gap_3()
                .px_5()
                .pt_4()
                .pb_3()
                .child(
                    v_flex()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap_1()
                        .child(
                            div()
                                .text_lg()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(section.label()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette.muted)
                                .child(section.description()),
                        ),
                )
                .child(
                    Button::new("settings-close")
                        .ghost()
                        .compact()
                        .icon(IconName::Close)
                        .accessibility_label("Close settings")
                        .tooltip("Close")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.close_settings(window, cx);
                        })),
                ),
        )
        .child(
            v_flex()
                .id("settings-body")
                .flex_1()
                .min_h(px(0.0))
                .gap_3()
                .px_5()
                .pb_4()
                .overflow_y_scroll()
                .child(body),
        )
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .px_5()
                .py_3()
                .border_t_1()
                .border_color(palette.border)
                .child(
                    div()
                        .text_xs()
                        .text_color(palette.muted)
                        .child("Changes save automatically."),
                )
                .child(
                    Button::new("settings-done")
                        .primary()
                        .label("Done")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.close_settings(window, cx);
                        })),
                ),
        );

    div()
        .id("settings-overlay")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
            if event.keystroke.key == "escape" {
                this.close_settings(window, cx);
            }
        }))
        .child(
            div()
                .id("settings-backdrop")
                .absolute()
                .inset_0()
                .bg(gpui_kit::hsla(0.0, 0.0, 0.0, 0.35))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.close_settings(window, cx);
                })),
        )
        .child(
            h_flex()
                .id("settings-card")
                .relative()
                .occlude()
                .w(px(760.0))
                .h(px(560.0))
                .max_w(relative(0.94))
                .max_h(relative(0.92))
                .overflow_hidden()
                .track_focus(&app.settings_focus)
                .focus_trap("settings-focus-trap", &app.settings_focus)
                .rounded_lg()
                .border_1()
                .border_color(palette.border)
                .bg(palette.background)
                .shadow_lg()
                .child(sidebar)
                .child(content),
        )
        .into_any_element()
}
