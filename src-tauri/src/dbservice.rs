//! HTTP client for the Sqail DbService backend.
//!
//! DbService is an ASP.NET (FastEndpoints) service that proxies queries
//! against one or more saved connections. Authentication uses JWT:
//! the client exchanges a pre-shared API key at /api/auth/token for a
//! short-lived bearer token, which is attached to subsequent requests.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::pool::DbServiceClient;
use crate::query::{QueryColumn, QueryResponse, QueryResult};
use crate::schema::{ColumnInfo, SchemaInfo, TableInfo};

#[derive(Debug, Serialize)]
struct TokenReq<'a> {
    #[serde(rename = "apiKey")]
    api_key: &'a str,
}

#[derive(Debug, Deserialize)]
struct TokenRes {
    token: String,
}

#[derive(Debug, Serialize)]
struct QueryReq<'a> {
    #[serde(rename = "connectionId")]
    connection_id: &'a str,
    sql: &'a str,
}

/// DbService query response — matches Sqail.DbService.Models.QueryResult.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DbServiceQueryResult {
    success: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    columns: Vec<DbServiceColumn>,
    #[serde(default)]
    rows: Vec<serde_json::Map<String, Value>>,
    #[serde(default)]
    rows_affected: i64,
    #[serde(default)]
    execution_time_ms: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DbServiceColumn {
    name: String,
    data_type: String,
    #[serde(default)]
    is_nullable: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DbServiceTable {
    schema: String,
    name: String,
    #[serde(default)]
    columns: Vec<DbServiceTableColumn>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DbServiceTableColumn {
    name: String,
    data_type: String,
    #[serde(default)]
    is_nullable: bool,
    #[serde(default)]
    is_primary_key: bool,
}

fn trim_base(url: &str) -> &str {
    url.trim_end_matches('/')
}

/// Exchange api_key for a JWT. Caches on the client.
pub async fn ensure_token(client: &Arc<DbServiceClient>) -> Result<String, String> {
    {
        let guard = client.jwt.lock().await;
        if !guard.is_empty() {
            return Ok(guard.clone());
        }
    }
    let url = format!("{}/api/auth/token", trim_base(&client.base_url));
    let res = client
        .http
        .post(&url)
        .json(&TokenReq { api_key: &client.api_key })
        .send()
        .await
        .map_err(|e| format!("DbService token request failed: {e}"))?;

    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        return Err(format!("DbService auth failed ({status}): {body}"));
    }
    let body: TokenRes = res
        .json()
        .await
        .map_err(|e| format!("DbService token parse failed: {e}"))?;
    let mut guard = client.jwt.lock().await;
    *guard = body.token.clone();
    Ok(body.token)
}

/// POST /api/connections/test — pings the remote backend using the configured remote_id.
pub async fn test(client: &Arc<DbServiceClient>) -> Result<String, String> {
    let token = ensure_token(client).await?;
    let url = format!("{}/api/connections/test", trim_base(&client.base_url));
    let res = client
        .http
        .post(&url)
        .bearer_auth(&token)
        .json(&serde_json::json!({ "connectionId": client.remote_id }))
        .send()
        .await
        .map_err(|e| format!("DbService test failed: {e}"))?;
    if res.status().is_success() {
        Ok("DbService connection successful".to_string())
    } else {
        Err(format!(
            "DbService test failed ({}): {}",
            res.status(),
            res.text().await.unwrap_or_default()
        ))
    }
}

pub async fn execute_query(client: &Arc<DbServiceClient>, sql: &str) -> QueryResponse {
    let start = std::time::Instant::now();
    let token = match ensure_token(client).await {
        Ok(t) => t,
        Err(e) => {
            return QueryResponse {
                results: vec![],
                total_time_ms: start.elapsed().as_millis() as u64,
                error: Some(e),
            };
        }
    };
    let url = format!("{}/api/query/execute", trim_base(&client.base_url));
    let res = client
        .http
        .post(&url)
        .bearer_auth(&token)
        .json(&QueryReq {
            connection_id: &client.remote_id,
            sql,
        })
        .send()
        .await;

    let res = match res {
        Ok(r) => r,
        Err(e) => {
            return QueryResponse {
                results: vec![],
                total_time_ms: start.elapsed().as_millis() as u64,
                error: Some(format!("DbService request failed: {e}")),
            };
        }
    };

    let parsed: Result<DbServiceQueryResult, _> = res.json().await;
    match parsed {
        Ok(r) if r.success => {
            let columns: Vec<QueryColumn> = r
                .columns
                .into_iter()
                .map(|c| QueryColumn {
                    name: c.name,
                    type_name: c.data_type,
                })
                .collect();
            let col_names: Vec<String> = columns.iter().map(|c| c.name.clone()).collect();
            let rows: Vec<Vec<Value>> = r
                .rows
                .into_iter()
                .map(|row| {
                    col_names
                        .iter()
                        .map(|n| row.get(n).cloned().unwrap_or(Value::Null))
                        .collect()
                })
                .collect();
            let row_count = rows.len();
            let is_mutation = row_count == 0 && r.rows_affected > 0;
            QueryResponse {
                results: vec![QueryResult {
                    columns,
                    rows,
                    row_count,
                    affected_rows: if r.rows_affected >= 0 {
                        Some(r.rows_affected as u64)
                    } else {
                        None
                    },
                    execution_time_ms: r.execution_time_ms.max(0) as u64,
                    is_mutation,
                    statement_index: 0,
                }],
                total_time_ms: start.elapsed().as_millis() as u64,
                error: None,
            }
        }
        Ok(r) => QueryResponse {
            results: vec![],
            total_time_ms: start.elapsed().as_millis() as u64,
            error: r.error.or_else(|| Some("DbService query failed".to_string())),
        },
        Err(e) => QueryResponse {
            results: vec![],
            total_time_ms: start.elapsed().as_millis() as u64,
            error: Some(format!("DbService response parse failed: {e}")),
        },
    }
}

/// Fetch all tables via /api/metadata/{id}/tables and optionally filter by schema.
pub async fn list_tables(
    client: &Arc<DbServiceClient>,
    schema: &str,
) -> Result<Vec<TableInfo>, String> {
    let token = ensure_token(client).await?;
    let url = format!(
        "{}/api/metadata/{}/tables",
        trim_base(&client.base_url),
        urlencoding::encode_or_passthrough(&client.remote_id)
    );
    let res = client
        .http
        .get(&url)
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| format!("DbService list_tables failed: {e}"))?;
    if !res.status().is_success() {
        return Err(format!(
            "DbService list_tables ({}): {}",
            res.status(),
            res.text().await.unwrap_or_default()
        ));
    }
    let tables: Vec<DbServiceTable> = res
        .json()
        .await
        .map_err(|e| format!("DbService tables parse failed: {e}"))?;
    Ok(tables
        .into_iter()
        .filter(|t| schema.is_empty() || t.schema == schema)
        .map(|t| TableInfo {
            name: t.name,
            schema: t.schema,
            table_type: "table".to_string(),
        })
        .collect())
}

