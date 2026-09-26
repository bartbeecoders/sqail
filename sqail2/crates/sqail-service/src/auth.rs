//! Bearer-token authentication, scopes and per-token rate limiting.

use std::num::NonZeroU32;

use axum::extract::{FromRequestParts, Request, State};
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use axum::middleware::Next;
use axum::response::Response;
use governor::{DefaultKeyedRateLimiter, Quota, RateLimiter};
use sqail_proto::Scope;
use uuid::Uuid;

use crate::error::ApiError;
use crate::state::AppState;

/// The authenticated caller, available to handlers as an extractor.
#[derive(Debug, Clone)]
pub struct Principal {
    pub token_id: Uuid,
    pub name: String,
    pub scope: Scope,
}

impl Principal {
    pub fn require(&self, needed: Scope) -> Result<(), ApiError> {
        if self.scope.allows(needed) {
            Ok(())
        } else {
            Err(ApiError::forbidden(format!(
                "requires the '{}' scope",
                needed.as_str()
            )))
        }
    }
}

impl<S: Send + Sync> FromRequestParts<S> for Principal {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<Principal>()
            .cloned()
            .ok_or_else(ApiError::unauthorized)
    }
}

pub fn rate_limiter(per_second: u32, burst: u32) -> DefaultKeyedRateLimiter<Uuid> {
    let rate = NonZeroU32::new(per_second.max(1)).expect("non-zero");
    let burst = NonZeroU32::new(burst.max(1)).expect("non-zero");
    RateLimiter::keyed(Quota::per_second(rate).allow_burst(burst))
}

/// Middleware: validate the bearer token, apply the rate limit, attach the
/// [`Principal`].
pub async fn require_token(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let secret = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .ok_or_else(ApiError::unauthorized)?;
    let info = state
        .store
        .token_by_secret(secret)?
        .filter(|t| !t.revoked)
        .ok_or_else(ApiError::unauthorized)?;
    if state.limiter.check_key(&info.id).is_err() {
        return Err(ApiError::too_many_requests());
    }
    state.touch_token(info.id);
    req.extensions_mut().insert(Principal {
        token_id: info.id,
        name: info.name,
        scope: info.scope,
    });
    Ok(next.run(req).await)
}
