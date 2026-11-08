use crate::models::{ConnectionProfile, Engine, ProfileInput, ProfileSummary, SslMode};
use keyring::v1::{Entry, Error as KeyringError};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::{fs, path::PathBuf};
use tauri::{AppHandle, Manager};
use uuid::Uuid;

const SERVICE_NAME: &str = "com.koyo.db";
const PROFILE_FILE: &str = "profiles.json";

fn profile_path(app: &AppHandle) -> Result<PathBuf, String> {
    let directory = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("Could not locate application config directory: {error}"))?;
    Ok(directory.join(PROFILE_FILE))
}

fn credential_entry(id: Uuid) -> Result<Entry, String> {
    Entry::new(SERVICE_NAME, &id.to_string()).map_err(|error| error.to_string())
}

pub fn load_profiles(app: &AppHandle) -> Result<Vec<ConnectionProfile>, String> {
    let path = profile_path(app)?;
    match fs::read(&path) {
        Ok(contents) => serde_json::from_slice(&contents)
            .map_err(|error| format!("Could not read saved connection profiles: {error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(format!("Could not read saved connection profiles: {error}")),
    }
}

fn write_profiles(app: &AppHandle, profiles: &[ConnectionProfile]) -> Result<(), String> {
    let path = profile_path(app)?;
    let directory = path
        .parent()
        .ok_or_else(|| "Invalid application config path".to_string())?;
    let mut directory_builder = fs::DirBuilder::new();
    directory_builder.recursive(true);
    #[cfg(unix)]
    directory_builder.mode(0o700);
    directory_builder
        .create(directory)
        .map_err(|error| format!("Could not create application config directory: {error}"))?;

    let temporary_path = directory.join(format!(".profiles-{}.tmp", Uuid::new_v4()));
    let contents = serde_json::to_vec_pretty(profiles)
        .map_err(|error| format!("Could not encode connection profiles: {error}"))?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&temporary_path)
        .map_err(|error| format!("Could not write connection profiles: {error}"))?;
    let write_result = file.write_all(&contents);
    drop(file);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary_path);
        return Err(format!("Could not write connection profiles: {error}"));
    }
    if let Err(error) = fs::rename(&temporary_path, &path) {
        let _ = fs::remove_file(&temporary_path);
        return Err(format!("Could not save connection profiles: {error}"));
    }
    Ok(())
}

#[tauri::command]
pub fn list_profiles(app: AppHandle) -> Result<Vec<ProfileSummary>, String> {
    load_profiles(&app).map(|profiles| profiles.iter().map(ProfileSummary::from).collect())
}

