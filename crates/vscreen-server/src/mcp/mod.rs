use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use rmcp::handler::server::tool::{ToolCallContext, ToolRouter};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::service::RequestContext;
use rmcp::{tool, tool_router, RoleServer, ServerHandler, ServiceExt};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};
use vscreen_core::instance::{InstanceId, LockToken, LockType, SessionId};

use crate::lock_manager::{InstanceLockManager, LockError};
use crate::state::AppState;
use crate::supervisor::InstanceSupervisor;

// ---------------------------------------------------------------------------
// Session guard — releases locks when the MCP session is dropped
// ---------------------------------------------------------------------------

struct SessionGuard {
    session_id: SessionId,
    lock_manager: Arc<InstanceLockManager>,
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        self.lock_manager.unregister_session(&self.session_id);
    }
}

type McpError = rmcp::ErrorData;

fn internal_error(msg: impl Into<String>) -> McpError {
    McpError {
        code: rmcp::model::ErrorCode::INTERNAL_ERROR,
        message: std::borrow::Cow::Owned(msg.into()),
        data: None,
    }
}

fn invalid_params(msg: impl Into<String>) -> McpError {
    McpError {
        code: rmcp::model::ErrorCode::INVALID_PARAMS,
        message: std::borrow::Cow::Owned(msg.into()),
        data: None,
    }
}

pub(crate) mod advisor;
use advisor::*;

mod captcha;
mod docs;
use docs::*;
mod interaction;
mod navigation;
mod observation;
mod session;
mod synthesis;
mod workflow;
mod workflow_interact;

// ---------------------------------------------------------------------------
// MCP Server
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct VScreenMcpServer {
    state: AppState,
    session_id: SessionId,
    tool_router: ToolRouter<Self>,
    advisor: Arc<Mutex<ToolAdvisor>>,
    _session_guard: Arc<SessionGuard>,
}

impl std::fmt::Debug for VScreenMcpServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VScreenMcpServer").finish_non_exhaustive()
    }
}

impl VScreenMcpServer {
    #[must_use]
    pub fn new(state: AppState) -> Self {
        let session_id = SessionId::new();
        info!(session_id = %session_id, "MCP session created");
        state.lock_manager.register_session(&session_id);
        let guard = Arc::new(SessionGuard {
            session_id: session_id.clone(),
            lock_manager: Arc::clone(&state.lock_manager),
        });
        Self {
            state,
            session_id,
            tool_router: Self::core_tools()
                + Self::navigation_tools()
                + Self::interaction_tools()
                + Self::observation_tools()
                + Self::session_tools()
                + Self::synthesis_tools()
                + Self::workflow_tools()
                + Self::workflow_interact_tools(),
            advisor: Arc::new(Mutex::new(ToolAdvisor::new())),
            _session_guard: guard,
        }
    }

    #[must_use]
    pub fn with_session_id(state: AppState, session_id: SessionId) -> Self {
        info!(session_id = %session_id, "MCP session created");
        state.lock_manager.register_session(&session_id);
        let guard = Arc::new(SessionGuard {
            session_id: session_id.clone(),
            lock_manager: Arc::clone(&state.lock_manager),
        });
        Self {
            state,
            session_id,
            tool_router: Self::core_tools()
                + Self::navigation_tools()
                + Self::interaction_tools()
                + Self::observation_tools()
                + Self::session_tools()
                + Self::synthesis_tools()
                + Self::workflow_tools()
                + Self::workflow_interact_tools(),
            advisor: Arc::new(Mutex::new(ToolAdvisor::new())),
            _session_guard: guard,
        }
    }

    fn get_supervisor(
        &self,
        instance_id: &str,
    ) -> Result<Arc<InstanceSupervisor>, McpError> {
        let id = InstanceId::from(instance_id);
        self.state
            .get_supervisor(&id)
            .ok_or_else(|| invalid_params(format!("no supervisor for instance: {instance_id}")))
    }

    /// Record an action in the session log and auto-capture a screenshot into history.
    async fn record_action(
        &self,
        instance_id: &str,
        tool_name: &str,
        params_summary: &str,
        result_summary: &str,
    ) {
        if let Ok(sup_arc) = self.get_supervisor(instance_id) {
            let url = sup_arc
                .evaluate_js("location.href")
                .await
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default();

            // Auto-capture a screenshot for the history ring buffer (JPEG, low quality for compactness)
            let screenshot_index = match sup_arc.capture_screenshot("jpeg", Some(40)).await {
                Ok(data) => sup_arc.with_screenshot_history_mut(|hist| {
                    hist.push(
                        data,
                        url.clone(),
                        format!("{tool_name}({params_summary})"),
                        0.0,
                        940,
                        false,
                    );
                    Some(hist.len().saturating_sub(1))
                }),
                Err(_) => None,
            };

            sup_arc.with_action_log_mut(|log| {
                log.record(
                    tool_name.to_string(),
                    params_summary.to_string(),
                    result_summary.to_string(),
                    url.clone(),
                    url,
                    screenshot_index,
                );
            });
        }
    }

    /// Capture a screenshot and run vision verification if configured.
    /// Returns a short description of what the vision model observed, or None.
    async fn vision_verify_action(
        &self,
        sup: &Arc<InstanceSupervisor>,
        before_screenshot: Option<&[u8]>,
        prompt: &str,
    ) -> Option<String> {
        let vision = self.state.vision_client.as_ref()?;
        if !vision.is_available().await {
            return None;
        }
        let after_screenshot = sup.capture_vision_screenshot().await.ok()?;

        let full_prompt = if before_screenshot.is_some() {
            format!("{prompt}\n\nNote: This is the AFTER screenshot. The action has already been performed.")
        } else {
            prompt.to_string()
        };

        match vision.analyze(&after_screenshot, &full_prompt).await {
            Ok(resp) => {
                let summary = resp.text.chars().take(300).collect::<String>();
                if summary.is_empty() { None } else { Some(summary) }
            }
            Err(e) => {
                debug!("vision verification error: {e}");
                None
            }
        }
    }

    fn require_exclusive(&self, instance_id: &str) -> Result<(), McpError> {
        self.require_lock(instance_id, LockType::Exclusive)
    }

    fn require_observer_or_exclusive(&self, instance_id: &str) -> Result<(), McpError> {
        self.require_lock(instance_id, LockType::Observer)
    }

    /// Core lock-check with auto-lock and dead-session reclaim.
    ///
    /// 1. Check if this session already holds a sufficient lock → pass.
    /// 2. If no lock exists (`NotHeld`) → auto-acquire for this session.
    /// 3. If another session holds it → check if that session is dead
    ///    or auto-acquired, then reclaim and auto-acquire for this session.
    fn require_lock(&self, instance_id: &str, required: LockType) -> Result<(), McpError> {
        if self.state.single_agent_mode {
            return Ok(());
        }
        let id = InstanceId::from(instance_id);
        match self.state.lock_manager.check_access(&id, &self.session_id, required) {
            Ok(()) => return Ok(()),
            Err(LockError::NotHeld { .. }) => {
                if self.auto_acquire(instance_id, required).is_ok() {
                    return Ok(());
                }
                // auto_acquire may fail if observers/exclusive from other
                // sessions block it; fall through to reclaim logic below.
            }
            Err(LockError::InstanceLocked { .. }) => {
                // Fall through to reclaim logic below.
            }
            Err(ref _e) => {}
        }

        // Reclaim: release locks held by dead sessions or auto-acquired locks.
        let should_reclaim = self.state.lock_manager.is_auto_acquired(&id);
        if should_reclaim {
            // Release all auto-acquired blockers (exclusive + observers).
            let status = self.state.lock_manager.status(&id);
            if let Some(ref exc) = status.exclusive_holder {
                if exc.auto_acquired {
                    debug!(instance_id, holder = %exc.session_id, "reclaiming auto-acquired exclusive lock");
                    let _ = self.state.lock_manager.release(&id, &exc.session_id);
                }
            }
            for obs in &status.observers {
                if obs.auto_acquired || !self.state.lock_manager.is_session_active(&obs.session_id) {
                    debug!(instance_id, holder = %obs.session_id, "reclaiming auto-acquired/dead observer lock");
                    let _ = self.state.lock_manager.release(&id, &obs.session_id);
                }
            }
            if self.auto_acquire(instance_id, required).is_ok() {
                return Ok(());
            }
        }

        // Re-run to get the current error for the error message
        self.state
            .lock_manager
            .check_access(&id, &self.session_id, required)
            .map_err(|e| lock_error_to_mcp(instance_id, e))
    }

