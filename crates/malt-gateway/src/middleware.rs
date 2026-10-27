// Auth + rate-limit enforcement — wires TokenStore/AuthContext/RateLimiter
// (built and unit-tested in isolation, but never previously attached to
// any router) into real axum middleware.
//
// Must be applied as the *last* step, after every route (including any
// added outside `build_router`, e.g. malt-bin's `/shutdown`) has been
// registered — axum's `.layer()` only wraps routes that existed at the
// point it was called, not ones added afterward. See `with_auth`.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::MatchedPath;
use axum::http::{Method, Request};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::Router;

use crate::auth::{AuthContext, AuthScope, TokenStore};
use crate::error::GatewayError;
use crate::rate_limit::RateLimiter;

/// Required scope per (method, route template) pair, matching
/// `docs/design/architecture.md`'s scope table. Route templates are
/// axum's `MatchedPath` form (`{id}`, not a real path segment).
/// Unrecognized (method, path) pairs fail closed to `Admin` — a route
/// this table doesn't know about should never default to open access.
fn required_scope(method: &Method, path: &str) -> AuthScope {
    match (method, path) {
        (&Method::GET, "/health") => AuthScope::Monitor,
        (&Method::GET, "/sessions") => AuthScope::Monitor,
        (&Method::GET, "/isolation/capabilities") => AuthScope::Read,
        (&Method::POST, "/sessions") => AuthScope::Interact,
        (&Method::GET, "/sessions/{id}") => AuthScope::Read,
        (&Method::DELETE, "/sessions/{id}") => AuthScope::Admin,
        (&Method::POST, "/sessions/{id}/exec") => AuthScope::Interact,
        (&Method::POST, "/sessions/{id}/send") => AuthScope::Interact,
        (&Method::POST, "/sessions/{id}/eof") => AuthScope::Interact,
        // Read, not Interact: knowing who holds the keyboard is observation,
        // and FR-020 keeps observation available to clients that cannot type.
        (&Method::GET, "/sessions/{id}/authority") => AuthScope::Read,
        (&Method::GET, "/sessions/{id}/output") => AuthScope::Read,
        (&Method::GET, "/sessions/{id}/output/text") => AuthScope::Read,
        (&Method::GET, "/sessions/{id}/output/stream") => AuthScope::Read,
        // Same sensitivity class as output: command text can contain paths,
        // arguments, and secrets typed at the prompt. Not Monitor, which is
        // for liveness/inventory only.
        (&Method::GET, "/sessions/{id}/history") => AuthScope::Read,
        // Same sensitivity class as history and output: the stream carries
        // command text.
        (&Method::GET, "/sessions/{id}/events") => AuthScope::Read,
        (&Method::GET, "/sessions/{id}/panes") => AuthScope::Read,
        (&Method::POST, "/sessions/{id}/panes/split") => AuthScope::Interact,
        (&Method::DELETE, "/sessions/{id}/panes/{pane_id}") => AuthScope::Interact,
        (&Method::POST, "/shutdown") => AuthScope::Admin,
        _ => AuthScope::Admin,
    }
}

async fn auth_middleware(
    token_store: Arc<TokenStore>,
    rate_limiter: Arc<RateLimiter>,
    req: Request<Body>,
    next: Next,
    body_limit: usize,
) -> Response {
    let (mut response, snapshot) = authorize_and_run(&token_store, &rate_limiter, req, next).await;
    use crate::rate_limit::delta_seconds;
    use axum::http::HeaderValue;
    let headers = response.headers_mut();
    for (name, value) in [
        ("x-ratelimit-limit", snapshot.limit as u64),
        ("x-ratelimit-remaining", snapshot.remaining as u64),
        ("x-ratelimit-reset", delta_seconds(snapshot.reset)),
        ("x-ratelimit-global-limit", snapshot.global_limit as u64),
        (
            "x-ratelimit-global-remaining",
            snapshot.global_remaining as u64,
        ),
        (
            "x-ratelimit-global-reset",
            delta_seconds(snapshot.global_reset),
        ),
        ("x-ratelimit-window", delta_seconds(rate_limiter.window())),
        ("x-request-body-limit", body_limit as u64),
    ] {
        headers.insert(name, HeaderValue::from(value));
    }
    response
}

async fn authorize_and_run(
    token_store: &TokenStore,
    rate_limiter: &RateLimiter,
    req: Request<Body>,
    next: Next,
) -> (Response, crate::rate_limit::RateSnapshot) {
    let unauthenticated = |error: GatewayError| (error.into_response(), rate_limiter.observe(None));
    let Some(path) = req.extensions().get::<MatchedPath>() else {
        return unauthenticated(GatewayError::Unauthorized("no matching route".into()));
    };
    let required = required_scope(req.method(), path.as_str());
    let token = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let Some(token) = token else {
        return unauthenticated(GatewayError::Unauthorized("missing bearer token".into()));
    };
    let Some(scope) = token_store.validate(token) else {
        return unauthenticated(GatewayError::Unauthorized(
            "invalid or expired token".into(),
        ));
    };
    let ctx = AuthContext::with_client(scope, token.to_string());
    // Deliberately charge admitted requests before checking scope (research R3).
    let snapshot = match rate_limiter.admit(ctx.client_id()) {
        Ok(snapshot) => snapshot,
        Err(refusal) => {
            return (
                GatewayError::RateLimited {
                    retry_after: refusal.retry_after,
                    cause: refusal.cause,
                }
                .into_response(),
                refusal.snapshot,
            )
        }
    };
    if !ctx.has_scope(required) {
        return (
            GatewayError::Forbidden {
                required: format!("{required:?}"),
            }
            .into_response(),
            snapshot,
        );
    }
    (next.run(req).await, snapshot)
}

/// Protect a fully assembled router with the caller's limiter and the default
/// body bound. Apply after every route, including externally added /shutdown.
pub fn with_auth(
    router: Router,
    token_store: Arc<TokenStore>,
    rate_limiter: Arc<RateLimiter>,
) -> Router {
    assemble(
        router,
        token_store,
        rate_limiter,
        crate::server::DEFAULT_BODY_LIMIT,
    )
}

/// Assemble validated operator settings into the actual enforcement objects.
pub fn with_auth_configured(
    router: Router,
    token_store: Arc<TokenStore>,
    limits: crate::limits::GatewayLimits,
) -> Result<Router, crate::limits::InvalidGatewayLimits> {
    limits.validate()?;
    let limiter = RateLimiter::with_limits(limits.per_caller, limits.system_wide, limits.window)
        .map_err(|error| crate::limits::InvalidGatewayLimits(error.to_string()))?;
    Ok(assemble(
        router,
        token_store,
        Arc::new(limiter),
        limits.body_bytes,
    ))
}

fn assemble(
    router: Router,
    token_store: Arc<TokenStore>,
    rate_limiter: Arc<RateLimiter>,
    body_limit: usize,
) -> Router {
    crate::server::with_body_limit(router, body_limit).layer(middleware::from_fn(
        move |req: Request<Body>, next: Next| {
            let token_store = token_store.clone();
            let rate_limiter = rate_limiter.clone();
            async move { auth_middleware(token_store, rate_limiter, req, next, body_limit).await }
        },
    ))
}
