//! The assistant panel on the right: provider, conversation, input.

use egui::text::LayoutJob;
use egui::{Color32, FontId, Key, Modifiers, RichText, TextFormat};

use super::cli::Provider;
use super::{Entry, mcp};
use crate::app::SqailApp;

const EXAMPLES: [&str; 3] = [
    "Which tables are there, and how do they relate?",
    "Top 10 customers by revenue last year",
    "Explain the query in my editor and make it faster",
];

pub fn input_id() -> egui::Id {
    egui::Id::new("assistant-input")
}

/// What a click in the conversation asks the app to do.
enum Action {
    Insert(String),
    NewTab(String),
    Ask(String),
}

pub fn ui(ui: &mut egui::Ui, app: &mut SqailApp) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.heading("Assistant");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let close = app
                .keymap
                .label(ui.ctx(), crate::commands::Command::ToggleAssistant);
            if ui
                .small_button("×")
                .on_hover_text(format!("Close ({close})"))
                .clicked()
            {
                app.settings.assistant.open = false;
                app.settings.save();
            }
            if ui
                .add_enabled(
                    !app.assistant.entries.is_empty(),
                    egui::Button::new("New chat"),
                )
                .clicked()
            {
                app.assistant.new_chat();
            }
        });
    });

    let locked = app.assistant.conversation.is_some();
    ui.horizontal(|ui| {
        let mut provider = app
            .assistant
            .conversation
            .as_ref()
            .map_or(app.settings.assistant.provider, |c| c.provider);
        ui.add_enabled_ui(!locked, |ui| {
            egui::ComboBox::from_id_salt("assistant-provider")
                .selected_text(provider.label())
                .show_ui(ui, |ui| {
                    for p in Provider::ALL {
                        ui.selectable_value(&mut provider, p, p.label());
                    }
                });
        })
        .response
        .on_disabled_hover_text("Start a new chat to switch");
        if !locked && provider != app.settings.assistant.provider {
            app.settings.assistant.provider = provider;
            app.settings.save();
        }
        let conn = match &app.assistant.conversation {
            Some(c) => Some((c.connection_name.clone(), c.engine)),
            None => app
                .tabs
                .get(app.active)
                .and_then(|t| t.connection)
                .and_then(|id| app.service.connection(id))
                .map(|c| (c.name.clone(), c.engine)),
        };
        match conn {
            Some((name, engine)) => {
                ui.label(RichText::new(format!("{name} · {}", mcp::engine_name(engine))).small())
            }
            None => ui.label(RichText::new("no connection").small().weak()),
        };
    });
    let provider = app
        .assistant
        .conversation
        .as_ref()
        .map_or(app.settings.assistant.provider, |c| c.provider);
    ui.label(
        RichText::new(format!(
            "{} can read the schema and run read-only queries; up to {} rows per query are sent to it.",
            provider.label(),
            app.settings.assistant.max_rows
        ))
        .small()
        .weak(),
    );
    ui.separator();

    let mut actions = Vec::new();
    egui::Panel::bottom("assistant-input-area")
        .resizable(false)
        .show(ui, |ui| input(ui, app));
    egui::CentralPanel::default().show(ui, |ui| {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| conversation(ui, app, &mut actions));
    });

    for a in actions {
        match a {
            Action::Insert(sql) => {
                if let Some(t) = app.tabs.get_mut(app.active) {
                    t.insert_at_cursor(&sql);
                }
            }
            Action::NewTab(sql) => {
                let conn = app
                    .assistant
                    .conversation
                    .as_ref()
                    .map(|c| c.connection)
                    .or_else(|| app.tabs.get(app.active).and_then(|t| t.connection));
                match conn {
                    Some(c) => {
                        app.open_text_tab(c, "Assistant".into(), sql);
                    }
                    None => {
                        let idx = app.new_tab(None);
                        app.tabs[idx].text = sql;
                        app.active = idx;
                    }
                }
            }
            Action::Ask(q) => {
                app.assistant.input = q;
                app.assistant_send();
            }
        }
    }
}

fn input(ui: &mut egui::Ui, app: &mut SqailApp) {
    ui.add_space(4.0);
    let id = input_id();
    // Enter sends; Shift+Enter is a new line. Taken before the text box
    // sees it.
    let send_key = ui.memory(|m| m.has_focus(id))
        && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
    ui.add(
        egui::TextEdit::multiline(&mut app.assistant.input)
            .id(id)
            .desired_rows(3)
            .desired_width(f32::INFINITY)
            .hint_text("Ask about the data, or describe the query you need…"),
    );
    ui.horizontal(|ui| {
        if ui
            .checkbox(
                &mut app.settings.assistant.include_editor,
                "Include editor SQL",
            )
            .on_hover_text("Send the SQL of the active tab with your question")
            .changed()
        {
            app.settings.save();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if app.assistant.is_running() {
                if ui.button("■ Stop").clicked() {
                    app.assistant.stop();
                }
                ui.spinner();
            } else {
                let ready = !app.assistant.input.trim().is_empty();
                let clicked = ui
                    .add_enabled(ready, egui::Button::new("Send"))
                    .on_hover_text("Enter")
                    .clicked();
                if clicked || (send_key && ready) {
                    app.assistant_send();
                    ui.memory_mut(|m| m.request_focus(id));
                }
            }
        });
    });
    ui.add_space(2.0);
}

