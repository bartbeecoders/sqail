//! llama-server lifecycle manager.
//!
//! A single `SidecarManager` owns the running llama-server child process
//! (or the fact that there isn't one). It exposes `start` / `stop` /
//! `status`, runs a health-check loop while alive, and auto-restarts
//! with exponential backoff on unexpected crashes (max 3 attempts per
//! "session" — so a broken model won't loop forever).
//!
//! Binary resolution order:
//! 1. `SQAIL_LLAMA_SERVER_PATH` env (dev escape hatch).
//! 2. Tauri `resource_dir()/llama-server-<target>` if packaged.
//! 3. `<project_root>/.cache/inline-ai/bin/llama-server` when running
//!    `pnpm tauri dev` from the repo (Phase A artifact).

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::time::{sleep, Instant};

use super::binaries as inline_binaries;
use super::models::{self, ModelEntry};

const HEALTH_INTERVAL: Duration = Duration::from_secs(5);
const HEALTH_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_RESTARTS: u32 = 3;

/// State machine visible to the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum SidecarStatus {
    Stopped,
    Starting { model_id: String },
    #[serde(rename_all = "camelCase")]
    Ready { model_id: String, port: u16 },
    Error { message: String },
}

impl SidecarStatus {
    /// Used by Phase C to gate inline completion requests on sidecar
    /// readiness — suppresses the unused-warning until then.
    #[allow(dead_code)]
    pub fn is_ready(&self) -> bool {
        matches!(self, SidecarStatus::Ready { .. })
    }
}

/// Tuning knobs for a single llama-server boot. Defaults come from the
/// Phase A benchmarks (4 K context is plenty for inline completions).
#[derive(Debug, Clone)]
pub struct StartOptions {
    pub ctx_size: u32,
    pub n_gpu_layers: i32,
    /// When true, launch with `--device none` to force CPU.
    pub cpu_only: bool,
}

impl Default for StartOptions {
    fn default() -> Self {
        Self {
            ctx_size: 4096,
            n_gpu_layers: 999,
            cpu_only: false,
        }
    }
}

struct Running {
    child: Child,
    port: u16,
    /// Read via `status` — kept here so a future `inline_sidecar_status`
    /// response can report it without touching the outer `Inner.status`.
    #[allow(dead_code)]
    model_id: String,
    opts: StartOptions,
}

pub struct SidecarManager {
    inner: Mutex<Inner>,
}

struct Inner {
    running: Option<Running>,
    status: SidecarStatus,
    /// Monotonically-increasing generation id. Incremented on every
    /// `start` / `stop` so background health-check tasks can detect that
    /// they've been orphaned and exit.
    generation: u64,
}

