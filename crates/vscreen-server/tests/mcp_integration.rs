#![allow(clippy::unwrap_used, clippy::expect_used)]

use rmcp::model::{CallToolRequestParams, RawContent};
use rmcp::{ClientHandler, ServiceExt};
use vscreen_core::config::AppConfig;
use vscreen_core::instance::InstanceId;
use vscreen_server::mcp::VScreenMcpServer;
use vscreen_server::AppState;

// ---------------------------------------------------------------------------
// Dummy MCP client handler (required by rmcp to create a client peer)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct TestClient;

impl ClientHandler for TestClient {
    fn get_info(&self) -> rmcp::model::ClientInfo {
        rmcp::model::ClientInfo::default()
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct McpTestHarness {
    client: rmcp::service::RunningService<rmcp::RoleClient, TestClient>,
    state: AppState,
    server_handle: tokio::task::JoinHandle<Result<(), anyhow::Error>>,
}

impl McpTestHarness {
    async fn start() -> Self {
        let config = AppConfig::default();
        let cancel = tokio_util::sync::CancellationToken::new();
        let state = AppState::new(config, cancel);

        let (server_transport, client_transport) = tokio::io::duplex(65536);

        let server = VScreenMcpServer::new(state.clone());
        let server_handle = tokio::spawn(async move {
            let svc = server.serve(server_transport).await?;
            svc.waiting().await?;
            anyhow::Ok(())
        });

        let client = TestClient
            .serve(client_transport)
            .await
            .expect("client should connect");

        Self {
            client,
            state,
            server_handle,
        }
    }

    async fn shutdown(self) {
        let _ = self.client.cancel().await;
        let _ = self.server_handle.await;
    }

    fn call_args(name: &str, args: serde_json::Value) -> CallToolRequestParams {
        CallToolRequestParams {
            meta: None,
            name: name.to_owned().into(),
            arguments: Some(args.as_object().unwrap().clone()),
            task: None,
        }
    }

    fn call_no_args(name: &str) -> CallToolRequestParams {
        CallToolRequestParams {
            meta: None,
            name: name.to_owned().into(),
            arguments: None,
            task: None,
        }
    }
}

fn extract_text(result: &rmcp::model::CallToolResult) -> &str {
    result
        .content
        .first()
        .and_then(|c| match &c.raw {
            RawContent::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .expect("expected text content")
}

fn extract_all_text(result: &rmcp::model::CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| match &c.raw {
            RawContent::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

fn register_test_instance(state: &AppState, id: &str) {
    let config = vscreen_core::instance::InstanceConfig {
        instance_id: InstanceId::from(id),
        cdp_endpoint: "ws://localhost:9222/devtools/page/TEST".into(),
        pulse_source: "test.monitor".into(),
        display: None,
        video: vscreen_core::config::VideoConfig::default(),
        audio: vscreen_core::config::AudioConfig::default(),
        rtp_output: None,
    };
    state.registry.create(config, 16).expect("create instance");
}

// ===========================================================================
// A) Protocol-level tests
// ===========================================================================

#[tokio::test]
async fn test_server_initialization() {
    let harness = McpTestHarness::start().await;
    // If we get here, the MCP handshake (initialize + initialized) succeeded
    harness.shutdown().await;
}

#[tokio::test]
async fn test_list_tools_returns_all() {
    let harness = McpTestHarness::start().await;

    let tools = harness
        .client
        .list_all_tools()
        .await
        .expect("list_all_tools");

    let tool_names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();

    let expected = [
        "vscreen_list_instances",
        "vscreen_screenshot",
        "vscreen_screenshot_sequence",
        "vscreen_navigate",
        "vscreen_get_page_info",
        "vscreen_click",
        "vscreen_double_click",
        "vscreen_type",
        "vscreen_key_press",
        "vscreen_key_combo",
        "vscreen_scroll",
        "vscreen_drag",
        "vscreen_hover",
        "vscreen_wait",
        "vscreen_wait_for_idle",
        "vscreen_execute_js",
        "vscreen_get_cursor_position",
        // Phase 1a: Screenshot history
        "vscreen_history_list",
        "vscreen_history_get",
        "vscreen_history_get_range",
        "vscreen_history_clear",
        // Phase 1b: Session log
        "vscreen_session_log",
        "vscreen_session_summary",
        // Phase 2a: Element discovery
        "vscreen_find_elements",
        "vscreen_find_by_text",
        // Phase 3a/3b: Wait conditions
        "vscreen_wait_for_text",
        "vscreen_wait_for_selector",
        // Phase 2d: Annotated screenshot
        "vscreen_screenshot_annotated",
        // Phase 4a: Navigation
        "vscreen_go_back",
        "vscreen_go_forward",
        "vscreen_reload",
        "vscreen_extract_text",
        // Phase 1c: Console
        "vscreen_console_log",
        "vscreen_console_clear",
        // Phase 2c: Accessibility tree
        "vscreen_accessibility_tree",
        // Phase 4c: Cookie/Storage
        "vscreen_get_cookies",
        "vscreen_set_cookie",
        "vscreen_get_storage",
        "vscreen_set_storage",
        // Phase 3c/3d: Advanced wait conditions
        "vscreen_wait_for_url",
        "vscreen_wait_for_network_idle",
        // Lock management
        "vscreen_instance_lock",
        "vscreen_instance_unlock",
        "vscreen_instance_lock_status",
        "vscreen_instance_lock_renew",
        // New high-impact tools
        "vscreen_click_element",
        "vscreen_batch_click",
        "vscreen_dismiss_dialogs",
        "vscreen_fill",
        "vscreen_select_option",
        "vscreen_scroll_to_element",
        "vscreen_list_frames",
        // Navigation and input discovery
        "vscreen_find_input",
        "vscreen_click_and_navigate",
        "vscreen_dismiss_ads",
        // Self-documentation
        "vscreen_help",
        // Element description
        "vscreen_describe_elements",
        // RTSP/Audio streaming
        "vscreen_audio_streams",
        "vscreen_audio_stream_info",
        "vscreen_audio_health",
        "vscreen_rtsp_teardown",
        // CAPTCHA solving
        "vscreen_solve_captcha",
        // Task planning
        "vscreen_plan",
    ];

    for name in &expected {
        assert!(
            tool_names.iter().any(|n| n == name),
            "missing tool: {name} (found: {tool_names:?})"
        );
    }
    assert_eq!(tools.len(), expected.len(), "unexpected extra tools");

    harness.shutdown().await;
}

#[tokio::test]
async fn test_all_tools_have_descriptions() {
    let harness = McpTestHarness::start().await;

    let tools = harness
        .client
        .list_all_tools()
        .await
        .expect("list_all_tools");

    for tool in &tools {
        assert!(
            tool.description.is_some() && !tool.description.as_ref().unwrap().is_empty(),
            "tool {} has no description",
            tool.name
        );
    }

    harness.shutdown().await;
}

#[tokio::test]
async fn test_all_tools_have_input_schemas() {
    let harness = McpTestHarness::start().await;

    let tools = harness
        .client
        .list_all_tools()
        .await
        .expect("list_all_tools");

    for tool in &tools {
        let schema = &tool.input_schema;
        let schema_str = serde_json::to_string(schema).unwrap();
        assert!(
            schema_str.contains("\"type\""),
            "tool {} has no type in schema",
            tool.name
        );
    }

    harness.shutdown().await;
}

#[tokio::test]
async fn test_call_nonexistent_tool() {
    let harness = McpTestHarness::start().await;

    let result = harness
        .client
        .call_tool(McpTestHarness::call_no_args("nonexistent_tool"))
        .await;

    assert!(result.is_err(), "calling nonexistent tool should fail");

    harness.shutdown().await;
}

// ===========================================================================
// B) Tool behavior tests (no supervisor needed)
// ===========================================================================

#[tokio::test]
async fn test_list_instances_empty() {
    let harness = McpTestHarness::start().await;

    let result = harness
        .client
        .call_tool(McpTestHarness::call_no_args("vscreen_list_instances"))
        .await
        .expect("call should succeed");

    let text = extract_text(&result);
    let parsed: Vec<serde_json::Value> =
        serde_json::from_str(text).expect("should be valid JSON array");
    assert!(parsed.is_empty());

    harness.shutdown().await;
}

#[tokio::test]
async fn test_list_instances_with_entries() {
    let harness = McpTestHarness::start().await;

    register_test_instance(&harness.state, "alpha");
    register_test_instance(&harness.state, "beta");

    let result = harness
        .client
        .call_tool(McpTestHarness::call_no_args("vscreen_list_instances"))
        .await
        .expect("call should succeed");

    let text = extract_text(&result);
    let parsed: Vec<serde_json::Value> =
        serde_json::from_str(text).expect("should be valid JSON array");
    assert_eq!(parsed.len(), 2);

    let ids: Vec<&str> = parsed
        .iter()
        .filter_map(|v| v.get("instance_id").and_then(|i| i.as_str()))
        .collect();
    assert!(ids.contains(&"alpha"));
    assert!(ids.contains(&"beta"));

    harness.shutdown().await;
}

#[tokio::test]
async fn test_list_instances_returns_parseable_json() {
    let harness = McpTestHarness::start().await;

    register_test_instance(&harness.state, "json-test");

    let result = harness
        .client
        .call_tool(McpTestHarness::call_no_args("vscreen_list_instances"))
        .await
        .expect("call should succeed");

    let text = extract_text(&result);
    let parsed: Vec<serde_json::Value> = serde_json::from_str(text).unwrap();
    let entry = &parsed[0];
    assert!(entry.get("instance_id").is_some());
    assert!(entry.get("state").is_some());
    assert!(entry.get("supervisor_running").is_some());

    harness.shutdown().await;
}

#[tokio::test]
async fn test_wait_tool() {
    let harness = McpTestHarness::start().await;

    let start = std::time::Instant::now();
    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_wait",
            serde_json::json!({"duration_ms": 50}),
        ))
        .await
        .expect("wait should succeed");

    let elapsed = start.elapsed();
    assert!(elapsed.as_millis() >= 40, "wait should take at least ~50ms");

    let text = extract_text(&result);
    assert!(text.contains("50ms"));

    harness.shutdown().await;
}

#[tokio::test]
async fn test_wait_tool_short_duration() {
    let harness = McpTestHarness::start().await;

    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_wait",
            serde_json::json!({"duration_ms": 1}),
        ))
        .await
        .expect("wait should succeed");

    let text = extract_text(&result);
    assert!(text.contains("1ms"));

    harness.shutdown().await;
}

// ===========================================================================
// C) Error path tests (tools that need a supervisor)
// ===========================================================================

macro_rules! test_no_supervisor {
    ($name:ident, $tool:expr, $args:expr) => {
        #[tokio::test]
        async fn $name() {
            let harness = McpTestHarness::start().await;
            register_test_instance(&harness.state, "no-sup");

            let result = harness
                .client
                .call_tool(McpTestHarness::call_args($tool, $args))
                .await;

            assert!(
                result.is_err(),
                "{} without supervisor should fail",
                $tool
            );

            harness.shutdown().await;
        }
    };
}

test_no_supervisor!(
    test_screenshot_no_supervisor,
    "vscreen_screenshot",
    serde_json::json!({"instance_id": "no-sup"})
);

test_no_supervisor!(
    test_screenshot_sequence_no_supervisor,
    "vscreen_screenshot_sequence",
    serde_json::json!({"instance_id": "no-sup", "count": 2, "interval_ms": 100})
);

test_no_supervisor!(
    test_click_no_supervisor,
    "vscreen_click",
    serde_json::json!({"instance_id": "no-sup", "x": 100, "y": 200})
);

test_no_supervisor!(
    test_double_click_no_supervisor,
    "vscreen_double_click",
    serde_json::json!({"instance_id": "no-sup", "x": 50, "y": 50})
);

test_no_supervisor!(
    test_type_no_supervisor,
    "vscreen_type",
    serde_json::json!({"instance_id": "no-sup", "text": "hello"})
);

test_no_supervisor!(
    test_navigate_no_supervisor,
    "vscreen_navigate",
    serde_json::json!({"instance_id": "no-sup", "url": "https://example.com"})
);

test_no_supervisor!(
    test_execute_js_no_supervisor,
    "vscreen_execute_js",
    serde_json::json!({"instance_id": "no-sup", "expression": "1+1"})
);

test_no_supervisor!(
    test_get_page_info_no_supervisor,
    "vscreen_get_page_info",
    serde_json::json!({"instance_id": "no-sup"})
);

test_no_supervisor!(
    test_get_cursor_position_no_supervisor,
    "vscreen_get_cursor_position",
    serde_json::json!({"instance_id": "no-sup"})
);

test_no_supervisor!(
    test_key_press_no_supervisor,
    "vscreen_key_press",
    serde_json::json!({"instance_id": "no-sup", "key": "Enter"})
);

test_no_supervisor!(
    test_key_combo_no_supervisor,
    "vscreen_key_combo",
    serde_json::json!({"instance_id": "no-sup", "keys": ["Control", "a"]})
);

test_no_supervisor!(
    test_scroll_no_supervisor,
    "vscreen_scroll",
    serde_json::json!({"instance_id": "no-sup", "x": 0, "y": 0, "delta_y": -120})
);

test_no_supervisor!(
    test_drag_no_supervisor,
    "vscreen_drag",
    serde_json::json!({"instance_id": "no-sup", "from_x": 0, "from_y": 0, "to_x": 100, "to_y": 100})
);

test_no_supervisor!(
    test_hover_no_supervisor,
    "vscreen_hover",
    serde_json::json!({"instance_id": "no-sup", "x": 42, "y": 84})
);

test_no_supervisor!(
    test_wait_for_idle_no_supervisor,
    "vscreen_wait_for_idle",
    serde_json::json!({"instance_id": "no-sup", "timeout_ms": 100})
);

// ===========================================================================
// D) Parameter validation tests
// ===========================================================================

macro_rules! test_invalid_params {
    ($name:ident, $tool:expr, $args:expr, $msg:expr) => {
        #[tokio::test]
        async fn $name() {
            let harness = McpTestHarness::start().await;

            let result = harness
                .client
                .call_tool(McpTestHarness::call_args($tool, $args))
                .await;

            assert!(result.is_err(), $msg);

            harness.shutdown().await;
        }
    };
}

test_invalid_params!(
    test_click_missing_x,
    "vscreen_click",
    serde_json::json!({"instance_id": "dev", "y": 100}),
    "click without x should fail"
);

test_invalid_params!(
    test_click_missing_y,
    "vscreen_click",
    serde_json::json!({"instance_id": "dev", "x": 100}),
    "click without y should fail"
);

test_invalid_params!(
    test_click_missing_instance_id,
    "vscreen_click",
    serde_json::json!({"x": 100, "y": 200}),
    "click without instance_id should fail"
);

test_invalid_params!(
    test_navigate_missing_url,
    "vscreen_navigate",
    serde_json::json!({"instance_id": "dev"}),
    "navigate without url should fail"
);

test_invalid_params!(
    test_type_missing_text,
    "vscreen_type",
    serde_json::json!({"instance_id": "dev"}),
    "type without text should fail"
);

test_invalid_params!(
    test_execute_js_missing_expression,
    "vscreen_execute_js",
    serde_json::json!({"instance_id": "dev"}),
    "execute_js without expression should fail"
);

test_invalid_params!(
    test_screenshot_sequence_missing_count,
    "vscreen_screenshot_sequence",
    serde_json::json!({"instance_id": "dev", "interval_ms": 100}),
    "screenshot_sequence without count should fail"
);

test_invalid_params!(
    test_drag_missing_to_coordinates,
    "vscreen_drag",
    serde_json::json!({"instance_id": "dev", "from_x": 0, "from_y": 0}),
    "drag without to_x/to_y should fail"
);

test_invalid_params!(
    test_wait_missing_duration,
    "vscreen_wait",
    serde_json::json!({}),
    "wait without duration_ms should fail"
);

#[tokio::test]
async fn test_key_combo_empty_keys() {
    let harness = McpTestHarness::start().await;
    register_test_instance(&harness.state, "combo-test");

    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_key_combo",
            serde_json::json!({"instance_id": "combo-test", "keys": []}),
        ))
        .await;

