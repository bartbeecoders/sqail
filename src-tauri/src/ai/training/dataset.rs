//! Build a SQL-query training dataset from a connected database.
//!
//! Output is JSONL (one JSON object per line) in the instruct-tuning
//! shape `{instruction, input, output}` — directly consumable by
//! `trl.SFTTrainer` with `formatting_func`, or by any PEFT recipe.
//!
//! Strategy:
//!
//! * **Schema block** — one tiny per-table "describe-the-schema" example
//!   plus one SELECT-columns example. Anchors the model in the exact
//!   table/column vocabulary of this database.
//! * **Metadata block** — if the user has generated object metadata for a
//!   table (via the existing metadata feature), we turn each
//!   description + example usage into a Q&A pair.
//! * **Sample-data block** — a handful of SELECT variants per table,
//!   with concrete values from `SELECT * LIMIT N` used in WHERE clauses.
//! * **Join block** — for every PK ← FK relationship we can detect from
//!   column-name conventions, emit a join template.
//!
//! None of this is cutting-edge — deliberately. It's the boring,
//! deterministic corpus that consistently improves per-schema SQL
//! generation quality when LoRA-fine-tuned on a base coder model.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::Column as _;
use sqlx::Row;
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::db::connections::Driver;
use crate::metadata::ObjectMetadata;
use crate::pool::DbPool;
use crate::schema::{self, ColumnInfo, TableInfo};

/// User-configurable knobs, surfaced in the frontend form.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetOptions {
    /// Comma-separated list of schemas to include. Empty = all non-system.
    #[serde(default)]
    pub schemas: Vec<String>,
    /// Max number of sample rows to pull per table — caps both the
    /// SELECT preview and the number of WHERE-clause examples derived.
    #[serde(default = "default_sample_rows")]
    pub sample_rows: u32,
    /// Upper bound on the number of tables processed. Protects large DBs
    /// from generating a multi-million-row corpus by accident.
    #[serde(default = "default_max_tables")]
    pub max_tables: u32,
    /// Include rows from `metadata.json` as Q&A pairs.
    #[serde(default = "default_true")]
    pub include_metadata: bool,
    /// Include sample SELECT/WHERE pairs derived from actual data.
    #[serde(default = "default_true")]
    pub include_samples: bool,
    /// Include heuristic join templates (PK ← FK by name convention).
    #[serde(default = "default_true")]
    pub include_joins: bool,
}

fn default_sample_rows() -> u32 {
    5
}
fn default_max_tables() -> u32 {
    200
}
fn default_true() -> bool {
    true
}

