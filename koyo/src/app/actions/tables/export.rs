use super::*;

impl KoyoApp {
    pub(crate) fn open_table_export(
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
        if table.exporting {
            return;
        }
        let Some(page) = table.page.as_ref() else {
            return;
        };
        let page_limit = page.limit.max(1);
        let current_page = page.offset / page_limit + 1;
        let total_pages = table
            .total_rows_cache
            .map(|total| {
                total
                    .div_ceil(u64::from(page_limit))
                    .min(u64::from(u32::MAX)) as u32
            })
            .unwrap_or(0);
        let page_input = cx.new(|cx| InputState::new(window, cx).placeholder("Page number"));
        let end_page_input = cx.new(|cx| InputState::new(window, cx).placeholder("End page"));
        let file_name = table_csv_file_name(&table.schema, &table.table);
        let path_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Choose where to save the CSV"));
        set_input(&path_input, &default_export_path(&file_name), window, cx);
        self.export = Some(TableExportDialog {
            tab_id,
            title: format!("{}.{}", table.schema, table.table).into(),
            current_page,
            total_pages,
            scope: ExportScope::CurrentPage,
            page: page_input,
            end_page: end_page_input,
            path: path_input,
            file_name,
            error: String::new(),
        });
        cx.notify();
    }

    pub(crate) fn close_table_export(&mut self, cx: &mut Context<Self>) {
        self.export = None;
        cx.notify();
    }

    pub(crate) fn browse_table_export_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.export.as_ref() else {
            return;
        };
        let path_input = dialog.path.clone();
        let file_name = dialog.file_name.clone();
        let Some(path) = rfd::FileDialog::new()
            .add_filter("CSV", &["csv"])
            .set_file_name(&file_name)
            .save_file()
        else {
            return;
        };
        set_input(&path_input, &path.display().to_string(), window, cx);
        cx.notify();
    }

    pub(crate) fn set_table_export_scope(&mut self, scope: ExportScope, cx: &mut Context<Self>) {
        if let Some(dialog) = self.export.as_mut() {
            dialog.scope = scope;
            dialog.error.clear();
        }
        cx.notify();
    }

    pub(crate) fn download_table_csv(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.export.as_ref() else {
            return;
        };
        let tab_id = dialog.tab_id;
        let scope = dialog.scope;
        let page_text = dialog.page.read(cx).value().to_string();
        let end_page_text = dialog.end_page.read(cx).value().to_string();
        let path_text = dialog.path.read(cx).value().trim().to_string();
        let Some(index) = self.tab_index(tab_id) else {
            self.export = None;
            return;
        };
        let Some(connection) = self.tabs[index].connection.clone() else {
            if let Some(dialog) = self.export.as_mut() {
                dialog.error = "This table is not connected.".into();
            }
            cx.notify();
            return;
        };
        let TabBody::Table(table) = &self.tabs[index].body else {
            self.export = None;
            return;
        };
        if table.exporting {
            return;
        }
        let Some((current_page, page_limit)) = table.page.as_ref().map(|page| {
            let limit = page.limit.max(1);
            (page.offset / limit + 1, limit)
        }) else {
            if let Some(dialog) = self.export.as_mut() {
                dialog.error = "Load a table page before downloading.".into();
            }
            cx.notify();
            return;
        };
        let total_pages = table
            .total_rows_cache
            .map(|total| {
                total
                    .div_ceil(u64::from(page_limit))
                    .min(u64::from(u32::MAX)) as u32
            })
            .unwrap_or(0);
        let (first_page, last_page) = match resolve_export_pages(
            scope,
            current_page,
            &page_text,
            &page_text,
            &end_page_text,
            total_pages,
        ) {
            Ok(range) => range,
            Err(error) => {
                if let Some(dialog) = self.export.as_mut() {
                    dialog.error = error;
                }
                cx.notify();
                return;
            }
        };
        let schema = table.schema.clone();
        let table_name = table.table.clone();
        let filters = table.filters.clone();
        let sort = table.sort.clone();
        let export_title = format!("{schema}.{table_name}");
        if path_text.is_empty() {
            if let Some(dialog) = self.export.as_mut() {
                dialog.error = "Choose where to save the CSV.".into();
            }
            cx.notify();
            return;
        }
        let path = PathBuf::from(path_text);
        if path.is_dir() {
            if let Some(dialog) = self.export.as_mut() {
                dialog.error = "Choose a file path, not a folder.".into();
            }
            cx.notify();
            return;
        }
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
            && !parent.is_dir()
        {
            if let Some(dialog) = self.export.as_mut() {
                dialog.error = "The selected folder does not exist.".into();
            }
            cx.notify();
            return;
        }
        let path = if path.extension().is_none() {
            path.with_extension("csv")
        } else {
            path
        };
        if let TabBody::Table(table) = &mut self.tabs[index].body {
            table.exporting = true;
            self.tabs[index].notice = format!("Exporting CSV to {}...", path.display());
        }
        self.export = None;
        self.push_toast(
            ToastCategory::Exports,
            tab_id.to_string(),
            Notification::info(format!("Exporting {export_title}.")).title("Downloading CSV"),
            cx,
        );
        cx.notify();

        let manager = self.manager.clone();
        let session_id = connection.session_id;
        let path_for_task = path.clone();
        self.spawn_task(
            cx,
            tab_id,
            async move {
                export_table_csv(
                    manager,
                    session_id,
                    schema,
                    table_name,
                    first_page,
                    last_page,
                    page_limit,
                    filters,
                    sort,
                    path_for_task,
                )
                .await
            },
            move |this, cx, result| {
                let Some(index) = this.tab_index(tab_id) else {
                    return;
                };
                if let TabBody::Table(table) = &mut this.tabs[index].body {
                    table.exporting = false;
                }
                match result {
                    Ok((rows, truncated)) => {
                        let suffix = if truncated {
                            " Some cell values were truncated by Koyo's limits."
                        } else {
                            ""
                        };
                        let message = format!(
                            "Downloaded {rows} {} to {}.{suffix}",
                            if rows == 1 { "row" } else { "rows" },
                            path.display()
                        );
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
                        log::error!("table CSV export failed on session {session_id}: {error}");
                        if database::is_connection_lost(&error) {
                            this.mark_connection_lost(
                                session_id,
                                "Connection lost while downloading CSV. Reconnect to continue.",
                                cx,
                            );
                        } else {
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
                }
                cx.notify();
            },
        );
    }
}
