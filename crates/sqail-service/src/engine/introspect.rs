//! Catalog queries per engine. All user-supplied names travel as bind
//! parameters; the only identifier ever spliced into SQL is an SQLite schema
//! name, and only after it matched an attached database.

use serde_json::Value;
use sqail_proto::{
    ColumnInfo, Ddl, Engine, ForeignKeyInfo, IndexInfo, NamedItem, Param, RoutineInfo, RoutineKind,
    SchemaObject, SchemaObjectKind, TableGrant, TableInfo, TableKind, TablePrivileges,
};

use super::{Conn, DbError, Result, fetch_rows};

/// Separator for aggregated name lists (never appears in identifiers).
const SEP: char = '\u{1f}';

fn param(s: Option<&str>) -> Param {
    s.map(|s| Param::Text(s.to_string())).unwrap_or(Param::Null)
}

fn get_str(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

fn get_bool(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_i64().unwrap_or(0) != 0,
        Value::String(s) => matches!(s.as_str(), "t" | "true" | "1"),
        _ => false,
    }
}

fn get_i64(v: &Value) -> i64 {
    match v {
        Value::Number(n) => n.as_i64().unwrap_or(0),
        Value::String(s) => s.parse().unwrap_or(0),
        _ => 0,
    }
}

fn split_list(v: &Value) -> Vec<String> {
    get_str(v)
        .map(|s| s.split(SEP).map(str::to_string).collect())
        .unwrap_or_default()
}

async fn rows(conn: &mut dyn Conn, sql: &str, params: Vec<Param>) -> Result<Vec<Vec<Value>>> {
    fetch_rows(conn, sql, params).await
}

// ------------------------------------------------------------ databases --

pub async fn databases(conn: &mut dyn Conn) -> Result<Vec<NamedItem>> {
    let sql = match conn.engine() {
        Engine::Unknown => return Err(DbError::unknown_engine()),
        Engine::Postgres => {
            "SELECT datname::text FROM pg_database WHERE NOT datistemplate AND datallowconn ORDER BY 1"
        }
        Engine::Mssql => {
            "SELECT name FROM sys.databases WHERE HAS_DBACCESS(name) = 1 ORDER BY name"
        }
        Engine::Sqlite => "SELECT name FROM pragma_database_list ORDER BY seq",
    };
    names(conn, sql, vec![]).await
}

pub async fn schemas(conn: &mut dyn Conn) -> Result<Vec<NamedItem>> {
    let sql = match conn.engine() {
        Engine::Unknown => return Err(DbError::unknown_engine()),
        Engine::Postgres => {
            "SELECT nspname::text FROM pg_namespace
             WHERE nspname NOT LIKE 'pg\\_%' AND nspname <> 'information_schema' ORDER BY 1"
        }
        Engine::Mssql => {
            "SELECT name FROM sys.schemas
             WHERE name NOT IN ('sys', 'INFORMATION_SCHEMA', 'guest') AND name NOT LIKE 'db[_]%'
             ORDER BY name"
        }
        Engine::Sqlite => "SELECT name FROM pragma_database_list ORDER BY seq",
    };
    names(conn, sql, vec![]).await
}

async fn names(conn: &mut dyn Conn, sql: &str, params: Vec<Param>) -> Result<Vec<NamedItem>> {
    Ok(rows(conn, sql, params)
        .await?
        .iter()
        .filter_map(|r| get_str(&r[0]).map(|name| NamedItem { name }))
        .collect())
}

// --------------------------------------------------------------- tables --

pub async fn tables(conn: &mut dyn Conn, schema: Option<&str>) -> Result<Vec<TableInfo>> {
    let (sql, params) = match conn.engine() {
        Engine::Unknown => return Err(DbError::unknown_engine()),
        Engine::Postgres => (
            // A partition names its parent when that is in the same schema.
            "SELECT n.nspname::text, c.relname::text, c.relkind::text,
                    (SELECT pc.relname::text FROM pg_inherits i
                     JOIN pg_class pc ON pc.oid = i.inhparent
                     WHERE c.relispartition AND i.inhrelid = c.oid
                       AND pc.relnamespace = c.relnamespace)
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f')
               AND ($1::text IS NULL OR n.nspname = $1)
               AND n.nspname NOT IN ('pg_catalog', 'information_schema')
               AND n.nspname NOT LIKE 'pg\\_toast%'
             ORDER BY 1, 2"
                .to_string(),
            vec![param(schema)],
        ),
        Engine::Mssql => (
            "SELECT s.name, o.name, RTRIM(o.type), NULL
             FROM sys.objects o JOIN sys.schemas s ON s.schema_id = o.schema_id
             WHERE o.type IN ('U', 'V') AND o.is_ms_shipped = 0 AND (@P1 IS NULL OR s.name = @P1)
             ORDER BY s.name, o.name"
                .to_string(),
            vec![param(schema)],
        ),
        Engine::Sqlite => {
            let db = sqlite_schema(conn, schema).await?;
            (
                format!(
                    "SELECT '{}', name, type, NULL FROM {}.sqlite_schema
                     WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\'
                     ORDER BY name",
                    db.name.replace('\'', "''"),
                    db.quoted
                ),
                vec![],
            )
        }
    };
    Ok(rows(conn, &sql, params)
        .await?
        .iter()
        .filter_map(|r| {
            let kind = get_str(&r[2]);
            Some(TableInfo {
                schema: get_str(&r[0]),
                name: get_str(&r[1])?,
                kind: match kind.as_deref() {
                    Some("v" | "V" | "view") => TableKind::View,
                    Some("m") => TableKind::MaterializedView,
                    _ => TableKind::Table,
                },
                foreign: kind.as_deref() == Some("f"),
                partitioned: kind.as_deref() == Some("p"),
                partition_of: get_str(&r[3]),
            })
        })
        .collect())
}

struct SqliteDb {
    name: String,
    quoted: String,
}

/// Validate an SQLite schema name against the attached databases.
async fn sqlite_schema(conn: &mut dyn Conn, schema: Option<&str>) -> Result<SqliteDb> {
    let name = schema.unwrap_or("main");
    let attached = rows(conn, "SELECT name FROM pragma_database_list", vec![]).await?;
    if !attached
        .iter()
        .any(|r| get_str(&r[0]).as_deref() == Some(name))
    {
        return Err(DbError::Invalid(format!("unknown schema '{name}'")));
    }
    Ok(SqliteDb {
        name: name.to_string(),
        quoted: format!("\"{}\"", name.replace('"', "\"\"")),
    })
}

// -------------------------------------------------------------- columns --

