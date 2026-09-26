//! The admin page and `/v1/admin`: static files, access rules, settings
//! validation, certificate upload, and applying settings by an in-process
//! restart. Needs nothing but a temp dir.

use std::net::SocketAddr;
use std::time::Duration;

use serde_json::{Value, json};
use sqail_service::Config;
use tokio::sync::mpsc;

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap()
}

fn config(dir: &tempfile::TempDir) -> Config {
    Config {
        data_dir: dir.path().to_path_buf(),
        bind: "127.0.0.1:0".parse().unwrap(),
        docs_ui: false,
        ..Default::default()
    }
}

/// A service under [`sqail_service::run`]; every (re)start reports its
/// address on `started`.
struct Supervised {
    started: mpsc::UnboundedReceiver<SocketAddr>,
    admin: String,
    addr: SocketAddr,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<anyhow::Result<()>>,
}

impl Supervised {
    async fn new(cfg: Config) -> Self {
        let (tx, mut started) = mpsc::unbounded_channel();
        let (admin_tx, admin_rx) = tokio::sync::oneshot::channel();
        let mut admin_tx = Some(admin_tx);
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let reload_cfg = cfg.clone();
        let task = tokio::spawn(sqail_service::run(
            cfg,
            move || reload_cfg.with_file_settings(reload_cfg.file_settings()?),
            async {
                let _ = stopped.await;
            },
            move |server| {
                if let Some(t) = admin_tx.take() {
                    let _ = t.send(server.bootstrap_token.clone().expect("fresh data dir"));
                }
                let _ = tx.send(server.addr);
                Ok(())
            },
        ));
        let addr = started.recv().await.expect("server starts");
        Self {
            started,
            admin: admin_rx.await.unwrap(),
            addr,
            stop: Some(stop),
            task,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("https://{}{path}", self.addr)
    }

    async fn next_start(&mut self) -> SocketAddr {
        let addr = tokio::time::timeout(Duration::from_secs(30), self.started.recv())
            .await
            .expect("restarted in time")
            .expect("still running");
        self.addr = addr;
        addr
    }

    async fn stop(mut self) {
        let _ = self.stop.take().unwrap().send(());
        self.task.await.unwrap().unwrap();
    }
}

async fn get(url: &str, token: &str) -> (u16, Value) {
    let res = client().get(url).bearer_auth(token).send().await.unwrap();
    let status = res.status().as_u16();
    let text = res.text().await.unwrap();
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

async fn send(method: reqwest::Method, url: &str, token: &str, body: &Value) -> (u16, Value) {
    let res = client()
        .request(method, url)
        .bearer_auth(token)
        .json(body)
        .send()
        .await
        .unwrap();
    let status = res.status().as_u16();
    let text = res.text().await.unwrap();
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

async fn token(base: &str, admin: &str, scope: &str) -> String {
    let (status, body) = send(
        reqwest::Method::POST,
        &format!("{base}/v1/tokens"),
        admin,
        &json!({"name": scope, "scope": scope}),
    )
    .await;
    assert_eq!(status, 201);
    body["token"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn admin_page_is_served_without_a_token_and_locked_down() {
    let dir = tempfile::tempdir().unwrap();
    let server = sqail_service::start(config(&dir)).await.unwrap();
    let base = format!("https://{}", server.addr);
    let http = client();

    let res = http.get(format!("{base}/admin/")).send().await.unwrap();
    assert_eq!(res.status(), 200);
    assert!(
        res.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    let csp = res.headers()["content-security-policy"].to_str().unwrap();
    assert!(csp.contains("script-src 'self'") && csp.contains("frame-ancestors 'none'"));
    assert!(res.text().await.unwrap().contains("app.js"));

    for (path, kind) in [
        ("/admin/app.js", "text/javascript"),
        ("/admin/app.css", "text/css"),
        ("/admin/icon.svg", "image/svg+xml"),
    ] {
        let res = http.get(format!("{base}{path}")).send().await.unwrap();
        assert_eq!(res.status(), 200, "{path}");
        assert!(
            res.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with(kind)
        );
    }

    let res = http.get(format!("{base}/")).send().await.unwrap();
    // reqwest follows the redirect to the page.
    assert_eq!(res.url().path(), "/admin/");
}

#[tokio::test]
async fn admin_api_needs_the_admin_scope() {
    let dir = tempfile::tempdir().unwrap();
    let server = sqail_service::start(config(&dir)).await.unwrap();
    let base = format!("https://{}", server.addr);
    let admin = server.bootstrap_token.clone().unwrap();

    let res = client()
        .get(format!("{base}/v1/admin/status"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401);
    let query = token(&base, &admin, "query").await;
    for path in ["/v1/admin/status", "/v1/admin/settings"] {
        assert_eq!(get(&format!("{base}{path}"), &query).await.0, 403, "{path}");
    }

    let (status, s) = get(&format!("{base}/v1/admin/status"), &admin).await;
    assert_eq!(status, 200);
    assert_eq!(s["fingerprint"], server.fingerprint.as_str());
    assert_eq!(s["self_signed"], true);
    assert_eq!(s["loopback_only"], true);
    assert_eq!(s["active_tokens"], 2);
    assert!(s["last_restart_error"].is_null());

    let (status, doc) = get(&format!("{base}/v1/admin/settings"), &admin).await;
    assert_eq!(status, 200);
    // No file yet: the defaults, not the harness overrides.
    assert_eq!(doc["settings"]["bind"], "127.0.0.1:7443");
    assert_eq!(doc["settings"]["limits"]["max_rows"], 5_000_000);
}

#[tokio::test]
async fn bad_settings_are_rejected_before_restarting() {
    let dir = tempfile::tempdir().unwrap();
    let server = sqail_service::start(config(&dir)).await.unwrap();
    let base = format!("https://{}", server.addr);
    let admin = server.bootstrap_token.clone().unwrap();
    let (_, doc) = get(&format!("{base}/v1/admin/settings"), &admin).await;
    let url = format!("{base}/v1/admin/settings");

    let mut s = doc["settings"].clone();
    s["limits"]["max_rows"] = json!(1);
    let (status, body) = send(reqwest::Method::PUT, &url, &admin, &s).await;
    assert_eq!(status, 400);
    assert!(body["detail"].as_str().unwrap().contains("max_rows"));

    let mut s = doc["settings"].clone();
    s["sqlite"]["allowed_dirs"] = json!([dir.path().join("missing")]);
    let (status, body) = send(reqwest::Method::PUT, &url, &admin, &s).await;
    assert_eq!(status, 400, "{body}");
    assert!(body["detail"].as_str().unwrap().contains("does not exist"));

    let mut s = doc["settings"].clone();
    s["tls"]["cert"] = json!(dir.path().join("nope.pem"));
    s["tls"]["key"] = json!(dir.path().join("nope.key"));
    let (status, _) = send(reqwest::Method::PUT, &url, &admin, &s).await;
    assert_eq!(status, 400);

    let mut s = doc["settings"].clone();
    s["bnid"] = json!("typo");
    let (status, _) = send(reqwest::Method::PUT, &url, &admin, &s).await;
    assert_eq!(status, 422, "unknown keys are rejected");

    assert!(
        !dir.path().join("sqail-service.toml").exists()
            || std::fs::read_to_string(dir.path().join("sqail-service.toml"))
                .unwrap()
                .is_empty(),
        "nothing was saved"
    );
}

#[tokio::test]
async fn certificate_upload_checks_the_pair() {
    let dir = tempfile::tempdir().unwrap();
    let server = sqail_service::start(config(&dir)).await.unwrap();
    let base = format!("https://{}", server.addr);
    let admin = server.bootstrap_token.clone().unwrap();
    let url = format!("{base}/v1/admin/certificate");

    let a = rcgen::generate_simple_self_signed(vec!["gw.example".into()]).unwrap();
    let b = rcgen::generate_simple_self_signed(vec!["other.example".into()]).unwrap();
    let (status, body) = send(
        reqwest::Method::POST,
        &url,
        &admin,
        &json!({"cert_pem": a.cert.pem(), "key_pem": b.signing_key.serialize_pem()}),
    )
    .await;
    assert_eq!(status, 400, "mismatched key: {body}");

    let (status, body) = send(
        reqwest::Method::POST,
        &url,
        &admin,
        &json!({"cert_pem": a.cert.pem(), "key_pem": a.signing_key.serialize_pem()}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["fingerprint"].as_str().unwrap().len(), 95);
    assert!(std::path::Path::new(body["key"].as_str().unwrap()).exists());
}

#[tokio::test]
async fn admin_ui_can_be_switched_off() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(&dir);
    cfg.admin_ui = false;
    let server = sqail_service::start(cfg).await.unwrap();
    let base = format!("https://{}", server.addr);
    let admin = server.bootstrap_token.clone().unwrap();
    let res = client().get(format!("{base}/admin/")).send().await.unwrap();
    assert_eq!(res.status(), 404);
    assert_eq!(get(&format!("{base}/v1/admin/status"), &admin).await.0, 404);
    // The regular API is unaffected.
    assert_eq!(get(&format!("{base}/v1/tokens"), &admin).await.0, 200);
}

#[tokio::test]
async fn applied_settings_restart_the_service_and_are_saved() {
    let dir = tempfile::tempdir().unwrap();
    let mut svc = Supervised::new(config(&dir)).await;
    let admin = svc.admin.clone();

    let (_, doc) = get(&svc.url("/v1/admin/settings"), &admin).await;
    let mut s = doc["settings"].clone();
    // Keep an ephemeral port so the test never collides with anything.
    s["bind"] = json!("127.0.0.1:0");
    s["limits"]["max_rows"] = json!(123_456);
    s["sqlite"]["allowed_dirs"] = json!([dir.path()]);
    let (status, body) = send(
        reqwest::Method::PUT,
        &svc.url("/v1/admin/settings"),
        &admin,
        &s,
    )
    .await;
    assert_eq!(status, 202, "{body}");
    svc.next_start().await;

    // Same data dir: tokens survive, the new settings are live and on disk.
    let (status, st) = get(&svc.url("/v1/admin/status"), &admin).await;
    assert_eq!(status, 200);
    assert_eq!(st["sqlite_enabled"], true);
    assert!(st["last_restart_error"].is_null(), "{st}");
    let saved: Config =
        toml::from_str(&std::fs::read_to_string(dir.path().join("sqail-service.toml")).unwrap())
            .unwrap();
    assert_eq!(saved.limits.max_rows, 123_456);
    assert_eq!(saved.sqlite.allowed_dirs, vec![dir.path().to_path_buf()]);

    // A plain restart re-reads the file.
    let (status, _) = send(
        reqwest::Method::POST,
        &svc.url("/v1/admin/restart"),
        &admin,
        &json!(null),
    )
    .await;
    assert_eq!(status, 202);
    svc.next_start().await;
    let (_, doc) = get(&svc.url("/v1/admin/settings"), &admin).await;
    assert_eq!(doc["settings"]["limits"]["max_rows"], 123_456);

    svc.stop().await;
}
