use serde::{Deserialize, Serialize};

/// Well-known Azure CLI public client ID, used by many developer tools.
const DEFAULT_CLIENT_ID: &str = "04b07795-a710-4e5e-8903-d5a1748a6b73";
const SQL_SCOPE: &str = "https://database.windows.net/.default offline_access";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCodeResponse {
    pub user_code: String,
    pub device_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: u64,
}

#[derive(Debug, Deserialize)]
struct DeviceCodeApiResponse {
    user_code: String,
    device_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TokenPollResponse {
    Success {
        access_token: String,
        refresh_token: Option<String>,
        expires_in: u64,
    },
    Error {
        error: String,
        #[allow(dead_code)]
        error_description: Option<String>,
    },
}

fn resolve_client_id(client_id: &str) -> &str {
    if client_id.is_empty() {
        DEFAULT_CLIENT_ID
    } else {
        client_id
    }
}

fn resolve_tenant(tenant_id: &str) -> &str {
    if tenant_id.is_empty() {
        "organizations"
    } else {
        tenant_id
    }
}

/// Initiate the device code flow. Returns a user code and verification URI
/// that must be shown to the user.
pub async fn start_device_code_flow(
    tenant_id: &str,
    client_id: &str,
) -> Result<DeviceCodeResponse, String> {
    let tenant = resolve_tenant(tenant_id);
    let cid = resolve_client_id(client_id);
    let url = format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/devicecode",
        tenant
    );

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .form(&[("client_id", cid), ("scope", SQL_SCOPE)])
        .send()
        .await
        .map_err(|e| format!("Device code request failed: {e}"))?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Device code request failed: {body}"));
    }

    let api_resp: DeviceCodeApiResponse = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse device code response: {e}"))?;

    Ok(DeviceCodeResponse {
        user_code: api_resp.user_code,
        device_code: api_resp.device_code,
        verification_uri: api_resp.verification_uri,
        expires_in: api_resp.expires_in,
        interval: api_resp.interval,
    })
}

/// Poll the token endpoint until the user completes authentication.
/// This will block (with sleeps) until success, expiry, or an unexpected error.
pub async fn poll_for_token(
    tenant_id: &str,
    client_id: &str,
    device_code: &str,
) -> Result<TokenResponse, String> {
    let tenant = resolve_tenant(tenant_id);
    let cid = resolve_client_id(client_id);
    let url = format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
        tenant
    );

    let client = reqwest::Client::new();
    let interval = std::time::Duration::from_secs(5);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(900); // 15 min max

    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err("Authentication timed out".to_string());
        }

        tokio::time::sleep(interval).await;

        let resp = client
            .post(&url)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", cid),
                ("device_code", device_code),
            ])
            .send()
            .await
            .map_err(|e| format!("Token poll request failed: {e}"))?;

        let body = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read token response: {e}"))?;

        let parsed: TokenPollResponse = serde_json::from_str(&body)
            .map_err(|e| format!("Failed to parse token response: {e}"))?;

        match parsed {
            TokenPollResponse::Success {
                access_token,
                refresh_token,
                expires_in,
            } => {
                return Ok(TokenResponse {
                    access_token,
                    refresh_token,
                    expires_in,
                });
            }
            TokenPollResponse::Error { error, .. } => match error.as_str() {
                "authorization_pending" => continue,
                "slow_down" => {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                }
                "expired_token" => return Err("Device code expired. Please try again.".to_string()),
                "authorization_declined" => {
                    return Err("Authentication was declined by the user.".to_string())
                }
                _ => return Err(format!("Authentication failed: {error}")),
            },
        }
    }
}
