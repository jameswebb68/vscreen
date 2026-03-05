#![no_main]

use libfuzzer_sys::fuzz_target;
use vscreen_transport::webrtc_session::data_channel::parse_input_message;

fuzz_target!(|data: &[u8]| {
    // Must never panic on arbitrary input
    let _ = parse_input_message(data);
});
