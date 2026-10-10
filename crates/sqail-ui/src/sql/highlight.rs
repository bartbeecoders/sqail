//! Syntax highlighting as an egui `LayoutJob`, memoized per frame.

use egui::cache::{ComputerMut, FrameCache};
use egui::text::{LayoutJob, TextFormat};
use egui::{FontId, Stroke};
use sqail_client::proto::Engine;

use super::lex::{Kind, is_keyword, tokenize};
use crate::theme::syntax;

#[derive(Hash, Clone, Copy)]
pub struct Key<'a> {
    pub text: &'a str,
    pub engine: Option<Engine>,
    pub dark: bool,
    /// Byte offsets of a matched bracket pair to emphasise.
    pub brackets: Option<(usize, usize)>,
    pub font_size_bits: u32,
}

#[derive(Default)]
struct Highlighter;

/// The key plus the theme generation: a new palette recolours the text.
impl ComputerMut<(Key<'_>, u64), LayoutJob> for Highlighter {
    fn compute(&mut self, (key, _generation): (Key<'_>, u64)) -> LayoutJob {
        layout(key)
    }
}

type Cache = FrameCache<LayoutJob, Highlighter>;

pub fn highlight(ctx: &egui::Context, key: Key<'_>) -> LayoutJob {
    let generation = crate::theme::generation();
    ctx.memory_mut(|m| m.caches.cache::<Cache>().get((key, generation)).clone())
}

/// Token classes that get distinct formats.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Text,
    Keyword,
    Func,
    Str,
    Number,
    Comment,
    Ident,
    Bracket,
}

fn layout(key: Key<'_>) -> LayoutJob {
    let colors = syntax(key.dark);
    let font = FontId::monospace(f32::from_bits(key.font_size_bits));
    let fmt = |color, italics, background| TextFormat {
        font_id: font.clone(),
        color,
        italics,
        background,
        underline: Stroke::NONE,
        ..Default::default()
    };
    let none = egui::Color32::TRANSPARENT;
    let formats = [
        (Class::Text, fmt(colors.text, false, none)),
        (Class::Keyword, fmt(colors.keyword, false, none)),
        (Class::Func, fmt(colors.func, false, none)),
        (Class::Str, fmt(colors.string, false, none)),
        (Class::Number, fmt(colors.number, false, none)),
        (Class::Comment, fmt(colors.comment, true, none)),
        (Class::Ident, fmt(colors.ident, false, none)),
        (Class::Bracket, fmt(colors.text, false, colors.bracket_bg)),
    ];
    let format_of = |c: Class| {
        formats
            .iter()
            .find(|(k, _)| *k == c)
            .map(|(_, f)| f.clone())
            .expect("all classes")
    };

    let tokens = tokenize(key.text, key.engine);
    let mut job = LayoutJob {
        text: key.text.to_string(),
        ..Default::default()
    };
    job.wrap.max_width = f32::INFINITY;
    // Adjacent tokens with the same class share one section; whitespace
    // joins whatever precedes it. Far fewer sections = faster layout.
    let mut current: Option<(Class, usize)> = None;
    let mut end = 0;
    for (i, t) in tokens.iter().enumerate() {
        let s = &key.text[t.range.clone()];
        let is_bracket = key
            .brackets
            .is_some_and(|(a, b)| t.range.start == a || t.range.start == b);
        let class = match t.kind {
            _ if is_bracket => Class::Bracket,
            Kind::Space => match current {
                Some((c, _)) if c != Class::Bracket => c,
                _ => Class::Text,
            },
            Kind::Comment => Class::Comment,
            Kind::String => Class::Str,
            Kind::Number => Class::Number,
            Kind::QuotedIdent | Kind::Variable => Class::Ident,
            Kind::Word if is_keyword(s) => Class::Keyword,
            Kind::Word if next_is_open(&tokens[i + 1..]) => Class::Func,
            _ => Class::Text,
        };
        match current {
            Some((c, _)) if c == class => {}
            Some((c, start)) => {
                push(&mut job, start..t.range.start, format_of(c));
                current = Some((class, t.range.start));
            }
            None => current = Some((class, t.range.start)),
        }
        end = t.range.end;
    }
    if let Some((c, start)) = current {
        push(&mut job, start..end, format_of(c));
    }
    job
}

