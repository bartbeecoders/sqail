//! Fonts and colour themes. A theme is a [`Palette`]: sqail's own light and
//! dark, a few well-known colour schemes, or the current Omarchy theme. egui
//! has one dark and one light style; a palette replaces the one matching its
//! brightness, and the editor's syntax colours come from it too.

use std::cell::RefCell;
use std::path::PathBuf;
use std::time::SystemTime;

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Stroke, Theme, Visuals};

use crate::settings::ThemePref;

pub fn engine_label(e: sqail_client::proto::Engine) -> &'static str {
    use sqail_client::proto::Engine;
    match e {
        Engine::Postgres => "PostgreSQL",
        Engine::Mssql => "SQL Server",
        Engine::Sqlite => "SQLite",
    }
}

/// The accent colour of the active dark or light palette.
pub fn accent(dark: bool) -> Color32 {
    with_active(dark, |p| p.accent)
}

/// Connection colour choices (name, hex). `None` = no colour.
pub const CONNECTION_COLORS: &[(&str, &str)] = &[
    ("green", "#27ae60"),
    ("blue", "#2980b9"),
    ("purple", "#8e44ad"),
    ("orange", "#e67e22"),
    ("red", "#c0392b"),
];

pub fn parse_hex(hex: &str) -> Option<Color32> {
    let h = hex.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "Inter".into(),
        FontData::from_static(include_bytes!("../assets/fonts/Inter.ttf")).into(),
    );
    fonts.font_data.insert(
        "JetBrainsMono".into(),
        FontData::from_static(include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf")).into(),
    );
    // Put ours first; keep egui's defaults behind them for missing glyphs.
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "Inter".into());
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "JetBrainsMono".into());
    ctx.set_fonts(fonts);

    for theme in [Theme::Dark, Theme::Light] {
        ctx.style_mut_of(theme, |style| {
            style.spacing.item_spacing = egui::vec2(8.0, 6.0);
            style.spacing.button_padding = egui::vec2(10.0, 4.0);
            style.spacing.interact_size.y = 26.0;
        });
    }
    set_palettes(ctx, SQAIL_DARK, SQAIL_LIGHT);
}

// --------------------------------------------------------------- palettes --

/// Every colour sqail picks itself; the rest of egui's style derives from it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub dark: bool,
    /// Panels (sidebar, toolbars).
    pub bg: Color32,
    /// Windows, dialogs and menus.
    pub surface: Color32,
    /// The editor, the grid and text fields.
    pub base: Color32,
    /// Striped rows, code backgrounds.
    pub faint: Color32,
    pub text: Color32,
    pub weak: Color32,
    pub accent: Color32,
    pub selection: Color32,
    pub red: Color32,
    pub yellow: Color32,
    pub keyword: Color32,
    pub string: Color32,
    pub number: Color32,
    pub comment: Color32,
    pub ident: Color32,
    pub func: Color32,
    /// Recolour egui's widgets from this palette. sqail's own light and dark
    /// keep egui's widget colours.
    pub tint_widgets: bool,
}

const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

pub const SQAIL_DARK: Palette = Palette {
    dark: true,
    bg: hex(0x1b1b1a),
    surface: hex(0x222220),
    base: hex(0x141413),
    faint: hex(0x242422),
    text: hex(0xdcdad4),
    weak: hex(0x8c8a84),
    accent: hex(0x6cc2a6),
    selection: hex(0x2f5a4c),
    red: hex(0xd64545),
    yellow: hex(0xe6a23c),
    keyword: hex(0x82aaff),
    string: hex(0xc3e88d),
    number: hex(0xf78c6c),
    comment: hex(0x7a7872),
    ident: hex(0xe6c07b),
    func: hex(0x89ddff),
    tint_widgets: false,
};

pub const SQAIL_LIGHT: Palette = Palette {
    dark: false,
    bg: hex(0xf5f4f1),
    surface: hex(0xfbfbf9),
    base: hex(0xffffff),
    faint: hex(0xeeece7),
    text: hex(0x242321),
    weak: hex(0x8a8780),
    accent: hex(0x2f6f5e),
    selection: hex(0xc9dbd5),
    red: hex(0xc0392b),
    yellow: hex(0xb7791f),
    keyword: hex(0x1f4fb8),
    string: hex(0x2e7d32),
    number: hex(0xb04a20),
    comment: hex(0x8a8780),
    ident: hex(0x8a5a00),
    func: hex(0x006b8f),
    tint_widgets: false,
};

