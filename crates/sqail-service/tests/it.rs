//! End-to-end tests: start the real HTTPS server in-process and talk to it.
//!
//! Tests without `#[ignore]` need nothing but a temp dir. The `#[ignore]`d
//! ones need the podman test databases (`scripts/db.sh up`) and run with
//! `scripts/check.sh --it` (`cargo test -- --ignored`).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sqail_proto::{
    ColumnInfo, Connection, CreatedToken, Ddl, ForeignKeyInfo, NamedItem, QueryEvent, RoutineInfo,
    SessionInfo, TableInfo, TestResult,
};
use sqail_service::{Config, Server};
use uuid::Uuid;

struct Harness {
    base: String,
    http: reqwest::Client,
    admin: String,
    _server: Server,
    dir: tempfile::TempDir,
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

async fn harness_with(tweak: impl FnOnce(&mut Config)) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    // The service's data dir sits inside the allowed SQLite dir, which is
    // exactly the setup where SQLite must still keep away from it.
    let data_dir = dir.path().join("service");
    std::fs::create_dir(&data_dir).unwrap();
    let mut cfg = Config {
        data_dir,
        bind: "127.0.0.1:0".parse().unwrap(),
        docs_ui: false,
        ..Default::default()
    };
    cfg.sqlite.allowed_dirs = vec![dir.path().to_path_buf(), repo_root().join("dev/data")];
    tweak(&mut cfg);
    let server = sqail_service::start(cfg).await.expect("server starts");
    let http = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap();
    Harness {
        base: format!("https://{}", server.addr),
        http,
        admin: server.bootstrap_token.clone().expect("fresh data dir"),
        _server: server,
        dir,
    }
}

async fn harness() -> Harness {
    harness_with(|_| {}).await
}

impl Harness {
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    async fn get(&self, path: &str) -> reqwest::Response {
        self.http
            .get(self.url(path))
            .bearer_auth(&self.admin)
            .send()
            .await
            .unwrap()
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> T {
        let res = self.get(path).await;
        let status = res.status();
        let body = res.text().await.unwrap();
        assert!(status.is_success(), "GET {path}: {status} {body}");
        serde_json::from_str(&body).unwrap()
    }

    async fn post(&self, path: &str, body: &Value, token: &str) -> reqwest::Response {
        self.http
            .post(self.url(path))
            .bearer_auth(token)
            .json(body)
            .send()
            .await
            .unwrap()
    }

    async fn create_connection(&self, body: Value) -> Uuid {
        let res = self.post("/v1/connections", &body, &self.admin).await;
        let status = res.status();
        let text = res.text().await.unwrap();
        assert_eq!(status, 201, "{text}");
        serde_json::from_str::<Connection>(&text).unwrap().id
    }

    async fn token(&self, scope: &str) -> String {
        let res = self
            .post(
                "/v1/tokens",
                &json!({"name": format!("t-{scope}"), "scope": scope}),
                &self.admin,
            )
            .await;
        res.json::<CreatedToken>().await.unwrap().token
    }

    /// Run SQL and collect every event of the stream.
    async fn query_on(&self, path: &str, body: Value, qid: Option<Uuid>) -> Vec<QueryEvent> {
        let mut req = self
            .http
            .post(self.url(path))
            .bearer_auth(&self.admin)
            .json(&body);
        if let Some(q) = qid {
            req = req.header("x-query-id", q.to_string());
        }
        let res = req.send().await.unwrap();
        let status = res.status();
        let text = res.text().await.unwrap();
        assert!(status.is_success(), "{path}: {status} {text}");
        text.lines()
            .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("bad line {l}: {e}")))
            .collect()
    }

    async fn query(&self, conn: Uuid, sql: &str) -> Vec<QueryEvent> {
        self.query_on(
            &format!("/v1/connections/{conn}/query"),
            json!({ "sql": sql }),
            None,
        )
        .await
    }

    async fn sqlite_temp(&self, read_only: bool) -> Uuid {
        let path = self.dir.path().join("t.db");
        self.create_connection(json!({
            "name": format!("tmp-{read_only}"),
            "params": {"engine": "sqlite", "path": path, "create": true},
            "read_only": read_only,
        }))
        .await
    }
}

/// Rows of result set `index`.
fn rows(events: &[QueryEvent], index: u32) -> Vec<Vec<Value>> {
    events
        .iter()
        .filter_map(|e| match e {
            QueryEvent::Rows { index: i, rows } if *i == index => Some(rows.clone()),
            _ => None,
        })
        .flatten()
        .collect()
}

fn error(events: &[QueryEvent]) -> Option<(String, String)> {
    events.iter().find_map(|e| match e {
        QueryEvent::Error { code, message, .. } => Some((code.clone(), message.clone())),
        _ => None,
    })
}

fn done(events: &[QueryEvent]) -> (bool, Option<bool>) {
    match events.last() {
        Some(QueryEvent::Done {
            cancelled,
            in_transaction,
            ..
        }) => (*cancelled, *in_transaction),
        other => panic!("stream must end with done, got {other:?}"),
    }
}

fn affected(events: &[QueryEvent]) -> Vec<u64> {
    events
        .iter()
        .filter_map(|e| match e {
            QueryEvent::RowsAffected { count } => Some(*count),
            _ => None,
        })
        .collect()
}

// =========================================================== no databases ==

#[tokio::test]
async fn health_needs_no_token_but_everything_else_does() {
    let h = harness().await;
    let res = h.http.get(h.url("/v1/health")).send().await.unwrap();
    assert_eq!(res.status(), 200);
    assert_eq!(res.headers()["x-content-type-options"], "nosniff");
    let res = h.http.get(h.url("/v1/info")).send().await.unwrap();
    assert_eq!(res.status(), 401);
    assert_eq!(res.headers()["content-type"], "application/problem+json");
    let res = h
        .http
        .get(h.url("/v1/info"))
        .bearer_auth("sq2_not-a-real-token")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401);
    let info: Value = h.get_json("/v1/info").await;
    assert_eq!(info["scope"], "admin");
}

#[tokio::test]
async fn scopes_and_revocation() {
    let h = harness().await;
    let read = h.token("read").await;
    let res = h
        .post(
            "/v1/connections",
            &json!({"name": "x", "params": {"engine": "sqlite", "path": "/x.db"}}),
            &read,
        )
        .await;
    assert_eq!(res.status(), 403);
    let res = h
        .http
        .get(h.url("/v1/connections"))
        .bearer_auth(&read)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "read scope may list profiles");

    let tokens: Vec<sqail_proto::TokenInfo> = h.get_json("/v1/tokens").await;
    let id = tokens.iter().find(|t| t.name == "t-read").unwrap().id;
    let res = h
        .http
        .delete(h.url(&format!("/v1/tokens/{id}")))
        .bearer_auth(&h.admin)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 204);
    let res = h
        .http
        .get(h.url("/v1/info"))
        .bearer_auth(&read)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401, "revoked token is rejected");
}

#[tokio::test]
async fn read_scope_only_queries_read_only_profiles() {
    let h = harness().await;
    let rw = h.sqlite_temp(false).await;
    h.query(rw, "CREATE TABLE t (a INTEGER)").await;
    let ro = h.sqlite_temp(true).await;
    let read = h.token("read").await;
    let q = json!({"sql": "SELECT 1"});
    let res = h
        .post(&format!("/v1/connections/{rw}/query"), &q, &read)
        .await;
    assert_eq!(res.status(), 403);
    let res = h
        .post(&format!("/v1/connections/{ro}/query"), &q, &read)
        .await;
    assert_eq!(res.status(), 200);
    // Read-only is enforced by the driver too.
    let ev = h.query(ro, "INSERT INTO t VALUES (1)").await;
    assert!(error(&ev).unwrap().1.contains("readonly"), "{ev:?}");
}

#[tokio::test]
async fn rate_limit_applies_per_token() {
    let h = harness_with(|c| {
        c.limits.requests_per_second = 1;
        c.limits.burst = 3;
    })
    .await;
    let mut statuses = Vec::new();
    for _ in 0..5 {
        statuses.push(h.get("/v1/info").await.status().as_u16());
    }
    assert!(statuses.contains(&429), "{statuses:?}");
}

#[tokio::test]
async fn passwords_are_write_only() {
    let h = harness().await;
    let id = h
        .create_connection(json!({
            "name": "pg",
            "params": {"engine": "postgres", "host": "db.invalid", "database": "x", "user": "u"},
            "password": "hunter2",
        }))
        .await;
    let res = h
        .get(&format!("/v1/connections/{id}"))
        .await
        .text()
        .await
        .unwrap();
    assert!(!res.contains("hunter2"));
    let conn: Connection = serde_json::from_str(&res).unwrap();
    assert!(conn.has_password);
    assert_eq!(conn.params.engine(), sqail_proto::Engine::Postgres);
}

#[tokio::test]
async fn copies_take_the_password_from_secret_from() {
    let h = harness().await;
    let profile = |name: &str| {
        json!({"name": name,
            "params": {"engine": "postgres", "host": "db.invalid", "database": "x", "user": "u"}})
    };
    let mut original = profile("pg");
    original["password"] = json!("hunter2");
    let id = h.create_connection(original).await;

    let res = h
        .post(
            &format!("/v1/connections?secret_from={id}"),
            &profile("pg (copy)"),
            &h.admin,
        )
        .await;
    assert_eq!(res.status(), 201);
    let copy: Connection = res.json().await.unwrap();
    assert_ne!(copy.id, id);
    assert!(copy.has_password, "the password was copied");

    // Without secret_from, or with an explicit empty password, nothing is copied.
    let plain = h.create_connection(profile("plain")).await;
    let plain: Connection = h.get_json(&format!("/v1/connections/{plain}")).await;
    assert!(!plain.has_password);
    let mut cleared = profile("cleared");
    cleared["password"] = json!("");
    let res = h
        .post(
            &format!("/v1/connections?secret_from={id}"),
            &cleared,
            &h.admin,
        )
        .await;
    assert!(!res.json::<Connection>().await.unwrap().has_password);

    let res = h
        .post(
            &format!("/v1/connections?secret_from={}", Uuid::new_v4()),
            &profile("missing"),
            &h.admin,
        )
        .await;
    assert_eq!(res.status(), 404);
}

#[tokio::test]
async fn azure_discovery_needs_an_entra_connection_and_admin() {
    let h = harness().await;
    let sql = h
        .create_connection(json!({"name": "sql", "params": {"engine": "mssql",
            "host": "db.invalid", "auth": {"method": "sql", "user": "sa"}}, "password": "p"}))
        .await;
    let res = h
        .post(
            &format!("/v1/connections/{sql}/azure/discover"),
            &json!({}),
            &h.admin,
        )
        .await;
    assert_eq!(res.status(), 400);
    assert!(res.text().await.unwrap().contains("Microsoft Entra ID"));

    let entra = h
        .create_connection(json!({"name": "azure", "params": {"engine": "mssql",
            "host": "x.database.windows.net",
            "auth": {"method": "entra_service_principal", "tenant": "t", "client_id": "c"}},
            "password": "secret"}))
        .await;
    let query = h.token("query").await;
    let res = h
        .post(
            &format!("/v1/connections/{entra}/azure/discover"),
            &json!({}),
            &query,
        )
        .await;
    assert_eq!(res.status(), 403);
}

const KEY_SENTINEL: &str = "SQAIL-KEY-SENTINEL-DO-NOT-LEAK";

/// A line of the key that cannot also appear in the certificate.
fn key_marker(pem: &str) -> &str {
    pem.lines()
        .filter(|l| !l.starts_with('-') && l.len() > 40)
        .max_by_key(|l| l.len())
        .unwrap()
}

fn client_material() -> (String, String, String) {
    use rcgen::{
        BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    };
    let ca_key = KeyPair::generate().unwrap();
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca_cert = ca_params.self_signed(&ca_key).unwrap();
    let issuer = Issuer::from_params(&ca_params, &ca_key);
    let client_key = KeyPair::generate().unwrap();
    let mut client_params = CertificateParams::new(vec!["sqail-client".into()]).unwrap();
    client_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    let client_cert = client_params.signed_by(&client_key, &issuer).unwrap();
    (ca_cert.pem(), client_cert.pem(), client_key.serialize_pem())
}

fn pg_profile(
    name: &str,
    mode: &str,
    root: Option<&str>,
    cert: Option<&str>,
    key: Option<&str>,
) -> Value {
    let mut params = json!({
        "engine": "postgres",
        "host": "db.invalid",
        "database": "x",
        "user": "u",
        "ssl_mode": mode,
    });
    if let Some(root) = root {
        params["ssl_root_cert"] = json!(root);
    }
    if let Some(cert) = cert {
        params["ssl_client_cert"] = json!(cert);
    }
    let mut body = json!({"name": name, "params": params, "password": "hunter2-ssl"});
    if let Some(key) = key {
        body["ssl_client_key"] = json!(key);
    }
    body
}

async fn assert_rejected(h: &Harness, body: &Value, detail: &str, secret: &str) {
    let res = h.post("/v1/connections", body, &h.admin).await;
    let status = res.status();
    let text = res.text().await.unwrap();
    assert_eq!(status, 400, "{text}");
    assert!(text.contains(detail), "{text}");
    assert!(!text.contains(secret), "{text}");
    assert!(
        text.len() < 4_000,
        "error response is {} bytes; it may echo a certificate",
        text.len()
    );
}

