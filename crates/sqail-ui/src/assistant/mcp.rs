//! `sqail mcp`: a Model Context Protocol server over stdio that lets an AI
//! CLI (Claude Code, Grok) explore one connection: list schemas and tables,
//! describe a table, and run read-only queries. The assistant panel starts
//! the CLI, which starts this server; the service URL, token and connection
//! arrive in the environment (see [`Env`]), never on the command line.

use std::future::Future;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use sqail_client::proto::{
    ColumnInfo, Engine, ForeignKeyInfo, IndexInfo, Param, Plan, PlanNode, QueryEvent, QueryRequest,
    TableInfo, TableKind,
};
use sqail_client::{Client, Identity, On, Target, Trust};
use std::collections::HashMap;

use futures::future::{AbortHandle, FutureExt, abortable, join_all};
use futures::stream::{FuturesUnordered, StreamExt};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Mutex;
use uuid::Uuid;

use super::guard::{called_names, check_read_only, without_terminator};
use crate::sql::lex::{Kind, Token, tokenize_checked};

/// Environment variables the assistant sets for `sqail mcp`.
pub struct Env;

impl Env {
    pub const URL: &'static str = "SQAIL_MCP_URL";
    pub const TOKEN: &'static str = "SQAIL_MCP_TOKEN";
    /// Pinned certificate fingerprint; unset = verify with the OS trust store.
    pub const FINGERPRINT: &'static str = "SQAIL_MCP_FINGERPRINT";
    pub const CLIENT_CERT: &'static str = "SQAIL_MCP_CLIENT_CERT";
    pub const CLIENT_KEY: &'static str = "SQAIL_MCP_CLIENT_KEY";
    pub const CONNECTION: &'static str = "SQAIL_MCP_CONNECTION";
    /// Most rows a query result shows the model (default [`DEFAULT_ROWS`]).
    pub const MAX_ROWS: &'static str = "SQAIL_MCP_MAX_ROWS";
}

pub const DEFAULT_ROWS: u64 = 100;
/// Most rows one assistant query may return (the `assistant.max_rows`
/// setting is clamped to it).
pub const MAX_ROWS_LIMIT: u64 = 1000;
/// Most columns of a result shown to the model.
const MAX_COLUMNS: usize = 50;
/// Most characters in one tool result. Claude Code turns larger MCP output
/// into an error.
const RESULT_CHARS: usize = 40_000;
/// Longest server message shown with a result.
const MESSAGE_CHARS: usize = 500;
/// Tables `list_tables` returns unless asked for another number.
const TABLES_DEFAULT: u64 = 200;
const TABLES_MAX: u64 = 1000;

/// The number of rows the assistant's queries return for the
/// `assistant.max_rows` setting `n`.
pub fn effective_max_rows(n: u64) -> u64 {
    n.clamp(1, MAX_ROWS_LIMIT)
}
/// Longest cell value shown to the model, in characters.
const CELL_CHARS: usize = 200;
const QUERY_TIMEOUT_MS: u64 = 30_000;
/// The temporary view through which PostgreSQL resolves the functions an
/// assistant query calls.
const PG_PROBE: &str = "pg_temp.sqail_assistant_probe";

/// The volatile functions and operator functions the probe view depends on.
const PG_PROBE_FUNCTIONS: &str = "\
SELECT DISTINCT n.nspname::text, p.proname::text, pg_catalog.pg_get_function_identity_arguments(p.oid)
FROM pg_catalog.pg_depend d
JOIN pg_catalog.pg_rewrite r ON r.oid = d.objid
LEFT JOIN pg_catalog.pg_operator o
  ON d.refclassid = 'pg_catalog.pg_operator'::pg_catalog.regclass AND o.oid = d.refobjid
JOIN pg_catalog.pg_proc p ON p.oid = CASE
  WHEN d.refclassid = 'pg_catalog.pg_proc'::pg_catalog.regclass THEN d.refobjid
  ELSE o.oprcode::pg_catalog.oid END
JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace
WHERE d.classid = 'pg_catalog.pg_rewrite'::pg_catalog.regclass
  AND r.ev_class = 'pg_temp.sqail_assistant_probe'::pg_catalog.regclass
  AND p.provolatile = 'v'
ORDER BY 1, 2, 3";

/// Volatile functions in any schema named like one of `$1` (a JSON array of
/// lowercase names), for when the probe view can't be created.
const PG_NAMED_FUNCTIONS: &str = "\
SELECT DISTINCT n.nspname::text, p.proname::text, pg_catalog.pg_get_function_identity_arguments(p.oid)
FROM pg_catalog.pg_proc p
JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace
WHERE p.provolatile = 'v'
  AND pg_catalog.lower(p.proname::text) IN
    (SELECT pg_catalog.jsonb_array_elements_text(($1::text)::pg_catalog.jsonb))
ORDER BY 1, 2, 3";

/// Built-in (`pg_catalog`) functions marked VOLATILE that only read: random
/// numbers, clocks, sizes, sequence and WAL positions, lock and wait
/// information. Everything else the catalog marks volatile is refused.
const READ_ONLY_VOLATILE: &[&str] = &[
    "array_sample",
    "array_shuffle",
    "clock_timestamp",
    "currval",
    "current_query",
    "gen_random_uuid",
    "lastval",
    "pg_blocking_pids",
    "pg_current_wal_flush_lsn",
    "pg_current_wal_insert_lsn",
    "pg_current_wal_lsn",
    "pg_database_size",
    "pg_get_wait_events",
    "pg_indexes_size",
    "pg_is_in_recovery",
    "pg_jit_available",
    "pg_last_wal_receive_lsn",
    "pg_last_wal_replay_lsn",
    "pg_last_xact_replay_timestamp",
    "pg_lock_status",
    "pg_partition_ancestors",
    "pg_partition_tree",
    "pg_relation_size",
    "pg_safe_snapshot_blocking_pids",
    "pg_sequence_last_value",
    "pg_table_size",
    "pg_tablespace_size",
    "pg_total_relation_size",
    "pg_xact_status",
    "random",
    "random_normal",
    "timeofday",
    "txid_status",
];

/// How long an assistant query on SQL Server waits for a lock before it
/// fails, in milliseconds.
pub const MSSQL_LOCK_TIMEOUT_MS: u64 = 5_000;

