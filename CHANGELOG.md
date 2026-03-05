# Changelog

All notable changes to vscreen are documented here.

## [0.2.0] — 2026-03-05

### MCP Tool System — Full Decomposition & Expansion

The monolithic `mcp.rs` (8,700+ lines) has been decomposed into a modular architecture across 13 dedicated modules (~11,700 lines total), with significant new capabilities added throughout.

**Architecture:**
- `mod.rs` — core tool dispatch, instance listing, locking, RTSP controls, planning
- `navigation.rs` — `vscreen_navigate`, `vscreen_wait` with configurable load strategies
- `interaction.rs` — click (single, double, element, navigate, batch), type, key press/combo, scroll, drag, hover, fill, select, dismiss dialogs/ads, CAPTCHA solver
- `observation.rs` — screenshot, find, extract text, accessibility tree, element descriptions, page info, frame listing, JS execution
- `session.rs` — history, session log/summary, console log/clear, storage inspection
- `workflow.rs` — high-level `vscreen_browse`, `vscreen_observe`, `vscreen_extract` (articles, tables, key-value, stats, links)
- `workflow_interact.rs` — `vscreen_interact`, `vscreen_synthesize`, `vscreen_solve_challenge`
- `synthesis.rs` — `vscreen_synthesis_manage`, `vscreen_synthesis_scrape`, `vscreen_synthesis_scrape_and_create`
- `captcha.rs` — reCAPTCHA detection, iframe discovery, challenge state, tile animation, grid geometry
- `advisor.rs` — tool selection advisor for MCP clients
- `docs.rs` — built-in documentation, task pattern routing, quickstart guides, topic help
- `params.rs` — shared parameter types and validation
- `tests.rs` — comprehensive unit test suite

**New MCP tools (not in 0.1.0):**
- `vscreen_browse` — navigate + wait + screenshot + optional text extraction in one call
- `vscreen_observe` — screenshot + visible text + interactive element summary
- `vscreen_extract` — structured data extraction (articles, tables, key-value pairs, stats, links) with auto-detection
- `vscreen_interact` — unified click/type/select/hover/scroll by text, selector, or coordinates
- `vscreen_synthesize` — list and create synthesis pages from MCP
- `vscreen_solve_challenge` — auto-detect and handle reCAPTCHA, cookie consent, ad overlays
- `vscreen_plan` — task pattern analysis with tool recommendations
- `vscreen_help` — built-in contextual documentation for all tools and workflows
- `vscreen_dismiss_ads` — detect and dismiss ad overlays and interstitials
- `vscreen_describe_elements` — rich descriptions of interactive elements at coordinates
- `vscreen_get_page_info` — full page metadata (title, URL, dimensions, frames, console state)
- `vscreen_list_frames` — enumerate all iframes with bounds and IDs
- `vscreen_session_summary` — condensed session activity summary
- `vscreen_console_log` / `vscreen_console_clear` — browser console access
- `vscreen_storage` — inspect cookies, localStorage, sessionStorage
- `vscreen_fill` — clear-and-replace form fills (works on contenteditable)
- `vscreen_select_option` — dropdown/select element interaction
- `vscreen_scroll_to_element` — scroll elements into view by selector or text
- `vscreen_drag` — drag from source to target coordinates
- `vscreen_hover` — hover at coordinates with optional element detection

### Vision LLM Integration

- Integrated Ollama-hosted vision model (qwen3-vl:8b) for visual understanding
- Streaming inference with thinking-phase detection and configurable abort thresholds
- Vision-powered overlay detection — identifies and classifies popups, cookie banners, ads, CAPTCHAs
- `vscreen_find` uses vision to locate elements by natural language description
- CAPTCHA header and per-tile analysis via cropped image regions sent to vision

### CAPTCHA Solver

- Automated reCAPTCHA v2 image challenge solver using 2-phase vision analysis
- Phase 1: header crop → vision extracts target object and challenge type (3×3 vs 4×4, static vs dynamic)
- Phase 2: individual tile crops → batched parallel vision analysis (3 concurrent) with per-tile match scoring
- Dynamic replacement tile detection via `ScreenshotWatcher` with perceptual hash grid change tracking
- Watcher baseline reset after tile clicks to prevent checkmark toggle loops
- `AbortOnDrop` RAII guard on all spawned vision tasks for guaranteed cleanup on early exit
- Overflow-safe grid dimension math with `saturating_sub`

### Screenshot Watcher

- New background monitoring system for detecting visual changes in clipped page regions
- Splits monitored area into a grid, computes perceptual hashes per cell, tracks changes via hamming distance
- Emits `GridChangeEvent` with changed cell indices, full screenshot, and per-cell cropped PNGs
- Used by the CAPTCHA solver to detect tile replacements without re-analyzing the entire grid

### Data Extraction Pipeline

