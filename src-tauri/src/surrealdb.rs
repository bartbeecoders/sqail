//! HTTP client for SurrealDB (Phase 1).
//!
//! Talks to a remote SurrealDB instance via the documented REST surface:
//!   - GET  /version       — health/version probe
//!   - POST /sql           — execute one or more SurrealQL statements
//!   - GET  /status        — used as a fallback aliveness probe
//!
//! Authentication uses HTTP Basic with the configured user/password and the
//! `Surreal-NS` / `Surreal-DB` headers. Bearer-token sign-in via `/signin`
//! lands in Phase 2.
//!
//! Design notes:
//!   - No persistent connection state — every call reuses the cached
//!     `reqwest::Client` and a fresh request.
//!   - SurrealDB returns an array of statement-result envelopes
//!     `[{ status, time, result }, …]`, one per SQL statement. We surface each
//!     envelope as a `QueryResult` so the UI can present per-statement timing
//!     and errors the same way it does for SQL backends.

use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::pool::SurrealDbClient;
use crate::query::{QueryColumn, QueryResponse, QueryResult};
use crate::schema::{ColumnInfo, IndexInfo, SchemaInfo, TableInfo};

/// Single statement envelope returned by `POST /sql`.
#[derive(Debug, Deserialize)]
struct SurrealStatement {
    /// "OK" or "ERR".
    #[serde(default)]
    status: String,
    /// e.g. "1.2ms".
    #[serde(default)]
    time: String,
    /// `result` is present on success; on error it carries the error message
    /// either here or in `detail`.
    #[serde(default)]
    result: Value,
    #[serde(default)]
    detail: Option<String>,
}

/// Build a reqwest client with sane timeouts.
pub fn build_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .expect("failed to build reqwest client")
}

fn trim_base(s: &str) -> &str {
    s.trim_end_matches('/')
}

/// `POST /sql` with the supplied SurrealQL — returns the raw statement
/// envelopes so callers can decide whether to surface them as data, schema, or
/// errors.
async fn post_sql(client: &Arc<SurrealDbClient>, sql: &str) -> Result<Vec<SurrealStatement>, String> {
    let url = format!("{}/sql", trim_base(&client.base_url));
    let mut req = client
        .http
        .post(&url)
        .header("Accept", "application/json")
        .header("Content-Type", "text/plain")
        .body(sql.to_string());
    if !client.namespace.is_empty() {
        req = req.header("Surreal-NS", &client.namespace);
    }
    if !client.database.is_empty() {
        req = req.header("Surreal-DB", &client.database);
    }
    if !client.user.is_empty() {
        req = req.basic_auth(&client.user, Some(&client.password));
    }

    let res = req
        .send()
        .await
        .map_err(|e| format!("SurrealDB request failed: {e}"))?;

    let status = res.status();
    let body = res
        .text()
        .await
        .map_err(|e| format!("SurrealDB body read failed: {e}"))?;
    if !status.is_success() {
        return Err(format!("SurrealDB HTTP {status}: {body}"));
    }
    serde_json::from_str::<Vec<SurrealStatement>>(&body)
        .map_err(|e| format!("SurrealDB parse failed ({e}): {body}"))
}

/// Connection probe: reach the version endpoint and run a trivial SurrealQL
/// statement to confirm credentials and namespace/database routing.
pub async fn test(client: &Arc<SurrealDbClient>) -> Result<String, String> {
    let url = format!("{}/version", trim_base(&client.base_url));
    let res = client
        .http
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("SurrealDB version check failed: {e}"))?;
    if !res.status().is_success() {
        return Err(format!(
            "SurrealDB version probe failed ({}): {}",
            res.status(),
            res.text().await.unwrap_or_default()
        ));
    }
    let version = res.text().await.unwrap_or_default();

    // Cheap end-to-end probe: requires NS/DB headers + auth to succeed.
    let stmts = post_sql(client, "RETURN 1;").await?;
    let first = stmts
        .into_iter()
        .next()
        .ok_or_else(|| "SurrealDB returned no result for RETURN 1".to_string())?;
    if first.status.eq_ignore_ascii_case("OK") {
        Ok(format!("SurrealDB connection successful — {}", version.trim()))
    } else {
        Err(format!(
            "SurrealDB probe failed: {}",
            first
                .detail
                .unwrap_or_else(|| serde_json::to_string(&first.result).unwrap_or_default())
        ))
    }
}

