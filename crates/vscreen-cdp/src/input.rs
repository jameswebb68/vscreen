use vscreen_core::event::InputEvent;

use crate::protocol::{CdpRequest, DispatchKeyEventParams, DispatchMouseEventParams};

/// Convert a modifier bitmask from the DataChannel compact format to CDP modifier flags.
///
/// DataChannel: ALT=1 CTRL=2 META=4 SHIFT=8
/// CDP:         Alt=1 Ctrl=2 Meta=4 Shift=8
/// (Same mapping, so we just cast.)
const fn to_cdp_modifiers(m: u8) -> u32 {
    m as u32
}

/// Map DataChannel button index (0=left, 1=middle, 2=right) to CDP button name.
fn button_name(b: u8) -> &'static str {
    match b {
        0 => "left",
        1 => "middle",
        2 => "right",
        3 => "back",
        4 => "forward",
        _ => "none",
    }
}

/// Map a DOM `KeyboardEvent.key` value to its Windows virtual key code.
/// CDP requires this for non-printable keys to actually take effect.
fn key_to_vk(key: &str) -> Option<u32> {
    match key {
        "Backspace" => Some(8),
        "Tab" => Some(9),
        "Enter" => Some(13),
        "Shift" => Some(16),
        "Control" => Some(17),
        "Alt" => Some(18),
        "Pause" => Some(19),
        "CapsLock" => Some(20),
        "Escape" => Some(27),
        " " => Some(32),
        "PageUp" => Some(33),
        "PageDown" => Some(34),
        "End" => Some(35),
        "Home" => Some(36),
        "ArrowLeft" => Some(37),
        "ArrowUp" => Some(38),
        "ArrowRight" => Some(39),
        "ArrowDown" => Some(40),
        "Insert" => Some(45),
        "Delete" => Some(46),
        "Meta" => Some(91),
        "ContextMenu" => Some(93),
        "F1" => Some(112),
        "F2" => Some(113),
        "F3" => Some(114),
        "F4" => Some(115),
        "F5" => Some(116),
        "F6" => Some(117),
        "F7" => Some(118),
        "F8" => Some(119),
        "F9" => Some(120),
        "F10" => Some(121),
        "F11" => Some(122),
        "F12" => Some(123),
        "NumLock" => Some(144),
        "ScrollLock" => Some(145),
        // Single printable characters: derive VK from uppercase ASCII
        s if s.len() == 1 => {
            let ch = s.chars().next().unwrap();
            match ch {
                'a'..='z' => Some(ch.to_ascii_uppercase() as u32),
                'A'..='Z' | '0'..='9' => Some(ch as u32),
                ';' | ':' => Some(186),
                '=' | '+' => Some(187),
                ',' | '<' => Some(188),
                '-' | '_' => Some(189),
                '.' | '>' => Some(190),
                '/' | '?' => Some(191),
                '`' | '~' => Some(192),
                '[' | '{' => Some(219),
                '\\' | '|' => Some(220),
                ']' | '}' => Some(221),
                '\'' | '"' => Some(222),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Convert an `InputEvent` into one or more `CdpRequest`s for `Input.dispatch*` methods.
///
/// Most events produce a single request. `Paste` produces a single `Input.insertText`.
#[must_use]
pub fn input_to_cdp(event: &InputEvent) -> CdpRequest {
    match event {
        InputEvent::MouseMove { x, y, b, m } => {
            let buttons = if *b > 0 { Some(*b as u32) } else { None };
            let button = if *b & 1 != 0 { Some("left") } else if *b & 2 != 0 { Some("right") } else if *b & 4 != 0 { Some("middle") } else { None };
            let params = DispatchMouseEventParams {
                event_type: "mouseMoved",
                x: *x,
                y: *y,
                button,
                buttons,
                click_count: None,
                delta_x: None,
                delta_y: None,
                modifiers: to_cdp_modifiers(*m),
            };
            CdpRequest::new(
                "Input.dispatchMouseEvent",
                Some(serde_json::to_value(&params).expect("infallible serialization")),
            )
        }
        InputEvent::MouseDown { x, y, b, m } => {
            let params = DispatchMouseEventParams {
                event_type: "mousePressed",
                x: *x,
                y: *y,
                button: Some(button_name(*b)),
                buttons: Some(1 << *b),
                click_count: Some(1),
                delta_x: None,
                delta_y: None,
                modifiers: to_cdp_modifiers(*m),
            };
            CdpRequest::new(
                "Input.dispatchMouseEvent",
                Some(serde_json::to_value(&params).expect("infallible serialization")),
            )
        }
        InputEvent::MouseUp { x, y, b, m } => {
            let params = DispatchMouseEventParams {
                event_type: "mouseReleased",
                x: *x,
                y: *y,
                button: Some(button_name(*b)),
                buttons: Some(0),
                click_count: Some(1),
                delta_x: None,
                delta_y: None,
                modifiers: to_cdp_modifiers(*m),
            };
            CdpRequest::new(
                "Input.dispatchMouseEvent",
                Some(serde_json::to_value(&params).expect("infallible serialization")),
            )
        }
        InputEvent::Wheel { x, y, dx, dy, m } => {
            let params = DispatchMouseEventParams {
                event_type: "mouseWheel",
                x: *x,
                y: *y,
                button: None,
                buttons: None,
                click_count: None,
                delta_x: Some(*dx),
                delta_y: Some(*dy),
                modifiers: to_cdp_modifiers(*m),
            };
            CdpRequest::new(
                "Input.dispatchMouseEvent",
                Some(serde_json::to_value(&params).expect("infallible serialization")),
            )
        }
        InputEvent::KeyDown { key, code, m } => {
            let vk = key_to_vk(key);
            let is_printable = key.len() == 1;
            let params = DispatchKeyEventParams {
                // rawKeyDown for non-printable; keyDown for printable (generates text)
                event_type: if is_printable { "keyDown" } else { "rawKeyDown" },
                key: Some(key.clone()),
                code: Some(code.clone()),
                text: if is_printable { Some(key.clone()) } else { None },
                modifiers: to_cdp_modifiers(*m),
                windows_virtual_key_code: vk,
            };
            CdpRequest::new(
                "Input.dispatchKeyEvent",
                Some(serde_json::to_value(&params).expect("infallible serialization")),
            )
        }
        InputEvent::KeyUp { key, code, m } => {
            let vk = key_to_vk(key);
            let params = DispatchKeyEventParams {
                event_type: "keyUp",
                key: Some(key.clone()),
                code: Some(code.clone()),
                text: None,
                modifiers: to_cdp_modifiers(*m),
                windows_virtual_key_code: vk,
            };
            CdpRequest::new(
                "Input.dispatchKeyEvent",
                Some(serde_json::to_value(&params).expect("infallible serialization")),
            )
        }
        InputEvent::Paste { text } => {
            CdpRequest::new(
                "Input.insertText",
                Some(serde_json::json!({ "text": text })),
            )
        }
        InputEvent::BitrateHint { .. } => {
            // Not a CDP event; handled by the supervisor's input relay.
            // Return a no-op that the write loop will serialize harmlessly.
            CdpRequest::new("Runtime.evaluate", Some(serde_json::json!({"expression": "void 0"})))
        }
    }
}

#[cfg(test)]
mod tests {
    use vscreen_core::event::modifiers;

    use super::*;

    #[test]
    fn mouse_move_to_cdp() {
        let event = InputEvent::MouseMove {
            x: 100.0,
            y: 200.0,
            b: 0,
            m: 0,
        };
        let req = input_to_cdp(&event);
        assert_eq!(req.method, "Input.dispatchMouseEvent");
        let params = req.params.expect("params");
        assert_eq!(params["type"], "mouseMoved");
        assert_eq!(params["x"], 100.0);
    }

    #[test]
    fn mouse_drag_to_cdp() {
        let event = InputEvent::MouseMove {
            x: 150.0,
            y: 250.0,
            b: 1, // left button held
            m: 0,
        };
        let req = input_to_cdp(&event);
        let params = req.params.expect("params");
        assert_eq!(params["type"], "mouseMoved");
        assert_eq!(params["button"], "left");
        assert_eq!(params["buttons"], 1);
    }

    #[test]
    fn mouse_down_to_cdp() {
        let event = InputEvent::MouseDown {
            x: 50.0,
            y: 75.0,
            b: 0,
            m: modifiers::CTRL,
        };
        let req = input_to_cdp(&event);
        let params = req.params.expect("params");
        assert_eq!(params["type"], "mousePressed");
        assert_eq!(params["button"], "left");
        assert_eq!(params["modifiers"], 2);
    }

    #[test]
    fn mouse_up_to_cdp() {
        let event = InputEvent::MouseUp {
            x: 50.0,
            y: 75.0,
            b: 2,
            m: 0,
        };
        let req = input_to_cdp(&event);
        let params = req.params.expect("params");
        assert_eq!(params["type"], "mouseReleased");
        assert_eq!(params["button"], "right");
    }

    #[test]
    fn wheel_to_cdp() {
        let event = InputEvent::Wheel {
            x: 0.0,
            y: 0.0,
            dx: 0.0,
            dy: -120.0,
            m: 0,
        };
        let req = input_to_cdp(&event);
        let params = req.params.expect("params");
        assert_eq!(params["type"], "mouseWheel");
        assert_eq!(params["deltaY"], -120.0);
    }

    #[test]
    fn key_down_printable() {
        let event = InputEvent::KeyDown {
            key: "a".into(),
            code: "KeyA".into(),
            m: modifiers::SHIFT,
        };
        let req = input_to_cdp(&event);
        assert_eq!(req.method, "Input.dispatchKeyEvent");
        let params = req.params.expect("params");
        assert_eq!(params["type"], "keyDown");
        assert_eq!(params["key"], "a");
        assert_eq!(params["text"], "a");
        assert_eq!(params["windowsVirtualKeyCode"], 65); // 'A'
        assert_eq!(params["modifiers"], 8);
    }

    #[test]
    fn key_down_backspace() {
        let event = InputEvent::KeyDown {
            key: "Backspace".into(),
            code: "Backspace".into(),
            m: 0,
        };
        let req = input_to_cdp(&event);
        let params = req.params.expect("params");
        assert_eq!(params["type"], "rawKeyDown");
        assert_eq!(params["windowsVirtualKeyCode"], 8);
        assert!(params.get("text").is_none() || params["text"].is_null());
    }

    #[test]
    fn key_down_arrow() {
        let event = InputEvent::KeyDown {
            key: "ArrowLeft".into(),
            code: "ArrowLeft".into(),
            m: 0,
        };
        let req = input_to_cdp(&event);
        let params = req.params.expect("params");
        assert_eq!(params["type"], "rawKeyDown");
        assert_eq!(params["windowsVirtualKeyCode"], 37);
    }

    #[test]
    fn key_down_enter() {
        let event = InputEvent::KeyDown {
            key: "Enter".into(),
            code: "Enter".into(),
            m: 0,
        };
        let req = input_to_cdp(&event);
        let params = req.params.expect("params");
        assert_eq!(params["type"], "rawKeyDown");
        assert_eq!(params["windowsVirtualKeyCode"], 13);
    }

    #[test]
    fn key_up_to_cdp() {
        let event = InputEvent::KeyUp {
            key: "a".into(),
            code: "KeyA".into(),
            m: 0,
        };
        let req = input_to_cdp(&event);
        let params = req.params.expect("params");
        assert_eq!(params["type"], "keyUp");
        assert_eq!(params["windowsVirtualKeyCode"], 65);
        assert!(params.get("text").is_none() || params["text"].is_null());
    }

    #[test]
    fn key_up_special() {
        let event = InputEvent::KeyUp {
            key: "Delete".into(),
            code: "Delete".into(),
            m: 0,
        };
        let req = input_to_cdp(&event);
        let params = req.params.expect("params");
        assert_eq!(params["type"], "keyUp");
        assert_eq!(params["windowsVirtualKeyCode"], 46);
    }

    #[test]
    fn modifier_mapping() {
        assert_eq!(to_cdp_modifiers(modifiers::ALT), 1);
        assert_eq!(to_cdp_modifiers(modifiers::CTRL), 2);
        assert_eq!(to_cdp_modifiers(modifiers::META), 4);
        assert_eq!(to_cdp_modifiers(modifiers::SHIFT), 8);
        assert_eq!(
            to_cdp_modifiers(modifiers::CTRL | modifiers::SHIFT),
            10
        );
    }

    #[test]
    fn vk_code_mapping() {
        assert_eq!(key_to_vk("Backspace"), Some(8));
        assert_eq!(key_to_vk("Tab"), Some(9));
        assert_eq!(key_to_vk("Enter"), Some(13));
        assert_eq!(key_to_vk("Escape"), Some(27));
        assert_eq!(key_to_vk("ArrowLeft"), Some(37));
        assert_eq!(key_to_vk("ArrowUp"), Some(38));
        assert_eq!(key_to_vk("ArrowRight"), Some(39));
        assert_eq!(key_to_vk("ArrowDown"), Some(40));
        assert_eq!(key_to_vk("Delete"), Some(46));
        assert_eq!(key_to_vk("a"), Some(65));
        assert_eq!(key_to_vk("0"), Some(48));
        assert_eq!(key_to_vk("F1"), Some(112));
        assert_eq!(key_to_vk(" "), Some(32));
    }
}
