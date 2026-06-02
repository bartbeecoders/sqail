use serde::{Deserialize, Serialize};

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

impl ConnectionConfig {
    /// Connection string for sqlx (Postgres, MySQL, SQLite only)
    pub fn connection_string(&self) -> String {
        match self.driver {
            Driver::Postgres => {
                let ssl = if self.ssl_mode.is_empty() {
                    "prefer"
                } else {
                    &self.ssl_mode
                };
                format!(
                    "postgres://{}:{}@{}:{}/{}?sslmode={}",
                    self.user, self.password, self.host, self.port, self.database, ssl
                )
            }
            Driver::Mysql => {
                format!(
                    "mysql://{}:{}@{}:{}/{}",
                    self.user, self.password, self.host, self.port, self.database
                )
            }
            Driver::Sqlite => {
                format!("sqlite:{}", self.file_path)
            }
            Driver::Mssql => {
                // Not used — MSSQL uses tiberius config directly
                String::new()
            }
            Driver::Dbservice => String::new(),
            Driver::Surrealdb => String::new(),
        }
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