/// SQL Server: a column's type as written in DDL (`c` is `sys.columns`,
/// `t` its `sys.types` row).
macro_rules! ms_type {
    () => {
        "CASE WHEN t.name IN ('varchar', 'char', 'varbinary', 'binary')
                   THEN t.name + '(' + IIF(c.max_length = -1, 'max', CAST(c.max_length AS varchar(10))) + ')'
              WHEN t.name IN ('nvarchar', 'nchar')
                   THEN t.name + '(' + IIF(c.max_length = -1, 'max', CAST(c.max_length / 2 AS varchar(10))) + ')'
              WHEN t.name IN ('decimal', 'numeric')
                   THEN t.name + '(' + CAST(c.precision AS varchar(3)) + ',' + CAST(c.scale AS varchar(3)) + ')'
              WHEN t.name IN ('datetime2', 'time', 'datetimeoffset')
                   THEN t.name + '(' + CAST(c.scale AS varchar(3)) + ')'
              ELSE t.name END"
    };
}

/// The schema an unqualified `table` resolves to on `conn`, the way the
/// engine resolves it in a statement: PostgreSQL's search path, SQL Server's
/// default schema then `dbo`, SQLite's `temp`, `main`, then attached
/// databases. `None` when no such table or view is visible.
pub async fn resolve_schema(conn: &mut dyn Conn, table: &str) -> Result<Option<String>> {
    let sql = match conn.engine() {
        Engine::Unknown => return Err(DbError::unknown_engine()),
        Engine::Postgres => {
            "SELECT n.nspname::text FROM pg_class c
             JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE c.oid = to_regclass(quote_ident($1))"
        }
        Engine::Mssql => "SELECT OBJECT_SCHEMA_NAME(OBJECT_ID(QUOTENAME(@P1)))",
        Engine::Sqlite => {
            let dbs = rows(
                conn,
                "SELECT name FROM pragma_database_list ORDER BY name <> 'temp', seq",
                vec![],
            )
            .await?;
            for db in dbs.iter().filter_map(|r| get_str(&r[0])) {
                let sql = format!(
                    "SELECT 1 FROM \"{}\".sqlite_master
                     WHERE type IN ('table', 'view') AND name = ?1 COLLATE NOCASE",
                    db.replace('"', "\"\"")
                );
                if !rows(conn, &sql, vec![param(Some(table))]).await?.is_empty() {
                    return Ok(Some(db));
                }
            }
            return Ok(None);
        }
    };
    Ok(rows(conn, sql, vec![param(Some(table))])
        .await?
        .first()
        .and_then(|r| get_str(&r[0])))
}

pub async fn columns(
    conn: &mut dyn Conn,
    schema: Option<&str>,
    table: &str,
) -> Result<Vec<ColumnInfo>> {
    let (sql, params) = match conn.engine() {
        Engine::Unknown => return Err(DbError::unknown_engine()),
        Engine::Postgres => (
            "SELECT a.attname::text, a.attnum::int8, format_type(a.atttypid, a.atttypmod)::text,
                    (NOT a.attnotnull)::bool, pg_get_expr(d.adbin, d.adrelid)::text,
                    COALESCE(a.attnum = ANY(i.indkey), false)::bool
             FROM pg_attribute a
             JOIN pg_class c ON c.oid = a.attrelid
             JOIN pg_namespace n ON n.oid = c.relnamespace
             LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
             LEFT JOIN pg_index i ON i.indrelid = c.oid AND i.indisprimary
             WHERE n.nspname = COALESCE($1, current_schema()) AND c.relname = $2
               AND a.attnum > 0 AND NOT a.attisdropped
             ORDER BY a.attnum",
            vec![param(schema), param(Some(table))],
        ),
        Engine::Mssql => (
            concat!(
                "SELECT c.name, c.column_id, ",
                ms_type!(),
                ",
                    c.is_nullable, dc.definition,
                    CAST(CASE WHEN pk.column_id IS NULL THEN 0 ELSE 1 END AS bit)
             FROM sys.columns c
             JOIN sys.objects o ON o.object_id = c.object_id
             JOIN sys.schemas s ON s.schema_id = o.schema_id
             JOIN sys.types t ON t.user_type_id = c.user_type_id
             LEFT JOIN sys.default_constraints dc ON dc.object_id = c.default_object_id
             LEFT JOIN (SELECT ic.object_id, ic.column_id FROM sys.indexes i
                        JOIN sys.index_columns ic ON ic.object_id = i.object_id AND ic.index_id = i.index_id
                        WHERE i.is_primary_key = 1) pk
                    ON pk.object_id = c.object_id AND pk.column_id = c.column_id
             WHERE s.name = COALESCE(@P1, SCHEMA_NAME()) AND o.name = @P2
             ORDER BY c.column_id"
            ),
            vec![param(schema), param(Some(table))],
        ),
        Engine::Sqlite => {
            sqlite_schema(conn, schema).await?;
            (
                "SELECT name, cid + 1, type, \"notnull\" = 0 AND pk = 0, dflt_value, pk > 0
                 FROM pragma_table_info(?1, ?2) ORDER BY cid",
                vec![param(Some(table)), param(Some(schema.unwrap_or("main")))],
            )
        }
    };
    Ok(rows(conn, sql, params)
        .await?
        .iter()
        .filter_map(|r| {
            Some(ColumnInfo {
                name: get_str(&r[0])?,
                ordinal: get_i64(&r[1]),
                data_type: get_str(&r[2]).unwrap_or_default(),
                nullable: get_bool(&r[3]),
                default: get_str(&r[4]),
                primary_key: get_bool(&r[5]),
            })
        })
        .collect())
}

// -------------------------------------------------------------- indexes --

pub async fn indexes(
    conn: &mut dyn Conn,
    schema: Option<&str>,
    table: &str,
) -> Result<Vec<IndexInfo>> {
    let (sql, params) = match conn.engine() {
        Engine::Unknown => return Err(DbError::unknown_engine()),
        Engine::Postgres => (
            "SELECT ic.relname::text,
                    (SELECT string_agg(pg_get_indexdef(i.indexrelid, k + 1, true), chr(31) ORDER BY k)
                     FROM generate_series(0, i.indnkeyatts - 1) k)::text,
                    i.indisunique, i.indisprimary,
                    EXISTS (SELECT 1 FROM pg_constraint con
                            WHERE con.conindid = i.indexrelid AND con.conrelid = i.indrelid
                              AND con.contype IN ('p', 'u', 'x')),
                    (SELECT string_agg((i.indoption[k] & 1)::text, '' ORDER BY k)
                     FROM generate_series(0, i.indnkeyatts - 1) k)::text,
                    (SELECT string_agg(pg_get_indexdef(i.indexrelid, k + 1, true), chr(31) ORDER BY k)
                     FROM generate_series(i.indnkeyatts, i.indnatts - 1) k)::text,
                    pg_get_expr(i.indpred, i.indrelid, true)::text,
                    NULL::text
             FROM pg_index i
             JOIN pg_class ic ON ic.oid = i.indexrelid
             JOIN pg_class c ON c.oid = i.indrelid
             JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = COALESCE($1, current_schema()) AND c.relname = $2
             ORDER BY 1",
            vec![param(schema), param(Some(table))],
        ),
        Engine::Mssql => (MS_INDEXES, vec![param(schema), param(Some(table))]),
        Engine::Sqlite => {
            sqlite_schema(conn, schema).await?;
            (
                "SELECT il.name,
                        (SELECT group_concat(name, char(31))
                         FROM (SELECT name FROM pragma_index_info(il.name, ?2) ORDER BY seqno)),
                        il.\"unique\", il.origin = 'pk', il.origin <> 'c',
                        NULL, NULL, NULL, NULL
                 FROM pragma_index_list(?1, ?2) il ORDER BY il.name",
                vec![param(Some(table)), param(Some(schema.unwrap_or("main")))],
            )
        }
    };
    Ok(rows(conn, sql, params)
        .await?
        .iter()
        .filter_map(|r| index_info(r))
        .collect())
}

