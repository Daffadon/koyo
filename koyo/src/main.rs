#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;

use app::KoyoApp;
use fs2::FileExt;
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;
use std::fs::{self, File, OpenOptions};
use std::path::Path;
use std::sync::Arc;

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};

actions!(koyo, [RunQuery, ToggleAi, ToggleSidebar]);

fn ensure_config_directory(directory: &Path) -> Result<(), String> {
    match fs::symlink_metadata(directory) {
        Ok(metadata) if !metadata.is_dir() => {
            return Err("The Koyo config path is not a directory.".into());
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            builder.mode(0o700);
            builder
                .create(directory)
                .map_err(|error| format!("Could not create the Koyo config directory: {error}"))?;
        }
        Err(error) => {
            return Err(format!(
                "Could not inspect the Koyo config directory: {error}"
            ));
        }
    }

    #[cfg(unix)]
    {
        let directory_file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(directory)
            .map_err(|error| format!("Could not open the Koyo config directory: {error}"))?;
        let metadata = directory_file
            .metadata()
            .map_err(|error| format!("Could not inspect the Koyo config directory: {error}"))?;
        if !metadata.is_dir() || metadata.uid() != unsafe { libc::geteuid() } {
            return Err("The Koyo config directory is not owned by this user.".into());
        }
        directory_file
            .set_permissions(fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("Could not secure the Koyo config directory: {error}"))?;
    }

    Ok(())
}

fn acquire_instance_lock() -> Result<File, String> {
    let directory = koyo_core::storage::default_config_dir();
    ensure_config_directory(&directory)?;
    let path = directory.join("koyo.lock");
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(&path)
        .map_err(|error| format!("Could not open the Koyo lock file: {error}"))?;
    #[cfg(unix)]
    {
        let metadata = file
            .metadata()
            .map_err(|error| format!("Could not inspect the Koyo lock file: {error}"))?;
        if !metadata.is_file() || metadata.uid() != unsafe { libc::geteuid() } {
            return Err("The Koyo lock file is not owned by this user.".into());
        }
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("Could not secure the Koyo lock file: {error}"))?;
    }
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
            gpui_kit::component::tooltip::TooltipDefaults::new()
                .with_show_delay(std::time::Duration::from_millis(120))
                .with_grace_period(std::time::Duration::from_millis(600))
                .install(cx);
            Theme::change(ThemeMode::Dark, None, cx);
            cx.bind_keys([
                KeyBinding::new("ctrl-enter", RunQuery, Some("Input")),
                KeyBinding::new("cmd-enter", RunQuery, Some("Input")),
                KeyBinding::new("ctrl-b", ToggleSidebar, Some("Koyo")),
                KeyBinding::new("cmd-b", ToggleSidebar, Some("Koyo")),
                KeyBinding::new("ctrl-alt-b", ToggleAi, Some("Koyo")),
                KeyBinding::new("cmd-alt-b", ToggleAi, Some("Koyo")),
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
