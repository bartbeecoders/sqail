//! Microsoft SQL Server via tiberius.
//!
//! Scripts are split on `GO` here (the server does not understand it). Each
//! batch runs as one request with multiple result sets. The (patched)
//! tiberius result stream also yields INFO messages (`PRINT`), DONE row
//! counts and errors where the server sends them, so they reach the client
//! in that order. As in SSMS and sqlcmd, a failed batch does not stop the
//! script: the next `GO` batch runs, and error lines count from the start of
//! the script.
//! Cancelling sends a TDS Attention, as SSMS does: it aborts the running
//! batch and leaves the session (its transaction, unless XACT_ABORT is on,
//! #temp tables and SET options) as it was; later `GO` batches do not run.
//! Only when the server does not acknowledge it in time does the canceller
//! fall back to `KILL <spid>` from a side connection (needs ALTER ANY
//! CONNECTION); the caller drops the connection if that does not stop it
//! either. A batch that is a single query is also stopped with an Attention
//! once it passes `max_rows`; other batches skip converting the rows past it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use futures::TryStreamExt;
use serde_json::Value;
use sqail_proto::{Column, Engine, LogicalType, MssqlAuth, MssqlEncrypt, MssqlParams, Param};
use tiberius::{
    AttentionHandle, AuthMethod, ColumnData, ColumnType, Config, EncryptionLevel, FromSql,
    QueryItem,
};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

use super::entra::Entra;
use super::split::{check_single_statement, single_query, split_go};
use super::value::{bytes, float, float32, int, text};
use super::{
    Cancel, Canceller, Conn, DbError, Driver, EventSink, ExecRequest, Result, ResultWriter,
};

type Client = tiberius::Client<Compat<TcpStream>>;

#[derive(Clone)]
pub struct MssqlDriver {
    config: Config,
    /// Microsoft Entra ID sign-in: a fresh or cached token per connection.
    entra: Option<Arc<Entra>>,
}

impl MssqlDriver {
    pub fn new(p: &MssqlParams, password: Option<&str>, read_only: bool) -> Result<Self> {
        let mut config = Config::new();
        let target = Target::parse(p);
        config.host(&target.host);
        config.port(target.port);
        if let Some(inst) = &target.instance {
            config.instance_name(inst);
        }
        if let Some(db) = &p.database {
            config.database(db);
        }
        config.application_name("sqail");
        config.encryption(match p.encrypt {
            MssqlEncrypt::Off => EncryptionLevel::Off,
            MssqlEncrypt::On => EncryptionLevel::On,
            MssqlEncrypt::Required => EncryptionLevel::Required,
        });
        if p.trust_server_certificate {
            config.trust_cert();
        }
        config.readonly(read_only);
        // The service enforces its own timeouts and cancellation.
        config.command_timeout(None);
        config.handshake_timeout(Some(Duration::from_secs(15)));
        match &p.auth {
            MssqlAuth::Sql { user } => {
                config.authentication(AuthMethod::sql_server(user, password.unwrap_or("")));
            }
            #[cfg(windows)]
            MssqlAuth::Integrated => config.authentication(AuthMethod::Integrated),
            #[cfg(not(windows))]
            MssqlAuth::Integrated => {
                return Err(DbError::Unsupported(
                    "Windows integrated authentication needs sqail-service running on Windows"
                        .into(),
                ));
            }
            // The token is set when a connection opens; see `open`.
            MssqlAuth::EntraPassword { .. }
            | MssqlAuth::EntraServicePrincipal { .. }
            | MssqlAuth::EntraManagedIdentity { .. } => {}
        }
        let entra = Entra::from_auth(&p.auth, password)?.map(Arc::new);
        Ok(Self { config, entra })
    }

