//! The visual table designer: a window per table where columns, the primary
//! key, indexes and grants are edited, reviewed as DDL and applied in one
//! transaction. The DDL itself comes from [`model`].

pub mod model;

use anyhow::bail;
use egui::{Color32, RichText};
use sqail_client::proto::{Engine, QueryEvent, QueryRequest, TablePrivileges};
use sqail_client::{Client, On};
use uuid::Uuid;

use crate::app::{Msg, SqailApp};
use model::{Design, IndexPart, Plan, Status};

const RED: Color32 = Color32::from_rgb(0xd6, 0x45, 0x45);
const GREEN: Color32 = Color32::from_rgb(0x3f, 0xa3, 0x5b);
const AMBER: Color32 = Color32::from_rgb(0xe6, 0xa2, 0x3c);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Section {
    #[default]
    Columns,
    Indexes,
    PrimaryKey,
    Security,
    Sql,
}

/// A table as fetched from the catalog.
pub struct Loaded {
    pub design: Design,
    pub privileges: Result<TablePrivileges, String>,
}

pub struct Designer {
    pub id: u64,
    pub conn: Uuid,
    pub conn_name: String,
    pub engine: Engine,
    pub read_only: bool,
    /// `(schema, table)` of the table on the server; `None` until created.
    pub table: Option<(String, String)>,
    pub design: Option<Design>,
    pub load_error: Option<String>,
    pub privileges: Option<Result<TablePrivileges, String>>,
    pub section: Section,
    /// Showing the statements, waiting for the user to run them.
    pub review: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub new_grantee: String,
    /// Grantees added on the security page that have no privilege yet.
    pub extra_grantees: Vec<String>,
    pub schemas: Vec<String>,
}

impl Designer {
    fn title(&self) -> String {
        match (&self.table, &self.design) {
            (Some((_, t)), _) => format!("Table · {t} · {}", self.conn_name),
            (None, Some(d)) if !d.table.name.is_empty() => {
                format!("New table · {} · {}", d.table.name, self.conn_name)
            }
            _ => format!("New table · {}", self.conn_name),
        }
    }

    fn plan(&self) -> Option<Result<Plan, Vec<String>>> {
        self.design.as_ref().map(Design::plan)
    }
}

#[derive(Default)]
pub struct Designers {
    pub list: Vec<Designer>,
    next: u64,
}

// ------------------------------------------------------------ opening --

/// Open the designer on `schema.table`, or on a new table when `table` is
/// `None`. An already open designer for the same table is reused.
pub fn open(app: &mut SqailApp, conn: Uuid, schema: Option<String>, table: Option<String>) {
    let Some(c) = app.service.connection(conn).cloned() else {
        return;
    };
    if let (Some(s), Some(t)) = (&schema, &table)
        && let Some(i) = app.designers.list.iter().position(|d| {
            d.conn == conn && d.table.as_ref().is_some_and(|(ds, dt)| ds == s && dt == t)
        })
    {
        // Bring it to the front.
        let d = app.designers.list.remove(i);
        app.designers.list.push(d);
        return;
    }
    app.designers.next += 1;
    let id = app.designers.next;
    let mut schemas = app.schema.schema_names(conn);
    if schemas.is_empty()
        && let Some(s) = &schema
    {
        schemas.push(s.clone());
    }
    let new_design = table.is_none().then(|| {
        Design::new_table(
            c.engine,
            schema.as_deref().or(schemas.first().map(String::as_str)),
        )
    });
    let d = Designer {
        id,
        conn,
        conn_name: c.name.clone(),
        engine: c.engine,
        read_only: c.read_only,
        table: schema.clone().zip(table.clone()),
        design: new_design,
        load_error: None,
        privileges: None,
        section: Section::Columns,
        review: false,
        busy: false,
        error: None,
        new_grantee: String::new(),
        extra_grantees: Vec::new(),
        schemas,
    };
    app.designers.list.push(d);
    if let (Some(s), Some(t)) = (schema, table) {
        load(app, id, s, t);
    }
}

