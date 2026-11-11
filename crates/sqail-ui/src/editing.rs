//! Inline data editing: decide whether a result can be edited, track pending
//! changes, and turn them into parameterised DML applied in one transaction.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail};
use sqail_client::proto::{ColumnInfo, Engine, LogicalType, Param, QueryEvent, QueryRequest};
use sqail_client::{Client, On};
use uuid::Uuid;

use crate::results::{Cell, ResultSet};
use crate::sql::lex::{Kind, Token, tokenize};
use crate::sql::quote_ident;

/// One item of a `SELECT` list, as far as editing cares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectItem {
    /// `*` or `alias.*`: every table column, in table order.
    Star,
    /// A bare table column (`col`, `alias.col`) without `AS`, so the result
    /// column really is that table column.
    Column(String),
    /// Anything else (`x AS id`, literals, expressions): read-only.
    Other,
}

/// What an editable `SELECT` reads: its single table and its select list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub schema: Option<String>,
    pub table: String,
    pub items: Vec<SelectItem>,
}

/// The single table a `SELECT` reads from, if it is simple enough to edit.
pub fn source_table(sql: &str, engine: Engine) -> Option<Source> {
    let toks: Vec<_> = tokenize(sql, Some(engine))
        .into_iter()
        .filter(|t| !matches!(t.kind, Kind::Space | Kind::Comment))
        .collect();
    let word = |i: usize| {
        toks.get(i)
            .map(|t| sql[t.range.clone()].to_ascii_uppercase())
    };
    if word(0).as_deref() != Some("SELECT") {
        return None;
    }
    let words: Vec<String> = toks
        .iter()
        .filter(|t| t.kind == Kind::Word)
        .map(|t| sql[t.range.clone()].to_ascii_uppercase())
        .collect();
    let count = |w: &str| words.iter().filter(|x| *x == w).count();
    let blockers = [
        "JOIN",
        "GROUP",
        "UNION",
        "INTERSECT",
        "EXCEPT",
        "DISTINCT",
        "HAVING",
        "INTO",
        "OVER",
        "APPLY",
        "PIVOT",
        "UNPIVOT",
    ];
    if count("SELECT") != 1 || count("FROM") != 1 || blockers.iter().any(|b| count(b) > 0) {
        return None;
    }
    let from = toks
        .iter()
        .position(|t| t.kind == Kind::Word && sql[t.range.clone()].eq_ignore_ascii_case("FROM"))?;
    // PostgreSQL folds unquoted names to lower case: `FROM Customers` reads
    // `customers`. Quoted names keep their case.
    let name = |i: usize| {
        toks.get(i).and_then(|t| match t.kind {
            Kind::Word if engine == Engine::Postgres => {
                Some(sql[t.range.clone()].to_ascii_lowercase())
            }
            Kind::Word | Kind::QuotedIdent => Some(unquote(&sql[t.range.clone()])),
            _ => None,
        })
    };
    let first = name(from + 1)?;
    let dot = toks
        .get(from + 2)
        .is_some_and(|t| &sql[t.range.clone()] == ".");
    let (schema, table, after) = if dot {
        (Some(first), name(from + 3)?, from + 4)
    } else {
        (None, first, from + 2)
    };
    // A comma after the table means several tables (old-style join).
    let mut i = after;
    while let Some(t) = toks.get(i) {
        let s = &sql[t.range.clone()];
        if s == "," {
            return None;
        }
        if t.kind == Kind::Word
            && ["WHERE", "ORDER", "LIMIT", "OFFSET", "FETCH"]
                .contains(&s.to_ascii_uppercase().as_str())
        {
            break;
        }
        i += 1;
    }
    let items = select_items(sql, &toks[1..from]);
    Some(Source {
        schema,
        table,
        items,
    })
}

