//! The table designer driven through the real UI (egui_kittest) against an
//! in-process sqail-service with an SQLite database: design an existing
//! table, create a new one, drop it. Screenshots land in `target/ui-shots/`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use sqail_client::proto::{ConnectionInput, ConnectionParams, QueryRequest, SqliteParams};
use sqail_client::{Client, On, Target, Trust};
use sqail_ui::SqailApp;
use sqail_ui::app::ServiceStatus;
use sqail_ui::dialogs::Dialog;
use sqail_ui::settings::{ServiceProfile, Settings};

fn shot(h: &mut Harness<'_, SqailApp>, name: &str) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/ui-shots");
    std::fs::create_dir_all(&dir).unwrap();
    match h.render() {
        Ok(img) => img.save(dir.join(format!("{name}.png"))).unwrap(),
        Err(e) => eprintln!("no screenshot {name}: {e}"),
    }
}

fn wait(h: &mut Harness<'_, SqailApp>, what: &str, done: impl Fn(&SqailApp) -> bool) {
    let start = Instant::now();
    while !done(h.state()) {
        if start.elapsed() >= Duration::from_secs(20) {
            shot(h, "designer-timeout");
        }
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "timed out waiting for {what}"
        );
        h.step();
        std::thread::sleep(Duration::from_millis(15));
    }
    h.run_steps(3);
}

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
        let conn = client
            .create_connection(&ConnectionInput {
                name: "people db".into(),
                params: ConnectionParams::Sqlite(SqliteParams {
                    path: dir.path().join("people.db").to_string_lossy().into(),
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
        client
            .query_all(
                On::Connection(conn.id),
                &QueryRequest::new(
                    "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT NOT NULL, score REAL);
                     INSERT INTO people (name, score) VALUES ('Ada', 9.5), ('Linus', 8.25);",
                ),
            )
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

fn designer_ready(a: &SqailApp) -> bool {
    a.designers
        .list
        .last()
        .is_some_and(|d| d.design.is_some() && !d.busy)
}

#[test]
fn design_create_and_drop_tables() {
    let _e = env();
    let mut h = Harness::builder()
        .with_size([1400.0, 900.0])
        .build_eframe(|cc| SqailApp::new(cc));
    wait(&mut h, "service connection", |a| {
        a.service.status == ServiceStatus::Connected && !a.service.connections.is_empty()
    });
    h.get_by_label("people db").click(); // expand the connection
    h.run_steps(2);
    let start = Instant::now();
    while h.query_by_label("⊞ people").is_none() {
        assert!(start.elapsed() < Duration::from_secs(20), "table in tree");
        h.step();
        std::thread::sleep(Duration::from_millis(15));
    }

    // --- design an existing table: add a column ---------------------------
    h.get_by_label("⊞ people").click_secondary();
    h.run_steps(2);
    h.get_by_label("Design table…").click();
    wait(&mut h, "designer to load", designer_ready);
    h.get_by_label("Columns (3)");

    // The window keeps its size while the mouse moves (every move repaints),
    // also with a problem line above the buttons.
    h.state_mut().designers.list[0]
        .design
        .as_mut()
        .unwrap()
        .table
        .name
        .clear();
    h.run_steps(4);
    let footer = |h: &Harness<'_, SqailApp>| h.get_by_label("Drop table…").rect();
    let before = footer(&h);
    for i in 0..60 {
        let t = i as f32;
        h.hover_at(egui::pos2(
            300.0 + (t * 37.0) % 900.0,
            150.0 + (t * 23.0) % 600.0,
        ));
        h.step();
    }
    let after = footer(&h);
    assert!(
        (after.bottom() - before.bottom()).abs() < 0.5,
        "the designer grew from {before:?} to {after:?}"
    );
    h.state_mut().designers.list[0]
        .design
        .as_mut()
        .unwrap()
        .table
        .name = "people".into();
    h.run_steps(2);
    h.get_by_label("+ Add column").click();
    h.run_steps(2);
    {
        let d = h.state_mut().designers.list[0].design.as_mut().unwrap();
        let id = d.table.columns.last().unwrap().id;
        d.column_mut(id).unwrap().name = "email".into();
    }
    h.run_steps(2);
    shot(&mut h, "designer-01-columns");
    h.get_by_label("Apply 1 change(s)…").click();
    h.run_steps(2);
    {
        let d = &h.state().designers.list[0];
        assert!(d.review);
        let plan = d.design.as_ref().unwrap().plan().unwrap();
        assert_eq!(plan.body, ["ALTER TABLE people ADD COLUMN email TEXT"]);
    }
    shot(&mut h, "designer-02-review");
    h.get_by_label("Apply").click();
    h.step();
    wait(&mut h, "change to apply and reload", |a| {
        let d = &a.designers.list[0];
        designer_ready(a)
            && !d.review
            && d.design
                .as_ref()
                .is_some_and(|x| x.table.columns.len() == 4 && x.plan().is_ok_and(|p| p.is_empty()))
    });
    shot(&mut h, "designer-02b-applied");
    h.get_by_label("Columns (4)");
    assert!(
        h.state().designers.list[0]
            .design
            .as_ref()
            .unwrap()
            .plan()
            .unwrap()
            .is_empty(),
        "reloaded table matches the design"
    );

    // --- create a table from the Tables header ----------------------------
    h.get_by_label("Tables (1)").click_secondary();
    h.run_steps(2);
    h.get_by_label("New table…").click();
    h.run_steps(2);
    assert_eq!(h.state().designers.list.len(), 2);
    {
        let d = h.state_mut().designers.list[1].design.as_mut().unwrap();
        d.table.name = "pets".into();
        let c = d.add_column();
        d.column_mut(c).unwrap().name = "owner".into();
        d.column_mut(c).unwrap().data_type = "INTEGER".into();
        let ix = d.add_index();
        d.index_mut(ix).unwrap().parts = vec![sqail_ui::designer::model::IndexPart::Column(c)];
    }
    h.run_steps(6);
    shot(&mut h, "designer-03-new-table");
    h.get_by_label("Indexes (1)").click();
    h.run_steps(4);
    shot(&mut h, "designer-03b-indexes");
    // Both windows have this tab.
    h.state_mut().designers.list[1].section = sqail_ui::designer::Section::PrimaryKey;
    h.run_steps(4);
    shot(&mut h, "designer-03c-primary-key");
    h.get_by_label("Create table…").click();
    h.run_steps(2);
    h.get_by_label("Create").click();
    h.step();
    wait(&mut h, "table to be created", |a| {
        a.designers
            .list
            .get(1)
            .is_some_and(|d| d.table.is_some() && d.design.is_some())
    });
    let start = Instant::now();
    while h.query_by_label("⊞ pets").is_none() {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "new table in tree"
        );
        h.step();
        std::thread::sleep(Duration::from_millis(15));
    }

    // --- drop it from the tree ----------------------------------------------
    h.get_by_label("⊞ pets").click_secondary();
    h.run_steps(2);
    // The tree's menu entry; the designer window has a button of that name too.
    h.get_all_by_label("Drop table…")
        .find(|n| n.rect().left() < 300.0)
        .expect("context menu entry")
        .click();
    h.run_steps(2);
    h.get_by_label_contains("DROP TABLE pets");
    shot(&mut h, "designer-04-drop");
    h.get_by_label("Drop").click();
    h.step();
    wait(&mut h, "drop", |a| matches!(a.dialog, Dialog::None));
    assert_eq!(
        h.state().designers.list.len(),
        1,
        "the dropped table's designer closed"
    );
    let start = Instant::now();
    while h.query_by_label("⊞ pets").is_some() {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "table gone from tree"
        );
        h.step();
        std::thread::sleep(Duration::from_millis(15));
    }
}
