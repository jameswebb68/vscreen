use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::Duration;

use tracing::{info, warn};

/// Dev environment: manages Xvfb, PulseAudio null-sink, and Chromium for testing.
pub(crate) struct DevEnvironment {
    display: u32,
    cdp_port: u16,
    xvfb: Option<Child>,
    chromium: Option<Child>,
    pulse_module_id: Option<u32>,
    cdp_endpoint: Option<String>,
    sink_name: String,
    pid_file: Option<PathBuf>,
}

impl std::fmt::Debug for DevEnvironment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DevEnvironment")
            .field("display", &self.display)
            .field("cdp_port", &self.cdp_port)
            .field("cdp_endpoint", &self.cdp_endpoint)
            .field("sink_name", &self.sink_name)
            .finish()
    }
}

impl DevEnvironment {
    /// Start the full dev environment: Xvfb, PulseAudio null-sink, Chromium.
    ///
    /// Performs conflict detection before starting:
    /// - Checks if the X11 display is already in use
    /// - Checks if the CDP port is already bound
    /// - Checks for an existing PID file from another vscreen instance
    ///
    /// # Errors
    /// Returns an error string if any component fails to start or conflicts are detected.
    pub(crate) async fn start(display: u32, cdp_port: u16) -> Result<Self, String> {
        // --- Conflict detection ---
        // PID file check must come first: if the previous vscreen is dead but
        // its children (Xvfb, Chromium) are still alive, this cleans them up
        // before the display/port checks run.
        check_pid_file(display)?;
        check_display_available(display)?;
        check_port_available(cdp_port)?;

        let pid_file_path = pid_file_path(display);
        let sink_name = format!("vscreen_dev_{display}");
        let mut env = Self {
            display,
            cdp_port,
            xvfb: None,
            chromium: None,
            pulse_module_id: None,
            cdp_endpoint: None,
            sink_name,
            pid_file: None,
        };

        env.start_xvfb()?;
        tokio::time::sleep(Duration::from_millis(500)).await;

        env.create_pulse_sink()?;

        env.launch_chromium().await?;

        // Write PID file after successful startup
        let pid = std::process::id();
        if let Err(e) = fs::write(&pid_file_path, pid.to_string()) {
            warn!(?e, path = %pid_file_path.display(), "failed to write PID file");
        } else {
            info!(path = %pid_file_path.display(), pid, "wrote PID file");
            env.pid_file = Some(pid_file_path);
        }

        Ok(env)
    }

