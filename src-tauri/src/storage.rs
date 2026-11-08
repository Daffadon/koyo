use crate::models::{ConnectionProfile, ProfileInput, ProfileSummary};
use keyring::v1::{Entry, Error as KeyringError};
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
    fs::create_dir_all(directory)
        .map_err(|error| format!("Could not create application config directory: {error}"))?;

    let temporary_path = directory.join(format!(".profiles-{}.tmp", Uuid::new_v4()));
    let contents = serde_json::to_vec_pretty(profiles)
        .map_err(|error| format!("Could not encode connection profiles: {error}"))?;
    fs::write(&temporary_path, contents)
        .map_err(|error| format!("Could not write connection profiles: {error}"))?;
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
    let saved = ConnectionProfile {
        id,
        name: name.to_owned(),
        host: host.to_owned(),
        port: profile.port,
        database: database.to_owned(),
        username: username.to_owned(),
        ssl_mode: profile.ssl_mode,
    };

    let old_profiles = profiles.clone();
    if let Some(index) = profiles.iter().position(|item| item.id == id) {
        profiles[index] = saved.clone();
    } else if is_new {
        profiles.push(saved.clone());
    } else {
        return Err("Connection profile no longer exists.".into());
    }

    write_profiles(&app, &profiles)?;
    let password_to_save = password.filter(|password| !password.is_empty());
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
