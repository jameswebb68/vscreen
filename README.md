# vscreen — Virtual Screen Media Bridge

vscreen turns a headless Chromium browser into a remotely viewable, controllable, and AI-automatable virtual screen. It captures the browser viewport via Chrome DevTools Protocol (CDP), encodes video as H.264 (default) or VP9 and audio as Opus, and delivers them over WebRTC to connected clients. Clients can send mouse and keyboard input back through a WebRTC DataChannel for full bidirectional interaction. An integrated MCP (Model Context Protocol) server exposes 63 tools for AI browser automation. Audio is also available via RTSP for external consumers.

## How it works

```
 Xvfb + Chromium           vscreen                  Browser Client
 ┌──────────────┐    ┌─────────────────┐     ┌──────────────────────┐
 │  Renders web  │───>│ CDP screencast  │     │                      │
 │  page at      │    │ JPEG → I420     │     │  <video> element     │
 │  1920×1080    │    │ → H264/VP9      │────>│  shows remote screen │
 │               │    │                 │     │                      │
 │  PulseAudio   │───>│ PA capture      │     │  Audio playback      │
 │  audio output │    │ → Opus encode   │────>│                      │
 │               │    │                 │     │  Mouse + keyboard    │
 │  Receives     │<───│ CDP input       │<────│  via DataChannel     │
 │  input events │    │ dispatch        │     │                      │
 └──────────────┘    └─────────────────┘     └──────────────────────┘
                           │     │
                           │     │  MCP (stdio / SSE / stdio-proxy)
                           │     ▼
                           │  AI Automation Clients
                           │  (Cursor, custom agents)
                           │
                           │ RTSP (Opus audio)
                           ▼
                     External consumers
                     (GStreamer, FFmpeg, VLC)
```

## Features

- **H.264 video** (default) via OpenH264 or **VP9** — selectable with `--video-codec`
- **Opus audio** captured from PulseAudio (default 48 kHz stereo @ 128 kbps)
- **WebRTC delivery** with automatic ICE/STUN negotiation and adaptive bitrate
- **Bidirectional input** — mouse, keyboard, scroll, drag, right-click, clipboard forwarded to Chromium
- **RTSP audio server** — pull-based Opus streaming for external consumers (`rtsp://host:8554/audio/{id}`)
- **Dev mode** — single command spawns Xvfb, PulseAudio, and Chromium automatically
- **MCP server** — 63 AI automation tools via stdio, HTTP/SSE, or stdio-proxy transport
- **Instance locking** — lease-based ownership with exclusive/observer modes for multi-agent coordination
- **REST API** — full programmatic instance management, screenshots, input, element discovery
- **Full-page screenshots** — capture entire scrollable pages beyond the viewport
- **Coordinate translation** — auto-scroll when clicking on full-page screenshot coordinates
- **Screenshot history** — ring buffer of last 20 screenshots with metadata for AI backtracking
- **Action session log** — timestamped history of all actions for "how did I get here" context
- **Element discovery** — find elements by CSS selector, visible text, accessibility tree, or input attributes
- **Smart synchronization** — wait for text, selector, URL change, or network idle
- **Console capture** — captured browser console.log/warn/error messages
- **Annotated screenshots** — numbered bounding boxes on interactive elements with legends
- **Cookie/storage management** — get/set cookies, localStorage, sessionStorage
- **Iframe support** — element discovery and click actions across cross-origin iframes
- **Vision LLM integration** — optional vision model for identifying unlabeled UI elements (icon-only buttons)
- **Automated captcha solving** — reCAPTCHA v2 image challenges via vision LLM
- **Dialog/ad dismissal** — auto-dismiss cookie consent banners, GDPR overlays, video ad overlays
- **Bearer token auth** — optional authentication via header or query parameter
- **Prometheus metrics** — `/metrics` endpoint for monitoring
- **Docker support** — multi-stage Dockerfile and docker-compose included
- **Test client** — built-in browser UI with URL navigation, fullscreen, stats HUD

---

## Quick Start (Dev Mode)

