//! Modal dialogs: service setup (first run), connection editor, confirmations.

use egui::{Color32, Id, Modal, RichText};
use sqail_client::proto::{
    Connection, ConnectionInput, ConnectionParams, MssqlAuth, MssqlEncrypt, MssqlParams, PgSslMode,
    PostgresParams, SqliteParams, TestResult,
};
use uuid::Uuid;

use crate::app::{Msg, SqailApp};
use crate::local_service;
use crate::settings::ServiceProfile;
use crate::theme;

#[derive(Default)]
pub enum Dialog {
    #[default]
    None,
    Welcome(WelcomeForm),
    Connection(Box<ConnForm>),
    ConfirmDelete(Uuid, String),
    ConfirmClose(usize),
    SaveSnippet {
        name: String,
        sql: String,
    },
    Import(Box<ImportForm>),
    ApplyEdits(Box<ApplyEdits>),
    DropTable(Box<DropTable>),
    /// Closing a tab (`Some(index)`) or quitting (`None`) with open transactions.
    ConfirmCloseTransaction(Option<usize>),
}

// ---------------------------------------------------------------- welcome --

#[derive(Default)]
pub struct WelcomeForm {
    pub url: String,
    pub token: String,
    pub busy: bool,
    pub error: Option<String>,
    /// Fingerprint seen on the server, awaiting the user's trust decision.
    pub fingerprint: Option<String>,
}

impl WelcomeForm {
    pub fn for_url(url: &str) -> Self {
        Self {
            url: url.into(),
            ..Default::default()
        }
    }

    pub fn on_probed(&mut self, r: Result<(String, sqail_client::proto::Health), String>) {
        self.busy = false;
        match r {
            Ok((fp, _)) => self.fingerprint = Some(fp),
            Err(e) => self.error = Some(e),
        }
    }
}

fn welcome(ctx: &egui::Context, app: &mut SqailApp) {
    let Dialog::Welcome(form) = &mut app.dialog else {
        return;
    };
    let mut close = false;
    let mut provision = false;
    let mut probe = false;
    let mut adopt: Option<(ServiceProfile, String)> = None;
    let has_service = app.service.client.is_some();
    let local_available = local_service::find_binary().is_some();

    let resp = Modal::new(Id::new("welcome")).show(ctx, |ui| {
        ui.set_width(520.0);
        ui.heading("Connect sqail to a sqail-service");
        ui.label(
            RichText::new("sqail never talks to databases directly. A sqail-service holds the connections and speaks HTTPS.")
                .weak(),
        );
        ui.add_space(8.0);

        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.strong("This computer");
            ui.label("Start (if needed) the sqail-service installed next to sqail and create a token for you.");
            ui.add_enabled_ui(!form.busy && local_available, |ui| {
                if ui.button("Use the local service").clicked() {
                    provision = true;
                }
            });
            if !local_available {
                ui.label(RichText::new("sqail-service was not found next to sqail or on PATH.").weak().small());
            }
        });
        ui.add_space(6.0);

        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.strong("Another service");
            egui::Grid::new("remote").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                let l = ui.label("URL");
                ui.add(egui::TextEdit::singleline(&mut form.url).hint_text("https://db-gateway:7443").desired_width(340.0)).labelled_by(l.id);
                ui.end_row();
                let l = ui.label("Token");
                ui.add(egui::TextEdit::singleline(&mut form.token).password(true).hint_text("sq2_…").desired_width(340.0)).labelled_by(l.id);
                ui.end_row();
            });
            match form.fingerprint.clone() {
                None => {
                    let ready = !form.busy && form.url.starts_with("https://") && !form.token.trim().is_empty();
                    if ui.add_enabled(ready, egui::Button::new("Connect")).clicked() {
                        probe = true;
                    }
                }
                Some(fp) => {
                    ui.add_space(4.0);
                    ui.label("The service presented this certificate (SHA-256). Compare it with the output of `sqail-service fingerprint` on the server:");
                    ui.add(egui::Label::new(RichText::new(&fp).monospace().small()).wrap());
                    ui.horizontal(|ui| {
                        let profile = |fingerprint: Option<String>| ServiceProfile {
                            name: form.url.trim_start_matches("https://").to_string(),
                            url: form.url.trim_end_matches('/').to_string(),
                            fingerprint,
                            local: false,
                            client_cert: None,
                            client_key: None,
                        };
                        if ui.button("Trust and connect").clicked() {
                            adopt = Some((profile(Some(fp.clone())), form.token.trim().to_string()));
                        }
                        if ui
                            .button("Use system trust")
                            .on_hover_text("For certificates issued by a CA your OS trusts")
                            .clicked()
                        {
                            adopt = Some((profile(None), form.token.trim().to_string()));
                        }
                        if ui.button("Back").clicked() {
                            form.fingerprint = None;
                        }
                    });
                }
            }
        });

        if form.busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Working…");
            });
        }
        if let Some(e) = &form.error {
            ui.colored_label(Color32::from_rgb(0xd6, 0x45, 0x45), e);
        }
        if has_service {
            ui.separator();
            if ui.button("Cancel").clicked() {
                close = true;
            }
        }
    });
    if resp.should_close() && has_service {
        close = true;
    }

    if provision {
        form.busy = true;
        form.error = None;
        app.worker.run(async {
            Msg::Provisioned(
                local_service::provision()
                    .await
                    .map_err(|e| format!("{e:#}")),
            )
        });
    }
    if probe {
        form.busy = true;
        form.error = None;
        let url = form.url.trim().trim_end_matches('/').to_string();
        app.worker.run(async move {
            Msg::Probed(sqail_client::probe(&url).await.map_err(|e| e.to_string()))
        });
    }
    if let Some((profile, token)) = adopt {
        app.adopt_service(profile, &token);
    } else if close {
        app.dialog = Dialog::None;
    }
}

// ------------------------------------------------------------- connection --

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MsAuthChoice {
    Sql,
    Integrated,
    EntraPassword,
    EntraServicePrincipal,
    EntraManagedIdentity,
}

