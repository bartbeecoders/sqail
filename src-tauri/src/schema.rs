use serde::{Deserialize, Serialize};
use sqlx::{AnyPool, Row};
use std::sync::Arc;

use crate::db::connections::Driver;
use crate::pool::DbPool;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaInfo {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableInfo {
    pub name: String,
    pub schema: String,
    pub table_type: String, // "table" or "view"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub is_nullable: bool,
    pub column_default: Option<String>,
    pub is_primary_key: bool,
    pub ordinal_position: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexInfo {
    pub name: String,
    pub is_unique: bool,
    pub columns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineInfo {
    pub name: String,
    pub schema: String,
    pub routine_type: String, // "function" or "procedure"
}

// ── PostgreSQL ──────────────────────────────────────────────

async fn pg_schemas(pool: &AnyPool) -> Result<Vec<SchemaInfo>, String> {
    let rows = sqlx::query(
        "SELECT schema_name FROM information_schema.schemata \
         WHERE schema_name NOT IN ('pg_catalog','information_schema','pg_toast') \
         ORDER BY schema_name",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .map(|r| SchemaInfo {
            name: r.get::<String, _>(0),
        })
        .collect())
}

async fn pg_tables(pool: &AnyPool, schema: &str) -> Result<Vec<TableInfo>, String> {
    let rows = sqlx::query(
        "SELECT table_name, table_type FROM information_schema.tables \
         WHERE table_schema = $1 ORDER BY table_name",
    )
    .bind(schema)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .map(|r| {
            let tt: String = r.get(1);
            TableInfo {
                name: r.get(0),
                schema: schema.to_string(),
                table_type: if tt.contains("VIEW") {
                    "view".to_string()
                } else {
                    "table".to_string()
                },
            }
        })
        .collect())
}

async fn pg_columns(pool: &AnyPool, schema: &str, table: &str) -> Result<Vec<ColumnInfo>, String> {
    let rows = sqlx::query(
        "SELECT c.column_name, c.data_type, c.is_nullable, c.column_default, c.ordinal_position, \
         CASE WHEN tc.constraint_type = 'PRIMARY KEY' THEN true ELSE false END as is_pk \
         FROM information_schema.columns c \
         LEFT JOIN information_schema.key_column_usage kcu \
           ON c.table_schema = kcu.table_schema AND c.table_name = kcu.table_name AND c.column_name = kcu.column_name \
         LEFT JOIN information_schema.table_constraints tc \
           ON kcu.constraint_name = tc.constraint_name AND tc.constraint_type = 'PRIMARY KEY' AND tc.table_schema = kcu.table_schema \
         WHERE c.table_schema = $1 AND c.table_name = $2 \
         ORDER BY c.ordinal_position",
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .map(|r| ColumnInfo {
            name: r.get(0),
            data_type: r.get(1),
            is_nullable: r.get::<String, _>(2) == "YES",
            column_default: r.get(3),
            is_primary_key: r.get::<bool, _>(4),
            ordinal_position: r.get::<i32, _>(5),
        })
        .collect())
}

async fn pg_indexes(pool: &AnyPool, schema: &str, table: &str) -> Result<Vec<IndexInfo>, String> {
    let rows = sqlx::query(
        "SELECT i.relname, ix.indisunique, array_agg(a.attname ORDER BY k.n) \
         FROM pg_index ix \
         JOIN pg_class t ON t.oid = ix.indrelid \
         JOIN pg_class i ON i.oid = ix.indexrelid \
         JOIN pg_namespace n ON n.oid = t.relnamespace \
         CROSS JOIN LATERAL unnest(ix.indkey) WITH ORDINALITY AS k(attnum, n) \
         JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = k.attnum \
         WHERE n.nspname = $1 AND t.relname = $2 \
         GROUP BY i.relname, ix.indisunique \
         ORDER BY i.relname",
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .map(|r| {
            let cols_str: String = r.get(2);
            let columns: Vec<String> = cols_str
                .trim_start_matches('{')
                .trim_end_matches('}')
                .split(',')
                .map(|s| s.to_string())
                .collect();
            IndexInfo {
                name: r.get(0),
                is_unique: r.get(1),
                columns,
            }
        })
        .collect())
}

async fn pg_routines(pool: &AnyPool, schema: &str) -> Result<Vec<RoutineInfo>, String> {
    let rows = sqlx::query(
        "SELECT routine_name, routine_type FROM information_schema.routines \
         WHERE routine_schema = $1 \
         ORDER BY routine_type, routine_name",
    )
    .bind(schema)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .map(|r| {
            let rt: String = r.get(1);
            RoutineInfo {
                name: r.get(0),
                schema: schema.to_string(),
                routine_type: if rt == "PROCEDURE" {
                    "procedure".to_string()
                } else {
                    "function".to_string()
                },
            }
        })
        .collect())
}

// ── MySQL ───────────────────────────────────────────────────

async fn mysql_schemas(pool: &AnyPool) -> Result<Vec<SchemaInfo>, String> {
    let rows = sqlx::query(
        "SELECT schema_name FROM information_schema.schemata \
         WHERE schema_name NOT IN ('mysql','information_schema','performance_schema','sys') \
         ORDER BY schema_name",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .map(|r| SchemaInfo {
            name: r.get::<String, _>(0),
        })
        .collect())
}

async fn mysql_tables(pool: &AnyPool, schema: &str) -> Result<Vec<TableInfo>, String> {
    let rows = sqlx::query(
        "SELECT table_name, table_type FROM information_schema.tables \
         WHERE table_schema = ? ORDER BY table_name",
    )
    .bind(schema)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .map(|r| {
            let tt: String = r.get(1);
            TableInfo {
                name: r.get(0),
                schema: schema.to_string(),
                table_type: if tt.contains("VIEW") {
                    "view".to_string()
                } else {
                    "table".to_string()
                },
            }
        })
        .collect())
}

