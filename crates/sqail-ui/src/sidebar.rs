//! The left panel: a switcher between connections, history and snippets.

use egui::{Color32, RichText};

use crate::app::SqailApp;
use crate::dialogs::Dialog;
use crate::local::Outcome;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    Connections,
    History,
    Snippets,
}

pub fn ui(ui: &mut egui::Ui, app: &mut SqailApp) {
    ui.horizontal(|ui| {
        for (v, label) in [
            (View::Connections, "Connections"),
            (View::History, "History"),
            (View::Snippets, "Snippets"),
        ] {
            if ui.selectable_label(app.sidebar == v, label).clicked() {
                app.sidebar = v;
            }
        }
    });
    ui.separator();
    match app.sidebar {
        View::Connections => crate::schema::sidebar_ui(ui, app),
        View::History => history_ui(ui, app),
        View::Snippets => snippets_ui(ui, app),
    }
}

fn first_line(sql: &str) -> String {
    let line = sql
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let mut s: String = line.chars().take(90).collect();
    if s.len() < sql.trim().len() {
        s.push('…');
    }
    s
}

fn ago(at: u64) -> String {
    let secs = crate::local::now_secs().saturating_sub(at);
    match secs {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86400 => format!("{} h ago", secs / 3600),
        _ => format!("{} d ago", secs / 86400),
    }
}

enum Pick {
    NewTab(String, Option<uuid::Uuid>),
    Insert(String),
    Copy(String),
}

fn apply(app: &mut SqailApp, ctx: &egui::Context, pick: Pick) {
    match pick {
        Pick::NewTab(sql, conn) => {
            let idx = app.new_tab(conn);
            app.tabs[idx].text = sql;
        }
        Pick::Insert(sql) => {
            if let Some(t) = app.tabs.get_mut(app.active) {
                t.insert_at_cursor(&sql);
            }
        }
        Pick::Copy(sql) => ctx.copy_text(sql),
    }
}

fn history_ui(ui: &mut egui::Ui, app: &mut SqailApp) {
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut app.history_filter)
                .hint_text("Search history…")
                .desired_width(ui.available_width() - 60.0),
        );
        if ui
            .small_button("Clear")
            .on_hover_text("Delete all history")
            .clicked()
        {
            app.history.clear();
        }
    });
    let filter = app.history_filter.to_lowercase();
    let mut pick = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let matching = app
                .history
                .entries
                .iter()
                .rev()
                .filter(|e| {
                    filter.is_empty()
                        || e.sql.to_lowercase().contains(&filter)
                        || e.connection_name.to_lowercase().contains(&filter)
                })
                .take(500);
            let mut any = false;
            for e in matching {
                any = true;
                let color = match e.outcome {
                    Outcome::Ok => ui.visuals().text_color(),
                    Outcome::Failed => Color32::from_rgb(0xd6, 0x45, 0x45),
                    Outcome::Cancelled => Color32::from_rgb(0xe6, 0xa2, 0x3c),
                };
                let resp = ui
                    .vertical(|ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(first_line(&e.sql)).monospace().color(color),
                            )
                            .selectable(false),
                        );
                        ui.add(
                            egui::Label::new(
                                RichText::new(format!(
                                    "{} · {} ms · {} rows · {}",
                                    e.connection_name,
                                    e.duration_ms,
                                    e.rows,
                                    ago(e.at)
                                ))
                                .small()
                                .weak(),
                            )
                            .selectable(false),
                        );
                    })
                    .response
                    .interact(egui::Sense::click())
                    .on_hover_text(&e.sql);
                if resp.double_clicked() {
                    pick = Some(Pick::NewTab(e.sql.clone(), e.connection));
                }
                resp.context_menu(|ui| {
                    if ui.button("Open in new tab").clicked() {
                        pick = Some(Pick::NewTab(e.sql.clone(), e.connection));
                        ui.close();
                    }
                    if ui.button("Insert at cursor").clicked() {
                        pick = Some(Pick::Insert(e.sql.clone()));
                        ui.close();
                    }
                    if ui.button("Copy").clicked() {
                        pick = Some(Pick::Copy(e.sql.clone()));
                        ui.close();
                    }
                });
                ui.separator();
            }
            if !any {
                ui.label(RichText::new("Nothing yet. Queries you run appear here.").weak());
            }
        });
    if let Some(p) = pick {
        apply(app, ui.ctx(), p);
    }
}

fn snippets_ui(ui: &mut egui::Ui, app: &mut SqailApp) {
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut app.snippet_filter)
                .hint_text("Search snippets…")
                .desired_width(ui.available_width() - 80.0),
        );
        if ui
            .button("+ Save")
            .on_hover_text("Save the selection or current statement")
            .clicked()
        {
            app.dialog = Dialog::SaveSnippet {
                name: String::new(),
                sql: app.current_sql_for_snippet(),
            };
        }
    });
    let filter = app.snippet_filter.to_lowercase();
    let mut pick = None;
    let mut delete = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if app.snippets.items.is_empty() {
                ui.label(RichText::new("No snippets. Select SQL and press “+ Save”.").weak());
            }
            for s in app.snippets.items.iter().filter(|s| {
                filter.is_empty()
                    || s.name.to_lowercase().contains(&filter)
                    || s.sql.to_lowercase().contains(&filter)
            }) {
                let resp = ui
                    .vertical(|ui| {
                        ui.add(egui::Label::new(RichText::new(&s.name).strong()).selectable(false));
                        ui.add(
                            egui::Label::new(
                                RichText::new(first_line(&s.sql)).monospace().small().weak(),
                            )
                            .selectable(false),
                        );
                    })
                    .response
                    .interact(egui::Sense::click())
                    .on_hover_text(&s.sql);
                if resp.double_clicked() {
                    pick = Some(Pick::Insert(s.sql.clone()));
                }
                resp.context_menu(|ui| {
                    if ui.button("Insert at cursor").clicked() {
                        pick = Some(Pick::Insert(s.sql.clone()));
                        ui.close();
                    }
                    if ui.button("Open in new tab").clicked() {
                        pick = Some(Pick::NewTab(s.sql.clone(), None));
                        ui.close();
                    }
                    if ui.button("Copy").clicked() {
                        pick = Some(Pick::Copy(s.sql.clone()));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Delete").clicked() {
                        delete = Some(s.id);
                        ui.close();
                    }
                });
                ui.separator();
            }
        });
    if let Some(id) = delete {
        app.snippets.remove(id);
    }
    if let Some(p) = pick {
        apply(app, ui.ctx(), p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_lines() {
        assert_eq!(first_line("\n  SELECT 1\nFROM t"), "SELECT 1…");
        assert_eq!(first_line("SELECT 1"), "SELECT 1");
    }
}
