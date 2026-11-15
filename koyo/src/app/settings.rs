use std::fs;
use std::path::PathBuf;

use koyo_core::models::Engine;
use koyo_core::storage::{EncryptedStateStore, default_config_dir};
use serde::{Deserialize, Serialize};

const STATE_FILE: &str = "state.enc";
const LEGACY_STATE_FILE: &str = "state.json";
pub(crate) const HISTORY_LIMIT: usize = 100;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PersistedState {
    pub(crate) theme: String,
    pub(crate) sidebar_open: bool,
    pub(crate) split: Option<String>,
    pub(crate) split_ratio: f32,
    pub(crate) history: Vec<QueryHistoryEntry>,
    pub(crate) workspace: WorkspaceSnapshot,
}

impl Default for PersistedState {
    fn default() -> Self {
        Self {
            theme: "dark".into(),
            sidebar_open: true,
            split: None,
            split_ratio: 0.5,
            history: Vec::new(),
            workspace: WorkspaceSnapshot::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct QueryHistoryEntry {
    pub(crate) sql: String,
    pub(crate) profile_id: Option<String>,
    pub(crate) profile_name: String,
    pub(crate) engine: Engine,
    pub(crate) success: bool,
    pub(crate) duration_ms: u64,
    pub(crate) at: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct WorkspaceSnapshot {
    pub(crate) split: Option<String>,
    pub(crate) primary: Vec<TabSnapshot>,
    pub(crate) secondary: Vec<TabSnapshot>,
    pub(crate) primary_active: usize,
    pub(crate) secondary_active: usize,
    pub(crate) focus_secondary: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum TabSnapshot {
    Query {
        profile_id: Option<String>,
        sql: String,
    },
    Table {
        profile_id: Option<String>,
        schema: String,
        table: String,
    },
}

impl TabSnapshot {
    pub(crate) fn profile_id(&self) -> Option<&str> {
        match self {
            TabSnapshot::Query { profile_id, .. } | TabSnapshot::Table { profile_id, .. } => {
                profile_id.as_deref()
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SettingsStore {
    encrypted: EncryptedStateStore,
    legacy_path: PathBuf,
}

pub(crate) struct LoadedState {
    pub(crate) state: PersistedState,
    pub(crate) warning: Option<String>,
}

impl SettingsStore {
    pub(crate) fn from_default_dir() -> Self {
        Self::new(default_config_dir())
    }

    pub(crate) fn new(config_dir: impl Into<PathBuf>) -> Self {
        let config_dir = config_dir.into();
        Self {
            encrypted: EncryptedStateStore::new(config_dir.join(STATE_FILE)),
            legacy_path: config_dir.join(LEGACY_STATE_FILE),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_key(config_dir: impl Into<PathBuf>, key: [u8; 32]) -> Self {
        let config_dir = config_dir.into();
        Self {
            encrypted: EncryptedStateStore::with_key(config_dir.join(STATE_FILE), key),
            legacy_path: config_dir.join(LEGACY_STATE_FILE),
        }
    }

    pub(crate) fn load(&self) -> Result<LoadedState, String> {
        if let Some(contents) = self.encrypted.load()? {
            let state = serde_json::from_slice(&contents)
                .map_err(|error| format!("Could not decode application state: {error}"))?;
            let warning = remove_legacy_state(&self.legacy_path);
            return Ok(LoadedState { state, warning });
        }

        if self.legacy_path.exists() {
            let contents = fs::read(&self.legacy_path)
                .map_err(|error| format!("Could not read legacy application state: {error}"))?;
            let state: PersistedState = serde_json::from_slice(&contents)
                .map_err(|error| format!("Could not decode legacy application state: {error}"))?;
            let encoded = serde_json::to_vec_pretty(&state)
                .map_err(|error| format!("Could not encode application state: {error}"))?;
            self.encrypted.save(&encoded)?;
            let warning = remove_legacy_state(&self.legacy_path);
            return Ok(LoadedState { state, warning });
        }

        Ok(LoadedState {
            state: PersistedState::default(),
            warning: None,
        })
    }

    pub(crate) fn save(&self, state: &PersistedState) -> Result<(), String> {
        let contents = serde_json::to_vec_pretty(state)
            .map_err(|error| format!("Could not encode application state: {error}"))?;
        self.encrypted.save(&contents)
    }
}

fn remove_legacy_state(path: &std::path::Path) -> Option<String> {
    if !path.exists() {
        return None;
    }
    fs::remove_file(path).err().map(|error| {
        format!(
            "Encrypted application state is in use, but the old plaintext state.json could not be removed: {error}"
        )
    })
}

pub(crate) fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{PersistedState, QueryHistoryEntry, SettingsStore, TabSnapshot, WorkspaceSnapshot};
    use koyo_core::models::Engine;

    #[test]
    fn persisted_state_round_trips_history_and_tabs() {
        let state = PersistedState {
            theme: "light".into(),
            sidebar_open: false,
            split: Some("horizontal".into()),
            split_ratio: 0.6,
            history: vec![QueryHistoryEntry {
                sql: "select 1".into(),
                profile_id: Some("profile".into()),
                profile_name: "Local".into(),
                engine: Engine::Sqlite,
                success: true,
                duration_ms: 3,
                at: 42,
            }],
            workspace: WorkspaceSnapshot {
                split: Some("horizontal".into()),
                primary: vec![TabSnapshot::Query {
                    profile_id: Some("profile".into()),
                    sql: "select 2".into(),
                }],
                secondary: vec![TabSnapshot::Table {
                    profile_id: None,
                    schema: "main".into(),
                    table: "users".into(),
                }],
                primary_active: 0,
                secondary_active: 0,
                focus_secondary: true,
            },
        };

        let encoded = serde_json::to_vec(&state).expect("encode state");
        let decoded: PersistedState = serde_json::from_slice(&encoded).expect("decode state");

        assert_eq!(decoded.theme, "light");
        assert!(!decoded.sidebar_open);
        assert_eq!(decoded.history.len(), 1);
        assert_eq!(decoded.history[0].sql, "select 1");
        assert_eq!(decoded.workspace.secondary.len(), 1);
        assert!(decoded.workspace.focus_secondary);
    }

    #[test]
    fn missing_state_uses_stable_defaults() {
        let state: PersistedState = serde_json::from_slice(b"{}").expect("defaults");
        assert_eq!(state.theme, "dark");
        assert!(state.sidebar_open);
        assert_eq!(state.split_ratio, 0.5);
    }

    #[test]
    fn settings_store_encrypts_and_reads_state() {
        let directory =
            std::env::temp_dir().join(format!("koyo-settings-encrypted-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let store = SettingsStore::with_key(&directory, [7u8; 32]);
        let mut state = PersistedState::default();
        state.theme = "light".into();
        state.sidebar_open = false;
        state.history.push(QueryHistoryEntry {
            sql: "select secret_value".into(),
            profile_id: None,
            profile_name: "No profile".into(),
            engine: Engine::Postgres,
            success: true,
            duration_ms: 1,
            at: 1,
        });

        store.save(&state).expect("save state");
        let encrypted = std::fs::read(directory.join("state.enc")).expect("read encrypted state");
        assert!(
            !encrypted
                .windows(b"select secret_value".len())
                .any(|window| window == b"select secret_value"),
            "plaintext SQL must not be written to state.enc"
        );

        let loaded = store.load().expect("load state").state;
        assert_eq!(loaded.theme, "light");
        assert!(!loaded.sidebar_open);
        assert_eq!(loaded.history.len(), 1);
        assert_eq!(loaded.history[0].sql, "select secret_value");
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn encrypted_state_rejects_tampering() {
        let directory =
            std::env::temp_dir().join(format!("koyo-settings-tamper-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let store = SettingsStore::with_key(&directory, [9u8; 32]);
        store.save(&PersistedState::default()).expect("save state");
        let path = directory.join("state.enc");
        let mut contents = std::fs::read(&path).expect("read state");
        let last = contents.len() - 1;
        contents[last] ^= 0x01;
        std::fs::write(&path, contents).expect("write tampered state");

        assert!(store.load().is_err());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn leftover_plaintext_state_is_removed_when_encrypted_state_exists() {
        let directory =
            std::env::temp_dir().join(format!("koyo-settings-cleanup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let store = SettingsStore::with_key(&directory, [13u8; 32]);
        store.save(&PersistedState::default()).expect("save state");
        std::fs::write(directory.join("state.json"), b"{}").expect("write stale plaintext");

        store.load().expect("load encrypted state");

        assert!(!directory.join("state.json").exists());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn plaintext_state_is_migrated_and_removed() {
        let directory =
            std::env::temp_dir().join(format!("koyo-settings-migrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("create config dir");
        let mut state = PersistedState::default();
        state.theme = "light".into();
        let plaintext = serde_json::to_vec_pretty(&state).expect("encode legacy state");
        std::fs::write(directory.join("state.json"), plaintext).expect("write legacy state");

        let store = SettingsStore::with_key(&directory, [11u8; 32]);
        let loaded = store.load().expect("migrate state").state;

        assert_eq!(loaded.theme, "light");
        assert!(!directory.join("state.json").exists());
        assert!(directory.join("state.enc").exists());
        let _ = std::fs::remove_dir_all(directory);
    }
}
