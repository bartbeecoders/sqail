//! SQL text helpers for the editor: lexing, highlighting, statement bounds.

pub mod complete;
pub mod drop;
pub mod format;
pub mod highlight;
pub mod lex;
pub mod statements;

/// Quote an identifier for the given engine.
pub fn quote_ident(engine: sqail_client::proto::Engine, name: &str) -> String {
    use sqail_client::proto::Engine;
    match engine {
        Engine::Mssql => format!("[{}]", name.replace(']', "]]")),
        Engine::Postgres | Engine::Sqlite => {
            let plain = name
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                && !lex::is_keyword(name);
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