/// What the tools need from a database. The real one is [`ServiceBackend`].
pub trait Backend {
    fn engine(&self) -> Engine;
    fn schemas(&self) -> impl Future<Output = Result<Vec<String>>>;
    fn tables(&self, schema: Option<&str>) -> impl Future<Output = Result<Vec<TableInfo>>>;
    fn describe(
        &self,
        schema: Option<&str>,
        table: &str,
    ) -> impl Future<Output = Result<(Vec<ColumnInfo>, Vec<IndexInfo>, Vec<ForeignKeyInfo>)>>;
    /// Run one already-checked read-only statement, at most `max_rows` rows.
    fn query(&self, sql: &str, max_rows: u64) -> impl Future<Output = Result<Rows>>;
    /// The estimated plan of one already-checked read-only statement,
    /// which is not run (no ANALYZE).
    fn explain(&self, sql: &str) -> impl Future<Output = Result<Plan>>;
}

#[derive(Debug, Default)]
pub struct Rows {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub truncated: bool,
    pub messages: Vec<String>,
}

/// Protocol versions this server speaks, newest first. The subset used here
/// (tools with text content) is the same in all of them.
const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Serve MCP on `reader`/`writer` (stdin/stdout) until the input ends.
/// Requests run concurrently (a slow query doesn't hold up the schema
/// tools), replies go out as they finish, and `notifications/cancelled`
/// stops a request, which then gets no reply.
pub async fn serve<B: Backend>(
    backend: &B,
    max_rows: u64,
    reader: impl AsyncBufRead + Unpin,
    mut writer: impl AsyncWrite + Unpin,
) -> Result<()> {
    let mut lines = reader.lines();
    let mut inflight = FuturesUnordered::new();
    // Request id (as JSON text) -> how to stop it.
    let mut running: HashMap<String, AbortHandle> = HashMap::new();
    let mut input_open = true;
    loop {
        tokio::select! {
            line = lines.next_line(), if input_open => {
                let Some(line) = line? else {
                    input_open = false;
                    continue;
                };
                if line.trim().is_empty() {
                    continue;
                }
                let msg = match serde_json::from_str::<Value>(&line) {
                    Ok(msg) => msg,
                    Err(e) => {
                        let reply = error_reply(Value::Null, -32700, &format!("parse error: {e}"));
                        write_reply(&mut writer, &reply).await?;
                        continue;
                    }
                };
                if msg.get("method").and_then(Value::as_str) == Some("notifications/cancelled") {
                    let id = msg.pointer("/params/requestId").map(Value::to_string);
                    if let Some(handle) = id.and_then(|id| running.remove(&id)) {
                        handle.abort();
                    }
                    continue;
                }
                let key = msg.get("id").filter(|_| msg.is_object()).map(Value::to_string);
                let (fut, handle) = abortable(handle_message(backend, max_rows, msg));
                if let Some(key) = &key {
                    running.insert(key.clone(), handle);
                }
                inflight.push(async move { (key, fut.await.ok().flatten()) }.boxed_local());
            }
            Some((key, reply)) = inflight.next(), if !inflight.is_empty() => {
                if let Some(key) = key {
                    running.remove(&key);
                }
                if let Some(reply) = reply {
                    write_reply(&mut writer, &reply).await?;
                }
            }
            else => break,
        }
    }
    Ok(())
}

async fn write_reply(writer: &mut (impl AsyncWrite + Unpin), reply: &Value) -> Result<()> {
    let mut out = serde_json::to_vec(reply)?;
    out.push(b'\n');
    writer.write_all(&out).await?;
    writer.flush().await?;
    Ok(())
}

/// One message: a request or notification, or a batch of them (JSON-RPC
/// 2.0; MCP 2025-03-26). A batch gets one array of replies, or none when it
/// held only notifications.
async fn handle_message<B: Backend>(backend: &B, max_rows: u64, msg: Value) -> Option<Value> {
    match msg {
        Value::Array(items) if items.is_empty() => {
            Some(error_reply(Value::Null, -32600, "empty batch"))
        }
        Value::Array(items) => {
            let replies: Vec<Value> =
                join_all(items.into_iter().map(|m| handle(backend, max_rows, m)))
                    .await
                    .into_iter()
                    .flatten()
                    .collect();
            (!replies.is_empty()).then_some(Value::Array(replies))
        }
        msg => handle(backend, max_rows, msg).await,
    }
}

/// One JSON-RPC message in, at most one reply out (none for notifications).
async fn handle<B: Backend>(backend: &B, max_rows: u64, msg: Value) -> Option<Value> {
    if !msg.is_object() {
        return Some(error_reply(Value::Null, -32600, "invalid request"));
    }
    let id = msg.get("id").cloned()?;
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let result = match msg.get("method").and_then(Value::as_str).unwrap_or("") {
        "initialize" => {
            // The client's version if this server speaks it, else the
            // newest this server speaks (the client may then disconnect).
            let asked = params.get("protocolVersion").and_then(Value::as_str);
            let version = asked
                .filter(|v| PROTOCOL_VERSIONS.contains(v))
                .unwrap_or(PROTOCOL_VERSIONS[0]);
            json!({
                "protocolVersion": version,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "sqail", "version": env!("CARGO_PKG_VERSION") },
                "instructions": instructions(backend.engine()),
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tool_definitions(max_rows) }),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            let (text, is_error) = match call_tool(backend, max_rows, name, &args).await {
                Ok(text) => (text, false),
                Err(e) => (format!("{e:#}"), true),
            };
            let text = capped(text);
            json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
        }
        other => return Some(error_reply(id, -32601, &format!("unknown method {other}"))),
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn error_reply(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn instructions(engine: Engine) -> String {
    format!(
        "Tools for exploring a {} database through sqail. Look at the schema before writing \
         queries. run_query only runs single read-only statements and returns a limited \
         number of rows; aggregate or filter instead of reading whole tables. explain_query \
         shows a statement's estimated plan without running it.",
        engine_name(engine)
    )
}

pub fn engine_name(engine: Engine) -> &'static str {
    match engine {
        Engine::Postgres => "PostgreSQL",
        Engine::Mssql => "SQL Server",
        Engine::Sqlite => "SQLite",
        Engine::Unknown => "an engine this sqail version does not support",
    }
}

fn tool_definitions(max_rows: u64) -> Value {
    json!([
        {
            "name": "list_schemas",
            "description": "List the schemas (namespaces) of the database.",
            "inputSchema": { "type": "object", "properties": {} },
            "annotations": { "readOnlyHint": true },
        },
        {
            "name": "list_tables",
            "description": format!(
                "List tables and views, as schema.name (kind), at most `limit` (default \
                 {TABLES_DEFAULT}). Without `schema` it covers every schema; when that is too many \
                 to list it returns the number per schema instead: then ask again with `schema` \
                 and/or `pattern`."
            ),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "schema": { "type": "string", "description": "Only this schema" },
                    "pattern": {
                        "type": "string",
                        "description": "Only names containing this text, in any case; with `*` or `%` (any run of characters) the whole name must match, e.g. `order*`"
                    },
                    "limit": { "type": "integer", "minimum": 1, "maximum": TABLES_MAX },
                },
            },
            "annotations": { "readOnlyHint": true },
        },
        {
            "name": "describe_table",
            "description": "Columns (type, nullability, primary key, default), indexes and foreign keys of one table or view.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "table": { "type": "string", "description": "Table or view name, optionally schema.name; quote names that contain dots or spaces" },
                    "schema": { "type": "string" },
                },
                "required": ["table"],
            },
            "annotations": { "readOnlyHint": true },
        },
        {
            "name": "run_query",
            "description": format!(
                "Run one read-only SQL statement (SELECT / WITH / VALUES) and return up to {max_rows} rows \
                 as a table. Writes, DDL, EXEC and multiple statements are refused. It runs in a \
                 transaction that is rolled back. Use it to check data and to verify a query before \
                 proposing it. The result shows at most {MAX_COLUMNS} columns, {CELL_CHARS} \
                 characters per value and {RESULT_CHARS} characters in all: select the columns \
                 you need."
            ),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "sql": { "type": "string" },
                    "max_rows": { "type": "integer", "minimum": 1, "maximum": max_rows },
                },
                "required": ["sql"],
            },
            "annotations": { "readOnlyHint": true },
        },
        {
            "name": "explain_query",
            "description": "The estimated execution plan of one read-only SQL statement (SELECT / WITH / \
                VALUES), as a tree of operators with estimated cost and rows. The statement is not \
                run (no ANALYZE), so it is quick and safe on slow queries. Pass the statement \
                itself, without EXPLAIN. Use it to see why a query is slow and to compare rewrites.",
            "inputSchema": {
                "type": "object",
                "properties": { "sql": { "type": "string" } },
                "required": ["sql"],
            },
            "annotations": { "readOnlyHint": true },
        },
    ])
}