Dev mode is the fastest way to get running. It spawns a virtual X11 display, PulseAudio sink, and Chromium instance automatically.

### Prerequisites

```bash
# Debian/Ubuntu
sudo apt install -y \
  build-essential cmake pkg-config libssl-dev libclang-dev \
  libvpx-dev libopus-dev libpulse-dev \
  chromium xvfb pulseaudio \
  fonts-noto fonts-noto-color-emoji
```

### Build and run

```bash
# Build the release binary
cargo build --release -p vscreen --features pulse-audio

# Start in dev mode (H.264 video by default)
./target/release/vscreen --dev

# Start with VP9 video instead
./target/release/vscreen --dev --video-codec vp9

# Start with a URL pre-loaded
./target/release/vscreen --dev --dev-url "https://www.youtube.com"

# Start with MCP server enabled (stdio)
./target/release/vscreen --dev --mcp-stdio

# Start with MCP SSE server on a separate port
./target/release/vscreen --dev --mcp-sse 0.0.0.0:8451

# Start with RTSP disabled
./target/release/vscreen --dev --no-rtsp
```

### What dev mode does

1. **Starts Xvfb** — a virtual X11 display (default `:99`) at 1920x1080x24
2. **Creates a PulseAudio null-sink** — named `vscreen_dev_99` for audio capture
3. **Launches Chromium** — headless, with remote debugging on port 9222, using the virtual display
4. **Connects via CDP** — screencast capture begins immediately
5. **Starts the HTTP/WS server** — on `0.0.0.0:8450` (default)
6. **Starts the RTSP server** — on port `8554` (default, disable with `--no-rtsp`)
7. **Creates a `dev` instance** — ready for WebRTC connections and API calls

The `dev` instance ID is always `"dev"`. All API calls and MCP tools use this ID.

### Connect the test client

```bash
cd tools/test-client
pnpm install   # or npm install
pnpm dev       # or npx vite
```

1. Open `http://localhost:5173` in your browser
2. The signal URL defaults to `/signal/dev` — Vite proxies it to the backend
3. Click **Connect**
4. Type a URL in the navigation bar and click **Go**
5. Interact with the remote browser — mouse, keyboard, scroll all work

Only port 5173 needs to be accessible — Vite proxies all API and WebSocket traffic to the vscreen backend on port 8450.

### Test client features

- **Video/audio playback** via WebRTC
- **Full mouse input** — click, double-click, drag, scroll, right-click
- **Full keyboard input** — all keys including arrows, backspace, delete, function keys
- **Clipboard** — Ctrl+V pastes from host, copy from remote browser syncs back
- **URL navigation bar** — type URLs and press Enter or click Go
- **Fullscreen** — click the Fullscreen button
- **Stats HUD** — press F2 to toggle FPS, bitrate, RTT, loss, and resolution overlay
- **Auto-reconnect** — WebSocket drops are retried with exponential backoff

### Receive audio externally

Audio is available via RTSP at `rtsp://localhost:8554/audio/dev`. Connect with any RTSP-capable player:

```bash
# FFmpeg
ffplay rtsp://localhost:8554/audio/dev

# VLC
vlc rtsp://localhost:8554/audio/dev

# GStreamer
gst-launch-1.0 rtspsrc location=rtsp://localhost:8554/audio/dev ! decodebin ! autoaudiosink
```

---

## MCP Server (AI Automation)

vscreen includes a built-in MCP server that exposes 63 tools for AI browser automation. This enables AI models to control the browser programmatically: navigate, take screenshots, click elements, type text, read page content, and more.

### Connection modes

vscreen supports three MCP transport modes:

- **SSE (recommended for Cursor)**: Start the server with `--mcp-sse 0.0.0.0:8451`, then configure your MCP client with the SSE URL. The server runs independently and survives client reconnections.
- **Stdio proxy**: For MCP clients that only support subprocess spawning, use `--mcp-stdio-proxy http://localhost:8451/mcp`. This lightweight proxy forwards messages to an existing SSE server without starting its own dev environment.
- **Stdio direct**: `--mcp-stdio` starts the MCP server on stdin/stdout. Use when the MCP client spawns vscreen as a subprocess.

