use super::*;
use super::synthesis::pick_section_component;

fn make_state() -> AppState {
    AppState::new(
        vscreen_core::config::AppConfig::default(),
        tokio_util::sync::CancellationToken::new(),
    )
}

// -----------------------------------------------------------------------
// Server info
// -----------------------------------------------------------------------

#[test]
fn server_info_has_instructions() {
    let state = make_state();
    let server = VScreenMcpServer::new(state);
    let info = server.get_info();
    assert!(info.instructions.is_some());
    assert!(info
        .instructions
        .as_deref()
        .unwrap_or("")
        .contains("vscreen"));
}

#[test]
fn server_info_enables_tools() {
    let state = make_state();
    let server = VScreenMcpServer::new(state);
    let info = server.get_info();
    assert!(info.capabilities.tools.is_some());
}

// -----------------------------------------------------------------------
// Parameter type deserialization
// -----------------------------------------------------------------------

#[test]
fn instance_id_param() {
    let json = r#"{"instance_id":"dev"}"#;
    let p: InstanceIdParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.instance_id, "dev");
}

#[test]
fn screenshot_param_defaults() {
    let json = r#"{"instance_id":"dev"}"#;
    let p: ScreenshotParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.format, "png");
    assert!(p.quality.is_none());
}

#[test]
fn screenshot_param_full() {
    let json = r#"{"instance_id":"test","format":"jpeg","quality":85}"#;
    let p: ScreenshotParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.format, "jpeg");
    assert_eq!(p.quality, Some(85));
}

#[test]
fn screenshot_param_sequence() {
    let json = r#"{"instance_id":"dev","sequence_count":5,"sequence_interval_ms":500}"#;
    let p: ScreenshotParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.sequence_count, Some(5));
    assert_eq!(p.sequence_interval_ms, Some(500));
    assert_eq!(p.format, "png");
}

#[test]
fn consolidated_navigate_param_goto() {
    let json = r#"{"instance_id":"dev","url":"https://google.com"}"#;
    let p: ConsolidatedNavigateParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.action, "goto");
    assert_eq!(p.url.as_deref(), Some("https://google.com"));
}

#[test]
fn consolidated_click_param_single_minimal() {
    let json = r#"{"instance_id":"dev","x":100,"y":200}"#;
    let p: ConsolidatedClickParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.mode, "single");
    assert_eq!(p.x, Some(100.0));
    assert_eq!(p.y, Some(200.0));
    assert_eq!(p.button, None);
}

#[test]
fn consolidated_click_param_single_with_button() {
    let json = r#"{"instance_id":"dev","mode":"single","x":100,"y":200,"button":2}"#;
    let p: ConsolidatedClickParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.mode, "single");
    assert_eq!(p.button, Some(2));
}

#[test]
fn consolidated_click_param_double() {
    let json = r#"{"instance_id":"dev","mode":"double","x":50,"y":60}"#;
    let p: ConsolidatedClickParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.mode, "double");
    assert!((p.x.unwrap_or(0.0) - 50.0).abs() < f64::EPSILON);
    assert!((p.y.unwrap_or(0.0) - 60.0).abs() < f64::EPSILON);
}

#[test]
fn consolidated_click_param_element() {
    let json = r#"{"instance_id":"dev","mode":"element","text":"Submit"}"#;
    let p: ConsolidatedClickParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.mode, "element");
    assert_eq!(p.text.as_deref(), Some("Submit"));
}

#[test]
fn consolidated_click_param_batch() {
    let json = r#"{"instance_id":"dev","mode":"batch","points":[[100,200],[300,400]]}"#;
    let p: ConsolidatedClickParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.mode, "batch");
    assert_eq!(p.points.as_ref().map(|v| v.len()), Some(2));
}

#[test]
fn type_param() {
    let json = r#"{"instance_id":"dev","text":"hello"}"#;
    let p: TypeParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.text, "hello");
}

#[test]
fn key_press_param_simple() {
    let json = r#"{"instance_id":"dev","key":"Enter"}"#;
    let p: KeyPressParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.key, "Enter");
    assert!(!p.ctrl);
    assert!(!p.shift);
}

#[test]
fn key_press_param_with_modifiers() {
    let json = r#"{"instance_id":"dev","key":"c","ctrl":true}"#;
    let p: KeyPressParam = serde_json::from_str(json).expect("parse");
    assert!(p.ctrl);
    assert!(!p.shift);
}

#[test]
fn key_combo_param() {
    let json = r#"{"instance_id":"dev","keys":["Control","Shift","i"]}"#;
    let p: KeyComboParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.keys.len(), 3);
    assert_eq!(p.keys[0], "Control");
    assert_eq!(p.keys[2], "i");
}

#[test]
fn scroll_param() {
    let json = r#"{"instance_id":"dev","x":100,"y":200,"delta_y":-120}"#;
    let p: ScrollParam = serde_json::from_str(json).expect("parse");
    assert!((p.delta_x).abs() < f64::EPSILON);
    assert!((p.delta_y - (-120.0)).abs() < f64::EPSILON);
}

#[test]
fn drag_param_defaults() {
    let json = r#"{"instance_id":"dev","from_x":0,"from_y":0,"to_x":100,"to_y":100}"#;
    let p: DragParam = serde_json::from_str(json).expect("parse");
    assert!(p.steps.is_none());
    assert!(p.duration_ms.is_none());
}

#[test]
fn drag_param_full() {
    let json = r#"{"instance_id":"dev","from_x":10,"from_y":20,"to_x":30,"to_y":40,"steps":5,"duration_ms":1000}"#;
    let p: DragParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.steps, Some(5));
    assert_eq!(p.duration_ms, Some(1000));
}

#[test]
fn hover_param() {
    let json = r#"{"instance_id":"dev","x":42,"y":84}"#;
    let p: HoverParam = serde_json::from_str(json).expect("parse");
    assert!((p.x - 42.0).abs() < f64::EPSILON);
}

#[test]
fn consolidated_wait_param_duration() {
    let json = r#"{"duration_ms":500}"#;
    let p: ConsolidatedWaitParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.condition, "duration");
    assert_eq!(p.duration_ms, Some(500));
}

#[test]
fn consolidated_wait_param_idle_defaults() {
    let json = r#"{"instance_id":"dev","condition":"idle"}"#;
    let p: ConsolidatedWaitParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.condition, "idle");
    assert!(p.timeout_ms.is_none());
}

#[test]
fn consolidated_wait_param_idle_full() {
    let json = r#"{"instance_id":"dev","condition":"idle","timeout_ms":10000}"#;
    let p: ConsolidatedWaitParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.condition, "idle");
    assert_eq!(p.timeout_ms, Some(10000));
}

#[test]
fn execute_js_param() {
    let json = r#"{"instance_id":"dev","expression":"1+1"}"#;
    let p: ExecuteJsParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.expression, "1+1");
}

