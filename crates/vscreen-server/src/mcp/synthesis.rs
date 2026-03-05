use std::sync::Arc;

use crate::supervisor::InstanceSupervisor;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::{tool, tool_router};

use super::params::*;
use super::{internal_error, invalid_params, McpError, VScreenMcpServer};

pub(crate) fn pick_section_component(article_count: usize) -> &'static str {
    match article_count {
        0..=3 => "hero",
        4..=12 => "card-grid",
        _ => "content-list",
    }
}

pub(super) struct ScrapeResult {
    pub url: String,
    pub page_title: String,
    pub articles: serde_json::Value,
    pub quality: serde_json::Value,
    pub push_status: Option<PushStatus>,
}

pub(super) enum PushStatus {
    Ok,
    Failed(String),
}

pub(super) enum ScrapeOutcome {
    Ok(ScrapeResult),
    Err { url: String, error: String },
    Panic(String),
}

pub(super) struct PushConfig {
    pub client: reqwest::Client,
    pub base_url: String,
    pub page_id: String,
}

/// Shared parallel scraping engine used by both `scrape_batch` and `scrape_and_create`.
/// Creates ephemeral tabs, scrapes each URL in parallel, optionally pushes results
/// to a synthesis page, then closes all tabs.
pub(super) async fn run_parallel_scrape(
    sup: &Arc<InstanceSupervisor>,
    urls: &[SynthesisScrapeUrlEntry],
    section_ids: Option<&[String]>,
    push_config: Option<&PushConfig>,
) -> Result<Vec<ScrapeOutcome>, McpError> {
    let mut tabs = Vec::new();
    for entry in urls {
        match sup.create_ephemeral_tab("about:blank").await {
            Ok(tab) => tabs.push(tab),
            Err(e) => {
                for tab in &tabs {
                    let _ = sup.close_ephemeral_tab(tab).await;
                }
                return Err(internal_error(format!("failed to create tab for {}: {e}", entry.url)));
            }
        }
    }

    let mut handles = Vec::new();
    for (i, entry) in urls.iter().enumerate() {
        let tab_client = tabs[i].client.clone();
        let url = entry.url.clone();
        let limit = entry.limit.unwrap_or(8);
        let source_raw = entry.source_label.clone().unwrap_or_default();
        let source = source_raw
            .replace('\\', "\\\\")
            .replace('\'', "\\'")
            .replace('\n', "\\n")
            .replace('\r', "\\r");

        let budget_ms: u64 = 12000;
        let js = include_str!("scraper.js")
            .replace("__LIMIT__", &limit.to_string())
            .replace("__SOURCE__", &source)
            .replace("__TIMEOUT_BUDGET_MS__", &budget_ms.to_string());

        let section_id = section_ids.map(|ids| ids[i].clone());
        let push_client = push_config.map(|pc| (pc.client.clone(), pc.base_url.clone(), pc.page_id.clone()));

        handles.push(tokio::spawn(async move {
            let client = tab_client.lock().await;
            if let Err(e) = client.navigate(&url).await {
                return ScrapeOutcome::Err { url, error: format!("navigate failed: {e}") };
            }
            drop(client);

            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(12);
            loop {
                let client = tab_client.lock().await;
                let state = client.evaluate_js("document.readyState").await;
                drop(client);
                if let Ok(v) = state {
                    if v.as_str().unwrap_or("") == "complete" { break; }
                }
                if tokio::time::Instant::now() >= deadline { break; }
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }

            {
                let client = tab_client.lock().await;
                let _ = client.evaluate_js("window.scrollTo(0, window.innerHeight * 3); void(0)").await;
            }
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            {
                let client = tab_client.lock().await;
                let _ = client.evaluate_js("window.scrollTo(0, 0); void(0)").await;
            }
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;

            let client = tab_client.lock().await;
            let result = match client.evaluate_js_async(&js).await {
                Ok(r) => r,
                Err(e) => return ScrapeOutcome::Err { url, error: format!("scrape failed: {e}") },
            };
            let page_title = client.evaluate_js("document.title").await
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default();
            drop(client);

            let raw_str = result.as_str().unwrap_or("{}");
            let parsed: serde_json::Value = serde_json::from_str(raw_str)
                .unwrap_or(serde_json::json!({"articles": [], "quality": {}}));
            let articles = parsed.get("articles").cloned()
                .unwrap_or(serde_json::Value::Array(vec![]));
            let quality = parsed.get("quality").cloned()
                .unwrap_or(serde_json::Value::Null);

            let article_count = articles.as_array().map(|a| a.len()).unwrap_or(0);

            let push_status = if let (Some(sid), Some((pc, base, pid))) = (&section_id, &push_client) {
                if article_count > 0 {
                    let push_body = serde_json::json!({
                        "section_id": sid,
                        "data": articles,
                    });
                    match pc.post(format!("{base}/api/pages/{pid}/push"))
                        .json(&push_body)
                        .send()
                        .await
                    {
                        Ok(resp) if resp.status().is_success() => Some(PushStatus::Ok),
                        Ok(resp) => {
                            let status = resp.status();
                            let body = resp.text().await.unwrap_or_default();
                            Some(PushStatus::Failed(format!("{status}: {body}")))
                        }
                        Err(e) => Some(PushStatus::Failed(format!("{e}"))),
                    }
                } else {
                    Some(PushStatus::Ok)
                }
            } else {
                None
            };

            ScrapeOutcome::Ok(ScrapeResult {
                url,
                page_title,
                articles,
                quality,
                push_status,
            })
        }));
    }

    let mut outcomes = Vec::new();
    let mut panic_idx = 0usize;
    for handle in handles {
        match handle.await {
            Ok(outcome) => outcomes.push(outcome),
            Err(e) => {
                outcomes.push(ScrapeOutcome::Panic(format!("task panic #{panic_idx}: {e}")));
                panic_idx += 1;
            }
        }
    }

    for tab in &tabs {
        let _ = sup.close_ephemeral_tab(tab).await;
    }

    Ok(outcomes)
}

