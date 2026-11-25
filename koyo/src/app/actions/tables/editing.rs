use super::*;

impl KoyoApp {
    pub(crate) fn apply_changes(
        &mut self,
        tab_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let TabBody::Table(table) = &self.tabs[index].body else {
            return;
        };
        let deletes = table
            .staged
            .iter()
            .filter(|change| matches!(change, StagedChange::Delete { .. }))
            .count();
        if deletes > 0 {
            self.confirm(
                window,
                cx,
                "Delete staged rows?",
                "Apply permanently deletes the marked rows. Other staged edits are applied in the same transaction.",
                move |this, cx| this.commit_staged_changes(tab_id, cx),
            );
        } else {
            self.commit_staged_changes(tab_id, cx);
        }
    }

    fn commit_staged_changes(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let Some(connection) = self.tabs[index].connection.clone() else {
            return;
        };
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        if table.staged.is_empty() || table.loading {
            return;
        }
        let schema = table.schema.clone();
        let name = table.table.clone();
        let offset = table.page.as_ref().map(|page| page.offset).unwrap_or(0);
        let filters = table.filters.clone();
        let sort = table.sort.clone();
        let changes = table
            .staged
            .iter()
            .map(|change| match change.clone() {
                StagedChange::Insert(insert) => GridChange::Insert {
                    values: insert.values,
                },
                StagedChange::Update {
                    key,
                    version,
                    values,
                    original,
                    truncated,
                } => GridChange::Update {
                    key,
                    version,
                    values,
                    original,
                    truncated,
                },
                StagedChange::Delete {
                    key,
                    version,
                    original,
                    truncated,
                } => GridChange::Delete {
                    key,
                    version,
                    original,
                    truncated,
                },
            })
            .collect::<Vec<_>>();
        table.loading = true;
        let operation_id = Uuid::new_v4();
        table.operation_id = Some(operation_id);
        cx.notify();

        let manager = self.manager.clone();
        let session_id = connection.session_id;
        self.spawn_task(
            cx,
            tab_id,
            async move {
                database::apply_grid_changes(
                    &manager,
                    session_id,
                    operation_id,
                    schema,
                    name,
                    changes,
                )
                .await
            },
            move |this, cx, result| {
                let Some(index) = this.tab_index(tab_id) else {
                    return;
                };
                let TabBody::Table(table) = &mut this.tabs[index].body else {
                    return;
                };
                if table.operation_id != Some(operation_id) {
                    return;
                }
                table.operation_id = None;
                table.loading = false;
                match result {
                    Ok(summary) => {
                        table.staged.clear();
                        table.selected_keys.clear();
                        table.total_rows_cache = None;
                        let message = format!(
                            "{} staged {} applied.",
                            summary.applied,
                            if summary.applied == 1 { "change" } else { "changes" }
                        );
                        this.tabs[index].notice = message.clone();
                        this.push_toast(
                            ToastCategory::Data,
                            tab_id.to_string(),
                            Notification::success(message).title("Changes applied"),
                            cx,
                        );
                        this.load_page(tab_id, offset, filters, sort, cx);
                    }
                    Err(error) => {
                        log::error!("applying staged changes failed on session {session_id}: {error}");
                        if database::is_connection_lost(&error) {
                            this.mark_connection_lost(
                                session_id,
                                "Connection lost while applying changes. Reconnect to retry.",
                                cx,
                            );
                        } else if error.starts_with("COMMIT_OUTCOME_UNKNOWN:") {
                            table.staged.clear();
                            table.selected_keys.clear();
                            let message = "Commit result could not be confirmed. Page reloaded; verify data before retrying.";
                            this.tabs[index].notice = message.into();
                            this.push_toast(
                                ToastCategory::Data,
                                tab_id.to_string(),
                                Notification::warning(
                                    "Verify the data before retrying.",
                                )
                                .title("Commit unconfirmed")
                                .autohide(false),
                                cx,
                            );
                            this.load_page(tab_id, offset, filters, sort, cx);
                        } else {
                            this.tabs[index].notice = error.clone();
                            this.push_toast(
                                ToastCategory::Data,
                                tab_id.to_string(),
                                Notification::error(
                                    "No changes were applied. See the tab for details.",
                                )
                                .title("Save failed")
                                .autohide(false),
                                cx,
                            );
                        }
                    }
                }
                cx.notify();
            },
        );
    }

    pub(crate) fn discard_changes(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        self.tabs[index].notice.clear();
        if let TabBody::Table(table) = &mut self.tabs[index].body {
            table.staged.clear();
        }
        cx.notify();
    }

