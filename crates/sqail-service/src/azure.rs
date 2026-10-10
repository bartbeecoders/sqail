//! Database discovery in Azure subscriptions through Azure Resource Manager.
//!
//! With an ARM token from a connection's Microsoft Entra ID identity, list the
//! subscriptions it can read, then the Azure SQL servers, SQL managed
//! instances and PostgreSQL flexible servers in each, then their databases.
//! A subscription or server that fails (no permission, provider not
//! registered) becomes a warning; only failing to list subscriptions fails the
//! whole discovery.

use std::time::Duration;

use futures::stream::{self, StreamExt};
use serde_json::Value;
use sqail_proto::{AzureDatabase, AzureDiscovery, AzureServerKind, AzureSubscription};

use crate::engine::{DbError, Result};

pub const ARM: &str = "https://management.azure.com";
const SUBSCRIPTIONS_API: &str = "2022-12-01";
const SQL_API: &str = "2021-11-01";
const POSTGRES_API: &str = "2022-12-01";
/// Following `nextLink` stops here; nobody pages through 100 pages of servers.
const MAX_PAGES: usize = 100;
/// Concurrent ARM requests.
const PARALLEL: usize = 8;

/// System databases that are not worth a connection profile.
const SKIP_SQL: &[&str] = &["master"];
const SKIP_POSTGRES: &[&str] = &["azure_maintenance", "azure_sys"];

pub struct Arm {
    http: reqwest::Client,
    base: String,
    token: String,
}

/// A server found in a subscription, before its databases are listed.
struct Server {
    kind: AzureServerKind,
    subscription_id: String,
    /// ARM resource ID; its databases are at `{id}/databases`.
    id: String,
    name: String,
    host: String,
    location: String,
    admin_login: Option<String>,
}

impl Arm {
    pub fn new(base: &str, token: String) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|e| DbError::Internal(e.to_string()))?;
        Ok(Self {
            http,
            base: base.trim_end_matches('/').to_string(),
            token,
        })
    }

    pub async fn discover(&self) -> Result<AzureDiscovery> {
        let subs = self
            .list(&format!("/subscriptions?api-version={SUBSCRIPTIONS_API}"))
            .await
            .map_err(|e| DbError::Connect(format!("listing Azure subscriptions failed: {e}")))?;
        let subscriptions: Vec<AzureSubscription> = subs
            .iter()
            .filter(|s| s["state"].as_str().is_none_or(|st| st == "Enabled"))
            .filter_map(|s| {
                let id = s["subscriptionId"].as_str()?.to_string();
                let name = s["displayName"].as_str().unwrap_or(&id).to_string();
                Some(AzureSubscription { id, name })
            })
            .collect();
        let mut warnings = Vec::new();
        if subscriptions.is_empty() {
            warnings.push("this identity cannot read any Azure subscription".to_string());
        }

        // Every (subscription, server kind) pair, then every server. The
        // streams own their items: borrowed ones trip up `Send` inference
        // in the axum handler.
        let lists: Vec<(AzureSubscription, AzureServerKind)> = subscriptions
            .iter()
            .flat_map(|s| {
                [
                    AzureServerKind::SqlServer,
                    AzureServerKind::SqlManagedInstance,
                    AzureServerKind::PostgresFlexible,
                ]
                .map(|kind| (s.clone(), kind))
            })
            .collect();
        let found: Vec<_> = stream::iter(lists)
            .map(|(sub, kind)| async move {
                let r = self.servers(&sub.id, kind).await;
                (sub, kind, r)
            })
            .buffered(PARALLEL)
            .collect()
            .await;
        let mut servers = Vec::new();
        for (sub, kind, r) in found {
            match r {
                Ok(s) => servers.extend(s),
                // An unregistered provider means there are no such servers.
                Err(e) if e.contains("NotRegistered") => {}
                Err(e) => warnings.push(format!("{} ({}): {e}", sub.name, kind.label())),
            }
        }

        let dbs: Vec<_> = stream::iter(servers)
            .map(|s| async move {
                let r = self.databases(&s).await;
                (s, r)
            })
            .buffered(PARALLEL)
            .collect()
            .await;
        let mut databases = Vec::new();
        for (server, r) in dbs {
            match r {
                Ok(d) => databases.extend(d),
                Err(e) => warnings.push(format!("{}: {e}", server.name)),
            }
        }
        databases.sort_by(|a, b| {
            (&a.subscription_id, &a.server, &a.database).cmp(&(
                &b.subscription_id,
                &b.server,
                &b.database,
            ))
        });
        Ok(AzureDiscovery {
            subscriptions,
            databases,
            warnings,
        })
    }

    async fn servers(
        &self,
        subscription: &str,
        kind: AzureServerKind,
    ) -> std::result::Result<Vec<Server>, String> {
        let (provider, api) = match kind {
            AzureServerKind::SqlServer => ("Microsoft.Sql/servers", SQL_API),
            AzureServerKind::SqlManagedInstance => ("Microsoft.Sql/managedInstances", SQL_API),
            AzureServerKind::PostgresFlexible => {
                ("Microsoft.DBforPostgreSQL/flexibleServers", POSTGRES_API)
            }
        };
        let items = self
            .list(&format!(
                "/subscriptions/{subscription}/providers/{provider}?api-version={api}"
            ))
            .await?;
        Ok(items
            .iter()
            .filter_map(|v| {
                let props = &v["properties"];
                Some(Server {
                    kind,
                    subscription_id: subscription.to_string(),
                    id: v["id"].as_str()?.to_string(),
                    name: v["name"].as_str()?.to_string(),
                    host: props["fullyQualifiedDomainName"].as_str()?.to_string(),
                    location: v["location"].as_str().unwrap_or_default().to_string(),
                    admin_login: props["administratorLogin"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(Into::into),
                })
            })
            .collect())
    }

    async fn databases(&self, s: &Server) -> std::result::Result<Vec<AzureDatabase>, String> {
        let (api, skip, port) = match s.kind {
            AzureServerKind::SqlServer | AzureServerKind::SqlManagedInstance => {
                (SQL_API, SKIP_SQL, 1433)
            }
            AzureServerKind::PostgresFlexible => (POSTGRES_API, SKIP_POSTGRES, 5432),
        };
        let items = self
            .list(&format!("{}/databases?api-version={api}", s.id))
            .await?;
        Ok(items
            .iter()
            .filter_map(|v| v["name"].as_str())
            .filter(|name| !skip.contains(name))
            .map(|name| AzureDatabase {
                kind: s.kind,
                subscription_id: s.subscription_id.clone(),
                resource_group: resource_group(&s.id).unwrap_or_default(),
                server: s.name.clone(),
                host: s.host.clone(),
                port,
                database: name.to_string(),
                location: s.location.clone(),
                admin_login: s.admin_login.clone(),
            })
            .collect())
    }

    /// Every item of a paged ARM list (`value` + `nextLink`).
    async fn list(&self, path: &str) -> std::result::Result<Vec<Value>, String> {
        let mut url = format!("{}{path}", self.base);
        let mut items = Vec::new();
        for _ in 0..MAX_PAGES {
            let res = self
                .http
                .get(&url)
                .bearer_auth(&self.token)
                .send()
                .await
                .map_err(|e| e.without_url().to_string())?;
            let status = res.status();
            let body: Value = res
                .json()
                .await
                .map_err(|_| format!("unexpected response (HTTP {status})"))?;
            if !status.is_success() {
                return Err(arm_error(&body).unwrap_or_else(|| format!("HTTP {status}")));
            }
            if let Some(v) = body["value"].as_array() {
                items.extend(v.iter().cloned());
            }
            match body["nextLink"].as_str() {
                // Only follow links back to the same endpoint: the token goes with them.
                Some(next) if next.starts_with(&self.base) => url = next.to_string(),
                _ => break,
            }
        }
        Ok(items)
    }
}