async fn mysql_columns(
    pool: &AnyPool,
    schema: &str,
    table: &str,
) -> Result<Vec<ColumnInfo>, String> {
    let rows = sqlx::query(
        "SELECT column_name, column_type, is_nullable, column_default, ordinal_position, column_key \
         FROM information_schema.columns \
         WHERE table_schema = ? AND table_name = ? \
         ORDER BY ordinal_position",
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .map(|r| ColumnInfo {
            name: r.get(0),
            data_type: r.get(1),
            is_nullable: r.get::<String, _>(2) == "YES",
            column_default: r.get(3),
            is_primary_key: r.get::<String, _>(5) == "PRI",
            ordinal_position: r.get::<i32, _>(4),
        })
        .collect())
}

async fn mysql_indexes(
    pool: &AnyPool,
    schema: &str,
    table: &str,
) -> Result<Vec<IndexInfo>, String> {
    let rows = sqlx::query(
        "SELECT index_name, NOT non_unique, GROUP_CONCAT(column_name ORDER BY seq_in_index) \
         FROM information_schema.statistics \
         WHERE table_schema = ? AND table_name = ? \
         GROUP BY index_name, non_unique \
         ORDER BY index_name",
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .map(|r| {
            let cols_str: String = r.get(2);
            IndexInfo {
                name: r.get(0),
                is_unique: r.get(1),
                columns: cols_str.split(',').map(|s| s.to_string()).collect(),
            }
        })
        .collect())
}

async fn mysql_routines(pool: &AnyPool, schema: &str) -> Result<Vec<RoutineInfo>, String> {
    let rows = sqlx::query(
        "SELECT routine_name, routine_type FROM information_schema.routines \
         WHERE routine_schema = ? \
         ORDER BY routine_type, routine_name",
    )
    .bind(schema)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .map(|r| {
            let rt: String = r.get(1);
            RoutineInfo {
                name: r.get(0),
                schema: schema.to_string(),
                routine_type: if rt == "PROCEDURE" {
                    "procedure".to_string()
                } else {
                    "function".to_string()
                },
            }
        })
        .collect())
}

// ── SQLite ──────────────────────────────────────────────────

async fn sqlite_schemas(_pool: &AnyPool) -> Result<Vec<SchemaInfo>, String> {
    Ok(vec![SchemaInfo {
        name: "main".to_string(),
    }])
}

async fn sqlite_tables(pool: &AnyPool, _schema: &str) -> Result<Vec<TableInfo>, String> {
    let rows = sqlx::query(
        "SELECT name, type FROM sqlite_master \
         WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' \
         ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .map(|r| TableInfo {
            name: r.get(0),
            schema: "main".to_string(),
            table_type: r.get(1),
        })
        .collect())
}

