use std::sync::atomic::{AtomicBool, Ordering};

use bytes::Bytes;
use openh264::encoder::{Encoder, EncoderConfig, FrameType};
use openh264::formats::YUVSource;
use openh264::OpenH264API;
use tracing::{debug, trace};
use vscreen_core::config::VideoConfig;
use vscreen_core::error::VideoError;
use vscreen_core::frame::{EncodedPacket, I420Buffer, VideoCodec};

/// Adapter that implements [`YUVSource`] for [`I420Buffer`] so we can pass
/// it directly to the openh264 encoder without copying.
struct I420Adapter<'a> {
    buf: &'a I420Buffer,
}

impl YUVSource for I420Adapter<'_> {
    fn dimensions(&self) -> (usize, usize) {
        (self.buf.width as usize, self.buf.height as usize)
    }

    fn strides(&self) -> (usize, usize, usize) {
        let y_stride = self.buf.width as usize;
        let uv_stride = ((self.buf.width + 1) / 2) as usize;
        (y_stride, uv_stride, uv_stride)
    }

    fn y(&self) -> &[u8] {
        &self.buf.y
    }

    fn u(&self) -> &[u8] {
        &self.buf.u
    }

    fn v(&self) -> &[u8] {
        &self.buf.v
    }
}

/// H.264 encoder using the openh264 (Cisco) library via its Rust bindings.
///
/// Outputs Annex-B NAL units which the downstream H.264 RTP packetizer
/// splits into single-NAL or FU-A packets.
pub struct H264Encoder {
    encoder: Encoder,
    config: VideoConfig,
    frame_count: u64,
    keyframe_requested: AtomicBool,
}

impl std::fmt::Debug for H264Encoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("H264Encoder")
            .field("config", &self.config)
            .field("frame_count", &self.frame_count)
            .finish_non_exhaustive()
    }
}

impl H264Encoder {
    /// Create a new H.264 encoder with the given config.
    ///
    /// # Errors
    /// Returns `VideoError` if the configuration is invalid or openh264 init fails.
    pub fn new(config: VideoConfig) -> Result<Self, VideoError> {
        if config.width == 0 || config.height == 0 {
            return Err(VideoError::InvalidResolution {
                width: config.width,
                height: config.height,
            });
        }
        if config.width % 2 != 0 || config.height % 2 != 0 {
            return Err(VideoError::InvalidResolution {
                width: config.width,
                height: config.height,
            });
        }

        let encoder = Self::build_encoder(&config)?;

        debug!(
            width = config.width,
            height = config.height,
            bitrate_kbps = config.bitrate_kbps,
            framerate = config.framerate,
            "H.264 encoder initialized (openh264)"
        );

        Ok(Self {
            encoder,
            config,
            frame_count: 0,
            keyframe_requested: AtomicBool::new(true),
        })
    }

    fn build_encoder(config: &VideoConfig) -> Result<Encoder, VideoError> {
        let enc_config = EncoderConfig::new()
            .set_bitrate_bps(config.bitrate_kbps * 1000)
            .max_frame_rate(config.framerate as f32)
            .rate_control_mode(openh264::encoder::RateControlMode::Bitrate)
            .enable_skip_frame(false);

        Encoder::with_api_config(OpenH264API::from_source(), enc_config)
            .map_err(|e| VideoError::EncodeFailed(format!("openh264 init: {e}")))
    }
}

