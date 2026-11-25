use std::collections::BTreeMap;

use gpui_kit::InteractiveElement;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Disableable, Icon, IconName, Size};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::widgets::{control_focus, is_activation_key};
use crate::app::{KoyoApp, Palette, QueryHistoryEntry, TabBody, engine_label, unix_seconds};

const MAX_EXPLORER_ROWS: usize = 500;

mod history;
use history::*;

fn open_table_from_menu(app: &Entity<KoyoApp>, schema: String, table: String, cx: &mut App) {
    let window_handle = app.read(cx).window_handle;
    let _ = cx.update_window(window_handle, |_, window, cx| {
        let _ = app.update(cx, |this, cx| {
            this.open_table(schema, table, window, cx);
        });
    });
}

fn query_table_from_menu(
    app: &Entity<KoyoApp>,
    engine: koyo_core::models::Engine,
    schema: String,
    table: String,
    cx: &mut App,
) {
    let window_handle = app.read(cx).window_handle;
    let _ = cx.update_window(window_handle, |_, window, cx| {
        let _ = app.update(cx, |this, cx| {
            this.query_table(engine, &schema, &table, window, cx);
        });
    });
}

fn metadata_group_icon(name: &str) -> IconName {
    match name {
        "Event Triggers" => IconName::Play,
        "Extensions" => IconName::Frame,
        "Storage" | "Storage Engines" => IconName::HardDrive,
        "System Info" => IconName::Info,
        "Roles" | "Users" | "Logins" => IconName::CircleUser,
        _ => IconName::Folder,
    }
}

fn schema_kind_label(kind: koyo_core::models::SchemaObjectKind) -> &'static str {
    use koyo_core::models::SchemaObjectKind::*;
    match kind {
        Table => "Tables",
        View => "Views",
        Function => "Functions",
        Trigger => "Triggers",
    }
}

fn schema_kind_icon(kind: koyo_core::models::SchemaObjectKind) -> IconName {
    use koyo_core::models::SchemaObjectKind::*;
    match kind {
        Table => IconName::LayoutDashboard,
        View => IconName::Eye,
        Function => IconName::FileText,
        Trigger => IconName::Play,
    }
}

impl KoyoApp {
    pub(super) fn render_sidebar(
        &self,
        palette: &Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let Some(tab) = self.sidebar_index().and_then(|index| self.tabs.get(index)) else {
            return div();
        };
        let tab_id = tab.id;
        let engine = tab.profile.as_ref().map(|profile| profile.engine);
        let profile = tab.profile.clone();
        let connection = tab.connection.clone();
        let tables = tab.tables.clone();
        let objects = tab.schema_objects.clone();
        let expanded = tab.expanded.clone();
        let selected_table = match &tab.body {
            TabBody::Table(table) => Some((table.schema.clone(), table.table.clone())),
            TabBody::Query(_) => None,
        };
        let live = connection.is_some();
        let has_explorer =
            live || !tables.is_empty() || !objects.is_empty() || !tab.metadata_groups.is_empty();
        let search = self.explorer_filter.read(cx).value().trim().to_lowercase();
        let searching = !search.is_empty();
        let history = &self.history;
        let history_search = self.history_filter.read(cx).value().trim().to_lowercase();
        let history_indices = history
            .iter()
            .enumerate()
            .filter(|(_, entry)| history_matches(entry, &history_search))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let reconnect_profile = if !live {
            profile.as_ref().map(|profile| profile.id)
        } else {
            None
        };
        let can_switch = match &tab.body {
            TabBody::Query(query) => !query.running,
            TabBody::Table(table) => table.staged.is_empty(),
        };
        let switch_tooltip = match (live, &profile) {
            (true, _) => "Switch connection",
            (false, Some(_)) => "Reconnect",
            (false, None) => "Connect",
        };

        let mut by_schema: BTreeMap<String, Vec<koyo_core::models::SchemaObject>> = BTreeMap::new();
        for object in &objects {
            if searching
                && !object.name.to_lowercase().contains(&search)
                && !object.schema.to_lowercase().contains(&search)
            {
                continue;
            }
            by_schema
                .entry(object.schema.clone())
                .or_default()
                .push(object.clone());
        }

        let mut schema_nodes = v_flex().gap_1();
        let mut object_budget = MAX_EXPLORER_ROWS;
        let mut hidden_objects = 0usize;
        for (schema, schema_objects) in &by_schema {
            let is_expanded = searching || expanded.contains(schema);
            let schema_for_toggle = schema.clone();
            let schema_for_keyboard = schema.clone();
            let toggle_id = SharedString::from(format!("schema-{schema}"));
            schema_nodes = schema_nodes.child(
                div()
                    .id(toggle_id)
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .aria_label(format!("Schema {schema}"))
                    .cursor_pointer()
                    .hover(|style| style.bg(palette.panel))
                    .track_focus(&control_focus(tab_id, format!("schema-node-{schema}"), cx))
                    .tab_stop(true)
                    .focus(|style| style.bg(palette.accent_soft).border_color(palette.ring))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                        if is_activation_key(event) {
                            this.set_expanded(
                                tab_id,
                                schema_for_keyboard.clone(),
                                !is_expanded,
                                cx,
                            );
                        }
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_expanded(tab_id, schema_for_toggle.clone(), !is_expanded, cx);
                    }))
                    .child(
                        Icon::new(if is_expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .with_size(Size::Small),
                    )
                    .child(Icon::new(IconName::HardDrive).with_size(Size::Small))
                    .child(div().flex_1().text_sm().child(schema.clone()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.muted)
                            .child(format!("{}", schema_objects.len())),
                    ),
            );

