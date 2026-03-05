use std::net::SocketAddr;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use serde::Serialize;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use uuid::Uuid;
use vscreen_core::instance::InstanceId;

use crate::health::{AggregatedHealth, HealthState, StreamHealth};
use crate::parser::SessionId;
use crate::quality::QualityTier;

const DEFAULT_TIMEOUT_SECS: u64 = 120;

// ───────────────────────── Media Types ─────────────────────────────

/// Type of media carried by a track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaType {
    Video,
    Audio,
}

impl std::fmt::Display for MediaType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Video => write!(f, "video"),
            Self::Audio => write!(f, "audio"),
        }
    }
}

/// Which media tracks a session should include.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct MediaConfig {
    pub video: bool,
    pub audio: bool,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            video: true,
            audio: true,
        }
    }
}

impl MediaConfig {
    /// Audio-only configuration (legacy `/audio/` URL).
    #[must_use]
    pub fn audio_only() -> Self {
        Self {
            video: false,
            audio: true,
        }
    }
}

// ───────────────────────── Track Info ──────────────────────────────

/// Per-track transport and state within a session.
pub struct TrackInfo {
    pub track_id: u8,
    pub media_type: MediaType,
    pub client_rtp_port: u16,
    pub client_rtcp_port: u16,
    pub server_rtp_port: u16,
    pub server_rtcp_port: u16,
    /// TCP interleaved channel pair (rtp_channel, rtcp_channel). None = UDP.
    pub interleaved_channels: Option<(u8, u8)>,
    pub rtp_task: Option<JoinHandle<()>>,
    pub health: StreamHealth,
}

impl std::fmt::Debug for TrackInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrackInfo")
            .field("track_id", &self.track_id)
            .field("media_type", &self.media_type)
            .field("client_rtp_port", &self.client_rtp_port)
            .field("server_rtp_port", &self.server_rtp_port)
            .finish_non_exhaustive()
    }
}

impl TrackInfo {
    /// Serializable info for this track.
    #[must_use]
    pub fn info(&self) -> TrackInfoSerialized {
        TrackInfoSerialized {
            track_id: self.track_id,
            media_type: self.media_type,
            client_rtp_port: self.client_rtp_port,
            server_rtp_port: self.server_rtp_port,
            packets_sent: self.health.packets_sent,
            bytes_sent: self.health.bytes_sent,
            health_state: self.health.state,
            client_packet_loss: self.health.client_packet_loss,
            client_jitter_ms: self.health.client_jitter_ms,
        }
    }
}

/// Serializable per-track info.
#[derive(Debug, Clone, Serialize)]
pub struct TrackInfoSerialized {
    pub track_id: u8,
    pub media_type: MediaType,
    pub client_rtp_port: u16,
    pub server_rtp_port: u16,
    pub packets_sent: u64,
    pub bytes_sent: u64,
    pub health_state: HealthState,
    pub client_packet_loss: f64,
    pub client_jitter_ms: f64,
}

// ───────────────────────── Session State ──────────────────────────

/// RTSP session state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionState {
    /// SETUP completed, waiting for PLAY.
    Ready,
    /// Actively streaming.
    Playing,
    /// Temporarily paused.
    Paused,
}

impl std::fmt::Display for SessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ready => write!(f, "ready"),
            Self::Playing => write!(f, "playing"),
            Self::Paused => write!(f, "paused"),
        }
    }
}

// ───────────────────────── RTSP Session ──────────────────────────

/// A single RTSP session representing one client connection with multiple tracks.
pub struct RtspSession {
    pub id: SessionId,
    pub instance_id: InstanceId,
    pub state: SessionState,
    pub client_addr: SocketAddr,
    pub media_config: MediaConfig,
    pub quality: QualityTier,
    pub tracks: Vec<TrackInfo>,
    pub created_at: Instant,
    pub last_activity: Instant,
    pub cancel: CancellationToken,
}

impl std::fmt::Debug for RtspSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RtspSession")
            .field("id", &self.id)
            .field("instance_id", &self.instance_id)
            .field("state", &self.state)
            .field("quality", &self.quality)
            .field("media_config", &self.media_config)
            .field("client_addr", &self.client_addr)
            .field("tracks", &self.tracks)
            .finish_non_exhaustive()
    }
}

impl RtspSession {
    /// Touch the session to reset idle timeout.
    pub fn touch(&mut self) {
        self.last_activity = Instant::now();
    }

    /// Whether the session has exceeded its idle timeout.
    #[must_use]
    pub fn is_expired(&self, timeout: Duration) -> bool {
        self.last_activity.elapsed() > timeout
    }

