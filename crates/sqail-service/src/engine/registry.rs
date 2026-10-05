//! Connection pools per profile, rebuilt when a profile changes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use sqail_proto::ConnectionParams;
use uuid::Uuid;

use super::mssql::MssqlDriver;
use super::pg::PgDriver;
use super::sqlite::SqliteDriver;
use super::{Driver, Pool, Result};

/// Everything needed to open connections for one profile.
pub struct DriverSpec<'a> {
    pub params: &'a ConnectionParams,
    pub password: Option<&'a str>,
    /// PEM private key for a PostgreSQL client certificate.
    pub ssl_client_key: Option<&'a str>,
    pub read_only: bool,
    pub sqlite_dirs: &'a [PathBuf],
}

pub fn build_driver(spec: &DriverSpec<'_>) -> Result<Arc<dyn Driver>> {
    Ok(match spec.params {
        ConnectionParams::Postgres(p) => Arc::new(PgDriver::new(
            p,
            spec.password,
            spec.ssl_client_key,
            spec.read_only,
        )?),
        ConnectionParams::Mssql(p) => Arc::new(MssqlDriver::new(p, spec.password, spec.read_only)?),
        ConnectionParams::Sqlite(p) => {
            Arc::new(SqliteDriver::new(p, spec.read_only, spec.sqlite_dirs)?)
        }
    })
}

/// A pool plus the profile version (`updated_at`) it was built from.
type Versioned = (DateTime<Utc>, Arc<Pool>);

#[derive(Default)]
pub struct Registry {
    pools: Mutex<HashMap<Uuid, Versioned>>,
}

impl Registry {
    /// The pool for profile `id` at version `updated_at`, built on first use.
    pub fn pool(
        &self,
        id: Uuid,
        updated_at: DateTime<Utc>,
        size: usize,
        spec: &DriverSpec<'_>,
    ) -> Result<Arc<Pool>> {
        let mut pools = self.pools.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((stamp, pool)) = pools.get(&id)
            && *stamp == updated_at
        {
            return Ok(pool.clone());
        }
        let pool = Pool::new(build_driver(spec)?, size);
        pools.insert(id, (updated_at, pool.clone()));
        Ok(pool)
    }

    /// Drop the pool (idle connections close; checked-out ones close on return).
    pub fn invalidate(&self, id: Uuid) {
        self.pools
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
    }
}
