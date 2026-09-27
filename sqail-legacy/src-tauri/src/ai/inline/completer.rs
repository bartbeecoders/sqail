//! Streaming FIM completer for the inline AI feature.
//!
//! On `start_completion`:
//! 1. Allocate a `request_id`.
//! 2. Register a cancel `oneshot::Sender` keyed by that id.
//! 3. Spawn a tokio task that POSTs to `http://127.0.0.1:{port}/infill`
//!    with `stream: true`.
//! 4. Parse the Server-Sent-Events stream, emitting `inline:chunk` per
//!    token and `inline:done` on the final frame.
//! 5. On cancel / error / done, remove the registry entry.
//!
//! We also run a **client-side safety fence**: truncate the running
//! output at the first `;` or `\n\n`, matching the `stop` array sent to
//! the server. A buggy server that ignores stops still can't produce
//! runaway completions.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use futures_util::StreamExt;
use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter};
use tokio::sync::{oneshot, Mutex};
use uuid::Uuid;

use super::fim::FimConfig;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChunkEvent {
    request_id: String,
    chunk: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DoneEvent {
    request_id: String,
    text: String,
    tokens: u32,
    ttft_ms: u32,
    total_ms: u32,
    stop_reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorEvent {
    request_id: String,
    error: String,
}

/// Owns the per-request cancellation registry. Cheap to clone via `Arc`.
#[derive(Default)]
pub struct CompletionRegistry {
    inner: Mutex<HashMap<Uuid, oneshot::Sender<()>>>,
}

impl CompletionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    async fn register(&self, id: Uuid, tx: oneshot::Sender<()>) {
        self.inner.lock().await.insert(id, tx);
    }

    async fn remove(&self, id: &Uuid) {
        self.inner.lock().await.remove(id);
    }

    /// Fire the cancel signal for `id`. Returns true if a request was
    /// actually cancelled.
    pub async fn cancel(&self, id: Uuid) -> bool {
        match self.inner.lock().await.remove(&id) {
            Some(tx) => {
                let _ = tx.send(());
                true
            }
            None => false,
        }
    }
}

/// One completion request's parameters.
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub prefix: String,
    pub suffix: String,
    pub port: u16,
    pub fim: FimConfig,
}

/// Kick off a streaming FIM request. Returns immediately with the
/// generated request id — the actual work happens in a spawned task
/// that emits `inline:chunk` / `inline:done` / `inline:error`.
pub fn start_completion(
    app: AppHandle,
    registry: Arc<CompletionRegistry>,
    req: CompletionRequest,
) -> String {
    let request_id = Uuid::new_v4();
    let request_id_str = request_id.to_string();

    let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
    let reg_clone = Arc::clone(&registry);

    tokio::spawn(async move {
        reg_clone.register(request_id, cancel_tx).await;
        let result = run_completion(&app, &request_id.to_string(), cancel_rx, req).await;
        reg_clone.remove(&request_id).await;

        if let Err(e) = result {
            // "cancelled" is a normal termination, not an error worth
            // reporting upstream.
            if e != "cancelled" {
                let _ = app.emit(
                    "inline:error",
                    ErrorEvent {
                        request_id: request_id.to_string(),
                        error: e,
                    },
                );
            }
        }
    });

    request_id_str
}

