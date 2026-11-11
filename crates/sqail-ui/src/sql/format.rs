//! Pretty-printing via `sqlformat`, with the bits it does not understand kept
//! out of its way: SQL Server `GO` separators and Postgres dollar-quoted bodies.
//! Its output is checked against our own lexer: when it changed more than
//! whitespace and keyword case (a literal, a number, an operator), the text is
//! left alone.

use sqail_client::proto::Engine;
use sqlformat::{Dialect, FormatOptions, Indent, QueryParams};

use super::lex::{Kind, Token, tokenize};
use super::statements::is_go_line;

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
    let out = if engine == Some(Engine::Mssql) {
        // Format each batch on its own; keep the GO lines. A `go` inside a
        // string or comment is not one (the lexer knows).
        let mut out = Vec::new();
        let mut from = 0;
        for t in tokenize(sql, engine) {
            if t.range.start < from || t.kind != Kind::Word || !is_go_line(sql, &t.range) {
                continue;
            }
            let line_start = sql[..t.range.start].rfind('\n').map_or(0, |p| p + 1);
            let line_end = sql[t.range.end..]
                .find('\n')
                .map_or(sql.len(), |p| t.range.end + p);
            let batch = &sql[from..line_start];
            if !batch.trim().is_empty() {
                out.push(one(batch));
            }
            // The rest of the line: a repeat count, a comment.
            out.push(format!("GO{}", sql[t.range.end..line_end].trim_end()));
            from = line_end;
        }
        if !sql[from..].trim().is_empty() {
            out.push(one(&sql[from..]));
        }
        out.join("\n")
    } else {
        one(sql)
    };
    match first_change(sql, &out, engine) {
        None => Ok(out),
        Some(near) => {
            let mut short: String = near.chars().take(40).collect();
            if short.len() < near.len() {
                short.push('…');
            }
            let near = short.replace('\n', " ");
            Err(format!(
                "left unchanged: the formatter would have changed more than layout and keyword case (near `{near}`)"
            ))
        }
    }
}

/// Multi-character operators our lexer splits into one-character tokens:
/// formatting must not put a space inside them.
const OPERATORS: &[&str] = &[
    "<>", "!=", ">=", "<=", "!<", "!>", "||", "&&", "->", ">>", "<<", "#>", "#-", "@>", "<@", "!~",
    "~*", "~~", "?|", "?&", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=",
];

/// Must `prev` and `next` stay together when they touch? Parts of one
/// operator, and a prefix glued to a literal (`B'01'`, `U&'x'`, `U&"x"`).
fn joined(sql: &str, prev: &Token, next: &Token) -> bool {
    let (p, n) = (&sql[prev.range.clone()], &sql[next.range.clone()]);
    match (prev.kind, next.kind) {
        (Kind::Punct, Kind::Punct) => OPERATORS.contains(&format!("{p}{n}").as_str()),
        (Kind::Word, Kind::String | Kind::QuotedIdent) => true,
        (Kind::Punct, Kind::String | Kind::QuotedIdent) => p == "&",
        (Kind::Word, Kind::Punct) => p.eq_ignore_ascii_case("u") && n == "&",
        _ => false,
    }
}

/// What a token means, for comparing before and after: words in any case,
/// comments up to whitespace, everything else exactly.
fn meaning(sql: &str, t: &Token) -> String {
    let s = &sql[t.range.clone()];
    match t.kind {
        Kind::Word => s.to_ascii_uppercase(),
        Kind::Comment => s.split_whitespace().collect::<Vec<_>>().join(" "),
        _ => s.to_string(),
    }
}

/// The first token of `before` that `after` changed beyond whitespace and
/// keyword case, if any.
fn first_change(before: &str, after: &str, engine: Option<Engine>) -> Option<String> {
    // (token, touches the previous token)
    let solid = |sql: &str| {
        let mut out: Vec<(Token, bool)> = Vec::new();
        let mut touching = false;
        for t in tokenize(sql, engine) {
            if t.kind == Kind::Space {
                touching = false;
            } else {
                out.push((t, touching));
                touching = true;
            }
        }
        out
    };
    let (a, b) = (solid(before), solid(after));
    for i in 0..a.len().max(b.len()) {
        let Some((ta, ga)) = a.get(i) else {
            return Some(after[b[i].0.range.clone()].to_string());
        };
        let near = || before[ta.range.clone()].to_string();
        let Some((tb, gb)) = b.get(i) else {
            return Some(near());
        };
        if ta.kind != tb.kind || meaning(before, ta) != meaning(after, tb) {
            return Some(near());
        }
        if i > 0 && ga != gb && joined(before, &a[i - 1].0, ta) {
            return Some(format!("{}{}", &before[a[i - 1].0.range.clone()], near()));
        }
    }
    None
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

    /// Inputs sqlformat used to change the meaning of: left alone (#142).
    #[test]
    fn never_changes_literals() {
        let ms = Some(Engine::Mssql);
        let pg = Some(Engine::Postgres);
        for (sql, engine) in [
            (
                "backup database db to disk = 'C:\\bak\\' with init; select name from t where x = 'it''s'",
                ms,
            ),
            ("select * from t where a like 'x%' escape '\\'", pg),
            ("select 0x1F", ms),
            ("select e'\\\\', 'x'", pg),
            ("select U&'d\\0061t'", pg),
            ("select B'0101'", pg),
        ] {
            for uppercase in [true, false] {
                let style = Style {
                    uppercase,
                    indent: 2,
                };
                match format(sql, engine, &style) {
                    Ok(out) => panic!("{sql:?} was formatted to {out:?}"),
                    Err(why) => assert!(why.starts_with("left unchanged"), "{why}"),
                }
            }
        }
    }

    /// A `go` line inside a string or block comment is not a batch separator.
    #[test]
    fn go_inside_strings_and_comments_is_text() {
        let sql = "select 'a\ngo\nb' as x from t\n/* c\ngo\n*/ select 2\ngo -- next\nselect 3";
        let out = format(sql, Some(Engine::Mssql), &STYLE).unwrap();
        assert!(out.contains("'a\ngo\nb'"), "{out}");
        assert!(out.contains("/* c\ngo\n*/"), "{out}");
        assert!(out.contains("\nGO -- next\n"), "{out}");
    }

    /// Ordinary queries still format: the check does not get in the way.
    #[test]
    fn everyday_queries_still_format() {
        let pg = Some(Engine::Postgres);
        let ms = Some(Engine::Mssql);
        for (sql, engine) in [
            ("select a,b from t where x=-1 and y<>2 and z>=3 or w!=4", pg),
            (
                "with c as (select id, count(*) n from o group by id) select c.*, o.name from c join o on o.id=c.id order by n desc",
                pg,
            ),
            ("select x::int, data->>'k', a || b from t -- note", pg),
            (
                "select case when a is null then 'none' else a end from t where s like 'a%' and d > now() - interval '1 day'",
                pg,
            ),
            ("select $1 from t where a = 'x' and b = $2", pg),
            (
                "select top 10 [x], N'ünï', @v from #tmp t with (nolock) where t.id in (1,2,3)",
                ms,
            ),
            ("insert into t (a, b) values (1, 'x'), (2, 'y')", None),
            ("update t set a = a + 1 where id = 5", None),
            ("select 1; select 'two'", None),
        ] {
            if let Err(why) = format(sql, engine, &STYLE) {
                panic!("{sql:?}: {why}");
            }
        }
    }
}
