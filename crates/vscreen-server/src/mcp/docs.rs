//! Documentation constants and task patterns for the vscreen MCP server.
//!
//! This module contains the server instructions, tool selection patterns for
//! vscreen_plan, and all DOC_* markdown constants used by vscreen_help and
//! the MCP resources.

// ---------------------------------------------------------------------------
// Task-to-tool routing patterns for vscreen_plan
// ---------------------------------------------------------------------------

pub(super) struct TaskPattern {
    pub(super) name: &'static str,
    pub(super) keywords: &'static [&'static str],
    pub(super) negative_keywords: &'static [&'static str],
    pub(super) recommendation: &'static str,
}

pub(super) static TASK_PATTERNS: &[TaskPattern] = &[
    TaskPattern {
        name: "Read page text",
        keywords: &["read text", "read the text", "read all", "get text", "page content", "what does the page say", "extract text", "page text", "all text", "visible text", "text on the page", "text content"],
        negative_keywords: &["click", "fill", "screenshot"],
        recommendation: "\
1. `vscreen_extract_text(instance_id)` — returns all visible text without screenshots\n\
   - Optionally pass `selector` to extract from a specific element\n\
   - Much faster and more accurate than screenshot + OCR",
    },
    TaskPattern {
        name: "See the full page",
        keywords: &["full page", "whole page", "entire page", "see everything", "overview", "what is on the page", "see the page"],
        negative_keywords: &[],
        recommendation: "\
1. `vscreen_screenshot(instance_id, full_page=true)` — captures the entire scrollable document in one image\n\
   - Do NOT scroll + screenshot repeatedly\n\
   - Use `clip={x,y,width,height}` to zoom into a specific region",
    },
    TaskPattern {
        name: "Find and click a button/link",
        keywords: &["click", "press", "tap", "button", "link", "sign in", "submit", "login", "log in"],
        negative_keywords: &["fill", "form", "type", "captcha"],
        recommendation: "\
1. `vscreen_click_element(instance_id, text=\"Button Text\")` — finds and clicks by visible text (main frame, with retries)\n\
   OR: `vscreen_click_element(instance_id, selector=\"button.submit\")` — by CSS selector\n\
   - For iframe elements: `vscreen_find_by_text(text=\"...\", include_iframes=true)` then `vscreen_click(x, y)`\n\
   - For click + navigation: `vscreen_click_and_navigate(text=\"Link Text\")`",
    },
    TaskPattern {
        name: "Fill a form",
        keywords: &["fill", "form", "enter", "type", "input", "field", "username", "password", "email"],
        negative_keywords: &["captcha"],
        recommendation: "\
1. `vscreen_find_input(instance_id, placeholder=\"...\")` or `vscreen_find_input(label=\"...\")` — discover form fields\n\
2. `vscreen_fill(instance_id, selector=\"input[name='field']\", value=\"...\")` — clear + fill each field\n\
3. `vscreen_click_element(instance_id, text=\"Submit\")` — submit the form\n\
   - Use `vscreen_screenshot` after submit to verify",
    },
    TaskPattern {
        name: "Navigate and extract data",
        keywords: &["navigate", "go to", "open", "visit", "load"],
        negative_keywords: &[],
        recommendation: "\
1. `vscreen_navigate(instance_id, url=\"...\", wait_until=\"load\")` — load the page\n\
2. `vscreen_dismiss_dialogs(instance_id)` — clear cookie consent / GDPR overlays\n\
3. `vscreen_extract_text(instance_id)` — get all text content\n\
   OR: `vscreen_screenshot(instance_id, full_page=true)` — see the visual layout",
    },
    TaskPattern {
        name: "Wait for dynamic content",
        keywords: &["wait", "loading", "spinner", "appear", "show up", "dynamic", "ajax", "async"],
        negative_keywords: &[],
        recommendation: "\
Use targeted waits instead of fixed delays:\n\
- `vscreen_wait_for_text(instance_id, text=\"...\")` — wait for specific text to appear\n\
- `vscreen_wait_for_selector(instance_id, selector=\"...\")` — wait for an element to exist\n\
- `vscreen_wait_for_url(instance_id, url_contains=\"...\")` — wait for navigation\n\
- `vscreen_wait_for_network_idle(instance_id)` — wait for all network requests to complete\n\
Do NOT use `vscreen_wait(5000)` loops.",
    },
    TaskPattern {
        name: "Find an element",
        keywords: &["find", "locate", "where is", "search for", "element"],
        negative_keywords: &["click", "fill", "text content"],
        recommendation: "\
- By visible text: `vscreen_find_by_text(instance_id, text=\"Sign In\")`\n\
- By CSS selector: `vscreen_find_elements(instance_id, selector=\"button.primary\")`\n\
- Set `include_iframes=true` when elements might be inside iframes\n\
- For semantic structure: `vscreen_accessibility_tree(instance_id)`",
    },
    TaskPattern {
        name: "Solve CAPTCHA",
        keywords: &["captcha", "recaptcha", "verify", "robot"],
        negative_keywords: &[],
        recommendation: "\
1. `vscreen_solve_captcha(instance_id)` — fully automated reCAPTCHA v2 solving\n\
   - Handles checkbox click, tile selection, verification, retries\n\
   - Requires vision LLM (--vision-url)\n\
   - For manual workflow: `vscreen_help(topic=\"captcha\")`",
    },
    TaskPattern {
        name: "Dismiss popups/overlays",
        keywords: &["dismiss", "close", "popup", "overlay", "cookie", "consent", "banner", "gdpr", "dialog"],
        negative_keywords: &[],
        recommendation: "\
1. `vscreen_dismiss_dialogs(instance_id)` — auto-detects and dismisses cookie consent, GDPR, privacy dialogs\n\
   - Supports OneTrust, CookieBot, Didomi, Quantcast, TrustArc, and many more\n\
   - For video ads: `vscreen_dismiss_ads(instance_id)`",
    },
    TaskPattern {
        name: "Scroll to content",
        keywords: &["scroll", "below", "bottom", "down", "more content"],
        negative_keywords: &[],
        recommendation: "\
- To see below-fold content: `vscreen_screenshot(instance_id, full_page=true)` — captures everything\n\
- To bring a specific element into view: `vscreen_scroll_to_element(instance_id, selector=\"#target\")`\n\
- Do NOT use `vscreen_scroll` + screenshot loops",
    },
    TaskPattern {
        name: "Get page metadata",
        keywords: &["url", "title", "page info", "current page", "what page"],
        negative_keywords: &["text", "content", "extract"],
        recommendation: "\
1. `vscreen_get_page_info(instance_id)` — returns URL, title, viewport dimensions, scroll position\n\
   - Do NOT use `vscreen_execute_js(\"document.title\")` for this",
    },
    TaskPattern {
        name: "Work with iframes",
        keywords: &["iframe", "frame", "embed", "widget", "recaptcha iframe"],
        negative_keywords: &["captcha solve"],
        recommendation: "\
1. `vscreen_list_frames(instance_id)` — discover all iframes with bounding rects\n\
2. `vscreen_find_by_text(text=\"...\", include_iframes=true)` — find elements inside iframes\n\
3. `vscreen_click(x, y)` — click using page-space coordinates from find tools\n\
   - `vscreen_click_element` does NOT search iframes\n\
   - Use `vscreen_screenshot(clip={x,y,w,h})` to zoom into an iframe region",
    },
    TaskPattern {
        name: "Navigate between pages",
        keywords: &["back", "forward", "history", "previous page", "go back", "next page"],
        negative_keywords: &[],
        recommendation: "\
- `vscreen_go_back(instance_id)` — browser back button\n\
- `vscreen_go_forward(instance_id)` — browser forward button\n\
- `vscreen_reload(instance_id)` — refresh the page",
    },
    TaskPattern {
        name: "Take multiple screenshots",
        keywords: &["animation", "transition", "sequence", "recording", "multiple screenshots", "over time"],
        negative_keywords: &[],
        recommendation: "\
1. `vscreen_screenshot_sequence(instance_id, count=5, interval_ms=1000)` — capture N screenshots at regular intervals\n\
   - Useful for observing animations, transitions, or verifying timed actions",
    },
    TaskPattern {
        name: "Identify unlabeled elements",
        keywords: &["icon", "unlabeled", "no text", "what is this button", "describe"],
        negative_keywords: &[],
        recommendation: "\
1. `vscreen_describe_elements(instance_id)` — uses vision LLM to identify icon-only buttons and unlabeled UI elements\n\
   - Requires --vision-url\n\
   - Try `vscreen_find_elements` or `vscreen_find_by_text` first for labeled elements",
    },
];