/// SQL Server indexes: name, key columns, unique, primary, constraint,
/// descending flags ("0"/"1" per key), included columns, filter, type.
/// Indexes without key columns (columnstore) are left out.
const MS_INDEXES: &str = "SELECT i.name,
        (SELECT STRING_AGG(c.name, CHAR(31)) WITHIN GROUP (ORDER BY ic.key_ordinal)
         FROM sys.index_columns ic
         JOIN sys.columns c ON c.object_id = ic.object_id AND c.column_id = ic.column_id
         WHERE ic.object_id = i.object_id AND ic.index_id = i.index_id AND ic.is_included_column = 0),
        i.is_unique, i.is_primary_key,
        CAST(i.is_primary_key | i.is_unique_constraint AS bit),
        (SELECT STRING_AGG(CAST(ic.is_descending_key AS varchar(1)), '') WITHIN GROUP (ORDER BY ic.key_ordinal)
         FROM sys.index_columns ic
         WHERE ic.object_id = i.object_id AND ic.index_id = i.index_id AND ic.is_included_column = 0),
        (SELECT STRING_AGG(c.name, CHAR(31)) WITHIN GROUP (ORDER BY ic.index_column_id)
         FROM sys.index_columns ic
         JOIN sys.columns c ON c.object_id = ic.object_id AND c.column_id = ic.column_id
         WHERE ic.object_id = i.object_id AND ic.index_id = i.index_id AND ic.is_included_column = 1),
        i.filter_definition, i.type_desc
 FROM sys.indexes i
 JOIN sys.objects o ON o.object_id = i.object_id
 JOIN sys.schemas s ON s.schema_id = o.schema_id
 WHERE s.name = COALESCE(@P1, SCHEMA_NAME()) AND o.name = @P2 AND i.name IS NOT NULL
   AND EXISTS (SELECT 1 FROM sys.index_columns ic
               WHERE ic.object_id = i.object_id AND ic.index_id = i.index_id
                 AND ic.is_included_column = 0)
 ORDER BY i.name";

/// One row of an index query (see [`MS_INDEXES`] for the column order).
fn index_info(r: &[Value]) -> Option<IndexInfo> {
    let descending: Vec<bool> = get_str(&r[5])
        .unwrap_or_default()
        .chars()
        .map(|c| c == '1')
        .collect();
    Some(IndexInfo {
        name: get_str(&r[0])?,
        columns: split_list(&r[1]),
        unique: get_bool(&r[2]),
        primary: get_bool(&r[3]),
        constraint: get_bool(&r[4]),
        descending: if descending.contains(&true) {
            descending
        } else {
            Vec::new()
        },
        include: split_list(&r[6]),
        filter: get_str(&r[7]),
        clustered: match get_str(&r[8]).as_deref() {
            Some("CLUSTERED") => Some(true),
            Some("NONCLUSTERED") => Some(false),
            _ => None,
        },
    })
}

// --------------------------------------------------------- foreign keys --

pub async fn foreign_keys(
    conn: &mut dyn Conn,
    schema: Option<&str>,
    table: &str,
) -> Result<Vec<ForeignKeyInfo>> {
    let (sql, params) = match conn.engine() {
        Engine::Unknown => return Err(DbError::unknown_engine()),
        Engine::Postgres => (
            "SELECT con.conname::text,
                    (SELECT string_agg(a.attname, chr(31) ORDER BY k.ord)
                     FROM unnest(con.conkey) WITH ORDINALITY k(attnum, ord)
                     JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.attnum)::text,
                    rn.nspname::text, rc.relname::text,
                    (SELECT string_agg(a.attname, chr(31) ORDER BY k.ord)
                     FROM unnest(con.confkey) WITH ORDINALITY k(attnum, ord)
                     JOIN pg_attribute a ON a.attrelid = con.confrelid AND a.attnum = k.attnum)::text
             FROM pg_constraint con
             JOIN pg_class c ON c.oid = con.conrelid
             JOIN pg_namespace n ON n.oid = c.relnamespace
             JOIN pg_class rc ON rc.oid = con.confrelid
             JOIN pg_namespace rn ON rn.oid = rc.relnamespace
             WHERE con.contype = 'f' AND n.nspname = COALESCE($1, current_schema()) AND c.relname = $2
             ORDER BY 1",
            vec![param(schema), param(Some(table))],
        ),
        Engine::Mssql => (MS_FOREIGN_KEYS, vec![param(schema), param(Some(table))]),
        Engine::Sqlite => {
            sqlite_schema(conn, schema).await?;
            let raw = rows(
                conn,
                "SELECT id, \"table\", \"from\", \"to\" FROM pragma_foreign_key_list(?1, ?2) ORDER BY id, seq",
                vec![param(Some(table)), param(Some(schema.unwrap_or("main")))],
            )
            .await?;
            let mut out: Vec<(i64, ForeignKeyInfo)> = Vec::new();
            for r in raw {
                let id = get_i64(&r[0]);
                if out.last().is_none_or(|(last, _)| *last != id) {
                    out.push((
                        id,
                        ForeignKeyInfo {
                            name: format!("fk_{table}_{id}"),
                            columns: vec![],
                            ref_schema: schema.map(str::to_string),
                            ref_table: get_str(&r[1]).unwrap_or_default(),
                            ref_columns: vec![],
                        },
                    ));
                }
                let fk = &mut out.last_mut().expect("pushed above").1;
                fk.columns.extend(get_str(&r[2]));
                // `to` is NULL when the FK references the primary key implicitly.
                fk.ref_columns.extend(get_str(&r[3]));
            }
            return Ok(out.into_iter().map(|(_, fk)| fk).collect());
        }
    };
    Ok(rows(conn, sql, params)
        .await?
        .iter()
        .filter_map(|r| {
            Some(ForeignKeyInfo {
                name: get_str(&r[0])?,
                columns: split_list(&r[1]),
                ref_schema: get_str(&r[2]),
                ref_table: get_str(&r[3])?,
                ref_columns: split_list(&r[4]),
            })
        })
        .collect())
}

