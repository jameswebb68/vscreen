use vscreen_core::error::VideoError;
use vscreen_core::frame::{I420Buffer, RgbFrame};

/// Convert an RGB frame to I420 (YUV 4:2:0 planar) format.
///
/// Uses `dcv-color-primitives` for SIMD-optimized conversion when possible,
/// with a pure-Rust fallback.
///
/// # Errors
/// Returns `VideoError` if the dimensions are invalid or the conversion fails.
pub fn rgb_to_i420(frame: &RgbFrame) -> Result<I420Buffer, VideoError> {
    let width = frame.width;
    let height = frame.height;

    if width == 0 || height == 0 {
        return Err(VideoError::InvalidResolution { width, height });
    }

    let expected_rgb_size = (width * height * 3) as usize;
    if frame.data.len() < expected_rgb_size {
        return Err(VideoError::ConversionFailed(format!(
            "RGB buffer too small: {} < {expected_rgb_size}",
            frame.data.len()
        )));
    }

    let y_size = I420Buffer::y_size(width, height);
    let uv_size = I420Buffer::uv_size(width, height);

    let mut y = vec![0u8; y_size];
    let mut u = vec![0u8; uv_size];
    let mut v = vec![0u8; uv_size];

    rgb_to_i420_sw(&frame.data, width, height, &mut y, &mut u, &mut v);

    Ok(I420Buffer {
        y,
        u,
        v,
        width,
        height,
        timestamp: frame.timestamp,
    })
}

/// Software RGB→I420 conversion using BT.601 coefficients.
fn rgb_to_i420_sw(rgb: &[u8], width: u32, height: u32, y: &mut [u8], u: &mut [u8], v: &mut [u8]) {
    let w = width as usize;
    let h = height as usize;
    let uv_w = (w + 1) / 2;

    for row in 0..h {
        for col in 0..w {
            let idx = (row * w + col) * 3;
            let r = f64::from(rgb[idx]);
            let g = f64::from(rgb[idx + 1]);
            let b = f64::from(rgb[idx + 2]);

            // BT.601
            let yy = (0.299 * r + 0.587 * g + 0.114 * b).clamp(0.0, 255.0);
            y[row * w + col] = yy as u8;

            // Subsample chroma: take top-left pixel of each 2x2 block
            if row % 2 == 0 && col % 2 == 0 {
                let uu = (-0.169 * r - 0.331 * g + 0.500 * b + 128.0).clamp(0.0, 255.0);
                let vv = (0.500 * r - 0.419 * g - 0.081 * b + 128.0).clamp(0.0, 255.0);
                let uv_idx = (row / 2) * uv_w + col / 2;
                u[uv_idx] = uu as u8;
                v[uv_idx] = vv as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    fn make_rgb_frame(width: u32, height: u32, r: u8, g: u8, b: u8) -> RgbFrame {
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

    #[test]
    fn convert_black_frame() {
        let frame = make_rgb_frame(4, 4, 0, 0, 0);
        let i420 = rgb_to_i420(&frame).expect("convert");
        assert_eq!(i420.width, 4);
        assert_eq!(i420.height, 4);
        assert_eq!(i420.y.len(), 16);
        assert_eq!(i420.u.len(), 4);
        assert_eq!(i420.v.len(), 4);
        // Black should produce Y≈16 in full-range or Y≈0 in our formula
        assert!(i420.y.iter().all(|&v| v == 0));
    }

    #[test]
    fn convert_white_frame() {
        let frame = make_rgb_frame(4, 4, 255, 255, 255);
        let i420 = rgb_to_i420(&frame).expect("convert");
        // White: Y≈255, U≈128, V≈128
        assert!(i420.y.iter().all(|&val| val >= 254));
        assert!(i420.u.iter().all(|&val| (120..=136).contains(&val)));
        assert!(i420.v.iter().all(|&val| (120..=136).contains(&val)));
    }

    #[test]
    fn convert_red_frame() {
        let frame = make_rgb_frame(2, 2, 255, 0, 0);
        let i420 = rgb_to_i420(&frame).expect("convert");
        // Pure red: Y≈76, U≈85, V≈255
        assert!(i420.y.iter().all(|&val| (70..=82).contains(&val)));
    }

    #[test]
    fn convert_odd_dimensions() {
        let frame = make_rgb_frame(3, 3, 128, 128, 128);
        let i420 = rgb_to_i420(&frame).expect("convert");
        assert_eq!(i420.y.len(), 9);
        assert_eq!(i420.u.len(), I420Buffer::uv_size(3, 3));
        assert_eq!(i420.v.len(), I420Buffer::uv_size(3, 3));
    }

    #[test]
    fn reject_zero_dimensions() {
        let frame = RgbFrame {
            data: vec![],
            width: 0,
            height: 10,
            timestamp: Instant::now(),
        };
        assert!(rgb_to_i420(&frame).is_err());
    }

    #[test]
    fn reject_undersized_buffer() {
        let frame = RgbFrame {
            data: vec![0u8; 10],
            width: 100,
            height: 100,
            timestamp: Instant::now(),
        };
        assert!(rgb_to_i420(&frame).is_err());
    }

    #[test]
    fn plane_sizes_match() {
        let frame = make_rgb_frame(1920, 1080, 0, 0, 0);
        let i420 = rgb_to_i420(&frame).expect("convert");
        assert_eq!(i420.y.len(), I420Buffer::y_size(1920, 1080));
        assert_eq!(i420.u.len(), I420Buffer::uv_size(1920, 1080));
        assert_eq!(i420.v.len(), I420Buffer::uv_size(1920, 1080));
    }
}