fn load(app: &mut SqailApp, id: u64, schema: String, table: String) {
    let Some(client) = app.service.client.clone() else {
        return;
    };
    let Some(d) = app.designers.list.iter_mut().find(|d| d.id == id) else {
        return;
    };
    d.design = None;
    d.load_error = None;
    d.review = false;
    d.extra_grantees.clear();
    let (conn, engine) = (d.conn, d.engine);
    app.worker.run(async move {
        let result = fetch(&client, conn, engine, &schema, &table)
            .await
            .map(Box::new);
        Msg::DesignerLoaded { id, result }
    });
}

pub async fn fetch(
    client: &Client,
    conn: Uuid,
    engine: Engine,
    schema: &str,
    table: &str,
) -> Result<Loaded, String> {
    let s = Some(schema);
    let (cols, idx, fks, privs) = futures::join!(
        client.columns(conn, s, table),
        client.indexes(conn, s, table),
        client.foreign_keys(conn, s, table),
        client.privileges(conn, s, table),
    );
    let err = |e: sqail_client::Error| e.to_string();
    let cols = cols.map_err(err)?;
    if cols.is_empty() {
        return Err(format!("table {table} not found"));
    }
    let idx = idx.map_err(err)?;
    let fks = fks.map_err(err)?;
    let privileges = privs.map_err(err);
    let grants = privileges
        .as_ref()
        .map(|p| p.grants.clone())
        .unwrap_or_default();
    Ok(Loaded {
        design: Design::from_catalog(engine, schema, table, &cols, &idx, fks, &grants),
        privileges,
    })
}

/// Handle [`Msg::DesignerLoaded`].
pub fn loaded(app: &mut SqailApp, id: u64, result: Result<Box<Loaded>, String>) {
    let Some(d) = app.designers.list.iter_mut().find(|d| d.id == id) else {
        return;
    };
    match result {
        Ok(l) => {
            let l = *l;
            d.design = Some(l.design);
            d.privileges = Some(l.privileges);
        }
        Err(e) => d.load_error = Some(e),
    }
}

/// Handle [`Msg::DesignerApplied`].
pub fn applied(app: &mut SqailApp, id: u64, result: Result<(), String>) {
    let Some(d) = app.designers.list.iter_mut().find(|d| d.id == id) else {
        return;
    };
    d.busy = false;
    match result {
        Ok(()) => {
            let Some(design) = &d.design else { return };
            let (schema, name) = (design.table.schema.clone(), design.table.name.clone());
            let old = d.table.replace((schema.clone(), name.clone()));
            let conn = d.conn;
            let created = old.is_none();
            if let Some((os, ot)) = old {
                app.schema.invalidate_table(conn, &os, &ot);
            }
            app.schema.invalidate_table(conn, &schema, &name);
            app.notify(
                if created {
                    format!("Created table {name}.")
                } else {
                    format!("Saved table {name}.")
                },
                false,
            );
            load(app, id, schema, name);
        }
        Err(e) => {
            d.error = Some(e);
            d.review = true;
        }
    }
}

/// Close designers showing a table that no longer exists.
pub fn forget_table(app: &mut SqailApp, conn: Uuid, schema: &str, table: &str) {
    app.designers.list.retain(|d| {
        !(d.conn == conn
            && d.table
                .as_ref()
                .is_some_and(|(s, t)| s == schema && t == table))
    });
}

// ------------------------------------------------------------ running --