impl Default for DatasetOptions {
    fn default() -> Self {
        Self {
            schemas: Vec::new(),
            sample_rows: default_sample_rows(),
            max_tables: default_max_tables(),
            include_metadata: true,
            include_samples: true,
            include_joins: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetStats {
    pub table_count: u32,
    pub example_count: u32,
    pub file_path: String,
    pub size_bytes: u64,
}

/// Root training directory for a given connection.
pub fn training_dir(app_data: &Path, connection_id: &str) -> PathBuf {
    app_data
        .join("inline-ai")
        .join("training")
        .join(connection_id)
}

/// One example line of the JSONL file.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Example {
    instruction: String,
    input: String,
    output: String,
}

/// Build the dataset. Writes to `<app_data>/inline-ai/training/<conn>/<job>/dataset.jsonl`
/// and returns stats. The pool must already be connected to the target
/// database — callers go through the existing `pools` map in `AppState`.
#[allow(clippy::too_many_arguments)]
pub async fn build(
    pool: DbPool,
    driver: &Driver,
    output_path: &Path,
    opts: &DatasetOptions,
    metadata: &[ObjectMetadata],
    connection_id: &str,
    dialect_label: &str,
    progress: impl Fn(u32, u32, &str) + Send,
) -> Result<DatasetStats, String> {
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("mkdir: {e}"))?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(output_path)
        .await
        .map_err(|e| format!("open output: {e}"))?;

    let mut examples: u32 = 0;

    // Schema-level preamble: a small "what database is this?" example.
    write_example(
        &mut file,
        &Example {
            instruction: "What database engine and dialect is this?".into(),
            input: String::new(),
            output: format!(
                "This is a {dialect_label} database. Queries use the {dialect_label} SQL dialect."
            ),
        },
    )
    .await?;
    examples += 1;

    // Collect (schema, tables) pairs — honouring the optional filter.
    let schemas = resolve_schemas(pool.clone(), driver, &opts.schemas).await?;
    let mut table_entries: Vec<TableInfo> = Vec::new();
    for schema_name in &schemas {
        let tables = schema::list_tables(pool.clone(), driver, schema_name)
            .await
            .unwrap_or_default();
        for t in tables {
            if t.table_type != "table" {
                continue;
            }
            table_entries.push(t);
            if table_entries.len() as u32 >= opts.max_tables {
                break;
            }
        }
        if table_entries.len() as u32 >= opts.max_tables {
            break;
        }
    }

    let total = table_entries.len() as u32;
    progress(0, total, "Building dataset");

    // Index of every column so we can build join templates later.
    let mut all_cols: Vec<(TableInfo, Vec<ColumnInfo>)> = Vec::with_capacity(table_entries.len());
    for (i, t) in table_entries.iter().enumerate() {
        let columns = schema::list_columns(pool.clone(), driver, &t.schema, &t.name)
            .await
            .unwrap_or_default();
        all_cols.push((t.clone(), columns));
        progress((i as u32) + 1, total, &format!("Reading {}.{}", t.schema, t.name));
    }

    for (i, (table, columns)) in all_cols.iter().enumerate() {
        let qualified = qualify(driver, &table.schema, &table.name);

        // Describe-columns example.
        let describe = describe_columns(&qualified, columns);
        write_example(
            &mut file,
            &Example {
                instruction: format!("Describe the columns of the {qualified} table."),
                input: String::new(),
                output: describe.clone(),
            },
        )
        .await?;
        examples += 1;

        // SELECT-all example.
        write_example(
            &mut file,
            &Example {
                instruction: format!(
                    "Write a SQL query that selects every row from the {qualified} table."
                ),
                input: String::new(),
                output: format!("SELECT * FROM {qualified};"),
            },
        )
        .await?;
        examples += 1;

        // Count example.
        write_example(
            &mut file,
            &Example {
                instruction: format!("How many rows are in the {qualified} table?"),
                input: String::new(),
                output: format!("SELECT COUNT(*) FROM {qualified};"),
            },
        )
        .await?;
        examples += 1;

        // Schema-grounded NL→SQL examples. These are the shape the model
        // was missing: `input` carries the authoritative column list and
        // `output` references only columns from that list. Without these,
        // the model memorises SELECT * over the table name but never
        // learns to read a column list and pick from it — so at inference
        // it fabricates columns ("mfg_date", "exp_date", …) that sound
        // right for the table's semantic role.
        for ex in derive_nl_to_sql_examples(&qualified, &table.name, columns) {
            write_example(&mut file, &ex).await?;
            examples += 1;
        }

        // Contrastive "only these columns exist" example — teaches the
        // model the exclusion rule (what not to do), not just inclusion.
        if let Some(ex) = derive_exclusion_example(&qualified, &table.name, columns) {
            write_example(&mut file, &ex).await?;
            examples += 1;
        }

        // Metadata-derived examples.
        if opts.include_metadata {
            if let Some(meta) = find_metadata(metadata, connection_id, &table.schema, &table.name) {
                if !meta.metadata.description.trim().is_empty() {
                    write_example(
                        &mut file,
                        &Example {
                            instruction: format!("What does the {qualified} table store?"),
                            input: String::new(),
                            output: meta.metadata.description.clone(),
                        },
                    )
                    .await?;
                    examples += 1;
                }
                if !meta.metadata.example_usage.trim().is_empty() {
                    write_example(
                        &mut file,
                        &Example {
                            instruction: format!(
                                "Give an example query that uses the {qualified} table."
                            ),
                            input: String::new(),
                            output: meta.metadata.example_usage.clone(),
                        },
                    )
                    .await?;
                    examples += 1;
                }
                for col in &meta.metadata.columns {
                    if col.description.trim().is_empty() {
                        continue;
                    }
                    write_example(
                        &mut file,
                        &Example {
                            instruction: format!(
                                "What does the '{}' column of {qualified} mean?",
                                col.name
                            ),
                            input: String::new(),
                            output: col.description.clone(),
                        },
                    )
                    .await?;
                    examples += 1;
                }
            }
        }

        // Sample-data-derived WHERE examples.
        if opts.include_samples {
            match fetch_samples(pool.clone(), driver, &qualified, opts.sample_rows).await {
                Ok(samples) => {
                    for ex in derive_where_examples(&qualified, columns, &samples) {
                        write_example(&mut file, &ex).await?;
                        examples += 1;
                    }
                }
                Err(e) => {
                    log::warn!("sample fetch failed for {qualified}: {e}");
                }
            }
        }

        if i as u32 + 1 < total {
            progress(
                i as u32 + 1,
                total,
                &format!("Wrote examples for {qualified}"),
            );
        }
    }

    // Join heuristics across the whole table set.
    if opts.include_joins {
        for ex in derive_join_examples(driver, &all_cols) {
            write_example(&mut file, &ex).await?;
            examples += 1;
        }
    }

    file.flush().await.map_err(|e| format!("flush: {e}"))?;
    drop(file);

    let size_bytes = fs::metadata(output_path)
        .await
        .map(|m| m.len())
        .unwrap_or(0);

    progress(total, total, "Done");

    Ok(DatasetStats {
        table_count: total,
        example_count: examples,
        file_path: output_path.to_string_lossy().to_string(),
        size_bytes,
    })
}

/// Which schemas to iterate. Empty user selection = all non-system.
async fn resolve_schemas(
    pool: DbPool,
    driver: &Driver,
    requested: &[String],
) -> Result<Vec<String>, String> {
    if !requested.is_empty() {
        return Ok(requested.to_vec());
    }
    let all = schema::list_schemas(pool, driver).await?;
    Ok(all.into_iter().map(|s| s.name).collect())
}

/// Produce a dialect-safe qualified name like `"schema"."table"` /
/// `` `schema`.`table` `` / `[schema].[table]`.
fn qualify(driver: &Driver, schema: &str, table: &str) -> String {
    match driver {
        Driver::Mysql => {
            if schema.is_empty() {
                format!("`{}`", table)
            } else {
                format!("`{}`.`{}`", schema, table)
            }
        }
        Driver::Mssql => {
            if schema.is_empty() {
                format!("[{}]", table)
            } else {
                format!("[{}].[{}]", schema, table)
            }
        }
        Driver::Sqlite => {
            // SQLite doesn't really have schemas in the same sense.
            format!("\"{}\"", table)
        }
        _ => {
            if schema.is_empty() {
                format!("\"{}\"", table)
            } else {
                format!("\"{}\".\"{}\"", schema, table)
            }
        }
    }
}

fn describe_columns(qualified: &str, columns: &[ColumnInfo]) -> String {
    let mut s = format!("The {qualified} table has {} columns:\n", columns.len());
    for c in columns {
        let nullable = if c.is_nullable { "NULL" } else { "NOT NULL" };
        let pk = if c.is_primary_key { ", PRIMARY KEY" } else { "" };
        let _ = writeln!(
            &mut s,
            "- {} {} {}{}",
            c.name, c.data_type, nullable, pk
        );
    }
    s
}

fn find_metadata<'a>(
    metadata: &'a [ObjectMetadata],
    connection_id: &str,
    schema: &str,
    table: &str,
) -> Option<&'a ObjectMetadata> {
    metadata.iter().find(|m| {
        m.connection_id == connection_id
            && m.schema_name == schema
            && m.object_name == table
    })
}

