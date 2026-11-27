use super::super::widgets;
use super::*;

const HISTORY_ROW_HEIGHT: f32 = 40.0;
const HISTORY_OVERSCAN: usize = 4;

fn history_preview(sql: &str) -> String {
    let mut preview = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    if preview.chars().count() > 70 {
        preview = preview.chars().take(67).collect::<String>() + "...";
    }
    preview
}

fn history_age(at: u64) -> String {
    let seconds = unix_seconds().saturating_sub(at);
    match seconds {
        0..=59 => "now".to_string(),
        60..=3_599 => format!("{}m ago", seconds / 60),
        3_600..=86_399 => format!("{}h ago", seconds / 3_600),
        _ => format!("{}d ago", seconds / 86_400),
    }
}

pub(super) fn history_matches(entry: &QueryHistoryEntry, search: &str) -> bool {
    if search.is_empty() {
        return true;
    }
    entry.sql.to_lowercase().contains(search)
        || entry.profile_name.to_lowercase().contains(search)
        || if entry.success {
            "succeeded".contains(search)
        } else {
            "failed".contains(search)
        }
}

fn history_row_range(offset_y: f32, viewport_height: f32, count: usize) -> std::ops::Range<usize> {
    if count == 0 {
        return 0..0;
    }
    let visible = if viewport_height > 0.0 {
        (viewport_height / HISTORY_ROW_HEIGHT).ceil() as usize
    } else {
        12
    };
    let first = ((-offset_y / HISTORY_ROW_HEIGHT).floor().max(0.0) as usize)
        .saturating_sub(HISTORY_OVERSCAN)
        .min(count.saturating_sub(1));
    let last = (first + visible + HISTORY_OVERSCAN * 2).min(count);
    first..last
}

fn clamp_history_scroll(scroll: &ScrollHandle, count: usize) {
    let viewport = f32::from(scroll.bounds().size.height);
    let max_scroll = (count as f32 * HISTORY_ROW_HEIGHT - viewport).max(0.0);
    let offset = scroll.offset();
    if f32::from(offset.y) < -max_scroll {
        scroll.set_offset(point(offset.x, px(-max_scroll)));
    }
}

fn history_visible_range(scroll: &ScrollHandle, count: usize) -> std::ops::Range<usize> {
    history_row_range(
        f32::from(scroll.offset().y),
        f32::from(scroll.bounds().size.height),
        count,
    )
}

