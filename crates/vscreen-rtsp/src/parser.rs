use std::collections::HashMap;
use std::fmt;

use thiserror::Error;

/// RTSP protocol version.
pub const RTSP_VERSION: &str = "RTSP/1.0";

// ───────────────────────────── Errors ─────────────────────────────

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("incomplete message")]
    Incomplete,
    #[error("invalid request line: {0}")]
    InvalidRequestLine(String),
    #[error("invalid response status line: {0}")]
    InvalidStatusLine(String),
    #[error("unknown method: {0}")]
    UnknownMethod(String),
    #[error("missing CSeq header")]
    MissingCSeq,
    #[error("invalid header: {0}")]
    InvalidHeader(String),
    #[error("invalid transport header: {0}")]
    InvalidTransport(String),
    #[error("content-length mismatch")]
    ContentLengthMismatch,
}

// ───────────────────────────── Method ─────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    Options,
    Describe,
    Setup,
    Play,
    Pause,
    Teardown,
    GetParameter,
}

impl Method {
    /// Parse from an ASCII method string.
    ///
    /// # Errors
    /// Returns `ParseError::UnknownMethod` if the string is not a recognized RTSP method.
    pub fn parse(s: &str) -> Result<Self, ParseError> {
        match s {
            "OPTIONS" => Ok(Self::Options),
            "DESCRIBE" => Ok(Self::Describe),
            "SETUP" => Ok(Self::Setup),
            "PLAY" => Ok(Self::Play),
            "PAUSE" => Ok(Self::Pause),
            "TEARDOWN" => Ok(Self::Teardown),
            "GET_PARAMETER" => Ok(Self::GetParameter),
            _ => Err(ParseError::UnknownMethod(s.to_owned())),
        }
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Options => "OPTIONS",
            Self::Describe => "DESCRIBE",
            Self::Setup => "SETUP",
            Self::Play => "PLAY",
            Self::Pause => "PAUSE",
            Self::Teardown => "TEARDOWN",
            Self::GetParameter => "GET_PARAMETER",
        };
        f.write_str(s)
    }
}

// ─────────────────────── Session ID ───────────────────────

/// RTSP session identifier (opaque string, typically a UUID).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(pub String);

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for SessionId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<uuid::Uuid> for SessionId {
    fn from(id: uuid::Uuid) -> Self {
        Self(id.to_string())
    }
}

// ───────────────────── RTSP Request ───────────────────────

#[derive(Debug, Clone)]
pub struct RtspRequest {
    pub method: Method,
    pub url: String,
    pub cseq: u32,
    pub session: Option<SessionId>,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
}

impl RtspRequest {
    /// Parse a complete RTSP request from raw bytes.
    ///
    /// # Errors
    /// Returns `ParseError` on malformed input.
    pub fn parse(data: &[u8]) -> Result<Self, ParseError> {
        let text = std::str::from_utf8(data).map_err(|_| {
            ParseError::InvalidRequestLine("non-UTF-8 data".to_owned())
        })?;

        let (header_section, body_section) = split_header_body(text);
        let mut lines = header_section.lines();

        // Request line: METHOD URL RTSP/1.0
        let request_line = lines
            .next()
            .ok_or_else(|| ParseError::InvalidRequestLine("empty".to_owned()))?;
        let parts: Vec<&str> = request_line.splitn(3, ' ').collect();
        if parts.len() < 3 {
            return Err(ParseError::InvalidRequestLine(request_line.to_owned()));
        }

        let method = Method::parse(parts[0])?;
        let url = parts[1].to_owned();

        // Parse headers
        let headers = parse_headers(lines)?;

        // Extract CSeq (required)
        let cseq: u32 = headers
            .get("cseq")
            .ok_or(ParseError::MissingCSeq)?
            .parse()
            .map_err(|_| ParseError::MissingCSeq)?;

        // Extract Session (optional)
        let session = headers.get("session").map(|s| {
            // Session header can include timeout: "abc123;timeout=60"
            let id = s.split(';').next().unwrap_or(s).trim();
            SessionId(id.to_owned())
        });

        // Body handling
        let body = if let Some(cl_str) = headers.get("content-length") {
            let cl: usize = cl_str
                .parse()
                .map_err(|_| ParseError::InvalidHeader("invalid Content-Length".to_owned()))?;
            if cl > 0 {
                let body_str = body_section.unwrap_or("");
                if body_str.len() < cl {
                    return Err(ParseError::Incomplete);
                }
                Some(body_str[..cl].to_owned())
            } else {
                None
            }
        } else {
            None
        };

        Ok(Self {
            method,
            url,
            cseq,
            session,
            headers,
            body,
        })
    }
}