/// SQL Server foreign keys: name, columns, referenced schema, table and
/// columns, then (for scripting) the ON DELETE and ON UPDATE actions,
/// whether the name is generated, and NOT FOR REPLICATION.
const MS_FOREIGN_KEYS: &str = "SELECT fk.name,
        STRING_AGG(pc.name, CHAR(31)) WITHIN GROUP (ORDER BY fkc.constraint_column_id),
        rs.name, rt.name,
        STRING_AGG(rc.name, CHAR(31)) WITHIN GROUP (ORDER BY fkc.constraint_column_id),
        fk.delete_referential_action_desc, fk.update_referential_action_desc,
        fk.is_system_named, fk.is_not_for_replication
 FROM sys.foreign_keys fk
 JOIN sys.foreign_key_columns fkc ON fkc.constraint_object_id = fk.object_id
 JOIN sys.columns pc ON pc.object_id = fkc.parent_object_id AND pc.column_id = fkc.parent_column_id
 JOIN sys.columns rc ON rc.object_id = fkc.referenced_object_id AND rc.column_id = fkc.referenced_column_id
 JOIN sys.objects rt ON rt.object_id = fk.referenced_object_id
 JOIN sys.schemas rs ON rs.schema_id = rt.schema_id
 JOIN sys.objects p ON p.object_id = fk.parent_object_id
 JOIN sys.schemas ps ON ps.schema_id = p.schema_id
 WHERE ps.name = COALESCE(@P1, SCHEMA_NAME()) AND p.name = @P2
 GROUP BY fk.name, rs.name, rt.name, fk.delete_referential_action_desc,
          fk.update_referential_action_desc, fk.is_system_named, fk.is_not_for_replication
 ORDER BY fk.name";

// ------------------------------------------------------------- routines --

pub async fn routines(conn: &mut dyn Conn, schema: Option<&str>) -> Result<Vec<RoutineInfo>> {
    let (sql, params) = match conn.engine() {
        Engine::Unknown => return Err(DbError::unknown_engine()),
        Engine::Postgres => (
            "SELECT n.nspname::text, p.proname::text,
                    CASE WHEN p.prokind = 'f' AND p.proretset THEN 'tf' ELSE p.prokind::text END,
                    pg_get_function_identity_arguments(p.oid)::text
             FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
             WHERE ($1::text IS NULL OR n.nspname = $1)
               AND n.nspname NOT IN ('pg_catalog', 'information_schema')
               AND p.prokind IN ('f', 'p')
             ORDER BY 1, 2, 4",
            vec![param(schema)],
        ),
        Engine::Mssql => (
            "SELECT s.name, o.name, RTRIM(o.type), NULL
             FROM sys.objects o JOIN sys.schemas s ON s.schema_id = o.schema_id
             WHERE o.type IN ('P', 'FN', 'IF', 'TF') AND o.is_ms_shipped = 0
               AND (@P1 IS NULL OR s.name = @P1)
             ORDER BY s.name, o.name",
            vec![param(schema)],
        ),
        Engine::Sqlite => return Ok(Vec::new()),
    };
    Ok(rows(conn, sql, params)
        .await?
        .iter()
        .filter_map(|r| {
            let kind = get_str(&r[2]);
            Some(RoutineInfo {
                schema: get_str(&r[0]),
                name: get_str(&r[1])?,
                kind: match kind.as_deref() {
                    Some("p" | "P") => RoutineKind::Procedure,
                    _ => RoutineKind::Function,
                },
                // SQL Server: inline (IF) and multi-statement (TF) table-valued
                // functions; Postgres: set-returning functions ('tf' above).
                table_valued: matches!(kind.as_deref(), Some("IF" | "TF" | "tf")),
                arguments: get_str(&r[3]),
            })
        })
        .collect())
}

// -------------------------------------------------------------- objects --

/// Sequences, user-defined types, synonyms and triggers, ordered by kind,
/// schema and name. Objects that belong to an extension are left out.
pub async fn objects(conn: &mut dyn Conn, schema: Option<&str>) -> Result<Vec<SchemaObject>> {
    let (sql, params) = match conn.engine() {
        Engine::Unknown => return Err(DbError::unknown_engine()),
        Engine::Postgres => (
            "WITH ext AS (SELECT objid FROM pg_depend WHERE deptype = 'e')
             SELECT n.nspname::text, c.relname::text, 'sequence', NULL::text,
                    format_type(s.seqtypid, NULL)::text
             FROM pg_sequence s JOIN pg_class c ON c.oid = s.seqrelid
             JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE ($1::text IS NULL OR n.nspname = $1) AND c.oid NOT IN (SELECT objid FROM ext)
             UNION ALL
             SELECT n.nspname::text, t.typname::text, 'type', NULL,
                    CASE t.typtype WHEN 'e' THEN 'enum' WHEN 'd' THEN 'domain'
                                   WHEN 'c' THEN 'composite' WHEN 'r' THEN 'range' END
             FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace
             WHERE t.typtype IN ('e', 'd', 'c', 'r')
               AND (t.typtype <> 'c'
                    OR (SELECT relkind FROM pg_class WHERE oid = t.typrelid) = 'c')
               AND ($1::text IS NULL OR n.nspname = $1)
               AND n.nspname NOT IN ('pg_catalog', 'information_schema')
               AND t.oid NOT IN (SELECT objid FROM ext)
             UNION ALL
             SELECT n.nspname::text, g.tgname::text, 'trigger', c.relname::text, NULL
             FROM pg_trigger g JOIN pg_class c ON c.oid = g.tgrelid
             JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE NOT g.tgisinternal AND g.tgparentid = 0
               AND ($1::text IS NULL OR n.nspname = $1)
               AND n.nspname NOT IN ('pg_catalog', 'information_schema')
             ORDER BY 3, 1, 2"
                .to_string(),
            vec![param(schema)],
        ),
        Engine::Mssql => (
            "SELECT SCHEMA_NAME(q.schema_id), q.name, 'sequence', NULL, TYPE_NAME(q.user_type_id)
             FROM sys.sequences q WHERE @P1 IS NULL OR SCHEMA_NAME(q.schema_id) = @P1
             UNION ALL
             SELECT SCHEMA_NAME(t.schema_id), t.name, 'type', NULL,
                    CASE WHEN t.is_table_type = 1 THEN 'table type'
                         WHEN t.is_assembly_type = 1 THEN 'CLR type'
                         ELSE TYPE_NAME(t.system_type_id) END
             FROM sys.types t
             WHERE t.is_user_defined = 1 AND (@P1 IS NULL OR SCHEMA_NAME(t.schema_id) = @P1)
             UNION ALL
             SELECT SCHEMA_NAME(y.schema_id), y.name, 'synonym', NULL, y.base_object_name
             FROM sys.synonyms y WHERE @P1 IS NULL OR SCHEMA_NAME(y.schema_id) = @P1
             UNION ALL
             SELECT SCHEMA_NAME(o.schema_id), tr.name, 'trigger', o.name, NULL
             FROM sys.triggers tr JOIN sys.objects o ON o.object_id = tr.parent_id
             WHERE tr.parent_class = 1 AND tr.is_ms_shipped = 0
               AND (@P1 IS NULL OR SCHEMA_NAME(o.schema_id) = @P1)
             ORDER BY 3, 1, 2"
                .to_string(),
            vec![param(schema)],
        ),
        Engine::Sqlite => {
            let db = sqlite_schema(conn, schema).await?;
            (
                format!(
                    "SELECT '{}', name, 'trigger', tbl_name, NULL FROM {}.sqlite_schema
                     WHERE type = 'trigger' ORDER BY name",
                    db.name.replace('\'', "''"),
                    db.quoted
                ),
                vec![],
            )
        }
    };
    Ok(rows(conn, &sql, params)
        .await?
        .iter()
        .filter_map(|r| {
            Some(SchemaObject {
                schema: get_str(&r[0]),
                name: get_str(&r[1])?,
                kind: match get_str(&r[2]).as_deref() {
                    Some("sequence") => SchemaObjectKind::Sequence,
                    Some("type") => SchemaObjectKind::Type,
                    Some("synonym") => SchemaObjectKind::Synonym,
                    Some("trigger") => SchemaObjectKind::Trigger,
                    _ => SchemaObjectKind::Other,
                },
                table: get_str(&r[3]),
                detail: get_str(&r[4]),
            })
        })
        .collect())
}

