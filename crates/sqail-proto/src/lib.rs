//! Wire types for the sqail-service REST API (`/v1`).
//!
//! Both the service and every client depend on this crate, so a change here is
//! an API change. Keep types plain data: no behaviour beyond small helpers, and
//! no driver-specific code.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Current REST API version prefix.
pub const API_VERSION: &str = "v1";

/// Media type of streamed query responses (one JSON [`QueryEvent`] per line).
pub const NDJSON: &str = "application/x-ndjson";

/// Request/response header carrying the client-chosen query id.
pub const QUERY_ID_HEADER: &str = "x-query-id";

// ---------------------------------------------------------------- service --

/// `GET /v1/health` — unauthenticated liveness probe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Health {
    pub status: String,
    pub version: String,
}

/// `GET /v1/info` — what this service supports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ServiceInfo {
    pub version: String,
    pub api_version: String,
    pub engines: Vec<Engine>,
    /// Scope of the token that made the request.
    pub scope: Scope,
}

/// Database engines sqail knows how to talk to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Postgres,
    Mssql,
    Sqlite,
    /// An engine newer than this build. Clients decode engine names they do
    /// not know into this, so an older client keeps working against a newer
    /// service; the service never sends it and rejects it on input.
    #[serde(other)]
    Unknown,
}

impl Engine {
    /// Every engine this build supports (without [`Engine::Unknown`]).
    pub const ALL: [Engine; 3] = [Engine::Postgres, Engine::Mssql, Engine::Sqlite];

    /// Whether this build knows the engine, i.e. it is not [`Engine::Unknown`].
    pub fn is_known(self) -> bool {
        self != Engine::Unknown
    }
}

/// Error body for every non-2xx response (`application/problem+json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Problem {
    pub title: String,
    pub status: u16,
    /// Stable machine-readable code, e.g. `unauthorized`, `not_found`.
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

// ----------------------------------------------------------------- tokens --

/// What an API token may do. Ordered: `Admin` > `Query` > `Read`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// Browse profiles and schema; run queries on read-only profiles only.
    Read,
    /// Run any query and open sessions.
    Query,
    /// Manage connection profiles, tokens and read the audit log.
    Admin,
}

impl Scope {
    pub fn allows(self, needed: Scope) -> bool {
        self >= needed
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Read => "read",
            Scope::Query => "query",
            Scope::Admin => "admin",
        }
    }
}

