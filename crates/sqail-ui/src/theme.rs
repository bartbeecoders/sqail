//! Fonts and visuals. Both light and dark are tuned; which one shows follows
//! the OS unless the user picks one.

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Stroke, Theme};

pub const ACCENT_DARK: Color32 = Color32::from_rgb(0x6c, 0xc2, 0xa6);
pub const ACCENT_LIGHT: Color32 = Color32::from_rgb(0x2f, 0x6f, 0x5e);

pub fn engine_label(e: sqail_client::proto::Engine) -> &'static str {
    use sqail_client::proto::Engine;
    match e {
        Engine::Postgres => "PostgreSQL",
        Engine::Mssql => "SQL Server",
        Engine::Sqlite => "SQLite",
    }
}

pub fn accent(dark: bool) -> Color32 {
    if dark { ACCENT_DARK } else { ACCENT_LIGHT }
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
            let dark = theme == Theme::Dark;
            let accent = accent(dark);
            style.spacing.item_spacing = egui::vec2(8.0, 6.0);
            style.spacing.button_padding = egui::vec2(10.0, 4.0);
            style.spacing.interact_size.y = 26.0;
            let v = &mut style.visuals;
            v.selection.bg_fill = accent.linear_multiply(if dark { 0.45 } else { 0.30 });
            v.selection.stroke = Stroke::new(1.0, accent);
            v.hyperlink_color = accent;
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
            if dark {
                v.panel_fill = Color32::from_rgb(0x1b, 0x1b, 0x1a);
                v.window_fill = Color32::from_rgb(0x22, 0x22, 0x20);
                v.extreme_bg_color = Color32::from_rgb(0x14, 0x14, 0x13);
                v.faint_bg_color = Color32::from_rgb(0x24, 0x24, 0x22);
            } else {
                v.panel_fill = Color32::from_rgb(0xf5, 0xf4, 0xf1);
                v.window_fill = Color32::from_rgb(0xfb, 0xfb, 0xf9);
                v.extreme_bg_color = Color32::WHITE;
                v.faint_bg_color = Color32::from_rgb(0xee, 0xec, 0xe7);
            }
        });
    }
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
    if dark {
        SyntaxColors {
            text: Color32::from_rgb(0xdc, 0xda, 0xd4),
            keyword: Color32::from_rgb(0x82, 0xaa, 0xff),
            string: Color32::from_rgb(0xc3, 0xe8, 0x8d),
            number: Color32::from_rgb(0xf7, 0x8c, 0x6c),
            comment: Color32::from_rgb(0x7a, 0x78, 0x72),
            ident: Color32::from_rgb(0xe6, 0xc0, 0x7b),
            func: Color32::from_rgb(0x89, 0xdd, 0xff),
            bracket_bg: Color32::from_rgba_unmultiplied(0x6c, 0xc2, 0xa6, 70),
        }
    } else {
        SyntaxColors {
            text: Color32::from_rgb(0x24, 0x23, 0x21),
            keyword: Color32::from_rgb(0x1f, 0x4f, 0xb8),
            string: Color32::from_rgb(0x2e, 0x7d, 0x32),
            number: Color32::from_rgb(0xb0, 0x4a, 0x20),
            comment: Color32::from_rgb(0x8a, 0x87, 0x80),
            ident: Color32::from_rgb(0x8a, 0x5a, 0x00),
            func: Color32::from_rgb(0x00, 0x6b, 0x8f),
            bracket_bg: Color32::from_rgba_unmultiplied(0x2f, 0x6f, 0x5e, 60),
        }
    }
}

/// Every icon glyph the UI uses; a test checks the bundled fonts have them.
pub const ICONS: &str = "⊞👁ƒ⚙⟳▶■●×…▲▼";

#[cfg(test)]
mod tests {
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
