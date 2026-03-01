use std::fmt;

use thiserror::Error;

// ---------------------------------------------------------------------------
// Top-level error
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum VScreenError {
    #[error("CDP error: {0}")]
    Cdp(#[from] CdpError),

    #[error("video error: {0}")]
    Video(#[from] VideoError),

    #[error("audio error: {0}")]
    Audio(#[from] AudioError),

    #[error("transport error: {0}")]
    Transport(#[from] TransportError),

    #[error("config error: {0}")]
    Config(#[from] ConfigError),

    #[error("instance not found: {0}")]
    InstanceNotFound(String),

    #[error("instance already exists: {0}")]
    InstanceAlreadyExists(String),

    #[error("invalid state transition: {0}")]
    InvalidState(String),

    #[error("invalid config: {0}")]
    InvalidConfig(String),

    #[error("limit exceeded: {0}")]
    LimitExceeded(String),

    #[error("no supervisor running for instance: {0}")]
    NoSupervisor(String),

    #[error("instance locked by another session: {0}")]
    InstanceLocked(String),

    #[error("lock not held: {0}")]
    LockNotHeld(String),

    #[error("lock wait timed out: {0}")]
    LockTimeout(String),

    #[error("service is shutting down")]
    ShuttingDown,

    #[error("channel closed")]
    ChannelClosed,
}

// ---------------------------------------------------------------------------
// Per-domain errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum CdpError {
    #[error("connection failed: {0}")]
    ConnectionFailed(String),

    #[error("connection lost")]
    ConnectionLost,

    #[error("reconnect exhausted after {attempts} attempts")]
    ReconnectExhausted { attempts: u32 },

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("screencast error: {0}")]
    Screencast(String),

    #[error("input dispatch failed: {0}")]
    InputDispatch(String),

    #[error("websocket error: {0}")]
    WebSocket(String),

    #[error("message too large: {size} bytes (max {max})")]
    MessageTooLarge { size: usize, max: usize },

    #[error("request timed out after {ms}ms")]
    Timeout { ms: u64 },
}

#[derive(Debug, Error)]
pub enum VideoError {
    #[error("decode failed: {0}")]
    DecodeFailed(String),

    #[error("frame too large: {size} bytes (max {max})")]
    FrameTooLarge { size: usize, max: usize },

    #[error("decode timeout after {ms}ms")]
    DecodeTimeout { ms: u64 },

    #[error("color conversion failed: {0}")]
    ConversionFailed(String),

    #[error("encode failed: {0}")]
    EncodeFailed(String),

    #[error("encoder not initialized")]
    EncoderNotInitialized,

    #[error("invalid resolution: {width}x{height}")]
    InvalidResolution { width: u32, height: u32 },
}

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("capture failed: {0}")]
    CaptureFailed(String),

    #[error("source not found: {0}")]
    SourceNotFound(String),

    #[error("source lost")]
    SourceLost,

    #[error("encode failed: {0}")]
    EncodeFailed(String),

    #[error("invalid sample rate: {0}")]
    InvalidSampleRate(u32),

    #[error("reconnect exhausted after {attempts} attempts")]
    ReconnectExhausted { attempts: u32 },
}

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("webrtc error: {0}")]
    WebRtc(String),

    #[error("signaling error: {0}")]
    Signaling(String),

    #[error("rtp send failed: {0}")]
    RtpSend(String),

    #[error("peer not found: {0}")]
    PeerNotFound(String),

    #[error("max peers reached: {current}/{max}")]
    MaxPeersReached { current: u32, max: u32 },

    #[error("data channel error: {0}")]
    DataChannel(String),

    #[error("ICE error: {0}")]
    Ice(String),
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("invalid value for {field}: {reason}")]
    InvalidValue {
        field: &'static str,
        reason: &'static str,
    },

    #[error("{field} out of range (min={min}, max={max})")]
    OutOfRange {
        field: &'static str,
        min: u64,
        max: u64,
    },

    #[error("parse error: {0}")]
    Parse(String),

    #[error("file error: {0}")]
    File(String),

    #[error("missing required field: {0}")]
    MissingField(&'static str),
}

/// API-level error for HTTP response mapping.
#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: u16,
    pub code: ApiErrorCode,
    pub message: String,
    pub details: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ApiErrorCode {
    InstanceNotFound,
    InstanceAlreadyExists,
    InstanceLocked,
    LockNotHeld,
    InvalidRequest,
    InvalidState,
    MaxInstancesReached,
    MaxPeersReached,
    ServiceUnavailable,
    CdpError,
    Timeout,
    InternalError,
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.status, self.message)
    }
}

impl std::error::Error for ApiError {}

