use std::sync::Arc;

use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use vscreen_audio::capture::spawn_capture_thread;
use vscreen_audio::encode::OpusEncoder;
use vscreen_cdp::client::{CdpClient, RetryConfig};
use vscreen_cdp::protocol::StartScreencastParams;
use vscreen_core::error::VScreenError;
use vscreen_core::event::PeerInputEvent;
use vscreen_core::frame::{AudioBuffer, EncodedPacket};
use vscreen_core::instance::InstanceConfig;
use vscreen_core::traits::AudioEncoder;
use vscreen_transport::rtp::RtpSender;
use vscreen_video::pipeline::VideoPipeline;

use crate::memory::{ActionLog, ConsoleBuffer, ScreenshotHistory};

/// Stealth script injected via Page.addScriptToEvaluateOnNewDocument to reduce
/// bot detection (e.g. reCAPTCHA). Runs before any page JavaScript on every navigation.
const STEALTH_SCRIPT: &str = r#"
(function() {
    // 1. Override navigator.webdriver to undefined
    Object.defineProperty(navigator, 'webdriver', {
        get: () => undefined,
        configurable: true
    });

    // 2. Add chrome runtime object (missing in headless, detected by reCAPTCHA)
    if (!window.chrome) {
        window.chrome = {
            runtime: {
                onMessage: { addListener: function() {}, removeListener: function() {} },
                sendMessage: function() {},
                connect: function() { return { onMessage: { addListener: function() {} } }; }
            },
            loadTimes: function() { return {}; },
            csi: function() { return {}; }
        };
    }

    // 3. Set realistic navigator.plugins (headless has empty plugins array)
    Object.defineProperty(navigator, 'plugins', {
        get: () => {
            return [
                { name: 'Chrome PDF Plugin', filename: 'internal-pdf-viewer', description: 'Portable Document Format' },
                { name: 'Chrome PDF Viewer', filename: 'mhjfbmdgcfjbbpaeojofohoefgiehjai', description: '' },
                { name: 'Native Client', filename: 'internal-nacl-plugin', description: '' }
            ];
        },
        configurable: true
    });

    // 4. Set realistic navigator.languages
    Object.defineProperty(navigator, 'languages', {
        get: () => ['en-US', 'en'],
        configurable: true
    });

    // 5. Override permissions query (headless returns "denied" for notifications)
    if (navigator.permissions) {
        const originalQuery = navigator.permissions.query;
        navigator.permissions.query = (parameters) => {
            if (parameters.name === 'notifications') {
                return Promise.resolve({ state: 'default', onchange: null });
            }
            return originalQuery.call(navigator.permissions, parameters);
        };
    }

    // 6. Fix webgl renderer (some bots check for "SwiftShader" in renderer string)
    const getParameter = WebGLRenderingContext.prototype.getParameter;
    WebGLRenderingContext.prototype.getParameter = function(parameter) {
        if (parameter === 37445) return 'Intel Inc.';        // UNMASKED_VENDOR_WEBGL
        if (parameter === 37446) return 'Intel Iris OpenGL Engine'; // UNMASKED_RENDERER_WEBGL
        return getParameter.call(this, parameter);
    };
})();
"#;

/// Command sent to the dedicated encode thread (C1).
struct EncodeCommand {
    data: bytes::Bytes,
    force_keyframe: bool,
    bitrate_hint: Option<u32>,
}

/// Last known cursor position, updated on every mouse event.
/// Uses a single `AtomicU64` to guarantee a consistent (x, y) pair on read.
#[derive(Debug)]
pub struct CursorPosition {
    packed: std::sync::atomic::AtomicU64,
}

impl CursorPosition {
    fn new() -> Self {
        Self {
            packed: std::sync::atomic::AtomicU64::new(Self::pack(0, 0)),
        }
    }

    fn pack(x: i32, y: i32) -> u64 {
        ((x as u32 as u64) << 32) | (y as u32 as u64)
    }

    fn unpack(v: u64) -> (i32, i32) {
        ((v >> 32) as u32 as i32, v as u32 as i32)
    }

    fn update(&self, x: f64, y: f64) {
        #[allow(clippy::cast_possible_truncation)]
        let packed = Self::pack(x as i32, y as i32);
        self.packed.store(packed, std::sync::atomic::Ordering::Relaxed);
    }

    fn get(&self) -> (i32, i32) {
        Self::unpack(self.packed.load(std::sync::atomic::Ordering::Relaxed))
    }
}

/// A lightweight, temporary browser tab for isolated operations like parallel scraping.
///
/// Created via [`InstanceSupervisor::create_ephemeral_tab`] and must be closed
/// via [`InstanceSupervisor::close_ephemeral_tab`] when done.
pub struct EphemeralTab {
    pub target_id: String,
    pub client: Arc<tokio::sync::Mutex<CdpClient>>,
    cancel: CancellationToken,
}