- `scraper.js` — multi-strategy article/card extractor: JSON-LD, `<article>` tags, heading+link combos, card patterns, list items, ARIA roles, table rows, OpenGraph fallback
- `scraper_table.js` — HTML table extraction with header detection and row parsing
- `scraper_kv.js` — key-value pair extraction from definition lists, label-value patterns, detail tables
- `scraper_stats.js` — numeric stat extraction from dashboard/stats sections
- Image recovery: best-image selection per article (img, picture, background-image, data-bg, video poster, og:image)
- Quality scoring, deduplication, budget/timeout controls, source labeling

### Synthesis System

- Full synthesis pipeline: MCP agents scrape content → create structured pages → SvelteKit renders live
- `vscreen_synthesis_manage` — list, create, update, delete synthesis pages via MCP
- `vscreen_synthesis_scrape` — extract article data from any page for synthesis input
- `vscreen_synthesis_scrape_and_create` — one-shot scrape + page creation
- SvelteKit app with 30+ component types: articles, cards, charts (bar/line/pie), data tables, timelines, accordions, modals, code blocks, image galleries, comparison tables, and more
- Component registry with lazy loading and error boundaries per section
- Pipeline acceleration via batch processing and concurrent scrape+render
- Dev-mode integration: `vscreen --dev` auto-starts the synthesis server alongside Xvfb, PulseAudio, and Chromium

### RTSP Streaming

- VP9 video + Opus audio RTSP streaming to standard players (VLC, ffplay)
- H.264 packetizer for broader client compatibility
- SDP generation with proper media descriptions
- Session management with transport negotiation (UDP/TCP interleaved)
- Health monitoring and automatic recovery
- Quality-adaptive encoding based on client capabilities

### Browser Stealth & Isolation

- `navigator.webdriver` removal
- `window.chrome` runtime injection (runtime, loadTimes, csi)
- Realistic plugin spoofing (Chrome PDF Plugin, Chrome PDF Viewer, Native Client)
- Language and permissions spoofing
- WebGL renderer masking (Intel GPU strings to avoid SwiftShader detection)
- `--disable-blink-features=AutomationControlled` launch flag
- Per-instance isolation: dedicated Xvfb display, unique `--user-data-dir`, separate PulseAudio null-sink

### Bug Fixes

- **Zombie process cleanup** — aggressive `pkill -9` and lock file cleanup (`/tmp/.X*-lock`) for stale Chromium, Xvfb, and PulseAudio processes that blocked instance restarts
- **Orphaned tokio::spawn tasks** — `AbortOnDrop` RAII guard ensures all spawned vision/analysis tasks are cancelled when the parent function exits early
- **u32 subtraction underflow** — CAPTCHA grid dimension calculation uses `saturating_sub` with a minimum-size guard to prevent panics on tiny screenshots
- **CAPTCHA checkmark toggle loop** — `ScreenshotWatcher` baseline reset after tile clicks prevents selected tiles from being detected as new changes
- **Ollama queue saturation** — batched tile analysis (3 concurrent) with increased per-tile timeout prevents Ollama from being overwhelmed by 9 simultaneous vision requests
- **Vision thinking-phase hang** — configurable abort threshold (75% of caller timeout) kills stuck thinking loops in the vision model
- **Concurrency anti-patterns** — fixed lock ordering, removed unnecessary `Arc<Mutex>` nesting, proper async task lifecycle management
- **RTSP session teardown** — clean session cleanup prevents transport resource leaks
- **Pipeline resilience** — graceful recovery from Chromium crashes, Xvfb disconnects, and CDP websocket drops during long-running sessions
- **WebSocket stability** — reconnection logic for live view frontend when the server restarts

### Testing

- Comprehensive MCP integration test suite covering all tool categories
- Unit tests for parameter validation, scraper strategies, component rendering
- E2E RTSP harness for streaming pipeline verification
- Process lifecycle tests for dev-mode startup/shutdown sequences

### Documentation

- Built-in `vscreen_help` with topic-specific documentation (quickstart, workflows, captcha, coordinates, iframes, locking, troubleshooting, tool selection)
- Server instruction prompts for MCP client onboarding
- Task pattern advisor for guiding agents to the right tools

## [0.1.0] — 2026-02-19

Initial release.

- Virtual screen media bridge with Xvfb + PulseAudio + Chromium
- CDP-based browser automation (navigation, click, type, scroll, screenshot)
- Real-time video capture and encoding pipeline
- Audio capture from PulseAudio null-sink
- WebRTC streaming with VP9 video and Opus audio
- Basic RTSP server for video/audio streaming
- MCP server with core browser interaction tools
- Dev mode: single-command `vscreen --dev` launches full stack
- Lock manager for multi-agent instance coordination
- WebSocket live view for real-time browser monitoring
