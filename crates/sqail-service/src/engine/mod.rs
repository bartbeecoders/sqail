//! Database engines behind one interface.
//!
//! A [`Driver`] is built from a connection profile and opens [`Conn`]s. A conn
//! executes scripts, streaming [`QueryEvent`]s into an [`EventSink`]. [`Pool`]
//! keeps idle conns per profile; sessions hold a conn of their own.

pub mod explain;
pub mod introspect;
pub mod mssql;
pub mod pg;
pub mod registry;
pub mod split;
pub mod sqlite;
mod tls_client;
mod value;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use sqail_proto::{Column, Engine, Param, QueryEvent};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};

pub type Result<T, E = DbError> = std::result::Result<T, E>;

/// Rows per `rows` event.
pub const ROW_BATCH: usize = 500;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// An error reported by the database server; safe and useful to show.
    #[error("{message}")]
    Database {
        code: Option<String>,
        message: String,
    },
    #[error("cannot connect: {0}")]
    Connect(String),
    #[error("query cancelled")]
    Cancelled,
    #[error("query timed out")]
    Timeout,
    #[error("client disconnected")]
    ClientGone,
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Unsupported(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl DbError {
    pub fn code(&self) -> &'static str {
        match self {
            DbError::Database { .. } => "db_error",
            DbError::Connect(_) => "connect_failed",
            DbError::Cancelled => "cancelled",
            DbError::Timeout => "timeout",
            DbError::ClientGone => "client_gone",
            DbError::Invalid(_) => "invalid_request",
            DbError::Unsupported(_) => "unsupported",
            DbError::Internal(_) => "internal",
        }
    }

    pub fn to_event(&self) -> QueryEvent {
        QueryEvent::Error {
            code: self.code().into(),
            message: self.to_string(),
            db_code: match self {
                DbError::Database { code, .. } => code.clone(),
                _ => None,
            },
        }
    }
}

/// Where a running query sends its events: an HTTP response stream, or an
/// in-memory buffer for internal catalog queries.
pub enum EventSink {
    Channel(mpsc::Sender<QueryEvent>),
    Collect(Mutex<Vec<QueryEvent>>),
}

impl EventSink {
    pub async fn send(&self, ev: QueryEvent) -> Result<()> {
        match self {
            EventSink::Channel(tx) => tx.send(ev).await.map_err(|_| DbError::ClientGone),
            EventSink::Collect(buf) => {
                buf.lock().unwrap_or_else(|e| e.into_inner()).push(ev);
                Ok(())
            }
        }
    }

    /// For drivers that run on a blocking thread (SQLite).
    pub fn send_blocking(&self, ev: QueryEvent) -> Result<()> {
        match self {
            EventSink::Channel(tx) => tx.blocking_send(ev).map_err(|_| DbError::ClientGone),
            EventSink::Collect(buf) => {
                buf.lock().unwrap_or_else(|e| e.into_inner()).push(ev);
                Ok(())
            }
        }
    }
}

/// What to run. `max_rows` applies per result set.
pub struct ExecRequest {
    pub sql: String,
    pub params: Vec<Param>,
    pub max_rows: u64,
}

/// Builds the result-set events for a driver: numbering, batching, truncation.
pub struct ResultWriter<'a> {
    sink: &'a EventSink,
    max_rows: u64,
    next_index: u32,
    current: Option<OpenResult>,
}

struct OpenResult {
    index: u32,
    count: u64,
    truncated: bool,
    batch: Vec<Vec<serde_json::Value>>,
}

impl<'a> ResultWriter<'a> {
    pub fn new(sink: &'a EventSink, max_rows: u64) -> Self {
        Self {
            sink,
            max_rows,
            next_index: 0,
            current: None,
        }
    }

    pub async fn start(&mut self, columns: Vec<Column>) -> Result<()> {
        self.end().await?;
        let index = self.next_index;
        self.next_index += 1;
        self.current = Some(OpenResult {
            index,
            count: 0,
            truncated: false,
            batch: Vec::with_capacity(ROW_BATCH),
        });
        self.sink
            .send(QueryEvent::ResultStart { index, columns })
            .await
    }

