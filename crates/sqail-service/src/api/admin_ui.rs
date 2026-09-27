//! The admin page: static files compiled into the binary, served at
//! `/admin/`. They need no token; every call they make to `/v1` does.

use axum::Router;
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;

use crate::state::AppState;

const INDEX: &str = include_str!("../../admin/index.html");
const SCRIPT: &str = include_str!("../../admin/app.js");
const STYLE: &str = include_str!("../../admin/app.css");
const ICON: &str = include_str!("../../../../packaging/icons/sqail.svg");

/// Only our own files; no inline code, no framing, no form posts elsewhere.
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; \
                   connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(|| async { Redirect::to("/admin/") }))
        .route("/admin", get(|| async { Redirect::to("/admin/") }))
        .route(
            "/admin/",
            get(|| async { asset(INDEX, "text/html; charset=utf-8") }),
        )
        .route(
            "/admin/app.js",
            get(|| async { asset(SCRIPT, "text/javascript; charset=utf-8") }),
        )
        .route(
            "/admin/app.css",
            get(|| async { asset(STYLE, "text/css; charset=utf-8") }),
        )
        .route(
            "/admin/icon.svg",
            get(|| async { asset(ICON, "image/svg+xml") }),
        )
}

fn asset(body: &'static str, content_type: &'static str) -> Response {
    let mut res = body.into_response();
    let headers = res.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    res
}
