use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::input::InputState;
use gpui_kit::component::notification::Notification;
use gpui_kit::prelude::*;
use gpui_kit::*;
use koyo_core::SessionManager;
use koyo_core::database;
use koyo_core::models::{ColumnCatalogEntry, GridChange, SchemaObjectKind, TableFilter, TableSort};
use uuid::Uuid;

use super::notifications::ToastCategory;
use crate::app::*;

mod editing;
mod export;
mod open;
mod page;

/// A dead TCP session can otherwise leave a page load spinning forever.
const TABLE_IO_TIMEOUT: Duration = Duration::from_secs(30);

pub(super) fn table_tab_to_focus(
    candidates: &[(u64, Pane, Option<uuid::Uuid>, &str, &str)],
    pane: Pane,
    profile_id: uuid::Uuid,
    schema: &str,
    table: &str,
) -> Option<u64> {
    candidates
        .iter()
        .find(
            |(_, candidate_pane, profile, candidate_schema, candidate_table)| {
                *candidate_pane == pane
                    && *profile == Some(profile_id)
                    && *candidate_schema == schema
                    && *candidate_table == table
            },
        )
        .map(|(id, _, _, _, _)| *id)
}

pub(super) fn stage_row_delete_change(
    staged: &mut Vec<StagedChange>,
    key: Vec<Option<String>>,
    version: String,
    original: Vec<Option<String>>,
    truncated: Vec<bool>,
) -> bool {
    if staged
        .iter()
        .any(|change| matches!(change, StagedChange::Delete { key: staged_key, .. } if staged_key == &key))
    {
        return false;
    }
    staged.retain(|change| !matches!(change, StagedChange::Update { key: staged_key, .. } if staged_key == &key));
    staged.push(StagedChange::Delete {
        key,
        version,
        original,
        truncated,
    });
    true
}

pub(super) fn stage_row_delete(
    staged: &mut Vec<StagedChange>,
    key: Vec<Option<String>>,
    version: String,
    original: Vec<Option<String>>,
    truncated: Vec<bool>,
) -> bool {
    if let Some(index) = staged
        .iter()
        .position(|change| matches!(change, StagedChange::Delete { key: staged_key, .. } if staged_key == &key))
    {
        staged.remove(index);
        return false;
    }
    stage_row_delete_change(staged, key, version, original, truncated);
    true
}

