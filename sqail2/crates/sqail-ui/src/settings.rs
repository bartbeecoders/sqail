//! User settings, stored as TOML in the config directory. Tokens are not in
//! here; see `secrets.rs`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub services: Vec<ServiceProfile>,
    /// URL of the service to connect to on start.
    pub active_service: Option<String>,
    /// Start the local sqail-service when sqail2 starts (if it is not running).
    pub autostart_local: bool,
    pub theme: ThemePref,
    pub editor_font_size: f32,
    /// Row cap per result set requested from the service.
    pub max_rows: u64,
    /// Open completions while typing (Ctrl+Space always works).
    pub autocomplete: bool,
    pub format_uppercase: bool,
    pub format_indent: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceProfile {
    pub name: String,
    pub url: String,
    /// Pinned certificate fingerprint; `None` = verify with the OS trust store.
    pub fingerprint: Option<String>,
    /// Set up via "Use a local service" (sqail2 can start it).
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
#[serde(rename_all = "lowercase")]
pub enum ThemePref {
    #[default]
    System,
    Light,
    Dark,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            services: Vec::new(),
            active_service: None,
            autostart_local: true,
            theme: ThemePref::System,
            editor_font_size: 14.0,
            max_rows: 100_000,
            autocomplete: true,
            format_uppercase: true,
            format_indent: 2,
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

/// `$SQAIL2_CONFIG_DIR`, else the OS config dir (e.g. `~/.config/sqail2`).
pub fn config_dir() -> PathBuf {
    if let Some(d) = CONFIG_DIR.with(|c| c.borrow().clone()) {
        return d;
    }
    if let Some(d) = std::env::var_os("SQAIL2_CONFIG_DIR") {
        return PathBuf::from(d);
    }
    directories::ProjectDirs::from("dev", "bartbeecoders", "sqail2")
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".sqail2"))
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