async fn sqlite_columns(
    pool: &AnyPool,
    _schema: &str,
    table: &str,
) -> Result<Vec<ColumnInfo>, String> {
    // pragma_table_info returns: cid, name, type, notnull, dflt_value, pk
    let sql = format!("SELECT * FROM pragma_table_info('{}')", table.replace('\'', "''"));
    let rows = sqlx::query(&sql)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .map(|r| ColumnInfo {
            name: r.get::<String, _>(1),
            data_type: r.get::<String, _>(2),
            is_nullable: r.get::<i32, _>(3) == 0,
            column_default: r.get::<Option<String>, _>(4),
            is_primary_key: r.get::<i32, _>(5) != 0,
            ordinal_position: r.get::<i32, _>(0),
        })
        .collect())
}

async fn sqlite_indexes(
    pool: &AnyPool,
    _schema: &str,
    table: &str,
) -> Result<Vec<IndexInfo>, String> {
    let index_rows = sqlx::query(
        "SELECT name, \"unique\" FROM pragma_index_list(?)",
    )
    .bind(table)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut result = Vec::new();
    for ir in &index_rows {
        let idx_name: String = ir.get(0);
        let is_unique: bool = ir.get::<i32, _>(1) != 0;
        let sql = format!("SELECT name FROM pragma_index_info('{}')", idx_name.replace('\'', "''"));
        let col_rows = sqlx::query(&sql)
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?;
        let columns: Vec<String> = col_rows.iter().map(|cr| cr.get(0)).collect();
        result.push(IndexInfo {
            name: idx_name,
            is_unique,
            columns,
        });
    }
    Ok(result)
}

async fn sqlite_routines(_pool: &AnyPool, _schema: &str) -> Result<Vec<RoutineInfo>, String> {
    Ok(vec![])
}

// ── MSSQL ───────────────────────────────────────────────────

async fn mssql_schemas(
    pool: &bb8::Pool<bb8_tiberius::ConnectionManager>,
) -> Result<Vec<SchemaInfo>, String> {
    let mut conn = pool.get().await.map_err(|e| e.to_string())?;
    let rows = conn
        .simple_query(
            "SELECT s.name FROM sys.schemas s \
             JOIN sys.database_principals p ON s.principal_id = p.principal_id \
             WHERE s.name NOT IN ('sys','INFORMATION_SCHEMA','guest','db_owner','db_accessadmin',\
             'db_securityadmin','db_ddladmin','db_backupoperator','db_datareader','db_datawriter',\
             'db_denydatareader','db_denydatawriter') \
             ORDER BY s.name",
        )
        .await
        .map_err(|e| e.to_string())?
        .into_first_result()
        .await
        .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .filter_map(|r| {
            r.get::<&str, _>(0)
                .map(|name| SchemaInfo { name: name.to_string() })
        })
        .collect())
}

async fn mssql_tables(
    pool: &bb8::Pool<bb8_tiberius::ConnectionManager>,
    schema: &str,
) -> Result<Vec<TableInfo>, String> {
    let mut conn = pool.get().await.map_err(|e| e.to_string())?;
    let query = format!(
        "SELECT TABLE_NAME, TABLE_TYPE FROM INFORMATION_SCHEMA.TABLES \
         WHERE TABLE_SCHEMA = '{}' ORDER BY TABLE_NAME",
        schema.replace('\'', "''")
    );
    let rows = conn
        .simple_query(&query)
        .await
        .map_err(|e| e.to_string())?
        .into_first_result()
        .await
        .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .filter_map(|r| {
            let name = r.get::<&str, _>(0)?;
            let tt = r.get::<&str, _>(1).unwrap_or("BASE TABLE");
            Some(TableInfo {
                name: name.to_string(),
                schema: schema.to_string(),
                table_type: if tt.contains("VIEW") {
                    "view".to_string()
                } else {
                    "table".to_string()
                },
            })
        })
        .collect())
}

