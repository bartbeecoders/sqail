//! Refuses SQL that would switch a read-only PostgreSQL profile to
//! read-write.
//!
//! Read-only profiles run with `default_transaction_read_only = on`, which
//! any statement could turn off again. This check is a second line of
//! defence, not a sandbox: a function the role may call can still change
//! settings. Real enforcement needs a database role without write rights.
//!
//! The T-SQL lexer in `split` does not know PostgreSQL's dollar quotes and
//! escape strings, which could hide a statement from it, so this module has
//! its own. Where the lexing depends on server settings
//! (`standard_conforming_strings`), the script is lexed both ways and must
//! pass both.
//!
//! The same lexer splits scripts into statements the way psql does, so that
//! each one runs (and commits) on its own.

/// A token, as far as the check needs one.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    /// Unquoted word, lowercased.
    Word(String),
    /// `"quoted"` identifier, lowercased (setting names ignore case).
    Quoted(String),
    /// `U&"..."` identifier: may spell anything through escapes.
    UnicodeIdent,
    Str,
    Semi,
    Open,
    Close,
    Other,
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || !c.is_ascii()
}

fn is_ident_char(c: char) -> bool {
    is_ident_start(c) || c.is_ascii_digit() || c == '$'
}

fn lex(sql: &str, backslash_strings: bool) -> Vec<Tok> {
    lex_spans(sql, backslash_strings)
        .into_iter()
        .map(|(t, _)| t)
        .collect()
}

/// Tokenize `sql`, with the index of the character each token starts at.
/// `backslash_strings`: treat `\` as an escape in plain `'...'` strings too,
/// as the server does with `standard_conforming_strings = off`.
fn lex_spans(sql: &str, backslash_strings: bool) -> Vec<(Tok, usize)> {
    let c: Vec<char> = sql.chars().collect();
    let n = c.len();
    let mut out = Vec::new();
    let mut i = 0;
    // Skip a string body starting after its opening quote; return the index
    // after the closing quote.
    let string_end = |mut i: usize, backslash: bool| -> usize {
        while i < n {
            match c[i] {
                '\\' if backslash => i += 2,
                '\'' if c.get(i + 1) == Some(&'\'') => i += 2,
                '\'' => return i + 1,
                _ => i += 1,
            }
        }
        n
    };
    while i < n {
        let ch = c[i];
        let next = c.get(i + 1).copied();
        let start = i;
        let mut out = Pusher {
            out: &mut out,
            start,
        };
        match ch {
            _ if ch.is_whitespace() => i += 1,
            '-' if next == Some('-') => {
                while i < n && c[i] != '\n' {
                    i += 1;
                }
            }
            '/' if next == Some('*') => {
                // Block comments nest in PostgreSQL.
                let mut depth = 0u32;
                while i < n {
                    if c[i] == '/' && c.get(i + 1) == Some(&'*') {
                        depth += 1;
                        i += 2;
                    } else if c[i] == '*' && c.get(i + 1) == Some(&'/') {
                        depth -= 1;
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
            }
            '\'' => {
                i = string_end(i + 1, backslash_strings);
                out.push(Tok::Str);
            }
            '"' => {
                let (name, end) = quoted(&c, i + 1);
                out.push(Tok::Quoted(name.to_lowercase()));
                i = end;
            }
            ';' => {
                out.push(Tok::Semi);
                i += 1;
            }
            '(' => {
                out.push(Tok::Open);
                i += 1;
            }
            ')' => {
                out.push(Tok::Close);
                i += 1;
            }
            '$' => {
                // `$tag$ ... $tag$` (tag may be empty), else a `$1` parameter.
                let mut j = i + 1;
                if c.get(j).is_some_and(|&ch| is_ident_start(ch)) {
                    while c.get(j).is_some_and(|&ch| is_ident_char(ch) && ch != '$') {
                        j += 1;
                    }
                }
                if c.get(j) == Some(&'$') {
                    let delim: Vec<char> = c[i..=j].to_vec();
                    let mut k = j + 1;
                    let mut end = n;
                    while k + delim.len() <= n {
                        if c[k..k + delim.len()] == delim[..] {
                            end = k + delim.len();
                            break;
                        }
                        k += 1;
                    }
                    out.push(Tok::Str);
                    i = end;
                } else {
                    out.push(Tok::Other);
                    i += 1;
                    while c.get(i).is_some_and(char::is_ascii_digit) {
                        i += 1;
                    }
                }
            }
            _ if is_ident_start(ch) => {
                let start = i;
                while i < n && is_ident_char(c[i]) {
                    i += 1;
                }
                let word: String = c[start..i].iter().collect::<String>().to_lowercase();
                match (word.as_str(), c.get(i), c.get(i + 1)) {
                    // E'...': backslash escapes regardless of settings.
                    ("e", Some('\''), _) => {
                        i = string_end(i + 1, true);
                        out.push(Tok::Str);
                    }
                    // U&"..." identifiers and U&'...' strings.
                    ("u", Some('&'), Some('"')) => {
                        i = quoted(&c, i + 2).1;
                        out.push(Tok::UnicodeIdent);
                    }
                    ("u", Some('&'), Some('\'')) => {
                        i = string_end(i + 2, backslash_strings);
                        out.push(Tok::Str);
                    }
                    // B'..', X'..', N'..' are plain strings with a prefix.
                    ("b" | "x" | "n", Some('\''), _) => {
                        i = string_end(i + 1, backslash_strings);
                        out.push(Tok::Str);
                    }
                    _ => out.push(Tok::Word(word)),
                }
            }
            _ => {
                out.push(Tok::Other);
                i += 1;
            }
        }
    }
    out
}

/// Pushes tokens with the start of the token being lexed.
struct Pusher<'a> {
    out: &'a mut Vec<(Tok, usize)>,
    start: usize,
}

impl Pusher<'_> {
    fn push(&mut self, t: Tok) {
        self.out.push((t, self.start));
    }
}

