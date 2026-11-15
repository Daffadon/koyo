use crate::models::{ConnectionProfile, ProfileInput, SslMode};
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use keyring::v1::{Entry, Error as KeyringError};
use std::fs;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use uuid::Uuid;
use zeroize::Zeroizing;

pub const APP_IDENTIFIER: &str = "com.koyo.db";
const SERVICE_NAME: &str = "com.koyo.db";
const PROFILE_FILE: &str = "profiles.json";
const STATE_KEY_ACCOUNT: &str = "state:key";
const STATE_MAGIC: &[u8] = b"KOYO-STATE-1\n";
const STATE_NONCE_LEN: usize = 24;

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

/// Replace `path` atomically with private permissions. The temporary file sits
/// beside the target so the rename stays on one filesystem.
pub fn write_private_file(path: &Path, contents: &[u8]) -> Result<(), String> {
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

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("state");
    let temporary_path = directory.join(format!(".{file_name}-{}.tmp", Uuid::new_v4()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&temporary_path)
        .map_err(|error| format!("Could not write {file_name}: {error}"))?;
    let write_result = file.write_all(contents);
    drop(file);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary_path);
        return Err(format!("Could not write {file_name}: {error}"));
    }
    if let Err(error) = fs::rename(&temporary_path, path) {
        let _ = fs::remove_file(&temporary_path);
        return Err(format!("Could not replace {file_name}: {error}"));
    }
    Ok(())
}

/// Encrypted application state. The 32-byte data key lives in the operating
/// system credential store; the file itself is XChaCha20-Poly1305 ciphertext.
#[derive(Clone)]
pub struct EncryptedStateStore {
    path: PathBuf,
    key_override: Option<[u8; 32]>,
}

impl std::fmt::Debug for EncryptedStateStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncryptedStateStore")
            .field("path", &self.path)
            .field("has_key_override", &self.key_override.is_some())
            .finish()
    }
}

impl EncryptedStateStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            key_override: None,
        }
    }

    /// Inject a key for tests or embedders that manage their own key material.
    pub fn with_key(path: impl Into<PathBuf>, key: [u8; 32]) -> Self {
        Self {
            path: path.into(),
            key_override: Some(key),
        }
    }

    pub fn load(&self) -> Result<Option<Vec<u8>>, String> {
        let contents = match fs::read(&self.path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(format!(
                    "Could not read encrypted application state: {error}"
                ));
            }
        };
        let key = self.existing_state_key()?;
        decrypt_state(&key, &contents).map(Some)
    }

    pub fn save(&self, plaintext: &[u8]) -> Result<(), String> {
        let key = if self.path.exists() {
            self.existing_state_key()?
        } else {
            self.create_or_load_state_key()?
        };
        let encrypted = encrypt_state(&key, plaintext)?;
        write_private_file(&self.path, &encrypted)
            .map_err(|error| format!("Could not save encrypted application state: {error}"))
    }

    fn state_entry(&self) -> Result<Entry, String> {
        Entry::new(SERVICE_NAME, STATE_KEY_ACCOUNT)
            .map_err(|error| format!("Could not open encrypted state key: {error}"))
    }

    fn existing_state_key(&self) -> Result<Zeroizing<[u8; 32]>, String> {
        if let Some(key) = self.key_override {
            return Ok(Zeroizing::new(key));
        }
        let entry = self.state_entry()?;
        match entry.get_password() {
            Ok(encoded) => decode_state_key(&encoded).map(Zeroizing::new),
            Err(KeyringError::NoEntry) => Err(
                "Encrypted application state exists, but its key is missing from the OS credential store."
                    .into(),
            ),
            Err(error) => Err(format!(
                "Could not read encrypted state key from the OS credential store: {error}"
            )),
        }
    }

    fn create_or_load_state_key(&self) -> Result<Zeroizing<[u8; 32]>, String> {
        if let Some(key) = self.key_override {
            return Ok(Zeroizing::new(key));
        }
        let entry = self.state_entry()?;
        match entry.get_password() {
            Ok(encoded) => decode_state_key(&encoded).map(Zeroizing::new),
            Err(KeyringError::NoEntry) => {
                let generated = XChaCha20Poly1305::generate_key(&mut OsRng);
                let mut key = Zeroizing::new([0u8; 32]);
                key.copy_from_slice(&generated);
                entry
                    .set_password(&encode_state_key(&key))
                    .map_err(|error| {
                        format!("Could not store encrypted state key in the OS credential store: {error}")
                    })?;
                Ok(key)
            }
            Err(error) => Err(format!(
                "Could not read encrypted state key from the OS credential store: {error}"
            )),
        }
    }
}

fn encrypt_state(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = XChaCha20Poly1305::new_from_slice(key)
        .map_err(|_| "Encrypted state key has an invalid length.".to_string())?;
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad: STATE_MAGIC,
            },
        )
        .map_err(|_| "Could not encrypt application state.".to_string())?;
    let mut output = Vec::with_capacity(STATE_MAGIC.len() + STATE_NONCE_LEN + ciphertext.len());
    output.extend_from_slice(STATE_MAGIC);
    output.extend_from_slice(&nonce);
    output.extend_from_slice(&ciphertext);
    Ok(output)
}

fn decrypt_state(key: &[u8; 32], contents: &[u8]) -> Result<Vec<u8>, String> {
    if !contents.starts_with(STATE_MAGIC) || contents.len() < STATE_MAGIC.len() + STATE_NONCE_LEN {
        return Err("Encrypted application state is invalid or truncated.".into());
    }
    let nonce_start = STATE_MAGIC.len();
    let nonce = XNonce::from_slice(&contents[nonce_start..nonce_start + STATE_NONCE_LEN]);
    let ciphertext = &contents[nonce_start + STATE_NONCE_LEN..];
    let cipher = XChaCha20Poly1305::new_from_slice(key)
        .map_err(|_| "Encrypted state key has an invalid length.".to_string())?;
    cipher
        .decrypt(
            nonce,
            Payload {
                msg: ciphertext,
                aad: STATE_MAGIC,
            },
        )
        .map_err(|_| {
            "Could not decrypt application state. The state key or file is invalid.".into()
        })
}

fn encode_state_key(key: &[u8; 32]) -> String {
    key.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn decode_state_key(encoded: &str) -> Result<[u8; 32], String> {
    if encoded.len() != 64 {
        return Err("Encrypted application state key is invalid.".into());
    }
    let mut key = [0u8; 32];
    for (index, byte) in key.iter_mut().enumerate() {
        let start = index * 2;
        *byte = u8::from_str_radix(&encoded[start..start + 2], 16)
            .map_err(|_| "Encrypted application state key is invalid.".to_string())?;
    }
    Ok(key)
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
        let contents = serde_json::to_vec_pretty(profiles)
            .map_err(|error| format!("Could not encode connection profiles: {error}"))?;
        write_private_file(&self.profile_path(), &contents)
            .map_err(|error| format!("Could not save connection profiles: {error}"))
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
