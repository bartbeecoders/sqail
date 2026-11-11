//! Export results to CSV / JSON / Excel / SQL, streaming row by row, and
//! import CSV files into a table. No UI here; the app drives these on the
//! background runtime.

use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use anyhow::{Context, Result, bail};
use futures::StreamExt;
use sqail_client::proto::{
    Column, ColumnInfo, Engine, LogicalType, Param, QueryEvent, QueryRequest,
};
use sqail_client::{Client, On};
use uuid::Uuid;

use crate::editing::{binary_param, binary_placeholder, is_binary_type, parse_hex};
use crate::results::Cell;
use crate::sql::quote_ident;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Csv,
    Json,
    Xlsx,
    Sql,
}

impl Format {
    pub const ALL: [Format; 4] = [Format::Csv, Format::Json, Format::Xlsx, Format::Sql];

    pub fn label(self) -> &'static str {
        match self {
            Format::Csv => "CSV",
            Format::Json => "JSON",
            Format::Xlsx => "Excel (.xlsx)",
            Format::Sql => "SQL INSERT statements",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Csv => "csv",
            Format::Json => "json",
            Format::Xlsx => "xlsx",
            Format::Sql => "sql",
        }
    }
}

/// Excel's hard limit is 1,048,576 rows per sheet (one is the header).
const XLSX_ROWS_PER_SHEET: u32 = 1_048_575;

/// How an export writes its file, besides the format.
#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    /// Target of SQL INSERT exports, already quoted; `exported` when unknown.
    pub table: Option<String>,
    /// CSV: start with a UTF-8 byte order mark (Excel on Windows needs it to
    /// read the file as UTF-8).
    pub csv_bom: bool,
    /// CSV: text written for NULL (empty: NULL and '' look the same).
    pub csv_null: String,
}

/// Cell formats for dates and times in Excel exports.
struct XlsxFormats {
    date: rust_xlsxwriter::Format,
    datetime: rust_xlsxwriter::Format,
    datetime_ms: rust_xlsxwriter::Format,
    time: rust_xlsxwriter::Format,
    time_ms: rust_xlsxwriter::Format,
}

impl XlsxFormats {
    fn new() -> Self {
        let f = |n: &str| rust_xlsxwriter::Format::new().set_num_format(n);
        Self {
            date: f("yyyy-mm-dd"),
            datetime: f("yyyy-mm-dd hh:mm:ss"),
            datetime_ms: f("yyyy-mm-dd hh:mm:ss.000"),
            time: f("hh:mm:ss"),
            time_ms: f("hh:mm:ss.000"),
        }
    }
}

enum Sink {
    Csv(Box<csv::Writer<BufWriter<File>>>),
    Json {
        out: BufWriter<File>,
        first: bool,
    },
    Xlsx {
        book: Box<rust_xlsxwriter::Workbook>,
        sheet: usize,
        row: u32,
        formats: Box<XlsxFormats>,
    },
    Sql {
        out: BufWriter<File>,
        table: String,
    },
}

/// Writes one result set to a file, a row at a time. The rows go to a
/// temporary file next to the target, which replaces the target only when
/// [`Exporter::finish`] succeeds: a failed or abandoned export leaves no
/// partial file (and an existing file untouched).
pub struct Exporter {
    /// `None` once finished (or while being finished).
    sink: Option<Sink>,
    columns: Vec<Column>,
    engine: Engine,
    csv_null: String,
    path: PathBuf,
    tmp: PathBuf,
    pub rows: u64,
}