impl vscreen_core::traits::VideoEncoder for H264Encoder {
    fn encode(&mut self, frame: &I420Buffer) -> Result<EncodedPacket, VideoError> {
        if frame.width != self.config.width || frame.height != self.config.height {
            return Err(VideoError::InvalidResolution {
                width: frame.width,
                height: frame.height,
            });
        }

        if self.keyframe_requested.swap(false, Ordering::SeqCst) {
            self.encoder.force_intra_frame();
        }

        let adapter = I420Adapter { buf: frame };
        let bitstream = self
            .encoder
            .encode(&adapter)
            .map_err(|e| VideoError::EncodeFailed(format!("openh264 encode: {e}")))?;

        let is_keyframe = matches!(
            bitstream.frame_type(),
            FrameType::IDR | FrameType::I
        );

        let data = bitstream.to_vec();

        let packet = EncodedPacket {
            data: Bytes::from(data),
            is_keyframe,
            pts: self.frame_count,
            duration: Some(1000 / u64::from(self.config.framerate)),
            codec: Some(VideoCodec::H264),
        };

        self.frame_count += 1;

        trace!(
            pts = packet.pts,
            is_keyframe = packet.is_keyframe,
            size = packet.size(),
            "encoded H.264 frame"
        );

        Ok(packet)
    }

    fn request_keyframe(&mut self) {
        self.keyframe_requested.store(true, Ordering::SeqCst);
        debug!("keyframe requested for next H.264 encode");
    }

    fn reconfigure(&mut self, config: VideoConfig) -> Result<(), VideoError> {
        if config.width == 0 || config.height == 0 {
            return Err(VideoError::InvalidResolution {
                width: config.width,
                height: config.height,
            });
        }
        if config.width % 2 != 0 || config.height % 2 != 0 {
            return Err(VideoError::InvalidResolution {
                width: config.width,
                height: config.height,
            });
        }

        self.encoder = Self::build_encoder(&config)?;

        debug!(
            width = config.width,
            height = config.height,
            bitrate_kbps = config.bitrate_kbps,
            "H.264 encoder reconfigured"
        );

        self.config = config;
        self.keyframe_requested.store(true, Ordering::SeqCst);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use vscreen_core::traits::VideoEncoder;

    use super::*;

    fn make_i420(width: u32, height: u32) -> I420Buffer {
        I420Buffer {
            y: vec![128u8; I420Buffer::y_size(width, height)],
            u: vec![128u8; I420Buffer::uv_size(width, height)],
            v: vec![128u8; I420Buffer::uv_size(width, height)],
            width,
            height,
            timestamp: Instant::now(),
        }
    }

    #[test]
    fn encode_produces_packet() {
        let config = VideoConfig::default();
        let mut encoder = H264Encoder::new(config).expect("init");
        let frame = make_i420(1920, 1080);
        let packet = encoder.encode(&frame).expect("encode");
        assert!(packet.is_keyframe);
        assert!(!packet.data.is_empty());
        assert_eq!(packet.codec, Some(VideoCodec::H264));
    }

    #[test]
    fn keyframe_request() {
        let config = VideoConfig::default();
        let mut encoder = H264Encoder::new(config).expect("init");
        let frame = make_i420(1920, 1080);
        let _ = encoder.encode(&frame).expect("encode 1");
        encoder.request_keyframe();
        let pkt = encoder.encode(&frame).expect("encode 2");
        assert!(pkt.is_keyframe);
    }

    #[test]
    fn reject_zero_resolution() {
        let mut config = VideoConfig::default();
        config.width = 0;
        assert!(H264Encoder::new(config).is_err());
    }

    #[test]
    fn reject_mismatched_frame() {
        let config = VideoConfig::default();
        let mut encoder = H264Encoder::new(config).expect("init");
        let frame = make_i420(640, 480);
        assert!(encoder.encode(&frame).is_err());
    }

    #[test]
    fn reject_odd_dimensions() {
        let config = VideoConfig {
            width: 1921,
            height: 1080,
            ..VideoConfig::default()
        };
        assert!(H264Encoder::new(config).is_err());
    }

    #[test]
    fn pts_increments() {
        let config = VideoConfig::default();
        let mut encoder = H264Encoder::new(config).expect("init");
        let frame = make_i420(1920, 1080);

        let p0 = encoder.encode(&frame).expect("e0");
        let p1 = encoder.encode(&frame).expect("e1");
        let p2 = encoder.encode(&frame).expect("e2");

        assert_eq!(p0.pts, 0);
        assert_eq!(p1.pts, 1);
        assert_eq!(p2.pts, 2);
    }
}
