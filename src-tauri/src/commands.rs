use std::sync::Arc;
use std::time::Duration;

use sqlx::any::AnyPoolOptions;
use tauri::State;
use tokio::net::TcpStream;
use tokio_util::compat::TokioAsyncWriteCompatExt;

use crate::db::connections::{ConnectionConfig, Driver};
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
pub async fn test_connection(config: ConnectionConfig) -> Result<String, String> {
    match config.driver {
        Driver::Mssql => {
            let tib_config = config.tiberius_config()?;
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
            let mgr = bb8_tiberius::ConnectionManager::build(config.tiberius_config()?)
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