impl std::fmt::Debug for EphemeralTab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EphemeralTab")
            .field("target_id", &self.target_id)
            .finish_non_exhaustive()
    }
}

impl EphemeralTab {
    /// Evaluate async JavaScript in this tab (expression may return a Promise).
    pub async fn evaluate_js_async(
        &self,
        expression: &str,
    ) -> Result<serde_json::Value, VScreenError> {
        let client = self.client.lock().await;
        client
            .evaluate_js_async(expression)
            .await
            .map_err(VScreenError::Cdp)
    }

    /// Evaluate synchronous JavaScript in this tab.
    pub async fn evaluate_js(
        &self,
        expression: &str,
    ) -> Result<serde_json::Value, VScreenError> {
        let client = self.client.lock().await;
        client
            .evaluate_js(expression)
            .await
            .map_err(VScreenError::Cdp)
    }

    /// Navigate this tab to a URL.
    pub async fn navigate(&self, url: &str) -> Result<(), VScreenError> {
        let client = self.client.lock().await;
        client.navigate(url).await.map_err(VScreenError::Cdp)
    }
}

/// Orchestrates the full media pipeline for a single instance:
/// CDP -> Video Pipeline -> broadcast, Audio Capture -> Opus -> broadcast,
/// Input relay, RTP output.
pub struct InstanceSupervisor {
    cancel: CancellationToken,
    cdp_client: Arc<tokio::sync::Mutex<CdpClient>>,
    video_broadcast: broadcast::Sender<EncodedPacket>,
    audio_broadcast: broadcast::Sender<EncodedPacket>,
    clipboard_broadcast: broadcast::Sender<String>,
    input_tx: mpsc::Sender<PeerInputEvent>,
    bitrate_hint_tx: mpsc::Sender<u32>,
    keyframe_flag: Arc<std::sync::atomic::AtomicBool>,
    cursor_position: Arc<CursorPosition>,
    tasks: tokio::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>,
    screenshot_history: std::sync::Mutex<ScreenshotHistory>,
    action_log: std::sync::Mutex<ActionLog>,
    console_buffer: std::sync::Mutex<ConsoleBuffer>,
    video_width: u32,
    video_height: u32,
    video_framerate: u32,
    video_codec: vscreen_core::frame::VideoCodec,
}

impl std::fmt::Debug for InstanceSupervisor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstanceSupervisor").finish_non_exhaustive()
    }
}

