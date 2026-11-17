use crate::models::{ConnectionProfile, ProfileInput, SslMode};
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use keyring::v1::{Entry, Error as KeyringError};
use std::fs;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use uuid::Uuid;
use zeroize::Zeroizing;

pub const APP_IDENTIFIER: &str = "com.koyo.db";
const SERVICE_NAME: &str = "com.koyo.db";
const PROFILE_FILE: &str = "profiles.enc";
const LEGACY_PROFILE_FILE: &str = "profiles.json";
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
    let write_result = file.write_all(contents).and_then(|_| file.sync_all());
    drop(file);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary_path);
        return Err(format!("Could not write {file_name}: {error}"));
    }
    if let Err(error) = fs::rename(&temporary_path, path) {
        let _ = fs::remove_file(&temporary_path);
        return Err(format!("Could not replace {file_name}: {error}"));
    }
    #[cfg(unix)]
    fs::File::open(directory)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("Could not flush the application config directory: {error}"))?;
    Ok(())
}

/// Encrypted application state. The 32-byte data key lives in the operating
/// system credential store; the file itself is XChaCha20-Poly1305 ciphertext.
#[derive(Clone)]
pub struct EncryptedStateStore {
    path: PathBuf,
    cached_key: Arc<Mutex<Option<Zeroizing<[u8; 32]>>>>,
}

impl std::fmt::Debug for EncryptedStateStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncryptedStateStore")
            .field("path", &self.path)
            .field(
                "has_cached_key",
                &self.cached_key.lock().is_ok_and(|key| key.is_some()),
            )
            .finish()
    }
}