impl SidecarManager {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                running: None,
                status: SidecarStatus::Stopped,
                generation: 0,
            }),
        }
    }

    pub async fn status(&self) -> SidecarStatus {
        self.inner.lock().await.status.clone()
    }

    /// Start llama-server with the given model. If a sidecar is already
    /// running on a different model, it is stopped first.
    pub async fn start(
        self: &Arc<Self>,
        app: AppHandle,
        model_id: String,
        opts: StartOptions,
    ) -> Result<u16, String> {
        let app_data = app
            .path()
            .app_data_dir()
            .map_err(|e| format!("app_data_dir: {e}"))?;

        let entry = models::find(&model_id)
            .ok_or_else(|| format!("unknown model id: {model_id}"))?;
        let model_path = models::model_path(&app_data, &entry);
        if !model_path.exists() {
            return Err(format!(
                "model file missing: {} (download it first)",
                model_path.display()
            ));
        }

        let server_bin = resolve_server_bin(&app)
            .ok_or_else(|| "llama-server binary not found".to_string())?;

        // Stop any existing sidecar (different model or restart).
        self.stop_internal().await;

        let generation = {
            let mut inner = self.inner.lock().await;
            inner.generation = inner.generation.wrapping_add(1);
            inner.generation
        };

        self.set_status(&app, SidecarStatus::Starting { model_id: model_id.clone() })
            .await;

        let (child, port) =
            spawn_server(&server_bin, &model_path, &entry, &opts).await?;
        wait_for_health(port).await?;

        {
            let mut inner = self.inner.lock().await;
            inner.running = Some(Running {
                child,
                port,
                model_id: model_id.clone(),
                opts: opts.clone(),
            });
        }

        self.set_status(
            &app,
            SidecarStatus::Ready { model_id: model_id.clone(), port },
        )
        .await;

        // Spawn the health/restart watcher. It exits when `generation`
        // moves on (meaning someone called start/stop again).
        let this = Arc::clone(self);
        let app2 = app.clone();
        let entry2 = entry.clone();
        let bin2 = server_bin.clone();
        let model_path2 = model_path.clone();
        tokio::spawn(async move {
            this.watch(generation, app2, entry2, bin2, model_path2).await;
        });

        Ok(port)
    }

    /// Stop the running sidecar (if any).
    pub async fn stop(&self, app: AppHandle) {
        self.stop_internal().await;
        self.set_status(&app, SidecarStatus::Stopped).await;
    }

    async fn stop_internal(&self) {
        let mut inner = self.inner.lock().await;
        inner.generation = inner.generation.wrapping_add(1);
        if let Some(mut r) = inner.running.take() {
            let _ = r.child.start_kill();
            let _ = r.child.wait().await;
        }
    }

    async fn set_status(&self, app: &AppHandle, status: SidecarStatus) {
        {
            let mut inner = self.inner.lock().await;
            inner.status = status.clone();
        }
        let _ = app.emit("inline:sidecar-status", status);
    }

    /// Background watcher: polls `/health`, restarts on failures with
    /// exponential backoff, bails out after `MAX_RESTARTS` or when
    /// another call supersedes this generation.
    async fn watch(
        self: Arc<Self>,
        generation: u64,
        app: AppHandle,
        entry: ModelEntry,
        server_bin: PathBuf,
        model_path: PathBuf,
    ) {
        let mut restart_attempt: u32 = 0;
        loop {
            sleep(HEALTH_INTERVAL).await;

            // Did someone else take over?
            let (is_current, port, opts) = {
                let inner = self.inner.lock().await;
                if inner.generation != generation {
                    return;
                }
                match &inner.running {
                    Some(r) => (true, r.port, r.opts.clone()),
                    None => (false, 0, StartOptions::default()),
                }
            };
            if !is_current {
                return;
            }

            let healthy = health_check(port).await;
            if healthy {
                restart_attempt = 0;
                continue;
            }

            restart_attempt += 1;
            if restart_attempt > MAX_RESTARTS {
                self.set_status(
                    &app,
                    SidecarStatus::Error {
                        message: format!(
                            "llama-server failed {MAX_RESTARTS} restarts; giving up"
                        ),
                    },
                )
                .await;
                self.stop_internal().await;
                return;
            }

            // Exponential backoff: 1s, 2s, 4s.
            let delay_secs = 1u64 << (restart_attempt - 1);
            log::warn!(
                "llama-server health check failed — restart #{} in {}s",
                restart_attempt,
                delay_secs
            );
            // Kill the current child before respawning.
            {
                let mut inner = self.inner.lock().await;
                if inner.generation != generation {
                    return;
                }
                if let Some(mut r) = inner.running.take() {
                    let _ = r.child.start_kill();
                    let _ = r.child.wait().await;
                }
            }
            sleep(Duration::from_secs(delay_secs)).await;

            self.set_status(
                &app,
                SidecarStatus::Starting { model_id: entry.id.clone() },
            )
            .await;

            match spawn_server(&server_bin, &model_path, &entry, &opts).await {
                Ok((child, new_port)) => match wait_for_health(new_port).await {
                    Ok(()) => {
                        let mut inner = self.inner.lock().await;
                        if inner.generation != generation {
                            return;
                        }
                        inner.running = Some(Running {
                            child,
                            port: new_port,
                            model_id: entry.id.clone(),
                            opts,
                        });
                        drop(inner);
                        self.set_status(
                            &app,
                            SidecarStatus::Ready {
                                model_id: entry.id.clone(),
                                port: new_port,
                            },
                        )
                        .await;
                    }
                    Err(e) => {
                        log::warn!("restart #{restart_attempt} never became healthy: {e}");
                    }
                },
                Err(e) => {
                    log::warn!("restart #{restart_attempt} failed to spawn: {e}");
                }
            }
        }
    }
}