/// A `"quoted"` identifier body starting at `i`: its text and the index
/// after the closing quote.
fn quoted(c: &[char], mut i: usize) -> (String, usize) {
    let mut s = String::new();
    while i < c.len() {
        if c[i] == '"' {
            if c.get(i + 1) == Some(&'"') {
                s.push('"');
                i += 2;
                continue;
            }
            return (s, i + 1);
        }
        s.push(c[i]);
        i += 1;
    }
    (s, i)
}

const READ_ONLY_SETTINGS: [&str; 2] = ["transaction_read_only", "default_transaction_read_only"];

fn word(t: &Tok) -> Option<&str> {
    match t {
        Tok::Word(w) => Some(w.as_str()),
        _ => None,
    }
}

fn check_statement(toks: &[Tok]) -> Option<&'static str> {
    let first = toks.first().and_then(word);
    // Two keywords in a row (comments between them were dropped already).
    let pair = |a: &str, b: &str| {
        toks.windows(2)
            .any(|w| word(&w[0]) == Some(a) && word(&w[1]) == Some(b))
    };
    if toks.contains(&Tok::UnicodeIdent) {
        return Some("U&\"...\" identifiers");
    }
    if first == Some("do") {
        return Some("DO blocks");
    }
    if toks.iter().any(|t| word(t) == Some("set_config")) {
        return Some("set_config()");
    }
    if first != Some("show")
        && toks.iter().any(|t| match t {
            Tok::Word(w) | Tok::Quoted(w) => READ_ONLY_SETTINGS.contains(&w.as_str()),
            _ => false,
        })
    {
        return Some("changing transaction_read_only");
    }
    if pair("read", "write") {
        return Some("READ WRITE transactions");
    }
    if pair("session", "characteristics") {
        return Some("SET SESSION CHARACTERISTICS");
    }
    if first == Some("reset") && toks.get(1).and_then(word) == Some("all") {
        return Some("RESET ALL");
    }
    None
}

