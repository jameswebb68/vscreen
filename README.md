# vscreen — Virtual Screen Media Bridge

**Give AI agents a real browser. Watch them live. Control everything.**

> **[Download the latest release](https://github.com/jameswebb68/vscreen/releases/latest)** — pre-built binaries for Linux and Windows.

vscreen turns a headless Chromium into a remotely viewable, controllable, and AI-automatable virtual screen. It captures the browser viewport via Chrome DevTools Protocol, encodes H.264/VP9 video + Opus audio, and streams everything over WebRTC. Clients send mouse and keyboard input back through a DataChannel for full bidirectional interaction. 63 MCP tools let AI agents automate the browser programmatically.

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

---

## Table of Contents

- [Features](#features)
- [Quick Start](#quick-start-dev-mode)
- [Test Client](#connect-the-test-client)
- [MCP Server (AI Automation)](#mcp-server-ai-automation)
  - [Connection Modes](#connection-modes)
  - [Cursor IDE Integration](#cursor-ide-integration)
  - [Instance Locking](#instance-locking)
  - [MCP Tool Reference](#mcp-tool-reference)
  - [Recommended AI Workflow](#recommended-ai-workflow)
- [CLI Reference](#cli-reference)
- [Configuration](#configuration)
- [REST API](#rest-api)
- [Docker](#docker)
- [Building from Source](#building-from-source)
- [Code Quality](#code-quality)
- [Project Structure](#project-structure)
- [Architecture Documentation](#architecture-documentation)
- [License](#license)

---

## Features

**Video & Audio**
- **H.264** (default) via OpenH264 or **VP9** — selectable with `--video-codec`
- **Opus audio** from PulseAudio (48 kHz stereo, 128 kbps)
- **WebRTC delivery** with ICE/STUN negotiation and adaptive bitrate
- **RTSP audio** — pull-based streaming for VLC, GStreamer, FFmpeg

**Browser Control**
- **Bidirectional input** — mouse, keyboard, scroll, drag, right-click, clipboard
- **Element discovery** — CSS selectors, visible text, accessibility tree, input attributes
- **Full-page screenshots** with automatic coordinate translation
- **Iframe support** — cross-origin element discovery and click actions
- **Smart sync** — wait for text, selector, URL change, or network idle

**AI Automation**
- **63 MCP tools** via stdio, HTTP/SSE, or stdio-proxy transport
- **Instance locking** — lease-based exclusive/observer modes for multi-agent coordination
- **Annotated screenshots** — numbered bounding boxes on interactive elements
- **Vision LLM integration** — identify unlabeled UI elements, solve reCAPTCHAs
- **Dialog/ad dismissal** — cookie consent, GDPR overlays, video ads
- **Screenshot history** — ring buffer of last 20 with metadata for AI backtracking
- **Action session log** — timestamped history of all actions taken

**Infrastructure**
- **REST API** — full programmatic instance management
- **Dev mode** — one command spawns Xvfb + PulseAudio + Chromium
- **Docker** — multi-stage Dockerfile and docker-compose
- **Bearer token auth** — optional, via header or query param
- **Prometheus metrics** — `/metrics` endpoint
- **Test client** — built-in browser UI with URL bar, fullscreen, stats HUD

---

## Quick Start (Dev Mode)

Dev mode spawns a virtual X11 display, PulseAudio sink, and Chromium instance automatically.

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
cargo build --release -p vscreen --features pulse-audio

# Start in dev mode (H.264 by default)
./target/release/vscreen --dev

# VP9 video instead
./target/release/vscreen --dev --video-codec vp9

# Pre-load a URL
./target/release/vscreen --dev --dev-url "https://www.youtube.com"

# With MCP server (SSE, recommended)
./target/release/vscreen --dev --mcp-sse 0.0.0.0:8451

# With MCP server (stdio)
./target/release/vscreen --dev --mcp-stdio

# Disable RTSP audio
./target/release/vscreen --dev --no-rtsp
```

### What dev mode does

1. **Starts Xvfb** — virtual X11 display (`:99`) at 1920x1080x24
2. **Creates a PulseAudio null-sink** — `vscreen_dev_99` for audio capture
3. **Launches Chromium** — headless, remote debugging on port 9222
4. **Connects via CDP** — screencast capture begins immediately
5. **Starts HTTP/WS server** — `0.0.0.0:8450`
6. **Starts RTSP server** — port `8554` (disable with `--no-rtsp`)
7. **Creates the `dev` instance** — ready for WebRTC and API calls

The dev instance ID is always `"dev"`.

### Connect the test client

```bash
cd tools/test-client
pnpm install && pnpm dev
```

Open `http://localhost:5173`, click **Connect**, and you're in. Vite proxies everything to the backend on port 8450.

**Test client features:** video/audio playback, full mouse + keyboard, clipboard sync, URL bar, fullscreen, stats HUD (F2), auto-reconnect.

### Receive audio externally

```bash
ffplay rtsp://localhost:8554/audio/dev        # FFmpeg
vlc rtsp://localhost:8554/audio/dev           # VLC
gst-launch-1.0 rtspsrc location=rtsp://localhost:8554/audio/dev ! decodebin ! autoaudiosink
```

---

## MCP Server (AI Automation)

vscreen includes a built-in MCP server with 63 tools for AI browser automation — navigate, screenshot, click, type, read content, solve CAPTCHAs, and more.

### Connection modes

| Mode | Flag | Best for |
|------|------|----------|
| **SSE** (recommended) | `--mcp-sse 0.0.0.0:8451` | Cursor IDE, persistent connections |
| **Stdio proxy** | `--mcp-stdio-proxy http://host:8451/mcp` | Clients that only support subprocess spawning |
| **Stdio direct** | `--mcp-stdio` | When the client spawns vscreen as a subprocess |

**Best practice:** Start the server once with `--dev --mcp-sse`, then connect via SSE URL or stdio proxy.

### Cursor IDE integration

**SSE (recommended):**

```json
{
  "mcpServers": {
    "vscreen": {
      "url": "http://localhost:8451/mcp"
    }
  }
}
```

<details>
<summary>Stdio and stdio proxy configs</summary>

**Stdio direct:**

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

**Stdio proxy to existing SSE server:**

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

</details>

### Instance locking

When multiple AI agents share a vscreen instance, use locking to prevent conflicts:

- **Exclusive lock** — only the lock holder can perform actions
- **Observer lock** — read-only access (screenshots, element queries)
- Locks have a TTL and must be renewed via heartbeat
- Single-agent mode (`--mcp-single-agent`) bypasses lock checks

### MCP tool reference

63 tools organized by category. Click to expand each group.

<details>
<summary><strong>Instance management</strong> (5 tools)</summary>

| Tool | Description |
|------|-------------|
| `vscreen_list_instances` | List all browser instances and their states |
| `vscreen_instance_lock` | Acquire exclusive or observer lock on an instance |
| `vscreen_instance_unlock` | Release lock on an instance |
| `vscreen_instance_lock_status` | Query lock status for one or all instances |
| `vscreen_instance_lock_renew` | Extend lock TTL (heartbeat) |

</details>

<details>
<summary><strong>Observation</strong> (6 tools)</summary>

| Tool | Description |
|------|-------------|
| `vscreen_screenshot` | Capture viewport or full-page screenshot (PNG/JPEG/WebP) with optional clip region |
| `vscreen_screenshot_sequence` | Capture N screenshots at fixed interval |
| `vscreen_screenshot_annotated` | Screenshot with numbered bounding boxes on interactive elements |
| `vscreen_get_page_info` | Get current URL, title, and viewport dimensions |
| `vscreen_get_cursor_position` | Get last known mouse cursor position |
| `vscreen_extract_text` | Extract visible text from page or specific element |

</details>

<details>
<summary><strong>Input actions</strong> (13 tools)</summary>

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

</details>

<details>
<summary><strong>Navigation</strong> (5 tools)</summary>

| Tool | Description |
|------|-------------|
| `vscreen_navigate` | Navigate to a URL (with `wait_until` option) |
| `vscreen_go_back` | Browser back button |
| `vscreen_go_forward` | Browser forward button |
| `vscreen_reload` | Reload the current page |
| `vscreen_click_and_navigate` | Click element and wait for URL change |

</details>

<details>
<summary><strong>Element discovery</strong> (6 tools)</summary>

| Tool | Description |
|------|-------------|
| `vscreen_find_elements` | Find elements by CSS selector with bounding boxes (supports `include_iframes`) |
| `vscreen_find_by_text` | Find elements by visible text content (supports `include_iframes`) |
| `vscreen_find_input` | Find text inputs by placeholder, aria-label, label, role, or name |
| `vscreen_accessibility_tree` | Get structured accessibility tree |
| `vscreen_describe_elements` | Identify unlabeled UI elements using vision LLM |
| `vscreen_list_frames` | List frames/iframes with bounding rectangles |

</details>

<details>
<summary><strong>Synchronization</strong> (6 tools)</summary>

| Tool | Description |
|------|-------------|
| `vscreen_wait` | Wait for a specified duration |
| `vscreen_wait_for_idle` | Wait until page is idle (readyState=complete) |
| `vscreen_wait_for_text` | Wait until text appears on page |
| `vscreen_wait_for_selector` | Wait until CSS selector matches |
| `vscreen_wait_for_url` | Wait until URL contains substring |
| `vscreen_wait_for_network_idle` | Wait until no pending network requests |

</details>

<details>
<summary><strong>Memory & context</strong> (6 tools)</summary>

| Tool | Description |
|------|-------------|
| `vscreen_history_list` | List screenshot history metadata |
| `vscreen_history_get` | Get a historical screenshot by index |
| `vscreen_history_get_range` | Get a range of historical screenshots |
| `vscreen_history_clear` | Clear screenshot history |
| `vscreen_session_log` | Get action session log (all MCP actions taken) |
| `vscreen_session_summary` | Get condensed session summary |

</details>

<details>
<summary><strong>Console capture</strong> (2 tools)</summary>

| Tool | Description |
|------|-------------|
| `vscreen_console_log` | Get captured console messages (log/warn/error) |
| `vscreen_console_clear` | Clear console buffer |

</details>

<details>
<summary><strong>Cookie & storage</strong> (4 tools)</summary>

| Tool | Description |
|------|-------------|
| `vscreen_get_cookies` | Get all cookies for current page |
| `vscreen_set_cookie` | Set a cookie |
| `vscreen_get_storage` | Read localStorage/sessionStorage |
| `vscreen_set_storage` | Write to localStorage/sessionStorage |

</details>

<details>
<summary><strong>Page interaction</strong> (4 tools)</summary>

| Tool | Description |
|------|-------------|
| `vscreen_execute_js` | Execute arbitrary JavaScript and return result |
| `vscreen_dismiss_dialogs` | Dismiss cookie consent, GDPR, and similar overlays |
| `vscreen_dismiss_ads` | Dismiss video ad overlays (e.g. YouTube skip button) |
| `vscreen_solve_captcha` | Automatically solve reCAPTCHA v2 image challenges (requires vision LLM) |

</details>

<details>
<summary><strong>Audio / RTSP</strong> (4 tools)</summary>

| Tool | Description |
|------|-------------|
| `vscreen_audio_streams` | List active RTSP audio sessions |
| `vscreen_audio_stream_info` | Get info and health for a specific audio stream |
| `vscreen_audio_health` | Get audio subsystem health |
| `vscreen_rtsp_teardown` | Force-teardown an RTSP session |

</details>

<details>
<summary><strong>Self-documentation</strong> (2 tools)</summary>

| Tool | Description |
|------|-------------|
| `vscreen_plan` | Get recommended tool sequence for a task |
| `vscreen_help` | Get contextual help on tools, workflows, and concepts |

</details>

### Recommended AI workflow

1. **Navigate** — `vscreen_navigate` to the target URL
2. **Wait** — `vscreen_wait_for_idle` for the page to load
3. **Observe** — `vscreen_screenshot` to see the page
4. **Discover** — `vscreen_find_elements` or `vscreen_find_by_text` to locate targets
5. **Act** — `vscreen_click`, `vscreen_type`, `vscreen_key_press`
6. **Verify** — `vscreen_screenshot` to confirm the result
7. **Repeat**

Use `vscreen_session_log` to review actions taken and `vscreen_history_get` to revisit earlier screenshots.

### Full-page screenshots and coordinate translation

When `full_page=true`, the viewport is temporarily resized to the full document height, capturing the entire page in one image. All click/hover/scroll tools accept these page-level coordinates and automatically scroll to the target before dispatching input.

### Iframe handling

- `vscreen_list_frames` — discover iframes and their bounding rectangles
- `vscreen_find_elements` / `vscreen_find_by_text` with `include_iframes=true` — search inside iframes
- Returned coordinates are page-space and work directly with `vscreen_click`

---

## CLI Reference

```
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
      --vision-url <URL>             Vision LLM URL (Ollama/OpenAI-compatible)
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

Layered: defaults → TOML file → environment variables → CLI flags.

<details>
<summary><strong>Example vscreen.toml</strong></summary>

```toml
[server]
listen = "0.0.0.0:8450"
# auth_token = "my-secret-token"

# [server.tls]
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
codec = "h264"

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

</details>

### Authentication

Set `auth_token` in the config file:

```toml
[server]
auth_token = "my-secret-token"
```

- HTTP: `Authorization: Bearer my-secret-token`
- WebSocket: `?token=my-secret-token`
- `/health` is exempt

---

## REST API

All endpoints accept and return JSON. Default: `0.0.0.0:8450`.

<details>
<summary><strong>Server</strong></summary>

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/health` | Server health check |
| GET | `/metrics` | Prometheus metrics |

</details>

<details>
<summary><strong>Instance CRUD</strong></summary>

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/instances` | Create a new instance |
| GET | `/instances` | List all instances |
| DELETE | `/instances/{id}` | Delete an instance |
| GET | `/instances/{id}/health` | Instance health |
| PATCH | `/instances/{id}/video` | Update video config at runtime |
| POST | `/instances/{id}/navigate` | Navigate to a URL |
| GET | `/instances/{id}/sdp` | Get RTP audio SDP descriptor |

</details>

<details>
<summary><strong>Screenshot & observation</strong></summary>

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/instances/{id}/screenshot` | Capture screenshot (`?format=png&quality=80&full_page=true`) |
| POST | `/instances/{id}/screenshot/sequence` | Capture screenshot sequence |
| GET | `/instances/{id}/page` | Get page info (URL, title, viewport) |
| GET | `/instances/{id}/cursor` | Get cursor position |

</details>

<details>
<summary><strong>Input</strong></summary>

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/instances/{id}/input` | Dispatch raw input event |
| POST | `/instances/{id}/input/click` | Click at coordinates |
| POST | `/instances/{id}/input/type` | Type text |
| POST | `/instances/{id}/input/key` | Press a key |
| POST | `/instances/{id}/input/scroll` | Scroll |
| POST | `/instances/{id}/input/drag` | Click-drag |
| POST | `/instances/{id}/input/hover` | Move mouse |

</details>

<details>
<summary><strong>Element discovery & navigation</strong></summary>

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/instances/{id}/find` | Find elements by CSS selector |
| POST | `/instances/{id}/extract-text` | Extract visible text |
| POST | `/instances/{id}/exec` | Execute JavaScript |
| POST | `/instances/{id}/go-back` | Navigate back |
| POST | `/instances/{id}/go-forward` | Navigate forward |
| POST | `/instances/{id}/reload` | Reload page |

</details>

<details>
<summary><strong>Memory & context</strong></summary>

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/instances/{id}/history` | Screenshot history metadata |
| GET | `/instances/{id}/history/{index}` | Get historical screenshot image |
| DELETE | `/instances/{id}/history` | Clear screenshot history |
| GET | `/instances/{id}/session` | Action session log |
| GET | `/instances/{id}/session/summary` | Session summary |
| GET | `/instances/{id}/console` | Console messages (`?level=error`) |
| DELETE | `/instances/{id}/console` | Clear console buffer |

</details>

<details>
<summary><strong>Audio / RTSP</strong></summary>

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/instances/{id}/audio/streams` | List RTSP audio sessions |
| GET | `/instances/{id}/audio/streams/{session_id}` | Stream info and health |
| DELETE | `/instances/{id}/audio/streams/{session_id}` | Force-teardown a session |
| GET | `/instances/{id}/audio/health` | Audio subsystem health |
| GET | `/rtsp/sessions` | List all RTSP sessions (global) |
| GET | `/rtsp/health` | RTSP server health |

</details>

### WebRTC signaling

```
WS /signal/{instance_id}
```

JSON messages: `offer`, `answer`, `ice_candidate`, `ice_complete`, `connected`, `disconnected`, `error`.

---

## Docker

```bash
# Build and run
docker build -t vscreen .
docker run -p 8450:8450 vscreen

# docker-compose
docker-compose up

# Custom start URL
docker run -p 8450:8450 vscreen --dev-url "https://www.youtube.com"

# With MCP SSE
docker run -p 8450:8450 -p 8451:8451 vscreen --mcp-sse 0.0.0.0:8451

# With RTSP audio exposed
docker run -p 8450:8450 -p 8554:8554 vscreen

# Without RTSP
docker run -p 8450:8450 vscreen --no-rtsp
```

---

## Building from Source

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# System dependencies (Debian/Ubuntu)
sudo apt install -y \
  build-essential cmake pkg-config libssl-dev libclang-dev \
  libvpx-dev libopus-dev libpulse-dev

# Build
cargo build --release -p vscreen --features pulse-audio

# Run tests (510+ tests across 8 crates)
cargo test --workspace
```

<details>
<summary>Build variants</summary>

**Without PulseAudio** (CI environments):

```bash
cargo build --release -p vscreen
```

**With TLS support:**

```bash
cargo build --release -p vscreen --features "pulse-audio,tls"
```

</details>

---

## Code Quality

~31,000 lines of Rust with strict quality enforcement.

| Metric | Value |
|--------|-------|
| Unit tests | 380+ |
| Async tests (`tokio::test`) | 130+ |
| Integration test suites | 9 |
| Fuzz targets | 3 |
| Criterion benchmarks | 5 |
| Source files | 71 `.rs` files across 8 crates |

### Compiler and lint enforcement

| Rule | Level |
|------|-------|
| `unsafe_code` | **forbid** (workspace-wide; `warn` in `vscreen-video` for codec FFI) |
| `unwrap_used` | **deny** — all error paths handled explicitly |
| `panic` | **deny** — graceful error propagation everywhere |
| `clippy::pedantic` + `clippy::nursery` | **warn** |
| `dbg_macro`, `print_stdout`, `print_stderr` | **deny** — structured `tracing` only |
| Cognitive complexity threshold | **15** |

### Supply chain security

[`cargo-deny`](https://github.com/EmbarkStudios/cargo-deny) enforces:

| Check | Policy |
|-------|--------|
| Known vulnerabilities | **denied** |
| Yanked crates | **denied** |
| Unknown registries | **denied** |
| Unknown git sources | **denied** |
| Dependency licenses | Only MIT, Apache-2.0, BSD, ISC, Zlib |
| Wildcard dependencies | **denied** |

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
│   └── vscreen-server/       # HTTP/WS API, MCP server (63 tools), supervisor
├── tools/
│   ├── test-client/          # Browser-based WebRTC test client (Vite + TS)
│   └── integration/          # E2E integration tests (Vitest + Playwright)
├── docs/                     # Architecture, deployment, dev mode docs
├── benches/                  # Audio and video benchmarks (Criterion)
├── fuzz/                     # Fuzz testing targets (cargo-fuzz)
├── scripts/                  # Build, deployment, and fixture scripts
├── Dockerfile                # Multi-stage Docker build
└── docker-compose.yml
```

---

## Architecture Documentation

- [Project Structure](docs/architecture/project-structure.md) — crate layout, module boundaries
- [Concurrency & Safety](docs/architecture/concurrency-safety.md) — race conditions, deadlock prevention
- [Error Handling](docs/architecture/error-handling.md) — error hierarchy, recovery strategies
- [Testing Strategy](docs/architecture/testing-strategy.md) — test plan across all levels
- [Build & Tooling](docs/architecture/build-and-tooling.md) — CI pipeline, dev scripts
- [Dev Mode](docs/dev-mode.md) — dev environment setup and usage
- [Deployment](docs/deployment.md) — production deployment guide

---

## License

**Source-Available Non-Commercial** — see [LICENSE](LICENSE).

Copyright (c) 2025–2026 Jonathan Retting. All rights reserved.

You may download and use this software for personal, educational, or research purposes. Commercial use, redistribution, and derivative works are prohibited without explicit permission from the author.
