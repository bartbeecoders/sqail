//! Pretty-printing via `sqlformat`, with the bits it does not understand kept
//! out of its way: SQL Server `GO` separators and Postgres dollar-quoted bodies.

use sqail_client::proto::Engine;
use sqlformat::{Dialect, FormatOptions, Indent, QueryParams};

use super::lex::{Kind, tokenize};

pub struct Style {
    pub uppercase: bool,
    pub indent: u8,
}

/// Formatted SQL, or an explanation of why it was left alone.
pub fn format(sql: &str, engine: Option<Engine>, style: &Style) -> Result<String, String> {
    let dollar_quoted = tokenize(sql, engine)
        .iter()
        .any(|t| t.kind == Kind::String && sql[t.range.clone()].starts_with('$'));
    if dollar_quoted {
        return Err("left unchanged: dollar-quoted bodies ($$ … $$) are not formatted".into());
    }
    let options = FormatOptions {
        indent: Indent::Spaces(style.indent),
        uppercase: Some(style.uppercase),
        lines_between_queries: 2,
        dialect: match engine {
            Some(Engine::Postgres) => Dialect::PostgreSql,
            Some(Engine::Mssql) => Dialect::SQLServer,
            _ => Dialect::Generic,
        },
        ..Default::default()
    };
    let one = |s: &str| sqlformat::format(s.trim(), &QueryParams::None, &options);
    if engine == Some(Engine::Mssql) {
        // Format each batch on its own; keep the GO lines.
        let mut out = Vec::new();
        let mut batch = String::new();
        for line in sql.lines() {
            if line.trim().eq_ignore_ascii_case("go")
                || line.trim().to_ascii_lowercase().starts_with("go ")
            {
                if !batch.trim().is_empty() {
                    out.push(one(&batch));
                }
                out.push(line.trim().to_ascii_uppercase());
                batch.clear();
            } else {
                batch.push_str(line);
                batch.push('\n');
            }
        }
        if !batch.trim().is_empty() {
            out.push(one(&batch));
        }
        return Ok(out.join("\n"));
    }
    Ok(one(sql))
}

#[cfg(test)]
mod tests {
    use super::*;

    const STYLE: Style = Style {
        uppercase: true,
        indent: 2,
    };

    #[test]
    fn formats_and_uppercases() {
        let out = format(
            "select a, b from t where x = 1",
            Some(Engine::Postgres),
            &STYLE,
        )
        .unwrap();
        assert!(
            out.starts_with("SELECT\n  a,\n  b\nFROM\n  t\nWHERE\n  x = 1"),
            "{out}"
        );
    }

    #[test]
    fn keeps_go_batches() {
        let out = format(
            "select 1\ngo\nselect [x] from [t]\nGO 2",
            Some(Engine::Mssql),
            &STYLE,
        )
        .unwrap();
        assert_eq!(out, "SELECT\n  1\nGO\nSELECT\n  [x]\nFROM\n  [t]\nGO 2");
    }

    #[test]
    fn refuses_dollar_quotes() {
        assert!(
            format(
                "create function f() returns int as $$ select 1 $$ language sql",
                Some(Engine::Postgres),
                &STYLE
            )
            .is_err()
        );
    }
}
