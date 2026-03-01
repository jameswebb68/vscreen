use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use vscreen_core::error::TransportError;
use vscreen_core::event::PeerInputEvent;
use vscreen_core::frame::EncodedPacket;
use vscreen_core::instance::PeerId;
use vscreen_core::frame::VideoCodec;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MIME_TYPE_H264, MIME_TYPE_OPUS, MIME_TYPE_VP9, MediaEngine};
use webrtc::api::{API, APIBuilder};
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;

use crate::webrtc_session::data_channel::DataChannelHandler;
use crate::webrtc_session::signaling::SignalingMessage;

/// Represents a single WebRTC peer connection with real media tracks.
pub struct PeerSession {
    peer_id: PeerId,
    state: PeerSessionState,
    pc: Arc<RTCPeerConnection>,
    video_track: Arc<TrackLocalStaticSample>,
    audio_track: Arc<TrackLocalStaticSample>,
    ice_buffer: Vec<SignalingMessage>,
    remote_description_set: bool,
    /// Outbound signaling messages (ICE candidates from server side).
    signaling_tx: mpsc::Sender<SignalingMessage>,
    input_tx: mpsc::Sender<PeerInputEvent>,
    /// Monotonic RTP timestamp counter for audio (48 kHz clock).
    audio_rtp_timestamp: u32,
}

impl std::fmt::Debug for PeerSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PeerSession")
            .field("peer_id", &self.peer_id)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerSessionState {
    New,
    Connecting,
    Connected,
    Disconnected,
    Failed,
}

fn build_api() -> Result<API, TransportError> {
    let mut media_engine = MediaEngine::default();
    media_engine.register_default_codecs().map_err(|e| {
        TransportError::WebRtc(format!("register codecs: {e}"))
    })?;

    let mut registry = Registry::new();
    registry = register_default_interceptors(registry, &mut media_engine).map_err(|e| {
        TransportError::WebRtc(format!("register interceptors: {e}"))
    })?;

    let api = APIBuilder::new()
        .with_media_engine(media_engine)
        .with_interceptor_registry(registry)
        .build();

    Ok(api)
}

