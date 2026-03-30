mod ai;
mod auth;
mod commands;
mod db;
mod pool;
mod query;
mod schema;
mod state;

use ai::store::{AiHistoryStore, AiProviderStore};
use db::store::ConnectionStore;
use state::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_sql::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_window_state::Builder::new().build())
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
                ConnectionStore::new(app_data_dir.clone()).expect("failed to create connection store");
            let ai_provider_store = AiProviderStore::new(&app_data_dir);
            let ai_history_store = AiHistoryStore::new(&app_data_dir);
            app.manage(AppState::new(store, ai_provider_store, ai_history_store));

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
            commands::start_entra_login,
            commands::poll_entra_token,
            commands::list_ai_providers,
            commands::create_ai_provider,
            commands::update_ai_provider,
            commands::delete_ai_provider,
            commands::set_default_ai_provider,
            commands::test_ai_provider,
            commands::ai_generate_sql,
            commands::ai_explain_query,
            commands::ai_optimize_query,
            commands::ai_generate_docs,
            commands::list_ai_history,
            commands::save_ai_history_entry,
            commands::clear_ai_history,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
