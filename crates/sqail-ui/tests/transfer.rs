//! Export/import against a real in-process service: CSV round trips on every
//! engine, and a streamed 1M-row export with flat memory.

use std::path::PathBuf;

use sqail_client::proto::{
    ConnectionInput, ConnectionParams, Engine, MssqlAuth, MssqlEncrypt, MssqlParams, PgSslMode,
    PostgresParams, QueryEvent, QueryRequest, SqliteParams,
};
use sqail_client::{Client, On, Target, Trust};
use sqail_ui::transfer::{self, Format, ImportSpec};
use uuid::Uuid;

struct Svc {
    rt: tokio::runtime::Runtime,
    dir: tempfile::TempDir,
    _server: sqail_service::Server,
    client: Client,
}

fn svc() -> Svc {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut cfg = sqail_service::Config {
        data_dir: dir.path().join("service"),
        bind: "127.0.0.1:0".parse().unwrap(),
        docs_ui: false,
        ..Default::default()
    };
    cfg.sqlite.allowed_dirs = vec![dir.path().to_path_buf(), root.join("dev/data")];
    std::fs::create_dir_all(&cfg.data_dir).unwrap();
    let server = rt.block_on(sqail_service::start(cfg)).unwrap();
    let client = Client::new(&Target {
        url: format!("https://{}", server.addr),
        token: server.bootstrap_token.clone().unwrap(),
        trust: Trust::Pinned(server.fingerprint.clone()),
        identity: None,
    })
    .unwrap();
    Svc {
        rt,
        dir,
        _server: server,
        client,
    }
}

impl Svc {
    fn connect(&self, name: &str, params: ConnectionParams, password: Option<&str>) -> Uuid {
        self.rt
            .block_on(self.client.create_connection(&ConnectionInput {
                name: name.into(),
                params,
                password: password.map(String::from),
                ssl_client_key: None,
                read_only: false,
                color: None,
                environment: None,
                folder: None,
            }))
            .unwrap()
            .id
    }

    fn rows(&self, conn: Uuid, sql: &str) -> Vec<Vec<serde_json::Value>> {
        let events = self
            .rt
            .block_on(
                self.client
                    .query_all(On::Connection(conn), &QueryRequest::new(sql)),
            )
            .unwrap();
        let mut out = Vec::new();
        for e in events {
            match e {
                QueryEvent::Rows { rows, .. } => out.extend(rows),
                QueryEvent::Error { message, .. } => panic!("{sql}: {message}"),
                _ => {}
            }
        }
        out
    }
}

/// Export `source` to CSV, import it into `target` (already created), and
/// check both tables hold the same rows.
/// `compare` is a query with `{t}` standing for the table name; it runs
/// against `source` and `target` and must return the same rows.
#[allow(clippy::too_many_arguments)]
fn round_trip(
    s: &Svc,
    conn: Uuid,
    engine: Engine,
    schema: Option<&str>,
    source: &str,
    source_sql: &str,
    target: &str,
    compare: &str,
) {
    let csv = s.dir.path().join(format!("{target}.csv"));
    let exported =
        s.rt.block_on(transfer::export_query(
            &s.client,
            conn,
            engine,
            source_sql,
            &csv,
            Format::Csv,
            |_| {},
        ))
        .unwrap();
    assert!(exported > 0);

    let columns =
        s.rt.block_on(s.client.columns(conn, schema, target))
            .unwrap();
    let preview = transfer::preview_csv(&csv, true).unwrap();
    let spec = ImportSpec {
        connection: conn,
        engine,
        schema: schema.map(String::from),
        table: target.into(),
        mapping: transfer::auto_map(&preview.headers, &columns),
        columns,
        has_header: true,
        delimiter: preview.delimiter,
        empty_is_null: true,
    };
    assert!(
        spec.mapping.iter().all(Option::is_some),
        "{:?}",
        spec.mapping
    );
    let imported =
        s.rt.block_on(transfer::import_csv(&s.client, &csv, &spec, |_| {}))
            .unwrap();
    assert_eq!(imported, exported);
    let source_rows = s.rows(conn, &compare.replace("{t}", source));
    let target_rows = s.rows(conn, &compare.replace("{t}", target));
    assert_eq!(source_rows, target_rows, "{engine:?} round trip differs");
}