// ----------------------------------------------------------- privileges --

pub async fn privileges(
    conn: &mut dyn Conn,
    schema: Option<&str>,
    table: &str,
) -> Result<TablePrivileges> {
    let (grants_sql, principals_sql, params) = match conn.engine() {
        Engine::Unknown => return Err(DbError::unknown_engine()),
        Engine::Sqlite => {
            return Ok(TablePrivileges {
                supported: false,
                owner: None,
                grants: Vec::new(),
                principals: Vec::new(),
            });
        }
        Engine::Postgres => (
            "SELECT CASE WHEN a.grantee = 0 THEN 'PUBLIC' ELSE pg_get_userbyid(a.grantee) END::text,
                    a.privilege_type::text, a.is_grantable, pg_get_userbyid(c.relowner)::text
             FROM pg_class c
             JOIN pg_namespace n ON n.oid = c.relnamespace
             LEFT JOIN LATERAL aclexplode(c.relacl) a ON a.grantee <> c.relowner
             WHERE n.nspname = COALESCE($1, current_schema()) AND c.relname = $2
             ORDER BY 1, 2",
            "SELECT rolname::text FROM pg_roles WHERE rolname NOT LIKE 'pg\\_%'
             UNION ALL SELECT 'PUBLIC' ORDER BY 1",
            vec![param(schema), param(Some(table))],
        ),
        Engine::Mssql => (
            "SELECT pr.name, p.permission_name, CAST(IIF(p.state = 'W', 1, 0) AS bit), NULL
             FROM sys.database_permissions p
             JOIN sys.database_principals pr ON pr.principal_id = p.grantee_principal_id
             WHERE p.class = 1 AND p.minor_id = 0 AND p.state IN ('G', 'W')
               AND p.major_id = OBJECT_ID(QUOTENAME(COALESCE(@P1, SCHEMA_NAME())) + '.' + QUOTENAME(@P2))
             ORDER BY pr.name, p.permission_name",
            "SELECT name FROM sys.database_principals
             WHERE type IN ('S', 'U', 'G', 'R', 'E', 'X') AND is_fixed_role = 0
               AND name NOT IN ('sys', 'INFORMATION_SCHEMA', 'dbo')
             ORDER BY name",
            vec![param(schema), param(Some(table))],
        ),
    };
    let raw = rows(conn, grants_sql, params).await?;
    if raw.is_empty() && conn.engine() == Engine::Postgres {
        return Err(DbError::Invalid(format!("table '{table}' not found")));
    }
    let owner = raw.first().and_then(|r| get_str(&r[3]));
    let grants = raw
        .iter()
        .filter_map(|r| {
            Some(TableGrant {
                grantee: get_str(&r[0])?,
                privilege: get_str(&r[1])?,
                grantable: get_bool(&r[2]),
            })
        })
        .collect();
    let principals = names(conn, principals_sql, vec![])
        .await?
        .into_iter()
        .map(|n| n.name)
        .collect();
    Ok(TablePrivileges {
        supported: true,
        owner,
        grants,
        principals,
    })
}

// ------------------------------------------------------------------ ddl --

/// `arguments` (PostgreSQL) picks one overload of a routine by its identity
/// arguments, as [`routines`] lists them; without it every overload is
/// scripted.
pub async fn ddl(
    conn: &mut dyn Conn,
    schema: Option<&str>,
    name: &str,
    arguments: Option<&str>,
) -> Result<Ddl> {
    let not_found = || DbError::Invalid(format!("object '{name}' not found"));
    let ddl = match conn.engine() {
        Engine::Unknown => return Err(DbError::unknown_engine()),
        Engine::Sqlite => {
            let db = sqlite_schema(conn, schema).await?;
            let sql = format!(
                "SELECT sql FROM {}.sqlite_schema WHERE tbl_name = ?1 AND sql IS NOT NULL
                 ORDER BY type NOT IN ('table', 'view'), name",
                db.quoted
            );
            let parts: Vec<String> = rows(conn, &sql, vec![param(Some(name))])
                .await?
                .iter()
                .filter_map(|r| get_str(&r[0]))
                .map(|s| format!("{s};"))
                .collect();
            if parts.is_empty() {
                return Err(not_found());
            }
            parts.join("\n\n")
        }
        Engine::Postgres => {
            // Names are quoted by the server: its quote_ident knows every
            // keyword the running version reserves.
            let kind = rows(
                conn,
                "SELECT c.relkind::text, c.oid::int8, n.nspname::text,
                        (quote_ident(n.nspname) || '.' || quote_ident(c.relname))::text
                 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
                 WHERE n.nspname = COALESCE($1, current_schema()) AND c.relname = $2",
                vec![param(schema), param(Some(name))],
            )
            .await?;
            match kind.first().and_then(|r| get_str(&r[0])).as_deref() {
                Some("v" | "m") => {
                    let def = rows(
                        conn,
                        "SELECT pg_get_viewdef($1::int8::oid, true)::text",
                        vec![Param::Int(get_i64(&kind[0][1]))],
                    )
                    .await?;
                    let kw = if get_str(&kind[0][0]).as_deref() == Some("m") {
                        "CREATE MATERIALIZED VIEW"
                    } else {
                        "CREATE OR REPLACE VIEW"
                    };
                    format!(
                        "{kw} {} AS\n{}",
                        get_str(&kind[0][3]).unwrap_or_default(),
                        def.first().and_then(|r| get_str(&r[0])).unwrap_or_default()
                    )
                }
                Some(_) => {
                    let schema_name = get_str(&kind[0][2]).unwrap_or_default();
                    pg_table_ddl(conn, &schema_name, name).await?
                }
                None => {
                    let defs = rows(
                        conn,
                        "SELECT pg_get_functiondef(p.oid)::text FROM pg_proc p
                         JOIN pg_namespace n ON n.oid = p.pronamespace
                         WHERE n.nspname = COALESCE($1, current_schema()) AND p.proname = $2
                           AND p.prokind IN ('f', 'p')
                           AND ($3::text IS NULL
                                OR pg_get_function_identity_arguments(p.oid) = $3)
                         ORDER BY pg_get_function_identity_arguments(p.oid)",
                        vec![param(schema), param(Some(name)), param(arguments)],
                    )
                    .await?;
                    let defs: Vec<String> = defs.iter().filter_map(|r| get_str(&r[0])).collect();
                    if defs.is_empty() {
                        return Err(not_found());
                    }
                    defs.join(";\n\n")
                }
            }
        }
        Engine::Mssql => {
            let obj = rows(
                conn,
                // OBJECT_DEFINITION is NULL both for encrypted modules and
                // when the login can see the object (any permission on it,
                // e.g. EXECUTE or SELECT) but lacks VIEW DEFINITION, so fetch
                // what is needed to tell the two apart.
                "SELECT RTRIM(o.type), OBJECT_DEFINITION(o.object_id), s.name,
                        OBJECTPROPERTY(o.object_id, 'IsEncrypted'),
                        HAS_PERMS_BY_NAME(QUOTENAME(s.name) + '.' + QUOTENAME(o.name),
                                          'OBJECT', 'VIEW DEFINITION'),
                        USER_NAME(), o.type_desc
                 FROM sys.objects o JOIN sys.schemas s ON s.schema_id = o.schema_id
                 WHERE s.name = COALESCE(@P1, SCHEMA_NAME()) AND o.name = @P2",
                vec![param(schema), param(Some(name))],
            )
            .await?;
            let row = obj.first().ok_or_else(not_found)?;
            let schema_name = get_str(&row[2]).unwrap_or_else(|| "dbo".into());
            match (get_str(&row[0]).as_deref(), get_str(&row[1])) {
                (Some("U"), _) => ms_table_ddl(conn, &schema_name, name).await?,
                (_, Some(def)) => def,
                (_, None) => {
                    return Err(DbError::Invalid(ms_no_definition(
                        &schema_name,
                        name,
                        get_bool(&row[3]),
                        // NULL means "could not check": don't blame permissions.
                        row[4].is_null() || get_bool(&row[4]),
                        get_str(&row[5]).as_deref(),
                        get_str(&row[6]).as_deref(),
                    )));
                }
            }
        }
    };
    Ok(Ddl { ddl })
}