impl MsAuthChoice {
    const ALL: [Self; 5] = [
        Self::Sql,
        Self::Integrated,
        Self::EntraPassword,
        Self::EntraServicePrincipal,
        Self::EntraManagedIdentity,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Sql => "SQL login",
            Self::Integrated => "Windows (integrated)",
            Self::EntraPassword => "Microsoft Entra password",
            Self::EntraServicePrincipal => "Microsoft Entra service principal",
            Self::EntraManagedIdentity => "Microsoft Entra managed identity",
        }
    }

    fn uses_user(self) -> bool {
        matches!(self, Self::Sql | Self::EntraPassword)
    }

    fn uses_password(self) -> bool {
        matches!(
            self,
            Self::Sql | Self::EntraPassword | Self::EntraServicePrincipal
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EngineChoice {
    Postgres,
    Mssql,
    Sqlite,
}

/// Which PostgreSQL certificate the file dialog is choosing.
#[derive(Clone, Copy)]
pub enum PgCertKind {
    Root,
    Client,
    Key,
}

/// Databases offered next to the Database field.
enum DbList {
    Loading,
    Ready(Vec<String>),
    Failed(String),
}

pub struct ConnForm {
    editing: Option<Uuid>,
    name: String,
    engine: EngineChoice,
    host: String,
    port: String,
    database: String,
    user: String,
    password: String,
    has_password: bool,
    pg_ssl: PgSslMode,
    pg_root_cert: String,
    pg_root_name: String,
    pg_client_cert: String,
    pg_client_cert_name: String,
    pg_client_key: String,
    pg_client_key_name: String,
    has_ssl_client_key: bool,
    /// The user cleared a stored client key and has not chosen a new one.
    pg_key_cleared: bool,
    /// A certificate file dialog to open on the next frame.
    pick: Option<PgCertKind>,
    ms_auth: MsAuthChoice,
    ms_tenant: String,
    ms_client_id: String,
    /// Entra password client-app override; API-only, kept when editing.
    ms_pw_client_id: Option<String>,
    ms_instance: String,
    ms_encrypt: MssqlEncrypt,
    ms_trust: bool,
    sqlite_path: String,
    sqlite_create: bool,
    read_only: bool,
    color: Option<String>,
    environment: String,
    folder: String,
    test: Option<Result<TestResult, String>>,
    /// The ⏷ list next to Database: the settings it was fetched for, and the result.
    dbs: Option<(String, DbList)>,
    testing: bool,
    pub saving: bool,
    pub error: Option<String>,
}

impl ConnForm {
    pub fn new_default() -> Self {
        Self {
            editing: None,
            name: String::new(),
            engine: EngineChoice::Postgres,
            host: "127.0.0.1".into(),
            port: "5432".into(),
            database: String::new(),
            user: String::new(),
            password: String::new(),
            has_password: false,
            pg_ssl: PgSslMode::Prefer,
            pg_root_cert: String::new(),
            pg_root_name: String::new(),
            pg_client_cert: String::new(),
            pg_client_cert_name: String::new(),
            pg_client_key: String::new(),
            pg_client_key_name: String::new(),
            has_ssl_client_key: false,
            pg_key_cleared: false,
            pick: None,
            ms_auth: MsAuthChoice::Sql,
            ms_tenant: String::new(),
            ms_client_id: String::new(),
            ms_pw_client_id: None,
            ms_instance: String::new(),
            ms_encrypt: MssqlEncrypt::Required,
            ms_trust: false,
            sqlite_path: String::new(),
            sqlite_create: false,
            read_only: false,
            color: None,
            environment: String::new(),
            folder: String::new(),
            test: None,
            dbs: None,
            testing: false,
            saving: false,
            error: None,
        }
    }

    pub fn edit(c: &Connection) -> Self {
        let mut f = Self::new_default();
        f.editing = Some(c.id);
        f.name = c.name.clone();
        f.has_password = c.has_password;
        f.read_only = c.read_only;
        f.color = c.color.clone();
        f.environment = c.environment.clone().unwrap_or_default();
        f.folder = c.folder.clone().unwrap_or_default();
        match &c.params {
            ConnectionParams::Postgres(p) => {
                f.engine = EngineChoice::Postgres;
                f.host = p.host.clone();
                f.port = p.port.to_string();
                f.database = p.database.clone();
                f.user = p.user.clone();
                f.pg_ssl = p.ssl_mode;
                f.pg_root_cert = p.ssl_root_cert.clone().unwrap_or_default();
                f.pg_client_cert = p.ssl_client_cert.clone().unwrap_or_default();
                f.has_ssl_client_key = c.has_ssl_client_key;
            }
            ConnectionParams::Mssql(p) => {
                f.engine = EngineChoice::Mssql;
                f.host = p.host.clone();
                f.port = p.port.to_string();
                f.database = p.database.clone().unwrap_or_default();
                f.ms_instance = p.instance.clone().unwrap_or_default();
                f.ms_encrypt = p.encrypt;
                f.ms_trust = p.trust_server_certificate;
                match &p.auth {
                    MssqlAuth::Sql { user } => f.user = user.clone(),
                    MssqlAuth::Integrated => f.ms_auth = MsAuthChoice::Integrated,
                    MssqlAuth::EntraPassword {
                        user,
                        tenant,
                        client_id,
                    } => {
                        f.ms_auth = MsAuthChoice::EntraPassword;
                        f.user = user.clone();
                        f.ms_tenant = tenant.clone().unwrap_or_default();
                        f.ms_pw_client_id = client_id.clone();
                    }
                    MssqlAuth::EntraServicePrincipal { tenant, client_id } => {
                        f.ms_auth = MsAuthChoice::EntraServicePrincipal;
                        f.ms_tenant = tenant.clone();
                        f.ms_client_id = client_id.clone();
                    }
                    MssqlAuth::EntraManagedIdentity { client_id } => {
                        f.ms_auth = MsAuthChoice::EntraManagedIdentity;
                        f.ms_client_id = client_id.clone().unwrap_or_default();
                    }
                }
            }
            ConnectionParams::Sqlite(p) => {
                f.engine = EngineChoice::Sqlite;
                f.sqlite_path = p.path.clone();
                f.sqlite_create = p.create;
            }
        }
        f
    }

    pub fn on_tested(&mut self, r: Result<TestResult, String>) {
        self.testing = false;
        self.test = Some(r);
    }

    pub fn on_databases(&mut self, key: String, r: Result<Vec<String>, String>) {
        // Ignore a list for settings that have changed since.
        if self.dbs.as_ref().is_some_and(|(k, _)| *k == key) {
            self.dbs = Some((
                key,
                match r {
                    Ok(names) => DbList::Ready(names),
                    Err(e) => DbList::Failed(e),
                },
            ));
        }
    }

    /// What the database list depends on: everything but the database itself.
    fn dbs_key(&self) -> Result<(String, ConnectionInput), String> {
        let input = self.input()?;
        let mut params = input.params.clone();
        match &mut params {
            ConnectionParams::Postgres(p) => p.database.clear(),
            ConnectionParams::Mssql(p) => p.database = None,
            ConnectionParams::Sqlite(_) => {}
        }
        Ok((
            format!("{params:?}|{:?}|{:?}", input.password, input.ssl_client_key),
            input,
        ))
    }

    fn input(&self) -> Result<ConnectionInput, String> {
        let port = |default: u16| -> Result<u16, String> {
            if self.port.trim().is_empty() {
                Ok(default)
            } else {
                self.port
                    .trim()
                    .parse()
                    .map_err(|_| "port must be a number".to_string())
            }
        };
        let opt = |s: &str| (!s.trim().is_empty()).then(|| s.trim().to_string());
        if self.engine == EngineChoice::Postgres {
            check_pem("CA certificate", &self.pg_root_cert, true)?;
            check_pem("client certificate", &self.pg_client_cert, true)?;
            check_pem("client key", &self.pg_client_key, false)?;
        }
        let params = match self.engine {
            EngineChoice::Postgres => ConnectionParams::Postgres(PostgresParams {
                host: self.host.trim().into(),
                port: port(5432)?,
                database: self.database.trim().into(),
                user: self.user.trim().into(),
                ssl_mode: self.pg_ssl,
                ssl_root_cert: opt(&self.pg_root_cert),
                ssl_client_cert: opt(&self.pg_client_cert),
            }),
            EngineChoice::Mssql => ConnectionParams::Mssql(MssqlParams {
                host: self.host.trim().into(),
                port: port(1433)?,
                instance: opt(&self.ms_instance),
                database: opt(&self.database),
                auth: match self.ms_auth {
                    MsAuthChoice::Sql => MssqlAuth::Sql {
                        user: self.user.trim().into(),
                    },
                    MsAuthChoice::Integrated => MssqlAuth::Integrated,
                    MsAuthChoice::EntraPassword => MssqlAuth::EntraPassword {
                        user: self.user.trim().into(),
                        tenant: opt(&self.ms_tenant),
                        client_id: self.ms_pw_client_id.clone(),
                    },
                    MsAuthChoice::EntraServicePrincipal => MssqlAuth::EntraServicePrincipal {
                        tenant: self.ms_tenant.trim().into(),
                        client_id: self.ms_client_id.trim().into(),
                    },
                    MsAuthChoice::EntraManagedIdentity => MssqlAuth::EntraManagedIdentity {
                        client_id: opt(&self.ms_client_id),
                    },
                },
                encrypt: self.ms_encrypt,
                trust_server_certificate: self.ms_trust,
            }),
            EngineChoice::Sqlite => ConnectionParams::Sqlite(SqliteParams {
                path: self.sqlite_path.trim().into(),
                create: self.sqlite_create,
            }),
        };
        let name = if self.name.trim().is_empty() {
            match self.engine {
                EngineChoice::Sqlite => self
                    .sqlite_path
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or("sqlite")
                    .to_string(),
                _ => format!("{}/{}", self.host.trim(), self.database.trim()),
            }
        } else {
            self.name.trim().to_string()
        };
        Ok(ConnectionInput {
            name,
            params,
            // Empty field while editing = keep the stored password.
            password: if self.password.is_empty() && self.editing.is_some() {
                None
            } else {
                Some(self.password.clone())
            },
            ssl_client_key: self.ssl_client_key()?,
            read_only: self.read_only,
            color: self.color.clone(),
            environment: opt(&self.environment),
            folder: opt(&self.folder),
        })
    }

    /// `None` keeps a stored key. `Some("")` clears it.
    fn ssl_client_key(&self) -> Result<Option<String>, String> {
        if self.engine != EngineChoice::Postgres {
            return Ok(None);
        }
        if !self.pg_client_key.is_empty() {
            return Ok(Some(self.pg_client_key.clone()));
        }
        if self.pg_client_cert.trim().is_empty() {
            return Ok(self.has_ssl_client_key.then(String::new));
        }
        if self.editing.is_some() && self.has_ssl_client_key && !self.pg_key_cleared {
            return Ok(None);
        }
        Err("Choose the private key for the client certificate".into())
    }

    pub fn set_cert(&mut self, kind: PgCertKind, name: String, pem: String) {
        let pem = pem.trim().to_string();
        let label = match kind {
            PgCertKind::Root => "CA certificate",
            PgCertKind::Client => "client certificate",
            PgCertKind::Key => "client key",
        };
        if let Err(e) = check_pem(label, &pem, !matches!(kind, PgCertKind::Key)) {
            self.error = Some(e);
            return;
        }
        match kind {
            PgCertKind::Root => {
                self.pg_root_cert = pem;
                self.pg_root_name = name;
            }
            PgCertKind::Client => {
                self.pg_client_cert = pem;
                self.pg_client_cert_name = name;
            }
            PgCertKind::Key => {
                self.pg_client_key = pem;
                self.pg_client_key_name = name;
                self.pg_key_cleared = false;
            }
        }
        self.test = None;
        self.error = None;
    }
}

fn ssl_label(mode: PgSslMode) -> &'static str {
    match mode {
        PgSslMode::Disable => "disable",
        PgSslMode::Prefer => "prefer",
        PgSslMode::Require => "require",
        PgSslMode::VerifyCa => "verify-ca",
        PgSslMode::VerifyFull => "verify-full",
    }
}

fn check_pem(label: &str, pem: &str, certificate: bool) -> Result<(), String> {
    let pem = pem.trim();
    if pem.is_empty() {
        return Ok(());
    }
    if pem.len() > 64 * 1024 {
        return Err(format!("{label} is larger than 64 KiB"));
    }
    if !certificate && pem.contains("ENCRYPTED") {
        return Err(format!(
            "{label} is encrypted; sqail needs an unencrypted PEM private key"
        ));
    }
    let marker = if certificate {
        "BEGIN CERTIFICATE"
    } else {
        "PRIVATE KEY"
    };
    if !pem.contains(marker) {
        return Err(format!(
            "{label} is not a PEM {}",
            if certificate {
                "certificate"
            } else {
                "private key"
            }
        ));
    }
    Ok(())
}

fn cert_row(ui: &mut egui::Ui, form: &mut ConnForm, kind: PgCertKind) {
    let (label, pem_set, name, stored, hover, choose, clear) = match kind {
        PgCertKind::Root => (
            "CA certificate",
            !form.pg_root_cert.trim().is_empty(),
            form.pg_root_name.clone(),
            false,
            "PEM of the CA that signed the server. Used with verify-ca and verify-full, instead of the system trust store.",
            "Choose CA",
            "Clear CA",
        ),
        PgCertKind::Client => (
            "Client certificate",
            !form.pg_client_cert.trim().is_empty(),
            form.pg_client_cert_name.clone(),
            false,
            "PEM, leaf first. For servers that require a client certificate.",
            "Choose certificate",
            "Clear certificate",
        ),
        PgCertKind::Key => (
            "Client key",
            !form.pg_client_key.is_empty(),
            form.pg_client_key_name.clone(),
            form.has_ssl_client_key && !form.pg_key_cleared,
            "Unencrypted PEM private key (PKCS#8, PKCS#1 or SEC1) matching the client certificate.",
            "Choose key",
            "Clear key",
        ),
    };
    let status = if !name.is_empty() {
        name
    } else if pem_set || stored {
        "stored".into()
    } else {
        "not set".into()
    };
    let l = ui.label(label);
    ui.horizontal(|ui| {
        ui.label(status).labelled_by(l.id).on_hover_text(hover);
        if ui.button(choose).clicked() {
            form.pick = Some(kind);
        }
        if (pem_set || stored) && ui.button(clear).clicked() {
            match kind {
                PgCertKind::Root => {
                    form.pg_root_cert.clear();
                    form.pg_root_name.clear();
                }
                PgCertKind::Client => {
                    form.pg_client_cert.clear();
                    form.pg_client_cert_name.clear();
                }
                PgCertKind::Key => {
                    form.pg_client_key.clear();
                    form.pg_client_key_name.clear();
                    form.pg_key_cleared = true;
                }
            }
            form.test = None;
        }
    });
    ui.end_row();
}

/// The ⏷ list next to Database. Returns whether to fetch the list.
fn database_menu(ui: &mut egui::Ui, form: &mut ConnForm) -> bool {
    ui.set_min_width(220.0);
    let key = match form.dbs_key() {
        Ok((key, _)) => key,
        Err(e) => {
            ui.colored_label(Color32::from_rgb(0xd6, 0x45, 0x45), e);
            return false;
        }
    };
    let Some((_, list)) = form.dbs.as_ref().filter(|(k, _)| *k == key) else {
        // Never fetched, or the settings changed since.
        ui.spinner();
        return true;
    };
    let mut refresh = false;
    match list {
        DbList::Loading => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Loading databases…");
            });
        }
        DbList::Failed(e) => {
            ui.set_max_width(360.0);
            ui.colored_label(Color32::from_rgb(0xd6, 0x45, 0x45), e.as_str());
            refresh = ui.button("Try again").clicked();
        }
        DbList::Ready(names) if names.is_empty() => {
            ui.label("No databases found for this login.");
        }
        DbList::Ready(names) => {
            let mut picked = None;
            egui::ScrollArea::vertical()
                .max_height(300.0)
                .show(ui, |ui| {
                    for name in names {
                        if ui
                            .selectable_label(form.database == *name, name.as_str())
                            .clicked()
                        {
                            picked = Some(name.clone());
                        }
                    }
                });
            if let Some(name) = picked {
                form.database = name;
                ui.close();
            }
            ui.separator();
            refresh = ui.button("Refresh").clicked();
        }
    }
    refresh
}

