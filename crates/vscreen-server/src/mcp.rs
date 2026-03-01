use std::collections::VecDeque;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use rmcp::handler::server::tool::{ToolCallContext, ToolRouter};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::service::RequestContext;
use rmcp::{tool, tool_router, RoleServer, ServerHandler, ServiceExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};
use vscreen_core::event::InputEvent;
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

/// RAII wrapper around `JoinHandle` that aborts the spawned task when dropped.
/// Prevents orphaned vision model requests from keeping the GPU busy after
/// the parent tool call is cancelled or times out.
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl<T> std::future::Future for AbortOnDrop<T> {
    type Output = Result<T, tokio::task::JoinError>;
    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        std::pin::Pin::new(&mut self.0).poll(cx)
    }
}

// ---------------------------------------------------------------------------
// Tool Advisor — session-aware anti-pattern detection
// ---------------------------------------------------------------------------

struct ToolCallRecord {
    tool_name: String,
}

struct ToolAdvisor {
    recent_calls: VecDeque<ToolCallRecord>,
}

impl ToolAdvisor {
    fn new() -> Self {
        Self {
            recent_calls: VecDeque::with_capacity(24),
        }
    }

    fn record(&mut self, record: ToolCallRecord) {
        if self.recent_calls.len() >= 20 {
            self.recent_calls.pop_front();
        }
        self.recent_calls.push_back(record);
    }

    fn check_anti_patterns(&self, tool_name: &str, args: &serde_json::Value) -> Option<String> {
        match tool_name {
            "vscreen_screenshot" => self.check_screenshot_patterns(args),
            "vscreen_scroll" => self.check_scroll_patterns(),
            "vscreen_wait" => self.check_wait_patterns(),
            "vscreen_execute_js" => self.check_js_patterns(args),
            _ => None,
        }
    }

    fn check_screenshot_patterns(&self, args: &serde_json::Value) -> Option<String> {
        let has_clip = args.get("clip").is_some();
        let has_full_page = args.get("full_page").and_then(|v| v.as_bool()).unwrap_or(false);
        if has_clip || has_full_page {
            return None;
        }
        let recent: Vec<&ToolCallRecord> = self.recent_calls.iter().rev().take(8).collect();
        let mut scroll_screenshot_pairs = 0;
        for window in recent.windows(2) {
            if window[0].tool_name == "vscreen_screenshot"
                && window[1].tool_name == "vscreen_scroll"
            {
                scroll_screenshot_pairs += 1;
            }
        }
        if scroll_screenshot_pairs >= 2 {
            return Some(
                "Advisor: You are scrolling and screenshotting repeatedly. \
                 Use vscreen_screenshot(full_page=true) to capture the entire page in one call."
                    .to_string(),
            );
        }
        None
    }

    fn check_scroll_patterns(&self) -> Option<String> {
        let recent: Vec<&ToolCallRecord> = self.recent_calls.iter().rev().take(6).collect();
        let has_recent_find = recent
            .iter()
            .any(|r| r.tool_name == "vscreen_find_elements" || r.tool_name == "vscreen_find_by_text");
        let scroll_count = recent
            .iter()
            .filter(|r| r.tool_name == "vscreen_scroll")
            .count();
        if scroll_count >= 2 {
            return Some(
                "Advisor: You have scrolled multiple times. Consider using \
                 vscreen_screenshot(full_page=true) to see the entire page, or \
                 vscreen_scroll_to_element(selector) to jump directly to a specific element."
                    .to_string(),
            );
        }
        if has_recent_find {
            return Some(
                "Advisor: You recently used find_elements/find_by_text. \
                 Use vscreen_scroll_to_element(selector) to bring a specific element into view \
                 instead of manual scroll deltas."
                    .to_string(),
            );
        }
        None
    }

    fn check_wait_patterns(&self) -> Option<String> {
        let recent: Vec<&ToolCallRecord> = self.recent_calls.iter().rev().take(6).collect();
        let wait_count = recent
            .iter()
            .filter(|r| r.tool_name == "vscreen_wait")
            .count();
        if wait_count >= 2 {
            return Some(
                "Advisor: You have used multiple fixed-duration waits. Consider using \
                 vscreen_wait_for_text(text=...), vscreen_wait_for_selector(selector=...), \
                 or vscreen_wait_for_network_idle instead of fixed waits."
                    .to_string(),
            );
        }
        None
    }

