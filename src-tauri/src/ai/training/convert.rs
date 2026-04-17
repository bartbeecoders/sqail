//! Convert a peft LoRA adapter into the llama.cpp GGUF format.
//!
//! llama-server loads adapters via `--lora <path.gguf>`, so a peft
//! `adapter_model.safetensors` needs to be translated first. llama.cpp
//! ships `convert_lora_to_gguf.py` for exactly this; we reuse that
//! script unmodified (it's actively maintained and handles per-model
//! quirks we don't want to reimplement).
//!
//! Resolution order for the llama.cpp checkout:
//! 1. `SQAIL_LLAMA_CPP_DIR` env (escape hatch).
//! 2. Tauri `resource_dir()/llama.cpp/` when packaged.
//! 3. `<project_root>/.cache/inline-ai/src/llama.cpp/` (dev fallback —
//!    this is what `scripts/fetch-llama-cpp.sh` writes to).
//!
//! Python is resolved via the same probe `training::env` uses. The
//! script needs `torch`, `safetensors`, `numpy`, `transformers`, and
//! `gguf` (the last one ships alongside the script).

use std::path::{Path, PathBuf};
use std::process::Stdio;

use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use super::env;
use super::models::{self, TrainedModel};

/// Find `convert_lora_to_gguf.py`. Returns `None` if neither env, nor
/// packaged resources, nor the dev cache has it.
pub fn resolve_convert_script(app: &AppHandle) -> Option<PathBuf> {
    let candidate = |root: PathBuf| {
        let p = root.join("convert_lora_to_gguf.py");
        if p.exists() {
            Some(p)
        } else {
            None
        }
    };
    if let Ok(v) = std::env::var("SQAIL_LLAMA_CPP_DIR") {
        if let Some(p) = candidate(PathBuf::from(v)) {
            return Some(p);
        }
    }
    if let Ok(res) = app.path().resource_dir() {
        if let Some(p) = candidate(res.join("llama.cpp")) {
            return Some(p);
        }
    }
    // Dev fallback — same layout as scripts/fetch-llama-cpp.sh.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let project_root = manifest.parent().unwrap_or(&manifest).to_path_buf();
    candidate(
        project_root
            .join(".cache")
            .join("inline-ai")
            .join("src")
            .join("llama.cpp"),
    )
}

/// Where the converted GGUF ends up on disk.
pub fn gguf_path(adapter_dir: &Path) -> PathBuf {
    adapter_dir.join("adapter.gguf")
}

/// Run the conversion for a given trained model. Updates `meta.json`
/// with the resulting `gguf_path` and returns it. Idempotent — if the
/// GGUF already exists, it's returned immediately.
///
/// Progress and errors are emitted on `training:log` events keyed by
/// the trained model id, same channel the trainer uses.
pub async fn ensure_converted(
    app: AppHandle,
    app_data: &Path,
    model: &TrainedModel,
) -> Result<PathBuf, String> {
    let adapter_dir = PathBuf::from(&model.adapter_path);
    let out_path = gguf_path(&adapter_dir);
    if out_path.exists() {
        return Ok(out_path);
    }

    let script = resolve_convert_script(&app).ok_or_else(|| {
        "convert_lora_to_gguf.py not found (checkout llama.cpp via \
         scripts/fetch-llama-cpp.sh, or set SQAIL_LLAMA_CPP_DIR)"
            .to_string()
    })?;
    let py = env::python_path().ok_or_else(|| {
        "python3 not found on PATH (set SQAIL_PYTHON)".to_string()
    })?;

    // Resolve base HF id from the catalog entry, same source the
    // trainer used. Fall back to a manual --base-model-id arg.
    let base_hf_id = super::jobs::base_hf_id(&model.base_model_id)
        .ok_or_else(|| format!("unknown base model: {}", model.base_model_id))?;

    let id = model.id.clone();
    emit_log(
        &app,
        &id,
        &format!(
            "Converting adapter to GGUF (base: {}) → {}",
            base_hf_id,
            out_path.display()
        ),
    );

    let mut cmd = Command::new(&py);
    cmd.arg(&script)
        .arg(&adapter_dir)
        .args(["--outfile", &out_path.to_string_lossy()])
        .args(["--outtype", "f16"])
        .args(["--base-model-id", &base_hf_id])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| format!("spawn: {e}"))?;
    let stdout = child.stdout.take().ok_or_else(|| "stdout".to_string())?;
    let stderr = child.stderr.take().ok_or_else(|| "stderr".to_string())?;

    let app_out = app.clone();
    let id_out = id.clone();
    let stdout_task = tokio::spawn(async move {
        let mut r = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = r.next_line().await {
            emit_log(&app_out, &id_out, &line);
        }
    });
    let app_err = app.clone();
    let id_err = id.clone();
    let stderr_task = tokio::spawn(async move {
        let mut r = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = r.next_line().await {
            emit_log(&app_err, &id_err, &line);
        }
    });

    let status = child.wait().await.map_err(|e| format!("wait: {e}"))?;
    let _ = stdout_task.await;
    let _ = stderr_task.await;

    if !status.success() {
        return Err(format!(
            "convert_lora_to_gguf.py exited with {:?}",
            status.code()
        ));
    }
    if !out_path.exists() {
        return Err("convert completed but output file is missing".into());
    }

    // Patch meta.json with the new path so subsequent activations are
    // one-shot.
    let mut updated = model.clone();
    updated.gguf_path = Some(out_path.to_string_lossy().to_string());
    models::save(app_data, &updated).await?;

    emit_log(&app, &id, "GGUF conversion complete");
    Ok(out_path)
}

fn emit_log(app: &AppHandle, id: &str, line: &str) {
    let _ = app.emit(
        "training:log",
        serde_json::json!({ "id": id, "line": line }),
    );
}