**Best practice**: Start the server once with `--dev --mcp-sse`, then connect via SSE URL or stdio proxy.

### Starting the MCP server

```bash
# SSE transport (recommended — persistent, survives reconnections)
./target/release/vscreen --dev --mcp-sse 0.0.0.0:8451

# stdio transport (for subprocess spawning)
./target/release/vscreen --dev --mcp-stdio

# Both transports simultaneously
./target/release/vscreen --dev --mcp-stdio --mcp-sse 0.0.0.0:8451

# Stdio proxy to an already-running SSE server (no dev mode, no pipelines)
./target/release/vscreen --mcp-stdio-proxy http://localhost:8451/mcp
```

### Cursor IDE integration

For connecting to an already-running vscreen with SSE (recommended):

```json
{
  "mcpServers": {
    "vscreen": {
      "url": "http://localhost:8451/mcp"
    }
  }
}
```

For subprocess spawning via stdio:

```json
{
  "mcpServers": {
    "vscreen": {
      "command": "/path/to/vscreen",
      "args": ["--mcp-stdio", "--dev"],
      "env": {}
    }
  }
}
```

For stdio proxy to an existing SSE server:

```json
{
  "mcpServers": {
    "vscreen": {
      "command": "/path/to/vscreen",
      "args": ["--mcp-stdio-proxy", "http://localhost:8451/mcp"],
      "env": {}
    }
  }
}
```

### Instance locking

When multiple AI agents share a vscreen instance, use locking to prevent conflicts:

- **Exclusive lock** — only the lock holder can perform actions
- **Observer lock** — read-only access (screenshots, element queries)
- Locks have a TTL and must be renewed via heartbeat (`vscreen_instance_lock_renew`)
- In single-agent mode (`--mcp-single-agent`), lock checks are bypassed

### MCP tool reference

#### Instance management

| Tool | Description |
|------|-------------|
| `vscreen_list_instances` | List all browser instances and their states |
| `vscreen_instance_lock` | Acquire exclusive or observer lock on an instance |
| `vscreen_instance_unlock` | Release lock on an instance |
| `vscreen_instance_lock_status` | Query lock status for one or all instances |
| `vscreen_instance_lock_renew` | Extend lock TTL (heartbeat) |

#### Observation

| Tool | Description |
|------|-------------|
| `vscreen_screenshot` | Capture viewport or full-page screenshot (PNG/JPEG/WebP) with optional clip region |
| `vscreen_screenshot_sequence` | Capture N screenshots at fixed interval |
| `vscreen_screenshot_annotated` | Screenshot with numbered bounding boxes on interactive elements |
| `vscreen_get_page_info` | Get current URL, title, and viewport dimensions |
| `vscreen_get_cursor_position` | Get last known mouse cursor position |
| `vscreen_extract_text` | Extract visible text from page or specific element |

#### Input actions

| Tool | Description |
|------|-------------|
| `vscreen_click` | Click at coordinates (auto-scrolls for full-page coords) |
| `vscreen_double_click` | Double-click at coordinates |
| `vscreen_type` | Append text into focused element |
| `vscreen_fill` | Clear field and replace with text (by selector) |
| `vscreen_key_press` | Press a key with optional modifiers |
| `vscreen_key_combo` | Press a key combination (e.g., Ctrl+A) |
| `vscreen_scroll` | Scroll at position |
| `vscreen_drag` | Click-drag between two points |
| `vscreen_hover` | Move mouse without clicking |
| `vscreen_batch_click` | Click multiple coordinates in one call |
| `vscreen_click_element` | Click by CSS selector or visible text (with retries, scroll-into-view) |
| `vscreen_select_option` | Select dropdown option by value or label |
| `vscreen_scroll_to_element` | Scroll element into view by CSS selector |

#### Navigation