/// Split the tokens between `SELECT` and `FROM` into items.
fn select_items(sql: &str, toks: &[Token]) -> Vec<SelectItem> {
    let is_word =
        |t: &Token, w: &str| t.kind == Kind::Word && sql[t.range.clone()].eq_ignore_ascii_case(w);
    // Skip `ALL` and T-SQL `TOP n | TOP (n) [PERCENT] [WITH TIES]`.
    let mut i = 0;
    if toks.first().is_some_and(|t| is_word(t, "ALL")) {
        i += 1;
    }
    if toks.get(i).is_some_and(|t| is_word(t, "TOP")) {
        i += 1;
        if toks.get(i).is_some_and(|t| t.kind == Kind::Open) {
            let mut depth = 0;
            while let Some(t) = toks.get(i) {
                i += 1;
                match t.kind {
                    Kind::Open => depth += 1,
                    Kind::Close => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        } else {
            i += 1;
        }
        if toks.get(i).is_some_and(|t| is_word(t, "PERCENT")) {
            i += 1;
        }
        if toks.get(i).is_some_and(|t| is_word(t, "WITH"))
            && toks.get(i + 1).is_some_and(|t| is_word(t, "TIES"))
        {
            i += 2;
        }
    }
    let mut items = Vec::new();
    let mut depth = 0i32;
    let mut item: Vec<&Token> = Vec::new();
    for t in toks.get(i..).unwrap_or_default() {
        match t.kind {
            Kind::Open => depth += 1,
            Kind::Close => depth -= 1,
            Kind::Punct if depth == 0 && &sql[t.range.clone()] == "," => {
                items.push(classify(sql, &item));
                item.clear();
                continue;
            }
            _ => {}
        }
        item.push(t);
    }
    items.push(classify(sql, &item));
    items
}

/// `name`, `q.name`, `s.q.name` → a column; the same ending in `*` → star;
/// anything else (aliases, literals, expressions) → other.
fn classify(sql: &str, item: &[&Token]) -> SelectItem {
    let text = |t: &Token| &sql[t.range.clone()];
    let is_name = |t: &Token| match t.kind {
        Kind::QuotedIdent => true,
        // Keywords that look like names but are values.
        Kind::Word => !matches!(
            text(t).to_ascii_uppercase().as_str(),
            "NULL"
                | "TRUE"
                | "FALSE"
                | "DEFAULT"
                | "CURRENT_DATE"
                | "CURRENT_TIME"
                | "CURRENT_TIMESTAMP"
                | "CURRENT_USER"
                | "SESSION_USER"
                | "USER"
                | "LOCALTIME"
                | "LOCALTIMESTAMP"
        ),
        _ => false,
    };
    if item.len().is_multiple_of(2) || item.len() > 5 {
        return SelectItem::Other;
    }
    let dots_ok = item
        .iter()
        .skip(1)
        .step_by(2)
        .all(|t| t.kind == Kind::Punct && text(t) == ".");
    let parts: Vec<&Token> = item.iter().step_by(2).copied().collect();
    let (last, quals) = parts.split_last().expect("odd length, so not empty");
    if !dots_ok || !quals.iter().all(|q| is_name(q)) {
        SelectItem::Other
    } else if last.kind == Kind::Punct && text(last) == "*" {
        SelectItem::Star
    } else if is_name(last) {
        SelectItem::Column(unquote(text(last)))
    } else {
        SelectItem::Other
    }
}

fn unquote(s: &str) -> String {
    let t = s.trim();
    match (t.chars().next(), t.chars().last()) {
        (Some('"'), Some('"')) | (Some('['), Some(']')) | (Some('`'), Some('`'))
            if t.len() >= 2 =>
        {
            t[1..t.len() - 1].to_string()
        }
        _ => t.to_string(),
    }
}

/// The table column behind each result column, following the select list.
/// `None` when the result doesn't line up with the select list.
fn map_columns(
    items: &[SelectItem],
    result_columns: &[sqail_client::proto::Column],
    table_columns: &[ColumnInfo],
) -> Option<Vec<Option<ColumnInfo>>> {
    let mut ordered: Vec<&ColumnInfo> = table_columns.iter().collect();
    ordered.sort_by_key(|c| c.ordinal);
    let mut out = Vec::with_capacity(result_columns.len());
    let mut rcols = result_columns.iter();
    for item in items {
        match item {
            SelectItem::Star => {
                for t in &ordered {
                    // `*` must expand to the table's columns, in order.
                    if !rcols.next()?.name.eq_ignore_ascii_case(&t.name) {
                        return None;
                    }
                    out.push(Some((*t).clone()));
                }
            }
            SelectItem::Column(name) => {
                let r = rcols.next()?;
                let t = table_columns.iter().find(|t| {
                    t.name.eq_ignore_ascii_case(name) && t.name.eq_ignore_ascii_case(&r.name)
                });
                out.push(t.cloned());
            }
            SelectItem::Other => {
                rcols.next()?;
                out.push(None);
            }
        }
    }
    rcols.next().is_none().then_some(out)
}

/// Whether a table column type, as the column list reports it, holds bytes.
pub fn is_binary_type(engine: Engine, data_type: &str) -> bool {
    let t = data_type.trim().to_ascii_lowercase();
    let base = t.split('(').next().unwrap_or("").trim();
    match engine {
        Engine::Postgres => base == "bytea",
        Engine::Mssql => matches!(base, "binary" | "varbinary" | "image"),
        Engine::Sqlite => t.contains("blob"),
        Engine::Unknown => false,
    }
}

/// A binary value as the grid and CSV export show it (`0x` + hex; psql's
/// `\x` prefix is accepted too) → lowercase hex digits without the prefix.
pub fn parse_hex(s: &str) -> std::result::Result<String, String> {
    let t = s.trim();
    let bad = |why: &str| {
        let shown: String = t.chars().take(40).collect();
        let more = if t.chars().count() > 40 { "…" } else { "" };
        format!(
            "'{shown}{more}' is not a binary value ({why}); write 0x followed by hex digits, e.g. 0xdeadbeef"
        )
    };
    let Some(hex) = ["0x", "0X", "\\x"].iter().find_map(|p| t.strip_prefix(p)) else {
        return Err(bad("no 0x prefix"));
    };
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(bad("not a hex digit"));
    }
    if hex.len() % 2 != 0 {
        return Err(bad("odd number of hex digits"));
    }
    Ok(hex.to_ascii_lowercase())
}

/// The parameter that carries the bytes `hex` (bare hex digits) to a
/// [`binary_placeholder`].
pub fn binary_param(engine: Engine, hex: &str) -> Param {
    match engine {
        // CONVERT style 1 wants the 0x prefix.
        Engine::Mssql => Param::Text(format!("0x{hex}")),
        _ => Param::Text(hex.to_string()),
    }
}

/// Placeholder `n` for a binary column: the hex text is decoded to bytes by
/// the database, so the column gets the real bytes, not the ASCII of `0x…`.
pub fn binary_placeholder(engine: Engine, n: usize) -> String {
    match engine {
        Engine::Postgres => format!("decode(${n}::text, 'hex')"),
        Engine::Mssql => format!("CONVERT(varbinary(max), @P{n}, 1)"),
        Engine::Sqlite | Engine::Unknown => format!("unhex(?{n})"),
    }
}

/// A new value for a cell: `None` = NULL.
pub type Value = Option<String>;

/// Pending edits on one result set.
pub struct EditState {
    pub schema: Option<String>,
    pub table: String,
    pub engine: Engine,
    /// Table column info per *result* column (None = not a table column, so
    /// read-only, e.g. an expression).
    pub target: Vec<Option<ColumnInfo>>,
    /// Result column indices of the primary key.
    pub pk: Vec<usize>,
    /// row index (in `ResultSet::rows`) → column → new value.
    pub changes: BTreeMap<usize, BTreeMap<usize, Value>>,
    pub deleted: BTreeSet<usize>,
    /// New rows: per result column, `None` = use the default (omit).
    pub inserted: Vec<Vec<Option<Value>>>,
    /// Cell being edited: (display position or `rows + i` for new rows, column, text).
    pub editing: Option<(usize, usize, String)>,
    /// Give the inline editor keyboard focus on its next frame.
    pub focus_editor: bool,
    /// The text in the inline editor was changed. An editor left unchanged
    /// records nothing, so opening a NULL cell (shown empty) and clicking
    /// away doesn't turn it into `''`.
    pub edit_dirty: bool,
}

impl EditState {
    /// Match result columns to the table through the select list. Only bare
    /// columns and `*` are editable; aliased columns, literals and
    /// expressions are read-only. Fails unless the whole primary key is in
    /// the result exactly once.
    pub fn new(
        engine: Engine,
        source: Source,
        result_columns: &[sqail_client::proto::Column],
        table_columns: &[ColumnInfo],
    ) -> Result<Self> {
        let Source {
            schema,
            table,
            items,
        } = source;
        let Some(mut target) = map_columns(&items, result_columns, table_columns) else {
            bail!("the result columns don't line up with the query's select list");
        };
        let pk_cols: Vec<&ColumnInfo> = table_columns.iter().filter(|c| c.primary_key).collect();
        if pk_cols.is_empty() {
            bail!("{table} has no primary key, so rows cannot be identified");
        }
        let mut pk = Vec::new();
        for p in &pk_cols {
            let mut hits = target
                .iter()
                .enumerate()
                .filter(|(_, t)| t.as_ref().is_some_and(|t| t.name == p.name))
                .map(|(i, _)| i);
            match (hits.next(), hits.next()) {
                (Some(i), None) => pk.push(i),
                (Some(_), Some(_)) => bail!(
                    "the primary key column {} is selected more than once",
                    p.name
                ),
                (None, _) => bail!(
                    "include the primary key column {} in the query, without an alias, to edit",
                    p.name
                ),
            }
        }
        // A column selected twice: only the first copy is editable, so two
        // cells never write the same column.
        let mut seen = BTreeSet::new();
        for t in &mut target {
            if let Some(c) = t
                && !seen.insert(c.name.clone())
            {
                *t = None;
            }
        }
        Ok(Self {
            schema,
            table,
            engine,
            target,
            pk,
            changes: BTreeMap::new(),
            deleted: BTreeSet::new(),
            inserted: Vec::new(),
            editing: None,
            focus_editor: false,
            edit_dirty: false,
        })
    }

    pub fn editable(&self, col: usize) -> bool {
        self.target.get(col).is_some_and(Option::is_some)
    }

    pub fn pending(&self) -> usize {
        self.changes.len() + self.deleted.len() + self.inserted.len()
    }

    pub fn set(&mut self, row: usize, col: usize, value: Value, rs: &ResultSet) {
        // Setting a cell back to its original value drops the change.
        let original = &rs.raw_row(row)[col];
        let same = match (&value, original) {
            (None, Cell::Null) => true,
            (Some(v), c) if !c.is_null() => *v == c.display(rs.columns[col].logical),
            _ => false,
        };
        let entry = self.changes.entry(row).or_default();
        if same {
            entry.remove(&col);
        } else {
            entry.insert(col, value);
        }
        if entry.is_empty() {
            self.changes.remove(&row);
        }
    }

    fn table_sql(&self) -> String {
        match self.schema.as_deref() {
            Some(s) if !(self.engine == Engine::Sqlite && s == "main") => {
                format!(
                    "{}.{}",
                    quote_ident(self.engine, s),
                    quote_ident(self.engine, &self.table)
                )
            }
            _ => quote_ident(self.engine, &self.table),
        }
    }

    /// Placeholder `n` for result column `c`. Binary columns (shown as
    /// `0x…`) are bound as hex and decoded by the database.
    fn ph(&self, rs: &ResultSet, n: usize, c: usize) -> String {
        if is_binary(rs, c) {
            return binary_placeholder(self.engine, n);
        }
        match self.engine {
            Engine::Postgres => format!("${n}::text::{}", self.col(c).data_type),
            Engine::Mssql => format!("@P{n}"),
            Engine::Sqlite | Engine::Unknown => format!("?{n}"),
        }
    }

    fn col(&self, i: usize) -> &ColumnInfo {
        self.target[i].as_ref().expect("editable column")
    }

    /// The parameter for a new value of result column `c`; binary values
    /// must be valid `0x` hex.
    fn value(&self, rs: &ResultSet, c: usize, v: &Value) -> Result<Param> {
        Ok(match v {
            None => Param::Null,
            Some(t) if is_binary(rs, c) => {
                let hex = parse_hex(t).map_err(|e| anyhow::anyhow!("{}: {e}", self.col(c).name))?;
                binary_param(self.engine, &hex)
            }
            Some(t) => Param::Text(t.clone()),
        })
    }

    /// WHERE clause matching the original primary key of `row`.
    fn key(&self, rs: &ResultSet, row: usize, n: &mut usize, params: &mut Vec<Param>) -> String {
        self.pk
            .iter()
            .map(|&c| {
                *n += 1;
                let col = self.col(c);
                let cell = &rs.raw_row(row)[c];
                params.push(match cell {
                    // Binary cells hold the bytes as bare hex already.
                    Cell::Text(hex) if is_binary(rs, c) => binary_param(self.engine, hex),
                    _ => Param::Text(cell.display(rs.columns[c].logical).into_owned()),
                });
                format!(
                    "{} = {}",
                    quote_ident(self.engine, &col.name),
                    self.ph(rs, *n, c)
                )
            })
            .collect::<Vec<_>>()
            .join(" AND ")
    }

    /// The statements to run, in order: deletes, updates, inserts. Fails,
    /// before anything is sent, on a value a column cannot take (a binary
    /// value that is not `0x` hex).
    pub fn statements(&self, rs: &ResultSet) -> Result<Vec<(String, Vec<Param>)>> {
        let table = self.table_sql();
        let mut out = Vec::new();
        for &row in &self.deleted {
            let (mut n, mut params) = (0, Vec::new());
            let key = self.key(rs, row, &mut n, &mut params);
            out.push((format!("DELETE FROM {table} WHERE {key}"), params));
        }
        for (&row, cols) in &self.changes {
            if self.deleted.contains(&row) || cols.is_empty() {
                continue;
            }
            let (mut n, mut params) = (0, Vec::new());
            let mut sets = Vec::new();
            for (&c, v) in cols {
                n += 1;
                params.push(self.value(rs, c, v)?);
                sets.push(format!(
                    "{} = {}",
                    quote_ident(self.engine, &self.col(c).name),
                    self.ph(rs, n, c)
                ));
            }
            let key = self.key(rs, row, &mut n, &mut params);
            out.push((
                format!("UPDATE {table} SET {} WHERE {key}", sets.join(", ")),
                params,
            ));
        }
        for row in &self.inserted {
            let (mut n, mut params, mut names, mut phs) = (0, Vec::new(), Vec::new(), Vec::new());
            for (c, v) in row.iter().enumerate() {
                if let (Some(v), true) = (v, self.editable(c)) {
                    n += 1;
                    params.push(self.value(rs, c, v)?);
                    names.push(quote_ident(self.engine, &self.col(c).name));
                    phs.push(self.ph(rs, n, c));
                }
            }
            let sql = if names.is_empty() {
                format!("INSERT INTO {table} DEFAULT VALUES")
            } else {
                format!(
                    "INSERT INTO {table} ({}) VALUES ({})",
                    names.join(", "),
                    phs.join(", ")
                )
            };
            out.push((sql, params));
        }
        Ok(out)
    }
}

/// Result column `c` holds bytes, shown as `0x` + hex.
fn is_binary(rs: &ResultSet, c: usize) -> bool {
    rs.columns[c].logical == LogicalType::Bytes
}

/// Show a statement with its parameters inlined, for the review dialog.
pub fn preview(engine: Engine, sql: &str, params: &[Param]) -> String {
    let mut s = sql.to_string();
    // Replace from the highest number down so $1 does not clobber $10.
    for (i, p) in params.iter().enumerate().rev() {
        let n = i + 1;
        let lit = match p {
            Param::Null => "NULL".to_string(),
            Param::Text(t) => {
                let q = format!("'{}'", t.replace('\'', "''"));
                if engine == Engine::Mssql {
                    format!("N{q}")
                } else {
                    q
                }
            }
            Param::Int(v) => v.to_string(),
            Param::Float(v) => v.to_string(),
            Param::Bool(v) => v.to_string(),
        };
        if engine == Engine::Postgres {
            // `decode($n::text, 'hex')` → `decode('value', 'hex')`
            s = s.replace(&format!("decode(${n}::text,"), &format!("decode({lit},"));
        }
        let (pat, rep) = match engine {
            // `$n::text::type` → `'value'::type`
            Engine::Postgres => (format!("${n}::text::"), format!("{lit}::")),
            Engine::Mssql => (format!("@P{n}"), lit),
            Engine::Sqlite | Engine::Unknown => (format!("?{n}"), lit),
        };
        s = s.replace(&pat, &rep);
    }
    s
}

/// How edits relate to the transaction of the tab's session. Either way they
/// apply atomically: all of them or none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditTx {
    /// Auto-commit tab, no transaction open: a transaction of their own that
    /// is committed.
    Commit,
    /// Manual-commit tab, no transaction open: open one and leave it open.
    Begin,
    /// A transaction is open: join it under a savepoint, uncommitted.
    Join,
}

