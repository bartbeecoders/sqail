//! Headless end-to-end test of the real app: an in-process sqail-service with
//! an SQLite database, the sqail UI driven by egui_kittest (clicks, keys) and
//! rendered with wgpu. Screenshots land in `target/ui-shots/` for review.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use sqail_client::proto::{ConnectionInput, ConnectionParams, QueryRequest, SqliteParams};
use sqail_client::{Client, On, Target, Trust};
use sqail_ui::SqailApp;
use sqail_ui::app::ServiceStatus;
use sqail_ui::results::Pane;
use sqail_ui::settings::{ServiceProfile, Settings};

fn shots_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/ui-shots");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn shot(h: &mut Harness<'_, SqailApp>, name: &str) {
    // Rendering needs a GPU adapter; skip quietly where there is none.
    match h.render() {
        Ok(img) => img.save(shots_dir().join(format!("{name}.png"))).unwrap(),
        Err(e) => eprintln!("no screenshot {name}: {e}"),
    }
}

/// Step frames until `done` holds (background messages arrive between frames).
fn wait(h: &mut Harness<'_, SqailApp>, what: &str, done: impl Fn(&SqailApp) -> bool) {
    let start = Instant::now();
    while !done(h.state()) {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "timed out waiting for {what}"
        );
        h.step();
        std::thread::sleep(Duration::from_millis(15));
    }
    h.step();
}

fn run_id(h: &Harness<'_, SqailApp>) -> u64 {
    let app = h.state();
    app.tabs[app.active].run.as_ref().map_or(0, |r| r.id)
}

/// Press keys, then wait for a *new* run in the active tab to finish.
fn run_with(
    h: &mut Harness<'_, SqailApp>,
    what: &str,
    press: impl FnOnce(&mut Harness<'_, SqailApp>),
) {
    let before = run_id(h);
    press(h);
    wait(h, what, |a| {
        a.tabs[a.active]
            .run
            .as_ref()
            .is_some_and(|r| r.id != before && !r.running)
    });
}

/// A running service with an SQLite connection "people db" (table `people`),
/// and a UI config dir pointing at it.
struct Env {
    rt: tokio::runtime::Runtime,
    dir: tempfile::TempDir,
    server: sqail_service::Server,
    ui_dir: PathBuf,
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
                color: Some("#c0392b".into()),
                environment: Some("test".into()),
                folder: None,
            })
            .await
            .unwrap();
        client
            .query_all(
                On::Connection(conn.id),
                &QueryRequest::new(
                    "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT NOT NULL, score REAL);
                     INSERT INTO people (name, score) VALUES ('Ada', 9.5), ('Linus', 8.25), ('Grace', NULL);",
                ),
            )
            .await
            .unwrap();
    });
    let ui_dir = dir.path().join("ui");
    sqail_ui::settings::override_config_dir(ui_dir.clone());
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
        rt,
        dir,
        server,
        ui_dir,
    }
}

fn app(size: [f32; 2]) -> Harness<'static, SqailApp> {
    let mut h = Harness::builder()
        .with_size(size)
        .build_eframe(|cc| SqailApp::new(cc));
    wait(&mut h, "service connection", |a| {
        a.service.status == ServiceStatus::Connected && !a.service.connections.is_empty()
    });
    h
}

#[test]
fn workspace_history_and_snippets_survive_a_restart() {
    let e = env();
    {
        let mut h = app([1280.0, 800.0]);
        h.state_mut().tabs[0].text = "SELECT 42 AS answer".into();
        h.step();
        run_with(&mut h, "answer", |h| {
            h.key_press_modifiers(Modifiers::COMMAND, Key::Enter)
        });
        assert_eq!(h.state().history.entries.len(), 1);
        h.key_press_modifiers(Modifiers::COMMAND, Key::T);
        h.step();
        h.state_mut().tabs[1].text = "-- unsaved draft".into();
        h.state_mut()
            .snippets
            .add("the answer".into(), "SELECT 42 AS answer".into());
        // Autosave runs about once a second; no clean shutdown afterwards.
        std::thread::sleep(Duration::from_millis(1100));
        h.run_steps(3);
    }
    assert!(e.ui_dir.join("workspace.json").exists());
    // A custom keybinding takes effect on the next start.
    std::fs::write(
        e.ui_dir.join("keybindings.toml"),
        "\"query.run\" = \"Ctrl+R\"\n",
    )
    .unwrap();

    let mut h = app([1280.0, 800.0]);
    {
        let a = h.state();
        assert_eq!(a.tabs.len(), 2);
        assert_eq!(a.tabs[0].text, "SELECT 42 AS answer");
        assert_eq!(a.tabs[1].text, "-- unsaved draft");
        assert_eq!(a.active, 1);
        assert!(a.tabs[1].connection.is_some(), "tabs keep their connection");
        assert_eq!(a.history.entries.len(), 1);
    }
    h.state_mut().active = 0;
    h.step();
    run_with(&mut h, "custom Ctrl+R", |h| {
        h.key_press_modifiers(Modifiers::COMMAND, Key::R)
    });
    assert_eq!(h.state().history.entries.len(), 2);
    h.state_mut().active = 1;
    h.step();
    h.get_by_label("History").click();
    h.run_steps(2);
    assert_eq!(h.query_all_by_label("SELECT 42 AS answer").count(), 2);
    shot(&mut h, "06-history");
    h.get_by_label("Snippets").click();
    h.run_steps(2);
    h.get_by_label("the answer").click_secondary();
    h.run_steps(2);
    h.get_by_label("Insert at cursor").click();
    h.run_steps(2);
    assert_eq!(
        h.state().tabs[1].text,
        "SELECT 42 AS answer-- unsaved draft",
        "inserted at the cursor"
    );
    drop(h);
    e.server.shutdown();
    drop(e.rt);
    drop(e.dir);
}

/// A `workspace.json` that does not parse (torn write, hand edit) is kept as
/// `workspace.json.bad` and reported, not replaced by the next autosave.
#[test]
fn a_damaged_workspace_is_kept_and_reported() {
    let e = env();
    let damaged = r#"{"tabs":[{"title":"x""#;
    std::fs::write(e.ui_dir.join("workspace.json"), damaged).unwrap();

    let mut h = app([1280.0, 800.0]);
    assert_eq!(h.state().tabs.len(), 1, "starts with one empty tab");
    h.state_mut().tabs[0].text = "SELECT 'new work'".into();
    // Autosave runs about once a second.
    std::thread::sleep(Duration::from_millis(1100));
    h.run_steps(3);
    assert!(
        h.query_by_label_contains("workspace.json could not be read")
            .is_some(),
        "the user is told"
    );
    shot(&mut h, "damaged-workspace");

    assert_eq!(
        std::fs::read_to_string(e.ui_dir.join("workspace.json.bad")).unwrap(),
        damaged,
        "the damaged file is kept"
    );
    let saved = std::fs::read_to_string(e.ui_dir.join("workspace.json")).unwrap();
    assert!(saved.contains("new work"), "{saved}");
    drop(h);

    // The next start is clean and keeps the earlier .bad file.
    let h = app([1280.0, 800.0]);
    assert_eq!(h.state().tabs[0].text, "SELECT 'new work'");
    assert!(
        h.query_by_label_contains("could not be read").is_none(),
        "no notice for a good file"
    );
    assert!(!e.ui_dir.join("workspace.json.1.bad").exists());
    drop(h);
    e.server.shutdown();
    drop(e.rt);
    drop(e.dir);
}

/// The Azure discovery dialog with a canned result (no Azure needed): add
/// the new databases, and see the existing one marked as added.
#[test]
fn azure_discovery_adds_connections() {
    use sqail_client::proto::{
        AzureDatabase, AzureDiscovery, AzureServerKind, AzureSubscription, MssqlAuth, MssqlEncrypt,
        MssqlParams,
    };
    use sqail_ui::dialogs::{Dialog, DiscoverForm};

    let e = env();
    let client = Client::new(&Target {
        url: format!("https://{}", e.server.addr),
        token: e.server.bootstrap_token.clone().unwrap(),
        trust: Trust::Pinned(e.server.fingerprint.clone()),
        identity: None,
    })
    .unwrap();
    let source =
        e.rt.block_on(client.create_connection(&ConnectionInput {
            name: "azure first".into(),
            params: ConnectionParams::Mssql(MssqlParams {
                host: "first.database.windows.net".into(),
                port: 1433,
                instance: None,
                database: Some("first".into()),
                auth: MssqlAuth::EntraServicePrincipal {
                    tenant: "contoso.onmicrosoft.com".into(),
                    client_id: "app".into(),
                },
                encrypt: MssqlEncrypt::Required,
                trust_server_certificate: false,
            }),
            password: Some("client-secret".into()),
            ssl_client_key: None,
            read_only: false,
            color: None,
            environment: None,
            folder: None,
        }))
        .unwrap();
    let db = |kind, server: &str, host: &str, database: &str| AzureDatabase {
        kind,
        subscription_id: "s1".into(),
        resource_group: "rg-data".into(),
        server: server.into(),
        host: host.into(),
        port: if kind == AzureServerKind::PostgresFlexible {
            5432
        } else {
            1433
        },
        database: database.into(),
        location: "westeurope".into(),
        admin_login: Some("dbadmin".into()),
    };
    let found = AzureDiscovery {
        subscriptions: vec![AzureSubscription {
            id: "s1".into(),
            name: "Development".into(),
        }],
        databases: vec![
            db(
                AzureServerKind::SqlServer,
                "first",
                "first.database.windows.net",
                "first",
            ),
            db(
                AzureServerKind::SqlServer,
                "first",
                "first.database.windows.net",
                "sales",
            ),
            db(
                AzureServerKind::PostgresFlexible,
                "pg1",
                "pg1.postgres.database.azure.com",
                "app",
            ),
        ],
        warnings: vec!["Locked (Azure SQL): AuthorizationFailed: no access".into()],
    };

    let mut h = app([1280.0, 800.0]);
    wait(&mut h, "both connections", |a| {
        a.service.connections.len() == 2
    });
    {
        let app = h.state_mut();
        let src = app.service.connection(source.id).unwrap().clone();
        let mut form = DiscoverForm::new(&src);
        form.on_discovered(Ok(found), &app.service.connections);
        app.dialog = Dialog::AzureDiscover(Box::new(form));
    }
    h.run_steps(4);
    shot(&mut h, "15-azure-discovery");
    h.get_by_label("Add 2 connections").click();
    wait(&mut h, "added connections", |a| {
        a.service.connections.len() == 4 && matches!(a.dialog, Dialog::None)
    });
    let conns = &h.state().service.connections;
    let sales = conns.iter().find(|c| c.name == "first/sales").unwrap();
    assert!(sales.has_password, "the client secret was copied");
    assert_eq!(sales.folder.as_deref(), Some("Azure"));
    let pg = conns.iter().find(|c| c.name == "pg1/app").unwrap();
    assert!(!pg.has_password);
    match &pg.params {
        ConnectionParams::Postgres(p) => assert_eq!(p.user, "dbadmin"),
        other => panic!("{other:?}"),
    }
}