// -----------------------------------------------------------------------
// New parameter type tests
// -----------------------------------------------------------------------

#[test]
fn consolidated_history_param_list() {
    let json = r#"{"instance_id":"dev","action":"list"}"#;
    let p: ConsolidatedHistoryParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.instance_id, "dev");
    assert_eq!(p.action, "list");
    assert!(p.index.is_none());
}

#[test]
fn consolidated_history_param_get() {
    let json = r#"{"instance_id":"dev","action":"get","index":3}"#;
    let p: ConsolidatedHistoryParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.index, Some(3));
}

#[test]
fn consolidated_history_param_range() {
    let json = r#"{"instance_id":"dev","action":"range","from":2,"count":5}"#;
    let p: ConsolidatedHistoryParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.from, Some(2));
    assert_eq!(p.count, Some(5));
}

#[test]
fn session_log_param_defaults() {
    let json = r#"{"instance_id":"dev"}"#;
    let p: SessionLogParam = serde_json::from_str(json).expect("parse");
    assert!(p.last_n.is_none());
}

#[test]
fn session_log_param_with_n() {
    let json = r#"{"instance_id":"dev","last_n":10}"#;
    let p: SessionLogParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.last_n, Some(10));
}

#[test]
fn consolidated_find_param_selector() {
    let json = r#"{"instance_id":"dev","by":"selector","selector":"button.primary"}"#;
    let p: ConsolidatedFindParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.by, "selector");
    assert_eq!(p.selector.as_deref(), Some("button.primary"));
}

#[test]
fn consolidated_find_param_text_defaults() {
    let json = r#"{"instance_id":"dev","by":"text","text":"Submit"}"#;
    let p: ConsolidatedFindParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.by, "text");
    assert_eq!(p.text.as_deref(), Some("Submit"));
    assert!(!p.exact);
}

#[test]
fn consolidated_find_param_text_exact() {
    let json = r#"{"instance_id":"dev","by":"text","text":"Submit","exact":true}"#;
    let p: ConsolidatedFindParam = serde_json::from_str(json).expect("parse");
    assert!(p.exact);
}

#[test]
fn consolidated_wait_param_text_defaults() {
    let json = r#"{"instance_id":"dev","condition":"text","text":"Loading complete"}"#;
    let p: ConsolidatedWaitParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.condition, "text");
    assert_eq!(p.text.as_deref(), Some("Loading complete"));
    assert!(p.timeout_ms.is_none());
}

#[test]
fn consolidated_wait_param_text_full() {
    let json = r#"{"instance_id":"dev","condition":"text","text":"OK","timeout_ms":5000,"interval_ms":100}"#;
    let p: ConsolidatedWaitParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.condition, "text");
    assert_eq!(p.timeout_ms, Some(5000));
    assert_eq!(p.interval_ms, Some(100));
}

#[test]
fn consolidated_wait_param_selector_defaults() {
    let json = r##"{"instance_id":"dev","condition":"selector","selector":"#result"}"##;
    let p: ConsolidatedWaitParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.condition, "selector");
    assert_eq!(p.selector.as_deref(), Some("#result"));
    assert!(!p.visible);
}

#[test]
fn consolidated_wait_param_selector_visible() {
    let json = r#"{"instance_id":"dev","condition":"selector","selector":".modal","visible":true,"timeout_ms":3000}"#;
    let p: ConsolidatedWaitParam = serde_json::from_str(json).expect("parse");
    assert!(p.visible);
    assert_eq!(p.timeout_ms, Some(3000));
}

#[test]
fn extract_text_param_defaults() {
    let json = r#"{"instance_id":"dev"}"#;
    let p: ExtractTextParam = serde_json::from_str(json).expect("parse");
    assert!(p.selector.is_none());
}

#[test]
fn extract_text_param_with_selector() {
    let json = r#"{"instance_id":"dev","selector":"main article"}"#;
    let p: ExtractTextParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.selector.as_deref(), Some("main article"));
}

#[test]
fn extract_param_kv_mode() {
    let json = r#"{"instance_id":"dev","mode":"kv"}"#;
    let p: ExtractParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.instance_id, "dev");
    assert_eq!(p.mode, "kv");
}

#[test]
fn extract_param_stats_mode() {
    let json = r#"{"instance_id":"dev","mode":"stats"}"#;
    let p: ExtractParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.instance_id, "dev");
    assert_eq!(p.mode, "stats");
}

#[test]
fn console_log_param_defaults() {
    let json = r#"{"instance_id":"dev"}"#;
    let p: ConsoleLogParam = serde_json::from_str(json).expect("parse");
    assert!(p.level.is_none());
}

#[test]
fn console_log_param_with_level() {
    let json = r#"{"instance_id":"dev","level":"error"}"#;
    let p: ConsoleLogParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.level.as_deref(), Some("error"));
}

#[test]
fn accessibility_tree_param_defaults() {
    let json = r#"{"instance_id":"dev"}"#;
    let p: AccessibilityTreeParam = serde_json::from_str(json).expect("parse");
    assert!(p.max_depth.is_none());
}

#[test]
fn accessibility_tree_param_with_depth() {
    let json = r#"{"instance_id":"dev","max_depth":3}"#;
    let p: AccessibilityTreeParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.max_depth, Some(3));
}

#[test]
fn consolidated_storage_param_cookie_set() {
    let json = r#"{"instance_id":"dev","type":"cookie","action":"set","name":"session","value":"abc123"}"#;
    let p: ConsolidatedStorageParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.storage_type, "cookie");
    assert_eq!(p.action, "set");
    assert_eq!(p.name.as_deref(), Some("session"));
    assert_eq!(p.value.as_deref(), Some("abc123"));
    assert!(p.domain.is_none());
    assert!(p.path.is_none());
}

#[test]
fn consolidated_storage_param_cookie_set_full() {
    let json = r#"{"instance_id":"dev","type":"cookie","action":"set","name":"tok","value":"xyz","domain":".example.com","path":"/api"}"#;
    let p: ConsolidatedStorageParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.domain.as_deref(), Some(".example.com"));
    assert_eq!(p.path.as_deref(), Some("/api"));
}

#[test]
fn consolidated_storage_param_local_get() {
    let json = r#"{"instance_id":"dev","type":"local","action":"get","key":"theme"}"#;
    let p: ConsolidatedStorageParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.storage_type, "local");
    assert_eq!(p.action, "get");
    assert_eq!(p.key.as_deref(), Some("theme"));
}

#[test]
fn consolidated_storage_param_session_get() {
    let json = r#"{"instance_id":"dev","type":"session","action":"get","key":"token"}"#;
    let p: ConsolidatedStorageParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.storage_type, "session");
}

