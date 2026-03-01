/// Track type for WebRTC media tracks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Video,
    Audio,
}

/// Configuration for a WebRTC media track.
#[derive(Debug, Clone)]
pub struct TrackConfig {
    pub kind: TrackKind,
    pub codec: &'static str,
    pub clock_rate: u32,
    pub payload_type: u8,
}

impl TrackConfig {
    /// VP9 video track configuration.
    #[must_use]
    pub fn vp9_video() -> Self {
        Self {
            kind: TrackKind::Video,
            codec: "video/VP9",
            clock_rate: 90000,
            payload_type: 96,
        }
    }

    /// H.264 video track configuration.
    #[must_use]
    pub fn h264_video() -> Self {
        Self {
            kind: TrackKind::Video,
            codec: "video/H264",
            clock_rate: 90000,
            payload_type: 102,
        }
    }

    /// Opus audio track configuration.
    #[must_use]
    pub fn opus_audio() -> Self {
        Self {
            kind: TrackKind::Audio,
            codec: "audio/opus",
            clock_rate: 48000,
            payload_type: 111,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_configs() {
        let video = TrackConfig::vp9_video();
        assert_eq!(video.kind, TrackKind::Video);
        assert_eq!(video.clock_rate, 90000);

        let audio = TrackConfig::opus_audio();
        assert_eq!(audio.kind, TrackKind::Audio);
        assert_eq!(audio.clock_rate, 48000);
    }
}
