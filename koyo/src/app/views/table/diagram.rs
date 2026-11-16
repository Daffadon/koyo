use gpui_kit::component::{Icon, IconName, Sizable, Size};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use koyo_core::models::{ColumnInfo, ForeignKeyInfo, TablePage};

use crate::app::*;

fn column_row(
    column: &ColumnInfo,
    foreign_key: bool,
    mono: &SharedString,
    palette: &Palette,
    last: bool,
) -> AnyElement {
    h_flex()
        .items_center()
        .gap_3()
        .px_3()
        .py_1()
        .when(!last, |this| this.border_b_1().border_color(palette.border))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .text_sm()
                .font_family(mono.clone())
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(column.name.clone()),
        )
        .child(
            div()
                .w(px(124.0))
                .flex_shrink_0()
                .text_right()
                .text_xs()
                .font_family(mono.clone())
                .text_color(palette.muted)
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(column.data_type.clone()),
        )
        .when(column.primary_key, |this| {
            this.child(
                div()
                    .px_2()
                    .py_0p5()
                    .rounded_sm()
                    .bg(palette.accent_soft)
                    .text_xs()
                    .font_family(mono.clone())
                    .text_color(palette.accent)
                    .child("PK"),
            )
        })
        .when(foreign_key, |this| {
            this.child(
                div()
                    .px_2()
                    .py_0p5()
                    .rounded_sm()
                    .bg(palette.warning.opacity(0.16))
                    .text_xs()
                    .font_family(mono.clone())
                    .text_color(palette.warning)
                    .child("FK"),
            )
        })
        .into_any_element()
}

fn table_sheet(
    table: &TableTab,
    page: Option<&TablePage>,
    palette: &Palette,
    mono: &SharedString,
) -> AnyElement {
    let column_count = page.map(|page| page.columns.len()).unwrap_or(0);
    let visible_columns = page
        .map(|page| page.columns.iter().take(12).collect::<Vec<_>>())
        .unwrap_or_default();

    let mut body = v_flex();
    for (index, column) in visible_columns.iter().enumerate() {
        let foreign_key = table.foreign_keys.iter().any(|fk| {
            fk.schema == table.schema && fk.table == table.table && fk.column == column.name
        });
        body = body.child(column_row(
            column,
            foreign_key,
            mono,
            palette,
            index + 1 == visible_columns.len(),
        ));
    }
    if column_count == 0 {
        body = body.child(
            div()
                .px_3()
                .py_4()
                .text_xs()
                .text_color(palette.muted)
                .child("No column metadata loaded."),
        );
    }

    v_flex()
        .w(px(420.0))
        .flex_shrink_0()
        .rounded_md()
        .border_1()
        .border_color(palette.border)
        .bg(palette.panel)
        .overflow_hidden()
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .gap_3()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(palette.border)
                .child(
                    h_flex()
                        .min_w(px(0.0))
                        .items_center()
                        .gap_2()
                        .child(
                            Icon::new(IconName::LayoutDashboard)
                                .with_size(Size::Small)
                                .text_color(palette.muted),
                        )
                        .child(
                            div()
                                .min_w(px(0.0))
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .font_family(mono.clone())
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(format!("{}.{}", table.schema, table.table)),
                        ),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(palette.muted)
                        .child(format!(
                            "{column_count} {}",
                            if column_count == 1 {
                                "column"
                            } else {
                                "columns"
                            }
                        )),
                ),
        )
        .child(body)
        .when(column_count > 12, |this| {
            this.child(
                div()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(palette.border)
                    .text_xs()
                    .text_color(palette.muted)
                    .child(format!("{} more columns not shown", column_count - 12)),
            )
        })
        .into_any_element()
}

fn relation_ref(
    schema: &str,
    table: &str,
    column: &str,
    current: bool,
    palette: &Palette,
    mono: &SharedString,
) -> AnyElement {
    h_flex()
        .max_w(px(245.0))
        .min_w(px(0.0))
        .items_center()
        .gap_1()
        .px_2()
        .py_1()
        .rounded_sm()
        .when(current, |this| {
            this.bg(palette.accent_soft).text_color(palette.accent)
        })
        .child(
            div()
                .min_w(px(0.0))
                .text_xs()
                .font_family(mono.clone())
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(format!("{schema}.{table}.{column}")),
        )
        .when(current, |this| {
            this.child(
                div()
                    .w(px(4.0))
                    .h(px(4.0))
                    .flex_shrink_0()
                    .rounded_full()
                    .bg(palette.accent),
            )
        })
        .into_any_element()
}