    async fn open(&self) -> Result<Client> {
        let mut config = self.config.clone();
        if let Some(entra) = &self.entra {
            config.authentication(AuthMethod::aad_token(entra.token().await?));
        }
        let mut redirected = false;
        // Follow at most one redirect (Azure SQL gateway routing).
        for _ in 0..2 {
            let tcp = tokio::time::timeout(Duration::from_secs(10), async {
                if redirected {
                    // The redirect target is a plain host and port.
                    TcpStream::connect(config.get_addr())
                        .await
                        .map_err(|e| e.to_string())
                } else {
                    // With a named instance this first asks SQL Browser
                    // (UDP 1434 on the host) for the instance's TCP port.
                    <TcpStream as tiberius::SqlBrowser>::connect_named(&config)
                        .await
                        .map_err(|e| e.to_string())
                }
            })
            .await
            .map_err(|_| {
                DbError::Connect(format!("timed out connecting to {}", config.get_addr()))
            })?
            .map_err(|e| DbError::Connect(format!("{}: {e}", config.get_addr())))?;
            tcp.set_nodelay(true).ok();
            match tiberius::Client::connect(config.clone(), tcp.compat_write()).await {
                Ok(client) => return Ok(client),
                Err(tiberius::error::Error::Routing { host, port }) => {
                    config.host(host);
                    config.port(port);
                    redirected = true;
                }
                Err(e) => return Err(DbError::Connect(ms_message(&e))),
            }
        }
        Err(DbError::Connect("too many redirects".into()))
    }
}

/// Where to connect, from the profile. Accepts what people paste from SSMS:
/// `host\INSTANCE` and `host,port` in the host field.
#[derive(Debug, PartialEq, Eq)]
struct Target {
    host: String,
    port: u16,
    instance: Option<String>,
}

/// SQL Browser's UDP port, asked for a named instance's TCP port.
const SQL_BROWSER_PORT: u16 = 1434;
const DEFAULT_PORT: u16 = 1433;

impl Target {
    fn parse(p: &MssqlParams) -> Self {
        let raw = p.host.trim().trim_start_matches("tcp:");
        let mut instance = p
            .instance
            .as_deref()
            .map(str::trim)
            .filter(|i| !i.is_empty())
            .map(String::from);
        let mut port = p.port;
        let mut host = raw.to_string();
        if let Some((h, prt)) = raw.split_once(',')
            && let Ok(prt) = prt.trim().parse::<u16>()
        {
            // An explicit port wins over any instance, as in SSMS.
            return Self {
                host: h.trim().to_string(),
                port: prt,
                instance: None,
            };
        }
        if let Some((h, inst)) = raw.split_once('\\') {
            host = h.trim().to_string();
            if instance.is_none() && !inst.trim().is_empty() {
                instance = Some(inst.trim().to_string());
            }
        }
        if host.is_empty() || host == "." || host.eq_ignore_ascii_case("(local)") {
            host = "localhost".into();
        }
        if instance.is_some() {
            if port == DEFAULT_PORT {
                port = SQL_BROWSER_PORT;
            } else {
                // A non-default port: connect straight to it.
                instance = None;
            }
        }
        Self {
            host,
            port,
            instance,
        }
    }
}

#[async_trait]
impl Driver for MssqlDriver {
    async fn connect(&self) -> Result<Box<dyn Conn>> {
        let mut client = self.open().await?;
        // Messages, row counts and errors in order, on user scripts' streams.
        client.set_messages(true);
        let spid = client
            .simple_query("SELECT CAST(@@SPID AS int)")
            .await
            .map_err(db_err)?
            .into_row()
            .await
            .map_err(db_err)?
            .and_then(|r| int_cell(&r, 0))
            .unwrap_or(0);
        let attention = client.attention_handle();
        Ok(Box::new(MssqlConn {
            client,
            in_tx: false,
            cancel: Arc::new(MssqlCancel {
                driver: self.clone(),
                spid,
                attention,
                runs: Arc::default(),
            }),
        }))
    }

    fn engine(&self) -> Engine {
        Engine::Mssql
    }
}

struct MssqlConn {
    client: Client,
    cancel: Arc<MssqlCancel>,
    /// The last transaction state the probe could read. With SHOWPLAN on
    /// the server answers the probe with a plan instead of running it; it
    /// does not run BEGIN/COMMIT either then, so the state stays as it was.
    in_tx: bool,
}

/// An `int` cell, or `None` for NULL or anything else. Probes get a plan
/// instead of their value when SHOWPLAN_XML/TEXT/ALL is on, so the shape
/// of their result is never trusted (`Row::get` would panic on it).
fn int_cell(row: &tiberius::Row, idx: usize) -> Option<i32> {
    row.try_get::<i32, _>(idx).ok().flatten()
}

