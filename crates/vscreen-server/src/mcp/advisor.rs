use std::collections::VecDeque;

// ---------------------------------------------------------------------------
// Tool Advisor — session-aware anti-pattern detection
// ---------------------------------------------------------------------------

pub(crate) struct ToolCallRecord {
    pub(crate) tool_name: String,
    pub(crate) args_snapshot: Option<serde_json::Value>,
}

pub(crate) struct ToolAdvisor {
    pub(crate) recent_calls: VecDeque<ToolCallRecord>,
    layer1_tip_shown: bool,
}

const LAYER1_TOOLS: &[&str] = &[
    "vscreen_browse",
    "vscreen_observe",
    "vscreen_interact",
    "vscreen_extract",
    "vscreen_synthesize",
    "vscreen_solve_challenge",
];

const CLICK_TOOLS: &[&str] = &[
    "vscreen_click",
    "vscreen_click_element",
    "vscreen_click_and_navigate",
    "vscreen_batch_click",
];

impl ToolAdvisor {
    pub(crate) fn new() -> Self {
        Self {
            recent_calls: VecDeque::with_capacity(24),
            layer1_tip_shown: false,
        }
    }

    /// Build a minimal args snapshot for pattern detection (avoids memory bloat).
    pub(crate) fn build_args_snapshot(tool_name: &str, args: &serde_json::Value) -> Option<serde_json::Value> {
        let obj = args.as_object()?;
        let mut snapshot = serde_json::Map::new();
        match tool_name {
            "vscreen_wait" => {
                if let Some(v) = obj.get("condition") {
                    snapshot.insert("condition".into(), v.clone());
                }
            }
            "vscreen_execute_js" => {
                if let Some(v) = obj.get("expression") {
                    snapshot.insert("expression".into(), v.clone());
                }
            }
            "vscreen_screenshot" => {
                if let Some(v) = obj.get("clip") {
                    snapshot.insert("clip".into(), v.clone());
                }
            }
            _ => {}
        }
        if snapshot.is_empty() && tool_name == "vscreen_wait" {
            // duration_ms without condition implies condition="duration"
            if obj.get("duration_ms").is_some() {
                snapshot.insert("condition".into(), serde_json::json!("duration"));
            }
        }
        if snapshot.is_empty() {
            None
        } else {
            Some(serde_json::Value::Object(snapshot))
        }
    }

    pub(crate) fn record(&mut self, record: ToolCallRecord) {
        if self.recent_calls.len() >= 20 {
            self.recent_calls.pop_front();
        }
        self.recent_calls.push_back(record);
    }

    pub(crate) fn check_anti_patterns(&mut self, tool_name: &str, args: &serde_json::Value) -> Option<String> {
        // 1. Tool-specific anti-patterns (more actionable)
        let pattern_hint = match tool_name {
            "vscreen_screenshot" => {
                self.check_click_wait_screenshot_pattern(tool_name)
                    .or_else(|| self.check_screenshot_patterns(args))
                    .or_else(|| self.check_repeated_iframe_screenshots())
            }
            "vscreen_scroll" => self.check_scroll_patterns(),
            "vscreen_wait" => self
                .check_click_wait_screenshot_pattern(tool_name)
                .or_else(|| self.check_wait_patterns(tool_name, args)),
            "vscreen_execute_js" => self.check_js_patterns(args),
            "vscreen_click" | "vscreen_click_element" | "vscreen_click_and_navigate" | "vscreen_batch_click" => {
                self.check_click_wait_screenshot_pattern(tool_name)
            }
            _ => None,
        };

        // 2. Layer-1 usage tracking & promotion (one-time tip, fallback)
        pattern_hint.or_else(|| self.check_layer1_promotion(tool_name))
    }

    fn check_layer1_promotion(&mut self, tool_name: &str) -> Option<String> {
        if self.layer1_tip_shown {
            return None;
        }
        let total = self.recent_calls.len();
        let used_layer1 = self.recent_calls.iter().any(|r| LAYER1_TOOLS.contains(&r.tool_name.as_str()));
        if used_layer1 {
            return None;
        }
        if total >= 5 && !LAYER1_TOOLS.contains(&tool_name) {
            self.layer1_tip_shown = true;
            return Some(
                "Tip: vscreen_browse, vscreen_interact, and vscreen_observe handle most tasks in a single call. \
                 Try these before using individual tools."
                    .to_string(),
            );
        }
        None
    }

    fn check_click_wait_screenshot_pattern(&self, _current_tool: &str) -> Option<String> {
        let recent: Vec<&ToolCallRecord> = self.recent_calls.iter().rev().take(4).collect();
        if recent.len() < 4 {
            return None;
        }
        let has_click = recent.iter().any(|r| CLICK_TOOLS.contains(&r.tool_name.as_str()));
        let has_wait = recent.iter().any(|r| r.tool_name == "vscreen_wait");
        let has_screenshot = recent.iter().any(|r| r.tool_name == "vscreen_screenshot");
        if has_click && has_wait && has_screenshot {
            return Some(
                "Advisor: vscreen_interact performs click + wait + screenshot in one call.".to_string(),
            );
        }
        None
    }

    fn check_repeated_iframe_screenshots(&self) -> Option<String> {
        let recent: Vec<&ToolCallRecord> = self.recent_calls.iter().rev().take(6).collect();
        let screenshot_count = recent
            .iter()
            .filter(|r| r.tool_name == "vscreen_screenshot")
            .filter(|r| {
                let clip = r.args_snapshot.as_ref().and_then(|a| a.get("clip"));
                if let Some(clip) = clip {
                    let w = clip.get("width").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let h = clip.get("height").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    w < 200.0 || h < 200.0
                } else {
                    false
                }
            })
            .count();
        if screenshot_count >= 3 {
            return Some(
                "Advisor: Multiple clipped screenshots on small regions may indicate a CAPTCHA grid. \
                 Use vscreen_solve_captcha for reCAPTCHA challenges."
                    .to_string(),
            );
        }
        None
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
            .any(|r| r.tool_name == "vscreen_find");
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

    fn check_wait_patterns(&self, _tool_name: &str, args: &serde_json::Value) -> Option<String> {
        let condition = match args.get("condition").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => {
                if args.get("duration_ms").is_some() {
                    "duration"
                } else {
                    return None;
                }
            }
        };
        if condition != "duration" {
            return None;
        }
        let recent: Vec<&ToolCallRecord> = self.recent_calls.iter().rev().take(6).collect();
        let duration_wait_count = recent
            .iter()
            .filter(|r| r.tool_name == "vscreen_wait")
            .filter(|r| {
                let c = r
                    .args_snapshot
                    .as_ref()
                    .and_then(|a| a.get("condition"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("duration");
                c == "duration"
            })
            .count();
        if duration_wait_count >= 2 {
            return Some(
                "Advisor: You have used multiple fixed-duration waits. Consider using \
                 vscreen_wait(condition=\"text\", text=...), vscreen_wait(condition=\"selector\", selector=...), \
                 or vscreen_wait(condition=\"network\") instead of fixed waits."
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
        if expr_lower.contains("fetch") && expr_lower.contains("/api/pages") {
            return Some(
                "Advisor: Use vscreen_synthesize or vscreen_synthesis_manage instead of fetch() \
                 for synthesis API calls."
                    .to_string(),
            );
        }
        None
    }
}
