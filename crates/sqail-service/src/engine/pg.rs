//! PostgreSQL via tokio-postgres.
//!
//! Scripts without parameters use the simple-query protocol: several
//! statements per request, text-format values (typed through the column OIDs
//! our vendored tokio-postgres exposes) and column headers even for empty
//! results. Parameterised statements use the extended protocol and binary
//! decoding.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::{StreamExt, TryStreamExt};
use serde_json::Value;
use sqail_proto::{Column, Engine, LogicalType, Param, PgSslMode, PostgresParams};
use tokio::sync::mpsc;
use tokio_postgres::types::{FromSql, ToSql, Type};
use tokio_postgres::{AsyncMessage, CancelToken, Client, SimpleQueryMessage};
use tokio_postgres_rustls::MakeRustlsConnect;

use super::value::{bytes, float, int, text};
use super::{
    Cancel, Canceller, Conn, DbError, Driver, EventSink, ExecRequest, Result, ResultWriter,
    tls_client,
};

pub struct PgDriver {
    config: tokio_postgres::Config,
    tls: MakeRustlsConnect,
    read_only: bool,
}

impl PgDriver {
    pub fn new(
        p: &PostgresParams,
        password: Option<&str>,
        ssl_client_key: Option<&str>,
        read_only: bool,
    ) -> Result<Self> {
        let mut config = tokio_postgres::Config::new();
        config
            .host(&p.host)
            .port(p.port)
            .dbname(&p.database)
            .user(&p.user)
            .application_name("sqail")
            .connect_timeout(Duration::from_secs(10))
            .ssl_mode(match p.ssl_mode {
                PgSslMode::Disable => tokio_postgres::config::SslMode::Disable,
                PgSslMode::Prefer => tokio_postgres::config::SslMode::Prefer,
                PgSslMode::Require | PgSslMode::VerifyCa | PgSslMode::VerifyFull => {
                    tokio_postgres::config::SslMode::Require
                }
            });
        if let Some(pw) = password {
            config.password(pw);
        }
        let tls = tls_client::for_postgres(tls_client::PgTls {
            mode: p.ssl_mode,
            root_pem: p.ssl_root_cert.as_deref(),
            client_cert_pem: p.ssl_client_cert.as_deref(),
            client_key_pem: ssl_client_key,
        })
        .map_err(DbError::Invalid)?;
        Ok(Self {
            config,
            tls: MakeRustlsConnect::new((*tls).clone()),
            read_only,
        })
    }
}

#[async_trait]
impl Driver for PgDriver {
    async fn connect(&self) -> Result<Box<dyn Conn>> {
        let (client, mut connection) = self
            .config
            .connect(self.tls.clone())
            .await
            .map_err(|e| DbError::Connect(pg_message(&e)))?;

        // Drive the connection; forward NOTICE/WARNING messages to the conn.
        let (notice_tx, notices) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut messages = futures::stream::poll_fn(move |cx| connection.poll_message(cx));
            while let Some(msg) = messages.next().await {
                match msg {
                    Ok(AsyncMessage::Notice(n)) => {
                        let _ = notice_tx.send((n.severity().to_string(), n.message().to_string()));
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::debug!(error = %e, "postgres connection closed");
                        break;
                    }
                }
            }
        });

        if self.read_only {
            client
                .batch_execute("SET default_transaction_read_only = on")
                .await
                .map_err(|e| DbError::Connect(pg_message(&e)))?;
        }
        Ok(Box::new(PgConn {
            cancel: Arc::new(PgCancel {
                token: client.cancel_token(),
                tls: self.tls.clone(),
            }),
            client,
            notices,
        }))
    }

    fn engine(&self) -> Engine {
        Engine::Postgres
    }
}

struct PgConn {
    client: Client,
    notices: mpsc::UnboundedReceiver<(String, String)>,
    cancel: Arc<PgCancel>,
}

struct PgCancel {
    token: CancelToken,
    tls: MakeRustlsConnect,
}

#[async_trait]
impl Cancel for PgCancel {
    async fn cancel(&self) -> bool {
        match self.token.cancel_query(self.tls.clone()).await {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!(error = %e, "postgres cancel request failed");
                false
            }
        }
    }
}

