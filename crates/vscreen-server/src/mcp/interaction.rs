use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::join_all;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::{tool, tool_router};
use tracing::{debug, info};
use vscreen_core::event::InputEvent;

use image::GenericImageView;

use crate::screenshot_watcher;
use crate::vision::prompts::CAPTCHA_TILE_SINGLE;
use super::captcha::{compute_grid_positions, compute_verify_button};
use super::params::*;
use super::{internal_error, invalid_params, McpError, VScreenMcpServer};

/// RAII guard that aborts a spawned task when dropped, preventing orphaned
/// vision requests from keeping the GPU/LLM busy after the parent tool call
/// is cancelled or times out.
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

#[tool_router(router = interaction_tools, vis = "pub(crate)")]
impl VScreenMcpServer {
    #[tool(description = "Unified click tool with modes: \"single\" (default) — click at x,y coordinates; \"double\" — double-click at x,y; \"element\" — click by CSS selector or visible text (with retries); \"navigate\" — click element and wait for URL change (with <a> fallback); \"batch\" — rapid multi-point clicks (e.g. [x,y] pairs). Coordinates auto-scroll into view. PREFER mode=\"element\" for main-frame elements with known selector/text. Use mode=\"single\" with coordinates from find tools or for iframe elements. Example: {\"instance_id\": \"dev\", \"mode\": \"single\", \"x\": 300, \"y\": 200}")]
    async fn vscreen_click(
        &self,
        Parameters(params): Parameters<ConsolidatedClickParam>,
    ) -> Result<CallToolResult, McpError> {
        let mode = params.mode.as_str();
        match mode {
            "single" => {
                let x = params.x.ok_or_else(|| invalid_params("mode=\"single\" requires x"))?;
                let y = params.y.ok_or_else(|| invalid_params("mode=\"single\" requires y"))?;
                self.click_helper_single(&params.instance_id, x, y, params.button, params.wait_after_ms).await
            }
            "double" => {
                let x = params.x.ok_or_else(|| invalid_params("mode=\"double\" requires x"))?;
                let y = params.y.ok_or_else(|| invalid_params("mode=\"double\" requires y"))?;
                self.click_helper_double(&params.instance_id, x, y).await
            }
            "element" => {
                if params.selector.is_none() && params.text.is_none() {
                    return Err(invalid_params("mode=\"element\" requires selector or text (or both)"));
                }
                self.click_helper_element(
                    &params.instance_id,
                    params.selector.as_deref(),
                    params.text.as_deref(),
                    params.text_exact,
                    params.index.unwrap_or(0),
                    params.button,
                    params.wait_after_ms,
                    params.retries.unwrap_or(0),
                    params.retry_delay_ms.unwrap_or(500),
                ).await
            }
            "navigate" => {
                if params.selector.is_none() && params.text.is_none() {
                    return Err(invalid_params("mode=\"navigate\" requires selector or text (or both)"));
                }
                self.click_helper_navigate(
                    &params.instance_id,
                    params.selector.as_deref(),
                    params.text.as_deref(),
                    params.timeout_ms.unwrap_or(5000),
                    params.fallback_to_link,
                ).await
            }
            "batch" => {
                let points = params.points.ok_or_else(|| invalid_params("mode=\"batch\" requires points"))?;
                if points.is_empty() {
                    return Err(invalid_params("mode=\"batch\" requires non-empty points array"));
                }
                self.click_helper_batch(&params.instance_id, &points, params.delay_between_ms.unwrap_or(50)).await
            }
            _ => Err(invalid_params(format!("invalid mode \"{mode}\": use single, double, element, navigate, or batch"))),
        }
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

    #[tool(description = "Scroll at the specified position. Coordinates can be from either a viewport or full-page screenshot — the system automatically scrolls to bring the position into view first. Use positive delta_y to scroll down, negative to scroll up. Typical scroll amount: 120 pixels per mouse wheel notch. PREFER vscreen_scroll_to_element when targeting a specific element. PREFER vscreen_screenshot(full_page=true) over scrolling to view below-fold content. Example: {\"instance_id\": \"dev\", \"x\": 512, \"y\": 400, \"delta_y\": 800}")]
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

        const TOOL_TIMEOUT: Duration = Duration::from_secs(300);
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

                    info!(round = rnd, screenshot_len = screenshot.len(), "solve_captcha: 2-phase analysis");

                    // ---- PHASE 1: Crop header and extract target + challenge type ----
                    // Get actual pixel dimensions for DPR-aware cropping
                    let (screenshot_pixel_w, screenshot_pixel_h) = image::load_from_memory(&screenshot)
                        .map(|img| img.dimensions())
                        .unwrap_or((iframe_w as u32, iframe_h as u32));
                    let _dpr_x = screenshot_pixel_w as f64 / iframe_w;
                    let dpr_y = screenshot_pixel_h as f64 / iframe_h;

                    let header_png = crop_region_png(
                        &screenshot,
                        0, 0,
                        screenshot_pixel_w,
                        (95.0 * dpr_y) as u32,
                    );

                    let header_result = if let Some(ref hdr) = header_png {
                        let v = vision.clone();
                        let hdr_owned = hdr.clone();
                        match tokio::time::timeout(
                            Duration::from_secs(20),
                            AbortOnDrop(tokio::spawn(async move {
                                v.analyze_fast(
                                    &hdr_owned,
                                    crate::vision::prompts::CAPTCHA_HEADER,
                                    Duration::from_secs(20),
                                ).await
                            })),
                        ).await {
                            Ok(Ok(Ok(resp))) => {
                                info!(round = rnd, text = %resp.text.chars().take(200).collect::<String>(), "solve_captcha: header response");
                                crate::vision::VisionClient::extract_json(&resp.text)
                                    .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                            }
                            Ok(Ok(Err(e))) => {
                                details.push(format!("Round {rnd}: header vision error: {e}"));
                                None
                            }
                            _ => {
                                details.push(format!("Round {rnd}: header vision timed out"));
                                None
                            }
                        }
                    } else {
                        details.push(format!("Round {rnd}: failed to crop header"));
                        None
                    };

                    let (target_name, challenge_type_from_header) = match header_result {
                        Some(ref hdr) => {
                            let t = hdr.get("target").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                            let ct = hdr.get("type").and_then(|v| v.as_str()).unwrap_or("dynamic").to_string();
                            details.push(format!("Round {rnd}: header → target={t}, type={ct}"));
                            (t, ct)
                        }
                        None => {
                            details.push(format!("Round {rnd}: header analysis failed, clicking verify/skip"));
                            self.cdp_click(&sup, verify_btn[0], verify_btn[1]).await?;
                            tokio::time::sleep(Duration::from_millis(500)).await;
                            continue;
                        }
                    };

                    // ---- PHASE 2: Crop individual tiles and analyze in parallel ----
                    // Determine actual pixel dimensions of the screenshot to handle DPR scaling
                    let (actual_w, actual_h) = match image::load_from_memory(&screenshot) {
                        Ok(img) => img.dimensions(),
                        Err(e) => {
                            details.push(format!("Round {rnd}: failed to decode screenshot: {e}"));
                            self.cdp_click(&sup, verify_btn[0], verify_btn[1]).await?;
                            tokio::time::sleep(Duration::from_millis(500)).await;
                            continue;
                        }
                    };
                    let scale_x = actual_w as f64 / iframe_w;
                    let scale_y = actual_h as f64 / iframe_h;
                    info!(round = rnd, actual_w, actual_h, iframe_w, iframe_h, scale_x, scale_y, "solve_captcha: screenshot dimensions");

                    let header_h = (95.0 * scale_y) as u32;
                    let footer_h = (65.0 * scale_y) as u32;
                    let grid_h = actual_h.saturating_sub(header_h).saturating_sub(footer_h);
                    if grid_h < 30 {
                        details.push(format!("Round {rnd}: screenshot too small for grid (h={actual_h}, header={header_h}, footer={footer_h})"));
                        self.cdp_click(&sup, verify_btn[0], verify_btn[1]).await?;
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        continue;
                    }
                    let n_cols: u32 = if grid_positions.len() == 9 { 3 } else { 4 };
                    let n_rows = n_cols;
                    let tile_w = actual_w / n_cols;
                    let tile_h = grid_h / n_rows;

                    let mut tile_crops: Vec<(usize, Vec<u8>)> = Vec::new();
                    for row in 0..n_rows {
                        for col in 0..n_cols {
                            let idx = (row * n_cols + col) as usize;
                            let tx = col * tile_w;
                            let ty = header_h + row * tile_h;
                            if let Some(png) = crop_region_png(&screenshot, tx, ty, tile_w, tile_h) {
                                tile_crops.push((idx + 1, png));
                            }
                        }
                    }
                    info!(round = rnd, tile_count = tile_crops.len(), tile_w, tile_h, "solve_captcha: tile crops");

                    // Process tiles in batches of 3 to avoid overwhelming the
                    // Ollama server — 9 concurrent requests queue up and ALL
                    // timeout. Batches of 3 keep each request within timeout.
                    let mut all_tile_results = Vec::new();
                    for batch in tile_crops.chunks(3) {
                        let batch_futures: Vec<_> = batch
                            .iter()
                            .map(|(tile_num, png_data)| {
                                let v = vision.clone();
                                let tn = *tile_num;
                                let pd = png_data.clone();
                                let prompt = CAPTCHA_TILE_SINGLE.replace("{target}", &target_name);
                                AbortOnDrop(tokio::spawn(async move {
                                    let result = v
                                        .analyze_fast(&pd, &prompt, Duration::from_secs(30))
                                        .await;
                                    (tn, result)
                                }))
                            })
                            .collect();
                        all_tile_results.extend(join_all(batch_futures).await);
                    }

                    let mut tile_indices: Vec<usize> = Vec::new();
                    for result in all_tile_results {
                        match result {
                            Ok((tile_num, Ok(resp))) => {
                                if let Some(json) = crate::vision::VisionClient::extract_json(&resp.text)
                                    .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                                {
                                    let contains = json.get("contains").and_then(|v| v.as_bool()).unwrap_or(false);
                                    let confidence = json.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                    info!(round = rnd, tile = tile_num, contains, confidence, "solve_captcha: tile result");
                                    if contains && confidence > 0.3 {
                                        tile_indices.push(tile_num);
                                    }
                                } else {
                                    info!(round = rnd, tile = tile_num, text = %resp.text.chars().take(200).collect::<String>(), "solve_captcha: tile no JSON");
                                }
                            }
                            Ok((tile_num, Err(e))) => {
                                info!(round = rnd, tile = tile_num, error = %e, "solve_captcha: tile vision error");
                            }
                            Err(e) => {
                                info!(error = %e, "solve_captcha: tile join error");
                            }
                        }
                    }

                    let challenge_type = challenge_type_from_header.clone();

                    details.push(format!(
                        "Round {rnd}: target=\"{target_name}\", type=\"{challenge_type}\", grid={}, tiles={tile_indices:?}",
                        grid_positions.len()
                    ));

                    info!(round = rnd, tiles = ?tile_indices, challenge_type = %challenge_type, grid_count = grid_positions.len(), "solve_captcha: parsed tiles");

                    let is_dynamic = challenge_type == "dynamic" || (grid_positions.len() == 9 && challenge_type != "select_all");

                    if is_dynamic && !tile_indices.is_empty() {
                        // ====== DYNAMIC REPLACEMENT FLOW ======
                        // Click initial matching tiles
                        let mut all_clicked: HashSet<usize> = HashSet::new();
                        for &idx in &tile_indices {
                            if let Some(pos) = idx.checked_sub(1).and_then(|i| grid_positions.get(i)) {
                                self.cdp_click(&sup, pos[0], pos[1]).await?;
                                all_clicked.insert(idx);
                                tokio::time::sleep(Duration::from_millis(100)).await;
                            }
                        }

                        // Wait for tile replacement animation before starting watcher
                        if let Some(ref bf_id) = bframe_id {
                            self.wait_for_tile_animation(&sup, bf_id, Duration::from_millis(1200)).await;
                        } else {
                            tokio::time::sleep(Duration::from_millis(800)).await;
                        }

                        let grid_clip = screenshot_watcher::ClipRegion {
                            x: iframe_x,
                            y: iframe_y + 95.0,
                            width: iframe_w,
                            height: iframe_h - 95.0 - 65.0,
                        };
                        let grid_dims = screenshot_watcher::GridDims {
                            rows: if grid_positions.len() == 9 { 3 } else { 4 },
                            cols: if grid_positions.len() == 9 { 3 } else { 4 },
                        };

                        // Start watcher — baseline captures the post-click state
                        // (initial tiles already have checkmarks from the clicks above).
                        let (mut watcher_handle, mut change_rx) =
                            screenshot_watcher::ScreenshotWatcher::start(
                                Arc::clone(&sup),
                                grid_clip.clone(),
                                grid_dims.clone(),
                                300,
                            );

                        let max_replacement_rounds = 6;
                        for sub_round in 0..max_replacement_rounds {
                            let change_event =
                                tokio::time::timeout(Duration::from_secs(5), change_rx.recv()).await;

                            let event = match change_event {
                                Ok(Ok(evt)) => evt,
                                _ => {
                                    details.push(format!(
                                        "Round {rnd}.{sub_round}: no tile changes detected, stopping"
                                    ));
                                    break;
                                }
                            };

                            // Only look at cells that were previously clicked
                            // (those are the ones that could have replacement tiles)
                            let replacement_cells: Vec<usize> = event
                                .changed_cells
                                .iter()
                                .filter(|&&cell_idx| all_clicked.contains(&(cell_idx + 1)))
                                .copied()
                                .collect();

                            if replacement_cells.is_empty() {
                                continue;
                            }

                            let vision_futures: Vec<_> = replacement_cells
                                .iter()
                                .filter_map(|&cell_idx| {
                                    let crop_data = event.cell_crops.get(&cell_idx)?.clone();
                                    let vision_clone = vision.clone();
                                    let prompt =
                                        CAPTCHA_TILE_SINGLE.replace("{target}", &target_name);
                                    Some(AbortOnDrop(tokio::spawn(async move {
                                        let result = vision_clone
                                            .analyze_fast(
                                                crop_data.as_ref(),
                                                &prompt,
                                                Duration::from_secs(15),
                                            )
                                            .await;
                                        (cell_idx, result)
                                    })))
                                })
                                .collect();

                            let vision_results = join_all(vision_futures).await;

                            let mut new_matches = Vec::new();
                            for result in vision_results {
                                if let Ok((cell_idx, Ok(resp))) = result {
                                    if let Some(json) = crate::vision::VisionClient::extract_json(
                                        &resp.text,
                                    )
                                    .and_then(|s| {
                                        serde_json::from_str::<serde_json::Value>(&s).ok()
                                    }) {
                                        let contains = json
                                            .get("contains")
                                            .and_then(|v| v.as_bool())
                                            .unwrap_or(false);
                                        let confidence = json
                                            .get("confidence")
                                            .and_then(|v| v.as_f64())
                                            .unwrap_or(0.0);
                                        details.push(format!(
                                            "Round {rnd}.{sub_round}: tile {} → contains={}, confidence={:.2}",
                                            cell_idx + 1,
                                            contains,
                                            confidence
                                        ));
                                        if contains && confidence > 0.3 {
                                            new_matches.push(cell_idx + 1);
                                        }
                                    }
                                }
                            }

                            if new_matches.is_empty() {
                                details.push(format!(
                                    "Round {rnd}.{sub_round}: no replacement tiles match"
                                ));
                                break;
                            }

                            // Click matching replacement tiles
                            for &tile_num in &new_matches {
                                if let Some(pos) = tile_num
                                    .checked_sub(1)
                                    .and_then(|i| grid_positions.get(i))
                                {
                                    self.cdp_click(&sup, pos[0], pos[1]).await?;
                                    all_clicked.insert(tile_num);
                                    tokio::time::sleep(Duration::from_millis(100)).await;
                                }
                            }
                            details.push(format!(
                                "Round {rnd}.{sub_round}: clicked replacement tiles {new_matches:?}"
                            ));

                            // CRITICAL FIX: After clicking tiles, the checkmark overlay
                            // changes the tile's visual hash. If we keep the same watcher,
                            // it detects the checkmark as a "change" and re-analyzes the
                            // same tile (which still contains the target), causing an
                            // infinite click/unclick toggle loop.
                            //
                            // Fix: stop the watcher, wait for the actual tile replacement
                            // animation to complete, then restart with a fresh baseline
                            // that includes the checkmarks. Only genuinely NEW replacement
                            // tiles (completely different images) will trigger future events.
                            watcher_handle.stop();
                            tokio::time::sleep(Duration::from_millis(1500)).await;
                            let (new_handle, new_rx) =
                                screenshot_watcher::ScreenshotWatcher::start(
                                    Arc::clone(&sup),
                                    grid_clip.clone(),
                                    grid_dims.clone(),
                                    300,
                                );
                            watcher_handle = new_handle;
                            change_rx = new_rx;
                        }

                        watcher_handle.stop();

                        // Click VERIFY
                        self.cdp_click(&sup, verify_btn[0], verify_btn[1]).await?;
                        details.push(format!(
                            "Round {rnd}: clicked verify after {clicks} tile clicks",
                            clicks = all_clicked.len()
                        ));

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
}

// Click helper methods (used by vscreen_click consolidated tool)
impl VScreenMcpServer {
    async fn click_helper_single(
        &self,
        instance_id: &str,
        x: f64,
        y: f64,
        button: Option<u8>,
        wait_after_ms: Option<u64>,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(instance_id)?;
        let sup_arc = self.get_supervisor(instance_id)?;

        let (vx, vy) = sup_arc
            .scroll_into_view_and_translate(x, y)
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        let btn = button.unwrap_or(0);
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

        self.record_action(instance_id, "click", &format!("({}, {})", x, y), &format!("Clicked at ({}, {})", x, y)).await;

        let mut msg = format!(
            "Clicked at page ({}, {}), viewport ({:.0}, {:.0})",
            x, y, vx, vy
        );

        if let Some(wait_ms) = wait_after_ms {
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

    async fn click_helper_double(
        &self,
        instance_id: &str,
        x: f64,
        y: f64,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(instance_id)?;
        let sup_arc = self.get_supervisor(instance_id)?;

        let (vx, vy) = sup_arc
            .scroll_into_view_and_translate(x, y)
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

        self.record_action(instance_id, "double_click", &format!("({}, {})", x, y), &format!("Double-clicked at ({}, {})", x, y)).await;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Double-clicked at page ({}, {}), viewport ({:.0}, {:.0})",
            x, y, vx, vy
        ))]))
    }

    async fn click_helper_element(
        &self,
        instance_id: &str,
        selector: Option<&str>,
        text: Option<&str>,
        text_exact: bool,
        index: usize,
        button: Option<u8>,
        wait_after_ms: Option<u64>,
        max_retries: u32,
        retry_delay: u64,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(instance_id)?;
        let sup = self.get_supervisor(instance_id)?;

        let selector_opt = selector.map(String::from);
        let text_opt = text.map(String::from);

        let build_find_js = |sel: &Option<String>, txt: &Option<String>, exact: bool, idx: usize| -> String {
            let find_expr = if let Some(s) = sel {
                let sel_json = serde_json::to_string(s).unwrap_or_default();
                let text_filter = if let Some(t) = txt {
                    let t_json = serde_json::to_string(t).unwrap_or_default();
                    if exact {
                        format!(".filter(el => (el.innerText || '').trim() === {t_json})")
                    } else {
                        format!(".filter(el => (el.innerText || '').toLowerCase().includes({t_json}.toLowerCase()))")
                    }
                } else {
                    String::new()
                };
                format!("Array.from(document.querySelectorAll({sel_json})){text_filter}")
            } else {
                let t_json = serde_json::to_string(txt.as_deref().unwrap_or("")).unwrap_or_default();
                let match_fn = if exact {
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
    target.scrollIntoView({{behavior: 'instant', block: 'center', inline: 'nearest'}});
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

        let js = build_find_js(&selector_opt, &text_opt, text_exact, index);

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

        let (vx, vy) = sup
            .scroll_into_view_and_translate(cx, cy)
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        let btn = button.unwrap_or(0);
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

        self.record_action(instance_id, "click_element", &format!("{tag}: {text}"), &format!("Clicked <{tag}> at ({cx}, {cy})")).await;

        let mut msg = format!(
            "Clicked <{tag}> \"{text}\" at page ({cx}, {cy}), viewport ({vx:.0}, {vy:.0}). {}/{found} matches.{retry_note}",
            index + 1,
        );

        if let Some(wait_ms) = wait_after_ms {
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

    async fn click_helper_batch(
        &self,
        instance_id: &str,
        points: &[[f64; 2]],
        delay: u64,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(instance_id)?;
        let sup = self.get_supervisor(instance_id)?;

        let mut clicked = 0u32;
        for point in points {
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

            if delay > 0 && (clicked as usize) < points.len() {
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            }
        }

        self.record_action(instance_id, "batch_click", &format!("{clicked} points"), &format!("Batch-clicked {clicked} points")).await;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Clicked {clicked} points with {delay}ms delay between each.",
        ))]))
    }

    async fn click_helper_navigate(
        &self,
        instance_id: &str,
        selector: Option<&str>,
        text: Option<&str>,
        timeout: u64,
        fallback_to_link: bool,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(instance_id)?;
        let sup = self.get_supervisor(instance_id)?;

        let before_url = sup.evaluate_js("location.href").await
            .map_err(|e| internal_error(e.to_string()))?;
        let before_url = before_url.as_str().unwrap_or("").to_string();

        let find_and_info_js = {
            let find_expr = if let Some(sel) = selector {
                let sel_json = serde_json::to_string(sel).unwrap_or_default();
                format!("document.querySelector({sel_json})")
            } else {
                let t_json = serde_json::to_string(text.unwrap_or("")).unwrap_or_default();
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

        if !navigated && fallback_to_link {
            if let Some(ref link_url) = href {
                if !link_url.is_empty() && link_url != &before_url {
                    let nav_js = format!("window.location.href = {}", serde_json::to_string(link_url).unwrap_or_default());
                    let _ = sup.evaluate_js(&nav_js).await;
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    navigated = true;
                }
            }
        }

        let page_info = sup.evaluate_js("JSON.stringify({url: location.href, title: document.title})").await
            .map_err(|e| internal_error(e.to_string()))?;
        let page_str = page_info.as_str().unwrap_or("{}");
        let page: serde_json::Value = serde_json::from_str(page_str).unwrap_or_default();
        let final_url = page.get("url").and_then(|v| v.as_str()).unwrap_or("?");
        let title = page.get("title").and_then(|v| v.as_str()).unwrap_or("?");

        let tag = el_info.get("tag").and_then(|v| v.as_str()).unwrap_or("?");
        let el_text = el_info.get("text").and_then(|v| v.as_str()).unwrap_or("");

        self.record_action(instance_id, "click_and_navigate", el_text, &format!("Navigated to {final_url}")).await;

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
}

/// Crop a region from a PNG screenshot and re-encode as PNG.
fn crop_region_png(png_data: &[u8], x: u32, y: u32, w: u32, h: u32) -> Option<Vec<u8>> {
    let img = image::load_from_memory(png_data).ok()?;
    let rgba = img.to_rgba8();
    let (img_w, img_h) = rgba.dimensions();
    let x = x.min(img_w.saturating_sub(1));
    let y = y.min(img_h.saturating_sub(1));
    let w = w.min(img_w - x);
    let h = h.min(img_h - y);
    if w == 0 || h == 0 {
        return None;
    }
    let cropped = image::imageops::crop_imm(&rgba, x, y, w, h).to_image();
    let mut buf = Vec::new();
    use image::codecs::png::PngEncoder;
    use image::ImageEncoder;
    PngEncoder::new(&mut buf)
        .write_image(cropped.as_raw(), w, h, image::ExtendedColorType::Rgba8)
        .ok()?;
    Some(buf)
}