fn push(job: &mut LayoutJob, byte_range: std::ops::Range<usize>, format: TextFormat) {
    job.sections.push(egui::text::LayoutSection {
        leading_space: 0.0,
        byte_range: byte_range.start.into()..byte_range.end.into(),
        format,
    });
}

fn next_is_open(rest: &[super::lex::Token]) -> bool {
    rest.iter()
        .find(|t| t.kind != Kind::Space)
        .is_some_and(|t| t.kind == Kind::Open)
}

/// If the char before or at `cursor` (byte offset) is a bracket outside
/// strings/comments, return it and its partner.
pub fn matching_bracket(
    text: &str,
    engine: Option<Engine>,
    cursor: usize,
) -> Option<(usize, usize)> {
    let tokens = tokenize(text, engine);
    let idx = tokens.iter().position(|t| {
        matches!(t.kind, Kind::Open | Kind::Close)
            && (t.range.start == cursor || t.range.end == cursor)
    })?;
    let (step, open_kind): (isize, Kind) = match tokens[idx].kind {
        Kind::Open => (1, Kind::Open),
        _ => (-1, Kind::Close),
    };
    let mut depth = 0i32;
    let mut i = idx as isize;
    while i >= 0 && (i as usize) < tokens.len() {
        let t = &tokens[i as usize];
        match t.kind {
            k if k == open_kind => depth += 1,
            Kind::Open | Kind::Close => {
                depth -= 1;
                if depth == 0 {
                    return Some((tokens[idx].range.start, t.range.start));
                }
            }
            _ => {}
        }
        i += step;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brackets_match_across_nesting_and_skip_strings() {
        let t = "f(a, g(b), ')')";
        assert_eq!(matching_bracket(t, None, 1), Some((1, 14)));
        assert_eq!(matching_bracket(t, None, 15), Some((14, 1)));
        assert_eq!(matching_bracket(t, None, 6), Some((6, 8)));
        assert_eq!(matching_bracket(t, None, 3), None);
    }
}

#[cfg(test)]
mod bench {
    use super::*;

    #[test]
    #[ignore = "timing probe; run with --release --nocapture"]
    fn highlight_and_layout_5000_lines() {
        let line = "SELECT o.id, c.name, sum(i.quantity * i.unit_price) FROM sales.orders o JOIN sales.customers c ON c.id = o.customer_id -- note\n";
        let text = line.repeat(5000);
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut out = ctx.run_ui(Default::default(), |_| {});
        out.textures_delta.clear();
        let key = Key {
            text: &text,
            engine: None,
            dark: true,
            brackets: None,
            font_size_bits: 14f32.to_bits(),
        };
        let t = std::time::Instant::now();
        let toks = super::super::lex::tokenize(&text, None);
        eprintln!("tokenize: {:?} ({} tokens)", t.elapsed(), toks.len());
        for _ in 0..3 {
            let t = std::time::Instant::now();
            let n = toks
                .iter()
                .enumerate()
                .filter(|(i, t)| {
                    let s = &text[t.range.clone()];
                    t.kind == Kind::Word && (is_keyword(s) || next_is_open(&toks[i + 1..]))
                })
                .count();
            eprintln!("classify only: {:?} ({n})", t.elapsed());
        }
        let t = std::time::Instant::now();
        let job = layout(key);
        eprintln!(
            "layout job: {:?} ({} sections)",
            t.elapsed(),
            job.sections.len()
        );
        let t = std::time::Instant::now();
        let job2 = job.clone();
        eprintln!("clone job: {:?} {}", t.elapsed(), job2.sections.len());
        let t = std::time::Instant::now();
        let galley = ctx.fonts_mut(|f| f.layout_job(job));
        eprintln!("galley: {:?} ({} rows)", t.elapsed(), galley.rows.len());
    }
}
