//! Service configuration: defaults → TOML file → `SQAIL_*` environment.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Environment variables that replace a key of the file, as `(variable, key)`.
const ENV_OVERRIDES: &[(&str, &str)] = &[
    ("SQAIL_BIND", "bind"),
    ("SQAIL_SQLITE_DIRS", "sqlite.allowed_dirs"),
    ("SQAIL_LOG_FORMAT", "log_format"),
    ("SQAIL_DOCS_UI", "docs_ui"),
    ("SQAIL_ADMIN_UI", "admin_ui"),
];

/// The service settings, as in `sqail-service.toml`.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Address to listen on. Defaults to loopback only.
    #[schema(value_type = String, example = "127.0.0.1:7443")]
    pub bind: SocketAddr,
    /// Where `service.db`, `master.key` and the dev certificate live.
    /// Not read from the TOML file (the file lives inside it).
    #[serde(skip)]
    pub data_dir: PathBuf,
    /// The TOML file these settings come from, and where the admin page
    /// saves them. `None` means `<data_dir>/sqail-service.toml`.
    #[serde(skip)]
    pub config_file: Option<PathBuf>,
    pub tls: TlsConfig,
    pub limits: Limits,
    pub sessions: SessionConfig,
    pub sqlite: SqliteConfig,
    pub audit: AuditConfig,
    /// `pretty` or `json`.
    pub log_format: String,
    /// Serve the interactive API docs at `/docs`.
    pub docs_ui: bool,
    /// Serve the admin page at `/admin/` and the `/v1/admin` API that
    /// changes these settings.
    pub admin_ui: bool,
    /// Create an admin token on a start with none active. Off when the
    /// caller provisions its own tokens (`serve --no-bootstrap-token`).
    #[serde(skip)]
    pub bootstrap_admin_token: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct TlsConfig {
    /// PEM certificate chain. When absent a self-signed dev certificate is
    /// generated in `<data_dir>/tls/`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub cert: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub key: Option<PathBuf>,
    /// Also accept TLS 1.2 (default: TLS 1.3 only).
    pub allow_tls12: bool,
    /// Mutual TLS: require client certificates signed by this CA (PEM).
    /// Bearer tokens are still required on top.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub client_ca: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    /// Default row cap per result set when the request does not set one.
    pub default_max_rows: u64,
    /// Hard row cap per result set.
    pub max_rows: u64,
    /// Default query timeout when the request does not set one.
    pub default_timeout_ms: u64,
    /// Hard query timeout.
    pub max_timeout_ms: u64,
    /// Maximum request body (SQL scripts included).
    pub body_limit_bytes: usize,
    /// Per-token sustained request rate.
    pub requests_per_second: u32,
    /// Per-token burst on top of the sustained rate.
    pub burst: u32,
    /// Pooled connections per connection profile.
    pub pool_size: usize,
    /// Timeout for non-streaming requests.
    pub request_timeout_secs: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SessionConfig {
    pub idle_timeout_secs: u64,
    pub max_per_token: usize,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SqliteConfig {
    /// SQLite profiles may only point at files inside these directories.
    /// Empty means SQLite is disabled.
    #[schema(value_type = Vec<String>)]
    pub allowed_dirs: Vec<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct AuditConfig {
    /// Record query text in the audit log (never row values).
    pub log_sql: bool,
    /// Truncate logged SQL to this many bytes.
    pub max_sql_len: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:7443".parse().expect("valid default addr"),
            data_dir: PathBuf::new(),
            config_file: None,
            tls: TlsConfig::default(),
            limits: Limits::default(),
            sessions: SessionConfig::default(),
            sqlite: SqliteConfig::default(),
            audit: AuditConfig::default(),
            log_format: "pretty".into(),
            docs_ui: cfg!(debug_assertions),
            admin_ui: true,
            bootstrap_admin_token: true,
        }
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            default_max_rows: 100_000,
            max_rows: 5_000_000,
            default_timeout_ms: 0,
            max_timeout_ms: 24 * 3600 * 1000,
            body_limit_bytes: 8 * 1024 * 1024,
            requests_per_second: 50,
            burst: 200,
            pool_size: 8,
            request_timeout_secs: 60,
        }
    }
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            idle_timeout_secs: 30 * 60,
            max_per_token: 32,
        }
    }
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            log_sql: true,
            max_sql_len: 4096,
        }
    }
}

impl Config {
    /// Resolve the data dir, read the TOML file (explicit path, or
    /// `<data_dir>/sqail-service.toml` when it exists) and apply env overrides.
    pub fn load(data_dir: Option<PathBuf>, config_file: Option<PathBuf>) -> Result<Self> {
        let data_dir =
            match data_dir.or_else(|| std::env::var_os("SQAIL_DATA_DIR").map(PathBuf::from)) {
                Some(d) => d,
                None => directories::ProjectDirs::from("dev", "bartbeecoders", "sqail-service")
                    .context("cannot determine a data directory; pass --data-dir")?
                    .data_dir()
                    .to_path_buf(),
            };
        std::fs::create_dir_all(&data_dir)
            .with_context(|| format!("creating data dir {}", data_dir.display()))?;

        let file = config_file.or_else(|| {
            let p = data_dir.join("sqail-service.toml");
            p.exists().then_some(p)
        });
        let mut cfg = match &file {
            Some(path) => Self::from_file(path)?,
            None => Self::default(),
        };
        cfg.data_dir = data_dir;
        cfg.config_file = file;
        cfg.apply_env()?;
        Ok(cfg)
    }