    fn auto_acquire(&self, instance_id: &str, lock_type: LockType) -> Result<(), McpError> {
        let id = InstanceId::from(instance_id);
        let ttl = Duration::from_secs(120);
        match self.state.lock_manager.acquire_auto(
            &id,
            &self.session_id,
            lock_type,
            ttl,
        ) {
            Ok(info) => {
                debug!(
                    instance_id,
                    session = %self.session_id,
                    lock_type = %lock_type,
                    token = %info.lock_token,
                    "auto-acquired lock"
                );
                Ok(())
            }
            Err(e) => Err(lock_error_to_mcp(instance_id, e)),
        }
    }

    fn parse_lock_type(s: &str) -> Result<LockType, McpError> {
        match s {
            "exclusive" => Ok(LockType::Exclusive),
            "observer" => Ok(LockType::Observer),
            _ => Err(invalid_params(format!(
                "invalid lock_type '{s}': must be 'exclusive' or 'observer'"
            ))),
        }
    }

    /// Dispatch a CDP mouse click at the given page-space coordinates.
    async fn cdp_click(
        &self,
        sup: &Arc<InstanceSupervisor>,
        page_x: f64,
        page_y: f64,
    ) -> Result<(), McpError> {
        let (vx, vy) = sup
            .scroll_into_view_and_translate(page_x, page_y)
            .await
            .map_err(|e| internal_error(e.to_string()))?;
        for (typ, buttons) in [
            ("mouseMoved", 0u32),
            ("mousePressed", 1u32),
            ("mouseReleased", 0u32),
        ] {
            let mut p = serde_json::json!({"type": typ, "x": vx, "y": vy, "modifiers": 0});
            if typ != "mouseMoved" {
                p["button"] = serde_json::json!("left");
                p["buttons"] = serde_json::json!(buttons);
                p["clickCount"] = serde_json::json!(1);
            }
            sup.send_cdp_command("Input.dispatchMouseEvent", Some(p))
                .await
                .map_err(|e| internal_error(e.to_string()))?;
        }
        Ok(())
    }

    /// Recursively find elements matching a CSS selector inside child frames.
    /// Returns elements with coordinates adjusted by the iframe's offset in the parent.
    async fn find_elements_in_frames(
        &self,
        sup: &std::sync::Arc<crate::supervisor::InstanceSupervisor>,
        frame_tree: &serde_json::Value,
        sel_json: &str,
    ) -> Vec<serde_json::Value> {
        let mut results = Vec::new();
        let child_frames = frame_tree
            .get("frameTree")
            .or_else(|| Some(frame_tree))
            .and_then(|ft| ft.get("childFrames"))
            .and_then(|cf| cf.as_array());

        if let Some(frames) = child_frames {
            for child in frames {
                let frame_id = child
                    .get("frame")
                    .and_then(|f| f.get("id"))
                    .and_then(|id| id.as_str())
                    .unwrap_or("");
                if frame_id.is_empty() { continue; }

                // Get iframe bounding rect from parent (for coordinate adjustment)
                let frame_url = child.get("frame").and_then(|f| f.get("url")).and_then(|u| u.as_str()).unwrap_or("");
                let rect_js = format!(
                    r#"(function(){{const f=document.querySelector('iframe[src*="{}"]')||document.querySelectorAll('iframe')[0];if(!f)return null;const r=f.getBoundingClientRect();return JSON.stringify({{x:r.left+window.scrollX,y:r.top+window.scrollY}})}})()"#,
                    frame_url.replace('"', "")
                );
                let offset = if let Ok(r) = sup.evaluate_js(&rect_js).await {
                    let s = r.as_str().unwrap_or("{}");
                    let o: serde_json::Value = serde_json::from_str(s).unwrap_or_default();
                    (
                        o.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0),
                        o.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    )
                } else { (0.0, 0.0) };

                let js = format!(
                    r#"JSON.stringify(Array.from(document.querySelectorAll({sel})).slice(0, 30).map(el => {{
    const r = el.getBoundingClientRect();
    return {{
        tag: el.tagName.toLowerCase(),
        text: (el.innerText || el.textContent || '').substring(0, 200).trim(),
        x: Math.round(r.left + r.width / 2 + window.scrollX),
        y: Math.round(r.top + r.height / 2 + window.scrollY),
        width: Math.round(r.width),
        height: Math.round(r.height),
        visible: r.width > 0 && r.height > 0 && r.top > -9000 && r.left > -9000,
        frame_id: "{frame_id}",
    }};
}}))"#,
                    sel = sel_json,
                    frame_id = frame_id
                );

                if let Ok(r) = sup.evaluate_js_in_frame(&js, frame_id).await {
                    let s = r.as_str().unwrap_or("[]");
                    if let Ok(mut els) = serde_json::from_str::<Vec<serde_json::Value>>(s) {
                        for el in &mut els {
                            if let Some(obj) = el.as_object_mut() {
                                if let Some(x) = obj.get("x").and_then(|v| v.as_f64()) {
                                    obj.insert("x".into(), serde_json::json!(x + offset.0));
                                }
                                if let Some(y) = obj.get("y").and_then(|v| v.as_f64()) {
                                    obj.insert("y".into(), serde_json::json!(y + offset.1));
                                }
                            }
                        }
                        results.extend(els);
                    }
                }
            }
        }
        results
    }

    /// Recursively find elements matching text inside child frames.
    async fn find_text_in_frames(
        &self,
        sup: &std::sync::Arc<crate::supervisor::InstanceSupervisor>,
        frame_tree: &serde_json::Value,
        search_json: &str,
        exact: bool,
    ) -> Vec<serde_json::Value> {
        let mut results = Vec::new();
        let child_frames = frame_tree
            .get("frameTree")
            .or_else(|| Some(frame_tree))
            .and_then(|ft| ft.get("childFrames"))
            .and_then(|cf| cf.as_array());

        if let Some(frames) = child_frames {
            for child in frames {
                let frame_id = child
                    .get("frame")
                    .and_then(|f| f.get("id"))
                    .and_then(|id| id.as_str())
                    .unwrap_or("");
                if frame_id.is_empty() { continue; }

                let frame_url = child.get("frame").and_then(|f| f.get("url")).and_then(|u| u.as_str()).unwrap_or("");
                let rect_js = format!(
                    r#"(function(){{const f=document.querySelector('iframe[src*="{}"]')||document.querySelectorAll('iframe')[0];if(!f)return null;const r=f.getBoundingClientRect();return JSON.stringify({{x:r.left+window.scrollX,y:r.top+window.scrollY}})}})()"#,
                    frame_url.replace('"', "")
                );
                let offset = if let Ok(r) = sup.evaluate_js(&rect_js).await {
                    let s = r.as_str().unwrap_or("{}");
                    let o: serde_json::Value = serde_json::from_str(s).unwrap_or_default();
                    (
                        o.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0),
                        o.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    )
                } else { (0.0, 0.0) };

                let match_fn = if exact {
                    format!("(el.innerText || '').trim() === {search_json}")
                } else {
                    format!("(el.innerText || '').toLowerCase().includes({search_json}.toLowerCase())")
                };
                let js = format!(
                    r#"JSON.stringify((function() {{
    const results = [];
    const walker = document.createTreeWalker(document.body || document.documentElement, NodeFilter.SHOW_ELEMENT);
    let el;
    while ((el = walker.nextNode()) && results.length < 20) {{
        if ({match_fn}) {{
            const r = el.getBoundingClientRect();
            if (r.width > 0 && r.height > 0 && r.top > -9000 && r.left > -9000) {{
                results.push({{
                    tag: el.tagName.toLowerCase(),
                    text: (el.innerText || '').substring(0, 200).trim(),
                    x: Math.round(r.left + r.width / 2 + window.scrollX),
                    y: Math.round(r.top + r.height / 2 + window.scrollY),
                    width: Math.round(r.width),
                    height: Math.round(r.height),
                    frame_id: "{frame_id}",
                }});
            }}
        }}
    }}
    return results;
}})())"#,
                    match_fn = match_fn,
                    frame_id = frame_id
                );

                if let Ok(r) = sup.evaluate_js_in_frame(&js, frame_id).await {
                    let s = r.as_str().unwrap_or("[]");
                    if let Ok(mut els) = serde_json::from_str::<Vec<serde_json::Value>>(s) {
                        for el in &mut els {
                            if let Some(obj) = el.as_object_mut() {
                                if let Some(x) = obj.get("x").and_then(|v| v.as_f64()) {
                                    obj.insert("x".into(), serde_json::json!(x + offset.0));
                                }
                                if let Some(y) = obj.get("y").and_then(|v| v.as_f64()) {
                                    obj.insert("y".into(), serde_json::json!(y + offset.1));
                                }
                            }
                        }
                        results.extend(els);
                    }
                }
            }
        }
        results
    }
}

