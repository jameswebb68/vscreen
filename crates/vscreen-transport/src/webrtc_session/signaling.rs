use serde::{Deserialize, Serialize};

/// WebRTC signaling messages exchanged over WebSocket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SignalingMessage {
    /// SDP offer from the client.
    Offer { sdp: String },
    /// SDP answer from the server.
    Answer { sdp: String },
    /// ICE candidate.
    IceCandidate {
        candidate: String,
        #[serde(rename = "sdpMLineIndex")]
        sdp_m_line_index: Option<u16>,
        #[serde(rename = "sdpMid")]
        sdp_mid: Option<String>,
    },
    /// Signal that ICE gathering is complete.
    IceComplete,
    /// Error from the server.
    Error { code: String, message: String },
    /// Peer connected acknowledgment.
    Connected { peer_id: String },
    /// Peer disconnected notification.
    Disconnected { reason: String },
    /// Clipboard content from the remote browser.
    Clipboard { text: String },
}

impl SignalingMessage {
    /// Parse a signaling message from JSON.
    ///
    /// # Errors
    /// Returns an error if deserialization fails.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Serialize to JSON.
    ///
    /// # Errors
    /// Returns an error if serialization fails (should not happen with valid data).
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offer_roundtrip() {
        let msg = SignalingMessage::Offer {
            sdp: "v=0\r\n...".into(),
        };
        let json = msg.to_json().expect("serialize");
        let parsed = SignalingMessage::from_json(&json).expect("parse");
        assert!(matches!(parsed, SignalingMessage::Offer { .. }));
    }

    #[test]
    fn answer_serialization() {
        let msg = SignalingMessage::Answer {
            sdp: "v=0\r\n...".into(),
        };
        let json = msg.to_json().expect("serialize");
        assert!(json.contains("\"type\":\"answer\""));
    }

    #[test]
    fn ice_candidate_deserialization() {
        let json = r#"{
            "type": "ice_candidate",
            "candidate": "candidate:1 1 UDP 2130706431 192.168.1.1 1234 typ host",
            "sdpMLineIndex": 0,
            "sdpMid": "0"
        }"#;
        let msg = SignalingMessage::from_json(json).expect("parse");
        assert!(matches!(msg, SignalingMessage::IceCandidate { .. }));
    }

    #[test]
    fn ice_complete() {
        let msg = SignalingMessage::IceComplete;
        let json = msg.to_json().expect("serialize");
        assert!(json.contains("ice_complete"));
    }

    #[test]
    fn error_message() {
        let msg = SignalingMessage::Error {
            code: "PEER_LIMIT".into(),
            message: "max peers reached".into(),
        };
        let json = msg.to_json().expect("serialize");
        assert!(json.contains("PEER_LIMIT"));
    }

    #[test]
    fn invalid_json_rejected() {
        let result = SignalingMessage::from_json("not json");
        assert!(result.is_err());
    }

    #[test]
    fn unknown_type_rejected() {
        let json = r#"{"type":"unknown","data":"test"}"#;
        let result = SignalingMessage::from_json(json);
        assert!(result.is_err());
    }
}