// ---------------------------------------------------------------------------
// Documentation constants
// ---------------------------------------------------------------------------

pub(super) const SERVER_INSTRUCTIONS: &str = r#"# vscreen MCP Server

Control headless Chromium browser instances for AI-driven web automation.

## Core Workflow

1. **Discover** — Call `vscreen_list_instances` to find available browser instances. Each returns an `instance_id` (e.g., "dev") used in all subsequent calls.
2. **Navigate** — Call `vscreen_navigate(instance_id, url, wait_until="load")` to load a page. Returns the final URL and page title.
3. **Observe** — Call `vscreen_screenshot(instance_id)` to capture the current page. Use `clip: {x, y, width, height}` to zoom into a specific region.
4. **Discover Elements** — Call `vscreen_find_elements(selector)` or `vscreen_find_by_text(text)` to locate interactive elements. Returns page-space coordinates you can pass directly to click/hover.
5. **Act** — Call `vscreen_click(x, y)`, `vscreen_type(text)`, `vscreen_fill(selector, value)`, etc. to interact with the page.
6. **Verify** — Take another screenshot to confirm the action had the desired effect.
7. **Repeat** steps 3-6 as needed.

## Coordinate System

All coordinates are in **page-space** (absolute position on the full page, not viewport-relative). The system automatically scrolls elements into view before clicking. Coordinates returned by `vscreen_find_elements` and `vscreen_find_by_text` can be passed directly to `vscreen_click` without any translation.

