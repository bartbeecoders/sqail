use std::sync::Arc;

/// Unified pool enum wrapping native sqlx pools and tiberius bb8 pool.
pub enum DbPool {
    Postgres(Arc<sqlx::PgPool>),
    Mysql(Arc<sqlx::MySqlPool>),
    Sqlite(Arc<sqlx::SqlitePool>),
    Mssql(Arc<bb8::Pool<bb8_tiberius::ConnectionManager>>),
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
        }
    }
}
