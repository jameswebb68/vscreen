use tracing::{debug, warn};
use vscreen_core::config::VideoConfig;
use vscreen_core::error::VideoError;
use vscreen_core::frame::{EncodedPacket, VideoCodec};
use vscreen_core::traits::VideoEncoder;

use crate::convert::rgb_to_i420;
use crate::decode::decode_jpeg;
use crate::encode::Vp9Encoder;
use crate::h264_encode::H264Encoder;
use vscreen_core::frame::RgbFrame;

/// Orchestrates the full video pipeline: JPEG decode -> RGB -> I420 -> encode.
///
/// The encoder backend (VP9 or H.264) is selected at construction time via
/// the `codec` parameter.
pub struct VideoPipeline {
    encoder: Box<dyn VideoEncoder>,
    codec: VideoCodec,
    config: VideoConfig,
    frames_processed: u64,
    frames_dropped: u64,
}

impl std::fmt::Debug for VideoPipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoPipeline")
            .field("codec", &self.codec)
            .field("config", &self.config)
            .field("frames_processed", &self.frames_processed)
            .field("frames_dropped", &self.frames_dropped)
            .finish_non_exhaustive()
    }
}

impl VideoPipeline {
    /// Create a new video pipeline using the codec specified in `config.codec`.
    ///
    /// # Errors
    /// Returns `VideoError` if the encoder cannot be initialized.
    pub fn new(config: VideoConfig) -> Result<Self, VideoError> {
        let codec = config.codec;
        Self::with_codec(config, codec)
    }

    /// Create a new video pipeline with an explicit codec selection.
    ///
    /// # Errors
    /// Returns `VideoError` if the encoder cannot be initialized.
    pub fn with_codec(config: VideoConfig, codec: VideoCodec) -> Result<Self, VideoError> {
        let encoder = Self::make_encoder(&config, codec)?;
        debug!(
            width = config.width,
            height = config.height,
            %codec,
            "video pipeline initialized"
        );
        Ok(Self {
            encoder,
            codec,
            config,
            frames_processed: 0,
            frames_dropped: 0,
        })
    }

    fn make_encoder(
        config: &VideoConfig,
        codec: VideoCodec,
    ) -> Result<Box<dyn VideoEncoder>, VideoError> {
        match codec {
            VideoCodec::Vp9 => Ok(Box::new(Vp9Encoder::new(config.clone())?)),
            VideoCodec::H264 => Ok(Box::new(H264Encoder::new(config.clone())?)),
        }
    }

    /// The codec this pipeline is using.
    #[must_use]
    pub fn codec(&self) -> VideoCodec {
        self.codec
    }

    /// Process raw JPEG data through the full pipeline.
    ///
    /// Steps: decode JPEG -> convert RGB -> I420 -> encode.
    ///
    /// # Errors
    /// Returns `VideoError` if any pipeline stage fails.
    pub fn process(&mut self, jpeg_data: &[u8]) -> Result<EncodedPacket, VideoError> {
        let timestamp = std::time::Instant::now();

        let mut rgb_frame = decode_jpeg(jpeg_data, timestamp)?;

        // Both VP9 and H.264 require even dimensions; pad if needed
        pad_to_even(&mut rgb_frame);

        if rgb_frame.width != self.config.width || rgb_frame.height != self.config.height {
            debug!(
                old_w = self.config.width,
                old_h = self.config.height,
                new_w = rgb_frame.width,
                new_h = rgb_frame.height,
                "resolution changed, reconfiguring encoder"
            );
            let new_config = VideoConfig {
                width: rgb_frame.width,
                height: rgb_frame.height,
                ..self.config.clone()
            };
            self.encoder.reconfigure(new_config.clone())?;
            self.config = new_config;
        }

        let i420 = rgb_to_i420(&rgb_frame)?;
        let packet = self.encoder.encode(&i420)?;

        self.frames_processed += 1;

        Ok(packet)
    }

    /// Reconfigure the pipeline (e.g., for runtime bitrate/framerate changes).
    ///
    /// # Errors
    /// Returns `VideoError` if reconfiguration fails.
    pub fn reconfigure(&mut self, config: VideoConfig) -> Result<(), VideoError> {
        self.encoder.reconfigure(config.clone())?;
        self.config = config;
        Ok(())
    }

    /// Reconfigure only the bitrate (for adaptive bitrate).
    pub fn reconfigure_bitrate(&mut self, bitrate_kbps: u32) {
        let new_config = VideoConfig {
            bitrate_kbps,
            ..self.config.clone()
        };
        if let Err(e) = self.reconfigure(new_config) {
            warn!(?e, "failed to reconfigure bitrate");
        }
    }

    /// Request the next frame to be a keyframe.
    pub fn request_keyframe(&mut self) {
        self.encoder.request_keyframe();
    }

    /// Total frames processed by this pipeline.
    #[must_use]
    pub fn frames_processed(&self) -> u64 {
        self.frames_processed
    }

    /// Total frames dropped (decode/encode failures).
    #[must_use]
    pub fn frames_dropped(&self) -> u64 {
        self.frames_dropped
    }

    /// Record a dropped frame.
    pub fn record_drop(&mut self) {
        self.frames_dropped += 1;
        warn!(
            total_dropped = self.frames_dropped,
            "video frame dropped"
        );
    }
}

/// Pad an RGB frame so both width and height are even (required by VP9).
/// Adds black pixels on the right/bottom edge if needed — at most 1 pixel.
fn pad_to_even(frame: &mut RgbFrame) {
    let target_w = (frame.width + 1) & !1;
    let target_h = (frame.height + 1) & !1;

    if target_w == frame.width && target_h == frame.height {
        return;
    }

    let old_stride = (frame.width * 3) as usize;
    let new_stride = (target_w * 3) as usize;
    let new_size = new_stride * target_h as usize;
    let mut padded = vec![0u8; new_size];

    for row in 0..frame.height as usize {
        let src = row * old_stride;
        let dst = row * new_stride;
        padded[dst..dst + old_stride].copy_from_slice(&frame.data[src..src + old_stride]);
    }

    frame.data = padded;
    frame.width = target_w;
    frame.height = target_h;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_creation() {
        let config = VideoConfig::default();
        let pipeline = VideoPipeline::new(config);
        assert!(pipeline.is_ok());
    }

    #[test]
    fn pipeline_rejects_bad_config() {
        let config = VideoConfig {
            width: 0,
            height: 0,
            ..VideoConfig::default()
        };
        assert!(VideoPipeline::new(config).is_err());
    }

    #[test]
    fn pipeline_counters() {
        let config = VideoConfig::default();
        let mut pipeline = VideoPipeline::new(config).expect("init");
        assert_eq!(pipeline.frames_processed(), 0);
        assert_eq!(pipeline.frames_dropped(), 0);
        pipeline.record_drop();
        assert_eq!(pipeline.frames_dropped(), 1);
    }

    #[test]
    fn pipeline_reject_empty_jpeg() {
        let config = VideoConfig::default();
        let mut pipeline = VideoPipeline::new(config).expect("init");
        assert!(pipeline.process(&[]).is_err());
    }
}
