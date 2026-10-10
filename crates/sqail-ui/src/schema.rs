//! The sidebar: connection profiles and a lazily-loaded schema tree.

use std::collections::HashMap;

use egui::{CollapsingHeader, Color32, RichText};
use sqail_client::Client;
use sqail_client::proto::{
    ColumnInfo, Connection, ForeignKeyInfo, IndexInfo, NamedItem, RoutineInfo, TableInfo, TableKind,
};
use uuid::Uuid;

use crate::app::{Msg, ServiceStatus, SqailApp};
use crate::dialogs::{ConnForm, Dialog, DiscoverForm};
use crate::editor::SqlDrop;
use crate::folders::{ConnDrag, Rename, RenameTarget};
use crate::sql;
use crate::sql::drop::{DropItem, ObjectKind};
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
    /// A connection or folder being renamed in place.
    pub rename: Option<crate::folders::Rename>,
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

    /// Forget what is cached about one table (and its schema's table list).
    pub fn invalidate_table(&mut self, conn: Uuid, schema: &str, table: &str) {
        if let Some(m) = self.data.get_mut(&conn) {
            m.retain(|k, _| match k {
                SchemaKey::Tables(s) => s != schema,
                SchemaKey::Columns(s, t)
                | SchemaKey::Indexes(s, t)
                | SchemaKey::ForeignKeys(s, t) => !(s == schema && t == table),
                _ => true,
            });
        }
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
    /// The connection form, optionally for a new connection in a folder.
    NewConnection(Option<String>),
    NewFolder,
    DeleteFolder(String),
    StartRename(RenameTarget, String),
    CommitRename,
    CancelRename,
    Move {
        conn: Uuid,
        folder: Option<String>,
    },
    Edit(Uuid),
    Duplicate(Uuid),
    DiscoverAzure(Uuid),
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
    Design {
        conn: Uuid,
        schema: String,
        table: String,
    },
    NewTable {
        conn: Uuid,
        schema: Option<String>,
    },
    DropTable {
        conn: Uuid,
        engine: sqail_client::proto::Engine,
        schema: String,
        table: String,
    },
}

pub fn sidebar_ui(ui: &mut egui::Ui, app: &mut SqailApp) {
    let mut actions = Vec::new();
    let connected = app.service.status == ServiceStatus::Connected;
    ui.horizontal(|ui| {
        ui.heading("Connections");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(connected, egui::Button::new("⟳").small())
                .on_hover_text("Refresh")
                .clicked()
            {
                app.refresh_connections();
                app.schema.clear();
            }
        });
    });
    // The tree's toolbar.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if ui
            .add_enabled(connected, egui::Button::new("+ Connection").small())
            .on_hover_text("New connection")
            .clicked()
        {
            actions.push(Action::NewConnection(None));
        }
        if ui
            .add_enabled(connected, egui::Button::new("+ Folder").small())
            .on_hover_text("New folder: drag connections into it")
            .clicked()
        {
            actions.push(Action::NewFolder);
        }
    });
    ui.add(
        egui::TextEdit::singleline(&mut app.schema.filter)
            .hint_text("Filter tables…")
            .desired_width(f32::INFINITY),
    );
    ui.separator();

    if !connected {
        ui.label(RichText::new("Not connected to a service.").weak());
        if ui.button("Connect to a service…").clicked() {
            app.dialog = Dialog::Welcome(Default::default());
        }
        return;
    }
    let folders = crate::folders::all_folders(app);
    if app.service.connections.is_empty() && folders.is_empty() {
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
    // Dragging a connection that is in a folder: offer to take it out.
    let dragging = egui::DragAndDrop::payload::<ConnDrag>(ui.ctx())
        .and_then(|d| connections.iter().find(|c| c.id == d.0))
        .is_some_and(|c| c.folder.is_some());
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Connections without a folder first, then the folders.
            for c in connections.iter().filter(|c| c.folder.is_none()) {
                connection_node(
                    ui,
                    app_ref(app, &folders),
                    &client,
                    c,
                    &filter,
                    dark,
                    &mut actions,
                );
            }
            for name in &folders {
                let members: Vec<&Connection> = connections
                    .iter()
                    .filter(|c| c.folder.as_deref() == Some(name.as_str()))
                    .collect();
                folder_node(
                    ui,
                    app,
                    &folders,
                    name,
                    &members,
                    &client,
                    &filter,
                    dark,
                    &mut actions,
                );
            }
            if dragging {
                ui.add_space(6.0);
                let (_, dropped) = ui.dnd_drop_zone::<ConnDrag, ()>(
                    egui::Frame::group(ui.style()).inner_margin(6),
                    |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new("Drop here to take it out of its folder").weak());
                    },
                );
                if let Some(d) = dropped {
                    actions.push(Action::Move {
                        conn: d.0,
                        folder: None,
                    });
                }
            }
        });

    for a in actions {
        apply(app, a, ui.ctx());
    }
}

