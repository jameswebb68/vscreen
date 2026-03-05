use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Tool parameter types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct InstanceIdParam {
    /// The instance ID to operate on (e.g. "dev")
    pub(crate) instance_id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct ClipRect {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct ScreenshotParam {
    /// The instance ID to operate on
    pub(crate) instance_id: String,
    /// Image format: "png" (default, best for AI vision), "jpeg", or "webp"
    #[serde(default = "default_png")]
    pub(crate) format: String,
    /// JPEG/WebP quality (0-100). Ignored for PNG.
    pub(crate) quality: Option<u32>,
    /// If true, captures the full scrollable page (not just the visible viewport).
    /// This temporarily resizes the browser viewport to the full document height.
    /// Useful for seeing all content on a page without scrolling.
    #[serde(default)]
    pub(crate) full_page: bool,
    /// Optional clip rectangle to capture only a specific region at full resolution.
    /// Useful for analyzing small areas (e.g. CAPTCHA grids) without scaling artifacts.
    #[serde(default)]
    pub(crate) clip: Option<ClipRect>,
    /// If true, overlay numbered bounding boxes on interactive elements (default: false)
    #[serde(default)]
    pub(crate) annotate: bool,
    /// CSS selector for annotation targets (only used when annotate=true)
    #[serde(default)]
    pub(crate) annotate_selector: Option<String>,
    /// Capture a sequence of screenshots. Set count and interval_ms.
    #[serde(default)]
    pub(crate) sequence_count: Option<u32>,
    /// Milliseconds between captures in sequence mode
    #[serde(default)]
    pub(crate) sequence_interval_ms: Option<u64>,
}

fn default_png() -> String {
    "png".into()
}

fn default_goto() -> String {
    "goto".into()
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct ConsolidatedNavigateParam {
    /// Action: "goto" (navigate to URL, default), "back" (browser back), "forward" (browser forward), "reload" (reload page)
    #[serde(default = "default_goto")]
    pub(crate) action: String,
    /// The instance ID to operate on
    pub(crate) instance_id: String,
    /// URL to navigate to (required for action="goto")
    #[serde(default)]
    pub(crate) url: Option<String>,
    /// When to consider navigation complete (for "goto"): "load" (default), "domcontentloaded", "networkidle", "none"
    #[serde(default)]
    pub(crate) wait_until: Option<String>,
}

fn default_duration() -> String {
    "duration".into()
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct ConsolidatedWaitParam {
    /// Wait condition: "duration" (fixed ms wait, default), "idle" (page idle), "text" (text appears), "selector" (element appears), "url" (URL matches), "network" (network idle)
    #[serde(default = "default_duration")]
    pub(crate) condition: String,
    /// Instance ID (not required for condition="duration")
    #[serde(default)]
    pub(crate) instance_id: Option<String>,
    /// Milliseconds to wait (for condition="duration")
    #[serde(default)]
    pub(crate) duration_ms: Option<u64>,
    /// Text to wait for (for condition="text")
    #[serde(default)]
    pub(crate) text: Option<String>,
    /// CSS selector to wait for (for condition="selector")
    #[serde(default)]
    pub(crate) selector: Option<String>,
    /// If true, wait for element to be visible (for condition="selector")
    #[serde(default)]
    pub(crate) visible: bool,
    /// URL substring to wait for (for condition="url")
    #[serde(default)]
    pub(crate) url_contains: Option<String>,
    /// Maximum wait time in milliseconds (default: 10000)
    #[serde(default)]
    pub(crate) timeout_ms: Option<u64>,
    /// Polling interval in milliseconds (default: 250)
    #[serde(default)]
    pub(crate) interval_ms: Option<u64>,
    /// Required idle duration in ms (for condition="idle" or "network", default: 500)
    #[serde(default)]
    pub(crate) idle_ms: Option<u64>,
}

fn default_single() -> String {
    "single".into()
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct ConsolidatedClickParam {
    /// Click mode: "single" (default) — coordinate click; "double" — double-click at coordinates; "element" — click by CSS/text; "navigate" — click + wait for URL change; "batch" — rapid multi-point clicks
    #[serde(default = "default_single")]
    pub(crate) mode: String,
    /// The instance ID to operate on
    pub(crate) instance_id: String,
    /// X coordinate in pixels (for mode="single" or "double")
    #[serde(default)]
    pub(crate) x: Option<f64>,
    /// Y coordinate in pixels (for mode="single" or "double")
    #[serde(default)]
    pub(crate) y: Option<f64>,
    /// Mouse button: 0=left (default), 1=middle, 2=right (for mode="single" or "element")
    #[serde(default)]
    pub(crate) button: Option<u8>,
    /// Wait this many milliseconds after clicking (for mode="single" or "element")
    #[serde(default)]
    pub(crate) wait_after_ms: Option<u64>,
    /// CSS selector (for mode="element" or "navigate")
    #[serde(default)]
    pub(crate) selector: Option<String>,
    /// Visible text to find and click (for mode="element" or "navigate")
    #[serde(default)]
    pub(crate) text: Option<String>,
    /// If true, match text exactly; otherwise match as substring (for mode="element", default: false)
    #[serde(default)]
    pub(crate) text_exact: bool,
    /// Which matching element to click (for mode="element", 0 = first match)
    #[serde(default)]
    pub(crate) index: Option<usize>,
    /// Number of retries if element is not found (for mode="element")
    #[serde(default)]
    pub(crate) retries: Option<u32>,
    /// Delay between retries in milliseconds (for mode="element")
    #[serde(default)]
    pub(crate) retry_delay_ms: Option<u64>,
    /// Timeout in milliseconds to wait for URL change (for mode="navigate")
    #[serde(default)]
    pub(crate) timeout_ms: Option<u64>,
    /// If true and the initial click doesn't navigate, try clicking the nearest <a> ancestor (for mode="navigate", default: true)
    #[serde(default = "default_true")]
    pub(crate) fallback_to_link: bool,
    /// Array of [x, y] coordinate pairs (for mode="batch")
    #[serde(default)]
    pub(crate) points: Option<Vec<[f64; 2]>>,
    /// Delay between clicks in milliseconds (for mode="batch")
    #[serde(default)]
    pub(crate) delay_between_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct TypeParam {
    /// The instance ID to operate on
    pub(crate) instance_id: String,
    /// Text to type/paste into the focused element
    pub(crate) text: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct KeyPressParam {
    /// The instance ID to operate on
    pub(crate) instance_id: String,
    /// Key name (e.g. "Enter", "Escape", "Tab", "Backspace", "ArrowDown", "a", "1")
    pub(crate) key: String,
    /// Whether to hold Ctrl
    #[serde(default)]
    pub(crate) ctrl: bool,
    /// Whether to hold Shift
    #[serde(default)]
    pub(crate) shift: bool,
    /// Whether to hold Alt
    #[serde(default)]
    pub(crate) alt: bool,
    /// Whether to hold Meta/Win
    #[serde(default)]
    pub(crate) meta: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct KeyComboParam {
    /// The instance ID to operate on
    pub(crate) instance_id: String,
    /// Keys to press simultaneously (e.g. ["Control", "a"] for Ctrl+A). Last key is the action key.
    pub(crate) keys: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct ScrollParam {
    /// The instance ID to operate on
    pub(crate) instance_id: String,
    /// X coordinate of scroll position (page coordinates, auto-scrolls if needed)
    pub(crate) x: f64,
    /// Y coordinate of scroll position (page coordinates, auto-scrolls if needed)
    pub(crate) y: f64,
    /// Horizontal scroll amount (positive=right)
    #[serde(default)]
    pub(crate) delta_x: f64,
    /// Vertical scroll amount (positive=down, negative=up). Typical: 120 per notch.
    pub(crate) delta_y: f64,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct DragParam {
    /// The instance ID to operate on
    pub(crate) instance_id: String,
    /// Start X coordinate (page coordinates, auto-scrolls to start position)
    pub(crate) from_x: f64,
    /// Start Y coordinate (page coordinates, auto-scrolls to start position)
    pub(crate) from_y: f64,
    /// End X coordinate (page coordinates)
    pub(crate) to_x: f64,
    /// End Y coordinate (page coordinates)
    pub(crate) to_y: f64,
    /// Number of intermediate mouse-move steps (default: 10)
    #[serde(default)]
    pub(crate) steps: Option<u32>,
    /// Duration of drag in milliseconds (default: 300)
    #[serde(default)]
    pub(crate) duration_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct HoverParam {
    /// The instance ID to operate on
    pub(crate) instance_id: String,
    /// X coordinate to move mouse to (page coordinates, auto-scrolls if needed)
    pub(crate) x: f64,
    /// Y coordinate to move mouse to (page coordinates, auto-scrolls if needed)
    pub(crate) y: f64,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct ExecuteJsParam {
    /// The instance ID to operate on
    pub(crate) instance_id: String,
    /// JavaScript expression to evaluate
    pub(crate) expression: String,
}

// -- Phase 1a: Screenshot history params --

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct ConsolidatedHistoryParam {
    /// Action: "list" (list entries), "get" (get single entry by index), "range" (get range), "clear" (clear history)
    pub(crate) action: String,
    /// The instance ID to operate on
    pub(crate) instance_id: String,
    /// Index for action="get" (0 = oldest)
    #[serde(default)]
    pub(crate) index: Option<usize>,
    /// Start index for action="range"
    #[serde(default)]
    pub(crate) from: Option<usize>,
    /// Count for action="range"
    #[serde(default)]
    pub(crate) count: Option<usize>,
}

// -- Phase 1b: Action log params --

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct SessionLogParam {
    /// The instance ID to operate on
    pub(crate) instance_id: String,
    /// Maximum number of recent entries to return (default: all)
    #[serde(default)]
    pub(crate) last_n: Option<usize>,
}

// -- Phase 2a: Element discovery params --

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct DescribeElementsParam {
    /// The instance ID to operate on
    pub(crate) instance_id: String,
    /// CSS selector to query (default: "button, [role='button'], a, input, select")
    pub(crate) selector: Option<String>,
    /// If true, describe ALL elements including those with labels (default: false — only unlabeled)
    #[serde(default)]
    pub(crate) include_labeled: bool,
}

// -- Phase 4a: Navigation params --

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct ExtractTextParam {
    /// The instance ID to operate on
    pub(crate) instance_id: String,
    /// Optional CSS selector to extract text from. If omitted, extracts full page text.
    #[serde(default)]
    pub(crate) selector: Option<String>,
}

// -- Phase 1c: Console params --

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct ConsoleLogParam {
    /// The instance ID to operate on
    pub(crate) instance_id: String,
    /// Filter by level: "log", "warn", "error", "info". If omitted, returns all.
    #[serde(default)]
    pub(crate) level: Option<String>,
}

// -- Phase 2c: Accessibility tree params --

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct AccessibilityTreeParam {
    /// The instance ID to operate on
    pub(crate) instance_id: String,
    /// Maximum depth to traverse (default: 5)
    #[serde(default)]
    pub(crate) max_depth: Option<u32>,
}

// -- Phase 4c: Cookie/Storage params --

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct ConsolidatedStorageParam {
    /// Storage type: "cookie", "local" (localStorage), "session" (sessionStorage)
    #[serde(rename = "type")]
    pub(crate) storage_type: String,
    /// Action: "get" or "set"
    pub(crate) action: String,
    /// The instance ID to operate on
    pub(crate) instance_id: String,
    /// Key to get/set (for local/session storage)
    #[serde(default)]
    pub(crate) key: Option<String>,
    /// Value to set
    #[serde(default)]
    pub(crate) value: Option<String>,
    /// Cookie name (for type="cookie", action="set")
    #[serde(default)]
    pub(crate) name: Option<String>,
    /// Cookie domain (for type="cookie", action="set")
    #[serde(default)]
    pub(crate) domain: Option<String>,
    /// Cookie path (for type="cookie", action="set")
    #[serde(default)]
    pub(crate) path: Option<String>,
}

// -- New high-impact tools params --

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct DismissDialogsParam {
    /// The instance ID to operate on
    pub(crate) instance_id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct SolveCaptchaParam {
    /// The instance ID to operate on
    pub(crate) instance_id: String,
    /// Maximum number of full page-reload attempts before giving up (default: 3)
    #[serde(default)]
    pub(crate) max_attempts: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct FillParam {
    /// The instance ID to operate on
    pub(crate) instance_id: String,
    /// CSS selector for the input element to fill
    pub(crate) selector: String,
    /// Text value to fill into the element
    pub(crate) value: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct SelectOptionParam {
    /// The instance ID to operate on
    pub(crate) instance_id: String,
    /// CSS selector for the select element
    pub(crate) selector: String,
    /// Option value attribute to select
    #[serde(default)]
    pub(crate) value: Option<String>,
    /// Option visible text (label) to select
    #[serde(default)]
    pub(crate) label: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct ScrollToElementParam {
    /// The instance ID to operate on
    pub(crate) instance_id: String,
    /// CSS selector for the element to scroll into view
    pub(crate) selector: String,
    /// Scroll alignment: "center" (default), "start", "end", "nearest"
    #[serde(default)]
    pub(crate) block: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct ConsolidatedFindParam {
    /// Search mode: "selector" (CSS selector), "text" (visible text), "input" (form inputs)
    pub(crate) by: String,
    /// The instance ID to operate on
    pub(crate) instance_id: String,
    /// CSS selector (for by="selector")
    #[serde(default)]
    pub(crate) selector: Option<String>,
    /// Text to search for (for by="text")
    #[serde(default)]
    pub(crate) text: Option<String>,
    /// Exact text match (for by="text", default: false)
    #[serde(default)]
    pub(crate) exact: bool,
    /// Search inside iframes too (for by="selector" or "text", default: false)
    #[serde(default)]
    pub(crate) include_iframes: bool,
    /// Search by placeholder text (for by="input")
    #[serde(default)]
    pub(crate) placeholder: Option<String>,
    /// Search by aria-label (for by="input")
    #[serde(default)]
    pub(crate) aria_label: Option<String>,
    /// Search by label text (for by="input")
    #[serde(default)]
    pub(crate) label: Option<String>,
    /// Search by role attribute (for by="input")
    #[serde(default)]
    pub(crate) role: Option<String>,
    /// Search by name attribute (for by="input")
    #[serde(default)]
    pub(crate) name: Option<String>,
    /// Search by input type (for by="input")
    #[serde(default)]
    pub(crate) input_type: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct DismissAdsParam {
    /// The instance ID to operate on
    pub(crate) instance_id: String,
    /// Timeout in milliseconds to wait for skip button to appear (default: 15000). Video ads often have a countdown before the skip button appears.
    #[serde(default)]
    pub(crate) timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct ListFramesParam {
    /// The instance ID to operate on
    pub(crate) instance_id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct PlanTaskParam {
    /// Short description of the task you want to accomplish, e.g. "read all text on the page",
    /// "click the Sign In button", "fill out a login form", "navigate to a URL and extract data".
    pub(crate) task: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct HelpParam {
    /// Topic to get help on. Can be a tool name (e.g., "vscreen_batch_click"),
    /// a concept ("coordinates", "iframes", "locking", "workflows"),
    /// or "tools" for the full tool reference. Use "quickstart" for a getting-started guide.
    pub(crate) topic: String,
}

// -- Lock management params --

fn default_lock_type() -> String {
    "exclusive".into()
}

fn default_ttl() -> u64 {
    120
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct ConsolidatedLockParam {
    /// Action: "acquire" (get a lock), "release" (unlock), "renew" (extend TTL), "status" (query status)
    pub(crate) action: String,
    /// The instance ID (required for all actions except status with no args)
    #[serde(default)]
    pub(crate) instance_id: Option<String>,
    /// Lock type for acquire: "exclusive" (default) or "observer"
    #[serde(default = "default_lock_type")]
    pub(crate) lock_type: String,
    /// Agent name (for acquire)
    #[serde(default)]
    pub(crate) agent_name: Option<String>,
    /// TTL in seconds (for acquire/renew, default: 120)
    #[serde(default = "default_ttl")]
    pub(crate) ttl_seconds: u64,
    /// Seconds to wait if locked (for acquire, default: 0 = fail immediately)
    #[serde(default)]
    pub(crate) wait_timeout_seconds: u64,
    /// Lock token (for release/renew if session changed)
    #[serde(default)]
    pub(crate) lock_token: Option<String>,
}

// -- Audio / RTSP params --

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct AudioStreamsParam {
    /// The instance ID to list audio streams for
    pub(crate) instance_id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct AudioStreamInfoParam {
    /// The instance ID
    pub(crate) instance_id: String,
    /// The RTSP session ID to get info for
    pub(crate) session_id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct AudioHealthParam {
    /// The instance ID to get audio health for
    pub(crate) instance_id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct RtspTeardownParam {
    /// The instance ID
    pub(crate) instance_id: String,
    /// The RTSP session ID to tear down
    pub(crate) session_id: String,
}


// ---------------------------------------------------------------------------
// Synthesis tool parameter structs
// ---------------------------------------------------------------------------

/// Consolidated parameter for vscreen_synthesis_manage. Action determines which fields are required.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct SynthesisManageParam {
    /// Action: "create", "update", "delete", "list", "push", "save", "navigate"
    pub(crate) action: String,
    /// Page title (required for action="create")
    #[serde(default)]
    pub(crate) title: Option<String>,
    /// Optional subtitle (for create/update)
    #[serde(default)]
    pub(crate) subtitle: Option<String>,
    /// Theme: 'dark' or 'light' (for create/update)
    #[serde(default)]
    pub(crate) theme: Option<String>,
    /// Layout: 'grid', 'list', 'split', 'tabs', or 'freeform' (for create/update)
    #[serde(default)]
    pub(crate) layout: Option<String>,
    /// Initial sections array (for create). Each section: {id, component, title?, data, meta?}.
    #[serde(default)]
    pub(crate) sections: Option<serde_json::Value>,
    /// If set, navigate this browser instance to the new page after creation (for action="create")
    #[serde(default)]
    pub(crate) navigate_instance: Option<String>,
    /// Target page ID (required for update, delete, push, save, navigate)
    #[serde(default)]
    pub(crate) page_id: Option<String>,
    /// Target section ID (required for action="push")
    #[serde(default)]
    pub(crate) section_id: Option<String>,
    /// Data items to append (required for action="push")
    #[serde(default)]
    pub(crate) data: Option<serde_json::Value>,
    /// Browser instance ID (required for action="navigate")
    #[serde(default)]
    pub(crate) instance_id: Option<String>,
    /// Page slug/ID to navigate to (required for action="navigate")
    #[serde(default)]
    pub(crate) page_slug: Option<String>,
}

/// Consolidated parameter for vscreen_synthesis_scrape. Mode determines which fields are required.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct SynthesisScrapeConsolidatedParam {
    /// Mode: "single" (default) — scrape one URL; "batch" — scrape multiple URLs in parallel
    #[serde(default = "default_synthesis_scrape_mode")]
    pub(crate) mode: String,
    /// Browser instance ID (required)
    pub(crate) instance_id: String,
    /// URL to scrape (required for mode="single")
    #[serde(default)]
    pub(crate) url: Option<String>,
    /// Maximum articles to extract (for mode="single", default: 8)
    #[serde(default)]
    pub(crate) limit: Option<usize>,
    /// Source badge label (for mode="single", e.g. 'CNN')
    #[serde(default)]
    pub(crate) source_label: Option<String>,
    /// Array of URLs to scrape (required for mode="batch"). Each: {url, limit?, source_label?}
    #[serde(default)]
    pub(crate) urls: Option<Vec<SynthesisScrapeUrlEntry>>,
}

fn default_synthesis_scrape_mode() -> String {
    "single".into()
}

/// A single URL entry for batch scraping.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct SynthesisScrapeUrlEntry {
    /// URL to scrape
    pub(crate) url: String,
    /// Maximum articles to extract from this URL (default: 8)
    pub(crate) limit: Option<usize>,
    /// Source badge label (e.g. 'CNN')
    pub(crate) source_label: Option<String>,
}

// ---------------------------------------------------------------------------
// Workflow tool params (Layer-1)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct BrowseParam {
    /// The instance ID to operate on
    pub(crate) instance_id: String,
    /// URL to navigate to
    pub(crate) url: String,
    /// Optional text or CSS selector to wait for after load (poll up to 10s)
    #[serde(default)]
    pub(crate) wait_for: Option<String>,
    /// If true, also return page text (default: false)
    #[serde(default)]
    pub(crate) extract_text: Option<bool>,
    /// If true, capture full-page screenshot (default: false)
    #[serde(default)]
    pub(crate) full_page: Option<bool>,
    /// If true, try to dismiss cookie/consent dialogs (default: true)
    #[serde(default)]
    pub(crate) dismiss_dialogs: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct ObserveParam {
    /// The instance ID to operate on
    pub(crate) instance_id: String,
    /// If true, capture full-page screenshot (default: false)
    #[serde(default)]
    pub(crate) full_page: Option<bool>,
    /// If true, include visible text (default: true)
    #[serde(default)]
    pub(crate) include_text: Option<bool>,
    /// If true, include interactive elements summary (default: false)
    #[serde(default)]
    pub(crate) include_elements: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct ExtractParam {
    /// The instance ID to operate on
    pub(crate) instance_id: String,
    /// Extraction mode: "articles", "table", "kv", "stats", "links", "text", "auto" (default: "auto")
    #[serde(default = "default_extract_mode")]
    pub(crate) mode: String,
    /// Optional CSS selector to scope extraction
    #[serde(default)]
    pub(crate) selector: Option<String>,
}

fn default_extract_mode() -> String {
    "auto".into()
}

// ---------------------------------------------------------------------------
// Synthesis tool parameter structs
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Workflow interact params (Layer-1)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct InteractParam {
    /// The instance ID to operate on
    pub(crate) instance_id: String,
    /// Action: "click", "type", "select", "hover", "scroll"
    pub(crate) action: String,
    /// Text or CSS selector to find the target element
    #[serde(default)]
    pub(crate) target: Option<String>,
    /// Target resolution: "text" (default), "selector", "coordinates"
    #[serde(default)]
    pub(crate) target_type: Option<String>,
    /// For type/select: the value to input
    #[serde(default)]
    pub(crate) value: Option<String>,
    /// For coordinates-based targeting: X coordinate
    #[serde(default)]
    pub(crate) x: Option<f64>,
    /// For coordinates-based targeting: Y coordinate
    #[serde(default)]
    pub(crate) y: Option<f64>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct SynthesizeParam {
    /// Action: "scrape_and_create", "create", "list"
    pub(crate) action: String,
    /// Browser instance ID (required for scrape_and_create)
    #[serde(default)]
    pub(crate) instance_id: Option<String>,
    /// Page title (for create/scrape_and_create)
    #[serde(default)]
    pub(crate) title: Option<String>,
    /// Page subtitle
    #[serde(default)]
    pub(crate) subtitle: Option<String>,
    /// Theme: "dark" (default) or "light"
    #[serde(default)]
    pub(crate) theme: Option<String>,
    /// URLs to scrape (for scrape_and_create)
    #[serde(default)]
    pub(crate) urls: Option<Vec<SynthesisScrapeUrlEntry>>,
    /// Sections JSON (for create)
    #[serde(default)]
    pub(crate) sections: Option<serde_json::Value>,
    /// Layout (for create)
    #[serde(default)]
    pub(crate) layout: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct SolveChallengeParam {
    /// The instance ID to operate on
    pub(crate) instance_id: String,
    /// Challenge type: "auto" (default), "captcha", "cookie_consent", "ad"
    #[serde(default)]
    pub(crate) challenge_type: Option<String>,
}

// ---------------------------------------------------------------------------
// Synthesis scrape-and-create param
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub(crate) struct SynthesisScrapeAndCreateParam {
    /// Browser instance ID (used to spawn ephemeral tabs and navigate to the result)
    pub(crate) instance_id: String,
    /// Page title (required). Example: "Global News Roundup"
    pub(crate) title: String,
    /// Page subtitle. Example: "Top stories from CNN, BBC, Reuters — March 2, 2026"
    pub(crate) subtitle: Option<String>,
    /// Page theme: 'dark' (default) or 'light'
    pub(crate) theme: Option<String>,
    /// Page layout: 'grid' (default), 'list', 'split', 'tabs', 'freeform'
    pub(crate) layout: Option<String>,
    /// Array of URLs to scrape in parallel, each with optional limit and source_label.
    /// Each URL becomes one section on the page, titled with its source_label.
    /// Example: [{"url": "https://cnn.com", "limit": 10, "source_label": "CNN"}]
    pub(crate) urls: Vec<SynthesisScrapeUrlEntry>,
    /// Component type for sections. Default: auto-selected based on article count
    /// (1-3: 'hero', 4-12: 'card-grid', 13+: 'content-list'). Override with e.g. 'card-grid'.
    pub(crate) component: Option<String>,
    /// Whether to navigate the browser to the new page after creation (default: true)
    pub(crate) navigate: Option<bool>,
}


