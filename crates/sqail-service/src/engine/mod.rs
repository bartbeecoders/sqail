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

/// Most rows per `rows` event.
pub const ROW_BATCH: usize = 500;
/// A `rows` event is sent once its rows' JSON reaches this size, so that
/// one NDJSON line stays bounded however wide the rows are. A query's
/// response channel holds up to 32 events, so this also bounds what the
/// service buffers for a slow client (about 32 MiB).
pub const BATCH_BYTES: usize = 1024 * 1024;
/// Largest JSON size of one row. Longer values are cut to fit, with a
/// warning, so that a single huge cell cannot make a line the client
/// refuses (its `MAX_LINE`, 256 MiB). A line is at most about
/// `BATCH_BYTES + MAX_ROW_BYTES`.
pub const MAX_ROW_BYTES: usize = 64 * 1024 * 1024;

/// The rows of one result set waiting to be sent: flushed at [`ROW_BATCH`]
/// rows or [`BATCH_BYTES`] of JSON, whichever comes first.
pub(crate) struct RowBatch {
    rows: Vec<Vec<serde_json::Value>>,
    bytes: usize,
    cut_warned: bool,
}

impl RowBatch {
    pub(crate) fn new() -> Self {
        Self {
            rows: Vec::with_capacity(ROW_BATCH),
            bytes: 0,
            cut_warned: false,
        }
    }

    /// Add a row, cutting values that make it longer than [`MAX_ROW_BYTES`].
    /// Returns a warning to send when this is the result set's first row
    /// with a cut value.
    pub(crate) fn push(&mut self, mut row: Vec<serde_json::Value>) -> Option<String> {
        let (bytes, cut) = cap_row(&mut row, MAX_ROW_BYTES);
        self.bytes += bytes;
        self.rows.push(row);
        if cut && !self.cut_warned {
            self.cut_warned = true;
            return Some(format!(
                "Some values in this result were cut (ending in …): a row may take at \
                 most {} MiB in a result. Select a part of them (substring) to see the rest.",
                MAX_ROW_BYTES / (1024 * 1024)
            ));
        }
        None
    }

    pub(crate) fn is_full(&self) -> bool {
        self.rows.len() >= ROW_BATCH || self.bytes >= BATCH_BYTES
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub(crate) fn take(&mut self) -> Vec<Vec<serde_json::Value>> {
        self.bytes = 0;
        std::mem::replace(&mut self.rows, Vec::with_capacity(ROW_BATCH))
    }
}

/// Bytes `s` takes as a JSON string, quotes included.
fn json_str_len(s: &str) -> usize {
    2 + s.bytes().map(escaped_len).sum::<usize>()
}

fn escaped_len(b: u8) -> usize {
    match b {
        b'"' | b'\\' | b'\n' | b'\r' | b'\t' | 0x08 | 0x0c => 2,
        0..0x20 => 6,
        _ => 1,
    }
}

/// Bytes `v` takes as JSON (an upper bound for numbers).
fn json_len(v: &serde_json::Value) -> usize {
    use serde_json::Value;
    match v {
        Value::Null => 4,
        Value::Bool(b) => {
            if *b {
                4
            } else {
                5
            }
        }
        Value::Number(_) => 24,
        Value::String(s) => json_str_len(s),
        // Brackets, and a `,` between values.
        Value::Array(a) => 1 + a.iter().map(|v| json_len(v) + 1).sum::<usize>().max(1),
        Value::Object(o) => {
            1 + o
                .iter()
                .map(|(k, v)| json_str_len(k) + json_len(v) + 2)
                .sum::<usize>()
                .max(1)
        }
    }
}

/// Cut values so that `row` takes at most about `max` bytes as JSON: a
/// string keeps the prefix that fits, followed by `…`; a JSON array or
/// object becomes the start of its JSON text. Returns the row's JSON size
/// and whether anything was cut.
fn cap_row(row: &mut [serde_json::Value], max: usize) -> (usize, bool) {
    use serde_json::Value;
    // `[` and `]`, and a `,` after each value but the last.
    let mut used = 1;
    let mut cut = false;
    for v in row.iter_mut() {
        let len = json_len(v) + 1;
        if used + len <= max {
            used += len;
            continue;
        }
        let budget = max.saturating_sub(used + 1);
        let text = match std::mem::take(v) {
            Value::String(s) => s,
            other @ (Value::Array(_) | Value::Object(_)) => other.to_string(),
            // Scalars are a few bytes: keep them.
            other => {
                *v = other;
                used += len;
                continue;
            }
        };
        let s = cut_to_json_len(text, budget);
        used += json_str_len(&s) + 1;
        *v = Value::String(s);
        cut = true;
    }
    (used, cut)
}

/// The longest prefix of `s` (on a char boundary) that, with a trailing
/// `…`, takes at most `budget` bytes as a JSON string.
fn cut_to_json_len(mut s: String, budget: usize) -> String {
    const MARK: &str = "…";
    let room = budget.saturating_sub(2 + MARK.len());
    let mut used = 0;
    // Bytes of a multi-byte char are all >= 0x80, so they take one byte
    // each; back off to a char boundary afterwards.
    let mut end = s.len();
    for (i, b) in s.bytes().enumerate() {
        used += escaped_len(b);
        if used > room {
            end = i;
            break;
        }
    }
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
    s.push_str(MARK);
    s
}

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
    /// An established connection died (server restart, idle kill, ...).
    #[error("connection to the database was lost: {0}")]
    Lost(String),
    /// The connection turned out to be dead before the query was sent to
    /// the server, so nothing ran: a pooled query is retried once on a
    /// fresh connection.
    #[error("connection to the database was lost before the query was sent; nothing was run")]
    Stale,
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
    /// The script failed, and its errors have been sent as `error` events
    /// already (SQL Server goes on after an error, as SSMS does).
    #[error("the script reported errors")]
    Reported,
}