impl From<&VScreenError> for ApiError {
    fn from(err: &VScreenError) -> Self {
        match err {
            VScreenError::InstanceNotFound(id) => Self {
                status: 404,
                code: ApiErrorCode::InstanceNotFound,
                message: format!("instance not found: {id}"),
                details: None,
            },
            VScreenError::InstanceAlreadyExists(id) => Self {
                status: 409,
                code: ApiErrorCode::InstanceAlreadyExists,
                message: format!("instance already exists: {id}"),
                details: None,
            },
            VScreenError::InstanceLocked(msg) => Self {
                status: 423,
                code: ApiErrorCode::InstanceLocked,
                message: msg.clone(),
                details: None,
            },
            VScreenError::LockNotHeld(msg) => Self {
                status: 409,
                code: ApiErrorCode::LockNotHeld,
                message: msg.clone(),
                details: None,
            },
            VScreenError::LockTimeout(msg) => Self {
                status: 408,
                code: ApiErrorCode::Timeout,
                message: msg.clone(),
                details: None,
            },
            VScreenError::InvalidState(reason) => Self {
                status: 400,
                code: ApiErrorCode::InvalidState,
                message: reason.clone(),
                details: None,
            },
            VScreenError::InvalidConfig(reason) => Self {
                status: 400,
                code: ApiErrorCode::InvalidRequest,
                message: reason.clone(),
                details: None,
            },
            VScreenError::LimitExceeded(reason) => Self {
                status: 429,
                code: ApiErrorCode::MaxInstancesReached,
                message: reason.clone(),
                details: None,
            },
            VScreenError::NoSupervisor(id) => Self {
                status: 503,
                code: ApiErrorCode::ServiceUnavailable,
                message: format!("no supervisor running for instance: {id}"),
                details: None,
            },
            VScreenError::ShuttingDown => Self {
                status: 503,
                code: ApiErrorCode::ServiceUnavailable,
                message: "service is shutting down".to_owned(),
                details: None,
            },
            VScreenError::Config(e) => Self {
                status: 400,
                code: ApiErrorCode::InvalidRequest,
                message: e.to_string(),
                details: None,
            },
            VScreenError::Cdp(CdpError::Timeout { ms }) => Self {
                status: 504,
                code: ApiErrorCode::Timeout,
                message: format!("CDP request timed out after {ms}ms"),
                details: None,
            },
            VScreenError::Cdp(cdp_err) => Self {
                status: 502,
                code: ApiErrorCode::CdpError,
                message: cdp_err.to_string(),
                details: None,
            },
            _ => Self {
                status: 500,
                code: ApiErrorCode::InternalError,
                message: "internal error".to_owned(),
                details: Some(err.to_string()),
            },
        }
    }
}

impl serde::Serialize for ApiError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;

        let mut outer = serializer.serialize_struct("ApiErrorResponse", 1)?;
        let inner = ApiErrorInner {
            code: self.code,
            message: &self.message,
            details: self.details.as_deref(),
        };
        outer.serialize_field("error", &inner)?;
        outer.end()
    }
}