async fn run_completion(
    app: &AppHandle,
    request_id: &str,
    mut cancel_rx: oneshot::Receiver<()>,
    req: CompletionRequest,
) -> Result<(), String> {
    let body = json!({
        "input_prefix": req.prefix,
        "input_suffix": req.suffix,
        "n_predict": req.fim.n_predict,
        "temperature": req.fim.temperature,
        "top_p": req.fim.top_p,
        "top_k": req.fim.top_k,
        "repeat_penalty": req.fim.repeat_penalty,
        "stream": true,
        "stop": req.fim.stops,
        "cache_prompt": true,
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("http client: {e}"))?;

    let url = format!("http://127.0.0.1:{}/infill", req.port);
    let resp = tokio::select! {
        _ = &mut cancel_rx => return Err("cancelled".into()),
        r = client.post(url).json(&body).send() =>
            r.map_err(|e| format!("POST /infill: {e}"))?,
    };

    if !resp.status().is_success() {
        return Err(format!("llama-server returned {}", resp.status()));
    }

    let started = Instant::now();
    let mut ttft_ms: Option<u32> = None;
    let mut text = String::new();
    let mut tokens: u32 = 0;
    let mut stop_reason = String::from("eos");

    let mut stream = resp.bytes_stream();
    let mut buf = Vec::<u8>::new();

    'outer: loop {
        let chunk = tokio::select! {
            _ = &mut cancel_rx => {
                stop_reason = "cancelled".into();
                break 'outer;
            }
            c = stream.next() => c,
        };
        let bytes = match chunk {
            None => break,
            Some(Ok(b)) => b,
            Some(Err(e)) => return Err(format!("stream: {e}")),
        };
        buf.extend_from_slice(&bytes);

        // Server-Sent-Events: frames end in "\n\n", each line is either
        // blank or begins with "data: ".
        while let Some(pos) = find_double_newline(&buf) {
            let frame = buf.drain(..pos + 2).collect::<Vec<u8>>();
            let Ok(frame_str) = std::str::from_utf8(&frame) else { continue };
            for line in frame_str.lines() {
                let Some(payload) = line.strip_prefix("data: ") else { continue };
                let payload = payload.trim();
                if payload.is_empty() || payload == "[DONE]" {
                    continue;
                }
                let val: serde_json::Value = match serde_json::from_str(payload) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let piece = val.get("content").and_then(|c| c.as_str()).unwrap_or("");
                if !piece.is_empty() {
                    if ttft_ms.is_none() {
                        ttft_ms = Some(started.elapsed().as_millis() as u32);
                    }
                    // Client-side fence: truncate at the first stop string.
                    let next = format!("{text}{piece}");
                    if let Some((trimmed, why)) = apply_stops(&next, req.fim.stops) {
                        let added = &trimmed[text.len()..];
                        if !added.is_empty() {
                            tokens += 1;
                            emit_chunk(app, request_id, added);
                        }
                        text = trimmed;
                        stop_reason = why.into();
                        break 'outer;
                    }
                    tokens += 1;
                    emit_chunk(app, request_id, piece);
                    text = next;
                }
                let stopped = val.get("stop").and_then(|b| b.as_bool()).unwrap_or(false);
                if stopped {
                    if let Some(reason) = val
                        .get("stop_type")
                        .and_then(|s| s.as_str())
                        .or_else(|| val.get("stopped_word").and_then(|s| s.as_str()))
                    {
                        stop_reason = reason.to_string();
                    }
                    break 'outer;
                }
            }
        }
    }

    let ttft = ttft_ms.unwrap_or(0);
    let total = started.elapsed().as_millis() as u32;

    let _ = app.emit(
        "inline:done",
        DoneEvent {
            request_id: request_id.to_string(),
            text,
            tokens,
            ttft_ms: ttft,
            total_ms: total,
            stop_reason,
        },
    );
    Ok(())
}

fn emit_chunk(app: &AppHandle, request_id: &str, chunk: &str) {
    let _ = app.emit(
        "inline:chunk",
        ChunkEvent {
            request_id: request_id.to_string(),
            chunk: chunk.to_string(),
        },
    );
}

fn find_double_newline(buf: &[u8]) -> Option<usize> {
    buf.windows(2).position(|w| w == b"\n\n")
}

/// If `text` contains any of the stop strings, return the text truncated
/// at the first occurrence + the stop that fired. Stops never appear in
/// the final output.
fn apply_stops(text: &str, stops: &[&str]) -> Option<(String, &'static str)> {
    // Exactly two built-in stops today; a linear scan is fine.
    let mut best: Option<(usize, &'static str)> = None;
    for stop in stops {
        if let Some(idx) = text.find(stop) {
            match best {
                Some((b, _)) if b <= idx => {}
                _ => {
                    // Only the two built-ins are acceptable here — we
                    // need a 'static lifetime for the reason string.
                    let tag: &'static str = match *stop {
                        ";" => "semicolon",
                        "\n\n" => "double_newline",
                        _ => "stop",
                    };
                    best = Some((idx, tag));
                }
            }
        }
    }
    best.map(|(idx, why)| (text[..idx].to_string(), why))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_at_semicolon() {
        let (t, why) = apply_stops("SELECT 1; more", &[";", "\n\n"]).unwrap();
        assert_eq!(t, "SELECT 1");
        assert_eq!(why, "semicolon");
    }

    #[test]
    fn stop_at_blank_line() {
        let (t, why) = apply_stops("SELECT 1\n\nSELECT 2", &[";", "\n\n"]).unwrap();
        assert_eq!(t, "SELECT 1");
        assert_eq!(why, "double_newline");
    }

    #[test]
    fn no_stop() {
        assert!(apply_stops("SELECT 1", &[";", "\n\n"]).is_none());
    }
}
