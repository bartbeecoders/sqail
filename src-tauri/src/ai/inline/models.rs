//! Model catalog + downloader.
//!
//! The Qwen2.5-Coder family is the default spine — Phase A benchmarks in
//! `Vibecoding/inline-ai-benchmarks.md` showed it as the strongest FIM
//! code model at every size we care about, and every variant ships with
//! the `tokenizer.ggml.fim_*` metadata that llama.cpp's `/infill`
//! endpoint depends on. DeepSeek-Coder-V2-Lite is kept as an alternative
//! high-VRAM performance pick.
//!
//! Size ladder (all Q4_K_M):
//!
//! * **low-end** — Qwen2.5-Coder-0.5B   (~400 MB, CPU-friendly)
//! * **low-end** — Qwen2.5-Coder-1.5B   (~1.1 GB)
//! * **default** — Qwen2.5-Coder-3B     (~2.1 GB)
//! * **performance** — Qwen2.5-Coder-7B  (~4.7 GB)
//! * **performance** — Qwen3.5-9B        (~5.3 GB, newer general instruct)
//! * **performance** — Qwen2.5-Coder-14B (~9.0 GB)
//! * **performance** — DeepSeek-Coder-V2-Lite (~10.4 GB, MoE)
//! * **performance** — Qwen3.5-27B          (~15.6 GB)
//! * **performance** — Qwen3-Coder-30B-A3B  (~17.3 GB, MoE coder)
//! * **performance** — Qwen3.6-35B-A3B      (~19.9 GB, MoE)
//!
//! The Qwen3.5 / 3.6 / Qwen3-Coder entries use community mirrors
//! (unsloth, bartowski) because Qwen hasn't published first-party GGUF
//! repos for those variants. All three have `<|fim_prefix|>` /
//! `<|fim_middle|>` / `<|fim_suffix|>` in the tokenizer, so
//! llama.cpp's `/infill` endpoint works for ghost-text. Qwen3.5 is a
//! general instruct model (not a coder-specialist) — chat/palette
//! quality is very good, ghost-text FIM quality is untested.
//!
//! Download files live under `<app_data>/inline-ai/models/`. Downloads
//! resume when partial (`.part` suffix) and verify SHA-256 when a digest
//! is pinned in the catalog.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::sync::Notify;

/// One entry in the hard-coded catalog.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelEntry {
    /// Stable id used everywhere. Lowercase kebab-case.
    pub id: String,
    /// Human-readable label for the settings UI.
    pub display_name: String,
    /// Tier label: "default" | "performance" | "low-end".
    pub tier: String,
    /// Direct download URL to a GGUF file (HuggingFace resolve/raw links).
    pub url: String,
    /// Filename stored on disk (last component of URL).
    pub filename: String,
    /// Known file size in bytes (informational — used for progress totals
    /// when the HTTP response is missing Content-Length).
    pub size_bytes: u64,
    /// Minimum VRAM for a full GPU offload — shown in the settings UI.
    pub min_vram_mib: u32,
    /// Optional SHA-256 of the final file. When present, downloads that
    /// don't match it are rejected.
    #[serde(default)]
    pub sha256: Option<String>,
}

/// Status of a single catalog entry (for `list()`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelListItem {
    #[serde(flatten)]
    pub entry: ModelEntry,
    /// True if the file exists locally at the expected path.
    pub downloaded: bool,
    /// Size on disk in bytes (0 if not downloaded).
    pub disk_size: u64,
}

/// Download progress event payload — emitted as `inline:model-download-progress`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    pub id: String,
    pub downloaded: u64,
    pub total: u64,
    /// One of: "started" | "progress" | "verifying" | "completed" | "cancelled" | "error".
    pub phase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Build the hard-coded catalog. Adding a model means bumping this list.
