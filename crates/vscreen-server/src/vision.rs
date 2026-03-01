use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Which API wire format to use when talking to the vision LLM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiFormat {
    /// Ollama native `/api/chat` (newline-delimited JSON streaming).
    Ollama,
    /// OpenAI-compatible `/v1/chat/completions` (SSE streaming).
    OpenAi,
}

/// Configuration for the external vision LLM.
#[derive(Debug, Clone)]
pub struct VisionConfig {
    /// Base URL, e.g. `http://spark.ms.sswt.org:11434`.
    pub url: String,
    /// Model name, e.g. `qwen3-vl:8b`.
    pub model: String,
    /// Wire format — auto-detected on first use if not set explicitly.
    pub api_format: Option<ApiFormat>,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Structured response from the vision model.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VisionResponse {
    /// Raw text returned by the model.
    pub text: String,
    /// Parsed click target, if the model identified one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub click_target: Option<ClickTarget>,
    /// Whether the model found what was asked about.
    pub found: bool,
}

/// A click target identified by the vision model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClickTarget {
    pub x: f64,
    pub y: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
}

/// Errors from the vision subsystem.
#[derive(Debug, thiserror::Error)]
pub enum VisionError {
    #[error("vision server unreachable: {0}")]
    Unreachable(String),
    #[error("vision request failed: {0}")]
    RequestFailed(String),
    #[error("vision response parse error: {0}")]
    ParseError(String),
}

// ---------------------------------------------------------------------------
// Ollama / OpenAI response fragments (for streaming)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct OllamaChatChunk {
    message: Option<OllamaMessage>,
    done: Option<bool>,
}

#[derive(Deserialize)]
struct OllamaMessage {
    content: Option<String>,
    thinking: Option<String>,
}

#[derive(Deserialize)]
struct OpenAiChunk {
    choices: Option<Vec<OpenAiChoice>>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    delta: Option<OpenAiDelta>,
}

#[derive(Deserialize)]
struct OpenAiDelta {
    content: Option<String>,
}

// ---------------------------------------------------------------------------
// VisionClient
// ---------------------------------------------------------------------------

/// Client for an external vision LLM (Ollama or OpenAI-compatible).
///
/// Lazily detects the API format on first call. Caches reachability to avoid
/// repeated timeouts when the server is down.
#[derive(Debug)]
pub struct VisionClient {
    config: VisionConfig,
    client: reqwest::Client,
    /// Cached API format after auto-detection.
    detected_format: RwLock<Option<ApiFormat>>,
    /// Cached availability (reset periodically).
    available: RwLock<Option<(bool, tokio::time::Instant)>>,
}

const AVAILABILITY_CACHE_SECS: u64 = 30;
const VISION_TIMEOUT_SECS: u64 = 180;
const CONNECT_TIMEOUT_SECS: u64 = 5;

impl VisionClient {
    /// Create a new vision client from configuration.
    #[must_use]
    pub fn new(config: VisionConfig) -> Arc<Self> {
        // No overall timeout on the client — vision model inference can take
        // minutes for large screenshots.  Per-request timeouts are set via
        // `.timeout()` on individual request builders instead.
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .build()
            .unwrap_or_default();

        info!(
            url = %config.url,
            model = %config.model,
            "vision client created"
        );

        Arc::new(Self {
            config,
            client,
            detected_format: RwLock::new(None),
            available: RwLock::new(None),
        })
    }

    /// Check whether the vision server is reachable (cached for 30s).
    pub async fn is_available(&self) -> bool {
        {
            let cache = self.available.read().await;
            if let Some((result, when)) = cache.as_ref() {
                if when.elapsed() < Duration::from_secs(AVAILABILITY_CACHE_SECS) {
                    return *result;
                }
            }
        }

        let result = self.probe_availability().await;
        {
            let mut cache = self.available.write().await;
            *cache = Some((result, tokio::time::Instant::now()));
        }
        result
    }

