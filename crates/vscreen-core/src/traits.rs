use crate::{
    error::{AudioError, TransportError, VideoError},
    frame::{AudioBuffer, EncodedPacket, I420Buffer, RawFrame},
    instance::PeerId,
};

// ---------------------------------------------------------------------------
// Frame / video traits
// ---------------------------------------------------------------------------

/// Produces raw screen frames (JPEG bytes from CDP or from a test fixture).
pub trait FrameSource: Send + Sync + 'static {
    /// Receive the next frame. Returns `None` when the source is exhausted.
    fn next_frame(
        &mut self,
    ) -> impl std::future::Future<Output = Result<Option<RawFrame>, VideoError>> + Send;

    /// Start the frame source.
    fn start(
        &mut self,
    ) -> impl std::future::Future<Output = Result<(), VideoError>> + Send;

    /// Stop the frame source.
    fn stop(
        &mut self,
    ) -> impl std::future::Future<Output = Result<(), VideoError>> + Send;
}

/// Encodes I420 frames into compressed packets.
pub trait VideoEncoder: Send + 'static {
    /// Encode a single I420 frame.
    ///
    /// # Errors
    /// Returns `VideoError` if encoding fails.
    fn encode(&mut self, frame: &I420Buffer) -> Result<EncodedPacket, VideoError>;

    /// Request the next encoded frame to be a keyframe.
    fn request_keyframe(&mut self);

    /// Apply new encoder configuration (between frames only).
    ///
    /// # Errors
    /// Returns `VideoError` if reconfiguration fails.
    fn reconfigure(
        &mut self,
        config: crate::config::VideoConfig,
    ) -> Result<(), VideoError>;
}

// ---------------------------------------------------------------------------
// Audio traits
// ---------------------------------------------------------------------------

/// Produces raw audio sample buffers.
pub trait AudioSource: Send + Sync + 'static {
    /// Receive the next audio buffer. Returns `None` when the source is exhausted.
    fn next_buffer(
        &mut self,
    ) -> impl std::future::Future<Output = Result<Option<AudioBuffer>, AudioError>> + Send;

    /// Start the audio source.
    fn start(
        &mut self,
    ) -> impl std::future::Future<Output = Result<(), AudioError>> + Send;

    /// Stop the audio source.
    fn stop(
        &mut self,
    ) -> impl std::future::Future<Output = Result<(), AudioError>> + Send;
}

/// Encodes raw audio into compressed packets.
pub trait AudioEncoder: Send + 'static {
    /// Encode an audio buffer.
    ///
    /// # Errors
    /// Returns `AudioError` if encoding fails.
    fn encode(&mut self, samples: &AudioBuffer) -> Result<EncodedPacket, AudioError>;

    /// Apply new encoder configuration.
    ///
    /// # Errors
    /// Returns `AudioError` if reconfiguration fails.
    fn reconfigure(
        &mut self,
        config: crate::config::AudioConfig,
    ) -> Result<(), AudioError>;
}

// ---------------------------------------------------------------------------
// Transport trait
// ---------------------------------------------------------------------------

/// Delivers encoded media to a peer.
pub trait MediaSink: Send + Sync + 'static {
    /// Send a video packet to the peer.
    fn send_video(
        &self,
        packet: &EncodedPacket,
    ) -> impl std::future::Future<Output = Result<(), TransportError>> + Send;

    /// Send an audio packet to the peer.
    fn send_audio(
        &self,
        packet: &EncodedPacket,
    ) -> impl std::future::Future<Output = Result<(), TransportError>> + Send;

    /// The peer ID of this sink.
    fn peer_id(&self) -> &PeerId;

    /// Whether the peer is still connected.
    fn is_connected(&self) -> bool;
}

// ---------------------------------------------------------------------------
// Test doubles
// ---------------------------------------------------------------------------

#[cfg(test)]
pub mod mocks {
    use std::{
        sync::{Arc, Mutex},
        time::Instant,
    };

    use bytes::Bytes;

    use super::*;

    /// Replays a fixed sequence of JPEG frames.
    #[derive(Debug)]
    pub struct FixtureFrameSource {
        frames: Vec<Bytes>,
        index: usize,
    }

    impl FixtureFrameSource {
        pub fn new(frames: Vec<Vec<u8>>) -> Self {
            Self {
                frames: frames.into_iter().map(Bytes::from).collect(),
                index: 0,
            }
        }
    }

    impl FrameSource for FixtureFrameSource {
        async fn next_frame(&mut self) -> Result<Option<RawFrame>, VideoError> {
            if self.index >= self.frames.len() {
                return Ok(None);
            }
            let data = self.frames[self.index].clone();
            self.index += 1;
            Ok(Some(RawFrame {
                data,
                timestamp: Instant::now(),
                session_id: 0,
            }))
        }

        async fn start(&mut self) -> Result<(), VideoError> {
            self.index = 0;
            Ok(())
        }

        async fn stop(&mut self) -> Result<(), VideoError> {
            Ok(())
        }
    }

    /// Records all sent packets for assertion.
    #[derive(Debug, Clone)]
    pub struct RecordingSink {
        pub video_packets: Arc<Mutex<Vec<EncodedPacket>>>,
        pub audio_packets: Arc<Mutex<Vec<EncodedPacket>>>,
        pub peer: PeerId,
        pub connected: Arc<std::sync::atomic::AtomicBool>,
    }

    impl RecordingSink {
        pub fn new() -> Self {
            Self {
                video_packets: Arc::new(Mutex::new(Vec::new())),
                audio_packets: Arc::new(Mutex::new(Vec::new())),
                peer: PeerId::new(),
                connected: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            }
        }
    }

    impl MediaSink for RecordingSink {
        async fn send_video(&self, packet: &EncodedPacket) -> Result<(), TransportError> {
            self.video_packets
                .lock()
                .map_err(|_| TransportError::WebRtc("lock poisoned".into()))?
                .push(packet.clone());
            Ok(())
        }

        async fn send_audio(&self, packet: &EncodedPacket) -> Result<(), TransportError> {
            self.audio_packets
                .lock()
                .map_err(|_| TransportError::WebRtc("lock poisoned".into()))?
                .push(packet.clone());
            Ok(())
        }

        fn peer_id(&self) -> &PeerId {
            &self.peer
        }

        fn is_connected(&self) -> bool {
            self.connected
                .load(std::sync::atomic::Ordering::Relaxed)
        }
    }

    #[tokio::test]
    async fn fixture_frame_source_replays() {
        let mut src = FixtureFrameSource::new(vec![vec![1, 2, 3], vec![4, 5, 6]]);
        src.start().await.expect("start");
        let f1 = src.next_frame().await.expect("frame 1");
        assert!(f1.is_some());
        let f2 = src.next_frame().await.expect("frame 2");
        assert!(f2.is_some());
        let f3 = src.next_frame().await.expect("frame 3 (none)");
        assert!(f3.is_none());
    }

    #[tokio::test]
    async fn recording_sink_captures() {
        let sink = RecordingSink::new();
        let pkt = EncodedPacket::new(vec![1, 2, 3], true, 0);
        sink.send_video(&pkt).await.expect("send video");
        sink.send_audio(&pkt).await.expect("send audio");

        assert_eq!(sink.video_packets.lock().expect("lock").len(), 1);
        assert_eq!(sink.audio_packets.lock().expect("lock").len(), 1);
        assert!(sink.is_connected());
    }
}