    pub async fn row(&mut self, row: Vec<serde_json::Value>) -> Result<()> {
        let Some(cur) = self.current.as_mut() else {
            return Err(DbError::Internal("row without result set".into()));
        };
        if cur.count >= self.max_rows {
            cur.truncated = true;
            return Ok(());
        }
        cur.count += 1;
        cur.batch.push(row);
        if cur.batch.len() >= ROW_BATCH {
            let rows = std::mem::replace(&mut cur.batch, Vec::with_capacity(ROW_BATCH));
            let index = cur.index;
            self.sink.send(QueryEvent::Rows { index, rows }).await?;
        }
        Ok(())
    }

    pub async fn end(&mut self) -> Result<()> {
        if let Some(cur) = self.current.take() {
            if !cur.batch.is_empty() {
                self.sink
                    .send(QueryEvent::Rows {
                        index: cur.index,
                        rows: cur.batch,
                    })
                    .await?;
            }
            self.sink
                .send(QueryEvent::ResultEnd {
                    index: cur.index,
                    row_count: cur.count,
                    truncated: cur.truncated,
                })
                .await?;
        }
        Ok(())
    }

    pub fn is_open(&self) -> bool {
        self.current.is_some()
    }

    pub fn results_seen(&self) -> u32 {
        self.next_index
    }

    pub async fn rows_affected(&mut self, count: u64) -> Result<()> {
        self.end().await?;
        self.sink.send(QueryEvent::RowsAffected { count }).await
    }

    pub async fn message(&mut self, severity: &str, text: impl Into<String>) -> Result<()> {
        self.sink
            .send(QueryEvent::Message {
                severity: severity.into(),
                text: text.into(),
            })
            .await
    }
}

/// Stops a query running on another task. Must be safe to call at any time,
/// including after the query finished.
#[async_trait]
pub trait Cancel: Send + Sync {
    /// Returns whether the request reached the server; if not, the caller
    /// should not wait for the query to wind down but drop the connection.
    async fn cancel(&self) -> bool;
}

pub type Canceller = Arc<dyn Cancel>;

/// One live database connection.
#[async_trait]
pub trait Conn: Send {
    /// Run a script, streaming events. Does not send `started`/`done`.
    async fn execute(&mut self, req: &ExecRequest, sink: &EventSink) -> Result<()>;
    fn canceller(&self) -> Canceller;
    /// Whether an explicit transaction is open (and must be ended by the user).
    async fn in_transaction(&mut self) -> Result<bool>;
    /// Cheap round trip to check the connection is still usable.
    async fn ping(&mut self) -> Result<()>;
    /// Human-readable server version.
    async fn server_version(&mut self) -> Result<String>;
    fn engine(&self) -> Engine;
}

/// Opens connections for one connection profile.
#[async_trait]
pub trait Driver: Send + Sync {
    async fn connect(&self) -> Result<Box<dyn Conn>>;
    fn engine(&self) -> Engine;
}

/// Run `sql` and return every result set (columns and rows).
pub async fn fetch_results(
    conn: &mut dyn Conn,
    sql: &str,
) -> Result<Vec<(Vec<Column>, Vec<Vec<serde_json::Value>>)>> {
    let sink = EventSink::Collect(Mutex::new(Vec::new()));
    let req = ExecRequest {
        sql: sql.to_string(),
        params: Vec::new(),
        max_rows: u64::MAX,
    };
    conn.execute(&req, &sink).await?;
    let EventSink::Collect(buf) = sink else {
        unreachable!()
    };
    let mut out: Vec<(Vec<Column>, Vec<Vec<serde_json::Value>>)> = Vec::new();
    for ev in buf.into_inner().unwrap_or_else(|e| e.into_inner()) {
        match ev {
            QueryEvent::ResultStart { columns, .. } => out.push((columns, Vec::new())),
            QueryEvent::Rows { rows, .. } => {
                if let Some(last) = out.last_mut() {
                    last.1.extend(rows);
                }
            }
            QueryEvent::Error {
                message, db_code, ..
            } => {
                return Err(DbError::Database {
                    code: db_code,
                    message,
                });
            }
            _ => {}
        }
    }
    Ok(out)
}