/// Run a plan: `pre`, then the body in one transaction (with `check`
/// before COMMIT), then `post`; on a fresh session that is closed after.
pub async fn apply(
    client: &Client,
    connection: Uuid,
    engine: Engine,
    plan: Plan,
) -> anyhow::Result<()> {
    let session = client.open_session(connection).await?.id;
    // Returns how many rows the statement's result sets had.
    let run = |sql: String| async move {
        let events = client
            .query_all(
                On::Session(session),
                &QueryRequest {
                    sql,
                    params: Vec::new(),
                    max_rows: Some(20),
                    timeout_ms: None,
                },
            )
            .await?;
        let mut rows = 0u64;
        for e in events {
            match e {
                QueryEvent::Error { message, .. } => bail!(message),
                QueryEvent::ResultEnd { row_count, .. } => rows += row_count,
                _ => {}
            }
        }
        Ok::<u64, anyhow::Error>(rows)
    };
    let begin = if engine == Engine::Mssql {
        "BEGIN TRANSACTION"
    } else {
        "BEGIN"
    };
    let result = async {
        for sql in &plan.pre {
            run(sql.clone()).await?;
        }
        run(begin.into()).await?;
        let body = async {
            let n = plan.body.len();
            for (i, sql) in plan.body.iter().enumerate() {
                run(sql.clone())
                    .await
                    .map_err(|e| anyhow::anyhow!("statement {} of {n}: {e}", i + 1))?;
            }
            if let Some(check) = &plan.check
                && run(check.clone()).await? > 0
            {
                bail!("the change would break foreign keys ({check} found problems)");
            }
            run("COMMIT".into()).await?;
            Ok::<(), anyhow::Error>(())
        }
        .await;
        if body.is_err() {
            let _ = run("ROLLBACK".into()).await;
        }
        body
    }
    .await;
    for sql in &plan.post {
        let _ = run(sql.clone()).await;
    }
    let _ = client.close_session(session).await;
    result
}

// ------------------------------------------------------------- window --

/// What the window asks the app to do after drawing.
enum Request {
    Apply(Plan),
    Reload,
    Drop,
    OpenScript(String),
    Close,
}

pub fn show(ctx: &egui::Context, app: &mut SqailApp) {
    let mut requests: Vec<(u64, Request)> = Vec::new();
    // Starts centred, each further window a little offset. The top-left
    // corner is the anchor so the window doesn't move when its height changes.
    let size = egui::vec2(860.0, 520.0);
    let screen = ctx.content_rect();
    let origin = (screen.center() - size / 2.0).max(screen.min);
    for d in &mut app.designers.list {
        let mut open = true;
        let offset = ((d.id - 1) % 6) as f32 * 28.0;
        egui::Window::new(d.title())
            .id(egui::Id::new(("designer", d.id)))
            .open(&mut open)
            .default_pos(origin + egui::vec2(offset, offset))
            .default_size(size)
            .min_width(560.0)
            .min_height(320.0)
            .resizable(true)
            .collapsible(true)
            .show(ctx, |ui| {
                if let Some(r) = window(ui, d) {
                    requests.push((d.id, r));
                }
            });
        if !open {
            requests.push((d.id, Request::Close));
        }
    }
    for (id, r) in requests {
        handle(app, id, r);
    }
}

fn handle(app: &mut SqailApp, id: u64, r: Request) {
    let Some(pos) = app.designers.list.iter().position(|d| d.id == id) else {
        return;
    };
    match r {
        Request::Close => {
            app.designers.list.remove(pos);
        }
        Request::Reload => {
            if let Some((s, t)) = app.designers.list[pos].table.clone() {
                load(app, id, s, t);
            }
        }
        Request::OpenScript(text) => {
            let d = &app.designers.list[pos];
            let title = match &d.design {
                Some(x) if !x.table.name.is_empty() => format!("{}.sql", x.table.name),
                _ => "table.sql".into(),
            };
            let conn = d.conn;
            app.open_text_tab(conn, title, text);
        }
        Request::Drop => {
            let d = &app.designers.list[pos];
            if let Some((schema, table)) = d.table.clone() {
                app.dialog = crate::dialogs::Dialog::DropTable(Box::new(
                    crate::dialogs::DropTable::new(d.conn, d.engine, schema, table),
                ));
            }
        }
        Request::Apply(plan) => {
            let Some(client) = app.service.client.clone() else {
                return;
            };
            let d = &mut app.designers.list[pos];
            d.busy = true;
            d.error = None;
            let (conn, engine) = (d.conn, d.engine);
            app.worker.run(async move {
                let result = apply(&client, conn, engine, plan)
                    .await
                    .map_err(|e| format!("{e:#}"));
                Msg::DesignerApplied { id, result }
            });
        }
    }
}

