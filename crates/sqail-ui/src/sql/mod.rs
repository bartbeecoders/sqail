//! SQL text helpers for the editor: lexing, highlighting, statement bounds.

pub mod complete;
pub mod drop;
pub mod format;
pub mod highlight;
pub mod lex;
pub mod statements;

/// Every keyword PostgreSQL won't take as a bare identifier: what
/// `SELECT word FROM pg_get_keywords() WHERE catcode <> 'U'` returns on
/// PostgreSQL 17 (reserved, type/function-name and column-name keywords).
/// The server's own `quote_ident` quotes exactly these, so quoted names match
/// `pg_get_indexdef` output. Sorted; `tests/designer.rs` checks it against
/// the test server. Don't use the highlighter's keyword list for this.
pub const PG_MUST_QUOTE: &[&str] = &[
    "all",
    "analyse",
    "analyze",
    "and",
    "any",
    "array",
    "as",
    "asc",
    "asymmetric",
    "authorization",
    "between",
    "bigint",
    "binary",
    "bit",
    "boolean",
    "both",
    "case",
    "cast",
    "char",
    "character",
    "check",
    "coalesce",
    "collate",
    "collation",
    "column",
    "concurrently",
    "constraint",
    "create",
    "cross",
    "current_catalog",
    "current_date",
    "current_role",
    "current_schema",
    "current_time",
    "current_timestamp",
    "current_user",
    "dec",
    "decimal",
    "default",
    "deferrable",
    "desc",
    "distinct",
    "do",
    "else",
    "end",
    "except",
    "exists",
    "extract",
    "false",
    "fetch",
    "float",
    "for",
    "foreign",
    "freeze",
    "from",
    "full",
    "grant",
    "greatest",
    "group",
    "grouping",
    "having",
    "ilike",
    "in",
    "initially",
    "inner",
    "inout",
    "int",
    "integer",
    "intersect",
    "interval",
    "into",
    "is",
    "isnull",
    "join",
    "json",
    "json_array",
    "json_arrayagg",
    "json_exists",
    "json_object",
    "json_objectagg",
    "json_query",
    "json_scalar",
    "json_serialize",
    "json_table",
    "json_value",
    "lateral",
    "leading",
    "least",
    "left",
    "like",
    "limit",
    "localtime",
    "localtimestamp",
    "merge_action",
    "national",
    "natural",
    "nchar",
    "none",
    "normalize",
    "not",
    "notnull",
    "null",
    "nullif",
    "numeric",
    "offset",
    "on",
    "only",
    "or",
    "order",
    "out",
    "outer",
    "overlaps",
    "overlay",
    "placing",
    "position",
    "precision",
    "primary",
    "real",
    "references",
    "returning",
    "right",
    "row",
    "select",
    "session_user",
    "setof",
    "similar",
    "smallint",
    "some",
    "substring",
    "symmetric",
    "system_user",
    "table",
    "tablesample",
    "then",
    "time",
    "timestamp",
    "to",
    "trailing",
    "treat",
    "trim",
    "true",
    "union",
    "unique",
    "user",
    "using",
    "values",
    "varchar",
    "variadic",
    "verbose",
    "when",
    "where",
    "window",
    "with",
    "xmlattributes",
    "xmlconcat",
    "xmlelement",
    "xmlexists",
    "xmlforest",
    "xmlnamespaces",
    "xmlparse",
    "xmlpi",
    "xmlroot",
    "xmlserialize",
    "xmltable",
];

/// SQLite's keywords (the 147 that `sqlite3_keyword_name` lists). SQLite
/// takes many of them as bare names, but not all: `current_date` reads the
/// date and `notnull` is an operator. Sorted.
const SQLITE_KEYWORDS: &[&str] = &[
    "abort",
    "action",
    "add",
    "after",
    "all",
    "alter",
    "always",
    "analyze",
    "and",
    "as",
    "asc",
    "attach",
    "autoincrement",
    "before",
    "begin",
    "between",
    "by",
    "cascade",
    "case",
    "cast",
    "check",
    "collate",
    "column",
    "commit",
    "conflict",
    "constraint",
    "create",
    "cross",
    "current",
    "current_date",
    "current_time",
    "current_timestamp",
    "database",
    "default",
    "deferrable",
    "deferred",
    "delete",
    "desc",
    "detach",
    "distinct",
    "do",
    "drop",
    "each",
    "else",
    "end",
    "escape",
    "except",
    "exclude",
    "exclusive",
    "exists",
    "explain",
    "fail",
    "filter",
    "first",
    "following",
    "for",
    "foreign",
    "from",
    "full",
    "generated",
    "glob",
    "group",
    "groups",
    "having",
    "if",
    "ignore",
    "immediate",
    "in",
    "index",
    "indexed",
    "initially",
    "inner",
    "insert",
    "instead",
    "intersect",
    "into",
    "is",
    "isnull",
    "join",
    "key",
    "last",
    "left",
    "like",
    "limit",
    "match",
    "materialized",
    "natural",
    "no",
    "not",
    "nothing",
    "notnull",
    "null",
    "nulls",
    "of",
    "offset",
    "on",
    "or",
    "order",
    "others",
    "outer",
    "over",
    "partition",
    "plan",
    "pragma",
    "preceding",
    "primary",
    "query",
    "raise",
    "range",
    "recursive",
    "references",
    "regexp",
    "reindex",
    "release",
    "rename",
    "replace",
    "restrict",
    "returning",
    "right",
    "rollback",
    "row",
    "rows",
    "savepoint",
    "select",
    "set",
    "table",
    "temp",
    "temporary",
    "then",
    "ties",
    "to",
    "transaction",
    "trigger",
    "unbounded",
    "union",
    "unique",
    "update",
    "using",
    "vacuum",
    "values",
    "view",
    "virtual",
    "when",
    "where",
    "window",
    "with",
    "without",
];