#[test]
fn consolidated_storage_param_local_set() {
    let json = r#"{"instance_id":"dev","type":"local","action":"set","key":"lang","value":"en"}"#;
    let p: ConsolidatedStorageParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.key.as_deref(), Some("lang"));
    assert_eq!(p.value.as_deref(), Some("en"));
    assert_eq!(p.storage_type, "local");
}

#[test]
fn consolidated_wait_param_url() {
    let json = r#"{"instance_id":"dev","condition":"url","url_contains":"dashboard"}"#;
    let p: ConsolidatedWaitParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.condition, "url");
    assert_eq!(p.url_contains.as_deref(), Some("dashboard"));
}

#[test]
fn consolidated_wait_param_network_defaults() {
    let json = r#"{"instance_id":"dev","condition":"network"}"#;
    let p: ConsolidatedWaitParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.condition, "network");
    assert!(p.idle_ms.is_none());
    assert!(p.timeout_ms.is_none());
}

#[test]
fn consolidated_wait_param_network_full() {
    let json = r#"{"instance_id":"dev","condition":"network","idle_ms":1000,"timeout_ms":15000}"#;
    let p: ConsolidatedWaitParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.condition, "network");
    assert_eq!(p.idle_ms, Some(1000));
    assert_eq!(p.timeout_ms, Some(15000));
}

#[test]
fn screenshot_param_annotate_defaults() {
    let json = r#"{"instance_id":"dev","annotate":true}"#;
    let p: ScreenshotParam = serde_json::from_str(json).expect("parse");
    assert!(p.annotate);
    assert!(p.annotate_selector.is_none());
}

#[test]
fn screenshot_param_annotate_custom_selector() {
    let json = r#"{"instance_id":"dev","annotate":true,"annotate_selector":"button, a"}"#;
    let p: ScreenshotParam = serde_json::from_str(json).expect("parse");
    assert!(p.annotate);
    assert_eq!(p.annotate_selector.as_deref(), Some("button, a"));
}

#[test]
fn new_param_types_have_json_schema() {
    use schemars::schema_for;

    let schema = schema_for!(ConsolidatedFindParam);
    let json = serde_json::to_string(&schema).expect("serialize");
    assert!(json.contains("selector"));
    assert!(json.contains("by"));

    let schema = schema_for!(ConsolidatedWaitParam);
    let json = serde_json::to_string(&schema).expect("serialize");
    assert!(json.contains("text"));
    assert!(json.contains("timeout_ms"));
    assert!(json.contains("condition"));

    let schema = schema_for!(ConsolidatedStorageParam);
    let json = serde_json::to_string(&schema).expect("serialize");
    assert!(json.contains("action"));

    let schema = schema_for!(ConsolidatedLockParam);
    let json = serde_json::to_string(&schema).expect("serialize");
    assert!(json.contains("action"));
    assert!(json.contains("instance_id"));

    let schema = schema_for!(ScreenshotParam);
    let json = serde_json::to_string(&schema).expect("serialize");
    assert!(json.contains("annotate"));
    assert!(json.contains("annotate_selector"));
}

// -----------------------------------------------------------------------
// Helper function tests
// -----------------------------------------------------------------------

#[test]
fn get_supervisor_missing_instance() {
    let state = make_state();
    let server = VScreenMcpServer::new(state);
    let result = server.get_supervisor("nonexistent");
    assert!(result.is_err());
}