const NORD: Palette = Palette {
    dark: true,
    bg: hex(0x2e3440),
    surface: hex(0x3b4252),
    base: hex(0x272c36),
    faint: hex(0x353b49),
    text: hex(0xd8dee9),
    weak: hex(0x8a93a5),
    accent: hex(0x88c0d0),
    selection: hex(0x434c5e),
    red: hex(0xbf616a),
    yellow: hex(0xebcb8b),
    keyword: hex(0x81a1c1),
    string: hex(0xa3be8c),
    number: hex(0xb48ead),
    comment: hex(0x6d7a96),
    ident: hex(0xebcb8b),
    func: hex(0x88c0d0),
    tint_widgets: true,
};

const TOKYO_NIGHT: Palette = Palette {
    dark: true,
    bg: hex(0x1a1b26),
    surface: hex(0x24283b),
    base: hex(0x16161e),
    faint: hex(0x1f2335),
    text: hex(0xc0caf5),
    weak: hex(0x737aa2),
    accent: hex(0x7aa2f7),
    selection: hex(0x33467c),
    red: hex(0xf7768e),
    yellow: hex(0xe0af68),
    keyword: hex(0xbb9af7),
    string: hex(0x9ece6a),
    number: hex(0xff9e64),
    comment: hex(0x565f89),
    ident: hex(0xe0af68),
    func: hex(0x7dcfff),
    tint_widgets: true,
};

const GRUVBOX: Palette = Palette {
    dark: true,
    bg: hex(0x282828),
    surface: hex(0x32302f),
    base: hex(0x1d2021),
    faint: hex(0x3c3836),
    text: hex(0xebdbb2),
    weak: hex(0xa89984),
    accent: hex(0x83a598),
    selection: hex(0x504945),
    red: hex(0xfb4934),
    yellow: hex(0xfabd2f),
    keyword: hex(0xfb4934),
    string: hex(0xb8bb26),
    number: hex(0xd3869b),
    comment: hex(0x928374),
    ident: hex(0xfabd2f),
    func: hex(0x8ec07c),
    tint_widgets: true,
};

const CATPPUCCIN_MOCHA: Palette = Palette {
    dark: true,
    bg: hex(0x1e1e2e),
    surface: hex(0x262637),
    base: hex(0x181825),
    faint: hex(0x313244),
    text: hex(0xcdd6f4),
    weak: hex(0x7f849c),
    accent: hex(0xcba6f7),
    selection: hex(0x45475a),
    red: hex(0xf38ba8),
    yellow: hex(0xf9e2af),
    keyword: hex(0xcba6f7),
    string: hex(0xa6e3a1),
    number: hex(0xfab387),
    comment: hex(0x6c7086),
    ident: hex(0xf9e2af),
    func: hex(0x89dceb),
    tint_widgets: true,
};

const CATPPUCCIN_LATTE: Palette = Palette {
    dark: false,
    bg: hex(0xe6e9ef),
    surface: hex(0xeff1f5),
    base: hex(0xf7f8fa),
    faint: hex(0xdce0e8),
    text: hex(0x4c4f69),
    weak: hex(0x8c8fa1),
    accent: hex(0x8839ef),
    selection: hex(0xccd0da),
    red: hex(0xd20f39),
    yellow: hex(0xdf8e1d),
    keyword: hex(0x8839ef),
    string: hex(0x40a02b),
    number: hex(0xfe640b),
    comment: hex(0x9ca0b0),
    ident: hex(0xdf8e1d),
    func: hex(0x04a5e5),
    tint_widgets: true,
};

const SOLARIZED_LIGHT: Palette = Palette {
    dark: false,
    bg: hex(0xeee8d5),
    surface: hex(0xfdf6e3),
    base: hex(0xfdf6e3),
    faint: hex(0xe6dfca),
    text: hex(0x586e75),
    weak: hex(0x93a1a1),
    accent: hex(0x268bd2),
    selection: hex(0xd9d2bd),
    red: hex(0xdc322f),
    yellow: hex(0xb58900),
    keyword: hex(0x859900),
    string: hex(0x2aa198),
    number: hex(0xd33682),
    comment: hex(0x93a1a1),
    ident: hex(0xb58900),
    func: hex(0x268bd2),
    tint_widgets: true,
};

