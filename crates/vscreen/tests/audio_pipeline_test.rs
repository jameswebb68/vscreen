mod common;

use vscreen_audio::pipeline::AudioPipeline;
use vscreen_core::config::AudioConfig;

use crate::common::fixtures::{silence_audio, sine_wave_audio};

#[tokio::test]
async fn audio_pipeline_produces_encoded_packets() {
    let config = AudioConfig {
        frame_duration_ms: 10,
        ..AudioConfig::default()
    };
    let mut pipeline = AudioPipeline::start("test", config).expect("start");

    let mut rx = pipeline.encoded_packets();
    let packet = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("timeout")
        .expect("packet");

    assert!(!packet.data.is_empty());

    pipeline.stop().await;
}

#[tokio::test]
async fn audio_pipeline_state_lifecycle() {
    let config = AudioConfig {
        frame_duration_ms: 10,
        ..AudioConfig::default()
    };
    let mut pipeline = AudioPipeline::start("test", config).expect("start");

    let state = pipeline.state();
    assert_eq!(
        *state.borrow(),
        vscreen_audio::AudioCaptureState::Capturing
    );

    pipeline.stop().await;
    assert_eq!(
        *pipeline.state().borrow(),
        vscreen_audio::AudioCaptureState::Stopped
    );
}

#[test]
fn opus_encode_sine_wave() {
    use vscreen_audio::encode::OpusEncoder;
    use vscreen_core::traits::AudioEncoder;

    let config = AudioConfig::default();
    let mut encoder = OpusEncoder::new(config.clone()).expect("init");
    let buf = sine_wave_audio(&config);

    let pkt = encoder.encode(&buf).expect("encode");
    assert!(!pkt.data.is_empty());
}

#[test]
fn opus_encode_silence() {
    use vscreen_audio::encode::OpusEncoder;
    use vscreen_core::traits::AudioEncoder;

    let config = AudioConfig::default();
    let mut encoder = OpusEncoder::new(config.clone()).expect("init");
    let buf = silence_audio(&config);

    let pkt = encoder.encode(&buf).expect("encode");
    assert!(!pkt.data.is_empty());
}
