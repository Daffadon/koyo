use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::Input;
use gpui_kit::component::{Disableable, Icon, IconName, Sizable, Size};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use gpui_kit::{InteractiveElement, StatefulInteractiveElement};
use koyo_core::models::Engine;

use super::widgets::{field_label, input_field};
use super::{ConnectionDialog, KoyoApp, Palette, engine_label};

const ENGINES: [Engine; 6] = [
    Engine::Postgres,
    Engine::Sqlite,
    Engine::Mysql,
    Engine::Mariadb,
    Engine::SqlServer,
    Engine::Oracle,
];

// Official database marks, embedded at compile time. Every fill is
// `currentColor` so icons follow the button/row text color and theme.
const ICON_POSTGRES: &[u8] = include_bytes!("../../assets/db-icons/postgresql.svg");
const ICON_SQLITE: &[u8] = include_bytes!("../../assets/db-icons/sqlite.svg");
const ICON_MYSQL: &[u8] = include_bytes!("../../assets/db-icons/mysql.svg");
const ICON_MARIADB: &[u8] = include_bytes!("../../assets/db-icons/mariadb.svg");
const ICON_SQL_SERVER: &[u8] = include_bytes!("../../assets/db-icons/sqlserver.svg");
const ICON_ORACLE: &[u8] = include_bytes!("../../assets/db-icons/oracle.svg");

pub(super) fn engine_icon(engine: Engine) -> Icon {
    let svg = match engine {
        Engine::Postgres => ICON_POSTGRES,
        Engine::Sqlite => ICON_SQLITE,
        Engine::Mysql => ICON_MYSQL,
        Engine::Mariadb => ICON_MARIADB,
        Engine::SqlServer => ICON_SQL_SERVER,
        Engine::Oracle => ICON_ORACLE,
    };
    Icon::default().data(svg)
}

