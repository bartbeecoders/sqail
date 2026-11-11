//! The AI assistant: a chat panel that runs Claude Code or Grok headless,
//! with sqail's own MCP server (`sqail mcp`) as their only tools, so they can
//! explore the active connection and test read-only queries.

pub mod cli;
pub mod guard;
pub mod mcp;
pub mod panel;
pub mod stream;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sqail_client::proto::Engine;
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::app::{Msg, SqailApp};
use cli::Provider;
use stream::Event;

/// `[assistant]` in settings.toml.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AssistantSettings {
    pub provider: Provider,
    /// Where the CLIs are, when not on PATH.
    pub claude_path: Option<PathBuf>,
    pub grok_path: Option<PathBuf>,
    /// Model passed to the CLI; empty = the CLI's default.
    pub claude_model: Option<String>,
    pub grok_model: Option<String>,
    /// Most rows one assistant query shows the model.
    pub max_rows: u64,
    /// Send the SQL of the active tab along with the question.
    pub include_editor: bool,
    pub open: bool,
}

impl Default for AssistantSettings {
    fn default() -> Self {
        Self {
            provider: Provider::default(),
            claude_path: None,
            grok_path: None,
            claude_model: None,
            grok_model: None,
            max_rows: mcp::DEFAULT_ROWS,
            include_editor: true,
            open: false,
        }
    }
}

impl AssistantSettings {
    fn program(&self, p: Provider) -> Option<PathBuf> {
        match p {
            Provider::ClaudeCode => self.claude_path.clone(),
            Provider::Grok => self.grok_path.clone(),
        }
    }

    fn model(&self, p: Provider) -> Option<String> {
        match p {
            Provider::ClaudeCode => self.claude_model.clone(),
            Provider::Grok => self.grok_model.clone(),
        }
    }
}

/// One item in the conversation.
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    User(String),
    Answer(String),
    Tool {
        id: String,
        name: String,
        input: serde_json::Value,
        result: Option<(String, bool)>,
    },
    Error(String),
    Note(String),
}

/// What a chat is bound to; fixed until "New chat".
pub struct Conversation {
    pub provider: Provider,
    pub connection: Uuid,
    pub connection_name: String,
    pub engine: Engine,
    pub workdir: PathBuf,
    /// The CLI's session, to continue with follow-up questions.
    pub session: Option<String>,
}

#[derive(Default)]
pub struct Assistant {
    pub input: String,
    pub entries: Vec<Entry>,
    pub conversation: Option<Conversation>,
    /// The turn in progress and how to stop it.
    running: Option<(u64, oneshot::Sender<()>)>,
    next_run: u64,
}

/// Messages from a running turn to the UI.
#[derive(Debug)]
pub enum Update {
    Event(Event),
    Finished(Result<bool, String>),
}

impl Assistant {
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// Forget the conversation, with its folder and the CLI's transcript.
    pub fn new_chat(&mut self) {
        self.stop();
        if let Some(c) = self.conversation.take() {
            cli::remove_conversation_data(&c.workdir);
        }
        self.entries.clear();
    }

    pub fn stop(&mut self) {
        if let Some((_, tx)) = self.running.take() {
            let _ = tx.send(());
            self.entries.push(Entry::Note("Stopped.".into()));
        }
    }