    assert!(result.is_err(), "key_combo with empty keys should fail");

    harness.shutdown().await;
}

#[tokio::test]
async fn test_screenshot_invalid_instance() {
    let harness = McpTestHarness::start().await;

    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_screenshot",
            serde_json::json!({"instance_id": "ghost"}),
        ))
        .await;

    assert!(
        result.is_err(),
        "screenshot for nonexistent instance should fail"
    );

    harness.shutdown().await;
}

// ===========================================================================
// E) Content format / edge case tests
// ===========================================================================

#[tokio::test]
async fn test_list_instances_supervisor_running_field() {
    let harness = McpTestHarness::start().await;
    register_test_instance(&harness.state, "check-sup");

    let result = harness
        .client
        .call_tool(McpTestHarness::call_no_args("vscreen_list_instances"))
        .await
        .expect("call should succeed");

    let text = extract_text(&result);
    let parsed: Vec<serde_json::Value> = serde_json::from_str(text).unwrap();
    let entry = &parsed[0];
    assert_eq!(
        entry.get("supervisor_running").and_then(|v| v.as_bool()),
        Some(false)
    );

    harness.shutdown().await;
}

#[tokio::test]
async fn test_multiple_tool_calls_on_same_client() {
    let harness = McpTestHarness::start().await;
    register_test_instance(&harness.state, "multi");

    let r1 = harness
        .client
        .call_tool(McpTestHarness::call_no_args("vscreen_list_instances"))
        .await
        .expect("list should work");
    assert!(extract_text(&r1).contains("multi"));

    let r2 = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_wait",
            serde_json::json!({"duration_ms": 5}),
        ))
        .await
        .expect("wait should work");
    assert!(extract_text(&r2).contains("5ms"));

    let r3 = harness
        .client
        .call_tool(McpTestHarness::call_no_args("vscreen_list_instances"))
        .await
        .expect("second list should work");
    assert!(extract_text(&r3).contains("multi"));

    harness.shutdown().await;
}

