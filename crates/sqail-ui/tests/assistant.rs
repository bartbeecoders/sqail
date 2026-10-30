//! The assistant's MCP server against real databases, and (opt-in) a whole
//! turn through a real Claude Code or Grok CLI. Needs `scripts/db.sh up`:
//!
//!     SQAIL_IT=1 cargo test -p sqail-ui --test assistant -- --ignored --nocapture
//!
//! The CLI test also needs the CLI installed and signed in; it spends a few
//! cents: SQAIL_ASSISTANT_E2E=claude (or grok) with the command above.

use std::time::Duration;

use serde_json::{Value, json};
use sqail_client::proto::{
    ConnectionInput, ConnectionParams, MssqlAuth, MssqlEncrypt, MssqlParams, PgSslMode,
    PostgresParams, QueryEvent, QueryRequest, SqliteParams,
};
use sqail_client::{Client, On, Target, Trust};
use sqail_ui::assistant::cli::{self, Provider, Request};
use sqail_ui::assistant::mcp::{Backend, Env, ServiceBackend, serve};
use sqail_ui::assistant::stream::Event;
use uuid::Uuid;

struct Fixture {
    _dir: tempfile::TempDir,
    rt: tokio::runtime::Runtime,
    target: Target,
    pg: Uuid,
    ms: Uuid,
    lite: Uuid,
    _server: sqail_service::Server,
}

fn fixture() -> Fixture {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = sqail_service::Config {
        data_dir: dir.path().join("service"),
        bind: "127.0.0.1:0".parse().unwrap(),
        docs_ui: false,
        ..Default::default()
    };
    cfg.sqlite.allowed_dirs = vec![dir.path().to_path_buf()];
    std::fs::create_dir_all(&cfg.data_dir).unwrap();
    let server = rt.block_on(sqail_service::start(cfg)).unwrap();
    let target = Target {
        url: format!("https://{}", server.addr),
        token: server.bootstrap_token.clone().unwrap(),
        trust: Trust::Pinned(server.fingerprint.clone()),
        identity: None,
    };
    let client = Client::new(&target).unwrap();
    let create = |name: &str, params: ConnectionParams, pw: &str| {
        rt.block_on(client.create_connection(&ConnectionInput {
            name: name.into(),
            params,
            password: Some(pw.into()),
            ssl_client_key: None,
            read_only: false,
            color: None,
            environment: None,
            folder: None,
        }))
        .unwrap()
        .id
    };
    let pg = create(
        "pg",
        ConnectionParams::Postgres(PostgresParams {
            host: "127.0.0.1".into(),
            port: 55432,
            database: "sqail_test".into(),
            user: "sqail".into(),
            ssl_mode: PgSslMode::Disable,
            ssl_root_cert: None,
            ssl_client_cert: None,
        }),
        "sqail_dev_pw",
    );
    let ms = create(
        "ms",
        ConnectionParams::Mssql(MssqlParams {
            host: "127.0.0.1".into(),
            port: 51433,
            instance: None,
            database: Some("sqail_test".into()),
            auth: MssqlAuth::Sql {
                user: "sqail".into(),
            },
            encrypt: MssqlEncrypt::Required,
            trust_server_certificate: true,
        }),
        "Sqail2_dev!Passw0rd",
    );
    let lite = create(
        "lite",
        ConnectionParams::Sqlite(SqliteParams {
            path: dir.path().join("lite.db").to_string_lossy().into_owned(),
            create: true,
        }),
        "",
    );
    Fixture {
        _dir: dir,
        rt,
        target,
        pg,
        ms,
        lite,
        _server: server,
    }
}

fn call(id: u32, name: &str, args: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": args}})
        .to_string()
        + "\n"
}

/// Replies to `calls`, as (text, is_error).
async fn mcp(backend: &ServiceBackend, calls: &[String]) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    serve(backend, 20, calls.concat().as_bytes(), &mut out)
        .await
        .unwrap();
    String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| {
            let v: Value = serde_json::from_str(l).unwrap();
            let r = &v["result"];
            (
                r["content"][0]["text"].as_str().unwrap().to_string(),
                r["isError"].as_bool().unwrap(),
            )
        })
        .collect()
}