async fn spawn_server(
    bin: &Path,
    model_path: &Path,
    entry: &ModelEntry,
    opts: &StartOptions,
) -> Result<(Child, u16), String> {
    let port = free_port()?;

    let mut cmd = Command::new(bin);
    cmd.arg("-m")
        .arg(model_path)
        .args(["--host", "127.0.0.1"])
        .args(["--port", &port.to_string()])
        .args(["--ctx-size", &opts.ctx_size.to_string()])
        .args(["--parallel", "1"])
        .arg("--no-mmap")
        .arg("--log-disable");
    if opts.cpu_only {
        cmd.args(["--device", "none"]);
    } else {
        cmd.args(["--n-gpu-layers", &opts.n_gpu_layers.to_string()]);
    }
    cmd.kill_on_drop(true);

    // LD_LIBRARY_PATH for the dev-cache build (libggml*.so live next to the binary).
    if let Some(parent) = bin.parent() {
        let existing = std::env::var("LD_LIBRARY_PATH").unwrap_or_default();
        let joined = if existing.is_empty() {
            parent.to_string_lossy().into_owned()
        } else {
            format!("{}:{existing}", parent.display())
        };
        cmd.env("LD_LIBRARY_PATH", joined);
    }

    // On Unix, run llama-server in its own process group so a drop-kill
    // tears the whole tree down even if it forks children (it doesn't
    // today, but we should be defensive).
    #[cfg(unix)]
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }

    let child = cmd
        .spawn()
        .map_err(|e| format!("spawn {}: {e}", bin.display()))?;

    log::info!(
        "started llama-server on 127.0.0.1:{port} pid={:?} model={}",
        child.id(),
        entry.filename
    );
    Ok((child, port))
}

fn free_port() -> Result<u16, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("bind: {e}"))?;
    listener
        .local_addr()
        .map(|a| a.port())
        .map_err(|e| format!("local_addr: {e}"))
}

async fn wait_for_health(port: u16) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(120);
    let client = reqwest::Client::builder()
        .timeout(HEALTH_TIMEOUT)
        .build()
        .map_err(|e| format!("http: {e}"))?;
    let url = format!("http://127.0.0.1:{port}/health");
    while Instant::now() < deadline {
        if let Ok(resp) = client.get(&url).send().await {
            if resp.status().is_success() {
                return Ok(());
            }
        }
        sleep(Duration::from_millis(500)).await;
    }
    Err("llama-server did not become healthy within 120s".into())
}

async fn health_check(port: u16) -> bool {
    let Ok(client) = reqwest::Client::builder().timeout(HEALTH_TIMEOUT).build() else {
        return false;
    };
    let url = format!("http://127.0.0.1:{port}/health");
    client.get(url).send().await.map(|r| r.status().is_success()).unwrap_or(false)
}

/// Find the `llama-server` executable. See module docs for the search
/// order.
fn resolve_server_bin(app: &AppHandle) -> Option<PathBuf> {
    if let Ok(p) = std::env::var("SQAIL_LLAMA_SERVER_PATH") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }

    // Packaged sidecar (externalBin in tauri.conf.json) — Tauri suffixes
    // the binary with the target triple. We try a handful of fixed names
    // inside resource_dir() and pick the first that exists.
    if let Ok(res_dir) = app.path().resource_dir() {
        for candidate in [
            res_dir.join("llama-server"),
            res_dir.join(if cfg!(windows) { "llama-server.exe" } else { "llama-server" }),
        ] {
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    // Runtime-downloaded binary in <app_data>/inline-ai/bin/.
    if let Ok(app_data) = app.path().app_data_dir() {
        if let Some(p) = inline_binaries::expected_binary_path(&app_data) {
            if p.exists() {
                return Some(p);
            }
        }
    }

    // Dev fallback — the Phase A script puts the binary here.
    let dev = dev_cache_bin();
    if dev.exists() {
        return Some(dev);
    }

    None
}

fn dev_cache_bin() -> PathBuf {
    // Walk up from CARGO_MANIFEST_DIR (= src-tauri) to the project root.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let project_root = manifest.parent().unwrap_or(&manifest).to_path_buf();
    let name = if cfg!(windows) { "llama-server.exe" } else { "llama-server" };
    project_root.join(".cache").join("inline-ai").join("bin").join(name)
}