    fn apply(&mut self, run: u64, update: Update) {
        if self.running.as_ref().map(|(r, _)| *r) != Some(run) {
            return; // A stopped turn still delivering.
        }
        match update {
            Update::Event(Event::Session(id)) => {
                if let Some(c) = &mut self.conversation {
                    c.session = Some(id);
                }
            }
            Update::Event(Event::TextDelta(t) | Event::Text(t)) => match self.entries.last_mut() {
                Some(Entry::Answer(a)) => a.push_str(&t),
                _ => self.entries.push(Entry::Answer(t)),
            },
            Update::Event(Event::ToolUse { id, name, input }) => {
                self.entries.push(Entry::Tool {
                    id,
                    name,
                    input,
                    result: None,
                });
            }
            Update::Event(Event::ToolResult { id, text, is_error }) => {
                if let Some(Entry::Tool { result, .. }) = self
                    .entries
                    .iter_mut()
                    .rev()
                    .find(|e| matches!(e, Entry::Tool { id: i, .. } if *i == id))
                {
                    *result = Some((text, is_error));
                }
            }
            Update::Event(Event::Done {
                session,
                is_error,
                message,
                cost_usd,
            }) => {
                if let (Some(s), Some(c)) = (session, &mut self.conversation) {
                    c.session = Some(s);
                }
                if is_error {
                    self.entries.push(Entry::Error(
                        message.unwrap_or_else(|| "The assistant stopped with an error.".into()),
                    ));
                } else if !self
                    .entries
                    .iter()
                    .rev()
                    .take_while(|e| !matches!(e, Entry::User(_)))
                    .any(|e| matches!(e, Entry::Answer(_)))
                    && let Some(m) = message
                {
                    // No streamed text (some CLIs only report the result).
                    self.entries.push(Entry::Answer(m));
                }
                if let Some(cost) = cost_usd.filter(|c| *c > 0.0) {
                    self.entries.push(Entry::Note(format!("${cost:.4}")));
                }
            }
            Update::Event(Event::Error(e)) => self.entries.push(Entry::Error(e)),
            Update::Event(Event::McpFailed { server, status }) => self.entries.push(Entry::Error(
                format!("The MCP server `{server}` did not start (status: {status})."),
            )),
            Update::Finished(result) => {
                self.running = None;
                if let Err(e) = result {
                    self.entries.push(Entry::Error(e));
                }
            }
        }
    }
}

/// Where conversations keep their private folders.
fn conversations_dir() -> PathBuf {
    crate::settings::config_dir().join("assistant")
}

/// Remove, in the background, conversations an earlier run left behind
/// (conversations do not outlive sqail, but a crash skips the clean-up).
pub fn sweep_stale_conversations() {
    let root = conversations_dir();
    std::thread::spawn(move || {
        cli::sweep_stale(&root, std::time::Duration::from_secs(24 * 3600));
    });
}

/// Instructions for the model: one line, so they survive any command line.
fn instructions(engine: Engine, connection: &str, max_rows: u64) -> String {
    let engine = mcp::engine_name(engine);
    format!(
        "You are the SQL assistant inside sqail, a desktop SQL editor. You work on the {engine} \
         database \"{connection}\" through sqail's tools: list_schemas, list_tables, describe_table \
         and run_query. Explore the schema with them instead of guessing names. run_query is \
         read-only and returns at most {max_rows} rows: use it to look at data and to check a query \
         before you propose it. You cannot change data or schema; when the user wants changes, \
         write the SQL for them to review and run themselves. Write {engine} SQL. Put each query \
         you propose in a ```sql fenced code block; the user can insert it into the editor or open \
         it in a new tab. Keep explanations short."
    )
}

const EDITOR_CONTEXT_CHARS: usize = 8000;

/// Whether `tab` is on the chat's connection: only then does its SQL go
/// with a question, and does Insert put an answer into it.
pub fn same_connection(tab: Option<&crate::editor::Tab>, conversation: &Conversation) -> bool {
    tab.and_then(|t| t.connection) == Some(conversation.connection)
}

/// What goes with the question besides itself (appended to the prompt),
/// and a note for the conversation about what was left out.
fn editor_context(
    tab: Option<&crate::editor::Tab>,
    conversation: &Conversation,
    include_editor: bool,
) -> (String, Option<String>) {
    let Some(sql) = tab
        .map(|t| t.text.trim())
        .filter(|s| include_editor && !s.is_empty())
    else {
        return (String::new(), None);
    };
    if !same_connection(tab, conversation) {
        return (
            String::new(),
            Some(format!(
                "The active tab is not on this chat's connection “{}”, so its SQL was not sent.",
                conversation.connection_name
            )),
        );
    }
    let mut shown: String = sql.chars().take(EDITOR_CONTEXT_CHARS).collect();
    let cut = shown.len() < sql.len();
    if cut {
        shown.push_str("\n-- …");
    }
    let mut context = format!("\n\nThe SQL in my editor:\n```sql\n{shown}\n```");
    if cut {
        context.push_str(&format!(
            "\n(Truncated: this is only the first {EDITOR_CONTEXT_CHARS} characters of the editor.)"
        ));
    }
    (context, None)
}