impl EncryptedStateStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            cached_key: Arc::new(Mutex::new(None)),
        }
    }

    /// Inject a key for tests or embedders that manage their own key material.
    pub fn with_key(path: impl Into<PathBuf>, key: [u8; 32]) -> Self {
        Self {
            path: path.into(),
            cached_key: Arc::new(Mutex::new(Some(Zeroizing::new(key)))),
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

    fn cached_key(&self) -> Result<Option<Zeroizing<[u8; 32]>>, String> {
        let guard = self
            .cached_key
            .lock()
            .map_err(|_| "Encrypted state key cache is corrupted.".to_string())?;
        Ok(guard.as_ref().map(|key| Zeroizing::new(**key)))
    }

    fn cache_key(&self, key: [u8; 32]) -> Result<Zeroizing<[u8; 32]>, String> {
        let mut guard = self
            .cached_key
            .lock()
            .map_err(|_| "Encrypted state key cache is corrupted.".to_string())?;
        *guard = Some(Zeroizing::new(key));
        Ok(Zeroizing::new(key))
    }

    fn existing_state_key(&self) -> Result<Zeroizing<[u8; 32]>, String> {
        if let Some(key) = self.cached_key()? {
            return Ok(key);
        }
        let entry = self.state_entry()?;
        match entry.get_password() {
            Ok(encoded) => {
                let key = decode_state_key(&encoded)?;
                self.cache_key(key)
            }
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
        if let Some(key) = self.cached_key()? {
            return Ok(key);
        }
        let entry = self.state_entry()?;
        match entry.get_password() {
            Ok(encoded) => {
                let key = decode_state_key(&encoded)?;
                self.cache_key(key)
            }
            Err(KeyringError::NoEntry) => {
                let generated = XChaCha20Poly1305::generate_key(&mut OsRng);
                let mut key = Zeroizing::new([0u8; 32]);
                key.copy_from_slice(&generated);
                entry
                    .set_password(&encode_state_key(&key))
                    .map_err(|error| {
                        format!("Could not store encrypted state key in the OS credential store: {error}")
                    })?;
                self.cache_key(*key)
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
    if encoded.len() != 64 || !encoded.is_ascii() {
        return Err("Encrypted application state key is invalid.".into());
    }
    let bytes = encoded.as_bytes();
    let mut key = [0u8; 32];
    for (index, byte) in key.iter_mut().enumerate() {
        let high = hex_nibble(bytes[index * 2])
            .ok_or_else(|| "Encrypted application state key is invalid.".to_string())?;
        let low = hex_nibble(bytes[index * 2 + 1])
            .ok_or_else(|| "Encrypted application state key is invalid.".to_string())?;
        *byte = (high << 4) | low;
    }
    Ok(key)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn decode_profiles(contents: &[u8]) -> Result<Vec<ConnectionProfile>, String> {
    serde_json::from_slice(contents)
        .map_err(|error| format!("Could not decode saved connection profiles: {error}"))
}

fn remove_legacy_profiles(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    fs::remove_file(path)
        .map_err(|error| format!("Encrypted profiles are active, but the old plaintext profiles.json could not be removed: {error}"))
}

#[derive(Clone, Debug)]
pub struct ProfileStore {
    encrypted: EncryptedStateStore,
    legacy_path: PathBuf,
    migration_warning: Arc<Mutex<Option<String>>>,
}

impl ProfileStore {
    pub fn new(config_dir: impl Into<PathBuf>) -> Self {
        let config_dir = config_dir.into();
        Self {
            encrypted: EncryptedStateStore::new(config_dir.join(PROFILE_FILE)),
            legacy_path: config_dir.join(LEGACY_PROFILE_FILE),
            migration_warning: Arc::new(Mutex::new(None)),
        }
    }

    pub fn from_default_dir() -> Self {
        Self::new(default_config_dir())
    }

    #[cfg(test)]
    pub fn with_key(config_dir: impl Into<PathBuf>, key: [u8; 32]) -> Self {
        let config_dir = config_dir.into();
        Self {
            encrypted: EncryptedStateStore::with_key(config_dir.join(PROFILE_FILE), key),
            legacy_path: config_dir.join(LEGACY_PROFILE_FILE),
            migration_warning: Arc::new(Mutex::new(None)),
        }
    }

    /// Consume the one-time warning raised when a plaintext profile file was
    /// imported. The old profile IDs were replaced, so passwords saved under
    /// the old IDs remain in the credential store until the user removes them.
    pub fn take_migration_warning(&self) -> Option<String> {
        self.migration_warning
            .lock()
            .ok()
            .and_then(|mut warning| warning.take())
    }

    pub fn load_profiles(&self) -> Result<Vec<ConnectionProfile>, String> {
        if let Some(contents) = self
            .encrypted
            .load()
            .map_err(|error| format!("Could not read saved connection profiles: {error}"))?
        {
            let profiles = decode_profiles(&contents)?;
            remove_legacy_profiles(&self.legacy_path)?;
            return Ok(profiles);
        }

        if self.legacy_path.exists() {
            let contents = fs::read(&self.legacy_path)
                .map_err(|error| format!("Could not read saved connection profiles: {error}"))?;
            let mut profiles = decode_profiles(
                contents
                    .strip_prefix(&[0xEF, 0xBB, 0xBF])
                    .unwrap_or(&contents),
            )?;
            // Legacy files did not bind IDs to targets. Give every imported
            // profile a fresh ID so planted UUIDs cannot reuse keyring entries.
            for profile in &mut profiles {
                profile.id = Uuid::new_v4();
            }
            let encoded = serde_json::to_vec_pretty(&profiles)
                .map_err(|error| format!("Could not encode connection profiles: {error}"))?;
            self.encrypted.save(&encoded).map_err(|error| {
                format!("Could not save encrypted connection profiles: {error}")
            })?;
            remove_legacy_profiles(&self.legacy_path)?;
            if let Ok(mut warning) = self.migration_warning.lock() {
                *warning = Some(
                    "Plaintext connection profiles were imported and encrypted. Old passwords remain in the OS credential store under the previous profile IDs; remove them manually if needed."
                        .into(),
                );
            }
            return Ok(profiles);
        }

        Ok(Vec::new())
    }

    fn write_profiles(&self, profiles: &[ConnectionProfile]) -> Result<(), String> {
        let contents = serde_json::to_vec_pretty(profiles)
            .map_err(|error| format!("Could not encode connection profiles: {error}"))?;
        self.encrypted
            .save(&contents)
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
        let old_profiles = profiles.clone();
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
        // Mutate the credential before a target change. A crash can then
        // leave a wrong or missing secret, never an old secret on a new host.
        if target_changed {
            if let Some(password) = password_to_save.as_deref() {
                let entry = credential_entry(id)?;
                let previous_secret = match entry.get_password() {
                    Ok(value) => Some(value),
                    Err(KeyringError::NoEntry) => None,
                    Err(error) => {
                        return Err(format!(
                            "Could not read the previous saved password: {error}"
                        ));
                    }
                };
                entry.set_password(password).map_err(|error| {
                    format!(
                        "Could not save password in the operating-system credential store: {error}"
                    )
                })?;
                if let Err(error) = self.write_profiles(&profiles) {
                    match previous_secret.as_deref() {
                        Some(previous) => {
                            let _ = entry.set_password(previous);
                        }
                        None => {
                            let _ = entry.delete_credential();
                        }
                    }
                    return Err(error);
                }
            } else {
                let entry = must_clear_credential
                    .then(|| credential_entry(id))
                    .transpose()?;
                self.write_profiles(&profiles)?;
                if let Some(entry) = entry
                    && let Err(error) = entry.delete_credential()
                {
                    let _ = self.write_profiles(&old_profiles);
                    return Err(format!(
                        "Could not remove the saved password for the changed connection target: {error}"
                    ));
                }
            }
        } else {
            self.write_profiles(&profiles)?;
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
        if engine.is_sqlite() {
            return self.write_profiles(&profiles);
        }
        let entry = credential_entry(id)?;
        self.write_profiles(&profiles)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Engine;

    fn profile() -> ConnectionProfile {
        ConnectionProfile {
            id: Uuid::new_v4(),
            name: "Local".into(),
            engine: Engine::Postgres,
            host: "localhost".into(),
            port: 5432,
            database: "app".into(),
            username: "app_user".into(),
            ssl_mode: SslMode::VerifyFull,
            sqlite_path: String::new(),
        }
    }

    fn temp_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("koyo-profiles-{label}-{}", std::process::id()))
    }

    #[test]
    fn profiles_are_encrypted_and_migrated() {
        let directory = temp_dir("encrypt");
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).expect("create config dir");
        let profile = profile();
        let plaintext = serde_json::to_vec_pretty(&[profile.clone()]).expect("encode profiles");
        fs::write(directory.join(LEGACY_PROFILE_FILE), plaintext).expect("write legacy profiles");

        let store = ProfileStore::with_key(&directory, [21u8; 32]);
        let loaded = store.load_profiles().expect("migrate profiles");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].username, "app_user");
        assert_ne!(loaded[0].id, profile.id, "legacy IDs must be regenerated");
        assert!(!directory.join(LEGACY_PROFILE_FILE).exists());
        assert!(store.take_migration_warning().is_some());
        assert!(store.take_migration_warning().is_none());
        let encrypted = fs::read(directory.join(PROFILE_FILE)).expect("read encrypted profiles");
        assert!(
            !encrypted
                .windows(b"app_user".len())
                .any(|window| window == b"app_user"),
            "plaintext profile data must not be written to profiles.enc"
        );
        let _ = fs::remove_dir_all(directory);
    }
}