    fn start_xvfb(&mut self) -> Result<(), String> {
        let display_arg = format!(":{}", self.display);
        let screen_arg = "0";
        let resolution = "1920x1080x24";

        info!(display = self.display, "starting Xvfb");

        let child = Command::new("Xvfb")
            .args([&display_arg, "-screen", screen_arg, resolution, "-ac", "-nolisten", "tcp"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("failed to start Xvfb: {e}"))?;

        info!(pid = child.id(), display = self.display, "Xvfb started");
        self.xvfb = Some(child);
        Ok(())
    }

    fn create_pulse_sink(&mut self) -> Result<(), String> {
        info!(sink = %self.sink_name, "creating PulseAudio null-sink");

        let output = Command::new("pactl")
            .args([
                "load-module",
                "module-null-sink",
                &format!("sink_name={}", self.sink_name),
                &format!("sink_properties=device.description={}", self.sink_name),
            ])
            .output()
            .map_err(|e| format!("failed to run pactl: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!(%stderr, "pactl failed, continuing without PulseAudio sink");
            return Ok(());
        }

        let module_id_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if let Ok(id) = module_id_str.parse::<u32>() {
            self.pulse_module_id = Some(id);
            info!(module_id = id, sink = %self.sink_name, "PulseAudio null-sink created");
        }

        Ok(())
    }

    async fn launch_chromium(&mut self) -> Result<(), String> {
        let display_env = format!(":{}", self.display);
        let cdp_port = self.cdp_port;

        info!(display = self.display, cdp_port, "launching Chromium");

        // Try chromium first, then chromium-browser, then google-chrome
        let browser = find_browser()?;

        let pulse_sink = if self.pulse_module_id.is_some() {
            format!("PULSE_SINK={}", self.sink_name)
        } else {
            String::new()
        };

        let mut cmd = Command::new(&browser);
        cmd.env("DISPLAY", &display_env);
        if !pulse_sink.is_empty() {
            cmd.env("PULSE_SINK", &self.sink_name);
        }
        let user_data_dir = format!("/tmp/vscreen-chrome-profile-{}", self.display);
        cmd.args([
            &format!("--remote-debugging-port={cdp_port}"),
            &format!("--user-data-dir={user_data_dir}"),
            "--no-sandbox",
            "--disable-gpu",
            "--use-gl=angle",
            "--use-angle=swiftshader",
            "--window-size=1920,1080",
            "--window-position=0,0",
            "--no-first-run",
            "--disable-default-apps",
            "--disable-extensions",
            "--disable-translate",
            "--disable-sync",
            "--autoplay-policy=no-user-gesture-required",
            "--force-color-profile=srgb",
            "--disable-background-timer-throttling",
            "--disable-renderer-backgrounding",
            "--disable-backgrounding-occluded-windows",
            "about:blank",
        ]);
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());

        let child = cmd.spawn().map_err(|e| format!("failed to launch {browser}: {e}"))?;
        info!(pid = child.id(), browser = %browser, "Chromium launched");
        self.chromium = Some(child);

        // Wait for CDP endpoint to become available
        let cdp_url = format!("http://127.0.0.1:{cdp_port}/json");
        let max_attempts = 30;
        for attempt in 1..=max_attempts {
            tokio::time::sleep(Duration::from_millis(500)).await;

            match reqwest_cdp_endpoint(&cdp_url).await {
                Ok(endpoint) => {
                    info!(endpoint = %endpoint, attempt, "CDP endpoint ready");
                    self.cdp_endpoint = Some(endpoint);
                    return Ok(());
                }
                Err(e) if attempt < max_attempts => {
                    if attempt % 5 == 0 {
                        warn!(attempt, max_attempts, error = %e, "still waiting for CDP endpoint");
                    }
                }
                Err(e) => return Err(format!("failed to get CDP endpoint after {max_attempts} attempts: {e}")),
            }
        }
        Err("CDP endpoint never became available".into())
    }

    /// Get the CDP WebSocket endpoint URL.
    #[must_use]
    pub(crate) fn cdp_endpoint(&self) -> Option<&str> {
        self.cdp_endpoint.as_deref()
    }

    /// Get the PulseAudio monitor source name.
    #[must_use]
    pub(crate) fn monitor_source(&self) -> String {
        format!("{}.monitor", self.sink_name)
    }

    /// Get the display number.
    #[must_use]
    #[allow(dead_code)]
    pub(crate) fn display(&self) -> u32 {
        self.display
    }

    /// Stop and clean up all dev environment processes.
    /// Sends SIGTERM first for graceful shutdown, then SIGKILL if needed.
    /// Chromium's entire process group is killed to prevent orphaned renderers.
    pub(crate) fn stop(&mut self) {
        if let Some(mut child) = self.chromium.take() {
            info!("stopping Chromium process tree");
            stop_process_tree(&mut child, std::time::Duration::from_secs(3));
        }

        if let Some(module_id) = self.pulse_module_id.take() {
            info!(module_id, "unloading PulseAudio module");
            let _ = Command::new("pactl")
                .args(["unload-module", &module_id.to_string()])
                .output();
        }

        if let Some(mut child) = self.xvfb.take() {
            info!("stopping Xvfb");
            stop_child_gracefully(&mut child, std::time::Duration::from_secs(2));
        }

        if let Some(pid_file) = self.pid_file.take() {
            if let Err(e) = fs::remove_file(&pid_file) {
                warn!(?e, path = %pid_file.display(), "failed to remove PID file");
            } else {
                info!(path = %pid_file.display(), "removed PID file");
            }
        }

        // Belt-and-suspenders: sweep for any strays matching our display
        cleanup_stale_processes_for_display(self.display);
    }
}

impl Drop for DevEnvironment {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Send SIGTERM first, wait up to `timeout`, then SIGKILL if still running.
fn stop_child_gracefully(child: &mut Child, timeout: std::time::Duration) {
    let pid = child.id();
    let _ = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .output();

    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < timeout => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
    }
}

/// Kill a process and all its descendants (the entire child tree).
/// First sends SIGTERM, then SIGKILL to all descendants via `pkill -P`.
fn stop_process_tree(child: &mut Child, timeout: std::time::Duration) {
    let pid = child.id();
    let pid_str = pid.to_string();

    // SIGTERM the main process and its immediate children
    let _ = Command::new("kill").args(["-TERM", &pid_str]).output();
    let _ = Command::new("pkill").args(["-TERM", "-P", &pid_str]).output();

    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < timeout => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            _ => {
                // Forcefully kill the entire subtree
                let _ = Command::new("pkill").args(["-9", "-P", &pid_str]).output();
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
    }
}

/// Kill any stale Xvfb or Chromium processes associated with a display and
/// unload orphaned PulseAudio null-sink modules.
/// Called during cleanup and startup to handle resources orphaned by prior crashes.
fn cleanup_stale_processes_for_display(disp: u32) {
    let display_str = format!(":{disp}");
    let profile_dir = format!("/tmp/vscreen-chrome-profile-{disp}");
    let sink_name = format!("vscreen_dev_{disp}");

    // Find and kill stale Xvfb processes for this display
    if let Ok(output) = Command::new("pgrep").args(["-f", &format!("Xvfb {display_str}")]).output() {
        if output.status.success() {
            let pids = String::from_utf8_lossy(&output.stdout);
            let my_pid = std::process::id().to_string();
            for pid_str in pids.split_whitespace() {
                if pid_str != my_pid {
                    info!(stale_pid = %pid_str, x11_display = disp, "killing stale Xvfb");
                    let _ = Command::new("kill").args(["-9", pid_str]).output();
                }
            }
        }
    }

    // Find and kill stale Chromium processes using this profile directory
    if let Ok(output) = Command::new("pgrep").args(["-f", &profile_dir]).output() {
        if output.status.success() {
            let pids = String::from_utf8_lossy(&output.stdout);
            for pid_str in pids.split_whitespace() {
                info!(stale_pid = %pid_str, x11_display = disp, "killing stale Chromium");
                let _ = Command::new("kill").args(["-9", pid_str]).output();
            }
        }
    }

    // Unload orphaned PulseAudio null-sink modules for this display.
    // `pactl list short modules` output: "ID\tmodule-name\targuments\n"
    if let Ok(output) = Command::new("pactl").args(["list", "short", "modules"]).output() {
        if output.status.success() {
            let listing = String::from_utf8_lossy(&output.stdout);
            for line in listing.lines() {
                if line.contains("module-null-sink") && line.contains(&sink_name) {
                    if let Some(module_id) = line.split_whitespace().next() {
                        info!(module_id, sink = %sink_name, "unloading orphaned PulseAudio module");
                        let _ = Command::new("pactl")
                            .args(["unload-module", module_id])
                            .output();
                    }
                }
            }
        }
    }

    // Give killed processes a moment to exit and release resources
    std::thread::sleep(std::time::Duration::from_millis(500));

    // Clean up stale X11 lock file
    let lock_path = format!("/tmp/.X{disp}-lock");
    if std::path::Path::new(&lock_path).exists() {
        let _ = fs::remove_file(&lock_path);
    }

    // Clean up stale X11 socket
    let socket_path = format!("/tmp/.X11-unix/X{disp}");
    if std::path::Path::new(&socket_path).exists() {
        let _ = fs::remove_file(&socket_path);
    }
}

fn find_browser() -> Result<String, String> {
    for name in &["chromium", "chromium-browser", "google-chrome", "google-chrome-stable"] {
        if Command::new("which")
            .arg(name)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Ok((*name).to_string());
        }
    }
    Err("no supported browser found (chromium, chromium-browser, google-chrome)".into())
}

