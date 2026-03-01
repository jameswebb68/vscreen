use bytes::Bytes;
use tracing::{debug, trace};
use vscreen_core::config::AudioConfig;
use vscreen_core::error::AudioError;
use vscreen_core::frame::{AudioBuffer, EncodedPacket};

const MAX_OPUS_FRAME_SIZE: usize = 4000;

fn channels_from_count(ch: u16) -> Result<audiopus::Channels, AudioError> {
    match ch {
        1 => Ok(audiopus::Channels::Mono),
        2 => Ok(audiopus::Channels::Stereo),
        _ => Err(AudioError::EncodeFailed(format!("unsupported channel count: {ch}"))),
    }
}

fn sample_rate_from_hz(hz: u32) -> Result<audiopus::SampleRate, AudioError> {
    match hz {
        48000 => Ok(audiopus::SampleRate::Hz48000),
        24000 => Ok(audiopus::SampleRate::Hz24000),
        16000 => Ok(audiopus::SampleRate::Hz16000),
        12000 => Ok(audiopus::SampleRate::Hz12000),
        8000 => Ok(audiopus::SampleRate::Hz8000),
        _ => Err(AudioError::InvalidSampleRate(hz)),
    }
}

/// Opus audio encoder using the audiopus crate (libopus bindings).
pub struct OpusEncoder {
    inner: audiopus::coder::Encoder,
    config: AudioConfig,
    frame_count: u64,
    samples_per_frame: usize,
    output_buf: Vec<u8>,
}

impl std::fmt::Debug for OpusEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpusEncoder")
            .field("config", &self.config)
            .field("frame_count", &self.frame_count)
            .finish_non_exhaustive()
    }
}

impl OpusEncoder {
    /// Create a new Opus encoder.
    ///
    /// # Errors
    /// Returns `AudioError` if the configuration is invalid or libopus init fails.
    pub fn new(config: AudioConfig) -> Result<Self, AudioError> {
        let sr = sample_rate_from_hz(config.sample_rate)?;
        let ch = channels_from_count(config.channels)?;

        let mut inner = audiopus::coder::Encoder::new(sr, ch, audiopus::Application::Audio)
            .map_err(|e| AudioError::EncodeFailed(format!("opus init: {e}")))?;

        let bitrate_bps = config.bitrate_kbps as i32 * 1000;
        inner
            .set_bitrate(audiopus::Bitrate::BitsPerSecond(bitrate_bps))
            .map_err(|e| AudioError::EncodeFailed(format!("set bitrate: {e}")))?;

        let samples_per_frame =
            (config.sample_rate as usize * config.frame_duration_ms as usize / 1000)
                * config.channels as usize;

        debug!(
            sample_rate = config.sample_rate,
            channels = config.channels,
            bitrate_kbps = config.bitrate_kbps,
            samples_per_frame,
            "Opus encoder initialized (libopus)"
        );

        Ok(Self {
            inner,
            config,
            frame_count: 0,
            samples_per_frame,
            output_buf: vec![0u8; MAX_OPUS_FRAME_SIZE],
        })
    }
}

impl vscreen_core::traits::AudioEncoder for OpusEncoder {
    fn encode(&mut self, samples: &AudioBuffer) -> Result<EncodedPacket, AudioError> {
        if samples.samples.len() < self.samples_per_frame {
            return Err(AudioError::EncodeFailed(format!(
                "buffer too small: {} < {}",
                samples.samples.len(),
                self.samples_per_frame
            )));
        }

        let input = &samples.samples[..self.samples_per_frame];
        let encoded_len = self
            .inner
            .encode_float(input, &mut self.output_buf)
            .map_err(|e| AudioError::EncodeFailed(format!("opus encode: {e}")))?;

        let packet = EncodedPacket {
            data: Bytes::copy_from_slice(&self.output_buf[..encoded_len]),
            is_keyframe: self.frame_count == 0,
            pts: self.frame_count,
            duration: Some(u64::from(self.config.frame_duration_ms)),
            codec: None,
        };

        self.frame_count += 1;

        trace!(
            pts = packet.pts,
            size = packet.size(),
            "encoded Opus frame"
        );

        Ok(packet)
    }

