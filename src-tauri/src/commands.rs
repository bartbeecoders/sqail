use std::sync::Arc;
use std::time::Duration;

use sqlx::any::AnyPoolOptions;
use tauri::{AppHandle, State};
use tokio::net::TcpStream;
use tokio_util::compat::TokioAsyncWriteCompatExt;

use crate::ai::client;
use crate::ai::provider::{AiHistoryEntry, AiProviderConfig};
use crate::auth::entra;
use crate::db::connections::{ConnectionConfig, Driver, MssqlAuthMethod};
use crate::pool::DbPool;
use crate::query::{self, QueryResponse};
use crate::schema::{self, ColumnInfo, IndexInfo, RoutineInfo, SchemaInfo, TableInfo};
use crate::state::AppState;

#[tauri::command]
pub async fn list_connections(state: State<'_, AppState>) -> Result<Vec<ConnectionConfig>, String> {
    let conns = state.connections.lock().await;
    Ok(conns.clone())
}

#[tauri::command]
pub async fn create_connection(
    state: State<'_, AppState>,
    config: ConnectionConfig,
) -> Result<ConnectionConfig, String> {
    let mut conns = state.connections.lock().await;

    let mut config = config;
    if config.id.is_empty() {
        config.id = uuid::Uuid::new_v4().to_string();
    }

    if config.port == 0 {
        config.port = match config.driver {
            Driver::Postgres => 5432,
            Driver::Mysql => 3306,
            Driver::Sqlite => 0,
            Driver::Mssql => 1433,
        };
    }

    conns.push(config.clone());
    drop(conns);
    state.save().await?;
    Ok(config)
}

#[tauri::command]
pub async fn update_connection(
    state: State<'_, AppState>,
    config: ConnectionConfig,
) -> Result<ConnectionConfig, String> {
    let mut conns = state.connections.lock().await;
    let pos = conns
        .iter()
        .position(|c| c.id == config.id)
        .ok_or_else(|| format!("Connection '{}' not found", config.id))?;

    let mut pools = state.pools.lock().await;
    if let Some(pool) = pools.remove(&config.id) {
        pool.close().await;
    }
    drop(pools);

    conns[pos] = config.clone();
    drop(conns);
    state.save().await?;
    Ok(config)
}

#[tauri::command]
pub async fn delete_connection(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let mut conns = state.connections.lock().await;
    let pos = conns
        .iter()
        .position(|c| c.id == id)
        .ok_or_else(|| format!("Connection '{id}' not found"))?;
    conns.remove(pos);
    drop(conns);

    let mut pools = state.pools.lock().await;
    if let Some(pool) = pools.remove(&id) {
        pool.close().await;
    }
    drop(pools);

    let mut active = state.active_connection_id.lock().await;
    if active.as_deref() == Some(&id) {
        *active = None;
    }
    drop(active);

    state.save().await?;
    Ok(())
}

#[tauri::command]
pub async fn test_connection(
    state: State<'_, AppState>,
    config: ConnectionConfig,
) -> Result<String, String> {
    match config.driver {
        Driver::Mssql => {
            let token = if config.mssql_auth_method == MssqlAuthMethod::EntraId {
                let tokens = state.entra_tokens.lock().await;
                Some(tokens.get(&config.id).cloned().ok_or_else(|| {
                    "Entra ID token not found. Please sign in first.".to_string()
                })?)
            } else {
                None
            };
            let tib_config = config.tiberius_config(token.as_deref())?;
            let tcp = TcpStream::connect(format!("{}:{}", config.host, config.port))
                .await
                .map_err(|e| format!("TCP connection failed: {e}"))?;
            tcp.set_nodelay(true).ok();
            let mut client = tiberius::Client::connect(tib_config, tcp.compat_write())
                .await
                .map_err(|e| format!("MSSQL connection failed: {e}"))?;
            let row = client
                .simple_query("SELECT 1")
                .await
                .map_err(|e| format!("Query failed: {e}"))?
                .into_row()
                .await
                .map_err(|e| format!("Fetch failed: {e}"))?;
            if row.is_some() {
                Ok("Connection successful".to_string())
            } else {
                Err("No result from test query".to_string())
            }
        }
        _ => {
            sqlx::any::install_default_drivers();
            let url = config.connection_string();
            let pool = AnyPoolOptions::new()
                .max_connections(1)
                .acquire_timeout(Duration::from_secs(5))
                .connect(&url)
                .await
                .map_err(|e| format!("Connection failed: {e}"))?;
            let row: (i32,) = sqlx::query_as("SELECT 1")
                .fetch_one(&pool)
                .await
                .map_err(|e| format!("Query failed: {e}"))?;
            pool.close().await;
            if row.0 == 1 {
                Ok("Connection successful".to_string())
            } else {
                Err("Unexpected result from test query".to_string())
            }
        }
    }
}