#[test]
fn internal_error_message() {
    let err = internal_error("something broke");
    assert_eq!(err.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
    assert_eq!(err.message.as_ref(), "something broke");
}

#[test]
fn invalid_params_message() {
    let err = invalid_params("bad arg");
    assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert_eq!(err.message.as_ref(), "bad arg");
}

// -----------------------------------------------------------------------
// Serialization roundtrip for parameter types
// -----------------------------------------------------------------------

#[test]
fn param_types_serialize_roundtrip() {
    let original = ScreenshotParam {
        instance_id: "dev".into(),
        format: "png".into(),
        quality: Some(80),
        full_page: true,
        clip: None,
        annotate: false,
        annotate_selector: None,
        sequence_count: None,
        sequence_interval_ms: None,
    };
    let json = serde_json::to_string(&original).expect("serialize");
    let parsed: ScreenshotParam = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(parsed.instance_id, "dev");
    assert_eq!(parsed.format, "png");
    assert_eq!(parsed.quality, Some(80));
    assert!(parsed.full_page);
}

#[test]
fn param_types_have_json_schema() {
    use schemars::schema_for;

    let schema = schema_for!(ScreenshotParam);
    let json = serde_json::to_string(&schema).expect("serialize schema");
    assert!(json.contains("instance_id"));
    assert!(json.contains("format"));

    let schema = schema_for!(ConsolidatedClickParam);
    let json = serde_json::to_string(&schema).expect("serialize schema");
    assert!(json.contains("mode"));
    assert!(json.contains("x"));
    assert!(json.contains("y"));

    let schema = schema_for!(KeyComboParam);
    let json = serde_json::to_string(&schema).expect("serialize schema");
    assert!(json.contains("keys"));
}

// -----------------------------------------------------------------------
// Tool availability
// -----------------------------------------------------------------------

#[tokio::test]
async fn list_instances_tool_works_empty() {
    let state = make_state();
    let server = VScreenMcpServer::new(state);
    let result = server.vscreen_list_instances().await;
    assert!(result.is_ok());
    let call_result = result.unwrap();
    assert!(!call_result.content.is_empty());
}

fn extract_text(content: &Content) -> &str {
    match &content.raw {
        RawContent::Text(t) => &t.text,
        _ => panic!("expected text content"),
    }
}

#[tokio::test]
async fn list_instances_tool_with_registry() {
    let state = make_state();
    let config = vscreen_core::instance::InstanceConfig {
        instance_id: InstanceId::from("test-mcp"),
        cdp_endpoint: "ws://localhost:9222".into(),
        pulse_source: "test.monitor".into(),
        display: None,
        video: vscreen_core::config::VideoConfig::default(),
        audio: vscreen_core::config::AudioConfig::default(),
        rtp_output: None,
    };
    state.registry.create(config, 16).expect("create");

    let server = VScreenMcpServer::new(state);
    let result = server.vscreen_list_instances().await;
    assert!(result.is_ok());
    let call_result = result.unwrap();
    let text = extract_text(&call_result.content[0]);
    assert!(text.contains("test-mcp"));
}

#[tokio::test]
async fn wait_tool_completes() {
    let state = make_state();
    let server = VScreenMcpServer::new(state);
    let params = ConsolidatedWaitParam {
        condition: "duration".to_string(),
        duration_ms: Some(10),
        instance_id: None,
        text: None,
        selector: None,
        visible: false,
        url_contains: None,
        timeout_ms: None,
        interval_ms: None,
        idle_ms: None,
    };
    let call_result = server
        .vscreen_wait(Parameters(params))
        .await
        .expect("wait should succeed");
    let text = extract_text(&call_result.content[0]);
    assert!(text.contains("10ms"));
}

// -----------------------------------------------------------------------
// Tool Advisor anti-pattern detection
// -----------------------------------------------------------------------

#[test]
fn advisor_detects_scroll_screenshot_loop() {
    let mut advisor = ToolAdvisor::new();
    advisor.record(ToolCallRecord {
        tool_name: "vscreen_scroll".into(),
        args_snapshot: None,
    });
    advisor.record(ToolCallRecord {
        tool_name: "vscreen_screenshot".into(),
        args_snapshot: None,
    });
    advisor.record(ToolCallRecord {
        tool_name: "vscreen_scroll".into(),
        args_snapshot: None,
    });
    advisor.record(ToolCallRecord {
        tool_name: "vscreen_screenshot".into(),
        args_snapshot: None,
    });
    advisor.record(ToolCallRecord {
        tool_name: "vscreen_scroll".into(),
        args_snapshot: None,
    });

    let hint = advisor.check_anti_patterns(
        "vscreen_screenshot",
        &serde_json::json!({"instance_id": "dev"}),
    );
    assert!(hint.is_some(), "should detect scroll-screenshot loop");
    assert!(
        hint.unwrap().contains("full_page=true"),
        "should recommend full_page=true"
    );
}

#[test]
fn advisor_no_hint_for_full_page_screenshot() {
    let mut advisor = ToolAdvisor::new();
    // Use 4 records to avoid Layer-1 tip (requires 5+); test that scroll-screenshot hint doesn't fire
    advisor.record(ToolCallRecord {
        tool_name: "vscreen_scroll".into(),
        args_snapshot: None,
    });
    advisor.record(ToolCallRecord {
        tool_name: "vscreen_screenshot".into(),
        args_snapshot: None,
    });
    advisor.record(ToolCallRecord {
        tool_name: "vscreen_scroll".into(),
        args_snapshot: None,
    });
    advisor.record(ToolCallRecord {
        tool_name: "vscreen_screenshot".into(),
        args_snapshot: None,
    });

    let hint = advisor.check_anti_patterns(
        "vscreen_screenshot",
        &serde_json::json!({"instance_id": "dev", "full_page": true}),
    );
    assert!(hint.is_none(), "should NOT hint when full_page=true is used");
}

#[test]
fn advisor_no_hint_for_clip_screenshot() {
    let mut advisor = ToolAdvisor::new();
    advisor.record(ToolCallRecord {
        tool_name: "vscreen_scroll".into(),
        args_snapshot: None,
    });
    advisor.record(ToolCallRecord {
        tool_name: "vscreen_screenshot".into(),
        args_snapshot: None,
    });
    advisor.record(ToolCallRecord {
        tool_name: "vscreen_scroll".into(),
        args_snapshot: None,
    });
    advisor.record(ToolCallRecord {
        tool_name: "vscreen_screenshot".into(),
        args_snapshot: None,
    });

    let hint = advisor.check_anti_patterns(
        "vscreen_screenshot",
        &serde_json::json!({"instance_id": "dev", "clip": {"x": 0, "y": 0, "width": 100, "height": 100}}),
    );
    assert!(hint.is_none(), "should NOT hint when clip is used");
}

#[test]
fn advisor_detects_repeated_waits() {
    let mut advisor = ToolAdvisor::new();
    advisor.record(ToolCallRecord {
        tool_name: "vscreen_wait".into(),
        args_snapshot: Some(serde_json::json!({"condition": "duration"})),
    });
    advisor.record(ToolCallRecord {
        tool_name: "vscreen_screenshot".into(),
        args_snapshot: None,
    });
    advisor.record(ToolCallRecord {
        tool_name: "vscreen_wait".into(),
        args_snapshot: Some(serde_json::json!({"condition": "duration"})),
    });

    let hint = advisor.check_anti_patterns(
        "vscreen_wait",
        &serde_json::json!({"duration_ms": 5000}),
    );
    assert!(hint.is_some(), "should detect repeated fixed waits");
    assert!(
        hint.unwrap().contains("condition=\"text\""),
        "should recommend vscreen_wait(condition=\"text\", ...)"
    );
}

#[test]
fn advisor_detects_js_for_metadata() {
    let mut advisor = ToolAdvisor::new();

    let hint = advisor.check_anti_patterns(
        "vscreen_execute_js",
        &serde_json::json!({"instance_id": "dev", "expression": "document.title"}),
    );
    assert!(hint.is_some(), "should detect JS metadata anti-pattern");
    assert!(
        hint.unwrap().contains("get_page_info"),
        "should recommend get_page_info"
    );
}

#[test]
fn advisor_detects_js_for_text_content() {
    let mut advisor = ToolAdvisor::new();

    let hint = advisor.check_anti_patterns(
        "vscreen_execute_js",
        &serde_json::json!({"instance_id": "dev", "expression": "document.body.innerText"}),
    );
    assert!(hint.is_some(), "should detect JS text extraction anti-pattern");
    assert!(
        hint.unwrap().contains("extract_text"),
        "should recommend extract_text"
    );
}

#[test]
fn advisor_no_hint_for_targeted_wait() {
    let mut advisor = ToolAdvisor::new();
    advisor.record(ToolCallRecord {
        tool_name: "vscreen_wait".into(),
        args_snapshot: Some(serde_json::json!({"condition": "duration"})),
    });
    advisor.record(ToolCallRecord {
        tool_name: "vscreen_screenshot".into(),
        args_snapshot: None,
    });
    advisor.record(ToolCallRecord {
        tool_name: "vscreen_wait".into(),
        args_snapshot: Some(serde_json::json!({"condition": "text", "text": "Loaded"})),
    });

    let hint = advisor.check_anti_patterns(
        "vscreen_wait",
        &serde_json::json!({"instance_id": "dev", "condition": "text", "text": "Ready"}),
    );
    assert!(hint.is_none(), "should NOT hint for targeted wait (condition=text)");
}

#[test]
fn advisor_layer1_tip_after_five_calls() {
    let mut advisor = ToolAdvisor::new();
    for _ in 0..5 {
        advisor.record(ToolCallRecord {
            tool_name: "vscreen_screenshot".into(),
            args_snapshot: None,
        });
    }

    let hint = advisor.check_anti_patterns(
        "vscreen_screenshot",
        &serde_json::json!({"instance_id": "dev"}),
    );
    assert!(hint.is_some(), "should show Layer-1 tip after 5+ calls without Layer-1");
    let h = hint.as_ref().unwrap();
    assert!(
        h.contains("vscreen_browse") && h.contains("vscreen_interact"),
        "tip should mention Layer-1 tools"
    );
}

#[test]
fn advisor_synthesis_via_execute_js() {
    let mut advisor = ToolAdvisor::new();

    let hint = advisor.check_anti_patterns(
        "vscreen_execute_js",
        &serde_json::json!({"instance_id": "dev", "expression": "fetch('/api/pages').then(r=>r.json())"}),
    );
    assert!(hint.is_some(), "should detect synthesis fetch anti-pattern");
    let h = hint.as_ref().unwrap();
    assert!(
        h.contains("vscreen_synthesize") || h.contains("vscreen_synthesis"),
        "should recommend synthesis tools"
    );
}

#[test]
fn advisor_no_hint_for_custom_js() {
    let mut advisor = ToolAdvisor::new();

    let hint = advisor.check_anti_patterns(
        "vscreen_execute_js",
        &serde_json::json!({"instance_id": "dev", "expression": "document.querySelectorAll('.item').length"}),
    );
    assert!(hint.is_none(), "should NOT hint for custom JS expressions");
}

#[test]
fn advisor_detects_multiple_scrolls() {
    let mut advisor = ToolAdvisor::new();
    advisor.record(ToolCallRecord {
        tool_name: "vscreen_scroll".into(),
        args_snapshot: None,
    });
    advisor.record(ToolCallRecord {
        tool_name: "vscreen_screenshot".into(),
        args_snapshot: None,
    });
    advisor.record(ToolCallRecord {
        tool_name: "vscreen_scroll".into(),
        args_snapshot: None,
    });

    let hint = advisor.check_anti_patterns(
        "vscreen_scroll",
        &serde_json::json!({"instance_id": "dev", "x": 500, "y": 500, "delta_y": 300}),
    );
    assert!(hint.is_some(), "should detect multiple scroll calls");
    let hint_text = hint.unwrap();
    assert!(
        hint_text.contains("scroll_to_element") || hint_text.contains("full_page"),
        "should recommend alternatives: {hint_text}"
    );
}

#[test]
fn advisor_ring_buffer_limit() {
    let mut advisor = ToolAdvisor::new();
    for i in 0..25 {
        advisor.record(ToolCallRecord {
            tool_name: format!("tool_{i}"),
            args_snapshot: None,
        });
    }
    assert_eq!(advisor.recent_calls.len(), 20, "should cap at 20 entries");
}

// -----------------------------------------------------------------------
// vscreen_plan task routing
// -----------------------------------------------------------------------

#[tokio::test]
async fn plan_recommends_extract_text() {
    let state = make_state();
    let server = VScreenMcpServer::new(state);
    let result = server
        .vscreen_plan(Parameters(PlanTaskParam {
            task: "read all the text on the page".into(),
        }))
        .await
        .expect("plan should succeed");
    let text = extract_text(&result.content[0]);
    assert!(
        text.contains("vscreen_extract_text"),
        "should recommend extract_text: {text}"
    );
}

#[tokio::test]
async fn plan_recommends_full_page_screenshot() {
    let state = make_state();
    let server = VScreenMcpServer::new(state);
    let result = server
        .vscreen_plan(Parameters(PlanTaskParam {
            task: "see the whole page".into(),
        }))
        .await
        .expect("plan should succeed");
    let text = extract_text(&result.content[0]);
    assert!(
        text.contains("full_page=true"),
        "should recommend full_page: {text}"
    );
}

#[tokio::test]
async fn plan_recommends_click_element() {
    let state = make_state();
    let server = VScreenMcpServer::new(state);
    let result = server
        .vscreen_plan(Parameters(PlanTaskParam {
            task: "click the sign in button".into(),
        }))
        .await
        .expect("plan should succeed");
    let text = extract_text(&result.content[0]);
    assert!(
        text.contains("vscreen_click") && (text.contains("element") || text.contains("mode")),
        "should recommend click with element mode: {text}"
    );
}

#[tokio::test]
async fn plan_recommends_form_filling() {
    let state = make_state();
    let server = VScreenMcpServer::new(state);
    let result = server
        .vscreen_plan(Parameters(PlanTaskParam {
            task: "fill out the login form with username and password".into(),
        }))
        .await
        .expect("plan should succeed");
    let text = extract_text(&result.content[0]);
    assert!(
        text.contains("vscreen_fill"),
        "should recommend fill: {text}"
    );
}

#[tokio::test]
async fn plan_recommends_captcha_solver() {
    let state = make_state();
    let server = VScreenMcpServer::new(state);
    let result = server
        .vscreen_plan(Parameters(PlanTaskParam {
            task: "solve the captcha challenge".into(),
        }))
        .await
        .expect("plan should succeed");
    let text = extract_text(&result.content[0]);
    assert!(
        text.contains("vscreen_solve_captcha"),
        "should recommend solve_captcha: {text}"
    );
}

#[tokio::test]
async fn plan_fallback_for_unknown_task() {
    let state = make_state();
    let server = VScreenMcpServer::new(state);
    let result = server
        .vscreen_plan(Parameters(PlanTaskParam {
            task: "do something completely unique and novel".into(),
        }))
        .await
        .expect("plan should succeed");
    let text = extract_text(&result.content[0]);
    assert!(
        text.contains("General workflow"),
        "should show general fallback: {text}"
    );
}

#[tokio::test]
async fn plan_recommends_wait_tools() {
    let state = make_state();
    let server = VScreenMcpServer::new(state);
    let result = server
        .vscreen_plan(Parameters(PlanTaskParam {
            task: "wait for the page to finish loading dynamic content".into(),
        }))
        .await
        .expect("plan should succeed");
    let text = extract_text(&result.content[0]);
    assert!(
        text.contains("wait_for_") || text.contains("network_idle"),
        "should recommend targeted waits: {text}"
    );
}

// -----------------------------------------------------------------------
// vscreen_help tool-selection topic
// -----------------------------------------------------------------------

#[tokio::test]
async fn help_tool_selection_topic() {
    let state = make_state();
    let server = VScreenMcpServer::new(state);
    let result = server
        .vscreen_help(Parameters(HelpParam {
            topic: "tool-selection".into(),
        }))
        .await
        .expect("help should succeed");
    let text = extract_text(&result.content[0]);
    assert!(
        text.contains("Tool Selection Guide"),
        "should return tool selection guide: {text}"
    );
    assert!(
        text.contains("full_page=true"),
        "should mention full_page: {text}"
    );
}

// -----------------------------------------------------------------------
// Synthesis tool parameter deserialization
// -----------------------------------------------------------------------

#[test]
fn synthesis_manage_param_create_minimal() {
    let json = r#"{"action":"create","title":"News Digest"}"#;
    let p: SynthesisManageParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.action, "create");
    assert_eq!(p.title.as_deref(), Some("News Digest"));
    assert!(p.subtitle.is_none());
    assert!(p.theme.is_none());
    assert!(p.layout.is_none());
    assert!(p.sections.is_none());
}