/// Pull up to `n` sample rows. Columns + values come back stringified.
async fn fetch_samples(
    pool: DbPool,
    driver: &Driver,
    qualified: &str,
    n: u32,
) -> Result<Vec<Vec<(String, String)>>, String> {
    if n == 0 {
        return Ok(Vec::new());
    }
    let sql = match driver {
        Driver::Mssql => format!("SELECT TOP {n} * FROM {qualified}"),
        _ => format!("SELECT * FROM {qualified} LIMIT {n}"),
    };

    match pool {
        DbPool::Postgres(p) => fetch_pg(&p, &sql).await,
        DbPool::Mysql(p) => fetch_mysql(&p, &sql).await,
        DbPool::Sqlite(p) => fetch_sqlite(&p, &sql).await,
        // MSSQL/DbService: out of scope for sample-based examples; schema
        // + metadata + join heuristics still apply.
        _ => Ok(Vec::new()),
    }
}

async fn fetch_pg(
    pool: &sqlx::PgPool,
    sql: &str,
) -> Result<Vec<Vec<(String, String)>>, String> {
    let rows = sqlx::query(sql).fetch_all(pool).await.map_err(|e| e.to_string())?;
    Ok(rows
        .iter()
        .map(|r| {
            (0..r.len())
                .map(|i| {
                    let name = r.column(i).name().to_string();
                    let val = pg_value(r, i);
                    (name, val)
                })
                .collect()
        })
        .collect())
}