/// Why SQL Server returned no definition for a visible object that isn't a
/// table.
fn ms_no_definition(
    schema: &str,
    name: &str,
    encrypted: bool,
    can_view_definition: bool,
    user: Option<&str>,
    type_desc: Option<&str>,
) -> String {
    let object = format!("{}.{}", ms_ident(schema), ms_ident(name));
    if encrypted {
        format!(
            "{object} was created WITH ENCRYPTION; SQL Server does not return the source \
             of encrypted modules, not even to their owner"
        )
    } else if !can_view_definition {
        let user = user.map_or_else(|| "<user>".to_string(), ms_ident);
        format!(
            "the service's database user {user} lacks VIEW DEFINITION on {object} \
             (EXECUTE or SELECT only lets it see that the object exists). \
             A database owner can grant it with:\n\
             GRANT VIEW DEFINITION ON SCHEMA::{} TO {user};",
            ms_ident(schema)
        )
    } else {
        let kind = type_desc.map_or_else(
            || "object".to_string(),
            |t| t.to_lowercase().replace('_', " "),
        );
        format!("{object} ({kind}) has no T-SQL definition to script")
    }
}

fn ms_ident(s: &str) -> String {
    format!("[{}]", s.replace(']', "]]"))
}

/// The table itself: quoted name, kind, persistence, partition key, and for
/// a partition its parent and bounds, then the table comment (as a literal).
const PG_DDL_TABLE: &str = "SELECT (quote_ident(n.nspname) || '.' || quote_ident(c.relname))::text,
        c.relkind::text, c.relpersistence::text,
        CASE WHEN c.relkind = 'p' THEN pg_get_partkeydef(c.oid) END::text,
        (SELECT quote_ident(pn.nspname) || '.' || quote_ident(p.relname)
         FROM pg_inherits h JOIN pg_class p ON p.oid = h.inhparent
         JOIN pg_namespace pn ON pn.oid = p.relnamespace
         WHERE h.inhrelid = c.oid AND c.relispartition)::text,
        CASE WHEN c.relispartition THEN pg_get_expr(c.relpartbound, c.oid) END::text,
        quote_literal(obj_description(c.oid, 'pg_class'))::text
 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
 WHERE n.nspname = $1 AND c.relname = $2";

/// Columns as DDL needs them: quoted name, type, NOT NULL, default (or
/// generation) expression, attidentity, attgenerated, a collation other than
/// the type's, identity START and INCREMENT, whether it is a serial (its
/// default draws from a sequence the column owns), and the comment (as a
/// literal).
const PG_DDL_COLUMNS: &str = "SELECT quote_ident(a.attname)::text, format_type(a.atttypid, a.atttypmod)::text,
        a.attnotnull, pg_get_expr(d.adbin, d.adrelid)::text, a.attidentity::text, a.attgenerated::text,
        CASE WHEN a.attcollation <> 0 AND a.attcollation <> t.typcollation
             THEN quote_ident(cn.nspname) || '.' || quote_ident(co.collname) END::text,
        s.seqstart::int8, s.seqincrement::int8,
        (a.attidentity = '' AND s.deptype = 'a'
         AND pg_get_expr(d.adbin, d.adrelid) = format('nextval(%L::regclass)', s.seqoid::regclass)),
        quote_literal(col_description(c.oid, a.attnum))::text
 FROM pg_attribute a
 JOIN pg_class c ON c.oid = a.attrelid
 JOIN pg_namespace n ON n.oid = c.relnamespace
 JOIN pg_type t ON t.oid = a.atttypid
 LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
 LEFT JOIN pg_collation co ON co.oid = a.attcollation
 LEFT JOIN pg_namespace cn ON cn.oid = co.collnamespace
 LEFT JOIN LATERAL (
     SELECT sc.oid AS seqoid, dep.deptype, sq.seqstart, sq.seqincrement
     FROM pg_depend dep
     JOIN pg_class sc ON sc.oid = dep.objid AND sc.relkind = 'S'
     JOIN pg_sequence sq ON sq.seqrelid = sc.oid
     WHERE dep.classid = 'pg_class'::regclass AND dep.refclassid = 'pg_class'::regclass
       AND dep.refobjid = c.oid AND dep.refobjsubid = a.attnum AND dep.deptype IN ('a', 'i')
     LIMIT 1) s ON true
 WHERE n.nspname = $1 AND c.relname = $2 AND a.attnum > 0 AND NOT a.attisdropped
 ORDER BY a.attnum";

