// Server setup — build_router, listener binding, graceful shutdown.

use crate::backend::GatewayBackend;
use crate::routes;
use axum::routing::{delete, get, post};
use axum::Router;
use std::sync::Arc;

/// Preserve axum's existing JSON ceiling while enforcing it for every route.
pub const DEFAULT_BODY_LIMIT: usize = 2 * 1024 * 1024;

/// Apply after all routes are assembled and inside authentication. The bound
/// is checked before buffering a declared oversized body and at each chunk
/// for bodies whose length is absent or understated.
pub fn with_body_limit(router: Router, limit: usize) -> Router {
    router
        .layer(axum::extract::DefaultBodyLimit::max(limit))
        .layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| async move {
                use axum::body::Body;
                use axum::http::{header::CONTENT_LENGTH, StatusCode};
                use axum::response::IntoResponse;
                use tokio_stream::StreamExt;

                let refusal =
                    || {
                        (StatusCode::PAYLOAD_TOO_LARGE, axum::Json(serde_json::json!({
                        "ok": false,
                        "error": {"code": "payload_too_large",
                            "message": format!("request body exceeds the {limit} byte limit")}
                    }))).into_response()
                    };
                if let Some(length) = request.headers().get(CONTENT_LENGTH) {
                    match length
                        .to_str()
                        .ok()
                        .and_then(|value| value.parse::<u64>().ok())
                    {
                        Some(length) if length > limit as u64 => return refusal(),
                        Some(_) => {}
                        None => {
                            return crate::error::GatewayError::BadRequest(
                                "invalid Content-Length".into(),
                            )
                            .into_response()
                        }
                    }
                }
                let (parts, body) = request.into_parts();
                let mut stream = body.into_data_stream();
                let mut buffered = Vec::new();
                while let Some(chunk) = stream.next().await {
                    let chunk = match chunk {
                        Ok(chunk) => chunk,
                        Err(error) => {
                            return crate::error::GatewayError::BadRequest(format!(
                                "could not read request body: {error}"
                            ))
                            .into_response()
                        }
                    };
                    if chunk.len() > limit.saturating_sub(buffered.len()) {
                        return refusal();
                    }
                    buffered.extend_from_slice(&chunk);
                }
                next.run(axum::http::Request::from_parts(parts, Body::from(buffered)))
                    .await
            },
        ))
}

/// Build the axum Router with all routes and middleware.
pub fn build_router(backend: Arc<dyn GatewayBackend>) -> Router {
    Router::new()
        .route(
            "/isolation/capabilities",
            get(routes::sessions::isolation_capabilities),
        )
        .route("/health", get(routes::health::health))
        .route(
            "/images",
            get(routes::images::list).post(routes::images::provision),
        )
        .route(
            "/images/{id}",
            get(routes::images::inspect).delete(routes::images::remove),
        )
        .route("/sessions", get(routes::sessions::list))
        .route("/sessions", post(routes::sessions::create))
        .route("/sessions/{id}", get(routes::sessions::get))
        .route("/sessions/{id}", delete(routes::sessions::destroy))
        .route("/sessions/{id}/exec", post(routes::sessions::exec))
        .route("/sessions/{id}/send", post(routes::sessions::send_input))
        .route("/sessions/{id}/eof", post(routes::sessions::end_input))
        .route(
            "/sessions/{id}/authority",
            get(routes::sessions::input_authority),
        )
        .route("/sessions/{id}/output", get(routes::sessions::output))
        .route(
            "/sessions/{id}/output/text",
            get(routes::sessions::output_text),
        )
        .route(
            "/sessions/{id}/output/stream",
            get(routes::sessions::output_stream),
        )
        .route("/sessions/{id}/history", get(routes::sessions::history))
        .route("/sessions/{id}/events", get(routes::sessions::events))
        .route("/sessions/{id}/panes", get(routes::panes::list))
        .route("/sessions/{id}/panes/split", post(routes::panes::split))
        .route(
            "/sessions/{id}/panes/{pane_id}",
            delete(routes::panes::close),
        )
        .with_state(backend)
}
