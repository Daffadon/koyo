use gpui_kit::InteractiveElement;
use gpui_kit::component::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::widgets::input_field;
use crate::app::*;

const SCOPES: [(ExportScope, &str); 4] = [
    (ExportScope::CurrentPage, "Current page"),
    (ExportScope::Page, "Page"),
    (ExportScope::Range, "Range"),
    (ExportScope::All, "All pages"),
];

pub(super) fn render_export(
    dialog: &TableExportDialog,
    palette: &Palette,
    cx: &mut Context<KoyoApp>,
) -> AnyElement {
    let scope_buttons =
        h_flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .children(
                SCOPES
                    .into_iter()
                    .enumerate()
                    .map(|(index, (scope, label))| {
                        Button::new(("export-scope", index))
                            .compact()
                            .label(label)
                            .when(scope == dialog.scope, |this| this.primary())
                            .when(scope != dialog.scope, |this| this.ghost())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_table_export_scope(scope, cx);
                            }))
                    }),
            );

    let detail = match dialog.scope {
        ExportScope::CurrentPage => {
            let total = if dialog.total_pages > 0 {
                format!(" of {}", dialog.total_pages)
            } else {
                String::new()
            };
            div()
                .text_xs()
                .text_color(palette.muted)
                .child(format!("Page {}{total}.", dialog.current_page))
                .into_any_element()
        }
        ExportScope::Page => input_field("Page", &dialog.page, false, palette).into_any_element(),
        ExportScope::Range => h_flex()
            .gap_3()
            .child(div().flex_1().min_w(px(0.0)).child(input_field(
                "Start page",
                &dialog.page,
                false,
                palette,
            )))
            .child(div().flex_1().min_w(px(0.0)).child(input_field(
                "End page",
                &dialog.end_page,
                false,
                palette,
            )))
            .into_any_element(),
        ExportScope::All => div()
            .text_xs()
            .text_color(palette.muted)
            .child("Every page, using the current filters and sort.")
            .into_any_element(),
    };

    div()
        .id("export-overlay")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
            if event.keystroke.key == "escape" {
                this.close_table_export(cx);
            }
        }))
        .child(
            div()
                .id("export-backdrop")
                .absolute()
                .inset_0()
                .bg(gpui_kit::hsla(0.0, 0.0, 0.0, 0.35))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.close_table_export(cx);
                })),
        )
        .child(
            v_flex()
                .relative()
                .occlude()
                .w(px(520.0))
                .max_w(relative(0.94))
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
                                .min_w(px(0.0))
                                .child(
                                    div()
                                        .text_lg()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("Download CSV"),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(palette.muted)
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .child(dialog.title.clone()),
                                ),
                        )
                        .child(
                            Button::new("export-close")
                                .ghost()
                                .compact()
                                .icon(IconName::Close)
                                .accessibility_label("Close CSV download")
                                .tooltip("Close")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.close_table_export(cx);
                                })),
                        ),
                )
                .child(
                    v_flex()
                        .gap_3()
                        .px_5()
                        .py_4()
                        .child(div().text_xs().text_color(palette.muted).child("Pages"))
                        .child(scope_buttons)
                        .child(detail)
                        .when(!dialog.error.is_empty(), |this| {
                            this.child(
                                div()
                                    .text_xs()
                                    .text_color(palette.danger)
                                    .child(dialog.error.clone()),
                            )
                        }),
                )
                .child(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .px_5()
                        .py_3()
                        .border_t_1()
                        .border_color(palette.border)
                        .child(
                            Button::new("export-cancel")
                                .secondary()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.close_table_export(cx);
                                })),
                        )
                        .child(
                            Button::new("export-download")
                                .primary()
                                .icon(IconName::ArrowDown)
                                .label("Download CSV")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.download_table_csv(window, cx);
                                })),
                        ),
                ),
        )
        .into_any_element()
}
