use serde::{Deserialize, Serialize};
use sqlx::{Column, Row, TypeInfo};
use std::sync::Arc;
use std::time::Instant;

use crate::pool::DbPool;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryColumn {
    pub name: String,
    pub type_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryResult {
    pub columns: Vec<QueryColumn>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub row_count: usize,
    pub affected_rows: Option<u64>,
    pub execution_time_ms: u64,
    pub is_mutation: bool,
    pub statement_index: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryResponse {
    pub results: Vec<QueryResult>,
    pub total_time_ms: u64,
    pub error: Option<String>,
}

fn decode_sqlx_value(row: &sqlx::any::AnyRow, idx: usize) -> serde_json::Value {
    if let Ok(v) = row.try_get::<Option<bool>, _>(idx) {
        return match v {
            Some(b) => serde_json::Value::Bool(b),
            None => serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<Option<i64>, _>(idx) {
        return match v {
            Some(n) => serde_json::json!(n),
            None => serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<Option<i32>, _>(idx) {
        return match v {
            Some(n) => serde_json::json!(n),
            None => serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<Option<f64>, _>(idx) {
        return match v {
            Some(n) => serde_json::json!(n),
            None => serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<Option<String>, _>(idx) {
        return match v {
            Some(s) => serde_json::Value::String(s),
            None => serde_json::Value::Null,
        };
    }
    if let Ok(v) = row.try_get::<Option<Vec<u8>>, _>(idx) {
        return match v {
            Some(bytes) => serde_json::Value::String(format!("0x{}", hex::encode(bytes))),
            None => serde_json::Value::Null,
        };
    }
    serde_json::Value::Null
}

fn decode_tiberius_value(col: &tiberius::Column, data: &tiberius::ColumnData<'_>) -> serde_json::Value {
    use tiberius::ColumnData;
    let _ = col; // col used for context if needed
    match data {
        ColumnData::Bit(v) => match v {
            Some(b) => serde_json::Value::Bool(*b),
            None => serde_json::Value::Null,
        },
        ColumnData::I16(v) => match v {
            Some(n) => serde_json::json!(n),
            None => serde_json::Value::Null,
        },
        ColumnData::I32(v) => match v {
            Some(n) => serde_json::json!(n),
            None => serde_json::Value::Null,
        },
        ColumnData::I64(v) => match v {
            Some(n) => serde_json::json!(n),
            None => serde_json::Value::Null,
        },
        ColumnData::F32(v) => match v {
            Some(n) => serde_json::json!(n),
            None => serde_json::Value::Null,
        },
        ColumnData::F64(v) => match v {
            Some(n) => serde_json::json!(n),
            None => serde_json::Value::Null,
        },
        ColumnData::U8(v) => match v {
            Some(n) => serde_json::json!(n),
            None => serde_json::Value::Null,
        },
        ColumnData::String(v) => match v {
            Some(s) => serde_json::Value::String(s.to_string()),
            None => serde_json::Value::Null,
        },
        ColumnData::Binary(v) => match v {
            Some(bytes) => serde_json::Value::String(format!("0x{}", hex::encode(bytes))),
            None => serde_json::Value::Null,
        },
        ColumnData::Numeric(v) => match v {
            Some(n) => {
                // Convert to f64 for JSON
                let s = format!("{n}");
                if let Ok(f) = s.parse::<f64>() {
                    serde_json::json!(f)
                } else {
                    serde_json::Value::String(s)
                }
            }
            None => serde_json::Value::Null,
        },
        ColumnData::Xml(v) => match v {
            Some(xml) => serde_json::Value::String(xml.to_string()),
            None => serde_json::Value::Null,
        },
        // DateTime types — render as string
        _ => {
            // For date/time/datetime/etc, try to use Display
            let s = format!("{data:?}");
            serde_json::Value::String(s)
        }
    }
}

fn is_mutation(sql: &str) -> bool {
    let trimmed = sql.trim_start().to_uppercase();
    trimmed.starts_with("INSERT")
        || trimmed.starts_with("UPDATE")
        || trimmed.starts_with("DELETE")
        || trimmed.starts_with("CREATE")
        || trimmed.starts_with("ALTER")
        || trimmed.starts_with("DROP")
        || trimmed.starts_with("TRUNCATE")
        || trimmed.starts_with("GRANT")
        || trimmed.starts_with("REVOKE")
}

fn split_statements(sql: &str) -> Vec<String> {
    let mut stmts: Vec<String> = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = sql.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        let c = chars[i];
        match c {
            '\'' => {
                current.push(c);
                i += 1;
                while i < len {
                    current.push(chars[i]);
                    if chars[i] == '\'' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            }
            '"' => {
                current.push(c);
                i += 1;
                while i < len {
                    current.push(chars[i]);
                    if chars[i] == '"' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            }
            '-' if i + 1 < len && chars[i + 1] == '-' => {
                while i < len && chars[i] != '\n' {
                    current.push(chars[i]);
                    i += 1;
                }
            }
            '/' if i + 1 < len && chars[i + 1] == '*' => {
                current.push(chars[i]);
                i += 1;
                current.push(chars[i]);
                i += 1;
                while i < len {
                    if chars[i] == '*' && i + 1 < len && chars[i + 1] == '/' {
                        current.push(chars[i]);
                        i += 1;
                        current.push(chars[i]);
                        i += 1;
                        break;
                    }
                    current.push(chars[i]);
                    i += 1;
                }
            }
            ';' => {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    stmts.push(trimmed);
                }
                current.clear();
                i += 1;
            }
            _ => {
                current.push(c);
                i += 1;
            }
        }
    }
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        stmts.push(trimmed);
    }
    stmts
}

// ── sqlx execution ──────────────────────────────────────────

async fn run_sqlx(pool: Arc<sqlx::AnyPool>, sql: &str) -> QueryResponse {
    let total_start = Instant::now();
    let statements = split_statements(sql);
    let mut results = Vec::new();
    let mut error = None;

    for (idx, stmt) in statements.iter().enumerate() {
        let start = Instant::now();

        if is_mutation(stmt) {
            match sqlx::query(stmt).execute(pool.as_ref()).await {
                Ok(result) => {
                    results.push(QueryResult {
                        columns: Vec::new(),
                        rows: Vec::new(),
                        row_count: 0,
                        affected_rows: Some(result.rows_affected()),
                        execution_time_ms: start.elapsed().as_millis() as u64,
                        is_mutation: true,
                        statement_index: idx,
                    });
                }
                Err(e) => {
                    error = Some(format!("Statement {}: {e}", idx + 1));
                    break;
                }
            }
        } else {
            match sqlx::query(stmt).fetch_all(pool.as_ref()).await {
                Ok(rows) => {
                    let elapsed = start.elapsed().as_millis() as u64;
                    let columns: Vec<QueryColumn> = if !rows.is_empty() {
                        rows[0]
                            .columns()
                            .iter()
                            .map(|col| QueryColumn {
                                name: col.name().to_string(),
                                type_name: col.type_info().name().to_string(),
                            })
                            .collect()
                    } else {
                        Vec::new()
                    };
                    let col_count = columns.len();
                    let data: Vec<Vec<serde_json::Value>> = rows
                        .iter()
                        .map(|row| (0..col_count).map(|i| decode_sqlx_value(row, i)).collect())
                        .collect();
                    let row_count = data.len();
                    results.push(QueryResult {
                        columns,
                        rows: data,
                        row_count,
                        affected_rows: None,
                        execution_time_ms: elapsed,
                        is_mutation: false,
                        statement_index: idx,
                    });
                }
                Err(e) => {
                    error = Some(format!("Statement {}: {e}", idx + 1));
                    break;
                }
            }
        }
    }

    QueryResponse {
        results,
        total_time_ms: total_start.elapsed().as_millis() as u64,
        error,
    }
}

// ── tiberius (MSSQL) execution ──────────────────────────────

async fn run_mssql(pool: Arc<bb8::Pool<bb8_tiberius::ConnectionManager>>, sql: &str) -> QueryResponse {
    let total_start = Instant::now();
    let statements = split_statements(sql);
    let mut results = Vec::new();
    let mut error = None;

    let mut conn = match pool.get().await {
        Ok(c) => c,
        Err(e) => {
            return QueryResponse {
                results: Vec::new(),
                total_time_ms: total_start.elapsed().as_millis() as u64,
                error: Some(format!("Failed to get MSSQL connection: {e}")),
            };
        }
    };

    for (idx, stmt) in statements.iter().enumerate() {
        let start = Instant::now();

        if is_mutation(stmt) {
            match conn.execute(stmt.as_str(), &[]).await {
                Ok(result) => {
                    let affected = result.rows_affected().iter().sum::<u64>();
                    results.push(QueryResult {
                        columns: Vec::new(),
                        rows: Vec::new(),
                        row_count: 0,
                        affected_rows: Some(affected),
                        execution_time_ms: start.elapsed().as_millis() as u64,
                        is_mutation: true,
                        statement_index: idx,
                    });
                }
                Err(e) => {
                    error = Some(format!("Statement {}: {e}", idx + 1));
                    break;
                }
            }
        } else {
            match conn.simple_query(stmt.as_str()).await {
                Ok(stream) => {
                    match stream.into_first_result().await {
                        Ok(rows) => {
                            let elapsed = start.elapsed().as_millis() as u64;
                            let columns: Vec<QueryColumn> = if !rows.is_empty() {
                                rows[0]
                                    .columns()
                                    .iter()
                                    .map(|col| QueryColumn {
                                        name: col.name().to_string(),
                                        type_name: format!("{:?}", col.column_type()),
                                    })
                                    .collect()
                            } else {
                                Vec::new()
                            };
                            let _col_count = columns.len();
                            let data: Vec<Vec<serde_json::Value>> = rows
                                .iter()
                                .map(|row| {
                                    row.cells()
                                        .map(|(col, cell)| decode_tiberius_value(col, cell))
                                        .collect()
                                })
                                .collect();
                            let row_count = data.len();
                            results.push(QueryResult {
                                columns,
                                rows: data,
                                row_count,
                                affected_rows: None,
                                execution_time_ms: elapsed,
                                is_mutation: false,
                                statement_index: idx,
                            });
                        }
                        Err(e) => {
                            error = Some(format!("Statement {}: {e}", idx + 1));
                            break;
                        }
                    }
                }
                Err(e) => {
                    error = Some(format!("Statement {}: {e}", idx + 1));
                    break;
                }
            }
        }
    }

    QueryResponse {
        results,
        total_time_ms: total_start.elapsed().as_millis() as u64,
        error,
    }
}

// ── Public dispatch ─────────────────────────────────────────

pub async fn run_query(pool: DbPool, sql: &str) -> QueryResponse {
    match pool {
        DbPool::Sqlx(p) => run_sqlx(p, sql).await,
        DbPool::Mssql(p) => run_mssql(p, sql).await,
    }
}
