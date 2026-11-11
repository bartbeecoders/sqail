//! A small T-SQL lexer: splits scripts on `GO` and recognises single DML
//! statements. It only needs to understand what can hide a `GO` or a `;`:
//! strings, quoted identifiers and comments.

use sqail_proto::go_line;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum St {
    Code,
    LineComment,
    /// Nesting depth (T-SQL block comments nest).
    Block(u32),
    Str,
    Bracket,
    Quoted,
}

/// Walk `line`, updating `st`, calling `on_code` for every char outside strings,
/// identifiers and comments.
fn scan(line: &str, st: &mut St, mut on_code: impl FnMut(usize, char)) {
    if *st == St::LineComment {
        *st = St::Code;
    }
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        let (pos, c) = chars[i];
        let next = chars.get(i + 1).map(|&(_, n)| n);
        match *st {
            St::Code => match (c, next) {
                ('-', Some('-')) => {
                    *st = St::LineComment;
                    return;
                }
                ('/', Some('*')) => {
                    *st = St::Block(1);
                    i += 1;
                }
                ('\'', _) => *st = St::Str,
                ('[', _) => *st = St::Bracket,
                ('"', _) => *st = St::Quoted,
                _ => on_code(pos, c),
            },
            St::LineComment => return,
            St::Block(depth) => match (c, next) {
                ('*', Some('/')) => {
                    *st = if depth == 1 {
                        St::Code
                    } else {
                        St::Block(depth - 1)
                    };
                    i += 1;
                }
                ('/', Some('*')) => {
                    *st = St::Block(depth + 1);
                    i += 1;
                }
                _ => {}
            },
            // Doubled quotes are escapes: leaving and re-entering is equivalent.
            St::Str if c == '\'' => *st = St::Code,
            St::Bracket if c == ']' => {
                if next == Some(']') {
                    i += 1;
                } else {
                    *st = St::Code;
                }
            }
            St::Quoted if c == '"' => *st = St::Code,
            _ => {}
        }
        i += 1;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    pub sql: String,
    /// `GO 5` runs the batch five times.
    pub repeat: u32,
    /// The script line (1-based) of the batch's first line; the server
    /// counts lines from the start of the batch.
    pub line: u32,
}

/// Split a script on lines consisting of `GO [count]` (outside comments and
/// strings), like sqlcmd and SSMS do. Empty batches are dropped.
pub fn split_go(script: &str) -> Vec<Batch> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut st = St::Code;
    let mut start = 1;
    for (n, line) in (1u32..).zip(script.split_inclusive('\n')) {
        let at_code = matches!(st, St::Code | St::LineComment);
        if at_code && let Some(repeat) = go_line(line) {
            push_batch(&mut out, std::mem::take(&mut cur), repeat, start);
            start = n + 1;
            st = St::Code;
            continue;
        }
        scan(line, &mut st, |_, _| {});
        cur.push_str(line);
    }
    push_batch(&mut out, cur, 1, start);
    out
}

fn push_batch(out: &mut Vec<Batch>, sql: String, repeat: u32, line: u32) {
    if !sql.trim().is_empty() {
        out.push(Batch { sql, repeat, line });
    }
}

/// Rough shape of a batch, enough to pick an execution strategy.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Shape {
    /// First keyword, uppercased.
    pub leading: Option<String>,
    /// Statements separated by `;` (a trailing `;` does not count).
    pub statements: usize,
    /// Mentions `OUTPUT` (DML that returns rows).
    pub has_output: bool,
    /// Every word outside strings, names and comments, uppercased.
    pub words: Vec<String>,
}

impl Shape {
    /// A lone INSERT/UPDATE/DELETE/MERGE without OUTPUT: returns no rows, so it
    /// can run in a mode that reports the affected row count.
    pub fn is_single_dml(&self) -> bool {
        self.statements == 1
            && !self.has_output
            && matches!(
                self.leading.as_deref(),
                Some("INSERT" | "UPDATE" | "DELETE" | "MERGE")
            )
    }
}

pub fn shape(sql: &str) -> Shape {
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut statements = 0usize;
    let mut pending_semicolon = false;
    let mut st = St::Code;
    let flush = |cur: &mut String, words: &mut Vec<String>| {
        if !cur.is_empty() {
            words.push(std::mem::take(cur).to_ascii_uppercase());
        }
    };
    for line in sql.split_inclusive('\n') {
        scan(line, &mut st, |_, c| {
            if c.is_alphanumeric() || c == '_' || c == '@' || c == '#' {
                if pending_semicolon || statements == 0 {
                    statements += 1;
                    pending_semicolon = false;
                }
                cur.push(c);
            } else {
                flush(&mut cur, &mut words);
                if c == ';' {
                    pending_semicolon = true;
                }
            }
        });
        flush(&mut cur, &mut words);
    }
    Shape {
        leading: words.first().cloned(),
        statements,
        has_output: words.iter().any(|w| w == "OUTPUT"),
        words,
    }
}

