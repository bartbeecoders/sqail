//! Microsoft SQL Server via tiberius.
//!
//! Scripts are split on `GO` here (the server does not understand it). Each
//! batch runs as one request with multiple result sets. tiberius does not
//! surface DONE row counts on result streams, so a batch that is a single
//! INSERT/UPDATE/DELETE/MERGE runs via `execute` to report affected rows.
//! Cancelling sends `KILL <spid>` from a side connection (needs ALTER ANY
//! CONNECTION); the caller also drops the connection if that does not stop it.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::TryStreamExt;
use serde_json::Value;
use sqail_proto::{Column, Engine, LogicalType, MssqlAuth, MssqlEncrypt, MssqlParams, Param};
use tiberius::{AuthMethod, ColumnData, ColumnType, Config, EncryptionLevel, FromSql, QueryItem};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

use super::entra::Entra;
use super::split::{shape, split_go};
use super::value::{bytes, float, int, text};
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
        let spid = client
            .simple_query("SELECT CAST(@@SPID AS int)")
            .await
            .map_err(db_err)?
            .into_row()
            .await
            .map_err(db_err)?
            .and_then(|r| r.get::<i32, _>(0))
            .unwrap_or(0);
        Ok(Box::new(MssqlConn {
            client,
            cancel: Arc::new(MssqlCancel {
                driver: self.clone(),
                spid,
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
}

struct MssqlCancel {
    driver: MssqlDriver,
    spid: i32,
}

#[async_trait]
impl Cancel for MssqlCancel {
    async fn cancel(&self) -> bool {
        if self.spid <= 0 {
            return false;
        }
        let res = async {
            let mut side = self.driver.open().await?;
            side.simple_query(format!("KILL {}", self.spid))
                .await
                .map_err(db_err)?
                .into_results()
                .await
                .map_err(db_err)
        }
        .await;
        match res {
            Ok(_) => true,
            Err(e) => {
                tracing::info!(spid = self.spid, error = %e, "KILL failed; dropping the connection instead");
                false
            }
        }
    }
}

async fn stream_results(
    mut stream: tiberius::QueryStream<'_>,
    out: &mut ResultWriter<'_>,
) -> Result<()> {
    while let Some(item) = stream.try_next().await.map_err(db_err)? {
        match item {
            QueryItem::Metadata(meta) => {
                out.start(meta.columns().iter().map(column).collect())
                    .await?
            }
            QueryItem::Row(row) => out.row(row.cells().map(|(_, d)| cell(d)).collect()).await?,
        }
    }
    out.end().await
}

impl MssqlConn {
    async fn run_batch(
        &mut self,
        sql: &str,
        params: &[Param],
        out: &mut ResultWriter<'_>,
    ) -> Result<()> {
        let results_before = out.results_seen();
        if shape(sql).is_single_dml() {
            let mut q = tiberius::Query::new(sql);
            bind_all(&mut q, params);
            let res = q.execute(&mut self.client).await.map_err(db_err)?;
            return out.rows_affected(res.total()).await;
        }
        let stream = if params.is_empty() {
            self.client.simple_query(sql).await.map_err(db_err)?
        } else {
            let mut q = tiberius::Query::new(sql);
            bind_all(&mut q, params);
            q.query(&mut self.client).await.map_err(db_err)?
        };
        stream_results(stream, out).await?;
        if out.results_seen() == results_before {
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
        let res = async {
            if !req.params.is_empty() {
                return self.run_batch(&req.sql, &req.params, &mut out).await;
            }
            for batch in split_go(&req.sql) {
                for _ in 0..batch.repeat {
                    self.run_batch(&batch.sql, &[], &mut out).await?;
                }
            }
            Ok(())
        }
        .await;
        out.end().await?;
        res
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
        Ok(row.and_then(|r| r.get::<i32, _>(0)).unwrap_or(0) > 0)
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
            .and_then(|r| r.get::<&str, _>(0).map(str::to_string))
            .unwrap_or_default())
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
        ColumnType::Money | ColumnType::Money4 => ("money", LogicalType::Decimal),
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
        other => {
            return Column {
                name: c.name().to_string(),
                type_name: format!("{other:?}").to_lowercase(),
                logical: LogicalType::Other,
            };
        }
    };
    Column {
        name: c.name().to_string(),
        type_name: type_name.to_string(),
        logical,
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
        ColumnData::F32(v) => v.map(|v| float(v.into())).unwrap_or(Value::Null),
        ColumnData::F64(v) => v.map(float).unwrap_or(Value::Null),
        ColumnData::Bit(v) => v.map(Value::Bool).unwrap_or(Value::Null),
        ColumnData::String(v) => v.as_ref().map(|s| text(s.as_ref())).unwrap_or(Value::Null),
        ColumnData::Guid(v) => v.map(|g| text(g.to_string())).unwrap_or(Value::Null),
        ColumnData::Binary(v) => v.as_ref().map(|b| bytes(b)).unwrap_or(Value::Null),
        ColumnData::Numeric(v) => v.map(|n| text(n.to_string())).unwrap_or(Value::Null),
        ColumnData::Xml(v) => v
            .as_ref()
            .map(|x| text(x.to_string()))
            .unwrap_or(Value::Null),
        ColumnData::Date(_) => chrono_cell::<chrono::NaiveDate>(d, |v| v.to_string()),
        ColumnData::Time(_) => chrono_cell::<chrono::NaiveTime>(d, |v| v.to_string()),
        ColumnData::DateTime(_) | ColumnData::SmallDateTime(_) | ColumnData::DateTime2(_) => {
            chrono_cell::<chrono::NaiveDateTime>(d, |v| {
                v.format("%Y-%m-%dT%H:%M:%S%.f").to_string()
            })
        }
        ColumnData::DateTimeOffset(_) => {
            chrono_cell::<chrono::DateTime<chrono::FixedOffset>>(d, |v| {
                v.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, false)
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
        tiberius::error::Error::Io { .. } => DbError::Connect(e.to_string()),
        _ => DbError::Database {
            code: None,
            message: e.to_string(),
        },
    }
}

/// `Msg 208, Level 16, State 1, Line 1: Invalid object name 'x'.` (SSMS style).
fn ms_message(e: &tiberius::error::Error) -> String {
    match e {
        tiberius::error::Error::Server(t) => format!(
            "Msg {}, Level {}, State {}, Line {}: {}",
            t.code(),
            t.class(),
            t.state(),
            t.line(),
            t.message()
        ),
        tiberius::error::Error::Tls(msg) => tls_message(msg),
        other => other.to_string(),
    }
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