    /// Analyse a screenshot with a text prompt. Returns the model's response.
    pub async fn analyze(
        &self,
        image_png: &[u8],
        prompt: &str,
    ) -> Result<VisionResponse, VisionError> {
        let format = self.resolve_format().await?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(image_png);

        let full_text = match format {
            ApiFormat::Ollama => self.call_ollama(&b64, prompt).await?,
            ApiFormat::OpenAi => self.call_openai(&b64, prompt).await?,
        };

        debug!(response_len = full_text.len(), "vision model responded");
        Self::parse_response(&full_text)
    }

    /// Like [`analyze`] but with a caller-specified timeout (e.g. 45s for captcha).
    pub async fn analyze_with_timeout(
        &self,
        image_png: &[u8],
        prompt: &str,
        timeout: Duration,
    ) -> Result<VisionResponse, VisionError> {
        match tokio::time::timeout(timeout, self.analyze(image_png, prompt)).await {
            Ok(result) => result,
            Err(_) => Err(VisionError::RequestFailed(format!(
                "vision analysis timed out after {}s",
                timeout.as_secs()
            ))),
        }
    }

    /// Fast analysis with early JSON exit and timeout for latency-sensitive
    /// use cases like CAPTCHA solving.
    pub async fn analyze_fast(
        &self,
        image_png: &[u8],
        prompt: &str,
        timeout: Duration,
    ) -> Result<VisionResponse, VisionError> {
        let fut = async {
            let format = self.resolve_format().await?;
            let b64 = base64::engine::general_purpose::STANDARD.encode(image_png);

            let full_text = match format {
                ApiFormat::Ollama => self.call_ollama_fast(&b64, prompt).await?,
                ApiFormat::OpenAi => self.call_openai(&b64, prompt).await?,
            };

            debug!(response_len = full_text.len(), "vision fast model responded");
            Self::parse_response(&full_text)
        };

        match tokio::time::timeout(timeout, fut).await {
            Ok(result) => result,
            Err(_) => Err(VisionError::RequestFailed(format!(
                "vision fast analysis timed out after {}s",
                timeout.as_secs()
            ))),
        }
    }

    // -----------------------------------------------------------------------
    // Internal: API format detection
    // -----------------------------------------------------------------------

    async fn resolve_format(&self) -> Result<ApiFormat, VisionError> {
        // Check explicit or cached format first.
        if let Some(ref fmt) = self.config.api_format {
            return Ok(fmt.clone());
        }
        {
            let cached = self.detected_format.read().await;
            if let Some(ref fmt) = *cached {
                return Ok(fmt.clone());
            }
        }

        let fmt = self.detect_format().await?;
        {
            let mut cached = self.detected_format.write().await;
            *cached = Some(fmt.clone());
        }
        Ok(fmt)
    }

