use std::sync::Arc;

/// Unified pool enum wrapping sqlx AnyPool and tiberius bb8 pool.
pub enum DbPool {
    Sqlx(Arc<sqlx::AnyPool>),
    Mssql(Arc<bb8::Pool<bb8_tiberius::ConnectionManager>>),
}

impl DbPool {
    pub async fn close(&self) {
        match self {
            DbPool::Sqlx(pool) => pool.close().await,
            DbPool::Mssql(_pool) => {
                // bb8 pools don't have an explicit close — connections are dropped
            }
        }
    }
}

impl Clone for DbPool {
    fn clone(&self) -> Self {
        match self {
            DbPool::Sqlx(p) => DbPool::Sqlx(p.clone()),
            DbPool::Mssql(p) => DbPool::Mssql(p.clone()),
        }
    }
}
