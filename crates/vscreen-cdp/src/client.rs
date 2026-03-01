use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use dashmap::DashMap;
use futures_util::stream::SplitStream;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument, warn};
use vscreen_core::error::CdpError;
use vscreen_core::event::InputEvent;
use vscreen_core::frame::RawFrame;

use crate::input::input_to_cdp;
use crate::protocol::{CdpMessage, CdpRequest, ScreencastFrameEvent, StartScreencastParams};
use crate::screencast::ScreencastManager;

/// Concrete WebSocket read stream type for reconnection (avoids generic recursion issues).
type WsReadStream = SplitStream<
    tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
>;

type PendingRequests = Arc<DashMap<u64, oneshot::Sender<CdpMessage>>>;

const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

// ---------------------------------------------------------------------------
// Connection state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CdpConnectionState {
    Connecting,
    Connected,
    Reconnecting { attempt: u32 },
    Disconnected,
}

// ---------------------------------------------------------------------------
// Retry configuration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub max_attempts: u32,
    pub jitter: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(5),
            max_attempts: 10,
            jitter: true,
        }
    }
}

// ---------------------------------------------------------------------------
// CDP Client
// ---------------------------------------------------------------------------

/// CDP WebSocket client managing connection, screencast, and input relay.
#[derive(Debug)]
pub struct CdpClient {
    endpoint: String,
    screencast: Arc<ScreencastManager>,
    state_tx: watch::Sender<CdpConnectionState>,
    state_rx: watch::Receiver<CdpConnectionState>,
    frame_tx: broadcast::Sender<RawFrame>,
    request_tx: Arc<tokio::sync::Mutex<mpsc::Sender<CdpRequest>>>,
    pending_requests: PendingRequests,
    cancel: CancellationToken,
    retry_config: RetryConfig,
    screencast_params: Arc<tokio::sync::Mutex<Option<StartScreencastParams>>>,
    /// Holds JoinHandles for background tasks
    _tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl CdpClient {
    /// Create a new `CdpClient` without connecting.
    #[must_use]
    pub fn new(
        endpoint: String,
        cancel: CancellationToken,
        retry_config: RetryConfig,
    ) -> Self {
        let (state_tx, state_rx) = watch::channel(CdpConnectionState::Disconnected);
        let (frame_tx, _) = broadcast::channel(3);
        let (request_tx, _request_rx) = mpsc::channel(64);
        let screencast = Arc::new(ScreencastManager::new());

        Self {
            endpoint,
            screencast,
            state_tx,
            state_rx,
            frame_tx,
            request_tx: Arc::new(tokio::sync::Mutex::new(request_tx)),
            pending_requests: Arc::new(DashMap::new()),
            cancel,
            retry_config,
            screencast_params: Arc::new(tokio::sync::Mutex::new(None)),
            _tasks: Vec::new(),
        }
    }

    /// Connect to the CDP endpoint and start background read/write tasks.
    ///
    /// # Errors
    /// Returns `CdpError` if the initial connection fails.
    #[instrument(skip(self), fields(endpoint = %self.endpoint))]
    pub async fn connect(&mut self) -> Result<(), CdpError> {
        let _ = self.state_tx.send(CdpConnectionState::Connecting);
        info!("connecting to CDP endpoint");

        let ws = Self::try_connect(&self.endpoint).await?;
        let _ = self.state_tx.send(CdpConnectionState::Connected);
        info!("CDP connection established");

        let (write, read) = ws.split();
        let (req_tx, req_rx) = mpsc::channel(64);

        {
            let mut tx_guard = self.request_tx.lock().await;
            *tx_guard = req_tx.clone();
        }

        let write_handle = tokio::spawn(Self::write_loop(
            write,
            req_rx,
            self.cancel.clone(),
        ));

        let connection_handle = tokio::spawn(Self::connection_loop(
            read,
            self.frame_tx.clone(),
            self.screencast.clone(),
            self.request_tx.clone(),
            self.pending_requests.clone(),
            self.state_tx.clone(),
            self.endpoint.clone(),
            self.retry_config.clone(),
            self.screencast_params.clone(),
            self.cancel.clone(),
        ));

        self._tasks.push(write_handle);
        self._tasks.push(connection_handle);

        Ok(())
    }

    async fn try_connect(
        endpoint: &str,
    ) -> Result<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        CdpError,
    > {
        let (ws, _) = tokio_tungstenite::connect_async(endpoint)
            .await
            .map_err(|e| CdpError::ConnectionFailed(e.to_string()))?;
        Ok(ws)
    }

    async fn write_loop(
        mut write: impl SinkExt<tokio_tungstenite::tungstenite::Message, Error = tokio_tungstenite::tungstenite::Error>
            + Unpin
            + Send,
        mut request_rx: mpsc::Receiver<CdpRequest>,
        cancel: CancellationToken,
    ) {
        loop {
            tokio::select! {
                biased;
                () = cancel.cancelled() => {
                    debug!("CDP write loop cancelled");
                    break;
                }
                msg = request_rx.recv() => {
                    match msg {
                        Some(req) => {
                            let json = match serde_json::to_string(&req) {
                                Ok(j) => j,
                                Err(e) => {
                                    error!(?e, "failed to serialize CDP request");
                                    continue;
                                }
                            };
                            if let Err(e) = write
                                .send(tokio_tungstenite::tungstenite::Message::Text(json.into()))
                                .await
                            {
                                error!(?e, "failed to send CDP message");
                                break;
                            }
                        }
                        None => break,
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn connection_loop(
        mut read: WsReadStream,
        frame_tx: broadcast::Sender<RawFrame>,
        screencast: Arc<ScreencastManager>,
        shared_request_tx: Arc<tokio::sync::Mutex<mpsc::Sender<CdpRequest>>>,
        pending_requests: PendingRequests,
        state_tx: watch::Sender<CdpConnectionState>,
        endpoint: String,
        retry_config: RetryConfig,
        screencast_params: Arc<tokio::sync::Mutex<Option<StartScreencastParams>>>,
        cancel: CancellationToken,
    ) {
        let mut generation = screencast.generation();

        'outer: loop {
            // --- Read messages until disconnect ---
            loop {
                tokio::select! {
                    biased;
                    () = cancel.cancelled() => {
                        debug!("CDP read loop cancelled");
                        return;
                    }
                    msg = read.next() => {
                        match msg {
                            Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => {
                                let req_tx = shared_request_tx.lock().await.clone();
                                if let Err(e) = Self::handle_message(
                                    &text,
                                    &frame_tx,
                                    &screencast,
                                    &req_tx,
                                    &pending_requests,
                                    generation,
                                )
                                .await
                                {
                                    warn!(?e, "error handling CDP message");
                                }
                            }
                            Some(Ok(_)) => {}
                            Some(Err(e)) => {
                                warn!(?e, "CDP WebSocket error");
                                break;
                            }
                            None => {
                                info!("CDP WebSocket closed");
                                break;
                            }
                        }
                    }
                }
            }

            // Fail all pending requests on disconnect
            pending_requests.clear();

            // --- Reconnection attempts ---
            for attempt in 0..retry_config.max_attempts {
                if cancel.is_cancelled() {
                    return;
                }

                let _ = state_tx.send(CdpConnectionState::Reconnecting {
                    attempt: attempt + 1,
                });
                let delay = backoff_delay(attempt, &retry_config);
                info!(
                    attempt = attempt + 1,
                    max = retry_config.max_attempts,
                    delay_ms = delay.as_millis() as u64,
                    "CDP reconnecting"
                );

                tokio::select! {
                    () = cancel.cancelled() => return,
                    () = tokio::time::sleep(delay) => {}
                }

                match Self::try_connect(&endpoint).await {
                    Ok(new_ws) => {
                        info!("CDP reconnected");
                        let _ = state_tx.send(CdpConnectionState::Connected);

                        let (new_write, new_read) = new_ws.split();
                        let (new_req_tx, new_req_rx) = mpsc::channel(64);

                        {
                            let mut tx_guard = shared_request_tx.lock().await;
                            *tx_guard = new_req_tx.clone();
                        }

                        tokio::spawn(Self::write_loop(
                            new_write,
                            new_req_rx,
                            cancel.clone(),
                        ));

                        // Bump generation so stale frames from the old
                        // connection are discarded (L3).
                        generation = screencast.bump_generation();
                        if let Some(params) = screencast_params.lock().await.clone() {
                            let req = screencast.start_request(&params);
                            let tx = shared_request_tx.lock().await.clone();
                            let _ = tx.send(req).await;
                        }

                        // Reassign read and continue outer loop
                        read = new_read;
                        continue 'outer;
                    }
                    Err(e) => {
                        warn!(attempt = attempt + 1, %e, "CDP reconnect failed");
                    }
                }
            }

            // Exhausted all attempts
            let _ = state_tx.send(CdpConnectionState::Disconnected);
            error!(
                attempts = retry_config.max_attempts,
                "CDP reconnect exhausted"
            );
            return;
        }
    }

    async fn handle_message(
        text: &str,
        frame_tx: &broadcast::Sender<RawFrame>,
        screencast: &ScreencastManager,
        request_tx: &mpsc::Sender<CdpRequest>,
        pending_requests: &DashMap<u64, oneshot::Sender<CdpMessage>>,
        generation: u64,
    ) -> Result<(), CdpError> {
        let msg: CdpMessage =
            serde_json::from_str(text).map_err(|e| CdpError::Protocol(e.to_string()))?;

        // Complete pending request-response correlations
        if msg.is_response() {
            if let Some(id) = msg.id {
                if let Some((_, tx)) = pending_requests.remove(&id) {
                    let _ = tx.send(msg);
                    return Ok(());
                }
            }
        }

        if msg.is_event() {
            if let Some(method) = &msg.method {
                if method == "Page.screencastFrame" {
                    if let Some(params) = &msg.params {
                        let event: ScreencastFrameEvent = serde_json::from_value(params.clone())
                            .map_err(|e| CdpError::Screencast(e.to_string()))?;

                        let session_id = event.session_id;

                        if let Some(frame) = screencast.decode_frame(&event, generation)? {
                            match frame_tx.send(frame) {
                                Ok(n) => {
                                    debug!(receivers = n, "broadcast screencast frame");
                                }
                                Err(_) => {
                                    debug!("no active screencast frame receivers");
                                }
                            }
                        }

                        // Always ack so Chrome sends the next frame
                        let ack = ScreencastManager::ack_request(session_id);
                        let _ = request_tx.send(ack).await;
                    }
                }
            }
        }

        Ok(())
    }

    // -------------------------------------------------------------------
    // Request-response methods
    // -------------------------------------------------------------------

    /// Send a CDP request and wait for the correlated response.
    ///
    /// # Errors
    /// Returns `CdpError` on connection loss, timeout, or CDP error response.
    pub async fn send_command_and_wait(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<CdpMessage, CdpError> {
        let req = CdpRequest::new(method, params);
        let id = req.id;

        let (tx, rx) = oneshot::channel();
        self.pending_requests.insert(id, tx);

        if let Err(e) = self.send_request(req).await {
            self.pending_requests.remove(&id);
            return Err(e);
        }

        match tokio::time::timeout(DEFAULT_REQUEST_TIMEOUT, rx).await {
            Ok(Ok(msg)) => {
                if let Some(ref err) = msg.error {
                    return Err(CdpError::Protocol(format!(
                        "CDP error {}: {}",
                        err.code, err.message
                    )));
                }
                Ok(msg)
            }
            Ok(Err(_)) => Err(CdpError::ConnectionLost),
            Err(_) => {
                self.pending_requests.remove(&id);
                Err(CdpError::Timeout { ms: DEFAULT_REQUEST_TIMEOUT.as_millis() as u64 })
            }
        }
    }

    /// Capture a full-page screenshot via CDP `Page.captureScreenshot`.
    ///
    /// Returns the raw image bytes (PNG, JPEG, or WebP depending on `format`).
    ///
    /// # Errors
    /// Returns `CdpError` on connection loss, timeout, or CDP error.
    pub async fn capture_screenshot(
        &self,
        format: &str,
        quality: Option<u32>,
    ) -> Result<bytes::Bytes, CdpError> {
        let mut params = serde_json::json!({
            "format": format,
            "fromSurface": true,
        });
        if let Some(q) = quality {
            params["quality"] = serde_json::json!(q);
        }

        let response = self
            .send_command_and_wait("Page.captureScreenshot", Some(params))
            .await?;

        let data = response
            .result
            .as_ref()
            .and_then(|r| r.get("data"))
            .and_then(|d| d.as_str())
            .ok_or_else(|| CdpError::Protocol("missing screenshot data in response".into()))?;

        let decoded = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|e| CdpError::Protocol(format!("base64 decode failed: {e}")))?;

        Ok(bytes::Bytes::from(decoded))
    }

    /// Capture a full-page screenshot by measuring document dimensions via JS,
    /// then using a clip rect with `captureBeyondViewport` to capture the entire
    /// scrollable document in one shot.
    ///
    /// # Errors
    /// Returns `CdpError` on connection loss, timeout, or CDP error.
    pub async fn capture_full_page_screenshot(
        &self,
        format: &str,
        quality: Option<u32>,
    ) -> Result<bytes::Bytes, CdpError> {
        // 1. Get the full document dimensions via JS (most reliable across Chrome versions)
        let dims = self.evaluate_js(
            "JSON.stringify({w: Math.max(document.documentElement.scrollWidth, document.body.scrollWidth), h: Math.max(document.documentElement.scrollHeight, document.body.scrollHeight)})"
        ).await?;

        let dims_str = dims.as_str().ok_or_else(|| {
            CdpError::Protocol("unexpected type for document dimensions".into())
        })?;
        let dims_obj: serde_json::Value = serde_json::from_str(dims_str)
            .map_err(|e| CdpError::Protocol(format!("failed to parse dimensions: {e}")))?;

        let content_width = dims_obj.get("w").and_then(|v| v.as_f64()).unwrap_or(1920.0);
        let content_height = dims_obj.get("h").and_then(|v| v.as_f64()).unwrap_or(1080.0);

        // Cap at 16384px (Chrome's max compositing texture size)
        let capped_width = content_width.min(16384.0);
        let capped_height = content_height.min(16384.0);

        // 2. Scroll to top to ensure consistent capture
        let _ = self.evaluate_js("window.scrollTo(0, 0)").await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        // 3. Temporarily override device metrics to full document size so Chrome
        //    renders the full page content (not just the Xvfb viewport)
        let override_params = serde_json::json!({
            "width": capped_width as u32,
            "height": capped_height as u32,
            "deviceScaleFactor": 1,
            "mobile": false,
            "screenWidth": capped_width as u32,
            "screenHeight": capped_height as u32,
        });
        self.send_command_and_wait("Emulation.setDeviceMetricsOverride", Some(override_params))
            .await?;

        // Give Chrome time to re-layout at the new viewport dimensions
        tokio::time::sleep(Duration::from_millis(300)).await;

        // 4. Capture the screenshot — the viewport is now the full document size
        let mut params = serde_json::json!({
            "format": format,
            "fromSurface": true,
        });
        if let Some(q) = quality {
            params["quality"] = serde_json::json!(q);
        }

        let response = self
            .send_command_and_wait("Page.captureScreenshot", Some(params))
            .await;

        // 5. Always restore the original viewport, even if capture failed
        let _ = self
            .send_command_and_wait("Emulation.clearDeviceMetricsOverride", None)
            .await;

        let response = response?;
        let data = response
            .result
            .as_ref()
            .and_then(|r| r.get("data"))
            .and_then(|d| d.as_str())
            .ok_or_else(|| CdpError::Protocol("missing screenshot data in response".into()))?;

        let decoded = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|e| CdpError::Protocol(format!("base64 decode failed: {e}")))?;

        Ok(bytes::Bytes::from(decoded))
    }

    /// Evaluate a JavaScript expression via CDP `Runtime.evaluate` and return its value.
    ///
    /// # Errors
    /// Returns `CdpError` on connection loss, timeout, JS exception, or CDP error.
    pub async fn evaluate_js(
        &self,
        expression: &str,
    ) -> Result<serde_json::Value, CdpError> {
        let params = serde_json::json!({
            "expression": expression,
            "returnByValue": true,
        });

        let response = self
            .send_command_and_wait("Runtime.evaluate", Some(params))
            .await?;

        let result_obj = response
            .result
            .ok_or_else(|| CdpError::Protocol("missing evaluation result".into()))?;

        if let Some(exception) = result_obj.get("exceptionDetails") {
            let text = exception
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("unknown JS error");
            return Err(CdpError::Protocol(format!("JS exception: {text}")));
        }

        Ok(result_obj
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(serde_json::Value::Null))
    }

    /// Capture a screenshot with an optional clip rectangle.
    ///
    /// # Errors
    /// Returns `CdpError` on connection loss, timeout, or CDP error.
    pub async fn capture_screenshot_clip(
        &self,
        format: &str,
        quality: Option<u32>,
        clip: Option<(f64, f64, f64, f64)>,
    ) -> Result<bytes::Bytes, CdpError> {
        self.capture_screenshot_clip_scaled(format, quality, clip, 1.0)
            .await
    }

    /// Capture a screenshot with optional clip rectangle and scale factor.
    ///
    /// `scale` controls the output resolution: 0.5 = half resolution, 1.0 = native.
    ///
    /// # Errors
    /// Returns `CdpError` on connection loss, timeout, or CDP error.
    pub async fn capture_screenshot_clip_scaled(
        &self,
        format: &str,
        quality: Option<u32>,
        clip: Option<(f64, f64, f64, f64)>,
        scale: f64,
    ) -> Result<bytes::Bytes, CdpError> {
        let mut params = serde_json::json!({
            "format": format,
            "fromSurface": true,
        });
        if let Some(q) = quality {
            params["quality"] = serde_json::json!(q);
        }
        if let Some((x, y, w, h)) = clip {
            params["clip"] = serde_json::json!({
                "x": x, "y": y, "width": w, "height": h, "scale": scale
            });
        }

        let response = self
            .send_command_and_wait("Page.captureScreenshot", Some(params))
            .await?;

        let data = response
            .result
            .as_ref()
            .and_then(|r| r.get("data"))
            .and_then(|d| d.as_str())
            .ok_or_else(|| CdpError::Protocol("missing screenshot data in response".into()))?;

        let decoded = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|e| CdpError::Protocol(format!("base64 decode failed: {e}")))?;

        Ok(bytes::Bytes::from(decoded))
    }

    /// Get the frame tree via `Page.getFrameTree`.
    ///
    /// # Errors
    /// Returns `CdpError` on connection loss, timeout, or CDP error.
    pub async fn get_frame_tree(&self) -> Result<serde_json::Value, CdpError> {
        let response = self
            .send_command_and_wait("Page.getFrameTree", None)
            .await?;
        Ok(response.result.unwrap_or(serde_json::Value::Null))
    }

    /// Evaluate JS in a specific frame's execution context.
    /// Uses `Page.createIsolatedWorld` to get a context for the frame.
    ///
    /// # Errors
    /// Returns `CdpError` on connection loss, timeout, or CDP error.
    pub async fn evaluate_js_in_frame(
        &self,
        expression: &str,
        frame_id: &str,
    ) -> Result<serde_json::Value, CdpError> {
        let world_response = self
            .send_command_and_wait(
                "Page.createIsolatedWorld",
                Some(serde_json::json!({
                    "frameId": frame_id,
                    "grantUniveralAccess": true,
                })),
            )
            .await?;

        let context_id = world_response
            .result
            .as_ref()
            .and_then(|r| r.get("executionContextId"))
            .and_then(|v| v.as_i64())
            .ok_or_else(|| CdpError::Protocol("failed to create isolated world for frame".into()))?;

        let params = serde_json::json!({
            "expression": expression,
            "contextId": context_id,
            "returnByValue": true,
        });

        let response = self
            .send_command_and_wait("Runtime.evaluate", Some(params))
            .await?;

        let result_obj = response
            .result
            .ok_or_else(|| CdpError::Protocol("missing evaluation result".into()))?;

        if let Some(exception) = result_obj.get("exceptionDetails") {
            let text = exception
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("unknown JS error");
            return Err(CdpError::Protocol(format!("JS exception in frame: {text}")));
        }

        Ok(result_obj
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(serde_json::Value::Null))
    }

    // -------------------------------------------------------------------
    // Existing public API
    // -------------------------------------------------------------------

    /// Subscribe to screencast frames.
    #[must_use]
    pub fn screencast_frames(&self) -> broadcast::Receiver<RawFrame> {
        self.frame_tx.subscribe()
    }

    /// Start screencast.
    ///
    /// # Errors
    /// Returns `CdpError` if the request cannot be sent.
    pub async fn start_screencast(
        &self,
        config: StartScreencastParams,
    ) -> Result<(), CdpError> {
        {
            let mut params = self.screencast_params.lock().await;
            *params = Some(config.clone());
        }
        let req = self.screencast.start_request(&config);
        self.send_request(req).await
    }

    /// Stop screencast.
    ///
    /// # Errors
    /// Returns `CdpError` if the request cannot be sent.
    pub async fn stop_screencast(&self) -> Result<(), CdpError> {
        let req = self.screencast.stop_request();
        self.send_request(req).await
    }

    /// Navigate to a URL.
    ///
    /// # Errors
    /// Returns `CdpError` if the request cannot be sent.
    pub async fn navigate(&self, url: &str) -> Result<(), CdpError> {
        let params = serde_json::json!({ "url": url });
        let req = CdpRequest::new("Page.navigate", Some(params));
        self.send_request(req).await
    }

    /// Dispatch an input event to Chromium via CDP.
    ///
    /// # Errors
    /// Returns `CdpError` if the request cannot be sent.
    pub async fn dispatch_input(&self, event: &InputEvent) -> Result<(), CdpError> {
        let req = input_to_cdp(event);
        self.send_request(req).await
    }

    /// Get a receiver for connection state changes.
    #[must_use]
    pub fn state(&self) -> watch::Receiver<CdpConnectionState> {
        self.state_rx.clone()
    }

    /// Send an arbitrary CDP request by method name and params (fire-and-forget).
    ///
    /// # Errors
    /// Returns `CdpError` if the request cannot be sent.
    pub async fn send_command(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<(), CdpError> {
        let req = CdpRequest::new(method, params);
        self.send_request(req).await
    }

    /// Send a raw CDP request.
    async fn send_request(&self, req: CdpRequest) -> Result<(), CdpError> {
        let tx = self.request_tx.lock().await;
        tx.send(req)
            .await
            .map_err(|_| CdpError::ConnectionLost)
    }
}

/// Compute delay for exponential backoff with optional jitter.
#[must_use]
pub fn backoff_delay(attempt: u32, config: &RetryConfig) -> Duration {
    let base = config
        .initial_delay
        .saturating_mul(1u32.checked_shl(attempt).unwrap_or(u32::MAX));
    let delay = base.min(config.max_delay);

    if config.jitter {
        let ms = delay.as_millis() as u64;
        let jittered = ms / 2 + (ms / 2).wrapping_mul(attempt as u64 + 1) % (ms / 2 + 1);
        Duration::from_millis(jittered)
    } else {
        delay
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_state() {
        let cancel = CancellationToken::new();
        let client = CdpClient::new(
            "ws://localhost:9222".into(),
            cancel,
            RetryConfig::default(),
        );
        assert_eq!(*client.state_rx.borrow(), CdpConnectionState::Disconnected);
    }

    #[test]
    fn backoff_increases() {
        let config = RetryConfig {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(5),
            max_attempts: 10,
            jitter: false,
        };
        let d0 = backoff_delay(0, &config);
        let d1 = backoff_delay(1, &config);
        let d2 = backoff_delay(2, &config);
        assert_eq!(d0, Duration::from_millis(100));
        assert_eq!(d1, Duration::from_millis(200));
        assert_eq!(d2, Duration::from_millis(400));
    }

    #[test]
    fn backoff_respects_max() {
        let config = RetryConfig {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(300),
            max_attempts: 10,
            jitter: false,
        };
        let d5 = backoff_delay(5, &config);
        assert_eq!(d5, Duration::from_millis(300));
    }

    #[test]
    fn backoff_with_jitter_bounded() {
        let config = RetryConfig {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(5),
            max_attempts: 10,
            jitter: true,
        };
        for attempt in 0..10 {
            let d = backoff_delay(attempt, &config);
            assert!(d <= config.max_delay);
        }
    }

    #[test]
    fn screencast_frame_subscription() {
        let cancel = CancellationToken::new();
        let client = CdpClient::new(
            "ws://localhost:9222".into(),
            cancel,
            RetryConfig::default(),
        );
        let _rx = client.screencast_frames();
    }

    #[test]
    fn pending_requests_starts_empty() {
        let cancel = CancellationToken::new();
        let client = CdpClient::new(
            "ws://localhost:9222".into(),
            cancel,
            RetryConfig::default(),
        );
        assert!(client.pending_requests.is_empty());
    }

    #[tokio::test]
    async fn send_command_and_wait_times_out_when_disconnected() {
        let cancel = CancellationToken::new();
        let client = CdpClient::new(
            "ws://localhost:9222".into(),
            cancel,
            RetryConfig::default(),
        );
        // Not connected — send_request will fail with ConnectionLost
        let result = client
            .send_command_and_wait("Test.method", None)
            .await;
        assert!(result.is_err());
        match result {
            Err(vscreen_core::error::CdpError::ConnectionLost) => {}
            other => panic!("expected ConnectionLost, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn handle_message_completes_pending_request() {
        let pending: DashMap<u64, oneshot::Sender<CdpMessage>> = DashMap::new();
        let (tx, rx) = oneshot::channel();
        pending.insert(42, tx);

        let response_json = r#"{"id":42,"result":{"data":"hello"}}"#;
        let (frame_tx, _) = broadcast::channel(1);
        let screencast = ScreencastManager::new();
        let (req_tx, _req_rx) = mpsc::channel(1);

        let result = CdpClient::handle_message(
            response_json,
            &frame_tx,
            &screencast,
            &req_tx,
            &pending,
            0,
        )
        .await;
        assert!(result.is_ok());

        // The pending request should have been removed and fulfilled
        assert!(pending.is_empty());
        let msg = rx.await.expect("oneshot resolved");
        assert_eq!(msg.id, Some(42));
        assert!(msg.result.is_some());
    }

    #[tokio::test]
    async fn handle_message_ignores_unmatched_response() {
        let pending: DashMap<u64, oneshot::Sender<CdpMessage>> = DashMap::new();

        let response_json = r#"{"id":999,"result":{}}"#;
        let (frame_tx, _) = broadcast::channel(1);
        let screencast = ScreencastManager::new();
        let (req_tx, _req_rx) = mpsc::channel(1);

        let result = CdpClient::handle_message(
            response_json,
            &frame_tx,
            &screencast,
            &req_tx,
            &pending,
            0,
        )
        .await;
        // No error — just no matching pending request
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn handle_message_passes_events_through() {
        let pending: DashMap<u64, oneshot::Sender<CdpMessage>> = DashMap::new();

        // An event (no id) should not affect pending requests
        let event_json = r#"{"method":"Page.loadEventFired","params":{}}"#;
        let (frame_tx, _) = broadcast::channel(1);
        let screencast = ScreencastManager::new();
        let (req_tx, _req_rx) = mpsc::channel(1);

        let result = CdpClient::handle_message(
            event_json,
            &frame_tx,
            &screencast,
            &req_tx,
            &pending,
            0,
        )
        .await;
        assert!(result.is_ok());
        assert!(pending.is_empty());
    }

    #[test]
    fn state_watcher() {
        let cancel = CancellationToken::new();
        let client = CdpClient::new(
            "ws://localhost:9222".into(),
            cancel,
            RetryConfig::default(),
        );
        let rx = client.state();
        assert_eq!(*rx.borrow(), CdpConnectionState::Disconnected);
    }
}
