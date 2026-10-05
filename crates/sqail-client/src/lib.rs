//! Typed async client for the sqail-service REST API.
//!
//! ```no_run
//! # async fn demo() -> Result<(), sqail_client::Error> {
//! use sqail_client::{Client, Target, Trust};
//! let target = Target {
//!     url: "https://127.0.0.1:7443".into(),
//!     token: "sq2_…".into(),
//!     trust: Trust::Pinned("AB:CD:…".into()),
//!     identity: None, // or Some(Identity::from_files(cert, key)?) for mTLS
//! };
//! let client = Client::new(&target)?;
//! let conns = client.connections().await?;
//! # Ok(()) }
//! ```

mod tls;

use std::pin::Pin;
use std::time::Duration;

use bytes::BytesMut;
use futures::{Stream, StreamExt};
use reqwest::{Method, RequestBuilder, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

pub use sqail_proto as proto;
use sqail_proto::*;
pub use tls::fingerprint_of;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The service answered with an error (`application/problem+json`).
    #[error("{}", .problem.detail.as_deref().unwrap_or(&.problem.title))]
    Api { status: u16, problem: Problem },
    /// The server's certificate does not match the pinned fingerprint.
    #[error("the service certificate does not match the pinned fingerprint")]
    CertificateMismatch,
    #[error("cannot reach the service: {0}")]
    Transport(String),
    #[error("unexpected response: {0}")]
    Decode(String),
    #[error("invalid configuration: {0}")]
    Config(String),
}

impl Error {
    pub fn status(&self) -> Option<u16> {
        match self {
            Error::Api { status, .. } => Some(*status),
            _ => None,
        }
    }

    fn from_reqwest(e: reqwest::Error) -> Self {
        // rustls errors are buried in the source chain.
        let mut src: Option<&dyn std::error::Error> = Some(&e);
        while let Some(s) = src {
            if s.to_string().contains(tls::MISMATCH_MARKER) {
                return Error::CertificateMismatch;
            }
            src = s.source();
        }
        if e.is_decode() {
            Error::Decode(e.to_string())
        } else {
            Error::Transport(chain(&e))
        }
    }
}

fn chain(e: &dyn std::error::Error) -> String {
    let mut msg = e.to_string();
    let mut src = e.source();
    while let Some(s) = src {
        let s_msg = s.to_string();
        if !msg.contains(&s_msg) {
            msg.push_str(": ");
            msg.push_str(&s_msg);
        }
        src = s.source();
    }
    msg
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// How to authenticate the service's TLS certificate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "fingerprint", rename_all = "snake_case")]
pub enum Trust {
    /// Accept exactly this certificate (SHA-256, `AB:CD:…`). For self-signed
    /// service certificates.
    Pinned(String),
    /// Normal verification against the OS trust store.
    System,
}

/// Where and how to reach a service.
#[derive(Debug, Clone)]
pub struct Target {
    pub url: String,
    pub token: String,
    pub trust: Trust,
    /// Client certificate for services that require mutual TLS.
    pub identity: Option<Identity>,
}

/// A client certificate chain and its private key, both PEM.
#[derive(Clone)]
pub struct Identity {
    pub cert_pem: Vec<u8>,
    pub key_pem: Vec<u8>,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the key.
        f.write_str("Identity { .. }")
    }
}

impl Identity {
    pub fn from_files(cert: &std::path::Path, key: &std::path::Path) -> Result<Self> {
        let read = |p: &std::path::Path| {
            std::fs::read(p).map_err(|e| Error::Config(format!("{}: {e}", p.display())))
        };
        Ok(Self {
            cert_pem: read(cert)?,
            key_pem: read(key)?,
        })
    }
}

/// Cheap to clone; share one per service.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: String,
    token: String,
}

/// A running query: its id (for [`Client::cancel`]) and its event stream.
pub struct QueryStream {
    pub query_id: Uuid,
    pub events: Pin<Box<dyn Stream<Item = Result<QueryEvent>> + Send>>,
}

/// Where a query runs.
#[derive(Debug, Clone, Copy)]
pub enum On {
    Connection(Uuid),
    Session(Uuid),
}

