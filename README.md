# vscreen — Virtual Screen Media Bridge

**Give AI agents a real browser. Watch them live. Control everything.**

> **[Download the latest release](https://github.com/jameswebb68/vscreen/releases/latest)** — pre-built binaries for Linux.

vscreen turns a headless Chromium into a remotely viewable, controllable, and AI-automatable virtual screen. It captures the browser viewport via Chrome DevTools Protocol, encodes H.264/VP9 video + Opus audio, and streams everything over WebRTC. Clients send mouse and keyboard input back through a DataChannel for full bidirectional interaction. 47 MCP tools let AI agents automate the browser programmatically — including the **Synthesis Bubble** system for AI-driven frontend page construction with one-shot multi-source web scraping.

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
                     External consumers          Synthesis Bubble
                     (GStreamer, FFmpeg, VLC)     ┌─────────────────┐
                                                 │ SvelteKit 5 dev │
                                                 │ server (HTTPS)  │
                                                 │ AI-built pages  │
                                                 │ 31 components   │
                                                 └─────────────────┘
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
- [Synthesis Bubble](#synthesis-bubble)
  - [Quick Start](#synthesis-quick-start)
  - [How It Works](#how-it-works)
  - [One-Shot Scrape and Create](#one-shot-scrape-and-create)
  - [Component Library](#component-library)
  - [Component Aliases](#component-aliases)
  - [Synthesis MCP Tools](#synthesis-mcp-tools)
  - [Persistence](#persistence)
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
- **47 MCP tools** via stdio, HTTP/SSE, or stdio-proxy transport — consolidated from granular endpoints into powerful multi-mode tools
- **High-level workflow tools** — `vscreen_browse`, `vscreen_observe`, `vscreen_extract`, `vscreen_interact` combine multiple steps into single calls
- **Instance locking** — lease-based exclusive/observer modes for multi-agent coordination
- **Annotated screenshots** — numbered bounding boxes on interactive elements
- **Vision LLM integration** — identify unlabeled UI elements, solve reCAPTCHAs, detect overlays
- **Screenshot watcher** — background perceptual hash grid monitors page regions for visual changes
- **Dialog/ad dismissal** — cookie consent, GDPR overlays, video ads
- **Screenshot history** — ring buffer of last 20 with metadata for AI backtracking
- **Action session log** — timestamped history of all actions taken
- **CDP tab multiplexing** — ephemeral browser tabs for parallel operations without disturbing the main tab

**Synthesis Bubble** — AI-driven frontend page construction
- **31 Svelte 5 components** — cards, tables, charts, timelines, code blocks, and more
- **One-shot scrape & create** — `vscreen_synthesis_scrape_and_create` scrapes multiple sites in parallel, creates a page, and navigates to it — all in a single MCP call
- **Batch scraping** — `vscreen_synthesis_scrape_batch` scrapes multiple URLs concurrently using ephemeral CDP tabs
- **Intelligent scraper** — standalone JS engine with JSON-LD extraction, locked image authority, ad filtering, content quality scoring, and timeout budget management
- **Component auto-selection** — automatically picks `hero`, `card-grid`, or `content-list` based on article count per section
- **Component aliases** — intuitive names like `articles`, `chart`, `table` resolve to canonical component types
- **Zod runtime validation** — section data validated per-component type at the API boundary with clear error messages
- **Error boundaries** — per-section `<svelte:boundary>` wrappers prevent a single bad section from killing the page
- **Component registry** — map-based lookup replaces brittle if-else chains, eliminating Svelte 5 hydration bugs
- **Real-time updates** — Server-Sent Events push data changes to pages live
- **Progressive rendering** — pages build live as each source finishes scraping via SSE
- **HTTPS frontend** — SvelteKit 5 dev server on `0.0.0.0:5174`, accessible from any browser on the network
- **Persistence** — save/load synthesized pages to disk across server restarts
- **Multiple layouts** — `grid`, `list`, `tabs`, and `split` layouts

**Browser Stealth & Isolation**
- **Anti-detection** — `navigator.webdriver` removal, `AutomationControlled` blink feature disabled, `window.chrome` runtime injection
- **Fingerprint spoofing** — realistic plugins, languages, permissions, WebGL renderer masking (Intel GPU strings)
- **Per-instance isolation** — dedicated Xvfb display, unique Chromium `--user-data-dir`, separate PulseAudio null-sink

**Infrastructure**
- **Process group management** — child processes (Xvfb, Chromium) spawned in isolated process groups with `SIGTERM`/`SIGKILL` group cleanup on shutdown
- **Graceful shutdown** — handles `SIGINT`, `SIGTERM`, and `SIGHUP` for clean process tree teardown
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
3. **Launches Chromium** — headless, remote debugging on port 9222 (in its own process group for clean shutdown)
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

vscreen includes a built-in MCP server with 47 tools for AI browser automation — navigate, screenshot, click, type, read content, solve CAPTCHAs, scrape websites, synthesize pages, and more.

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

47 tools organized by category. Many tools consolidate multiple operations via `action` or `mode` parameters — fewer round-trips, more capability per call. Click to expand each group.

<details>
<summary><strong>Workflow</strong> (6 tools) — high-level multi-step operations</summary>

| Tool | Description |
|------|-------------|
| `vscreen_browse` | Navigate to a URL and get an overview — navigate, optionally dismiss dialogs, wait, screenshot, return page info with optional text extraction |
| `vscreen_observe` | Show what's on the page — screenshot plus optional visible text and interactive elements summary |
| `vscreen_extract` | Extract structured data — modes: `articles`, `table`, `kv`, `stats`, `links`, `text`, `auto` |
| `vscreen_interact` | Perform an action: click, type, select, hover, or scroll — target by text, CSS selector, or coordinates |
| `vscreen_synthesize` | Build/manage synthesis pages — actions: `list`, `create`, `scrape_and_create` |
| `vscreen_solve_challenge` | Detect and handle page blockers — auto-detect or specify `captcha`, `cookie_consent`, or `ad` |

</details>

<details>
<summary><strong>Instance & locking</strong> (2 tools)</summary>

| Tool | Description |
|------|-------------|
| `vscreen_list_instances` | List all browser instances with states, supervisor status, and lock info |
| `vscreen_lock` | Manage instance locks — actions: `acquire`, `release`, `renew`, `status` |

</details>

<details>
<summary><strong>Observation</strong> (7 tools)</summary>

| Tool | Description |
|------|-------------|
| `vscreen_screenshot` | Capture screenshot (PNG/JPEG/WebP) — supports full-page, clip region, annotated bounding boxes, and multi-frame sequences |
| `vscreen_find` | Find elements — modes: `selector` (CSS), `text` (visible text), `input` (by placeholder/label/role). Supports `include_iframes` |
| `vscreen_get_page_info` | Get current URL, title, viewport dimensions, and scroll position |
| `vscreen_extract_text` | Extract all visible text from the page body |
| `vscreen_accessibility_tree` | Get structured accessibility tree |
| `vscreen_describe_elements` | Identify unlabeled UI elements using vision LLM |
| `vscreen_list_frames` | List frames/iframes with bounding rectangles in page-space coordinates |

</details>

<details>
<summary><strong>Input</strong> (14 tools)</summary>

| Tool | Description |
|------|-------------|
| `vscreen_click` | Click at page-space coordinates (auto-scrolls into view) |
| `vscreen_double_click` | Double-click at coordinates |
| `vscreen_type` | Type text into the focused element character by character |
| `vscreen_fill` | Clear and replace input field content by CSS selector |
| `vscreen_key_press` | Press a single key (Enter, Tab, Escape, arrows, F1–F12, etc.) |
| `vscreen_key_combo` | Press a key combination (e.g., Ctrl+A, Alt+Tab) |
| `vscreen_scroll` | Scroll by pixel delta at a given position |
| `vscreen_drag` | Click-drag between two points with interpolation |
| `vscreen_hover` | Hover at coordinates (triggers CSS :hover and JS events) |
| `vscreen_batch_click` | Click multiple points rapidly in one call |
| `vscreen_click_element` | Click by CSS selector or visible text (main frame only) |
| `vscreen_click_and_navigate` | Click element and wait for URL change (handles SPA pushState) |
| `vscreen_select_option` | Select dropdown option by value or label |
| `vscreen_scroll_to_element` | Scroll element into view by CSS selector |

</details>

<details>
<summary><strong>Navigation & sync</strong> (2 tools)</summary>

| Tool | Description |
|------|-------------|
| `vscreen_navigate` | Navigate the browser — actions: `goto` (with `wait_until`), `back`, `forward`, `reload` |
| `vscreen_wait` | Wait for a condition — `duration`, `idle`, `text`, `selector`, `url`, `network` |

</details>

<details>
<summary><strong>Page interaction</strong> (4 tools)</summary>

| Tool | Description |
|------|-------------|
| `vscreen_execute_js` | Execute arbitrary JavaScript in the main frame |
| `vscreen_dismiss_dialogs` | Auto-dismiss cookie consent, privacy, and GDPR overlays (OneTrust, CookieBot, Didomi, etc.) |
| `vscreen_dismiss_ads` | Dismiss video platform ad overlays (YouTube skip button, pre-roll ads) |
| `vscreen_solve_captcha` | Automatically solve reCAPTCHA v2 image challenges using 2-phase vision analysis |

</details>

<details>
<summary><strong>Session & storage</strong> (5 tools)</summary>

| Tool | Description |
|------|-------------|
| `vscreen_history` | Manage screenshot history — actions: `list`, `get`, `range`, `clear` |
| `vscreen_session_log` | Get timestamped action log of all MCP actions taken |
| `vscreen_session_summary` | Get condensed session summary with action counts and duration |
| `vscreen_console_log` | Get captured browser console messages (log/warn/error) |
| `vscreen_console_clear` | Clear console message buffer |

</details>

<details>
<summary><strong>Storage & cookies</strong> (1 tool)</summary>

| Tool | Description |
|------|-------------|
| `vscreen_storage` | Read/write cookies, localStorage, and sessionStorage — `type` + `action` params |

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
| `vscreen_help` | Get contextual help on any tool, workflow, or concept |

</details>

<details>
<summary><strong>Synthesis</strong> (3 tools)</summary>

| Tool | Description |
|------|-------------|
| `vscreen_synthesis_manage` | Manage synthesis pages — actions: `create`, `update`, `delete`, `list`, `push`, `save`, `navigate` |
| `vscreen_synthesis_scrape` | Scrape structured article data — modes: `single` (one URL) or `batch` (parallel multi-URL via ephemeral CDP tabs) |
| `vscreen_synthesis_scrape_and_create` | One-shot: scrape multiple URLs in parallel AND create a synthesis page with progressive rendering via SSE |

</details>

> **Design note:** 0.2.0 consolidated 73 granular tools into 47 multi-mode tools. Operations like navigate/back/forward/reload are now a single `vscreen_navigate` with an `action` parameter. Element search by selector/text/input is a single `vscreen_find`. Cookie/localStorage/sessionStorage access is a single `vscreen_storage`. This reduces MCP round-trips and simplifies agent workflows.

### Recommended AI workflow

**Quick path (workflow tools):**

1. **Browse** — `vscreen_browse(url=...)` navigates, waits, screenshots, and returns page info in one call
2. **Interact** — `vscreen_interact(action="click", target="Sign In")` acts on elements by text or selector
3. **Observe** — `vscreen_observe(include_elements=true)` screenshots with element inventory
4. **Extract** — `vscreen_extract(mode="articles")` pulls structured data
5. **Repeat**

**Granular path (precise control):**

1. **Navigate** — `vscreen_navigate` to the target URL
2. **Wait** — `vscreen_wait(condition="idle")` for the page to load
3. **Screenshot** — `vscreen_screenshot(full_page=true)` to see the entire page
4. **Discover** — `vscreen_find(by="text", text="Sign In")` to locate targets
5. **Act** — `vscreen_click`, `vscreen_type`, `vscreen_key_press`
6. **Verify** — `vscreen_screenshot` to confirm the result
7. **Repeat**

**Critical rules:**
- Always use `vscreen_screenshot(full_page=true)` to capture entire pages. Never scroll+screenshot in loops.
- Prefer `vscreen_wait(condition="text")` / `vscreen_wait(condition="selector")` over fixed duration waits.
- For multi-site scraping, use `vscreen_synthesis_scrape(mode="batch")` or `vscreen_synthesis_scrape_and_create`.

Use `vscreen_session_log` to review actions taken and `vscreen_history(action="get")` to revisit earlier screenshots.

### Full-page screenshots and coordinate translation

When `full_page=true`, the viewport is temporarily resized to the full document height, capturing the entire page in one image. All click/hover/scroll tools accept these page-level coordinates and automatically scroll to the target before dispatching input.

### Iframe handling

- `vscreen_list_frames` — discover iframes and their bounding rectangles
- `vscreen_find_elements` / `vscreen_find_by_text` with `include_iframes=true` — search inside iframes
- Returned coordinates are page-space and work directly with `vscreen_click`

---

## Synthesis Bubble

The Synthesis Bubble is a dedicated SvelteKit 5 frontend that lets AI agents build custom, interactive web pages from scraped data — no CORS issues, no fragile DOM injection, full rendering control.

### Synthesis quick start

```bash
# Start vscreen with synthesis enabled
./target/release/vscreen --dev --synthesis --mcp-sse 0.0.0.0:8451

# Or with custom port
./target/release/vscreen --dev --synthesis --synthesis-port 5174 --mcp-sse 0.0.0.0:8451

# With vision model for element identification and CAPTCHA solving
./target/release/vscreen --dev --synthesis --mcp-sse 0.0.0.0:8451 \
  --vision-url http://localhost:11434 --vision-model qwen3-vl:8b
```

**Prerequisites** (in addition to the standard vscreen dependencies):

```bash
# Node.js 20+ and pnpm
npm install -g pnpm

# Install synthesis dependencies (one-time)
cd tools/synthesis && pnpm install
```

When `--synthesis` is passed, vscreen automatically:
1. Starts the SvelteKit dev server on `https://0.0.0.0:5174` (HTTPS with self-signed cert)
2. Waits for the server to become healthy
3. Loads any previously persisted pages from `tools/synthesis/.data/`
4. Adds `--ignore-certificate-errors` to the Chromium instance so the agent can navigate to the synthesis server without SSL warnings

Users can open synthesis pages directly in their own browser at `https://host:5174/page/slug` (accept the self-signed cert on first visit).

### How it works

There are three ways to create synthesis pages, from most manual to fully automated:

**Manual (3+ MCP calls):**

```
1. vscreen_synthesis_scrape(url="https://cnn.com", limit=6, source_label="CNN")
   → Returns 6 articles as JSON

2. vscreen_synthesis_create({
     title: "News Digest",
     sections: [{ id: "cnn", component: "card-grid", data: [...] }],
     navigate_instance: "dev"
   })
   → Page visible at https://0.0.0.0:5174/page/news-digest

3. vscreen_screenshot(instance_id="dev", full_page=true)
   → Verify the result
```

**Batch scrape + manual create (2 MCP calls):**

```
1. vscreen_synthesis_scrape_batch({
     instance_id: "dev",
     urls: [
       { url: "https://cnn.com", limit: 6, source_label: "CNN" },
       { url: "https://bbc.com/news", limit: 6, source_label: "BBC" }
     ]
   })
   → Returns structured JSON with all articles per source

2. vscreen_synthesis_create({ ... })
```

### One-shot scrape and create

**Fully automated (1 MCP call):**

```
vscreen_synthesis_scrape_and_create({
  instance_id: "dev",
  title: "News Roundup",
  subtitle: "March 2026",
  theme: "dark",
  urls: [
    { url: "https://cnn.com", limit: 8, source_label: "CNN" },
    { url: "https://bbc.com/news", limit: 8, source_label: "BBC" },
    { url: "https://reuters.com", limit: 8, source_label: "Reuters" },
    { url: "https://huffpost.com", limit: 8, source_label: "HuffPost" }
  ]
})
```

This single call:
1. Opens 4 ephemeral browser tabs in parallel (main tab stays untouched)
2. Navigates each tab, lazy-scrolls to load content, runs the scraper
3. Creates the page with empty sections and navigates the browser to it
4. Pushes scraped articles to each section as they complete (page live-updates via SSE)
5. Auto-selects the best component type per section based on article count
6. Returns per-source article counts, push status, and the page URL

### Component library

31 components across 8 categories, all built with Svelte 5 runes and Tailwind CSS 4.

| Category | Components |
|----------|------------|
| **Content** | `card-grid`, `content-list`, `image-gallery`, `hero`, `article-card`, `source-badge`, `stats-row` |
| **Data Visualization** | `data-table` (sortable, paginated), `bar-chart`, `line-chart`, `pie-chart`, `progress-bar` |
| **Navigation** | `sidebar`, `breadcrumbs`, `pagination` |
| **Interactive** | `accordion`, `modal`, `filter-bar`, `timeline` |
| **Content Blocks** | `markdown-block`, `code-block`, `quote-block`, `key-value-list` |
| **Composite** | `comparison-table`, `notification-banner` |
| **Realtime** | `live-feed`, `status-indicator` |
| **Layout** | `page-shell`, `component-renderer`, `tab-layout`, `split-layout` |

All chart components use raw SVG — no Chart.js or D3 dependencies. The `Section.meta` field passes component-specific configuration (e.g., column definitions for tables, series data for charts).

Components are rendered via a registry map (not if-else chains), preventing Svelte 5 hydration bugs. Each section is wrapped in a `<svelte:boundary>` error boundary — a failing component shows a diagnostic error card instead of breaking the page.

### Component aliases

Intuitive shorthand names are automatically resolved to canonical component types:

| Alias | Resolves to |
|-------|------------|
| `articles`, `article-list`, `article-grid`, `news-grid` | `card-grid` |
| `news-list` | `content-list` |
| `feed` | `live-feed` |
| `chart` | `bar-chart` |
| `table` | `data-table` |
| `kv`, `kvlist` | `key-value-list` |
| `stats` | `stats-row` |
| `images`, `gallery` | `image-gallery` |
| `markdown` | `markdown-block` |
| `code` | `code-block` |

### Synthesis MCP tools

In 0.2.0, synthesis tools were consolidated into 3 multi-action tools:

| Tool | Purpose |
|------|---------|
| `vscreen_synthesis_manage` | All page lifecycle operations via `action` param: `create` (new page with title/theme/layout/sections), `update` (modify existing), `delete`, `list`, `push` (append data to section, triggers SSE live update), `save` (persist to disk), `navigate` (open in browser, auto-bypasses SSL). |
| `vscreen_synthesis_scrape` | Extract structured article data — `single` mode for one URL, `batch` mode for parallel multi-URL scraping via ephemeral CDP tabs. Uses JSON-LD, DOM heuristics, ad filtering, content quality scoring, and image authority locking. |
| `vscreen_synthesis_scrape_and_create` | One-shot: scrape multiple URLs in parallel AND create a page. Progressive rendering — creates page with empty sections first, pushes articles as each source completes (page live-updates via SSE). Auto-selects component types based on article count (1-3: `hero`, 4-12: `card-grid`, 13+: `content-list`). |

### Scraper architecture

The scraper engine runs as a standalone JavaScript file (`crates/vscreen-server/src/mcp/scraper.js`) injected into ephemeral browser tabs. It uses a multi-strategy extraction pipeline:

1. **Strategy 0: JSON-LD** — extracts `NewsArticle` schemas with locked images (never overwritten by later phases)
2. **Strategy 1: `<article>` elements** — semantic HTML article containers
3. **Strategy 2: Heading + link combinations** — `<h2>/<h3>` tags with ancestor `<a>` links
4. **Strategy 3: Data-attribute cards** — `[data-*]` attribute containers
5. **Strategy 4: `<li>` elements** — list items with headings or images

Post-extraction phases:
- **Ad filtering** — rejects images from ad domains and ad-related DOM elements
- **Image recovery** — finds missing images via container walk (max 3 levels, semantic boundaries only)
- **Content quality scoring** — ranks articles by completeness, penalizes promotional content
- **Timeout budget** — skips expensive og:image fetches when time runs low
- **Deduplication** — by URL and title similarity

### Persistence

Pages are stored in-memory by default. Use `vscreen_synthesis_save` to write a page to `tools/synthesis/.data/{id}.json`. On startup, all `.json` files in that directory are automatically loaded.

The `.data/` directory is git-ignored.

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
      --synthesis                    Start the Synthesis Bubble frontend (SvelteKit 5)
      --synthesis-port <PORT>        Synthesis dev server port [default: 5174]
      --synthesis-host <HOST>        Synthesis dev server host [default: 0.0.0.0]
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

# Run Rust tests (626+ tests across 8 crates)
cargo test --workspace

# Run Synthesis tests (306 tests)
cd tools/synthesis && pnpm validate
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

~32,000 lines of Rust + ~5,000 lines of TypeScript/Svelte with strict quality enforcement.

| Metric | Value |
|--------|-------|
| Rust tests (unit + async + integration) | 639+ |
| Synthesis component tests (Vitest) | 306 |
| Synthesis test files | 35 |
| Fuzz targets | 3 |
| Criterion benchmarks | 5 |
| Rust source files | 80 `.rs` files across 8 crates |
| Synthesis components | 31 Svelte 5 components |
| MCP tools | 47 (consolidated from 73 granular tools) |

### Test coverage highlights

- **MCP param deserialization** — every tool parameter struct has positive and negative deserialization tests
- **Component selection** — boundary condition tests for the auto-selector (0, 3, 4, 12, 13, 1000 articles)
- **Process lifecycle** — `is_process_alive` correctness, `stop_child_gracefully` and `stop_process_tree` with real child processes and process group verification
- **Integration tests** — all 47 tools verified via MCP protocol, including no-supervisor and invalid-params error paths
- **Synthesis components** — all 31 component types tested with valid and malformed data via Vitest

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
│   ├── vscreen/              # Binary entry point, dev mode, CLI, process lifecycle
│   ├── vscreen-core/         # Shared types, config, errors, traits
│   ├── vscreen-cdp/          # Chrome DevTools Protocol client (incl. ephemeral tab support)
│   ├── vscreen-video/        # JPEG decode → RGB → I420 → H264/VP9
│   ├── vscreen-audio/        # PulseAudio capture → Opus encode
│   ├── vscreen-transport/    # WebRTC sessions, RTP sender
│   ├── vscreen-rtsp/         # RTSP audio server, session management
│   └── vscreen-server/       # HTTP/WS API, MCP server (47 tools), supervisor
│       └── src/
│           ├── mcp/
│           │   ├── mod.rs             # VScreenMcpServer, tool router, core tools (list, lock, audio, plan, help)
│           │   ├── navigation.rs      # vscreen_navigate, vscreen_wait
│           │   ├── interaction.rs     # click, type, fill, key, scroll, drag, hover, CAPTCHA solver
│           │   ├── observation.rs     # screenshot, find, page info, extract text, accessibility, describe
│           │   ├── session.rs         # history, session log/summary, console, storage
│           │   ├── workflow.rs        # vscreen_browse, vscreen_observe, vscreen_extract
│           │   ├── workflow_interact.rs # vscreen_interact, vscreen_synthesize, vscreen_solve_challenge
│           │   ├── synthesis.rs       # vscreen_synthesis_manage, scrape, scrape_and_create
│           │   ├── captcha.rs         # reCAPTCHA helpers: detection, iframe discovery, grid geometry
│           │   ├── advisor.rs         # Anti-pattern detection and tool selection hints
│           │   ├── docs.rs            # Built-in documentation, task routing, topic help
│           │   ├── params.rs          # All parameter structs (Deserialize, Serialize, JsonSchema)
│           │   ├── scraper.js         # Standalone article scraper engine (injected into browser)
│           │   ├── scraper_table.js   # Table extraction
│           │   ├── scraper_kv.js      # Key-value pair extraction
│           │   ├── scraper_stats.js   # Numeric stat extraction
│           │   └── tests.rs           # Unit tests
│           ├── screenshot_watcher.rs  # Perceptual hash grid change detection
│           ├── supervisor.rs          # Browser lifecycle, CDP management, stealth injection
│           └── vision.rs              # Ollama vision LLM client, streaming inference
├── tools/
│   ├── synthesis/            # Synthesis Bubble (SvelteKit 5, 24 components, HTTPS)
│   │   ├── src/lib/components/   # content/, viz/, nav/, interactive/, blocks/, composite/, layout/
│   │   ├── src/lib/server/       # pages.ts, ws.ts (SSE broadcast), schemas.ts (Zod validation)
│   │   ├── src/lib/types/        # TypeScript interfaces for all data models
│   │   └── src/routes/           # API routes + dynamic page renderer with error boundaries
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
- [Data Extraction Coverage](docs/data-extraction-coverage.md) — website category taxonomy, scraper gap analysis, and implementation roadmap

---

## License

**Source-Available Non-Commercial** — see [LICENSE](LICENSE).

Copyright (c) 2025–2026 Jonathan Retting. All rights reserved.

You may download and use this software for personal, educational, or research purposes. Commercial use, redistribution, and derivative works are prohibited without explicit permission from the author.