impl KoyoApp {
    pub(super) fn render_history_panel(
        &self,
        tab_id: u64,
        history: &[QueryHistoryEntry],
        history_search: &str,
        history_indices: &[usize],
        palette: &Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if history.is_empty() {
            None
        } else {
            let collapsed = self.history_collapsed;
            let history_body = if history_indices.is_empty() {
                div()
                    .px_2()
                    .py_4()
                    .text_xs()
                    .text_color(palette.muted)
                    .child(format!("No history matches \"{history_search}\"."))
                    .into_any_element()
            } else {
                clamp_history_scroll(&self.history_scroll, history_indices.len());
                let range = history_visible_range(&self.history_scroll, history_indices.len());
                let first = range.start;
                let last = range.end;
                let row_count = history_indices.len();
                v_flex()
                    .id("history-list")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .track_scroll(&self.history_scroll)
                    .px_2()
                    .pb_2()
                    .child(
                        v_flex()
                            .h(px(row_count as f32 * HISTORY_ROW_HEIGHT))
                            .flex_shrink_0()
                            .child(div().h(px(first as f32 * HISTORY_ROW_HEIGHT)))
                            .children((first..last).map(|position| {
                                let index = history_indices[position];
                                let entry = &history[index];
                                let preview = history_preview(&entry.sql);
                                let detail = format!(
                                    "{} · {} · {} · {} ms",
                                    if entry.success { "Succeeded" } else { "Failed" },
                                    history_age(entry.at),
                                    entry.profile_name,
                                    entry.duration_ms
                                );
                                div()
                                    .id(("history-entry", index))
                                    .h(px(HISTORY_ROW_HEIGHT))
                                    .flex_shrink_0()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .px_2()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .hover(|style| style.bg(palette.panel))
                                    .track_focus(&control_focus(
                                        tab_id,
                                        format!("history-{index}"),
                                        cx,
                                    ))
                                    .tab_stop(true)
                                    .focus(|style| {
                                        style.bg(palette.accent_soft).border_color(palette.ring)
                                    })
                                    .on_key_down(cx.listener(
                                        move |this, event: &KeyDownEvent, window, cx| {
                                            if is_activation_key(event) {
                                                this.load_history_entry(index, window, cx);
                                            }
                                        },
                                    ))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.load_history_entry(index, window, cx);
                                    }))
                                    .child(
                                        div()
                                            .w(px(6.0))
                                            .h(px(6.0))
                                            .flex_shrink_0()
                                            .rounded_full()
                                            .bg(if entry.success {
                                                palette.success
                                            } else {
                                                palette.danger
                                            }),
                                    )
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .overflow_hidden()
                                                    .whitespace_nowrap()
                                                    .text_ellipsis()
                                                    .child(preview),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(palette.muted)
                                                    .overflow_hidden()
                                                    .whitespace_nowrap()
                                                    .text_ellipsis()
                                                    .child(detail),
                                            ),
                                    )
                                    .child(widgets::button_tooltip(
                                        Button::new(("history-run", index))
                                            .ghost()
                                            .compact()
                                            .icon(IconName::Play)
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                cx.stop_propagation();
                                                this.rerun_history(index, window, cx);
                                            })),
                                        "Re-run query",
                                    ))
                            }))
                            .child(div().h(px((row_count - last) as f32 * HISTORY_ROW_HEIGHT))),
                    )
                    .into_any_element()
            };
            Some(
                v_flex()
                    .flex_shrink_0()
                    .max_h(px(240.0))
                    .border_t_1()
                    .border_color(palette.sidebar_border)
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .px_4()
                            .pt_3()
                            .pb_1()
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_1()
                                    .child(widgets::button_tooltip(
                                        Button::new("history-toggle")
                                            .ghost()
                                            .compact()
                                            .icon(if collapsed {
                                                IconName::ChevronRight
                                            } else {
                                                IconName::ChevronDown
                                            })
                                            .accessibility_label(if collapsed {
                                                "Expand query history"
                                            } else {
                                                "Collapse query history"
                                            })
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.toggle_history(cx);
                                            })),
                                        if collapsed {
                                            "Expand history"
                                        } else {
                                            "Collapse history"
                                        },
                                    ))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(palette.muted)
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(format!("HISTORY ({})", history.len())),
                                    ),
                            )
                            .child(widgets::button_tooltip(
                                Button::new("clear-history")
                                    .ghost()
                                    .compact()
                                    .icon(IconName::Delete)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.request_clear_history(window, cx);
                                    })),
                                "Clear query history",
                            )),
                    )
                    .when(!collapsed, |this| {
                        this.child(
                            div().px_3().pb_2().child(
                                Input::new(&self.history_filter)
                                    .aria_label("Search history")
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
                        .child(history_body)
                    })
                    .into_any_element(),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{history_matches, history_row_range};
    use crate::app::QueryHistoryEntry;

    fn entry(sql: &str, profile: &str, success: bool) -> QueryHistoryEntry {
        QueryHistoryEntry {
            sql: sql.into(),
            profile_id: None,
            profile_name: profile.into(),
            success,
            duration_ms: 1,
            at: 0,
        }
    }

    #[test]
    fn history_search_matches_sql_profile_and_status() {
        let failed = entry("SELECT * FROM users", "Local", false);
        assert!(history_matches(&failed, ""));
        assert!(history_matches(&failed, "users"));
        assert!(history_matches(&failed, "local"));
        assert!(history_matches(&failed, "failed"));
        assert!(!history_matches(&failed, "succeeded"));
        assert!(!history_matches(&failed, "orders"));
        assert!(history_matches(
            &entry("SELECT 1", "Prod", true),
            "succeeded"
        ));
    }

    #[test]
    fn history_row_window_follows_scroll_and_clamps_to_count() {
        assert_eq!(history_row_range(0.0, 160.0, 100), 0..12);
        assert_eq!(history_row_range(-400.0, 160.0, 100), 6..18);
        assert_eq!(history_row_range(0.0, 160.0, 3), 0..3);
        assert_eq!(history_row_range(0.0, 160.0, 0), 0..0);
        assert_eq!(history_row_range(-10_000.0, 160.0, 100), 99..100);
    }
}