fn window(ui: &mut egui::Ui, d: &mut Designer) -> Option<Request> {
    if let Some(e) = &d.load_error {
        ui.colored_label(RED, e);
        return ui.button("Retry").clicked().then_some(Request::Reload);
    }
    if d.design.is_none() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Loading table…");
        });
        return None;
    }
    if d.review && !matches!(d.plan(), Some(Ok(_))) {
        d.review = false;
    }
    // Panels, not a height computed from `available_height()`: the body gets
    // exactly what the header and footer leave, so the window cannot feed
    // its own size back into itself and grow a little on every repaint.
    let mut request = None;
    egui::Panel::top(egui::Id::new(("designer_top", d.id)))
        .resizable(false)
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            header(ui, d);
            ui.add_space(4.0);
            if d.review {
                review_top(ui, d);
            } else {
                tabs(ui, d);
            }
            ui.add_space(2.0);
        });
    egui::Panel::bottom(egui::Id::new(("designer_bottom", d.id)))
        .resizable(false)
        .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(0, 4)))
        .show(ui, |ui| {
            if d.review {
                review_bottom(ui, d, &mut request);
            } else {
                footer_ui(ui, d, &mut request);
            }
        });
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(0, 4)))
        .show(ui, |ui| {
            egui::ScrollArea::both()
                .id_salt(("designer_body", d.id, d.review))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if d.review {
                        review_body(ui, d);
                        return;
                    }
                    match d.section {
                        Section::Columns => columns(ui, d),
                        Section::Indexes => indexes(ui, d),
                        Section::PrimaryKey => primary_key(ui, d),
                        Section::Security => security(ui, d),
                        Section::Sql => sql(ui, d, &mut request),
                    }
                });
        });
    request
}

fn tabs(ui: &mut egui::Ui, d: &mut Designer) {
    ui.horizontal(|ui| {
        let design = d.design.as_ref().expect("loaded");
        let supported = d.engine != Engine::Sqlite;
        for (s, label) in [
            (
                Section::Columns,
                format!("Columns ({})", design.table.columns.len()),
            ),
            (
                Section::Indexes,
                format!("Indexes ({})", design.table.indexes.len()),
            ),
            (Section::PrimaryKey, "Primary key".to_string()),
            (Section::Security, "Security".to_string()),
            (Section::Sql, "SQL".to_string()),
        ] {
            let enabled = s != Section::Security || supported;
            let resp = ui.add_enabled(enabled, egui::Button::selectable(d.section == s, label));
            let resp = if enabled {
                resp
            } else {
                resp.on_disabled_hover_text("SQLite has no table privileges")
            };
            if resp.clicked() {
                d.section = s;
            }
        }
    });
}

fn header(ui: &mut egui::Ui, d: &mut Designer) {
    let editable = !d.review && !d.busy;
    let is_new = d.table.is_none();
    let schemas = d.schemas.clone();
    let design = d.design.as_mut().expect("loaded");
    ui.horizontal(|ui| {
        if d.engine != Engine::Sqlite || schemas.len() > 1 {
            ui.label("Schema");
            if is_new && editable {
                egui::ComboBox::from_id_salt(("designer_schema", d.id))
                    .selected_text(design.table.schema.clone())
                    .show_ui(ui, |ui| {
                        for s in &schemas {
                            ui.selectable_value(&mut design.table.schema, s.clone(), s);
                        }
                    });
            } else {
                ui.label(RichText::new(&design.table.schema).strong());
            }
        }
        ui.label("Table");
        ui.add_enabled(
            editable,
            egui::TextEdit::singleline(&mut design.table.name)
                .hint_text("name")
                .desired_width(220.0),
        );
        if d.read_only {
            ui.label(RichText::new("read-only connection").color(AMBER));
        }
    });
}

fn status_label(ui: &mut egui::Ui, s: Status) {
    match s {
        Status::New => ui.label(RichText::new("new").small().color(GREEN)),
        Status::Changed => ui.label(RichText::new("changed").small().color(AMBER)),
        Status::Same => ui.label(""),
    };
}