fn connection_form(ctx: &egui::Context, app: &mut SqailApp) {
    let Dialog::Connection(form) = &mut app.dialog else {
        return;
    };
    let mut close = false;
    let mut test = false;
    let mut want_dbs = false;
    let mut save = false;
    let before = form.engine;

    let resp = Modal::new(Id::new("conn_form")).show(ctx, |ui| {
        ui.set_width(560.0);
        ui.heading(if form.editing.is_some() {
            "Edit connection"
        } else {
            "New connection"
        });
        ui.add_space(4.0);
        // The certificate rows make this taller than a short window. Keep
        // Test and Save on screen.
        egui::ScrollArea::vertical()
            .max_height((ui.ctx().content_rect().height() - 180.0).max(240.0))
            .show(ui, |ui| {
                egui::Grid::new("conn_grid")
                    .num_columns(2)
                    .spacing([10.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Engine");
                        ui.horizontal(|ui| {
                            ui.selectable_value(
                                &mut form.engine,
                                EngineChoice::Postgres,
                                "PostgreSQL",
                            );
                            ui.selectable_value(
                                &mut form.engine,
                                EngineChoice::Mssql,
                                "SQL Server",
                            );
                            ui.selectable_value(&mut form.engine, EngineChoice::Sqlite, "SQLite");
                        });
                        ui.end_row();
                        let l = ui.label("Name");
                        ui.add(
                            egui::TextEdit::singleline(&mut form.name)
                                .hint_text("defaults to host/database")
                                .desired_width(320.0),
                        )
                        .labelled_by(l.id);
                        ui.end_row();

                        match form.engine {
                            EngineChoice::Postgres | EngineChoice::Mssql => {
                                let host = ui.label("Host");
                                ui.horizontal(|ui| {
                                    ui.add(
                                        egui::TextEdit::singleline(&mut form.host)
                                            .desired_width(230.0),
                                    )
                                    .labelled_by(host.id);
                                    let l = ui.label("Port");
                                    ui.add(
                                        egui::TextEdit::singleline(&mut form.port)
                                            .desired_width(60.0),
                                    )
                                    .labelled_by(l.id);
                                });
                                ui.end_row();
                                if form.engine == EngineChoice::Mssql {
                                    let l = ui.label("Instance");
                                    ui.add(
                                egui::TextEdit::singleline(&mut form.ms_instance)
                                    .hint_text("optional, e.g. SQLEXPRESS")
                                    .desired_width(320.0),
                            )
                            .labelled_by(l.id)
                            .on_hover_text(
                                "Named instance: its port is looked up via SQL Browser (UDP 1434) \
                                 while Port is 1433. Enter the instance's real port instead to \
                                 connect directly. The host field also accepts SERVER\\INSTANCE \
                                 and SERVER,PORT.",
                            );
                                    ui.end_row();
                                }
                                let l = ui.label("Database");
                                ui.horizontal(|ui| {
                                    ui.add(
                                        egui::TextEdit::singleline(&mut form.database)
                                            .desired_width(290.0),
                                    )
                                    .labelled_by(l.id);
                                    ui.menu_button("⏷", |ui| {
                                        want_dbs = database_menu(ui, form);
                                    })
                                    .response
                                    .on_hover_text(
                                        "Choose from the databases this login can access",
                                    );
                                });
                                ui.end_row();
                                if form.engine == EngineChoice::Mssql {
                                    let l = ui.label("Authentication");
                                    egui::ComboBox::from_id_salt("ms_auth")
                                .selected_text(form.ms_auth.label())
                                .width(320.0)
                                .show_ui(ui, |ui| {
                                    for a in MsAuthChoice::ALL {
                                        ui.selectable_value(&mut form.ms_auth, a, a.label());
                                    }
                                })
                                .response
                                .labelled_by(l.id)
                                .on_hover_text(
                                    "Azure SQL: a SQL login, or Microsoft Entra ID. Entra sign-in \
                                     happens on the service host; managed identity only works when \
                                     sqail-service runs on Azure.",
                                );
                                    ui.end_row();
                                    let tenant = match form.ms_auth {
                                        MsAuthChoice::EntraServicePrincipal => Some("required"),
                                        MsAuthChoice::EntraPassword => {
                                            Some("optional, e.g. contoso.com")
                                        }
                                        _ => None,
                                    };
                                    if let Some(hint) = tenant {
                                        let l = ui.label("Tenant");
                                        ui.add(
                                            egui::TextEdit::singleline(&mut form.ms_tenant)
                                                .hint_text(hint)
                                                .desired_width(320.0),
                                        )
                                        .labelled_by(l.id)
                                        .on_hover_text("Directory (tenant) ID or domain");
                                        ui.end_row();
                                    }
                                    let client_id = match form.ms_auth {
                                        MsAuthChoice::EntraServicePrincipal => {
                                            Some("application ID")
                                        }
                                        MsAuthChoice::EntraManagedIdentity => {
                                            Some("optional: a user-assigned identity")
                                        }
                                        _ => None,
                                    };
                                    if let Some(hint) = client_id {
                                        let l = ui.label("Client ID");
                                        ui.add(
                                            egui::TextEdit::singleline(&mut form.ms_client_id)
                                                .hint_text(hint)
                                                .desired_width(320.0),
                                        )
                                        .labelled_by(l.id);
                                        ui.end_row();
                                    }
                                }
                                let ms =
                                    (form.engine == EngineChoice::Mssql).then_some(form.ms_auth);
                                if ms.is_none_or(MsAuthChoice::uses_user) {
                                    let l = ui.label("User");
                                    let hint = if ms == Some(MsAuthChoice::EntraPassword) {
                                        "user@contoso.com"
                                    } else {
                                        ""
                                    };
                                    ui.add(
                                        egui::TextEdit::singleline(&mut form.user)
                                            .hint_text(hint)
                                            .desired_width(320.0),
                                    )
                                    .labelled_by(l.id);
                                    ui.end_row();
                                }
                                if ms.is_none_or(MsAuthChoice::uses_password) {
                                    let pw_label = ui.label(
                                        if ms == Some(MsAuthChoice::EntraServicePrincipal) {
                                            "Client secret"
                                        } else {
                                            "Password"
                                        },
                                    );
                                    let hint = if form.has_password { "unchanged" } else { "" };
                                    ui.add(
                                        egui::TextEdit::singleline(&mut form.password)
                                            .password(true)
                                            .hint_text(hint)
                                            .desired_width(320.0),
                                    )
                                    .labelled_by(pw_label.id);
                                    ui.end_row();
                                }
                                if form.engine == EngineChoice::Postgres {
                                    ui.label("SSL mode");
                                    egui::ComboBox::from_id_salt("pg_ssl")
                                .selected_text(ssl_label(form.pg_ssl))
                                .show_ui(ui, |ui| {
                                    for m in [
                                        PgSslMode::Disable,
                                        PgSslMode::Prefer,
                                        PgSslMode::Require,
                                        PgSslMode::VerifyCa,
                                        PgSslMode::VerifyFull,
                                    ] {
                                        ui.selectable_value(&mut form.pg_ssl, m, ssl_label(m));
                                    }
                                })
                                .response
                                .on_hover_text(
                                    "prefer tries TLS without checking the certificate. require \
                                     demands TLS. verify-ca checks the CA. verify-full also checks \
                                     the host name.",
                                );
                                    ui.end_row();
                                    cert_row(ui, form, PgCertKind::Root);
                                    cert_row(ui, form, PgCertKind::Client);
                                    cert_row(ui, form, PgCertKind::Key);
                                } else {
                                    ui.label("Encryption");
                                    ui.horizontal(|ui| {
                                        egui::ComboBox::from_id_salt("ms_enc")
                                            .selected_text(format!("{:?}", form.ms_encrypt))
                                            .show_ui(ui, |ui| {
                                                for m in [
                                                    MssqlEncrypt::Required,
                                                    MssqlEncrypt::On,
                                                    MssqlEncrypt::Off,
                                                ] {
                                                    ui.selectable_value(
                                                        &mut form.ms_encrypt,
                                                        m,
                                                        format!("{m:?}"),
                                                    );
                                                }
                                            });
                                        ui.checkbox(&mut form.ms_trust, "Trust server certificate");
                                    });
                                    ui.end_row();
                                }
                            }
                            EngineChoice::Sqlite => {
                                let l = ui.label("Database file");
                                ui.add(
                                    egui::TextEdit::singleline(&mut form.sqlite_path)
                                        .hint_text("absolute path on the service host")
                                        .desired_width(320.0),
                                )
                                .labelled_by(l.id);
                                ui.end_row();
                                ui.label("");
                                ui.checkbox(
                                    &mut form.sqlite_create,
                                    "Create the file if it does not exist",
                                );
                                ui.end_row();
                            }
                        }

                        ui.label("Environment");
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut form.environment)
                                    .hint_text("dev, test, prod…")
                                    .desired_width(120.0),
                            );
                            let l = ui.label("Folder");
                            ui.add(
                                egui::TextEdit::singleline(&mut form.folder).desired_width(120.0),
                            )
                            .labelled_by(l.id);
                        });
                        ui.end_row();
                        ui.label("Colour");
                        ui.horizontal(|ui| {
                            if ui.selectable_label(form.color.is_none(), "none").clicked() {
                                form.color = None;
                            }
                            for (name, hex) in theme::CONNECTION_COLORS {
                                let c = theme::parse_hex(hex).unwrap_or(Color32::GRAY);
                                let selected = form.color.as_deref() == Some(*hex);
                                let text = RichText::new("●").color(c).size(18.0);
                                if ui
                                    .add(egui::Button::new(text).frame(selected))
                                    .on_hover_text(*name)
                                    .clicked()
                                {
                                    form.color = Some(hex.to_string());
                                }
                            }
                        });
                        ui.end_row();
                        ui.label("");
                        ui.checkbox(&mut form.read_only, "Read-only");
                        ui.end_row();
                    });
            });

        ui.add_space(6.0);
        match &form.test {
            Some(Ok(r)) if r.ok => {
                ui.colored_label(
                    Color32::from_rgb(0x3c, 0xb3, 0x71),
                    format!(
                        "Connected in {} ms — {}",
                        r.latency_ms,
                        r.server_version
                            .as_deref()
                            .unwrap_or("")
                            .lines()
                            .next()
                            .unwrap_or("")
                    ),
                );
            }
            Some(Ok(r)) => {
                ui.colored_label(
                    Color32::from_rgb(0xd6, 0x45, 0x45),
                    r.error.clone().unwrap_or_default(),
                );
            }
            Some(Err(e)) => {
                ui.colored_label(Color32::from_rgb(0xd6, 0x45, 0x45), e);
            }
            None => {}
        }
        if let Some(e) = &form.error {
            ui.colored_label(Color32::from_rgb(0xd6, 0x45, 0x45), e);
        }
        ui.separator();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!form.testing, egui::Button::new("Test"))
                .clicked()
            {
                test = true;
            }
            if form.testing {
                ui.spinner();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        !form.saving,
                        egui::Button::new(RichText::new("Save").strong()),
                    )
                    .clicked()
                {
                    save = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
    });
    if resp.should_close() {
        close = true;
    }
    if form.engine != before {
        form.port = match form.engine {
            EngineChoice::Postgres => "5432".into(),
            EngineChoice::Mssql => "1433".into(),
            EngineChoice::Sqlite => String::new(),
        };
        form.test = None;
    }

    let Some(client) = app.service.client.clone() else {
        return;
    };
    if want_dbs {
        match form.dbs_key() {
            Err(e) => form.dbs = Some((String::new(), DbList::Failed(e))),
            Ok((key, input)) => {
                form.dbs = Some((key.clone(), DbList::Loading));
                // Editing without retyping the password: use the stored one.
                let secret_from = form
                    .editing
                    .filter(|_| input.password.is_none() && form.has_password);
                let client = client.clone();
                app.worker.run(async move {
                    let r = client.databases_unsaved(&input, secret_from).await;
                    Msg::ConnDatabases(
                        key,
                        r.map(|v| v.into_iter().map(|d| d.name).collect())
                            .map_err(|e| e.to_string()),
                    )
                });
            }
        }
    }
    if test || save {
        match form.input() {
            Err(e) => form.error = Some(e),
            Ok(input) if test => {
                form.testing = true;
                form.test = None;
                form.error = None;
                // Omitted password or client key: the service still has them.
                let secret_from = form.editing.filter(|_| {
                    (input.password.is_none() && form.has_password)
                        || (input.ssl_client_key.is_none() && form.has_ssl_client_key)
                });
                app.worker.run(async move {
                    let r = client.test_unsaved(&input, secret_from).await;
                    Msg::ConnTested(r.map_err(|e| e.to_string()))
                });
            }
            Ok(input) => {
                form.saving = true;
                form.error = None;
                let editing = form.editing;
                app.worker.run(async move {
                    let r = match editing {
                        Some(id) => client.update_connection(id, &input).await,
                        None => client.create_connection(&input).await,
                    };
                    Msg::ConnSaved(r.map_err(|e| e.to_string()))
                });
            }
        }
    }
    let kind = form.pick.take();
    if close {
        app.dialog = Dialog::None;
        return;
    }
    let Some(kind) = kind else {
        return;
    };
    app.worker.run(async move {
        let picked = rfd::AsyncFileDialog::new()
            .add_filter("PEM", &["pem", "crt", "cer", "key"])
            .add_filter("All files", &["*"])
            .pick_file()
            .await;
        let result = match picked {
            None => None,
            Some(file) => {
                let name = file.file_name();
                let path = file.path().to_path_buf();
                Some(match std::fs::metadata(&path).map(|m| m.len()) {
                    Ok(len) if len > 64 * 1024 => {
                        Err("that file is larger than 64 KiB".to_string())
                    }
                    _ => std::fs::read_to_string(&path)
                        .map(|pem| (name, pem))
                        .map_err(|e| e.to_string()),
                })
            }
        };
        Msg::ConnCertPicked { kind, result }
    });
}