/// The tab's × closes it, a middle click too, and a click on the tab
/// selects it.
#[test]
fn tabs_close_with_the_close_button() {
    let e = env();
    let mut h = app([1280.0, 800.0]);
    h.state_mut().new_tab(None);
    h.state_mut().new_tab(None);
    h.run_steps(2);
    assert_eq!(h.state().tabs.len(), 3);

    h.get_by_label("Query 1").click();
    h.run_steps(2);
    assert_eq!(h.state().active, 0, "clicking a tab selects it");

    h.get_all_by_label("×")
        .nth(1)
        .expect("second tab's ×")
        .click();
    h.run_steps(2);
    let titles: Vec<_> = h.state().tabs.iter().map(|t| t.title.clone()).collect();
    assert_eq!(titles, ["Query 1", "Query 3"]);

    h.get_by_label("Query 3")
        .click_button(egui::PointerButton::Middle);
    h.run_steps(2);
    assert_eq!(h.state().tabs.len(), 1);

    // Unsaved changes: Save in the prompt writes the file, then closes the tab.
    let file = e.dir.path().join("notes.sql");
    let idx = h.state_mut().new_tab(None);
    h.state_mut().tabs[idx].set_file(file.clone(), "SELECT 1".into());
    h.state_mut().tabs[idx].text = "SELECT 2".into();
    h.run_steps(2);
    h.get_all_by_label("×").nth(1).expect("notes.sql ×").click();
    h.run_steps(4);
    h.get_by_label("Unsaved changes");
    h.get_by_label("Save").click();
    wait(&mut h, "saved and closed", |a| a.tabs.len() == 1);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "SELECT 2");
}

/// Only the text that was written is marked saved, and the unsaved-changes
/// prompt acts on its own tab even when tabs before it close meanwhile (#156).
#[test]
fn saving_and_close_prompts_follow_the_right_text_and_tab() {
    let e = env();
    let mut h = app([1280.0, 800.0]);
    // A save finishes after the user typed on: the tab stays dirty.
    let file = e.dir.path().join("race.sql");
    let id = h.state().tabs[0].id;
    h.state_mut().tabs[0].text = "SELECT 1 -- more".into();
    std::fs::write(&file, "SELECT 1").unwrap();
    h.state().worker.sink().send(sqail_ui::app::Msg::FileSaved {
        tab: id,
        result: Ok((file.clone(), "SELECT 1".into())),
        close: false,
    });
    h.run_steps(2);
    assert_eq!(h.state().tabs[0].title, "race.sql");
    assert!(
        h.state().tabs[0].is_dirty(),
        "the typed text is not in the file"
    );

    // Tabs A, B, C (all dirty); the prompt for C is open when A closes.
    for name in ["B", "C"] {
        let i = h.state_mut().new_tab(None);
        h.state_mut().tabs[i].text = format!("SELECT '{name}'");
    }
    h.run_steps(2);
    let c = h.state().tabs[2].id;
    h.state_mut().request_close_tab(2);
    h.run_steps(2);
    assert!(matches!(h.state().dialog, sqail_ui::dialogs::Dialog::ConfirmClose(x) if x == c));
    h.state_mut().close_tab(0);
    h.run_steps(2);
    h.get_by_label("Discard").click();
    h.run_steps(2);
    let texts: Vec<_> = h.state().tabs.iter().map(|t| t.text.clone()).collect();
    assert_eq!(
        texts,
        ["SELECT 'B'"],
        "C was discarded, not the tab after it"
    );

    // A prompt whose tab is gone just closes.
    h.state_mut().request_close_tab(0);
    h.run_steps(2);
    let b = h.state().tabs[0].id;
    h.state_mut().new_tab(None);
    h.state_mut().close_tab(0);
    h.run_steps(2);
    assert!(h.state().tab_index(b).is_none());
    assert!(matches!(h.state().dialog, sqail_ui::dialogs::Dialog::None));
}

/// A revoked token, found by any request after startup, shows in the status
/// bar and leads back to the token prompt (#157).
#[test]
fn a_revoked_token_leads_back_to_the_token_prompt() {
    let e = env();
    let mut h = app([1280.0, 800.0]);
    let out = outside(&e.server);
    e.rt.block_on(async {
        for t in out.tokens().await.unwrap() {
            out.revoke_token(t.id).await.unwrap();
        }
    });
    h.state_mut().tabs[0].text = "SELECT 1".into();
    h.step();
    run_with(&mut h, "rejected query", |h| h.key_press(Key::F5));
    wait(&mut h, "token prompt", |a| {
        matches!(a.dialog, sqail_ui::dialogs::Dialog::Welcome(_))
    });
    assert!(matches!(h.state().service.status, ServiceStatus::Failed(_)));
    h.get_by_label_contains("no longer accepts the token");
    shot(&mut h, "13-token-revoked");
}

/// When the service goes away the status bar says so (with Reconnect), and
/// says Connected again once it answers (#157).
#[test]
fn losing_the_service_shows_in_the_status_bar() {
    let Env {
        rt,
        dir: _dir,
        server,
        ui_dir: _,
    } = env();
    let mut h = app([1280.0, 800.0]);
    server.shutdown();
    rt.block_on(server.wait()).unwrap();
    h.state_mut().tabs[0].text = "SELECT 1".into();
    h.step();
    run_with(&mut h, "query without a service", |h| h.key_press(Key::F5));
    assert!(h.state().tabs[0].run.as_ref().unwrap().failed);
    wait(&mut h, "status failed", |a| {
        matches!(a.service.status, ServiceStatus::Failed(_))
    });
    h.get_by_label("Reconnect");
    shot(&mut h, "14-service-lost");
}

