//! The sidebar: connection profiles and a lazily-loaded schema tree.

use std::collections::HashMap;

use egui::{CollapsingHeader, Color32, RichText};
use sqail_client::Client;
use sqail_client::proto::{
    ColumnInfo, Connection, ForeignKeyInfo, IndexInfo, NamedItem, RoutineInfo, TableInfo, TableKind,
};
use uuid::Uuid;

use crate::app::{Msg, ServiceStatus, SqailApp};
use crate::dialogs::{ConnForm, Dialog};
use crate::sql;
use crate::theme;

pub enum Load<T> {
    Loading,
    Ready(T),
    Failed(String),
}

/// What a schema request fetched.
pub enum SchemaData {
    Schemas(Vec<NamedItem>),
    Tables(Vec<TableInfo>),
    Routines(Vec<RoutineInfo>),
    Columns(Vec<ColumnInfo>),
    Indexes(Vec<IndexInfo>),
    ForeignKeys(Vec<ForeignKeyInfo>),
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum SchemaKey {
    Schemas,
    Tables(String),
    Routines(String),
    Columns(String, String),
    Indexes(String, String),
    ForeignKeys(String, String),
}

pub struct SchemaMsg {
    pub connection: Uuid,
    pub key: SchemaKey,
    pub result: Result<SchemaData, String>,
}

#[derive(Default)]
pub struct SchemaTree {
    data: HashMap<Uuid, HashMap<SchemaKey, Load<SchemaData>>>,
    pub filter: String,
}

impl SchemaTree {
    pub fn apply(&mut self, m: SchemaMsg) {
        let entry = self.data.entry(m.connection).or_default();
        entry.insert(
            m.key,
            match m.result {
                Ok(d) => Load::Ready(d),
                Err(e) => Load::Failed(e),
            },
        );
    }

    pub fn invalidate(&mut self, conn: Uuid) {
        self.data.remove(&conn);
    }

    pub fn clear(&mut self) {
        self.data.clear();
    }

    pub fn retain(&mut self, conns: &[Connection]) {
        self.data.retain(|id, _| conns.iter().any(|c| c.id == *id));
    }

    /// Names of the schemas already loaded for `conn`.
    pub fn schema_names(&self, conn: Uuid) -> Vec<String> {
        match self.ready(conn, &SchemaKey::Schemas) {
            Some(SchemaData::Schemas(s)) => s.iter().map(|n| n.name.clone()).collect(),
            _ => Vec::new(),
        }
    }

    /// Every table/view loaded for `conn`: (schema, table).
    pub fn loaded_tables(&self, conn: Uuid) -> Vec<(String, TableInfo)> {
        let Some(map) = self.data.get(&conn) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (k, v) in map {
            if let (SchemaKey::Tables(s), Load::Ready(SchemaData::Tables(ts))) = (k, v) {
                out.extend(ts.iter().map(|t| (s.clone(), t.clone())));
            }
        }
        out.sort_by(|a, b| (&a.0, &a.1.name).cmp(&(&b.0, &b.1.name)));
        out
    }

    /// Start loading `key` unless it is cached or in flight.
    pub fn ensure(
        &mut self,
        client: &Option<Client>,
        worker: &crate::worker::Worker,
        conn: Uuid,
        key: SchemaKey,
    ) {
        let _ = self.get(client, worker, conn, key);
    }

    fn ready(&self, conn: Uuid, key: &SchemaKey) -> Option<&SchemaData> {
        match self.data.get(&conn)?.get(key)? {
            Load::Ready(d) => Some(d),
            _ => None,
        }
    }

