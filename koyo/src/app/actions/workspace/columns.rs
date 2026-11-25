use super::*;

impl KoyoApp {
    pub(crate) fn begin_column_resize(
        &mut self,
        tab_id: u64,
        column: usize,
        pointer_x: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        let column_count = table
            .page
            .as_ref()
            .map(|page| page.columns.len())
            .unwrap_or(0);
        if table.column_widths.len() < column_count {
            table
                .column_widths
                .resize(column_count, DEFAULT_COLUMN_WIDTH);
        }
        if column < column_count {
            table.resize_pointer_x = Some(pointer_x);
            table.resize_column = Some(column);
        }
        cx.notify();
    }

    pub(crate) fn end_column_resize(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        if let TabBody::Table(table) = &mut self.tabs[index].body {
            table.resize_pointer_x = None;
            table.resize_column = None;
        }
        cx.notify();
    }

    pub(crate) fn take_column_resized(&mut self, tab_id: u64) -> bool {
        let Some(index) = self.tab_index(tab_id) else {
            return false;
        };
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return false;
        };
        let resized = table.column_resized;
        table.column_resized = false;
        resized
    }

    pub(crate) fn resize_column(
        &mut self,
        tab_id: u64,
        column: usize,
        pointer_x: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        if table.resize_column != Some(column) {
            return;
        }
        let Some(previous_x) = table.resize_pointer_x else {
            return;
        };
        let Some(width) = table.column_widths.get_mut(column) else {
            return;
        };
        let delta = pointer_x - previous_x;
        if delta.abs() > 0.5 {
            table.column_resized = true;
            *width = (*width + delta).clamp(MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH);
        }
        table.resize_pointer_x = Some(pointer_x);
        cx.notify();
    }

    pub(crate) fn nudge_column_width(
        &mut self,
        tab_id: u64,
        column: usize,
        delta: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        let column_count = table
            .page
            .as_ref()
            .map(|page| page.columns.len())
            .unwrap_or(0);
        if table.column_widths.len() < column_count {
            table
                .column_widths
                .resize(column_count, DEFAULT_COLUMN_WIDTH);
        }
        if let Some(width) = table.column_widths.get_mut(column) {
            *width = (*width + delta).clamp(MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH);
        }
        cx.notify();
    }

    pub(crate) fn nudge_result_column_width(
        &mut self,
        tab_id: u64,
        result_index: usize,
        column: usize,
        delta: f32,
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
        if let Some(width) = widths.get_mut(column) {
            *width = (*width + delta).clamp(MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH);
        }
        cx.notify();
    }

    pub(crate) fn autosize_column(&mut self, tab_id: u64, column: usize, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let TabBody::Table(table) = &mut self.tabs[index].body else {
            return;
        };
        let Some(page) = table.page.as_ref() else {
            return;
        };
        let Some(info) = page.columns.get(column) else {
            return;
        };
        let mut longest = info
            .name
            .chars()
            .count()
            .max(info.data_type.chars().count());
        for row in &page.rows {
            if let Some(Some(value)) = row.values.get(column) {
                longest = longest.max(value.chars().count());
            }
        }
        // Monospace text at text_sm is about 7.2 px per glyph; include the
        // cell's horizontal padding and the sort affordance.
        let width = if page.rows.is_empty() {
            DEFAULT_COLUMN_WIDTH
        } else {
            (longest as f32 * 7.2 + 44.0).clamp(MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH)
        };
        if table.column_widths.len() < page.columns.len() {
            table
                .column_widths
                .resize(page.columns.len(), DEFAULT_COLUMN_WIDTH);
        }
        if let Some(current) = table.column_widths.get_mut(column) {
            *current = width;
        }
        table.resize_pointer_x = None;
        table.resize_column = None;
        cx.notify();
    }

    pub(crate) fn resize_active_column(
        &mut self,
        tab_id: u64,
        pointer_x: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(tab_id) else {
            return;
        };
        let Some(column) = self.tabs[index]
            .body
            .table()
            .and_then(|table| table.resize_column)
        else {
            return;
        };
        self.resize_column(tab_id, column, pointer_x, cx);
    }
}