/// Run `sql` and return the rows of its first result set (catalog queries).
pub async fn fetch_rows(
    conn: &mut dyn Conn,
    sql: &str,
    params: Vec<Param>,
) -> Result<Vec<Vec<serde_json::Value>>> {
    let sink = EventSink::Collect(Mutex::new(Vec::new()));
    let req = ExecRequest {
        sql: sql.to_string(),
        params,
        max_rows: u64::MAX,
    };
    conn.execute(&req, &sink).await?;
    let EventSink::Collect(buf) = sink else {
        unreachable!()
    };
    let mut out = Vec::new();
    for ev in buf.into_inner().unwrap_or_else(|e| e.into_inner()) {
        match ev {
            QueryEvent::Rows { index: 0, rows } => out.extend(rows),
            QueryEvent::Error { message, .. } => return Err(DbError::Internal(message)),
            _ => {}
        }
    }
    Ok(out)
}

// ------------------------------------------------------------------ pool --

/// Idle connections for one profile, with a cap on concurrently open ones.
pub struct Pool {
    driver: Arc<dyn Driver>,
    idle: Mutex<Vec<(Box<dyn Conn>, Instant)>>,
    permits: Arc<Semaphore>,
}

/// Idle connections older than this are pinged before reuse.
const REVALIDATE_AFTER: Duration = Duration::from_secs(30);
/// Idle connections older than this are closed.
const MAX_IDLE: Duration = Duration::from_secs(300);

impl Pool {
    pub fn new(driver: Arc<dyn Driver>, size: usize) -> Arc<Self> {
        Arc::new(Self {
            driver,
            idle: Mutex::new(Vec::new()),
            permits: Arc::new(Semaphore::new(size.max(1))),
        })
    }

    pub fn driver(&self) -> &Arc<dyn Driver> {
        &self.driver
    }

    pub async fn get(self: &Arc<Self>) -> Result<Pooled> {
        let permit = tokio::time::timeout(
            Duration::from_secs(30),
            self.permits.clone().acquire_owned(),
        )
        .await
        .map_err(|_| DbError::Connect("connection pool exhausted (all connections busy)".into()))?
        .map_err(|_| DbError::Internal("pool closed".into()))?;

        loop {
            let candidate = self.idle.lock().unwrap_or_else(|e| e.into_inner()).pop();
            let Some((mut conn, since)) = candidate else {
                break;
            };
            let age = since.elapsed();
            if age > MAX_IDLE {
                continue;
            }
            if age > REVALIDATE_AFTER && conn.ping().await.is_err() {
                continue;
            }
            return Ok(Pooled::new(conn, self.clone(), permit));
        }
        let conn = self.driver.connect().await?;
        Ok(Pooled::new(conn, self.clone(), permit))
    }

    fn put_back(&self, conn: Box<dyn Conn>) {
        let mut idle = self.idle.lock().unwrap_or_else(|e| e.into_inner());
        idle.retain(|(_, since)| since.elapsed() < MAX_IDLE);
        idle.push((conn, Instant::now()));
    }
}

/// A checked-out connection. Returned to the pool on drop unless marked broken.
pub struct Pooled {
    conn: Option<Box<dyn Conn>>,
    pool: Arc<Pool>,
    _permit: OwnedSemaphorePermit,
    broken: bool,
}

impl Pooled {
    fn new(conn: Box<dyn Conn>, pool: Arc<Pool>, permit: OwnedSemaphorePermit) -> Self {
        Self {
            conn: Some(conn),
            pool,
            _permit: permit,
            broken: false,
        }
    }

    /// Discard instead of returning to the pool.
    pub fn mark_broken(&mut self) {
        self.broken = true;
    }

    pub fn conn(&mut self) -> &mut dyn Conn {
        self.conn.as_deref_mut().expect("conn present until drop")
    }
}

impl Drop for Pooled {
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take()
            && !self.broken
        {
            self.pool.put_back(conn);
        }
    }
}