#[tokio::test]
async fn pg_ssl_certs_are_stored_and_the_key_is_write_only() {
    let (ca, cert, key) = client_material();
    let marker = key_marker(&key).to_string();
    let h = harness().await;

    let ca_id = h
        .create_connection(pg_profile("ca", "verify-ca", Some(&ca), None, None))
        .await;
    let ca_body = h
        .get(&format!("/v1/connections/{ca_id}"))
        .await
        .text()
        .await
        .unwrap();
    assert!(ca_body.contains("verify-ca"), "{ca_body}");
    assert!(ca_body.contains("BEGIN CERTIFICATE"), "{ca_body}");
    assert!(!ca_body.contains("\"ssl_client_key\""));
    let ca_conn: Connection = serde_json::from_str(&ca_body).unwrap();
    assert!(!ca_conn.has_ssl_client_key);

    let id = h
        .create_connection(pg_profile("pg", "require", None, Some(&cert), Some(&key)))
        .await;
    let got = h
        .get(&format!("/v1/connections/{id}"))
        .await
        .text()
        .await
        .unwrap();
    assert!(!got.contains(&marker), "{got}");
    assert!(!got.contains("hunter2-ssl"), "{got}");
    assert!(!got.contains("\"ssl_client_key\""));
    assert!(got.contains("ssl_client_cert"));
    let conn: Connection = serde_json::from_str(&got).unwrap();
    assert!(conn.has_ssl_client_key);
    assert!(conn.has_password);

    // Omitting the key on update keeps the stored one.
    let res = h
        .http
        .put(h.url(&format!("/v1/connections/{id}")))
        .bearer_auth(&h.admin)
        .json(&pg_profile("pg", "require", None, Some(&cert), None))
        .send()
        .await
        .unwrap();
    let status = res.status();
    let text = res.text().await.unwrap();
    assert_eq!(status, 200, "{text}");
    assert!(!text.contains(&marker), "{text}");
    let conn: Connection = serde_json::from_str(&text).unwrap();
    assert!(conn.has_ssl_client_key);

    let other = rcgen::KeyPair::generate().unwrap().serialize_pem();
    let other_marker = key_marker(&other).to_string();
    let res = h
        .http
        .put(h.url(&format!("/v1/connections/{id}")))
        .bearer_auth(&h.admin)
        .json(&pg_profile(
            "pg",
            "require",
            None,
            Some(&cert),
            Some(&other),
        ))
        .send()
        .await
        .unwrap();
    let status = res.status();
    let text = res.text().await.unwrap();
    assert_eq!(status, 400, "{text}");
    assert!(!text.contains(&other_marker), "{text}");
    let still: Connection = h.get_json(&format!("/v1/connections/{id}")).await;
    assert!(still.has_ssl_client_key);

    let encrypted = format!(
        "-----BEGIN ENCRYPTED PRIVATE KEY-----\n{KEY_SENTINEL}\n-----END ENCRYPTED PRIVATE KEY-----\n"
    );
    assert_rejected(
        &h,
        &pg_profile("bad-enc", "require", None, Some(&cert), Some(&encrypted)),
        "encrypted",
        KEY_SENTINEL,
    )
    .await;
    assert_rejected(
        &h,
        &pg_profile("bad-disable", "disable", None, Some(&cert), Some(&key)),
        "disable",
        &marker,
    )
    .await;
    assert_rejected(
        &h,
        &pg_profile("bad-root", "require", Some(&ca), None, None),
        "verify-ca",
        "BEGIN CERTIFICATE",
    )
    .await;
    assert_rejected(
        &h,
        &pg_profile("bad-key", "require", None, None, Some(KEY_SENTINEL)),
        "a client key needs a client certificate",
        KEY_SENTINEL,
    )
    .await;
    let mut huge = pg_profile("bad-size", "require", None, None, None);
    huge["params"]["ssl_client_cert"] = json!("A".repeat(64 * 1024 + 1));
    assert_rejected(&h, &huge, "larger than 64 KiB", "AAAA").await;
    assert_rejected(
        &h,
        &json!({
            "name": "lite",
            "params": {"engine": "sqlite", "path": h.dir.path().join("a.db"), "create": true},
            "ssl_client_key": KEY_SENTINEL,
        }),
        "only for PostgreSQL",
        KEY_SENTINEL,
    )
    .await;

    // The stored key is used when the body omits it, and it is not echoed.
    let probe = pg_profile("pg", "require", None, Some(&cert), None);
    let res = h
        .post(
            &format!("/v1/connections/test?secret_from={id}"),
            &probe,
            &h.admin,
        )
        .await;
    let status = res.status();
    let text = res.text().await.unwrap();
    assert_eq!(status, 200, "{text}");
    assert!(!text.contains(&marker), "{text}");
    assert!(!text.contains("hunter2-ssl"), "{text}");
    let result: TestResult = serde_json::from_str(&text).unwrap();
    assert!(!result.ok, "{text}");
    let res = h.post("/v1/connections/test", &probe, &h.admin).await;
    let status = res.status();
    let text = res.text().await.unwrap();
    assert_eq!(status, 400, "{text}");
    assert!(text.contains("private key"), "{text}");

    let res = h
        .http
        .put(h.url(&format!("/v1/connections/{id}")))
        .bearer_auth(&h.admin)
        .json(&pg_profile("pg", "require", None, None, None))
        .send()
        .await
        .unwrap();
    let status = res.status();
    let text = res.text().await.unwrap();
    assert_eq!(status, 200, "{text}");
    let conn: Connection = serde_json::from_str(&text).unwrap();
    assert!(!conn.has_ssl_client_key);
    assert!(!text.contains("ssl_client_cert"));
}

#[tokio::test]
async fn unknown_engines_are_rejected_with_400() {
    let h = harness().await;
    let body = json!({"name": "x", "params": {"engine": "oracle", "host": "db"}});
    for path in [
        "/v1/connections",
        "/v1/connections/test",
        "/v1/connections/databases",
    ] {
        let res = h.post(path, &body, &h.admin).await;
        let status = res.status();
        let text = res.text().await.unwrap();
        assert_eq!(status, 400, "{path}: {text}");
        assert!(
            text.contains("engine 'oracle' is not supported by this sqail-service version"),
            "{path}: {text}"
        );
    }
    // A known engine with a bad field still gets serde's own message.
    let body = json!({"name": "x", "params": {"engine": "postgres", "host": "db"}});
    let res = h.post("/v1/connections", &body, &h.admin).await;
    let status = res.status();
    let text = res.text().await.unwrap();
    assert!(status.is_client_error(), "{text}");
    assert!(text.contains("database"), "{text}");
}

#[tokio::test]
async fn sqlite_outside_allowed_dirs_is_rejected() {
    let h = harness().await;
    let res = h
        .post(
            "/v1/connections",
            &json!({"name": "x", "params": {"engine": "sqlite", "path": "/etc/passwd"}}),
            &h.admin,
        )
        .await;
    assert_eq!(res.status(), 400);
}

#[tokio::test]
async fn sqlite_profiles_cannot_open_the_service_data_dir() {
    let h = harness().await;
    let data = h.dir.path().join("service");
    for path in [data.join("service.db"), data.join("new.db")] {
        let res = h
            .post(
                "/v1/connections",
                &json!({"name": "x", "params": {"engine": "sqlite", "path": path, "create": true}}),
                &h.admin,
            )
            .await;
        assert_eq!(res.status(), 400, "{}", path.display());
    }
}

#[tokio::test]
async fn sqlite_attach_and_vacuum_into_cannot_leave_the_allowed_dirs() {
    let h = harness().await;
    let rw = h.sqlite_temp(false).await;
    let ro = h.sqlite_temp(true).await;
    h.query(rw, "CREATE TABLE t (a INTEGER); INSERT INTO t VALUES (1)")
        .await;
    let outside = tempfile::tempdir().unwrap();
    let service_db = h.dir.path().join("service/service.db");
    let stray = outside.path().join("stray.db");
    let copy = h.dir.path().join("copy.db");
    let targets = [service_db.display(), stray.display()].map(|p| p.to_string());

    for (conn, kind) in [(rw, "writable"), (ro, "read-only")] {
        for t in &targets {
            for sql in [
                format!("ATTACH DATABASE '{t}' AS x"),
                format!("VACUUM INTO '{t}'"),
            ] {
                let ev = h.query(conn, &sql).await;
                assert!(error(&ev).is_some(), "{kind}: {sql} was allowed");
            }
        }
        let ev = h
            .query(conn, &format!("ATTACH 'file:{}' AS x", stray.display()))
            .await;
        assert!(error(&ev).is_some(), "{kind}: URI attach was allowed");
    }
    assert!(!stray.exists());

    // service.db was not replaced and the service still works.
    let ev = h.query(rw, "SELECT a FROM t").await;
    assert_eq!(rows(&ev, 0), vec![vec![json!(1)]]);

    // Read-only profiles write no files at all, even inside the allowed dir.
    let ev = h
        .query(ro, &format!("VACUUM INTO '{}'", copy.display()))
        .await;
    assert!(error(&ev).is_some());
    assert!(!copy.exists());

    // Writable profiles may still use files inside the allowed dirs.
    let ev = h
        .query(rw, &format!("VACUUM INTO '{}'", copy.display()))
        .await;
    assert!(error(&ev).is_none(), "{ev:?}");
    let ev = h
        .query(
            rw,
            &format!(
                "ATTACH '{}' AS c; SELECT a FROM c.t; DETACH c",
                copy.display()
            ),
        )
        .await;
    assert!(error(&ev).is_none(), "{ev:?}");
    assert_eq!(rows(&ev, 0), vec![vec![json!(1)]]);
}

#[tokio::test]
async fn unsaved_profiles_list_their_databases_for_admins_only() {
    let h = harness().await;
    let body = json!({"name": "x", "params": {"engine": "sqlite",
        "path": h.dir.path().join("list.db"), "create": true}});
    let res = h.post("/v1/connections/databases", &body, &h.admin).await;
    assert_eq!(res.status(), 200);
    let dbs: Vec<NamedItem> = res.json().await.unwrap();
    assert_eq!(
        dbs.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
        ["main"]
    );

    let query = h.token("query").await;
    let res = h.post("/v1/connections/databases", &body, &query).await;
    assert_eq!(res.status(), 403);

    // Nothing listens on port 1: a connection error, not a server error.
    let body = json!({"name": "x", "params": {"engine": "postgres", "host": "127.0.0.1",
        "port": 1, "database": "", "user": "u"}, "password": "p"});
    let res = h.post("/v1/connections/databases", &body, &h.admin).await;
    assert_eq!(res.status(), 502);
}