impl VScreenMcpServer {
    pub(super) fn synthesis_base_url(&self) -> Result<String, McpError> {
        self.state
            .synthesis_url
            .clone()
            .ok_or_else(|| invalid_params(
                "synthesis server not running. Start vscreen with --synthesis to enable.".to_string(),
            ))
    }

    pub(super) fn synthesis_client(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_default()
    }

    async fn synthesis_manage_create(&self, params: SynthesisManageParam) -> Result<CallToolResult, McpError> {
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
        if let Some(ref instance_id) = params.navigate_instance {
            self.require_exclusive(instance_id)?;
            if let Ok(sup) = self.get_supervisor(instance_id) {
                let nav_url = url.clone();
                let _ = sup.navigate(&nav_url).await;
                let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
                loop {
                    let state = sup.evaluate_js("document.readyState").await;
                    if let Ok(v) = state {
                        if v.as_str().unwrap_or("") == "complete" { break; }
                    }
                    if tokio::time::Instant::now() >= deadline { break; }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                let pi = sup.get_page_info().await.unwrap_or(serde_json::Value::Null);
                let t = pi.get("title").and_then(|v| v.as_str()).unwrap_or("");
                if t.contains("Privacy error") || t.contains("not private") {
                    let _ = sup.evaluate_js("document.querySelector('#details-button')?.click()").await;
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    let _ = sup.evaluate_js("document.querySelector('#proceed-link')?.click()").await;
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
                nav_status = format!("\nNavigated instance '{instance_id}' to {nav_url}");
            }
        }

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Created synthesis page '{}'.\nID: {id}\nURL: {url}{nav_status}\n\nPage data:\n{text}",
            title
        ))]))
    }

    async fn synthesis_manage_push(&self, params: SynthesisManageParam) -> Result<CallToolResult, McpError> {
        let page_id = params.page_id.ok_or_else(|| invalid_params("action=push requires page_id"))?;
        let section_id = params.section_id.ok_or_else(|| invalid_params("action=push requires section_id"))?;
        let data = params.data.ok_or_else(|| invalid_params("action=push requires data"))?;
        let base_url = self.synthesis_base_url()?;
        let client = self.synthesis_client();

        let body = serde_json::json!({ "section_id": section_id, "data": data });

        let resp = client
            .post(format!("{base_url}/api/pages/{page_id}/push"))
            .json(&body)
            .send()
            .await
            .map_err(|e| internal_error(format!("synthesis request failed: {e}")))?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(internal_error(format!("synthesis push failed ({status}): {text}")));
        }

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Pushed data to section '{section_id}' of page '{page_id}'.\n\nSection data:\n{text}"
        ))]))
    }

    async fn synthesis_manage_update(&self, params: SynthesisManageParam) -> Result<CallToolResult, McpError> {
        let page_id = params.page_id.ok_or_else(|| invalid_params("action=update requires page_id"))?;
        let base_url = self.synthesis_base_url()?;
        let client = self.synthesis_client();

        let mut body = serde_json::Map::new();
        if let Some(title) = params.title {
            body.insert("title".into(), serde_json::Value::String(title));
        }
        if let Some(subtitle) = params.subtitle {
            body.insert("subtitle".into(), serde_json::Value::String(subtitle));
        }
        if let Some(theme) = params.theme {
            body.insert("theme".into(), serde_json::Value::String(theme));
        }
        if let Some(layout) = params.layout {
            body.insert("layout".into(), serde_json::Value::String(layout));
        }
        if let Some(sections) = params.sections {
            body.insert("sections".into(), sections);
        }

        let resp = client
            .put(format!("{base_url}/api/pages/{page_id}"))
            .json(&body)
            .send()
            .await
            .map_err(|e| internal_error(format!("synthesis request failed: {e}")))?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(internal_error(format!("synthesis update failed ({status}): {text}")));
        }

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Updated page '{page_id}'.\n\n{text}"
        ))]))
    }

    async fn synthesis_manage_delete(&self, params: SynthesisManageParam) -> Result<CallToolResult, McpError> {
        let page_id = params.page_id.ok_or_else(|| invalid_params("action=delete requires page_id"))?;
        let base_url = self.synthesis_base_url()?;
        let client = self.synthesis_client();

        let resp = client
            .delete(format!("{base_url}/api/pages/{page_id}"))
            .send()
            .await
            .map_err(|e| internal_error(format!("synthesis request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(internal_error(format!("synthesis delete failed ({status}): {text}")));
        }

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Deleted page '{page_id}'."
        ))]))
    }

    async fn synthesis_manage_list(&self) -> Result<CallToolResult, McpError> {
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

    async fn synthesis_manage_navigate(&self, params: SynthesisManageParam) -> Result<CallToolResult, McpError> {
        let instance_id = params.instance_id.ok_or_else(|| invalid_params("action=navigate requires instance_id"))?;
        let page_slug = params.page_slug.ok_or_else(|| invalid_params("action=navigate requires page_slug"))?;
        let base_url = self.synthesis_base_url()?;
        let url = format!("{base_url}/page/{page_slug}");

        self.require_exclusive(&instance_id)?;
        let sup = self.get_supervisor(&instance_id)?;

        sup.navigate(&url)
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut loaded = false;
        loop {
            let state = sup.evaluate_js("document.readyState").await;
            if let Ok(v) = state {
                if v.as_str().unwrap_or("") == "complete" {
                    loaded = true;
                    break;
                }
            }
            if tokio::time::Instant::now() >= deadline { break; }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        let page_info = sup.get_page_info().await.unwrap_or(serde_json::Value::Null);
        let title = page_info.get("title").and_then(|v| v.as_str()).unwrap_or("");
        if title.contains("Privacy error") || title.contains("not private") {
            let _ = sup.evaluate_js("document.querySelector('#details-button')?.click()").await;
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let _ = sup.evaluate_js("document.querySelector('#proceed-link')?.click()").await;
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;

            let deadline2 = tokio::time::Instant::now() + std::time::Duration::from_secs(8);
            loaded = false;
            loop {
                let state = sup.evaluate_js("document.readyState").await;
                if let Ok(v) = state {
                    if v.as_str().unwrap_or("") == "complete" {
                        loaded = true;
                        break;
                    }
                }
                if tokio::time::Instant::now() >= deadline2 { break; }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }

        let page_info = sup.get_page_info().await.unwrap_or(serde_json::Value::Null);
        let title = page_info.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let load_status = if loaded { "fully loaded" } else { "WARNING: page did not fully load within 10s timeout" };

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Navigated to synthesis page: {url}\nTitle: {title}\nStatus: {load_status}"
        ))]))
    }

    async fn synthesis_manage_save(&self, params: SynthesisManageParam) -> Result<CallToolResult, McpError> {
        let page_id = params.page_id.ok_or_else(|| invalid_params("action=save requires page_id"))?;
        let base_url = self.synthesis_base_url()?;
        let client = self.synthesis_client();

        let resp = client
            .post(format!("{base_url}/api/pages/{page_id}/save"))
            .send()
            .await
            .map_err(|e| internal_error(format!("synthesis request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(internal_error(format!("save failed ({status}): {text}")));
        }

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Saved page '{page_id}' to disk."
        ))]))
    }

    async fn synthesis_scrape_single(&self, params: SynthesisScrapeConsolidatedParam) -> Result<CallToolResult, McpError> {
        let url = params.url.ok_or_else(|| invalid_params("mode=single requires url"))?;
        self.require_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;

        sup.navigate(&url)
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(12);
        loop {
            let state = sup.evaluate_js("document.readyState").await;
            if let Ok(v) = state {
                if v.as_str().unwrap_or("") == "complete" { break; }
            }
            if tokio::time::Instant::now() >= deadline { break; }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        let _ = sup.evaluate_js("window.scrollTo(0, window.innerHeight * 3); void(0)").await;
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        let _ = sup.evaluate_js("window.scrollTo(0, 0); void(0)").await;
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;

        let limit = params.limit.unwrap_or(8);
        let source_raw = params.source_label.as_deref().unwrap_or("");
        let source = source_raw
            .replace('\\', "\\\\")
            .replace('\'', "\\'")
            .replace('\n', "\\n")
            .replace('\r', "\\r");

        let budget_ms = 12000;
        let js = include_str!("scraper.js")
            .replace("__LIMIT__", &limit.to_string())
            .replace("__SOURCE__", &source)
            .replace("__TIMEOUT_BUDGET_MS__", &budget_ms.to_string());

        let result = sup.evaluate_js_async(&js).await
            .map_err(|e| internal_error(format!("scrape JS failed: {e}")))?;

        let raw_str = result.as_str().unwrap_or("{}");
        let page_info = sup.get_page_info().await.unwrap_or(serde_json::Value::Null);
        let page_title = page_info.get("title").and_then(|v| v.as_str()).unwrap_or("");

        let parsed: serde_json::Value = serde_json::from_str(raw_str)
            .unwrap_or(serde_json::json!({"articles": [], "quality": {}}));

        let articles = parsed.get("articles").cloned()
            .unwrap_or(serde_json::Value::Array(vec![]));
        let quality = parsed.get("quality").cloned()
            .unwrap_or(serde_json::Value::Null);

        let found = quality.get("found").and_then(|v| v.as_u64()).unwrap_or(0);
        let returned = quality.get("returned").and_then(|v| v.as_u64()).unwrap_or(0);
        let with_images = quality.get("withImages").and_then(|v| v.as_u64()).unwrap_or(0);
        let unique_images = quality.get("uniqueImages").and_then(|v| v.as_u64()).unwrap_or(0);
        let with_desc = quality.get("withDescriptions").and_then(|v| v.as_u64()).unwrap_or(0);

        let found_note = if found > returned {
            format!(" (found {found}, limited to {returned})")
        } else {
            String::new()
        };

        let image_note = if unique_images < with_images {
            format!("{with_images}/{returned} with images ({unique_images} unique)")
        } else {
            format!("{with_images}/{returned} with images")
        };

        let articles_json = serde_json::to_string_pretty(&articles).unwrap_or_default();
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Scraped {returned} articles from {url}{found_note}\nPage title: {page_title}\nQuality: {image_note}, {with_desc}/{returned} with descriptions\n\nArticles:\n{articles_json}"
        ))]))
    }

    async fn synthesis_scrape_batch(&self, params: SynthesisScrapeConsolidatedParam) -> Result<CallToolResult, McpError> {
        let urls = params.urls.ok_or_else(|| invalid_params("mode=batch requires urls"))?;
        if urls.is_empty() {
            return Err(invalid_params("urls array must not be empty"));
        }
        if urls.len() > 8 {
            return Err(invalid_params("maximum 8 URLs per batch"));
        }

        self.require_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;

        let outcomes = run_parallel_scrape(&sup, &urls, None, None).await?;

        let mut results_map = serde_json::Map::new();
        let mut total_articles = 0u64;
        let mut total_with_images = 0u64;
        let mut total_with_desc = 0u64;
        let mut source_count = 0u64;

        for outcome in outcomes {
            match outcome {
                ScrapeOutcome::Ok(r) => {
                    source_count += 1;
                    let returned = r.quality.get("returned").and_then(|v| v.as_u64()).unwrap_or(0);
                    let with_images = r.quality.get("withImages").and_then(|v| v.as_u64()).unwrap_or(0);
                    let with_desc = r.quality.get("withDescriptions").and_then(|v| v.as_u64()).unwrap_or(0);
                    total_articles += returned;
                    total_with_images += with_images;
                    total_with_desc += with_desc;
                    results_map.insert(r.url, serde_json::json!({
                        "page_title": r.page_title,
                        "articles": r.articles,
                        "quality": r.quality,
                    }));
                }
                ScrapeOutcome::Err { url, error } => {
                    results_map.insert(url, serde_json::json!({ "error": error }));
                }
                ScrapeOutcome::Panic(msg) => {
                    results_map.insert(msg.clone(), serde_json::json!({ "error": msg }));
                }
            }
        }

        let output = serde_json::json!({
            "results": results_map,
            "summary": format!(
                "Scraped {total_articles} articles from {source_count} sites, {total_with_images} with images, {total_with_desc} with descriptions"
            ),
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&output).unwrap_or_default(),
        )]))
    }
}

