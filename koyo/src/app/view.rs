use std::collections::BTreeMap;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::tab::{Tab as BarTab, TabBar};
use gpui_kit::component::{Disableable, Icon, IconName, Size};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use gpui_kit::{CursorStyle, DragMoveEvent, InteractiveElement, StatefulInteractiveElement};

use super::{Palette, Pane, SplitAxis, TabBody, engine_label, query_view, table_view};

#[derive(Clone, Copy)]
struct PaneSplitDrag;

impl Render for PaneSplitDrag {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

#[derive(Clone)]
struct TabDrag {
    tab_id: u64,
    title: SharedString,
}

impl Render for TabDrag {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .text_sm()
            .child(self.title.clone())
    }
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

impl super::KoyoApp {
    pub(super) fn render_sidebar(
        &self,
        palette: &Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let Some(tab) = self.focused_index().and_then(|index| self.tabs.get(index)) else {
            return div();
        };
        let tab_id = tab.id;
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
        let reconnect_profile = if matches!(tab.body, TabBody::Table(_)) && !live {
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
            by_schema
                .entry(object.schema.clone())
                .or_default()
                .push(object.clone());
        }

        let mut schema_nodes = v_flex().gap_1();
        for (schema, schema_objects) in &by_schema {
            let is_expanded = expanded.contains(schema);
            let schema_for_toggle = schema.clone();
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
                    .cursor_pointer()
                    .hover(|style| style.bg(palette.panel))
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
                let category_expanded = expanded.contains(&category_key);
                let category_for_toggle = category_key.clone();
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
                    let clickable = matches!(
                        kind,
                        koyo_core::models::SchemaObjectKind::Table
                            | koyo_core::models::SchemaObjectKind::View
                    );
                    schema_nodes = schema_nodes.child(
                        div()
                            .id(item_id)
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .py_1()
                            .ml_7()
                            .rounded_md()
                            .when(clickable, |this| this.cursor_pointer())
                            .when(selected, |this| this.bg(palette.accent_soft))
                            .when(!selected && clickable, |this| {
                                this.hover(|style| style.bg(palette.panel))
                            })
                            .when(clickable, |this| {
                                let name = name.clone();
                                this.on_click(cx.listener(move |this, _, window, cx| {
                                    this.open_table(schema_name.clone(), name.clone(), window, cx);
                                }))
                            })
                            .child(Icon::new(schema_kind_icon(kind)).with_size(Size::Small))
                            .child(div().flex_1().text_sm().child(name.clone()))
                            .when(
                                kind == koyo_core::models::SchemaObjectKind::Table && !editable,
                                |this| {
                                    this.child(
                                        div()
                                            .w(px(6.0))
                                            .h(px(6.0))
                                            .rounded_full()
                                            .bg(palette.muted),
                                    )
                                },
                            ),
                    );
                }
            }
        }

        let schemas_expanded = expanded.contains("Schemas");
        let mut tree = v_flex()
            .id("schema-tree")
            .flex_1()
            .overflow_y_scroll()
            .p_2()
            .gap_1();

