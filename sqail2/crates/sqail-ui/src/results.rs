//! Query results held by a tab: compact cells, result sets, messages, and the
//! state of the run that produced them.

use std::cmp::Ordering;
use std::time::{Duration, Instant};

use serde_json::Value;
use sqail_client::proto::{Column, LogicalType, QueryEvent};
use uuid::Uuid;

/// One cell, ~24 bytes. A million rows × a few columns stays manageable.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(Box<str>),
}

impl Cell {
    pub fn from_json(v: Value) -> Self {
        match v {
            Value::Null => Cell::Null,
            Value::Bool(b) => Cell::Bool(b),
            Value::Number(n) => match n.as_i64() {
                Some(i) => Cell::Int(i),
                None => Cell::Float(n.as_f64().unwrap_or(f64::NAN)),
            },
            Value::String(s) => Cell::Text(s.into_boxed_str()),
            other => Cell::Text(other.to_string().into_boxed_str()),
        }
    }

    /// Text shown in the grid and copied to the clipboard.
    pub fn display(&self, logical: LogicalType) -> std::borrow::Cow<'_, str> {
        use std::borrow::Cow;
        match self {
            Cell::Null => Cow::Borrowed("NULL"),
            Cell::Bool(b) => Cow::Borrowed(if *b { "true" } else { "false" }),
            Cell::Int(i) => Cow::Owned(i.to_string()),
            Cell::Float(f) => Cow::Owned(f.to_string()),
            Cell::Text(s) if logical == LogicalType::Bytes => Cow::Owned(format!("0x{s}")),
            Cell::Text(s) => Cow::Borrowed(s),
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Cell::Null)
    }

    fn cmp(&self, other: &Cell) -> Ordering {
        use Cell::*;
        match (self, other) {
            (Null, Null) => Ordering::Equal,
            (Null, _) => Ordering::Less,
            (_, Null) => Ordering::Greater,
            (Bool(a), Bool(b)) => a.cmp(b),
            (Int(a), Int(b)) => a.cmp(b),
            (Float(a), Float(b)) => a.total_cmp(b),
            (Int(a), Float(b)) => (*a as f64).total_cmp(b),
            (Float(a), Int(b)) => a.total_cmp(&(*b as f64)),
            (Text(a), Text(b)) => match (a.parse::<f64>(), b.parse::<f64>()) {
                // Decimals travel as strings; compare them numerically.
                (Ok(x), Ok(y)) => x.total_cmp(&y),
                _ => a.cmp(b),
            },
            (a, b) => a.rank().cmp(&b.rank()),
        }
    }

    fn rank(&self) -> u8 {
        match self {
            Cell::Null => 0,
            Cell::Bool(_) => 1,
            Cell::Int(_) | Cell::Float(_) => 2,
            Cell::Text(_) => 3,
        }
    }
}

pub struct ResultSet {
    pub columns: Vec<Column>,
    pub rows: Vec<Box<[Cell]>>,
    pub complete: bool,
    pub truncated: bool,
    /// Sorted view: `order[i]` is the row shown at position `i`.
    pub order: Option<Vec<u32>>,
    pub sort: Option<(usize, bool)>,
}

impl ResultSet {
    /// A complete result set from ready-made rows (tests, fixtures).
    pub fn for_test(columns: Vec<Column>, rows: Vec<Vec<Cell>>) -> Self {
        let mut rs = Self::new(columns);
        rs.rows = rows.into_iter().map(Vec::into_boxed_slice).collect();
        rs.complete = true;
        rs
    }

