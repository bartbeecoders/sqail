//! The command palette (Ctrl+Shift+P) and quick-open for tables and
//! snippets (Ctrl+P): a filter box over a list, driven by the keyboard.

use egui::{Id, Key, Modal, Modifiers, RichText};

use crate::app::SqailApp;
use crate::commands::{Command, fuzzy_score};

pub struct Palette {
    /// Quick-open (tables, snippets) instead of commands.
    pub quick: bool,
    pub query: String,
    pub selected: usize,
}

impl Palette {
    pub fn new(quick: bool) -> Self {
        Self {
            quick,
            query: String::new(),
            selected: 0,
        }
    }
}

#[derive(Clone)]
enum Item {
    Command(Command),
    Table { schema: String, name: String },
    Snippet { sql: String },
}

struct Row {
    label: String,
    detail: String,
    item: Item,
}

fn rows(app: &SqailApp, ctx: &egui::Context, quick: bool) -> Vec<Row> {
    if !quick {
        return Command::ALL
            .iter()
            .map(|&c| Row {
                label: c.title().to_string(),
                detail: app.keymap.label(ctx, c),
                item: Item::Command(c),
            })
            .collect();
    }
    let mut out = Vec::new();
    if let Some(conn) = app.tabs.get(app.active).and_then(|t| t.connection) {
        for (schema, t) in app.schema.loaded_tables(conn) {
            out.push(Row {
                label: format!("{schema}.{}", t.name),
                detail: format!("{:?}", t.kind).to_lowercase(),
                item: Item::Table {
                    schema,
                    name: t.name,
                },
            });
        }
    }
    for s in &app.snippets.items {
        out.push(Row {
            label: s.name.clone(),
            detail: "snippet".into(),
            item: Item::Snippet { sql: s.sql.clone() },
        });
    }
    out
}

pub fn show(ctx: &egui::Context, app: &mut SqailApp) {
    let Some(p) = app.palette.as_ref() else {
        return;
    };
    let (quick, query) = (p.quick, p.query.clone());
    let mut matches: Vec<(i32, Row)> = rows(app, ctx, quick)
        .into_iter()
        .filter_map(|r| fuzzy_score(&query, &r.label).map(|s| (s, r)))
        .collect();
    matches.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.label.cmp(&b.1.label)));
    let n = matches.len();

    let (down, up, enter, esc) = ctx.input_mut(|i| {
        (
            i.consume_key(Modifiers::NONE, Key::ArrowDown),
            i.consume_key(Modifiers::NONE, Key::ArrowUp),
            i.consume_key(Modifiers::NONE, Key::Enter),
            i.consume_key(Modifiers::NONE, Key::Escape),
        )
    });
    let Some(p) = app.palette.as_mut() else {
        return;
    };
    if n > 0 {
        if down {
            p.selected = (p.selected + 1) % n;
        }
        if up {
            p.selected = (p.selected + n - 1) % n;
        }
    }
    p.selected = p.selected.min(n.saturating_sub(1));
    let mut chosen = enter.then_some(p.selected).filter(|_| n > 0);
    let mut close = esc;

    let resp =
        Modal::new(Id::new("palette")).show(ctx, |ui| {
            ui.set_width(560.0);
            let hint = if quick {
                "Open table or snippet…"
            } else {
                "Type a command…"
            };
            let field = ui.add(
                egui::TextEdit::singleline(&mut p.query)
                    .hint_text(hint)
                    .desired_width(f32::INFINITY),
            );
            field.request_focus();
            if field.changed() {
                p.selected = 0;
            }
            ui.separator();
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            if matches.is_empty() {
                let msg = if quick {
                    "Nothing matches. (Tables appear once the connection's schema has loaded.)"
                } else {
                    "No command matches."
                };
                ui.label(RichText::new(msg).weak());
            }
            for (i, (_, r)) in matches.iter().enumerate() {
                let selected = i == p.selected;
                let resp = ui
                    .horizontal(|ui| {
                        let l = ui.selectable_label(selected, &r.label);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(&r.detail).weak().small());
                        });
                        l
                    })
                    .inner;
                if selected && (down || up) {
                    resp.scroll_to_me(None);
                }
                if resp.clicked() {
                    chosen = Some(i);
                }
            }
        });
        });
    if resp.should_close() {
        close = true;
    }

    if let Some(i) = chosen
        && let Some((_, row)) = matches.into_iter().nth(i)
    {
        app.palette = None;
        match row.item {
            Item::Command(c) => app.execute(ctx, c),
            Item::Table { schema, name } => {
                let conn = app.tabs.get(app.active).and_then(|t| t.connection);
                if let (Some(conn), Some(engine)) = (conn, app.engine_of(conn)) {
                    let text = crate::sql::select_top(engine, Some(&schema), &name, 100);
                    let idx = app.open_text_tab(conn, name, text.clone());
                    app.run_sql(idx, text);
                }
            }
            Item::Snippet { sql } => {
                if let Some(t) = app.tabs.get_mut(app.active) {
                    t.insert_at_cursor(&sql);
                }
            }
        }
    } else if close {
        app.palette = None;
    }
}
