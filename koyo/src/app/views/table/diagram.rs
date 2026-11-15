use gpui_kit::component::{Icon, IconName, Sizable, Size};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use gpui_kit::{InteractiveElement, StatefulInteractiveElement};
use koyo_core::models::TablePage;

use crate::app::*;

fn relation_card(
    schema: &str,
    table: &str,
    column: &str,
    palette: &Palette,
    highlight: bool,
) -> AnyElement {
    v_flex()
        .w(px(210.0))
        .flex_shrink_0()
        .gap_1()
        .p_3()
        .rounded_lg()
        .border_1()
        .border_color(if highlight {
            palette.accent
        } else {
            palette.border
        })
        .bg(if highlight {
            palette.accent_soft
        } else {
            palette.panel
        })
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(format!("{schema}.{table}")),
        )
        .child(
            div()
                .text_xs()
                .text_color(palette.muted)
                .child(column.to_string()),
        )
        .into_any_element()
}

pub(super) fn render_diagram(
    tab_id: u64,
    table: &TableTab,
    page: Option<&TablePage>,
    palette: &Palette,
) -> AnyElement {
    let mut content = v_flex()
        .id(("table-diagram", tab_id))
        .flex_1()
        .min_h(px(0.0))
        .overflow_y_scroll()
        .p_5()
        .gap_4();
    if let Some(page) = page {
        let mut columns = v_flex().gap_1();
        for column in page.columns.iter().take(12) {
            columns = columns.child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(180.0))
                            .text_sm()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(column.name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.muted)
                            .child(column.data_type.clone()),
                    )
                    .when(column.primary_key, |this| {
                        this.child(
                            div()
                                .px_1()
                                .rounded_sm()
                                .bg(palette.panel_alt)
                                .text_xs()
                                .text_color(palette.warning)
                                .child("PK"),
                        )
                    }),
            );
        }
        content = content.child(
            v_flex()
                .gap_2()
                .p_4()
                .rounded_lg()
                .border_1()
                .border_color(palette.accent)
                .bg(palette.panel)
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("{}.{}", table.schema, table.table)),
                )
                .child(columns),
        );
    }

    let title = h_flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child("Relationships"),
        )
        .child(
            div()
                .text_xs()
                .text_color(palette.muted)
                .child(format!("{}", table.foreign_keys.len())),
        );
    content = content.child(title);

    if table.foreign_keys_loading {
        content = content.child(
            div()
                .text_xs()
                .text_color(palette.muted)
                .child("Loading relationships..."),
        );
    } else if table.foreign_keys.is_empty() {
        content = content.child(
            div()
                .p_4()
                .rounded_lg()
                .border_1()
                .border_color(palette.border)
                .bg(palette.panel)
                .text_xs()
                .text_color(palette.muted)
                .child("No foreign keys point at or from this table."),
        );
    } else {
        for fk in &table.foreign_keys {
            let outgoing = fk.schema == table.schema && fk.table == table.table;
            let (left_schema, left_table, left_column) = if outgoing {
                (fk.schema.as_str(), fk.table.as_str(), fk.column.as_str())
            } else {
                (
                    fk.referenced_schema.as_str(),
                    fk.referenced_table.as_str(),
                    fk.referenced_column.as_str(),
                )
            };
            let (right_schema, right_table, right_column) = if outgoing {
                (
                    fk.referenced_schema.as_str(),
                    fk.referenced_table.as_str(),
                    fk.referenced_column.as_str(),
                )
            } else {
                (fk.schema.as_str(), fk.table.as_str(), fk.column.as_str())
            };
            content = content.child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(relation_card(
                        left_schema,
                        left_table,
                        left_column,
                        palette,
                        outgoing,
                    ))
                    .child(
                        v_flex()
                            .w(px(90.0))
                            .flex_shrink_0()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .w_full()
                                    .text_xs()
                                    .text_color(palette.muted)
                                    .text_center()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(fk.constraint_name.clone()),
                            )
                            .child(
                                h_flex()
                                    .items_center()
                                    .w_full()
                                    .child(div().flex_1().h(px(1.0)).bg(palette.border))
                                    .child(
                                        Icon::new(IconName::ChevronRight)
                                            .with_size(Size::Small)
                                            .text_color(palette.muted),
                                    ),
                            ),
                    )
                    .child(relation_card(
                        right_schema,
                        right_table,
                        right_column,
                        palette,
                        !outgoing,
                    )),
            );
        }
    }

    content
        .child(
            div()
                .text_xs()
                .text_color(palette.muted)
                .child("Drag the Results divider or this pane split to resize."),
        )
        .into_any_element()
}