| Tool | Description |
|------|-------------|
| `vscreen_navigate` | Navigate to a URL (with `wait_until` option) |
| `vscreen_go_back` | Browser back button |
| `vscreen_go_forward` | Browser forward button |
| `vscreen_reload` | Reload the current page |
| `vscreen_click_and_navigate` | Click element and wait for URL change |

#### Element discovery

| Tool | Description |
|------|-------------|
| `vscreen_find_elements` | Find elements by CSS selector with bounding boxes (supports `include_iframes`) |
| `vscreen_find_by_text` | Find elements by visible text content (supports `include_iframes`) |
| `vscreen_find_input` | Find text inputs by placeholder, aria-label, label, role, or name |
| `vscreen_accessibility_tree` | Get structured accessibility tree |
| `vscreen_describe_elements` | Identify unlabeled UI elements using vision LLM |
| `vscreen_list_frames` | List frames/iframes with bounding rectangles |

#### Synchronization

| Tool | Description |
|------|-------------|
| `vscreen_wait` | Wait for a specified duration |
| `vscreen_wait_for_idle` | Wait until page is idle (readyState=complete) |
| `vscreen_wait_for_text` | Wait until text appears on page |
| `vscreen_wait_for_selector` | Wait until CSS selector matches |
| `vscreen_wait_for_url` | Wait until URL contains substring |
| `vscreen_wait_for_network_idle` | Wait until no pending network requests |

#### Memory / context

| Tool | Description |
|------|-------------|
| `vscreen_history_list` | List screenshot history metadata |
| `vscreen_history_get` | Get a historical screenshot by index |
| `vscreen_history_get_range` | Get a range of historical screenshots |
| `vscreen_history_clear` | Clear screenshot history |
| `vscreen_session_log` | Get action session log (all MCP actions taken) |
| `vscreen_session_summary` | Get condensed session summary |

#### Console capture

| Tool | Description |
|------|-------------|
| `vscreen_console_log` | Get captured console messages (log/warn/error) |
| `vscreen_console_clear` | Clear console buffer |

#### Cookie & storage

| Tool | Description |
|------|-------------|
| `vscreen_get_cookies` | Get all cookies for current page |
| `vscreen_set_cookie` | Set a cookie |
| `vscreen_get_storage` | Read localStorage/sessionStorage |
| `vscreen_set_storage` | Write to localStorage/sessionStorage |

#### Page interaction

| Tool | Description |
|------|-------------|
| `vscreen_execute_js` | Execute arbitrary JavaScript and return result |
| `vscreen_dismiss_dialogs` | Dismiss cookie consent, GDPR, and similar overlays |
| `vscreen_dismiss_ads` | Dismiss video ad overlays (e.g. YouTube skip button) |
| `vscreen_solve_captcha` | Automatically solve reCAPTCHA v2 image challenges (requires vision LLM) |

#### Audio / RTSP

| Tool | Description |
|------|-------------|
| `vscreen_audio_streams` | List active RTSP audio sessions |
| `vscreen_audio_stream_info` | Get info and health for a specific audio stream |
| `vscreen_audio_health` | Get audio subsystem health |
| `vscreen_rtsp_teardown` | Force-teardown an RTSP session |

#### Self-documentation

| Tool | Description |
|------|-------------|
| `vscreen_plan` | Get recommended tool sequence for a task |
| `vscreen_help` | Get contextual help on tools, workflows, and concepts |

### Recommended AI workflow

The recommended flow for AI browser automation is:

1. **Navigate** — `vscreen_navigate` to the target URL
2. **Wait** — `vscreen_wait_for_idle` or `vscreen_wait` for the page to load
3. **Observe** — `vscreen_screenshot` (or with `full_page=true`) to see the page
4. **Discover** — `vscreen_find_elements` or `vscreen_find_by_text` to locate targets
5. **Act** — `vscreen_click`, `vscreen_type`, `vscreen_key_press`
6. **Verify** — `vscreen_wait` then `vscreen_screenshot` to confirm the result
7. **Repeat** — continue until the task is complete

Use `vscreen_session_log` to review what actions have been taken, and `vscreen_history_get` to look back at earlier screenshots.

### Full-page screenshots and coordinate translation

