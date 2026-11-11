//! Query results held by a tab: compact cells, result sets, messages, and the
//! state of the run that produced them.

use std::cmp::Ordering;
use std::time::{Duration, Instant};

use serde_json::Value;
use sqail_client::proto::{Column, LogicalType, QueryEvent};
use uuid::Uuid;

/// Text of a cell. Up to [`CellStr::INLINE`] bytes (dates, decimals, codes,
/// short names: most values) are stored inside the cell, so a result of a
/// million rows doesn't make millions of small allocations.
#[derive(Clone)]
pub enum CellStr {
    Inline { len: u8, buf: [u8; CellStr::INLINE] },
    Heap(Box<str>),
}

impl CellStr {
    pub const INLINE: usize = 22;

    pub fn as_str(&self) -> &str {
        match self {
            // Only ever filled from a &str, so this cannot fail.
            CellStr::Inline { len, buf } => {
                std::str::from_utf8(&buf[..*len as usize]).unwrap_or("")
            }
            CellStr::Heap(s) => s,
        }
    }
}

impl From<&str> for CellStr {
    fn from(s: &str) -> Self {
        if s.len() <= Self::INLINE {
            let mut buf = [0; Self::INLINE];
            buf[..s.len()].copy_from_slice(s.as_bytes());
            CellStr::Inline {
                len: s.len() as u8,
                buf,
            }
        } else {
            CellStr::Heap(s.into())
        }
    }
}

impl From<String> for CellStr {
    fn from(s: String) -> Self {
        if s.len() <= Self::INLINE {
            s.as_str().into()
        } else {
            CellStr::Heap(s.into_boxed_str())
        }
    }
}

impl std::ops::Deref for CellStr {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq for CellStr {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl std::fmt::Debug for CellStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_str().fmt(f)
    }
}

impl std::fmt::Display for CellStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_str().fmt(f)
    }
}

/// One cell, 24 bytes. A million rows × a few columns stays manageable.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(CellStr),
}

/// A cell prepared for sorting: text that looks like a number (decimals
/// travel as strings) is parsed once, not on every comparison.
///
/// The comparison must be a total order (the standard sort may panic
/// otherwise), so values sort in fixed groups: NULL, booleans, numbers, then
/// text. Within text, values that parse as a finite number come first in
/// numeric order and the rest follow in string order, so `9`, `10`, `5a`
/// always sort as `9 < 10 < 5a`. Floats compare by value (`-0.0 == 0`) with
/// NaN after every other number, and integers compare with floats exactly.
enum SortKey<'a> {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(Option<f64>, &'a str),
}

/// Floats by value, NaNs (all equal) after every number.
fn cmp_f64(a: f64, b: f64) -> Ordering {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
    }
}

/// An integer against a float without rounding the integer to `f64`, which
/// would make `2^53` and `2^53 + 1` both equal to the float `2^53` but not
/// to each other.
fn cmp_i64_f64(i: i64, f: f64) -> Ordering {
    // 2^63: the first float above every i64.
    const LIMIT: f64 = 9_223_372_036_854_775_808.0;
    if f.is_nan() || f >= LIMIT {
        return Ordering::Less;
    }
    if f < -LIMIT {
        return Ordering::Greater;
    }
    let t = f.trunc();
    // In range and integral, so the cast is exact.
    match i.cmp(&(t as i64)) {
        Ordering::Equal => cmp_f64(t, f),
        o => o,
    }
}

impl SortKey<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        use SortKey::*;
        match (self, other) {
            (Null, Null) => Ordering::Equal,
            (Bool(a), Bool(b)) => a.cmp(b),
            (Int(a), Int(b)) => a.cmp(b),
            (Float(a), Float(b)) => cmp_f64(*a, *b),
            (Int(a), Float(b)) => cmp_i64_f64(*a, *b),
            (Float(a), Int(b)) => cmp_i64_f64(*b, *a).reverse(),
            (Text(Some(x), _), Text(Some(y), _)) => cmp_f64(*x, *y),
            (Text(Some(_), _), Text(None, _)) => Ordering::Less,
            (Text(None, _), Text(Some(_), _)) => Ordering::Greater,
            (Text(None, a), Text(None, b)) => a.cmp(b),
            (a, b) => a.rank().cmp(&b.rank()),
        }
    }

    fn rank(&self) -> u8 {
        match self {
            SortKey::Null => 0,
            SortKey::Bool(_) => 1,
            SortKey::Int(_) | SortKey::Float(_) => 2,
            SortKey::Text(..) => 3,
        }
    }
}

