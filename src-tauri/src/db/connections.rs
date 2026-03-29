use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Driver {
    Postgres,
    Mysql,
    Sqlite,
    Mssql,
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
    pub color: String,
}

impl ConnectionConfig {
    pub fn new(name: String, driver: Driver) -> Self {
        let port = match driver {
            Driver::Postgres => 5432,
            Driver::Mysql => 3306,
            Driver::Sqlite => 0,
            Driver::Mssql => 1433,
        };
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            driver,
            host: String::new(),
            port,
            database: String::new(),
            user: String::new(),
            password: String::new(),
            file_path: String::new(),
            ssl_mode: String::new(),
            color: String::new(),
        }
    }

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

    /// Build a tiberius Config for MSSQL connections
    pub fn tiberius_config(&self) -> Result<tiberius::Config, String> {
        let mut config = tiberius::Config::new();
        config.host(&self.host);
        config.port(self.port);
        config.database(&self.database);
        config.authentication(tiberius::AuthMethod::sql_server(&self.user, &self.password));
        config.trust_cert();
        Ok(config)
    }
}
