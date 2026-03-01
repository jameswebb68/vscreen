use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::{AudioConfig, RtpOutputConfig, VideoConfig};

// ---------------------------------------------------------------------------
// Typed IDs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InstanceId(pub String);

impl fmt::Display for InstanceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for InstanceId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for InstanceId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl InstanceId {
    /// Returns `true` if the ID is safe for use in RTSP URLs and HTTP paths.
    /// Allows alphanumeric, hyphens, underscores, and dots. Must be 1-128 chars.
    #[must_use]
    pub fn is_url_safe(&self) -> bool {
        let s = &self.0;
        !s.is_empty()
            && s.len() <= 128
            && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PeerId(pub Uuid);

impl PeerId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for PeerId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ---------------------------------------------------------------------------
// Instance configuration (per-instance)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceConfig {
    pub instance_id: InstanceId,
    pub cdp_endpoint: String,
    pub pulse_source: String,
    #[serde(default)]
    pub display: Option<String>,
    #[serde(default)]
    pub video: VideoConfig,
    #[serde(default)]
    pub audio: AudioConfig,
    #[serde(default)]
    pub rtp_output: Option<RtpOutputConfig>,
}

// ---------------------------------------------------------------------------
// State machine (runtime)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum InstanceState {
    Created,
    Starting,
    Running,
    Degraded {
        reason: DegradedReason,
        since: chrono::DateTime<chrono::Utc>,
    },
    Failed {
        reason: FailureReason,
        since: chrono::DateTime<chrono::Utc>,
    },
    Stopping,
    Stopped,
}

impl InstanceState {
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Stopped | Self::Failed { .. })
    }

    #[must_use]
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running | Self::Degraded { .. })
    }

    #[must_use]
    pub fn can_accept_peers(&self) -> bool {
        matches!(self, Self::Running | Self::Degraded { .. })
    }
}