/// Eager auth + NS/DB validation. Phase 1 has no per-connection state to
/// open, but exposing this matches the DbService flow and gives the UI a
/// fast-fail signal when connecting.
pub async fn connect(client: &Arc<SurrealDbClient>) -> Result<(), String> {
    test(client).await.map(|_| ())
}

// ── Query execution ───────────────────────────────────────────────

pub async fn execute_query(client: &Arc<SurrealDbClient>, sql: &str) -> QueryResponse {
    let total_start = Instant::now();
    let stmts = match post_sql(client, sql).await {
        Ok(s) => s,
        Err(e) => {
            return QueryResponse {
                results: vec![],
                total_time_ms: total_start.elapsed().as_millis() as u64,
                error: Some(e),
            };
        }
    };

    let mut results: Vec<QueryResult> = Vec::with_capacity(stmts.len());
    let mut error: Option<String> = None;

    for (idx, stmt) in stmts.into_iter().enumerate() {
        let elapsed_ms = parse_time_ms(&stmt.time);
        if !stmt.status.eq_ignore_ascii_case("OK") {
            let msg = stmt
                .detail
                .or_else(|| {
                    if stmt.result.is_string() {
                        stmt.result.as_str().map(|s| s.to_string())
                    } else {
                        Some(serde_json::to_string(&stmt.result).unwrap_or_default())
                    }
                })
                .unwrap_or_else(|| "SurrealDB statement failed".to_string());
            error = Some(format!("Statement {}: {msg}", idx + 1));
            break;
        }
        results.push(materialise_result(idx, elapsed_ms, stmt.result));
    }

    QueryResponse {
        results,
        total_time_ms: total_start.elapsed().as_millis() as u64,
        error,
    }
}