#[test]
fn sqlite_csv_round_trip_and_failed_import_rolls_back() {
    let s = svc();
    let conn = s.connect(
        "lite",
        ConnectionParams::Sqlite(SqliteParams {
            path: s.dir.path().join("t.db").to_string_lossy().into(),
            create: true,
        }),
        None,
    );
    s.rows(
        conn,
        "CREATE TABLE src (id INTEGER PRIMARY KEY, name TEXT, score REAL, note TEXT);
         INSERT INTO src VALUES (1, 'Ada, \"the first\"', 9.5, NULL), (2, 'Linus', 8.25, 'multi
line'), (3, 'Grace', NULL, '');
         CREATE TABLE dst (id INTEGER PRIMARY KEY, name TEXT, score REAL, note TEXT);",
    );
    round_trip(
        &s,
        conn,
        Engine::Sqlite,
        Some("main"),
        "src",
        "SELECT * FROM src ORDER BY id",
        "dst",
        "SELECT id, name, score, coalesce(note, '') FROM {t} ORDER BY id",
    );
    // Note: '' and NULL both become an empty CSV field; the import maps empty
    // to NULL, hence the coalesce in the comparison.
    assert_eq!(
        s.rows(conn, "SELECT count(*) FROM dst"),
        vec![vec![serde_json::json!(3)]]
    );

    // A duplicate key fails the import and leaves the table untouched.
    let csv = s.dir.path().join("dup.csv");
    std::fs::write(&csv, "id,name\n10,x\n10,y\n").unwrap();
    let columns =
        s.rt.block_on(s.client.columns(conn, Some("main"), "dst"))
            .unwrap();
    let preview = transfer::preview_csv(&csv, true).unwrap();
    let spec = ImportSpec {
        connection: conn,
        engine: Engine::Sqlite,
        schema: Some("main".into()),
        table: "dst".into(),
        mapping: transfer::auto_map(&preview.headers, &columns),
        columns,
        has_header: true,
        delimiter: b',',
        empty_is_null: true,
    };
    let err =
        s.rt.block_on(transfer::import_csv(&s.client, &csv, &spec, |_| {}))
            .unwrap_err();
    assert!(format!("{err:#}").contains("UNIQUE"), "{err:#}");
    assert_eq!(
        s.rows(conn, "SELECT count(*) FROM dst"),
        vec![vec![serde_json::json!(3)]]
    );
}

#[test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
fn csv_round_trip_on_every_engine() {
    let s = svc();
    let suffix = &Uuid::new_v4().simple().to_string()[..8];

    let pg = s.connect(
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
        Some("sqail_dev_pw"),
    );
    let t = format!("it_import_{suffix}");
    s.rows(pg, &format!("CREATE TABLE sales.{t} (id int PRIMARY KEY, sku varchar(32), name text, price numeric(10,2), active boolean)"));
    round_trip(
        &s,
        pg,
        Engine::Postgres,
        Some("sales"),
        "products",
        "SELECT id, sku, name, price, active FROM sales.products ORDER BY id",
        &t,
        "SELECT id, sku, name, price, active FROM sales.{t} ORDER BY id",
    );
    s.rows(pg, &format!("DROP TABLE sales.{t}"));

    let ms = s.connect(
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
        Some("Sqail2_dev!Passw0rd"),
    );
    s.rows(ms, &format!("CREATE TABLE sales.{t} (id int PRIMARY KEY, sku varchar(32), name nvarchar(200), price decimal(10,2), active bit)"));
    round_trip(
        &s,
        ms,
        Engine::Mssql,
        Some("sales"),
        "products",
        "SELECT id, sku, name, price, active FROM sales.products ORDER BY id",
        &t,
        "SELECT id, sku, name, price, active FROM sales.{t} ORDER BY id",
    );
    s.rows(ms, &format!("DROP TABLE sales.{t}"));
}

fn rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .map(str::to_string)
        })
        .and_then(|l| l.split_whitespace().nth(1).and_then(|v| v.parse().ok()))
        .unwrap_or(0)
}

#[test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
fn million_row_export_streams_with_flat_memory() {
    let s = svc();
    let pg = s.connect(
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
        Some("sqail_dev_pw"),
    );
    let before = rss_kb();
    let mut peak = before;
    let path = s.dir.path().join("big.csv");
    let n =
        s.rt.block_on(transfer::export_query(
            &s.client,
            pg,
            Engine::Postgres,
            "SELECT oi.*, g FROM sales.order_items oi CROSS JOIN generate_series(1, 34) g",
            &path,
            Format::Csv,
            |_| peak = peak.max(rss_kb()),
        ))
        .unwrap();
    assert_eq!(n, 1_014_900);
    let size = std::fs::metadata(&path).unwrap().len();
    eprintln!(
        "exported {n} rows, {} MB on disk; RSS grew {} MB",
        size / 1_000_000,
        (peak - before) / 1024
    );
    // The service runs in this process too; both sides must stream.
    assert!(
        peak - before < 200 * 1024,
        "memory grew by {} MB",
        (peak - before) / 1024
    );
}
