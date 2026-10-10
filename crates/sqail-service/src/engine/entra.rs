//! Microsoft Entra ID (Azure AD) access tokens for Azure SQL, and for Azure
//! Resource Manager when discovering databases.
//!
//! A token is fetched when a connection opens and reused until five minutes
//! before it expires. Secrets and tokens never reach the logs or error
//! messages; errors carry Entra's own explanation (`AADSTS…`).

use std::time::{Duration, Instant};

use serde_json::Value;
use sqail_proto::MssqlAuth;
use tokio::sync::Mutex;

use super::{DbError, Result};

/// Microsoft.Data.SqlClient's public client app, which SSMS and ADO.NET use
/// for Entra password sign-in.
const SQLCLIENT_APP: &str = "2fd908ad-0664-4344-b9be-cd3e8b574c38";
/// The Azure CLI's public client app, pre-authorized for Resource Manager.
const AZURE_CLI_APP: &str = "04b07795-8ddb-461a-bbee-02f9e1bf7b46";
const AUTHORITY: &str = "https://login.microsoftonline.com";
/// Azure Instance Metadata Service (VMs, VM scale sets, AKS node identity).
const IMDS: &str = "http://169.254.169.254/metadata/identity/oauth2/token";
const REFRESH_MARGIN: Duration = Duration::from_secs(300);

/// What a token is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Audience {
    /// Signing in to Azure SQL.
    Sql,
    /// Azure Resource Manager: listing subscriptions and servers.
    Arm,
}

impl Audience {
    /// Token scope (v2 endpoints).
    fn scope(self) -> &'static str {
        match self {
            Audience::Sql => "https://database.windows.net/.default",
            Audience::Arm => "https://management.azure.com/.default",
        }
    }

    /// Resource (managed identity endpoints).
    fn resource(self) -> &'static str {
        match self {
            Audience::Sql => "https://database.windows.net/",
            Audience::Arm => "https://management.azure.com/",
        }
    }

    /// Public client app for password sign-in when the profile names none.
    fn password_app(self) -> &'static str {
        match self {
            Audience::Sql => SQLCLIENT_APP,
            Audience::Arm => AZURE_CLI_APP,
        }
    }
}

enum Flow {
    Password {
        tenant: String,
        client_id: String,
        user: String,
        password: String,
    },
    ServicePrincipal {
        tenant: String,
        client_id: String,
        secret: String,
    },
    ManagedIdentity {
        client_id: Option<String>,
    },
}

pub struct Entra {
    flow: Flow,
    audience: Audience,
    authority: String,
    imds: String,
    /// App Service, Functions and Container Apps: `IDENTITY_ENDPOINT` and
    /// `IDENTITY_HEADER`, used instead of IMDS when set.
    app_service: Option<(String, String)>,
    http: reqwest::Client,
    cached: Mutex<Option<(String, Instant)>>,
}

impl Entra {
    /// An Azure SQL token source; `None` for methods that are not Entra ID.
    pub fn from_auth(auth: &MssqlAuth, password: Option<&str>) -> Result<Option<Self>> {
        Self::for_audience(auth, password, Audience::Sql)
    }