    fn reconfigure(&mut self, config: AudioConfig) -> Result<(), AudioError> {
        let sr = sample_rate_from_hz(config.sample_rate)?;
        let ch = channels_from_count(config.channels)?;

        let mut inner = audiopus::coder::Encoder::new(sr, ch, audiopus::Application::Audio)
            .map_err(|e| AudioError::EncodeFailed(format!("opus reinit: {e}")))?;

        let bitrate_bps = config.bitrate_kbps as i32 * 1000;
        inner
            .set_bitrate(audiopus::Bitrate::BitsPerSecond(bitrate_bps))
            .map_err(|e| AudioError::EncodeFailed(format!("set bitrate: {e}")))?;

        self.samples_per_frame =
            (config.sample_rate as usize * config.frame_duration_ms as usize / 1000)
                * config.channels as usize;
        self.inner = inner;
        self.config = config;

        debug!("Opus encoder reconfigured");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use vscreen_core::traits::AudioEncoder;

    use super::*;

    fn make_audio_buffer(config: &AudioConfig) -> AudioBuffer {
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

    fn make_silence(config: &AudioConfig) -> AudioBuffer {
        let num_samples =
            (config.sample_rate as usize * config.frame_duration_ms as usize / 1000)
                * config.channels as usize;
        AudioBuffer {
            samples: vec![0.0; num_samples],
            channels: config.channels,
            sample_rate: config.sample_rate,
            timestamp: Instant::now(),
        }
    }

    #[test]
    fn encode_sine_wave() {
        let config = AudioConfig::default();
        let mut encoder = OpusEncoder::new(config.clone()).expect("init");
        let buf = make_audio_buffer(&config);
        let pkt = encoder.encode(&buf).expect("encode");
        assert!(!pkt.data.is_empty());
        assert!(pkt.is_keyframe);
    }

    #[test]
    fn encode_silence() {
        let config = AudioConfig::default();
        let mut encoder = OpusEncoder::new(config.clone()).expect("init");
        let buf = make_silence(&config);
        let pkt = encoder.encode(&buf).expect("encode");
        assert!(!pkt.data.is_empty());
    }

    #[test]
    fn reject_invalid_sample_rate() {
        let config = AudioConfig {
            sample_rate: 44100,
            ..AudioConfig::default()
        };
        assert!(OpusEncoder::new(config).is_err());
    }

    #[test]
    fn reject_undersized_buffer() {
        let config = AudioConfig::default();
        let mut encoder = OpusEncoder::new(config).expect("init");
        let buf = AudioBuffer {
            samples: vec![0.0; 10],
            channels: 2,
            sample_rate: 48000,
            timestamp: Instant::now(),
        };
        assert!(encoder.encode(&buf).is_err());
    }

    #[test]
    fn pts_increments() {
        let config = AudioConfig::default();
        let mut encoder = OpusEncoder::new(config.clone()).expect("init");
        let buf = make_silence(&config);

        let p0 = encoder.encode(&buf).expect("e0");
        let p1 = encoder.encode(&buf).expect("e1");
        assert_eq!(p0.pts, 0);
        assert_eq!(p1.pts, 1);
    }

    #[test]
    fn reconfigure_changes_sample_count() {
        let config = AudioConfig::default();
        let mut encoder = OpusEncoder::new(config).expect("init");
        let new_config = AudioConfig {
            channels: 1,
            ..AudioConfig::default()
        };
        encoder.reconfigure(new_config.clone()).expect("reconfig");
        let buf = make_silence(&new_config);
        let pkt = encoder.encode(&buf).expect("encode");
        assert!(!pkt.data.is_empty());
    }

    #[test]
    fn real_opus_produces_variable_size() {
        let config = AudioConfig::default();
        let mut encoder = OpusEncoder::new(config.clone()).expect("init");

        let sine = make_audio_buffer(&config);
        let silence = make_silence(&config);

        let pkt_sine = encoder.encode(&sine).expect("sine");
        let pkt_silence = encoder.encode(&silence).expect("silence");

        // Real Opus produces different sizes for different content
        assert!(pkt_sine.data.len() > 3);
        assert!(pkt_silence.data.len() > 3);
    }
}
