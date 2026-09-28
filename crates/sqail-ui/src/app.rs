//! The application: state, message handling, layout and shortcuts.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use egui::{Align, Color32, Layout, RichText};
use futures::StreamExt;
use sqail_client::proto::{
    Connection, Engine, QueryEvent, QueryRequest, ServiceInfo, SessionInfo, TestResult,
};
use sqail_client::{Client, On, Target, Trust};
use uuid::Uuid;

use crate::commands::Command;
use crate::dialogs::{ConnForm, Dialog, WelcomeForm};
use crate::editor::{EditorAction, Tab};
use crate::local_service::{self, Provisioned};
use crate::results::Run;
use crate::schema::{SchemaMsg, SchemaTree};
use crate::secrets;
use crate::settings::{ServiceProfile, Settings, ThemePref};
use crate::theme;
use crate::worker::Worker;

/// Everything background tasks report back.
pub enum Msg {
    Connected {
        url: String,
        result: Result<(Client, ServiceInfo), sqail_client::Error>,
    },
    Connections(Result<Vec<Connection>, String>),
    Probed(Result<(String, sqail_client::proto::Health), String>),
    Provisioned(Result<Provisioned, String>),
    ConnTested(Result<TestResult, String>),
    ConnDatabases(String, Result<Vec<String>, String>),
    ConnSaved(Result<Connection, String>),
    ConnDeleted(Result<Uuid, String>),
    SessionOpened {
        tab: u64,
        connection: Uuid,
        session: SessionInfo,
    },
    Query {
        tab: u64,
        run: u64,
        event: QueryEvent,
    },
    QueryFailed {
        tab: u64,
        run: u64,
        error: String,
    },
    Schema(SchemaMsg),
    OpenInTab {
        connection: Uuid,
        title: String,
        text: String,
        run: bool,
    },
    FileOpened(Result<(PathBuf, String), String>),
    FileSaved {
        tab: u64,
        result: Result<PathBuf, String>,
    },
    Notice {
        text: String,
        error: bool,
    },
    /// Progress of an AI assistant turn.
    Assistant {
        run: u64,
        update: crate::assistant::Update,
    },
    ImportPicked {
        conn: Uuid,
        schema: String,
        table: String,
        path: PathBuf,
    },
    ImportColumns(Result<Vec<sqail_client::proto::ColumnInfo>, String>),
    ImportProgress(u64),
    ImportDone(Result<u64, String>),
    EditReady {
        tab: u64,
        run: u64,
        result: usize,
        state: Result<Box<crate::editing::EditState>, String>,
    },
    EditsApplied {
        tab: u64,
        result: Result<usize, String>,
    },
    Plan {
        tab: u64,
        run: u64,
        result: Result<sqail_client::proto::Plan, String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceStatus {
    /// No service configured yet.
    Unconfigured,
    Connecting,
    Connected,
    Failed(String),
}

pub struct Service {
    pub profile: Option<ServiceProfile>,
    pub client: Option<Client>,
    pub info: Option<ServiceInfo>,
    pub status: ServiceStatus,
    pub connections: Vec<Connection>,
    pub token_store: Option<secrets::Store>,
}

impl Service {
    pub fn connection(&self, id: Uuid) -> Option<&Connection> {
        self.connections.iter().find(|c| c.id == id)
    }
}

pub struct SqailApp {
    pub settings: Settings,
    pub worker: Worker,
    pub service: Service,
    pub tabs: Vec<Tab>,
    pub active: usize,
    next_tab: u64,
    next_run: u64,
    pub schema: SchemaTree,
    pub dialog: Dialog,
    notice: Option<(String, bool, Instant)>,
    /// Value shown in the cell viewer window: (title, text).
    pub viewer: Option<(String, String)>,
    pending_close: Option<usize>,
    pub history: crate::local::History,
    pub snippets: crate::local::Snippets,
    pub sidebar: crate::sidebar::View,
    pub history_filter: String,
    pub snippet_filter: String,
    saved_workspace: Option<crate::local::Workspace>,
    workspace_checked: Instant,
    pub keymap: crate::commands::Keymap,
    /// Set once the user confirmed quitting despite open transactions.
    pub allow_close: bool,
    pub palette: Option<crate::palette::Palette>,
    pub assistant: crate::assistant::Assistant,
}

impl SqailApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install(&cc.egui_ctx);
        let settings = Settings::load();
        apply_theme(&cc.egui_ctx, settings.theme);
        let mut app = Self {
            worker: Worker::new(cc.egui_ctx.clone()),
            service: Service {
                profile: None,
                client: None,
                info: None,
                status: ServiceStatus::Unconfigured,
                connections: Vec::new(),
                token_store: None,
            },
            tabs: Vec::new(),
            active: 0,
            next_tab: 1,
            next_run: 1,
            schema: SchemaTree::default(),
            dialog: Dialog::None,
            notice: None,
            viewer: None,
            pending_close: None,
            history: crate::local::History::load(),
            snippets: crate::local::Snippets::load(),
            sidebar: Default::default(),
            history_filter: String::new(),
            snippet_filter: String::new(),
            saved_workspace: None,
            workspace_checked: Instant::now(),
            keymap: crate::commands::Keymap::load(),
            allow_close: false,
            palette: None,
            assistant: Default::default(),
            settings,
        };
        app.restore_workspace();
        if let Some(w) = app.keymap.warnings.first().cloned() {
            app.notify(w, true);
        }
        match app.settings.active().cloned() {
            Some(profile) => app.connect(profile),
            None => {
                let mut form = WelcomeForm::default();
                // Development convenience (scripts/dev.sh): skip the first-run
                // choice and set up the local service directly.
                if std::env::var_os("SQAIL_AUTO_LOCAL").is_some()
                    && local_service::find_binary().is_some()
                {
                    form.busy = true;
                    app.worker.run(async {
                        Msg::Provisioned(
                            local_service::provision()
                                .await
                                .map_err(|e| format!("{e:#}")),
                        )
                    });
                }
                app.dialog = Dialog::Welcome(form);
            }
        }
        app
    }

    // ------------------------------------------------------------ service --

    /// Connect to a configured service using its stored token.
    pub fn connect(&mut self, profile: ServiceProfile) {
        let Some((token, store)) = secrets::load_token(&profile.url) else {
            self.service.profile = Some(profile.clone());
            self.service.status = ServiceStatus::Failed("no token stored for this service".into());
            self.dialog = Dialog::Welcome(WelcomeForm::for_url(&profile.url));
            return;
        };
        self.service.profile = Some(profile.clone());
        self.service.token_store = Some(store);
        self.service.client = None;
        self.service.status = ServiceStatus::Connecting;
        let trust = match &profile.fingerprint {
            Some(fp) => Trust::Pinned(fp.clone()),
            None => Trust::System,
        };
        let autostart = profile.local && self.settings.autostart_local;
        let (cert, key) = (profile.client_cert.clone(), profile.client_key.clone());
        let url = profile.url.clone();
        self.worker.run(async move {
            if autostart && let Err(e) = local_service::ensure_running(&url).await {
                tracing::warn!(error = %e, "could not start the local service");
            }
            let result = async {
                let identity = match (&cert, &key) {
                    (Some(c), Some(k)) => Some(sqail_client::Identity::from_files(c, k)?),
                    _ => None,
                };
                let client = Client::new(&Target {
                    url: url.clone(),
                    token,
                    trust,
                    identity,
                })?;
                let info = client.info().await?;
                Ok((client, info))
            }
            .await;
            Msg::Connected { url, result }
        });
    }