/// A tab is bound to a connection by itself only when it is the only one:
/// deleting the tab's connection leaves it unbound when several remain (#157).
#[test]
fn tabs_are_not_bound_to_the_first_of_several_connections() {
    let e = env();
    let mut h = app([1280.0, 800.0]);
    let only = h.state().tabs[0].connection;
    assert!(only.is_some(), "the only connection is chosen");
    let out = outside(&e.server);
    e.rt.block_on(async {
        for name in ["production", "staging"] {
            out.create_connection(&ConnectionInput {
                name: name.into(),
                params: ConnectionParams::Sqlite(SqliteParams {
                    path: e
                        .dir
                        .path()
                        .join(format!("{name}.db"))
                        .to_string_lossy()
                        .into(),
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
        }
        // The tab's connection goes away; two others remain.
        out.delete_connection(only.unwrap()).await.unwrap();
    });
    h.state_mut().refresh_connections();
    wait(&mut h, "new list", |a| {
        a.service.connections.len() == 2 && a.service.connections.iter().all(|c| Some(c.id) != only)
    });
    assert_eq!(
        h.state().tabs[0].connection,
        None,
        "not bound to production"
    );
}

/// A profile of an engine newer than this build is listed as unsupported, and
/// not offered in the tab's connection picker.
#[test]
fn unknown_engines_are_listed_as_unsupported() {
    let _e = env();
    let mut h = app([1280.0, 800.0]);
    let newer: sqail_client::proto::Connection = serde_json::from_value(serde_json::json!({
        "id": "00000000-0000-0000-0000-0000000000aa", "name": "warehouse",
        "engine": "oracle", "params": {"engine": "oracle", "host": "db"},
        "has_password": true, "read_only": false, "color": null, "environment": null,
        "folder": null, "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"
    }))
    .unwrap();
    h.state_mut().service.connections.push(newer);
    h.run_steps(2);
    h.get_by_label("warehouse");
    h.get_by_label("not supported by this sqail version");
    shot(&mut h, "unknown-engine");

    let idx = h.state_mut().new_tab(None);
    h.state_mut().tabs[idx].connection = None;
    h.run_steps(2);
    h.get_by_role(egui::accesskit::Role::ComboBox).click();
    h.run_steps(2);
    assert!(
        h.query_by_label_contains("people db  ·  SQLite").is_some(),
        "known connections are offered"
    );
    assert!(
        h.query_by_label_contains("warehouse  ·").is_none(),
        "unsupported connections are not offered"
    );
    h.key_press(Key::Escape);
    h.run_steps(2);

    // Its menu still offers Delete.
    real_click(&mut h, "warehouse", egui::PointerButton::Secondary);
    h.run_steps(2);
    h.get_by_label("Delete…").click();
    h.run_steps(2);
    assert!(matches!(
        h.state().dialog,
        sqail_ui::dialogs::Dialog::ConfirmDelete(_, ref name) if name == "warehouse"
    ));
}

/// The connection tree: a new folder from the toolbar, renamed in place, a
/// connection dragged into it, and the connection and folder renamed.
#[test]
fn connection_tree_folders_rename_and_drag() {
    let e = env();
    let url = format!("https://{}", e.server.addr);
    let mut h = app([1280.0, 800.0]);
    let settle = |h: &mut Harness<'_, SqailApp>| h.run_steps(4);
    let type_and_enter = |h: &mut Harness<'_, SqailApp>, text: &str| {
        // The field opens focused with its text selected: typing replaces it.
        h.event(egui::Event::Text(text.into()));
        h.step();
        h.key_press(Key::Enter);
        h.run_steps(4);
    };

    // New folder: the toolbar adds it and opens it for renaming.
    h.get_by_label("+ Folder").click();
    settle(&mut h);
    type_and_enter(&mut h, "Prod");
    assert_eq!(h.state().settings.folders[&url], ["Prod"]);
    h.get_by_label("Empty: drag connections here");
    shot(&mut h, "18-tree-new-folder");

    // Drag the connection onto the folder.
    let from = h.get_by_label("people db").rect().center();
    let to = h.get_by_label("Prod").rect().center();
    h.hover_at(from);
    h.step();
    h.drag_at(from);
    h.step();
    for i in 1..=6 {
        h.hover_at(from + (to - from) * (i as f32 / 6.0));
        h.step();
    }
    shot(&mut h, "19-tree-dragging");
    h.drop_at(to);
    settle(&mut h);
    wait(&mut h, "moved into the folder", |a| {
        a.service.connections[0].folder.as_deref() == Some("Prod")
    });
    assert!(
        !h.state().settings.folders.contains_key(&url),
        "a folder with connections is stored on the service"
    );

    // Rename the connection, then the folder, in place.
    h.get_by_label("people db").click_secondary();
    settle(&mut h);
    h.get_by_label("Rename").click();
    settle(&mut h);
    type_and_enter(&mut h, "people");
    wait(&mut h, "renamed connection", |a| {
        a.service.connections[0].name == "people"
    });
    h.get_by_label("Prod").click_secondary();
    settle(&mut h);
    h.get_by_label("Rename").click();
    settle(&mut h);
    type_and_enter(&mut h, "Production");
    wait(&mut h, "renamed folder", |a| {
        a.service.connections[0].folder.as_deref() == Some("Production")
    });
    shot(&mut h, "20-tree-renamed");

    // The service has it too, not just the tree.
    let client = Client::new(&Target {
        url,
        token: e.server.bootstrap_token.clone().unwrap(),
        trust: Trust::Pinned(e.server.fingerprint.clone()),
        identity: None,
    })
    .unwrap();
    let saved = e.rt.block_on(client.connections()).unwrap();
    assert_eq!(saved[0].name, "people");
    assert_eq!(saved[0].folder.as_deref(), Some("Production"));
}

/// Pinning tabs: a pinned tab moves first, loses its ×, ignores middle
/// clicks, can sit on its own row, and stays pinned after a restart.
#[test]
fn pinned_tabs() {
    let _e = env();
    let titles = |h: &Harness<'_, SqailApp>| -> Vec<(String, bool)> {
        h.state()
            .tabs
            .iter()
            .map(|t| (t.title.clone(), t.pinned))
            .collect()
    };
    {
        let mut h = app([1280.0, 800.0]);
        let settle = |h: &mut Harness<'_, SqailApp>| h.run_steps(4);
        h.state_mut().new_tab(None);
        h.state_mut().new_tab(None);
        settle(&mut h);
        assert_eq!(h.state().active, 2);

        h.get_by_label("Query 3").click_secondary();
        settle(&mut h);
        h.get_by_label("Pin tab").click();
        settle(&mut h);
        assert_eq!(
            titles(&h),
            [
                ("Query 3".into(), true),
                ("Query 1".into(), false),
                ("Query 2".into(), false)
            ]
        );
        assert_eq!(h.state().active, 0, "the active tab moved with it");
        assert_eq!(h.get_all_by_label("×").count(), 2, "no × on the pinned tab");
        h.get_by_label("Query 3")
            .click_button(egui::PointerButton::Middle);
        settle(&mut h);
        assert_eq!(h.state().tabs.len(), 3, "middle click spares pinned tabs");

        // By default pinned tabs have their own row, above the others.
        assert!(h.state().settings.pinned_tabs_row);
        let (pin, other) = (
            h.get_by_label("Query 3").rect(),
            h.get_by_label("Query 1").rect(),
        );
        assert!(pin.bottom() <= other.top(), "{pin:?} above {other:?}");
        h.get_by_label("Pinned");
        shot(&mut h, "21-pinned-row");
        // One row: the pinned tab sits left of the others.
        h.get_by_label("Query 1").click_secondary();
        settle(&mut h);
        h.get_by_label("Pinned tabs on a separate row").click();
        settle(&mut h);
        assert!(!h.state().settings.pinned_tabs_row);
        let (pin, other) = (
            h.get_by_label("Query 3").rect(),
            h.get_by_label("Query 1").rect(),
        );
        assert!(pin.center().y == other.center().y && pin.left() < other.left());

        // Pin a second one: it goes after the first pinned tab.
        h.state_mut().set_pinned(2, true);
        settle(&mut h);
        assert_eq!(
            titles(&h),
            [
                ("Query 3".into(), true),
                ("Query 2".into(), true),
                ("Query 1".into(), false)
            ]
        );
        h.state().workspace().save();
    }
    let h = app([1280.0, 800.0]);
    assert_eq!(
        titles(&h),
        [
            ("Query 3".into(), true),
            ("Query 2".into(), true),
            ("Query 1".into(), false)
        ]
    );
}

/// Press and release on separate frames, like a real mouse.
fn real_click(h: &mut Harness<'_, SqailApp>, label: &str, button: egui::PointerButton) {
    let pos = h.get_by_label(label).rect().center();
    h.hover_at(pos);
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
        h.step();
    }
    h.run_steps(3);
}

#[test]
fn pin_with_real_mouse_clicks() {
    let _e = env();
    let mut h = app([1280.0, 800.0]);
    h.state_mut().new_tab(None);
    h.run_steps(4);
    real_click(&mut h, "Query 2", egui::PointerButton::Secondary);
    shot(&mut h, "22-pin-menu");
    real_click(&mut h, "Pin tab", egui::PointerButton::Primary);
    assert!(
        h.state().tabs[0].pinned,
        "{:?}",
        h.state()
            .tabs
            .iter()
            .map(|t| (&t.title, t.pinned))
            .collect::<Vec<_>>()
    );
}

/// With no connections at all, the tree's toolbar still works.
#[test]
fn toolbar_works_on_an_empty_tree() {
    let e = env();
    let client = Client::new(&Target {
        url: format!("https://{}", e.server.addr),
        token: e.server.bootstrap_token.clone().unwrap(),
        trust: Trust::Pinned(e.server.fingerprint.clone()),
        identity: None,
    })
    .unwrap();
    e.rt.block_on(async {
        for c in client.connections().await.unwrap() {
            client.delete_connection(c.id).await.unwrap();
        }
    });
    let mut h = Harness::builder()
        .with_size([1280.0, 800.0])
        .build_eframe(|cc| SqailApp::new(cc));
    wait(&mut h, "service connection", |a| {
        a.service.status == ServiceStatus::Connected
    });
    h.run_steps(4);
    h.get_by_label("+ Folder").click();
    h.run_steps(4);
    assert!(
        h.state().schema.rename.is_some(),
        "the new folder is being named"
    );
    h.key_press(Key::Enter);
    h.run_steps(4);
    h.get_by_label("New folder");
    h.get_by_label("+ Connection").click();
    h.run_steps(4);
    assert!(matches!(
        h.state().dialog,
        sqail_ui::dialogs::Dialog::Connection(_)
    ));
}

/// The settings window: pick a theme, turn off the close prompt, and both
/// take effect and are saved.
#[test]
fn settings_window_changes_theme_and_close_prompt() {
    use sqail_ui::settings::ThemePref;

    let e = env();
    let mut h = app([1280.0, 800.0]);
    let settle = |h: &mut Harness<'_, SqailApp>| h.run_steps(4);
    h.get_by_label("File").click();
    settle(&mut h);
    h.get_by_label_contains("Settings…").click();
    settle(&mut h);
    h.get_by_label("Settings");

    h.get_by_label("Theme").click();
    settle(&mut h);
    h.get_by_label("Nord").click();
    settle(&mut h);
    assert_eq!(h.state().settings.theme, ThemePref::Nord);
    assert_eq!(h.ctx.theme(), egui::Theme::Dark);
    shot(&mut h, "16-settings-nord");

    h.get_by_label("Tabs").click();
    settle(&mut h);
    h.get_by_label("Ask before closing a tab with unsaved changes")
        .click();
    settle(&mut h);
    assert!(!h.state().settings.confirm_close_tab);
    h.get_by_label("Close").click();
    settle(&mut h);
    assert!(matches!(h.state().dialog, sqail_ui::dialogs::Dialog::None));

    h.state_mut().tabs[0].text =
        "-- Nord\nSELECT id, upper(name) AS name\nFROM people\nWHERE score > 9.5 AND name <> 'Ada';"
            .into();
    h.run_steps(4);
    shot(&mut h, "17-nord-editor");

    let saved = std::fs::read_to_string(e.ui_dir.join("settings.toml")).unwrap();
    assert!(saved.contains("theme = \"nord\""), "{saved}");
    assert!(saved.contains("confirm_close_tab = false"), "{saved}");

    // A tab with unsaved changes now closes without asking.
    let idx = h.state_mut().new_tab(None);
    h.state_mut().tabs[idx].text = "SELECT 'draft'".into();
    h.run_steps(2);
    h.get_all_by_label("×").nth(1).expect("draft ×").click();
    settle(&mut h);
    assert_eq!(h.state().tabs.len(), 1);
    assert!(matches!(h.state().dialog, sqail_ui::dialogs::Dialog::None));
}

/// Find bar: Enter / Shift+Enter step through matches and keep the focus in
/// the field (so the text is never replaced), F3 / Shift+F3 work from the
/// editor, and Esc in the editor doesn't close the bar (#66).
#[test]
fn find_bar_steps_without_editing() {
    let e = env();
    let mut h = app([1280.0, 800.0]);
    let text = "SELECT name FROM people WHERE name = 'x' OR NAME IS NULL";
    h.state_mut().tabs[0].text = text.into();
    h.step();
    let selected = |h: &Harness<'_, SqailApp>| {
        let c = h.state().tabs[0].cursor;
        let (a, b) = (c.primary.index.0, c.secondary.index.0);
        (a.min(b), a.max(b))
    };
    let tab_id = h.state().tabs[0].id;
    let field_focused = |h: &Harness<'_, SqailApp>| {
        h.ctx
            .memory(|m| m.has_focus(sqail_ui::editor::find_field_id(tab_id)))
    };

    h.key_press_modifiers(Modifiers::COMMAND, Key::F);
    h.run_steps(2);
    assert!(field_focused(&h), "Ctrl+F focuses the find field");
    h.event(egui::Event::Text("name".into()));
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert_eq!(selected(&h), (7, 11));
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert_eq!(selected(&h), (30, 34), "second Enter finds the next match");
    assert_eq!(h.state().tabs[0].text, text, "Enter never edits the text");
    assert!(field_focused(&h), "focus stays in the find field");
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert_eq!(selected(&h), (44, 48), "case-insensitive by default");
    h.key_press_modifiers(Modifiers::SHIFT, Key::Enter);
    h.run_steps(2);
    assert_eq!(selected(&h), (30, 34), "Shift+Enter goes back");
    shot(&mut h, "14-find");

    // Esc in the field closes the bar and returns to the editor.
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(!h.state().tabs[0].find.open);
    // F3 / Shift+F3 from the editor reopen it and step.
    h.key_press(Key::F3);
    h.run_steps(3);
    assert!(h.state().tabs[0].find.open);
    assert_eq!(selected(&h), (44, 48));
    h.key_press(Key::F3);
    h.run_steps(3);
    assert_eq!(selected(&h), (7, 11), "wraps around");
    h.key_press_modifiers(Modifiers::SHIFT, Key::F3);
    h.run_steps(3);
    assert_eq!(selected(&h), (44, 48));
    assert!(!field_focused(&h));
    // Esc in the editor (e.g. to dismiss a popup) leaves the bar open.
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(h.state().tabs[0].find.open);
    assert_eq!(h.state().tabs[0].text, text);
    drop(h);
    e.server.shutdown();
}

