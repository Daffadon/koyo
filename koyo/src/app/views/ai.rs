use std::sync::Arc;

use gpui_kit::StatefulInteractiveElement;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Textarea;
use gpui_kit::component::text::TextView;
use gpui_kit::component::{ActiveTheme, Disableable, Icon, IconName, Sizable, Size};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::app::{AiMessage, AiRole, KoyoApp, Palette};

pub(super) fn render(
    app: &KoyoApp,
    palette: &Palette,
    width: f32,
    cx: &mut Context<KoyoApp>,
) -> AnyElement {
    let model = app.ai_model.clone();

    let header = h_flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .border_b_1()
        .border_color(palette.sidebar_border)
        .child(
            div()
                .size(px(26.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_md()
                .bg(palette.accent_soft)
                .child(
                    Icon::new(IconName::Bot)
                        .with_size(Size::Small)
                        .text_color(palette.accent),
                ),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w(px(0.0))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("AI"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(palette.muted)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(model),
                ),
        )
        .child(
            h_flex()
                .items_center()
                .gap_1()
                .child(super::widgets::button_tooltip_if(
                    !(app.ai_pending || app.ai_messages().is_empty()),
                    Button::new("ai-new-chat")
                        .ghost()
                        .compact()
                        .icon(IconName::Plus)
                        .disabled(app.ai_pending || app.ai_messages().is_empty())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.new_ai_thread(window, cx);
                        })),
                    "New chat",
                ))
                .child(super::widgets::button_tooltip_if(
                    !app.ai_pending,
                    Button::new("ai-settings")
                        .ghost()
                        .compact()
                        .icon(IconName::Settings)
                        .disabled(app.ai_pending)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_ai_settings(window, cx);
                        })),
                    "AI provider settings",
                ))
                .child(super::widgets::button_tooltip(
                    Button::new("ai-close")
                        .ghost()
                        .compact()
                        .icon(IconName::PanelRightClose)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.close_ai(window, cx);
                        })),
                    "Hide AI",
                )),
        );

    let thread_index = app
        .ai_threads
        .iter()
        .position(|thread| thread.id == app.ai_active_thread)
        .unwrap_or(0);
    let thread_count = app.ai_threads.len();
    let thread_title = app
        .ai_threads
        .get(thread_index)
        .map(|thread| thread.title.clone())
        .unwrap_or_else(|| "New chat".into());
    let thread_bar = h_flex()
        .items_center()
        .gap_1()
        .px_3()
        .py_1()
        .border_b_1()
        .border_color(palette.sidebar_border)
        .child(super::widgets::button_tooltip(
            Button::new("ai-thread-prev")
                .ghost()
                .compact()
                .icon(IconName::ChevronLeft)
                .disabled(app.ai_pending || thread_count < 2)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.switch_ai_thread(-1, cx);
                })),
            "Previous chat",
        ))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .text_sm()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(thread_title),
        )
        .child(div().text_xs().text_color(palette.muted).child(format!(
            "{}/{}",
            thread_index + 1,
            thread_count
        )))
        .child(super::widgets::button_tooltip(
            Button::new("ai-thread-next")
                .ghost()
                .compact()
                .icon(IconName::ChevronRight)
                .disabled(app.ai_pending || thread_count < 2)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.switch_ai_thread(1, cx);
                })),
            "Next chat",
        ))
        .child(super::widgets::button_tooltip(
            Button::new("ai-compact")
                .ghost()
                .compact()
                .icon(IconName::Minimize)
                .disabled(app.ai_pending || app.ai_messages().is_empty())
                .on_click(cx.listener(|this, _, window, cx| {
                    this.compact_ai_context(window, cx);
                })),
            "Compact context",
        ))
        .child(super::widgets::button_tooltip(
            Button::new("ai-thread-delete")
                .ghost()
                .compact()
                .icon(IconName::CircleX)
                .disabled(app.ai_pending || thread_count <= 1)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.request_delete_ai_thread(window, cx);
                })),
            "Delete chat",
        ));

    let settings = v_flex()
        .id("ai-settings")
        .flex_1()
        .min_h(px(0.0))
        .overflow_y_scroll()
        .gap_3()
        .p_3()
        .child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("OpenAI-compatible provider"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(palette.muted)
                        .whitespace_normal()
                        .child(
                            "Use base_url, api_key, model, system_prompt, temperature, max_tokens, and optional headers. system_prompt is appended after Koyo's built-in policy; it cannot replace it. Values are stored in encrypted state.enc.",
                        ),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(palette.muted)
                .child("Settings JSON"),
        )
        .child(
            div()
                .rounded_md()
                .border_1()
                .border_color(palette.border)
                .bg(palette.background)
                .p_2()
                .child(
                    Textarea::new(&app.ai_config_input)
                        .aria_label("AI provider settings JSON")
                        .w_full()
                        .h(px(320.0))
                        .bordered(false)
                        .font_family(cx.theme().mono_font_family.clone()),
                ),
        )
        .when(!app.ai_config_error.is_empty(), |this| {
            this.child(
                h_flex()
                    .items_start()
                    .gap_2()
                    .rounded_md()
                    .border_1()
                    .border_color(palette.danger)
                    .bg(palette.panel)
                    .p_2()
                    .child(
                        Icon::new(IconName::CircleAlert)
                            .with_size(Size::Small)
                            .text_color(palette.danger),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_xs()
                            .text_color(palette.danger)
                            .whitespace_normal()
                            .child(app.ai_config_error.clone()),
                    ),
            )
        })
        .child(
            h_flex()
                .justify_end()
                .gap_2()
                .child(
                    Button::new("ai-settings-cancel")
                        .secondary()
                        .compact()
                        .label("Cancel")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.cancel_ai_settings(window, cx);
                        })),
                )
                .child(
                    Button::new("ai-settings-save")
                        .primary()
                        .compact()
                        .label("Save")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.save_ai_settings(window, cx);
                        })),
                ),
        );

    let mut transcript = v_flex().gap_3().p_3();
    if app.ai_messages().is_empty() && !app.ai_pending && app.ai_error.is_empty() {
        transcript = transcript.child(
            v_flex()
                .items_center()
                .gap_2()
                .py_8()
                .child(
                    Icon::new(IconName::Bot)
                        .with_size(Size::Large)
                        .text_color(palette.muted),
                )
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Ask about your database"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(palette.muted)
                        .text_center()
                        .whitespace_normal()
                        .child(
                            "Use the settings button to configure an OpenAI-compatible provider.",
                        ),
                )
                .child(
                    Button::new("ai-empty-settings")
                        .primary()
                        .compact()
                        .label("Provider settings")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_ai_settings(window, cx);
                        })),
                ),
        );
    }
    let thread_id = app.ai_active_thread;
    for (index, message) in app.ai_messages().iter().enumerate() {
        transcript = transcript.child(render_message(message, thread_id, index, palette));
    }
    if app.ai_pending {
        transcript = transcript.child(
            h_flex()
                .items_center()
                .gap_2()
                .text_xs()
                .text_color(palette.muted)
                .child(Icon::new(IconName::Bot).with_size(Size::Small))
                .child(if app.ai_compacting {
                    "Compacting context..."
                } else {
                    "Thinking..."
                }),
        );
    }
    if !app.ai_error.is_empty() {
        transcript = transcript.child(
            h_flex()
                .items_start()
                .gap_2()
                .rounded_md()
                .border_1()
                .border_color(palette.danger)
                .bg(palette.panel)
                .p_2()
                .child(
                    Icon::new(IconName::CircleAlert)
                        .with_size(Size::Small)
                        .text_color(palette.danger),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_xs()
                        .text_color(palette.danger)
                        .whitespace_normal()
                        .child(app.ai_error.clone()),
                ),
        );
    }

    let chat = div()
        .id("ai-transcript")
        .flex_1()
        .min_h(px(0.0))
        .overflow_y_scroll()
        .track_scroll(&app.ai_scroll)
        .child(transcript);

    let composer = v_flex()
        .gap_2()
        .p_3()
        .border_t_1()
        .border_color(palette.sidebar_border)
        .child(
            div()
                .text_xs()
                .text_color(palette.muted)
                .child("Live schema metadata attached · AI cannot execute SQL"),
        )
        .child(
            div()
                .rounded_md()
                .border_1()
                .border_color(palette.border)
                .bg(palette.background)
                .p_2()
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key == "enter" && !event.keystroke.modifiers.shift {
                        cx.stop_propagation();
                        this.send_ai_message(window, cx);
                    }
                }))
                .child(
                    Textarea::new(&app.ai_input)
                        .aria_label("Message the database assistant")
                        .w_full()
                        .bordered(false)
                        .disabled(app.ai_pending),
                ),
        )
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_xs()
                        .text_color(palette.muted)
                        .child(if app.ai_pending {
                            "Press Stop to cancel."
                        } else {
                            "Enter to send · Shift+Enter for newline"
                        }),
                )
                .child(if app.ai_pending {
                    Button::new("ai-stop")
                        .secondary()
                        .compact()
                        .icon(IconName::Square)
                        .label("Stop")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.stop_ai(cx);
                        }))
                        .into_any_element()
                } else {
                    Button::new("ai-send")
                        .primary()
                        .compact()
                        .icon(IconName::ArrowUp)
                        .label("Send")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.send_ai_message(window, cx);
                        }))
                        .into_any_element()
                }),
        );

    v_flex()
        .id("ai-sidebar")
        .w(px(width))
        .h_full()
        .flex_shrink_0()
        .bg(palette.sidebar)
        .child(header)
        .when(app.ai_settings_open, |this| this.child(settings))
        .when(!app.ai_settings_open, |this| {
            this.child(thread_bar).child(chat).child(composer)
        })
        .into_any_element()
}