/// A float as psql prints it: shortest digits, with an exponent outside
/// 1e-4 .. 1e15 (`1.1805916207174113e+21`, not 22 digits; `1e-320`, not
/// 320 characters), and the server's names for the non-finite values.
pub fn float_text(f: f64) -> String {
    if f.is_nan() {
        "NaN".into()
    } else if f.is_infinite() {
        if f > 0.0 { "Infinity" } else { "-Infinity" }.into()
    } else if f == 0.0 || (1e-4..1e15).contains(&f.abs()) {
        f.to_string()
    } else {
        let s = format!("{f:e}");
        let (mantissa, exp) = s.split_once('e').unwrap_or((&s, "0"));
        let (sign, digits) = match exp.strip_prefix('-') {
            Some(d) => ('-', d),
            None => ('+', exp),
        };
        format!("{mantissa}e{sign}{digits:0>2}")
    }
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
            Value::String(s) => Cell::Text(s.into()),
            other => Cell::Text(other.to_string().into()),
        }
    }

    /// Text shown in the grid and copied to the clipboard.
    pub fn display(&self, logical: LogicalType) -> std::borrow::Cow<'_, str> {
        use std::borrow::Cow;
        match self {
            Cell::Null => Cow::Borrowed("NULL"),
            Cell::Bool(b) => Cow::Borrowed(if *b { "true" } else { "false" }),
            Cell::Int(i) => Cow::Owned(i.to_string()),
            Cell::Float(f) => Cow::Owned(float_text(*f)),
            Cell::Text(s) if logical == LogicalType::Bytes => Cow::Owned(format!("0x{s}")),
            Cell::Text(s) => Cow::Borrowed(s.as_str()),
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Cell::Null)
    }

    fn sort_key(&self) -> SortKey<'_> {
        match self {
            Cell::Null => SortKey::Null,
            Cell::Bool(b) => SortKey::Bool(*b),
            Cell::Int(i) => SortKey::Int(*i),
            Cell::Float(f) => SortKey::Float(*f),
            Cell::Text(s) => SortKey::Text(s.parse::<f64>().ok().filter(|f| f.is_finite()), s),
        }
    }

    #[cfg(test)]
    fn cmp(&self, other: &Cell) -> Ordering {
        self.sort_key().cmp(&other.sort_key())
    }
}

pub struct ResultSet {
    pub columns: Vec<Column>,
    /// Every cell, row after row (`columns.len()` per row). One allocation
    /// for the whole result instead of one per row keeps a million-row
    /// result compact and the heap unfragmented.
    cells: Vec<Cell>,
    len: usize,
    pub complete: bool,
    pub truncated: bool,
    /// Sorted view: `order[i]` is the row shown at position `i`.
    pub order: Option<Vec<u32>>,
    pub sort: Option<(usize, bool)>,
    /// Column widths for the grid, measured once per result set.
    pub widths: ColumnWidths,
    /// Text lines per row with multi-line rows on, measured as rows are shown.
    pub lines: RowLines,
}

/// How many text lines each row of the grid needs with multi-line rows on.
/// Rows are measured when they are drawn (only the visible ones), so a
/// million-row result is never laid out as a whole; rows not measured yet
/// count as one line.
#[derive(Debug, Default)]
pub struct RowLines {
    /// Lines per row: original rows by their index in the result, new rows
    /// (while editing) after them. 0 = not measured.
    lines: Vec<u8>,
    /// Measured rows taller than one line. While there are none the grid
    /// keeps its fixed row height.
    tall: usize,
    /// What the measurements are for: column widths and the font.
    key: Vec<f32>,
}

impl RowLines {
    pub fn get(&self, row: usize) -> u8 {
        self.lines.get(row).copied().unwrap_or(0).max(1)
    }

    /// Record a measurement; true if it changed.
    pub fn set(&mut self, row: usize, n: u8) -> bool {
        if row >= self.lines.len() {
            if n <= 1 {
                return false;
            }
            self.lines.resize(row + 1, 0);
        }
        let old = std::mem::replace(&mut self.lines[row], n);
        self.tall = self.tall + usize::from(n > 1) - usize::from(old > 1);
        old.max(1) != n.max(1)
    }

