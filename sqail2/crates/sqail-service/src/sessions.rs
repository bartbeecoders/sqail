//! Sessions: a dedicated connection per session so transactions span requests.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use sqail_proto::SessionInfo;
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

use crate::engine::Conn;

pub struct Session {
    pub id: Uuid,
    pub connection_id: Uuid,
    pub owner: Uuid,
    pub created_at: DateTime<Utc>,
    last_used: Mutex<DateTime<Utc>>,
    /// `None` once the connection was lost; the session must be recreated.
    pub conn: Arc<AsyncMutex<Option<Box<dyn Conn>>>>,
    in_transaction: AtomicBool,
}

impl Session {
    pub fn new(connection_id: Uuid, owner: Uuid, conn: Box<dyn Conn>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            connection_id,
            owner,
            created_at: now,
            last_used: Mutex::new(now),
            conn: Arc::new(AsyncMutex::new(Some(conn))),
            in_transaction: AtomicBool::new(false),
        }
    }

    pub fn touch(&self) {
        *self.last_used.lock().unwrap_or_else(|e| e.into_inner()) = Utc::now();
    }

    pub fn last_used(&self) -> DateTime<Utc> {
        *self.last_used.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_in_transaction(&self, v: bool) {
        self.in_transaction.store(v, Ordering::Relaxed);
    }

    pub fn busy(&self) -> bool {
        self.conn.try_lock().is_err()
    }

    pub fn info(&self, idle_timeout_secs: u64) -> SessionInfo {
        SessionInfo {
            id: self.id,
            connection_id: self.connection_id,
            created_at: self.created_at,
            last_used_at: self.last_used(),
            in_transaction: self.in_transaction.load(Ordering::Relaxed),
            busy: self.busy(),
            idle_timeout_secs,
        }
    }
}

#[derive(Default)]
pub struct Sessions {
    map: Mutex<HashMap<Uuid, Arc<Session>>>,
}

impl Sessions {
    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, Arc<Session>>> {
        self.map.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn insert(&self, s: Arc<Session>) {
        self.map().insert(s.id, s);
    }

    pub fn get(&self, id: Uuid) -> Option<Arc<Session>> {
        self.map().get(&id).cloned()
    }

    pub fn remove(&self, id: Uuid) -> Option<Arc<Session>> {
        self.map().remove(&id)
    }

    pub fn owned_by(&self, owner: Uuid) -> Vec<Arc<Session>> {
        self.map()
            .values()
            .filter(|s| s.owner == owner)
            .cloned()
            .collect()
    }

    /// Close sessions idle for longer than `idle_secs`. Dropping the
    /// connection makes the server roll back any open transaction.
    pub fn reap(&self, idle_secs: u64) -> usize {
        let cutoff = Utc::now() - chrono::Duration::seconds(idle_secs as i64);
        let mut map = self.map();
        let before = map.len();
        map.retain(|_, s| s.busy() || s.last_used() > cutoff);
        before - map.len()
    }

    /// Close every session using connection profile `connection_id`.
    pub fn close_for_connection(&self, connection_id: Uuid) {
        self.map().retain(|_, s| s.connection_id != connection_id);
    }
}
