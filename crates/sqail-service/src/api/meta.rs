use axum::Json;
use sqail_proto::{API_VERSION, Engine, Health, ServiceInfo};

use crate::auth::Principal;

/// Liveness probe (no authentication).
#[utoipa::path(get, path = "/v1/health", tag = "meta", security(()),
    responses((status = 200, body = Health)))]
pub async fn health() -> Json<Health> {
    Json(Health {
        status: "ok".into(),
        version: env!("CARGO_PKG_VERSION").into(),
    })
}

/// Service version and capabilities, plus the caller's scope.
#[utoipa::path(get, path = "/v1/info", tag = "meta",
    responses((status = 200, body = ServiceInfo), (status = 401, body = sqail_proto::Problem)))]
pub async fn info(p: Principal) -> Json<ServiceInfo> {
    Json(ServiceInfo {
        version: env!("CARGO_PKG_VERSION").into(),
        api_version: API_VERSION.into(),
        engines: Engine::ALL.to_vec(),
        scope: p.scope,
    })
}
