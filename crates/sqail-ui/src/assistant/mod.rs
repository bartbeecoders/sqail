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
use crate::editor::Tab;
use crate::results::Severity;
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
    /// The next question carries its own context: don't add the editor's.
    no_editor_once: bool,
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
            Update::Event(Event::TextDelta(t)) => match self.entries.last_mut() {
                Some(Entry::Answer(a)) => a.push_str(&t),
                _ => self.entries.push(Entry::Answer(t)),
            },
            // A whole message: a paragraph of its own after another one.
            Update::Event(Event::Text(t)) => match self.entries.last_mut() {
                Some(Entry::Answer(a)) => {
                    if !a.is_empty() && !a.ends_with("\n\n") {
                        a.push_str(if a.ends_with('\n') { "\n" } else { "\n\n" });
                    }
                    a.push_str(&t);
                }
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
         database \"{connection}\" through sqail's tools: list_schemas, list_tables, describe_table, \
         run_query and explain_query (the estimated plan, for questions about performance). \
         Explore the schema with them instead of guessing names. run_query is \
         read-only and returns at most {max_rows} rows: use it to look at data and to check a query \
         before you propose it. You cannot change data or schema; when the user wants changes, \
         write the SQL for them to review and run themselves. Write {engine} SQL. Put each query \
         you propose in a ```sql fenced code block; the user can insert it into the editor or open \
         it in a new tab. Keep explanations short."
    )
}

const EDITOR_CONTEXT_CHARS: usize = 8000;
const ERROR_CONTEXT_CHARS: usize = 2000;

fn cut(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

/// The error messages of the tab's last run, if it failed.
fn last_error(tab: &Tab) -> Option<(&str, String)> {
    let run = tab.run.as_ref().filter(|r| r.failed && !r.running)?;
    let errors: Vec<&str> = run
        .messages
        .iter()
        .filter(|m| matches!(m.severity, Severity::Error))
        .map(|m| m.text.trim())
        .collect();
    (!errors.is_empty()).then(|| (run.sql.trim(), errors.join("\n")))
}

/// What the editor adds to a question: the selection, else the statement at
/// the cursor, else the whole tab; and the error of the tab's last run.
pub fn editor_context(tab: &Tab, engine: Option<Engine>) -> Option<String> {
    let mut out = String::new();
    let current = tab.current_sql(engine);
    let selected = tab.cursor.primary.index != tab.cursor.secondary.index;
    let (label, sql) = match current.trim() {
        "" => ("The SQL in my editor", tab.text.trim()),
        s if selected => ("The SQL I selected in the editor", s),
        s => ("The statement at my cursor in the editor", s),
    };
    if !sql.is_empty() {
        let sql = cut(sql, EDITOR_CONTEXT_CHARS);
        out.push_str(&format!("\n\n{label}:\n```sql\n{sql}\n```"));
    }
    if let Some((ran, error)) = last_error(tab) {
        if ran != sql {
            let ran = cut(ran, EDITOR_CONTEXT_CHARS);
            out.push_str(&format!(
                "\n\nThe last run in this tab:\n```sql\n{ran}\n```"
            ));
        }
        let error = cut(&error, ERROR_CONTEXT_CHARS);
        out.push_str(&format!("\n\nIt failed with:\n```\n{error}\n```"));
    }
    (!out.is_empty()).then_some(out)
}

impl SqailApp {
    pub fn assistant_update(&mut self, run: u64, update: Update) {
        self.assistant.apply(run, update);
    }

    /// "Ask assistant" on a failed run: open the panel and ask why the
    /// tab's last query failed, with its SQL and error.
    pub fn assistant_ask_about_error(&mut self, idx: usize) {
        let Some(tab) = self.tabs.get(idx) else {
            return;
        };
        let Some((sql, error)) = last_error(tab) else {
            return;
        };
        if self.assistant.is_running() {
            return self.notify(
                "The assistant is still answering; ask again when it's done.",
                true,
            );
        }
        if let Some(c) = &self.assistant.conversation
            && Some(c.connection) != tab.connection
        {
            return self.notify(
                "The assistant's chat is on another connection: start a new chat to ask about this error.",
                true,
            );
        }
        let sql = cut(sql, EDITOR_CONTEXT_CHARS);
        let error = cut(&error, ERROR_CONTEXT_CHARS);
        self.active = idx;
        self.assistant.input = format!(
            "This query failed:\n```sql\n{sql}\n```\nError:\n```\n{error}\n```\nWhy, and how do I fix it?"
        );
        self.assistant.no_editor_once = true;
        if !self.settings.assistant.open {
            self.settings.assistant.open = true;
            self.settings.save();
        }
        self.assistant_send();
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
            && !std::mem::take(&mut self.assistant.no_editor_once)
            && let Some(context) = tab.and_then(|t| editor_context(t, Some(conn.engine)))
        {
            prompt.push_str(&context);
        }
        let mut env = vec![
            (mcp::Env::URL.to_string(), profile.url.clone()),
            (mcp::Env::TOKEN.to_string(), token),
            (mcp::Env::CONNECTION.to_string(), conn.id.to_string()),
            (
                mcp::Env::MAX_ROWS.to_string(),
                mcp::effective_max_rows(settings.max_rows).to_string(),
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
                mcp::effective_max_rows(settings.max_rows),
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

    #[test]
    fn whole_messages_are_separate_paragraphs() {
        let mut a = running();
        a.apply(1, Update::Event(Event::Text("First.".into())));
        a.apply(1, Update::Event(Event::Text("Second.".into())));
        a.apply(1, Update::Event(Event::TextDelta(" More".into())));
        assert_eq!(a.entries, [Entry::Answer("First.\n\nSecond. More".into())]);
    }

    #[test]
    fn editor_context_is_the_selection_or_statement_and_the_error() {
        use crate::results::{Message, Run};
        use egui::text::{CCursor, CCursorRange};
        let pg = Some(Engine::Postgres);
        let mut tab = Tab::new(1, "t".into(), None);
        tab.text = "SELECT 1;\n\nSELECT name\nFROM people;\n".into();
        // The cursor in the second statement.
        tab.cursor = CCursorRange::one(CCursor::new(15));
        let ctx = editor_context(&tab, pg).unwrap();
        assert_eq!(
            ctx,
            "\n\nThe statement at my cursor in the editor:\n```sql\nSELECT name\nFROM people\n```"
        );
        // A selection wins.
        tab.cursor = CCursorRange::two(CCursor::new(0), CCursor::new(8));
        let ctx = editor_context(&tab, pg).unwrap();
        assert!(
            ctx.starts_with("\n\nThe SQL I selected in the editor:\n```sql\nSELECT 1\n```"),
            "{ctx}"
        );
        // The last run's error, and its SQL when that is other SQL.
        let mut run = Run::new(1, "SELECT nope FROM people".into());
        run.running = false;
        run.failed = true;
        run.messages.push(Message {
            severity: Severity::Error,
            text: "column \"nope\" does not exist".into(),
        });
        tab.run = Some(run);
        let ctx = editor_context(&tab, pg).unwrap();
        assert!(
            ctx.ends_with(
                "The last run in this tab:\n```sql\nSELECT nope FROM people\n```\n\n\
                 It failed with:\n```\ncolumn \"nope\" does not exist\n```"
            ),
            "{ctx}"
        );
        // Nothing to say for an empty tab without errors.
        let empty = Tab::new(2, "t".into(), None);
        assert_eq!(editor_context(&empty, pg), None);
    }
}