impl Exporter {
    pub fn create(
        path: &Path,
        format: Format,
        columns: Vec<Column>,
        engine: Engine,
        opts: &ExportOptions,
    ) -> Result<Self> {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let dir = match path.parent() {
            Some(d) if !d.as_os_str().is_empty() => d,
            _ => Path::new("."),
        };
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let tmp = dir.join(format!(
            ".{name}.{}.{}.part",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let mut ex = Self {
            sink: None,
            columns,
            engine,
            csv_null: opts.csv_null.clone(),
            path: path.to_path_buf(),
            tmp,
            rows: 0,
        };
        // On an error below, dropping `ex` removes the temporary file.
        ex.sink = Some(ex.open(format, opts)?);
        Ok(ex)
    }

    fn open(&self, format: Format, opts: &ExportOptions) -> Result<Sink> {
        let (engine, path) = (self.engine, &self.tmp);
        let file = || {
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .with_context(|| format!("creating {}", self.path.display()))
        };
        let names: Vec<&str> = self.columns.iter().map(|c| c.name.as_str()).collect();
        let sink = match format {
            Format::Csv => {
                let mut out = BufWriter::new(file()?);
                if opts.csv_bom {
                    out.write_all(b"\xEF\xBB\xBF")?;
                }
                let mut w = csv::Writer::from_writer(out);
                w.write_record(&names)?;
                Sink::Csv(Box::new(w))
            }
            Format::Json => {
                let mut out = BufWriter::new(file()?);
                out.write_all(b"[\n")?;
                Sink::Json { out, first: true }
            }
            Format::Xlsx => {
                let mut book = Box::new(rust_xlsxwriter::Workbook::new());
                let sheet = book.add_worksheet_with_constant_memory()?;
                write_xlsx_header(sheet, &names)?;
                Sink::Xlsx {
                    book,
                    sheet: 0,
                    row: 1,
                    formats: Box::new(XlsxFormats::new()),
                }
            }
            Format::Sql => {
                let cols: Vec<String> = names.iter().map(|n| quote_ident(engine, n)).collect();
                let table = format!(
                    "INSERT INTO {} ({}) VALUES ",
                    opts.table.as_deref().unwrap_or("exported"),
                    cols.join(", ")
                );
                Sink::Sql {
                    out: BufWriter::new(file()?),
                    table,
                }
            }
        };
        Ok(sink)
    }

    pub fn row(&mut self, cells: &[Cell]) -> Result<()> {
        self.rows += 1;
        let Some(sink) = self.sink.as_mut() else {
            bail!("the export is finished");
        };
        match sink {
            Sink::Csv(w) => {
                let fields: Vec<String> = cells
                    .iter()
                    .zip(&self.columns)
                    .map(|(c, col)| {
                        if c.is_null() {
                            self.csv_null.clone()
                        } else {
                            c.display(col.logical).into_owned()
                        }
                    })
                    .collect();
                w.write_record(&fields)?;
            }
            Sink::Json { out, first } => {
                out.write_all(if *first { b"  {" } else { b",\n  {" })?;
                *first = false;
                for (i, (c, col)) in cells.iter().zip(&self.columns).enumerate() {
                    if i > 0 {
                        out.write_all(b", ")?;
                    }
                    serde_json::to_writer(&mut *out, &col.name)?;
                    out.write_all(b": ")?;
                    // JSON columns are embedded as JSON, exactly as the
                    // database sent them (no number rounding on the way).
                    if let Cell::Text(s) = c
                        && col.logical == LogicalType::Json
                        && serde_json::from_str::<serde::de::IgnoredAny>(s).is_ok()
                    {
                        out.write_all(s.trim().as_bytes())?;
                        continue;
                    }
                    let v = match c {
                        Cell::Null => serde_json::Value::Null,
                        Cell::Bool(b) => serde_json::Value::Bool(*b),
                        Cell::Int(i) => serde_json::Value::from(*i),
                        Cell::Float(f) => serde_json::Number::from_f64(*f)
                            .map(serde_json::Value::Number)
                            .unwrap_or_else(|| serde_json::Value::String(f.to_string())),
                        Cell::Text(s) => serde_json::Value::String(s.to_string()),
                    };
                    serde_json::to_writer(&mut *out, &v)?;
                }
                out.write_all(b"}")?;
            }
            Sink::Xlsx {
                book,
                sheet,
                row,
                formats,
            } => {
                if *row > XLSX_ROWS_PER_SHEET {
                    let ws = book.add_worksheet_with_constant_memory()?;
                    let names: Vec<&str> = self.columns.iter().map(|c| c.name.as_str()).collect();
                    write_xlsx_header(ws, &names)?;
                    *sheet += 1;
                    *row = 1;
                }
                let ws = book.worksheet_from_index(*sheet)?;
                for (i, (c, col)) in cells.iter().zip(&self.columns).enumerate() {
                    let i = i as u16;
                    match xlsx_value(c, col.logical) {
                        XlsxValue::Empty => {}
                        XlsxValue::Bool(b) => {
                            ws.write_boolean(*row, i, b)?;
                        }
                        XlsxValue::Number(v) => {
                            ws.write_number(*row, i, v)?;
                        }
                        XlsxValue::Date(d, kind) => {
                            let f = match kind {
                                DateKind::Date => &formats.date,
                                DateKind::DateTime => &formats.datetime,
                                DateKind::DateTimeMs => &formats.datetime_ms,
                                DateKind::Time => &formats.time,
                                DateKind::TimeMs => &formats.time_ms,
                            };
                            ws.write_number_with_format(*row, i, d, f)?;
                        }
                        XlsxValue::Text(text) => {
                            ws.write_string(*row, i, text)?;
                        }
                    }
                }
                *row += 1;
            }
            Sink::Sql { out, table } => {
                let values: Vec<String> = cells
                    .iter()
                    .zip(&self.columns)
                    .map(|(c, col)| sql_literal(self.engine, c, col.logical))
                    .collect();
                writeln!(out, "{table}({});", values.join(", "))?;
            }
        }
        Ok(())
    }

    /// Complete the file and move it into place. On an error the target is
    /// left as it was.
    pub fn finish(mut self) -> Result<u64> {
        let Some(sink) = self.sink.take() else {
            bail!("the export is finished");
        };
        // Close the file before it is renamed (Windows can't rename an open
        // file).
        let closed: Result<()> = (|| {
            let file = match sink {
                Sink::Csv(w) => Some(
                    w.into_inner()
                        .map_err(|e| e.into_error())?
                        .into_inner()
                        .map_err(|e| e.into_error())?,
                ),
                Sink::Json { mut out, .. } => {
                    out.write_all(b"\n]\n")?;
                    Some(out.into_inner().map_err(|e| e.into_error())?)
                }
                Sink::Xlsx { mut book, .. } => {
                    book.save(&self.tmp)?;
                    None
                }
                Sink::Sql { out, .. } => Some(out.into_inner().map_err(|e| e.into_error())?),
            };
            if let Some(f) = file {
                f.sync_all()?;
            }
            Ok(())
        })();
        closed?;
        std::fs::rename(&self.tmp, &self.path)
            .with_context(|| format!("writing {}", self.path.display()))?;
        Ok(self.rows)
    }
}

impl Drop for Exporter {
    fn drop(&mut self) {
        // Unfinished (failed or abandoned): close and remove the partial file.
        // After a successful finish it was renamed and this finds nothing.
        drop(self.sink.take());
        let _ = std::fs::remove_file(&self.tmp);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DateKind {
    Date,
    DateTime,
    DateTimeMs,
    Time,
    TimeMs,
}

/// What an Excel cell gets for a value.
#[derive(Debug, PartialEq)]
enum XlsxValue {
    Empty,
    Bool(bool),
    Number(f64),
    /// An Excel serial date (days since 1900), shown with a date format.
    Date(f64, DateKind),
    Text(String),
}

/// Excel keeps 15 significant digits: numbers with more are written as
/// text so no digit is lost; decimals, dates and times that Excel holds
/// exactly become real numbers and dates.
fn xlsx_value(c: &Cell, logical: LogicalType) -> XlsxValue {
    match c {
        Cell::Null => XlsxValue::Empty,
        Cell::Bool(b) => XlsxValue::Bool(*b),
        Cell::Int(v) if v.unsigned_abs() < 1_000_000_000_000_000 => XlsxValue::Number(*v as f64),
        Cell::Int(v) => XlsxValue::Text(v.to_string()),
        Cell::Float(v) if v.is_finite() => XlsxValue::Number(*v),
        Cell::Float(v) => XlsxValue::Text(crate::results::float_text(*v)),
        Cell::Text(s) => {
            let s = s.as_str();
            if logical == LogicalType::Decimal
                && let Some(v) = excel_exact_decimal(s)
            {
                return XlsxValue::Number(v);
            }
            if let Some((d, kind)) = excel_datetime(s, logical) {
                return XlsxValue::Date(d.to_excel(), kind);
            }
            // Excel cells hold at most 32,767 characters.
            let text = c.display(logical);
            XlsxValue::Text(if text.chars().count() > 32_767 {
                text.chars().take(32_767).collect()
            } else {
                text.into_owned()
            })
        }
    }
}

/// `-?digits[.digits][e±digits]`: a number literal SQL reads as a number.
fn plain_number(s: &str) -> bool {
    let s = s.strip_prefix('-').unwrap_or(s);
    let (mantissa, exp) = match s.split_once(['e', 'E']) {
        Some((m, e)) => (m, Some(e)),
        None => (s, None),
    };
    let digits = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
    let mantissa_ok = match mantissa.split_once('.') {
        Some((i, f)) => digits(i) && digits(f),
        None => digits(mantissa),
    };
    let exp_ok = exp.is_none_or(|e| digits(e.strip_prefix(['+', '-']).unwrap_or(e)));
    mantissa_ok && exp_ok
}

/// A decimal Excel shows exactly: plain digits, at most 15 significant.
fn excel_exact_decimal(s: &str) -> Option<f64> {
    if !plain_number(s) || s.contains(['e', 'E']) {
        return None;
    }
    let unsigned = s.trim_start_matches('-');
    let (int, frac) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    let all = format!("{int}{frac}");
    let significant = all.trim_start_matches('0').trim_end_matches('0').len();
    if significant > 15 {
        return None;
    }
    s.parse().ok()
}

/// A date, time or timestamp as the service sends it (`2024-01-31`,
/// `13:45:12.997`, `2024-01-31T13:45:12.5` or with a space), when Excel holds
/// it exactly: years 1900 to 9999, at most millisecond precision. Time zones
/// are kept as text: Excel has none, and dropping the offset would change
/// the value.
fn excel_datetime(
    s: &str,
    logical: LogicalType,
) -> Option<(rust_xlsxwriter::ExcelDateTime, DateKind)> {
    use rust_xlsxwriter::ExcelDateTime;
    fn num<T: std::str::FromStr>(t: &str, len: usize) -> Option<T> {
        (t.len() == len && t.bytes().all(|b| b.is_ascii_digit()))
            .then(|| t.parse().ok())
            .flatten()
    }
    fn date(t: &str) -> Option<ExcelDateTime> {
        let mut p = t.split('-');
        let (y, m, d) = (p.next()?, p.next()?, p.next()?);
        if p.next().is_some() {
            return None;
        }
        let y: u16 = num(y, 4)?;
        // Day 0 (1899-12-31) and earlier don't show as dates in Excel.
        if y < 1900 {
            return None;
        }
        ExcelDateTime::from_ymd(y, num(m, 2)?, num(d, 2)?).ok()
    }
    /// (hour, minute, second, millisecond, has a fraction)
    fn time(t: &str) -> Option<(u16, u8, u8, u16, bool)> {
        let (hms, frac) = t.split_once('.').unwrap_or((t, ""));
        let mut p = hms.split(':');
        let (h, m, sec) = (p.next()?, p.next()?, p.next()?);
        if p.next().is_some() || !frac.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        // Excel keeps milliseconds; finer digits must be zero.
        if frac.len() > 3 && frac[3..].bytes().any(|b| b != b'0') {
            return None;
        }
        let ms = format!("{:0<3}", &frac[..frac.len().min(3)]);
        let (h, m, sec): (u16, u8, u8) = (num(h, 2)?, num(m, 2)?, num(sec, 2)?);
        if h > 23 || m > 59 || sec > 59 {
            return None;
        }
        Some((h, m, sec, ms.parse().ok()?, !frac.is_empty()))
    }
    match logical {
        LogicalType::Date => Some((date(s)?, DateKind::Date)),
        LogicalType::Time => {
            let (h, m, sec, ms, frac) = time(s)?;
            let d = ExcelDateTime::from_hms_milli(h, m, sec, ms).ok()?;
            Some((
                d,
                if frac {
                    DateKind::TimeMs
                } else {
                    DateKind::Time
                },
            ))
        }
        LogicalType::Timestamp => {
            let (d, t) = s.split_once(['T', ' '])?;
            let (h, m, sec, ms, frac) = time(t)?;
            let d = date(d)?.and_hms_milli(h, m, sec, ms).ok()?;
            Some((
                d,
                if frac {
                    DateKind::DateTimeMs
                } else {
                    DateKind::DateTime
                },
            ))
        }
        _ => None,
    }
}

fn write_xlsx_header(ws: &mut rust_xlsxwriter::Worksheet, names: &[&str]) -> Result<()> {
    let bold = rust_xlsxwriter::Format::new().set_bold();
    for (i, n) in names.iter().enumerate() {
        ws.write_string_with_format(0, i as u16, *n, &bold)?;
    }
    Ok(())
}

/// A value as an SQL literal in the engine's dialect.
pub fn sql_literal(engine: Engine, c: &Cell, logical: LogicalType) -> String {
    match c {
        Cell::Null => "NULL".into(),
        Cell::Bool(b) => match engine {
            Engine::Postgres => if *b { "TRUE" } else { "FALSE" }.into(),
            _ => if *b { "1" } else { "0" }.into(),
        },
        Cell::Int(i) => i.to_string(),
        Cell::Float(f) if f.is_finite() => f.to_string(),
        Cell::Float(f) => format!("'{f}'"),
        Cell::Text(s) if logical == LogicalType::Bytes => match engine {
            Engine::Postgres => format!("'\\x{s}'::bytea"),
            Engine::Mssql => format!("0x{s}"),
            Engine::Sqlite | Engine::Unknown => format!("X'{s}'"),
        },
        // Only digits go in bare: `NaN` and `Infinity` parse as floats but
        // are not SQL literals.
        Cell::Text(s) if logical == LogicalType::Decimal && plain_number(s) => s.to_string(),
        Cell::Text(s) => {
            let quoted = format!("'{}'", s.replace('\'', "''"));
            if engine == Engine::Mssql {
                format!("N{quoted}")
            } else {
                quoted
            }
        }
    }
}

/// The statement of `script` that produced result set `result` (of `results`
/// shown), for exporting that whole result by running it again. Only a single
/// read-only statement qualifies, and only when the script's read-only
/// statements line up one-to-one with its result sets: otherwise it is not
/// clear which statement produced the result, and running anything again
/// could repeat writes. `finished` is false when the run stopped early
/// (error, cancel), so later statements may not have produced theirs.
pub fn statement_for_result(
    script: &str,
    engine: Engine,
    result: usize,
    results: usize,
    finished: bool,
) -> std::result::Result<String, String> {
    const UNCLEAR: &str = "Only a result of a read-only statement can be exported whole, and \
        this script's results can't be matched to its statements. Run the SELECT on its own, \
        then export again.";
    let reads: Vec<&str> = executed_statements(script, engine)
        .into_iter()
        .filter(|s| crate::assistant::guard::check_read_only(s, engine).is_ok())
        .collect();
    let lines_up = if finished {
        reads.len() == results
    } else {
        reads.len() >= results
    };
    match reads.get(result) {
        Some(s) if lines_up && result < results => Ok(s.to_string()),
        _ => Err(UNCLEAR.into()),
    }
}

/// The quoted table an SQL INSERT export of result `result` targets: the
/// single table its statement reads from, when that is known.
pub fn insert_target(
    script: &str,
    engine: Engine,
    result: usize,
    results: usize,
    finished: bool,
) -> Option<String> {
    let sql = statement_for_result(script, engine, result, results, finished).ok()?;
    let src = crate::editing::source_table(&sql, engine)?;
    let table = quote_ident(engine, &src.table);
    Some(match src.schema {
        Some(s) => format!("{}.{table}", quote_ident(engine, &s)),
        None => table,
    })
}

/// Statements as the engine sees them: the editor's splitter also breaks at
/// blank lines (for "run statement at cursor"), so pieces separated only by
/// whitespace are joined again.
fn executed_statements(script: &str, engine: Engine) -> Vec<&str> {
    let mut out: Vec<std::ops::Range<usize>> = Vec::new();
    for r in crate::sql::statements::split(script, Some(engine)) {
        match out.last_mut() {
            Some(prev) if script[prev.end..r.start].trim().is_empty() => prev.end = r.end,
            _ => out.push(r),
        }
    }
    out.into_iter().map(|r| &script[r]).collect()
}

/// What an export wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exported {
    pub rows: u64,
    /// The rows stopped at a row limit: the query returns more than the file
    /// holds.
    pub truncated: bool,
}

/// The notice after an export of `rows` rows to `path`, and whether it is a
/// warning. `whole` is a re-run of the query (cut off by the service's
/// `limits.max_rows`), otherwise the shown result (cut off by the tab's row
/// limit).
pub fn export_notice(rows: u64, path: &Path, truncated: bool, whole: bool) -> (String, bool) {
    let done = format!("Exported {rows} rows to {}", path.display());
    if !truncated {
        return (done, false);
    }
    let remedy = if whole {
        "the service stopped at its row limit (limits.max_rows in its config), so rows are \
         missing. Raise that limit to export them all."
    } else {
        "the result was cut off at the row limit, so rows are missing. Use Export → Whole \
         query to export them all."
    };
    (format!("{done}, but {remedy}"), true)
}

/// Run `sql` (one statement) and stream its first result set to `path`
/// without holding the rows in memory. `progress` gets row counts. With
/// `expect_columns`, the export is refused unless the result has exactly
/// these column names (it must be the result the user is looking at).
#[allow(clippy::too_many_arguments)]
pub async fn export_query(
    client: &Client,
    on: On,
    engine: Engine,
    sql: &str,
    expect_columns: Option<&[String]>,
    path: &Path,
    format: Format,
    opts: &ExportOptions,
    mut progress: impl FnMut(u64),
) -> Result<Exported> {
    let req = QueryRequest {
        sql: sql.to_string(),
        params: vec![],
        // As many as the service allows.
        max_rows: Some(u64::MAX),
        timeout_ms: None,
        single_statement: false,
    };
    let mut stream = client.query(on, &req).await?;
    let mut exporter: Option<Exporter> = None;
    let mut error = None;
    let mut truncated = false;
    while let Some(ev) = stream.events.next().await {
        match ev? {
            QueryEvent::ResultStart { index: 0, columns } => {
                if let Some(expected) = expect_columns
                    && !columns.iter().map(|c| &c.name).eq(expected.iter())
                {
                    bail!("the query now returns different columns than the result shown");
                }
                exporter = Some(Exporter::create(path, format, columns, engine, opts)?);
            }
            QueryEvent::Rows { index: 0, rows } => {
                if let Some(ex) = exporter.as_mut() {
                    for r in rows {
                        let cells: Vec<Cell> = r.into_iter().map(Cell::from_json).collect();
                        ex.row(&cells)?;
                    }
                    progress(ex.rows);
                }
            }
            QueryEvent::ResultEnd {
                index: 0,
                truncated: t,
                ..
            } => truncated = t,
            QueryEvent::Error { message, .. } => error = Some(message),
            _ => {}
        }
    }
    if let Some(e) = error {
        bail!(e);
    }
    match exporter {
        Some(ex) => Ok(Exported {
            rows: ex.finish()?,
            truncated,
        }),
        None => bail!("the query returned no result set"),
    }
}

// ----------------------------------------------------------------- import --

/// A parsed CSV header plus a few sample rows, for the mapping dialog.
#[derive(Debug, Clone, Default)]
pub struct CsvPreview {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub delimiter: u8,
    pub encoding: Encoding,
}

/// The delimiters the import offers.
pub const DELIMITERS: [u8; 4] = *b",;\t|";

pub fn delimiter_label(d: u8) -> String {
    match d {
        b',' => "comma (,)".into(),
        b';' => "semicolon (;)".into(),
        b'\t' => "tab".into(),
        b'|' => "pipe (|)".into(),
        d => (d as char).to_string(),
    }
}

/// Guess the delimiter from the first lines of `sample`: the one of
/// , ; tab | that splits every line into the same number of fields, the
/// most fields winning; without such a one, the most frequent in the first
/// line; without any, a comma (a single-column file).
pub fn sniff_delimiter(sample: &str) -> u8 {
    let lines: Vec<&str> = sample
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(10)
        .collect();
    let score = |d: u8| {
        let counts: Vec<usize> = lines
            .iter()
            .map(|l| l.bytes().filter(|&b| b == d).count())
            .collect();
        let first = counts.first().copied().unwrap_or(0);
        let consistent = first > 0 && counts.iter().all(|&c| c == first);
        (consistent, first)
    };
    let mut best = (b',', (false, 0));
    for d in DELIMITERS {
        let sc = score(d);
        if sc > best.1 {
            best = (d, sc);
        }
    }
    best.0
}

/// Text encodings the import reads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Encoding {
    #[default]
    Utf8,
    Utf16Le,
    Utf16Be,
    /// Windows' Western European ("ANSI") code page.
    Windows1252,
}

impl Encoding {
    pub const ALL: [Encoding; 4] = [
        Encoding::Utf8,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
        Encoding::Windows1252,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Encoding::Utf8 => "UTF-8",
            Encoding::Utf16Le => "UTF-16 LE",
            Encoding::Utf16Be => "UTF-16 BE",
            Encoding::Windows1252 => "Windows-1252",
        }
    }

    /// The encoding of a file starting with `head`: from its byte order
    /// mark, else UTF-16 when every other byte is zero, else UTF-8 when it
    /// is valid UTF-8, else Windows-1252.
    pub fn detect(head: &[u8]) -> Encoding {
        match head {
            [0xEF, 0xBB, 0xBF, ..] => return Encoding::Utf8,
            [0xFF, 0xFE, ..] => return Encoding::Utf16Le,
            [0xFE, 0xFF, ..] => return Encoding::Utf16Be,
            _ => {}
        }
        let pairs = head.len() / 2;
        if pairs >= 2 {
            let zeros = |odd: bool| {
                head.as_chunks::<2>()
                    .0
                    .iter()
                    .filter(|p| p[usize::from(odd)] == 0)
                    .count()
            };
            if zeros(true) * 10 >= pairs * 9 && zeros(false) == 0 {
                return Encoding::Utf16Le;
            }
            if zeros(false) * 10 >= pairs * 9 && zeros(true) == 0 {
                return Encoding::Utf16Be;
            }
        }
        match std::str::from_utf8(head) {
            Ok(_) => Encoding::Utf8,
            // Cut in the middle of a character at the end of the sample.
            Err(e) if e.error_len().is_none() => Encoding::Utf8,
            Err(_) => Encoding::Windows1252,
        }
    }
}

/// Windows-1252 bytes 0x80..=0x9F; the rest are the same as Latin-1.
const CP1252_HIGH: [char; 32] = [
    '\u{20AC}', '\u{81}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{8D}', '\u{017D}', '\u{8F}',
    '\u{90}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{9D}', '\u{017E}', '\u{0178}',
];

/// Reads text in `encoding` as UTF-8, a chunk at a time, so a large file is
/// never held in memory. A byte order mark is dropped. UTF-8 passes through
/// (the CSV reader checks it).
///
/// A small self-contained decoder; the .sql file open/save code may grow its
/// own, and the two can be unified later.
struct Decode<R> {
    inner: R,
    encoding: Encoding,
    /// Decoded bytes not yet handed out, from `pos`.
    out: Vec<u8>,
    pos: usize,
    /// Undecoded bytes left over (half a UTF-16 unit or surrogate pair).
    rest: Vec<u8>,
    start: bool,
}

impl<R: Read> Decode<R> {
    fn new(inner: R, encoding: Encoding) -> Self {
        Self {
            inner,
            encoding,
            out: Vec::new(),
            pos: 0,
            rest: Vec::new(),
            start: true,
        }
    }

