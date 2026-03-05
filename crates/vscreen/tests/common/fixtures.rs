use std::time::Instant;

use vscreen_core::config::{AudioConfig, VideoConfig};
use vscreen_core::frame::{AudioBuffer, RgbFrame};
use vscreen_core::instance::{InstanceConfig, InstanceId};

/// Generate a synthetic JPEG test frame (reads the 2x2 fixture file).
pub fn test_jpeg_2x2() -> Vec<u8> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../vscreen-video/tests/fixtures/white_2x2.jpg");
    std::fs::read(path).expect("read test JPEG fixture")
}


/// Generate a solid-color RGB frame.
pub fn solid_rgb_frame(width: u32, height: u32, r: u8, g: u8, b: u8) -> RgbFrame {
    let size = (width * height * 3) as usize;
    let mut data = vec![0u8; size];
    for pixel in data.chunks_exact_mut(3) {
        pixel[0] = r;
        pixel[1] = g;
        pixel[2] = b;
    }
    RgbFrame {
        data,
        width,
        height,
        timestamp: Instant::now(),
    }
}

/// Generate a 440 Hz sine wave audio buffer.
pub fn sine_wave_audio(config: &AudioConfig) -> AudioBuffer {
    let num_samples =
        (config.sample_rate as usize * config.frame_duration_ms as usize / 1000)
            * config.channels as usize;
    let samples: Vec<f32> = (0..num_samples)
        .map(|i| {
            let t = i as f32 / config.sample_rate as f32;
            (t * 440.0 * 2.0 * std::f32::consts::PI).sin() * 0.5
        })
        .collect();

    AudioBuffer {
        samples,
        channels: config.channels,
        sample_rate: config.sample_rate,
        timestamp: Instant::now(),
    }
}

/// Generate a silence audio buffer.
pub fn silence_audio(config: &AudioConfig) -> AudioBuffer {
    let num_samples =
        (config.sample_rate as usize * config.frame_duration_ms as usize / 1000)
            * config.channels as usize;
    AudioBuffer {
        samples: vec![0.0; num_samples],
        channels: config.channels,
        sample_rate: config.sample_rate,
        timestamp: Instant::now(),
    }
}

/// Generate a test instance configuration.
pub fn test_instance_config(id: &str) -> InstanceConfig {
    InstanceConfig {
        instance_id: InstanceId::from(id),
        cdp_endpoint: format!("ws://localhost:9222/devtools/page/{id}"),
        pulse_source: format!("{id}.monitor"),
        display: None,
        video: VideoConfig::default(),
        audio: AudioConfig::default(),
        rtp_output: None,
    }
}

/// Generate a minimal SDP offer for testing.
pub fn test_sdp_offer() -> String {
    "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\n\
     m=video 9 UDP/TLS/RTP/SAVPF 96\r\n\
     a=rtpmap:96 VP9/90000\r\n\
     m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n\
     a=rtpmap:111 opus/48000/2\r\n"
        .to_owned()
}