fn columns(ui: &mut egui::Ui, d: &mut Designer) {
    let engine = d.engine;
    let id = d.id;
    let design = d.design.as_mut().expect("loaded");
    let is_new = design.is_new();
    let mut remove = None;
    let mut toggle_pk = None;
    let mut moved = None;
    let row_h = ui.spacing().interact_size.y;
    egui::Grid::new(("designer_columns", id))
        .striped(true)
        .num_columns(7)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label(RichText::new("Key").strong())
                .on_hover_text("Part of the primary key");
            ui.label(RichText::new("Name").strong());
            ui.label(RichText::new("Type").strong());
            ui.label(RichText::new("Null").strong())
                .on_hover_text("Allows NULL");
            ui.label(RichText::new("Default").strong());
            ui.label("");
            ui.label("");
            ui.end_row();
            let ids: Vec<u64> = design.table.columns.iter().map(|c| c.id).collect();
            let last = ids.len().saturating_sub(1);
            for (row, cid) in ids.iter().copied().enumerate() {
                let mut in_pk = design.in_pk(cid);
                if ui.checkbox(&mut in_pk, "").changed() {
                    toggle_pk = Some(cid);
                }
                let status = design.column_status(cid);
                let c = design.column_mut(cid).expect("listed");
                ui.add_sized([170.0, row_h], egui::TextEdit::singleline(&mut c.name));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    ui.add(
                        egui::TextEdit::singleline(&mut c.data_type)
                            .desired_width(170.0)
                            .font(egui::TextStyle::Monospace),
                    );
                    ui.menu_button("⏷", |ui| {
                        for t in model::type_suggestions(engine) {
                            if ui.button(RichText::new(*t).monospace()).clicked() {
                                c.data_type = t.to_string();
                                ui.close();
                            }
                        }
                    });
                });
                ui.add_enabled(!in_pk, egui::Checkbox::new(&mut c.nullable, ""))
                    .on_disabled_hover_text("Key columns cannot be NULL");
                ui.add_sized(
                    [150.0, row_h],
                    egui::TextEdit::singleline(&mut c.default)
                        .hint_text("none")
                        .font(egui::TextStyle::Monospace),
                );
                status_label(ui, status);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if is_new {
                        if ui
                            .add_enabled(row > 0, egui::Button::new("↑").small())
                            .clicked()
                        {
                            moved = Some((cid, -1));
                        }
                        if ui
                            .add_enabled(row < last, egui::Button::new("↓").small())
                            .clicked()
                        {
                            moved = Some((cid, 1));
                        }
                    }
                    if ui
                        .small_button("×")
                        .on_hover_text("Remove column")
                        .clicked()
                    {
                        remove = Some(cid);
                    }
                });
                ui.end_row();
            }
        });
    if let Some(c) = toggle_pk {
        design.toggle_pk(c);
    }
    if let Some(c) = remove {
        design.remove_column(c);
    }
    if let Some((c, delta)) = moved {
        design.move_column(c, delta);
    }
    ui.add_space(4.0);
    if ui.button("+ Add column").clicked() {
        design.add_column();
    }
    let removed = design.removed_columns();
    if !removed.is_empty() {
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Dropped:").color(RED));
            for c in removed {
                ui.label(RichText::new(&c.name).strikethrough().color(RED));
                if ui.small_button("restore").clicked() {
                    design.restore_column(c.id);
                }
            }
        });
    }
}