    /// Decode the next chunk into `out`; false at the end of the input.
    fn fill(&mut self) -> std::io::Result<bool> {
        let mut buf = [0u8; 16 * 1024];
        let n = self.inner.read(&mut buf)?;
        let mut bytes = std::mem::take(&mut self.rest);
        bytes.extend_from_slice(&buf[..n]);
        if self.start && (bytes.len() >= 2 || n == 0) {
            self.start = false;
            let bom: &[u8] = match self.encoding {
                Encoding::Utf16Le => &[0xFF, 0xFE],
                Encoding::Utf16Be => &[0xFE, 0xFF],
                Encoding::Utf8 | Encoding::Windows1252 => &[],
            };
            if !bom.is_empty() && bytes.starts_with(bom) {
                bytes.drain(..2);
            }
        } else if self.start {
            self.rest = bytes;
            return Ok(true);
        }
        self.out.clear();
        self.pos = 0;
        let bad =
            |what: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, what.to_string());
        match self.encoding {
            Encoding::Utf8 => self.out = bytes,
            Encoding::Windows1252 => {
                let mut text = String::with_capacity(bytes.len());
                for b in bytes {
                    text.push(match b {
                        0x80..=0x9F => CP1252_HIGH[usize::from(b - 0x80)],
                        b => char::from(b),
                    });
                }
                self.out = text.into_bytes();
            }
            Encoding::Utf16Le | Encoding::Utf16Be => {
                let le = self.encoding == Encoding::Utf16Le;
                let mut units: Vec<u16> = bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|p| {
                        if le {
                            u16::from_le_bytes([p[0], p[1]])
                        } else {
                            u16::from_be_bytes([p[0], p[1]])
                        }
                    })
                    .collect();
                let mut keep = bytes.len() % 2;
                // A high surrogate at the end waits for its pair.
                if n > 0 && units.last().is_some_and(|u| (0xD800..0xDC00).contains(u)) {
                    units.pop();
                    keep += 2;
                }
                if n == 0 && keep > 0 {
                    return Err(bad("the file ends in the middle of a UTF-16 character"));
                }
                self.rest = bytes[bytes.len() - keep..].to_vec();
                let text: String = char::decode_utf16(units)
                    .collect::<Result<_, _>>()
                    .map_err(|_| bad("not valid UTF-16 text"))?;
                self.out = text.into_bytes();
            }
        }
        Ok(n > 0 || !self.out.is_empty())
    }
}

