//! The results area: result-set tabs, the virtualised grid, and messages.

use egui::{Align, Color32, Layout, RichText, Sense};
use egui_extras::{Column, TableBuilder};
use sqail_client::proto::LogicalType;

use crate::editing::EditState;
use crate::results::{Pane, ResultSet, Run, Selection, Severity, fmt_duration, selection_tsv};

#[derive(Default)]
pub struct GridOutput {
    pub cancel: bool,
    /// Open the value viewer with (title, full text).
    pub view: Option<(String, String)>,
    /// Export: format, and whether to re-run the whole query (vs. the rows
    /// already loaded in the shown result).
    pub export: Option<(crate::transfer::Format, bool)>,
    /// Start editing the shown result.
    pub start_edit: bool,
    /// Open the review dialog for pending edits.
    pub review_edits: bool,
}

/// Longest text we lay out in a cell; the viewer shows the rest.
const CELL_PREVIEW: usize = 256;

/// `editable`: `Ok` if the shown result may be edited, else the reason.
pub fn results_ui(ui: &mut egui::Ui, run: &mut Run, editable: Result<(), String>) -> GridOutput {
    let mut out = GridOutput::default();

    ui.horizontal(|ui| {
        for (i, rs) in run.results.iter().enumerate() {
            let resp = if rs.truncated {
                // A capped result must not pass for the whole answer.
                let label = RichText::new(format!("Result {} ({}+)", i + 1, fmt_count(rs.len())))
                    .color(ui.visuals().warn_fg_color);
                ui.selectable_label(run.pane == Pane::Result(i), label)
                    .on_hover_text("Row limit reached: more rows exist. Raise it under Query → Row limit, or export the whole query.")
            } else {
                let label = format!("Result {} ({})", i + 1, fmt_count(rs.len()));
                ui.selectable_label(run.pane == Pane::Result(i), label)
            };
            if resp.clicked() {
                run.pane = Pane::Result(i);
                run.selection = None;
            }
        }
        if run.plan.is_some()
            && ui
                .selectable_label(run.pane == Pane::Plan, "Plan")
                .clicked()
        {
            run.pane = Pane::Plan;
        }
        let errors = run
            .messages
            .iter()
            .filter(|m| m.severity == Severity::Error)
            .count();
        let msg_label = if errors > 0 {
            RichText::new(format!("Messages ({})", run.messages.len()))
                .color(Color32::from_rgb(0xd6, 0x45, 0x45))
        } else {
            RichText::new(format!("Messages ({})", run.messages.len()))
        };
        if ui
            .selectable_label(run.pane == Pane::Messages, msg_label)
            .clicked()
        {
            run.pane = Pane::Messages;
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if !run.running && !run.results.is_empty() {
                ui.menu_button("Export", |ui| {
                    for f in crate::transfer::Format::ALL {
                        if ui
                            .button(format!("Shown result as {}…", f.label()))
                            .clicked()
                        {
                            out.export = Some((f, false));
                            ui.close();
                        }
                    }
                    ui.separator();
                    ui.label(
                        RichText::new("Re-run the query and stream every row:")
                            .weak()
                            .small(),
                    );
                    for f in crate::transfer::Format::ALL {
                        if ui
                            .button(format!("Whole query to {}…", f.label()))
                            .clicked()
                        {
                            out.export = Some((f, true));
                            ui.close();
                        }
                    }
                });
            }
            if run.running {
                if ui.button("■ Stop").clicked() {
                    out.cancel = true;
                }
                ui.label(format!(
                    "Running… {} · {} rows",
                    fmt_duration(run.elapsed()),
                    fmt_count(run.total_rows())
                ));
                ui.spinner();
            } else {
                let status = if run.cancelled {
                    "Cancelled"
                } else if run.failed {
                    "Failed"
                } else {
                    "Done"
                };
                ui.label(
                    RichText::new(format!(
                        "{status} · {} rows · {}",
                        fmt_count(run.total_rows()),
                        fmt_duration(run.elapsed())
                    ))
                    .weak(),
                );
            }
        });
    });
    ui.separator();

    if let Pane::Result(i) = run.pane
        && !run.running
    {
        edit_toolbar(ui, run, i, editable, &mut out);
    }

    match run.pane {
        Pane::Messages => messages_ui(ui, run),
        Pane::Plan => match &run.plan {
            Some(plan) => plan_ui(ui, plan),
            None if run.running => {
                ui.spinner();
            }
            None => messages_ui(ui, run),
        },
        Pane::Result(i) if i < run.results.len() => {
            let generation = run.generation;
            let Run {
                results,
                selection,
                edit,
                ..
            } = run;
            let edit = edit.as_mut().filter(|(ri, _)| *ri == i).map(|(_, e)| e);
            grid_ui(
                ui,
                &mut results[i],
                selection,
                edit,
                (generation, i),
                &mut out,
            );
        }
        Pane::Result(_) => {}
    }
    out
}