impl Drop for MssqlConn {
    fn drop(&mut self) {
        self.cancel.runs.running.store(false, Ordering::Release);
    }
}

/// How long the server gets to acknowledge an Attention before the
/// canceller falls back to KILL. Shorter than the callers' grace periods
/// (5 s and more), so that the KILL can still stop the query in time.
const ATTENTION_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone)]
struct MssqlCancel {
    driver: MssqlDriver,
    spid: i32,
    attention: AttentionHandle,
    runs: Arc<Runs>,
}

/// Which `execute` is running, so a late KILL fallback cannot hit a later
/// query or a closed connection's session id.
#[derive(Default)]
struct Runs {
    running: AtomicBool,
    finished: AtomicU64,
}

impl Runs {
    fn start(&self) {
        self.running.store(true, Ordering::Release);
    }

    fn finish(&self) {
        self.finished.fetch_add(1, Ordering::AcqRel);
        self.running.store(false, Ordering::Release);
    }

    fn still_running(&self, finished: u64) -> bool {
        self.running.load(Ordering::Acquire) && self.finished.load(Ordering::Acquire) == finished
    }
}

impl MssqlCancel {
    /// Ends the whole session, not just the batch. Fails without ALTER ANY
    /// CONNECTION.
    async fn kill(&self, finished: Option<u64>) -> bool {
        if self.spid <= 0 {
            return false;
        }
        let res = async {
            let mut side = self.driver.open().await?;
            if finished.is_some_and(|f| !self.runs.still_running(f)) {
                return Ok(false);
            }
            side.simple_query(format!("KILL {}", self.spid))
                .await
                .map_err(db_err)?
                .into_results()
                .await
                .map_err(db_err)?;
            Ok::<_, DbError>(true)
        }
        .await;
        match res {
            Ok(killed) => killed,
            Err(e) => {
                tracing::info!(spid = self.spid, error = %e, "KILL failed; dropping the connection instead");
                false
            }
        }
    }
}

#[async_trait]
impl Cancel for MssqlCancel {
    /// Requests an Attention, which the running `execute` sends on its next
    /// read, so it needs that future polled: the callers wait for it after
    /// cancelling. If the query has not stopped after [`ATTENTION_TIMEOUT`],
    /// KILL it. A request made while no `execute` runs stops the next one
    /// before its first batch (the caller may cancel before the query's
    /// future was first polled); no caller cancels after a query finished.
    async fn cancel(&self) -> bool {
        let finished = self.runs.finished.load(Ordering::Acquire);
        self.attention.request();
        let this = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(ATTENTION_TIMEOUT).await;
            if this.runs.still_running(finished) {
                tracing::info!(
                    spid = this.spid,
                    "SQL Server did not acknowledge the Attention; trying KILL"
                );
                this.kill(Some(finished)).await;
            }
        });
        true
    }

    /// Nothing reads the connection any more, so an Attention would not go
    /// out: KILL before it is closed.
    async fn cancel_abandoned(&self) -> bool {
        self.kill(None).await
    }
}

/// What one run of a batch reported.
#[derive(Default)]
struct BatchOutput {
    /// Added to error line numbers: the batch's first line in the script,
    /// less one.
    line_offset: u32,
    /// Messages, row counts or errors were sent.
    reported: bool,
}

