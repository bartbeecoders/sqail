use serde::{Deserialize, Serialize};
use sqlx::mysql::MySqlConnectOptions;
use sqlx::postgres::{PgConnectOptions, PgSslMode};
use sqlx::sqlite::SqliteConnectOptions;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Driver {
    Postgres,
    Mysql,
    Sqlite,
    Mssql,
    Dbservice,
    Surrealdb,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MssqlAuthMethod {
    #[default]
    SqlServer,
    Windows,
    EntraId,
}

/// Connection encryption level for MSSQL (tiberius).
/// Maps to `tiberius::EncryptionLevel`. Defaults to `Required` to preserve
/// existing behavior; `Off` (no TLS at all) is the workaround for older
/// servers whose only TLS protocols/ciphers modern Windows SChannel refuses.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MssqlEncryption {
    /// Encrypt everything; fail if the server can't. (tiberius `Required`)
    #[default]
    Required,
    /// Encrypt only the login handshake. (tiberius `Off`)
    LoginOnly,
    /// No encryption at all — skip the TLS handshake. (tiberius `NotSupported`)
    Off,
}

impl std::fmt::Display for Driver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Driver::Postgres => write!(f, "postgres"),
            Driver::Mysql => write!(f, "mysql"),
            Driver::Sqlite => write!(f, "sqlite"),
            Driver::Mssql => write!(f, "mssql"),
            Driver::Dbservice => write!(f, "dbservice"),
            Driver::Surrealdb => write!(f, "surrealdb"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionConfig {
    pub id: String,
    pub name: String,
    pub driver: Driver,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub database: String,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub file_path: String,
    #[serde(default)]
    pub ssl_mode: String,
    // ── Postgres certificate-based SSL (paths to PEM files) ──
    /// CA certificate used to verify the server (libpq `sslrootcert`).
    #[serde(default)]
    pub ssl_root_cert: String,
    /// Client certificate for mutual TLS (libpq `sslcert`).
    #[serde(default)]
    pub ssl_client_cert: String,
    /// Client private key, unencrypted PEM (libpq `sslkey`): PKCS#8, PKCS#1
    /// or SEC1 — rustls accepts all three.
    #[serde(default)]
    pub ssl_client_key: String,
    #[serde(default)]
    pub integrated_security: bool,
    #[serde(default)]
    pub trust_server_certificate: bool,
    #[serde(default)]
    pub mssql_auth_method: MssqlAuthMethod,
    #[serde(default)]
    pub mssql_encryption: MssqlEncryption,
    #[serde(default)]
    pub tenant_id: String,
    #[serde(default)]
    pub azure_client_id: String,
    #[serde(default)]
    pub color: String,
    // ── DbService backend ──
    #[serde(default)]
    pub dbservice_url: String,
    #[serde(default)]
    pub dbservice_api_key: String,
    #[serde(default)]
    pub dbservice_remote_id: String,
    // ── SurrealDB backend ──
    #[serde(default)]
    pub surreal_namespace: String,
}

/// Certificate-based SSL settings for Postgres. All fields are optional
/// file paths to PEM-encoded material; empty strings mean "not set".
#[derive(Debug, Clone, Copy, Default)]
pub struct PgSslCerts<'a> {
    /// CA certificate that must have signed the server certificate.
    pub root_cert: &'a str,
    /// Client certificate presented to the server (mutual TLS).
    pub client_cert: &'a str,
    /// Private key matching `client_cert`.
    pub client_key: &'a str,
}

impl<'a> PgSslCerts<'a> {
    pub fn new(root_cert: &'a str, client_cert: &'a str, client_key: &'a str) -> Self {
        Self { root_cert, client_cert, client_key }
    }

    fn root(&self) -> Option<&'a str> {
        Some(self.root_cert.trim()).filter(|s| !s.is_empty())
    }

    /// sqlx (native-tls) only presents a client identity when *both* the
    /// certificate and the key are set, so a half-filled pair is treated as
    /// "no client auth" rather than an error.
    fn client_pair(&self) -> Option<(&'a str, &'a str)> {
        let cert = self.client_cert.trim();
        let key = self.client_key.trim();
        (!cert.is_empty() && !key.is_empty()).then_some((cert, key))
    }
}

/// Build Postgres connect options from raw field values.
///
/// Uses the sqlx builder instead of a formatted `postgres://…` URL so the
/// password (and every other field) needs no percent-encoding — a password
/// containing `@`, `:`, `/`, `?`, `#`, `%` or a space would otherwise make the
/// URL parser fail with a misleading error such as "invalid port number".
///
/// `certs` carries optional PEM file paths. When a certificate is supplied
/// but the mode is unset/unrecognized, the mode is raised (`verify-ca` for a
/// root cert, `require` for a client pair) so the certificates are actually
/// used instead of silently ignored under `prefer`.
pub fn build_pg_connect_options(
    host: &str,
    port: u16,
    user: &str,
    password: &str,
    database: &str,
    ssl_mode: &str,
    certs: PgSslCerts<'_>,
) -> PgConnectOptions {
    let root = certs.root();
    let client_pair = certs.client_pair();
    let mode = match ssl_mode.to_ascii_lowercase().as_str() {
        "disable" => PgSslMode::Disable,
        "allow" => PgSslMode::Allow,
        "prefer" => PgSslMode::Prefer,
        "require" => PgSslMode::Require,
        "verify-ca" => PgSslMode::VerifyCa,
        "verify-full" => PgSslMode::VerifyFull,
        // Empty or unrecognized (e.g. the SurrealDB "https" flag) → libpq default,
        // unless certificates were supplied, in which case TLS must be on.
        _ if root.is_some() => PgSslMode::VerifyCa,
        _ if client_pair.is_some() => PgSslMode::Require,
        _ => PgSslMode::Prefer,
    };
    let mut opts = PgConnectOptions::new()
        .host(host)
        .port(port)
        .username(user)
        .password(password)
        .ssl_mode(mode);
    if !database.is_empty() {
        opts = opts.database(database);
    }
    if let Some(root) = root {
        opts = opts.ssl_root_cert(root);
    }
    if let Some((cert, key)) = client_pair {
        opts = opts.ssl_client_cert(cert).ssl_client_key(key);
    }
    opts
}

