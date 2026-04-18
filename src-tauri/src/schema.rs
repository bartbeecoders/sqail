use serde::{Deserialize, Serialize};
use sqlx::Row;
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForeignKeyInfo {
    pub constraint_name: String,
    pub source_schema: String,
    pub source_table: String,
    pub source_column: String,
    pub target_schema: String,
    pub target_table: String,
    pub target_column: String,
    pub ordinal: i32,
}

// ── PostgreSQL ──────────────────────────────────────────────

async fn pg_schemas(pool: &sqlx::PgPool) -> Result<Vec<SchemaInfo>, String> {
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

async fn pg_tables(pool: &sqlx::PgPool, schema: &str) -> Result<Vec<TableInfo>, String> {
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

async fn pg_columns(pool: &sqlx::PgPool, schema: &str, table: &str) -> Result<Vec<ColumnInfo>, String> {
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
            ordinal_position: r.get::<i32, _>(4),
            is_primary_key: r.get::<bool, _>(5),
        })
        .collect())
}

async fn pg_indexes(pool: &sqlx::PgPool, schema: &str, table: &str) -> Result<Vec<IndexInfo>, String> {
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
            let columns: Vec<String> = r.get(2);
            IndexInfo {
                name: r.get(0),
                is_unique: r.get(1),
                columns,
            }
        })
        .collect())
}