impl ThemePref {
    /// The palette of a named theme; `None` for System/Light/Dark (sqail's
    /// own pair, picked by egui) and Omarchy (read from disk).
    fn palette(self) -> Option<Palette> {
        match self {
            ThemePref::System | ThemePref::Light | ThemePref::Dark | ThemePref::Omarchy => None,
            ThemePref::Nord => Some(NORD),
            ThemePref::TokyoNight => Some(TOKYO_NIGHT),
            ThemePref::Gruvbox => Some(GRUVBOX),
            ThemePref::CatppuccinMocha => Some(CATPPUCCIN_MOCHA),
            ThemePref::CatppuccinLatte => Some(CATPPUCCIN_LATTE),
            ThemePref::SolarizedLight => Some(SOLARIZED_LIGHT),
        }
    }
}

struct Active {
    dark: Palette,
    light: Palette,
    /// Bumped on every change, so cached highlighting is redone.
    generation: u64,
}

thread_local! {
    static ACTIVE: RefCell<Active> = const {
        RefCell::new(Active {
            dark: SQAIL_DARK,
            light: SQAIL_LIGHT,
            generation: 0,
        })
    };
}

fn with_active<T>(dark: bool, f: impl FnOnce(&Palette) -> T) -> T {
    ACTIVE.with(|a| {
        let a = a.borrow();
        f(if dark { &a.dark } else { &a.light })
    })
}

/// Changes whenever the palettes change.
pub fn generation() -> u64 {
    ACTIVE.with(|a| a.borrow().generation)
}

fn set_palettes(ctx: &egui::Context, dark: Palette, light: Palette) {
    for (theme, p) in [(Theme::Dark, dark), (Theme::Light, light)] {
        ctx.style_mut_of(theme, |style| style.visuals = visuals(&p));
    }
    ACTIVE.with(|a| {
        let mut a = a.borrow_mut();
        a.dark = dark;
        a.light = light;
        a.generation += 1;
    });
}

/// Show `pref`. Omarchy falls back to following the system when no Omarchy
/// theme is found; the error says so.
pub fn apply(ctx: &egui::Context, pref: ThemePref) -> Result<(), String> {
    let (palette, result) = match pref {
        ThemePref::Omarchy => match omarchy_palette() {
            Some(p) => (Some(p), Ok(())),
            None => (
                None,
                Err(format!(
                    "No Omarchy theme found ({}); following the system instead.",
                    omarchy_dir().join("colors.toml").display()
                )),
            ),
        },
        other => (other.palette(), Ok(())),
    };
    match palette {
        Some(p) if p.dark => {
            set_palettes(ctx, p, SQAIL_LIGHT);
            ctx.set_theme(egui::ThemePreference::Dark);
        }
        Some(p) => {
            set_palettes(ctx, SQAIL_DARK, p);
            ctx.set_theme(egui::ThemePreference::Light);
        }
        None => {
            set_palettes(ctx, SQAIL_DARK, SQAIL_LIGHT);
            ctx.set_theme(match pref {
                ThemePref::Light => egui::ThemePreference::Light,
                ThemePref::Dark => egui::ThemePreference::Dark,
                _ => egui::ThemePreference::System,
            });
        }
    }
    result
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

fn visuals(p: &Palette) -> Visuals {
    let mut v = if p.dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    for w in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
        &mut v.widgets.noninteractive,
    ] {
        w.corner_radius = CornerRadius::same(6);
    }
    v.panel_fill = p.bg;
    v.window_fill = p.surface;
    v.extreme_bg_color = p.base;
    v.faint_bg_color = p.faint;
    v.hyperlink_color = p.accent;
    if !p.tint_widgets {
        v.selection.bg_fill = p.accent.linear_multiply(if p.dark { 0.45 } else { 0.30 });
        v.selection.stroke = Stroke::new(1.0, p.accent);
        return v;
    }
    v.code_bg_color = p.faint;
    v.weak_text_color = Some(p.weak);
    v.error_fg_color = p.red;
    v.warn_fg_color = p.yellow;
    v.selection.bg_fill = p.selection;
    v.selection.stroke = Stroke::new(1.0, p.text);
    v.text_cursor.stroke = Stroke::new(2.0, p.accent);
    let border = mix(p.bg, p.text, 0.18);
    v.window_stroke = Stroke::new(1.0, border);
    let bright = mix(
        p.text,
        if p.dark {
            Color32::WHITE
        } else {
            Color32::BLACK
        },
        0.3,
    );
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.bg;
    w.noninteractive.weak_bg_fill = p.bg;
    w.noninteractive.bg_stroke = Stroke::new(1.0, border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    for (state, amount) in [
        (&mut w.inactive, 0.10),
        (&mut w.open, 0.14),
        (&mut w.hovered, 0.18),
        (&mut w.active, 0.26),
    ] {
        state.bg_fill = mix(p.surface, p.text, amount);
        state.weak_bg_fill = mix(p.surface, p.text, amount * 0.8);
        state.fg_stroke = Stroke::new(1.0, p.text);
    }
    w.inactive.bg_stroke = Stroke::NONE;
    w.open.bg_stroke = Stroke::new(1.0, border);
    w.hovered.bg_stroke = Stroke::new(1.0, mix(p.bg, p.text, 0.35));
    w.hovered.fg_stroke = Stroke::new(1.5, bright);
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(2.0, bright);
    v
}

// ---------------------------------------------------------------- omarchy --

/// `~/.local/state/omarchy/current/theme`, where Omarchy keeps the files of
/// the theme in use.
fn omarchy_dir() -> PathBuf {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| directories::BaseDirs::new().map(|d| d.home_dir().join(".local/state")))
        .unwrap_or_else(|| PathBuf::from(".local/state"));
    state.join("omarchy/current/theme")
}

