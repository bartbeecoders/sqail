//! Catalog queries per engine. All user-supplied names travel as bind
//! parameters; the only identifier ever spliced into SQL is an SQLite schema
//! name, and only after it matched an attached database.

use serde_json::Value;
use sqail_proto::{
    ColumnInfo, Ddl, Engine, ForeignKeyInfo, IndexInfo, NamedItem, Param, RoutineInfo, RoutineKind,
    TableInfo, TableKind,
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
        Engine::Postgres => (
            "SELECT n.nspname::text, c.relname::text, c.relkind::text
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
            "SELECT s.name, o.name, RTRIM(o.type)
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
                    "SELECT '{}', name, type FROM {}.sqlite_schema
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
            Some(TableInfo {
                schema: get_str(&r[0]),
                name: get_str(&r[1])?,
                kind: match get_str(&r[2]).as_deref() {
                    Some("v" | "V" | "view") => TableKind::View,
                    Some("m") => TableKind::MaterializedView,
                    _ => TableKind::Table,
                },
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

pub async fn columns(
    conn: &mut dyn Conn,
    schema: Option<&str>,
    table: &str,
) -> Result<Vec<ColumnInfo>> {
    let (sql, params) = match conn.engine() {
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
            "SELECT c.name, c.column_id,
                    CASE WHEN t.name IN ('varchar', 'char', 'varbinary', 'binary')
                              THEN t.name + '(' + IIF(c.max_length = -1, 'max', CAST(c.max_length AS varchar(10))) + ')'
                         WHEN t.name IN ('nvarchar', 'nchar')
                              THEN t.name + '(' + IIF(c.max_length = -1, 'max', CAST(c.max_length / 2 AS varchar(10))) + ')'
                         WHEN t.name IN ('decimal', 'numeric')
                              THEN t.name + '(' + CAST(c.precision AS varchar(3)) + ',' + CAST(c.scale AS varchar(3)) + ')'
                         WHEN t.name IN ('datetime2', 'time', 'datetimeoffset')
                              THEN t.name + '(' + CAST(c.scale AS varchar(3)) + ')'
                         ELSE t.name END,
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
             ORDER BY c.column_id",
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
        Engine::Postgres => (
            "SELECT ic.relname::text,
                    (SELECT string_agg(pg_get_indexdef(i.indexrelid, k + 1, true), chr(31) ORDER BY k)
                     FROM generate_series(0, i.indnkeyatts - 1) k)::text,
                    i.indisunique, i.indisprimary
             FROM pg_index i
             JOIN pg_class ic ON ic.oid = i.indexrelid
             JOIN pg_class c ON c.oid = i.indrelid
             JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = COALESCE($1, current_schema()) AND c.relname = $2
             ORDER BY 1",
            vec![param(schema), param(Some(table))],
        ),
        Engine::Mssql => (
            "SELECT i.name,
                    STRING_AGG(c.name, CHAR(31)) WITHIN GROUP (ORDER BY ic.key_ordinal),
                    i.is_unique, i.is_primary_key
             FROM sys.indexes i
             JOIN sys.index_columns ic ON ic.object_id = i.object_id AND ic.index_id = i.index_id
             JOIN sys.columns c ON c.object_id = ic.object_id AND c.column_id = ic.column_id
             JOIN sys.objects o ON o.object_id = i.object_id
             JOIN sys.schemas s ON s.schema_id = o.schema_id
             WHERE s.name = COALESCE(@P1, SCHEMA_NAME()) AND o.name = @P2
               AND i.name IS NOT NULL AND ic.is_included_column = 0
             GROUP BY i.name, i.is_unique, i.is_primary_key
             ORDER BY i.name",
            vec![param(schema), param(Some(table))],
        ),
        Engine::Sqlite => {
            sqlite_schema(conn, schema).await?;
            (
                "SELECT il.name,
                        (SELECT group_concat(name, char(31))
                         FROM (SELECT name FROM pragma_index_info(il.name, ?2) ORDER BY seqno)),
                        il.\"unique\", il.origin = 'pk'
                 FROM pragma_index_list(?1, ?2) il ORDER BY il.name",
                vec![param(Some(table)), param(Some(schema.unwrap_or("main")))],
            )
        }
    };
    Ok(rows(conn, sql, params)
        .await?
        .iter()
        .filter_map(|r| {
            Some(IndexInfo {
                name: get_str(&r[0])?,
                columns: split_list(&r[1]),
                unique: get_bool(&r[2]),
                primary: get_bool(&r[3]),
            })
        })
        .collect())
}

// --------------------------------------------------------- foreign keys --

pub async fn foreign_keys(
    conn: &mut dyn Conn,
    schema: Option<&str>,
    table: &str,
) -> Result<Vec<ForeignKeyInfo>> {
    let (sql, params) = match conn.engine() {
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
        Engine::Mssql => (
            "SELECT fk.name,
                    STRING_AGG(pc.name, CHAR(31)) WITHIN GROUP (ORDER BY fkc.constraint_column_id),
                    rs.name, rt.name,
                    STRING_AGG(rc.name, CHAR(31)) WITHIN GROUP (ORDER BY fkc.constraint_column_id)
             FROM sys.foreign_keys fk
             JOIN sys.foreign_key_columns fkc ON fkc.constraint_object_id = fk.object_id
             JOIN sys.columns pc ON pc.object_id = fkc.parent_object_id AND pc.column_id = fkc.parent_column_id
             JOIN sys.columns rc ON rc.object_id = fkc.referenced_object_id AND rc.column_id = fkc.referenced_column_id
             JOIN sys.objects rt ON rt.object_id = fk.referenced_object_id
             JOIN sys.schemas rs ON rs.schema_id = rt.schema_id
             JOIN sys.objects p ON p.object_id = fk.parent_object_id
             JOIN sys.schemas ps ON ps.schema_id = p.schema_id
             WHERE ps.name = COALESCE(@P1, SCHEMA_NAME()) AND p.name = @P2
             GROUP BY fk.name, rs.name, rt.name
             ORDER BY fk.name",
            vec![param(schema), param(Some(table))],
        ),
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

// ------------------------------------------------------------- routines --

pub async fn routines(conn: &mut dyn Conn, schema: Option<&str>) -> Result<Vec<RoutineInfo>> {
    let (sql, params) = match conn.engine() {
        Engine::Postgres => (
            "SELECT n.nspname::text, p.proname::text, p.prokind::text
             FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
             WHERE ($1::text IS NULL OR n.nspname = $1)
               AND n.nspname NOT IN ('pg_catalog', 'information_schema')
               AND p.prokind IN ('f', 'p')
             ORDER BY 1, 2",
            vec![param(schema)],
        ),
        Engine::Mssql => (
            "SELECT s.name, o.name, RTRIM(o.type)
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
            Some(RoutineInfo {
                schema: get_str(&r[0]),
                name: get_str(&r[1])?,
                kind: match get_str(&r[2]).as_deref() {
                    Some("p" | "P") => RoutineKind::Procedure,
                    _ => RoutineKind::Function,
                },
            })
        })
        .collect())
}

// ------------------------------------------------------------------ ddl --

pub async fn ddl(conn: &mut dyn Conn, schema: Option<&str>, name: &str) -> Result<Ddl> {
    let not_found = || DbError::Invalid(format!("object '{name}' not found"));
    let ddl = match conn.engine() {
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
            let kind = rows(
                conn,
                "SELECT c.relkind::text, c.oid::int8 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
                 WHERE n.nspname = COALESCE($1, current_schema()) AND c.relname = $2",
                vec![param(schema), param(Some(name))],
            )
            .await?;
            let schema_name = schema.unwrap_or("public");
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
                        "{kw} {}.{} AS\n{}",
                        pg_ident(schema_name),
                        pg_ident(name),
                        def.first().and_then(|r| get_str(&r[0])).unwrap_or_default()
                    )
                }
                Some(_) => pg_table_ddl(conn, schema_name, name).await?,
                None => {
                    let defs = rows(
                        conn,
                        "SELECT pg_get_functiondef(p.oid)::text FROM pg_proc p
                         JOIN pg_namespace n ON n.oid = p.pronamespace
                         WHERE n.nspname = COALESCE($1, current_schema()) AND p.proname = $2
                           AND p.prokind IN ('f', 'p')",
                        vec![param(schema), param(Some(name))],
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
                "SELECT RTRIM(o.type), OBJECT_DEFINITION(o.object_id), s.name
                 FROM sys.objects o JOIN sys.schemas s ON s.schema_id = o.schema_id
                 WHERE s.name = COALESCE(@P1, SCHEMA_NAME()) AND o.name = @P2",
                vec![param(schema), param(Some(name))],
            )
            .await?;
            let row = obj.first().ok_or_else(not_found)?;
            let schema_name = get_str(&row[2]).unwrap_or_else(|| "dbo".into());
            match get_str(&row[0]).as_deref() {
                Some("U") => ms_table_ddl(conn, &schema_name, name).await?,
                _ => get_str(&row[1]).ok_or_else(|| {
                    DbError::Invalid(format!("no definition available for '{name}' (encrypted?)"))
                })?,
            }
        }
    };
    Ok(Ddl { ddl })
}

fn pg_ident(s: &str) -> String {
    let plain = s
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if plain {
        s.to_string()
    } else {
        format!("\"{}\"", s.replace('"', "\"\""))
    }
}

fn ms_ident(s: &str) -> String {
    format!("[{}]", s.replace(']', "]]"))
}

async fn pg_table_ddl(conn: &mut dyn Conn, schema: &str, table: &str) -> Result<String> {
    let cols = columns(conn, Some(schema), table).await?;
    let mut lines: Vec<String> = cols
        .iter()
        .map(|c| {
            let mut l = format!("    {} {}", pg_ident(&c.name), c.data_type);
            if let Some(d) = &c.default {
                l.push_str(&format!(" DEFAULT {d}"));
            }
            if !c.nullable {
                l.push_str(" NOT NULL");
            }
            l
        })
        .collect();
    let cons = rows(
        conn,
        "SELECT con.conname::text, pg_get_constraintdef(con.oid, true)::text
         FROM pg_constraint con
         JOIN pg_class c ON c.oid = con.conrelid
         JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = $1 AND c.relname = $2
         ORDER BY con.contype = 'f', con.conname",
        vec![param(Some(schema)), param(Some(table))],
    )
    .await?;
    for r in &cons {
        if let (Some(name), Some(def)) = (get_str(&r[0]), get_str(&r[1])) {
            lines.push(format!("    CONSTRAINT {} {def}", pg_ident(&name)));
        }
    }
    let mut out = format!(
        "CREATE TABLE {}.{} (\n{}\n);",
        pg_ident(schema),
        pg_ident(table),
        lines.join(",\n")
    );
    let idx = rows(
        conn,
        "SELECT pg_get_indexdef(i.indexrelid)::text
         FROM pg_index i
         JOIN pg_class c ON c.oid = i.indrelid
         JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = $1 AND c.relname = $2
           AND NOT EXISTS (SELECT 1 FROM pg_constraint con WHERE con.conindid = i.indexrelid)
         ORDER BY 1",
        vec![param(Some(schema)), param(Some(table))],
    )
    .await?;
    for r in &idx {
        if let Some(def) = get_str(&r[0]) {
            out.push_str(&format!("\n{def};"));
        }
    }
    Ok(out)
}

async fn ms_table_ddl(conn: &mut dyn Conn, schema: &str, table: &str) -> Result<String> {
    let cols = columns(conn, Some(schema), table).await?;
    let mut lines: Vec<String> = cols
        .iter()
        .map(|c| {
            let mut l = format!("    {} {}", ms_ident(&c.name), c.data_type);
            l.push_str(if c.nullable { " NULL" } else { " NOT NULL" });
            if let Some(d) = &c.default {
                l.push_str(&format!(" DEFAULT {d}"));
            }
            l
        })
        .collect();
    let idx = indexes(conn, Some(schema), table).await?;
    if let Some(pk) = idx.iter().find(|i| i.primary) {
        lines.push(format!(
            "    CONSTRAINT {} PRIMARY KEY ({})",
            ms_ident(&pk.name),
            pk.columns
                .iter()
                .map(|c| ms_ident(c))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for fk in foreign_keys(conn, Some(schema), table).await? {
        lines.push(format!(
            "    CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {}.{} ({})",
            ms_ident(&fk.name),
            fk.columns
                .iter()
                .map(|c| ms_ident(c))
                .collect::<Vec<_>>()
                .join(", "),
            ms_ident(fk.ref_schema.as_deref().unwrap_or("dbo")),
            ms_ident(&fk.ref_table),
            fk.ref_columns
                .iter()
                .map(|c| ms_ident(c))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let mut out = format!(
        "CREATE TABLE {}.{} (\n{}\n);",
        ms_ident(schema),
        ms_ident(table),
        lines.join(",\n")
    );
    for i in idx.iter().filter(|i| !i.primary) {
        out.push_str(&format!(
            "\nCREATE {}INDEX {} ON {}.{} ({});",
            if i.unique { "UNIQUE " } else { "" },
            ms_ident(&i.name),
            ms_ident(schema),
            ms_ident(table),
            i.columns
                .iter()
                .map(|c| ms_ident(c))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_quoting() {
        assert_eq!(pg_ident("orders"), "orders");
        assert_eq!(pg_ident("Orders"), "\"Orders\"");
        assert_eq!(pg_ident("a\"b"), "\"a\"\"b\"");
        assert_eq!(ms_ident("a]b"), "[a]]b]");
    }
}
