use super::*;

impl KoyoApp {
    pub(super) fn restore_workspace(
        &mut self,
        snapshot: &WorkspaceSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.split = match snapshot.split.as_deref() {
            Some("vertical") => Some(SplitAxis::Vertical),
            Some("horizontal") => Some(SplitAxis::Horizontal),
            _ => None,
        };
        if !snapshot.secondary.is_empty() && self.split.is_none() {
            self.split = Some(SplitAxis::Vertical);
        }
        if snapshot.secondary.is_empty() {
            self.split = None;
        }

        let mut primary_ids = Vec::new();
        let mut secondary_ids = Vec::new();
        for (pane, tabs, ids) in [
            (Pane::Primary, &snapshot.primary, &mut primary_ids),
            (Pane::Secondary, &snapshot.secondary, &mut secondary_ids),
        ] {
            if pane == Pane::Secondary && self.split.is_none() {
                continue;
            }
            for saved_tab in tabs {
                let profile = saved_tab.profile_id().and_then(|id| {
                    self.profiles
                        .iter()
                        .find(|profile| profile.id.to_string() == id)
                        .cloned()
                });
                match saved_tab {
                    TabSnapshot::Query { sql, .. } => {
                        let mut tab = self.make_query_tab(profile, window, cx);
                        if let TabBody::Query(query) = &mut tab.body {
                            query.editor.update(cx, |editor, cx| {
                                editor.set_value(sql.clone(), window, cx);
                            });
                            // Restored SQL is not a starter; connection setup
                            // must not overwrite it.
                            query.starter = String::new().into();
                        }
                        let id = self.add_tab(tab, pane);
                        ids.push(id);
                    }
                    TabSnapshot::Table { schema, table, .. } => {
                        let Some(profile) = profile else {
                            continue;
                        };
                        let mut tab =
                            self.make_table_tab(profile, schema.clone(), table.clone(), window, cx);
                        tab.connecting = false;
                        tab.notice = "Restored tab. Reconnect to load rows.".into();
                        if let TabBody::Table(table) = &mut tab.body {
                            table.loading = false;
                        }
                        let id = self.add_tab(tab, pane);
                        ids.push(id);
                    }
                }
            }
        }

        if primary_ids.is_empty() && secondary_ids.is_empty() {
            return;
        }
        self.primary_active = primary_ids
            .get(snapshot.primary_active)
            .copied()
            .or_else(|| primary_ids.first().copied())
            .unwrap_or(0);
        self.secondary_active = secondary_ids
            .get(snapshot.secondary_active)
            .copied()
            .or_else(|| secondary_ids.first().copied())
            .unwrap_or(0);
        self.focus = if snapshot.focus_secondary && self.split.is_some() {
            Pane::Secondary
        } else {
            Pane::Primary
        };
        self.normalize_panes();
        self.sidebar_tab = self
            .pane_active_id(self.focus)
            .or_else(|| self.primary_tabs.first().copied())
            .or_else(|| self.secondary_tabs.first().copied());
    }

    pub(crate) fn persist_state(&mut self, cx: &App) {
        if self.persistence_disabled {
            return;
        }
        let state = self.persisted_state(cx);
        self.save_queue.submit(state);
    }

    pub(crate) fn persist_state_sync(&mut self, cx: &App) {
        if self.persistence_disabled {
            return;
        }
        let state = self.persisted_state(cx);
        if let Err(error) = self.save_queue.flush(state) {
            log::error!("could not save Koyo state: {error}");
            self.status = error;
        }
    }

    fn persisted_state(&mut self, cx: &App) -> PersistedState {
        PersistedState {
            theme: self.theme_choice.as_str().into(),
            sidebar_open: self.sidebar_open,
            sidebar_width: self.sidebar_width,
            ai_open: self.ai_open,
            ai_width: self.ai_width,
            ai_config_json: self.ai_config_json.clone(),
            ai_messages: Vec::new(),
            ai_threads: self.ai_threads_snapshot(),
            ai_active_thread: self.ai_active_thread,
            next_ai_thread_id: self.next_ai_thread_id,
            split: self.split.map(|axis| {
                match axis {
                    SplitAxis::Vertical => "vertical",
                    SplitAxis::Horizontal => "horizontal",
                }
                .to_string()
            }),
            split_ratio: self.split_ratio,
            history: self.history.clone(),
            history_collapsed: self.history_collapsed,
            workspace: self.workspace_snapshot(cx),
            notifications: self.notifications.clone(),
            idle_pool_size: self.idle_pool_size,
            idle_pool_ttl_secs: self.idle_pool_ttl_secs,
            page_limit: self.page_limit,
            query_timeout_secs: self.query_timeout_secs,
            history_limit: self.history_limit,
            shortcuts: self.shortcuts.clone(),
        }
    }

    fn workspace_snapshot(&self, cx: &App) -> WorkspaceSnapshot {
        let snapshot = |tab: &Tab| -> Option<TabSnapshot> {
            let profile_id = tab.profile.as_ref().map(|profile| profile.id.to_string());
            match &tab.body {
                TabBody::Query(query) => Some(TabSnapshot::Query {
                    profile_id,
                    sql: query.editor.read(cx).value().to_string(),
                }),
                TabBody::Table(table) => Some(TabSnapshot::Table {
                    profile_id,
                    schema: table.schema.clone(),
                    table: table.table.clone(),
                }),
            }
        };
        let primary = self
            .primary_tabs
            .iter()
            .filter_map(|id| self.tab_index(*id))
            .filter_map(|index| snapshot(&self.tabs[index]))
            .collect::<Vec<_>>();
        let secondary = self
            .secondary_tabs
            .iter()
            .filter_map(|id| self.tab_index(*id))
            .filter_map(|index| snapshot(&self.tabs[index]))
            .collect::<Vec<_>>();
        WorkspaceSnapshot {
            split: self.split.map(|axis| {
                match axis {
                    SplitAxis::Vertical => "vertical",
                    SplitAxis::Horizontal => "horizontal",
                }
                .to_string()
            }),
            primary_active: self
                .primary_tabs
                .iter()
                .position(|id| *id == self.primary_active)
                .unwrap_or(0),
            secondary_active: self
                .secondary_tabs
                .iter()
                .position(|id| *id == self.secondary_active)
                .unwrap_or(0),
            primary,
            secondary,
            focus_secondary: self.focus == Pane::Secondary,
        }
    }

    pub(crate) fn has_unsaved_work(&self, cx: &App) -> bool {
        self.session_transactions.values().any(|open| *open)
            || self.tabs.iter().any(|tab| match &tab.body {
                TabBody::Query(query) => query.running || query_has_unsaved_sql(query, cx),
                TabBody::Table(table) => !table.staged.is_empty() || table.exporting,
            })
    }

    pub(crate) fn confirm_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm = Some(ConfirmState {
            title: "Close Koyo?".into(),
            description: "A query or export is running, a transaction is open, a SQL editor has text, or edits are staged. Running work stops and uncommitted work is lost; saved workspace SQL is restored when possible.".into(),
            action: std::rc::Rc::new(|_, _| {}),
            close_window: true,
            return_focus: window.focused(cx),
        });
        self.confirm_focus.focus(window, cx);
        cx.notify();
    }
}