// ---------------------------------------------------------- confirmations --

fn confirm_delete(ctx: &egui::Context, app: &mut SqailApp) {
    let Dialog::ConfirmDelete(id, name) = &app.dialog else {
        return;
    };
    let (id, name) = (*id, name.clone());
    let mut decision = None;
    let resp = Modal::new(Id::new("confirm_delete")).show(ctx, |ui| {
        ui.heading("Delete connection?");
        ui.label(format!(
            "“{name}” will be removed from the service for everyone using it."
        ));
        ui.horizontal(|ui| {
            if ui
                .button(RichText::new("Delete").color(Color32::from_rgb(0xd6, 0x45, 0x45)))
                .clicked()
            {
                decision = Some(true);
            }
            if ui.button("Cancel").clicked() {
                decision = Some(false);
            }
        });
    });
    if resp.should_close() {
        decision = Some(false);
    }
    match decision {
        Some(true) => {
            app.dialog = Dialog::None;
            if let Some(client) = app.service.client.clone() {
                app.worker.run(async move {
                    Msg::ConnDeleted(
                        client
                            .delete_connection(id)
                            .await
                            .map(|_| id)
                            .map_err(|e| e.to_string()),
                    )
                });
            }
        }
        Some(false) => app.dialog = Dialog::None,
        None => {}
    }
}