fn lock_error_to_mcp(instance_id: &str, err: LockError) -> McpError {
    match err {
        LockError::InstanceLocked {
            holder_session,
            holder_agent,
            expires_at,
        } => {
            let remaining = (expires_at - chrono::Utc::now()).num_seconds().max(0);
            let agent_str = holder_agent
                .as_deref()
                .map(|n| format!(" (agent: '{n}')"))
                .unwrap_or_default();
            McpError {
                code: rmcp::model::ErrorCode::INVALID_REQUEST,
                message: std::borrow::Cow::Owned(format!(
                    "Instance '{instance_id}' is exclusively locked by session '{holder_session}'{agent_str}.\n\
                     Lock expires in {remaining}s (at {expires_at}).\n\
                     To use this instance, either:\n  \
                       1. Wait for the lock to expire or be released\n  \
                       2. Call vscreen_lock with action='acquire' and wait_timeout_seconds to queue\n  \
                       3. Call vscreen_lock with action='status' to check current status"
                )),
                data: None,
            }
        }
        LockError::NotHeld { .. } => McpError {
            code: rmcp::model::ErrorCode::INVALID_REQUEST,
            message: std::borrow::Cow::Owned(format!(
                "No lock held on instance '{instance_id}' by this session.\n\
                 Call vscreen_lock with action='acquire' to acquire a lock before using this tool."
            )),
            data: None,
        },
        LockError::Timeout { .. } => McpError {
            code: rmcp::model::ErrorCode::INVALID_REQUEST,
            message: std::borrow::Cow::Owned(format!(
                "Timed out waiting for lock on instance '{instance_id}'."
            )),
            data: None,
        },
    }
}

// ---------------------------------------------------------------------------
// Tool help generation
// ---------------------------------------------------------------------------

