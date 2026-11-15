use std::time::Instant;

use gpui_kit::prelude::*;
use koyo_core::database;
use koyo_core::models::Engine;

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
        let TabBody::Query(query) = &mut self.tabs[index].body else {
            return;
        };
        query.running = true;
        query.cancelling = false;
        query.error.clear();
        query.output = None;
        query.duration_ms = None;
        self.tabs[index].notice.clear();
        cx.notify();

        let started = Instant::now();
        let manager = self.manager.clone();
        let session_id = connection.session_id;
        self.spawn_task(
            cx,
            async move { database::run_sql(&manager, session_id, sql).await },
            move |this, cx, result| {
                let Some(index) = this.tab_index(tab_id) else {
                    return;
                };
                let TabBody::Query(query) = &mut this.tabs[index].body else {
                    return;
                };
                query.running = false;
                query.cancelling = false;
                query.duration_ms = Some(started.elapsed().as_millis());
                match result {
                    Ok(output) => {
                        query.output = Some(output);
                        query.active_result = 0;
                        query.error.clear();
                    }
                    Err(error) => query.error = error,
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
            async move { database::cancel_query(&manager, connection.session_id).await },
            move |this, cx, result| {
                let Some(index) = this.tab_index(tab_id) else {
                    return;
                };
                match &mut this.tabs[index].body {
                    TabBody::Query(query) => query.cancelling = false,
                    TabBody::Table(table) => table.cancelling = false,
                }
                match result {
                    Ok(()) => {
                        if engine == Some(Engine::SqlServer) {
                            this.disconnect_now(tab_id, cx);
                            this.tabs[index].notice =
                                "SQL Server cancelled the statement by closing its session. Reconnect to continue."
                                    .into();
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