impl DbError {
    /// A profile whose engine this build does not know ([`Engine::Unknown`]).
    pub fn unknown_engine() -> Self {
        DbError::Unsupported("this engine is not supported by this sqail-service version".into())
    }

    pub fn code(&self) -> &'static str {
        match self {
            DbError::Database { .. } => "db_error",
            DbError::Connect(_) => "connect_failed",
            DbError::Lost(_) | DbError::Stale => "connection_lost",
            DbError::Cancelled => "cancelled",
            DbError::Timeout => "timeout",
            DbError::ClientGone => "client_gone",
            DbError::Invalid(_) => "invalid_request",
            DbError::Unsupported(_) => "unsupported",
            DbError::Internal(_) => "internal",
            DbError::Reported => "db_error",
        }
    }

    /// Whether the connection is unusable after this error.
    pub fn is_connection_gone(&self) -> bool {
        matches!(
            self,
            DbError::Connect(_) | DbError::Lost(_) | DbError::Stale
        )
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
    errors: bool,
}

struct OpenResult {
    index: u32,
    count: u64,
    truncated: bool,
    batch: RowBatch,
}

impl<'a> ResultWriter<'a> {
    pub fn new(sink: &'a EventSink, max_rows: u64) -> Self {
        Self {
            sink,
            max_rows,
            next_index: 0,
            current: None,
            errors: false,
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
            batch: RowBatch::new(),
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
        let warning = cur.batch.push(row);
        let index = cur.index;
        let full = cur.batch.is_full().then(|| cur.batch.take());
        if let Some(text) = warning {
            self.message("warning", text).await?;
        }
        if let Some(rows) = full {
            self.sink.send(QueryEvent::Rows { index, rows }).await?;
        }
        Ok(())
    }

    pub async fn end(&mut self) -> Result<()> {
        if let Some(mut cur) = self.current.take() {
            if !cur.batch.is_empty() {
                self.sink
                    .send(QueryEvent::Rows {
                        index: cur.index,
                        rows: cur.batch.take(),
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

    /// Whether the open result set holds `max_rows` rows, so that any
    /// further row is dropped. Drivers then skip converting rows (calling
    /// [`skip_row`](Self::skip_row) instead) or stop reading altogether.
    pub fn is_truncated(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|cur| cur.count >= self.max_rows)
    }

    /// Record a row past `max_rows` without converting it.
    pub fn skip_row(&mut self) {
        if let Some(cur) = self.current.as_mut() {
            cur.truncated = true;
        }
    }

    /// The row limit to ask a server for so that truncation shows: one more
    /// than `max_rows`, or `None` (no limit) when that does not fit the wire.
    pub fn fetch_limit(&self) -> Option<i32> {
        i32::try_from(self.max_rows.saturating_add(1)).ok()
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

    /// An error that does not end the script: SQL Server goes on with the
    /// batch or the next one, and reports each error where it happens.
    pub async fn error(&mut self, e: &DbError) -> Result<()> {
        self.errors = true;
        self.end().await?;
        self.sink.send(e.to_event()).await
    }

    /// Whether [`error`](Self::error) was called.
    pub fn reported_errors(&self) -> bool {
        self.errors
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

    /// Like [`cancel`](Self::cancel), for a query whose connection is about
    /// to be closed without being polled again. A driver whose cancel needs
    /// the running query's future polled (SQL Server's Attention) overrides it.
    async fn cancel_abandoned(&self) -> bool {
        self.cancel().await
    }
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
    /// Whether the driver already knows the connection is dead (the server
    /// closed it), without a round trip.
    fn is_closed(&self) -> bool {
        false
    }
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
    match conn.execute(&req, &sink).await {
        // The error is among the events.
        Ok(()) | Err(DbError::Reported) => {}
        Err(e) => return Err(e),
    }
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
    match conn.execute(&req, &sink).await {
        // The error is among the events.
        Ok(()) | Err(DbError::Reported) => {}
        Err(e) => return Err(e),
    }
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
/// How long that ping may take (a connection a firewall dropped silently
/// would otherwise hang until TCP gives up).
const PING_TIMEOUT: Duration = Duration::from_secs(5);
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
            // Closed by the server meanwhile (restart, pg_terminate_backend,
            // idle timeout): replace it rather than fail the next query.
            if age > MAX_IDLE || conn.is_closed() {
                continue;
            }
            if age > REVALIDATE_AFTER
                && !matches!(
                    tokio::time::timeout(PING_TIMEOUT, conn.ping()).await,
                    Ok(Ok(()))
                )
            {
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

    /// Replace the connection with a fresh one, keeping the pool slot: for
    /// a connection found dead before anything was sent on it.
    pub async fn reconnect(&mut self) -> Result<()> {
        self.conn = Some(self.pool.driver.connect().await?);
        self.used = false;
        Ok(())
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
                let _ = tokio::time::timeout(ABANDON_CANCEL_TIMEOUT, canceller.cancel_abandoned())
                    .await;
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
        /// What `is_closed` says (the server closed the connections).
        dead: std::sync::atomic::AtomicBool,
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
        fn is_closed(&self) -> bool {
            self.counts.dead.load(Ordering::SeqCst)
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

    /// A pooled connection the server closed while it was idle is replaced
    /// instead of failing the next query.
    #[tokio::test]
    async fn connections_closed_while_idle_are_replaced() {
        let (p, counts) = pool(true);
        let mut c = p.get().await.unwrap();
        c.conn();
        c.release();
        counts.dead.store(true, Ordering::SeqCst);
        let mut c = p.get().await.unwrap();
        assert_eq!(n(&counts.connects), 2);
        assert_eq!(n(&counts.closed), 1);
        counts.dead.store(false, Ordering::SeqCst);
        // And a checked-out connection found dead is swapped in place.
        c.conn();
        c.reconnect().await.unwrap();
        assert_eq!(n(&counts.connects), 3);
        assert_eq!(n(&counts.closed), 2);
        c.release();
        assert_eq!(idle(&p), 1);
    }

    #[test]
    fn lost_connections_say_so() {
        assert_eq!(DbError::Stale.code(), "connection_lost");
        assert!(
            DbError::Lost("x".into())
                .to_string()
                .starts_with("connection to the database was lost")
        );
        assert!(DbError::Stale.is_connection_gone() && !DbError::Timeout.is_connection_gone());
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

    #[test]
    fn json_len_matches_serde() {
        use serde_json::json;
        for v in [
            json!(null),
            json!(true),
            json!("plain"),
            json!("quote \" back \\ nl \n tab \t ctl \u{1} é ☃"),
            json!(["a", null, false]),
            json!({"k": "v", "n": null}),
        ] {
            assert_eq!(
                json_len(&v),
                serde_json::to_string(&v).unwrap().len(),
                "{v}"
            );
        }
        assert!(
            json_len(&json!(-1.5e300)) >= serde_json::to_string(&json!(-1.5e300)).unwrap().len()
        );
    }

    /// Wide rows are sent in batches of about `BATCH_BYTES`, so that one
    /// NDJSON line stays far below what the typed client accepts.
    #[tokio::test]
    async fn row_batches_are_capped_by_size() {
        let sink = EventSink::Collect(Mutex::new(Vec::new()));
        let mut w = ResultWriter::new(&sink, u64::MAX);
        w.start(Vec::new()).await.unwrap();
        let cell = "x".repeat(600_000);
        for i in 0..500 {
            w.row(vec![serde_json::json!(i), serde_json::json!(cell)])
                .await
                .unwrap();
        }
        w.end().await.unwrap();
        let EventSink::Collect(buf) = sink else {
            unreachable!()
        };
        let mut rows = 0;
        let mut lines = 0;
        for ev in buf.into_inner().unwrap() {
            if let QueryEvent::Rows { rows: r, .. } = &ev {
                let line = serde_json::to_vec(&ev).unwrap().len();
                assert!(line < BATCH_BYTES + 1_000_000, "{line}");
                rows += r.len();
                lines += 1;
            }
            assert!(!matches!(ev, QueryEvent::Message { .. }));
        }
        assert_eq!(rows, 500);
        assert!(lines > 50, "{lines}");
    }

    #[test]
    fn the_largest_line_fits_the_client() {
        // A batch flushes once it reaches BATCH_BYTES, so its last row
        // starts below it; that row takes at most MAX_ROW_BYTES (plus the
        // event's own few bytes).
        const { assert!(BATCH_BYTES + MAX_ROW_BYTES + 4096 < sqail_client::MAX_LINE) };
    }

    /// A single value too large for a line is cut, with one warning per
    /// result set, instead of producing a line the client would refuse.
    #[tokio::test]
    async fn huge_values_are_cut_with_a_warning() {
        let sink = EventSink::Collect(Mutex::new(Vec::new()));
        let mut w = ResultWriter::new(&sink, u64::MAX);
        w.start(Vec::new()).await.unwrap();
        let huge = "é".repeat(MAX_ROW_BYTES / 2 + 10);
        for _ in 0..2 {
            w.row(vec![serde_json::json!(1), serde_json::json!(huge)])
                .await
                .unwrap();
        }
        w.row(vec![serde_json::json!("small")]).await.unwrap();
        w.end().await.unwrap();
        let EventSink::Collect(buf) = sink else {
            unreachable!()
        };
        let events = buf.into_inner().unwrap();
        let warnings = events
            .iter()
            .filter(|e| matches!(e, QueryEvent::Message { severity, .. } if severity == "warning"))
            .count();
        assert_eq!(warnings, 1);
        let rows: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                QueryEvent::Rows { rows, .. } => Some(rows.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        assert_eq!(rows.len(), 3);
        for row in &rows[..2] {
            let len = serde_json::to_string(row).unwrap().len();
            assert!(len <= MAX_ROW_BYTES && len > MAX_ROW_BYTES - 64, "{len}");
            let s = row[1].as_str().unwrap();
            assert!(s.ends_with('…') && s.starts_with("éé"));
        }
        assert_eq!(rows[2], vec![serde_json::json!("small")]);
    }

    #[test]
    fn cutting_respects_escapes() {
        let mut row = vec![serde_json::json!("\u{1}".repeat(100))];
        let (len, cut) = cap_row(&mut row, 64);
        assert!(cut);
        assert_eq!(len, serde_json::to_string(&row).unwrap().len());
        assert!(len <= 64, "{len}");
        let mut row = vec![serde_json::json!({"a": "b".repeat(100)})];
        assert!(cap_row(&mut row, 40).1);
        assert!(row[0].as_str().unwrap().starts_with("{\"a\":\"bbb"));
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
