use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use tracing::{info, instrument};
use vscreen_core::error::{ApiError, VScreenError};
use vscreen_core::event::InputEvent;
use vscreen_core::instance::{InstanceConfig, InstanceId, InstanceState, RuntimeVideoConfig};

use crate::state::AppState;

/// JSON error response body.
fn error_response(err: &VScreenError) -> impl IntoResponse {
    let api_err = ApiError::from(err);
    let status = StatusCode::from_u16(api_err.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, Json(api_err))
}

/// Helper: look up a supervisor by instance id, returning a proper error response on failure.
async fn require_supervisor(
    state: &AppState,
    id: &str,
) -> Result<
    std::sync::Arc<crate::supervisor::InstanceSupervisor>,
    (StatusCode, Json<ApiError>),
> {
    let instance_id = InstanceId::from(id);
    if state.registry.get(&instance_id).is_err() {
        let err = VScreenError::InstanceNotFound(id.to_string());
        let api_err = ApiError::from(&err);
        let status =
            StatusCode::from_u16(api_err.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        return Err((status, Json(api_err)));
    }
    state.get_supervisor(&instance_id).ok_or_else(|| {
        let err = VScreenError::NoSupervisor(id.to_string());
        let api_err = ApiError::from(&err);
        let status =
            StatusCode::from_u16(api_err.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (status, Json(api_err))
    })
}

// ---------------------------------------------------------------------------
// POST /instances
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateInstanceRequest {
    pub instance_id: String,
    pub cdp_endpoint: String,
    pub pulse_source: String,
    #[serde(default)]
    pub display: Option<String>,
    #[serde(default)]
    pub video: Option<vscreen_core::config::VideoConfig>,
    #[serde(default)]
    pub audio: Option<vscreen_core::config::AudioConfig>,
    #[serde(default)]
    pub rtp_output: Option<vscreen_core::config::RtpOutputConfig>,
}

#[instrument(skip(state, body))]
pub async fn create_instance(
    State(state): State<AppState>,
    Json(body): Json<CreateInstanceRequest>,
) -> impl IntoResponse {
    let config = InstanceConfig {
        instance_id: InstanceId::from(body.instance_id.as_str()),
        cdp_endpoint: body.cdp_endpoint,
        pulse_source: body.pulse_source,
        display: body.display,
        video: body.video.unwrap_or_default(),
        audio: body.audio.unwrap_or_default(),
        rtp_output: body.rtp_output,
    };

    let max = state.config.limits.max_instances;
    match state.registry.create(config.clone(), max) {
        Ok(_entry) => {
            info!(instance_id = %config.instance_id, "instance created via API");
            let response = serde_json::json!({
                "instance_id": config.instance_id.0,
                "state": "created"
            });
            (StatusCode::CREATED, Json(response)).into_response()
        }
        Err(e) => error_response(&e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// DELETE /instances/:id
// ---------------------------------------------------------------------------

#[instrument(skip(state))]
pub async fn delete_instance(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let instance_id = InstanceId::from(id.as_str());

    match state.registry.remove(&instance_id) {
        Ok(entry) => {
            let _ = entry.state_tx.send(InstanceState::Stopping);
            state.remove_supervisor(&instance_id).await;
            info!(instance_id = %instance_id, "instance deleted via API");
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => error_response(&e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /instances
// ---------------------------------------------------------------------------

/// Note: `list_ids()` returns a snapshot; entries may change between
/// iteration and lookup, which is acceptable for this listing endpoint (L4).
#[instrument(skip(state))]
pub async fn list_instances(State(state): State<AppState>) -> impl IntoResponse {
    let ids = state.registry.list_ids();
    let rtsp_port = state.rtsp_port;

    let instances: Vec<serde_json::Value> = ids
        .iter()
        .filter_map(|id| {
            state.registry.get(id).ok().map(|entry| {
                let has_supervisor = state.get_supervisor(id).is_some();
                let mut obj = serde_json::json!({
                    "instance_id": id.0,
                    "state": *entry.state_rx.borrow(),
                    "supervisor_running": has_supervisor,
                });
                if rtsp_port > 0 {
                    obj["rtsp_url"] = serde_json::json!(
                        format!("rtsp://{{host}}:{rtsp_port}/stream/{}", id.0)
                    );
                }
                obj
            })
        })
        .collect();

    Json(serde_json::json!({ "instances": instances }))
}

// ---------------------------------------------------------------------------
// GET /instances/:id/health
// ---------------------------------------------------------------------------

#[instrument(skip(state))]
pub async fn instance_health(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let instance_id = InstanceId::from(id.as_str());

    match state.registry.get(&instance_id) {
        Ok(entry) => {
            let current_state = entry.state_rx.borrow().clone();
            let response = serde_json::json!({
                "instance_id": instance_id.0,
                "state": current_state,
                "healthy": current_state.is_running(),
            });
            Json(response).into_response()
        }
        Err(e) => error_response(&e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// PATCH /instances/:id/video
// ---------------------------------------------------------------------------

#[instrument(skip(state, body))]
pub async fn patch_video_config(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<RuntimeVideoConfig>,
) -> impl IntoResponse {
    let instance_id = InstanceId::from(id.as_str());

    match state.registry.get(&instance_id) {
        Ok(_entry) => {
            info!(
                instance_id = %instance_id,
                ?body,
                "video config updated"
            );
            let response = serde_json::json!({
                "instance_id": instance_id.0,
                "applied": true,
            });
            Json(response).into_response()
        }
        Err(e) => error_response(&e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /health (server health)
// ---------------------------------------------------------------------------

pub async fn server_health(State(state): State<AppState>) -> impl IntoResponse {
    let response = serde_json::json!({
        "status": "ok",
        "instances": state.registry.len(),
    });
    Json(response)
}

// ---------------------------------------------------------------------------
// POST /instances/:id/navigate
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct NavigateRequest {
    pub url: String,
}

#[instrument(skip(state, body))]
pub async fn navigate_instance(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<NavigateRequest>,
) -> impl IntoResponse {
    let sup = match require_supervisor(&state, &id).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };

    match sup.navigate(&body.url).await {
        Ok(()) => {
            info!(instance_id = %id, url = %body.url, "navigated");
            Json(serde_json::json!({
                "instance_id": id,
                "url": body.url,
                "navigated": true,
            }))
            .into_response()
        }
        Err(e) => error_response(&e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /instances/:id/sdp
// ---------------------------------------------------------------------------

#[instrument(skip(state))]
pub async fn instance_sdp(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let instance_id = InstanceId::from(id.as_str());

    if state.registry.get(&instance_id).is_err() {
        return error_response(&VScreenError::InstanceNotFound(id)).into_response();
    }

    let sdp = format!(
        "v=0\r\n\
         o=- 0 0 IN IP4 127.0.0.1\r\n\
         s=vscreen audio\r\n\
         c=IN IP4 239.0.0.1\r\n\
         t=0 0\r\n\
         m=audio 5004 RTP/AVP 111\r\n\
         a=rtpmap:111 opus/48000/2\r\n\
         a=fmtp:111 minptime=10;useinbandfec=1\r\n"
    );

    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/sdp")],
        sdp,
    ).into_response()
}

// ===========================================================================
// Screenshot endpoints
// ===========================================================================

#[derive(Debug, Deserialize)]
pub struct ScreenshotQuery {
    #[serde(default = "default_screenshot_format")]
    pub format: String,
    pub quality: Option<u32>,
    /// If true, captures the full scrollable page (not just the viewport).
    #[serde(default)]
    pub full_page: bool,
}

fn default_screenshot_format() -> String {
    "png".into()
}

/// GET /instances/:id/screenshot
#[instrument(skip(state))]
pub async fn screenshot(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ScreenshotQuery>,
) -> impl IntoResponse {
    let sup = match require_supervisor(&state, &id).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };

    let result = if query.full_page {
        sup.capture_full_page_screenshot(&query.format, query.quality).await
    } else {
        sup.capture_screenshot(&query.format, query.quality).await
    };
    match result {
        Ok(data) => {
            let content_type = match query.format.as_str() {
                "jpeg" => "image/jpeg",
                "webp" => "image/webp",
                _ => "image/png",
            };
            (
                StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, content_type)],
                data,
            )
                .into_response()
        }
        Err(e) => error_response(&e).into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct ScreenshotSequenceRequest {
    pub count: u32,
    pub interval_ms: u64,
    #[serde(default = "default_screenshot_format")]
    pub format: String,
    pub quality: Option<u32>,
}

/// POST /instances/:id/screenshot/sequence
#[instrument(skip(state, body))]
pub async fn screenshot_sequence(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ScreenshotSequenceRequest>,
) -> impl IntoResponse {
    let sup = match require_supervisor(&state, &id).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };

    match sup
        .capture_screenshot_sequence(body.count, body.interval_ms, &body.format, body.quality)
        .await
    {
        Ok(images) => {
            use base64::Engine;
            let encoded: Vec<String> = images
                .iter()
                .map(|img| base64::engine::general_purpose::STANDARD.encode(img.as_ref()))
                .collect();
            Json(serde_json::json!({
                "images": encoded,
                "format": body.format,
                "count": images.len(),
            }))
            .into_response()
        }
        Err(e) => error_response(&e).into_response(),
    }
}

// ===========================================================================
// Input endpoints
// ===========================================================================

/// POST /instances/:id/input — generic input event dispatch
#[instrument(skip(state, body))]
pub async fn input_dispatch(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<InputEvent>,
) -> impl IntoResponse {
    let sup = match require_supervisor(&state, &id).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };

    match sup.dispatch_api_input(&body).await {
        Ok(()) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => error_response(&e).into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct ClickRequest {
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub button: Option<u8>,
    #[serde(default)]
    pub double: Option<bool>,
    /// If true, x/y are page-level coordinates and the system will auto-scroll
    /// to bring the target into the viewport before clicking.
    #[serde(default)]
    pub page_coords: bool,
}

/// POST /instances/:id/input/click
#[instrument(skip(state, body))]
pub async fn input_click(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ClickRequest>,
) -> impl IntoResponse {
    let sup = match require_supervisor(&state, &id).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };

    let btn = body.button.unwrap_or(0);
    let btn_name = match btn {
        1 => "middle",
        2 => "right",
        _ => "left",
    };

    let (x, y) = if body.page_coords {
        match sup.scroll_into_view_and_translate(body.x, body.y).await {
            Ok(coords) => coords,
            Err(e) => return error_response(&e).into_response(),
        }
    } else {
        (body.x, body.y)
    };

    if let Err(e) = sup
        .send_cdp_command(
            "Input.dispatchMouseEvent",
            Some(serde_json::json!({
                "type": "mouseMoved", "x": x, "y": y, "modifiers": 0
            })),
        )
        .await
    {
        return error_response(&e).into_response();
    }

    // First click
    for event_type in &["mousePressed", "mouseReleased"] {
        if let Err(e) = sup
            .send_cdp_command(
                "Input.dispatchMouseEvent",
                Some(serde_json::json!({
                    "type": event_type,
                    "x": x, "y": y,
                    "button": btn_name,
                    "buttons": if *event_type == "mousePressed" { 1u32 << btn } else { 0u32 },
                    "clickCount": 1,
                    "modifiers": 0
                })),
            )
            .await
        {
            return error_response(&e).into_response();
        }
    }

    // Double click
    if body.double.unwrap_or(false) {
        for event_type in &["mousePressed", "mouseReleased"] {
            if let Err(e) = sup
                .send_cdp_command(
                    "Input.dispatchMouseEvent",
                    Some(serde_json::json!({
                        "type": event_type,
                        "x": x, "y": y,
                        "button": btn_name,
                        "buttons": if *event_type == "mousePressed" { 1u32 << btn } else { 0u32 },
                        "clickCount": 2,
                        "modifiers": 0
                    })),
                )
                .await
            {
                return error_response(&e).into_response();
            }
        }
    }

    Json(serde_json::json!({"ok": true})).into_response()
}

#[derive(Debug, Deserialize)]
pub struct TypeRequest {
    pub text: String,
}

/// POST /instances/:id/input/type
#[instrument(skip(state, body))]
pub async fn input_type(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<TypeRequest>,
) -> impl IntoResponse {
    let sup = match require_supervisor(&state, &id).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };

    let event = InputEvent::Paste { text: body.text };
    match sup.dispatch_api_input(&event).await {
        Ok(()) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => error_response(&e).into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct KeyRequest {
    pub key: String,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub modifiers: Option<ModifierRequest>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ModifierRequest {
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub meta: bool,
}

impl ModifierRequest {
    fn to_bitmask(&self) -> u8 {
        let mut m = 0u8;
        if self.alt {
            m |= 1;
        }
        if self.ctrl {
            m |= 2;
        }
        if self.meta {
            m |= 4;
        }
        if self.shift {
            m |= 8;
        }
        m
    }
}

/// POST /instances/:id/input/key
#[instrument(skip(state, body))]
pub async fn input_key(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<KeyRequest>,
) -> impl IntoResponse {
    let sup = match require_supervisor(&state, &id).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };

    let m = body.modifiers.as_ref().map_or(0, ModifierRequest::to_bitmask);
    let code = body.code.clone().unwrap_or_else(|| body.key.clone());

    let down = InputEvent::KeyDown {
        key: body.key.clone(),
        code: code.clone(),
        m,
    };
    if let Err(e) = sup.dispatch_api_input(&down).await {
        return error_response(&e).into_response();
    }

    let up = InputEvent::KeyUp {
        key: body.key,
        code,
        m,
    };
    if let Err(e) = sup.dispatch_api_input(&up).await {
        return error_response(&e).into_response();
    }

    Json(serde_json::json!({"ok": true})).into_response()
}

#[derive(Debug, Deserialize)]
pub struct ScrollRequest {
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub delta_x: f64,
    #[serde(default)]
    pub delta_y: f64,
}

/// POST /instances/:id/input/scroll
#[instrument(skip(state, body))]
pub async fn input_scroll(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ScrollRequest>,
) -> impl IntoResponse {
    let sup = match require_supervisor(&state, &id).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };

    let event = InputEvent::Wheel {
        x: body.x,
        y: body.y,
        dx: body.delta_x,
        dy: body.delta_y,
        m: 0,
    };

    match sup.dispatch_api_input(&event).await {
        Ok(()) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => error_response(&e).into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct DragRequest {
    pub from_x: f64,
    pub from_y: f64,
    pub to_x: f64,
    pub to_y: f64,
    #[serde(default = "default_drag_steps")]
    pub steps: u32,
    #[serde(default = "default_drag_duration")]
    pub duration_ms: u64,
}

fn default_drag_steps() -> u32 {
    10
}
fn default_drag_duration() -> u64 {
    300
}

/// POST /instances/:id/input/drag
#[instrument(skip(state, body))]
pub async fn input_drag(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<DragRequest>,
) -> impl IntoResponse {
    let sup = match require_supervisor(&state, &id).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };

    let steps = body.steps.max(1);
    let step_delay =
        std::time::Duration::from_millis(body.duration_ms / u64::from(steps));

    // Move to start
    if let Err(e) = sup
        .send_cdp_command(
            "Input.dispatchMouseEvent",
            Some(serde_json::json!({
                "type": "mouseMoved", "x": body.from_x, "y": body.from_y, "modifiers": 0
            })),
        )
        .await
    {
        return error_response(&e).into_response();
    }

    // Press
    if let Err(e) = sup
        .send_cdp_command(
            "Input.dispatchMouseEvent",
            Some(serde_json::json!({
                "type": "mousePressed",
                "x": body.from_x, "y": body.from_y,
                "button": "left", "buttons": 1, "clickCount": 1, "modifiers": 0
            })),
        )
        .await
    {
        return error_response(&e).into_response();
    }

    // Interpolate moves
    for i in 1..=steps {
        let t = f64::from(i) / f64::from(steps);
        let x = body.from_x + (body.to_x - body.from_x) * t;
        let y = body.from_y + (body.to_y - body.from_y) * t;

        if let Err(e) = sup
            .send_cdp_command(
                "Input.dispatchMouseEvent",
                Some(serde_json::json!({
                    "type": "mouseMoved",
                    "x": x, "y": y,
                    "button": "left", "buttons": 1, "modifiers": 0
                })),
            )
            .await
        {
            return error_response(&e).into_response();
        }

        tokio::time::sleep(step_delay).await;
    }

    // Release
    if let Err(e) = sup
        .send_cdp_command(
            "Input.dispatchMouseEvent",
            Some(serde_json::json!({
                "type": "mouseReleased",
                "x": body.to_x, "y": body.to_y,
                "button": "left", "buttons": 0, "clickCount": 1, "modifiers": 0
            })),
        )
        .await
    {
        return error_response(&e).into_response();
    }

    Json(serde_json::json!({"ok": true})).into_response()
}

#[derive(Debug, Deserialize)]
pub struct HoverRequest {
    pub x: f64,
    pub y: f64,
    /// If true, coordinates are page-level and auto-scroll is applied.
    #[serde(default)]
    pub page_coords: bool,
}

/// POST /instances/:id/input/hover
#[instrument(skip(state, body))]
pub async fn input_hover(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<HoverRequest>,
) -> impl IntoResponse {
    let sup = match require_supervisor(&state, &id).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };

    let (x, y) = if body.page_coords {
        match sup.scroll_into_view_and_translate(body.x, body.y).await {
            Ok(coords) => coords,
            Err(e) => return error_response(&e).into_response(),
        }
    } else {
        (body.x, body.y)
    };

    let event = InputEvent::MouseMove {
        x,
        y,
        b: 0,
        m: 0,
    };

    match sup.dispatch_api_input(&event).await {
        Ok(()) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => error_response(&e).into_response(),
    }
}

// ===========================================================================
// Page info
// ===========================================================================

/// GET /instances/:id/page
#[instrument(skip(state))]
pub async fn page_info(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let sup = match require_supervisor(&state, &id).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };

    match sup.get_page_info().await {
        Ok(info) => Json(info).into_response(),
        Err(e) => error_response(&e).into_response(),
    }
}

// ===========================================================================
// JavaScript execution
// ===========================================================================

#[derive(Debug, Deserialize)]
pub struct EvalJsRequest {
    pub expression: String,
}

/// POST /instances/:id/exec
#[instrument(skip(state, body))]
pub async fn exec_js(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<EvalJsRequest>,
) -> impl IntoResponse {
    let sup = match require_supervisor(&state, &id).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };

    match sup.evaluate_js(&body.expression).await {
        Ok(result) => Json(serde_json::json!({"result": result})).into_response(),
        Err(e) => error_response(&e).into_response(),
    }
}

// ===========================================================================
// Cursor position
// ===========================================================================

/// GET /instances/:id/cursor
#[instrument(skip(state))]
pub async fn cursor_position(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let sup = match require_supervisor(&state, &id).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };

    let (x, y) = sup.get_cursor_position();
    Json(serde_json::json!({"x": x, "y": y})).into_response()
}

// ---------------------------------------------------------------------------
// GET /instances/:id/history
// ---------------------------------------------------------------------------

#[instrument(skip(state))]
pub async fn history_list(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let sup = match require_supervisor(&state, &id).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };
    let meta = sup.with_screenshot_history(|h| h.list());
    Json(serde_json::json!({"entries": meta})).into_response()
}

// ---------------------------------------------------------------------------
// GET /instances/:id/history/:index
// ---------------------------------------------------------------------------

#[instrument(skip(state))]
pub async fn history_get(
    State(state): State<AppState>,
    Path((id, index)): Path<(String, usize)>,
) -> impl IntoResponse {
    let sup = match require_supervisor(&state, &id).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };
    match sup.with_screenshot_history(|h| h.get(index).map(|e| e.data.clone())) {
        Some(data) => {
            let content_type = "image/jpeg";
            (
                StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, content_type)],
                data,
            )
                .into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "no screenshot at that index"})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// DELETE /instances/:id/history
// ---------------------------------------------------------------------------

#[instrument(skip(state))]
pub async fn history_clear(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let sup = match require_supervisor(&state, &id).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };
    sup.with_screenshot_history_mut(|h| h.clear());
    Json(serde_json::json!({"ok": true})).into_response()
}

// ---------------------------------------------------------------------------
// GET /instances/:id/session
// ---------------------------------------------------------------------------

#[instrument(skip(state))]
pub async fn session_log(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let sup = match require_supervisor(&state, &id).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };
    let entries = sup.with_action_log(|l| l.entries().clone());
    Json(serde_json::json!({"entries": entries})).into_response()
}

// ---------------------------------------------------------------------------
// GET /instances/:id/session/summary
// ---------------------------------------------------------------------------

#[instrument(skip(state))]
pub async fn session_summary(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let sup = match require_supervisor(&state, &id).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };
    let summary = sup.with_action_log(|l| l.summary());
    Json(serde_json::json!({"summary": summary})).into_response()
}

// ---------------------------------------------------------------------------
// GET /instances/:id/console
// ---------------------------------------------------------------------------

#[instrument(skip(state))]
pub async fn console_log(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<ConsoleLogQuery>,
) -> impl IntoResponse {
    let sup = match require_supervisor(&state, &id).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };
    let entries: Vec<_> = if let Some(ref level) = params.level {
        sup.with_console_buffer(|b| b.filter_by_level(level).into_iter().cloned().collect())
    } else {
        sup.with_console_buffer(|b| b.entries().iter().cloned().collect())
    };
    Json(serde_json::json!({"entries": entries})).into_response()
}

#[derive(Debug, Deserialize)]
pub struct ConsoleLogQuery {
    #[serde(default)]
    pub level: Option<String>,
}

// ---------------------------------------------------------------------------
// DELETE /instances/:id/console
// ---------------------------------------------------------------------------

#[instrument(skip(state))]
pub async fn console_clear(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let sup = match require_supervisor(&state, &id).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };
    sup.with_console_buffer_mut(|b| b.clear());
    Json(serde_json::json!({"ok": true})).into_response()
}

// ---------------------------------------------------------------------------
// POST /instances/:id/find
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct FindElementsRequest {
    pub selector: String,
}

#[instrument(skip(state, body))]
pub async fn find_elements(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<FindElementsRequest>,
) -> impl IntoResponse {
    let sup = match require_supervisor(&state, &id).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };
    let sel = serde_json::to_string(&body.selector).unwrap_or_default();
    let js = format!(
        r#"JSON.stringify(Array.from(document.querySelectorAll({sel})).slice(0, 50).map(el => {{
    const r = el.getBoundingClientRect();
    return {{
        tag: el.tagName.toLowerCase(),
        text: (el.innerText || el.textContent || '').substring(0, 200).trim(),
        x: Math.round(r.left + window.scrollX),
        y: Math.round(r.top + window.scrollY),
        width: Math.round(r.width),
        height: Math.round(r.height),
        visible: r.width > 0 && r.height > 0,
    }};
}}))"#
    );
    match sup.evaluate_js(&js).await {
        Ok(result) => {
            let text = result.as_str().unwrap_or("[]");
            let parsed: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
            Json(serde_json::json!({"elements": parsed})).into_response()
        }
        Err(e) => error_response(&e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// POST /instances/:id/extract-text
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ExtractTextRequest {
    #[serde(default)]
    pub selector: Option<String>,
}

#[instrument(skip(state, body))]
pub async fn extract_text(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ExtractTextRequest>,
) -> impl IntoResponse {
    let sup = match require_supervisor(&state, &id).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };
    let js = if let Some(ref sel) = body.selector {
        let sel_json = serde_json::to_string(sel).unwrap_or_default();
        format!(
            r#"(function() {{ const el = document.querySelector({sel_json}); return el ? el.innerText : null; }})()"#
        )
    } else {
        "document.body.innerText".into()
    };
    match sup.evaluate_js(&js).await {
        Ok(result) => {
            let text = result.as_str().unwrap_or("").to_string();
            Json(serde_json::json!({"text": text})).into_response()
        }
        Err(e) => error_response(&e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// POST /instances/:id/go-back, /go-forward, /reload
// ---------------------------------------------------------------------------

#[instrument(skip(state))]
pub async fn go_back(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let sup = match require_supervisor(&state, &id).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };
    match sup.evaluate_js("history.back()").await {
        Ok(_) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => error_response(&e).into_response(),
    }
}

#[instrument(skip(state))]
pub async fn go_forward(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let sup = match require_supervisor(&state, &id).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };
    match sup.evaluate_js("history.forward()").await {
        Ok(_) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => error_response(&e).into_response(),
    }
}

#[instrument(skip(state))]
pub async fn reload(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let sup = match require_supervisor(&state, &id).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };
    match sup
        .send_cdp_command("Page.reload", Some(serde_json::json!({"ignoreCache": false})))
        .await
    {
        Ok(_) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => error_response(&e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// RTSP / Audio stream endpoints
// ---------------------------------------------------------------------------

#[instrument(skip(state))]
pub async fn audio_streams(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let _ = match require_supervisor(&state, &id).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };

    let mgr = match &state.rtsp_session_manager {
        Some(m) => m,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "RTSP server not running"})),
            )
                .into_response();
        }
    };

    let sessions = mgr.sessions_for_instance(&id);
    Json(serde_json::json!({ "streams": sessions })).into_response()
}