async fn mssql_columns(
    pool: &bb8::Pool<bb8_tiberius::ConnectionManager>,
    schema: &str,
    table: &str,
) -> Result<Vec<ColumnInfo>, String> {
    let mut conn = pool.get().await.map_err(|e| e.to_string())?;
    let query = format!(
        "SELECT c.COLUMN_NAME, c.DATA_TYPE, c.IS_NULLABLE, c.COLUMN_DEFAULT, c.ORDINAL_POSITION, \
         CASE WHEN pk.COLUMN_NAME IS NOT NULL THEN 1 ELSE 0 END as is_pk \
         FROM INFORMATION_SCHEMA.COLUMNS c \
         LEFT JOIN ( \
           SELECT kcu.TABLE_SCHEMA, kcu.TABLE_NAME, kcu.COLUMN_NAME \
           FROM INFORMATION_SCHEMA.TABLE_CONSTRAINTS tc \
           JOIN INFORMATION_SCHEMA.KEY_COLUMN_USAGE kcu \
             ON tc.CONSTRAINT_NAME = kcu.CONSTRAINT_NAME AND tc.TABLE_SCHEMA = kcu.TABLE_SCHEMA \
           WHERE tc.CONSTRAINT_TYPE = 'PRIMARY KEY' \
         ) pk ON c.TABLE_SCHEMA = pk.TABLE_SCHEMA AND c.TABLE_NAME = pk.TABLE_NAME AND c.COLUMN_NAME = pk.COLUMN_NAME \
         WHERE c.TABLE_SCHEMA = '{}' AND c.TABLE_NAME = '{}' \
         ORDER BY c.ORDINAL_POSITION",
        schema.replace('\'', "''"),
        table.replace('\'', "''")
    );
    let rows = conn
        .simple_query(&query)
        .await
        .map_err(|e| e.to_string())?
        .into_first_result()
        .await
        .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .filter_map(|r| {
            let name = r.get::<&str, _>(0)?;
            let data_type = r.get::<&str, _>(1).unwrap_or("unknown");
            let nullable = r.get::<&str, _>(2).unwrap_or("NO");
            let default: Option<String> = r.get::<&str, _>(3).map(|s| s.to_string());
            let ordinal = r.get::<i32, _>(4).unwrap_or(0);
            let is_pk = r.get::<i32, _>(5).unwrap_or(0) != 0;
            Some(ColumnInfo {
                name: name.to_string(),
                data_type: data_type.to_string(),
                is_nullable: nullable == "YES",
                column_default: default,
                is_primary_key: is_pk,
                ordinal_position: ordinal,
            })
        })
        .collect())
}

async fn mssql_indexes(
    pool: &bb8::Pool<bb8_tiberius::ConnectionManager>,
    schema: &str,
    table: &str,
) -> Result<Vec<IndexInfo>, String> {
    let mut conn = pool.get().await.map_err(|e| e.to_string())?;
    let query = format!(
        "SELECT i.name, i.is_unique, STRING_AGG(c.name, ',') WITHIN GROUP (ORDER BY ic.key_ordinal) \
         FROM sys.indexes i \
         JOIN sys.index_columns ic ON i.object_id = ic.object_id AND i.index_id = ic.index_id \
         JOIN sys.columns c ON ic.object_id = c.object_id AND ic.column_id = c.column_id \
         JOIN sys.tables t ON i.object_id = t.object_id \
         JOIN sys.schemas s ON t.schema_id = s.schema_id \
         WHERE s.name = '{}' AND t.name = '{}' AND i.name IS NOT NULL \
         GROUP BY i.name, i.is_unique \
         ORDER BY i.name",
        schema.replace('\'', "''"),
        table.replace('\'', "''")
    );
    let rows = conn
        .simple_query(&query)
        .await
        .map_err(|e| e.to_string())?
        .into_first_result()
        .await
        .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .filter_map(|r| {
            let name = r.get::<&str, _>(0)?;
            let is_unique = r.get::<bool, _>(1).unwrap_or(false);
            let cols_str = r.get::<&str, _>(2).unwrap_or("");
            Some(IndexInfo {
                name: name.to_string(),
                is_unique,
                columns: cols_str.split(',').map(|s| s.to_string()).collect(),
            })
        })
        .collect())
}