    /// The cached value, or start loading it.
    fn get(
        &mut self,
        app_client: &Option<Client>,
        worker: &crate::worker::Worker,
        conn: Uuid,
        key: SchemaKey,
    ) -> Option<&Load<SchemaData>> {
        let entry = self.data.entry(conn).or_default();
        if !entry.contains_key(&key) {
            let client = app_client.clone()?;
            entry.insert(key.clone(), Load::Loading);
            let k = key.clone();
            worker.run(async move {
                let s = |x: &String| x.clone();
                let result = match &k {
                    SchemaKey::Schemas => client.schemas(conn).await.map(SchemaData::Schemas),
                    SchemaKey::Tables(sc) => client
                        .tables(conn, Some(&s(sc)))
                        .await
                        .map(SchemaData::Tables),
                    SchemaKey::Routines(sc) => client
                        .routines(conn, Some(&s(sc)))
                        .await
                        .map(SchemaData::Routines),
                    SchemaKey::Columns(sc, t) => client
                        .columns(conn, Some(sc), t)
                        .await
                        .map(SchemaData::Columns),
                    SchemaKey::Indexes(sc, t) => client
                        .indexes(conn, Some(sc), t)
                        .await
                        .map(SchemaData::Indexes),
                    SchemaKey::ForeignKeys(sc, t) => client
                        .foreign_keys(conn, Some(sc), t)
                        .await
                        .map(SchemaData::ForeignKeys),
                };
                Msg::Schema(SchemaMsg {
                    connection: conn,
                    key: k,
                    result: result.map_err(|e| e.to_string()),
                })
            });
        }
        self.data.get(&conn).and_then(|m| m.get(&key))
    }
}

/// Requests from the tree to the app (applied after drawing).
enum Action {
    Import {
        conn: Uuid,
        schema: String,
        table: String,
    },
    NewTab(Uuid),
    Edit(Uuid),
    Delete(Uuid, String),
    Test(Uuid),
    Refresh(Uuid),
    SelectTop {
        conn: Uuid,
        schema: String,
        table: String,
    },
    Ddl {
        conn: Uuid,
        schema: String,
        name: String,
    },
    Copy(String),
}

pub fn sidebar_ui(ui: &mut egui::Ui, app: &mut SqailApp) {
    let mut actions = Vec::new();
    ui.horizontal(|ui| {
        ui.heading("Connections");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let connected = app.service.status == ServiceStatus::Connected;
            if ui
                .add_enabled(connected, egui::Button::new("+ Add"))
                .on_hover_text("New connection")
                .clicked()
            {
                app.dialog = Dialog::Connection(Box::new(ConnForm::new_default()));
            }
            if ui
                .add_enabled(connected, egui::Button::new("⟳"))
                .on_hover_text("Refresh")
                .clicked()
            {
                app.refresh_connections();
                app.schema.clear();
            }
        });
    });
    ui.add(
        egui::TextEdit::singleline(&mut app.schema.filter)
            .hint_text("Filter tables…")
            .desired_width(f32::INFINITY),
    );
    ui.separator();

    if app.service.status != ServiceStatus::Connected {
        ui.label(RichText::new("Not connected to a service.").weak());
        if ui.button("Connect to a service…").clicked() {
            app.dialog = Dialog::Welcome(Default::default());
        }
        return;
    }
    if app.service.connections.is_empty() {
        ui.label(RichText::new("No connections yet.").weak());
        if ui.button("Add a connection…").clicked() {
            app.dialog = Dialog::Connection(Box::new(ConnForm::new_default()));
        }
        return;
    }

    let dark = ui.visuals().dark_mode;
    let connections = app.service.connections.clone();
    let client = app.service.client.clone();
    let filter = app.schema.filter.to_lowercase();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut folders: Vec<Option<String>> =
                connections.iter().map(|c| c.folder.clone()).collect();
            folders.sort();
            folders.dedup();
            for folder in folders {
                let members: Vec<&Connection> =
                    connections.iter().filter(|c| c.folder == folder).collect();
                match &folder {
                    Some(name) => {
                        CollapsingHeader::new(RichText::new(name).strong())
                            .id_salt(("folder", name))
                            .default_open(true)
                            .show(ui, |ui| {
                                for c in &members {
                                    connection_node(
                                        ui,
                                        app_ref(app),
                                        &client,
                                        c,
                                        &filter,
                                        dark,
                                        &mut actions,
                                    );
                                }
                            });
                    }
                    None => {
                        for c in &members {
                            connection_node(
                                ui,
                                app_ref(app),
                                &client,
                                c,
                                &filter,
                                dark,
                                &mut actions,
                            );
                        }
                    }
                }
            }
        });

    for a in actions {
        apply(app, a, ui.ctx());
    }
}

/// The parts of the app the tree needs while drawing.
struct TreeCtx<'a> {
    schema: &'a mut SchemaTree,
    worker: &'a crate::worker::Worker,
}

