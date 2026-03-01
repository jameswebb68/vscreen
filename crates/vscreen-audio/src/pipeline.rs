use tokio::sync::{broadcast, mpsc, watch};
use tracing::{debug, info, warn};
use vscreen_core::config::AudioConfig;
use vscreen_core::error::AudioError;
use vscreen_core::frame::{AudioBuffer, EncodedPacket};
use vscreen_core::traits::AudioEncoder;

use crate::capture::{spawn_capture_thread, AudioCaptureHandle};
use crate::encode::OpusEncoder;

/// Audio capture state reported via watch channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioCaptureState {
    Idle,
    Capturing,
    Lost,
    Stopped,
}

/// Orchestrates audio capture → encode → broadcast.
#[derive(Debug)]
pub struct AudioPipeline {
    config: AudioConfig,
    source_name: String,
    encoded_tx: broadcast::Sender<EncodedPacket>,
    state_tx: watch::Sender<AudioCaptureState>,
    state_rx: watch::Receiver<AudioCaptureState>,
    capture_handle: Option<AudioCaptureHandle>,
    encode_task: Option<tokio::task::JoinHandle<()>>,
}

impl AudioPipeline {
    /// Create and start the audio pipeline.
    ///
    /// Spawns a dedicated OS thread for PulseAudio capture and a tokio task
    /// for Opus encoding.
    ///
    /// # Errors
    /// Returns `AudioError` if the encoder or capture thread cannot be started.
    pub fn start(
        source_name: &str,
        config: AudioConfig,
    ) -> Result<Self, AudioError> {
        let (encoded_tx, _) = broadcast::channel(5);
        let (state_tx, state_rx) = watch::channel(AudioCaptureState::Idle);

        let (capture_tx, capture_rx) = mpsc::channel(5);

        let capture_handle = spawn_capture_thread(source_name, &config, capture_tx)?;
        let _ = state_tx.send(AudioCaptureState::Capturing);

        let encoder = OpusEncoder::new(config.clone())?;
        let encoded_tx_clone = encoded_tx.clone();
        let state_tx_clone = state_tx.clone();

        let encode_task = tokio::spawn(Self::encode_loop(
            capture_rx,
            encoder,
            encoded_tx_clone,
            state_tx_clone,
        ));

        info!(source = source_name, "audio pipeline started");

        Ok(Self {
            config,
            source_name: source_name.to_owned(),
            encoded_tx,
            state_tx,
            state_rx,
            capture_handle: Some(capture_handle),
            encode_task: Some(encode_task),
        })
    }

    async fn encode_loop(
        mut capture_rx: mpsc::Receiver<AudioBuffer>,
        mut encoder: OpusEncoder,
        encoded_tx: broadcast::Sender<EncodedPacket>,
        state_tx: watch::Sender<AudioCaptureState>,
    ) {
        while let Some(buffer) = capture_rx.recv().await {
            match encoder.encode(&buffer) {
                Ok(packet) => {
                    match encoded_tx.send(packet) {
                        Ok(n) => {
                            debug!(receivers = n, "broadcast encoded audio");
                        }
                        Err(_) => {
                            debug!("no active audio receivers");
                        }
                    }
                }
                Err(e) => {
                    warn!(?e, "audio encode failed, skipping frame");
                }
            }
        }

        debug!("audio encode loop ended (capture channel closed)");
        let _ = state_tx.send(AudioCaptureState::Stopped);
    }

    /// Subscribe to encoded audio packets.
    #[must_use]
    pub fn encoded_packets(&self) -> broadcast::Receiver<EncodedPacket> {
        self.encoded_tx.subscribe()
    }

    /// Get the current capture state.
    #[must_use]
    pub fn state(&self) -> watch::Receiver<AudioCaptureState> {
        self.state_rx.clone()
    }

    /// Stop the pipeline (capture thread + encode task).
    pub async fn stop(&mut self) {
        if let Some(handle) = self.capture_handle.take() {
            handle.stop();
            // Don't join on async - just let it finish
        }

        if let Some(task) = self.encode_task.take() {
            task.abort();
        }

        let _ = self.state_tx.send(AudioCaptureState::Stopped);
        info!(source = %self.source_name, "audio pipeline stopped");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pipeline_starts_and_produces_packets() {
        let config = AudioConfig {
            frame_duration_ms: 10,
            ..AudioConfig::default()
        };
        let mut pipeline =
            AudioPipeline::start("test-source", config).expect("start pipeline");

        let mut rx = pipeline.encoded_packets();
        let packet = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("timeout")
            .expect("packet");

        assert!(!packet.data.is_empty());

        pipeline.stop().await;
        assert_eq!(*pipeline.state_rx.borrow(), AudioCaptureState::Stopped);
    }

    #[tokio::test]
    async fn pipeline_state_transitions() {
        let config = AudioConfig {
            frame_duration_ms: 10,
            ..AudioConfig::default()
        };
        let mut pipeline =
            AudioPipeline::start("test-source", config).expect("start pipeline");

        assert_eq!(*pipeline.state_rx.borrow(), AudioCaptureState::Capturing);

        pipeline.stop().await;
        assert_eq!(*pipeline.state_rx.borrow(), AudioCaptureState::Stopped);
    }
}