/// Stream a batch's result sets into `out`. Rows past `max_rows` are not
/// converted; with `stop_at_cap` the first one ends the stream and `true`
/// is returned, and the caller must cancel the rest of the batch.
async fn stream_results(
    mut stream: tiberius::QueryStream<'_>,
    out: &mut ResultWriter<'_>,
    stop_at_cap: bool,
    attention: &AttentionHandle,
    batch: &mut BatchOutput,
) -> Result<bool> {
    while let Some(item) = stream.try_next().await.map_err(db_err)? {
        match item {
            // Cancelled: what the server sent before the Attention reached it
            // is read to the acknowledgement but not passed on.
            _ if attention.is_requested() => {}
            QueryItem::Metadata(meta) => {
                out.start(meta.columns().iter().map(column).collect())
                    .await?
            }
            QueryItem::Message(m) => {
                batch.reported = true;
                out.message("info", m.message()).await?;
            }
            QueryItem::Error(e) => {
                batch.reported = true;
                out.error(&DbError::Database {
                    code: Some(e.code().to_string()),
                    message: server_message(&e, batch.line_offset),
                })
                .await?;
            }
            // The end of a SELECT: its row count is the result set's.
            QueryItem::Done(_) if out.is_open() => out.end().await?,
            QueryItem::Done(d) => {
                if let Some(count) = d.rows() {
                    batch.reported = true;
                    out.rows_affected(count).await?;
                }
            }
            QueryItem::Row(_) if out.is_truncated() => {
                out.skip_row();
                if stop_at_cap {
                    return Ok(true);
                }
            }
            QueryItem::Row(row) => out.row(row.cells().map(|(_, d)| cell(d)).collect()).await?,
        }
    }
    Ok(false)
}

impl MssqlConn {
    /// Whether an Attention, which aborts the running batch, would leave
    /// the session as it was. It would not with XACT_ABORT on inside a
    /// transaction: the server then rolls the transaction back.
    async fn attention_is_harmless(&mut self) -> Result<bool> {
        let row = self
            .client
            .simple_query("SELECT CAST(@@TRANCOUNT AS int), CAST(@@OPTIONS & 16384 AS int)")
            .await
            .map_err(db_err)?
            .into_row()
            .await
            .map_err(db_err)?;
        // Anything but two ints (a plan, under SHOWPLAN) means unknown: no.
        Ok(
            row.is_some_and(|r| match (int_cell(&r, 0), int_cell(&r, 1)) {
                (Some(trancount), Some(xact_abort)) => trancount == 0 || xact_abort == 0,
                _ => false,
            }),
        )
    }

    /// Run one batch; `line` is its first line in the script. Errors the
    /// server reports go to `out` and do not fail the batch.
    async fn run_batch(
        &mut self,
        sql: &str,
        line: u32,
        params: &[Param],
        out: &mut ResultWriter<'_>,
    ) -> Result<()> {
        if self.cancel.attention.is_requested() {
            return Err(DbError::Cancelled);
        }
        let results_before = out.results_seen();
        let mut batch = BatchOutput {
            line_offset: line.saturating_sub(1),
            reported: false,
        };
        // A lone query is stopped once it passes max_rows. The probe that
        // decides whether that is safe would change @@ROWCOUNT and @@ERROR,
        // so a query reading those runs to the end.
        let stop_at_cap = out.fetch_limit().is_some()
            && !sql.contains("@@")
            && single_query(sql)
            && self.attention_is_harmless().await?;
        let stream = if params.is_empty() {
            self.client.simple_query(sql).await.map_err(db_err)?
        } else {
            let mut q = tiberius::Query::new(sql);
            bind_all(&mut q, params);
            q.query(&mut self.client).await.map_err(db_err)?
        };
        if stream_results(stream, out, stop_at_cap, &self.cancel.attention, &mut batch).await? {
            // An Attention aborts the batch; tiberius then drains the
            // acknowledgement, which leaves the connection ready for reuse.
            self.client
                .cancel_query()
                .await
                .map_err(|e| DbError::Connect(format!("cannot stop the query at max_rows: {e}")))?;
        }
        out.end().await?;
        if self.cancel.attention.is_requested() {
            // Aborted, not completed; later batches do not run.
            return Err(DbError::Cancelled);
        }
        if out.results_seen() == results_before && !batch.reported {
            out.message("info", "Commands completed successfully.")
                .await?;
        }
        Ok(())
    }
}

fn bind_all<'a>(q: &mut tiberius::Query<'a>, params: &'a [Param]) {
    for p in params {
        match p {
            Param::Null => q.bind(Option::<&str>::None),
            Param::Bool(b) => q.bind(*b),
            Param::Int(v) => q.bind(*v),
            Param::Float(v) => q.bind(*v),
            Param::Text(s) => q.bind(s.as_str()),
        }
    }
}