pub fn catalog() -> Vec<ModelEntry> {
    vec![
        ModelEntry {
            id: "qwen-coder-0_5b-q4".into(),
            display_name: "Qwen2.5-Coder-0.5B (Q4_K_M) — CPU".into(),
            tier: "low-end".into(),
            url: "https://huggingface.co/Qwen/Qwen2.5-Coder-0.5B-Instruct-GGUF/resolve/main/qwen2.5-coder-0.5b-instruct-q4_k_m.gguf".into(),
            filename: "qwen2.5-coder-0.5b-instruct-q4_k_m.gguf".into(),
            size_bytes: 397_807_744,
            min_vram_mib: 700,
            sha256: None,
        },
        ModelEntry {
            id: "qwen-coder-1_5b-q4".into(),
            display_name: "Qwen2.5-Coder-1.5B (Q4_K_M)".into(),
            tier: "low-end".into(),
            url: "https://huggingface.co/Qwen/Qwen2.5-Coder-1.5B-Instruct-GGUF/resolve/main/qwen2.5-coder-1.5b-instruct-q4_k_m.gguf".into(),
            filename: "qwen2.5-coder-1.5b-instruct-q4_k_m.gguf".into(),
            size_bytes: 1_117_320_704,
            min_vram_mib: 1800,
            sha256: None,
        },
        ModelEntry {
            id: "qwen-coder-3b-q4".into(),
            display_name: "Qwen2.5-Coder-3B (Q4_K_M) — default".into(),
            tier: "default".into(),
            url: "https://huggingface.co/Qwen/Qwen2.5-Coder-3B-Instruct-GGUF/resolve/main/qwen2.5-coder-3b-instruct-q4_k_m.gguf".into(),
            filename: "qwen2.5-coder-3b-instruct-q4_k_m.gguf".into(),
            size_bytes: 2_100_000_000,
            min_vram_mib: 2700,
            sha256: None,
        },
        ModelEntry {
            id: "qwen-coder-7b-q4".into(),
            display_name: "Qwen2.5-Coder-7B (Q4_K_M) — performance".into(),
            tier: "performance".into(),
            url: "https://huggingface.co/Qwen/Qwen2.5-Coder-7B-Instruct-GGUF/resolve/main/qwen2.5-coder-7b-instruct-q4_k_m.gguf".into(),
            filename: "qwen2.5-coder-7b-instruct-q4_k_m.gguf".into(),
            size_bytes: 4_683_073_984,
            min_vram_mib: 5600,
            sha256: None,
        },
        ModelEntry {
            id: "qwen-coder-14b-q4".into(),
            display_name: "Qwen2.5-Coder-14B (Q4_K_M) — performance".into(),
            tier: "performance".into(),
            url: "https://huggingface.co/Qwen/Qwen2.5-Coder-14B-Instruct-GGUF/resolve/main/qwen2.5-coder-14b-instruct-q4_k_m.gguf".into(),
            filename: "qwen2.5-coder-14b-instruct-q4_k_m.gguf".into(),
            size_bytes: 8_988_110_656,
            min_vram_mib: 10240,
            sha256: None,
        },
        ModelEntry {
            id: "deepseek-coder-v2-lite-q4".into(),
            display_name: "DeepSeek-Coder-V2-Lite (Q4_K_M) — performance".into(),
            tier: "performance".into(),
            url: "https://huggingface.co/bartowski/DeepSeek-Coder-V2-Lite-Instruct-GGUF/resolve/main/DeepSeek-Coder-V2-Lite-Instruct-Q4_K_M.gguf".into(),
            filename: "DeepSeek-Coder-V2-Lite-Instruct-Q4_K_M.gguf".into(),
            size_bytes: 10_400_000_000,
            min_vram_mib: 11500,
            sha256: None,
        },
        // Qwen3.5 is a general-purpose instruct model with FIM tokens in
        // its vocab. Community GGUF only — no first-party repo.
        ModelEntry {
            id: "qwen3_5-9b-q4".into(),
            display_name: "Qwen3.5-9B (Q4_K_M) — performance".into(),
            tier: "performance".into(),
            url: "https://huggingface.co/unsloth/Qwen3.5-9B-GGUF/resolve/main/Qwen3.5-9B-Q4_K_M.gguf".into(),
            filename: "Qwen3.5-9B-Q4_K_M.gguf".into(),
            size_bytes: 5_680_522_464,
            min_vram_mib: 6400,
            sha256: None,
        },
        ModelEntry {
            id: "qwen3_5-27b-q4".into(),
            display_name: "Qwen3.5-27B (Q4_K_M) — performance".into(),
            tier: "performance".into(),
            url: "https://huggingface.co/unsloth/Qwen3.5-27B-GGUF/resolve/main/Qwen3.5-27B-Q4_K_M.gguf".into(),
            filename: "Qwen3.5-27B-Q4_K_M.gguf".into(),
            size_bytes: 16_740_812_704,
            min_vram_mib: 18432,
            sha256: None,
        },
        // Qwen3-Coder is the current coder-specialist MoE. 30B total /
        // ~3B active params → much faster than a dense 30B at inference.
        ModelEntry {
            id: "qwen3-coder-30b-a3b-q4".into(),
            display_name: "Qwen3-Coder-30B-A3B (Q4_K_M) — performance".into(),
            tier: "performance".into(),
            url: "https://huggingface.co/unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF/resolve/main/Qwen3-Coder-30B-A3B-Instruct-Q4_K_M.gguf".into(),
            filename: "Qwen3-Coder-30B-A3B-Instruct-Q4_K_M.gguf".into(),
            size_bytes: 18_556_689_568,
            min_vram_mib: 20480,
            sha256: None,
        },
        // Qwen3.6 — newest MoE from Qwen, general instruct with FIM
        // tokens in vocab. Using bartowski's mirror (slightly smaller
        // file than unsloth's UD variant).
        ModelEntry {
            id: "qwen3_6-35b-a3b-q4".into(),
            display_name: "Qwen3.6-35B-A3B (Q4_K_M) — performance".into(),
            tier: "performance".into(),
            url: "https://huggingface.co/bartowski/Qwen_Qwen3.6-35B-A3B-GGUF/resolve/main/Qwen_Qwen3.6-35B-A3B-Q4_K_M.gguf".into(),
            filename: "Qwen_Qwen3.6-35B-A3B-Q4_K_M.gguf".into(),
            size_bytes: 21_391_448_384,
            min_vram_mib: 23552,
            sha256: None,
        },
    ]
}