#[test]
fn narrow_window_keeps_the_editor_out_of_the_side_panels() {
    let e = env();
    let mut h = app([900.0, 800.0]);
    h.state_mut().settings.assistant.open = true;
    h.state_mut().tabs[0].text = "SELECT id, name, score FROM people".into();
    h.run_steps(20);
    run_with(&mut h, "narrow run", |h| {
        h.key_press_modifiers(Modifiers::COMMAND, Key::Enter)
    });
    shot(&mut h, "14-narrow");
    let assistant = h.get_by_label("Assistant").rect().left();
    for label in ["▶ Run", "▶▶ Script", "Auto-commit", "Messages ("] {
        let r = h.get_by_label_contains(label).rect();
        assert!(
            r.right() <= assistant,
            "{label} {r:?} runs into the assistant at {assistant}"
        );
    }
    // Export sits at the right edge of the results pane. The assistant heading
    // is 8 px inside its panel, and the central column keeps an 8 px margin
    // before that panel unless something widened it.
    let export = h.get_by_label("Export").rect();
    assert!(
        export.right() <= assistant - 16.0,
        "the central column grew past its panel: Export {export:?}, assistant at {assistant}"
    );
    let messages = h.get_by_label_contains("Messages (").rect();
    // The run status is dropped when there's no room, never drawn over the tabs.
    if let Some(status) = h.query_by_label_contains("rows ·") {
        let status = status.rect();
        assert!(
            status.left() >= messages.right(),
            "run status {status:?} is drawn over the result tabs {messages:?}"
        );
    }

    // --- the mouse wheel zooms the editor text ----------------------------
    let editor = egui::pos2(430.0, 300.0);
    let size = h.state().settings.editor_font_size;
    let wheel = |h: &mut Harness<'_, SqailApp>, lines: f32, modifiers: Modifiers| {
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, lines),
            modifiers,
            phase: egui::TouchPhase::Move,
        });
        h.run_steps(3);
    };
    h.hover_at(editor);
    wheel(&mut h, 1.0, Modifiers::COMMAND);
    assert_eq!(
        h.state().settings.editor_font_size,
        size + 1.0,
        "Ctrl+wheel up"
    );
    wheel(&mut h, -1.0, Modifiers::NONE);
    assert_eq!(
        h.state().settings.editor_font_size,
        size + 1.0,
        "a plain wheel scrolls"
    );
    let middle = |h: &mut Harness<'_, SqailApp>, pressed: bool| {
        h.event(egui::Event::PointerButton {
            pos: editor,
            button: egui::PointerButton::Middle,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    };
    middle(&mut h, true);
    wheel(&mut h, -2.0, Modifiers::NONE);
    middle(&mut h, false);
    assert_eq!(
        h.state().settings.editor_font_size,
        size - 1.0,
        "middle button + wheel down"
    );
    drop(h);
    e.server.shutdown();
}

