use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Driver {
    Postgres,
    Mysql,
    Sqlite,
    Mssql,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MssqlAuthMethod {
    SqlServer,
    Windows,
    EntraId,
}

impl Default for MssqlAuthMethod {
    fn default() -> Self {
        MssqlAuthMethod::SqlServer
    }
}

impl std::fmt::Display for Driver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Driver::Postgres => write!(f, "postgres"),
            Driver::Mysql => write!(f, "mysql"),
            Driver::Sqlite => write!(f, "sqlite"),
            Driver::Mssql => write!(f, "mssql"),
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
    pub tenant_id: String,
    #[serde(default)]
    pub azure_client_id: String,
    #[serde(default)]
    pub color: String,
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

        if self.trust_server_certificate {
            config.trust_cert();
        }
        Ok(config)
    }
}