    pub(crate) fn set_row_selected(
        &mut self,
        tab_id: u64,
        row_index: usize,
        selected: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let connected = self.tabs[index].connection.is_some();
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        if !connected || table.loading {
            return;
        }
        let Some((row, key_truncated)) =
            table
                .page
                .as_ref()
                .filter(|page| page.editable)
                .and_then(|page| {
                    let row = page.rows.get(row_index)?;
                    Some((row.clone(), key_is_truncated(page, row)))
                })
        else {
            return;
        };
        let already_deleted = table
            .staged
            .iter()
            .any(|change| matches!(change, StagedChange::Delete { key, .. } if key == &row.key));
        if key_truncated || already_deleted {
            return;
        }
        if selected {
            table.selected_keys.insert(row.key);
        } else {
            table.selected_keys.remove(&row.key);
        }
        cx.notify();
    }

    pub(crate) fn set_all_rows_selected(
        &mut self,
        tab_id: u64,
        selected: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let connected = self.tabs[index].connection.is_some();
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        if !connected || table.loading {
            return;
        }
        table.selected_keys.clear();
        if !selected {
            cx.notify();
            return;
        }
        let Some(page) = table.page.as_ref().filter(|page| page.editable) else {
            return;
        };
        for row in &page.rows {
            let already_deleted = table.staged.iter().any(
                |change| matches!(change, StagedChange::Delete { key, .. } if key == &row.key),
            );
            if !already_deleted && !key_is_truncated(page, row) {
                table.selected_keys.insert(row.key.clone());
            }
        }
        cx.notify();
    }

    pub(crate) fn delete_selected_rows(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let connected = self.tabs[index].connection.is_some();
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        if !connected || table.loading || table.selected_keys.is_empty() {
            return;
        }
        let Some(page) = table.page.as_ref().filter(|page| page.editable) else {
            return;
        };
        let rows = page.rows.clone();
        let mut staged = 0usize;
        let mut skipped = 0usize;
        for row in rows {
            if !table.selected_keys.contains(&row.key) {
                continue;
            }
            if key_is_truncated(page, &row) {
                skipped += 1;
                continue;
            }
            if stage_row_delete_change(
                &mut table.staged,
                row.key,
                row.version,
                row.values,
                row.truncated,
            ) {
                staged += 1;
            }
        }
        table.selected_keys.clear();
        this_notice(
            &mut self.tabs[index],
            &if skipped > 0 {
                format!(
                    "{staged} row{} marked for deletion; {skipped} skipped because the primary key was truncated.",
                    if staged == 1 { "" } else { "s" }
                )
            } else {
                format!(
                    "{staged} row{} marked for deletion. Apply changes to delete {}.",
                    if staged == 1 { "" } else { "s" },
                    if staged == 1 { "it" } else { "them" }
                )
            },
        );
        cx.notify();
    }

    pub(crate) fn clear_row_selection(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        if let TabBody::Table(table) = &mut self.tabs[index].body {
            table.selected_keys.clear();
        }
        cx.notify();
    }

    pub(crate) fn toggle_row_delete(
        &mut self,
        tab_id: u64,
        row_index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let connected = self.tabs[index].connection.is_some();
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        if !connected || table.loading {
            return;
        }
        let Some((row, key_truncated)) =
            table
                .page
                .as_ref()
                .filter(|page| page.editable)
                .and_then(|page| {
                    let row = page.rows.get(row_index)?;
                    let key_truncated = page.columns.iter().enumerate().any(|(index, column)| {
                        column.primary_key && row.truncated.get(index).copied().unwrap_or(false)
                    });
                    Some((row.clone(), key_truncated))
                })
        else {
            return;
        };
        if key_truncated {
            this_notice(
                &mut self.tabs[index],
                "The primary key was truncated for display. Filter the row before deleting it.",
            );
            cx.notify();
            return;
        }
        let deleted = stage_row_delete(
            &mut table.staged,
            row.key.clone(),
            row.version,
            row.values,
            row.truncated,
        );
        if deleted {
            table.selected_keys.remove(&row.key);
        }
        this_notice(
            &mut self.tabs[index],
            if deleted {
                "Row marked for deletion. Apply changes to delete it."
            } else {
                "Row deletion undone."
            },
        );
        cx.notify();
    }

    pub(crate) fn add_row(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        self.tabs[index].notice.clear();
        let editable = self.tabs[index]
            .body
            .table()
            .and_then(|table| table.page.as_ref())
            .is_some_and(|page| page.editable);
        if !editable || self.tabs[index].connection.is_none() {
            return;
        }
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        let local_id = table.next_insert_id;
        table.next_insert_id += 1;
        table.staged.push(StagedChange::Insert(StagedInsert {
            local_id,
            values: BTreeMap::new(),
        }));
        cx.notify();
    }
}
