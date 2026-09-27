//! Every user action as a named command, with default shortcuts that
//! `keybindings.toml` can override. Menus, the command palette and keyboard
//! handling all go through here.

use std::collections::BTreeMap;

use egui::{Key, KeyboardShortcut, Modifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Command {
    RunCurrent,
    RunScript,
    Cancel,
    Explain,
    ExplainAnalyze,
    Commit,
    Rollback,
    ToggleAutocommit,
    NewTab,
    CloseTab,
    NextTab,
    PrevTab,
    OpenFile,
    Save,
    SaveAs,
    Find,
    Format,
    SaveSnippet,
    CommandPalette,
    QuickOpen,
    FontBigger,
    FontSmaller,
    ToggleTheme,
    ShowConnections,
    ShowHistory,
    ShowSnippets,
    NewConnection,
    RefreshConnections,
    ConnectService,
}

impl Command {
    pub const ALL: [Command; 29] = [
        Command::RunCurrent,
        Command::RunScript,
        Command::Cancel,
        Command::Explain,
        Command::ExplainAnalyze,
        Command::Commit,
        Command::Rollback,
        Command::ToggleAutocommit,
        Command::NewTab,
        Command::CloseTab,
        Command::NextTab,
        Command::PrevTab,
        Command::OpenFile,
        Command::Save,
        Command::SaveAs,
        Command::Find,
        Command::Format,
        Command::SaveSnippet,
        Command::CommandPalette,
        Command::QuickOpen,
        Command::FontBigger,
        Command::FontSmaller,
        Command::ToggleTheme,
        Command::ShowConnections,
        Command::ShowHistory,
        Command::ShowSnippets,
        Command::NewConnection,
        Command::RefreshConnections,
        Command::ConnectService,
    ];

    /// Stable id used in `keybindings.toml`.
    pub fn id(self) -> &'static str {
        match self {
            Command::RunCurrent => "query.run",
            Command::RunScript => "query.run_script",
            Command::Cancel => "query.cancel",
            Command::Explain => "query.explain",
            Command::ExplainAnalyze => "query.explain_analyze",
            Command::Commit => "transaction.commit",
            Command::Rollback => "transaction.rollback",
            Command::ToggleAutocommit => "transaction.toggle_autocommit",
            Command::NewTab => "tab.new",
            Command::CloseTab => "tab.close",
            Command::NextTab => "tab.next",
            Command::PrevTab => "tab.previous",
            Command::OpenFile => "file.open",
            Command::Save => "file.save",
            Command::SaveAs => "file.save_as",
            Command::Find => "edit.find",
            Command::Format => "edit.format",
            Command::SaveSnippet => "edit.save_snippet",
            Command::CommandPalette => "view.command_palette",
            Command::QuickOpen => "view.quick_open",
            Command::FontBigger => "view.font_bigger",
            Command::FontSmaller => "view.font_smaller",
            Command::ToggleTheme => "view.toggle_theme",
            Command::ShowConnections => "view.connections",
            Command::ShowHistory => "view.history",
            Command::ShowSnippets => "view.snippets",
            Command::NewConnection => "connection.new",
            Command::RefreshConnections => "connection.refresh",
            Command::ConnectService => "service.connect",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Command::RunCurrent => "Run statement or selection",
            Command::RunScript => "Run script",
            Command::Cancel => "Cancel query",
            Command::Explain => "Explain query plan",
            Command::ExplainAnalyze => "Explain analyze (runs the statement, rolled back)",
            Command::Commit => "Commit transaction",
            Command::Rollback => "Roll back transaction",
            Command::ToggleAutocommit => "Toggle auto-commit",
            Command::NewTab => "New tab",
            Command::CloseTab => "Close tab",
            Command::NextTab => "Next tab",
            Command::PrevTab => "Previous tab",
            Command::OpenFile => "Open file…",
            Command::Save => "Save",
            Command::SaveAs => "Save as…",
            Command::Find => "Find / replace",
            Command::Format => "Format SQL",
            Command::SaveSnippet => "Save as snippet…",
            Command::CommandPalette => "Command palette",
            Command::QuickOpen => "Open table or snippet…",
            Command::FontBigger => "Larger editor font",
            Command::FontSmaller => "Smaller editor font",
            Command::ToggleTheme => "Toggle light / dark theme",
            Command::ShowConnections => "Show connections",
            Command::ShowHistory => "Show query history",
            Command::ShowSnippets => "Show snippets",
            Command::NewConnection => "New connection…",
            Command::RefreshConnections => "Refresh connections",
            Command::ConnectService => "Connect to a service…",
        }
    }

    fn default_shortcuts(self) -> &'static [&'static str] {
        match self {
            Command::RunCurrent => &["Ctrl+Enter"],
            Command::RunScript => &["F5", "Ctrl+Shift+Enter"],
            Command::Cancel => &["Escape"],
            Command::Explain => &["Ctrl+E"],
            Command::NewTab => &["Ctrl+T"],
            Command::CloseTab => &["Ctrl+W"],
            Command::NextTab => &["Ctrl+PageDown"],
            Command::PrevTab => &["Ctrl+PageUp"],
            Command::OpenFile => &["Ctrl+O"],
            Command::Save => &["Ctrl+S"],
            Command::SaveAs => &["Ctrl+Shift+S"],
            Command::Find => &["Ctrl+F"],
            Command::Format => &["Ctrl+Shift+F"],
            Command::CommandPalette => &["Ctrl+Shift+P"],
            Command::QuickOpen => &["Ctrl+P"],
            Command::FontBigger => &["Ctrl+Equals", "Ctrl+Plus"],
            Command::FontSmaller => &["Ctrl+Minus"],
            _ => &[],
        }
    }
}

