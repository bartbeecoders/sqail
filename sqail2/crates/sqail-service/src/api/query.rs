//! Running SQL: streamed NDJSON responses, cancellation, timeouts, sessions.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use futures::StreamExt;
use sqail_proto::{
    CreateSession, NDJSON, QUERY_ID_HEADER, QueryEvent, QueryRequest, Scope, SessionInfo,
};
use tokio::sync::{OwnedMutexGuard, mpsc};
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::pool_for;
use crate::auth::Principal;
use crate::engine::{Conn, DbError, EventSink, ExecRequest, Pooled};
use crate::error::{ApiError, ApiResult};
use crate::sessions::Session;
use crate::state::{AppState, RunningQuery};
use crate::store::AuditEvent;

/// Grace period for a cancelled query to stop before its connection is dropped.
const CANCEL_GRACE: Duration = Duration::from_secs(5);

/// Where the query runs.
enum Target {
    Pooled(Pooled),
    Session {
        session: Arc<Session>,
        guard: OwnedMutexGuard<Option<Box<dyn Conn>>>,
    },
}

/// Run a script on a pooled connection; the response streams `QueryEvent`s.
#[utoipa::path(post, path = "/v1/connections/{id}/query", tag = "queries",
    params(("id" = Uuid, Path), ("x-query-id" = Option<Uuid>, Header, description = "Client-chosen id, usable with DELETE /v1/queries/{id}")),
    request_body = QueryRequest,
    responses((status = 200, description = "NDJSON stream of QueryEvent", content_type = "application/x-ndjson", body = QueryEvent)))]
pub async fn run(
    State(state): State<AppState>,
    p: Principal,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(req): Json<QueryRequest>,
) -> ApiResult<Response> {
    let (_, pool) = pool_for(&state, &p, id)?;
    let query_id = query_id(&headers)?;
    // Connect before answering so connection errors are a proper HTTP error.
    let conn = pool.get().await?;
    start(state, p, id, query_id, req, Target::Pooled(conn))
}

/// Cancel a running query. Only its owner (or an admin) may cancel it.
#[utoipa::path(delete, path = "/v1/queries/{id}", tag = "queries",
    params(("id" = Uuid, Path)), responses((status = 202), (status = 404, body = sqail_proto::Problem)))]
pub async fn cancel(
    State(state): State<AppState>,
    p: Principal,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let queries = state.queries.lock().unwrap_or_else(|e| e.into_inner());
    match queries.get(&id) {
        Some(q) if q.owner == p.token_id || p.scope == Scope::Admin => {
            q.cancel.cancel();
            Ok(StatusCode::ACCEPTED)
        }
        _ => Err(ApiError::not_found("running query")),
    }
}

fn query_id(headers: &HeaderMap) -> ApiResult<Uuid> {
    match headers.get(QUERY_ID_HEADER) {
        None => Ok(Uuid::new_v4()),
        Some(v) => v
            .to_str()
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| ApiError::bad_request("x-query-id must be a UUID")),
    }
}

fn start(
    state: AppState,
    p: Principal,
    connection_id: Uuid,
    query_id: Uuid,
    req: QueryRequest,
    target: Target,
) -> ApiResult<Response> {
    if req.sql.trim().is_empty() {
        return Err(ApiError::bad_request("sql is empty"));
    }
    let limits = &state.config.limits;
    let max_rows = req
        .max_rows
        .unwrap_or(limits.default_max_rows)
        .clamp(1, limits.max_rows);
    let timeout_ms = req.timeout_ms.unwrap_or(limits.default_timeout_ms);
    let timeout =
        (timeout_ms > 0).then(|| Duration::from_millis(timeout_ms.min(limits.max_timeout_ms)));

    let cancel = CancellationToken::new();
    {
        let mut queries = state.queries.lock().unwrap_or_else(|e| e.into_inner());
        if queries.contains_key(&query_id) {
            return Err(ApiError::conflict(
                "a query with this id is already running",
            ));
        }
        queries.insert(
            query_id,
            RunningQuery {
                owner: p.token_id,
                cancel: cancel.clone(),
            },
        );
    }

    let (tx, rx) = mpsc::channel::<QueryEvent>(32);
    let exec = ExecRequest {
        sql: req.sql,
        params: req.params,
        max_rows,
    };
    tokio::spawn(drive(
        state,
        p,
        connection_id,
        query_id,
        exec,
        target,
        cancel,
        timeout,
        tx,
    ));

    let body = ReceiverStream::new(rx).map(|ev| {
        let mut line = serde_json::to_vec(&ev).expect("events serialize");
        line.push(b'\n');
        Ok::<_, Infallible>(Bytes::from(line))
    });
    let mut res = Body::from_stream(body).into_response();
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(NDJSON));
    h.insert(
        QUERY_ID_HEADER,
        HeaderValue::from_str(&query_id.to_string()).expect("uuid is a valid header"),
    );
    Ok(res)
}

