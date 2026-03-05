use std::time::Duration;

use base64::Engine;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::{tool, tool_router};

use super::params::*;
use super::{internal_error, invalid_params, McpError, VScreenMcpServer};

const SCRAPER_TABLE_JS: &str = include_str!("scraper_table.js");
const SCRAPER_KV_JS: &str = include_str!("scraper_kv.js");
const SCRAPER_STATS_JS: &str = include_str!("scraper_stats.js");

#[tool_router(router = workflow_tools, vis = "pub(crate)")]
impl VScreenMcpServer {
    #[tool(description = "Navigate to a URL and see what's there. One-shot workflow: navigate, optionally dismiss dialogs, wait for content, capture screenshot, and return page info with optional text extraction. Use when you want to go to a page and get an overview. Example: {\"instance_id\": \"dev\", \"url\": \"https://example.com\", \"extract_text\": true}")]
    async fn vscreen_browse(
        &self,
        Parameters(params): Parameters<BrowseParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;

        // 1. Navigate via CDP
        let nav_params = serde_json::json!({"url": params.url, "transitionType": "typed"});
        sup.send_cdp_command("Page.navigate", Some(nav_params))
            .await
            .map_err(|e| {
                internal_error(format!(
                    "{}. Hint: Use vscreen_navigate(action=\"goto\", instance_id=\"...\", url=\"...\") for direct navigation with custom wait_until options.",
                    e
                ))
            })?;

        // 2. Wait for page load
        tokio::time::sleep(Duration::from_millis(1500)).await;

        // 3. Dismiss dialogs if requested (default: true)
        if params.dismiss_dialogs != Some(false) {
            let _ = sup
                .evaluate_js(r#"(function(){
  const selectors = [
    '[class*="cookie"] button', '[class*="consent"] button',
    '[id*="cookie"] button', '[id*="consent"] button',
    '.cookie-banner button', '#cookie-banner button',
    'button[data-testid*="accept"]', 'button[data-testid*="agree"]'
  ];
  for(const sel of selectors) {
    const btn = document.querySelector(sel);
    if(btn && btn.offsetHeight > 0) { btn.click(); return 'dismissed'; }
  }
  return 'none';
})()"#)
                .await;
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        // 4. If wait_for is set, poll up to 10s
        if let Some(ref wait_for) = params.wait_for {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            let is_selector = wait_for.contains('[')
                || wait_for.contains('(')
                || wait_for.starts_with('#')
                || wait_for.starts_with('.');
            let search = serde_json::to_string(wait_for).unwrap_or_default();

            let check_js = if is_selector {
                format!("!!document.querySelector({search})")
            } else {
                format!("document.body?.innerText?.includes({search}) || false")
            };

            loop {
                if let Ok(v) = sup.evaluate_js(&check_js).await {
                    if v.as_bool() == Some(true) {
                        break;
                    }
                }
                if tokio::time::Instant::now() >= deadline {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }

        // 5. Capture screenshot (full page if requested)
        let screenshot_data = if params.full_page == Some(true) {
            sup.capture_full_page_screenshot("png", None)
                .await
        } else {
            sup.capture_screenshot("png", None).await
        }
        .map_err(|e| internal_error(e.to_string()))?;

        let screenshot_b64 = base64::engine::general_purpose::STANDARD.encode(&screenshot_data);

        // 6. Get page info
        let page_info = sup
            .get_page_info()
            .await
            .map_err(|e| internal_error(e.to_string()))?;
        let url = page_info
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let title = page_info
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // 7. Extract text if requested
        let text_preview = if params.extract_text == Some(true) {
            let text_result = sup
                .evaluate_js("document.body?.innerText?.substring(0, 5000) || ''")
                .await
                .ok();
            let full_text = text_result
                .as_ref()
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let preview = if full_text.len() > 2000 {
                format!("{}...", &full_text[..2000])
            } else {
                full_text.to_string()
            };
            Some(preview)
        } else {
            None
        };

        // 8. Count links and forms
        let links_count: u32 = sup
            .evaluate_js("document.querySelectorAll('a[href]').length")
            .await
            .ok()
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let forms_count: u32 = sup
            .evaluate_js("document.querySelectorAll('form').length")
            .await
            .ok()
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        // 9. Record action
        self.record_action(
            &params.instance_id,
            "vscreen_browse",
            &params.url,
            &format!("url={}, title={}", url, title),
        )
        .await;

        // 10. Build result
        let mut result = serde_json::json!({
            "url": url,
            "title": title,
            "links_count": links_count,
            "forms_count": forms_count,
        });
        if let Some(ref t) = text_preview {
            result["text_preview"] = serde_json::json!(t);
        }

        let content = vec![
            Content::image(screenshot_b64, "image/png"),
            Content::text(serde_json::to_string_pretty(&result).unwrap_or_default()),
        ];
        Ok(CallToolResult::success(content))
    }

    #[tool(description = "Show what's on the page. Captures screenshot and optionally includes visible text and interactive elements summary. Use when you want to observe the current page state. Example: {\"instance_id\": \"dev\", \"include_elements\": true}")]
    async fn vscreen_observe(
        &self,
        Parameters(params): Parameters<ObserveParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;

        // 1. Capture screenshot
        let screenshot_data = if params.full_page == Some(true) {
            sup.capture_full_page_screenshot("png", None)
                .await
        } else {
            sup.capture_screenshot("png", None).await
        }
        .map_err(|e| internal_error(e.to_string()))?;

        let screenshot_b64 = base64::engine::general_purpose::STANDARD.encode(&screenshot_data);

        // 2. Get page info
        let page_info = sup
            .get_page_info()
            .await
            .map_err(|e| internal_error(e.to_string()))?;
        let url = page_info
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let title = page_info
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // 3. Include text if requested (default: true)
        let text_excerpt = if params.include_text != Some(false) {
            let text_result = sup
                .evaluate_js("document.body?.innerText?.substring(0, 3000) || ''")
                .await
                .ok();
            text_result
                .as_ref()
                .and_then(|v| v.as_str())
                .map(|s| {
                    if s.len() > 1500 {
                        format!("{}...", &s[..1500])
                    } else {
                        s.to_string()
                    }
                })
        } else {
            None
        };

        // 4. Include elements if requested (default: false)
        let element_summary = if params.include_elements == Some(true) {
            let js = r#"JSON.stringify((function(){
  const els = document.querySelectorAll('a, button, input, select, textarea, [role="button"]');
  const summary = { links: 0, buttons: 0, inputs: 0 };
  els.forEach(el => {
    const tag = (el.tagName || '').toLowerCase();
    const role = (el.getAttribute('role') || '').toLowerCase();
    if (tag === 'a' || role === 'link') summary.links++;
    else if (tag === 'button' || role === 'button') summary.buttons++;
    else if (['input','select','textarea'].includes(tag)) summary.inputs++;
  });
  return summary;
})())"#;
            let result = sup.evaluate_js(js).await.ok();
            result
                .as_ref()
                .and_then(|v| v.as_str())
                .map(String::from)
        } else {
            None
        };