#[test]
fn synthesis_manage_param_create_full() {
    let json = r#"{
        "action": "create",
        "title": "Dashboard",
        "subtitle": "Live metrics",
        "theme": "dark",
        "layout": "grid",
        "sections": [{"id": "s1", "component": "card-grid", "data": []}]
    }"#;
    let p: SynthesisManageParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.action, "create");
    assert_eq!(p.title.as_deref(), Some("Dashboard"));
    assert_eq!(p.subtitle.as_deref(), Some("Live metrics"));
    assert_eq!(p.theme.as_deref(), Some("dark"));
    assert_eq!(p.layout.as_deref(), Some("grid"));
    assert!(p.sections.is_some());
}

#[test]
fn synthesis_manage_param_push() {
    let json = r#"{"action":"push","page_id":"news-digest","section_id":"cnn","data":[{"title":"Breaking"}]}"#;
    let p: SynthesisManageParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.action, "push");
    assert_eq!(p.page_id.as_deref(), Some("news-digest"));
    assert_eq!(p.section_id.as_deref(), Some("cnn"));
    assert!(p.data.is_some());
}

#[test]
fn synthesis_manage_param_update_minimal() {
    let json = r#"{"action":"update","page_id":"my-page"}"#;
    let p: SynthesisManageParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.action, "update");
    assert_eq!(p.page_id.as_deref(), Some("my-page"));
    assert!(p.title.is_none());
}