When `full_page=true` is set on `vscreen_screenshot`, the system temporarily resizes the browser viewport to the full document height and captures the entire page in a single image. This can produce images much taller than 1080px.

All click/hover/scroll tools accept **page-level coordinates** from full-page screenshots. The system automatically scrolls the page to bring the target into view before dispatching the input event — no manual coordinate conversion needed.

### Iframe handling

Cross-origin iframes (e.g., reCAPTCHA, embedded widgets) require special handling:

- Use `vscreen_list_frames` to discover all iframes and their bounding rectangles
- Use `vscreen_find_elements` or `vscreen_find_by_text` with `include_iframes=true` to search inside iframes
- Returned coordinates are already translated to page-space and can be passed directly to `vscreen_click`

---

## CLI Reference

```
vscreen — Virtual Screen Media Bridge

Usage: vscreen [OPTIONS]

Options:
  -c, --config <CONFIG>              Path to a TOML configuration file
  -l, --listen <LISTEN>              Listen address [default: 0.0.0.0:8450]
      --log-level <LOG_LEVEL>        Log level [default: info]
      --log-json                     Output structured JSON logs
      --dev                          Start in dev mode (Xvfb + PulseAudio + Chromium)
      --dev-url <DEV_URL>            Navigate Chromium to this URL on startup
      --dev-display <DEV_DISPLAY>    X11 display number for dev mode [default: 99]
      --dev-cdp-port <PORT>          CDP debugging port for Chromium [default: 9222]
      --mcp-stdio                    Start MCP server on stdin/stdout
      --mcp-sse <ADDR>               Start MCP SSE server on the given address
      --mcp-stdio-proxy <URL>        Stdio proxy to an existing SSE MCP server
      --mcp-single-agent             Bypass lock checks (single-agent mode)
      --rtsp-port <PORT>             RTSP audio server port [default: 8554]
      --no-rtsp                      Disable RTSP audio server
      --video-codec <CODEC>          Video codec: h264 (default) or vp9 [default: h264]
      --vision-url <URL>             Vision LLM URL for element identification (Ollama/OpenAI-compatible)
      --vision-model <MODEL>         Vision model name [default: qwen3-vl:8b]
  -h, --help                         Print help
  -V, --version                      Print version
```

### Environment variables

| Variable | Equivalent flag |
|----------|----------------|
| `VSCREEN_CONFIG` | `--config` |
| `VSCREEN_LISTEN` | `--listen` |
| `VSCREEN_LOG_LEVEL` | `--log-level` |
| `VSCREEN_LOG_JSON` | `--log-json` |
| `VSCREEN_VISION_URL` | `--vision-url` |
| `VSCREEN_VISION_MODEL` | `--vision-model` |

---

## Configuration

vscreen uses layered configuration: defaults → TOML file → environment variables → CLI flags.

### Example `vscreen.toml`

```toml
[server]
listen = "0.0.0.0:8450"
# auth_token = "my-secret-token"  # Uncomment to enable bearer token auth

# [server.tls]  # Uncomment for built-in TLS
# cert_path = "/path/to/cert.pem"
# key_path = "/path/to/key.pem"

[webrtc]
stun_servers = ["stun:stun.l.google.com:19302"]

[defaults.video]
width = 1920
height = 1080
framerate = 30
bitrate_kbps = 4000
keyframe_interval = 60
cpu_used = 6
codec = "h264"  # or "vp9"

[defaults.audio]
sample_rate = 48000
channels = 2
bitrate_kbps = 128
frame_duration_ms = 20

[limits]
max_instances = 16
max_peers_per_instance = 8
frame_queue_depth = 3
max_frame_size = 2097152

[logging]
level = "info"
json = false
```

### Authentication

Set `auth_token` in the config file to enable bearer token authentication:

```toml
[server]
auth_token = "my-secret-token"
```

- HTTP requests: include `Authorization: Bearer my-secret-token` header
- WebSocket connections: append `?token=my-secret-token` to the URL
- The `/health` endpoint is exempt from authentication

---

## REST API