impl EditTx {
    pub fn for_tab(autocommit: bool, in_transaction: bool) -> Self {
        match (in_transaction, autocommit) {
            (true, _) => EditTx::Join,
            (false, true) => EditTx::Commit,
            (false, false) => EditTx::Begin,
        }
    }
}

/// Savepoint that makes edits inside an open transaction all-or-nothing.
const SAVEPOINT: &str = "sqail_edit";

/// Apply statements atomically on `session`, the tab's own session, so they
/// hit the table the result came from (its search path, database and
/// temporary tables) and see its uncommitted rows.
pub async fn apply(
    client: &Client,
    session: Uuid,
    engine: Engine,
    stmts: Vec<(String, Vec<Param>)>,
    tx: EditTx,
) -> Result<usize> {
    let run = |sql: String, params: Vec<Param>| async move {
        let events = client
            .query_all(
                On::Session(session),
                &QueryRequest {
                    sql,
                    params,
                    max_rows: Some(1),
                    timeout_ms: None,
                    single_statement: false,
                },
            )
            .await?;
        let mut affected = 0u64;
        for e in events {
            match e {
                QueryEvent::Error { message, .. } => bail!(message),
                QueryEvent::RowsAffected { count } => affected += count,
                _ => {}
            }
        }
        Ok::<u64, anyhow::Error>(affected)
    };
    let mssql = engine == Engine::Mssql;
    let start = match (tx, mssql) {
        (EditTx::Join, true) => format!("SAVE TRANSACTION {SAVEPOINT}"),
        (EditTx::Join, false) => format!("SAVEPOINT {SAVEPOINT}"),
        (_, true) => "BEGIN TRANSACTION".into(),
        (_, false) => "BEGIN".into(),
    };
    let result = async {
        run(start, vec![]).await?;
        let n = stmts.len();
        for (i, (sql, params)) in stmts.into_iter().enumerate() {
            let affected = run(sql.clone(), params)
                .await
                .map_err(|e| anyhow::anyhow!("statement {} of {n}: {e}", i + 1))?;
            // Guard against keys that no longer match (row changed or gone).
            if affected == 0 && !sql.starts_with("INSERT") {
                bail!(
                    "statement {} of {n} matched no row; the data changed since it was loaded",
                    i + 1
                );
            }
        }
        match tx {
            EditTx::Commit => {
                run("COMMIT".into(), vec![]).await?;
            }
            // SQL Server savepoints need no release.
            EditTx::Join if !mssql => {
                run(format!("RELEASE SAVEPOINT {SAVEPOINT}"), vec![]).await?;
            }
            EditTx::Join | EditTx::Begin => {}
        }
        Ok::<usize, anyhow::Error>(n)
    }
    .await;
    if result.is_err() {
        let undo = match (tx, mssql) {
            // Back to before the edits; the rest of the transaction stays.
            // An error that doomed or ended the transaction leaves nothing to
            // go back to.
            (EditTx::Join, true) => {
                format!("IF XACT_STATE() = 1 ROLLBACK TRANSACTION {SAVEPOINT}")
            }
            (EditTx::Join, false) => {
                format!("ROLLBACK TO SAVEPOINT {SAVEPOINT}; RELEASE SAVEPOINT {SAVEPOINT}")
            }
            _ => "ROLLBACK".into(),
        };
        let _ = run(undo, vec![]).await;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqail_client::proto::{Column, LogicalType};

    #[test]
    fn simple_selects_are_editable() {
        let src = |s| source_table(s, Engine::Postgres).map(|s| (s.schema, s.table));
        assert_eq!(
            src("SELECT * FROM sales.products WHERE id < 10 ORDER BY id"),
            Some((Some("sales".into()), "products".into()))
        );
        assert_eq!(
            src("select id, name from \"People\" p limit 5"),
            Some((None, "People".into()))
        );
        // Unquoted names fold to lower case on PostgreSQL (#164); quoted
        // ones keep their case.
        assert_eq!(
            src("SELECT * FROM Sales.Customers"),
            Some((Some("sales".into()), "customers".into()))
        );
        assert_eq!(
            src("SELECT * FROM \"Sales\".Customers"),
            Some((Some("Sales".into()), "customers".into()))
        );
        assert_eq!(
            source_table("SELECT * FROM Customers", Engine::Mssql).map(|s| s.table),
            Some("Customers".into()),
            "SQL Server keeps the case"
        );
        assert_eq!(src("SELECT * FROM a JOIN b ON a.id = b.id"), None);
        assert_eq!(src("SELECT * FROM a, b"), None);
        assert_eq!(src("SELECT count(*) FROM a GROUP BY x"), None);
        assert_eq!(src("SELECT * FROM a WHERE id IN (SELECT id FROM b)"), None);
        assert_eq!(src("UPDATE a SET x = 1"), None);
        assert_eq!(
            source_table("SELECT TOP (10) * FROM [sales].[products]", Engine::Mssql),
            Some(Source {
                schema: Some("sales".into()),
                table: "products".into(),
                items: vec![SelectItem::Star],
            })
        );
        let mssql = |s| source_table(s, Engine::Mssql).map(|s| (s.schema, s.table));
        assert_eq!(mssql("SELECT * FROM a CROSS APPLY f(a.id)"), None);
        assert_eq!(mssql("SELECT * FROM a OUTER APPLY f(a.id)"), None);
        assert_eq!(
            mssql("SELECT * FROM a PIVOT (sum(x) FOR y IN ([1])) p"),
            None
        );
        assert_eq!(mssql("SELECT * FROM a UNPIVOT (x FOR y IN (b, c)) u"), None);
    }

    #[test]
    fn select_list_items() {
        use SelectItem::{Column as C, Other as O, Star as S};
        let items = |s, e| source_table(s, e).unwrap().items;
        let pg = Engine::Postgres;
        assert_eq!(
            items(
                "SELECT p.*, id, p.name, \"Mixed\", other_id AS id, other_id id, 'x' AS status, \
                 a + b, coalesce(a, b), NULL, 42 FROM s.t p",
                pg
            ),
            vec![
                S,
                C("id".into()),
                C("name".into()),
                C("Mixed".into()),
                O,
                O,
                O,
                O,
                O,
                O,
                O
            ]
        );
        assert_eq!(items("SELECT s.t.id FROM s.t", pg), vec![C("id".into())]);
        let ms = Engine::Mssql;
        assert_eq!(
            items("SELECT TOP 5 PERCENT WITH TIES [id], x = id FROM t", ms),
            vec![C("id".into()), O]
        );
        assert_eq!(items("SELECT ALL id FROM t", ms), vec![C("id".into())]);
    }

    fn info(name: &str, ordinal: i64, pk: bool) -> ColumnInfo {
        ColumnInfo {
            name: name.into(),
            ordinal,
            data_type: "int".into(),
            nullable: !pk,
            default: None,
            primary_key: pk,
        }
    }

    fn rcol(name: &str) -> Column {
        Column {
            name: name.into(),
            type_name: "int4".into(),
            logical: LogicalType::Int,
        }
    }

    /// Build an edit state for `sql` against table t(id pk, other_id, status, name).
    fn edit_for(sql: &str, result: &[&str]) -> Result<EditState> {
        let tcols = [
            info("id", 1, true),
            info("other_id", 2, false),
            info("status", 3, false),
            info("name", 4, false),
        ];
        let rcols: Vec<Column> = result.iter().map(|n| rcol(n)).collect();
        let source = source_table(sql, Engine::Postgres).expect("editable query");
        EditState::new(Engine::Postgres, source, &rcols, &tcols)
    }

    #[test]
    fn aliased_column_is_not_the_primary_key() {
        // Mapping by result name would use other_id as the key (#67).
        let err = edit_for("SELECT other_id AS id, name FROM t", &["id", "name"])
            .err()
            .expect("must refuse");
        assert!(err.to_string().contains("primary key"), "{err}");
        // Aliasing the key itself is refused as well.
        assert!(edit_for("SELECT id AS key, name FROM t", &["key", "name"]).is_err());
        // An alias that happens to be a table column name stays read-only.
        let e = edit_for("SELECT id, other_id AS name FROM t", &["id", "name"]).unwrap();
        assert!(e.editable(0));
        assert!(!e.editable(1));
    }

    #[test]
    fn literals_are_read_only() {
        let e = edit_for("SELECT 'x' AS status, id FROM t", &["status", "id"]).unwrap();
        assert!(!e.editable(0));
        assert_eq!(e.pk, vec![1]);
        let e = edit_for("SELECT NULL AS name, id FROM t", &["name", "id"]).unwrap();
        assert!(!e.editable(0));
    }

    #[test]
    fn duplicate_columns() {
        let err = edit_for("SELECT id, id FROM t", &["id", "id"])
            .err()
            .unwrap();
        assert!(err.to_string().contains("more than once"), "{err}");
        // A duplicated non-key column: only the first copy is editable.
        let e = edit_for("SELECT id, name, name FROM t", &["id", "name", "name"]).unwrap();
        assert!(e.editable(1));
        assert!(!e.editable(2));
        // `*` plus the key again duplicates the key.
        let all = ["id", "other_id", "status", "name"];
        assert!(edit_for("SELECT *, id FROM t", &[&all[..], &["id"]].concat()).is_err());
    }

    #[test]
    fn star_expands_in_table_order() {
        let all = ["id", "other_id", "status", "name"];
        let e = edit_for("SELECT * FROM t", &all).unwrap();
        assert!((0..4).all(|c| e.editable(c)));
        assert_eq!(e.pk, vec![0]);
        let e = edit_for(
            "SELECT 1 AS n, t.* FROM t",
            &[&["n"][..], &all[..]].concat(),
        )
        .unwrap();
        assert!(!e.editable(0));
        assert_eq!(e.pk, vec![1]);
        assert_eq!(e.target[4].as_ref().unwrap().name, "name");
        // A result that doesn't line up (stale table info) is refused.
        assert!(edit_for("SELECT * FROM t", &["id", "name"]).is_err());
        assert!(edit_for("SELECT * FROM t", &["other_id", "id", "status", "name"]).is_err());
    }

    fn setup(engine: Engine) -> (EditState, ResultSet) {
        let col = |n: &str, t: &str, pk: bool| ColumnInfo {
            name: n.into(),
            ordinal: 0,
            data_type: t.into(),
            nullable: !pk,
            default: None,
            primary_key: pk,
        };
        let rcols = vec![
            Column {
                name: "id".into(),
                type_name: "int4".into(),
                logical: LogicalType::Int,
            },
            Column {
                name: "price".into(),
                type_name: "numeric".into(),
                logical: LogicalType::Decimal,
            },
            Column {
                name: "label".into(),
                type_name: "text".into(),
                logical: LogicalType::Text,
            },
        ];
        let tcols = vec![
            col("id", "integer", true),
            col("price", "numeric(10,2)", false),
        ];
        let source = source_table("SELECT id, price, label FROM sales.products", engine).unwrap();
        let e = EditState::new(engine, source, &rcols, &tcols).unwrap();
        let rs = ResultSet::for_test(
            rcols,
            vec![vec![
                Cell::Int(1),
                Cell::Text("4.11".into()),
                Cell::Text("x".into()),
            ]],
        );
        (e, rs)
    }

    #[test]
    fn expressions_are_read_only_and_pk_is_required() {
        let (e, _) = setup(Engine::Postgres);
        assert!(e.editable(1));
        assert!(!e.editable(2), "label is not a table column");
        let rcols = vec![Column {
            name: "price".into(),
            type_name: "n".into(),
            logical: LogicalType::Decimal,
        }];
        let tcols = vec![ColumnInfo {
            name: "id".into(),
            ordinal: 1,
            data_type: "int".into(),
            nullable: false,
            default: None,
            primary_key: true,
        }];
        let source = source_table("SELECT price FROM t", Engine::Postgres).unwrap();
        let err = EditState::new(Engine::Postgres, source, &rcols, &tcols)
            .err()
            .unwrap();
        assert!(err.to_string().contains("primary key"));
    }

    #[test]
    fn statements_per_dialect() {
        let (mut e, rs) = setup(Engine::Postgres);
        e.set(0, 1, Some("5.00".into()), &rs);
        e.inserted
            .push(vec![Some(Some("99".into())), Some(None), None]);
        let st = e.statements(&rs).unwrap();
        assert_eq!(
            st[0].0,
            "UPDATE sales.products SET price = $1::text::numeric(10,2) WHERE id = $2::text::integer"
        );
        assert_eq!(
            st[0].1,
            vec![Param::Text("5.00".into()), Param::Text("1".into())]
        );
        assert_eq!(
            st[1].0,
            "INSERT INTO sales.products (id, price) VALUES ($1::text::integer, $2::text::numeric(10,2))"
        );
        assert_eq!(st[1].1, vec![Param::Text("99".into()), Param::Null]);
        assert_eq!(
            preview(Engine::Postgres, &st[0].0, &st[0].1),
            "UPDATE sales.products SET price = '5.00'::numeric(10,2) WHERE id = '1'::integer"
        );

        let (mut e, rs) = setup(Engine::Mssql);
        e.deleted.insert(0);
        e.set(0, 1, Some("1".into()), &rs); // ignored: the row is deleted
        let st = e.statements(&rs).unwrap();
        assert_eq!(st.len(), 1);
        assert_eq!(st[0].0, "DELETE FROM [sales].[products] WHERE [id] = @P1");
        assert_eq!(
            preview(Engine::Mssql, &st[0].0, &st[0].1),
            "DELETE FROM [sales].[products] WHERE [id] = N'1'"
        );
    }

    #[test]
    fn setting_the_original_value_drops_the_change() {
        let (mut e, rs) = setup(Engine::Sqlite);
        e.set(0, 1, Some("9".into()), &rs);
        assert_eq!(e.pending(), 1);
        e.set(0, 1, Some("4.11".into()), &rs);
        assert_eq!(e.pending(), 0);
    }

    #[test]
    fn hex_values_parse_strictly() {
        assert_eq!(parse_hex("0xDEADbeef").unwrap(), "deadbeef");
        assert_eq!(parse_hex(" 0x ").unwrap(), "", "0x alone is empty");
        assert_eq!(parse_hex("\\x0a").unwrap(), "0a");
        assert!(parse_hex("deadbeef").unwrap_err().contains("no 0x prefix"));
        assert!(parse_hex("0xabc").unwrap_err().contains("odd number"));
        assert!(parse_hex("0xzz").unwrap_err().contains("hex digit"));
        assert!(parse_hex("").is_err());
        assert!(is_binary_type(Engine::Postgres, "bytea"));
        assert!(!is_binary_type(Engine::Postgres, "bytea[]"));
        assert!(is_binary_type(Engine::Mssql, "varbinary(max)"));
        assert!(is_binary_type(Engine::Mssql, "binary(4)"));
        assert!(!is_binary_type(Engine::Mssql, "nvarchar(10)"));
        assert!(is_binary_type(Engine::Sqlite, "BLOB"));
        assert!(!is_binary_type(Engine::Sqlite, "TEXT"));
    }

    /// A table keyed by a binary column with a binary value column.
    fn binary_setup(engine: Engine) -> (EditState, ResultSet) {
        let rcols: Vec<Column> = ["k", "b"]
            .iter()
            .map(|n| Column {
                name: (*n).into(),
                type_name: "bytea".into(),
                logical: LogicalType::Bytes,
            })
            .collect();
        let tcols: Vec<ColumnInfo> = ["k", "b"]
            .iter()
            .enumerate()
            .map(|(i, n)| ColumnInfo {
                name: (*n).into(),
                ordinal: i as i64 + 1,
                data_type: "bytea".into(),
                nullable: i == 1,
                default: None,
                primary_key: i == 0,
            })
            .collect();
        let source = source_table("SELECT k, b FROM t", engine).unwrap();
        let e = EditState::new(engine, source, &rcols, &tcols).unwrap();
        let rs = ResultSet::for_test(
            rcols,
            vec![vec![
                Cell::Text("0102".into()),
                Cell::Text("deadbeef".into()),
            ]],
        );
        (e, rs)
    }

    #[test]
    fn binary_columns_bind_bytes() {
        let (mut e, rs) = binary_setup(Engine::Postgres);
        e.set(0, 1, Some("0xDEADBEEE".into()), &rs);
        e.inserted
            .push(vec![Some(Some("0xff".into())), Some(Some("0x".into()))]);
        let st = e.statements(&rs).unwrap();
        assert_eq!(
            st[0].0,
            "UPDATE t SET b = decode($1::text, 'hex') WHERE k = decode($2::text, 'hex')"
        );
        assert_eq!(
            st[0].1,
            vec![Param::Text("deadbeee".into()), Param::Text("0102".into())]
        );
        assert_eq!(
            preview(Engine::Postgres, &st[0].0, &st[0].1),
            "UPDATE t SET b = decode('deadbeee', 'hex') WHERE k = decode('0102', 'hex')"
        );
        assert_eq!(
            st[1].0,
            "INSERT INTO t (k, b) VALUES (decode($1::text, 'hex'), decode($2::text, 'hex'))"
        );
        assert_eq!(
            st[1].1,
            vec![Param::Text("ff".into()), Param::Text("".into())]
        );

        let (mut e, rs) = binary_setup(Engine::Mssql);
        e.set(0, 1, Some("0xdeadbeee".into()), &rs);
        let st = e.statements(&rs).unwrap();
        assert_eq!(
            st[0].0,
            "UPDATE [t] SET [b] = CONVERT(varbinary(max), @P1, 1) \
             WHERE [k] = CONVERT(varbinary(max), @P2, 1)"
        );
        assert_eq!(
            st[0].1,
            vec![
                Param::Text("0xdeadbeee".into()),
                Param::Text("0x0102".into())
            ]
        );

        let (mut e, rs) = binary_setup(Engine::Sqlite);
        e.deleted.insert(0);
        let st = e.statements(&rs).unwrap();
        assert_eq!(st[0].0, "DELETE FROM t WHERE k = unhex(?1)");
        assert_eq!(st[0].1, vec![Param::Text("0102".into())]);

        // Invalid hex is refused before anything is sent.
        let (mut e, rs) = binary_setup(Engine::Postgres);
        e.set(0, 1, Some("hello".into()), &rs);
        let err = e.statements(&rs).unwrap_err().to_string();
        assert!(err.starts_with("b: 'hello' is not a binary value"), "{err}");
    }
}