#[allow(clippy::too_many_arguments)]
async fn drive(
    state: AppState,
    p: Principal,
    connection_id: Uuid,
    query_id: Uuid,
    exec: ExecRequest,
    mut target: Target,
    cancel: CancellationToken,
    timeout: Option<Duration>,
    tx: mpsc::Sender<QueryEvent>,
) {
    let started = Instant::now();
    let sink = EventSink::Channel(tx);
    let _ = sink.send(QueryEvent::Started { query_id }).await;

    let conn: &mut dyn Conn = match &mut target {
        Target::Pooled(c) => c.conn(),
        Target::Session { guard, .. } => guard.as_deref_mut().expect("checked before start"),
    };
    let (result, mut broken) = run_controlled(conn, &exec, &sink, &cancel, timeout).await;

    if matches!(result, Err(DbError::Cancelled | DbError::Timeout)) && !broken {
        broken = tokio::time::timeout(Duration::from_secs(3), conn.ping())
            .await
            .map_or(true, |r| r.is_err());
    }
    if matches!(result, Err(DbError::Connect(_))) {
        broken = true;
    }

    // Transaction state decides what happens to the connection next.
    let in_tx = if broken {
        Some(false)
    } else {
        match tokio::time::timeout(Duration::from_secs(3), conn.in_transaction()).await {
            Ok(Ok(v)) => Some(v),
            _ => {
                broken = true;
                None
            }
        }
    };

    if let Err(e) = &result
        && !matches!(e, DbError::ClientGone)
    {
        let _ = sink.send(e.to_event()).await;
    }

    let session_in_tx = match &mut target {
        Target::Pooled(c) => {
            if in_tx == Some(true) {
                let _ = sink
                    .send(QueryEvent::Message {
                        severity: "warning".into(),
                        text: "The script left a transaction open. It was rolled back because \
                               the query did not run in a session."
                            .into(),
                    })
                    .await;
                broken = true;
            }
            if broken {
                c.mark_broken();
            }
            None
        }
        Target::Session { session, guard } => {
            if broken {
                **guard = None;
                let _ = sink
                    .send(QueryEvent::Message {
                        severity: "error".into(),
                        text: "The session's connection was lost; open a new session.".into(),
                    })
                    .await;
            }
            let v = in_tx.unwrap_or(false);
            session.set_in_transaction(v);
            session.touch();
            Some(v)
        }
    };

    let elapsed_ms = started.elapsed().as_millis() as u64;
    let _ = sink
        .send(QueryEvent::Done {
            elapsed_ms,
            cancelled: matches!(result, Err(DbError::Cancelled)),
            in_transaction: session_in_tx,
        })
        .await;

    state
        .queries
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&query_id);
    let audit = &state.config.audit;
    state.store.audit(AuditEvent {
        actor: &p.name,
        action: "query",
        target: Some(connection_id.to_string()),
        detail: audit
            .log_sql
            .then(|| truncate(&exec.sql, audit.max_sql_len)),
        duration_ms: Some(elapsed_ms as i64),
        success: result.is_ok(),
    });
}

/// Execute with cancellation and timeout. Returns the result and whether the
/// connection must be discarded (it did not stop within the grace period).
async fn run_controlled(
    conn: &mut dyn Conn,
    exec: &ExecRequest,
    sink: &EventSink,
    cancel: &CancellationToken,
    timeout: Option<Duration>,
) -> (crate::engine::Result<()>, bool) {
    let canceller = conn.canceller();
    let fut = conn.execute(exec, sink);
    tokio::pin!(fut);
    let deadline = async {
        match timeout {
            Some(d) => tokio::time::sleep(d).await,
            None => std::future::pending().await,
        }
    };
    tokio::pin!(deadline);

    let reason = tokio::select! {
        r = &mut fut => return (r, false),
        _ = cancel.cancelled() => DbError::Cancelled,
        _ = &mut deadline => DbError::Timeout,
    };
    if !canceller.cancel().await {
        return (Err(reason), true);
    }
    match tokio::time::timeout(CANCEL_GRACE, &mut fut).await {
        // The query stopped (normally with a "cancelled" error from the server).
        Ok(_) => (Err(reason), false),
        // Still running: give up on this connection.
        Err(_) => (Err(reason), true),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

// -------------------------------------------------------------- sessions --

pub(crate) fn owned_session(state: &AppState, p: &Principal, id: Uuid) -> ApiResult<Arc<Session>> {
    state
        .sessions
        .get(id)
        .filter(|s| s.owner == p.token_id)
        .ok_or_else(|| ApiError::not_found("session"))
}

#[utoipa::path(get, path = "/v1/sessions", tag = "sessions",
    responses((status = 200, body = Vec<SessionInfo>)))]
