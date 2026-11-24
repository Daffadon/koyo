use gpui_kit::component::TitleBar;
use gpui_kit::prelude::*;
use gpui_kit::*;
use gpui_kit::{CursorStyle, InteractiveElement};

use super::{connection, query, table, widgets};
use crate::app::*;
use crate::{
    CompleteFilter, DeleteLine, DuplicateLine, FormatQuery, MoveLineDown, MoveLineUp, MoveTabLeft,
    MoveTabRight, RefreshTable, RunQuery, ToggleAi, ToggleComment, ToggleSidebar,
};

const AI_DIVIDER_WIDTH: f32 = 6.0;
const SIDEBAR_DIVIDER_WIDTH: f32 = 6.0;

impl Render for KoyoApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_workspace_tab(window, cx);
        self.normalize_panes();
        query::prune_scroll_handles(self);
        table::prune_scroll_handles(self);
        widgets::prune_control_focus(self);
        self.apply_pending_starters(window, cx);

        let palette = palette(self.theme_choice, cx);
        let aurora = (self.theme_choice == ThemeChoice::Kimi).then(|| {
            div()
                .absolute()
                .inset_0()
                .child(img(kimi_aurora()).size_full().object_fit(ObjectFit::Cover))
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .bg(gpui_kit::hsla(0.0, 0.0, 0.0, 0.18)),
                )
        });
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
        let export = self
            .export
            .as_ref()
            .map(|dialog| super::export::render_export(dialog, &palette, cx));
        let settings = self
            .settings_open
            .then(|| super::settings::render_settings(self, &palette, cx));
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
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, event: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.ai_resizing = true;
                        let window_width = f32::from(window.viewport_size().width);
                        this.set_ai_width(
                            window_width - f32::from(event.position.x) - AI_DIVIDER_WIDTH / 2.0,
                            cx,
                        );
                    }),
                )
                .child(div().w(px(1.0)).h_full().bg(palette.border))
                .into_any_element();
            let panel = super::ai::render(self, &palette, self.ai_width, cx);
            (divider, panel)
        });

        let titlebar = TitleBar::new()
            .bg(palette.sidebar_gradient)
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
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .track_focus(&self.root_focus)
            .key_context("Koyo")
            .bg(palette.canvas)
            .text_color(palette.foreground)
            .when_some(aurora, |this, aurora| this.child(aurora))
            .when(show_custom_titlebar, |this| this.child(titlebar))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h(px(0.0))
                    .when(self.sidebar_open, |this| {
                        this.child(self.render_sidebar(&palette, cx)).child(
                            div()
                                .id("sidebar-divider")
                                .w(px(SIDEBAR_DIVIDER_WIDTH))
                                .h_full()
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor(CursorStyle::ResizeLeftRight)
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, event: &MouseDownEvent, _, cx| {
                                        cx.stop_propagation();
                                        this.sidebar_resizing = true;
                                        this.set_sidebar_width(
                                            f32::from(event.position.x)
                                                - SIDEBAR_DIVIDER_WIDTH / 2.0,
                                            cx,
                                        );
                                    }),
                                )
                                .child(div().w(px(1.0)).h_full().bg(palette.border)),
                        )
                    })
                    .child(self.render_workspace(window, &palette, cx))
                    .when_some(ai, |this, (divider, panel)| {
                        this.child(divider).child(panel)
                    })
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                        if this.sidebar_resizing {
                            this.set_sidebar_width(
                                f32::from(event.position.x) - SIDEBAR_DIVIDER_WIDTH / 2.0,
                                cx,
                            );
                        } else if this.ai_resizing {
                            let window_width = f32::from(window.viewport_size().width);
                            this.set_ai_width(
                                window_width - f32::from(event.position.x) - AI_DIVIDER_WIDTH / 2.0,
                                cx,
                            );
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            if this.ai_resizing || this.sidebar_resizing {
                                this.ai_resizing = false;
                                this.sidebar_resizing = false;
                                this.persist_state(cx);
                            }
                        }),
                    ),
            )
            .on_action(cx.listener(|this, _: &ToggleComment, window, cx| {
                this.toggle_sql_comment(window, cx);
            }))
            .on_action(cx.listener(|this, _: &MoveLineUp, window, cx| {
                this.move_line_up(window, cx);
            }))
            .on_action(cx.listener(|this, _: &MoveLineDown, window, cx| {
                this.move_line_down(window, cx);
            }))
            .on_action(cx.listener(|this, _: &DuplicateLine, window, cx| {
                this.duplicate_line(window, cx);
            }))
            .on_action(cx.listener(|this, _: &DeleteLine, window, cx| {
                this.delete_line(window, cx);
            }))
            .on_action(cx.listener(|this, _: &RunQuery, _, cx| {
                this.run_active_query(cx);
            }))
            .on_action(cx.listener(|this, _: &FormatQuery, window, cx| {
                this.format_active_query(window, cx);
            }))
            .on_action(cx.listener(|this, _: &MoveTabLeft, _, cx| {
                this.move_active_tab(-1, cx);
            }))
            .on_action(cx.listener(|this, _: &MoveTabRight, _, cx| {
                this.move_active_tab(1, cx);
            }))
            .on_action(cx.listener(|this, _: &RefreshTable, window, cx| {
                this.refresh_active_table(window, cx);
            }))
            .on_action(cx.listener(|this, _: &CompleteFilter, window, cx| {
                if !this.complete_active_filter(window, cx) {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                this.toggle_sidebar(cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleAi, window, cx| {
                this.toggle_ai(window, cx);
            }))
            .when_some(dialog, |this, dialog| this.child(dialog))
            .when_some(cell_edit, |this, editor| this.child(editor))
            .when_some(export, |this, export| this.child(export))
            .when_some(settings, |this, settings| this.child(settings))
            .when_some(confirm, |this, confirm| this.child(confirm))
    }
}
