use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::db::connections::Driver;
use crate::pool::DbPool;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationResult {
    pub ok: bool,
    pub error: Option<String>,
    /// Brief explanation when `ok == true` but validation was skipped (e.g., DDL,
    /// unsupported driver, multi-statement script). UI can show as a soft warning.
    pub note: Option<String>,
}

impl ValidationResult {
    fn ok() -> Self {
        Self { ok: true, error: None, note: None }
    }
    fn skipped(note: &str) -> Self {
        Self { ok: true, error: None, note: Some(note.to_string()) }
    }
    fn failed(err: String) -> Self {
        Self { ok: false, error: Some(err), note: None }
    }
}

fn short_name() -> String {
    // PREPARE identifiers must be valid SQL identifiers. Use a UUID without hyphens.
    format!("_sqail_v_{}", uuid::Uuid::new_v4().simple())
}

/// Heuristic: skip validation for DDL and transaction-control statements, since
/// PG/MySQL can't PREPARE them. We let those pass.
fn is_prepareable(sql: &str) -> bool {
    let trimmed = sql.trim_start().to_uppercase();
    // Allow the common DML/DQL statements.
    matches!(
        trimmed
            .split(|c: char| c.is_whitespace())
            .next()
            .unwrap_or(""),
        "SELECT" | "INSERT" | "UPDATE" | "DELETE" | "WITH" | "VALUES" | "MERGE" | "CALL"
    )
}

// ── PostgreSQL ──────────────────────────────────────────────

async fn pg_validate(pool: &sqlx::PgPool, sql: &str) -> ValidationResult {
    let sql_trimmed = sql.trim().trim_end_matches(';').to_string();
    if sql_trimmed.is_empty() {
        return ValidationResult::failed("Empty query".to_string());
    }
    if !is_prepareable(&sql_trimmed) {
        return ValidationResult::skipped("Validation skipped (DDL / non-prepareable statement)");
    }
    let name = short_name();
    let prepare = format!("PREPARE {} AS {}", name, sql_trimmed);
    match sqlx::query(&prepare).execute(pool).await {
        Ok(_) => {
            // Best-effort cleanup; ignore errors.
            let _ = sqlx::query(&format!("DEALLOCATE {}", name))
                .execute(pool)
                .await;
            ValidationResult::ok()
        }
        Err(e) => ValidationResult::failed(e.to_string()),
    }
}

// ── MySQL ───────────────────────────────────────────────────

async fn mysql_validate(pool: &sqlx::MySqlPool, sql: &str) -> ValidationResult {
    let sql_trimmed = sql.trim().trim_end_matches(';').to_string();
    if sql_trimmed.is_empty() {
        return ValidationResult::failed("Empty query".to_string());
    }
    if !is_prepareable(&sql_trimmed) {
        return ValidationResult::skipped("Validation skipped (DDL / non-prepareable statement)");
    }
    let name = short_name();
    // MySQL's PREPARE takes either a literal or a user variable. Using a user
    // variable avoids having to escape the SQL inside a quoted literal.
    let set_stmt = "SET @_sqail_validate_sql = ?";
    if let Err(e) = sqlx::query(set_stmt).bind(&sql_trimmed).execute(pool).await {
        return ValidationResult::failed(e.to_string());
    }
    let prepare = format!("PREPARE {} FROM @_sqail_validate_sql", name);
    match sqlx::query(&prepare).execute(pool).await {
        Ok(_) => {
            let _ = sqlx::query(&format!("DEALLOCATE PREPARE {}", name))
                .execute(pool)
                .await;
            ValidationResult::ok()
        }
        Err(e) => ValidationResult::failed(e.to_string()),
    }
}

// ── SQLite ──────────────────────────────────────────────────

async fn sqlite_validate(pool: &sqlx::SqlitePool, sql: &str) -> ValidationResult {
    let sql_trimmed = sql.trim().trim_end_matches(';').to_string();
    if sql_trimmed.is_empty() {
        return ValidationResult::failed("Empty query".to_string());
    }
    // EXPLAIN runs the compiler without executing the statement.
    let explain = format!("EXPLAIN {}", sql_trimmed);
    match sqlx::query(&explain).fetch_all(pool).await {
        Ok(_) => ValidationResult::ok(),
        Err(e) => ValidationResult::failed(e.to_string()),
    }
}

// ── MSSQL ───────────────────────────────────────────────────

async fn mssql_validate(
    pool: &bb8::Pool<bb8_tiberius::ConnectionManager>,
    sql: &str,
) -> ValidationResult {
    let sql_trimmed = sql.trim().to_string();
    if sql_trimmed.is_empty() {
        return ValidationResult::failed("Empty query".to_string());
    }
    let mut conn = match pool.get().await {
        Ok(c) => c,
        Err(e) => return ValidationResult::failed(e.to_string()),
    };
    // Wrap the user's SQL between PARSEONLY toggles. This parses the batch
    // without executing it; any syntax error surfaces as a normal error.
    let batch = format!("SET PARSEONLY ON\n{}\nSET PARSEONLY OFF", sql_trimmed);
    let outcome: Result<(), tiberius::error::Error> = async {
        conn.simple_query(&batch)
            .await?
            .into_first_result()
            .await?;
        Ok(())
    }
    .await;
    match outcome {
        Ok(()) => ValidationResult::ok(),
        Err(e) => ValidationResult::failed(e.to_string()),
    }
}

// ── Public dispatch ─────────────────────────────────────────

fn get_pg_pool(pool: &DbPool) -> Option<&Arc<sqlx::PgPool>> {
    if let DbPool::Postgres(p) = pool { Some(p) } else { None }
}
fn get_mysql_pool(pool: &DbPool) -> Option<&Arc<sqlx::MySqlPool>> {
    if let DbPool::Mysql(p) = pool { Some(p) } else { None }
}
fn get_sqlite_pool(pool: &DbPool) -> Option<&Arc<sqlx::SqlitePool>> {
    if let DbPool::Sqlite(p) = pool { Some(p) } else { None }
}
fn get_mssql_pool(pool: &DbPool) -> Option<&Arc<bb8::Pool<bb8_tiberius::ConnectionManager>>> {
    if let DbPool::Mssql(p) = pool { Some(p) } else { None }
}

pub async fn validate(pool: DbPool, driver: &Driver, sql: &str) -> ValidationResult {
    match driver {
        Driver::Postgres => match get_pg_pool(&pool) {
            Some(p) => pg_validate(p, sql).await,
            None => ValidationResult::failed("Driver mismatch (expected Postgres pool)".into()),
        },
        Driver::Mysql => match get_mysql_pool(&pool) {
            Some(p) => mysql_validate(p, sql).await,
            None => ValidationResult::failed("Driver mismatch (expected MySQL pool)".into()),
        },
        Driver::Sqlite => match get_sqlite_pool(&pool) {
            Some(p) => sqlite_validate(p, sql).await,
            None => ValidationResult::failed("Driver mismatch (expected SQLite pool)".into()),
        },
        Driver::Mssql => match get_mssql_pool(&pool) {
            Some(p) => mssql_validate(p, sql).await,
            None => ValidationResult::failed("Driver mismatch (expected MSSQL pool)".into()),
        },
        Driver::Dbservice => {
            ValidationResult::skipped("Validation not supported via DbService")
        }
    }
}