/// What in `sql` could switch a read-only session to read-write, if anything.
pub fn read_only_violation(sql: &str) -> Option<&'static str> {
    [false, true].into_iter().find_map(|backslash| {
        lex(sql, backslash)
            .split(|t| *t == Tok::Semi)
            .find_map(check_statement)
    })
}

/// Whether `sql` is one row-returning query (`SELECT`, `WITH`, `VALUES`,
/// `TABLE`) and nothing else, however the server lexes strings. Such a query
/// can run through the extended protocol, where the server stops after the
/// rows asked for.
pub fn single_query(sql: &str) -> bool {
    [false, true].into_iter().all(|backslash| {
        let toks = lex(sql, backslash);
        let mut stmts = toks.split(|t| *t == Tok::Semi).filter(|s| !s.is_empty());
        let first = stmts.next();
        stmts.next().is_none()
            && first
                .and_then(|s| s.first())
                .and_then(word)
                .is_some_and(|w| matches!(w, "select" | "with" | "values" | "table"))
    })
}

/// A `COPY` that streams its data through the client connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyStdio {
    /// `COPY … FROM STDIN`: the client must send the rows.
    FromStdin,
    /// `COPY … TO STDOUT`, in text or CSV format.
    ToStdout,
    /// `COPY … TO STDOUT` in binary format.
    ToStdoutBinary,
}