    fn new(columns: Vec<Column>) -> Self {
        Self {
            columns,
            rows: Vec::new(),
            complete: false,
            truncated: false,
            order: None,
            sort: None,
        }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Index into `rows` of the row displayed at `pos`.
    pub fn row_index(&self, pos: usize) -> usize {
        self.order.as_ref().map_or(pos, |o| o[pos] as usize)
    }

    /// The row displayed at `pos`, honouring the sort.
    pub fn row(&self, pos: usize) -> &[Cell] {
        let idx = self.order.as_ref().map_or(pos, |o| o[pos] as usize);
        &self.rows[idx]
    }

    /// Cycle ascending → descending → unsorted for column `col`.
    pub fn toggle_sort(&mut self, col: usize) {
        self.sort = match self.sort {
            Some((c, true)) if c == col => Some((col, false)),
            Some((c, false)) if c == col => None,
            _ => Some((col, true)),
        };
        self.order = self.sort.map(|(c, asc)| {
            let mut order: Vec<u32> = (0..self.rows.len() as u32).collect();
            order.sort_by(|&a, &b| {
                let o = self.rows[a as usize][c].cmp(&self.rows[b as usize][c]);
                if asc { o } else { o.reverse() }
            });
            order
        });
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

pub struct Message {
    pub severity: Severity,
    pub text: String,
}

/// Which pane of the results area is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Result(usize),
    Messages,
    Plan,
}

/// Selected cells: an anchor and a focus corner (display positions).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Selection {
    pub anchor: (usize, usize),
    pub focus: (usize, usize),
}

impl Selection {
    pub fn rows(&self) -> std::ops::RangeInclusive<usize> {
        self.anchor.0.min(self.focus.0)..=self.anchor.0.max(self.focus.0)
    }
    pub fn cols(&self) -> std::ops::RangeInclusive<usize> {
        self.anchor.1.min(self.focus.1)..=self.anchor.1.max(self.focus.1)
    }
    pub fn contains(&self, row: usize, col: usize) -> bool {
        self.rows().contains(&row) && self.cols().contains(&col)
    }
}

/// One execution of SQL from a tab.
pub struct Run {
    pub id: u64,
    pub query_id: Option<Uuid>,
    pub sql: String,
    pub started: Instant,
    pub elapsed: Option<Duration>,
    pub results: Vec<ResultSet>,
    pub messages: Vec<Message>,
    pub running: bool,
    pub cancelled: bool,
    pub failed: bool,
    pub in_transaction: Option<bool>,
    pub pane: Pane,
    pub selection: Option<Selection>,
    /// Increments when results are reset so grid widget state (widths) resets.
    pub generation: u64,
    /// Inline editing of result set `.0`.
    pub edit: Option<(usize, crate::editing::EditState)>,
    /// Why editing is unavailable, or that it is being prepared.
    pub edit_status: Option<String>,
    /// Set for EXPLAIN runs.
    pub plan: Option<sqail_client::proto::Plan>,
}

impl Run {
    pub fn new(id: u64, sql: String) -> Self {
        Self {
            id,
            query_id: None,
            sql,
            started: Instant::now(),
            elapsed: None,
            results: Vec::new(),
            messages: Vec::new(),
            running: true,
            cancelled: false,
            failed: false,
            in_transaction: None,
            pane: Pane::Messages,
            selection: None,
            generation: id,
            edit: None,
            edit_status: None,
            plan: None,
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.elapsed.unwrap_or_else(|| self.started.elapsed())
    }

    pub fn total_rows(&self) -> usize {
        self.results.iter().map(ResultSet::len).sum()
    }

    fn msg(&mut self, severity: Severity, text: impl Into<String>) {
        self.messages.push(Message {
            severity,
            text: text.into(),
        });
    }

    /// An EXPLAIN finished.
    pub fn set_plan(&mut self, plan: sqail_client::proto::Plan) {
        self.plan = Some(plan);
        self.pane = Pane::Plan;
        self.finish(None);
        self.msg(
            Severity::Info,
            format!("Plan ready in {}", fmt_duration(self.elapsed())),
        );
    }

    /// Transport-level failure (the stream could not start or broke).
    pub fn fail(&mut self, error: String) {
        self.msg(Severity::Error, error);
        self.failed = true;
        self.finish(None);
        self.pane = Pane::Messages;
    }

    fn finish(&mut self, elapsed: Option<Duration>) {
        self.running = false;
        self.elapsed = Some(elapsed.unwrap_or_else(|| self.started.elapsed()));
        for r in &mut self.results {
            r.complete = true;
        }
    }

    pub fn apply(&mut self, ev: QueryEvent) {
        match ev {
            QueryEvent::Started { query_id } => self.query_id = Some(query_id),
            QueryEvent::ResultStart { columns, .. } => {
                self.results.push(ResultSet::new(columns));
                if self.results.len() == 1 {
                    self.pane = Pane::Result(0);
                }
            }
            QueryEvent::Rows { index, rows } => {
                if let Some(rs) = self.results.get_mut(index as usize) {
                    rs.rows.reserve(rows.len());
                    rs.rows.extend(
                        rows.into_iter()
                            .map(|r| r.into_iter().map(Cell::from_json).collect::<Box<[Cell]>>()),
                    );
                }
            }
            QueryEvent::ResultEnd {
                index,
                row_count,
                truncated,
            } => {
                if let Some(rs) = self.results.get_mut(index as usize) {
                    rs.complete = true;
                    rs.truncated = truncated;
                }
                let note = if truncated {
                    " (truncated: row limit reached)"
                } else {
                    ""
                };
                self.msg(
                    Severity::Info,
                    format!(
                        "Result {}: {row_count} row{}{note}",
                        index + 1,
                        plural(row_count)
                    ),
                );
            }
            QueryEvent::RowsAffected { count } => {
                self.msg(
                    Severity::Info,
                    format!("{count} row{} affected", plural(count)),
                );
            }
            QueryEvent::Message { severity, text } => {
                let sev = match severity.as_str() {
                    "warning" => Severity::Warning,
                    "error" => Severity::Error,
                    _ => Severity::Info,
                };
                self.msg(sev, text);
            }
            QueryEvent::Error { message, .. } => {
                self.failed = true;
                self.msg(Severity::Error, message);
                self.pane = Pane::Messages;
            }
            QueryEvent::Done {
                elapsed_ms,
                cancelled,
                in_transaction,
            } => {
                self.cancelled = cancelled;
                self.in_transaction = in_transaction;
                self.finish(Some(Duration::from_millis(elapsed_ms)));
                let what = if cancelled {
                    "Cancelled"
                } else if self.failed {
                    "Failed"
                } else {
                    "Completed"
                };
                self.msg(
                    Severity::Info,
                    format!("{what} in {}", fmt_duration(self.elapsed())),
                );
                if self.results.is_empty() || self.failed {
                    self.pane = Pane::Messages;
                }
            }
        }
    }
}

fn plural(n: u64) -> &'static str {
    if n == 1 { "" } else { "s" }
}

pub fn fmt_duration(d: Duration) -> String {
    let ms = d.as_millis();
    if ms < 1000 {
        format!("{ms} ms")
    } else if ms < 60_000 {
        format!("{:.2} s", d.as_secs_f64())
    } else {
        format!("{}m {:02}s", ms / 60_000, (ms / 1000) % 60)
    }
}

/// Tab-separated text of the selected cells (optionally with a header row).
pub fn selection_tsv(rs: &ResultSet, sel: &Selection, headers: bool) -> String {
    let cols: Vec<usize> = sel.cols().filter(|&c| c < rs.columns.len()).collect();
    let mut out = String::new();
    if headers {
        let names: Vec<&str> = cols.iter().map(|&c| rs.columns[c].name.as_str()).collect();
        out.push_str(&names.join("\t"));
        out.push('\n');
    }
    for r in sel.rows().filter(|&r| r < rs.len()) {
        let row = rs.row(r);
        let cells: Vec<String> = cols
            .iter()
            .map(|&c| {
                if row[c].is_null() {
                    String::new()
                } else {
                    row[c]
                        .display(rs.columns[c].logical)
                        .replace(['\t', '\n', '\r'], " ")
                }
            })
            .collect();
        out.push_str(&cells.join("\t"));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn col(name: &str, logical: LogicalType) -> Column {
        Column {
            name: name.into(),
            type_name: "t".into(),
            logical,
        }
    }

    fn run_with(rows: Vec<Vec<Value>>) -> Run {
        let mut run = Run::new(1, "q".into());
        run.apply(QueryEvent::ResultStart {
            index: 0,
            columns: vec![col("n", LogicalType::Int), col("s", LogicalType::Text)],
        });
        run.apply(QueryEvent::Rows { index: 0, rows });
        run.apply(QueryEvent::ResultEnd {
            index: 0,
            row_count: 3,
            truncated: false,
        });
        run.apply(QueryEvent::Done {
            elapsed_ms: 5,
            cancelled: false,
            in_transaction: None,
        });
        run
    }

    #[test]
    fn events_build_results() {
        let run = run_with(vec![
            vec![json!(2), json!("b")],
            vec![json!(1), Value::Null],
            vec![json!(3), json!("a")],
        ]);
        assert!(!run.running);
        assert_eq!(run.pane, Pane::Result(0));
        assert_eq!(run.results[0].len(), 3);
        assert!(run.messages.iter().any(|m| m.text == "Result 1: 3 rows"));
    }

    #[test]
    fn sorting_cycles_and_nulls_first() {
        let mut run = run_with(vec![
            vec![json!(2), json!("b")],
            vec![json!(1), Value::Null],
            vec![json!(3), json!("a")],
        ]);
        let rs = &mut run.results[0];
        rs.toggle_sort(1);
        assert_eq!(rs.row(0)[1], Cell::Null);
        assert_eq!(rs.row(2)[1], Cell::Text("b".into()));
        rs.toggle_sort(1);
        assert_eq!(rs.row(0)[1], Cell::Text("b".into()));
        rs.toggle_sort(1);
        assert!(rs.order.is_none());
    }

    #[test]
    fn decimals_sort_numerically() {
        assert_eq!(
            Cell::Text("10.5".into()).cmp(&Cell::Text("9.1".into())),
            Ordering::Greater
        );
    }

    #[test]
    fn tsv_copy() {
        let run = run_with(vec![
            vec![json!(1), json!("a\tb")],
            vec![json!(2), Value::Null],
        ]);
        let sel = Selection {
            anchor: (0, 0),
            focus: (1, 1),
        };
        assert_eq!(
            selection_tsv(&run.results[0], &sel, true),
            "n\ts\n1\ta b\n2\t\n"
        );
    }
}