impl<R: Read> Read for Decode<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.pos >= self.out.len() {
            if !self.fill()? {
                return Ok(0);
            }
        }
        let n = buf.len().min(self.out.len() - self.pos);
        buf[..n].copy_from_slice(&self.out[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

/// A CSV reader error, with the file's line number when known.
fn csv_error(e: csv::Error) -> anyhow::Error {
    let line = e.position().map(|p| p.line());
    let e = match e.kind() {
        csv::ErrorKind::Utf8 { .. } => anyhow::anyhow!(
            "the text is not valid UTF-8; pick the file's encoding (e.g. Windows-1252)"
        ),
        _ => anyhow::Error::new(e),
    };
    match line {
        Some(l) => e.context(format!("CSV line {l}")),
        None => e,
    }
}

/// Preview `path` for the import dialog. `delimiter` and `encoding` are the
/// user's choice; `None` detects them.
pub fn preview_csv(
    path: &Path,
    has_header: bool,
    delimiter: Option<u8>,
    encoding: Option<Encoding>,
) -> Result<CsvPreview> {
    let text = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let encoding = encoding.unwrap_or_else(|| Encoding::detect(&text[..text.len().min(8192)]));
    let delimiter = match delimiter {
        Some(d) => d,
        None => {
            let mut head = Vec::new();
            Decode::new(&text[..text.len().min(8192)], encoding)
                .read_to_end(&mut head)
                .ok();
            sniff_delimiter(&String::from_utf8_lossy(&head))
        }
    };
    let mut r = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(has_header)
        .flexible(true)
        .from_reader(Decode::new(&text[..], encoding));
    let headers = if has_header {
        r.headers()
            .map_err(csv_error)?
            .iter()
            .map(str::to_string)
            .collect()
    } else {
        Vec::new()
    };
    let rows: Vec<Vec<String>> = r
        .records()
        .take(8)
        .map(|rec| rec.map(|r| r.iter().map(str::to_string).collect()))
        .collect::<Result<_, _>>()
        .map_err(csv_error)?;
    let width = rows
        .iter()
        .map(Vec::len)
        .max()
        .unwrap_or(0)
        .max(headers.len());
    let headers = if headers.is_empty() {
        (1..=width).map(|i| format!("column {i}")).collect()
    } else {
        headers
    };
    Ok(CsvPreview {
        headers,
        rows,
        delimiter,
        encoding,
    })
}

/// Target column → source CSV column (by index), guessed from header names.
pub fn auto_map(csv_headers: &[String], columns: &[ColumnInfo]) -> Vec<Option<usize>> {
    columns
        .iter()
        .map(|c| {
            let norm = |s: &str| s.to_lowercase().replace([' ', '_', '-'], "");
            csv_headers.iter().position(|h| norm(h) == norm(&c.name))
        })
        .collect()
}

pub struct ImportSpec {
    pub connection: Uuid,
    pub engine: Engine,
    pub schema: Option<String>,
    pub table: String,
    pub columns: Vec<ColumnInfo>,
    /// Per target column: which CSV column feeds it (None = skip).
    pub mapping: Vec<Option<usize>>,
    pub has_header: bool,
    pub delimiter: u8,
    pub encoding: Encoding,
    pub empty_is_null: bool,
}

/// Placeholders the engine accepts in one statement (minus headroom).
fn param_limit(engine: Engine) -> usize {
    match engine {
        Engine::Postgres => 65_000,
        Engine::Mssql | Engine::Unknown => 2_000,
        Engine::Sqlite => 30_000,
    }
}

fn placeholder(engine: Engine, n: usize, col: &ColumnInfo) -> String {
    if is_binary_type(engine, &col.data_type) {
        // `0x…` text (as exported) decoded to the real bytes.
        return binary_placeholder(engine, n);
    }
    match engine {
        // Bind as text and let Postgres convert to the column type.
        Engine::Postgres => format!("${n}::text::{}", col.data_type),
        Engine::Mssql => format!("@P{n}"),
        Engine::Sqlite | Engine::Unknown => format!("?{n}"),
    }
}

/// The INSERT for a batch of `rows` rows into the mapped columns.
pub fn insert_sql(spec: &ImportSpec, rows: usize) -> String {
    let targets: Vec<&ColumnInfo> = spec
        .columns
        .iter()
        .zip(&spec.mapping)
        .filter(|(_, m)| m.is_some())
        .map(|(c, _)| c)
        .collect();
    let table = match spec.schema.as_deref() {
        Some(s) if !(spec.engine == Engine::Sqlite && s == "main") => {
            format!(
                "{}.{}",
                quote_ident(spec.engine, s),
                quote_ident(spec.engine, &spec.table)
            )
        }
        _ => quote_ident(spec.engine, &spec.table),
    };
    let cols: Vec<String> = targets
        .iter()
        .map(|c| quote_ident(spec.engine, &c.name))
        .collect();
    let mut n = 0;
    let tuples: Vec<String> = (0..rows)
        .map(|_| {
            let ph: Vec<String> = targets
                .iter()
                .map(|c| {
                    n += 1;
                    placeholder(spec.engine, n, c)
                })
                .collect();
            format!("({})", ph.join(", "))
        })
        .collect();
    format!(
        "INSERT INTO {table} ({}) VALUES {}",
        cols.join(", "),
        tuples.join(", ")
    )
}

async fn exec(client: &Client, session: Uuid, sql: String, params: Vec<Param>) -> Result<()> {
    let events = client
        .query_all(
            On::Session(session),
            &QueryRequest {
                sql,
                params,
                max_rows: Some(1),
                timeout_ms: None,
                single_statement: false,
            },
        )
        .await?;
    for e in events {
        if let QueryEvent::Error { message, .. } = e {
            bail!(message);
        }
    }
    Ok(())
}

/// Import a CSV file in one transaction. Returns the number of rows. Every
/// row must have as many fields as the header (or the first row). Setting
/// `cancel` stops the import and rolls it back.
pub async fn import_csv(
    client: &Client,
    path: &Path,
    spec: &ImportSpec,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64),
) -> Result<u64> {
    let sources: Vec<usize> = spec.mapping.iter().flatten().copied().collect();
    if sources.is_empty() {
        bail!("map at least one column");
    }
    // Target column per source, in the same (column) order as `sources`.
    let targets: Vec<&ColumnInfo> = spec
        .columns
        .iter()
        .zip(&spec.mapping)
        .filter(|(_, m)| m.is_some())
        .map(|(c, _)| c)
        .collect();
    let binary: Vec<bool> = targets
        .iter()
        .map(|c| is_binary_type(spec.engine, &c.data_type))
        .collect();
    let per_row = sources.len();
    let batch_rows = (param_limit(spec.engine) / per_row).clamp(1, 500);
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(spec.delimiter)
        .has_headers(spec.has_header)
        .flexible(true)
        .from_reader(Decode::new(std::io::BufReader::new(file), spec.encoding));
    let cancelled = || {
        if cancel.load(Ordering::Relaxed) {
            bail!("import cancelled");
        }
        Ok(())
    };

    let session = client.open_session(spec.connection).await?.id;
    let begin = if spec.engine == Engine::Mssql {
        "BEGIN TRANSACTION"
    } else {
        "BEGIN"
    };
    let result = async {
        exec(client, session, begin.into(), vec![]).await?;
        let mut done = 0u64;
        let mut batch: Vec<Param> = Vec::with_capacity(batch_rows * per_row);
        let mut in_batch = 0usize;
        let mut line = 0u64;
        let mut width = if spec.has_header {
            Some(reader.headers().map_err(csv_error)?.len())
        } else {
            None
        };
        let mut rec = csv::StringRecord::new();
        loop {
            cancelled()?;
            if !reader.read_record(&mut rec).map_err(csv_error)? {
                break;
            }
            // The physical line the record starts on (quoted fields can
            // hold line breaks).
            line = rec.position().map_or(line + 1, |p| p.line());
            let expected = *width.get_or_insert(rec.len());
            if rec.len() != expected {
                let what = if spec.has_header {
                    "the header"
                } else {
                    "the first row"
                };
                bail!(
                    "CSV line {line} has {} fields, {what} has {expected}",
                    rec.len()
                );
            }
            for (i, &src) in sources.iter().enumerate() {
                let v = rec.get(src).unwrap_or("");
                batch.push(if v.is_empty() && spec.empty_is_null {
                    Param::Null
                } else if binary[i] {
                    let hex = parse_hex(v).map_err(|e| {
                        anyhow::anyhow!("CSV line {line}, column {}: {e}", targets[i].name)
                    })?;
                    binary_param(spec.engine, &hex)
                } else {
                    Param::Text(v.to_string())
                });
            }
            in_batch += 1;
            if in_batch == batch_rows {
                cancelled()?;
                exec(
                    client,
                    session,
                    insert_sql(spec, in_batch),
                    std::mem::take(&mut batch),
                )
                .await
                .with_context(|| format!("rows ending at CSV line {line}"))?;
                done += in_batch as u64;
                in_batch = 0;
                progress(done);
            }
        }
        if in_batch > 0 {
            cancelled()?;
            exec(client, session, insert_sql(spec, in_batch), batch)
                .await
                .with_context(|| format!("rows ending at CSV line {line}"))?;
            done += in_batch as u64;
            progress(done);
        }
        cancelled()?;
        exec(client, session, "COMMIT".into(), vec![]).await?;
        Ok::<u64, anyhow::Error>(done)
    }
    .await;
    if result.is_err() {
        let _ = exec(client, session, "ROLLBACK".into(), vec![]).await;
    }
    let _ = client.close_session(session).await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, ty: &str) -> ColumnInfo {
        ColumnInfo {
            name: name.into(),
            ordinal: 0,
            data_type: ty.into(),
            nullable: true,
            default: None,
            primary_key: false,
        }
    }

    #[test]
    fn whole_export_reruns_only_the_statement_shown() {
        let pg = Engine::Postgres;
        let pick = |sql: &str, i, n, finished| statement_for_result(sql, pg, i, n, finished);
        // Writes before the SELECT are not repeated.
        assert_eq!(
            pick("INSERT INTO t VALUES (1);\nSELECT * FROM t", 0, 1, true).unwrap(),
            "SELECT * FROM t"
        );
        // The second result comes from the second read.
        let script = "SELECT 1 AS a;\nUPDATE t SET a = 1;\nSELECT *\n\nFROM t\nWHERE a = 1;";
        assert_eq!(pick(script, 0, 2, true).unwrap(), "SELECT 1 AS a");
        assert_eq!(
            pick(script, 1, 2, true).unwrap(),
            "SELECT *\n\nFROM t\nWHERE a = 1"
        );
        // Manual-commit runs start with BEGIN.
        assert_eq!(pick("BEGIN;\nSELECT 2", 0, 1, true).unwrap(), "SELECT 2");
        // A run that stopped early produced only the first results.
        assert_eq!(
            pick("SELECT 1;\nSELECT x FROM missing;\nSELECT 3", 0, 1, false).unwrap(),
            "SELECT 1"
        );
        // A statement that writes and returns rows can't be re-run, and the
        // results no longer line up with the reads.
        assert!(pick("INSERT INTO t VALUES (1) RETURNING *", 0, 1, true).is_err());
        assert!(pick("DELETE FROM t RETURNING *;\nSELECT * FROM t", 1, 2, true).is_err());
        assert!(pick("SELECT 1", 1, 1, true).is_err());
        // SQL Server needs no `;`: one chunk with two results is unclear.
        let ms = |sql: &str, i, n| statement_for_result(sql, Engine::Mssql, i, n, true);
        assert!(ms("SELECT 1\nSELECT 2", 1, 2).is_err());
        assert!(ms("INSERT INTO t VALUES (1)\nSELECT * FROM t", 0, 1).is_err());
        assert_eq!(ms("SELECT 1\nGO\nSELECT 2\nGO", 1, 2).unwrap(), "SELECT 2");
    }

    #[test]
    fn truncated_exports_say_rows_are_missing() {
        let p = Path::new("out.csv");
        let (text, warn) = export_notice(5, p, false, false);
        assert_eq!(text, "Exported 5 rows to out.csv");
        assert!(!warn);
        let (text, warn) = export_notice(100_000, p, true, false);
        assert!(warn);
        assert!(text.contains("rows are missing"), "{text}");
        assert!(text.contains("Whole query"), "{text}");
        let (text, warn) = export_notice(5_000_000, p, true, true);
        assert!(warn);
        assert!(text.contains("rows are missing"), "{text}");
        assert!(text.contains("limits.max_rows"), "{text}");
    }

    #[test]
    fn literals_per_dialect() {
        assert_eq!(
            sql_literal(Engine::Mssql, &Cell::Text("it's".into()), LogicalType::Text),
            "N'it''s'"
        );
        assert_eq!(
            sql_literal(Engine::Postgres, &Cell::Bool(true), LogicalType::Bool),
            "TRUE"
        );
        assert_eq!(
            sql_literal(
                Engine::Sqlite,
                &Cell::Text("ff00".into()),
                LogicalType::Bytes
            ),
            "X'ff00'"
        );
        assert_eq!(
            sql_literal(
                Engine::Postgres,
                &Cell::Text("1.50".into()),
                LogicalType::Decimal
            ),
            "1.50"
        );
        assert_eq!(
            sql_literal(Engine::Postgres, &Cell::Null, LogicalType::Int),
            "NULL"
        );
    }

    #[test]
    fn delimiter_sniffing() {
        assert_eq!(sniff_delimiter("a;b;c"), b';');
        assert_eq!(sniff_delimiter("a\tb"), b'\t');
        assert_eq!(sniff_delimiter("a,b"), b',');
        assert_eq!(sniff_delimiter("a|b\n1|2"), b'|');
        // A single column has no delimiter: a comma does no harm.
        assert_eq!(sniff_delimiter("name\nAda\nLinus"), b',');
        assert_eq!(sniff_delimiter(""), b',');
        // The delimiter that splits every line alike wins over one that only
        // appears in the header.
        assert_eq!(sniff_delimiter("id;name,full\n1;x\n2;y"), b';');
        assert_eq!(sniff_delimiter("a,b;c\n1,2;3"), b',');
    }

    fn decode(bytes: &[u8], enc: Encoding) -> std::io::Result<String> {
        let mut out = String::new();
        // A tiny inner buffer exercises characters split across reads.
        struct Slow<'a>(&'a [u8]);
        impl Read for Slow<'_> {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                let n = buf.len().min(self.0.len()).min(3);
                buf[..n].copy_from_slice(&self.0[..n]);
                self.0 = &self.0[n..];
                Ok(n)
            }
        }
        Decode::new(Slow(bytes), enc).read_to_string(&mut out)?;
        Ok(out)
    }

    #[test]
    fn encodings_are_detected_and_decoded() {
        let text = "naïve,€ 5,😀\n";
        let mut le = vec![0xFF, 0xFE];
        let mut be = vec![0xFE, 0xFF];
        for u in text.encode_utf16() {
            le.extend(u.to_le_bytes());
            be.extend(u.to_be_bytes());
        }
        assert_eq!(Encoding::detect(&le), Encoding::Utf16Le);
        assert_eq!(Encoding::detect(&be), Encoding::Utf16Be);
        assert_eq!(decode(&le, Encoding::Utf16Le).unwrap(), text);
        assert_eq!(decode(&be, Encoding::Utf16Be).unwrap(), text);
        // Without a BOM.
        let plain: Vec<u8> = "id,n\n1,a\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(Encoding::detect(&plain), Encoding::Utf16Le);
        assert_eq!(decode(&plain, Encoding::Utf16Le).unwrap(), "id,n\n1,a\n");
        let cp1252 = b"na\xEFve,\x80 5,\x93q\x94\n";
        assert_eq!(Encoding::detect(cp1252), Encoding::Windows1252);
        assert_eq!(
            decode(cp1252, Encoding::Windows1252).unwrap(),
            "naïve,€ 5,\u{201C}q\u{201D}\n"
        );
        let utf8 = "\u{feff}naïve\n".as_bytes();
        assert_eq!(Encoding::detect(utf8), Encoding::Utf8);
        assert_eq!(Encoding::detect(b"plain ascii"), Encoding::Utf8);
        // A truncated UTF-16 file is an error, not silently cut.
        assert!(decode(&le[..le.len() - 1], Encoding::Utf16Le).is_err());
    }

    #[test]
    fn mapping_matches_loose_names() {
        let m = auto_map(
            &["Customer ID".into(), "name".into(), "extra".into()],
            &[
                col("customer_id", "int"),
                col("Name", "text"),
                col("missing", "text"),
            ],
        );
        assert_eq!(m, vec![Some(0), Some(1), None]);
    }

    #[test]
    fn insert_statements_per_dialect() {
        let spec = ImportSpec {
            connection: Uuid::nil(),
            engine: Engine::Postgres,
            schema: Some("sales".into()),
            table: "Products".into(),
            columns: vec![
                col("id", "integer"),
                col("price", "numeric(10,2)"),
                col("skip", "text"),
            ],
            mapping: vec![Some(0), Some(1), None],
            has_header: true,
            delimiter: b',',
            encoding: Encoding::Utf8,
            empty_is_null: true,
        };
        assert_eq!(
            insert_sql(&spec, 2),
            "INSERT INTO sales.\"Products\" (id, price) VALUES ($1::text::integer, $2::text::numeric(10,2)), ($3::text::integer, $4::text::numeric(10,2))"
        );
        let spec = ImportSpec {
            engine: Engine::Mssql,
            ..spec
        };
        assert_eq!(
            insert_sql(&spec, 1),
            "INSERT INTO [sales].[Products] ([id], [price]) VALUES (@P1, @P2)"
        );
    }

    fn column(name: &str, logical: LogicalType) -> Column {
        Column {
            name: name.into(),
            type_name: String::new(),
            logical,
        }
    }

    #[test]
    fn insert_literals_quote_what_is_not_a_number() {
        let dec = |s: &str| {
            sql_literal(
                Engine::Postgres,
                &Cell::Text(s.into()),
                LogicalType::Decimal,
            )
        };
        assert_eq!(dec("NaN"), "'NaN'");
        assert_eq!(dec("Infinity"), "'Infinity'");
        assert_eq!(dec("-inf"), "'-inf'");
        assert_eq!(dec("-12.50"), "-12.50");
        assert_eq!(dec("1.5e+3"), "1.5e+3");
        assert_eq!(dec("12345678901234567890.123"), "12345678901234567890.123");
        assert_eq!(dec("1."), "'1.'");
        // SQL Server money and decimal arrive as exact text.
        assert_eq!(
            sql_literal(
                Engine::Mssql,
                &Cell::Text("1.1000".into()),
                LogicalType::Decimal
            ),
            "1.1000"
        );
    }

    #[test]
    fn excel_cells_keep_values_exact() {
        let v = |c: Cell, l| xlsx_value(&c, l);
        let t = |s: &str, l| xlsx_value(&Cell::Text(s.into()), l);
        assert_eq!(v(Cell::Int(42), LogicalType::Int), XlsxValue::Number(42.0));
        assert_eq!(
            v(Cell::Int(999_999_999_999_999), LogicalType::Int),
            XlsxValue::Number(999_999_999_999_999.0)
        );
        assert_eq!(
            v(Cell::Int(1_234_567_890_123_456_789), LogicalType::Int),
            XlsxValue::Text("1234567890123456789".into())
        );
        assert_eq!(t("1.50", LogicalType::Decimal), XlsxValue::Number(1.5));
        assert_eq!(t("1.1000", LogicalType::Decimal), XlsxValue::Number(1.1));
        assert_eq!(t("-0.001", LogicalType::Decimal), XlsxValue::Number(-0.001));
        assert_eq!(
            t("1234567890.1234567", LogicalType::Decimal),
            XlsxValue::Text("1234567890.1234567".into())
        );
        assert_eq!(
            t("NaN", LogicalType::Decimal),
            XlsxValue::Text("NaN".into())
        );
        let date = |s: &str, l| match t(s, l) {
            XlsxValue::Date(d, k) => Some((d, k)),
            _ => None,
        };
        // 2024-01-31 is Excel day 45322.
        assert_eq!(
            date("2024-01-31", LogicalType::Date),
            Some((45322.0, DateKind::Date))
        );
        assert_eq!(
            date("2024-01-31T12:00:00", LogicalType::Timestamp),
            Some((45322.5, DateKind::DateTime))
        );
        assert_eq!(
            date("2024-01-31 12:00:00", LogicalType::Timestamp).map(|d| d.1),
            Some(DateKind::DateTime)
        );
        assert_eq!(
            date("2024-01-31T13:45:12.997", LogicalType::Timestamp).map(|d| d.1),
            Some(DateKind::DateTimeMs)
        );
        assert_eq!(
            date("2024-01-31T13:45:12.5000000", LogicalType::Timestamp).map(|d| d.1),
            Some(DateKind::DateTimeMs)
        );
        assert_eq!(
            date("12:00:00", LogicalType::Time),
            Some((0.5, DateKind::Time))
        );
        // Finer than milliseconds, outside Excel's years, time zones and
        // special values stay text.
        for (s, l) in [
            ("2024-01-31T13:45:12.1234567", LogicalType::Timestamp),
            ("13:45:12.1234567", LogicalType::Time),
            ("1899-12-31", LogicalType::Date),
            ("infinity", LogicalType::Date),
            ("2024-01-31 BC", LogicalType::Date),
            ("2024-01-31T13:45:12+01:00", LogicalType::TimestampTz),
            ("2024-01-31T13:45:12+01:00", LogicalType::Timestamp),
        ] {
            assert_eq!(t(s, l), XlsxValue::Text(s.into()), "{s}");
        }
    }

    #[test]
    fn json_columns_are_embedded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("j.json");
        let columns = vec![
            column("j", LogicalType::Json),
            column("t", LogicalType::Text),
        ];
        let mut ex = Exporter::create(
            &path,
            Format::Json,
            columns,
            Engine::Postgres,
            &ExportOptions::default(),
        )
        .unwrap();
        ex.row(&[
            Cell::Text(r#"{"k": 1, "big": 123456789012345678901234567890}"#.into()),
            Cell::Text("{\"k\":1}".into()),
        ])
        .unwrap();
        ex.row(&[Cell::Text("not json".into()), Cell::Null])
            .unwrap();
        ex.finish().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        // The number is written as the database sent it, digit for digit.
        assert!(text.contains("123456789012345678901234567890"), "{text}");
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json[0]["j"]["k"], 1);
        assert_eq!(json[0]["t"], "{\"k\":1}");
        assert_eq!(json[1]["j"], "not json");
    }

    #[test]
    fn csv_bom_and_null_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("n.csv");
        let opts = ExportOptions {
            csv_bom: true,
            csv_null: "NULL".into(),
            ..Default::default()
        };
        let columns = vec![
            column("a", LogicalType::Text),
            column("b", LogicalType::Text),
        ];
        let mut ex =
            Exporter::create(&path, Format::Csv, columns, Engine::Postgres, &opts).unwrap();
        ex.row(&[Cell::Null, Cell::Text("".into())]).unwrap();
        ex.row(&[Cell::Text("é".into()), Cell::Null]).unwrap();
        ex.finish().unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            "\u{feff}a,b\nNULL,\né,NULL\n".as_bytes()
        );
    }

    #[test]
    fn unfinished_exports_leave_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.csv");
        let columns = vec![column("a", LogicalType::Int)];
        for f in Format::ALL {
            let path = path.with_extension(f.extension());
            let opts = ExportOptions::default();
            let mut ex =
                Exporter::create(&path, f, columns.clone(), Engine::Sqlite, &opts).unwrap();
            ex.row(&[Cell::Int(1)]).unwrap();
            drop(ex);
            assert!(!path.exists(), "{f:?}");
        }
        // An existing file stays as it was until the export completes.
        std::fs::write(&path, "old").unwrap();
        let opts = ExportOptions::default();
        let mut ex =
            Exporter::create(&path, Format::Csv, columns.clone(), Engine::Sqlite, &opts).unwrap();
        ex.row(&[Cell::Int(1)]).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old");
        drop(ex);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old");
        let ex = Exporter::create(&path, Format::Csv, columns, Engine::Sqlite, &opts).unwrap();
        ex.finish().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a\n");
        // No temporary files are left next to it.
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("out.csv")]);
    }

    #[test]
    fn insert_target_is_the_source_table() {
        let pg = Engine::Postgres;
        assert_eq!(
            insert_target(
                "SELECT * FROM sales.\"Products\" WHERE id > 1",
                pg,
                0,
                1,
                true
            ),
            Some("sales.\"Products\"".into())
        );
        assert_eq!(
            insert_target("SELECT id FROM orders", Engine::Mssql, 0, 1, true),
            Some("[orders]".into())
        );
        assert_eq!(
            insert_target("SELECT * FROM a JOIN b ON a.id = b.id", pg, 0, 1, true),
            None
        );
        assert_eq!(insert_target("SELECT 1", pg, 0, 1, true), None);
    }

    #[test]
    fn exports_every_format() {
        let dir = tempfile::tempdir().unwrap();
        let columns = vec![
            Column {
                name: "id".into(),
                type_name: "int".into(),
                logical: LogicalType::Int,
            },
            Column {
                name: "name".into(),
                type_name: "text".into(),
                logical: LogicalType::Text,
            },
        ];
        let rows = [
            vec![Cell::Int(1), Cell::Text("a,\"b\"".into())],
            vec![Cell::Int(2), Cell::Null],
        ];
        for f in Format::ALL {
            let path = dir.path().join(format!("out.{}", f.extension()));
            let opts = ExportOptions {
                table: Some("t".into()),
                ..Default::default()
            };
            let mut ex =
                Exporter::create(&path, f, columns.clone(), Engine::Postgres, &opts).unwrap();
            for r in &rows {
                ex.row(r).unwrap();
            }
            assert_eq!(ex.finish().unwrap(), 2);
            assert!(std::fs::metadata(&path).unwrap().len() > 0);
        }
        let csv = std::fs::read_to_string(dir.path().join("out.csv")).unwrap();
        assert_eq!(csv, "id,name\n1,\"a,\"\"b\"\"\"\n2,\n");
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join("out.json")).unwrap())
                .unwrap();
        assert_eq!(
            json,
            serde_json::json!([{"id": 1, "name": "a,\"b\""}, {"id": 2, "name": null}])
        );
        let sql = std::fs::read_to_string(dir.path().join("out.sql")).unwrap();
        assert_eq!(
            sql,
            "INSERT INTO t (id, name) VALUES (1, 'a,\"b\"');\nINSERT INTO t (id, name) VALUES (2, NULL);\n"
        );
    }

    #[test]
    fn xlsx_export_writes_a_workbook() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.xlsx");
        let columns = vec![
            Column {
                name: "id".into(),
                type_name: "int4".into(),
                logical: LogicalType::Int,
            },
            Column {
                name: "name".into(),
                type_name: "text".into(),
                logical: LogicalType::Text,
            },
        ];
        let opts = ExportOptions::default();
        let mut ex =
            Exporter::create(&path, Format::Xlsx, columns, Engine::Postgres, &opts).unwrap();
        ex.row(&[Cell::Int(1), Cell::Text("a".into())]).unwrap();
        ex.row(&[Cell::Int(2), Cell::Null]).unwrap();
        assert_eq!(ex.finish().unwrap(), 2);
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.starts_with(b"PK"), "an xlsx file is a zip archive");
    }
}