#[tokio::test]
async fn test_list_instances_after_registry_mutation() {
    let harness = McpTestHarness::start().await;

    // Start empty
    let r1 = harness
        .client
        .call_tool(McpTestHarness::call_no_args("vscreen_list_instances"))
        .await
        .expect("list should work");
    let p1: Vec<serde_json::Value> = serde_json::from_str(extract_text(&r1)).unwrap();
    assert!(p1.is_empty());

    // Add an instance
    register_test_instance(&harness.state, "dynamic");

    let r2 = harness
        .client
        .call_tool(McpTestHarness::call_no_args("vscreen_list_instances"))
        .await
        .expect("list should work");
    let p2: Vec<serde_json::Value> = serde_json::from_str(extract_text(&r2)).unwrap();
    assert_eq!(p2.len(), 1);
    assert!(extract_text(&r2).contains("dynamic"));

    // Remove it
    harness
        .state
        .registry
        .remove(&InstanceId::from("dynamic"))
        .expect("remove");

    let r3 = harness
        .client
        .call_tool(McpTestHarness::call_no_args("vscreen_list_instances"))
        .await
        .expect("list should work");
    let p3: Vec<serde_json::Value> = serde_json::from_str(extract_text(&r3)).unwrap();
    assert!(p3.is_empty());

    harness.shutdown().await;
}

