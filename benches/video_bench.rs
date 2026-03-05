use std::time::Instant;

use criterion::{Criterion, criterion_group, criterion_main};
use vscreen_core::config::VideoConfig;
use vscreen_core::frame::RgbFrame;
use vscreen_video::convert::rgb_to_i420;

fn make_rgb(width: u32, height: u32) -> RgbFrame {
    RgbFrame {
        data: vec![128u8; (width * height * 3) as usize],
        width,
        height,
        timestamp: Instant::now(),
    }
}

fn bench_rgb_to_i420_1080p(c: &mut Criterion) {
    let frame = make_rgb(1920, 1080);
    c.bench_function("rgb_to_i420_1080p", |b| {
        b.iter(|| rgb_to_i420(&frame).expect("convert"))
    });
}

fn bench_rgb_to_i420_720p(c: &mut Criterion) {
    let frame = make_rgb(1280, 720);
    c.bench_function("rgb_to_i420_720p", |b| {
        b.iter(|| rgb_to_i420(&frame).expect("convert"))
    });
}

fn bench_rgb_to_i420_480p(c: &mut Criterion) {
    let frame = make_rgb(854, 480);
    c.bench_function("rgb_to_i420_480p", |b| {
        b.iter(|| rgb_to_i420(&frame).expect("convert"))
    });
}

criterion_group!(
    benches,
    bench_rgb_to_i420_1080p,
    bench_rgb_to_i420_720p,
    bench_rgb_to_i420_480p
);
criterion_main!(benches);
