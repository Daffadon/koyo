use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::{Focusable, Window};
use koyo_core::models::TableSummary;

use crate::app::*;

fn table_matches(table: &TableSummary, query: &str) -> bool {
    query.is_empty()
        || table.name.to_lowercase().contains(query)
        || table.schema.to_lowercase().contains(query)
}

impl KoyoApp {
    pub(crate) fn quick_open_matches(&self, query: &str) -> Vec<TableSummary> {
        let Some(tab) = self.sidebar_index().and_then(|index| self.tabs.get(index)) else {
            return Vec::new();
        };
        let query = query.trim().to_lowercase();
        let mut matches = tab
            .tables
            .iter()
            .filter(|table| table_matches(table, &query))
            .cloned()
            .collect::<Vec<_>>();
        matches.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.schema.to_lowercase().cmp(&b.schema.to_lowercase()))
        });
        matches
    }

    pub(crate) fn open_quick_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(open) = self.quick_open.as_ref() {
            open.input.read(cx).focus_handle(cx).focus(window, cx);
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search tables..."));
        let return_focus = window.focused(cx);
        let observed = input.clone();
        cx.observe(&input, move |this, _, cx| {
            if let Some(open) = this.quick_open.as_mut()
                && open.input == observed
            {
                open.selected = 0;
                cx.notify();
            }
        })
        .detach();
        input.read(cx).focus_handle(cx).focus(window, cx);
        self.quick_open = Some(QuickOpen {
            input,
            selected: 0,
            return_focus,
        });
        cx.notify();
    }

    pub(crate) fn close_quick_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.quick_open.take() else {
            return;
        };
        if let Some(focus) = open.return_focus.clone() {
            focus.focus(window, cx);
        } else {
            self.root_focus.focus(window, cx);
        }
        cx.notify();
    }

    pub(crate) fn move_quick_open(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(open) = self.quick_open.as_ref() else {
            return;
        };
        let query = open.input.read(cx).value().to_string();
        let count = self.quick_open_matches(&query).len();
        if count == 0 {
            return;
        }
        let Some(open) = self.quick_open.as_mut() else {
            return;
        };
        if delta < 0 {
            open.selected = open.selected.saturating_sub(1);
        } else {
            open.selected = (open.selected + 1).min(count - 1);
        }
        cx.notify();
    }

    pub(crate) fn accept_quick_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.quick_open.as_ref() else {
            return;
        };
        let query = open.input.read(cx).value().to_string();
        let selected = open.selected;
        let Some(table) = self.quick_open_matches(&query).get(selected).cloned() else {
            return;
        };
        self.close_quick_open(window, cx);
        self.open_table(table.schema, table.name, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::table_matches;
    use koyo_core::models::TableSummary;

    fn table(schema: &str, name: &str) -> TableSummary {
        TableSummary {
            schema: schema.into(),
            name: name.into(),
            editable: true,
        }
    }

    #[test]
    fn filters_tables_by_schema_or_name() {
        let users = table("public", "users");
        assert!(table_matches(&users, ""));
        assert!(table_matches(&users, "user"));
        assert!(table_matches(&users, "public"));
        assert!(!table_matches(&users, "orders"));
    }
}