/// Convert a single SurrealDB result value into a `QueryResult`. The shape
/// rules are:
///   - Array of objects → columns are the union of keys (preserving the order
///     they first appear), `id` floated to the front when present.
///   - Array of scalars → one column "value".
///   - Single object    → one row, columns = its keys.
///   - Anything else    → one row × one column "value".
fn materialise_result(idx: usize, elapsed_ms: u64, value: Value) -> QueryResult {
    match value {
        Value::Array(items) => {
            let mut col_order: Vec<String> = Vec::new();
            let mut seen = std::collections::HashSet::<String>::new();
            let mut all_objects = true;
            for item in &items {
                match item {
                    Value::Object(map) => {
                        for k in map.keys() {
                            if seen.insert(k.clone()) {
                                col_order.push(k.clone());
                            }
                        }
                    }
                    _ => {
                        all_objects = false;
                    }
                }
            }
            if !all_objects || col_order.is_empty() {
                let rows: Vec<Vec<Value>> = items.into_iter().map(|v| vec![v]).collect();
                let row_count = rows.len();
                return QueryResult {
                    columns: vec![QueryColumn {
                        name: "value".to_string(),
                        type_name: "any".to_string(),
                    }],
                    rows,
                    row_count,
                    affected_rows: None,
                    execution_time_ms: elapsed_ms,
                    is_mutation: false,
                    statement_index: idx,
                };
            }
            // Float `id` to the front for record-shaped results.
            if let Some(pos) = col_order.iter().position(|c| c == "id") {
                let id = col_order.remove(pos);
                col_order.insert(0, id);
            }
            let columns: Vec<QueryColumn> = col_order
                .iter()
                .map(|name| QueryColumn {
                    name: name.clone(),
                    type_name: "any".to_string(),
                })
                .collect();
            let rows: Vec<Vec<Value>> = items
                .into_iter()
                .map(|item| match item {
                    Value::Object(map) => col_order
                        .iter()
                        .map(|k| map.get(k).cloned().unwrap_or(Value::Null))
                        .collect(),
                    other => col_order
                        .iter()
                        .enumerate()
                        .map(|(i, _)| if i == 0 { other.clone() } else { Value::Null })
                        .collect(),
                })
                .collect();
            let row_count = rows.len();
            QueryResult {
                columns,
                rows,
                row_count,
                affected_rows: None,
                execution_time_ms: elapsed_ms,
                is_mutation: false,
                statement_index: idx,
            }
        }
        Value::Object(map) => {
            let col_order: Vec<String> = map.keys().cloned().collect();
            let columns: Vec<QueryColumn> = col_order
                .iter()
                .map(|name| QueryColumn {
                    name: name.clone(),
                    type_name: "any".to_string(),
                })
                .collect();
            let row: Vec<Value> = col_order
                .iter()
                .map(|k| map.get(k).cloned().unwrap_or(Value::Null))
                .collect();
            QueryResult {
                columns,
                rows: vec![row],
                row_count: 1,
                affected_rows: None,
                execution_time_ms: elapsed_ms,
                is_mutation: false,
                statement_index: idx,
            }
        }
        scalar => QueryResult {
            columns: vec![QueryColumn {
                name: "value".to_string(),
                type_name: "any".to_string(),
            }],
            rows: vec![vec![scalar]],
            row_count: 1,
            affected_rows: None,
            execution_time_ms: elapsed_ms,
            is_mutation: false,
            statement_index: idx,
        },
    }
}

/// Parse `"1.2ms"` / `"500µs"` / `"3s"` from SurrealDB's `time` field into
/// milliseconds. Best-effort — falls back to 0 on unrecognised formats so we
/// never fail a query because of telemetry.
fn parse_time_ms(s: &str) -> u64 {
    let s = s.trim();
    if s.is_empty() {
        return 0;
    }
    let (num_part, unit) = if let Some(idx) = s.find(|c: char| c.is_alphabetic() || c == 'µ') {
        s.split_at(idx)
    } else {
        (s, "")
    };
    let n: f64 = num_part.parse().unwrap_or(0.0);
    let ms = match unit {
        "ns" => n / 1_000_000.0,
        "us" | "µs" => n / 1_000.0,
        "ms" => n,
        "s" => n * 1_000.0,
        "m" => n * 60_000.0,
        _ => n,
    };
    ms.max(0.0).round() as u64
}

// ── Schema introspection ──────────────────────────────────────────

/// Phase 1 returns the configured database as the single "schema" so the
/// existing tree shape continues to make sense. The label includes the
/// namespace when present so users can tell which scope they're browsing.
pub async fn list_schemas(client: &Arc<SurrealDbClient>) -> Result<Vec<SchemaInfo>, String> {
    let label = match (client.namespace.as_str(), client.database.as_str()) {
        ("", "") => "default".to_string(),
        ("", db) => db.to_string(),
        (ns, "") => ns.to_string(),
        (ns, db) => format!("{ns}/{db}"),
    };
    Ok(vec![SchemaInfo { name: label }])
}

#[derive(Debug, Serialize, Deserialize)]
struct InfoForDb {
    #[serde(default)]
    tables: Map<String, Value>,
}