pub fn find(id: &str) -> Option<ModelEntry> {
    catalog().into_iter().find(|m| m.id == id)
}

/// `<app_data>/inline-ai/models/` — created lazily on first access.
pub fn models_dir(app_data: &Path) -> PathBuf {
    app_data.join("inline-ai").join("models")
}

/// Path a given model resolves to on disk.
pub fn model_path(app_data: &Path, entry: &ModelEntry) -> PathBuf {
    models_dir(app_data).join(&entry.filename)
}

/// Path used while a download is in flight.
fn partial_path(app_data: &Path, entry: &ModelEntry) -> PathBuf {
    models_dir(app_data).join(format!("{}.part", entry.filename))
}

pub async fn list(app_data: &Path) -> Vec<ModelListItem> {
    let mut out = Vec::new();
    for entry in catalog() {
        let path = model_path(app_data, &entry);
        let (downloaded, disk_size) = match fs::metadata(&path).await {
            Ok(m) => (m.is_file(), m.len()),
            Err(_) => (false, 0),
        };
        out.push(ModelListItem { entry, downloaded, disk_size });
    }
    out
}

/// Delete a downloaded model's GGUF from disk. No-op if absent.
pub async fn delete(app_data: &Path, id: &str) -> Result<(), String> {
    let entry = find(id).ok_or_else(|| format!("unknown model id: {id}"))?;
    let path = model_path(app_data, &entry);
    match fs::remove_file(&path).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("failed to delete {}: {e}", path.display())),
    }
}

