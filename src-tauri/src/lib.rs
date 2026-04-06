mod ai;
mod auth;
mod commands;
mod db;
mod metadata;
mod pool;
mod query;
mod query_history;
mod schema;
mod state;

use ai::store::{AiHistoryStore, AiProviderStore};
use db::store::ConnectionStore;
use metadata::MetadataStore;
use query_history::{QueryHistoryStore, SavedQueryStore};
use state::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_sql::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::SIZE
                        | tauri_plugin_window_state::StateFlags::POSITION
                        | tauri_plugin_window_state::StateFlags::MAXIMIZED
                        | tauri_plugin_window_state::StateFlags::VISIBLE
                        | tauri_plugin_window_state::StateFlags::FULLSCREEN,
                )
                .build(),
        )
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
            let query_history_store = QueryHistoryStore::new(&app_data_dir);
            let saved_query_store = SavedQueryStore::new(&app_data_dir);
            let metadata_store = MetadataStore::new(&app_data_dir);
            app.manage(AppState::new(
                store,
                ai_provider_store,
                ai_history_store,
                query_history_store,
                saved_query_store,
                metadata_store,
            ));

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_connections,
            commands::create_connection,
            commands::update_connection,
            commands::delete_connection,
            commands::test_connection,
            commands::list_databases,
            commands::connect,
            commands::disconnect,
            commands::get_active_connection,
            commands::execute_query,
            commands::list_schemas,
            commands::list_tables,
            commands::list_columns,
            commands::list_indexes,
            commands::list_routines,
            commands::get_view_definition,
            commands::get_routine_definition,
            commands::start_entra_login,
            commands::poll_entra_token,
            commands::list_ai_providers,
            commands::create_ai_provider,
            commands::update_ai_provider,
            commands::delete_ai_provider,
            commands::set_default_ai_provider,
            commands::test_ai_provider,
            commands::list_openrouter_models,
            commands::ai_generate_sql,
            commands::ai_explain_query,
            commands::ai_optimize_query,
            commands::ai_generate_docs,
            commands::ai_format_sql,
            commands::ai_comment_sql,
            commands::list_ai_history,
            commands::save_ai_history_entry,
            commands::clear_ai_history,
            commands::list_query_history,
            commands::save_query_history_entry,
            commands::delete_query_history_entry,
            commands::clear_query_history,
            commands::list_saved_queries,
            commands::create_saved_query,
            commands::update_saved_query,
            commands::delete_saved_query,
            commands::generate_all_metadata,
            commands::generate_single_metadata,
            commands::generate_schema_metadata,
            commands::list_metadata,
            commands::update_metadata,
            commands::delete_all_metadata,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