    pub fn refresh_connections(&mut self) {
        let Some(client) = self.service.client.clone() else {
            return;
        };
        self.worker.run(async move {
            Msg::Connections(client.connections().await.map_err(|e| e.to_string()))
        });
    }

    /// Store a new service (token → credential store) and connect to it.
    pub fn adopt_service(&mut self, profile: ServiceProfile, token: &str) {
        match secrets::save_token(&profile.url, token) {
            Ok(store) => {
                if store == secrets::Store::File {
                    self.notify("No OS credential store found: the token is saved in tokens.toml (owner-only).", false);
                }
            }
            Err(e) => {
                self.notify(format!("Could not store the token: {e}"), true);
                return;
            }
        }
        self.settings.upsert_service(profile.clone());
        self.settings.save();
        self.dialog = Dialog::None;
        self.connect(profile);
    }

    pub fn forget_service(&mut self) {
        if let Some(p) = self.service.profile.take() {
            secrets::delete_token(&p.url);
            self.settings.services.retain(|s| s.url != p.url);
            self.settings.active_service = None;
            self.settings.save();
        }
        self.service.client = None;
        self.service.info = None;
        self.service.connections.clear();
        self.service.status = ServiceStatus::Unconfigured;
        self.dialog = Dialog::Welcome(WelcomeForm::default());
    }

    // ---------------------------------------------------------- workspace --

    fn restore_workspace(&mut self) {
        let ws = crate::local::Workspace::load().filter(|w| !w.tabs.is_empty());
        let Some(ws) = ws else {
            self.new_tab(None);
            return;
        };
        for saved in &ws.tabs {
            let id = self.next_tab;
            self.next_tab += 1;
            self.tabs.push(Tab::restore(id, saved));
        }
        self.active = ws.active.min(self.tabs.len() - 1);
        self.saved_workspace = Some(ws);
    }

    pub fn workspace(&self) -> crate::local::Workspace {
        crate::local::Workspace {
            tabs: self.tabs.iter().map(Tab::snapshot).collect(),
            active: self.active,
        }
    }

    /// Persist open tabs when they changed (checked about once a second).
    fn autosave_workspace(&mut self, force: bool) {
        if !force && self.workspace_checked.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.workspace_checked = Instant::now();
        let ws = self.workspace();
        if self.saved_workspace.as_ref() != Some(&ws) {
            ws.save();
            self.saved_workspace = Some(ws);
        }
    }

    /// SQL for "save as snippet": the selection, else the statement at the
    /// cursor, else the whole tab.
    pub fn current_sql_for_snippet(&self) -> String {
        let Some(tab) = self.tabs.get(self.active) else {
            return String::new();
        };
        let sql = tab.current_sql(self.engine_of(tab.connection));
        if sql.trim().is_empty() {
            tab.text.clone()
        } else {
            sql
        }
    }