impl PeerSession {
    /// Create a new peer session with real WebRTC peer connection and media tracks.
    ///
    /// Returns the session plus receivers for:
    /// - `signaling_rx`: outbound signaling messages (ICE candidates to send to client)
    /// - `input_rx`: input events from the DataChannel
    ///
    /// # Errors
    /// Returns `TransportError` if the WebRTC API or peer connection cannot be created.
    pub async fn new(
        peer_id: PeerId,
        input_tx: mpsc::Sender<PeerInputEvent>,
        video_codec: VideoCodec,
    ) -> Result<(Self, mpsc::Receiver<SignalingMessage>), TransportError> {
        let api = build_api()?;

        let rtc_config = RTCConfiguration {
            ice_servers: vec![RTCIceServer {
                urls: vec!["stun:stun.l.google.com:19302".to_owned()],
                ..Default::default()
            }],
            ..Default::default()
        };

        let pc = api.new_peer_connection(rtc_config).await.map_err(|e| {
            TransportError::WebRtc(format!("create peer connection: {e}"))
        })?;
        let pc = Arc::new(pc);

        let (video_mime, video_fmtp) = match video_codec {
            VideoCodec::H264 => (
                MIME_TYPE_H264.to_owned(),
                "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f".to_owned(),
            ),
            VideoCodec::Vp9 => (MIME_TYPE_VP9.to_owned(), String::new()),
        };

        let video_track = Arc::new(TrackLocalStaticSample::new(
            RTCRtpCodecCapability {
                mime_type: video_mime,
                clock_rate: 90000,
                channels: 0,
                sdp_fmtp_line: video_fmtp,
                rtcp_feedback: vec![],
            },
            "video".to_owned(),
            "vscreen".to_owned(),
        ));

        let audio_track = Arc::new(TrackLocalStaticSample::new(
            RTCRtpCodecCapability {
                mime_type: MIME_TYPE_OPUS.to_owned(),
                clock_rate: 48000,
                channels: 2,
                sdp_fmtp_line: "minptime=10;useinbandfec=1".to_owned(),
                rtcp_feedback: vec![],
            },
            "audio".to_owned(),
            "vscreen".to_owned(),
        ));

        pc.add_track(video_track.clone()).await.map_err(|e| {
            TransportError::WebRtc(format!("add video track: {e}"))
        })?;
        pc.add_track(audio_track.clone()).await.map_err(|e| {
            TransportError::WebRtc(format!("add audio track: {e}"))
        })?;

        let (signaling_tx, signaling_rx) = mpsc::channel(128);

        // Wire on_ice_candidate to forward ICE candidates to the signaling channel
        let sig_tx = signaling_tx.clone();
        pc.on_ice_candidate(Box::new(move |candidate| {
            let sig_tx = sig_tx.clone();
            Box::pin(async move {
                if let Some(c) = candidate {
                    let json = match c.to_json() {
                        Ok(j) => j,
                        Err(e) => {
                            warn!(?e, "failed to serialize ICE candidate");
                            return;
                        }
                    };
                    let msg = SignalingMessage::IceCandidate {
                        candidate: json.candidate,
                        sdp_m_line_index: json.sdp_mline_index,
                        sdp_mid: json.sdp_mid,
                    };
                    if sig_tx.try_send(msg).is_err() {
                        warn!("signaling channel full, dropping ICE candidate");
                    }
                }
            })
        }));

        // Wire on_peer_connection_state_change for logging
        let peer_id_clone = peer_id;
        pc.on_peer_connection_state_change(Box::new(move |state| {
            info!(%peer_id_clone, ?state, "peer connection state changed");
            Box::pin(async {})
        }));

        // Accept the "input" DataChannel created by the client (offerer)
        let input_tx_for_dc = input_tx.clone();
        let peer_id_for_dc = peer_id;
        pc.on_data_channel(Box::new(move |dc| {
            let input_tx = input_tx_for_dc.clone();
            let peer_id = peer_id_for_dc;
            Box::pin(async move {
                let label = dc.label().to_owned();
                info!(%peer_id, label, "data channel opened");

                let dc_handler = DataChannelHandler::with_sender(peer_id, input_tx);
                dc.on_message(Box::new(move |msg| {
                    if let Err(e) = dc_handler.handle_message(&msg.data) {
                        warn!(?e, "input data channel message error");
                    }
                    Box::pin(async {})
                }));
            })
        }));

        info!(%peer_id, "peer session created with WebRTC tracks");

        Ok((
            Self {
                peer_id,
                state: PeerSessionState::New,
                pc,
                video_track,
                audio_track,
                ice_buffer: Vec::new(),
                remote_description_set: false,
                signaling_tx,
                input_tx,
                audio_rtp_timestamp: 0,
            },
            signaling_rx,
        ))
    }

    /// Process an incoming signaling message from the peer.
    ///
    /// # Errors
    /// Returns `TransportError` if the message is invalid or processing fails.
    pub async fn handle_signaling(
        &mut self,
        msg: SignalingMessage,
    ) -> Result<Option<SignalingMessage>, TransportError> {
        match msg {
            SignalingMessage::Offer { sdp } => self.handle_offer(&sdp).await,
            SignalingMessage::IceCandidate {
                candidate,
                sdp_m_line_index,
                sdp_mid,
            } => {
                self.handle_ice_candidate(candidate, sdp_m_line_index, sdp_mid).await?;
                Ok(None)
            }
            SignalingMessage::IceComplete => {
                debug!(peer = %self.peer_id, "ICE gathering complete");
                Ok(None)
            }
            _ => Err(TransportError::Signaling(format!(
                "unexpected message type from peer {}",
                self.peer_id
            ))),
        }
    }

    async fn handle_offer(&mut self, sdp: &str) -> Result<Option<SignalingMessage>, TransportError> {
        debug!(peer = %self.peer_id, sdp_len = sdp.len(), "received SDP offer");
        self.state = PeerSessionState::Connecting;

        let offer = RTCSessionDescription::offer(sdp.to_owned()).map_err(|e| {
            TransportError::Signaling(format!("parse offer: {e}"))
        })?;

        self.pc.set_remote_description(offer).await.map_err(|e| {
            TransportError::Signaling(format!("set remote description: {e}"))
        })?;

        self.remote_description_set = true;

        // Drain buffered ICE candidates
        let buffered = std::mem::take(&mut self.ice_buffer);
        if !buffered.is_empty() {
            debug!(
                peer = %self.peer_id,
                count = buffered.len(),
                "applying buffered ICE candidates"
            );
            for msg in buffered {
                if let SignalingMessage::IceCandidate {
                    candidate,
                    sdp_m_line_index,
                    sdp_mid,
                } = msg
                {
                    let init = RTCIceCandidateInit {
                        candidate,
                        sdp_mid,
                        sdp_mline_index: sdp_m_line_index,
                        username_fragment: None,
                    };
                    if let Err(e) = self.pc.add_ice_candidate(init).await {
                        warn!(peer = %self.peer_id, ?e, "failed to apply buffered ICE candidate");
                    }
                }
            }
        }

        let answer = self.pc.create_answer(None).await.map_err(|e| {
            TransportError::Signaling(format!("create answer: {e}"))
        })?;

        let answer_sdp = answer.sdp.clone();

        self.pc.set_local_description(answer).await.map_err(|e| {
            TransportError::Signaling(format!("set local description: {e}"))
        })?;

        Ok(Some(SignalingMessage::Answer { sdp: answer_sdp }))
    }