fn pid_file_path(display: u32) -> PathBuf {
    PathBuf::from(format!("/tmp/vscreen-dev-{display}.pid"))
}

/// Fail if the X11 display lock file exists and the owning process is alive.
/// If the lock is stale, clean up orphaned processes and remove the lock.
fn check_display_available(disp: u32) -> Result<(), String> {
    let lock_path = format!("/tmp/.X{disp}-lock");
    let lock = std::path::Path::new(&lock_path);
    if !lock.exists() {
        return Ok(());
    }

    if let Ok(contents) = fs::read_to_string(lock) {
        if let Ok(pid) = contents.trim().parse::<i32>() {
            if is_process_alive(pid) {
                return Err(format!(
                    "X11 display :{disp} is already in use (Xvfb PID {pid}). \
                     Use --dev-display to choose a different display, or stop the other instance."
                ));
            }
            warn!(x11_display = disp, stale_pid = pid, "cleaning up stale display");
            cleanup_stale_processes_for_display(disp);
        }
    }
    Ok(())
}

/// Fail if the given TCP port is already bound.
fn check_port_available(port: u16) -> Result<(), String> {
    match std::net::TcpListener::bind(("127.0.0.1", port)) {
        Ok(_listener) => Ok(()),
        Err(_) => Err(format!(
            "CDP port {port} is already in use. \
             Use --dev-cdp-port to choose a different port, or stop the other instance."
        )),
    }
}