// ──────────────────── RTSP Response ───────────────────────

#[derive(Debug, Clone)]
pub struct RtspResponse {
    pub status: u16,
    pub reason: String,
    pub cseq: u32,
    pub session: Option<SessionId>,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
}

impl RtspResponse {
    /// Create a simple response with a given status and CSeq.
    #[must_use]
    pub fn new(status: u16, reason: &str, cseq: u32) -> Self {
        Self {
            status,
            reason: reason.to_owned(),
            cseq,
            session: None,
            headers: HashMap::new(),
            body: None,
        }
    }

    /// 200 OK response.
    #[must_use]
    pub fn ok(cseq: u32) -> Self {
        Self::new(200, "OK", cseq)
    }

    /// 404 Not Found response.
    #[must_use]
    pub fn not_found(cseq: u32) -> Self {
        Self::new(404, "Not Found", cseq)
    }

    /// 454 Session Not Found response.
    #[must_use]
    pub fn session_not_found(cseq: u32) -> Self {
        Self::new(454, "Session Not Found", cseq)
    }

    /// 461 Unsupported Transport response.
    #[must_use]
    pub fn unsupported_transport(cseq: u32) -> Self {
        Self::new(461, "Unsupported Transport", cseq)
    }

    /// 500 Internal Server Error response.
    #[must_use]
    pub fn internal_error(cseq: u32) -> Self {
        Self::new(500, "Internal Server Error", cseq)
    }

    /// 501 Not Implemented response.
    #[must_use]
    pub fn not_implemented(cseq: u32) -> Self {
        Self::new(501, "Not Implemented", cseq)
    }

    /// Set the Session header.
    #[must_use]
    pub fn with_session(mut self, session: SessionId, timeout_secs: u32) -> Self {
        self.session = Some(session);
        self.headers.insert(
            "Session".to_owned(),
            format!("{};timeout={timeout_secs}", self.session.as_ref().map_or("", |s| &s.0)),
        );
        self
    }

    /// Add a header.
    pub fn header(&mut self, key: &str, value: &str) {
        self.headers.insert(key.to_owned(), value.to_owned());
    }

    /// Set the body with appropriate Content-Length/Content-Type.
    pub fn set_body(&mut self, content_type: &str, body: String) {
        self.headers
            .insert("Content-Type".to_owned(), content_type.to_owned());
        self.headers
            .insert("Content-Length".to_owned(), body.len().to_string());
        self.body = Some(body);
    }

    /// Serialize to wire format.
    #[must_use]
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = String::with_capacity(512);

        buf.push_str(&format!(
            "{RTSP_VERSION} {} {}\r\n",
            self.status, self.reason
        ));
        buf.push_str(&format!("CSeq: {}\r\n", self.cseq));

        // Write Session header from the field if not already in headers map
        if let Some(ref session) = self.session {
            if !self.headers.contains_key("Session") {
                buf.push_str(&format!("Session: {}\r\n", session.0));
            }
        }

        for (key, value) in &self.headers {
            buf.push_str(&format!("{key}: {value}\r\n"));
        }

        buf.push_str("\r\n");

        if let Some(ref body) = self.body {
            buf.push_str(body);
        }

        buf.into_bytes()
    }
}

// ───────────────────── Transport Header ───────────────────