#[allow(clippy::too_many_arguments)]
fn folder_node(
    ui: &mut egui::Ui,
    app: &mut SqailApp,
    folders: &[String],
    name: &str,
    members: &[&Connection],
    client: &Option<Client>,
    filter: &str,
    dark: bool,
    actions: &mut Vec<Action>,
) {
    let target = RenameTarget::Folder(name.to_string());
    if let Some(r) = app.schema.rename.as_mut().filter(|r| r.target == target) {
        rename_field(ui, r, actions);
        return;
    }
    let header = CollapsingHeader::new(RichText::new(name).strong())
        .id_salt(("folder", name))
        .default_open(true)
        .show(ui, |ui| {
            if members.is_empty() {
                ui.label(RichText::new("Empty: drag connections here").weak().small());
            }
            for c in members {
                connection_node(ui, app_ref(app, folders), client, c, filter, dark, actions);
            }
        });
    let resp = &header.header_response;
    if let Some(conn) = conn_drop(ui, resp) {
        actions.push(Action::Move {
            conn,
            folder: Some(name.to_string()),
        });
    }
    resp.context_menu(|ui| {
        if ui.button("New connection here…").clicked() {
            actions.push(Action::NewConnection(Some(name.to_string())));
            ui.close();
        }
        if ui.button("Rename").clicked() {
            actions.push(Action::StartRename(target.clone(), name.to_string()));
            ui.close();
        }
        if members.is_empty() && ui.button("Delete folder").clicked() {
            actions.push(Action::DeleteFolder(name.to_string()));
            ui.close();
        }
    });
}

/// The in-place rename field. Enter or clicking elsewhere saves; Esc cancels.
fn rename_field(ui: &mut egui::Ui, r: &mut Rename, actions: &mut Vec<Action>) {
    let id = egui::Id::new("tree_rename");
    let mut out = egui::TextEdit::singleline(&mut r.text)
        .id(id)
        .desired_width(f32::INFINITY)
        .show(ui);
    if r.focus {
        r.focus = false;
        out.response.request_focus();
        let all = egui::text::CCursorRange::two(
            egui::text::CCursor::new(0),
            egui::text::CCursor::new(r.text.chars().count()),
        );
        out.state.cursor.set_char_range(Some(all));
        out.state.store(ui.ctx(), id);
    } else if out.response.lost_focus() {
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            actions.push(Action::CancelRename);
        } else {
            actions.push(Action::CommitRename);
        }
    }
}

/// While a connection is dragged over `resp`, outline it; the connection
/// dropped on it.
fn conn_drop(ui: &egui::Ui, resp: &egui::Response) -> Option<Uuid> {
    if resp.dnd_hover_payload::<ConnDrag>().is_some() {
        ui.painter().rect_stroke(
            resp.rect.expand(2.0),
            4.0,
            egui::Stroke::new(1.5, theme::accent(ui.visuals().dark_mode)),
            egui::StrokeKind::Outside,
        );
    }
    resp.dnd_release_payload::<ConnDrag>().map(|d| d.0)
}

