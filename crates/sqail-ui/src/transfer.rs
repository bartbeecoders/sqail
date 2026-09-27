//! Export results to CSV / JSON / Excel / SQL, streaming row by row, and
//! import CSV files into a table. No UI here; the app drives these on the
//! background runtime.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use futures::StreamExt;
use sqail_client::proto::{
    Column, ColumnInfo, Engine, LogicalType, Param, QueryEvent, QueryRequest,
};
use sqail_client::{Client, On};
use uuid::Uuid;

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
        path: PathBuf,
    },
    Sql {
        out: BufWriter<File>,
        table: String,
    },
}

/// Writes one result set to a file, a row at a time.
pub struct Exporter {
    sink: Sink,
    columns: Vec<Column>,
    engine: Engine,
    pub rows: u64,
}

impl Exporter {
    /// `table` names the target of SQL INSERT exports.
    pub fn create(
        path: &Path,
        format: Format,
        columns: Vec<Column>,
        engine: Engine,
        table: &str,
    ) -> Result<Self> {
        let file = || File::create(path).with_context(|| format!("creating {}", path.display()));
        let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
        let sink = match format {
            Format::Csv => {
                let mut w = csv::Writer::from_writer(BufWriter::new(file()?));
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
                let sheet = book.add_worksheet_with_constant_memory();
                write_xlsx_header(sheet, &names)?;
                Sink::Xlsx {
                    book,
                    sheet: 0,
                    row: 1,
                    path: path.to_path_buf(),
                }
            }
            Format::Sql => {
                let cols: Vec<String> = names.iter().map(|n| quote_ident(engine, n)).collect();
                let table = format!("INSERT INTO {table} ({}) VALUES ", cols.join(", "));
                Sink::Sql {
                    out: BufWriter::new(file()?),
                    table,
                }
            }
        };
        Ok(Self {
            sink,
            columns,
            engine,
            rows: 0,
        })
    }

