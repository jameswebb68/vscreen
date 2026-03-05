use base64::Engine;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::{tool, tool_router};

use super::params::*;
use super::{internal_error, invalid_params, McpError, VScreenMcpServer};

#[tool_router(router = session_tools, vis = "pub(crate)")]
impl VScreenMcpServer {
    #[tool(description = "Manage screenshot history. Actions: 'list' (list entries, no images), 'get' (retrieve specific screenshot by index), 'range' (retrieve range from/count), 'clear' (clear buffer). Example: {\"instance_id\": \"dev\", \"action\": \"list\"} or {\"instance_id\": \"dev\", \"action\": \"get\", \"index\": 0}")]
    async fn vscreen_history(
        &self,
        Parameters(params): Parameters<ConsolidatedHistoryParam>,
    ) -> Result<CallToolResult, McpError> {
        match params.action.as_str() {
            "list" => {
                self.require_observer_or_exclusive(&params.instance_id)?;
                let sup = self.get_supervisor(&params.instance_id)?;
                let meta = sup.with_screenshot_history(|h| h.list());
                let text = serde_json::to_string_pretty(&meta).unwrap_or_default();
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }
            "get" => {
                let index = params
                    .index
                    .ok_or_else(|| invalid_params("action='get' requires 'index'"))?;
                self.require_observer_or_exclusive(&params.instance_id)?;
                let sup = self.get_supervisor(&params.instance_id)?;
                let entry = sup
                    .with_screenshot_history(|h| h.get(index).cloned())
                    .ok_or_else(|| invalid_params(format!("no screenshot at index {}", index)))?;
                let b64 = base64::engine::general_purpose::STANDARD.encode(&entry.data);
                Ok(CallToolResult::success(vec![
                    Content::text(serde_json::json!({
                        "index": index,
                        "timestamp_ms": entry.timestamp_ms,
                        "url": entry.url,
                        "action_label": entry.action_label,
                    }).to_string()),
                    Content::image(b64, "image/jpeg"),
                ]))
            }
            "range" => {
                let from = params
                    .from
                    .ok_or_else(|| invalid_params("action='range' requires 'from'"))?;
                let count = params
                    .count
                    .ok_or_else(|| invalid_params("action='range' requires 'count'"))?;
                self.require_observer_or_exclusive(&params.instance_id)?;
                let sup = self.get_supervisor(&params.instance_id)?;
                let entries = sup.with_screenshot_history(|h| {
                    h.get_range(from, count)
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
                        "index": from + i,
                        "timestamp_ms": entry.timestamp_ms,
                        "url": entry.url,
                        "action_label": entry.action_label,
                    }).to_string()));
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&entry.data);
                    content.push(Content::image(b64, "image/jpeg"));
                }
                Ok(CallToolResult::success(content))
            }
            "clear" => {
                self.require_exclusive(&params.instance_id)?;
                let sup = self.get_supervisor(&params.instance_id)?;
                sup.with_screenshot_history_mut(|h| h.clear());
                Ok(CallToolResult::success(vec![Content::text(
                    "Screenshot history cleared.",
                )]))
            }
            _ => Err(invalid_params(format!(
                "unknown action '{}'; use 'list', 'get', 'range', or 'clear'",
                params.action
            ))),
        }
    }

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

    #[tool(description = "Read/write cookies and web storage. Type: 'cookie', 'local' (localStorage), 'session' (sessionStorage). Action: 'get' or 'set'. For cookies: get returns all cookies; set requires name+value (optional: domain, path). For storage: get requires key; set requires key+value. Example: {\"instance_id\": \"dev\", \"type\": \"cookie\", \"action\": \"get\"} or {\"instance_id\": \"dev\", \"type\": \"local\", \"action\": \"set\", \"key\": \"theme\", \"value\": \"dark\"}")]
    async fn vscreen_storage(
        &self,
        Parameters(params): Parameters<ConsolidatedStorageParam>,
    ) -> Result<CallToolResult, McpError> {
        match (params.storage_type.as_str(), params.action.as_str()) {
            ("cookie", "get") => {
                self.require_observer_or_exclusive(&params.instance_id)?;
                let sup = self.get_supervisor(&params.instance_id)?;
                let result = sup
                    .send_cdp_command_and_wait("Network.getCookies", None)
                    .await
                    .map_err(|e| internal_error(e.to_string()))?;
                let text = serde_json::to_string_pretty(&result).unwrap_or_default();
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }
            ("cookie", "set") => {
                let name = params
                    .name
                    .ok_or_else(|| invalid_params("type='cookie' action='set' requires 'name'"))?;
                let value = params
                    .value
                    .ok_or_else(|| invalid_params("type='cookie' action='set' requires 'value'"))?;
                self.require_exclusive(&params.instance_id)?;
                let sup = self.get_supervisor(&params.instance_id)?;
                let mut cookie = serde_json::json!({
                    "name": name,
                    "value": value,
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
                    name
                ))]))
            }
            ("local" | "session", "get") => {
                let key = params
                    .key
                    .ok_or_else(|| invalid_params("type='local'/'session' action='get' requires 'key'"))?;
                self.require_observer_or_exclusive(&params.instance_id)?;
                let sup = self.get_supervisor(&params.instance_id)?;
                let storage = if params.storage_type == "session" {
                    "sessionStorage"
                } else {
                    "localStorage"
                };
                let key_json = serde_json::to_string(&key).unwrap_or_default();
                let js = format!("{storage}.getItem({key_json})");
                let result = sup
                    .evaluate_js(&js)
                    .await
                    .map_err(|e| internal_error(e.to_string()))?;
                let text = serde_json::to_string_pretty(&result).unwrap_or_default();
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }
            ("local" | "session", "set") => {
                let key = params
                    .key
                    .ok_or_else(|| invalid_params("type='local'/'session' action='set' requires 'key'"))?;
                let value = params
                    .value
                    .ok_or_else(|| invalid_params("type='local'/'session' action='set' requires 'value'"))?;
                self.require_exclusive(&params.instance_id)?;
                let sup = self.get_supervisor(&params.instance_id)?;
                let storage = if params.storage_type == "session" {
                    "sessionStorage"
                } else {
                    "localStorage"
                };
                let key_json = serde_json::to_string(&key).unwrap_or_default();
                let val_json = serde_json::to_string(&value).unwrap_or_default();
                let js = format!("{storage}.setItem({key_json}, {val_json})");
                sup.evaluate_js(&js)
                    .await
                    .map_err(|e| internal_error(e.to_string()))?;
                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Stored '{}'='{}' in {storage}.",
                    key, value
                ))]))
            }
            _ => Err(invalid_params(format!(
                "invalid type/action: type must be 'cookie', 'local', or 'session'; action must be 'get' or 'set'"
            ))),
        }
    }
}
