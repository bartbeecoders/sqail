use std::sync::Arc;

/// Handle for a DbService remote backend: cached HTTP client + auth state.
#[derive(Debug)]
pub struct DbServiceClient {
    pub base_url: String,
    pub remote_id: String,
    pub jwt: tokio::sync::Mutex<String>,
    pub api_key: String,
    pub http: reqwest::Client,
}

/// Unified pool enum wrapping native sqlx pools, tiberius bb8 pool, and DbService HTTP client.
pub enum DbPool {
    Postgres(Arc<sqlx::PgPool>),
    Mysql(Arc<sqlx::MySqlPool>),
    Sqlite(Arc<sqlx::SqlitePool>),
    Mssql(Arc<bb8::Pool<bb8_tiberius::ConnectionManager>>),
    DbService(Arc<DbServiceClient>),
}

impl DbPool {
    pub async fn close(&self) {
        match self {
            DbPool::Postgres(pool) => pool.close().await,
            DbPool::Mysql(pool) => pool.close().await,
            DbPool::Sqlite(pool) => pool.close().await,
            DbPool::Mssql(_pool) => {
                // bb8 pools don't have an explicit close — connections are dropped
            }
            DbPool::DbService(_) => {
                // Stateless HTTP — nothing to close
            }
        }
    }
}

impl Clone for DbPool {
    fn clone(&self) -> Self {
        match self {
            DbPool::Postgres(p) => DbPool::Postgres(p.clone()),
            DbPool::Mysql(p) => DbPool::Mysql(p.clone()),
            DbPool::Sqlite(p) => DbPool::Sqlite(p.clone()),
            DbPool::Mssql(p) => DbPool::Mssql(p.clone()),
            DbPool::DbService(p) => DbPool::DbService(p.clone()),
        }
    }
}