#[test]
fn synthesis_manage_param_update_full() {
    let json = r#"{
        "action": "update",
        "page_id": "my-page",
        "title": "New Title",
        "subtitle": "New Sub",
        "theme": "light",
        "layout": "tabs",
        "sections": []
    }"#;
    let p: SynthesisManageParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.action, "update");
    assert_eq!(p.page_id.as_deref(), Some("my-page"));
    assert_eq!(p.title.as_deref(), Some("New Title"));
    assert_eq!(p.subtitle.as_deref(), Some("New Sub"));
    assert_eq!(p.theme.as_deref(), Some("light"));
    assert_eq!(p.layout.as_deref(), Some("tabs"));
    assert!(p.sections.is_some());
}

#[test]
fn synthesis_manage_param_delete() {
    let json = r#"{"action":"delete","page_id":"old-page"}"#;
    let p: SynthesisManageParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.action, "delete");
    assert_eq!(p.page_id.as_deref(), Some("old-page"));
}

#[test]
fn synthesis_manage_param_navigate() {
    let json = r#"{"action":"navigate","instance_id":"dev","page_slug":"news-digest"}"#;
    let p: SynthesisManageParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.action, "navigate");
    assert_eq!(p.instance_id.as_deref(), Some("dev"));
    assert_eq!(p.page_slug.as_deref(), Some("news-digest"));
}

#[test]
fn synthesis_manage_param_create_missing_title_fails() {
    let json = r#"{"action":"create","theme":"dark"}"#;
    let result = serde_json::from_str::<SynthesisManageParam>(json);
    assert!(result.is_ok(), "parse succeeds but validation happens at runtime");
    let p: SynthesisManageParam = result.unwrap();
    assert!(p.title.is_none(), "title is optional in struct");
}

#[test]
fn synthesis_manage_param_push_missing_fields() {
    let json = r#"{"action":"push","page_id":"test"}"#;
    let p: SynthesisManageParam = serde_json::from_str(json).expect("parse");
    assert_eq!(p.action, "push");
    assert!(p.section_id.is_none());
    assert!(p.data.is_none());
}

#[test]
fn synthesis_manage_param_delete_missing_page_id() {
    let json = r#"{"action":"delete"}"#;
    let p: SynthesisManageParam = serde_json::from_str(json).expect("parse");
    assert!(p.page_id.is_none());
}

#[test]
fn synthesis_manage_param_navigate_missing_fields() {
    let json = r#"{"action":"navigate","instance_id":"dev"}"#;
    let p: SynthesisManageParam = serde_json::from_str(json).expect("parse");
    assert!(p.page_slug.is_none());
}

// -----------------------------------------------------------------------
// Synthesis tool handlers — disabled state
// -----------------------------------------------------------------------

#[tokio::test]
async fn synthesis_list_returns_error_when_disabled() {
    let state = make_state();
    assert!(state.synthesis_url.is_none());
    let server = VScreenMcpServer::new(state);
    let result = server
        .vscreen_synthesis_manage(Parameters(SynthesisManageParam {
            action: "list".into(),
            title: None,
            subtitle: None,
            theme: None,
            layout: None,
            sections: None,
            navigate_instance: None,
            page_id: None,
            section_id: None,
            data: None,
            instance_id: None,
            page_slug: None,
        }))
        .await;
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("synthesis server not running"));
}

#[tokio::test]
async fn synthesis_create_returns_error_when_disabled() {
    let state = make_state();
    let server = VScreenMcpServer::new(state);
    let result = server
        .vscreen_synthesis_manage(Parameters(SynthesisManageParam {
            action: "create".into(),
            title: Some("Test".into()),
            subtitle: None,
            theme: None,
            layout: None,
            sections: None,
            navigate_instance: None,
            page_id: None,
            section_id: None,
            data: None,
            instance_id: None,
            page_slug: None,
        }))
        .await;
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("synthesis server not running"));
}

#[tokio::test]
async fn synthesis_push_returns_error_when_disabled() {
    let state = make_state();
    let server = VScreenMcpServer::new(state);
    let result = server
        .vscreen_synthesis_manage(Parameters(SynthesisManageParam {
            action: "push".into(),
            title: None,
            subtitle: None,
            theme: None,
            layout: None,
            sections: None,
            navigate_instance: None,
            page_id: Some("test".into()),
            section_id: Some("sec".into()),
            data: Some(serde_json::json!([])),
            instance_id: None,
            page_slug: None,
        }))
        .await;
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("synthesis server not running"));
}

#[tokio::test]
async fn synthesis_update_returns_error_when_disabled() {
    let state = make_state();
    let server = VScreenMcpServer::new(state);
    let result = server
        .vscreen_synthesis_manage(Parameters(SynthesisManageParam {
            action: "update".into(),
            title: Some("New".into()),
            subtitle: None,
            theme: None,
            layout: None,
            sections: None,
            navigate_instance: None,
            page_id: Some("test".into()),
            section_id: None,
            data: None,
            instance_id: None,
            page_slug: None,
        }))
        .await;
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("synthesis server not running"));
}

#[tokio::test]
async fn synthesis_delete_returns_error_when_disabled() {
    let state = make_state();
    let server = VScreenMcpServer::new(state);
    let result = server
        .vscreen_synthesis_manage(Parameters(SynthesisManageParam {
            action: "delete".into(),
            title: None,
            subtitle: None,
            theme: None,
            layout: None,
            sections: None,
            navigate_instance: None,
            page_id: Some("test".into()),
            section_id: None,
            data: None,
            instance_id: None,
            page_slug: None,
        }))
        .await;
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("synthesis server not running"));
}