#[tauri::command]
pub async fn connect(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let conns = state.connections.lock().await;
    let config = conns
        .iter()
        .find(|c| c.id == id)
        .ok_or_else(|| format!("Connection '{id}' not found"))?
        .clone();
    drop(conns);

    let pool = match config.driver {
        Driver::Mssql => {
            let token = if config.mssql_auth_method == MssqlAuthMethod::EntraId {
                let tokens = state.entra_tokens.lock().await;
                Some(tokens.get(&id).cloned().ok_or_else(|| {
                    "Entra ID token not found. Please sign in first.".to_string()
                })?)
            } else {
                None
            };
            let mgr = bb8_tiberius::ConnectionManager::build(config.tiberius_config(token.as_deref())?)
                .map_err(|e| format!("Failed to build MSSQL pool manager: {e}"))?;
            let pool = bb8::Pool::builder()
                .max_size(5)
                .connection_timeout(Duration::from_secs(10))
                .build(mgr)
                .await
                .map_err(|e| format!("MSSQL pool creation failed: {e}"))?;
            DbPool::Mssql(Arc::new(pool))
        }
        _ => {
            sqlx::any::install_default_drivers();
            let url = config.connection_string();
            let pool = AnyPoolOptions::new()
                .max_connections(5)
                .acquire_timeout(Duration::from_secs(10))
                .connect(&url)
                .await
                .map_err(|e| format!("Connection failed: {e}"))?;
            DbPool::Sqlx(Arc::new(pool))
        }
    };

    let mut pools = state.pools.lock().await;
    pools.insert(id.clone(), pool);
    drop(pools);

    let mut active = state.active_connection_id.lock().await;
    *active = Some(id);

    Ok(())
}

#[tauri::command]
pub async fn disconnect(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let mut pools = state.pools.lock().await;
    if let Some(pool) = pools.remove(&id) {
        pool.close().await;
    }
    drop(pools);

    let mut active = state.active_connection_id.lock().await;
    if active.as_deref() == Some(&id) {
        *active = None;
    }

    Ok(())
}

#[tauri::command]
pub async fn get_active_connection(
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let active = state.active_connection_id.lock().await;
    Ok(active.clone())
}

#[tauri::command]
pub async fn execute_query(
    state: State<'_, AppState>,
    connection_id: String,
    sql: String,
) -> Result<QueryResponse, String> {
    let pools = state.pools.lock().await;
    let pool = pools
        .get(&connection_id)
        .ok_or_else(|| format!("No active pool for connection '{connection_id}'"))?
        .clone();
    drop(pools);

    Ok(query::run_query(pool, &sql).await)
}

// ── Schema introspection commands ───────────────────────────

async fn get_pool_and_driver(
    state: &State<'_, AppState>,
    connection_id: &str,
) -> Result<(DbPool, Driver), String> {
    let pools = state.pools.lock().await;
    let pool = pools
        .get(connection_id)
        .ok_or_else(|| format!("No active pool for connection '{connection_id}'"))?
        .clone();
    drop(pools);

    let conns = state.connections.lock().await;
    let driver = conns
        .iter()
        .find(|c| c.id == connection_id)
        .map(|c| c.driver.clone())
        .ok_or_else(|| format!("Connection '{connection_id}' not found"))?;
    drop(conns);

    Ok((pool, driver))
}

#[tauri::command]
pub async fn list_schemas(
    state: State<'_, AppState>,
    connection_id: String,
) -> Result<Vec<SchemaInfo>, String> {
    let (pool, driver) = get_pool_and_driver(&state, &connection_id).await?;
    schema::list_schemas(pool, &driver).await
}