#[test]
fn editor_runs_queries_end_to_end() {
    let e = env();
    let dir = &e.dir;
    let server = &e.server;
    let mut h = app([1280.0, 800.0]);
    h.get_by_label("people db");
    assert!(
        h.state().tabs[0].connection.is_some(),
        "first tab picks the only connection"
    );

    // --- Ctrl+Enter runs the statement under the cursor ----------------
    h.state_mut().tabs[0].text =
        "SELECT id, name, score FROM people ORDER BY id;\n\nSELECT count(*) AS n FROM people"
            .into();
    h.step();
    run_with(&mut h, "first run", |h| {
        h.key_press_modifiers(Modifiers::COMMAND, Key::Enter)
    });
    {
        let run = h.state().tabs[0].run.as_ref().unwrap();
        assert!(
            !run.failed,
            "{:?}",
            run.messages.iter().map(|m| &m.text).collect::<Vec<_>>()
        );
        assert_eq!(run.results.len(), 1, "only the statement at the cursor ran");
        assert_eq!(run.results[0].len(), 3);
        assert_eq!(run.pane, Pane::Result(0));
    }
    h.get_by_label("Linus");
    h.get_by_label("Result 1 (3)");
    shot(&mut h, "01-results");

    // --- sorting by the column's arrow (the name selects the column) ----
    h.get_all_by_label("▲▼").nth(2).expect("score sort").click();
    h.step();
    h.step();
    {
        let rs = &h.state().tabs[0].run.as_ref().unwrap().results[0];
        assert_eq!(rs.sort, Some((2, true)));
        assert!(rs.row(0)[2].is_null(), "NULL sorts first");
    }

    // --- F5 runs the whole script: two result sets ----------------------
    run_with(&mut h, "script run", |h| h.key_press(Key::F5));
    assert_eq!(h.state().tabs[0].run.as_ref().unwrap().results.len(), 2);
    h.get_by_label("Result 2 (1)").click();
    h.step();
    h.get_by_label("3");

    // --- errors land in the Messages pane --------------------------------
    h.state_mut().tabs[0].text = "SELECT * FROM missing_table".into();
    h.step();
    run_with(&mut h, "failing run", |h| {
        h.key_press_modifiers(Modifiers::COMMAND, Key::Enter)
    });
    {
        let run = h.state().tabs[0].run.as_ref().unwrap();
        assert!(run.failed);
        assert_eq!(run.pane, Pane::Messages);
    }
    h.get_by_label_contains("no such table");
    shot(&mut h, "02-error");

    // --- side panels collapse to rails; wide results scroll sideways -------
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::B);
    h.run_steps(20);
    assert!(!h.state().settings.sidebar_open);
    let cols: Vec<String> = (1..=30)
        .map(|i| format!("'value {i}' AS column_{i}"))
        .collect();
    h.state_mut().tabs[0].text = format!("SELECT {}", cols.join(", "));
    h.step();
    run_with(&mut h, "wide run", |h| {
        h.key_press_modifiers(Modifiers::COMMAND, Key::Enter)
    });
    let first = h.get_by_label("column_1").rect();
    h.hover_at(first.center());
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(-3000.0, 0.0),
        modifiers: Modifiers::NONE,
        phase: egui::TouchPhase::Move,
    });
    h.run_steps(3);
    let last = h.get_by_label("column_30").rect();
    assert!(
        last.right() <= 1280.0,
        "the last column scrolled into view: {last:?}"
    );
    shot(&mut h, "02b-wide-results");
    h.get_by_label("»").click();
    h.run_steps(20);
    assert!(h.state().settings.sidebar_open);

    // --- Esc cancels a long query -----------------------------------------
    h.state_mut().tabs[0].text =
        "WITH RECURSIVE r(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM r) SELECT count(*) FROM r"
            .into();
    h.step();
    let before = run_id(&h);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    wait(&mut h, "query to start", |a| {
        a.tabs[0]
            .run
            .as_ref()
            .is_some_and(|r| r.id != before && r.running && r.query_id.is_some())
    });
    std::thread::sleep(Duration::from_millis(200));
    h.key_press(Key::Escape);
    wait(&mut h, "cancel", |a| {
        a.tabs[0].run.as_ref().is_some_and(|r| !r.running)
    });
    assert!(h.state().tabs[0].run.as_ref().unwrap().cancelled);

    // --- transactions span runs (the tab owns a session) ------------------
    h.state_mut().tabs[0].text = "BEGIN; INSERT INTO people (name) VALUES ('Tx')".into();
    h.step();
    run_with(&mut h, "begin", |h| h.key_press(Key::F5));
    assert_eq!(
        h.state().tabs[0].run.as_ref().unwrap().in_transaction,
        Some(true)
    );
    h.get_by_label("Transaction open");
    run_with(&mut h, "rollback", |h| h.get_by_label("Rollback").click());
    assert_eq!(
        h.state().tabs[0].run.as_ref().unwrap().in_transaction,
        Some(false)
    );

    // --- autocomplete: tables after FROM, columns after `alias.` ------------
    h.state_mut().tabs[0].text = String::new();
    h.run_steps(2);
    let editor = h.get_by_role(egui::accesskit::Role::MultilineTextInput);
    editor.click();
    h.run_steps(2);
    h.get_by_role(egui::accesskit::Role::MultilineTextInput)
        .type_text("SELECT * FROM pe");
    wait(&mut h, "table completions", |a| {
        a.tabs[0]
            .completion
            .as_ref()
            .is_some_and(|p| p.items.first().is_some_and(|i| i.label == "people"))
    });
    shot(&mut h, "05-autocomplete");
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state().tabs[0].text, "SELECT * FROM people");
    h.get_by_role(egui::accesskit::Role::MultilineTextInput)
        .type_text(" p WHERE p.");
    wait(&mut h, "column completions", |a| {
        a.tabs[0]
            .completion
            .as_ref()
            .is_some_and(|p| p.items.iter().any(|i| i.label == "score"))
    });
    h.key_press(Key::ArrowDown);
    h.key_press(Key::ArrowDown);
    h.step();
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert_eq!(
        h.state().tabs[0].text,
        "SELECT * FROM people p WHERE p.score"
    );
    assert!(h.state().tabs[0].completion.is_none());

    // A space ends the word: the popup closes and Enter is a newline (#65).
    h.get_by_role(egui::accesskit::Role::MultilineTextInput)
        .type_text(" > 1 AND sc");
    wait(&mut h, "completions for sc", |a| {
        a.tabs[0]
            .completion
            .as_ref()
            .is_some_and(|p| p.items.iter().any(|i| i.label == "score"))
    });
    h.get_by_role(egui::accesskit::Role::MultilineTextInput)
        .type_text(" ");
    h.run_steps(2);
    assert!(
        h.state().tabs[0].completion.is_none(),
        "space closes the popup"
    );
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert!(
        h.state().tabs[0]
            .text
            .starts_with("SELECT * FROM people p WHERE p.score > 1 AND sc \n"),
        "{:?}",
        h.state().tabs[0].text
    );
    // Same after a comma: no popup of every column.
    h.get_by_role(egui::accesskit::Role::MultilineTextInput)
        .type_text("na");
    wait(&mut h, "completions for na", |a| {
        a.tabs[0].completion.is_some()
    });
    h.get_by_role(egui::accesskit::Role::MultilineTextInput)
        .type_text(", ");
    h.run_steps(2);
    assert!(h.state().tabs[0].completion.is_none(), "comma closes it");
    h.key_press(Key::Tab);
    h.run_steps(2);
    assert!(
        h.state().tabs[0].text.trim_end().ends_with("sc \nna,"),
        "Tab must not insert a column: {:?}",
        h.state().tabs[0].text
    );

    // --- schema tree: expanding the connection lists its tables -----------
    h.get_by_label("people db").click();
    for _ in 0..200 {
        h.step();
        std::thread::sleep(Duration::from_millis(15));
        if h.query_by_label("⊞ people").is_some() {
            break;
        }
    }
    h.get_by_label("⊞ people");
    shot(&mut h, "03-schema");

    // --- create a connection through the form -----------------------------
    // Modals position themselves over a few frames; settle before clicking.
    let settle = |h: &mut Harness<'_, SqailApp>| h.run_steps(4);
    h.get_by_label("+ Connection").click();
    settle(&mut h);
    h.get_by_label("SQLite").click();
    settle(&mut h);
    h.get_by_label("Name").click();
    settle(&mut h);
    h.get_by_label("Name").type_text("scratch");
    settle(&mut h);
    h.get_by_label("Database file").click();
    settle(&mut h);
    let scratch = dir.path().join("scratch.db");
    h.get_by_label("Database file")
        .type_text(&scratch.to_string_lossy());
    settle(&mut h);
    h.get_by_label("Create the file if it does not exist")
        .click();
    settle(&mut h);
    h.get_by_label("Test").click();
    for _ in 0..200 {
        h.step();
        std::thread::sleep(Duration::from_millis(10));
        if h.query_by_label_contains("Connected in").is_some() {
            break;
        }
    }
    h.get_by_label_contains("Connected in");
    shot(&mut h, "04-connection-form");
    settle(&mut h);
    h.get_by_label("Save").click();
    wait(&mut h, "saved connection", |a| {
        a.service.connections.len() == 2
    });
    assert!(
        h.state()
            .service
            .connections
            .iter()
            .any(|c| c.name == "scratch")
    );

    // --- duplicate it from the context menu, then double-click to edit ----
    settle(&mut h);
    h.get_by_label("scratch").click_secondary();
    settle(&mut h);
    h.get_by_label("Duplicate…").click();
    settle(&mut h);
    h.get_by_label("Duplicate connection");
    h.get_by_label("Save").click();
    wait(&mut h, "duplicated connection", |a| {
        a.service.connections.len() == 3
    });
    let copy = h
        .state()
        .service
        .connections
        .iter()
        .find(|c| c.name == "scratch (copy)")
        .expect("copy saved")
        .clone();
    match &copy.params {
        ConnectionParams::Sqlite(p) => assert_eq!(p.path, scratch.to_string_lossy()),
        other => panic!("copied as {other:?}"),
    }
    settle(&mut h);
    // The harness spreads the two clicks over a few 0.25 s frames: allow a
    // slower double-click than egui's default 0.3 s, and let enough time
    // pass since the last click that this is not a triple click.
    h.ctx
        .options_mut(|o| o.input_options.max_double_click_delay = 1.0);
    h.run_steps(10);
    let node = h.get_by_label("scratch (copy)");
    node.click();
    node.click();
    settle(&mut h);
    h.get_by_label("Edit connection");
    h.get_by_label("Cancel").click();
    settle(&mut h);
    assert!(matches!(h.state().dialog, sqail_ui::dialogs::Dialog::None));

    // --- CSV import through the dialog -------------------------------------
    let csv = dir.path().join("more.csv");
    std::fs::write(&csv, "Name;Score\nKen;7.5\nBarbara;\n").unwrap();
    let conn = h.state().tabs[0].connection.unwrap();
    h.state_mut()
        .open_import(conn, "main".into(), "people".into(), csv);
    wait(&mut h, "import columns", |a| {
        matches!(&a.dialog, sqail_ui::dialogs::Dialog::Import(_))
    });
    for _ in 0..100 {
        h.step();
        std::thread::sleep(Duration::from_millis(10));
        // The mapping grid appears once the table's columns are loaded.
        if h.query_by_label("First value").is_some() {
            break;
        }
    }
    h.run_steps(4);
    shot(&mut h, "07-import");
    h.get_by_label("Import").click();
    for _ in 0..300 {
        h.step();
        std::thread::sleep(Duration::from_millis(10));
        // Shown in the dialog and in the status bar.
        if h.query_all_by_label("Imported 2 rows.").next().is_some() {
            break;
        }
    }
    assert!(h.query_all_by_label("Imported 2 rows.").next().is_some());
    h.get_by_label("Close").click();
    h.run_steps(2);
    h.state_mut().tabs[0].text = "SELECT count(*) AS n, count(score) AS scored FROM people".into();
    h.step();
    run_with(&mut h, "count after import", |h| h.key_press(Key::F5));
    assert_eq!(
        h.state().tabs[0].run.as_ref().unwrap().results[0].row(0),
        &[
            sqail_ui::results::Cell::Int(5),
            sqail_ui::results::Cell::Int(3)
        ]
    );

    // --- inline data editing ---------------------------------------------
    h.state_mut().tabs[0].text = "SELECT id, name, score FROM people ORDER BY id".into();
    h.step();
    run_with(&mut h, "select for editing", |h| h.key_press(Key::F5));
    h.get_by_label("✎ Edit data").click();
    wait(&mut h, "edit mode", |a| {
        a.tabs[0].run.as_ref().is_some_and(|r| r.edit.is_some())
    });
    h.get_by_label("Linus").click();
    h.step();
    h.key_press(Key::F2);
    h.run_steps(2);
    assert!(
        h.state().tabs[0]
            .run
            .as_ref()
            .unwrap()
            .edit
            .as_ref()
            .unwrap()
            .1
            .editing
            .is_some()
    );
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    // The inline editor has keyboard focus.
    h.event(egui::Event::Text("Linus T.".into()));
    h.step();
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert_eq!(
        h.state().tabs[0]
            .run
            .as_ref()
            .unwrap()
            .edit
            .as_ref()
            .unwrap()
            .1
            .pending(),
        1
    );
    shot(&mut h, "08-editing");
    h.get_by_label("Review & apply (1)").click();
    h.run_steps(4);
    match &h.state().dialog {
        sqail_ui::dialogs::Dialog::ApplyEdits(d) => {
            assert_eq!(
                d.previews,
                ["UPDATE people SET name = 'Linus T.' WHERE id = '2'"]
            )
        }
        _ => panic!("review dialog not open"),
    }
    shot(&mut h, "09-apply-edits");
    let before = run_id(&h);
    h.get_by_label("Apply").click();
    wait(&mut h, "re-run after apply", |a| {
        a.tabs[0]
            .run
            .as_ref()
            .is_some_and(|r| r.id != before && !r.running)
    });
    assert_eq!(
        h.state().tabs[0].run.as_ref().unwrap().results[0].row(1)[1],
        sqail_ui::results::Cell::Text("Linus T.".into())
    );

    // --- formatter ---------------------------------------------------------
    h.state_mut().tabs[0].text = "select id,name from people where score>1".into();
    h.step();
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::F);
    h.step();
    assert_eq!(
        h.state().tabs[0].text,
        "SELECT\n  id,\n  name\nFROM\n  people\nWHERE\n  score > 1"
    );

    // --- query plan ----------------------------------------------------------
    h.state_mut().tabs[0].text = "SELECT * FROM people p WHERE p.id = 1".into();
    h.step();
    run_with(&mut h, "explain", |h| {
        h.key_press_modifiers(Modifiers::COMMAND, Key::E)
    });
    {
        let run = h.state().tabs[0].run.as_ref().unwrap();
        assert!(
            !run.failed,
            "{:?}",
            run.messages.iter().map(|m| &m.text).collect::<Vec<_>>()
        );
        assert_eq!(run.pane, sqail_ui::results::Pane::Plan);
        assert!(run.plan.as_ref().is_some_and(|p| !p.roots.is_empty()));
    }
    h.get_by_label_contains("SEARCH p USING INTEGER PRIMARY KEY");
    shot(&mut h, "10-plan");

    // --- manual commit: a transaction stays open until rolled back ---------
    h.get_by_label("Auto-commit").click();
    h.step();
    assert!(!h.state().tabs[0].autocommit);
    h.state_mut().tabs[0].text = "UPDATE people SET score = 0 WHERE id = 1".into();
    h.step();
    run_with(&mut h, "manual-commit update", |h| h.key_press(Key::F5));
    assert!(h.state().tabs[0].in_transaction, "BEGIN was added");
    h.get_by_label("Transaction open");
    // Closing the tab now asks first.
    h.key_press_modifiers(Modifiers::COMMAND, Key::W);
    h.run_steps(3);
    assert!(matches!(
        h.state().dialog,
        sqail_ui::dialogs::Dialog::ConfirmCloseTransaction(Some(id)) if id == h.state().tabs[0].id
    ));
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    run_with(&mut h, "rollback", |h| h.get_by_label("Rollback").click());
    assert!(!h.state().tabs[0].in_transaction);
    h.state_mut().tabs[0].text = "SELECT score FROM people WHERE id = 1".into();
    h.step();
    run_with(&mut h, "score unchanged", |h| h.key_press(Key::F5));
    assert_eq!(
        h.state().tabs[0].run.as_ref().unwrap().results[0].row(0)[0],
        sqail_ui::results::Cell::Float(9.5)
    );
    // The SELECT opened a transaction, but the run, the history and result
    // editing all see only the user's SQL (#79).
    assert!(h.state().tabs[0].in_transaction, "BEGIN was sent first");
    assert_eq!(
        h.state().tabs[0].run.as_ref().unwrap().sql,
        "SELECT score FROM people WHERE id = 1"
    );
    assert_eq!(
        h.state().history.entries.last().map(|e| e.sql.as_str()),
        Some("SELECT score FROM people WHERE id = 1")
    );
    assert_eq!(h.state().editability(0), Ok(()));
    assert!(
        !h.get_by_label("✎ Edit data").accesskit_node().is_disabled(),
        "Edit is offered in a manual-commit tab"
    );
    run_with(&mut h, "rollback the select's transaction", |h| {
        h.get_by_label("Rollback").click()
    });
    assert!(!h.state().tabs[0].in_transaction);
    h.get_by_label("Auto-commit").click();
    h.step();

    // --- command palette: “new tab” -----------------------------------------
    let tabs = h.state().tabs.len();
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    h.run_steps(3);
    assert!(h.state().palette.is_some());
    h.event(egui::Event::Text("new tab".into()));
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert!(h.state().palette.is_none());
    assert_eq!(h.state().tabs.len(), tabs + 1);
    assert_eq!(
        h.state().tabs[tabs].connection,
        h.state().tabs[0].connection,
        "new tabs keep the connection"
    );

    // --- quick open: a table by name ----------------------------------------
    h.key_press_modifiers(Modifiers::COMMAND, Key::P);
    h.run_steps(3);
    h.event(egui::Event::Text("peop".into()));
    h.step();
    assert_eq!(
        h.state().palette.as_ref().map(|p| p.query.as_str()),
        Some("peop")
    );
    for _ in 0..100 {
        h.step();
        std::thread::sleep(Duration::from_millis(10));
        if h.query_by_label("main.people").is_some() {
            break;
        }
    }
    shot(&mut h, "11-quick-open");
    let before = h.state().tabs.len();
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state().tabs.len(), before + 1);
    wait(&mut h, "quick-open query", |a| {
        a.tabs[a.active].run.as_ref().is_some_and(|r| !r.running)
    });
    assert_eq!(
        h.state().tabs[h.state().active]
            .run
            .as_ref()
            .unwrap()
            .results[0]
            .len(),
        5
    );

    drop(h);
    server.shutdown();
}

