//! Schema-aware completion. Pure: the catalog is behind a trait, and anything
//! it has not loaded yet is simply missing from the suggestions (the caller
//! loads it and asks again).

use std::ops::Range;

use sqail_client::proto::{ColumnInfo, Engine, ForeignKeyInfo, TableInfo, TableKind};

use super::lex::{Kind, Token, is_keyword, keywords, tokenize};
use super::{quote_ident, statements};

/// What the completer may look up. `None` = not loaded (yet).
pub trait Catalog {
    fn schemas(&self) -> Option<Vec<String>>;
    fn tables(&self, schema: &str) -> Option<Vec<TableInfo>>;
    fn columns(&self, schema: &str, table: &str) -> Option<Vec<ColumnInfo>>;
    fn foreign_keys(&self, schema: &str, table: &str) -> Option<Vec<ForeignKeyInfo>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CandKind {
    Column,
    Join,
    Table,
    View,
    Schema,
    Keyword,
}

impl CandKind {
    pub fn tag(self) -> &'static str {
        match self {
            CandKind::Column => "col",
            CandKind::Join => "join",
            CandKind::Table => "table",
            CandKind::View => "view",
            CandKind::Schema => "schema",
            CandKind::Keyword => "kw",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub label: String,
    pub insert: String,
    pub kind: CandKind,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// Byte range of `text` the chosen candidate replaces (the typed prefix).
    pub range: Range<usize>,
    pub items: Vec<Candidate>,
}

pub fn default_schema(engine: Engine) -> &'static str {
    match engine {
        Engine::Postgres => "public",
        Engine::Mssql => "dbo",
        Engine::Sqlite => "main",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Source {
    /// A catalog table or view, or a CTE when unqualified and one has the name.
    Table {
        schema: Option<String>,
        table: String,
    },
    /// `(SELECT …) alias`: byte range of the subquery.
    Derived(Range<usize>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TableRef {
    source: Source,
    alias: Option<String>,
    /// The parenthesised group the reference sits in (the whole statement at
    /// top level); it is visible inside it. `depth` is how deeply nested.
    scope: Range<usize>,
    depth: usize,
}

impl TableRef {
    fn table(&self) -> Option<&str> {
        match &self.source {
            Source::Table { table, .. } => Some(table),
            Source::Derived(_) => None,
        }
    }

    /// How the query refers to it: the alias, else the table name.
    fn handle(&self) -> &str {
        self.alias.as_deref().or(self.table()).unwrap_or_default()
    }

    fn visible_at(&self, pos: usize) -> bool {
        self.scope.start <= pos && pos <= self.scope.end
    }
}

/// `WITH name [(columns)] AS (body)`.
struct Cte {
    name: String,
    columns: Vec<String>,
    body: Range<usize>,
}

fn unquote(s: &str) -> String {
    let t = s.trim();
    let inner = match (t.chars().next(), t.chars().last()) {
        (Some('"'), Some('"')) | (Some('['), Some(']')) | (Some('`'), Some('`'))
            if t.len() >= 2 =>
        {
            &t[1..t.len() - 1]
        }
        _ => t,
    };
    inner.replace("\"\"", "\"").replace("]]", "]")
}

fn is_ident(t: &Token) -> bool {
    matches!(t.kind, Kind::Word | Kind::QuotedIdent)
}

/// A column that exists only in the query (CTE or subquery output).
fn named_column(name: String) -> ColumnInfo {
    ColumnInfo {
        name,
        ordinal: 0,
        data_type: String::new(),
        nullable: true,
        default: None,
        primary_key: false,
    }
}

/// One statement's table references and CTEs, with enough structure to
/// resolve an alias to its columns.
struct Query<'a> {
    sql: &'a str,
    /// Tokens without whitespace and comments.
    sig: Vec<&'a Token>,
    /// For each `(` / `)` in `sig`, the index of its partner.
    partner: Vec<Option<usize>>,
    refs: Vec<TableRef>,
    ctes: Vec<Cte>,
}

impl<'a> Query<'a> {
    fn new(sql: &'a str, toks: &'a [Token]) -> Self {
        let sig: Vec<&Token> = toks
            .iter()
            .filter(|t| !matches!(t.kind, Kind::Space | Kind::Comment))
            .collect();
        let mut partner = vec![None; sig.len()];
        let mut open = Vec::new();
        for (i, t) in sig.iter().enumerate() {
            match t.kind {
                Kind::Open => open.push(i),
                Kind::Close => {
                    if let Some(o) = open.pop() {
                        partner[o] = Some(i);
                        partner[i] = Some(o);
                    }
                }
                _ => {}
            }
        }
        let mut q = Self {
            sql,
            sig,
            partner,
            refs: Vec::new(),
            ctes: Vec::new(),
        };
        q.ctes = q.find_ctes();
        q.refs = q.find_refs();
        q
    }

    fn text(&self, i: usize) -> &'a str {
        self.sig.get(i).map_or("", |t| &self.sql[t.range.clone()])
    }

    fn is_word(&self, i: usize, w: &str) -> bool {
        self.sig.get(i).is_some_and(|t| t.kind == Kind::Word)
            && self.text(i).eq_ignore_ascii_case(w)
    }

    fn kind(&self, i: usize) -> Option<Kind> {
        self.sig.get(i).map(|t| t.kind)
    }

    /// An identifier usable as a name (not a keyword) at `i`.
    fn name_at(&self, i: usize) -> Option<String> {
        let t = self.sig.get(i)?;
        (is_ident(t) && (t.kind == Kind::QuotedIdent || !is_keyword(self.text(i))))
            .then(|| unquote(self.text(i)))
    }

    /// Bytes inside the parentheses opened at `open`.
    fn inside(&self, open: usize) -> Range<usize> {
        let start = self.sig[open].range.end;
        let end = self.partner[open].map_or(self.sql.len(), |c| self.sig[c].range.start);
        start..end
    }

    /// `WITH [RECURSIVE] a [(x, y)] AS [NOT] [MATERIALIZED] (…), b AS (…)`.
    fn find_ctes(&self) -> Vec<Cte> {
        let mut out = Vec::new();
        for i in 0..self.sig.len() {
            // `WITH (NOLOCK)` is a table hint, not a CTE.
            if !self.is_word(i, "WITH") || self.kind(i + 1) == Some(Kind::Open) {
                continue;
            }
            let mut j = i + 1;
            if self.is_word(j, "RECURSIVE") {
                j += 1;
            }
            while let Some(name) = self.name_at(j) {
                j += 1;
                let mut columns = Vec::new();
                if self.kind(j) == Some(Kind::Open) {
                    let Some(close) = self.partner[j] else { break };
                    columns = (j + 1..close).filter_map(|k| self.name_at(k)).collect();
                    j = close + 1;
                }
                if !self.is_word(j, "AS") {
                    break;
                }
                j += 1;
                if self.is_word(j, "NOT") {
                    j += 1;
                }
                if self.is_word(j, "MATERIALIZED") {
                    j += 1;
                }
                if self.kind(j) != Some(Kind::Open) {
                    break;
                }
                out.push(Cte {
                    name,
                    columns,
                    body: self.inside(j),
                });
                let Some(close) = self.partner[j] else { break };
                j = close + 1;
                if self.text(j) != "," {
                    break;
                }
                j += 1;
            }
        }
        out
    }

    /// `[AS] alias` at `j`: the alias and the index after it.
    fn alias_at(&self, mut j: usize) -> (Option<String>, usize) {
        if self.is_word(j, "AS") {
            j += 1;
        }
        match self.name_at(j) {
            Some(a) => (Some(a), j + 1),
            None => (None, j),
        }
    }

    /// Table references: `FROM a`, `JOIN s.b AS x`, `FROM (SELECT …) d`, …
    fn find_refs(&self) -> Vec<TableRef> {
        let n = self.sig.len();
        let mut out = Vec::new();
        let mut open: Vec<usize> = Vec::new();
        // Per nesting level: inside a FROM list (a `,` starts a new table).
        let mut in_from = vec![false];
        let mut i = 0;
        while i < n {
            match self.sig[i].kind {
                Kind::Open => {
                    open.push(i);
                    in_from.push(false);
                    i += 1;
                    continue;
                }
                Kind::Close => {
                    open.pop();
                    if in_from.len() > 1 {
                        in_from.pop();
                    }
                    i += 1;
                    continue;
                }
                _ => {}
            }
            let word = self.text(i).to_ascii_uppercase();
            let is_word = self.sig[i].kind == Kind::Word;
            let comma = self.sig[i].kind == Kind::Punct && word == ",";
            let from_list = in_from.last_mut().expect("never empty");
            if is_word
                && matches!(
                    word.as_str(),
                    "WHERE"
                        | "GROUP"
                        | "ORDER"
                        | "HAVING"
                        | "ON"
                        | "SET"
                        | "VALUES"
                        | "LIMIT"
                        | "UNION"
                        | "SELECT"
                )
            {
                *from_list = false;
            }
            let starts_ref = (is_word
                && matches!(
                    word.as_str(),
                    "FROM" | "JOIN" | "APPLY" | "UPDATE" | "INTO" | "TABLE"
                ))
                || (comma && *from_list);
            if !starts_ref {
                i += 1;
                continue;
            }
            *from_list = comma || matches!(word.as_str(), "FROM" | "JOIN" | "APPLY");
            let (scope, depth) = match open.last() {
                Some(&o) => (self.inside(o), open.len()),
                None => (0..self.sql.len(), 0),
            };
            let mut j = i + 1;
            if self.is_word(j, "LATERAL") {
                j += 1;
            }
            if self.kind(j) == Some(Kind::Open) {
                // Derived table; carry on inside it for its own references.
                if let Some(close) = self.partner[j]
                    && let (Some(alias), _) = self.alias_at(close + 1)
                {
                    out.push(TableRef {
                        source: Source::Derived(self.inside(j)),
                        alias: Some(alias),
                        scope,
                        depth,
                    });
                }
                i = j;
                continue;
            }
            // [schema .] table [AS] [alias]
            let Some(mut table) = self.name_at(j) else {
                i = j;
                continue;
            };
            let mut schema = None;
            if self.text(j + 1) == "." {
                match self.name_at(j + 2) {
                    Some(t) => {
                        schema = Some(table);
                        table = t;
                        j += 2;
                    }
                    None => {
                        // `FROM sales.` while typing: not a reference yet.
                        i = j + 2;
                        continue;
                    }
                }
            }
            let (alias, next) = self.alias_at(j + 1);
            out.push(TableRef {
                source: Source::Table { schema, table },
                alias,
                scope,
                depth,
            });
            i = next;
        }
        out
    }

    fn cte(&self, r: &TableRef) -> Option<&Cte> {
        match &r.source {
            Source::Table {
                schema: None,
                table,
            } => self
                .ctes
                .iter()
                .find(|c| c.name.eq_ignore_ascii_case(table)),
            _ => None,
        }
    }

    /// Visible references at `pos`, innermost scope first.
    fn visible(&self, pos: usize) -> Vec<&TableRef> {
        let mut v: Vec<&TableRef> = self.refs.iter().filter(|r| r.visible_at(pos)).collect();
        v.sort_by_key(|r| std::cmp::Reverse(r.depth));
        v
    }

    /// Columns of a reference: from the catalog, or what a CTE or subquery
    /// selects.
    fn columns(&self, r: &TableRef, cx: &Cx<'_>, level: u8) -> Vec<ColumnInfo> {
        if level > 4 {
            return Vec::new();
        }
        if let Some(c) = self.cte(r) {
            return if c.columns.is_empty() {
                self.select_columns(c.body.clone(), cx, level + 1)
            } else {
                c.columns.iter().cloned().map(named_column).collect()
            };
        }
        match &r.source {
            Source::Derived(body) => self.select_columns(body.clone(), cx, level + 1),
            Source::Table { schema, table } => cx
                .cat
                .columns(&cx.schema_of(schema.as_deref(), table), table)
                .unwrap_or_default(),
        }
    }

    /// Output columns of the `SELECT` in `body`, as far as they can be named.
    fn select_columns(&self, body: Range<usize>, cx: &Cx<'_>, level: u8) -> Vec<ColumnInfo> {
        let idx: Vec<usize> = (0..self.sig.len())
            .filter(|&i| {
                let r = &self.sig[i].range;
                body.start <= r.start && r.end <= body.end
            })
            .collect();
        // Top level of the body only.
        let mut depth = 0i32;
        let mut top = Vec::new();
        for &i in &idx {
            match self.sig[i].kind {
                Kind::Open => {
                    if depth == 0 {
                        top.push(i);
                    }
                    depth += 1;
                    continue;
                }
                Kind::Close => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                top.push(i);
            }
        }
        let Some(sel) = top.iter().position(|&i| self.is_word(i, "SELECT")) else {
            return Vec::new();
        };
        let mut k = sel + 1;
        while top
            .get(k)
            .is_some_and(|&i| self.is_word(i, "DISTINCT") || self.is_word(i, "ALL"))
        {
            k += 1;
        }
        if top.get(k).is_some_and(|&i| self.is_word(i, "TOP")) {
            // `TOP n` or `TOP (n)` (only the parentheses are top level).
            k += if top
                .get(k + 1)
                .is_some_and(|&i| self.sig[i].kind == Kind::Open)
            {
                3
            } else {
                2
            };
        }
        let end = top[k.min(top.len())..]
            .iter()
            .position(|&i| {
                ["FROM", "INTO", "WHERE", "UNION"]
                    .iter()
                    .any(|w| self.is_word(i, w))
            })
            .map_or(top.len(), |p| k + p);
        let items = top.get(k..end).unwrap_or_default();

        let here: Vec<&TableRef> = self.refs.iter().filter(|r| r.scope == body).collect();
        let mut out = Vec::new();
        for item in items.split(|&i| self.text(i) == ",") {
            let Some(&last) = item.last() else { continue };
            if self.text(last) == "*" {
                let qual = (item.len() == 3).then(|| unquote(self.text(item[0])));
                for r in &here {
                    if qual
                        .as_deref()
                        .is_none_or(|q| r.handle().eq_ignore_ascii_case(q))
                    {
                        out.extend(self.columns(r, cx, level));
                    }
                }
            } else if item.len() >= 3 && self.text(item[1]) == "=" {
                // T-SQL `name = expr`.
                out.extend(self.name_at(item[0]).map(named_column));
            } else if let Some(name) = self.name_at(last) {
                // `x`, `t.x`, `expr AS x`, `expr x`.
                out.push(named_column(name));
            }
        }
        out
    }
}

/// What resolving references needs besides the query text.
struct Cx<'a> {
    cat: &'a dyn Catalog,
    default: &'a str,
}

impl Cx<'_> {
    /// The schema of a table: as written, else the default schema, else the
    /// first loaded schema that has it.
    fn schema_of(&self, schema: Option<&str>, table: &str) -> String {
        if let Some(s) = schema {
            return s.to_string();
        }
        let has = |s: &str| {
            self.cat
                .tables(s)
                .is_some_and(|ts| ts.iter().any(|t| t.name.eq_ignore_ascii_case(table)))
        };
        // Not loaded yet: assume the default; it is asked for again on load.
        if self.cat.tables(self.default).is_none() || has(self.default) {
            return self.default.to_string();
        }
        self.cat
            .schemas()
            .unwrap_or_default()
            .into_iter()
            .find(|s| !s.eq_ignore_ascii_case(self.default) && has(s))
            .unwrap_or_else(|| self.default.to_string())
    }
}

/// Candidates for the cursor at byte `cursor` in `text`.
pub fn complete(
    text: &str,
    cursor: usize,
    engine: Engine,
    cat: &dyn Catalog,
) -> Option<Completion> {
    let cursor = cursor.min(text.len());
    let stmt = statements::split(text, Some(engine))
        .into_iter()
        .find(|r| r.start <= cursor && cursor <= r.end + 1)
        .unwrap_or(cursor..cursor);
    let start = stmt.start.min(cursor);
    let end = stmt.end.max(cursor);
    let sql = &text[start..end];
    let rel = cursor - start;
    let toks = tokenize(sql, Some(engine));

    // No completion inside strings or comments.
    if toks.iter().any(|t| {
        matches!(t.kind, Kind::String | Kind::Comment | Kind::QuotedIdent)
            && t.range.start < rel
            && rel < t.range.end
    }) {
        return None;
    }

    // The word being typed.
    let bytes = sql.as_bytes();
    let mut p = rel;
    while p > 0
        && (bytes[p - 1].is_ascii_alphanumeric() || bytes[p - 1] == b'_' || bytes[p - 1] >= 0x80)
    {
        p -= 1;
    }
    let prefix = &sql[p..rel];
    // Qualifier: `x.` right before the prefix.
    let qualifier = (p > 0 && bytes[p - 1] == b'.').then(|| {
        let q_end = p - 1;
        let tok = toks
            .iter()
            .rev()
            .find(|t| t.range.end == q_end && is_ident(t));
        tok.map(|t| unquote(&sql[t.range.clone()]))
    });
    let qualifier = qualifier.flatten();

    let query = Query::new(sql, &toks);
    let refs = query.visible(rel);
    let default = default_schema(engine);
    let cx = Cx { cat, default };
    let mut items = Vec::new();

    if let Some(q) = &qualifier {
        // Aliases win over schema names, which win over bare table names;
        // the innermost scope wins over outer ones.
        let by_alias = refs.iter().find(|r| {
            r.alias
                .as_deref()
                .is_some_and(|a| a.eq_ignore_ascii_case(q))
        });
        let is_schema = cat
            .schemas()
            .unwrap_or_default()
            .iter()
            .any(|s| s.eq_ignore_ascii_case(q));
        let by_table = refs
            .iter()
            .find(|r| r.table().is_some_and(|t| t.eq_ignore_ascii_case(q)));
        let hit = by_alias.or(if is_schema { None } else { by_table });
        if let Some(r) = hit {
            let source = r.table().unwrap_or(r.handle());
            for c in query.columns(r, &cx, 0) {
                items.push(column_cand(engine, &c, source));
            }
        } else if is_schema {
            for t in cat.tables(q).unwrap_or_default() {
                items.push(table_cand(engine, &t, None));
            }
        }
    } else {
        // Which clause are we in? The last keyword before the prefix decides.
        let before: Vec<&Token> = toks
            .iter()
            .filter(|t| t.range.end <= p && !matches!(t.kind, Kind::Space | Kind::Comment))
            .collect();
        let last = before
            .last()
            .map(|t| sql[t.range.clone()].to_ascii_uppercase());
        let clause = before
            .iter()
            .rev()
            .find(|t| t.kind == Kind::Word && is_keyword(&sql[t.range.clone()]))
            .map(|t| sql[t.range.clone()].to_ascii_uppercase());
        let table_position = matches!(
            last.as_deref(),
            Some("FROM" | "JOIN" | "UPDATE" | "INTO" | "TABLE")
        ) || (last.as_deref() == Some(",")
            && clause.as_deref() == Some("FROM"));

        if table_position {
            let schemas = cat.schemas().unwrap_or_else(|| vec![default.to_string()]);
            for s in &schemas {
                let qualify = !s.eq_ignore_ascii_case(default);
                for t in cat.tables(s).unwrap_or_default() {
                    items.push(table_cand(engine, &t, qualify.then_some(s.as_str())));
                }
                if qualify && engine != Engine::Sqlite {
                    items.push(Candidate {
                        label: s.clone(),
                        insert: quote_ident(engine, s),
                        kind: CandKind::Schema,
                        detail: "schema".into(),
                    });
                }
            }
            for c in &query.ctes {
                items.push(Candidate {
                    label: c.name.clone(),
                    insert: quote_ident(engine, &c.name),
                    kind: CandKind::Table,
                    detail: "CTE".into(),
                });
            }
            if last.as_deref() == Some("JOIN") {
                items.extend(join_hints(engine, &query, &refs, &cx));
            }
        } else {
            let columns: Vec<(&TableRef, Vec<ColumnInfo>)> = refs
                .iter()
                .map(|r| (*r, query.columns(r, &cx, 0)))
                .collect();
            // A name in more than one table is offered qualified: `o.id`.
            let mut seen = std::collections::HashMap::<String, usize>::new();
            for (_, cols) in &columns {
                for c in cols {
                    *seen.entry(c.name.to_lowercase()).or_default() += 1;
                }
            }
            for (r, cols) in &columns {
                let source = r.table().unwrap_or(r.handle());
                for c in cols {
                    let mut cand = column_cand(engine, c, source);
                    if seen[&c.name.to_lowercase()] > 1 {
                        cand.label = format!("{}.{}", r.handle(), c.name);
                        cand.insert =
                            format!("{}.{}", quote_ident(engine, r.handle()), cand.insert);
                    }
                    items.push(cand);
                }
                if let Some(a) = &r.alias {
                    items.push(Candidate {
                        label: a.clone(),
                        insert: quote_ident(engine, a),
                        kind: CandKind::Table,
                        detail: format!("alias of {source}"),
                    });
                }
            }
            if refs.is_empty() {
                for t in cat.tables(default).unwrap_or_default() {
                    items.push(table_cand(engine, &t, None));
                }
            }
        }
        // After FROM/JOIN only names make sense, unless nothing matches.
        let names_match = items
            .iter()
            .any(|c| c.label.to_lowercase().contains(&prefix.to_lowercase()));
        if !prefix.is_empty() && !(table_position && names_match) {
            for k in keywords() {
                items.push(Candidate {
                    label: k.to_string(),
                    insert: k.to_string(),
                    kind: CandKind::Keyword,
                    detail: String::new(),
                });
            }
        }
    }

    let lower = prefix.to_lowercase();
    let mut scored: Vec<(u8, Candidate)> = items
        .into_iter()
        .filter_map(|c| {
            let l = c.label.to_lowercase();
            // Qualified columns (`o.id`) match on the column part.
            let name = match c.kind {
                CandKind::Column => l.rsplit('.').next().unwrap_or(&l),
                _ => &l,
            };
            let rank = if lower.is_empty() || name.starts_with(&lower) {
                0
            } else if name.contains(&lower) {
                1
            } else {
                return None;
            };
            // Do not offer exactly what is already typed.
            (l != lower).then_some((rank, c))
        })
        .collect();
    scored.sort_by(|a, b| (a.0, a.1.kind, &a.1.label).cmp(&(b.0, b.1.kind, &b.1.label)));
    scored.dedup_by(|a, b| a.1.label == b.1.label && a.1.kind == b.1.kind);
    let items: Vec<Candidate> = scored.into_iter().map(|(_, c)| c).take(60).collect();
    (!items.is_empty()).then(|| Completion {
        range: start + p..start + rel,
        items,
    })
}

fn column_cand(engine: Engine, c: &ColumnInfo, table: &str) -> Candidate {
    let ty = format!("{}{}", c.data_type, if c.primary_key { " PK" } else { "" });
    Candidate {
        label: c.name.clone(),
        insert: quote_ident(engine, &c.name),
        kind: CandKind::Column,
        detail: if ty.is_empty() {
            table.to_string()
        } else {
            format!("{ty} · {table}")
        },
    }
}

fn table_cand(engine: Engine, t: &TableInfo, schema: Option<&str>) -> Candidate {
    let insert = match schema {
        Some(s) => format!(
            "{}.{}",
            quote_ident(engine, s),
            quote_ident(engine, &t.name)
        ),
        None => quote_ident(engine, &t.name),
    };
    Candidate {
        label: match schema {
            Some(s) => format!("{s}.{}", t.name),
            None => t.name.clone(),
        },
        insert,
        kind: if t.kind == TableKind::Table {
            CandKind::Table
        } else {
            CandKind::View
        },
        detail: t.schema.clone().unwrap_or_default(),
    }
}

/// `JOIN` suggestions from foreign keys of the tables already in the query.
fn join_hints(
    engine: Engine,
    query: &Query<'_>,
    refs: &[&TableRef],
    cx: &Cx<'_>,
) -> Vec<Candidate> {
    let default = cx.default;
    let mut out = Vec::new();
    for r in refs {
        let Source::Table { schema, table } = &r.source else {
            continue;
        };
        if query.cte(r).is_some() {
            continue;
        }
        let schema = cx.schema_of(schema.as_deref(), table);
        for fk in cx.cat.foreign_keys(&schema, table).unwrap_or_default() {
            let target = &fk.ref_table;
            let alias = short_alias(target, refs);
            let conds: Vec<String> = fk
                .columns
                .iter()
                .zip(&fk.ref_columns)
                .map(|(c, rc)| {
                    format!(
                        "{alias}.{} = {}.{}",
                        quote_ident(engine, rc),
                        r.handle(),
                        quote_ident(engine, c)
                    )
                })
                .collect();
            if conds.is_empty() {
                continue;
            }
            let qualified_target = match fk.ref_schema.as_deref() {
                Some(s) if !s.eq_ignore_ascii_case(default) => {
                    format!("{}.{}", quote_ident(engine, s), quote_ident(engine, target))
                }
                _ => quote_ident(engine, target),
            };
            let text = format!("{qualified_target} {alias} ON {}", conds.join(" AND "));
            out.push(Candidate {
                label: text.clone(),
                insert: text,
                kind: CandKind::Join,
                detail: format!("via {}", fk.name),
            });
        }
    }
    out
}

/// First letter(s) of the table, not clashing with existing handles.
fn short_alias(table: &str, refs: &[&TableRef]) -> String {
    let base: String = table
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .take(1)
        .collect::<String>()
        .to_lowercase();
    let base = if base.is_empty() {
        "t".to_string()
    } else {
        base
    };
    let taken = |a: &str| refs.iter().any(|r| r.handle().eq_ignore_ascii_case(a));
    if !taken(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}{n}"))
        .find(|a| !taken(a))
        .expect("infinite")
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqail_client::proto::{ColumnInfo, ForeignKeyInfo, TableInfo, TableKind};

    struct Fake;

    fn col(name: &str, pk: bool) -> ColumnInfo {
        ColumnInfo {
            name: name.into(),
            ordinal: 0,
            data_type: "int".into(),
            nullable: !pk,
            default: None,
            primary_key: pk,
        }
    }

    impl Catalog for Fake {
        fn schemas(&self) -> Option<Vec<String>> {
            Some(vec!["public".into(), "sales".into()])
        }
        fn tables(&self, schema: &str) -> Option<Vec<TableInfo>> {
            let t = |n: &str| TableInfo {
                schema: Some(schema.into()),
                name: n.into(),
                kind: TableKind::Table,
            };
            Some(match schema {
                "sales" => vec![t("orders"), t("customers")],
                _ => vec![t("notes")],
            })
        }
        fn columns(&self, schema: &str, table: &str) -> Option<Vec<ColumnInfo>> {
            // Strict about the schema, like the real catalog.
            Some(match (schema, table) {
                ("sales", "orders") => vec![
                    col("id", true),
                    col("customer_id", false),
                    col("status", false),
                ],
                ("sales", "customers") => vec![col("id", true), col("name", false)],
                ("public", "notes") => vec![col("body", false)],
                _ => return None,
            })
        }
        fn foreign_keys(&self, _schema: &str, table: &str) -> Option<Vec<ForeignKeyInfo>> {
            Some(match table {
                "orders" => vec![ForeignKeyInfo {
                    name: "fk_cust".into(),
                    columns: vec!["customer_id".into()],
                    ref_schema: Some("sales".into()),
                    ref_table: "customers".into(),
                    ref_columns: vec!["id".into()],
                }],
                _ => vec![],
            })
        }
    }

    fn labels(text: &str) -> Vec<String> {
        let cursor = text.find('|').expect("cursor marker");
        let t = text.replace('|', "");
        complete(&t, cursor, Engine::Postgres, &Fake)
            .map(|c| c.items.into_iter().map(|i| i.label).collect())
            .unwrap_or_default()
    }

    #[test]
    fn alias_dot_lists_columns() {
        let l = labels("SELECT o.| FROM sales.orders o");
        assert_eq!(l, ["customer_id", "id", "status"]);
        let l = labels("SELECT o.st| FROM sales.orders AS o");
        assert_eq!(
            l,
            ["status", "customer_id"],
            "prefix matches first, then substring"
        );
    }

    #[test]
    fn schema_dot_lists_tables() {
        let l = labels("SELECT * FROM sales.|");
        assert_eq!(l, ["customers", "orders"]);
    }

    #[test]
    fn table_position_offers_tables_and_schemas() {
        let l = labels("SELECT * FROM |");
        assert!(l.contains(&"notes".to_string()), "{l:?}");
        assert!(l.contains(&"sales.orders".to_string()), "{l:?}");
        assert!(l.contains(&"sales".to_string()), "{l:?}");
    }

    #[test]
    fn column_position_offers_columns_then_keywords() {
        let l = labels("SELECT na| FROM sales.customers c");
        assert_eq!(l.first().map(String::as_str), Some("name"));
        let l = labels("SELECT * FROM sales.orders WHERE sta|");
        assert_eq!(l.first().map(String::as_str), Some("status"));
    }

    #[test]
    fn join_hints_from_foreign_keys() {
        let l = labels("SELECT * FROM sales.orders o JOIN |");
        assert!(
            l.contains(&"sales.customers c ON c.id = o.customer_id".to_string()),
            "{l:?}"
        );
    }

    #[test]
    fn nothing_inside_strings() {
        assert!(labels("SELECT 'o.|' FROM sales.orders o").is_empty());
    }

    #[test]
    fn replacement_range_is_the_prefix() {
        let t = "SELECT o.sta FROM sales.orders o";
        let c = complete(t, 12, Engine::Postgres, &Fake).unwrap();
        assert_eq!(&t[c.range.clone()], "sta");
    }

    #[test]
    fn unqualified_table_found_in_its_schema() {
        let l = labels("SELECT o.| FROM orders o");
        assert_eq!(l, ["customer_id", "id", "status"]);
    }

    #[test]
    fn alias_in_where_after_joins() {
        let l = labels(
            "SELECT * FROM sales.orders o JOIN sales.customers c ON c.id = o.customer_id WHERE c.|",
        );
        assert_eq!(l, ["id", "name"]);
    }

    #[test]
    fn innermost_alias_wins() {
        let l =
            labels("SELECT * FROM sales.orders c WHERE x IN (SELECT c.| FROM sales.customers c)");
        assert_eq!(l, ["id", "name"]);
        let l =
            labels("SELECT c.| FROM sales.orders c WHERE x IN (SELECT 1 FROM sales.customers c)");
        assert_eq!(
            l,
            ["customer_id", "id", "status"],
            "inner scope is not visible outside"
        );
    }

    #[test]
    fn cte_columns() {
        let l = labels("WITH x AS (SELECT id, name FROM sales.customers) SELECT x.| FROM x");
        assert_eq!(l, ["id", "name"]);
        let l = labels("WITH x (a, b) AS (SELECT 1, 2) SELECT y.| FROM x y");
        assert_eq!(l, ["a", "b"]);
        let l = labels("WITH x AS (SELECT c.* FROM sales.customers c) SELECT x.| FROM x");
        assert_eq!(l, ["id", "name"], "star expands to the table's columns");
        let l = labels("WITH x AS (SELECT 1 AS n) SELECT * FROM |");
        assert!(
            l.contains(&"x".to_string()),
            "CTEs are offered as tables: {l:?}"
        );
    }

    #[test]
    fn derived_table_columns() {
        let l = labels("SELECT s.| FROM (SELECT id, status AS st, count(*) n FROM sales.orders) s");
        assert_eq!(l, ["id", "n", "st"]);
        let t = "SELECT d. FROM (SELECT TOP (5) id FROM sales.orders) d";
        let c = complete(t, 9, Engine::Mssql, &Fake).unwrap();
        let l: Vec<_> = c.items.into_iter().map(|i| i.label).collect();
        assert_eq!(l, ["id"]);
    }

    #[test]
    fn table_hint_is_not_an_alias() {
        let t = "SELECT o. FROM sales.orders o WITH (NOLOCK)";
        let c = complete(t, 9, Engine::Mssql, &Fake).unwrap();
        assert_eq!(c.items.len(), 3);
    }

    #[test]
    fn shared_column_names_are_qualified() {
        let l = labels("SELECT i| FROM sales.orders o JOIN sales.customers c ON 1=1");
        assert!(
            l.contains(&"o.id".to_string()) && l.contains(&"c.id".to_string()),
            "{l:?}"
        );
        assert!(!l.contains(&"id".to_string()), "{l:?}");
        assert!(
            l.contains(&"customer_id".to_string()),
            "unique names stay bare: {l:?}"
        );
    }
}
