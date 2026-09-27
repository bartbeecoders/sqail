//! The NDJSON that `claude -p --output-format stream-json` and
//! `grok -p --output-format streaming-messages-json` print, as a few events.
//! Both use the Anthropic Messages shapes: `system/init`, `stream_event`
//! (text deltas), whole `assistant` and `user` messages, and a final `result`.

use std::collections::HashSet;

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The CLI's session id, for continuing the conversation.
    Session(String),
    /// Part of the assistant's answer, as it is generated.
    TextDelta(String),
    /// A whole text block whose deltas were not streamed.
    Text(String),
    /// A sqail tool call (name without the MCP prefix).
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        id: String,
        text: String,
        is_error: bool,
    },
    Done {
        session: Option<String>,
        is_error: bool,
        /// The final answer, or why it failed.
        message: Option<String>,
        cost_usd: Option<f64>,
    },
}

/// Tools the CLIs use to find MCP tools; not interesting to show.
const LOOKUP_TOOLS: &[&str] = &["search_tool", "ToolSearch"];

#[derive(Default)]
pub struct Parser {
    /// Messages whose text arrived as deltas (their whole copy is skipped).
    streamed: HashSet<String>,
    current: Option<String>,
    /// Tool calls that are hidden, so are their results.
    hidden: HashSet<String>,
}

impl Parser {
    pub fn line(&mut self, line: &str) -> Vec<Event> {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(String::from);
        match v.get("type").and_then(Value::as_str).unwrap_or("") {
            "system" if s(&v, "subtype").as_deref() == Some("init") => s(&v, "session_id")
                .map(Event::Session)
                .into_iter()
                .collect(),
            "stream_event" => {
                let ev = &v["event"];
                match ev.get("type").and_then(Value::as_str).unwrap_or("") {
                    "message_start" => {
                        self.current = s(&ev["message"], "id");
                        Vec::new()
                    }
                    "content_block_delta" if ev["delta"]["type"] == "text_delta" => {
                        if let Some(id) = &self.current {
                            self.streamed.insert(id.clone());
                        }
                        s(&ev["delta"], "text")
                            .map(Event::TextDelta)
                            .into_iter()
                            .collect()
                    }
                    _ => Vec::new(),
                }
            }
            "assistant" => {
                let msg = &v["message"];
                let streamed = s(msg, "id").is_some_and(|id| self.streamed.contains(&id));
                let mut out = Vec::new();
                for block in msg["content"].as_array().into_iter().flatten() {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") if !streamed => {
                            if let Some(t) = s(block, "text").filter(|t| !t.is_empty()) {
                                out.push(Event::Text(t));
                            }
                        }
                        Some("tool_use") => {
                            let id = s(block, "id").unwrap_or_default();
                            let mut name = s(block, "name").unwrap_or_default();
                            let mut input = block.get("input").cloned().unwrap_or(Value::Null);
                            // Grok calls MCP tools through `use_tool`.
                            if name == "use_tool" {
                                name = s(&input, "tool_name").unwrap_or(name);
                                input = input.get("tool_input").cloned().unwrap_or(Value::Null);
                            }
                            if LOOKUP_TOOLS.contains(&name.as_str()) {
                                self.hidden.insert(id);
                                continue;
                            }
                            out.push(Event::ToolUse {
                                id,
                                name: tool_name(&name).into(),
                                input,
                            });
                        }
                        _ => {}
                    }
                }
                out
            }
            "user" => {
                let mut out = Vec::new();
                for block in v["message"]["content"].as_array().into_iter().flatten() {
                    if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                        continue;
                    }
                    let id = s(block, "tool_use_id").unwrap_or_default();
                    if self.hidden.contains(&id) {
                        continue;
                    }
                    out.push(Event::ToolResult {
                        id,
                        text: result_text(block.get("content").unwrap_or(&Value::Null)),
                        is_error: block
                            .get("is_error")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    });
                }
                out
            }
            "result" => {
                let is_error = v.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                let message = s(&v, "result").filter(|r| !r.is_empty()).or_else(|| {
                    let errors: Vec<String> = v["errors"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|e| e.as_str().map(String::from))
                        .collect();
                    (!errors.is_empty()).then(|| errors.join("; "))
                });
                vec![Event::Done {
                    session: s(&v, "session_id"),
                    is_error,
                    message,
                    cost_usd: v.get("total_cost_usd").and_then(Value::as_f64),
                }]
            }
            _ => Vec::new(),
        }
    }
}

/// `mcp__sqail__run_query` (Claude) and `sqail__run_query` (Grok) → `run_query`.
pub fn tool_name(name: &str) -> &str {
    name.strip_prefix("mcp__sqail__")
        .or_else(|| name.strip_prefix("sqail__"))
        .unwrap_or(name)
}

