use std::fs;
use std::path::PathBuf;

use koyo_core::models::Engine;
use koyo_core::storage::{default_config_dir, write_private_file};
use serde::{Deserialize, Serialize};

const STATE_FILE: &str = "state.json";
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
    path: PathBuf,
}

impl SettingsStore {
    pub(crate) fn from_default_dir() -> Self {
        Self::new(default_config_dir().join(STATE_FILE))
    }

    pub(crate) fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub(crate) fn load(&self) -> PersistedState {
        fs::read(&self.path)
            .ok()
            .and_then(|contents| serde_json::from_slice(&contents).ok())
            .unwrap_or_default()
    }

    pub(crate) fn save(&self, state: &PersistedState) -> Result<(), String> {
        let contents = serde_json::to_vec_pretty(state)
            .map_err(|error| format!("Could not encode application state: {error}"))?;
        write_private_file(&self.path, &contents)
    }
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
    fn settings_store_writes_and_reads_state() {
        let directory = std::env::temp_dir().join(format!("koyo-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let store = SettingsStore::new(directory.join("state.json"));
        let mut state = PersistedState::default();
        state.theme = "light".into();
        state.sidebar_open = false;
        state.history.push(QueryHistoryEntry {
            sql: "select 1".into(),
            profile_id: None,
            profile_name: "No profile".into(),
            engine: Engine::Postgres,
            success: true,
            duration_ms: 1,
            at: 1,
        });

        store.save(&state).expect("save state");
        let loaded = store.load();

        assert_eq!(loaded.theme, "light");
        assert!(!loaded.sidebar_open);
        assert_eq!(loaded.history.len(), 1);
        let _ = std::fs::remove_dir_all(directory);
    }
}
