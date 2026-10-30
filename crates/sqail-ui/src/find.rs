//! Plain-text search for the editor's find bar. Pure functions on byte
//! offsets, so they are easy to test and need no copy of the buffer.
//! Case-insensitive search compares characters case-folded one by one
//! instead of lowercasing the text, so offsets always point into `text`
//! even where lowercasing changes the byte length.

use std::ops::Range;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Opts {
    pub match_case: bool,
    /// Search towards the start of the text.
    pub backwards: bool,
}

/// The end of a match of `query` starting at byte `at`, if there is one.
fn match_at(text: &str, at: usize, query: &str, match_case: bool) -> Option<usize> {
    let rest = text.get(at..)?;
    if match_case {
        return rest.starts_with(query).then(|| at + query.len());
    }
    let mut chars = rest.char_indices();
    for q in query.chars() {
        let (_, c) = chars.next()?;
        if c != q && !c.to_lowercase().eq(q.to_lowercase()) {
            return None;
        }
    }
    Some(chars.next().map_or(text.len(), |(i, _)| at + i))
}

/// Every non-overlapping match of `query` in `text`, in order.
pub fn matches<'a>(
    text: &'a str,
    query: &'a str,
    match_case: bool,
) -> impl Iterator<Item = Range<usize>> + 'a {
    let mut pos = 0;
    std::iter::from_fn(move || {
        if query.is_empty() {
            return None;
        }
        while pos <= text.len() {
            if match_case {
                let p = text[pos..].find(query)? + pos;
                pos = p + query.len();
                return Some(p..pos);
            }
            if let Some(end) = match_at(text, pos, query, false) {
                let start = pos;
                pos = end;
                return Some(start..end);
            }
            // Next character boundary.
            pos += text[pos..].chars().next().map_or(1, char::len_utf8);
        }
        None
    })
}

/// Whether `range` is exactly a match of `query` (e.g. the current selection).
pub fn is_match(text: &str, range: &Range<usize>, query: &str, match_case: bool) -> bool {
    !query.is_empty()
        && text.is_char_boundary(range.start)
        && match_at(text, range.start, query, match_case) == Some(range.end)
}

/// The next match of `query` searching from byte `from`, wrapping around.
/// Forwards: the first match starting at or after `from`. Backwards: the
/// last match starting before `from`.
pub fn find(text: &str, query: &str, from: usize, opts: Opts) -> Option<Range<usize>> {
    let mut all = matches(text, query, opts.match_case);
    if opts.backwards {
        let mut last = None;
        let mut before = None;
        for m in all {
            if m.start < from {
                before = Some(m.clone());
            }
            last = Some(m);
        }
        before.or(last)
    } else {
        let first = all.next()?;
        if first.start >= from {
            return Some(first);
        }
        all.find(|m| m.start >= from).or(Some(first))
    }
}

/// `text` with every match of `query` replaced, and the number replaced.
pub fn replace_all(text: &str, query: &str, with: &str, match_case: bool) -> (String, usize) {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    let mut n = 0;
    for m in matches(text, query, match_case) {
        out.push_str(&text[last..m.start]);
        out.push_str(with);
        last = m.end;
        n += 1;
    }
    out.push_str(&text[last..]);
    (out, n)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FWD: Opts = Opts {
        match_case: false,
        backwards: false,
    };
    const BACK: Opts = Opts {
        match_case: false,
        backwards: true,
    };

    #[test]
    fn forwards_wraps_around() {
        let t = "select a from t where a = 'A'";
        assert_eq!(find(t, "a", 0, FWD), Some(7..8));
        assert_eq!(find(t, "a", 8, FWD), Some(22..23));
        assert_eq!(find(t, "a", 23, FWD), Some(27..28), "case-insensitive");
        assert_eq!(find(t, "a", 28, FWD), Some(7..8), "wraps");
        let exact = Opts {
            match_case: true,
            ..FWD
        };
        assert_eq!(find(t, "A", 0, exact), Some(27..28));
        assert_eq!(find(t, "zz", 0, FWD), None);
        assert_eq!(find(t, "", 0, FWD), None);
        assert_eq!(find("", "a", 0, FWD), None);
    }

    #[test]
    fn backwards_wraps_around() {
        let t = "ab ab ab";
        assert_eq!(find(t, "AB", 6, BACK), Some(3..5));
        assert_eq!(find(t, "ab", 3, BACK), Some(0..2));
        assert_eq!(find(t, "ab", 0, BACK), Some(6..8), "wraps to the last");
    }

    #[test]
    fn offsets_stay_right_when_lowercase_changes_length() {
        // 'İ' (2 bytes) lowercases to "i̇" (3 bytes); 'ẞ' (3) to 'ß' (2).
        let t = "İSTANBUL ẞ select";
        let m = find(t, "select", 0, FWD).unwrap();
        assert_eq!(&t[m], "select");
        let m = find(t, "SELECT", 0, FWD).unwrap();
        assert_eq!(&t[m.clone()], "select");
        let m = find(t, "ß", 0, FWD).unwrap();
        assert_eq!(&t[m], "ẞ");
        assert_eq!(find("café CAFÉ", "Café", 1, FWD), Some(6..11));
        let (out, n) = replace_all(t, "SELECT", "x", false);
        assert_eq!((out.as_str(), n), ("İSTANBUL ẞ x", 1));
    }

    #[test]
    fn replace_and_is_match() {
        let t = "a.Id = b.id";
        assert_eq!(
            replace_all(t, "id", "key", false),
            ("a.key = b.key".into(), 2)
        );
        assert_eq!(
            replace_all(t, "id", "key", true),
            ("a.Id = b.key".into(), 1)
        );
        assert!(is_match(t, &(2..4), "ID", false));
        assert!(!is_match(t, &(2..4), "ID", true));
        assert!(!is_match(t, &(2..3), "id", false));
        assert!(!is_match(t, &(2..4), "", false));
        // Matches don't overlap.
        assert_eq!(matches("aaaa", "aa", true).count(), 2);
        assert_eq!(matches("aaaa", "AA", false).count(), 2);
    }
}