            if !is_expanded {
                continue;
            }

            for kind in [
                koyo_core::models::SchemaObjectKind::Table,
                koyo_core::models::SchemaObjectKind::View,
                koyo_core::models::SchemaObjectKind::Function,
                koyo_core::models::SchemaObjectKind::Trigger,
            ] {
                let items = schema_objects
                    .iter()
                    .filter(|object| object.kind == kind)
                    .collect::<Vec<_>>();
                if items.is_empty() {
                    continue;
                }
                let category_key = format!("{schema}/{kind:?}");
                let category_expanded = searching || expanded.contains(&category_key);
                let category_for_toggle = category_key.clone();
                let category_for_keyboard = category_key.clone();
                schema_nodes = schema_nodes.child(
                    div()
                        .id(SharedString::from(format!("schema-kind-{category_key}")))
                        .flex()
                        .items_center()
                        .gap_2()
                        .ml_3()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .cursor_pointer()
                        .hover(|style| style.bg(palette.panel))
                        .track_focus(&control_focus(
                            tab_id,
                            format!("schema-kind-{category_key}"),
                            cx,
                        ))
                        .tab_stop(true)
                        .focus(|style| style.bg(palette.accent_soft).border_color(palette.ring))
                        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                            if is_activation_key(event) {
                                this.set_expanded(
                                    tab_id,
                                    category_for_keyboard.clone(),
                                    !category_expanded,
                                    cx,
                                );
                            }
                        }))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_expanded(
                                tab_id,
                                category_for_toggle.clone(),
                                !category_expanded,
                                cx,
                            );
                        }))
                        .child(
                            Icon::new(if category_expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .with_size(Size::Small),
                        )
                        .child(
                            Icon::new(if category_expanded {
                                IconName::FolderOpen
                            } else {
                                IconName::Folder
                            })
                            .with_size(Size::Small),
                        )
                        .child(div().flex_1().text_xs().child(schema_kind_label(kind)))
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette.muted)
                                .child(format!("{}", items.len())),
                        ),
                );
                if !category_expanded {
                    continue;
                }
                for object in items {
                    if object_budget == 0 {
                        hidden_objects += 1;
                        continue;
                    }
                    object_budget -= 1;
                    let schema_name = object.schema.clone();
                    let name = object.name.clone();
                    let kind = object.kind;
                    let item_id = SharedString::from(format!("{schema}.{kind:?}.{name}"));
                    let selected =
                        selected_table
                            .as_ref()
                            .is_some_and(|(selected_schema, selected_name)| {
                                selected_schema == &schema_name && selected_name == &name
                            });
                    let editable = tables.iter().any(|table| {
                        table.schema == schema_name && table.name == name && table.editable
                    });
                    let opens_table = matches!(
                        kind,
                        koyo_core::models::SchemaObjectKind::Table
                            | koyo_core::models::SchemaObjectKind::View
                    );
                    let clickable = opens_table
                        || matches!(
                            kind,
                            koyo_core::models::SchemaObjectKind::Function
                                | koyo_core::models::SchemaObjectKind::Trigger
                        );
                    let context_schema = schema_name.clone();
                    let context_table = name.clone();
                    let node = div()
                        .id(item_id)
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .py_1()
                        .ml_7()
                        .rounded_md()
                        .when(clickable, |this| this.cursor_pointer())
                        .when(selected, |this| this.bg(palette.selection))
                        .when(!selected && clickable, |this| {
                            this.hover(|style| style.bg(palette.panel))
                        })
                        .aria_label(format!("{kind:?} {name}"))
                        .when(clickable, |this| {
                            let name = name.clone();
                            let schema_name_for_keyboard = schema_name.clone();
                            let name_for_keyboard = name.clone();
                            this.track_focus(&control_focus(
                                tab_id,
                                format!("schema-object-{schema_name}-{kind:?}-{name}"),
                                cx,
                            ))
                            .tab_stop(true)
                            .focus(|style| style.bg(palette.accent_soft).border_color(palette.ring))
                            .on_key_down(cx.listener(
                                move |this, event: &KeyDownEvent, window, cx| {
                                    if !is_activation_key(event) {
                                        return;
                                    }
                                    if opens_table {
                                        this.open_table(
                                            schema_name_for_keyboard.clone(),
                                            name_for_keyboard.clone(),
                                            window,
                                            cx,
                                        );
                                    } else {
                                        this.open_schema_definition(
                                            schema_name_for_keyboard.clone(),
                                            name_for_keyboard.clone(),
                                            kind,
                                            window,
                                            cx,
                                        );
                                    }
                                },
                            ))
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    if opens_table {
                                        this.open_table(
                                            schema_name.clone(),
                                            name.clone(),
                                            window,
                                            cx,
                                        );
                                    } else {
                                        this.open_schema_definition(
                                            schema_name.clone(),
                                            name.clone(),
                                            kind,
                                            window,
                                            cx,
                                        );
                                    }
                                },
                            ))
                        })
                        .child(Icon::new(schema_kind_icon(kind)).with_size(Size::Small))
                        .child(div().flex_1().text_sm().child(name.clone()))
                        .when(
                            kind == koyo_core::models::SchemaObjectKind::Table && !editable,
                            |this| {
                                this.child(
                                    div().w(px(6.0)).h(px(6.0)).rounded_full().bg(palette.muted),
                                )
                            },
                        );
                    let node = if opens_table {
                        let app = cx.entity();
                        let menu_schema = context_schema;
                        let menu_table = context_table;
                        node.context_menu(move |menu, _, _cx| {
                            let mut menu = menu;
                            if let Some(engine) = engine {
                                let open_app = app.clone();
                                let open_schema = menu_schema.clone();
                                let open_table = menu_table.clone();
                                menu = menu.item(PopupMenuItem::new("Open data").on_click(
                                    move |_, _, cx| {
                                        open_table_from_menu(
                                            &open_app,
                                            open_schema.clone(),
                                            open_table.clone(),
                                            cx,
                                        );
                                    },
                                ));
                                let query_app = app.clone();
                                let query_schema = menu_schema.clone();
                                let query_table = menu_table.clone();
                                menu = menu.item(PopupMenuItem::new("Query table").on_click(
                                    move |_, _, cx| {
                                        query_table_from_menu(
                                            &query_app,
                                            engine,
                                            query_schema.clone(),
                                            query_table.clone(),
                                            cx,
                                        );
                                    },
                                ));
                            }
                            let qualified = format!("{menu_schema}.{menu_table}");
                            menu.separator().item(
                                PopupMenuItem::new("Copy qualified name").on_click(
                                    move |_, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            qualified.clone(),
                                        ));
                                    },
                                ),
                            )
                        })
                        .into_any_element()
                    } else {
                        node.into_any_element()
                    };
                    schema_nodes = schema_nodes.child(node);
                }
            }
        }

        let schemas_expanded = searching || expanded.contains("Schemas");
        let mut tree = v_flex()
            .id("schema-tree")
            .flex_1()
            .overflow_y_scroll()
            .p_2()
            .gap_1();

        if !searching || !by_schema.is_empty() {
            let schemas_key = "Schemas".to_string();
            let schemas_key_for_keyboard = schemas_key.clone();
            tree = tree.child(
                div()
                    .id("schema-group-schemas")
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .aria_label("Schemas")
                    .cursor_pointer()
                    .hover(|style| style.bg(palette.panel))
                    .track_focus(&control_focus(tab_id, "schema-group", cx))
                    .tab_stop(true)
                    .focus(|style| style.bg(palette.accent_soft).border_color(palette.ring))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                        if is_activation_key(event) {
                            this.set_expanded(
                                tab_id,
                                schemas_key_for_keyboard.clone(),
                                !schemas_expanded,
                                cx,
                            );
                        }
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_expanded(tab_id, schemas_key.clone(), !schemas_expanded, cx);
                    }))
                    .child(
                        Icon::new(if schemas_expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .with_size(Size::Small),
                    )
                    .child(Icon::new(IconName::FolderOpen).with_size(Size::Small))
                    .child(div().flex_1().text_sm().child("Schemas"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.muted)
                            .child(format!("{}", by_schema.len())),
                    ),
            );
            if schemas_expanded {
                tree = tree.child(div().ml_3().child(schema_nodes));
            }
        }

        let mut visible_groups = 0;
        for group in &tab.metadata_groups {
            let group_name_matches = group.name.to_lowercase().contains(&search);
            let group_items = group
                .items
                .iter()
                .filter(|item| {
                    !searching
                        || group_name_matches
                        || item.name.to_lowercase().contains(&search)
                        || item.detail.to_lowercase().contains(&search)
                })
                .collect::<Vec<_>>();
            if searching && group_items.is_empty() {
                continue;
            }
            visible_groups += 1;
            let key = format!("group:{}", group.name);
            let is_expanded = searching || expanded.contains(&key);
            let key_for_toggle = key.clone();
            let key_for_keyboard = key.clone();
            tree = tree.child(
                div()
                    .id(SharedString::from(format!("metadata-group-{key}")))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .aria_label(group.name.clone())
                    .cursor_pointer()
                    .hover(|style| style.bg(palette.panel))
                    .track_focus(&control_focus(tab_id, format!("metadata-group-{key}"), cx))
                    .tab_stop(true)
                    .focus(|style| style.bg(palette.accent_soft).border_color(palette.ring))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                        if is_activation_key(event) {
                            this.set_expanded(tab_id, key_for_keyboard.clone(), !is_expanded, cx);
                        }
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_expanded(tab_id, key_for_toggle.clone(), !is_expanded, cx);
                    }))
                    .child(
                        Icon::new(if is_expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .with_size(Size::Small),
                    )
                    .child(Icon::new(metadata_group_icon(&group.name)).with_size(Size::Small))
                    .child(div().flex_1().text_sm().child(group.name.clone()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.muted)
                            .child(format!("{}", group_items.len())),
                    ),
            );
            if is_expanded {
                for item in group_items {
                    if object_budget == 0 {
                        hidden_objects += 1;
                        continue;
                    }
                    object_budget -= 1;
                    tree = tree.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .ml_7()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .child(
                                Icon::new(IconName::FileText)
                                    .with_size(Size::Small)
                                    .text_color(palette.muted),
                            )
                            .child(div().text_sm().flex_shrink_0().child(item.name.clone()))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .text_xs()
                                    .text_color(palette.muted)
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_right()
                                    .child(item.detail.clone()),
                            ),
                    );
                }
            }
        }

        if hidden_objects > 0 {
            tree = tree.child(
                div()
                    .px_2()
                    .py_3()
                    .text_xs()
                    .text_color(palette.warning)
                    .child(format!(
                        "Showing the first {MAX_EXPLORER_ROWS} objects; {hidden_objects} more hidden. Use search to narrow the list."
                    )),
            );
        }

        if searching && by_schema.is_empty() && visible_groups == 0 {
            tree = tree.child(
                div()
                    .px_2()
                    .py_4()
                    .text_xs()
                    .text_color(palette.muted)
                    .child(format!("No tables match \"{search}\".")),
            );
        }

        if tab.catalog_loading && objects.is_empty() && tab.metadata_groups.is_empty() {
            tree = tree.child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .p_3()
                    .text_xs()
                    .text_color(palette.muted)
                    .child(Spinner::new().with_size(Size::Small).color(palette.muted))
                    .child("Loading database objects..."),
            );
        } else if objects.is_empty() && tab.metadata_groups.is_empty() {
            tree = tree.child(
                div()
                    .p_3()
                    .text_xs()
                    .text_color(palette.muted)
                    .child(if live {
                        "No metadata found."
                    } else {
                        "Connect to browse schemas and table data."
                    }),
            );
        }

        let history_panel = self.render_history_panel(
            tab_id,
            history,
            &history_search,
            &history_indices,
            palette,
            cx,
        );

        let search_field = Input::new(&self.explorer_filter)
            .aria_label("Search tables")
            .w_full()
            .cleanable(true)
            .with_size(Size::Small)
            .prefix(
                Icon::new(IconName::Search)
                    .with_size(Size::Small)
                    .text_color(palette.muted),
            );

        v_flex()
            .w(px(self.sidebar_width))
            .h_full()
            .flex_shrink_0()
            .bg(palette.sidebar_gradient)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(palette.sidebar_border)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(super::widgets::logo_mark(&palette, 28.0))
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Koyo"),
                            ),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1()
                            .child(super::widgets::button_tooltip(
                                "top-tooltip-manage",
                                Button::new("manage-connections")
                                    .ghost()
                                    .compact()
                                    .icon(IconName::Plus)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_connection_dialog(None, window, cx);
                                    })),
                                "Manage connections",
                            ))
                            .child(super::widgets::button_tooltip(
                                "top-tooltip-settings",
                                Button::new("settings")
                                    .ghost()
                                    .compact()
                                    .icon(IconName::Settings2)
                                    .accessibility_label("Open settings")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_settings(window, cx);
                                    })),
                                "Settings",
                            ))
                            .child(super::widgets::button_tooltip(
                                "top-tooltip-collapse-sidebar",
                                Button::new("collapse-sidebar")
                                    .ghost()
                                    .compact()
                                    .icon(IconName::PanelLeftClose)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_sidebar(cx);
                                    })),
                                "Hide sidebar",
                            )),
                    ),
            )
            .child(
                v_flex()
                    .gap_2()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(palette.sidebar_border)
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .when_some(profile.as_ref(), |this, profile| {
                                this.child(
                                    super::connection::engine_icon(profile.engine)
                                        .with_size(Size::Small)
                                        .text_color(palette.muted),
                                )
                            })
                            .when(profile.is_none(), |this| {
                                this.child(
                                    Icon::new(IconName::HardDrive)
                                        .with_size(Size::Small)
                                        .text_color(palette.muted),
                                )
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(
                                        profile
                                            .as_ref()
                                            .map(|profile| profile.name.clone())
                                            .unwrap_or_else(|| "Not connected".into()),
                                    ),
                            )
                            .child(div().text_xs().text_color(palette.muted).child(
                                match &profile {
                                    Some(profile) => engine_label(profile.engine),
                                    None => "No session",
                                },
                            ))
                            .child(div().w(px(6.0)).h(px(6.0)).rounded_full().bg(if live {
                                palette.success
                            } else {
                                palette.muted
                            })),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.muted)
                            .whitespace_normal()
                            .child(match &profile {
                                Some(profile) if profile.engine.is_sqlite() => {
                                    profile.sqlite_path.clone()
                                }
                                Some(profile) => format!(
                                    "{}@{}:{}",
                                    profile.username, profile.host, profile.port
                                ),
                                None => "Choose a saved profile".into(),
                            }),
                    )
                    .when_some(profile.as_ref(), |this, profile| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(palette.muted)
                                .whitespace_normal()
                                .child(if profile.engine.is_sqlite() {
                                    "SQLite file".to_string()
                                } else {
                                    format!("Database: {}", profile.database)
                                }),
                        )
                    })
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("sidebar-switch-connection")
                                    .ghost()
                                    .compact()
                                    .icon(IconName::Globe)
                                    .label(switch_tooltip)
                                    .disabled(!can_switch)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        match reconnect_profile {
                                            Some(profile_id) => this.connect_profile(
                                                profile_id,
                                                Some(tab_id),
                                                false,
                                                window,
                                                cx,
                                            ),
                                            None => this.open_connection_dialog(
                                                Some(tab_id),
                                                window,
                                                cx,
                                            ),
                                        }
                                    })),
                            )
                            .when(live, |this| {
                                this.child(
                                    Button::new("disconnect")
                                        .ghost()
                                        .compact()
                                        .icon(IconName::CircleX)
                                        .label("Disconnect")
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.request_disconnect(tab_id, window, cx);
                                        })),
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_4()
                    .pt_3()
                    .pb_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.muted)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("EXPLORER"),
                    )
                    .when(has_explorer, |this| {
                        this.child(
                            h_flex()
                                .items_center()
                                .gap_1()
                                .child(super::widgets::button_tooltip(
                                    ("tooltip-expand-all", tab_id),
                                    Button::new("expand-all-tables")
                                        .ghost()
                                        .compact()
                                        .icon(IconName::ChevronsUpDown)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.set_all_expanded(tab_id, true, cx);
                                        })),
                                    "Expand all",
                                ))
                                .child(super::widgets::button_tooltip(
                                    ("tooltip-collapse-all", tab_id),
                                    Button::new("collapse-all-tables")
                                        .ghost()
                                        .compact()
                                        .icon(IconName::Minus)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.set_all_expanded(tab_id, false, cx);
                                        })),
                                    "Collapse all",
                                ))
                                .when(live, |this| {
                                    this.child(super::widgets::button_tooltip(
                                        ("tooltip-refresh-tables", tab_id),
                                        Button::new("refresh-tables")
                                            .ghost()
                                            .compact()
                                            .icon(IconName::RefreshCw)
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.refresh_tables(tab_id, cx);
                                            })),
                                        "Refresh tables",
                                    ))
                                }),
                        )
                    }),
            )
            .when(has_explorer, |this| {
                this.child(
                    v_flex()
                        .gap_1()
                        .px_3()
                        .pb_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette.muted)
                                .child("Search tables"),
                        )
                        .child(search_field),
                )
            })
            .when(has_explorer, |this| this.child(tree))
            .when(!has_explorer, |this| {
                this.child(
                    v_flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .gap_3()
                        .p_4()
                        .child(
                            gpui_kit::component::Icon::new(IconName::HardDrive)
                                .with_size(Size::Large)
                                .text_color(palette.muted),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette.muted)
                                .text_center()
                                .child(if profile.is_some() {
                                    "Session disconnected. Reconnect to browse."
                                } else {
                                    "Connect to browse schemas and table data."
                                }),
                        )
                        .child(
                            Button::new("reconnect")
                                .primary()
                                .compact()
                                .label(if profile.is_some() {
                                    "Reconnect"
                                } else {
                                    "Connect"
                                })
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    match reconnect_profile {
                                        Some(profile_id) => this.connect_profile(
                                            profile_id,
                                            Some(tab_id),
                                            false,
                                            window,
                                            cx,
                                        ),
                                        None => {
                                            this.open_connection_dialog(Some(tab_id), window, cx)
                                        }
                                    }
                                })),
                        ),
                )
            })
            .when_some(history_panel, |this, panel| this.child(panel))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .py_2()
                    .border_t_1()
                    .border_color(palette.sidebar_border)
                    .text_xs()
                    .text_color(palette.muted)
                    .child(div().w(px(6.0)).h(px(6.0)).rounded_full().bg(if live {
                        palette.success
                    } else {
                        palette.muted
                    }))
                    .child(match &connection {
                        Some(info) => format!(
                            "{} · {} {}",
                            info.current_user,
                            engine_label(info.profile.engine),
                            info.server_version.split(' ').next().unwrap_or_default()
                        ),
                        None => "Local profiles only".to_string(),
                    }),
            )
    }
}
