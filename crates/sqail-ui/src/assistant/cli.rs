//! Running Claude Code or Grok headless, with sqail's MCP server as their
//! only tools, and turning their output into [`Event`]s.

use std::path::{Path, PathBuf};
use std::process::Stdio;

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
}

pub enum Outcome {
    Finished,
    Stopped,
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
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(text.as_bytes()).await?;
        // Dropping the pipe closes it, so the CLI stops reading.
    }
    let stdout = child.stdout.take().context("no stdout")?;
    let mut stderr = child.stderr.take().context("no stderr")?;
    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf).await;
        String::from_utf8_lossy(&buf).into_owned()
    });

    let mut lines = BufReader::new(stdout).lines();
    let mut parser = Parser::default();
    let mut done = false;
    loop {
        tokio::select! {
            line = lines.next_line() => match line? {
                Some(line) => {
                    for ev in parser.line(&line) {
                        done |= matches!(ev, Event::Done { .. });
                        on_event(ev);
                    }
                }
                None => break,
            },
            _ = &mut stop => {
                let _ = child.kill().await;
                return Ok(Outcome::Stopped);
            }
        }
    }
    let status = child.wait().await?;
    let stderr = stderr_task.await.unwrap_or_default();
    if !status.success() && !done {
        let tail: String = stderr
            .trim()
            .chars()
            .rev()
            .take(1500)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        bail!(
            "{} exited with {status}{}",
            req.provider.label(),
            if tail.is_empty() {
                String::new()
            } else {
                format!(":\n{tail}")
            }
        );
    }
    Ok(Outcome::Finished)
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