pub async fn session_list(State(state): State<AppState>, p: Principal) -> Json<Vec<SessionInfo>> {
    let idle = state.config.sessions.idle_timeout_secs;
    Json(
        state
            .sessions
            .owned_by(p.token_id)
            .iter()
            .map(|s| s.info(idle))
            .collect(),
    )
}

/// Open a dedicated connection. Idle sessions close after `idle_timeout_secs`,
/// rolling back any open transaction.
#[utoipa::path(post, path = "/v1/sessions", tag = "sessions", request_body = CreateSession,
    responses((status = 201, body = SessionInfo)))]
pub async fn session_create(
    State(state): State<AppState>,
    p: Principal,
    Json(body): Json<CreateSession>,
) -> ApiResult<(StatusCode, Json<SessionInfo>)> {
    if state.sessions.owned_by(p.token_id).len() >= state.config.sessions.max_per_token {
        return Err(ApiError::conflict("too many open sessions for this token"));
    }
    let (_, pool) = pool_for(&state, &p, body.connection_id)?;
    let conn = pool.driver().connect().await?;
    let session = Arc::new(Session::new(body.connection_id, p.token_id, conn));
    state.sessions.insert(session.clone());
    state.store.audit(AuditEvent {
        actor: &p.name,
        action: "session.open",
        target: Some(body.connection_id.to_string()),
        detail: Some(session.id.to_string()),
        duration_ms: None,
        success: true,
    });
    Ok((
        StatusCode::CREATED,
        Json(session.info(state.config.sessions.idle_timeout_secs)),
    ))
}

#[utoipa::path(get, path = "/v1/sessions/{id}", tag = "sessions",
    params(("id" = Uuid, Path)), responses((status = 200, body = SessionInfo)))]
pub async fn session_get(
    State(state): State<AppState>,
    p: Principal,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<SessionInfo>> {
    let s = owned_session(&state, &p, id)?;
    Ok(Json(s.info(state.config.sessions.idle_timeout_secs)))
}

/// Close a session. An open transaction is rolled back by the server.
#[utoipa::path(delete, path = "/v1/sessions/{id}", tag = "sessions",
    params(("id" = Uuid, Path)), responses((status = 204)))]
pub async fn session_close(
    State(state): State<AppState>,
    p: Principal,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    owned_session(&state, &p, id)?;
    state.sessions.remove(id);
    Ok(StatusCode::NO_CONTENT)
}

/// Run a script on the session's connection. One query at a time per session.
#[utoipa::path(post, path = "/v1/sessions/{id}/query", tag = "sessions",
    params(("id" = Uuid, Path), ("x-query-id" = Option<Uuid>, Header)),
    request_body = QueryRequest,
    responses((status = 200, description = "NDJSON stream of QueryEvent", content_type = "application/x-ndjson", body = QueryEvent),
              (status = 409, body = sqail_proto::Problem)))]
pub async fn session_run(
    State(state): State<AppState>,
    p: Principal,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(req): Json<QueryRequest>,
) -> ApiResult<Response> {
    let session = owned_session(&state, &p, id)?;
    // Re-check the profile: it may have been made writable/deleted meanwhile.
    pool_for(&state, &p, session.connection_id)?;
    let guard = session
        .conn
        .clone()
        .try_lock_owned()
        .map_err(|_| ApiError::conflict("the session is running another query"))?;
    if guard.is_none() {
        state.sessions.remove(id);
        return Err(ApiError::conflict(
            "the session's connection was lost; open a new session",
        ));
    }
    session.touch();
    let query_id = query_id(&headers)?;
    let connection_id = session.connection_id;
    start(
        state,
        p,
        connection_id,
        query_id,
        req,
        Target::Session { session, guard },
    )
}

#[cfg(test)]
mod tests {
    use super::truncate;

    #[test]
    fn truncate_respects_char_boundaries() {
        assert_eq!(truncate("héllo", 2), "h…");
        assert_eq!(truncate("abc", 5), "abc");
    }
}