impl InstanceSupervisor {
    /// Start the full pipeline for an instance.
    ///
    /// # Errors
    /// Returns `VScreenError` if any component fails to initialize.
    pub async fn start(config: InstanceConfig) -> Result<Self, VScreenError> {
        let cancel = CancellationToken::new();
        let (video_broadcast, _) = broadcast::channel::<EncodedPacket>(8);
        let (audio_broadcast, _) = broadcast::channel::<EncodedPacket>(50);
        let (clipboard_broadcast, _) = broadcast::channel::<String>(8);
        let (input_tx, input_rx) = mpsc::channel::<PeerInputEvent>(100);
        let (bitrate_hint_tx, bitrate_hint_rx) = mpsc::channel::<u32>(8);
        let keyframe_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cursor_position = Arc::new(CursorPosition::new());

        let mut tasks = Vec::new();

        // --- CDP Client ---
        let mut cdp = CdpClient::new(
            config.cdp_endpoint.clone(),
            cancel.clone(),
            RetryConfig::default(),
        );

        cdp.connect().await.map_err(VScreenError::Cdp)?;

        // Inject stealth scripts before any page loads (reduces bot detection)
        cdp.send_command_and_wait("Page.enable", None)
            .await
            .map_err(VScreenError::Cdp)?;
        cdp.send_command_and_wait(
            "Page.addScriptToEvaluateOnNewDocument",
            Some(serde_json::json!({ "source": STEALTH_SCRIPT })),
        )
        .await
        .map_err(VScreenError::Cdp)?;

        let screencast_params = StartScreencastParams {
            format: "jpeg".into(),
            quality: 80,
            max_width: config.video.width,
            max_height: config.video.height,
            every_nth_frame: 1,
        };
        cdp.start_screencast(screencast_params)
            .await
            .map_err(VScreenError::Cdp)?;

        let cdp = Arc::new(tokio::sync::Mutex::new(cdp));

        // --- Video Pipeline (C1: dedicated encode thread off async runtime) ---
        let (video_ready_tx, video_ready_rx) =
            tokio::sync::oneshot::channel::<Result<(), String>>();
        {
            let cancel = cancel.clone();
            let video_tx = video_broadcast.clone();
            let cdp = cdp.clone();
            let video_config = config.video.clone();
            let keyframe_flag = keyframe_flag.clone();
            let mut bitrate_hint_rx = bitrate_hint_rx;

            let (encode_tx, encode_rx) =
                std::sync::mpsc::sync_channel::<EncodeCommand>(2);

            std::thread::Builder::new()
                .name("vscreen-encode".into())
                .spawn(move || {
                    let mut pipeline = match VideoPipeline::with_codec(video_config.clone(), video_config.codec) {
                        Ok(p) => {
                            let _ = video_ready_tx.send(Ok(()));
                            p
                        }
                        Err(e) => {
                            let _ = video_ready_tx.send(Err(e.to_string()));
                            return;
                        }
                    };

                    while let Ok(cmd) = encode_rx.recv() {
                        if let Some(kbps) = cmd.bitrate_hint {
                            info!(bitrate_kbps = kbps, "applying adaptive bitrate hint");
                            pipeline.reconfigure_bitrate(kbps);
                            metrics::gauge!("vscreen_video_bitrate_kbps")
                                .set(f64::from(kbps));
                        }
                        if cmd.force_keyframe {
                            pipeline.request_keyframe();
                        }
                        let start = std::time::Instant::now();
                        match pipeline.process(&cmd.data) {
                            Ok(packet) => {
                                metrics::histogram!("vscreen_encode_duration_seconds")
                                    .record(start.elapsed().as_secs_f64());
                                metrics::counter!("vscreen_frames_encoded_total")
                                    .increment(1);
                                let _ = video_tx.send(packet);
                            }
                            Err(e) => {
                                warn!(?e, "video encode failed");
                                pipeline.record_drop();
                                metrics::counter!("vscreen_frames_dropped_total")
                                    .increment(1);
                            }
                        }
                    }
                })
                .expect("spawn encode thread");

            let task = tokio::spawn(async move {
                let mut frame_rx = {
                    let client = cdp.lock().await;
                    client.screencast_frames()
                };

                info!("video pipeline task started");

                loop {
                    tokio::select! {
                        () = cancel.cancelled() => break,
                        frame = frame_rx.recv() => {
                            match frame {
                                Ok(mut raw_frame) => {
                                    let mut skipped = 0u64;
                                    loop {
                                        match frame_rx.try_recv() {
                                            Ok(newer) => {
                                                raw_frame = newer;
                                                skipped += 1;
                                            }
                                            Err(_) => break,
                                        }
                                    }
                                    if skipped > 0 {
                                        debug!(skipped, "skipped stale video frames");
                                        metrics::counter!("vscreen_frames_skipped_total").increment(skipped);
                                    }

                                    let force_keyframe = keyframe_flag.swap(
                                        false,
                                        std::sync::atomic::Ordering::Relaxed,
                                    );

                                    let mut latest_bitrate = None;
                                    while let Ok(br) = bitrate_hint_rx.try_recv() {
                                        latest_bitrate = Some(br);
                                    }

                                    let cmd = EncodeCommand {
                                        data: raw_frame.data,
                                        force_keyframe,
                                        bitrate_hint: latest_bitrate,
                                    };

                                    if encode_tx.send(cmd).is_err() {
                                        warn!("encode thread exited, stopping video pipeline");
                                        break;
                                    }
                                }
                                Err(broadcast::error::RecvError::Lagged(n)) => {
                                    warn!(n, "video pipeline lagged, dropped frames");
                                }
                                Err(broadcast::error::RecvError::Closed) => break,
                            }
                        }
                    }
                }

                info!("video pipeline task stopped");
            });
            tasks.push(task);
        }

        // Wait for video pipeline readiness (H4)
        match video_ready_rx.await {
            Ok(Ok(())) => info!("video pipeline ready"),
            Ok(Err(e)) => {
                cancel.cancel();
                return Err(VScreenError::InvalidState(format!(
                    "video pipeline failed to initialize: {e}"
                )));
            }
            Err(_) => {
                cancel.cancel();
                return Err(VScreenError::InvalidState(
                    "video pipeline thread exited before signaling readiness".into(),
                ));
            }
        }

        // --- Audio Capture + Encode Task (H4: oneshot for startup) ---
        let (audio_ready_tx, audio_ready_rx) =
            tokio::sync::oneshot::channel::<Result<(), String>>();
        {
            let cancel = cancel.clone();
            let audio_tx = audio_broadcast.clone();
            let audio_config = config.audio.clone();
            let pulse_source = config.pulse_source.clone();

            let task = tokio::spawn(async move {
                let (capture_tx, mut capture_rx) = mpsc::channel::<AudioBuffer>(50);

                let capture_handle = match spawn_capture_thread(&pulse_source, &audio_config, capture_tx) {
                    Ok(h) => h,
                    Err(e) => {
                        let _ = audio_ready_tx.send(Err(e.to_string()));
                        return;
                    }
                };

                let mut encoder = match OpusEncoder::new(audio_config) {
                    Ok(e) => {
                        let _ = audio_ready_tx.send(Ok(()));
                        e
                    }
                    Err(e) => {
                        let _ = audio_ready_tx.send(Err(e.to_string()));
                        capture_handle.stop();
                        return;
                    }
                };

                info!("audio pipeline task started");

                loop {
                    tokio::select! {
                        () = cancel.cancelled() => break,
                        buffer = capture_rx.recv() => {
                            match buffer {
                                Some(buf) => {
                                    match encoder.encode(&buf) {
                                        Ok(packet) => {
                                            metrics::counter!("vscreen_audio_frames_total").increment(1);
                                            let _ = audio_tx.send(packet);
                                        }
                                        Err(e) => {
                                            warn!(?e, "audio encode failed");
                                        }
                                    }
                                }
                                None => break,
                            }
                        }
                    }
                }

                capture_handle.stop();
                info!("audio pipeline task stopped");
            });
            tasks.push(task);
        }

        // Wait for audio pipeline readiness (H4)
        match audio_ready_rx.await {
            Ok(Ok(())) => info!("audio pipeline ready"),
            Ok(Err(e)) => {
                warn!(error = %e, "audio pipeline failed to start, continuing without audio");
            }
            Err(_) => {
                warn!("audio pipeline task exited before signaling readiness");
            }
        }

        // --- RTP Output Task ---
        if let Some(rtp_config) = &config.rtp_output {
            let cancel = cancel.clone();
            let rtp_sender = match RtpSender::new(rtp_config.clone()) {
                Ok(s) => Some(s),
                Err(e) => {
                    warn!(?e, "failed to create RTP sender, skipping RTP output");
                    None
                }
            };

            if let Some(mut rtp_sender) = rtp_sender {
                let mut audio_rx = audio_broadcast.subscribe();

                let task = tokio::spawn(async move {
                    if let Err(e) = rtp_sender.bind().await {
                        error!(?e, "failed to bind RTP socket");
                        return;
                    }

                    info!("RTP output task started");

                    loop {
                        tokio::select! {
                            () = cancel.cancelled() => break,
                            pkt = audio_rx.recv() => {
                                match pkt {
                                    Ok(packet) => {
                                        if let Err(e) = rtp_sender.send(&packet).await {
                                            warn!(?e, "RTP send failed");
                                        }
                                    }
                                    Err(broadcast::error::RecvError::Lagged(n)) => {
                                        warn!(n, "RTP output lagged");
                                    }
                                    Err(broadcast::error::RecvError::Closed) => break,
                                }
                            }
                        }
                    }

                    info!("RTP output task stopped");
                });
                tasks.push(task);
            }
        }

        // --- Input Relay Task ---
        {
            let cancel = cancel.clone();
            let cdp = cdp.clone();
            let bitrate_tx = bitrate_hint_tx.clone();
            let cursor_pos = cursor_position.clone();
            let mut input_rx = input_rx;

            let task = tokio::spawn(async move {
                info!("input relay task started");

                loop {
                    tokio::select! {
                        () = cancel.cancelled() => break,
                        event = input_rx.recv() => {
                            match event {
                                Some(peer_event) => {
                                    if let vscreen_core::event::InputEvent::BitrateHint { kbps } = &peer_event.event {
                                        if bitrate_tx.try_send(*kbps).is_err() {
                                            debug!(kbps, "bitrate hint dropped (channel full)");
                                        }
                                        continue;
                                    }
                                    // Track cursor position
                                    match &peer_event.event {
                                        vscreen_core::event::InputEvent::MouseMove { x, y, .. }
                                        | vscreen_core::event::InputEvent::MouseDown { x, y, .. }
                                        | vscreen_core::event::InputEvent::MouseUp { x, y, .. }
                                        | vscreen_core::event::InputEvent::Wheel { x, y, .. } => {
                                            cursor_pos.update(*x, *y);
                                        }
                                        _ => {}
                                    }
                                    let client = cdp.lock().await;
                                    if let Err(e) = client.dispatch_input(&peer_event.event).await {
                                        warn!(?e, "input dispatch failed");
                                    }
                                }
                                None => break,
                            }
                        }
                    }
                }

                info!("input relay task stopped");
            });
            tasks.push(task);
        }

        // --- Repaint Ticker Task ---
        // CDP screencast only emits frames when Chrome's compositor detects
        // visual changes. Many sites use composited layers that Chrome
        // doesn't flag as "changed", causing stale regions. This task
        // periodically injects a requestAnimationFrame loop that forces
        // continuous compositor frame generation, and re-injects after
        // navigation destroys the previous context.
        {
            let cancel = cancel.clone();
            let cdp = cdp.clone();

            let task = tokio::spawn(async move {
                info!("repaint ticker started");
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

                let inject_js = serde_json::json!({
                    "expression": concat!(
                        "(function(){",
                        "if(window.__vs)return;",
                        "window.__vs=1;",
                        "var c=document.createElement('canvas');",
                        "c.width=2;c.height=2;",
                        "c.style.cssText='position:fixed;bottom:0;right:0;width:1px;height:1px;",
                        "pointer-events:none;z-index:2147483647;opacity:0.01';",
                        "document.documentElement.appendChild(c);",
                        "var x=c.getContext('2d'),f=0;",
                        "function t(){",
                        "x.fillStyle='rgb('+(f%256)+',0,0)';",
                        "x.fillRect(0,0,2,2);",
                        "f++;",
                        "requestAnimationFrame(t)}",
                        "t();",
                        "})()"
                    ),
                    "returnByValue": false
                });

                loop {
                    tokio::select! {
                        () = cancel.cancelled() => break,
                        _ = interval.tick() => {
                            let client = cdp.lock().await;
                            let _ = client.send_command(
                                "Runtime.evaluate",
                                Some(inject_js.clone()),
                            ).await;
                        }
                    }
                }

                info!("repaint ticker stopped");
            });
            tasks.push(task);
        }

        info!(instance_id = %config.instance_id, "instance supervisor started");

        Ok(Self {
            cancel,
            cdp_client: cdp,
            video_broadcast,
            audio_broadcast,
            clipboard_broadcast,
            input_tx,
            bitrate_hint_tx,
            keyframe_flag,
            cursor_position,
            tasks: tokio::sync::Mutex::new(tasks),
            screenshot_history: std::sync::Mutex::new(ScreenshotHistory::new(20)),
            action_log: std::sync::Mutex::new(ActionLog::new(100)),
            console_buffer: std::sync::Mutex::new(ConsoleBuffer::new(200)),
            video_width: config.video.width,
            video_height: config.video.height,
            video_framerate: config.video.framerate,
            video_codec: config.video.codec,
        })
    }