#[test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
fn mcp_tools_on_sql_server() {
    let f = fixture();
    f.rt.block_on(async {
        let backend = ServiceBackend::connect(&f.target, f.ms).await.unwrap();
        let r = mcp(
            &backend,
            &[
                call(1, "list_tables", json!({"schema": "sales"})),
                call(2, "describe_table", json!({"table": "sales.orders"})),
                call(3, "run_query", json!({"sql": "SELECT status, COUNT(*) AS n FROM sales.big_orders GROUP BY status ORDER BY status"})),
                call(4, "run_query", json!({"sql": "SELECT id FROM sales.big_orders ORDER BY id"})),
                call(5, "run_query", json!({"sql": "DELETE FROM sales.big_orders"})),
                call(6, "run_query", json!({"sql": "SELECT * FROM no_such_table"})),
            ],
        )
        .await;
        assert!(!r[0].1 && r[0].0.contains("sales.big_orders (table)"), "{:?}", r[0]);
        assert!(!r[1].1, "{:?}", r[1]);
        assert!(r[1].0.contains("customer_id int NOT NULL"), "{}", r[1].0);
        assert!(r[1].0.contains("-> sales.customers (id)"), "{}", r[1].0);
        assert_eq!(
            r[2],
            (
                "status | n\ncancelled | 200000\ndelivered | 200000\nnew | 200000\npaid | 200000\nshipped | 200000\n(5 rows)".into(),
                false
            )
        );
        // Capped at 20 rows (the server's limit in this test).
        assert!(r[3].0.contains("(20 rows; stopped at 20, more rows exist"), "{}", r[3].0);
        assert!(r[4].1 && r[4].0.starts_with("refused:"), "{:?}", r[4]);
        assert!(r[5].1, "{:?}", r[5]);

        // Second barrier: even SQL that slips past the check is rolled back.
        let before = count(&backend, "SELECT COUNT(*) FROM sales.customers").await;
        backend
            .query("INSERT INTO sales.customers (name, email, country) VALUES ('x', 'assistant@test', 'BE')", 5)
            .await
            .unwrap();
        assert_eq!(count(&backend, "SELECT COUNT(*) FROM sales.customers").await, before);
        backend.close().await;
    });
}

#[test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
fn postgres_assistant_transactions_are_read_only() {
    let f = fixture();
    f.rt.block_on(async {
        let backend = ServiceBackend::connect(&f.target, f.pg).await.unwrap();
        let r = mcp(
            &backend,
            &[call(
                1,
                "run_query",
                json!({"sql": "SELECT count(*) FROM sales.orders"}),
            )],
        )
        .await;
        assert!(!r[0].1, "{:?}", r[0]);
        let err = backend
            .query(
                "INSERT INTO sales.customers (name, email) VALUES ('x', 'assistant@test')",
                5,
            )
            .await
            .unwrap_err();
        assert!(
            format!("{err:#}").contains("read-only transaction"),
            "{err:#}"
        );
        // The session is usable again afterwards.
        let r = mcp(
            &backend,
            &[call(2, "run_query", json!({"sql": "SELECT 1 AS one"}))],
        )
        .await;
        assert_eq!(r[0], ("one\n1\n(1 row)".into(), false));
        backend.close().await;
    });
}

/// Runs `sql` outside the assistant, failing on any error.
async fn admin(f: &Fixture, conn: Uuid, sql: &str) -> Vec<QueryEvent> {
    let client = Client::new(&f.target).unwrap();
    let events = client
        .query_all(On::Connection(conn), &QueryRequest::new(sql))
        .await
        .unwrap();
    if let Some(QueryEvent::Error { message, .. }) = events
        .iter()
        .find(|e| matches!(e, QueryEvent::Error { .. }))
    {
        panic!("{sql}: {message}");
    }
    events
}

/// Queries that try to end the assistant's transaction and write after it,
/// sent straight to [`ServiceBackend::query`] (past the guard): the service's
/// single-statement check and the rollback must leave the canary unchanged.
fn adversarial(f: &Fixture, conn: Uuid, attempts: &[&str]) {
    let table = format!("sqail_canary_{}", Uuid::new_v4().simple());
    f.rt.block_on(async {
        admin(f, conn, &format!("CREATE TABLE {table} (n int)")).await;
        admin(f, conn, &format!("INSERT INTO {table} VALUES (1)")).await;
        let backend = ServiceBackend::connect(&f.target, conn).await.unwrap();
        for attempt in attempts {
            let sql = attempt.replace("{t}", &table);
            let engine = backend.engine();
            assert!(
                sqail_ui::assistant::guard::check_read_only(&sql, engine).is_err(),
                "guard allowed: {sql}"
            );
            let _ = backend.query(&sql, 5).await;
            let n = count(&backend, &format!("SELECT COUNT(*) FROM {table}")).await;
            assert_eq!(n, 1, "{engine:?} wrote through: {sql}");
        }
        backend.close().await;
        admin(f, conn, &format!("DROP TABLE {table}")).await;
    });
}