#[tokio::test]
async fn openapi_document_is_served() {
    let h = harness().await;
    let doc: Value = h
        .http
        .get(h.url("/v1/openapi.json"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(doc["paths"]["/v1/connections/{id}/query"]["post"].is_object());
    assert!(doc["components"]["schemas"]["QueryEvent"].is_object());
}

#[tokio::test]
async fn sqlite_script_streams_results() {
    let h = harness().await;
    let c = h.sqlite_temp(false).await;
    let ev = h
        .query(
            c,
            "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT); \
             INSERT INTO t (name) VALUES ('a'), ('b'), ('c'); \
             SELECT id, name FROM t ORDER BY id; SELECT 1 AS x WHERE 0",
        )
        .await;
    assert!(matches!(ev[0], QueryEvent::Started { .. }));
    assert_eq!(affected(&ev), vec![0, 3]);
    assert_eq!(
        rows(&ev, 0),
        vec![
            vec![json!(1), json!("a")],
            vec![json!(2), json!("b")],
            vec![json!(3), json!("c")]
        ]
    );
    assert!(
        ev.iter().any(|e| matches!(
            e,
            QueryEvent::ResultEnd {
                index: 1,
                row_count: 0,
                ..
            }
        )),
        "empty result still has a result set"
    );
    assert_eq!(done(&ev), (false, None));
}

#[tokio::test]
async fn max_rows_truncates() {
    let h = harness().await;
    let c = h.sqlite_temp(false).await;
    let ev = h
        .query_on(
            &format!("/v1/connections/{c}/query"),
            json!({"sql": "WITH RECURSIVE r(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM r LIMIT 5000) SELECT i FROM r", "max_rows": 1200}),
            None,
        )
        .await;
    assert_eq!(rows(&ev, 0).len(), 1200);
    assert!(ev.iter().any(|e| matches!(
        e,
        QueryEvent::ResultEnd {
            row_count: 1200,
            truncated: true,
            ..
        }
    )));
}

/// Bodies up to `limits.body_limit_bytes` (8 MiB) are accepted, not just
/// axum's default 2 MiB, and a larger one is a 413 problem document whether
/// it announces its length or not.
#[tokio::test]
async fn body_limit_is_the_configured_one() {
    let h = harness().await;
    let c = h.sqlite_temp(false).await;
    let path = format!("/v1/connections/{c}/query");
    let body = |len: usize| json!({ "sql": format!("SELECT 1 AS x -- {}", "y".repeat(len)) });
    let ev = h.query_on(&path, body(5_000_000), None).await;
    assert_eq!(rows(&ev, 0), vec![vec![json!(1)]]);

    let check = |res: reqwest::Response| async move {
        assert_eq!(res.status(), 413);
        assert_eq!(
            res.headers()["content-type"],
            "application/problem+json",
            "{:?}",
            res.headers()
        );
        let p: Value = res.json().await.unwrap();
        assert_eq!(p["code"], "payload_too_large", "{p}");
        assert!(p["detail"].as_str().unwrap().contains("8388608"), "{p}");
    };
    check(h.post(&path, &body(9_000_000), &h.admin).await).await;
    // Without content-length (chunked), the extractor notices instead.
    let bytes = serde_json::to_vec(&body(9_000_000)).unwrap();
    let chunks: Vec<Result<Vec<u8>, std::io::Error>> =
        bytes.chunks(1 << 20).map(|c| Ok(c.to_vec())).collect();
    let res = h
        .http
        .post(h.url(&path))
        .bearer_auth(&h.admin)
        .header("content-type", "application/json")
        .body(reqwest::Body::wrap_stream(futures::stream::iter(chunks)))
        .send()
        .await
        .unwrap();
    check(res).await;
}

/// Wide rows come in batches bounded in bytes, not only in rows (500 rows
/// of 600 kB were one 300 MB line).
#[tokio::test]
async fn wide_rows_are_batched_by_size() {
    let h = harness().await;
    let c = h.sqlite_temp(false).await;
    let res = h
        .post(
            &format!("/v1/connections/{c}/query"),
            &json!({"sql": "WITH RECURSIVE g(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM g LIMIT 500) \
                            SELECT n, hex(zeroblob(300000)) AS x FROM g"}),
            &h.admin,
        )
        .await;
    assert_eq!(res.status(), 200);
    let text = res.text().await.unwrap();
    let longest = text.lines().map(str::len).max().unwrap();
    assert!(longest < 2 * 1024 * 1024, "{longest}");
    let ev: Vec<QueryEvent> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let r = rows(&ev, 0);
    assert_eq!(r.len(), 500);
    assert_eq!(r[499][1].as_str().unwrap().len(), 600_000);
}

#[tokio::test]
async fn errors_are_events_and_scripts_stop() {
    let h = harness().await;
    let c = h.sqlite_temp(false).await;
    let ev = h
        .query(c, "SELECT 1; SELECT * FROM missing; SELECT 2")
        .await;
    assert_eq!(rows(&ev, 0), vec![vec![json!(1)]]);
    let (code, msg) = error(&ev).unwrap();
    assert_eq!(code, "db_error");
    assert!(msg.contains("no such table"), "{msg}");
    assert!(rows(&ev, 1).is_empty());
    done(&ev);
}

#[tokio::test]
async fn open_transaction_outside_session_is_rolled_back() {
    let h = harness().await;
    let c = h.sqlite_temp(false).await;
    h.query(c, "CREATE TABLE t (a INTEGER)").await;
    let ev = h.query(c, "BEGIN; INSERT INTO t VALUES (1)").await;
    assert!(
        ev.iter()
            .any(|e| matches!(e, QueryEvent::Message { severity, .. } if severity == "warning"))
    );
    let ev = h.query(c, "SELECT count(*) FROM t").await;
    assert_eq!(rows(&ev, 0), vec![vec![json!(0)]]);
}

#[tokio::test]
async fn cancel_stops_a_running_query() {
    let h = harness().await;
    let c = h.sqlite_temp(false).await;
    let qid = Uuid::new_v4();
    let canceller = {
        let (http, url, token) = (
            h.http.clone(),
            h.url(&format!("/v1/queries/{qid}")),
            h.admin.clone(),
        );
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            http.delete(url)
                .bearer_auth(token)
                .send()
                .await
                .unwrap()
                .status()
        })
    };
    let started = Instant::now();
    let ev = h
        .query_on(
            &format!("/v1/connections/{c}/query"),
            json!({"sql": "WITH RECURSIVE r(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM r) SELECT count(*) FROM r"}),
            Some(qid),
        )
        .await;
    assert_eq!(canceller.await.unwrap(), 202);
    assert!(done(&ev).0);
    assert_eq!(error(&ev).unwrap().0, "cancelled");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn timeout_stops_a_running_query() {
    let h = harness().await;
    let c = h.sqlite_temp(false).await;
    let ev = h
        .query_on(
            &format!("/v1/connections/{c}/query"),
            json!({"sql": "WITH RECURSIVE r(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM r) SELECT count(*) FROM r", "timeout_ms": 300}),
            None,
        )
        .await;
    assert_eq!(error(&ev).unwrap().0, "timeout");
}

#[tokio::test]
async fn audit_log_records_queries() {
    let h = harness().await;
    let c = h.sqlite_temp(false).await;
    h.query(c, "SELECT 42").await;
    let page: sqail_proto::AuditPage = h.get_json("/v1/audit?limit=10").await;
    let q = page
        .items
        .iter()
        .find(|e| e.action == "query")
        .expect("query audited");
    assert_eq!(q.detail.as_deref(), Some("SELECT 42"));
    assert!(page.items.iter().any(|e| e.action == "connection.create"));
}

// ======================================================= engine matrix ==
// Shared checks, run against every engine. Each engine has its own dialect
// snippets; the assertions are the same.

struct Dialect {
    engine: &'static str,
    profile: Value,
    /// Qualified name of the sales schema tables.
    t: fn(&str) -> String,
    sleep: &'static str,
    param_sql: &'static str,
    begin: &'static str,
    rollback: &'static str,
    /// Count rows with a given name without blocking on uncommitted locks.
    count_by_name: &'static str,
    schema: Option<&'static str>,
    routine: Option<&'static str>,
    enforces_read_only: bool,
}

fn postgres() -> Dialect {
    Dialect {
        engine: "postgres",
        profile: json!({"engine": "postgres", "host": "127.0.0.1", "port": 55432, "database": "sqail_test", "user": "sqail", "ssl_mode": "disable"}),
        t: |n| format!("sales.{n}"),
        sleep: "SELECT pg_sleep(30)",
        param_sql: "SELECT id, name FROM sales.customers WHERE id = $1 AND name LIKE $2",
        begin: "BEGIN",
        rollback: "ROLLBACK",
        count_by_name: "SELECT count(*) FROM sales.customers WHERE name = 'it-session'",
        schema: Some("sales"),
        routine: Some("customer_revenue"),
        enforces_read_only: true,
    }
}

fn mssql() -> Dialect {
    Dialect {
        engine: "mssql",
        profile: json!({"engine": "mssql", "host": "127.0.0.1", "port": 51433, "database": "sqail_test",
                        "auth": {"method": "sql", "user": "sqail"}, "trust_server_certificate": true}),
        t: |n| format!("sales.{n}"),
        sleep: "WAITFOR DELAY '00:00:30'",
        param_sql: "SELECT id, name FROM sales.customers WHERE id = @P1 AND name LIKE @P2",
        begin: "BEGIN TRANSACTION",
        rollback: "ROLLBACK",
        count_by_name: "SELECT count(*) FROM sales.customers WITH (READPAST) WHERE name = 'it-session'",
        schema: Some("sales"),
        routine: Some("customer_orders"),
        // ApplicationIntent=ReadOnly only matters on availability groups.
        enforces_read_only: false,
    }
}

fn sqlite_dev() -> Dialect {
    Dialect {
        engine: "sqlite",
        profile: json!({"engine": "sqlite", "path": repo_root().join("dev/data/sqail_test.db").canonicalize().expect("run scripts/db.sh up")}),
        t: |n| n.to_string(),
        sleep: "WITH RECURSIVE r(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM r) SELECT count(*) FROM r",
        param_sql: "SELECT id, name FROM customers WHERE id = ?1 AND name LIKE ?2",
        begin: "BEGIN",
        rollback: "ROLLBACK",
        count_by_name: "SELECT count(*) FROM customers WHERE name = 'it-session'",
        schema: None,
        routine: None,
        enforces_read_only: true,
    }
}

fn password(engine: &str) -> Option<&'static str> {
    match engine {
        "postgres" => Some("sqail_dev_pw"),
        "mssql" => Some("Sqail2_dev!Passw0rd"),
        _ => None,
    }
}

async fn profile(h: &Harness, d: &Dialect, read_only: bool) -> Uuid {
    h.create_connection(json!({
        "name": format!("{} ro={read_only}", d.engine),
        "params": d.profile,
        "password": password(d.engine),
        "read_only": read_only,
    }))
    .await
}

fn schema_qs(d: &Dialect) -> String {
    d.schema.map(|s| format!("schema={s}&")).unwrap_or_default()
}

async fn check_engine(d: Dialect) {
    let h = harness().await;
    let c = profile(&h, &d, false).await;

    // connectivity
    let test: TestResult = h
        .post(&format!("/v1/connections/{c}/test"), &json!({}), &h.admin)
        .await
        .json()
        .await
        .unwrap();
    assert!(test.ok, "{}: {:?}", d.engine, test.error);

    // the connection form's database list: unsaved settings without a
    // database, with the password given or taken from the saved profile
    if d.engine != "sqlite" {
        let mut params = d.profile.clone();
        // An empty database, as the connection form sends it.
        params["database"] = json!("");
        for (body, qs) in [
            (
                json!({"name": "x", "params": params, "password": password(d.engine)}),
                String::new(),
            ),
            (
                json!({"name": "x", "params": params}),
                format!("?secret_from={c}"),
            ),
        ] {
            let res = h
                .post(&format!("/v1/connections/databases{qs}"), &body, &h.admin)
                .await;
            let status = res.status();
            let text = res.text().await.unwrap();
            assert!(status.is_success(), "{}: {status} {text}", d.engine);
            let dbs: Vec<NamedItem> = serde_json::from_str(&text).unwrap();
            assert!(
                dbs.iter().any(|n| n.name == "sqail_test"),
                "{}: {dbs:?}",
                d.engine
            );
        }
    }

    // the seeded data is identical on every engine
    let ev = h
        .query(
            c,
            &format!("SELECT count(*) AS n FROM {}", (d.t)("order_items")),
        )
        .await;
    assert_eq!(rows(&ev, 0), vec![vec![json!(29850)]], "{}", d.engine);

    // type zoo: two rows, the second all NULL
    let ev = h
        .query(
            c,
            &format!("SELECT * FROM {} ORDER BY id", (d.t)("type_zoo")),
        )
        .await;
    let zoo = rows(&ev, 0);
    assert_eq!(zoo.len(), 2, "{} {ev:?}", d.engine);
    assert!(
        zoo[1][1..].iter().all(Value::is_null),
        "{}: {:?}",
        d.engine,
        zoo[1]
    );
    assert!(error(&ev).is_none(), "{}: {ev:?}", d.engine);

    // parameters
    let ev = h
        .query_on(
            &format!("/v1/connections/{c}/query"),
            json!({"sql": d.param_sql, "params": [7, "Customer%"]}),
            None,
        )
        .await;
    assert_eq!(
        rows(&ev, 0),
        vec![vec![json!(7), json!("Customer 7")]],
        "{} {ev:?}",
        d.engine
    );

    // truncation
    let ev = h
        .query_on(
            &format!("/v1/connections/{c}/query"),
            json!({"sql": format!("SELECT * FROM {}", (d.t)("orders")), "max_rows": 100}),
            None,
        )
        .await;
    assert_eq!(rows(&ev, 0).len(), 100);
    assert!(ev.iter().any(|e| matches!(
        e,
        QueryEvent::ResultEnd {
            truncated: true,
            ..
        }
    )));

    // DML reports affected rows
    let ev = h
        .query(
            c,
            &format!(
                "UPDATE {} SET active = active WHERE id <= 3",
                (d.t)("products")
            ),
        )
        .await;
    assert_eq!(affected(&ev), vec![3], "{} {ev:?}", d.engine);

    // cancel
    let qid = Uuid::new_v4();
    let (http, url, token) = (
        h.http.clone(),
        h.url(&format!("/v1/queries/{qid}")),
        h.admin.clone(),
    );
    let cancel = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(700)).await;
        http.delete(url)
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status()
    });
    let started = Instant::now();
    let ev = h
        .query_on(
            &format!("/v1/connections/{c}/query"),
            json!({"sql": d.sleep}),
            Some(qid),
        )
        .await;
    assert_eq!(cancel.await.unwrap(), 202);
    assert!(done(&ev).0, "{}: {ev:?}", d.engine);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{} cancel took {:?}",
        d.engine,
        started.elapsed()
    );
    // the pool recovers
    let ev = h.query(c, "SELECT 1").await;
    assert_eq!(rows(&ev, 0), vec![vec![json!(1)]]);

    // sessions: a transaction spans requests and is isolated until rollback
    let s1: SessionInfo = h
        .post("/v1/sessions", &json!({"connection_id": c}), &h.admin)
        .await
        .json()
        .await
        .unwrap();
    let s1_path = format!("/v1/sessions/{}/query", s1.id);
    let run_s1 = |sql: String| h.query_on(&s1_path, json!({ "sql": sql }), None);
    let ev = run_s1(d.begin.to_string()).await;
    assert_eq!(done(&ev).1, Some(true), "{} {ev:?}", d.engine);
    let ev = run_s1(format!(
        "INSERT INTO {} (name, email, country) VALUES ('it-session', NULL, 'BE')",
        (d.t)("customers")
    ))
    .await;
    assert!(error(&ev).is_none(), "{} {ev:?}", d.engine);
    let inside = run_s1(d.count_by_name.to_string()).await;
    assert_eq!(rows(&inside, 0), vec![vec![json!(1)]]);
    let outside = h.query(c, d.count_by_name).await;
    assert_eq!(
        rows(&outside, 0),
        vec![vec![json!(0)]],
        "{}: uncommitted row leaked",
        d.engine
    );
    let info: SessionInfo = h.get_json(&format!("/v1/sessions/{}", s1.id)).await;
    assert!(info.in_transaction);
    let ev = run_s1(d.rollback.to_string()).await;
    assert_eq!(done(&ev).1, Some(false));
    let ev = run_s1(d.count_by_name.to_string()).await;
    assert_eq!(rows(&ev, 0), vec![vec![json!(0)]]);
    let res = h
        .http
        .delete(h.url(&format!("/v1/sessions/{}", s1.id)))
        .bearer_auth(&h.admin)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 204);

    // schema browsing returns the same logical model everywhere
    let qs = schema_qs(&d);
    let tables: Vec<TableInfo> = h
        .get_json(&format!("/v1/connections/{c}/schema/tables?{qs}"))
        .await;
    let names: Vec<&str> = tables.iter().map(|t| t.name.as_str()).collect();
    for want in [
        "customers",
        "orders",
        "order_items",
        "products",
        "type_zoo",
        "order_totals",
    ] {
        assert!(
            names.contains(&want),
            "{}: {want} missing from {names:?}",
            d.engine
        );
    }
    let view = tables.iter().find(|t| t.name == "order_totals").unwrap();
    assert_eq!(view.kind, sqail_proto::TableKind::View);

    let cols: Vec<ColumnInfo> = h
        .get_json(&format!(
            "/v1/connections/{c}/schema/columns?{qs}name=orders"
        ))
        .await;
    let col_names: Vec<&str> = cols.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        col_names,
        ["id", "customer_id", "ordered_at", "status"],
        "{}",
        d.engine
    );
    assert!(cols[0].primary_key && !cols[0].nullable);

    let fks: Vec<ForeignKeyInfo> = h
        .get_json(&format!(
            "/v1/connections/{c}/schema/foreign-keys?{qs}name=order_items"
        ))
        .await;
    let mut refs: Vec<&str> = fks.iter().map(|f| f.ref_table.as_str()).collect();
    refs.sort();
    assert_eq!(refs, ["orders", "products"], "{}", d.engine);

    let idx: Vec<sqail_proto::IndexInfo> = h
        .get_json(&format!(
            "/v1/connections/{c}/schema/indexes?{qs}name=orders"
        ))
        .await;
    assert!(
        idx.iter()
            .any(|i| i.columns == ["customer_id"] && !i.unique),
        "{}: {idx:?}",
        d.engine
    );

    if let Some(routine) = d.routine {
        let rs: Vec<RoutineInfo> = h
            .get_json(&format!("/v1/connections/{c}/schema/routines?{qs}"))
            .await;
        assert!(rs.iter().any(|r| r.name == routine), "{}: {rs:?}", d.engine);
    }

    let ddl: Ddl = h
        .get_json(&format!("/v1/connections/{c}/ddl?{qs}name=orders"))
        .await;
    assert!(
        ddl.ddl.to_uppercase().contains("CREATE TABLE"),
        "{}: {}",
        d.engine,
        ddl.ddl
    );
    let ddl: Ddl = h
        .get_json(&format!("/v1/connections/{c}/ddl?{qs}name=order_totals"))
        .await;
    assert!(
        ddl.ddl.to_uppercase().contains("VIEW"),
        "{}: {}",
        d.engine,
        ddl.ddl
    );

    // query plans
    let plan_sql = format!(
        "SELECT o.id, t.total FROM {} o JOIN {} t ON t.order_id = o.id WHERE o.customer_id = 5",
        (d.t)("orders"),
        (d.t)("order_totals")
    );
    for analyze in [false, true] {
        let res = h
            .post(
                &format!("/v1/connections/{c}/explain"),
                &json!({"sql": plan_sql, "analyze": analyze}),
                &h.admin,
            )
            .await;
        let status = res.status();
        let body = res.text().await.unwrap();
        assert_eq!(
            status, 200,
            "{} explain analyze={analyze}: {body}",
            d.engine
        );
        let plan: sqail_proto::Plan = serde_json::from_str(&body).unwrap();
        assert!(!plan.roots.is_empty(), "{}: empty plan", d.engine);
        fn count(n: &sqail_proto::PlanNode) -> usize {
            1 + n.children.iter().map(count).sum::<usize>()
        }
        assert!(
            count(&plan.roots[0]) >= 2,
            "{}: {:#?}",
            d.engine,
            plan.roots
        );
    }
    // Analyzing a write changes nothing: it runs in a rolled-back transaction.
    if d.engine != "sqlite" {
        let before = h.query(c, "SELECT sum(price) FROM sales.products").await;
        let res = h
            .post(
                &format!("/v1/connections/{c}/explain"),
                &json!({"sql": "UPDATE sales.products SET price = price + 1", "analyze": true}),
                &h.admin,
            )
            .await;
        assert_eq!(res.status(), 200);
        let after = h.query(c, "SELECT sum(price) FROM sales.products").await;
        assert_eq!(
            rows(&before, 0),
            rows(&after, 0),
            "EXPLAIN ANALYZE must not change data"
        );
    }

    // read-only profiles
    if d.enforces_read_only {
        let ro = profile(&h, &d, true).await;
        let ev = h
            .query(
                ro,
                &format!(
                    "UPDATE {} SET active = active WHERE id = 1",
                    (d.t)("products")
                ),
            )
            .await;
        assert!(
            error(&ev).is_some(),
            "{}: write on read-only profile succeeded",
            d.engine
        );
    }
}