#[tauri::command]
pub async fn list_tables(
    state: State<'_, AppState>,
    connection_id: String,
    schema_name: String,
) -> Result<Vec<TableInfo>, String> {
    let (pool, driver) = get_pool_and_driver(&state, &connection_id).await?;
    schema::list_tables(pool, &driver, &schema_name).await
}

#[tauri::command]
pub async fn list_columns(
    state: State<'_, AppState>,
    connection_id: String,
    schema_name: String,
    table_name: String,
) -> Result<Vec<ColumnInfo>, String> {
    let (pool, driver) = get_pool_and_driver(&state, &connection_id).await?;
    schema::list_columns(pool, &driver, &schema_name, &table_name).await
}

#[tauri::command]
pub async fn list_indexes(
    state: State<'_, AppState>,
    connection_id: String,
    schema_name: String,
    table_name: String,
) -> Result<Vec<IndexInfo>, String> {
    let (pool, driver) = get_pool_and_driver(&state, &connection_id).await?;
    schema::list_indexes(pool, &driver, &schema_name, &table_name).await
}

#[tauri::command]
pub async fn list_routines(
    state: State<'_, AppState>,
    connection_id: String,
    schema_name: String,
) -> Result<Vec<RoutineInfo>, String> {
    let (pool, driver) = get_pool_and_driver(&state, &connection_id).await?;
    schema::list_routines(pool, &driver, &schema_name).await
}

// ── Entra ID auth commands ────────────────────────────────

#[tauri::command]
pub async fn start_entra_login(
    tenant_id: String,
    azure_client_id: String,
) -> Result<entra::DeviceCodeResponse, String> {
    entra::start_device_code_flow(&tenant_id, &azure_client_id).await
}

#[tauri::command]
pub async fn poll_entra_token(
    state: State<'_, AppState>,
    connection_id: String,
    tenant_id: String,
    azure_client_id: String,
    device_code: String,
) -> Result<(), String> {
    let token_resp =
        entra::poll_for_token(&tenant_id, &azure_client_id, &device_code).await?;
    let mut tokens = state.entra_tokens.lock().await;
    tokens.insert(connection_id, token_resp.access_token);
    Ok(())
}

// ── AI provider commands ───────────────────────────────────

#[tauri::command]
pub async fn list_ai_providers(
    state: State<'_, AppState>,
) -> Result<Vec<AiProviderConfig>, String> {
    let providers = state.ai_providers.lock().await;
    Ok(providers.clone())
}

#[tauri::command]
pub async fn create_ai_provider(
    state: State<'_, AppState>,
    config: AiProviderConfig,
) -> Result<AiProviderConfig, String> {
    let mut providers = state.ai_providers.lock().await;

    let mut config = config;
    if config.id.is_empty() {
        config.id = uuid::Uuid::new_v4().to_string();
    }

    // If this is the first provider or marked as default, ensure only one default
    if config.is_default || providers.is_empty() {
        for p in providers.iter_mut() {
            p.is_default = false;
        }
        config.is_default = true;
    }

    providers.push(config.clone());
    drop(providers);
    state.save_ai_providers().await?;
    Ok(config)
}

#[tauri::command]
pub async fn update_ai_provider(
    state: State<'_, AppState>,
    config: AiProviderConfig,
) -> Result<AiProviderConfig, String> {
    let mut providers = state.ai_providers.lock().await;
    let pos = providers
        .iter()
        .position(|p| p.id == config.id)
        .ok_or_else(|| format!("AI provider '{}' not found", config.id))?;

    if config.is_default {
        for p in providers.iter_mut() {
            p.is_default = false;
        }
    }

    providers[pos] = config.clone();
    drop(providers);
    state.save_ai_providers().await?;
    Ok(config)
}

#[tauri::command]
pub async fn delete_ai_provider(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let mut providers = state.ai_providers.lock().await;
    let pos = providers
        .iter()
        .position(|p| p.id == id)
        .ok_or_else(|| format!("AI provider '{id}' not found"))?;
    let was_default = providers[pos].is_default;
    providers.remove(pos);

    // If we removed the default, make the first remaining one the default
    if was_default && !providers.is_empty() {
        providers[0].is_default = true;
    }

    drop(providers);
    state.save_ai_providers().await?;
    Ok(())
}

