use crate::models::{ConnectionProfile, ProfileInput, SslMode};
use keyring::v1::{Entry, Error as KeyringError};
use std::fs;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::PathBuf;
use uuid::Uuid;

pub const APP_IDENTIFIER: &str = "com.koyo.db";
const SERVICE_NAME: &str = "com.koyo.db";
const PROFILE_FILE: &str = "profiles.json";

pub fn default_config_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return PathBuf::from(appdata).join(APP_IDENTIFIER);
    }
    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join(APP_IDENTIFIER);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(config) = std::env::var_os("XDG_CONFIG_HOME") {
            return PathBuf::from(config).join(APP_IDENTIFIER);
        }
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(".config").join(APP_IDENTIFIER);
        }
    }
    // All normal desktop launches have one of the variables above. If a
    // stripped environment lacks them, use a fixed executable-relative path
    // rather than a working directory an attacker might control.
    if let Ok(executable) = std::env::current_exe()
        && let Some(parent) = executable.parent()
    {
        return parent.join(APP_IDENTIFIER);
    }
    PathBuf::from(APP_IDENTIFIER)
}

#[derive(Clone, Debug)]
pub struct ProfileStore {
    config_dir: PathBuf,
}

impl ProfileStore {
    pub fn new(config_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: config_dir.into(),
        }
    }

    pub fn from_default_dir() -> Self {
        Self::new(default_config_dir())
    }

    fn profile_path(&self) -> PathBuf {
        self.config_dir.join(PROFILE_FILE)
    }

    pub fn load_profiles(&self) -> Result<Vec<ConnectionProfile>, String> {
        let path = self.profile_path();
        match fs::read(&path) {
            Ok(contents) => {
                let contents = contents
                    .strip_prefix(&[0xEF, 0xBB, 0xBF])
                    .unwrap_or(&contents);
                serde_json::from_slice(contents)
                    .map_err(|error| format!("Could not read saved connection profiles: {error}"))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(format!("Could not read saved connection profiles: {error}")),
        }
    }

    fn write_profiles(&self, profiles: &[ConnectionProfile]) -> Result<(), String> {
        let path = self.profile_path();
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

    pub fn save_profile(
        &self,
        profile: ProfileInput,
        password: Option<String>,
    ) -> Result<ConnectionProfile, String> {
        let name = profile.name.trim();
        let sqlite_path = profile.sqlite_path.trim();
        let host = profile.host.trim();
        let database = profile.database.trim();
        let username = profile.username.trim();
        if name.is_empty() {
            return Err("Connection name is required.".into());
        }
        if profile.engine.is_sqlite() {
            if sqlite_path.is_empty() {
                return Err("SQLite database file is required.".into());
            }
        } else {
            if host.is_empty() || database.is_empty() || username.is_empty() {
                return Err("Host, database, and username are required.".into());
            }
            if profile.port == 0 {
                return Err("Port must be between 1 and 65535.".into());
            }
        }

        let mut profiles = self.load_profiles()?;
        let is_new = profile.id.is_none();
        let id = profile.id.unwrap_or_else(Uuid::new_v4);
        let previous = profiles.iter().find(|item| item.id == id).cloned();
        let saved = ConnectionProfile {
            id,
            name: name.to_owned(),
            engine: profile.engine,
            host: if profile.engine.is_sqlite() {
                String::new()
            } else {
                host.to_owned()
            },
            port: if profile.engine.is_sqlite() {
                0
            } else {
                profile.port
            },
            database: if profile.engine.is_sqlite() {
                String::new()
            } else {
                database.to_owned()
            },
            username: if profile.engine.is_sqlite() {
                String::new()
            } else {
                username.to_owned()
            },
            ssl_mode: if profile.engine.is_sqlite() {
                SslMode::Disable
            } else {
                profile.ssl_mode
            },
            sqlite_path: if profile.engine.is_sqlite() {
                sqlite_path.to_owned()
            } else {
                String::new()
            },
        };
        let target_changed = previous.as_ref().is_some_and(|previous| {
            previous.engine != saved.engine
                || (!saved.engine.is_sqlite()
                    && (previous.host != saved.host
                        || previous.port != saved.port
                        || previous.database != saved.database
                        || previous.username != saved.username))
                || (saved.engine.is_sqlite() && previous.sqlite_path != saved.sqlite_path)
        });

        let old_profiles = profiles.clone();
        if let Some(index) = profiles.iter().position(|item| item.id == id) {
            profiles[index] = saved.clone();
        } else if is_new {
            profiles.push(saved.clone());
        } else {
            return Err("Connection profile no longer exists.".into());
        }

        let password_to_save = if saved.engine.needs_password() {
            password.filter(|password| !password.is_empty())
        } else {
            None
        };
        let must_clear_credential = target_changed
            && (saved.engine.needs_password()
                || previous
                    .as_ref()
                    .is_some_and(|previous| previous.engine.needs_password()));
        // Commit the profile first. If credential cleanup fails, roll the
        // profile back so the old target and its saved password stay paired.
        self.write_profiles(&profiles)?;
        if password_to_save.is_none() && must_clear_credential {
            let entry = credential_entry(id)?;
            match entry.delete_credential() {
                Ok(()) | Err(KeyringError::NoEntry) => {}
                Err(error) => {
                    let _ = self.write_profiles(&old_profiles);
                    return Err(format!(
                        "Could not remove the saved password for the changed connection target: {error}"
                    ));
                }
            }
        }
        if let Some(password) = password_to_save {
            let entry = match credential_entry(id) {
                Ok(entry) => entry,
                Err(error) => {
                    let _ = self.write_profiles(&old_profiles);
                    return Err(error);
                }
            };
            if let Err(error) = entry.set_password(&password) {
                let _ = self.write_profiles(&old_profiles);
                return Err(format!(
                    "Could not save password in the operating-system credential store: {error}"
                ));
            }
        }

        Ok(saved)
    }

    pub fn delete_profile(&self, id: Uuid) -> Result<(), String> {
        let mut profiles = self.load_profiles()?;
        let engine = profiles
            .iter()
            .find(|profile| profile.id == id)
            .map(|profile| profile.engine)
            .ok_or_else(|| "Connection profile does not exist.".to_string())?;
        let old_profiles = profiles.clone();
        profiles.retain(|profile| profile.id != id);
        self.write_profiles(&profiles)?;
        if engine.is_sqlite() {
            return Ok(());
        }

        let entry = match credential_entry(id) {
            Ok(entry) => entry,
            Err(error) => {
                let _ = self.write_profiles(&old_profiles);
                return Err(error);
            }
        };
        match entry.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(error) => {
                let _ = self.write_profiles(&old_profiles);
                Err(format!(
                    "Could not remove password from the operating-system credential store: {error}"
                ))
            }
        }
    }

    pub fn load_password(&self, id: Uuid) -> Result<String, String> {
        credential_entry(id)?
            .get_password()
            .map_err(|error| match error {
                KeyringError::NoEntry => String::new(),
                error => format!(
                    "Could not read password from the operating-system credential store: {error}"
                ),
            })
    }
}

fn credential_entry(id: Uuid) -> Result<Entry, String> {
    Entry::new(SERVICE_NAME, &id.to_string()).map_err(|error| error.to_string())
}