fn conversation(ui: &mut egui::Ui, app: &SqailApp, actions: &mut Vec<Action>) {
    if app.assistant.entries.is_empty() {
        ui.add_space(8.0);
        ui.label(RichText::new("Try:").weak());
        for ex in EXAMPLES {
            if ui.link(ex).clicked() {
                actions.push(Action::Ask(ex.into()));
            }
        }
        return;
    }
    let engine = app.assistant.conversation.as_ref().map(|c| c.engine);
    for entry in &app.assistant.entries {
        match entry {
            Entry::User(text) => {
                ui.add_space(6.0);
                egui::Frame::new()
                    .fill(ui.visuals().faint_bg_color)
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(text);
                    });
            }
            Entry::Answer(text) => answer(ui, text, engine, actions),
            Entry::Tool {
                id,
                name,
                input,
                result,
            } => tool(ui, id, name, input, result.as_ref()),
            Entry::Error(e) => {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            Entry::Note(n) => {
                ui.label(RichText::new(n).small().weak());
            }
        }
    }
}

fn tool(
    ui: &mut egui::Ui,
    id: &str,
    name: &str,
    input: &serde_json::Value,
    result: Option<&(String, bool)>,
) {
    let arg = |k: &str| input.get(k).and_then(|v| v.as_str()).unwrap_or("");
    let summary = match name {
        "run_query" => one_line(arg("sql"), 70),
        "describe_table" => arg("table").to_string(),
        "list_tables" => arg("schema").to_string(),
        _ => String::new(),
    };
    let failed = result.is_some_and(|(_, e)| *e);
    let mut header = RichText::new(format!(
        "{name}  {summary}{}",
        if result.is_none() { "  …" } else { "" }
    ))
    .small()
    .monospace();
    header = if failed {
        header.color(ui.visuals().error_fg_color)
    } else {
        header.weak()
    };
    egui::CollapsingHeader::new(header)
        .id_salt(("assistant-tool", id))
        .default_open(false)
        .show(ui, |ui| {
            if name == "run_query" {
                ui.label(RichText::new(arg("sql")).monospace().small());
                ui.separator();
            }
            match result {
                Some((text, _)) => {
                    let shown: String = text.lines().take(40).collect::<Vec<_>>().join("\n");
                    ui.label(RichText::new(shown).monospace().small());
                }
                None => {
                    ui.spinner();
                }
            }
        });
}

fn answer(
    ui: &mut egui::Ui,
    text: &str,
    engine: Option<sqail_client::proto::Engine>,
    actions: &mut Vec<Action>,
) {
    for seg in segments(text) {
        match seg {
            Segment::Text(t) => {
                let t = t.trim_matches('\n');
                if !t.is_empty() {
                    ui.label(inline_markdown(ui, t));
                }
            }
            Segment::Code {
                lang,
                code,
                complete,
            } => {
                egui::Frame::new()
                    .fill(ui.visuals().extreme_bg_color)
                    .corner_radius(4.0)
                    .inner_margin(egui::Margin::same(6))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let sql = lang.is_empty() || lang.eq_ignore_ascii_case("sql");
                        let code_text = code.trim_end();
                        if sql {
                            // Highlighted like the editor.
                            let size = ui.style().text_styles[&egui::TextStyle::Monospace].size;
                            let mut job = crate::sql::highlight::highlight(
                                ui.ctx(),
                                crate::sql::highlight::Key {
                                    text: code_text,
                                    engine,
                                    dark: ui.visuals().dark_mode,
                                    brackets: None,
                                    font_size_bits: size.to_bits(),
                                },
                            );
                            job.wrap.max_width = ui.available_width();
                            ui.add(egui::Label::new(job).selectable(true));
                        } else {
                            ui.add(
                                egui::Label::new(RichText::new(code_text).monospace())
                                    .selectable(true),
                            );
                        }
                        if complete && sql {
                            ui.horizontal(|ui| {
                                let code = code.trim().to_string();
                                if ui
                                    .small_button("Insert")
                                    .on_hover_text("Insert at the cursor in the editor")
                                    .clicked()
                                {
                                    actions.push(Action::Insert(code.clone()));
                                }
                                if ui.small_button("New tab").clicked() {
                                    actions.push(Action::NewTab(code.clone()));
                                }
                                if ui.small_button("Copy").clicked() {
                                    ui.ctx().copy_text(code);
                                }
                            });
                        }
                    });
            }
        }
    }
}

fn one_line(s: &str, max: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &flat[..i]),
        None => flat,
    }
}

