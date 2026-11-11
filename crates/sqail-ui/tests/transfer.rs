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
    svc_with(|_| {})
}

fn svc_with(tweak: impl FnOnce(&mut sqail_service::Config)) -> Svc {
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
    tweak(&mut cfg);
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
            On::Connection(conn),
            engine,
            source_sql,
            None,
            &csv,
            Format::Csv,
            &transfer::ExportOptions::default(),
            |_| {},
        ))
        .unwrap()
        .rows;
    assert!(exported > 0);

    let columns =
        s.rt.block_on(s.client.columns(conn, schema, target))
            .unwrap();
    let preview = transfer::preview_csv(&csv, true, None, None).unwrap();
    let spec = ImportSpec {
        connection: conn,
        engine,
        schema: schema.map(String::from),
        table: target.into(),
        mapping: transfer::auto_map(&preview.headers, &columns),
        columns,
        has_header: true,
        delimiter: preview.delimiter,
        encoding: preview.encoding,
        empty_is_null: true,
    };
    assert!(
        spec.mapping.iter().all(Option::is_some),
        "{:?}",
        spec.mapping
    );
    let imported =
        s.rt.block_on(transfer::import_csv(
            &s.client,
            &csv,
            &spec,
            &Default::default(),
            |_| {},
        ))
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
    let preview = transfer::preview_csv(&csv, true, None, None).unwrap();
    let spec = ImportSpec {
        connection: conn,
        engine: Engine::Sqlite,
        schema: Some("main".into()),
        table: "dst".into(),
        mapping: transfer::auto_map(&preview.headers, &columns),
        columns,
        has_header: true,
        delimiter: b',',
        encoding: transfer::Encoding::Utf8,
        empty_is_null: true,
    };
    let err =
        s.rt.block_on(transfer::import_csv(
            &s.client,
            &csv,
            &spec,
            &Default::default(),
            |_| {},
        ))
        .unwrap_err();
    assert!(format!("{err:#}").contains("UNIQUE"), "{err:#}");
    assert_eq!(
        s.rows(conn, "SELECT count(*) FROM dst"),
        vec![vec![serde_json::json!(3)]]
    );

    // Binary values export as 0x… and import as the same bytes.
    s.rows(
        conn,
        "CREATE TABLE bsrc (id INTEGER PRIMARY KEY, b BLOB);
         INSERT INTO bsrc VALUES (1, X'DEADBEEF'), (2, X''), (3, NULL), (4, X'00FF');
         CREATE TABLE bdst (id INTEGER PRIMARY KEY, b BLOB);",
    );
    round_trip(
        &s,
        conn,
        Engine::Sqlite,
        Some("main"),
        "bsrc",
        "SELECT * FROM bsrc ORDER BY id",
        "bdst",
        "SELECT id, typeof(b), length(b), hex(b) FROM {t} ORDER BY id",
    );
    assert_eq!(
        s.rows(conn, "SELECT typeof(b), length(b) FROM bdst WHERE id = 1"),
        vec![vec![serde_json::json!("blob"), serde_json::json!(4)]]
    );

    // Text that is not 0x hex is refused for a binary column.
    let csv = s.dir.path().join("badhex.csv");
    std::fs::write(&csv, "id,b\n10,0xdeadbeef\n11,hello\n").unwrap();
    let columns =
        s.rt.block_on(s.client.columns(conn, Some("main"), "bdst"))
            .unwrap();
    let spec = ImportSpec {
        connection: conn,
        engine: Engine::Sqlite,
        schema: Some("main".into()),
        table: "bdst".into(),
        mapping: vec![Some(0), Some(1)],
        columns,
        has_header: true,
        delimiter: b',',
        encoding: transfer::Encoding::Utf8,
        empty_is_null: true,
    };
    let err =
        s.rt.block_on(transfer::import_csv(
            &s.client,
            &csv,
            &spec,
            &Default::default(),
            |_| {},
        ))
        .unwrap_err();
    let err = format!("{err:#}");
    assert!(
        err.contains("CSV line 3, column b: 'hello' is not a binary value"),
        "{err}"
    );
    assert_eq!(
        s.rows(conn, "SELECT count(*) FROM bdst"),
        vec![vec![serde_json::json!(4)]]
    );
}

