//! What dropping a schema object into the editor inserts. On a blank line it
//! becomes a whole statement; inside a statement only its name (procedures
//! get their `EXEC`/`CALL` unless one is already there).

use sqail_client::proto::Engine;

use super::format::{Style, format};
use super::lex::{Kind, tokenize};
use super::{qualified_name, quote_ident};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    Table,
    View,
    Procedure,
    Function,
}

/// A schema object being dragged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropItem {
    pub engine: Engine,
    pub schema: String,
    pub name: String,
    pub kind: ObjectKind,
}

impl DropItem {
    pub fn qualified(&self) -> String {
        qualified_name(self.engine, Some(&self.schema), &self.name)
    }
}

/// Where the drop lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spot {
    /// A blank line, or below the text.
    Line,
    /// Inside a statement. `after_call`: right after `EXEC`/`EXECUTE`/`CALL`.
    Inline { after_call: bool },
}

/// The spot for a drop at byte `at` of `text`.
pub fn spot(text: &str, at: usize, engine: Engine) -> Spot {
    let at = at.min(text.len());
    let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[at..].find('\n').map_or(text.len(), |i| at + i);
    if text[line_start..line_end].trim().is_empty() {
        return Spot::Line;
    }
    let before = &text[..at];
    let after_call = tokenize(before, Some(engine))
        .iter()
        .rev()
        .find(|t| !matches!(t.kind, Kind::Space | Kind::Comment))
        .is_some_and(|t| {
            let w = &before[t.range.clone()];
            t.kind == Kind::Word
                && ["EXEC", "EXECUTE", "CALL"]
                    .iter()
                    .any(|k| w.eq_ignore_ascii_case(k))
        });
    Spot::Inline { after_call }
}

/// The text to insert for `item` at `spot`. `columns` (when known) spell out
/// the select list; otherwise it is `*`.
pub fn text_for(item: &DropItem, spot: Spot, columns: Option<&[String]>, style: &Style) -> String {
    let name = item.qualified();
    let engine = item.engine;
    let call = |name: &str| match engine {
        Engine::Mssql => format!("EXEC {name}"),
        _ => format!("CALL {name}()"),
    };
    match (item.kind, spot) {
        (ObjectKind::Table | ObjectKind::View, Spot::Inline { .. }) => name,
        (ObjectKind::Table | ObjectKind::View, Spot::Line) => {
            let list = match columns {
                Some(cols) if !cols.is_empty() => cols
                    .iter()
                    .map(|c| quote_ident(engine, c))
                    .collect::<Vec<_>>()
                    .join(", "),
                _ => "*".to_string(),
            };
            let sql = format!("SELECT {list} FROM {name};");
            format(&sql, Some(engine), style).unwrap_or(sql)
        }
        (ObjectKind::Procedure, Spot::Inline { after_call: true }) => match engine {
            Engine::Mssql => name,
            _ => format!("{name}()"),
        },
        (ObjectKind::Procedure, Spot::Inline { after_call: false }) => call(&name),
        (ObjectKind::Procedure, Spot::Line) => format!("{};", call(&name)),
        (ObjectKind::Function, Spot::Inline { .. }) => format!("{name}()"),
        (ObjectKind::Function, Spot::Line) => format!("SELECT {name}();"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(engine: Engine, kind: ObjectKind) -> DropItem {
        DropItem {
            engine,
            schema: if engine == Engine::Mssql {
                "dbo"
            } else {
                "sales"
            }
            .into(),
            name: "Orders".into(),
            kind,
        }
    }

    const STYLE: Style = Style {
        uppercase: true,
        indent: 2,
    };

    #[test]
    fn spots() {
        let pg = Engine::Postgres;
        assert_eq!(spot("", 0, pg), Spot::Line);
        assert_eq!(spot("SELECT 1;\n\n", 11, pg), Spot::Line);
        assert_eq!(spot("SELECT 1;\n   \nx", 12, pg), Spot::Line);
        assert_eq!(
            spot("SELECT * FROM ", 14, pg),
            Spot::Inline { after_call: false }
        );
        assert_eq!(
            spot("exec  ", 6, Engine::Mssql),
            Spot::Inline { after_call: true }
        );
    }

    #[test]
    fn tables_inline_are_just_the_name() {
        let t = item(Engine::Postgres, ObjectKind::Table);
        let s = Spot::Inline { after_call: false };
        assert_eq!(text_for(&t, s, None, &STYLE), "sales.\"Orders\"");
        let v = item(Engine::Mssql, ObjectKind::View);
        assert_eq!(text_for(&v, s, None, &STYLE), "[dbo].[Orders]");
    }

    #[test]
    fn tables_on_a_blank_line_are_a_formatted_select() {
        let t = item(Engine::Postgres, ObjectKind::Table);
        let cols = ["id".to_string(), "Status".to_string()];
        assert_eq!(
            text_for(&t, Spot::Line, Some(&cols), &STYLE),
            "SELECT\n  id,\n  \"Status\"\nFROM\n  sales.\"Orders\";"
        );
        let v = item(Engine::Mssql, ObjectKind::View);
        assert_eq!(
            text_for(&v, Spot::Line, None, &STYLE),
            "SELECT\n  *\nFROM\n  [dbo].[Orders];"
        );
    }

    #[test]
    fn procedures_become_calls() {
        let p = item(Engine::Mssql, ObjectKind::Procedure);
        assert_eq!(
            text_for(&p, Spot::Line, None, &STYLE),
            "EXEC [dbo].[Orders];"
        );
        let inline = Spot::Inline { after_call: false };
        assert_eq!(text_for(&p, inline, None, &STYLE), "EXEC [dbo].[Orders]");
        let after = Spot::Inline { after_call: true };
        assert_eq!(text_for(&p, after, None, &STYLE), "[dbo].[Orders]");
        let p = item(Engine::Postgres, ObjectKind::Procedure);
        assert_eq!(
            text_for(&p, Spot::Line, None, &STYLE),
            "CALL sales.\"Orders\"();"
        );
        let f = item(Engine::Postgres, ObjectKind::Function);
        assert_eq!(
            text_for(&f, Spot::Line, None, &STYLE),
            "SELECT sales.\"Orders\"();"
        );
    }
}