#[derive(Debug, PartialEq)]
enum Segment<'a> {
    Text(&'a str),
    /// `complete` is false while the closing fence has not streamed in yet.
    Code {
        lang: &'a str,
        code: &'a str,
        complete: bool,
    },
}

/// Split an answer into text and fenced code blocks.
fn segments(text: &str) -> Vec<Segment<'_>> {
    let mut out = Vec::new();
    let mut rest = text;
    loop {
        let Some(open) = find_fence(rest) else {
            if !rest.is_empty() {
                out.push(Segment::Text(rest));
            }
            return out;
        };
        if open > 0 {
            out.push(Segment::Text(&rest[..open]));
        }
        let after = &rest[open + 3..];
        let (lang, body) = match after.find('\n') {
            Some(nl) => (after[..nl].trim(), &after[nl + 1..]),
            None => (after.trim(), ""),
        };
        match find_fence(body) {
            Some(close) => {
                out.push(Segment::Code {
                    lang,
                    code: &body[..close],
                    complete: true,
                });
                let tail = &body[close + 3..];
                rest = tail.strip_prefix('\n').unwrap_or(tail);
            }
            None => {
                out.push(Segment::Code {
                    lang,
                    code: body,
                    complete: false,
                });
                return out;
            }
        }
    }
}

/// A ``` at the start of a line.
fn find_fence(s: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(i) = s[from..].find("```") {
        let at = from + i;
        let line_start = s[..at].rfind('\n').map_or(0, |n| n + 1);
        if s[line_start..at].trim().is_empty() {
            return Some(at);
        }
        from = at + 3;
    }
    None
}

/// `code` and **bold**; everything else as plain text.
fn inline_markdown(ui: &egui::Ui, text: &str) -> LayoutJob {
    let style = ui.style();
    let body = FontId::proportional(style.text_styles[&egui::TextStyle::Body].size);
    let mono = FontId::monospace(style.text_styles[&egui::TextStyle::Monospace].size);
    let color = ui.visuals().text_color();
    let strong = ui.visuals().strong_text_color();
    let code_bg = ui.visuals().faint_bg_color;
    let mut job = LayoutJob::default();
    job.wrap.max_width = ui.available_width();
    let fmt = |font: FontId, color: Color32, bg: Color32| TextFormat {
        font_id: font,
        color,
        background: bg,
        ..Default::default()
    };
    let mut rest = text;
    while !rest.is_empty() {
        let tick = rest.find('`');
        let bold = rest.find("**");
        match (tick, bold) {
            (Some(t), b) if b.is_none_or(|b| t < b) => {
                job.append(
                    &rest[..t],
                    0.0,
                    fmt(body.clone(), color, Color32::TRANSPARENT),
                );
                let after = &rest[t + 1..];
                match after.find('`') {
                    Some(e) => {
                        job.append(&after[..e], 0.0, fmt(mono.clone(), strong, code_bg));
                        rest = &after[e + 1..];
                    }
                    None => {
                        job.append(
                            &rest[t..],
                            0.0,
                            fmt(body.clone(), color, Color32::TRANSPARENT),
                        );
                        rest = "";
                    }
                }
            }
            (_, Some(b)) => {
                job.append(
                    &rest[..b],
                    0.0,
                    fmt(body.clone(), color, Color32::TRANSPARENT),
                );
                let after = &rest[b + 2..];
                match after.find("**") {
                    Some(e) => {
                        job.append(
                            &after[..e],
                            0.0,
                            fmt(body.clone(), strong, Color32::TRANSPARENT),
                        );
                        rest = &after[e + 2..];
                    }
                    None => {
                        job.append(
                            &rest[b..],
                            0.0,
                            fmt(body.clone(), color, Color32::TRANSPARENT),
                        );
                        rest = "";
                    }
                }
            }
            _ => {
                job.append(rest, 0.0, fmt(body.clone(), color, Color32::TRANSPARENT));
                rest = "";
            }
        }
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_split_into_text_and_code() {
        let text = "Here it is:\n```sql\nSELECT 1;\n```\nThen `x`.\n```\nSELECT 2";
        assert_eq!(
            segments(text),
            [
                Segment::Text("Here it is:\n"),
                Segment::Code {
                    lang: "sql",
                    code: "SELECT 1;\n",
                    complete: true
                },
                Segment::Text("Then `x`.\n"),
                // Still streaming: no closing fence yet.
                Segment::Code {
                    lang: "",
                    code: "SELECT 2",
                    complete: false
                },
            ]
        );
        // Backticks inside a line are not fences.
        assert_eq!(
            segments("use ```x``` inline"),
            [Segment::Text("use ```x``` inline")]
        );
    }

    #[test]
    fn long_sql_is_shortened_to_one_line() {
        assert_eq!(one_line("SELECT a,\n  b\nFROM t", 70), "SELECT a, b FROM t");
        assert_eq!(one_line("abcdef", 3), "abc…");
    }
}
