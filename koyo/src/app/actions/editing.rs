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
        if !page.editable {
            return;
        }
        let Some(info) = page.columns.get(column) else {
            return;
        };
        let Some(row_data) = page.rows.get(row) else {
            return;
        };
        if info.generated || info.identity {
            return;
        }
        if key_is_truncated(page, row_data) {
            this_notice(
                &mut self.tabs[index],
                "The primary key was truncated for display. Filter the row before editing it.",
            );
            cx.notify();
            return;
        }
        if row_data.truncated.get(column).copied().unwrap_or(false) {
            this_notice(
                &mut self.tabs[index],
                "This cell was truncated for display. Filter the row before editing it.",
            );
            cx.notify();
            return;
        }
        let original_value = row_data.values.get(column).cloned().flatten();
        let current = table
            .staged
            .iter()
            .find_map(|change| match change {
                StagedChange::Update { key, values, .. } if key == &row_data.key => Some(
                    values
                        .get(&info.name)
                        .cloned()
                        .unwrap_or_else(|| original_value.clone()),
                ),
                _ => None,
            })
            .unwrap_or_else(|| original_value.clone());
        let is_null = current.is_none();
        let text = current.unwrap_or_default();
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Value")
                .default_value(text)
                .auto_grow(6, 18)
        });
        let handle = input.read(cx).focus_handle(cx);
        self.cell_edit = Some(CellEdit {
            tab_id,
            target: CellTarget::Update {
                key: row_data.key.clone(),
                version: row_data.version.clone(),
                original: row_data.values.clone(),
                truncated: row_data.truncated.clone(),
                column: info.name.clone(),
                original_value,
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
        if !page.editable {
            return;
        }
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

    pub(crate) fn close_cell_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cell_edit = None;
        self.root_focus.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn apply_cell_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
                truncated,
                column,
                original_value,
            } => CellTarget::Update {
                key: key.clone(),
                version: version.clone(),
                original: original.clone(),
                truncated: truncated.clone(),
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
        self.root_focus.focus(window, cx);
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
        match target {
            CellTarget::Update {
                key,
                version,
                original,
                truncated,
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
                        truncated,
                    });
                }
            }
            CellTarget::Insert { local_id, column } => {
                let Some(column_name) = table
                    .page
                    .as_ref()
                    .and_then(|page| page.columns.get(column))
                    .map(|info| info.name.clone())
                else {
                    return;
                };
                let Some(change) = table.staged.iter_mut().find_map(|change| match change {
                    StagedChange::Insert(insert) if insert.local_id == local_id => Some(insert),
                    _ => None,
                }) else {
                    return;
                };
                if is_default {
                    change.values.remove(&column_name);
                } else if is_null {
                    change.values.insert(column_name, None);
                } else {
                    change.values.insert(column_name, Some(value));
                }
            }
        }
        cx.notify();
    }
}