    /// Subscribe to encoded video packets.
    #[must_use]
    pub fn video_receiver(&self) -> broadcast::Receiver<EncodedPacket> {
        self.video_broadcast.subscribe()
    }

    /// Subscribe to encoded audio packets.
    #[must_use]
    pub fn audio_receiver(&self) -> broadcast::Receiver<EncodedPacket> {
        self.audio_broadcast.subscribe()
    }

    /// Get a reference to the audio broadcast sender (for RTSP server subscription).
    #[must_use]
    pub fn audio_broadcast(&self) -> &broadcast::Sender<EncodedPacket> {
        &self.audio_broadcast
    }

    /// Get a reference to the video broadcast sender (for RTSP server subscription).
    #[must_use]
    pub fn video_broadcast(&self) -> &broadcast::Sender<EncodedPacket> {
        &self.video_broadcast
    }

    /// Configured video resolution.
    #[must_use]
    pub fn video_resolution(&self) -> (u32, u32) {
        (self.video_width, self.video_height)
    }

    /// Configured video framerate.
    #[must_use]
    pub fn video_framerate(&self) -> u32 {
        self.video_framerate
    }

    /// Configured video codec.
    #[must_use]
    pub fn video_codec(&self) -> vscreen_core::frame::VideoCodec {
        self.video_codec
    }

