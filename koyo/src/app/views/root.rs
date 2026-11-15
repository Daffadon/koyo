use gpui_kit::prelude::*;
use gpui_kit::*;

use super::{connection, query, table, widgets};
use crate::RunQuery;
use crate::app::*;

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

        div()
            .flex()
            .size_full()
            .bg(palette.background)
            .text_color(palette.foreground)
            .when(self.sidebar_open, |this| {
                this.child(self.render_sidebar(&palette, cx))
            })
            .child(self.render_workspace(window, &palette, cx))
            .on_action(cx.listener(|this, _: &RunQuery, _, cx| {
                this.run_active_query(cx);
            }))
            .when_some(dialog, |this, dialog| this.child(dialog))
            .when_some(cell_edit, |this, editor| this.child(editor))
            .when_some(confirm, |this, confirm| this.child(confirm))
    }
}