// ===========================================================================
// F) New advanced tool tests (no supervisor — error path validation)
// ===========================================================================

#[tokio::test]
async fn test_history_list_no_supervisor() {
    let harness = McpTestHarness::start().await;
    register_test_instance(&harness.state, "hist");
    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_history_list",
            serde_json::json!({"instance_id": "hist"}),
        ))
        .await;
    assert!(result.is_err() || result.unwrap().is_error.unwrap_or(false));
    harness.shutdown().await;
}

#[tokio::test]
async fn test_history_get_no_supervisor() {
    let harness = McpTestHarness::start().await;
    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_history_get",
            serde_json::json!({"instance_id": "nope", "index": 0}),
        ))
        .await;
    assert!(result.is_err());
    harness.shutdown().await;
}

#[tokio::test]
async fn test_session_log_no_supervisor() {
    let harness = McpTestHarness::start().await;
    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_session_log",
            serde_json::json!({"instance_id": "nope"}),
        ))
        .await;
    assert!(result.is_err());
    harness.shutdown().await;
}

#[tokio::test]
async fn test_session_summary_no_supervisor() {
    let harness = McpTestHarness::start().await;
    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_session_summary",
            serde_json::json!({"instance_id": "nope"}),
        ))
        .await;
    assert!(result.is_err());
    harness.shutdown().await;
}

