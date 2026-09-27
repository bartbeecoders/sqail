//! "Use a local service": find the sqail-service binary, start it if it is not
//! running, and provision a token through its CLI (same machine, same user,
//! same data directory).

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail};

pub const LOCAL_URL: &str = "https://127.0.0.1:7443";

pub struct Provisioned {
    pub url: String,
    pub fingerprint: String,
    pub token: String,
}

fn exe_name() -> &'static str {
    if cfg!(windows) {
        "sqail-service.exe"
    } else {
        "sqail-service"
    }
}

/// Next to the sqail executable first, then `$PATH`.
pub fn find_binary() -> Option<PathBuf> {
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(PathBuf::from))
    {
        let candidate = dir.join(exe_name());
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join(exe_name()))
            .find(|p| p.is_file())
    })
}

async fn is_up(url: &str) -> bool {
    sqail_client::probe(url).await.is_ok()
}

/// Start the service in the background unless it already answers.
pub async fn ensure_running(url: &str) -> Result<()> {
    if is_up(url).await {
        return Ok(());
    }
    let bin = find_binary().context("sqail-service binary not found next to sqail or on PATH")?;
    tracing::info!(bin = %bin.display(), "starting local sqail-service");
    let mut cmd = Command::new(&bin);
    // provision() creates this user's token itself; a bootstrap token
    // nobody sees would only be clutter.
    cmd.args(["serve", "--no-bootstrap-token"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn()
        .with_context(|| format!("starting {}", bin.display()))?;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if is_up(url).await {
            return Ok(());
        }
    }
    bail!("sqail-service did not come up on {url} within 10 s")
}

/// Start (if needed) and create a fresh admin token for this user.
pub async fn provision() -> Result<Provisioned> {
    let url = LOCAL_URL.to_string();
    ensure_running(&url).await?;
    let (fingerprint, _) = sqail_client::probe(&url).await?;
    let bin = find_binary().context("sqail-service binary not found")?;
    let host = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "this computer".into());
    let out = tokio::process::Command::new(&bin)
        .args([
            "token",
            "create",
            "--name",
            &format!("sqail on {host}"),
            "--scope",
            "admin",
        ])
        .stdin(Stdio::null())
        .output()
        .await
        .with_context(|| format!("running {}", bin.display()))?;
    if !out.status.success() {
        bail!(
            "token create failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let token = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !token.starts_with("sq2_") {
        bail!("unexpected output from token create");
    }
    Ok(Provisioned {
        url,
        fingerprint,
        token,
    })
}
