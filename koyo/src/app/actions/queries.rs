use std::time::Instant;

use gpui_kit::prelude::*;
use gpui_kit::{ClipboardItem, Window};
use koyo_core::database;
use koyo_core::models::{ConnectionProfile, Engine};
use uuid::Uuid;

use crate::app::*;

impl KoyoApp {
    pub(crate) fn run_active_query(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.focused_index().and_then(|index| self.tabs.get(index)) else {
            return;
        };
        if !matches!(tab.body, TabBody::Query(_)) {
            return;
        }
        let tab_id = tab.id;
        self.run_query(tab_id, cx);
    }

    pub(crate) fn run_query(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let Some(connection) = self.tabs[index].connection.clone() else {
            self.tabs[index].notice = "Connect a database to run SQL.".into();
            cx.notify();
            return;
        };
        let TabBody::Query(query) = &self.tabs[index].body else {
            return;
        };
        if query.running {
            return;
        }
        let sql = query.editor.read(cx).value().to_string();
        if sql.trim().is_empty() {
            self.tabs[index].notice = "Enter SQL to run.".into();
            cx.notify();
            return;
        }
        let history_sql = sql.clone();
        let history_profile = self.tabs[index].profile.clone();
        let operation_id = Uuid::new_v4();
        let TabBody::Query(query) = &mut self.tabs[index].body else {
            return;
        };
        query.running = true;
        query.cancelling = false;
        query.operation_id = Some(operation_id);
        query.error.clear();
        query.output = None;
        query.duration_ms = None;
        query.result_column_widths.clear();
        self.tabs[index].notice.clear();
        cx.notify();

        let started = Instant::now();
        let manager = self.manager.clone();
        let session_id = connection.session_id;
        self.spawn_task(
            cx,
            tab_id,
            async move { database::run_sql(&manager, session_id, operation_id, sql).await },
            move |this, cx, result| {
                let Some(index) = this.tab_index(tab_id) else {
                    return;
                };
                let TabBody::Query(query) = &mut this.tabs[index].body else {
                    return;
                };
                if query.operation_id != Some(operation_id) {
                    return;
                }
                query.operation_id = None;
                query.running = false;
                query.cancelling = false;
                query.duration_ms = Some(started.elapsed().as_millis());
                let success = result.is_ok();
                match result {
                    Ok(output) => {
                        query.output = Some(output);
                        query.active_result = 0;
                        query.error.clear();
                    }
                    Err(error) => query.error = error,
                }
                let duration_ms = started.elapsed().as_millis();
                this.record_query_history(history_sql, history_profile, success, duration_ms, cx);
                cx.notify();
            },
        );
    }

    pub(crate) fn cancel_task(&mut self, tab_id: u64, table_only: bool, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let Some(connection) = self.tabs[index].connection.clone() else {
            return;
        };
        let running = match &self.tabs[index].body {
            TabBody::Query(query) => !table_only && query.running,
            TabBody::Table(table) => table.loading,
        };
        if !running {
            return;
        }
        let operation_id = match &self.tabs[index].body {
            TabBody::Query(query) => query.operation_id,
            TabBody::Table(table) => table.operation_id,
        };
        let Some(operation_id) = operation_id else {
            self.tabs[index].notice = "This operation cannot be cancelled.".into();
            cx.notify();
            return;
        };
        match &mut self.tabs[index].body {
            TabBody::Query(query) => query.cancelling = true,
            TabBody::Table(table) => table.cancelling = true,
        }
        cx.notify();

        let manager = self.manager.clone();
        let engine = self.tabs[index]
            .profile
            .as_ref()
            .map(|profile| profile.engine);
        self.spawn_task(
            cx,
            tab_id,
            async move {
                database::cancel_query(&manager, connection.session_id, operation_id).await
            },
            move |this, cx, result| {
                let Some(index) = this.tab_index(tab_id) else {
                    return;
                };
                match &mut this.tabs[index].body {
                    TabBody::Query(query) => query.cancelling = false,
                    TabBody::Table(table) => table.cancelling = false,
                }
                match result {
                    Ok(sent) => {
                        if sent && engine == Some(Engine::SqlServer) {
                            this.disconnect_tabs_for_session(
                                connection.session_id,
                                "SQL Server cancelled the statement by closing its session. Reconnect to continue.",
                                cx,
                            );
                        } else {
                            this.tabs[index].notice = "Cancellation requested.".into();
                        }
                    }
                    Err(error) => this.tabs[index].notice = error,
                }
                cx.notify();
            },
        );
    }