/// Transport mode for RTP delivery.
#[derive(Debug, Clone)]
pub enum TransportMode {
    /// UDP unicast: client receives RTP/RTCP on specified ports.
    UdpUnicast {
        client_rtp_port: u16,
        client_rtcp_port: u16,
    },
    /// TCP interleaved: RTP/RTCP multiplexed on the RTSP TCP connection.
    TcpInterleaved {
        rtp_channel: u8,
        rtcp_channel: u8,
    },
}

/// Parsed RTSP Transport header.
#[derive(Debug, Clone)]
pub struct TransportHeader {
    pub mode: TransportMode,
}

impl TransportHeader {
    /// Parse the Transport header value.
    ///
    /// Supports:
    /// - `RTP/AVP;unicast;client_port=X-Y` (UDP)
    /// - `RTP/AVP/TCP;unicast;interleaved=X-Y` (TCP interleaved)
    ///
    /// # Errors
    /// Returns `ParseError::InvalidTransport` if the header cannot be parsed.
    pub fn parse(value: &str) -> Result<Self, ParseError> {
        let lower = value.to_lowercase();

        if !lower.contains("rtp/avp") {
            return Err(ParseError::InvalidTransport(
                "missing RTP/AVP".to_owned(),
            ));
        }

        // TCP interleaved mode
        if lower.contains("rtp/avp/tcp") || lower.contains("interleaved=") {
            let interleaved_part = lower
                .split(';')
                .find(|s| s.trim().starts_with("interleaved="))
                .and_then(|s| s.trim().strip_prefix("interleaved="))
                .ok_or_else(|| {
                    ParseError::InvalidTransport(
                        "TCP mode requires interleaved= parameter".to_owned(),
                    )
                })?;

            let parts: Vec<&str> = interleaved_part.split('-').collect();
            let rtp_channel: u8 = parts[0].parse().map_err(|_| {
                ParseError::InvalidTransport("invalid interleaved channel".to_owned())
            })?;
            let rtcp_channel: u8 = if parts.len() > 1 {
                parts[1].parse().map_err(|_| {
                    ParseError::InvalidTransport("invalid interleaved channel".to_owned())
                })?
            } else {
                rtp_channel + 1
            };

            return Ok(Self {
                mode: TransportMode::TcpInterleaved {
                    rtp_channel,
                    rtcp_channel,
                },
            });
        }

        // UDP unicast mode
        let port_part = value
            .split(';')
            .find(|part| part.trim().to_lowercase().starts_with("client_port="))
            .ok_or_else(|| {
                ParseError::InvalidTransport("missing client_port".to_owned())
            })?;

        let port_value = port_part
            .split('=')
            .nth(1)
            .ok_or_else(|| {
                ParseError::InvalidTransport("malformed client_port".to_owned())
            })?
            .trim();

        let ports: Vec<&str> = port_value.split('-').collect();
        let rtp_port: u16 = ports[0].parse().map_err(|_| {
            ParseError::InvalidTransport("invalid RTP port number".to_owned())
        })?;

        let rtcp_port = if ports.len() > 1 {
            ports[1].parse().map_err(|_| {
                ParseError::InvalidTransport("invalid RTCP port number".to_owned())
            })?
        } else {
            rtp_port + 1
        };

        Ok(Self {
            mode: TransportMode::UdpUnicast {
                client_rtp_port: rtp_port,
                client_rtcp_port: rtcp_port,
            },
        })
    }

    /// Whether this transport uses TCP interleaved mode.
    #[must_use]
    pub fn is_tcp_interleaved(&self) -> bool {
        matches!(self.mode, TransportMode::TcpInterleaved { .. })
    }

