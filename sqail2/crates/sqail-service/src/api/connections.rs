use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use sqail_proto::{Connection, ConnectionInput, ConnectionParams, Scope, TestResult};
use uuid::Uuid;

use super::{decrypt_password, load_connection};
use crate::auth::Principal;
use crate::engine::registry::{DriverSpec, build_driver};
use crate::engine::sqlite::check_path;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::store::{AuditEvent, ConnectionRecord};

#[utoipa::path(get, path = "/v1/connections", tag = "connections",
    responses((status = 200, body = Vec<Connection>)))]
pub async fn list(
    State(state): State<AppState>,
    _p: Principal,
) -> ApiResult<Json<Vec<Connection>>> {
    Ok(Json(state.store.connection_list()?))
}

#[utoipa::path(get, path = "/v1/connections/{id}", tag = "connections",
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Connection), (status = 404, body = sqail_proto::Problem)))]
pub async fn get(
    State(state): State<AppState>,
    _p: Principal,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Connection>> {
    Ok(Json(load_connection(&state, id)?.info))
}

fn validate(state: &AppState, input: &ConnectionInput) -> ApiResult<()> {
    let name = input.name.trim();
    if name.is_empty() || name.len() > 200 {
        return Err(ApiError::bad_request("name must be 1-200 characters"));
    }
    if let Some(c) = &input.color
        && !(c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|ch| ch.is_ascii_hexdigit()))
    {
        return Err(ApiError::bad_request("color must look like #rrggbb"));
    }
    match &input.params {
        ConnectionParams::Postgres(p) if p.host.is_empty() || p.database.is_empty() => {
            Err(ApiError::bad_request("host and database are required"))
        }
        ConnectionParams::Mssql(p) if p.host.is_empty() => {
            Err(ApiError::bad_request("host is required"))
        }
        ConnectionParams::Sqlite(p) => {
            check_path(
                std::path::Path::new(&p.path),
                &state.config.sqlite.allowed_dirs,
            )?;
            Ok(())
        }
        _ => Ok(()),
    }
}

fn record<'a>(input: &'a ConnectionInput, secret: Option<&'a str>) -> ConnectionRecord<'a> {
    ConnectionRecord {
        name: input.name.trim(),
        params: &input.params,
        secret,
        read_only: input.read_only,
        color: input.color.as_deref(),
        environment: input.environment.as_deref(),
        folder: input.folder.as_deref(),
    }
}

#[utoipa::path(post, path = "/v1/connections", tag = "connections", request_body = ConnectionInput,
    responses((status = 201, body = Connection), (status = 400, body = sqail_proto::Problem)))]
pub async fn create(
    State(state): State<AppState>,
    p: Principal,
    Json(input): Json<ConnectionInput>,
) -> ApiResult<(StatusCode, Json<Connection>)> {
    p.require(Scope::Admin)?;
    validate(&state, &input)?;
    let secret = match input.password.as_deref() {
        Some(pw) if !pw.is_empty() => Some(state.key.encrypt(pw)?),
        _ => None,
    };
    let id = state
        .store
        .connection_insert(&record(&input, secret.as_deref()))?;
    audit(&state, &p, "connection.create", id, &input.name);
    Ok((StatusCode::CREATED, Json(load_connection(&state, id)?.info)))
}

#[utoipa::path(put, path = "/v1/connections/{id}", tag = "connections", request_body = ConnectionInput,
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Connection), (status = 404, body = sqail_proto::Problem)))]
pub async fn update(
    State(state): State<AppState>,
    p: Principal,
    Path(id): Path<Uuid>,
    Json(input): Json<ConnectionInput>,
) -> ApiResult<Json<Connection>> {
    p.require(Scope::Admin)?;
    validate(&state, &input)?;
    let existing = load_connection(&state, id)?;
    let secret = match input.password.as_deref() {
        None => existing.secret,
        Some("") => None,
        Some(pw) => Some(state.key.encrypt(pw)?),
    };
    state
        .store
        .connection_update(id, &record(&input, secret.as_deref()))?;
    state.pools.invalidate(id);
    audit(&state, &p, "connection.update", id, &input.name);
    Ok(Json(load_connection(&state, id)?.info))
}

#[utoipa::path(delete, path = "/v1/connections/{id}", tag = "connections",
    params(("id" = Uuid, Path)),
    responses((status = 204), (status = 404, body = sqail_proto::Problem)))]
pub async fn delete(
    State(state): State<AppState>,
    p: Principal,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    p.require(Scope::Admin)?;
    let existing = load_connection(&state, id)?;
    state.store.connection_delete(id)?;
    state.pools.invalidate(id);
    state.sessions.close_for_connection(id);
    audit(&state, &p, "connection.delete", id, &existing.info.name);
    Ok(StatusCode::NO_CONTENT)
}

/// Open a fresh connection with a stored profile and report the server version.
#[utoipa::path(post, path = "/v1/connections/{id}/test", tag = "connections",
    params(("id" = Uuid, Path)), responses((status = 200, body = TestResult)))]
pub async fn test_saved(
    State(state): State<AppState>,
    _p: Principal,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<TestResult>> {
    let stored = load_connection(&state, id)?;
    let password = decrypt_password(&state, &stored)?;
    Ok(Json(
        run_test(
            &state,
            &stored.info.params,
            password.as_deref(),
            stored.info.read_only,
        )
        .await,
    ))
}

/// Test a profile before saving it. Admin only: it connects to arbitrary hosts.
#[utoipa::path(post, path = "/v1/connections/test", tag = "connections", request_body = ConnectionInput,
    responses((status = 200, body = TestResult)))]
pub async fn test_unsaved(
    State(state): State<AppState>,
    p: Principal,
    Json(input): Json<ConnectionInput>,
) -> ApiResult<Json<TestResult>> {
    p.require(Scope::Admin)?;
    validate(&state, &input)?;
    Ok(Json(
        run_test(
            &state,
            &input.params,
            input.password.as_deref(),
            input.read_only,
        )
        .await,
    ))
}

async fn run_test(
    state: &AppState,
    params: &ConnectionParams,
    password: Option<&str>,
    read_only: bool,
) -> TestResult {
    let started = Instant::now();
    let res = async {
        let driver = build_driver(&DriverSpec {
            params,
            password,
            read_only,
            sqlite_dirs: &state.config.sqlite.allowed_dirs,
        })?;
        let mut conn = driver.connect().await?;
        conn.server_version().await
    };
    let res = tokio::time::timeout(Duration::from_secs(20), res).await;
    let latency_ms = started.elapsed().as_millis() as u64;
    match res {
        Ok(Ok(version)) => TestResult {
            ok: true,
            latency_ms,
            server_version: Some(version),
            error: None,
        },
        Ok(Err(e)) => TestResult {
            ok: false,
            latency_ms,
            server_version: None,
            error: Some(e.to_string()),
        },
        Err(_) => TestResult {
            ok: false,
            latency_ms,
            server_version: None,
            error: Some("timed out after 20s".into()),
        },
    }
}

fn audit(state: &AppState, p: &Principal, action: &str, id: Uuid, name: &str) {
    state.store.audit(AuditEvent {
        actor: &p.name,
        action,
        target: Some(id.to_string()),
        detail: Some(name.to_string()),
        duration_ms: None,
        success: true,
    });
}
