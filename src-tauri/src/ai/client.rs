use futures_util::StreamExt;
use serde_json::json;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::ai::prompt::build_system_prompt;
use crate::ai::provider::{AiProviderConfig, AiProviderType};

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct StreamChunkPayload {
    request_id: String,
    chunk: String,
    done: bool,
}

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct StreamErrorPayload {
    request_id: String,
    error: String,
}

/// Spawn a streaming AI request. Emits `ai:stream-chunk`, `ai:stream-done`, `ai:stream-error` events.
pub async fn stream_ai_response(
    app_handle: AppHandle,
    request_id: String,
    config: &AiProviderConfig,
    user_message: &str,
    flow: &str,
    driver: Option<&str>,
    schema_context: Option<&str>,
) {
    let system_prompt = build_system_prompt(flow, driver, schema_context);

    let result = match config.provider {
        AiProviderType::Claude => {
            stream_claude(&app_handle, &request_id, config, &system_prompt, user_message).await
        }
        AiProviderType::OpenAi | AiProviderType::OpenAiCompatible => {
            stream_openai(&app_handle, &request_id, config, &system_prompt, user_message).await
        }
        AiProviderType::Minimax => {
            stream_minimax(&app_handle, &request_id, config, &system_prompt, user_message).await
        }
        AiProviderType::Zai => {
            stream_zai(&app_handle, &request_id, config, &system_prompt, user_message).await
        }
        AiProviderType::ClaudeCodeCli => {
            stream_claude_code_cli(&app_handle, &request_id, &system_prompt, user_message).await
        }
        AiProviderType::LmStudio => {
            stream_lm_studio(&app_handle, &request_id, config, &system_prompt, user_message).await
        }
    };

    if let Err(e) = result {
        let _ = app_handle.emit(
            "ai:stream-error",
            StreamErrorPayload {
                request_id: request_id.clone(),
                error: e,
            },
        );
    }
}

/// Test connectivity for an AI provider. Returns Ok(message) on success.
pub async fn test_ai_provider(config: &AiProviderConfig) -> Result<String, String> {
    match config.provider {
        AiProviderType::Claude => test_claude(config).await,
        AiProviderType::OpenAi | AiProviderType::OpenAiCompatible => test_openai(config).await,
        AiProviderType::Minimax => test_minimax(config).await,
        AiProviderType::Zai => test_zai(config).await,
        AiProviderType::ClaudeCodeCli => test_claude_code_cli().await,
        AiProviderType::LmStudio => test_lm_studio(config).await,
    }
}

// ── Claude (Anthropic) ─────────────────────────────────────

async fn stream_claude(
    app_handle: &AppHandle,
    request_id: &str,
    config: &AiProviderConfig,
    system_prompt: &str,
    user_message: &str,
) -> Result<(), String> {
    let client = reqwest::Client::new();
    let url = config
        .base_url
        .as_deref()
        .unwrap_or("https://api.anthropic.com/v1/messages");

    let body = json!({
        "model": config.model,
        "max_tokens": 4096,
        "stream": true,
        "system": system_prompt,
        "messages": [
            { "role": "user", "content": user_message }
        ]
    });

    let resp = client
        .post(url)
        .header("x-api-key", &config.api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Claude API error ({status}): {text}"));
    }

    stream_sse_events(app_handle, request_id, resp, extract_claude_delta).await
}

async fn test_claude(config: &AiProviderConfig) -> Result<String, String> {
    let client = reqwest::Client::new();
    let url = config
        .base_url
        .as_deref()
        .unwrap_or("https://api.anthropic.com/v1/messages");

    let body = json!({
        "model": config.model,
        "max_tokens": 16,
        "messages": [{ "role": "user", "content": "Hi" }]
    });

    let resp = client
        .post(url)
        .header("x-api-key", &config.api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?;

    if resp.status().is_success() {
        Ok(format!("Connected to Claude (model: {})", config.model))
    } else {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        Err(format!("Claude API error ({status}): {text}"))
    }
}

// ── OpenAI / OpenAI-compatible ─────────────────────────────

fn resolve_openai_url(base: Option<&str>, default: &str) -> String {
    let base = base.unwrap_or(default);
    if base.ends_with("/chat/completions") || base.ends_with("/chat/completions/") {
        base.to_string()
    } else {
        format!("{}/chat/completions", base.trim_end_matches('/'))
    }
}

fn build_openai_body(
    config: &AiProviderConfig,
    system_prompt: &str,
    user_message: &str,
    stream: bool,
) -> serde_json::Value {
    json!({
        "model": config.model,
        "stream": stream,
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": user_message }
        ]
    })
}

