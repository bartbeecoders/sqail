//! The settings window (File → Settings…, Ctrl+,): every application-wide
//! setting in one place. Changes apply at once and are saved to
//! `settings.toml`; keyboard shortcuts stay in `keybindings.toml`.

use std::path::PathBuf;

use egui::{Id, Modal, RichText};

use crate::app::SqailApp;
use crate::assistant::cli::Provider;
use crate::dialogs::Dialog;
use crate::settings::{Settings, ThemePref};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Section {
    #[default]
    Appearance,
    Editor,
    Tabs,
    Queries,
    Service,
    Assistant,
}

impl Section {
    const ALL: [Section; 6] = [
        Section::Appearance,
        Section::Editor,
        Section::Tabs,
        Section::Queries,
        Section::Service,
        Section::Assistant,
    ];

    fn label(self) -> &'static str {
        match self {
            Section::Appearance => "Appearance",
            Section::Editor => "Editor",
            Section::Tabs => "Tabs",
            Section::Queries => "Queries",
            Section::Service => "Service",
            Section::Assistant => "AI assistant",
        }
    }
}

#[derive(Default)]
pub struct SettingsPage {
    pub section: Section,
}

const SCALES: [f32; 8] = [0.75, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0];
const ROW_LIMITS: [u64; 4] = [1_000, 10_000, 100_000, 1_000_000];

pub fn show(ctx: &egui::Context, app: &mut SqailApp) {
    let Dialog::Settings(page) = &mut app.dialog else {
        return;
    };
    let before = app.settings.clone();
    let mut close = false;
    let resp = Modal::new(Id::new("settings")).show(ctx, |ui| {
        ui.set_width(680.0);
        ui.heading("Settings");
        ui.add_space(4.0);
        // A fixed-height body: the vertical separator would otherwise take
        // the whole screen height.
        let height = (ui.ctx().content_rect().height() - 160.0).clamp(200.0, 380.0);
        let body = egui::vec2(ui.available_width(), height);
        ui.allocate_ui_with_layout(body, egui::Layout::left_to_right(egui::Align::Min), |ui| {
            ui.set_height(height);
            ui.vertical(|ui| {
                ui.set_width(130.0);
                for s in Section::ALL {
                    ui.selectable_value(&mut page.section, s, s.label());
                }
            });
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("settings-body")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Grid::new(("settings", page.section))
                        .num_columns(2)
                        .spacing([16.0, 10.0])
                        .show(ui, |ui| section(ui, page.section, &mut app.settings));
                });
        });
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Close").clicked() {
                close = true;
            }
            ui.label(
                RichText::new("Changes apply right away. Shortcuts: keybindings.toml.").weak(),
            );
        });
    });
    if resp.should_close() {
        close = true;
    }
    if app.settings != before {
        let ctx = ctx.clone();
        let (theme, scale) = (app.settings.theme, app.settings.ui_scale);
        if theme != before.theme {
            app.set_theme(&ctx, theme);
        }
        if scale != before.ui_scale {
            app.set_ui_scale(&ctx, scale);
        }
        app.settings.save();
    }
    if close {
        app.dialog = Dialog::None;
    }
}

/// A labelled row; `add` gets the label's id for `labelled_by`.
fn row(ui: &mut egui::Ui, label: &str, hint: &str, add: impl FnOnce(&mut egui::Ui, Id)) {
    let id = ui.label(label).id;
    ui.vertical(|ui| {
        ui.set_max_width(440.0);
        add(ui, id);
        if !hint.is_empty() {
            ui.label(RichText::new(hint).small().weak());
        }
    });
    ui.end_row();
}

