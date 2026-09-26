use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;
use sqail_proto::{AuditPage, Scope};
use utoipa::IntoParams;

use crate::auth::Principal;
use crate::error::ApiResult;
use crate::state::AppState;

#[derive(Debug, Deserialize, IntoParams)]
pub struct AuditQuery {
    /// Return entries older than this id (from `next_before`).
    before: Option<i64>,
    /// Page size, 1-500 (default 100).
    limit: Option<u32>,
}

/// Newest first.
#[utoipa::path(get, path = "/v1/audit", tag = "audit", params(AuditQuery),
    responses((status = 200, body = AuditPage)))]
pub async fn list(
    State(state): State<AppState>,
    p: Principal,
    Query(q): Query<AuditQuery>,
) -> ApiResult<Json<AuditPage>> {
    p.require(Scope::Admin)?;
    let limit = q.limit.unwrap_or(100).clamp(1, 500);
    Ok(Json(state.store.audit_list(q.before, limit)?))
}