/// Fail if a PID file exists for this display and the owning process is alive.
/// If the PID is stale, clean up all orphaned processes for the display.
fn check_pid_file(disp: u32) -> Result<(), String> {
    let path = pid_file_path(disp);
    if !path.exists() {
        return Ok(());
    }

    if let Ok(contents) = fs::read_to_string(&path) {
        if let Ok(pid) = contents.trim().parse::<i32>() {
            if is_process_alive(pid) {
                return Err(format!(
                    "vscreen dev already running on display :{disp} (PID {pid}). \
                     Use --mcp-sse to connect to the existing server, or stop it first."
                ));
            }
            warn!(x11_display = disp, stale_pid = pid, "cleaning up stale vscreen instance");
            cleanup_stale_processes_for_display(disp);
            let _ = fs::remove_file(&path);
        }
    }
    Ok(())
}

/// Check if a process with the given PID is alive via `kill -0`.
fn is_process_alive(pid: i32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Query the CDP /json endpoint to get the WebSocket debugger URL.
async fn reqwest_cdp_endpoint(url: &str) -> Result<String, String> {
    // Use a simple TCP connection since we don't want to add reqwest dependency
    let response = tokio::time::timeout(
        Duration::from_secs(2),
        fetch_http_get(url),
    )
    .await
    .map_err(|_| "timeout".to_string())?
    .map_err(|e| e.to_string())?;

    // Parse the JSON array and get the first entry's webSocketDebuggerUrl
    let entries: Vec<serde_json::Value> = serde_json::from_str(&response)
        .map_err(|e| format!("parse CDP json: {e}"))?;

    entries
        .first()
        .and_then(|e| e.get("webSocketDebuggerUrl"))
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| "no webSocketDebuggerUrl in CDP response".into())
}

/// Minimal HTTP GET using raw TCP (avoids adding reqwest dependency).
///
/// Uses HTTP/1.1 (required by Chromium's CDP server) and reads incrementally,
/// returning as soon as Content-Length bytes of body have been received rather
/// than waiting for the connection to close.
async fn fetch_http_get(url: &str) -> Result<String, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let url_parts: Vec<&str> = url
        .strip_prefix("http://")
        .unwrap_or(url)
        .splitn(2, '/')
        .collect();
    let host_port = url_parts.first().ok_or("bad url")?;
    let path = url_parts.get(1).map_or("/", |p| p);

    let mut stream = tokio::net::TcpStream::connect(host_port)
        .await
        .map_err(|e| format!("connect: {e}"))?;

    let request =
        format!("GET /{path} HTTP/1.1\r\nHost: {host_port}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|e| format!("write: {e}"))?;

    let mut buf = Vec::with_capacity(8192);
    let mut tmp = [0u8; 4096];

    loop {
        let n = stream
            .read(&mut tmp)
            .await
            .map_err(|e| format!("read: {e}"))?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);

        if let Some(body) = try_extract_body(&buf) {
            return Ok(body);
        }
    }

    // Fallback: connection closed, try to parse whatever we got
    try_extract_body(&buf).ok_or_else(|| "no HTTP body".into())
}

/// Attempt to extract the HTTP body from a partially-received response buffer.
/// Returns `Some(body)` if enough data has been received, `None` otherwise.
fn try_extract_body(buf: &[u8]) -> Option<String> {
    let header_end = buf.windows(4).position(|w| w == b"\r\n\r\n")?;
    let headers = std::str::from_utf8(&buf[..header_end]).ok()?;
    let body_start = header_end + 4;
    let body_bytes = &buf[body_start..];

    // Try Content-Length first
    for line in headers.lines() {
        let lower = line.to_ascii_lowercase();
        if let Some(val) = lower.strip_prefix("content-length:") {
            if let Ok(cl) = val.trim().parse::<usize>() {
                if body_bytes.len() >= cl {
                    return String::from_utf8(body_bytes[..cl].to_vec()).ok();
                }
                return None; // need more data
            }
        }
    }

    // No Content-Length — check for complete JSON
    let text = std::str::from_utf8(body_bytes).ok()?;
    let trimmed = text.trim();
    if trimmed.ends_with(']') || trimmed.ends_with('}') {
        return Some(trimmed.to_string());
    }

    None
}
