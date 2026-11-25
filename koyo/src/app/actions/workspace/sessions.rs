use super::*;

impl KoyoApp {
    pub(crate) fn spawn_task<T, F>(
        &self,
        cx: &mut Context<Self>,
        tab_id: u64,
        task: impl Future<Output = T> + Send + 'static,
        done: F,
    ) where
        T: Send + 'static,
        F: FnOnce(&mut Self, &mut Context<Self>, T) + 'static,
    {
        let handle = self.runtime.spawn(task);
        cx.spawn(async move |this, cx| match handle.await {
            Ok(value) => {
                let _ = this.update(cx, |this, cx| done(this, cx, value));
            }
            Err(error) => {
                let message = format!("Background task failed unexpectedly: {error}");
                let _ = this.update(cx, |this, cx| this.fail_task(tab_id, message, cx));
            }
        })
        .detach();
    }

    pub(crate) fn fail_task(&mut self, tab_id: u64, error: String, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            if let Some(dialog) = self.dialog.as_mut() {
                dialog.busy = false;
                dialog.error = error.clone();
            }
            self.status = error;
            cx.notify();
            return;
        };
        self.tabs[index].connecting = false;
        self.tabs[index].notice = error.clone();
        match &mut self.tabs[index].body {
            TabBody::Query(query) => {
                query.running = false;
                query.cancelling = false;
                query.operation_id = None;
                query.pending_run = false;
                query.error = error.clone();
            }
            TabBody::Table(table) => {
                table.loading = false;
                table.cancelling = false;
                table.operation_id = None;
                table.foreign_keys_loading = false;
                table.exporting = false;
            }
        }
        if let Some(dialog) = self.dialog.as_mut() {
            dialog.busy = false;
            dialog.error = error.clone();
        }
        self.status = error;
        cx.notify();
    }

    pub(crate) fn disconnect_session(&mut self, session_id: Uuid) {
        self.catalog_generations.remove(&session_id);
        self.session_transactions.remove(&session_id);
        let manager = self.manager.clone();
        self.runtime.spawn(async move {
            let _ = database::disconnect(&manager, session_id).await;
        });
    }

    /// Split panes share one live session. Only the last tab holding a
    /// session may tear it down.
    pub(super) fn session_is_used(
        connections: impl IntoIterator<Item = (u64, Option<Uuid>)>,
        session_id: Uuid,
        except_tab: Option<u64>,
    ) -> bool {
        connections.into_iter().any(|(tab_id, connection)| {
            Some(tab_id) != except_tab && connection == Some(session_id)
        })
    }

    pub(super) fn session_in_use(&self, session_id: Uuid, except_tab: Option<u64>) -> bool {
        Self::session_is_used(
            self.tabs.iter().map(|tab| {
                (
                    tab.id,
                    tab.connection
                        .as_ref()
                        .map(|connection| connection.session_id),
                )
            }),
            session_id,
            except_tab,
        )
    }

    pub(crate) fn disconnect_session_if_unused(
        &mut self,
        session_id: Uuid,
        except_tab: Option<u64>,
    ) {
        if !self.session_in_use(session_id, except_tab) {
            self.disconnect_session(session_id);
        }
    }

    pub(super) fn tab_catalog(&self, index: usize) -> CatalogSnapshot {
        let tab = &self.tabs[index];
        CatalogSnapshot {
            tables: tab.tables.clone(),
            schema_objects: tab.schema_objects.clone(),
            metadata_groups: tab.metadata_groups.clone(),
            columns: tab.columns.clone(),
        }
    }

    /// One live session per profile is reused across panes. A reconnect from
    /// any tab should attach the same connection to every disconnected tab for
    /// that profile, including the explorer's source tab.
    pub(crate) fn connection_for_profile(
        &mut self,
        profile: &ConnectionProfile,
    ) -> Option<(ConnectionInfo, CatalogSnapshot)> {
        if let Some((index, tab)) = self.tabs.iter().enumerate().find(|(_, tab)| {
            tab.connection.as_ref().is_some_and(|connection| {
                connection.profile.id == profile.id
                    && same_connection_target(&connection.profile, profile)
            })
        }) {
            let connection = tab.connection.clone()?;
            return Some((connection, self.tab_catalog(index)));
        }
        loop {
            let parked = self.parked_connections.iter().position(|parked| {
                parked.profile.id == profile.id && same_connection_target(&parked.profile, profile)
            })?;
            let parked = self.parked_connections.remove(parked);
            if parked.parked_at.elapsed() > std::time::Duration::from_secs(self.idle_pool_ttl_secs)
            {
                self.disconnect_session(parked.connection.session_id);
                continue;
            }
            return Some((
                parked.connection,
                CatalogSnapshot {
                    tables: parked.tables,
                    schema_objects: parked.schema_objects,
                    metadata_groups: parked.metadata_groups,
                    columns: parked.columns,
                },
            ));
        }
    }

    pub(crate) fn profile_tab_ids(&self, profile_id: Uuid) -> Vec<u64> {
        self.tabs
            .iter()
            .filter(|tab| {
                tab.profile
                    .as_ref()
                    .is_some_and(|profile| profile.id == profile_id)
                    || tab
                        .connection
                        .as_ref()
                        .is_some_and(|connection| connection.profile.id == profile_id)
            })
            .map(|tab| tab.id)
            .collect()
    }

    fn profile_session_ids(&self, profile_id: Uuid) -> Vec<Uuid> {
        let mut sessions = self
            .tabs
            .iter()
            .filter_map(|tab| {
                tab.connection
                    .as_ref()
                    .filter(|connection| connection.profile.id == profile_id)
                    .map(|connection| connection.session_id)
            })
            .chain(
                self.parked_connections
                    .iter()
                    .filter(|parked| parked.profile.id == profile_id)
                    .map(|parked| parked.connection.session_id),
            )
            .collect::<Vec<_>>();
        sessions.sort_unstable();
        sessions.dedup();
        sessions
    }

    fn profile_needs_disconnect_confirm(&self, profile_id: Uuid) -> bool {
        self.tabs
            .iter()
            .filter(|tab| {
                tab.profile
                    .as_ref()
                    .is_some_and(|profile| profile.id == profile_id)
                    || tab
                        .connection
                        .as_ref()
                        .is_some_and(|connection| connection.profile.id == profile_id)
            })
            .any(|tab| match &tab.body {
                TabBody::Query(query) => query.running,
                TabBody::Table(table) => !table.staged.is_empty(),
            })
            || self
                .profile_session_ids(profile_id)
                .into_iter()
                .any(|session_id| self.session_transaction_open(session_id))
    }

    fn disconnect_profile(
        &mut self,
        profile_id: Uuid,
        notice: &str,
        reason: DisconnectReason,
        keep_staged: bool,
        cx: &mut Context<Self>,
    ) {
        let ids = self.profile_tab_ids(profile_id);
        if ids.is_empty() {
            return;
        }
        let sessions = self.profile_session_ids(profile_id);
        self.parked_connections
            .retain(|parked| parked.profile.id != profile_id);
        if self
            .cell_edit
            .as_ref()
            .is_some_and(|edit| ids.contains(&edit.tab_id))
        {
            self.cell_edit = None;
        }
        if self
            .export
            .as_ref()
            .is_some_and(|dialog| ids.contains(&dialog.tab_id))
        {
            self.export = None;
        }
        let dialog_target_in_group = self
            .dialog
            .as_ref()
            .and_then(|dialog| dialog.target_tab)
            .is_some_and(|tab_id| ids.contains(&tab_id));
        for tab_id in ids {
            let Some(index) = self.tab_index(tab_id) else {
                continue;
            };
            let had_staged = self.tabs[index]
                .body
                .table()
                .is_some_and(|table| !table.staged.is_empty());
            self.tabs[index].connection = None;
            self.tabs[index].connecting = false;
            self.tabs[index].catalog_loading = false;
            self.tabs[index].notice = if had_staged && !keep_staged {
                format!("{notice} Staged edits in this tab were discarded.")
            } else {
                notice.to_string()
            };
            match &mut self.tabs[index].body {
                TabBody::Query(query) => {
                    query.running = false;
                    query.cancelling = false;
                    query.operation_id = None;
                    query.pending_run = false;
                }
                TabBody::Table(table) => {
                    table.loading = false;
                    table.cancelling = false;
                    table.operation_id = None;
                    table.foreign_keys_loading = false;
                    table.exporting = false;
                    if !keep_staged {
                        table.staged.clear();
                    }
                    table.selected_keys.clear();
                }
            }
        }
        for session_id in &sessions {
            self.catalog_generations.remove(session_id);
            self.session_transactions.remove(session_id);
            self.disconnect_session(*session_id);
        }
        if dialog_target_in_group && let Some(dialog) = self.dialog.as_mut() {
            dialog.busy = false;
            dialog.error = notice.to_string();
        }
        self.status = notice.to_string();
        let toast = match reason {
            DisconnectReason::Manual => {
                Notification::info(notice.to_string()).title("Disconnected")
            }
            DisconnectReason::Lost => Notification::error(notice.to_string())
                .title("Connection lost")
                .autohide(false),
            DisconnectReason::SessionClosed => Notification::warning(notice.to_string())
                .title("Session closed")
                .autohide(false),
        };
        self.push_toast(
            ToastCategory::Connections,
            profile_id.to_string(),
            toast,
            cx,
        );
        cx.notify();
    }

    pub(crate) fn profile_is_connecting(&self, target: &ConnectionProfile) -> bool {
        self.tabs.iter().any(|tab| {
            tab.connecting
                && tab.profile.as_ref().is_some_and(|profile| {
                    profile.id == target.id && same_connection_target(profile, target)
                })
        })
    }

    pub(crate) fn attach_connection_to_tab(
        &mut self,
        tab_id: u64,
        connection: ConnectionInfo,
        catalog: CatalogSnapshot,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(index) = self.tab_index(tab_id) else {
            return false;
        };
        if self.tabs[index].connection.is_some() {
            return false;
        }
        let preserve_staged = self.tabs[index]
            .profile
            .as_ref()
            .is_some_and(|profile| same_connection_target(profile, &connection.profile));
        let had_staged = self.tabs[index]
            .body
            .table()
            .is_some_and(|table| !table.staged.is_empty());
        let profile = connection.profile.clone();
        self.tabs[index].connection = Some(connection);
        self.tabs[index].connecting = false;
        self.tabs[index].profile = Some(profile);
        self.tabs[index].notice.clear();
        self.tabs[index].tables = catalog.tables.clone();
        self.tabs[index].schema_objects = catalog.schema_objects.clone();
        self.tabs[index].metadata_groups = catalog.metadata_groups.clone();
        self.tabs[index].columns = catalog.columns.clone();
        let mut table_view = None;
        match &mut self.tabs[index].body {
            TabBody::Query(query) => {
                query.completion.borrow_mut().replace(
                    &catalog.tables,
                    &catalog.schema_objects,
                    &catalog.columns,
                );
                query.error.clear();
                query.output = None;
                query.duration_ms = None;
                query.running = false;
                query.cancelling = false;
                query.operation_id = None;
                query.pending_run = false;
                query.active_result = 0;
            }
            TabBody::Table(table) => {
                table_view = Some((
                    table.page.as_ref().map(|page| page.offset).unwrap_or(0),
                    table.filters.clone(),
                    table.sort.clone(),
                ));
                table.page = None;
                table.total_rows_cache = None;
                if !preserve_staged {
                    table.staged.clear();
                }
                table.selected_keys.clear();
                table.loading = false;
                table.operation_id = None;
                table.foreign_keys.clear();
                table.foreign_keys_loading = false;
                table.indexes.clear();
                table.diagram_metadata_loaded = false;
            }
        }
        if had_staged {
            self.tabs[index].notice = if preserve_staged {
                "Reconnected. Staged edits kept; review before applying.".into()
            } else {
                "Reconnected. Staged edits discarded because the connection target changed.".into()
            };
        }
        if self
            .cell_edit
            .as_ref()
            .is_some_and(|edit| edit.tab_id == tab_id)
        {
            self.cell_edit = None;
        }
        if self
            .export
            .as_ref()
            .is_some_and(|dialog| dialog.tab_id == tab_id)
        {
            self.export = None;
        }
        if self.dialog.as_ref().and_then(|dialog| dialog.target_tab) == Some(tab_id) {
            self.dialog = None;
        }
        if let Some((offset, filters, sort)) = table_view {
            self.load_page(tab_id, offset, filters, sort, cx);
        }
        true
    }

    pub(crate) fn session_transaction_open(&self, session_id: Uuid) -> bool {
        self.session_transactions
            .get(&session_id)
            .copied()
            .unwrap_or(false)
    }

    pub(crate) fn set_session_transaction(
        &mut self,
        session_id: Uuid,
        open: bool,
        cx: &mut Context<Self>,
    ) {
        if self.session_transactions.insert(session_id, open) != Some(open) {
            cx.notify();
        }
    }

    fn session_is_busy(&self, session_id: Uuid) -> bool {
        self.tabs.iter().any(|tab| {
            tab.connection
                .as_ref()
                .is_some_and(|connection| connection.session_id == session_id)
                && (tab.catalog_loading
                    || tab
                        .body
                        .table()
                        .is_some_and(|table| table.foreign_keys_loading)
                    || match &tab.body {
                        TabBody::Query(query) => query.running || query.cancelling,
                        TabBody::Table(table) => {
                            table.loading || table.cancelling || table.exporting
                        }
                    })
        })
    }

    fn session_is_tracked(&self, session_id: Uuid) -> bool {
        self.tabs.iter().any(|tab| {
            tab.connection
                .as_ref()
                .is_some_and(|connection| connection.session_id == session_id)
        }) || self
            .parked_connections
            .iter()
            .any(|parked| parked.connection.session_id == session_id)
    }

    fn probe_connections(&mut self, cx: &mut Context<Self>) {
        let mut sessions = Vec::new();
        for tab in &self.tabs {
            if let Some(connection) = &tab.connection
                && !self.session_is_busy(connection.session_id)
                && !sessions.contains(&connection.session_id)
            {
                sessions.push(connection.session_id);
            }
        }
        for parked in &self.parked_connections {
            if !sessions.contains(&parked.connection.session_id) {
                sessions.push(parked.connection.session_id);
            }
        }
        for session_id in sessions {
            let manager = self.manager.clone();
            cx.spawn(async move |this, cx| {
                if database::ping(&manager, session_id).await.is_ok() {
                    return;
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(1))
                    .await;
                if database::ping(&manager, session_id).await.is_ok() {
                    return;
                }
                log::warn!("session {session_id} failed two refocus pings");
                let _ = this.update(cx, |this, cx| {
                    if !this.session_is_tracked(session_id) || this.session_is_busy(session_id) {
                        return;
                    }
                    this.mark_connection_lost(
                        session_id,
                        "Connection lost while Koyo was in the background. Reconnect to continue.",
                        cx,
                    );
                });
            })
            .detach();
        }
    }

    pub(super) fn handle_window_activation(&mut self, active: bool, cx: &mut Context<Self>) {
        if !active {
            self.inactive_since
                .get_or_insert_with(std::time::Instant::now);
            return;
        }
        let Some(inactive_since) = self.inactive_since.take() else {
            return;
        };
        if inactive_since.elapsed() >= INACTIVE_CONNECTION_RECHECK {
            self.probe_connections(cx);
        }
    }

    /// An engine-reported dead session. A lost session takes its whole
    /// profile group down so no pane keeps using a half-dead connection.
    /// Staged edits stay available for a same-target reconnect.
    pub(crate) fn mark_connection_lost(
        &mut self,
        session_id: Uuid,
        notice: &str,
        cx: &mut Context<Self>,
    ) {
        let profile_id = self
            .tabs
            .iter()
            .find(|tab| {
                tab.connection
                    .as_ref()
                    .is_some_and(|connection| connection.session_id == session_id)
            })
            .and_then(|tab| tab.profile.as_ref().map(|profile| profile.id));
        if let Some(profile_id) = profile_id {
            self.disconnect_profile(profile_id, notice, DisconnectReason::Lost, true, cx);
            return;
        }
        self.parked_connections
            .retain(|parked| parked.connection.session_id != session_id);
        self.catalog_generations.remove(&session_id);
        self.disconnect_session(session_id);
        self.status = notice.to_string();
        cx.notify();
    }

    /// Disconnect a closed engine session. If its profile has other tabs,
    /// take the whole profile group down so the UI cannot show a live pane and
    /// a disconnected explorer for the same profile.
    pub(crate) fn disconnect_tabs_for_session(
        &mut self,
        session_id: Uuid,
        notice: &str,
        warning: bool,
        cx: &mut Context<Self>,
    ) {
        let profile_id = self
            .tabs
            .iter()
            .find(|tab| {
                tab.connection
                    .as_ref()
                    .is_some_and(|connection| connection.session_id == session_id)
            })
            .and_then(|tab| tab.profile.as_ref().map(|profile| profile.id));
        if let Some(profile_id) = profile_id {
            self.disconnect_profile(
                profile_id,
                notice,
                if warning {
                    DisconnectReason::SessionClosed
                } else {
                    DisconnectReason::Manual
                },
                warning,
                cx,
            );
            return;
        }
        self.parked_connections
            .retain(|parked| parked.connection.session_id != session_id);
        self.catalog_generations.remove(&session_id);
        self.disconnect_session(session_id);
        self.status = notice.to_string();
        cx.notify();
    }

    pub(crate) fn request_disconnect(
        &mut self,
        tab_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let Some(profile) = self.tabs[index]
            .connection
            .as_ref()
            .map(|connection| connection.profile.clone())
            .or_else(|| self.tabs[index].profile.clone())
        else {
            return;
        };
        let needs_confirm = self.profile_needs_disconnect_confirm(profile.id);
        let notice = format!(
            "Disconnected from {}. Reconnect from any tab.",
            profile.name
        );
        if needs_confirm {
            let confirm_notice = notice.clone();
            let tab_count = self.profile_tab_ids(profile.id).len();
            self.confirm(
                window,
                cx,
                "Disconnect?",
                format!(
                    "A query is running or edits are staged in one of {tab_count} tabs on this profile. Disconnecting all of them discards those edits."
                ),
                move |this, cx| {
                    this.disconnect_profile(
                        profile.id,
                        &confirm_notice,
                        DisconnectReason::Manual,
                        false,
                        cx,
                    );
                },
            );
        } else {
            self.disconnect_profile(profile.id, &notice, DisconnectReason::Manual, false, cx);
        }
    }

    pub(super) fn spawn_pool_probe(&self, session_id: Uuid, cx: &mut Context<Self>) {
        let manager = self.manager.clone();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(PARKED_CONNECTION_PROBE)
                    .await;
                let parked = this
                    .update(cx, |this, _| {
                        this.parked_connections
                            .iter()
                            .any(|parked| parked.connection.session_id == session_id)
                    })
                    .unwrap_or(false);
                if !parked {
                    break;
                }
                let alive = database::ping(&manager, session_id).await.is_ok();
                let keep = this.update(cx, |this, cx| {
                    let Some(index) = this
                        .parked_connections
                        .iter()
                        .position(|parked| parked.connection.session_id == session_id)
                    else {
                        return false;
                    };
                    let expired = this.parked_connections[index].parked_at.elapsed()
                        > std::time::Duration::from_secs(this.idle_pool_ttl_secs);
                    if alive && !expired {
                        return true;
                    }
                    let parked = this.parked_connections.remove(index);
                    this.disconnect_session(parked.connection.session_id);
                    cx.notify();
                    false
                });
                if !matches!(keep, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }
}