/// The parts of the app the tree needs while drawing.
struct TreeCtx<'a> {
    schema: &'a mut SchemaTree,
    worker: &'a crate::worker::Worker,
    /// Every folder, for “Move to folder”.
    folders: &'a [String],
}

fn app_ref<'a>(app: &'a mut SqailApp, folders: &'a [String]) -> TreeCtx<'a> {
    TreeCtx {
        schema: &mut app.schema,
        worker: &app.worker,
        folders,
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
    if let Some(r) = t
        .schema
        .rename
        .as_mut()
        .filter(|r| r.target == RenameTarget::Connection(c.id))
    {
        rename_field(ui, r, actions);
        return;
    }
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
        if ui.button("Rename").clicked() {
            actions.push(Action::StartRename(
                RenameTarget::Connection(c.id),
                c.name.clone(),
            ));
            ui.close();
        }
        if ui.button("Duplicate…").clicked() {
            actions.push(Action::Duplicate(c.id));
            ui.close();
        }
        ui.menu_button("Move to folder", |ui| {
            if c.folder.is_some() && ui.button("(no folder)").clicked() {
                actions.push(Action::Move {
                    conn: c.id,
                    folder: None,
                });
                ui.close();
            }
            for f in t.folders {
                if c.folder.as_deref() != Some(f.as_str()) && ui.button(f).clicked() {
                    actions.push(Action::Move {
                        conn: c.id,
                        folder: Some(f.clone()),
                    });
                    ui.close();
                }
            }
        });
        if ui.button("Test").clicked() {
            actions.push(Action::Test(c.id));
            ui.close();
        }
        if ui.button("Refresh schema").clicked() {
            actions.push(Action::Refresh(c.id));
            ui.close();
        }
        if !c.read_only && ui.button("New table…").clicked() {
            actions.push(Action::NewTable {
                conn: c.id,
                schema: None,
            });
            ui.close();
        }
        if let sqail_client::proto::ConnectionParams::Mssql(p) = &c.params
            && p.auth.is_entra()
            && ui
                .button("Discover Azure databases…")
                .on_hover_text("Add the databases in the Azure subscriptions this sign-in can read")
                .clicked()
        {
            actions.push(Action::DiscoverAzure(c.id));
            ui.close();
        }
        ui.separator();
        if ui.button("Delete…").clicked() {
            actions.push(Action::Delete(c.id, c.name.clone()));
            ui.close();
        }
    });
    if header.header_response.double_clicked() {
        actions.push(Action::Edit(c.id));
    }
    // Drag the row onto a folder (or a connection in one) to move it there.
    let row = &header.header_response;
    let drag = ui.interact(row.rect, row.id, egui::Sense::click_and_drag());
    if drag.dragged()
        && let Some(pos) = ui.ctx().pointer_latest_pos()
    {
        egui::Area::new(egui::Id::new("conn_drag_label"))
            .order(egui::Order::Tooltip)
            .interactable(false)
            .fixed_pos(pos + egui::vec2(14.0, 10.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.label(c.name.as_str());
                });
            });
    }
    drag.dnd_set_drag_payload(ConnDrag(c.id));
    if let Some(conn) = conn_drop(ui, row)
        && conn != c.id
    {
        actions.push(Action::Move {
            conn,
            folder: c.folder.clone(),
        });
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
        let header = CollapsingHeader::new(format!("{label} ({})", items.len()))
            .id_salt((label, conn, schema))
            .default_open(kind == TableKind::Table || !filter.is_empty())
            .show(ui, |ui| {
                for table in items {
                    table_node(ui, t, client, conn, engine, schema, table, actions);
                }
            });
        if kind == TableKind::Table {
            header.header_response.context_menu(|ui| {
                if ui.button("New table…").clicked() {
                    actions.push(Action::NewTable {
                        conn,
                        schema: Some(schema.into()),
                    });
                    ui.close();
                }
            });
        }
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
                            let kind = if r.kind == sqail_client::proto::RoutineKind::Procedure {
                                ObjectKind::Procedure
                            } else {
                                ObjectKind::Function
                            };
                            let item = DropItem {
                                engine,
                                schema: schema.into(),
                                name: name.clone(),
                                kind,
                            };
                            drag_source(ui, &resp, conn, item);
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
    let kind = if table.kind == TableKind::Table {
        ObjectKind::Table
    } else {
        ObjectKind::View
    };
    let item = DropItem {
        engine,
        schema: schema.into(),
        name: name.clone(),
        kind,
    };
    if drag_source(ui, &resp.header_response, conn, item).drag_started() {
        // A drop on a blank line lists the columns; have them ready.
        t.schema.ensure(
            client,
            t.worker,
            conn,
            SchemaKey::Columns(schema.into(), name.clone()),
        );
    }
    resp.header_response.context_menu(|ui| {
        if ui.button("SELECT top 100").clicked() {
            actions.push(Action::SelectTop {
                conn,
                schema: schema.into(),
                table: name.clone(),
            });
            ui.close();
        }
        if table.kind == TableKind::Table && ui.button("Design table…").clicked() {
            actions.push(Action::Design {
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
        if table.kind == TableKind::Table {
            ui.separator();
            if ui
                .button(RichText::new("Drop table…").color(Color32::from_rgb(0xd6, 0x45, 0x45)))
                .clicked()
            {
                actions.push(Action::DropTable {
                    conn,
                    engine,
                    schema: schema.into(),
                    table: name.clone(),
                });
                ui.close();
            }
            ui.separator();
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

/// Make a tree row draggable into the editor. Re-registers the row's own id,
/// so it keeps its click behaviour and gains drag.
fn drag_source(
    ui: &egui::Ui,
    row: &egui::Response,
    connection: Uuid,
    item: DropItem,
) -> egui::Response {
    let drag = ui.interact(row.rect, row.id, egui::Sense::click_and_drag());
    if drag.dragged()
        && let Some(pos) = ui.ctx().pointer_latest_pos()
    {
        egui::Area::new(egui::Id::new("sql_drag_label"))
            .order(egui::Order::Tooltip)
            .interactable(false)
            .fixed_pos(pos + egui::vec2(14.0, 10.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.label(RichText::new(item.qualified()).monospace());
                });
            });
    }
    drag.dnd_set_drag_payload(SqlDrop { connection, item });
    drag
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
        Action::NewConnection(folder) => {
            app.dialog = Dialog::Connection(Box::new(ConnForm::in_folder(folder)));
        }
        Action::NewFolder => crate::folders::new_folder(app),
        Action::DeleteFolder(name) => crate::folders::delete_folder(app, &name),
        Action::StartRename(target, current) => {
            app.schema.rename = Some(Rename::new(target, &current));
        }
        Action::CommitRename => crate::folders::commit_rename(app),
        Action::CancelRename => app.schema.rename = None,
        Action::Move { conn, folder } => crate::folders::move_connection(app, conn, folder),
        Action::Duplicate(id) => {
            if let Some(c) = app.service.connection(id) {
                let taken: Vec<&str> = app
                    .service
                    .connections
                    .iter()
                    .map(|c| c.name.as_str())
                    .collect();
                app.dialog = Dialog::Connection(Box::new(ConnForm::duplicate(c, &taken)));
            }
        }
        Action::DiscoverAzure(id) => {
            let (Some(client), Some(c)) = (app.service.client.clone(), app.service.connection(id))
            else {
                return;
            };
            app.dialog = Dialog::AzureDiscover(Box::new(DiscoverForm::new(c)));
            app.worker.run(async move {
                Msg::AzureDiscovered {
                    source: id,
                    result: client.azure_discover(id).await.map_err(|e| e.to_string()),
                }
            });
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
        Action::Design {
            conn,
            schema,
            table,
        } => crate::designer::open(app, conn, Some(schema), Some(table)),
        Action::NewTable { conn, schema } => crate::designer::open(app, conn, schema, None),
        Action::DropTable {
            conn,
            engine,
            schema,
            table,
        } => {
            app.dialog = Dialog::DropTable(Box::new(crate::dialogs::DropTable::new(
                conn, engine, schema, table,
            )));
        }
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
