use std::ops::Range;
use std::time::Instant;

use gpui_kit::component::WindowExt;
use gpui_kit::component::input::EditorState;
use gpui_kit::component::notification::Notification;
use gpui_kit::prelude::*;
use gpui_kit::{ClipboardItem, Entity, Window};
use koyo_core::database;
use koyo_core::models::{ConnectionProfile, Engine};
use uuid::Uuid;

use crate::app::*;

struct QueryCsvToast;

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

    pub(crate) fn format_query(
        &mut self,
        tab_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let engine = self.tabs[index]
            .engine()
            .unwrap_or(koyo_core::models::Engine::Sqlite);
        let TabBody::Query(query) = &mut self.tabs[index].body else {
            return;
        };
        let (value, range, cursor) = {
            let editor = query.editor.read(cx);
            (
                editor.value().to_string(),
                editor.selected_range(),
                editor.cursor(),
            )
        };
        if value.trim().is_empty() {
            return;
        }
        let (source, selected) = if range.is_empty() {
            (value.as_str(), false)
        } else {
            (&value[range.clone()], true)
        };
        if source.trim().is_empty() {
            return;
        }
        let formatted = format_sql(engine, source);
        if formatted == source {
            return;
        }
        let (next, selection) = if selected {
            let start = range.start;
            let end = range.end;
            let mut next = String::with_capacity(value.len() + formatted.len());
            next.push_str(&value[..start]);
            next.push_str(&formatted);
            next.push_str(&value[end..]);
            let end = start + formatted.len();
            (next, start..end)
        } else {
            let cursor = formatted.floor_char_boundary(cursor.min(formatted.len()));
            let next = formatted;
            let selection = cursor..cursor;
            (next, selection)
        };
        query.editor.update(cx, |editor, cx| {
            editor.set_value(next, window, cx);
            editor.set_selected_range(selection, cx);
        });
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
        let (selected, full) = {
            let editor = query.editor.read(cx);
            (
                editor.selected_value().to_string(),
                editor.value().to_string(),
            )
        };
        let sql = if selected.trim().is_empty() {
            full
        } else {
            selected
        };
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
        query.result_resize_pointer_x = None;
        query.result_resize_column = None;
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
                let lost = match result {
                    Ok(output) => {
                        query.output = Some(output);
                        query.active_result = 0;
                        query.error.clear();
                        None
                    }
                    Err(error) => {
                        if database::is_connection_lost(&error) {
                            Some(error)
                        } else {
                            query.error = error;
                            None
                        }
                    }
                };
                let duration_ms = started.elapsed().as_millis();
                this.record_query_history(history_sql, history_profile, success, duration_ms, cx);
                if let Some(error) = lost {
                    this.mark_connection_lost(session_id, &format!("Connection lost: {error}"), cx);
                }
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

    fn focused_query_editor(&self) -> Option<Entity<EditorState>> {
        self.focused_index().and_then(|index| {
            self.tabs.get(index).and_then(|tab| match &tab.body {
                TabBody::Query(query) => Some(query.editor.clone()),
                TabBody::Table(_) => None,
            })
        })
    }

    fn apply_editor_edit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        edit: fn(&str, Range<usize>) -> EditorEdit,
    ) {
        let Some(editor) = self.focused_query_editor() else {
            return;
        };
        editor.update(cx, |editor, cx| {
            let text = editor.value().to_string();
            let selection = editor.selected_range();
            let result = edit(&text, selection.clone());
            if result.text == text && result.selection == selection {
                return;
            }
            editor.set_selected_range(0..text.len(), cx);
            editor.replace(result.text, window, cx);
            let len = editor.value().len();
            let start = result.selection.start.min(len);
            let end = result.selection.end.min(len);
            editor.set_selected_range(start..end, cx);
        });
    }

    pub(crate) fn toggle_sql_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_editor_edit(window, cx, toggle_comment);
    }

    pub(crate) fn move_line_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_editor_edit(window, cx, move_line_up);
    }

    pub(crate) fn move_line_down(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_editor_edit(window, cx, move_line_down);
    }

    pub(crate) fn duplicate_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_editor_edit(window, cx, duplicate_line);
    }

    pub(crate) fn delete_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_editor_edit(window, cx, delete_line);
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

    pub(crate) fn download_active_result(
        &mut self,
        tab_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let exported = match &self.tabs[index].body {
            TabBody::Query(query) => query.output.as_ref().and_then(|output| {
                let result = output.result_sets.get(
                    query
                        .active_result
                        .min(output.result_sets.len().saturating_sub(1)),
                )?;
                Some((results_as_csv(result), output.truncated))
            }),
            TabBody::Table(_) => None,
        };
        let Some((csv, truncated)) = exported else {
            return;
        };
        let Some(path) = rfd::FileDialog::new()
            .add_filter("CSV", &["csv"])
            .set_file_name("query-result.csv")
            .save_file()
        else {
            return;
        };
        let path = if path.extension().is_none() {
            path.with_extension("csv")
        } else {
            path
        };
        let suffix = if truncated {
            " The result was truncated; the file contains the rows Koyo returned."
        } else {
            ""
        };
        match std::fs::write(&path, csv) {
            Ok(()) => {
                let message = format!("Downloaded active result to {}.{suffix}", path.display());
                self.tabs[index].notice = message.clone();
                window.push_notification(Notification::success(message).title("CSV ready"), cx);
            }
            Err(error) => {
                let message = format!("CSV download failed: {error}");
                self.tabs[index].notice = message.clone();
                window.push_notification(
                    Notification::error(message)
                        .title("CSV failed")
                        .id::<QueryCsvToast>(),
                    cx,
                );
            }
        }
        cx.notify();
    }

    pub(crate) fn copy_active_result_json(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let json = match &self.tabs[index].body {
            TabBody::Query(query) => query.output.as_ref().and_then(|output| {
                let result = output.result_sets.get(
                    query
                        .active_result
                        .min(output.result_sets.len().saturating_sub(1)),
                )?;
                Some(results_as_json(result))
            }),
            TabBody::Table(_) => None,
        };
        let Some(json) = json else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(json));
        self.tabs[index].notice = "Copied active result as JSON.".into();
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
            query.result_resize_column = Some((result_index, column));
        }
        cx.notify();
    }

    pub(crate) fn resize_active_result_column(
        &mut self,
        tab_id: u64,
        pointer_x: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let TabBody::Query(query) = &self.tabs[index].body else {
            return;
        };
        let Some((result_index, column)) = query.result_resize_column else {
            return;
        };
        self.resize_result_column(tab_id, result_index, column, pointer_x, cx);
    }

    pub(crate) fn resize_result_column(
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
        if query.result_resize_column != Some((result_index, column)) {
            return;
        }
        let Some(previous_x) = query.result_resize_pointer_x else {
            return;
        };
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
            query.result_resize_column = None;
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
