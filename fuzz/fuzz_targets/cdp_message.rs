#![no_main]

use libfuzzer_sys::fuzz_target;
use vscreen_cdp::protocol::CdpMessage;

fuzz_target!(|data: &[u8]| {
    // Must never panic on arbitrary input
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = serde_json::from_str::<CdpMessage>(text);
    }
});