async fn call_tool<B: Backend>(
    backend: &B,
    max_rows: u64,
    name: &str,
    args: &Value,
) -> Result<String> {
    let arg = |k: &str| {
        args.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };
    match name {
        "list_schemas" => Ok(backend.schemas().await?.join("\n")),
        "list_tables" => {
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(TABLES_DEFAULT)
                .clamp(1, TABLES_MAX) as usize;
            let pattern = arg("pattern");
            let schema = arg("schema");
            let tables: Vec<TableInfo> = backend
                .tables(schema)
                .await?
                .into_iter()
                .filter(|t| pattern.is_none_or(|p| name_matches(p, &t.name)))
                .collect();
            Ok(tables_text(
                &tables,
                limit,
                schema.is_none() && pattern.is_none(),
            ))
        }
        "describe_table" => {
            let table = arg("table").ok_or_else(|| anyhow!("`table` is required"))?;
            let (qualifier, table) = split_name(table, backend.engine())?;
            let schema = arg("schema").map(str::to_string).or(qualifier);
            let (cols, idx, fks) = backend.describe(schema.as_deref(), &table).await?;
            if cols.is_empty() {
                return Err(anyhow!("table {table} not found (use list_tables)"));
            }
            Ok(describe_text(&cols, &idx, &fks))
        }
        "run_query" => {
            let sql = arg("sql").ok_or_else(|| anyhow!("`sql` is required"))?;
            check_read_only(sql, backend.engine()).map_err(|e| anyhow!("refused: {e}"))?;
            let n = args
                .get("max_rows")
                .and_then(Value::as_u64)
                .unwrap_or(max_rows)
                .clamp(1, max_rows);
            Ok(rows_text(&backend.query(sql, n).await?, n))
        }
        "explain_query" => {
            let sql = arg("sql").ok_or_else(|| anyhow!("`sql` is required"))?;
            let first = sql.split_whitespace().next().unwrap_or("");
            if first.eq_ignore_ascii_case("EXPLAIN") {
                return Err(anyhow!(
                    "pass the statement itself, without EXPLAIN: explain_query never runs it"
                ));
            }
            check_read_only(sql, backend.engine()).map_err(|e| anyhow!("refused: {e}"))?;
            Ok(plan_text(&backend.explain(sql).await?))
        }
        other => Err(anyhow!("unknown tool {other}")),
    }
}

/// `name` matches `pattern`: case-insensitive, `*` and `%` match any run of
/// characters, and without them the pattern may match anywhere.
fn name_matches(pattern: &str, name: &str) -> bool {
    let pattern = pattern.to_lowercase().replace('%', "*");
    let name = name.to_lowercase();
    if !pattern.contains('*') {
        return name.contains(&pattern);
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    let (first, rest) = parts.split_first().expect("split yields one part at least");
    let Some(mut tail) = name.strip_prefix(first) else {
        return false;
    };
    let (last, middle) = rest.split_last().expect("a `*` makes two parts");
    for part in middle {
        match tail.find(part) {
            Some(i) => tail = &tail[i + part.len()..],
            None => return false,
        }
    }
    tail.len() >= last.len() && tail.ends_with(last)
}

/// At most `limit` tables, one per line; when `whole_database` and there are
/// more, the number per schema instead.
fn tables_text(tables: &[TableInfo], limit: usize, whole_database: bool) -> String {
    if tables.is_empty() {
        return "(no tables)".into();
    }
    if whole_database && tables.len() > limit {
        let mut counts: Vec<(&str, usize)> = Vec::new();
        for t in tables {
            let s = t.schema.as_deref().unwrap_or("(default)");
            match counts.iter_mut().find(|(n, _)| *n == s) {
                Some((_, c)) => *c += 1,
                None => counts.push((s, 1)),
            }
        }
        let mut out = format!(
            "{} tables and views in {} schemas, too many to list. Per schema:\n",
            tables.len(),
            counts.len()
        );
        for (s, n) in counts {
            out.push_str(&format!("{s}: {n}\n"));
        }
        out.push_str("List them with `schema` and/or `pattern`.");
        return out;
    }
    let mut out: Vec<String> = tables
        .iter()
        .take(limit)
        .map(|t| {
            let kind = match t.kind {
                TableKind::Table => "table",
                TableKind::View => "view",
                TableKind::MaterializedView => "materialized view",
            };
            match &t.schema {
                Some(s) => format!("{s}.{} ({kind})", t.name),
                None => format!("{} ({kind})", t.name),
            }
        })
        .collect();
    if tables.len() > limit {
        out.push(format!(
            "(… and {} more: narrow it down with `pattern`)",
            tables.len() - limit
        ));
    }
    out.join("\n")
}

/// `[db.]schema.table` with the engine's identifier rules: quoted parts
/// keep their dots and case, doubled quotes unescaped. The database part,
/// if any, is ignored (the connection has one).
fn split_name(name: &str, engine: Engine) -> Result<(Option<String>, String)> {
    let (tokens, unterminated) = tokenize_checked(name, Some(engine));
    let tokens: Vec<Token> = tokens
        .into_iter()
        .filter(|t| !matches!(t.kind, Kind::Space | Kind::Comment))
        .collect();
    let text = |t: &Token| &name[t.range.clone()];
    let mut parts = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        let ok = if i % 2 == 0 {
            matches!(t.kind, Kind::Word | Kind::QuotedIdent | Kind::Number)
        } else {
            text(t) == "."
        };
        if !ok || unterminated {
            // Not a plain qualified name: take it as one name.
            return Ok((None, name.to_string()));
        }
        if i % 2 == 0 {
            parts.push(match t.kind {
                Kind::QuotedIdent => unquote_name(text(t)),
                _ => text(t).to_string(),
            });
        }
    }
    if tokens.len().is_multiple_of(2) {
        return Ok((None, name.to_string()));
    }
    match parts.len() {
        1 => Ok((None, parts.remove(0))),
        2 | 3 => {
            let table = parts.pop().expect("two parts at least");
            Ok((parts.pop(), table))
        }
        _ => Err(anyhow!(
            "can't read the table name {name:?}: give it as schema.table, quoting parts that contain dots"
        )),
    }
}