fn app_ref(app: &mut SqailApp) -> TreeCtx<'_> {
    TreeCtx {
        schema: &mut app.schema,
        worker: &app.worker,
    }
}

fn connection_node(
    ui: &mut egui::Ui,
    mut t: TreeCtx<'_>,
    client: &Option<Client>,
    c: &Connection,
    filter: &str,
    dark: bool,
    actions: &mut Vec<Action>,
) {
    let color = c
        .color
        .as_deref()
        .and_then(theme::parse_hex)
        .unwrap_or(theme::accent(dark));
    let mut title = RichText::new(c.name.as_str()).strong();
    if c.read_only {
        title = title.italics();
    }
    let engine = match c.engine {
        sqail_client::proto::Engine::Postgres => "pg",
        sqail_client::proto::Engine::Mssql => "mssql",
        sqail_client::proto::Engine::Sqlite => "sqlite",
    };
    let header = CollapsingHeader::new(title)
        .id_salt(("conn", c.id))
        .icon(move |ui, openness, response| {
            let center = response.rect.center();
            ui.painter().circle_filled(center, 4.0 + openness, color);
        })
        .show(ui, |ui| {
            let id = c.id;
            let loaded = t.schema.get(client, t.worker, id, SchemaKey::Schemas);
            let schemas: Vec<String> = match loaded {
                Some(Load::Ready(SchemaData::Schemas(s))) => {
                    s.iter().map(|n| n.name.clone()).collect()
                }
                Some(Load::Failed(e)) => {
                    ui.colored_label(Color32::from_rgb(0xd6, 0x45, 0x45), e.clone());
                    return;
                }
                _ => {
                    ui.spinner();
                    return;
                }
            };
            let single = schemas.len() == 1;
            for schema in schemas {
                if single {
                    schema_children(ui, &mut t, client, id, c.engine, &schema, filter, actions);
                } else {
                    CollapsingHeader::new(schema.clone())
                        .id_salt(("schema", id, &schema))
                        .default_open(!filter.is_empty())
                        .show(ui, |ui| {
                            schema_children(
                                ui, &mut t, client, id, c.engine, &schema, filter, actions,
                            )
                        });
                }
            }
        });
    header.header_response.clone().on_hover_text(format!(
        "{engine}{}{}",
        c.environment
            .as_deref()
            .map(|e| format!(" · {e}"))
            .unwrap_or_default(),
        if c.read_only { " · read-only" } else { "" }
    ));
    header.header_response.context_menu(|ui| {
        if ui.button("New query tab").clicked() {
            actions.push(Action::NewTab(c.id));
            ui.close();
        }
        if ui.button("Edit…").clicked() {
            actions.push(Action::Edit(c.id));
            ui.close();
        }
        if ui.button("Test").clicked() {
            actions.push(Action::Test(c.id));
            ui.close();
        }
        if ui.button("Refresh schema").clicked() {
            actions.push(Action::Refresh(c.id));
            ui.close();
        }
        ui.separator();
        if ui.button("Delete…").clicked() {
            actions.push(Action::Delete(c.id, c.name.clone()));
            ui.close();
        }
    });
    if header.header_response.double_clicked() {
        actions.push(Action::NewTab(c.id));
    }
}