All endpoints accept and return JSON. The server listens on `0.0.0.0:8450` by default.

### Server

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/health` | Server health check |
| GET | `/metrics` | Prometheus metrics |

### Instance CRUD

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/instances` | Create a new instance |
| GET | `/instances` | List all instances |
| DELETE | `/instances/{id}` | Delete an instance |
| GET | `/instances/{id}/health` | Instance health |
| PATCH | `/instances/{id}/video` | Update video config at runtime |
| POST | `/instances/{id}/navigate` | Navigate to a URL |
| GET | `/instances/{id}/sdp` | Get RTP audio SDP descriptor |

### Screenshot & observation

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/instances/{id}/screenshot` | Capture screenshot (`?format=png&quality=80&full_page=true`) |
| POST | `/instances/{id}/screenshot/sequence` | Capture screenshot sequence |
| GET | `/instances/{id}/page` | Get page info (URL, title, viewport) |
| GET | `/instances/{id}/cursor` | Get cursor position |

### Input

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/instances/{id}/input` | Dispatch raw input event |
| POST | `/instances/{id}/input/click` | Click at coordinates |
| POST | `/instances/{id}/input/type` | Type text |
| POST | `/instances/{id}/input/key` | Press a key |
| POST | `/instances/{id}/input/scroll` | Scroll |
| POST | `/instances/{id}/input/drag` | Click-drag |
| POST | `/instances/{id}/input/hover` | Move mouse |

### Element discovery & navigation

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/instances/{id}/find` | Find elements by CSS selector |
| POST | `/instances/{id}/extract-text` | Extract visible text |
| POST | `/instances/{id}/exec` | Execute JavaScript |
| POST | `/instances/{id}/go-back` | Navigate back |
| POST | `/instances/{id}/go-forward` | Navigate forward |
| POST | `/instances/{id}/reload` | Reload page |

### Memory & context

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/instances/{id}/history` | Screenshot history metadata |
| GET | `/instances/{id}/history/{index}` | Get historical screenshot image |
| DELETE | `/instances/{id}/history` | Clear screenshot history |
| GET | `/instances/{id}/session` | Action session log |
| GET | `/instances/{id}/session/summary` | Session summary |
| GET | `/instances/{id}/console` | Console messages (`?level=error`) |
| DELETE | `/instances/{id}/console` | Clear console buffer |