/// A quoted name's text: quotes stripped, doubled closing quotes unescaped.
fn unquote_name(raw: &str) -> String {
    let close = match raw.as_bytes().first() {
        Some(b'[') => ']',
        Some(&q) => q as char,
        None => return String::new(),
    };
    let inner = raw.get(1..raw.len().saturating_sub(1)).unwrap_or("");
    inner.replace(&format!("{close}{close}"), &close.to_string())
}

/// `text` cut to [`RESULT_CHARS`] at a line end, saying what was cut.
fn capped(text: String) -> String {
    if text.len() <= RESULT_CHARS {
        return text;
    }
    let mut end = RESULT_CHARS;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let end = text[..end].rfind('\n').unwrap_or(end);
    let rest = &text[end..];
    let lines = rest.trim_start_matches('\n').lines().count();
    format!(
        "{}\n(output cut at {RESULT_CHARS} characters: {lines} more line{} not shown; narrow the request)",
        &text[..end],
        if lines == 1 { "" } else { "s" }
    )
}

/// A plan as an indented tree: operator, object, estimated cost and rows,
/// and the operator's details. Falls back to the engine's own output.
fn plan_text(plan: &Plan) -> String {
    fn node(out: &mut String, n: &PlanNode, depth: usize) {
        let pad = "  ".repeat(depth);
        out.push_str(&format!("{pad}-> {}", n.label));
        if let Some(o) = &n.object {
            out.push_str(&format!(" on {o}"));
        }
        let mut est = Vec::new();
        if let Some(c) = n.cost {
            est.push(format!("cost={c:.2}"));
        }
        if let Some(r) = n.rows {
            est.push(format!("rows={r:.0}"));
        }
        if !est.is_empty() {
            out.push_str(&format!(" ({})", est.join(" ")));
        }
        out.push('\n');
        for p in &n.props {
            let v = p.value.replace('\n', " ");
            let v: String = v.chars().take(300).collect();
            out.push_str(&format!("{pad}     {}: {v}\n", p.key));
        }
        for c in &n.children {
            node(out, c, depth + 1);
        }
    }
    if plan.roots.is_empty() {
        return plan.raw.clone();
    }
    let mut out = String::from("Estimated plan (not run):\n");
    for root in &plan.roots {
        node(&mut out, root, 0);
    }
    out
}

fn describe_text(cols: &[ColumnInfo], idx: &[IndexInfo], fks: &[ForeignKeyInfo]) -> String {
    let mut out = String::from("Columns:\n");
    for c in cols {
        out.push_str(&format!(
            "  {} {}{}{}{}\n",
            c.name,
            c.data_type,
            if c.nullable { "" } else { " NOT NULL" },
            if c.primary_key { " PRIMARY KEY" } else { "" },
            c.default
                .as_deref()
                .map(|d| format!(" DEFAULT {d}"))
                .unwrap_or_default(),
        ));
    }
    if !idx.is_empty() {
        out.push_str("Indexes:\n");
        for i in idx {
            out.push_str(&format!(
                "  {} ({}){}{}\n",
                i.name,
                i.columns.join(", "),
                if i.unique { " UNIQUE" } else { "" },
                if i.primary { " PRIMARY" } else { "" },
            ));
        }
    }
    if !fks.is_empty() {
        out.push_str("Foreign keys:\n");
        for f in fks {
            let target = match &f.ref_schema {
                Some(s) => format!("{s}.{}", f.ref_table),
                None => f.ref_table.clone(),
            };
            out.push_str(&format!(
                "  {} ({}) -> {target} ({})\n",
                f.name,
                f.columns.join(", "),
                f.ref_columns.join(", ")
            ));
        }
    }
    out
}

/// Rows as a pipe-separated table, which models read well and cheaply:
/// at most [`MAX_COLUMNS`] columns and about [`RESULT_CHARS`] characters,
/// saying what was left out.
fn rows_text(r: &Rows, max_rows: u64) -> String {
    let mut out = String::new();
    for m in &r.messages {
        let m: String = m.chars().take(MESSAGE_CHARS).collect();
        out.push_str(&format!("Message: {m}\n"));
    }
    if r.columns.is_empty() {
        out.push_str("(no result set)");
        return out;
    }
    let ncols = r.columns.len().min(MAX_COLUMNS);
    out.push_str(&r.columns[..ncols].join(" | "));
    out.push('\n');
    // Leave room for the closing note.
    let budget = RESULT_CHARS.saturating_sub(400);
    let mut shown = 0;
    for row in &r.rows {
        let cells: Vec<String> = row.iter().take(ncols).map(cell_text).collect();
        let line = cells.join(" | ");
        if out.len() + line.len() + 1 > budget {
            break;
        }
        out.push_str(&line);
        out.push('\n');
        shown += 1;
    }
    let n = r.rows.len();
    out.push_str(&format!("({n} row{}", if n == 1 { "" } else { "s" }));
    if r.truncated {
        out.push_str(&format!(
            "; stopped at {max_rows}, more rows exist: aggregate, filter or add ORDER BY … LIMIT/TOP"
        ));
    }
    if shown < n {
        out.push_str(&format!(
            "; only the first {shown} shown, the output is limited to {RESULT_CHARS} characters"
        ));
    }
    if ncols < r.columns.len() {
        out.push_str(&format!(
            "; only the first {ncols} of {} columns shown: select the columns you need",
            r.columns.len()
        ));
    }
    out.push(')');
    out
}