#[tokio::test]
async fn test_find_elements_no_supervisor() {
    let harness = McpTestHarness::start().await;
    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_find_elements",
            serde_json::json!({"instance_id": "nope", "selector": "button"}),
        ))
        .await;
    assert!(result.is_err());
    harness.shutdown().await;
}

#[tokio::test]
async fn test_find_by_text_no_supervisor() {
    let harness = McpTestHarness::start().await;
    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_find_by_text",
            serde_json::json!({"instance_id": "nope", "text": "Submit"}),
        ))
        .await;
    assert!(result.is_err());
    harness.shutdown().await;
}

#[tokio::test]
async fn test_go_back_no_supervisor() {
    let harness = McpTestHarness::start().await;
    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_go_back",
            serde_json::json!({"instance_id": "nope"}),
        ))
        .await;
    assert!(result.is_err());
    harness.shutdown().await;
}

#[tokio::test]
async fn test_go_forward_no_supervisor() {
    let harness = McpTestHarness::start().await;
    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_go_forward",
            serde_json::json!({"instance_id": "nope"}),
        ))
        .await;
    assert!(result.is_err());
    harness.shutdown().await;
}

#[tokio::test]
async fn test_reload_no_supervisor() {
    let harness = McpTestHarness::start().await;
    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_reload",
            serde_json::json!({"instance_id": "nope"}),
        ))
        .await;
    assert!(result.is_err());
    harness.shutdown().await;
}