    /// Whether any measured row is taller than one line.
    pub fn any_tall(&self) -> bool {
        self.tall > 0
    }

    /// Forget the measurements when the column widths or the font change.
    /// Returns true if they were dropped.
    pub fn invalidate_unless(&mut self, key: &[f32]) -> bool {
        if self.key == key {
            return false;
        }
        self.key = key.to_vec();
        self.lines.clear();
        self.tall = 0;
        true
    }
}

/// The grid's column widths for one result set. The grid measures the first
/// rows once (and again only while they are still arriving), not every frame.
#[derive(Debug, Default)]
pub struct ColumnWidths {
    /// Rows measured so far.
    pub sampled: usize,
    /// Natural width of each column, in characters.
    pub chars: Vec<usize>,
    /// Width of each column as shown in the last frame, in points.
    pub shown: Vec<f32>,
    /// Widths (points) to start from instead of `chars`, after an auto-fit.
    pub initial: Option<Vec<f32>>,
    /// Drop the table's remembered widths next frame and use `initial`.
    pub reset: bool,
}

impl ResultSet {
    /// A complete result set from ready-made rows (tests, fixtures).
    pub fn for_test(columns: Vec<Column>, rows: Vec<Vec<Cell>>) -> Self {
        let mut rs = Self::new(columns);
        for row in rows {
            rs.push_row(row);
        }
        rs.complete = true;
        rs
    }