impl VScreenMcpServer {
    fn get_tool_help(&self, tool_name: &str) -> String {
        let tools: &[(&str, &str, &str)] = &[
            ("vscreen_list_instances", "List all browser instances", "Parameters: none\nReturns: Array of {instance_id, state, locked_by, supervisor_running}\nUse this first to discover available instances."),
            ("vscreen_screenshot", "Capture a screenshot", "Parameters:\n  instance_id: string (required)\n  format: \"png\" | \"jpeg\" | \"webp\" (default: \"png\")\n  quality: number 0-100 (for jpeg/webp only)\n  full_page: bool (default: false) — capture entire scrollable page\n  clip: {x, y, width, height} (optional) — capture only a specific region\n  annotate: bool (default: false) — overlay numbered bounding boxes on interactive elements\n  annotate_selector: string (optional) — CSS selector for annotation targets\n  sequence_count + sequence_interval_ms: capture multiple screenshots at intervals\nReturns: Base64-encoded image(s)\nExample: {\"instance_id\": \"dev\", \"full_page\": true} or {\"instance_id\": \"dev\", \"annotate\": true}"),
            ("vscreen_navigate", "Navigate the browser (goto/back/forward/reload)", "Parameters:\n  instance_id: string (required)\n  action: \"goto\" (default) | \"back\" | \"forward\" | \"reload\"\n  url: string (required for action=\"goto\")\n  wait_until: \"load\" | \"domcontentloaded\" | \"networkidle\" | \"none\" (for goto, default: \"load\")\nExample: {\"instance_id\": \"dev\", \"url\": \"https://example.com\"} or {\"instance_id\": \"dev\", \"action\": \"back\"}"),
            ("vscreen_click", "Click at page-space coordinates", "Parameters:\n  instance_id: string (required)\n  x: number (required) — page-space X coordinate\n  y: number (required) — page-space Y coordinate\n  button: 0 | 1 | 2 (default: 0) — 0=left, 1=middle, 2=right\nAuto-scrolls the target into view before clicking.\nCoordinates from vscreen_find can be used directly."),
            ("vscreen_double_click", "Double-click at coordinates", "Parameters:\n  instance_id: string (required)\n  x: number (required)\n  y: number (required)\nAuto-scrolls the target into view."),
            ("vscreen_type", "Type text into the currently focused element", "Parameters:\n  instance_id: string (required)\n  text: string (required) — text to type character by character\nTo focus an element first, click on it with vscreen_click.\nFor clearing and filling a field, use vscreen_fill instead."),
            ("vscreen_fill", "Clear and fill an input field by CSS selector", "Parameters:\n  instance_id: string (required)\n  selector: string (required) — CSS selector for the input (e.g., \"input[name='email']\")\n  value: string (required) — value to fill\nClears existing content first, then types the new value.\nExample: {\"instance_id\": \"dev\", \"selector\": \"#username\", \"value\": \"user@example.com\"}"),
            ("vscreen_key_press", "Press a single key", "Parameters:\n  instance_id: string (required)\n  key: string (required) — DOM key name\nKey names: \"Enter\", \"Tab\", \"Escape\", \"Backspace\", \"Delete\", \"ArrowUp\", \"ArrowDown\", \"ArrowLeft\", \"ArrowRight\", \"Home\", \"End\", \"PageUp\", \"PageDown\", \"F1\"-\"F12\", \"Space\""),
            ("vscreen_key_combo", "Press a key combination", "Parameters:\n  instance_id: string (required)\n  keys: [string, string, ...] (required) — keys to press simultaneously\nOrder matters: modifier keys first, then the main key.\nExamples: [\"Control\", \"a\"], [\"Control\", \"c\"], [\"Alt\", \"Tab\"], [\"Control\", \"Shift\", \"i\"]"),
            ("vscreen_scroll", "Scroll by pixel delta", "Parameters:\n  instance_id: string (required)\n  x: number (required) — page position to scroll at\n  y: number (required) — page position to scroll at\n  delta_x: number (default: 0) — horizontal scroll pixels\n  delta_y: number (default: 0) — vertical scroll pixels (positive = scroll down)"),
            ("vscreen_drag", "Drag from one point to another", "Parameters:\n  instance_id: string (required)\n  start_x, start_y: numbers — drag start coordinates\n  end_x, end_y: numbers — drag end coordinates\n  steps: number (default: 10) — interpolation steps"),
            ("vscreen_hover", "Hover at coordinates", "Parameters:\n  instance_id: string (required)\n  x: number (required)\n  y: number (required)\nTriggers CSS :hover states and JavaScript mouseover/mouseenter events."),
            ("vscreen_batch_click", "Click multiple points rapidly in one call", "Parameters:\n  instance_id: string (required)\n  points: [[x1,y1], [x2,y2], ...] (required) — array of [x, y] coordinate pairs\n  delay_between_ms: number (default: 50) — milliseconds between clicks\nIdeal for timed challenges like reCAPTCHA tile grids where individual MCP round-trips would be too slow.\nExample: {\"instance_id\": \"dev\", \"points\": [[135,224],[135,324],[135,424]], \"delay_between_ms\": 200}"),
            ("vscreen_click_element", "Click element by CSS selector or visible text", "Parameters:\n  instance_id: string (required)\n  selector: string (optional) — CSS selector\n  text: string (optional) — visible text to match\n  index: number (default: 0) — which match to click (if multiple)\n  button: 0 | 1 | 2 (default: 0)\nNote: Searches MAIN FRAME ONLY. For iframe elements, use vscreen_find(by='text', include_iframes=true) then vscreen_click."),
            ("vscreen_find", "Find elements on the page", "Parameters:\n  instance_id: string (required)\n  by: \"selector\" | \"text\" | \"input\" (required) — search mode\n  selector: string (for by=\"selector\") — CSS selector\n  text: string (for by=\"text\") — visible text to search for\n  exact: bool (for by=\"text\", default: false)\n  include_iframes: bool (default: false) — search inside iframes\n  placeholder, aria_label, label, role, name, input_type (for by=\"input\") — at least one required\nReturns: Element metadata with bounding boxes in page coordinates. Use with vscreen_click.\nExample: {\"instance_id\": \"dev\", \"by\": \"text\", \"text\": \"Sign In\"}"),
            ("vscreen_wait", "Wait for a condition (duration/idle/text/selector/url/network)", "Parameters:\n  condition: \"duration\" (default) | \"idle\" | \"text\" | \"selector\" | \"url\" | \"network\"\n  instance_id: string (required except for condition=\"duration\")\n  duration_ms: number (for condition=\"duration\")\n  text: string (for condition=\"text\")\n  selector: string (for condition=\"selector\")\n  url_contains: string (for condition=\"url\")\n  timeout_ms: number (default: 10000)\n  interval_ms: number (default: 250)\n  idle_ms: number (for idle/network, default: 500)\nExample: {\"duration_ms\": 2000} or {\"instance_id\": \"dev\", \"condition\": \"text\", \"text\": \"Welcome\"}"),
            ("vscreen_get_page_info", "Get page title, URL, viewport, and scroll position", "Parameters:\n  instance_id: string (required)\nReturns: {url, title, viewport: {width, height}, scrollX, scrollY}"),
            ("vscreen_extract_text", "Extract all visible text from the page", "Parameters:\n  instance_id: string (required)\nReturns: All visible text content from the page body."),
            ("vscreen_execute_js", "Execute JavaScript in the MAIN frame", "Parameters:\n  instance_id: string (required)\n  expression: string (required) — JavaScript to evaluate\nReturns: The expression result as JSON.\nRuns in the MAIN FRAME ONLY — cannot access iframe content.\nExample: {\"instance_id\": \"dev\", \"expression\": \"document.title\"}"),
            ("vscreen_list_frames", "List all frames including iframes", "Parameters:\n  instance_id: string (required)\nReturns: Frame tree (parent-child) + iframe bounding rectangles with page-space coordinates.\nEach iframe includes: src, name, title, x, y, width, height, visible.\nUse the bounding rect with vscreen_screenshot(clip=...) to zoom into an iframe."),
            ("vscreen_dismiss_dialogs", "Auto-dismiss cookie consent, privacy, and GDPR dialogs", "Parameters:\n  instance_id: string (required)\nChecks for OneTrust, CookieBot, Didomi, Quantcast, TrustArc, and many other consent frameworks.\nAlso matches common button text patterns in multiple languages.\nReturns which dialog was dismissed or 'no dialog found'.\nCall this after navigating to a new page if you expect consent overlays."),
            ("vscreen_solve_captcha", "Automatically solve reCAPTCHA v2 image challenges", "Parameters:\n  instance_id: string (required)\n  max_attempts: number (default: 3) — max page-reload retry cycles\nFinds the reCAPTCHA checkbox, clicks it, uses vision LLM to identify tiles, clicks them + VERIFY.\nHandles multi-round challenges, retries, and timeouts internally.\nReturns: {solved: bool, rounds: N, attempts: N, details: [...]}\nRequires vision LLM (--vision-url).\nSee `vscreen_help(topic=\"captcha\")` for the manual workflow fallback."),
            ("vscreen_click_and_navigate", "Click an element and wait for navigation", "Parameters:\n  instance_id: string (required)\n  selector: string (optional) — CSS selector\n  text: string (optional) — visible text match\n  timeout_ms: number (default: 5000) — how long to wait for URL change\n  fallback_to_link: bool (default: true) — try nearest <a> href if click doesn't navigate\nClicks the element, waits for URL change. If URL doesn't change and fallback_to_link is true, tries direct navigation via the nearest <a> tag's href.\nIdeal for SPA navigation (YouTube, React apps) where clicks trigger pushState."),
            ("vscreen_dismiss_ads", "Dismiss video platform ad overlays", "Parameters:\n  instance_id: string (required)\n  timeout_ms: number (default: 15000) — how long to wait for skip button\nDetects and dismisses YouTube skip buttons, pre-roll ad overlays, and generic close buttons.\nHandles localized skip button text (English, German, French, Spanish, Portuguese, Russian).\nWaits and polls for skip button to appear since video ads have a countdown."),
            ("vscreen_select_option", "Select a dropdown option", "Parameters:\n  instance_id: string (required)\n  selector: string (required) — CSS selector for the <select> element\n  value: string (optional) — option value attribute to select\n  label: string (optional) — visible text of the option to select\nProvide either value OR label, not both."),
            ("vscreen_scroll_to_element", "Scroll an element into view", "Parameters:\n  instance_id: string (required)\n  selector: string (required) — CSS selector\n  block: \"center\" | \"start\" | \"end\" | \"nearest\" (default: \"center\")"),
            ("vscreen_accessibility_tree", "Get the accessibility tree", "Parameters:\n  instance_id: string (required)\nReturns: Structured accessibility tree representation of the page."),
            ("vscreen_describe_elements", "Identify unlabeled UI elements using vision", "Parameters:\n  instance_id: string (required)\n  selector: string (default: \"button, [role='button'], a, input, select\") — CSS selector\n  include_labeled: bool (default: false) — if true, describes ALL elements\nUses vision LLM to identify icon-only buttons and other elements that lack text/aria-label.\nReturns: Array of elements with AI-generated descriptions: {tag, x, y, width, height, icon, action, label}\nRequires vision LLM to be configured (--vision-url)."),
            ("vscreen_storage", "Read/write cookies and web storage", "Parameters:\n  instance_id: string (required)\n  type: \"cookie\" | \"local\" | \"session\" (required)\n  action: \"get\" | \"set\" (required)\n  key: string (for local/session — get/set)\n  value: string (for set)\n  name: string (for cookie set)\n  domain, path: string (optional, for cookie set)\nExample: {\"instance_id\": \"dev\", \"type\": \"cookie\", \"action\": \"get\"} or {\"instance_id\": \"dev\", \"type\": \"local\", \"action\": \"set\", \"key\": \"theme\", \"value\": \"dark\"}"),
            ("vscreen_history", "Manage screenshot history", "Parameters:\n  instance_id: string (required)\n  action: \"list\" | \"get\" | \"range\" | \"clear\" (required)\n  index: number (for action=\"get\", 0 = oldest)\n  from, count: numbers (for action=\"range\")\nExample: {\"instance_id\": \"dev\", \"action\": \"list\"} or {\"instance_id\": \"dev\", \"action\": \"get\", \"index\": 0}"),
            ("vscreen_session_log", "Get recent action log", "Parameters:\n  instance_id: string (required)\n  last_n: number (optional) — number of recent actions to return"),
            ("vscreen_session_summary", "Get session summary", "Parameters:\n  instance_id: string (required)\nReturns: Action count, duration, screenshot count, and other session metrics."),
            ("vscreen_console_log", "Get captured browser console messages", "Parameters:\n  instance_id: string (required)\n  last_n: number (optional) — number of recent messages"),
            ("vscreen_console_clear", "Clear captured console messages", "Parameters:\n  instance_id: string (required)"),
            ("vscreen_lock", "Manage instance locks", "Parameters:\n  action: \"acquire\" | \"release\" | \"renew\" | \"status\" (required)\n  instance_id: string (required for acquire/release/renew; optional for status — omit for all instances)\n  lock_type: \"exclusive\" | \"observer\" (for acquire, default: \"exclusive\")\n  agent_name: string (optional, for acquire)\n  ttl_seconds: number (for acquire/renew, default: 120)\n  wait_timeout_seconds: number (for acquire, default: 0 = fail immediately)\n  lock_token: string (for release/renew if session changed)\nExample: {\"action\": \"acquire\", \"instance_id\": \"dev\"} or {\"action\": \"status\"}\nIn single-agent mode, locks are auto-acquired."),
            ("vscreen_plan", "Get the recommended tool sequence for a task", "Parameters:\n  task: string (required) — short description of what to accomplish\nExample: vscreen_plan(task=\"read all text on the page\")\nReturns: step-by-step tool recommendations with parameters.\nCall BEFORE starting multi-step workflows for optimal tool selection."),
            ("vscreen_help", "Get contextual documentation (this tool)", "Parameters:\n  topic: string (required) — tool name or concept\nTopics: quickstart, workflows, coordinates, iframes, locking, tools, tool-selection, troubleshooting, or any vscreen_* tool name."),
        ];

        if let Some((name, desc, details)) = tools.iter().find(|(n, _, _)| *n == tool_name) {
            format!("# {name}\n\n{desc}\n\n{details}")
        } else {
            let available: Vec<&str> = tools.iter().map(|(n, _, _)| *n).collect();
            format!(
                "Unknown tool: '{}'\n\nAvailable tools:\n{}",
                tool_name,
                available.join("\n")
            )
        }
    }
}

pub(crate) mod params;
use params::*;

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

