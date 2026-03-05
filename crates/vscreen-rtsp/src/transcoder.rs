use bytes::Bytes;
use tracing::{debug, trace};
use vscreen_core::error::AudioError;
use vscreen_core::frame::EncodedPacket;

use crate::quality::QualityTier;

const MAX_OPUS_FRAME_SIZE: usize = 4000;
const MASTER_SAMPLE_RATE: u32 = 48000;
const MASTER_CHANNELS: u16 = 2;
const MASTER_FRAME_DURATION_MS: u32 = 20;

fn channels_from_count(ch: u16) -> Result<audiopus::Channels, AudioError> {
    match ch {
        1 => Ok(audiopus::Channels::Mono),
        2 => Ok(audiopus::Channels::Stereo),
        _ => Err(AudioError::EncodeFailed(format!(
            "unsupported channel count: {ch}"
        ))),
    }
}

/// Opus transcoder that decodes master-quality packets and re-encodes
/// at a different bitrate/channel configuration.
pub struct OpusTranscoder {
    decoder: audiopus::coder::Decoder,
    encoder: audiopus::coder::Encoder,
    tier: QualityTier,
    pcm_buffer: Vec<f32>,
    downmix_buffer: Vec<f32>,
    output_buffer: Vec<u8>,
    frame_count: u64,
}

impl std::fmt::Debug for OpusTranscoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpusTranscoder")
            .field("tier", &self.tier)
            .field("frame_count", &self.frame_count)
            .finish_non_exhaustive()
    }
}

impl OpusTranscoder {
    /// Create a new transcoder for the given quality tier.
    ///
    /// The decoder is configured for the master stream (48kHz stereo).
    /// The encoder is configured for the target tier's bitrate and channels.
    ///
    /// # Errors
    /// Returns `AudioError` if libopus initialization fails.
    pub fn new(tier: QualityTier) -> Result<Self, AudioError> {
        let decoder = audiopus::coder::Decoder::new(
            audiopus::SampleRate::Hz48000,
            audiopus::Channels::Stereo,
        )
        .map_err(|e| AudioError::EncodeFailed(format!("opus decoder init: {e}")))?;

        let target_channels = channels_from_count(tier.channels())?;
        let mut encoder =
            audiopus::coder::Encoder::new(
                audiopus::SampleRate::Hz48000,
                target_channels,
                audiopus::Application::Audio,
            )
            .map_err(|e| AudioError::EncodeFailed(format!("opus encoder init: {e}")))?;

        #[allow(clippy::cast_possible_wrap)]
        let bitrate_bps = tier.bitrate_bps() as i32;
        encoder
            .set_bitrate(audiopus::Bitrate::BitsPerSecond(bitrate_bps))
            .map_err(|e| AudioError::EncodeFailed(format!("set bitrate: {e}")))?;

        let frame_samples =
            (MASTER_SAMPLE_RATE as usize * MASTER_FRAME_DURATION_MS as usize) / 1000;
        let pcm_size = frame_samples * MASTER_CHANNELS as usize;
        let downmix_size = frame_samples * tier.channels() as usize;

        debug!(
            tier = %tier,
            target_bitrate_kbps = tier.bitrate_kbps(),
            target_channels = tier.channels(),
            "Opus transcoder initialized"
        );

        Ok(Self {
            decoder,
            encoder,
            tier,
            pcm_buffer: vec![0.0f32; pcm_size],
            downmix_buffer: vec![0.0f32; downmix_size],
            output_buffer: vec![0u8; MAX_OPUS_FRAME_SIZE],
            frame_count: 0,
        })
    }

    /// The quality tier this transcoder targets.
    #[must_use]
    pub fn tier(&self) -> QualityTier {
        self.tier
    }

