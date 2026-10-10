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
    /// Row cap per result set requested from the service.
    pub max_rows: u64,
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
            max_rows: 100_000,
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

    pub fn load() -> Self {
        match std::fs::read_to_string(Self::path()) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "settings.toml is invalid; using defaults");
                Self::default()
            }),
            // First start: on Omarchy, look like the rest of the desktop.
            Err(_) if crate::theme::omarchy_available() => Self {
                theme: ThemePref::Omarchy,
                ..Self::default()
            },
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) {
        let res = (|| -> anyhow::Result<()> {
            std::fs::create_dir_all(config_dir())?;
            std::fs::write(Self::path(), toml::to_string_pretty(self)?)?;
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
}