/// Download `id` into `<app_data>/inline-ai/models/`. Emits
/// `inline:model-download-progress` events as it runs. Honours
/// `cancel.notified()` for early aborts.
///
/// Progress is throttled to at most one event per ~200 ms to keep the
/// Tauri IPC channel quiet on fast networks.
pub async fn download(
    app: AppHandle,
    app_data: PathBuf,
    id: String,
    cancel: Arc<Notify>,
) -> Result<PathBuf, String> {
    let entry = find(&id).ok_or_else(|| format!("unknown model id: {id}"))?;
    fs::create_dir_all(models_dir(&app_data))
        .await
        .map_err(|e| format!("mkdir models dir: {e}"))?;

    let final_path = model_path(&app_data, &entry);
    if final_path.exists() {
        emit_progress(&app, &id, 0, 0, "completed", None);
        return Ok(final_path);
    }

    let partial = partial_path(&app_data, &entry);
    let already_have = match fs::metadata(&partial).await {
        Ok(m) if m.is_file() => m.len(),
        _ => 0,
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60 * 60))
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let mut req = client.get(&entry.url);
    if already_have > 0 {
        req = req.header("Range", format!("bytes={already_have}-"));
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() && status.as_u16() != 206 {
        let err = format!("http {status}");
        emit_progress(&app, &id, already_have, 0, "error", Some(&err));
        return Err(err);
    }

    // Total bytes: prefer Content-Range (resume case), then Content-Length,
    // then the catalog's expected size as a last-resort fallback.
    let total = resp
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.rsplit('/').next())
        .and_then(|n| n.parse::<u64>().ok())
        .or_else(|| resp.content_length().map(|cl| cl + already_have))
        .unwrap_or(entry.size_bytes);

    emit_progress(&app, &id, already_have, total, "started", None);

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&partial)
        .await
        .map_err(|e| format!("open partial: {e}"))?;

    let mut stream = resp.bytes_stream();
    let mut downloaded = already_have;
    let mut last_tick = std::time::Instant::now();

    loop {
        tokio::select! {
            _ = cancel.notified() => {
                emit_progress(&app, &id, downloaded, total, "cancelled", None);
                return Err("cancelled".into());
            }
            chunk = stream.next() => {
                match chunk {
                    None => break,
                    Some(Ok(bytes)) => {
                        file.write_all(&bytes)
                            .await
                            .map_err(|e| format!("write: {e}"))?;
                        downloaded += bytes.len() as u64;
                        if last_tick.elapsed() >= Duration::from_millis(200) {
                            emit_progress(&app, &id, downloaded, total, "progress", None);
                            last_tick = std::time::Instant::now();
                        }
                    }
                    Some(Err(e)) => {
                        let err = format!("network: {e}");
                        emit_progress(&app, &id, downloaded, total, "error", Some(&err));
                        return Err(err);
                    }
                }
            }
        }
    }

    file.flush().await.map_err(|e| format!("flush: {e}"))?;
    drop(file);

    // SHA-256 verification (when pinned).
    if let Some(want) = &entry.sha256 {
        emit_progress(&app, &id, downloaded, total, "verifying", None);
        let got = sha256_file(&partial).await?;
        if !got.eq_ignore_ascii_case(want) {
            let _ = fs::remove_file(&partial).await;
            let err = format!("sha256 mismatch: got {got}, want {want}");
            emit_progress(&app, &id, downloaded, total, "error", Some(&err));
            return Err(err);
        }
    }

    fs::rename(&partial, &final_path)
        .await
        .map_err(|e| format!("rename: {e}"))?;

    emit_progress(&app, &id, downloaded, total, "completed", None);
    Ok(final_path)
}

fn emit_progress(
    app: &AppHandle,
    id: &str,
    downloaded: u64,
    total: u64,
    phase: &str,
    error: Option<&str>,
) {
    let _ = app.emit(
        "inline:model-download-progress",
        DownloadProgress {
            id: id.to_string(),
            downloaded,
            total,
            phase: phase.to_string(),
            error: error.map(|s| s.to_string()),
        },
    );
}

async fn sha256_file(path: &Path) -> Result<String, String> {
    use tokio::io::AsyncReadExt;
    let mut f = fs::File::open(path)
        .await
        .map_err(|e| format!("open for hash: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf).await.map_err(|e| format!("read: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}
