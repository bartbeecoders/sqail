//! HTTP routes. Every route except `/v1/health`, the OpenAPI document and
//! the admin page's static files requires a bearer token.

pub mod admin;
mod admin_ui;
mod audit;
mod connections;
mod explain;
mod meta;
mod query;
mod schema;
mod tokens;

use std::sync::Arc;
use std::time::Duration;

use axum::http::{HeaderName, HeaderValue, StatusCode, header};
use axum::{Router, middleware};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::sensitive_headers::SetSensitiveRequestHeadersLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::auth::{Principal, require_token};
use crate::engine::Pool;
use crate::engine::registry::DriverSpec;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::store::StoredConnection;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "sqail-service",
        description = "HTTPS gateway between sqail and SQL databases. Query results stream as NDJSON (`application/x-ndjson`), one `QueryEvent` per line.",
    ),
    modifiers(&BearerAuth),
    security(("bearer" = [])),
    tags(
        (name = "meta", description = "Health and service info"),
        (name = "tokens", description = "API tokens (admin)"),
        (name = "connections", description = "Connection profiles"),
        (name = "queries", description = "Running SQL"),
        (name = "sessions", description = "Dedicated connections for transactions"),
        (name = "schema", description = "Catalog browsing"),
        (name = "audit", description = "Audit log (admin)"),
        (name = "admin", description = "Service status and settings, for the admin page (admin)"),
    )
)]
struct ApiDoc;

struct BearerAuth;

impl Modify for BearerAuth {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer",
            SecurityScheme::Http(HttpBuilder::new().scheme(HttpAuthScheme::Bearer).build()),
        );
    }
}

pub fn router(state: AppState) -> Router {
    let protected = OpenApiRouter::new()
        .routes(routes!(meta::info))
        .routes(routes!(tokens::list, tokens::create))
        .routes(routes!(tokens::revoke))
        .routes(routes!(connections::list, connections::create))
        .routes(routes!(
            connections::get,
            connections::update,
            connections::delete
        ))
        .routes(routes!(connections::test_saved))
        .routes(routes!(connections::test_unsaved))
        .routes(routes!(connections::databases_unsaved))
        .routes(routes!(query::run))
        .routes(routes!(query::cancel))
        .routes(routes!(explain::on_connection))
        .routes(routes!(explain::on_session))
        .routes(routes!(query::session_list, query::session_create))
        .routes(routes!(query::session_get, query::session_close))
        .routes(routes!(query::session_run))
        .routes(routes!(schema::databases))
        .routes(routes!(schema::schemas))
        .routes(routes!(schema::tables))
        .routes(routes!(schema::columns))
        .routes(routes!(schema::indexes))
        .routes(routes!(schema::foreign_keys))
        .routes(routes!(schema::privileges))
        .routes(routes!(schema::routines))
        .routes(routes!(schema::ddl))
        .routes(routes!(audit::list))
        .routes(routes!(admin::status))
        .routes(routes!(admin::settings, admin::update_settings))
        .routes(routes!(admin::restart))
        .routes(routes!(admin::upload_certificate))
        .routes(routes!(admin::backup))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_token));

    let (api, openapi) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .routes(routes!(meta::health))
        .merge(protected)
        .split_for_parts();
    let openapi = Arc::new(openapi);

    let mut app = api.route(
        "/v1/openapi.json",
        axum::routing::get({
            let doc = openapi.clone();
            move || async move { axum::Json(doc.as_ref().clone()) }
        }),
    );
    if state.config.docs_ui {
        let html = utoipa_scalar::Scalar::new(openapi.as_ref().clone())
            .title("sqail-service API")
            .to_html();
        app = app.route(
            "/docs",
            axum::routing::get(move || async move { axum::response::Html(html) }),
        );
    }
    if state.config.admin_ui {
        app = app.merge(admin_ui::routes());
    }

    let request_id = HeaderName::from_static("x-request-id");
    // The timeout covers producing the response head; streamed query bodies
    // are governed by the per-query timeout instead.
    let timeout = Duration::from_secs(state.config.limits.request_timeout_secs);
    app.with_state(state.clone())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::GATEWAY_TIMEOUT,
            timeout,
        ))
        .layer(RequestBodyLimitLayer::new(
            state.config.limits.body_limit_bytes,
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
        .layer(PropagateRequestIdLayer::new(request_id.clone()))
        .layer(TraceLayer::new_for_http())
        .layer(SetRequestIdLayer::new(request_id, MakeRequestUuid))
        .layer(SetSensitiveRequestHeadersLayer::new([
            header::AUTHORIZATION,
        ]))
}

/// Load a profile and check the caller may use it at all.
pub(crate) fn load_connection(state: &AppState, id: Uuid) -> ApiResult<StoredConnection> {
    state
        .store
        .connection_get(id)?
        .ok_or_else(|| ApiError::not_found("connection"))
}

pub(crate) fn decrypt_password(
    state: &AppState,
    stored: &StoredConnection,
) -> ApiResult<Option<String>> {
    decrypt_secret(state, stored.secret.as_deref())
}

pub(crate) fn decrypt_ssl_key(
    state: &AppState,
    stored: &StoredConnection,
) -> ApiResult<Option<String>> {
    decrypt_secret(state, stored.ssl_key.as_deref())
}

fn decrypt_secret(state: &AppState, stored: Option<&str>) -> ApiResult<Option<String>> {
    stored
        .map(|s| state.key.decrypt(s))
        .transpose()
        .map_err(ApiError::from)
}

/// The pool for a profile, enforcing the scope rules for running SQL:
/// `read` tokens may only use read-only profiles.
pub(crate) fn pool_for(
    state: &AppState,
    principal: &Principal,
    id: Uuid,
) -> ApiResult<(StoredConnection, Arc<Pool>)> {
    let stored = load_connection(state, id)?;
    if !stored.info.read_only {
        principal.require(sqail_proto::Scope::Query)?;
    }
    let password = decrypt_password(state, &stored)?;
    let ssl_key = decrypt_ssl_key(state, &stored)?;
    let pool = state.pools.pool(
        id,
        stored.info.updated_at,
        state.config.limits.pool_size,
        &DriverSpec {
            params: &stored.info.params,
            password: password.as_deref(),
            ssl_client_key: ssl_key.as_deref(),
            read_only: stored.info.read_only,
            sqlite_dirs: &state.config.sqlite.allowed_dirs,
        },
    )?;
    Ok((stored, pool))
}