/// Import options and failures: rows with too few fields are refused with
/// their physical line, other encodings and delimiters import, and Cancel
/// rolls back.
#[test]
fn csv_import_checks_rows_encodings_and_cancel() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let s = svc();
    let conn = s.connect(
        "lite",
        ConnectionParams::Sqlite(SqliteParams {
            path: s.dir.path().join("imp.db").to_string_lossy().into(),
            create: true,
        }),
        None,
    );
    s.rows(
        conn,
        "CREATE TABLE p (id INTEGER PRIMARY KEY, name TEXT, note TEXT)",
    );
    let columns =
        s.rt.block_on(s.client.columns(conn, Some("main"), "p"))
            .unwrap();
    let import = |name: &str, bytes: &[u8], cancel: &AtomicBool, after: &dyn Fn(u64)| {
        let csv = s.dir.path().join(name);
        std::fs::write(&csv, bytes).unwrap();
        let preview = transfer::preview_csv(&csv, true, None, None).unwrap();
        let spec = ImportSpec {
            connection: conn,
            engine: Engine::Sqlite,
            schema: Some("main".into()),
            table: "p".into(),
            mapping: transfer::auto_map(&preview.headers, &columns),
            columns: columns.clone(),
            has_header: true,
            delimiter: preview.delimiter,
            encoding: preview.encoding,
            empty_is_null: true,
        };
        s.rt.block_on(transfer::import_csv(&s.client, &csv, &spec, cancel, after))
            .map_err(|e| format!("{e:#}"))
    };
    let go = AtomicBool::new(false);
    let count = || s.rows(conn, "SELECT count(*) FROM p")[0][0].clone();

    // Line 4 is short; line 2's quoted note spans two lines, so it is the
    // fourth line of the file, not the third record.
    let err = import(
        "short.csv",
        b"id,name,note\n1,a,\"two\nlines\"\n2,b\n3,c,x\n",
        &go,
        &|_| {},
    )
    .unwrap_err();
    assert!(
        err.contains("CSV line 4 has 2 fields, the header has 3"),
        "{err}"
    );
    assert_eq!(count(), serde_json::json!(0));

    // Windows-1252 with semicolons (and a comma in a value).
    let n = import(
        "ansi.csv",
        b"id;name;note\n1;Ren\xE9e;\x80 5, more\n",
        &go,
        &|_| {},
    )
    .unwrap();
    assert_eq!(n, 1);
    // UTF-16 with a BOM, tab-separated.
    let mut utf16 = vec![0xFF, 0xFE];
    utf16.extend(
        "id\tname\tnote\n2\t日本\t😀\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes),
    );
    assert_eq!(import("u16.csv", &utf16, &go, &|_| {}).unwrap(), 1);
    assert_eq!(
        s.rows(conn, "SELECT id, name, note FROM p ORDER BY id"),
        vec![
            vec![
                serde_json::json!(1),
                serde_json::json!("Renée"),
                serde_json::json!("€ 5, more")
            ],
            vec![
                serde_json::json!(2),
                serde_json::json!("日本"),
                serde_json::json!("😀")
            ],
        ]
    );

    // Cancelled after the first batch: nothing stays.
    let mut big = String::from("id,name,note\n");
    for i in 100..3100 {
        big.push_str(&format!("{i},n{i},\n"));
    }
    let cancel = AtomicBool::new(false);
    let err = import("big.csv", big.as_bytes(), &cancel, &|n| {
        assert!(n < 3000, "the import ran to the end");
        cancel.store(true, Ordering::Relaxed);
    })
    .unwrap_err();
    assert!(err.contains("cancelled"), "{err}");
    assert_eq!(count(), serde_json::json!(2));
}