#[tool_router(router = synthesis_tools, vis = "pub(crate)")]
impl VScreenMcpServer {
    #[tool(description = "Manage synthesis pages. Action (required): create — new page (title required, subtitle?, theme?, layout?, sections?, navigate_instance?); update — change page (page_id required, title?, subtitle?, theme?, layout?, sections?); delete — remove page (page_id required); list — list all pages (no extra args); push — append data to section (page_id, section_id, data required); save — persist page to disk (page_id required); navigate — open page in browser (instance_id, page_slug required). Requires --synthesis flag.")]
    pub(crate) async fn vscreen_synthesis_manage(
        &self,
        Parameters(params): Parameters<SynthesisManageParam>,
    ) -> Result<CallToolResult, McpError> {
        let action = params.action.to_lowercase();
        match action.as_str() {
            "create" => self.synthesis_manage_create(params).await,
            "update" => self.synthesis_manage_update(params).await,
            "delete" => self.synthesis_manage_delete(params).await,
            "list" => self.synthesis_manage_list().await,
            "push" => self.synthesis_manage_push(params).await,
            "save" => self.synthesis_manage_save(params).await,
            "navigate" => self.synthesis_manage_navigate(params).await,
            _ => Err(invalid_params(format!(
                "invalid action '{action}'; expected: create, update, delete, list, push, save, navigate"
            ))),
        }
    }