/// `Ctrl+Shift+P`, `F5`, `Alt+Enter`, … (`Ctrl` and `Cmd` both mean the
/// platform's command key).
pub fn parse_shortcut(s: &str) -> Option<KeyboardShortcut> {
    let mut modifiers = Modifiers::NONE;
    let parts: Vec<&str> = s.split('+').map(str::trim).collect();
    let (key, mods) = match parts.split_last()? {
        // `Ctrl++` → the last two parts are "" and "".
        (&"", rest) if rest.last() == Some(&"") => ("+", &rest[..rest.len() - 1]),
        (k, rest) => (*k, rest),
    };
    for m in mods {
        match m.to_ascii_lowercase().as_str() {
            "ctrl" | "cmd" | "command" | "control" => modifiers |= Modifiers::COMMAND,
            "shift" => modifiers |= Modifiers::SHIFT,
            "alt" | "option" => modifiers |= Modifiers::ALT,
            _ => return None,
        }
    }
    let key = Key::from_name(key)
        .or_else(|| Key::from_name(&key.to_uppercase()))
        .or_else(|| Key::from_name(&capitalize(key)))?;
    Some(KeyboardShortcut::new(modifiers, key))
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}

pub struct Keymap {
    /// Most specific (most modifiers) first, so Ctrl+Shift+S wins over Ctrl+S.
    bindings: Vec<(KeyboardShortcut, Command)>,
    /// Problems in keybindings.toml, shown to the user once.
    pub warnings: Vec<String>,
}

impl Keymap {
    pub fn defaults() -> Self {
        Self::build(&BTreeMap::new())
    }

    /// Defaults, with `keybindings.toml` (command id → shortcut or list) on top.
    pub fn load() -> Self {
        let path = crate::settings::config_dir().join("keybindings.toml");
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::defaults();
        };
        match toml::from_str::<BTreeMap<String, toml::Value>>(&text) {
            Ok(map) => Self::build(&map),
            Err(e) => {
                let mut k = Self::defaults();
                k.warnings.push(format!("keybindings.toml: {e}"));
                k
            }
        }
    }

    fn build(overrides: &BTreeMap<String, toml::Value>) -> Self {
        let mut warnings = Vec::new();
        for id in overrides.keys() {
            if !Command::ALL.iter().any(|c| c.id() == id) {
                warnings.push(format!("keybindings.toml: unknown command “{id}”"));
            }
        }
        let mut bindings = Vec::new();
        for cmd in Command::ALL {
            let shortcuts: Vec<String> = match overrides.get(cmd.id()) {
                Some(toml::Value::String(s)) => vec![s.clone()],
                Some(toml::Value::Array(a)) => a
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect(),
                Some(_) => {
                    warnings.push(format!(
                        "keybindings.toml: {} must be a string or list",
                        cmd.id()
                    ));
                    continue;
                }
                None => cmd
                    .default_shortcuts()
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            };
            for s in shortcuts {
                match parse_shortcut(&s) {
                    Some(sc) => bindings.push((sc, cmd)),
                    None => warnings.push(format!(
                        "keybindings.toml: cannot parse shortcut “{s}” for {}",
                        cmd.id()
                    )),
                }
            }
        }
        let weight = |m: Modifiers| m.command as u8 + m.shift as u8 + m.alt as u8;
        bindings.sort_by_key(|(sc, _)| std::cmp::Reverse(weight(sc.modifiers)));
        Self { bindings, warnings }
    }

    pub fn bindings(&self) -> &[(KeyboardShortcut, Command)] {
        &self.bindings
    }

    /// The first shortcut bound to `cmd`.
    pub fn shortcut(&self, cmd: Command) -> Option<KeyboardShortcut> {
        self.bindings
            .iter()
            .find(|(_, c)| *c == cmd)
            .map(|(s, _)| *s)
    }

    pub fn label(&self, ctx: &egui::Context, cmd: Command) -> String {
        self.shortcut(cmd)
            .map(|s| ctx.format_shortcut(&s))
            .unwrap_or_default()
    }
}