async fn pg_routines(pool: &sqlx::PgPool, schema: &str) -> Result<Vec<RoutineInfo>, String> {
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

async fn mysql_schemas(pool: &sqlx::MySqlPool) -> Result<Vec<SchemaInfo>, String> {
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

async fn mysql_tables(pool: &sqlx::MySqlPool, schema: &str) -> Result<Vec<TableInfo>, String> {
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
    pool: &sqlx::MySqlPool,
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
    pool: &sqlx::MySqlPool,
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

async fn mysql_routines(pool: &sqlx::MySqlPool, schema: &str) -> Result<Vec<RoutineInfo>, String> {
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

async fn sqlite_schemas(_pool: &sqlx::SqlitePool) -> Result<Vec<SchemaInfo>, String> {
    Ok(vec![SchemaInfo {
        name: "main".to_string(),
    }])
}

async fn sqlite_tables(pool: &sqlx::SqlitePool, _schema: &str) -> Result<Vec<TableInfo>, String> {
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
    pool: &sqlx::SqlitePool,
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
    pool: &sqlx::SqlitePool,
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

async fn sqlite_routines(_pool: &sqlx::SqlitePool, _schema: &str) -> Result<Vec<RoutineInfo>, String> {
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

fn get_pg_pool(pool: &DbPool) -> Result<&Arc<sqlx::PgPool>, String> {
    match pool {
        DbPool::Postgres(p) => Ok(p),
        _ => Err("Expected PostgreSQL pool".to_string()),
    }
}

fn get_mysql_pool(pool: &DbPool) -> Result<&Arc<sqlx::MySqlPool>, String> {
    match pool {
        DbPool::Mysql(p) => Ok(p),
        _ => Err("Expected MySQL pool".to_string()),
    }
}

fn get_sqlite_pool(pool: &DbPool) -> Result<&Arc<sqlx::SqlitePool>, String> {
    match pool {
        DbPool::Sqlite(p) => Ok(p),
        _ => Err("Expected SQLite pool".to_string()),
    }
}

fn get_mssql_pool(pool: &DbPool) -> Result<&Arc<bb8::Pool<bb8_tiberius::ConnectionManager>>, String> {
    match pool {
        DbPool::Mssql(p) => Ok(p),
        _ => Err("Expected MSSQL pool".to_string()),
    }
}

fn get_dbservice_client(pool: &DbPool) -> Result<&Arc<crate::pool::DbServiceClient>, String> {
    match pool {
        DbPool::DbService(c) => Ok(c),
        _ => Err("Expected DbService client".to_string()),
    }
}

pub async fn list_schemas(
    pool: DbPool,
    driver: &Driver,
) -> Result<Vec<SchemaInfo>, String> {
    match driver {
        Driver::Postgres => pg_schemas(get_pg_pool(&pool)?).await,
        Driver::Mysql => mysql_schemas(get_mysql_pool(&pool)?).await,
        Driver::Sqlite => sqlite_schemas(get_sqlite_pool(&pool)?).await,
        Driver::Mssql => mssql_schemas(get_mssql_pool(&pool)?).await,
        Driver::Dbservice => crate::dbservice::list_schemas(get_dbservice_client(&pool)?).await,
    }
}

pub async fn list_tables(
    pool: DbPool,
    driver: &Driver,
    schema: &str,
) -> Result<Vec<TableInfo>, String> {
    match driver {
        Driver::Postgres => pg_tables(get_pg_pool(&pool)?, schema).await,
        Driver::Mysql => mysql_tables(get_mysql_pool(&pool)?, schema).await,
        Driver::Sqlite => sqlite_tables(get_sqlite_pool(&pool)?, schema).await,
        Driver::Mssql => mssql_tables(get_mssql_pool(&pool)?, schema).await,
        Driver::Dbservice => {
            crate::dbservice::list_tables(get_dbservice_client(&pool)?, schema).await
        }
    }
}

pub async fn list_columns(
    pool: DbPool,
    driver: &Driver,
    schema: &str,
    table: &str,
) -> Result<Vec<ColumnInfo>, String> {
    match driver {
        Driver::Postgres => pg_columns(get_pg_pool(&pool)?, schema, table).await,
        Driver::Mysql => mysql_columns(get_mysql_pool(&pool)?, schema, table).await,
        Driver::Sqlite => sqlite_columns(get_sqlite_pool(&pool)?, schema, table).await,
        Driver::Mssql => mssql_columns(get_mssql_pool(&pool)?, schema, table).await,
        Driver::Dbservice => {
            crate::dbservice::list_columns(get_dbservice_client(&pool)?, schema, table).await
        }
    }
}

pub async fn list_indexes(
    pool: DbPool,
    driver: &Driver,
    schema: &str,
    table: &str,
) -> Result<Vec<IndexInfo>, String> {
    match driver {
        Driver::Postgres => pg_indexes(get_pg_pool(&pool)?, schema, table).await,
        Driver::Mysql => mysql_indexes(get_mysql_pool(&pool)?, schema, table).await,
        Driver::Sqlite => sqlite_indexes(get_sqlite_pool(&pool)?, schema, table).await,
        Driver::Mssql => mssql_indexes(get_mssql_pool(&pool)?, schema, table).await,
        Driver::Dbservice => Ok(Vec::new()),
    }
}

pub async fn list_routines(
    pool: DbPool,
    driver: &Driver,
    schema: &str,
) -> Result<Vec<RoutineInfo>, String> {
    match driver {
        Driver::Postgres => pg_routines(get_pg_pool(&pool)?, schema).await,
        Driver::Mysql => mysql_routines(get_mysql_pool(&pool)?, schema).await,
        Driver::Sqlite => sqlite_routines(get_sqlite_pool(&pool)?, schema).await,
        Driver::Mssql => mssql_routines(get_mssql_pool(&pool)?, schema).await,
        Driver::Dbservice => Ok(Vec::new()),
    }
}

// ── View definition ────────────────────────────────────────

async fn pg_view_definition(
    pool: &sqlx::PgPool,
    schema: &str,
    name: &str,
) -> Result<String, String> {
    let row = sqlx::query(
        "SELECT pg_get_viewdef(c.oid, true) \
         FROM pg_class c \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind IN ('v', 'm') \
         LIMIT 1",
    )
    .bind(schema)
    .bind(name)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    match row {
        Some(r) => {
            let body: String = r.get(0);
            Ok(format!(
                "CREATE OR REPLACE VIEW \"{}\".\"{}\" AS\n{}",
                schema, name, body
            ))
        }
        None => Err(format!("View {}.{} not found", schema, name)),
    }
}

async fn mysql_view_definition(
    pool: &sqlx::MySqlPool,
    _schema: &str,
    name: &str,
) -> Result<String, String> {
    let query = format!("SHOW CREATE VIEW `{}`", name);
    let row = sqlx::query(&query)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;

    match row {
        Some(r) => {
            let def: String = r.try_get(1).unwrap_or_default();
            Ok(def)
        }
        None => Err(format!("View {} not found", name)),
    }
}

async fn sqlite_view_definition(
    pool: &sqlx::SqlitePool,
    _schema: &str,
    name: &str,
) -> Result<String, String> {
    let row = sqlx::query(
        "SELECT sql FROM sqlite_master WHERE type = 'view' AND name = ?1",
    )
    .bind(name)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    match row {
        Some(r) => {
            let def: String = r.get(0);
            Ok(def)
        }
        None => Err(format!("View {} not found", name)),
    }
}

async fn mssql_view_definition(
    pool: &bb8::Pool<bb8_tiberius::ConnectionManager>,
    schema: &str,
    name: &str,
) -> Result<String, String> {
    let mut conn = pool.get().await.map_err(|e| e.to_string())?;
    let query = format!(
        "SELECT OBJECT_DEFINITION(OBJECT_ID('{}.{}'))",
        schema.replace('\'', "''"),
        name.replace('\'', "''"),
    );
    let stream = conn.query(&*query, &[]).await.map_err(|e| e.to_string())?;
    let row = stream
        .into_first_result()
        .await
        .map_err(|e| e.to_string())?;

    match row.first() {
        Some(r) => {
            let def: Option<&str> = r.get(0);
            match def {
                Some(d) => Ok(d.to_string()),
                None => Err(format!("View {}.{} not found", schema, name)),
            }
        }
        None => Err(format!("View {}.{} not found", schema, name)),
    }
}

pub async fn get_view_definition(
    pool: DbPool,
    driver: &Driver,
    schema: &str,
    name: &str,
) -> Result<String, String> {
    match driver {
        Driver::Postgres => pg_view_definition(get_pg_pool(&pool)?, schema, name).await,
        Driver::Mysql => mysql_view_definition(get_mysql_pool(&pool)?, schema, name).await,
        Driver::Sqlite => sqlite_view_definition(get_sqlite_pool(&pool)?, schema, name).await,
        Driver::Mssql => mssql_view_definition(get_mssql_pool(&pool)?, schema, name).await,
        Driver::Dbservice => Err("View definitions not supported via DbService".to_string()),
    }
}

// ── Routine definition ─────────────────────────────────────

async fn pg_routine_definition(
    pool: &sqlx::PgPool,
    schema: &str,
    name: &str,
) -> Result<String, String> {
    let row = sqlx::query(
        "SELECT pg_get_functiondef(p.oid) \
         FROM pg_proc p \
         JOIN pg_namespace n ON n.oid = p.pronamespace \
         WHERE n.nspname = $1 AND p.proname = $2 \
         LIMIT 1",
    )
    .bind(schema)
    .bind(name)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    match row {
        Some(r) => {
            let def: String = r.get(0);
            // pg_get_functiondef returns CREATE, replace with CREATE OR REPLACE
            let alter = if def.starts_with("CREATE FUNCTION") {
                def.replacen("CREATE FUNCTION", "CREATE OR REPLACE FUNCTION", 1)
            } else if def.starts_with("CREATE PROCEDURE") {
                def.replacen("CREATE PROCEDURE", "CREATE OR REPLACE PROCEDURE", 1)
            } else {
                def
            };
            Ok(alter)
        }
        None => Err(format!("Routine {}.{} not found", schema, name)),
    }
}

async fn mysql_routine_definition(
    pool: &sqlx::MySqlPool,
    _schema: &str,
    name: &str,
    routine_type: &str,
) -> Result<String, String> {
    // SHOW CREATE PROCEDURE/FUNCTION returns the full DDL
    let query = if routine_type == "procedure" {
        format!("SHOW CREATE PROCEDURE `{}`", name)
    } else {
        format!("SHOW CREATE FUNCTION `{}`", name)
    };
    let row = sqlx::query(&query)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;

    match row {
        Some(r) => {
            // Column index 2 contains the DDL body
            let def: String = r.try_get(2).unwrap_or_else(|_| {
                r.try_get::<String, _>(1).unwrap_or_default()
            });
            Ok(def)
        }
        None => Err(format!("Routine {} not found", name)),
    }
}

async fn mssql_routine_definition(
    pool: &bb8::Pool<bb8_tiberius::ConnectionManager>,
    schema: &str,
    name: &str,
) -> Result<String, String> {
    let mut conn = pool.get().await.map_err(|e| e.to_string())?;
    let query = format!(
        "SELECT OBJECT_DEFINITION(OBJECT_ID('{}.{}'))",
        schema.replace('\'', "''"),
        name.replace('\'', "''"),
    );
    let stream = conn.query(&*query, &[]).await.map_err(|e| e.to_string())?;
    let row = stream
        .into_first_result()
        .await
        .map_err(|e| e.to_string())?;

    match row.first() {
        Some(r) => {
            let def: Option<&str> = r.get(0);
            match def {
                Some(d) => {
                    // Replace CREATE with ALTER
                    let altered = if let Some(pos) = d.find("PROCEDURE") {
                        if let Some(create_pos) = d[..pos].rfind("CREATE") {
                            format!("ALTER{}", &d[create_pos + "CREATE".len()..])
                        } else {
                            d.to_string()
                        }
                    } else if let Some(pos) = d.find("FUNCTION") {
                        if let Some(create_pos) = d[..pos].rfind("CREATE") {
                            format!("ALTER{}", &d[create_pos + "CREATE".len()..])
                        } else {
                            d.to_string()
                        }
                    } else {
                        d.to_string()
                    };
                    Ok(altered)
                }
                None => Err(format!("Routine {}.{} not found", schema, name)),
            }
        }
        None => Err(format!("Routine {}.{} not found", schema, name)),
    }
}

pub async fn get_routine_definition(
    pool: DbPool,
    driver: &Driver,
    schema: &str,
    name: &str,
    routine_type: &str,
) -> Result<String, String> {
    match driver {
        Driver::Postgres => pg_routine_definition(get_pg_pool(&pool)?, schema, name).await,
        Driver::Mysql => {
            mysql_routine_definition(get_mysql_pool(&pool)?, schema, name, routine_type).await
        }
        Driver::Sqlite => Err("SQLite does not support stored routines".to_string()),
        Driver::Mssql => mssql_routine_definition(get_mssql_pool(&pool)?, schema, name).await,
        Driver::Dbservice => Err("Routine definitions not supported via DbService".to_string()),
    }
}

// ── Foreign keys ────────────────────────────────────────────

async fn pg_foreign_keys(pool: &sqlx::PgPool, schema: &str) -> Result<Vec<ForeignKeyInfo>, String> {
    let rows = sqlx::query(
        "SELECT tc.constraint_name, \
                kcu.table_schema, kcu.table_name, kcu.column_name, kcu.ordinal_position, \
                ccu.table_schema AS target_schema, ccu.table_name AS target_table, ccu.column_name AS target_column \
         FROM information_schema.table_constraints tc \
         JOIN information_schema.key_column_usage kcu \
           ON tc.constraint_name = kcu.constraint_name AND tc.table_schema = kcu.table_schema \
         JOIN information_schema.constraint_column_usage ccu \
           ON ccu.constraint_name = tc.constraint_name AND ccu.table_schema = tc.table_schema \
         WHERE tc.constraint_type = 'FOREIGN KEY' AND tc.table_schema = $1 \
         ORDER BY tc.constraint_name, kcu.ordinal_position",
    )
    .bind(schema)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .map(|r| ForeignKeyInfo {
            constraint_name: r.get(0),
            source_schema: r.get(1),
            source_table: r.get(2),
            source_column: r.get(3),
            ordinal: r.get::<i32, _>(4),
            target_schema: r.get(5),
            target_table: r.get(6),
            target_column: r.get(7),
        })
        .collect())
}

async fn mysql_foreign_keys(
    pool: &sqlx::MySqlPool,
    schema: &str,
) -> Result<Vec<ForeignKeyInfo>, String> {
    let rows = sqlx::query(
        "SELECT constraint_name, table_schema, table_name, column_name, ordinal_position, \
                referenced_table_schema, referenced_table_name, referenced_column_name \
         FROM information_schema.key_column_usage \
         WHERE referenced_table_name IS NOT NULL AND table_schema = ? \
         ORDER BY constraint_name, ordinal_position",
    )
    .bind(schema)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .map(|r| ForeignKeyInfo {
            constraint_name: r.get(0),
            source_schema: r.get(1),
            source_table: r.get(2),
            source_column: r.get(3),
            ordinal: r.get::<i32, _>(4),
            target_schema: r.get(5),
            target_table: r.get(6),
            target_column: r.get(7),
        })
        .collect())
}

async fn sqlite_foreign_keys(
    pool: &sqlx::SqlitePool,
    _schema: &str,
) -> Result<Vec<ForeignKeyInfo>, String> {
    // Enumerate tables, then query pragma_foreign_key_list per table.
    let table_rows = sqlx::query(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut result = Vec::new();
    for tr in &table_rows {
        let table_name: String = tr.get(0);
        let sql = format!(
            "SELECT id, seq, \"table\", \"from\", \"to\" FROM pragma_foreign_key_list('{}')",
            table_name.replace('\'', "''")
        );
        let fk_rows = sqlx::query(&sql)
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?;

        for fr in &fk_rows {
            let fk_id: i32 = fr.get(0);
            let seq: i32 = fr.get(1);
            let target_table: String = fr.get(2);
            let source_column: String = fr.get(3);
            let target_column: Option<String> = fr.try_get(4).ok();
            result.push(ForeignKeyInfo {
                constraint_name: format!("{}__fk{}", table_name, fk_id),
                source_schema: "main".to_string(),
                source_table: table_name.clone(),
                source_column,
                ordinal: seq + 1,
                target_schema: "main".to_string(),
                target_table,
                target_column: target_column.unwrap_or_default(),
            });
        }
    }
    Ok(result)
}

async fn mssql_foreign_keys(
    pool: &bb8::Pool<bb8_tiberius::ConnectionManager>,
    schema: &str,
) -> Result<Vec<ForeignKeyInfo>, String> {
    let mut conn = pool.get().await.map_err(|e| e.to_string())?;
    let query = format!(
        "SELECT fk.name, SCHEMA_NAME(fk.schema_id), OBJECT_NAME(fk.parent_object_id), pc.name, \
                fkc.constraint_column_id, OBJECT_SCHEMA_NAME(fk.referenced_object_id), \
                OBJECT_NAME(fk.referenced_object_id), rc.name \
         FROM sys.foreign_keys fk \
         JOIN sys.foreign_key_columns fkc ON fk.object_id = fkc.constraint_object_id \
         JOIN sys.columns pc ON fkc.parent_object_id = pc.object_id AND fkc.parent_column_id = pc.column_id \
         JOIN sys.columns rc ON fkc.referenced_object_id = rc.object_id AND fkc.referenced_column_id = rc.column_id \
         WHERE SCHEMA_NAME(fk.schema_id) = '{}' \
         ORDER BY fk.name, fkc.constraint_column_id",
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
            let src_schema = r.get::<&str, _>(1).unwrap_or(schema);
            let src_table = r.get::<&str, _>(2)?;
            let src_column = r.get::<&str, _>(3)?;
            let ordinal = r.get::<i32, _>(4).unwrap_or(0);
            let tgt_schema = r.get::<&str, _>(5).unwrap_or("dbo");
            let tgt_table = r.get::<&str, _>(6)?;
            let tgt_column = r.get::<&str, _>(7)?;
            Some(ForeignKeyInfo {
                constraint_name: name.to_string(),
                source_schema: src_schema.to_string(),
                source_table: src_table.to_string(),
                source_column: src_column.to_string(),
                ordinal,
                target_schema: tgt_schema.to_string(),
                target_table: tgt_table.to_string(),
                target_column: tgt_column.to_string(),
            })
        })
        .collect())
}

pub async fn list_foreign_keys(
    pool: DbPool,
    driver: &Driver,
    schema: &str,
) -> Result<Vec<ForeignKeyInfo>, String> {
    match driver {
        Driver::Postgres => pg_foreign_keys(get_pg_pool(&pool)?, schema).await,
        Driver::Mysql => mysql_foreign_keys(get_mysql_pool(&pool)?, schema).await,
        Driver::Sqlite => sqlite_foreign_keys(get_sqlite_pool(&pool)?, schema).await,
        Driver::Mssql => mssql_foreign_keys(get_mssql_pool(&pool)?, schema).await,
        Driver::Dbservice => Ok(Vec::new()),
    }
}