    /// Add a UDP track to this session.
    pub fn add_track(
        &mut self,
        track_id: u8,
        media_type: MediaType,
        client_rtp_port: u16,
        client_rtcp_port: u16,
        server_rtp_port: u16,
        server_rtcp_port: u16,
    ) {
        self.tracks.push(TrackInfo {
            track_id,
            media_type,
            client_rtp_port,
            client_rtcp_port,
            server_rtp_port,
            server_rtcp_port,
            interleaved_channels: None,
            rtp_task: None,
            health: StreamHealth::default(),
        });
        self.touch();
    }

    /// Add a TCP interleaved track to this session.
    pub fn add_interleaved_track(
        &mut self,
        track_id: u8,
        media_type: MediaType,
        rtp_channel: u8,
        rtcp_channel: u8,
    ) {
        self.tracks.push(TrackInfo {
            track_id,
            media_type,
            client_rtp_port: 0,
            client_rtcp_port: 0,
            server_rtp_port: 0,
            server_rtcp_port: 0,
            interleaved_channels: Some((rtp_channel, rtcp_channel)),
            rtp_task: None,
            health: StreamHealth::default(),
        });
        self.touch();
    }

    /// Find a track by its ID.
    #[must_use]
    pub fn track(&self, track_id: u8) -> Option<&TrackInfo> {
        self.tracks.iter().find(|t| t.track_id == track_id)
    }

    /// Find a mutable track by its ID.
    pub fn track_mut(&mut self, track_id: u8) -> Option<&mut TrackInfo> {
        self.tracks.iter_mut().find(|t| t.track_id == track_id)
    }

    /// Find a track by media type.
    #[must_use]
    pub fn track_by_type(&self, media_type: MediaType) -> Option<&TrackInfo> {
        self.tracks.iter().find(|t| t.media_type == media_type)
    }

    /// Find a mutable track by media type.
    pub fn track_by_type_mut(&mut self, media_type: MediaType) -> Option<&mut TrackInfo> {
        self.tracks.iter_mut().find(|t| t.media_type == media_type)
    }

    /// Whether any track has a running RTP task.
    #[must_use]
    pub fn has_rtp_tasks(&self) -> bool {
        self.tracks.iter().any(|t| t.rtp_task.is_some())
    }

    /// Transition to Playing state.
    ///
    /// # Errors
    /// Returns a string error if the session is not in a valid state to play.
    pub fn play(&mut self) -> Result<(), String> {
        match self.state {
            SessionState::Ready | SessionState::Paused => {
                self.state = SessionState::Playing;
                self.touch();
                Ok(())
            }
            SessionState::Playing => Ok(()),
        }
    }

    /// Transition to Paused state.
    ///
    /// # Errors
    /// Returns a string error if the session is not playing.
    pub fn pause(&mut self) -> Result<(), String> {
        match self.state {
            SessionState::Playing => {
                self.state = SessionState::Paused;
                self.touch();
                Ok(())
            }
            SessionState::Paused => Ok(()),
            SessionState::Ready => Err("cannot pause: session not started".to_owned()),
        }
    }

    /// Tear down the session, cancelling any running tasks.
    pub fn teardown(&mut self) {
        self.cancel.cancel();
        for track in &mut self.tracks {
            if let Some(task) = track.rtp_task.take() {
                task.abort();
            }
        }
    }

    /// Serializable session info for API responses.
    #[must_use]
    pub fn info(&self) -> SessionInfo {
        let total_packets: u64 = self.tracks.iter().map(|t| t.health.packets_sent).sum();
        let total_bytes: u64 = self.tracks.iter().map(|t| t.health.bytes_sent).sum();

        // Aggregate health across tracks
        let worst_health = self
            .tracks
            .iter()
            .map(|t| t.health.state)
            .max_by_key(|s| match s {
                HealthState::Healthy => 0,
                HealthState::Degraded => 1,
                HealthState::Failed => 2,
            })
            .unwrap_or(HealthState::Healthy);

        let max_loss = self
            .tracks
            .iter()
            .map(|t| t.health.client_packet_loss)
            .fold(0.0_f64, f64::max);
        let max_jitter = self
            .tracks
            .iter()
            .map(|t| t.health.client_jitter_ms)
            .fold(0.0_f64, f64::max);

        SessionInfo {
            session_id: self.id.0.clone(),
            instance_id: self.instance_id.0.clone(),
            state: self.state,
            media_config: self.media_config,
            quality: self.quality,
            client_addr: self.client_addr.to_string(),
            tracks: self.tracks.iter().map(TrackInfo::info).collect(),
            packets_sent: total_packets,
            bytes_sent: total_bytes,
            health_state: worst_health,
            uptime_secs: self.created_at.elapsed().as_secs_f64(),
            client_packet_loss: max_loss,
            client_jitter_ms: max_jitter,
        }
    }
}