fn edit_toolbar(
    ui: &mut egui::Ui,
    run: &mut Run,
    i: usize,
    editable: Result<(), String>,
    out: &mut GridOutput,
) {
    let editing = run.edit.as_ref().is_some_and(|(ri, _)| *ri == i);
    ui.horizontal(|ui| {
        if !editing {
            let resp = ui.add_enabled(editable.is_ok(), egui::Button::new("✎ Edit data"));
            let resp = match &editable {
                Ok(()) => resp.on_hover_text("Edit, add and delete rows of this table"),
                Err(why) => resp.on_disabled_hover_text(why.as_str()),
            };
            if resp.clicked() {
                out.start_edit = true;
            }
            if let Some(s) = &run.edit_status {
                ui.label(RichText::new(s).weak());
            }
            return;
        }
        let ncols = run.results[i].columns.len();
        let Some((_, e)) = run.edit.as_mut() else {
            return;
        };
        let pending = e.pending();
        ui.label(RichText::new(format!("Editing {}", e.table)).strong());
        if ui.button("+ Row").clicked() {
            e.inserted.push(vec![None; ncols]);
        }
        if ui
            .add_enabled(run.selection.is_some(), egui::Button::new("Delete rows"))
            .on_hover_text("Delete the selected rows")
            .clicked()
            && let Some(sel) = run.selection
        {
            let rs = &run.results[i];
            for pos in sel.rows().filter(|&p| p < rs.len()) {
                e.deleted.insert(rs.row_index(pos));
            }
        }
        if ui
            .add_enabled(pending > 0, egui::Button::new("Revert"))
            .clicked()
        {
            e.changes.clear();
            e.deleted.clear();
            e.inserted.clear();
            e.editing = None;
        }
        if ui
            .add_enabled(
                pending > 0,
                egui::Button::new(RichText::new(format!("Review & apply ({pending})")).strong()),
            )
            .clicked()
        {
            out.review_edits = true;
        }
        if ui
            .add_enabled(pending == 0, egui::Button::new("Done"))
            .on_disabled_hover_text("Apply or revert the pending changes first")
            .clicked()
        {
            run.edit = None;
        }
        ui.label(
            RichText::new("Double-click or F2 to edit a cell · right-click for NULL / delete")
                .weak()
                .small(),
        );
    });
}

fn fmt_count(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn plan_ui(ui: &mut egui::Ui, plan: &sqail_client::proto::Plan) {
    let raw_id = ui.id().with("plan_raw");
    let mut raw = ui
        .ctx()
        .data(|d| d.get_temp::<bool>(raw_id).unwrap_or(false));
    ui.horizontal(|ui| {
        if plan.analyzed {
            ui.label(RichText::new("Actual plan (the statement ran and was rolled back)").weak());
        } else {
            ui.label(RichText::new("Estimated plan").weak());
        }
        if ui.checkbox(&mut raw, "Raw").changed() {
            ui.ctx().data_mut(|d| d.insert_temp(raw_id, raw));
        }
    });
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if raw {
                ui.add(egui::Label::new(RichText::new(&plan.raw).monospace()).selectable(true));
                return;
            }
            for (i, root) in plan.roots.iter().enumerate() {
                let total = root.cost.filter(|c| *c > 0.0);
                plan_node(ui, root, total, &mut vec![i]);
            }
        });
}

