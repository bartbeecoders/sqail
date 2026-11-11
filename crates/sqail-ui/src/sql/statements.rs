//! Statement boundaries in an editor buffer, for "run statement at cursor".
//!
//! A statement ends at `;`, at a `GO` line (SQL Server), or at a blank line,
//! except inside parentheses or a `BEGIN … END` / `CASE … END` block, so
//! procedure and trigger bodies stay whole. In T-SQL a module
//! (`CREATE|ALTER [OR ALTER] PROC|FUNCTION|TRIGGER|VIEW`) runs to the next
//! `GO` whether or not its body has `BEGIN … END`, so `;` and blank lines
//! inside it don't end it. So does a batch from a `DECLARE` on: its
//! variables only exist within the batch.
//!
//! An unclosed `(` or `CASE` must not swallow the rest of the buffer: a blank
//! line followed by a word that starts a statement (`SELECT`, `DELETE`, …)
//! ends it anyway, unless the text before the blank line clearly continues
//! (`(`, `,`, `UNION`, `IN`, …). In T-SQL, `IF … ; ELSE` stays one statement.

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
    // `BEGIN … END` blocks, and open `CASE`s (always the innermost).
    let mut blocks = 0i32;
    let mut cases = 0i32;
    let mssql = engine == Some(Engine::Mssql);
    // The last token that is not space or a comment.
    let mut prev_sig: Option<usize> = None;
    let mut prev_word: Option<String> = None;
    // The first words of the current statement, to spot a T-SQL module.
    let mut head: Vec<String> = Vec::new();
    let mut module = false;

    let close = |start: &mut Option<usize>, end: usize, out: &mut Vec<Range<usize>>| {
        if let Some(s) = start.take()
            && end > s
        {
            out.push(s..end);
        }
    };

    // The rest of a `GO` line (a count, a comment) belongs to no statement.
    let mut skip_until = 0usize;
    for (idx, t) in tokens.iter().enumerate() {
        if t.range.start < skip_until {
            continue;
        }
        let s = &text[t.range.clone()];
        if !matches!(t.kind, Kind::Space | Kind::Comment) {
            prev_sig = Some(idx);
        }
        match t.kind {
            Kind::Space => {
                let blank_line = s.matches('\n').count() >= 2;
                if !blank_line || module {
                    continue;
                }
                let next = next_word(text, &tokens[idx + 1..]);
                if (parens > 0 || cases > 0)
                    && next.as_deref().is_some_and(starts_statement)
                    && prev_sig.is_some_and(|p| ends_expression(text, &tokens[p]))
                {
                    // An unclosed `(` or `CASE` before a new statement.
                    parens = 0;
                    cases = 0;
                }
                let else_follows = mssql && next.as_deref() == Some("ELSE");
                if parens == 0 && blocks + cases <= 0 && !else_follows {
                    close(&mut start, end, &mut out);
                    head.clear();
                }
                continue;
            }
            Kind::Semicolon
                if parens <= 0
                    && blocks + cases <= 0
                    && !module
                    // T-SQL `IF … ; ELSE …` is one statement.
                    && !(mssql && next_word(text, &tokens[idx + 1..]).as_deref() == Some("ELSE")) =>
            {
                // `BEGIN;` in Postgres starts a transaction, not a block.
                close(&mut start, end, &mut out);
                prev_word = None;
                head.clear();
                continue;
            }
            Kind::Word if mssql && is_go_line(text, &t.range) => {
                close(&mut start, end, &mut out);
                blocks = 0;
                cases = 0;
                parens = 0;
                module = false;
                head.clear();
                prev_word = None;
                skip_until = text[t.range.end..]
                    .find('\n')
                    .map_or(text.len(), |p| t.range.end + p);
                continue;
            }
            Kind::Open => parens += 1,
            // A stray `)` must not turn off splitting for the rest.
            Kind::Close => parens = (parens - 1).max(0),
            Kind::Word => {
                let upper = s.to_ascii_uppercase();
                match upper.as_str() {
                    "CASE" => cases += 1,
                    "BEGIN" => {
                        let next = next_word(text, &tokens[idx + 1..]);
                        let is_tx = next.as_deref().is_none_or(is_transaction_word)
                            || prev_word.as_deref() == Some("END");
                        if !is_tx {
                            blocks += 1;
                        }
                    }
                    "END" if cases > 0 => cases -= 1,
                    "END" if blocks > 0 => blocks -= 1,
                    _ => {}
                }
                if mssql && !module && head.len() < 4 {
                    head.push(upper.clone());
                    module = is_tsql_module(&head);
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

/// Whether a statement starting with `head` is a T-SQL module definition,
/// whose body runs to the next `GO`, or a `DECLARE`, whose variables the rest
/// of the batch needs.
fn is_tsql_module(head: &[String]) -> bool {
    let rest = match head {
        [first, ..] if first == "DECLARE" => return true,
        [first, rest @ ..] if first == "CREATE" || first == "ALTER" => rest,
        _ => return false,
    };
    let rest = match rest {
        [or, alter, rest @ ..] if or == "OR" && alter == "ALTER" => rest,
        _ => rest,
    };
    matches!(
        rest.first().map(String::as_str),
        Some("PROC" | "PROCEDURE" | "FUNCTION" | "TRIGGER" | "VIEW")
    )
}

/// Words that can follow `BEGIN` when it starts a transaction rather than a
/// block: T-SQL `BEGIN TRAN|DISTRIBUTED`, SQLite `BEGIN IMMEDIATE|DEFERRED|
/// EXCLUSIVE`, Postgres `BEGIN WORK|ISOLATION …|READ ONLY|[NOT] DEFERRABLE`.
fn is_transaction_word(word: &str) -> bool {
    matches!(
        word,
        "TRAN"
            | "TRANSACTION"
            | "WORK"
            | "DISTRIBUTED"
            | "ISOLATION"
            | "IMMEDIATE"
            | "DEFERRED"
            | "EXCLUSIVE"
            | "READ"
            | "NOT"
            | "DEFERRABLE"
            // T-SQL Service Broker: `BEGIN DIALOG [CONVERSATION]`,
            // `BEGIN CONVERSATION TIMER`.
            | "DIALOG"
            | "CONVERSATION"
    )
}

/// Words that start a statement, for ending an unclosed `(` or `CASE` at a
/// blank line.
fn starts_statement(word: &str) -> bool {
    matches!(
        word,
        "SELECT"
            | "INSERT"
            | "UPDATE"
            | "DELETE"
            | "MERGE"
            | "CREATE"
            | "ALTER"
            | "DROP"
            | "TRUNCATE"
            | "DECLARE"
            | "SET"
            | "EXEC"
            | "EXECUTE"
            | "IF"
            | "WHILE"
            | "PRINT"
            | "USE"
            | "GRANT"
            | "REVOKE"
            | "COMMIT"
            | "ROLLBACK"
    )
}

/// Whether `t` can be the last token of a statement, rather than one that
/// asks for more (`(`, `,`, an operator, `UNION`, `IN`, …).
fn ends_expression(text: &str, t: &super::lex::Token) -> bool {
    match t.kind {
        Kind::Number | Kind::String | Kind::QuotedIdent | Kind::Variable | Kind::Close => true,
        Kind::Semicolon => true,
        Kind::Word => !matches!(
            text[t.range.clone()].to_ascii_uppercase().as_str(),
            "AND"
                | "OR"
                | "NOT"
                | "IN"
                | "EXISTS"
                | "ANY"
                | "SOME"
                | "ALL"
                | "UNION"
                | "EXCEPT"
                | "INTERSECT"
                | "MINUS"
                | "AS"
                | "THEN"
                | "ELSE"
                | "WHEN"
                | "ON"
                | "SELECT"
                | "FROM"
                | "WHERE"
                | "BY"
                | "IS"
                | "LIKE"
                | "BETWEEN"
                | "JOIN"
                | "WITH"
                | "VALUES"
                | "INTO"
                | "SET"
                | "RETURN"
                | "RETURNS"
                | "BEGIN"
        ),
        _ => false,
    }
}

fn next_word(text: &str, rest: &[super::lex::Token]) -> Option<String> {
    rest.iter()
        .find(|t| !matches!(t.kind, Kind::Space | Kind::Comment))
        .and_then(|t| match t.kind {
            Kind::Word => Some(text[t.range.clone()].to_ascii_uppercase()),
            _ => None,
        })
}

/// Whether the word token at `range` starts a `GO` line: first on its line,
/// and the line is a batch separator by the same rule the service uses
/// ([`sqail_client::proto::go_line`]: `GO`, `GO n` with n > 0, a comment).
pub(super) fn is_go_line(text: &str, range: &Range<usize>) -> bool {
    let line_start = text[..range.start].rfind('\n').map_or(0, |p| p + 1);
    let line_end = text[range.end..]
        .find('\n')
        .map_or(text.len(), |p| range.end + p);
    text[line_start..range.start].trim().is_empty()
        && sqail_client::proto::go_line(&text[line_start..line_end]).is_some()
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
    fn tsql_modules_run_to_go() {
        let e = Some(Engine::Mssql);
        // No BEGIN … END: the `;` must not end the procedure.
        let t = "CREATE PROCEDURE p AS\nSELECT 1;\nSELECT 2;\nGO\nEXEC p";
        assert_eq!(
            stmts(t, e),
            ["CREATE PROCEDURE p AS\nSELECT 1;\nSELECT 2;", "EXEC p"]
        );
        // A blank line inside the body.
        let t = "ALTER PROC p AS\nSELECT 1\n\nSELECT 2\nGO\nEXEC p";
        assert_eq!(
            stmts(t, e),
            ["ALTER PROC p AS\nSELECT 1\n\nSELECT 2", "EXEC p"]
        );
        // CREATE OR ALTER, lowercase, functions/triggers/views, to end of buffer.
        for kind in ["function f()", "trigger tr ON t AFTER INSERT", "view v"] {
            let t = format!("create or alter {kind} AS\nSELECT 1;\n\nSELECT 2;");
            assert_eq!(stmts(&t, e).len(), 1, "{kind}");
        }
        // Statements after GO split normally again.
        let t = "CREATE VIEW v AS SELECT 1;\nGO\nSELECT 1;\nSELECT 2";
        assert_eq!(stmts(t, e).len(), 3);
        // Other engines and other CREATE statements are unaffected.
        assert_eq!(stmts("CREATE TABLE t (a int);\nSELECT 1", e).len(), 2);
        let t = "CREATE VIEW v AS SELECT 1;\nSELECT 2";
        assert_eq!(stmts(t, Some(Engine::Postgres)).len(), 2);
        // The cursor in the second body statement still picks the whole module.
        let t = "CREATE PROCEDURE p AS\nSELECT 1;\n\nSELECT 2;\nGO";
        assert_eq!(at(t, e, t.find("2").unwrap()).unwrap().start, 0);
    }

    #[test]
    fn begin_transaction_is_not_a_block() {
        let t = "BEGIN TRANSACTION\n\nUPDATE t SET a = 1\n\nCOMMIT";
        assert_eq!(stmts(t, Some(Engine::Mssql)).len(), 3);
        assert_eq!(stmts("BEGIN;\n\nSELECT 1", Some(Engine::Postgres)).len(), 2);
    }

    #[test]
    fn begin_transaction_modes_are_not_blocks() {
        for (begin, engine) in [
            ("BEGIN IMMEDIATE;", Engine::Sqlite),
            ("BEGIN DEFERRED TRANSACTION;", Engine::Sqlite),
            ("BEGIN EXCLUSIVE;", Engine::Sqlite),
            ("begin immediate;", Engine::Sqlite),
            ("BEGIN READ ONLY;", Engine::Postgres),
            ("BEGIN READ WRITE;", Engine::Postgres),
            ("BEGIN ISOLATION LEVEL SERIALIZABLE;", Engine::Postgres),
            ("BEGIN NOT DEFERRABLE;", Engine::Postgres),
            ("BEGIN DEFERRABLE;", Engine::Postgres),
        ] {
            let t = format!("{begin}\nUPDATE t SET a = 1;\nCOMMIT;\nSELECT 1");
            assert_eq!(
                stmts(&t, Some(engine)),
                [
                    begin.trim_end_matches(';'),
                    "UPDATE t SET a = 1",
                    "COMMIT",
                    "SELECT 1"
                ],
                "{begin}"
            );
        }
    }

    #[test]
    fn begin_try_catch_are_blocks() {
        let body = "BEGIN TRY\n  SELECT 1;\n\n  SELECT 2;\nEND TRY\nBEGIN CATCH\n  SELECT 3;\n\n  SELECT 4;\nEND CATCH";
        let t = format!("{body}\nGO\nSELECT 5");
        assert_eq!(stmts(&t, Some(Engine::Mssql)), [body, "SELECT 5"]);
    }

    #[test]
    fn go_ends_unbalanced_parens_and_blocks() {
        let ms = Some(Engine::Mssql);
        let t = "SELECT (1\nGO\nSELECT 2;\nSELECT 3\n\nSELECT 4";
        assert_eq!(
            stmts(t, ms),
            ["SELECT (1", "SELECT 2", "SELECT 3", "SELECT 4"]
        );
        let t = "IF 1 = 1\nBEGIN\n  SELECT 1\nGO\nSELECT 2;\nSELECT 3";
        assert_eq!(
            stmts(t, ms),
            ["IF 1 = 1\nBEGIN\n  SELECT 1", "SELECT 2", "SELECT 3"]
        );
        // `GO n`, lowercase `go`, and GO inside a string or comment.
        let t = "SELECT '\nGO\n' AS a -- GO\nFROM t\nGO 5\nSELECT 1\ngo\nSELECT 2";
        assert_eq!(
            stmts(t, ms),
            ["SELECT '\nGO\n' AS a -- GO\nFROM t", "SELECT 1", "SELECT 2"]
        );
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
    fn dialect_strings_and_comments() {
        assert_eq!(
            stmts(r"SELECT E'a\'b'; SELECT 1", Some(Engine::Postgres)),
            [r"SELECT E'a\'b'", "SELECT 1"]
        );
        let t = "SELECT 1 /* a /* b */ ; SELECT 2";
        assert_eq!(
            stmts(t, Some(Engine::Sqlite)),
            ["SELECT 1 /* a /* b */", "SELECT 2"]
        );
        assert_eq!(stmts(t, Some(Engine::Postgres)), [t]);
    }

    #[test]
    fn go_lines() {
        let ms = Some(Engine::Mssql);
        assert_eq!(
            stmts("SELECT 1\nGO 5\nSELECT 2", ms),
            ["SELECT 1", "SELECT 2"]
        );
        assert_eq!(
            stmts("SELECT 1\nGO -- next\nSELECT 2", ms),
            ["SELECT 1", "SELECT 2"]
        );
        let t = "SELECT 'a\nGO\nb'\n/* x\nGO\n*/ SELECT 2";
        assert_eq!(stmts(t, ms), [t]);
    }

    #[test]
    fn if_else_is_one_statement() {
        let ms = Some(Engine::Mssql);
        let t = "IF 1 = 1\n    SELECT 1;\nELSE\n    SELECT 2;\nSELECT 3";
        let body = "IF 1 = 1\n    SELECT 1;\nELSE\n    SELECT 2";
        assert_eq!(stmts(t, ms), [body, "SELECT 3"]);
        assert_eq!(&t[at(t, ms, t.find("ELSE").unwrap()).unwrap()], body);
        let t = "IF 1 = 1\nBEGIN\n  SELECT 1;\nEND;\n\nELSE\nBEGIN\n  SELECT 2;\nEND";
        assert_eq!(stmts(t, ms), [t]);
        // Elsewhere ELSE is not special (it isn't valid there).
        assert_eq!(stmts("SELECT 1;\nELSE", None).len(), 2);
    }

    #[test]
    fn unclosed_parens_and_case_end_before_a_new_statement() {
        for e in [None, Some(Engine::Postgres), Some(Engine::Mssql)] {
            let t = "SELECT f(1;\n\nSELECT 2;\n\nDELETE FROM t";
            assert_eq!(
                &t[at(t, e, t.find('2').unwrap()).unwrap()],
                "SELECT 2",
                "{e:?}"
            );
            assert_eq!(stmts(t, e).last(), Some(&"DELETE FROM t"), "{e:?}");
            let t = "SELECT CASE WHEN a THEN 1\n\nSELECT 2;\nDELETE FROM t;";
            assert_eq!(
                stmts(t, e),
                ["SELECT CASE WHEN a THEN 1", "SELECT 2", "DELETE FROM t"],
                "{e:?}"
            );
        }
        // A blank line inside parentheses that clearly continue is kept.
        let t = "SELECT * FROM t WHERE a IN (\n\nSELECT b FROM u\n)";
        assert_eq!(stmts(t, None), [t]);
        let t = "SELECT * FROM (SELECT 1 UNION\n\nSELECT 2) x";
        assert_eq!(stmts(t, None), [t]);
        // A blank line inside a BEGIN … END block is kept.
        let t = "BEGIN TRY\n  SELECT 1\n\n  SELECT 2\nEND TRY";
        assert_eq!(stmts(t, Some(Engine::Mssql)), [t]);
        // A stray `)` does not turn off blank-line splitting.
        assert_eq!(stmts("SELECT 1)\n\nSELECT 2\n\nSELECT 3", None).len(), 3);
    }

    #[test]
    fn begin_dialog_is_not_a_block() {
        let ms = Some(Engine::Mssql);
        let t = "BEGIN DIALOG @h FROM SERVICE a TO SERVICE 'b';\nSELECT 1\n\nSELECT 2";
        assert_eq!(stmts(t, ms).len(), 3);
        let t = "BEGIN CONVERSATION TIMER (@h) TIMEOUT = 5;\nSELECT 1";
        assert_eq!(stmts(t, ms).len(), 2);
    }

    #[test]
    fn declare_keeps_the_batch_together() {
        let ms = Some(Engine::Mssql);
        let t = "SELECT 0;\nDECLARE @x int = 5\n\nSELECT @x;\n\nSELECT @x + 1\nGO\nSELECT 2";
        let batch = "DECLARE @x int = 5\n\nSELECT @x;\n\nSELECT @x + 1";
        assert_eq!(stmts(t, ms), ["SELECT 0", batch, "SELECT 2"]);
        assert_eq!(&t[at(t, ms, t.find("@x;").unwrap()).unwrap()], batch);
        // Not for other engines.
        let t = "DECLARE c CURSOR FOR SELECT 1;\nSELECT 2";
        assert_eq!(stmts(t, Some(Engine::Postgres)).len(), 2);
    }

    #[test]
    fn go_lines_follow_the_service_rule() {
        let ms = Some(Engine::Mssql);
        // `GO 0` is not a separator for the service, so not here either.
        assert_eq!(stmts("SELECT 1\nGO 0\nSELECT 2", ms).len(), 1);
        assert_eq!(stmts("SELECT 1\nGO 2 -- twice\nSELECT 2", ms).len(), 2);
        assert_eq!(stmts("SELECT 1\nGO2\nSELECT 2", ms).len(), 2);
        assert_eq!(stmts("SELECT 1\nGOTO x\nSELECT 2", ms).len(), 1);
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