#[tool_router(router = core_tools)]
impl VScreenMcpServer {
    #[tool(description = "List all browser instances and their states. Returns instance IDs, supervisor status, and lock information (who holds the lock, lock type, expiry). Use this to discover which instances are available before acquiring a lock.")]
    async fn vscreen_list_instances(&self) -> Result<CallToolResult, McpError> {
        let ids = self.state.registry.list_ids();
        let instances: Vec<serde_json::Value> = ids
            .iter()
            .filter_map(|id| {
                self.state.registry.get(id).ok().map(|entry| {
                    let has_supervisor = self.state.get_supervisor(id).is_some();
                    let lock_status = self.state.lock_manager.status(id);
                    let locked_by = lock_status.exclusive_holder.as_ref().map(|h| {
                        serde_json::json!({
                            "session_id": h.session_id.to_string(),
                            "agent_name": h.agent_name,
                            "lock_type": h.lock_type,
                            "expires_at": h.expires_at.to_rfc3339(),
                        })
                    });
                    let observer_count = lock_status.observers.len();
                    let queue_depth = lock_status.wait_queue.len();
                    let you_hold = lock_status.exclusive_holder.as_ref()
                        .map_or(false, |h| h.session_id == self.session_id)
                        || lock_status.observers.iter().any(|o| o.session_id == self.session_id);
                    let mut obj = serde_json::json!({
                        "instance_id": id.0,
                        "state": *entry.state_rx.borrow(),
                        "supervisor_running": has_supervisor,
                        "locked_by": locked_by,
                        "observer_count": observer_count,
                        "queue_depth": queue_depth,
                        "you_hold_lock": you_hold,
                    });
                    let port = self.state.rtsp_port;
                    if port > 0 {
                        obj["rtsp_url"] = serde_json::json!(
                            format!("rtsp://{{host}}:{port}/stream/{}", id.0)
                        );
                    }
                    obj
                })
            })
            .collect();

        let text = serde_json::to_string_pretty(&instances).unwrap_or_default();
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    // -----------------------------------------------------------------------
    // Lock management (consolidated)
    // -----------------------------------------------------------------------

    #[tool(description = "Manage instance locks. Actions: 'acquire' (get lock, default exclusive), 'release' (unlock), 'renew' (extend TTL), 'status' (query lock state). Locks are auto-acquired in single-agent mode. For multi-agent: acquire exclusive for writes, observer for reads. Example: {\"action\": \"acquire\", \"instance_id\": \"dev\"} or {\"action\": \"status\"}")]
    async fn vscreen_lock(
        &self,
        Parameters(params): Parameters<ConsolidatedLockParam>,
    ) -> Result<CallToolResult, McpError> {
        let action = params.action.to_lowercase();
        match action.as_str() {
            "acquire" => {
                let instance_id = params
                    .instance_id
                    .as_ref()
                    .ok_or_else(|| invalid_params("instance_id required for action 'acquire'"))?;
                let lock_type = Self::parse_lock_type(&params.lock_type)?;
                let ttl = Duration::from_secs(params.ttl_seconds.max(1));
                let id = InstanceId::from(instance_id.as_str());

                let result = if params.wait_timeout_seconds > 0 {
                    self.state
                        .lock_manager
                        .acquire_or_wait(
                            &id,
                            &self.session_id,
                            params.agent_name.clone(),
                            lock_type,
                            ttl,
                            Duration::from_secs(params.wait_timeout_seconds),
                        )
                        .await
                } else {
                    self.state.lock_manager.acquire(
                        &id,
                        &self.session_id,
                        params.agent_name.clone(),
                        lock_type,
                        ttl,
                    )
                };

                match result {
                    Ok(info) => {
                        let text = serde_json::to_string_pretty(&info).unwrap_or_default();
                        Ok(CallToolResult::success(vec![Content::text(format!(
                            "Lock acquired on instance '{}'.\n{text}",
                            instance_id
                        ))]))
                    }
                    Err(e) => Err(lock_error_to_mcp(instance_id, e)),
                }
            }
            "release" => {
                let instance_id = params
                    .instance_id
                    .as_ref()
                    .ok_or_else(|| invalid_params("instance_id required for action 'release'"))?;
                let id = InstanceId::from(instance_id.as_str());
                let token = params.lock_token.as_deref().and_then(LockToken::parse);
                match self.state.lock_manager.release_with_token(&id, &self.session_id, token.as_ref()) {
                    Ok(promoted) => {
                        let extra = if promoted {
                            " Next session in queue has been promoted."
                        } else {
                            ""
                        };
                        Ok(CallToolResult::success(vec![Content::text(format!(
                            "Lock released on instance '{}'.{extra}",
                            instance_id
                        ))]))
                    }
                    Err(e) => Err(lock_error_to_mcp(instance_id, e)),
                }
            }
            "renew" => {
                let instance_id = params
                    .instance_id
                    .as_ref()
                    .ok_or_else(|| invalid_params("instance_id required for action 'renew'"))?;
                let id = InstanceId::from(instance_id.as_str());
                let ttl = Duration::from_secs(params.ttl_seconds.max(1));
                let token = params.lock_token.as_deref().and_then(LockToken::parse);
                match self.state.lock_manager.renew_with_token(&id, &self.session_id, token.as_ref(), ttl) {
                    Ok(info) => {
                        let text = serde_json::to_string_pretty(&info).unwrap_or_default();
                        Ok(CallToolResult::success(vec![Content::text(format!(
                            "Lock renewed on instance '{}'.\n{text}",
                            instance_id
                        ))]))
                    }
                    Err(e) => Err(lock_error_to_mcp(instance_id, e)),
                }
            }
            "status" => {
                let statuses = if let Some(ref id) = params.instance_id {
                    vec![self.state.lock_manager.status(&InstanceId::from(id.as_str()))]
                } else {
                    self.state.lock_manager.status_all()
                };

                let mut enriched: Vec<serde_json::Value> = Vec::new();
                for status in &statuses {
                    let mut val = serde_json::to_value(status).unwrap_or_default();
                    let is_caller_holder = status
                        .exclusive_holder
                        .as_ref()
                        .map_or(false, |h| h.session_id == self.session_id);
                    let is_caller_observer = status
                        .observers
                        .iter()
                        .any(|o| o.session_id == self.session_id);
                    let queue_pos = status
                        .wait_queue
                        .iter()
                        .position(|w| w.session_id == self.session_id);
                    if let Some(obj) = val.as_object_mut() {
                        obj.insert("you_hold_exclusive".into(), serde_json::json!(is_caller_holder));
                        obj.insert("you_hold_observer".into(), serde_json::json!(is_caller_observer));
                        obj.insert(
                            "your_queue_position".into(),
                            serde_json::json!(queue_pos),
                        );
                    }
                    enriched.push(val);
                }

                let text = serde_json::to_string_pretty(&enriched).unwrap_or_default();
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }
            _ => Err(invalid_params(format!(
                "invalid action '{}': must be 'acquire', 'release', 'renew', or 'status'",
                params.action
            ))),
        }
    }

    // -----------------------------------------------------------------------
    // Audio / RTSP tools
    // -----------------------------------------------------------------------

    #[tool(description = "List active RTSP streaming sessions (audio+video) for a browser instance. Returns session IDs, quality tiers, media config (video/audio enabled), per-track details, client addresses, packet/byte counts, and health state. Use this to monitor who is consuming media from an instance via RTSP.")]
    async fn vscreen_audio_streams(
        &self,
        Parameters(params): Parameters<AudioStreamsParam>,
    ) -> Result<CallToolResult, McpError> {
        let mgr = self.state.rtsp_session_manager.as_ref()
            .ok_or_else(|| internal_error("RTSP server not running"))?;
        let sessions = mgr.sessions_for_instance(&params.instance_id);
        let text = serde_json::to_string_pretty(&sessions).unwrap_or_default();
        if sessions.is_empty() {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "No active RTSP sessions for instance '{}'.\n\nRTSP URL: rtsp://SERVER:8554/stream/{}\nAudio-only: rtsp://SERVER:8554/audio/{}\nDisable video: rtsp://SERVER:8554/stream/{}?video=false",
                params.instance_id, params.instance_id, params.instance_id, params.instance_id
            ))]))
        } else {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "{} active RTSP session(s) for instance '{}':\n{text}",
                sessions.len(), params.instance_id
            ))]))
        }
    }

    #[tool(description = "Get detailed info and health metrics for a specific RTSP session (audio+video). Returns quality tier, media config, per-track info (packets/bytes sent, health), client packet loss, jitter, uptime, and overall health state (healthy/degraded/failed).")]
    async fn vscreen_audio_stream_info(
        &self,
        Parameters(params): Parameters<AudioStreamInfoParam>,
    ) -> Result<CallToolResult, McpError> {
        let mgr = self.state.rtsp_session_manager.as_ref()
            .ok_or_else(|| internal_error("RTSP server not running"))?;
        let session = mgr.get(&params.session_id)
            .ok_or_else(|| invalid_params(format!("RTSP session '{}' not found", params.session_id)))?;
        let info = session.info();
        if info.instance_id != params.instance_id {
            return Err(invalid_params(format!(
                "Session '{}' belongs to instance '{}', not '{}'",
                params.session_id, info.instance_id, params.instance_id
            )));
        }
        let text = serde_json::to_string_pretty(&info).unwrap_or_default();
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "Get aggregated streaming health summary for a browser instance. Shows total sessions, healthy/degraded/failed track counts, and cumulative packet/byte statistics across all RTSP sessions (audio+video).")]
    async fn vscreen_audio_health(
        &self,
        Parameters(params): Parameters<AudioHealthParam>,
    ) -> Result<CallToolResult, McpError> {
        let mgr = self.state.rtsp_session_manager.as_ref()
            .ok_or_else(|| internal_error("RTSP server not running"))?;
        let health = mgr.aggregated_health(&params.instance_id);
        let text = serde_json::to_string_pretty(&health).unwrap_or_default();
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "Force-disconnect an RTSP streaming session (all tracks). Stops RTP delivery for both video and audio and releases server resources. The RTSP client will need to re-establish the session to resume streaming.")]
    async fn vscreen_rtsp_teardown(
        &self,
        Parameters(params): Parameters<RtspTeardownParam>,
    ) -> Result<CallToolResult, McpError> {
        let mgr = self.state.rtsp_session_manager.as_ref()
            .ok_or_else(|| internal_error("RTSP server not running"))?;
        match mgr.remove(&params.session_id) {
            Some(session) => {
                if session.instance_id.0 != params.instance_id {
                    return Ok(CallToolResult::success(vec![Content::text(format!(
                        "Warning: session '{}' was for instance '{}', not '{}'. It has been torn down regardless.",
                        params.session_id, session.instance_id, params.instance_id
                    ))]));
                }
                let total_packets: u64 = session.tracks.iter().map(|t| t.health.packets_sent).sum();
                let total_bytes: u64 = session.tracks.iter().map(|t| t.health.bytes_sent).sum();
                Ok(CallToolResult::success(vec![Content::text(format!(
                    "RTSP session '{}' torn down. Packets sent: {}, bytes sent: {}",
                    params.session_id, total_packets, total_bytes
                ))]))
            }
            None => Err(invalid_params(format!(
                "RTSP session '{}' not found", params.session_id
            ))),
        }
    }

    #[tool(description = "Get the recommended tool sequence for a task. Call this BEFORE starting a multi-step workflow to get the optimal tool plan. Pass a short description of what you want to accomplish (e.g. 'read all text on the page', 'click the Sign In button', 'fill out a form', 'navigate and extract data').")]
    async fn vscreen_plan(
        &self,
        Parameters(params): Parameters<PlanTaskParam>,
    ) -> Result<CallToolResult, McpError> {
        let task = params.task.to_lowercase();
        let mut recommendations = Vec::new();

        for pattern in TASK_PATTERNS {
            let matches_keywords = pattern
                .keywords
                .iter()
                .any(|kw| task.contains(kw));
            let blocked_by_negative = pattern
                .negative_keywords
                .iter()
                .any(|nk| task.contains(nk));
            if matches_keywords && !blocked_by_negative {
                recommendations.push(pattern);
            }
        }

        let text = if recommendations.is_empty() {
            format!(
                "No specific tool plan found for: \"{}\"\n\n\
                 General workflow:\n\
                 1. `vscreen_screenshot(full_page=true)` — see the entire page\n\
                 2. `vscreen_find_by_text(text=\"...\")` or `vscreen_find_elements(selector=\"...\")` — locate elements\n\
                 3. `vscreen_click(x, y)` or `vscreen_click_element(text=\"...\")` — interact\n\
                 4. `vscreen_screenshot` — verify result\n\n\
                 Tip: Call `vscreen_help(topic=\"tools\")` for the full tool reference.",
                params.task
            )
        } else {
            let mut out = format!("Recommended plan for: \"{}\"\n\n", params.task);
            for (i, pattern) in recommendations.iter().enumerate() {
                if recommendations.len() > 1 {
                    out.push_str(&format!("### Option {} — {}\n", i + 1, pattern.name));
                }
                out.push_str(pattern.recommendation);
                out.push_str("\n\n");
            }
            out
        };

        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "Get contextual documentation about vscreen tools, workflows, and concepts. Use this when unsure how to accomplish a task or how a tool's parameters work. Topics: 'quickstart' (getting started), 'workflows' (reCAPTCHA, cookies, forms, scraping), 'coordinates' (coordinate system explained), 'iframes' (cross-origin iframe handling), 'locking' (multi-agent lock management), 'tools' (complete tool reference by category), 'troubleshooting' (common issues and fixes), or any tool name like 'vscreen_batch_click' for that tool's detailed usage.")]
    async fn vscreen_help(
        &self,
        Parameters(params): Parameters<HelpParam>,
    ) -> Result<CallToolResult, McpError> {
        let topic = params.topic.trim().to_lowercase();
        let text = match topic.as_str() {
            "quickstart" | "start" | "getting-started" | "getting_started" => DOC_QUICKSTART.to_string(),
            "workflows" | "workflow" | "recipes" | "cookies" | "cookie" | "forms" | "form" => DOC_WORKFLOWS.to_string(),
            "captcha" | "recaptcha" | "captcha-solve" | "solve-captcha" => DOC_CAPTCHA.to_string(),
            "coordinates" | "coords" | "coordinate" | "position" | "positions" => DOC_COORDINATES.to_string(),
            "iframes" | "iframe" | "frames" | "frame" => DOC_IFRAMES.to_string(),
            "locking" | "lock" | "locks" | "multi-agent" | "multi_agent" => DOC_LOCKING.to_string(),
            "tools" | "reference" | "tool-reference" | "tool_reference" | "all" => DOC_TOOLS_HEADER.to_string(),
            "troubleshooting" | "troubleshoot" | "errors" | "error" | "help" | "debug" => DOC_TROUBLESHOOTING.to_string(),
            "tool-selection" | "tool_selection" | "selection" | "choosing" | "which-tool" | "which_tool" | "best-tool" | "best_tool" | "advisor" => DOC_TOOL_SELECTION.to_string(),
            _ if topic.starts_with("vscreen_") => {
                self.get_tool_help(&topic)
            }
            _ => {
                format!(
                    "Unknown topic: '{}'\n\nAvailable topics:\n\
                     - quickstart — Getting started guide\n\
                     - workflows — Common task recipes (cookies, forms, scraping)\n\
                     - captcha — Solving reCAPTCHA challenges (automated & manual)\n\
                     - coordinates — How the coordinate system works\n\
                     - iframes — Working with cross-origin iframes\n\
                     - locking — Multi-agent instance lock management\n\
                     - tools — Complete tool reference by category\n\
                     - tool-selection — Which tool to use for what task\n\
                     - troubleshooting — Common issues and solutions\n\
                     - Any tool name (e.g., vscreen_batch_click) — Detailed tool usage\n\n\
                     Tip: You can also use `list_resources` to browse documentation resources.",
                    params.topic
                )
            }
        };
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }
}

