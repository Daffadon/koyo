use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, IconName, Sizable, Size};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use gpui_kit::{InteractiveElement, StatefulInteractiveElement};
use koyo_core::models::{ColumnInfo, Engine, ForeignKeyInfo, TablePage};

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

fn open_related_from_menu(
    app: &Entity<KoyoApp>,
    tab_id: u64,
    schema: String,
    table: String,
    mode: OpenTableMode,
    cx: &mut App,
) {
    let window_handle = app.read(cx).window_handle;
    let _ = cx.update_window(window_handle, |_, window, cx| {
        let _ = app.update(cx, |this, cx| {
            this.open_related_table(tab_id, schema, table, mode, window, cx);
        });
    });
}

#[allow(clippy::too_many_arguments)]
fn relation_ref(
    tab_id: u64,
    role: &str,
    schema: &str,
    table: &str,
    column: &str,
    show_schema: bool,
    current: bool,
    palette: &Palette,
    mono: &SharedString,
    index: usize,
    cx: &mut Context<KoyoApp>,
) -> AnyElement {
    let full = format!("{schema}.{table}.{column}");
    let tooltip = format!("Open {schema}.{table} ({column}). Right-click for tab options.");
    let open_schema = schema.to_string();
    let open_table = table.to_string();
    let menu_schema = open_schema.clone();
    let menu_table = open_table.clone();
    let app = cx.entity();
    let label = if show_schema {
        format!("{schema}.{table}")
    } else {
        table.to_string()
    };
    h_flex()
        .id(SharedString::from(format!(
            "relationship-{tab_id}-{role}-{index}-{full}"
        )))
        .max_w(px(320.0))
        .min_w(px(0.0))
        .items_center()
        .gap_1()
        .px_2()
        .py_1()
        .rounded_sm()
        .when(current, |this| {
            this.bg(palette.accent_soft).text_color(palette.accent)
        })
        .cursor_pointer()
        .aria_label(format!("Open {label}"))
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .on_click(cx.listener(move |this, _, window, cx| {
            this.open_related_table(
                tab_id,
                open_schema.clone(),
                open_table.clone(),
                OpenTableMode::FocusOrOpen,
                window,
                cx,
            );
        }))
        .context_menu(move |menu, _, _cx| {
            let mut menu = menu;
            for (label, mode) in [
                ("Open table", OpenTableMode::FocusOrOpen),
                ("Open in new tab", OpenTableMode::NewTab),
                ("Open in other pane", OpenTableMode::OtherPane),
            ] {
                let app = app.clone();
                let schema = menu_schema.clone();
                let table = menu_table.clone();
                menu = menu.item(PopupMenuItem::new(label).on_click(move |_, _, cx| {
                    open_related_from_menu(&app, tab_id, schema.clone(), table.clone(), mode, cx);
                }));
            }
            menu
        })
        .child(
            v_flex()
                .min_w(px(0.0))
                .gap_0p5()
                .child(
                    div()
                        .min_w(px(0.0))
                        .text_xs()
                        .font_family(mono.clone())
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(label),
                )
                .child(
                    div()
                        .min_w(px(0.0))
                        .text_xs()
                        .font_family(mono.clone())
                        .text_color(if current {
                            palette.accent
                        } else {
                            palette.muted
                        })
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(column.to_string()),
                ),
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

#[allow(clippy::too_many_arguments)]
fn relationship_row(
    tab_id: u64,
    fk: &ForeignKeyInfo,
    table: &TableTab,
    palette: &Palette,
    mono: &SharedString,
    index: usize,
    first: bool,
    cx: &mut Context<KoyoApp>,
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
    let show_schema = !from_schema.eq_ignore_ascii_case(to_schema);

    h_flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .when(!first, |this| {
            this.border_t_1().border_color(palette.border)
        })
        .child(relation_ref(
            tab_id,
            "from",
            from_schema,
            from_table,
            from_column,
            show_schema,
            outgoing,
            palette,
            mono,
            index,
            cx,
        ))
        .child(
            Icon::new(IconName::ChevronRight)
                .with_size(Size::Small)
                .text_color(palette.muted),
        )
        .child(relation_ref(
            tab_id,
            "to",
            to_schema,
            to_table,
            to_column,
            show_schema,
            !outgoing,
            palette,
            mono,
            index,
            cx,
        ))
        .child(div().flex_1().min_w(px(0.0)))
        .when(!fk.constraint_name.is_empty(), |this| {
            let constraint = fk.constraint_name.clone();
            this.child(
                div()
                    .id(SharedString::from(format!(
                        "relationship-constraint-{tab_id}-{index}"
                    )))
                    .max_w(px(180.0))
                    .flex_shrink_0()
                    .text_right()
                    .text_xs()
                    .font_family(mono.clone())
                    .text_color(palette.muted)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .tooltip(move |window, cx| Tooltip::new(constraint.clone()).build(window, cx))
                    .child(fk.constraint_name.clone()),
            )
        })
        .into_any_element()
}

fn relationships_panel(
    tab_id: u64,
    table: &TableTab,
    palette: &Palette,
    mono: &SharedString,
    cx: &mut Context<KoyoApp>,
) -> AnyElement {
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
            panel = panel.child(relationship_row(
                tab_id,
                fk,
                table,
                palette,
                mono,
                index,
                index == 0,
                cx,
            ));
        }
    }

    panel.into_any_element()
}

