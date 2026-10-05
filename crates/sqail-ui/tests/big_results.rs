//! A million-row SQL Server table (`sales.big_orders`, seeded by
//! `scripts/db.sh up`) through the real app: streaming it into the grid,
//! memory, scrolling and sorting. Needs the podman test databases:
//!
//!     SQAIL_IT=1 cargo test --release -p sqail-ui --test big_results -- --ignored --nocapture

use std::path::PathBuf;
use std::time::{Duration, Instant};

use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use sqail_client::proto::{
    ConnectionInput, ConnectionParams, MssqlAuth, MssqlEncrypt, MssqlParams,
};
use sqail_client::{Client, Target, Trust};
use sqail_ui::SqailApp;
use sqail_ui::settings::{ServiceProfile, Settings};

const ROWS: usize = 1_000_000;

fn rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
        })
        .unwrap_or(0)
}

fn step_until(
    h: &mut Harness<'_, SqailApp>,
    what: &str,
    limit: Duration,
    done: impl Fn(&SqailApp) -> bool,
) {
    let start = Instant::now();
    while !done(h.state()) {
        assert!(start.elapsed() < limit, "timed out waiting for {what}");
        h.step();
    }
}

#[test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
fn mssql_million_rows() {
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
    let token = server.bootstrap_token.clone().unwrap();
    let client = Client::new(&Target {
        url: url.clone(),
        token: token.clone(),
        trust: Trust::Pinned(server.fingerprint.clone()),
        identity: None,
    })
    .unwrap();
    rt.block_on(client.create_connection(&ConnectionInput {
        name: "ms".into(),
        params: ConnectionParams::Mssql(MssqlParams {
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
        password: Some("Sqail2_dev!Passw0rd".into()),
        ssl_client_key: None,
        read_only: false,
        color: None,
        environment: None,
        folder: None,
    }))
    .unwrap();

    sqail_ui::settings::override_config_dir(dir.path().join("ui"));
    sqail_ui::secrets::force_file_store();
    let mut settings = Settings {
        max_rows: 1_000_000,
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
    step_until(&mut h, "connections", Duration::from_secs(20), |a| {
        a.service.connections.len() == 1
    });
    let ms = h.state().service.connections[0].id;
    h.state_mut().tabs[0].connection = Some(ms);
    h.state_mut().tabs[0].text = "SELECT * FROM sales.big_orders".into();
    h.step();

    // --- stream a million rows into the grid ------------------------------
    let rss_before = rss_kb();
    let before = h.state().tabs[0].run.as_ref().map_or(0, |r| r.id);
    h.key_press(Key::F5);
    let start = Instant::now();
    let mut first_rows = None;
    let (mut frames, mut total, mut worst) = (0u32, Duration::ZERO, Duration::ZERO);
    loop {
        let (done, rows) = h.state().tabs[0]
            .run
            .as_ref()
            .filter(|r| r.id != before)
            .map_or((false, 0), |r| (!r.running, r.total_rows()));
        if first_rows.is_none() && rows > 0 {
            first_rows = Some(start.elapsed());
        }
        if done {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(120),
            "1M rows took too long"
        );
        let t = Instant::now();
        h.step();
        let dt = t.elapsed();
        frames += 1;
        total += dt;
        worst = worst.max(dt);
    }
    let elapsed = start.elapsed();
    let rss_after = rss_kb();
    let run = h.state().tabs[0].run.as_ref().unwrap();
    assert!(
        !run.failed,
        "query failed: {:?}",
        run.messages.iter().map(|m| &m.text).collect::<Vec<_>>()
    );
    let rs = &run.results[0];
    assert_eq!(rs.len(), ROWS);
    assert!(
        !rs.truncated,
        "the result must not be capped at the row limit"
    );
    assert_eq!(rs.columns.len(), 10);
    eprintln!(
        "streamed {ROWS} rows × 10 columns: first rows after {:?}, all after {elapsed:?}",
        first_rows.unwrap_or_default()
    );
    eprintln!(
        "  frames while streaming: {frames}, avg {:?}, worst {worst:?}",
        total / frames.max(1)
    );
    eprintln!(
        "  memory growth: {} MB (~{} bytes/row)",
        rss_after.saturating_sub(rss_before) / 1024,
        rss_after.saturating_sub(rss_before) * 1024 / ROWS as u64
    );

    // --- scroll through it ------------------------------------------------
    let grid = h.get_by_label("Result 1 (1,000,000)").rect().center();
    let (mut total, mut worst) = (Duration::ZERO, Duration::ZERO);
    let n = 60;
    for i in 0..n {
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
        total += dt;
        worst = worst.max(dt);
    }
    eprintln!("  scrolling: avg {:?}/frame, worst {worst:?}", total / n);

    // --- sort by an int, a decimal and a text column ----------------------
    for (col, name) in [
        (0, "id (int)"),
        (7, "total (decimal)"),
        (9, "note (text, mostly NULL)"),
    ] {
        let t = Instant::now();
        h.state_mut().tabs[0].run.as_mut().unwrap().results[0].toggle_sort(col);
        eprintln!("  sort by {name}: {:?}", t.elapsed());
    }
    let rs = &h.state().tabs[0].run.as_ref().unwrap().results[0];
    // Ascending by note: the 900,000 NULLs come first.
    assert!(rs.row(0)[9].is_null());
    assert!(!rs.row(ROWS - 1)[9].is_null());
    h.step();

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/ui-shots");
    std::fs::create_dir_all(&root).unwrap();
    if let Ok(img) = h.render() {
        img.save(root.join("12-mssql-million-rows.png")).unwrap();
    }
}