    fn new(columns: Vec<Column>) -> Self {
        Self {
            columns,
            cells: Vec::new(),
            len: 0,
            complete: false,
            truncated: false,
            order: None,
            sort: None,
            widths: ColumnWidths::default(),
            lines: RowLines::default(),
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Append a row; it is padded with NULLs or cut to the column count.
    fn push_row(&mut self, row: impl IntoIterator<Item = Cell>) {
        let width = self.columns.len();
        let start = self.cells.len();
        self.cells.extend(row.into_iter().take(width));
        self.cells.resize(start + width, Cell::Null);
        self.len += 1;
    }

    /// The row stored at `idx`, ignoring the sort (see [`Self::row`]).
    pub fn raw_row(&self, idx: usize) -> &[Cell] {
        let width = self.columns.len();
        &self.cells[idx * width..(idx + 1) * width]
    }

    /// All rows in the order they arrived.
    pub fn raw_rows(&self) -> impl ExactSizeIterator<Item = &[Cell]> {
        (0..self.len).map(|i| self.raw_row(i))
    }

    /// Every cell, row after row (`columns.len()` per row).
    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    /// Index into `rows` of the row displayed at `pos`.
    pub fn row_index(&self, pos: usize) -> usize {
        self.order.as_ref().map_or(pos, |o| o[pos] as usize)
    }

    /// The row displayed at `pos`, honouring the sort.
    pub fn row(&self, pos: usize) -> &[Cell] {
        self.raw_row(self.row_index(pos))
    }

    /// Cycle ascending → descending → unsorted for column `col`.
    pub fn toggle_sort(&mut self, col: usize) {
        self.sort = match self.sort {
            Some((c, true)) if c == col => Some((col, false)),
            Some((c, false)) if c == col => None,
            _ => Some((col, true)),
        };
        self.order = self.sort.map(|(c, asc)| {
            let keys: Vec<SortKey<'_>> = self.raw_rows().map(|r| r[c].sort_key()).collect();
            let mut order: Vec<u32> = (0..self.len as u32).collect();
            // Stable, so equal values keep their original order.
            order.sort_by(|&a, &b| {
                let o = keys[a as usize].cmp(&keys[b as usize]);
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

/// Where a click landed in the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridHit {
    Cell(usize, usize),
    /// The row-number gutter.
    Row(usize),
    /// The column name (not the sort button).
    Column(usize),
    /// The `#` corner.
    Table,
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

    /// `n_rows` may be 0. `n_cols` is at least 1.
    pub fn click(
        current: Option<Self>,
        hit: GridHit,
        shift: bool,
        n_rows: usize,
        n_cols: usize,
    ) -> Self {
        let last_row = n_rows.saturating_sub(1);
        let last_col = n_cols.saturating_sub(1);
        match hit {
            GridHit::Table => Self {
                anchor: (0, 0),
                focus: (last_row, last_col),
            },
            GridHit::Row(r) => match (current, shift) {
                (Some(s), true) => Self {
                    anchor: (s.anchor.0, 0),
                    focus: (r, last_col),
                },
                _ => Self {
                    anchor: (r, 0),
                    focus: (r, last_col),
                },
            },
            GridHit::Column(c) => match (current, shift) {
                (Some(s), true) => Self {
                    anchor: (0, s.anchor.1),
                    focus: (last_row, c),
                },
                _ => Self {
                    anchor: (0, c),
                    focus: (last_row, c),
                },
            },
            GridHit::Cell(r, c) => match (current, shift) {
                (Some(s), true) => Self {
                    anchor: s.anchor,
                    focus: (r, c),
                },
                _ => Self {
                    anchor: (r, c),
                    focus: (r, c),
                },
            },
        }
    }
}

/// Clipboard copies stop here so a million-row result cannot freeze the UI.
pub const COPY_ROW_LIMIT: usize = 50_000;

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
            // Newer than this build; the client already drops these.
            QueryEvent::Unknown => {}
            QueryEvent::ResultStart { columns, .. } => {
                self.results.push(ResultSet::new(columns));
                if self.results.len() == 1 {
                    self.pane = Pane::Result(0);
                }
            }
            QueryEvent::Rows { index, rows } => {
                if let Some(rs) = self.results.get_mut(index as usize) {
                    rs.cells.reserve(rows.len() * rs.columns.len());
                    for r in rows {
                        rs.push_row(r.into_iter().map(Cell::from_json));
                    }
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
    selection_tsv_capped(rs, sel, headers, usize::MAX).0
}

/// Like [`selection_tsv`], but at most `max_rows` data rows. The bool is
/// `true` when more rows were selected than were written.
pub fn selection_tsv_capped(
    rs: &ResultSet,
    sel: &Selection,
    headers: bool,
    max_rows: usize,
) -> (String, bool) {
    selection_tsv_with(rs, sel, headers, max_rows, rs.len(), |pos, c| {
        let cell = &rs.row(pos)[c];
        (!cell.is_null()).then(|| cell.display(rs.columns[c].logical).into_owned())
    })
}

/// Tab-separated text of the selected cells of a grid of `n_rows` rows,
/// each value (`None` = NULL) from `value(position, column)`, so a grid
/// being edited copies what it shows. At most `max_rows` data rows; the bool
/// is `true` when more were selected.
pub fn selection_tsv_with(
    rs: &ResultSet,
    sel: &Selection,
    headers: bool,
    max_rows: usize,
    n_rows: usize,
    value: impl Fn(usize, usize) -> Option<String>,
) -> (String, bool) {
    let cols: Vec<usize> = sel.cols().filter(|&c| c < rs.columns.len()).collect();
    let mut out = String::new();
    if headers {
        let names: Vec<&str> = cols.iter().map(|&c| rs.columns[c].name.as_str()).collect();
        out.push_str(&names.join("\t"));
        out.push('\n');
    }
    let mut truncated = false;
    for (written, r) in sel.rows().filter(|&r| r < n_rows).enumerate() {
        if written == max_rows {
            truncated = true;
            break;
        }
        let cells: Vec<String> = cols
            .iter()
            .map(|&c| {
                value(r, c)
                    .map(|v| v.replace(['\t', '\n', '\r'], " "))
                    .unwrap_or_default()
            })
            .collect();
        out.push_str(&cells.join("\t"));
        out.push('\n');
    }
    (out, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_print_like_psql() {
        for (f, want) in [
            (0.0, "0"),
            (1.5, "1.5"),
            (-0.0001, "-0.0001"),
            (0.00001, "1e-05"),
            (123456789012345.0, "123456789012345"),
            (1e15, "1e+15"),
            (2f64.powi(70), "1.1805916207174113e+21"),
            (1e300, "1e+300"),
            (1e-320, "1e-320"),
            (-2.5e-7, "-2.5e-07"),
            (f64::NAN, "NaN"),
            (f64::INFINITY, "Infinity"),
            (f64::NEG_INFINITY, "-Infinity"),
        ] {
            assert_eq!(float_text(f), want, "{f}");
        }
    }

    #[test]
    fn row_lines_cache_counts_tall_rows_and_invalidates() {
        let mut l = RowLines::default();
        assert!(l.invalidate_unless(&[100.0, 8.0]));
        assert_eq!(l.get(5), 1, "unmeasured rows are one line");
        assert!(!l.set(1_000_000, 1), "one-line rows need no storage");
        assert!(l.lines.is_empty());
        assert!(!l.any_tall());

        assert!(l.set(5, 3));
        assert_eq!(l.get(5), 3);
        assert!(l.any_tall());
        assert!(!l.set(5, 3), "unchanged");
        assert!(l.set(7, 2));
        assert!(l.set(5, 1));
        assert!(l.any_tall(), "row 7 is still tall");
        assert!(l.set(7, 1));
        assert!(!l.any_tall());

        l.set(2, 4);
        assert!(!l.invalidate_unless(&[100.0, 8.0]), "same widths keep it");
        assert_eq!(l.get(2), 4);
        assert!(
            l.invalidate_unless(&[120.0, 8.0]),
            "a resized column drops it"
        );
        assert_eq!(l.get(2), 1);
        assert!(!l.any_tall());
    }
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

    /// Every pair and triple of `cells` must agree with a total order.
    fn assert_total_order(cells: &[Cell]) {
        for a in cells {
            assert_eq!(a.cmp(a), Ordering::Equal, "{a:?} is not equal to itself");
            for b in cells {
                assert_eq!(a.cmp(b), b.cmp(a).reverse(), "{a:?} vs {b:?}");
                for c in cells {
                    if a.cmp(b) != Ordering::Greater && b.cmp(c) != Ordering::Greater {
                        assert_ne!(
                            a.cmp(c),
                            Ordering::Greater,
                            "{a:?} <= {b:?} <= {c:?} but {a:?} > {c:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn mixed_values_form_a_total_order() {
        let big = 1i64 << 53;
        let cells = vec![
            Cell::Null,
            Cell::Bool(false),
            Cell::Bool(true),
            Cell::Int(0),
            Cell::Int(-3),
            Cell::Int(big),
            Cell::Int(big + 1),
            Cell::Int(i64::MAX),
            Cell::Int(i64::MIN),
            Cell::Float(big as f64),
            Cell::Float(0.0),
            Cell::Float(-0.0),
            Cell::Float(2.5),
            Cell::Float(-2.5),
            Cell::Float(f64::NAN),
            Cell::Float(-f64::NAN),
            Cell::Float(f64::INFINITY),
            Cell::Float(f64::NEG_INFINITY),
            Cell::Float(9.3e18),
            Cell::Float(-9.3e18),
            Cell::Float(-9_223_372_036_854_775_808.0),
            Cell::Text("10".into()),
            Cell::Text("9".into()),
            Cell::Text("5a".into()),
            Cell::Text("9.0".into()),
            Cell::Text("-1".into()),
            Cell::Text("NaN".into()),
            Cell::Text("inf".into()),
            Cell::Text("".into()),
            Cell::Text("abc".into()),
            Cell::Text("1e400".into()),
        ];
        assert_total_order(&cells);
    }

    #[test]
    fn numeric_text_sorts_before_other_text() {
        let t = |s: &str| Cell::Text(s.into());
        assert_eq!(t("9").cmp(&t("10")), Ordering::Less);
        assert_eq!(t("10").cmp(&t("5a")), Ordering::Less);
        assert_eq!(t("5a").cmp(&t("9")), Ordering::Greater);
        assert_eq!(t("NaN").cmp(&t("1")), Ordering::Greater, "NaN text is text");
        let big = 1i64 << 53;
        assert_eq!(
            Cell::Int(big).cmp(&Cell::Float(big as f64)),
            Ordering::Equal
        );
        assert_eq!(
            Cell::Int(big + 1).cmp(&Cell::Float(big as f64)),
            Ordering::Greater
        );
        assert_eq!(
            Cell::Float(f64::NAN).cmp(&Cell::Int(i64::MAX)),
            Ordering::Greater
        );
        assert_eq!(Cell::Float(-0.0).cmp(&Cell::Float(0.0)), Ordering::Equal);
        assert_eq!(Cell::Null.cmp(&Cell::Float(f64::NAN)), Ordering::Less);
    }

    #[test]
    fn sorting_mixed_codes_never_panics() {
        // A small xorshift so the test needs no extra dependency.
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for seed in 0..150 {
            let rows = [30usize, 100, 1000][seed % 3];
            let mixed_types = seed % 2 == 1;
            let cells: Vec<Vec<Cell>> = (0..rows)
                .map(|_| {
                    let n = next() % 1000;
                    let kind = next() % if mixed_types { 8 } else { 2 };
                    let cell = match kind {
                        0 => Cell::Text(n.to_string().into()),
                        1 => Cell::Text(format!("{n}a").into()),
                        2 => Cell::Text(format!("{n}.5").into()),
                        3 => Cell::Null,
                        4 => Cell::Int(n as i64),
                        5 if n % 7 == 0 => Cell::Float(f64::NAN),
                        5 => Cell::Float(n as f64 / 3.0),
                        6 => Cell::Bool(n % 2 == 0),
                        _ => Cell::Text("NaN".into()),
                    };
                    vec![cell]
                })
                .collect();
            let mut rs = ResultSet::for_test(vec![col("code", LogicalType::Text)], cells);
            rs.toggle_sort(0);
            for p in 1..rs.len() {
                let o = rs.row(p - 1)[0].cmp(&rs.row(p)[0]);
                assert_ne!(o, Ordering::Greater, "seed {seed}: ascending at {p}");
            }
            rs.toggle_sort(0);
            for p in 1..rs.len() {
                let o = rs.row(p - 1)[0].cmp(&rs.row(p)[0]);
                assert_ne!(o, Ordering::Less, "seed {seed}: descending at {p}");
            }
        }
    }

    #[test]
    fn cells_stay_small() {
        assert_eq!(std::mem::size_of::<Cell>(), 24);
    }

    #[test]
    fn short_text_is_inline_and_long_text_on_the_heap() {
        let date = "2026-01-31 12:00:00.123"; // 23 bytes: one over the limit
        for s in ["", "paid", "1234.56", "héllo ✓ 世界", &date[..22], date] {
            let c = CellStr::from(s);
            assert_eq!(c.as_str(), s);
            assert_eq!(CellStr::from(s.to_string()).as_str(), s);
            assert_eq!(
                matches!(c, CellStr::Inline { .. }),
                s.len() <= CellStr::INLINE,
                "{s:?}"
            );
        }
    }

    #[test]
    fn big_integers_sort_exactly() {
        // Beyond 2^53 these are equal as f64.
        let (a, b) = (
            Cell::Int(9_007_199_254_740_993),
            Cell::Int(9_007_199_254_740_992),
        );
        assert_eq!(a.cmp(&b), Ordering::Greater);
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
        let (capped, truncated) = selection_tsv_capped(&run.results[0], &sel, false, 1);
        assert_eq!(capped, "1\ta b\n");
        assert!(truncated);
    }

    #[test]
    fn clicks_select_rows_columns_and_the_table() {
        let table = Selection::click(None, GridHit::Table, false, 4, 3);
        assert!(table.contains(0, 0) && table.contains(3, 2));
        assert!(!table.contains(4, 0));

        let row = Selection::click(None, GridHit::Row(1), false, 4, 3);
        assert_eq!(row.rows(), 1..=1);
        assert_eq!(row.cols(), 0..=2);
        let rows = Selection::click(Some(row), GridHit::Row(3), true, 4, 3);
        assert_eq!(rows.rows(), 1..=3);
        assert_eq!(rows.cols(), 0..=2);

        let col = Selection::click(None, GridHit::Column(1), false, 4, 3);
        assert_eq!(col.rows(), 0..=3);
        assert_eq!(col.cols(), 1..=1);
        let cols = Selection::click(Some(col), GridHit::Column(0), true, 4, 3);
        assert_eq!(cols.cols(), 0..=1);
        assert_eq!(cols.rows(), 0..=3);

        let cell = Selection::click(None, GridHit::Cell(1, 1), false, 4, 3);
        assert_eq!(cell.anchor, (1, 1));
        let block = Selection::click(Some(cell), GridHit::Cell(2, 2), true, 4, 3);
        assert_eq!(block.rows(), 1..=2);
        assert_eq!(block.cols(), 1..=2);
        // Shift-click a row number from a cell still selects whole rows.
        let from_cell = Selection::click(Some(cell), GridHit::Row(3), true, 4, 3);
        assert_eq!(from_cell.rows(), 1..=3);
        assert_eq!(from_cell.cols(), 0..=2);
    }
}