impl std::str::FromStr for Scope {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "read" => Ok(Scope::Read),
            "query" => Ok(Scope::Query),
            "admin" => Ok(Scope::Admin),
            other => Err(format!("unknown scope '{other}' (read, query, admin)")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TokenInfo {
    pub id: Uuid,
    pub name: String,
    pub scope: Scope,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateToken {
    pub name: String,
    pub scope: Scope,
}

/// Returned once on creation; the secret is never retrievable again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreatedToken {
    pub info: TokenInfo,
    pub token: String,
}

// ------------------------------------------------------------ connections --

/// Engine-specific connection parameters. Secrets are *not* part of this type;
/// see [`ConnectionInput::password`].
///
/// Deserializing never fails on an unknown `engine` tag: such parameters
/// become [`ConnectionParams::Other`], so an older client can still list the
/// connections of a newer service. Known tags with bad fields still fail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(tag = "engine", rename_all = "lowercase")]
pub enum ConnectionParams {
    Postgres(PostgresParams),
    Mssql(MssqlParams),
    Sqlite(SqliteParams),
    /// Parameters of an engine this build does not know, kept verbatim
    /// (including the `engine` tag). Never sent by the service; rejected on
    /// input. Left out of the OpenAPI schema.
    #[serde(untagged, skip_deserializing)]
    Other(serde_json::Value),
}

impl ConnectionParams {
    pub fn engine(&self) -> Engine {
        match self {
            ConnectionParams::Postgres(_) => Engine::Postgres,
            ConnectionParams::Mssql(_) => Engine::Mssql,
            ConnectionParams::Sqlite(_) => Engine::Sqlite,
            ConnectionParams::Other(_) => Engine::Unknown,
        }
    }

    /// The `engine` tag as sent, also for [`ConnectionParams::Other`].
    pub fn engine_name(&self) -> &str {
        match self {
            ConnectionParams::Postgres(_) => "postgres",
            ConnectionParams::Mssql(_) => "mssql",
            ConnectionParams::Sqlite(_) => "sqlite",
            ConnectionParams::Other(v) => v
                .get("engine")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown"),
        }
    }
}

impl<'de> Deserialize<'de> for ConnectionParams {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;

        /// The known variants, decoded by serde's own internally tagged
        /// implementation (and with its error messages).
        #[derive(Deserialize)]
        #[serde(tag = "engine", rename_all = "lowercase")]
        enum Known {
            Postgres(PostgresParams),
            Mssql(MssqlParams),
            Sqlite(SqliteParams),
        }

        let value = serde_json::Value::deserialize(deserializer)?;
        let engine = match value.get("engine") {
            Some(serde_json::Value::String(tag)) => {
                serde_json::from_value::<Engine>(serde_json::Value::String(tag.clone()))
                    .map_err(D::Error::custom)?
            }
            Some(_) => return Err(D::Error::custom("`engine` must be a string")),
            None if value.is_object() => return Err(D::Error::missing_field("engine")),
            None => return Err(D::Error::custom("connection params must be an object")),
        };
        if !engine.is_known() {
            return Ok(ConnectionParams::Other(value));
        }
        Ok(match Known::deserialize(value).map_err(D::Error::custom)? {
            Known::Postgres(p) => ConnectionParams::Postgres(p),
            Known::Mssql(p) => ConnectionParams::Mssql(p),
            Known::Sqlite(p) => ConnectionParams::Sqlite(p),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct PostgresParams {
    pub host: String,
    #[serde(default = "default_pg_port")]
    pub port: u16,
    pub database: String,
    pub user: String,
    #[serde(default)]
    pub ssl_mode: PgSslMode,
    /// PEM CA certificate, or a bundle, that signed the server certificate.
    /// Used only when `ssl_mode` is `verify-ca` or `verify-full`, in place of
    /// the operating system's trust store.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssl_root_cert: Option<String>,
    /// PEM client certificate (leaf first) for mutual TLS. The matching
    /// private key is [`ConnectionInput::ssl_client_key`], which is write-only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssl_client_cert: Option<String>,
}

fn default_pg_port() -> u16 {
    5432
}

/// Same meaning as libpq's `sslmode`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "kebab-case")]
pub enum PgSslMode {
    Disable,
    /// Try TLS without verifying the certificate, fall back to plain.
    #[default]
    Prefer,
    /// TLS required; certificate not verified.
    Require,
    /// TLS required; certificate chain verified, host name not checked.
    VerifyCa,
    /// TLS required; certificate chain and host name verified.
    VerifyFull,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct MssqlParams {
    pub host: String,
    #[serde(default = "default_mssql_port")]
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    pub auth: MssqlAuth,
    #[serde(default)]
    pub encrypt: MssqlEncrypt,
    #[serde(default)]
    pub trust_server_certificate: bool,
}

fn default_mssql_port() -> u16 {
    1433
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum MssqlAuth {
    /// SQL Server login; the password goes in [`ConnectionInput::password`].
    Sql { user: String },
    /// Windows integrated auth (only when the service runs on Windows).
    Integrated,
    /// Microsoft Entra ID user and password (Azure SQL); the password goes in
    /// [`ConnectionInput::password`]. Accounts that require MFA cannot use it.
    EntraPassword {
        /// `user@contoso.com`.
        user: String,
        /// Tenant ID or domain; `organizations` when omitted.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant: Option<String>,
        /// Public client app that signs in; Microsoft's SqlClient app when omitted.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_id: Option<String>,
    },
    /// Microsoft Entra ID service principal (app registration); the client
    /// secret goes in [`ConnectionInput::password`].
    EntraServicePrincipal {
        /// Tenant ID or domain.
        tenant: String,
        /// Application (client) ID.
        client_id: String,
    },
    /// The managed identity of the Azure host sqail-service runs on.
    EntraManagedIdentity {
        /// Client ID of a user-assigned identity; the system-assigned one when omitted.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_id: Option<String>,
    },
}

impl MssqlAuth {
    /// Whether this is a Microsoft Entra ID method, which can also list the
    /// databases in Azure subscriptions.
    pub fn is_entra(&self) -> bool {
        matches!(
            self,
            MssqlAuth::EntraPassword { .. }
                | MssqlAuth::EntraServicePrincipal { .. }
                | MssqlAuth::EntraManagedIdentity { .. }
        )
    }

    /// Whether this method takes a secret in [`ConnectionInput::password`].
    pub fn uses_password(&self) -> bool {
        !matches!(
            self,
            MssqlAuth::Integrated | MssqlAuth::EntraManagedIdentity { .. }
        )
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum MssqlEncrypt {
    /// Only the login packet is encrypted.
    Off,
    /// Encrypt if the server supports it.
    On,
    /// Always encrypt; fail otherwise.
    #[default]
    Required,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SqliteParams {
    /// Absolute path on the *service* host. Must be inside one of the
    /// service's allowed SQLite directories.
    pub path: String,
    /// Create the file if it does not exist.
    #[serde(default)]
    pub create: bool,
}

/// Body of `POST /v1/connections` and `PUT /v1/connections/{id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ConnectionInput {
    pub name: String,
    pub params: ConnectionParams,
    /// Write-only. On update: `None` keeps the stored password, `""` clears it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// Write-only PEM private key for [`PostgresParams::ssl_client_cert`].
    /// On update: `None` keeps the stored key, `""` clears it. Unencrypted
    /// PKCS#8, PKCS#1 or SEC1. PostgreSQL only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssl_client_key: Option<String>,
    #[serde(default)]
    pub read_only: bool,
    /// UI accent colour, e.g. `#c0392b` for production.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Free-form environment tag, e.g. `dev`, `test`, `prod`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
}

/// A stored connection profile as returned by the API (never includes secrets).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Connection {
    pub id: Uuid,
    pub name: String,
    pub engine: Engine,
    pub params: ConnectionParams,
    pub has_password: bool,
    /// A PostgreSQL client private key is stored. The key itself is never returned.
    #[serde(default)]
    pub has_ssl_client_key: bool,
    pub read_only: bool,
    pub color: Option<String>,
    pub environment: Option<String>,
    pub folder: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Result of a connection test.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TestResult {
    pub ok: bool,
    pub latency_ms: u64,
    pub server_version: Option<String>,
    pub error: Option<String>,
}

// ------------------------------------------------------- azure discovery --

/// `POST /v1/connections/{id}/azure/discover` — the databases in the Azure
/// subscriptions that a Microsoft Entra ID connection's identity can read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct AzureDiscovery {
    pub subscriptions: Vec<AzureSubscription>,
    pub databases: Vec<AzureDatabase>,
    /// Subscriptions or servers that could not be listed, and why.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct AzureSubscription {
    pub id: String,
    pub name: String,
}

/// The kind of Azure resource a discovered database lives on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum AzureServerKind {
    /// Azure SQL Database logical server.
    SqlServer,
    /// Azure SQL Managed Instance.
    SqlManagedInstance,
    /// Azure Database for PostgreSQL flexible server.
    PostgresFlexible,
}

impl AzureServerKind {
    pub fn engine(self) -> Engine {
        match self {
            AzureServerKind::SqlServer | AzureServerKind::SqlManagedInstance => Engine::Mssql,
            AzureServerKind::PostgresFlexible => Engine::Postgres,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AzureServerKind::SqlServer => "Azure SQL",
            AzureServerKind::SqlManagedInstance => "SQL Managed Instance",
            AzureServerKind::PostgresFlexible => "PostgreSQL",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct AzureDatabase {
    pub kind: AzureServerKind,
    pub subscription_id: String,
    pub resource_group: String,
    /// The server's resource name.
    pub server: String,
    /// Fully qualified host name to connect to.
    pub host: String,
    pub port: u16,
    pub database: String,
    pub location: String,
    /// The server's administrator login, when Azure reports one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admin_login: Option<String>,
}

// ---------------------------------------------------------------- queries --

/// A bind parameter. JSON `null`, booleans, numbers and strings map directly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(untagged)]
pub enum Param {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
}

/// Body of `POST /v1/connections/{id}/query` and `POST /v1/sessions/{id}/query`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct QueryRequest {
    /// One or more statements. SQL Server scripts may use `GO` separators.
    /// With `params`, exactly one statement is allowed; placeholders are the
    /// engine's native ones (`$1` Postgres, `@P1` SQL Server, `?1` SQLite).
    pub sql: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<Param>,
    /// Row cap *per result set*; the service enforces its own maximum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_rows: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Refuse the request unless `sql` is exactly one statement (no `GO`
    /// either). Checked by the engine where it can (Postgres parses it as a
    /// prepared statement, SQLite checks the prepared statement's tail);
    /// SQL Server is best effort: one batch, one `;`-separated statement and
    /// no statement keyword (transaction control, DDL, control flow) after
    /// the first word.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub single_statement: bool,
}

impl QueryRequest {
    pub fn new(sql: impl Into<String>) -> Self {
        Self {
            sql: sql.into(),
            params: Vec::new(),
            max_rows: None,
            timeout_ms: None,
            single_statement: false,
        }
    }
}

/// How a column's cells are encoded in JSON and should be displayed.
///
/// Cell encoding: `null` for SQL NULL; otherwise `Bool` → bool, `Int` → number
/// (i64 — beware JavaScript clients past 2^53), `Float` → number (NaN/±Inf as
/// strings), `Bytes` → lowercase hex string, everything else → string
/// (`Decimal` keeps full precision, temporals are ISO-8601, `Json` is JSON text).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum LogicalType {
    Bool,
    Int,
    Float,
    Decimal,
    Text,
    Bytes,
    Date,
    Time,
    Timestamp,
    TimestampTz,
    Uuid,
    Json,
    /// Anything else, including logical types newer than this build.
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Column {
    pub name: String,
    /// The engine's own type name, e.g. `int4`, `nvarchar`, `INTEGER`.
    pub type_name: String,
    pub logical: LogicalType,
}

/// One line of a streamed query response.
///
/// Order: `started`, then for every statement either
/// `result_start` → `rows`* → `result_end`, or `rows_affected`; `message`s may
/// appear anywhere; finally exactly one `done`. An `error` ends the script
/// early but is still followed by `done`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum QueryEvent {
    Started {
        query_id: Uuid,
    },
    ResultStart {
        index: u32,
        columns: Vec<Column>,
    },
    Rows {
        index: u32,
        #[cfg_attr(feature = "openapi", schema(value_type = Vec<Vec<Object>>))]
        rows: Vec<Vec<serde_json::Value>>,
    },
    ResultEnd {
        index: u32,
        row_count: u64,
        /// `max_rows` was hit; further rows were discarded.
        truncated: bool,
    },
    RowsAffected {
        count: u64,
    },
    Message {
        severity: String,
        text: String,
    },
    Error {
        code: String,
        message: String,
        /// SQLSTATE (Postgres) or error number (SQL Server), when known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        db_code: Option<String>,
    },
    Done {
        elapsed_ms: u64,
        cancelled: bool,
        /// Session queries only: whether a transaction is still open.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        in_transaction: Option<bool>,
    },
    /// An event newer than this build. Clients decode event names they do
    /// not know into this (and may skip it); the service never sends it.
    #[serde(other)]
    Unknown,
}

// --------------------------------------------------------------- sessions --

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateSession {
    pub connection_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SessionInfo {
    pub id: Uuid,
    pub connection_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub last_used_at: DateTime<Utc>,
    pub in_transaction: bool,
    pub busy: bool,
    /// Idle sessions are closed (and rolled back) after this many seconds.
    pub idle_timeout_secs: u64,
}

// ------------------------------------------------------------------- plans --

/// Body of `POST /v1/connections/{id}/explain` and `/v1/sessions/{id}/explain`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ExplainRequest {
    /// One statement.
    pub sql: String,
    /// Execute the statement to get actual row counts and timings. It runs
    /// inside a transaction that is always rolled back (Postgres, SQL
    /// Server); SQLite has no analyze mode and returns the estimated plan.
    #[serde(default)]
    pub analyze: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct PlanProp {
    pub key: String,
    pub value: String,
}

/// One operator in a query plan, engine-neutral.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct PlanNode {
    /// Operator, e.g. `Hash Join`, `Clustered Index Seek`, `SCAN orders`.
    pub label: String,
    /// Object the operator works on (table/index), when known.
    pub object: Option<String>,
    /// Estimated cost of this subtree (engine units).
    pub cost: Option<f64>,
    /// Estimated rows.
    pub rows: Option<f64>,
    /// Actual rows (analyze only).
    pub actual_rows: Option<f64>,
    /// Actual time of this subtree in ms (analyze only, Postgres).
    pub actual_ms: Option<f64>,
    pub props: Vec<PlanProp>,
    #[cfg_attr(feature = "openapi", schema(no_recursion))]
    pub children: Vec<PlanNode>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Plan {
    /// One root per statement.
    pub roots: Vec<PlanNode>,
    /// The engine's own output (JSON, XML or text).
    pub raw: String,
    pub analyzed: bool,
}

// ----------------------------------------------------------------- schema --

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct NamedItem {
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TableKind {
    Table,
    View,
    MaterializedView,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TableInfo {
    pub schema: Option<String>,
    pub name: String,
    pub kind: TableKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ColumnInfo {
    pub name: String,
    pub ordinal: i64,
    pub data_type: String,
    pub nullable: bool,
    pub default: Option<String>,
    pub primary_key: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct IndexInfo {
    pub name: String,
    /// Key columns (or expressions, on PostgreSQL) in key order.
    pub columns: Vec<String>,
    pub unique: bool,
    pub primary: bool,
    /// Backs a `PRIMARY KEY` or `UNIQUE` constraint, so it is dropped with
    /// `ALTER TABLE … DROP CONSTRAINT` rather than `DROP INDEX`.
    #[serde(default)]
    pub constraint: bool,
    /// Per key column, whether it is sorted descending. Empty when every key
    /// is ascending (or the service is older than this field).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub descending: Vec<bool>,
    /// Non-key columns stored in the index (`INCLUDE (…)`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,
    /// The predicate of a filtered (partial) index, as the engine prints it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    /// SQL Server: `CLUSTERED` (true) or `NONCLUSTERED` (false). None for
    /// other engines and for SQL Server's other index types (columnstore,
    /// XML, spatial).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clustered: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ForeignKeyInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub ref_schema: Option<String>,
    pub ref_table: String,
    pub ref_columns: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum RoutineKind {
    Function,
    Procedure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct RoutineInfo {
    pub schema: Option<String>,
    pub name: String,
    pub kind: RoutineKind,
}

/// One privilege on a table held by a role or user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TableGrant {
    /// Role, user or `PUBLIC` (Postgres) / `public` (SQL Server).
    pub grantee: String,
    /// `SELECT`, `INSERT`, `UPDATE`, `DELETE`, …
    pub privilege: String,
    /// Held `WITH GRANT OPTION`.
    pub grantable: bool,
}

/// `GET /v1/connections/{id}/schema/privileges` — who may do what on a table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TablePrivileges {
    /// Whether the engine has table privileges at all (SQLite does not).
    pub supported: bool,
    /// The table's owner, who holds every privilege implicitly (Postgres).
    pub owner: Option<String>,
    /// Explicit grants, owner excluded.
    pub grants: Vec<TableGrant>,
    /// Roles and users that could be granted privileges.
    pub principals: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Ddl {
    pub ddl: String,
}

// ------------------------------------------------------------------ audit --

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct AuditEntry {
    pub id: i64,
    pub at: DateTime<Utc>,
    /// Token name, or `cli` for local administrative commands.
    pub actor: String,
    pub action: String,
    pub target: Option<String>,
    pub detail: Option<String>,
    pub duration_ms: Option<i64>,
    pub success: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct AuditPage {
    pub items: Vec<AuditEntry>,
    /// Pass as `before` to fetch the next (older) page.
    pub next_before: Option<i64>,
}

/// A SQL Server batch separator line: `GO`, optionally followed by a repeat
/// count `n > 0` and a `--` comment, case-insensitive. Returns the count (1
/// for a bare `GO`). The caller makes sure the line does not start inside a
/// string or block comment. Shared by the service (which runs the batches)
/// and the editor (which picks the statement at the cursor), so both agree
/// on what a `GO` line is.
pub fn go_line(line: &str) -> Option<u32> {
    let line = line.trim();
    let line = match line.find("--") {
        Some(i) => line[..i].trim_end(),
        None => line,
    };
    // `get` rather than slicing: the line may start with a multi-byte char.
    let (Some(word), Some(rest)) = (line.get(..2), line.get(2..)) else {
        return None;
    };
    if !word.eq_ignore_ascii_case("go") {
        return None;
    }
    let rest = rest.trim();
    if rest.is_empty() {
        Some(1)
    } else {
        rest.parse().ok().filter(|&n| n > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_lines() {
        for (line, n) in [
            ("GO", Some(1)),
            (" go \r\n", Some(1)),
            ("GO 3 -- thrice", Some(3)),
            ("GO--x", Some(1)),
            ("GO 0", None),
            ("GO -1", None),
            ("GO x", None),
            ("GOTO label", None),
            ("é", None),
            ("", None),
        ] {
            assert_eq!(go_line(line), n, "{line:?}");
        }
    }

    #[test]
    fn engine_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&Engine::Mssql).unwrap(), "\"mssql\"");
    }

    #[test]
    fn entra_auth_json() {
        let sp: MssqlAuth = serde_json::from_str(
            r#"{"method":"entra_service_principal","tenant":"contoso.com","client_id":"app"}"#,
        )
        .unwrap();
        assert_eq!(
            sp,
            MssqlAuth::EntraServicePrincipal {
                tenant: "contoso.com".into(),
                client_id: "app".into()
            }
        );
        assert!(sp.uses_password());
        let mi: MssqlAuth = serde_json::from_str(r#"{"method":"entra_managed_identity"}"#).unwrap();
        assert_eq!(mi, MssqlAuth::EntraManagedIdentity { client_id: None });
        assert!(!mi.uses_password());
        let pw = MssqlAuth::EntraPassword {
            user: "ann@contoso.com".into(),
            tenant: None,
            client_id: None,
        };
        assert_eq!(
            serde_json::to_string(&pw).unwrap(),
            r#"{"method":"entra_password","user":"ann@contoso.com"}"#
        );
    }

    #[test]
    fn scope_ordering() {
        assert!(Scope::Admin.allows(Scope::Query));
        assert!(Scope::Query.allows(Scope::Read));
        assert!(!Scope::Read.allows(Scope::Query));
    }

    #[test]
    fn params_are_plain_json() {
        let p: Vec<Param> = serde_json::from_str(r#"[null, true, 42, 1.5, "x"]"#).unwrap();
        assert_eq!(
            p,
            vec![
                Param::Null,
                Param::Bool(true),
                Param::Int(42),
                Param::Float(1.5),
                Param::Text("x".into())
            ]
        );
    }

    #[test]
    fn postgres_certs_are_optional_and_verify_ca_is_kebab_case() {
        let p: PostgresParams =
            serde_json::from_str(r#"{"host":"db","database":"app","user":"app"}"#).unwrap();
        assert_eq!(p.ssl_mode, PgSslMode::Prefer);
        assert!(p.ssl_root_cert.is_none());
        assert!(p.ssl_client_cert.is_none());
        assert_eq!(
            serde_json::from_str::<PgSslMode>(r#""verify-ca""#).unwrap(),
            PgSslMode::VerifyCa
        );
        let back = PostgresParams {
            host: "db".into(),
            port: 5432,
            database: "app".into(),
            user: "app".into(),
            ssl_mode: PgSslMode::VerifyFull,
            ssl_root_cert: Some(
                "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n".into(),
            ),
            ssl_client_cert: None,
        };
        let json = serde_json::to_string(&back).unwrap();
        assert!(json.contains("ssl_root_cert"));
        assert!(!json.contains("ssl_client_cert"));
    }

    #[test]
    fn connection_params_tagged_by_engine() {
        let json = r#"{"engine":"mssql","host":"db","auth":{"method":"sql","user":"sa"}}"#;
        let p: ConnectionParams = serde_json::from_str(json).unwrap();
        let ConnectionParams::Mssql(m) = &p else {
            panic!("wrong variant")
        };
        assert_eq!(m.port, 1433);
        assert_eq!(m.encrypt, MssqlEncrypt::Required);
        assert_eq!(p.engine(), Engine::Mssql);
    }

    #[test]
    fn unknown_engine_round_trip() {
        let e: Engine = serde_json::from_str(r#""oracle""#).unwrap();
        assert_eq!(e, Engine::Unknown);
        assert!(!e.is_known());
        assert_eq!(serde_json::to_string(&e).unwrap(), r#""unknown""#);
        let back: Engine = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(back, Engine::Unknown);
        assert!(Engine::ALL.iter().all(|e| e.is_known()));
        // A newer service's info still parses.
        let info: ServiceInfo = serde_json::from_str(
            r#"{"version":"9","api_version":"v1","engines":["postgres","oracle"],"scope":"read"}"#,
        )
        .unwrap();
        assert_eq!(info.engines, vec![Engine::Postgres, Engine::Unknown]);
    }

    #[test]
    fn unknown_connection_params_are_kept_verbatim() {
        let json = r#"{"engine":"oracle","host":"db","service_name":"XE","port":1521}"#;
        let p: ConnectionParams = serde_json::from_str(json).unwrap();
        let ConnectionParams::Other(raw) = &p else {
            panic!("wrong variant: {p:?}")
        };
        assert_eq!(raw["service_name"], "XE");
        assert_eq!(p.engine(), Engine::Unknown);
        assert_eq!(p.engine_name(), "oracle");
        // Serializes back to exactly what came in.
        let again = serde_json::to_value(&p).unwrap();
        assert_eq!(
            again,
            serde_json::from_str::<serde_json::Value>(json).unwrap()
        );
        assert_eq!(
            serde_json::from_value::<ConnectionParams>(again).unwrap(),
            p
        );
    }

    #[test]
    fn connection_list_survives_an_unknown_engine() {
        let at = "2026-01-01T00:00:00Z";
        let json = format!(
            r#"[
              {{"id":"00000000-0000-0000-0000-000000000001","name":"new","engine":"oracle",
                "params":{{"engine":"oracle","host":"db"}},"has_password":true,"read_only":false,
                "color":null,"environment":null,"folder":null,"created_at":"{at}","updated_at":"{at}"}},
              {{"id":"00000000-0000-0000-0000-000000000002","name":"old","engine":"sqlite",
                "params":{{"engine":"sqlite","path":"/tmp/a.db"}},"has_password":false,"read_only":false,
                "color":null,"environment":null,"folder":null,"created_at":"{at}","updated_at":"{at}"}}
            ]"#
        );
        let list: Vec<Connection> = serde_json::from_str(&json).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].engine, Engine::Unknown);
        assert!(matches!(list[0].params, ConnectionParams::Other(_)));
        assert_eq!(list[1].engine, Engine::Sqlite);
        assert!(matches!(list[1].params, ConnectionParams::Sqlite(_)));
    }

    #[test]
    fn known_connection_params_still_report_bad_fields() {
        let err = serde_json::from_str::<ConnectionParams>(r#"{"engine":"postgres","host":"db"}"#)
            .unwrap_err()
            .to_string();
        assert!(err.contains("missing field `database`"), "{err}");
        let err = serde_json::from_str::<ConnectionParams>(r#"{"host":"db"}"#)
            .unwrap_err()
            .to_string();
        assert!(err.contains("missing field `engine`"), "{err}");
        assert!(serde_json::from_str::<ConnectionParams>(r#"{"engine":1}"#).is_err());
        assert!(serde_json::from_str::<ConnectionParams>(r#""postgres""#).is_err());
    }

    #[test]
    fn unknown_logical_type_is_other() {
        let c: Column =
            serde_json::from_str(r#"{"name":"g","type_name":"geometry","logical":"geometry"}"#)
                .unwrap();
        assert_eq!(c.logical, LogicalType::Other);
    }

    #[test]
    fn unknown_query_event_round_trip() {
        let ev: QueryEvent =
            serde_json::from_str(r#"{"event":"progress","percent":50,"note":"x"}"#).unwrap();
        assert_eq!(ev, QueryEvent::Unknown);
        let line = serde_json::to_string(&ev).unwrap();
        assert_eq!(line, r#"{"event":"unknown"}"#);
        assert_eq!(
            serde_json::from_str::<QueryEvent>(&line).unwrap(),
            QueryEvent::Unknown
        );
        // Known events with bad fields still fail.
        assert!(serde_json::from_str::<QueryEvent>(r#"{"event":"rows","index":0}"#).is_err());
    }

    #[test]
    fn query_event_round_trip() {
        let ev = QueryEvent::Rows {
            index: 0,
            rows: vec![vec![serde_json::json!(1), serde_json::Value::Null]],
        };
        let line = serde_json::to_string(&ev).unwrap();
        assert!(line.starts_with(r#"{"event":"rows""#));
        assert_eq!(serde_json::from_str::<QueryEvent>(&line).unwrap(), ev);
    }
}
