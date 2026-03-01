use std::time::Instant;

use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;
use tracing::info;

/// Auth middleware that checks Bearer token on all routes except /health and /metrics.
pub async fn auth_middleware(
    axum::extract::State(state): axum::extract::State<crate::state::AppState>,
    request: Request,
    next: Next,
) -> Response {
    // Check if auth is configured
    let token = match &state.config.server.auth_token {
        Some(t) => t,
        None => return next.run(request).await,
    };

    let path = request.uri().path();

    // Exempt health and metrics endpoints
    if path == "/health" || path == "/metrics" {
        return next.run(request).await;
    }

    // For WebSocket upgrades, check query param ?token=...
    if path.starts_with("/signal/") {
        if let Some(query) = request.uri().query() {
            let params: Vec<&str> = query.split('&').collect();
            for param in params {
                if let Some(val) = param.strip_prefix("token=") {
                    if val == token {
                        return next.run(request).await;
                    }
                }
            }
        }
        return Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(axum::body::Body::from(r#"{"error":"unauthorized"}"#))
            .expect("response builder should not fail for static content");
    }

    // Check Authorization: Bearer header
    if let Some(auth_header) = request.headers().get("authorization") {
        if let Ok(auth_str) = auth_header.to_str() {
            if let Some(bearer_token) = auth_str.strip_prefix("Bearer ") {
                if bearer_token == token {
                    return next.run(request).await;
                }
            }
        }
    }

    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .body(axum::body::Body::from(r#"{"error":"unauthorized"}"#))
        .expect("response builder should not fail for static content")
}

/// Request logging middleware.
pub async fn request_logger(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let uri = request.uri().clone();
    let start = Instant::now();

    let response = next.run(request).await;

    let duration = start.elapsed();
    let status = response.status();

    info!(
        %method,
        %uri,
        %status,
        duration_ms = duration.as_millis(),
        "request completed"
    );

    response
}
