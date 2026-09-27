//! Randomized fuzzing for everything that parses untrusted text: the SQL
//! lexer, statement splitter, completer, formatter, bracket matcher, edit
//! detection, the service's `GO` splitter, and the client's NDJSON decoder.
//!
//!     cargo run --release -p sqail-fuzz -- [seconds] [seed]
//!
//! Inputs mix SQL-significant fragments (quotes, `$$`, `GO`, comments,
//! brackets, multi-byte characters) with random characters and mutations of
//! earlier inputs. Every target also checks its invariants, not only "no
//! panic". A failing input is written to `fuzz-crash-<n>.txt`.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

use sqail_proto::{ColumnInfo, Engine, ForeignKeyInfo, QueryEvent, TableInfo, TableKind};

/// Tiny deterministic PRNG (xorshift64*), so a seed reproduces a run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
}

const PIECES: &[&str] = &[
    "SELECT",
    "select",
    "FROM",
    "WHERE",
    "JOIN",
    "ON",
    "GROUP BY",
    "INSERT",
    "UPDATE",
    "DELETE",
    "SET",
    "BEGIN",
    "END",
    "CASE",
    "WHEN",
    "THEN",
    "BEGIN TRANSACTION",
    "COMMIT",
    "GO",
    "go 3",
    "\nGO\n",
    "'",
    "''",
    "\"",
    "[",
    "]",
    "]]",
    "`",
    "$$",
    "$tag$",
    "$1",
    "?1",
    "@P1",
    "@@SPID",
    "#tmp",
    "N'",
    "E'",
    "X'",
    "--",
    "/*",
    "*/",
    ";",
    ",",
    ".",
    "(",
    ")",
    "::",
    "*",
    "=",
    " ",
    "  ",
    "\t",
    "\n",
    "\n\n",
    "\r\n",
    "a",
    "o.",
    "sales.",
    "orders o",
    "é",
    "世界",
    "😀",
    "\u{0}",
    "\u{200b}",
    "0",
    "1.5",
    ".5",
    "1e9",
    "TOP (10)",
    "LIMIT 5",
    "AS",
    "WITH",
    "TOP",
    "APPLY",
    "LATERAL",
    "*",
    "x",
];

fn generate(rng: &mut Rng, corpus: &[String]) -> String {
    let mut s = if !corpus.is_empty() && rng.below(3) == 0 {
        corpus[rng.below(corpus.len())].clone()
    } else {
        String::new()
    };
    for _ in 0..rng.below(40) + 1 {
        match rng.below(10) {
            0 => s.push(char::from_u32(rng.below(0x3000) as u32).unwrap_or('?')),
            1 if !s.is_empty() => {
                // Delete a random char range (mutation).
                let chars: Vec<char> = s.chars().collect();
                let a = rng.below(chars.len());
                let b = (a + rng.below(5)).min(chars.len());
                s = chars[..a].iter().chain(&chars[b..]).collect();
            }
            _ => s.push_str(rng.pick(PIECES)),
        }
    }
    s
}

/// A random char boundary of `s` (as a byte offset).
fn boundary(rng: &mut Rng, s: &str) -> usize {
    let bounds: Vec<usize> = s.char_indices().map(|(i, _)| i).chain([s.len()]).collect();
    *rng.pick(&bounds)
}

struct Catalog;

impl sqail_ui::sql::complete::Catalog for Catalog {
    fn schemas(&self) -> Option<Vec<String>> {
        Some(vec!["public".into(), "sales".into(), "é".into()])
    }
    fn tables(&self, schema: &str) -> Option<Vec<TableInfo>> {
        Some(vec![TableInfo {
            schema: Some(schema.into()),
            name: "orders".into(),
            kind: TableKind::Table,
        }])
    }
    fn columns(&self, _: &str, _: &str) -> Option<Vec<ColumnInfo>> {
        Some(vec![ColumnInfo {
            name: "id".into(),
            ordinal: 1,
            data_type: "int".into(),
            nullable: false,
            default: None,
            primary_key: true,
        }])
    }
    fn foreign_keys(&self, _: &str, _: &str) -> Option<Vec<ForeignKeyInfo>> {
        Some(vec![ForeignKeyInfo {
            name: "fk".into(),
            columns: vec!["id".into()],
            ref_schema: Some("sales".into()),
            ref_table: "orders".into(),
            ref_columns: vec!["id".into()],
        }])
    }
}

fn check(cond: bool, what: &str) {
    assert!(cond, "invariant violated: {what}");
}