fn cell_text(v: &Value) -> String {
    let s = match v {
        Value::Null => return "NULL".into(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let s = s.replace(['\n', '\r'], " ").replace('|', "\\|");
    match s.char_indices().nth(CELL_CHARS) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s,
    }
}

/// The real backend: sqail-service through the typed client. Queries run on
/// a dedicated session, each inside a transaction that is rolled back.
pub struct ServiceBackend {
    client: Client,
    connection: Uuid,
    engine: Engine,
    session: Mutex<Option<Uuid>>,
    /// Held while a query runs on the session, which runs one at a time.
    /// True while a query runs: still true when the next one starts means
    /// the last one was stopped midway (`notifications/cancelled`) or
    /// failed to roll back, so its transaction may still be open, and the
    /// session is replaced (closing it rolls back).
    turn: Mutex<bool>,
    /// SQL Server: the statements that start each assistant transaction,
    /// decided on first use (see [`ServiceBackend::mssql_begin`]).
    mssql_begin: Mutex<Option<String>>,
}

impl ServiceBackend {
    pub async fn connect(target: &Target, connection: Uuid) -> Result<Self> {
        let client = Client::new(target).context("creating the service client")?;
        let engine = client
            .connection(connection)
            .await
            .context("looking up the connection on the service")?
            .engine;
        Ok(Self {
            client,
            connection,
            engine,
            session: Mutex::new(None),
            turn: Mutex::new(false),
            mssql_begin: Mutex::new(None),
        })
    }

    /// Wait for the session, replacing it if the last query on it was left
    /// unfinished. Set the guard back to false once the transaction is
    /// rolled back.
    async fn begin_turn(&self) -> Result<(tokio::sync::MutexGuard<'_, bool>, On)> {
        let mut turn = self.turn.lock().await;
        if *turn && let Some(id) = self.session.lock().await.take() {
            let _ = self.client.close_session(id).await;
        }
        *turn = true;
        let id = self.session().await?;
        Ok((turn, On::Session(id)))
    }

    async fn session(&self) -> Result<Uuid> {
        let mut s = self.session.lock().await;
        if let Some(id) = *s {
            return Ok(id);
        }
        let id = self.client.open_session(self.connection).await?.id;
        *s = Some(id);
        Ok(id)
    }

    /// SQL Server has no read-only transaction, so an assistant query must
    /// at least not block others or wait on them for long: a lock timeout,
    /// and an isolation level that takes no shared locks. That is SNAPSHOT
    /// where the database allows it (consistent reads from row versions),
    /// else READ UNCOMMITTED (which may read uncommitted changes). Set
    /// before every transaction, so a reset connection cannot lose it.
    async fn mssql_begin(&self, on: On) -> Result<String> {
        let mut cached = self.mssql_begin.lock().await;
        if let Some(sql) = &*cached {
            return Ok(sql.clone());
        }
        let mut req = QueryRequest::new(
            "SELECT snapshot_isolation_state FROM sys.databases WHERE database_id = DB_ID()",
        );
        req.max_rows = Some(1);
        let snapshot = self
            .client
            .query_all(on, &req)
            .await?
            .into_iter()
            .any(|ev| match ev {
                QueryEvent::Rows { rows, .. } => {
                    rows.first().and_then(|r| r.first()).and_then(Value::as_i64) == Some(1)
                }
                _ => false,
            });
        let level = if snapshot {
            "SNAPSHOT"
        } else {
            "READ UNCOMMITTED"
        };
        let sql = format!(
            "SET LOCK_TIMEOUT {MSSQL_LOCK_TIMEOUT_MS}; \
             SET TRANSACTION ISOLATION LEVEL {level}; BEGIN TRANSACTION"
        );
        *cached = Some(sql.clone());
        Ok(sql)
    }

    /// PostgreSQL: refuse a query that calls a function the catalog marks
    /// VOLATILE (it may have side effects), unless it is a known read-only
    /// built-in ([`READ_ONLY_VOLATILE`]). The server resolves the names
    /// itself (search path, overloads, operators, quoted and qualified
    /// names): the query is wrapped in a temporary view, which parses it
    /// without running it, and the view's dependencies list the functions
    /// and operators it calls. The view lives in the assistant's transaction
    /// and goes with the rollback. Then the transaction is made read-only
    /// for the query itself.
    ///
    /// Where the transaction can't create the view (a standby, or
    /// `default_transaction_read_only`), the check falls back to the names
    /// written as calls in the query, refusing any of them that names a
    /// volatile function in any schema.
    async fn pg_check_functions(&self, on: On, sql: &str) -> Result<()> {
        let body = without_terminator(sql, Engine::Postgres);
        let show = body
            .trim_start()
            .get(..4)
            .is_some_and(|w| w.eq_ignore_ascii_case("SHOW"));
        let mut probed = false;
        if !show {
            // The guard checked that the parentheses balance, so the query
            // means the same inside the subquery.
            let mut req = QueryRequest::new(format!(
                "CREATE TEMPORARY VIEW {PG_PROBE} AS SELECT 1 FROM (\n{body}\n) AS sqail_q"
            ));
            req.timeout_ms = Some(QUERY_TIMEOUT_MS);
            req.single_statement = true;
            let events = self.client.query_all(on, &req).await?;
            match events.into_iter().find_map(|ev| match ev {
                QueryEvent::Error {
                    message, db_code, ..
                } => Some((message, db_code)),
                _ => None,
            }) {
                None => probed = true,
                // A read-only transaction: check by name below.
                Some((_, Some(code))) if code == "25006" => {
                    run_all(&self.client, on, "ROLLBACK", 1).await?;
                    run_all(&self.client, on, "BEGIN", 1).await?;
                }
                Some((message, Some(code))) if code == "42501" && message.contains("temporary") => {
                    bail!(
                        "refused: sqail checks which functions a query calls through a temporary \
                     view, and this login may not create one (grant it TEMPORARY on the \
                     database): {message}"
                    )
                }
                // The query itself is wrong: the same error as running it.
                Some((message, _)) => bail!("{message}"),
            }
        }
        run_all(&self.client, on, "SET TRANSACTION READ ONLY", 1).await?;
        if show {
            return Ok(());
        }
        let mut req = if probed {
            QueryRequest::new(PG_PROBE_FUNCTIONS)
        } else {
            let names = called_names(body, Engine::Postgres);
            if names.is_empty() {
                return Ok(());
            }
            let mut req = QueryRequest::new(PG_NAMED_FUNCTIONS);
            req.params = vec![Param::Text(serde_json::to_string(&names)?)];
            req
        };
        req.max_rows = Some(20);
        let mut volatile = Vec::new();
        for ev in self.client.query_all(on, &req).await? {
            match ev {
                QueryEvent::Rows { rows, .. } => {
                    for row in rows {
                        let text = |i: usize| row.get(i).and_then(Value::as_str).unwrap_or("");
                        let (schema, name, args) = (text(0), text(1), text(2));
                        if !(schema == "pg_catalog" && READ_ONLY_VOLATILE.contains(&name)) {
                            volatile.push(format!("{schema}.{name}({args})"));
                        }
                    }
                }
                QueryEvent::Error { message, .. } => {
                    bail!("checking the functions the query calls: {message}")
                }
                _ => {}
            }
        }
        if !volatile.is_empty() {
            bail!(
                "refused: the query calls {}, which PostgreSQL marks VOLATILE (it may change \
                 something); the assistant may only call STABLE or IMMUTABLE functions",
                volatile.join(", ")
            );
        }
        Ok(())
    }

    pub async fn close(&self) {
        if let Some(id) = self.session.lock().await.take() {
            let _ = self.client.close_session(id).await;
        }
    }
}

impl Backend for ServiceBackend {
    fn engine(&self) -> Engine {
        self.engine
    }

    async fn explain(&self, sql: &str) -> Result<Plan> {
        self.plan(sql).await
    }

    async fn schemas(&self) -> Result<Vec<String>> {
        Ok(self
            .client
            .schemas(self.connection)
            .await?
            .into_iter()
            .map(|s| s.name)
            .collect())
    }

    async fn tables(&self, schema: Option<&str>) -> Result<Vec<TableInfo>> {
        Ok(self.client.tables(self.connection, schema).await?)
    }

    async fn describe(
        &self,
        schema: Option<&str>,
        table: &str,
    ) -> Result<(Vec<ColumnInfo>, Vec<IndexInfo>, Vec<ForeignKeyInfo>)> {
        let c = &self.client;
        let (cols, idx, fks) = tokio::join!(
            c.columns(self.connection, schema, table),
            c.indexes(self.connection, schema, table),
            c.foreign_keys(self.connection, schema, table),
        );
        Ok((cols?, idx.unwrap_or_default(), fks.unwrap_or_default()))
    }

    async fn query(&self, sql: &str, max_rows: u64) -> Result<Rows> {
        let (mut turn, session) = self.begin_turn().await?;
        let begin = match self.engine {
            // Read-only from the function check on, see `pg_check_functions`.
            Engine::Postgres => "BEGIN".to_string(),
            Engine::Mssql => self.mssql_begin(session).await?,
            Engine::Sqlite => "BEGIN".to_string(),
            // No known way to make it read-only, so don't run anything.
            Engine::Unknown => anyhow::bail!("this engine is not supported by this sqail version"),
        };
        run_all(&self.client, session, &begin, 1).await?;
        let result = async {
            if self.engine == Engine::Postgres {
                self.pg_check_functions(session, sql).await?;
            }
            let mut req = QueryRequest::new(sql);
            req.max_rows = Some(max_rows);
            req.timeout_ms = Some(QUERY_TIMEOUT_MS);
            // The service refuses a second statement even where the guard's
            // lexer and the engine disagree on where a statement ends.
            req.single_statement = true;
            Ok::<_, anyhow::Error>(self.client.query_all(session, &req).await?)
        }
        .await;
        // Always undo, whatever the query did or however it failed.
        let rollback = run_all(&self.client, session, "ROLLBACK", 1).await;
        *turn = rollback.is_err();
        let events = result?;
        rollback.context("rolling back the assistant's transaction")?;
        let mut out = Rows::default();
        for ev in events {
            match ev {
                QueryEvent::ResultStart { index: 0, columns } => {
                    out.columns = columns.into_iter().map(|c| c.name).collect();
                }
                QueryEvent::Rows { index: 0, rows } => out.rows.extend(rows),
                QueryEvent::ResultEnd {
                    index: 0,
                    truncated,
                    ..
                } => out.truncated = truncated,
                QueryEvent::Message { text, .. } => out.messages.push(text),
                QueryEvent::Error { message, .. } => return Err(anyhow!("{message}")),
                _ => {}
            }
        }
        Ok(out)
    }
}

impl ServiceBackend {
    /// The plan through the service's explain endpoint, without ANALYZE,
    /// on the assistant's session. On PostgreSQL inside a read-only
    /// transaction that is rolled back, should anything run after all.
    async fn plan(&self, sql: &str) -> Result<Plan> {
        let (mut turn, session) = self.begin_turn().await?;
        let pg = self.engine == Engine::Postgres;
        if pg {
            run_all(&self.client, session, "BEGIN READ ONLY", 1).await?;
        }
        let plan = self.client.explain(session, sql, false).await;
        let rollback = if pg {
            run_all(&self.client, session, "ROLLBACK", 1).await
        } else {
            Ok(())
        };
        *turn = rollback.is_err();
        let plan = plan?;
        rollback.context("rolling back the assistant's transaction")?;
        Ok(plan)
    }
}

async fn run_all(client: &Client, on: On, sql: &str, max_rows: u64) -> Result<()> {
    let mut req = QueryRequest::new(sql);
    req.max_rows = Some(max_rows);
    for ev in client.query_all(on, &req).await? {
        if let QueryEvent::Error { message, .. } = ev {
            return Err(anyhow!("{sql}: {message}"));
        }
    }
    Ok(())
}

/// Entry point of `sqail mcp`: configuration from [`Env`], MCP on stdio.
pub fn run_from_env() -> Result<()> {
    let var = |k: &str| {
        std::env::var(k)
            .map_err(|_| anyhow!("{k} is not set; `sqail mcp` is started by sqail's assistant"))
    };
    let url = var(Env::URL)?;
    let token = var(Env::TOKEN)?;
    let connection: Uuid = var(Env::CONNECTION)?
        .parse()
        .context("SQAIL_MCP_CONNECTION")?;
    let trust = match std::env::var(Env::FINGERPRINT) {
        Ok(fp) if !fp.is_empty() => Trust::Pinned(fp),
        _ => Trust::System,
    };
    let identity = match (
        std::env::var_os(Env::CLIENT_CERT),
        std::env::var_os(Env::CLIENT_KEY),
    ) {
        (Some(c), Some(k)) => Some(Identity::from_files(Path::new(&c), Path::new(&k))?),
        _ => None,
    };
    let max_rows = std::env::var(Env::MAX_ROWS)
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(DEFAULT_ROWS, effective_max_rows);
    let target = Target {
        url,
        token,
        trust,
        identity,
    };
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let backend = ServiceBackend::connect(&target, connection).await?;
            let stdin = tokio::io::BufReader::new(tokio::io::stdin());
            let res = serve(&backend, max_rows, stdin, tokio::io::stdout()).await;
            backend.close().await;
            res
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqail_client::proto::PlanProp;

    struct Fake;

    impl Backend for Fake {
        fn engine(&self) -> Engine {
            Engine::Postgres
        }
        async fn schemas(&self) -> Result<Vec<String>> {
            Ok(vec!["public".into(), "sales".into()])
        }
        async fn tables(&self, _schema: Option<&str>) -> Result<Vec<TableInfo>> {
            Ok(vec![TableInfo {
                schema: Some("sales".into()),
                name: "orders".into(),
                kind: TableKind::Table,
            }])
        }
        async fn describe(
            &self,
            schema: Option<&str>,
            table: &str,
        ) -> Result<(Vec<ColumnInfo>, Vec<IndexInfo>, Vec<ForeignKeyInfo>)> {
            assert_eq!((schema, table), (Some("sales"), "orders"));
            Ok((
                vec![ColumnInfo {
                    name: "id".into(),
                    ordinal: 1,
                    data_type: "integer".into(),
                    nullable: false,
                    default: None,
                    primary_key: true,
                }],
                vec![],
                vec![ForeignKeyInfo {
                    name: "fk_customer".into(),
                    columns: vec!["customer_id".into()],
                    ref_schema: Some("sales".into()),
                    ref_table: "customers".into(),
                    ref_columns: vec!["id".into()],
                }],
            ))
        }
        async fn explain(&self, sql: &str) -> Result<Plan> {
            let leaf = PlanNode {
                label: "Seq Scan".into(),
                object: Some("sales.orders".into()),
                cost: Some(12.5),
                rows: Some(300.0),
                actual_rows: None,
                actual_ms: None,
                props: vec![PlanProp {
                    key: "Filter".into(),
                    value: sql.into(),
                }],
                children: vec![],
            };
            Ok(Plan {
                roots: vec![PlanNode {
                    label: "Aggregate".into(),
                    object: None,
                    cost: Some(13.0),
                    rows: Some(1.0),
                    actual_rows: None,
                    actual_ms: None,
                    props: vec![],
                    children: vec![leaf],
                }],
                raw: String::new(),
                analyzed: false,
            })
        }
        async fn query(&self, sql: &str, max_rows: u64) -> Result<Rows> {
            if sql.contains("'slow'") {
                // Runs until it is cancelled.
                std::future::pending::<()>().await;
            }
            Ok(Rows {
                columns: vec!["sql".into(), "n".into()],
                rows: vec![
                    vec![json!(sql), json!(max_rows)],
                    vec![Value::Null, json!("a|b\nc")],
                ],
                truncated: true,
                messages: vec![],
            })
        }
    }

    /// The replies to `input`, in the order of their ids (requests run
    /// concurrently, so replies may come in any order); batches last.
    async fn exchange(input: &str) -> Vec<Value> {
        let mut out = Vec::new();
        serve(&Fake, 50, input.as_bytes(), &mut out).await.unwrap();
        let mut replies: Vec<Value> = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        replies.sort_by_key(|r| r["id"].as_u64().unwrap_or(u64::MAX));
        replies
    }

    fn call(id: u32, name: &str, args: Value) -> String {
        json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": args}})
            .to_string()
            + "\n"
    }

    fn text(reply: &Value) -> (&str, bool) {
        let r = &reply["result"];
        (
            r["content"][0]["text"].as_str().unwrap(),
            r["isError"].as_bool().unwrap(),
        )
    }

    #[tokio::test]
    async fn handshake_and_tool_list() {
        let replies = exchange(concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":3,"method":"nope"}"#,
            "\n",
        ))
        .await;
        // No reply to the notification.
        assert_eq!(replies.len(), 3);
        assert_eq!(replies[0]["result"]["protocolVersion"], "2025-03-26");
        // A version this server doesn't speak gets its newest.
        let newer = exchange(concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2099-01-01"}}"#,
            "\n",
        ))
        .await;
        assert_eq!(newer[0]["result"]["protocolVersion"], PROTOCOL_VERSIONS[0]);
        let names: Vec<&str> = replies[1]["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "list_schemas",
                "list_tables",
                "describe_table",
                "run_query",
                "explain_query"
            ]
        );
        assert_eq!(replies[2]["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn tools_answer_in_text() {
        let replies = exchange(
            &[
                call(1, "list_tables", json!({})),
                call(2, "describe_table", json!({"table": "sales.orders"})),
                call(3, "run_query", json!({"sql": "SELECT 1", "max_rows": 500})),
                call(4, "run_query", json!({"sql": "DELETE FROM sales.orders"})),
            ]
            .concat(),
        )
        .await;
        assert_eq!(text(&replies[0]), ("sales.orders (table)", false));
        let (desc, err) = text(&replies[1]);
        assert!(!err);
        assert!(desc.contains("id integer NOT NULL PRIMARY KEY"), "{desc}");
        assert!(
            desc.contains("fk_customer (customer_id) -> sales.customers (id)"),
            "{desc}"
        );
        // max_rows is capped at the server's limit (50 here).
        let (rows, err) = text(&replies[2]);
        assert!(!err);
        assert!(
            rows.starts_with("sql | n\nSELECT 1 | 50\nNULL | a\\|b c\n"),
            "{rows}"
        );
        assert!(rows.contains("more rows exist"), "{rows}");
        let (refused, err) = text(&replies[3]);
        assert!(err);
        assert!(refused.starts_with("refused:"), "{refused}");
    }

    #[tokio::test]
    async fn batches_get_one_array_of_replies() {
        let replies = exchange(concat!(
            r#"[{"jsonrpc":"2.0","id":1,"method":"ping"},{"jsonrpc":"2.0","method":"notifications/initialized"},{"jsonrpc":"2.0","id":2,"method":"nope"}]"#,
            "\n",
            r#"[{"jsonrpc":"2.0","method":"notifications/initialized"}]"#,
            "\n",
            "[]\n",
            "[1]\n",
        ))
        .await;
        // Nothing for the batch of notifications only.
        assert_eq!(replies.len(), 3, "{replies:?}");
        let empty = replies.iter().find(|r| r.is_object()).unwrap();
        assert_eq!(empty["error"]["code"], -32600);
        let arrays: Vec<&Vec<Value>> = replies.iter().filter_map(Value::as_array).collect();
        let full = arrays.iter().find(|a| a.len() == 2).unwrap();
        assert_eq!(full[0]["id"], 1);
        assert_eq!(full[0]["result"], json!({}));
        assert_eq!(full[1]["error"]["code"], -32601);
        let invalid = arrays.iter().find(|a| a.len() == 1).unwrap();
        assert_eq!(invalid[0]["error"]["code"], -32600);
    }

    #[tokio::test]
    async fn cancelled_requests_stop_and_others_go_on() {
        let replies = exchange(
            &[
                call(1, "run_query", json!({"sql": "SELECT 'slow'"})),
                call(2, "list_tables", json!({})),
                json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
                       "params": {"requestId": 1, "reason": "user"}})
                .to_string()
                    + "\n",
                call(3, "run_query", json!({"sql": "SELECT 1"})),
            ]
            .concat(),
        )
        .await;
        // The slow query never answers (serve would not return otherwise),
        // and doesn't hold up the others.
        let ids: Vec<u64> = replies.iter().map(|r| r["id"].as_u64().unwrap()).collect();
        assert_eq!(ids, [2, 3]);
    }

    #[test]
    fn names_follow_identifier_rules() {
        let pg = Engine::Postgres;
        let split = |n: &str, e| split_name(n, e).unwrap();
        assert_eq!(split("orders", pg), (None, "orders".into()));
        assert_eq!(
            split("sales.orders", pg),
            (Some("sales".into()), "orders".into())
        );
        assert_eq!(
            split("db.dbo.t", Engine::Mssql),
            (Some("dbo".into()), "t".into())
        );
        assert_eq!(split(r#""a.b"."T""#, pg), (Some("a.b".into()), "T".into()));
        assert_eq!(split(r#""x""y""#, pg), (None, r#"x"y"#.into()));
        assert_eq!(
            split("[my schema].[t.1]", Engine::Mssql),
            (Some("my schema".into()), "t.1".into())
        );
        assert_eq!(
            split("`a`.`b`", Engine::Sqlite),
            (Some("a".into()), "b".into())
        );
        // Not a qualified name: taken as one.
        assert_eq!(split("my table", pg), (None, "my table".into()));
        assert!(split_name("a.b.c.d", pg).is_err());
    }

    #[test]
    fn table_lists_stay_short() {
        let t = |schema: &str, name: &str| TableInfo {
            schema: Some(schema.into()),
            name: name.into(),
            kind: TableKind::Table,
        };
        let tables: Vec<TableInfo> = (0..300)
            .map(|i| t(if i < 290 { "big" } else { "small" }, &format!("t{i}")))
            .collect();
        let text = tables_text(&tables, 200, true);
        assert!(text.contains("300 tables and views in 2 schemas"), "{text}");
        assert!(text.contains("big: 290\nsmall: 10"), "{text}");
        let text = tables_text(&tables, 200, false);
        assert_eq!(text.lines().count(), 201);
        assert!(
            text.ends_with("(… and 100 more: narrow it down with `pattern`)"),
            "{text}"
        );
        assert_eq!(tables_text(&tables[..3], 200, true).lines().count(), 3);
        assert!(name_matches("ORDER", "sales_orders"));
        assert!(name_matches("order*", "Orders_2024"));
        assert!(!name_matches("order*", "sales_orders"));
        assert!(name_matches("%_items", "order_items"));
        assert!(name_matches("o*d*s", "orders"));
        assert!(!name_matches("o*d*s", "orderx"));
        assert!(name_matches("*", "x"));
    }

    #[test]
    fn results_stay_within_budget() {
        let wide = Rows {
            columns: (0..120).map(|i| format!("c{i}")).collect(),
            rows: (0..1000)
                .map(|_| (0..120).map(|_| json!("x".repeat(300))).collect())
                .collect(),
            truncated: false,
            messages: vec!["m".repeat(5000)],
        };
        let text = rows_text(&wide, 1000);
        assert!(text.len() <= RESULT_CHARS, "{}", text.len());
        assert!(
            text.contains("only the first 50 of 120 columns shown"),
            "{text}"
        );
        assert!(text.contains("(1000 rows; only the first"), "{text}");
        assert_eq!(
            text.lines().next().unwrap().len(),
            "Message: ".len() + MESSAGE_CHARS
        );
        let header = text.lines().nth(1).unwrap();
        assert_eq!(header.split(" | ").count(), 50);

        let long = (0..10_000)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let cut = capped(long);
        assert!(cut.len() < RESULT_CHARS + 200);
        assert!(
            cut.contains("more lines not shown"),
            "{}",
            &cut[cut.len() - 200..]
        );
        assert_eq!(capped("short".into()), "short");
    }

    #[tokio::test]
    async fn plans_are_shown_as_a_tree() {
        let replies = exchange(
            &[
                call(
                    1,
                    "explain_query",
                    json!({"sql": "SELECT count(*) FROM sales.orders"}),
                ),
                call(
                    2,
                    "explain_query",
                    json!({"sql": "EXPLAIN ANALYZE SELECT 1"}),
                ),
                call(
                    3,
                    "explain_query",
                    json!({"sql": "DELETE FROM sales.orders"}),
                ),
            ]
            .concat(),
        )
        .await;
        assert_eq!(
            text(&replies[0]),
            (
                "Estimated plan (not run):\n-> Aggregate (cost=13.00 rows=1)\n  \
                 -> Seq Scan on sales.orders (cost=12.50 rows=300)\n       \
                 Filter: SELECT count(*) FROM sales.orders\n",
                false
            )
        );
        let (err, is_err) = text(&replies[1]);
        assert!(is_err && err.contains("without EXPLAIN"), "{err}");
        let (err, is_err) = text(&replies[2]);
        assert!(is_err && err.starts_with("refused:"), "{err}");
    }
}