#[tokio::test]
async fn test_extract_text_no_supervisor() {
    let harness = McpTestHarness::start().await;
    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_extract_text",
            serde_json::json!({"instance_id": "nope"}),
        ))
        .await;
    assert!(result.is_err());
    harness.shutdown().await;
}

#[tokio::test]
async fn test_console_log_no_supervisor() {
    let harness = McpTestHarness::start().await;
    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_console_log",
            serde_json::json!({"instance_id": "nope"}),
        ))
        .await;
    assert!(result.is_err());
    harness.shutdown().await;
}

#[tokio::test]
async fn test_console_clear_no_supervisor() {
    let harness = McpTestHarness::start().await;
    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_console_clear",
            serde_json::json!({"instance_id": "nope"}),
        ))
        .await;
    assert!(result.is_err());
    harness.shutdown().await;
}

#[tokio::test]
async fn test_get_cookies_no_supervisor() {
    let harness = McpTestHarness::start().await;
    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_get_cookies",
            serde_json::json!({"instance_id": "nope"}),
        ))
        .await;
    assert!(result.is_err());
    harness.shutdown().await;
}

#[tokio::test]
async fn test_set_cookie_no_supervisor() {
    let harness = McpTestHarness::start().await;
    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_set_cookie",
            serde_json::json!({"instance_id": "nope", "name": "k", "value": "v"}),
        ))
        .await;
    assert!(result.is_err());
    harness.shutdown().await;
}

#[tokio::test]
async fn test_get_storage_no_supervisor() {
    let harness = McpTestHarness::start().await;
    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_get_storage",
            serde_json::json!({"instance_id": "nope", "key": "k"}),
        ))
        .await;
    assert!(result.is_err());
    harness.shutdown().await;
}

#[tokio::test]
async fn test_set_storage_no_supervisor() {
    let harness = McpTestHarness::start().await;
    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_set_storage",
            serde_json::json!({"instance_id": "nope", "key": "k", "value": "v"}),
        ))
        .await;
    assert!(result.is_err());
    harness.shutdown().await;
}

#[tokio::test]
async fn test_accessibility_tree_no_supervisor() {
    let harness = McpTestHarness::start().await;
    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_accessibility_tree",
            serde_json::json!({"instance_id": "nope"}),
        ))
        .await;
    assert!(result.is_err());
    harness.shutdown().await;
}

#[tokio::test]
async fn test_annotated_screenshot_no_supervisor() {
    let harness = McpTestHarness::start().await;
    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_screenshot_annotated",
            serde_json::json!({"instance_id": "nope"}),
        ))
        .await;
    assert!(result.is_err());
    harness.shutdown().await;
}

#[tokio::test]
async fn test_wait_for_url_no_supervisor() {
    let harness = McpTestHarness::start().await;
    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_wait_for_url",
            serde_json::json!({"instance_id": "nope", "url_contains": "test"}),
        ))
        .await;
    assert!(result.is_err());
    harness.shutdown().await;
}

#[tokio::test]
async fn test_wait_for_network_idle_no_supervisor() {
    let harness = McpTestHarness::start().await;
    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_wait_for_network_idle",
            serde_json::json!({"instance_id": "nope"}),
        ))
        .await;
    assert!(result.is_err());
    harness.shutdown().await;
}

// Missing required params validation
#[tokio::test]
async fn test_find_elements_missing_selector() {
    let harness = McpTestHarness::start().await;
    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_find_elements",
            serde_json::json!({"instance_id": "dev"}),
        ))
        .await;
    assert!(result.is_err());
    harness.shutdown().await;
}