        let schemas_key = "Schemas".to_string();
        tree = tree.child(
            div()
                .id("schema-group-schemas")
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .rounded_md()
                .cursor_pointer()
                .hover(|style| style.bg(palette.panel))
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

        for group in &tab.metadata_groups {
            let key = format!("group:{}", group.name);
            let is_expanded = expanded.contains(&key);
            let key_for_toggle = key.clone();
            tree = tree.child(
                div()
                    .id(SharedString::from(format!("metadata-group-{key}")))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|style| style.bg(palette.panel))
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
                            .child(format!("{}", group.items.len())),
                    ),
            );
            if is_expanded {
                for item in &group.items {
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

        if objects.is_empty() && tab.metadata_groups.is_empty() {
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

        v_flex()
            .w(px(300.0))
            .h_full()
            .flex_shrink_0()
            .bg(palette.sidebar)
            .border_r_1()
            .border_color(palette.sidebar_border)
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
                            .child(
                                div()
                                    .size(px(28.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_md()
                                    .bg(palette.accent)
                                    .text_color(palette.background)
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("K"),
                            )
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
                            .child(
                                Button::new("manage-connections")
                                    .ghost()
                                    .compact()
                                    .icon(IconName::Plus)
                                    .tooltip("Manage connections")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_connection_dialog(None, window, cx);
                                    })),
                            )
                            .child(
                                Button::new("theme-toggle")
                                    .ghost()
                                    .compact()
                                    .icon(if cx.theme().is_dark() {
                                        IconName::Sun
                                    } else {
                                        IconName::Moon
                                    })
                                    .tooltip(if cx.theme().is_dark() {
                                        "Light theme"
                                    } else {
                                        "Dark theme"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_theme(cx);
                                    })),
                            )
                            .child(
                                Button::new("collapse-sidebar")
                                    .ghost()
                                    .compact()
                                    .icon(IconName::PanelLeftClose)
                                    .tooltip("Hide sidebar")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_sidebar(cx);
                                    })),
                            ),
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
                                    super::connection_view::engine_icon(profile.engine)
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
                    .when(live, |this| {
                        this.child(
                            Button::new("refresh-tables")
                                .ghost()
                                .compact()
                                .icon(IconName::RefreshCw)
                                .tooltip("Refresh tables")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.refresh_tables(tab_id, cx);
                                })),
                        )
                    }),
            )
            .when(live, |this| this.child(tree))
            .when(!live, |this| {
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

    pub(super) fn render_workspace(
        &mut self,
        _window: &mut Window,
        palette: &Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        self.normalize_panes();
        let primary = self.pane_index(Pane::Primary).unwrap_or(0);
        let focused = self.focused_index().unwrap_or(primary);
        let first = self.render_pane(Pane::Primary, primary, true, palette, cx);
        let content = match self.split {
            Some(SplitAxis::Vertical) => {
                let secondary = self.pane_index(Pane::Secondary).unwrap_or(primary);
                let second = self.render_pane(Pane::Secondary, secondary, false, palette, cx);
                let ratio = self.split_ratio;
                h_flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .on_drag_move::<PaneSplitDrag>(cx.listener(
                        |this, event: &DragMoveEvent<PaneSplitDrag>, _, cx| {
                            let width = f32::from(event.bounds.size.width).max(1.0);
                            let x = f32::from(event.event.position.x - event.bounds.origin.x);
                            this.set_split_ratio(x / width, cx);
                        },
                    ))
                    .child(
                        div()
                            .flex_grow(ratio)
                            .flex_basis(px(0.0))
                            .w_full()
                            .min_w(px(0.0))
                            .h_full()
                            .child(first),
                    )
                    .child(
                        div()
                            .id("pane-divider-vertical")
                            .w(px(6.0))
                            .h_full()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor(CursorStyle::ResizeLeftRight)
                            .on_drag(PaneSplitDrag, |_, _, _, cx| {
                                cx.stop_propagation();
                                cx.new(|_| PaneSplitDrag)
                            })
                            .child(div().w(px(1.0)).h_full().bg(palette.border)),
                    )
                    .child(
                        div()
                            .flex_grow(1.0 - ratio)
                            .flex_basis(px(0.0))
                            .w_full()
                            .min_w(px(0.0))
                            .h_full()
                            .child(second),
                    )
                    .into_any_element()
            }
            Some(SplitAxis::Horizontal) => {
                let secondary = self.pane_index(Pane::Secondary).unwrap_or(primary);
                let second = self.render_pane(Pane::Secondary, secondary, false, palette, cx);
                let ratio = self.split_ratio;
                v_flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .on_drag_move::<PaneSplitDrag>(cx.listener(
                        |this, event: &DragMoveEvent<PaneSplitDrag>, _, cx| {
                            let height = f32::from(event.bounds.size.height).max(1.0);
                            let y = f32::from(event.event.position.y - event.bounds.origin.y);
                            this.set_split_ratio(y / height, cx);
                        },
                    ))
                    .child(
                        div()
                            .flex_grow(ratio)
                            .flex_basis(px(0.0))
                            .min_h(px(0.0))
                            .w_full()
                            .h_full()
                            .child(first),
                    )
                    .child(
                        div()
                            .id("pane-divider-horizontal")
                            .h(px(6.0))
                            .w_full()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor(CursorStyle::ResizeUpDown)
                            .on_drag(PaneSplitDrag, |_, _, _, cx| {
                                cx.stop_propagation();
                                cx.new(|_| PaneSplitDrag)
                            })
                            .child(div().h(px(1.0)).w_full().bg(palette.border)),
                    )
                    .child(
                        div()
                            .flex_grow(1.0 - ratio)
                            .flex_basis(px(0.0))
                            .min_h(px(0.0))
                            .w_full()
                            .h_full()
                            .child(second),
                    )
                    .into_any_element()
            }
            None => first,
        };

        let Some(tab) = self.tabs.get(focused) else {
            return v_flex().flex_1().min_w(px(0.0)).h_full().child(content);
        };
        let profile = tab.profile.clone();
        let connection = tab.connection.clone();
        let live = connection.is_some();
        let engine = connection
            .as_ref()
            .map(|info| info.profile.engine)
            .or(profile.as_ref().map(|profile| profile.engine));
        let rows = match &tab.body {
            TabBody::Table(table) => table.page.as_ref().map(|page| page.rows.len()).unwrap_or(0),
            TabBody::Query(query) => query
                .output
                .as_ref()
                .map(|output| {
                    let returned = output
                        .result_sets
                        .iter()
                        .map(|result| result.rows.len())
                        .sum::<usize>();
                    if returned > 0 {
                        returned
                    } else {
                        output.affected_rows.iter().copied().sum::<u64>() as usize
                    }
                })
                .unwrap_or(0),
        };
        let columns = match &tab.body {
            TabBody::Table(table) => table
                .page
                .as_ref()
                .map(|page| page.columns.len())
                .unwrap_or(0),
            TabBody::Query(_) => 0,
        };

        v_flex()
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .child(content)
            .child(
                StatusBar::new()
                    .left(
                        h_flex()
                            .items_center()
                            .gap_1()
                            .child(div().w(px(6.0)).h(px(6.0)).rounded_full().bg(if live {
                                palette.success
                            } else {
                                palette.muted
                            }))
                            .child(match (&live, &connection, &profile) {
                                (true, Some(info), _) => format!(
                                    "Connected to {}",
                                    if info.profile.engine.is_sqlite() {
                                        info.profile.sqlite_path.clone()
                                    } else {
                                        format!("{}:{}", info.profile.host, info.profile.port)
                                    }
                                ),
                                (false, _, Some(profile)) => {
                                    format!("Disconnected · {}", profile.name)
                                }
                                _ => "Disconnected".to_string(),
                            }),
                    )
                    .left(if columns > 0 {
                        format!("{columns} columns")
                    } else {
                        engine.map(engine_label).unwrap_or("No session").to_string()
                    })
                    .right(format!("{rows} {}", if rows == 1 { "row" } else { "rows" }))
                    .right("UTF-8"),
            )
    }

    fn render_pane(
        &mut self,
        pane: Pane,
        index: usize,
        show_sidebar_toggle: bool,
        palette: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(tab) = self.tabs.get(index) else {
            return div().into_any_element();
        };
        let tab_id = tab.id;
        let notice = tab.notice.clone();
        let body = match tab.body {
            TabBody::Query(_) => query_view::render(self, index, palette, cx),
            TabBody::Table(_) => table_view::render(self, index, palette, cx),
        };
        let tabs = self.tab_elements(pane, cx);
        let selected = self.pane_selected(pane);
        let focused = self.focus == pane;
        let sidebar_open = self.sidebar_open;
        let split = self.split;
        let tab_bar_id: SharedString = match pane {
            Pane::Primary => "primary-tabs".into(),
            Pane::Secondary => "secondary-tabs".into(),
        };

        v_flex()
            .flex_1()
            .w_full()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .when(split != Some(SplitAxis::Horizontal), |this| this.h_full())
            .when(split == Some(SplitAxis::Horizontal), |this| this.w_full())
            .bg(palette.background)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.focus_pane(pane, cx);
                }),
            )
            .on_drop::<TabDrag>(cx.listener(move |this, drag: &TabDrag, _, cx| {
                this.move_tab_to_pane(drag.tab_id, pane, cx);
            }))
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .overflow_hidden()
                    .border_b_1()
                    .border_color(palette.border)
                    .when(focused, |this| this.bg(palette.panel))
                    .when(show_sidebar_toggle && !sidebar_open, |this| {
                        this.child(
                            Button::new("expand-sidebar")
                                .ghost()
                                .compact()
                                .icon(IconName::PanelLeftOpen)
                                .tooltip("Show sidebar")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.toggle_sidebar(cx);
                                })),
                        )
                    })
                    .child(
                        div().flex_1().min_w(px(0.0)).overflow_hidden().child(
                            TabBar::new(tab_bar_id)
                                .underline()
                                .max_width(px(260.0))
                                .selected_index(selected)
                                .children(tabs)
                                .suffix(
                                    Button::new(("pane-new-tab", index))
                                        .ghost()
                                        .compact()
                                        .icon(IconName::Plus)
                                        .tooltip("New tab")
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.focus_pane(pane, cx);
                                            this.new_sql_tab(window, cx);
                                        })),
                                ),
                        ),
                    )
                    .child(
                        Button::new(format!("close-active-{pane:?}"))
                            .flex_shrink_0()
                            .ghost()
                            .compact()
                            .icon(IconName::Close)
                            .tooltip("Close active tab")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.close_active_tab(pane, window, cx);
                            })),
                    )
                    .when(split.is_some(), |this| {
                        let target = match pane {
                            Pane::Primary => Pane::Secondary,
                            Pane::Secondary => Pane::Primary,
                        };
                        let icon = match pane {
                            Pane::Primary => IconName::ArrowRight,
                            Pane::Secondary => IconName::ArrowLeft,
                        };
                        this.child(
                            Button::new(format!("move-tab-{pane:?}"))
                                .flex_shrink_0()
                                .ghost()
                                .compact()
                                .icon(icon)
                                .tooltip("Move active tab to other pane")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.move_tab_to_pane(tab_id, target, cx);
                                })),
                        )
                    })
                    .when(pane == Pane::Primary, |this| {
                        this.child(
                            h_flex()
                                .flex_shrink_0()
                                .items_center()
                                .gap_1()
                                .child(
                                    Button::new("split-vertical")
                                        .when(split == Some(SplitAxis::Vertical), |this| {
                                            this.secondary()
                                        })
                                        .when(split != Some(SplitAxis::Vertical), |this| {
                                            this.ghost()
                                        })
                                        .compact()
                                        .icon(IconName::PanelLeft)
                                        .tooltip(if split == Some(SplitAxis::Vertical) {
                                            "Close split"
                                        } else {
                                            "Split vertically"
                                        })
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.toggle_split(SplitAxis::Vertical, window, cx);
                                        })),
                                )
                                .child(
                                    Button::new("split-horizontal")
                                        .when(split == Some(SplitAxis::Horizontal), |this| {
                                            this.secondary()
                                        })
                                        .when(split != Some(SplitAxis::Horizontal), |this| {
                                            this.ghost()
                                        })
                                        .compact()
                                        .icon(IconName::PanelBottom)
                                        .tooltip(if split == Some(SplitAxis::Horizontal) {
                                            "Close split"
                                        } else {
                                            "Split horizontally"
                                        })
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.toggle_split(SplitAxis::Horizontal, window, cx);
                                        })),
                                ),
                        )
                    }),
            )
            .when(!notice.is_empty(), |this| {
                this.child(
                    div()
                        .mx_2()
                        .mt_1()
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .border_1()
                        .border_color(palette.border)
                        .bg(palette.panel)
                        .text_xs()
                        .text_color(palette.muted)
                        .child(notice),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .w_full()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .child(body),
            )
            .into_any_element()
    }

    fn tab_elements(&self, pane: Pane, cx: &mut Context<Self>) -> Vec<BarTab> {
        self.pane_tab_ids(pane)
            .iter()
            .enumerate()
            .filter_map(|(order, tab_id)| {
                let index = self.tab_index(*tab_id)?;
                let tab = &self.tabs[index];
                let icon = match tab.profile.as_ref() {
                    Some(profile) => super::connection_view::engine_icon(profile.engine),
                    None => Icon::new(match tab.body {
                        TabBody::Query(_) => IconName::SquareTerminal,
                        TabBody::Table(_) => IconName::LayoutDashboard,
                    }),
                };
                let title = tab.title.clone();
                let drag = TabDrag {
                    tab_id: tab.id,
                    title: title.clone(),
                };
                Some(
                    BarTab::new()
                        .prefix(
                            div()
                                .id(("tab-drag", drag.tab_id))
                                .flex()
                                .items_center()
                                .on_drag(drag, |drag, _, _, cx| {
                                    cx.stop_propagation();
                                    cx.new(|_| drag.clone())
                                })
                                .child(icon.with_size(Size::Small)),
                        )
                        .label(title)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.switch_tab(pane, order, cx);
                        })),
                )
            })
            .collect()
    }
}