    pub fn row(&mut self, cells: &[Cell]) -> Result<()> {
        self.rows += 1;
        match &mut self.sink {
            Sink::Csv(w) => {
                let fields: Vec<String> = cells
                    .iter()
                    .zip(&self.columns)
                    .map(|(c, col)| {
                        if c.is_null() {
                            String::new()
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
                book, sheet, row, ..
            } => {
                if *row > XLSX_ROWS_PER_SHEET {
                    let ws = book.add_worksheet_with_constant_memory();
                    let names: Vec<&str> = self.columns.iter().map(|c| c.name.as_str()).collect();
                    write_xlsx_header(ws, &names)?;
                    *sheet += 1;
                    *row = 1;
                }
                let ws = book.worksheet_from_index(*sheet)?;
                for (i, (c, col)) in cells.iter().zip(&self.columns).enumerate() {
                    let i = i as u16;
                    match c {
                        Cell::Null => {}
                        Cell::Bool(b) => {
                            ws.write_boolean(*row, i, *b)?;
                        }
                        Cell::Int(v) => {
                            ws.write_number(*row, i, *v as f64)?;
                        }
                        Cell::Float(v) => {
                            ws.write_number(*row, i, *v)?;
                        }
                        Cell::Text(_) => {
                            // Excel cells hold at most 32,767 characters.
                            let text = c.display(col.logical);
                            let text: String = if text.chars().count() > 32_767 {
                                text.chars().take(32_767).collect()
                            } else {
                                text.into_owned()
                            };
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

    pub fn finish(self) -> Result<u64> {
        match self.sink {
            Sink::Csv(mut w) => w.flush()?,
            Sink::Json { mut out, .. } => {
                out.write_all(b"\n]\n")?;
                out.flush()?;
            }
            Sink::Xlsx { mut book, path, .. } => book.save(&path)?,
            Sink::Sql { mut out, .. } => out.flush()?,
        }
        Ok(self.rows)
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
            Engine::Sqlite => format!("X'{s}'"),
        },
        Cell::Text(s) if logical == LogicalType::Decimal && s.parse::<f64>().is_ok() => {
            s.to_string()
        }
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

/// Run `sql` on a pooled connection and stream its first result set to
/// `path` without holding the rows in memory. `progress` gets row counts.
pub async fn export_query(
    client: &Client,
    connection: Uuid,
    engine: Engine,
    sql: &str,
    path: &Path,
    format: Format,
    mut progress: impl FnMut(u64),
) -> Result<u64> {
    let req = QueryRequest {
        sql: sql.to_string(),
        params: vec![],
        // As many as the service allows.
        max_rows: Some(u64::MAX),
        timeout_ms: None,
    };
    let mut stream = client.query(On::Connection(connection), &req).await?;
    let mut exporter: Option<Exporter> = None;
    let mut error = None;
    while let Some(ev) = stream.events.next().await {
        match ev? {
            QueryEvent::ResultStart { index: 0, columns } => {
                exporter = Some(Exporter::create(path, format, columns, engine, "exported")?);
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
            QueryEvent::Error { message, .. } => error = Some(message),
            _ => {}
        }
    }
    if let Some(e) = error {
        bail!(e);
    }
    match exporter {
        Some(ex) => ex.finish(),
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
}

/// Guess the delimiter from the first line: the most frequent of , ; tab |.
pub fn sniff_delimiter(first_line: &str) -> u8 {
    b",;\t|"
        .iter()
        .copied()
        .max_by_key(|&d| first_line.bytes().filter(|&b| b == d).count())
        .unwrap_or(b',')
}

pub fn preview_csv(path: &Path, has_header: bool) -> Result<CsvPreview> {
    let text = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let first = String::from_utf8_lossy(&text[..text.len().min(4096)]);
    let delimiter = sniff_delimiter(first.lines().next().unwrap_or(""));
    let mut r = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(has_header)
        .flexible(true)
        .from_reader(&text[..]);
    let headers = if has_header {
        r.headers()?.iter().map(str::to_string).collect()
    } else {
        Vec::new()
    };
    let rows: Vec<Vec<String>> = r
        .records()
        .take(8)
        .map(|rec| rec.map(|r| r.iter().map(str::to_string).collect()))
        .collect::<Result<_, _>>()?;
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
    pub empty_is_null: bool,
}

/// Placeholders the engine accepts in one statement (minus headroom).
fn param_limit(engine: Engine) -> usize {
    match engine {
        Engine::Postgres => 65_000,
        Engine::Mssql => 2_000,
        Engine::Sqlite => 30_000,
    }
}

fn placeholder(engine: Engine, n: usize, col: &ColumnInfo) -> String {
    match engine {
        // Bind as text and let Postgres convert to the column type.
        Engine::Postgres => format!("${n}::text::{}", col.data_type),
        Engine::Mssql => format!("@P{n}"),
        Engine::Sqlite => format!("?{n}"),
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

/// Import a CSV file in one transaction. Returns the number of rows.
pub async fn import_csv(
    client: &Client,
    path: &Path,
    spec: &ImportSpec,
    mut progress: impl FnMut(u64),
) -> Result<u64> {
    let sources: Vec<usize> = spec.mapping.iter().flatten().copied().collect();
    if sources.is_empty() {
        bail!("map at least one column");
    }
    let per_row = sources.len();
    let batch_rows = (param_limit(spec.engine) / per_row).clamp(1, 500);
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(spec.delimiter)
        .has_headers(spec.has_header)
        .flexible(true)
        .from_reader(std::io::BufReader::new(file));

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
        let mut line = if spec.has_header { 1u64 } else { 0 };
        for rec in reader.records() {
            line += 1;
            let rec = rec.with_context(|| format!("CSV line {line}"))?;
            for &src in &sources {
                let v = rec.get(src).unwrap_or("");
                batch.push(if v.is_empty() && spec.empty_is_null {
                    Param::Null
                } else {
                    Param::Text(v.to_string())
                });
            }
            in_batch += 1;
            if in_batch == batch_rows {
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
            exec(client, session, insert_sql(spec, in_batch), batch)
                .await
                .with_context(|| format!("rows ending at CSV line {line}"))?;
            done += in_batch as u64;
            progress(done);
        }
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
            let mut ex =
                Exporter::create(&path, f, columns.clone(), Engine::Postgres, "t").unwrap();
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
}