/// Serializable session information.
#[derive(Debug, Clone, Serialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub instance_id: String,
    pub state: SessionState,
    pub media_config: MediaConfig,
    pub quality: QualityTier,
    pub client_addr: String,
    pub tracks: Vec<TrackInfoSerialized>,
    pub packets_sent: u64,
    pub bytes_sent: u64,
    pub health_state: HealthState,
    pub uptime_secs: f64,
    pub client_packet_loss: f64,
    pub client_jitter_ms: f64,
}

// ─────────────────── Session Manager ─────────────────────────────

/// Manages all active RTSP sessions with timeout-based cleanup.
pub struct RtspSessionManager {
    sessions: DashMap<String, RtspSession>,
    timeout: Duration,
    cancel: CancellationToken,
}

impl std::fmt::Debug for RtspSessionManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RtspSessionManager")
            .field("session_count", &self.sessions.len())
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl RtspSessionManager {
    /// Create a new session manager.
    #[must_use]
    pub fn new(cancel: CancellationToken) -> Self {
        Self {
            sessions: DashMap::new(),
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
            cancel,
        }
    }

    /// Create with a custom timeout.
    #[must_use]
    pub fn with_timeout(cancel: CancellationToken, timeout: Duration) -> Self {
        Self {
            sessions: DashMap::new(),
            timeout,
            cancel,
        }
    }

    /// Create a new RTSP session (tracks are added later via SETUP).
    pub fn create_session(
        &self,
        instance_id: InstanceId,
        client_addr: SocketAddr,
        media_config: MediaConfig,
        quality: QualityTier,
    ) -> SessionId {
        let session_id = SessionId::from(Uuid::new_v4());
        let session = RtspSession {
            id: session_id.clone(),
            instance_id,
            state: SessionState::Ready,
            client_addr,
            media_config,
            quality,
            tracks: Vec::with_capacity(2),
            created_at: Instant::now(),
            last_activity: Instant::now(),
            cancel: self.cancel.child_token(),
        };

        info!(
            session_id = %session.id,
            instance_id = %session.instance_id,
            quality = %quality,
            ?media_config,
            "RTSP session created"
        );

        self.sessions.insert(session_id.0.clone(), session);
        session_id
    }

    /// Get a reference to a session.
    pub fn get(&self, session_id: &str) -> Option<dashmap::mapref::one::Ref<'_, String, RtspSession>> {
        self.sessions.get(session_id)
    }

    /// Get a mutable reference to a session.
    pub fn get_mut(&self, session_id: &str) -> Option<dashmap::mapref::one::RefMut<'_, String, RtspSession>> {
        self.sessions.get_mut(session_id)
    }

    /// Reset the idle timeout for a session (keepalive).
    pub fn touch(&self, session_id: &str) {
        if let Some(mut session) = self.sessions.get_mut(session_id) {
            session.touch();
        }
    }

    /// Remove and teardown a session.
    pub fn remove(&self, session_id: &str) -> Option<RtspSession> {
        if let Some((_, mut session)) = self.sessions.remove(session_id) {
            session.teardown();
            info!(session_id, "RTSP session removed");
            Some(session)
        } else {
            None
        }
    }

    /// List all sessions for a given instance.
    #[must_use]
    pub fn sessions_for_instance(&self, instance_id: &str) -> Vec<SessionInfo> {
        self.sessions
            .iter()
            .filter(|entry| entry.value().instance_id.0 == instance_id)
            .map(|entry| entry.value().info())
            .collect()
    }

    /// List all sessions across all instances.
    #[must_use]
    pub fn all_sessions(&self) -> Vec<SessionInfo> {
        self.sessions.iter().map(|entry| entry.value().info()).collect()
    }

    /// Total number of active sessions.
    #[must_use]
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Count sessions per instance.
    #[must_use]
    pub fn session_count_for_instance(&self, instance_id: &str) -> usize {
        self.sessions
            .iter()
            .filter(|entry| entry.value().instance_id.0 == instance_id)
            .count()
    }

    /// Session timeout duration.
    #[must_use]
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Timeout duration in seconds (for RTSP Session header).
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn timeout_secs(&self) -> u32 {
        self.timeout.as_secs() as u32
    }

    /// Compute aggregated health for a given instance.
    #[must_use]
    pub fn aggregated_health(&self, instance_id: &str) -> AggregatedHealth {
        let mut agg = AggregatedHealth {
            total_sessions: 0,
            healthy: 0,
            degraded: 0,
            failed: 0,
            total_packets_sent: 0,
            total_bytes_sent: 0,
        };

        for entry in &self.sessions {
            let session = entry.value();
            if session.instance_id.0 != instance_id {
                continue;
            }
            agg.total_sessions += 1;

            for track in &session.tracks {
                agg.total_packets_sent += track.health.packets_sent;
                agg.total_bytes_sent += track.health.bytes_sent;
                match track.health.state {
                    HealthState::Healthy => agg.healthy += 1,
                    HealthState::Degraded => agg.degraded += 1,
                    HealthState::Failed => agg.failed += 1,
                }
            }
        }

        agg
    }

    /// Run the timeout reaper loop. Call in a spawned task.
    pub async fn run_reaper(&self) {
        let interval = Duration::from_secs(10);

        loop {
            tokio::select! {
                () = self.cancel.cancelled() => {
                    debug!("RTSP session reaper shutting down");
                    break;
                }
                () = tokio::time::sleep(interval) => {
                    self.reap_expired();
                }
            }
        }

        self.teardown_all();
    }

    pub fn reap_expired(&self) {
        let expired: Vec<String> = self
            .sessions
            .iter()
            .filter(|entry| entry.value().is_expired(self.timeout))
            .map(|entry| entry.key().clone())
            .collect();

        for session_id in expired {
            warn!(session_id, "RTSP session expired, tearing down");
            self.remove(&session_id);
            metrics::counter!("vscreen_rtsp_sessions_expired_total").increment(1);
        }
    }

    /// Create a child cancellation token.
    #[must_use]
    pub fn cancel_token_child(&self) -> CancellationToken {
        self.cancel.child_token()
    }

    /// Teardown all sessions (for shutdown).
    pub fn teardown_all(&self) {
        let keys: Vec<String> = self.sessions.iter().map(|e| e.key().clone()).collect();
        for key in keys {
            self.remove(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddrV4};

    use super::*;

    fn test_addr() -> SocketAddr {
        SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 5000))
    }

    fn create_manager() -> RtspSessionManager {
        let cancel = CancellationToken::new();
        RtspSessionManager::with_timeout(cancel, Duration::from_secs(60))
    }

    #[test]
    fn create_and_get_session() {
        let mgr = create_manager();
        let sid = mgr.create_session(
            InstanceId::from("dev"),
            test_addr(),
            MediaConfig::default(),
            QualityTier::Standard,
        );

        assert!(mgr.get(&sid.0).is_some());
        assert_eq!(mgr.session_count(), 1);
    }

    #[test]
    fn add_tracks_to_session() {
        let mgr = create_manager();
        let sid = mgr.create_session(
            InstanceId::from("dev"),
            test_addr(),
            MediaConfig::default(),
            QualityTier::Standard,
        );

        {
            let mut session = mgr.get_mut(&sid.0).expect("session");
            assert_eq!(session.tracks.len(), 0);

            session.add_track(0, MediaType::Video, 5000, 5001, 6000, 6001);
            session.add_track(1, MediaType::Audio, 5002, 5003, 6002, 6003);
            assert_eq!(session.tracks.len(), 2);
        }

        let session = mgr.get(&sid.0).expect("session");
        assert!(session.track(0).is_some());
        assert!(session.track(1).is_some());
        assert_eq!(session.track(0).unwrap().media_type, MediaType::Video);
        assert_eq!(session.track(1).unwrap().media_type, MediaType::Audio);
    }

    #[test]
    fn remove_session() {
        let mgr = create_manager();
        let sid = mgr.create_session(
            InstanceId::from("dev"),
            test_addr(),
            MediaConfig::default(),
            QualityTier::High,
        );

        let removed = mgr.remove(&sid.0);
        assert!(removed.is_some());
        assert_eq!(mgr.session_count(), 0);
    }

    #[test]
    fn session_state_transitions() {
        let mgr = create_manager();
        let sid = mgr.create_session(
            InstanceId::from("dev"),
            test_addr(),
            MediaConfig::default(),
            QualityTier::Standard,
        );

        {
            let mut session = mgr.get_mut(&sid.0).expect("session");
            assert_eq!(session.state, SessionState::Ready);
            session.play().expect("play");
            assert_eq!(session.state, SessionState::Playing);
            session.pause().expect("pause");
            assert_eq!(session.state, SessionState::Paused);
            session.play().expect("resume");
            assert_eq!(session.state, SessionState::Playing);
        }
    }

    #[test]
    fn cannot_pause_when_ready() {
        let mgr = create_manager();
        let sid = mgr.create_session(
            InstanceId::from("dev"),
            test_addr(),
            MediaConfig::default(),
            QualityTier::Low,
        );

        let mut session = mgr.get_mut(&sid.0).expect("session");
        assert!(session.pause().is_err());
    }

    #[test]
    fn sessions_for_instance() {
        let mgr = create_manager();
        mgr.create_session(
            InstanceId::from("dev"),
            test_addr(),
            MediaConfig::default(),
            QualityTier::Standard,
        );
        mgr.create_session(
            InstanceId::from("dev"),
            test_addr(),
            MediaConfig::default(),
            QualityTier::High,
        );
        mgr.create_session(
            InstanceId::from("other"),
            test_addr(),
            MediaConfig::audio_only(),
            QualityTier::Low,
        );

        assert_eq!(mgr.sessions_for_instance("dev").len(), 2);
        assert_eq!(mgr.sessions_for_instance("other").len(), 1);
        assert_eq!(mgr.session_count_for_instance("dev"), 2);
    }

    #[test]
    fn session_expiry_detection() {
        let mgr = RtspSessionManager::with_timeout(
            CancellationToken::new(),
            Duration::from_millis(1),
        );
        let sid = mgr.create_session(
            InstanceId::from("dev"),
            test_addr(),
            MediaConfig::default(),
            QualityTier::Standard,
        );

        std::thread::sleep(Duration::from_millis(10));

        let session = mgr.get(&sid.0).expect("session");
        assert!(session.is_expired(Duration::from_millis(1)));
    }

    #[test]
    fn teardown_all_clears_sessions() {
        let mgr = create_manager();
        mgr.create_session(
            InstanceId::from("dev"),
            test_addr(),
            MediaConfig::default(),
            QualityTier::Standard,
        );
        mgr.create_session(
            InstanceId::from("dev2"),
            test_addr(),
            MediaConfig::audio_only(),
            QualityTier::High,
        );

        mgr.teardown_all();
        assert_eq!(mgr.session_count(), 0);
    }

    #[test]
    fn session_info_with_tracks() {
        let mgr = create_manager();
        let sid = mgr.create_session(
            InstanceId::from("test-instance"),
            test_addr(),
            MediaConfig::default(),
            QualityTier::Medium,
        );

        {
            let mut session = mgr.get_mut(&sid.0).expect("session");
            session.add_track(0, MediaType::Video, 5000, 5001, 6000, 6001);
            session.add_track(1, MediaType::Audio, 5002, 5003, 6002, 6003);
        }

        let session = mgr.get(&sid.0).expect("session");
        let info = session.info();
        assert_eq!(info.instance_id, "test-instance");
        assert_eq!(info.quality, QualityTier::Medium);
        assert_eq!(info.state, SessionState::Ready);
        assert_eq!(info.tracks.len(), 2);
        assert_eq!(info.tracks[0].media_type, MediaType::Video);
        assert_eq!(info.tracks[1].media_type, MediaType::Audio);
        assert!(info.media_config.video);
        assert!(info.media_config.audio);
    }

    #[test]
    fn track_lookup_by_type() {
        let mgr = create_manager();
        let sid = mgr.create_session(
            InstanceId::from("dev"),
            test_addr(),
            MediaConfig::default(),
            QualityTier::Standard,
        );

        {
            let mut session = mgr.get_mut(&sid.0).expect("session");
            session.add_track(0, MediaType::Video, 5000, 5001, 6000, 6001);
            session.add_track(1, MediaType::Audio, 5002, 5003, 6002, 6003);
        }

        let session = mgr.get(&sid.0).expect("session");
        let video = session.track_by_type(MediaType::Video);
        assert!(video.is_some());
        assert_eq!(video.unwrap().track_id, 0);

        let audio = session.track_by_type(MediaType::Audio);
        assert!(audio.is_some());
        assert_eq!(audio.unwrap().track_id, 1);
    }

    #[test]
    fn teardown_cleans_tracks() {
        let mgr = create_manager();
        let sid = mgr.create_session(
            InstanceId::from("dev"),
            test_addr(),
            MediaConfig::default(),
            QualityTier::Standard,
        );

        {
            let mut session = mgr.get_mut(&sid.0).expect("session");
            session.add_track(0, MediaType::Video, 5000, 5001, 6000, 6001);
            session.add_track(1, MediaType::Audio, 5002, 5003, 6002, 6003);
        }

        let removed = mgr.remove(&sid.0).expect("removed");
        assert!(removed.cancel.is_cancelled());
    }
}