async fn stream_openai(
    app_handle: &AppHandle,
    request_id: &str,
    config: &AiProviderConfig,
    system_prompt: &str,
    user_message: &str,
) -> Result<(), String> {
    let client = reqwest::Client::new();
    let url = resolve_openai_url(config.base_url.as_deref(), "https://api.openai.com/v1");
    let body = build_openai_body(config, system_prompt, user_message, true);

    let resp = client
        .post(&url)
        .header("authorization", format!("Bearer {}", config.api_key))
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("OpenAI API error ({status}): {text}"));
    }

    stream_sse_events(app_handle, request_id, resp, extract_openai_delta).await
}

async fn test_openai(config: &AiProviderConfig) -> Result<String, String> {
    let client = reqwest::Client::new();
    let url = resolve_openai_url(config.base_url.as_deref(), "https://api.openai.com/v1");
    let body = json!({
        "model": config.model,
        "max_tokens": 16,
        "messages": [{ "role": "user", "content": "Hi" }]
    });

    let resp = client
        .post(&url)
        .header("authorization", format!("Bearer {}", config.api_key))
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?;

    if resp.status().is_success() {
        Ok(format!("Connected to OpenAI (model: {})", config.model))
    } else {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        Err(format!("OpenAI API error ({status}): {text}"))
    }
}

// ── Minimax ────────────────────────────────────────────────
// OpenAI-compatible API at https://api.minimax.io/v1

async fn stream_minimax(
    app_handle: &AppHandle,
    request_id: &str,
    config: &AiProviderConfig,
    system_prompt: &str,
    user_message: &str,
) -> Result<(), String> {
    let client = reqwest::Client::new();
    let url = resolve_openai_url(config.base_url.as_deref(), "https://api.minimax.io/v1");
    let body = build_openai_body(config, system_prompt, user_message, true);

    let resp = client
        .post(&url)
        .header("authorization", format!("Bearer {}", config.api_key))
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Minimax API error ({status}): {text}"));
    }

    stream_sse_events(app_handle, request_id, resp, extract_openai_delta).await
}

async fn test_minimax(config: &AiProviderConfig) -> Result<String, String> {
    let client = reqwest::Client::new();
    let url = resolve_openai_url(config.base_url.as_deref(), "https://api.minimax.io/v1");
    let body = json!({
        "model": config.model,
        "max_tokens": 16,
        "messages": [{ "role": "user", "content": "Hi" }]
    });

    let resp = client
        .post(&url)
        .header("authorization", format!("Bearer {}", config.api_key))
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?;

    if resp.status().is_success() {
        Ok(format!("Connected to Minimax (model: {})", config.model))
    } else {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        Err(format!("Minimax API error ({status}): {text}"))
    }
}

// ── Z.ai ───────────────────────────────────────────────────
// OpenAI-compatible API at https://api.z.ai/api/paas/v4/

async fn stream_zai(
    app_handle: &AppHandle,
    request_id: &str,
    config: &AiProviderConfig,
    system_prompt: &str,
    user_message: &str,
) -> Result<(), String> {
    let client = reqwest::Client::new();
    let url = resolve_openai_url(config.base_url.as_deref(), "https://api.z.ai/api/paas/v4");
    let body = build_openai_body(config, system_prompt, user_message, true);

    let resp = client
        .post(&url)
        .header("authorization", format!("Bearer {}", config.api_key))
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Z.ai API error ({status}): {text}"));
    }

    stream_sse_events(app_handle, request_id, resp, extract_openai_delta).await
}

async fn test_zai(config: &AiProviderConfig) -> Result<String, String> {
    let client = reqwest::Client::new();
    let url = resolve_openai_url(config.base_url.as_deref(), "https://api.z.ai/api/paas/v4");
    let body = json!({
        "model": config.model,
        "max_tokens": 16,
        "messages": [{ "role": "user", "content": "Hi" }]
    });

    let resp = client
        .post(&url)
        .header("authorization", format!("Bearer {}", config.api_key))
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?;

    if resp.status().is_success() {
        Ok(format!("Connected to Z.ai (model: {})", config.model))
    } else {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        Err(format!("Z.ai API error ({status}): {text}"))
    }
}

// ── Claude Code CLI ────────────────────────────────────────
// Spawns the `claude` CLI binary as a subprocess and streams stdout.

async fn stream_claude_code_cli(
    app_handle: &AppHandle,
    request_id: &str,
    system_prompt: &str,
    user_message: &str,
) -> Result<(), String> {
    let full_prompt = format!("{system_prompt}\n\n{user_message}");

    let mut child = Command::new("claude")
        .arg("--print")
        .arg("--output-format")
        .arg("text")
        .arg(&full_prompt)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn claude CLI: {e}. Is claude installed and in PATH?"))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Failed to capture claude stdout".to_string())?;

    let mut reader = BufReader::new(stdout).lines();
    let mut full_text = String::new();

    while let Some(line) = reader
        .next_line()
        .await
        .map_err(|e| format!("Read error: {e}"))?
    {
        if !full_text.is_empty() {
            full_text.push('\n');
        }
        full_text.push_str(&line);

        let chunk = if full_text.len() == line.len() {
            line.clone()
        } else {
            format!("\n{line}")
        };

        let _ = app_handle.emit(
            "ai:stream-chunk",
            StreamChunkPayload {
                request_id: request_id.to_string(),
                chunk,
                done: false,
            },
        );
    }

    let status = child
        .wait()
        .await
        .map_err(|e| format!("Failed to wait for claude process: {e}"))?;

    if !status.success() {
        let stderr = child.stderr.take();
        let mut err_msg = format!("claude CLI exited with {status}");
        if let Some(stderr) = stderr {
            let mut err_reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = err_reader.next_line().await {
                err_msg.push('\n');
                err_msg.push_str(&line);
            }
        }
        return Err(err_msg);
    }

    let _ = app_handle.emit(
        "ai:stream-done",
        StreamChunkPayload {
            request_id: request_id.to_string(),
            chunk: full_text,
            done: true,
        },
    );

    Ok(())
}

