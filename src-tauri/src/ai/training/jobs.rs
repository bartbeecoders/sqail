//! Training job lifecycle.
//!
//! `start` is non-blocking: it generates the dataset, spawns the Python
//! trainer, wires up stdout parsing (one JSON line per progress tick),
//! and returns a job id that the UI can subscribe to.
//!
//! Events emitted on the app handle:
//!
//! * `training:update`  — whenever the job's state transitions or its
//!   progress numbers change. Payload is the full `TrainingJob`.
//! * `training:log`     — unstructured stderr lines from the Python
//!   process. Shown in a collapsible panel in the UI.
//! * `training:done`    — the job finished (either successfully or not).
//!   The trained-model list should be refreshed after this.
//!
//! The Python script is `scripts/train_sql_lora.py`; see its header for
//! the exact CLI + JSONL protocol.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::Notify;

use super::dataset::{self, DatasetOptions, DatasetStats};
use super::env;
use super::models::{self, TrainedModel};
use crate::ai::inline::models as inline_models;
use crate::db::connections::Driver;
use crate::metadata::ObjectMetadata;
use crate::pool::DbPool;

/// Phases a job transitions through.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrainingPhase {
    Queued,
    PreparingDataset,
    Training,
    Saving,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrainingHyperparams {
    #[serde(default = "default_epochs")]
    pub epochs: f32,
    #[serde(default = "default_lr")]
    pub learning_rate: f32,
    #[serde(default = "default_rank")]
    pub lora_rank: u32,
    #[serde(default = "default_alpha")]
    pub lora_alpha: u32,
    #[serde(default = "default_max_steps")]
    pub max_steps: i32,
    #[serde(default = "default_batch")]
    pub batch_size: u32,
}

fn default_epochs() -> f32 {
    3.0
}
fn default_lr() -> f32 {
    2e-4
}
fn default_rank() -> u32 {
    8
}
fn default_alpha() -> u32 {
    16
}
fn default_max_steps() -> i32 {
    -1
}
fn default_batch() -> u32 {
    1
}

impl Default for TrainingHyperparams {
    fn default() -> Self {
        Self {
            epochs: default_epochs(),
            learning_rate: default_lr(),
            lora_rank: default_rank(),
            lora_alpha: default_alpha(),
            max_steps: default_max_steps(),
            batch_size: default_batch(),
        }
    }
}

/// The user-visible training job record. Serialised into
/// `training:update` events and surfaced via `training_list_jobs`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrainingJob {
    pub id: String,
    pub connection_id: String,
    pub connection_name: String,
    pub base_model_id: String,
    pub base_model_hf_id: String,
    pub phase: TrainingPhase,
    /// 0.0 – 1.0. Best-effort (sum of dataset-gen + training progress).
    pub progress: f32,
    pub step: Option<u64>,
    pub total_steps: Option<u64>,
    pub loss: Option<f64>,
    pub message: Option<String>,
    pub error: Option<String>,
    pub dataset_stats: Option<DatasetStats>,
    pub hyperparams: TrainingHyperparams,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub output_model_id: Option<String>,
    /// Absolute path to the on-disk trainer log. Written even on
    /// failure so the user can always inspect what happened.
    #[serde(default)]
    pub log_path: Option<String>,
}

#[derive(Clone)]
pub struct StartArgs {
    pub connection_id: String,
    pub connection_name: String,
    pub base_model_id: String,
    pub base_model_hf_id: String,
    pub driver: Driver,
    pub dialect_label: String,
    pub pool: DbPool,
    pub metadata: Vec<ObjectMetadata>,
    pub options: DatasetOptions,
    pub hyperparams: TrainingHyperparams,
}

