#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;

use app::KoyoApp;
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;
use std::sync::Arc;

actions!(koyo, [RunQuery]);

fn main() {
    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("create database runtime"),
    );

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            Theme::change(ThemeMode::Dark, None, cx);
            cx.bind_keys([
                KeyBinding::new("ctrl-enter", RunQuery, Some("Input")),
                KeyBinding::new("cmd-enter", RunQuery, Some("Input")),
            ]);

            let bounds = Bounds::centered(None, size(px(1440.0), px(960.0)), cx);
            let runtime = runtime.clone();
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Koyo".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                cx,
                move |window, cx| cx.new(|cx| KoyoApp::new(window, cx, runtime)),
            )
            .expect("open Koyo window");
        });
}