#[tokio::test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
async fn engine_postgres() {
    check_engine(postgres()).await;
}

#[tokio::test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
async fn engine_mssql() {
    check_engine(mssql()).await;
}

#[tokio::test]
#[ignore = "needs the dev SQLite database (scripts/db.sh up)"]
async fn engine_sqlite() {
    check_engine(sqlite_dev()).await;
}

/// Explain analyze on a session with an open transaction undoes only the
/// explained statement: the transaction and its earlier work survive, also
/// when the explained statement fails.
async fn explain_analyze_keeps_the_open_transaction(
    engine: &str,
    begin: &str,
    h: &Harness,
    c: Uuid,
) {
    let t = format!("it_explain_tx_{}", Uuid::new_v4().simple());
    let ev = h
        .query(
            c,
            &format!("CREATE TABLE {t} (id int PRIMARY KEY, v int); INSERT INTO {t} VALUES (1, 0)"),
        )
        .await;
    assert!(error(&ev).is_none(), "{} {ev:?}", engine);
    let s: SessionInfo = h
        .post("/v1/sessions", &json!({"connection_id": c}), &h.admin)
        .await
        .json()
        .await
        .unwrap();
    let qp = format!("/v1/sessions/{}/query", s.id);
    let ep = format!("/v1/sessions/{}/explain", s.id);
    let run = |sql: String| h.query_on(&qp, json!({ "sql": sql }), None);
    let ep = &ep;
    let explain = |sql: String| async move {
        let res = h
            .post(ep, &json!({"sql": sql, "analyze": true}), &h.admin)
            .await;
        (res.status(), res.text().await.unwrap())
    };
    let ev = run(begin.to_string()).await;
    assert_eq!(done(&ev).1, Some(true), "{} {ev:?}", engine);
    let ev = run(format!("UPDATE {t} SET v = 1 WHERE id = 1")).await;
    assert!(error(&ev).is_none(), "{} {ev:?}", engine);

    let (status, body) = explain(format!("UPDATE {t} SET v = 2 WHERE id = 1")).await;
    assert_eq!(status, 200, "{}: {body}", engine);
    // Statements that fail to parse or to run leave the transaction usable.
    for bad in ["SELEC 1", "SELECT * FROM it_explain_no_such_table"] {
        let (status, body) = explain(bad.to_string()).await;
        assert!(!status.is_success(), "{} {bad}: {body}", engine);
    }
    let ev = run(format!("SELECT v FROM {t} WHERE id = 1")).await;
    assert!(error(&ev).is_none(), "{} {ev:?}", engine);
    assert_eq!(rows(&ev, 0), vec![vec![json!(1)]], "{}", engine);
    assert_eq!(done(&ev).1, Some(true), "{}: transaction lost", engine);

    let ev = run("COMMIT".to_string()).await;
    assert!(error(&ev).is_none(), "{} {ev:?}", engine);
    assert_eq!(done(&ev).1, Some(false), "{} {ev:?}", engine);
    let ev = h.query(c, &format!("SELECT v FROM {t} WHERE id = 1")).await;
    assert_eq!(
        rows(&ev, 0),
        vec![vec![json!(1)]],
        "{}: commit lost the update",
        engine
    );

    // Without a transaction, analyzing still keeps nothing and opens none.
    let (status, body) = explain(format!("UPDATE {t} SET v = 3 WHERE id = 1")).await;
    assert_eq!(status, 200, "{}: {body}", engine);
    let ev = run(format!("SELECT v FROM {t} WHERE id = 1")).await;
    assert_eq!(rows(&ev, 0), vec![vec![json!(1)]], "{}", engine);
    assert_eq!(done(&ev).1, Some(false), "{}", engine);

    if engine == "postgres" {
        // An aborted transaction is refused with a clear error.
        run(begin.to_string()).await;
        let ev = run("SELECT 1 / 0".to_string()).await;
        assert!(error(&ev).is_some());
        let (status, body) = explain(format!("SELECT v FROM {t}")).await;
        assert_eq!(status, 400, "{body}");
        assert!(body.contains("aborted"), "{body}");
        run("ROLLBACK".to_string()).await;
    }
    h.query(c, &format!("DROP TABLE {t}")).await;
}

#[tokio::test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
async fn postgres_explain_analyze_keeps_the_open_transaction() {
    let h = harness().await;
    let d = postgres();
    let c = profile(&h, &d, false).await;
    explain_analyze_keeps_the_open_transaction(d.engine, d.begin, &h, c).await;
}

#[tokio::test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
async fn mssql_explain_analyze_keeps_the_open_transaction() {
    let h = harness().await;
    let d = mssql();
    let c = profile(&h, &d, false).await;
    explain_analyze_keeps_the_open_transaction(d.engine, d.begin, &h, c).await;
}

#[tokio::test]
async fn sqlite_explain_analyze_keeps_the_open_transaction() {
    // SQLite has no actual plans: analyze never executes the statement.
    let h = harness().await;
    let c = h.sqlite_temp(false).await;
    explain_analyze_keeps_the_open_transaction("sqlite", "BEGIN", &h, c).await;
}

#[tokio::test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
async fn mssql_scripts_split_on_go_and_return_multiple_results() {
    let h = harness().await;
    let c = profile(&h, &mssql(), false).await;
    let ev = h
        .query(
            c,
            "EXEC sales.customer_orders 5\nGO\nPRINT 'x'\nGO 2\nSELECT 1 AS one",
        )
        .await;
    assert_eq!(rows(&ev, 0).len(), 10);
    assert_eq!(rows(&ev, 1).len(), 10);
    assert_eq!(rows(&ev, 2), vec![vec![json!(1)]]);
    let msgs = ev
        .iter()
        .filter(|e| matches!(e, QueryEvent::Message { .. }))
        .count();
    assert_eq!(msgs, 2, "PRINT batch ran twice: {ev:?}");
}

/// The events of a run, short: `msg:<text>`, `result:<index>`,
/// `affected:<count>`, `error:<message>`.
fn outline(events: &[QueryEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            QueryEvent::Message { text, .. } => Some(format!("msg:{text}")),
            QueryEvent::ResultStart { index, .. } => Some(format!("result:{index}")),
            QueryEvent::RowsAffected { count } => Some(format!("affected:{count}")),
            QueryEvent::Error { message, .. } => Some(format!("error:{message}")),
            _ => None,
        })
        .collect()
}

/// SQL Server's PRINT and low-severity RAISERROR messages, row counts and
/// errors reach the client where the server sent them, and a GO script goes
/// on after a failed batch, with script line numbers (#150).
#[tokio::test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
async fn mssql_messages_row_counts_and_errors_arrive_in_order() {
    let h = harness().await;
    let c = profile(&h, &mssql(), false).await;
    let run = async |sql: &str| outline(&h.query(c, sql).await);

    assert_eq!(
        run("PRINT 'hello'; SELECT 1 AS a; PRINT 'after'").await,
        ["msg:hello", "result:0", "msg:after"]
    );
    assert_eq!(
        run("RAISERROR('warn10', 10, 1); SELECT 1 AS a; RAISERROR('info0', 0, 1) WITH NOWAIT")
            .await,
        ["msg:warn10", "result:0", "msg:info0"]
    );

    // row counts of every statement, not just a lone DML one
    assert_eq!(
        run("CREATE TABLE #it_150_counts (id int); \
             INSERT INTO #it_150_counts VALUES (1), (2); \
             UPDATE #it_150_counts SET id = 3; DELETE FROM #it_150_counts WHERE id = 3")
        .await,
        ["affected:2", "affected:2", "affected:2"]
    );
    assert_eq!(
        run("SET NOCOUNT ON; CREATE TABLE #it_150_counts (id int); \
             INSERT INTO #it_150_counts VALUES (1)")
        .await,
        ["msg:Commands completed successfully."]
    );
    let ev = h
        .query_on(
            &format!("/v1/connections/{c}/query"),
            json!({"sql": "UPDATE sales.products SET active = active WHERE id <= @P1", "params": [3]}),
            None,
        )
        .await;
    assert_eq!(outline(&ev), ["affected:3"]);

    // an error between the result sets around it
    assert_eq!(
        run("SELECT 1 AS a; RAISERROR('boom16', 16, 1); SELECT 3 AS c").await,
        [
            "result:0",
            "error:Msg 50000, Level 16, State 1, Line 1: boom16",
            "result:1"
        ]
    );

    // the next GO batch runs after a failed one; lines count from the
    // start of the script
    let ev = h
        .query(
            c,
            "SELECT 1 AS a\nGO\n\n\nSELECT\n  * FROM zz_it_150_nope\nGO\nSELECT 3 AS c",
        )
        .await;
    assert_eq!(
        outline(&ev),
        [
            "result:0",
            "error:Msg 208, Level 16, State 1, Line 5: Invalid object name 'zz_it_150_nope'.",
            "result:1"
        ]
    );
    assert_eq!(rows(&ev, 1), vec![vec![json!(3)]]);
    assert_eq!(error(&ev).unwrap().0, "db_error");
    assert!(!done(&ev).0);
    // still audited as a failure
    let page: sqail_proto::AuditPage = h.get_json("/v1/audit?limit=50").await;
    let entry = page
        .items
        .iter()
        .find(|e| {
            e.detail
                .as_deref()
                .is_some_and(|d| d.contains("zz_it_150_nope"))
        })
        .expect("query audited");
    assert!(!entry.success);

    // inside a procedure the line counts from the procedure, which is named
    let ev = run(
        "CREATE PROCEDURE #it_150_proc AS\nSELECT 1 AS a\nRAISERROR('in proc', 16, 1)\n\
                  GO\nPRINT 'before'\nEXEC #it_150_proc",
    )
    .await;
    assert_eq!(ev.len(), 4, "{ev:?}");
    assert_eq!(
        ev[..2],
        ["msg:Commands completed successfully.", "msg:before"]
    );
    assert_eq!(ev[2], "result:0");
    assert!(
        ev[3].starts_with("error:Msg 50000, Level 16, State 1, Procedure #it_150_proc")
            && ev[3].ends_with(", Line 3: in proc"),
        "{ev:?}"
    );
}

/// A profile for `user` on the test SQL Server (all dev logins share a
/// password).
async fn mssql_as(h: &Harness, user: &str) -> Uuid {
    let mut params = mssql().profile;
    params["auth"]["user"] = json!(user);
    h.create_connection(json!({
        "name": format!("mssql as {user}"),
        "params": params,
        "password": password("mssql"),
    }))
    .await
}

/// GET .../ddl: the script, or the problem's detail.
async fn ddl_of(h: &Harness, c: Uuid, schema: &str, name: &str) -> Result<String, String> {
    let res = h
        .get(&format!(
            "/v1/connections/{c}/ddl?schema={schema}&name={name}"
        ))
        .await;
    let ok = res.status().is_success();
    let body: Value = res.json().await.unwrap();
    if ok {
        Ok(body["ddl"].as_str().unwrap().to_string())
    } else {
        Err(body["detail"].as_str().unwrap_or_default().to_string())
    }
}

const MSSQL_LEAST: &str = "sqail_it_least";

/// Routines, a view and a trigger in schema `it_ddl`, plus a login that can
/// read and execute them but has no VIEW DEFINITION. Idempotent.
async fn mssql_ddl_fixture(h: &Harness) {
    let sa = mssql_as(h, "sa").await;
    let ev = h
        .query(
            sa,
            &format!(
                "IF SCHEMA_ID('it_ddl') IS NULL EXEC('CREATE SCHEMA it_ddl')
GO
IF OBJECT_ID('it_ddl.t') IS NULL CREATE TABLE it_ddl.t (id int)
GO
CREATE OR ALTER PROCEDURE it_ddl.p AS SELECT 1 AS one
GO
CREATE OR ALTER FUNCTION it_ddl.f (@x int) RETURNS int AS BEGIN RETURN @x + 1 END
GO
CREATE OR ALTER VIEW it_ddl.v AS SELECT id FROM it_ddl.t
GO
CREATE OR ALTER TRIGGER it_ddl.tr ON it_ddl.t AFTER INSERT AS SET NOCOUNT ON
GO
CREATE OR ALTER PROCEDURE it_ddl.enc WITH ENCRYPTION AS SELECT 2 AS two
GO
IF SUSER_ID('{MSSQL_LEAST}') IS NULL
    CREATE LOGIN {MSSQL_LEAST} WITH PASSWORD = '{pw}', CHECK_POLICY = OFF;
IF USER_ID('{MSSQL_LEAST}') IS NULL CREATE USER {MSSQL_LEAST} FOR LOGIN {MSSQL_LEAST};
ALTER ROLE db_datareader ADD MEMBER {MSSQL_LEAST};
GRANT EXECUTE ON SCHEMA::it_ddl TO {MSSQL_LEAST};
REVOKE VIEW DEFINITION ON SCHEMA::it_ddl FROM {MSSQL_LEAST};",
                pw = password("mssql").unwrap()
            ),
        )
        .await;
    assert!(error(&ev).is_none(), "{ev:?}");
}