fn block_images(_: &SharedUri) -> ImageSource {
    ImageSource::Custom(Arc::new(|_, _| {
        Some(Err(ImageCacheError::Asset(
            "AI message images are disabled.".into(),
        )))
    }))
}

fn render_message(
    message: &AiMessage,
    thread_id: u64,
    index: usize,
    palette: &Palette,
) -> AnyElement {
    match message.role {
        AiRole::User => h_flex()
            .justify_end()
            .child(
                div()
                    .max_w(px(300.0))
                    .rounded_lg()
                    .bg(palette.accent_soft)
                    .px_3()
                    .py_2()
                    .text_sm()
                    .whitespace_normal()
                    .child(message.content.clone()),
            )
            .into_any_element(),
        AiRole::Summary => v_flex()
            .gap_2()
            .rounded_md()
            .border_1()
            .border_color(palette.border)
            .bg(palette.panel)
            .p_2()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .text_color(palette.muted)
                    .child(Icon::new(IconName::Minimize).with_size(Size::Small))
                    .child("Context summary"),
            )
            .child(
                TextView::markdown(
                    SharedString::from(format!("ai-message-{thread_id}-{index}")),
                    message.content.clone(),
                )
                .image_source(block_images)
                .on_link_click(|url, _, _, cx| {
                    if url.starts_with("https://") || url.starts_with("http://") {
                        cx.open_url(url);
                    } else {
                        log::info!("blocked non-http(s) AI Markdown link");
                    }
                })
                .w_full()
                .text_sm()
                .selectable(true),
            )
            .into_any_element(),
        AiRole::Assistant => h_flex()
            .items_start()
            .gap_2()
            .child(
                div()
                    .size(px(22.0))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(palette.accent_soft)
                    .child(
                        Icon::new(IconName::Bot)
                            .with_size(Size::Small)
                            .text_color(palette.accent),
                    ),
            )
            .child(
                div().flex_1().min_w(px(0.0)).child(
                    TextView::markdown(
                        SharedString::from(format!("ai-message-{thread_id}-{index}")),
                        message.content.clone(),
                    )
                    .image_source(block_images)
                    .on_link_click(|url, _, _, cx| {
                        if url.starts_with("https://") || url.starts_with("http://") {
                            cx.open_url(url);
                        } else {
                            log::info!("blocked non-http(s) AI Markdown link");
                        }
                    })
                    .w_full()
                    .text_sm()
                    .selectable(true),
                ),
            )
            .into_any_element(),
    }
}
