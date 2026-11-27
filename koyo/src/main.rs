#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;

use app::KoyoApp;
use gpui_kit::component::input::ShowCompletions;
use gpui_kit::component::{Theme, ThemeMode, TitleBar};
use gpui_kit::*;
use std::fs::{self, File, OpenOptions};
use std::path::Path;
use std::sync::Arc;

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};

actions!(
    koyo,
    [
        CloseTab,
        CompleteFilter,
        DeleteLine,
        DuplicateLine,
        FormatQuery,
        MoveLineDown,
        MoveLineUp,
        MoveTabLeft,
        MoveTabRight,
        NewTab,
        NextTab,
        PreviousTab,
        RefreshTable,
        RunQuery,
        ToggleAi,
        ToggleComment,
        ToggleSidebar,
        QuickOpen
    ]
);

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
    file.try_lock()
        .map_err(|_| "Koyo is already running in another window.".to_string())?;
    Ok(file)
}

fn main() {
    let config_directory = koyo_core::storage::default_config_dir();
    if let Err(error) = ensure_config_directory(&config_directory) {
        rfd::MessageDialog::new()
            .set_title("Koyo")
            .set_description(error)
            .show();
        return;
    }
    // Acquire the single-instance lock before initializing logging so a second
    // instance cannot rotate or write to the active instance's log file.
    let _instance_lock = match acquire_instance_lock() {
        Ok(lock) => lock,
        Err(error) => {
            eprintln!("Could not acquire the Koyo instance lock: {error}");
            rfd::MessageDialog::new()
                .set_title("Koyo")
                .set_description(error)
                .show();
            return;
        }
    };
    if let Err(error) = app::init_logging() {
        eprintln!("Could not initialize Koyo logging: {error}");
        rfd::MessageDialog::new()
            .set_title("Koyo")
            .set_description(format!("Could not initialize Koyo logging: {error}"))
            .show();
    }
    app::ensure_crypto_provider();
    app::install_panic_hook();
    log::info!(
        "Koyo {} starting on {} {}",
        app::VERSION,
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => Arc::new(runtime),
        Err(error) => {
            let message = format!("Could not create the database runtime: {error}");
            log::error!("{message}");
            rfd::MessageDialog::new()
                .set_title("Koyo")
                .set_description(message)
                .show();
            return;
        }
    };

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            gpui_kit::component::tooltip::TooltipDefaults::new()
                .with_show_delay(std::time::Duration::from_millis(120))
                .with_grace_period(std::time::Duration::from_millis(600))
                .install(cx);
            Theme::change(ThemeMode::Dark, None, cx);
            // Non-Koyo editor overrides; all `koyo::*` bindings are applied
            // from `SHORTCUTS` by `KoyoApp::apply_shortcut_bindings`.
            cx.bind_keys([
                // The SQL editor handles Tab in its capture listener so a
                // completion accept cannot also insert indentation.
                KeyBinding::new("tab", NoAction {}, Some("Input && mode == editor")),
                KeyBinding::new(
                    "ctrl-space",
                    ShowCompletions,
                    Some("Input && mode == editor"),
                ),
            ]);
            // Capture handles Ctrl+Backspace so trailing whitespace can be
            // removed without also deleting the word before it.
            #[cfg(not(target_os = "macos"))]
            cx.bind_keys([KeyBinding::new(
                "ctrl-backspace",
                NoAction {},
                Some("Input && mode == editor"),
            )]);

            let bounds = Bounds::centered(None, size(px(1440.0), px(960.0)), cx);
            let runtime = runtime.clone();
            let mut options = TitleBar::window_options();
            options.window_bounds = Some(WindowBounds::Windowed(bounds));
            options.window_decorations = Some(WindowDecorations::Client);
            options.app_id = Some("koyo".into());
            options.icon = image::load_from_memory(include_bytes!("../assets/koyo.png"))
                .ok()
                .map(|icon| Arc::new(icon.to_rgba8()));
            let mut titlebar = TitleBar::title_bar_options();
            titlebar.title = Some("Koyo".into());
            options.titlebar = Some(titlebar);
            if let Err(error) = gpui_kit::open_window(options, cx, move |window, cx| {
                cx.new(|cx| KoyoApp::new(window, cx, runtime))
            }) {
                let message = format!("Could not open the Koyo window: {error}");
                log::error!("{message}");
                rfd::MessageDialog::new()
                    .set_title("Koyo")
                    .set_description(message)
                    .show();
            }
        });
}