    /// `None` for authentication methods that are not Entra ID.
    pub fn for_audience(
        auth: &MssqlAuth,
        password: Option<&str>,
        audience: Audience,
    ) -> Result<Option<Self>> {
        let secret = |what: &str| match password {
            Some(p) if !p.is_empty() => Ok(p.to_string()),
            _ => Err(DbError::Invalid(format!("{what} is required"))),
        };
        let flow = match auth {
            MssqlAuth::EntraPassword {
                user,
                tenant,
                client_id,
            } => Flow::Password {
                tenant: tenant_of(tenant.as_deref().unwrap_or("organizations"))?,
                client_id: nonempty(client_id.as_deref())
                    .unwrap_or(audience.password_app())
                    .into(),
                user: user.trim().into(),
                password: secret("password")?,
            },
            MssqlAuth::EntraServicePrincipal { tenant, client_id } => Flow::ServicePrincipal {
                tenant: tenant_of(tenant)?,
                client_id: client_id.trim().into(),
                secret: secret("client secret")?,
            },
            MssqlAuth::EntraManagedIdentity { client_id } => Flow::ManagedIdentity {
                client_id: nonempty(client_id.as_deref()).map(Into::into),
            },
            MssqlAuth::Sql { .. } | MssqlAuth::Integrated => return Ok(None),
        };
        let app_service = match (
            std::env::var("IDENTITY_ENDPOINT"),
            std::env::var("IDENTITY_HEADER"),
        ) {
            (Ok(e), Ok(h)) if !e.is_empty() && !h.is_empty() => Some((e, h)),
            _ => None,
        };
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| DbError::Internal(e.to_string()))?;
        Ok(Some(Self {
            flow,
            audience,
            authority: AUTHORITY.into(),
            imds: IMDS.into(),
            app_service,
            http,
            cached: Mutex::new(None),
        }))
    }

    /// A valid access token, from the cache or freshly fetched.
    pub async fn token(&self) -> Result<String> {
        let mut cached = self.cached.lock().await;
        if let Some((token, expires)) = &*cached
            && Instant::now() + REFRESH_MARGIN < *expires
        {
            return Ok(token.clone());
        }
        let (token, lifetime) = self
            .fetch()
            .await
            .map_err(|e| DbError::Connect(format!("Microsoft Entra sign-in failed: {e}")))?;
        *cached = Some((token.clone(), Instant::now() + lifetime));
        Ok(token)
    }

    async fn fetch(&self) -> std::result::Result<(String, Duration), String> {
        let scope = self.audience.scope();
        let token_url = |tenant: &str| format!("{}/{tenant}/oauth2/v2.0/token", self.authority);
        let req = match &self.flow {
            Flow::Password {
                tenant,
                client_id,
                user,
                password,
            } => self.http.post(token_url(tenant)).form(&[
                ("grant_type", "password"),
                ("client_id", client_id),
                ("scope", scope),
                ("username", user),
                ("password", password),
            ]),
            Flow::ServicePrincipal {
                tenant,
                client_id,
                secret,
            } => self.http.post(token_url(tenant)).form(&[
                ("grant_type", "client_credentials"),
                ("client_id", client_id),
                ("client_secret", secret),
                ("scope", scope),
            ]),
            Flow::ManagedIdentity { client_id } => {
                let mut query = vec![("resource", self.audience.resource())];
                if let Some(id) = client_id {
                    query.push(("client_id", id));
                }
                match &self.app_service {
                    Some((endpoint, header)) => self
                        .http
                        .get(endpoint)
                        .query(&[("api-version", "2019-08-01")])
                        .query(&query)
                        .header("X-IDENTITY-HEADER", header),
                    None => self
                        .http
                        .get(&self.imds)
                        .query(&[("api-version", "2018-02-01")])
                        .query(&query)
                        .header("Metadata", "true"),
                }
            }
        };
        let res = req.send().await.map_err(|e| {
            if matches!(self.flow, Flow::ManagedIdentity { .. }) && self.app_service.is_none() {
                format!(
                    "no managed identity endpoint answered ({}); managed identity only works \
                     when sqail-service runs on Azure",
                    chain(&e)
                )
            } else {
                chain(&e)
            }
        })?;
        let status = res.status();
        let body: Value = res
            .json()
            .await
            .map_err(|_| format!("unexpected response (HTTP {status})"))?;
        if !status.is_success() {
            return Err(error_of(&body).unwrap_or_else(|| format!("HTTP {status}")));
        }
        let token = body["access_token"]
            .as_str()
            .filter(|t| !t.is_empty())
            .ok_or("the response has no access token")?;
        let secs = seconds(&body["expires_in"]).unwrap_or(3600);
        Ok((token.to_string(), Duration::from_secs(secs)))
    }
}

fn nonempty(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}

/// A tenant ID or domain; it becomes part of the token URL's path.
fn tenant_of(t: &str) -> Result<String> {
    let t = t.trim();
    if t.is_empty()
        || !t
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
    {
        return Err(DbError::Invalid(
            "tenant must be a tenant ID or a domain such as contoso.onmicrosoft.com".into(),
        ));
    }
    Ok(t.to_string())
}

/// `expires_in` is a number from Entra and a string from managed identity.
fn seconds(v: &Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_str()?.parse().ok())
}

/// The first line of Entra's `error_description` (the rest is trace and
/// correlation IDs), or App Service's `message`.
fn error_of(body: &Value) -> Option<String> {
    let text = body["error_description"]
        .as_str()
        .or_else(|| body["message"].as_str())
        .or_else(|| body["error"].as_str())?;
    Some(text.lines().next().unwrap_or(text).trim().to_string())
}