    fn record_history(&mut self, tab_id: u64) {
        let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) else {
            return;
        };
        let Some(run) = tab.run.as_ref() else { return };
        let connection_name = tab
            .connection
            .and_then(|c| self.service.connection(c))
            .map(|c| c.name.clone())
            .unwrap_or_default();
        let entry = crate::local::HistoryEntry {
            at: crate::local::now_secs(),
            connection: tab.connection,
            connection_name,
            sql: run.sql.clone(),
            duration_ms: run.elapsed().as_millis() as u64,
            rows: run.total_rows() as u64,
            outcome: if run.cancelled {
                crate::local::Outcome::Cancelled
            } else if run.failed {
                crate::local::Outcome::Failed
            } else {
                crate::local::Outcome::Ok
            },
        };
        self.history.push(entry);
    }

    pub fn notify(&mut self, text: impl Into<String>, error: bool) {
        self.notice = Some((text.into(), error, Instant::now()));
    }

    // --------------------------------------------------------------- tabs --

    pub fn new_tab(&mut self, connection: Option<Uuid>) -> usize {
        let connection =
            connection.or_else(|| self.tabs.get(self.active).and_then(|t| t.connection));
        let id = self.next_tab;
        self.next_tab += 1;
        self.tabs
            .push(Tab::new(id, format!("Query {id}"), connection));
        self.active = self.tabs.len() - 1;
        self.active
    }

    pub fn open_text_tab(&mut self, connection: Uuid, title: String, text: String) -> usize {
        let idx = self.new_tab(Some(connection));
        let tab = &mut self.tabs[idx];
        tab.title = title;
        tab.text = text;
        idx
    }

    pub fn request_close_tab(&mut self, idx: usize) {
        if self.tabs[idx].in_transaction {
            self.dialog = Dialog::ConfirmCloseTransaction(Some(idx));
        } else if self.tabs[idx].is_dirty() {
            self.pending_close = Some(idx);
            self.dialog = Dialog::ConfirmClose(idx);
        } else {
            self.close_tab(idx);
        }
    }

    pub fn close_tab(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(idx);
        self.cancel_run_of(&tab);
        if let (Some(client), Some((_, session))) = (self.service.client.clone(), tab.session) {
            self.worker.detach(async move {
                let _ = client.close_session(session).await;
            });
        }
        if self.tabs.is_empty() {
            self.new_tab(tab.connection);
        }
        self.active = self.active.min(self.tabs.len() - 1);
        self.pending_close = None;
    }

    pub fn engine_of(&self, conn: Option<Uuid>) -> Option<Engine> {
        conn.and_then(|c| self.service.connection(c))
            .map(|c| c.engine)
    }

    // ------------------------------------------------------------ queries --

    /// Run `sql` in tab `idx` on its session (opened on first use).
    pub fn run_sql(&mut self, idx: usize, sql: String) {
        let Some(client) = self.service.client.clone() else {
            self.notify("Not connected to a service.", true);
            return;
        };
        let tab = &mut self.tabs[idx];
        let Some(conn) = tab.connection else {
            self.notify("Choose a connection for this tab first.", true);
            return;
        };
        if tab.run.as_ref().is_some_and(|r| r.running) {
            self.notify("A query is already running in this tab.", true);
            return;
        }
        let mut sql = sql.trim().to_string();
        if sql.is_empty() {
            return;
        }
        // Manual-commit tabs open a transaction on the first run.
        let manual_begin = !tab.autocommit && !tab.in_transaction;
        if manual_begin {
            let engine = self.service.connection(conn).map(|c| c.engine);
            let begin = if engine == Some(Engine::Mssql) {
                "BEGIN TRANSACTION"
            } else {
                "BEGIN"
            };
            sql = format!("{begin};\n{sql}");
        }
        let tab = &mut self.tabs[idx];
        let run_id = self.next_run;
        self.next_run += 1;
        tab.run = Some(Run::new(run_id, sql.clone()));
        let session = tab.session.filter(|(c, _)| *c == conn).map(|(_, s)| s);
        let tab_id = tab.id;
        let req = QueryRequest {
            sql,
            params: Vec::new(),
            max_rows: Some(self.settings.max_rows),
            timeout_ms: None,
        };
        self.worker.spawn(move |sink| async move {
            let fail = |error: String| Msg::QueryFailed {
                tab: tab_id,
                run: run_id,
                error,
            };
            let mut session = session;
            // One retry: the session may have expired or lost its connection.
            for attempt in 0..2 {
                let sid = match session {
                    Some(s) => s,
                    None => match client.open_session(conn).await {
                        Ok(info) => {
                            let sid = info.id;
                            sink.send(Msg::SessionOpened {
                                tab: tab_id,
                                connection: conn,
                                session: info,
                            });
                            sid
                        }
                        Err(e) => return sink.send(fail(e.to_string())),
                    },
                };
                match client.query(On::Session(sid), &req).await {
                    Ok(mut stream) => {
                        while let Some(ev) = stream.events.next().await {
                            match ev {
                                Ok(event) => sink.send(Msg::Query {
                                    tab: tab_id,
                                    run: run_id,
                                    event,
                                }),
                                Err(e) => return sink.send(fail(e.to_string())),
                            }
                        }
                        return;
                    }
                    Err(e)
                        if attempt == 0
                            && matches!(e.status(), Some(404 | 409))
                            && is_session_gone(&e) =>
                    {
                        session = None;
                    }
                    Err(e) => return sink.send(fail(e.to_string())),
                }
            }
        });
    }

    pub fn cancel_run(&mut self, idx: usize) {
        if let Some(tab) = self.tabs.get(idx) {
            self.cancel_run_of_ref(tab.run.as_ref());
        }
    }

    fn cancel_run_of(&self, tab: &Tab) {
        self.cancel_run_of_ref(tab.run.as_ref());
    }

    fn cancel_run_of_ref(&self, run: Option<&Run>) {
        let (Some(client), Some(run)) = (self.service.client.clone(), run) else {
            return;
        };
        if let (true, Some(qid)) = (run.running, run.query_id) {
            self.worker.detach(async move {
                let _ = client.cancel(qid).await;
            });
        }
    }

    fn tab_mut(&mut self, id: u64) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    // ----------------------------------------------------------- messages --

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Connected { url, result } => {
                if self.service.profile.as_ref().map(|p| &p.url) != Some(&url) {
                    return; // a newer connect superseded this one
                }
                match result {
                    Ok((client, info)) => {
                        self.service.client = Some(client);
                        self.service.info = Some(info);
                        self.service.status = ServiceStatus::Connected;
                        self.refresh_connections();
                    }
                    Err(e) => {
                        let auth = sqail_client::is_auth_error(&e);
                        self.service.status = ServiceStatus::Failed(e.to_string());
                        if auth || matches!(e, sqail_client::Error::CertificateMismatch) {
                            let mut form = WelcomeForm::for_url(&url);
                            form.error = Some(e.to_string());
                            self.dialog = Dialog::Welcome(form);
                        }
                    }
                }
            }
            Msg::Connections(Ok(list)) => {
                // Tabs bound to a deleted profile lose their binding.
                for tab in &mut self.tabs {
                    if tab
                        .connection
                        .is_some_and(|c| !list.iter().any(|x| x.id == c))
                    {
                        tab.connection = None;
                    }
                }
                if self.tabs.iter().all(|t| t.connection.is_none())
                    && let (Some(first), Some(tab)) = (list.first(), self.tabs.get_mut(self.active))
                {
                    tab.connection = Some(first.id);
                }
                self.schema.retain(&list);
                self.service.connections = list;
            }
            Msg::Connections(Err(e)) => {
                self.notify(format!("Could not load connections: {e}"), true)
            }
            Msg::Probed(r) => {
                if let Dialog::Welcome(form) = &mut self.dialog {
                    form.on_probed(r);
                }
            }
            Msg::Provisioned(r) => match r {
                Ok(p) => {
                    let profile = ServiceProfile {
                        name: "Local".into(),
                        url: p.url.clone(),
                        fingerprint: Some(p.fingerprint.clone()),
                        local: true,
                        client_cert: None,
                        client_key: None,
                    };
                    self.adopt_service(profile, &p.token);
                }
                Err(e) => {
                    if let Dialog::Welcome(form) = &mut self.dialog {
                        form.busy = false;
                        form.error = Some(e);
                    }
                }
            },
            Msg::ConnTested(r) => {
                if let Dialog::Connection(form) = &mut self.dialog {
                    form.on_tested(r);
                }
            }
            Msg::ConnDatabases(key, r) => {
                if let Dialog::Connection(form) = &mut self.dialog {
                    form.on_databases(key, r);
                }
            }
            Msg::ConnSaved(r) => match r {
                Ok(conn) => {
                    self.dialog = Dialog::None;
                    self.notify(format!("Saved connection “{}”.", conn.name), false);
                    self.schema.invalidate(conn.id);
                    if let Some(tab) = self.tabs.get_mut(self.active)
                        && tab.connection.is_none()
                    {
                        tab.connection = Some(conn.id);
                    }
                    self.refresh_connections();
                }
                Err(e) => {
                    if let Dialog::Connection(form) = &mut self.dialog {
                        form.saving = false;
                        form.error = Some(e);
                    }
                }
            },
            Msg::ConnDeleted(r) => match r {
                Ok(_) => self.refresh_connections(),
                Err(e) => self.notify(format!("Delete failed: {e}"), true),
            },
            Msg::SessionOpened {
                tab,
                connection,
                session,
            } => {
                if let Some(t) = self.tab_mut(tab) {
                    t.session = Some((connection, session.id));
                }
            }
            Msg::Query { tab, run, event } => {
                let done = matches!(event, QueryEvent::Done { .. });
                if let Some(r) = self
                    .tab_mut(tab)
                    .and_then(|t| t.run.as_mut())
                    .filter(|r| r.id == run)
                {
                    r.apply(event);
                    let in_tx = r.in_transaction;
                    if done {
                        if let Some(t) = self.tab_mut(tab) {
                            t.in_transaction = in_tx == Some(true);
                        }
                        self.record_history(tab);
                    }
                }
            }
            Msg::QueryFailed { tab, run, error } => {
                if let Some(t) = self.tab_mut(tab) {
                    // The session may be broken; start fresh next time.
                    t.session = None;
                    if let Some(r) = t.run.as_mut().filter(|r| r.id == run) {
                        r.fail(error);
                        self.record_history(tab);
                    }
                }
            }
            Msg::Schema(m) => {
                self.schema.apply(m);
                for t in &mut self.tabs {
                    if t.completion.is_some() || t.completion_pending.is_some() {
                        t.completion_stale = true;
                    }
                }
            }
            Msg::OpenInTab {
                connection,
                title,
                text,
                run,
            } => {
                let idx = self.open_text_tab(connection, title, text.clone());
                if run {
                    self.run_sql(idx, text);
                }
            }
            Msg::FileOpened(Ok((path, text))) => {
                let conn = self.tabs.get(self.active).and_then(|t| t.connection);
                // Reuse an untouched empty tab.
                let idx = match self.tabs.get(self.active) {
                    Some(t) if t.text.is_empty() && t.path.is_none() && t.run.is_none() => {
                        self.active
                    }
                    _ => self.new_tab(conn),
                };
                let tab = &mut self.tabs[idx];
                tab.set_file(path, text);
            }
            Msg::FileOpened(Err(e)) => self.notify(format!("Open failed: {e}"), true),
            Msg::FileSaved { tab, result } => match result {
                Ok(path) => {
                    if let Some(t) = self.tab_mut(tab) {
                        t.mark_saved(path);
                    }
                }
                Err(e) => self.notify(format!("Save failed: {e}"), true),
            },
            Msg::Notice { text, error } => self.notify(text, error),
            Msg::Assistant { run, update } => self.assistant_update(run, update),
            Msg::ImportPicked {
                conn,
                schema,
                table,
                path,
            } => self.open_import(conn, schema, table, path),
            Msg::EditReady {
                tab,
                run,
                result,
                state,
            } => {
                if let Some(r) = self
                    .tab_mut(tab)
                    .and_then(|t| t.run.as_mut())
                    .filter(|r| r.id == run)
                {
                    match state {
                        Ok(s) => {
                            r.edit = Some((result, *s));
                            r.edit_status = None;
                        }
                        Err(e) => r.edit_status = Some(e),
                    }
                }
            }
            Msg::EditsApplied { tab, result } => match result {
                Ok(n) => {
                    self.dialog = Dialog::None;
                    self.notify(format!("Applied {n} change(s)."), false);
                    if let Some(idx) = self.tabs.iter().position(|t| t.id == tab) {
                        let sql = self.tabs[idx].run.as_ref().map(|r| r.sql.clone());
                        if let Some(sql) = sql {
                            self.run_sql(idx, sql);
                        }
                    }
                }
                Err(e) => {
                    if let Dialog::ApplyEdits(d) = &mut self.dialog {
                        d.busy = false;
                        d.error = Some(e);
                    }
                }
            },
            Msg::Plan { tab, run, result } => {
                if let Some(r) = self
                    .tab_mut(tab)
                    .and_then(|t| t.run.as_mut())
                    .filter(|r| r.id == run)
                {
                    match result {
                        Ok(plan) => r.set_plan(plan),
                        Err(e) => r.fail(e),
                    }
                }
            }
            Msg::ImportColumns(r) => {
                if let Dialog::Import(f) = &mut self.dialog {
                    f.set_columns(r);
                }
            }
            Msg::ImportProgress(n) => {
                if let Dialog::Import(f) = &mut self.dialog {
                    f.progress = n;
                }
            }
            Msg::ImportDone(r) => {
                if let Ok(n) = &r {
                    self.notify(format!("Imported {n} rows."), false);
                }
                if let Dialog::Import(f) = &mut self.dialog {
                    f.busy = false;
                    f.result = Some(r);
                }
            }
        }
    }

    /// Show the CSV import dialog for `path` into `schema.table`.
    pub fn open_import(&mut self, conn: Uuid, schema: String, table: String, path: PathBuf) {
        let Some(engine) = self.engine_of(Some(conn)) else {
            return;
        };
        match crate::transfer::preview_csv(&path, true) {
            Ok(preview) => {
                self.dialog = Dialog::Import(Box::new(crate::dialogs::ImportForm::new(
                    conn,
                    engine,
                    schema.clone(),
                    table.clone(),
                    path,
                    preview,
                )));
                if let Some(client) = self.service.client.clone() {
                    self.worker.run(async move {
                        Msg::ImportColumns(
                            client
                                .columns(conn, Some(&schema), &table)
                                .await
                                .map_err(|e| e.to_string()),
                        )
                    });
                }
            }
            Err(e) => self.notify(format!("Cannot read CSV: {e:#}"), true),
        }
    }

    /// Show the query plan of the statement at the cursor (plan viewer).
    pub fn explain(&mut self, idx: usize, analyze: bool) {
        let Some(client) = self.service.client.clone() else {
            return;
        };
        let engine = self.engine_of(self.tabs.get(idx).and_then(|t| t.connection));
        let Some(tab) = self.tabs.get_mut(idx) else {
            return;
        };
        let Some(conn) = tab.connection else {
            self.notify("Choose a connection for this tab first.", true);
            return;
        };
        if tab.run.as_ref().is_some_and(|r| r.running) {
            return;
        }
        let sql = tab.current_sql(engine).trim().to_string();
        if sql.is_empty() {
            return;
        }
        let run_id = self.next_run;
        self.next_run += 1;
        let mut run = Run::new(run_id, sql.clone());
        run.pane = crate::results::Pane::Plan;
        tab.run = Some(run);
        let on = match tab.session {
            Some((c, s)) if c == conn => On::Session(s),
            _ => On::Connection(conn),
        };
        let tab_id = tab.id;
        self.worker.run(async move {
            Msg::Plan {
                tab: tab_id,
                run: run_id,
                result: client
                    .explain(on, &sql, analyze)
                    .await
                    .map_err(|e| e.to_string()),
            }
        });
    }

    // ----------------------------------------------------------- editing --

    /// Whether the shown result of tab `idx` can be edited; the reason if not.
    pub fn editability(&self, idx: usize) -> Result<(), String> {
        let tab = self.tabs.get(idx).ok_or("no tab")?;
        let conn = tab
            .connection
            .and_then(|c| self.service.connection(c))
            .ok_or("no connection")?;
        if conn.read_only {
            return Err("The connection is read-only".into());
        }
        let run = tab.run.as_ref().ok_or("no results")?;
        if run.results.len() != 1 {
            return Err("Only a query with a single result can be edited".into());
        }
        crate::editing::source_table(&run.sql, conn.engine)
            .map(|_| ())
            .ok_or_else(|| {
                "Editing works for a simple SELECT from one table (no joins or grouping)".into()
            })
    }

    fn start_edit(&mut self, idx: usize) {
        let Some(client) = self.service.client.clone() else {
            return;
        };
        let Some(tab) = self.tabs.get_mut(idx) else {
            return;
        };
        let (Some(conn), Some(run)) = (tab.connection, tab.run.as_mut()) else {
            return;
        };
        let Some(engine) = self.service.connection(conn).map(|c| c.engine) else {
            return;
        };
        let Some((schema, table)) = crate::editing::source_table(&run.sql, engine) else {
            return;
        };
        run.edit_status = Some("Loading table information…".into());
        let columns = run.results[0].columns.clone();
        let (tab_id, run_id) = (tab.id, run.id);
        self.worker.run(async move {
            let state = match client.columns(conn, schema.as_deref(), &table).await {
                Ok(cols) if cols.is_empty() => Err(format!("table {table} not found")),
                Ok(cols) => crate::editing::EditState::new(engine, schema, table, &columns, &cols)
                    .map(Box::new)
                    .map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            };
            Msg::EditReady {
                tab: tab_id,
                run: run_id,
                result: 0,
                state,
            }
        });
    }

    fn review_edits(&mut self, idx: usize) {
        let Some(tab) = self.tabs.get(idx) else {
            return;
        };
        let (Some(conn), Some(run)) = (tab.connection, tab.run.as_ref()) else {
            return;
        };
        let Some((ri, e)) = run.edit.as_ref() else {
            return;
        };
        let stmts = e.statements(&run.results[*ri]);
        let previews = stmts
            .iter()
            .map(|(sql, params)| crate::editing::preview(e.engine, sql, params))
            .collect();
        self.dialog = Dialog::ApplyEdits(Box::new(crate::dialogs::ApplyEdits {
            tab: tab.id,
            connection: conn,
            engine: e.engine,
            statements: stmts,
            previews,
            busy: false,
            error: None,
        }));
    }

    // ------------------------------------------------------------ export --

    /// Export the shown result (from memory) or re-run the query to a file.
    pub fn export(&mut self, idx: usize, format: crate::transfer::Format, whole: bool) {
        use crate::transfer::{Exporter, export_query};
        let Some(tab) = self.tabs.get(idx) else {
            return;
        };
        let (Some(run), Some(conn)) = (tab.run.as_ref(), tab.connection) else {
            return;
        };
        let Some(engine) = self.engine_of(Some(conn)) else {
            return;
        };
        let stem: String = tab
            .title
            .trim_end_matches(".sql")
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let file_name = format!("{stem}.{}", format.extension());
        let pick = move || async move {
            rfd::AsyncFileDialog::new()
                .add_filter(format.label(), &[format.extension()])
                .set_file_name(&file_name)
                .save_file()
                .await
                .map(|f| f.path().to_path_buf())
        };
        if whole {
            let (Some(client), sql) = (self.service.client.clone(), run.sql.clone()) else {
                return;
            };
            self.worker.spawn(move |sink| async move {
                let Some(path) = pick().await else { return };
                let mut last = 0;
                let progress = |n: u64| {
                    if n >= last + 50_000 {
                        last = n;
                        sink.send(Msg::Notice {
                            text: format!("Exporting… {n} rows"),
                            error: false,
                        });
                    }
                };
                let res = export_query(&client, conn, engine, &sql, &path, format, progress).await;
                sink.send(match res {
                    Ok(n) => Msg::Notice {
                        text: format!("Exported {n} rows to {}", path.display()),
                        error: false,
                    },
                    Err(e) => Msg::Notice {
                        text: format!("Export failed: {e:#}"),
                        error: true,
                    },
                });
            });
        } else {
            let crate::results::Pane::Result(i) = run.pane else {
                self.notify("Show a result set to export it.", true);
                return;
            };
            let Some(rs) = run.results.get(i) else { return };
            let (columns, cells) = (rs.columns.clone(), rs.cells().to_vec());
            self.worker.spawn(move |sink| async move {
                let Some(path) = pick().await else { return };
                let res = tokio::task::spawn_blocking(move || {
                    let width = columns.len().max(1);
                    let mut ex = Exporter::create(&path, format, columns, engine, "exported")?;
                    for r in cells.chunks(width) {
                        ex.row(r)?;
                    }
                    ex.finish().map(|n| (n, path))
                })
                .await;
                sink.send(match res {
                    Ok(Ok((n, path))) => Msg::Notice {
                        text: format!("Exported {n} rows to {}", path.display()),
                        error: false,
                    },
                    Ok(Err(e)) => Msg::Notice {
                        text: format!("Export failed: {e:#}"),
                        error: true,
                    },
                    Err(e) => Msg::Notice {
                        text: format!("Export failed: {e}"),
                        error: true,
                    },
                });
            });
        }
    }

    // -------------------------------------------------------------- files --

    pub fn open_file(&mut self) {
        self.worker.run(async {
            let Some(file) = rfd::AsyncFileDialog::new()
                .add_filter("SQL", &["sql"])
                .add_filter("All files", &["*"])
                .pick_file()
                .await
            else {
                return Msg::Notice {
                    text: String::new(),
                    error: false,
                };
            };
            let path = file.path().to_path_buf();
            Msg::FileOpened(
                std::fs::read_to_string(&path)
                    .map(|t| (path, t))
                    .map_err(|e| e.to_string()),
            )
        });
    }

    pub fn save_tab(&mut self, idx: usize, save_as: bool) {
        let tab = &self.tabs[idx];
        let (id, text) = (tab.id, tab.text.clone());
        let known = tab.path.clone().filter(|_| !save_as);
        let suggested = format!("{}.sql", tab.title.trim_end_matches(".sql"));
        self.worker.run(async move {
            let path = match known {
                Some(p) => p,
                None => match rfd::AsyncFileDialog::new()
                    .add_filter("SQL", &["sql"])
                    .set_file_name(&suggested)
                    .save_file()
                    .await
                {
                    Some(f) => f.path().to_path_buf(),
                    None => {
                        return Msg::Notice {
                            text: String::new(),
                            error: false,
                        };
                    }
                },
            };
            let result = std::fs::write(&path, text)
                .map(|_| path)
                .map_err(|e| e.to_string());
            Msg::FileSaved { tab: id, result }
        });
    }

    // ---------------------------------------------------------- shortcuts --

    fn shortcuts(&mut self, ctx: &egui::Context) {
        // Dialogs and palettes own the keyboard while they are open.
        if !matches!(self.dialog, Dialog::None) || self.palette.is_some() {
            return;
        }
        let running = self
            .tabs
            .get(self.active)
            .is_some_and(|t| t.run.as_ref().is_some_and(|r| r.running));
        // Typing in the assistant: editor shortcuts (Ctrl+Enter, …) stay off.
        let assistant_typing = ctx.memory(|m| m.has_focus(crate::assistant::panel::input_id()));
        let bindings = self.keymap.bindings().to_vec();
        for (sc, cmd) in bindings {
            if assistant_typing
                && !matches!(cmd, Command::ToggleAssistant | Command::CommandPalette)
            {
                continue;
            }
            // Esc only means "cancel" while something is running.
            if cmd == Command::Cancel && !running {
                continue;
            }
            if ctx.input_mut(|i| i.consume_shortcut(&sc)) {
                self.execute(ctx, cmd);
            }
        }
    }

    /// Run a command from a shortcut, menu or the palette.
    pub fn execute(&mut self, ctx: &egui::Context, cmd: Command) {
        let idx = self.active;
        match cmd {
            Command::RunCurrent => self.editor_action(EditorAction::RunCurrent),
            Command::RunScript => self.editor_action(EditorAction::RunAll),
            Command::Cancel => self.editor_action(EditorAction::Cancel),
            Command::Explain => self.explain(idx, false),
            Command::ExplainAnalyze => self.explain(idx, true),
            Command::Commit => self.editor_action(EditorAction::Commit),
            Command::Rollback => self.editor_action(EditorAction::Rollback),
            Command::ToggleAutocommit => {
                if let Some(t) = self.tabs.get_mut(idx) {
                    t.autocommit = !t.autocommit;
                    let state = if t.autocommit { "on" } else { "off" };
                    self.notify(format!("Auto-commit {state} for this tab."), false);
                }
            }
            Command::NewTab => {
                self.new_tab(None);
            }
            Command::CloseTab => self.request_close_tab(idx),
            Command::NextTab if !self.tabs.is_empty() => self.active = (idx + 1) % self.tabs.len(),
            Command::PrevTab if !self.tabs.is_empty() => {
                self.active = (idx + self.tabs.len() - 1) % self.tabs.len()
            }
            Command::NextTab | Command::PrevTab => {}
            Command::OpenFile => self.open_file(),
            Command::Save => self.save_tab(idx, false),
            Command::SaveAs => self.save_tab(idx, true),
            Command::Find => {
                if let Some(t) = self.tabs.get_mut(idx) {
                    t.find.open = true;
                    t.find.focus = true;
                }
            }
            Command::Format => self.format_tab(idx),
            Command::SaveSnippet => {
                self.dialog = Dialog::SaveSnippet {
                    name: String::new(),
                    sql: self.current_sql_for_snippet(),
                }
            }
            Command::CommandPalette => self.palette = Some(crate::palette::Palette::new(false)),
            Command::QuickOpen => {
                self.load_all_tables();
                self.palette = Some(crate::palette::Palette::new(true));
            }
            Command::FontBigger => self.set_font_size(self.settings.editor_font_size + 1.0),
            Command::FontSmaller => self.set_font_size(self.settings.editor_font_size - 1.0),
            Command::ToggleTheme => {
                let pref = if ctx.theme() == egui::Theme::Dark {
                    ThemePref::Light
                } else {
                    ThemePref::Dark
                };
                self.settings.theme = pref;
                self.settings.save();
                apply_theme(ctx, pref);
            }
            Command::ShowConnections => self.sidebar = crate::sidebar::View::Connections,
            Command::ShowHistory => self.sidebar = crate::sidebar::View::History,
            Command::ShowSnippets => self.sidebar = crate::sidebar::View::Snippets,
            Command::ToggleAssistant => {
                self.settings.assistant.open = !self.settings.assistant.open;
                self.settings.save();
                if self.settings.assistant.open {
                    ctx.memory_mut(|m| m.request_focus(crate::assistant::panel::input_id()));
                }
            }
            Command::NewConnection => {
                self.dialog = Dialog::Connection(Box::new(ConnForm::new_default()))
            }
            Command::RefreshConnections => {
                self.refresh_connections();
                self.schema.clear();
            }
            Command::ConnectService => self.dialog = Dialog::Welcome(WelcomeForm::default()),
        }
    }

    /// Format the selection, or the whole tab.
    fn format_tab(&mut self, idx: usize) {
        let engine = self.engine_of(self.tabs.get(idx).and_then(|t| t.connection));
        let style = crate::sql::format::Style {
            uppercase: self.settings.format_uppercase,
            indent: self.settings.format_indent,
        };
        let Some(tab) = self.tabs.get_mut(idx) else {
            return;
        };
        let (a, b) = (tab.cursor.primary.index.0, tab.cursor.secondary.index.0);
        let (start, end) = if a == b {
            (0, tab.text.len())
        } else {
            (
                crate::editor::char_to_byte(&tab.text, a.min(b)),
                crate::editor::char_to_byte(&tab.text, a.max(b)),
            )
        };
        match crate::sql::format::format(&tab.text[start..end], engine, &style) {
            Ok(formatted) => {
                tab.text.replace_range(start..end, &formatted);
                tab.select_bytes(start, start + formatted.len());
            }
            Err(why) => self.notify(format!("Format: {why}"), true),
        }
    }

    /// Ask for the table lists of every schema of the active connection
    /// (for quick-open).
    fn load_all_tables(&mut self) {
        let Some(conn) = self.tabs.get(self.active).and_then(|t| t.connection) else {
            return;
        };
        let client = self.service.client.clone();
        self.schema.ensure(
            &client,
            &self.worker,
            conn,
            crate::schema::SchemaKey::Schemas,
        );
        for s in self.schema.schema_names(conn) {
            self.schema.ensure(
                &client,
                &self.worker,
                conn,
                crate::schema::SchemaKey::Tables(s),
            );
        }
    }

    /// A menu entry for `cmd`, with its current shortcut.
    fn menu_item(&mut self, ui: &mut egui::Ui, cmd: Command) {
        let label = self.keymap.label(ui.ctx(), cmd);
        if ui
            .add(egui::Button::new(cmd.title()).shortcut_text(label))
            .clicked()
        {
            let ctx = ui.ctx().clone();
            self.execute(&ctx, cmd);
            ui.close();
        }
    }

    fn set_font_size(&mut self, size: f32) {
        self.settings.editor_font_size = size.clamp(9.0, 28.0);
        self.settings.save();
    }

    pub fn editor_action(&mut self, action: EditorAction) {
        let idx = self.active;
        let Some(tab) = self.tabs.get(idx) else {
            return;
        };
        match action {
            EditorAction::RunCurrent => {
                let engine = self.engine_of(tab.connection);
                let sql = tab.current_sql(engine);
                self.run_sql(idx, sql);
            }
            EditorAction::RunAll => {
                let sql = tab.text.clone();
                self.run_sql(idx, sql);
            }
            EditorAction::Cancel => self.cancel_run(idx),
            EditorAction::Commit => self.run_sql(idx, "COMMIT".into()),
            EditorAction::Rollback => self.run_sql(idx, "ROLLBACK".into()),
        }
    }

    // ------------------------------------------------------------- layout --

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                for c in [
                    Command::NewTab,
                    Command::OpenFile,
                    Command::Save,
                    Command::SaveAs,
                    Command::CloseTab,
                ] {
                    self.menu_item(ui, c);
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                for c in [
                    Command::Find,
                    Command::Format,
                    Command::SaveSnippet,
                    Command::CommandPalette,
                    Command::QuickOpen,
                ] {
                    self.menu_item(ui, c);
                }
                ui.separator();
                let mut up = self.settings.format_uppercase;
                if ui.checkbox(&mut up, "Format: uppercase keywords").changed() {
                    self.settings.format_uppercase = up;
                    self.settings.save();
                }
                let mut auto = self.settings.autocomplete;
                if ui.checkbox(&mut auto, "Complete while typing").changed() {
                    self.settings.autocomplete = auto;
                    self.settings.save();
                }
            });
            ui.menu_button("Query", |ui| {
                for c in [
                    Command::RunCurrent,
                    Command::RunScript,
                    Command::Cancel,
                    Command::Explain,
                    Command::ExplainAnalyze,
                ] {
                    self.menu_item(ui, c);
                }
                ui.separator();
                for c in [
                    Command::Commit,
                    Command::Rollback,
                    Command::ToggleAutocommit,
                ] {
                    self.menu_item(ui, c);
                }
                ui.separator();
                ui.menu_button("Row limit", |ui| {
                    for n in [1_000u64, 10_000, 100_000, 1_000_000] {
                        if ui
                            .radio(self.settings.max_rows == n, format!("{n} rows"))
                            .clicked()
                        {
                            self.settings.max_rows = n;
                            self.settings.save();
                        }
                    }
                });
            });
            ui.menu_button("Connections", |ui| {
                self.menu_item(ui, Command::NewConnection);
                self.menu_item(ui, Command::RefreshConnections);
            });
            ui.menu_button("Service", |ui| {
                self.menu_item(ui, Command::ConnectService);
                if ui.button("Reconnect").clicked()
                    && let Some(p) = self.service.profile.clone()
                {
                    self.connect(p);
                }
                let mut auto = self.settings.autostart_local;
                if ui
                    .checkbox(&mut auto, "Start the local service automatically")
                    .changed()
                {
                    self.settings.autostart_local = auto;
                    self.settings.save();
                }
                if self.settings.services.len() > 1 {
                    ui.menu_button("Switch to", |ui| {
                        for s in self.settings.services.clone() {
                            if ui.button(format!("{}  ({})", s.name, s.url)).clicked() {
                                self.settings.active_service = Some(s.url.clone());
                                self.settings.save();
                                self.connect(s);
                            }
                        }
                    });
                }
                ui.separator();
                if ui.button("Forget this service").clicked() {
                    self.forget_service();
                }
            });
            ui.menu_button("View", |ui| {
                for (pref, label) in [
                    (ThemePref::System, "Follow system"),
                    (ThemePref::Light, "Light"),
                    (ThemePref::Dark, "Dark"),
                ] {
                    if ui.radio(self.settings.theme == pref, label).clicked() {
                        self.settings.theme = pref;
                        self.settings.save();
                        apply_theme(ui.ctx(), pref);
                    }
                }
                ui.separator();
                for c in [
                    Command::ShowConnections,
                    Command::ShowHistory,
                    Command::ShowSnippets,
                    Command::ToggleAssistant,
                    Command::FontBigger,
                    Command::FontSmaller,
                ] {
                    self.menu_item(ui, c);
                }
            });
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let dark = ui.visuals().dark_mode;
        ui.horizontal(|ui| {
            let (dot, text) = match &self.service.status {
                ServiceStatus::Connected => {
                    let p = self.service.profile.as_ref();
                    let scope = self
                        .service
                        .info
                        .as_ref()
                        .map(|i| i.scope.as_str())
                        .unwrap_or("");
                    (
                        Color32::from_rgb(0x3c, 0xb3, 0x71),
                        format!("{}  · {scope}", p.map(|p| p.url.as_str()).unwrap_or("")),
                    )
                }
                ServiceStatus::Connecting => (
                    Color32::from_rgb(0xe6, 0xa2, 0x3c),
                    "Connecting…".to_string(),
                ),
                ServiceStatus::Unconfigured => (Color32::GRAY, "No service configured".to_string()),
                ServiceStatus::Failed(e) => {
                    (Color32::from_rgb(0xd6, 0x45, 0x45), format!("Service: {e}"))
                }
            };
            ui.label(RichText::new("●").color(dot));
            let resp = ui.label(RichText::new(text).small());
            if let Some(store) = self.service.token_store {
                resp.on_hover_text(format!("Token stored in the {}", store.describe()));
            }

            if let Some(tab) = self.tabs.get(self.active)
                && let Some(conn) = tab.connection.and_then(|c| self.service.connection(c))
            {
                ui.separator();
                let color = conn
                    .color
                    .as_deref()
                    .and_then(theme::parse_hex)
                    .unwrap_or(theme::accent(dark));
                ui.label(RichText::new("●").color(color));
                let mut label = format!("{} · {}", conn.name, theme::engine_label(conn.engine));
                if let Some(env) = &conn.environment {
                    label.push_str(&format!(" · {env}"));
                }
                if conn.read_only {
                    label.push_str(" · read-only");
                }
                ui.label(RichText::new(label).small());
            }

            if let Some((text, error, at)) = &self.notice
                && !text.is_empty()
                && at.elapsed() < Duration::from_secs(8)
            {
                ui.separator();
                let color = if *error {
                    Color32::from_rgb(0xd6, 0x45, 0x45)
                } else {
                    ui.visuals().weak_text_color()
                };
                ui.label(RichText::new(text).small().color(color));
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(tab) = self.tabs.get(self.active) {
                    let (line, col) = tab.line_col();
                    ui.label(
                        RichText::new(format!("Ln {line}, Col {col}"))
                            .small()
                            .weak(),
                    );
                }
            });
        });
    }

    fn tab_strip(&mut self, ui: &mut egui::Ui) {
        let dark = ui.visuals().dark_mode;
        let mut close = None;
        let mut new_tab = false;
        egui::ScrollArea::horizontal()
            .id_salt("tab_strip")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (i, tab) in self.tabs.iter().enumerate() {
                        let conn = tab.connection.and_then(|c| self.service.connection(c));
                        let color = conn
                            .and_then(|c| c.color.as_deref())
                            .and_then(theme::parse_hex);
                        let selected = i == self.active;
                        let frame = egui::Frame::new()
                            .inner_margin(egui::Margin::symmetric(8, 3))
                            .corner_radius(6)
                            .fill(if selected {
                                ui.visuals().extreme_bg_color
                            } else {
                                Color32::TRANSPARENT
                            })
                            .stroke(if selected {
                                egui::Stroke::new(
                                    1.0,
                                    ui.visuals().widgets.noninteractive.bg_stroke.color,
                                )
                            } else {
                                egui::Stroke::NONE
                            });
                        let resp = frame
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 4.0;
                                    ui.label(RichText::new("●").color(color.unwrap_or(
                                        if tab.connection.is_some() {
                                            theme::accent(dark)
                                        } else {
                                            Color32::GRAY
                                        },
                                    )));
                                    let mut title = tab.title.clone();
                                    if tab.is_dirty() {
                                        title.push_str(" •");
                                    }
                                    let t = RichText::new(title);
                                    ui.label(if selected { t.strong() } else { t });
                                    if tab.run.as_ref().is_some_and(|r| r.running) {
                                        ui.spinner();
                                    }
                                    if ui
                                        .small_button("×")
                                        .on_hover_text("Close (Ctrl+W)")
                                        .clicked()
                                    {
                                        close = Some(i);
                                    }
                                });
                            })
                            .response
                            .interact(egui::Sense::click());
                        if resp.clicked() {
                            self.active = i;
                        }
                        if resp.middle_clicked() {
                            close = Some(i);
                        }
                    }
                    if ui.button("+").on_hover_text("New tab (Ctrl+T)").clicked() {
                        new_tab = true;
                    }
                });
            });
        if let Some(i) = close {
            self.request_close_tab(i);
        }
        if new_tab {
            self.new_tab(None);
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let idx = self.active;
        let connections = self.service.connections.clone();
        let Some(tab) = self.tabs.get_mut(idx) else {
            return;
        };
        let mut action = None;
        ui.horizontal(|ui| {
            let current = tab
                .connection
                .and_then(|c| connections.iter().find(|x| x.id == c));
            let label = current.map_or("Choose connection…".to_string(), |c| c.name.clone());
            let before = tab.connection;
            egui::ComboBox::from_id_salt(("conn_pick", tab.id))
                .selected_text(label)
                .width(220.0)
                .show_ui(ui, |ui| {
                    for c in &connections {
                        let color = c
                            .color
                            .as_deref()
                            .and_then(theme::parse_hex)
                            .unwrap_or(ui.visuals().weak_text_color());
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("●").color(color));
                            ui.selectable_value(
                                &mut tab.connection,
                                Some(c.id),
                                format!("{}  ·  {}", c.name, theme::engine_label(c.engine)),
                            );
                        });
                    }
                });
            if tab.connection != before {
                tab.session = None; // sessions are per connection
            }
            let running = tab.run.as_ref().is_some_and(|r| r.running);
            ui.add_enabled_ui(!running && tab.connection.is_some(), |ui| {
                if ui
                    .button("▶ Run")
                    .on_hover_text("Run statement at cursor or selection (Ctrl+Enter)")
                    .clicked()
                {
                    action = Some(EditorAction::RunCurrent);
                }
                if ui
                    .button("▶▶ Script")
                    .on_hover_text("Run the whole script (F5)")
                    .clicked()
                {
                    action = Some(EditorAction::RunAll);
                }
            });
            if running && ui.button("■ Stop").on_hover_text("Cancel (Esc)").clicked() {
                action = Some(EditorAction::Cancel);
            }
            ui.separator();
            ui.checkbox(&mut tab.autocommit, "Auto-commit")
                .on_hover_text("Off: the first run opens a transaction that stays open until you commit or roll back");
            if tab.in_transaction {
                ui.separator();
                ui.label(
                    RichText::new("Transaction open")
                        .color(Color32::from_rgb(0xe6, 0xa2, 0x3c))
                        .strong(),
                );
                if ui.button("Commit").clicked() {
                    action = Some(EditorAction::Commit);
                }
                if ui.button("Rollback").clicked() {
                    action = Some(EditorAction::Rollback);
                }
            }
        });
        if let Some(a) = action {
            self.editor_action(a);
        }
    }

    fn central(&mut self, ui: &mut egui::Ui) {
        self.tab_strip(ui);
        self.toolbar(ui);
        let dark = ui.visuals().dark_mode;
        let idx = self.active;
        let engine = self.engine_of(self.tabs.get(idx).and_then(|t| t.connection));
        let conn_color = self
            .tabs
            .get(idx)
            .and_then(|t| t.connection)
            .and_then(|c| self.service.connection(c))
            .and_then(|c| c.color.as_deref())
            .and_then(theme::parse_hex);
        let font_size = self.settings.editor_font_size;

        let has_run = self.tabs.get(idx).is_some_and(|t| t.run.is_some());
        let mut grid_out = crate::grid::GridOutput::default();
        let editable = self.editability(idx);
        if has_run {
            egui::Panel::bottom(egui::Id::new(("results", self.tabs[idx].id)))
                .resizable(true)
                .default_size(ui.available_height() * 0.45)
                .size_range(80.0..=ui.available_height() - 80.0)
                .show(ui, |ui| {
                    if let Some(run) = self.tabs[idx].run.as_mut() {
                        grid_out = crate::grid::results_ui(ui, run, editable.clone());
                    }
                });
        }
        if grid_out.cancel {
            self.cancel_run(idx);
        }
        if let Some(v) = grid_out.view {
            self.viewer = Some(v);
        }
        if let Some((format, whole)) = grid_out.export {
            self.export(idx, format, whole);
        }
        if grid_out.start_edit {
            self.start_edit(idx);
        }
        if grid_out.review_edits {
            self.review_edits(idx);
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                if let Some(color) = conn_color {
                    let r = ui.available_rect_before_wrap();
                    ui.painter().rect_filled(
                        egui::Rect::from_min_size(r.min, egui::vec2(r.width(), 3.0)),
                        0.0,
                        color,
                    );
                    ui.add_space(3.0);
                }
                let conn = self.tabs.get(idx).and_then(|t| t.connection);
                let catalog = conn.map(|c| crate::schema::TreeCatalog::new(&self.schema, c));
                let style = crate::sql::format::Style {
                    uppercase: self.settings.format_uppercase,
                    indent: self.settings.format_indent,
                };
                if let Some(tab) = self.tabs.get_mut(idx) {
                    let env = crate::editor::EditorEnv {
                        engine,
                        dark,
                        font_size,
                        catalog: catalog
                            .as_ref()
                            .map(|c| c as &dyn crate::sql::complete::Catalog),
                        autocomplete: self.settings.autocomplete,
                        format: &style,
                    };
                    crate::editor::editor_ui(ui, tab, &env);
                }
                // Load whatever the completer looked for and did not find.
                if let (Some(conn), Some(cat)) = (conn, catalog) {
                    let missing = cat.missing.take();
                    for key in missing {
                        self.schema
                            .ensure(&self.service.client, &self.worker, conn, key);
                    }
                }
            });
    }

    fn value_viewer(&mut self, ctx: &egui::Context) {
        let Some((title, text)) = &mut self.viewer else {
            return;
        };
        let mut open = true;
        let pretty = serde_json::from_str::<serde_json::Value>(text)
            .ok()
            .filter(|v| v.is_object() || v.is_array())
            .and_then(|v| serde_json::to_string_pretty(&v).ok());
        egui::Window::new(format!("Value · {title}"))
            .open(&mut open)
            .default_size([520.0, 360.0])
            .resizable(true)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Copy").clicked() {
                        ui.ctx().copy_text(text.clone());
                    }
                    ui.label(RichText::new(format!("{} characters", text.chars().count())).weak());
                });
                egui::ScrollArea::both().show(ui, |ui| {
                    let mut shown = pretty.clone().unwrap_or_else(|| text.clone());
                    ui.add(
                        egui::TextEdit::multiline(&mut shown)
                            .code_editor()
                            .desired_width(f32::INFINITY)
                            .interactive(true),
                    );
                });
            });
        if !open {
            self.viewer = None;
        }
    }
}