/// Spawn a training job. Returns the job id (also used as the trained
/// model id on success).
pub async fn start(
    app: AppHandle,
    state: super::state::TrainingState,
    app_data: PathBuf,
    script_path: PathBuf,
    args: StartArgs,
) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let job = TrainingJob {
        id: id.clone(),
        connection_id: args.connection_id.clone(),
        connection_name: args.connection_name.clone(),
        base_model_id: args.base_model_id.clone(),
        base_model_hf_id: args.base_model_hf_id.clone(),
        phase: TrainingPhase::Queued,
        progress: 0.0,
        step: None,
        total_steps: None,
        loss: None,
        message: Some("Queued".into()),
        error: None,
        dataset_stats: None,
        hyperparams: args.hyperparams.clone(),
        started_at: chrono::Utc::now().to_rfc3339(),
        finished_at: None,
        output_model_id: None,
        log_path: None,
    };
    state.jobs.lock().await.insert(id.clone(), job.clone());
    let cancel = Arc::new(Notify::new());
    state.cancels.lock().await.insert(id.clone(), cancel.clone());
    let _ = app.emit("training:update", &job);

    let app_clone = app.clone();
    let state_clone = state.clone();
    let id_clone = id.clone();

    tokio::spawn(async move {
        let outcome = run_job(
            app_clone.clone(),
            state_clone.clone(),
            app_data,
            script_path,
            id_clone.clone(),
            args,
            cancel,
        )
        .await;

        // Clear the cancel slot; the job struct stays in `jobs` so the UI
        // can inspect the final state.
        state_clone.cancels.lock().await.remove(&id_clone);

        let final_job = state_clone.jobs.lock().await.get(&id_clone).cloned();
        let _ = app_clone.emit("training:done", &final_job);
        if let Err(e) = outcome {
            log::warn!("training job {id_clone} failed: {e}");
        }
    });

    id
}

pub async fn cancel(state: &super::state::TrainingState, job_id: &str) -> bool {
    let cancels = state.cancels.lock().await;
    if let Some(n) = cancels.get(job_id) {
        n.notify_waiters();
        true
    } else {
        false
    }
}