pub async fn list_schemas(client: &Arc<DbServiceClient>) -> Result<Vec<SchemaInfo>, String> {
    // Derive schemas from the tables endpoint.
    let tables = list_tables(client, "").await?;
    let mut seen: std::collections::BTreeSet<String> = Default::default();
    for t in &tables {
        seen.insert(t.schema.clone());
    }
    if seen.is_empty() {
        seen.insert("default".to_string());
    }
    Ok(seen.into_iter().map(|name| SchemaInfo { name }).collect())
}

pub async fn list_columns(
    client: &Arc<DbServiceClient>,
    schema: &str,
    table: &str,
) -> Result<Vec<ColumnInfo>, String> {
    let token = ensure_token(client).await?;
    let url = format!(
        "{}/api/metadata/{}/tables",
        trim_base(&client.base_url),
        urlencoding::encode_or_passthrough(&client.remote_id)
    );
    let res = client
        .http
        .get(&url)
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| format!("DbService list_columns failed: {e}"))?;
    if !res.status().is_success() {
        return Err(format!(
            "DbService list_columns ({}): {}",
            res.status(),
            res.text().await.unwrap_or_default()
        ));
    }
    let tables: Vec<DbServiceTable> = res
        .json()
        .await
        .map_err(|e| format!("DbService columns parse failed: {e}"))?;
    let target = tables
        .into_iter()
        .find(|t| t.schema == schema && t.name == table)
        .ok_or_else(|| format!("Table {schema}.{table} not found"))?;
    Ok(target
        .columns
        .into_iter()
        .enumerate()
        .map(|(i, c)| ColumnInfo {
            name: c.name,
            data_type: c.data_type,
            is_nullable: c.is_nullable,
            column_default: None,
            is_primary_key: c.is_primary_key,
            ordinal_position: (i + 1) as i32,
        })
        .collect())
}

/// Build a reqwest client (accept-invalid-certs option mirrors ai/client.rs).
pub fn build_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .expect("failed to build reqwest client")
}

// Minimal URL-component encoder so we don't pull in a new crate just for this.
mod urlencoding {
    pub fn encode_or_passthrough(s: &str) -> String {
        // Only encode characters that would break the URL path segment.
        let mut out = String::with_capacity(s.len());
        for b in s.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(b as char);
                }
                _ => out.push_str(&format!("%{:02X}", b)),
            }
        }
        out
    }
}