#[tauri::command]
pub async fn set_default_ai_provider(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let mut providers = state.ai_providers.lock().await;
    let mut found = false;
    for p in providers.iter_mut() {
        p.is_default = p.id == id;
        if p.id == id {
            found = true;
        }
    }
    if !found {
        return Err(format!("AI provider '{id}' not found"));
    }
    drop(providers);
    state.save_ai_providers().await?;
    Ok(())
}

#[tauri::command]
pub async fn test_ai_provider(config: AiProviderConfig) -> Result<String, String> {
    client::test_ai_provider(&config).await
}

// ── AI flow commands ───────────────────────────────────────

async fn get_default_provider(state: &State<'_, AppState>) -> Result<AiProviderConfig, String> {
    let providers = state.ai_providers.lock().await;
    providers
        .iter()
        .find(|p| p.is_default)
        .or_else(|| providers.first())
        .cloned()
        .ok_or_else(|| "No AI provider configured. Add one in AI settings.".to_string())
}

#[tauri::command]
pub async fn ai_generate_sql(
    app_handle: AppHandle,
    state: State<'_, AppState>,
    prompt: String,
    schema_context: String,
    driver: String,
) -> Result<String, String> {
    let config = get_default_provider(&state).await?;
    let request_id = uuid::Uuid::new_v4().to_string();
    let rid = request_id.clone();

    tokio::spawn(async move {
        client::stream_ai_response(
            app_handle,
            rid,
            &config,
            &prompt,
            "generate_sql",
            Some(&driver),
            Some(&schema_context),
        )
        .await;
    });

    Ok(request_id)
}

#[tauri::command]
pub async fn ai_explain_query(
    app_handle: AppHandle,
    state: State<'_, AppState>,
    sql: String,
    schema_context: String,
    driver: String,
) -> Result<String, String> {
    let config = get_default_provider(&state).await?;
    let request_id = uuid::Uuid::new_v4().to_string();
    let rid = request_id.clone();

    tokio::spawn(async move {
        client::stream_ai_response(
            app_handle,
            rid,
            &config,
            &sql,
            "explain",
            Some(&driver),
            Some(&schema_context),
        )
        .await;
    });

    Ok(request_id)
}

#[tauri::command]
pub async fn ai_optimize_query(
    app_handle: AppHandle,
    state: State<'_, AppState>,
    sql: String,
    schema_context: String,
    driver: String,
) -> Result<String, String> {
    let config = get_default_provider(&state).await?;
    let request_id = uuid::Uuid::new_v4().to_string();
    let rid = request_id.clone();

    tokio::spawn(async move {
        client::stream_ai_response(
            app_handle,
            rid,
            &config,
            &sql,
            "optimize",
            Some(&driver),
            Some(&schema_context),
        )
        .await;
    });

    Ok(request_id)
}

#[tauri::command]
pub async fn ai_generate_docs(
    app_handle: AppHandle,
    state: State<'_, AppState>,
    schema_context: String,
    driver: String,
) -> Result<String, String> {
    let config = get_default_provider(&state).await?;
    let request_id = uuid::Uuid::new_v4().to_string();
    let rid = request_id.clone();

    tokio::spawn(async move {
        client::stream_ai_response(
            app_handle,
            rid,
            &config,
            "Generate documentation for the following database schema.",
            "document",
            Some(&driver),
            Some(&schema_context),
        )
        .await;
    });

    Ok(request_id)
}

// ── AI history commands ────────────────────────────────────

#[tauri::command]
pub async fn list_ai_history(
    state: State<'_, AppState>,
) -> Result<Vec<AiHistoryEntry>, String> {
    let history = state.ai_history.lock().await;
    Ok(history.clone())
}

#[tauri::command]
pub async fn save_ai_history_entry(
    state: State<'_, AppState>,
    entry: AiHistoryEntry,
) -> Result<(), String> {
    let mut history = state.ai_history.lock().await;
    history.push(entry);
    drop(history);
    state.save_ai_history().await
}

#[tauri::command]
pub async fn clear_ai_history(
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut history = state.ai_history.lock().await;
    history.clear();
    drop(history);
    state.save_ai_history().await
}
