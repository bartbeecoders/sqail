//! A forgiving SQL lexer. It never fails: anything it does not understand is
//! punctuation. Used for highlighting and for finding statement boundaries.

use std::ops::Range;

use sqail_client::proto::Engine;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Space,
    Comment,
    String,
    QuotedIdent,
    Number,
    Word,
    Variable,
    Punct,
    Open,
    Close,
    Semicolon,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: Kind,
    pub range: Range<usize>,
}

pub fn tokenize(s: &str, engine: Option<Engine>) -> Vec<Token> {
    let b = s.as_bytes();
    let mssql = engine == Some(Engine::Mssql);
    let pg = engine == Some(Engine::Postgres);
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let start = i;
        let c = b[i];
        let next = b.get(i + 1).copied();
        let kind = match c {
            b' ' | b'\t' | b'\r' | b'\n' => {
                while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\r' | b'\n') {
                    i += 1;
                }
                Kind::Space
            }
            b'-' if next == Some(b'-') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                Kind::Comment
            }
            b'/' if next == Some(b'*') => {
                // T-SQL and Postgres nest block comments; SQLite does not,
                // but nesting is harmless for display.
                let mut depth = 0;
                while i < b.len() {
                    if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                        depth += 1;
                        i += 2;
                    } else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                        depth -= 1;
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
                Kind::Comment
            }
            b'\'' => {
                i = skip_quoted(b, i + 1, b'\'');
                Kind::String
            }
            b'N' | b'n' | b'E' | b'e' | b'X' | b'x' if next == Some(b'\'') => {
                i = skip_quoted(b, i + 2, b'\'');
                Kind::String
            }
            b'"' => {
                i = skip_quoted(b, i + 1, b'"');
                Kind::QuotedIdent
            }
            b'`' => {
                i = skip_quoted(b, i + 1, b'`');
                Kind::QuotedIdent
            }
            b'[' if !pg => {
                i = skip_quoted(b, i + 1, b']');
                Kind::QuotedIdent
            }
            b'$' if pg => match dollar_tag(b, i) {
                Some(tag_end) => {
                    let tag = &b[i..tag_end];
                    i = find(b, tag_end, tag).map_or(b.len(), |p| p + tag.len());
                    Kind::String
                }
                None => {
                    // $1 placeholder
                    i += 1;
                    while i < b.len() && b[i].is_ascii_digit() {
                        i += 1;
                    }
                    Kind::Variable
                }
            },
            b'0'..=b'9' => {
                i += 1;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'.' || b[i] == b'_')
                {
                    i += 1;
                }
                Kind::Number
            }
            b'.' if next.is_some_and(|n| n.is_ascii_digit()) => {
                i += 1;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
                Kind::Number
            }
            b':' if next == Some(b':') => {
                i += 2;
                Kind::Punct
            }
            b'@' | b':' | b'?' if next.is_some_and(|n| is_word_byte(n) || n == b'@') => {
                i += 1;
                while i < b.len() && (is_word_byte(b[i]) || b[i] == b'@') {
                    i += 1;
                }
                Kind::Variable
            }
            b'#' if mssql => {
                i += 1;
                while i < b.len() && (is_word_byte(b[i]) || b[i] == b'#') {
                    i += 1;
                }
                Kind::Word
            }
            _ if is_word_byte(c) => {
                while i < b.len() && is_word_byte(b[i]) {
                    i += 1;
                }
                Kind::Word
            }
            b'(' => {
                i += 1;
                Kind::Open
            }
            b')' => {
                i += 1;
                Kind::Close
            }
            b';' => {
                i += 1;
                Kind::Semicolon
            }
            _ => {
                // One whole UTF-8 character.
                i += utf8_len(c);
                Kind::Punct
            }
        };
        out.push(Token {
            kind,
            range: start..i.min(b.len()),
        });
    }
    out
}

fn is_word_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80
}

fn utf8_len(first: u8) -> usize {
    match first {
        0xF0..=0xFF => 4,
        0xE0..=0xEF => 3,
        0xC0..=0xDF => 2,
        _ => 1,
    }
}

