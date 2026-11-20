//! Running Claude Code or Grok headless, with sqail's MCP server as their
//! only tools, and turning their output into [`Event`]s.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::oneshot;

use super::stream::{Event, Parser};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    #[default]
    ClaudeCode,
    Grok,
}

impl Provider {
    pub const ALL: [Provider; 2] = [Provider::ClaudeCode, Provider::Grok];

    pub fn label(self) -> &'static str {
        match self {
            Provider::ClaudeCode => "Claude Code",
            Provider::Grok => "Grok",
        }
    }

    fn program(self) -> &'static str {
        match self {
            Provider::ClaudeCode => "claude",
            Provider::Grok => "grok",
        }
    }

    /// The setting that overrides where the CLI is, for error messages.
    pub fn path_setting(self) -> &'static str {
        match self {
            Provider::ClaudeCode => "assistant.claude_path",
            Provider::Grok => "assistant.grok_path",
        }
    }
}

/// The tools of `sqail mcp`, as the CLIs name them.
const TOOLS: [&str; 4] = ["list_schemas", "list_tables", "describe_table", "run_query"];

/// Everything needed to run one turn of a conversation.
pub struct Request {
    pub provider: Provider,
    /// The CLI; `None` looks for `claude` / `grok` on PATH.
    pub program: Option<PathBuf>,
    pub model: Option<String>,
    /// Instructions for the model (engine, connection, how to answer).
    pub instructions: String,
    pub prompt: String,
    /// Session to continue (from an earlier [`Event::Session`]).
    pub resume: Option<String>,
    /// Private working directory of this conversation (holds the MCP config).
    pub workdir: PathBuf,
    /// How to start sqail's MCP server: normally `[<sqail exe>, "mcp"]`.
    pub mcp_command: Vec<String>,
    /// Environment for the CLI, which passes it on to the MCP server
    /// (service URL, token, connection). Kept out of files and arguments.
    pub env: Vec<(String, String)>,
    /// Timeouts of the turn.
    pub limits: Limits,
}

pub enum Outcome {
    Finished,
    Stopped,
}

/// How long a turn may take before sqail gives up on the CLI.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// No output at all for this long (e.g. a CLI waiting for a login).
    pub idle: Duration,
    /// The whole turn.
    pub total: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            idle: Duration::from_secs(5 * 60),
            total: Duration::from_secs(30 * 60),
        }
    }
}