/// Connect once, unauthenticated, and report the certificate fingerprint and
/// health. Used to show the fingerprint to the user before pinning it.
pub async fn probe(url: &str) -> Result<(String, Health)> {
    let (config, seen) = tls::recording();
    let http = reqwest::Client::builder()
        .tls_backend_preconfigured(config)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(Error::from_reqwest)?;
    let res = http
        .get(format!("{}/v1/health", url.trim_end_matches('/')))
        .send()
        .await
        .map_err(Error::from_reqwest)?;
    let health = res.json::<Health>().await.map_err(Error::from_reqwest)?;
    let fp = seen
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .ok_or_else(|| Error::Transport("no certificate presented".into()))?;
    Ok((fp, health))
}

impl Client {
    pub fn new(target: &Target) -> Result<Self> {
        let url = target.url.trim_end_matches('/');
        if !url.starts_with("https://") {
            return Err(Error::Config(
                "the service URL must start with https://".into(),
            ));
        }
        let mut builder = reqwest::Client::builder();
        // `System` without a client certificate uses reqwest's own rustls
        // setup with the platform verifier; everything else is ours.
        match (&target.trust, &target.identity) {
            (Trust::System, None) => {}
            (trust, identity) => {
                let cfg = tls::config(trust, identity.as_ref()).map_err(Error::Config)?;
                builder = builder.tls_backend_preconfigured(cfg);
            }
        }
        let http = builder
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(60))
            .user_agent(concat!("sqail-client/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(Error::from_reqwest)?;
        Ok(Self {
            http,
            base: url.to_string(),
            token: target.token.clone(),
        })
    }

    fn req(&self, method: Method, path: &str) -> RequestBuilder {
        self.http
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(&self.token)
    }

    async fn send(rb: RequestBuilder) -> Result<reqwest::Response> {
        let res = rb.send().await.map_err(Error::from_reqwest)?;
        if res.status().is_success() {
            return Ok(res);
        }
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        let problem = serde_json::from_str::<Problem>(&text).unwrap_or_else(|_| Problem {
            title: status.canonical_reason().unwrap_or("Error").into(),
            status: status.as_u16(),
            code: "http_error".into(),
            detail: (!text.is_empty()).then_some(text),
        });
        Err(Error::Api {
            status: status.as_u16(),
            problem,
        })
    }

    async fn json<T: DeserializeOwned>(rb: RequestBuilder) -> Result<T> {
        let res = Self::send(rb.timeout(Duration::from_secs(60))).await?;
        res.json().await.map_err(Error::from_reqwest)
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        Self::json(self.req(Method::GET, path)).await
    }

    async fn post<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> {
        Self::json(self.req(Method::POST, path).json(body)).await
    }

    async fn delete(&self, path: &str) -> Result<()> {
        Self::send(
            self.req(Method::DELETE, path)
                .timeout(Duration::from_secs(30)),
        )
        .await?;
        Ok(())
    }

    // ------------------------------------------------------------- meta --

    pub async fn health(&self) -> Result<Health> {
        self.get("/v1/health").await
    }

    pub async fn info(&self) -> Result<ServiceInfo> {
        self.get("/v1/info").await
    }

    // ------------------------------------------------------ connections --

    pub async fn connections(&self) -> Result<Vec<Connection>> {
        self.get("/v1/connections").await
    }

    pub async fn connection(&self, id: Uuid) -> Result<Connection> {
        self.get(&format!("/v1/connections/{id}")).await
    }

    pub async fn create_connection(&self, input: &ConnectionInput) -> Result<Connection> {
        self.post("/v1/connections", input).await
    }

    pub async fn update_connection(&self, id: Uuid, input: &ConnectionInput) -> Result<Connection> {
        Self::json(
            self.req(Method::PUT, &format!("/v1/connections/{id}"))
                .json(input),
        )
        .await
    }

    pub async fn delete_connection(&self, id: Uuid) -> Result<()> {
        self.delete(&format!("/v1/connections/{id}")).await
    }

    pub async fn test_connection(&self, id: Uuid) -> Result<TestResult> {
        self.post(
            &format!("/v1/connections/{id}/test"),
            &serde_json::json!({}),
        )
        .await
    }

    /// Try `input` before saving it. With no password or client key in
    /// `input`, `secret_from` names the saved profile whose secret to use.
    pub async fn test_unsaved(
        &self,
        input: &ConnectionInput,
        secret_from: Option<Uuid>,
    ) -> Result<TestResult> {
        let path = match secret_from {
            Some(id) => format!("/v1/connections/test?secret_from={id}"),
            None => "/v1/connections/test".into(),
        };
        self.post(&path, input).await
    }

    /// Databases the login in `input` can access. With no password or client
    /// key in `input`, `secret_from` names the saved profile whose secret to use.
    pub async fn databases_unsaved(
        &self,
        input: &ConnectionInput,
        secret_from: Option<Uuid>,
    ) -> Result<Vec<NamedItem>> {
        let path = match secret_from {
            Some(id) => format!("/v1/connections/databases?secret_from={id}"),
            None => "/v1/connections/databases".into(),
        };
        self.post(&path, input).await
    }

    // ---------------------------------------------------------- queries --

    /// Start a query. Events arrive as the service produces them; the stream
    /// ends after `done`. Dropping the stream abandons the query.
    pub async fn query(&self, on: On, req: &QueryRequest) -> Result<QueryStream> {
        self.query_with_id(on, req, Uuid::new_v4()).await
    }

    pub async fn query_with_id(
        &self,
        on: On,
        req: &QueryRequest,
        query_id: Uuid,
    ) -> Result<QueryStream> {
        let path = match on {
            On::Connection(id) => format!("/v1/connections/{id}/query"),
            On::Session(id) => format!("/v1/sessions/{id}/query"),
        };
        let res = Self::send(
            self.req(Method::POST, &path)
                .header(QUERY_ID_HEADER, query_id.to_string())
                .json(req),
        )
        .await?;
        Ok(QueryStream {
            query_id,
            events: Box::pin(ndjson(res)),
        })
    }

    /// Run a query to completion and collect its events.
    pub async fn query_all(&self, on: On, req: &QueryRequest) -> Result<Vec<QueryEvent>> {
        let mut qs = self.query(on, req).await?;
        let mut out = Vec::new();
        while let Some(ev) = qs.events.next().await {
            out.push(ev?);
        }
        Ok(out)
    }

    pub async fn cancel(&self, query_id: Uuid) -> Result<()> {
        match self.delete(&format!("/v1/queries/{query_id}")).await {
            // Already finished: nothing to cancel.
            Err(Error::Api { status: 404, .. }) => Ok(()),
            other => other,
        }
    }

    /// Query plan for one statement.
    pub async fn explain(&self, on: On, sql: &str, analyze: bool) -> Result<Plan> {
        let path = match on {
            On::Connection(id) => format!("/v1/connections/{id}/explain"),
            On::Session(id) => format!("/v1/sessions/{id}/explain"),
        };
        self.post(
            &path,
            &ExplainRequest {
                sql: sql.to_string(),
                analyze,
            },
        )
        .await
    }

    // --------------------------------------------------------- sessions --

    pub async fn open_session(&self, connection_id: Uuid) -> Result<SessionInfo> {
        self.post("/v1/sessions", &CreateSession { connection_id })
            .await
    }

    pub async fn session(&self, id: Uuid) -> Result<SessionInfo> {
        self.get(&format!("/v1/sessions/{id}")).await
    }

    pub async fn close_session(&self, id: Uuid) -> Result<()> {
        match self.delete(&format!("/v1/sessions/{id}")).await {
            Err(Error::Api { status: 404, .. }) => Ok(()),
            other => other,
        }
    }

    // ----------------------------------------------------------- schema --

    fn schema_path(id: Uuid, what: &str, schema: Option<&str>, name: Option<&str>) -> String {
        let mut q = Vec::new();
        if let Some(s) = schema {
            q.push(format!("schema={}", encode(s)));
        }
        if let Some(n) = name {
            q.push(format!("name={}", encode(n)));
        }
        let qs = if q.is_empty() {
            String::new()
        } else {
            format!("?{}", q.join("&"))
        };
        format!("/v1/connections/{id}/{what}{qs}")
    }

    pub async fn databases(&self, id: Uuid) -> Result<Vec<NamedItem>> {
        self.get(&Self::schema_path(id, "schema/databases", None, None))
            .await
    }

    pub async fn schemas(&self, id: Uuid) -> Result<Vec<NamedItem>> {
        self.get(&Self::schema_path(id, "schema/schemas", None, None))
            .await
    }

    pub async fn tables(&self, id: Uuid, schema: Option<&str>) -> Result<Vec<TableInfo>> {
        self.get(&Self::schema_path(id, "schema/tables", schema, None))
            .await
    }

    pub async fn columns(
        &self,
        id: Uuid,
        schema: Option<&str>,
        table: &str,
    ) -> Result<Vec<ColumnInfo>> {
        self.get(&Self::schema_path(
            id,
            "schema/columns",
            schema,
            Some(table),
        ))
        .await
    }

    pub async fn indexes(
        &self,
        id: Uuid,
        schema: Option<&str>,
        table: &str,
    ) -> Result<Vec<IndexInfo>> {
        self.get(&Self::schema_path(
            id,
            "schema/indexes",
            schema,
            Some(table),
        ))
        .await
    }

    pub async fn foreign_keys(
        &self,
        id: Uuid,
        schema: Option<&str>,
        table: &str,
    ) -> Result<Vec<ForeignKeyInfo>> {
        self.get(&Self::schema_path(
            id,
            "schema/foreign-keys",
            schema,
            Some(table),
        ))
        .await
    }

    pub async fn privileges(
        &self,
        id: Uuid,
        schema: Option<&str>,
        table: &str,
    ) -> Result<TablePrivileges> {
        self.get(&Self::schema_path(
            id,
            "schema/privileges",
            schema,
            Some(table),
        ))
        .await
    }

    pub async fn routines(&self, id: Uuid, schema: Option<&str>) -> Result<Vec<RoutineInfo>> {
        self.get(&Self::schema_path(id, "schema/routines", schema, None))
            .await
    }

    pub async fn ddl(&self, id: Uuid, schema: Option<&str>, name: &str) -> Result<Ddl> {
        self.get(&Self::schema_path(id, "ddl", schema, Some(name)))
            .await
    }

    // ------------------------------------------------------ admin bits --

    pub async fn tokens(&self) -> Result<Vec<TokenInfo>> {
        self.get("/v1/tokens").await
    }

    pub async fn create_token(&self, name: &str, scope: Scope) -> Result<CreatedToken> {
        self.post(
            "/v1/tokens",
            &CreateToken {
                name: name.into(),
                scope,
            },
        )
        .await
    }

    pub async fn revoke_token(&self, id: Uuid) -> Result<()> {
        self.delete(&format!("/v1/tokens/{id}")).await
    }

    pub async fn audit(&self, before: Option<i64>, limit: u32) -> Result<AuditPage> {
        let mut path = format!("/v1/audit?limit={limit}");
        if let Some(b) = before {
            path.push_str(&format!("&before={b}"));
        }
        self.get(&path).await
    }
}

/// Minimal percent-encoding for query-string values.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Decode an NDJSON body into events, one per line.
/// Longest NDJSON line accepted (one batch of rows). Guards against a
/// misbehaving server making the client buffer without bound.
pub const MAX_LINE: usize = 256 * 1024 * 1024;

/// Incremental NDJSON → [`QueryEvent`] decoder; bytes may arrive in any
/// chunking. Pure (no I/O), so it can be fuzzed.
#[derive(Default)]
pub struct NdjsonDecoder {
    buf: BytesMut,
    /// Bytes of `buf` already known to contain no newline.
    scanned: usize,
}

impl NdjsonDecoder {
    /// Feed bytes; returns the events of every completed line.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<Result<QueryEvent>> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        loop {
            match self.buf[self.scanned..].iter().position(|&b| b == b'\n') {
                Some(rel) => {
                    let pos = self.scanned + rel;
                    let line = self.buf.split_to(pos + 1);
                    self.scanned = 0;
                    if let Some(ev) = Self::decode(&line[..pos]) {
                        out.push(ev);
                    }
                }
                None => {
                    self.scanned = self.buf.len();
                    if self.buf.len() > MAX_LINE {
                        self.buf.clear();
                        self.scanned = 0;
                        out.push(Err(Error::Decode(format!(
                            "event line longer than {MAX_LINE} bytes"
                        ))));
                    }
                    return out;
                }
            }
        }
    }

    /// The stream ended: decode a final line without a trailing newline.
    pub fn finish(&mut self) -> Option<Result<QueryEvent>> {
        let rest = std::mem::take(&mut self.buf);
        self.scanned = 0;
        Self::decode(&rest)
    }

    fn decode(line: &[u8]) -> Option<Result<QueryEvent>> {
        if line.iter().all(u8::is_ascii_whitespace) {
            return None;
        }
        Some(
            serde_json::from_slice::<QueryEvent>(line)
                .map_err(|e| Error::Decode(format!("bad event: {e}"))),
        )
    }
}