fn confirm_close(ctx: &egui::Context, app: &mut SqailApp) {
    let Dialog::ConfirmClose(idx) = app.dialog else {
        return;
    };
    let title = app
        .tabs
        .get(idx)
        .map(|t| t.title.clone())
        .unwrap_or_default();
    let mut decision = None;
    let resp = Modal::new(Id::new("confirm_close")).show(ctx, |ui| {
        ui.heading("Unsaved changes");
        ui.label(format!("“{title}” has unsaved changes."));
        ui.horizontal(|ui| {
            if ui.button("Save").clicked() {
                decision = Some(0);
            }
            if ui.button("Discard").clicked() {
                decision = Some(1);
            }
            if ui.button("Cancel").clicked() {
                decision = Some(2);
            }
        });
    });
    if resp.should_close() {
        decision = Some(2);
    }
    match decision {
        Some(0) => {
            app.dialog = Dialog::None;
            app.save_tab(idx, false);
        }
        Some(1) => {
            app.dialog = Dialog::None;
            app.close_tab(idx);
        }
        Some(_) => app.dialog = Dialog::None,
        None => {}
    }
}

fn save_snippet(ctx: &egui::Context, app: &mut SqailApp) {
    let Dialog::SaveSnippet { name, sql } = &mut app.dialog else {
        return;
    };
    let mut decision = None;
    let resp = Modal::new(Id::new("save_snippet")).show(ctx, |ui| {
        ui.set_width(460.0);
        ui.heading("Save snippet");
        let l = ui.label("Snippet name");
        let field = ui
            .add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY))
            .labelled_by(l.id);
        if name.is_empty() && !field.has_focus() {
            field.request_focus();
        }
        egui::ScrollArea::vertical()
            .max_height(200.0)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(sql)
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .desired_rows(6),
                );
            });
        ui.horizontal(|ui| {
            let ok = !name.trim().is_empty() && !sql.trim().is_empty();
            if ui
                .add_enabled(ok, egui::Button::new("Save snippet"))
                .clicked()
                || (ok && field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
            {
                decision = Some(true);
            }
            if ui.button("Cancel").clicked() {
                decision = Some(false);
            }
        });
    });
    if resp.should_close() {
        decision = Some(false);
    }
    match decision {
        Some(true) => {
            let (n, s) = (name.trim().to_string(), sql.clone());
            app.snippets.add(n.clone(), s);
            app.dialog = Dialog::None;
            app.notify(format!("Saved snippet “{n}”."), false);
        }
        Some(false) => app.dialog = Dialog::None,
        None => {}
    }
}