// ---------------------------------------------------------------------------
// ServerHandler implementation with Resources + Prompts
// ---------------------------------------------------------------------------

impl ServerHandler for VScreenMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(SERVER_INSTRUCTIONS.into()),
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_prompts()
                .build(),
            ..Default::default()
        }
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, McpError>> + Send + '_ {
        let make_resource = |uri: &str, name: &str, title: &str, desc: &str| -> Resource {
            RawResource {
                uri: uri.into(),
                name: name.into(),
                title: Some(title.into()),
                description: Some(desc.into()),
                mime_type: Some("text/markdown".into()),
                size: None,
                icons: None,
                meta: None,
            }.no_annotation()
        };
        let resources = vec![
            make_resource("vscreen://docs/quickstart", "Quick Start Guide", "Getting started with vscreen MCP", "Step-by-step guide to navigating, observing, and interacting with web pages."),
            make_resource("vscreen://docs/workflows", "Common Workflows", "Recipes for common tasks", "reCAPTCHA solving, cookie consent, article extraction, form filling, multi-page navigation."),
            make_resource("vscreen://docs/coordinates", "Coordinate System", "How coordinates work", "Page-space coordinates, iframe translation, viewport size, click target calculation."),
            make_resource("vscreen://docs/iframes", "Iframe Handling", "Working with cross-origin iframes", "Discovering iframes, finding elements across frames, clicking iframe content."),
            make_resource("vscreen://docs/locking", "Instance Locking", "Multi-agent lock management", "Lock types, single vs multi-agent mode, lock workflow, TTL and renewal."),
            make_resource("vscreen://docs/tools-reference", "Tool Reference", "Complete tool listing by category", "All available tools organized by category with descriptions."),
            make_resource("vscreen://docs/troubleshooting", "Troubleshooting", "Solutions to common problems", "Element not found, click doesn't work, timeouts, cookie consent, JavaScript errors."),
            make_resource("vscreen://docs/tool-selection", "Tool Selection Guide", "Which tool to use for what task", "Decision tables for observing, finding, clicking, waiting, scrolling. Avoid anti-patterns."),
        ];
        std::future::ready(Ok(ListResourcesResult {
            meta: None,
            resources,
            next_cursor: None,
        }))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ReadResourceResult, McpError>> + Send + '_ {
        let content = match request.uri.as_str() {
            "vscreen://docs/quickstart" => Some(DOC_QUICKSTART),
            "vscreen://docs/workflows" => Some(DOC_WORKFLOWS),
            "vscreen://docs/coordinates" => Some(DOC_COORDINATES),
            "vscreen://docs/iframes" => Some(DOC_IFRAMES),
            "vscreen://docs/locking" => Some(DOC_LOCKING),
            "vscreen://docs/tools-reference" => Some(DOC_TOOLS_HEADER),
            "vscreen://docs/troubleshooting" => Some(DOC_TROUBLESHOOTING),
            "vscreen://docs/tool-selection" => Some(DOC_TOOL_SELECTION),
            _ => None,
        };
        match content {
            Some(text) => std::future::ready(Ok(ReadResourceResult {
                contents: vec![ResourceContents::TextResourceContents {
                    uri: request.uri,
                    mime_type: Some("text/markdown".into()),
                    text: text.into(),
                    meta: None,
                }],
            })),
            None => std::future::ready(Err(McpError {
                code: ErrorCode::INVALID_PARAMS,
                message: std::borrow::Cow::Owned(format!("Unknown resource: {}", request.uri)),
                data: None,
            })),
        }
    }

    fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListPromptsResult, McpError>> + Send + '_ {
        let prompts = vec![
            Prompt {
                name: "navigate_and_observe".into(),
                title: Some("Navigate and Observe".into()),
                description: Some("Step-by-step workflow for navigating to a URL and gathering information about the page.".into()),
                arguments: Some(vec![
                    PromptArgument { name: "url".into(), title: None, description: Some("The URL to navigate to".into()), required: Some(true) },
                    PromptArgument { name: "instance_id".into(), title: None, description: Some("Instance ID (default: 'dev')".into()), required: Some(false) },
                ]),
                icons: None,
                meta: None,
            },
            Prompt {
                name: "solve_captcha".into(),
                title: Some("Solve reCAPTCHA".into()),
                description: Some("Complete workflow for solving Google reCAPTCHA challenges including iframe discovery, checkbox clicking, tile selection, and verification.".into()),
                arguments: Some(vec![
                    PromptArgument { name: "instance_id".into(), title: None, description: Some("Instance ID (default: 'dev')".into()), required: Some(false) },
                ]),
                icons: None,
                meta: None,
            },
            Prompt {
                name: "extract_article".into(),
                title: Some("Extract Article Content".into()),
                description: Some("Workflow for navigating to a news article URL and extracting its full text content, handling cookie consent dialogs.".into()),
                arguments: Some(vec![
                    PromptArgument { name: "url".into(), title: None, description: Some("The article URL".into()), required: Some(true) },
                    PromptArgument { name: "instance_id".into(), title: None, description: Some("Instance ID (default: 'dev')".into()), required: Some(false) },
                ]),
                icons: None,
                meta: None,
            },
            Prompt {
                name: "fill_form".into(),
                title: Some("Fill Web Form".into()),
                description: Some("Workflow for discovering form fields on a page and filling them with values.".into()),
                arguments: Some(vec![
                    PromptArgument { name: "instance_id".into(), title: None, description: Some("Instance ID (default: 'dev')".into()), required: Some(false) },
                ]),
                icons: None,
                meta: None,
            },
            Prompt {
                name: "interact_with_iframe".into(),
                title: Some("Interact with Iframe Content".into()),
                description: Some("Workflow for discovering, locating, and interacting with elements inside cross-origin iframes.".into()),
                arguments: Some(vec![
                    PromptArgument { name: "instance_id".into(), title: None, description: Some("Instance ID (default: 'dev')".into()), required: Some(false) },
                ]),
                icons: None,
                meta: None,
            },
        ];
        std::future::ready(Ok(ListPromptsResult {
            meta: None,
            prompts,
            next_cursor: None,
        }))
    }

    fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<GetPromptResult, McpError>> + Send + '_ {
        let instance_id = request.arguments.as_ref()
            .and_then(|a| a.get("instance_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("dev")
            .to_string();
        let url = request.arguments.as_ref()
            .and_then(|a| a.get("url"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let result = match request.name.as_str() {
            "navigate_and_observe" => Ok(GetPromptResult {
                description: Some("Navigate to a URL and gather information".into()),
                messages: vec![
                    PromptMessage::new_text(PromptMessageRole::User, format!(
                        "Navigate to {url} on instance '{instance_id}' and report what you see.\n\n\
                         Follow these steps:\n\
                         1. Call `vscreen_navigate` with instance_id=\"{instance_id}\", url=\"{url}\", wait_until=\"load\"\n\
                         2. Call `vscreen_screenshot` with instance_id=\"{instance_id}\" to see the page\n\
                         3. Call `vscreen_dismiss_dialogs` with instance_id=\"{instance_id}\" to clear any overlays\n\
                         4. If a dialog was dismissed, take another screenshot\n\
                         5. Call `vscreen_get_page_info` for title and URL\n\
                         6. Call `vscreen_extract_text` for the page content\n\
                         7. Summarize what you found"
                    )),
                ],
            }),
            "solve_captcha" => Ok(GetPromptResult {
                description: Some("Step-by-step reCAPTCHA solving workflow".into()),
                messages: vec![
                    PromptMessage::new_text(PromptMessageRole::User, format!(
                        "Solve the reCAPTCHA on instance '{instance_id}'.\n\n\
                         Follow these steps precisely:\n\n\
                         **Step 1: Discover the reCAPTCHA iframe**\n\
                         Call `vscreen_list_frames(instance_id=\"{instance_id}\")`. Look for an iframe with title containing 'reCAPTCHA' or URL containing 'recaptcha/api2/anchor'. Note its bounding rect (x, y, width, height).\n\n\
                         **Step 2: Find and click the checkbox**\n\
                         Call `vscreen_find_by_text(instance_id=\"{instance_id}\", text=\"I'm not a robot\", include_iframes=true)`. \
                         Look for the result with tag 'label' or the smallest containing div. \
                         Click approximately 28px left of the text x-coordinate at the text y-center: `vscreen_click(x=text.x - 28, y=text.y + text.height/2)`.\n\n\
                         **Step 3: Check for challenge**\n\
                         Call `vscreen_wait(duration_ms=3000)` then `vscreen_screenshot(instance_id=\"{instance_id}\")`. \
                         If a green checkmark appears, you're done! If a tile grid challenge appeared, continue.\n\n\
                         **Step 4: Analyze the challenge**\n\
                         Call `vscreen_list_frames` again — the challenge iframe (title containing 'challenge expires') should now be visible. \
                         Use `vscreen_screenshot(instance_id=\"{instance_id}\", clip=challenge_iframe_bounds)` for a close-up of the challenge grid. \
                         Identify which tiles match the prompt (e.g., 'traffic lights', 'crosswalks').\n\n\
                         **Step 5: Click tiles**\n\
                         Calculate tile centers based on the challenge iframe bounds. For a 4x4 grid with iframe at (ix, iy, w, h): \
                         header is ~90px, grid is ~(h-150)px. Each cell is grid_size/4 pixels. \
                         Cell(row,col) center = (ix + col*cell_w + cell_w/2, iy + 90 + row*cell_h + cell_h/2). \
                         Call `vscreen_batch_click(instance_id=\"{instance_id}\", points=[[x1,y1],[x2,y2],...], delay_between_ms=200)` with ALL correct tiles in one call.\n\n\
                         **Step 6: Verify**\n\
                         Call `vscreen_find_by_text(instance_id=\"{instance_id}\", text=\"VERIFY\", include_iframes=true)`. \
                         Click the button center. Wait 3 seconds, take a screenshot to confirm the green checkmark."
                    )),
                ],
            }),
            "extract_article" => Ok(GetPromptResult {
                description: Some("Extract article content from a URL".into()),
                messages: vec![
                    PromptMessage::new_text(PromptMessageRole::User, format!(
                        "Extract the article text from {url} on instance '{instance_id}'.\n\n\
                         Follow these steps:\n\
                         1. Call `vscreen_navigate(instance_id=\"{instance_id}\", url=\"{url}\", wait_until=\"load\")`\n\
                         2. Call `vscreen_dismiss_dialogs(instance_id=\"{instance_id}\")` to handle cookie consent\n\
                         3. Call `vscreen_wait(duration_ms=1000)` for dynamic content to load\n\
                         4. Call `vscreen_extract_text(instance_id=\"{instance_id}\")` for the full page text\n\
                         5. If the text is too noisy, try: `vscreen_find_elements(instance_id=\"{instance_id}\", selector=\"article, main, [role=main], .article-body, .story-body\")` \
                            to find the article container, then use `vscreen_execute_js` to extract just that element's innerText\n\
                         6. Return the extracted article title and body text"
                    )),
                ],
            }),
            "fill_form" => Ok(GetPromptResult {
                description: Some("Discover and fill form fields".into()),
                messages: vec![
                    PromptMessage::new_text(PromptMessageRole::User, format!(
                        "Fill the form on instance '{instance_id}'.\n\n\
                         Follow these steps:\n\
                         1. Call `vscreen_screenshot(instance_id=\"{instance_id}\")` to see the form\n\
                         2. Call `vscreen_find_elements(instance_id=\"{instance_id}\", selector=\"input, select, textarea\")` to discover all fields\n\
                         3. For text inputs: `vscreen_fill(instance_id=\"{instance_id}\", selector=\"input[name='fieldname']\", value=\"value\")`\n\
                         4. For dropdowns: `vscreen_select_option(instance_id=\"{instance_id}\", selector=\"select[name='field']\", value=\"option_value\")`\n\
                         5. For checkboxes/radios: click at the element's coordinates\n\
                         6. To submit: `vscreen_find_elements(instance_id=\"{instance_id}\", selector=\"button[type='submit'], input[type='submit']\")` then click\n\
                         7. Take a screenshot to verify submission result"
                    )),
                ],
            }),
            "interact_with_iframe" => Ok(GetPromptResult {
                description: Some("Interact with cross-origin iframe content".into()),
                messages: vec![
                    PromptMessage::new_text(PromptMessageRole::User, format!(
                        "Interact with iframe content on instance '{instance_id}'.\n\n\
                         Cross-origin iframes have separate DOM trees. Here's how to work with them:\n\n\
                         1. Call `vscreen_list_frames(instance_id=\"{instance_id}\")` to discover all iframes.\n\
                            Note the iframe's bounding rect (x, y, width, height) and visibility.\n\n\
                         2. To find elements inside iframes, use:\n\
                            `vscreen_find_elements(instance_id=\"{instance_id}\", selector=\"button\", include_iframes=true)`\n\
                            or `vscreen_find_by_text(instance_id=\"{instance_id}\", text=\"Click me\", include_iframes=true)`\n\
                            Results include a `frame_id` and page-space coordinates.\n\n\
                         3. Click using the returned page-space coordinates:\n\
                            `vscreen_click(instance_id=\"{instance_id}\", x=result.x + result.width/2, y=result.y + result.height/2)`\n\n\
                         4. To zoom into an iframe for analysis:\n\
                            `vscreen_screenshot(instance_id=\"{instance_id}\", clip={{\"x\": iframe.x, \"y\": iframe.y, \"width\": iframe.width, \"height\": iframe.height}})`\n\n\
                         Limitations:\n\
                         - `vscreen_execute_js` runs in the main frame only\n\
                         - `vscreen_click_element` searches the main frame only\n\
                         - Use `vscreen_find_by_text` + `vscreen_click` for iframe elements instead"
                    )),
                ],
            }),
            _ => Err(McpError {
                code: ErrorCode::INVALID_PARAMS,
                message: std::borrow::Cow::Owned(format!("Unknown prompt: {}", request.name)),
                data: None,
            }),
        };
        std::future::ready(result)
    }

    // -- Manual tool dispatch (replaces #[tool_handler]) to inject advisor hints --

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let tool_name = request.name.to_string();
        let args_value = request
            .arguments
            .as_ref()
            .map(|a| serde_json::Value::Object(a.clone()))
            .unwrap_or(serde_json::Value::Null);

        let args_snapshot = ToolAdvisor::build_args_snapshot(&tool_name, &args_value);
        let hint = {
            let mut advisor = self.advisor.lock().await;
            let hint = advisor.check_anti_patterns(&tool_name, &args_value);
            advisor.record(ToolCallRecord {
                tool_name: tool_name.clone(),
                args_snapshot,
            });
            hint
        };

        let tcc = ToolCallContext::new(self, request, context);
        let mut result = self.tool_router.call(tcc).await?;

        if let Some(hint_text) = hint {
            result.content.push(Content::text(format!("\n\n---\n{hint_text}")));
        }

        Ok(result)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult {
            tools: self.tool_router.list_all(),
            meta: None,
            next_cursor: None,
        })
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tool_router.get(name).cloned()
    }
}

