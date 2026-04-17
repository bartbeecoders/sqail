//! Runtime download of the `llama-server` sidecar binary.
//!
//! We ship sqail without a bundled `llama-server`. When the user first
//! enables inline AI, we fetch a platform-appropriate build from the
//! upstream llama.cpp GitHub release and drop the executable (plus any
//! shared libraries next to it) into `<app_data>/inline-ai/bin/`. The
//! sidecar manager then finds it there via the usual resolver chain.
//!
//! Default variants (all Q4-fast, broad-compat, no CUDA runtime needed):
//! * Windows x86_64 → Vulkan build (`.zip`)
//! * macOS arm64    → Metal build  (`.tar.gz`)
//! * macOS x86_64   → Metal build  (`.tar.gz`)
//! * Linux x86_64   → Vulkan build (`.tar.gz`)
//!
//! Power users can override by setting `SQAIL_LLAMA_SERVER_PATH` before
//! launching sqail — the sidecar resolver prefers that.

use std::io::{self, Cursor};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::sync::Notify;

/// Upstream release tag. Kept in lockstep with
/// `scripts/fetch-llama-cpp.sh` so the dev build and the shipped
/// runtime are speaking the same protocol.
pub const RELEASE_TAG: &str = "b8815";

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)] // platform-gated: only one variant is live per host
enum Archive {
    Zip,
    TarGz,
}

/// One platform entry in the catalog.
#[derive(Debug, Clone)]
struct Asset {
    url: String,
    archive: Archive,
    /// Name of the executable inside the archive — used both to verify
    /// extraction succeeded and to report the final binary path.
    exe_name: &'static str,
}

/// Resolve the asset for the *current* host. `None` means no runtime
/// download is offered — the user can still bring their own via
/// `SQAIL_LLAMA_SERVER_PATH`.
fn asset_for_host() -> Option<Asset> {
    let base = format!(
        "https://github.com/ggml-org/llama.cpp/releases/download/{RELEASE_TAG}"
    );

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        return Some(Asset {
            url: format!("{base}/llama-{RELEASE_TAG}-bin-win-vulkan-x64.zip"),
            archive: Archive::Zip,
            exe_name: "llama-server.exe",
        });
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        return Some(Asset {
            url: format!("{base}/llama-{RELEASE_TAG}-bin-macos-arm64.tar.gz"),
            archive: Archive::TarGz,
            exe_name: "llama-server",
        });
    }

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        return Some(Asset {
            url: format!("{base}/llama-{RELEASE_TAG}-bin-macos-x64.tar.gz"),
            archive: Archive::TarGz,
            exe_name: "llama-server",
        });
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        return Some(Asset {
            url: format!("{base}/llama-{RELEASE_TAG}-bin-ubuntu-vulkan-x64.tar.gz"),
            archive: Archive::TarGz,
            exe_name: "llama-server",
        });
    }

    #[allow(unreachable_code)]
    {
        let _ = base;
        None
    }
}

/// Status surface for the frontend. Mirrors the shape of
/// `ModelListItem` but there's only ever one binary.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BinaryStatus {
    pub supported: bool,
    pub installed: bool,
    pub path: Option<String>,
    pub url: Option<String>,
    pub release_tag: &'static str,
}

/// Progress event payload — emitted as `inline:binary-download-progress`.
/// Same phase vocabulary as the model downloader so the frontend can
/// reuse the download-state reducer shape.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: u64,
    /// One of: "started" | "progress" | "extracting" | "completed"
    ///       | "cancelled" | "error".
    pub phase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `<app_data>/inline-ai/bin/` — where the extracted binary + siblings
/// end up.
pub fn bin_dir(app_data: &Path) -> PathBuf {
    app_data.join("inline-ai").join("bin")
}

fn partial_path(app_data: &Path, asset: &Asset) -> PathBuf {
    let name = match asset.archive {
        Archive::Zip => "llama-server.zip.part",
        Archive::TarGz => "llama-server.tar.gz.part",
    };
    bin_dir(app_data).join(name)
}

/// Expected path of the extracted binary.
pub fn expected_binary_path(app_data: &Path) -> Option<PathBuf> {
    asset_for_host().map(|a| bin_dir(app_data).join(a.exe_name))
}