#[tokio::test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
async fn mssql_routine_definitions() {
    let h = harness().await;
    mssql_ddl_fixture(&h).await;
    let owner = profile(&h, &mssql(), false).await;
    let least = mssql_as(&h, MSSQL_LEAST).await;

    // The least-privileged login sees every object…
    let ev = h
        .query(
            least,
            "SELECT count(*) FROM sys.objects WHERE schema_id = SCHEMA_ID('it_ddl') AND name IN ('p', 'f', 'v', 'tr', 'enc')",
        )
        .await;
    assert_eq!(rows(&ev, 0), vec![vec![json!(5)]], "{ev:?}");

    for (name, expect) in [
        ("p", "PROCEDURE it_ddl.p"),
        ("f", "FUNCTION it_ddl.f"),
        ("v", "VIEW it_ddl.v"),
        ("tr", "TRIGGER it_ddl.tr"),
    ] {
        // …the owner gets the source…
        let ddl = ddl_of(&h, owner, "it_ddl", name).await;
        assert!(
            ddl.as_ref().is_ok_and(|d| d.contains(expect)),
            "{name}: {ddl:?}"
        );
        // …and the other login is told which GRANT it lacks.
        let err = ddl_of(&h, least, "it_ddl", name).await.unwrap_err();
        assert!(
            err.contains(&format!(
                "[{MSSQL_LEAST}] lacks VIEW DEFINITION on [it_ddl].[{name}]"
            )),
            "{name}: {err}"
        );
        assert!(
            err.ends_with(&format!(
                "GRANT VIEW DEFINITION ON SCHEMA::[it_ddl] TO [{MSSQL_LEAST}];"
            )),
            "{name}: {err}"
        );
    }

    // Encrypted modules have no source for anyone, and that is what both hear.
    for c in [owner, least] {
        let err = ddl_of(&h, c, "it_ddl", "enc").await.unwrap_err();
        assert!(err.contains("WITH ENCRYPTION"), "{err}");
        assert!(!err.contains("GRANT"), "{err}");
    }

    // The suggested GRANT is enough.
    let sa = mssql_as(&h, "sa").await;
    let grant = |verb: &str| {
        format!(
            "{verb} VIEW DEFINITION ON SCHEMA::it_ddl {} {MSSQL_LEAST}",
            if verb == "GRANT" { "TO" } else { "FROM" }
        )
    };
    let ev = h.query(sa, &grant("GRANT")).await;
    assert!(error(&ev).is_none(), "{ev:?}");
    let ddl = ddl_of(&h, least, "it_ddl", "p").await;
    let ev = h.query(sa, &grant("REVOKE")).await;
    assert!(error(&ev).is_none(), "{ev:?}");
    assert!(
        ddl.as_ref().is_ok_and(|d| d.contains("PROCEDURE it_ddl.p")),
        "{ddl:?}"
    );
}

/// Whether result set `index` ended with `row_count` rows and was truncated.
fn truncated_at(events: &[QueryEvent], index: u32, row_count: u64) -> bool {
    events.iter().any(|e| {
        matches!(e, QueryEvent::ResultEnd { index: i, row_count: n, truncated: true }
            if *i == index && *n == row_count)
    })
}

/// Run `sql` with `max_rows` on `path`; the events and how long it took.
async fn timed(
    h: &Harness,
    path: &str,
    sql: &str,
    params: Value,
    max_rows: u64,
) -> (Vec<QueryEvent>, Duration) {
    let t = Instant::now();
    let ev = h
        .query_on(
            path,
            json!({"sql": sql, "params": params, "max_rows": max_rows}),
            None,
        )
        .await;
    let took = t.elapsed();
    eprintln!("{took:>10.2?}  max_rows={max_rows}  {sql}");
    (ev, took)
}

/// A big query with a small `max_rows` stops on the server instead of
/// streaming every row to the service (#59), and leaves the connection and
/// any open transaction as they were.
#[tokio::test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
async fn postgres_stops_reading_at_max_rows() {
    let h = harness().await;
    let c = profile(&h, &postgres(), false).await;
    let path = format!("/v1/connections/{c}/query");
    // 4M rows; materialising 2000 rows per generate_series is instant.
    let big = "SELECT a.g, md5((a.g * b.g)::text) FROM generate_series(1, 2000) a(g), \
               generate_series(1, 2000) b(g)";
    let (ev, took) = timed(&h, &path, big, json!([]), 100).await;
    assert!(error(&ev).is_none(), "{ev:?}");
    assert_eq!(rows(&ev, 0).len(), 100);
    assert_eq!(
        rows(&ev, 0)[0],
        vec![json!(1), json!("c4ca4238a0b923820dcc509a6f75849b")]
    );
    assert!(truncated_at(&ev, 0, 100), "{ev:?}");
    assert!(took < Duration::from_secs(2), "took {took:?}");

    // parameterised (binary) path
    let (ev, took) = timed(
        &h,
        &path,
        "SELECT a.g FROM generate_series(1, 2000) a(g), generate_series(1, $1::int) b(g)",
        json!([2000]),
        100,
    )
    .await;
    assert_eq!(rows(&ev, 0).len(), 100);
    assert!(truncated_at(&ev, 0, 100), "{ev:?}");
    assert!(took < Duration::from_secs(2), "took {took:?}");

    // exactly max_rows rows is not truncated
    let (ev, _) = timed(&h, &path, "SELECT generate_series(1, 100)", json!([]), 100).await;
    assert_eq!(rows(&ev, 0).len(), 100);
    assert!(ev.iter().any(|e| matches!(
        e,
        QueryEvent::ResultEnd {
            row_count: 100,
            truncated: false,
            ..
        }
    )));

    // scripts still truncate every result set and run every statement
    let (ev, _) = timed(
        &h,
        &path,
        "SELECT generate_series(1, 5000); SELECT generate_series(1, 3) AS n",
        json!([]),
        100,
    )
    .await;
    assert!(truncated_at(&ev, 0, 100), "{ev:?}");
    assert_eq!(rows(&ev, 1).len(), 3);

    // inside a session's transaction: the transaction stays open and usable
    let s: SessionInfo = h
        .post("/v1/sessions", &json!({"connection_id": c}), &h.admin)
        .await
        .json()
        .await
        .unwrap();
    let sp = format!("/v1/sessions/{}/query", s.id);
    let (ev, _) = timed(&h, &sp, "BEGIN", json!([]), 100).await;
    assert_eq!(done(&ev).1, Some(true));
    timed(
        &h,
        &sp,
        "CREATE TEMP TABLE it_cap (a int); INSERT INTO it_cap VALUES (1)",
        json!([]),
        100,
    )
    .await;
    let (ev, took) = timed(&h, &sp, big, json!([]), 100).await;
    assert!(truncated_at(&ev, 0, 100), "{ev:?}");
    assert!(took < Duration::from_secs(2), "took {took:?}");
    assert_eq!(done(&ev).1, Some(true), "transaction still open");
    let (ev, _) = timed(&h, &sp, "SELECT count(*) FROM it_cap", json!([]), 100).await;
    assert_eq!(rows(&ev, 0), vec![vec![json!(1)]], "{ev:?}");
    let (ev, _) = timed(&h, &sp, "ROLLBACK", json!([]), 100).await;
    assert_eq!(done(&ev).1, Some(false));
}

/// SQL Server: a single query is stopped with an Attention once it passes
/// `max_rows`; other batches skip converting the extra rows (#59).
#[tokio::test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
async fn mssql_stops_reading_at_max_rows() {
    let h = harness().await;
    let c = profile(&h, &mssql(), false).await;
    let path = format!("/v1/connections/{c}/query");
    // A few thousand objects squared: millions of rows.
    let big = "SELECT a.object_id, b.name FROM sys.all_objects a CROSS JOIN sys.all_objects b";
    let (ev, took) = timed(&h, &path, big, json!([]), 100).await;
    assert!(error(&ev).is_none(), "{ev:?}");
    assert_eq!(rows(&ev, 0).len(), 100);
    assert!(truncated_at(&ev, 0, 100), "{ev:?}");
    assert!(took < Duration::from_secs(2), "took {took:?}");

    // later batches of the script still run
    let (ev, took) = timed(
        &h,
        &path,
        &format!("{big}\nGO\nSELECT 1 AS one"),
        json!([]),
        100,
    )
    .await;
    assert!(truncated_at(&ev, 0, 100), "{ev:?}");
    assert_eq!(rows(&ev, 1), vec![vec![json!(1)]], "{ev:?}");
    assert!(took < Duration::from_secs(2), "took {took:?}");

    // a second statement in the batch: not cut short
    let (ev, _) = timed(
        &h,
        &path,
        "SELECT TOP 5000 a.object_id FROM sys.all_objects a CROSS JOIN sys.all_objects b\nSELECT 2 AS two",
        json!([]),
        100,
    )
    .await;
    assert!(truncated_at(&ev, 0, 100), "{ev:?}");
    assert_eq!(rows(&ev, 1), vec![vec![json!(2)]], "{ev:?}");

    // a session keeps its connection and transaction
    let s: SessionInfo = h
        .post("/v1/sessions", &json!({"connection_id": c}), &h.admin)
        .await
        .json()
        .await
        .unwrap();
    let sp = format!("/v1/sessions/{}/query", s.id);
    let (ev, _) = timed(
        &h,
        &sp,
        "BEGIN TRANSACTION; CREATE TABLE #it_cap (a int); INSERT INTO #it_cap VALUES (1)",
        json!([]),
        100,
    )
    .await;
    assert_eq!(done(&ev).1, Some(true), "{ev:?}");
    let (ev, took) = timed(&h, &sp, big, json!([]), 100).await;
    assert!(truncated_at(&ev, 0, 100), "{ev:?}");
    assert!(took < Duration::from_secs(2), "took {took:?}");
    assert_eq!(done(&ev).1, Some(true), "transaction still open: {ev:?}");
    let (ev, _) = timed(&h, &sp, "SELECT count(*) FROM #it_cap", json!([]), 100).await;
    assert_eq!(rows(&ev, 0), vec![vec![json!(1)]], "{ev:?}");
    // With XACT_ABORT on, an Attention would roll the transaction back, so
    // the query runs to the end instead.
    timed(&h, &sp, "SET XACT_ABORT ON", json!([]), 100).await;
    let (ev, _) = timed(
        &h,
        &sp,
        "SELECT TOP 20000 a.object_id FROM sys.all_objects a CROSS JOIN sys.all_objects b",
        json!([]),
        100,
    )
    .await;
    assert!(truncated_at(&ev, 0, 100), "{ev:?}");
    assert_eq!(done(&ev).1, Some(true), "transaction still open: {ev:?}");
    let (ev, _) = timed(&h, &sp, "SELECT count(*) FROM #it_cap", json!([]), 100).await;
    assert_eq!(rows(&ev, 0), vec![vec![json!(1)]], "{ev:?}");
    let (ev, _) = timed(&h, &sp, "ROLLBACK", json!([]), 100).await;
    assert_eq!(done(&ev).1, Some(false));
}

/// The first column of result set `index` holds a showplan.
fn is_showplan(events: &[QueryEvent], index: u32) -> bool {
    rows(events, index)
        .first()
        .and_then(|r| r.first())
        .and_then(Value::as_str)
        .is_some_and(|s| s.starts_with("<ShowPlanXML"))
}

/// With SHOWPLAN_XML on, the server answers the service's own probes
/// (@@TRANCOUNT, @@OPTIONS) with a plan; that must not panic (#147).
#[tokio::test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
async fn mssql_showplan_xml_does_not_break_the_probes() {
    let h = harness().await;
    let c = profile(&h, &mssql(), false).await;
    let path = format!("/v1/connections/{c}/query");
    // pooled; a lone query also runs the Attention probe
    for sql in [
        "SET SHOWPLAN_XML ON\nGO\nselect 1 a",
        "SET SHOWPLAN_XML ON\nGO\nselect 1 a; select 2 b",
    ] {
        let (ev, _) = timed(&h, &path, sql, json!([]), 100).await;
        assert!(error(&ev).is_none(), "{ev:?}");
        assert!(is_showplan(&ev, 0), "{ev:?}");
        assert_eq!(done(&ev), (false, None), "{ev:?}");
    }
    assert!(h.get("/v1/health").await.status().is_success());

    // a session keeps its connection and its transaction state
    let s: SessionInfo = h
        .post("/v1/sessions", &json!({"connection_id": c}), &h.admin)
        .await
        .json()
        .await
        .unwrap();
    let sp = format!("/v1/sessions/{}/query", s.id);
    let (ev, _) = timed(&h, &sp, "BEGIN TRANSACTION", json!([]), 100).await;
    assert_eq!(done(&ev).1, Some(true), "{ev:?}");
    let (ev, _) = timed(&h, &sp, "SET SHOWPLAN_XML ON", json!([]), 100).await;
    assert_eq!(done(&ev).1, Some(true), "{ev:?}");
    for sql in ["select 1 a", "SELECT @@TRANCOUNT"] {
        let (ev, _) = timed(&h, &sp, sql, json!([]), 100).await;
        assert!(error(&ev).is_none(), "{ev:?}");
        assert!(is_showplan(&ev, 0), "{ev:?}");
        assert!(
            !ev.iter()
                .any(|e| matches!(e, QueryEvent::Message { severity, .. } if severity == "error")),
            "{ev:?}"
        );
        assert_eq!(done(&ev).1, Some(true), "{ev:?}");
    }
    let (ev, _) = timed(&h, &sp, "SET SHOWPLAN_XML OFF", json!([]), 100).await;
    assert_eq!(done(&ev).1, Some(true), "{ev:?}");
    let (ev, _) = timed(&h, &sp, "SELECT @@TRANCOUNT", json!([]), 100).await;
    assert_eq!(rows(&ev, 0), vec![vec![json!(1)]], "{ev:?}");
    let (ev, _) = timed(&h, &sp, "ROLLBACK", json!([]), 100).await;
    assert_eq!(done(&ev).1, Some(false), "{ev:?}");
}