#[allow(clippy::too_many_arguments)]
fn schema_children(
    ui: &mut egui::Ui,
    t: &mut TreeCtx<'_>,
    client: &Option<Client>,
    conn: Uuid,
    engine: sqail_client::proto::Engine,
    schema: &str,
    filter: &str,
    actions: &mut Vec<Action>,
) {
    let tables = match t
        .schema
        .get(client, t.worker, conn, SchemaKey::Tables(schema.into()))
    {
        Some(Load::Ready(SchemaData::Tables(v))) => v.clone(),
        Some(Load::Failed(e)) => {
            ui.colored_label(Color32::from_rgb(0xd6, 0x45, 0x45), e.clone());
            return;
        }
        _ => {
            ui.spinner();
            return;
        }
    };
    let matches = |name: &str| filter.is_empty() || name.to_lowercase().contains(filter);
    for (kind, label) in [(TableKind::Table, "Tables"), (TableKind::View, "Views")] {
        let items: Vec<&TableInfo> = tables
            .iter()
            .filter(|x| {
                (x.kind == kind
                    || (kind == TableKind::View && x.kind == TableKind::MaterializedView))
                    && matches(&x.name)
            })
            .collect();
        if items.is_empty() && kind == TableKind::View {
            continue;
        }
        CollapsingHeader::new(format!("{label} ({})", items.len()))
            .id_salt((label, conn, schema))
            .default_open(kind == TableKind::Table || !filter.is_empty())
            .show(ui, |ui| {
                for table in items {
                    table_node(ui, t, client, conn, engine, schema, table, actions);
                }
            });
    }
    if engine != sqail_client::proto::Engine::Sqlite {
        CollapsingHeader::new("Routines")
            .id_salt(("routines", conn, schema))
            .show(ui, |ui| {
                match t
                    .schema
                    .get(client, t.worker, conn, SchemaKey::Routines(schema.into()))
                {
                    Some(Load::Ready(SchemaData::Routines(rs))) => {
                        if rs.is_empty() {
                            ui.label(RichText::new("none").weak());
                        }
                        for r in rs.iter().filter(|r| matches(&r.name)) {
                            let icon = if r.kind == sqail_client::proto::RoutineKind::Procedure {
                                "⚙"
                            } else {
                                "ƒ"
                            };
                            let resp = ui.selectable_label(false, format!("{icon} {}", r.name));
                            let name = r.name.clone();
                            resp.context_menu(|ui| {
                                if ui.button("Script definition").clicked() {
                                    actions.push(Action::Ddl {
                                        conn,
                                        schema: schema.into(),
                                        name: name.clone(),
                                    });
                                    ui.close();
                                }
                                if ui.button("Copy name").clicked() {
                                    actions.push(Action::Copy(name.clone()));
                                    ui.close();
                                }
                            });
                        }
                    }
                    Some(Load::Failed(e)) => {
                        ui.colored_label(Color32::from_rgb(0xd6, 0x45, 0x45), e.clone());
                    }
                    _ => {
                        ui.spinner();
                    }
                }
            });
    }
}