// ----------------------------------------------------------------- import --

pub struct ImportForm {
    conn: Uuid,
    engine: sqail_client::proto::Engine,
    schema: String,
    table: String,
    path: std::path::PathBuf,
    preview: crate::transfer::CsvPreview,
    has_header: bool,
    empty_is_null: bool,
    columns: Option<Result<Vec<sqail_client::proto::ColumnInfo>, String>>,
    mapping: Vec<Option<usize>>,
    pub busy: bool,
    pub progress: u64,
    pub result: Option<Result<u64, String>>,
}

impl ImportForm {
    pub fn new(
        conn: Uuid,
        engine: sqail_client::proto::Engine,
        schema: String,
        table: String,
        path: std::path::PathBuf,
        preview: crate::transfer::CsvPreview,
    ) -> Self {
        Self {
            conn,
            engine,
            schema,
            table,
            path,
            preview,
            has_header: true,
            empty_is_null: true,
            columns: None,
            mapping: Vec::new(),
            busy: false,
            progress: 0,
            result: None,
        }
    }

    pub fn set_columns(&mut self, r: Result<Vec<sqail_client::proto::ColumnInfo>, String>) {
        if let Ok(cols) = &r {
            self.mapping = crate::transfer::auto_map(&self.preview.headers, cols);
        }
        self.columns = Some(r);
    }
}