/// T-SQL reserved words that begin a statement of their own and cannot
/// appear inside another one. Being reserved, they are never a bare name.
const STATEMENT_WORDS: &[&str] = &[
    "BACKUP",
    "BEGIN",
    "BREAK",
    "CHECKPOINT",
    "CLOSE",
    "COMMIT",
    "CONTINUE",
    "CREATE",
    "DBCC",
    "DEALLOCATE",
    "DECLARE",
    "DENY",
    "DROP",
    "EXEC",
    "EXECUTE",
    "GOTO",
    "GRANT",
    "IF",
    "KILL",
    "OPEN",
    "PRINT",
    "RAISERROR",
    "READTEXT",
    "RECONFIGURE",
    "RESTORE",
    "RETURN",
    "REVERT",
    "REVOKE",
    "ROLLBACK",
    "SAVE",
    "SETUSER",
    "SHUTDOWN",
    "TRUNCATE",
    "UPDATETEXT",
    "USE",
    "WAITFOR",
    "WHILE",
    "WRITETEXT",
];

/// Best-effort check that `sql` is one T-SQL statement: one batch (no `GO`),
/// no `;` between statements, and no statement-only keyword after the first
/// word. T-SQL needs no separator between statements, so this cannot be
/// exact (`SELECT 1 SELECT 2` passes); it stops a second statement that
/// controls the transaction, changes the schema or runs code.
pub fn check_single_statement(sql: &str) -> Result<(), String> {
    let batches = split_go(sql);
    if batches.len() > 1 || batches.iter().any(|b| b.repeat > 1) {
        return Err("expected a single statement, got several batches (GO)".into());
    }
    let sh = shape(sql);
    if sh.statements > 1 {
        return Err("expected a single statement, got several".into());
    }
    if let Some(w) = sh
        .words
        .iter()
        .skip(1)
        .find(|w| STATEMENT_WORDS.contains(&w.as_str()))
    {
        return Err(format!(
            "expected a single statement; {w} would start another one"
        ));
    }
    Ok(())
}

/// Words that start a statement other than a query (beyond
/// [`STATEMENT_WORDS`]). Some are not reserved and could be bare names; then
/// the batch merely misses the fast path in [`single_query`].
const NON_QUERY_WORDS: &[&str] = &[
    "ALTER", "BULK", "DELETE", "DISABLE", "ENABLE", "FETCH", "GET", "INSERT", "MERGE", "MOVE",
    "RECEIVE", "SEND", "SET", "THROW", "UPDATE",
];

/// Words before `(` that make the parenthesis a subquery or a list rather
/// than the start of another statement.
const PAREN_OK_AFTER: &[&str] = &[
    "ALL",
    "AND",
    "ANY",
    "APPLY",
    "AS",
    "BETWEEN",
    "BY",
    "CASE",
    "DISTINCT",
    "ELSE",
    "EXCEPT",
    "EXISTS",
    "FROM",
    "HAVING",
    "IN",
    "INTERSECT",
    "IS",
    "JOIN",
    "LIKE",
    "NOT",
    "ON",
    "OR",
    "SELECT",
    "SOME",
    "THEN",
    "TOP",
    "UNION",
    "WHEN",
    "WHERE",
];

#[derive(Debug, PartialEq, Eq)]
enum QTok {
    Word(String),
    Open,
    Close,
    Semi,
    Punct,
}