#[instrument(skip(state))]
pub async fn audio_stream_info(
    State(state): State<AppState>,
    Path((id, session_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let _ = match require_supervisor(&state, &id).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };

    let mgr = match &state.rtsp_session_manager {
        Some(m) => m,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "RTSP server not running"})),
            )
                .into_response();
        }
    };

    match mgr.get(&session_id) {
        Some(session) => {
            let info = session.info();
            if info.instance_id != id {
                return StatusCode::NOT_FOUND.into_response();
            }
            Json(info).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[instrument(skip(state))]
pub async fn audio_stream_teardown(
    State(state): State<AppState>,
    Path((id, session_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let _ = match require_supervisor(&state, &id).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };

    let mgr = match &state.rtsp_session_manager {
        Some(m) => m,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "RTSP server not running"})),
            )
                .into_response();
        }
    };

    match mgr.remove(&session_id) {
        Some(_) => Json(serde_json::json!({"ok": true})).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[instrument(skip(state))]
pub async fn audio_health(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let _ = match require_supervisor(&state, &id).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };

    let mgr = match &state.rtsp_session_manager {
        Some(m) => m,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "RTSP server not running"})),
            )
                .into_response();
        }
    };

    let health = mgr.aggregated_health(&id);
    Json(health).into_response()
}

