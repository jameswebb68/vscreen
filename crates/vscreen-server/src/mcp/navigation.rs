use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::{tool, tool_router};

use super::params::*;
use super::{internal_error, McpError, VScreenMcpServer};

// ---------------------------------------------------------------------------
// Private helpers (logic from original tools, used by consolidated tools)
// ---------------------------------------------------------------------------

impl VScreenMcpServer {
    async fn nav_helper_goto(
        &self,
        instance_id: &str,
        url: &str,
        wait_until: Option<&str>,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(instance_id)?;
        let sup = self.get_supervisor(instance_id)?;
        sup.navigate(url)
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        let wait_until = wait_until.unwrap_or("load");
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
                    if tokio::time::Instant::now() >= deadline {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }
            "networkidle" => {
                let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
                loop {
                    let state = sup.evaluate_js("document.readyState").await;
                    if let Ok(v) = state {
                        if v.as_str().unwrap_or("") == "complete" {
                            break;
                        }
                    }
                    if tokio::time::Instant::now() >= deadline {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            _ => {
                let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
                loop {
                    let state = sup.evaluate_js("document.readyState").await;
                    if let Ok(v) = state {
                        if v.as_str().unwrap_or("") == "complete" {
                            break;
                        }
                    }
                    if tokio::time::Instant::now() >= deadline {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }
        }

        let _ = sup.enable_console_capture().await;

        let page_info = sup.get_page_info().await.unwrap_or(serde_json::Value::Null);
        let final_url = page_info
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or(url);
        let title = page_info.get("title").and_then(|v| v.as_str()).unwrap_or("");

        self.record_action(
            instance_id,
            "navigate",
            url,
            &format!("Navigated to {}", url),
        )
        .await;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Navigated to {final_url}\nTitle: {title}"
        ))]))
    }

    async fn nav_helper_back(&self, instance_id: &str) -> Result<CallToolResult, McpError> {
        self.require_exclusive(instance_id)?;
        let sup = self.get_supervisor(instance_id)?;
        sup.evaluate_js("history.back()")
            .await
            .map_err(|e| internal_error(e.to_string()))?;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        Ok(CallToolResult::success(vec![Content::text(
            "Navigated back in history.",
        )]))
    }

    async fn nav_helper_forward(&self, instance_id: &str) -> Result<CallToolResult, McpError> {
        self.require_exclusive(instance_id)?;
        let sup = self.get_supervisor(instance_id)?;
        sup.evaluate_js("history.forward()")
            .await
            .map_err(|e| internal_error(e.to_string()))?;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        Ok(CallToolResult::success(vec![Content::text(
            "Navigated forward in history.",
        )]))
    }

    async fn nav_helper_reload(&self, instance_id: &str) -> Result<CallToolResult, McpError> {
        self.require_exclusive(instance_id)?;
        let sup = self.get_supervisor(instance_id)?;
        sup.send_cdp_command("Page.reload", Some(serde_json::json!({"ignoreCache": false})))
            .await
            .map_err(|e| internal_error(e.to_string()))?;
        Ok(CallToolResult::success(vec![Content::text(
            "Page reload initiated.",
        )]))
    }

    async fn wait_helper_duration(&self, duration_ms: u64) -> Result<CallToolResult, McpError> {
        tokio::time::sleep(std::time::Duration::from_millis(duration_ms)).await;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Waited {}ms",
            duration_ms
        ))]))
    }

    async fn wait_helper_idle(
        &self,
        instance_id: &str,
        timeout_ms: u64,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(instance_id)?;
        let sup = self.get_supervisor(instance_id)?;
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

    async fn wait_helper_text(
        &self,
        instance_id: &str,
        text: &str,
        timeout: u64,
        interval: u64,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(instance_id)?;
        let sup = self.get_supervisor(instance_id)?;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout);
        let search = serde_json::to_string(text).unwrap_or_default();
        let js = format!("document.body.innerText.includes({search})");

        loop {
            {
                let result = sup.evaluate_js(&js).await.map_err(|e| internal_error(e.to_string()))?;
                if result.as_bool() == Some(true) {
                    return Ok(CallToolResult::success(vec![Content::text(format!(
                        "Text '{}' found on page.",
                        text
                    ))]));
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(CallToolResult::success(vec![Content::text(format!(
                    "Timed out after {}ms waiting for text '{}'.",
                    timeout, text
                ))]));
            }
            tokio::time::sleep(std::time::Duration::from_millis(interval)).await;
        }
    }

    async fn wait_helper_selector(
        &self,
        instance_id: &str,
        selector: &str,
        visible: bool,
        timeout: u64,
        interval: u64,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(instance_id)?;
        let sup = self.get_supervisor(instance_id)?;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout);
        let sel = serde_json::to_string(selector).unwrap_or_default();
        let js = if visible {
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
                        selector
                    ))]));
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(CallToolResult::success(vec![Content::text(format!(
                    "Timed out after {}ms waiting for selector '{}'.",
                    timeout, selector
                ))]));
            }
            tokio::time::sleep(std::time::Duration::from_millis(interval)).await;
        }
    }

    async fn wait_helper_url(
        &self,
        instance_id: &str,
        url_contains: &str,
        timeout: u64,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(instance_id)?;
        let sup = self.get_supervisor(instance_id)?;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout);

        loop {
            {
                let result = sup
                    .evaluate_js("location.href")
                    .await
                    .map_err(|e| internal_error(e.to_string()))?;
                if let Some(url) = result.as_str() {
                    if url.contains(url_contains) {
                        return Ok(CallToolResult::success(vec![Content::text(format!(
                            "URL now contains '{}': {}",
                            url_contains, url
                        ))]));
                    }
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(CallToolResult::success(vec![Content::text(format!(
                    "Timed out after {}ms waiting for URL to contain '{}'.",
                    timeout, url_contains
                ))]));
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    }

    async fn wait_helper_network(
        &self,
        instance_id: &str,
        idle_threshold: u64,
        timeout: u64,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(instance_id)?;
        let sup = self.get_supervisor(instance_id)?;
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
                    let last_activity = info
                        .get("lastActivity")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0);

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
}

// ---------------------------------------------------------------------------
// Consolidated tools (2 tools replace 10)
// ---------------------------------------------------------------------------

#[tool_router(router = navigation_tools, vis = "pub(crate)")]
impl VScreenMcpServer {
    #[tool(description = "Navigate the browser. Actions: 'goto' (navigate to URL, default), 'back' (browser back), 'forward' (browser forward), 'reload' (reload page). For goto: set url (required) and wait_until ('load'|'domcontentloaded'|'networkidle'|'none'). Example: {\"instance_id\": \"dev\", \"url\": \"https://example.com\"} or {\"instance_id\": \"dev\", \"action\": \"back\"}")]
    async fn vscreen_navigate(
        &self,
        Parameters(params): Parameters<ConsolidatedNavigateParam>,
    ) -> Result<CallToolResult, McpError> {
        match params.action.as_str() {
            "back" => self.nav_helper_back(&params.instance_id).await,
            "forward" => self.nav_helper_forward(&params.instance_id).await,
            "reload" => self.nav_helper_reload(&params.instance_id).await,
            _ => {
                let url = params
                    .url
                    .as_ref()
                    .map(|s| s.as_str())
                    .ok_or_else(|| super::invalid_params("url is required for action='goto'"))?;
                self.nav_helper_goto(
                    &params.instance_id,
                    url,
                    params.wait_until.as_deref(),
                )
                .await
            }
        }
    }

    #[tool(description = "Wait for a condition. Conditions: 'duration' (fixed ms, default), 'idle' (page idle), 'text' (text appears on page), 'selector' (CSS element appears), 'url' (URL contains substring), 'network' (network activity stops). Example: {\"duration_ms\": 2000} or {\"instance_id\": \"dev\", \"condition\": \"text\", \"text\": \"Welcome\"}")]
    pub(crate) async fn vscreen_wait(
        &self,
        Parameters(params): Parameters<ConsolidatedWaitParam>,
    ) -> Result<CallToolResult, McpError> {
        match params.condition.as_str() {
            "idle" => {
                let instance_id = params
                    .instance_id
                    .as_ref()
                    .ok_or_else(|| super::invalid_params("instance_id required for condition='idle'"))?;
                let timeout_ms = params.timeout_ms.unwrap_or(5000);
                self.wait_helper_idle(instance_id, timeout_ms).await
            }
            "text" => {
                let instance_id = params
                    .instance_id
                    .as_ref()
                    .ok_or_else(|| super::invalid_params("instance_id required for condition='text'"))?;
                let text = params
                    .text
                    .as_ref()
                    .map(|s| s.as_str())
                    .ok_or_else(|| super::invalid_params("text required for condition='text'"))?;
                let timeout = params.timeout_ms.unwrap_or(10000);
                let interval = params.interval_ms.unwrap_or(250);
                self.wait_helper_text(instance_id, text, timeout, interval).await
            }
            "selector" => {
                let instance_id = params
                    .instance_id
                    .as_ref()
                    .ok_or_else(|| super::invalid_params("instance_id required for condition='selector'"))?;
                let selector = params
                    .selector
                    .as_ref()
                    .map(|s| s.as_str())
                    .ok_or_else(|| super::invalid_params("selector required for condition='selector'"))?;
                let timeout = params.timeout_ms.unwrap_or(10000);
                let interval = params.interval_ms.unwrap_or(250);
                self.wait_helper_selector(
                    instance_id,
                    selector,
                    params.visible,
                    timeout,
                    interval,
                )
                .await
            }
            "url" => {
                let instance_id = params
                    .instance_id
                    .as_ref()
                    .ok_or_else(|| super::invalid_params("instance_id required for condition='url'"))?;
                let url_contains = params
                    .url_contains
                    .as_ref()
                    .map(|s| s.as_str())
                    .ok_or_else(|| super::invalid_params("url_contains required for condition='url'"))?;
                let timeout = params.timeout_ms.unwrap_or(10000);
                self.wait_helper_url(instance_id, url_contains, timeout).await
            }
            "network" => {
                let instance_id = params
                    .instance_id
                    .as_ref()
                    .ok_or_else(|| super::invalid_params("instance_id required for condition='network'"))?;
                let idle_ms = params.idle_ms.unwrap_or(500);
                let timeout = params.timeout_ms.unwrap_or(10000);
                self.wait_helper_network(instance_id, idle_ms, timeout).await
            }
            _ => {
                let duration_ms = params.duration_ms.unwrap_or(1000);
                self.wait_helper_duration(duration_ms).await
            }
        }
    }
}