#[allow(clippy::too_many_arguments)]
fn table_node(
    ui: &mut egui::Ui,
    t: &mut TreeCtx<'_>,
    client: &Option<Client>,
    conn: Uuid,
    engine: sqail_client::proto::Engine,
    schema: &str,
    table: &TableInfo,
    actions: &mut Vec<Action>,
) {
    let icon = if table.kind == TableKind::Table {
        "⊞"
    } else {
        "👁"
    };
    let resp = CollapsingHeader::new(format!("{icon} {}", table.name))
        .id_salt(("table", conn, schema, &table.name))
        .show(ui, |ui| {
            let (sc, name) = (schema.to_string(), table.name.clone());
            match t.schema.get(
                client,
                t.worker,
                conn,
                SchemaKey::Columns(sc.clone(), name.clone()),
            ) {
                Some(Load::Ready(SchemaData::Columns(cols))) => {
                    for col in cols {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            ui.label(&col.name);
                            if col.primary_key {
                                ui.label(
                                    RichText::new("PK")
                                        .small()
                                        .strong()
                                        .color(theme::accent(ui.visuals().dark_mode)),
                                );
                            }
                            let mut ty = col.data_type.clone();
                            if !col.nullable {
                                ty.push_str(" not null");
                            }
                            ui.label(RichText::new(ty).weak().small());
                        });
                    }
                }
                Some(Load::Failed(e)) => {
                    ui.colored_label(Color32::from_rgb(0xd6, 0x45, 0x45), e.clone());
                }
                _ => {
                    ui.spinner();
                }
            }
            if table.kind == TableKind::Table {
                CollapsingHeader::new("Indexes")
                    .id_salt(("idx", conn, &sc, &name))
                    .show(ui, |ui| {
                        match t.schema.get(
                            client,
                            t.worker,
                            conn,
                            SchemaKey::Indexes(sc.clone(), name.clone()),
                        ) {
                            Some(Load::Ready(SchemaData::Indexes(ix))) => {
                                if ix.is_empty() {
                                    ui.label(RichText::new("none").weak());
                                }
                                for i in ix {
                                    let kind = if i.primary {
                                        "primary"
                                    } else if i.unique {
                                        "unique"
                                    } else {
                                        ""
                                    };
                                    ui.label(format!(
                                        "{} ({}) {kind}",
                                        i.name,
                                        i.columns.join(", ")
                                    ));
                                }
                            }
                            Some(Load::Failed(e)) => {
                                ui.colored_label(Color32::from_rgb(0xd6, 0x45, 0x45), e.clone());
                            }
                            _ => {
                                ui.spinner();
                            }
                        }
                    });
                CollapsingHeader::new("Foreign keys")
                    .id_salt(("fk", conn, &sc, &name))
                    .show(ui, |ui| {
                        match t.schema.get(
                            client,
                            t.worker,
                            conn,
                            SchemaKey::ForeignKeys(sc.clone(), name.clone()),
                        ) {
                            Some(Load::Ready(SchemaData::ForeignKeys(fks))) => {
                                if fks.is_empty() {
                                    ui.label(RichText::new("none").weak());
                                }
                                for f in fks {
                                    ui.label(format!(
                                        "({}) → {} ({})",
                                        f.columns.join(", "),
                                        f.ref_table,
                                        f.ref_columns.join(", ")
                                    ))
                                    .on_hover_text(&f.name);
                                }
                            }
                            Some(Load::Failed(e)) => {
                                ui.colored_label(Color32::from_rgb(0xd6, 0x45, 0x45), e.clone());
                            }
                            _ => {
                                ui.spinner();
                            }
                        }
                    });
            }
        });
    let name = table.name.clone();
    resp.header_response.context_menu(|ui| {
        if ui.button("SELECT top 100").clicked() {
            actions.push(Action::SelectTop {
                conn,
                schema: schema.into(),
                table: name.clone(),
            });
            ui.close();
        }
        if table.kind == TableKind::Table && ui.button("Import CSV…").clicked() {
            actions.push(Action::Import {
                conn,
                schema: schema.into(),
                table: name.clone(),
            });
            ui.close();
        }
        if ui.button("Script CREATE").clicked() {
            actions.push(Action::Ddl {
                conn,
                schema: schema.into(),
                name: name.clone(),
            });
            ui.close();
        }
        if ui.button("Copy name").clicked() {
            let qualified = format!(
                "{}.{}",
                sql::quote_ident(engine, schema),
                sql::quote_ident(engine, &name)
            );
            actions.push(Action::Copy(qualified));
            ui.close();
        }
    });
    if resp.header_response.double_clicked() {
        actions.push(Action::SelectTop {
            conn,
            schema: schema.into(),
            table: name,
        });
    }
}

fn apply(app: &mut SqailApp, a: Action, ctx: &egui::Context) {
    match a {
        Action::NewTab(conn) => {
            app.new_tab(Some(conn));
        }
        Action::Edit(id) => {
            if let Some(c) = app.service.connection(id) {
                app.dialog = Dialog::Connection(Box::new(ConnForm::edit(c)));
            }
        }
        Action::Delete(id, name) => app.dialog = Dialog::ConfirmDelete(id, name),
        Action::Test(id) => {
            let Some(client) = app.service.client.clone() else {
                return;
            };
            let name = app
                .service
                .connection(id)
                .map(|c| c.name.clone())
                .unwrap_or_default();
            app.worker.run(async move {
                match client.test_connection(id).await {
                    Ok(r) if r.ok => Msg::Notice {
                        text: format!("{name}: OK in {} ms", r.latency_ms),
                        error: false,
                    },
                    Ok(r) => Msg::Notice {
                        text: format!("{name}: {}", r.error.unwrap_or_default()),
                        error: true,
                    },
                    Err(e) => Msg::Notice {
                        text: format!("{name}: {e}"),
                        error: true,
                    },
                }
            });
        }
        Action::Refresh(id) => app.schema.invalidate(id),
        Action::SelectTop {
            conn,
            schema,
            table,
        } => {
            let Some(engine) = app.engine_of(Some(conn)) else {
                return;
            };
            let text = sql::select_top(engine, Some(&schema), &table, 100);
            let idx = app.open_text_tab(conn, table, text.clone());
            app.run_sql(idx, text);
        }
        Action::Ddl { conn, schema, name } => {
            let Some(client) = app.service.client.clone() else {
                return;
            };
            app.worker.run(async move {
                match client.ddl(conn, Some(&schema), &name).await {
                    Ok(d) => Msg::OpenInTab {
                        connection: conn,
                        title: format!("{name}.sql"),
                        text: d.ddl,
                        run: false,
                    },
                    Err(e) => Msg::Notice {
                        text: format!("Script {name}: {e}"),
                        error: true,
                    },
                }
            });
        }
        Action::Copy(text) => ctx.copy_text(text),
        Action::Import {
            conn,
            schema,
            table,
        } => {
            app.worker.run(async move {
                let file = rfd::AsyncFileDialog::new()
                    .add_filter("CSV", &["csv", "tsv", "txt"])
                    .pick_file()
                    .await;
                match file {
                    Some(f) => Msg::ImportPicked {
                        conn,
                        schema,
                        table,
                        path: f.path().to_path_buf(),
                    },
                    None => Msg::Notice {
                        text: String::new(),
                        error: false,
                    },
                }
            });
        }
    }
}