## Iframe Handling

Cross-origin iframes (e.g., reCAPTCHA, embedded widgets) require special handling:
- Use `vscreen_list_frames` to discover all iframes and their bounding rectangles.
- Use `vscreen_find_elements(include_iframes=true)` or `vscreen_find_by_text(include_iframes=true)` to search inside iframes. Returned coordinates are already translated to page-space.
- Click iframe elements using the returned page-space coordinates with `vscreen_click`.
- The `frame_id` in results identifies which iframe contains each element.

## Connection Modes

vscreen supports multiple ways to connect:

- **SSE direct (recommended for Cursor)**: Start the server with `vscreen --dev --mcp-sse 0.0.0.0:8451`, then configure your MCP client with `{"url": "http://localhost:8451/mcp"}`. The server runs independently and survives client reconnections.
- **Stdio proxy**: For MCP clients that only support subprocess spawning, use `vscreen --mcp-stdio-proxy http://localhost:8451/mcp`. This lightweight proxy forwards messages to an existing SSE server without starting its own dev environment.
- **Stdio direct**: `vscreen --mcp-stdio --dev` starts a full server in subprocess mode. Not recommended when another vscreen instance is already running (causes resource conflicts).

**Best practice**: Start the server once with `--dev --mcp-sse`, then connect via SSE URL or stdio proxy.

## Instance Locking

- **Single-agent mode** (default with `--mcp-stdio`): Locks are auto-acquired. No explicit locking needed.
- **Multi-agent mode**: Call `vscreen_instance_lock(lock_type="exclusive")` before interacting. Release with `vscreen_instance_unlock` when done.

## Tool Categories

- **Observation** (read-only): `vscreen_screenshot`, `vscreen_find_elements`, `vscreen_find_by_text`, `vscreen_get_page_info`, `vscreen_list_frames`, `vscreen_extract_text`, `vscreen_accessibility_tree`, `vscreen_screenshot_annotated`, `vscreen_describe_elements`
- **Input Actions**: `vscreen_click`, `vscreen_type`, `vscreen_fill`, `vscreen_key_press`, `vscreen_key_combo`, `vscreen_scroll`, `vscreen_drag`, `vscreen_hover`, `vscreen_batch_click`, `vscreen_click_element`, `vscreen_select_option`, `vscreen_scroll_to_element`
- **Navigation**: `vscreen_navigate`, `vscreen_go_back`, `vscreen_go_forward`, `vscreen_reload`
- **Synchronization**: `vscreen_wait`, `vscreen_wait_for_idle`, `vscreen_wait_for_text`, `vscreen_wait_for_selector`, `vscreen_wait_for_url`, `vscreen_wait_for_network_idle`
- **Memory & Context**: `vscreen_history_list`, `vscreen_history_get`, `vscreen_session_log`, `vscreen_session_summary`, `vscreen_console_log`
- **Cookie & Storage**: `vscreen_get_cookies`, `vscreen_set_cookie`, `vscreen_get_storage`, `vscreen_set_storage`
- **Utility**: `vscreen_dismiss_dialogs`, `vscreen_execute_js`, `vscreen_help`

## Tool Selection Rules

OBSERVE the full page:
- NEVER scroll + screenshot repeatedly. Use `vscreen_screenshot(full_page=true)` for the entire document in one call.
- Use `vscreen_screenshot(clip={x,y,width,height})` to zoom into a specific region at full resolution.

READ page content:
- NEVER screenshot to read text. Use `vscreen_extract_text` for all visible text in one call.
- Use `vscreen_get_page_info` for URL, title, viewport — not `vscreen_execute_js("document.title")`.

FIND elements:
- Use `vscreen_find_by_text` when you know the visible label (e.g. "Sign In").
- Use `vscreen_find_elements` when you have a CSS selector.
- Use `vscreen_accessibility_tree` for semantic page structure.
- Set `include_iframes=true` when elements might be inside iframes.

CLICK elements:
- Main frame by text/selector: prefer `vscreen_click_element` (has retries + wait built-in).
- By coordinates (from find tools) or inside iframes: use `vscreen_click`.
- Rapid multi-click (CAPTCHA tiles): use `vscreen_batch_click`.
- Click + expect navigation: use `vscreen_click_and_navigate`.

