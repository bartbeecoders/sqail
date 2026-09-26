use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use sqail_proto::{CreateToken, CreatedToken, Scope, TokenInfo};
use uuid::Uuid;

use crate::auth::Principal;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::store::AuditEvent;

#[utoipa::path(get, path = "/v1/tokens", tag = "tokens",
    responses((status = 200, body = Vec<TokenInfo>)))]
pub async fn list(State(state): State<AppState>, p: Principal) -> ApiResult<Json<Vec<TokenInfo>>> {
    p.require(Scope::Admin)?;
    Ok(Json(state.store.token_list()?))
}

/// Create a token. The secret is returned once and never stored in clear.
#[utoipa::path(post, path = "/v1/tokens", tag = "tokens", request_body = CreateToken,
    responses((status = 201, body = CreatedToken)))]
pub async fn create(
    State(state): State<AppState>,
    p: Principal,
    Json(body): Json<CreateToken>,
) -> ApiResult<(StatusCode, Json<CreatedToken>)> {
    p.require(Scope::Admin)?;
    let name = body.name.trim();
    if name.is_empty() || name.len() > 100 {
        return Err(ApiError::bad_request("name must be 1-100 characters"));
    }
    let created = state.store.token_create(name, body.scope)?;
    state.store.audit(AuditEvent {
        actor: &p.name,
        action: "token.create",
        target: Some(created.info.id.to_string()),
        detail: Some(format!("{} ({})", created.info.name, body.scope.as_str())),
        duration_ms: None,
        success: true,
    });
    Ok((StatusCode::CREATED, Json(created)))
}

#[utoipa::path(delete, path = "/v1/tokens/{id}", tag = "tokens",
    params(("id" = Uuid, Path)), responses((status = 204), (status = 404, body = sqail_proto::Problem)))]
pub async fn revoke(
    State(state): State<AppState>,
    p: Principal,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    p.require(Scope::Admin)?;
    if !state.store.token_revoke(id)? {
        return Err(ApiError::not_found("active token"));
    }
    state.store.audit(AuditEvent {
        actor: &p.name,
        action: "token.revoke",
        target: Some(id.to_string()),
        detail: None,
        duration_ms: None,
        success: true,
    });
    Ok(StatusCode::NO_CONTENT)
}