/// A client of the test service, for looking at the database from outside.
fn outside(server: &sqail_service::Server) -> Client {
    Client::new(&Target {
        url: format!("https://{}", server.addr),
        token: server.bootstrap_token.clone().unwrap(),
        trust: Trust::Pinned(server.fingerprint.clone()),
        identity: None,
    })
    .unwrap()
}

/// `people.score` by id, read on a fresh connection (committed data only).
fn scores(e: &Env, client: &Client, conn: uuid::Uuid) -> Vec<serde_json::Value> {
    let events =
        e.rt.block_on(client.query_all(
            On::Connection(conn),
            &QueryRequest::new("SELECT score FROM people ORDER BY id"),
        ))
        .unwrap();
    events
        .into_iter()
        .filter_map(|ev| match ev {
            sqail_client::proto::QueryEvent::Rows { rows, .. } => Some(rows),
            _ => None,
        })
        .flatten()
        .map(|mut r| r.remove(0))
        .collect()
}

/// #152: when the session of a manual-commit tab goes away mid-transaction,
/// the next statement must not silently run (and auto-commit) on a new one.
#[test]
fn lost_session_does_not_auto_commit() {
    let e = env();
    let client = outside(&e.server);
    let mut h = app([1280.0, 800.0]);
    let conn = h.state().tabs[0].connection.unwrap();
    h.state_mut().tabs[0].autocommit = false;
    h.state_mut().tabs[0].text = "UPDATE people SET score = 1 WHERE id = 1".into();
    h.step();
    run_with(&mut h, "update in a transaction", |h| h.key_press(Key::F5));
    assert!(h.state().tabs[0].in_transaction);
    let (_, sid) = h.state().tabs[0].session.unwrap();

    // The service drops the session (idle reaper, restart, lost connection).
    e.rt.block_on(client.close_session(sid)).unwrap();
    h.state_mut().tabs[0].text = "UPDATE people SET score = 2 WHERE id = 2".into();
    h.step();
    run_with(&mut h, "update after the session is gone", |h| {
        h.key_press(Key::F5)
    });
    {
        let tab = &h.state().tabs[0];
        let run = tab.run.as_ref().unwrap();
        assert!(run.failed);
        assert!(
            run.messages
                .iter()
                .any(|m| m.text.contains("open transaction was rolled back")),
            "{:?}",
            run.messages.iter().map(|m| &m.text).collect::<Vec<_>>()
        );
        assert!(!tab.in_transaction, "the transaction state is cleared");
    }
    h.get_by_label_contains("open transaction was rolled back");
    assert!(h.query_by_label("Transaction open").is_none());
    assert_eq!(
        scores(&e, &client, conn)[..2],
        [serde_json::json!(9.5), serde_json::json!(8.25)],
        "the first update was rolled back, the second was not run"
    );

    // Running it again starts a new transaction on a new session.
    run_with(&mut h, "deliberate re-run", |h| h.key_press(Key::F5));
    assert!(!h.state().tabs[0].run.as_ref().unwrap().failed);
    assert!(h.state().tabs[0].in_transaction, "BEGIN on the new session");
    assert_eq!(
        scores(&e, &client, conn)[1],
        serde_json::json!(8.25),
        "not committed"
    );

    // Outside a transaction the lost session is replaced silently, and a
    // manual-commit tab still gets its BEGIN on the new one.
    run_with(&mut h, "rollback", |h| h.get_by_label("Rollback").click());
    assert!(!h.state().tabs[0].in_transaction);
    let (_, sid) = h.state().tabs[0].session.unwrap();
    e.rt.block_on(client.close_session(sid)).unwrap();
    h.state_mut().tabs[0].text = "UPDATE people SET score = 3 WHERE id = 3".into();
    h.step();
    run_with(&mut h, "update on a replaced session", |h| {
        h.key_press(Key::F5)
    });
    assert!(!h.state().tabs[0].run.as_ref().unwrap().failed);
    assert!(h.state().tabs[0].in_transaction, "BEGIN was re-sent");
    assert_ne!(h.state().tabs[0].session.unwrap().1, sid);
    assert_eq!(
        scores(&e, &client, conn)[2],
        serde_json::Value::Null,
        "not committed"
    );

    drop(h);
    e.server.shutdown();
}