    #[tool(description = "Scrape structured article data from webpages. Mode: single (default) — one URL (instance_id, url required; limit?, source_label?); batch — multiple URLs in parallel (instance_id, urls required). Uses DOM heuristics: JSON-LD, <article>, heading+link patterns, OpenGraph. Returns JSON for synthesis pages.")]
    pub(crate) async fn vscreen_synthesis_scrape(
        &self,
        Parameters(params): Parameters<SynthesisScrapeConsolidatedParam>,
    ) -> Result<CallToolResult, McpError> {
        let mode = params.mode.to_lowercase();
        match mode.as_str() {
            "single" => self.synthesis_scrape_single(params).await,
            "batch" => self.synthesis_scrape_batch(params).await,
            _ => Err(invalid_params(format!(
                "invalid mode '{mode}'; expected: single, batch"
            ))),
        }
    }

    #[tool(description = "One-shot: scrape multiple URLs in parallel AND create a synthesis page from the results. Each URL becomes a section on the page. Combines vscreen_synthesis_scrape_batch + vscreen_synthesis_create into a single call. Uses progressive rendering: creates the page with empty sections first, navigates to it, then pushes articles as each source completes (page live-updates via SSE). Args: instance_id (required), title (required), urls (required, array of {url, limit?, source_label?}), subtitle?, theme? ('dark'|'light'), layout? ('grid'|'list'|'tabs'), component? (override auto-selection), navigate? (default true). Example: {\"instance_id\": \"dev\", \"title\": \"News Roundup\", \"subtitle\": \"March 2026\", \"urls\": [{\"url\": \"https://cnn.com\", \"limit\": 8, \"source_label\": \"CNN\"}, {\"url\": \"https://bbc.com/news\", \"source_label\": \"BBC\"}]}")]
    pub(crate) async fn vscreen_synthesis_scrape_and_create(
        &self,
        Parameters(params): Parameters<SynthesisScrapeAndCreateParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;
        let base_url = self.synthesis_base_url()?;
        let client = self.synthesis_client();

        if params.urls.is_empty() {
            return Err(invalid_params("urls array must not be empty"));
        }
        if params.urls.len() > 8 {
            return Err(invalid_params("maximum 8 URLs per batch"));
        }

        // Build section IDs from source labels
        let section_ids: Vec<String> = params.urls.iter().enumerate().map(|(i, entry)| {
            let label = entry.source_label.as_deref().unwrap_or("source");
            let slug: String = label.to_lowercase()
                .chars()
                .map(|c| if c.is_alphanumeric() { c } else { '-' })
                .collect();
            let slug = slug.trim_matches('-').to_string();
            if slug.is_empty() { format!("source-{i}") } else { slug }
        }).collect();

        let empty_sections: Vec<serde_json::Value> = params.urls.iter().enumerate().map(|(i, entry)| {
            serde_json::json!({
                "id": section_ids[i],
                "component": params.component.as_deref().unwrap_or("card-grid"),
                "title": entry.source_label.as_deref().unwrap_or("Source"),
                "data": []
            })
        }).collect();

        // 1. Create page with empty sections
        let page_body = serde_json::json!({
            "title": params.title,
            "subtitle": params.subtitle,
            "theme": params.theme.as_deref().unwrap_or("dark"),
            "layout": params.layout.as_deref().unwrap_or("grid"),
            "sections": empty_sections,
        });

        let resp = client
            .post(format!("{base_url}/api/pages"))
            .json(&page_body)
            .send()
            .await
            .map_err(|e| internal_error(format!("create page failed: {e}")))?;

        let status = resp.status();
        let page_text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(internal_error(format!("create page failed ({status}): {page_text}")));
        }