/// Whether an Omarchy theme is installed (new installs default to it).
pub fn omarchy_available() -> bool {
    omarchy_dir().join("colors.toml").is_file()
}

/// Changes when the user switches Omarchy themes.
pub fn omarchy_stamp() -> Option<SystemTime> {
    std::fs::metadata(omarchy_dir().join("colors.toml"))
        .and_then(|m| m.modified())
        .ok()
}

fn omarchy_palette() -> Option<Palette> {
    let dir = omarchy_dir();
    let text = std::fs::read_to_string(dir.join("colors.toml")).ok()?;
    parse_omarchy(&text, dir.join("light.mode").exists())
}

/// An Omarchy `colors.toml` as a palette. `background` and `foreground` are
/// required; everything else has a fallback. Light themes say `mode =
/// "light"` or ship a `light.mode` file.
pub fn parse_omarchy(text: &str, light_mode_file: bool) -> Option<Palette> {
    let table: toml::Table = toml::from_str(text).ok()?;
    let color = |key: &str| table.get(key)?.as_str().and_then(parse_hex);
    let bg = color("background")?;
    let text = color("foreground")?;
    let dark = !light_mode_file && table.get("mode").and_then(|m| m.as_str()) != Some("light");
    let base_default = if dark { SQAIL_DARK } else { SQAIL_LIGHT };
    let accent = color("accent")
        .or(color("blue"))
        .unwrap_or(base_default.accent);
    let (base, surface) = if dark {
        (
            color("dark_background").unwrap_or(mix(bg, Color32::BLACK, 0.2)),
            color("lighter_background").unwrap_or(mix(bg, text, 0.05)),
        )
    } else {
        (mix(bg, Color32::WHITE, 0.55), mix(bg, Color32::WHITE, 0.35))
    };
    let yellow = color("yellow").unwrap_or(base_default.yellow);
    Some(Palette {
        dark,
        bg,
        surface,
        base,
        faint: mix(bg, text, 0.06),
        text,
        weak: mix(text, bg, 0.4),
        accent,
        selection: color("selection").unwrap_or(mix(bg, accent, 0.3)),
        red: color("red").unwrap_or(base_default.red),
        yellow,
        keyword: color("blue").unwrap_or(base_default.keyword),
        string: color("green").unwrap_or(base_default.string),
        number: color("orange")
            .or(color("magenta"))
            .unwrap_or(base_default.number),
        comment: color("muted").unwrap_or(mix(text, bg, 0.5)),
        ident: yellow,
        func: color("cyan").unwrap_or(base_default.func),
        tint_widgets: true,
    })
}