/// Run one turn, sending every event to `on_event`, until the CLI exits or
/// `stop` fires (then the CLI is killed).
pub async fn run(
    req: Request,
    mut on_event: impl FnMut(Event),
    mut stop: oneshot::Receiver<()>,
) -> Result<Outcome> {
    let program = match &req.program {
        Some(p) => p.clone(),
        None => find_program(req.provider.program()).ok_or_else(|| {
            anyhow!(
                "{} (`{}`) was not found on PATH. Install it, or set {} in settings.toml.",
                req.provider.label(),
                req.provider.program(),
                req.provider.path_setting()
            )
        })?,
    };
    std::fs::create_dir_all(&req.workdir)
        .with_context(|| format!("creating {}", req.workdir.display()))?;
    let (args, stdin) = arguments(&req)?;
    let mut cmd = tokio::process::Command::new(&program);
    cmd.args(&args)
        .current_dir(&req.workdir)
        .envs(req.env.iter().map(|(k, v)| (k, v)))
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd
        .spawn()
        .with_context(|| format!("starting {}", program.display()))?;
    // The prompt is written while stdout is read: a CLI that exits at once
    // (unknown flag, old version) then reports its own error, not a broken
    // pipe.
    let writer = match (stdin, child.stdin.take()) {
        (Some(text), Some(mut pipe)) => Some(tokio::spawn(async move {
            // Dropping the pipe closes it, so the CLI stops reading.
            pipe.write_all(text.as_bytes()).await
        })),
        _ => None,
    };
    let mut stdout = BufReader::new(child.stdout.take().context("no stdout")?);
    let mut stderr = child.stderr.take().context("no stderr")?;
    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf).await;
        String::from_utf8_lossy(&buf).into_owned()
    });

    let label = req.provider.label();
    let mut parser = Parser::default();
    let mut done = false;
    // Whether the turn left anything to show in the conversation.
    let mut answered = false;
    // Output that is not the CLI's JSON (e.g. plain text from a CLI that
    // ignored the output format), shown when nothing else came.
    let mut stray = String::new();
    let deadline = tokio::time::Instant::now() + req.limits.total;
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let read = tokio::time::timeout_at(
            deadline.min(tokio::time::Instant::now() + req.limits.idle),
            stdout.read_until(b'\n', &mut buf),
        );
        tokio::select! {
            read = read => match read {
                Ok(Ok(0)) => break,
                Ok(Ok(_)) => {
                    // Lossy: one odd byte must not end the turn.
                    let text = String::from_utf8_lossy(&buf);
                    let line = text.trim_end_matches(['\r', '\n']);
                    let events = parser.line(line);
                    if events.is_empty() && !line.trim_start().starts_with('{') {
                        stray.push_str(line);
                        stray.push('\n');
                        keep_tail(&mut stray, TAIL_CHARS);
                    }
                    for ev in events {
                        done |= matches!(ev, Event::Done { .. });
                        answered |= shows_something(&ev);
                        let ev = match ev {
                            Event::McpFailed { server, status } => {
                                let why = tokio::select! {
                                    why = diagnose_mcp(&req) => why,
                                    _ = &mut stop => {
                                        let _ = child.kill().await;
                                        return Ok(Outcome::Stopped);
                                    }
                                };
                                Event::Error(mcp_failed_message(&server, &status, &why))
                            }
                            ev => ev,
                        };
                        on_event(ev);
                    }
                }
                Ok(Err(e)) => {
                    let _ = child.kill().await;
                    return Err(anyhow!(e).context(format!("reading {label}'s output")));
                }
                Err(_) => {
                    let _ = child.kill().await;
                    let stderr = stderr_tail(stderr_task).await;
                    let what = if tokio::time::Instant::now() >= deadline {
                        format!("did not finish within {}", minutes(req.limits.total))
                    } else {
                        format!(
                            "printed nothing for {} (is it waiting for a sign-in?)",
                            minutes(req.limits.idle)
                        )
                    };
                    bail!("{label} {what}, so sqail stopped it{}", with_tail(&stderr));
                }
            },
            _ = &mut stop => {
                let _ = child.kill().await;
                return Ok(Outcome::Stopped);
            }
        }
    }
    let status = child.wait().await?;
    let stderr = stderr_tail(stderr_task).await;
    let written = match writer {
        Some(w) => w.await.unwrap_or(Ok(())),
        None => Ok(()),
    };
    if !status.success() && !done {
        bail!("{label} exited with {status}{}", with_tail(&stderr));
    }
    if let Err(e) = written
        && e.kind() != std::io::ErrorKind::BrokenPipe
    {
        bail!("sending the question to {label}: {e}{}", with_tail(&stderr));
    }
    if !answered {
        let mut why = stray.trim().to_string();
        if !stderr.is_empty() {
            if !why.is_empty() {
                why.push('\n');
            }
            why.push_str(&stderr);
        }
        keep_tail(&mut why, TAIL_CHARS);
        bail!("No answer received from {label}{}", with_tail(&why));
    }
    Ok(Outcome::Finished)
}

/// The most of a CLI's stderr or stray output shown in an error.
const TAIL_CHARS: usize = 1500;

/// Whether an event leaves something visible in the conversation.
fn shows_something(ev: &Event) -> bool {
    match ev {
        Event::Session(_) => false,
        Event::Done {
            is_error, message, ..
        } => *is_error || message.is_some(),
        Event::TextDelta(t) | Event::Text(t) => !t.is_empty(),
        Event::ToolUse { .. }
        | Event::ToolResult { .. }
        | Event::McpFailed { .. }
        | Event::Error(_) => true,
    }
}