async fn fetch_mysql(
    pool: &sqlx::MySqlPool,
    sql: &str,
) -> Result<Vec<Vec<(String, String)>>, String> {
    let rows = sqlx::query(sql).fetch_all(pool).await.map_err(|e| e.to_string())?;
    Ok(rows
        .iter()
        .map(|r| {
            (0..r.len())
                .map(|i| {
                    let name = r.column(i).name().to_string();
                    let val = mysql_value(r, i);
                    (name, val)
                })
                .collect()
        })
        .collect())
}

async fn fetch_sqlite(
    pool: &sqlx::SqlitePool,
    sql: &str,
) -> Result<Vec<Vec<(String, String)>>, String> {
    let rows = sqlx::query(sql).fetch_all(pool).await.map_err(|e| e.to_string())?;
    Ok(rows
        .iter()
        .map(|r| {
            (0..r.len())
                .map(|i| {
                    let name = r.column(i).name().to_string();
                    let val = sqlite_value(r, i);
                    (name, val)
                })
                .collect()
        })
        .collect())
}

fn pg_value(row: &sqlx::postgres::PgRow, idx: usize) -> String {
    use sqlx::Column;
    use sqlx::TypeInfo;
    let type_name = row.column(idx).type_info().name();
    match type_name {
        "INT2" | "INT4" | "INT8" | "OID" => row
            .try_get::<i64, _>(idx)
            .ok()
            .map(|v| v.to_string())
            .unwrap_or_default(),
        "FLOAT4" | "FLOAT8" | "NUMERIC" => row
            .try_get::<f64, _>(idx)
            .ok()
            .map(|v| v.to_string())
            .unwrap_or_default(),
        "BOOL" => row
            .try_get::<bool, _>(idx)
            .ok()
            .map(|v| v.to_string())
            .unwrap_or_default(),
        _ => row
            .try_get::<String, _>(idx)
            .ok()
            .unwrap_or_default(),
    }
}

fn mysql_value(row: &sqlx::mysql::MySqlRow, idx: usize) -> String {
    if let Ok(v) = row.try_get::<String, _>(idx) {
        return v;
    }
    if let Ok(v) = row.try_get::<i64, _>(idx) {
        return v.to_string();
    }
    if let Ok(v) = row.try_get::<f64, _>(idx) {
        return v.to_string();
    }
    if let Ok(v) = row.try_get::<bool, _>(idx) {
        return v.to_string();
    }
    String::new()
}

fn sqlite_value(row: &sqlx::sqlite::SqliteRow, idx: usize) -> String {
    if let Ok(v) = row.try_get::<String, _>(idx) {
        return v;
    }
    if let Ok(v) = row.try_get::<i64, _>(idx) {
        return v.to_string();
    }
    if let Ok(v) = row.try_get::<f64, _>(idx) {
        return v.to_string();
    }
    String::new()
}