    async fn detect_format(&self) -> Result<ApiFormat, VisionError> {
        let url = &self.config.url;

        // URL-based heuristic
        if url.contains("/v1/") || url.ends_with("/chat/completions") {
            info!(url, "detected OpenAI-compatible API from URL pattern");
            return Ok(ApiFormat::OpenAi);
        }

        // Probe Ollama's /api/tags endpoint
        let tags_url = format!("{}/api/tags", url.trim_end_matches('/'));
        match self.client.get(&tags_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                info!(url, "detected Ollama API (tags endpoint responded)");
                Ok(ApiFormat::Ollama)
            }
            _ => {
                // Probe OpenAI-compatible /v1/models
                let models_url = format!("{}/v1/models", url.trim_end_matches('/'));
                match self.client.get(&models_url).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        info!(url, "detected OpenAI-compatible API (models endpoint responded)");
                        Ok(ApiFormat::OpenAi)
                    }
                    _ => {
                        // Default to Ollama since that's the primary use case
                        warn!(url, "could not auto-detect API format, defaulting to Ollama");
                        Ok(ApiFormat::Ollama)
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Internal: Ollama native API
    // -----------------------------------------------------------------------

    async fn call_ollama(&self, image_b64: &str, prompt: &str) -> Result<String, VisionError> {
        let url = format!("{}/api/chat", self.config.url.trim_end_matches('/'));

        // Do NOT set "think": false — on VL models it's unsupported and
        // actually reduces the content token budget.  The thinking field is
        // separate from content and we simply ignore it.
        // Do NOT set num_predict — it limits thinking + content combined,
        // and VL models can spend 500+ tokens thinking before producing content.
        let body = serde_json::json!({
            "model": self.config.model,
            "messages": [
                {
                    "role": "system",
                    "content": "You are a visual analysis assistant. Respond with the requested JSON object."
                },
                {
                    "role": "user",
                    "content": prompt,
                    "images": [image_b64]
                }
            ],
            "format": "json",
            "stream": true,
            "options": {
                "temperature": 0.1
            }
        });

        let resp = self
            .client
            .post(&url)
            .timeout(Duration::from_secs(VISION_TIMEOUT_SECS))
            .json(&body)
            .send()
            .await
            .map_err(|e| VisionError::RequestFailed(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(VisionError::RequestFailed(format!(
                "HTTP {status}: {text}"
            )));
        }

        // Stream newline-delimited JSON chunks.  Only collect msg.content;
        // msg.thinking is produced by VL models but is separate and ignored.
        let mut stream = resp.bytes_stream();
        let mut full_text = String::new();
        let mut buf = String::new();

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result
                .map_err(|e| VisionError::RequestFailed(e.to_string()))?;
            buf.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(newline_pos) = buf.find('\n') {
                let line = buf[..newline_pos].trim().to_string();
                buf = buf[newline_pos + 1..].to_string();

                if line.is_empty() {
                    continue;
                }

                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) {
                    if let Some(err) = val.get("error").and_then(|e| e.as_str()) {
                        return Err(VisionError::RequestFailed(format!("Ollama error: {err}")));
                    }
                }

                if let Ok(chunk) = serde_json::from_str::<OllamaChatChunk>(&line) {
                    if let Some(msg) = chunk.message {
                        if let Some(ref content) = msg.content {
                            full_text.push_str(content);
                        }
                        // msg.thinking is intentionally ignored
                    }
                    if chunk.done == Some(true) {
                        return Ok(full_text);
                    }
                }
            }
        }

        Ok(full_text)
    }

    /// Ollama call with early JSON extraction. Stops streaming as soon as a
    /// complete JSON object is found in the content field.
    /// Like `call_ollama` but with early-exit for latency-sensitive use cases.
    async fn call_ollama_fast(&self, image_b64: &str, prompt: &str) -> Result<String, VisionError> {
        let url = format!("{}/api/chat", self.config.url.trim_end_matches('/'));

        // No num_predict limit. qwen3-vl thinking tokens + content tokens
        // share the budget, and capping it causes content_len=0 when the model
        // spends 1400+ tokens thinking. The early JSON exit stops streaming
        // as soon as valid JSON arrives in content, keeping wall-clock time
        // bounded regardless of thinking length.
        let body = serde_json::json!({
            "model": self.config.model,
            "messages": [
                {
                    "role": "system",
                    "content": "You are a visual analysis assistant. Respond with the requested JSON object."
                },
                {
                    "role": "user",
                    "content": prompt,
                    "images": [image_b64]
                }
            ],
            "format": "json",
            "stream": true,
            "options": {
                "temperature": 0.1
            }
        });

        let resp = self
            .client
            .post(&url)
            .timeout(Duration::from_secs(120))
            .json(&body)
            .send()
            .await
            .map_err(|e| VisionError::RequestFailed(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(VisionError::RequestFailed(format!("HTTP {status}: {text}")));
        }

        let mut stream = resp.bytes_stream();
        let mut full_text = String::new();
        let mut buf = String::new();
        let stream_start = std::time::Instant::now();
        let mut first_content_at: Option<std::time::Instant> = None;
        let mut thinking_tokens = 0u32;
        let mut content_tokens = 0u32;

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result
                .map_err(|e| VisionError::RequestFailed(e.to_string()))?;
            buf.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(newline_pos) = buf.find('\n') {
                let line = buf[..newline_pos].trim().to_string();
                buf = buf[newline_pos + 1..].to_string();

                if line.is_empty() {
                    continue;
                }

                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) {
                    if let Some(err) = val.get("error").and_then(|e| e.as_str()) {
                        return Err(VisionError::RequestFailed(format!("Ollama error: {err}")));
                    }
                }

                if let Ok(chunk) = serde_json::from_str::<OllamaChatChunk>(&line) {
                    if let Some(msg) = chunk.message {
                        if let Some(ref thinking) = msg.thinking {
                            if !thinking.is_empty() {
                                thinking_tokens += 1;
                            }
                        }
                        if let Some(ref content) = msg.content {
                            if !content.is_empty() {
                                if first_content_at.is_none() {
                                    first_content_at = Some(std::time::Instant::now());
                                    info!(
                                        thinking_ms = stream_start.elapsed().as_millis() as u64,
                                        thinking_tokens,
                                        "vision fast: first content token"
                                    );
                                }
                                content_tokens += 1;
                            }
                            full_text.push_str(content);
                        }
                    }

                    // Bail if the model is stuck thinking and hasn't produced
                    // any content after 25 seconds of streaming
                    if full_text.is_empty()
                        && thinking_tokens > 500
                        && stream_start.elapsed() > Duration::from_secs(25)
                    {
                        warn!(
                            thinking_tokens,
                            elapsed_ms = stream_start.elapsed().as_millis() as u64,
                            "vision fast: aborting — too long in thinking phase"
                        );
                        return Err(VisionError::RequestFailed(
                            "model stuck in thinking phase, no content produced".into(),
                        ));
                    }

                    if !full_text.is_empty() && Self::extract_json(&full_text).is_some() {
                        info!(
                            content_len = full_text.len(),
                            content_tokens,
                            thinking_tokens,
                            total_ms = stream_start.elapsed().as_millis() as u64,
                            "early JSON exit in fast mode"
                        );
                        return Ok(full_text);
                    }

                    if chunk.done == Some(true) {
                        info!(
                            content_len = full_text.len(),
                            content_tokens,
                            thinking_tokens,
                            total_ms = stream_start.elapsed().as_millis() as u64,
                            "vision fast: stream done"
                        );
                        return Ok(full_text);
                    }
                }
            }
        }

        info!(
            content_len = full_text.len(),
            content_tokens,
            thinking_tokens,
            total_ms = stream_start.elapsed().as_millis() as u64,
            "vision fast: stream ended"
        );
        Ok(full_text)
    }