impl PgConn {
    async fn forward_notices(&mut self, out: &mut ResultWriter<'_>) -> Result<()> {
        while let Ok((severity, message)) = self.notices.try_recv() {
            out.message(&severity.to_lowercase(), message).await?;
        }
        Ok(())
    }

    async fn run_simple(&mut self, sql: &str, out: &mut ResultWriter<'_>) -> Result<()> {
        let stream = self.client.simple_query_raw(sql).await.map_err(db_err)?;
        futures::pin_mut!(stream);
        let mut columns: Vec<LogicalType> = Vec::new();
        loop {
            let item = stream.try_next().await;
            self.forward_notices(out).await?;
            match item.map_err(db_err)? {
                None => break,
                Some(SimpleQueryMessage::RowDescription(cols)) => {
                    let meta: Vec<Column> = cols
                        .iter()
                        .map(|c| column(c.name(), c.type_oid()))
                        .collect();
                    columns = meta.iter().map(|c| c.logical).collect();
                    out.start(meta).await?;
                }
                Some(SimpleQueryMessage::Row(row)) => {
                    let cells = columns
                        .iter()
                        .enumerate()
                        .map(|(i, &logical)| match row.get(i) {
                            None => Value::Null,
                            Some(s) => text_cell(logical, s),
                        })
                        .collect();
                    out.row(cells).await?;
                }
                Some(SimpleQueryMessage::CommandComplete(n)) => {
                    if out.is_open() {
                        out.end().await?;
                    } else {
                        out.rows_affected(n).await?;
                    }
                }
                Some(_) => {}
            }
        }
        self.forward_notices(out).await
    }

    async fn run_prepared(
        &mut self,
        sql: &str,
        params: &[Param],
        out: &mut ResultWriter<'_>,
    ) -> Result<()> {
        let stmt = self.client.prepare(sql).await.map_err(db_err)?;
        if stmt.params().len() != params.len() {
            return Err(DbError::Invalid(format!(
                "statement expects {} parameter(s), got {}",
                stmt.params().len(),
                params.len()
            )));
        }
        let bound = stmt
            .params()
            .iter()
            .zip(params)
            .enumerate()
            .map(|(i, (ty, p))| bind(i + 1, ty, p))
            .collect::<Result<Vec<_>>>()?;
        let types: Vec<Type> = stmt.columns().iter().map(|c| c.type_().clone()).collect();
        if !types.is_empty() {
            out.start(
                stmt.columns()
                    .iter()
                    .map(|c| column(c.name(), c.type_().oid()))
                    .collect(),
            )
            .await?;
        }
        let rows = self
            .client
            .query_raw(
                &stmt,
                bound.iter().map(|b| b.as_ref() as &(dyn ToSql + Sync)),
            )
            .await
            .map_err(db_err)?;
        futures::pin_mut!(rows);
        while let Some(row) = rows.try_next().await.map_err(db_err)? {
            let cells = types
                .iter()
                .enumerate()
                .map(|(i, ty)| binary_cell(&row, i, ty))
                .collect::<Result<Vec<_>>>()?;
            out.row(cells).await?;
        }
        self.forward_notices(out).await?;
        if types.is_empty() {
            out.rows_affected(rows.rows_affected().unwrap_or(0)).await
        } else {
            out.end().await
        }
    }
}

#[async_trait]
impl Conn for PgConn {
    async fn execute(&mut self, req: &ExecRequest, sink: &EventSink) -> Result<()> {
        let mut out = ResultWriter::new(sink, req.max_rows);
        let res = if req.params.is_empty() {
            self.run_simple(&req.sql, &mut out).await
        } else {
            self.run_prepared(&req.sql, &req.params, &mut out).await
        };
        // Close a result interrupted by an error so clients see its rows.
        out.end().await?;
        res
    }

    fn canceller(&self) -> Canceller {
        self.cancel.clone()
    }

