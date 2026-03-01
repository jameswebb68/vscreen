use std::time::Instant;

use criterion::{Criterion, criterion_group, criterion_main};
use vscreen_audio::encode::OpusEncoder;
use vscreen_core::config::AudioConfig;
use vscreen_core::frame::AudioBuffer;
use vscreen_core::traits::AudioEncoder;

fn make_audio(config: &AudioConfig) -> AudioBuffer {
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

fn bench_opus_encode_20ms(c: &mut Criterion) {
    let config = AudioConfig::default();
    let mut encoder = OpusEncoder::new(config.clone()).expect("init");
    let buffer = make_audio(&config);

    c.bench_function("opus_encode_20ms_stereo", |b| {
        b.iter(|| encoder.encode(&buffer).expect("encode"))
    });
}

fn bench_opus_encode_mono(c: &mut Criterion) {
    let config = AudioConfig {
        channels: 1,
        ..AudioConfig::default()
    };
    let mut encoder = OpusEncoder::new(config.clone()).expect("init");
    let buffer = make_audio(&config);

    c.bench_function("opus_encode_20ms_mono", |b| {
        b.iter(|| encoder.encode(&buffer).expect("encode"))
    });
}

criterion_group!(benches, bench_opus_encode_20ms, bench_opus_encode_mono);
criterion_main!(benches);