    /// Subscribe to clipboard content from the remote browser.
    #[must_use]
    pub fn clipboard_receiver(&self) -> broadcast::Receiver<String> {
        self.clipboard_broadcast.subscribe()
    }

    /// Get a sender for input events (to share with peer sessions).
    #[must_use]
    pub fn input_sender(&self) -> mpsc::Sender<PeerInputEvent> {
        self.input_tx.clone()
    }

    /// Get a sender for adaptive bitrate hints (target kbps).
    #[must_use]
    pub fn bitrate_hint_sender(&self) -> mpsc::Sender<u32> {
        self.bitrate_hint_tx.clone()
    }

    /// Request the next video frame to be a keyframe (e.g. after WebRTC connect).
    pub fn request_keyframe(&self) {
        self.keyframe_flag
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Navigate the CDP session to a URL.
    ///
    /// # Errors
    /// Returns `VScreenError` if the CDP command fails.
    pub async fn navigate(&self, url: &str) -> Result<(), VScreenError> {
        let client = self.cdp_client.lock().await;
        client.navigate(url).await.map_err(VScreenError::Cdp)
    }

    /// Capture a screenshot via CDP `Page.captureScreenshot`.
    ///
    /// # Errors
    /// Returns `VScreenError` if the CDP command fails.
    pub async fn capture_screenshot(
        &self,
        format: &str,
        quality: Option<u32>,
    ) -> Result<bytes::Bytes, VScreenError> {
        let client = self.cdp_client.lock().await;
        client
            .capture_screenshot(format, quality)
            .await
            .map_err(VScreenError::Cdp)
    }

    /// Capture a full-page screenshot by temporarily resizing the viewport to match
    /// the full document height, capturing, then restoring the viewport.
    ///
    /// # Errors
    /// Returns `VScreenError` if the CDP command fails.
    pub async fn capture_full_page_screenshot(
        &self,
        format: &str,
        quality: Option<u32>,
    ) -> Result<bytes::Bytes, VScreenError> {
        let client = self.cdp_client.lock().await;
        client
            .capture_full_page_screenshot(format, quality)
            .await
            .map_err(VScreenError::Cdp)
    }

    /// Capture a sequence of screenshots at a fixed interval.
    ///
    /// # Errors
    /// Returns `VScreenError` if any capture fails.
    pub async fn capture_screenshot_sequence(
        &self,
        count: u32,
        interval_ms: u64,
        format: &str,
        quality: Option<u32>,
    ) -> Result<Vec<bytes::Bytes>, VScreenError> {
        let mut images = Vec::with_capacity(count as usize);
        for i in 0..count {
            let img = self.capture_screenshot(format, quality).await?;
            images.push(img);
            if i < count - 1 {
                tokio::time::sleep(std::time::Duration::from_millis(interval_ms)).await;
            }
        }
        Ok(images)
    }

    /// Dispatch an input event directly via CDP (bypasses peer input channel).
    ///
    /// # Errors
    /// Returns `VScreenError` if the CDP command fails.
    pub async fn dispatch_api_input(
        &self,
        event: &vscreen_core::event::InputEvent,
    ) -> Result<(), VScreenError> {
        let client = self.cdp_client.lock().await;
        client.dispatch_input(event).await.map_err(VScreenError::Cdp)
    }

    /// Translate page-level coordinates to viewport-relative coordinates by
    /// scrolling to bring the target position into view first. Returns the
    /// viewport-relative `(x, y)` after scrolling.
    ///
    /// If the coordinates are already within the visible viewport, no scroll
    /// occurs and the coordinates are returned adjusted for the current scroll
    /// offset.
    ///
    /// # Errors
    /// Returns `VScreenError` if JS evaluation fails.
    pub async fn scroll_into_view_and_translate(
        &self,
        page_x: f64,
        page_y: f64,
    ) -> Result<(f64, f64), VScreenError> {
        let client = self.cdp_client.lock().await;

        let info = client
            .evaluate_js("JSON.stringify({scrollX:window.scrollX,scrollY:window.scrollY,innerW:window.innerWidth,innerH:window.innerHeight})")
            .await
            .map_err(VScreenError::Cdp)?;

        let info_str = info.as_str().unwrap_or("{}");
        let info_obj: serde_json::Value = serde_json::from_str(info_str)
            .map_err(|e| VScreenError::InvalidState(format!("viewport info parse failed: {e}")))?;

        let scroll_x = info_obj.get("scrollX").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let scroll_y = info_obj.get("scrollY").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let inner_w = info_obj.get("innerW").and_then(|v| v.as_f64()).unwrap_or(1920.0);
        let inner_h = info_obj.get("innerH").and_then(|v| v.as_f64()).unwrap_or(940.0);

        let margin = 50.0;
        let y_in_view = page_y >= scroll_y + margin && page_y <= scroll_y + inner_h - margin;
        let x_in_view = page_x >= scroll_x + margin && page_x <= scroll_x + inner_w - margin;

        if x_in_view && y_in_view {
            return Ok((page_x - scroll_x, page_y - scroll_y));
        }

        let target_scroll_x = if x_in_view { scroll_x } else { (page_x - inner_w / 2.0).max(0.0) };
        let target_scroll_y = if y_in_view { scroll_y } else { (page_y - inner_h / 2.0).max(0.0) };
        let result = client
            .evaluate_js(&format!(
                "window.scrollTo({target_scroll_x}, {target_scroll_y}); JSON.stringify({{x:window.scrollX,y:window.scrollY}})"
            ))
            .await
            .map_err(VScreenError::Cdp)?;

        let scroll_str = result.as_str().unwrap_or("{}");
        let scroll_obj: serde_json::Value = serde_json::from_str(scroll_str).unwrap_or_default();
        let new_scroll_x = scroll_obj.get("x").and_then(|v| v.as_f64()).unwrap_or(target_scroll_x);
        let new_scroll_y = scroll_obj.get("y").and_then(|v| v.as_f64()).unwrap_or(target_scroll_y);

        drop(client);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        Ok((page_x - new_scroll_x, page_y - new_scroll_y))
    }

    /// Send a raw CDP command (fire-and-forget).
    ///
    /// # Errors
    /// Returns `VScreenError` if the CDP command fails.
    pub async fn send_cdp_command(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<(), VScreenError> {
        let client = self.cdp_client.lock().await;
        client
            .send_command(method, params)
            .await
            .map_err(VScreenError::Cdp)
    }

    /// Send a CDP command and wait for a response with a result payload.
    ///
    /// # Errors
    /// Returns `VScreenError` if the CDP command fails.
    pub async fn send_cdp_command_and_wait(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, VScreenError> {
        let client = self.cdp_client.lock().await;
        let msg = client
            .send_command_and_wait(method, params)
            .await
            .map_err(VScreenError::Cdp)?;
        Ok(msg.result.unwrap_or(serde_json::Value::Null))
    }

    /// Get page info (URL, title, viewport) via JS evaluation.
    ///
    /// # Errors
    /// Returns `VScreenError` if the JS evaluation fails.
    pub async fn get_page_info(&self) -> Result<serde_json::Value, VScreenError> {
        let client = self.cdp_client.lock().await;
        client
            .evaluate_js(
                "({url: location.href, title: document.title, viewport: {width: window.innerWidth, height: window.innerHeight}, scrollX: Math.round(window.scrollX), scrollY: Math.round(window.scrollY)})",
            )
            .await
            .map_err(VScreenError::Cdp)
    }

    /// Execute arbitrary JavaScript and return the result.
    ///
    /// # Errors
    /// Returns `VScreenError` if the JS evaluation fails.
    pub async fn evaluate_js(&self, expression: &str) -> Result<serde_json::Value, VScreenError> {
        let client = self.cdp_client.lock().await;
        client
            .evaluate_js(expression)
            .await
            .map_err(VScreenError::Cdp)
    }

    /// Execute async JavaScript (expression may return a Promise).
    ///
    /// # Errors
    /// Returns `VScreenError` if the JS evaluation fails.
    pub async fn evaluate_js_async(
        &self,
        expression: &str,
    ) -> Result<serde_json::Value, VScreenError> {
        let client = self.cdp_client.lock().await;
        client
            .evaluate_js_async(expression)
            .await
            .map_err(VScreenError::Cdp)
    }

    /// Execute JavaScript in a specific frame's context.
    ///
    /// # Errors
    /// Returns `VScreenError` if the JS evaluation fails.
    pub async fn evaluate_js_in_frame(
        &self,
        expression: &str,
        frame_id: &str,
    ) -> Result<serde_json::Value, VScreenError> {
        let client = self.cdp_client.lock().await;
        client
            .evaluate_js_in_frame(expression, frame_id)
            .await
            .map_err(VScreenError::Cdp)
    }

    /// Get the page frame tree via CDP.
    ///
    /// # Errors
    /// Returns `VScreenError` if the CDP call fails.
    pub async fn get_frame_tree(&self) -> Result<serde_json::Value, VScreenError> {
        let client = self.cdp_client.lock().await;
        client
            .get_frame_tree()
            .await
            .map_err(VScreenError::Cdp)
    }

    /// Capture a screenshot with an optional clip rectangle.
    ///
    /// # Errors
    /// Returns `VScreenError` on CDP failure.
    pub async fn capture_screenshot_clip(
        &self,
        format: &str,
        quality: Option<u32>,
        clip: Option<(f64, f64, f64, f64)>,
    ) -> Result<bytes::Bytes, VScreenError> {
        let client = self.cdp_client.lock().await;
        client
            .capture_screenshot_clip(format, quality, clip)
            .await
            .map_err(VScreenError::Cdp)
    }

    /// Capture a downscaled JPEG for vision model analysis.
    ///
    /// Uses a scale factor of 0.5 on the full viewport (960x540 from 1920x1080)
    /// and JPEG quality=60 to keep the payload small while retaining enough
    /// detail for a vision LLM.
    ///
    /// # Errors
    /// Returns `VScreenError` on CDP failure.
    pub async fn capture_vision_screenshot(&self) -> Result<bytes::Bytes, VScreenError> {
        let client = self.cdp_client.lock().await;
        client
            .capture_screenshot_clip_scaled(
                "jpeg",
                Some(60),
                Some((0.0, 0.0, f64::from(self.video_width), f64::from(self.video_height))),
                0.5,
            )
            .await
            .map_err(VScreenError::Cdp)
    }

    /// Get the last known cursor position.
    #[must_use]
    pub fn get_cursor_position(&self) -> (i32, i32) {
        self.cursor_position.get()
    }

    // -----------------------------------------------------------------------
    // Memory systems (interior mutability via std::sync::Mutex)
    // -----------------------------------------------------------------------

    pub fn with_screenshot_history<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&ScreenshotHistory) -> R,
    {
        let guard = self.screenshot_history.lock().expect("screenshot_history lock");
        f(&guard)
    }

    pub fn with_screenshot_history_mut<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut ScreenshotHistory) -> R,
    {
        let mut guard = self.screenshot_history.lock().expect("screenshot_history lock");
        f(&mut guard)
    }

    pub fn with_action_log<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&ActionLog) -> R,
    {
        let guard = self.action_log.lock().expect("action_log lock");
        f(&guard)
    }