    pub(crate) fn copy_active_result(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let csv = match &self.tabs[index].body {
            TabBody::Query(query) => query.output.as_ref().and_then(|output| {
                let result = output.result_sets.get(
                    query
                        .active_result
                        .min(output.result_sets.len().saturating_sub(1)),
                )?;
                Some(results_as_csv(result))
            }),
            TabBody::Table(_) => None,
        };
        let Some(csv) = csv else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(csv));
        self.tabs[index].notice = "Copied active result as CSV.".into();
        cx.notify();
    }

    pub(crate) fn copy_to_clipboard(
        &mut self,
        text: String,
        message: &'static str,
        cx: &mut Context<Self>,
    ) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.status = message.into();
        cx.notify();
    }

    fn record_query_history(
        &mut self,
        sql: String,
        profile: Option<ConnectionProfile>,
        success: bool,
        duration_ms: u128,
        cx: &mut Context<Self>,
    ) {
        let sql = sql.trim_end().to_string();
        if sql.trim().is_empty() {
            return;
        }
        let profile_id = profile.as_ref().map(|profile| profile.id.to_string());
        self.history
            .retain(|entry| !(entry.sql == sql && entry.profile_id == profile_id));
        self.history.insert(
            0,
            QueryHistoryEntry {
                sql,
                profile_id,
                profile_name: profile
                    .as_ref()
                    .map(|profile| profile.name.clone())
                    .unwrap_or_else(|| "No profile".into()),
                success,
                duration_ms: duration_ms.min(u64::MAX as u128) as u64,
                at: unix_seconds(),
            },
        );
        self.history.truncate(HISTORY_LIMIT);
        self.persist_state(cx);
    }

    fn history_profile(&self, entry: &QueryHistoryEntry) -> Option<ConnectionProfile> {
        let id = entry.profile_id.as_deref()?;
        self.profiles
            .iter()
            .find(|profile| profile.id.to_string() == id)
            .cloned()
    }

    fn set_history_sql(
        &mut self,
        tab_id: u64,
        sql: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let TabBody::Query(query) = &mut self.tabs[index].body else {
            return;
        };
        query.editor.update(cx, |editor, cx| {
            editor.set_value(sql.to_string(), window, cx);
        });
        // History SQL is not a starter; a later connection must not replace it.
        query.starter = String::new().into();
        query.pending_run = false;
        cx.notify();
    }

    pub(crate) fn load_history_entry(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.history.get(index).cloned() else {
            return;
        };
        let profile = self.history_profile(&entry);
        let tab_id = if let Some(tab_index) = self.focused_index() {
            if matches!(self.tabs[tab_index].body, TabBody::Query(_))
                && !matches!(&self.tabs[tab_index].body, TabBody::Query(query) if query.running)
            {
                self.tabs[tab_index].id
            } else {
                let tab = self.make_query_tab(profile, window, cx);
                self.add_tab(tab, self.focus)
            }
        } else {
            let tab = self.make_query_tab(profile, window, cx);
            self.add_tab(tab, self.focus)
        };
        self.set_history_sql(tab_id, &entry.sql, window, cx);
        self.status = "Loaded query from history.".into();
        cx.notify();
    }

    pub(crate) fn rerun_history(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.history.get(index).cloned() else {
            return;
        };
        let desired_profile = self.history_profile(&entry);
        let desired_id = entry.profile_id.clone();
        let matches_profile = |profile: Option<&ConnectionProfile>| match (&desired_id, profile) {
            (None, _) => true,
            (Some(id), Some(profile)) => profile.id.to_string() == *id,
            (Some(_), None) => false,
        };
        let tab_id = self
            .focused_index()
            .filter(|index| match &self.tabs[*index].body {
                TabBody::Query(query) => {
                    !query.running && matches_profile(self.tabs[*index].profile.as_ref())
                }
                TabBody::Table(_) => false,
            })
            .map(|index| self.tabs[index].id)
            .unwrap_or_else(|| {
                let tab = self.make_query_tab(desired_profile.clone(), window, cx);
                self.add_tab(tab, self.focus)
            });
        self.set_history_sql(tab_id, &entry.sql, window, cx);
        let connected = self
            .tab_index(tab_id)
            .is_some_and(|index| self.tabs[index].connection.is_some());
        if connected {
            self.run_query(tab_id, cx);
        } else if let Some(profile) = desired_profile.or_else(|| {
            self.tab_index(tab_id)
                .and_then(|index| self.tabs[index].profile.clone())
        }) {
            if let Some(index) = self.tab_index(tab_id)
                && let TabBody::Query(query) = &mut self.tabs[index].body
            {
                query.pending_run = true;
            }
            self.connect_profile(profile.id, Some(tab_id), false, window, cx);
        } else {
            if let Some(index) = self.tab_index(tab_id) {
                self.tabs[index].notice = "Connect a database to re-run this query.".into();
            }
            cx.notify();
        }
    }

    pub(crate) fn request_clear_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm(
            window,
            cx,
            "Clear query history?",
            "This removes all saved SQL history from this machine.",
            |this, cx| this.clear_history(cx),
        );
    }

    pub(crate) fn clear_history(&mut self, cx: &mut Context<Self>) {
        self.history.clear();
        self.persist_state(cx);
        cx.notify();
    }

    pub(crate) fn begin_result_column_resize(
        &mut self,
        tab_id: u64,
        result_index: usize,
        column: usize,
        pointer_x: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let TabBody::Query(query) = &mut self.tabs[index].body else {
            return;
        };
        let column_count = query
            .output
            .as_ref()
            .and_then(|output| output.result_sets.get(result_index))
            .map(|result| result.columns.len())
            .unwrap_or(0);
        if query.result_column_widths.len() <= result_index {
            query
                .result_column_widths
                .resize(result_index + 1, Vec::new());
        }
        let widths = &mut query.result_column_widths[result_index];
        if widths.len() < column_count {
            widths.resize(column_count, RESULT_COLUMN_WIDTH);
        }
        if column < column_count {
            query.result_resize_pointer_x = Some(pointer_x);
        }
        cx.notify();
    }

    pub(crate) fn resize_result_column(
        &mut self,
        tab_id: u64,
        result_index: usize,
        column: usize,
        pointer_x: f32,
        fallback_origin_x: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let TabBody::Query(query) = &mut self.tabs[index].body else {
            return;
        };
        let previous_x = query.result_resize_pointer_x.unwrap_or(fallback_origin_x);
        let Some(width) = query
            .result_column_widths
            .get_mut(result_index)
            .and_then(|widths| widths.get_mut(column))
        else {
            return;
        };
        let delta = pointer_x - previous_x;
        if delta.abs() > 0.5 {
            *width = (*width + delta).clamp(MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH);
        }
        query.result_resize_pointer_x = Some(pointer_x);
        cx.notify();
    }

    pub(crate) fn end_result_column_resize(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        if let TabBody::Query(query) = &mut self.tabs[index].body {
            query.result_resize_pointer_x = None;
        }
        cx.notify();
    }

    pub(crate) fn toggle_results(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        if let TabBody::Query(query) = &mut self.tabs[index].body {
            query.results_collapsed = !query.results_collapsed;
            cx.notify();
        }
    }

    pub(crate) fn set_result_view(
        &mut self,
        tab_id: u64,
        view: ResultView,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        if let TabBody::Query(query) = &mut self.tabs[index].body {
            query.result_view = view;
            cx.notify();
        }
    }

    pub(crate) fn set_active_result(&mut self, tab_id: u64, index: usize, cx: &mut Context<Self>) {
        let Some(tab_index) = self.tab_index(tab_id) else {
            return;
        };
        if let TabBody::Query(query) = &mut self.tabs[tab_index].body {
            query.active_result = index;
        }
        cx.notify();
    }
}
