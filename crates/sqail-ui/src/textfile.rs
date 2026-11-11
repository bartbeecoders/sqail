//! Reading and writing `.sql` files: encoding, byte-order mark and line
//! endings.
//!
//! The editor buffer is always UTF-8 with `\n` line endings and no BOM. What
//! the file had on disk is kept in a [`FileFormat`] next to the tab and put
//! back on save, so saving an untouched file writes the same bytes.

use serde::{Deserialize, Serialize};

/// The character encoding of a file on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Encoding {
    #[default]
    #[serde(rename = "utf-8")]
    Utf8,
    #[serde(rename = "utf-16le")]
    Utf16Le,
    #[serde(rename = "utf-16be")]
    Utf16Be,
    /// The fallback for bytes that are not UTF-8 and have no BOM (an "ANSI"
    /// script from a Western-European Windows system).
    #[serde(rename = "windows-1252")]
    Windows1252,
}

impl Encoding {
    pub fn label(self) -> &'static str {
        match self {
            Encoding::Utf8 => "UTF-8",
            Encoding::Utf16Le => "UTF-16 LE",
            Encoding::Utf16Be => "UTF-16 BE",
            Encoding::Windows1252 => "Windows-1252",
        }
    }
}

/// How a file is written to disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileFormat {
    pub encoding: Encoding,
    /// Starts with a byte-order mark.
    pub bom: bool,
    /// Lines end in `\r\n` rather than `\n`.
    pub crlf: bool,
}

impl Default for FileFormat {
    /// New files: UTF-8 without BOM, with the platform's line ending.
    fn default() -> Self {
        Self {
            encoding: Encoding::Utf8,
            bom: false,
            crlf: cfg!(windows),
        }
    }
}

const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

/// Windows-1252 bytes 0x80..=0x9F. The five bytes the code page leaves
/// undefined (0x81, 0x8D, 0x8F, 0x90, 0x9D) map to the C1 control of the same
/// value, so every byte decodes and encodes back to itself.
const CP1252_HIGH: [char; 32] = [
    '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008D}', '\u{017D}', '\u{008F}',
    '\u{0090}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
];

fn cp1252_decode(b: u8) -> char {
    match b {
        0x80..=0x9F => CP1252_HIGH[usize::from(b - 0x80)],
        _ => char::from(b),
    }
}

fn cp1252_encode(c: char) -> Option<u8> {
    match u32::from(c) {
        n @ (0..=0x7F | 0xA0..=0xFF) => u8::try_from(n).ok(),
        _ => CP1252_HIGH
            .iter()
            .position(|&h| h == c)
            .and_then(|i| u8::try_from(0x80 + i).ok()),
    }
}

/// Decode a file's bytes into editor text (`\n` line endings, no BOM) and
/// the format to write it back in.
pub fn decode(bytes: &[u8]) -> Result<(String, FileFormat), String> {
    let (encoding, bom, text) = if let Some(rest) = bytes.strip_prefix(UTF8_BOM) {
        let text = std::str::from_utf8(rest)
            .map_err(|e| format!("the file has a UTF-8 byte-order mark but is not UTF-8: {e}"))?;
        (Encoding::Utf8, true, text.to_string())
    } else if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        (Encoding::Utf16Le, true, utf16(rest, u16::from_le_bytes)?)
    } else if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        (Encoding::Utf16Be, true, utf16(rest, u16::from_be_bytes)?)
    } else {
        match std::str::from_utf8(bytes) {
            Ok(text) => (Encoding::Utf8, false, text.to_string()),
            Err(_) => (
                Encoding::Windows1252,
                false,
                bytes.iter().map(|&b| cp1252_decode(b)).collect(),
            ),
        }
    };
    let crlf_count = text.matches("\r\n").count();
    let lf_count = text.matches('\n').count() - crlf_count;
    let crlf = match crlf_count.cmp(&lf_count) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        // No line endings (or a tie): the platform's.
        std::cmp::Ordering::Equal if crlf_count == 0 => FileFormat::default().crlf,
        std::cmp::Ordering::Equal => true,
    };
    let text = if crlf_count > 0 {
        text.replace("\r\n", "\n")
    } else {
        text
    };
    Ok((
        text,
        FileFormat {
            encoding,
            bom,
            crlf,
        },
    ))
}

fn utf16(bytes: &[u8], word: fn([u8; 2]) -> u16) -> Result<String, String> {
    let (pairs, rest) = bytes.as_chunks::<2>();
    if !rest.is_empty() {
        return Err("the file has a UTF-16 byte-order mark but an odd length".into());
    }
    let units = pairs.iter().map(|&c| word(c));
    char::decode_utf16(units)
        .collect::<Result<String, _>>()
        .map_err(|e| format!("the file is not valid UTF-16: {e}"))
}

/// Can `text` be written in `format`'s encoding? Only Windows-1252 can fail.
pub fn can_encode(text: &str, format: &FileFormat) -> bool {
    format.encoding != Encoding::Windows1252 || text.chars().all(|c| cp1252_encode(c).is_some())
}