fn indexes(ui: &mut egui::Ui, d: &mut Designer) {
    let id = d.id;
    let engine = d.engine;
    let design = d.design.as_mut().expect("loaded");
    if design.table.indexes.is_empty() {
        ui.label(RichText::new("No indexes besides the primary key.").weak());
    }
    let column_ids: Vec<(u64, String)> = design
        .table
        .columns
        .iter()
        .map(|c| (c.id, c.name.clone()))
        .collect();
    let mut remove = None;
    let ids: Vec<u64> = design.table.indexes.iter().map(|i| i.id).collect();
    egui::Grid::new(("designer_indexes", id))
        .striped(true)
        .num_columns(5)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            if !ids.is_empty() {
                ui.label(RichText::new("Name").strong());
                ui.label(RichText::new("Unique").strong());
                ui.label(RichText::new("Columns").strong());
                ui.label("");
                ui.label("");
                ui.end_row();
            }
            for iid in ids {
                let status = {
                    let i = design
                        .table
                        .indexes
                        .iter()
                        .find(|i| i.id == iid)
                        .expect("listed");
                    design.index_status(i)
                };
                let labels: Vec<String> = {
                    let i = design
                        .table
                        .indexes
                        .iter()
                        .find(|i| i.id == iid)
                        .expect("listed");
                    i.parts.iter().map(|p| design.part_label(p)).collect()
                };
                let i = design.index_mut(iid).expect("listed");
                ui.add_sized(
                    [200.0, ui.spacing().interact_size.y],
                    egui::TextEdit::singleline(&mut i.name),
                );
                ui.checkbox(&mut i.unique, "");
                let shown = if labels.is_empty() {
                    "choose…".to_string()
                } else {
                    labels.join(", ")
                };
                ui.menu_button(shown, |ui| {
                    ui.set_min_width(220.0);
                    ui.label(RichText::new("Key order").weak());
                    let mut up = None;
                    let mut drop_part = None;
                    for (pos, (p, label)) in i.parts.iter().zip(&labels).enumerate() {
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(pos > 0, egui::Button::new("↑").small())
                                .clicked()
                            {
                                up = Some(pos);
                            }
                            if ui.small_button("×").clicked() {
                                drop_part = Some(pos);
                            }
                            let text = RichText::new(label);
                            ui.label(if matches!(p, IndexPart::Expr(_)) {
                                text.italics()
                            } else {
                                text
                            });
                        });
                    }
                    if let Some(pos) = up {
                        i.parts.swap(pos, pos - 1);
                    }
                    if let Some(pos) = drop_part {
                        i.parts.remove(pos);
                    }
                    ui.separator();
                    ui.label(RichText::new("Add").weak());
                    for (cid, name) in &column_ids {
                        let part = IndexPart::Column(*cid);
                        if !i.parts.contains(&part) && ui.button(format!("+ {name}")).clicked() {
                            i.parts.push(part);
                        }
                    }
                });
                ui.horizontal(|ui| {
                    if i.constraint {
                        ui.label(RichText::new("constraint").small().weak())
                            .on_hover_text(if engine == Engine::Sqlite {
                                "Declared as UNIQUE in the table; changing it rebuilds the table"
                            } else {
                                "Backs a UNIQUE constraint"
                            });
                    }
                    status_label(ui, status);
                });
                if ui.small_button("×").on_hover_text("Drop index").clicked() {
                    remove = Some(iid);
                }
                ui.end_row();
            }
        });
    if let Some(i) = remove {
        design.remove_index(i);
    }
    ui.add_space(4.0);
    if ui.button("+ Add index").clicked() {
        design.add_index();
    }
    if engine == Engine::Postgres {
        ui.add_space(6.0);
        ui.label(
            RichText::new(
                "An index you change is re-created as a plain B-tree index (method, \
                 INCLUDE columns and WHERE clause are not kept).",
            )
            .small()
            .weak(),
        );
    }
}