async fn run_job(
    app: AppHandle,
    state: super::state::TrainingState,
    app_data: PathBuf,
    script_path: PathBuf,
    id: String,
    args: StartArgs,
    cancel: Arc<Notify>,
) -> Result<(), String> {
    // 1. Dataset generation.
    transition(&app, &state, &id, |j| {
        j.phase = TrainingPhase::PreparingDataset;
        j.message = Some("Generating dataset".into());
        j.progress = 0.0;
    })
    .await;

    let job_dir = dataset::training_dir(&app_data, &args.connection_id).join(&id);
    tokio::fs::create_dir_all(&job_dir)
        .await
        .map_err(|e| format!("mkdir job dir: {e}"))?;
    let dataset_path = job_dir.join("dataset.jsonl");
    let log_path = job_dir.join("trainer.log");

    // Open the log file up front and record where it lives on the job —
    // so the UI can surface the path even if the trainer explodes before
    // writing anything. We write a header line so the file is always
    // non-empty and easy to spot in a file manager.
    let log_writer = Arc::new(tokio::sync::Mutex::new(
        tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&log_path)
            .await
            .map_err(|e| format!("open log: {e}"))?,
    ));
    {
        let mut w = log_writer.lock().await;
        let header = format!(
            "# sqail training log\n# job: {id}\n# base_model: {}\n# connection: {}\n# started: {}\n\n",
            args.base_model_id,
            args.connection_name,
            chrono::Utc::now().to_rfc3339(),
        );
        let _ = w.write_all(header.as_bytes()).await;
        let _ = w.flush().await;
    }

    let log_path_str = log_path.to_string_lossy().to_string();
    transition(&app, &state, &id, |j| {
        j.log_path = Some(log_path_str.clone());
    })
    .await;

    let app_for_progress = app.clone();
    let state_for_progress = state.clone();
    let id_for_progress = id.clone();
    let stats = dataset::build(
        args.pool.clone(),
        &args.driver,
        &dataset_path,
        &args.options,
        &args.metadata,
        &args.connection_id,
        &args.dialect_label,
        move |cur, total, msg| {
            let pct = if total == 0 {
                0.0
            } else {
                (cur as f32 / total as f32) * 0.2
            };
            let app2 = app_for_progress.clone();
            let state2 = state_for_progress.clone();
            let id2 = id_for_progress.clone();
            let msg = msg.to_string();
            tokio::spawn(async move {
                transition(&app2, &state2, &id2, |j| {
                    j.progress = pct;
                    j.message = Some(msg);
                })
                .await;
            });
        },
    )
    .await
    .map_err(|e| {
        finish_failure(&app, &state, &id, format!("dataset: {e}"));
        e
    })?;

    transition(&app, &state, &id, |j| {
        j.dataset_stats = Some(stats.clone());
        j.progress = 0.2;
        j.message = Some(format!(
            "Dataset ready ({} examples)",
            stats.example_count
        ));
    })
    .await;

    // 2. Python trainer.
    let py = env::python_path().ok_or_else(|| {
        finish_failure(&app, &state, &id, "python3 not found on PATH".into());
        "python3 not found".to_string()
    })?;

    let output_model_id = id.clone();
    let output_dir = models::model_dir(&app_data, &output_model_id);
    tokio::fs::create_dir_all(&output_dir)
        .await
        .map_err(|e| format!("mkdir output: {e}"))?;

    transition(&app, &state, &id, |j| {
        j.phase = TrainingPhase::Training;
        j.message = Some("Launching trainer".into());
    })
    .await;

    let mut cmd = Command::new(&py);
    cmd.arg(&script_path)
        .args(["--dataset", &dataset_path.to_string_lossy()])
        .args(["--base-model", &args.base_model_hf_id])
        .args(["--output-dir", &output_dir.to_string_lossy()])
        .args(["--epochs", &format!("{}", args.hyperparams.epochs)])
        .args(["--lr", &format!("{}", args.hyperparams.learning_rate)])
        .args(["--lora-rank", &args.hyperparams.lora_rank.to_string()])
        .args(["--lora-alpha", &args.hyperparams.lora_alpha.to_string()])
        .args(["--max-steps", &args.hyperparams.max_steps.to_string()])
        .args(["--batch-size", &args.hyperparams.batch_size.to_string()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| {
        let msg = format!("spawn python: {e}");
        finish_failure(&app, &state, &id, msg.clone());
        msg
    })?;

    let stdout = child.stdout.take().ok_or_else(|| "stdout".to_string())?;
    let stderr = child.stderr.take().ok_or_else(|| "stderr".to_string())?;

    let state_stdout = state.clone();
    let app_stdout = app.clone();
    let id_stdout = id.clone();
    let log_stdout = log_writer.clone();
    let stdout_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            append_log_line(&log_stdout, "out", &line).await;
            handle_stdout_line(&app_stdout, &state_stdout, &id_stdout, &line).await;
        }
    });

    let app_stderr = app.clone();
    let id_stderr = id.clone();
    let log_stderr = log_writer.clone();
    let stderr_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            append_log_line(&log_stderr, "err", &line).await;
            let _ = app_stderr.emit(
                "training:log",
                serde_json::json!({ "id": id_stderr, "line": line }),
            );
        }
    });

    let wait = async {
        child.wait().await.map_err(|e| format!("wait: {e}"))
    };

    let status = tokio::select! {
        s = wait => s?,
        _ = cancel.notified() => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            finish_cancelled(&app, &state, &id).await;
            return Ok(());
        }
    };

    let _ = stdout_task.await;
    let _ = stderr_task.await;

    {
        let mut w = log_writer.lock().await;
        let _ = w.flush().await;
    }

    if !status.success() {
        let code = status.code();
        let tail = read_log_tail(&log_path, 20)
            .await
            .unwrap_or_default();
        let msg = if tail.is_empty() {
            format!(
                "trainer exited with status {:?} — full log: {}",
                code, log_path_str
            )
        } else {
            format!(
                "trainer exited with status {:?}\n\nLast lines of log ({}):\n{}",
                code, log_path_str, tail
            )
        };
        finish_failure(&app, &state, &id, msg);
        return Ok(());
    }

    // 3. Register the trained model on disk.
    transition(&app, &state, &id, |j| {
        j.phase = TrainingPhase::Saving;
        j.message = Some("Saving model record".into());
        j.progress = 0.98;
    })
    .await;

    let connection_name_clone = args.connection_name.clone();
    let hp_epochs = args.hyperparams.epochs;
    let display_name = format!("{} · {}", args.connection_name, args.base_model_id);
    let trained = TrainedModel {
        id: output_model_id.clone(),
        display_name,
        base_model_id: args.base_model_id.clone(),
        connection_id: args.connection_id.clone(),
        connection_name: connection_name_clone,
        dataset_size: stats.size_bytes,
        example_count: stats.example_count,
        table_count: stats.table_count,
        created_at: chrono::Utc::now().to_rfc3339(),
        adapter_path: output_dir.to_string_lossy().to_string(),
        gguf_path: None,
    };
    models::save(&app_data, &trained)
        .await
        .map_err(|e| format!("save model: {e}"))?;

    transition(&app, &state, &id, |j| {
        j.phase = TrainingPhase::Completed;
        j.progress = 1.0;
        j.message = Some(format!(
            "Done — {:.0} epochs, {} examples",
            hp_epochs, stats.example_count
        ));
        j.output_model_id = Some(output_model_id.clone());
        j.finished_at = Some(chrono::Utc::now().to_rfc3339());
    })
    .await;

    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct TrainerLine {
    phase: Option<String>,
    message: Option<String>,
    step: Option<u64>,
    total_steps: Option<u64>,
    progress: Option<f32>,
    loss: Option<f64>,
    error: Option<String>,
}