async fn pg_table_ddl(conn: &mut dyn Conn, schema: &str, table: &str) -> Result<String> {
    let params = || vec![param(Some(schema)), param(Some(table))];
    let t = rows(conn, PG_DDL_TABLE, params()).await?;
    let t = t
        .first()
        .ok_or_else(|| DbError::Invalid(format!("table '{table}' not found")))?;
    let qname = get_str(&t[0]).unwrap_or_default();
    let partition_of = get_str(&t[4]);
    let cols = rows(conn, PG_DDL_COLUMNS, params()).await?;
    let mut lines = Vec::new();
    let mut comments = Vec::new();
    if let Some(c) = get_str(&t[6]) {
        comments.push(format!("COMMENT ON TABLE {qname} IS {c};"));
    }
    for r in &cols {
        let name = get_str(&r[0]).unwrap_or_default();
        if let Some(c) = get_str(&r[10]) {
            comments.push(format!("COMMENT ON COLUMN {qname}.{name} IS {c};"));
        }
        // A partition takes its columns from the parent.
        if partition_of.is_none() {
            lines.push(pg_column(r));
        }
    }
    let cons = rows(
        conn,
        "SELECT quote_ident(con.conname)::text, pg_get_constraintdef(con.oid, true)::text
         FROM pg_constraint con
         JOIN pg_class c ON c.oid = con.conrelid
         JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = $1 AND c.relname = $2 AND con.contype <> 'n'
           AND (con.conislocal OR NOT c.relispartition)
         ORDER BY con.contype = 'f', con.conname",
        params(),
    )
    .await?;
    for r in &cons {
        if let (Some(name), Some(def)) = (get_str(&r[0]), get_str(&r[1])) {
            lines.push(format!("    CONSTRAINT {name} {def}"));
        }
    }
    let unlogged = if get_str(&t[2]).as_deref() == Some("u") {
        "UNLOGGED "
    } else {
        ""
    };
    let mut out = match &partition_of {
        Some(parent) => {
            let body = if lines.is_empty() {
                String::new()
            } else {
                format!(" (\n{}\n)", lines.join(",\n"))
            };
            format!(
                "CREATE {unlogged}TABLE {qname} PARTITION OF {parent}{body}\n{};",
                get_str(&t[5]).unwrap_or_default()
            )
        }
        None => format!(
            "CREATE {unlogged}TABLE {qname} (\n{}\n){};",
            lines.join(",\n"),
            get_str(&t[3])
                .map(|k| format!(" PARTITION BY {k}"))
                .unwrap_or_default()
        ),
    };
    // Indexes of their own: not behind a constraint, not a partition's copy
    // of its parent's index.
    let idx = rows(
        conn,
        "SELECT pg_get_indexdef(i.indexrelid)::text
         FROM pg_index i
         JOIN pg_class c ON c.oid = i.indrelid
         JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = $1 AND c.relname = $2
           AND NOT EXISTS (SELECT 1 FROM pg_constraint con WHERE con.conindid = i.indexrelid)
           AND NOT EXISTS (SELECT 1 FROM pg_inherits h WHERE h.inhrelid = i.indexrelid)
         ORDER BY 1",
        params(),
    )
    .await?;
    for r in &idx {
        if let Some(def) = get_str(&r[0]) {
            // A partitioned table's index prints as `ON ONLY`, which would
            // leave the copy's index invalid; let it cover the partitions.
            out.push_str(&format!("\n{};", def.replacen(" ON ONLY ", " ON ", 1)));
        }
    }
    for c in comments {
        out.push('\n');
        out.push_str(&c);
    }
    Ok(out)
}

/// One column definition from a [`PG_DDL_COLUMNS`] row.
fn pg_column(r: &[Value]) -> String {
    let name = get_str(&r[0]).unwrap_or_default();
    let data_type = get_str(&r[1]).unwrap_or_default();
    // `serial` creates the sequence (named for the new table) and the
    // default; scripting the default would point at the original's sequence.
    let serial = match data_type.as_str() {
        _ if !get_bool(&r[9]) => None,
        "integer" => Some("serial"),
        "bigint" => Some("bigserial"),
        "smallint" => Some("smallserial"),
        _ => None,
    };
    let mut l = format!("    {name} {}", serial.unwrap_or(&data_type));
    if let Some(c) = get_str(&r[6]) {
        l.push_str(&format!(" COLLATE {c}"));
    }
    let expr = get_str(&r[3]).unwrap_or_default();
    match (get_str(&r[4]).as_deref(), get_str(&r[5]).as_deref()) {
        (Some(kind @ ("a" | "d")), _) => {
            l.push_str(if kind == "a" {
                " GENERATED ALWAYS AS IDENTITY"
            } else {
                " GENERATED BY DEFAULT AS IDENTITY"
            });
            let (start, step) = (get_i64(&r[7]), get_i64(&r[8]));
            if (start, step) != (1, 1) {
                l.push_str(&format!(" (START WITH {start} INCREMENT BY {step})"));
            }
        }
        (_, Some("s")) => l.push_str(&format!(" GENERATED ALWAYS AS ({expr}) STORED")),
        (_, Some("v")) => l.push_str(&format!(" GENERATED ALWAYS AS ({expr}) VIRTUAL")),
        _ if serial.is_none() && !expr.is_empty() => l.push_str(&format!(" DEFAULT {expr}")),
        _ => {}
    }
    if get_bool(&r[2]) {
        l.push_str(" NOT NULL");
    }
    l
}

/// SQL Server columns as DDL needs them: name, type, nullable, identity seed,
/// increment and NOT FOR REPLICATION, computed definition and PERSISTED,
/// default constraint name, definition and whether its name is generated,
/// and a collation other than the database's.
const MS_DDL_COLUMNS: &str = concat!(
    "SELECT c.name, ",
    ms_type!(),
    ", c.is_nullable, c.is_identity,
        CAST(idc.seed_value AS nvarchar(40)), CAST(idc.increment_value AS nvarchar(40)),
        idc.is_not_for_replication, cc.definition, cc.is_persisted,
        dc.name, dc.definition, dc.is_system_named,
        CASE WHEN c.collation_name COLLATE DATABASE_DEFAULT
                  <> CAST(DATABASEPROPERTYEX(DB_NAME(), 'Collation') AS sysname) COLLATE DATABASE_DEFAULT
             THEN c.collation_name END
 FROM sys.columns c
 JOIN sys.objects o ON o.object_id = c.object_id
 JOIN sys.schemas s ON s.schema_id = o.schema_id
 JOIN sys.types t ON t.user_type_id = c.user_type_id
 LEFT JOIN sys.identity_columns idc ON idc.object_id = c.object_id AND idc.column_id = c.column_id
 LEFT JOIN sys.computed_columns cc ON cc.object_id = c.object_id AND cc.column_id = c.column_id
 LEFT JOIN sys.default_constraints dc ON dc.object_id = c.default_object_id
 WHERE s.name = @P1 AND o.name = @P2
 ORDER BY c.column_id"
);

/// `CONSTRAINT [name] `, or nothing for a name SQL Server generated (so the
/// script can run next to the original).
fn ms_constraint(name: &str, system_named: bool) -> String {
    if system_named {
        String::new()
    } else {
        format!("CONSTRAINT {} ", ms_ident(name))
    }
}