    pub fn with_action_log_mut<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut ActionLog) -> R,
    {
        let mut guard = self.action_log.lock().expect("action_log lock");
        f(&mut guard)
    }

    pub fn with_console_buffer<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&ConsoleBuffer) -> R,
    {
        let guard = self.console_buffer.lock().expect("console_buffer lock");
        f(&guard)
    }

    pub fn with_console_buffer_mut<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut ConsoleBuffer) -> R,
    {
        let mut guard = self.console_buffer.lock().expect("console_buffer lock");
        f(&mut guard)
    }

    /// Inject a JS-based console capture shim that intercepts console.log/warn/error/info
    /// and stores messages in a JS-side ring buffer, which we can read back later.
    pub async fn enable_console_capture(&self) -> Result<(), VScreenError> {
        let js = r#"(function() {
    if (window.__vscreen_console) return 'already_enabled';
    window.__vscreen_console = [];
    const MAX = 200;
    ['log','warn','error','info'].forEach(level => {
        const orig = console[level];
        console[level] = function() {
            const text = Array.from(arguments).map(a => {
                try { return typeof a === 'string' ? a : JSON.stringify(a); }
                catch(_) { return String(a); }
            }).join(' ');
            window.__vscreen_console.push({level, text, t: Date.now()});
            if (window.__vscreen_console.length > MAX) window.__vscreen_console.shift();
            return orig.apply(console, arguments);
        };
    });
    return 'enabled';
})()"#;
        self.evaluate_js(js).await?;
        Ok(())
    }

    /// Read captured console messages from the JS-side buffer and sync them
    /// into our Rust-side `ConsoleBuffer`.
    pub async fn sync_console_messages(&self) -> Result<(), VScreenError> {
        let js = r#"(function() {
    const msgs = window.__vscreen_console || [];
    window.__vscreen_console = [];
    return JSON.stringify(msgs);
})()"#;
        let result = self.evaluate_js(js).await?;
        if let Some(json_str) = result.as_str() {
            if let Ok(entries) = serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
                self.with_console_buffer_mut(|buf| {
                    for entry in entries {
                        let level = entry.get("level").and_then(|v| v.as_str()).unwrap_or("log");
                        let text = entry.get("text").and_then(|v| v.as_str()).unwrap_or("");
                        buf.push(level.to_string(), text.to_string());
                    }
                });
            }
        }
        Ok(())
    }

    /// Create an ephemeral browser tab for isolated operations (e.g. parallel scraping).
    ///
    /// Returns an `EphemeralTab` with its own CDP connection. The tab is separate
    /// from the main tab and will not affect its state. The caller must call
    /// `close_ephemeral_tab` when done.
    ///
    /// # Errors
    /// Returns `VScreenError` if tab creation or CDP connection fails.
    pub async fn create_ephemeral_tab(
        &self,
        url: &str,
    ) -> Result<EphemeralTab, VScreenError> {
        let (target_id, ws_url) = {
            let client = self.cdp_client.lock().await;
            let tid = client.create_target(url).await.map_err(VScreenError::Cdp)?;
            let ws = client.endpoint_for_target(&tid);
            (tid, ws)
        };

        let cancel = CancellationToken::new();
        let mut tab_client = CdpClient::new(
            ws_url,
            cancel.clone(),
            RetryConfig {
                max_attempts: 3,
                ..RetryConfig::default()
            },
        );
        tab_client.connect().await.map_err(VScreenError::Cdp)?;

        Ok(EphemeralTab {
            target_id,
            client: Arc::new(tokio::sync::Mutex::new(tab_client)),
            cancel,
        })
    }

    /// Close an ephemeral tab, disconnecting its CDP client and removing the target.
    ///
    /// # Errors
    /// Returns `VScreenError` if the target cannot be closed.
    pub async fn close_ephemeral_tab(&self, tab: &EphemeralTab) -> Result<(), VScreenError> {
        tab.cancel.cancel();
        let client = self.cdp_client.lock().await;
        client
            .close_target(&tab.target_id)
            .await
            .map_err(VScreenError::Cdp)
    }

    /// Get the current page URL via JS evaluation, returning an empty string on failure.
    pub async fn current_url(&self) -> String {
        self.evaluate_js("location.href")
            .await
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default()
    }

    /// Stop all tasks and clean up.
    pub async fn stop(&self) {
        self.cancel.cancel();
        let mut tasks = self.tasks.lock().await;
        for task in tasks.drain(..) {
            let _ = task.await;
        }
        info!("instance supervisor stopped");
    }
}

impl Drop for InstanceSupervisor {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // CursorPosition
    // -----------------------------------------------------------------------

    #[test]
    fn cursor_position_initial() {
        let pos = CursorPosition::new();
        assert_eq!(pos.get(), (0, 0));
    }

    #[test]
    fn cursor_position_update() {
        let pos = CursorPosition::new();
        pos.update(150.7, 300.2);
        assert_eq!(pos.get(), (150, 300));
    }

    #[test]
    fn cursor_position_negative() {
        let pos = CursorPosition::new();
        pos.update(-10.0, -20.0);
        assert_eq!(pos.get(), (-10, -20));
    }

    #[test]
    fn cursor_position_multiple_updates() {
        let pos = CursorPosition::new();
        pos.update(100.0, 200.0);
        assert_eq!(pos.get(), (100, 200));
        pos.update(500.5, 600.9);
        assert_eq!(pos.get(), (500, 600));
    }

    #[test]
    fn cursor_position_shared_arc() {
        let pos = Arc::new(CursorPosition::new());
        let pos2 = pos.clone();
        pos.update(42.0, 84.0);
        assert_eq!(pos2.get(), (42, 84));
    }
}
