use crate::models::{ConnectionProfile, ProfileInput, ProfileSummary};
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
    let host = profile.host.trim();
    let database = profile.database.trim();
    let username = profile.username.trim();
    if name.is_empty() || host.is_empty() || database.is_empty() || username.is_empty() {
        return Err("Name, host, database, and username are required.".into());
    }
    if profile.port == 0 {
        return Err("Port must be between 1 and 65535.".into());
    }

    let mut profiles = load_profiles(&app)?;
    let is_new = profile.id.is_none();
    let id = profile.id.unwrap_or_else(Uuid::new_v4);
    let previous = profiles.iter().find(|item| item.id == id).cloned();
    let saved = ConnectionProfile {
        id,
        name: name.to_owned(),
        host: host.to_owned(),
        port: profile.port,
        database: database.to_owned(),
        username: username.to_owned(),
        ssl_mode: profile.ssl_mode,
    };
    let target_changed = previous.as_ref().is_some_and(|previous| {
        previous.host != saved.host
            || previous.port != saved.port
            || previous.database != saved.database
            || previous.username != saved.username
    });

    let old_profiles = profiles.clone();
    if let Some(index) = profiles.iter().position(|item| item.id == id) {
        profiles[index] = saved.clone();
    } else if is_new {
        profiles.push(saved.clone());
    } else {
        return Err("Connection profile no longer exists.".into());
    }

    let password_to_save = password.filter(|password| !password.is_empty());
    if password_to_save.is_none() && target_changed {
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
    let old_profiles = profiles.clone();
    let old_len = profiles.len();
    profiles.retain(|profile| profile.id != id);
    if profiles.len() == old_len {
        return Err("Connection profile does not exist.".into());
    }
    write_profiles(&app, &profiles)?;

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
