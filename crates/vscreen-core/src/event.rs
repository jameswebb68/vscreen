use serde::{Deserialize, Serialize};

use crate::instance::PeerId;

/// Input event from a browser client via DataChannel.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum InputEvent {
    /// Mouse move (b = pressed-buttons bitmask: 1=left, 2=right, 4=middle)
    #[serde(rename = "mm")]
    MouseMove { x: f64, y: f64, #[serde(default)] b: u16, m: u8 },

    /// Mouse button down
    #[serde(rename = "md")]
    MouseDown { x: f64, y: f64, b: u8, m: u8 },

    /// Mouse button up
    #[serde(rename = "mu")]
    MouseUp { x: f64, y: f64, b: u8, m: u8 },

    /// Mouse wheel
    #[serde(rename = "wh")]
    Wheel { x: f64, y: f64, dx: f64, dy: f64, m: u8 },

    /// Key down
    #[serde(rename = "kd")]
    KeyDown { key: String, code: String, m: u8 },

    /// Key up
    #[serde(rename = "ku")]
    KeyUp { key: String, code: String, m: u8 },

    /// Clipboard paste
    #[serde(rename = "paste")]
    Paste { text: String },

    /// Adaptive bitrate hint from client
    #[serde(rename = "br")]
    BitrateHint { kbps: u32 },
}

/// Modifier bitmask flags.
pub mod modifiers {
    pub const ALT: u8 = 1;
    pub const CTRL: u8 = 2;
    pub const META: u8 = 4;
    pub const SHIFT: u8 = 8;
}

impl InputEvent {
    /// Extract the modifier bitmask from any event.
    #[must_use]
    pub fn modifiers(&self) -> u8 {
        match self {
            Self::MouseMove { m, .. }
            | Self::MouseDown { m, .. }
            | Self::MouseUp { m, .. }
            | Self::Wheel { m, .. }
            | Self::KeyDown { m, .. }
            | Self::KeyUp { m, .. } => *m,
            Self::Paste { .. } | Self::BitrateHint { .. } => 0,
        }
    }

    /// Whether the event is a keyboard event.
    #[must_use]
    pub fn is_keyboard(&self) -> bool {
        matches!(self, Self::KeyDown { .. } | Self::KeyUp { .. })
    }

    /// Whether the event is a mouse event.
    #[must_use]
    pub fn is_mouse(&self) -> bool {
        matches!(
            self,
            Self::MouseMove { .. } | Self::MouseDown { .. } | Self::MouseUp { .. } | Self::Wheel { .. }
        )
    }
}

/// An input event tagged with the peer that sent it.
#[derive(Debug, Clone)]
pub struct PeerInputEvent {
    pub peer_id: PeerId,
    pub event: InputEvent,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_move_serialization() {
        let event = InputEvent::MouseMove {
            x: 100.0,
            y: 200.0,
            b: 0,
            m: 0,
        };
        let json = serde_json::to_string(&event).expect("serialize");
        assert!(json.contains("\"t\":\"mm\""));
        assert!(json.contains("\"x\":100.0"));
    }

    #[test]
    fn key_down_deserialization() {
        let json = r#"{"t":"kd","key":"a","code":"KeyA","m":2}"#;
        let event: InputEvent = serde_json::from_str(json).expect("deserialize");
        assert!(event.is_keyboard());
        assert!(!event.is_mouse());
        assert_eq!(event.modifiers(), modifiers::CTRL);
    }

    #[test]
    fn mouse_down_deserialization() {
        let json = r#"{"t":"md","x":50,"y":75,"b":0,"m":0}"#;
        let event: InputEvent = serde_json::from_str(json).expect("deserialize");
        assert!(event.is_mouse());
        assert!(!event.is_keyboard());
    }

    #[test]
    fn wheel_event() {
        let json = r#"{"t":"wh","x":0,"y":0,"dx":0,"dy":-120,"m":0}"#;
        let event: InputEvent = serde_json::from_str(json).expect("deserialize");
        assert!(event.is_mouse());
    }

    #[test]
    fn modifier_extraction() {
        let event = InputEvent::KeyDown {
            key: "a".into(),
            code: "KeyA".into(),
            m: modifiers::CTRL | modifiers::SHIFT,
        };
        let m = event.modifiers();
        assert_eq!(m & modifiers::CTRL, modifiers::CTRL);
        assert_eq!(m & modifiers::SHIFT, modifiers::SHIFT);
        assert_eq!(m & modifiers::ALT, 0);
    }
}
