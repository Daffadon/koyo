use gpui_kit::component::input::TextareaState;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::app::*;

impl KoyoApp {
    pub(crate) fn open_cell_editor(
        &mut self,
        tab_id: u64,
        row: usize,
        column: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        if self.tabs[index].connection.is_none() {
            return;
        }
        let TabBody::Table(table) = &self.tabs[index].body else {
            return;
        };
        if table.loading {
            return;
        }
        let Some(page) = table.page.as_ref() else {
            return;
        };
        let Some(info) = page.columns.get(column) else {
            return;
        };
        if page.rows.get(row).is_none() || info.generated || info.identity {
            return;
        }
        if table
            .staged
            .iter()
            .any(|change| matches!(change, StagedChange::Delete { key, .. } if key == &page.rows[row].key))
        {
            return;
        }
        let current = table
            .staged
            .iter()
            .find_map(|change| match change {
                StagedChange::Update { key, values, .. } if key == &page.rows[row].key => Some(
                    values
                        .get(&info.name)
                        .cloned()
                        .unwrap_or_else(|| page.rows[row].values[column].clone()),
                ),
                _ => None,
            })
            .unwrap_or_else(|| page.rows[row].values[column].clone());
        let is_null = current.is_none();
        let text = current.unwrap_or_default();
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Value")
                .default_value(text)
                .auto_grow(6, 18)
        });
        let handle = input.read(cx).focus_handle(cx);
        let row_data = &page.rows[row];
        self.cell_edit = Some(CellEdit {
            tab_id,
            target: CellTarget::Update {
                key: row_data.key.clone(),
                version: row_data.version.clone(),
                original: row_data.values.clone(),
                column: info.name.clone(),
                original_value: row_data.values[column].clone(),
            },
            input,
            is_null,
            insert_is_default: false,
            title: format!("Edit {}", info.name).into(),
            column_name: info.name.clone(),
            nullable: info.nullable,
        });
        handle.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn open_insert_cell_editor(
        &mut self,
        tab_id: u64,
        local_id: u64,
        column: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let TabBody::Table(table) = &self.tabs[index].body else {
            return;
        };
        if table.loading {
            return;
        }
        let Some(page) = table.page.as_ref() else {
            return;
        };
        let Some(info) = page.columns.get(column) else {
            return;
        };
        if info.generated || info.identity {
            return;
        }
        let Some(insert) = table.staged.iter().find_map(|change| match change {
            StagedChange::Insert(insert) if insert.local_id == local_id => Some(insert),
            _ => None,
        }) else {
            return;
        };
        let existing = insert.values.get(&info.name).cloned();
        let (is_default, is_null, text) = match existing {
            None => (true, false, String::new()),
            Some(None) => (false, true, String::new()),
            Some(Some(value)) => (false, false, value),
        };
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Value")
                .default_value(text)
                .auto_grow(6, 18)
        });
        let handle = input.read(cx).focus_handle(cx);
        self.cell_edit = Some(CellEdit {
            tab_id,
            target: CellTarget::Insert { local_id, column },
            input,
            is_null,
            insert_is_default: is_default,
            title: format!("New value for {}", info.name).into(),
            column_name: info.name.clone(),
            nullable: info.nullable,
        });
        handle.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn set_cell_null(&mut self, is_null: bool, cx: &mut Context<Self>) {
        if let Some(edit) = self.cell_edit.as_mut() {
            edit.is_null = is_null;
            if is_null {
                edit.insert_is_default = false;
            }
        }
        cx.notify();
    }

    pub(crate) fn set_cell_default(&mut self, is_default: bool, cx: &mut Context<Self>) {
        if let Some(edit) = self.cell_edit.as_mut() {
            edit.insert_is_default = is_default;
            if is_default {
                edit.is_null = false;
            }
        }
        cx.notify();
    }

    pub(crate) fn close_cell_editor(&mut self, cx: &mut Context<Self>) {
        self.cell_edit = None;
        cx.notify();
    }

    pub(crate) fn apply_cell_editor(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self.cell_edit.as_ref() else {
            return;
        };
        if !edit.nullable && edit.is_null && !edit.insert_is_default {
            return;
        }
        let value = edit.input.read(cx).value().to_string();
        let tab_id = edit.tab_id;
        let target = match &edit.target {
            CellTarget::Update {
                key,
                version,
                original,
                column,
                original_value,
            } => CellTarget::Update {
                key: key.clone(),
                version: version.clone(),
                original: original.clone(),
                column: column.clone(),
                original_value: original_value.clone(),
            },
            CellTarget::Insert { local_id, column } => CellTarget::Insert {
                local_id: *local_id,
                column: *column,
            },
        };
        let is_null = edit.is_null;
        let is_default = edit.insert_is_default;
        self.cell_edit = None;
        self.update_cell_value(tab_id, target, is_null, is_default, value, cx);
        cx.notify();
    }

    fn update_cell_value(
        &mut self,
        tab_id: u64,
        target: CellTarget,
        is_null: bool,
        is_default: bool,
        value: String,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        let page = table.page.clone();
        match target {
            CellTarget::Update {
                key,
                version,
                original,
                column,
                original_value,
            } => {
                let existing = table.staged.iter().position(|change| match change {
                    StagedChange::Update {
                        key: staged_key, ..
                    } => staged_key == &key,
                    _ => false,
                });
                let mut values = existing
                    .and_then(|index| match &table.staged[index] {
                        StagedChange::Update { values, .. } => Some(values.clone()),
                        _ => None,
                    })
                    .unwrap_or_default();
                let new_value = if is_null { None } else { Some(value) };
                if original_value == new_value {
                    values.remove(&column);
                } else {
                    values.insert(column, new_value);
                }
                if let Some(index) = existing {
                    table.staged.remove(index);
                }
                if !values.is_empty() {
                    table.staged.push(StagedChange::Update {
                        key,
                        version,
                        values,
                        original,
                    });
                }
            }
            CellTarget::Insert { local_id, column } => {
                let Some(page) = page else {
                    return;
                };
                let Some(info) = page.columns.get(column) else {
                    return;
                };
                let Some(change) = table.staged.iter_mut().find_map(|change| match change {
                    StagedChange::Insert(insert) if insert.local_id == local_id => Some(insert),
                    _ => None,
                }) else {
                    return;
                };
                if is_default {
                    change.values.remove(&info.name);
                } else if is_null {
                    change.values.insert(info.name.clone(), None);
                } else {
                    change.values.insert(info.name.clone(), Some(value));
                }
            }
        }
        cx.notify();
    }

    pub(crate) fn toggle_row_delete(
        &mut self,
        tab_id: u64,
        row: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let TabBody::Table(table) = &self.tabs[index].body else {
            return;
        };
        let Some(page) = table.page.as_ref() else {
            return;
        };
        let Some(row_data) = page.rows.get(row) else {
            return;
        };
        let key = row_data.key.clone();
        let existing = table.staged.iter().position(|change| match change {
            StagedChange::Delete {
                key: staged_key, ..
            } => staged_key == &key,
            StagedChange::Update {
                key: staged_key, ..
            } => staged_key == &key,
            _ => false,
        });
        match existing {
            Some(position) if matches!(table.staged[position], StagedChange::Delete { .. }) => {
                self.stage_row_delete(tab_id, row, false, cx);
            }
            Some(_) => {
                self.confirm(
                    window,
                    cx,
                    "Stage deletion?",
                    "Staged edits to this row will be discarded.",
                    move |this, cx| this.stage_row_delete(tab_id, row, true, cx),
                );
            }
            None => {
                self.stage_row_delete(tab_id, row, true, cx);
            }
        }
    }

    fn stage_row_delete(&mut self, tab_id: u64, row: usize, delete: bool, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        self.tabs[index].notice.clear();
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        let Some(page) = table.page.clone() else {
            return;
        };
        let Some(row_data) = page.rows.get(row).cloned() else {
            return;
        };
        let key = row_data.key.clone();
        table.staged.retain(|change| match change {
            StagedChange::Insert(_) => true,
            _ => change.row_key() != Some(key.as_slice()),
        });
        if delete {
            table.staged.push(StagedChange::Delete {
                key,
                version: row_data.version,
                original: row_data.values,
            });
        }
        cx.notify();
    }
}