/// "Export whole result" re-runs only the statement behind the result, on
/// the tab's session: the script's write is not repeated, and a temp table
/// that only exists in the session is still there.
#[test]
fn whole_export_reruns_one_statement_on_the_session() {
    let s = svc();
    let conn = s.connect(
        "lite",
        ConnectionParams::Sqlite(SqliteParams {
            path: s.dir.path().join("w.db").to_string_lossy().into(),
            create: true,
        }),
        None,
    );
    s.rows(conn, "CREATE TABLE t (a INTEGER)");
    let session = s.rt.block_on(s.client.open_session(conn)).unwrap().id;
    let script =
        "CREATE TEMP TABLE tmp AS SELECT 7 AS a;\nINSERT INTO t VALUES (1);\nSELECT a FROM tmp";
    let events =
        s.rt.block_on(
            s.client
                .query_all(On::Session(session), &QueryRequest::new(script)),
        )
        .unwrap();
    let results = events
        .iter()
        .filter(|e| matches!(e, QueryEvent::ResultStart { .. }))
        .count();
    let sql = transfer::statement_for_result(script, Engine::Sqlite, 0, results, true).unwrap();
    assert_eq!(sql, "SELECT a FROM tmp");

    let path = s.dir.path().join("whole.csv");
    let export = |cols: &[String]| {
        s.rt.block_on(transfer::export_query(
            &s.client,
            On::Session(session),
            Engine::Sqlite,
            &sql,
            Some(cols),
            &path,
            Format::Csv,
            &transfer::ExportOptions::default(),
            |_| {},
        ))
    };
    let done = export(&["a".into()]).unwrap();
    assert_eq!((done.rows, done.truncated), (1, false));
    assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), "a\n7");
    // The write ran once, with the script.
    assert_eq!(
        s.rows(conn, "SELECT count(*) FROM t"),
        [[serde_json::json!(1)]]
    );
    // A different shape than the result shown is refused.
    assert!(export(&["b".into()]).is_err());
}

/// A whole-query export that stops at the service's `limits.max_rows` says
/// so instead of passing for the complete result.
#[test]
fn whole_export_reports_the_service_row_limit() {
    let s = svc_with(|cfg| {
        cfg.limits.default_max_rows = 3;
        cfg.limits.max_rows = 5;
    });
    let conn = s.connect(
        "lite",
        ConnectionParams::Sqlite(SqliteParams {
            path: s.dir.path().join("cap.db").to_string_lossy().into(),
            create: true,
        }),
        None,
    );
    let path = s.dir.path().join("cap.csv");
    let export = |sql: &str| {
        s.rt.block_on(transfer::export_query(
            &s.client,
            On::Connection(conn),
            Engine::Sqlite,
            sql,
            None,
            &path,
            Format::Csv,
            &transfer::ExportOptions::default(),
            |_| {},
        ))
        .unwrap()
    };
    let many = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 8) \
                SELECT i FROM n";
    let done = export(many);
    assert_eq!((done.rows, done.truncated), (5, true));
    assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 6);
    let (text, warning) = transfer::export_notice(done.rows, &path, done.truncated, true);
    assert!(warning && text.contains("rows are missing"), "{text}");

    let done = export("SELECT 1 AS i UNION ALL SELECT 2");
    assert_eq!((done.rows, done.truncated), (2, false));
}

/// An export that fails midway (here a runtime error after some rows)
/// leaves no partial file, and an existing file untouched.
#[test]
fn failed_export_leaves_no_partial_file() {
    let s = svc();
    let conn = s.connect(
        "lite",
        ConnectionParams::Sqlite(SqliteParams {
            path: s.dir.path().join("fail.db").to_string_lossy().into(),
            create: true,
        }),
        None,
    );
    let out = s.dir.path().join("out");
    std::fs::create_dir(&out).unwrap();
    let sql = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 5000) \
               SELECT CASE WHEN i < 4000 THEN i ELSE abs(-9223372036854775807 - 1) END AS i FROM n";
    for format in Format::ALL {
        let path = out.join(format!("fail.{}", format.extension()));
        let mut written = 0;
        let err =
            s.rt.block_on(transfer::export_query(
                &s.client,
                On::Connection(conn),
                Engine::Sqlite,
                sql,
                None,
                &path,
                format,
                &transfer::ExportOptions::default(),
                |n| written = n,
            ))
            .unwrap_err();
        assert!(format!("{err:#}").contains("overflow"), "{err:#}");
        // Rows were written before the error.
        assert!(written > 0, "{format:?}");
    }
    let left: Vec<_> = std::fs::read_dir(&out).unwrap().collect();
    assert!(left.is_empty(), "{left:?}");

    let path = out.join("keep.csv");
    std::fs::write(&path, "old").unwrap();
    assert!(
        s.rt.block_on(transfer::export_query(
            &s.client,
            On::Connection(conn),
            Engine::Sqlite,
            sql,
            None,
            &path,
            Format::Csv,
            &transfer::ExportOptions::default(),
            |_| {},
        ))
        .is_err()
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "old");
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

    // bytea exports as 0x… and imports as the same bytes, not their ASCII.
    let (bs, bd) = (format!("it_bsrc_{suffix}"), format!("it_bdst_{suffix}"));
    s.rows(
        pg,
        &format!(
            "CREATE TABLE sales.{bs} (id int PRIMARY KEY, b bytea);
             INSERT INTO sales.{bs} VALUES (1, '\\xdeadbeef'), (2, ''), (3, NULL);
             CREATE TABLE sales.{bd} (id int PRIMARY KEY, b bytea); SELECT 1"
        ),
    );
    round_trip(
        &s,
        pg,
        Engine::Postgres,
        Some("sales"),
        &bs,
        &format!("SELECT * FROM sales.{bs} ORDER BY id"),
        &bd,
        "SELECT id, octet_length(b), encode(b, 'hex') FROM sales.{t} ORDER BY id",
    );
    s.rows(pg, &format!("DROP TABLE sales.{bs}, sales.{bd}; SELECT 1"));

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

    s.rows(
        ms,
        &format!(
            "CREATE TABLE sales.{bs} (id int PRIMARY KEY, b varbinary(max), f binary(3));
             INSERT INTO sales.{bs} VALUES (1, 0xdeadbeef, 0x0102), (2, 0x, NULL), (3, NULL, 0xffffff);
             CREATE TABLE sales.{bd} (id int PRIMARY KEY, b varbinary(max), f binary(3)); SELECT 1"
        ),
    );
    round_trip(
        &s,
        ms,
        Engine::Mssql,
        Some("sales"),
        &bs,
        &format!("SELECT * FROM sales.{bs} ORDER BY id"),
        &bd,
        "SELECT id, DATALENGTH(b), b, f FROM sales.{t} ORDER BY id",
    );
    s.rows(ms, &format!("DROP TABLE sales.{bs}, sales.{bd}; SELECT 1"));
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

