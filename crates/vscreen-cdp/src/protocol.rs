use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Message ID generator
// ---------------------------------------------------------------------------

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Generate a unique monotonic message ID for CDP requests.
pub fn next_message_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Outgoing CDP messages
// ---------------------------------------------------------------------------

/// A CDP method call sent to the browser.
#[derive(Debug, Clone, Serialize)]
pub struct CdpRequest {
    pub id: u64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

impl CdpRequest {
    #[must_use]
    pub fn new(method: impl Into<String>, params: Option<serde_json::Value>) -> Self {
        Self {
            id: next_message_id(),
            method: method.into(),
            params,
        }
    }
}

// ---------------------------------------------------------------------------
// Incoming CDP messages
// ---------------------------------------------------------------------------

/// A raw CDP message received from the browser (before dispatch).
#[derive(Debug, Clone, Deserialize)]
pub struct CdpMessage {
    /// Present on method responses.
    pub id: Option<u64>,
    /// Present on events.
    pub method: Option<String>,
    /// Result payload for responses.
    pub result: Option<serde_json::Value>,
    /// Error payload for responses.
    pub error: Option<CdpErrorPayload>,
    /// Params payload for events.
    pub params: Option<serde_json::Value>,
}

impl CdpMessage {
    /// Whether this message is a response to a request.
    #[must_use]
    pub fn is_response(&self) -> bool {
        self.id.is_some()
    }

    /// Whether this message is an event.
    #[must_use]
    pub fn is_event(&self) -> bool {
        self.method.is_some() && self.id.is_none()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CdpErrorPayload {
    pub code: i64,
    pub message: String,
    pub data: Option<String>,
}

// ---------------------------------------------------------------------------
// Screencast-specific protocol types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct StartScreencastParams {
    pub format: &'static str,
    #[serde(rename = "maxWidth")]
    pub max_width: u32,
    #[serde(rename = "maxHeight")]
    pub max_height: u32,
    #[serde(rename = "everyNthFrame")]
    pub every_nth_frame: u32,
    pub quality: u32,
}

impl Default for StartScreencastParams {
    fn default() -> Self {
        Self {
            format: "jpeg",
            max_width: 1920,
            max_height: 1080,
            every_nth_frame: 1,
            quality: 80,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ScreencastFrameAckParams {
    #[serde(rename = "sessionId")]
    pub session_id: u32,
}

/// The event data for `Page.screencastFrame`.
#[derive(Debug, Clone, Deserialize)]
pub struct ScreencastFrameEvent {
    /// Base64-encoded JPEG image data.
    pub data: String,
    /// Screencast frame metadata.
    pub metadata: ScreencastFrameMetadata,
    /// Session identifier for ack.
    #[serde(rename = "sessionId")]
    pub session_id: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScreencastFrameMetadata {
    #[serde(rename = "offsetTop")]
    pub offset_top: f64,
    #[serde(rename = "pageScaleFactor")]
    pub page_scale_factor: f64,
    #[serde(rename = "deviceWidth")]
    pub device_width: f64,
    #[serde(rename = "deviceHeight")]
    pub device_height: f64,
    #[serde(rename = "scrollOffsetX")]
    pub scroll_offset_x: f64,
    #[serde(rename = "scrollOffsetY")]
    pub scroll_offset_y: f64,
    pub timestamp: Option<f64>,
}

// ---------------------------------------------------------------------------
// Input protocol types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct DispatchMouseEventParams {
    #[serde(rename = "type")]
    pub event_type: &'static str,
    pub x: f64,
    pub y: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub button: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buttons: Option<u32>,
    #[serde(rename = "clickCount", skip_serializing_if = "Option::is_none")]
    pub click_count: Option<u32>,
    #[serde(rename = "deltaX", skip_serializing_if = "Option::is_none")]
    pub delta_x: Option<f64>,
    #[serde(rename = "deltaY", skip_serializing_if = "Option::is_none")]
    pub delta_y: Option<f64>,
    pub modifiers: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct DispatchKeyEventParams {
    #[serde(rename = "type")]
    pub event_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// The text generated by the key press (required for character insertion).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub modifiers: u32,
    /// Windows virtual key code — required for non-printable keys (Backspace, arrows, etc.).
    #[serde(rename = "windowsVirtualKeyCode", skip_serializing_if = "Option::is_none")]
    pub windows_virtual_key_code: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_ids_are_unique() {
        let a = next_message_id();
        let b = next_message_id();
        assert_ne!(a, b);
        assert!(b > a);
    }

    #[test]
    fn cdp_request_serialization() {
        let req = CdpRequest::new(
            "Page.startScreencast",
            Some(serde_json::json!({ "format": "jpeg" })),
        );
        let json = serde_json::to_string(&req).expect("serialize");
        assert!(json.contains("Page.startScreencast"));
        assert!(json.contains("\"id\""));
    }

    #[test]
    fn cdp_request_without_params() {
        let req = CdpRequest::new("Page.stopScreencast", None);
        let json = serde_json::to_string(&req).expect("serialize");
        assert!(!json.contains("params"));
    }

    #[test]
    fn cdp_response_parsing() {
        let json = r#"{"id":1,"result":{"data":"abc"}}"#;
        let msg: CdpMessage = serde_json::from_str(json).expect("parse");
        assert!(msg.is_response());
        assert!(!msg.is_event());
        assert_eq!(msg.id, Some(1));
    }

    #[test]
    fn cdp_event_parsing() {
        let json = r#"{"method":"Page.screencastFrame","params":{"sessionId":1}}"#;
        let msg: CdpMessage = serde_json::from_str(json).expect("parse");
        assert!(msg.is_event());
        assert!(!msg.is_response());
        assert_eq!(msg.method.as_deref(), Some("Page.screencastFrame"));
    }

    #[test]
    fn cdp_error_parsing() {
        let json = r#"{"id":2,"error":{"code":-32000,"message":"Not found"}}"#;
        let msg: CdpMessage = serde_json::from_str(json).expect("parse");
        assert!(msg.is_response());
        let err = msg.error.expect("error");
        assert_eq!(err.code, -32000);
    }

    #[test]
    fn screencast_frame_event_parsing() {
        let json = r#"{
            "data": "base64data",
            "metadata": {
                "offsetTop": 0.0,
                "pageScaleFactor": 1.0,
                "deviceWidth": 1920.0,
                "deviceHeight": 1080.0,
                "scrollOffsetX": 0.0,
                "scrollOffsetY": 0.0
            },
            "sessionId": 42
        }"#;
        let event: ScreencastFrameEvent = serde_json::from_str(json).expect("parse");
        assert_eq!(event.session_id, 42);
        assert_eq!(event.data, "base64data");
    }

    #[test]
    fn start_screencast_params_serialization() {
        let params = StartScreencastParams::default();
        let json = serde_json::to_value(&params).expect("serialize");
        assert_eq!(json["format"], "jpeg");
        assert_eq!(json["maxWidth"], 1920);
    }

    #[test]
    fn dispatch_mouse_event_serialization() {
        let params = DispatchMouseEventParams {
            event_type: "mouseMoved",
            x: 100.0,
            y: 200.0,
            button: None,
            buttons: None,
            click_count: None,
            delta_x: None,
            delta_y: None,
            modifiers: 0,
        };
        let json = serde_json::to_string(&params).expect("serialize");
        assert!(json.contains("mouseMoved"));
        assert!(!json.contains("button")); // skipped when None
    }
}