impl fmt::Display for InstanceState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Created => write!(f, "created"),
            Self::Starting => write!(f, "starting"),
            Self::Running => write!(f, "running"),
            Self::Degraded { reason, .. } => write!(f, "degraded ({reason})"),
            Self::Failed { reason, .. } => write!(f, "failed ({reason})"),
            Self::Stopping => write!(f, "stopping"),
            Self::Stopped => write!(f, "stopped"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DegradedReason {
    CdpDisconnected { reconnect_attempt: u32 },
    AudioSourceLost { reconnect_attempt: u32 },
    EncoderOverloaded { dropped_frames: u64 },
}

impl fmt::Display for DegradedReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CdpDisconnected { reconnect_attempt } => {
                write!(f, "CDP disconnected (attempt {reconnect_attempt})")
            }
            Self::AudioSourceLost { reconnect_attempt } => {
                write!(f, "audio source lost (attempt {reconnect_attempt})")
            }
            Self::EncoderOverloaded { dropped_frames } => {
                write!(f, "encoder overloaded ({dropped_frames} frames dropped)")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FailureReason {
    CdpReconnectExhausted,
    AudioSourcePermanentlyLost,
    InternalError { message: String },
}

impl fmt::Display for FailureReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CdpReconnectExhausted => write!(f, "CDP reconnect exhausted"),
            Self::AudioSourcePermanentlyLost => write!(f, "audio source permanently lost"),
            Self::InternalError { message } => write!(f, "internal error: {message}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Session and lock types (instance ownership / deconfliction)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub Uuid);

impl SessionId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LockType {
    Exclusive,
    Observer,
}

impl fmt::Display for LockType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exclusive => write!(f, "exclusive"),
            Self::Observer => write!(f, "observer"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LockToken(pub Uuid);

impl LockToken {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for LockToken {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for LockToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl LockToken {
    /// Parse a lock token from a string. Returns `None` if the string is not a valid UUID.
    pub fn parse(s: &str) -> Option<Self> {
        Uuid::parse_str(s).ok().map(Self)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LockInfo {
    pub session_id: SessionId,
    pub agent_name: Option<String>,
    pub lock_type: LockType,
    pub lock_token: LockToken,
    pub acquired_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    /// If true, this lock was auto-acquired (not via explicit vscreen_instance_lock).
    /// Auto-acquired locks can be reclaimed by any new session.
    #[serde(default)]
    pub auto_acquired: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LockStatus {
    pub instance_id: InstanceId,
    pub exclusive_holder: Option<LockInfo>,
    pub observers: Vec<LockInfo>,
    pub wait_queue: Vec<WaitQueueEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WaitQueueEntry {
    pub session_id: SessionId,
    pub agent_name: Option<String>,
    pub requested_lock_type: LockType,
    pub queued_at: chrono::DateTime<chrono::Utc>,
}

// ---------------------------------------------------------------------------
// Runtime reconfiguration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeVideoConfig {
    pub bitrate_kbps: Option<u32>,
    pub framerate: Option<u32>,
    pub cpu_used: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_id_display() {
        let id = InstanceId::from("test-instance");
        assert_eq!(id.to_string(), "test-instance");
    }

    #[test]
    fn instance_id_equality() {
        let a = InstanceId::from("a");
        let b = InstanceId::from("a");
        assert_eq!(a, b);
    }

    #[test]
    fn instance_id_url_safe() {
        assert!(InstanceId::from("dev").is_url_safe());
        assert!(InstanceId::from("my-instance-1").is_url_safe());
        assert!(InstanceId::from("test_123.v2").is_url_safe());
        assert!(!InstanceId::from("").is_url_safe());
        assert!(!InstanceId::from("has space").is_url_safe());
        assert!(!InstanceId::from("path/traversal").is_url_safe());
        assert!(!InstanceId::from("query?param").is_url_safe());
        assert!(!InstanceId::from("a".repeat(129).as_str()).is_url_safe());
    }

    #[test]
    fn peer_id_uniqueness() {
        let a = PeerId::new();
        let b = PeerId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn instance_state_is_terminal() {
        assert!(InstanceState::Stopped.is_terminal());
        assert!(InstanceState::Failed {
            reason: FailureReason::CdpReconnectExhausted,
            since: chrono::Utc::now(),
        }
        .is_terminal());
        assert!(!InstanceState::Running.is_terminal());
    }

    #[test]
    fn instance_state_is_running() {
        assert!(InstanceState::Running.is_running());
        assert!(InstanceState::Degraded {
            reason: DegradedReason::CdpDisconnected {
                reconnect_attempt: 1
            },
            since: chrono::Utc::now(),
        }
        .is_running());
        assert!(!InstanceState::Created.is_running());
    }

    #[test]
    fn instance_state_serialization() {
        let state = InstanceState::Running;
        let json = serde_json::to_string(&state).expect("serialize");
        assert!(json.contains("\"state\":\"running\""));
    }

    #[test]
    fn degraded_state_serialization() {
        let state = InstanceState::Degraded {
            reason: DegradedReason::CdpDisconnected {
                reconnect_attempt: 3,
            },
            since: chrono::Utc::now(),
        };
        let json = serde_json::to_string(&state).expect("serialize");
        assert!(json.contains("\"state\":\"degraded\""));
        assert!(json.contains("cdp_disconnected"));
    }

    #[test]
    fn instance_config_deserialization() {
        let json = r#"{
            "instance_id": "test-1",
            "cdp_endpoint": "ws://localhost:9222/devtools/page/ABC",
            "pulse_source": "sink-1.monitor"
        }"#;
        let config: InstanceConfig = serde_json::from_str(json).expect("deserialize");
        assert_eq!(config.instance_id.0, "test-1");
        assert_eq!(config.pulse_source, "sink-1.monitor");
    }

    #[test]
    fn runtime_video_config_partial() {
        let json = r#"{"bitrate_kbps": 2000}"#;
        let config: RuntimeVideoConfig = serde_json::from_str(json).expect("deserialize");
        assert_eq!(config.bitrate_kbps, Some(2000));
        assert!(config.framerate.is_none());
        assert!(config.cpu_used.is_none());
    }

    #[test]
    fn session_id_uniqueness() {
        let a = SessionId::new();
        let b = SessionId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn session_id_display() {
        let id = SessionId::new();
        let s = id.to_string();
        assert!(!s.is_empty());
        assert!(uuid::Uuid::parse_str(&s).is_ok());
    }

    #[test]
    fn lock_type_serialization() {
        let exc = LockType::Exclusive;
        let obs = LockType::Observer;
        assert_eq!(serde_json::to_string(&exc).unwrap(), "\"exclusive\"");
        assert_eq!(serde_json::to_string(&obs).unwrap(), "\"observer\"");
    }

    #[test]
    fn lock_type_deserialization() {
        let exc: LockType = serde_json::from_str("\"exclusive\"").unwrap();
        let obs: LockType = serde_json::from_str("\"observer\"").unwrap();
        assert_eq!(exc, LockType::Exclusive);
        assert_eq!(obs, LockType::Observer);
    }

    #[test]
    fn lock_type_display() {
        assert_eq!(LockType::Exclusive.to_string(), "exclusive");
        assert_eq!(LockType::Observer.to_string(), "observer");
    }

    #[test]
    fn lock_info_serialization() {
        let info = LockInfo {
            session_id: SessionId::new(),
            agent_name: Some("test-agent".into()),
            lock_type: LockType::Exclusive,
            lock_token: LockToken::new(),
            acquired_at: chrono::Utc::now(),
            expires_at: chrono::Utc::now(),
            auto_acquired: false,
        };
        let json = serde_json::to_string(&info).expect("serialize");
        assert!(json.contains("test-agent"));
        assert!(json.contains("exclusive"));
    }

    #[test]
    fn lock_status_serialization() {
        let status = LockStatus {
            instance_id: InstanceId::from("dev"),
            exclusive_holder: None,
            observers: vec![],
            wait_queue: vec![],
        };
        let json = serde_json::to_string(&status).expect("serialize");
        assert!(json.contains("dev"));
    }
}
