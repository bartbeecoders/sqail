//! Inline data editing: decide whether a result can be edited, track pending
//! changes, and turn them into parameterised DML applied in one transaction.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail};
use sqail_client::proto::{ColumnInfo, Engine, Param, QueryEvent, QueryRequest};
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
    let name = |i: usize| {
        toks.get(i)
            .filter(|t| matches!(t.kind, Kind::Word | Kind::QuotedIdent))
            .map(|t| unquote(&sql[t.range.clone()]))
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

    fn ph(&self, n: usize, col: &ColumnInfo) -> String {
        match self.engine {
            Engine::Postgres => format!("${n}::text::{}", col.data_type),
            Engine::Mssql => format!("@P{n}"),
            Engine::Sqlite => format!("?{n}"),
        }
    }

    fn col(&self, i: usize) -> &ColumnInfo {
        self.target[i].as_ref().expect("editable column")
    }

    /// WHERE clause matching the original primary key of `row`.
    fn key(&self, rs: &ResultSet, row: usize, n: &mut usize, params: &mut Vec<Param>) -> String {
        self.pk
            .iter()
            .map(|&c| {
                *n += 1;
                let col = self.col(c);
                params.push(Param::Text(
                    rs.raw_row(row)[c]
                        .display(rs.columns[c].logical)
                        .into_owned(),
                ));
                format!(
                    "{} = {}",
                    quote_ident(self.engine, &col.name),
                    self.ph(*n, col)
                )
            })
            .collect::<Vec<_>>()
            .join(" AND ")
    }

    /// The statements to run, in order: deletes, updates, inserts.
    pub fn statements(&self, rs: &ResultSet) -> Vec<(String, Vec<Param>)> {
        let table = self.table_sql();
        let mut out = Vec::new();
        let value = |v: &Value| v.clone().map_or(Param::Null, Param::Text);
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
            let sets: Vec<String> = cols
                .iter()
                .map(|(&c, v)| {
                    n += 1;
                    params.push(value(v));
                    let col = self.col(c);
                    format!(
                        "{} = {}",
                        quote_ident(self.engine, &col.name),
                        self.ph(n, col)
                    )
                })
                .collect();
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
                    params.push(value(v));
                    let col = self.col(c);
                    names.push(quote_ident(self.engine, &col.name));
                    phs.push(self.ph(n, col));
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
        out
    }
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
        let (pat, rep) = match engine {
            // `$n::text::type` → `'value'::type`
            Engine::Postgres => (format!("${n}::text::"), format!("{lit}::")),
            Engine::Mssql => (format!("@P{n}"), lit),
            Engine::Sqlite => (format!("?{n}"), lit),
        };
        s = s.replace(&pat, &rep);
    }
    s
}

/// Apply statements in one transaction on a fresh session.
pub async fn apply(
    client: &Client,
    connection: Uuid,
    engine: Engine,
    stmts: Vec<(String, Vec<Param>)>,
) -> Result<usize> {
    let session = client.open_session(connection).await?.id;
    let run = |sql: String, params: Vec<Param>| async move {
        let events = client
            .query_all(
                On::Session(session),
                &QueryRequest {
                    sql,
                    params,
                    max_rows: Some(1),
                    timeout_ms: None,
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
    let begin = if engine == Engine::Mssql {
        "BEGIN TRANSACTION"
    } else {
        "BEGIN"
    };
    let result = async {
        run(begin.into(), vec![]).await?;
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
        run("COMMIT".into(), vec![]).await?;
        Ok::<usize, anyhow::Error>(n)
    }
    .await;
    if result.is_err() {
        let _ = run("ROLLBACK".into(), vec![]).await;
    }
    let _ = client.close_session(session).await;
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
        let st = e.statements(&rs);
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
        let st = e.statements(&rs);
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
}
