//! Secrets and row values must never reach the logs. Everything the service
//! (and the HTTP stack) logs at TRACE is captured while a token is used, a
//! profile with a password is created, and a sentinel value is selected.
//!
//! Database drivers stay at the production level (warn): at debug they echo
//! SQL text, which may contain literals; that is documented in docs/security.md.

use std::io::Write;
use std::sync::{Arc, Mutex};

use serde_json::json;

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn logs_contain_no_secrets_or_row_values() {
    let capture = Capture::default();
    let writer = capture.clone();
    tracing_subscriber::fmt()
        .with_env_filter("trace,tokio_postgres=warn,tiberius=warn,rusqlite=warn")
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .init();

    let dir = tempfile::tempdir().unwrap();
    let mut cfg = sqail_service::Config {
        data_dir: dir.path().to_path_buf(),
        bind: "127.0.0.1:0".parse().unwrap(),
        docs_ui: false,
        ..Default::default()
    };
    cfg.sqlite.allowed_dirs = vec![dir.path().to_path_buf()];
    let server = sqail_service::start(cfg).await.unwrap();
    let token = server.bootstrap_token.clone().unwrap();
    let http = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap();
    let url = |p: &str| format!("https://{}{p}", server.addr);

    let password = "PASSWORD_SENTINEL_7f3a";
    let row_value = "ROWVALUE_SENTINEL_91c2";
    let res: serde_json::Value = http
        .post(url("/v1/connections"))
        .bearer_auth(&token)
        .json(&json!({
            "name": "t",
            "params": {"engine": "sqlite", "path": dir.path().join("t.db"), "create": true},
            "password": password,
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = res["id"].as_str().unwrap().to_string();
    // The value only exists in the data: build it with SQL so the literal
    // never appears in a request either.
    let body = http
        .post(url(&format!("/v1/connections/{id}/query")))
        .bearer_auth(&token)
        .json(&json!({"sql": "CREATE TABLE s (v TEXT); INSERT INTO s VALUES ('ROWVALUE_' || 'SENTINEL_91c2'); SELECT v FROM s"}))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        body.contains(row_value),
        "the query really returned the sentinel"
    );
    // A failed login attempt with a wrong token must not log that token either.
    let wrong = "sq2_WRONGTOKEN_SENTINEL";
    let _ = http
        .get(url("/v1/info"))
        .bearer_auth(wrong)
        .send()
        .await
        .unwrap();

    server.shutdown();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let logs = String::from_utf8_lossy(&capture.0.lock().unwrap()).into_owned();
    assert!(
        logs.len() > 1000,
        "logging was captured ({} bytes)",
        logs.len()
    );
    for secret in [token.as_str(), password, row_value, wrong] {
        assert!(!logs.contains(secret), "leaked into logs: {secret}");
    }
}