/// Conservative check that a batch is exactly one query (`SELECT ...` or
/// `WITH ... SELECT ...`) and nothing after it. T-SQL needs no separator
/// between statements, so anything that might begin a second statement
/// fails the check. Such a batch can be stopped once its rows reach
/// `max_rows` without skipping work the script still had to do.
pub fn single_query(sql: &str) -> bool {
    // The code with strings, names and comments blanked out.
    let mut code = String::with_capacity(sql.len());
    let mut st = St::Code;
    for line in sql.split_inclusive('\n') {
        let mut buf = vec![' '; line.len()];
        scan(line, &mut st, |pos, c| {
            buf[pos] = if c.is_ascii() { c } else { 'x' };
        });
        // Blanked strings and names still separate words.
        code.extend(buf);
        code.push(' ');
    }
    let mut toks: Vec<(QTok, u32)> = Vec::new();
    let mut depth = 0u32;
    let mut word = String::new();
    let flush = |word: &mut String, toks: &mut Vec<(QTok, u32)>, depth: u32| {
        if !word.is_empty() {
            toks.push((QTok::Word(std::mem::take(word).to_ascii_uppercase()), depth));
        }
    };
    for c in code.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '_' | '@' | '#' | '$') {
            word.push(c);
            continue;
        }
        flush(&mut word, &mut toks, depth);
        match c {
            '(' => {
                toks.push((QTok::Open, depth));
                depth += 1;
            }
            ')' => {
                depth = depth.saturating_sub(1);
                toks.push((QTok::Close, depth));
            }
            ';' => toks.push((QTok::Semi, depth)),
            _ if c.is_whitespace() => {}
            _ => toks.push((QTok::Punct, depth)),
        }
    }
    flush(&mut word, &mut toks, depth);

    let w = |i: usize| match toks.get(i) {
        Some((QTok::Word(w), _)) => Some(w.as_str()),
        _ => None,
    };
    let leading = w(0);
    if !matches!(leading, Some("SELECT" | "WITH")) {
        return false;
    }
    let mut selects = usize::from(leading == Some("SELECT"));
    for (i, (tok, d)) in toks.iter().enumerate().skip(1) {
        if *d > 0 {
            continue;
        }
        let prev = &toks[i - 1].0;
        match tok {
            QTok::Semi => {
                if toks[i + 1..].iter().any(|(t, _)| *t != QTok::Semi) {
                    return false;
                }
            }
            QTok::Open => {
                let subquery = matches!(w(i + 1), Some("SELECT" | "WITH"));
                let ok = match prev {
                    QTok::Word(p) => PAREN_OK_AFTER.contains(&p.as_str()),
                    QTok::Punct | QTok::Open => true,
                    QTok::Close | QTok::Semi => false,
                };
                if subquery && !ok {
                    return false;
                }
            }
            QTok::Word(word) => {
                let word = word.as_str();
                if STATEMENT_WORDS.contains(&word) || NON_QUERY_WORDS.contains(&word) {
                    return false;
                }
                match word {
                    "SELECT" => {
                        let combined = matches!(prev, QTok::Word(p)
                            if matches!(p.as_str(), "UNION" | "ALL" | "EXCEPT" | "INTERSECT"));
                        if !combined {
                            selects += 1;
                        }
                    }
                    // Table hints, TOP ... WITH TIES, GROUP BY ... WITH ROLLUP.
                    "WITH" => {
                        let hint = matches!(toks.get(i + 1), Some((QTok::Open, _)))
                            || matches!(w(i + 1), Some("TIES" | "ROLLUP" | "CUBE"));
                        if !hint {
                            return false;
                        }
                    }
                    _ => {}
                }
            }
            QTok::Close | QTok::Punct => {}
        }
    }
    selects == 1 && depth == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_query_check() {
        for ok in [
            "SELECT * FROM t",
            "select a, b from t where c = 'x; DELETE' -- UPDATE\n;",
            "SELECT TOP 5 WITH TIES a FROM t WITH (NOLOCK) ORDER BY a",
            "SELECT a FROM t UNION ALL SELECT b FROM u EXCEPT SELECT c FROM v",
            "WITH c AS (SELECT 1 AS a), d AS (SELECT 2 AS a) SELECT a FROM c",
            "SELECT (SELECT 1), x FROM (SELECT 2 AS x) s WHERE x IN (SELECT 3)",
            "SELECT [update], \"set\" FROM t",
            "SELECT CASE WHEN a = 1 THEN 'x' END FROM t",
        ] {
            assert!(single_query(ok), "{ok}");
        }
        for bad in [
            "SELECT 1\nSELECT 2",
            "SELECT 1; SELECT 2",
            "SELECT * FROM t\nUPDATE t SET a = 1",
            "SELECT * FROM t\nEXEC p",
            "SELECT * FROM t\nPRINT 'x'",
            "SELECT * FROM t\nCOMMIT",
            "SELECT 1\nWITH c AS (SELECT 1) SELECT * FROM c",
            "SELECT a FROM t\n(SELECT 2)",
            "WITH c AS (SELECT 1 AS a) INSERT INTO t SELECT a FROM c",
            "INSERT INTO t SELECT 1",
            "EXEC p",
            "(SELECT 1)",
            "SELECT 1 INSERT INTO t VALUES (1)",
        ] {
            assert!(!single_query(bad), "{bad}");
        }
    }

    fn sqls(script: &str) -> Vec<String> {
        split_go(script)
            .into_iter()
            .map(|b| b.sql.trim().to_string())
            .collect()
    }

    #[test]
    fn splits_on_go_lines() {
        assert_eq!(
            sqls("SELECT 1\nGO\nSELECT 2\n go \nSELECT 3"),
            ["SELECT 1", "SELECT 2", "SELECT 3"]
        );
    }

    #[test]
    fn go_with_count_and_comment() {
        let b = split_go("PRINT 'x'\nGO 3 -- thrice\n");
        assert_eq!(
            b,
            vec![Batch {
                sql: "PRINT 'x'\n".into(),
                repeat: 3,
                line: 1,
            }]
        );
    }

    #[test]
    fn batches_know_their_first_script_line() {
        let lines: Vec<u32> = split_go("SELECT 1\nGO\n\n\nSELECT\n * FROM x\nGO\nGO\nSELECT 3")
            .iter()
            .map(|b| b.line)
            .collect();
        assert_eq!(lines, [1, 3, 9]);
    }

    #[test]
    fn ignores_go_inside_strings_and_comments() {
        let script = "SELECT 'a\nGO\nb'\n/* x\nGO\n*/\nSELECT [go\nGO\n]\nGO\nSELECT 2";
        let b = sqls(script);
        assert_eq!(b.len(), 2, "{b:?}");
        assert!(b[0].starts_with("SELECT 'a"));
        assert_eq!(b[1], "SELECT 2");
    }

    #[test]
    fn nested_block_comments() {
        assert_eq!(sqls("/* a /* b */\nGO\n*/ SELECT 1\nGO\nSELECT 2").len(), 2);
    }

    #[test]
    fn words_starting_with_go_are_not_separators() {
        assert_eq!(sqls("SELECT 1\nGOTO label\nGO"), ["SELECT 1\nGOTO label"]);
    }

    #[test]
    fn line_comment_ends_at_newline() {
        assert_eq!(
            sqls("SELECT 1 -- note\nGO\nSELECT 2"),
            ["SELECT 1 -- note", "SELECT 2"]
        );
    }

    #[test]
    fn multibyte_line_starts_do_not_panic() {
        // Found by sqail-fuzz: slicing at byte 2 split a multi-byte char.
        assert_eq!(sqls("世界\nGO\nSELECT 1"), ["世界", "SELECT 1"]);
        assert_eq!(split_go("😀").len(), 1);
        assert_eq!(split_go("g").len(), 1);
    }

    #[test]
    fn shape_detects_single_dml() {
        assert!(shape("  update t set a = 1 where b = 'x;y';").is_single_dml());
        assert!(shape("-- c\nDELETE FROM t").is_single_dml());
        assert!(!shape("UPDATE t SET a = 1; SELECT * FROM t").is_single_dml());
        assert!(!shape("UPDATE t SET a = 1 OUTPUT inserted.a").is_single_dml());
        assert!(!shape("SELECT 1").is_single_dml());
        assert_eq!(shape("SELECT 1; SELECT 2;").statements, 2);
    }

    #[test]
    fn single_statement_check() {
        for ok in [
            "SELECT 1",
            "SELECT 1;",
            "UPDATE t SET a = 1 WHERE b = 'x; COMMIT'",
            "BEGIN TRANSACTION",
            "SELECT [commit] FROM t -- ROLLBACK",
            "WITH c AS (SELECT 1 AS a) SELECT a FROM c UNION SELECT 2",
        ] {
            assert_eq!(check_single_statement(ok), Ok(()), "{ok}");
        }
        for bad in [
            "SELECT 1; SELECT 2",
            "SELECT 1\nGO\nSELECT 2",
            "SELECT 1\nGO 2",
            "SELECT 1 COMMIT",
            "SELECT 1 ROLLBACK TRANSACTION",
            "SELECT 1 BEGIN TRAN",
            "SELECT 1 DROP TABLE t",
            "SELECT 1 EXEC sp_who",
            "SELECT 1 IF 1 = 1 PRINT 'x'",
            "SELECT 'a''' COMMIT",
            "SELECT 1 /* /* */ */ COMMIT",
        ] {
            assert!(check_single_statement(bad).is_err(), "{bad}");
        }
    }
}