async fn mssql_routines(
    pool: &bb8::Pool<bb8_tiberius::ConnectionManager>,
    schema: &str,
) -> Result<Vec<RoutineInfo>, String> {
    let mut conn = pool.get().await.map_err(|e| e.to_string())?;
    let query = format!(
        "SELECT r.ROUTINE_NAME, r.ROUTINE_TYPE \
         FROM INFORMATION_SCHEMA.ROUTINES r \
         WHERE r.ROUTINE_SCHEMA = '{}' \
         ORDER BY r.ROUTINE_TYPE, r.ROUTINE_NAME",
        schema.replace('\'', "''")
    );
    let rows = conn
        .simple_query(&query)
        .await
        .map_err(|e| e.to_string())?
        .into_first_result()
        .await
        .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .filter_map(|r| {
            let name = r.get::<&str, _>(0)?;
            let rt = r.get::<&str, _>(1).unwrap_or("FUNCTION");
            Some(RoutineInfo {
                name: name.to_string(),
                schema: schema.to_string(),
                routine_type: if rt == "PROCEDURE" {
                    "procedure".to_string()
                } else {
                    "function".to_string()
                },
            })
        })
        .collect())
}

// ── Public dispatch ─────────────────────────────────────────

fn get_sqlx_pool(pool: &DbPool) -> Result<&Arc<AnyPool>, String> {
    match pool {
        DbPool::Sqlx(p) => Ok(p),
        DbPool::Mssql(_) => Err("Expected sqlx pool, got MSSQL".to_string()),
    }
}

fn get_mssql_pool(pool: &DbPool) -> Result<&Arc<bb8::Pool<bb8_tiberius::ConnectionManager>>, String> {
    match pool {
        DbPool::Mssql(p) => Ok(p),
        DbPool::Sqlx(_) => Err("Expected MSSQL pool, got sqlx".to_string()),
    }
}

pub async fn list_schemas(
    pool: DbPool,
    driver: &Driver,
) -> Result<Vec<SchemaInfo>, String> {
    match driver {
        Driver::Postgres => pg_schemas(get_sqlx_pool(&pool)?).await,
        Driver::Mysql => mysql_schemas(get_sqlx_pool(&pool)?).await,
        Driver::Sqlite => sqlite_schemas(get_sqlx_pool(&pool)?).await,
        Driver::Mssql => mssql_schemas(get_mssql_pool(&pool)?).await,
    }
}

pub async fn list_tables(
    pool: DbPool,
    driver: &Driver,
    schema: &str,
) -> Result<Vec<TableInfo>, String> {
    match driver {
        Driver::Postgres => pg_tables(get_sqlx_pool(&pool)?, schema).await,
        Driver::Mysql => mysql_tables(get_sqlx_pool(&pool)?, schema).await,
        Driver::Sqlite => sqlite_tables(get_sqlx_pool(&pool)?, schema).await,
        Driver::Mssql => mssql_tables(get_mssql_pool(&pool)?, schema).await,
    }
}

pub async fn list_columns(
    pool: DbPool,
    driver: &Driver,
    schema: &str,
    table: &str,
) -> Result<Vec<ColumnInfo>, String> {
    match driver {
        Driver::Postgres => pg_columns(get_sqlx_pool(&pool)?, schema, table).await,
        Driver::Mysql => mysql_columns(get_sqlx_pool(&pool)?, schema, table).await,
        Driver::Sqlite => sqlite_columns(get_sqlx_pool(&pool)?, schema, table).await,
        Driver::Mssql => mssql_columns(get_mssql_pool(&pool)?, schema, table).await,
    }
}

pub async fn list_indexes(
    pool: DbPool,
    driver: &Driver,
    schema: &str,
    table: &str,
) -> Result<Vec<IndexInfo>, String> {
    match driver {
        Driver::Postgres => pg_indexes(get_sqlx_pool(&pool)?, schema, table).await,
        Driver::Mysql => mysql_indexes(get_sqlx_pool(&pool)?, schema, table).await,
        Driver::Sqlite => sqlite_indexes(get_sqlx_pool(&pool)?, schema, table).await,
        Driver::Mssql => mssql_indexes(get_mssql_pool(&pool)?, schema, table).await,
    }
}

pub async fn list_routines(
    pool: DbPool,
    driver: &Driver,
    schema: &str,
) -> Result<Vec<RoutineInfo>, String> {
    match driver {
        Driver::Postgres => pg_routines(get_sqlx_pool(&pool)?, schema).await,
        Driver::Mysql => mysql_routines(get_sqlx_pool(&pool)?, schema).await,
        Driver::Sqlite => sqlite_routines(get_sqlx_pool(&pool)?, schema).await,
        Driver::Mssql => mssql_routines(get_mssql_pool(&pool)?, schema).await,
    }
}