/// The text of a tool result: a string, a list of `{type: text}` blocks, or
/// Grok's JSON envelope (`{"type": "MCP", "output": {"OkayOutput": …}}`).
fn result_text(content: &Value) -> String {
    match content {
        Value::String(s) => match serde_json::from_str::<Value>(s) {
            Ok(v @ Value::Object(_)) if v.get("output").is_some() => result_text(&v["output"]),
            _ => s.clone(),
        },
        Value::Array(items) => items.iter().map(result_text).collect::<Vec<_>>().join("\n"),
        Value::Object(o) => {
            if let Some(t) = o.get("text").and_then(Value::as_str) {
                return t.to_string();
            }
            // {"OkayOutput": "…"}, {"type": "content", "content": {…}} and
            // similar wrappers; `type` is a tag, not content.
            o.iter()
                .filter(|(k, _)| k.as_str() != "type")
                .map(|(_, v)| result_text(v))
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        }
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(lines: &[Value]) -> Vec<Event> {
        let mut p = Parser::default();
        lines.iter().flat_map(|l| p.line(&l.to_string())).collect()
    }

    #[test]
    fn claude_code_stream() {
        let events = parse(&[
            json!({"type": "system", "subtype": "init", "session_id": "s1", "tools": ["mcp__sqail__run_query"]}),
            json!({"type": "stream_event", "event": {"type": "message_start", "message": {"id": "m1"}}}),
            json!({"type": "stream_event", "event": {"type": "content_block_delta", "delta": {"type": "thinking_delta", "thinking": "hm"}}}),
            json!({"type": "stream_event", "event": {"type": "content_block_delta", "delta": {"type": "text_delta", "text": "Let me "}}}),
            json!({"type": "stream_event", "event": {"type": "content_block_delta", "delta": {"type": "text_delta", "text": "look."}}}),
            json!({"type": "assistant", "message": {"id": "m1", "content": [
                {"type": "text", "text": "Let me look."},
                {"type": "tool_use", "id": "t1", "name": "mcp__sqail__run_query", "input": {"sql": "SELECT 1"}}]}}),
            json!({"type": "user", "message": {"content": [
                {"type": "tool_result", "tool_use_id": "t1", "content": [{"type": "text", "text": "?column?\n1"}]}]}}),
            json!({"type": "result", "subtype": "success", "is_error": false, "result": "Done.", "session_id": "s1", "total_cost_usd": 0.01}),
        ]);
        assert_eq!(
            events,
            [
                Event::Session("s1".into()),
                Event::TextDelta("Let me ".into()),
                Event::TextDelta("look.".into()),
                // The whole text block is not repeated after its deltas.
                Event::ToolUse {
                    id: "t1".into(),
                    name: "run_query".into(),
                    input: json!({"sql": "SELECT 1"})
                },
                Event::ToolResult {
                    id: "t1".into(),
                    text: "?column?\n1".into(),
                    is_error: false
                },
                Event::Done {
                    session: Some("s1".into()),
                    is_error: false,
                    message: Some("Done.".into()),
                    cost_usd: Some(0.01)
                },
            ]
        );
    }

    #[test]
    fn grok_stream() {
        let events = parse(&[
            json!({"type": "assistant", "message": {"id": "msg_0", "content": [
                {"type": "text", "text": "Checking."},
                {"type": "tool_use", "id": "c0", "name": "search_tool", "input": {"query": "sqail"}}]}}),
            json!({"type": "user", "message": {"content": [
                {"type": "tool_result", "tool_use_id": "c0", "content": "{\"type\":\"SearchTool\"}"}]}}),
            json!({"type": "assistant", "message": {"id": "msg_1", "content": [
                {"type": "tool_use", "id": "c1", "name": "use_tool", "input": {"tool_name": "sqail__list_tables", "tool_input": {}}}]}}),
            json!({"type": "user", "message": {"content": [
                {"type": "tool_result", "tool_use_id": "c1", "content": "{\"type\":\"MCP\",\"tool_name\":\"list_tables\",\"output\":{\"OkayOutput\":\"sales.orders (table)\"}}"}]}}),
            json!({"type": "user", "message": {"content": [
                {"type": "tool_result", "tool_use_id": "c2", "is_error": true,
                 "content": [{"type": "content", "content": {"type": "text", "text": "User cancelled the execution"}}]}]}}),
            json!({"type": "result", "subtype": "error_during_execution", "is_error": true, "errors": ["cancelled"], "session_id": "g1"}),
        ]);
        assert_eq!(
            events,
            [
                // Without deltas, whole text blocks come through.
                Event::Text("Checking.".into()),
                Event::ToolUse {
                    id: "c1".into(),
                    name: "list_tables".into(),
                    input: json!({})
                },
                Event::ToolResult {
                    id: "c1".into(),
                    text: "sales.orders (table)".into(),
                    is_error: false
                },
                Event::ToolResult {
                    id: "c2".into(),
                    text: "User cancelled the execution".into(),
                    is_error: true
                },
                Event::Done {
                    session: Some("g1".into()),
                    is_error: true,
                    message: Some("cancelled".into()),
                    cost_usd: None
                },
            ]
        );
    }

    #[test]
    fn junk_lines_are_ignored() {
        let mut p = Parser::default();
        assert!(p.line("Warning: no stdin data received").is_empty());
        assert!(p.line("{\"type\": \"rate_limit_event\"}").is_empty());
    }
}