fn import(ctx: &egui::Context, app: &mut SqailApp) {
    let Dialog::Import(form) = &mut app.dialog else {
        return;
    };
    let mut close = false;
    let mut start = false;
    let mut reheader = false;
    let resp = Modal::new(Id::new("import")).show(ctx, |ui| {
        ui.set_width(640.0);
        ui.heading(format!("Import CSV into {}.{}", form.schema, form.table));
        ui.label(
            RichText::new(form.path.display().to_string())
                .monospace()
                .weak(),
        );
        ui.horizontal(|ui| {
            reheader = ui
                .checkbox(&mut form.has_header, "First row is a header")
                .changed();
            ui.checkbox(&mut form.empty_is_null, "Empty fields are NULL");
            let d = match form.preview.delimiter {
                b'\t' => "tab".to_string(),
                b => (b as char).to_string(),
            };
            ui.label(RichText::new(format!("delimiter: {d}")).weak());
        });
        ui.separator();
        match &form.columns {
            None => {
                ui.spinner();
            }
            Some(Err(e)) => {
                ui.colored_label(Color32::from_rgb(0xd6, 0x45, 0x45), e);
            }
            Some(Ok(cols)) => {
                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .show(ui, |ui| {
                        egui::Grid::new("import_map")
                            .num_columns(4)
                            .striped(true)
                            .show(ui, |ui| {
                                ui.strong("Column");
                                ui.strong("Type");
                                ui.strong("From CSV");
                                ui.strong("First value");
                                ui.end_row();
                                for (i, c) in cols.iter().enumerate() {
                                    ui.label(&c.name);
                                    ui.label(RichText::new(&c.data_type).weak());
                                    let current = form.mapping[i]
                                        .and_then(|m| form.preview.headers.get(m))
                                        .cloned()
                                        .unwrap_or_else(|| "(skip)".into());
                                    egui::ComboBox::from_id_salt(("map", i))
                                        .selected_text(current)
                                        .width(180.0)
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(
                                                &mut form.mapping[i],
                                                None,
                                                "(skip)",
                                            );
                                            for (h_i, h) in form.preview.headers.iter().enumerate()
                                            {
                                                ui.selectable_value(
                                                    &mut form.mapping[i],
                                                    Some(h_i),
                                                    h,
                                                );
                                            }
                                        });
                                    let sample = form.mapping[i]
                                        .and_then(|m| {
                                            form.preview.rows.first().and_then(|r| r.get(m))
                                        })
                                        .cloned()
                                        .unwrap_or_default();
                                    ui.label(RichText::new(sample).monospace());
                                    ui.end_row();
                                }
                            });
                    });
            }
        }
        ui.separator();
        if form.busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!("Imported {} rows…", form.progress));
            });
        }
        match &form.result {
            Some(Ok(n)) => {
                ui.colored_label(
                    Color32::from_rgb(0x3c, 0xb3, 0x71),
                    format!("Imported {n} rows."),
                );
            }
            Some(Err(e)) => {
                ui.colored_label(
                    Color32::from_rgb(0xd6, 0x45, 0x45),
                    format!("Nothing imported (rolled back): {e}"),
                );
            }
            None => {}
        }
        ui.horizontal(|ui| {
            let ready = !form.busy
                && form.result.as_ref().is_none_or(|r| r.is_err())
                && matches!(form.columns, Some(Ok(_)))
                && form.mapping.iter().any(Option::is_some);
            if ui
                .add_enabled(ready, egui::Button::new(RichText::new("Import").strong()))
                .clicked()
            {
                start = true;
            }
            if ui
                .add_enabled(!form.busy, egui::Button::new("Close"))
                .clicked()
            {
                close = true;
            }
        });
    });
    if resp.should_close() && !form.busy {
        close = true;
    }
    if reheader && let Ok(p) = crate::transfer::preview_csv(&form.path, form.has_header) {
        form.preview = p;
        if let Some(Ok(cols)) = &form.columns {
            form.mapping = crate::transfer::auto_map(&form.preview.headers, cols);
        }
    }
    if start
        && let (Some(client), Some(Ok(cols))) = (app.service.client.clone(), form.columns.as_ref())
    {
        form.busy = true;
        form.progress = 0;
        form.result = None;
        let spec = crate::transfer::ImportSpec {
            connection: form.conn,
            engine: form.engine,
            schema: Some(form.schema.clone()),
            table: form.table.clone(),
            columns: cols.clone(),
            mapping: form.mapping.clone(),
            has_header: form.has_header,
            delimiter: form.preview.delimiter,
            empty_is_null: form.empty_is_null,
        };
        let path = form.path.clone();
        app.worker.spawn(move |sink| async move {
            let res = crate::transfer::import_csv(&client, &path, &spec, |n| {
                sink.send(Msg::ImportProgress(n))
            })
            .await;
            sink.send(Msg::ImportDone(res.map_err(|e| format!("{e:#}"))));
        });
    }
    if close {
        app.dialog = Dialog::None;
    }
}

// ------------------------------------------------------------ apply edits --

pub struct ApplyEdits {
    pub tab: u64,
    pub connection: Uuid,
    pub engine: sqail_client::proto::Engine,
    pub statements: Vec<(String, Vec<sqail_client::proto::Param>)>,
    pub previews: Vec<String>,
    pub busy: bool,
    pub error: Option<String>,
}