async fn handle_stdout_line(
    app: &AppHandle,
    state: &super::state::TrainingState,
    id: &str,
    line: &str,
) {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }
    let Ok(parsed) = serde_json::from_str::<TrainerLine>(trimmed) else {
        // Not JSON — treat as a log line.
        let _ = app.emit(
            "training:log",
            serde_json::json!({ "id": id, "line": line }),
        );
        return;
    };

    transition(app, state, id, |j| {
        if let Some(m) = &parsed.message {
            j.message = Some(m.clone());
        }
        if let Some(s) = parsed.step {
            j.step = Some(s);
        }
        if let Some(t) = parsed.total_steps {
            j.total_steps = Some(t);
        }
        if let Some(l) = parsed.loss {
            j.loss = Some(l);
        }
        if let Some(e) = &parsed.error {
            j.error = Some(e.clone());
        }
        // Map phase to our enum + progress. Trainer owns the 0.2 .. 0.95
        // slice of the overall progress bar.
        if let Some(p) = &parsed.phase {
            match p.as_str() {
                "loading" => {
                    j.phase = TrainingPhase::Training;
                    j.progress = 0.25;
                }
                "training" => {
                    j.phase = TrainingPhase::Training;
                    if let Some(prog) = parsed.progress {
                        j.progress = 0.25 + prog.clamp(0.0, 1.0) * 0.7;
                    }
                }
                "saving" => {
                    j.phase = TrainingPhase::Saving;
                    j.progress = 0.95;
                }
                _ => {}
            }
        }
    })
    .await;
}

/// Merge edits into the job record, store it, and emit a
/// `training:update` event for the UI.
async fn transition(
    app: &AppHandle,
    state: &super::state::TrainingState,
    id: &str,
    edit: impl FnOnce(&mut TrainingJob),
) {
    let mut jobs = state.jobs.lock().await;
    if let Some(job) = jobs.get_mut(id) {
        edit(job);
        let copy = job.clone();
        drop(jobs);
        let _ = app.emit("training:update", &copy);
    }
}