fn minutes(d: Duration) -> String {
    match d.as_secs() {
        s if s < 120 => format!("{s} seconds"),
        s => format!("{} minutes", s / 60),
    }
}

/// The end of the CLI's stderr, once it is closed.
async fn stderr_tail(task: tokio::task::JoinHandle<String>) -> String {
    // A grandchild (the MCP server) may keep stderr open after a kill.
    let text = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    let mut text = text.trim().to_string();
    keep_tail(&mut text, TAIL_CHARS);
    text
}

/// Keep the last `max` characters of `s`.
fn keep_tail(s: &mut String, max: usize) {
    let n = s.chars().count();
    if n > max {
        let cut = s.char_indices().nth(n - max).map_or(0, |(i, _)| i);
        s.drain(..cut);
    }
}

fn with_tail(tail: &str) -> String {
    if tail.is_empty() {
        ".".into()
    } else {
        format!(":\n{tail}")
    }
}

fn mcp_failed_message(server: &str, status: &str, why: &str) -> String {
    let what = if server == "sqail" {
        "sqail's database tools did not start".to_string()
    } else {
        format!("The MCP server `{server}` did not start")
    };
    format!(
        "{what} (status: {status}), so the assistant cannot see the database{}",
        with_tail(why)
    )
}

/// Why `sqail mcp` fails: start it the way the CLI does, with no input, and
/// return its error. It connects to the service before it reads anything,
/// so a stopped service, a rejected token or a deleted connection shows here.
async fn diagnose_mcp(req: &Request) -> String {
    let Some((exe, args)) = req.mcp_command.split_first() else {
        return "no MCP server command".into();
    };
    let mut cmd = tokio::process::Command::new(exe);
    cmd.args(args)
        .current_dir(&req.workdir)
        .envs(req.env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = match tokio::time::timeout(Duration::from_secs(20), cmd.output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => return format!("starting {exe}: {e}"),
        Err(_) => return "it did not finish starting within 20 seconds".into(),
    };
    let mut text = String::from_utf8_lossy(&out.stderr).trim().to_string();
    keep_tail(&mut text, TAIL_CHARS);
    if !text.is_empty() {
        text
    } else if out.status.success() {
        "it starts when sqail runs it directly; the CLI's own logs may say more".into()
    } else {
        format!("it exited with {}", out.status)
    }
}

/// The CLI arguments, and the prompt when it goes through stdin. Also writes
/// the MCP configuration into the working directory.
fn arguments(req: &Request) -> Result<(Vec<String>, Option<String>)> {
    let (exe, mcp_args) = req
        .mcp_command
        .split_first()
        .ok_or_else(|| anyhow!("no MCP server command"))?;
    let s = |v: &str| v.to_string();
    Ok(match req.provider {
        Provider::ClaudeCode => {
            let config = serde_json::json!({
                "mcpServers": { "sqail": { "command": exe, "args": mcp_args } }
            });
            let path = req.workdir.join("sqail-mcp.json");
            std::fs::write(&path, serde_json::to_vec_pretty(&config)?)?;
            let allowed: Vec<String> = TOOLS.iter().map(|t| format!("mcp__sqail__{t}")).collect();
            let mut args = vec![
                s("-p"),
                s("--output-format"),
                s("stream-json"),
                s("--verbose"),
                s("--include-partial-messages"),
                // No built-in tools (shell, files, web): only sqail's.
                s("--tools"),
                s(""),
                s("--mcp-config"),
                path.to_string_lossy().into_owned(),
                s("--strict-mcp-config"),
                s("--allowedTools"),
                allowed.join(","),
                s("--append-system-prompt"),
                req.instructions.clone(),
            ];
            if let Some(id) = &req.resume {
                args.extend([s("--resume"), id.clone()]);
            }
            if let Some(m) = req.model.as_deref().filter(|m| !m.is_empty()) {
                args.extend([s("--model"), m.to_string()]);
            }
            // The prompt goes through stdin: no length or quoting limits.
            (args, Some(req.prompt.clone()))
        }
        Provider::Grok => {
            // Grok reads MCP servers from the project config of its working
            // directory, which is this conversation's private folder.
            let mut server = toml::Table::new();
            server.insert("command".into(), toml::Value::String(exe.clone()));
            server.insert(
                "args".into(),
                toml::Value::Array(mcp_args.iter().cloned().map(toml::Value::String).collect()),
            );
            server.insert("enabled".into(), toml::Value::Boolean(true));
            let mut servers = toml::Table::new();
            servers.insert("sqail".into(), toml::Value::Table(server));
            let mut root = toml::Table::new();
            root.insert("mcp_servers".into(), toml::Value::Table(servers));
            let dir = req.workdir.join(".grok");
            std::fs::create_dir_all(&dir)?;
            std::fs::write(dir.join("config.toml"), toml::to_string(&root)?)?;
            // Grok has no system-prompt append; the first turn carries the
            // instructions, later turns continue that session.
            let prompt = if req.resume.is_some() {
                req.prompt.clone()
            } else {
                format!("{}\n\n{}", req.instructions, req.prompt)
            };
            let mut args = vec![
                // One argument, so a prompt starting with `-` is not a flag.
                format!("--single={prompt}"),
                s("--trust"),
                s("--output-format"),
                s("streaming-messages-json"),
                s("--include-partial-messages"),
                // Only the built-ins that reach MCP tools; `dontAsk` denies
                // everything not explicitly allowed below (including other
                // MCP servers from the user's own Grok config).
                s("--tools"),
                s("search_tool,use_tool"),
                s("--permission-mode"),
                s("dontAsk"),
            ];
            for t in TOOLS {
                args.extend([s("--allow"), format!("sqail__{t}")]);
            }
            if req.resume.is_some() {
                // Each conversation has its own folder, so "the most recent
                // session here" is this conversation.
                args.push(s("--continue"));
            }
            if let Some(m) = req.model.as_deref().filter(|m| !m.is_empty()) {
                args.extend([s("--model"), m.to_string()]);
            }
            (args, None)
        }
    })
}

/// `name` on PATH. On Windows also `name.exe` / `.cmd` / `.bat`, and an npm
/// `.cmd` shim is resolved to the native executable it starts, because
/// batch files cannot take multi-line arguments.
pub fn find_program(name: &str) -> Option<PathBuf> {
    let exts: &[&str] = if cfg!(windows) {
        &["exe", "cmd", "bat"]
    } else {
        &[""]
    };
    let paths = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&paths) {
        for ext in exts {
            let p = if ext.is_empty() {
                dir.join(name)
            } else {
                dir.join(format!("{name}.{ext}"))
            };
            if p.is_file() {
                if matches!(*ext, "cmd" | "bat")
                    && let Some(native) = shim_target(&p)
                {
                    return Some(native);
                }
                return Some(p);
            }
        }
    }
    None
}