#[tokio::test]
async fn test_wait_for_text_missing_text() {
    let harness = McpTestHarness::start().await;
    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_wait_for_text",
            serde_json::json!({"instance_id": "dev"}),
        ))
        .await;
    assert!(result.is_err());
    harness.shutdown().await;
}

#[tokio::test]
async fn test_history_get_missing_index() {
    let harness = McpTestHarness::start().await;
    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_history_get",
            serde_json::json!({"instance_id": "dev"}),
        ))
        .await;
    assert!(result.is_err());
    harness.shutdown().await;
}

// ===========================================================================
// F) Lock management integration tests
// ===========================================================================

#[tokio::test]
async fn test_lock_acquire_and_release() {
    let harness = McpTestHarness::start().await;
    register_test_instance(&harness.state, "lock-test");

    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_instance_lock",
            serde_json::json!({"instance_id": "lock-test"}),
        ))
        .await
        .expect("lock should succeed");
    let text = extract_text(&result);
    assert!(text.contains("Lock acquired"), "got: {text}");

    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_instance_unlock",
            serde_json::json!({"instance_id": "lock-test"}),
        ))
        .await
        .expect("unlock should succeed");
    let text = extract_text(&result);
    assert!(text.contains("Lock released"), "got: {text}");

    harness.shutdown().await;
}

#[tokio::test]
async fn test_lock_status_shows_holder() {
    let harness = McpTestHarness::start().await;
    register_test_instance(&harness.state, "status-test");

    // Lock it
    harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_instance_lock",
            serde_json::json!({"instance_id": "status-test", "agent_name": "test-agent"}),
        ))
        .await
        .expect("lock");

    // Check status
    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_instance_lock_status",
            serde_json::json!({"instance_id": "status-test"}),
        ))
        .await
        .expect("status");
    let text = extract_text(&result);
    assert!(text.contains("test-agent"), "status should show agent name: {text}");
    assert!(text.contains("you_hold_exclusive"), "status should show caller holds lock: {text}");

    harness.shutdown().await;
}

#[tokio::test]
async fn test_lock_renew_extends_ttl() {
    let harness = McpTestHarness::start().await;
    register_test_instance(&harness.state, "renew-test");

    harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_instance_lock",
            serde_json::json!({"instance_id": "renew-test", "ttl_seconds": 30}),
        ))
        .await
        .expect("lock");

    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_instance_lock_renew",
            serde_json::json!({"instance_id": "renew-test", "ttl_seconds": 300}),
        ))
        .await
        .expect("renew");
    let text = extract_text(&result);
    assert!(text.contains("Lock renewed"), "got: {text}");

    harness.shutdown().await;
}

#[tokio::test]
async fn test_unlock_without_lock_fails() {
    let harness = McpTestHarness::start().await;
    register_test_instance(&harness.state, "no-lock");

    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_instance_unlock",
            serde_json::json!({"instance_id": "no-lock"}),
        ))
        .await;
    assert!(result.is_err(), "unlock without lock should fail");

    harness.shutdown().await;
}

#[tokio::test]
async fn test_renew_without_lock_fails() {
    let harness = McpTestHarness::start().await;

    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_instance_lock_renew",
            serde_json::json!({"instance_id": "no-lock"}),
        ))
        .await;
    assert!(result.is_err(), "renew without lock should fail");

    harness.shutdown().await;
}

#[tokio::test]
async fn test_lock_status_all_instances() {
    let harness = McpTestHarness::start().await;

    let result = harness
        .client
        .call_tool(McpTestHarness::call_no_args(
            "vscreen_instance_lock_status",
        ))
        .await
        .expect("status all should work");
    let text = extract_text(&result);
    assert!(text.starts_with('['), "should return JSON array: {text}");

    harness.shutdown().await;
}

#[tokio::test]
async fn test_lock_observer_type() {
    let harness = McpTestHarness::start().await;
    register_test_instance(&harness.state, "obs-test");

    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_instance_lock",
            serde_json::json!({"instance_id": "obs-test", "lock_type": "observer"}),
        ))
        .await
        .expect("observer lock");
    let text = extract_text(&result);
    assert!(text.contains("Lock acquired"), "got: {text}");
    assert!(text.contains("observer"), "should be observer lock: {text}");

    harness.shutdown().await;
}