fn finish_failure(
    app: &AppHandle,
    state: &super::state::TrainingState,
    id: &str,
    err: String,
) {
    let state = state.clone();
    let app = app.clone();
    let id = id.to_string();
    tokio::spawn(async move {
        transition(&app, &state, &id, |j| {
            j.phase = TrainingPhase::Failed;
            j.error = Some(err.clone());
            j.finished_at = Some(chrono::Utc::now().to_rfc3339());
        })
        .await;
    });
}

async fn finish_cancelled(
    app: &AppHandle,
    state: &super::state::TrainingState,
    id: &str,
) {
    transition(app, state, id, |j| {
        j.phase = TrainingPhase::Cancelled;
        j.message = Some("Cancelled".into());
        j.finished_at = Some(chrono::Utc::now().to_rfc3339());
    })
    .await;
}

/// Resolve the HF repo id for a given catalog entry. The Python trainer
/// takes the HF id rather than a local GGUF path — fine-tuning a
/// quantised GGUF directly is not supported by peft.
pub fn base_hf_id(model_id: &str) -> Option<String> {
    // The three catalog entries map cleanly onto Hugging Face repos.
    let id = match model_id {
        "qwen-coder-1_5b-q4" => "Qwen/Qwen2.5-Coder-1.5B-Instruct",
        "qwen-coder-3b-q4" => "Qwen/Qwen2.5-Coder-3B-Instruct",
        "deepseek-coder-v2-lite-q4" => "deepseek-ai/DeepSeek-Coder-V2-Lite-Instruct",
        _ => return None,
    };
    Some(id.to_string())
}

/// Convenience — look up the catalog entry and resolve its HF id in one
/// step. Unknown ids yield `None` and the caller must refuse to start.
pub fn lookup_base(model_id: &str) -> Option<(inline_models::ModelEntry, String)> {
    let entry = inline_models::find(model_id)?;
    let hf = base_hf_id(model_id)?;
    Some((entry, hf))
}

/// Append a single line to the shared trainer log. Prefixed with
/// `stream` (`out` / `err`) and a UTC timestamp so a user tailing the
/// file can tell at a glance what came from where.
async fn append_log_line(
    writer: &Arc<tokio::sync::Mutex<tokio::fs::File>>,
    stream: &str,
    line: &str,
) {
    let ts = chrono::Utc::now().format("%H:%M:%S%.3f");
    let formatted = format!("[{ts}][{stream}] {line}\n");
    let mut w = writer.lock().await;
    let _ = w.write_all(formatted.as_bytes()).await;
}

/// Read up to `n` non-empty lines from the tail of the trainer log —
/// used to enrich the failure message in the UI.
async fn read_log_tail(path: &Path, n: usize) -> Result<String, String> {
    let contents = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| format!("read log: {e}"))?;
    let lines: Vec<&str> = contents
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .collect();
    let start = lines.len().saturating_sub(n);
    Ok(lines[start..].join("\n"))
}

/// Read the full trainer log for a given job. Used by the
/// `training_read_log` tauri command so the UI can show history after
/// the in-memory ring buffer has moved on.
pub async fn read_log(
    app_data: &Path,
    connection_id: &str,
    job_id: &str,
) -> Result<String, String> {
    let path = dataset::training_dir(app_data, connection_id)
        .join(job_id)
        .join("trainer.log");
    if !path.exists() {
        return Err(format!("log not found: {}", path.display()));
    }
    tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("read log: {e}"))
}

/// Helper used by `commands::training_preview_dataset` to get a quick
/// estimate of dataset size without actually training.
pub async fn preview(
    pool: DbPool,
    driver: Driver,
    app_data: &Path,
    connection_id: &str,
    dialect_label: &str,
    options: &DatasetOptions,
    metadata: &[ObjectMetadata],
) -> Result<DatasetStats, String> {
    let dir = dataset::training_dir(app_data, connection_id).join("_preview");
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("mkdir preview: {e}"))?;
    let path = dir.join("dataset.jsonl");
    dataset::build(
        pool,
        &driver,
        &path,
        options,
        metadata,
        connection_id,
        dialect_label,
        |_, _, _| {},
    )
    .await
}
