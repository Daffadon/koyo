mod database;
mod models;
mod sqlite;
mod storage;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(database::AppState::default())
        .invoke_handler(tauri::generate_handler![
            storage::list_profiles,
            storage::save_profile,
            storage::delete_profile,
            storage::pick_sqlite_file,
            database::connect_profile,
            database::disconnect,
            database::list_tables,
            database::load_table_page,
            database::apply_grid_changes,
            database::run_sql,
            database::cancel_query,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Koyo");
}