    async fn in_transaction(&mut self) -> Result<bool> {
        // In a transaction block, now() is the transaction start and differs
        // from the start of this statement. A failed transaction rejects
        // every statement (SQLSTATE 25P02) and also needs a ROLLBACK.
        match self
            .client
            .simple_query("SELECT now() <> statement_timestamp()")
            .await
        {
            Ok(msgs) => Ok(msgs
                .iter()
                .any(|m| matches!(m, SimpleQueryMessage::Row(r) if r.get(0) == Some("t")))),
            Err(e)
                if e.code()
                    == Some(&tokio_postgres::error::SqlState::IN_FAILED_SQL_TRANSACTION) =>
            {
                Ok(true)
            }
            Err(e) => Err(db_err(e)),
        }
    }

    async fn ping(&mut self) -> Result<()> {
        self.client.simple_query("").await.map_err(db_err)?;
        Ok(())
    }

    async fn server_version(&mut self) -> Result<String> {
        let msgs = self
            .client
            .simple_query("SELECT version()")
            .await
            .map_err(db_err)?;
        Ok(msgs
            .iter()
            .find_map(|m| match m {
                SimpleQueryMessage::Row(r) => r.get(0).map(str::to_string),
                _ => None,
            })
            .unwrap_or_default())
    }

    fn engine(&self) -> Engine {
        Engine::Postgres
    }
}

// ------------------------------------------------------------- mapping --

fn column(name: &str, oid: u32) -> Column {
    let (type_name, logical) = match Type::from_oid(oid) {
        Some(ty) => (ty.name().to_string(), logical(&ty)),
        None => (format!("oid:{oid}"), LogicalType::Other),
    };
    Column {
        name: name.to_string(),
        type_name,
        logical,
    }
}

fn logical(ty: &Type) -> LogicalType {
    match *ty {
        Type::BOOL => LogicalType::Bool,
        Type::INT2 | Type::INT4 | Type::INT8 | Type::OID => LogicalType::Int,
        Type::FLOAT4 | Type::FLOAT8 => LogicalType::Float,
        Type::NUMERIC => LogicalType::Decimal,
        Type::TEXT
        | Type::VARCHAR
        | Type::BPCHAR
        | Type::NAME
        | Type::UNKNOWN
        | Type::CHAR
        | Type::XML
        | Type::MONEY => LogicalType::Text,
        Type::BYTEA => LogicalType::Bytes,
        Type::DATE => LogicalType::Date,
        Type::TIME | Type::TIMETZ => LogicalType::Time,
        Type::TIMESTAMP => LogicalType::Timestamp,
        Type::TIMESTAMPTZ => LogicalType::TimestampTz,
        Type::UUID => LogicalType::Uuid,
        Type::JSON | Type::JSONB => LogicalType::Json,
        _ => LogicalType::Other,
    }
}

/// Convert a simple-protocol (text format) value.
fn text_cell(logical: LogicalType, s: &str) -> Value {
    match logical {
        LogicalType::Bool => Value::Bool(s == "t"),
        LogicalType::Int => s.parse::<i64>().map(int).unwrap_or_else(|_| text(s)),
        LogicalType::Float => s.parse::<f64>().map(float).unwrap_or_else(|_| text(s)),
        LogicalType::Bytes => text(s.strip_prefix("\\x").unwrap_or(s)),
        _ => text(s),
    }
}

/// Convert an extended-protocol (binary format) value.
fn binary_cell(row: &tokio_postgres::Row, i: usize, ty: &Type) -> Result<Value> {
    fn get<'a, T: FromSql<'a>>(row: &'a tokio_postgres::Row, i: usize) -> Result<Option<T>> {
        row.try_get::<_, Option<T>>(i).map_err(db_err)
    }
    let v = match *ty {
        Type::BOOL => get::<bool>(row, i)?.map(Value::Bool),
        Type::INT2 => get::<i16>(row, i)?.map(|v| int(v.into())),
        Type::INT4 => get::<i32>(row, i)?.map(|v| int(v.into())),
        Type::INT8 => get::<i64>(row, i)?.map(int),
        Type::OID => get::<u32>(row, i)?.map(|v| int(v.into())),
        Type::FLOAT4 => get::<f32>(row, i)?.map(|v| float(v.into())),
        Type::FLOAT8 => get::<f64>(row, i)?.map(float),
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::UNKNOWN | Type::XML => {
            get::<String>(row, i)?.map(text)
        }
        Type::BYTEA => get::<Vec<u8>>(row, i)?.map(|b| bytes(&b)),
        Type::UUID => get::<uuid::Uuid>(row, i)?.map(|u| text(u.to_string())),
        Type::JSON | Type::JSONB => get::<Value>(row, i)?.map(|j| text(j.to_string())),
        Type::DATE => get::<chrono::NaiveDate>(row, i)?.map(|d| text(d.to_string())),
        Type::TIME => get::<chrono::NaiveTime>(row, i)?.map(|t| text(t.to_string())),
        Type::TIMESTAMP => get::<chrono::NaiveDateTime>(row, i)?
            .map(|t| text(t.format("%Y-%m-%dT%H:%M:%S%.f").to_string())),
        Type::TIMESTAMPTZ => get::<chrono::DateTime<chrono::Utc>>(row, i)?
            .map(|t| text(t.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true))),
        Type::NUMERIC => get::<Numeric>(row, i)?.map(|n| text(n.0)),
        _ => get::<Raw>(row, i)?.map(|r| r.0),
    };
    Ok(v.unwrap_or(Value::Null))
}

