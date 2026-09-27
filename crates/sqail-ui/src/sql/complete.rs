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
struct TableRef {
    schema: Option<String>,
    table: String,
    alias: Option<String>,
}

impl TableRef {
    fn handle(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.table)
    }
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

/// Table references in a statement: `FROM a`, `JOIN s.b AS x`, `UPDATE c y`, …
fn table_refs(sql: &str, toks: &[Token]) -> Vec<TableRef> {
    let sig: Vec<&Token> = toks
        .iter()
        .filter(|t| !matches!(t.kind, Kind::Space | Kind::Comment))
        .collect();
    let text = |t: &Token| &sql[t.range.clone()];
    let mut out = Vec::new();
    let mut i = 0;
    let mut in_from = false;
    while i < sig.len() {
        let t = sig[i];
        let word = text(t).to_ascii_uppercase();
        let starts_ref = matches!(word.as_str(), "FROM" | "JOIN" | "UPDATE" | "INTO" | "TABLE")
            || (in_from && t.kind == Kind::Punct && text(t) == ",");
        if t.kind == Kind::Word
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
            )
        {
            in_from = false;
        }
        if starts_ref {
            in_from = matches!(word.as_str(), "FROM" | "JOIN") || text(t) == ",";
            // [schema .] table [AS] [alias]
            let mut j = i + 1;
            if j < sig.len() && is_ident(sig[j]) && !is_keyword(text(sig[j])) {
                let mut schema = None;
                let mut table = unquote(text(sig[j]));
                if j + 1 < sig.len() && text(sig[j + 1]) == "." {
                    if j + 2 < sig.len() && is_ident(sig[j + 2]) {
                        schema = Some(table);
                        table = unquote(text(sig[j + 2]));
                        j += 2;
                    } else {
                        // `FROM sales.` while typing: not a reference yet.
                        i = j + 2;
                        continue;
                    }
                }
                j += 1;
                if j < sig.len() && text(sig[j]).eq_ignore_ascii_case("AS") {
                    j += 1;
                }
                let alias = (j < sig.len() && is_ident(sig[j]) && !is_keyword(text(sig[j])))
                    .then(|| unquote(text(sig[j])));
                if alias.is_some() {
                    j += 1;
                }
                out.push(TableRef {
                    schema,
                    table,
                    alias,
                });
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out
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

    let refs = table_refs(sql, &toks);
    let default = default_schema(engine);
    let schema_of = |r: &TableRef| r.schema.clone().unwrap_or_else(|| default.to_string());
    let mut items = Vec::new();

    if let Some(q) = &qualifier {
        // Aliases win over schema names, which win over bare table names.
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
        let by_table = refs.iter().find(|r| r.table.eq_ignore_ascii_case(q));
        let hit = by_alias.or(if is_schema { None } else { by_table });
        if let Some(r) = hit {
            for c in cat.columns(&schema_of(r), &r.table).unwrap_or_default() {
                items.push(column_cand(engine, &c, &r.table));
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
            if last.as_deref() == Some("JOIN") {
                items.extend(join_hints(engine, &refs, default, cat));
            }
        } else {
            for r in &refs {
                for c in cat.columns(&schema_of(r), &r.table).unwrap_or_default() {
                    items.push(column_cand(engine, &c, &r.table));
                }
                if let Some(a) = &r.alias {
                    items.push(Candidate {
                        label: a.clone(),
                        insert: a.clone(),
                        kind: CandKind::Table,
                        detail: format!("alias of {}", r.table),
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
            let rank = if lower.is_empty() || l.starts_with(&lower) {
                0
            } else if l.contains(&lower) {
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
    Candidate {
        label: c.name.clone(),
        insert: quote_ident(engine, &c.name),
        kind: CandKind::Column,
        detail: format!(
            "{}{} · {table}",
            c.data_type,
            if c.primary_key { " PK" } else { "" }
        ),
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
    refs: &[TableRef],
    default: &str,
    cat: &dyn Catalog,
) -> Vec<Candidate> {
    let mut out = Vec::new();
    for r in refs {
        let schema = r.schema.clone().unwrap_or_else(|| default.to_string());
        for fk in cat.foreign_keys(&schema, &r.table).unwrap_or_default() {
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
fn short_alias(table: &str, refs: &[TableRef]) -> String {
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
        fn columns(&self, _schema: &str, table: &str) -> Option<Vec<ColumnInfo>> {
            Some(match table {
                "orders" => vec![
                    col("id", true),
                    col("customer_id", false),
                    col("status", false),
                ],
                "customers" => vec![col("id", true), col("name", false)],
                _ => vec![col("body", false)],
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
}
