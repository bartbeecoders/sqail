//! The connection form's database list (the ⏷ next to Database), against the
//! podman PostgreSQL test database: fill in the form without a database, pick
//! `sqail_test` from the list and save.

use std::time::{Duration, Instant};

use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use sqail_client::proto::ConnectionParams;
use sqail_ui::SqailApp;
use sqail_ui::app::ServiceStatus;
use sqail_ui::settings::{ServiceProfile, Settings};

/// Screenshots land in `target/ui-shots/`; rendering needs a GPU adapter.
fn shot(h: &mut Harness<'_, SqailApp>, name: &str) {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/ui-shots");
    std::fs::create_dir_all(&dir).unwrap();
    match h.render() {
        Ok(img) => img.save(dir.join(format!("{name}.png"))).unwrap(),
        Err(e) => eprintln!("no screenshot {name}: {e}"),
    }
}

/// Step frames until `done` holds (background messages arrive between frames).
fn wait(h: &mut Harness<'_, SqailApp>, what: &str, done: impl Fn(&Harness<'_, SqailApp>) -> bool) {
    let start = Instant::now();
    while !done(h) {
        if start.elapsed() > Duration::from_secs(20) {
            shot(h, "conn-form-timeout");
            panic!("timed out waiting for {what}");
        }
        h.step();
        std::thread::sleep(Duration::from_millis(15));
    }
    h.step();
}

/// Replace the text of the field labelled `label`.
fn fill(h: &mut Harness<'_, SqailApp>, label: &str, text: &str) {
    h.get_by_label(label).focus();
    h.step();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.get_by_label(label).type_text(text);
    h.step();
}

#[test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
fn database_list_fills_the_database_field() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let cfg = sqail_service::Config {
        data_dir: dir.path().join("service"),
        bind: "127.0.0.1:0".parse().unwrap(),
        docs_ui: false,
        ..Default::default()
    };
    std::fs::create_dir_all(&cfg.data_dir).unwrap();
    let server = rt.block_on(sqail_service::start(cfg)).unwrap();
    let url = format!("https://{}", server.addr);
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
    sqail_ui::secrets::save_token(&url, server.bootstrap_token.as_ref().unwrap()).unwrap();

    let mut h = Harness::builder()
        .with_size([1280.0, 800.0])
        .build_eframe(|cc| SqailApp::new(cc));
    wait(&mut h, "service connection", |h| {
        h.state().service.status == ServiceStatus::Connected
    });

    h.get_by_label("+ Connection").click();
    h.step();
    // PostgreSQL on 127.0.0.1 is the form's default.
    fill(&mut h, "Port", "55432");
    fill(&mut h, "User", "sqail");
    fill(&mut h, "Password", "sqail_dev_pw");

    h.get_by_label("⏷").click();
    wait(&mut h, "the database list", |h| {
        h.query_by_label("sqail_test").is_some()
    });
    shot(&mut h, "conn-form-databases");
    h.get_by_label("sqail_test").click();
    h.step();
    h.step();
    assert!(
        h.query_by_label("Refresh").is_none(),
        "picking a database closes the list"
    );

    h.get_by_label("Save").click();
    wait(&mut h, "the saved connection", |h| {
        !h.state().service.connections.is_empty()
    });
    let conn = &h.state().service.connections[0];
    assert_eq!(conn.name, "127.0.0.1/sqail_test");
    let ConnectionParams::Postgres(p) = &conn.params else {
        panic!("{:?}", conn.params)
    };
    assert_eq!(p.database, "sqail_test");
}