/// `/subscriptions/{s}/resourceGroups/{rg}/providers/…` → `rg`.
fn resource_group(id: &str) -> Option<String> {
    let mut parts = id.split('/');
    parts
        .by_ref()
        .find(|p| p.eq_ignore_ascii_case("resourceGroups"))?;
    parts.next().map(Into::into)
}

/// `{"error": {"code": "AuthorizationFailed", "message": "…"}}` → `AuthorizationFailed: …`.
fn arm_error(body: &Value) -> Option<String> {
    let err = &body["error"];
    let code = err["code"].as_str()?;
    Some(match err["message"].as_str() {
        Some(m) => format!("{code}: {}", m.lines().next().unwrap_or(m).trim()),
        None => code.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use axum::extract::{Path, Query};
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::get;
    use axum::{Json, Router};
    use serde_json::json;
    use std::collections::HashMap;

    use super::*;

    /// A fake Resource Manager with two subscriptions: `s1` holds a SQL
    /// server (two pages of databases) and a PostgreSQL server; `s2` refuses.
    async fn fake_arm() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let b = base.clone();
        let authorized = |h: &HeaderMap| h["authorization"] == "Bearer tok";
        let app = Router::new()
            .route(
                "/subscriptions",
                get(move |h: HeaderMap| async move {
                    if !authorized(&h) {
                        return (
                            StatusCode::UNAUTHORIZED,
                            Json(json!({"error": {"code": "InvalidAuthenticationToken",
                                "message": "The access token is invalid."}})),
                        );
                    }
                    (StatusCode::OK, Json(json!({"value": [
                        {"subscriptionId": "s1", "displayName": "Dev", "state": "Enabled"},
                        {"subscriptionId": "s2", "displayName": "Locked", "state": "Enabled"},
                        {"subscriptionId": "s3", "displayName": "Gone", "state": "Disabled"},
                    ]})))
                }),
            )
            .route(
                "/subscriptions/{sub}/providers/{ns}/{kind}",
                get(move |Path((sub, ns, kind)): Path<(String, String, String)>| async move {
                    let rg = "/subscriptions/s1/resourceGroups/rg-data/providers";
                    match (sub.as_str(), ns.as_str(), kind.as_str()) {
                        ("s2", _, _) => (
                            StatusCode::FORBIDDEN,
                            Json(json!({"error": {"code": "AuthorizationFailed",
                                "message": "The client does not have authorization.\nmore"}})),
                        ),
                        (_, "Microsoft.Sql", "servers") => (StatusCode::OK, Json(json!({"value": [{
                            "id": format!("{rg}/Microsoft.Sql/servers/sql1"),
                            "name": "sql1", "location": "westeurope",
                            "properties": {"fullyQualifiedDomainName": "sql1.database.windows.net",
                                "administratorLogin": "sqladmin"}}]}))),
                        (_, "Microsoft.Sql", "managedInstances") => (
                            StatusCode::CONFLICT,
                            Json(json!({"error": {"code": "SubscriptionNotRegistered",
                                "message": "not registered"}})),
                        ),
                        (_, "Microsoft.DBforPostgreSQL", _) => (StatusCode::OK, Json(json!({"value": [{
                            "id": format!("{rg}/Microsoft.DBforPostgreSQL/flexibleServers/pg1"),
                            "name": "pg1", "location": "northeurope",
                            "properties": {"fullyQualifiedDomainName": "pg1.postgres.database.azure.com",
                                "administratorLogin": "pgadmin"}}]}))),
                        _ => (StatusCode::NOT_FOUND, Json(json!({}))),
                    }
                }),
            )
            .route(
                "/subscriptions/s1/resourceGroups/rg-data/providers/{ns}/{kind}/{server}/databases",
                get(
                    move |Path((_, _, server)): Path<(String, String, String)>,
                          Query(q): Query<HashMap<String, String>>| {
                        let b = b.clone();
                        async move {
                            Json(match (server.as_str(), q.get("page")) {
                                ("sql1", None) => json!({
                                    "value": [{"name": "master"}, {"name": "sales"}],
                                    "nextLink": format!("{b}/subscriptions/s1/resourceGroups/rg-data/providers/Microsoft.Sql/servers/sql1/databases?api-version=x&page=2"),
                                }),
                                ("sql1", Some(_)) => json!({"value": [{"name": "hr"}]}),
                                _ => json!({"value": [{"name": "app"}, {"name": "azure_sys"},
                                    {"name": "azure_maintenance"}]}),
                            })
                        }
                    },
                ),
            );
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        base
    }

    #[tokio::test]
    async fn discovers_servers_and_databases() {
        let base = fake_arm().await;
        let d = Arm::new(&base, "tok".into())
            .unwrap()
            .discover()
            .await
            .unwrap();
        let ids: Vec<_> = d.subscriptions.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["s1", "s2"], "disabled subscriptions are skipped");
        let found: Vec<_> = d
            .databases
            .iter()
            .map(|x| (x.kind, x.host.as_str(), x.port, x.database.as_str()))
            .collect();
        assert_eq!(
            found,
            [
                (
                    AzureServerKind::PostgresFlexible,
                    "pg1.postgres.database.azure.com",
                    5432,
                    "app"
                ),
                (
                    AzureServerKind::SqlServer,
                    "sql1.database.windows.net",
                    1433,
                    "hr"
                ),
                (
                    AzureServerKind::SqlServer,
                    "sql1.database.windows.net",
                    1433,
                    "sales"
                ),
            ]
        );
        assert_eq!(d.databases[0].resource_group, "rg-data");
        assert_eq!(d.databases[0].admin_login.as_deref(), Some("pgadmin"));
        // s2 refuses each kind; the unregistered provider is not a warning.
        assert_eq!(d.warnings.len(), 3, "{:?}", d.warnings);
        assert!(
            d.warnings[0].starts_with("Locked (Azure SQL): AuthorizationFailed: The client"),
            "{:?}",
            d.warnings
        );
        assert!(!d.warnings[0].contains("more"));
    }

    #[tokio::test]
    async fn a_rejected_token_fails_the_discovery() {
        let base = fake_arm().await;
        let err = Arm::new(&base, "wrong".into())
            .unwrap()
            .discover()
            .await
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("listing Azure subscriptions failed: InvalidAuthenticationToken"),
            "{err}"
        );
    }

    #[test]
    fn resource_groups() {
        assert_eq!(
            resource_group(
                "/subscriptions/s/resourceGroups/rg-1/providers/Microsoft.Sql/servers/x"
            )
            .as_deref(),
            Some("rg-1")
        );
        assert_eq!(resource_group("/subscriptions/s"), None);
    }
}