    /// Format for use in a response Transport header.
    #[must_use]
    pub fn format_response(&self, server_rtp_port: u16, server_rtcp_port: u16) -> String {
        match &self.mode {
            TransportMode::UdpUnicast {
                client_rtp_port,
                client_rtcp_port,
            } => format!(
                "RTP/AVP;unicast;client_port={}-{};server_port={}-{}",
                client_rtp_port, client_rtcp_port, server_rtp_port, server_rtcp_port,
            ),
            TransportMode::TcpInterleaved {
                rtp_channel,
                rtcp_channel,
            } => format!(
                "RTP/AVP/TCP;unicast;interleaved={}-{}",
                rtp_channel, rtcp_channel,
            ),
        }
    }
}

// ──────────────── URL Parsing Helpers ─────────────────────

/// Extract the URL path from an RTSP URL, stripping scheme, host, and query string.
fn extract_url_path(url: &str) -> &str {
    let path_start = url.find("://").map(|i| i + 3).unwrap_or(0);
    let after_host = &url[path_start..];
    let path = after_host.find('/').map(|i| &after_host[i..]).unwrap_or("");
    path.split('?').next().unwrap_or(path)
}

/// Extract the instance ID from an RTSP URL.
///
/// Supported formats:
/// - `rtsp://host:port/stream/{instance_id}` (primary)
/// - `rtsp://host:port/stream/{instance_id}/trackID=N`
/// - `rtsp://host:port/audio/{instance_id}` (legacy, audio-only)
#[must_use]
pub fn extract_instance_id(url: &str) -> Option<String> {
    let path = extract_url_path(url);
    let path = path.strip_prefix('/').unwrap_or(path);
    let segments: Vec<&str> = path.split('/').collect();

    if segments.len() >= 2
        && (segments[0] == "stream" || segments[0] == "audio")
        && !segments[1].is_empty()
    {
        Some(segments[1].to_owned())
    } else {
        None
    }
}

/// Check if this URL uses the legacy `/audio/` prefix (audio-only mode).
#[must_use]
pub fn is_audio_only_url(url: &str) -> bool {
    let path = extract_url_path(url);
    let path = path.strip_prefix('/').unwrap_or(path);
    path.starts_with("audio/")
}

/// Extract the track ID from an RTSP URL.
///
/// Matches `trackID=N` as the last path segment. Returns `None` for aggregate URLs.
#[must_use]
pub fn extract_track_id(url: &str) -> Option<u8> {
    let path = extract_url_path(url);
    let path = path.strip_prefix('/').unwrap_or(path);
    let segments: Vec<&str> = path.split('/').collect();

    // Look for trackID=N in the last segment
    if let Some(last) = segments.last() {
        if let Some(id_str) = last.strip_prefix("trackID=") {
            return id_str.parse().ok();
        }
    }
    None
}

/// Parse media configuration from URL query parameters.
///
/// Default: both video and audio enabled. Legacy `/audio/` URLs default to audio-only.
#[must_use]
pub fn parse_media_config(url: &str) -> crate::session::MediaConfig {
    use crate::session::MediaConfig;

    if is_audio_only_url(url) {
        return MediaConfig::audio_only();
    }

    let params = extract_query_params(url);

    let video = params
        .get("video")
        .map_or(true, |v| !matches!(v.as_str(), "false" | "0" | "no"));
    let audio = params
        .get("audio")
        .map_or(true, |v| !matches!(v.as_str(), "false" | "0" | "no"));

    MediaConfig { video, audio }
}

/// Extract query parameters from an RTSP URL.
#[must_use]
pub fn extract_query_params(url: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();
    if let Some(query_start) = url.find('?') {
        let query = &url[query_start + 1..];
        for pair in query.split('&') {
            if let Some((key, value)) = pair.split_once('=') {
                params.insert(key.to_lowercase(), value.to_owned());
            }
        }
    }
    params
}

// ──────────────── Internal Helpers ─────────────────────────

/// Split raw message text into header section and optional body.
fn split_header_body(text: &str) -> (&str, Option<&str>) {
    if let Some(pos) = text.find("\r\n\r\n") {
        let headers = &text[..pos];
        let body = &text[pos + 4..];
        if body.is_empty() {
            (headers, None)
        } else {
            (headers, Some(body))
        }
    } else {
        (text, None)
    }
}

