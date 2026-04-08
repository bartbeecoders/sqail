use std::sync::Arc;
use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use sqlx::mysql::MySqlPoolOptions;
use sqlx::sqlite::SqlitePoolOptions;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::net::TcpStream;
use tokio_util::compat::TokioAsyncWriteCompatExt;

use crate::ai::client;
use crate::ai::provider::{AiHistoryEntry, AiProviderConfig};
use crate::auth::entra;
use crate::db::connections::{ConnectionConfig, Driver, MssqlAuthMethod};
use crate::metadata::{GeneratedMetadata, ObjectMetadata};
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
            Driver::Dbservice => 0,
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
        Driver::Postgres => {
            let url = config.connection_string();
            let pool = PgPoolOptions::new()
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
        Driver::Mysql => {
            let url = config.connection_string();
            let pool = MySqlPoolOptions::new()
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
        Driver::Sqlite => {
            let url = config.connection_string();
            let pool = SqlitePoolOptions::new()
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
        Driver::Dbservice => {
            if config.dbservice_url.is_empty() {
                return Err("DbService URL is required".to_string());
            }
            let client = Arc::new(crate::pool::DbServiceClient {
                base_url: config.dbservice_url.clone(),
                remote_id: config.dbservice_remote_id.clone(),
                jwt: tokio::sync::Mutex::new(String::new()),
                api_key: config.dbservice_api_key.clone(),
                http: crate::dbservice::build_http_client(),
            });
            crate::dbservice::test(&client).await
        }
    }
}

#[tauri::command]
pub async fn list_databases(
    host: String,
    port: u16,
    user: String,
    password: String,
    driver: Driver,
    ssl_mode: String,
) -> Result<Vec<String>, String> {
    match driver {
        Driver::Postgres => {
            let ssl = if ssl_mode.is_empty() { "prefer" } else { &ssl_mode };
            let url = format!(
                "postgres://{}:{}@{}:{}/postgres?sslmode={}",
                user, password, host, port, ssl
            );
            let pool = PgPoolOptions::new()
                .max_connections(1)
                .acquire_timeout(Duration::from_secs(5))
                .connect(&url)
                .await
                .map_err(|e| format!("Connection failed: {e}"))?;
            let rows = sqlx::query_scalar::<_, String>(
                "SELECT datname FROM pg_database \
                 WHERE datistemplate = false AND datallowconn = true \
                 ORDER BY datname",
            )
            .fetch_all(&pool)
            .await
            .map_err(|e| format!("Query failed: {e}"))?;
            pool.close().await;
            Ok(rows)
        }
        Driver::Mysql => {
            let url = format!(
                "mysql://{}:{}@{}:{}/information_schema",
                user, password, host, port
            );
            let pool = MySqlPoolOptions::new()
                .max_connections(1)
                .acquire_timeout(Duration::from_secs(5))
                .connect(&url)
                .await
                .map_err(|e| format!("Connection failed: {e}"))?;
            let rows = sqlx::query_scalar::<_, String>(
                "SELECT schema_name FROM information_schema.schemata \
                 WHERE schema_name NOT IN ('mysql','information_schema','performance_schema','sys') \
                 ORDER BY schema_name",
            )
            .fetch_all(&pool)
            .await
            .map_err(|e| format!("Query failed: {e}"))?;
            pool.close().await;
            Ok(rows)
        }
        Driver::Dbservice => Err("Database listing not supported for DbService driver".to_string()),
        _ => Err("Database listing not supported for this driver".to_string()),
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
        Driver::Postgres => {
            let url = config.connection_string();
            let pool = PgPoolOptions::new()
                .max_connections(5)
                .acquire_timeout(Duration::from_secs(10))
                .connect(&url)
                .await
                .map_err(|e| format!("Connection failed: {e}"))?;
            DbPool::Postgres(Arc::new(pool))
        }
        Driver::Mysql => {
            let url = config.connection_string();
            let pool = MySqlPoolOptions::new()
                .max_connections(5)
                .acquire_timeout(Duration::from_secs(10))
                .connect(&url)
                .await
                .map_err(|e| format!("Connection failed: {e}"))?;
            DbPool::Mysql(Arc::new(pool))
        }
        Driver::Sqlite => {
            let url = config.connection_string();
            let pool = SqlitePoolOptions::new()
                .max_connections(5)
                .acquire_timeout(Duration::from_secs(10))
                .connect(&url)
                .await
                .map_err(|e| format!("Connection failed: {e}"))?;
            DbPool::Sqlite(Arc::new(pool))
        }
        Driver::Dbservice => {
            if config.dbservice_url.is_empty() {
                return Err("DbService URL is required".to_string());
            }
            let client = Arc::new(crate::pool::DbServiceClient {
                base_url: config.dbservice_url.clone(),
                remote_id: config.dbservice_remote_id.clone(),
                jwt: tokio::sync::Mutex::new(String::new()),
                api_key: config.dbservice_api_key.clone(),
                http: crate::dbservice::build_http_client(),
            });
            // Eagerly exchange api_key for JWT so connect() fails fast on bad creds.
            crate::dbservice::ensure_token(&client)
                .await
                .map_err(|e| format!("DbService auth failed: {e}"))?;
            // Open the pool on the remote backend so query/metadata calls can
            // reference it by id.
            crate::dbservice::connect(&client).await?;
            DbPool::DbService(client)
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

#[tauri::command]
pub async fn get_view_definition(
    state: State<'_, AppState>,
    connection_id: String,
    schema_name: String,
    view_name: String,
) -> Result<String, String> {
    let (pool, driver) = get_pool_and_driver(&state, &connection_id).await?;
    schema::get_view_definition(pool, &driver, &schema_name, &view_name).await
}

#[tauri::command]
pub async fn get_routine_definition(
    state: State<'_, AppState>,
    connection_id: String,
    schema_name: String,
    routine_name: String,
    routine_type: String,
) -> Result<String, String> {
    let (pool, driver) = get_pool_and_driver(&state, &connection_id).await?;
    schema::get_routine_definition(pool, &driver, &schema_name, &routine_name, &routine_type).await
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

#[tauri::command]
pub async fn list_openrouter_models(api_key: String, accept_invalid_certs: bool) -> Result<Vec<serde_json::Value>, String> {
    client::list_openrouter_models(&api_key, accept_invalid_certs).await
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

#[tauri::command]
pub async fn ai_format_sql(
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
            "format_sql",
            Some(&driver),
            Some(&schema_context),
        )
        .await;
    });

    Ok(request_id)
}

#[tauri::command]
pub async fn ai_comment_sql(
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
            "comment_sql",
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

// ── Query history commands ────────────────────────────────

use crate::query_history::{QueryHistoryEntry, SavedQuery};

#[tauri::command]
pub async fn list_query_history(
    state: State<'_, AppState>,
) -> Result<Vec<QueryHistoryEntry>, String> {
    let history = state.query_history.lock().await;
    Ok(history.clone())
}

#[tauri::command]
pub async fn save_query_history_entry(
    state: State<'_, AppState>,
    entry: QueryHistoryEntry,
) -> Result<(), String> {
    let mut history = state.query_history.lock().await;
    history.push(entry);
    drop(history);
    state.save_query_history().await
}

#[tauri::command]
pub async fn delete_query_history_entry(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let mut history = state.query_history.lock().await;
    history.retain(|e| e.id != id);
    drop(history);
    state.save_query_history().await
}

#[tauri::command]
pub async fn clear_query_history(
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut history = state.query_history.lock().await;
    history.clear();
    drop(history);
    state.save_query_history().await
}

// ── Saved queries commands ────────────────────────────────

#[tauri::command]
pub async fn list_saved_queries(
    state: State<'_, AppState>,
) -> Result<Vec<SavedQuery>, String> {
    let queries = state.saved_queries.lock().await;
    Ok(queries.clone())
}

#[tauri::command]
pub async fn create_saved_query(
    state: State<'_, AppState>,
    query: SavedQuery,
) -> Result<(), String> {
    let mut queries = state.saved_queries.lock().await;
    queries.push(query);
    drop(queries);
    state.save_saved_queries().await
}

#[tauri::command]
pub async fn update_saved_query(
    state: State<'_, AppState>,
    query: SavedQuery,
) -> Result<(), String> {
    let mut queries = state.saved_queries.lock().await;
    if let Some(existing) = queries.iter_mut().find(|q| q.id == query.id) {
        *existing = query;
    } else {
        return Err("Saved query not found".to_string());
    }
    drop(queries);
    state.save_saved_queries().await
}

#[tauri::command]
pub async fn delete_saved_query(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let mut queries = state.saved_queries.lock().await;
    queries.retain(|q| q.id != id);
    drop(queries);
    state.save_saved_queries().await
}

// ── Metadata commands ─────────────────────────────────────

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct MetadataProgressPayload {
    connection_id: String,
    current: usize,
    total: usize,
    object_name: String,
    status: String, // "generating", "complete", "error"
}

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct MetadataDonePayload {
    connection_id: String,
    total_generated: usize,
}

fn build_object_prompt(
    object_name: &str,
    object_type: &str,
    schema_name: &str,
    columns: &[ColumnInfo],
    indexes: &[IndexInfo],
) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "{}: {}.{} (type: {})",
        object_type, schema_name, object_name, object_type
    ));

    if !columns.is_empty() {
        lines.push("Columns:".to_string());
        for col in columns {
            let mut def = format!("  - {} {}", col.name, col.data_type);
            if col.is_primary_key {
                def.push_str(" PRIMARY KEY");
            }
            if !col.is_nullable {
                def.push_str(" NOT NULL");
            }
            if let Some(ref default) = col.column_default {
                def.push_str(&format!(" DEFAULT {}", default));
            }
            lines.push(def);
        }
    }

    if !indexes.is_empty() {
        lines.push("Indexes:".to_string());
        for idx in indexes {
            let unique = if idx.is_unique { " UNIQUE" } else { "" };
            lines.push(format!(
                "  - {}{} ({})",
                idx.name,
                unique,
                idx.columns.join(", ")
            ));
        }
    }

    lines.join("\n")
}

fn strip_thinking_blocks(text: &str) -> String {
    let mut result = text.to_string();
    // Strip <think>...</think> blocks (used by reasoning models)
    while let Some(start) = result.find("<think>") {
        if let Some(end) = result.find("</think>") {
            result = format!("{}{}", &result[..start], &result[end + "</think>".len()..]);
        } else {
            // Unclosed <think> tag — strip from <think> to end
            result = result[..start].to_string();
            break;
        }
    }
    result
}

fn parse_metadata_response(response: &str) -> GeneratedMetadata {
    // Strip thinking blocks from reasoning models
    let response = strip_thinking_blocks(response);
    let response = response.trim();

    // Strip markdown fences if present
    let cleaned = response
        .strip_prefix("```json")
        .or_else(|| response.strip_prefix("```"))
        .unwrap_or(response);
    let cleaned = cleaned
        .strip_suffix("```")
        .unwrap_or(cleaned)
        .trim();

    if let Ok(parsed) = serde_json::from_str::<GeneratedMetadata>(cleaned) {
        return parsed;
    }

    // Try to extract the first JSON object from the response
    if let Some(start) = cleaned.find('{') {
        if let Some(end) = cleaned.rfind('}') {
            let json_slice = &cleaned[start..=end];
            if let Ok(parsed) = serde_json::from_str::<GeneratedMetadata>(json_slice) {
                return parsed;
            }
        }
    }

    // Fallback: use the raw response as the description
    GeneratedMetadata {
        description: response.to_string(),
        columns: Vec::new(),
        example_usage: String::new(),
        related_objects: Vec::new(),
        dependencies: Vec::new(),
    }
}

#[tauri::command]
pub async fn generate_all_metadata(
    app_handle: AppHandle,
    state: State<'_, AppState>,
    connection_id: String,
) -> Result<String, String> {
    let (pool, driver) = get_pool_and_driver(&state, &connection_id).await?;
    let ai_config = get_default_provider(&state).await?;
    let driver_str = format!("{:?}", driver).to_lowercase();
    let generation_id = uuid::Uuid::new_v4().to_string();
    let conn_id = connection_id.clone();

    // Gather all schemas, tables, and routines upfront
    let schemas = schema::list_schemas(pool.clone(), &driver).await?;

    let mut all_objects: Vec<(String, String, String)> = Vec::new(); // (schema, name, type)

    for s in &schemas {
        let tables = schema::list_tables(pool.clone(), &driver, &s.name).await.unwrap_or_default();
        for t in &tables {
            all_objects.push((s.name.clone(), t.name.clone(), t.table_type.clone()));
        }
        let routines = schema::list_routines(pool.clone(), &driver, &s.name).await.unwrap_or_default();
        for r in &routines {
            all_objects.push((s.name.clone(), r.name.clone(), r.routine_type.clone()));
        }
    }

    let total = all_objects.len();

    tokio::spawn(async move {
        let app_state = app_handle.state::<AppState>();
        let mut generated_count = 0;

        for (i, (schema_name, object_name, object_type)) in all_objects.iter().enumerate() {
            let _ = app_handle.emit(
                "metadata:progress",
                MetadataProgressPayload {
                    connection_id: conn_id.clone(),
                    current: i + 1,
                    total,
                    object_name: object_name.clone(),
                    status: "generating".to_string(),
                },
            );

            // Fetch columns and indexes for tables/views
            let (columns, indexes) = if object_type == "table" || object_type == "view" {
                let cols = schema::list_columns(pool.clone(), &driver, schema_name, object_name)
                    .await
                    .unwrap_or_default();
                let idxs = schema::list_indexes(pool.clone(), &driver, schema_name, object_name)
                    .await
                    .unwrap_or_default();
                (cols, idxs)
            } else {
                (Vec::new(), Vec::new())
            };

            let prompt = build_object_prompt(object_name, object_type, schema_name, &columns, &indexes);

            match client::call_ai_response(
                &ai_config,
                &prompt,
                "generate_metadata",
                Some(&driver_str),
                None,
            )
            .await
            {
                Ok(response) => {
                    let metadata = parse_metadata_response(&response);
                    let now = chrono::Utc::now().to_rfc3339();
                    let entry = ObjectMetadata {
                        id: uuid::Uuid::new_v4().to_string(),
                        connection_id: conn_id.clone(),
                        schema_name: schema_name.clone(),
                        object_name: object_name.clone(),
                        object_type: object_type.clone(),
                        metadata,
                        generated_at: now.clone(),
                        updated_at: now,
                    };

                    // Upsert into state
                    let mut entries = app_state.metadata.lock().await;
                    if let Some(existing) = entries.iter_mut().find(|e| {
                        e.connection_id == conn_id
                            && e.schema_name == *schema_name
                            && e.object_name == *object_name
                    }) {
                        *existing = entry;
                    } else {
                        entries.push(entry);
                    }
                    drop(entries);
                    generated_count += 1;

                    let _ = app_handle.emit(
                        "metadata:progress",
                        MetadataProgressPayload {
                            connection_id: conn_id.clone(),
                            current: i + 1,
                            total,
                            object_name: object_name.clone(),
                            status: "complete".to_string(),
                        },
                    );
                }
                Err(err) => {
                    let _ = app_handle.emit(
                        "metadata:progress",
                        MetadataProgressPayload {
                            connection_id: conn_id.clone(),
                            current: i + 1,
                            total,
                            object_name: object_name.clone(),
                            status: "error".to_string(),
                        },
                    );
                    eprintln!("Metadata generation error for {object_name}: {err}");
                }
            }
        }

        // Save to disk
        if let Err(e) = app_state.save_metadata().await {
            eprintln!("Failed to save metadata: {e}");
        }

        let _ = app_handle.emit(
            "metadata:done",
            MetadataDonePayload {
                connection_id: conn_id,
                total_generated: generated_count,
            },
        );
    });

    Ok(generation_id)
}

#[tauri::command]
pub async fn generate_single_metadata(
    _app_handle: AppHandle,
    state: State<'_, AppState>,
    connection_id: String,
    schema_name: String,
    object_name: String,
    object_type: String,
) -> Result<String, String> {
    let (pool, driver) = get_pool_and_driver(&state, &connection_id).await?;
    let ai_config = get_default_provider(&state).await?;
    let driver_str = format!("{:?}", driver).to_lowercase();

    let (columns, indexes) = if object_type == "table" || object_type == "view" {
        let cols = schema::list_columns(pool.clone(), &driver, &schema_name, &object_name)
            .await
            .unwrap_or_default();
        let idxs = schema::list_indexes(pool.clone(), &driver, &schema_name, &object_name)
            .await
            .unwrap_or_default();
        (cols, idxs)
    } else {
        (Vec::new(), Vec::new())
    };

    let prompt = build_object_prompt(&object_name, &object_type, &schema_name, &columns, &indexes);

    let response = client::call_ai_response(
        &ai_config,
        &prompt,
        "generate_metadata",
        Some(&driver_str),
        None,
    )
    .await?;

    let metadata = parse_metadata_response(&response);
    let now = chrono::Utc::now().to_rfc3339();
    let entry = ObjectMetadata {
        id: uuid::Uuid::new_v4().to_string(),
        connection_id: connection_id.clone(),
        schema_name: schema_name.clone(),
        object_name: object_name.clone(),
        object_type: object_type.clone(),
        metadata,
        generated_at: now.clone(),
        updated_at: now,
    };

    let mut entries = state.metadata.lock().await;
    if let Some(existing) = entries.iter_mut().find(|e| {
        e.connection_id == connection_id
            && e.schema_name == schema_name
            && e.object_name == object_name
    }) {
        *existing = entry.clone();
    } else {
        entries.push(entry.clone());
    }
    drop(entries);
    state.save_metadata().await?;

    Ok(entry.id)
}

#[tauri::command]
pub async fn generate_schema_metadata(
    app_handle: AppHandle,
    state: State<'_, AppState>,
    connection_id: String,
    schema_name: String,
) -> Result<String, String> {
    let (pool, driver) = get_pool_and_driver(&state, &connection_id).await?;
    let ai_config = get_default_provider(&state).await?;
    let driver_str = format!("{:?}", driver).to_lowercase();
    let generation_id = uuid::Uuid::new_v4().to_string();
    let conn_id = connection_id.clone();
    let schema = schema_name.clone();

    // Gather all tables and routines in this schema
    let mut all_objects: Vec<(String, String, String)> = Vec::new();
    let tables = schema::list_tables(pool.clone(), &driver, &schema)
        .await
        .unwrap_or_default();
    for t in &tables {
        all_objects.push((schema.clone(), t.name.clone(), t.table_type.clone()));
    }
    let routines = schema::list_routines(pool.clone(), &driver, &schema)
        .await
        .unwrap_or_default();
    for r in &routines {
        all_objects.push((schema.clone(), r.name.clone(), r.routine_type.clone()));
    }

    let total = all_objects.len();

    tokio::spawn(async move {
        let app_state = app_handle.state::<AppState>();
        let mut generated_count = 0;

        for (i, (schema_name, object_name, object_type)) in all_objects.iter().enumerate() {
            let _ = app_handle.emit(
                "metadata:progress",
                MetadataProgressPayload {
                    connection_id: conn_id.clone(),
                    current: i + 1,
                    total,
                    object_name: object_name.clone(),
                    status: "generating".to_string(),
                },
            );

            let (columns, indexes) = if object_type == "table" || object_type == "view" {
                let cols = schema::list_columns(pool.clone(), &driver, schema_name, object_name)
                    .await
                    .unwrap_or_default();
                let idxs = schema::list_indexes(pool.clone(), &driver, schema_name, object_name)
                    .await
                    .unwrap_or_default();
                (cols, idxs)
            } else {
                (Vec::new(), Vec::new())
            };

            let prompt =
                build_object_prompt(object_name, object_type, schema_name, &columns, &indexes);

            match client::call_ai_response(
                &ai_config,
                &prompt,
                "generate_metadata",
                Some(&driver_str),
                None,
            )
            .await
            {
                Ok(response) => {
                    let metadata = parse_metadata_response(&response);
                    let now = chrono::Utc::now().to_rfc3339();
                    let entry = ObjectMetadata {
                        id: uuid::Uuid::new_v4().to_string(),
                        connection_id: conn_id.clone(),
                        schema_name: schema_name.clone(),
                        object_name: object_name.clone(),
                        object_type: object_type.clone(),
                        metadata,
                        generated_at: now.clone(),
                        updated_at: now,
                    };

                    let mut entries = app_state.metadata.lock().await;
                    if let Some(existing) = entries.iter_mut().find(|e| {
                        e.connection_id == conn_id
                            && e.schema_name == *schema_name
                            && e.object_name == *object_name
                    }) {
                        *existing = entry;
                    } else {
                        entries.push(entry);
                    }
                    drop(entries);
                    generated_count += 1;
                }
                Err(err) => {
                    eprintln!("Metadata generation error for {object_name}: {err}");
                }
            }

            let _ = app_handle.emit(
                "metadata:progress",
                MetadataProgressPayload {
                    connection_id: conn_id.clone(),
                    current: i + 1,
                    total,
                    object_name: object_name.clone(),
                    status: "complete".to_string(),
                },
            );
        }

        if let Err(e) = app_state.save_metadata().await {
            eprintln!("Failed to save metadata: {e}");
        }

        let _ = app_handle.emit(
            "metadata:done",
            MetadataDonePayload {
                connection_id: conn_id,
                total_generated: generated_count,
            },
        );
    });

    Ok(generation_id)
}

#[tauri::command]
pub async fn list_metadata(
    state: State<'_, AppState>,
    connection_id: String,
) -> Result<Vec<ObjectMetadata>, String> {
    let entries = state.metadata.lock().await;
    Ok(entries
        .iter()
        .filter(|e| e.connection_id == connection_id)
        .cloned()
        .collect())
}

#[tauri::command]
pub async fn update_metadata(
    state: State<'_, AppState>,
    entry: ObjectMetadata,
) -> Result<(), String> {
    let mut entries = state.metadata.lock().await;
    let pos = entries
        .iter()
        .position(|e| e.id == entry.id)
        .ok_or_else(|| format!("Metadata entry '{}' not found", entry.id))?;

    let mut updated = entry;
    updated.updated_at = chrono::Utc::now().to_rfc3339();
    entries[pos] = updated;
    drop(entries);
    state.save_metadata().await
}

#[tauri::command]
pub async fn delete_all_metadata(
    state: State<'_, AppState>,
    connection_id: String,
) -> Result<(), String> {
    let mut entries = state.metadata.lock().await;
    entries.retain(|e| e.connection_id != connection_id);
    drop(entries);
    state.save_metadata().await
}
