use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{Disableable, IconName};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::app::actions::settings::NotificationPreference;
use crate::app::{KoyoApp, Palette};

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

pub(super) fn render_settings(
    app: &KoyoApp,
    palette: &Palette,
    cx: &mut Context<KoyoApp>,
) -> AnyElement {
    let notifications = &app.notifications;
    let categories = notifications.enabled;
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
            v_flex()
                .id("settings-card")
                .relative()
                .occlude()
                .w(px(520.0))
                .max_w(relative(0.92))
                .max_h(relative(0.92))
                .track_focus(&app.settings_focus)
                .rounded_lg()
                .border_1()
                .border_color(palette.border)
                .bg(palette.background)
                .shadow_lg()
                .p_5()
                .gap_4()
                .child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            v_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_lg()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("Settings"),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(palette.muted)
                                        .child("Koyo keeps these preferences on this device."),
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
                        .pt_2()
                        .overflow_y_scroll()
                        .border_t_1()
                        .border_color(palette.border)
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(palette.muted)
                                .child("NOTIFICATIONS"),
                        )
                        .child(preference_row(
                            "notify-enabled",
                            "Show in-app notifications",
                            "A short toast in the corner. Turn this off to silence every event.",
                            notifications.enabled,
                            true,
                            NotificationPreference::Enabled,
                            &palette,
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
                                    &palette,
                                    cx,
                                ))
                                .child(preference_row(
                                    "notify-queries",
                                    "Query completion",
                                    "Finished queries and query failures. Off by default to avoid noise.",
                                    notifications.queries,
                                    categories,
                                    NotificationPreference::Queries,
                                    &palette,
                                    cx,
                                ))
                                .child(preference_row(
                                    "notify-data",
                                    "Table edits",
                                    "Applied row changes and failed saves.",
                                    notifications.data,
                                    categories,
                                    NotificationPreference::Data,
                                    &palette,
                                    cx,
                                ))
                                .child(preference_row(
                                    "notify-exports",
                                    "CSV exports",
                                    "Export started, export ready, and export failures.",
                                    notifications.exports,
                                    categories,
                                    NotificationPreference::Exports,
                                    &palette,
                                    cx,
                                )),
                        ),
                )
                .child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .pt_3()
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
                ),
        )
        .into_any_element()
}