        // 5. Record action
        self.record_action(
            &params.instance_id,
            "vscreen_observe",
            "",
            &format!("url={}, title={}", url, title),
        )
        .await;

        // 6. Build annotation text
        let mut annotation = format!("URL: {url}\nTitle: {title}\n");
        if let Some(ref t) = text_excerpt {
            annotation.push_str(&format!("\nText excerpt:\n{t}\n"));
        }
        if let Some(ref e) = element_summary {
            annotation.push_str(&format!("\nElements: {e}\n"));
        }

        let content = vec![
            Content::image(screenshot_b64, "image/png"),
            Content::text(annotation),
        ];
        Ok(CallToolResult::success(content))
    }

    #[tool(description = "Extract structured data from the page. Modes: articles (card/article elements), table (tabular data), kv (key-value pairs from definition lists and info boxes), stats (numeric statistics/metrics), links (all links), text (visible text), auto (tries articles/table/kv/stats/links). Optional selector to scope extraction. Example: {\"instance_id\": \"dev\", \"mode\": \"articles\"}")]
    async fn vscreen_extract(
        &self,
        Parameters(params): Parameters<ExtractParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;

        let mode = params.mode.to_lowercase();
        let scope_js = if let Some(ref sel) = params.selector {
            let sel_escaped = serde_json::to_string(sel).unwrap_or_else(|_| "\"\"".into());
            format!("document.querySelector({}) || document.body", sel_escaped)
        } else {
            "document.body".to_string()
        };

        let (mode_used, data, item_count, component_suggestion) = match mode.as_str() {
            "articles" => {
                let js = format!(
                    r#"(function(){{
  const root = {scope_js};
  const articles = [];
  (root || document.body).querySelectorAll('article, [class*="card"], [class*="article"]').forEach(el => {{
    const a = el.querySelector('a[href]');
    const img = el.querySelector('img');
    const title = el.querySelector('h1,h2,h3,h4')?.textContent?.trim();
    if(title && a) articles.push({{
      title, url: a.href,
      image: img?.src || null,
      description: el.querySelector('p')?.textContent?.trim()?.substring(0,200) || null
    }});
  }});
  return JSON.stringify(articles.slice(0, 20));
}})()"#,
                    scope_js = scope_js
                );
                let result = sup.evaluate_js(&js).await.map_err(|e| internal_error(e.to_string()))?;
                let s = result.as_str().unwrap_or("[]");
                let arr: Vec<serde_json::Value> = serde_json::from_str(s).unwrap_or_default();
                let count = arr.len();
                let suggestion = if count > 0 {
                    super::synthesis::pick_section_component(count)
                } else {
                    "none"
                };
                ("articles".to_string(), serde_json::json!(arr), count, suggestion.to_string())
            }
            "table" => {
                let js = SCRAPER_TABLE_JS.replace(
                    "const tables = document.querySelectorAll('table')",
                    &format!(
                        "const root = {}; const tables = (root || document.body).querySelectorAll('table')",
                        scope_js
                    ),
                );
                let result = sup.evaluate_js(&js).await.map_err(|e| internal_error(e.to_string()))?;
                let s = result.as_str().unwrap_or("[]");
                let arr: Vec<serde_json::Value> = serde_json::from_str(s).unwrap_or_default();
                let count = arr
                    .iter()
                    .map(|t| t.get("row_count").and_then(|v| v.as_u64()).unwrap_or(0) as usize)
                    .sum();
                ("table".to_string(), serde_json::json!(arr), count, "table".to_string())
            }
            "kv" | "key_value" => {
                let result = sup
                    .evaluate_js(SCRAPER_KV_JS)
                    .await
                    .map_err(|e| internal_error(e.to_string()))?;
                let s = result.as_str().unwrap_or("[]");
                let arr: Vec<serde_json::Value> = serde_json::from_str(s).unwrap_or_default();
                (
                    "kv".to_string(),
                    serde_json::json!(arr),
                    arr.len(),
                    "key-value-list".to_string(),
                )
            }
            "stats" => {
                let result = sup
                    .evaluate_js(SCRAPER_STATS_JS)
                    .await
                    .map_err(|e| internal_error(e.to_string()))?;
                let s = result.as_str().unwrap_or("[]");
                let arr: Vec<serde_json::Value> = serde_json::from_str(s).unwrap_or_default();
                (
                    "stats".to_string(),
                    serde_json::json!(arr),
                    arr.len(),
                    "stats-row".to_string(),
                )
            }
            "links" => {
                let js = r#"(function(){
  return JSON.stringify(Array.from(document.querySelectorAll('a[href]')).map(a => ({
    text: a.textContent.trim().substring(0,100),
    href: a.href
  })).filter(l => l.text && l.href).slice(0,100));
})()"#;
                let result = sup.evaluate_js(js).await.map_err(|e| internal_error(e.to_string()))?;
                let s = result.as_str().unwrap_or("[]");
                let arr: Vec<serde_json::Value> = serde_json::from_str(s).unwrap_or_default();
                ("links".to_string(), serde_json::json!(arr), arr.len(), "link-list".to_string())
            }
            "text" => {
                let js = format!(
                    r#"(function(){{ const root = {scope_js}; return (root || document.body)?.innerText || ''; }})()"#,
                    scope_js = scope_js
                );
                let result = sup.evaluate_js(&js).await.map_err(|e| internal_error(e.to_string()))?;
                let s = result.as_str().unwrap_or("");
                ("text".to_string(), serde_json::json!(s), s.len(), "text".to_string())
            }
            "auto" => {
                // Try articles first
                let articles_js = r#"(function(){
  const articles = [];
  document.querySelectorAll('article, [class*="card"], [class*="article"]').forEach(el => {
    const a = el.querySelector('a[href]');
    const img = el.querySelector('img');
    const title = el.querySelector('h1,h2,h3,h4')?.textContent?.trim();
    if(title && a) articles.push({
      title, url: a.href,
      image: img?.src || null,
      description: el.querySelector('p')?.textContent?.trim()?.substring(0,200) || null
    });
  });
  return JSON.stringify(articles.slice(0, 20));
})()"#;
                if let Ok(r) = sup.evaluate_js(articles_js).await {
                    let s = r.as_str().unwrap_or("[]");
                    let arr: Vec<serde_json::Value> = serde_json::from_str(s).unwrap_or_default();
                    if !arr.is_empty() {
                        let suggestion = super::synthesis::pick_section_component(arr.len());
                        return self.finish_extract(
                            &params.instance_id,
                            "articles",
                            serde_json::json!(arr),
                            arr.len(),
                            suggestion.to_string(),
                        )
                        .await;
                    }
                }

                // Try table (using scraper_table.js)
                let table_js = SCRAPER_TABLE_JS.replace(
                    "const tables = document.querySelectorAll('table')",
                    &format!(
                        "const root = {}; const tables = (root || document.body).querySelectorAll('table')",
                        scope_js
                    ),
                );
                if let Ok(r) = sup.evaluate_js(&table_js).await {
                    let s = r.as_str().unwrap_or("[]");
                    let arr: Vec<serde_json::Value> = serde_json::from_str(s).unwrap_or_default();
                    let count: usize = arr
                        .iter()
                        .map(|t| t.get("row_count").and_then(|v| v.as_u64()).unwrap_or(0) as usize)
                        .sum();
                    if count > 0 {
                        return self
                            .finish_extract(
                                &params.instance_id,
                                "table",
                                serde_json::json!(arr),
                                count,
                                "table".to_string(),
                            )
                            .await;
                    }
                }

                // Try kv
                if let Ok(r) = sup.evaluate_js(SCRAPER_KV_JS).await {
                    let s = r.as_str().unwrap_or("[]");
                    let arr: Vec<serde_json::Value> = serde_json::from_str(s).unwrap_or_default();
                    if !arr.is_empty() {
                        return self
                            .finish_extract(
                                &params.instance_id,
                                "kv",
                                serde_json::json!(arr),
                                arr.len(),
                                "key-value-list".to_string(),
                            )
                            .await;
                    }
                }

                // Try stats
                if let Ok(r) = sup.evaluate_js(SCRAPER_STATS_JS).await {
                    let s = r.as_str().unwrap_or("[]");
                    let arr: Vec<serde_json::Value> = serde_json::from_str(s).unwrap_or_default();
                    if !arr.is_empty() {
                        return self
                            .finish_extract(
                                &params.instance_id,
                                "stats",
                                serde_json::json!(arr),
                                arr.len(),
                                "stats-row".to_string(),
                            )
                            .await;
                    }
                }

                // Fall back to links
                let links_js = r#"(function(){
  return JSON.stringify(Array.from(document.querySelectorAll('a[href]')).map(a => ({
    text: a.textContent.trim().substring(0,100),
    href: a.href
  })).filter(l => l.text && l.href).slice(0,100));
})()"#;
                let result = sup.evaluate_js(links_js).await.map_err(|e| internal_error(e.to_string()))?;
                let s = result.as_str().unwrap_or("[]");
                let arr: Vec<serde_json::Value> = serde_json::from_str(s).unwrap_or_default();
                ("links".to_string(), serde_json::json!(arr), arr.len(), "link-list".to_string())
            }
            _ => {
                return Err(invalid_params(format!(
                    "invalid mode '{}': must be 'articles', 'table', 'kv', 'stats', 'links', 'text', or 'auto'",
                    params.mode
                )));
            }
        };

        self.finish_extract(
            &params.instance_id,
            &mode_used,
            data,
            item_count,
            component_suggestion,
        )
        .await
    }

    async fn finish_extract(
        &self,
        instance_id: &str,
        mode_used: &str,
        data: serde_json::Value,
        item_count: usize,
        component_suggestion: String,
    ) -> Result<CallToolResult, McpError> {
        self.record_action(
            instance_id,
            "vscreen_extract",
            mode_used,
            &format!("mode={}, items={}", mode_used, item_count),
        )
        .await;

        let mut result = serde_json::json!({
            "mode_used": mode_used,
            "data": data,
            "item_count": item_count,
            "component_suggestion": component_suggestion,
        });
        if item_count == 0 {
            result["hint"] = serde_json::json!(
                "Try vscreen_execute_js for custom extraction, or vscreen_extract_text for raw text content."
            );
        }

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        )]))
    }
}