/// Cost spent in this operator alone (its subtree minus its children).
fn self_cost(n: &sqail_client::proto::PlanNode) -> Option<f64> {
    let own = n.cost?;
    let children: f64 = n.children.iter().filter_map(|c| c.cost).sum();
    Some((own - children).max(0.0))
}

fn fmt_num(v: f64) -> String {
    if v >= 1000.0 {
        fmt_count(v.round() as usize)
    } else if v.fract() == 0.0 {
        format!("{v}")
    } else {
        format!("{v:.2}")
    }
}

fn plan_node(
    ui: &mut egui::Ui,
    n: &sqail_client::proto::PlanNode,
    total: Option<f64>,
    path: &mut Vec<usize>,
) {
    let share = match (self_cost(n), total) {
        (Some(c), Some(t)) => Some(c / t),
        _ => None,
    };
    let hot = share.map_or(Color32::TRANSPARENT, |s| {
        if s > 0.4 {
            Color32::from_rgb(0xd6, 0x45, 0x45)
        } else if s > 0.15 {
            Color32::from_rgb(0xe6, 0xa2, 0x3c)
        } else {
            ui.visuals().weak_text_color()
        }
    });
    let mut header = RichText::new(&n.label).strong();
    if share.is_some_and(|s| s > 0.15) {
        header = header.color(hot);
    }
    let mut stats = Vec::new();
    if let Some(o) = &n.object {
        stats.push(o.clone());
    }
    if let Some(c) = n.cost {
        stats.push(match share {
            Some(s) => format!("cost {} ({:.0}% here)", fmt_num(c), s * 100.0),
            None => format!("cost {}", fmt_num(c)),
        });
    }
    if let Some(r) = n.rows {
        stats.push(format!("est. {} rows", fmt_num(r)));
    }
    if let Some(r) = n.actual_rows {
        stats.push(format!("actual {} rows", fmt_num(r)));
    }
    if let Some(ms) = n.actual_ms {
        stats.push(format!("{ms:.2} ms"));
    }
    let hover = n
        .props
        .iter()
        .map(|p| format!("{}: {}", p.key, p.value))
        .collect::<Vec<_>>()
        .join("\n");
    let line = |ui: &mut egui::Ui| {
        ui.horizontal(|ui| {
            if let Some(s) = share {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(40.0, 8.0), Sense::hover());
                ui.painter()
                    .rect_filled(rect, 2.0, ui.visuals().faint_bg_color);
                let mut fill = rect;
                fill.set_width(rect.width() * s.clamp(0.0, 1.0) as f32);
                ui.painter().rect_filled(fill, 2.0, hot);
            }
            ui.label(header.clone());
            ui.label(RichText::new(stats.join(" · ")).weak());
        })
        .response
    };
    if n.children.is_empty() {
        let r = ui
            .indent(egui::Id::new(("plan", path.clone())), |ui| line(ui))
            .inner;
        if !hover.is_empty() {
            r.on_hover_text(&hover);
        }
        return;
    }
    let resp = egui::CollapsingHeader::new(
        RichText::new(format!("{}  {}", n.label, stats.join(" · "))).color(
            if share.is_some_and(|s| s > 0.15) {
                hot
            } else {
                ui.visuals().text_color()
            },
        ),
    )
    .id_salt(("plan", path.clone()))
    .default_open(true)
    .show(ui, |ui| {
        for (i, c) in n.children.iter().enumerate() {
            path.push(i);
            plan_node(ui, c, total, path);
            path.pop();
        }
    });
    if !hover.is_empty() {
        resp.header_response.on_hover_text(hover);
    }
}

fn messages_ui(ui: &mut egui::Ui, run: &Run) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            ui.label(RichText::new(&run.sql).monospace().weak())
                .on_hover_text("The SQL that ran");
            ui.add_space(4.0);
            for m in &run.messages {
                let color = match m.severity {
                    Severity::Error => Color32::from_rgb(0xd6, 0x45, 0x45),
                    Severity::Warning => Color32::from_rgb(0xe6, 0xa2, 0x3c),
                    Severity::Info => ui.visuals().text_color(),
                };
                ui.add(
                    egui::Label::new(RichText::new(&m.text).monospace().color(color))
                        .selectable(true),
                );
            }
        });
}