/// Short, schema-qualified column list suitable for the `input` field of
/// an instruction-tuning example. Kept compact so it fits in the model's
/// context window even on very wide tables.
fn column_list_input(qualified: &str, columns: &[ColumnInfo]) -> String {
    let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
    format!(
        "Schema for {qualified}: columns = {}.\n\
         Only these columns exist on this table.",
        names.join(", ")
    )
}

/// Build natural-language → SELECT examples with the authoritative column
/// list in `input`. This is the shape the corpus was previously missing:
/// without it the model learns the table's name but not *how to read a
/// column list and pick from it*.
///
/// Emits, for each table:
/// * "List all …" with every column projected.
/// * "Show the first 10 …" with a LIMIT / TOP variant.
/// * "Show each row's <col> and <col>" for the first two real columns.
/// * "Count the number of …".
/// * For each primary-key column: "Find the … with <pk> = <value>".
fn derive_nl_to_sql_examples(
    qualified: &str,
    table_name: &str,
    columns: &[ColumnInfo],
) -> Vec<Example> {
    let mut out = Vec::new();
    if columns.is_empty() {
        return out;
    }

    let stem_plural = pluralize_label(table_name);
    let stem_singular = singular(table_name).replace('_', " ");
    let input = column_list_input(qualified, columns);

    let col_names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
    let all_cols = col_names.join(", ");

    // "List all …" — every column, no WHERE. Forces the model to project
    // real columns rather than falling back on SELECT *.
    out.push(Example {
        instruction: format!("List all {stem_plural}."),
        input: input.clone(),
        output: format!("SELECT {all_cols} FROM {qualified};"),
    });
    out.push(Example {
        instruction: format!("Show me every row from {qualified}."),
        input: input.clone(),
        output: format!("SELECT {all_cols} FROM {qualified};"),
    });

    // "Show the first N …" — LIMIT variant.
    out.push(Example {
        instruction: format!("Show the first 10 {stem_plural}."),
        input: input.clone(),
        output: format!("SELECT {all_cols} FROM {qualified} LIMIT 10;"),
    });

    // Two-column projection — teaches "pick a subset of the listed columns".
    if columns.len() >= 2 {
        let c0 = columns[0].name.as_str();
        let c1 = columns[1].name.as_str();
        out.push(Example {
            instruction: format!(
                "Show each {stem_singular}'s {c0} and {c1}."
            ),
            input: input.clone(),
            output: format!("SELECT {c0}, {c1} FROM {qualified};"),
        });
    }

    // Count.
    out.push(Example {
        instruction: format!("How many {stem_plural} are there?"),
        input: input.clone(),
        output: format!("SELECT COUNT(*) FROM {qualified};"),
    });

    // Primary-key lookups — a very common NL shape, and one the model
    // needs to learn to answer with real PK columns (not `id`).
    for c in columns.iter().filter(|c| c.is_primary_key) {
        let pk = c.name.as_str();
        let placeholder = if c.data_type.to_lowercase().contains("int") {
            "1".to_string()
        } else {
            "'<value>'".to_string()
        };
        out.push(Example {
            instruction: format!(
                "Find the {stem_singular} with {pk} equal to {placeholder}."
            ),
            input: input.clone(),
            output: format!(
                "SELECT {all_cols} FROM {qualified} WHERE {pk} = {placeholder};"
            ),
        });
    }

    out
}

/// Contrastive example: reinforces that columns outside the provided list
/// must not appear in the query. Only emitted for tables with at least
/// three columns, otherwise the "only these" framing is silly.
fn derive_exclusion_example(
    qualified: &str,
    table_name: &str,
    columns: &[ColumnInfo],
) -> Option<Example> {
    if columns.len() < 3 {
        return None;
    }
    let stem_plural = pluralize_label(table_name);
    let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
    let all_cols = names.join(", ");
    let input = format!(
        "Schema for {qualified}: columns = {all_cols}.\n\
         These are the ONLY columns on this table. Any other column name \
         (for example created_at, updated_at, mfg_date, exp_date, description) \
         does NOT exist and must not appear in the query."
    );
    Some(Example {
        instruction: format!(
            "Write a SQL query that lists all {stem_plural} using only the \
             columns that exist on this table."
        ),
        input,
        output: format!("SELECT {all_cols} FROM {qualified};"),
    })
}