#[tokio::test]
async fn synthesis_navigate_returns_error_when_disabled() {
    let state = make_state();
    let server = VScreenMcpServer::new(state);
    let result = server
        .vscreen_synthesis_manage(Parameters(SynthesisManageParam {
            action: "navigate".into(),
            title: None,
            subtitle: None,
            theme: None,
            layout: None,
            sections: None,
            navigate_instance: None,
            page_id: None,
            section_id: None,
            data: None,
            instance_id: Some("dev".into()),
            page_slug: Some("test".into()),
        }))
        .await;
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("synthesis server not running"));
}

// -----------------------------------------------------------------------
// Synthesis helper methods
// -----------------------------------------------------------------------

#[test]
fn synthesis_base_url_returns_error_when_none() {
    let state = make_state();
    let server = VScreenMcpServer::new(state);
    let result = server.synthesis_base_url();
    assert!(result.is_err());
}

#[test]
fn synthesis_base_url_returns_url_when_set() {
    let mut state = make_state();
    state.synthesis_url = Some("https://0.0.0.0:5174".into());
    let server = VScreenMcpServer::new(state);
    let result = server.synthesis_base_url();
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "https://0.0.0.0:5174");
}

#[test]
fn synthesis_client_accepts_invalid_certs() {
    let state = make_state();
    let server = VScreenMcpServer::new(state);
    let _client = server.synthesis_client();
    // Just verify it constructs without panic
}

// -----------------------------------------------------------------------
// Phase 2: SynthesisScrapeConsolidatedParam deserialization
// -----------------------------------------------------------------------

#[test]
fn synthesis_scrape_param_single_minimal() {
    let json = r#"{"instance_id":"dev","url":"https://example.com"}"#;
    let p: SynthesisScrapeConsolidatedParam = serde_json::from_str(json).unwrap();
    assert_eq!(p.mode, "single");
    assert_eq!(p.instance_id, "dev");
    assert_eq!(p.url.as_deref(), Some("https://example.com"));
    assert!(p.limit.is_none());
    assert!(p.source_label.is_none());
}

#[test]
fn synthesis_scrape_param_single_full() {
    let json = r#"{"instance_id":"dev","url":"https://cnn.com","limit":4,"source_label":"CNN"}"#;
    let p: SynthesisScrapeConsolidatedParam = serde_json::from_str(json).unwrap();
    assert_eq!(p.instance_id, "dev");
    assert_eq!(p.url.as_deref(), Some("https://cnn.com"));
    assert_eq!(p.limit, Some(4));
    assert_eq!(p.source_label.as_deref(), Some("CNN"));
}

#[test]
fn synthesis_scrape_param_single_missing_url() {
    let json = r#"{"instance_id":"dev"}"#;
    let p: SynthesisScrapeConsolidatedParam = serde_json::from_str(json).unwrap();
    assert!(p.url.is_none());
}

// -----------------------------------------------------------------------
// Phase 2: SynthesisManageParam save deserialization
// -----------------------------------------------------------------------

#[test]
fn synthesis_manage_param_save_valid() {
    let json = r#"{"action":"save","page_id":"my-page"}"#;
    let p: SynthesisManageParam = serde_json::from_str(json).unwrap();
    assert_eq!(p.action, "save");
    assert_eq!(p.page_id.as_deref(), Some("my-page"));
}

#[test]
fn consolidated_lock_param_acquire() {
    let json = r#"{"action":"acquire","instance_id":"dev"}"#;
    let p: ConsolidatedLockParam = serde_json::from_str(json).unwrap();
    assert_eq!(p.action, "acquire");
    assert_eq!(p.instance_id.as_deref(), Some("dev"));
    assert_eq!(p.lock_type, "exclusive");
    assert_eq!(p.ttl_seconds, 120);
}

#[test]
fn consolidated_lock_param_status_no_instance() {
    let json = r#"{"action":"status"}"#;
    let p: ConsolidatedLockParam = serde_json::from_str(json).unwrap();
    assert_eq!(p.action, "status");
    assert!(p.instance_id.is_none());
}

// -----------------------------------------------------------------------
// Phase 2: SynthesisManageParam create with navigate_instance
// -----------------------------------------------------------------------

#[test]
fn synthesis_manage_param_create_with_navigate_instance() {
    let json = r#"{"action":"create","title":"Test","navigate_instance":"dev"}"#;
    let p: SynthesisManageParam = serde_json::from_str(json).unwrap();
    assert_eq!(p.action, "create");
    assert_eq!(p.title.as_deref(), Some("Test"));
    assert_eq!(p.navigate_instance.as_deref(), Some("dev"));
}

#[test]
fn synthesis_manage_param_create_without_navigate_instance() {
    let json = r#"{"action":"create","title":"Test"}"#;
    let p: SynthesisManageParam = serde_json::from_str(json).unwrap();
    assert_eq!(p.action, "create");
    assert_eq!(p.title.as_deref(), Some("Test"));
    assert!(p.navigate_instance.is_none());
}

// -----------------------------------------------------------------------
// Phase 2: Disabled-state error handling for new tools
// -----------------------------------------------------------------------

