use base64::Engine;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::{tool, tool_router};

use super::params::*;
use super::{internal_error, invalid_params, McpError, VScreenMcpServer};

#[tool_router(router = observation_tools, vis = "pub(crate)")]
impl VScreenMcpServer {
    #[tool(description = "Capture a screenshot. Returns PNG by default. Options: full_page=true for entire document, clip={x,y,width,height} for a region, annotate=true to overlay numbered bounding boxes on interactive elements, sequence_count+sequence_interval_ms for multi-frame capture. ALWAYS use full_page=true to see the entire page. PREFER vscreen_extract_text when you only need text. Example: {\"instance_id\": \"dev\", \"full_page\": true} or {\"instance_id\": \"dev\", \"annotate\": true}")]
    async fn vscreen_screenshot(
        &self,
        Parameters(params): Parameters<ScreenshotParam>,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;

        // Sequence mode: both count and interval must be set
        if let (Some(count), Some(interval_ms)) = (params.sequence_count, params.sequence_interval_ms) {
            return self.do_screenshot_sequence(&sup, count, interval_ms, &params.format, params.quality).await;
        }

        // Annotate mode
        if params.annotate {
            let selector = params.annotate_selector.as_deref();
            return self.do_screenshot_annotated(&sup, selector).await;
        }

        // Normal screenshot
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

    async fn do_screenshot_sequence(
        &self,
        sup: &std::sync::Arc<crate::supervisor::InstanceSupervisor>,
        count: u32,
        interval_ms: u64,
        format: &str,
        quality: Option<u32>,
    ) -> Result<CallToolResult, McpError> {
        let images = sup
            .capture_screenshot_sequence(count, interval_ms, format, quality)
            .await
            .map_err(|e| internal_error(e.to_string()))?;

        let mime = match format {
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

    async fn do_screenshot_annotated(
        &self,
        sup: &std::sync::Arc<crate::supervisor::InstanceSupervisor>,
        selector_opt: Option<&str>,
    ) -> Result<CallToolResult, McpError> {
        let selector = selector_opt.unwrap_or(
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

    #[tool(description = "Find elements on the page. Modes: by='selector' (CSS query), by='text' (visible text search), by='input' (find form inputs). Returns element metadata with bounding boxes in page coordinates (use directly with vscreen_click). Set include_iframes=true to search inside iframes. For by='input': use placeholder, aria_label, label, role, name, or input_type to filter. Example: {\"instance_id\": \"dev\", \"by\": \"text\", \"text\": \"Sign In\"} or {\"instance_id\": \"dev\", \"by\": \"selector\", \"selector\": \"button.submit\"}")]
    async fn vscreen_find(
        &self,
        Parameters(params): Parameters<ConsolidatedFindParam>,
    ) -> Result<CallToolResult, McpError> {
        let by = params.by.to_lowercase();
        match by.as_str() {
            "selector" => {
                let selector = params.selector.as_deref().ok_or_else(|| {
                    invalid_params("by='selector' requires selector parameter")
                })?;
                self.do_find_elements(&params.instance_id, selector, params.include_iframes).await
            }
            "text" => {
                let text = params.text.as_deref().ok_or_else(|| {
                    invalid_params("by='text' requires text parameter")
                })?;
                self.do_find_by_text(&params.instance_id, text, params.exact, params.include_iframes).await
            }
            "input" => {
                if params.placeholder.is_none()
                    && params.aria_label.is_none()
                    && params.label.is_none()
                    && params.role.is_none()
                    && params.name.is_none()
                    && params.input_type.is_none()
                {
                    return Err(invalid_params(
                        "by='input' requires at least one of: placeholder, aria_label, label, role, name, input_type",
                    ));
                }
                self.do_find_input(&params).await
            }
            _ => Err(invalid_params(format!(
                "by must be 'selector', 'text', or 'input', got '{by}'"
            ))),
        }
    }

    async fn do_find_elements(
        &self,
        instance_id: &str,
        selector: &str,
        include_iframes: bool,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(instance_id)?;
        let sup = self.get_supervisor(instance_id)?;
        let sel_json = serde_json::to_string(selector).unwrap_or_default();
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

        if include_iframes {
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

    async fn do_find_by_text(
        &self,
        instance_id: &str,
        text: &str,
        exact: bool,
        include_iframes: bool,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(instance_id)?;
        let sup = self.get_supervisor(instance_id)?;
        let search = serde_json::to_string(text).unwrap_or_default();
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

        if include_iframes {
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

    async fn do_find_input(
        &self,
        params: &ConsolidatedFindParam,
    ) -> Result<CallToolResult, McpError> {
        self.require_observer_or_exclusive(&params.instance_id)?;
        let sup = self.get_supervisor(&params.instance_id)?;

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
                "No matching input elements found. Try broader search criteria, or use vscreen_find(by='selector', selector='...') instead.",
            )]))
        } else {
            let text = serde_json::to_string_pretty(&elements).unwrap_or_default();
            Ok(CallToolResult::success(vec![Content::text(format!(
                "Found {} input(s):\n{text}\n\nTip: use vscreen_fill(selector, value) to type into a match.",
                elements.len()
            ))]))
        }
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

    #[tool(description = "List all frames (including iframes) in the page. Returns frame IDs, URLs, names, and bounding rectangles. Use this to understand page structure before interacting with iframe content. Each iframe's bounding rect gives page-space coordinates that can be used with vscreen_screenshot(clip=...) to zoom in. Related: vscreen_find(by='selector' or 'text', include_iframes=true).")]
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

    #[tool(description = "Execute JavaScript in the MAIN FRAME only (cannot access iframe content). The 'expression' is evaluated via CDP Runtime.evaluate and returns the result as JSON. Useful for custom DOM manipulation not covered by existing tools. For iframe content, use vscreen_find(by='selector', include_iframes=true) instead. PREFER vscreen_get_page_info for page metadata (title, URL, viewport). PREFER vscreen_extract_text for reading page content. Only use execute_js for custom queries.")]
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
}