/// ✎ Edit data applies on the tab's own session (#162): a temp table there
/// shadows `people`, and that is the table edited. With Auto-commit off the
/// edits join the tab's transaction and stay uncommitted.
#[test]
fn grid_edits_apply_on_the_tabs_session() {
    use sqail_ui::dialogs::Dialog;
    use sqail_ui::editing::EditTx;
    use sqail_ui::results::Cell;

    let e = env();
    let mut h = app([1280.0, 800.0]);
    let conn = h.state().tabs[0].connection.expect("people db");
    let client = Client::new(&Target {
        url: format!("https://{}", e.server.addr),
        token: e.server.bootstrap_token.clone().unwrap(),
        trust: Trust::Pinned(e.server.fingerprint.clone()),
        identity: None,
    })
    .unwrap();
    let main_name = |id: i64| {
        let ev =
            e.rt.block_on(client.query_all(
                On::Connection(conn),
                &QueryRequest::new(format!("SELECT name FROM main.people WHERE id = {id}")),
            ))
            .unwrap();
        format!("{ev:?}")
    };
    let edit_first_name = |h: &mut Harness<'_, SqailApp>, value: &str| {
        h.get_by_label("✎ Edit data").click();
        wait(h, "edit mode", |a| {
            a.tabs[0].run.as_ref().is_some_and(|r| r.edit.is_some())
        });
        let run = h.state_mut().tabs[0].run.as_mut().unwrap();
        let (ri, ed) = run.edit.as_mut().unwrap();
        ed.set(0, 1, Some(value.into()), &run.results[*ri]);
        h.step();
        h.get_by_label("Review & apply (1)").click();
        h.run_steps(4);
    };
    let apply = |h: &mut Harness<'_, SqailApp>| {
        let before = run_id(h);
        h.get_by_label("Apply").click();
        wait(h, "re-run after apply", |a| {
            a.tabs[0]
                .run
                .as_ref()
                .is_some_and(|r| r.id != before && !r.running)
        });
    };
    let first_name = |h: &Harness<'_, SqailApp>| {
        h.state().tabs[0].run.as_ref().unwrap().results[0].row(0)[1].clone()
    };

    h.state_mut().tabs[0].text =
        "CREATE TEMP TABLE people (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
         INSERT INTO people VALUES (2, 'Shadow')"
            .into();
    h.step();
    run_with(&mut h, "temp table", |h| h.key_press(Key::F5));
    h.state_mut().tabs[0].text = "SELECT id, name FROM people".into();
    h.step();
    run_with(&mut h, "select the temp table", |h| h.key_press(Key::F5));
    assert_eq!(first_name(&h), Cell::Text("Shadow".into()));

    // Auto-commit: the temp table is edited and committed.
    edit_first_name(&mut h, "Edited");
    assert!(matches!(&h.state().dialog, Dialog::ApplyEdits(d) if d.tx == EditTx::Commit));
    apply(&mut h);
    assert_eq!(first_name(&h), Cell::Text("Edited".into()));
    assert!(!h.state().tabs[0].in_transaction);
    assert!(
        main_name(2).contains("Linus"),
        "main.people untouched: {}",
        main_name(2)
    );

    // Manual commit: the edits join the tab's transaction.
    h.get_by_label("Auto-commit").click();
    h.step();
    run_with(&mut h, "select in a transaction", |h| h.key_press(Key::F5));
    assert!(h.state().tabs[0].in_transaction);
    edit_first_name(&mut h, "Uncommitted");
    assert!(matches!(&h.state().dialog, Dialog::ApplyEdits(d) if d.tx == EditTx::Join));
    h.get_by_label_contains("not committed until you click Commit");
    apply(&mut h);
    assert_eq!(first_name(&h), Cell::Text("Uncommitted".into()));
    assert!(h.state().tabs[0].in_transaction, "still open");
    h.get_by_label("Transaction open");
    h.get_by_label_contains("not committed yet");
    run_with(&mut h, "rollback", |h| h.get_by_label("Rollback").click());
    assert!(!h.state().tabs[0].in_transaction);
    h.state_mut().tabs[0].autocommit = true;
    run_with(&mut h, "select after rollback", |h| h.key_press(Key::F5));
    assert_eq!(first_name(&h), Cell::Text("Edited".into()));
}

// ============================================================ big + engines ==

/// Frame time while scrolling a ~1M row result, a 5,000-line script in the
/// editor, and the `sales` schema on every engine. Needs `scripts/db.sh up`.
/// Run with `--release` for representative timings.
#[test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
fn large_results_and_all_engines() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut cfg = sqail_service::Config {
        data_dir: dir.path().join("service"),
        bind: "127.0.0.1:0".parse().unwrap(),
        docs_ui: false,
        ..Default::default()
    };
    cfg.sqlite.allowed_dirs = vec![root.join("dev/data")];
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
    let sqlite_path = root
        .join("dev/data/sqail_test.db")
        .canonicalize()
        .expect("scripts/db.sh up");
    let profiles = [
        (
            "pg",
            ConnectionParams::Postgres(sqail_client::proto::PostgresParams {
                host: "127.0.0.1".into(),
                port: 55432,
                database: "sqail_test".into(),
                user: "sqail".into(),
                ssl_mode: sqail_client::proto::PgSslMode::Disable,
                ssl_root_cert: None,
                ssl_client_cert: None,
            }),
            Some("sqail_dev_pw"),
        ),
        (
            "ms",
            ConnectionParams::Mssql(sqail_client::proto::MssqlParams {
                host: "127.0.0.1".into(),
                port: 51433,
                instance: None,
                database: Some("sqail_test".into()),
                auth: sqail_client::proto::MssqlAuth::Sql {
                    user: "sqail".into(),
                },
                encrypt: sqail_client::proto::MssqlEncrypt::Required,
                trust_server_certificate: true,
            }),
            Some("Sqail2_dev!Passw0rd"),
        ),
        (
            "lite",
            ConnectionParams::Sqlite(SqliteParams {
                path: sqlite_path.to_string_lossy().into(),
                create: false,
            }),
            None,
        ),
    ];
    rt.block_on(async {
        for (name, params, pw) in profiles {
            client
                .create_connection(&ConnectionInput {
                    name: name.into(),
                    params,
                    password: pw.map(String::from),
                    ssl_client_key: None,
                    read_only: false,
                    color: None,
                    environment: None,
                    folder: None,
                })
                .await
                .unwrap();
        }
    });

    sqail_ui::settings::override_config_dir(dir.path().join("ui"));
    sqail_ui::secrets::force_file_store();
    let mut settings = Settings {
        max_rows: 2_000_000,
        ..Default::default()
    };
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

    let mut h = Harness::builder()
        .with_size([1600.0, 1000.0])
        .build_eframe(|cc| SqailApp::new(cc));
    wait(&mut h, "connections", |a| a.service.connections.len() == 3);

    // --- ~1M rows from Postgres, then scroll -----------------------------
    let pg = h
        .state()
        .service
        .connections
        .iter()
        .find(|c| c.name == "pg")
        .unwrap()
        .id;
    h.state_mut().tabs[0].connection = Some(pg);
    h.state_mut().tabs[0].text =
        "SELECT oi.*, g FROM sales.order_items oi CROSS JOIN generate_series(1, 34) g".into();
    h.step();
    let started = Instant::now();
    let before = run_id(&h);
    h.key_press(Key::F5);
    let start = Instant::now();
    // Frame times while rows stream in (the UI must stay responsive).
    let (mut frames, mut frame_total, mut frame_worst) = (0u32, Duration::ZERO, Duration::ZERO);
    while !h.state().tabs[0]
        .run
        .as_ref()
        .is_some_and(|r| r.id != before && !r.running)
    {
        assert!(
            start.elapsed() < Duration::from_secs(180),
            "1M rows took too long"
        );
        let t = Instant::now();
        h.step();
        let dt = t.elapsed();
        frames += 1;
        frame_total += dt;
        frame_worst = frame_worst.max(dt);
    }
    eprintln!(
        "while streaming: {frames} frames, avg {:?}/frame, worst {:?}",
        frame_total / frames.max(1),
        frame_worst
    );
    let rows = h.state().tabs[0].run.as_ref().unwrap().total_rows();
    assert_eq!(rows, 29_850 * 34);
    eprintln!(
        "streamed {rows} rows into the grid in {:?}",
        started.elapsed()
    );

    let grid = h.get_by_label("Result 1 (1,014,900)").rect().center();
    let mut worst = Duration::ZERO;
    let mut total = Duration::ZERO;
    let frames = 60;
    for i in 0..frames {
        h.hover_at(egui::pos2(grid.x, grid.y + 200.0));
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, if i % 20 == 19 { 5000.0 } else { -40.0 }),
            modifiers: Modifiers::NONE,
            phase: egui::TouchPhase::Move,
        });
        let t = Instant::now();
        h.step();
        let dt = t.elapsed();
        worst = worst.max(dt);
        total += dt;
    }
    eprintln!(
        "scrolling 1M rows: avg {:?}/frame, worst {:?}",
        total / frames,
        worst
    );
    shot(&mut h, "10-million-rows");

    // --- a 5,000-line script: time to lay out an edited buffer -------------
    let line = "SELECT o.id, c.name, sum(i.quantity * i.unit_price) FROM sales.orders o JOIN sales.customers c ON c.id = o.customer_id -- note\n";
    h.state_mut().tabs[0].text = line.repeat(5000);
    h.step();
    let mut edit_total = Duration::ZERO;
    for i in 0..10 {
        h.state_mut().tabs[0].text.insert(i * 10, 'x');
        let t = Instant::now();
        h.step();
        edit_total += t.elapsed();
    }
    let mut idle_total = Duration::ZERO;
    for _ in 0..10 {
        let t = Instant::now();
        h.step();
        idle_total += t.elapsed();
    }
    eprintln!(
        "5,000-line editor: avg {:?} per edited frame, {:?} per unchanged frame (harness overhead incl. AccessKit)",
        edit_total / 10,
        idle_total / 10
    );

    // --- schema tree on every engine ---------------------------------------
    for name in ["pg", "ms", "lite"] {
        h.get_by_label(name).click();
        for _ in 0..300 {
            h.step();
            std::thread::sleep(Duration::from_millis(10));
            if h.query_by_label("sales").is_some() || h.query_by_label("⊞ orders").is_some() {
                break;
            }
        }
        if let Some(sales) = h.query_by_label("sales") {
            sales.click();
        }
        for _ in 0..300 {
            h.step();
            std::thread::sleep(Duration::from_millis(10));
            if h.query_by_label("⊞ orders").is_some() {
                break;
            }
        }
        h.get_by_label("⊞ orders");
        h.get_by_label("Views (1)").click();
        h.step();
        h.step();
        h.get_by_label("👁 order_totals");
        shot(&mut h, &format!("11-schema-{name}"));
        // collapse again so the next engine's labels are unambiguous
        h.get_by_label("Views (1)").click();
        if let Some(sales) = h.query_by_label("sales") {
            sales.click();
        }
        h.get_by_label(name).click();
        h.step();
    }
    drop(h);
    server.shutdown();
}