/// Key columns with their sort order.
fn ms_keys(i: &IndexInfo) -> String {
    i.columns
        .iter()
        .enumerate()
        .map(|(n, c)| {
            if i.descending.get(n) == Some(&true) {
                format!("{} DESC", ms_ident(c))
            } else {
                ms_ident(c)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn ms_list(names: &[String]) -> String {
    names
        .iter()
        .map(|c| ms_ident(c))
        .collect::<Vec<_>>()
        .join(", ")
}

async fn ms_table_ddl(conn: &mut dyn Conn, schema: &str, table: &str) -> Result<String> {
    let params = || vec![param(Some(schema)), param(Some(table))];
    let qname = format!("{}.{}", ms_ident(schema), ms_ident(table));
    let mut lines: Vec<String> = rows(conn, MS_DDL_COLUMNS, params())
        .await?
        .iter()
        .map(|r| {
            let name = ms_ident(&get_str(&r[0]).unwrap_or_default());
            let nullable = get_bool(&r[2]);
            if let Some(def) = get_str(&r[7]) {
                let persisted = get_bool(&r[8]);
                return format!(
                    "    {name} AS {def}{}{}",
                    if persisted { " PERSISTED" } else { "" },
                    if persisted && !nullable {
                        " NOT NULL"
                    } else {
                        ""
                    }
                );
            }
            let mut l = format!("    {name} {}", get_str(&r[1]).unwrap_or_default());
            if let Some(c) = get_str(&r[12]) {
                l.push_str(&format!(" COLLATE {c}"));
            }
            if get_bool(&r[3]) {
                l.push_str(&format!(
                    " IDENTITY({}, {})",
                    get_str(&r[4]).unwrap_or_else(|| "1".into()),
                    get_str(&r[5]).unwrap_or_else(|| "1".into())
                ));
                if get_bool(&r[6]) {
                    l.push_str(" NOT FOR REPLICATION");
                }
            }
            l.push_str(if nullable { " NULL" } else { " NOT NULL" });
            if let Some(d) = get_str(&r[10]) {
                let df = get_str(&r[9]).unwrap_or_default();
                l.push_str(&format!(
                    " {}DEFAULT {d}",
                    ms_constraint(&df, get_bool(&r[11]))
                ));
            }
            l
        })
        .collect();
    let idx_rows = rows(conn, MS_INDEXES, params()).await?;
    let system_named: Vec<String> = rows(
        conn,
        "SELECT k.name FROM sys.key_constraints k
         WHERE k.parent_object_id = OBJECT_ID(QUOTENAME(@P1) + '.' + QUOTENAME(@P2))
           AND k.is_system_named = 1",
        params(),
    )
    .await?
    .iter()
    .filter_map(|r| get_str(&r[0]))
    .collect();
    let idx: Vec<(IndexInfo, Option<String>)> = idx_rows
        .iter()
        .filter_map(|r| Some((index_info(r)?, get_str(&r[8]))))
        .collect();
    let clustering = |i: &IndexInfo| match i.clustered {
        Some(true) => "CLUSTERED ",
        Some(false) => "NONCLUSTERED ",
        None => "",
    };
    for (i, _) in idx.iter().filter(|(i, _)| i.constraint) {
        lines.push(format!(
            "    {}{} {}({})",
            ms_constraint(&i.name, system_named.contains(&i.name)),
            if i.primary { "PRIMARY KEY" } else { "UNIQUE" },
            clustering(i),
            ms_keys(i)
        ));
    }
    let checks = rows(
        conn,
        "SELECT name, definition, is_system_named, is_not_for_replication
         FROM sys.check_constraints
         WHERE parent_object_id = OBJECT_ID(QUOTENAME(@P1) + '.' + QUOTENAME(@P2))
         ORDER BY name",
        params(),
    )
    .await?;
    for r in &checks {
        lines.push(format!(
            "    {}CHECK {}{}",
            ms_constraint(&get_str(&r[0]).unwrap_or_default(), get_bool(&r[2])),
            if get_bool(&r[3]) {
                "NOT FOR REPLICATION "
            } else {
                ""
            },
            get_str(&r[1]).unwrap_or_default()
        ));
    }
    let action = |v: &Value, on: &str| match get_str(v).as_deref() {
        Some("CASCADE") => format!(" ON {on} CASCADE"),
        Some("SET_NULL") => format!(" ON {on} SET NULL"),
        Some("SET_DEFAULT") => format!(" ON {on} SET DEFAULT"),
        _ => String::new(),
    };
    for r in rows(conn, MS_FOREIGN_KEYS, params()).await? {
        lines.push(format!(
            "    {}FOREIGN KEY ({}) REFERENCES {}.{} ({}){}{}{}",
            ms_constraint(&get_str(&r[0]).unwrap_or_default(), get_bool(&r[7])),
            ms_list(&split_list(&r[1])),
            ms_ident(&get_str(&r[2]).unwrap_or_else(|| "dbo".into())),
            ms_ident(&get_str(&r[3]).unwrap_or_default()),
            ms_list(&split_list(&r[4])),
            action(&r[5], "DELETE"),
            action(&r[6], "UPDATE"),
            if get_bool(&r[8]) {
                " NOT FOR REPLICATION"
            } else {
                ""
            }
        ));
    }
    let mut out = format!("CREATE TABLE {qname} (\n{}\n);", lines.join(",\n"));
    for (i, kind) in idx.iter().filter(|(i, _)| !i.constraint) {
        if i.clustered.is_none() {
            out.push_str(&format!(
                "\n-- {} ({}) is not scripted",
                ms_ident(&i.name),
                kind.as_deref().unwrap_or("index").to_lowercase()
            ));
            continue;
        }
        out.push_str(&format!(
            "\nCREATE {}{}INDEX {} ON {qname} ({})",
            if i.unique { "UNIQUE " } else { "" },
            if i.clustered == Some(true) {
                "CLUSTERED "
            } else {
                ""
            },
            ms_ident(&i.name),
            ms_keys(i)
        ));
        if !i.include.is_empty() {
            out.push_str(&format!(" INCLUDE ({})", ms_list(&i.include)));
        }
        if let Some(f) = &i.filter {
            out.push_str(&format!(" WHERE {f}"));
        }
        out.push(';');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_quoting() {
        assert_eq!(ms_ident("a]b"), "[a]]b]");
    }

    #[test]
    fn mssql_missing_definition_says_why() {
        let enc = ms_no_definition("dbo", "p", true, false, Some("app"), None);
        assert!(enc.contains("WITH ENCRYPTION"), "{enc}");
        assert!(!enc.contains("GRANT"), "{enc}");

        let perm = ms_no_definition("sales", "p", false, false, Some("app]x"), None);
        assert!(
            perm.contains("lacks VIEW DEFINITION on [sales].[p]"),
            "{perm}"
        );
        assert!(
            perm.ends_with("GRANT VIEW DEFINITION ON SCHEMA::[sales] TO [app]]x];"),
            "{perm}"
        );

        let clr = ms_no_definition("dbo", "p", false, true, None, Some("CLR_STORED_PROCEDURE"));
        assert!(clr.contains("(clr stored procedure)"), "{clr}");
    }
}