fn is_numeric(l: LogicalType) -> bool {
    matches!(
        l,
        LogicalType::Int | LogicalType::Float | LogicalType::Decimal
    )
}

fn preview(s: &str) -> String {
    let line = s.lines().next().unwrap_or("");
    let mut p: String = line.chars().take(CELL_PREVIEW).collect();
    if p.len() < s.len() {
        p.push('…');
    }
    p
}

/// Right-click actions on a cell.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CellAction {
    Copy,
    CopyHeaders,
    CopyRow,
    View,
    SetNull,
    SetDefault,
    DeleteRow,
    RestoreRow,
}

/// What a grid position shows while editing: an original row (by index into
/// `rs.rows`) or a new row (by index into `inserted`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum RowRef {
    Existing(usize),
    New(usize),
}

fn row_ref(rs: &ResultSet, pos: usize) -> RowRef {
    if pos < rs.len() {
        RowRef::Existing(rs.row_index(pos))
    } else {
        RowRef::New(pos - rs.len())
    }
}

/// The text a cell shows, honouring pending edits. `None` = NULL.
fn cell_text(
    rs: &ResultSet,
    edit: Option<&EditState>,
    pos: usize,
    c: usize,
) -> (Option<String>, bool) {
    let logical = rs.columns[c].logical;
    match (row_ref(rs, pos), edit) {
        (RowRef::Existing(r), Some(e)) if e.changes.get(&r).is_some_and(|m| m.contains_key(&c)) => {
            (e.changes[&r][&c].clone(), true)
        }
        (RowRef::Existing(r), _) => {
            let cell = &rs.rows[r][c];
            (
                (!cell.is_null()).then(|| cell.display(logical).into_owned()),
                false,
            )
        }
        (RowRef::New(i), Some(e)) => match &e.inserted[i][c] {
            Some(v) => (v.clone(), true),
            None => (Some("DEFAULT".into()), false),
        },
        (RowRef::New(_), None) => (None, false),
    }
}