    /// Transcode a master-quality encoded packet to the target tier.
    ///
    /// Steps: decode Opus → PCM f32 → optional channel downmix → re-encode Opus
    ///
    /// # Errors
    /// Returns `AudioError` if decode or re-encode fails.
    pub fn transcode(&mut self, packet: &EncodedPacket) -> Result<EncodedPacket, AudioError> {
        // Decode master Opus → interleaved PCM f32 (stereo)
        let opus_packet = audiopus::packet::Packet::try_from(packet.data.as_ref())
            .map_err(|e| AudioError::EncodeFailed(format!("opus packet: {e}")))?;
        let mut_signals = audiopus::MutSignals::try_from(&mut self.pcm_buffer)
            .map_err(|e| AudioError::EncodeFailed(format!("mut signals: {e}")))?;
        let decoded_samples = self
            .decoder
            .decode_float(Some(opus_packet), mut_signals, false)
            .map_err(|e| AudioError::EncodeFailed(format!("opus decode: {e}")))?;

        let total_pcm = decoded_samples * MASTER_CHANNELS as usize;

        // Channel conversion if needed
        let encode_input = if self.tier.channels() == 1 && MASTER_CHANNELS == 2 {
            // Stereo → mono: average L+R
            let mono_samples = decoded_samples;
            for i in 0..mono_samples {
                let l = self.pcm_buffer[i * 2];
                let r = self.pcm_buffer[i * 2 + 1];
                self.downmix_buffer[i] = (l + r) * 0.5;
            }
            &self.downmix_buffer[..mono_samples]
        } else {
            &self.pcm_buffer[..total_pcm]
        };

        // Re-encode at target bitrate
        let encoded_len = self
            .encoder
            .encode_float(encode_input, &mut self.output_buffer)
            .map_err(|e| AudioError::EncodeFailed(format!("opus re-encode: {e}")))?;

        let result = EncodedPacket {
            data: Bytes::copy_from_slice(&self.output_buffer[..encoded_len]),
            is_keyframe: packet.is_keyframe,
            pts: self.frame_count,
            duration: packet.duration,
            codec: None,
        };

        self.frame_count += 1;

        trace!(
            tier = %self.tier,
            input_bytes = packet.data.len(),
            output_bytes = result.data.len(),
            pts = result.pts,
            "transcoded Opus frame"
        );

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_master_packet() -> EncodedPacket {
        use vscreen_core::config::AudioConfig;

        let config = AudioConfig {
            sample_rate: 48000,
            channels: 2,
            bitrate_kbps: 256,
            frame_duration_ms: 20,
        };

        let samples_per_frame =
            (config.sample_rate as usize * config.frame_duration_ms as usize / 1000)
                * config.channels as usize;

        let samples: Vec<f32> = (0..samples_per_frame)
            .map(|i| {
                let t = i as f32 / config.sample_rate as f32;
                (t * 440.0 * 2.0 * std::f32::consts::PI).sin() * 0.5
            })
            .collect();

        let mut encoder = audiopus::coder::Encoder::new(
            audiopus::SampleRate::Hz48000,
            audiopus::Channels::Stereo,
            audiopus::Application::Audio,
        )
        .expect("encoder");
        encoder
            .set_bitrate(audiopus::Bitrate::BitsPerSecond(256_000))
            .expect("bitrate");

        let mut output = vec![0u8; MAX_OPUS_FRAME_SIZE];
        let len = encoder.encode_float(&samples, &mut output).expect("encode");

        EncodedPacket {
            data: Bytes::copy_from_slice(&output[..len]),
            is_keyframe: true,
            pts: 0,
            duration: Some(20),
            codec: None,
        }
    }

    #[test]
    fn transcode_to_low() {
        let master_pkt = make_master_packet();
        let mut tc = OpusTranscoder::new(QualityTier::Low).expect("init");
        let result = tc.transcode(&master_pkt).expect("transcode");
        assert!(!result.data.is_empty());
        // Lower bitrate should produce smaller packets
        assert!(result.data.len() <= master_pkt.data.len());
    }

    #[test]
    fn transcode_to_medium() {
        let master_pkt = make_master_packet();
        let mut tc = OpusTranscoder::new(QualityTier::Medium).expect("init");
        let result = tc.transcode(&master_pkt).expect("transcode");
        assert!(!result.data.is_empty());
    }

    #[test]
    fn transcode_to_standard() {
        let master_pkt = make_master_packet();
        let mut tc = OpusTranscoder::new(QualityTier::Standard).expect("init");
        let result = tc.transcode(&master_pkt).expect("transcode");
        assert!(!result.data.is_empty());
    }

    #[test]
    fn transcode_preserves_keyframe_flag() {
        let master_pkt = make_master_packet();
        let mut tc = OpusTranscoder::new(QualityTier::Medium).expect("init");
        let result = tc.transcode(&master_pkt).expect("transcode");
        assert!(result.is_keyframe);
    }

    #[test]
    fn frame_count_increments() {
        let master_pkt = make_master_packet();
        let mut tc = OpusTranscoder::new(QualityTier::Standard).expect("init");
        let r0 = tc.transcode(&master_pkt).expect("tc0");
        let r1 = tc.transcode(&master_pkt).expect("tc1");
        assert_eq!(r0.pts, 0);
        assert_eq!(r1.pts, 1);
    }
}
