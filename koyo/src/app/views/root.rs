use gpui_kit::component::TitleBar;
use gpui_kit::prelude::*;
use gpui_kit::*;
use gpui_kit::{CursorStyle, DragMoveEvent, InteractiveElement, StatefulInteractiveElement};

use super::{connection, query, table, widgets};
use crate::app::*;
use crate::{RunQuery, ToggleAi, ToggleSidebar};

const AI_DIVIDER_WIDTH: f32 = 6.0;

struct AiResizeDrag;

impl Render for AiResizeDrag {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

impl Render for KoyoApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.primary_tabs.is_empty() {
            let tab = self.make_query_tab(None, window, cx);
            self.add_tab(tab, Pane::Primary);
        }
        self.normalize_panes();
        query::prune_scroll_handles(self);
        table::prune_scroll_handles(self);
        self.apply_pending_starters(window, cx);

        let palette = palette(cx);
        let show_custom_titlebar = !cfg!(target_os = "linux")
            || matches!(window.window_decorations(), Decorations::Client { .. });
        let dialog = if let Some(dialog) = self.dialog.as_ref() {
            Some(connection::render_dialog(self, dialog, &palette, cx))
        } else {
            None
        };
        let cell_edit = if let Some(edit) = self.cell_edit.as_ref() {
            Some(widgets::render_cell_editor(edit, &palette, cx))
        } else {
            None
        };
        let confirm = if let Some(pending) = self.confirm.as_ref() {
            Some(widgets::render_confirm(pending, &palette, cx))
        } else {
            None
        };
        let ai = self.ai_open.then(|| {
            let divider = div()
                .id("ai-divider")
                .w(px(AI_DIVIDER_WIDTH))
                .h_full()
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .cursor(CursorStyle::ResizeLeftRight)
                .on_drag(AiResizeDrag, |_, _, _, cx| {
                    cx.stop_propagation();
                    cx.new(|_| AiResizeDrag)
                })
                .child(div().w(px(1.0)).h_full().bg(palette.border))
                .into_any_element();
            let panel = super::ai::render(self, &palette, self.ai_width, cx);
            (divider, panel)
        });

        let titlebar = TitleBar::new()
            .bg(palette.sidebar)
            .border_color(palette.sidebar_border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .h_full()
                    .child(widgets::logo_mark(&palette, 20.0))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Koyo"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.muted)
                            .child("Database client"),
                    ),
            );

        div()
            .flex()
            .flex_col()
            .size_full()
            .key_context("Koyo")
            .bg(palette.background)
            .text_color(palette.foreground)
            .when(show_custom_titlebar, |this| this.child(titlebar))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h(px(0.0))
                    .when(self.sidebar_open, |this| {
                        this.child(self.render_sidebar(&palette, cx))
                    })
                    .child(self.render_workspace(window, &palette, cx))
                    .when_some(ai, |this, (divider, panel)| {
                        this.child(divider).child(panel)
                    })
                    .on_drag_move::<AiResizeDrag>(cx.listener(
                        |this, event: &DragMoveEvent<AiResizeDrag>, _, cx| {
                            let width = f32::from(event.bounds.size.width)
                                - f32::from(event.event.position.x - event.bounds.origin.x)
                                - AI_DIVIDER_WIDTH / 2.0;
                            this.set_ai_width(width, cx);
                        },
                    ))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            if this.ai_resizing {
                                this.ai_resizing = false;
                                this.persist_state(cx);
                            }
                        }),
                    ),
            )
            .on_action(cx.listener(|this, _: &RunQuery, _, cx| {
                this.run_active_query(cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                this.toggle_sidebar(cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleAi, window, cx| {
                this.toggle_ai(window, cx);
            }))
            .when_some(dialog, |this, dialog| this.child(dialog))
            .when_some(cell_edit, |this, editor| this.child(editor))
            .when_some(confirm, |this, confirm| this.child(confirm))
    }
}