/// All targets for one input. Panics on a bug.
fn run_targets(rng: &mut Rng, s: &str) {
    use sqail_ui::sql::{complete, format, highlight, lex, statements};
    let engines = [
        None,
        Some(Engine::Postgres),
        Some(Engine::Mssql),
        Some(Engine::Sqlite),
    ];
    for engine in engines {
        // Tokens tile the input exactly, on char boundaries.
        let toks = lex::tokenize(s, engine);
        let mut at = 0;
        for t in &toks {
            check(
                t.range.start == at && t.range.end > t.range.start,
                "tokens are contiguous and non-empty",
            );
            check(
                s.is_char_boundary(t.range.start) && s.is_char_boundary(t.range.end),
                "token on char boundaries",
            );
            at = t.range.end;
        }
        check(at == s.len(), "tokens cover the input");

        for r in statements::split(s, engine) {
            check(
                r.start < r.end && r.end <= s.len(),
                "statement range in bounds",
            );
            check(
                s.is_char_boundary(r.start) && s.is_char_boundary(r.end),
                "statement on char boundaries",
            );
        }
        let cursor = boundary(rng, s);
        if let Some(r) = statements::at(s, engine, cursor) {
            check(
                r.end <= s.len() && s.is_char_boundary(r.start),
                "statement at cursor valid",
            );
        }
        if let Some((a, b)) = highlight::matching_bracket(s, engine, cursor) {
            check(
                s[a..].starts_with(['(', ')']) && s[b..].starts_with(['(', ')']),
                "brackets match brackets",
            );
        }
        if let Some(e) = engine {
            if let Some(c) = complete::complete(s, cursor, e, &Catalog) {
                check(
                    c.range.end == cursor && s.is_char_boundary(c.range.start),
                    "completion replaces the typed prefix",
                );
            }
            let _ = sqail_ui::editing::source_table(s, e);
        }
        let style = format::Style {
            uppercase: true,
            indent: 2,
        };
        let _ = format::format(s, engine, &style);
        let _ = sqail_ui::transfer::sniff_delimiter(s);
    }

    // Service: GO batches are pieces of the script.
    for b in sqail_service::engine::split::split_go(s) {
        check(s.contains(b.sql.trim()), "GO batch comes from the script");
        check(b.repeat > 0, "GO count positive");
    }
    let _ = sqail_service::engine::split::shape(s);

    // Client: garbage never panics the decoder, however it is chunked.
    let mut d = sqail_client::NdjsonDecoder::default();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let n = rng.below(16) + 1;
        let _ = d.push(&bytes[i..(i + n).min(bytes.len())]);
        i += n;
    }
    let _ = d.finish();
}

/// Valid events survive arbitrary chunking unchanged.
fn decoder_round_trip(rng: &mut Rng, s: &str) {
    let events = vec![
        QueryEvent::Message {
            severity: "info".into(),
            text: s.to_string(),
        },
        QueryEvent::Rows {
            index: 0,
            rows: vec![vec![
                serde_json::Value::String(s.to_string()),
                serde_json::Value::Null,
            ]],
        },
        QueryEvent::Done {
            elapsed_ms: 1,
            cancelled: false,
            in_transaction: Some(true),
        },
    ];
    let mut body = Vec::new();
    for e in &events {
        body.extend(serde_json::to_vec(e).expect("serializes"));
        body.push(b'\n');
    }
    let mut d = sqail_client::NdjsonDecoder::default();
    let mut got = Vec::new();
    let mut i = 0;
    while i < body.len() {
        let n = rng.below(64) + 1;
        got.extend(
            d.push(&body[i..(i + n).min(body.len())])
                .into_iter()
                .map(|r| r.expect("valid event")),
        );
        i += n;
    }
    check(d.finish().is_none(), "nothing left after complete lines");
    check(got == events, "events round-trip through the decoder");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let secs: u64 = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(60);
    let seed: u64 = args.get(2).and_then(|a| a.parse().ok()).unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1)
            | 1
    });
    println!("fuzzing for {secs}s with seed {seed}");
    // Keep panic messages for our report, not stderr noise.
    std::panic::set_hook(Box::new(|_| {}));

    let mut rng = Rng(seed);
    let mut corpus: Vec<String> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut n: u64 = 0;
    let mut crashes = 0;
    while Instant::now() < deadline {
        let input = generate(&mut rng, &corpus);
        let mut sub = Rng(rng.next() | 1);
        let res = catch_unwind(AssertUnwindSafe(|| {
            run_targets(&mut sub, &input);
            decoder_round_trip(&mut sub, &input);
        }));
        if let Err(p) = res {
            crashes += 1;
            let msg = p
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            let file = format!("fuzz-crash-{crashes}.txt");
            let _ = std::fs::write(&file, &input);
            println!(
                "CRASH #{crashes}: {msg}\n  input {:?}\n  saved to {file}",
                input
            );
            if crashes >= 5 {
                break;
            }
        } else if corpus.len() < 512 && rng.below(8) == 0 {
            corpus.push(input);
        }
        n += 1;
    }
    println!("{n} inputs, {crashes} crash(es)");
    if crashes > 0 {
        std::process::exit(1);
    }
}