/// Index after the closing quote; a doubled quote is an escape.
fn skip_quoted(b: &[u8], mut i: usize, q: u8) -> usize {
    while i < b.len() {
        if b[i] == q {
            if b.get(i + 1) == Some(&q) {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    b.len()
}

/// `$tag$` or `$$` starting at `i`: returns the index after the opening tag.
fn dollar_tag(b: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 1;
    while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
        if j == i + 1 && b[j].is_ascii_digit() {
            return None;
        }
        j += 1;
    }
    (b.get(j) == Some(&b'$')).then_some(j + 1)
}

fn find(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    hay.get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

pub fn keywords() -> &'static [&'static str] {
    KEYWORDS
}

/// Case-insensitive keyword check without allocating. Keywords are bucketed
/// by first letter, so a lookup is a few comparisons (this runs for every
/// word on every edit).
pub fn is_keyword(word: &str) -> bool {
    let b = word.as_bytes();
    if b.len() < 2 || b.len() > 16 || !b[0].is_ascii_alphabetic() {
        return false;
    }
    let bucket = &buckets()[(b[0].to_ascii_uppercase() - b'A') as usize];
    KEYWORDS[bucket.clone()]
        .iter()
        .any(|k| k.len() == b.len() && k.as_bytes().eq_ignore_ascii_case(b))
}

fn buckets() -> &'static [std::ops::Range<usize>; 26] {
    static B: std::sync::OnceLock<[std::ops::Range<usize>; 26]> = std::sync::OnceLock::new();
    B.get_or_init(|| {
        std::array::from_fn(|i| {
            let letter = b'A' + i as u8;
            let start = KEYWORDS.partition_point(|k| k.as_bytes()[0] < letter);
            let end = KEYWORDS.partition_point(|k| k.as_bytes()[0] <= letter);
            start..end
        })
    })
}

/// Sorted; binary-searched.
const KEYWORDS: &[&str] = &[
    "ADD",
    "ALL",
    "ALTER",
    "AND",
    "ANY",
    "AS",
    "ASC",
    "AUTHORIZATION",
    "AUTOINCREMENT",
    "BACKUP",
    "BEGIN",
    "BETWEEN",
    "BIGINT",
    "BINARY",
    "BIT",
    "BOOLEAN",
    "BREAK",
    "BY",
    "CALL",
    "CASCADE",
    "CASE",
    "CAST",
    "CATCH",
    "CHAR",
    "CHECK",
    "CLOSE",
    "COLLATE",
    "COLUMN",
    "COMMIT",
    "CONSTRAINT",
    "CONTINUE",
    "CONVERT",
    "CREATE",
    "CROSS",
    "CURRENT",
    "CURSOR",
    "DATABASE",
    "DATE",
    "DATETIME",
    "DATETIME2",
    "DATETIMEOFFSET",
    "DEALLOCATE",
    "DECIMAL",
    "DECLARE",
    "DEFAULT",
    "DELETE",
    "DESC",
    "DISTINCT",
    "DO",
    "DOUBLE",
    "DROP",
    "ELSE",
    "END",
    "ESCAPE",
    "EXCEPT",
    "EXEC",
    "EXECUTE",
    "EXISTS",
    "EXPLAIN",
    "FALSE",
    "FETCH",
    "FILTER",
    "FIRST",
    "FLOAT",
    "FOR",
    "FOREIGN",
    "FROM",
    "FULL",
    "FUNCTION",
    "GO",
    "GOTO",
    "GRANT",
    "GROUP",
    "HAVING",
    "IDENTITY",
    "IF",
    "ILIKE",
    "IN",
    "INDEX",
    "INNER",
    "INSERT",
    "INT",
    "INTEGER",
    "INTERSECT",
    "INTERVAL",
    "INTO",
    "IS",
    "JOIN",
    "JSON",
    "JSONB",
    "KEY",
    "LANGUAGE",
    "LAST",
    "LATERAL",
    "LEFT",
    "LIKE",
    "LIMIT",
    "MATERIALIZED",
    "MERGE",
    "MONEY",
    "NCHAR",
    "NEXT",
    "NOCOUNT",
    "NOT",
    "NULL",
    "NULLS",
    "NUMERIC",
    "NVARCHAR",
    "OF",
    "OFF",
    "OFFSET",
    "ON",
    "ONLY",
    "OPEN",
    "OR",
    "ORDER",
    "OUTER",
    "OUTPUT",
    "OVER",
    "PARTITION",
    "PERCENT",
    "PIVOT",
    "PRAGMA",
    "PRIMARY",
    "PRINT",
    "PROC",
    "PROCEDURE",
    "RAISERROR",
    "REAL",
    "RECURSIVE",
    "REFERENCES",
    "REPLACE",
    "RETURN",
    "RETURNING",
    "RETURNS",
    "REVOKE",
    "RIGHT",
    "ROLLBACK",
    "ROW",
    "ROWS",
    "SAVEPOINT",
    "SCHEMA",
    "SELECT",
    "SEQUENCE",
    "SERIAL",
    "SET",
    "SMALLINT",
    "TABLE",
    "TEMP",
    "TEMPORARY",
    "TEXT",
    "THEN",
    "THROW",
    "TIES",
    "TIME",
    "TIMESTAMP",
    "TIMESTAMPTZ",
    "TINYINT",
    "TO",
    "TOP",
    "TRAN",
    "TRANSACTION",
    "TRIGGER",
    "TRUE",
    "TRUNCATE",
    "TRY",
    "TYPE",
    "UNION",
    "UNIQUE",
    "UNIQUEIDENTIFIER",
    "UNPIVOT",
    "UPDATE",
    "USE",
    "USING",
    "UUID",
    "VACUUM",
    "VALUES",
    "VARBINARY",
    "VARCHAR",
    "VIEW",
    "WAITFOR",
    "WHEN",
    "WHERE",
    "WHILE",
    "WINDOW",
    "WITH",
    "WITHOUT",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(s: &str, e: Option<Engine>) -> Vec<(Kind, &str)> {
        tokenize(s, e)
            .into_iter()
            .filter(|t| t.kind != Kind::Space)
            .map(|t| (t.kind, &s[t.range]))
            .collect()
    }

    #[test]
    fn keyword_lookup() {
        for k in KEYWORDS {
            assert!(is_keyword(k) && is_keyword(&k.to_lowercase()), "{k}");
        }
        for w in ["orders", "x", "SELECTX", "_select", "sélect", "9"] {
            assert!(!is_keyword(w), "{w}");
        }
    }

    #[test]
    fn keywords_are_sorted() {
        let mut sorted = KEYWORDS.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, KEYWORDS);
    }

    #[test]
    fn basic_tokens() {
        assert_eq!(
            kinds("SELECT 'a''b', 1.5 -- x\nFROM [t]", Some(Engine::Mssql)),
            vec![
                (Kind::Word, "SELECT"),
                (Kind::String, "'a''b'"),
                (Kind::Punct, ","),
                (Kind::Number, "1.5"),
                (Kind::Comment, "-- x"),
                (Kind::Word, "FROM"),
                (Kind::QuotedIdent, "[t]"),
            ]
        );
    }

    #[test]
    fn postgres_dollar_quotes_and_casts() {
        let k = kinds(
            "DO $f$ BEGIN; END $f$; SELECT $1::int",
            Some(Engine::Postgres),
        );
        assert_eq!(k[1], (Kind::String, "$f$ BEGIN; END $f$"));
        assert!(k.contains(&(Kind::Variable, "$1")));
        assert!(k.contains(&(Kind::Punct, "::")));
    }

    #[test]
    fn unterminated_things_do_not_panic() {
        for s in ["'abc", "/* x", "\"q", "$$ body", "é'"] {
            tokenize(s, Some(Engine::Postgres));
            tokenize(s, Some(Engine::Mssql));
        }
    }
}