WAIT for changes:
- NEVER use `vscreen_wait(5000)` + screenshot loops. Use:
  - `vscreen_wait_for_text` when expecting specific text to appear.
  - `vscreen_wait_for_selector` when expecting an element to appear.
  - `vscreen_wait_for_url` when expecting navigation.
  - `vscreen_wait_for_network_idle` when waiting for async data loads.

SCROLL:
- To bring an element into view: use `vscreen_scroll_to_element` (not manual scroll deltas).
- To capture below-fold content: use `vscreen_screenshot(full_page=true)` instead of scrolling.

PLAN complex tasks:
- Call `vscreen_plan(task="...")` before multi-step workflows to get the optimal tool sequence.

## Error Recovery

- If a click doesn't produce the expected result, take a screenshot and re-analyze.
- If an element isn't found, try `include_iframes: true`, a broader CSS selector, or `vscreen_wait_for_selector` first.
- If a page has a cookie consent overlay, call `vscreen_dismiss_dialogs` first.
- Call `vscreen_help(topic)` for detailed guidance on any tool or workflow.
"#;

pub(super) const DOC_QUICKSTART: &str = r#"# vscreen Quick Start Guide

## 1. Discover Instances

```json
// Call vscreen_list_instances (no arguments)
// Returns: [{"instance_id": "dev", "state": {"state": "running"}, ...}]
```

## 2. Navigate to a Page

```json
// Call vscreen_navigate
{"instance_id": "dev", "url": "https://example.com", "wait_until": "load"}
// Returns: "Navigated to https://example.com\nTitle: Example Domain"
```

`wait_until` options:
- `"load"` — Wait for the load event (recommended default)
- `"domcontentloaded"` — Wait for DOM ready (faster, but images may not be loaded)
- `"networkidle"` — Wait until network is quiet for 500ms (slowest, most complete)
- `"none"` — Don't wait at all

## 3. Take a Screenshot

```json
// Call vscreen_screenshot
{"instance_id": "dev"}
// Returns: base64-encoded PNG image
```

Options:
- `format`: `"png"` (default), `"jpeg"`, `"webp"`
- `quality`: 0-100 (for jpeg/webp)
- `full_page`: true to capture the entire scrollable page
- `clip`: `{"x": 100, "y": 200, "width": 400, "height": 300}` to capture a region

## 4. Find Elements

```json
// By CSS selector:
{"instance_id": "dev", "selector": "button.submit", "include_iframes": true}
// Returns: [{"tag": "button", "text": "Submit", "x": 150, "y": 400, "width": 100, "height": 40, ...}]

// By visible text:
{"instance_id": "dev", "text": "Sign In", "include_iframes": true}
// Returns: [{"tag": "a", "text": "Sign In", "x": 800, "y": 20, "width": 60, "height": 20, ...}]
```

## 5. Click an Element

```json
// Use coordinates from find_elements/find_by_text:
{"instance_id": "dev", "x": 150, "y": 400}
// The system auto-scrolls the element into view before clicking.
```

## 6. Type Text

```json
// Type into the focused element:
{"instance_id": "dev", "text": "Hello World"}

// Or fill a specific input field (clears first):
{"instance_id": "dev", "selector": "input[name='email']", "value": "user@example.com"}
```

## 7. Verify Results

Take another screenshot after each action to confirm it worked as expected.
"#;

pub(super) const DOC_CAPTCHA: &str = r#"# Solving reCAPTCHA Challenges

## Automated (Recommended)

Use `vscreen_solve_captcha` for fully automated reCAPTCHA v2 solving:

```json
{"instance_id": "dev"}
```

This tool:
1. Finds the "I'm not a robot" checkbox in the reCAPTCHA iframe
2. Clicks it to trigger the image challenge
3. Uses vision LLM to identify which tiles match the target object
4. Clicks all matching tiles + VERIFY in rapid succession via batch click
5. Handles multi-round challenges (reCAPTCHA often requires 2-5 rounds)
6. Retries with page reload if the challenge expires

Parameters:
- `instance_id` (required): The browser instance
- `max_attempts` (default: 3): Max page-reload retry cycles

Returns: `{solved: bool, rounds: N, attempts: N, details: [...]}`

**Requires** `--vision-url` to be configured.

## Manual Workflow (Fallback)

If vision LLM is not available or for non-reCAPTCHA CAPTCHAs:

1. `vscreen_find_by_text("I'm not a robot", include_iframes=true)` — Find checkbox
2. `vscreen_click(x, y)` — Click the checkbox center coordinates
3. `vscreen_wait(3000)` — Wait for challenge
4. `vscreen_screenshot(clip={challenge_iframe_bounds})` — Capture tiles
5. Identify correct tiles visually
6. `vscreen_batch_click(points=[...tiles, verify_btn], delay_between_ms=150)` — Click all at once
7. `vscreen_screenshot` — Check result

