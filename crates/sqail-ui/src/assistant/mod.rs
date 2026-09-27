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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

    /// Forget the conversation (and its CLI session folder).
    pub fn new_chat(&mut self) {
        self.stop();
        if let Some(c) = self.conversation.take() {
            let _ = std::fs::remove_dir_all(&c.workdir);
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
            Update::Finished(result) => {
                self.running = None;
                if let Err(e) = result {
                    self.entries.push(Entry::Error(e));
                }
            }
        }
    }
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

impl SqailApp {
    pub fn assistant_update(&mut self, run: u64, update: Update) {
        self.assistant.apply(run, update);
    }

    /// Send the text in the input box as the next turn.
    pub fn assistant_send(&mut self) {
        let question = self.assistant.input.trim().to_string();
        if question.is_empty() || self.assistant.is_running() {
            return;
        }
        let tab = self.tabs.get(self.active);
        // A chat stays on the connection it started with.
        let conn_id = match &self.assistant.conversation {
            Some(c) => Some(c.connection),
            None => tab.and_then(|t| t.connection),
        };
        let Some(conn) = conn_id.and_then(|id| self.service.connection(id)).cloned() else {
            self.assistant.entries.push(Entry::Error(
                "Choose a connection for the editor tab first.".into(),
            ));
            return;
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
                workdir: crate::settings::config_dir()
                    .join("assistant")
                    .join(Uuid::new_v4().to_string()),
                session: None,
            });
        let mut prompt = question.clone();
        if settings.include_editor
            && let Some(sql) = tab.map(|t| t.text.trim()).filter(|s| !s.is_empty())
        {
            let sql: String = sql.chars().take(EDITOR_CONTEXT_CHARS).collect();
            prompt.push_str(&format!("\n\nThe SQL in my editor:\n```sql\n{sql}\n```"));
        }
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
        };
        self.assistant.entries.push(Entry::User(question));
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
    fn late_events_of_a_stopped_turn_are_ignored() {
        let mut a = running();
        a.stop();
        a.apply(1, Update::Event(Event::TextDelta("late".into())));
        assert_eq!(a.entries, [Entry::Note("Stopped.".into())]);
    }
}