fn indexes_panel(
    tab_id: u64,
    table: &TableTab,
    palette: &Palette,
    mono: &SharedString,
) -> AnyElement {
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
            let name = item.name.clone();
            let columns = item.columns.join(", ");
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
                        v_flex()
                            .min_w(px(0.0))
                            .flex_1()
                            .gap_0p5()
                            .child(
                                div()
                                    .id(SharedString::from(format!("index-name-{tab_id}-{index}")))
                                    .min_w(px(0.0))
                                    .text_xs()
                                    .font_family(mono.clone())
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .tooltip(move |window, cx| {
                                        Tooltip::new(name.clone()).build(window, cx)
                                    })
                                    .child(item.name.clone()),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!(
                                        "index-columns-{tab_id}-{index}"
                                    )))
                                    .min_w(px(0.0))
                                    .text_xs()
                                    .font_family(mono.clone())
                                    .text_color(palette.muted)
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .tooltip(move |window, cx| {
                                        Tooltip::new(columns.clone()).build(window, cx)
                                    })
                                    .child(item.columns.join(", ")),
                            ),
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

fn ddl_panel(
    tab_id: u64,
    engine: Engine,
    table: &TableTab,
    page: Option<&TablePage>,
    palette: &Palette,
    mono: &SharedString,
    cx: &mut Context<KoyoApp>,
) -> AnyElement {
    let columns = page.map(|page| page.columns.as_slice()).unwrap_or(&[]);
    let ddl = table_ddl(
        engine,
        &table.schema,
        &table.table,
        columns,
        &table.foreign_keys,
    );
    let ddl_for_copy = ddl.clone();
    v_flex()
        .w_full()
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
                    v_flex()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Create table"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette.muted)
                                .child("DDL from current metadata"),
                        ),
                )
                .child(
                    Button::new(("copy-table-ddl", tab_id))
                        .ghost()
                        .compact()
                        .icon(IconName::Copy)
                        .accessibility_label("Copy CREATE TABLE DDL")
                        .tooltip("Copy CREATE TABLE DDL")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.copy_to_clipboard(
                                ddl_for_copy.clone(),
                                "Copied CREATE TABLE DDL.",
                                cx,
                            );
                        })),
                ),
        )
        .child(
            v_flex()
                .id(("table-ddl", tab_id))
                .w_full()
                .max_h(px(300.0))
                .overflow_y_scroll()
                .p_3()
                .gap_0p5()
                .children(ddl.lines().map(|line| {
                    div()
                        .whitespace_nowrap()
                        .text_xs()
                        .font_family(mono.clone())
                        .text_color(palette.foreground)
                        .child(line.to_string())
                })),
        )
        .into_any_element()
}

pub(super) fn render_diagram(
    tab_id: u64,
    table: &TableTab,
    page: Option<&TablePage>,
    engine: Engine,
    palette: &Palette,
    mono: &SharedString,
    cx: &mut Context<KoyoApp>,
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
                        .child(relationships_panel(tab_id, table, palette, mono, cx))
                        .child(indexes_panel(tab_id, table, palette, mono)),
                ),
        )
        .child(ddl_panel(tab_id, engine, table, page, palette, mono, cx))
        .child(
            div()
                .text_xs()
                .text_color(palette.muted)
                .child("Drag the Results divider or this pane split to resize."),
        )
        .into_any_element()
}