/// The `.exe` an npm `.cmd` shim runs (`"%dp0%\node_modules\…\x.exe" %*`).
fn shim_target(shim: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(shim).ok()?;
    let rel = shim_exe(&text)?;
    let p = shim.parent()?.join(rel);
    p.is_file().then_some(p)
}

fn shim_exe(text: &str) -> Option<String> {
    for marker in ["%dp0%\\", "%~dp0\\", "%~dp0"] {
        for (i, _) in text.match_indices(marker) {
            let rest = &text[i + marker.len()..];
            let end = rest.find(['"', ' ', '\r', '\n']).unwrap_or(rest.len());
            let rel = &rest[..end];
            if rel.to_ascii_lowercase().ends_with(".exe") {
                return Some(rel.replace('\\', std::path::MAIN_SEPARATOR_STR));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(provider: Provider, dir: &Path, resume: Option<&str>) -> Request {
        Request {
            provider,
            program: None,
            model: Some("m1".into()),
            instructions: "INSTR".into(),
            prompt: "line one\nline two".into(),
            resume: resume.map(String::from),
            workdir: dir.to_path_buf(),
            mcp_command: vec!["/opt/sqail/sqail".into(), "mcp".into()],
            env: vec![],
            limits: Limits::default(),
        }
    }

    fn after<'a>(args: &'a [String], flag: &str) -> Vec<&'a str> {
        args.windows(2)
            .filter(|w| w[0] == flag)
            .map(|w| w[1].as_str())
            .collect()
    }

    #[test]
    fn claude_code_gets_only_sqail_tools_and_the_prompt_on_stdin() {
        let dir = tempfile::tempdir().unwrap();
        let (args, stdin) = arguments(&req(Provider::ClaudeCode, dir.path(), Some("s1"))).unwrap();
        assert_eq!(stdin.as_deref(), Some("line one\nline two"));
        assert_eq!(after(&args, "--tools"), [""]);
        assert!(args.contains(&"--strict-mcp-config".to_string()));
        assert_eq!(
            after(&args, "--allowedTools"),
            [
                "mcp__sqail__list_schemas,mcp__sqail__list_tables,mcp__sqail__describe_table,mcp__sqail__run_query"
            ]
        );
        assert_eq!(after(&args, "--resume"), ["s1"]);
        assert_eq!(after(&args, "--model"), ["m1"]);
        let config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("sqail-mcp.json")).unwrap())
                .unwrap();
        assert_eq!(config["mcpServers"]["sqail"]["command"], "/opt/sqail/sqail");
        assert_eq!(config["mcpServers"]["sqail"]["args"][0], "mcp");
    }

    #[test]
    fn grok_is_locked_down_to_sqail_tools() {
        let dir = tempfile::tempdir().unwrap();
        let (args, stdin) = arguments(&req(Provider::Grok, dir.path(), None)).unwrap();
        assert!(stdin.is_none());
        assert_eq!(args[0], "--single=INSTR\n\nline one\nline two");
        assert_eq!(after(&args, "--tools"), ["search_tool,use_tool"]);
        assert_eq!(after(&args, "--permission-mode"), ["dontAsk"]);
        assert_eq!(after(&args, "--allow").len(), 4);
        assert!(!args.contains(&"--continue".to_string()));
        let config: toml::Table =
            toml::from_str(&std::fs::read_to_string(dir.path().join(".grok/config.toml")).unwrap())
                .unwrap();
        assert_eq!(
            config["mcp_servers"]["sqail"]["command"].as_str(),
            Some("/opt/sqail/sqail")
        );

        let (args, _) = arguments(&req(Provider::Grok, dir.path(), Some("g1"))).unwrap();
        assert!(args.contains(&"--continue".to_string()));
        assert_eq!(args[0], "--single=line one\nline two");
    }

    /// An executable shell script in `dir`.
    #[cfg(unix)]
    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// One turn of a fake `claude` whose script is `body`.
    #[cfg(unix)]
    fn turn(body: &str, edit: impl FnOnce(&mut Request)) -> (Result<Outcome>, Vec<Event>) {
        let dir = tempfile::tempdir().unwrap();
        let mut r = req(Provider::ClaudeCode, &dir.path().join("work"), None);
        r.program = Some(script(dir.path(), "claude", body));
        edit(&mut r);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (_stop, rx) = oneshot::channel();
        let mut events = Vec::new();
        let outcome = rt.block_on(run(r, |e| events.push(e), rx));
        (outcome, events)
    }

    #[cfg(unix)]
    const RESULT: &str = r#"{"type":"result","is_error":false,"result":"fine","session_id":"s"}"#;

    #[cfg(unix)]
    #[test]
    fn a_turn_without_an_answer_says_so_with_the_output() {
        let (outcome, events) = turn(
            "cat > /dev/null; echo 'Hello from plain text'; echo 'some warning' >&2",
            |_| {},
        );
        let err = format!("{:#}", outcome.err().expect("an error"));
        assert!(
            err.starts_with("No answer received from Claude Code"),
            "{err}"
        );
        assert!(err.contains("Hello from plain text"), "{err}");
        assert!(err.contains("some warning"), "{err}");
        assert!(events.is_empty(), "{events:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_cli_that_exits_at_once_reports_its_stderr() {
        // It never reads the prompt, which is larger than a pipe buffer.
        let (outcome, _) = turn("echo 'error: unknown option --foo' >&2; exit 2", |r| {
            r.prompt = "x".repeat(1 << 20);
        });
        let err = format!("{:#}", outcome.err().expect("an error"));
        assert!(err.contains("exited with"), "{err}");
        assert!(err.contains("unknown option --foo"), "{err}");
        assert!(!err.to_lowercase().contains("broken pipe"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn invalid_utf8_does_not_end_the_turn() {
        let (outcome, events) = turn(
            &format!("cat > /dev/null; printf 'bad \\377 byte\\n'; echo '{RESULT}'"),
            |_| {},
        );
        assert!(matches!(outcome, Ok(Outcome::Finished)));
        assert!(
            matches!(events.last(), Some(Event::Done { message: Some(m), .. }) if m == "fine"),
            "{events:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_silent_cli_times_out() {
        let (outcome, _) = turn("echo 'waiting for login' >&2; exec sleep 30", |r| {
            r.limits.idle = Duration::from_millis(300);
        });
        let err = format!("{:#}", outcome.err().expect("an error"));
        assert!(err.contains("printed nothing for 0 seconds"), "{err}");
        assert!(err.contains("waiting for login"), "{err}");

        let (outcome, _) = turn(
            "while true; do echo '{\"type\":\"keepalive\"}'; sleep 0.1; done",
            |r| r.limits.total = Duration::from_millis(500),
        );
        let err = format!("{:#}", outcome.err().expect("an error"));
        assert!(err.contains("did not finish within"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_mcp_server_is_reported_with_its_reason() {
        let init = r#"{"type":"system","subtype":"init","session_id":"s","mcp_servers":[{"name":"sqail","status":"failed"}]}"#;
        let (outcome, events) = turn(
            &format!("cat > /dev/null; echo '{init}'; echo '{RESULT}'"),
            |r| {
                let mcp = script(
                    r.workdir.parent().unwrap(),
                    "fake-sqail",
                    "echo \"sqail mcp: looking up the connection: $SQAIL_TEST_WHY\" >&2; exit 1",
                );
                r.mcp_command = vec![mcp.to_string_lossy().into_owned(), "mcp".into()];
                r.env = vec![("SQAIL_TEST_WHY".into(), "404 not found".into())];
            },
        );
        assert!(matches!(outcome, Ok(Outcome::Finished)));
        let error = events
            .iter()
            .find_map(|e| match e {
                Event::Error(e) => Some(e.as_str()),
                _ => None,
            })
            .expect("an error event");
        assert!(
            error.contains("sqail's database tools did not start (status: failed)"),
            "{error}"
        );
        assert!(
            error.contains("looking up the connection: 404 not found"),
            "{error}"
        );
    }

    #[test]
    fn npm_shims_resolve_to_the_native_binary() {
        let shim = "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n:start\r\nSETLOCAL\r\nCALL :find_dp0\r\n\"%dp0%\\node_modules\\@xai-official\\grok\\bin\\grok.exe\"   %*\r\n";
        let sep = std::path::MAIN_SEPARATOR_STR;
        assert_eq!(
            shim_exe(shim).unwrap(),
            ["node_modules", "@xai-official", "grok", "bin", "grok.exe"].join(sep)
        );
        let js = "\"%_prog%\"  \"%dp0%\\node_modules\\@anthropic-ai\\claude-code\\cli.js\" %*\r\n";
        assert_eq!(shim_exe(js), None);
    }
}