/// Run `body` on `path` and cancel it with DELETE /v1/queries after `after`.
/// The events and how long the query took.
async fn cancelled_after(
    h: &Harness,
    path: &str,
    body: Value,
    after: Duration,
) -> (Vec<QueryEvent>, Duration) {
    let qid = Uuid::new_v4();
    let (http, url, token) = (
        h.http.clone(),
        h.url(&format!("/v1/queries/{qid}")),
        h.admin.clone(),
    );
    let cancel = tokio::spawn(async move {
        tokio::time::sleep(after).await;
        http.delete(url)
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status()
    });
    let t = Instant::now();
    let ev = h.query_on(path, body, Some(qid)).await;
    let took = t.elapsed();
    assert_eq!(cancel.await.unwrap(), 202);
    (ev, took)
}

fn connection_lost(ev: &[QueryEvent]) -> bool {
    ev.iter().any(
        |e| matches!(e, QueryEvent::Message { text, .. } if text.contains("connection was lost")),
    )
}

/// Cancelling on SQL Server sends an Attention, which aborts the batch only:
/// the session keeps its connection, transaction and #temp tables, whether
/// or not the login may KILL (#148).
#[tokio::test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
async fn mssql_cancel_keeps_the_session() {
    let h = harness().await;
    for user in ["sqail", "sa"] {
        let c = mssql_as(&h, user).await;
        let s: SessionInfo = h
            .post("/v1/sessions", &json!({"connection_id": c}), &h.admin)
            .await
            .json()
            .await
            .unwrap();
        let sp = format!("/v1/sessions/{}/query", s.id);
        let run = |sql: &str| h.query_on(&sp, json!({ "sql": sql }), None);
        let ev = run("CREATE TABLE #it_cancel_148 (i int); BEGIN TRANSACTION; \
                      INSERT INTO #it_cancel_148 VALUES (2)")
        .await;
        assert_eq!(done(&ev).1, Some(true), "{user}: {ev:?}");
        let spid = rows(&run("SELECT @@SPID AS spid").await, 0);
        let still_there = async |what: &str| {
            let now = rows(&run("SELECT @@SPID AS spid").await, 0);
            assert_eq!(now, spid, "{user} {what}: same session");
            let ev = run("SELECT count(*) FROM #it_cancel_148").await;
            assert_eq!(rows(&ev, 0), vec![vec![json!(1)]], "{user} {what}: {ev:?}");
            assert_eq!(done(&ev).1, Some(true), "{user} {what}: transaction open");
        };

        // a batch that waits
        let (ev, took) = cancelled_after(
            &h,
            &sp,
            json!({"sql": "WAITFOR DELAY '00:00:30'; SELECT 1 AS one"}),
            Duration::from_millis(800),
        )
        .await;
        assert!(done(&ev).0, "{user}: {ev:?}");
        assert_eq!(error(&ev).unwrap().0, "cancelled", "{user}: {ev:?}");
        assert_eq!(done(&ev).1, Some(true), "{user}: {ev:?}");
        assert!(!connection_lost(&ev), "{user}: {ev:?}");
        // well before the KILL fallback (3 s)
        assert!(took < Duration::from_millis(2500), "{user}: took {took:?}");
        still_there("after waitfor").await;

        // a large result mid-stream: two statements, so that max_rows does
        // not stop it first
        let (ev, took) = cancelled_after(
            &h,
            &sp,
            json!({"sql": "SELECT 1 AS one; SELECT a.object_id, b.name, c.name AS c \
                           FROM sys.all_objects a CROSS JOIN sys.all_objects b \
                           CROSS JOIN sys.all_objects c",
                   "max_rows": 5_000_000}),
            Duration::from_millis(1500),
        )
        .await;
        assert!(done(&ev).0, "{user}: {:?}", ev.last());
        assert!(!rows(&ev, 1).is_empty(), "{user}: cancelled mid-stream");
        assert!(!connection_lost(&ev), "{user}");
        assert!(took < Duration::from_secs(4), "{user}: took {took:?}");
        still_there("after a large result").await;

        // the timeout cancels the same way
        let ev = h
            .query_on(
                &sp,
                json!({"sql": "WAITFOR DELAY '00:00:30'", "timeout_ms": 500}),
                None,
            )
            .await;
        assert_eq!(error(&ev).unwrap().0, "timeout", "{user}: {ev:?}");
        assert!(!connection_lost(&ev), "{user}: {ev:?}");
        still_there("after a timeout").await;

        // later GO batches do not run after a cancel
        let (ev, _) = cancelled_after(
            &h,
            &sp,
            json!({"sql": "WAITFOR DELAY '00:00:30'\nGO\nINSERT INTO #it_cancel_148 VALUES (3)"}),
            Duration::from_millis(500),
        )
        .await;
        assert!(done(&ev).0, "{user}: {ev:?}");
        still_there("after a cancelled script").await;

        let ev = run("ROLLBACK").await;
        assert_eq!(done(&ev).1, Some(false), "{user}: {ev:?}");
    }
}

