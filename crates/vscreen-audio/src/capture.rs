use std::time::Instant;

use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use vscreen_core::config::AudioConfig;
use vscreen_core::error::AudioError;
use vscreen_core::frame::AudioBuffer;

/// Handle to a running audio capture thread.
#[derive(Debug)]
pub struct AudioCaptureHandle {
    thread: Option<std::thread::JoinHandle<()>>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl AudioCaptureHandle {
    /// Signal the capture thread to stop.
    pub fn stop(&self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Wait for the capture thread to finish.
    ///
    /// # Errors
    /// Returns `AudioError` if the thread panicked.
    pub fn join(mut self) -> Result<(), AudioError> {
        if let Some(handle) = self.thread.take() {
            handle
                .join()
                .map_err(|_| AudioError::CaptureFailed("capture thread panicked".into()))?;
        }
        Ok(())
    }
}

impl Drop for AudioCaptureHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Spawn a dedicated OS thread for audio capture.
///
/// When the `pulse-audio` feature is enabled, reads from a real PulseAudio
/// monitor source. Otherwise, generates silence frames for testing.
///
/// # Errors
/// Returns `AudioError` if the thread cannot be spawned.
pub fn spawn_capture_thread(
    source_name: &str,
    config: &AudioConfig,
    tx: mpsc::Sender<AudioBuffer>,
) -> Result<AudioCaptureHandle, AudioError> {
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cancel_clone = cancel.clone();
    let source = source_name.to_owned();
    let sample_rate = config.sample_rate;
    let channels = config.channels;
    let frame_duration_ms = config.frame_duration_ms;

    let thread = std::thread::Builder::new()
        .name(format!("audio-{source}"))
        .spawn(move || {
            info!(source = %source, "audio capture thread started");
            capture_loop(&source, sample_rate, channels, frame_duration_ms, tx, cancel_clone);
            info!(source = %source, "audio capture thread stopped");
        })
        .map_err(|e| AudioError::CaptureFailed(format!("thread spawn: {e}")))?;

    Ok(AudioCaptureHandle {
        thread: Some(thread),
        cancel,
    })
}

#[cfg(feature = "pulse-audio")]
fn capture_loop(
    source: &str,
    sample_rate: u32,
    channels: u16,
    frame_duration_ms: u32,
    tx: mpsc::Sender<AudioBuffer>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    use libpulse_binding::def::BufferAttr;
    use libpulse_binding::sample::{Format, Spec};
    use libpulse_binding::stream::Direction;
    use libpulse_simple_binding::Simple;

    let samples_per_frame =
        (sample_rate as usize * frame_duration_ms as usize / 1000) * channels as usize;
    let byte_buf_len = samples_per_frame * std::mem::size_of::<f32>();

    let spec = Spec {
        format: Format::F32le,
        channels: channels as u8,
        rate: sample_rate,
    };

    // Set fragsize to one frame so PulseAudio wakes us once per frame
    // instead of buffering multiple frames and delivering them in bursts.
    let buf_attr = BufferAttr {
        maxlength: u32::MAX,
        tlength: u32::MAX,
        prebuf: u32::MAX,
        minreq: u32::MAX,
        fragsize: byte_buf_len as u32,
    };

    debug!(source, sample_rate, channels, samples_per_frame, fragsize = byte_buf_len, "opening PulseAudio source");

    let pa = match Simple::new(
        None,
        "vscreen",
        Direction::Record,
        Some(source),
        "audio capture",
        &spec,
        None,
        Some(&buf_attr),
    ) {
        Ok(s) => s,
        Err(e) => {
            warn!(source, %e, "failed to open PulseAudio source, falling back to silence");
            silence_loop(sample_rate, channels, frame_duration_ms, tx, cancel);
            return;
        }
    };

    info!(source, "PulseAudio capture connected");

    let mut byte_buf = vec![0u8; byte_buf_len];

    loop {
        if cancel.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }

        match pa.read(&mut byte_buf) {
            Ok(()) => {
                let samples: Vec<f32> = byte_buf
                    .chunks_exact(4)
                    .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                    .collect();

                let buffer = AudioBuffer {
                    samples,
                    channels,
                    sample_rate,
                    timestamp: Instant::now(),
                };

                // Use blocking send on a dedicated OS thread — never drop audio frames.
                if tx.blocking_send(buffer).is_err() {
                    debug!("audio capture channel closed, stopping");
                    break;
                }
            }
            Err(e) => {
                warn!(%e, "PulseAudio read error, sending silence");
                let buffer = AudioBuffer {
                    samples: vec![0.0f32; samples_per_frame],
                    channels,
                    sample_rate,
                    timestamp: Instant::now(),
                };
                if tx.blocking_send(buffer).is_err() {
                    break;
                }
            }
        }
    }
}

#[cfg(not(feature = "pulse-audio"))]
fn capture_loop(
    _source: &str,
    sample_rate: u32,
    channels: u16,
    frame_duration_ms: u32,
    tx: mpsc::Sender<AudioBuffer>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    debug!("pulse-audio feature disabled, using silence generator");
    silence_loop(sample_rate, channels, frame_duration_ms, tx, cancel);
}

fn silence_loop(
    sample_rate: u32,
    channels: u16,
    frame_duration_ms: u32,
    tx: mpsc::Sender<AudioBuffer>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let samples_per_frame =
        (sample_rate as usize * frame_duration_ms as usize / 1000) * channels as usize;
    let sleep_duration = std::time::Duration::from_millis(u64::from(frame_duration_ms));

    loop {
        if cancel.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }

        let buffer = AudioBuffer {
            samples: vec![0.0f32; samples_per_frame],
            channels,
            sample_rate,
            timestamp: Instant::now(),
        };

        match tx.try_send(buffer) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                warn!("audio capture channel full, dropping frame");
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                debug!("audio capture channel closed, stopping");
                break;
            }
        }

        std::thread::sleep(sleep_duration);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn capture_produces_buffers() {
        let config = AudioConfig {
            frame_duration_ms: 10,
            ..AudioConfig::default()
        };
        let (tx, mut rx) = mpsc::channel(5);

        let handle = spawn_capture_thread("test-source", &config, tx).expect("spawn");

        let buf = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .expect("timeout")
            .expect("buffer");

        assert_eq!(buf.channels, config.channels);
        assert_eq!(buf.sample_rate, config.sample_rate);
        assert!(!buf.samples.is_empty());

        handle.stop();
    }

    #[tokio::test]
    async fn capture_stops_on_cancel() {
        let config = AudioConfig {
            frame_duration_ms: 10,
            ..AudioConfig::default()
        };
        let (tx, _rx) = mpsc::channel(5);

        let handle = spawn_capture_thread("test-source", &config, tx).expect("spawn");
        handle.stop();
    }

    #[tokio::test]
    async fn capture_handles_full_channel() {
        let config = AudioConfig {
            frame_duration_ms: 5,
            ..AudioConfig::default()
        };
        let (tx, _rx) = mpsc::channel(1);

        let handle = spawn_capture_thread("test-source", &config, tx).expect("spawn");

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        handle.stop();
    }
}