#[tokio::test]
async fn test_lock_invalid_type_rejected() {
    let harness = McpTestHarness::start().await;
    register_test_instance(&harness.state, "bad-type");

    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_instance_lock",
            serde_json::json!({"instance_id": "bad-type", "lock_type": "invalid"}),
        ))
        .await;
    assert!(result.is_err(), "invalid lock type should fail");

    harness.shutdown().await;
}

#[tokio::test]
async fn test_tool_requires_lock() {
    let harness = McpTestHarness::start().await;
    register_test_instance(&harness.state, "guard-test");

    // Try to click without a lock -- should fail
    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_click",
            serde_json::json!({"instance_id": "guard-test", "x": 100, "y": 200}),
        ))
        .await;
    assert!(result.is_err(), "click without lock should fail");

    // Acquire lock
    harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_instance_lock",
            serde_json::json!({"instance_id": "guard-test"}),
        ))
        .await
        .expect("lock");

    // Click should still fail (no supervisor) but the error should be about missing supervisor, not missing lock
    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_click",
            serde_json::json!({"instance_id": "guard-test", "x": 100, "y": 200}),
        ))
        .await;
    assert!(result.is_err(), "click without supervisor should still fail");

    harness.shutdown().await;
}

#[tokio::test]
async fn test_list_instances_shows_lock_info() {
    let harness = McpTestHarness::start().await;
    register_test_instance(&harness.state, "list-lock");

    harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_instance_lock",
            serde_json::json!({"instance_id": "list-lock", "agent_name": "my-agent"}),
        ))
        .await
        .expect("lock");

    let result = harness
        .client
        .call_tool(McpTestHarness::call_no_args("vscreen_list_instances"))
        .await
        .expect("list");
    let text = extract_text(&result);
    assert!(text.contains("my-agent"), "list should show lock agent: {text}");
    assert!(text.contains("you_hold_lock"), "list should show caller info: {text}");

    harness.shutdown().await;
}

// ===========================================================================
// I) Tool Advisor integration tests
// ===========================================================================

#[tokio::test]
async fn test_advisor_hint_on_repeated_waits() {
    let harness = McpTestHarness::start().await;

    // First wait -- no hint
    let r1 = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_wait",
            serde_json::json!({"duration_ms": 100}),
        ))
        .await
        .expect("wait 1");
    let all1 = extract_all_text(&r1);
    assert!(!all1.contains("Advisor"), "first wait should not have advisor hint");

    // Second wait -- still building up
    harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_wait",
            serde_json::json!({"duration_ms": 100}),
        ))
        .await
        .expect("wait 2");

    // Third wait -- should trigger the hint (2+ waits in recent calls)
    let r3 = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_wait",
            serde_json::json!({"duration_ms": 100}),
        ))
        .await
        .expect("wait 3");
    let all3 = extract_all_text(&r3);
    assert!(
        all3.contains("Advisor"),
        "repeated waits should trigger advisor hint: {all3}"
    );
    assert!(
        all3.contains("wait_for_text") || all3.contains("wait_for_selector"),
        "should recommend targeted waits: {all3}"
    );

    harness.shutdown().await;
}

#[tokio::test]
async fn test_plan_tool_via_protocol() {
    let harness = McpTestHarness::start().await;

    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_plan",
            serde_json::json!({"task": "navigate to a URL and read the page"}),
        ))
        .await
        .expect("plan");
    let text = extract_text(&result);
    assert!(
        text.contains("vscreen_navigate"),
        "should recommend navigate: {text}"
    );

    harness.shutdown().await;
}

#[tokio::test]
async fn test_help_tool_selection_via_protocol() {
    let harness = McpTestHarness::start().await;

    let result = harness
        .client
        .call_tool(McpTestHarness::call_args(
            "vscreen_help",
            serde_json::json!({"topic": "tool-selection"}),
        ))
        .await
        .expect("help");
    let text = extract_text(&result);
    assert!(
        text.contains("Tool Selection Guide"),
        "should return guide: {text}"
    );

    harness.shutdown().await;
}

#[tokio::test]
async fn test_list_tools_includes_plan() {
    let harness = McpTestHarness::start().await;

    let tools = harness
        .client
        .list_all_tools()
        .await
        .expect("list tools");
    let names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
    assert!(
        names.contains(&"vscreen_plan".to_string()),
        "tool list should include vscreen_plan"
    );

    harness.shutdown().await;
}
