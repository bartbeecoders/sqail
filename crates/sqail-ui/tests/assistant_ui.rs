//! The assistant panel in the real app, with a fake `claude` that replays a
//! recorded answer: sending, streaming into the conversation, tool calls, and
//! the Insert / New tab buttons on a proposed query. No AI involved.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use sqail_client::proto::{ConnectionInput, ConnectionParams, SqliteParams};
use sqail_client::{Client, Target, Trust};
use sqail_ui::SqailApp;
use sqail_ui::app::ServiceStatus;
use sqail_ui::assistant::Entry;
use sqail_ui::settings::{ServiceProfile, Settings};

const SQL: &str = "SELECT name, score FROM people ORDER BY score DESC";

/// A stand-in for `claude -p --output-format stream-json`: it records its
/// arguments and stdin, then prints a short conversation.
fn fake_claude(dir: &Path) -> PathBuf {
    let lines = [
        serde_json::json!({"type": "system", "subtype": "init", "session_id": "fake-session"}),
        serde_json::json!({"type": "stream_event", "event": {"type": "message_start", "message": {"id": "m1"}}}),
        serde_json::json!({"type": "stream_event", "event": {"type": "content_block_delta", "delta": {"type": "text_delta", "text": "Checking the scores.\n"}}}),
        serde_json::json!({"type": "assistant", "message": {"id": "m1", "content": [
            {"type": "text", "text": "Checking the scores.\n"},
            {"type": "tool_use", "id": "t1", "name": "mcp__sqail__run_query", "input": {"sql": "SELECT max(score) FROM people"}}]}}),
        serde_json::json!({"type": "user", "message": {"content": [
            {"type": "tool_result", "tool_use_id": "t1", "content": [{"type": "text", "text": "max(score)\n9.5\n(1 row)"}]}]}}),
        serde_json::json!({"type": "stream_event", "event": {"type": "message_start", "message": {"id": "m2"}}}),
        serde_json::json!({"type": "stream_event", "event": {"type": "content_block_delta", "delta": {"type": "text_delta",
            "text": format!("Top scorers first:\n```sql\n{SQL}\n```\n")}}}),
        serde_json::json!({"type": "result", "subtype": "success", "is_error": false, "result": "", "session_id": "fake-session", "total_cost_usd": 0.001}),
    ];
    let mut script = String::from("#!/bin/sh\n");
    script.push_str(&format!(
        "printf '%s\\n' \"$@\" > '{}'\n",
        dir.join("args.txt").display()
    ));
    script.push_str(&format!("cat > '{}'\n", dir.join("stdin.txt").display()));
    // printf, not echo: dash's echo (Ubuntu's /bin/sh) turns the `\n` in
    // JSON strings into real newlines.
    for l in lines {
        script.push_str(&format!(
            "printf '%s\\n' '{}'\n",
            l.to_string().replace('\'', "'\\''")
        ));
    }
    let path = dir.join("claude");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn step_until(h: &mut Harness<'_, SqailApp>, what: &str, done: impl Fn(&SqailApp) -> bool) {
    let start = Instant::now();
    while !done(h.state()) {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "timed out waiting for {what}; conversation: {:?}",
            h.state().assistant.entries
        );
        h.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    h.step();
}

#[test]
fn assistant_panel_streams_an_answer_and_inserts_sql() {
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
    rt.block_on(client.create_connection(&ConnectionInput {
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
    }))
    .unwrap();

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
    settings.assistant.open = true;
    settings.assistant.claude_path = Some(fake_claude(dir.path()));
    settings.save();
    sqail_ui::secrets::save_token(&url, &token).unwrap();

    let mut h = Harness::builder()
        .with_size([1400.0, 850.0])
        .build_eframe(|cc| SqailApp::new(cc));
    step_until(&mut h, "service", |a| {
        a.service.status == ServiceStatus::Connected && !a.service.connections.is_empty()
    });
    let conn = h.state().service.connections[0].id;
    h.state_mut().tabs[0].connection = Some(conn);
    h.state_mut().tabs[0].text = "SELECT * FROM people".into();
    h.step();

    h.state_mut().assistant.input = "Who scores best?".into();
    h.get_by_label("Send").click();
    step_until(&mut h, "the answer", |a| {
        !a.assistant.is_running() && a.assistant.entries.len() >= 4
    });

    let entries = h.state().assistant.entries.clone();
    assert_eq!(entries[0], Entry::User("Who scores best?".into()));
    assert_eq!(entries[1], Entry::Answer("Checking the scores.\n".into()));
    assert!(
        matches!(&entries[2], Entry::Tool { name, result: Some((r, false)), .. } if name == "run_query" && r.contains("9.5")),
        "{:?}",
        entries[2]
    );
    assert!(
        matches!(&entries[3], Entry::Answer(a) if a.contains(SQL)),
        "{:?}",
        entries[3]
    );
    // The session is kept for the next question.
    assert_eq!(
        h.state()
            .assistant
            .conversation
            .as_ref()
            .and_then(|c| c.session.as_deref()),
        Some("fake-session")
    );

    // What the CLI got: only sqail's tools, and the question plus the
    // editor's SQL on stdin; the token only in the environment.
    let args = std::fs::read_to_string(dir.path().join("args.txt")).unwrap();
    assert!(args.contains("--strict-mcp-config"), "{args}");
    assert!(
        !args.contains(&token),
        "the token must not be on the command line"
    );
    let stdin = std::fs::read_to_string(dir.path().join("stdin.txt")).unwrap();
    assert!(stdin.starts_with("Who scores best?"), "{stdin}");
    assert!(
        stdin.contains("```sql\nSELECT * FROM people\n```"),
        "{stdin}"
    );

    let shots = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/ui-shots");
    std::fs::create_dir_all(&shots).unwrap();
    if let Ok(img) = h.render() {
        img.save(shots.join("13-assistant.png")).unwrap();
    }

    // Insert puts the proposed query at the cursor of the active tab.
    h.state_mut().tabs[0].text.clear();
    h.step();
    h.get_by_label("Insert").click();
    h.step();
    assert_eq!(h.state().tabs[0].text, SQL);
    // New tab opens it next to the others, on the chat's connection.
    let tabs = h.state().tabs.len();
    h.get_by_label("New tab").click();
    h.step();
    let app = h.state();
    assert_eq!(app.tabs.len(), tabs + 1);
    assert_eq!(app.tabs[app.active].text, SQL);
    assert_eq!(app.tabs[app.active].connection, Some(conn));
}
