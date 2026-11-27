use std::ops::Range;
use std::time::{Duration, Instant};

use gpui_kit::component::input::EditorState;
use gpui_kit::component::notification::Notification;
use gpui_kit::prelude::*;
use gpui_kit::{ClipboardItem, Entity, Focusable, Window};
use koyo_core::database;
use koyo_core::models::{ConnectionProfile, Engine, QueryOutput, SchemaObjectKind};
use uuid::Uuid;

use super::notifications::ToastCategory;
use crate::app::*;

const QUERY_TIMEOUT: Duration = Duration::from_secs(600);

fn definition_from_output(
    engine: Engine,
    kind: SchemaObjectKind,
    output: &QueryOutput,
) -> Option<String> {
    if engine == Engine::Sqlite && kind == SchemaObjectKind::Function {
        return None;
    }
    let result = output.result_sets.first()?;
    let column = result.columns.iter().position(|name| {
        let name = name.to_lowercase();
        name == "definition"
            || name == "sql"
            || name.contains("create function")
            || name.contains("create trigger")
            || name.contains("sql original statement")
    })?;
    let parts = result
        .rows
        .iter()
        .filter_map(|row| row.get(column).and_then(Option::as_deref))
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return None;
    }
    Some(if engine == Engine::Oracle {
        parts.join("\n")
    } else {
        parts.join("\n\n")
    })
}

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

    pub(crate) fn format_active_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.focused_index().and_then(|index| self.tabs.get(index)) else {
            return;
        };
        if !matches!(tab.body, TabBody::Query(_)) {
            return;
        }
        let tab_id = tab.id;
        self.format_query(tab_id, window, cx);
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
            // Formatting is a normal edit, not a document reset: keep one
            // undo step so Ctrl/Cmd+Z restores the pre-format SQL.
            editor.replace_all(next, window, cx);
            editor.set_selected_range(selection, cx);
        });
        query.editor.read(cx).focus_handle(cx).focus(window, cx);
    }

    pub(crate) fn run_query(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let TabBody::Query(query) = &self.tabs[index].body else {
            return;
        };
        if query.running {
            return;
        }
        if self.tabs[index].connection.is_some() {
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
            if let Some(risk) = sql_risk(&sql) {
                let (title, description) = match risk {
                    SqlRisk::UnfilteredDelete => (
                        "Run DELETE without WHERE?",
                        "This statement has no WHERE clause and can delete every row from its target table. Run it?",
                    ),
                    SqlRisk::UnfilteredUpdate => (
                        "Run UPDATE without WHERE?",
                        "This statement has no WHERE clause and can update every row in its target table. Run it?",
                    ),
                    SqlRisk::DestructiveDdl => (
                        "Run destructive DDL?",
                        "TRUNCATE or DROP can permanently remove data. Run it?",
                    ),
                    SqlRisk::DynamicSql => (
                        "Run dynamic SQL?",
                        "EXECUTE and DO blocks can hide destructive statements. Run it?",
                    ),
                };
                self.confirm_deferred(
                    title,
                    description,
                    move |this, cx| this.run_query_sql(tab_id, cx),
                    cx,
                );
                return;
            }
        }
        self.run_query_sql(tab_id, cx);
    }

    fn run_query_sql(&mut self, tab_id: u64, cx: &mut Context<Self>) {
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
        let transaction_sql = sql.clone();
        let history_profile = self.tabs[index].profile.clone();
        let operation_id = Uuid::new_v4();
        let TabBody::Query(query) = &mut self.tabs[index].body else {
            return;
        };
        query.running = true;
        query.cancelling = false;
        query.operation_id = Some(operation_id);
        query.error.clear();
        // Keep the previous result visible while the next statement runs.
        // Replacing it with an empty state made every Run collapse the grid.
        query.duration_ms = None;
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
            {
                let cancel_manager = manager.clone();
                async move {
                    super::connections::run_with_cancel(
                        QUERY_TIMEOUT,
                        "Query timed out after 10 minutes. Cancel was requested.",
                        &cancel_manager,
                        session_id,
                        operation_id,
                        async move {
                            database::run_sql(&manager, session_id, operation_id, sql).await
                        },
                    )
                    .await
                }
            },
            move |this, cx, result| {
                let Some(index) = this.tab_index(tab_id) else {
                    return;
                };
                let engine = this.tabs[index].engine().unwrap_or(Engine::Postgres);
                let TabBody::Query(query) = &mut this.tabs[index].body else {
                    return;
                };
                if query.operation_id != Some(operation_id) {
                    return;
                }
                query.operation_id = None;
                query.running = false;
                let cancelled = query.cancelling;
                query.cancelling = false;
                let duration_ms = started.elapsed().as_millis();
                query.duration_ms = Some(duration_ms);
                let success = result.is_ok();
                let definition_kind = query.definition_kind;
                let is_definition = definition_kind.is_some();
                let toast = if is_definition {
                    None
                } else {
                    match &result {
                        Ok(output) => {
                            let returned = output
                                .result_sets
                                .iter()
                                .map(|result| result.rows.len())
                                .sum::<usize>();
                            let affected = output.affected_rows.iter().copied().sum::<u64>();
                            let detail = if returned > 0 {
                                format!(
                                    "{returned} {} in {duration_ms} ms",
                                    if returned == 1 { "row" } else { "rows" }
                                )
                            } else if affected > 0 {
                                format!(
                                    "{affected} {} affected in {duration_ms} ms",
                                    if affected == 1 { "row" } else { "rows" }
                                )
                            } else {
                                format!("Statement finished in {duration_ms} ms")
                            };
                            Some(Notification::success(detail).title("Query complete"))
                        }
                        Err(error) if !cancelled && !database::is_connection_lost(error) => Some(
                            Notification::error("See the tab for details.")
                                .title("Query failed")
                                .autohide(false),
                        ),
                        Err(_) => None,
                    }
                };
                let mut pending_definition = None;
                let lost = match result {
                    Ok(output) => {
                        pending_definition = definition_kind
                            .and_then(|kind| definition_from_output(engine, kind, &output));
                        query.definition_kind = None;
                        query.output = Some(output);
                        query.active_result = 0;
                        query.error.clear();
                        if pending_definition.is_some() {
                            query.results_collapsed = true;
                        }
                        None
                    }
                    Err(error) => {
                        query.definition_kind = None;
                        if !cancelled {
                            log::error!("query failed on session {session_id}: {error}");
                        }
                        if database::is_connection_lost(&error) {
                            Some(error)
                        } else {
                            query.error = error;
                            None
                        }
                    }
                };
                if let Some(definition) = pending_definition {
                    this.tabs[index].pending_starter = Some(definition.into());
                }
                if !is_definition {
                    this.record_query_history(
                        history_sql,
                        history_profile,
                        success,
                        duration_ms,
                        cx,
                    );
                }
                if let Some(error) = lost {
                    this.mark_connection_lost(session_id, &format!("Connection lost: {error}"), cx);
                } else {
                    let open = this.session_transaction_open(session_id);
                    this.set_session_transaction(
                        session_id,
                        transaction_state_after(open, &transaction_sql),
                        cx,
                    );
                    if let Some(toast) = toast {
                        this.push_toast(ToastCategory::Queries, tab_id.to_string(), toast, cx);
                    }
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
                                true,
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

    pub(crate) fn focused_query_editor(&self) -> Option<Entity<EditorState>> {
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
        _window: &mut Window,
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
                Some((result.clone(), output.truncated))
            }),
            TabBody::Table(_) => None,
        };
        let Some((result, truncated)) = exported else {
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
        let path_for_task = path.clone();
        self.spawn_task(
            cx,
            tab_id,
            async move {
                let csv = results_as_csv(&result);
                write_private_export(&path_for_task, csv.as_bytes())
            },
            move |this, cx, result| {
                let Some(index) = this.tab_index(tab_id) else {
                    return;
                };
                let suffix = if truncated {
                    " The result was truncated; the file contains the rows Koyo returned."
                } else {
                    ""
                };
                match result {
                    Ok(()) => {
                        let message =
                            format!("Downloaded active result to {}.{suffix}", path.display());
                        this.tabs[index].notice = message;
                        let file_name = path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("CSV");
                        this.push_toast(
                            ToastCategory::Exports,
                            tab_id.to_string(),
                            Notification::success(format!("Saved {file_name}.")).title("CSV ready"),
                            cx,
                        );
                    }
                    Err(error) => {
                        log::error!("CSV download failed: {error}");
                        let message = format!("CSV download failed: {error}");
                        this.tabs[index].notice = message.clone();
                        this.push_toast(
                            ToastCategory::Exports,
                            tab_id.to_string(),
                            Notification::error("Export failed. See the tab for details.")
                                .title("CSV failed")
                                .autohide(false),
                            cx,
                        );
                    }
                }
                cx.notify();
            },
        );
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

    pub(crate) fn query_table(
        &mut self,
        engine: Engine,
        schema: &str,
        table: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_sql_tab(window, cx);
        let Some(tab_id) = self.pane_active_id(self.focus) else {
            return;
        };
        let sql = table_preview_sql(engine, schema, table);
        self.set_history_sql(tab_id, &sql, window, cx);
        self.focus_query_editor(tab_id, window, cx);
        self.status = format!("Loaded a preview query for {schema}.{table}.");
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
        self.focus_query_editor(tab_id, window, cx);
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

#[cfg(test)]
mod tests {
    use super::definition_from_output;
    use koyo_core::models::{Engine, QueryOutput, QueryResultSet, SchemaObjectKind};

    fn output(columns: &[&str], rows: &[&[Option<&str>]]) -> QueryOutput {
        QueryOutput {
            result_sets: vec![QueryResultSet {
                columns: columns.iter().map(|name| (*name).into()).collect(),
                rows: rows
                    .iter()
                    .map(|row| row.iter().map(|value| value.map(str::to_string)).collect())
                    .collect(),
            }],
            affected_rows: Vec::new(),
            truncated: false,
        }
    }

    #[test]
    fn extracts_postgres_function_definition() {
        let output = output(
            &["definition"],
            &[&[Some(
                "CREATE FUNCTION get_child_locs() RETURNS void AS $$ BEGIN END $$;",
            )]],
        );
        assert_eq!(
            definition_from_output(Engine::Postgres, SchemaObjectKind::Function, &output)
                .as_deref(),
            Some("CREATE FUNCTION get_child_locs() RETURNS void AS $$ BEGIN END $$;")
        );
    }

    #[test]
    fn extracts_mysql_show_create_column() {
        let output = output(
            &["Function", "sql_mode", "Create Function"],
            &[&[
                Some("f"),
                Some(""),
                Some("CREATE FUNCTION f() RETURNS int RETURN 1"),
            ]],
        );
        assert_eq!(
            definition_from_output(Engine::Mysql, SchemaObjectKind::Function, &output).as_deref(),
            Some("CREATE FUNCTION f() RETURNS int RETURN 1")
        );
    }

    #[test]
    fn joins_oracle_source_lines() {
        let output = output(
            &["DEFINITION"],
            &[
                &[Some("FUNCTION f RETURN NUMBER IS")],
                &[Some("BEGIN RETURN 1; END;")],
            ],
        );
        assert_eq!(
            definition_from_output(Engine::Oracle, SchemaObjectKind::Function, &output).as_deref(),
            Some("FUNCTION f RETURN NUMBER IS\nBEGIN RETURN 1; END;")
        );
    }

    #[test]
    fn leaves_sqlite_function_names_alone() {
        let output = output(&["name"], &[&[Some("f")]]);
        assert_eq!(
            definition_from_output(Engine::Sqlite, SchemaObjectKind::Function, &output),
            None
        );
    }
}