/// The bytes to write for editor text `text`. Characters the encoding
/// cannot hold (check [`can_encode`] first) are written as `?`.
pub fn encode(text: &str, format: &FileFormat) -> Vec<u8> {
    let owned;
    let text = if format.crlf {
        owned = text.replace('\n', "\r\n");
        &owned
    } else {
        text
    };
    let mut out = Vec::with_capacity(text.len() + 3);
    match format.encoding {
        Encoding::Utf8 => {
            if format.bom {
                out.extend_from_slice(UTF8_BOM);
            }
            out.extend_from_slice(text.as_bytes());
        }
        Encoding::Utf16Le | Encoding::Utf16Be => {
            let le = format.encoding == Encoding::Utf16Le;
            let mut put =
                |u: u16| out.extend_from_slice(&if le { u.to_le_bytes() } else { u.to_be_bytes() });
            if format.bom {
                put(0xFEFF);
            }
            text.encode_utf16().for_each(put);
        }
        Encoding::Windows1252 => {
            out.extend(text.chars().map(|c| cp1252_encode(c).unwrap_or(b'?')));
        }
    }
    out
}

/// Read and decode a file.
pub fn read(path: &std::path::Path) -> Result<(String, FileFormat), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    decode(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(bytes: &[u8]) -> (String, FileFormat) {
        let (text, format) = decode(bytes).unwrap();
        assert_eq!(encode(&text, &format), bytes, "{format:?}");
        (text, format)
    }

    #[test]
    fn crlf_is_normalised_and_restored() {
        let (text, f) = round_trip(b"SELECT 1\r\nSELECT 2\r\n");
        assert_eq!(text, "SELECT 1\nSELECT 2\n");
        assert!(f.crlf);
        let (text, f) = round_trip(b"SELECT 1\nSELECT 2\n");
        assert_eq!(text, "SELECT 1\nSELECT 2\n");
        assert!(!f.crlf);
    }

    #[test]
    fn mixed_line_endings_follow_the_majority() {
        let (text, f) = decode(b"a\r\nb\r\nc\nd").unwrap();
        assert_eq!(text, "a\nb\nc\nd");
        assert!(f.crlf);
        assert_eq!(encode(&text, &f), b"a\r\nb\r\nc\r\nd");
        let (_, f) = decode(b"a\nb\nc\r\nd").unwrap();
        assert!(!f.crlf);
    }

    #[test]
    fn no_line_endings_use_the_platform_default() {
        let (_, f) = round_trip(b"SELECT 1");
        assert_eq!(f.crlf, cfg!(windows));
        assert_eq!(f, FileFormat::default());
    }

    #[test]
    fn utf8_bom_is_stripped_and_restored() {
        let (text, f) = round_trip(b"\xEF\xBB\xBFSELECT 1;\r\n");
        assert_eq!(text, "SELECT 1;\n");
        assert_eq!(f.encoding, Encoding::Utf8);
        assert!(f.bom && f.crlf);
    }

    #[test]
    fn utf16_with_bom_decodes() {
        let mut le = vec![0xFF, 0xFE];
        let mut be = vec![0xFE, 0xFF];
        for u in "SELECT 'é€😀'\r\n".encode_utf16() {
            le.extend_from_slice(&u.to_le_bytes());
            be.extend_from_slice(&u.to_be_bytes());
        }
        let (text, f) = round_trip(&le);
        assert_eq!(text, "SELECT 'é€😀'\n");
        assert_eq!((f.encoding, f.bom, f.crlf), (Encoding::Utf16Le, true, true));
        let (text, f) = round_trip(&be);
        assert_eq!(text, "SELECT 'é€😀'\n");
        assert_eq!(f.encoding, Encoding::Utf16Be);
        assert!(decode(&[0xFF, 0xFE, 0x41]).is_err(), "odd length");
        assert!(decode(&[0xFF, 0xFE, 0x00, 0xD8]).is_err(), "lone surrogate");
    }

    #[test]
    fn non_utf8_falls_back_to_windows_1252() {
        // "café – €" in Windows-1252.
        let (text, f) = round_trip(b"caf\xE9 \x96 \x80\r\n");
        assert_eq!(text, "caf\u{e9} \u{2013} \u{20ac}\n");
        assert_eq!(f.encoding, Encoding::Windows1252);
        assert!(!f.bom);
        // Every byte value survives a round trip, the undefined ones too.
        let all: Vec<u8> = (0..=255u8).filter(|&b| b != b'\r').collect();
        round_trip(&all);
    }

    #[test]
    fn characters_outside_windows_1252_are_reported() {
        let f = FileFormat {
            encoding: Encoding::Windows1252,
            bom: false,
            crlf: false,
        };
        assert!(can_encode("café € ™", &f));
        assert!(!can_encode("SELECT '😀'", &f));
        assert_eq!(encode("a😀", &f), b"a?");
        assert!(can_encode("😀", &FileFormat::default()));
    }

    #[test]
    fn format_serialises_readably() {
        let f = FileFormat {
            encoding: Encoding::Utf16Le,
            bom: true,
            crlf: true,
        };
        assert_eq!(
            serde_json::to_string(&f).unwrap(),
            r#"{"encoding":"utf-16le","bom":true,"crlf":true}"#
        );
    }
}