#[async_trait]
impl Conn for MssqlConn {
    async fn execute(&mut self, req: &ExecRequest, sink: &EventSink) -> Result<()> {
        let mut out = ResultWriter::new(sink, req.max_rows);
        self.cancel.runs.start();
        let res = async {
            if req.single_statement {
                check_single_statement(&req.sql).map_err(DbError::Invalid)?;
            }
            if !req.params.is_empty() {
                return self.run_batch(&req.sql, 1, &req.params, &mut out).await;
            }
            for batch in split_go(&req.sql) {
                for _ in 0..batch.repeat {
                    self.run_batch(&batch.sql, batch.line, &[], &mut out)
                        .await?;
                }
            }
            Ok(())
        }
        .await;
        // After an Attention, read its acknowledgement so the session can
        // run the next request.
        let res = match self.client.finish_attention().await {
            Ok(false) => res,
            Ok(true) if matches!(res, Err(DbError::ClientGone)) => res,
            Ok(true) => Err(DbError::Cancelled),
            Err(e) => Err(DbError::Connect(format!("cannot cancel the query: {e}"))),
        };
        self.cancel.runs.finish();
        out.end().await?;
        match res {
            // Failed, though the script ran on (audited as a failure).
            Ok(()) if out.reported_errors() => Err(DbError::Reported),
            res => res,
        }
    }

    fn canceller(&self) -> Canceller {
        self.cancel.clone()
    }

    async fn in_transaction(&mut self) -> Result<bool> {
        let row = self
            .client
            .simple_query("SELECT CAST(@@TRANCOUNT AS int)")
            .await
            .map_err(db_err)?
            .into_row()
            .await
            .map_err(db_err)?;
        match row.as_ref().and_then(|r| int_cell(r, 0)) {
            Some(n) => self.in_tx = n > 0,
            None => tracing::debug!(
                "@@TRANCOUNT probe returned no int (SHOWPLAN on?); keeping the last known state"
            ),
        }
        Ok(self.in_tx)
    }

