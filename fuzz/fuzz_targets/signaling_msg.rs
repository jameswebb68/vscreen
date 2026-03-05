#![no_main]

use libfuzzer_sys::fuzz_target;
use vscreen_transport::webrtc_session::signaling::SignalingMessage;

fuzz_target!(|data: &[u8]| {
    // Must never panic on arbitrary input
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = SignalingMessage::from_json(text);
    }
});