/// Build MySQL connect options from raw field values. See
/// [`build_pg_connect_options`] for why this avoids hand-built URLs.
pub fn build_mysql_connect_options(
    host: &str,
    port: u16,
    user: &str,
    password: &str,
    database: &str,
) -> MySqlConnectOptions {
    let mut opts = MySqlConnectOptions::new()
        .host(host)
        .port(port)
        .username(user)
        .password(password);
    if !database.is_empty() {
        opts = opts.database(database);
    }
    opts
}

impl ConnectionConfig {
    /// sqlx connect options for a Postgres connection, built from typed
    /// fields. See [`build_pg_connect_options`].
    pub fn pg_connect_options(&self) -> PgConnectOptions {
        build_pg_connect_options(
            &self.host,
            self.port,
            &self.user,
            &self.password,
            &self.database,
            &self.ssl_mode,
            PgSslCerts::new(&self.ssl_root_cert, &self.ssl_client_cert, &self.ssl_client_key),
        )
    }

    /// sqlx connect options for a MySQL connection. See [`build_mysql_connect_options`].
    pub fn mysql_connect_options(&self) -> MySqlConnectOptions {
        build_mysql_connect_options(
            &self.host,
            self.port,
            &self.user,
            &self.password,
            &self.database,
        )
    }

    /// sqlx connect options for a SQLite connection. `.filename()` takes the
    /// path verbatim, so paths with spaces or reserved characters are safe.
    pub fn sqlite_connect_options(&self) -> SqliteConnectOptions {
        SqliteConnectOptions::new().filename(&self.file_path)
    }

    /// Build a tiberius Config for MSSQL connections.
    /// For Entra ID auth, pass the access token obtained from the device code flow.
    pub fn tiberius_config(&self, entra_token: Option<&str>) -> Result<tiberius::Config, String> {
        let mut config = tiberius::Config::new();
        config.host(&self.host);
        config.port(self.port);
        config.database(&self.database);

        match self.mssql_auth_method {
            MssqlAuthMethod::EntraId => {
                let token = entra_token
                    .ok_or_else(|| "Entra ID auth requires an access token".to_string())?;
                config.authentication(tiberius::AuthMethod::aad_token(token));
            }
            MssqlAuthMethod::Windows => {
                config.authentication(tiberius::AuthMethod::Integrated);
            }
            MssqlAuthMethod::SqlServer => {
                // Backward compat: also check legacy integrated_security flag
                if self.integrated_security {
                    config.authentication(tiberius::AuthMethod::Integrated);
                } else {
                    config.authentication(tiberius::AuthMethod::sql_server(&self.user, &self.password));
                }
            }
        }

        config.encryption(match self.mssql_encryption {
            MssqlEncryption::Required => tiberius::EncryptionLevel::Required,
            MssqlEncryption::LoginOnly => tiberius::EncryptionLevel::Off,
            MssqlEncryption::Off => tiberius::EncryptionLevel::NotSupported,
        });

        if self.trust_server_certificate {
            config.trust_cert();
        }
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(ssl_mode: &str, root: &str, cert: &str, key: &str) -> PgConnectOptions {
        build_pg_connect_options(
            "db.example.com",
            5432,
            "app",
            "s3cret",
            "appdb",
            ssl_mode,
            PgSslCerts::new(root, cert, key),
        )
    }

    #[test]
    fn unset_mode_without_certs_stays_prefer() {
        assert!(matches!(opts("", "", "", "").get_ssl_mode(), PgSslMode::Prefer));
    }

    #[test]
    fn explicit_mode_is_respected() {
        assert!(matches!(opts("disable", "", "", "").get_ssl_mode(), PgSslMode::Disable));
        assert!(matches!(opts("Verify-Full", "", "", "").get_ssl_mode(), PgSslMode::VerifyFull));
    }

    #[test]
    fn root_cert_raises_unset_mode_to_verify_ca() {
        assert!(matches!(opts("", "/ca.crt", "", "").get_ssl_mode(), PgSslMode::VerifyCa));
    }

    #[test]
    fn client_pair_raises_unset_mode_to_require() {
        assert!(matches!(opts("", "", "/c.crt", "/c.key").get_ssl_mode(), PgSslMode::Require));
    }

    #[test]
    fn half_client_pair_does_not_change_mode() {
        assert!(matches!(opts("", "", "/c.crt", "").get_ssl_mode(), PgSslMode::Prefer));
        assert!(matches!(opts("", "", "", "  ").get_ssl_mode(), PgSslMode::Prefer));
    }

    #[test]
    fn explicit_mode_wins_over_cert_elevation() {
        assert!(matches!(opts("require", "/ca.crt", "", "").get_ssl_mode(), PgSslMode::Require));
    }
}
