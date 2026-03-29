mod commands;
mod db;
mod pool;
mod query;
mod schema;
mod state;

use db::store::ConnectionStore;
use state::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_sql::Builder::default().build())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            let app_data_dir = app
                .path()
                .app_data_dir()
                .expect("failed to resolve app data dir");
            eprintln!("App data dir: {}", app_data_dir.display());
            let store =
                ConnectionStore::new(app_data_dir).expect("failed to create connection store");
            app.manage(AppState::new(store));

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_connections,
            commands::create_connection,
            commands::update_connection,
            commands::delete_connection,
            commands::test_connection,
            commands::connect,
            commands::disconnect,
            commands::get_active_connection,
            commands::execute_query,
            commands::list_schemas,
            commands::list_tables,
            commands::list_columns,
            commands::list_indexes,
            commands::list_routines,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