/// Whether the statement `toks` is a `COPY` through the client connection.
/// `FROM STDIN` / `TO STDOUT` count outside parentheses only (a query's
/// `FROM stdin` is a table).
fn copy_statement(toks: &[Tok]) -> Option<CopyStdio> {
    if toks.first().and_then(word) != Some("copy") {
        return None;
    }
    let mut depth = 0u32;
    for (i, t) in toks.iter().enumerate() {
        match t {
            Tok::Open => depth += 1,
            Tok::Close => depth = depth.saturating_sub(1),
            Tok::Word(w) if depth == 0 => {
                match (w.as_str(), toks.get(i + 1).and_then(word)) {
                    ("from", Some("stdin")) => return Some(CopyStdio::FromStdin),
                    ("to", Some("stdout")) => {
                        // `(FORMAT binary)`, or the old `BINARY` / `WITH BINARY`.
                        let binary = toks[i + 2..].iter().any(|t| word(t) == Some("binary"));
                        return Some(if binary {
                            CopyStdio::ToStdoutBinary
                        } else {
                            CopyStdio::ToStdout
                        });
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    None
}

/// The `COPY … FROM STDIN` / `TO STDOUT` statements in `sql`, however the
/// server lexes strings, the one that needs the most first (`FROM STDIN`,
/// then binary, then `TO STDOUT`).
pub fn copy_stdio(sql: &str) -> Option<CopyStdio> {
    [false, true]
        .into_iter()
        .flat_map(|backslash| {
            lex(sql, backslash)
                .split(|t| *t == Tok::Semi)
                .filter_map(copy_statement)
                .collect::<Vec<_>>()
        })
        .min_by_key(|c| match c {
            CopyStdio::FromStdin => 0,
            CopyStdio::ToStdoutBinary => 1,
            CopyStdio::ToStdout => 2,
        })
}

/// One statement of a script.
#[derive(Debug, PartialEq, Eq)]
pub struct Statement<'a> {
    /// From after the previous statement's `;` up to (not including) its
    /// own, comments and whitespace in front included.
    pub sql: &'a str,
    /// Characters of the script before `sql` (server error positions count
    /// characters).
    pub offset: usize,
}

/// `(start, end)` character ranges of the statements in `toks`, split like
/// psql: on `;` outside parentheses and outside the `BEGIN ... END` body of
/// a `CREATE [OR REPLACE] FUNCTION|PROCEDURE`. Ranges without a token
/// (`;;`, a trailing comment) are left out.
fn statement_ranges(toks: &[(Tok, usize)], n: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut has_token = false;
    let mut parens = 0u32;
    let mut begins = 0u32;
    let mut words: Vec<&str> = Vec::new();
    for (t, at) in toks {
        match t {
            Tok::Semi if parens == 0 && begins == 0 => {
                if has_token {
                    out.push((start, *at));
                }
                start = at + 1;
                has_token = false;
                words.clear();
                continue;
            }
            Tok::Open => parens += 1,
            Tok::Close => parens = parens.saturating_sub(1),
            Tok::Word(w) => {
                if words.len() < 4 {
                    words.push(w);
                }
                let routine = matches!(
                    words.as_slice(),
                    ["create", "function" | "procedure", ..]
                        | ["create", "or", "replace", "function" | "procedure", ..]
                );
                if routine && parens == 0 {
                    match w.as_str() {
                        "begin" => begins += 1,
                        "case" if begins > 0 => begins += 1,
                        "end" if begins > 0 => begins -= 1,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        has_token = true;
    }
    if has_token {
        out.push((start, n));
    }
    out
}

/// Split a script into its statements, or `None` when that depends on how
/// the server lexes strings (`standard_conforming_strings`, `'a\'`).
pub fn split_statements(sql: &str) -> Option<Vec<Statement<'_>>> {
    let n = sql.chars().count();
    let [plain, backslash] =
        [false, true].map(|backslash| statement_ranges(&lex_spans(sql, backslash), n));
    if plain != backslash {
        return None;
    }
    let bytes: Vec<usize> = sql
        .char_indices()
        .map(|(b, _)| b)
        .chain([sql.len()])
        .collect();
    Some(
        plain
            .into_iter()
            .map(|(start, end)| Statement {
                sql: &sql[bytes[start]..bytes[end]],
                offset: start,
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(sql: &str) -> Option<Vec<&str>> {
        split_statements(sql).map(|v| v.into_iter().map(|s| s.sql.trim()).collect())
    }

    #[test]
    fn scripts_split_like_psql() {
        let cases: &[(&str, &[&str])] = &[
            ("select 1", &["select 1"]),
            ("select 1;", &["select 1"]),
            (
                "insert into zz values (1); insert into zz values (2); select 1/0",
                &[
                    "insert into zz values (1)",
                    "insert into zz values (2)",
                    "select 1/0",
                ],
            ),
            (
                "select 1;; ; select 2; -- done\n",
                &["select 1", "select 2"],
            ),
            (
                "select 'a;b', \"c;d\", $$e;f$$, $t$g;h$t$, E'i\\';j' /* k; /* l; */ */; select 2",
                &[
                    "select 'a;b', \"c;d\", $$e;f$$, $t$g;h$t$, E'i\\';j' /* k; /* l; */ */",
                    "select 2",
                ],
            ),
            // Rules with several actions keep theirs.
            (
                "create rule r as on insert to t do also (insert into a values (1); insert into b values (2)); select 1",
                &[
                    "create rule r as on insert to t do also (insert into a values (1); insert into b values (2))",
                    "select 1",
                ],
            ),
            // SQL-standard function bodies.
            (
                "CREATE FUNCTION f() RETURNS int LANGUAGE sql BEGIN ATOMIC SELECT 1; SELECT CASE WHEN true THEN 2 END; END; SELECT f()",
                &[
                    "CREATE FUNCTION f() RETURNS int LANGUAGE sql BEGIN ATOMIC SELECT 1; SELECT CASE WHEN true THEN 2 END; END",
                    "SELECT f()",
                ],
            ),
            (
                "create or replace procedure p() begin atomic insert into t values (1); end; call p()",
                &[
                    "create or replace procedure p() begin atomic insert into t values (1); end",
                    "call p()",
                ],
            ),
            // BEGIN/END elsewhere are statements of their own.
            (
                "begin; select case when true then 1 end; end; commit",
                &["begin", "select case when true then 1 end", "end", "commit"],
            ),
            (
                "create function g() returns int as $$ begin return 1; end $$ language plpgsql; select g()",
                &[
                    "create function g() returns int as $$ begin return 1; end $$ language plpgsql",
                    "select g()",
                ],
            ),
            ("", &[]),
            ("-- only a comment", &[]),
        ];
        for (sql, want) in cases {
            assert_eq!(split(sql).as_deref(), Some(*want), "{sql}");
        }
    }

    #[test]
    fn statements_know_their_offset() {
        let sql = "select 'é'; select x";
        let stmts = split_statements(sql).unwrap();
        assert_eq!(stmts[1].sql, " select x");
        assert_eq!(stmts[1].offset, 11);
        assert_eq!(
            sql.chars().skip(stmts[1].offset).collect::<String>(),
            " select x"
        );
    }

    #[test]
    fn ambiguous_strings_are_not_split() {
        // Two statements with standard_conforming_strings on, one with it off.
        assert_eq!(split(r"select 'a\'; select 2; -- '"), None);
        // A backslash that does not hide a quote is fine.
        assert_eq!(
            split(r"select 'C:\temp'; select 2").as_deref(),
            Some(&[r"select 'C:\temp'", "select 2"][..])
        );
    }

    #[test]
    fn single_queries() {
        for sql in [
            "SELECT 1",
            "  select * from t;  ",
            "-- note\nWITH c AS (SELECT 1) SELECT * FROM c;",
            "VALUES (1), (2)",
            "TABLE t",
            r"SELECT 'a;b', $$x; y$$, E'it\'s; ok', 'x'';y'",
            "SELECT $tag$ ; $tag$ /* ; /* ; */ */",
        ] {
            assert!(single_query(sql), "{sql}");
        }
        for sql in [
            "SELECT 1; SELECT 2",
            "SELECT 1; COMMIT",
            "INSERT INTO t VALUES (1)",
            "BEGIN",
            "EXPLAIN SELECT 1",
            "(SELECT 1)",
            "",
            ";",
            // With standard_conforming_strings off the first `;` is in a string.
            r"SELECT 'a\'; SELECT 2; --'",
        ] {
            assert!(!single_query(sql), "{sql}");
        }
    }

    fn refused(sql: &str) -> bool {
        read_only_violation(sql).is_some()
    }

    #[test]
    fn ordinary_sql_passes() {
        for sql in [
            "SELECT 1",
            "SELECT * FROM t WHERE a = 'read write'; SELECT 2",
            "SHOW transaction_read_only",
            "SHOW default_transaction_read_only",
            "SELECT current_setting('transaction_read_only')",
            "SELECT read, write FROM perms",
            "BEGIN; SELECT 1; COMMIT",
            "BEGIN READ ONLY; SELECT 1; ROLLBACK",
            "START TRANSACTION ISOLATION LEVEL SERIALIZABLE",
            "SET search_path = app, public",
            "SET statement_timeout = '5s'",
            "RESET search_path",
            "SELECT $$ SET transaction_read_only = off $$",
            "SELECT $tag$ RESET ALL $tag$",
            "SELECT 'it''s'; -- SET transaction_read_only = off",
            "/* /* nested */ SET default_transaction_read_only = off */ SELECT 1",
            "SELECT $1::int",
            "SELECT a$b FROM t",
        ] {
            assert_eq!(read_only_violation(sql), None, "{sql}");
        }
    }

    #[test]
    fn transaction_mode_changes_are_refused() {
        for sql in [
            "SET default_transaction_read_only = off",
            "set DEFAULT_TRANSACTION_READ_ONLY to false",
            "SET SESSION default_transaction_read_only = off",
            "SET LOCAL transaction_read_only = off",
            "SET \"default_transaction_read_only\" = off",
            "SET \"DEFAULT_TRANSACTION_READ_ONLY\" = off",
            "RESET default_transaction_read_only",
            "RESET transaction_read_only",
            "RESET ALL",
            "reset /* x */ all",
            "SET SESSION CHARACTERISTICS AS TRANSACTION READ WRITE",
            "SET TRANSACTION READ WRITE",
            "BEGIN READ WRITE",
            "BEGIN TRANSACTION ISOLATION LEVEL READ COMMITTED, READ WRITE",
            "START TRANSACTION READ /* c */ WRITE",
            "ALTER ROLE CURRENT_USER SET default_transaction_read_only = off",
            "SELECT set_config('default_transaction_read_only', 'off', false)",
            "SELECT pg_catalog.SET_CONFIG('x', 'y', false)",
            "DO $$ BEGIN EXECUTE 'SE' || 'T default_transaction_read_only = off'; END $$",
            "SET U&\"default_transaction_read_onl\\0079\" = off",
            "SELECT 1; SET default_transaction_read_only = off",
        ] {
            assert!(refused(sql), "{sql}");
        }
    }

    #[test]
    fn strings_cannot_hide_a_statement() {
        // Each hides the SET from a lexer that gets one quoting rule wrong.
        for sql in [
            // E'' strings use backslash escapes.
            "SELECT E'\\''; SET default_transaction_read_only = off; -- '",
            // Plain strings do too with standard_conforming_strings = off.
            "SELECT '\\''; SET default_transaction_read_only = off; -- '",
            // ...and do not with it on (the default).
            "SELECT 'a\\'; SET default_transaction_read_only = off; -- '",
            // A dollar-quoted string containing a quote.
            "SELECT $a$'$a$; SET default_transaction_read_only = off; -- '",
            "SELECT $$'$$; SET default_transaction_read_only = off; -- '",
            // A quoted identifier containing a quote.
            "SELECT 1 AS \"'\"; SET default_transaction_read_only = off; -- '",
        ] {
            assert!(refused(sql), "{sql}");
        }
    }

    #[test]
    fn copy_through_the_connection_is_found() {
        use CopyStdio::*;
        let cases: &[(&str, Option<CopyStdio>)] = &[
            ("copy t from stdin", Some(FromStdin)),
            (
                "COPY public.t (a, b) FROM stdin;\n1\t2\n\\.\n",
                Some(FromStdin),
            ),
            (
                "select 1; copy t from stdin with (format csv)",
                Some(FromStdin),
            ),
            ("copy (select 1) to stdout", Some(ToStdout)),
            ("COPY t TO STDOUT WITH (FORMAT csv, HEADER)", Some(ToStdout)),
            ("copy t to stdout (format binary)", Some(ToStdoutBinary)),
            ("copy t to stdout with binary", Some(ToStdoutBinary)),
            // The worst one wins.
            ("copy t to stdout; copy u from stdin", Some(FromStdin)),
            ("copy t to '/tmp/x'", None),
            ("copy t from '/tmp/x'", None),
            ("copy t from program 'cat x'", None),
            ("copy (select * from stdin) to '/tmp/x'", None),
            ("select * from stdin", None),
            ("select 'copy t from stdin'", None),
            ("select $$copy t to stdout$$", None),
            ("-- copy t from stdin\nselect 1", None),
            ("do $$ begin copy t to stdout; end $$", None),
        ];
        for (sql, want) in cases {
            assert_eq!(copy_stdio(sql), *want, "{sql}");
        }
        // Hidden from one lexing only: still found.
        assert_eq!(
            copy_stdio(r"select 'a\'; copy t from stdin; -- '"),
            Some(FromStdin)
        );
    }

    #[test]
    fn lexer_survives_odd_input() {
        for sql in ["'", "\"", "$$", "$a$", "/*", "E'", "U&\"", "-- x", "$", "é"] {
            let _ = read_only_violation(sql);
        }
    }
}