/// SQL INSERT and JSON exports of Postgres numeric NaN, bigints and jsonb
/// give valid SQL and native JSON.
#[test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
fn postgres_exports_keep_values_exact() {
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
    let sql = "SELECT 'NaN'::numeric AS n, 1234567890123456789::bigint AS id, \
               '{\"k\": 1}'::jsonb AS j";
    let export = |format: Format, table: Option<&str>| {
        let path = s.dir.path().join(format!("pg.{}", format.extension()));
        s.rt.block_on(transfer::export_query(
            &s.client,
            On::Connection(pg),
            Engine::Postgres,
            sql,
            None,
            &path,
            format,
            &transfer::ExportOptions {
                table: table.map(String::from),
                ..Default::default()
            },
            |_| {},
        ))
        .unwrap();
        std::fs::read_to_string(&path).unwrap()
    };
    let json: serde_json::Value = serde_json::from_str(&export(Format::Json, None)).unwrap();
    assert_eq!(
        json,
        serde_json::json!([{"n": "NaN", "id": 1234567890123456789i64, "j": {"k": 1}}])
    );
    let t = format!("it_exact_{}", &Uuid::new_v4().simple().to_string()[..8]);
    let insert = export(Format::Sql, Some(&t));
    assert!(insert.contains("'NaN'"), "{insert}");
    let session = s.rt.block_on(s.client.open_session(pg)).unwrap().id;
    let run = |sql: &str| {
        let events =
            s.rt.block_on(
                s.client
                    .query_all(On::Session(session), &QueryRequest::new(sql)),
            )
            .unwrap();
        for e in &events {
            if let QueryEvent::Error { message, .. } = e {
                panic!("{sql}: {message}");
            }
        }
        events
    };
    run(&format!(
        "CREATE TEMP TABLE {t} (n numeric, id bigint, j jsonb)"
    ));
    run(&insert);
    let rows: Vec<_> = run(&format!("SELECT n::text, id, j->>'k' FROM {t}"))
        .into_iter()
        .filter_map(|e| match e {
            QueryEvent::Rows { rows, .. } => Some(rows),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(
        rows,
        vec![vec![
            serde_json::json!("NaN"),
            serde_json::json!(1234567890123456789i64),
            serde_json::json!("1")
        ]]
    );
    let _ = s.rt.block_on(s.client.close_session(session));
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
            On::Connection(pg),
            Engine::Postgres,
            "SELECT oi.*, g FROM sales.order_items oi CROSS JOIN generate_series(1, 34) g",
            None,
            &path,
            Format::Csv,
            &transfer::ExportOptions::default(),
            |_| peak = peak.max(rss_kb()),
        ))
        .unwrap()
        .rows;
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
