#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;

use app::KoyoApp;
use fs2::FileExt;
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;
use std::fs::{self, File, OpenOptions};
use std::sync::Arc;

actions!(koyo, [RunQuery]);

fn acquire_instance_lock() -> Result<File, String> {
    let directory = koyo_core::storage::default_config_dir();
    fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not create the Koyo config directory: {error}"))?;
    let path = directory.join("koyo.lock");
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(&path)
        .map_err(|error| format!("Could not open the Koyo lock file: {error}"))?;
    file.try_lock_exclusive()
        .map_err(|_| "Koyo is already running in another window.".to_string())?;
    Ok(file)
}

fn main() {
    let _instance_lock = match acquire_instance_lock() {
        Ok(lock) => lock,
        Err(error) => {
            rfd::MessageDialog::new()
                .set_title("Koyo")
                .set_description(error)
                .show();
            return;
        }
    };
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