fn grid_ui(
    ui: &mut egui::Ui,
    rs: &mut ResultSet,
    selection: &mut Option<Selection>,
    mut edit: Option<&mut EditState>,
    salt: (u64, usize),
    out: &mut GridOutput,
) {
    if rs.columns.is_empty() {
        ui.label(RichText::new("(no columns)").weak());
        return;
    }
    let mono = egui::TextStyle::Monospace.resolve(ui.style());
    let char_w = ui.fonts_mut(|f| f.glyph_width(&mono, '0'));
    let row_h = ui.fonts_mut(|f| f.row_height(&mono)) + 6.0;

    // Initial widths from the header and the first rows.
    let widths: Vec<f32> = rs
        .columns
        .iter()
        .enumerate()
        .map(|(c, col)| {
            let mut chars = col.name.chars().count() + 2;
            for r in rs.rows.iter().take(50) {
                chars = chars.max(r[c].display(col.logical).chars().count().min(48));
            }
            (chars as f32 * char_w + 20.0).clamp(56.0, 420.0)
        })
        .collect();
    let total_rows = rs.len() + edit.as_ref().map_or(0, |e| e.inserted.len());
    let num_w = (total_rows.max(1).to_string().len() as f32) * char_w + 16.0;

    // Keyboard copy when the grid has the last click (not while typing in a cell).
    let grid_id = ui.id().with(("grid", salt));
    let grid_active = ui
        .ctx()
        .memory(|m| m.data.get_temp::<bool>(grid_id).unwrap_or(false));
    let typing = edit.as_ref().is_some_and(|e| e.editing.is_some());
    if grid_active
        && !typing
        && let Some(sel) = selection.as_ref()
        && ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy)))
    {
        ui.ctx().copy_text(selection_tsv(rs, sel, false));
    }

    let mut clicked_header = None;
    let mut clicked_cell: Option<(usize, usize, bool)> = None;
    let mut dbl_cell = None;
    // F2 edits the selected cell, like a spreadsheet.
    if grid_active
        && !typing
        && edit.is_some()
        && let Some(sel) = selection.as_ref()
        && ui.input(|i| i.key_pressed(egui::Key::F2))
    {
        dbl_cell = Some(sel.focus);
    }
    let mut ctx_action: Option<(CellAction, usize, usize)> = None;
    // Inline editor result: Some(Some(text)) = commit, Some(None) = cancel.
    let mut finished_edit: Option<Option<String>> = None;
    let sel_fill = ui.visuals().selection.bg_fill;
    let null_color = ui.visuals().weak_text_color();
    let changed_fill = Color32::from_rgba_unmultiplied(0xe6, 0xa2, 0x3c, 50);
    let new_fill = Color32::from_rgba_unmultiplied(0x3c, 0xb3, 0x71, 40);
    let deleted_fill = Color32::from_rgba_unmultiplied(0xd6, 0x45, 0x45, 45);

    let mut table = TableBuilder::new(ui)
        .id_salt(salt)
        .striped(true)
        .resizable(true)
        .sense(Sense::click())
        .auto_shrink([false, false])
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(Column::exact(num_w));
    for w in &widths {
        table = table.column(
            Column::initial(*w)
                .at_least(40.0)
                .clip(true)
                .resizable(true),
        );
    }
    table
        .header(row_h, |mut header| {
            header.col(|ui| {
                ui.label(RichText::new("#").weak());
            });
            for (c, col) in rs.columns.iter().enumerate() {
                header.col(|ui| {
                    let arrow = match rs.sort {
                        Some((sc, true)) if sc == c => " ▲",
                        Some((sc, false)) if sc == c => " ▼",
                        _ => "",
                    };
                    let read_only = edit.as_ref().is_some_and(|e| !e.editable(c));
                    let mut text = RichText::new(format!("{}{arrow}", col.name)).strong();
                    if read_only {
                        text = text.weak();
                    }
                    let resp = ui
                        .add(egui::Label::new(text).sense(Sense::click()).truncate())
                        .on_hover_text(format!(
                            "{} · {}{}\nClick to sort",
                            col.name,
                            col.type_name,
                            if read_only { " · read-only (not a table column)" } else { "" }
                        ));
                    if resp.clicked() {
                        clicked_header = Some(c);
                    }
                });
            }
        })
        .body(|body| {
            body.rows(row_h, total_rows, |mut row| {
                let pos = row.index();
                let rref = row_ref(rs, pos);
                let deleted = matches!((rref, edit.as_deref()), (RowRef::Existing(r), Some(e)) if e.deleted.contains(&r));
                let is_new = matches!(rref, RowRef::New(_));
                row.col(|ui| {
                    let label = if is_new { "+".to_string() } else { (pos + 1).to_string() };
                    ui.label(RichText::new(label).weak().monospace());
                });
                for c in 0..rs.columns.len() {
                    let logical = rs.columns[c].logical;
                    let selected = selection.as_ref().is_some_and(|s| s.contains(pos, c));
                    let (value, changed) = cell_text(rs, edit.as_deref(), pos, c);
                    let editing_here = edit
                        .as_ref()
                        .and_then(|e| e.editing.as_ref())
                        .is_some_and(|(p, col, _)| *p == pos && *col == c);
                    let (_, resp) = row.col(|ui| {
                        let fill = if deleted {
                            Some(deleted_fill)
                        } else if selected {
                            Some(sel_fill)
                        } else if changed {
                            Some(changed_fill)
                        } else if is_new {
                            Some(new_fill)
                        } else {
                            None
                        };
                        if let Some(f) = fill {
                            ui.painter().rect_filled(ui.max_rect(), 0.0, f);
                        }
                        if editing_here && let Some(e) = edit.as_deref_mut() {
                            let focus = std::mem::take(&mut e.focus_editor);
                            let Some((_, _, buf)) = e.editing.as_mut() else { return };
                            let r = ui.add(egui::TextEdit::singleline(buf).font(mono.clone()).desired_width(f32::INFINITY));
                            if focus {
                                r.request_focus();
                            }
                            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                finished_edit = Some(None);
                            } else if r.lost_focus() || (r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                                finished_edit = Some(Some(buf.clone()));
                            }
                            return;
                        }
                        let text = match &value {
                            None => RichText::new("NULL").italics().color(null_color),
                            Some(v) if is_new && !changed => RichText::new(v).italics().color(null_color),
                            Some(v) => {
                                let t = RichText::new(preview(v)).monospace();
                                if deleted { t.strikethrough() } else { t }
                            }
                        };
                        if is_numeric(logical) && value.is_some() && !(is_new && !changed) {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.add(egui::Label::new(text).truncate().selectable(false));
                            });
                        } else {
                            ui.add(egui::Label::new(text).truncate().selectable(false));
                        }
                    });
                    if resp.clicked() {
                        let shift = resp.ctx.input(|i| i.modifiers.shift);
                        clicked_cell = Some((pos, c, shift));
                    }
                    if resp.double_clicked() {
                        dbl_cell = Some((pos, c));
                    }
                    let editable = edit.as_ref().is_some_and(|e| e.editable(c)) && !deleted;
                    let in_edit = edit.is_some();
                    resp.context_menu(|ui| {
                        let mut item = |ui: &mut egui::Ui, label: &str, a: CellAction| {
                            if ui.button(label).clicked() {
                                ctx_action = Some((a, pos, c));
                                ui.close();
                            }
                        };
                        item(ui, "Copy", CellAction::Copy);
                        item(ui, "Copy with headers", CellAction::CopyHeaders);
                        item(ui, "Copy row", CellAction::CopyRow);
                        item(ui, "View value", CellAction::View);
                        if in_edit {
                            ui.separator();
                            if editable {
                                item(ui, "Set NULL", CellAction::SetNull);
                            }
                            if is_new && editable {
                                item(ui, "Set DEFAULT", CellAction::SetDefault);
                            }
                            if deleted {
                                item(ui, "Restore row", CellAction::RestoreRow);
                            } else {
                                item(ui, "Delete row", CellAction::DeleteRow);
                            }
                        }
                    });
                }
            });
        });

    if let Some(c) = clicked_header
        && rs.complete
    {
        rs.toggle_sort(c);
        *selection = None;
    }
    if let Some((r, c, shift)) = clicked_cell {
        *selection = Some(match (selection.as_ref(), shift) {
            (Some(s), true) => Selection {
                anchor: s.anchor,
                focus: (r, c),
            },
            _ => Selection {
                anchor: (r, c),
                focus: (r, c),
            },
        });
        ui.ctx().memory_mut(|m| m.data.insert_temp(grid_id, true));
    }
    let view = |rs: &ResultSet, edit: Option<&EditState>, r: usize, c: usize| {
        let col = &rs.columns[c];
        (
            format!("{} (row {})", col.name, r + 1),
            cell_text(rs, edit, r, c).0.unwrap_or_else(|| "NULL".into()),
        )
    };

    // Finish an inline edit.
    if let (Some(done), Some(e)) = (finished_edit, edit.as_deref_mut())
        && let Some((pos, c, _)) = e.editing.take()
        && let Some(text) = done
    {
        match row_ref(rs, pos) {
            RowRef::Existing(r) => e.set(r, c, Some(text), rs),
            RowRef::New(i) => e.inserted[i][c] = Some(Some(text)),
        }
    }

    if let Some((pos, c)) = dbl_cell {
        match edit.as_deref_mut() {
            Some(e)
                if e.editable(c)
                    && !matches!(row_ref(rs, pos), RowRef::Existing(r) if e.deleted.contains(&r)) =>
            {
                let current = cell_text(rs, Some(e), pos, c).0;
                let current = match (row_ref(rs, pos), current) {
                    (RowRef::New(i), _) if e.inserted[i][c].is_none() => String::new(),
                    (_, v) => v.unwrap_or_default(),
                };
                e.editing = Some((pos, c, current));
                e.focus_editor = true;
            }
            _ => out.view = Some(view(rs, edit.as_deref(), pos, c)),
        }
    }
    if let Some((action, r, c)) = ctx_action {
        // Right-clicking outside the selection selects that cell first.
        let sel = match selection.as_ref() {
            Some(s) if s.contains(r, c) => *s,
            _ => Selection {
                anchor: (r, c),
                focus: (r, c),
            },
        };
        *selection = Some(sel);
        match action {
            CellAction::Copy => ui.ctx().copy_text(selection_tsv(rs, &sel, false)),
            CellAction::CopyHeaders => ui.ctx().copy_text(selection_tsv(rs, &sel, true)),
            CellAction::CopyRow => {
                let row_sel = Selection {
                    anchor: (sel.rows().start().to_owned(), 0),
                    focus: (*sel.rows().end(), rs.columns.len() - 1),
                };
                ui.ctx().copy_text(selection_tsv(rs, &row_sel, false));
            }
            CellAction::View => out.view = Some(view(rs, edit.as_deref(), r, c)),
            _ => {
                if let Some(e) = edit {
                    apply_cell_action(e, rs, action, &sel);
                }
            }
        }
    }
    if ui.input(|i| i.pointer.any_click()) && clicked_cell.is_none() && ctx_action.is_none() {
        // A click elsewhere hands keyboard copy back to the editor.
        ui.ctx().memory_mut(|m| m.data.insert_temp(grid_id, false));
    }
}