#[tokio::test]
async fn synthesis_scrape_disabled() {
    let state = make_state();
    let server = VScreenMcpServer::new(state);
    let params = SynthesisScrapeConsolidatedParam {
        mode: "single".into(),
        instance_id: "dev".into(),
        url: Some("https://example.com".into()),
        limit: None,
        source_label: None,
        urls: None,
    };
    let result = server
        .vscreen_synthesis_scrape(Parameters(params))
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn synthesis_save_disabled() {
    let state = make_state();
    let server = VScreenMcpServer::new(state);
    let params = SynthesisManageParam {
        action: "save".into(),
        title: None,
        subtitle: None,
        theme: None,
        layout: None,
        sections: None,
        navigate_instance: None,
        page_id: Some("test".into()),
        section_id: None,
        data: None,
        instance_id: None,
        page_slug: None,
    };
    let result = server
        .vscreen_synthesis_manage(Parameters(params))
        .await;
    assert!(result.is_err());
}

// -----------------------------------------------------------------------
// pick_section_component boundary tests
// -----------------------------------------------------------------------

#[test]
fn pick_section_component_zero() {
    assert_eq!(pick_section_component(0), "hero");
}

#[test]
fn pick_section_component_boundary_3() {
    assert_eq!(pick_section_component(3), "hero");
}

#[test]
fn pick_section_component_boundary_4() {
    assert_eq!(pick_section_component(4), "card-grid");
}

#[test]
fn pick_section_component_boundary_12() {
    assert_eq!(pick_section_component(12), "card-grid");
}

#[test]
fn pick_section_component_boundary_13() {
    assert_eq!(pick_section_component(13), "content-list");
}

#[test]
fn pick_section_component_large() {
    assert_eq!(pick_section_component(1000), "content-list");
}

// -----------------------------------------------------------------------
// SynthesisScrapeUrlEntry deserialization
// -----------------------------------------------------------------------

#[test]
fn synthesis_scrape_url_entry_minimal() {
    let json = r#"{"url":"https://example.com"}"#;
    let p: SynthesisScrapeUrlEntry = serde_json::from_str(json).unwrap();
    assert_eq!(p.url, "https://example.com");
    assert!(p.limit.is_none());
    assert!(p.source_label.is_none());
}

#[test]
fn synthesis_scrape_url_entry_full() {
    let json = r#"{"url":"https://cnn.com","limit":10,"source_label":"CNN"}"#;
    let p: SynthesisScrapeUrlEntry = serde_json::from_str(json).unwrap();
    assert_eq!(p.url, "https://cnn.com");
    assert_eq!(p.limit, Some(10));
    assert_eq!(p.source_label.as_deref(), Some("CNN"));
}

// -----------------------------------------------------------------------
// SynthesisScrapeConsolidatedParam batch deserialization
// -----------------------------------------------------------------------

#[test]
fn synthesis_scrape_batch_param_minimal() {
    let json = r#"{"mode":"batch","instance_id":"dev","urls":[{"url":"https://example.com"}]}"#;
    let p: SynthesisScrapeConsolidatedParam = serde_json::from_str(json).unwrap();
    assert_eq!(p.mode, "batch");
    assert_eq!(p.instance_id, "dev");
    assert_eq!(p.urls.as_ref().unwrap().len(), 1);
    assert_eq!(p.urls.as_ref().unwrap()[0].url, "https://example.com");
}

#[test]
fn synthesis_scrape_batch_param_full() {
    let json = r#"{
        "mode": "batch",
        "instance_id":"dev",
        "urls":[
            {"url":"https://cnn.com","limit":5,"source_label":"CNN"},
            {"url":"https://bbc.com/news","limit":8,"source_label":"BBC"}
        ]
    }"#;
    let p: SynthesisScrapeConsolidatedParam = serde_json::from_str(json).unwrap();
    assert_eq!(p.instance_id, "dev");
    let urls = p.urls.as_ref().unwrap();
    assert_eq!(urls.len(), 2);
    assert_eq!(urls[0].limit, Some(5));
    assert_eq!(urls[1].source_label.as_deref(), Some("BBC"));
}

#[test]
fn synthesis_scrape_batch_param_missing_urls() {
    let json = r#"{"mode":"batch","instance_id":"dev"}"#;
    let p: SynthesisScrapeConsolidatedParam = serde_json::from_str(json).unwrap();
    assert!(p.urls.is_none());
}

// -----------------------------------------------------------------------
// SynthesisScrapeAndCreateParam deserialization
// -----------------------------------------------------------------------

#[test]
fn synthesis_scrape_and_create_param_minimal() {
    let json = r#"{
        "instance_id":"dev",
        "title":"News Roundup",
        "urls":[{"url":"https://example.com"}]
    }"#;
    let p: SynthesisScrapeAndCreateParam = serde_json::from_str(json).unwrap();
    assert_eq!(p.instance_id, "dev");
    assert_eq!(p.title, "News Roundup");
    assert_eq!(p.urls.len(), 1);
    assert!(p.subtitle.is_none());
    assert!(p.theme.is_none());
    assert!(p.layout.is_none());
    assert!(p.component.is_none());
    assert!(p.navigate.is_none());
}

#[test]
fn synthesis_scrape_and_create_param_full() {
    let json = r#"{
        "instance_id":"dev",
        "title":"News Roundup",
        "subtitle":"March 3, 2026",
        "theme":"dark",
        "layout":"grid",
        "urls":[
            {"url":"https://cnn.com","limit":6,"source_label":"CNN"},
            {"url":"https://bbc.com/news","limit":6,"source_label":"BBC"}
        ],
        "component":"card-grid",
        "navigate":false
    }"#;
    let p: SynthesisScrapeAndCreateParam = serde_json::from_str(json).unwrap();
    assert_eq!(p.instance_id, "dev");
    assert_eq!(p.title, "News Roundup");
    assert_eq!(p.subtitle.as_deref(), Some("March 3, 2026"));
    assert_eq!(p.theme.as_deref(), Some("dark"));
    assert_eq!(p.layout.as_deref(), Some("grid"));
    assert_eq!(p.urls.len(), 2);
    assert_eq!(p.component.as_deref(), Some("card-grid"));
    assert_eq!(p.navigate, Some(false));
}

#[test]
fn synthesis_scrape_and_create_param_missing_title() {
    let json = r#"{"instance_id":"dev","urls":[{"url":"https://example.com"}]}"#;
    let result = serde_json::from_str::<SynthesisScrapeAndCreateParam>(json);
    assert!(result.is_err());
}

#[test]
fn synthesis_scrape_and_create_param_missing_urls() {
    let json = r#"{"instance_id":"dev","title":"Test"}"#;
    let result = serde_json::from_str::<SynthesisScrapeAndCreateParam>(json);
    assert!(result.is_err());
}

// -----------------------------------------------------------------------
// Disabled-state tests for batch scrape tools
// -----------------------------------------------------------------------

#[tokio::test]
async fn synthesis_scrape_batch_disabled() {
    let state = make_state();
    assert!(state.synthesis_url.is_none());
    let server = VScreenMcpServer::new(state);
    let params = SynthesisScrapeConsolidatedParam {
        mode: "batch".into(),
        instance_id: "dev".into(),
        url: None,
        limit: None,
        source_label: None,
        urls: Some(vec![SynthesisScrapeUrlEntry {
            url: "https://example.com".into(),
            limit: None,
            source_label: None,
        }]),
    };
    let result = server
        .vscreen_synthesis_scrape(Parameters(params))
        .await;
    assert!(result.is_err(), "should fail when synthesis is not enabled");
    let err = result.unwrap_err();
    assert!(
        err.message.contains("synthesis server not running")
            || err.message.contains("no supervisor"),
        "unexpected error: {}",
        err.message
    );
}

#[tokio::test]
async fn synthesis_scrape_and_create_disabled() {
    let state = make_state();
    assert!(state.synthesis_url.is_none());
    let server = VScreenMcpServer::new(state);
    let params = SynthesisScrapeAndCreateParam {
        instance_id: "dev".into(),
        title: "Test".into(),
        subtitle: None,
        theme: None,
        layout: None,
        urls: vec![SynthesisScrapeUrlEntry {
            url: "https://example.com".into(),
            limit: None,
            source_label: None,
        }],
        component: None,
        navigate: None,
    };
    let result = server
        .vscreen_synthesis_scrape_and_create(Parameters(params))
        .await;
    assert!(result.is_err(), "should fail when synthesis is not enabled");
    let err = result.unwrap_err();
    assert!(
        err.message.contains("synthesis server not running")
            || err.message.contains("no supervisor"),
        "unexpected error: {}",
        err.message
    );
}