#[tauri::command]
pub fn save_profile(
    app: AppHandle,
    profile: ProfileInput,
    password: Option<String>,
) -> Result<ProfileSummary, String> {
    let name = profile.name.trim();
    let sqlite_path = profile.sqlite_path.trim();
    let host = profile.host.trim();
    let database = profile.database.trim();
    let username = profile.username.trim();
    if name.is_empty() {
        return Err("Connection name is required.".into());
    }
    if profile.engine == Engine::Postgres {
        if host.is_empty() || database.is_empty() || username.is_empty() {
            return Err("Host, database, and username are required.".into());
        }
        if profile.port == 0 {
            return Err("Port must be between 1 and 65535.".into());
        }
    } else if sqlite_path.is_empty() {
        return Err("SQLite database file is required.".into());
    }

    let mut profiles = load_profiles(&app)?;
    let is_new = profile.id.is_none();
    let id = profile.id.unwrap_or_else(Uuid::new_v4);
    let previous = profiles.iter().find(|item| item.id == id).cloned();
    let saved = ConnectionProfile {
        id,
        name: name.to_owned(),
        engine: profile.engine,
        host: if profile.engine == Engine::Postgres {
            host.to_owned()
        } else {
            String::new()
        },
        port: if profile.engine == Engine::Postgres {
            profile.port
        } else {
            0
        },
        database: if profile.engine == Engine::Postgres {
            database.to_owned()
        } else {
            String::new()
        },
        username: if profile.engine == Engine::Postgres {
            username.to_owned()
        } else {
            String::new()
        },
        ssl_mode: if profile.engine == Engine::Postgres {
            profile.ssl_mode
        } else {
            SslMode::Disable
        },
        sqlite_path: if profile.engine == Engine::Sqlite {
            sqlite_path.to_owned()
        } else {
            String::new()
        },
    };
    let target_changed = previous.as_ref().is_some_and(|previous| {
        previous.engine != saved.engine
            || (saved.engine == Engine::Postgres
                && (previous.host != saved.host
                    || previous.port != saved.port
                    || previous.database != saved.database
                    || previous.username != saved.username))
            || (saved.engine == Engine::Sqlite && previous.sqlite_path != saved.sqlite_path)
    });

    let old_profiles = profiles.clone();
    if let Some(index) = profiles.iter().position(|item| item.id == id) {
        profiles[index] = saved.clone();
    } else if is_new {
        profiles.push(saved.clone());
    } else {
        return Err("Connection profile no longer exists.".into());
    }

    let password_to_save = if saved.engine == Engine::Postgres {
        password.filter(|password| !password.is_empty())
    } else {
        None
    };
    let must_clear_credential = target_changed
        && (saved.engine == Engine::Postgres
            || previous
                .as_ref()
                .is_some_and(|previous| previous.engine == Engine::Postgres));
    if password_to_save.is_none() && must_clear_credential {
        let entry = match credential_entry(id) {
            Ok(entry) => entry,
            Err(error) => return Err(error),
        };
        match entry.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => {}
            Err(error) => {
                return Err(format!(
                    "Could not remove the saved password for the changed connection target: {error}"
                ));
            }
        }
    }

    write_profiles(&app, &profiles)?;
    if let Some(password) = password_to_save {
        let entry = match credential_entry(id) {
            Ok(entry) => entry,
            Err(error) => {
                let _ = write_profiles(&app, &old_profiles);
                return Err(error);
            }
        };
        if let Err(error) = entry.set_password(&password) {
            let _ = write_profiles(&app, &old_profiles);
            return Err(format!(
                "Could not save password in the operating-system credential store: {error}"
            ));
        }
    }

    Ok(ProfileSummary::from(&saved))
}

#[tauri::command]
pub fn delete_profile(app: AppHandle, id: Uuid) -> Result<(), String> {
    let mut profiles = load_profiles(&app)?;
    let engine = profiles
        .iter()
        .find(|profile| profile.id == id)
        .map(|profile| profile.engine)
        .ok_or_else(|| "Connection profile does not exist.".to_string())?;
    let old_profiles = profiles.clone();
    profiles.retain(|profile| profile.id != id);
    write_profiles(&app, &profiles)?;
    if engine == Engine::Sqlite {
        return Ok(());
    }

    let entry = match credential_entry(id) {
        Ok(entry) => entry,
        Err(error) => {
            let _ = write_profiles(&app, &old_profiles);
            return Err(error);
        }
    };
    match entry.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(error) => {
            let _ = write_profiles(&app, &old_profiles);
            Err(format!(
                "Could not remove password from the operating-system credential store: {error}"
            ))
        }
    }
}

#[tauri::command]
pub fn pick_sqlite_file(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let selected = app
        .dialog()
        .file()
        .add_filter("SQLite database", &["db", "sqlite", "sqlite3"])
        .blocking_pick_file();
    selected
        .map(|path| {
            path.into_path()
                .map(|path| path.to_string_lossy().into_owned())
                .map_err(|error| error.to_string())
        })
        .transpose()
}

pub fn load_password(id: Uuid) -> Result<String, String> {
    credential_entry(id)?
        .get_password()
        .map_err(|error| match error {
            KeyringError::NoEntry => String::new(),
            error => format!(
                "Could not read password from the operating-system credential store: {error}"
            ),
        })
}