pub(super) fn render_dialog(
    app: &KoyoApp,
    dialog: &ConnectionDialog,
    palette: &Palette,
    cx: &mut Context<KoyoApp>,
) -> AnyElement {
    let busy = dialog.busy;
    let visible = app
        .profiles
        .iter()
        .filter(|profile| profile.engine == dialog.engine)
        .cloned()
        .collect::<Vec<_>>();
    let editing_name = dialog
        .editing
        .and_then(|id| app.profiles.iter().find(|profile| profile.id == id))
        .map(|profile| profile.name.clone());

    let engine_picker =
        h_flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .children(ENGINES.iter().enumerate().map(|(index, engine)| {
                let engine = *engine;
                Button::new(("engine-choice", index))
                    .compact()
                    .icon(engine_icon(engine))
                    .label(engine_label(engine))
                    .when(engine == dialog.engine, |this| this.primary())
                    .when(engine != dialog.engine, |this| this.ghost())
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.choose_engine(engine, window, cx);
                    }))
            }));

    let saved = v_flex()
        .w(px(350.0))
        .h_full()
        .flex_shrink_0()
        .border_r_1()
        .border_color(palette.border)
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .px_4()
                .py_3()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(palette.muted)
                        .child("SAVED CONNECTIONS"),
                )
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .rounded_full()
                        .bg(palette.panel)
                        .text_xs()
                        .text_color(palette.muted)
                        .child(format!("{}", visible.len())),
                ),
        )
        .child(
            v_flex()
                .id("saved-profiles")
                .flex_1()
                .overflow_y_scroll()
                .p_2()
                .gap_1()
                .when(visible.is_empty(), |this| {
                    this.child(
                        div()
                            .p_3()
                            .text_xs()
                            .text_color(palette.muted)
                            .child(format!(
                                "No saved {} connections yet.",
                                engine_label(dialog.engine)
                            )),
                    )
                })
                .children(visible.iter().enumerate().map(|(index, profile)| {
                    let profile = profile.clone();
                    let connect_id = profile.id;
                    let edit_id = profile.id;
                    let delete_id = profile.id;
                    let endpoint = if profile.engine.is_sqlite() {
                        profile.sqlite_path.clone()
                    } else {
                        format!("{}@{}:{}", profile.username, profile.host, profile.port)
                    };
                    let database = if profile.engine.is_sqlite() {
                        "SQLite file".to_string()
                    } else {
                        format!("Database: {}", profile.database)
                    };
                    v_flex()
                        .id(("saved-profile", index))
                        .gap_1()
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .border_1()
                        .border_color(palette.border)
                        .when(dialog.editing == Some(profile.id), |this| {
                            this.bg(palette.panel_alt)
                        })
                        .child(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    engine_icon(profile.engine)
                                        .with_size(Size::Small)
                                        .text_color(palette.muted),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .child(profile.name.clone()),
                                )
                                .child(
                                    Button::new(("profile-edit", index))
                                        .ghost()
                                        .compact()
                                        .icon(IconName::Settings2)
                                        .tooltip("Edit connection")
                                        .disabled(busy)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.edit_profile_in_dialog(edit_id, window, cx);
                                        })),
                                )
                                .child(
                                    Button::new(("profile-delete", index))
                                        .ghost()
                                        .compact()
                                        .icon(IconName::Delete)
                                        .tooltip("Remove connection")
                                        .disabled(busy)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.delete_profile(delete_id, window, cx);
                                        })),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette.muted)
                                .whitespace_normal()
                                .child(endpoint),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette.muted)
                                .whitespace_normal()
                                .child(database),
                        )
                        .child(
                            h_flex().justify_end().child(
                                Button::new(("profile-connect", index))
                                    .primary()
                                    .compact()
                                    .icon(IconName::Globe)
                                    .label("Connect")
                                    .disabled(busy)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.connect_from_dialog(connect_id, window, cx);
                                    })),
                            ),
                        )
                })),
        )
        .child(
            h_flex()
                .items_center()
                .gap_2()
                .px_4()
                .py_3()
                .border_t_1()
                .border_color(palette.border)
                .child(
                    Icon::new(IconName::CircleCheck)
                        .with_size(Size::Small)
                        .text_color(palette.success),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(palette.muted)
                        .child("Passwords stay in the operating-system credential store."),
                ),
        );

    let form =
        v_flex()
            .id("connection-form")
            .flex_1()
            .h_full()
            .overflow_y_scroll()
            .p_4()
            .gap_3()
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(
                        if dialog.editing.is_some() {
                            "Edit connection"
                        } else {
                            "New connection"
                        },
                    ))
                    .when_some(editing_name, |this, name| {
                        this.child(div().text_xs().text_color(palette.muted).child(name))
                    }),
            )
            .child(input_field("Connection name", &dialog.name, busy, palette))
            .when(dialog.engine == Engine::Sqlite, |this| {
                this.child(
                    v_flex()
                        .gap_1()
                        .child(field_label("Database file", palette))
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    div().flex_1().min_w(px(0.0)).child(
                                        Input::new(&dialog.sqlite_path).w_full().disabled(busy),
                                    ),
                                )
                                .child(
                                    Button::new("browse-sqlite")
                                        .secondary()
                                        .label("Browse")
                                        .disabled(busy)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.browse_sqlite(window, cx);
                                        })),
                                ),
                        ),
                )
            })
            .when(dialog.engine != Engine::Sqlite, |this| {
                this.child(
                    h_flex()
                        .items_end()
                        .gap_3()
                        .child(div().flex_1().min_w(px(0.0)).child(input_field(
                            "Host",
                            &dialog.host,
                            busy,
                            palette,
                        )))
                        .child(
                            v_flex()
                                .w(px(120.0))
                                .gap_1()
                                .child(field_label("Port", palette))
                                .child(Input::new(&dialog.port).w_full().disabled(busy)),
                        ),
                )
                .child(
                    h_flex()
                        .items_end()
                        .gap_3()
                        .child(div().flex_1().min_w(px(0.0)).child(input_field(
                            if dialog.engine == Engine::Oracle {
                                "Service name"
                            } else {
                                "Database"
                            },
                            &dialog.database,
                            busy,
                            palette,
                        )))
                        .child(div().flex_1().min_w(px(0.0)).child(input_field(
                            "Username",
                            &dialog.username,
                            busy,
                            palette,
                        ))),
                )
                .child(
                    v_flex()
                        .gap_1()
                        .child(field_label("Password", palette))
                        .child(Input::new(&dialog.password).w_full().disabled(busy))
                        .child(div().text_xs().text_color(palette.muted).child(
                            if dialog.editing.is_some() {
                                "Blank keeps the saved password only when the target is unchanged."
                            } else {
                                "Can be blank for passwordless authentication."
                            },
                        )),
                )
                .child(
                    Checkbox::new("disable-tls")
                        .checked(dialog.ssl_disable)
                        .label("Disable TLS (trusted local connections only)")
                        .disabled(busy)
                        .on_change(cx.listener(|this, checked, _, cx| {
                            this.set_ssl_disable(*checked, cx);
                        })),
                )
                .when(dialog.ssl_disable, |this| {
                    this.child(div().text_xs().text_color(palette.warning).child(
                        "Traffic will not be encrypted. Use only on trusted local networks.",
                    ))
                })
            })
            .when(!dialog.error.is_empty(), |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(palette.danger)
                        .child(dialog.error.clone()),
                )
            })
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .pt_2()
                    .child(
                        Button::new("dialog-cancel")
                            .secondary()
                            .label("Cancel")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.close_connection_dialog(cx);
                            })),
                    )
                    .child(
                        Button::new("dialog-save")
                            .primary()
                            .label(if busy {
                                "Connecting..."
                            } else {
                                "Save and connect"
                            })
                            .loading(busy)
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save_dialog(window, cx);
                            })),
                    ),
            );

    div()
        .id("connection-overlay")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
            if event.keystroke.key == "escape"
                && !this.dialog.as_ref().is_some_and(|dialog| dialog.busy)
            {
                this.close_connection_dialog(cx);
            }
        }))
        .child(
            div()
                .id("connection-backdrop")
                .absolute()
                .inset_0()
                .bg(gpui_kit::hsla(0.0, 0.0, 0.0, 0.35))
                .on_click(cx.listener(|this, _, _, cx| {
                    if !this.dialog.as_ref().is_some_and(|dialog| dialog.busy) {
                        this.close_connection_dialog(cx);
                    }
                })),
        )
        .child(
            v_flex()
                .relative()
                .occlude()
                .w(px(980.0))
                .h(px(660.0))
                .max_w(relative(0.94))
                .max_h(relative(0.92))
                .rounded_lg()
                .border_1()
                .border_color(palette.border)
                .bg(palette.background)
                .shadow_lg()
                .overflow_hidden()
                .child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .px_5()
                        .py_4()
                        .border_b_1()
                        .border_color(palette.border)
                        .child(
                            v_flex()
                                .child(
                                    div()
                                        .text_lg()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("Connections"),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(palette.muted)
                                        .child("Choose a saved profile or add a database."),
                                ),
                        )
                        .child(
                            Button::new("dialog-close")
                                .ghost()
                                .compact()
                                .icon(IconName::Close)
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.close_connection_dialog(cx);
                                })),
                        ),
                )
                .child(
                    div()
                        .px_5()
                        .py_3()
                        .border_b_1()
                        .border_color(palette.border)
                        .child(engine_picker),
                )
                .child(h_flex().flex_1().min_h(px(0.0)).child(saved).child(form)),
        )
        .into_any_element()
}