    fn check_js_patterns(&self, args: &serde_json::Value) -> Option<String> {
        let expr = args
            .get("expression")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let expr_lower = expr.to_lowercase();
        if expr_lower.contains("document.title")
            || expr_lower.contains("document.url")
            || expr_lower.contains("location.href")
            || expr_lower.contains("window.location")
        {
            return Some(
                "Advisor: Use vscreen_get_page_info to get page title, URL, and viewport \
                 instead of executing JavaScript."
                    .to_string(),
            );
        }
        if expr_lower.contains("innertext")
            || expr_lower.contains("textcontent")
            || expr_lower.contains("document.body")
        {
            return Some(
                "Advisor: Use vscreen_extract_text to get all visible page text \
                 instead of executing JavaScript."
                    .to_string(),
            );
        }
        None
    }
}

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
            tool_router: Self::tool_router(),
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
            tool_router: Self::tool_router(),
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

    /// Check if the reCAPTCHA is solved by looking for the green checkmark in the anchor iframe.
    async fn captcha_is_solved(&self, sup: &Arc<InstanceSupervisor>) -> bool {
        let frame_tree = match sup.get_frame_tree().await {
            Ok(ft) => ft,
            Err(_) => return false,
        };
        let child_frames = frame_tree
            .get("frameTree")
            .or(Some(&frame_tree))
            .and_then(|ft| ft.get("childFrames"))
            .and_then(|cf| cf.as_array());

        if let Some(frames) = child_frames {
            for child in frames {
                let url = child
                    .get("frame")
                    .and_then(|f| f.get("url"))
                    .and_then(|u| u.as_str())
                    .unwrap_or("");
                if !url.contains("recaptcha") || !url.contains("anchor") {
                    continue;
                }
                let frame_id = child
                    .get("frame")
                    .and_then(|f| f.get("id"))
                    .and_then(|id| id.as_str())
                    .unwrap_or("");
                if frame_id.is_empty() {
                    continue;
                }
                let check_js = r#"(function(){
                    const anchor = document.getElementById('recaptcha-anchor');
                    if (anchor && anchor.getAttribute('aria-checked') === 'true') return 'solved';
                    return 'unsolved';
                })()"#;
                if let Ok(val) = sup.evaluate_js_in_frame(check_js, frame_id).await {
                    let s = val.as_str().unwrap_or("");
                    if s == "solved" {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Find the reCAPTCHA challenge (bframe) iframe and return its page-space bounds.
    async fn find_captcha_challenge_iframe(
        &self,
        sup: &Arc<InstanceSupervisor>,
    ) -> Option<(f64, f64, f64, f64)> {
        let js = r#"JSON.stringify(Array.from(document.querySelectorAll('iframe')).map(f => {
            const r = f.getBoundingClientRect();
            return {
                title: f.title || '',
                src: f.src || '',
                x: r.left + window.scrollX,
                y: r.top + window.scrollY,
                width: r.width,
                height: r.height,
                visible: r.width > 0 && r.height > 0 && r.top > -9000,
            };
        }))"#;
        let result = sup.evaluate_js(js).await.ok()?;
        let s = result.as_str().unwrap_or("[]");
        let iframes: Vec<serde_json::Value> = serde_json::from_str(s).unwrap_or_default();

        iframes.iter().find_map(|f| {
            let title = f.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let src = f.get("src").and_then(|v| v.as_str()).unwrap_or("");
            let visible = f.get("visible").and_then(|v| v.as_bool()).unwrap_or(false);
            if !visible {
                return None;
            }
            if title.contains("challenge") || src.contains("bframe") {
                let x = f.get("x").and_then(|v| v.as_f64())?;
                let y = f.get("y").and_then(|v| v.as_f64())?;
                let w = f.get("width").and_then(|v| v.as_f64())?;
                let h = f.get("height").and_then(|v| v.as_f64())?;
                if w > 100.0 && h > 100.0 {
                    return Some((x, y, w, h));
                }
            }
            None
        })
    }

    /// Check if the reCAPTCHA challenge has expired by looking for expiry text
    /// in both the main page and the anchor iframe.
    async fn captcha_check_expired(&self, sup: &Arc<InstanceSupervisor>) -> bool {
        let js = r#"document.body.innerText.includes('Verification challenge expired') ||
                     document.body.innerText.includes('challenge expired')"#;
        if sup.evaluate_js(js).await.ok().and_then(|v| v.as_bool()).unwrap_or(false) {
            return true;
        }
        // Also check inside the anchor iframe
        let frame_tree = match sup.get_frame_tree().await {
            Ok(ft) => ft,
            Err(_) => return false,
        };
        let child_frames = frame_tree
            .get("frameTree")
            .or(Some(&frame_tree))
            .and_then(|ft| ft.get("childFrames"))
            .and_then(|cf| cf.as_array());
        if let Some(frames) = child_frames {
            for child in frames {
                let url = child.get("frame").and_then(|f| f.get("url")).and_then(|u| u.as_str()).unwrap_or("");
                if !url.contains("recaptcha") { continue; }
                let fid = child.get("frame").and_then(|f| f.get("id")).and_then(|id| id.as_str()).unwrap_or("");
                if fid.is_empty() { continue; }
                let check = r#"document.body.innerText.includes('expired') || document.body.innerText.includes('Expired')"#;
                if let Ok(val) = sup.evaluate_js_in_frame(check, fid).await {
                    if val.as_bool().unwrap_or(false) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Find the reCAPTCHA challenge (bframe) iframe's CDP frame ID for running JS inside it.
    async fn find_captcha_bframe_id(&self, sup: &Arc<InstanceSupervisor>) -> Option<String> {
        let frame_tree = sup.get_frame_tree().await.ok()?;
        let child_frames = frame_tree
            .get("frameTree")
            .or(Some(&frame_tree))
            .and_then(|ft| ft.get("childFrames"))
            .and_then(|cf| cf.as_array())?;

        for child in child_frames {
            let url = child
                .get("frame")
                .and_then(|f| f.get("url"))
                .and_then(|u| u.as_str())
                .unwrap_or("");
            if url.contains("recaptcha") && url.contains("bframe") {
                let frame_id = child
                    .get("frame")
                    .and_then(|f| f.get("id"))
                    .and_then(|id| id.as_str())
                    .unwrap_or("");
                if !frame_id.is_empty() {
                    return Some(frame_id.to_string());
                }
            }
        }
        None
    }

    /// Check the challenge iframe DOM for tile replacement indicators.
    /// Returns a JSON object with:
    ///   - `has_new_images`: true if "Please also check the new images" is visible
    ///   - `tiles_animating`: count of tiles currently in animation/transition
    ///   - `header_text`: the current instruction header text
    async fn captcha_challenge_state(
        &self,
        sup: &Arc<InstanceSupervisor>,
        bframe_id: &str,
    ) -> Option<serde_json::Value> {
        let js = r#"(function(){
            var header = document.querySelector('.rc-imageselect-desc-wrapper, .rc-imageselect-desc, .rc-imageselect-desc-no-canonical');
            var headerText = header ? header.innerText.trim() : '';
            var hasNewImages = headerText.toLowerCase().includes('also check the new images') ||
                               headerText.toLowerCase().includes('check the new images');
            var tiles = document.querySelectorAll('.rc-imageselect-tile');
            var animating = 0;
            tiles.forEach(function(t) {
                var style = window.getComputedStyle(t);
                if (style.transition && style.transition !== 'none' && style.transition !== 'all 0s ease 0s') {
                    var td = t.querySelector('.rc-image-tile-33, .rc-image-tile-44, .rc-image-tile-11');
                    if (td && td.classList.contains('rc-image-tile-33')) animating++;
                }
            });
            var dynamicTiles = document.querySelectorAll('.rc-imageselect-dynamic-selected');
            return JSON.stringify({
                has_new_images: hasNewImages,
                tiles_animating: animating,
                dynamic_selected: dynamicTiles.length,
                header_text: headerText,
                is_dynamic: document.querySelectorAll('.rc-imageselect-dynamic-selected, .rc-imageselect-tileselected').length > 0 || hasNewImages
            });
        })()"#;
        let val = sup.evaluate_js_in_frame(js, bframe_id).await.ok()?;
        let s = val.as_str().unwrap_or("{}");
        serde_json::from_str(s).ok()
    }

    /// Wait for tile replacement animations to settle by polling DOM state.
    /// Returns when no tiles are animating or after max_wait.
    async fn wait_for_tile_animation(
        &self,
        sup: &Arc<InstanceSupervisor>,
        bframe_id: &str,
        max_wait: Duration,
    ) {
        let start = std::time::Instant::now();
        let poll_interval = Duration::from_millis(200);
        loop {
            if start.elapsed() >= max_wait {
                break;
            }
            tokio::time::sleep(poll_interval).await;
            if let Some(state) = self.captcha_challenge_state(sup, bframe_id).await {
                let animating = state.get("tiles_animating").and_then(|v| v.as_u64()).unwrap_or(0);
                let dynamic = state.get("dynamic_selected").and_then(|v| v.as_u64()).unwrap_or(0);
                if animating == 0 && dynamic == 0 {
                    break;
                }
            }
        }
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

// ---------------------------------------------------------------------------
// reCAPTCHA grid geometry helpers
// ---------------------------------------------------------------------------

/// Compute tile center coordinates in page-space for a reCAPTCHA challenge grid.
///
/// The challenge iframe contains:
/// - A blue header (~95px tall)
/// - A tile grid that fills most of the remaining space
/// - A footer bar (~60px tall) with VERIFY/SKIP
///
/// Grid tiles are equal-sized squares arranged in either 3x3 or 4x4 layout.
/// We detect the grid size from the iframe dimensions: wider iframes (>350px
/// with height >500px) use 3x3 when wide, 4x4 when the header is shorter.
fn compute_grid_positions(
    iframe_x: f64,
    iframe_y: f64,
    iframe_w: f64,
    iframe_h: f64,
) -> Vec<[f64; 2]> {
    let header_h = 95.0;
    let footer_h = 65.0;
    let grid_h = iframe_h - header_h - footer_h;
    let grid_y = iframe_y + header_h;
    let grid_x = iframe_x;

    // Determine grid dimensions: if each tile would be >110px in a 4x4, it's 3x3
    let tile_w_4 = iframe_w / 4.0;
    let (cols, rows) = if tile_w_4 > 95.0 && grid_h / 4.0 > 95.0 {
        // Could be either; use aspect ratio — 3x3 tiles are ~130px, 4x4 ~97px
        if grid_h / 3.0 > 120.0 && iframe_w / 3.0 > 120.0 {
            (3usize, 3usize)
        } else {
            (4, 4)
        }
    } else {
        (3, 3)
    };

    let tile_w = iframe_w / cols as f64;
    let tile_h = grid_h / rows as f64;

    let mut positions = Vec::with_capacity(cols * rows);
    for row in 0..rows {
        for col in 0..cols {
            positions.push([
                grid_x + col as f64 * tile_w + tile_w / 2.0,
                grid_y + row as f64 * tile_h + tile_h / 2.0,
            ]);
        }
    }
    positions
}

/// Compute the VERIFY/SKIP button center in page-space.
fn compute_verify_button(
    iframe_x: f64,
    iframe_y: f64,
    iframe_w: f64,
    iframe_h: f64,
) -> [f64; 2] {
    // VERIFY button sits in the footer, right-aligned, ~50px from right edge
    [iframe_x + iframe_w - 50.0, iframe_y + iframe_h - 30.0]
}

#[cfg(test)]
mod captcha_grid_tests {
    use super::*;

    #[test]
    fn grid_3x3_positions() {
        let positions = compute_grid_positions(85.0, 84.0, 400.0, 580.0);
        assert_eq!(positions.len(), 9);
        // First tile center should be roughly at (85 + 400/3/2, 84 + 95 + 420/3/2)
        let first = positions[0];
        assert!(first[0] > 100.0 && first[0] < 200.0, "x={}", first[0]);
        assert!(first[1] > 200.0 && first[1] < 300.0, "y={}", first[1]);
        // Last tile center
        let last = positions[8];
        assert!(last[0] > 380.0 && last[0] < 500.0, "x={}", last[0]);
        assert!(last[1] > 480.0 && last[1] < 600.0, "y={}", last[1]);
    }

    #[test]
    fn verify_button_position() {
        let btn = compute_verify_button(85.0, 84.0, 400.0, 580.0);
        assert!(btn[0] > 400.0 && btn[0] < 500.0, "x={}", btn[0]);
        assert!(btn[1] > 600.0 && btn[1] < 680.0, "y={}", btn[1]);
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
                       2. Call vscreen_instance_lock with wait_timeout_seconds to queue\n  \
                       3. Call vscreen_instance_lock_status to check current status"
                )),
                data: None,
            }
        }
        LockError::NotHeld { .. } => McpError {
            code: rmcp::model::ErrorCode::INVALID_REQUEST,
            message: std::borrow::Cow::Owned(format!(
                "No lock held on instance '{instance_id}' by this session.\n\
                 Call vscreen_instance_lock to acquire a lock before using this tool."
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
            ("vscreen_screenshot", "Capture a screenshot of the browser", "Parameters:\n  instance_id: string (required) — e.g., \"dev\"\n  format: \"png\" | \"jpeg\" | \"webp\" (default: \"png\")\n  quality: number 0-100 (for jpeg/webp only)\n  full_page: bool (default: false) — capture entire scrollable page\n  clip: {x, y, width, height} (optional) — capture only a specific region\nReturns: Base64-encoded image\nExample: {\"instance_id\": \"dev\", \"clip\": {\"x\": 85, \"y\": 84, \"width\": 400, \"height\": 580}}"),
            ("vscreen_screenshot_sequence", "Capture multiple screenshots at intervals", "Parameters:\n  instance_id: string (required)\n  count: number (required) — how many screenshots\n  interval_ms: number (required) — milliseconds between captures\n  format: \"png\" | \"jpeg\" | \"webp\" (default: \"png\")\n  quality: number 0-100 (optional)\nReturns: Multiple images\nExample: {\"instance_id\": \"dev\", \"count\": 3, \"interval_ms\": 1000}"),
            ("vscreen_screenshot_annotated", "Screenshot with numbered bounding boxes on elements", "Parameters:\n  instance_id: string (required)\n  selector: string (default: \"a, button, input, select, textarea, [role=button], [onclick]\")\nReturns: Annotated screenshot + text legend mapping numbers to elements with {index, tag, text, ariaLabel, role, x, y, width, height, page_x, page_y}\nNote: Only annotates main-frame elements. For iframes, use vscreen_find_by_text(include_iframes=true)."),
            ("vscreen_navigate", "Navigate to a URL", "Parameters:\n  instance_id: string (required)\n  url: string (required)\n  wait_until: \"load\" | \"domcontentloaded\" | \"networkidle\" | \"none\" (default: \"load\")\nReturns: Final URL and page title\nExample: {\"instance_id\": \"dev\", \"url\": \"https://example.com\", \"wait_until\": \"load\"}"),
            ("vscreen_click", "Click at page-space coordinates", "Parameters:\n  instance_id: string (required)\n  x: number (required) — page-space X coordinate\n  y: number (required) — page-space Y coordinate\n  button: 0 | 1 | 2 (default: 0) — 0=left, 1=middle, 2=right\nAuto-scrolls the target into view before clicking.\nCoordinates from vscreen_find_elements/vscreen_find_by_text can be used directly."),
            ("vscreen_double_click", "Double-click at coordinates", "Parameters:\n  instance_id: string (required)\n  x: number (required)\n  y: number (required)\nAuto-scrolls the target into view."),
            ("vscreen_type", "Type text into the currently focused element", "Parameters:\n  instance_id: string (required)\n  text: string (required) — text to type character by character\nTo focus an element first, click on it with vscreen_click.\nFor clearing and filling a field, use vscreen_fill instead."),
            ("vscreen_fill", "Clear and fill an input field by CSS selector", "Parameters:\n  instance_id: string (required)\n  selector: string (required) — CSS selector for the input (e.g., \"input[name='email']\")\n  value: string (required) — value to fill\nClears existing content first, then types the new value.\nExample: {\"instance_id\": \"dev\", \"selector\": \"#username\", \"value\": \"user@example.com\"}"),
            ("vscreen_key_press", "Press a single key", "Parameters:\n  instance_id: string (required)\n  key: string (required) — DOM key name\nKey names: \"Enter\", \"Tab\", \"Escape\", \"Backspace\", \"Delete\", \"ArrowUp\", \"ArrowDown\", \"ArrowLeft\", \"ArrowRight\", \"Home\", \"End\", \"PageUp\", \"PageDown\", \"F1\"-\"F12\", \"Space\""),
            ("vscreen_key_combo", "Press a key combination", "Parameters:\n  instance_id: string (required)\n  keys: [string, string, ...] (required) — keys to press simultaneously\nOrder matters: modifier keys first, then the main key.\nExamples: [\"Control\", \"a\"], [\"Control\", \"c\"], [\"Alt\", \"Tab\"], [\"Control\", \"Shift\", \"i\"]"),
            ("vscreen_scroll", "Scroll by pixel delta", "Parameters:\n  instance_id: string (required)\n  x: number (required) — page position to scroll at\n  y: number (required) — page position to scroll at\n  delta_x: number (default: 0) — horizontal scroll pixels\n  delta_y: number (default: 0) — vertical scroll pixels (positive = scroll down)"),
            ("vscreen_drag", "Drag from one point to another", "Parameters:\n  instance_id: string (required)\n  start_x, start_y: numbers — drag start coordinates\n  end_x, end_y: numbers — drag end coordinates\n  steps: number (default: 10) — interpolation steps"),
            ("vscreen_hover", "Hover at coordinates", "Parameters:\n  instance_id: string (required)\n  x: number (required)\n  y: number (required)\nTriggers CSS :hover states and JavaScript mouseover/mouseenter events."),
            ("vscreen_batch_click", "Click multiple points rapidly in one call", "Parameters:\n  instance_id: string (required)\n  points: [[x1,y1], [x2,y2], ...] (required) — array of [x, y] coordinate pairs\n  delay_between_ms: number (default: 50) — milliseconds between clicks\nIdeal for timed challenges like reCAPTCHA tile grids where individual MCP round-trips would be too slow.\nExample: {\"instance_id\": \"dev\", \"points\": [[135,224],[135,324],[135,424]], \"delay_between_ms\": 200}"),
            ("vscreen_click_element", "Click element by CSS selector or visible text", "Parameters:\n  instance_id: string (required)\n  selector: string (optional) — CSS selector\n  text: string (optional) — visible text to match\n  index: number (default: 0) — which match to click (if multiple)\n  button: 0 | 1 | 2 (default: 0)\nNote: Searches MAIN FRAME ONLY. For iframe elements, use vscreen_find_by_text(include_iframes=true) then vscreen_click."),
            ("vscreen_find_elements", "Find elements by CSS selector", "Parameters:\n  instance_id: string (required)\n  selector: string (required) — CSS selector (e.g., \"button\", \"a.nav-link\", \"input[type=text]\")\n  include_iframes: bool (default: false) — search inside iframes too\nReturns: Array of {tag, text, x, y, width, height, visible, occluded, id, type, value, href, ariaLabel, role, title, tabIndex, svgContent, frame_id}\nCoordinates are in page-space and can be used directly with vscreen_click."),
            ("vscreen_find_by_text", "Find elements by visible text", "Parameters:\n  instance_id: string (required)\n  text: string (required) — text to search for\n  exact: bool (default: false) — exact match vs contains\n  include_iframes: bool (default: false) — search inside iframes too\nReturns: Array of {tag, text, x, y, width, height, frame_id}\nCoordinates are in page-space. Essential for iframe interaction (e.g., reCAPTCHA)."),
            ("vscreen_wait", "Wait a fixed duration", "Parameters:\n  duration_ms: number (required) — milliseconds to wait\nUse between action and observation. Does NOT require instance_id."),
            ("vscreen_wait_for_idle", "Wait for no screencast frames", "Parameters:\n  instance_id: string (required)\n  idle_ms: number (default: 500) — milliseconds of no frames to consider idle\n  timeout_ms: number (default: 10000) — max wait time"),
            ("vscreen_wait_for_text", "Wait for text to appear on page", "Parameters:\n  instance_id: string (required)\n  text: string (required) — text to wait for\n  timeout_ms: number (default: 10000)\n  poll_interval_ms: number (default: 500)"),
            ("vscreen_wait_for_selector", "Wait for a CSS selector to match", "Parameters:\n  instance_id: string (required)\n  selector: string (required)\n  timeout_ms: number (default: 10000)\n  poll_interval_ms: number (default: 500)"),
            ("vscreen_wait_for_url", "Wait for URL to match a pattern", "Parameters:\n  instance_id: string (required)\n  url_contains: string (required)\n  timeout_ms: number (default: 10000)"),
            ("vscreen_wait_for_network_idle", "Wait for network activity to stop", "Parameters:\n  instance_id: string (required)\n  idle_ms: number (default: 500)\n  timeout_ms: number (default: 30000)"),
            ("vscreen_get_page_info", "Get page title, URL, viewport, and scroll position", "Parameters:\n  instance_id: string (required)\nReturns: {url, title, viewport: {width, height}, scrollX, scrollY}"),
            ("vscreen_extract_text", "Extract all visible text from the page", "Parameters:\n  instance_id: string (required)\nReturns: All visible text content from the page body."),
            ("vscreen_execute_js", "Execute JavaScript in the MAIN frame", "Parameters:\n  instance_id: string (required)\n  expression: string (required) — JavaScript to evaluate\nReturns: The expression result as JSON.\nRuns in the MAIN FRAME ONLY — cannot access iframe content.\nExample: {\"instance_id\": \"dev\", \"expression\": \"document.title\"}"),
            ("vscreen_list_frames", "List all frames including iframes", "Parameters:\n  instance_id: string (required)\nReturns: Frame tree (parent-child) + iframe bounding rectangles with page-space coordinates.\nEach iframe includes: src, name, title, x, y, width, height, visible.\nUse the bounding rect with vscreen_screenshot(clip=...) to zoom into an iframe."),
            ("vscreen_dismiss_dialogs", "Auto-dismiss cookie consent, privacy, and GDPR dialogs", "Parameters:\n  instance_id: string (required)\nChecks for OneTrust, CookieBot, Didomi, Quantcast, TrustArc, and many other consent frameworks.\nAlso matches common button text patterns in multiple languages.\nReturns which dialog was dismissed or 'no dialog found'.\nCall this after navigating to a new page if you expect consent overlays."),
            ("vscreen_solve_captcha", "Automatically solve reCAPTCHA v2 image challenges", "Parameters:\n  instance_id: string (required)\n  max_attempts: number (default: 3) — max page-reload retry cycles\nFinds the reCAPTCHA checkbox, clicks it, uses vision LLM to identify tiles, clicks them + VERIFY.\nHandles multi-round challenges, retries, and timeouts internally.\nReturns: {solved: bool, rounds: N, attempts: N, details: [...]}\nRequires vision LLM (--vision-url).\nSee `vscreen_help(topic=\"captcha\")` for the manual workflow fallback."),
            ("vscreen_find_input", "Find text inputs by placeholder, aria-label, label, role, or name", "Parameters:\n  instance_id: string (required)\n  placeholder: string (optional) — match by placeholder text\n  aria_label: string (optional) — match by aria-label\n  label: string (optional) — match by associated <label> text\n  role: string (optional) — match by role attribute (e.g. \"searchbox\")\n  name: string (optional) — match by name attribute\n  input_type: string (optional) — match by input type (e.g. \"email\")\nAt least one search parameter required.\nReturns: Array of matching inputs with selector, type, value, and page coordinates.\nTip: Use vscreen_fill(selector, value) to type into a result."),
            ("vscreen_click_and_navigate", "Click an element and wait for navigation", "Parameters:\n  instance_id: string (required)\n  selector: string (optional) — CSS selector\n  text: string (optional) — visible text match\n  timeout_ms: number (default: 5000) — how long to wait for URL change\n  fallback_to_link: bool (default: true) — try nearest <a> href if click doesn't navigate\nClicks the element, waits for URL change. If URL doesn't change and fallback_to_link is true, tries direct navigation via the nearest <a> tag's href.\nIdeal for SPA navigation (YouTube, React apps) where clicks trigger pushState."),
            ("vscreen_dismiss_ads", "Dismiss video platform ad overlays", "Parameters:\n  instance_id: string (required)\n  timeout_ms: number (default: 15000) — how long to wait for skip button\nDetects and dismisses YouTube skip buttons, pre-roll ad overlays, and generic close buttons.\nHandles localized skip button text (English, German, French, Spanish, Portuguese, Russian).\nWaits and polls for skip button to appear since video ads have a countdown."),
            ("vscreen_select_option", "Select a dropdown option", "Parameters:\n  instance_id: string (required)\n  selector: string (required) — CSS selector for the <select> element\n  value: string (optional) — option value attribute to select\n  label: string (optional) — visible text of the option to select\nProvide either value OR label, not both."),
            ("vscreen_scroll_to_element", "Scroll an element into view", "Parameters:\n  instance_id: string (required)\n  selector: string (required) — CSS selector\n  block: \"center\" | \"start\" | \"end\" | \"nearest\" (default: \"center\")"),
            ("vscreen_accessibility_tree", "Get the accessibility tree", "Parameters:\n  instance_id: string (required)\nReturns: Structured accessibility tree representation of the page."),
            ("vscreen_describe_elements", "Identify unlabeled UI elements using vision", "Parameters:\n  instance_id: string (required)\n  selector: string (default: \"button, [role='button'], a, input, select\") — CSS selector\n  include_labeled: bool (default: false) — if true, describes ALL elements\nUses vision LLM to identify icon-only buttons and other elements that lack text/aria-label.\nReturns: Array of elements with AI-generated descriptions: {tag, x, y, width, height, icon, action, label}\nRequires vision LLM to be configured (--vision-url)."),
            ("vscreen_go_back", "Navigate back in browser history", "Parameters:\n  instance_id: string (required)"),
            ("vscreen_go_forward", "Navigate forward in browser history", "Parameters:\n  instance_id: string (required)"),
            ("vscreen_reload", "Reload the current page", "Parameters:\n  instance_id: string (required)"),
            ("vscreen_get_cookies", "Get cookies for the current page", "Parameters:\n  instance_id: string (required)\nReturns: Array of cookies with name, value, domain, path, expiry."),
            ("vscreen_set_cookie", "Set a browser cookie", "Parameters:\n  instance_id: string (required)\n  name: string (required)\n  value: string (required)\n  domain: string (optional)\n  path: string (optional)\n  expires: number (optional) — Unix timestamp"),
            ("vscreen_get_storage", "Get localStorage or sessionStorage", "Parameters:\n  instance_id: string (required)\n  storage_type: \"local\" | \"session\" (default: \"local\")\n  key: string (optional) — specific key, or omit for all"),
            ("vscreen_set_storage", "Set localStorage or sessionStorage", "Parameters:\n  instance_id: string (required)\n  storage_type: \"local\" | \"session\" (default: \"local\")\n  key: string (required)\n  value: string (required)"),
            ("vscreen_history_list", "List screenshot history entries", "Parameters:\n  instance_id: string (required)\nReturns: List of historical screenshot metadata (index, timestamp, action)."),
            ("vscreen_history_get", "Get a specific historical screenshot", "Parameters:\n  instance_id: string (required)\n  index: number (required) — 0 = oldest\nReturns: The screenshot image from history."),
            ("vscreen_history_get_range", "Get a range of historical screenshots", "Parameters:\n  instance_id: string (required)\n  start: number (required)\n  end: number (required)\nReturns: Multiple screenshots from history."),
            ("vscreen_history_clear", "Clear screenshot history", "Parameters:\n  instance_id: string (required)"),
            ("vscreen_session_log", "Get recent action log", "Parameters:\n  instance_id: string (required)\n  last_n: number (optional) — number of recent actions to return"),
            ("vscreen_session_summary", "Get session summary", "Parameters:\n  instance_id: string (required)\nReturns: Action count, duration, screenshot count, and other session metrics."),
            ("vscreen_console_log", "Get captured browser console messages", "Parameters:\n  instance_id: string (required)\n  last_n: number (optional) — number of recent messages"),
            ("vscreen_console_clear", "Clear captured console messages", "Parameters:\n  instance_id: string (required)"),
            ("vscreen_instance_lock", "Acquire a lock on an instance", "Parameters:\n  instance_id: string (required)\n  lock_type: \"exclusive\" | \"observer\" (default: \"exclusive\")\n  ttl_seconds: number (default: 300) — lock auto-expires after this\n  wait_timeout_seconds: number (default: 0) — wait this long if locked (0 = fail immediately)\nReturns: lock_token (save for unlock/renew)\nIn single-agent mode, locks are auto-acquired."),
            ("vscreen_instance_unlock", "Release a lock", "Parameters:\n  instance_id: string (required)\n  lock_token: string (required) — from lock acquisition"),
            ("vscreen_instance_lock_status", "Check lock status", "Parameters:\n  instance_id: string (optional) — omit for all instances\nReturns: Lock holder, type, expiry, and queue depth."),
            ("vscreen_instance_lock_renew", "Renew/extend a lock", "Parameters:\n  instance_id: string (required)\n  lock_token: string (required)\n  ttl_seconds: number (default: 300)"),
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

// ---------------------------------------------------------------------------
// Tool parameter types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct InstanceIdParam {
    /// The instance ID to operate on (e.g. "dev")
    instance_id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct ClipRect {
    /// X coordinate of clip region (page coordinates)
    x: f64,
    /// Y coordinate of clip region (page coordinates)
    y: f64,
    /// Width of clip region in pixels
    width: f64,
    /// Height of clip region in pixels
    height: f64,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct ScreenshotParam {
    /// The instance ID to operate on
    instance_id: String,
    /// Image format: "png" (default, best for AI vision), "jpeg", or "webp"
    #[serde(default = "default_png")]
    format: String,
    /// JPEG/WebP quality (0-100). Ignored for PNG.
    quality: Option<u32>,
    /// If true, captures the full scrollable page (not just the visible viewport).
    /// This temporarily resizes the browser viewport to the full document height.
    /// Useful for seeing all content on a page without scrolling.
    #[serde(default)]
    full_page: bool,
    /// Optional clip rectangle to capture only a specific region at full resolution.
    /// Useful for analyzing small areas (e.g. CAPTCHA grids) without scaling artifacts.
    #[serde(default)]
    clip: Option<ClipRect>,
}

fn default_png() -> String {
    "png".into()
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct ScreenshotSequenceParam {
    /// The instance ID to operate on
    instance_id: String,
    /// Number of screenshots to capture
    count: u32,
    /// Milliseconds between each capture
    interval_ms: u64,
    /// Image format: "png" (default), "jpeg", or "webp"
    #[serde(default = "default_png")]
    format: String,
    /// JPEG/WebP quality (0-100)
    quality: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct NavigateParam {
    /// The instance ID to operate on
    instance_id: String,
    /// URL to navigate to (e.g. "https://example.com")
    url: String,
    /// When to consider navigation complete: "none" (return immediately), "load" (wait for page load, default), "domcontentloaded", "networkidle"
    #[serde(default)]
    wait_until: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct ClickParam {
    /// The instance ID to operate on
    instance_id: String,
    /// X coordinate in pixels from the left edge of the page (works with both viewport and full-page screenshot coordinates)
    x: f64,
    /// Y coordinate in pixels from the top of the page (works with both viewport and full-page screenshot coordinates — auto-scrolls if needed)
    y: f64,
    /// Mouse button: 0=left (default), 1=middle, 2=right
    #[serde(default)]
    button: Option<u8>,
    /// Wait this many milliseconds after clicking for the page to react (navigation, rendering, animations). If set, returns page URL and title after the wait.
    #[serde(default)]
    wait_after_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct DoubleClickParam {
    /// The instance ID to operate on
    instance_id: String,
    /// X coordinate in pixels from the left edge of the page (auto-scrolls if needed)
    x: f64,
    /// Y coordinate in pixels from the top of the page (auto-scrolls if needed)
    y: f64,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct TypeParam {
    /// The instance ID to operate on
    instance_id: String,
    /// Text to type/paste into the focused element
    text: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct KeyPressParam {
    /// The instance ID to operate on
    instance_id: String,
    /// Key name (e.g. "Enter", "Escape", "Tab", "Backspace", "ArrowDown", "a", "1")
    key: String,
    /// Whether to hold Ctrl
    #[serde(default)]
    ctrl: bool,
    /// Whether to hold Shift
    #[serde(default)]
    shift: bool,
    /// Whether to hold Alt
    #[serde(default)]
    alt: bool,
    /// Whether to hold Meta/Win
    #[serde(default)]
    meta: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct KeyComboParam {
    /// The instance ID to operate on
    instance_id: String,
    /// Keys to press simultaneously (e.g. ["Control", "a"] for Ctrl+A). Last key is the action key.
    keys: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct ScrollParam {
    /// The instance ID to operate on
    instance_id: String,
    /// X coordinate of scroll position (page coordinates, auto-scrolls if needed)
    x: f64,
    /// Y coordinate of scroll position (page coordinates, auto-scrolls if needed)
    y: f64,
    /// Horizontal scroll amount (positive=right)
    #[serde(default)]
    delta_x: f64,
    /// Vertical scroll amount (positive=down, negative=up). Typical: 120 per notch.
    delta_y: f64,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct DragParam {
    /// The instance ID to operate on
    instance_id: String,
    /// Start X coordinate (page coordinates, auto-scrolls to start position)
    from_x: f64,
    /// Start Y coordinate (page coordinates, auto-scrolls to start position)
    from_y: f64,
    /// End X coordinate (page coordinates)
    to_x: f64,
    /// End Y coordinate (page coordinates)
    to_y: f64,
    /// Number of intermediate mouse-move steps (default: 10)
    #[serde(default)]
    steps: Option<u32>,
    /// Duration of drag in milliseconds (default: 300)
    #[serde(default)]
    duration_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct HoverParam {
    /// The instance ID to operate on
    instance_id: String,
    /// X coordinate to move mouse to (page coordinates, auto-scrolls if needed)
    x: f64,
    /// Y coordinate to move mouse to (page coordinates, auto-scrolls if needed)
    y: f64,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct WaitParam {
    /// Duration to wait in milliseconds
    duration_ms: u64,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct WaitForIdleParam {
    /// The instance ID to operate on
    instance_id: String,
    /// Maximum time to wait in milliseconds (default: 5000)
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct ExecuteJsParam {
    /// The instance ID to operate on
    instance_id: String,
    /// JavaScript expression to evaluate
    expression: String,
}

// -- Phase 1a: Screenshot history params --

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct HistoryListParam {
    /// The instance ID to operate on
    instance_id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct HistoryGetParam {
    /// The instance ID to operate on
    instance_id: String,
    /// Index in the history buffer (0 = oldest still in buffer)
    index: usize,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct HistoryGetRangeParam {
    /// The instance ID to operate on
    instance_id: String,
    /// Start index (0 = oldest)
    from: usize,
    /// Number of screenshots to return
    count: usize,
}

// -- Phase 1b: Action log params --

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct SessionLogParam {
    /// The instance ID to operate on
    instance_id: String,
    /// Maximum number of recent entries to return (default: all)
    #[serde(default)]
    last_n: Option<usize>,
}

// -- Phase 2a: Element discovery params --

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct FindElementsParam {
    /// The instance ID to operate on
    instance_id: String,
    /// CSS selector to query (e.g. "button", "a.nav-link", "#submit")
    selector: String,
    /// If true, also search inside iframes (default: false). Only searches the main frame by default.
    #[serde(default)]
    include_iframes: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct FindByTextParam {
    /// The instance ID to operate on
    instance_id: String,
    /// Text to search for in visible page content
    text: String,
    /// If true, match the exact full text; otherwise match as substring (default)
    #[serde(default)]
    exact: bool,
    /// If true, also search inside iframes (default: false). Only searches the main frame by default.
    #[serde(default)]
    include_iframes: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct DescribeElementsParam {
    /// The instance ID to operate on
    instance_id: String,
    /// CSS selector to query (default: "button, [role='button'], a, input, select")
    selector: Option<String>,
    /// If true, describe ALL elements including those with labels (default: false — only unlabeled)
    #[serde(default)]
    include_labeled: bool,
}

// -- Phase 3a/3b: Wait conditions params --

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct WaitForTextParam {
    /// The instance ID to operate on
    instance_id: String,
    /// Text to wait for on the page
    text: String,
    /// Maximum time to wait in milliseconds (default: 10000)
    #[serde(default)]
    timeout_ms: Option<u64>,
    /// Polling interval in milliseconds (default: 250)
    #[serde(default)]
    interval_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct WaitForSelectorParam {
    /// The instance ID to operate on
    instance_id: String,
    /// CSS selector to wait for
    selector: String,
    /// If true, also require the element to be visible (default: false)
    #[serde(default)]
    visible: bool,
    /// Maximum time to wait in milliseconds (default: 10000)
    #[serde(default)]
    timeout_ms: Option<u64>,
    /// Polling interval in milliseconds (default: 250)
    #[serde(default)]
    interval_ms: Option<u64>,
}

// -- Phase 4a: Navigation params --

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct ExtractTextParam {
    /// The instance ID to operate on
    instance_id: String,
    /// Optional CSS selector to extract text from. If omitted, extracts full page text.
    #[serde(default)]
    selector: Option<String>,
}

// -- Phase 1c: Console params --

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct ConsoleLogParam {
    /// The instance ID to operate on
    instance_id: String,
    /// Filter by level: "log", "warn", "error", "info". If omitted, returns all.
    #[serde(default)]
    level: Option<String>,
}

// -- Phase 2c: Accessibility tree params --

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct AccessibilityTreeParam {
    /// The instance ID to operate on
    instance_id: String,
    /// Maximum depth to traverse (default: 5)
    #[serde(default)]
    max_depth: Option<u32>,
}

// -- Phase 4c: Cookie/Storage params --

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct SetCookieParam {
    /// The instance ID to operate on
    instance_id: String,
    /// Cookie name
    name: String,
    /// Cookie value
    value: String,
    /// Cookie domain (defaults to current page domain)
    #[serde(default)]
    domain: Option<String>,
    /// Cookie path (defaults to "/")
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct StorageGetParam {
    /// The instance ID to operate on
    instance_id: String,
    /// Storage key to read
    key: String,
    /// "local" for localStorage (default), "session" for sessionStorage
    #[serde(default = "default_local")]
    storage_type: String,
}

fn default_local() -> String {
    "local".into()
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct StorageSetParam {
    /// The instance ID to operate on
    instance_id: String,
    /// Storage key to set
    key: String,
    /// Value to store
    value: String,
    /// "local" for localStorage (default), "session" for sessionStorage
    #[serde(default = "default_local")]
    storage_type: String,
}

// -- Phase 3c/3d: Advanced wait params --

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct WaitForUrlParam {
    /// The instance ID to operate on
    instance_id: String,
    /// Substring that the URL must contain
    url_contains: String,
    /// Maximum time to wait in milliseconds (default: 10000)
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct WaitForNetworkIdleParam {
    /// The instance ID to operate on
    instance_id: String,
    /// Required idle duration in milliseconds (default: 500)
    #[serde(default)]
    idle_ms: Option<u64>,
    /// Maximum time to wait in milliseconds (default: 10000)
    #[serde(default)]
    timeout_ms: Option<u64>,
}

// -- Phase 2d: Annotated screenshot params --

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct AnnotatedScreenshotParam {
    /// The instance ID to operate on
    instance_id: String,
    /// CSS selector for elements to annotate (default: interactive elements — a, button, input, select, textarea, [role=button])
    #[serde(default)]
    selector: Option<String>,
    /// If true, also annotate elements inside iframes (default: false). Only annotates the main frame by default.
    #[serde(default)]
    include_iframes: bool,
}

// -- New high-impact tools params --

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct ClickElementParam {
    /// The instance ID to operate on
    instance_id: String,
    /// CSS selector to find the element to click (e.g. "button.submit", "#login")
    #[serde(default)]
    selector: Option<String>,
    /// Visible text to find and click. Searches all visible elements.
    #[serde(default)]
    text: Option<String>,
    /// If true, match text exactly; otherwise match as substring (default: false)
    #[serde(default)]
    text_exact: bool,
    /// Which matching element to click (0 = first match, default)
    #[serde(default)]
    index: Option<usize>,
    /// Mouse button: 0=left (default), 1=middle, 2=right
    #[serde(default)]
    button: Option<u8>,
    /// Wait this many milliseconds after clicking for the page to react (navigation, rendering, animations). If set, returns page URL and title after the wait.
    #[serde(default)]
    wait_after_ms: Option<u64>,
    /// Number of retries if element is not found (default: 0 = no retries). Useful for elements that appear after dynamic loading.
    #[serde(default)]
    retries: Option<u32>,
    /// Delay between retries in milliseconds (default: 500)
    #[serde(default)]
    retry_delay_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct BatchClickParam {
    /// The instance ID to operate on
    instance_id: String,
    /// Array of [x, y] coordinate pairs to click in sequence
    points: Vec<[f64; 2]>,
    /// Delay between clicks in milliseconds (default: 50)
    #[serde(default)]
    delay_between_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct DismissDialogsParam {
    /// The instance ID to operate on
    instance_id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct SolveCaptchaParam {
    /// The instance ID to operate on
    instance_id: String,
    /// Maximum number of full page-reload attempts before giving up (default: 3)
    #[serde(default)]
    max_attempts: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct FillParam {
    /// The instance ID to operate on
    instance_id: String,
    /// CSS selector for the input element to fill
    selector: String,
    /// Text value to fill into the element
    value: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct SelectOptionParam {
    /// The instance ID to operate on
    instance_id: String,
    /// CSS selector for the select element
    selector: String,
    /// Option value attribute to select
    #[serde(default)]
    value: Option<String>,
    /// Option visible text (label) to select
    #[serde(default)]
    label: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct ScrollToElementParam {
    /// The instance ID to operate on
    instance_id: String,
    /// CSS selector for the element to scroll into view
    selector: String,
    /// Scroll alignment: "center" (default), "start", "end", "nearest"
    #[serde(default)]
    block: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct FindInputParam {
    /// The instance ID to operate on
    instance_id: String,
    /// Search by placeholder text (substring match, case-insensitive)
    #[serde(default)]
    placeholder: Option<String>,
    /// Search by aria-label attribute (substring match, case-insensitive)
    #[serde(default)]
    aria_label: Option<String>,
    /// Search by associated label text (substring match, case-insensitive)
    #[serde(default)]
    label: Option<String>,
    /// Search by role attribute (e.g. "textbox", "searchbox", "combobox")
    #[serde(default)]
    role: Option<String>,
    /// Search by name attribute (exact match)
    #[serde(default)]
    name: Option<String>,
    /// Search by input type (e.g. "text", "email", "password", "search")
    #[serde(default)]
    input_type: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct ClickAndNavigateParam {
    /// The instance ID to operate on
    instance_id: String,
    /// CSS selector of the element to click
    #[serde(default)]
    selector: Option<String>,
    /// Visible text of the element to click (substring match)
    #[serde(default)]
    text: Option<String>,
    /// Timeout in milliseconds to wait for URL change after clicking (default: 5000)
    #[serde(default)]
    timeout_ms: Option<u64>,
    /// If true and the initial click doesn't navigate, try clicking the nearest <a> ancestor
    #[serde(default = "default_true")]
    fallback_to_link: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct DismissAdsParam {
    /// The instance ID to operate on
    instance_id: String,
    /// Timeout in milliseconds to wait for skip button to appear (default: 15000). Video ads often have a countdown before the skip button appears.
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct ListFramesParam {
    /// The instance ID to operate on
    instance_id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct PlanTaskParam {
    /// Short description of the task you want to accomplish, e.g. "read all text on the page",
    /// "click the Sign In button", "fill out a login form", "navigate to a URL and extract data".
    task: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct HelpParam {
    /// Topic to get help on. Can be a tool name (e.g., "vscreen_batch_click"),
    /// a concept ("coordinates", "iframes", "locking", "workflows"),
    /// or "tools" for the full tool reference. Use "quickstart" for a getting-started guide.
    topic: String,
}

// -- Lock management params --

fn default_lock_type() -> String {
    "exclusive".into()
}

fn default_ttl() -> u64 {
    120
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct LockParam {
    /// The instance ID to lock (e.g. "dev")
    instance_id: String,
    /// Lock type: "exclusive" (default, full control) or "observer" (read-only, coexists with other observers)
    #[serde(default = "default_lock_type")]
    lock_type: String,
    /// Optional human-readable name for this agent (shown to other agents in lock-denied messages)
    #[serde(default)]
    agent_name: Option<String>,
    /// Lock TTL in seconds (default: 120). The lock expires after this duration unless renewed.
    #[serde(default = "default_ttl")]
    ttl_seconds: u64,
    /// Seconds to wait if the lock is held by another session (0 = fail immediately, default). Set > 0 to queue.
    #[serde(default)]
    wait_timeout_seconds: u64,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct UnlockParam {
    /// The instance ID to unlock
    instance_id: String,
    /// Lock token returned by vscreen_instance_lock. Pass this if your session ID has changed since acquiring the lock.
    #[serde(default)]
    lock_token: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct LockStatusParam {
    /// Instance ID to query (omit to get status of all instances)
    #[serde(default)]
    instance_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct LockRenewParam {
    /// The instance ID whose lock to renew
    instance_id: String,
    /// New TTL in seconds (default: 120)
    #[serde(default = "default_ttl")]
    ttl_seconds: u64,
    /// Lock token returned by vscreen_instance_lock. Pass this if your session ID has changed since acquiring the lock.
    #[serde(default)]
    lock_token: Option<String>,
}

// -- Audio / RTSP params --

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct AudioStreamsParam {
    /// The instance ID to list audio streams for
    instance_id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct AudioStreamInfoParam {
    /// The instance ID
    instance_id: String,
    /// The RTSP session ID to get info for
    session_id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct AudioHealthParam {
    /// The instance ID to get audio health for
    instance_id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct RtspTeardownParam {
    /// The instance ID
    instance_id: String,
    /// The RTSP session ID to tear down
    session_id: String,
}

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

#[tool_router]
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
    // Lock management tools
    // -----------------------------------------------------------------------

    #[tool(description = "Acquire a lock on a browser instance. Locks are auto-acquired when needed, so explicit locking is optional for single-agent use. Use explicit locks when multiple agents share instances.\n\n- 'exclusive' lock (default): full control (click, type, navigate, etc.). Only one exclusive holder at a time.\n- 'observer' lock: read-only access (screenshot, get_page_info, etc.). Multiple observers can coexist, but not with an exclusive lock.\n\nReturns a lock_token that can be used to release/renew the lock even if the session ID changes. Locks expire after ttl_seconds (default: 120). Call vscreen_instance_lock_renew to extend. Set wait_timeout_seconds > 0 to queue if another session holds the lock.")]
    async fn vscreen_instance_lock(
        &self,
        Parameters(params): Parameters<LockParam>,
    ) -> Result<CallToolResult, McpError> {
        let lock_type = Self::parse_lock_type(&params.lock_type)?;
        let ttl = Duration::from_secs(params.ttl_seconds.max(1));
        let instance_id = InstanceId::from(params.instance_id.as_str());

        let result = if params.wait_timeout_seconds > 0 {
            self.state
                .lock_manager
                .acquire_or_wait(
                    &instance_id,
                    &self.session_id,
                    params.agent_name,
                    lock_type,
                    ttl,
                    Duration::from_secs(params.wait_timeout_seconds),
                )
                .await
        } else {
            self.state.lock_manager.acquire(
                &instance_id,
                &self.session_id,
                params.agent_name,
                lock_type,
                ttl,
            )
        };

        match result {
            Ok(info) => {
                let text = serde_json::to_string_pretty(&info).unwrap_or_default();
                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Lock acquired on instance '{}'.\n{text}",
                    params.instance_id
                ))]))
            }
            Err(e) => Err(lock_error_to_mcp(&params.instance_id, e)),
        }
    }

    #[tool(description = "Release your lock on a browser instance. If other sessions are waiting in the queue, the next one will be promoted automatically. You can pass a lock_token if your session has changed since acquiring the lock.")]
    async fn vscreen_instance_unlock(
        &self,
        Parameters(params): Parameters<UnlockParam>,
    ) -> Result<CallToolResult, McpError> {
        let instance_id = InstanceId::from(params.instance_id.as_str());
        let token = params.lock_token.as_deref().and_then(LockToken::parse);
        match self.state.lock_manager.release_with_token(&instance_id, &self.session_id, token.as_ref()) {
            Ok(promoted) => {
                let extra = if promoted {
                    " Next session in queue has been promoted."
                } else {
                    ""
                };
                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Lock released on instance '{}'.{extra}",
                    params.instance_id
                ))]))
            }
            Err(e) => Err(lock_error_to_mcp(&params.instance_id, e)),
        }
    }

    #[tool(description = "Query the lock status of one or all browser instances. Shows the current exclusive holder, observers, and wait queue.")]
    async fn vscreen_instance_lock_status(
        &self,
        Parameters(params): Parameters<LockStatusParam>,
    ) -> Result<CallToolResult, McpError> {
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

    #[tool(description = "Renew (extend) your lock's TTL on a browser instance. Call this periodically to prevent expiration. Returns the updated lock info with new expiry time. You can pass a lock_token if your session has changed.")]
    async fn vscreen_instance_lock_renew(
        &self,
        Parameters(params): Parameters<LockRenewParam>,
    ) -> Result<CallToolResult, McpError> {
        let instance_id = InstanceId::from(params.instance_id.as_str());
        let ttl = Duration::from_secs(params.ttl_seconds.max(1));
        let token = params.lock_token.as_deref().and_then(LockToken::parse);
        match self.state.lock_manager.renew_with_token(&instance_id, &self.session_id, token.as_ref(), ttl) {
            Ok(info) => {
                let text = serde_json::to_string_pretty(&info).unwrap_or_default();
                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Lock renewed on instance '{}'.\n{text}",
                    params.instance_id
                ))]))
            }
            Err(e) => Err(lock_error_to_mcp(&params.instance_id, e)),
        }
    }

    // -----------------------------------------------------------------------
    // Instance tools (require locks)
    // -----------------------------------------------------------------------

    #[tool(description = "Capture a screenshot of the browser instance. Returns a PNG image by default. Use this to observe the current state of the webpage. Set full_page=true to capture the entire scrollable document, not just the visible viewport. Set clip={x,y,width,height} to capture a specific region at full resolution (useful for analyzing small areas like CAPTCHA grids). Recommended flow: perform action → wait → screenshot → observe. PREFER full_page=true over scroll+screenshot loops. PREFER vscreen_extract_text when you only need to read page text.")]
    async fn vscreen_screenshot(
        &self,
        Parameters(params): Parameters<ScreenshotParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        let data = if params.full_page {
            sup.capture_full_page_screenshot(&params.format, params.quality)
                .await
        } else if let Some(ref clip) = params.clip {
            sup.capture_screenshot_clip(
                &params.format,
                params.quality,
                Some((clip.x, clip.y, clip.width, clip.height)),
            )
            .await
        } else {
            sup.capture_screenshot(&params.format, params.quality)
                .await
        }
        .map_err(|e| internal_error(e.to_string()))?;

        let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
        let mime = match params.format.as_str() {
            "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            _ => "image/png",
        };

        Ok(CallToolResult::success(vec![Content::image(b64, mime)]))
    }

    #[tool(description = "Capture a sequence of screenshots at a fixed interval. Useful for observing page transitions, animations, or verifying an action completed.")]
    async fn vscreen_screenshot_sequence(
        &self,
        Parameters(params): Parameters<ScreenshotSequenceParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        let images = sup
            .capture_screenshot_sequence(
                params.count,
                params.interval_ms,
                &params.format,
                params.quality,
            )
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        let mime = match params.format.as_str() {
            "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            _ => "image/png",
        };

        let content: Vec<Content> = images
            .iter()
            .map(|img| {
                let b64 = base64::engine::general_purpose::STANDARD.encode(img.as_ref());
                Content::image(b64, mime)
            })
            .collect();

        Ok(CallToolResult::success(content))
    }

    #[tool(description = "Navigate the browser instance to a URL. By default waits for page load before returning. Set wait_until to control: 'load' (default, waits for full load), 'domcontentloaded' (faster, DOM ready), 'networkidle' (waits for network quiet), 'none' (returns immediately). Returns the final URL and title (useful for detecting redirects).")]
    async fn vscreen_navigate(
        &self,
        Parameters(params): Parameters<NavigateParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        sup.navigate(&params.url)
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        let wait_until = params.wait_until.as_deref().unwrap_or("load");
        match wait_until {
            "none" => {}
            "domcontentloaded" => {
                let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
                loop {
                    let state = sup.evaluate_js("document.readyState").await;
                    if let Ok(v) = state {
                        let s = v.as_str().unwrap_or("");
                        if s == "interactive" || s == "complete" {
                            break;
                        }
                    }
                    if tokio::time::Instant::now() >= deadline { break; }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }
            "networkidle" => {
                let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
                // First wait for readyState complete
                loop {
                    let state = sup.evaluate_js("document.readyState").await;
                    if let Ok(v) = state {
                        if v.as_str().unwrap_or("") == "complete" { break; }
                    }
                    if tokio::time::Instant::now() >= deadline { break; }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                // Then wait for network idle (500ms no activity)
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            _ => {
                // "load" — wait for document.readyState === 'complete'
                let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
                loop {
                    let state = sup.evaluate_js("document.readyState").await;
                    if let Ok(v) = state {
                        if v.as_str().unwrap_or("") == "complete" { break; }
                    }
                    if tokio::time::Instant::now() >= deadline { break; }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }
        }

        // Re-install console capture after navigation (page JS context resets)
        let _ = sup.enable_console_capture().await;

        // Get final page info
        let page_info = sup.get_page_info().await.unwrap_or(serde_json::Value::Null);
        let final_url = page_info.get("url").and_then(|v| v.as_str()).unwrap_or(&params.url);
        let title = page_info.get("title").and_then(|v| v.as_str()).unwrap_or("");

        self.record_action(&params.instance_id, "navigate", &params.url, &format!("Navigated to {}", params.url)).await;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Navigated to {final_url}\nTitle: {title}"
        ))]))
    }

    #[tool(description = "Get current page information: URL, title, and viewport dimensions. PREFER this over vscreen_execute_js for page metadata — no JavaScript needed.")]
    async fn vscreen_get_page_info(
        &self,
        Parameters(params): Parameters<InstanceIdParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        let info = sup
            .get_page_info()
            .await
            .map_err(|e| internal_error(e.to_string()))?;
        let text = serde_json::to_string_pretty(&info).unwrap_or_default();
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "Click at the specified coordinates. Coordinates can be from either a viewport or full-page screenshot — the system automatically scrolls to bring the target into view if needed. After clicking, wait ~500ms then take a screenshot to verify the result. PREFER vscreen_click_element for main-frame elements with known selector/text. Use vscreen_click only with coordinates from find tools or for iframe elements.")]
    async fn vscreen_click(
        &self,
        Parameters(params): Parameters<ClickParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(&params.instance_id)?;
        let sup_arc = self.get_supervisor(&params.instance_id)?;

        // Translate page coordinates to viewport-relative (scrolls if needed)
        let (vx, vy) = sup_arc
            .scroll_into_view_and_translate(params.x, params.y)
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        let btn = params.button.unwrap_or(0);
        let btn_name = match btn {
            1 => "middle",
            2 => "right",
            _ => "left",
        };

        for (method, typ, buttons) in [
            ("Input.dispatchMouseEvent", "mouseMoved", 0u32),
            ("Input.dispatchMouseEvent", "mousePressed", 1u32 << btn),
            ("Input.dispatchMouseEvent", "mouseReleased", 0u32),
        ] {
            let mut p = serde_json::json!({
                "type": typ, "x": vx, "y": vy, "modifiers": 0
            });
            if typ != "mouseMoved" {
                p["button"] = serde_json::json!(btn_name);
                p["buttons"] = serde_json::json!(buttons);
                p["clickCount"] = serde_json::json!(1);
            }
            sup_arc.send_cdp_command(method, Some(p))
                .await
                .map_err(|e| internal_error(e.to_string()))?;
        }

        self.record_action(&params.instance_id, "click", &format!("({}, {})", params.x, params.y), &format!("Clicked at ({}, {})", params.x, params.y)).await;

        let mut msg = format!(
            "Clicked at page ({}, {}), viewport ({:.0}, {:.0})",
            params.x, params.y, vx, vy
        );

        if let Some(wait_ms) = params.wait_after_ms {
            tokio::time::sleep(Duration::from_millis(wait_ms)).await;
            if let Ok(info) = sup_arc.evaluate_js("JSON.stringify({url: location.href, title: document.title})").await {
                if let Some(s) = info.as_str() {
                    if let Ok(page_info) = serde_json::from_str::<serde_json::Value>(s) {
                        let url = page_info.get("url").and_then(|v| v.as_str()).unwrap_or("?");
                        let title = page_info.get("title").and_then(|v| v.as_str()).unwrap_or("?");
                        msg.push_str(&format!("\nAfter {wait_ms}ms — URL: {url}\nTitle: {title}"));
                    }
                }
            }
        }

        if let Some(analysis) = self.vision_verify_action(
            &sup_arc, None, crate::vision::prompts::VERIFY_CLICK
        ).await {
            msg.push_str(&format!("\nVision: {analysis}"));
        }

        Ok(CallToolResult::success(vec![Content::text(msg)]))
    }

    #[tool(description = "Double-click at the specified coordinates. Coordinates can be from either a viewport or full-page screenshot — the system automatically scrolls to bring the target into view if needed. Useful for selecting words, opening items, etc.")]
    async fn vscreen_double_click(
        &self,
        Parameters(params): Parameters<DoubleClickParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(&params.instance_id)?;
        let sup_arc = self.get_supervisor(&params.instance_id)?;

        let (vx, vy) = sup_arc
            .scroll_into_view_and_translate(params.x, params.y)
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        for click_count in [1, 2] {
            for typ in ["mousePressed", "mouseReleased"] {
                let buttons = if typ == "mousePressed" { 1u32 } else { 0u32 };
                sup_arc.send_cdp_command(
                    "Input.dispatchMouseEvent",
                    Some(serde_json::json!({
                        "type": typ, "x": vx, "y": vy,
                        "button": "left", "buttons": buttons,
                        "clickCount": click_count, "modifiers": 0
                    })),
                )
                .await
                .map_err(|e| internal_error(e.to_string()))?;
            }
        }

        self.record_action(&params.instance_id, "double_click", &format!("({}, {})", params.x, params.y), &format!("Double-clicked at ({}, {})", params.x, params.y)).await;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Double-clicked at page ({}, {}), viewport ({:.0}, {:.0})",
            params.x, params.y, vx, vy
        ))]))
    }

    #[tool(description = "Type text into the currently focused element. This inserts text directly (like paste), not character-by-character. Click on an input field first to focus it.")]
    async fn vscreen_type(
        &self,
        Parameters(params): Parameters<TypeParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        let event = InputEvent::Paste {
            text: params.text.clone(),
        };
        sup.dispatch_api_input(&event)
            .await
            .map_err(|e| internal_error(e.to_string()))?;
        let text_preview: String = params.text.chars().take(30).collect();
        self.record_action(&params.instance_id, "type", &text_preview, &format!("Typed {} chars", params.text.len())).await;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Typed {} characters",
            params.text.len()
        ))]))
    }

    #[tool(description = "Press a single key, optionally with modifier keys held. Use key names like: Enter, Escape, Tab, Backspace, Delete, ArrowUp, ArrowDown, ArrowLeft, ArrowRight, Home, End, PageUp, PageDown, F1-F12, or single characters like 'a', '1'.")]
    async fn vscreen_key_press(
        &self,
        Parameters(params): Parameters<KeyPressParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        let mut m = 0u8;
        if params.alt {
            m |= 1;
        }
        if params.ctrl {
            m |= 2;
        }
        if params.meta {
            m |= 4;
        }
        if params.shift {
            m |= 8;
        }
        let code = params.key.clone();

        let down = InputEvent::KeyDown {
            key: params.key.clone(),
            code: code.clone(),
            m,
        };
        sup.dispatch_api_input(&down)
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        let up = InputEvent::KeyUp {
            key: params.key.clone(),
            code,
            m,
        };
        sup.dispatch_api_input(&up)
            .await
            .map_err(|e| internal_error(e.to_string()))?;
        self.record_action(&params.instance_id, "key_press", &params.key, &format!("Pressed {}", params.key)).await;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Pressed key: {}",
            params.key
        ))]))
    }

    #[tool(description = "Press a key combination. Keys are pressed in order and released in reverse. Example: [\"Control\", \"a\"] for Ctrl+A, [\"Control\", \"Shift\", \"i\"] for Ctrl+Shift+I.")]
    async fn vscreen_key_combo(
        &self,
        Parameters(params): Parameters<KeyComboParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(&params.instance_id)?;
        if params.keys.is_empty() {
            return Err(invalid_params("keys array must not be empty"));
        }

        let sup = self.get_supervisor(&params.instance_id)?;

        for key in &params.keys {
            let down = InputEvent::KeyDown {
                key: key.clone(),
                code: key.clone(),
                m: 0,
            };
            sup.dispatch_api_input(&down)
                .await
                .map_err(|e| internal_error(e.to_string()))?;
        }

        for key in params.keys.iter().rev() {
            let up = InputEvent::KeyUp {
                key: key.clone(),
                code: key.clone(),
                m: 0,
            };
            sup.dispatch_api_input(&up)
                .await
                .map_err(|e| internal_error(e.to_string()))?;
        }

        let combo_str = params.keys.join("+");
        self.record_action(&params.instance_id, "key_combo", &combo_str, &format!("Pressed {combo_str}")).await;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Pressed key combo: {combo_str}",
        ))]))
    }

    #[tool(description = "Scroll at the specified position. Coordinates can be from either a viewport or full-page screenshot — the system automatically scrolls to bring the position into view first. Use positive delta_y to scroll down, negative to scroll up. Typical scroll amount: 120 pixels per mouse wheel notch. PREFER vscreen_scroll_to_element when targeting a specific element. PREFER vscreen_screenshot(full_page=true) over scrolling to view below-fold content.")]
    async fn vscreen_scroll(
        &self,
        Parameters(params): Parameters<ScrollParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(&params.instance_id)?;
        let sup_arc = self.get_supervisor(&params.instance_id)?;

        let (vx, vy) = sup_arc
            .scroll_into_view_and_translate(params.x, params.y)
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        let event = InputEvent::Wheel {
            x: vx,
            y: vy,
            dx: params.delta_x,
            dy: params.delta_y,
            m: 0,
        };
        sup_arc.dispatch_api_input(&event)
            .await
            .map_err(|e| internal_error(e.to_string()))?;
        self.record_action(&params.instance_id, "scroll", &format!("({}, {}) dy={}", params.x, params.y, params.delta_y), &format!("Scrolled at ({}, {})", params.x, params.y)).await;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Scrolled at page ({}, {}), viewport ({:.0}, {:.0}) by ({}, {})",
            params.x, params.y, vx, vy, params.delta_x, params.delta_y
        ))]))
    }

    #[tool(description = "Click and drag from one position to another. Coordinates can be from either a viewport or full-page screenshot — the system automatically scrolls to bring the start position into view. Useful for sliders, selections, drag-and-drop. The drag is performed with the left mouse button held.")]
    async fn vscreen_drag(
        &self,
        Parameters(params): Parameters<DragParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(&params.instance_id)?;
        let sup_arc = self.get_supervisor(&params.instance_id)?;

        // Translate the start position (scrolls if needed)
        let (from_vx, from_vy) = sup_arc
            .scroll_into_view_and_translate(params.from_x, params.from_y)
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        // Translate the end position relative to the same scroll offset
        let to_vx = params.to_x;
        let to_vy = params.to_y - (params.from_y - from_vy);
        let steps = params.steps.unwrap_or(10).max(1);
        let duration = params.duration_ms.unwrap_or(300);
        let step_delay = std::time::Duration::from_millis(duration / u64::from(steps));

        sup_arc.send_cdp_command(
            "Input.dispatchMouseEvent",
            Some(serde_json::json!({"type":"mouseMoved","x":from_vx,"y":from_vy,"modifiers":0})),
        ).await.map_err(|e| internal_error(e.to_string()))?;

        sup_arc.send_cdp_command(
            "Input.dispatchMouseEvent",
            Some(serde_json::json!({"type":"mousePressed","x":from_vx,"y":from_vy,"button":"left","buttons":1,"clickCount":1,"modifiers":0})),
        ).await.map_err(|e| internal_error(e.to_string()))?;

        for i in 1..=steps {
            let t = f64::from(i) / f64::from(steps);
            let x = from_vx + (to_vx - from_vx) * t;
            let y = from_vy + (to_vy - from_vy) * t;
            sup_arc.send_cdp_command(
                "Input.dispatchMouseEvent",
                Some(serde_json::json!({"type":"mouseMoved","x":x,"y":y,"button":"left","buttons":1,"modifiers":0})),
            ).await.map_err(|e| internal_error(e.to_string()))?;
            tokio::time::sleep(step_delay).await;
        }

        sup_arc.send_cdp_command(
            "Input.dispatchMouseEvent",
            Some(serde_json::json!({"type":"mouseReleased","x":to_vx,"y":to_vy,"button":"left","buttons":0,"clickCount":1,"modifiers":0})),
        ).await.map_err(|e| internal_error(e.to_string()))?;

        self.record_action(&params.instance_id, "drag", &format!("({},{}) -> ({},{})", params.from_x, params.from_y, params.to_x, params.to_y), "Dragged").await;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Dragged from page ({}, {}) to ({}, {})",
            params.from_x, params.from_y, params.to_x, params.to_y
        ))]))
    }

    #[tool(description = "Move the mouse to the specified coordinates without clicking. Coordinates can be from either a viewport or full-page screenshot — the system automatically scrolls to bring the target into view if needed. Useful for triggering hover effects, tooltips, or dropdown menus.")]
    async fn vscreen_hover(
        &self,
        Parameters(params): Parameters<HoverParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(&params.instance_id)?;
        let sup_arc = self.get_supervisor(&params.instance_id)?;

        let (vx, vy) = sup_arc
            .scroll_into_view_and_translate(params.x, params.y)
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        let event = InputEvent::MouseMove {
            x: vx,
            y: vy,
            b: 0,
            m: 0,
        };
        sup_arc.dispatch_api_input(&event)
            .await
            .map_err(|e| internal_error(e.to_string()))?;
        self.record_action(&params.instance_id, "hover", &format!("({}, {})", params.x, params.y), &format!("Hovered at ({}, {})", params.x, params.y)).await;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Hovered at ({}, {})",
            params.x, params.y
        ))]))
    }

    #[tool(description = "Wait for a specified duration in milliseconds. Does NOT require instance_id. Parameter: duration_ms (required). Use between actions to let the page settle. Typical: 500ms after click, 1000-3000ms after navigation, 3000ms after reCAPTCHA checkbox click. PREFER vscreen_wait_for_text, vscreen_wait_for_selector, or vscreen_wait_for_url over fixed-duration waits when you know what to expect.")]
    async fn vscreen_wait(
        &self,
        Parameters(params): Parameters<WaitParam>,
    ) -> Result<CallToolResult, McpError> {
        tokio::time::sleep(std::time::Duration::from_millis(params.duration_ms)).await;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Waited {}ms",
            params.duration_ms
        ))]))
    }

    #[tool(description = "Wait until the page is idle (document.readyState === 'complete' and no pending network requests). Times out after the specified duration. Use after navigation.")]
    async fn vscreen_wait_for_idle(
        &self,
        Parameters(params): Parameters<WaitForIdleParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        let timeout_ms = params.timeout_ms.unwrap_or(5000);
        let deadline =
            tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);

        loop {
            {
                let result = sup
                    .evaluate_js("document.readyState")
                    .await
                    .map_err(|e| internal_error(e.to_string()))?;
                if result.as_str() == Some("complete") {
                    return Ok(CallToolResult::success(vec![Content::text(
                        "Page is idle (readyState=complete)",
                    )]));
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(CallToolResult::success(vec![Content::text(format!(
                    "Timed out after {}ms (readyState not complete)",
                    timeout_ms
                ))]));
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }

    #[tool(description = "Get the current mouse cursor position (last known position from mouse events). Returns {x, y} coordinates.")]
    async fn vscreen_get_cursor_position(
        &self,
        Parameters(params): Parameters<InstanceIdParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        let (x, y) = sup.get_cursor_position();
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::json!({"x": x, "y": y}).to_string(),
        )]))
    }

    #[tool(description = "Execute JavaScript in the MAIN FRAME only (cannot access iframe content). The 'expression' is evaluated via CDP Runtime.evaluate and returns the result as JSON. Useful for custom DOM manipulation not covered by existing tools. For iframe content, use vscreen_find_elements(include_iframes=true) instead. PREFER vscreen_get_page_info for page metadata (title, URL, viewport). PREFER vscreen_extract_text for reading page content. Only use execute_js for custom queries.")]
    async fn vscreen_execute_js(
        &self,
        Parameters(params): Parameters<ExecuteJsParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        let result = sup
            .evaluate_js(&params.expression)
            .await
            .map_err(|e| internal_error(e.to_string()))?;
        let text = serde_json::to_string_pretty(&result).unwrap_or_default();
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    // -----------------------------------------------------------------------
    // Phase 1a: Screenshot History
    // -----------------------------------------------------------------------

    #[tool(description = "List metadata for all screenshots stored in the history ring buffer. Returns timestamps, URLs, and action labels — no image data. Use vscreen_history_get to retrieve actual images.")]
    async fn vscreen_history_list(
        &self,
        Parameters(params): Parameters<HistoryListParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        let meta = sup.with_screenshot_history(|h| h.list());
        let text = serde_json::to_string_pretty(&meta).unwrap_or_default();
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "Retrieve a specific historical screenshot by index (0 = oldest in buffer). Returns the image data. Use vscreen_history_list first to see available indices.")]
    async fn vscreen_history_get(
        &self,
        Parameters(params): Parameters<HistoryGetParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        let entry = sup
            .with_screenshot_history(|h| h.get(params.index).cloned())
            .ok_or_else(|| invalid_params(format!("no screenshot at index {}", params.index)))?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&entry.data);
        Ok(CallToolResult::success(vec![
            Content::text(serde_json::json!({
                "index": params.index,
                "timestamp_ms": entry.timestamp_ms,
                "url": entry.url,
                "action_label": entry.action_label,
            }).to_string()),
            Content::image(b64, "image/jpeg"),
        ]))
    }

    #[tool(description = "Retrieve a range of historical screenshots. Returns metadata plus image data for each. Useful for reviewing a sequence of past actions.")]
    async fn vscreen_history_get_range(
        &self,
        Parameters(params): Parameters<HistoryGetRangeParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        let entries = sup.with_screenshot_history(|h| {
            h.get_range(params.from, params.count)
                .into_iter()
                .cloned()
                .collect::<Vec<_>>()
        });
        if entries.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No screenshots in the requested range.",
            )]));
        }
        let mut content = Vec::new();
        for (i, entry) in entries.iter().enumerate() {
            content.push(Content::text(serde_json::json!({
                "index": params.from + i,
                "timestamp_ms": entry.timestamp_ms,
                "url": entry.url,
                "action_label": entry.action_label,
            }).to_string()));
            let b64 = base64::engine::general_purpose::STANDARD.encode(&entry.data);
            content.push(Content::image(b64, "image/jpeg"));
        }
        Ok(CallToolResult::success(content))
    }

    #[tool(description = "Clear the screenshot history buffer. Use when starting a new task or to free memory.")]
    async fn vscreen_history_clear(
        &self,
        Parameters(params): Parameters<InstanceIdParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        sup.with_screenshot_history_mut(|h| h.clear());
        Ok(CallToolResult::success(vec![Content::text(
            "Screenshot history cleared.",
        )]))
    }

    // -----------------------------------------------------------------------
    // Phase 1b: Action Session Log
    // -----------------------------------------------------------------------

    #[tool(description = "Retrieve the action session log — a timestamped history of every MCP action performed. Useful for understanding 'how did I get here' context. Returns tool names, parameters, results, and URLs.")]
    async fn vscreen_session_log(
        &self,
        Parameters(params): Parameters<SessionLogParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        let entries = sup.with_action_log(|l| {
            if let Some(n) = params.last_n {
                l.last_n(n).into_iter().cloned().collect::<Vec<_>>()
            } else {
                l.entries().iter().cloned().collect::<Vec<_>>()
            }
        });
        let text = serde_json::to_string_pretty(&entries).unwrap_or_default();
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "Get a condensed text summary of the current session — a quick overview of all actions taken so far, in chronological order.")]
    async fn vscreen_session_summary(
        &self,
        Parameters(params): Parameters<InstanceIdParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        let summary = sup.with_action_log(|l| l.summary());
        Ok(CallToolResult::success(vec![Content::text(summary)]))
    }

    // -----------------------------------------------------------------------
    // Phase 2a: Element Discovery
    // -----------------------------------------------------------------------

    #[tool(description = "Find elements matching a CSS selector. Returns an array of element metadata including tag name, text content, bounding box (page coordinates compatible with click/hover), visibility, occlusion status, and key attributes. The 'occluded' field is true when another element with higher z-index covers the center of this element. Set include_iframes=true to also search inside iframes. Much more reliable than guessing coordinates from screenshots.")]
    async fn vscreen_find_elements(
        &self,
        Parameters(params): Parameters<FindElementsParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        let sel_json = serde_json::to_string(&params.selector).unwrap_or_default();
        let js = format!(
            r#"JSON.stringify(Array.from(document.querySelectorAll({sel})).slice(0, 50).map(el => {{
    const r = el.getBoundingClientRect();
    const vis = r.width > 0 && r.height > 0 && window.getComputedStyle(el).visibility !== 'hidden';
    let occluded = false;
    if (vis) {{
        const cx = r.left + r.width / 2;
        const cy = r.top + r.height / 2;
        const top = document.elementFromPoint(cx, cy);
        if (top && top !== el && !el.contains(top) && !top.contains(el)) {{
            occluded = true;
        }}
    }}
    return {{
        tag: el.tagName.toLowerCase(),
        text: (el.innerText || el.textContent || '').substring(0, 200).trim(),
        x: Math.round(r.left + r.width / 2 + window.scrollX),
        y: Math.round(r.top + r.height / 2 + window.scrollY),
        width: Math.round(r.width),
        height: Math.round(r.height),
        visible: vis,
        occluded: occluded,
        id: el.id || undefined,
        className: el.className || undefined,
        href: el.href || undefined,
        type: el.type || undefined,
        value: el.value || undefined,
        placeholder: el.placeholder || undefined,
        ariaLabel: el.getAttribute('aria-label') || undefined,
        role: el.getAttribute('role') || undefined,
        title: el.getAttribute('title') || undefined,
        tabIndex: el.tabIndex >= 0 ? el.tabIndex : undefined,
        svgContent: (() => {{ const st = el.querySelector('svg title, svg desc'); return st ? st.textContent.trim() : undefined; }})(),
    }};
}}))"#,
            sel = sel_json
        );
        let result = sup
            .evaluate_js(&js)
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        let mut main_str = if let Some(s) = result.as_str() {
            s.to_string()
        } else {
            serde_json::to_string_pretty(&result).unwrap_or_default()
        };

        if params.include_iframes {
            if let Ok(frame_tree) = sup.get_frame_tree().await {
                let iframe_results = self.find_elements_in_frames(&sup, &frame_tree, &sel_json).await;
                if !iframe_results.is_empty() {
                    let mut all: Vec<serde_json::Value> = serde_json::from_str(&main_str).unwrap_or_default();
                    all.extend(iframe_results);
                    main_str = serde_json::to_string(&all).unwrap_or(main_str);
                }
            }
        }

        Ok(CallToolResult::success(vec![Content::text(main_str)]))
    }

    #[tool(description = "Find elements by their visible text content. Returns matching elements with bounding boxes (page coordinates). PREFER this when you know the visible label (e.g. 'Sign In', 'Submit'). Set include_iframes=true to also search inside iframes. For main-frame clicks, consider vscreen_click_element(text='...') instead for a single-step find+click.")]
    async fn vscreen_find_by_text(
        &self,
        Parameters(params): Parameters<FindByTextParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        let exact = params.exact;
        let search = serde_json::to_string(&params.text).unwrap_or_default();
        let match_fn = if exact {
            format!("(el.innerText || '').trim() === {search}")
        } else {
            format!("(el.innerText || '').toLowerCase().includes({search}.toLowerCase())")
        };
        let js = format!(
            r#"JSON.stringify((function() {{
    const search = {search};
    const results = [];
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_ELEMENT);
    let el;
    while ((el = walker.nextNode()) && results.length < 30) {{
        if ({match_fn}) {{
            const r = el.getBoundingClientRect();
            if (r.width > 0 && r.height > 0) {{
                results.push({{
                    tag: el.tagName.toLowerCase(),
                    text: (el.innerText || '').substring(0, 200).trim(),
                    x: Math.round(r.left + r.width / 2 + window.scrollX),
                    y: Math.round(r.top + r.height / 2 + window.scrollY),
                    width: Math.round(r.width),
                    height: Math.round(r.height),
                    id: el.id || undefined,
                    className: el.className || undefined,
                }});
            }}
        }}
    }}
    return results;
}})())"#
        );
        let result = sup
            .evaluate_js(&js)
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        let mut text = if let Some(s) = result.as_str() {
            s.to_string()
        } else {
            serde_json::to_string_pretty(&result).unwrap_or_default()
        };

        if params.include_iframes {
            if let Ok(frame_tree) = sup.get_frame_tree().await {
                let iframe_results = self.find_text_in_frames(&sup, &frame_tree, &search, exact).await;
                if !iframe_results.is_empty() {
                    let mut all: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap_or_default();
                    all.extend(iframe_results);
                    text = serde_json::to_string(&all).unwrap_or(text);
                }
            }
        }

        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    // -----------------------------------------------------------------------
    // Phase 3a/3b: Wait Conditions
    // -----------------------------------------------------------------------

    #[tool(description = "Wait until specific text appears on the page. Polls the page body text at a configurable interval. Returns immediately if text is already present. Use after navigation or actions that trigger content changes.")]
    async fn vscreen_wait_for_text(
        &self,
        Parameters(params): Parameters<WaitForTextParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        let timeout = params.timeout_ms.unwrap_or(10000);
        let interval = params.interval_ms.unwrap_or(250);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout);
        let search = serde_json::to_string(&params.text).unwrap_or_default();
        let js = format!("document.body.innerText.includes({search})");

        loop {
            {
                let result = sup.evaluate_js(&js).await.map_err(|e| internal_error(e.to_string()))?;
                if result.as_bool() == Some(true) {
                    return Ok(CallToolResult::success(vec![Content::text(format!(
                        "Text '{}' found on page.",
                        params.text
                    ))]));
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(CallToolResult::success(vec![Content::text(format!(
                    "Timed out after {}ms waiting for text '{}'.",
                    timeout, params.text
                ))]));
            }
            tokio::time::sleep(std::time::Duration::from_millis(interval)).await;
        }
    }

    #[tool(description = "Wait until a CSS selector matches at least one element on the page. Optionally wait for the element to be visible. Polls at a configurable interval. Use after navigation or AJAX requests.")]
    async fn vscreen_wait_for_selector(
        &self,
        Parameters(params): Parameters<WaitForSelectorParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        let timeout = params.timeout_ms.unwrap_or(10000);
        let interval = params.interval_ms.unwrap_or(250);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout);
        let sel = serde_json::to_string(&params.selector).unwrap_or_default();
        let js = if params.visible {
            format!(
                r#"(function() {{ const el = document.querySelector({sel}); if (!el) return false; const r = el.getBoundingClientRect(); return r.width > 0 && r.height > 0 && window.getComputedStyle(el).visibility !== 'hidden'; }})()"#
            )
        } else {
            format!("!!document.querySelector({sel})")
        };

        loop {
            {
                let result = sup.evaluate_js(&js).await.map_err(|e| internal_error(e.to_string()))?;
                if result.as_bool() == Some(true) {
                    return Ok(CallToolResult::success(vec![Content::text(format!(
                        "Selector '{}' found on page.",
                        params.selector
                    ))]));
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(CallToolResult::success(vec![Content::text(format!(
                    "Timed out after {}ms waiting for selector '{}'.",
                    timeout, params.selector
                ))]));
            }
            tokio::time::sleep(std::time::Duration::from_millis(interval)).await;
        }
    }

    // -----------------------------------------------------------------------
    // Phase 2d: Annotated Screenshot
    // -----------------------------------------------------------------------

    #[tool(description = "Capture a screenshot with numbered bounding box annotations overlaid on interactive elements. Annotates MAIN FRAME elements only. Returns the annotated image plus a JSON legend mapping each number to element details (tag, text, coordinates). Default selector targets interactive elements: a, button, input, select, textarea, [role=button], [onclick]. For iframe elements, use vscreen_find_by_text(include_iframes=true) instead.")]
    async fn vscreen_screenshot_annotated(
        &self,
        Parameters(params): Parameters<AnnotatedScreenshotParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;

        let selector = params.selector.as_deref().unwrap_or(
            "a, button, input, select, textarea, [role='button'], [onclick], [tabindex]"
        );
        let sel_json = serde_json::to_string(selector).unwrap_or_default();

        // Inject DOM overlay elements onto the page, discover elements, and return legend
        let inject_js = format!(
            r#"(function() {{
    const els = Array.from(document.querySelectorAll({sel})).slice(0, 40);
    const results = [];
    const container = document.createElement('div');
    container.id = '__vscreen_annotation_overlay__';
    container.style.cssText = 'pointer-events:none;position:fixed;top:0;left:0;width:100%;height:100%;z-index:2147483647;';
    let idx = 0;
    els.forEach(el => {{
        const r = el.getBoundingClientRect();
        if (r.width <= 0 || r.height <= 0) return;
        idx++;
        const box = document.createElement('div');
        box.style.cssText = 'position:fixed;border:2px solid rgba(255,0,0,0.8);pointer-events:none;box-sizing:border-box;'
            + 'left:' + r.left + 'px;top:' + r.top + 'px;width:' + r.width + 'px;height:' + r.height + 'px;';
        const lbl = document.createElement('div');
        lbl.style.cssText = 'position:absolute;top:-18px;left:-1px;background:rgba(255,0,0,0.9);color:white;'
            + 'font:bold 12px monospace;padding:1px 4px;line-height:16px;white-space:nowrap;';
        lbl.textContent = String(idx);
        box.appendChild(lbl);
        container.appendChild(box);
        const ariaLbl = el.getAttribute('aria-label') || '';
        const elRole = el.getAttribute('role') || '';
        results.push({{
            index: idx,
            tag: el.tagName.toLowerCase(),
            text: (el.innerText || el.value || el.placeholder || ariaLbl || '').substring(0, 100).trim(),
            ariaLabel: ariaLbl || undefined,
            role: elRole || undefined,
            x: Math.round(r.left),
            y: Math.round(r.top),
            width: Math.round(r.width),
            height: Math.round(r.height),
            page_x: Math.round(r.left + window.scrollX),
            page_y: Math.round(r.top + window.scrollY),
        }});
    }});
    document.body.appendChild(container);
    return JSON.stringify(results);
}})()"#,
            sel = sel_json
        );

        let elements_result = sup
            .evaluate_js(&inject_js)
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        let legend_str = if let Some(s) = elements_result.as_str() {
            s.to_string()
        } else {
            serde_json::to_string(&elements_result).unwrap_or_default()
        };

        // Capture screenshot with the overlay visible
        let screenshot_data = sup
            .capture_screenshot("png", None)
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        // Remove the overlay
        let _ = sup
            .evaluate_js("(function(){const o=document.getElementById('__vscreen_annotation_overlay__');if(o)o.remove();})()")
            .await;

        let element_count: usize = serde_json::from_str::<Vec<serde_json::Value>>(&legend_str)
            .map(|v| v.len())
            .unwrap_or(0);

        let mut content = Vec::new();
        content.push(Content::text(format!(
            "Annotated screenshot with {element_count} interactive elements. Red bounding boxes with numbered labels are overlaid on the page. Use the legend below to identify elements by number and get their clickable coordinates (page_x, page_y)."
        )));

        let screenshot_b64 = base64::engine::general_purpose::STANDARD.encode(&screenshot_data);
        content.push(Content::image(screenshot_b64, "image/png"));
        content.push(Content::text(format!("Element legend:\n{legend_str}")));

        Ok(CallToolResult::success(content))
    }

    // -----------------------------------------------------------------------
    // Phase 4a: Navigation
    // -----------------------------------------------------------------------

    #[tool(description = "Navigate the browser back in history (like pressing the Back button).")]
    async fn vscreen_go_back(
        &self,
        Parameters(params): Parameters<InstanceIdParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        sup.evaluate_js("history.back()")
            .await
            .map_err(|e| internal_error(e.to_string()))?;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        Ok(CallToolResult::success(vec![Content::text(
            "Navigated back in history.",
        )]))
    }

    #[tool(description = "Navigate the browser forward in history (like pressing the Forward button).")]
    async fn vscreen_go_forward(
        &self,
        Parameters(params): Parameters<InstanceIdParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        sup.evaluate_js("history.forward()")
            .await
            .map_err(|e| internal_error(e.to_string()))?;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        Ok(CallToolResult::success(vec![Content::text(
            "Navigated forward in history.",
        )]))
    }

    #[tool(description = "Reload the current page.")]
    async fn vscreen_reload(
        &self,
        Parameters(params): Parameters<InstanceIdParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        sup.send_cdp_command("Page.reload", Some(serde_json::json!({"ignoreCache": false})))
            .await
            .map_err(|e| internal_error(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(
            "Page reload initiated.",
        )]))
    }

    #[tool(description = "Extract visible text from the page or a specific element. Returns clean text without HTML. PREFER this over vscreen_screenshot when you only need to read page text — faster and more accurate. Optionally pass a CSS selector to extract text from a specific element.")]
    async fn vscreen_extract_text(
        &self,
        Parameters(params): Parameters<ExtractTextParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        let js = if let Some(ref sel) = params.selector {
            let sel_json = serde_json::to_string(sel).unwrap_or_default();
            format!(
                r#"(function() {{ const el = document.querySelector({sel_json}); return el ? el.innerText : 'Element not found: ' + {sel_json}; }})()"#
            )
        } else {
            "document.body.innerText".into()
        };
        let result = sup
            .evaluate_js(&js)
            .await
            .map_err(|e| internal_error(e.to_string()))?;
        let text = result.as_str().unwrap_or("").to_string();
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    // -----------------------------------------------------------------------
    // Phase 1c: Console Capture
    // -----------------------------------------------------------------------

    #[tool(description = "Retrieve captured browser console messages (console.log, console.warn, console.error, etc). Optionally filter by log level. Useful for debugging JavaScript errors or understanding page behavior. Automatically enables console capture if not already active.")]
    async fn vscreen_console_log(
        &self,
        Parameters(params): Parameters<ConsoleLogParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        // Ensure console capture is installed and sync messages
        let _ = sup.enable_console_capture().await;
        let _ = sup.sync_console_messages().await;

        let entries = sup.with_console_buffer(|buf| {
            if let Some(ref level) = params.level {
                buf.filter_by_level(level)
                    .into_iter()
                    .cloned()
                    .collect::<Vec<_>>()
            } else {
                buf.entries().iter().cloned().collect::<Vec<_>>()
            }
        });
        let text = serde_json::to_string_pretty(&entries).unwrap_or_default();
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "Clear the console message buffer.")]
    async fn vscreen_console_clear(
        &self,
        Parameters(params): Parameters<InstanceIdParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        sup.with_console_buffer_mut(|b| b.clear());
        Ok(CallToolResult::success(vec![Content::text(
            "Console buffer cleared.",
        )]))
    }

    // -----------------------------------------------------------------------
    // Phase 2c: Accessibility Tree
    // -----------------------------------------------------------------------

    #[tool(description = "Get the accessibility tree of the page. Returns a structured tree of elements with their roles, names, values, and states. Useful for understanding page structure semantically, especially for complex UIs where CSS selectors are difficult.")]
    async fn vscreen_accessibility_tree(
        &self,
        Parameters(params): Parameters<AccessibilityTreeParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        let max_depth = params.max_depth.unwrap_or(5);

        let js = format!(
            r#"(function() {{
    function walkTree(node, depth) {{
        if (depth > {max_depth}) return null;
        const role = node.role || '';
        const name = node.name || '';
        if (!role && !name && (!node.children || node.children.length === 0)) return null;
        const result = {{ role: role, name: name }};
        if (node.value) result.value = node.value;
        if (node.description) result.description = node.description;
        if (node.disabled) result.disabled = true;
        if (node.focused) result.focused = true;
        if (node.children && node.children.length > 0) {{
            result.children = node.children.map(c => walkTree(c, depth + 1)).filter(Boolean);
            if (result.children.length === 0) delete result.children;
        }}
        return result;
    }}
    return 'accessibility_via_cdp';
}})()"#
        );

        // Use CDP Accessibility.getFullAXTree for real tree data
        let tree_result = sup
            .send_cdp_command_and_wait("Accessibility.getFullAXTree", Some(serde_json::json!({"depth": max_depth})))
            .await;

        match tree_result {
            Ok(response) => {
                let text = serde_json::to_string_pretty(&response).unwrap_or_default();
                // Truncate if extremely large
                let truncated = if text.len() > 50000 {
                    format!("{}...\n(truncated, {} bytes total)", &text[..50000], text.len())
                } else {
                    text
                };
                Ok(CallToolResult::success(vec![Content::text(truncated)]))
            }
            Err(_) => {
                // Fallback: use JS-based accessibility info
                let result = sup.evaluate_js(&js).await.map_err(|e| internal_error(e.to_string()))?;
                let text = serde_json::to_string_pretty(&result).unwrap_or_default();
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }
        }
    }

    // -----------------------------------------------------------------------
    // Phase 2d: Vision-Powered Element Description
    // -----------------------------------------------------------------------

    #[tool(description = "Identify unlabeled UI elements (icon-only buttons, etc.) using vision LLM analysis. \