/// Turn a `snake_case_table_name` into a human-readable plural label like
/// "process orders". Preserves existing trailing "s" so "orders" stays
/// "orders" rather than becoming "orderss".
fn pluralize_label(table_name: &str) -> String {
    let readable = table_name.replace('_', " ");
    if readable.ends_with('s') || readable.ends_with("ies") {
        readable
    } else if readable.ends_with('y') {
        let base = &readable[..readable.len() - 1];
        format!("{base}ies")
    } else {
        format!("{readable}s")
    }
}

/// For each sampled row, pick up to two non-empty "scalar-looking" values
/// and turn them into an NL prompt → SELECT ... WHERE example.
fn derive_where_examples(
    qualified: &str,
    columns: &[ColumnInfo],
    samples: &[Vec<(String, String)>],
) -> Vec<Example> {
    let mut out = Vec::new();
    let col_name: std::collections::HashSet<&str> =
        columns.iter().map(|c| c.name.as_str()).collect();
    let schema_input = column_list_input(qualified, columns);

    for row in samples {
        let mut picks = 0;
        for (name, value) in row {
            if picks >= 2 {
                break;
            }
            if value.is_empty() || value.len() > 64 {
                continue;
            }
            if !col_name.contains(name.as_str()) {
                continue;
            }
            if is_numeric(value) {
                out.push(Example {
                    instruction: format!(
                        "Find rows in {qualified} where {name} is {value}."
                    ),
                    input: schema_input.clone(),
                    output: format!("SELECT * FROM {qualified} WHERE {name} = {value};"),
                });
            } else {
                let escaped = value.replace('\'', "''");
                out.push(Example {
                    instruction: format!(
                        "Find rows in {qualified} where {name} equals '{value}'."
                    ),
                    input: schema_input.clone(),
                    output: format!(
                        "SELECT * FROM {qualified} WHERE {name} = '{escaped}';"
                    ),
                });
            }
            picks += 1;
        }
    }
    out
}

fn is_numeric(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-')
        && s.matches('-').count() <= 1
        && s.matches('.').count() <= 1
}

/// Heuristic: for each `*_id` column, look for a table whose PK matches the
/// prefix. Emits an INNER JOIN template for each match.
fn derive_join_examples(
    driver: &Driver,
    all: &[(TableInfo, Vec<ColumnInfo>)],
) -> Vec<Example> {
    let mut by_name: std::collections::HashMap<String, &TableInfo> =
        std::collections::HashMap::new();
    for (t, _) in all {
        // Stem: remove plural "s" — crude but good enough for stock naming.
        let stem = singular(&t.name);
        by_name.entry(stem).or_insert(t);
    }

    let mut out = Vec::new();
    for (t, cols) in all {
        for c in cols {
            if let Some(stem) = c.name.strip_suffix("_id") {
                if let Some(target) = by_name.get(stem) {
                    if target.schema == t.schema && target.name == t.name {
                        continue;
                    }
                    let left = qualify(driver, &t.schema, &t.name);
                    let right = qualify(driver, &target.schema, &target.name);
                    out.push(Example {
                        instruction: format!(
                            "Join {left} with {right} so I can see each {} alongside the matching {} row.",
                            t.name, target.name
                        ),
                        input: String::new(),
                        output: format!(
                            "SELECT * FROM {left} AS a INNER JOIN {right} AS b ON a.{} = b.id;",
                            c.name
                        ),
                    });
                }
            }
        }
    }
    out
}

fn singular(s: &str) -> String {
    if let Some(rest) = s.strip_suffix("ies") {
        return format!("{rest}y");
    }
    if let Some(rest) = s.strip_suffix('s') {
        return rest.to_string();
    }
    s.to_string()
}

async fn write_example(file: &mut tokio::fs::File, ex: &Example) -> Result<(), String> {
    let line = json!(ex).to_string();
    file.write_all(line.as_bytes())
        .await
        .map_err(|e| format!("write: {e}"))?;
    file.write_all(b"\n")
        .await
        .map_err(|e| format!("write: {e}"))?;
    Ok(())
}