/// `INFO FOR DB` returns an object whose `tables` key is itself an object
/// keyed by table name; the values are the `DEFINE TABLE` statements.
pub async fn list_tables(
    client: &Arc<SurrealDbClient>,
    _schema: &str,
) -> Result<Vec<TableInfo>, String> {
    let stmts = post_sql(client, "INFO FOR DB;").await?;
    let first = stmts
        .into_iter()
        .next()
        .ok_or_else(|| "INFO FOR DB returned no result".to_string())?;
    if !first.status.eq_ignore_ascii_case("OK") {
        return Err(first
            .detail
            .unwrap_or_else(|| "INFO FOR DB failed".to_string()));
    }

    // Strip a single-element array wrapper if SurrealDB emits one.
    let value = match first.result {
        Value::Array(mut a) if a.len() == 1 => a.remove(0),
        v => v,
    };
    let info: InfoForDb = serde_json::from_value(value).unwrap_or(InfoForDb {
        tables: Default::default(),
    });

    let schema_label = match (client.namespace.as_str(), client.database.as_str()) {
        ("", "") => "default".to_string(),
        ("", db) => db.to_string(),
        (ns, "") => ns.to_string(),
        (ns, db) => format!("{ns}/{db}"),
    };

    let mut tables: Vec<TableInfo> = info
        .tables
        .into_iter()
        .map(|(name, _def)| TableInfo {
            name,
            schema: schema_label.clone(),
            table_type: "table".to_string(),
        })
        .collect();
    tables.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(tables)
}

#[derive(Debug, Serialize, Deserialize)]
struct InfoForTable {
    #[serde(default)]
    fields: Map<String, Value>,
    #[serde(default)]
    indexes: Map<String, Value>,
}

/// Pull defined fields from `INFO FOR TABLE` (authoritative for
/// SCHEMAFULL tables). For SCHEMALESS tables the `fields` map is typically
/// empty and we fall back to sampling document keys via
/// `SELECT * FROM <table> LIMIT 25`.
pub async fn list_columns(
    client: &Arc<SurrealDbClient>,
    _schema: &str,
    table: &str,
) -> Result<Vec<ColumnInfo>, String> {
    let info = fetch_table_info(client, table).await?;

    let mut cols: Vec<ColumnInfo> = Vec::new();
    let mut seen = std::collections::HashSet::<String>::new();

    if !info.fields.is_empty() {
        let mut idx: i32 = 1;
        // `id` is implicit on every record — surface it as the first column
        // so the user always sees a primary key.
        if seen.insert("id".to_string()) {
            cols.push(ColumnInfo {
                name: "id".to_string(),
                data_type: "record".to_string(),
                is_nullable: false,
                column_default: None,
                is_primary_key: true,
                ordinal_position: idx,
            });
            idx += 1;
        }
        for (name, def) in info.fields {
            if !seen.insert(name.clone()) {
                continue;
            }
            cols.push(ColumnInfo {
                name,
                data_type: parse_field_type(&def).unwrap_or_else(|| "any".to_string()),
                is_nullable: true,
                column_default: None,
                is_primary_key: false,
                ordinal_position: idx,
            });
            idx += 1;
        }
        return Ok(cols);
    }

    // Schemaless fallback — sample documents and infer keys.
    let sql = format!("SELECT * FROM {} LIMIT 25;", quote_ident(table));
    let stmts = post_sql(client, &sql).await?;
    let first = match stmts.into_iter().next() {
        Some(s) if s.status.eq_ignore_ascii_case("OK") => s,
        _ => {
            return Ok(vec![ColumnInfo {
                name: "id".to_string(),
                data_type: "record".to_string(),
                is_nullable: false,
                column_default: None,
                is_primary_key: true,
                ordinal_position: 1,
            }]);
        }
    };

    let mut idx: i32 = 1;
    if seen.insert("id".to_string()) {
        cols.push(ColumnInfo {
            name: "id".to_string(),
            data_type: "record".to_string(),
            is_nullable: false,
            column_default: None,
            is_primary_key: true,
            ordinal_position: idx,
        });
        idx += 1;
    }
    if let Value::Array(rows) = first.result {
        for row in rows {
            if let Value::Object(map) = row {
                for (k, v) in map {
                    if !seen.insert(k.clone()) {
                        continue;
                    }
                    cols.push(ColumnInfo {
                        name: k,
                        data_type: format!("{} (inferred)", json_type(&v)),
                        is_nullable: true,
                        column_default: None,
                        is_primary_key: false,
                        ordinal_position: idx,
                    });
                    idx += 1;
                }
            }
        }
    }
    Ok(cols)
}

