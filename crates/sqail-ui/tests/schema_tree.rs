//! The sidebar's schema tree on a large synthetic catalog (egui_kittest): the
//! filter reveals matches, and drawing stays cheap with thousands of tables.
//! The catalog is put straight into the tree's cache, so no database is
//! needed; the in-process service only answers (and fails) stray requests.

use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use sqail_client::proto::{
    ColumnInfo, Connection, ConnectionInput, ConnectionParams, NamedItem, RoutineInfo, RoutineKind,
    SqliteParams, TableInfo, TableKind,
};
use sqail_client::{Client, Target, Trust};
use sqail_ui::SqailApp;
use sqail_ui::app::ServiceStatus;
use sqail_ui::schema::{SchemaData, SchemaKey, SchemaMsg};
use sqail_ui::settings::{ServiceProfile, Settings};

struct Env {
    _rt: tokio::runtime::Runtime,
    _dir: tempfile::TempDir,
    _server: sqail_service::Server,
}

fn env() -> Env {
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
    let url = format!("https://{}", server.addr);
    let token = server.bootstrap_token.clone().unwrap();
    let client = Client::new(&Target {
        url: url.clone(),
        token: token.clone(),
        trust: Trust::Pinned(server.fingerprint.clone()),
        identity: None,
    })
    .unwrap();
    rt.block_on(async {
        client
            .create_connection(&ConnectionInput {
                name: "small db".into(),
                params: ConnectionParams::Sqlite(SqliteParams {
                    path: dir.path().join("small.db").to_string_lossy().into(),
                    create: true,
                }),
                password: None,
                ssl_client_key: None,
                read_only: false,
                color: None,
                environment: None,
                folder: None,
            })
            .await
            .unwrap();
    });
    sqail_ui::settings::override_config_dir(dir.path().join("ui"));
    sqail_ui::secrets::force_file_store();
    let mut settings = Settings::default();
    settings.upsert_service(ServiceProfile {
        name: "test".into(),
        url: url.clone(),
        fingerprint: Some(server.fingerprint.clone()),
        local: false,
        client_cert: None,
        client_key: None,
    });
    settings.save();
    sqail_ui::secrets::save_token(&url, &token).unwrap();
    Env {
        _rt: rt,
        _dir: dir,
        _server: server,
    }
}

fn app() -> Harness<'static, SqailApp> {
    let mut h = Harness::builder()
        .with_size([1280.0, 800.0])
        .build_eframe(|cc| SqailApp::new(cc));
    let start = Instant::now();
    while !(h.state().service.status == ServiceStatus::Connected
        && !h.state().service.connections.is_empty())
    {
        assert!(start.elapsed() < Duration::from_secs(20), "service");
        h.step();
        std::thread::sleep(Duration::from_millis(15));
    }
    h.run_steps(3);
    h
}

const BIG: &str = "00000000-0000-0000-0000-0000000000b1";

/// A PostgreSQL profile the service does not know: everything the tree shows
/// for it comes from the cache this test fills.
fn big_connection() -> Connection {
    serde_json::from_value(serde_json::json!({
        "id": BIG, "name": "big", "engine": "postgres",
        "params": {"engine": "postgres", "host": "db", "database": "big", "user": "u"},
        "has_password": true, "read_only": false, "color": null, "environment": null,
        "folder": null, "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"
    }))
    .unwrap()
}

fn put(h: &mut Harness<'_, SqailApp>, key: SchemaKey, data: SchemaData) {
    h.state_mut().schema.apply(SchemaMsg {
        connection: BIG.parse().unwrap(),
        key,
        result: Ok(data),
    });
}

/// Schema `big` with `n` tables `t_00000`…, plus `few` schemas `s1`… with
/// ten tables and one function each.
fn catalog(h: &mut Harness<'_, SqailApp>, n: usize, few: usize) {
    h.state_mut().service.connections.push(big_connection());
    let mut schemas = vec![NamedItem { name: "big".into() }];
    schemas.extend((1..=few).map(|i| NamedItem {
        name: format!("s{i}"),
    }));
    put(h, SchemaKey::Schemas, SchemaData::Schemas(schemas));
    let table = |s: &str, name: String| TableInfo {
        schema: Some(s.into()),
        name,
        kind: TableKind::Table,
    };
    put(
        h,
        SchemaKey::Tables("big".into()),
        SchemaData::Tables((0..n).map(|i| table("big", format!("t_{i:05}"))).collect()),
    );
    put(
        h,
        SchemaKey::Routines("big".into()),
        SchemaData::Routines(vec![]),
    );
    for i in 1..=few {
        let s = format!("s{i}");
        put(
            h,
            SchemaKey::Tables(s.clone()),
            SchemaData::Tables(
                (0..10)
                    .map(|j| table(&s, format!("{s}_table_{j}")))
                    .collect(),
            ),
        );
        put(
            h,
            SchemaKey::Routines(s.clone()),
            SchemaData::Routines(vec![RoutineInfo {
                schema: Some(s.clone()),
                name: format!("{s}_fn"),
                kind: RoutineKind::Function,
            }]),
        );
    }
}

/// Median wall time of `frames` frames.
fn frame_time(h: &mut Harness<'_, SqailApp>, frames: usize) -> Duration {
    let mut times: Vec<Duration> = (0..frames)
        .map(|_| {
            let t = Instant::now();
            h.step();
            t.elapsed()
        })
        .collect();
    times.sort();
    times[frames / 2]
}

