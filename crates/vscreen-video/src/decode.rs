use std::io::Cursor;

use tracing::{debug, warn};
use vscreen_core::error::VideoError;
use vscreen_core::frame::RgbFrame;

const MAX_FRAME_SIZE: usize = 2 * 1024 * 1024; // 2 MB

/// Decode JPEG data into an RGB frame.
///
/// # Errors
/// Returns `VideoError` if:
/// - The input exceeds the 2 MB size limit
/// - The JPEG data is invalid or corrupted
pub fn decode_jpeg(data: &[u8], timestamp: std::time::Instant) -> Result<RgbFrame, VideoError> {
    if data.len() > MAX_FRAME_SIZE {
        return Err(VideoError::FrameTooLarge {
            size: data.len(),
            max: MAX_FRAME_SIZE,
        });
    }

    if data.is_empty() {
        return Err(VideoError::DecodeFailed("empty input".into()));
    }

    let cursor = Cursor::new(data);
    let mut decoder = zune_jpeg::JpegDecoder::new(cursor);
    decoder.decode_headers().map_err(|e| {
        warn!(?e, "JPEG header decode failed");
        VideoError::DecodeFailed(format!("header: {e}"))
    })?;

    let (width, height) = decoder.dimensions().ok_or_else(|| {
        VideoError::DecodeFailed("no dimensions after header decode".into())
    })?;

    let width = width as u32;
    let height = height as u32;

    if width == 0 || height == 0 {
        return Err(VideoError::InvalidResolution { width, height });
    }

    let pixels = decoder.decode().map_err(|e| {
        warn!(?e, "JPEG pixel decode failed");
        VideoError::DecodeFailed(format!("pixels: {e}"))
    })?;

    debug!(width, height, size = data.len(), "decoded JPEG frame");

    Ok(RgbFrame {
        data: pixels,
        width,
        height,
        timestamp,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_jpeg() -> Vec<u8> {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/white_2x2.jpg");
        std::fs::read(path).expect("read test fixture")
    }

    #[test]
    fn reject_oversized_input() {
        let data = vec![0xFF; MAX_FRAME_SIZE + 1];
        let result = decode_jpeg(&data, std::time::Instant::now());
        assert!(matches!(result, Err(VideoError::FrameTooLarge { .. })));
    }

    #[test]
    fn reject_empty_input() {
        let result = decode_jpeg(&[], std::time::Instant::now());
        assert!(matches!(result, Err(VideoError::DecodeFailed(_))));
    }

    #[test]
    fn reject_corrupt_data() {
        let data = vec![0xFF, 0xD8, 0xFF, 0x00, 0x00];
        let result = decode_jpeg(&data, std::time::Instant::now());
        assert!(result.is_err());
    }

    #[test]
    fn decode_valid_jpeg() {
        let data = fixture_jpeg();
        let frame = decode_jpeg(&data, std::time::Instant::now()).expect("decode");
        assert_eq!(frame.width, 2);
        assert_eq!(frame.height, 2);
        assert!(!frame.data.is_empty());
    }
}