/// Index list for a single table, derived from the same `INFO FOR TABLE`
/// envelope. Each entry's value is a `DEFINE INDEX` SurrealQL statement; we
/// parse the column list out of `FIELDS …` / `COLUMNS …` and detect the
/// `UNIQUE` keyword.
pub async fn list_indexes(
    client: &Arc<SurrealDbClient>,
    _schema: &str,
    table: &str,
) -> Result<Vec<IndexInfo>, String> {
    let info = fetch_table_info(client, table).await?;
    let mut out: Vec<IndexInfo> = Vec::new();
    for (name, def) in info.indexes {
        let def_str = match def {
            Value::String(s) => s,
            other => serde_json::to_string(&other).unwrap_or_default(),
        };
        let upper = def_str.to_uppercase();
        let is_unique = upper.contains(" UNIQUE");
        let columns = parse_index_columns(&def_str);
        out.push(IndexInfo {
            name,
            is_unique,
            columns,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

async fn fetch_table_info(
    client: &Arc<SurrealDbClient>,
    table: &str,
) -> Result<InfoForTable, String> {
    let sql = format!("INFO FOR TABLE {};", quote_ident(table));
    let stmts = post_sql(client, &sql).await?;
    let first = stmts
        .into_iter()
        .next()
        .ok_or_else(|| format!("INFO FOR TABLE {table} returned no result"))?;
    if !first.status.eq_ignore_ascii_case("OK") {
        return Err(first.detail.unwrap_or_else(|| {
            format!("INFO FOR TABLE {table} failed")
        }));
    }
    let value = match first.result {
        Value::Array(mut a) if a.len() == 1 => a.remove(0),
        v => v,
    };
    Ok(serde_json::from_value(value).unwrap_or(InfoForTable {
        fields: Default::default(),
        indexes: Default::default(),
    }))
}

/// Pull a SurrealDB type out of a `DEFINE FIELD … TYPE <T>` statement.
fn parse_field_type(def: &Value) -> Option<String> {
    let s = def.as_str()?;
    let upper = s.to_uppercase();
    let idx = upper.find(" TYPE ")?;
    let rest = &s[idx + " TYPE ".len()..];
    let end = rest
        .find([' ', ';', '\n', '\t'])
        .unwrap_or(rest.len());
    Some(rest[..end].trim().to_string())
}

/// Best-effort column extraction from a `DEFINE INDEX … FIELDS a, b` /
/// `COLUMNS a, b` clause.
fn parse_index_columns(def: &str) -> Vec<String> {
    let upper = def.to_uppercase();
    let key = if let Some(i) = upper.find(" FIELDS ") {
        i + " FIELDS ".len()
    } else if let Some(i) = upper.find(" COLUMNS ") {
        i + " COLUMNS ".len()
    } else {
        return Vec::new();
    };
    let rest = &def[key..];
    let end = rest.find(';').unwrap_or(rest.len());
    rest[..end]
        .split(',')
        .map(|s| s.trim().trim_end_matches(';').trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn quote_ident(name: &str) -> String {
    // SurrealQL identifiers can be wrapped in backticks. Escape any backtick
    // in the name itself by doubling it.
    let mut out = String::with_capacity(name.len() + 2);
    out.push('`');
    for c in name.chars() {
        if c == '`' {
            out.push('`');
            out.push('`');
        } else {
            out.push(c);
        }
    }
    out.push('`');
    out
}

fn json_type(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(n) if n.is_i64() => "int",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