pub async fn status(app_data: &Path) -> BinaryStatus {
    let Some(asset) = asset_for_host() else {
        return BinaryStatus {
            supported: false,
            installed: false,
            path: None,
            url: None,
            release_tag: RELEASE_TAG,
        };
    };

    let bin_path = bin_dir(app_data).join(asset.exe_name);
    let installed = fs::metadata(&bin_path)
        .await
        .map(|m| m.is_file())
        .unwrap_or(false);

    BinaryStatus {
        supported: true,
        installed,
        path: installed.then(|| bin_path.to_string_lossy().into_owned()),
        url: Some(asset.url),
        release_tag: RELEASE_TAG,
    }
}

/// Delete the on-disk binary + any siblings extracted with it. No-op if
/// the directory is missing.
pub async fn delete(app_data: &Path) -> Result<(), String> {
    let dir = bin_dir(app_data);
    match fs::remove_dir_all(&dir).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("failed to delete {}: {e}", dir.display())),
    }
}

/// Download + extract the platform-appropriate llama-server. Emits
/// `inline:binary-download-progress` events throughout. Honours
/// `cancel.notified()` for early aborts.
pub async fn download(
    app: AppHandle,
    app_data: PathBuf,
    cancel: Arc<Notify>,
) -> Result<PathBuf, String> {
    let asset = asset_for_host().ok_or_else(|| {
        "Automatic runtime download is not available for this platform. \
         Set SQAIL_LLAMA_SERVER_PATH to point at your own llama-server binary."
            .to_string()
    })?;

    let dir = bin_dir(&app_data);
    fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("mkdir bin dir: {e}"))?;

    let partial = partial_path(&app_data, &asset);
    let already_have = match fs::metadata(&partial).await {
        Ok(m) if m.is_file() => m.len(),
        _ => 0,
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60 * 30))
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let mut req = client.get(&asset.url);
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
        emit_progress(&app, already_have, 0, "error", Some(&err));
        return Err(err);
    }

    let total = resp
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.rsplit('/').next())
        .and_then(|n| n.parse::<u64>().ok())
        .or_else(|| resp.content_length().map(|cl| cl + already_have))
        .unwrap_or(0);

    emit_progress(&app, already_have, total, "started", None);

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
                emit_progress(&app, downloaded, total, "cancelled", None);
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
                            emit_progress(&app, downloaded, total, "progress", None);
                            last_tick = std::time::Instant::now();
                        }
                    }
                    Some(Err(e)) => {
                        let err = format!("network: {e}");
                        emit_progress(&app, downloaded, total, "error", Some(&err));
                        return Err(err);
                    }
                }
            }
        }
    }
    file.flush().await.map_err(|e| format!("flush: {e}"))?;
    drop(file);

    emit_progress(&app, downloaded, total, "extracting", None);

    let archive_bytes = fs::read(&partial)
        .await
        .map_err(|e| format!("read archive: {e}"))?;

    // Extraction is synchronous — hop onto a blocking thread so we don't
    // stall the tokio runtime while chugging through ~100 MB of zip.
    let dir_for_extract = dir.clone();
    let exe_name = asset.exe_name.to_string();
    let archive_kind = asset.archive;
    let extract_result: Result<(), String> = tokio::task::spawn_blocking(move || {
        match archive_kind {
            Archive::Zip => extract_zip(&archive_bytes, &dir_for_extract, &exe_name),
            Archive::TarGz => extract_tar_gz(&archive_bytes, &dir_for_extract, &exe_name),
        }
    })
    .await
    .map_err(|e| format!("extract join: {e}"))?;
    extract_result?;

    // Drop the staged archive now that extraction succeeded.
    let _ = fs::remove_file(&partial).await;

    let bin_path = dir.join(asset.exe_name);
    if !bin_path.exists() {
        let err = format!(
            "extraction finished but {} was not produced",
            bin_path.display()
        );
        emit_progress(&app, downloaded, total, "error", Some(&err));
        return Err(err);
    }

    // `chmod +x` on Unix so the spawner can exec it.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(&bin_path) {
            let mut perms = meta.permissions();
            perms.set_mode(perms.mode() | 0o755);
            let _ = std::fs::set_permissions(&bin_path, perms);
        }
    }

    emit_progress(&app, downloaded, total, "completed", None);
    Ok(bin_path)
}