/// Parse header lines into a case-insensitive map.
/// Keys are stored in lowercase for lookup but we preserve original casing in the map.
fn parse_headers<'a>(
    lines: impl Iterator<Item = &'a str>,
) -> Result<HashMap<String, String>, ParseError> {
    let mut headers = HashMap::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (key, value) = trimmed.split_once(':').ok_or_else(|| {
            ParseError::InvalidHeader(trimmed.to_owned())
        })?;
        headers.insert(key.trim().to_lowercase(), value.trim().to_owned());
    }
    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_request(text: &str) -> Vec<u8> {
        text.replace('\n', "\r\n").into_bytes()
    }

    #[test]
    fn parse_options_request() {
        let raw = make_request(
            "OPTIONS rtsp://localhost:8554/audio/dev RTSP/1.0\n\
             CSeq: 1\n\
             \n",
        );
        let req = RtspRequest::parse(&raw).expect("parse OPTIONS");
        assert_eq!(req.method, Method::Options);
        assert_eq!(req.cseq, 1);
    }

    #[test]
    fn parse_describe_request() {
        let raw = make_request(
            "DESCRIBE rtsp://localhost:8554/audio/dev RTSP/1.0\n\
             CSeq: 2\n\
             Accept: application/sdp\n\
             \n",
        );
        let req = RtspRequest::parse(&raw).expect("parse DESCRIBE");
        assert_eq!(req.method, Method::Describe);
        assert_eq!(req.url, "rtsp://localhost:8554/audio/dev");
        assert_eq!(req.cseq, 2);
    }

    #[test]
    fn parse_setup_request() {
        let raw = make_request(
            "SETUP rtsp://localhost:8554/audio/dev/track1 RTSP/1.0\n\
             CSeq: 3\n\
             Transport: RTP/AVP;unicast;client_port=5000-5001\n\
             \n",
        );
        let req = RtspRequest::parse(&raw).expect("parse SETUP");
        assert_eq!(req.method, Method::Setup);
        assert_eq!(req.cseq, 3);

        let transport_str = req.headers.get("transport").expect("transport header");
        let transport = TransportHeader::parse(transport_str).expect("parse transport");
        match &transport.mode {
            TransportMode::UdpUnicast { client_rtp_port, client_rtcp_port } => {
                assert_eq!(*client_rtp_port, 5000);
                assert_eq!(*client_rtcp_port, 5001);
            }
            _ => panic!("expected UDP unicast"),
        }
    }

    #[test]
    fn parse_play_with_session() {
        let raw = make_request(
            "PLAY rtsp://localhost:8554/audio/dev RTSP/1.0\n\
             CSeq: 4\n\
             Session: abc-123\n\
             \n",
        );
        let req = RtspRequest::parse(&raw).expect("parse PLAY");
        assert_eq!(req.method, Method::Play);
        assert_eq!(req.session.as_ref().map(|s| s.0.as_str()), Some("abc-123"));
    }

    #[test]
    fn parse_session_with_timeout() {
        let raw = make_request(
            "PLAY rtsp://localhost:8554/audio/dev RTSP/1.0\n\
             CSeq: 5\n\
             Session: abc-123;timeout=60\n\
             \n",
        );
        let req = RtspRequest::parse(&raw).expect("parse PLAY");
        assert_eq!(req.session.as_ref().map(|s| s.0.as_str()), Some("abc-123"));
    }

    #[test]
    fn missing_cseq() {
        let raw = make_request(
            "OPTIONS rtsp://localhost:8554/audio/dev RTSP/1.0\n\
             \n",
        );
        assert!(RtspRequest::parse(&raw).is_err());
    }

    #[test]
    fn unknown_method() {
        let raw = make_request(
            "FOOBAR rtsp://localhost RTSP/1.0\n\
             CSeq: 1\n\
             \n",
        );
        let err = RtspRequest::parse(&raw).unwrap_err();
        assert!(matches!(err, ParseError::UnknownMethod(_)));
    }

    #[test]
    fn transport_header_parsing() {
        let t = TransportHeader::parse("RTP/AVP;unicast;client_port=5000-5001")
            .expect("parse transport");
        match &t.mode {
            TransportMode::UdpUnicast { client_rtp_port, client_rtcp_port } => {
                assert_eq!(*client_rtp_port, 5000);
                assert_eq!(*client_rtcp_port, 5001);
            }
            _ => panic!("expected UDP unicast"),
        }
    }

    #[test]
    fn transport_header_single_port() {
        let t = TransportHeader::parse("RTP/AVP;unicast;client_port=5000")
            .expect("parse transport");
        match &t.mode {
            TransportMode::UdpUnicast { client_rtp_port, client_rtcp_port } => {
                assert_eq!(*client_rtp_port, 5000);
                assert_eq!(*client_rtcp_port, 5001);
            }
            _ => panic!("expected UDP unicast"),
        }
    }

    #[test]
    fn transport_missing_rtp_avp() {
        let result = TransportHeader::parse("UDP;unicast;client_port=5000");
        assert!(result.is_err());
    }

    #[test]
    fn transport_tcp_interleaved() {
        let t = TransportHeader::parse("RTP/AVP/TCP;unicast;interleaved=0-1")
            .expect("parse transport");
        match &t.mode {
            TransportMode::TcpInterleaved { rtp_channel, rtcp_channel } => {
                assert_eq!(*rtp_channel, 0);
                assert_eq!(*rtcp_channel, 1);
            }
            _ => panic!("expected TCP interleaved"),
        }
        assert!(t.is_tcp_interleaved());
    }

    #[test]
    fn transport_tcp_interleaved_channels_2_3() {
        let t = TransportHeader::parse("RTP/AVP/TCP;unicast;interleaved=2-3")
            .expect("parse transport");
        match &t.mode {
            TransportMode::TcpInterleaved { rtp_channel, rtcp_channel } => {
                assert_eq!(*rtp_channel, 2);
                assert_eq!(*rtcp_channel, 3);
            }
            _ => panic!("expected TCP interleaved"),
        }
    }

    #[test]
    fn transport_format_response_udp() {
        let t = TransportHeader {
            mode: TransportMode::UdpUnicast {
                client_rtp_port: 5000,
                client_rtcp_port: 5001,
            },
        };
        let resp = t.format_response(6000, 6001);
        assert_eq!(
            resp,
            "RTP/AVP;unicast;client_port=5000-5001;server_port=6000-6001"
        );
    }

    #[test]
    fn transport_format_response_tcp() {
        let t = TransportHeader {
            mode: TransportMode::TcpInterleaved {
                rtp_channel: 0,
                rtcp_channel: 1,
            },
        };
        let resp = t.format_response(0, 0);
        assert_eq!(resp, "RTP/AVP/TCP;unicast;interleaved=0-1");
    }

    #[test]
    fn response_serialize() {
        let mut resp = RtspResponse::ok(1);
        resp.header("Public", "DESCRIBE, SETUP, PLAY, PAUSE, TEARDOWN");
        let bytes = resp.serialize();
        let text = String::from_utf8(bytes).expect("utf8");
        assert!(text.starts_with("RTSP/1.0 200 OK\r\n"));
        assert!(text.contains("CSeq: 1\r\n"));
        assert!(text.contains("Public: DESCRIBE, SETUP, PLAY, PAUSE, TEARDOWN"));
    }

    #[test]
    fn response_with_sdp_body() {
        let mut resp = RtspResponse::ok(2);
        let body = "v=0\r\n".to_owned();
        let body_len = body.len(); // 5 bytes
        resp.set_body("application/sdp", body);
        let bytes = resp.serialize();
        let text = String::from_utf8(bytes).expect("utf8");
        assert!(text.contains("Content-Type: application/sdp"));
        assert!(text.contains(&format!("Content-Length: {body_len}")));
        assert!(text.ends_with("\r\n\r\nv=0\r\n"));
    }

    #[test]
    fn extract_instance_id_standard() {
        assert_eq!(
            extract_instance_id("rtsp://localhost:8554/audio/dev"),
            Some("dev".to_owned())
        );
    }

    #[test]
    fn extract_instance_id_with_query() {
        assert_eq!(
            extract_instance_id("rtsp://localhost:8554/audio/my-instance?tier=high"),
            Some("my-instance".to_owned())
        );
    }

    #[test]
    fn extract_instance_id_with_track() {
        assert_eq!(
            extract_instance_id("rtsp://localhost:8554/audio/dev/track1"),
            Some("dev".to_owned())
        );
    }

    #[test]
    fn extract_instance_id_missing() {
        assert_eq!(extract_instance_id("rtsp://localhost:8554/"), None);
        assert_eq!(extract_instance_id("rtsp://localhost:8554/audio/"), None);
    }

    #[test]
    fn extract_query_params_basic() {
        let params =
            extract_query_params("rtsp://host/audio/dev?tier=high&kbps=192");
        assert_eq!(params.get("tier").map(String::as_str), Some("high"));
        assert_eq!(params.get("kbps").map(String::as_str), Some("192"));
    }

    #[test]
    fn method_display() {
        assert_eq!(Method::Options.to_string(), "OPTIONS");
        assert_eq!(Method::GetParameter.to_string(), "GET_PARAMETER");
    }

    // ──────── New URL routing tests ────────

    #[test]
    fn extract_instance_id_stream_url() {
        assert_eq!(
            extract_instance_id("rtsp://localhost:8554/stream/dev"),
            Some("dev".to_owned())
        );
    }

    #[test]
    fn extract_instance_id_stream_with_track() {
        assert_eq!(
            extract_instance_id("rtsp://localhost:8554/stream/my-inst/trackID=0"),
            Some("my-inst".to_owned())
        );
    }

    #[test]
    fn extract_instance_id_stream_with_query() {
        assert_eq!(
            extract_instance_id("rtsp://localhost:8554/stream/foo?video=false"),
            Some("foo".to_owned())
        );
    }

    #[test]
    fn is_audio_only_url_test() {
        assert!(is_audio_only_url("rtsp://localhost/audio/dev"));
        assert!(!is_audio_only_url("rtsp://localhost/stream/dev"));
        assert!(!is_audio_only_url("rtsp://localhost/stream/dev?audio=true"));
    }

    #[test]
    fn extract_track_id_video() {
        assert_eq!(
            extract_track_id("rtsp://localhost/stream/dev/trackID=0"),
            Some(0)
        );
    }

    #[test]
    fn extract_track_id_audio() {
        assert_eq!(
            extract_track_id("rtsp://localhost/stream/dev/trackID=1"),
            Some(1)
        );
    }

    #[test]
    fn extract_track_id_aggregate() {
        assert_eq!(
            extract_track_id("rtsp://localhost/stream/dev"),
            None
        );
    }

    #[test]
    fn parse_media_config_default() {
        let cfg = parse_media_config("rtsp://localhost/stream/dev");
        assert!(cfg.video);
        assert!(cfg.audio);
    }

    #[test]
    fn parse_media_config_video_disabled() {
        let cfg = parse_media_config("rtsp://localhost/stream/dev?video=false");
        assert!(!cfg.video);
        assert!(cfg.audio);
    }

    #[test]
    fn parse_media_config_audio_disabled() {
        let cfg = parse_media_config("rtsp://localhost/stream/dev?audio=0");
        assert!(cfg.video);
        assert!(!cfg.audio);
    }

    #[test]
    fn parse_media_config_both_disabled() {
        let cfg = parse_media_config("rtsp://localhost/stream/dev?video=no&audio=false");
        assert!(!cfg.video);
        assert!(!cfg.audio);
    }

    #[test]
    fn parse_media_config_legacy_audio_url() {
        let cfg = parse_media_config("rtsp://localhost/audio/dev");
        assert!(!cfg.video);
        assert!(cfg.audio);
    }
}
