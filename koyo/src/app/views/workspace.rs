use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::tab::{Tab as BarTab, TabBar};
use gpui_kit::component::{Icon, IconName, Size};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use gpui_kit::{CursorStyle, DragMoveEvent, InteractiveElement, StatefulInteractiveElement};

use super::{query, table, widgets};
use crate::app::{KoyoApp, Palette, Pane, SplitAxis, TabBody, engine_label};

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

impl KoyoApp {
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
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.persist_state(cx)),
                    )
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
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.persist_state(cx)),
                    )
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
            TabBody::Query(_) => query::render(self, index, palette, cx),
            TabBody::Table(_) => table::render(self, index, palette, cx),
        };
        let tabs = self.tab_elements(pane, cx);
        let selected = self.pane_selected(pane);
        let focused = self.focus == pane;
        let sidebar_open = self.sidebar_open;
        let ai_open = self.ai_open;
        let split = self.split;
        let tab_bar_id: SharedString = match pane {
            Pane::Primary => "primary-tabs".into(),
            Pane::Secondary => "secondary-tabs".into(),
        };
        let tab_scroll = self.tab_scroll(pane).clone();

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
                        this.child(widgets::button_tooltip(
                            "top-tooltip-sidebar",
                            Button::new("expand-sidebar")
                                .ghost()
                                .compact()
                                .icon(IconName::PanelLeftOpen)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.toggle_sidebar(cx);
                                })),
                            "Show sidebar",
                        ))
                    })
                    .child(
                        div().flex_1().min_w(px(0.0)).overflow_hidden().child(
                            TabBar::new(tab_bar_id)
                                .underline()
                                .menu(true)
                                .track_scroll(&tab_scroll)
                                .max_width(px(260.0))
                                .selected_index(selected)
                                .children(tabs)
                                .suffix(widgets::button_tooltip(
                                    ("top-tooltip-new", index),
                                    Button::new(("pane-new-tab", index))
                                        .ghost()
                                        .compact()
                                        .icon(IconName::Plus)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.focus_pane(pane, cx);
                                            this.new_sql_tab(window, cx);
                                        })),
                                    "New tab",
                                )),
                        ),
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
                        this.child(widgets::button_tooltip(
                            ("top-tooltip-move", index),
                            Button::new(format!("move-tab-{pane:?}"))
                                .flex_shrink_0()
                                .ghost()
                                .compact()
                                .icon(icon)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.move_tab_to_pane(tab_id, target, cx);
                                })),
                            "Move active tab to other pane",
                        ))
                    })
                    .when(pane == Pane::Primary, |this| {
                        this.child(
                            h_flex()
                                .flex_shrink_0()
                                .items_center()
                                .gap_1()
                                .child(widgets::button_tooltip(
                                    "top-tooltip-ai",
                                    Button::new("toggle-ai")
                                        .when(ai_open, |this| this.secondary())
                                        .when(!ai_open, |this| this.ghost())
                                        .compact()
                                        .icon(IconName::Bot)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.toggle_ai(window, cx);
                                        })),
                                    if ai_open { "Hide AI" } else { "Show AI" },
                                ))
                                .child(widgets::button_tooltip(
                                    "top-tooltip-split-vertical",
                                    Button::new("split-vertical")
                                        .when(split == Some(SplitAxis::Vertical), |this| {
                                            this.secondary()
                                        })
                                        .when(split != Some(SplitAxis::Vertical), |this| {
                                            this.ghost()
                                        })
                                        .compact()
                                        .icon(IconName::PanelLeft)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.toggle_split(SplitAxis::Vertical, window, cx);
                                        })),
                                    if split == Some(SplitAxis::Vertical) {
                                        "Close split"
                                    } else {
                                        "Split vertically"
                                    },
                                ))
                                .child(widgets::button_tooltip(
                                    "top-tooltip-split-horizontal",
                                    Button::new("split-horizontal")
                                        .when(split == Some(SplitAxis::Horizontal), |this| {
                                            this.secondary()
                                        })
                                        .when(split != Some(SplitAxis::Horizontal), |this| {
                                            this.ghost()
                                        })
                                        .compact()
                                        .icon(IconName::PanelBottom)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.toggle_split(SplitAxis::Horizontal, window, cx);
                                        })),
                                    if split == Some(SplitAxis::Horizontal) {
                                        "Close split"
                                    } else {
                                        "Split horizontally"
                                    },
                                )),
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
                    Some(profile) => super::connection::engine_icon(profile.engine),
                    None => Icon::new(match tab.body {
                        TabBody::Query(_) => IconName::SquareTerminal,
                        TabBody::Table(_) => IconName::LayoutDashboard,
                    }),
                };
                let title = tab.title.clone();
                let tab_id = tab.id;
                let drag = TabDrag {
                    tab_id,
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
                        .suffix(widgets::button_tooltip(
                            ("tooltip-tab-close", tab_id),
                            Button::new(("tab-close", tab_id))
                                .ghost()
                                .compact()
                                .icon(IconName::Close)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.close_tab(tab_id, window, cx);
                                })),
                            "Close tab",
                        ))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.switch_tab(pane, order, cx);
                        })),
                )
            })
            .collect()
    }
}
