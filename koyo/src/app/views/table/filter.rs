use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::{Disableable, IconName};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use koyo_core::models::{ColumnInfo, FilterKind, FilterOperator, TableFilter};

use super::super::widgets::button_tooltip;
use crate::app::*;

pub(super) fn render_filter_bar(
    tab_id: u64,
    table: &TableTab,
    columns: &[ColumnInfo],
    selected_column: Option<usize>,
    palette: &Palette,
    cx: &mut Context<KoyoApp>,
) -> AnyElement {
    if columns.is_empty() {
        return div().into_any_element();
    }
    let locked = table.loading || !table.staged.is_empty();
    let selected = selected_column.and_then(|index| columns.get(index));
    let kind = selected
        .map(|column| column.filter_kind)
        .unwrap_or(FilterKind::Text);
    let operators = operator_options(kind);
    let needs_value = filter_needs_value(table.filter_operator);
    let operator = table.filter_operator;

    let mut bar = v_flex()
        .px_5()
        .py_2()
        .gap_2()
        .border_b_1()
        .border_color(palette.border)
        .bg(palette.sidebar);

    bar = bar.child(
        h_flex()
            .items_center()
            .gap_2()
            .flex_wrap()
            .child(div().text_xs().text_color(palette.muted).child("Filter"))
            .children(columns.iter().enumerate().map(|(index, column)| {
                Button::new(("filter-column", index))
                    .compact()
                    .ghost()
                    .label(column.name.clone())
                    .when(selected_column == Some(index), |this| this.primary())
                    .disabled(locked)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_filter_column(tab_id, index, window, cx);
                    }))
            })),
    );

    if selected.is_some() {
        bar = bar.child(
            h_flex()
                .items_center()
                .gap_2()
                .flex_wrap()
                .children(operators.iter().map(|(op, label)| {
                    let op = *op;
                    Button::new(SharedString::from(format!("filter-op-{op:?}")))
                        .compact()
                        .ghost()
                        .label(*label)
                        .when(op == operator, |this| this.primary())
                        .disabled(locked)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_filter_operator(tab_id, op, cx);
                        }))
                }))
                .child(
                    div().w(px(220.0)).child(
                        Input::new(&table.filter_value)
                            .w_full()
                            .disabled(locked || !needs_value),
                    ),
                )
                .child(
                    Button::new("apply-filter")
                        .primary()
                        .compact()
                        .icon(IconName::Search)
                        .label("Apply filter")
                        .disabled(locked)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.apply_filter(tab_id, window, cx);
                        })),
                )
                .child(
                    Button::new("clear-filters")
                        .secondary()
                        .compact()
                        .label("Clear")
                        .disabled(table.loading)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.request_clear_filters(tab_id, window, cx);
                        })),
                ),
        );
    }

    let filter_hint = if !table.filter_error.is_empty() {
        Some((table.filter_error.clone(), palette.danger))
    } else if !table.staged.is_empty() {
        Some((
            "Apply or discard staged edits to change filters.".to_string(),
            palette.warning,
        ))
    } else if table.loading {
        Some(("Loading filtered rows...".to_string(), palette.muted))
    } else {
        None
    };
    if let Some((message, color)) = filter_hint {
        bar = bar.child(div().text_xs().text_color(color).child(message));
    }

    if !table.filters.is_empty() {
        bar = bar.child(h_flex().items_center().gap_1().flex_wrap().children(
            table.filters.iter().map(|filter| {
                let column = filter.column.clone();
                h_flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .bg(palette.panel)
                    .text_xs()
                    .child(filter_label(filter))
                    .child(button_tooltip(
                        format!("tooltip-remove-filter-{}", filter.column),
                        Button::new(SharedString::from(format!(
                            "remove-filter-{}",
                            filter.column
                        )))
                        .ghost()
                        .compact()
                        .icon(IconName::Close)
                        .disabled(table.loading)
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.request_remove_filter(tab_id, column.clone(), window, cx);
                            },
                        )),
                        "Remove filter",
                    ))
            }),
        ));
    }

    bar.into_any_element()
}

fn filter_label(filter: &TableFilter) -> String {
    let operator = match filter.operator {
        FilterOperator::Contains => "contains",
        FilterOperator::Equals => "=",
        FilterOperator::NotEquals => "≠",
        FilterOperator::LessThan => "<",
        FilterOperator::LessThanOrEqual => "≤",
        FilterOperator::GreaterThan => ">",
        FilterOperator::GreaterThanOrEqual => "≥",
        FilterOperator::IsNull => "is null",
        FilterOperator::IsNotNull => "is not null",
        FilterOperator::IsTrue => "is true",
        FilterOperator::IsFalse => "is false",
    };
    match &filter.value {
        Some(value) => format!("{} {} {}", filter.column, operator, value),
        None => format!("{} {}", filter.column, operator),
    }
}