    async fn ping(&mut self) -> Result<()> {
        self.client
            .simple_query("SELECT 1")
            .await
            .map_err(db_err)?
            .into_results()
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn server_version(&mut self) -> Result<String> {
        let row = self
            .client
            .simple_query("SELECT @@VERSION")
            .await
            .map_err(db_err)?
            .into_row()
            .await
            .map_err(db_err)?;
        Ok(row
            .and_then(|r| r.try_get::<&str, _>(0).ok().flatten().map(str::to_string))
            .unwrap_or_default())
    }

    async fn reset(&mut self) -> Result<()> {
        // tiberius has no sp_reset_connection, and a script can change far
        // more than can be checked (USE, SET LANGUAGE/DATEFORMAT/ROWCOUNT,
        // SHOWPLAN, #temp tables, EXECUTE AS ...). Connections that ran user
        // SQL are closed; only catalog calls reuse pooled connections.
        Err(DbError::Unsupported(
            "SQL Server connections are not reused after user SQL".into(),
        ))
    }

    fn engine(&self) -> Engine {
        Engine::Mssql
    }
}

// ------------------------------------------------------------- mapping --

fn column(c: &tiberius::Column) -> Column {
    let (type_name, logical) = match c.column_type() {
        ColumnType::Bit | ColumnType::Bitn => ("bit", LogicalType::Bool),
        ColumnType::Int1 => ("tinyint", LogicalType::Int),
        ColumnType::Int2 => ("smallint", LogicalType::Int),
        ColumnType::Int4 => ("int", LogicalType::Int),
        ColumnType::Int8 => ("bigint", LogicalType::Int),
        ColumnType::Intn => ("int", LogicalType::Int),
        ColumnType::Float4 => ("real", LogicalType::Float),
        ColumnType::Float8 | ColumnType::Floatn => ("float", LogicalType::Float),
        ColumnType::Money => ("money", LogicalType::Decimal),
        ColumnType::Money4 => ("smallmoney", LogicalType::Decimal),
        ColumnType::Decimaln => ("decimal", LogicalType::Decimal),
        ColumnType::Numericn => ("numeric", LogicalType::Decimal),
        ColumnType::Guid => ("uniqueidentifier", LogicalType::Uuid),
        ColumnType::Daten => ("date", LogicalType::Date),
        ColumnType::Timen => ("time", LogicalType::Time),
        ColumnType::Datetime4 => ("smalldatetime", LogicalType::Timestamp),
        ColumnType::Datetime | ColumnType::Datetimen => ("datetime", LogicalType::Timestamp),
        ColumnType::Datetime2 => ("datetime2", LogicalType::Timestamp),
        ColumnType::DatetimeOffsetn => ("datetimeoffset", LogicalType::TimestampTz),
        ColumnType::BigVarChar => ("varchar", LogicalType::Text),
        ColumnType::BigChar => ("char", LogicalType::Text),
        ColumnType::NVarchar => ("nvarchar", LogicalType::Text),
        ColumnType::NChar => ("nchar", LogicalType::Text),
        ColumnType::Text => ("text", LogicalType::Text),
        ColumnType::NText => ("ntext", LogicalType::Text),
        ColumnType::Xml => ("xml", LogicalType::Text),
        ColumnType::BigVarBin => ("varbinary", LogicalType::Bytes),
        ColumnType::BigBinary => ("binary", LogicalType::Bytes),
        ColumnType::Image => ("image", LogicalType::Bytes),
        // hierarchyid, geometry, geography: their serialized bytes. tiberius
        // does not pass the UDT's own name on.
        ColumnType::Udt => ("udt", LogicalType::Bytes),
        ColumnType::SSVariant => ("sql_variant", LogicalType::Other),
        other => {
            return Column {
                name: column_name(c),
                type_name: format!("{other:?}").to_lowercase(),
                logical: LogicalType::Other,
            };
        }
    };
    Column {
        name: column_name(c),
        type_name: type_name.to_string(),
        logical,
    }
}

/// An expression without an alias has no name; SSMS shows this instead.
fn column_name(c: &tiberius::Column) -> String {
    match c.name() {
        "" => "(No column name)".to_string(),
        name => name.to_string(),
    }
}

/// `hh:mm:ss` and `digits` fraction digits of `nanos` (always a whole number
/// of the type's own increments, so this is exact).
fn time_text(t: chrono::NaiveTime, digits: u8) -> String {
    use chrono::Timelike;
    let hms = t.format("%H:%M:%S");
    match digits.min(9) {
        0 => hms.to_string(),
        n => {
            let frac = t.nanosecond() % 1_000_000_000 / 10u32.pow(9 - u32::from(n));
            format!("{hms}.{frac:0width$}", width = usize::from(n))
        }
    }
}

/// `datetime` counts 1/300 s ticks; SQL Server shows (and parses back) them
/// rounded to milliseconds: .997, not .996666666.
fn datetime_text(v: chrono::NaiveDateTime) -> String {
    use chrono::Timelike;
    let ms = (v.nanosecond() % 1_000_000_000 + 500_000) / 1_000_000;
    let v = v.with_nanosecond(0).unwrap_or(v) + chrono::TimeDelta::milliseconds(ms.into());
    format!(
        "{}.{:03}",
        v.format("%Y-%m-%dT%H:%M:%S"),
        v.nanosecond() / 1_000_000
    )
}

/// A decimal's exact digits; tiberius prints `0.0` for scale 0.
fn numeric_text(n: tiberius::numeric::Numeric) -> String {
    if n.scale() == 0 {
        n.value().to_string()
    } else {
        n.to_string()
    }
}

fn cell(d: &ColumnData<'static>) -> Value {
    fn chrono_cell<T: for<'a> FromSql<'a>>(
        d: &ColumnData<'static>,
        fmt: impl Fn(T) -> String,
    ) -> Value {
        match T::from_sql(d) {
            Ok(Some(v)) => text(fmt(v)),
            Ok(None) => Value::Null,
            Err(e) => text(format!("<{e}>")),
        }
    }
    match d {
        ColumnData::U8(v) => v.map(|v| int(v.into())).unwrap_or(Value::Null),
        ColumnData::I16(v) => v.map(|v| int(v.into())).unwrap_or(Value::Null),
        ColumnData::I32(v) => v.map(|v| int(v.into())).unwrap_or(Value::Null),
        ColumnData::I64(v) => v.map(int).unwrap_or(Value::Null),
        ColumnData::F32(v) => v.map(float32).unwrap_or(Value::Null),
        ColumnData::F64(v) => v.map(float).unwrap_or(Value::Null),
        ColumnData::Bit(v) => v.map(Value::Bool).unwrap_or(Value::Null),
        ColumnData::String(v) => v.as_ref().map(|s| text(s.as_ref())).unwrap_or(Value::Null),
        ColumnData::Guid(v) => v.map(|g| text(g.to_string())).unwrap_or(Value::Null),
        ColumnData::Binary(v) => v.as_ref().map(|b| bytes(b)).unwrap_or(Value::Null),
        // decimal, numeric, and money (decoded exactly with scale 4)
        ColumnData::Numeric(v) => v.map(|n| text(numeric_text(n))).unwrap_or(Value::Null),
        ColumnData::Xml(v) => v
            .as_ref()
            .map(|x| text(x.to_string()))
            .unwrap_or(Value::Null),
        ColumnData::Date(_) => chrono_cell::<chrono::NaiveDate>(d, |v| v.to_string()),
        // time, datetime2 and datetimeoffset print the column's own scale.
        ColumnData::Time(t) => {
            let scale = t.map_or(0, |t| t.scale());
            chrono_cell::<chrono::NaiveTime>(d, |v| time_text(v, scale))
        }
        ColumnData::DateTime(_) => chrono_cell::<chrono::NaiveDateTime>(d, datetime_text),
        ColumnData::SmallDateTime(_) => {
            chrono_cell::<chrono::NaiveDateTime>(d, |v| v.format("%Y-%m-%dT%H:%M:%S").to_string())
        }
        ColumnData::DateTime2(dt) => {
            let scale = dt.map_or(0, |dt| dt.time().scale());
            chrono_cell::<chrono::NaiveDateTime>(d, |v| {
                format!("{}T{}", v.format("%Y-%m-%d"), time_text(v.time(), scale))
            })
        }
        ColumnData::DateTimeOffset(dto) => {
            let scale = dto.map_or(0, |dto| dto.datetime2().time().scale());
            chrono_cell::<chrono::DateTime<chrono::FixedOffset>>(d, |v| {
                format!(
                    "{}T{}{}",
                    v.format("%Y-%m-%d"),
                    time_text(v.time(), scale),
                    v.format("%:z")
                )
            })
        }
    }
}

fn db_err(e: tiberius::error::Error) -> DbError {
    match &e {
        tiberius::error::Error::Server(t) => DbError::Database {
            code: Some(t.code().to_string()),
            message: ms_message(&e),
        },
        tiberius::error::Error::Io { .. } => DbError::Lost(e.to_string()),
        _ => DbError::Database {
            code: None,
            message: e.to_string(),
        },
    }
}

/// `Msg 208, Level 16, State 1, Line 1: Invalid object name 'x'.` (SSMS style).
fn ms_message(e: &tiberius::error::Error) -> String {
    match e {
        tiberius::error::Error::Server(t) => server_message(t, 0),
        tiberius::error::Error::Tls(msg) => tls_message(msg),
        other => other.to_string(),
    }
}

/// A server error, SSMS style. Inside a procedure, the line counts from the
/// procedure's start and the procedure is named; otherwise it counts from
/// the batch's, and `line_offset` (the batch's first script line less one)
/// makes it a script line.
fn server_message(t: &tiberius::error::TokenError, line_offset: u32) -> String {
    let at = if t.procedure().is_empty() {
        format!("Line {}", t.line().saturating_add(line_offset))
    } else {
        format!("Procedure {}, Line {}", t.procedure(), t.line())
    };
    format!(
        "Msg {}, Level {}, State {}, {at}: {}",
        t.code(),
        t.class(),
        t.state(),
        t.message()
    )
}

/// tiberius explains TLS failures in terms of its own API; say what to do
/// in the profile instead.
fn tls_message(raw: &str) -> String {
    let reason = if raw.contains("UnsupportedCertVersion") {
        "it is an old-style (X.509 v1) certificate, like the self-signed one SQL Server generates when no certificate is configured"
    } else if raw.contains("UnknownIssuer") {
        "it is not issued by a CA this service host trusts"
    } else if raw.contains("NotValidForName") {
        "it is not issued for the host name in this profile"
    } else if raw.contains("Expired") {
        "it has expired"
    } else {
        return format!("TLS with SQL Server failed: {raw}");
    };
    format!(
        "SQL Server's TLS certificate was rejected: {reason}. Give SQL Server a certificate \
         from a CA the service host trusts (and use the name it was issued for), or, for \
         test servers, enable \"Trust server certificate\" in this profile."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(host: &str, port: u16, instance: Option<&str>) -> Target {
        Target::parse(&MssqlParams {
            host: host.into(),
            port,
            instance: instance.map(String::from),
            database: None,
            auth: MssqlAuth::Sql { user: "u".into() },
            encrypt: MssqlEncrypt::Required,
            trust_server_certificate: false,
        })
    }

    fn t(host: &str, port: u16, instance: Option<&str>) -> Target {
        Target {
            host: host.into(),
            port,
            instance: instance.map(String::from),
        }
    }

    #[test]
    fn temporal_and_decimal_text() {
        use chrono::{NaiveDate, NaiveTime};
        let t = NaiveTime::from_hms_nano_opt(23, 59, 59, 123_456_700).unwrap();
        assert_eq!(time_text(t, 7), "23:59:59.1234567");
        assert_eq!(time_text(t, 3), "23:59:59.123");
        assert_eq!(time_text(t, 0), "23:59:59");
        let day = NaiveDate::from_ymd_opt(2024, 2, 29).unwrap();
        for (nanos, want) in [
            (996_666_666, "2024-02-29T13:45:12.997"),
            (3_333_333, "2024-02-29T13:45:12.003"),
            (0, "2024-02-29T13:45:12.000"),
            (999_600_000, "2024-02-29T13:45:13.000"),
        ] {
            let v = day.and_hms_nano_opt(13, 45, 12, nanos).unwrap();
            assert_eq!(datetime_text(v), want);
        }
        use tiberius::numeric::Numeric;
        assert_eq!(numeric_text(Numeric::new_with_scale(0, 0)), "0");
        assert_eq!(numeric_text(Numeric::new_with_scale(-42, 0)), "-42");
        assert_eq!(numeric_text(Numeric::new_with_scale(11000, 4)), "1.1000");
        assert_eq!(numeric_text(Numeric::new_with_scale(-5, 1)), "-0.5");
    }

    #[test]
    fn tls_failures_say_what_to_change() {
        let m = tls_message(
            "the server's certificate was rejected: Other(OtherError(UnsupportedCertVersion)). … Config::trust_cert() …",
        );
        assert!(
            m.contains("X.509 v1") && m.contains("Trust server certificate"),
            "{m}"
        );
        assert!(!m.contains("Config::"), "{m}");
        assert!(tls_message("weird").contains("weird"));
    }

    #[test]
    fn plain_host_and_port() {
        assert_eq!(target("db1", 1433, None), t("db1", 1433, None));
        assert_eq!(target(" db1 ", 5000, None), t("db1", 5000, None));
    }

    #[test]
    fn named_instances_go_through_sql_browser() {
        assert_eq!(
            target("db1", 1433, Some("SQLEXPRESS")),
            t("db1", 1434, Some("SQLEXPRESS"))
        );
        assert_eq!(
            target(r"db1\SQLEXPRESS", 1433, None),
            t("db1", 1434, Some("SQLEXPRESS"))
        );
        assert_eq!(
            target(r".\SQLEXPRESS", 1433, None),
            t("localhost", 1434, Some("SQLEXPRESS"))
        );
        // The instance field wins over the one pasted into the host.
        assert_eq!(target(r"db1\A", 1433, Some("B")), t("db1", 1434, Some("B")));
        // Blank instance field means none.
        assert_eq!(target("db1", 1433, Some("  ")), t("db1", 1433, None));
    }

    #[test]
    fn explicit_ports_skip_sql_browser() {
        assert_eq!(target("db1,14330", 1433, Some("X")), t("db1", 14330, None));
        assert_eq!(target("tcp:db1,1500", 1433, None), t("db1", 1500, None));
        assert_eq!(target("db1", 14330, Some("X")), t("db1", 14330, None));
    }
}
