//! End-to-end tests: start the real HTTPS server in-process and talk to it.
//!
//! Tests without `#[ignore]` need nothing but a temp dir. The `#[ignore]`d
//! ones need the podman test databases (`scripts/db.sh up`) and run with
//! `scripts/check.sh --it` (`cargo test -- --ignored`).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sqail_proto::{
    ColumnInfo, Connection, CreatedToken, Ddl, ForeignKeyInfo, QueryEvent, RoutineInfo,
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
    let mut cfg = Config {
        data_dir: dir.path().to_path_buf(),
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