// ======================================================== session lifecycle ==

/// Wait until session `sid` is gone on the service (closes are detached).
fn wait_closed(e: &Env, client: &Client, sid: uuid::Uuid) {
    let start = Instant::now();
    while e.rt.block_on(client.session(sid)).is_ok() {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "session {sid} still open"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A manual-commit tab with `UPDATE people SET score = 5 WHERE id = 1` open.
fn open_transaction(h: &mut Harness<'_, SqailApp>) -> uuid::Uuid {
    h.state_mut().tabs[0].autocommit = false;
    h.state_mut().tabs[0].text = "UPDATE people SET score = 5 WHERE id = 1".into();
    h.step();
    run_with(h, "update in a transaction", |h| h.key_press(Key::F5));
    assert!(h.state().tabs[0].in_transaction);
    h.state().tabs[0].session.unwrap().1
}

/// #154: changing the connection of a tab with an open transaction asks,
/// rolls it back by closing the old session, and starts clean on the new one.
#[test]
fn switching_connection_closes_the_old_session() {
    let e = env();
    let client = outside(&e.server);
    let mut h = app([1280.0, 800.0]);
    let other = e.rt.block_on(async {
        let conn = client
            .create_connection(&ConnectionInput {
                name: "other db".into(),
                params: ConnectionParams::Sqlite(SqliteParams {
                    path: e.dir.path().join("other.db").to_string_lossy().into(),
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
                    "CREATE TABLE people (id INTEGER PRIMARY KEY, score REAL);
                     INSERT INTO people (score) VALUES (1);",
                ),
            )
            .await
            .unwrap();
        conn.id
    });
    h.state_mut().refresh_connections();
    wait(&mut h, "both connections", |a| {
        a.service.connections.len() == 2
    });
    let first = h.state().tabs[0].connection.unwrap();
    let sid = open_transaction(&mut h);

    // Asks first; Cancel keeps everything as it was.
    h.state_mut().set_connection(0, Some(other));
    h.run_steps(3);
    assert!(matches!(
        h.state().dialog,
        sqail_ui::dialogs::Dialog::ConfirmSwitchTransaction(id, Some(c))
            if id == h.state().tabs[0].id && c == other
    ));
    shot(&mut h, "12-switch-transaction");
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert_eq!(h.state().tabs[0].connection, Some(first));
    assert!(h.state().tabs[0].in_transaction);
    assert_eq!(h.state().tabs[0].session.map(|s| s.1), Some(sid));

    // Roll back and switch: the old session is closed, its work rolled back.
    h.state_mut().set_connection(0, Some(other));
    h.run_steps(3);
    h.get_by_label("Roll back and switch").click();
    h.run_steps(3);
    assert_eq!(h.state().tabs[0].connection, Some(other));
    assert!(!h.state().tabs[0].in_transaction);
    assert!(h.state().tabs[0].session.is_none());
    assert!(h.query_by_label("Transaction open").is_none());
    wait_closed(&e, &client, sid);
    assert_eq!(scores(&e, &client, first)[0], serde_json::json!(9.5));

    // The next run on the new connection starts its own transaction.
    h.state_mut().tabs[0].text = "UPDATE people SET score = 9 WHERE id = 1".into();
    h.step();
    run_with(&mut h, "update on the other connection", |h| {
        h.key_press(Key::F5)
    });
    assert!(!h.state().tabs[0].run.as_ref().unwrap().failed);
    assert!(h.state().tabs[0].in_transaction, "BEGIN was sent");
    assert_eq!(
        scores(&e, &client, other),
        [serde_json::json!(1.0)],
        "not committed"
    );

    // Without a transaction the switch is immediate and still closes it.
    run_with(&mut h, "rollback", |h| h.get_by_label("Rollback").click());
    let sid = h.state().tabs[0].session.unwrap().1;
    // (through the toolbar's connection picker this time)
    h.get_by_role(egui::accesskit::Role::ComboBox).click();
    h.run_steps(3);
    h.get_by_label_contains("people db  ·").click();
    h.run_steps(3);
    assert!(matches!(h.state().dialog, sqail_ui::dialogs::Dialog::None));
    assert_eq!(h.state().tabs[0].connection, Some(first));
    wait_closed(&e, &client, sid);

    // A deleted profile takes its sessions (and transaction) with it; the
    // tab starts clean on the next connection instead of reporting a lost
    // transaction.
    h.state_mut().set_connection(0, Some(other));
    h.state_mut().tabs[0].text = "UPDATE people SET score = 2 WHERE id = 1".into();
    h.step();
    run_with(&mut h, "update before the delete", |h| h.key_press(Key::F5));
    assert!(h.state().tabs[0].in_transaction);
    e.rt.block_on(client.delete_connection(other)).unwrap();
    h.state_mut().refresh_connections();
    wait(&mut h, "deleted profile", |a| {
        a.service.connections.len() == 1
    });
    // The only tab falls back to the remaining connection.
    assert_eq!(h.state().tabs[0].connection, Some(first));
    assert!(!h.state().tabs[0].in_transaction);
    assert!(h.state().tabs[0].session.is_none());
    h.state_mut().tabs[0].text = "SELECT score FROM people WHERE id = 1".into();
    h.step();
    run_with(&mut h, "run after the delete", |h| h.key_press(Key::F5));
    assert!(!h.state().tabs[0].run.as_ref().unwrap().failed);
    assert!(h.state().tabs[0].in_transaction, "a new transaction began");

    drop(h);
    e.server.shutdown();
}

/// #154: a 409 "session busy" fails the run but keeps the session (and its
/// transaction); runs wait while an export holds the session.
#[test]
fn busy_session_is_kept() {
    let e = env();
    let client = outside(&e.server);
    let mut h = app([1280.0, 800.0]);
    let conn = h.state().tabs[0].connection.unwrap();
    let sid = open_transaction(&mut h);

    // Something else (an export, here an outside client) holds the session.
    let mut stream =
        e.rt.block_on(client.query(
            On::Session(sid),
            &QueryRequest::new(
                "WITH RECURSIVE r(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM r) \
                 SELECT count(*) FROM r",
            ),
        ))
        .unwrap();
    h.state_mut().tabs[0].text = "UPDATE people SET score = 6 WHERE id = 2".into();
    h.step();
    run_with(&mut h, "run on a busy session", |h| h.key_press(Key::F5));
    {
        let tab = &h.state().tabs[0];
        let run = tab.run.as_ref().unwrap();
        assert!(run.failed);
        assert!(
            run.messages
                .iter()
                .any(|m| m.text.contains("running another")),
            "{:?}",
            run.messages.iter().map(|m| &m.text).collect::<Vec<_>>()
        );
        assert_eq!(tab.session.map(|s| s.1), Some(sid), "session kept");
        assert!(tab.in_transaction);
    }
    e.rt.block_on(async {
        use futures::StreamExt;
        client.cancel(stream.query_id).await.unwrap();
        while stream.events.next().await.is_some() {}
    });

    // Once free, the same session (and transaction) carries on.
    run_with(&mut h, "run after the session is free", |h| {
        h.key_press(Key::F5)
    });
    assert!(!h.state().tabs[0].run.as_ref().unwrap().failed);
    assert_eq!(h.state().tabs[0].session.map(|s| s.1), Some(sid));
    assert!(h.state().tabs[0].in_transaction);
    assert_eq!(
        scores(&e, &client, conn)[..2],
        [serde_json::json!(9.5), serde_json::json!(8.25)],
        "both updates still uncommitted"
    );

    // While an export uses the session, runs are refused up front.
    h.state_mut().tabs[0].session_busy = true;
    let before = run_id(&h);
    h.key_press(Key::F5);
    h.run_steps(3);
    assert_eq!(run_id(&h), before, "nothing ran");
    h.get_by_label_contains("Wait for the export to finish");

    drop(h);
    e.server.shutdown();
}

/// #154: quitting closes every session, so open transactions really roll
/// back instead of holding locks until the idle timeout.
#[test]
fn quitting_closes_all_sessions() {
    let e = env();
    let client = outside(&e.server);
    let mut h = app([1280.0, 800.0]);
    let conn = h.state().tabs[0].connection.unwrap();
    let sid = open_transaction(&mut h);
    let idx = h.state_mut().new_tab(Some(conn));
    h.state_mut().tabs[idx].text = "SELECT 1".into();
    h.step();
    run_with(&mut h, "second tab", |h| h.key_press(Key::F5));
    let sid2 = h.state().tabs[idx].session.unwrap().1;

    h.state_mut().close_sessions(Duration::from_secs(2));
    assert!(h.state().tabs.iter().all(|t| t.session.is_none()));
    for s in [sid, sid2] {
        assert!(e.rt.block_on(client.session(s)).is_err(), "closed at once");
    }
    // The rolled-back transaction no longer blocks writers.
    e.rt.block_on(client.query_all(
        On::Connection(conn),
        &QueryRequest::new("UPDATE people SET score = 7 WHERE id = 3"),
    ))
    .unwrap();
    assert_eq!(
        scores(&e, &client, conn),
        [
            serde_json::json!(9.5),
            serde_json::json!(8.25),
            serde_json::json!(7.0)
        ]
    );
    // The tab knows its transaction went with the session (#152's guard
    // must not report a lost transaction): the next run just starts over.
    assert!(h.state().tabs.iter().all(|t| !t.in_transaction));
    h.state_mut().active = 0;
    h.state_mut().tabs[0].text = "SELECT score FROM people WHERE id = 1".into();
    h.step();
    run_with(&mut h, "run after closing sessions", |h| {
        h.key_press(Key::F5)
    });
    assert!(!h.state().tabs[0].run.as_ref().unwrap().failed);
    assert!(h.state().tabs[0].in_transaction, "a new transaction began");

    drop(h);
    e.server.shutdown();
}