fn primary_key(ui: &mut egui::Ui, d: &mut Designer) {
    let engine = d.engine;
    let design = d.design.as_mut().expect("loaded");
    if engine != Engine::Sqlite {
        ui.horizontal(|ui| {
            ui.label("Constraint name");
            ui.add(
                egui::TextEdit::singleline(&mut design.table.primary_key.name)
                    .hint_text("chosen by the database")
                    .desired_width(240.0),
            );
        });
        ui.add_space(4.0);
    }
    let pk = design.table.primary_key.columns.clone();
    if pk.is_empty() {
        ui.label(RichText::new("The table has no primary key.").weak());
    }
    let mut moved = None;
    let mut remove = None;
    for (pos, cid) in pk.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(format!("{}.", pos + 1));
            ui.label(RichText::new(design.column_name(*cid)).strong());
            if ui
                .add_enabled(pos > 0, egui::Button::new("↑").small())
                .clicked()
            {
                moved = Some((pos, -1));
            }
            if ui
                .add_enabled(pos + 1 < pk.len(), egui::Button::new("↓").small())
                .clicked()
            {
                moved = Some((pos, 1));
            }
            if ui
                .small_button("×")
                .on_hover_text("Take out of the key")
                .clicked()
            {
                remove = Some(*cid);
            }
        });
    }
    if let Some((pos, delta)) = moved {
        design.move_pk_column(pos, delta);
    }
    if let Some(c) = remove {
        design.toggle_pk(c);
    }
    ui.add_space(4.0);
    let candidates: Vec<(u64, String)> = design
        .table
        .columns
        .iter()
        .filter(|c| !pk.contains(&c.id))
        .map(|c| (c.id, c.name.clone()))
        .collect();
    ui.horizontal(|ui| {
        ui.add_enabled_ui(!candidates.is_empty(), |ui| {
            ui.menu_button("+ Add column to key", |ui| {
                for (cid, name) in &candidates {
                    if ui.button(name).clicked() {
                        design.toggle_pk(*cid);
                        ui.close();
                    }
                }
            });
        });
        if !pk.is_empty()
            && ui
                .button(RichText::new("Remove primary key").color(RED))
                .clicked()
        {
            design.table.primary_key.columns.clear();
        }
    });
    ui.add_space(6.0);
    ui.label(
        RichText::new("Key columns become NOT NULL. Changing the key drops and re-creates it.")
            .small()
            .weak(),
    );
}

fn security(ui: &mut egui::Ui, d: &mut Designer) {
    let engine = d.engine;
    let id = d.id;
    let (principals, owner) = match &d.privileges {
        Some(Ok(p)) => (p.principals.clone(), p.owner.clone()),
        Some(Err(e)) => {
            ui.colored_label(RED, format!("Could not read the grants: {e}"));
            return;
        }
        None => (Vec::new(), None),
    };
    let design = d.design.as_mut().expect("loaded");
    if let Some(o) = owner {
        ui.label(format!("Owner: {o} (holds every privilege)"));
        ui.add_space(4.0);
    }
    let mut privs: Vec<String> = model::privileges(engine)
        .iter()
        .map(|s| s.to_string())
        .collect();
    for (_, p) in &design.table.grants {
        if !privs.contains(p) {
            privs.push(p.clone());
        }
    }
    let mut grantees = design.grantees();
    for g in &d.extra_grantees {
        if !grantees.contains(g) {
            grantees.push(g.clone());
        }
    }
    if grantees.is_empty() {
        ui.label(RichText::new("Nobody has been granted privileges on this table.").weak());
    }
    let mut revoke = None;
    egui::Grid::new(("designer_grants", id))
        .striped(true)
        .num_columns(privs.len() + 2)
        .spacing([10.0, 4.0])
        .show(ui, |ui| {
            if !grantees.is_empty() {
                ui.label(RichText::new("Grantee").strong());
                for p in &privs {
                    ui.label(RichText::new(p).strong().small());
                }
                ui.label("");
                ui.end_row();
            }
            for g in &grantees {
                ui.label(g);
                for p in &privs {
                    let mut on = design.has_grant(g, p);
                    if ui.checkbox(&mut on, "").changed() {
                        design.set_grant(g, p, on);
                    }
                }
                if ui
                    .small_button("×")
                    .on_hover_text("Revoke everything")
                    .clicked()
                {
                    revoke = Some(g.clone());
                }
                ui.end_row();
            }
        });
    if let Some(g) = revoke {
        design.revoke_all(&g);
        d.extra_grantees.retain(|x| x != &g);
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let others: Vec<&String> = principals
            .iter()
            .filter(|p| !grantees.contains(p))
            .collect();
        ui.add(
            egui::TextEdit::singleline(&mut d.new_grantee)
                .hint_text("role or user")
                .desired_width(180.0),
        );
        if !others.is_empty() {
            ui.menu_button("⏷", |ui| {
                for p in others {
                    if ui.button(p).clicked() {
                        d.new_grantee = p.clone();
                        ui.close();
                    }
                }
            });
        }
        let name = d.new_grantee.trim().to_string();
        if ui
            .add_enabled(!name.is_empty(), egui::Button::new("+ Add grantee"))
            .clicked()
        {
            if !grantees.contains(&name) {
                d.extra_grantees.push(name);
            }
            d.new_grantee.clear();
        }
    });
}