async fn test_claude_code_cli() -> Result<String, String> {
    let output = Command::new("claude")
        .arg("--version")
        .output()
        .await
        .map_err(|e| format!("Failed to run claude CLI: {e}. Is claude installed and in PATH?"))?;

    if output.status.success() {
        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(format!("Claude Code CLI found: {version}"))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(format!("claude CLI error: {stderr}"))
    }
}

// ── LM Studio ──────────────────────────────────────────────
// OpenAI-compatible local server

async fn stream_lm_studio(
    app_handle: &AppHandle,
    request_id: &str,
    config: &AiProviderConfig,
    system_prompt: &str,
    user_message: &str,
) -> Result<(), String> {
    let client = reqwest::Client::new();
    let url = resolve_openai_url(config.base_url.as_deref(), "https://llm.hideterms.com/v1");
    let body = build_openai_body(config, system_prompt, user_message, true);

    let mut req = client
        .post(&url)
        .header("content-type", "application/json");

    // LM Studio may not require an API key when running locally
    if !config.api_key.is_empty() {
        req = req.header("authorization", format!("Bearer {}", config.api_key));
    }

    let resp = req
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("LM Studio API error ({status}): {text}"));
    }

    stream_sse_events(app_handle, request_id, resp, extract_openai_delta).await
}

async fn test_lm_studio(config: &AiProviderConfig) -> Result<String, String> {
    let client = reqwest::Client::new();
    // Hit the /models endpoint to verify connectivity
    let base = config
        .base_url
        .as_deref()
        .unwrap_or("https://llm.hideterms.com/v1");
    let url = format!("{}/models", base.trim_end_matches('/'));

    let mut req = client.get(&url);
    if !config.api_key.is_empty() {
        req = req.header("authorization", format!("Bearer {}", config.api_key));
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}. Is LM Studio running?"))?;

    if resp.status().is_success() {
        let body = resp
            .json::<serde_json::Value>()
            .await
            .unwrap_or(json!({}));
        let count = body
            .get("data")
            .and_then(|d| d.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        Ok(format!("Connected to LM Studio ({count} model(s) available)"))
    } else {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        Err(format!("LM Studio API error ({status}): {text}"))
    }
}

// ── Shared SSE streaming helper ────────────────────────────

async fn stream_sse_events(
    app_handle: &AppHandle,
    request_id: &str,
    resp: reqwest::Response,
    extract_delta: fn(&serde_json::Value) -> Option<String>,
) -> Result<(), String> {
    let mut full_text = String::new();
    let mut stream = resp.bytes_stream();
    let mut buffer = String::new();

    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|e| format!("Stream error: {e}"))?;
        buffer.push_str(&String::from_utf8_lossy(&bytes));

        while let Some(line_end) = buffer.find('\n') {
            let line = buffer[..line_end].trim().to_string();
            buffer = buffer[line_end + 1..].to_string();

            if let Some(data) = line.strip_prefix("data: ") {
                if data == "[DONE]" {
                    continue;
                }
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(text) = extract_delta(&parsed) {
                        full_text.push_str(&text);
                        let _ = app_handle.emit(
                            "ai:stream-chunk",
                            StreamChunkPayload {
                                request_id: request_id.to_string(),
                                chunk: text,
                                done: false,
                            },
                        );
                    }
                }
            }
        }
    }

    let _ = app_handle.emit(
        "ai:stream-done",
        StreamChunkPayload {
            request_id: request_id.to_string(),
            chunk: full_text,
            done: true,
        },
    );

    Ok(())
}

// ── Delta extractors ───────────────────────────────────────

fn extract_claude_delta(json: &serde_json::Value) -> Option<String> {
    if json.get("type")?.as_str()? == "content_block_delta" {
        return json
            .get("delta")?
            .get("text")?
            .as_str()
            .map(|s| s.to_string());
    }
    None
}

fn extract_openai_delta(json: &serde_json::Value) -> Option<String> {
    json.get("choices")?
        .get(0)?
        .get("delta")?
        .get("content")?
        .as_str()
        .map(|s| s.to_string())
}
