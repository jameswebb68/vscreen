use axum::middleware;
use axum::routing::{delete, get, patch, post};
use axum::Router;

use crate::handlers;
use crate::middleware::request_logger;
use crate::state::AppState;
use crate::ws;

async fn metrics_handler() -> impl axum::response::IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        crate::metrics::render(),
    )
}

/// Build the axum router with all routes.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        // Server-level
        .route("/health", get(handlers::server_health))
        .route("/metrics", get(metrics_handler))
        // Instance CRUD
        .route("/instances", post(handlers::create_instance))
        .route("/instances", get(handlers::list_instances))
        .route("/instances/{id}", delete(handlers::delete_instance))
        .route("/instances/{id}/health", get(handlers::instance_health))
        .route("/instances/{id}/video", patch(handlers::patch_video_config))
        .route("/instances/{id}/navigate", post(handlers::navigate_instance))
        .route("/instances/{id}/sdp", get(handlers::instance_sdp))
        // Screenshot
        .route("/instances/{id}/screenshot", get(handlers::screenshot))
        .route("/instances/{id}/screenshot/sequence", post(handlers::screenshot_sequence))
        // Input
        .route("/instances/{id}/input", post(handlers::input_dispatch))
        .route("/instances/{id}/input/click", post(handlers::input_click))
        .route("/instances/{id}/input/type", post(handlers::input_type))
        .route("/instances/{id}/input/key", post(handlers::input_key))
        .route("/instances/{id}/input/scroll", post(handlers::input_scroll))
        .route("/instances/{id}/input/drag", post(handlers::input_drag))
        .route("/instances/{id}/input/hover", post(handlers::input_hover))
        // Page introspection
        .route("/instances/{id}/page", get(handlers::page_info))
        .route("/instances/{id}/exec", post(handlers::exec_js))
        .route("/instances/{id}/cursor", get(handlers::cursor_position))
        // Screenshot history
        .route("/instances/{id}/history", get(handlers::history_list))
        .route("/instances/{id}/history", delete(handlers::history_clear))
        .route("/instances/{id}/history/{index}", get(handlers::history_get))
        // Session log
        .route("/instances/{id}/session", get(handlers::session_log))
        .route("/instances/{id}/session/summary", get(handlers::session_summary))
        // Console capture
        .route("/instances/{id}/console", get(handlers::console_log))
        .route("/instances/{id}/console", delete(handlers::console_clear))
        // Element discovery & text extraction
        .route("/instances/{id}/find", post(handlers::find_elements))
        .route("/instances/{id}/extract-text", post(handlers::extract_text))
        // Navigation
        .route("/instances/{id}/go-back", post(handlers::go_back))
        .route("/instances/{id}/go-forward", post(handlers::go_forward))
        .route("/instances/{id}/reload", post(handlers::reload))
        // Audio / RTSP
        .route("/instances/{id}/audio/streams", get(handlers::audio_streams))
        .route("/instances/{id}/audio/streams/{session_id}", get(handlers::audio_stream_info))
        .route("/instances/{id}/audio/streams/{session_id}", delete(handlers::audio_stream_teardown))
        .route("/instances/{id}/audio/health", get(handlers::audio_health))
        .route("/rtsp/sessions", get(handlers::rtsp_all_sessions))
        .route("/rtsp/health", get(handlers::rtsp_health))
        // WebSocket signaling
        .route("/signal/{instance_id}", get(ws::ws_signal))
        // Middleware
        .layer(middleware::from_fn(request_logger))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::auth_middleware,
        ))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn make_state() -> AppState {
        let mut config = vscreen_core::config::AppConfig::default();
        config.server.auth_token = None;
        AppState::new(config, tokio_util::sync::CancellationToken::new())
    }

    #[test]
    fn router_builds() {
        let state = make_state();
        let _router = build_router(state);
    }

    #[tokio::test]
    async fn health_endpoint() {
        let state = make_state();
        let router = build_router(state);

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn list_instances_empty() {
        let state = make_state();
        let router = build_router(state);

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/instances")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn delete_nonexistent_returns_404() {
        let state = make_state();
        let router = build_router(state);

        let response = router
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/instances/nope")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn screenshot_nonexistent_returns_404() {
        let state = make_state();
        let router = build_router(state);

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/instances/nope/screenshot")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn cursor_nonexistent_returns_404() {
        let state = make_state();
        let router = build_router(state);

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/instances/nope/cursor")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn page_nonexistent_returns_404() {
        let state = make_state();
        let router = build_router(state);

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/instances/nope/page")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn click_nonexistent_returns_404() {
        let state = make_state();
        let router = build_router(state);

        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/instances/nope/input/click")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"x":0,"y":0}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn instance_registered_but_no_supervisor_returns_503() {
        let state = make_state();
        let config = vscreen_core::instance::InstanceConfig {
            instance_id: vscreen_core::instance::InstanceId::from("test"),
            cdp_endpoint: "ws://localhost:9222".into(),
            pulse_source: "test.monitor".into(),
            display: None,
            video: vscreen_core::config::VideoConfig::default(),
            audio: vscreen_core::config::AudioConfig::default(),
            rtp_output: None,
        };
        state.registry.create(config, 16).expect("create");

        let router = build_router(state);

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/instances/test/screenshot")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
    }
}
