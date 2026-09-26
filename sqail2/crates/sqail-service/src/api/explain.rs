//! Query plans.

use axum::Json;
use axum::extract::{Path, State};
use sqail_proto::{ExplainRequest, Plan, Scope};
use uuid::Uuid;

use super::pool_for;
use super::query::owned_session;
use crate::auth::Principal;
use crate::engine::{DbError, explain};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::store::AuditEvent;

fn audit(state: &AppState, p: &Principal, target: Uuid, req: &ExplainRequest, ok: bool) {
    let a = &state.config.audit;
    state.store.audit(AuditEvent {
        actor: &p.name,
        action: if req.analyze {
            "explain.analyze"
        } else {
            "explain"
        },
        target: Some(target.to_string()),
        detail: a
            .log_sql
            .then(|| req.sql.chars().take(a.max_sql_len).collect()),
        duration_ms: None,
        success: ok,
    });
}

/// Plan for one statement on a pooled connection.
#[utoipa::path(post, path = "/v1/connections/{id}/explain", tag = "queries",
    params(("id" = Uuid, Path)), request_body = ExplainRequest,
    responses((status = 200, body = Plan), (status = 400, body = sqail_proto::Problem)))]
pub async fn on_connection(
    State(state): State<AppState>,
    p: Principal,
    Path(id): Path<Uuid>,
    Json(req): Json<ExplainRequest>,
) -> ApiResult<Json<Plan>> {
    if req.analyze {
        // ANALYZE executes the statement.
        p.require(Scope::Query)?;
    }
    let (_, pool) = pool_for(&state, &p, id)?;
    let mut c = pool.get().await?;
    let res = explain::explain(c.conn(), &req.sql, req.analyze).await;
    if matches!(res, Err(DbError::Connect(_))) {
        c.mark_broken();
    }
    audit(&state, &p, id, &req, res.is_ok());
    Ok(Json(res?))
}

/// Plan for one statement on a session (sees its temp tables and settings).
#[utoipa::path(post, path = "/v1/sessions/{id}/explain", tag = "sessions",
    params(("id" = Uuid, Path)), request_body = ExplainRequest,
    responses((status = 200, body = Plan), (status = 409, body = sqail_proto::Problem)))]
pub async fn on_session(
    State(state): State<AppState>,
    p: Principal,
    Path(id): Path<Uuid>,
    Json(req): Json<ExplainRequest>,
) -> ApiResult<Json<Plan>> {
    let session = owned_session(&state, &p, id)?;
    pool_for(&state, &p, session.connection_id)?;
    let mut guard = session
        .conn
        .clone()
        .try_lock_owned()
        .map_err(|_| ApiError::conflict("the session is running another query"))?;
    let conn = guard.as_deref_mut().ok_or_else(|| {
        ApiError::conflict("the session's connection was lost; open a new session")
    })?;
    let res = explain::explain(conn, &req.sql, req.analyze).await;
    session.touch();
    audit(&state, &p, session.connection_id, &req, res.is_ok());
    Ok(Json(res?))
}
