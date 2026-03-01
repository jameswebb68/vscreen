mod common;

use vscreen_core::config::VideoConfig;
use vscreen_core::frame::VideoCodec;
use vscreen_video::convert::rgb_to_i420;
use vscreen_video::decode::decode_jpeg;
use vscreen_video::pipeline::VideoPipeline;

use crate::common::fixtures::{solid_rgb_frame, test_jpeg_2x2};

#[test]
fn full_pipeline_jpeg_to_encoded_vp9() {
    let jpeg_data = test_jpeg_2x2();
    let config = VideoConfig {
        width: 2,
        height: 2,
        codec: VideoCodec::Vp9,
        ..VideoConfig::default()
    };
    let mut pipeline = VideoPipeline::new(config).expect("create pipeline");

    let packet = pipeline.process(&jpeg_data).expect("process");
    assert!(!packet.data.is_empty());
    assert!(packet.is_keyframe);
    assert_eq!(pipeline.frames_processed(), 1);
}

#[test]
fn rgb_to_i420_to_encode() {
    let frame = solid_rgb_frame(640, 480, 128, 64, 200);
    let i420 = rgb_to_i420(&frame).expect("convert");

    assert_eq!(i420.width, 640);
    assert_eq!(i420.height, 480);
    assert_eq!(i420.y.len(), 640 * 480);
}

#[test]
fn decode_and_convert_chain() {
    let jpeg_data = test_jpeg_2x2();
    let rgb = decode_jpeg(&jpeg_data, std::time::Instant::now()).expect("decode");
    let i420 = rgb_to_i420(&rgb).expect("convert");
    assert_eq!(i420.width, 2);
    assert_eq!(i420.height, 2);
}

#[test]
fn pipeline_handles_resolution_change_vp9() {
    let jpeg_data = test_jpeg_2x2();
    let config = VideoConfig {
        width: 1920,
        height: 1080,
        codec: VideoCodec::Vp9,
        ..VideoConfig::default()
    };
    let mut pipeline = VideoPipeline::new(config).expect("create pipeline");

    let packet = pipeline.process(&jpeg_data).expect("process");
    assert!(!packet.data.is_empty());
}

#[test]
fn pipeline_reject_corrupt_jpeg() {
    let config = VideoConfig::default();
    let mut pipeline = VideoPipeline::new(config).expect("create pipeline");

    let corrupt = vec![0xFF, 0xD8, 0xFF, 0x00, 0x00];
    let result = pipeline.process(&corrupt);
    assert!(result.is_err());
}