    async fn handle_ice_candidate(
        &mut self,
        candidate: String,
        sdp_m_line_index: Option<u16>,
        sdp_mid: Option<String>,
    ) -> Result<(), TransportError> {
        if !self.remote_description_set {
            debug!(
                peer = %self.peer_id,
                "buffering ICE candidate (remote description not set)"
            );
            self.ice_buffer.push(SignalingMessage::IceCandidate {
                candidate,
                sdp_m_line_index,
                sdp_mid,
            });
            return Ok(());
        }

        debug!(
            peer = %self.peer_id,
            candidate_len = candidate.len(),
            "adding ICE candidate"
        );

        let init = RTCIceCandidateInit {
            candidate,
            sdp_mid,
            sdp_mline_index: sdp_m_line_index,
            username_fragment: None,
        };

        self.pc.add_ice_candidate(init).await.map_err(|e| {
            TransportError::Ice(format!("add candidate: {e}"))
        })?;

        Ok(())
    }

    /// Write a video sample to the WebRTC video track.
    ///
    /// # Errors
    /// Returns `TransportError` if the write fails.
    pub async fn write_video(&self, packet: &EncodedPacket) -> Result<(), TransportError> {
        let duration_ms = packet.duration.unwrap_or(33);
        let sample = webrtc::media::Sample {
            data: packet.data.clone(),
            timestamp: SystemTime::now(),
            duration: Duration::from_millis(duration_ms),
            packet_timestamp: 0,
            prev_dropped_packets: 0,
            prev_padding_packets: 0,
        };
        self.video_track.write_sample(&sample).await.map_err(|e| {
            TransportError::WebRtc(format!("write video: {e}"))
        })?;
        Ok(())
    }

    /// Write an audio sample to the WebRTC audio track.
    ///
    /// # Errors
    /// Returns `TransportError` if the write fails.
    pub async fn write_audio(&mut self, packet: &EncodedPacket) -> Result<(), TransportError> {
        let duration_ms = packet.duration.unwrap_or(20);
        // Opus at 48 kHz: 20ms frame = 960 samples per channel
        let samples_per_frame = (48000_u32 * duration_ms as u32) / 1000;

        let sample = webrtc::media::Sample {
            data: packet.data.clone(),
            timestamp: SystemTime::now(),
            duration: Duration::from_millis(duration_ms),
            packet_timestamp: self.audio_rtp_timestamp,
            prev_dropped_packets: 0,
            prev_padding_packets: 0,
        };

        self.audio_rtp_timestamp = self.audio_rtp_timestamp.wrapping_add(samples_per_frame);

        self.audio_track.write_sample(&sample).await.map_err(|e| {
            TransportError::WebRtc(format!("write audio: {e}"))
        })?;
        Ok(())
    }

    /// Get the peer ID.
    #[must_use]
    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    /// Get the current session state.
    #[must_use]
    pub fn state(&self) -> &PeerSessionState {
        &self.state
    }

    /// Mark the session as connected.
    pub fn set_connected(&mut self) {
        self.state = PeerSessionState::Connected;
        info!(peer = %self.peer_id, "peer connected");
    }

    /// Mark the session as disconnected.
    pub fn set_disconnected(&mut self) {
        self.state = PeerSessionState::Disconnected;
        info!(peer = %self.peer_id, "peer disconnected");
    }

    /// Close the peer connection.
    pub async fn close(&self) {
        if let Err(e) = self.pc.close().await {
            error!(peer = %self.peer_id, ?e, "error closing peer connection");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_session_state_variants() {
        assert_eq!(PeerSessionState::New, PeerSessionState::New);
        assert_ne!(PeerSessionState::New, PeerSessionState::Connected);
    }
}