/// Decode an NDJSON body into events, one per line.
fn ndjson(res: reqwest::Response) -> impl Stream<Item = Result<QueryEvent>> + Send {
    let body = res.bytes_stream();
    let state = (
        body,
        NdjsonDecoder::default(),
        std::collections::VecDeque::new(),
        false,
    );
    futures::stream::unfold(
        state,
        |(mut body, mut dec, mut ready, mut ended)| async move {
            loop {
                if let Some(ev) = ready.pop_front() {
                    return Some((ev, (body, dec, ready, ended)));
                }
                if ended {
                    return None;
                }
                match body.next().await {
                    Some(Ok(chunk)) => ready.extend(dec.push(&chunk)),
                    Some(Err(e)) => {
                        ended = true;
                        ready.push_back(Err(Error::from_reqwest(e)));
                    }
                    None => {
                        ended = true;
                        ready.extend(dec.finish());
                    }
                }
            }
        },
    )
}

/// Is this status an authentication problem (token missing/invalid/revoked)?
pub fn is_auth_error(e: &Error) -> bool {
    matches!(e, Error::Api { status, .. } if *status == StatusCode::UNAUTHORIZED.as_u16())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoder_handles_any_chunking() {
        let events = vec![
            QueryEvent::Started {
                query_id: Uuid::nil(),
            },
            QueryEvent::Message {
                severity: "info".into(),
                text: "é\n✓".into(),
            },
            QueryEvent::Done {
                elapsed_ms: 1,
                cancelled: false,
                in_transaction: None,
            },
        ];
        let body: Vec<u8> = events
            .iter()
            .flat_map(|e| {
                let mut l = serde_json::to_vec(e).unwrap();
                l.push(b'\n');
                l
            })
            .collect();
        for size in [1, 2, 7, body.len()] {
            let mut d = NdjsonDecoder::default();
            let mut got = Vec::new();
            for chunk in body.chunks(size) {
                got.extend(d.push(chunk).into_iter().map(|r| r.unwrap()));
            }
            assert!(d.finish().is_none());
            assert_eq!(got, events, "chunk size {size}");
        }
    }

    #[test]
    fn decoder_reports_garbage_and_skips_blank_lines() {
        let mut d = NdjsonDecoder::default();
        let out = d.push(b"\n  \n{nope}\n");
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], Err(Error::Decode(_))));
        assert!(
            d.push(
                b"{\"event\":\"started\",\"query_id\":\"00000000-0000-0000-0000-000000000000\"}"
            )
            .is_empty()
        );
        assert!(matches!(d.finish(), Some(Ok(QueryEvent::Started { .. }))));
    }

    #[test]
    fn query_encoding() {
        assert_eq!(encode("sales"), "sales");
        assert_eq!(encode("a b&c=é"), "a%20b%26c%3D%C3%A9");
    }

    #[test]
    fn trust_serializes_readably() {
        let t = Trust::Pinned("AB".into());
        assert_eq!(
            serde_json::to_string(&t).unwrap(),
            r#"{"kind":"pinned","fingerprint":"AB"}"#
        );
    }
}