#[test]
fn adversarial_queries_on_sqlite_change_nothing() {
    let f = fixture();
    adversarial(
        &f,
        f.lite,
        &[
            "SELECT 1; COMMIT; INSERT INTO {t} VALUES (2)",
            "SELECT 1 /* /* */ ; COMMIT; INSERT INTO {t} VALUES (2); /* */",
            r"SELECT 'a\'; COMMIT; INSERT INTO {t} VALUES (2); SELECT '",
            "SELECT 1; END; DELETE FROM {t}",
        ],
    );
}

#[test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
fn adversarial_queries_on_postgres_change_nothing() {
    let f = fixture();
    adversarial(
        &f,
        f.pg,
        &[
            "SELECT 1; COMMIT; INSERT INTO {t} VALUES (2)",
            r"SELECT E'\''; COMMIT; INSERT INTO {t} VALUES (2); SELECT ''",
            "SELECT 1 AS a$b$; COMMIT; INSERT INTO {t} VALUES (2); SELECT $b$",
            "SELECT 1; END; DELETE FROM {t}",
        ],
    );
}

#[test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
fn adversarial_queries_on_sql_server_change_nothing() {
    let f = fixture();
    adversarial(
        &f,
        f.ms,
        &[
            "SELECT 1 COMMIT INSERT INTO {t} VALUES (2)",
            "SELECT 1; COMMIT; INSERT INTO {t} VALUES (2)",
            "SELECT 1\nGO\nCOMMIT\nGO\nINSERT INTO {t} VALUES (2)",
            "SELECT 1 ROLLBACK DELETE FROM {t}",
            "SELECT 1 IF @@TRANCOUNT > 0 COMMIT DELETE FROM {t}",
        ],
    );
}

async fn count(b: &ServiceBackend, sql: &str) -> i64 {
    let rows = b.query(sql, 1).await.unwrap();
    rows.rows[0][0].as_i64().unwrap()
}

/// One real turn: the CLI starts `sqail mcp` (this package's binary), which
/// talks to the in-process service. Opt-in, it calls a paid model.
#[test]
#[ignore = "needs the databases and a signed-in CLI (SQAIL_ASSISTANT_E2E=claude|grok)"]
fn cli_answers_with_sqail_tools() {
    let provider = match std::env::var("SQAIL_ASSISTANT_E2E").as_deref() {
        Ok("claude") => Provider::ClaudeCode,
        Ok("grok") => Provider::Grok,
        _ => {
            eprintln!("skipped: set SQAIL_ASSISTANT_E2E=claude or grok");
            return;
        }
    };
    let f = fixture();
    let work = tempfile::tempdir().unwrap();
    let Trust::Pinned(fp) = &f.target.trust else {
        unreachable!()
    };
    let req = Request {
        provider,
        program: None,
        model: match provider {
            Provider::ClaudeCode => Some("haiku".into()),
            Provider::Grok => None,
        },
        instructions: "You are a SQL assistant for a SQL Server database. Use the sqail tools (list_tables, describe_table, run_query) to answer.".into(),
        prompt: "How many rows does sales.big_orders have? Use run_query, then answer with just the number.".into(),
        resume: None,
        workdir: work.path().to_path_buf(),
        mcp_command: vec![env!("CARGO_BIN_EXE_sqail").into(), "mcp".into()],
        env: vec![
            (Env::URL.into(), f.target.url.clone()),
            (Env::TOKEN.into(), f.target.token.clone()),
            (Env::FINGERPRINT.into(), fp.clone()),
            (Env::CONNECTION.into(), f.ms.to_string()),
        ],
    };
    let (_stop, rx) = tokio::sync::oneshot::channel();
    let mut events = Vec::new();
    let outcome = f.rt.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(240),
            cli::run(req, |e| events.push(e), rx),
        )
        .await
    });
    let outcome = outcome.expect("the CLI took more than 4 minutes").unwrap();
    assert!(matches!(outcome, cli::Outcome::Finished));
    for e in &events {
        eprintln!("{e:?}");
    }
    let queried = events.iter().any(|e| {
        matches!(e, Event::ToolResult { text, is_error: false, .. } if text.contains("1000000"))
    });
    assert!(queried, "no successful run_query returning 1000000");
    let answer: String = events
        .iter()
        .filter_map(|e| match e {
            Event::TextDelta(t) | Event::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        answer.replace([',', '.', ' '], "").contains("1000000"),
        "answer: {answer}"
    );
}