fn chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut src = e.source();
    while let Some(inner) = src {
        s.push_str(": ");
        s.push_str(&inner.to_string());
        src = inner.source();
    }
    s
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::extract::{Path, Query};
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::{get, post};
    use axum::{Form, Json, Router};
    use serde_json::json;

    use super::*;

    /// A fake Entra token endpoint and IMDS; returns its base URL and a call counter.
    async fn fake_entra() -> (String, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let c1 = calls.clone();
        let c2 = calls.clone();
        let app = Router::new()
            .route(
                "/{tenant}/oauth2/v2.0/token",
                post(
                    move |Path(tenant): Path<String>, Form(f): Form<HashMap<String, String>>| {
                        c1.fetch_add(1, Ordering::SeqCst);
                        async move {
                            let aud = if f["scope"].contains("management") { "arm" } else { "sql" };
                            assert!(
                                f["scope"] == Audience::Sql.scope() || f["scope"] == Audience::Arm.scope()
                            );
                            let ok = match f["grant_type"].as_str() {
                                "password" => f["password"] == "pw",
                                "client_credentials" => f["client_secret"] == "secret",
                                _ => false,
                            };
                            if ok {
                                let t = match aud {
                                    "sql" => format!("tok-{tenant}-{}", f["client_id"]),
                                    _ => format!("{aud}-{tenant}-{}", f["client_id"]),
                                };
                                (StatusCode::OK, Json(json!({"access_token": t, "expires_in": 3599})))
                            } else {
                                (
                                    StatusCode::BAD_REQUEST,
                                    Json(json!({"error": "invalid_grant",
                                        "error_description": "AADSTS50126: Error validating credentials.\r\nTrace ID: x"})),
                                )
                            }
                        }
                    },
                ),
            )
            .route(
                "/imds",
                get(
                    move |headers: HeaderMap, Query(q): Query<HashMap<String, String>>| {
                        c2.fetch_add(1, Ordering::SeqCst);
                        async move {
                            assert_eq!(headers["metadata"], "true");
                            assert_eq!(q["resource"], Audience::Sql.resource());
                            let id = q.get("client_id").cloned().unwrap_or("system".into());
                            Json(json!({"access_token": format!("mi-{id}"), "expires_in": "86399"}))
                        }
                    },
                ),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (base, calls)
    }

    fn entra(base: &str, auth: MssqlAuth, password: Option<&str>) -> Entra {
        let mut e = Entra::from_auth(&auth, password).unwrap().unwrap();
        e.authority = base.into();
        e.imds = format!("{base}/imds");
        e.app_service = None;
        e
    }

    #[tokio::test]
    async fn service_principal_token_is_cached() {
        let (base, calls) = fake_entra().await;
        let auth = MssqlAuth::EntraServicePrincipal {
            tenant: "contoso.onmicrosoft.com".into(),
            client_id: "app".into(),
        };
        let e = entra(&base, auth, Some("secret"));
        assert_eq!(e.token().await.unwrap(), "tok-contoso.onmicrosoft.com-app");
        assert_eq!(e.token().await.unwrap(), "tok-contoso.onmicrosoft.com-app");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "second call used the cache"
        );
    }

    #[tokio::test]
    async fn password_defaults_to_organizations_and_sqlclient() {
        let (base, _) = fake_entra().await;
        let auth = MssqlAuth::EntraPassword {
            user: "ann@contoso.com".into(),
            tenant: None,
            client_id: None,
        };
        let e = entra(&base, auth, Some("pw"));
        assert_eq!(
            e.token().await.unwrap(),
            format!("tok-organizations-{SQLCLIENT_APP}")
        );
    }

    #[tokio::test]
    async fn rejected_credentials_explain_without_leaking() {
        let (base, _) = fake_entra().await;
        let auth = MssqlAuth::EntraPassword {
            user: "ann@contoso.com".into(),
            tenant: Some("contoso.com".into()),
            client_id: None,
        };
        let e = entra(&base, auth, Some("WRONG_PW_SENTINEL"));
        let msg = e.token().await.unwrap_err().to_string();
        assert!(msg.contains("AADSTS50126"), "{msg}");
        assert!(!msg.contains("Trace ID"), "{msg}");
        assert!(!msg.contains("WRONG_PW_SENTINEL"), "{msg}");
    }

    #[tokio::test]
    async fn managed_identity_uses_imds() {
        let (base, _) = fake_entra().await;
        let system = entra(
            &base,
            MssqlAuth::EntraManagedIdentity { client_id: None },
            None,
        );
        assert_eq!(system.token().await.unwrap(), "mi-system");
        let user = MssqlAuth::EntraManagedIdentity {
            client_id: Some("uami".into()),
        };
        assert_eq!(entra(&base, user, None).token().await.unwrap(), "mi-uami");
    }

    #[tokio::test]
    async fn resource_manager_tokens_default_to_the_azure_cli_app() {
        let (base, _) = fake_entra().await;
        let auth = MssqlAuth::EntraPassword {
            user: "ann@contoso.com".into(),
            tenant: None,
            client_id: None,
        };
        let mut e = Entra::for_audience(&auth, Some("pw"), Audience::Arm)
            .unwrap()
            .unwrap();
        e.authority = base.clone();
        assert_eq!(
            e.token().await.unwrap(),
            format!("arm-organizations-{AZURE_CLI_APP}")
        );
        let sp = MssqlAuth::EntraServicePrincipal {
            tenant: "t".into(),
            client_id: "app".into(),
        };
        let mut e = Entra::for_audience(&sp, Some("secret"), Audience::Arm)
            .unwrap()
            .unwrap();
        e.authority = base;
        assert_eq!(e.token().await.unwrap(), "arm-t-app");
    }

    #[test]
    fn secrets_and_tenants_are_checked() {
        let sp = MssqlAuth::EntraServicePrincipal {
            tenant: "t".into(),
            client_id: "c".into(),
        };
        assert!(Entra::from_auth(&sp, None).is_err());
        assert!(Entra::from_auth(&sp, Some("")).is_err());
        let bad = MssqlAuth::EntraServicePrincipal {
            tenant: "../evil".into(),
            client_id: "c".into(),
        };
        assert!(Entra::from_auth(&bad, Some("s")).is_err());
        let sql = MssqlAuth::Sql { user: "sa".into() };
        assert!(Entra::from_auth(&sql, Some("pw")).unwrap().is_none());
    }
}