fn emit_progress(
    app: &AppHandle,
    downloaded: u64,
    total: u64,
    phase: &str,
    error: Option<&str>,
) {
    let _ = app.emit(
        "inline:binary-download-progress",
        DownloadProgress {
            downloaded,
            total,
            phase: phase.to_string(),
            error: error.map(|s| s.to_string()),
        },
    );
}

/// Flatten a zip archive into `dest`. Upstream releases ship
/// `build/bin/llama-server.exe` with DLLs next to it — we ignore the
/// leading directories and copy every file under the directory that
/// contains `exe_name` into `dest` verbatim.
fn extract_zip(bytes: &[u8], dest: &Path, exe_name: &str) -> Result<(), String> {
    let reader = Cursor::new(bytes);
    let mut archive =
        zip::ZipArchive::new(reader).map_err(|e| format!("open zip: {e}"))?;

    // First pass — find the entry whose basename matches exe_name so we
    // know which prefix to keep.
    let mut keep_prefix: Option<String> = None;
    for i in 0..archive.len() {
        let f = archive
            .by_index(i)
            .map_err(|e| format!("zip index {i}: {e}"))?;
        let name = f.name().to_string();
        if basename(&name).eq_ignore_ascii_case(exe_name) {
            keep_prefix = Some(
                name.rsplit_once('/')
                    .map(|(parent, _)| format!("{parent}/"))
                    .unwrap_or_default(),
            );
            break;
        }
    }
    let prefix = keep_prefix.ok_or_else(|| {
        format!("archive did not contain an entry named {exe_name}")
    })?;

    std::fs::create_dir_all(dest).map_err(|e| format!("mkdir dest: {e}"))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("zip index {i}: {e}"))?;
        let raw_name = entry.name().to_string();
        if entry.is_dir() {
            continue;
        }
        // Only keep files that live in the exe's directory.
        let Some(stripped) = raw_name.strip_prefix(&prefix) else {
            continue;
        };
        if stripped.contains('/') {
            continue;
        }
        let out_path = dest.join(stripped);
        let mut out = std::fs::File::create(&out_path)
            .map_err(|e| format!("create {}: {e}", out_path.display()))?;
        io::copy(&mut entry, &mut out).map_err(|e| format!("copy: {e}"))?;
    }
    Ok(())
}

fn extract_tar_gz(bytes: &[u8], dest: &Path, exe_name: &str) -> Result<(), String> {
    let gz = flate2::read::GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(gz);

    std::fs::create_dir_all(dest).map_err(|e| format!("mkdir dest: {e}"))?;

    // We don't know the prefix up front without a first pass — stream
    // once into memory buffers, then emit. llama.cpp release tarballs
    // are ~30 MB so this is cheap.
    let mut pending: Vec<(String, Vec<u8>)> = Vec::new();
    for entry in archive.entries().map_err(|e| format!("tar entries: {e}"))? {
        let mut entry = entry.map_err(|e| format!("tar entry: {e}"))?;
        let header = entry.header();
        if !header.entry_type().is_file() {
            continue;
        }
        let path = entry
            .path()
            .map_err(|e| format!("tar path: {e}"))?
            .to_string_lossy()
            .into_owned();
        let mut buf = Vec::new();
        io::copy(&mut entry, &mut buf).map_err(|e| format!("tar copy: {e}"))?;
        pending.push((path, buf));
    }

    let prefix = pending
        .iter()
        .find(|(p, _)| basename(p) == exe_name)
        .map(|(p, _)| {
            p.rsplit_once('/')
                .map(|(parent, _)| format!("{parent}/"))
                .unwrap_or_default()
        })
        .ok_or_else(|| format!("archive did not contain an entry named {exe_name}"))?;

    for (path, body) in pending {
        let Some(stripped) = path.strip_prefix(&prefix) else {
            continue;
        };
        if stripped.contains('/') {
            continue;
        }
        let out_path = dest.join(stripped);
        std::fs::write(&out_path, body)
            .map_err(|e| format!("write {}: {e}", out_path.display()))?;
    }
    Ok(())
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}