    /// `file` (settings as in the TOML file) with this config's data dir,
    /// file path and startup flags, plus the environment overrides: what the
    /// service runs with once `file` is saved.
    pub fn with_file_settings(&self, file: Config) -> Result<Self> {
        let mut cfg = Config {
            data_dir: self.data_dir.clone(),
            config_file: self.config_file.clone(),
            bootstrap_admin_token: self.bootstrap_admin_token,
            ..file
        };
        cfg.apply_env()?;
        Ok(cfg)
    }

    /// The settings as written in the config file (defaults when there is
    /// none), without environment overrides.
    pub fn file_settings(&self) -> Result<Self> {
        let path = self.config_path();
        if path.exists() {
            Self::from_file(&path)
        } else {
            Ok(Self::default())
        }
    }

    /// Where the settings are read from and saved to.
    pub fn config_path(&self) -> PathBuf {
        self.config_file
            .clone()
            .unwrap_or_else(|| self.data_dir.join("sqail-service.toml"))
    }

    /// Keys whose file value is currently replaced by an environment variable.
    pub fn env_overrides() -> Vec<&'static str> {
        ENV_OVERRIDES
            .iter()
            .filter(|(var, _)| std::env::var_os(var).is_some())
            .map(|(_, key)| *key)
            .collect()
    }

    /// Render as the content of the config file.
    pub fn to_toml(&self) -> Result<String> {
        Ok(format!(
            "# sqail-service settings, saved by the admin page (/admin/).\n\
             # Every key is described in sqail-service.example.toml.\n\n{}",
            toml::to_string(self).context("serializing settings")?
        ))
    }

    /// Reject values the service cannot run with. Paths are not checked
    /// here: loading the certificates and opening the folders does that.
    pub fn check(&self) -> Result<(), String> {
        let l = &self.limits;
        let problems = [
            (
                l.default_max_rows == 0,
                "limits.default_max_rows must be at least 1",
            ),
            (
                l.max_rows < l.default_max_rows,
                "limits.max_rows must be at least limits.default_max_rows",
            ),
            (
                l.default_timeout_ms > l.max_timeout_ms,
                "limits.default_timeout_ms must not exceed limits.max_timeout_ms",
            ),
            (
                l.body_limit_bytes < 4096,
                "limits.body_limit_bytes must be at least 4096",
            ),
            (
                l.requests_per_second == 0,
                "limits.requests_per_second must be at least 1",
            ),
            (l.burst == 0, "limits.burst must be at least 1"),
            (
                l.pool_size == 0 || l.pool_size > 256,
                "limits.pool_size must be 1-256",
            ),
            (
                l.request_timeout_secs == 0,
                "limits.request_timeout_secs must be at least 1",
            ),
            (
                self.sessions.idle_timeout_secs < 10,
                "sessions.idle_timeout_secs must be at least 10",
            ),
            (
                self.sessions.max_per_token == 0,
                "sessions.max_per_token must be at least 1",
            ),
            (
                self.tls.cert.is_some() != self.tls.key.is_some(),
                "tls.cert and tls.key must be set together",
            ),
            (
                !matches!(self.log_format.as_str(), "pretty" | "json"),
                "log_format must be pretty or json",
            ),
        ];
        match problems.iter().find(|(bad, _)| *bad) {
            Some((_, msg)) => Err((*msg).to_string()),
            None => Ok(()),
        }
    }

    fn from_file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing config {}", path.display()))
    }

    fn apply_env(&mut self) -> Result<()> {
        if let Ok(v) = std::env::var("SQAIL_BIND") {
            self.bind = v.parse().with_context(|| format!("SQAIL_BIND={v}"))?;
        }
        if let Some(v) = std::env::var_os("SQAIL_SQLITE_DIRS") {
            self.sqlite.allowed_dirs = std::env::split_paths(&v).collect();
        }
        if let Ok(v) = std::env::var("SQAIL_LOG_FORMAT") {
            self.log_format = v;
        }
        if let Ok(v) = std::env::var("SQAIL_DOCS_UI") {
            self.docs_ui = matches!(v.as_str(), "1" | "true" | "yes");
        }
        if let Ok(v) = std::env::var("SQAIL_ADMIN_UI") {
            self.admin_ui = matches!(v.as_str(), "1" | "true" | "yes");
        }
        Ok(())
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("service.db")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_overrides_defaults() {
        let cfg: Config = toml::from_str(
            r#"
            bind = "0.0.0.0:9000"
            [limits]
            max_rows = 10
            "#,
        )
        .unwrap();
        assert_eq!(cfg.bind.port(), 9000);
        assert_eq!(cfg.limits.max_rows, 10);
        assert_eq!(cfg.limits.default_max_rows, 100_000);
    }

    #[test]
    fn toml_round_trips() {
        let mut cfg = Config {
            bind: "0.0.0.0:9443".parse().unwrap(),
            ..Default::default()
        };
        cfg.tls.cert = Some("/etc/sqail/cert.pem".into());
        cfg.tls.key = Some("/etc/sqail/key.pem".into());
        cfg.sqlite.allowed_dirs = vec!["/srv/sqlite".into()];
        let back: Config = toml::from_str(&cfg.to_toml().unwrap()).unwrap();
        assert_eq!(back.bind, cfg.bind);
        assert_eq!(back.tls.cert, cfg.tls.cert);
        assert_eq!(back.sqlite.allowed_dirs, cfg.sqlite.allowed_dirs);
        assert!(back.tls.client_ca.is_none());
    }

    #[test]
    fn check_rejects_nonsense() {
        assert!(Config::default().check().is_ok());
        let mut cfg = Config::default();
        cfg.limits.max_rows = 1;
        assert!(cfg.check().unwrap_err().contains("max_rows"));
        let mut cfg = Config::default();
        cfg.tls.cert = Some("c.pem".into());
        assert!(cfg.check().unwrap_err().contains("together"));
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(toml::from_str::<Config>("bnid = \"x\"").is_err());
    }
}
