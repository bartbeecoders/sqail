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

/// Convert days since an epoch year-01-01 into (year, month, day).
fn days_to_ymd(days: i64, epoch_year: i32) -> (i32, u32, u32) {
    // Convert to a civil date using a simple algorithm.
    // Shift to epoch 0000-03-01 for easier leap year math.
    let epoch_jdn = jdn(epoch_year, 1, 1);
    let target_jdn = epoch_jdn + days;
    jdn_to_ymd(target_jdn)
}

/// Julian Day Number from (year, month, day).
fn jdn(y: i32, m: u32, d: u32) -> i64 {
    let y = y as i64;
    let m = m as i64;
    let d = d as i64;
    (1461 * (y + 4800 + (m - 14) / 12)) / 4
        + (367 * (m - 2 - 12 * ((m - 14) / 12))) / 12
        - (3 * ((y + 4900 + (m - 14) / 12) / 100)) / 4
        + d
        - 32075
}

/// JDN back to (year, month, day).
fn jdn_to_ymd(jdn: i64) -> (i32, u32, u32) {
    let a = jdn + 32044;
    let b = (4 * a + 3) / 146097;
    let c = a - (146097 * b) / 4;
    let d = (4 * c + 3) / 1461;
    let e = c - (1461 * d) / 4;
    let m = (5 * e + 2) / 153;
    let day = (e - (153 * m + 2) / 5 + 1) as u32;
    let month = (m + 3 - 12 * (m / 10)) as u32;
    let year = (100 * b + d - 4800 + m / 10) as i32;
    (year, month, day)
}

/// Format seconds + fractional nanoseconds as HH:MM:SS[.fff].
fn format_time(total_secs: u64, nanos: u64) -> String {
    let h = total_secs / 3600;
    let m = (total_secs % 3600) / 60;
    let s = total_secs % 60;
    if nanos == 0 {
        format!("{h:02}:{m:02}:{s:02}")
    } else {
        // Trim trailing zeros from fractional part
        let frac = format!("{:09}", nanos);
        let trimmed = frac.trim_end_matches('0');
        format!("{h:02}:{m:02}:{s:02}.{trimmed}")
    }
}

fn decode_tiberius_value(col: &tiberius::Column, data: &tiberius::ColumnData<'_>) -> serde_json::Value {
    use tiberius::ColumnData;
    let _ = col;
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
        ColumnData::Guid(v) => match v {
            Some(uuid) => serde_json::Value::String(uuid.to_string()),
            None => serde_json::Value::Null,
        },
        ColumnData::Binary(v) => match v {
            Some(bytes) => serde_json::Value::String(format!("0x{}", hex::encode(bytes))),
            None => serde_json::Value::Null,
        },
        ColumnData::Numeric(v) => match v {
            Some(n) => {
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
        // datetime: days since 1900-01-01, seconds_fragments in 1/300ths of a second
        ColumnData::DateTime(v) => match v {
            Some(dt) => {
                let (y, mo, d) = days_to_ymd(dt.days() as i64, 1900);
                let total_ns = dt.seconds_fragments() as u64 * 1_000_000_000 / 300;
                let total_secs = total_ns / 1_000_000_000;
                let nanos = total_ns % 1_000_000_000;
                let time = format_time(total_secs, nanos);
                serde_json::Value::String(format!("{y:04}-{mo:02}-{d:02} {time}"))
            }
            None => serde_json::Value::Null,
        },
        // smalldatetime: days since 1900-01-01, seconds_fragments = minutes since midnight
        ColumnData::SmallDateTime(v) => match v {
            Some(dt) => {
                let (y, mo, d) = days_to_ymd(dt.days() as i64, 1900);
                let mins = dt.seconds_fragments() as u64;
                let total_secs = mins * 60;
                let time = format_time(total_secs, 0);
                serde_json::Value::String(format!("{y:04}-{mo:02}-{d:02} {time}"))
            }
            None => serde_json::Value::Null,
        },
        // date (tds73): days since 0001-01-01
        ColumnData::Date(v) => match v {
            Some(dt) => {
                let (y, mo, d) = days_to_ymd(dt.days() as i64, 1);
                serde_json::Value::String(format!("{y:04}-{mo:02}-{d:02}"))
            }
            None => serde_json::Value::Null,
        },
        // time (tds73): increments of 10^-scale seconds
        ColumnData::Time(v) => match v {
            Some(t) => {
                let divisor = 10u64.pow(t.scale() as u32);
                let total_secs = t.increments() / divisor;
                let remainder = t.increments() % divisor;
                let nanos = remainder * 1_000_000_000 / divisor;
                serde_json::Value::String(format_time(total_secs, nanos))
            }
            None => serde_json::Value::Null,
        },
        // datetime2 (tds73): Date + Time
        ColumnData::DateTime2(v) => match v {
            Some(dt2) => {
                let (y, mo, d) = days_to_ymd(dt2.date().days() as i64, 1);
                let t = dt2.time();
                let divisor = 10u64.pow(t.scale() as u32);
                let total_secs = t.increments() / divisor;
                let remainder = t.increments() % divisor;
                let nanos = remainder * 1_000_000_000 / divisor;
                let time = format_time(total_secs, nanos);
                serde_json::Value::String(format!("{y:04}-{mo:02}-{d:02} {time}"))
            }
            None => serde_json::Value::Null,
        },
        // datetimeoffset (tds73): DateTime2 + offset in minutes
        ColumnData::DateTimeOffset(v) => match v {
            Some(dto) => {
                let dt2 = dto.datetime2();
                let (y, mo, d) = days_to_ymd(dt2.date().days() as i64, 1);
                let t = dt2.time();
                let divisor = 10u64.pow(t.scale() as u32);
                let total_secs = t.increments() / divisor;
                let remainder = t.increments() % divisor;
                let nanos = remainder * 1_000_000_000 / divisor;
                let time = format_time(total_secs, nanos);
                let off = dto.offset();
                let sign = if off >= 0 { '+' } else { '-' };
                let abs_off = off.unsigned_abs() as u32;
                let oh = abs_off / 60;
                let om = abs_off % 60;
                serde_json::Value::String(format!("{y:04}-{mo:02}-{d:02} {time} {sign}{oh:02}:{om:02}"))
            }
            None => serde_json::Value::Null,
        },
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