fn is_session_gone(e: &sqail_client::Error) -> bool {
    match e {
        sqail_client::Error::Api { status: 404, .. } => true,
        sqail_client::Error::Api { problem, .. } => problem
            .detail
            .as_deref()
            .is_some_and(|d| d.contains("lost")),
        _ => false,
    }
}

fn apply_theme(ctx: &egui::Context, pref: ThemePref) {
    ctx.set_theme(match pref {
        ThemePref::System => egui::ThemePreference::System,
        ThemePref::Light => egui::ThemePreference::Light,
        ThemePref::Dark => egui::ThemePreference::Dark,
    });
}

impl eframe::App for SqailApp {
    fn on_exit(&mut self) {
        self.autosave_workspace(true);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Ask before quitting with open transactions (they would roll back).
        if ctx.input(|i| i.viewport().close_requested())
            && !self.allow_close
            && self.tabs.iter().any(|t| t.in_transaction)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.dialog = Dialog::ConfirmCloseTransaction(None);
        }
        let msgs: Vec<Msg> = self.worker.drain().collect();
        for m in msgs {
            self.handle(m);
        }
        self.shortcuts(&ctx);

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("sidebar")
            .resizable(true)
            .default_size(280.0)
            .size_range(180.0..=520.0)
            .show(ui, |ui| crate::sidebar::ui(ui, self));
        if self.settings.assistant.open {
            egui::Panel::right("assistant")
                .resizable(true)
                .default_size(420.0)
                .size_range(300.0..=900.0)
                .show(ui, |ui| crate::assistant::panel::ui(ui, self));
        }
        egui::CentralPanel::default().show(ui, |ui| self.central(ui));

        self.value_viewer(&ctx);
        crate::palette::show(&ctx, self);
        self.autosave_workspace(false);
        crate::dialogs::show(&ctx, self);

        let running = self
            .tabs
            .iter()
            .any(|t| t.run.as_ref().is_some_and(|r| r.running));
        if running || matches!(self.service.status, ServiceStatus::Connecting) {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if self
            .notice
            .as_ref()
            .is_some_and(|(_, _, at)| at.elapsed() < Duration::from_secs(8))
        {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }
}