/// `(name, type)` of the columns of result set `index`.
fn column_types(events: &[QueryEvent], index: u32) -> Vec<(String, String)> {
    events
        .iter()
        .find_map(|e| match e {
            QueryEvent::ResultStart { index: i, columns } if *i == index => Some(
                columns
                    .iter()
                    .map(|c| (c.name.clone(), c.type_name.clone()))
                    .collect(),
            ),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no result {index}: {events:?}"))
}

/// Values reach clients exactly and in a form SQL Server parses back (#151).
#[tokio::test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
async fn mssql_values_are_exact() {
    let h = harness().await;
    let c = profile(&h, &mssql(), false).await;
    let select = "select cast(922337203685477.5807 as money) g, cast(1.10 as money) q, \
         cast(-214748.3648 as smallmoney) sm, \
         cast('2024-02-29 13:45:12.997' as datetime) d, \
         cast('2024-02-29 13:45:29' as smalldatetime) sd, \
         cast('23:59:59.1234567' as time(7)) t7, cast('23:59:59' as time(0)) t0, \
         cast('9999-12-31 23:59:59.9999999' as datetime2(7)) d7, \
         cast('2024-01-02 03:04:05.12' as datetime2(2)) d2, \
         cast('2024-01-02 03:04:05.1234567 +02:00' as datetimeoffset(7)) o7, \
         cast('2024-01-02 03:04:05 -05:30' as datetimeoffset(0)) o0, \
         cast(0 as decimal(38,0)) n0, cast(99999999999999999999999999999999999999 as decimal(38,0)) n38, \
         cast(-1.50 as decimal(5,2)) n2, cast(0.1 as real) r, cast(1e300 as float) f, \
         hierarchyid::Parse('/1/2/') h, 1 + 1";
    let ev = h.query(c, select).await;
    assert!(error(&ev).is_none(), "{ev:?}");
    let row = rows(&ev, 0).remove(0);
    assert_eq!(
        row,
        vec![
            json!("922337203685477.5807"),
            json!("1.1000"),
            json!("-214748.3648"),
            json!("2024-02-29T13:45:12.997"),
            json!("2024-02-29T13:45:00"),
            json!("23:59:59.1234567"),
            json!("23:59:59"),
            json!("9999-12-31T23:59:59.9999999"),
            json!("2024-01-02T03:04:05.12"),
            json!("2024-01-02T03:04:05.1234567+02:00"),
            json!("2024-01-02T03:04:05-05:30"),
            json!("0"),
            json!("99999999999999999999999999999999999999"),
            json!("-1.50"),
            json!(0.1),
            json!(1e300),
            json!("5b40"),
            json!(2),
        ],
        "{ev:?}"
    );
    let types: Vec<String> = column_types(&ev, 0).into_iter().map(|(_, t)| t).collect();
    assert_eq!(
        types,
        [
            "money",
            "money",
            "smallmoney",
            "datetime",
            "smalldatetime",
            "time",
            "time",
            "datetime2",
            "datetime2",
            "datetimeoffset",
            "datetimeoffset",
            "decimal",
            "decimal",
            "decimal",
            "real",
            "float",
            "udt",
            "int"
        ]
    );
    assert_eq!(column_types(&ev, 0)[17].0, "(No column name)");

    // The text goes back into SQL Server as the same value.
    let texts: Vec<String> = row[..14]
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let back = format!(
        "select case when cast('{}' as money) = cast(922337203685477.5807 as money) \
           and cast('{}' as datetime) = cast('2024-02-29 13:45:12.997' as datetime) \
           and cast('{}' as datetime2(7)) = cast('9999-12-31 23:59:59.9999999' as datetime2(7)) \
           and cast('{}' as datetimeoffset(7)) = cast('2024-01-02 03:04:05.1234567 +02:00' as datetimeoffset(7)) \
           and cast('{}' as time(7)) = cast('23:59:59.1234567' as time(7)) \
           then 1 else 0 end ok",
        texts[0], texts[3], texts[7], texts[9], texts[5]
    );
    let ev = h.query(c, &back).await;
    assert_eq!(rows(&ev, 0), vec![vec![json!(1)]], "{ev:?}");

    // nullable columns of a table take the same paths
    let t = format!("#it_values_{}", Uuid::new_v4().simple());
    let ev = h
        .query(
            c,
            &format!(
                "create table {t} (m money null, sm smallmoney null, d datetime null, sd smalldatetime null); \
                 insert into {t} values (1.10, 2.5, '2024-02-29 13:45:12.997', '2024-02-29 13:45'), (null, null, null, null); \
                 select * from {t}"
            ),
        )
        .await;
    assert!(error(&ev).is_none(), "{ev:?}");
    assert_eq!(
        rows(&ev, 0),
        vec![
            vec![
                json!("1.1000"),
                json!("2.5000"),
                json!("2024-02-29T13:45:12.997"),
                json!("2024-02-29T13:45:00")
            ],
            vec![Value::Null, Value::Null, Value::Null, Value::Null],
        ]
    );
    let types: Vec<String> = column_types(&ev, 0).into_iter().map(|(_, t)| t).collect();
    assert_eq!(types, ["money", "smallmoney", "datetime", "smalldatetime"]);
}

/// PostgreSQL: non-finite floats as the server spells them, and the names of
/// the database's own types (#151).
#[tokio::test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
async fn postgres_float_and_type_names() {
    let h = harness().await;
    let c = profile(&h, &postgres(), false).await;
    let ev = h
        .query(
            c,
            "select '-Infinity'::float8 a, 'Infinity'::float4 b, 'NaN'::float8 n, 2^70 p, 1e-320::float8 s",
        )
        .await;
    assert_eq!(
        rows(&ev, 0),
        vec![vec![
            json!("-Infinity"),
            json!("Infinity"),
            json!("NaN"),
            json!(1.1805916207174113e21),
            json!(1e-320)
        ]],
        "{ev:?}"
    );
    // float4 through the binary path keeps its own digits
    let ev = h
        .query_on(
            &format!("/v1/connections/{c}/query"),
            json!({"sql": "select 0.1::float4 r, $1::int i", "params": [0]}),
            None,
        )
        .await;
    assert_eq!(rows(&ev, 0), vec![vec![json!(0.1), json!(0)]], "{ev:?}");

    // A type created in a session: named from the next script on.
    let s: SessionInfo = h
        .post("/v1/sessions", &json!({"connection_id": c}), &h.admin)
        .await
        .json()
        .await
        .unwrap();
    let sp = format!("/v1/sessions/{}/query", s.id);
    let ty = format!("it_mood_{}", Uuid::new_v4().simple());
    let ev = h
        .query_on(
            &sp,
            json!({"sql": format!("create type pg_temp.{ty} as enum ('ok')")}),
            None,
        )
        .await;
    assert!(error(&ev).is_none(), "{ev:?}");
    for _ in 0..2 {
        let ev = h
            .query_on(
                &sp,
                json!({"sql": format!("select 'ok'::pg_temp.{ty} m, array['ok']::pg_temp.{ty}[] a")}),
                None,
            )
            .await;
        assert_eq!(rows(&ev, 0), vec![vec![json!("ok"), json!("{ok}")]]);
        let types: Vec<String> = column_types(&ev, 0).into_iter().map(|(_, t)| t).collect();
        assert!(
            types[0].ends_with(&ty) && types[1].ends_with(&format!("{ty}[]")),
            "{types:?}"
        );
    }
    // the parameterised path names it too
    let ev = h
        .query_on(
            &sp,
            json!({"sql": format!("select 'ok'::pg_temp.{ty} m, $1::int i"), "params": [1]}),
            None,
        )
        .await;
    assert_eq!(column_types(&ev, 0)[0].1, ty, "{ev:?}");

    // A script runs statement by statement, so a type it creates is named
    // in its later statements already (#149).
    let ty = format!("it_mood_{}", Uuid::new_v4().simple());
    let ev = h
        .query_on(
            &sp,
            json!({"sql": format!(
                "create type pg_temp.{ty} as enum ('ok'); create temp table it_mood_t (i int); \
                 select 'ok'::pg_temp.{ty} m"
            )}),
            None,
        )
        .await;
    assert!(error(&ev).is_none(), "{ev:?}");
    assert!(column_types(&ev, 0)[0].1.ends_with(&ty), "{ev:?}");
}

/// A PostgreSQL script runs statement by statement like in psql: a failing
/// statement does not roll back the ones before it, unless the script
/// opened a transaction (#149).
#[tokio::test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
async fn postgres_scripts_commit_each_statement() {
    let h = harness().await;
    let c = profile(&h, &postgres(), false).await;
    let t = format!("sales.it_autocommit_{}", Uuid::new_v4().simple());
    let count = |ev: &[QueryEvent]| rows(ev, 0)[0][0].clone();
    let ev = h.query(c, &format!("create table {t} (i int)")).await;
    assert!(error(&ev).is_none(), "{ev:?}");

    let ev = h
        .query(
            c,
            &format!("insert into {t} values (1); insert into {t} values (2); select 1/0"),
        )
        .await;
    assert_eq!(affected(&ev), [1, 1], "{ev:?}");
    assert_eq!(
        error(&ev).map(|e| e.1),
        Some("ERROR: division by zero".to_string())
    );
    let ev = h.query(c, &format!("select count(*) from {t}")).await;
    assert_eq!(count(&ev), json!(2), "earlier statements stay: {ev:?}");

    // A transaction the script opens is rolled back as a whole.
    let ev = h
        .query(
            c,
            &format!("begin; insert into {t} values (3); select 1/0; commit"),
        )
        .await;
    assert!(error(&ev).is_some(), "{ev:?}");
    let ev = h.query(c, &format!("select count(*) from {t}")).await;
    assert_eq!(count(&ev), json!(2), "{ev:?}");
    // ...and what it committed stays.
    let ev = h
        .query(
            c,
            &format!("begin; insert into {t} values (4); commit; select 1/0"),
        )
        .await;
    assert!(error(&ev).is_some(), "{ev:?}");
    let ev = h.query(c, &format!("select count(*) from {t}")).await;
    assert_eq!(count(&ev), json!(3), "{ev:?}");

    // Statements that refuse transaction blocks run inside a script.
    let ev = h.query(c, &format!("select 1; vacuum {t}; select 2")).await;
    assert!(error(&ev).is_none(), "{ev:?}");
    assert_eq!(rows(&ev, 1), vec![vec![json!(2)]], "{ev:?}");

    // Results, notices and row counts keep their order.
    let ev = h
        .query(
            c,
            &format!(
                "select 1 a; do $$begin raise notice 'between'; end$$; \
                 update {t} set i = i; select 2 b"
            ),
        )
        .await;
    let kinds: Vec<String> = ev
        .iter()
        .filter_map(|e| match e {
            QueryEvent::ResultStart { index, .. } => Some(format!("result {index}")),
            QueryEvent::Message { text, .. } => Some(text.clone()),
            QueryEvent::RowsAffected { count } => Some(format!("affected {count}")),
            _ => None,
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "result 0",
            "between",
            "affected 0",
            "affected 3",
            "result 1"
        ],
        "{ev:?}"
    );

    // A function body with semicolons is one statement.
    let ev = h
        .query(
            c,
            "create function pg_temp.it_atomic() returns int language sql \
             begin atomic select 1; select case when true then 2 end; end; \
             select pg_temp.it_atomic()",
        )
        .await;
    assert!(error(&ev).is_none(), "{ev:?}");
    assert_eq!(rows(&ev, 0), vec![vec![json!(2)]], "{ev:?}");

    // Error positions count from the start of the script.
    let ev = h.query(c, "select 1; select it_no_such_column").await;
    let msg = error(&ev).unwrap().1;
    assert!(msg.ends_with("POSITION: 18"), "{msg}");

    // Where the statements depend on standard_conforming_strings, the script
    // runs as one message, and the error says what that means.
    let ev = h
        .query(
            c,
            &format!(r"insert into {t} values (6); select 'a\'; select 1/0; -- '"),
        )
        .await;
    let msg = error(&ev).unwrap().1;
    assert!(
        msg.starts_with("ERROR: division by zero") && msg.contains("ran as one transaction"),
        "{msg}"
    );
    let ev = h.query(c, &format!("select count(*) from {t}")).await;
    assert_eq!(count(&ev), json!(3), "{ev:?}");

    // max_rows still caps each result set.
    let (ev, _) = timed(
        &h,
        &format!("/v1/connections/{c}/query"),
        "select generate_series(1, 10); select 2",
        json!([]),
        5,
    )
    .await;
    assert!(truncated_at(&ev, 0, 5), "{ev:?}");
    assert_eq!(rows(&ev, 1), vec![vec![json!(2)]], "{ev:?}");

    // A session keeps a transaction the script left open.
    let s: SessionInfo = h
        .post("/v1/sessions", &json!({"connection_id": c}), &h.admin)
        .await
        .json()
        .await
        .unwrap();
    let sp = format!("/v1/sessions/{}/query", s.id);
    let ev = h
        .query_on(
            &sp,
            json!({"sql": format!("begin; insert into {t} values (5); select 1")}),
            None,
        )
        .await;
    assert_eq!(done(&ev).1, Some(true), "{ev:?}");
    let ev = h
        .query_on(&sp, json!({"sql": "rollback; select 1"}), None)
        .await;
    assert_eq!(done(&ev).1, Some(false), "{ev:?}");

    let ev = h
        .query(c, &format!("select count(*) from {t}; drop table {t}"))
        .await;
    assert_eq!(count(&ev), json!(3), "{ev:?}");
}

/// `COPY … FROM STDIN` (pasted pg_dump output) is refused before anything
/// runs, leaving a session's transaction and temp tables; `COPY … TO
/// STDOUT` is a one-column result.
#[tokio::test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
async fn postgres_copy_stdin_and_stdout() {
    let h = harness().await;
    let c = profile(&h, &postgres(), false).await;
    let s: SessionInfo = h
        .post("/v1/sessions", &json!({"connection_id": c}), &h.admin)
        .await
        .json()
        .await
        .unwrap();
    let path = format!("/v1/sessions/{}/query", s.id);
    let run = |sql: &str| h.query_on(&path, json!({ "sql": sql }), None);
    let ev = run("BEGIN; CREATE TEMP TABLE copy187 (a int, b text); \
                  INSERT INTO copy187 VALUES (1, 'x'), (2, 'y,z')")
    .await;
    assert_eq!(done(&ev).1, Some(true), "{ev:?}");

    // As pg_dump writes it, after a statement that must not run either.
    let ev = run("INSERT INTO copy187 VALUES (3, 'no');\n\
                  COPY copy187 (a, b) FROM stdin;\n4\tw\n\\.\n")
    .await;
    let (code, message) = error(&ev).expect("refused");
    assert_eq!(code, "unsupported");
    assert!(
        message.contains("FROM STDIN") && message.contains("Import CSV"),
        "{message}"
    );
    assert_eq!(done(&ev).1, Some(true), "the transaction survives: {ev:?}");
    assert!(!connection_lost(&ev), "{ev:?}");

    let ev = run("copy copy187 to stdout (format binary)").await;
    assert_eq!(error(&ev).unwrap().0, "unsupported");

    let ev = run(
        "SELECT count(*) FROM copy187; COPY (SELECT * FROM copy187 ORDER BY a) TO STDOUT; \
                  copy copy187 to stdout with (format csv, header); SELECT 'after'",
    )
    .await;
    assert_eq!(error(&ev), None, "{ev:?}");
    assert_eq!(rows(&ev, 0), vec![vec![json!(2)]]);
    assert_eq!(column_types(&ev, 1), vec![("copy".into(), "text".into())]);
    assert_eq!(
        rows(&ev, 1),
        vec![vec![json!("1\tx")], vec![json!("2\ty,z")]]
    );
    assert_eq!(
        rows(&ev, 2),
        vec![
            vec![json!("a,b")],
            vec![json!("1,x")],
            vec![json!("2,\"y,z\"")]
        ]
    );
    assert_eq!(rows(&ev, 3), vec![vec![json!("after")]]);
    assert_eq!(done(&ev).1, Some(true));

    // A COPY error leaves a usable session, as any other error does.
    let ev = run("ROLLBACK; COPY no_such_table_187 TO STDOUT").await;
    assert!(
        error(&ev).unwrap().1.contains("no_such_table_187"),
        "{ev:?}"
    );
    let ev = run("COPY (SELECT 1) TO STDOUT").await;
    assert_eq!(rows(&ev, 0), vec![vec![json!("1")]]);
    assert_eq!(done(&ev).1, Some(false));
}

#[tokio::test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
async fn postgres_types_and_notices() {
    let h = harness().await;
    let c = profile(&h, &postgres(), false).await;
    let ev = h
        .query(c, "SELECT c_numeric, c_bytes, c_uuid, c_bool FROM sales.type_zoo WHERE id = 1; DO $$BEGIN RAISE NOTICE 'hi'; END$$")
        .await;
    assert_eq!(
        rows(&ev, 0),
        vec![vec![
            json!("12345678901234567890.0123456789"),
            json!("deadbeef"),
            json!("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11"),
            json!(true)
        ]]
    );
    assert!(
        ev.iter()
            .any(|e| matches!(e, QueryEvent::Message { text, .. } if text == "hi"))
    );
    // the same through the binary (parameterised) path
    let ev = h
        .query_on(
            &format!("/v1/connections/{c}/query"),
            json!({"sql": "SELECT c_numeric, c_bytes, c_uuid, c_bool FROM sales.type_zoo WHERE id = $1", "params": [1]}),
            None,
        )
        .await;
    assert_eq!(
        rows(&ev, 0),
        vec![vec![
            json!("12345678901234567890.0123456789"),
            json!("deadbeef"),
            json!("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11"),
            json!(true)
        ]]
    );
}

/// One pooled connection per profile, so every request reuses the same one.
async fn single_connection_harness(request_timeout_secs: u64) -> Harness {
    harness_with(|c| {
        c.limits.pool_size = 1;
        c.limits.request_timeout_secs = request_timeout_secs;
    })
    .await
}

fn one(ev: &[QueryEvent]) -> Value {
    assert!(error(ev).is_none(), "{ev:?}");
    rows(ev, 0)[0][0].clone()
}

async fn columns_of(h: &Harness, c: Uuid, qs: &str) -> Vec<ColumnInfo> {
    h.get_json(&format!("/v1/connections/{c}/schema/columns?{qs}"))
        .await
}

#[tokio::test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
async fn postgres_pooled_connections_are_reset() {
    let h = single_connection_harness(60).await;
    let c = profile(&h, &postgres(), false).await;
    let pid = one(&h.query(c, "SELECT pg_backend_pid()").await);
    let ev = h
        .query(
            c,
            "SET search_path = sales; SET application_name = 'leak'; \
             CREATE TEMP TABLE it_leak (a int); PREPARE it_leak_p AS SELECT 1; \
             SELECT pg_advisory_lock(4242)",
        )
        .await;
    assert!(error(&ev).is_none(), "{ev:?}");
    // The script's search_path would make `customers` resolve to sales.
    assert!(columns_of(&h, c, "name=customers").await.is_empty());
    assert!(
        !columns_of(&h, c, "schema=sales&name=customers")
            .await
            .is_empty()
    );
    let ev = h
        .query(
            c,
            "SELECT pg_backend_pid(), current_setting('search_path'), \
             current_setting('application_name'), to_regclass('pg_temp.it_leak') IS NULL, \
             (SELECT count(*) FROM pg_prepared_statements WHERE name = 'it_leak_p'), \
             (SELECT count(*) FROM pg_locks WHERE locktype = 'advisory' AND pid = pg_backend_pid())",
        )
        .await;
    assert_eq!(
        rows(&ev, 0),
        vec![vec![
            pid,
            json!("\"$user\", public"),
            json!("sqail"),
            json!(true),
            json!(0),
            json!(0)
        ]],
        "same connection, nothing left over"
    );
    // The reset keeps the driver's own prepared statements: the binary
    // (parameterised) path still works on the reused connection.
    let ev = h
        .query_on(
            &format!("/v1/connections/{c}/query"),
            json!({"sql": "SELECT c_uuid FROM sales.type_zoo WHERE id = $1", "params": [1]}),
            None,
        )
        .await;
    assert_eq!(one(&ev), json!("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11"));
}

#[tokio::test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
async fn mssql_connections_that_ran_scripts_are_not_reused() {
    let h = single_connection_harness(60).await;
    let c = profile(&h, &mssql(), false).await;
    let ev = h
        .query(
            c,
            "USE master; SET LANGUAGE Deutsch; SET ROWCOUNT 1; CREATE TABLE #it_leak (a int)",
        )
        .await;
    assert!(error(&ev).is_none(), "{ev:?}");
    let cols = columns_of(&h, c, "schema=sales&name=customers").await;
    assert!(cols.len() > 1, "{cols:?}");
    let ev = h
        .query(
            c,
            "SELECT DB_NAME(), @@LANGUAGE, CASE WHEN OBJECT_ID('tempdb..#it_leak') IS NULL THEN 1 ELSE 0 END",
        )
        .await;
    // Not @@SPID: SQL Server hands a closed session's id to the next login,
    // so a fresh connection can get the same one. None of the script's
    // state survives, which is what reuse would have kept.
    assert_eq!(
        rows(&ev, 0)[0],
        [json!("sqail_test"), json!("us_english"), json!(1)],
        "the script's connection was reused"
    );
    let ev = h.query(c, "SELECT TOP 3 id FROM sales.customers").await;
    assert_eq!(rows(&ev, 0).len(), 3);
}

/// An explain that outlives the request timeout, then a normal query on the
/// same single-connection profile.
async fn explain_timeout_leaves_a_clean_pool(d: Dialect, sleep: &str) {
    let h = single_connection_harness(2).await;
    let c = profile(&h, &d, false).await;
    let started = Instant::now();
    let res = h
        .post(
            &format!("/v1/connections/{c}/explain"),
            &json!({"sql": sleep, "analyze": true}),
            &h.admin,
        )
        .await;
    assert_eq!(res.status(), 504, "{}", res.text().await.unwrap());
    assert!(started.elapsed() < Duration::from_secs(8));
    let started = Instant::now();
    let ev = h
        .query(c, &format!("SELECT count(*) FROM {}", (d.t)("customers")))
        .await;
    assert!(error(&ev).is_none(), "{ev:?}");
    assert_eq!(rows(&ev, 0).len(), 1, "rows, not a plan: {ev:?}");
    assert_eq!(done(&ev).1, None);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the next query waited for the abandoned one"
    );
    let tx = if d.engine == "mssql" {
        "SELECT @@TRANCOUNT"
    } else {
        // Two statements, so it runs with the simple protocol: a lone query
        // goes through the extended one, where every message restarts
        // statement_timestamp() (#59).
        "SELECT CASE WHEN now() = statement_timestamp() THEN 0 ELSE 1 END; SELECT 1"
    };
    assert_eq!(one(&h.query(c, tx).await), json!(0));
    // Catalog calls work too.
    assert!(
        !columns_of(&h, c, "schema=sales&name=customers")
            .await
            .is_empty()
    );
}

#[tokio::test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
async fn postgres_explain_past_the_request_timeout() {
    explain_timeout_leaves_a_clean_pool(postgres(), "SELECT pg_sleep(20)").await;
}

#[tokio::test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
async fn mssql_explain_past_the_request_timeout() {
    explain_timeout_leaves_a_clean_pool(mssql(), "WAITFOR DELAY '00:00:20'").await;
}

#[tokio::test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
async fn postgres_read_only_profiles_stay_read_only() {
    let h = single_connection_harness(60).await;
    let ro = profile(&h, &postgres(), true).await;
    let pid = one(&h.query(ro, "SELECT pg_backend_pid()").await);
    let attempts = [
        "SET default_transaction_read_only = off",
        "SET SESSION default_transaction_read_only TO false",
        "SET SESSION CHARACTERISTICS AS TRANSACTION READ WRITE",
        "BEGIN READ WRITE; UPDATE sales.products SET active = active WHERE id = 1; COMMIT",
        "START TRANSACTION READ WRITE",
        "SET TRANSACTION READ WRITE",
        "RESET ALL",
        "SELECT set_config('default_transaction_read_only', 'off', false)",
        "SELECT 1; SET default_transaction_read_only = off",
    ];
    let session: SessionInfo = {
        let res = h
            .post("/v1/sessions", &json!({"connection_id": ro}), &h.admin)
            .await;
        assert_eq!(res.status(), 201);
        res.json().await.unwrap()
    };
    for sql in attempts {
        for path in [
            format!("/v1/connections/{ro}/query"),
            format!("/v1/sessions/{}/query", session.id),
        ] {
            let res = h.post(&path, &json!({"sql": sql}), &h.admin).await;
            assert_eq!(res.status(), 403, "{path}: {sql}");
        }
        let res = h
            .post(
                &format!("/v1/connections/{ro}/explain"),
                &json!({"sql": sql, "analyze": true}),
                &h.admin,
            )
            .await;
        assert_eq!(res.status(), 403, "explain: {sql}");
    }
    // Writable profiles are not restricted.
    let rw = profile(&h, &postgres(), false).await;
    let ev = h
        .query(
            rw,
            "SET default_transaction_read_only = off; SHOW transaction_read_only",
        )
        .await;
    assert_eq!(one(&ev), json!("off"));

    // The read-only connection is still the same and still read-only.
    let ev = h
        .query(
            ro,
            "SELECT pg_backend_pid(), current_setting('default_transaction_read_only')",
        )
        .await;
    assert_eq!(rows(&ev, 0), vec![vec![pid, json!("on")]]);
    let ev = h
        .query(ro, "UPDATE sales.products SET active = active WHERE id = 1")
        .await;
    assert!(error(&ev).is_some(), "write on a read-only profile");
}

/// `docs/openapi.json` is the published API reference; it must match what
/// the service serves. Regenerate with `SQAIL_BLESS=1 cargo test -p
/// sqail-service --test it openapi`.
#[tokio::test]
async fn published_openapi_document_is_current() {
    let h = harness().await;
    let served: Value = h
        .http
        .get(h.url("/v1/openapi.json"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let path = repo_root().join("docs/openapi.json");
    let pretty = serde_json::to_string_pretty(&served).unwrap() + "\n";
    if std::env::var_os("SQAIL_BLESS").is_some() {
        std::fs::write(&path, &pretty).unwrap();
        return;
    }
    let published: Value = serde_json::from_str(
        &std::fs::read_to_string(&path).expect("docs/openapi.json exists (SQAIL_BLESS=1)"),
    )
    .unwrap();
    assert!(
        published == served,
        "docs/openapi.json is stale; regenerate with SQAIL_BLESS=1"
    );
}

#[tokio::test]
async fn no_bootstrap_token_when_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let server = sqail_service::start(Config {
        data_dir: dir.path().to_path_buf(),
        bind: "127.0.0.1:0".parse().unwrap(),
        bootstrap_admin_token: false,
        ..Default::default()
    })
    .await
    .unwrap();
    assert!(server.bootstrap_token.is_none());
}

// ======================================================= script CREATE ==

/// Run `sql`, failing on any error.
async fn exec(h: &Harness, c: Uuid, sql: &str) -> Vec<QueryEvent> {
    let ev = h.query(c, sql).await;
    if let Some((_, msg)) = error(&ev) {
        panic!("{msg}\n--- in ---\n{sql}");
    }
    ev
}

/// Script each table of schema `from` with *Script CREATE*, run the scripts
/// with the schema renamed to `to`, and compare what `describe` (one text
/// row per catalog fact, for a `schema`, `table` pair) says about both.
async fn script_round_trip(
    h: &Harness,
    c: Uuid,
    from: &str,
    to: &str,
    tables: &[&str],
    describe: fn(&str, &str) -> String,
) {
    for t in tables {
        let ddl = ddl_of(h, c, from, t).await.unwrap();
        exec(h, c, &ddl.replace(from, to)).await;
    }
    for t in tables {
        let a = rows(&exec(h, c, &describe(from, t)).await, 0);
        let b = rows(&exec(h, c, &describe(to, t)).await, 0);
        let text = |rows: Vec<Vec<Value>>, schema: &str| -> Vec<String> {
            rows.iter()
                .map(|r| {
                    r[0].as_str()
                        .unwrap_or_default()
                        .replace(schema, "<schema>")
                })
                .collect()
        };
        let (a, b) = (text(a, from), text(b, to));
        assert!(!a.is_empty(), "nothing describes {from}.{t}");
        assert_eq!(
            a,
            b,
            "{t}: the scripted copy differs\n{}",
            ddl_of(h, c, from, t).await.unwrap()
        );
    }
}

fn pg_describe(schema: &str, table: &str) -> String {
    let q = format!("{schema}.{table}");
    format!(
        "SELECT 'col ' || a.attname || ' ' || format_type(a.atttypid, a.atttypmod) || ' '
                || a.attnotnull || ' ' || a.attidentity::text || ' ' || a.attgenerated::text || ' '
                || coalesce(pg_get_expr(d.adbin, d.adrelid), '-') || ' '
                || coalesce(co.collname, '-') || ' '
                || coalesce(col_description(a.attrelid, a.attnum), '-') || ' '
                || coalesce((SELECT sq.seqstart || '/' || sq.seqincrement FROM pg_sequence sq
                             WHERE sq.seqrelid = pg_get_serial_sequence('{q}', a.attname)::regclass), '-')
         FROM pg_attribute a
         LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
         LEFT JOIN pg_collation co ON co.oid = a.attcollation
         WHERE a.attrelid = '{q}'::regclass AND a.attnum > 0 AND NOT a.attisdropped
         UNION ALL
         SELECT 'con ' || conname || ' ' || contype::text || ' ' || pg_get_constraintdef(oid)
         FROM pg_constraint WHERE conrelid = '{q}'::regclass AND contype <> 'n'
         UNION ALL
         SELECT 'ix ' || indisvalid || ' ' || pg_get_indexdef(indexrelid)
         FROM pg_index WHERE indrelid = '{q}'::regclass
         UNION ALL
         SELECT 'rel ' || relkind::text || ' ' || relpersistence::text || ' ' || relispartition || ' '
                || coalesce(pg_get_partkeydef(oid), '-') || ' '
                || coalesce(pg_get_expr(relpartbound, oid), '-') || ' '
                || coalesce(obj_description(oid, 'pg_class'), '-')
         FROM pg_class WHERE oid = '{q}'::regclass
         ORDER BY 1"
    )
}

#[tokio::test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
async fn postgres_script_create_recreates_the_table() {
    let h = harness().await;
    let c = profile(&h, &postgres(), false).await;
    let tag = &Uuid::new_v4().simple().to_string()[..8];
    let (a, b) = (format!("it171_{tag}_a"), format!("it171_{tag}_b"));
    exec(
        &h,
        c,
        &format!(
            r#"CREATE SCHEMA {a}; CREATE SCHEMA {b};
            CREATE TABLE {a}.parent (
                id integer GENERATED ALWAYS AS IDENTITY (START WITH 10 INCREMENT BY 5) PRIMARY KEY,
                price numeric(10,2), qty int CHECK (qty >= 0),
                total numeric GENERATED ALWAYS AS (price * qty) STORED,
                s serial, big bigserial,
                "user" text COLLATE "C" UNIQUE, "array" int DEFAULT 1,
                b bigint GENERATED BY DEFAULT AS IDENTITY);
            COMMENT ON TABLE {a}.parent IS 'it''s the parent';
            COMMENT ON COLUMN {a}.parent."user" IS 'who';
            CREATE INDEX ix_parent ON {a}.parent (qty DESC, price) INCLUDE ("array") WHERE qty > 0;
            CREATE TABLE {a}.child (
                id int PRIMARY KEY,
                pid int REFERENCES {a}.parent (id) ON DELETE CASCADE ON UPDATE SET NULL);
            CREATE UNLOGGED TABLE {a}.events (id int, at date NOT NULL) PARTITION BY RANGE (at);
            CREATE INDEX ix_events ON {a}.events (at);
            CREATE TABLE {a}.events_2026 PARTITION OF {a}.events
                FOR VALUES FROM ('2026-01-01') TO ('2027-01-01');"#
        ),
    )
    .await;
    script_round_trip(
        &h,
        c,
        &a,
        &b,
        &["parent", "child", "events", "events_2026"],
        pg_describe,
    )
    .await;
    // The copy works: identity and serials fill themselves in.
    exec(
        &h,
        c,
        &format!(r#"INSERT INTO {b}.parent (price, qty, "user") VALUES (2, 3, 'x')"#),
    )
    .await;
    let r = rows(
        &exec(
            &h,
            c,
            &format!(r#"SELECT id, total::text, s, big, "array" FROM {b}.parent"#),
        )
        .await,
        0,
    );
    assert_eq!(
        r,
        [vec![json!(10), json!("6.00"), json!(1), json!(1), json!(1)]]
    );
    exec(
        &h,
        c,
        &format!("DROP SCHEMA {a} CASCADE; DROP SCHEMA {b} CASCADE"),
    )
    .await;
}

fn ms_describe(schema: &str, table: &str) -> String {
    let q = format!("{schema}.{table}");
    format!(
        "SELECT CONCAT('col ', c.name, ' ', t.name, ' ', c.max_length, ' ', c.precision, ' ', c.scale,
                       ' ', c.is_nullable, ' ', c.is_identity, ' ', CAST(ic.seed_value AS nvarchar(20)),
                       ' ', CAST(ic.increment_value AS nvarchar(20)), ' ', c.collation_name,
                       ' ', cc.definition, ' ', cc.is_persisted, ' ', dc.definition,
                       ' ', IIF(dc.is_system_named = 1, '*', dc.name)) COLLATE DATABASE_DEFAULT
         FROM sys.columns c
         JOIN sys.types t ON t.user_type_id = c.user_type_id
         LEFT JOIN sys.identity_columns ic ON ic.object_id = c.object_id AND ic.column_id = c.column_id
         LEFT JOIN sys.computed_columns cc ON cc.object_id = c.object_id AND cc.column_id = c.column_id
         LEFT JOIN sys.default_constraints dc ON dc.object_id = c.default_object_id
         WHERE c.object_id = OBJECT_ID('{q}')
         UNION ALL
         SELECT CONCAT('ix ', IIF(k.is_system_named = 1, '*', i.name), ' ', i.type_desc, ' ', i.is_unique,
                       ' ', i.is_primary_key, ' ', i.is_unique_constraint, ' ', i.filter_definition,
                       ' ', col.name, ' ', x.key_ordinal, ' ', x.is_descending_key,
                       ' ', x.is_included_column, ' ', x.index_column_id) COLLATE DATABASE_DEFAULT
         FROM sys.indexes i
         JOIN sys.index_columns x ON x.object_id = i.object_id AND x.index_id = i.index_id
         JOIN sys.columns col ON col.object_id = x.object_id AND col.column_id = x.column_id
         LEFT JOIN sys.key_constraints k ON k.parent_object_id = i.object_id AND k.unique_index_id = i.index_id
         WHERE i.object_id = OBJECT_ID('{q}')
         UNION ALL
         SELECT CONCAT('ck ', IIF(is_system_named = 1, '*', name), ' ', definition) COLLATE DATABASE_DEFAULT
         FROM sys.check_constraints WHERE parent_object_id = OBJECT_ID('{q}')
         UNION ALL
         SELECT CONCAT('fk ', IIF(fk.is_system_named = 1, '*', fk.name), ' ',
                       OBJECT_NAME(fk.referenced_object_id), ' ', fk.delete_referential_action_desc,
                       ' ', fk.update_referential_action_desc,
                       ' ', COL_NAME(x.parent_object_id, x.parent_column_id),
                       ' ', COL_NAME(x.referenced_object_id, x.referenced_column_id)) COLLATE DATABASE_DEFAULT
         FROM sys.foreign_keys fk
         JOIN sys.foreign_key_columns x ON x.constraint_object_id = fk.object_id
         WHERE fk.parent_object_id = OBJECT_ID('{q}')
         ORDER BY 1"
    )
}

#[tokio::test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
async fn mssql_script_create_recreates_the_table() {
    let h = harness().await;
    let c = profile(&h, &mssql(), false).await;
    let tag = &Uuid::new_v4().simple().to_string()[..8];
    let (a, b) = (format!("it171_{tag}_a"), format!("it171_{tag}_b"));
    exec(&h, c, &format!("CREATE SCHEMA {a}")).await;
    exec(&h, c, &format!("CREATE SCHEMA {b}")).await;
    exec(
        &h,
        c,
        &format!(
            "CREATE TABLE {a}.parent (
                id int IDENTITY(10, 5) CONSTRAINT pk_parent PRIMARY KEY NONCLUSTERED,
                code nvarchar(20) COLLATE Latin1_General_BIN NOT NULL CONSTRAINT df_code DEFAULT N'',
                a int CONSTRAINT df_a DEFAULT 0, b int DEFAULT 1, inc nvarchar(50),
                tot AS (a * b), tp AS (a + 1) PERSISTED NOT NULL,
                CONSTRAINT ck_a CHECK (a >= 0), CONSTRAINT uq_code UNIQUE (code, a), CHECK (b <> 7));
            CREATE CLUSTERED INDEX cx_parent ON {a}.parent (a);
            CREATE INDEX ix_code ON {a}.parent (code DESC) INCLUDE (inc) WHERE a > 0;
            CREATE UNIQUE INDEX ux_inc ON {a}.parent (inc, b DESC) WHERE inc IS NOT NULL;
            CREATE TABLE {a}.child (
                id int NOT NULL PRIMARY KEY, pid int,
                CONSTRAINT fk_child FOREIGN KEY (pid) REFERENCES {a}.parent (id)
                    ON DELETE CASCADE ON UPDATE SET NULL,
                FOREIGN KEY (pid) REFERENCES {a}.parent (id));"
        ),
    )
    .await;
    script_round_trip(&h, c, &a, &b, &["parent", "child"], ms_describe).await;
    exec(
        &h,
        c,
        &format!("INSERT INTO {b}.parent (code, a, b) VALUES (N'x', 2, 3)"),
    )
    .await;
    let r = rows(
        &exec(&h, c, &format!("SELECT id, tot, tp FROM {b}.parent")).await,
        0,
    );
    assert_eq!(r, [vec![json!(10), json!(6), json!(3)]]);
    exec(
        &h,
        c,
        &format!(
            "DROP TABLE {b}.child; DROP TABLE {b}.parent; DROP TABLE {a}.child; DROP TABLE {a}.parent;
             DROP SCHEMA {a}; DROP SCHEMA {b};"
        ),
    )
    .await;
}
