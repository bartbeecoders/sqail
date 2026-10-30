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
    Other,
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || !c.is_ascii()
}

fn is_ident_char(c: char) -> bool {
    is_ident_start(c) || c.is_ascii_digit() || c == '$'
}

/// Tokenize `sql`. `backslash_strings`: treat `\` as an escape in plain
/// `'...'` strings too, as the server does with
/// `standard_conforming_strings = off`.
fn lex(sql: &str, backslash_strings: bool) -> Vec<Tok> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn lexer_survives_odd_input() {
        for sql in ["'", "\"", "$$", "$a$", "/*", "E'", "U&\"", "-- x", "$", "é"] {
            let _ = read_only_violation(sql);
        }
    }
}