/// Syntax colours for the SQL editor.
pub struct SyntaxColors {
    pub text: Color32,
    pub keyword: Color32,
    pub string: Color32,
    pub number: Color32,
    pub comment: Color32,
    pub ident: Color32,
    pub func: Color32,
    pub bracket_bg: Color32,
}

pub fn syntax(dark: bool) -> SyntaxColors {
    with_active(dark, |p| SyntaxColors {
        text: p.text,
        keyword: p.keyword,
        string: p.string,
        number: p.number,
        comment: p.comment,
        ident: p.ident,
        func: p.func,
        bracket_bg: Color32::from_rgba_unmultiplied(
            p.accent.r(),
            p.accent.g(),
            p.accent.b(),
            if p.dark { 70 } else { 60 },
        ),
    })
}

/// Every icon glyph the UI uses; a test checks the bundled fonts have them.
pub const ICONS: &str = "⊞👁ƒ⚙⟳▶■●×…▲▼";

#[cfg(test)]
mod tests {
    use super::*;

    const NORD_COLORS: &str = r##"
mode = "dark"
accent = "#81a1c1"
selection = "#434c5e"
muted = "#4c566a"
background = "#2e3440"
dark_background = "#222730"
lighter_background = "#3b4252"
foreground = "#d8dee9"
red = "#bf616a"
yellow = "#ebcb8b"
orange = "#d5967a"
green = "#a3be8c"
cyan = "#88c0d0"
blue = "#81a1c1"
"##;

    #[test]
    fn omarchy_colors_become_a_palette() {
        let p = parse_omarchy(NORD_COLORS, false).unwrap();
        assert!(p.dark && p.tint_widgets);
        assert_eq!(p.bg, hex(0x2e3440));
        assert_eq!(p.base, hex(0x222730));
        assert_eq!(p.text, hex(0xd8dee9));
        assert_eq!(
            (p.keyword, p.string, p.number),
            (hex(0x81a1c1), hex(0xa3be8c), hex(0xd5967a))
        );

        // A light.mode file or mode = "light" makes it a light theme.
        assert!(!parse_omarchy(NORD_COLORS, true).unwrap().dark);
        let light = NORD_COLORS.replace(r#"mode = "dark""#, r#"mode = "light""#);
        assert!(!parse_omarchy(&light, false).unwrap().dark);

        // Only background and foreground are required.
        let minimal = "background = \"#ffffff\"\nforeground = \"#000000\"";
        let m = parse_omarchy(minimal, false).unwrap();
        assert_eq!(m.accent, SQAIL_DARK.accent);
        assert!(parse_omarchy("foreground = \"#000000\"", false).is_none());
        assert!(parse_omarchy("not toml", false).is_none());
    }

    #[test]
    fn named_themes_replace_the_matching_style_and_syntax() {
        let ctx = egui::Context::default();
        install(&ctx);
        let before = generation();
        apply(&ctx, ThemePref::Nord).unwrap();
        assert!(generation() > before, "highlighting is redone");
        assert_eq!(ctx.style_of(Theme::Dark).visuals.panel_fill, NORD.bg);
        assert_eq!(syntax(true).keyword, NORD.keyword);
        assert_eq!(
            accent(false),
            SQAIL_LIGHT.accent,
            "the light side is sqail's"
        );

        apply(&ctx, ThemePref::CatppuccinLatte).unwrap();
        assert_eq!(
            ctx.style_of(Theme::Light).visuals.panel_fill,
            CATPPUCCIN_LATTE.bg
        );
        assert_eq!(ctx.style_of(Theme::Dark).visuals.panel_fill, SQAIL_DARK.bg);

        apply(&ctx, ThemePref::Dark).unwrap();
        assert_eq!(syntax(true).keyword, SQAIL_DARK.keyword);
    }
    #[test]
    fn icons_render_with_bundled_fonts() {
        let ctx = egui::Context::default();
        super::install(&ctx);
        let mut out = ctx.run_ui(Default::default(), |_| {});
        out.textures_delta.clear();
        let font = egui::FontId::proportional(14.0);
        for c in super::ICONS.chars() {
            assert!(ctx.fonts_mut(|f| f.has_glyph(&font, c)), "no glyph for {c}");
        }
    }
}