impl SqailApp {
    pub fn assistant_update(&mut self, run: u64, update: Update) {
        self.assistant.apply(run, update);
    }

    /// Whether the active tab is on the chat's connection (true before a
    /// chat has started: it will be).
    pub fn assistant_tab_matches(&self) -> bool {
        self.assistant
            .conversation
            .as_ref()
            .is_none_or(|c| same_connection(self.tabs.get(self.active), c))
    }

    /// Send the text in the input box as the next turn.
    pub fn assistant_send(&mut self) {
        let question = self.assistant.input.trim().to_string();
        if question.is_empty() || self.assistant.is_running() {
            return;
        }
        let tab = self.tabs.get(self.active);
        // A chat stays on the connection it started with.
        let conn = match &self.assistant.conversation {
            Some(c) => self.service.connection(c.connection).ok_or_else(|| {
                format!(
                    "This chat's connection “{}” is no longer available (deleted, or another \
                     service is connected). Start a new chat.",
                    c.connection_name
                )
            }),
            None => tab
                .and_then(|t| t.connection)
                .and_then(|id| self.service.connection(id))
                .ok_or_else(|| "Choose a connection for the editor tab first.".to_string()),
        };
        let conn = match conn {
            Ok(c) => c.clone(),
            Err(e) => {
                self.assistant.entries.push(Entry::Error(e));
                return;
            }
        };
        let (Some(profile), true) = (self.service.profile.clone(), self.service.client.is_some())
        else {
            self.assistant
                .entries
                .push(Entry::Error("Connect to a service first.".into()));
            return;
        };
        let Some((token, _)) = crate::secrets::load_token(&profile.url) else {
            self.assistant.entries.push(Entry::Error(
                "No token for this service; reconnect to it.".into(),
            ));
            return;
        };
        let settings = self.settings.assistant.clone();
        let conversation = self
            .assistant
            .conversation
            .get_or_insert_with(|| Conversation {
                provider: settings.provider,
                connection: conn.id,
                connection_name: conn.name.clone(),
                engine: conn.engine,
                workdir: conversations_dir().join(Uuid::new_v4().to_string()),
                session: None,
            });
        let (context, note) = editor_context(tab, conversation, settings.include_editor);
        let prompt = format!("{question}{context}");
        let mut env = vec![
            (mcp::Env::URL.to_string(), profile.url.clone()),
            (mcp::Env::TOKEN.to_string(), token),
            (mcp::Env::CONNECTION.to_string(), conn.id.to_string()),
            (
                mcp::Env::MAX_ROWS.to_string(),
                settings.max_rows.to_string(),
            ),
        ];
        if let Some(fp) = &profile.fingerprint {
            env.push((mcp::Env::FINGERPRINT.to_string(), fp.clone()));
        }
        if let (Some(c), Some(k)) = (&profile.client_cert, &profile.client_key) {
            env.push((
                mcp::Env::CLIENT_CERT.to_string(),
                c.to_string_lossy().into_owned(),
            ));
            env.push((
                mcp::Env::CLIENT_KEY.to_string(),
                k.to_string_lossy().into_owned(),
            ));
        }
        let exe = std::env::current_exe()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "sqail".into());
        let req = cli::Request {
            provider: conversation.provider,
            program: settings.program(conversation.provider),
            model: settings.model(conversation.provider),
            instructions: instructions(
                conversation.engine,
                &conversation.connection_name,
                settings.max_rows,
            ),
            prompt,
            resume: conversation.session.clone(),
            workdir: conversation.workdir.clone(),
            mcp_command: vec![exe, "mcp".into()],
            env,
            limits: cli::Limits::default(),
        };
        self.assistant.entries.push(Entry::User(question));
        self.assistant.entries.extend(note.map(Entry::Note));
        self.assistant.input.clear();
        let run = self.assistant.next_run;
        self.assistant.next_run += 1;
        let (tx, rx) = oneshot::channel();
        self.assistant.running = Some((run, tx));
        self.worker.spawn(move |sink| async move {
            let events = sink.clone();
            let result = cli::run(
                req,
                move |ev| {
                    events.send(Msg::Assistant {
                        run,
                        update: Update::Event(ev),
                    })
                },
                rx,
            )
            .await;
            let finished = match result {
                Ok(cli::Outcome::Finished) => Ok(true),
                Ok(cli::Outcome::Stopped) => Ok(false),
                Err(e) => Err(format!("{e:#}")),
            };
            sink.send(Msg::Assistant {
                run,
                update: Update::Finished(finished),
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn running() -> Assistant {
        let (tx, _rx) = oneshot::channel();
        Assistant {
            running: Some((1, tx)),
            next_run: 2,
            ..Default::default()
        }
    }

    #[test]
    fn events_build_the_conversation() {
        let mut a = running();
        for u in [
            Update::Event(Event::TextDelta("Let me ".into())),
            Update::Event(Event::TextDelta("check.".into())),
            Update::Event(Event::ToolUse {
                id: "t1".into(),
                name: "run_query".into(),
                input: json!({"sql": "SELECT 1"}),
            }),
            Update::Event(Event::ToolResult {
                id: "t1".into(),
                text: "1".into(),
                is_error: false,
            }),
            Update::Event(Event::TextDelta("Done.".into())),
            Update::Event(Event::Done {
                session: Some("s".into()),
                is_error: false,
                message: Some("Done.".into()),
                cost_usd: Some(0.02),
            }),
            Update::Finished(Ok(true)),
        ] {
            a.apply(1, u);
        }
        assert_eq!(
            a.entries,
            [
                Entry::Answer("Let me check.".into()),
                Entry::Tool {
                    id: "t1".into(),
                    name: "run_query".into(),
                    input: json!({"sql": "SELECT 1"}),
                    result: Some(("1".into(), false))
                },
                Entry::Answer("Done.".into()),
                Entry::Note("$0.0200".into()),
            ]
        );
        assert!(!a.is_running());
    }

    #[test]
    fn result_text_is_used_when_nothing_streamed() {
        let mut a = running();
        a.entries.push(Entry::User("q".into()));
        a.apply(
            1,
            Update::Event(Event::Done {
                session: None,
                is_error: false,
                message: Some("Answer".into()),
                cost_usd: None,
            }),
        );
        assert_eq!(a.entries.last(), Some(&Entry::Answer("Answer".into())));
    }

    #[test]
    fn editor_sql_goes_only_from_a_tab_on_the_chats_connection() {
        let chat = Uuid::new_v4();
        let conversation = Conversation {
            provider: Provider::ClaudeCode,
            connection: chat,
            connection_name: "pg".into(),
            engine: Engine::Postgres,
            workdir: PathBuf::new(),
            session: None,
        };
        let mut tab = crate::editor::Tab::new(1, "t".into(), Some(chat));
        tab.text = "SELECT 1".into();
        assert_eq!(
            editor_context(Some(&tab), &conversation, true),
            (
                "\n\nThe SQL in my editor:\n```sql\nSELECT 1\n```".into(),
                None
            )
        );
        assert_eq!(
            editor_context(Some(&tab), &conversation, false),
            (String::new(), None)
        );

        for other in [Some(Uuid::new_v4()), None] {
            tab.connection = other;
            let (context, note) = editor_context(Some(&tab), &conversation, true);
            assert_eq!(context, "");
            assert!(note.unwrap().contains("not on this chat's connection “pg”"));
        }

        tab.connection = Some(chat);
        tab.text = "x".repeat(EDITOR_CONTEXT_CHARS + 10);
        let (context, _) = editor_context(Some(&tab), &conversation, true);
        assert!(context.contains("\n-- …\n```"), "{context}");
        assert!(
            context.ends_with("(Truncated: this is only the first 8000 characters of the editor.)")
        );
        assert_eq!(context.matches('x').count(), EDITOR_CONTEXT_CHARS);
    }

    #[test]
    fn error_events_become_error_entries() {
        let mut a = running();
        a.apply(1, Update::Event(Event::Error("tools failed".into())));
        assert_eq!(a.entries, [Entry::Error("tools failed".into())]);
    }

    #[test]
    fn late_events_of_a_stopped_turn_are_ignored() {
        let mut a = running();
        a.stop();
        a.apply(1, Update::Event(Event::TextDelta("late".into())));
        assert_eq!(a.entries, [Entry::Note("Stopped.".into())]);
    }
}