        let page: serde_json::Value = serde_json::from_str(&page_text)
            .map_err(|e| internal_error(format!("invalid page JSON: {e}")))?;
        let page_id = page.get("id").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
        let page_url = format!("{base_url}/page/{page_id}");

        // 2. Navigate to the page immediately (user sees skeleton)
        let should_nav = params.navigate.unwrap_or(true);
        if should_nav {
            let _ = sup.navigate(&page_url).await;
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                let state = sup.evaluate_js("document.readyState").await;
                if let Ok(v) = state {
                    if v.as_str().unwrap_or("") == "complete" { break; }
                }
                if tokio::time::Instant::now() >= deadline { break; }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            let pi = sup.get_page_info().await.unwrap_or(serde_json::Value::Null);
            let t = pi.get("title").and_then(|v| v.as_str()).unwrap_or("");
            if t.contains("Privacy error") || t.contains("not private") {
                let _ = sup.evaluate_js("document.querySelector('#details-button')?.click()").await;
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                let _ = sup.evaluate_js("document.querySelector('#proceed-link')?.click()").await;
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        }

        // 3. Scrape in parallel, pushing results to the page progressively
        let push_cfg = PushConfig {
            client: self.synthesis_client(),
            base_url: base_url.clone(),
            page_id: page_id.clone(),
        };
        let outcomes = run_parallel_scrape(&sup, &params.urls, Some(&section_ids), Some(&push_cfg)).await?;

        // 4. Build final sections with correct component types and PUT
        let mut summary_lines = Vec::new();
        let mut total_articles = 0u64;
        let mut final_sections = Vec::new();

        for (i, outcome) in outcomes.iter().enumerate() {
            let sid = &section_ids[i];
            let source_title = params.urls[i].source_label.as_deref().unwrap_or("Source");

            match outcome {
                ScrapeOutcome::Ok(r) => {
                    let article_count = r.articles.as_array().map(|a| a.len()).unwrap_or(0);
                    let component = params.component.as_deref()
                        .unwrap_or_else(|| pick_section_component(article_count));
                    let returned = r.quality.get("returned").and_then(|v| v.as_u64()).unwrap_or(0);
                    let with_images = r.quality.get("withImages").and_then(|v| v.as_u64()).unwrap_or(0);
                    let with_desc = r.quality.get("withDescriptions").and_then(|v| v.as_u64()).unwrap_or(0);
                    total_articles += returned;

                    let push_note = match &r.push_status {
                        Some(PushStatus::Ok) => ", pushed OK".to_string(),
                        Some(PushStatus::Failed(e)) => format!(", push FAILED: {e}"),
                        None => String::new(),
                    };
                    summary_lines.push(format!(
                        "  {sid}: {returned} articles, {with_images} with images, {with_desc} with descriptions{push_note} ({url})",
                        url = r.url,
                    ));

                    final_sections.push(serde_json::json!({
                        "id": sid,
                        "component": component,
                        "title": source_title,
                        "data": r.articles,
                    }));
                }
                ScrapeOutcome::Err { url, error } => {
                    summary_lines.push(format!("  {sid}: FAILED — {error} ({url})"));
                    final_sections.push(serde_json::json!({
                        "id": sid,
                        "component": "card-grid",
                        "title": source_title,
                        "data": [],
                    }));
                }
                ScrapeOutcome::Panic(msg) => {
                    summary_lines.push(format!("  {sid}: PANIC — {msg}"));
                    final_sections.push(serde_json::json!({
                        "id": sid,
                        "component": "card-grid",
                        "title": source_title,
                        "data": [],
                    }));
                }
            }
        }

        // PUT final sections with correct component types (triggers SSE update)
        let _ = client
            .put(format!("{base_url}/api/pages/{page_id}"))
            .json(&serde_json::json!({ "sections": final_sections }))
            .send()
            .await;

        // Reload the page so SvelteKit re-runs its load function with final data.
        // The SSE onopen refetch covers live-update cases, but the initial
        // navigation races with scraping, so a reload after completion ensures
        // the user sees the final state.
        if should_nav {
            let _ = sup.evaluate_js("location.reload()").await;
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                let state = sup.evaluate_js("document.readyState").await;
                if let Ok(v) = state {
                    if v.as_str().unwrap_or("") == "complete" { break; }
                }
                if tokio::time::Instant::now() >= deadline { break; }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }

        let nav_note = if should_nav {
            format!("\nNavigated to: {page_url}")
        } else {
            format!("\nURL: {page_url}")
        };

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Created page '{}' with {total_articles} total articles across {} sources.{nav_note}\nPage ID: {page_id}\n\nPer-source results:\n{}",
            params.title,
            params.urls.len(),
            summary_lines.join("\n"),
        ))]))
    }

}