fn section(ui: &mut egui::Ui, section: Section, s: &mut Settings) {
    match section {
        Section::Appearance => {
            row(
                ui,
                "Theme",
                "Omarchy uses the colours of the current Omarchy theme and follows it when \
                 you switch.",
                |ui, label| {
                    egui::ComboBox::from_id_salt("theme")
                        .width(260.0)
                        .selected_text(s.theme.label())
                        .show_ui(ui, |ui| {
                            for t in ThemePref::ALL {
                                ui.selectable_value(&mut s.theme, t, t.label());
                            }
                        })
                        .response
                        .labelled_by(label);
                },
            );
            row(ui, "Interface size", "", |ui, label| {
                egui::ComboBox::from_id_salt("ui_scale")
                    .selected_text(percent(s.ui_scale))
                    .show_ui(ui, |ui| {
                        for f in SCALES {
                            ui.selectable_value(&mut s.ui_scale, f, percent(f));
                        }
                    })
                    .response
                    .labelled_by(label);
            });
            row(
                ui,
                "Editor font size",
                "Also Ctrl+= / Ctrl+-, or Ctrl+wheel over the editor.",
                |ui, label| {
                    ui.add(egui::Slider::new(&mut s.editor_font_size, 9.0..=28.0).step_by(1.0))
                        .labelled_by(label);
                },
            );
        }
        Section::Editor => {
            row(ui, "Completion", "Ctrl+Space always opens it.", |ui, _| {
                ui.checkbox(&mut s.autocomplete, "Complete while typing");
            });
            row(ui, "Format SQL", "", |ui, _| {
                ui.checkbox(&mut s.format_uppercase, "Uppercase keywords");
                ui.horizontal(|ui| {
                    ui.label("Indent");
                    for n in [2u8, 4] {
                        ui.selectable_value(&mut s.format_indent, n, format!("{n} spaces"));
                    }
                });
            });
        }
        Section::Tabs => {
            row(
                ui,
                "Closing tabs",
                "Off: a tab with unsaved changes closes without asking and the changes are \
                 lost. Tabs with an open transaction always ask.",
                |ui, _| {
                    ui.checkbox(
                        &mut s.confirm_close_tab,
                        "Ask before closing a tab with unsaved changes",
                    );
                },
            );
        }
        Section::Queries => {
            row(
                ui,
                "Row limit",
                "Most rows fetched per result set; the service may cap it lower.",
                |ui, label| {
                    egui::ComboBox::from_id_salt("max_rows")
                        .selected_text(format!("{} rows", s.max_rows))
                        .show_ui(ui, |ui| {
                            for n in ROW_LIMITS {
                                ui.selectable_value(&mut s.max_rows, n, format!("{n} rows"));
                            }
                        })
                        .response
                        .labelled_by(label);
                },
            );
        }
        Section::Service => {
            row(
                ui,
                "Local service",
                "For a service set up with “Use a local service”.",
                |ui, _| {
                    ui.checkbox(
                        &mut s.autostart_local,
                        "Start the local service when sqail starts",
                    );
                },
            );
        }
        Section::Assistant => {
            let a = &mut s.assistant;
            row(ui, "Provider", "", |ui, label| {
                egui::ComboBox::from_id_salt("provider")
                    .selected_text(a.provider.label())
                    .show_ui(ui, |ui| {
                        for p in Provider::ALL {
                            ui.selectable_value(&mut a.provider, p, p.label());
                        }
                    })
                    .response
                    .labelled_by(label);
            });
            row(
                ui,
                "Claude Code model",
                "Empty: the CLI's default.",
                |ui, label| {
                    optional_text(ui, label, &mut a.claude_model, "e.g. sonnet");
                },
            );
            row(
                ui,
                "Grok model",
                "Empty: the CLI's default.",
                |ui, label| {
                    optional_text(ui, label, &mut a.grok_model, "");
                },
            );
            row(
                ui,
                "claude program",
                "Empty: found on PATH.",
                |ui, label| {
                    optional_path(ui, label, &mut a.claude_path);
                },
            );
            row(ui, "grok program", "Empty: found on PATH.", |ui, label| {
                optional_path(ui, label, &mut a.grok_path);
            });
            row(
                ui,
                "Rows shown to the model",
                "Most rows of one query result the assistant sees.",
                |ui, label| {
                    ui.add(egui::DragValue::new(&mut a.max_rows).range(1..=10_000))
                        .labelled_by(label);
                },
            );
            row(ui, "Context", "", |ui, _| {
                ui.checkbox(
                    &mut a.include_editor,
                    "Send the active tab's SQL with each question",
                );
            });
        }
    }
}

fn percent(f: f32) -> String {
    format!("{:.0} %", f * 100.0)
}

/// A text field for an optional string; empty means `None`.
fn optional_text(ui: &mut egui::Ui, label: Id, value: &mut Option<String>, hint: &str) {
    let mut text = value.clone().unwrap_or_default();
    if ui
        .add(
            egui::TextEdit::singleline(&mut text)
                .hint_text(hint)
                .desired_width(260.0),
        )
        .labelled_by(label)
        .changed()
    {
        let t = text.trim();
        *value = (!t.is_empty()).then(|| t.to_string());
    }
}

fn optional_path(ui: &mut egui::Ui, label: Id, value: &mut Option<PathBuf>) {
    let mut text = value
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    if ui
        .add(egui::TextEdit::singleline(&mut text).desired_width(260.0))
        .labelled_by(label)
        .changed()
    {
        let t = text.trim();
        *value = (!t.is_empty()).then(|| PathBuf::from(t));
    }
}