fn relationship_row(
    fk: &ForeignKeyInfo,
    table: &TableTab,
    palette: &Palette,
    mono: &SharedString,
    first: bool,
) -> AnyElement {
    let outgoing = fk.schema == table.schema && fk.table == table.table;
    let (from_schema, from_table, from_column) = if outgoing {
        (fk.schema.as_str(), fk.table.as_str(), fk.column.as_str())
    } else {
        (
            fk.referenced_schema.as_str(),
            fk.referenced_table.as_str(),
            fk.referenced_column.as_str(),
        )
    };
    let (to_schema, to_table, to_column) = if outgoing {
        (
            fk.referenced_schema.as_str(),
            fk.referenced_table.as_str(),
            fk.referenced_column.as_str(),
        )
    } else {
        (fk.schema.as_str(), fk.table.as_str(), fk.column.as_str())
    };

    h_flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .when(!first, |this| {
            this.border_t_1().border_color(palette.border)
        })
        .child(relation_ref(
            from_schema,
            from_table,
            from_column,
            outgoing,
            palette,
            mono,
        ))
        .child(
            Icon::new(IconName::ChevronRight)
                .with_size(Size::Small)
                .text_color(palette.muted),
        )
        .child(relation_ref(
            to_schema, to_table, to_column, !outgoing, palette, mono,
        ))
        .child(div().flex_1().min_w(px(0.0)))
        .when(!fk.constraint_name.is_empty(), |this| {
            this.child(
                div()
                    .max_w(px(180.0))
                    .flex_shrink_0()
                    .text_right()
                    .text_xs()
                    .font_family(mono.clone())
                    .text_color(palette.muted)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(fk.constraint_name.clone()),
            )
        })
        .into_any_element()
}

fn relationships_panel(table: &TableTab, palette: &Palette, mono: &SharedString) -> AnyElement {
    let mut panel = v_flex()
        .flex_1()
        .min_w(px(340.0))
        .rounded_md()
        .border_1()
        .border_color(palette.border)
        .bg(palette.background)
        .overflow_hidden()
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .gap_3()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(palette.border)
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
                ),
        );

    if table.foreign_keys_loading {
        panel = panel.child(
            div()
                .px_3()
                .py_4()
                .text_xs()
                .text_color(palette.muted)
                .child("Loading relationships..."),
        );
    } else if table.foreign_keys.is_empty() {
        panel = panel.child(
            div()
                .px_3()
                .py_4()
                .text_xs()
                .text_color(palette.muted)
                .child("No foreign keys point at or from this table."),
        );
    } else {
        for (index, fk) in table.foreign_keys.iter().enumerate() {
            panel = panel.child(relationship_row(fk, table, palette, mono, index == 0));
        }
    }

    panel.into_any_element()
}

fn indexes_panel(table: &TableTab, palette: &Palette, mono: &SharedString) -> AnyElement {
    let mut panel = v_flex()
        .flex_1()
        .min_w(px(340.0))
        .rounded_md()
        .border_1()
        .border_color(palette.border)
        .bg(palette.background)
        .overflow_hidden()
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .gap_3()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(palette.border)
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Indexes and constraints"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(palette.muted)
                        .child(format!("{}", table.indexes.len())),
                ),
        );

    if table.foreign_keys_loading {
        panel = panel.child(
            div()
                .px_3()
                .py_4()
                .text_xs()
                .text_color(palette.muted)
                .child("Loading indexes..."),
        );
    } else if table.indexes.is_empty() {
        panel = panel.child(
            div()
                .px_3()
                .py_4()
                .text_xs()
                .text_color(palette.muted)
                .child("No indexes reported for this relation."),
        );
    } else {
        for (index, item) in table.indexes.iter().enumerate() {
            panel = panel.child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .when(index > 0, |this| {
                        this.border_t_1().border_color(palette.border)
                    })
                    .child(
                        div()
                            .min_w(px(0.0))
                            .flex_1()
                            .text_xs()
                            .font_family(mono.clone())
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(item.name.clone()),
                    )
                    .child(
                        div()
                            .max_w(px(240.0))
                            .flex_shrink_0()
                            .text_right()
                            .text_xs()
                            .font_family(mono.clone())
                            .text_color(palette.muted)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(item.columns.join(", ")),
                    )
                    .when(item.primary, |this| {
                        this.child(
                            div()
                                .px_2()
                                .py_0p5()
                                .rounded_sm()
                                .bg(palette.accent_soft)
                                .text_xs()
                                .font_family(mono.clone())
                                .text_color(palette.accent)
                                .child("PK"),
                        )
                    })
                    .when(item.unique && !item.primary, |this| {
                        this.child(
                            div()
                                .px_2()
                                .py_0p5()
                                .rounded_sm()
                                .bg(palette.success.opacity(0.14))
                                .text_xs()
                                .font_family(mono.clone())
                                .text_color(palette.success)
                                .child("UNIQUE"),
                        )
                    }),
            );
        }
    }

    panel.into_any_element()
}

pub(super) fn render_diagram(
    tab_id: u64,
    table: &TableTab,
    page: Option<&TablePage>,
    palette: &Palette,
    mono: &SharedString,
) -> AnyElement {
    v_flex()
        .id(("table-diagram", tab_id))
        .flex_1()
        .min_h(px(0.0))
        .overflow_y_scroll()
        .p_5()
        .gap_4()
        .child(
            h_flex()
                .items_start()
                .gap_4()
                .flex_wrap()
                .child(table_sheet(table, page, palette, mono))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w(px(340.0))
                        .gap_4()
                        .child(relationships_panel(table, palette, mono))
                        .child(indexes_panel(table, palette, mono)),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(palette.muted)
                .child("Drag the Results divider or this pane split to resize."),
        )
        .into_any_element()
}
