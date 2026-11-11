//! User settings, stored as TOML in the config directory. Tokens are not in
//! here; see `secrets.rs`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub services: Vec<ServiceProfile>,
    /// URL of the service to connect to on start.
    pub active_service: Option<String>,
    /// Start the local sqail-service when sqail starts (if it is not running).
    pub autostart_local: bool,
    pub theme: ThemePref,
    pub editor_font_size: f32,
    /// Size of the whole interface (egui zoom factor), 1.0 = 100 %.
    pub ui_scale: f32,
    /// Ask before closing a tab with unsaved changes. Tabs with an open
    /// transaction always ask.
    pub confirm_close_tab: bool,
    /// Show pinned tabs on a row of their own, above the other tabs.
    pub pinned_tabs_row: bool,
    /// Row cap per result set requested from the service.
    pub max_rows: u64,
    /// Result grid: wrap values and show their line breaks, rows growing to
    /// fit (up to 10 lines).
    pub multiline_rows: bool,
    /// Open completions while typing (Ctrl+Space always works).
    pub autocomplete: bool,
    pub format_uppercase: bool,
    pub format_indent: u8,
    pub assistant: crate::assistant::AssistantSettings,
    /// The object browser on the left is expanded (not collapsed to a rail).
    pub sidebar_open: bool,
    /// Connection folders that have no connections yet, per service URL.
    /// Folders with connections are stored on the service.
    pub folders: std::collections::BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceProfile {
    pub name: String,
    pub url: String,
    /// Pinned certificate fingerprint; `None` = verify with the OS trust store.
    pub fingerprint: Option<String>,
    /// Set up via "Use a local service" (sqail can start it).
    #[serde(default)]
    pub local: bool,
    /// Mutual TLS: client certificate and key (PEM files), when the service
    /// requires one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_cert: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_key: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ThemePref {
    /// sqail's light or dark, following the operating system.
    #[default]
    System,
    Light,
    Dark,
    /// The colours of the current Omarchy theme, followed live.
    Omarchy,
    Nord,
    TokyoNight,
    Gruvbox,
    CatppuccinMocha,
    CatppuccinLatte,
    SolarizedLight,
}

impl ThemePref {
    pub const ALL: [ThemePref; 10] = [
        ThemePref::System,
        ThemePref::Light,
        ThemePref::Dark,
        ThemePref::Omarchy,
        ThemePref::Nord,
        ThemePref::TokyoNight,
        ThemePref::Gruvbox,
        ThemePref::CatppuccinMocha,
        ThemePref::CatppuccinLatte,
        ThemePref::SolarizedLight,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ThemePref::System => "Follow system (sqail light / dark)",
            ThemePref::Light => "sqail light",
            ThemePref::Dark => "sqail dark",
            ThemePref::Omarchy => "Omarchy (current theme)",
            ThemePref::Nord => "Nord",
            ThemePref::TokyoNight => "Tokyo Night",
            ThemePref::Gruvbox => "Gruvbox",
            ThemePref::CatppuccinMocha => "Catppuccin Mocha",
            ThemePref::CatppuccinLatte => "Catppuccin Latte",
            ThemePref::SolarizedLight => "Solarized Light",
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            services: Vec::new(),
            active_service: None,
            autostart_local: true,
            theme: ThemePref::System,
            editor_font_size: 14.0,
            ui_scale: 1.0,
            confirm_close_tab: true,
            pinned_tabs_row: true,
            max_rows: 100_000,
            multiline_rows: false,
            autocomplete: true,
            format_uppercase: true,
            format_indent: 2,
            assistant: Default::default(),
            sidebar_open: true,
            folders: Default::default(),
        }
    }
}

thread_local! {
    static CONFIG_DIR: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Use `dir` for settings, tokens, history and workspace on this thread (the
/// UI thread; tests run one app per thread).
pub fn override_config_dir(dir: PathBuf) {
    CONFIG_DIR.with(|c| *c.borrow_mut() = Some(dir));
}

/// `$SQAIL_CONFIG_DIR`, else the OS config dir (e.g. `~/.config/sqail`).
pub fn config_dir() -> PathBuf {
    if let Some(d) = CONFIG_DIR.with(|c| c.borrow().clone()) {
        return d;
    }
    if let Some(d) = std::env::var_os("SQAIL_CONFIG_DIR") {
        return PathBuf::from(d);
    }
    directories::ProjectDirs::from("dev", "bartbeecoders", "sqail")
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".sqail"))
}

/// Up to 1.0 the app was called sqail2. Move its config dir (settings,
/// tokens.toml, history, workspace) and eframe's window state to the new
/// names, once, if the new ones do not exist yet.
pub fn migrate_from_sqail2() {
    let config = if std::env::var_os("SQAIL_CONFIG_DIR").is_some() {
        None
    } else {
        directories::ProjectDirs::from("dev", "bartbeecoders", "sqail2")
            .zip(directories::ProjectDirs::from(
                "dev",
                "bartbeecoders",
                "sqail",
            ))
            .map(|(old, new)| {
                (
                    old.config_dir().to_path_buf(),
                    new.config_dir().to_path_buf(),
                )
            })
    };
    let window = eframe::storage_dir("sqail2").zip(eframe::storage_dir("sqail"));
    for (old, new) in config.into_iter().chain(window) {
        if new.exists() || !old.is_dir() {
            continue;
        }
        if let Some(parent) = new.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::rename(&old, &new) {
            Ok(()) => {
                tracing::info!(from = %old.display(), to = %new.display(), "moved sqail2 data")
            }
            Err(e) => {
                tracing::warn!(error = %e, from = %old.display(), "could not move sqail2 data")
            }
        }
    }
}