/// 5 000 tables in one schema: frame times with the schema open, unfiltered
/// and filtered. Printed (run with `--nocapture`) and bounded loosely, so a
/// slow CI machine does not fail it but drawing every row every frame does.
#[test]
fn large_catalog_frame_time() {
    let _e = env();
    let mut h = app();
    catalog(&mut h, 5000, 0);
    h.run_steps(2);
    let base = frame_time(&mut h, 30);
    h.run_steps(2);
    h.get_by_label("big").click();
    h.run_steps(4);
    // A single schema: its tables are listed right under the connection.
    h.get_by_label("Tables (5000)");
    let open = frame_time(&mut h, 30);
    h.state_mut().schema.filter = "t_04".into();
    h.run_steps(4);
    h.get_by_label("Tables (1000)");
    let filtered = frame_time(&mut h, 30);
    eprintln!(
        "schema tree, 5000 tables: closed {base:?}, open {open:?}, filtered (1000 rows) {filtered:?}"
    );
    // Before rows off screen were skipped, an open 5 000-table schema took
    // ~400 ms a frame in a debug build; now it is about the closed tree's.
    assert!(
        open < base * 4 + Duration::from_millis(30),
        "an open 5000-table schema costs {open:?} a frame (closed: {base:?})"
    );
    assert!(
        filtered < base * 4 + Duration::from_millis(30),
        "a filtered 5000-table schema costs {filtered:?} a frame (closed: {base:?})"
    );

    // Scrolled to the end, the last table is drawn where it belongs.
    h.state_mut().schema.filter.clear();
    h.run_steps(4);
    h.get_by_label("big").hover();
    h.run_steps(2);
    for _ in 0..40 {
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Page,
            delta: egui::vec2(0.0, -100.0),
            modifiers: egui::Modifiers::NONE,
            phase: egui::TouchPhase::Move,
        });
        h.step();
    }
    h.run_steps(4);
    let last = h.get_by_label("⊞ t_04999");
    let before = h.get_by_label("⊞ t_04998");
    let (a, b) = (before.rect(), last.rect());
    assert!(
        (b.top() - a.bottom()).abs() < 8.0,
        "rows stay in order: {a:?} then {b:?}"
    );
}

/// The filter opens every node with a match (also one closed earlier), hides
/// schemas without one, and matches loaded columns.
#[test]
fn filter_reveals_matches() {
    let _e = env();
    let mut h = app();
    catalog(&mut h, 20, 3);
    put(
        &mut h,
        SchemaKey::Columns("s2".into(), "s2_table_5".into()),
        SchemaData::Columns(vec![ColumnInfo {
            name: "needle_col".into(),
            ordinal: 1,
            data_type: "int".into(),
            nullable: true,
            default: None,
            primary_key: false,
        }]),
    );
    h.run_steps(2);
    h.get_by_label("big").click();
    h.run_steps(4);
    // Open s1 and close it again: the filter must still open it.
    h.get_by_label("s1").click();
    h.run_steps(4);
    h.get_by_label("Tables (10)");
    h.get_by_label("s1").click();
    h.run_steps(4);
    assert!(h.query_by_label("⊞ s1_table_3").is_none());

    h.state_mut().schema.filter = "S1_TABLE_3".into();
    h.run_steps(6);
    h.get_by_label("⊞ s1_table_3");
    assert!(h.query_by_label("s2").is_none(), "s2 has no match");
    assert!(h.query_by_label("s3").is_none(), "s3 has no match");
    assert!(h.query_by_label("big").is_some());

    // A routine matches too.
    h.state_mut().schema.filter = "s3_fn".into();
    h.run_steps(6);
    h.get_by_label("ƒ s3_fn");
    assert!(h.query_by_label("s1").is_none());

    // A loaded column matches: its table is listed and opened.
    h.state_mut().schema.filter = "needle".into();
    h.run_steps(6);
    h.get_by_label("⊞ s2_table_5");
    h.get_by_label("needle_col");
    assert!(h.query_by_label("⊞ s2_table_4").is_none());

    // No filter: everything is back.
    h.state_mut().schema.filter.clear();
    h.run_steps(6);
    h.get_by_label("s1");
    h.get_by_label("s3");
}

/// Typing a filter loads every schema's tables in one request, not one
/// request per schema.
#[test]
fn filter_loads_tables_in_one_request() {
    let _e = env();
    let mut h = app();
    h.state_mut().service.connections.push(big_connection());
    let schemas = (0..50)
        .map(|i| NamedItem {
            name: format!("s{i}"),
        })
        .collect();
    put(&mut h, SchemaKey::Schemas, SchemaData::Schemas(schemas));
    h.run_steps(2);
    h.get_by_label("big").click();
    h.run_steps(4);
    h.state_mut().schema.filter = "orders".into();
    h.run_steps(10);
    let keys = h.state().schema.cached_keys(BIG.parse().unwrap());
    assert!(keys.contains(&SchemaKey::AllTables), "{keys:?}");
    assert!(keys.contains(&SchemaKey::AllRoutines), "{keys:?}");
    let per_schema = keys
        .iter()
        .filter(|k| matches!(k, SchemaKey::Tables(_) | SchemaKey::Routines(_)))
        .count();
    assert_eq!(per_schema, 0, "no request per schema: {keys:?}");

    // The batch's answer is filed per schema; schemas without a match hide.
    let table = |s: &str, n: &str| TableInfo {
        schema: Some(s.into()),
        name: n.into(),
        kind: TableKind::Table,
    };
    put(
        &mut h,
        SchemaKey::AllTables,
        SchemaData::Tables(vec![table("s7", "orders"), table("s9", "customers")]),
    );
    put(&mut h, SchemaKey::AllRoutines, SchemaData::Routines(vec![]));
    h.run_steps(6);
    h.get_by_label("⊞ orders");
    assert!(h.query_by_label("s9").is_none());
    assert!(h.query_by_label("s0").is_none());
}