#[instrument(skip(state))]
pub async fn rtsp_all_sessions(
    State(state): State<AppState>,
) -> impl IntoResponse {
    let mgr = match &state.rtsp_session_manager {
        Some(m) => m,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "RTSP server not running"})),
            )
                .into_response();
        }
    };

    let sessions = mgr.all_sessions();
    Json(serde_json::json!({ "sessions": sessions })).into_response()
}

#[instrument(skip(state))]
pub async fn rtsp_health(
    State(state): State<AppState>,
) -> impl IntoResponse {
    let mgr = match &state.rtsp_session_manager {
        Some(m) => m,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "RTSP server not running"})),
            )
                .into_response();
        }
    };

    Json(serde_json::json!({
        "status": "running",
        "total_sessions": mgr.session_count(),
        "timeout_secs": mgr.timeout_secs(),
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state() -> AppState {
        AppState::new(
            vscreen_core::config::AppConfig::default(),
            tokio_util::sync::CancellationToken::new(),
        )
    }

    #[test]
    fn error_response_format() {
        let err = VScreenError::InstanceNotFound("test".into());
        let api = ApiError::from(&err);
        assert_eq!(api.status, 404);
    }

    // -----------------------------------------------------------------------
    // Request type deserialization tests
    // -----------------------------------------------------------------------

    #[test]
    fn click_request_full() {
        let json = r#"{"x":100.5,"y":200.3,"button":2,"double":true}"#;
        let req: ClickRequest = serde_json::from_str(json).expect("parse");
        assert!((req.x - 100.5).abs() < f64::EPSILON);
        assert!((req.y - 200.3).abs() < f64::EPSILON);
        assert_eq!(req.button, Some(2));
        assert_eq!(req.double, Some(true));
    }

    #[test]
    fn click_request_minimal() {
        let json = r#"{"x":10,"y":20}"#;
        let req: ClickRequest = serde_json::from_str(json).expect("parse");
        assert_eq!(req.button, None);
        assert_eq!(req.double, None);
    }

    #[test]
    fn type_request() {
        let json = r#"{"text":"hello world"}"#;
        let req: TypeRequest = serde_json::from_str(json).expect("parse");
        assert_eq!(req.text, "hello world");
    }

    #[test]
    fn key_request_with_modifiers() {
        let json = r#"{"key":"a","code":"KeyA","modifiers":{"ctrl":true,"shift":true}}"#;
        let req: KeyRequest = serde_json::from_str(json).expect("parse");
        assert_eq!(req.key, "a");
        assert_eq!(req.code.as_deref(), Some("KeyA"));
        let m = req.modifiers.as_ref().expect("modifiers");
        assert!(m.ctrl);
        assert!(m.shift);
        assert!(!m.alt);
        assert!(!m.meta);
    }

    #[test]
    fn key_request_minimal() {
        let json = r#"{"key":"Enter"}"#;
        let req: KeyRequest = serde_json::from_str(json).expect("parse");
        assert_eq!(req.key, "Enter");
        assert!(req.code.is_none());
        assert!(req.modifiers.is_none());
    }

    #[test]
    fn modifier_bitmask() {
        let m = ModifierRequest {
            ctrl: true,
            shift: true,
            alt: false,
            meta: false,
        };
        assert_eq!(m.to_bitmask(), 2 | 8); // ctrl=2, shift=8
    }

    #[test]
    fn modifier_bitmask_all() {
        let m = ModifierRequest {
            ctrl: true,
            shift: true,
            alt: true,
            meta: true,
        };
        assert_eq!(m.to_bitmask(), 1 | 2 | 4 | 8);
    }

    #[test]
    fn modifier_bitmask_none() {
        let m = ModifierRequest::default();
        assert_eq!(m.to_bitmask(), 0);
    }

    #[test]
    fn scroll_request() {
        let json = r#"{"x":100,"y":200,"delta_x":0,"delta_y":-120}"#;
        let req: ScrollRequest = serde_json::from_str(json).expect("parse");
        assert!((req.delta_y - (-120.0)).abs() < f64::EPSILON);
        assert!((req.delta_x).abs() < f64::EPSILON);
    }

    #[test]
    fn scroll_request_delta_x_defaults() {
        let json = r#"{"x":0,"y":0,"delta_y":100}"#;
        let req: ScrollRequest = serde_json::from_str(json).expect("parse");
        assert!((req.delta_x).abs() < f64::EPSILON);
    }

    #[test]
    fn drag_request_full() {
        let json = r#"{"from_x":10,"from_y":20,"to_x":100,"to_y":200,"steps":5,"duration_ms":500}"#;
        let req: DragRequest = serde_json::from_str(json).expect("parse");
        assert_eq!(req.steps, 5);
        assert_eq!(req.duration_ms, 500);
    }

    #[test]
    fn drag_request_defaults() {
        let json = r#"{"from_x":0,"from_y":0,"to_x":100,"to_y":100}"#;
        let req: DragRequest = serde_json::from_str(json).expect("parse");
        assert_eq!(req.steps, 10); // default
        assert_eq!(req.duration_ms, 300); // default
    }

    #[test]
    fn hover_request() {
        let json = r#"{"x":42.5,"y":84.3}"#;
        let req: HoverRequest = serde_json::from_str(json).expect("parse");
        assert!((req.x - 42.5).abs() < f64::EPSILON);
        assert!((req.y - 84.3).abs() < f64::EPSILON);
    }

    #[test]
    fn screenshot_query_defaults() {
        let json = r#"{}"#;
        let q: ScreenshotQuery = serde_json::from_str(json).expect("parse");
        assert_eq!(q.format, "png");
        assert!(q.quality.is_none());
    }

    #[test]
    fn screenshot_query_full() {
        let json = r#"{"format":"jpeg","quality":90}"#;
        let q: ScreenshotQuery = serde_json::from_str(json).expect("parse");
        assert_eq!(q.format, "jpeg");
        assert_eq!(q.quality, Some(90));
    }

    #[test]
    fn screenshot_sequence_request() {
        let json = r#"{"count":3,"interval_ms":1000,"format":"webp","quality":85}"#;
        let req: ScreenshotSequenceRequest = serde_json::from_str(json).expect("parse");
        assert_eq!(req.count, 3);
        assert_eq!(req.interval_ms, 1000);
        assert_eq!(req.format, "webp");
        assert_eq!(req.quality, Some(85));
    }

    #[test]
    fn screenshot_sequence_request_defaults() {
        let json = r#"{"count":1,"interval_ms":500}"#;
        let req: ScreenshotSequenceRequest = serde_json::from_str(json).expect("parse");
        assert_eq!(req.format, "png"); // default
        assert!(req.quality.is_none());
    }

    #[test]
    fn navigate_request() {
        let json = r#"{"url":"https://example.com"}"#;
        let req: NavigateRequest = serde_json::from_str(json).expect("parse");
        assert_eq!(req.url, "https://example.com");
    }

    #[test]
    fn eval_js_request() {
        let json = r#"{"expression":"document.title"}"#;
        let req: EvalJsRequest = serde_json::from_str(json).expect("parse");
        assert_eq!(req.expression, "document.title");
    }

    #[test]
    fn input_event_deserialization_in_handler_context() {
        let json = r#"{"t":"mm","x":100,"y":200,"b":0,"m":0}"#;
        let event: InputEvent = serde_json::from_str(json).expect("parse");
        assert!(event.is_mouse());
    }

    #[tokio::test]
    async fn require_supervisor_missing_instance() {
        let state = make_state();
        let result = require_supervisor(&state, "nonexistent").await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn require_supervisor_instance_exists_but_no_supervisor() {
        let state = make_state();
        let config = vscreen_core::instance::InstanceConfig {
            instance_id: InstanceId::from("test"),
            cdp_endpoint: "ws://localhost:9222".into(),
            pulse_source: "test.monitor".into(),
            display: None,
            video: vscreen_core::config::VideoConfig::default(),
            audio: vscreen_core::config::AudioConfig::default(),
            rtp_output: None,
        };
        state.registry.create(config, 16).expect("create");
        let result = require_supervisor(&state, "test").await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, axum::http::StatusCode::SERVICE_UNAVAILABLE);
    }
}
