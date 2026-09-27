//! Statement boundaries in an editor buffer, for "run statement at cursor".
//!
//! A statement ends at `;`, at a `GO` line (SQL Server), or at a blank line,
//! except inside parentheses or a `BEGIN … END` / `CASE … END` block, so
//! procedure and trigger bodies stay whole.

use std::ops::Range;

use sqail_client::proto::Engine;

use super::lex::{Kind, tokenize};

/// Byte ranges of the statements in `text`, trimmed, without the separator.
pub fn split(text: &str, engine: Option<Engine>) -> Vec<Range<usize>> {
    let tokens = tokenize(text, engine);
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let mut end = 0usize;
    let mut parens = 0i32;
    let mut blocks = 0i32;
    let mut prev_word: Option<String> = None;

    let close = |start: &mut Option<usize>, end: usize, out: &mut Vec<Range<usize>>| {
        if let Some(s) = start.take()
            && end > s
        {
            out.push(s..end);
        }
    };

    for (idx, t) in tokens.iter().enumerate() {
        let s = &text[t.range.clone()];
        match t.kind {
            Kind::Space => {
                let blank_line = s.matches('\n').count() >= 2;
                if blank_line && parens == 0 && blocks <= 0 {
                    close(&mut start, end, &mut out);
                }
                continue;
            }
            Kind::Semicolon if parens <= 0 && blocks <= 0 => {
                // `BEGIN;` in Postgres starts a transaction, not a block.
                close(&mut start, end, &mut out);
                prev_word = None;
                continue;
            }
            Kind::Word
                if engine == Some(Engine::Mssql)
                    && s.eq_ignore_ascii_case("GO")
                    && is_alone_on_line(text, &t.range) =>
            {
                close(&mut start, end, &mut out);
                blocks = 0;
                continue;
            }
            Kind::Open => parens += 1,
            Kind::Close => parens -= 1,
            Kind::Word => {
                let upper = s.to_ascii_uppercase();
                match upper.as_str() {
                    "CASE" => blocks += 1,
                    "BEGIN" => {
                        let next = next_word(text, &tokens[idx + 1..]);
                        let is_tx = matches!(
                            next.as_deref(),
                            Some("TRAN" | "TRANSACTION" | "WORK" | "DISTRIBUTED" | "ISOLATION")
                                | None
                        ) || prev_word.as_deref() == Some("END");
                        if !is_tx {
                            blocks += 1;
                        }
                    }
                    "END" if blocks > 0 => blocks -= 1,
                    _ => {}
                }
                prev_word = Some(upper);
            }
            _ => {}
        }
        if start.is_none() {
            start = Some(t.range.start);
        }
        end = t.range.end;
    }
    close(&mut start, end, &mut out);
    // Leading comments belong to the statement; trailing-only comment chunks
    // are still "statements" (harmless to run), which keeps this simple.
    out
}

fn next_word(text: &str, rest: &[super::lex::Token]) -> Option<String> {
    rest.iter()
        .find(|t| !matches!(t.kind, Kind::Space | Kind::Comment))
        .and_then(|t| match t.kind {
            Kind::Word => Some(text[t.range.clone()].to_ascii_uppercase()),
            _ => None,
        })
}

fn is_alone_on_line(text: &str, range: &Range<usize>) -> bool {
    let line_start = text[..range.start].rfind('\n').map_or(0, |p| p + 1);
    let line_end = text[range.end..]
        .find('\n')
        .map_or(text.len(), |p| range.end + p);
    let before = text[line_start..range.start].trim();
    let after = text[range.end..line_end].trim();
    let after = after.split("--").next().unwrap_or("").trim();
    before.is_empty() && (after.is_empty() || after.chars().all(|c| c.is_ascii_digit()))
}

/// The statement to run for a cursor at byte offset `cursor`: the one that
/// contains it, else the closest one before it, else the first after it.
pub fn at(text: &str, engine: Option<Engine>, cursor: usize) -> Option<Range<usize>> {
    let stmts = split(text, engine);
    if let Some(s) = stmts.iter().find(|s| s.start <= cursor && cursor <= s.end) {
        return Some(s.clone());
    }
    // Cursor after a statement on the same line (e.g. after the `;`).
    let before = stmts.iter().rev().find(|s| s.end <= cursor);
    if let Some(b) = before
        && !text[b.end..cursor].contains('\n')
    {
        return Some(b.clone());
    }
    stmts.iter().find(|s| s.start >= cursor).or(before).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stmts(text: &str, e: Option<Engine>) -> Vec<&str> {
        split(text, e).into_iter().map(|r| &text[r]).collect()
    }

    #[test]
    fn splits_on_semicolons_and_blank_lines() {
        let t = "SELECT 1; SELECT 'a;b';\n\nSELECT 3\nFROM t\n\nSELECT 4";
        assert_eq!(
            stmts(t, None),
            ["SELECT 1", "SELECT 'a;b'", "SELECT 3\nFROM t", "SELECT 4"]
        );
    }

    #[test]
    fn blocks_survive_blank_lines() {
        let t = "CREATE PROCEDURE p AS\nBEGIN\n  SELECT 1;\n\n  SELECT 2;\nEND\nGO\nEXEC p";
        let s = stmts(t, Some(Engine::Mssql));
        assert_eq!(s.len(), 2, "{s:?}");
        // SQLite triggers use BEGIN … END with semicolons inside.
        let t = "CREATE TRIGGER x AFTER INSERT ON t BEGIN\n  UPDATE a SET b = 1;\n  DELETE FROM c;\nEND;\nSELECT 1";
        assert_eq!(stmts(t, Some(Engine::Sqlite)).len(), 2);
        let t = "CREATE PROCEDURE p AS\nBEGIN\n  SELECT 1\n\n  SELECT 2\nEND\nGO\nEXEC p";
        let s = stmts(t, Some(Engine::Mssql));
        assert_eq!(
            s,
            [
                "CREATE PROCEDURE p AS\nBEGIN\n  SELECT 1\n\n  SELECT 2\nEND",
                "EXEC p"
            ]
        );
    }

    #[test]
    fn begin_transaction_is_not_a_block() {
        let t = "BEGIN TRANSACTION\n\nUPDATE t SET a = 1\n\nCOMMIT";
        assert_eq!(stmts(t, Some(Engine::Mssql)).len(), 3);
        assert_eq!(stmts("BEGIN;\n\nSELECT 1", Some(Engine::Postgres)).len(), 2);
    }

    #[test]
    fn case_end_is_balanced() {
        let t = "SELECT CASE WHEN a THEN 1 END\n\nFROM t";
        assert_eq!(stmts(t, None).len(), 2);
    }

    #[test]
    fn dollar_quoted_bodies_are_whole() {
        let t = "CREATE FUNCTION f() RETURNS int AS $$\nSELECT 1;\n\nSELECT 2;\n$$ LANGUAGE sql;\nSELECT f()";
        assert_eq!(stmts(t, Some(Engine::Postgres)).len(), 2);
    }

    #[test]
    fn statement_at_cursor() {
        let t = "SELECT 1;\nSELECT 2;\n\n\nSELECT 3";
        let pick = |c| &t[at(t, None, c).unwrap()];
        assert_eq!(pick(0), "SELECT 1");
        assert_eq!(pick(9), "SELECT 1"); // right after ';'
        assert_eq!(pick(12), "SELECT 2");
        assert_eq!(pick(t.len()), "SELECT 3");
        assert_eq!(pick(21), "SELECT 3"); // in the blank lines: next statement
    }
}