impl Settings {
    fn path() -> PathBuf {
        config_dir().join("settings.toml")
    }

    /// The settings, and a message when `settings.toml` was damaged: it is
    /// then kept as `settings.toml.bad` and the defaults are used, so the
    /// next save does not destroy the user's services and pinned
    /// fingerprints.
    pub fn load() -> (Self, Option<String>) {
        let loaded = crate::local::load_file(&Self::path(), |t| {
            toml::from_str::<Self>(t).map_err(|e| match e.span() {
                Some(s) => format!(
                    "line {}: {}",
                    t.as_bytes()[..s.start.min(t.len())]
                        .iter()
                        .filter(|&&b| b == b'\n')
                        .count()
                        + 1,
                    e.message()
                ),
                None => e.message().to_string(),
            })
        });
        match loaded {
            Ok(Some(mut s)) => {
                s.clamp();
                (s, None)
            }
            // First start: on Omarchy, look like the rest of the desktop.
            Ok(None) if crate::theme::omarchy_available() => (
                Self {
                    theme: ThemePref::Omarchy,
                    ..Self::default()
                },
                None,
            ),
            Ok(None) => (Self::default(), None),
            Err(e) => (Self::default(), Some(e)),
        }
    }

    /// Bring hand-edited values into the ranges the settings page offers;
    /// e.g. `ui_scale = 0.1` would make the window unusable.
    pub fn clamp(&mut self) {
        let finite = |v: f32, default: f32| if v.is_finite() { v } else { default };
        self.ui_scale = finite(self.ui_scale, 1.0).clamp(0.75, 2.0);
        self.editor_font_size = finite(self.editor_font_size, 14.0).clamp(9.0, 28.0);
        // No upper bound: the service caps it (`limits.max_rows`), and a
        // hand-set higher cap is deliberate.
        self.max_rows = self.max_rows.max(1);
        self.format_indent = self.format_indent.clamp(2, 4);
        self.assistant.max_rows = self.assistant.max_rows.clamp(1, 10_000);
    }

    pub fn save(&self) {
        let res = (|| -> anyhow::Result<()> {
            crate::local::write_atomic(&Self::path(), toml::to_string_pretty(self)?.as_bytes())?;
            Ok(())
        })();
        if let Err(e) = res {
            tracing::error!(error = %e, "could not save settings");
        }
    }

    pub fn active(&self) -> Option<&ServiceProfile> {
        let url = self.active_service.as_ref()?;
        self.services.iter().find(|s| &s.url == url)
    }

    pub fn upsert_service(&mut self, profile: ServiceProfile) {
        self.active_service = Some(profile.url.clone());
        match self.services.iter_mut().find(|s| s.url == profile.url) {
            Some(existing) => *existing = profile,
            None => self.services.push(profile),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn themes_keep_their_old_names_and_add_kebab_case_ones() {
        #[derive(Deserialize)]
        struct T {
            theme: ThemePref,
        }
        let t = |s: &str| {
            toml::from_str::<T>(&format!("theme = \"{s}\""))
                .unwrap()
                .theme
        };
        assert_eq!(t("dark"), ThemePref::Dark);
        assert_eq!(t("system"), ThemePref::System);
        assert_eq!(t("tokyo-night"), ThemePref::TokyoNight);
        assert_eq!(t("omarchy"), ThemePref::Omarchy);
        // Settings written before these existed still load.
        let old: Settings = toml::from_str("theme = \"light\"\nmax_rows = 10").unwrap();
        assert_eq!(old.theme, ThemePref::Light);
        assert!(old.confirm_close_tab);
        assert_eq!(old.ui_scale, 1.0);
    }

    #[test]
    fn hand_edited_values_are_clamped_to_what_the_ui_offers() {
        let mut s: Settings = toml::from_str(
            "ui_scale = 0.1\neditor_font_size = 300.0\nmax_rows = 0\nformat_indent = 200\n\
             [assistant]\nmax_rows = 0\n",
        )
        .unwrap();
        s.clamp();
        assert_eq!(s.ui_scale, 0.75);
        assert_eq!(s.editor_font_size, 28.0);
        assert_eq!(s.max_rows, 1);
        assert_eq!(s.format_indent, 4);
        assert_eq!(s.assistant.max_rows, 1);

        let mut s = Settings {
            ui_scale: f32::NAN,
            editor_font_size: f32::INFINITY,
            max_rows: 2_000_000,
            ..Default::default()
        };
        s.clamp();
        assert_eq!(s.ui_scale, 1.0);
        assert_eq!(s.editor_font_size, 14.0);
        assert_eq!(s.max_rows, 2_000_000, "the service caps rows, not the UI");
        // Valid values are left alone.
        let mut d = Settings::default();
        d.clamp();
        assert_eq!(d, Settings::default());
    }

    #[test]
    fn a_damaged_settings_file_is_kept_and_reported() {
        let dir = tempfile::tempdir().unwrap();
        override_config_dir(dir.path().to_path_buf());
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "services = [\nui_scale = 1.5\n").unwrap();
        let (s, err) = Settings::load();
        let err = err.expect("reported");
        assert!(err.contains("settings.toml could not be read"), "{err}");
        assert!(err.contains("line "), "{err}");
        assert!(s.services.is_empty());
        // The next save does not destroy the original.
        s.save();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("settings.toml.bad")).unwrap(),
            "services = [\nui_scale = 1.5\n"
        );
        let (_, err) = Settings::load();
        assert_eq!(err, None, "the new file is valid");
        // A valid file with an unusable value loads, clamped.
        std::fs::write(&path, "ui_scale = 0.1\n").unwrap();
        let (s, err) = Settings::load();
        assert_eq!((s.ui_scale, err), (0.75, None));
    }
}
