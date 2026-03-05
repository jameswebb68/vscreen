use std::time::Duration;

use base64::Engine;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::{tool, tool_router};

use super::params::*;
use super::{internal_error, invalid_params, McpError, VScreenMcpServer};

#[tool_router(router = workflow_interact_tools, vis = "pub(crate)")]
impl VScreenMcpServer {
    #[tool(description = "Perform an action on the page: click, type, select, hover, or scroll. Resolve target by text (default), CSS selector, or coordinates. Returns screenshot and page info. Example: {\"instance_id\": \"dev\", \"action\": \"click\", \"target\": \"Sign In\", \"target_type\": \"text\"}")]
    async fn vscreen_interact(
        &self,
        Parameters(params): Parameters<InteractParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;

        let action = params.action.to_lowercase();
        let target_type = params.target_type.as_deref().unwrap_or("text");

        // Resolve target coordinates
        let (page_x, page_y, target_found) = if target_type == "coordinates" {
            let x = params.x.ok_or_else(|| invalid_params("target_type=coordinates requires x and y"))?;
            let y = params.y.ok_or_else(|| invalid_params("target_type=coordinates requires x and y"))?;
            (x, y, true)
        } else if target_type == "selector" {
            let target = params.target.as_deref().ok_or_else(|| invalid_params("target_type=selector requires target"))?;
            let sel_json = serde_json::to_string(target).unwrap_or_default();
            let js = format!(
                r#"(function(){{
    const el = document.querySelector({sel_json});
    if (!el) return JSON.stringify({{found: false}});
    el.scrollIntoView({{behavior: 'instant', block: 'center', inline: 'nearest'}});
    const r = el.getBoundingClientRect();
    return JSON.stringify({{found: true, x: r.left + r.width/2 + window.scrollX, y: r.top + r.height/2 + window.scrollY}});
}})()"#,
            );
            let result = sup.evaluate_js(&js).await.map_err(|e| internal_error(e.to_string()))?;
            let s = result.as_str().unwrap_or("{}");
            let info: serde_json::Value = serde_json::from_str(s).unwrap_or_default();
            let found = info.get("found").and_then(|v| v.as_bool()).unwrap_or(false);
            let x = info.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let y = info.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
            (x, y, found)
        } else {
            // text (default)
            let target = params.target.as_deref().ok_or_else(|| invalid_params("target required for text/selector targeting"))?;
            let escaped = target.replace('\\', "\\\\").replace('"', "\\\"");
            let js = format!(
                r#"(function(){{
  const search = "{}";
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  while(walker.nextNode()) {{
    if(walker.currentNode.textContent.includes(search)) {{
      const el = walker.currentNode.parentElement;
      if (!el) continue;
      const r = el.getBoundingClientRect();
      if (r.width > 0 && r.height > 0) {{
        el.scrollIntoView({{behavior: 'instant', block: 'center', inline: 'nearest'}});
        const r2 = el.getBoundingClientRect();
        return JSON.stringify({{x: r2.left + r2.width/2 + window.scrollX, y: r2.top + r2.height/2 + window.scrollY, found: true}});
      }}
    }}
  }}
  return JSON.stringify({{found: false}});
}})()"#,
                escaped
            );
            let result = sup.evaluate_js(&js).await.map_err(|e| internal_error(e.to_string()))?;
            let s = result.as_str().unwrap_or("{}");
            let info: serde_json::Value = serde_json::from_str(s).unwrap_or_default();
            let found = info.get("found").and_then(|v| v.as_bool()).unwrap_or(false);
            let x = info.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let y = info.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
            (x, y, found)
        };

        if !target_found && action != "scroll" {
            return Ok(CallToolResult::success(vec![Content::text(
                serde_json::json!({
                    "success": false,
                    "action_performed": action,
                    "target_found": false,
                    "message": "Target element not found",
                    "hint": "Try vscreen_find(by=\"text\", text=\"...\") or vscreen_find(by=\"selector\", selector=\"...\") to search for elements, then use vscreen_click with coordinates.",
                })
                .to_string(),
            )]));
        }

        let url_before = sup
            .evaluate_js("location.href")
            .await
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default();

        // Perform action
        match action.as_str() {
            "click" => {
                self.cdp_click(&sup, page_x, page_y).await?;
            }
            "type" => {
                let value = params.value.as_deref().ok_or_else(|| invalid_params("action=type requires value"))?;
                self.cdp_click(&sup, page_x, page_y).await?;
                tokio::time::sleep(Duration::from_millis(100)).await;
                sup.send_cdp_command(
                    "Input.insertText",
                    Some(serde_json::json!({"text": value})),
                )
                .await
                .map_err(|e| internal_error(e.to_string()))?;
            }
            "select" => {
                let value = params.value.as_deref().ok_or_else(|| invalid_params("action=select requires value"))?;
                let target = params.target.as_deref().ok_or_else(|| invalid_params("action=select requires target (selector)"))?;
                let sel_json = serde_json::to_string(target).unwrap_or_default();
                let val_json = serde_json::to_string(value).unwrap_or_default();
                let js = format!(
                    r#"(function() {{
    const sel = document.querySelector({sel_json});
    if (!sel) return JSON.stringify({{error: 'Select not found'}});
    if (sel.tagName.toLowerCase() !== 'select') return JSON.stringify({{error: 'Not a select element'}});
    for (let i = 0; i < sel.options.length; i++) {{
        if (sel.options[i].value === {val_json} || sel.options[i].text.trim() === {val_json}) {{
            sel.selectedIndex = i;
            sel.dispatchEvent(new Event('change', {{bubbles: true}}));
            sel.dispatchEvent(new Event('input', {{bubbles: true}}));
            return JSON.stringify({{ok: true}});
        }}
    }}
    return JSON.stringify({{error: 'Option not found'}});
}})()"#
                );
                let result = sup.evaluate_js(&js).await.map_err(|e| internal_error(e.to_string()))?;
                let s = result.as_str().unwrap_or("{}");
                let info: serde_json::Value = serde_json::from_str(s).unwrap_or_default();
                if let Some(err) = info.get("error").and_then(|v| v.as_str()) {
                    return Err(invalid_params(err.to_string()));
                }
            }
            "hover" => {
                let (vx, vy) = sup
                    .scroll_into_view_and_translate(page_x, page_y)
                    .await
                    .map_err(|e| internal_error(e.to_string()))?;
                sup.send_cdp_command(
                    "Input.dispatchMouseEvent",
                    Some(serde_json::json!({"type": "mouseMoved", "x": vx, "y": vy, "modifiers": 0})),
                )
                .await
                .map_err(|e| internal_error(e.to_string()))?;
            }
            "scroll" => {
                let target = params.target.as_deref().ok_or_else(|| invalid_params("action=scroll requires target"))?;
                let js = if target_type == "selector" {
                    let sel_json = serde_json::to_string(target).unwrap_or_default();
                    format!(
                        r#"(function() {{
    const el = document.querySelector({sel_json});
    if (!el) return JSON.stringify({{error: 'Element not found'}});
    el.scrollIntoView({{behavior: 'instant', block: 'center', inline: 'nearest'}});
    return JSON.stringify({{ok: true}});
}})()"#
                    )
                } else {
                    let escaped = target.replace('\\', "\\\\").replace('"', "\\\"");
                    format!(
                        r#"(function() {{
    const search = "{}";
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    while(walker.nextNode()) {{
        if(walker.currentNode.textContent.includes(search)) {{
            const el = walker.currentNode.parentElement;
            if (el && el.getBoundingClientRect().width > 0) {{
                el.scrollIntoView({{behavior: 'instant', block: 'center', inline: 'nearest'}});
                return JSON.stringify({{ok: true}});
            }}
        }}
    }}
    return JSON.stringify({{error: 'Element not found'}});
}})()"#,
                        escaped
                    )
                };
                let result = sup.evaluate_js(&js).await.map_err(|e| internal_error(e.to_string()))?;
                let s = result.as_str().unwrap_or("{}");
                let info: serde_json::Value = serde_json::from_str(s).unwrap_or_default();
                if let Some(err) = info.get("error").and_then(|v| v.as_str()) {
                    return Err(invalid_params(err.to_string()));
                }
            }
            _ => {
                return Err(invalid_params(format!(
                    "invalid action '{action}': must be click, type, select, hover, or scroll"
                )));
            }
        }

        tokio::time::sleep(Duration::from_millis(500)).await;

        let screenshot_data = sup
            .capture_screenshot("png", None)
            .await
            .map_err(|e| internal_error(e.to_string()))?;
        let screenshot_b64 = base64::engine::general_purpose::STANDARD.encode(&screenshot_data);

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
        let page_changed = url != url_before;

        self.record_action(
            &params.instance_id,
            "interact",
            &format!("{} {:?}", action, params.target),
            &format!("{action} performed"),
        )
        .await;

        let result = serde_json::json!({
            "success": true,
            "action_performed": action,
            "target_found": target_found,
            "page_changed": page_changed,
            "url": url,
            "title": title,
        });

        Ok(CallToolResult::success(vec![
            Content::image(screenshot_b64, "image/png"),
            Content::text(serde_json::to_string_pretty(&result).unwrap_or_default()),
        ]))
    }

    #[tool(description = "Build/manage synthesis pages. Actions: 'list' (list pages), 'create' (create page with title/sections), 'scrape_and_create' (hint to use vscreen_synthesis_scrape_and_create). Requires --synthesis. Example: {\"action\": \"list\"} or {\"action\": \"create\", \"title\": \"My Page\", \"instance_id\": \"dev\"}")]
    async fn vscreen_synthesize(
        &self,
        Parameters(params): Parameters<SynthesizeParam>,
    ) -> Result<CallToolResult, McpError> {
        let action = params.action.to_lowercase();

        match action.as_str() {
            "list" => {
                let base_url = self.synthesis_base_url()?;
                let client = self.synthesis_client();
                let resp = client
                    .get(format!("{base_url}/api/pages"))
                    .send()
                    .await
                    .map_err(|e| internal_error(format!("synthesis request failed: {e}")))?;
                let text = resp.text().await.unwrap_or_default();
                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Synthesis pages:\n{text}"
                ))]))
            }
            "create" => {
                let title = params.title.ok_or_else(|| invalid_params("action=create requires title"))?;
                let base_url = self.synthesis_base_url()?;
                let client = self.synthesis_client();

                let body = serde_json::json!({
                    "title": title,
                    "subtitle": params.subtitle,
                    "theme": params.theme.unwrap_or_else(|| "dark".into()),
                    "layout": params.layout.unwrap_or_else(|| "grid".into()),
                    "sections": params.sections.unwrap_or_default(),
                });

                let resp = client
                    .post(format!("{base_url}/api/pages"))
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| internal_error(format!("synthesis request failed: {e}")))?;

                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();

                if !status.is_success() {
                    return Err(internal_error(format!("synthesis create failed ({status}): {text}")));
                }

                let page: serde_json::Value = serde_json::from_str(&text)
                    .map_err(|e| internal_error(format!("invalid JSON response: {e}")))?;
                let id = page.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
                let url = format!("{base_url}/page/{id}");

                let mut nav_status = String::new();
                if let Some(ref instance_id) = params.instance_id {
                    if self.require_exclusive(instance_id).is_ok() {
                        if let Ok(sup) = self.get_supervisor(instance_id) {
                            let nav_url = url.clone();
                            let _ = sup.navigate(&nav_url).await;
                            tokio::time::sleep(Duration::from_millis(1000)).await;
                            nav_status = format!("\nNavigated instance '{instance_id}' to {nav_url}");
                        }
                    }
                }

                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Created synthesis page '{}'.\npage_id: {id}\nurl: {url}{nav_status}",
                    title
                ))]))
            }
            "scrape_and_create" => {
                if params.urls.is_none() || params.title.is_none() || params.instance_id.is_none() {
                    return Err(invalid_params(
                        "action=scrape_and_create requires urls, title, and instance_id",
                    ));
                }
                let msg = "Use vscreen_synthesis_scrape_and_create directly for scrape+create workflows - it handles progressive rendering and parallel scraping.";
                Ok(CallToolResult::success(vec![Content::text(msg)]))
            }
            _ => Err(invalid_params(format!(
                "invalid action '{action}': must be list, create, or scrape_and_create"
            ))),
        }
    }

    #[tool(description = "Detect and handle page blockers: reCAPTCHA, cookie consent, ad overlays. Use challenge_type='auto' to auto-detect, or specify 'captcha', 'cookie_consent', or 'ad'. Returns detection result and recommended action. Example: {\"instance_id\": \"dev\", \"challenge_type\": \"auto\"}")]
    async fn vscreen_solve_challenge(
        &self,
        Parameters(params): Parameters<SolveChallengeParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;

        let challenge_type = params
            .challenge_type
            .as_deref()
            .unwrap_or("auto")
            .to_lowercase();

        match challenge_type.as_str() {
            "captcha" => {
                return Ok(CallToolResult::success(vec![Content::text(format!(
                    "reCAPTCHA detected or requested. Use vscreen_solve_captcha(instance_id: \"{}\") to solve the reCAPTCHA. Requires vision LLM (--vision-url).",
                    params.instance_id
                ))]));
            }
            "cookie_consent" => {
                // Try dismiss_dialogs logic directly
                let find_js = r#"(function() {
    const selectors = [
        '[class*="cookie"] button', '[class*="consent"] button',
        '[id*="cookie"] button', '[id*="consent"] button',
        '.cookie-banner button', '#cookie-banner button',
        'button[data-testid*="accept"]', 'button[data-testid*="agree"]',
        '#onetrust-accept-btn-handler', '.cc-accept-all', '#accept-all-cookies'
    ];
    for(const sel of selectors) {
        const btn = document.querySelector(sel);
        if(btn && btn.offsetHeight > 0) {
            const r = btn.getBoundingClientRect();
            return JSON.stringify({found: true, x: r.left + r.width/2 + window.scrollX, y: r.top + r.height/2 + window.scrollY});
        }
    }
    return JSON.stringify({found: false});
})()"#;
                let result = sup.evaluate_js(find_js).await.map_err(|e| internal_error(e.to_string()))?;
                let s = result.as_str().unwrap_or("{}");
                let info: serde_json::Value = serde_json::from_str(s).unwrap_or_default();
                if info.get("found").and_then(|v| v.as_bool()).unwrap_or(false) {
                    let x = info.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let y = info.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    self.cdp_click(&sup, x, y).await?;
                    self.record_action(&params.instance_id, "solve_challenge", "cookie_consent", "Dismissed consent dialog").await;
                    return Ok(CallToolResult::success(vec![Content::text(
                        "Cookie consent dialog detected and dismissed.",
                    )]));
                }
                return Ok(CallToolResult::success(vec![Content::text(
                    "No cookie consent dialog found.",
                )]));
            }
            "ad" => {
                return Ok(CallToolResult::success(vec![Content::text(format!(
                    "Ad overlay mode. Use vscreen_dismiss_ads(instance_id: \"{}\") to wait for and click skip buttons, or vscreen_click(mode=\"element\", text=\"Skip\") to click skip by text.",
                    params.instance_id
                ))]));
            }
            "auto" | _ => {
                // Auto-detect
                let recaptcha_js = r#"!!document.querySelector('iframe[src*="recaptcha"]')"#;
                let consent_js = r#"!!(document.querySelector('[class*="cookie"] button, [class*="consent"] button, [id*="cookie"] button, [id*="consent"] button'))"#;
                let ad_js = r#"!!(document.querySelector('.ytp-ad-skip-button, .ytp-skip-ad-button, [class*="ad-overlay"]'))"#;

                let (recaptcha, consent, ad) = tokio::join!(
                    sup.evaluate_js(recaptcha_js),
                    sup.evaluate_js(consent_js),
                    sup.evaluate_js(ad_js),
                );

                let has_recaptcha = recaptcha.ok().and_then(|v| v.as_bool()).unwrap_or(false);
                let has_consent = consent.ok().and_then(|v| v.as_bool()).unwrap_or(false);
                let has_ad = ad.ok().and_then(|v| v.as_bool()).unwrap_or(false);

                if has_recaptcha {
                    return Ok(CallToolResult::success(vec![Content::text(format!(
                        "Detected: reCAPTCHA. Use vscreen_solve_captcha(instance_id: \"{}\") to solve. Requires vision LLM (--vision-url).",
                        params.instance_id
                    ))]));
                }
                if has_consent {
                    // Try to dismiss
                    let find_js = r#"(function() {
    const selectors = [
        '[class*="cookie"] button', '[class*="consent"] button',
        '[id*="cookie"] button', '[id*="consent"] button',
        '#onetrust-accept-btn-handler', '.cc-accept-all'
    ];
    for(const sel of selectors) {
        const btn = document.querySelector(sel);
        if(btn && btn.offsetHeight > 0) {
            const r = btn.getBoundingClientRect();
            return JSON.stringify({found: true, x: r.left + r.width/2 + window.scrollX, y: r.top + r.height/2 + window.scrollY});
        }
    }
    return JSON.stringify({found: false});
})()"#;
                    let result = sup.evaluate_js(find_js).await.map_err(|e| internal_error(e.to_string()))?;
                    let s = result.as_str().unwrap_or("{}");
                    let info: serde_json::Value = serde_json::from_str(s).unwrap_or_default();
                    if info.get("found").and_then(|v| v.as_bool()).unwrap_or(false) {
                        let x = info.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let y = info.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
                        self.cdp_click(&sup, x, y).await?;
                        self.record_action(&params.instance_id, "solve_challenge", "cookie_consent", "Dismissed consent dialog").await;
                        return Ok(CallToolResult::success(vec![Content::text(
                            "Detected: cookie consent. Dismissed.",
                        )]));
                    }
                    return Ok(CallToolResult::success(vec![Content::text(
                        "Detected: cookie consent overlay but could not find dismiss button. Try vscreen_dismiss_dialogs.",
                    )]));
                }
                if has_ad {
                    return Ok(CallToolResult::success(vec![Content::text(format!(
                        "Detected: ad overlay. Use vscreen_dismiss_ads(instance_id: \"{}\") to wait for and click skip button.",
                        params.instance_id
                    ))]));
                }

                Ok(CallToolResult::success(vec![Content::text(
                    "No blocker detected (reCAPTCHA, cookie consent, or ad overlay).",
                )]))
            }
        }
    }
}