/// The schema cache seen through the completer's [`Catalog`] trait. Lookups
/// that miss are recorded so the app can load them.
pub struct TreeCatalog<'a> {
    tree: &'a SchemaTree,
    conn: Uuid,
    pub missing: std::cell::RefCell<Vec<SchemaKey>>,
}

impl<'a> TreeCatalog<'a> {
    pub fn new(tree: &'a SchemaTree, conn: Uuid) -> Self {
        Self {
            tree,
            conn,
            missing: Default::default(),
        }
    }

    fn lookup(&self, key: SchemaKey) -> Option<&'a SchemaData> {
        let found = self.tree.ready(self.conn, &key);
        if found.is_none() {
            self.missing.borrow_mut().push(key);
        }
        found
    }

    /// Canonical spelling of a schema name (identifiers are case-insensitive
    /// unless quoted; the catalog has the real spelling).
    fn schema_name(&self, schema: &str) -> String {
        match self.tree.ready(self.conn, &SchemaKey::Schemas) {
            Some(SchemaData::Schemas(list)) => list
                .iter()
                .find(|s| s.name.eq_ignore_ascii_case(schema))
                .map(|s| s.name.clone())
                .unwrap_or_else(|| schema.to_string()),
            _ => schema.to_string(),
        }
    }

    fn table_name(&self, schema: &str, table: &str) -> String {
        match self
            .tree
            .ready(self.conn, &SchemaKey::Tables(schema.to_string()))
        {
            Some(SchemaData::Tables(list)) => list
                .iter()
                .find(|t| t.name.eq_ignore_ascii_case(table))
                .map(|t| t.name.clone())
                .unwrap_or_else(|| table.to_string()),
            _ => table.to_string(),
        }
    }
}

impl sql::complete::Catalog for TreeCatalog<'_> {
    fn schemas(&self) -> Option<Vec<String>> {
        match self.lookup(SchemaKey::Schemas)? {
            SchemaData::Schemas(s) => Some(s.iter().map(|n| n.name.clone()).collect()),
            _ => None,
        }
    }

    fn tables(&self, schema: &str) -> Option<Vec<TableInfo>> {
        match self.lookup(SchemaKey::Tables(self.schema_name(schema)))? {
            SchemaData::Tables(t) => Some(t.clone()),
            _ => None,
        }
    }

    fn columns(&self, schema: &str, table: &str) -> Option<Vec<ColumnInfo>> {
        let schema = self.schema_name(schema);
        let table = self.table_name(&schema, table);
        match self.lookup(SchemaKey::Columns(schema, table))? {
            SchemaData::Columns(c) => Some(c.clone()),
            _ => None,
        }
    }

    fn foreign_keys(&self, schema: &str, table: &str) -> Option<Vec<ForeignKeyInfo>> {
        let schema = self.schema_name(schema);
        let table = self.table_name(&schema, table);
        match self.lookup(SchemaKey::ForeignKeys(schema, table))? {
            SchemaData::ForeignKeys(f) => Some(f.clone()),
            _ => None,
        }
    }
}