// ---------------------------------------------------------------------------
// Transport runners
// ---------------------------------------------------------------------------

/// Run the MCP server on stdin/stdout (for subprocess spawning by MCP clients).
///
/// # Errors
/// Returns an error if the server fails to start.
pub async fn run_mcp_stdio(state: AppState) -> Result<(), Box<dyn std::error::Error>> {
    info!("starting MCP server on stdio");
    state.lock_manager.spawn_reaper(state.cancel.clone());
    let server = VScreenMcpServer::new(state);
    let service = server
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|e| format!("MCP stdio server error: {e}"))?;
    service.waiting().await?;
    Ok(())
}

/// Run the MCP server on HTTP (streamable HTTP transport).
///
/// # Errors
/// Returns an error if the server fails to bind or start.
pub async fn run_mcp_sse(
    state: AppState,
    addr: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    info!(addr, "starting MCP server on HTTP/SSE");

    state.lock_manager.spawn_reaper(state.cancel.clone());
    let cancel = state.cancel.clone();
    let state_clone = state;

    use rmcp::transport::streamable_http_server::session::local::SessionConfig;
    let session_config = SessionConfig {
        channel_capacity: SessionConfig::DEFAULT_CHANNEL_CAPACITY,
        keep_alive: Some(Duration::from_secs(300)),
    };
    let session_manager = Arc::new(
        rmcp::transport::streamable_http_server::session::local::LocalSessionManager {
            sessions: Default::default(),
            session_config,
        },
    );

    let config = rmcp::transport::StreamableHttpServerConfig {
        cancellation_token: cancel.into(),
        ..Default::default()
    };

    let service = rmcp::transport::StreamableHttpService::new(
        move || {
            let server = VScreenMcpServer::new(state_clone.clone());
            Ok(server)
        },
        session_manager,
        config,
    );

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("MCP SSE bind error: {e}"))?;

    info!(addr, "MCP SSE server listening");

    use tower_http::trace::TraceLayer;
    let trace_layer = TraceLayer::new_for_http()
        .make_span_with(|req: &axum::http::Request<_>| {
            let method = req.method().clone();
            let uri = req.uri().clone();
            let session_id = req
                .headers()
                .get("mcp-session-id")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("-");
            tracing::info_span!("mcp_http", %method, %uri, session_id)
        })
        .on_response(
            |response: &axum::http::Response<_>,
             latency: std::time::Duration,
             _span: &tracing::Span| {
                let status = response.status();
                if status.is_server_error() {
                    tracing::error!(
                        %status,
                        latency_ms = latency.as_millis(),
                        "MCP HTTP server error response"
                    );
                } else {
                    tracing::debug!(
                        %status,
                        latency_ms = latency.as_millis(),
                        "MCP HTTP response"
                    );
                }
            },
        );

    // Middleware: strip Last-Event-Id from GET requests.
    // Cursor's Streamable HTTP client tracks event IDs across all SSE responses
    // (including POST response streams). When it opens a GET SSE stream for
    // notifications, it sends Last-Event-Id from the last POST response.
    // rmcp's session manager can't resume from request-scoped channels that
    // are already closed, causing a 500. Stripping the header forces a fresh
    // standalone stream instead of a failed resume.
    let strip_last_event_id = axum::middleware::from_fn(
        |mut req: axum::http::Request<axum::body::Body>,
         next: axum::middleware::Next| async move {
            if req.method() == axum::http::Method::GET {
                req.headers_mut().remove("last-event-id");
            }
            next.run(req).await
        },
    );

    let app = axum::Router::new()
        .fallback_service(service)
        .layer(strip_last_event_id)
        .layer(trace_layer);

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            warn!(%e, "MCP SSE server error");
        }
    });

    Ok(())
}

#[cfg(test)]
mod tests;