    // -----------------------------------------------------------------------
    // Internal: OpenAI-compatible API
    // -----------------------------------------------------------------------

    async fn call_openai(&self, image_b64: &str, prompt: &str) -> Result<String, VisionError> {
        let base = self.config.url.trim_end_matches('/');
        let url = if base.ends_with("/chat/completions") {
            base.to_string()
        } else if base.ends_with("/v1") {
            format!("{base}/chat/completions")
        } else {
            format!("{base}/v1/chat/completions")
        };

        let body = serde_json::json!({
            "model": self.config.model,
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": prompt},
                    {"type": "image_url", "image_url": {
                        "url": format!("data:image/png;base64,{image_b64}")
                    }}
                ]
            }],
            "stream": true,
            "temperature": 0.1,
            "max_tokens": 256
        });

        let resp = self
            .client
            .post(&url)
            .timeout(Duration::from_secs(VISION_TIMEOUT_SECS))
            .json(&body)
            .send()
            .await
            .map_err(|e| VisionError::RequestFailed(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(VisionError::RequestFailed(format!(
                "HTTP {status}: {text}"
            )));
        }

        // Stream SSE chunks
        let mut stream = resp.bytes_stream();
        let mut full_text = String::new();
        let mut buf = String::new();

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result
                .map_err(|e| VisionError::RequestFailed(e.to_string()))?;
            buf.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(newline_pos) = buf.find('\n') {
                let line = buf[..newline_pos].trim().to_string();
                buf = buf[newline_pos + 1..].to_string();

                if line.is_empty() || line == "data: [DONE]" {
                    continue;
                }

                let json_str = line.strip_prefix("data: ").unwrap_or(&line);
                if let Ok(chunk) = serde_json::from_str::<OpenAiChunk>(json_str) {
                    if let Some(choices) = chunk.choices {
                        for choice in choices {
                            if let Some(delta) = choice.delta {
                                if let Some(content) = delta.content {
                                    full_text.push_str(&content);
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(full_text)
    }

    // -----------------------------------------------------------------------
    // Internal: availability probe
    // -----------------------------------------------------------------------

    async fn probe_availability(&self) -> bool {
        let base = self.config.url.trim_end_matches('/');

        // Try Ollama tags endpoint first (fast health check)
        let tags_url = format!("{base}/api/tags");
        if let Ok(resp) = self
            .client
            .get(&tags_url)
            .timeout(Duration::from_secs(3))
            .send()
            .await
        {
            if resp.status().is_success() {
                return true;
            }
        }

        // Try OpenAI models endpoint
        let models_url = format!("{base}/v1/models");
        if let Ok(resp) = self
            .client
            .get(&models_url)
            .timeout(Duration::from_secs(3))
            .send()
            .await
        {
            if resp.status().is_success() {
                return true;
            }
        }

        // Try a bare GET to the base URL (some servers respond to /)
        if let Ok(resp) = self
            .client
            .get(base)
            .timeout(Duration::from_secs(3))
            .send()
            .await
        {
            if resp.status().is_success() {
                return true;
            }
        }

        warn!(url = %self.config.url, "vision server is not reachable");
        false
    }

    // -----------------------------------------------------------------------
    // Internal: parse model output into structured VisionResponse
    // -----------------------------------------------------------------------

    fn parse_response(text: &str) -> Result<VisionResponse, VisionError> {
        // The model may return JSON directly or embed it in prose.
        // Try to extract JSON from the response.
        if let Some(json_str) = Self::extract_json(text) {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&json_str) {
                let found = parsed
                    .get("found")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);

                let click_target = if let (Some(x), Some(y)) = (
                    parsed.get("x").and_then(|v| v.as_f64()),
                    parsed.get("y").and_then(|v| v.as_f64()),
                ) {
                    Some(ClickTarget {
                        x,
                        y,
                        description: parsed
                            .get("description")
                            .and_then(|v| v.as_str())
                            .map(String::from),
                        confidence: parsed.get("confidence").and_then(|v| v.as_f64()),
                    })
                } else {
                    None
                };

                return Ok(VisionResponse {
                    text: text.to_string(),
                    click_target,
                    found,
                });
            }
        }

        // If no JSON found, treat entire text as a natural-language response.
        let found = !text.to_lowercase().contains("not found")
            && !text.to_lowercase().contains("no ")
            && !text.to_lowercase().contains("\"found\": false")
            && !text.to_lowercase().contains("found: false");

        Ok(VisionResponse {
            text: text.to_string(),
            click_target: None,
            found,
        })
    }

    /// Extract the first JSON object `{...}` from possibly surrounding prose.
    pub fn extract_json(text: &str) -> Option<String> {
        // Try a markdown code block first: ```json ... ```
        if let Some(start) = text.find("```json") {
            let after = &text[start + 7..];
            if let Some(end) = after.find("```") {
                return Some(after[..end].trim().to_string());
            }
        }
        if let Some(start) = text.find("```") {
            let after = &text[start + 3..];
            if let Some(end) = after.find("```") {
                let block = after[..end].trim();
                if block.starts_with('{') {
                    return Some(block.to_string());
                }
            }
        }

        // Find first { and matching }, skipping braces inside JSON strings.
        let mut depth = 0i32;
        let mut start = None;
        let mut in_string = false;
        let mut escape = false;
        for (i, ch) in text.char_indices() {
            if escape {
                escape = false;
                continue;
            }
            if in_string {
                match ch {
                    '\\' => escape = true,
                    '"' => in_string = false,
                    _ => {}
                }
                continue;
            }
            match ch {
                '"' if depth > 0 => in_string = true,
                '{' => {
                    if depth == 0 {
                        start = Some(i);
                    }
                    depth += 1;
                }
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        if let Some(s) = start {
                            return Some(text[s..=i].to_string());
                        }
                    }
                }
                _ => {}
            }
        }

        None
    }
}

// ---------------------------------------------------------------------------
// Predefined prompts for tool-specific vision queries
// ---------------------------------------------------------------------------

pub mod prompts {
    /// Prompt for detecting and locating cookie consent / privacy dialogs.
    pub const DISMISS_DIALOG: &str = "\
You are analyzing a browser screenshot. Is there a cookie consent, privacy, or GDPR dialog overlay visible on this page?

If YES: Return a JSON object with the center coordinates of the \"Accept All\", \"Accept\", \"I Agree\", or similar consent button:
{\"found\": true, \"x\": <number>, \"y\": <number>, \"description\": \"<button text>\"}

If NO consent/cookie dialog is visible: Return:
{\"found\": false}

IMPORTANT: Only identify actual consent/cookie/privacy overlays. Do NOT identify navigation menus, video players, ads, or other page elements. The coordinates should be the CENTER of the button in pixels from the top-left of the screenshot.";

    /// Prompt for detecting and locating video ad skip buttons.
    pub const DISMISS_AD: &str = "\
You are analyzing a browser screenshot of a video player page. Is there a video advertisement playing with a \"Skip\" or \"Skip Ad\" button visible?

If YES: Return a JSON object with the center coordinates of the skip button:
{\"found\": true, \"x\": <number>, \"y\": <number>, \"description\": \"Skip Ad button\"}

If NO skip button is visible but an ad appears to be playing (e.g. countdown timer visible):
{\"found\": false, \"description\": \"Ad playing but skip not yet available\"}

If NO ad is playing:
{\"found\": false}

IMPORTANT: Only identify actual ad skip buttons inside the video player area. Do NOT identify \"Skip navigation\" links or other non-ad elements.";

    /// Prompt for identifying an unlabeled UI element from a cropped screenshot.
    pub const DESCRIBE_ELEMENT: &str = "\
You are looking at a cropped screenshot of a single UI element (button, icon, or control). \
What is this element? What icon does it show? What would clicking it likely do?

Return ONLY a JSON object:
{\"icon\": \"<what the icon depicts>\", \"action\": \"<likely action when clicked>\", \"label\": \"<suggested accessible label>\"}";

    /// Prompt for describing the current page state after an action.
    pub const VERIFY_CLICK: &str = "\
You are analyzing a browser screenshot taken AFTER a click action was performed. Briefly describe what you see on the page.

Return a JSON object:
{\"description\": \"<brief description of the current page state>\"}

Focus on: what page/content is shown, any video playing, any overlays/dialogs visible, any errors. Keep the description under 50 words.";

    /// Prompt for identifying which tiles in a reCAPTCHA grid contain the target object.
    pub const CAPTCHA_TILES: &str = "\
This is a reCAPTCHA challenge screenshot with a blue header and a grid of image tiles.

Grid numbering (left-to-right, top-to-bottom starting at 1):
  3x3: [1,2,3 / 4,5,6 / 7,8,9]
  4x4: [1,2,3,4 / 5,6,7,8 / 9,10,11,12 / 13,14,15,16]

Read the header to find the target object. Determine the challenge type:
  - \"dynamic\": 3x3 grid where tiles get replaced after clicking (header says \"select all images with\")
  - \"select_all\": 4x4 grid of one split image (header says \"select all squares with\")
  - \"none_skip\": any grid with a SKIP button (no tiles match)

Check each tile for the target. Return JSON:
{\"target\": \"<object>\", \"grid_size\": 9, \"tiles\": [<indices>], \"confidence\": 0.8, \"type\": \"dynamic\"}

For 4x4 grids, select ALL squares containing any part of the target.";

    /// Prompt for checking whether a single cropped tile contains a specific target object.
    pub const CAPTCHA_TILE_SINGLE: &str = "\
Does this cropped image tile contain a {target}? Look carefully at the entire image.

Return ONLY a JSON object:
{\"contains\": true, \"confidence\": <0.0-1.0>} or {\"contains\": false, \"confidence\": <0.0-1.0>}";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_json_from_prose() {
        let text = r#"Here is the result: {"found": true, "x": 500, "y": 300, "description": "Accept All"} and that's it."#;
        let json = VisionClient::extract_json(text).expect("should extract");
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(v["x"], 500);
        assert_eq!(v["y"], 300);
    }

    #[test]
    fn extract_json_from_code_block() {
        let text = "Sure! Here is the analysis:\n```json\n{\"found\": false}\n```";
        let json = VisionClient::extract_json(text).expect("should extract");
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(v["found"], false);
    }

    #[test]
    fn extract_json_bare() {
        let text = r#"{"found": true, "x": 100, "y": 200}"#;
        let json = VisionClient::extract_json(text).expect("should extract");
        assert_eq!(json, text);
    }

    #[test]
    fn extract_json_none() {
        assert!(VisionClient::extract_json("no json here").is_none());
    }

    #[test]
    fn extract_json_with_braces_in_strings() {
        let text = r#"{"description": "Click the } button", "found": true}"#;
        let json = VisionClient::extract_json(text).expect("should extract full object");
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(v["found"], true);
        assert_eq!(v["description"], "Click the } button");
    }

    #[test]
    fn extract_json_with_escaped_quotes_in_strings() {
        let text = r#"{"label": "the \"OK\" button", "x": 10}"#;
        let json = VisionClient::extract_json(text).expect("should extract");
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(v["x"], 10);
    }

    #[test]
    fn extract_json_with_multibyte_prefix() {
        let text = "Result: \u{1f3af} {\"found\": true}";
        let json = VisionClient::extract_json(text).expect("should extract");
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(v["found"], true);
    }

    #[test]
    fn parse_response_with_coordinates() {
        let text = r#"{"found": true, "x": 512, "y": 340, "description": "Accept All", "confidence": 0.95}"#;
        let resp = VisionClient::parse_response(text).expect("parse");
        assert!(resp.found);
        let target = resp.click_target.expect("should have target");
        assert!((target.x - 512.0).abs() < f64::EPSILON);
        assert!((target.y - 340.0).abs() < f64::EPSILON);
        assert_eq!(target.description.as_deref(), Some("Accept All"));
    }

    #[test]
    fn parse_response_not_found() {
        let text = r#"{"found": false}"#;
        let resp = VisionClient::parse_response(text).expect("parse");
        assert!(!resp.found);
        assert!(resp.click_target.is_none());
    }

    #[test]
    fn parse_response_natural_language_negative() {
        let text = "No consent dialog was found on this page.";
        let resp = VisionClient::parse_response(text).expect("parse");
        assert!(!resp.found);
    }
}
