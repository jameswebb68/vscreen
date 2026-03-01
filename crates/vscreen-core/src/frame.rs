use std::fmt;
use std::time::Instant;

use bytes::Bytes;
use serde::{Deserialize, Serialize};

/// Supported video codecs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoCodec {
    Vp9,
    H264,
}

impl Default for VideoCodec {
    fn default() -> Self {
        Self::H264
    }
}

impl fmt::Display for VideoCodec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Vp9 => write!(f, "vp9"),
            Self::H264 => write!(f, "h264"),
        }
    }
}

impl std::str::FromStr for VideoCodec {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "vp9" => Ok(Self::Vp9),
            "h264" | "h.264" => Ok(Self::H264),
            other => Err(format!("unknown video codec: {other}")),
        }
    }
}

/// Raw JPEG frame data received from CDP screencast.
#[derive(Debug, Clone)]
pub struct RawFrame {
    pub data: Bytes,
    pub timestamp: Instant,
    pub session_id: u32,
}

impl RawFrame {
    #[must_use]
    pub fn new(data: impl Into<Bytes>, session_id: u32) -> Self {
        Self {
            data: data.into(),
            timestamp: Instant::now(),
            session_id,
        }
    }

    #[must_use]
    pub fn size(&self) -> usize {
        self.data.len()
    }
}

/// Decoded RGB frame ready for color conversion.
#[derive(Debug, Clone)]
pub struct RgbFrame {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub timestamp: Instant,
}

/// YUV I420 buffer ready for VP9 encoding.
#[derive(Debug, Clone)]
pub struct I420Buffer {
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub timestamp: Instant,
}

impl I420Buffer {
    /// Expected Y plane size.
    #[must_use]
    pub fn y_size(width: u32, height: u32) -> usize {
        (width * height) as usize
    }

    /// Expected U or V plane size (quarter of Y).
    #[must_use]
    pub fn uv_size(width: u32, height: u32) -> usize {
        (((width + 1) / 2) * ((height + 1) / 2)) as usize
    }
}

/// Encoded media packet (VP9, H.264, or Opus).
#[derive(Debug, Clone)]
pub struct EncodedPacket {
    pub data: Bytes,
    pub is_keyframe: bool,
    pub pts: u64,
    pub duration: Option<u64>,
    /// Which video codec produced this packet (`None` for audio).
    pub codec: Option<VideoCodec>,
}

impl EncodedPacket {
    #[must_use]
    pub fn new(data: impl Into<Bytes>, is_keyframe: bool, pts: u64) -> Self {
        Self {
            data: data.into(),
            is_keyframe,
            pts,
            duration: None,
            codec: None,
        }
    }

    #[must_use]
    pub fn with_codec(data: impl Into<Bytes>, is_keyframe: bool, pts: u64, codec: VideoCodec) -> Self {
        Self {
            data: data.into(),
            is_keyframe,
            pts,
            duration: None,
            codec: Some(codec),
        }
    }

    #[must_use]
    pub fn size(&self) -> usize {
        self.data.len()
    }
}

/// Raw audio sample buffer.
#[derive(Debug, Clone)]
pub struct AudioBuffer {
    pub samples: Vec<f32>,
    pub channels: u16,
    pub sample_rate: u32,
    pub timestamp: Instant,
}

impl AudioBuffer {
    #[must_use]
    pub fn num_frames(&self) -> usize {
        if self.channels == 0 {
            return 0;
        }
        self.samples.len() / self.channels as usize
    }

    #[must_use]
    pub fn duration_ms(&self) -> f64 {
        if self.sample_rate == 0 {
            return 0.0;
        }
        self.num_frames() as f64 / f64::from(self.sample_rate) * 1000.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_frame_size() {
        let frame = RawFrame::new(vec![0u8; 1024], 1);
        assert_eq!(frame.size(), 1024);
    }

    #[test]
    fn i420_plane_sizes() {
        assert_eq!(I420Buffer::y_size(1920, 1080), 1920 * 1080);
        // 1920/2=960, (1080+1)/2=540 → but exact: 960*540=518400
        assert_eq!(I420Buffer::uv_size(1920, 1080), (1920 / 2) * ((1080 + 1) / 2));
    }

    #[test]
    fn i420_odd_dimensions() {
        assert_eq!(I420Buffer::uv_size(3, 3), 2 * 2);
    }

    #[test]
    fn encoded_packet_size() {
        let pkt = EncodedPacket::new(vec![0u8; 512], true, 0);
        assert_eq!(pkt.size(), 512);
        assert!(pkt.is_keyframe);
    }

    #[test]
    fn audio_buffer_num_frames() {
        let buf = AudioBuffer {
            samples: vec![0.0; 960],
            channels: 2,
            sample_rate: 48000,
            timestamp: Instant::now(),
        };
        assert_eq!(buf.num_frames(), 480);
    }

    #[test]
    fn audio_buffer_duration() {
        let buf = AudioBuffer {
            samples: vec![0.0; 960],
            channels: 2,
            sample_rate: 48000,
            timestamp: Instant::now(),
        };
        let dur = buf.duration_ms();
        assert!((dur - 10.0).abs() < 0.01);
    }

    #[test]
    fn audio_buffer_zero_channels() {
        let buf = AudioBuffer {
            samples: vec![0.0; 100],
            channels: 0,
            sample_rate: 48000,
            timestamp: Instant::now(),
        };
        assert_eq!(buf.num_frames(), 0);
    }
}