#[derive(serde::Serialize)]
struct ApiErrorInner<'a> {
    code: ApiErrorCode,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<&'a str>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vscreen_error_display() {
        let err = VScreenError::InstanceNotFound("test-1".into());
        assert_eq!(err.to_string(), "instance not found: test-1");
    }

    #[test]
    fn cdp_error_display() {
        let err = CdpError::ReconnectExhausted { attempts: 10 };
        assert_eq!(
            err.to_string(),
            "reconnect exhausted after 10 attempts"
        );
    }

    #[test]
    fn video_error_display() {
        let err = VideoError::FrameTooLarge {
            size: 3_000_000,
            max: 2_097_152,
        };
        assert!(err.to_string().contains("3000000"));
    }

    #[test]
    fn api_error_from_vscreen_error() {
        let err = VScreenError::InstanceNotFound("abc".into());
        let api: ApiError = (&err).into();
        assert_eq!(api.status, 404);
        assert_eq!(api.code, ApiErrorCode::InstanceNotFound);
    }

    #[test]
    fn api_error_serialization() {
        let api = ApiError {
            status: 404,
            code: ApiErrorCode::InstanceNotFound,
            message: "not found".into(),
            details: None,
        };
        let json = serde_json::to_string(&api).expect("serialize");
        assert!(json.contains("INSTANCE_NOT_FOUND"));
        assert!(json.contains("\"error\""));
    }

    #[test]
    fn config_error_display() {
        let err = ConfigError::OutOfRange {
            field: "limits.frame_queue_depth",
            min: 1,
            max: 30,
        };
        assert!(err.to_string().contains("frame_queue_depth"));
    }

    #[test]
    fn transport_error_display() {
        let err = TransportError::MaxPeersReached {
            current: 5,
            max: 5,
        };
        assert!(err.to_string().contains("5/5"));
    }

    #[test]
    fn audio_error_display() {
        let err = AudioError::SourceNotFound("pulse_monitor".into());
        assert!(err.to_string().contains("pulse_monitor"));
    }

    #[test]
    fn vscreen_error_from_cdp() {
        let cdp = CdpError::ConnectionLost;
        let err: VScreenError = cdp.into();
        assert!(matches!(err, VScreenError::Cdp(_)));
    }

    #[test]
    fn api_error_shutting_down() {
        let err = VScreenError::ShuttingDown;
        let api: ApiError = (&err).into();
        assert_eq!(api.status, 503);
        assert_eq!(api.code, ApiErrorCode::ServiceUnavailable);
    }

    #[test]
    fn cdp_timeout_display() {
        let err = CdpError::Timeout { ms: 15000 };
        assert_eq!(err.to_string(), "request timed out after 15000ms");
    }

    #[test]
    fn api_error_from_cdp_timeout() {
        let cdp = CdpError::Timeout { ms: 5000 };
        let err = VScreenError::Cdp(cdp);
        let api: ApiError = (&err).into();
        assert_eq!(api.status, 504);
        assert_eq!(api.code, ApiErrorCode::Timeout);
        assert!(api.message.contains("5000"));
    }

    #[test]
    fn api_error_from_cdp_generic() {
        let cdp = CdpError::ConnectionLost;
        let err = VScreenError::Cdp(cdp);
        let api: ApiError = (&err).into();
        assert_eq!(api.status, 502);
        assert_eq!(api.code, ApiErrorCode::CdpError);
    }

    #[test]
    fn no_supervisor_error_display() {
        let err = VScreenError::NoSupervisor("dev".into());
        assert_eq!(
            err.to_string(),
            "no supervisor running for instance: dev"
        );
    }

    #[test]
    fn api_error_from_no_supervisor() {
        let err = VScreenError::NoSupervisor("my-instance".into());
        let api: ApiError = (&err).into();
        assert_eq!(api.status, 503);
        assert_eq!(api.code, ApiErrorCode::ServiceUnavailable);
        assert!(api.message.contains("my-instance"));
    }

    #[test]
    fn api_error_code_serialization_cdp() {
        let api = ApiError {
            status: 502,
            code: ApiErrorCode::CdpError,
            message: "cdp error".into(),
            details: None,
        };
        let json = serde_json::to_string(&api).expect("serialize");
        assert!(json.contains("CDP_ERROR"));
    }

    #[test]
    fn api_error_code_serialization_timeout() {
        let api = ApiError {
            status: 504,
            code: ApiErrorCode::Timeout,
            message: "timed out".into(),
            details: None,
        };
        let json = serde_json::to_string(&api).expect("serialize");
        assert!(json.contains("TIMEOUT"));
    }

    #[test]
    fn instance_locked_error_display() {
        let err = VScreenError::InstanceLocked("held by session abc".into());
        assert_eq!(
            err.to_string(),
            "instance locked by another session: held by session abc"
        );
    }

    #[test]
    fn api_error_from_instance_locked() {
        let err = VScreenError::InstanceLocked("held by agent X".into());
        let api: ApiError = (&err).into();
        assert_eq!(api.status, 423);
        assert_eq!(api.code, ApiErrorCode::InstanceLocked);
    }

    #[test]
    fn lock_not_held_error_display() {
        let err = VScreenError::LockNotHeld("dev".into());
        assert_eq!(err.to_string(), "lock not held: dev");
    }

    #[test]
    fn api_error_from_lock_not_held() {
        let err = VScreenError::LockNotHeld("dev".into());
        let api: ApiError = (&err).into();
        assert_eq!(api.status, 409);
        assert_eq!(api.code, ApiErrorCode::LockNotHeld);
    }

    #[test]
    fn lock_timeout_error_display() {
        let err = VScreenError::LockTimeout("dev".into());
        assert_eq!(err.to_string(), "lock wait timed out: dev");
    }

    #[test]
    fn api_error_from_lock_timeout() {
        let err = VScreenError::LockTimeout("dev".into());
        let api: ApiError = (&err).into();
        assert_eq!(api.status, 408);
        assert_eq!(api.code, ApiErrorCode::Timeout);
    }

    #[test]
    fn api_error_code_serialization_instance_locked() {
        let api = ApiError {
            status: 423,
            code: ApiErrorCode::InstanceLocked,
            message: "locked".into(),
            details: None,
        };
        let json = serde_json::to_string(&api).expect("serialize");
        assert!(json.contains("INSTANCE_LOCKED"));
    }
}