async fn export_table_csv(
    manager: Arc<SessionManager>,
    session_id: Uuid,
    schema: String,
    table: String,
    first_page: u32,
    last_page: Option<u32>,
    filters: Vec<TableFilter>,
    sort: Option<TableSort>,
    path: PathBuf,
) -> Result<(usize, bool), String> {
    let mut file =
        File::create(&path).map_err(|error| format!("Could not create file: {error}"))?;
    let mut page_number = first_page;
    let mut rows = 0usize;
    let mut truncated = false;
    let mut first_chunk = true;
    loop {
        if last_page.is_some_and(|last| page_number > last) {
            break;
        }
        let offset = page_number
            .saturating_sub(1)
            .checked_mul(PAGE_LIMIT)
            .ok_or_else(|| "Page range is too large.".to_string())?;
        let page = tokio::time::timeout(
            TABLE_IO_TIMEOUT,
            database::load_table_page(
                &manager,
                session_id,
                Uuid::new_v4(),
                schema.clone(),
                table.clone(),
                offset,
                PAGE_LIMIT,
                filters.clone(),
                sort.clone(),
                false,
            ),
        )
        .await
        .unwrap_or_else(|_| {
            Err("CSV export timed out. The database connection may have closed.".into())
        })?;
        let chunk = table_page_as_csv(&page, first_chunk);
        if !chunk.is_empty() {
            file.write_all(chunk.as_bytes())
                .map_err(|error| format!("Could not write file: {error}"))?;
        }
        first_chunk = false;
        rows += page.rows.len();
        truncated |= page
            .rows
            .iter()
            .any(|row| row.truncated.iter().any(|value| *value));
        if !page.has_more || page.rows.is_empty() {
            break;
        }
        page_number = page_number
            .checked_add(1)
            .ok_or_else(|| "Page range is too large.".to_string())?;
    }
    file.flush()
        .map_err(|error| format!("Could not finish file: {error}"))?;
    Ok((rows, truncated))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;

    use koyo_core::SessionManager;
    use koyo_core::database;
    use koyo_core::models::{ConnectionProfile, Engine, SslMode};
    use uuid::Uuid;

    use super::{export_table_csv, stage_row_delete, stage_row_delete_change, table_tab_to_focus};
    use crate::app::{Pane, StagedChange};
    use std::collections::BTreeMap;

    #[test]
    fn table_focus_is_scoped_to_one_pane() {
        let profile = Uuid::new_v4();
        let other = Uuid::new_v4();
        let candidates = [
            (1, Pane::Primary, Some(profile), "main", "users"),
            (2, Pane::Secondary, Some(profile), "main", "users"),
            (3, Pane::Primary, Some(profile), "main", "orders"),
            (4, Pane::Primary, Some(other), "main", "users"),
        ];

        assert_eq!(
            table_tab_to_focus(&candidates, Pane::Primary, profile, "main", "users"),
            Some(1)
        );
        assert_eq!(
            table_tab_to_focus(&candidates, Pane::Secondary, profile, "main", "users"),
            Some(2)
        );
        assert_eq!(
            table_tab_to_focus(&candidates, Pane::Primary, profile, "main", "missing"),
            None
        );
        assert_eq!(
            table_tab_to_focus(&candidates, Pane::Primary, other, "main", "missing"),
            None
        );
    }

    #[tokio::test]
    async fn exports_every_page_with_one_header() {
        let database_path = std::env::temp_dir().join(format!("koyo-export-{}.db", Uuid::new_v4()));
        let csv_path = std::env::temp_dir().join(format!("koyo-export-{}.csv", Uuid::new_v4()));
        let manager = Arc::new(SessionManager::default());
        let profile = ConnectionProfile {
            id: Uuid::new_v4(),
            name: "export test".into(),
            engine: Engine::Sqlite,
            host: String::new(),
            port: 0,
            database: String::new(),
            username: String::new(),
            ssl_mode: SslMode::Disable,
            sqlite_path: database_path.display().to_string(),
        };
        let connection = database::connect(&manager, &profile, "")
            .await
            .expect("connect sqlite");
        database::run_sql(
            &manager,
            connection.session_id,
            Uuid::new_v4(),
            "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT)".into(),
        )
        .await
        .expect("create sqlite table");
        database::run_sql(
            &manager,
            connection.session_id,
            Uuid::new_v4(),
            "WITH RECURSIVE seq(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM seq WHERE n < 250) \
             INSERT INTO people(name) SELECT 'Person ' || n FROM seq"
                .into(),
        )
        .await
        .expect("seed sqlite");

        let (rows, truncated) = export_table_csv(
            manager,
            connection.session_id,
            "main".into(),
            "people".into(),
            1,
            None,
            Vec::new(),
            None,
            csv_path.clone(),
        )
        .await
        .expect("export csv");

        assert_eq!(rows, 250);
        assert!(!truncated);
        let csv = fs::read_to_string(&csv_path).expect("read csv");
        assert_eq!(csv.lines().count(), 251);
        assert_eq!(csv.lines().next(), Some("id,name"));
        assert!(csv.contains("Person 250"));

        let _ = fs::remove_file(database_path);
        let _ = fs::remove_file(csv_path);
    }

    #[test]
    fn delete_supersedes_update_and_toggles() {
        let key = vec![Some("7".to_string())];
        let mut staged = vec![StagedChange::Update {
            key: key.clone(),
            version: "v1".into(),
            values: BTreeMap::from([("name".to_string(), Some("changed".to_string()))]),
            original: vec![Some("7".to_string()), Some("old".to_string())],
            truncated: vec![false, false],
        }];

        assert!(stage_row_delete(
            &mut staged,
            key.clone(),
            "v1".into(),
            vec![Some("7".to_string()), Some("old".to_string())],
            vec![false, false],
        ));
        assert!(matches!(staged.as_slice(), [StagedChange::Delete { .. }]));

        assert!(!stage_row_delete(
            &mut staged,
            key.clone(),
            "v1".into(),
            vec![Some("7".to_string()), Some("old".to_string())],
            vec![false, false],
        ));
        assert!(staged.is_empty());

        assert!(stage_row_delete_change(
            &mut staged,
            key.clone(),
            "v1".into(),
            vec![Some("7".to_string()), Some("old".to_string())],
            vec![false, false],
        ));
        assert!(!stage_row_delete_change(
            &mut staged,
            key,
            "v1".into(),
            vec![Some("7".to_string()), Some("old".to_string())],
            vec![false, false],
        ));
        assert!(matches!(staged.as_slice(), [StagedChange::Delete { .. }]));
    }
}