/// Edit actions apply to every selected row (and, for values, selected cell).
fn apply_cell_action(e: &mut EditState, rs: &ResultSet, action: CellAction, sel: &Selection) {
    for pos in sel.rows() {
        let rref = row_ref(rs, pos);
        match action {
            CellAction::DeleteRow => match rref {
                RowRef::Existing(r) => {
                    e.deleted.insert(r);
                }
                RowRef::New(_) => {}
            },
            CellAction::RestoreRow => {
                if let RowRef::Existing(r) = rref {
                    e.deleted.remove(&r);
                }
            }
            CellAction::SetNull | CellAction::SetDefault => {
                let cols: Vec<usize> = sel.cols().filter(|&c| e.editable(c)).collect();
                for c in cols {
                    match (rref, action) {
                        (RowRef::Existing(r), CellAction::SetNull) => e.set(r, c, None, rs),
                        (RowRef::New(i), CellAction::SetNull) => e.inserted[i][c] = Some(None),
                        (RowRef::New(i), CellAction::SetDefault) => e.inserted[i][c] = None,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    // New rows are removed outright (after the loop, highest first).
    if action == CellAction::DeleteRow {
        let mut new_rows: Vec<usize> = sel
            .rows()
            .filter_map(|p| match row_ref(rs, p) {
                RowRef::New(i) => Some(i),
                RowRef::Existing(_) => None,
            })
            .collect();
        new_rows.sort_unstable_by(|a, b| b.cmp(a));
        for i in new_rows {
            if i < e.inserted.len() {
                e.inserted.remove(i);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_have_separators() {
        assert_eq!(fmt_count(0), "0");
        assert_eq!(fmt_count(1234), "1,234");
        assert_eq!(fmt_count(1_000_000), "1,000,000");
    }

    #[test]
    fn previews_are_short_single_lines() {
        assert_eq!(preview("a\nb"), "a…");
        assert_eq!(preview(&"x".repeat(300)).chars().count(), CELL_PREVIEW + 1);
        assert_eq!(preview("ok"), "ok");
    }

    #[test]
    fn truncated_results_are_flagged_on_their_tab() {
        use egui_kittest::kittest::Queryable;
        use sqail_client::proto::{Column, LogicalType, QueryEvent};
        let mut run = Run::new(1, "q".into());
        run.apply(QueryEvent::ResultStart {
            index: 0,
            columns: vec![Column {
                name: "n".into(),
                type_name: "int".into(),
                logical: LogicalType::Int,
            }],
        });
        run.apply(QueryEvent::Rows {
            index: 0,
            rows: vec![vec![1.into()], vec![2.into()]],
        });
        run.apply(QueryEvent::ResultEnd {
            index: 0,
            row_count: 2,
            truncated: true,
        });
        run.apply(QueryEvent::Done {
            elapsed_ms: 1,
            cancelled: false,
            in_transaction: None,
        });
        let h = egui_kittest::Harness::new_ui_state(
            |ui, run: &mut Run| {
                results_ui(ui, run, Err("test".into()));
            },
            run,
        );
        h.get_by_label("Result 1 (2+)");
    }
}