### Audio / RTSP

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/instances/{id}/audio/streams` | List RTSP audio sessions for an instance |
| GET | `/instances/{id}/audio/streams/{session_id}` | Get stream info and health |
| DELETE | `/instances/{id}/audio/streams/{session_id}` | Force-teardown a session |
| GET | `/instances/{id}/audio/health` | Audio subsystem health |
| GET | `/rtsp/sessions` | List all RTSP sessions (global) |
| GET | `/rtsp/health` | RTSP server health |

### WebRTC signaling

```
WS /signal/{instance_id}
```

The WebSocket carries JSON signaling messages: `offer`, `answer`, `ice_candidate`, `ice_complete`, `connected`, `disconnected`, `error`.

---

## Docker

### Build and run

```bash
docker build -t vscreen .
docker run -p 8450:8450 vscreen
```

### With docker-compose

```bash
docker-compose up
```

### Custom start URL

```bash
docker run -p 8450:8450 vscreen --dev-url "https://www.youtube.com"
```

### With MCP SSE enabled

```bash
docker run -p 8450:8450 -p 8451:8451 vscreen --mcp-sse 0.0.0.0:8451
```

### With RTSP audio exposed

```bash
docker run -p 8450:8450 -p 8554:8554 vscreen
```

### With RTSP disabled

```bash
docker run -p 8450:8450 vscreen --no-rtsp
```

---

## Project Structure

```
vscreen/
├── crates/
│   ├── vscreen/              # Binary entry point, dev mode, CLI
│   ├── vscreen-core/         # Shared types, config, errors, traits
│   ├── vscreen-cdp/          # Chrome DevTools Protocol client
│   ├── vscreen-video/        # JPEG decode → RGB → I420 → H264/VP9
│   ├── vscreen-audio/        # PulseAudio capture → Opus encode
│   ├── vscreen-transport/    # WebRTC sessions, RTP sender
│   ├── vscreen-rtsp/         # RTSP audio server, session management
│   │   └── src/
│   │       ├── server.rs     # RTSP listener and connection handler
│   │       ├── session.rs    # Session lifecycle management
│   │       ├── handler.rs    # RTSP method handlers (DESCRIBE, SETUP, PLAY, TEARDOWN)
│   │       ├── parser.rs     # RTSP protocol parser
│   │       ├── sdp.rs        # SDP generation
│   │       ├── transport.rs  # RTP unicast transport
│   │       ├── transcoder.rs # Audio transcoding
│   │       ├── h264_packetizer.rs  # H.264 RTP packetizer (RFC 6184 FU-A)
│   │       ├── vp9_packetizer.rs   # VP9 RTP packetizer
│   │       ├── quality.rs    # Quality tier management
│   │       └── health.rs     # Health monitoring and watchdog
│   └── vscreen-server/       # HTTP/WS API, MCP server, instance supervisor
│       └── src/
│           ├── handlers.rs   # REST API handlers
│           ├── mcp.rs        # MCP server (63 tools)
│           ├── memory.rs     # Screenshot history, action log, console buffer
│           ├── supervisor.rs # Instance pipeline orchestration
│           ├── router.rs     # Route registration
│           ├── state.rs      # Shared application state
│           ├── ws.rs         # WebSocket signaling
│           ├── middleware.rs # Auth, logging middleware
│           ├── metrics.rs    # Prometheus metrics
│           ├── lock_manager.rs  # Instance ownership and locking
│           └── vision.rs     # Vision LLM client for element identification
├── tools/
│   ├── test-client/          # Browser-based WebRTC test client (Vite + TS)
│   └── integration/          # E2E integration tests (Vitest + Playwright)
├── docs/
│   ├── architecture/         # Detailed architecture documentation
│   ├── discovery/            # Original design and discovery documents
│   ├── deployment.md         # Deployment guide
│   └── dev-mode.md           # Dev mode guide
├── benches/                  # Audio and video benchmarks (Criterion)
├── fuzz/                     # Fuzz testing targets (cargo-fuzz)
├── scripts/                  # Build and deployment scripts
├── Dockerfile                # Multi-stage Docker build
└── docker-compose.yml        # Docker Compose config
```

---

## Building from Source

```bash
# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install system dependencies (Debian/Ubuntu)
sudo apt install -y \
  build-essential cmake pkg-config libssl-dev libclang-dev \
  libvpx-dev libopus-dev libpulse-dev

# Build
cargo build --release -p vscreen --features pulse-audio

# Run tests (450+ tests across 8 crates)
cargo test --workspace
```

### Build without PulseAudio

For CI environments or systems without PulseAudio, omit the feature flag:

```bash
cargo build --release -p vscreen
cargo test --workspace
```

### Build with TLS support

```bash
cargo build --release -p vscreen --features "pulse-audio,tls"
```

---

## Architecture Documentation

Detailed architecture documents are in `docs/`:

- [Project Structure](docs/architecture/project-structure.md) — crate layout, module boundaries
- [Concurrency & Safety](docs/architecture/concurrency-safety.md) — race conditions, deadlock prevention
- [Error Handling](docs/architecture/error-handling.md) — error hierarchy, recovery strategies
- [Testing Strategy](docs/architecture/testing-strategy.md) — test plan across all levels
- [Build & Tooling](docs/architecture/build-and-tooling.md) — CI pipeline, dev scripts
- [Dev Mode](docs/dev-mode.md) — dev environment setup and usage
- [Deployment](docs/deployment.md) — production deployment guide

---

## License

Source-Available Non-Commercial — see [LICENSE](LICENSE).

Copyright (c) 2025–2026 Jonathan Retting. All rights reserved.

You may download and use this software for personal, educational, or research purposes. Commercial use, redistribution, and derivative works are prohibited without explicit permission from the author.