/// Bind a JSON parameter to the type Postgres inferred for the placeholder.
fn bind(n: usize, ty: &Type, p: &Param) -> Result<Box<dyn ToSql + Sync + Send>> {
    let bad = || {
        DbError::Invalid(format!(
            "parameter ${n}: cannot bind {p:?} to {}; cast the placeholder, e.g. ${n}::text",
            ty.name()
        ))
    };
    Ok(match (p, ty) {
        (Param::Null, _) => Box::new(None::<String>) as Box<dyn ToSql + Sync + Send>,
        (Param::Bool(b), &Type::BOOL) => Box::new(*b),
        (Param::Int(v), &Type::INT2) => Box::new(i16::try_from(*v).map_err(|_| bad())?),
        (Param::Int(v), &Type::INT4) => Box::new(i32::try_from(*v).map_err(|_| bad())?),
        (Param::Int(v), &Type::INT8) => Box::new(*v),
        (Param::Int(v), &Type::FLOAT8) => Box::new(*v as f64),
        (Param::Float(v), &Type::FLOAT8) => Box::new(*v),
        (Param::Float(v), &Type::FLOAT4) => Box::new(*v as f32),
        (Param::Text(s), &Type::TEXT | &Type::VARCHAR | &Type::BPCHAR | &Type::NAME) => {
            Box::new(s.clone())
        }
        (Param::Text(s), &Type::UUID) => Box::new(s.parse::<uuid::Uuid>().map_err(|_| bad())?),
        (Param::Text(s), &Type::JSON | &Type::JSONB) => {
            Box::new(serde_json::from_str::<Value>(s).map_err(|_| bad())?)
        }
        _ => return Err(bad()),
    })
}

/// Any type we do not decode natively: enums are UTF-8, the rest is shown as
/// a byte count (run without parameters to get the text representation).
struct Raw(Value);

impl<'a> FromSql<'a> for Raw {
    fn from_sql(
        ty: &Type,
        raw: &'a [u8],
    ) -> std::result::Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Raw(match ty.kind() {
            tokio_postgres::types::Kind::Enum(_) => text(String::from_utf8_lossy(raw)),
            _ => text(format!("<{} binary, {} bytes>", ty.name(), raw.len())),
        }))
    }
    fn accepts(_: &Type) -> bool {
        true
    }
}

/// `numeric` in its binary wire format, rendered as an exact decimal string.
struct Numeric(String);

impl<'a> FromSql<'a> for Numeric {
    fn from_sql(
        _: &Type,
        raw: &'a [u8],
    ) -> std::result::Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Numeric(numeric_to_string(raw).ok_or("malformed numeric")?))
    }
    fn accepts(ty: &Type) -> bool {
        *ty == Type::NUMERIC
    }
}

