//! Database engines behind one interface.
//!
//! A [`Driver`] is built from a connection profile and opens [`Conn`]s. A conn
//! executes scripts, streaming [`QueryEvent`]s into an [`EventSink`]. [`Pool`]
//! keeps idle conns per profile; sessions hold a conn of their own.

pub mod entra;
pub mod explain;
pub mod introspect;
pub mod mssql;
pub mod pg;
pub mod pg_guard;
pub mod registry;
pub mod split;
pub mod sqlite;
pub(crate) mod tls_client;
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
    /// Refuse anything but exactly one statement ([`sqail_proto::QueryRequest::single_statement`]).
    pub single_statement: bool,
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
    /// Undo session state that user SQL may have changed (settings, role,
    /// temp tables, prepared statements, ...), so that the connection can be
    /// pooled for the next borrower. An error means "do not reuse".
    async fn reset(&mut self) -> Result<()>;
    fn engine(&self) -> Engine;
}

/// Run `work`, a query on a connection whose canceller is `canceller`, for at
/// most `limit`. Past it the query is cancelled and gets `grace` to stop.
/// Returns the result (`Timeout` when cut off) and whether the connection
/// must be discarded because the query may still be running.
pub async fn with_deadline<T>(
    canceller: Canceller,
    limit: Duration,
    grace: Duration,
    work: impl Future<Output = Result<T>>,
) -> (Result<T>, bool) {
    tokio::pin!(work);
    if let Ok(r) = tokio::time::timeout(limit, &mut work).await {
        return (r, false);
    }
    if !canceller.cancel().await {
        return (Err(DbError::Timeout), true);
    }
    let stopped = tokio::time::timeout(grace, &mut work).await.is_ok();
    (Err(DbError::Timeout), !stopped)
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
        single_statement: false,
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
        single_statement: false,
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

/// How long a reset may take before the connection is given up.
const RESET_TIMEOUT: Duration = Duration::from_secs(5);
/// How long an abandoned connection's cancel request may take.
const ABANDON_CANCEL_TIMEOUT: Duration = Duration::from_secs(10);

/// A checked-out connection.
///
/// It goes back to the pool only through [`Pooled::release`] or
/// [`Pooled::reset_and_release`]. Dropped any other way (the request future
/// was dropped by the request timeout, an early return, a panic) after it
/// was used, it is closed, and whatever may still run on it is cancelled
/// first. Unused, it goes back as it was.
pub struct Pooled {
    conn: Option<Box<dyn Conn>>,
    pool: Arc<Pool>,
    permit: Option<OwnedSemaphorePermit>,
    used: bool,
}

impl Pooled {
    fn new(conn: Box<dyn Conn>, pool: Arc<Pool>, permit: OwnedSemaphorePermit) -> Self {
        Self {
            conn: Some(conn),
            pool,
            permit: Some(permit),
            used: false,
        }
    }

    pub fn conn(&mut self) -> &mut dyn Conn {
        self.used = true;
        self.conn
            .as_deref_mut()
            .expect("conn present until released")
    }

    /// Return the connection as it is. Only for connections that ran nothing
    /// but the service's own catalog SQL, which leaves no session state.
    pub fn release(mut self) {
        if let Some(conn) = self.conn.take() {
            self.pool.put_back(conn);
        }
    }

    /// Reset the session state user SQL may have left, then return the
    /// connection; close it if the reset fails.
    pub async fn reset_and_release(mut self) {
        let Some(mut conn) = self.conn.take() else {
            return;
        };
        match tokio::time::timeout(RESET_TIMEOUT, conn.reset()).await {
            Ok(Ok(())) => self.pool.put_back(conn),
            Ok(Err(e)) => tracing::debug!(error = %e, "connection not reusable; closing it"),
            Err(_) => tracing::debug!("connection reset timed out; closing it"),
        }
    }

    /// Close instead of returning to the pool.
    pub fn discard(mut self) {
        self.conn.take();
    }
}

impl Drop for Pooled {
    fn drop(&mut self) {
        let Some(conn) = self.conn.take() else {
            return;
        };
        if !self.used {
            self.pool.put_back(conn);
            return;
        }
        // Abandoned mid-use: a query may still be running. Cancel it while
        // the connection is still open (a SQL Server KILL must not hit a
        // session id that has been handed to someone else), then close it.
        // The permit is held until then so the pool does not overcommit.
        let permit = self.permit.take();
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(async move {
                let canceller = conn.canceller();
                let _ = tokio::time::timeout(ABANDON_CANCEL_TIMEOUT, canceller.cancel()).await;
                drop(conn);
                drop(permit);
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[derive(Default)]
    struct Counts {
        connects: AtomicUsize,
        resets: AtomicUsize,
        cancels: AtomicUsize,
        closed: AtomicUsize,
    }

    struct FakeDriver {
        counts: Arc<Counts>,
        reset_ok: bool,
    }

    struct FakeConn {
        counts: Arc<Counts>,
        reset_ok: bool,
    }

    struct FakeCancel(Arc<Counts>);

    #[async_trait]
    impl Cancel for FakeCancel {
        async fn cancel(&self) -> bool {
            self.0.cancels.fetch_add(1, Ordering::SeqCst);
            true
        }
    }

    impl Drop for FakeConn {
        fn drop(&mut self) {
            self.counts.closed.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl Conn for FakeConn {
        async fn execute(&mut self, _: &ExecRequest, _: &EventSink) -> Result<()> {
            std::future::pending().await
        }
        fn canceller(&self) -> Canceller {
            Arc::new(FakeCancel(self.counts.clone()))
        }
        async fn in_transaction(&mut self) -> Result<bool> {
            Ok(false)
        }
        async fn ping(&mut self) -> Result<()> {
            Ok(())
        }
        async fn server_version(&mut self) -> Result<String> {
            Ok("fake".into())
        }
        async fn reset(&mut self) -> Result<()> {
            self.counts.resets.fetch_add(1, Ordering::SeqCst);
            if self.reset_ok {
                Ok(())
            } else {
                Err(DbError::Unsupported("no".into()))
            }
        }
        fn engine(&self) -> Engine {
            Engine::Sqlite
        }
    }

    #[async_trait]
    impl Driver for FakeDriver {
        async fn connect(&self) -> Result<Box<dyn Conn>> {
            self.counts.connects.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(FakeConn {
                counts: self.counts.clone(),
                reset_ok: self.reset_ok,
            }))
        }
        fn engine(&self) -> Engine {
            Engine::Sqlite
        }
    }

    fn pool(reset_ok: bool) -> (Arc<Pool>, Arc<Counts>) {
        let counts = Arc::new(Counts::default());
        let driver = Arc::new(FakeDriver {
            counts: counts.clone(),
            reset_ok,
        });
        (Pool::new(driver, 1), counts)
    }

    fn idle(p: &Pool) -> usize {
        p.idle.lock().unwrap().len()
    }

    fn n(a: &AtomicUsize) -> usize {
        a.load(Ordering::SeqCst)
    }

    #[tokio::test]
    async fn released_connections_are_reused() {
        let (p, counts) = pool(true);
        let mut c = p.get().await.unwrap();
        c.conn();
        c.release();
        assert_eq!(idle(&p), 1);
        let mut c = p.get().await.unwrap();
        c.conn();
        c.reset_and_release().await;
        assert_eq!(idle(&p), 1);
        assert_eq!(n(&counts.connects), 1);
        assert_eq!(n(&counts.resets), 1);
        assert_eq!(n(&counts.cancels), 0);
    }

    #[tokio::test]
    async fn failed_resets_and_discards_close_the_connection() {
        let (p, counts) = pool(false);
        let c = p.get().await.unwrap();
        c.reset_and_release().await;
        assert_eq!(idle(&p), 0);
        let c = p.get().await.unwrap();
        c.discard();
        assert_eq!(idle(&p), 0);
        assert_eq!(n(&counts.connects), 2);
        assert_eq!(n(&counts.closed), 2);
    }

    #[tokio::test]
    async fn unused_connections_go_back_on_drop() {
        let (p, counts) = pool(true);
        drop(p.get().await.unwrap());
        assert_eq!(idle(&p), 1);
        assert_eq!(n(&counts.closed), 0);
    }

    #[tokio::test]
    async fn abandoned_connections_are_cancelled_and_closed() {
        let (p, counts) = pool(true);
        let mut c = p.get().await.unwrap();
        // A request future dropped mid-query, as the request timeout does.
        let req = ExecRequest {
            sql: "x".into(),
            params: vec![],
            max_rows: 1,
            single_statement: false,
        };
        let sink = EventSink::Collect(Mutex::new(Vec::new()));
        let run = tokio::time::timeout(Duration::from_millis(10), c.conn().execute(&req, &sink));
        assert!(run.await.is_err());
        drop(c);
        // The pool's only permit comes back once the cancel has gone out.
        let mut c = p.get().await.unwrap();
        assert_eq!(n(&counts.cancels), 1);
        assert_eq!(n(&counts.closed), 1);
        assert_eq!(n(&counts.connects), 2);
        assert_eq!(idle(&p), 0);
        c.conn();
        c.release();
    }

    #[tokio::test]
    async fn deadlines_cancel_and_report_whether_the_query_stopped() {
        let (p, counts) = pool(true);
        let mut c = p.get().await.unwrap();
        let canceller = c.conn().canceller();
        let ms = Duration::from_millis(20);
        // Still running after the cancel and the grace period: discard.
        let (r, broken) = with_deadline(
            canceller.clone(),
            ms,
            ms,
            std::future::pending::<Result<()>>(),
        )
        .await;
        assert!(matches!(r, Err(DbError::Timeout)) && broken);
        // Stops within the grace period: keep.
        let work = async {
            tokio::time::sleep(ms * 2).await;
            Ok(())
        };
        let (r, broken) = with_deadline(canceller.clone(), ms, ms * 5, work).await;
        assert!(matches!(r, Err(DbError::Timeout)) && !broken);
        // In time.
        let (r, broken) = with_deadline(canceller, ms, ms, async { Ok(1) }).await;
        assert_eq!(r.unwrap(), 1);
        assert!(!broken);
        assert_eq!(n(&counts.cancels), 2);
        c.release();
    }
}
