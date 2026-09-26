//! Throughput and latency of sqail-service on the podman test databases.
//!
//!     scripts/db.sh up
//!     cargo run --release -p sqail-client --example bench
//!
//! Starts an in-process service, then per engine measures a small query's
//! latency, and time-to-first-row, rows/s and memory for 1M streamed rows.
//! Prints a Markdown table (see docs/benchmarks.md).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use futures::StreamExt;
use sqail_client::proto::*;
use sqail_client::{Client, On, Target, Trust};

fn rss_mb() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .map(str::to_string)
        })
        .and_then(|l| {
            l.split_whitespace()
                .nth(1)
                .and_then(|v| v.parse::<f64>().ok())
        })
        .map_or(0.0, |kb| kb / 1024.0)
}

struct Stream {
    first_row: Duration,
    total: Duration,
    rows: u64,
    peak_rss_growth: f64,
}

async fn stream(client: &Client, conn: uuid::Uuid, sql: &str) -> anyhow::Result<Stream> {
    let req = QueryRequest {
        sql: sql.into(),
        params: vec![],
        max_rows: Some(u64::MAX),
        timeout_ms: None,
    };
    let base = rss_mb();
    let mut peak = base;
    let start = Instant::now();
    let mut first_row = None;
    let mut rows = 0u64;
    let mut q = client.query(On::Connection(conn), &req).await?;
    while let Some(ev) = q.events.next().await {
        match ev? {
            QueryEvent::Rows { rows: r, .. } => {
                first_row.get_or_insert_with(|| start.elapsed());
                rows += r.len() as u64;
                if rows % 50_000 < 500 {
                    peak = peak.max(rss_mb());
                }
            }
            QueryEvent::Error { message, .. } => anyhow::bail!(message),
            _ => {}
        }
    }
    Ok(Stream {
        first_row: first_row.unwrap_or_default(),
        total: start.elapsed(),
        rows,
        peak_rss_growth: peak - base,
    })
}

async fn latency(
    client: &Client,
    conn: uuid::Uuid,
    sql: &str,
) -> anyhow::Result<(Duration, Duration)> {
    let mut samples = Vec::new();
    for _ in 0..60 {
        let t = Instant::now();
        client
            .query_all(On::Connection(conn), &QueryRequest::new(sql))
            .await?;
        samples.push(t.elapsed());
    }
    samples.drain(..10); // warm-up
    samples.sort();
    Ok((
        samples[samples.len() / 2],
        samples[samples.len() * 95 / 100],
    ))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut cfg = sqail_service::Config {
        data_dir: dir.path().to_path_buf(),
        bind: "127.0.0.1:0".parse()?,
        docs_ui: false,
        ..Default::default()
    };
    cfg.sqlite.allowed_dirs = vec![root.join("dev/data")];
    let server = sqail_service::start(cfg).await?;
    let client = Client::new(&Target {
        url: format!("https://{}", server.addr),
        token: server.bootstrap_token.clone().unwrap(),
        trust: Trust::Pinned(server.fingerprint.clone()),
        identity: None,
    })?;

    let sqlite_path = root.join("dev/data/sqail_test.db").canonicalize()?;
    let engines: Vec<(&str, ConnectionParams, Option<&str>, &str)> = vec![
        (
            "PostgreSQL 17",
            ConnectionParams::Postgres(PostgresParams {
                host: "127.0.0.1".into(),
                port: 55432,
                database: "sqail_test".into(),
                user: "sqail".into(),
                ssl_mode: PgSslMode::Disable,
            }),
            Some("sqail_dev_pw"),
            "SELECT g AS id, g * 2 AS n, 'row ' || g AS label, now() AS at FROM generate_series(1, 1000000) g",
        ),
        (
            "SQL Server 2022",
            ConnectionParams::Mssql(MssqlParams {
                host: "127.0.0.1".into(),
                port: 51433,
                instance: None,
                database: Some("sqail_test".into()),
                auth: MssqlAuth::Sql { user: "sqail".into() },
                encrypt: MssqlEncrypt::Required,
                trust_server_certificate: true,
            }),
            Some("Sqail2_dev!Passw0rd"),
            "SELECT TOP (1000000) CAST(ROW_NUMBER() OVER (ORDER BY (SELECT NULL)) AS int) AS id, 2 AS n,
                    CONCAT('row ', a.object_id) AS label, SYSDATETIME() AS at
             FROM sys.all_objects a CROSS JOIN sys.all_objects b CROSS JOIN sys.all_objects c",
        ),
        (
            "SQLite",
            ConnectionParams::Sqlite(SqliteParams {
                path: sqlite_path.to_string_lossy().into(),
                create: false,
            }),
            None,
            "WITH RECURSIVE r(g) AS (SELECT 1 UNION ALL SELECT g + 1 FROM r LIMIT 1000000)
             SELECT g AS id, g * 2 AS n, 'row ' || g AS label, datetime('now') AS at FROM r",
        ),
    ];

    println!(
        "| Engine | `SELECT 1` p50 / p95 | First row (1M) | 1M rows total | Rows/s | Memory growth |"
    );
    println!("|---|---|---|---|---|---|");
    for (name, params, password, big) in engines {
        let conn = client
            .create_connection(&ConnectionInput {
                name: name.into(),
                params,
                password: password.map(String::from),
                read_only: false,
                color: None,
                environment: None,
                folder: None,
            })
            .await?
            .id;
        let (p50, p95) = latency(&client, conn, "SELECT 1").await?;
        // Warm the pool and caches once, then measure.
        let _ = stream(&client, conn, big).await?;
        let s = stream(&client, conn, big).await?;
        println!(
            "| {name} | {:.1} ms / {:.1} ms | {:.1} ms | {:.2} s | {:.0} k | {:.0} MB |",
            p50.as_secs_f64() * 1e3,
            p95.as_secs_f64() * 1e3,
            s.first_row.as_secs_f64() * 1e3,
            s.total.as_secs_f64(),
            s.rows as f64 / s.total.as_secs_f64() / 1e3,
            s.peak_rss_growth
        );
        assert_eq!(s.rows, 1_000_000, "{name} returned {} rows", s.rows);
    }
    server.shutdown();
    Ok(())
}