fn numeric_to_string(raw: &[u8]) -> Option<String> {
    let u16_at = |i: usize| raw.get(i..i + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
    let ndigits = u16_at(0)? as usize;
    let weight = u16_at(2)? as i16 as i32;
    let sign = u16_at(4)?;
    let dscale = u16_at(6)? as usize;
    match sign {
        0xC000 => return Some("NaN".into()),
        0xD000 => return Some("Infinity".into()),
        0xF000 => return Some("-Infinity".into()),
        _ => {}
    }
    let digits: Vec<u16> = (0..ndigits)
        .map(|i| u16_at(8 + i * 2))
        .collect::<Option<_>>()?;
    let digit = |pos: i32| -> u16 {
        usize::try_from(pos)
            .ok()
            .and_then(|p| digits.get(p))
            .copied()
            .unwrap_or(0)
    };
    let mut int_part = String::new();
    for pos in 0..=weight {
        let d = digit(pos);
        if int_part.is_empty() {
            if d != 0 {
                int_part = d.to_string();
            }
        } else {
            int_part.push_str(&format!("{d:04}"));
        }
    }
    if int_part.is_empty() {
        int_part.push('0');
    }
    let mut frac = String::new();
    let mut pos = weight + 1;
    while frac.len() < dscale {
        frac.push_str(&format!("{:04}", digit(pos)));
        pos += 1;
    }
    frac.truncate(dscale);
    let mut s = String::new();
    if sign == 0x4000 {
        s.push('-');
    }
    s.push_str(&int_part);
    if dscale > 0 {
        s.push('.');
        s.push_str(&frac);
    }
    Some(s)
}

fn db_err(e: tokio_postgres::Error) -> DbError {
    match e.as_db_error() {
        Some(db) => DbError::Database {
            code: Some(db.code().code().to_string()),
            message: pg_message(&e),
        },
        None if e.is_closed() => DbError::Connect("connection closed".into()),
        None => DbError::Database {
            code: None,
            message: e.to_string(),
        },
    }
}

/// `ERROR: message` plus detail/hint/position, like psql prints it.
fn pg_message(e: &tokio_postgres::Error) -> String {
    let Some(db) = e.as_db_error() else {
        return e.to_string();
    };
    let mut msg = format!("{}: {}", db.severity(), db.message());
    if let Some(d) = db.detail() {
        msg.push_str(&format!("\nDETAIL: {d}"));
    }
    if let Some(h) = db.hint() {
        msg.push_str(&format!("\nHINT: {h}"));
    }
    if let Some(tokio_postgres::error::ErrorPosition::Original(p)) = db.position() {
        msg.push_str(&format!("\nPOSITION: {p}"));
    }
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(ndigits: &[u16], weight: i16, sign: u16, dscale: u16) -> Vec<u8> {
        let mut v = Vec::new();
        for x in [ndigits.len() as u16, weight as u16, sign, dscale] {
            v.extend_from_slice(&x.to_be_bytes());
        }
        for d in ndigits {
            v.extend_from_slice(&d.to_be_bytes());
        }
        v
    }

    #[test]
    fn numeric_decoding() {
        // 12345678.9 = [1234, 5678, 9000], weight 1, dscale 1
        assert_eq!(
            numeric_to_string(&enc(&[1234, 5678, 9000], 1, 0, 1)).unwrap(),
            "12345678.9"
        );
        // -0.0012 = [12], weight -1, dscale 4
        assert_eq!(
            numeric_to_string(&enc(&[12], -1, 0x4000, 4)).unwrap(),
            "-0.0012"
        );
        // 10000 = [1], weight 1, dscale 0
        assert_eq!(numeric_to_string(&enc(&[1], 1, 0, 0)).unwrap(), "10000");
        // 0.00 = no digits, dscale 2
        assert_eq!(numeric_to_string(&enc(&[], 0, 0, 2)).unwrap(), "0.00");
        assert_eq!(numeric_to_string(&enc(&[], 0, 0xC000, 0)).unwrap(), "NaN");
    }

    #[test]
    fn text_cells() {
        assert_eq!(text_cell(LogicalType::Bool, "t"), Value::Bool(true));
        assert_eq!(text_cell(LogicalType::Int, "42"), serde_json::json!(42));
        assert_eq!(
            text_cell(LogicalType::Float, "NaN"),
            serde_json::json!("NaN")
        );
        assert_eq!(
            text_cell(LogicalType::Bytes, "\\xdeadbeef"),
            serde_json::json!("deadbeef")
        );
        assert_eq!(
            text_cell(LogicalType::Decimal, "1.50"),
            serde_json::json!("1.50")
        );
    }
}