/// Whether PostgreSQL needs `word` (lowercase) quoted to read it as a name.
pub fn pg_must_quote(word: &str) -> bool {
    PG_MUST_QUOTE.binary_search(&word).is_ok()
}

/// Quote an identifier for the given engine.
pub fn quote_ident(engine: sqail_client::proto::Engine, name: &str) -> String {
    use sqail_client::proto::Engine;
    match engine {
        Engine::Mssql => format!("[{}]", name.replace(']', "]]")),
        // Standard SQL quoting for unknown engines too.
        Engine::Postgres | Engine::Sqlite | Engine::Unknown => {
            let plain = name
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                && match engine {
                    Engine::Postgres => !pg_must_quote(name),
                    Engine::Sqlite => {
                        !lex::is_keyword(name) && SQLITE_KEYWORDS.binary_search(&name).is_err()
                    }
                    _ => !pg_must_quote(name) && !lex::is_keyword(name),
                };
            if plain {
                name.to_string()
            } else {
                format!("\"{}\"", name.replace('"', "\"\""))
            }
        }
    }
}

/// `schema.table`, quoted; SQLite's default `main` schema is left out.
pub fn qualified_name(
    engine: sqail_client::proto::Engine,
    schema: Option<&str>,
    table: &str,
) -> String {
    use sqail_client::proto::Engine;
    match schema {
        Some(s) if engine != Engine::Sqlite || s != "main" => {
            format!("{}.{}", quote_ident(engine, s), quote_ident(engine, table))
        }
        _ => quote_ident(engine, table),
    }
}

/// `SELECT … first 100 rows` in the engine's dialect.
pub fn select_top(
    engine: sqail_client::proto::Engine,
    schema: Option<&str>,
    table: &str,
    n: u32,
) -> String {
    use sqail_client::proto::Engine;
    let name = qualified_name(engine, schema, table);
    match engine {
        Engine::Mssql => format!("SELECT TOP ({n}) *\nFROM {name};"),
        _ => format!("SELECT *\nFROM {name}\nLIMIT {n};"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqail_client::proto::Engine;

    #[test]
    fn postgres_quotes_every_word_it_reserves() {
        let mut sorted = PG_MUST_QUOTE.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted, PG_MUST_QUOTE);
        let mut sorted = SQLITE_KEYWORDS.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, SQLITE_KEYWORDS);
        for w in ["current_date", "notnull", "raise", "order"] {
            assert_eq!(quote_ident(Engine::Sqlite, w), format!("\"{w}\""), "{w}");
        }
        for w in [
            "user",
            "array",
            "both",
            "leading",
            "analyze",
            "natural",
            "current_date",
            "position",
            "select",
        ] {
            assert_eq!(quote_ident(Engine::Postgres, w), format!("\"{w}\""), "{w}");
        }
        // Unreserved keywords and plain names stay bare, as Postgres prints them.
        for w in ["name", "key", "type", "text", "index", "orders", "_x1"] {
            assert_eq!(quote_ident(Engine::Postgres, w), w);
        }
        assert_eq!(
            qualified_name(Engine::Postgres, Some("s"), "user"),
            "s.\"user\""
        );
    }

    #[test]
    fn select_top_per_dialect() {
        assert_eq!(
            select_top(Engine::Mssql, Some("sales"), "orders", 100),
            "SELECT TOP (100) *\nFROM [sales].[orders];"
        );
        assert_eq!(
            select_top(Engine::Postgres, Some("sales"), "Order", 5),
            "SELECT *\nFROM sales.\"Order\"\nLIMIT 5;"
        );
        assert_eq!(
            select_top(Engine::Sqlite, Some("main"), "t", 1),
            "SELECT *\nFROM t\nLIMIT 1;"
        );
    }
}