For each element matching the selector that has no text, aria-label, title, or placeholder, a cropped screenshot \
is sent to the vision model which describes the icon and its likely function. \
Returns elements enriched with AI-generated descriptions. Requires --vision-url to be configured. \
Up to 10 unlabeled elements are analyzed per call.")]
    async fn vscreen_describe_elements(
        &self,
        Parameters(params): Parameters<DescribeElementsParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;

        let vision = self
            .state
            .vision_client
            .as_ref()
            .ok_or_else(|| internal_error("Vision LLM not configured (--vision-url required)"))?;

        if !vision.is_available().await {
            return Err(internal_error("Vision LLM is not reachable"));
        }

        let default_sel = "button, [role='button'], a, input, select".to_string();
        let selector = params.selector.as_deref().unwrap_or(&default_sel);
        let sel_json = serde_json::to_string(selector).unwrap_or_default();

        // Find all matching elements with viewport-relative bounding boxes.
        // We use viewport coords for the CDP clip, then convert to page-space for output.
        let js = format!(
            r#"JSON.stringify(Array.from(document.querySelectorAll({sel})).slice(0, 50).map(el => {{
    const r = el.getBoundingClientRect();
    const vis = r.width > 0 && r.height > 0 && window.getComputedStyle(el).visibility !== 'hidden';
    if (!vis) return null;
    const svgTitle = el.querySelector('svg title, svg desc');
    return {{
        tag: el.tagName.toLowerCase(),
        text: (el.innerText || el.textContent || '').substring(0, 200).trim(),
        ariaLabel: el.getAttribute('aria-label') || '',
        title: el.getAttribute('title') || '',
        placeholder: el.placeholder || '',
        svgDesc: svgTitle ? svgTitle.textContent.trim() : '',
        role: el.getAttribute('role') || '',
        vx: r.left,
        vy: r.top,
        vw: r.width,
        vh: r.height,
        x: Math.round(r.left + window.scrollX),
        y: Math.round(r.top + window.scrollY),
        width: Math.round(r.width),
        height: Math.round(r.height),
    }};
}}).filter(Boolean))"#,
            sel = sel_json
        );
        let result = sup
            .evaluate_js(&js)
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        let elements: Vec<serde_json::Value> = if let Some(s) = result.as_str() {
            serde_json::from_str(s).unwrap_or_default()
        } else {
            serde_json::from_value(result.clone()).unwrap_or_default()
        };

        let mut output: Vec<serde_json::Value> = Vec::new();
        let mut vision_count = 0u32;
        const MAX_VISION_CALLS: u32 = 10;
        const CROP_PADDING: f64 = 8.0;

        for el in &elements {
            let text = el.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let aria = el.get("ariaLabel").and_then(|v| v.as_str()).unwrap_or("");
            let title = el.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let placeholder = el.get("placeholder").and_then(|v| v.as_str()).unwrap_or("");
            let svg_desc = el.get("svgDesc").and_then(|v| v.as_str()).unwrap_or("");
            let has_label = !text.is_empty()
                || !aria.is_empty()
                || !title.is_empty()
                || !placeholder.is_empty()
                || !svg_desc.is_empty();

            if has_label && !params.include_labeled {
                let mut entry = serde_json::json!({
                    "tag": el.get("tag"),
                    "text": text,
                    "ariaLabel": aria,
                    "title": title,
                    "role": el.get("role"),
                    "x": el.get("x"),
                    "y": el.get("y"),
                    "width": el.get("width"),
                    "height": el.get("height"),
                });
                if !svg_desc.is_empty() {
                    entry["svgDesc"] = serde_json::json!(svg_desc);
                }
                output.push(entry);
                continue;
            }

            // For unlabeled elements (or all if include_labeled), use vision
            if vision_count < MAX_VISION_CALLS {
                let vx = el.get("vx").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let vy = el.get("vy").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let vw = el.get("vw").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let vh = el.get("vh").and_then(|v| v.as_f64()).unwrap_or(0.0);

                let clip_x = (vx - CROP_PADDING).max(0.0);
                let clip_y = (vy - CROP_PADDING).max(0.0);
                let clip_w = vw + CROP_PADDING * 2.0;
                let clip_h = vh + CROP_PADDING * 2.0;

                if let Ok(crop) = sup
                    .capture_screenshot_clip("jpeg", Some(80), Some((clip_x, clip_y, clip_w, clip_h)))
                    .await
                {
                    vision_count += 1;
                    let mut entry = serde_json::json!({
                        "tag": el.get("tag"),
                        "role": el.get("role"),
                        "x": el.get("x"),
                        "y": el.get("y"),
                        "width": el.get("width"),
                        "height": el.get("height"),
                    });
                    if !text.is_empty() { entry["text"] = serde_json::json!(text); }
                    if !aria.is_empty() { entry["ariaLabel"] = serde_json::json!(aria); }
                    if !svg_desc.is_empty() { entry["svgDesc"] = serde_json::json!(svg_desc); }

                    match vision.analyze(&crop, crate::vision::prompts::DESCRIBE_ELEMENT).await {
                        Ok(resp) => {
                            if let Some(json_str) = crate::vision::VisionClient::extract_json(&resp.text) {
                                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&json_str) {
                                    entry["ai_icon"] = parsed.get("icon").cloned().unwrap_or_default();
                                    entry["ai_action"] = parsed.get("action").cloned().unwrap_or_default();
                                    entry["ai_label"] = parsed.get("label").cloned().unwrap_or_default();
                                }
                            } else {
                                entry["ai_description"] = serde_json::json!(resp.text);
                            }
                        }
                        Err(e) => {
                            entry["ai_error"] = serde_json::json!(e.to_string());
                        }
                    }
                    output.push(entry);
                } else {
                    // Crop failed — include without vision
                    output.push(serde_json::json!({
                        "tag": el.get("tag"),
                        "role": el.get("role"),
                        "x": el.get("x"),
                        "y": el.get("y"),
                        "width": el.get("width"),
                        "height": el.get("height"),
                        "ai_error": "screenshot crop failed",
                    }));
                }
            }
        }

        let result_text = serde_json::to_string_pretty(&output).unwrap_or_default();
        Ok(CallToolResult::success(vec![Content::text(result_text)]))
    }

    // -----------------------------------------------------------------------
    // Phase 4c: Cookie/Storage Management
    // -----------------------------------------------------------------------

    #[tool(description = "Get all cookies for the current page. Returns cookie names, values, domains, paths, and expiry information.")]
    async fn vscreen_get_cookies(
        &self,
        Parameters(params): Parameters<InstanceIdParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        let result = sup
            .send_cdp_command_and_wait("Network.getCookies", None)
            .await
            .map_err(|e| internal_error(e.to_string()))?;
        let text = serde_json::to_string_pretty(&result).unwrap_or_default();
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "Set a cookie in the browser. Useful for authentication flows or testing cookie-dependent behavior.")]
    async fn vscreen_set_cookie(
        &self,
        Parameters(params): Parameters<SetCookieParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        let mut cookie = serde_json::json!({
            "name": params.name,
            "value": params.value,
        });
        if let Some(ref domain) = params.domain {
            cookie["domain"] = serde_json::json!(domain);
        } else {
            let url = sup.current_url().await;
            cookie["url"] = serde_json::json!(url);
        }
        if let Some(ref path) = params.path {
            cookie["path"] = serde_json::json!(path);
        }
        sup.send_cdp_command("Network.setCookie", Some(cookie))
            .await
            .map_err(|e| internal_error(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Cookie '{}' set.",
            params.name
        ))]))
    }

    #[tool(description = "Read a value from localStorage or sessionStorage.")]
    async fn vscreen_get_storage(
        &self,
        Parameters(params): Parameters<StorageGetParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        let storage = if params.storage_type == "session" {
            "sessionStorage"
        } else {
            "localStorage"
        };
        let key_json = serde_json::to_string(&params.key).unwrap_or_default();
        let js = format!("{storage}.getItem({key_json})");
        let result = sup
            .evaluate_js(&js)
            .await
            .map_err(|e| internal_error(e.to_string()))?;
        let text = serde_json::to_string_pretty(&result).unwrap_or_default();
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "Write a value to localStorage or sessionStorage.")]
    async fn vscreen_set_storage(
        &self,
        Parameters(params): Parameters<StorageSetParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        let storage = if params.storage_type == "session" {
            "sessionStorage"
        } else {
            "localStorage"
        };
        let key_json = serde_json::to_string(&params.key).unwrap_or_default();
        let val_json = serde_json::to_string(&params.value).unwrap_or_default();
        let js = format!("{storage}.setItem({key_json}, {val_json})");
        sup.evaluate_js(&js)
            .await
            .map_err(|e| internal_error(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Stored '{}'='{}' in {storage}.",
            params.key, params.value
        ))]))
    }

    // -----------------------------------------------------------------------
    // Phase 3c/3d: Advanced Wait Conditions
    // -----------------------------------------------------------------------

    #[tool(description = "Wait until the page URL contains the specified substring. Useful after clicking a link or submitting a form to verify navigation occurred.")]
    async fn vscreen_wait_for_url(
        &self,
        Parameters(params): Parameters<WaitForUrlParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        let timeout = params.timeout_ms.unwrap_or(10000);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout);

        loop {
            {
                let result = sup
                    .evaluate_js("location.href")
                    .await
                    .map_err(|e| internal_error(e.to_string()))?;
                if let Some(url) = result.as_str() {
                    if url.contains(&params.url_contains) {
                        return Ok(CallToolResult::success(vec![Content::text(format!(
                            "URL now contains '{}': {}",
                            params.url_contains, url
                        ))]));
                    }
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(CallToolResult::success(vec![Content::text(format!(
                    "Timed out after {}ms waiting for URL to contain '{}'.",
                    timeout, params.url_contains
                ))]));
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    }

    #[tool(description = "Wait until there are no pending network requests for the specified idle period. Useful after page loads or AJAX-heavy interactions.")]
    async fn vscreen_wait_for_network_idle(
        &self,
        Parameters(params): Parameters<WaitForNetworkIdleParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        let idle_threshold = params.idle_ms.unwrap_or(500);
        let timeout = params.timeout_ms.unwrap_or(10000);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout);

        let setup_js = r#"(function() {
    if (!window.__vscreen_net_mon) {
        window.__vscreen_net_mon = { pending: 0, lastActivity: Date.now() };
        const orig_fetch = window.fetch;
        window.fetch = function() {
            window.__vscreen_net_mon.pending++;
            window.__vscreen_net_mon.lastActivity = Date.now();
            return orig_fetch.apply(this, arguments).finally(() => {
                window.__vscreen_net_mon.pending--;
                window.__vscreen_net_mon.lastActivity = Date.now();
            });
        };
        const orig_open = XMLHttpRequest.prototype.open;
        const orig_send = XMLHttpRequest.prototype.send;
        XMLHttpRequest.prototype.open = function() { return orig_open.apply(this, arguments); };
        XMLHttpRequest.prototype.send = function() {
            window.__vscreen_net_mon.pending++;
            window.__vscreen_net_mon.lastActivity = Date.now();
            this.addEventListener('loadend', () => {
                window.__vscreen_net_mon.pending--;
                window.__vscreen_net_mon.lastActivity = Date.now();
            });
            return orig_send.apply(this, arguments);
        };
    }
    return 'ok';
})()"#;

        sup.evaluate_js(setup_js)
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        loop {
            {
                let result = sup
                    .evaluate_js("JSON.stringify(window.__vscreen_net_mon || {pending:0,lastActivity:0})")
                    .await
                    .map_err(|e| internal_error(e.to_string()))?;
                let info_str = result.as_str().unwrap_or("{}");
                if let Ok(info) = serde_json::from_str::<serde_json::Value>(info_str) {
                    let pending = info.get("pending").and_then(|v| v.as_i64()).unwrap_or(0);
                    let last_activity = info.get("lastActivity").and_then(|v| v.as_f64()).unwrap_or(0.0);

                    let now_result = sup
                        .evaluate_js("Date.now()")
                        .await
                        .map_err(|e| internal_error(e.to_string()))?;
                    let now = now_result.as_f64().unwrap_or(0.0);

                    if pending == 0 && (now - last_activity) >= idle_threshold as f64 {
                        return Ok(CallToolResult::success(vec![Content::text(format!(
                            "Network idle for {}ms (0 pending requests).",
                            idle_threshold
                        ))]));
                    }
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(CallToolResult::success(vec![Content::text(format!(
                    "Timed out after {}ms waiting for network idle.",
                    timeout
                ))]));
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }

    // -----------------------------------------------------------------------
    // New high-impact tools
    // -----------------------------------------------------------------------

    #[tool(description = "Click an element by CSS selector or visible text. Finds the element, scrolls it into view with scrollIntoView(), re-queries its position, and clicks its center. Supports retries for dynamically-loaded elements and wait_after_ms to observe the click result. Searches the MAIN FRAME ONLY — for iframe elements, use vscreen_find_by_text(include_iframes=true) then vscreen_click. Provide either 'selector' or 'text' (or both).")]
    async fn vscreen_click_element(
        &self,
        Parameters(params): Parameters<ClickElementParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;

        if params.selector.is_none() && params.text.is_none() {
            return Err(McpError::invalid_params(
                "must provide 'selector' or 'text' (or both)",
                None,
            ));
        }

        let index = params.index.unwrap_or(0);
        let max_retries = params.retries.unwrap_or(0);
        let retry_delay = params.retry_delay_ms.unwrap_or(500);

        // Build the JS that finds elements AND scrolls the target into view,
        // then re-queries its position for accurate coordinates.
        let build_find_js = |selector: &Option<String>, text: &Option<String>, text_exact: bool, idx: usize| -> String {
            let find_expr = if let Some(sel) = selector {
                let sel_json = serde_json::to_string(sel).unwrap_or_default();
                let text_filter = if let Some(t) = text {
                    let t_json = serde_json::to_string(t).unwrap_or_default();
                    if text_exact {
                        format!(".filter(el => (el.innerText || '').trim() === {t_json})")
                    } else {
                        format!(".filter(el => (el.innerText || '').toLowerCase().includes({t_json}.toLowerCase()))")
                    }
                } else {
                    String::new()
                };
                format!("Array.from(document.querySelectorAll({sel_json})){text_filter}")
            } else {
                let t_json = serde_json::to_string(text.as_deref().unwrap_or("")).unwrap_or_default();
                let match_fn = if text_exact {
                    format!("(el.innerText || '').trim() === {t_json}")
                } else {
                    format!("(el.innerText || '').toLowerCase().includes({t_json}.toLowerCase())")
                };
                format!(
                    r#"(function() {{
    const results = [];
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_ELEMENT);
    let el;
    while (el = walker.nextNode()) {{
        if ({match_fn}) {{
            const r = el.getBoundingClientRect();
            if (r.width > 0 && r.height > 0) results.push(el);
        }}
    }}
    // Sort by area (smallest first) so the most specific/innermost match is at index 0
    results.sort((a, b) => {{
        const ra = a.getBoundingClientRect();
        const rb = b.getBoundingClientRect();
        return (ra.width * ra.height) - (rb.width * rb.height);
    }});
    return results;
}})()"#
                )
            };

            format!(
                r#"JSON.stringify((function() {{
    const all = {find_expr}.filter(el => {{
        const r = el.getBoundingClientRect();
        return r.width > 0 && r.height > 0;
    }});
    if (all.length === 0) return {{found: 0}};
    const idx = {idx};
    if (idx >= all.length) return {{found: all.length, error: 'index out of range'}};
    const target = all[idx];
    // Scroll the actual DOM element into view for accurate positioning
    target.scrollIntoView({{behavior: 'instant', block: 'center', inline: 'nearest'}});
    // Re-query position after scroll
    const r = target.getBoundingClientRect();
    return {{
        found: all.length,
        tag: target.tagName.toLowerCase(),
        text: (target.innerText || target.value || target.getAttribute('aria-label') || '').substring(0, 200).trim(),
        href: target.href || target.closest('a')?.href || undefined,
        page_x: Math.round(r.left + r.width/2 + window.scrollX),
        page_y: Math.round(r.top + r.height/2 + window.scrollY),
        width: Math.round(r.width),
        height: Math.round(r.height),
    }};
}})())"#
            )
        };

        let js = build_find_js(&params.selector, &params.text, params.text_exact, index);

        let mut attempt = 0u32;
        let el_info: serde_json::Value = loop {
            let result = sup
                .evaluate_js(&js)
                .await
                .map_err(|e| internal_error(e.to_string()))?;

            let result_str = result.as_str().unwrap_or("{}");
            let info: serde_json::Value = serde_json::from_str(result_str).unwrap_or_default();
            let found = info.get("found").and_then(|v| v.as_u64()).unwrap_or(0);

            if found > 0 && info.get("error").is_none() {
                break info;
            }

            if attempt >= max_retries {
                if found == 0 {
                    let tried = if max_retries > 0 {
                        format!(" (tried {} times over {}ms)", attempt + 1, attempt as u64 * retry_delay)
                    } else {
                        String::new()
                    };
                    return Ok(CallToolResult::success(vec![Content::text(
                        format!("No matching elements found{tried}."),
                    )]));
                }
                if let Some(err) = info.get("error").and_then(|v| v.as_str()) {
                    return Err(McpError::invalid_params(
                        format!("{err} (found {found} elements)"),
                        None,
                    ));
                }
            }

            attempt += 1;
            tokio::time::sleep(Duration::from_millis(retry_delay)).await;
        };

        let cx = el_info.get("page_x").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let cy = el_info.get("page_y").and_then(|v| v.as_f64()).unwrap_or(0.0);

        // Translate to viewport coordinates (handles any remaining scroll offset)
        let (vx, vy) = sup
            .scroll_into_view_and_translate(cx, cy)
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        let btn = params.button.unwrap_or(0);
        let btn_name = match btn {
            1 => "middle",
            2 => "right",
            _ => "left",
        };
        for (method, typ, buttons) in [
            ("Input.dispatchMouseEvent", "mouseMoved", 0u32),
            ("Input.dispatchMouseEvent", "mousePressed", 1u32 << btn),
            ("Input.dispatchMouseEvent", "mouseReleased", 0u32),
        ] {
            let mut p = serde_json::json!({
                "type": typ, "x": vx, "y": vy, "modifiers": 0
            });
            if typ != "mouseMoved" {
                p["button"] = serde_json::json!(btn_name);
                p["buttons"] = serde_json::json!(buttons);
                p["clickCount"] = serde_json::json!(1);
            }
            sup.send_cdp_command(method, Some(p))
                .await
                .map_err(|e| internal_error(e.to_string()))?;
        }

        let tag = el_info.get("tag").and_then(|v| v.as_str()).unwrap_or("?");
        let text = el_info.get("text").and_then(|v| v.as_str()).unwrap_or("");
        let found = el_info.get("found").and_then(|v| v.as_u64()).unwrap_or(1);
        let retry_note = if attempt > 0 { format!(" (found after {} retries)", attempt) } else { String::new() };

        self.record_action(&params.instance_id, "click_element", &format!("{tag}: {text}"), &format!("Clicked <{tag}> at ({cx}, {cy})")).await;

        let mut msg = format!(
            "Clicked <{tag}> \"{text}\" at page ({cx}, {cy}), viewport ({vx:.0}, {vy:.0}). {}/{found} matches.{retry_note}",
            index + 1,
        );

        if let Some(wait_ms) = params.wait_after_ms {
            tokio::time::sleep(Duration::from_millis(wait_ms)).await;
            if let Ok(info) = sup.evaluate_js("JSON.stringify({url: location.href, title: document.title})").await {
                if let Some(s) = info.as_str() {
                    if let Ok(page_info) = serde_json::from_str::<serde_json::Value>(s) {
                        let url = page_info.get("url").and_then(|v| v.as_str()).unwrap_or("?");
                        let title = page_info.get("title").and_then(|v| v.as_str()).unwrap_or("?");
                        msg.push_str(&format!("\nAfter {wait_ms}ms — URL: {url}\nTitle: {title}"));
                    }
                }
            }
        }

        if let Some(analysis) = self.vision_verify_action(
            &sup, None, crate::vision::prompts::VERIFY_CLICK
        ).await {
            msg.push_str(&format!("\nVision: {analysis}"));
        }

        Ok(CallToolResult::success(vec![Content::text(msg)]))
    }

    #[tool(description = "Click multiple coordinates in rapid succession in one call. The 'points' parameter is an array of [x, y] coordinate pairs, e.g. [[135,224],[135,324],[135,424]]. Set 'delay_between_ms' for timing between clicks (default: 50ms). Essential for timed challenges like reCAPTCHA tile grids where individual MCP round-trips would be too slow. Each click auto-scrolls into view. Example: {\"instance_id\":\"dev\",\"points\":[[100,200],[200,300]],\"delay_between_ms\":200}")]
    async fn vscreen_batch_click(
        &self,
        Parameters(params): Parameters<BatchClickParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        let delay = params.delay_between_ms.unwrap_or(50);

        let mut clicked = 0u32;
        for point in &params.points {
            let (vx, vy) = sup
                .scroll_into_view_and_translate(point[0], point[1])
                .await
                .map_err(|e| internal_error(e.to_string()))?;

            for (typ, buttons) in [
                ("mouseMoved", 0u32),
                ("mousePressed", 1u32),
                ("mouseReleased", 0u32),
            ] {
                let mut p = serde_json::json!({
                    "type": typ, "x": vx, "y": vy, "modifiers": 0
                });
                if typ != "mouseMoved" {
                    p["button"] = serde_json::json!("left");
                    p["buttons"] = serde_json::json!(buttons);
                    p["clickCount"] = serde_json::json!(1);
                }
                sup.send_cdp_command("Input.dispatchMouseEvent", Some(p))
                    .await
                    .map_err(|e| internal_error(e.to_string()))?;
            }
            clicked += 1;

            if delay > 0 && (clicked as usize) < params.points.len() {
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            }
        }

        self.record_action(&params.instance_id, "batch_click", &format!("{clicked} points"), &format!("Batch-clicked {clicked} points")).await;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Clicked {clicked} points with {delay}ms delay between each.",
        ))]))
    }

    #[tool(description = "Auto-dismiss common cookie consent, privacy, and legal agreement dialogs. Uses a three-tier detection strategy: (1) known consent framework selectors, (2) structural overlay detection (position:fixed containers with consent buttons), (3) vision LLM screenshot analysis if configured. Returns which dialog was dismissed or 'no dialog found'.")]
    async fn vscreen_dismiss_dialogs(
        &self,
        Parameters(params): Parameters<DismissDialogsParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;

        // Returns element coordinates for CDP click — never uses JS .click()
        let find_dialog_button_js = r#"(function() {
    function elCoords(el, extra) {
        const r = el.getBoundingClientRect();
        return Object.assign({found: true,
            x: Math.round(r.left + r.width / 2 + window.scrollX),
            y: Math.round(r.top + r.height / 2 + window.scrollY),
            text: (el.innerText || el.value || '').trim().substring(0, 100)}, extra);
    }

    // --- Tier 1: Known consent framework selectors (high confidence) ---
    const selectors = [
        '#onetrust-accept-btn-handler',
        '#CybotCookiebotDialogBodyLevelButtonLevelOptinAllowAll',
        '.cookie-consent-accept-all',
        '[data-testid="cookie-policy-manage-dialog-btn-accept-all"]',
        '.cc-accept-all',
        '#accept-all-cookies',
        '.js-accept-cookies',
        '#gdpr-cookie-accept',
        '.cookie-banner-accept',
        '#cookie-accept',
        '.fc-cta-consent',
        '#didomi-notice-agree-button',
        '.qc-cmp2-summary-buttons button:first-child',
        '#truste-consent-button',
        '.cookie-notice-accept-button',
        '#sp-cc-accept',
        '.evidon-banner-acceptbutton',
        '#ez-accept-all',
        '.cmpboxbtnyes',
        '[data-gdpr-consent="accept"]',
        '[data-cookie-consent="accept"]',
    ];
    for (const sel of selectors) {
        const el = document.querySelector(sel);
        if (el && el.offsetWidth > 0 && el.offsetHeight > 0) {
            return JSON.stringify(elCoords(el, {method: 'selector', selector: sel}));
        }
    }

    // --- Tier 2: Structural overlay detection ---
    const consentExact = ['ok', 'close', 'dismiss', 'continue', 'agree', 'consent'];
    const consentContains = [
        'accept all', 'accept cookies', 'i accept', 'allow all',
        'accept & close', 'got it', 'i agree', 'accept and continue',
        'akzeptieren', 'tout accepter', 'aceptar todo', 'accetta tutti',
    ];
    function isConsentText(t) {
        const tl = t.toLowerCase();
        for (const p of consentExact) { if (tl === p) return p; }
        for (const p of consentContains) { if (tl.includes(p)) return p; }
        return null;
    }
    const allEls = document.querySelectorAll('*');
    for (const container of allEls) {
        const style = getComputedStyle(container);
        const pos = style.position;
        if (pos !== 'fixed' && pos !== 'sticky') continue;
        const rect = container.getBoundingClientRect();
        if (rect.width < 200 || rect.height < 40) continue;
        const zIndex = parseInt(style.zIndex) || 0;
        const coversWidth = rect.width >= window.innerWidth * 0.5;
        const hasHighZ = zIndex >= 100;
        const hasBg = style.backgroundColor !== 'rgba(0, 0, 0, 0)' && style.backgroundColor !== 'transparent';
        if (!coversWidth && !hasHighZ && !hasBg) continue;
        const buttons = container.querySelectorAll('button, [role="button"], input[type="submit"]');
        for (const btn of buttons) {
            if (btn.offsetWidth <= 0 || btn.offsetHeight <= 0) continue;
            const t = (btn.innerText || btn.value || '').trim();
            if (t.length > 40) continue;
            const matched = isConsentText(t);
            if (matched) {
                return JSON.stringify(elCoords(btn, {method: 'structural_overlay', pattern: matched, container_z: zIndex}));
            }
        }
    }

    return JSON.stringify({found: false});
})()"#;

        let result = sup
            .evaluate_js(find_dialog_button_js)
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        let result_str = result.as_str().unwrap_or("{}");
        let info: serde_json::Value = serde_json::from_str(result_str).unwrap_or_default();

        if info.get("found").and_then(|v| v.as_bool()).unwrap_or(false) {
            let method = info.get("method").and_then(|v| v.as_str()).unwrap_or("?");
            let text = info.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let click_x = info.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let click_y = info.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);

            // CDP click — simulates real browser mouse events
            let (vx, vy) = sup
                .scroll_into_view_and_translate(click_x, click_y)
                .await
                .map_err(|e| internal_error(e.to_string()))?;

            for (typ, buttons) in [("mouseMoved", 0u32), ("mousePressed", 1u32), ("mouseReleased", 0u32)] {
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

            self.record_action(&params.instance_id, "dismiss_dialog", text, &format!("Dismissed dialog via {method}")).await;
            return Ok(CallToolResult::success(vec![Content::text(format!(
                "Dismissed dialog: \"{text}\" (matched via {method}, clicked at {click_x},{click_y}).",
            ))]));
        }

        // --- Tier 3: Vision LLM fallback ---
        if let Some(ref vision) = self.state.vision_client {
            if vision.is_available().await {
                if let Ok(screenshot) = sup.capture_vision_screenshot().await {
                    match vision.analyze(&screenshot, crate::vision::prompts::DISMISS_DIALOG).await {
                        Ok(resp) if resp.found => {
                            if let Some(target) = resp.click_target {
                                // Vision screenshot uses 0.5x scale — coordinates must
                                // be doubled to convert back to page-space.
                                let page_x = target.x * 2.0;
                                let page_y = target.y * 2.0;
                                let (vx, vy) = sup
                                    .scroll_into_view_and_translate(page_x, page_y)
                                    .await
                                    .map_err(|e| internal_error(e.to_string()))?;

                                for (typ, buttons) in [("mouseMoved", 0u32), ("mousePressed", 1u32), ("mouseReleased", 0u32)] {
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

                                let desc = target.description.as_deref().unwrap_or("consent button");
                                self.record_action(&params.instance_id, "dismiss_dialog", desc, "Dismissed dialog via vision LLM").await;
                                return Ok(CallToolResult::success(vec![Content::text(format!(
                                    "Dismissed dialog: \"{desc}\" (matched via vision LLM at ({page_x}, {page_y})).",
                                ))]));
                            }
                            return Ok(CallToolResult::success(vec![Content::text(format!(
                                "Vision LLM detected a dialog but could not determine button coordinates. Details: {}",
                                resp.text.chars().take(200).collect::<String>()
                            ))]));
                        }
                        Ok(_) => { /* vision says no dialog found */ }
                        Err(e) => {
                            debug!("vision LLM error for dismiss_dialogs: {e}");
                        }
                    }
                }
            }
        }

        Ok(CallToolResult::success(vec![Content::text(
            "No consent/cookie dialog found to dismiss.",
        )]))
    }

    // -----------------------------------------------------------------------
    // CAPTCHA solving
    // -----------------------------------------------------------------------

    #[tool(description = "Automatically solve a reCAPTCHA v2 image challenge on the current page. Uses vision LLM to identify which tiles match the target object, then clicks them and verifies. Handles multi-round challenges, retries, and timeouts internally.\n\nParameters:\n  instance_id: string (required)\n  max_attempts: number (default: 3) — max full page-reload retry cycles\n\nRequires vision LLM to be configured (--vision-url). Returns a JSON result with {solved, rounds, details}.")]
    async fn vscreen_solve_captcha(
        &self,
        Parameters(params): Parameters<SolveCaptchaParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;

        let vision = self
            .state
            .vision_client
            .as_ref()
            .ok_or_else(|| internal_error("Vision LLM not configured (--vision-url required)"))?
            .clone();

        let max_attempts = params.max_attempts.unwrap_or(3).max(1).min(10);

        const TOOL_TIMEOUT: Duration = Duration::from_secs(180);
        const VISION_ROUND_TIMEOUT: Duration = Duration::from_secs(45);

        let instance_id = params.instance_id.clone();
        info!(timeout_secs = TOOL_TIMEOUT.as_secs(), vision_timeout_secs = VISION_ROUND_TIMEOUT.as_secs(), "solve_captcha starting");
        let result = tokio::time::timeout(TOOL_TIMEOUT, async {
            let mut total_rounds = 0u32;
            let mut details: Vec<String> = Vec::new();

            for attempt in 0..max_attempts {
                if attempt > 0 {
                    details.push(format!("--- Attempt {} ---", attempt + 1));
                    sup.send_cdp_command(
                        "Page.navigate",
                        Some(serde_json::json!({"url": sup.evaluate_js("location.href").await
                            .ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()})),
                    )
                    .await
                    .map_err(|e| internal_error(e.to_string()))?;
                    tokio::time::sleep(Duration::from_millis(2000)).await;
                }

                // Step 1: Find the checkbox — run frame tree + text search
                let frame_tree = sup
                    .get_frame_tree()
                    .await
                    .map_err(|e| internal_error(e.to_string()))?;
                let search_json = serde_json::to_string("I'm not a robot").unwrap_or_default();
                let checkbox_results =
                    self.find_text_in_frames(&sup, &frame_tree, &search_json, false).await;

                let checkbox_label = checkbox_results
                    .iter()
                    .find(|el| {
                        el.get("tag").and_then(|v| v.as_str()) == Some("label")
                            || el.get("tag").and_then(|v| v.as_str()) == Some("div")
                    })
                    .or_else(|| checkbox_results.first());

                let (cb_x, cb_y) = match checkbox_label {
                    Some(el) => {
                        let x = el.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let y = el.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
                        (x, y)
                    }
                    None => {
                        details.push("No reCAPTCHA checkbox found on page".into());
                        continue;
                    }
                };

                let click_x = cb_x - 28.0;
                let click_y = cb_y;
                info!(x = click_x, y = click_y, "solve_captcha: clicking checkbox");
                self.cdp_click(&sup, click_x, click_y).await?;
                details.push(format!("Clicked checkbox at ({click_x}, {click_y})"));
                tokio::time::sleep(Duration::from_millis(1500)).await;

                // Step 2: Challenge loop
                let max_rounds = 10u32;
                let mut cached_iframe_bounds: Option<(f64, f64, f64, f64)> = None;
                let max_replacement_sub_rounds = 6u32;

                for round in 0..max_rounds {
                    total_rounds += 1;
                    let rnd = round + 1;

                    // Run solved-check and iframe discovery concurrently
                    let (solved, iframe_bounds) = tokio::join!(
                        self.captcha_is_solved(&sup),
                        self.find_captcha_challenge_iframe(&sup)
                    );

                    if solved {
                        details.push("Green checkmark detected — solved!".into());
                        self.record_action(
                            &instance_id,
                            "solve_captcha",
                            &format!("attempt={}, rounds={total_rounds}", attempt + 1),
                            "CAPTCHA solved",
                        )
                        .await;
                        return Ok(serde_json::json!({
                            "solved": true,
                            "rounds": total_rounds,
                            "attempts": attempt + 1,
                            "details": details,
                        }));
                    }

                    let bounds = iframe_bounds.or(cached_iframe_bounds);
                    let (iframe_x, iframe_y, iframe_w, iframe_h) = match bounds {
                        Some(b) => {
                            cached_iframe_bounds = Some(b);
                            b
                        }
                        None => {
                            if round == 0 {
                                tokio::time::sleep(Duration::from_millis(1000)).await;
                                if self.captcha_is_solved(&sup).await {
                                    details.push("Solved without challenge (auto-pass)".into());
                                    self.record_action(
                                        &instance_id,
                                        "solve_captcha",
                                        "auto-pass",
                                        "CAPTCHA solved without challenge",
                                    )
                                    .await;
                                    return Ok(serde_json::json!({
                                        "solved": true,
                                        "rounds": total_rounds,
                                        "attempts": attempt + 1,
                                        "details": details,
                                    }));
                                }
                            }
                            details.push("No challenge iframe found".into());
                            break;
                        }
                    };

                    let grid_positions =
                        compute_grid_positions(iframe_x, iframe_y, iframe_w, iframe_h);
                    let verify_btn =
                        compute_verify_button(iframe_x, iframe_y, iframe_w, iframe_h);

                    // Discover the bframe ID for DOM queries inside the challenge iframe
                    let bframe_id = self.find_captcha_bframe_id(&sup).await;

                    // --- Screenshot + vision analysis ---
                    let screenshot = sup
                        .capture_screenshot_clip(
                            "png",
                            None,
                            Some((iframe_x, iframe_y, iframe_w, iframe_h)),
                        )
                        .await
                        .map_err(|e| internal_error(e.to_string()))?;

                    // Tiny screenshot means the challenge iframe collapsed (solved, expired, or gone)
                    if screenshot.len() < 20_000 {
                        info!(round = rnd, screenshot_len = screenshot.len(), "solve_captcha: screenshot too small, checking state");
                        if self.captcha_is_solved(&sup).await {
                            details.push("Green checkmark detected — solved!".into());
                            self.record_action(&instance_id, "solve_captcha", &format!("attempt={}, rounds={total_rounds}", attempt + 1), "CAPTCHA solved").await;
                            return Ok(serde_json::json!({ "solved": true, "rounds": total_rounds, "attempts": attempt + 1, "details": details }));
                        }
                        if self.captcha_check_expired(&sup).await {
                            details.push("Challenge expired after verify — reloading".into());
                            break;
                        }
                        details.push(format!("Round {rnd}: iframe collapsed (screenshot {}B), retrying", screenshot.len()));
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        continue;
                    }

                    info!(round = rnd, screenshot_len = screenshot.len(), "solve_captcha: sending to vision");
                    let vision_clone = vision.clone();
                    let screenshot_owned = screenshot.to_vec();

                    let vision_handle = AbortOnDrop(tokio::spawn(async move {
                        vision_clone
                            .analyze_fast(
                                &screenshot_owned,
                                crate::vision::prompts::CAPTCHA_TILES,
                                VISION_ROUND_TIMEOUT,
                            )
                            .await
                    }));

                    let vision_result = match tokio::time::timeout(
                        VISION_ROUND_TIMEOUT + Duration::from_secs(2),
                        vision_handle,
                    ).await {
                        Ok(Ok(Ok(resp))) => Some(resp),
                        Ok(Ok(Err(e))) => {
                            details.push(format!("Round {rnd}: vision error: {e}"));
                            None
                        }
                        _ => {
                            details.push(format!("Round {rnd}: vision timed out, skipping"));
                            None
                        }
                    };

                    info!(round = rnd, has_result = vision_result.is_some(), "solve_captcha: vision done");

                    // --- Parse vision response ---
                    let parsed_json: Option<serde_json::Value> = vision_result.as_ref().and_then(|resp| {
                        info!(round = rnd, text_len = resp.text.len(), text_preview = %resp.text.chars().take(700).collect::<String>(), "solve_captcha: vision response");
                        crate::vision::VisionClient::extract_json(&resp.text)
                            .and_then(|json_str| serde_json::from_str(&json_str).ok())
                    });

                    let (tile_indices, challenge_type) = if let Some(ref parsed) = parsed_json {
                        let target = parsed.get("target").and_then(|v| v.as_str()).unwrap_or("unknown");
                        let tiles: Vec<usize> = parsed
                            .get("tiles")
                            .and_then(|v| v.as_array())
                            .map(|arr| arr.iter().filter_map(|v| v.as_u64().map(|n| n as usize)).filter(|&n| n >= 1).collect())
                            .unwrap_or_default();
                        let grid_size = parsed.get("grid_size").and_then(|v| v.as_u64()).unwrap_or(9) as usize;
                        let confidence = parsed.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let ctype = parsed.get("type").and_then(|v| v.as_str()).unwrap_or("dynamic").to_string();
                        details.push(format!(
                            "Round {rnd}: target=\"{target}\", type=\"{ctype}\", grid={grid_size}, tiles={tiles:?}, confidence={confidence:.2}"
                        ));
                        (tiles, ctype)
                    } else {
                        if vision_result.is_some() {
                            details.push(format!("Round {rnd}: failed to parse vision JSON"));
                        } else if !details.last().map(|d| d.contains("timed out")).unwrap_or(false) {
                            details.push(format!("Round {rnd}: vision returned no result"));
                        }
                        (Vec::new(), "unknown".to_string())
                    };

                    info!(round = rnd, tiles = ?tile_indices, challenge_type = %challenge_type, grid_count = grid_positions.len(), "solve_captcha: parsed tiles");

                    let is_dynamic = challenge_type == "dynamic" || (grid_positions.len() == 9 && challenge_type != "select_all");

                    if is_dynamic && !tile_indices.is_empty() {
                        // ====== DYNAMIC REPLACEMENT FLOW ======
                        // Click matching tiles, wait for replacements, re-analyze, repeat
                        let mut all_clicked: std::collections::HashSet<usize> = std::collections::HashSet::new();
                        let mut current_tiles = tile_indices;

                        for sub_round in 0..max_replacement_sub_rounds {
                            let tile_click_points: Vec<[f64; 2]> = current_tiles
                                .iter()
                                .filter(|&&idx| !all_clicked.contains(&idx))
                                .filter_map(|&idx| {
                                    let i = idx.checked_sub(1)?;
                                    grid_positions.get(i).copied()
                                })
                                .collect();

                            if tile_click_points.is_empty() {
                                info!(round = rnd, sub_round, "solve_captcha: no new tiles to click, proceeding to verify");
                                break;
                            }

                            // Track which tiles we've clicked
                            for &idx in &current_tiles {
                                all_clicked.insert(idx);
                            }

                            info!(round = rnd, sub_round, clicks = tile_click_points.len(), clicked_indices = ?current_tiles, "solve_captcha: clicking tiles (dynamic)");

                            // Click only matching tiles (NOT verify)
                            for (i, point) in tile_click_points.iter().enumerate() {
                                self.cdp_click(&sup, point[0], point[1]).await?;
                                if i < tile_click_points.len() - 1 {
                                    tokio::time::sleep(Duration::from_millis(100)).await;
                                }
                            }

                            // Wait for tile replacement animation
                            if let Some(ref bf_id) = bframe_id {
                                self.wait_for_tile_animation(&sup, bf_id, Duration::from_millis(1200)).await;
                            } else {
                                tokio::time::sleep(Duration::from_millis(800)).await;
                            }

                            // Check DOM state for "new images" prompt
                            if let Some(ref bf_id) = bframe_id {
                                if let Some(state) = self.captcha_challenge_state(&sup, bf_id).await {
                                    let header = state.get("header_text").and_then(|v| v.as_str()).unwrap_or("");
                                    let has_new = state.get("has_new_images").and_then(|v| v.as_bool()).unwrap_or(false);
                                    info!(round = rnd, sub_round, has_new, header_text = header, "solve_captcha: DOM state after click");
                                    details.push(format!("Round {rnd}.{sub_round}: DOM: has_new_images={has_new}, header=\"{header}\""));
                                }
                            }

                            // Immediately re-screenshot to catch replacement tiles
                            let re_screenshot = sup
                                .capture_screenshot_clip(
                                    "png",
                                    None,
                                    Some((iframe_x, iframe_y, iframe_w, iframe_h)),
                                )
                                .await
                                .map_err(|e| internal_error(e.to_string()))?;

                            info!(round = rnd, sub_round, screenshot_len = re_screenshot.len(), "solve_captcha: re-screenshot for replacement tiles");

                            let vc = vision.clone();
                            let ss = re_screenshot.to_vec();
                            let sub_handle = AbortOnDrop(tokio::spawn(async move {
                                vc.analyze_fast(&ss, crate::vision::prompts::CAPTCHA_TILES, VISION_ROUND_TIMEOUT).await
                            }));

                            let re_vision = match tokio::time::timeout(
                                VISION_ROUND_TIMEOUT + Duration::from_secs(2),
                                sub_handle,
                            ).await {
                                Ok(Ok(Ok(resp))) => Some(resp),
                                _ => {
                                    details.push(format!("Round {rnd}.{sub_round}: replacement vision timed out"));
                                    None
                                }
                            };

                            // Parse replacement tile analysis
                            let new_tiles: Vec<usize> = re_vision.as_ref().and_then(|resp| {
                                info!(round = rnd, sub_round, text_len = resp.text.len(), "solve_captcha: replacement vision response");
                                crate::vision::VisionClient::extract_json(&resp.text)
                                    .and_then(|json_str| serde_json::from_str::<serde_json::Value>(&json_str).ok())
                                    .map(|parsed| {
                                        let tiles: Vec<usize> = parsed
                                            .get("tiles")
                                            .and_then(|v| v.as_array())
                                            .map(|arr| arr.iter().filter_map(|v| v.as_u64().map(|n| n as usize)).filter(|&n| n >= 1).collect())
                                            .unwrap_or_default();
                                        let target = parsed.get("target").and_then(|v| v.as_str()).unwrap_or("unknown");
                                        let confidence = parsed.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                        // Only consider tiles we haven't already clicked
                                        let new: Vec<usize> = tiles.into_iter().filter(|t| !all_clicked.contains(t)).collect();
                                        details.push(format!(
                                            "Round {rnd}.{sub_round}: replacement analysis: target=\"{target}\", new_tiles={new:?}, confidence={confidence:.2}"
                                        ));
                                        new
                                    })
                            }).unwrap_or_default();

                            info!(round = rnd, sub_round, new_tiles = ?new_tiles, "solve_captcha: replacement tiles found");

                            if new_tiles.is_empty() {
                                details.push(format!("Round {rnd}.{sub_round}: no new replacement tiles match"));
                                break;
                            }
                            current_tiles = new_tiles;
                        }

                        // All replacement tiles handled — now click VERIFY
                        info!(round = rnd, total_clicked = all_clicked.len(), "solve_captcha: clicking verify after dynamic round");
                        self.cdp_click(&sup, verify_btn[0], verify_btn[1]).await?;
                        details.push(format!("Round {rnd}: clicked verify after {clicks} tile clicks", clicks = all_clicked.len()));

                    } else if !tile_indices.is_empty() {
                        // ====== STATIC / SELECT_ALL FLOW ======
                        // Click all tiles + verify in one batch
                        let mut click_points: Vec<[f64; 2]> = tile_indices
                            .iter()
                            .filter_map(|&idx| {
                                let i = idx.checked_sub(1)?;
                                grid_positions.get(i).copied()
                            })
                            .collect();

                        info!(round = rnd, clicks = click_points.len(), verify = ?verify_btn, "solve_captcha: static click targets");
                        click_points.push(verify_btn);

                        for (i, point) in click_points.iter().enumerate() {
                            self.cdp_click(&sup, point[0], point[1]).await?;
                            if i < click_points.len() - 1 {
                                tokio::time::sleep(Duration::from_millis(120)).await;
                            }
                        }
                        details.push(format!("Round {rnd}: static flow, clicked {} tiles + verify", tile_indices.len()));
                    } else {
                        // No tiles found — click verify/skip anyway
                        info!(round = rnd, "solve_captcha: no tiles found, clicking verify/skip");
                        self.cdp_click(&sup, verify_btn[0], verify_btn[1]).await?;
                        details.push(format!("Round {rnd}: no tiles matched, clicked verify/skip"));
                    }

                    // Wait for challenge to update after verify click
                    tokio::time::sleep(Duration::from_millis(1500)).await;

                    // Check expiry
                    if self.captcha_check_expired(&sup).await {
                        details.push("Challenge expired — reloading page".into());
                        break;
                    }
                }
            }

            self.record_action(
                &instance_id,
                "solve_captcha",
                &format!("rounds={total_rounds}"),
                "CAPTCHA solve failed",
            )
            .await;

            Ok::<serde_json::Value, McpError>(serde_json::json!({
                "solved": false,
                "rounds": total_rounds,
                "attempts": max_attempts,
                "details": details,
            }))
        })
        .await;

        match result {
            Ok(Ok(json)) => Ok(CallToolResult::success(vec![Content::text(json.to_string())])),
            Ok(Err(e)) => Err(e),
            Err(_) => {
                self.record_action(
                    &params.instance_id,
                    "solve_captcha",
                    "timeout",
                    &format!("CAPTCHA solve timed out ({}s)", TOOL_TIMEOUT.as_secs()),
                )
                .await;
                info!("solve_captcha: global timeout after {}s", TOOL_TIMEOUT.as_secs());
                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::json!({
                        "solved": false,
                        "error": format!("Tool timed out after {} seconds", TOOL_TIMEOUT.as_secs()),
                    })
                    .to_string(),
                )]))
            }
        }
    }

    #[tool(description = "Clear an input field and fill it with new text. Parameters: selector (CSS selector, e.g. \"input[name='email']\", \"#username\"), value (text to fill). Finds the element, focuses it, selects all existing content (Ctrl+A), deletes it, then types the new value. Works with input, textarea, and contenteditable elements. Searches MAIN FRAME only.")]
    async fn vscreen_fill(
        &self,
        Parameters(params): Parameters<FillParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;

        let sel_json = serde_json::to_string(&params.selector).unwrap_or_default();

        let js = format!(
            r#"(function() {{
    const el = document.querySelector({sel_json});
    if (!el) return JSON.stringify({{error: 'Element not found: ' + {sel_json}}});
    el.focus();
    if (el.isContentEditable) {{
        el.textContent = '';
    }} else {{
        el.value = '';
    }}
    el.dispatchEvent(new Event('input', {{bubbles: true}}));
    el.dispatchEvent(new Event('change', {{bubbles: true}}));
    return JSON.stringify({{ok: true, tag: el.tagName.toLowerCase()}});
}})()"#
        );

        let clear_result = sup
            .evaluate_js(&js)
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        let clear_str = clear_result.as_str().unwrap_or("{}");
        let clear_info: serde_json::Value = serde_json::from_str(clear_str).unwrap_or_default();
        if let Some(err) = clear_info.get("error").and_then(|v| v.as_str()) {
            return Err(invalid_params(err.to_string()));
        }

        let tag = clear_info.get("tag").and_then(|v| v.as_str()).unwrap_or("?").to_string();

        // Now type the value using CDP insertText
        sup.send_cdp_command(
            "Input.insertText",
            Some(serde_json::json!({"text": params.value})),
        )
        .await
        .map_err(|e| internal_error(e.to_string()))?;

        // Dispatch events after insert
        let dispatch_js = format!(
            r#"(function() {{
    const el = document.querySelector({sel_json});
    if (el) {{
        el.dispatchEvent(new Event('input', {{bubbles: true}}));
        el.dispatchEvent(new Event('change', {{bubbles: true}}));
    }}
}})()"#
        );
        let _ = sup.evaluate_js(&dispatch_js).await;

        self.record_action(&params.instance_id, "fill", &params.selector, &format!("Filled <{tag}> with \"{}\"", params.value)).await;

        let mut msg = format!(
            "Filled <{tag}> ({}) with \"{}\".",
            params.selector, params.value
        );

        if let Some(analysis) = self.vision_verify_action(
            &sup, None, crate::vision::prompts::VERIFY_CLICK
        ).await {
            msg.push_str(&format!("\nVision: {analysis}"));
        }

        Ok(CallToolResult::success(vec![Content::text(msg)]))
    }

    #[tool(description = "Select an option from a <select> dropdown element by value or visible label. Dispatches the 'change' event after selection. Provide either 'value' (option value attribute) or 'label' (visible text).")]
    async fn vscreen_select_option(
        &self,
        Parameters(params): Parameters<SelectOptionParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;

        if params.value.is_none() && params.label.is_none() {
            return Err(McpError::invalid_params(
                "must provide 'value' or 'label'",
                None,
            ));
        }

        let sel_json = serde_json::to_string(&params.selector).unwrap_or_default();
        let val_json = serde_json::to_string(&params.value).unwrap_or_default();
        let label_json = serde_json::to_string(&params.label).unwrap_or_default();

        let js = format!(
            r#"(function() {{
    const sel = document.querySelector({sel_json});
    if (!sel) return JSON.stringify({{error: 'Select element not found: ' + {sel_json}}});
    if (sel.tagName.toLowerCase() !== 'select') return JSON.stringify({{error: 'Element is not a <select>: ' + sel.tagName}});
    const byValue = {val_json};
    const byLabel = {label_json};
    let found = false;
    for (let i = 0; i < sel.options.length; i++) {{
        const opt = sel.options[i];
        if ((byValue !== null && opt.value === byValue) || (byLabel !== null && opt.text.trim() === byLabel)) {{
            sel.selectedIndex = i;
            found = true;
            sel.dispatchEvent(new Event('change', {{bubbles: true}}));
            sel.dispatchEvent(new Event('input', {{bubbles: true}}));
            return JSON.stringify({{ok: true, selectedIndex: i, value: opt.value, text: opt.text.trim()}});
        }}
    }}
    const options = Array.from(sel.options).map(o => ({{value: o.value, text: o.text.trim()}}));
    return JSON.stringify({{error: 'Option not found', available: options}});
}})()"#
        );

        let result = sup
            .evaluate_js(&js)
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        let result_str = result.as_str().unwrap_or("{}");
        let info: serde_json::Value = serde_json::from_str(result_str).unwrap_or_default();

        if let Some(err) = info.get("error").and_then(|v| v.as_str()) {
            let text = serde_json::to_string_pretty(&info).unwrap_or_default();
            return Err(invalid_params(format!("{err}: {text}")));
        }

        let selected_text = info.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let selected_value = info.get("value").and_then(|v| v.as_str()).unwrap_or("").to_string();
        self.record_action(&params.instance_id, "select_option", &params.selector, &format!("Selected \"{selected_text}\" (value={selected_value})")).await;

        let mut msg = format!(
            "Selected option \"{selected_text}\" (value=\"{selected_value}\") in {}.",
            params.selector
        );

        if let Some(analysis) = self.vision_verify_action(
            &sup, None, crate::vision::prompts::VERIFY_CLICK
        ).await {
            msg.push_str(&format!("\nVision: {analysis}"));
        }

        Ok(CallToolResult::success(vec![Content::text(msg)]))
    }

    #[tool(description = "Scroll an element into view by CSS selector. Uses the native scrollIntoView API. Returns the element's new bounding box coordinates for subsequent interaction. PREFER this over manual vscreen_scroll with pixel deltas when targeting a specific element.")]
    async fn vscreen_scroll_to_element(
        &self,
        Parameters(params): Parameters<ScrollToElementParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;

        let sel_json = serde_json::to_string(&params.selector).unwrap_or_default();
        let block = params.block.as_deref().unwrap_or("center");
        let block_json = serde_json::to_string(block).unwrap_or_default();

        let js = format!(
            r#"(function() {{
    const el = document.querySelector({sel_json});
    if (!el) return JSON.stringify({{error: 'Element not found: ' + {sel_json}}});
    el.scrollIntoView({{behavior: 'instant', block: {block_json}, inline: 'nearest'}});
    const r = el.getBoundingClientRect();
    return JSON.stringify({{
        tag: el.tagName.toLowerCase(),
        text: (el.innerText || el.value || '').substring(0, 100).trim(),
        x: Math.round(r.left), y: Math.round(r.top),
        width: Math.round(r.width), height: Math.round(r.height),
        page_x: Math.round(r.left + window.scrollX),
        page_y: Math.round(r.top + window.scrollY),
        center_x: Math.round(r.left + r.width/2 + window.scrollX),
        center_y: Math.round(r.top + r.height/2 + window.scrollY),
    }});
}})()"#
        );

        let result = sup
            .evaluate_js(&js)
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        let result_str = result.as_str().unwrap_or("{}");
        let info: serde_json::Value = serde_json::from_str(result_str).unwrap_or_default();

        if let Some(err) = info.get("error").and_then(|v| v.as_str()) {
            return Err(invalid_params(err.to_string()));
        }

        let text = serde_json::to_string_pretty(&info).unwrap_or_default();
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Scrolled element into view. Position:\n{text}"
        ))]))
    }

    #[tool(description = "List all frames (including iframes) in the page. Returns frame IDs, URLs, names, and bounding rectangles. Use this to understand page structure before interacting with iframe content. Each iframe's bounding rect gives page-space coordinates that can be used with vscreen_screenshot(clip=...) to zoom in. Related: vscreen_find_elements(include_iframes=true), vscreen_find_by_text(include_iframes=true).")]
    async fn vscreen_list_frames(
        &self,
        Parameters(params): Parameters<ListFramesParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;

        let frame_tree = sup
            .get_frame_tree()
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        let iframe_rects_js = r#"JSON.stringify(Array.from(document.querySelectorAll('iframe')).map(f => {
    const r = f.getBoundingClientRect();
    return {
        src: f.src || '',
        name: f.name || '',
        title: f.title || '',
        id: f.id || '',
        x: Math.round(r.left + window.scrollX),
        y: Math.round(r.top + window.scrollY),
        width: Math.round(r.width),
        height: Math.round(r.height),
        visible: r.width > 0 && r.height > 0 && r.top > -9000,
    };
}))"#;

        let rects_result = sup
            .evaluate_js(iframe_rects_js)
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        let rects_str = rects_result.as_str().unwrap_or("[]");
        let frame_tree_str = serde_json::to_string_pretty(&frame_tree).unwrap_or_default();

        let mut text = format!("Frame tree:\n{frame_tree_str}\n\nIframe bounding rects:\n{rects_str}");
        if text.len() > 8000 {
            text.truncate(8000);
            text.push_str("\n... (truncated)");
        }
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    // -----------------------------------------------------------------------
    // Navigation and input discovery tools
    // -----------------------------------------------------------------------

    #[tool(description = "Find text input elements on the page by placeholder, aria-label, associated label text, role, name attribute, or input type. Returns matching inputs with their CSS selector, bounding box, and current value. Abstracts away implementation-specific selectors (e.g. YouTube's ytSearchboxComponentInput). At least one search parameter must be provided.")]
    async fn vscreen_find_input(
        &self,
        Parameters(params): Parameters<FindInputParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;

        if params.placeholder.is_none()
            && params.aria_label.is_none()
            && params.label.is_none()
            && params.role.is_none()
            && params.name.is_none()
            && params.input_type.is_none()
        {
            return Err(McpError::invalid_params(
                "at least one search parameter required (placeholder, aria_label, label, role, name, or input_type)",
                None,
            ));
        }

        let mut filters = Vec::new();

        if let Some(ref ph) = params.placeholder {
            let v = serde_json::to_string(ph).unwrap_or_default();
            filters.push(format!(
                "(el.placeholder || '').toLowerCase().includes({v}.toLowerCase())"
            ));
        }
        if let Some(ref al) = params.aria_label {
            let v = serde_json::to_string(al).unwrap_or_default();
            filters.push(format!(
                "(el.getAttribute('aria-label') || '').toLowerCase().includes({v}.toLowerCase())"
            ));
        }
        if let Some(ref r) = params.role {
            let v = serde_json::to_string(r).unwrap_or_default();
            filters.push(format!(
                "(el.getAttribute('role') || '').toLowerCase() === {v}.toLowerCase()"
            ));
        }
        if let Some(ref n) = params.name {
            let v = serde_json::to_string(n).unwrap_or_default();
            filters.push(format!("el.name === {v}"));
        }
        if let Some(ref t) = params.input_type {
            let v = serde_json::to_string(t).unwrap_or_default();
            filters.push(format!("(el.type || 'text').toLowerCase() === {v}.toLowerCase()"));
        }

        let label_filter = if let Some(ref lbl) = params.label {
            let v = serde_json::to_string(lbl).unwrap_or_default();
            format!(
                r#"|| (function() {{
                    if (el.id) {{
                        const lbl = document.querySelector('label[for="' + el.id + '"]');
                        if (lbl && (lbl.innerText || '').toLowerCase().includes({v}.toLowerCase())) return true;
                    }}
                    const parent = el.closest('label');
                    if (parent && (parent.innerText || '').toLowerCase().includes({v}.toLowerCase())) return true;
                    return false;
                }})()"#
            )
        } else {
            String::new()
        };

        let filter_expr = if filters.is_empty() {
            format!("(false {label_filter})")
        } else {
            format!("({} {label_filter})", filters.join(" || "))
        };

        let js = format!(
            r#"JSON.stringify((function() {{
    const inputs = document.querySelectorAll('input, textarea, select, [contenteditable="true"], [role="textbox"], [role="searchbox"], [role="combobox"]');
    const results = [];
    for (const el of inputs) {{
        const r = el.getBoundingClientRect();
        if (r.width <= 0 || r.height <= 0) continue;
        if (!({filter_expr})) continue;
        const id = el.id ? '#' + el.id : '';
        const name = el.name ? '[name="' + el.name + '"]' : '';
        const tag = el.tagName.toLowerCase();
        const type_attr = el.type ? '[type="' + el.type + '"]' : '';
        const selector = id || (tag + name + type_attr) || tag;
        results.push({{
            tag: tag,
            selector: selector,
            type: el.type || null,
            placeholder: el.placeholder || null,
            aria_label: el.getAttribute('aria-label') || null,
            role: el.getAttribute('role') || null,
            name: el.name || null,
            value: (el.value || el.textContent || '').substring(0, 200),
            page_x: Math.round(r.left + r.width/2 + window.scrollX),
            page_y: Math.round(r.top + r.height/2 + window.scrollY),
            width: Math.round(r.width),
            height: Math.round(r.height),
        }});
        if (results.length >= 20) break;
    }}
    return results;
}})())"#
        );

        let result = sup.evaluate_js(&js).await
            .map_err(|e| internal_error(e.to_string()))?;

        let result_str = result.as_str().unwrap_or("[]");
        let elements: Vec<serde_json::Value> = serde_json::from_str(result_str).unwrap_or_default();

        if elements.is_empty() {
            Ok(CallToolResult::success(vec![Content::text(
                "No matching input elements found. Try broader search criteria, or use vscreen_find_elements with a CSS selector.",
            )]))
        } else {
            let text = serde_json::to_string_pretty(&elements).unwrap_or_default();
            Ok(CallToolResult::success(vec![Content::text(format!(
                "Found {} input(s):\n{text}\n\nTip: use vscreen_fill(selector, value) to type into a match.",
                elements.len()
            ))]))
        }
    }

    #[tool(description = "Click an element and wait for navigation (URL change). Clicks the element found by selector or text, then polls for a URL change. If the URL doesn't change and fallback_to_link is true (default), tries clicking the nearest <a> ancestor or navigating to its href directly. Returns the new page URL and title. Ideal for SPA navigation where clicks trigger pushState.")]
    async fn vscreen_click_and_navigate(
        &self,
        Parameters(params): Parameters<ClickAndNavigateParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;

        if params.selector.is_none() && params.text.is_none() {
            return Err(McpError::invalid_params(
                "must provide 'selector' or 'text' (or both)",
                None,
            ));
        }

        let timeout = params.timeout_ms.unwrap_or(5000);

        // Capture the URL before clicking
        let before_url = sup.evaluate_js("location.href").await
            .map_err(|e| internal_error(e.to_string()))?;
        let before_url = before_url.as_str().unwrap_or("").to_string();

        // Build JS that finds the element, scrolls into view, clicks it, and returns element info + href
        let find_and_info_js = {
            let find_expr = if let Some(ref sel) = params.selector {
                let sel_json = serde_json::to_string(sel).unwrap_or_default();
                format!("document.querySelector({sel_json})")
            } else {
                let t_json = serde_json::to_string(params.text.as_deref().unwrap_or("")).unwrap_or_default();
                format!(
                    r#"(function() {{
    const searchText = {t_json}.toLowerCase();
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_ELEMENT);
    let best = null;
    let bestArea = Infinity;
    let el;
    while (el = walker.nextNode()) {{
        const t = (el.innerText || '').toLowerCase();
        if (!t.includes(searchText)) continue;
        const r = el.getBoundingClientRect();
        if (r.width <= 0 || r.height <= 0) continue;
        const area = r.width * r.height;
        if (area < bestArea) {{
            best = el;
            bestArea = area;
        }}
    }}
    return best;
}})()"#
                )
            };
            format!(
                r#"JSON.stringify((function() {{
    const el = {find_expr};
    if (!el) return {{error: 'Element not found'}};
    el.scrollIntoView({{behavior: 'instant', block: 'center', inline: 'nearest'}});
    const r = el.getBoundingClientRect();
    const link = el.closest('a') || el.querySelector('a');
    return {{
        tag: el.tagName.toLowerCase(),
        text: (el.innerText || '').substring(0, 200).trim(),
        page_x: Math.round(r.left + r.width/2 + window.scrollX),
        page_y: Math.round(r.top + r.height/2 + window.scrollY),
        href: el.href || (link ? link.href : null),
    }};
}})())"#
            )
        };

        let result = sup.evaluate_js(&find_and_info_js).await
            .map_err(|e| internal_error(e.to_string()))?;
        let result_str = result.as_str().unwrap_or("{}");
        let el_info: serde_json::Value = serde_json::from_str(result_str).unwrap_or_default();

        if let Some(err) = el_info.get("error").and_then(|v| v.as_str()) {
            return Ok(CallToolResult::success(vec![Content::text(format!(
                "Navigation failed: {err}",
            ))]));
        }

        let cx = el_info.get("page_x").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let cy = el_info.get("page_y").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let href = el_info.get("href").and_then(|v| v.as_str()).map(String::from);

        // Click the element
        let (vx, vy) = sup.scroll_into_view_and_translate(cx, cy).await
            .map_err(|e| internal_error(e.to_string()))?;

        for (typ, buttons) in [("mouseMoved", 0u32), ("mousePressed", 1u32), ("mouseReleased", 0u32)] {
            let mut p = serde_json::json!({"type": typ, "x": vx, "y": vy, "modifiers": 0});
            if typ != "mouseMoved" {
                p["button"] = serde_json::json!("left");
                p["buttons"] = serde_json::json!(buttons);
                p["clickCount"] = serde_json::json!(1);
            }
            sup.send_cdp_command("Input.dispatchMouseEvent", Some(p)).await
                .map_err(|e| internal_error(e.to_string()))?;
        }

        // Poll for URL change
        let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout);
        let mut navigated = false;
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(200)).await;
            if let Ok(url_val) = sup.evaluate_js("location.href").await {
                if let Some(current_url) = url_val.as_str() {
                    if current_url != before_url {
                        navigated = true;
                        break;
                    }
                }
            }
        }

        // Fallback: try direct navigation via href if available
        if !navigated && params.fallback_to_link {
            if let Some(ref link_url) = href {
                if !link_url.is_empty() && link_url != &before_url {
                    let nav_js = format!("window.location.href = {}", serde_json::to_string(link_url).unwrap_or_default());
                    let _ = sup.evaluate_js(&nav_js).await;
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    navigated = true;
                }
            }
        }

        // Get final page info
        let page_info = sup.evaluate_js("JSON.stringify({url: location.href, title: document.title})").await
            .map_err(|e| internal_error(e.to_string()))?;
        let page_str = page_info.as_str().unwrap_or("{}");
        let page: serde_json::Value = serde_json::from_str(page_str).unwrap_or_default();
        let final_url = page.get("url").and_then(|v| v.as_str()).unwrap_or("?");
        let title = page.get("title").and_then(|v| v.as_str()).unwrap_or("?");

        let tag = el_info.get("tag").and_then(|v| v.as_str()).unwrap_or("?");
        let el_text = el_info.get("text").and_then(|v| v.as_str()).unwrap_or("");

        self.record_action(&params.instance_id, "click_and_navigate", el_text, &format!("Navigated to {final_url}")).await;

        let vision_note = self.vision_verify_action(
            &sup, None, crate::vision::prompts::VERIFY_CLICK
        ).await;
        let vision_suffix = vision_note.map_or(String::new(), |a| format!("\nVision: {a}"));

        if navigated {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "Clicked <{tag}> \"{el_text}\" — navigation detected.\nURL: {final_url}\nTitle: {title}{vision_suffix}",
            ))]))
        } else {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "Clicked <{tag}> \"{el_text}\" but URL did not change after {timeout}ms.\nURL: {final_url}\nTitle: {title}\nHint: the page may use client-side rendering without URL changes, or the click missed the target.{vision_suffix}",
            ))]))
        }
    }

    #[tool(description = "Detect and dismiss video platform ad overlays (YouTube skip button, pre-roll ads, interstitial ads). Uses CSS selectors only (no text matching) to avoid false positives. Waits for skip button to appear (video ads often have a countdown), then clicks it. Falls back to vision LLM if configured. Set timeout_ms to control how long to wait for skip button (default: 15000ms).")]
    async fn vscreen_dismiss_ads(
        &self,
        Parameters(params): Parameters<DismissAdsParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        let timeout = params.timeout_ms.unwrap_or(15000);

        // CSS-only detection — returns element coordinates for CDP click (not JS .click())
        let find_ad_button_js = r#"(function() {
    // YouTube skip button selectors (various versions)
    const skipSelectors = [
        '.ytp-skip-ad-button',
        '.ytp-ad-skip-button',
        '.ytp-ad-skip-button-modern',
        'button.ytp-ad-skip-button',
        '.videoAdUiSkipButton',
        '[id^="skip-button"]',
    ];
    for (const sel of skipSelectors) {
        const el = document.querySelector(sel);
        if (el && el.offsetWidth > 0 && el.offsetHeight > 0) {
            const r = el.getBoundingClientRect();
            return JSON.stringify({found: true, type: 'skip_button', selector: sel,
                text: (el.innerText || '').trim(),
                x: Math.round(r.left + r.width / 2 + window.scrollX),
                y: Math.round(r.top + r.height / 2 + window.scrollY)});
        }
    }
    // Ad-specific close/overlay buttons
    const adCloseSelectors = [
        '.ytp-ad-overlay-close-button',
        '.ad-close-button',
        '.close-ad',
        '[aria-label="Close ad"]',
    ];
    for (const sel of adCloseSelectors) {
        const el = document.querySelector(sel);
        if (el && el.offsetWidth > 0 && el.offsetHeight > 0) {
            const r = el.getBoundingClientRect();
            return JSON.stringify({found: true, type: 'close_overlay', selector: sel,
                text: (el.innerText || '').trim(),
                x: Math.round(r.left + r.width / 2 + window.scrollX),
                y: Math.round(r.top + r.height / 2 + window.scrollY)});
        }
    }
    // Detect if ad is playing but skip not yet available
    const adPlaying = document.querySelector('.ad-showing, .ytp-ad-player-overlay, [class*="ad-interrupting"]');
    if (adPlaying) {
        return JSON.stringify({found: false, ad_detected: true, message: 'Ad playing but skip button not yet available'});
    }
    return JSON.stringify({found: false, ad_detected: false});
})()"#;

        let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout);
        let mut last_result = serde_json::json!({"found": false, "ad_detected": false});

        loop {
            let result = sup.evaluate_js(find_ad_button_js).await
                .map_err(|e| internal_error(e.to_string()))?;
            let result_str = result.as_str().unwrap_or("{}");
            let info: serde_json::Value = serde_json::from_str(result_str).unwrap_or_default();

            if info.get("found").and_then(|v| v.as_bool()).unwrap_or(false) {
                let dtype = info.get("type").and_then(|v| v.as_str()).unwrap_or("?");
                let text = info.get("text").and_then(|v| v.as_str()).unwrap_or("");
                let click_x = info.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let click_y = info.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);

                // Use CDP Input.dispatchMouseEvent for a real browser click
                let (vx, vy) = sup
                    .scroll_into_view_and_translate(click_x, click_y)
                    .await
                    .map_err(|e| internal_error(e.to_string()))?;

                for (typ, buttons) in [("mouseMoved", 0u32), ("mousePressed", 1u32), ("mouseReleased", 0u32)] {
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

                self.record_action(&params.instance_id, "dismiss_ad", text, &format!("Dismissed ad via {dtype}")).await;
                return Ok(CallToolResult::success(vec![Content::text(format!(
                    "Ad dismissed via {dtype}: \"{text}\" (clicked at {click_x},{click_y})",
                ))]));
            }

            last_result = info.clone();

            if tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        // Tier 3: Vision LLM fallback when ad is detected but selectors failed
        let ad_detected = last_result.get("ad_detected").and_then(|v| v.as_bool()).unwrap_or(false);
        if ad_detected {
            if let Some(ref vision) = self.state.vision_client {
                if vision.is_available().await {
                    if let Ok(screenshot) = sup.capture_vision_screenshot().await {
                        match vision.analyze(&screenshot, crate::vision::prompts::DISMISS_AD).await {
                            Ok(resp) if resp.found => {
                                if let Some(target) = resp.click_target {
                                    // Vision screenshot uses 0.5x scale — coordinates must
                                    // be doubled to convert back to page-space.
                                    let page_x = target.x * 2.0;
                                    let page_y = target.y * 2.0;
                                    let (vx, vy) = sup
                                        .scroll_into_view_and_translate(page_x, page_y)
                                        .await
                                        .map_err(|e| internal_error(e.to_string()))?;

                                    for (typ, buttons) in [("mouseMoved", 0u32), ("mousePressed", 1u32), ("mouseReleased", 0u32)] {
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

                                    let desc = target.description.as_deref().unwrap_or("skip button");
                                    self.record_action(&params.instance_id, "dismiss_ad", desc, "Dismissed ad via vision LLM").await;
                                    return Ok(CallToolResult::success(vec![Content::text(format!(
                                        "Ad dismissed via vision LLM: \"{desc}\" at ({page_x}, {page_y})",
                                    ))]));
                                }
                            }
                            Ok(_) => { /* vision says no skip button */ }
                            Err(e) => {
                                debug!("vision LLM error for dismiss_ads: {e}");
                            }
                        }
                    }
                }
            }

            let msg = last_result.get("message").and_then(|v| v.as_str()).unwrap_or("Ad detected but could not dismiss");
            Ok(CallToolResult::success(vec![Content::text(format!(
                "Timed out after {timeout}ms. {msg}. The ad may need to play through.",
            ))]))
        } else {
            Ok(CallToolResult::success(vec![Content::text(
                "No video ad overlay detected on the current page.",
            )]))
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
// Task-to-tool routing patterns for vscreen_plan
// ---------------------------------------------------------------------------

struct TaskPattern {
    name: &'static str,
    keywords: &'static [&'static str],
    negative_keywords: &'static [&'static str],
    recommendation: &'static str,
}

static TASK_PATTERNS: &[TaskPattern] = &[
    TaskPattern {
        name: "Read page text",
        keywords: &["read text", "read the text", "read all", "get text", "page content", "what does the page say", "extract text", "page text", "all text", "visible text", "text on the page", "text content"],
        negative_keywords: &["click", "fill", "screenshot"],
        recommendation: "\
1. `vscreen_extract_text(instance_id)` — returns all visible text without screenshots\n\
   - Optionally pass `selector` to extract from a specific element\n\
   - Much faster and more accurate than screenshot + OCR",
    },
    TaskPattern {
        name: "See the full page",
        keywords: &["full page", "whole page", "entire page", "see everything", "overview", "what is on the page", "see the page"],
        negative_keywords: &[],
        recommendation: "\
1. `vscreen_screenshot(instance_id, full_page=true)` — captures the entire scrollable document in one image\n\
   - Do NOT scroll + screenshot repeatedly\n\
   - Use `clip={x,y,width,height}` to zoom into a specific region",
    },
    TaskPattern {
        name: "Find and click a button/link",
        keywords: &["click", "press", "tap", "button", "link", "sign in", "submit", "login", "log in"],
        negative_keywords: &["fill", "form", "type", "captcha"],
        recommendation: "\
1. `vscreen_click_element(instance_id, text=\"Button Text\")` — finds and clicks by visible text (main frame, with retries)\n\
   OR: `vscreen_click_element(instance_id, selector=\"button.submit\")` — by CSS selector\n\
   - For iframe elements: `vscreen_find_by_text(text=\"...\", include_iframes=true)` then `vscreen_click(x, y)`\n\
   - For click + navigation: `vscreen_click_and_navigate(text=\"Link Text\")`",
    },
    TaskPattern {
        name: "Fill a form",
        keywords: &["fill", "form", "enter", "type", "input", "field", "username", "password", "email"],
        negative_keywords: &["captcha"],
        recommendation: "\
1. `vscreen_find_input(instance_id, placeholder=\"...\")` or `vscreen_find_input(label=\"...\")` — discover form fields\n\
2. `vscreen_fill(instance_id, selector=\"input[name='field']\", value=\"...\")` — clear + fill each field\n\
3. `vscreen_click_element(instance_id, text=\"Submit\")` — submit the form\n\
   - Use `vscreen_screenshot` after submit to verify",
    },
    TaskPattern {
        name: "Navigate and extract data",
        keywords: &["navigate", "go to", "open", "visit", "load"],
        negative_keywords: &[],
        recommendation: "\
1. `vscreen_navigate(instance_id, url=\"...\", wait_until=\"load\")` — load the page\n\
2. `vscreen_dismiss_dialogs(instance_id)` — clear cookie consent / GDPR overlays\n\
3. `vscreen_extract_text(instance_id)` — get all text content\n\
   OR: `vscreen_screenshot(instance_id, full_page=true)` — see the visual layout",
    },
    TaskPattern {
        name: "Wait for dynamic content",
        keywords: &["wait", "loading", "spinner", "appear", "show up", "dynamic", "ajax", "async"],
        negative_keywords: &[],
        recommendation: "\
Use targeted waits instead of fixed delays:\n\
- `vscreen_wait_for_text(instance_id, text=\"...\")` — wait for specific text to appear\n\
- `vscreen_wait_for_selector(instance_id, selector=\"...\")` — wait for an element to exist\n\
- `vscreen_wait_for_url(instance_id, url_contains=\"...\")` — wait for navigation\n\
- `vscreen_wait_for_network_idle(instance_id)` — wait for all network requests to complete\n\
Do NOT use `vscreen_wait(5000)` loops.",
    },
    TaskPattern {
        name: "Find an element",
        keywords: &["find", "locate", "where is", "search for", "element"],
        negative_keywords: &["click", "fill", "text content"],
        recommendation: "\
- By visible text: `vscreen_find_by_text(instance_id, text=\"Sign In\")`\n\
- By CSS selector: `vscreen_find_elements(instance_id, selector=\"button.primary\")`\n\
- Set `include_iframes=true` when elements might be inside iframes\n\
- For semantic structure: `vscreen_accessibility_tree(instance_id)`",
    },
    TaskPattern {
        name: "Solve CAPTCHA",
        keywords: &["captcha", "recaptcha", "verify", "robot"],
        negative_keywords: &[],
        recommendation: "\
1. `vscreen_solve_captcha(instance_id)` — fully automated reCAPTCHA v2 solving\n\
   - Handles checkbox click, tile selection, verification, retries\n\
   - Requires vision LLM (--vision-url)\n\
   - For manual workflow: `vscreen_help(topic=\"captcha\")`",
    },
    TaskPattern {
        name: "Dismiss popups/overlays",
        keywords: &["dismiss", "close", "popup", "overlay", "cookie", "consent", "banner", "gdpr", "dialog"],
        negative_keywords: &[],
        recommendation: "\
1. `vscreen_dismiss_dialogs(instance_id)` — auto-detects and dismisses cookie consent, GDPR, privacy dialogs\n\
   - Supports OneTrust, CookieBot, Didomi, Quantcast, TrustArc, and many more\n\
   - For video ads: `vscreen_dismiss_ads(instance_id)`",
    },
    TaskPattern {
        name: "Scroll to content",
        keywords: &["scroll", "below", "bottom", "down", "more content"],
        negative_keywords: &[],
        recommendation: "\
- To see below-fold content: `vscreen_screenshot(instance_id, full_page=true)` — captures everything\n\
- To bring a specific element into view: `vscreen_scroll_to_element(instance_id, selector=\"#target\")`\n\
- Do NOT use `vscreen_scroll` + screenshot loops",
    },
    TaskPattern {
        name: "Get page metadata",
        keywords: &["url", "title", "page info", "current page", "what page"],
        negative_keywords: &["text", "content", "extract"],
        recommendation: "\
1. `vscreen_get_page_info(instance_id)` — returns URL, title, viewport dimensions, scroll position\n\
   - Do NOT use `vscreen_execute_js(\"document.title\")` for this",
    },
    TaskPattern {
        name: "Work with iframes",
        keywords: &["iframe", "frame", "embed", "widget", "recaptcha iframe"],
        negative_keywords: &["captcha solve"],
        recommendation: "\
1. `vscreen_list_frames(instance_id)` — discover all iframes with bounding rects\n\
2. `vscreen_find_by_text(text=\"...\", include_iframes=true)` — find elements inside iframes\n\
3. `vscreen_click(x, y)` — click using page-space coordinates from find tools\n\
   - `vscreen_click_element` does NOT search iframes\n\
   - Use `vscreen_screenshot(clip={x,y,w,h})` to zoom into an iframe region",
    },
    TaskPattern {
        name: "Navigate between pages",
        keywords: &["back", "forward", "history", "previous page", "go back", "next page"],
        negative_keywords: &[],
        recommendation: "\
- `vscreen_go_back(instance_id)` — browser back button\n\
- `vscreen_go_forward(instance_id)` — browser forward button\n\
- `vscreen_reload(instance_id)` — refresh the page",
    },
    TaskPattern {
        name: "Take multiple screenshots",
        keywords: &["animation", "transition", "sequence", "recording", "multiple screenshots", "over time"],
        negative_keywords: &[],
        recommendation: "\
1. `vscreen_screenshot_sequence(instance_id, count=5, interval_ms=1000)` — capture N screenshots at regular intervals\n\
   - Useful for observing animations, transitions, or verifying timed actions",
    },
    TaskPattern {
        name: "Identify unlabeled elements",
        keywords: &["icon", "unlabeled", "no text", "what is this button", "describe"],
        negative_keywords: &[],
        recommendation: "\
1. `vscreen_describe_elements(instance_id)` — uses vision LLM to identify icon-only buttons and unlabeled UI elements\n\
   - Requires --vision-url\n\
   - Try `vscreen_find_elements` or `vscreen_find_by_text` first for labeled elements",
    },
];

// ---------------------------------------------------------------------------
// Documentation constants
// ---------------------------------------------------------------------------

const SERVER_INSTRUCTIONS: &str = r#"# vscreen MCP Server

Control headless Chromium browser instances for AI-driven web automation.

## Core Workflow

1. **Discover** — Call `vscreen_list_instances` to find available browser instances. Each returns an `instance_id` (e.g., "dev") used in all subsequent calls.
2. **Navigate** — Call `vscreen_navigate(instance_id, url, wait_until="load")` to load a page. Returns the final URL and page title.
3. **Observe** — Call `vscreen_screenshot(instance_id)` to capture the current page. Use `clip: {x, y, width, height}` to zoom into a specific region.
4. **Discover Elements** — Call `vscreen_find_elements(selector)` or `vscreen_find_by_text(text)` to locate interactive elements. Returns page-space coordinates you can pass directly to click/hover.
5. **Act** — Call `vscreen_click(x, y)`, `vscreen_type(text)`, `vscreen_fill(selector, value)`, etc. to interact with the page.
6. **Verify** — Take another screenshot to confirm the action had the desired effect.
7. **Repeat** steps 3-6 as needed.

## Coordinate System

All coordinates are in **page-space** (absolute position on the full page, not viewport-relative). The system automatically scrolls elements into view before clicking. Coordinates returned by `vscreen_find_elements` and `vscreen_find_by_text` can be passed directly to `vscreen_click` without any translation.

## Iframe Handling

Cross-origin iframes (e.g., reCAPTCHA, embedded widgets) require special handling:
- Use `vscreen_list_frames` to discover all iframes and their bounding rectangles.
- Use `vscreen_find_elements(include_iframes=true)` or `vscreen_find_by_text(include_iframes=true)` to search inside iframes. Returned coordinates are already translated to page-space.
- Click iframe elements using the returned page-space coordinates with `vscreen_click`.
- The `frame_id` in results identifies which iframe contains each element.

## Connection Modes

vscreen supports multiple ways to connect:

- **SSE direct (recommended for Cursor)**: Start the server with `vscreen --dev --mcp-sse 0.0.0.0:8451`, then configure your MCP client with `{"url": "http://localhost:8451/mcp"}`. The server runs independently and survives client reconnections.
- **Stdio proxy**: For MCP clients that only support subprocess spawning, use `vscreen --mcp-stdio-proxy http://localhost:8451/mcp`. This lightweight proxy forwards messages to an existing SSE server without starting its own dev environment.
- **Stdio direct**: `vscreen --mcp-stdio --dev` starts a full server in subprocess mode. Not recommended when another vscreen instance is already running (causes resource conflicts).

**Best practice**: Start the server once with `--dev --mcp-sse`, then connect via SSE URL or stdio proxy.

## Instance Locking

- **Single-agent mode** (default with `--mcp-stdio`): Locks are auto-acquired. No explicit locking needed.
- **Multi-agent mode**: Call `vscreen_instance_lock(lock_type="exclusive")` before interacting. Release with `vscreen_instance_unlock` when done.

## Tool Categories

- **Observation** (read-only): `vscreen_screenshot`, `vscreen_find_elements`, `vscreen_find_by_text`, `vscreen_get_page_info`, `vscreen_list_frames`, `vscreen_extract_text`, `vscreen_accessibility_tree`, `vscreen_screenshot_annotated`, `vscreen_describe_elements`
- **Input Actions**: `vscreen_click`, `vscreen_type`, `vscreen_fill`, `vscreen_key_press`, `vscreen_key_combo`, `vscreen_scroll`, `vscreen_drag`, `vscreen_hover`, `vscreen_batch_click`, `vscreen_click_element`, `vscreen_select_option`, `vscreen_scroll_to_element`
- **Navigation**: `vscreen_navigate`, `vscreen_go_back`, `vscreen_go_forward`, `vscreen_reload`
- **Synchronization**: `vscreen_wait`, `vscreen_wait_for_idle`, `vscreen_wait_for_text`, `vscreen_wait_for_selector`, `vscreen_wait_for_url`, `vscreen_wait_for_network_idle`
- **Memory & Context**: `vscreen_history_list`, `vscreen_history_get`, `vscreen_session_log`, `vscreen_session_summary`, `vscreen_console_log`
- **Cookie & Storage**: `vscreen_get_cookies`, `vscreen_set_cookie`, `vscreen_get_storage`, `vscreen_set_storage`
- **Utility**: `vscreen_dismiss_dialogs`, `vscreen_execute_js`, `vscreen_help`

## Tool Selection Rules

OBSERVE the full page:
- NEVER scroll + screenshot repeatedly. Use `vscreen_screenshot(full_page=true)` for the entire document in one call.
- Use `vscreen_screenshot(clip={x,y,width,height})` to zoom into a specific region at full resolution.

READ page content:
- NEVER screenshot to read text. Use `vscreen_extract_text` for all visible text in one call.
- Use `vscreen_get_page_info` for URL, title, viewport — not `vscreen_execute_js("document.title")`.

FIND elements:
- Use `vscreen_find_by_text` when you know the visible label (e.g. "Sign In").
- Use `vscreen_find_elements` when you have a CSS selector.
- Use `vscreen_accessibility_tree` for semantic page structure.
- Set `include_iframes=true` when elements might be inside iframes.

CLICK elements:
- Main frame by text/selector: prefer `vscreen_click_element` (has retries + wait built-in).
- By coordinates (from find tools) or inside iframes: use `vscreen_click`.
- Rapid multi-click (CAPTCHA tiles): use `vscreen_batch_click`.
- Click + expect navigation: use `vscreen_click_and_navigate`.

WAIT for changes:
- NEVER use `vscreen_wait(5000)` + screenshot loops. Use:
  - `vscreen_wait_for_text` when expecting specific text to appear.
  - `vscreen_wait_for_selector` when expecting an element to appear.
  - `vscreen_wait_for_url` when expecting navigation.
  - `vscreen_wait_for_network_idle` when waiting for async data loads.

SCROLL:
- To bring an element into view: use `vscreen_scroll_to_element` (not manual scroll deltas).
- To capture below-fold content: use `vscreen_screenshot(full_page=true)` instead of scrolling.

PLAN complex tasks:
- Call `vscreen_plan(task="...")` before multi-step workflows to get the optimal tool sequence.

## Error Recovery

- If a click doesn't produce the expected result, take a screenshot and re-analyze.
- If an element isn't found, try `include_iframes: true`, a broader CSS selector, or `vscreen_wait_for_selector` first.
- If a page has a cookie consent overlay, call `vscreen_dismiss_dialogs` first.
- Call `vscreen_help(topic)` for detailed guidance on any tool or workflow.
"#;

const DOC_QUICKSTART: &str = r#"# vscreen Quick Start Guide

## 1. Discover Instances

```json
// Call vscreen_list_instances (no arguments)
// Returns: [{"instance_id": "dev", "state": {"state": "running"}, ...}]
```

## 2. Navigate to a Page

```json
// Call vscreen_navigate
{"instance_id": "dev", "url": "https://example.com", "wait_until": "load"}
// Returns: "Navigated to https://example.com\nTitle: Example Domain"
```

`wait_until` options:
- `"load"` — Wait for the load event (recommended default)
- `"domcontentloaded"` — Wait for DOM ready (faster, but images may not be loaded)
- `"networkidle"` — Wait until network is quiet for 500ms (slowest, most complete)
- `"none"` — Don't wait at all

## 3. Take a Screenshot

```json
// Call vscreen_screenshot
{"instance_id": "dev"}
// Returns: base64-encoded PNG image
```

Options:
- `format`: `"png"` (default), `"jpeg"`, `"webp"`
- `quality`: 0-100 (for jpeg/webp)
- `full_page`: true to capture the entire scrollable page
- `clip`: `{"x": 100, "y": 200, "width": 400, "height": 300}` to capture a region

## 4. Find Elements

```json
// By CSS selector:
{"instance_id": "dev", "selector": "button.submit", "include_iframes": true}
// Returns: [{"tag": "button", "text": "Submit", "x": 150, "y": 400, "width": 100, "height": 40, ...}]

// By visible text:
{"instance_id": "dev", "text": "Sign In", "include_iframes": true}
// Returns: [{"tag": "a", "text": "Sign In", "x": 800, "y": 20, "width": 60, "height": 20, ...}]
```

## 5. Click an Element

```json
// Use coordinates from find_elements/find_by_text:
{"instance_id": "dev", "x": 150, "y": 400}
// The system auto-scrolls the element into view before clicking.
```

## 6. Type Text

```json
// Type into the focused element:
{"instance_id": "dev", "text": "Hello World"}

// Or fill a specific input field (clears first):
{"instance_id": "dev", "selector": "input[name='email']", "value": "user@example.com"}
```

## 7. Verify Results

Take another screenshot after each action to confirm it worked as expected.
"#;

const DOC_CAPTCHA: &str = r#"# Solving reCAPTCHA Challenges

## Automated (Recommended)

Use `vscreen_solve_captcha` for fully automated reCAPTCHA v2 solving:

```json
{"instance_id": "dev"}
```

This tool:
1. Finds the "I'm not a robot" checkbox in the reCAPTCHA iframe
2. Clicks it to trigger the image challenge
3. Uses vision LLM to identify which tiles match the target object
4. Clicks all matching tiles + VERIFY in rapid succession via batch click
5. Handles multi-round challenges (reCAPTCHA often requires 2-5 rounds)
6. Retries with page reload if the challenge expires

Parameters:
- `instance_id` (required): The browser instance
- `max_attempts` (default: 3): Max page-reload retry cycles

Returns: `{solved: bool, rounds: N, attempts: N, details: [...]}`

**Requires** `--vision-url` to be configured.

## Manual Workflow (Fallback)

If vision LLM is not available or for non-reCAPTCHA CAPTCHAs:

1. `vscreen_find_by_text("I'm not a robot", include_iframes=true)` — Find checkbox
2. `vscreen_click(x, y)` — Click the checkbox center coordinates
3. `vscreen_wait(3000)` — Wait for challenge
4. `vscreen_screenshot(clip={challenge_iframe_bounds})` — Capture tiles
5. Identify correct tiles visually
6. `vscreen_batch_click(points=[...tiles, verify_btn], delay_between_ms=150)` — Click all at once
7. `vscreen_screenshot` — Check result

### Grid Coordinates Reference

Challenge iframe is typically at (85, 84, 400x580). Tile centers:

**3x3 grid** (9 separate images):
```
Tile 1: (152, 249)  Tile 2: (285, 249)  Tile 3: (418, 249)
Tile 4: (152, 389)  Tile 5: (285, 389)  Tile 6: (418, 389)
Tile 7: (152, 529)  Tile 8: (285, 529)  Tile 9: (418, 529)
```

**4x4 grid** (single image split into tiles):
```
Tile 1:  (135, 237)  Tile 2:  (235, 237)  Tile 3:  (335, 237)  Tile 4:  (435, 237)
Tile 5:  (135, 342)  Tile 6:  (235, 342)  Tile 7:  (335, 342)  Tile 8:  (435, 342)
Tile 9:  (135, 447)  Tile 10: (235, 447)  Tile 11: (335, 447)  Tile 12: (435, 447)
Tile 13: (135, 552)  Tile 14: (235, 552)  Tile 15: (335, 552)  Tile 16: (435, 552)
```

**VERIFY/SKIP button**: ~(435, 634)

### Important Notes

- Use `vscreen_batch_click` — individual clicks are too slow for the 2-minute timer
- After challenge transitions, `find_elements` may return stale tiles; use hardcoded positions
- Always reload the page after a timeout (don't re-click the expired checkbox)
- reCAPTCHA may require 2-5 rounds of image selection
"#;

const DOC_WORKFLOWS: &str = r#"# Common Workflows

## Solving reCAPTCHA

Use `vscreen_solve_captcha(instance_id)` for automated solving, or see `vscreen_help(topic="captcha")` for the full manual workflow and grid coordinate reference.

## Handling Cookie Consent Dialogs

1. First try `vscreen_dismiss_dialogs(instance_id)` — Handles OneTrust, CookieBot, and common patterns
2. If that fails: `vscreen_find_by_text("Accept")` or `vscreen_find_by_text("I agree")`
3. `vscreen_click` on the found button
4. Cookies persist across navigations (browser profile is saved)

## Extracting Article Text

1. `vscreen_navigate(url, wait_until="load")` — Load the article
2. `vscreen_dismiss_dialogs` — Clear any overlays
3. `vscreen_extract_text` — Get all visible text from the page
4. Or for structured extraction: `vscreen_find_elements("article, main, .content")` then `vscreen_execute_js` with a custom text extraction script

## Filling Forms

1. `vscreen_find_elements("input, select, textarea")` — Discover all form fields
2. For each text input: `vscreen_fill(selector="input[name='fieldname']", value="value")`
3. For dropdowns: `vscreen_select_option(selector="select[name='country']", value="US")`
4. For checkboxes/radios: `vscreen_click` at the element coordinates
5. Submit: `vscreen_find_elements("button[type='submit'], input[type='submit']")` then `vscreen_click`

## Multi-Page Navigation

1. `vscreen_navigate(url)` — Start at the first page
2. Process the page content
3. `vscreen_find_by_text("Next")` or `vscreen_find_elements("a.next, [rel='next']")` — Find pagination
4. `vscreen_click` the next button
5. `vscreen_wait_for_url` or `vscreen_wait_for_idle` — Wait for navigation to complete
6. Repeat from step 2

## Taking Sequential Screenshots

```json
{"instance_id": "dev", "count": 3, "interval_ms": 1000}
```
Returns 3 screenshots taken 1 second apart — useful for observing animations, loading sequences, or dynamic content.
"#;

const DOC_COORDINATES: &str = r#"# Coordinate System

## Page-Space Coordinates

All vscreen coordinates use **page-space** — the absolute position on the full document, measured from the top-left corner (0, 0) of the page.

- `vscreen_click(x, y)` — x and y are page-space. The system automatically scrolls to make the point visible before dispatching the click.
- `vscreen_find_elements` — Returns `x`, `y`, `width`, `height` in page-space.
- `vscreen_find_by_text` — Returns `x`, `y`, `width`, `height` in page-space.
- `vscreen_screenshot(clip)` — The clip rectangle uses page-space coordinates.

## Iframe Coordinate Translation

When `include_iframes: true` is used with `vscreen_find_elements` or `vscreen_find_by_text`, elements inside iframes have their coordinates automatically translated to page-space. The returned `x` and `y` values account for the iframe's position on the page, so you can pass them directly to `vscreen_click`.

## Click Target Calculation

To click the center of a found element:
```
click_x = element.x + element.width / 2
click_y = element.y + element.height / 2
```

## Full-Page Screenshots

When `full_page: true` is used, the screenshot captures the entire scrollable page. The resulting image may be much larger than the viewport (1920x1080). Coordinates from a full-page screenshot are still in page-space and can be used directly with `vscreen_click`.

## Viewport Size

The default viewport is **1920x1080**. Elements visible without scrolling have y-coordinates between 0 and 1080.
"#;

const DOC_IFRAMES: &str = r#"# Working with Iframes

## Overview

Many web pages embed cross-origin content in iframes (reCAPTCHA, ads, embedded widgets, payment forms). These iframes have separate DOM trees that are not accessible via normal `vscreen_execute_js` calls.

## Discovering Iframes

Call `vscreen_list_frames` to get:
1. The complete frame tree (parent-child relationships)
2. Bounding rectangles for each iframe on the page
3. Frame IDs, names, URLs, and visibility status

Example response includes:
```json
{"src": "https://www.google.com/recaptcha/...", "name": "a-xyz", "title": "reCAPTCHA",
 "x": 33, "y": 336, "width": 304, "height": 78, "visible": true}
```

## Finding Elements in Iframes

Use `include_iframes: true` on search tools:

```json
// vscreen_find_elements
{"instance_id": "dev", "selector": "button", "include_iframes": true}

// vscreen_find_by_text
{"instance_id": "dev", "text": "I'm not a robot", "include_iframes": true}
```

Results include a `frame_id` field identifying which frame contains each element, and coordinates are already translated to page-space.

## Clicking Iframe Elements

Simply use the page-space coordinates from search results:
```json
// Element found at {"x": 86, "y": 337, "frame_id": "ABC123"}
// Click directly using those coordinates:
{"instance_id": "dev", "x": 120, "y": 370}
```

## Limitations

- `vscreen_execute_js` runs only in the main frame
- `vscreen_click_element` searches only the main frame (use `vscreen_find_by_text` + `vscreen_click` for iframe elements)
- `vscreen_screenshot_annotated` only annotates main-frame elements
"#;

const DOC_LOCKING: &str = r#"# Instance Locking

## When Do You Need Locking?

- **Single-agent mode** (e.g., `--mcp-stdio` spawned by one client): Locks are auto-acquired transparently. You don't need to call any lock tools.
- **Multi-agent mode** (e.g., `--mcp-sse` with multiple clients): Explicit locking prevents agents from interfering with each other.

## Lock Types

- **`exclusive`** — Full control. Only one session can hold an exclusive lock. Required for any input action (click, type, navigate, etc.).
- **`observer`** — Read-only access. Multiple observers can coexist with each other, but not with an exclusive lock. Allows screenshots, find_elements, get_page_info, etc.

## Workflow

```
1. vscreen_instance_lock(instance_id="dev", lock_type="exclusive")
   → Returns: lock_token (save this for unlock/renew)

2. ... perform your actions ...

3. vscreen_instance_lock_renew(instance_id="dev", lock_token="...", ttl_seconds=300)
   → Extends the lock if your task takes longer than expected

4. vscreen_instance_unlock(instance_id="dev", lock_token="...")
   → Release the lock when done
```

## Lock Parameters

- `ttl_seconds` (default: 300) — Lock automatically expires after this many seconds
- `wait_timeout_seconds` (default: 0) — If the instance is already locked, wait this many seconds for it to become available (0 = fail immediately)
- `lock_token` — Returned on lock acquisition; required for unlock and renew

## Checking Lock Status

```json
// All instances:
vscreen_instance_lock_status()

// Specific instance:
vscreen_instance_lock_status(instance_id="dev")
```
"#;

const DOC_TROUBLESHOOTING: &str = r#"# Troubleshooting

## "Element not found"

- Try `include_iframes: true` — the element may be inside an iframe
- Use a broader CSS selector (e.g., `button` instead of `button.specific-class`)
- The element may not be loaded yet — use `vscreen_wait_for_selector(selector, timeout_ms=10000)` first
- The element may be below the fold — it still has page-space coordinates, so `vscreen_find_elements` will find it

## "Click doesn't seem to work"

- Take a `vscreen_screenshot` to verify the coordinates visually
- The element may be behind an overlay (cookie consent, modal). Use `vscreen_dismiss_dialogs` first
- The element may be inside an iframe — use `vscreen_find_by_text(include_iframes=true)` to get correct page-space coordinates
- Try `vscreen_click_element(selector=".button")` for main-frame elements

## "InstanceLocked" error

- Another MCP session holds the lock
- Check with `vscreen_instance_lock_status(instance_id)`
- In single-agent mode, this shouldn't happen — it may indicate a stale session
- Wait for the lock: `vscreen_instance_lock(wait_timeout_seconds=30)`

## "Timeout waiting for text/selector"

- The content may be dynamically loaded after the page "load" event
- Increase `timeout_ms` (default is 10000ms / 10 seconds)
- Use `vscreen_wait_for_network_idle` to wait for all AJAX requests to complete first
- Check the page with `vscreen_screenshot` to see current state

## "Cookie consent blocking content"

- Call `vscreen_dismiss_dialogs(instance_id)` — handles OneTrust, CookieBot, and common patterns
- If that doesn't work: `vscreen_find_by_text("Accept all")` then `vscreen_click`
- Cookies persist across navigations within the same instance

## "JavaScript returns null/undefined"

- The DOM element may not exist yet — use `vscreen_wait_for_selector` first
- `vscreen_execute_js` runs in the **main frame only** — it cannot access iframe content
- Wrap your expression in a try-catch: `"try { ... } catch(e) { e.message }"`

## "Page looks different than expected"

- Take a `vscreen_screenshot(full_page=true)` to see the entire page
- The page may have responsive breakpoints — viewport is 1920x1080
- Dynamic content may have changed — use `vscreen_screenshot_sequence` to watch for changes

## "Connection closed" or MCP disconnects

- **If using `--mcp-stdio --dev`**: The stdio subprocess starts a full dev environment. If another vscreen instance is already running, resources conflict (Xvfb display, CDP port, HTTP port), causing crashes.
- **Fix**: Switch to SSE mode. Start the server once: `vscreen --dev --mcp-sse 0.0.0.0:8451`. Then configure your MCP client with `{"url": "http://localhost:8451/mcp"}`.
- **Alternative**: Use the stdio proxy: `vscreen --mcp-stdio-proxy http://localhost:8451/mcp`. This is lightweight and can be freely restarted without affecting the server.

## "Display :99 already in use" or "CDP port in use"

- Another vscreen instance is running on the same display or CDP port.
- Use `--dev-display N` to choose a different X11 display number.
- Use `--dev-cdp-port N` to choose a different Chrome DevTools Protocol port.
- Or stop the existing instance first.

## "vscreen dev already running (PID ...)"

- vscreen writes a PID file to prevent duplicate instances.
- Stop the existing instance or connect to it via SSE/proxy.
- If the PID file is stale (process crashed), vscreen will clean it up automatically on next start.
"#;

const DOC_TOOL_SELECTION: &str = r##"# Tool Selection Guide

Choosing the right tool avoids unnecessary round-trips and gives better results.

## Observing the Page

| Goal | Best Tool | NOT This |
|------|-----------|----------|
| See the full page | `vscreen_screenshot(full_page=true)` | scroll + screenshot repeatedly |
| Read page text | `vscreen_extract_text` | screenshot + OCR |
| Zoom into a region | `vscreen_screenshot(clip={x,y,w,h})` | full screenshot + cropping |
| Page title/URL/viewport | `vscreen_get_page_info` | `vscreen_execute_js("document.title")` |
| Identify interactive elements | `vscreen_screenshot_annotated` | manual screenshot + guessing |

## Finding Elements

| Goal | Best Tool |
|------|-----------|
| Know the visible text | `vscreen_find_by_text(text="Sign In")` |
| Know the CSS selector | `vscreen_find_elements(selector="button.submit")` |
| Element is inside an iframe | add `include_iframes=true` to find tools |
| Need semantic structure | `vscreen_accessibility_tree` |
| Icon-only/unlabeled buttons | `vscreen_describe_elements` (requires vision LLM) |

## Clicking Elements

| Goal | Best Tool |
|------|-----------|
| Click by text (main frame) | `vscreen_click_element(text="Submit")` |
| Click by selector (main frame) | `vscreen_click_element(selector="#btn")` |
| Click in iframe or with coordinates | `vscreen_click(x, y)` |
| Click + wait for navigation | `vscreen_click_and_navigate(text="Next")` |
| Click many tiles rapidly | `vscreen_batch_click(points=[[x1,y1],[x2,y2]])` |

## Waiting for Changes

| Goal | Best Tool | NOT This |
|------|-----------|----------|
| Text appears | `vscreen_wait_for_text(text="Success")` | `vscreen_wait(5000)` loop |
| Element appears | `vscreen_wait_for_selector(selector=".result")` | `vscreen_wait(5000)` loop |
| Page navigates | `vscreen_wait_for_url(url_contains="/dashboard")` | `vscreen_wait(5000)` loop |
| Data loads (AJAX) | `vscreen_wait_for_network_idle` | `vscreen_wait(5000)` loop |
| Brief pause after action | `vscreen_wait(500)` | — this is fine for short pauses |

## Scrolling

| Goal | Best Tool | NOT This |
|------|-----------|----------|
| See below-fold content | `vscreen_screenshot(full_page=true)` | scroll + screenshot loop |
| Bring element into view | `vscreen_scroll_to_element(selector)` | `vscreen_scroll(x, y, delta_y)` |
| Precise pixel scroll needed | `vscreen_scroll(x, y, delta_y=120)` | — this is the right tool |

## Planning

Call `vscreen_plan(task="describe your goal")` before multi-step workflows to get
the optimal tool sequence and avoid anti-patterns.
"##;

const DOC_TOOLS_HEADER: &str = r#"# Tool Reference

All tools accept `instance_id` as their first parameter (the browser instance to operate on).
Use `vscreen_list_instances` to discover available instances.

## Observation Tools (Read-Only)

These tools do not modify page state and can be called freely.

| Tool | Description |
|------|-------------|
| `vscreen_list_instances` | List all browser instances and their states |
| `vscreen_screenshot` | Capture page screenshot (png/jpeg/webp, optional clip region) |
| `vscreen_screenshot_sequence` | Capture multiple screenshots at intervals |
| `vscreen_screenshot_annotated` | Screenshot with numbered bounding boxes on elements |
| `vscreen_get_page_info` | Get page title, URL, and viewport dimensions |
| `vscreen_find_elements` | Find elements by CSS selector (supports iframe search) |
| `vscreen_find_by_text` | Find elements by visible text (supports iframe search) |
| `vscreen_list_frames` | List all frames/iframes with bounding rectangles |
| `vscreen_extract_text` | Extract all visible text from the page |
| `vscreen_accessibility_tree` | Get the accessibility tree (structured page representation) |
| `vscreen_describe_elements` | Identify unlabeled icon-only elements using vision LLM |
| `vscreen_get_cookies` | Get cookies for the current page |
| `vscreen_get_storage` | Get localStorage or sessionStorage values |
| `vscreen_history_list` | List screenshot history entries |
| `vscreen_history_get` | Get a specific historical screenshot |
| `vscreen_session_log` | Get recent action log |
| `vscreen_session_summary` | Get session summary (action count, duration) |
| `vscreen_console_log` | Get captured browser console messages |
| `vscreen_instance_lock_status` | Check lock status |

## Input Action Tools

These tools modify page state (click, type, scroll, etc.).

| Tool | Description |
|------|-------------|
| `vscreen_click` | Click at page-space coordinates (auto-scrolls) |
| `vscreen_double_click` | Double-click at coordinates |
| `vscreen_type` | Type text into the focused element |
| `vscreen_fill` | Clear and fill an input field by CSS selector |
| `vscreen_key_press` | Press a single key (e.g., "Enter", "Escape", "Tab") |
| `vscreen_key_combo` | Press a key combination (e.g., ["Control", "a"]) |
| `vscreen_scroll` | Scroll by pixel delta |
| `vscreen_drag` | Drag from one point to another |
| `vscreen_hover` | Hover at coordinates |
| `vscreen_click_element` | Click element by CSS selector or visible text (with retries + wait_after_ms) |
| `vscreen_click_and_navigate` | Click element and wait for URL change (with <a> fallback) |
| `vscreen_batch_click` | Click multiple points rapidly in one call (for timed challenges) |
| `vscreen_select_option` | Select a dropdown option by value or label |
| `vscreen_scroll_to_element` | Scroll an element into view by CSS selector |

## Navigation Tools

| Tool | Description |
|------|-------------|
| `vscreen_navigate` | Navigate to a URL with optional wait condition |
| `vscreen_go_back` | Navigate back in browser history |
| `vscreen_go_forward` | Navigate forward in browser history |
| `vscreen_reload` | Reload the current page |

## Synchronization Tools

| Tool | Description |
|------|-------------|
| `vscreen_wait` | Wait a fixed number of milliseconds |
| `vscreen_wait_for_idle` | Wait for no screencast frames for a duration |
| `vscreen_wait_for_text` | Wait for specific text to appear on the page |
| `vscreen_wait_for_selector` | Wait for a CSS selector to match an element |
| `vscreen_wait_for_url` | Wait for the URL to match a pattern |
| `vscreen_wait_for_network_idle` | Wait for network activity to stop |

## Utility Tools

| Tool | Description |
|------|-------------|
| `vscreen_execute_js` | Execute JavaScript in the main frame |
| `vscreen_find_input` | Find text inputs by placeholder, aria-label, label, role, or name |
| `vscreen_dismiss_dialogs` | Auto-dismiss cookie consent, GDPR, and privacy dialogs |
| `vscreen_dismiss_ads` | Dismiss video platform ad overlays (YouTube skip, etc.) |
| `vscreen_set_cookie` | Set a browser cookie |
| `vscreen_set_storage` | Set a localStorage or sessionStorage value |
| `vscreen_plan` | Get recommended tool sequence for a task (call before multi-step workflows) |
| `vscreen_help` | Get contextual documentation about any tool or topic |
"#;

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

        let hint = {
            let mut advisor = self.advisor.lock().await;
            let hint = advisor.check_anti_patterns(&tool_name, &args_value);
            advisor.record(ToolCallRecord {
                tool_name: tool_name.clone(),
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
mod tests {
    use super::*;

    fn make_state() -> AppState {
        AppState::new(
            vscreen_core::config::AppConfig::default(),
            tokio_util::sync::CancellationToken::new(),
        )
    }

    // -----------------------------------------------------------------------
    // Server info
    // -----------------------------------------------------------------------

    #[test]
    fn server_info_has_instructions() {
        let state = make_state();
        let server = VScreenMcpServer::new(state);
        let info = server.get_info();
        assert!(info.instructions.is_some());
        assert!(info
            .instructions
            .as_deref()
            .unwrap_or("")
            .contains("vscreen"));
    }

    #[test]
    fn server_info_enables_tools() {
        let state = make_state();
        let server = VScreenMcpServer::new(state);
        let info = server.get_info();
        assert!(info.capabilities.tools.is_some());
    }

    // -----------------------------------------------------------------------
    // Parameter type deserialization
    // -----------------------------------------------------------------------

    #[test]
    fn instance_id_param() {
        let json = r#"{"instance_id":"dev"}"#;
        let p: InstanceIdParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.instance_id, "dev");
    }

    #[test]
    fn screenshot_param_defaults() {
        let json = r#"{"instance_id":"dev"}"#;
        let p: ScreenshotParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.format, "png");
        assert!(p.quality.is_none());
    }

    #[test]
    fn screenshot_param_full() {
        let json = r#"{"instance_id":"test","format":"jpeg","quality":85}"#;
        let p: ScreenshotParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.format, "jpeg");
        assert_eq!(p.quality, Some(85));
    }

    #[test]
    fn screenshot_sequence_param() {
        let json = r#"{"instance_id":"dev","count":5,"interval_ms":500}"#;
        let p: ScreenshotSequenceParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.count, 5);
        assert_eq!(p.interval_ms, 500);
        assert_eq!(p.format, "png");
    }

    #[test]
    fn navigate_param() {
        let json = r#"{"instance_id":"dev","url":"https://google.com"}"#;
        let p: NavigateParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.url, "https://google.com");
    }

    #[test]
    fn click_param_minimal() {
        let json = r#"{"instance_id":"dev","x":100,"y":200}"#;
        let p: ClickParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.button, None);
    }

    #[test]
    fn click_param_with_button() {
        let json = r#"{"instance_id":"dev","x":100,"y":200,"button":2}"#;
        let p: ClickParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.button, Some(2));
    }

    #[test]
    fn double_click_param() {
        let json = r#"{"instance_id":"dev","x":50,"y":60}"#;
        let p: DoubleClickParam = serde_json::from_str(json).expect("parse");
        assert!((p.x - 50.0).abs() < f64::EPSILON);
        assert!((p.y - 60.0).abs() < f64::EPSILON);
    }

    #[test]
    fn type_param() {
        let json = r#"{"instance_id":"dev","text":"hello"}"#;
        let p: TypeParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.text, "hello");
    }

    #[test]
    fn key_press_param_simple() {
        let json = r#"{"instance_id":"dev","key":"Enter"}"#;
        let p: KeyPressParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.key, "Enter");
        assert!(!p.ctrl);
        assert!(!p.shift);
    }

    #[test]
    fn key_press_param_with_modifiers() {
        let json = r#"{"instance_id":"dev","key":"c","ctrl":true}"#;
        let p: KeyPressParam = serde_json::from_str(json).expect("parse");
        assert!(p.ctrl);
        assert!(!p.shift);
    }

    #[test]
    fn key_combo_param() {
        let json = r#"{"instance_id":"dev","keys":["Control","Shift","i"]}"#;
        let p: KeyComboParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.keys.len(), 3);
        assert_eq!(p.keys[0], "Control");
        assert_eq!(p.keys[2], "i");
    }

    #[test]
    fn scroll_param() {
        let json = r#"{"instance_id":"dev","x":100,"y":200,"delta_y":-120}"#;
        let p: ScrollParam = serde_json::from_str(json).expect("parse");
        assert!((p.delta_x).abs() < f64::EPSILON);
        assert!((p.delta_y - (-120.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn drag_param_defaults() {
        let json = r#"{"instance_id":"dev","from_x":0,"from_y":0,"to_x":100,"to_y":100}"#;
        let p: DragParam = serde_json::from_str(json).expect("parse");
        assert!(p.steps.is_none());
        assert!(p.duration_ms.is_none());
    }

    #[test]
    fn drag_param_full() {
        let json = r#"{"instance_id":"dev","from_x":10,"from_y":20,"to_x":30,"to_y":40,"steps":5,"duration_ms":1000}"#;
        let p: DragParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.steps, Some(5));
        assert_eq!(p.duration_ms, Some(1000));
    }

    #[test]
    fn hover_param() {
        let json = r#"{"instance_id":"dev","x":42,"y":84}"#;
        let p: HoverParam = serde_json::from_str(json).expect("parse");
        assert!((p.x - 42.0).abs() < f64::EPSILON);
    }

    #[test]
    fn wait_param() {
        let json = r#"{"duration_ms":500}"#;
        let p: WaitParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.duration_ms, 500);
    }

    #[test]
    fn wait_for_idle_param_defaults() {
        let json = r#"{"instance_id":"dev"}"#;
        let p: WaitForIdleParam = serde_json::from_str(json).expect("parse");
        assert!(p.timeout_ms.is_none());
    }

    #[test]
    fn wait_for_idle_param_full() {
        let json = r#"{"instance_id":"dev","timeout_ms":10000}"#;
        let p: WaitForIdleParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.timeout_ms, Some(10000));
    }

    #[test]
    fn execute_js_param() {
        let json = r#"{"instance_id":"dev","expression":"1+1"}"#;
        let p: ExecuteJsParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.expression, "1+1");
    }

    // -----------------------------------------------------------------------
    // New parameter type tests
    // -----------------------------------------------------------------------

    #[test]
    fn history_list_param() {
        let json = r#"{"instance_id":"dev"}"#;
        let p: HistoryListParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.instance_id, "dev");
    }

    #[test]
    fn history_get_param() {
        let json = r#"{"instance_id":"dev","index":3}"#;
        let p: HistoryGetParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.index, 3);
    }

    #[test]
    fn history_get_range_param() {
        let json = r#"{"instance_id":"dev","from":2,"count":5}"#;
        let p: HistoryGetRangeParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.from, 2);
        assert_eq!(p.count, 5);
    }

    #[test]
    fn session_log_param_defaults() {
        let json = r#"{"instance_id":"dev"}"#;
        let p: SessionLogParam = serde_json::from_str(json).expect("parse");
        assert!(p.last_n.is_none());
    }

    #[test]
    fn session_log_param_with_n() {
        let json = r#"{"instance_id":"dev","last_n":10}"#;
        let p: SessionLogParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.last_n, Some(10));
    }

    #[test]
    fn find_elements_param() {
        let json = r#"{"instance_id":"dev","selector":"button.primary"}"#;
        let p: FindElementsParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.selector, "button.primary");
    }

    #[test]
    fn find_by_text_param_defaults() {
        let json = r#"{"instance_id":"dev","text":"Submit"}"#;
        let p: FindByTextParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.text, "Submit");
        assert!(!p.exact);
    }

    #[test]
    fn find_by_text_param_exact() {
        let json = r#"{"instance_id":"dev","text":"Submit","exact":true}"#;
        let p: FindByTextParam = serde_json::from_str(json).expect("parse");
        assert!(p.exact);
    }

    #[test]
    fn wait_for_text_param_defaults() {
        let json = r#"{"instance_id":"dev","text":"Loading complete"}"#;
        let p: WaitForTextParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.text, "Loading complete");
        assert!(p.timeout_ms.is_none());
        assert!(p.interval_ms.is_none());
    }

    #[test]
    fn wait_for_text_param_full() {
        let json = r#"{"instance_id":"dev","text":"OK","timeout_ms":5000,"interval_ms":100}"#;
        let p: WaitForTextParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.timeout_ms, Some(5000));
        assert_eq!(p.interval_ms, Some(100));
    }

    #[test]
    fn wait_for_selector_param_defaults() {
        let json = r##"{"instance_id":"dev","selector":"#result"}"##;
        let p: WaitForSelectorParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.selector, "#result");
        assert!(!p.visible);
    }

    #[test]
    fn wait_for_selector_param_visible() {
        let json = r#"{"instance_id":"dev","selector":".modal","visible":true,"timeout_ms":3000}"#;
        let p: WaitForSelectorParam = serde_json::from_str(json).expect("parse");
        assert!(p.visible);
        assert_eq!(p.timeout_ms, Some(3000));
    }

    #[test]
    fn extract_text_param_defaults() {
        let json = r#"{"instance_id":"dev"}"#;
        let p: ExtractTextParam = serde_json::from_str(json).expect("parse");
        assert!(p.selector.is_none());
    }

    #[test]
    fn extract_text_param_with_selector() {
        let json = r#"{"instance_id":"dev","selector":"main article"}"#;
        let p: ExtractTextParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.selector.as_deref(), Some("main article"));
    }

    #[test]
    fn console_log_param_defaults() {
        let json = r#"{"instance_id":"dev"}"#;
        let p: ConsoleLogParam = serde_json::from_str(json).expect("parse");
        assert!(p.level.is_none());
    }

    #[test]
    fn console_log_param_with_level() {
        let json = r#"{"instance_id":"dev","level":"error"}"#;
        let p: ConsoleLogParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.level.as_deref(), Some("error"));
    }

    #[test]
    fn accessibility_tree_param_defaults() {
        let json = r#"{"instance_id":"dev"}"#;
        let p: AccessibilityTreeParam = serde_json::from_str(json).expect("parse");
        assert!(p.max_depth.is_none());
    }

    #[test]
    fn accessibility_tree_param_with_depth() {
        let json = r#"{"instance_id":"dev","max_depth":3}"#;
        let p: AccessibilityTreeParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.max_depth, Some(3));
    }

    #[test]
    fn set_cookie_param() {
        let json = r#"{"instance_id":"dev","name":"session","value":"abc123"}"#;
        let p: SetCookieParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.name, "session");
        assert_eq!(p.value, "abc123");
        assert!(p.domain.is_none());
        assert!(p.path.is_none());
    }

    #[test]
    fn set_cookie_param_full() {
        let json = r#"{"instance_id":"dev","name":"tok","value":"xyz","domain":".example.com","path":"/api"}"#;
        let p: SetCookieParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.domain.as_deref(), Some(".example.com"));
        assert_eq!(p.path.as_deref(), Some("/api"));
    }

    #[test]
    fn storage_get_param_defaults() {
        let json = r#"{"instance_id":"dev","key":"theme"}"#;
        let p: StorageGetParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.key, "theme");
        assert_eq!(p.storage_type, "local");
    }

    #[test]
    fn storage_get_param_session() {
        let json = r#"{"instance_id":"dev","key":"token","storage_type":"session"}"#;
        let p: StorageGetParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.storage_type, "session");
    }

    #[test]
    fn storage_set_param() {
        let json = r#"{"instance_id":"dev","key":"lang","value":"en"}"#;
        let p: StorageSetParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.key, "lang");
        assert_eq!(p.value, "en");
        assert_eq!(p.storage_type, "local");
    }

    #[test]
    fn wait_for_url_param() {
        let json = r#"{"instance_id":"dev","url_contains":"dashboard"}"#;
        let p: WaitForUrlParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.url_contains, "dashboard");
        assert!(p.timeout_ms.is_none());
    }

    #[test]
    fn wait_for_network_idle_param_defaults() {
        let json = r#"{"instance_id":"dev"}"#;
        let p: WaitForNetworkIdleParam = serde_json::from_str(json).expect("parse");
        assert!(p.idle_ms.is_none());
        assert!(p.timeout_ms.is_none());
    }

    #[test]
    fn wait_for_network_idle_param_full() {
        let json = r#"{"instance_id":"dev","idle_ms":1000,"timeout_ms":15000}"#;
        let p: WaitForNetworkIdleParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.idle_ms, Some(1000));
        assert_eq!(p.timeout_ms, Some(15000));
    }

    #[test]
    fn annotated_screenshot_param_defaults() {
        let json = r#"{"instance_id":"dev"}"#;
        let p: AnnotatedScreenshotParam = serde_json::from_str(json).expect("parse");
        assert!(p.selector.is_none());
    }

    #[test]
    fn annotated_screenshot_param_custom_selector() {
        let json = r#"{"instance_id":"dev","selector":"button, a"}"#;
        let p: AnnotatedScreenshotParam = serde_json::from_str(json).expect("parse");
        assert_eq!(p.selector.as_deref(), Some("button, a"));
    }

    #[test]
    fn new_param_types_have_json_schema() {
        use schemars::schema_for;

        let schema = schema_for!(FindElementsParam);
        let json = serde_json::to_string(&schema).expect("serialize");
        assert!(json.contains("selector"));

        let schema = schema_for!(WaitForTextParam);
        let json = serde_json::to_string(&schema).expect("serialize");
        assert!(json.contains("text"));
        assert!(json.contains("timeout_ms"));

        let schema = schema_for!(SetCookieParam);
        let json = serde_json::to_string(&schema).expect("serialize");
        assert!(json.contains("name"));
        assert!(json.contains("value"));

        let schema = schema_for!(AnnotatedScreenshotParam);
        let json = serde_json::to_string(&schema).expect("serialize");
        assert!(json.contains("selector"));
    }

    // -----------------------------------------------------------------------
    // Helper function tests
    // -----------------------------------------------------------------------

    #[test]
    fn get_supervisor_missing_instance() {
        let state = make_state();
        let server = VScreenMcpServer::new(state);
        let result = server.get_supervisor("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn internal_error_message() {
        let err = internal_error("something broke");
        assert_eq!(err.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
        assert_eq!(err.message.as_ref(), "something broke");
    }

    #[test]
    fn invalid_params_message() {
        let err = invalid_params("bad arg");
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert_eq!(err.message.as_ref(), "bad arg");
    }

    // -----------------------------------------------------------------------
    // Serialization roundtrip for parameter types
    // -----------------------------------------------------------------------

    #[test]
    fn param_types_serialize_roundtrip() {
        let original = ScreenshotParam {
            instance_id: "dev".into(),
            format: "png".into(),
            quality: Some(80),
            full_page: true,
            clip: None,
        };
        let json = serde_json::to_string(&original).expect("serialize");
        let parsed: ScreenshotParam = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.instance_id, "dev");
        assert_eq!(parsed.format, "png");
        assert_eq!(parsed.quality, Some(80));
        assert!(parsed.full_page);
    }

    #[test]
    fn param_types_have_json_schema() {
        use schemars::schema_for;

        let schema = schema_for!(ScreenshotParam);
        let json = serde_json::to_string(&schema).expect("serialize schema");
        assert!(json.contains("instance_id"));
        assert!(json.contains("format"));

        let schema = schema_for!(ClickParam);
        let json = serde_json::to_string(&schema).expect("serialize schema");
        assert!(json.contains("x"));
        assert!(json.contains("y"));

        let schema = schema_for!(KeyComboParam);
        let json = serde_json::to_string(&schema).expect("serialize schema");
        assert!(json.contains("keys"));
    }

    // -----------------------------------------------------------------------
    // Tool availability
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn list_instances_tool_works_empty() {
        let state = make_state();
        let server = VScreenMcpServer::new(state);
        let result = server.vscreen_list_instances().await;
        assert!(result.is_ok());
        let call_result = result.unwrap();
        assert!(!call_result.content.is_empty());
    }

    fn extract_text(content: &Content) -> &str {
        match &content.raw {
            RawContent::Text(t) => &t.text,
            _ => panic!("expected text content"),
        }
    }

    #[tokio::test]
    async fn list_instances_tool_with_registry() {
        let state = make_state();
        let config = vscreen_core::instance::InstanceConfig {
            instance_id: InstanceId::from("test-mcp"),
            cdp_endpoint: "ws://localhost:9222".into(),
            pulse_source: "test.monitor".into(),
            display: None,
            video: vscreen_core::config::VideoConfig::default(),
            audio: vscreen_core::config::AudioConfig::default(),
            rtp_output: None,
        };
        state.registry.create(config, 16).expect("create");

        let server = VScreenMcpServer::new(state);
        let result = server.vscreen_list_instances().await;
        assert!(result.is_ok());
        let call_result = result.unwrap();
        let text = extract_text(&call_result.content[0]);
        assert!(text.contains("test-mcp"));
    }

    #[tokio::test]
    async fn wait_tool_completes() {
        let state = make_state();
        let server = VScreenMcpServer::new(state);
        let params = WaitParam { duration_ms: 10 };
        let call_result = server
            .vscreen_wait(Parameters(params))
            .await
            .expect("wait should succeed");
        let text = extract_text(&call_result.content[0]);
        assert!(text.contains("10ms"));
    }

    // -----------------------------------------------------------------------
    // Tool Advisor anti-pattern detection
    // -----------------------------------------------------------------------

    #[test]
    fn advisor_detects_scroll_screenshot_loop() {
        let mut advisor = ToolAdvisor::new();
        advisor.record(ToolCallRecord { tool_name: "vscreen_scroll".into() });
        advisor.record(ToolCallRecord { tool_name: "vscreen_screenshot".into() });
        advisor.record(ToolCallRecord { tool_name: "vscreen_scroll".into() });
        advisor.record(ToolCallRecord { tool_name: "vscreen_screenshot".into() });
        advisor.record(ToolCallRecord { tool_name: "vscreen_scroll".into() });

        let hint = advisor.check_anti_patterns(
            "vscreen_screenshot",
            &serde_json::json!({"instance_id": "dev"}),
        );
        assert!(hint.is_some(), "should detect scroll-screenshot loop");
        assert!(
            hint.unwrap().contains("full_page=true"),
            "should recommend full_page=true"
        );
    }

    #[test]
    fn advisor_no_hint_for_full_page_screenshot() {
        let mut advisor = ToolAdvisor::new();
        advisor.record(ToolCallRecord { tool_name: "vscreen_scroll".into() });
        advisor.record(ToolCallRecord { tool_name: "vscreen_screenshot".into() });
        advisor.record(ToolCallRecord { tool_name: "vscreen_scroll".into() });
        advisor.record(ToolCallRecord { tool_name: "vscreen_screenshot".into() });
        advisor.record(ToolCallRecord { tool_name: "vscreen_scroll".into() });

        let hint = advisor.check_anti_patterns(
            "vscreen_screenshot",
            &serde_json::json!({"instance_id": "dev", "full_page": true}),
        );
        assert!(hint.is_none(), "should NOT hint when full_page=true is used");
    }

    #[test]
    fn advisor_no_hint_for_clip_screenshot() {
        let mut advisor = ToolAdvisor::new();
        advisor.record(ToolCallRecord { tool_name: "vscreen_scroll".into() });
        advisor.record(ToolCallRecord { tool_name: "vscreen_screenshot".into() });
        advisor.record(ToolCallRecord { tool_name: "vscreen_scroll".into() });
        advisor.record(ToolCallRecord { tool_name: "vscreen_screenshot".into() });

        let hint = advisor.check_anti_patterns(
            "vscreen_screenshot",
            &serde_json::json!({"instance_id": "dev", "clip": {"x": 0, "y": 0, "width": 100, "height": 100}}),
        );
        assert!(hint.is_none(), "should NOT hint when clip is used");
    }

    #[test]
    fn advisor_detects_repeated_waits() {
        let mut advisor = ToolAdvisor::new();
        advisor.record(ToolCallRecord { tool_name: "vscreen_wait".into() });
        advisor.record(ToolCallRecord { tool_name: "vscreen_screenshot".into() });
        advisor.record(ToolCallRecord { tool_name: "vscreen_wait".into() });

        let hint = advisor.check_anti_patterns(
            "vscreen_wait",
            &serde_json::json!({"duration_ms": 5000}),
        );
        assert!(hint.is_some(), "should detect repeated fixed waits");
        assert!(
            hint.unwrap().contains("wait_for_text"),
            "should recommend wait_for_text"
        );
    }

    #[test]
    fn advisor_detects_js_for_metadata() {
        let advisor = ToolAdvisor::new();

        let hint = advisor.check_anti_patterns(
            "vscreen_execute_js",
            &serde_json::json!({"instance_id": "dev", "expression": "document.title"}),
        );
        assert!(hint.is_some(), "should detect JS metadata anti-pattern");
        assert!(
            hint.unwrap().contains("get_page_info"),
            "should recommend get_page_info"
        );
    }

    #[test]
    fn advisor_detects_js_for_text_content() {
        let advisor = ToolAdvisor::new();

        let hint = advisor.check_anti_patterns(
            "vscreen_execute_js",
            &serde_json::json!({"instance_id": "dev", "expression": "document.body.innerText"}),
        );
        assert!(hint.is_some(), "should detect JS text extraction anti-pattern");
        assert!(
            hint.unwrap().contains("extract_text"),
            "should recommend extract_text"
        );
    }

    #[test]
    fn advisor_no_hint_for_custom_js() {
        let advisor = ToolAdvisor::new();

        let hint = advisor.check_anti_patterns(
            "vscreen_execute_js",
            &serde_json::json!({"instance_id": "dev", "expression": "document.querySelectorAll('.item').length"}),
        );
        assert!(hint.is_none(), "should NOT hint for custom JS expressions");
    }

    #[test]
    fn advisor_detects_multiple_scrolls() {
        let mut advisor = ToolAdvisor::new();
        advisor.record(ToolCallRecord { tool_name: "vscreen_scroll".into() });
        advisor.record(ToolCallRecord { tool_name: "vscreen_screenshot".into() });
        advisor.record(ToolCallRecord { tool_name: "vscreen_scroll".into() });

        let hint = advisor.check_anti_patterns(
            "vscreen_scroll",
            &serde_json::json!({"instance_id": "dev", "x": 500, "y": 500, "delta_y": 300}),
        );
        assert!(hint.is_some(), "should detect multiple scroll calls");
        let hint_text = hint.unwrap();
        assert!(
            hint_text.contains("scroll_to_element") || hint_text.contains("full_page"),
            "should recommend alternatives: {hint_text}"
        );
    }

    #[test]
    fn advisor_ring_buffer_limit() {
        let mut advisor = ToolAdvisor::new();
        for i in 0..25 {
            advisor.record(ToolCallRecord {
                tool_name: format!("tool_{i}"),
            });
        }
        assert_eq!(advisor.recent_calls.len(), 20, "should cap at 20 entries");
    }

    // -----------------------------------------------------------------------
    // vscreen_plan task routing
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn plan_recommends_extract_text() {
        let state = make_state();
        let server = VScreenMcpServer::new(state);
        let result = server
            .vscreen_plan(Parameters(PlanTaskParam {
                task: "read all the text on the page".into(),
            }))
            .await
            .expect("plan should succeed");
        let text = extract_text(&result.content[0]);
        assert!(
            text.contains("vscreen_extract_text"),
            "should recommend extract_text: {text}"
        );
    }

    #[tokio::test]
    async fn plan_recommends_full_page_screenshot() {
        let state = make_state();
        let server = VScreenMcpServer::new(state);
        let result = server
            .vscreen_plan(Parameters(PlanTaskParam {
                task: "see the whole page".into(),
            }))
            .await
            .expect("plan should succeed");
        let text = extract_text(&result.content[0]);
        assert!(
            text.contains("full_page=true"),
            "should recommend full_page: {text}"
        );
    }

    #[tokio::test]
    async fn plan_recommends_click_element() {
        let state = make_state();
        let server = VScreenMcpServer::new(state);
        let result = server
            .vscreen_plan(Parameters(PlanTaskParam {
                task: "click the sign in button".into(),
            }))
            .await
            .expect("plan should succeed");
        let text = extract_text(&result.content[0]);
        assert!(
            text.contains("vscreen_click_element"),
            "should recommend click_element: {text}"
        );
    }

    #[tokio::test]
    async fn plan_recommends_form_filling() {
        let state = make_state();
        let server = VScreenMcpServer::new(state);
        let result = server
            .vscreen_plan(Parameters(PlanTaskParam {
                task: "fill out the login form with username and password".into(),
            }))
            .await
            .expect("plan should succeed");
        let text = extract_text(&result.content[0]);
        assert!(
            text.contains("vscreen_fill"),
            "should recommend fill: {text}"
        );
    }

    #[tokio::test]
    async fn plan_recommends_captcha_solver() {
        let state = make_state();
        let server = VScreenMcpServer::new(state);
        let result = server
            .vscreen_plan(Parameters(PlanTaskParam {
                task: "solve the captcha challenge".into(),
            }))
            .await
            .expect("plan should succeed");
        let text = extract_text(&result.content[0]);
        assert!(
            text.contains("vscreen_solve_captcha"),
            "should recommend solve_captcha: {text}"
        );
    }

    #[tokio::test]
    async fn plan_fallback_for_unknown_task() {
        let state = make_state();
        let server = VScreenMcpServer::new(state);
        let result = server
            .vscreen_plan(Parameters(PlanTaskParam {
                task: "do something completely unique and novel".into(),
            }))
            .await
            .expect("plan should succeed");
        let text = extract_text(&result.content[0]);
        assert!(
            text.contains("General workflow"),
            "should show general fallback: {text}"
        );
    }

    #[tokio::test]
    async fn plan_recommends_wait_tools() {
        let state = make_state();
        let server = VScreenMcpServer::new(state);
        let result = server
            .vscreen_plan(Parameters(PlanTaskParam {
                task: "wait for the page to finish loading dynamic content".into(),
            }))
            .await
            .expect("plan should succeed");
        let text = extract_text(&result.content[0]);
        assert!(
            text.contains("wait_for_") || text.contains("network_idle"),
            "should recommend targeted waits: {text}"
        );
    }

    // -----------------------------------------------------------------------
    // vscreen_help tool-selection topic
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn help_tool_selection_topic() {
        let state = make_state();
        let server = VScreenMcpServer::new(state);
        let result = server
            .vscreen_help(Parameters(HelpParam {
                topic: "tool-selection".into(),
            }))
            .await
            .expect("help should succeed");
        let text = extract_text(&result.content[0]);
        assert!(
            text.contains("Tool Selection Guide"),
            "should return tool selection guide: {text}"
        );
        assert!(
            text.contains("full_page=true"),
            "should mention full_page: {text}"
        );
    }
}
