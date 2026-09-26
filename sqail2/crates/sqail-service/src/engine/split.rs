//! A small T-SQL lexer: splits scripts on `GO` and recognises single DML
//! statements. It only needs to understand what can hide a `GO` or a `;`:
//! strings, quoted identifiers and comments.

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
}

/// Split a script on lines consisting of `GO [count]` (outside comments and
/// strings), like sqlcmd and SSMS do. Empty batches are dropped.
pub fn split_go(script: &str) -> Vec<Batch> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut st = St::Code;
    for line in script.split_inclusive('\n') {
        let at_code = matches!(st, St::Code | St::LineComment);
        if at_code && let Some(repeat) = go_line(line) {
            push_batch(&mut out, std::mem::take(&mut cur), repeat);
            st = St::Code;
            continue;
        }
        scan(line, &mut st, |_, _| {});
        cur.push_str(line);
    }
    push_batch(&mut out, cur, 1);
    out
}

fn push_batch(out: &mut Vec<Batch>, sql: String, repeat: u32) {
    if !sql.trim().is_empty() {
        out.push(Batch { sql, repeat });
    }
}

fn go_line(line: &str) -> Option<u32> {
    let line = line.trim();
    let line = match line.find("--") {
        Some(i) => line[..i].trim_end(),
        None => line,
    };
    // `get` rather than slicing: the line may start with a multi-byte char.
    let (Some(word), Some(rest)) = (line.get(..2), line.get(2..)) else {
        return None;
    };
    if !word.eq_ignore_ascii_case("go") {
        return None;
    }
    let rest = rest.trim();
    if rest.is_empty() {
        Some(1)
    } else {
        rest.parse().ok().filter(|&n| n > 0)
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                repeat: 3
            }]
        );
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
}