fn sql(ui: &mut egui::Ui, d: &Designer, request: &mut Option<Request>) {
    match d.plan() {
        Some(Ok(plan)) if plan.is_empty() => {
            ui.label(RichText::new("No changes.").weak());
        }
        Some(Ok(plan)) => {
            let mut text = plan.script(d.engine);
            if ui.button("Open in editor").clicked() {
                *request = Some(Request::OpenScript(text.clone()));
            }
            ui.add(
                egui::TextEdit::multiline(&mut text)
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .interactive(true),
            );
        }
        Some(Err(problems)) => {
            for p in problems {
                ui.colored_label(RED, p);
            }
        }
        None => {}
    }
}

fn review_top(ui: &mut egui::Ui, d: &Designer) {
    let Some(Ok(plan)) = d.plan() else { return };
    ui.heading(if d.table.is_none() {
        "Create the table"
    } else {
        "Apply the changes"
    });
    ui.label(
        RichText::new("These statements run in one transaction; if one fails, nothing changes.")
            .weak(),
    );
    for w in &plan.warnings {
        ui.colored_label(AMBER, w);
    }
}

fn review_body(ui: &mut egui::Ui, d: &Designer) {
    let Some(Ok(plan)) = d.plan() else { return };
    let mut text = plan.script(d.engine);
    ui.add(
        egui::TextEdit::multiline(&mut text)
            .code_editor()
            .desired_width(f32::INFINITY)
            .interactive(true),
    );
}

fn review_bottom(ui: &mut egui::Ui, d: &mut Designer, request: &mut Option<Request>) {
    let Some(Ok(plan)) = d.plan() else { return };
    if let Some(e) = &d.error {
        ui.colored_label(RED, format!("Rolled back: {e}"));
    }
    ui.horizontal(|ui| {
        let label = if d.table.is_none() { "Create" } else { "Apply" };
        if ui
            .add_enabled(!d.busy, egui::Button::new(RichText::new(label).strong()))
            .clicked()
        {
            *request = Some(Request::Apply(plan));
        }
        if d.busy {
            ui.spinner();
        }
        if ui.add_enabled(!d.busy, egui::Button::new("Back")).clicked() {
            d.review = false;
            d.error = None;
        }
    });
}

fn footer_ui(ui: &mut egui::Ui, d: &mut Designer, request: &mut Option<Request>) {
    let plan = d.plan();
    if let Some(Err(problems)) = &plan {
        ui.colored_label(RED, problems.join(" "));
    }
    let pending = match &plan {
        Some(Ok(p)) => p.body.len(),
        _ => 0,
    };
    ui.horizontal(|ui| {
        let is_new = d.table.is_none();
        let label = if is_new {
            "Create table…".to_string()
        } else if pending > 0 {
            format!("Apply {pending} change(s)…")
        } else {
            "Apply…".to_string()
        };
        let ready = matches!(&plan, Some(Ok(p)) if !p.is_empty());
        if ui
            .add_enabled(
                ready && !d.read_only && !d.busy,
                egui::Button::new(RichText::new(label).strong()),
            )
            .on_disabled_hover_text(if d.read_only {
                "The connection is read-only"
            } else {
                "Nothing to apply"
            })
            .clicked()
        {
            d.review = true;
            d.error = None;
        }
        if !is_new {
            if ui
                .add_enabled(pending > 0, egui::Button::new("Revert"))
                .on_hover_text("Discard the edits")
                .clicked()
                && let Some(x) = d.design.as_mut()
            {
                x.revert();
                d.extra_grantees.clear();
            }
            if ui
                .button("⟳")
                .on_hover_text("Reload from the database (discards the edits)")
                .clicked()
            {
                *request = Some(Request::Reload);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        !d.read_only,
                        egui::Button::new(RichText::new("Drop table…").color(RED)),
                    )
                    .clicked()
                {
                    *request = Some(Request::Drop);
                }
            });
        }
    });
}