### Grid Coordinates Reference

Challenge iframe is typically at (85, 84, 400x580). Tile centers:

**3x3 grid** (9 separate images):
```
Tile 1: (152, 249)  Tile 2: (285, 249)  Tile 3: (418, 249)
Tile 4: (152, 389)  Tile 5: (285, 389)  Tile 6: (418, 389)
Tile 7: (152, 529)  Tile 8: (285, 529)  Tile 9: (418, 529)
```

**4x4 grid** (single image split into tiles):
```
Tile 1:  (135, 237)  Tile 2:  (235, 237)  Tile 3:  (335, 237)  Tile 4:  (435, 237)
Tile 5:  (135, 342)  Tile 6:  (235, 342)  Tile 7:  (335, 342)  Tile 8:  (435, 342)
Tile 9:  (135, 447)  Tile 10: (235, 447)  Tile 11: (335, 447)  Tile 12: (435, 447)
Tile 13: (135, 552)  Tile 14: (235, 552)  Tile 15: (335, 552)  Tile 16: (435, 552)
```

**VERIFY/SKIP button**: ~(435, 634)

### Important Notes

- Use `vscreen_batch_click` — individual clicks are too slow for the 2-minute timer
- After challenge transitions, `find_elements` may return stale tiles; use hardcoded positions
- Always reload the page after a timeout (don't re-click the expired checkbox)
- reCAPTCHA may require 2-5 rounds of image selection
"#;

pub(super) const DOC_WORKFLOWS: &str = r#"# Common Workflows

## Solving reCAPTCHA

Use `vscreen_solve_captcha(instance_id)` for automated solving, or see `vscreen_help(topic="captcha")` for the full manual workflow and grid coordinate reference.

## Handling Cookie Consent Dialogs

1. First try `vscreen_dismiss_dialogs(instance_id)` — Handles OneTrust, CookieBot, and common patterns
2. If that fails: `vscreen_find_by_text("Accept")` or `vscreen_find_by_text("I agree")`
3. `vscreen_click` on the found button
4. Cookies persist across navigations (browser profile is saved)

## Extracting Article Text

1. `vscreen_navigate(url, wait_until="load")` — Load the article
2. `vscreen_dismiss_dialogs` — Clear any overlays
3. `vscreen_extract_text` — Get all visible text from the page
4. Or for structured extraction: `vscreen_find_elements("article, main, .content")` then `vscreen_execute_js` with a custom text extraction script

## Filling Forms

1. `vscreen_find_elements("input, select, textarea")` — Discover all form fields
2. For each text input: `vscreen_fill(selector="input[name='fieldname']", value="value")`
3. For dropdowns: `vscreen_select_option(selector="select[name='country']", value="US")`
4. For checkboxes/radios: `vscreen_click` at the element coordinates
5. Submit: `vscreen_find_elements("button[type='submit'], input[type='submit']")` then `vscreen_click`

## Multi-Page Navigation

1. `vscreen_navigate(url)` — Start at the first page
2. Process the page content
3. `vscreen_find_by_text("Next")` or `vscreen_find_elements("a.next, [rel='next']")` — Find pagination
4. `vscreen_click` the next button
5. `vscreen_wait_for_url` or `vscreen_wait_for_idle` — Wait for navigation to complete
6. Repeat from step 2

## Taking Sequential Screenshots

```json
{"instance_id": "dev", "count": 3, "interval_ms": 1000}
```
Returns 3 screenshots taken 1 second apart — useful for observing animations, loading sequences, or dynamic content.
"#;

pub(super) const DOC_COORDINATES: &str = r#"# Coordinate System

## Page-Space Coordinates

All vscreen coordinates use **page-space** — the absolute position on the full document, measured from the top-left corner (0, 0) of the page.

- `vscreen_click(x, y)` — x and y are page-space. The system automatically scrolls to make the point visible before dispatching the click.
- `vscreen_find_elements` — Returns `x`, `y`, `width`, `height` in page-space.
- `vscreen_find_by_text` — Returns `x`, `y`, `width`, `height` in page-space.
- `vscreen_screenshot(clip)` — The clip rectangle uses page-space coordinates.

## Iframe Coordinate Translation

When `include_iframes: true` is used with `vscreen_find_elements` or `vscreen_find_by_text`, elements inside iframes have their coordinates automatically translated to page-space. The returned `x` and `y` values account for the iframe's position on the page, so you can pass them directly to `vscreen_click`.

## Click Target Calculation

To click the center of a found element:
```
click_x = element.x + element.width / 2
click_y = element.y + element.height / 2
```

## Full-Page Screenshots

When `full_page: true` is used, the screenshot captures the entire scrollable page. The resulting image may be much larger than the viewport (1920x1080). Coordinates from a full-page screenshot are still in page-space and can be used directly with `vscreen_click`.

## Viewport Size

The default viewport is **1920x1080**. Elements visible without scrolling have y-coordinates between 0 and 1080.
"#;

pub(super) const DOC_IFRAMES: &str = r#"# Working with Iframes

## Overview

Many web pages embed cross-origin content in iframes (reCAPTCHA, ads, embedded widgets, payment forms). These iframes have separate DOM trees that are not accessible via normal `vscreen_execute_js` calls.

## Discovering Iframes

Call `vscreen_list_frames` to get:
1. The complete frame tree (parent-child relationships)
2. Bounding rectangles for each iframe on the page
3. Frame IDs, names, URLs, and visibility status

Example response includes:
```json
{"src": "https://www.google.com/recaptcha/...", "name": "a-xyz", "title": "reCAPTCHA",
 "x": 33, "y": 336, "width": 304, "height": 78, "visible": true}
```

## Finding Elements in Iframes

Use `include_iframes: true` on search tools:

```json
// vscreen_find_elements
{"instance_id": "dev", "selector": "button", "include_iframes": true}

// vscreen_find_by_text
{"instance_id": "dev", "text": "I'm not a robot", "include_iframes": true}
```

Results include a `frame_id` field identifying which frame contains each element, and coordinates are already translated to page-space.

## Clicking Iframe Elements

Simply use the page-space coordinates from search results:
```json
// Element found at {"x": 86, "y": 337, "frame_id": "ABC123"}
// Click directly using those coordinates:
{"instance_id": "dev", "x": 120, "y": 370}
```

## Limitations

- `vscreen_execute_js` runs only in the main frame
- `vscreen_click_element` searches only the main frame (use `vscreen_find_by_text` + `vscreen_click` for iframe elements)
- `vscreen_screenshot_annotated` only annotates main-frame elements
"#;

pub(super) const DOC_LOCKING: &str = r#"# Instance Locking

## When Do You Need Locking?

- **Single-agent mode** (e.g., `--mcp-stdio` spawned by one client): Locks are auto-acquired transparently. You don't need to call any lock tools.
- **Multi-agent mode** (e.g., `--mcp-sse` with multiple clients): Explicit locking prevents agents from interfering with each other.

## Lock Types

- **`exclusive`** — Full control. Only one session can hold an exclusive lock. Required for any input action (click, type, navigate, etc.).
- **`observer`** — Read-only access. Multiple observers can coexist with each other, but not with an exclusive lock. Allows screenshots, find_elements, get_page_info, etc.

## Workflow

```
1. vscreen_instance_lock(instance_id="dev", lock_type="exclusive")
   → Returns: lock_token (save this for unlock/renew)

2. ... perform your actions ...

3. vscreen_instance_lock_renew(instance_id="dev", lock_token="...", ttl_seconds=300)
   → Extends the lock if your task takes longer than expected

4. vscreen_instance_unlock(instance_id="dev", lock_token="...")
   → Release the lock when done
```

## Lock Parameters

- `ttl_seconds` (default: 300) — Lock automatically expires after this many seconds
- `wait_timeout_seconds` (default: 0) — If the instance is already locked, wait this many seconds for it to become available (0 = fail immediately)
- `lock_token` — Returned on lock acquisition; required for unlock and renew

## Checking Lock Status

```json
// All instances:
vscreen_instance_lock_status()

// Specific instance:
vscreen_instance_lock_status(instance_id="dev")
```
"#;

pub(super) const DOC_TROUBLESHOOTING: &str = r#"# Troubleshooting

## "Element not found"

- Try `include_iframes: true` — the element may be inside an iframe
- Use a broader CSS selector (e.g., `button` instead of `button.specific-class`)
- The element may not be loaded yet — use `vscreen_wait_for_selector(selector, timeout_ms=10000)` first
- The element may be below the fold — it still has page-space coordinates, so `vscreen_find_elements` will find it

## "Click doesn't seem to work"

- Take a `vscreen_screenshot` to verify the coordinates visually
- The element may be behind an overlay (cookie consent, modal). Use `vscreen_dismiss_dialogs` first
- The element may be inside an iframe — use `vscreen_find_by_text(include_iframes=true)` to get correct page-space coordinates
- Try `vscreen_click_element(selector=".button")` for main-frame elements

## "InstanceLocked" error

- Another MCP session holds the lock
- Check with `vscreen_instance_lock_status(instance_id)`
- In single-agent mode, this shouldn't happen — it may indicate a stale session
- Wait for the lock: `vscreen_instance_lock(wait_timeout_seconds=30)`

## "Timeout waiting for text/selector"

- The content may be dynamically loaded after the page "load" event
- Increase `timeout_ms` (default is 10000ms / 10 seconds)
- Use `vscreen_wait_for_network_idle` to wait for all AJAX requests to complete first
- Check the page with `vscreen_screenshot` to see current state

## "Cookie consent blocking content"

- Call `vscreen_dismiss_dialogs(instance_id)` — handles OneTrust, CookieBot, and common patterns
- If that doesn't work: `vscreen_find_by_text("Accept all")` then `vscreen_click`
- Cookies persist across navigations within the same instance

## "JavaScript returns null/undefined"

- The DOM element may not exist yet — use `vscreen_wait_for_selector` first
- `vscreen_execute_js` runs in the **main frame only** — it cannot access iframe content
- Wrap your expression in a try-catch: `"try { ... } catch(e) { e.message }"`

## "Page looks different than expected"

- Take a `vscreen_screenshot(full_page=true)` to see the entire page
- The page may have responsive breakpoints — viewport is 1920x1080
- Dynamic content may have changed — use `vscreen_screenshot_sequence` to watch for changes

## "Connection closed" or MCP disconnects

- **If using `--mcp-stdio --dev`**: The stdio subprocess starts a full dev environment. If another vscreen instance is already running, resources conflict (Xvfb display, CDP port, HTTP port), causing crashes.
- **Fix**: Switch to SSE mode. Start the server once: `vscreen --dev --mcp-sse 0.0.0.0:8451`. Then configure your MCP client with `{"url": "http://localhost:8451/mcp"}`.
- **Alternative**: Use the stdio proxy: `vscreen --mcp-stdio-proxy http://localhost:8451/mcp`. This is lightweight and can be freely restarted without affecting the server.

## "Display :99 already in use" or "CDP port in use"

- Another vscreen instance is running on the same display or CDP port.
- Use `--dev-display N` to choose a different X11 display number.
- Use `--dev-cdp-port N` to choose a different Chrome DevTools Protocol port.
- Or stop the existing instance first.

## "vscreen dev already running (PID ...)"

- vscreen writes a PID file to prevent duplicate instances.
- Stop the existing instance or connect to it via SSE/proxy.
- If the PID file is stale (process crashed), vscreen will clean it up automatically on next start.
"#;

pub(super) const DOC_TOOL_SELECTION: &str = r##"# Tool Selection Guide

Choosing the right tool avoids unnecessary round-trips and gives better results.

## Observing the Page

| Goal | Best Tool | NOT This |
|------|-----------|----------|
| See the full page | `vscreen_screenshot(full_page=true)` | scroll + screenshot repeatedly |
| Read page text | `vscreen_extract_text` | screenshot + OCR |
| Zoom into a region | `vscreen_screenshot(clip={x,y,w,h})` | full screenshot + cropping |
| Page title/URL/viewport | `vscreen_get_page_info` | `vscreen_execute_js("document.title")` |
| Identify interactive elements | `vscreen_screenshot_annotated` | manual screenshot + guessing |

## Finding Elements

| Goal | Best Tool |
|------|-----------|
| Know the visible text | `vscreen_find_by_text(text="Sign In")` |
| Know the CSS selector | `vscreen_find_elements(selector="button.submit")` |
| Element is inside an iframe | add `include_iframes=true` to find tools |
| Need semantic structure | `vscreen_accessibility_tree` |
| Icon-only/unlabeled buttons | `vscreen_describe_elements` (requires vision LLM) |

## Clicking Elements

| Goal | Best Tool |
|------|-----------|
| Click by text (main frame) | `vscreen_click_element(text="Submit")` |
| Click by selector (main frame) | `vscreen_click_element(selector="#btn")` |
| Click in iframe or with coordinates | `vscreen_click(x, y)` |
| Click + wait for navigation | `vscreen_click_and_navigate(text="Next")` |
| Click many tiles rapidly | `vscreen_batch_click(points=[[x1,y1],[x2,y2]])` |

## Waiting for Changes

| Goal | Best Tool | NOT This |
|------|-----------|----------|
| Text appears | `vscreen_wait_for_text(text="Success")` | `vscreen_wait(5000)` loop |
| Element appears | `vscreen_wait_for_selector(selector=".result")` | `vscreen_wait(5000)` loop |
| Page navigates | `vscreen_wait_for_url(url_contains="/dashboard")` | `vscreen_wait(5000)` loop |
| Data loads (AJAX) | `vscreen_wait_for_network_idle` | `vscreen_wait(5000)` loop |
| Brief pause after action | `vscreen_wait(500)` | — this is fine for short pauses |

## Scrolling

| Goal | Best Tool | NOT This |
|------|-----------|----------|
| See below-fold content | `vscreen_screenshot(full_page=true)` | scroll + screenshot loop |
| Bring element into view | `vscreen_scroll_to_element(selector)` | `vscreen_scroll(x, y, delta_y)` |
| Precise pixel scroll needed | `vscreen_scroll(x, y, delta_y=120)` | — this is the right tool |

## Planning

Call `vscreen_plan(task="describe your goal")` before multi-step workflows to get
the optimal tool sequence and avoid anti-patterns.
"##;

pub(super) const DOC_TOOLS_HEADER: &str = r#"# Tool Reference

All tools accept `instance_id` as their first parameter (the browser instance to operate on).
Use `vscreen_list_instances` to discover available instances.

## Observation Tools (Read-Only)

These tools do not modify page state and can be called freely.

| Tool | Description |
|------|-------------|
| `vscreen_list_instances` | List all browser instances and their states |
| `vscreen_screenshot` | Capture page screenshot (png/jpeg/webp, optional clip region) |
| `vscreen_screenshot_sequence` | Capture multiple screenshots at intervals |
| `vscreen_screenshot_annotated` | Screenshot with numbered bounding boxes on elements |
| `vscreen_get_page_info` | Get page title, URL, and viewport dimensions |
| `vscreen_find_elements` | Find elements by CSS selector (supports iframe search) |
| `vscreen_find_by_text` | Find elements by visible text (supports iframe search) |
| `vscreen_list_frames` | List all frames/iframes with bounding rectangles |
| `vscreen_extract_text` | Extract all visible text from the page |
| `vscreen_accessibility_tree` | Get the accessibility tree (structured page representation) |
| `vscreen_describe_elements` | Identify unlabeled icon-only elements using vision LLM |
| `vscreen_get_cookies` | Get cookies for the current page |
| `vscreen_get_storage` | Get localStorage or sessionStorage values |
| `vscreen_history_list` | List screenshot history entries |
| `vscreen_history_get` | Get a specific historical screenshot |
| `vscreen_session_log` | Get recent action log |
| `vscreen_session_summary` | Get session summary (action count, duration) |
| `vscreen_console_log` | Get captured browser console messages |
| `vscreen_instance_lock_status` | Check lock status |

## Input Action Tools

These tools modify page state (click, type, scroll, etc.).

| Tool | Description |
|------|-------------|
| `vscreen_click` | Click at page-space coordinates (auto-scrolls) |
| `vscreen_double_click` | Double-click at coordinates |
| `vscreen_type` | Type text into the focused element |
| `vscreen_fill` | Clear and fill an input field by CSS selector |
| `vscreen_key_press` | Press a single key (e.g., "Enter", "Escape", "Tab") |
| `vscreen_key_combo` | Press a key combination (e.g., ["Control", "a"]) |
| `vscreen_scroll` | Scroll by pixel delta |
| `vscreen_drag` | Drag from one point to another |
| `vscreen_hover` | Hover at coordinates |
| `vscreen_click_element` | Click element by CSS selector or visible text (with retries + wait_after_ms) |
| `vscreen_click_and_navigate` | Click element and wait for URL change (with <a> fallback) |
| `vscreen_batch_click` | Click multiple points rapidly in one call (for timed challenges) |
| `vscreen_select_option` | Select a dropdown option by value or label |
| `vscreen_scroll_to_element` | Scroll an element into view by CSS selector |

## Navigation Tools

| Tool | Description |
|------|-------------|
| `vscreen_navigate` | Navigate to a URL with optional wait condition |
| `vscreen_go_back` | Navigate back in browser history |
| `vscreen_go_forward` | Navigate forward in browser history |
| `vscreen_reload` | Reload the current page |

## Synchronization Tools

| Tool | Description |
|------|-------------|
| `vscreen_wait` | Wait a fixed number of milliseconds |
| `vscreen_wait_for_idle` | Wait for no screencast frames for a duration |
| `vscreen_wait_for_text` | Wait for specific text to appear on the page |
| `vscreen_wait_for_selector` | Wait for a CSS selector to match an element |
| `vscreen_wait_for_url` | Wait for the URL to match a pattern |
| `vscreen_wait_for_network_idle` | Wait for network activity to stop |

## Utility Tools

| Tool | Description |
|------|-------------|
| `vscreen_execute_js` | Execute JavaScript in the main frame |
| `vscreen_find_input` | Find text inputs by placeholder, aria-label, label, role, or name |
| `vscreen_dismiss_dialogs` | Auto-dismiss cookie consent, GDPR, and privacy dialogs |
| `vscreen_dismiss_ads` | Dismiss video platform ad overlays (YouTube skip, etc.) |
| `vscreen_set_cookie` | Set a browser cookie |
| `vscreen_set_storage` | Set a localStorage or sessionStorage value |
| `vscreen_plan` | Get recommended tool sequence for a task (call before multi-step workflows) |
| `vscreen_help` | Get contextual documentation about any tool or topic |
"#;