fn apply_edits(ctx: &egui::Context, app: &mut SqailApp) {
    let Dialog::ApplyEdits(d) = &mut app.dialog else {
        return;
    };
    let mut apply = false;
    let mut close = false;
    let resp = Modal::new(Id::new("apply_edits")).show(ctx, |ui| {
        ui.set_width(700.0);
        ui.heading(format!("Apply {} change(s)", d.statements.len()));
        ui.label(
            RichText::new(
                "These statements run in one transaction; if any fails, nothing is changed.",
            )
            .weak(),
        );
        egui::ScrollArea::vertical()
            .max_height(320.0)
            .show(ui, |ui| {
                let mut text = d
                    .previews
                    .iter()
                    .map(|p| format!("{p};"))
                    .collect::<Vec<_>>()
                    .join("\n");
                ui.add(
                    egui::TextEdit::multiline(&mut text)
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .interactive(true),
                );
            });
        if let Some(e) = &d.error {
            ui.colored_label(
                Color32::from_rgb(0xd6, 0x45, 0x45),
                format!("Rolled back: {e}"),
            );
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!d.busy, egui::Button::new(RichText::new("Apply").strong()))
                .clicked()
            {
                apply = true;
            }
            if d.busy {
                ui.spinner();
            }
            if ui
                .add_enabled(!d.busy, egui::Button::new("Cancel"))
                .clicked()
            {
                close = true;
            }
        });
    });
    if resp.should_close() && !d.busy {
        close = true;
    }
    if apply && let Some(client) = app.service.client.clone() {
        d.busy = true;
        d.error = None;
        let (tab, conn, engine, stmts) = (d.tab, d.connection, d.engine, d.statements.clone());
        app.worker.run(async move {
            let result = crate::editing::apply(&client, conn, engine, stmts)
                .await
                .map_err(|e| format!("{e:#}"));
            Msg::EditsApplied { tab, result }
        });
    }
    if close {
        app.dialog = Dialog::None;
    }
}

// ------------------------------------------------------------- drop table --

pub struct DropTable {
    pub connection: Uuid,
    pub engine: sqail_client::proto::Engine,
    pub schema: String,
    pub table: String,
    /// Postgres: also drop views and foreign keys that depend on the table.
    pub cascade: bool,
    pub busy: bool,
    pub error: Option<String>,
}

impl DropTable {
    pub fn new(
        connection: Uuid,
        engine: sqail_client::proto::Engine,
        schema: String,
        table: String,
    ) -> Self {
        Self {
            connection,
            engine,
            schema,
            table,
            cascade: false,
            busy: false,
            error: None,
        }
    }

    fn sql(&self) -> String {
        crate::designer::model::drop_table(self.engine, &self.schema, &self.table, self.cascade)
    }
}

fn drop_table(ctx: &egui::Context, app: &mut SqailApp) {
    let Dialog::DropTable(d) = &mut app.dialog else {
        return;
    };
    let mut run = false;
    let mut close = false;
    let resp = Modal::new(Id::new("drop_table")).show(ctx, |ui| {
        ui.set_width(480.0);
        ui.heading(format!("Drop table {}?", d.table));
        ui.label("The table and all its rows are deleted. This cannot be undone.");
        if d.engine == sqail_client::proto::Engine::Postgres {
            ui.checkbox(
                &mut d.cascade,
                "Also drop the views and foreign keys that depend on it (CASCADE)",
            );
        }
        ui.add_space(4.0);
        ui.label(RichText::new(format!("{};", d.sql())).monospace());
        if let Some(e) = &d.error {
            ui.colored_label(Color32::from_rgb(0xd6, 0x45, 0x45), e);
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !d.busy,
                    egui::Button::new(
                        RichText::new("Drop")
                            .strong()
                            .color(Color32::from_rgb(0xd6, 0x45, 0x45)),
                    ),
                )
                .clicked()
            {
                run = true;
            }
            if d.busy {
                ui.spinner();
            }
            if ui
                .add_enabled(!d.busy, egui::Button::new("Cancel"))
                .clicked()
            {
                close = true;
            }
        });
    });
    if resp.should_close() && !d.busy {
        close = true;
    }
    if run && let Some(client) = app.service.client.clone() {
        d.busy = true;
        d.error = None;
        let plan = crate::designer::model::Plan {
            body: vec![d.sql()],
            ..Default::default()
        };
        let (conn, engine, schema, table) =
            (d.connection, d.engine, d.schema.clone(), d.table.clone());
        app.worker.run(async move {
            let result = crate::designer::apply(&client, conn, engine, plan)
                .await
                .map_err(|e| format!("{e:#}"));
            Msg::TableDropped {
                conn,
                schema,
                table,
                result,
            }
        });
    }
    if close {
        app.dialog = Dialog::None;
    }
}

fn confirm_close_transaction(ctx: &egui::Context, app: &mut SqailApp, target: Option<usize>) {
    let mut decision = None;
    let resp = Modal::new(Id::new("confirm_close_tx")).show(ctx, |ui| {
        ui.set_width(440.0);
        ui.heading("Uncommitted transaction");
        let what = match target {
            Some(i) => format!(
                "“{}” has an open transaction.",
                app.tabs.get(i).map_or("", |t| t.title.as_str())
            ),
            None => "Some tabs have open transactions.".to_string(),
        };
        ui.label(what);
        ui.label(
            RichText::new("Closing rolls the changes back. Commit first to keep them.").weak(),
        );
        ui.horizontal(|ui| {
            let label = if target.is_some() {
                "Roll back and close"
            } else {
                "Roll back and quit"
            };
            if ui
                .button(RichText::new(label).color(Color32::from_rgb(0xd6, 0x45, 0x45)))
                .clicked()
            {
                decision = Some(true);
            }
            if ui.button("Cancel").clicked() {
                decision = Some(false);
            }
        });
    });
    if resp.should_close() {
        decision = Some(false);
    }
    match decision {
        Some(true) => {
            app.dialog = Dialog::None;
            match target {
                Some(i) => {
                    if let Some(t) = app.tabs.get_mut(i) {
                        t.in_transaction = false;
                    }
                    app.request_close_tab(i);
                }
                None => {
                    app.allow_close = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        Some(false) => app.dialog = Dialog::None,
        None => {}
    }
}

pub fn show(ctx: &egui::Context, app: &mut SqailApp) {
    match app.dialog {
        Dialog::None => {}
        Dialog::Welcome(_) => welcome(ctx, app),
        Dialog::Connection(_) => connection_form(ctx, app),
        Dialog::ConfirmDelete(..) => confirm_delete(ctx, app),
        Dialog::ConfirmClose(_) => confirm_close(ctx, app),
        Dialog::SaveSnippet { .. } => save_snippet(ctx, app),
        Dialog::Import(_) => import(ctx, app),
        Dialog::ApplyEdits(_) => apply_edits(ctx, app),
        Dialog::DropTable(_) => drop_table(ctx, app),
        Dialog::ConfirmCloseTransaction(target) => confirm_close_transaction(ctx, app, target),
    }
}
