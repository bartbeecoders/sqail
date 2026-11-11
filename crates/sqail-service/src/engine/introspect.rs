//! Catalog queries per engine. All user-supplied names travel as bind
//! parameters; the only identifier ever spliced into SQL is an SQLite schema
//! name, and only after it matched an attached database.

use serde_json::Value;
use sqail_proto::{
    CheckInfo, ColumnInfo, Ddl, Engine, ForeignKeyInfo, GeneratedInfo, IdentityInfo, IndexInfo,
    NamedItem, Param, ReferencingKey, ReferentialAction, RoutineInfo, RoutineKind, TableDetails,
    TableGrant, TableInfo, TableKind, TablePrivileges,
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
    // SQL Server picks a catalog read as the victim when it deadlocks with
    // DDL running elsewhere (Msg 1205); a read is safe to run again.
    let mut tries = 0;
    loop {
        match fetch_rows(conn, sql, params.clone()).await {
            Err(DbError::Internal(m)) if m.contains("Msg 1205,") && tries < 3 => {
                tries += 1;
                tokio::time::sleep(std::time::Duration::from_millis(20 * tries)).await;
            }
            r => return r,
        }
    }
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

/// SQL Server: the identity and computed-column rows of column `c` (`idc`,
/// `cc`) and its default constraint (`dc`).
macro_rules! ms_column_joins {
    () => {
        "LEFT JOIN sys.identity_columns idc ON idc.object_id = c.object_id AND idc.column_id = c.column_id
         LEFT JOIN sys.computed_columns cc ON cc.object_id = c.object_id AND cc.column_id = c.column_id
         LEFT JOIN sys.default_constraints dc ON dc.object_id = c.default_object_id"
    };
}

/// PostgreSQL: the sequence that column `a` of table `c` owns, as `s`
/// (`seqoid`, `deptype` 'i' for identity or 'a' for serial, `seqstart`,
/// `seqincrement`).
macro_rules! pg_owned_sequence {
    () => {
        "LEFT JOIN LATERAL (
             SELECT sc.oid AS seqoid, dep.deptype, sq.seqstart, sq.seqincrement
             FROM pg_depend dep
             JOIN pg_class sc ON sc.oid = dep.objid AND sc.relkind = 'S'
             JOIN pg_sequence sq ON sq.seqrelid = sc.oid
             WHERE dep.classid = 'pg_class'::regclass AND dep.refclassid = 'pg_class'::regclass
               AND dep.refobjid = c.oid AND dep.refobjsubid = a.attnum AND dep.deptype IN ('a', 'i')
             LIMIT 1) s ON true"
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
    // Columns 0-5 as in `ColumnInfo`; then the identity kind ('a' always,
    // 'd' by default, '' none), seed and increment, the generation kind
    // ('s' stored, 'v' virtual, '' none) whose expression is column 4, and
    // the comment.
    let (sql, params) = match conn.engine() {
        Engine::Unknown => return Err(DbError::unknown_engine()),
        Engine::Postgres => (
            concat!(
                "SELECT a.attname::text, a.attnum::int8, format_type(a.atttypid, a.atttypmod)::text,
                    (NOT a.attnotnull)::bool, pg_get_expr(d.adbin, d.adrelid)::text,
                    COALESCE(a.attnum = ANY(i.indkey), false)::bool,
                    a.attidentity::text, s.seqstart::text, s.seqincrement::text, a.attgenerated::text,
                    col_description(c.oid, a.attnum)::text
             FROM pg_attribute a
             JOIN pg_class c ON c.oid = a.attrelid
             JOIN pg_namespace n ON n.oid = c.relnamespace
             LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
             LEFT JOIN pg_index i ON i.indrelid = c.oid AND i.indisprimary ",
                pg_owned_sequence!(),
                "
             WHERE n.nspname = COALESCE($1, current_schema()) AND c.relname = $2
               AND a.attnum > 0 AND NOT a.attisdropped
             ORDER BY a.attnum"
            ),
            vec![param(schema), param(Some(table))],
        ),
        Engine::Mssql => (
            concat!(
                "SELECT c.name, c.column_id, ",
                ms_type!(),
                ",
                    c.is_nullable, COALESCE(cc.definition, dc.definition),
                    CAST(CASE WHEN pk.column_id IS NULL THEN 0 ELSE 1 END AS bit),
                    IIF(c.is_identity = 1, 'a', ''),
                    CAST(idc.seed_value AS nvarchar(40)), CAST(idc.increment_value AS nvarchar(40)),
                    CASE WHEN cc.is_persisted = 1 THEN 's' WHEN c.is_computed = 1 THEN 'v' ELSE '' END,
                    CAST(ep.value AS nvarchar(max))
             FROM sys.columns c
             JOIN sys.objects o ON o.object_id = c.object_id
             JOIN sys.schemas s ON s.schema_id = o.schema_id
             JOIN sys.types t ON t.user_type_id = c.user_type_id ",
                ms_column_joins!(),
                "
             LEFT JOIN sys.extended_properties ep ON ep.class = 1 AND ep.major_id = c.object_id
                    AND ep.minor_id = c.column_id AND ep.name = 'MS_Description'
             LEFT JOIN (SELECT ic.object_id, ic.column_id FROM sys.indexes i
                        JOIN sys.index_columns ic ON ic.object_id = i.object_id AND ic.index_id = i.index_id
                        WHERE i.is_primary_key = 1) pk
                    ON pk.object_id = c.object_id AND pk.column_id = c.column_id
             WHERE s.name = COALESCE(@P1, SCHEMA_NAME()) AND o.name = @P2
             ORDER BY c.column_id"
            ),
            vec![param(schema), param(Some(table))],
        ),
        Engine::Sqlite => return sqlite_columns(conn, schema, table).await,
    };
    Ok(rows(conn, sql, params)
        .await?
        .iter()
        .filter_map(|r| {
            let expr = get_str(&r[4]);
            let generated = match get_str(&r[9]).as_deref() {
                Some(g @ ("s" | "v")) => Some(GeneratedInfo {
                    expression: expr.clone().unwrap_or_default(),
                    stored: g == "s",
                }),
                _ => None,
            };
            let identity = match get_str(&r[6]).as_deref() {
                Some(k @ ("a" | "d")) => Some(IdentityInfo {
                    always: k == "a",
                    seed: get_str(&r[7]),
                    increment: get_str(&r[8]),
                }),
                _ => None,
            };
            Some(ColumnInfo {
                name: get_str(&r[0])?,
                ordinal: get_i64(&r[1]),
                data_type: get_str(&r[2]).unwrap_or_default(),
                nullable: get_bool(&r[3]),
                default: if generated.is_some() { None } else { expr },
                primary_key: get_bool(&r[5]),
                identity,
                generated,
                comment: get_str(&r[10]),
            })
        })
        .collect())
}

/// SQLite columns, generated ones included (`pragma_table_info` leaves
/// them out). Their expressions are only in the `CREATE TABLE` text.
async fn sqlite_columns(
    conn: &mut dyn Conn,
    schema: Option<&str>,
    table: &str,
) -> Result<Vec<ColumnInfo>> {
    let db = sqlite_schema(conn, schema).await?;
    let params = || vec![param(Some(table)), param(Some(&db.name))];
    // A lone `INTEGER PRIMARY KEY` is the rowid, numbered by SQLite, unless
    // the table is WITHOUT ROWID (its key then has an index of its own).
    let raw = rows(
        conn,
        "SELECT name, cid + 1, type, \"notnull\" = 0 AND pk = 0, dflt_value, pk > 0, hidden,
                pk = 1 AND upper(type) = 'INTEGER'
                  AND (SELECT count(*) FROM pragma_table_info(?1, ?2) WHERE pk > 0) = 1
                  AND NOT EXISTS (SELECT 1 FROM pragma_index_list(?1, ?2) WHERE origin = 'pk')
         FROM pragma_table_xinfo(?1, ?2) WHERE hidden IN (0, 2, 3) ORDER BY cid",
        params(),
    )
    .await?;
    let create_sql = if raw.iter().any(|r| get_i64(&r[6]) != 0) {
        let sql = format!(
            "SELECT sql FROM {}.sqlite_schema WHERE type = 'table' AND name = ?1",
            db.quoted
        );
        rows(conn, &sql, vec![param(Some(table))])
            .await?
            .first()
            .and_then(|r| get_str(&r[0]))
    } else {
        None
    };
    Ok(raw
        .iter()
        .filter_map(|r| {
            let name = get_str(&r[0])?;
            let hidden = get_i64(&r[6]);
            let generated = (hidden == 2 || hidden == 3).then(|| GeneratedInfo {
                expression: create_sql
                    .as_deref()
                    .and_then(|s| sqlite_generated_expr(s, &name))
                    .unwrap_or_default(),
                stored: hidden == 3,
            });
            Some(ColumnInfo {
                ordinal: get_i64(&r[1]),
                data_type: get_str(&r[2]).unwrap_or_default(),
                nullable: get_bool(&r[3]),
                default: get_str(&r[4]),
                primary_key: get_bool(&r[5]),
                identity: get_bool(&r[7]).then(IdentityInfo::default),
                generated,
                name,
                comment: None,
            })
        })
        .collect())
}

/// The top-level pieces of `text`, split at commas outside quotes and
/// parentheses, up to an unmatched `)`.
fn sqlite_split(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start, mut quote) = (0i32, 0, None::<char>);
    for (i, ch) in text.char_indices() {
        match (quote, ch) {
            (Some(q), c) if c == q || (q == '[' && c == ']') => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"' | '`' | '[') => quote = Some(ch),
            (None, '(') => depth += 1,
            (None, ')') if depth == 0 => {
                out.push(&text[start..i]);
                return out;
            }
            (None, ')') => depth -= 1,
            (None, ',') if depth == 0 => {
                out.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&text[start..]);
    out
}

/// The column and constraint definitions of an SQLite `CREATE TABLE`.
fn sqlite_defs(create: &str) -> Vec<&str> {
    match create.find('(') {
        Some(open) => sqlite_split(&create[open + 1..])
            .into_iter()
            .map(str::trim)
            .collect(),
        None => Vec::new(),
    }
}

/// The names and words of `def` outside parentheses, with their offsets;
/// quoted names unquoted. An opening parenthesis is the word `(`.
fn sqlite_words(def: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut chars = def.char_indices().peekable();
    let mut depth = 0;
    while let Some((i, ch)) = chars.next() {
        match ch {
            '\'' | '"' | '`' | '[' => {
                let close = if ch == '[' { ']' } else { ch };
                let mut name = String::new();
                for (_, c) in chars.by_ref() {
                    if c == close {
                        break;
                    }
                    name.push(c);
                }
                if depth == 0 && ch != '\'' {
                    out.push((i, name));
                }
            }
            '(' => {
                if depth == 0 {
                    out.push((i, "(".into()));
                }
                depth += 1;
            }
            ')' => depth -= 1,
            c if c.is_alphanumeric() || c == '_' => {
                let mut end = i + c.len_utf8();
                while let Some(&(j, c)) = chars.peek() {
                    if !(c.is_alphanumeric() || c == '_' || c == '$') {
                        break;
                    }
                    end = j + c.len_utf8();
                    chars.next();
                }
                if depth == 0 {
                    out.push((i, def[i..end].to_string()));
                }
            }
            _ => {}
        }
    }
    out
}

/// The inside of the parentheses that open at `open` in `def`.
fn sqlite_parens(def: &str, open: usize) -> Option<String> {
    let inner = &def[open + 1..];
    let end = sqlite_split(inner)
        .iter()
        .map(|p| p.len() + 1)
        .sum::<usize>()
        - 1;
    inner[end..]
        .starts_with(')')
        .then(|| inner[..end].trim().to_string())
}

/// The expression of generated column `column` in an SQLite `CREATE TABLE`
/// statement: what is inside the parentheses after `AS` in its definition.
fn sqlite_generated_expr(create: &str, column: &str) -> Option<String> {
    sqlite_defs(create).into_iter().find_map(|def| {
        let words = sqlite_words(def);
        if !words.first()?.1.eq_ignore_ascii_case(column) {
            return None;
        }
        let at = words
            .windows(2)
            .position(|w| w[0].1.eq_ignore_ascii_case("AS") && w[1].1 == "(")?;
        sqlite_parens(def, words[at + 1].0)
    })
}

/// The CHECK constraints in an SQLite `CREATE TABLE` statement, on columns
/// and on the table, with their names where given.
fn sqlite_checks(create: &str) -> Vec<CheckInfo> {
    let mut out = Vec::new();
    for def in sqlite_defs(create) {
        let words = sqlite_words(def);
        for (k, w) in words.windows(2).enumerate() {
            if !(w[0].1.eq_ignore_ascii_case("CHECK") && w[1].1 == "(") {
                continue;
            }
            let name = match k.checked_sub(2).map(|n| &words[n].1) {
                Some(c) if c.eq_ignore_ascii_case("CONSTRAINT") => words[k - 1].1.clone(),
                _ => String::new(),
            };
            if let Some(expression) = sqlite_parens(def, w[1].0) {
                out.push(CheckInfo { name, expression });
            }
        }
    }
    out
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

/// PostgreSQL foreign keys: the holding table's schema and name, then the
/// [`fk_info`] columns. Followed by a condition on `n`/`c` (the holding
/// table) or `rn`/`rc` (the referenced one).
macro_rules! pg_foreign_keys {
    () => {
        "SELECT n.nspname::text, c.relname::text, con.conname::text,
                (SELECT string_agg(a.attname, chr(31) ORDER BY k.ord)
                 FROM unnest(con.conkey) WITH ORDINALITY k(attnum, ord)
                 JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.attnum)::text,
                rn.nspname::text, rc.relname::text,
                (SELECT string_agg(a.attname, chr(31) ORDER BY k.ord)
                 FROM unnest(con.confkey) WITH ORDINALITY k(attnum, ord)
                 JOIN pg_attribute a ON a.attrelid = con.confrelid AND a.attnum = k.attnum)::text,
                con.confdeltype::text, con.confupdtype::text,
                NULLIF(concat_ws(' ',
                    CASE WHEN con.confmatchtype = 'f' THEN 'MATCH FULL' END,
                    CASE WHEN con.condeferrable THEN 'DEFERRABLE' END,
                    CASE WHEN con.condeferred THEN 'INITIALLY DEFERRED' END,
                    CASE WHEN NOT con.convalidated THEN 'NOT VALID' END), '')
         FROM pg_constraint con
         JOIN pg_class c ON c.oid = con.conrelid
         JOIN pg_namespace n ON n.oid = c.relnamespace
         JOIN pg_class rc ON rc.oid = con.confrelid
         JOIN pg_namespace rn ON rn.oid = rc.relnamespace
         WHERE con.contype = 'f' "
    };
}

/// SQL Server foreign keys, columns as in [`pg_foreign_keys`], then (for
/// scripting) whether the name is generated. `$where` is a condition on
/// `ps`/`p` (the holding table) or `rs`/`rt` (the referenced one).
macro_rules! ms_foreign_keys {
    ($where:literal) => {
        concat!(
            "SELECT ps.name, p.name, fk.name,
                STRING_AGG(pc.name, CHAR(31)) WITHIN GROUP (ORDER BY fkc.constraint_column_id),
                rs.name, rt.name,
                STRING_AGG(rc.name, CHAR(31)) WITHIN GROUP (ORDER BY fkc.constraint_column_id),
                fk.delete_referential_action_desc, fk.update_referential_action_desc,
                IIF(fk.is_not_for_replication = 1, 'NOT FOR REPLICATION', NULL),
                fk.is_system_named
             FROM sys.foreign_keys fk
             JOIN sys.foreign_key_columns fkc ON fkc.constraint_object_id = fk.object_id
             JOIN sys.columns pc ON pc.object_id = fkc.parent_object_id AND pc.column_id = fkc.parent_column_id
             JOIN sys.columns rc ON rc.object_id = fkc.referenced_object_id AND rc.column_id = fkc.referenced_column_id
             JOIN sys.objects rt ON rt.object_id = fk.referenced_object_id
             JOIN sys.schemas rs ON rs.schema_id = rt.schema_id
             JOIN sys.objects p ON p.object_id = fk.parent_object_id
             JOIN sys.schemas ps ON ps.schema_id = p.schema_id
             WHERE ",
            $where,
            "
             GROUP BY ps.name, p.name, fk.name, rs.name, rt.name, fk.delete_referential_action_desc,
                      fk.update_referential_action_desc, fk.is_system_named, fk.is_not_for_replication
             ORDER BY ps.name, p.name, fk.name"
        )
    };
}

/// The foreign keys a table holds.
const MS_FOREIGN_KEYS: &str =
    ms_foreign_keys!("ps.name = COALESCE(@P1, SCHEMA_NAME()) AND p.name = @P2");

fn fk_action(v: &Value) -> Option<ReferentialAction> {
    get_str(v).and_then(|s| ReferentialAction::parse(&s))
}

/// One row of a foreign-key query from its third column on: name,
/// columns, referenced schema, table and columns, ON DELETE, ON UPDATE,
/// options.
fn fk_info(r: &[Value]) -> Option<ForeignKeyInfo> {
    Some(ForeignKeyInfo {
        name: get_str(&r[0])?,
        columns: split_list(&r[1]),
        ref_schema: get_str(&r[2]),
        ref_table: get_str(&r[3])?,
        ref_columns: split_list(&r[4]),
        on_delete: fk_action(&r[5]),
        on_update: fk_action(&r[6]),
        options: get_str(&r[7]),
    })
}

/// SQLite: a table's `pragma_foreign_key_list` (id, table, from, to,
/// on_delete, on_update, match; one row per column).
const SQLITE_FKS: &str = "SELECT id, \"table\", \"from\", \"to\", on_delete, on_update, \"match\"
 FROM pragma_foreign_key_list(?1, ?2) ORDER BY id, seq";

pub async fn foreign_keys(
    conn: &mut dyn Conn,
    schema: Option<&str>,
    table: &str,
) -> Result<Vec<ForeignKeyInfo>> {
    let (sql, params) = match conn.engine() {
        Engine::Unknown => return Err(DbError::unknown_engine()),
        Engine::Postgres => (
            concat!(
                pg_foreign_keys!(),
                "AND n.nspname = COALESCE($1, current_schema()) AND c.relname = $2 ORDER BY 3"
            ),
            vec![param(schema), param(Some(table))],
        ),
        Engine::Mssql => (MS_FOREIGN_KEYS, vec![param(schema), param(Some(table))]),
        Engine::Sqlite => {
            let db = sqlite_schema(conn, schema).await?;
            let params = vec![param(Some(table)), param(Some(&db.name))];
            let raw = rows(conn, SQLITE_FKS, params).await?;
            return Ok(sqlite_fks(table, schema, &raw));
        }
    };
    Ok(rows(conn, sql, params)
        .await?
        .iter()
        .filter_map(|r| fk_info(&r[2..]))
        .collect())
}

/// [`SQLITE_FKS`] rows as the keys of `table`.
fn sqlite_fks(table: &str, schema: Option<&str>, raw: &[Vec<Value>]) -> Vec<ForeignKeyInfo> {
    let mut out: Vec<(i64, ForeignKeyInfo)> = Vec::new();
    for r in raw {
        let id = get_i64(&r[0]);
        if out.last().is_none_or(|(last, _)| *last != id) {
            out.push((
                id,
                ForeignKeyInfo {
                    name: format!("fk_{table}_{id}"),
                    ref_schema: schema.map(str::to_string),
                    ref_table: get_str(&r[1]).unwrap_or_default(),
                    on_delete: fk_action(&r[4]),
                    on_update: fk_action(&r[5]),
                    options: get_str(&r[6])
                        .filter(|m| !m.eq_ignore_ascii_case("NONE"))
                        .map(|m| format!("MATCH {m}")),
                    ..Default::default()
                },
            ));
        }
        let fk = &mut out.last_mut().expect("pushed above").1;
        fk.columns.extend(get_str(&r[2]));
        // `to` is NULL when the FK references the primary key implicitly.
        fk.ref_columns.extend(get_str(&r[3]));
    }
    out.into_iter().map(|(_, fk)| fk).collect()
}

/// The foreign keys, in any table of the database, that reference `table`
/// (its own self-references included).
pub async fn referencing_keys(
    conn: &mut dyn Conn,
    schema: Option<&str>,
    table: &str,
) -> Result<Vec<ReferencingKey>> {
    let (sql, params) = match conn.engine() {
        Engine::Unknown => return Err(DbError::unknown_engine()),
        Engine::Postgres => (
            concat!(
                pg_foreign_keys!(),
                "AND rn.nspname = COALESCE($1, current_schema()) AND rc.relname = $2
                 ORDER BY 1, 2, 3"
            ),
            vec![param(schema), param(Some(table))],
        ),
        Engine::Mssql => (
            ms_foreign_keys!("rs.name = COALESCE(@P1, SCHEMA_NAME()) AND rt.name = @P2"),
            vec![param(schema), param(Some(table))],
        ),
        Engine::Sqlite => {
            let db = sqlite_schema(conn, schema).await?;
            let sql = format!(
                "SELECT DISTINCT m.name FROM {}.sqlite_schema m, pragma_foreign_key_list(m.name, ?2) f
                 WHERE m.type = 'table' AND f.\"table\" = ?1 COLLATE NOCASE ORDER BY m.name",
                db.quoted
            );
            let tables = rows(conn, &sql, vec![param(Some(table)), param(Some(&db.name))]).await?;
            let mut out = Vec::new();
            for t in tables.iter().filter_map(|r| get_str(&r[0])) {
                let params = vec![param(Some(&t)), param(Some(&db.name))];
                let raw = rows(conn, SQLITE_FKS, params).await?;
                for key in sqlite_fks(&t, schema, &raw) {
                    if key.ref_table.eq_ignore_ascii_case(table) {
                        out.push(ReferencingKey {
                            schema: Some(db.name.clone()),
                            table: t.clone(),
                            key,
                        });
                    }
                }
            }
            return Ok(out);
        }
    };
    Ok(rows(conn, sql, params)
        .await?
        .iter()
        .filter_map(|r| {
            Some(ReferencingKey {
                schema: get_str(&r[0]),
                table: get_str(&r[1])?,
                key: fk_info(&r[2..])?,
            })
        })
        .collect())
}

// ---------------------------------------------------------- table details --

pub async fn table_details(
    conn: &mut dyn Conn,
    schema: Option<&str>,
    table: &str,
) -> Result<TableDetails> {
    let not_found = || DbError::Invalid(format!("table '{table}' not found"));
    let sql = match conn.engine() {
        Engine::Unknown => return Err(DbError::unknown_engine()),
        Engine::Sqlite => {
            sqlite_schema(conn, schema).await?;
            return Ok(TableDetails { comment: None });
        }
        Engine::Postgres => {
            "SELECT obj_description(c.oid, 'pg_class')::text
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = COALESCE($1, current_schema()) AND c.relname = $2"
        }
        Engine::Mssql => {
            "SELECT CAST(ep.value AS nvarchar(max))
             FROM sys.objects o
             JOIN sys.schemas s ON s.schema_id = o.schema_id
             LEFT JOIN sys.extended_properties ep ON ep.class = 1 AND ep.major_id = o.object_id
                    AND ep.minor_id = 0 AND ep.name = 'MS_Description'
             WHERE s.name = COALESCE(@P1, SCHEMA_NAME()) AND o.name = @P2"
        }
    };
    let rows = rows(conn, sql, vec![param(schema), param(Some(table))]).await?;
    let row = rows.first().ok_or_else(not_found)?;
    Ok(TableDetails {
        comment: get_str(&row[0]),
    })
}

// --------------------------------------------------------------- checks --

/// SQL Server CHECK constraints of a table: name, definition, whether the
/// name is generated, NOT FOR REPLICATION.
const MS_CHECKS: &str = "SELECT name, definition, is_system_named, is_not_for_replication
 FROM sys.check_constraints
 WHERE parent_object_id = OBJECT_ID(QUOTENAME(COALESCE(@P1, SCHEMA_NAME())) + '.' + QUOTENAME(@P2))
 ORDER BY name";

pub async fn checks(
    conn: &mut dyn Conn,
    schema: Option<&str>,
    table: &str,
) -> Result<Vec<CheckInfo>> {
    let (sql, params) = match conn.engine() {
        Engine::Unknown => return Err(DbError::unknown_engine()),
        Engine::Postgres => (
            "SELECT con.conname::text, pg_get_expr(con.conbin, con.conrelid, true)::text
             FROM pg_constraint con
             JOIN pg_class c ON c.oid = con.conrelid
             JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE con.contype = 'c' AND n.nspname = COALESCE($1, current_schema()) AND c.relname = $2
             ORDER BY 1",
            vec![param(schema), param(Some(table))],
        ),
        Engine::Mssql => (MS_CHECKS, vec![param(schema), param(Some(table))]),
        Engine::Sqlite => {
            let db = sqlite_schema(conn, schema).await?;
            let sql = format!(
                "SELECT sql FROM {}.sqlite_schema WHERE type = 'table' AND name = ?1",
                db.quoted
            );
            let create = rows(conn, &sql, vec![param(Some(table))])
                .await?
                .first()
                .and_then(|r| get_str(&r[0]))
                .unwrap_or_default();
            return Ok(sqlite_checks(&create));
        }
    };
    Ok(rows(conn, sql, params)
        .await?
        .iter()
        .filter_map(|r| {
            Some(CheckInfo {
                name: get_str(&r[0])?,
                expression: get_str(&r[1])?,
            })
        })
        .collect())
}

// ------------------------------------------------------------- routines --

pub async fn routines(conn: &mut dyn Conn, schema: Option<&str>) -> Result<Vec<RoutineInfo>> {
    let (sql, params) = match conn.engine() {
        Engine::Unknown => return Err(DbError::unknown_engine()),
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

pub async fn ddl(conn: &mut dyn Conn, schema: Option<&str>, name: &str) -> Result<Ddl> {
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
const PG_DDL_COLUMNS: &str = concat!(
    "SELECT quote_ident(a.attname)::text, format_type(a.atttypid, a.atttypmod)::text,
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
 LEFT JOIN pg_namespace cn ON cn.oid = co.collnamespace ",
    pg_owned_sequence!(),
    "
 WHERE n.nspname = $1 AND c.relname = $2 AND a.attnum > 0 AND NOT a.attisdropped
 ORDER BY a.attnum"
);

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
 JOIN sys.types t ON t.user_type_id = c.user_type_id ",
    ms_column_joins!(),
    "
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
    for r in &rows(conn, MS_CHECKS, params()).await? {
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
    for r in rows(conn, MS_FOREIGN_KEYS, params()).await? {
        let Some(fk) = fk_info(&r[2..]) else { continue };
        let action = |a: Option<ReferentialAction>, on: &str| match a {
            Some(
                a @ (ReferentialAction::Cascade
                | ReferentialAction::SetNull
                | ReferentialAction::SetDefault),
            ) => format!(" ON {on} {}", a.sql()),
            _ => String::new(),
        };
        lines.push(format!(
            "    {}FOREIGN KEY ({}) REFERENCES {}.{} ({}){}{}{}",
            ms_constraint(&fk.name, get_bool(&r[10])),
            ms_list(&fk.columns),
            ms_ident(fk.ref_schema.as_deref().unwrap_or("dbo")),
            ms_ident(&fk.ref_table),
            ms_list(&fk.ref_columns),
            action(fk.on_delete, "DELETE"),
            action(fk.on_update, "UPDATE"),
            fk.options.map(|o| format!(" {o}")).unwrap_or_default()
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
    fn sqlite_generated_expressions() {
        let sql = "CREATE TABLE t (id INTEGER PRIMARY KEY, \"has\" TEXT DEFAULT 'a,b',
            total REAL GENERATED ALWAYS AS (price * (1 + tax)) STORED,
            [Label x] TEXT AS (upper(\"has\")) VIRTUAL, alias AS (id), CHECK (id > 0))";
        let e = |c: &str| sqlite_generated_expr(sql, c);
        assert_eq!(e("total").as_deref(), Some("price * (1 + tax)"));
        assert_eq!(e("Label x").as_deref(), Some("upper(\"has\")"));
        assert_eq!(e("ALIAS").as_deref(), Some("id"));
        assert_eq!(e("has"), None);
        assert_eq!(e("missing"), None);
        assert_eq!(
            sqlite_checks(sql),
            [CheckInfo {
                name: String::new(),
                expression: "id > 0".into()
            }]
        );
        let sql = "CREATE TABLE \"t\" (a INTEGER CHECK (a > 0) NOT NULL, b TEXT CHECK(b <> ')'),
            CONSTRAINT \"ck ab\" CHECK ((a, b) IS NOT NULL), constraint x check (length(b) < 9))";
        let names: Vec<(String, String)> = sqlite_checks(sql)
            .into_iter()
            .map(|c| (c.name, c.expression))
            .collect();
        assert_eq!(
            names,
            [
                (String::new(), "a > 0".to_string()),
                (String::new(), "b <> ')'".to_string()),
                ("ck ab".to_string(), "(a, b) IS NOT NULL".to_string()),
                ("x".to_string(), "length(b) < 9".to_string()),
            ]
        );
    }

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
