//! `sqail mcp`: a Model Context Protocol server over stdio that lets an AI
//! CLI (Claude Code, Grok) explore one connection: list schemas and tables,
//! describe a table, and run read-only queries. The assistant panel starts
//! the CLI, which starts this server; the service URL, token and connection
//! arrive in the environment (see [`Env`]), never on the command line.

use std::future::Future;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use sqail_client::proto::{
    ColumnInfo, Engine, ForeignKeyInfo, IndexInfo, QueryEvent, QueryRequest, TableInfo, TableKind,
};
use sqail_client::{Client, Identity, On, Target, Trust};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Mutex;
use uuid::Uuid;

use super::guard::check_read_only;

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
/// Longest cell value shown to the model, in characters.
const CELL_CHARS: usize = 200;
const QUERY_TIMEOUT_MS: u64 = 30_000;

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
}

#[derive(Debug, Default)]
pub struct Rows {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub truncated: bool,
    pub messages: Vec<String>,
}

/// Serve MCP on `reader`/`writer` (stdin/stdout) until the input ends.
pub async fn serve<B: Backend>(
    backend: &B,
    max_rows: u64,
    reader: impl AsyncBufRead + Unpin,
    mut writer: impl AsyncWrite + Unpin,
) -> Result<()> {
    let mut lines = reader.lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(backend, max_rows, msg).await,
            Err(e) => Some(error_reply(
                Value::Null,
                -32700,
                &format!("parse error: {e}"),
            )),
        };
        if let Some(reply) = reply {
            let mut out = serde_json::to_vec(&reply)?;
            out.push(b'\n');
            writer.write_all(&out).await?;
            writer.flush().await?;
        }
    }
    Ok(())
}

/// One JSON-RPC message in, at most one reply out (none for notifications).
async fn handle<B: Backend>(backend: &B, max_rows: u64, msg: Value) -> Option<Value> {
    let id = msg.get("id").cloned()?;
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let result = match msg.get("method").and_then(Value::as_str).unwrap_or("") {
        "initialize" => json!({
            // Speak whatever version the client asked for; the subset used
            // here (tools with text content) is the same in all of them.
            "protocolVersion": params.get("protocolVersion").cloned().unwrap_or(json!("2025-06-18")),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "sqail", "version": env!("CARGO_PKG_VERSION") },
            "instructions": instructions(backend.engine()),
        }),
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tool_definitions(max_rows) }),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            let (text, is_error) = match call_tool(backend, max_rows, name, &args).await {
                Ok(text) => (text, false),
                Err(e) => (format!("{e:#}"), true),
            };
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
         number of rows; aggregate or filter instead of reading whole tables.",
        engine_name(engine)
    )
}

pub fn engine_name(engine: Engine) -> &'static str {
    match engine {
        Engine::Postgres => "PostgreSQL",
        Engine::Mssql => "SQL Server",
        Engine::Sqlite => "SQLite",
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
            "description": "List tables and views, as schema.name (kind). Without `schema`, lists every schema.",
            "inputSchema": {
                "type": "object",
                "properties": { "schema": { "type": "string", "description": "Only this schema" } },
            },
            "annotations": { "readOnlyHint": true },
        },
        {
            "name": "describe_table",
            "description": "Columns (type, nullability, primary key, default), indexes and foreign keys of one table or view.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "table": { "type": "string", "description": "Table or view name, optionally schema.name" },
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
                 proposing it."
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
            let tables = backend.tables(arg("schema")).await?;
            if tables.is_empty() {
                return Ok("(no tables)".into());
            }
            Ok(tables
                .iter()
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
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "describe_table" => {
            let table = arg("table").ok_or_else(|| anyhow!("`table` is required"))?;
            let (schema, table) = match (arg("schema"), table.split_once('.')) {
                (Some(s), _) => (Some(s), table),
                (None, Some((s, t))) => (Some(s), t),
                (None, None) => (None, table),
            };
            let (cols, idx, fks) = backend.describe(schema, table).await?;
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
        other => Err(anyhow!("unknown tool {other}")),
    }
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

/// Rows as a pipe-separated table, which models read well and cheaply.
fn rows_text(r: &Rows, max_rows: u64) -> String {
    let mut out = String::new();
    for m in &r.messages {
        out.push_str(&format!("Message: {m}\n"));
    }
    if r.columns.is_empty() {
        out.push_str("(no result set)");
        return out;
    }
    out.push_str(&r.columns.join(" | "));
    out.push('\n');
    for row in &r.rows {
        let cells: Vec<String> = row.iter().map(cell_text).collect();
        out.push_str(&cells.join(" | "));
        out.push('\n');
    }
    out.push_str(&format!(
        "({} row{}",
        r.rows.len(),
        if r.rows.len() == 1 { "" } else { "s" }
    ));
    if r.truncated {
        out.push_str(&format!(
            "; stopped at {max_rows}, more rows exist: aggregate, filter or add ORDER BY … LIMIT/TOP"
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
        })
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
        let session = On::Session(self.session().await?);
        let begin = match self.engine {
            Engine::Postgres => "BEGIN READ ONLY",
            Engine::Mssql => "BEGIN TRANSACTION",
            Engine::Sqlite => "BEGIN",
        };
        run_all(&self.client, session, begin, 1).await?;
        let mut req = QueryRequest::new(sql);
        req.max_rows = Some(max_rows);
        req.timeout_ms = Some(QUERY_TIMEOUT_MS);
        let result = self.client.query_all(session, &req).await;
        // Always undo, whatever the query did or however it failed.
        let rollback = run_all(&self.client, session, "ROLLBACK", 1).await;
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
        .unwrap_or(DEFAULT_ROWS)
        .clamp(1, 1000);
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
        async fn query(&self, sql: &str, max_rows: u64) -> Result<Rows> {
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

    async fn exchange(input: &str) -> Vec<Value> {
        let mut out = Vec::new();
        serve(&Fake, 50, input.as_bytes(), &mut out).await.unwrap();
        String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
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
        let names: Vec<&str> = replies[1]["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            ["list_schemas", "list_tables", "describe_table", "run_query"]
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
}