/// Subsequence match score for the palettes: higher is better, None = no match.
pub fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let q: Vec<char> = query.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let mut score = 0;
    let mut qi = 0;
    let mut last_match: Option<usize> = None;
    for (ti, &c) in t.iter().enumerate() {
        if qi < q.len() && c == q[qi] {
            score += 10;
            if last_match == Some(ti.wrapping_sub(1)) {
                score += 15; // consecutive
            }
            if ti == 0 || !t[ti - 1].is_alphanumeric() {
                score += 20; // word start
            }
            last_match = Some(ti);
            qi += 1;
        }
    }
    (qi == q.len()).then_some(score - t.len() as i32 / 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_parse() {
        assert_eq!(
            parse_shortcut("Ctrl+Shift+P"),
            Some(KeyboardShortcut::new(
                Modifiers::COMMAND | Modifiers::SHIFT,
                Key::P
            ))
        );
        assert_eq!(
            parse_shortcut("F5"),
            Some(KeyboardShortcut::new(Modifiers::NONE, Key::F5))
        );
        assert_eq!(
            parse_shortcut("alt+enter"),
            Some(KeyboardShortcut::new(Modifiers::ALT, Key::Enter))
        );
        assert_eq!(parse_shortcut("Ctrl+Hyper+X"), None);
        assert_eq!(parse_shortcut("Ctrl+NoSuchKey"), None);
    }

    #[test]
    fn every_command_has_a_unique_id_and_defaults_parse() {
        let mut ids: Vec<&str> = Command::ALL.iter().map(|c| c.id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), Command::ALL.len());
        let k = Keymap::defaults();
        assert!(k.warnings.is_empty(), "{:?}", k.warnings);
    }

    #[test]
    fn overrides_replace_defaults_and_report_mistakes() {
        let map: BTreeMap<String, toml::Value> = toml::from_str(
            r#"
            "query.run" = "Ctrl+R"
            "tab.new" = ["Ctrl+N", "Ctrl+T"]
            "nope" = "F1"
            "#,
        )
        .unwrap();
        let k = Keymap::build(&map);
        assert_eq!(k.shortcut(Command::RunCurrent), parse_shortcut("Ctrl+R"));
        assert_eq!(
            k.bindings()
                .iter()
                .filter(|(_, c)| *c == Command::NewTab)
                .count(),
            2
        );
        assert_eq!(k.warnings.len(), 1);
    }

    #[test]
    fn specific_shortcuts_come_first() {
        let k = Keymap::defaults();
        let pos = |s: &str| {
            k.bindings()
                .iter()
                .position(|(sc, _)| Some(*sc) == parse_shortcut(s))
                .unwrap()
        };
        assert!(pos("Ctrl+Shift+S") < pos("Ctrl+S"));
        assert!(pos("Ctrl+Shift+Enter") < pos("Ctrl+Enter"));
    }

    #[test]
    fn fuzzy_matching() {
        assert!(
            fuzzy_score("rs", "Run script").unwrap()
                > fuzzy_score("rs", "Toggle light / dark theme").unwrap_or(i32::MIN)
        );
        assert!(fuzzy_score("xyz", "Run script").is_none());
        assert!(fuzzy_score("ord", "sales.orders").is_some());
    }
}
