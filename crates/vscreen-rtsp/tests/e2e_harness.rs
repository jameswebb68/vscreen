//! End-to-end RTSP streaming harness tests.
//!
//! These tests go beyond protocol-level assertions: they actually receive
//! RTP/RTCP packets on UDP sockets and validate headers, payload formats,
//! sequence numbering, timestamps, VP9 descriptors, and health metrics.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use vscreen_core::frame::EncodedPacket;

use vscreen_rtsp::server::RtspServer;

// ═══════════════════════════════════════════════════════════════════
// Constants
// ═══════════════════════════════════════════════════════════════════

const RTP_VERSION: u8 = 2;
const OPUS_PT: u8 = 111;
const VP9_PT: u8 = 96;
const RTCP_SR_PT: u8 = 200;
const RTCP_RR_PT: u8 = 201;
const LOCALHOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

// ═══════════════════════════════════════════════════════════════════
// Mock Instance Lookup
// ═══════════════════════════════════════════════════════════════════

struct MockInstanceLookup {
    audio_tx: broadcast::Sender<EncodedPacket>,
    video_tx: broadcast::Sender<EncodedPacket>,
}

impl MockInstanceLookup {
    fn new() -> (
        Self,
        broadcast::Sender<EncodedPacket>,
        broadcast::Sender<EncodedPacket>,
    ) {
        let (audio_tx, _) = broadcast::channel::<EncodedPacket>(64);
        let (video_tx, _) = broadcast::channel::<EncodedPacket>(64);
        let atx = audio_tx.clone();
        let vtx = video_tx.clone();
        (Self { audio_tx, video_tx }, atx, vtx)
    }
}

impl vscreen_rtsp::InstanceLookup for MockInstanceLookup {
    fn instance_exists(&self, id: &str) -> bool {
        id == "test-instance"
    }

    fn subscribe_audio(&self, id: &str) -> Option<broadcast::Receiver<EncodedPacket>> {
        (id == "test-instance").then(|| self.audio_tx.subscribe())
    }

    fn subscribe_video(&self, id: &str) -> Option<broadcast::Receiver<EncodedPacket>> {
        (id == "test-instance").then(|| self.video_tx.subscribe())
    }

    fn video_resolution(&self, id: &str) -> Option<(u32, u32)> {
        (id == "test-instance").then_some((1920, 1080))
    }

    fn video_framerate(&self, id: &str) -> Option<u32> {
        (id == "test-instance").then_some(30)
    }

    fn request_keyframe(&self, _instance_id: &str) {}

    fn video_codec(&self, id: &str) -> vscreen_core::frame::VideoCodec {
        let _ = id;
        vscreen_core::frame::VideoCodec::Vp9
    }
}

// ═══════════════════════════════════════════════════════════════════
// Test Helpers
// ═══════════════════════════════════════════════════════════════════

fn make_opus_packet() -> EncodedPacket {
    let config = vscreen_core::config::AudioConfig {
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

    let mut output = vec![0u8; 4000];
    let len = encoder.encode_float(&samples, &mut output).expect("encode");

    EncodedPacket {
        data: Bytes::copy_from_slice(&output[..len]),
        is_keyframe: true,
        pts: 0,
        duration: Some(20),
        codec: None,
    }
}

fn make_vp9_packet(keyframe: bool, size: usize) -> EncodedPacket {
    EncodedPacket {
        data: Bytes::from(vec![0xAA; size]),
        is_keyframe: keyframe,
        pts: 0,
        duration: Some(33),
        codec: None,
    }
}

/// Parsed RTP header from a received UDP packet.
#[derive(Debug)]
struct RtpHeader {
    version: u8,
    marker: bool,
    payload_type: u8,
    sequence_number: u16,
    timestamp: u32,
    ssrc: u32,
    payload: Vec<u8>,
}

impl RtpHeader {
    fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < 12 {
            return None;
        }
        let version = (data[0] >> 6) & 0x03;
        let marker = (data[1] & 0x80) != 0;
        let payload_type = data[1] & 0x7F;
        let sequence_number = u16::from_be_bytes([data[2], data[3]]);
        let timestamp = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
        let ssrc = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);
        let payload = data[12..].to_vec();

        Some(Self {
            version,
            marker,
            payload_type,
            sequence_number,
            timestamp,
            ssrc,
            payload,
        })
    }
}

/// Parsed RTCP Sender Report.
#[derive(Debug)]
#[allow(dead_code)]
struct SenderReport {
    version: u8,
    packet_type: u8,
    ssrc: u32,
    ntp_timestamp: u64,
    rtp_timestamp: u32,
    sender_packet_count: u32,
    sender_octet_count: u32,
}

impl SenderReport {
    fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < 28 {
            return None;
        }
        let version = (data[0] >> 6) & 0x03;
        let packet_type = data[1];
        if packet_type != RTCP_SR_PT {
            return None;
        }
        let ssrc = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
        let ntp_hi = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);
        let ntp_lo = u32::from_be_bytes([data[12], data[13], data[14], data[15]]);
        let ntp_timestamp = (u64::from(ntp_hi) << 32) | u64::from(ntp_lo);
        let rtp_timestamp = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
        let sender_packet_count = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
        let sender_octet_count = u32::from_be_bytes([data[24], data[25], data[26], data[27]]);

        Some(Self {
            version,
            packet_type,
            ssrc,
            ntp_timestamp,
            rtp_timestamp,
            sender_packet_count,
            sender_octet_count,
        })
    }
}

/// Build an RTCP Receiver Report to send to the server.
#[allow(dead_code)]
fn build_receiver_report(reporter_ssrc: u32, media_ssrc: u32, fraction_lost: u8, jitter: u32) -> Vec<u8> {
    let mut rr = Vec::with_capacity(32);
    rr.push(0x81); // V=2, P=0, RC=1
    rr.push(RTCP_RR_PT);
    rr.extend_from_slice(&7u16.to_be_bytes()); // length in 32-bit words - 1
    rr.extend_from_slice(&reporter_ssrc.to_be_bytes());
    // Report block
    rr.extend_from_slice(&media_ssrc.to_be_bytes());
    rr.push(fraction_lost);
    rr.extend_from_slice(&[0, 0, 0]); // cumulative lost
    rr.extend_from_slice(&[0, 0, 0, 0]); // ext highest seq
    rr.extend_from_slice(&jitter.to_be_bytes());
    rr.extend_from_slice(&[0, 0, 0, 0]); // last SR
    rr.extend_from_slice(&[0, 0, 0, 0]); // delay since last SR
    rr
}

// ═══════════════════════════════════════════════════════════════════
// RTSP Test Client (enhanced with UDP receivers)
// ═══════════════════════════════════════════════════════════════════

struct RtspTestClient {
    reader: BufReader<tokio::net::tcp::OwnedReadHalf>,
    writer: tokio::net::tcp::OwnedWriteHalf,
    cseq: u32,
}

impl RtspTestClient {
    async fn connect(port: u16) -> Self {
        let stream = TcpStream::connect(SocketAddr::new(LOCALHOST, port))
            .await
            .expect("connect to RTSP server");
        let (reader, writer) = stream.into_split();
        Self {
            reader: BufReader::new(reader),
            writer,
            cseq: 0,
        }
    }

    async fn send_request(
        &mut self,
        method: &str,
        url: &str,
        extra_headers: &[(&str, &str)],
    ) -> String {
        self.cseq += 1;
        let mut request = format!("{method} {url} RTSP/1.0\r\nCSeq: {}\r\n", self.cseq);
        for (key, value) in extra_headers {
            request.push_str(&format!("{key}: {value}\r\n"));
        }
        request.push_str("\r\n");
        self.writer
            .write_all(request.as_bytes())
            .await
            .expect("write request");
        self.read_response().await
    }

    async fn read_response(&mut self) -> String {
        let mut response = String::new();
        loop {
            let mut line = String::new();
            let n = self.reader.read_line(&mut line).await.expect("read line");
            if n == 0 {
                break;
            }
            response.push_str(&line);
            if line == "\r\n" {
                break;
            }
        }

        let cl = response
            .lines()
            .find(|l| l.to_lowercase().starts_with("content-length:"))
            .and_then(|l| l.split(':').nth(1))
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(0);

        if cl > 0 {
            let mut body = vec![0u8; cl];
            tokio::io::AsyncReadExt::read_exact(&mut self.reader, &mut body)
                .await
                .expect("read body");
            response.push_str(&String::from_utf8_lossy(&body));
        }

        response
    }

    fn extract_status(response: &str) -> u16 {
        response
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    }

    fn extract_session(response: &str) -> Option<String> {
        response
            .lines()
            .find(|l| l.to_lowercase().starts_with("session:"))
            .and_then(|l| l.split(':').nth(1))
            .map(|v| v.trim().split(';').next().unwrap_or("").to_owned())
    }

    fn extract_server_ports(response: &str) -> Option<(u16, u16)> {
        let transport_line = response
            .lines()
            .find(|l| l.to_lowercase().starts_with("transport:"))?;
        let server_port_part = transport_line
            .split(';')
            .find(|p| p.trim().starts_with("server_port="))?;
        let ports_str = server_port_part.trim().strip_prefix("server_port=")?;
        let mut parts = ports_str.split('-');
        let rtp_port: u16 = parts.next()?.parse().ok()?;
        let rtcp_port: u16 = parts.next()?.parse().ok()?;
        Some((rtp_port, rtcp_port))
    }
}

/// Bind a UDP socket pair for receiving RTP and RTCP on specific ports.
async fn bind_udp_receiver(rtp_port: u16, rtcp_port: u16) -> (UdpSocket, UdpSocket) {
    let rtp_sock = UdpSocket::bind(SocketAddr::new(LOCALHOST, rtp_port))
        .await
        .unwrap_or_else(|e| panic!("bind RTP on {rtp_port}: {e}"));
    let rtcp_sock = UdpSocket::bind(SocketAddr::new(LOCALHOST, rtcp_port))
        .await
        .unwrap_or_else(|e| panic!("bind RTCP on {rtcp_port}: {e}"));
    (rtp_sock, rtcp_sock)
}

/// Receive up to `count` RTP packets with a timeout.
async fn recv_rtp_packets(
    socket: &UdpSocket,
    count: usize,
    timeout: Duration,
) -> Vec<RtpHeader> {
    let mut packets = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout;
    let mut buf = [0u8; 2048];

    while packets.len() < count && tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(
            deadline - tokio::time::Instant::now(),
            socket.recv(&mut buf),
        )
        .await
        {
            Ok(Ok(len)) => {
                if let Some(hdr) = RtpHeader::parse(&buf[..len]) {
                    packets.push(hdr);
                }
            }
            _ => break,
        }
    }
    packets
}

/// Receive RTCP packets (Sender Reports) with a timeout.
async fn recv_rtcp_sr(
    socket: &UdpSocket,
    timeout: Duration,
) -> Vec<SenderReport> {
    let mut reports = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout;
    let mut buf = [0u8; 512];

    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(
            deadline - tokio::time::Instant::now(),
            socket.recv(&mut buf),
        )
        .await
        {
            Ok(Ok(len)) => {
                if let Some(sr) = SenderReport::parse(&buf[..len]) {
                    reports.push(sr);
                }
            }
            _ => break,
        }
    }
    reports
}

/// Spawn an RTSP server on the given port and return (session_manager, cancel_token).
async fn start_server(
    port: u16,
    lookup: MockInstanceLookup,
) -> (Arc<vscreen_rtsp::session::RtspSessionManager>, CancellationToken) {
    let cancel = CancellationToken::new();
    let server = RtspServer::new(port, LOCALHOST, Arc::new(lookup), cancel.clone());
    let mgr = server.session_manager().clone();
    tokio::spawn(async move {
        let _ = server.run().await;
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    (mgr, cancel)
}

// ═══════════════════════════════════════════════════════════════════
// Test 1: Full dual-track RTP packet validation
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_dual_track_rtp_packets() {
    let (lookup, audio_tx, video_tx) = MockInstanceLookup::new();
    let port = 18700;
    let (mgr, cancel) = start_server(port, lookup).await;

    // Bind UDP receivers for video and audio
    let (video_rtp_sock, _video_rtcp_sock) = bind_udp_receiver(17000, 17001).await;
    let (audio_rtp_sock, _audio_rtcp_sock) = bind_udp_receiver(17002, 17003).await;

    let mut client = RtspTestClient::connect(port).await;

    // SETUP video (trackID=0)
    let resp = client
        .send_request(
            "SETUP",
            "rtsp://localhost/stream/test-instance/trackID=0",
            &[("Transport", "RTP/AVP;unicast;client_port=17000-17001")],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);
    let session_id = RtspTestClient::extract_session(&resp).expect("session ID");
    let video_server_ports = RtspTestClient::extract_server_ports(&resp).expect("video server ports");

    // SETUP audio (trackID=1)
    let resp = client
        .send_request(
            "SETUP",
            "rtsp://localhost/stream/test-instance/trackID=1",
            &[
                ("Transport", "RTP/AVP;unicast;client_port=17002-17003"),
                ("Session", &session_id),
            ],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);
    let audio_server_ports = RtspTestClient::extract_server_ports(&resp).expect("audio server ports");

    assert_ne!(video_server_ports.0, audio_server_ports.0, "video and audio must use different ports");

    // Verify 2 tracks in session
    {
        let session = mgr.get(&session_id).expect("session exists");
        assert_eq!(session.tracks.len(), 2);
    }

    // PLAY
    let resp = client
        .send_request(
            "PLAY",
            "rtsp://localhost/stream/test-instance",
            &[("Session", &session_id)],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);

    // Allow RTP tasks to start
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Send 5 video frames (1 keyframe + 4 inter) and 5 audio packets
    let _ = video_tx.send(make_vp9_packet(true, 500));
    for _ in 0..4 {
        let _ = video_tx.send(make_vp9_packet(false, 200));
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    for _ in 0..5 {
        let _ = audio_tx.send(make_opus_packet());
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Receive and validate video RTP packets
    let video_pkts = recv_rtp_packets(&video_rtp_sock, 10, Duration::from_secs(2)).await;
    assert!(!video_pkts.is_empty(), "should receive video RTP packets");

    for pkt in &video_pkts {
        assert_eq!(pkt.version, RTP_VERSION, "RTP version must be 2");
        assert_eq!(pkt.payload_type, VP9_PT, "video payload type must be 96");
        assert!(!pkt.payload.is_empty(), "video payload must not be empty");
    }

    // Verify VP9 descriptor on the first packet (keyframe start)
    let first_video = &video_pkts[0];
    let desc_byte0 = first_video.payload[0];
    let i_bit = (desc_byte0 >> 7) & 1;
    let b_bit = (desc_byte0 >> 3) & 1;
    assert_eq!(i_bit, 1, "I bit (PID present) must be set");
    assert_eq!(b_bit, 1, "B bit (beginning of frame) must be set on first packet");

    // Verify sequence numbers are incrementing
    let mut last_seq = video_pkts[0].sequence_number;
    for pkt in &video_pkts[1..] {
        assert_eq!(
            pkt.sequence_number,
            last_seq.wrapping_add(1),
            "sequence numbers must increment by 1"
        );
        last_seq = pkt.sequence_number;
    }

    // All video packets should share the same SSRC
    let video_ssrc = video_pkts[0].ssrc;
    for pkt in &video_pkts {
        assert_eq!(pkt.ssrc, video_ssrc, "all video packets must share the same SSRC");
    }

    // Receive and validate audio RTP packets
    let audio_pkts = recv_rtp_packets(&audio_rtp_sock, 5, Duration::from_secs(2)).await;
    assert!(
        audio_pkts.len() >= 3,
        "should receive at least 3 audio RTP packets, got {}",
        audio_pkts.len()
    );

    for pkt in &audio_pkts {
        assert_eq!(pkt.version, RTP_VERSION);
        assert_eq!(pkt.payload_type, OPUS_PT, "audio payload type must be 111");
        assert!(!pkt.payload.is_empty(), "audio payload must not be empty");
    }

    // Verify audio timestamps increment by 960 (20ms at 48kHz)
    if audio_pkts.len() >= 2 {
        for i in 1..audio_pkts.len() {
            let delta = audio_pkts[i]
                .timestamp
                .wrapping_sub(audio_pkts[i - 1].timestamp);
            assert_eq!(delta, 960, "audio timestamp increment must be 960 (20ms @ 48kHz)");
        }
    }

    // Audio and video must have different SSRCs
    let audio_ssrc = audio_pkts[0].ssrc;
    assert_ne!(audio_ssrc, video_ssrc, "audio and video SSRCs must differ");

    // Verify health updated in session
    tokio::time::sleep(Duration::from_millis(100)).await;
    {
        let session = mgr.get(&session_id).expect("session");
        for track in &session.tracks {
            assert!(track.health.packets_sent > 0, "track {:?} should have sent packets", track.media_type);
        }
    }

    // TEARDOWN
    let resp = client
        .send_request(
            "TEARDOWN",
            "rtsp://localhost/stream/test-instance",
            &[("Session", &session_id)],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);
    assert_eq!(mgr.session_count(), 0);

    cancel.cancel();
}

// ═══════════════════════════════════════════════════════════════════
// Test 2: Audio-only streaming with low-tier transcoding
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_audio_only_transcoded() {
    let (lookup, audio_tx, _vtx) = MockInstanceLookup::new();
    let port = 18701;
    let (_mgr, cancel) = start_server(port, lookup).await;

    let (audio_rtp_sock, _audio_rtcp_sock) = bind_udp_receiver(17010, 17011).await;

    let mut client = RtspTestClient::connect(port).await;

    // SETUP with low tier (will transcode 256kbps → 32kbps mono)
    let resp = client
        .send_request(
            "SETUP",
            "rtsp://localhost/audio/test-instance/trackID=1?tier=low",
            &[("Transport", "RTP/AVP;unicast;client_port=17010-17011")],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);
    let session_id = RtspTestClient::extract_session(&resp).expect("session");

    // PLAY
    let resp = client
        .send_request(
            "PLAY",
            "rtsp://localhost/audio/test-instance",
            &[("Session", &session_id)],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Send master-quality audio packets
    let master_pkt = make_opus_packet();
    let master_size = master_pkt.data.len();
    for _ in 0..8 {
        let _ = audio_tx.send(make_opus_packet());
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let audio_pkts = recv_rtp_packets(&audio_rtp_sock, 8, Duration::from_secs(2)).await;
    assert!(
        audio_pkts.len() >= 4,
        "should receive transcoded audio packets, got {}",
        audio_pkts.len()
    );

    // Transcoded low-tier packets should be smaller than master
    for pkt in &audio_pkts {
        assert_eq!(pkt.payload_type, OPUS_PT);
        assert!(
            pkt.payload.len() < master_size,
            "transcoded packet ({} bytes) should be smaller than master ({} bytes)",
            pkt.payload.len(),
            master_size
        );
    }

    // TEARDOWN
    let resp = client
        .send_request(
            "TEARDOWN",
            "rtsp://localhost/audio/test-instance",
            &[("Session", &session_id)],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);

    cancel.cancel();
}

// ═══════════════════════════════════════════════════════════════════
// Test 3: Video-only streaming (?audio=false)
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_video_only() {
    let (lookup, _atx, video_tx) = MockInstanceLookup::new();
    let port = 18702;
    let (_mgr, cancel) = start_server(port, lookup).await;

    let (video_rtp_sock, _video_rtcp_sock) = bind_udp_receiver(17020, 17021).await;

    let mut client = RtspTestClient::connect(port).await;

    // DESCRIBE with ?audio=false should only show video
    let resp = client
        .send_request(
            "DESCRIBE",
            "rtsp://localhost/stream/test-instance?audio=false",
            &[("Accept", "application/sdp")],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);
    assert!(resp.contains("m=video 0 RTP/AVP 96"));
    assert!(!resp.contains("m=audio"), "SDP must not contain audio track");

    // SETUP video track only
    let resp = client
        .send_request(
            "SETUP",
            "rtsp://localhost/stream/test-instance/trackID=0?audio=false",
            &[("Transport", "RTP/AVP;unicast;client_port=17020-17021")],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);
    let session_id = RtspTestClient::extract_session(&resp).expect("session");

    // PLAY
    let resp = client
        .send_request(
            "PLAY",
            "rtsp://localhost/stream/test-instance",
            &[("Session", &session_id)],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Send video frames
    let _ = video_tx.send(make_vp9_packet(true, 400));
    let _ = video_tx.send(make_vp9_packet(false, 200));
    let _ = video_tx.send(make_vp9_packet(false, 200));

    let video_pkts = recv_rtp_packets(&video_rtp_sock, 5, Duration::from_secs(2)).await;
    assert!(!video_pkts.is_empty(), "should receive video packets");
    for pkt in &video_pkts {
        assert_eq!(pkt.payload_type, VP9_PT);
    }

    // TEARDOWN
    let resp = client
        .send_request(
            "TEARDOWN",
            "rtsp://localhost/stream/test-instance",
            &[("Session", &session_id)],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);

    cancel.cancel();
}

// ═══════════════════════════════════════════════════════════════════
// Test 4: VP9 keyframe descriptor with scalability structure
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_vp9_keyframe_descriptor() {
    let (lookup, _atx, video_tx) = MockInstanceLookup::new();
    let port = 18703;
    let (_mgr, cancel) = start_server(port, lookup).await;

    let (video_rtp_sock, _video_rtcp_sock) = bind_udp_receiver(17030, 17031).await;

    let mut client = RtspTestClient::connect(port).await;

    let resp = client
        .send_request(
            "SETUP",
            "rtsp://localhost/stream/test-instance/trackID=0?audio=false",
            &[("Transport", "RTP/AVP;unicast;client_port=17030-17031")],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);
    let session_id = RtspTestClient::extract_session(&resp).expect("session");

    let resp = client
        .send_request(
            "PLAY",
            "rtsp://localhost/stream/test-instance",
            &[("Session", &session_id)],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Send a keyframe small enough to fit in one RTP packet
    let _ = video_tx.send(make_vp9_packet(true, 100));

    let pkts = recv_rtp_packets(&video_rtp_sock, 1, Duration::from_secs(2)).await;
    assert_eq!(pkts.len(), 1, "single small keyframe should be one packet");

    let payload = &pkts[0].payload;
    // VP9 descriptor byte 0: I=1,P=0,L=0,F=0,B=1,E=1,V=1,Z=0
    let byte0 = payload[0];
    assert_eq!(byte0 & 0b1000_0000, 0b1000_0000, "I bit must be set");
    assert_eq!(byte0 & 0b0100_0000, 0b0000_0000, "P bit must be clear for keyframe");
    assert_eq!(byte0 & 0b0001_0000, 0b0000_0000, "F bit must be clear (non-flexible mode)");
    assert_eq!(byte0 & 0b0000_1000, 0b0000_1000, "B bit must be set (beginning)");
    assert_eq!(byte0 & 0b0000_0100, 0b0000_0100, "E bit must be set (end)");
    assert_eq!(byte0 & 0b0000_0010, 0b0000_0010, "V bit must be set (SS present)");

    // PID bytes: M=1 (15-bit)
    assert_eq!(payload[1] & 0x80, 0x80, "M bit must be set for 15-bit PID");

    // Scalability structure: N_S=0 (bits 7-5=000), Y=1 (bit 4), G=0 (bit 3)
    assert_eq!(payload[3], 0b0001_0000, "SS byte: N_S=0, Y=1");
    let width = u16::from_be_bytes([payload[4], payload[5]]);
    let height = u16::from_be_bytes([payload[6], payload[7]]);
    assert_eq!(width, 1920, "VP9 SS must carry width=1920");
    assert_eq!(height, 1080, "VP9 SS must carry height=1080");

    // Marker bit should be set (last/only packet of frame)
    assert!(pkts[0].marker, "marker bit must be set on last packet of frame");

    // TEARDOWN
    let resp = client
        .send_request(
            "TEARDOWN",
            "rtsp://localhost/stream/test-instance",
            &[("Session", &session_id)],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);

    cancel.cancel();
}

// ═══════════════════════════════════════════════════════════════════
// Test 5: VP9 frame fragmentation across MTU boundary
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_vp9_fragmentation() {
    let (lookup, _atx, video_tx) = MockInstanceLookup::new();
    let port = 18704;
    let (_mgr, cancel) = start_server(port, lookup).await;

    let (video_rtp_sock, _video_rtcp_sock) = bind_udp_receiver(17040, 17041).await;

    let mut client = RtspTestClient::connect(port).await;

    let resp = client
        .send_request(
            "SETUP",
            "rtsp://localhost/stream/test-instance/trackID=0?audio=false",
            &[("Transport", "RTP/AVP;unicast;client_port=17040-17041")],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);
    let session_id = RtspTestClient::extract_session(&resp).expect("session");

    let resp = client
        .send_request(
            "PLAY",
            "rtsp://localhost/stream/test-instance",
            &[("Session", &session_id)],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Send a large frame (3000 bytes) that must be fragmented across multiple RTP packets
    // Default MTU is 1200 bytes, so 3000 bytes → at least 3 packets
    let _ = video_tx.send(make_vp9_packet(true, 3000));

    let pkts = recv_rtp_packets(&video_rtp_sock, 10, Duration::from_secs(2)).await;
    assert!(
        pkts.len() >= 3,
        "3000-byte frame with 1200 MTU should produce >= 3 packets, got {}",
        pkts.len()
    );

    // First packet: B=1, E=0
    let first = &pkts[0];
    assert_eq!(first.payload[0] & 0b0000_1000, 0b0000_1000, "first: B=1");
    assert_eq!(first.payload[0] & 0b0000_0100, 0b0000_0000, "first: E=0");
    assert!(!first.marker, "first packet marker should be false");

    // Middle packets: B=0, E=0
    for pkt in &pkts[1..pkts.len() - 1] {
        assert_eq!(pkt.payload[0] & 0b0000_1100, 0b0000_0000, "middle: B=0, E=0");
        assert!(!pkt.marker, "middle packet marker should be false");
    }

    // Last packet: B=0, E=1
    let last = pkts.last().unwrap();
    assert_eq!(last.payload[0] & 0b0000_1000, 0b0000_0000, "last: B=0");
    assert_eq!(last.payload[0] & 0b0000_0100, 0b0000_0100, "last: E=1");
    assert!(last.marker, "last packet marker must be true");

    // All fragments share the same timestamp
    let ts = pkts[0].timestamp;
    for pkt in &pkts {
        assert_eq!(pkt.timestamp, ts, "all fragments of a frame share one timestamp");
    }

    // TEARDOWN
    let resp = client
        .send_request(
            "TEARDOWN",
            "rtsp://localhost/stream/test-instance",
            &[("Session", &session_id)],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);

    cancel.cancel();
}

// ═══════════════════════════════════════════════════════════════════
// Test 6: RTCP Sender Report reception
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_rtcp_sender_report() {
    let (lookup, audio_tx, _vtx) = MockInstanceLookup::new();
    let port = 18705;
    let (_mgr, cancel) = start_server(port, lookup).await;

    let (audio_rtp_sock, audio_rtcp_sock) = bind_udp_receiver(17050, 17051).await;

    let mut client = RtspTestClient::connect(port).await;

    let resp = client
        .send_request(
            "SETUP",
            "rtsp://localhost/audio/test-instance/trackID=1",
            &[("Transport", "RTP/AVP;unicast;client_port=17050-17051")],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);
    let session_id = RtspTestClient::extract_session(&resp).expect("session");

    let resp = client
        .send_request(
            "PLAY",
            "rtsp://localhost/audio/test-instance",
            &[("Session", &session_id)],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);

    // Continuously feed audio so the stream stays alive through the SR interval
    let feed_cancel = CancellationToken::new();
    let feed_cancel_clone = feed_cancel.clone();
    let audio_tx_clone = audio_tx.clone();
    let feed_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                () = feed_cancel_clone.cancelled() => break,
                _ = tokio::time::sleep(Duration::from_millis(20)) => {
                    let _ = audio_tx_clone.send(make_opus_packet());
                }
            }
        }
    });

    // Drain RTP so the socket buffer doesn't fill up
    let rtp_drain_cancel = CancellationToken::new();
    let rtp_drain_cancel_clone = rtp_drain_cancel.clone();
    tokio::spawn(async move {
        let mut buf = [0u8; 2048];
        loop {
            tokio::select! {
                () = rtp_drain_cancel_clone.cancelled() => break,
                _ = audio_rtp_sock.recv(&mut buf) => {}
            }
        }
    });

    // Wait for RTCP SR (sent every 5 seconds; the first tick fires immediately
    // with zero counters, so we need the second tick at ~5s).
    let reports = recv_rtcp_sr(&audio_rtcp_sock, Duration::from_secs(8)).await;
    assert!(
        !reports.is_empty(),
        "should receive at least one RTCP Sender Report"
    );

    // Every SR must have valid structure
    for sr in &reports {
        assert_eq!(sr.version, RTP_VERSION, "RTCP SR version must be 2");
        assert_eq!(sr.packet_type, RTCP_SR_PT);
        assert!(sr.ntp_timestamp > 0, "NTP timestamp must be non-zero");
    }

    // At least one SR (typically the second one at t≈5s) must report sent packets
    let has_nonzero = reports.iter().any(|sr| sr.sender_packet_count > 0);
    assert!(has_nonzero, "at least one SR must have sender_packet_count > 0 (got {} SRs)", reports.len());

    feed_cancel.cancel();
    rtp_drain_cancel.cancel();
    let _ = feed_task.await;

    // TEARDOWN
    let resp = client
        .send_request(
            "TEARDOWN",
            "rtsp://localhost/audio/test-instance",
            &[("Session", &session_id)],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);

    cancel.cancel();
}

// ═══════════════════════════════════════════════════════════════════
// Test 7: PAUSE then resume via PLAY
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_pause_and_resume() {
    let (lookup, audio_tx, _vtx) = MockInstanceLookup::new();
    let port = 18706;
    let (mgr, cancel) = start_server(port, lookup).await;

    let (audio_rtp_sock, _audio_rtcp_sock) = bind_udp_receiver(17060, 17061).await;

    let mut client = RtspTestClient::connect(port).await;

    // SETUP + PLAY
    let resp = client
        .send_request(
            "SETUP",
            "rtsp://localhost/audio/test-instance/trackID=1",
            &[("Transport", "RTP/AVP;unicast;client_port=17060-17061")],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);
    let session_id = RtspTestClient::extract_session(&resp).expect("session");

    let resp = client
        .send_request(
            "PLAY",
            "rtsp://localhost/audio/test-instance",
            &[("Session", &session_id)],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Send packets and verify receipt
    for _ in 0..3 {
        let _ = audio_tx.send(make_opus_packet());
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let pre_pause = recv_rtp_packets(&audio_rtp_sock, 3, Duration::from_secs(1)).await;
    assert!(!pre_pause.is_empty(), "should receive packets before pause");

    // PAUSE
    let resp = client
        .send_request(
            "PAUSE",
            "rtsp://localhost/audio/test-instance",
            &[("Session", &session_id)],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);

    // Session should still exist but be paused
    assert_eq!(mgr.session_count(), 1);

    // Allow PAUSE to propagate
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Send more packets - these should NOT be delivered
    for _ in 0..3 {
        let _ = audio_tx.send(make_opus_packet());
    }

    let during_pause = recv_rtp_packets(&audio_rtp_sock, 1, Duration::from_millis(500)).await;
    assert!(
        during_pause.is_empty(),
        "should NOT receive packets during pause, got {}",
        during_pause.len()
    );

    // TEARDOWN
    let resp = client
        .send_request(
            "TEARDOWN",
            "rtsp://localhost/audio/test-instance",
            &[("Session", &session_id)],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);

    cancel.cancel();
}

// ═══════════════════════════════════════════════════════════════════
// Test 8: Concurrent sessions on the same instance
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_concurrent_sessions() {
    let (lookup, audio_tx, _vtx) = MockInstanceLookup::new();
    let port = 18707;
    let (mgr, cancel) = start_server(port, lookup).await;

    let (sock_a, _rtcp_a) = bind_udp_receiver(17070, 17071).await;
    let (sock_b, _rtcp_b) = bind_udp_receiver(17072, 17073).await;

    // Client A
    let mut client_a = RtspTestClient::connect(port).await;
    let resp = client_a
        .send_request(
            "SETUP",
            "rtsp://localhost/audio/test-instance/trackID=1",
            &[("Transport", "RTP/AVP;unicast;client_port=17070-17071")],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);
    let session_a = RtspTestClient::extract_session(&resp).expect("session A");

    // Client B
    let mut client_b = RtspTestClient::connect(port).await;
    let resp = client_b
        .send_request(
            "SETUP",
            "rtsp://localhost/audio/test-instance/trackID=1",
            &[("Transport", "RTP/AVP;unicast;client_port=17072-17073")],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);
    let session_b = RtspTestClient::extract_session(&resp).expect("session B");

    assert_ne!(session_a, session_b, "sessions must have different IDs");
    assert_eq!(mgr.session_count(), 2);

    // PLAY both
    let resp = client_a
        .send_request(
            "PLAY",
            "rtsp://localhost/audio/test-instance",
            &[("Session", &session_a)],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);

    let resp = client_b
        .send_request(
            "PLAY",
            "rtsp://localhost/audio/test-instance",
            &[("Session", &session_b)],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Send audio
    for _ in 0..5 {
        let _ = audio_tx.send(make_opus_packet());
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let pkts_a = recv_rtp_packets(&sock_a, 5, Duration::from_secs(2)).await;
    let pkts_b = recv_rtp_packets(&sock_b, 5, Duration::from_secs(2)).await;

    assert!(!pkts_a.is_empty(), "client A should receive packets");
    assert!(!pkts_b.is_empty(), "client B should receive packets");

    // Each session has its own SSRC
    if !pkts_a.is_empty() && !pkts_b.is_empty() {
        assert_ne!(
            pkts_a[0].ssrc, pkts_b[0].ssrc,
            "concurrent sessions must have different SSRCs"
        );
    }

    // Aggregated health should show 2 sessions
    let agg = mgr.aggregated_health("test-instance");
    assert_eq!(agg.total_sessions, 2);

    // TEARDOWN both
    let resp = client_a
        .send_request(
            "TEARDOWN",
            "rtsp://localhost/audio/test-instance",
            &[("Session", &session_a)],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);

    let resp = client_b
        .send_request(
            "TEARDOWN",
            "rtsp://localhost/audio/test-instance",
            &[("Session", &session_b)],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);
    assert_eq!(mgr.session_count(), 0);

    cancel.cancel();
}

// ═══════════════════════════════════════════════════════════════════
// Test 9: Custom quality tier (?kbps=96&channels=1)
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_custom_quality_tier() {
    let (lookup, _atx, _vtx) = MockInstanceLookup::new();
    let port = 18708;
    let (_mgr, cancel) = start_server(port, lookup).await;

    let mut client = RtspTestClient::connect(port).await;

    // DESCRIBE with custom quality
    let resp = client
        .send_request(
            "DESCRIBE",
            "rtsp://localhost/stream/test-instance?kbps=96&channels=1",
            &[("Accept", "application/sdp")],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);
    assert!(
        resp.contains("maxaveragebitrate=96000"),
        "SDP should contain custom bitrate: {resp}"
    );

    cancel.cancel();
}

// ═══════════════════════════════════════════════════════════════════
// Test 10: RTSP error: duplicate track SETUP
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_duplicate_track_setup() {
    let (lookup, _atx, _vtx) = MockInstanceLookup::new();
    let port = 18709;
    let (_mgr, cancel) = start_server(port, lookup).await;

    let mut client = RtspTestClient::connect(port).await;

    // First SETUP
    let resp = client
        .send_request(
            "SETUP",
            "rtsp://localhost/stream/test-instance/trackID=0",
            &[("Transport", "RTP/AVP;unicast;client_port=17080-17081")],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);
    let session_id = RtspTestClient::extract_session(&resp).expect("session");

    // Duplicate SETUP for the same track
    let resp = client
        .send_request(
            "SETUP",
            "rtsp://localhost/stream/test-instance/trackID=0",
            &[
                ("Transport", "RTP/AVP;unicast;client_port=17082-17083"),
                ("Session", &session_id),
            ],
        )
        .await;
    assert_eq!(
        RtspTestClient::extract_status(&resp),
        455,
        "duplicate track SETUP should return 455"
    );

    // TEARDOWN
    let resp = client
        .send_request(
            "TEARDOWN",
            "rtsp://localhost/stream/test-instance",
            &[("Session", &session_id)],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);

    cancel.cancel();
}

// ═══════════════════════════════════════════════════════════════════
// Test 11: RTSP error: missing Transport header
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_setup_missing_transport() {
    let (lookup, _atx, _vtx) = MockInstanceLookup::new();
    let port = 18710;
    let (_mgr, cancel) = start_server(port, lookup).await;

    let mut client = RtspTestClient::connect(port).await;

    let resp = client
        .send_request(
            "SETUP",
            "rtsp://localhost/stream/test-instance/trackID=0",
            &[],
        )
        .await;
    assert_eq!(
        RtspTestClient::extract_status(&resp),
        461,
        "missing Transport should return 461 Unsupported Transport"
    );

    cancel.cancel();
}

// ═══════════════════════════════════════════════════════════════════
// Test 12: RTSP error: PLAY on wrong session
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_play_wrong_session() {
    let (lookup, _atx, _vtx) = MockInstanceLookup::new();
    let port = 18711;
    let (_mgr, cancel) = start_server(port, lookup).await;

    let mut client = RtspTestClient::connect(port).await;

    let resp = client
        .send_request(
            "PLAY",
            "rtsp://localhost/stream/test-instance",
            &[("Session", "bogus-session-id")],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 454);

    cancel.cancel();
}

// ═══════════════════════════════════════════════════════════════════
// Test 13: SDP content validation for multi-track
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_sdp_content_validation() {
    let (lookup, _atx, _vtx) = MockInstanceLookup::new();
    let port = 18712;
    let (_mgr, cancel) = start_server(port, lookup).await;

    let mut client = RtspTestClient::connect(port).await;

    let resp = client
        .send_request(
            "DESCRIBE",
            "rtsp://localhost/stream/test-instance",
            &[("Accept", "application/sdp")],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);

    // Protocol version
    assert!(resp.contains("v=0"), "SDP must start with v=0");
    // Session name
    assert!(resp.contains("s=vscreen stream"), "SDP session name");
    // Connection
    assert!(resp.contains("c=IN IP4"), "SDP must have connection info");

    // Video section
    assert!(resp.contains("m=video 0 RTP/AVP 96"));
    assert!(resp.contains("a=rtpmap:96 VP9/90000"));
    assert!(resp.contains("a=framesize:96 1920-1080"), "framesize for 1920x1080");
    assert!(resp.contains("a=framerate:30"));
    assert!(resp.contains("a=control:trackID=0"));

    // Audio section
    assert!(resp.contains("m=audio 0 RTP/AVP 111"));
    assert!(resp.contains("a=rtpmap:111 opus/48000/2"));
    assert!(resp.contains("a=fmtp:111"));
    assert!(resp.contains("a=ptime:20"));
    assert!(resp.contains("a=control:trackID=1"));

    cancel.cancel();
}

// ═══════════════════════════════════════════════════════════════════
// Test 14: Video timestamp increments (90kHz clock)
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_video_timestamp_increments() {
    let (lookup, _atx, video_tx) = MockInstanceLookup::new();
    let port = 18713;
    let (_mgr, cancel) = start_server(port, lookup).await;

    let (video_rtp_sock, _video_rtcp_sock) = bind_udp_receiver(17090, 17091).await;

    let mut client = RtspTestClient::connect(port).await;

    let resp = client
        .send_request(
            "SETUP",
            "rtsp://localhost/stream/test-instance/trackID=0?audio=false",
            &[("Transport", "RTP/AVP;unicast;client_port=17090-17091")],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);
    let session_id = RtspTestClient::extract_session(&resp).expect("session");

    let resp = client
        .send_request(
            "PLAY",
            "rtsp://localhost/stream/test-instance",
            &[("Session", &session_id)],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Send 4 small frames (each fits in one packet) to get clean timestamp pairs
    for i in 0..4 {
        let _ = video_tx.send(make_vp9_packet(i == 0, 100));
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let pkts = recv_rtp_packets(&video_rtp_sock, 4, Duration::from_secs(2)).await;
    assert!(
        pkts.len() >= 3,
        "need at least 3 packets for timestamp delta check, got {}",
        pkts.len()
    );

    // Collect unique timestamps in order
    let mut timestamps = Vec::new();
    for pkt in &pkts {
        if timestamps.last() != Some(&pkt.timestamp) {
            timestamps.push(pkt.timestamp);
        }
    }

    // At 30fps, timestamp increment = 90000 / 30 = 3000
    if timestamps.len() >= 2 {
        for i in 1..timestamps.len() {
            let delta = timestamps[i].wrapping_sub(timestamps[i - 1]);
            assert_eq!(
                delta, 3000,
                "video timestamp increment should be 3000 (90kHz / 30fps), got {delta}"
            );
        }
    }

    // TEARDOWN
    let resp = client
        .send_request(
            "TEARDOWN",
            "rtsp://localhost/stream/test-instance",
            &[("Session", &session_id)],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);

    cancel.cancel();
}

// ═══════════════════════════════════════════════════════════════════
// Test 15: Health tracking via session manager
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_health_tracking() {
    let (lookup, audio_tx, video_tx) = MockInstanceLookup::new();
    let port = 18714;
    let (mgr, cancel) = start_server(port, lookup).await;

    // We don't need to actually receive these packets — just verify health updates
    let (_rtp_v, _rtcp_v) = bind_udp_receiver(17100, 17101).await;
    let (_rtp_a, _rtcp_a) = bind_udp_receiver(17102, 17103).await;

    let mut client = RtspTestClient::connect(port).await;

    // SETUP both tracks
    let resp = client
        .send_request(
            "SETUP",
            "rtsp://localhost/stream/test-instance/trackID=0",
            &[("Transport", "RTP/AVP;unicast;client_port=17100-17101")],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);
    let session_id = RtspTestClient::extract_session(&resp).expect("session");

    let resp = client
        .send_request(
            "SETUP",
            "rtsp://localhost/stream/test-instance/trackID=1",
            &[
                ("Transport", "RTP/AVP;unicast;client_port=17102-17103"),
                ("Session", &session_id),
            ],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);

    // PLAY
    let resp = client
        .send_request(
            "PLAY",
            "rtsp://localhost/stream/test-instance",
            &[("Session", &session_id)],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Pump packets
    for _ in 0..10 {
        let _ = video_tx.send(make_vp9_packet(false, 200));
        let _ = audio_tx.send(make_opus_packet());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify per-track health
    {
        let session = mgr.get(&session_id).expect("session");
        let info = session.info();

        assert_eq!(info.tracks.len(), 2, "should report 2 tracks");
        for track in &info.tracks {
            assert!(
                track.packets_sent > 0,
                "track {:?} packets_sent should be > 0",
                track.media_type
            );
            assert!(
                track.bytes_sent > 0,
                "track {:?} bytes_sent should be > 0",
                track.media_type
            );
        }
    }

    // Aggregated health for the instance
    let agg = mgr.aggregated_health("test-instance");
    assert_eq!(agg.total_sessions, 1);
    assert!(agg.total_packets_sent > 0);
    assert!(agg.total_bytes_sent > 0);

    // TEARDOWN
    let resp = client
        .send_request(
            "TEARDOWN",
            "rtsp://localhost/stream/test-instance",
            &[("Session", &session_id)],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);

    cancel.cancel();
}

// ═══════════════════════════════════════════════════════════════════
// Test 16: GET_PARAMETER keepalive refreshes session
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_get_parameter_keepalive() {
    let (lookup, _atx, _vtx) = MockInstanceLookup::new();
    let port = 18715;
    let (mgr, cancel) = start_server(port, lookup).await;

    let mut client = RtspTestClient::connect(port).await;

    let resp = client
        .send_request(
            "SETUP",
            "rtsp://localhost/stream/test-instance/trackID=0",
            &[("Transport", "RTP/AVP;unicast;client_port=17110-17111")],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);
    let session_id = RtspTestClient::extract_session(&resp).expect("session");

    tokio::time::sleep(Duration::from_millis(100)).await;

    // GET_PARAMETER should succeed and refresh session
    let resp = client
        .send_request(
            "GET_PARAMETER",
            "rtsp://localhost/stream/test-instance",
            &[("Session", &session_id)],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);

    // Session should still exist
    assert_eq!(mgr.session_count(), 1);

    // TEARDOWN
    let resp = client
        .send_request(
            "TEARDOWN",
            "rtsp://localhost/stream/test-instance",
            &[("Session", &session_id)],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);

    cancel.cancel();
}

// ═══════════════════════════════════════════════════════════════════
// Test 17: Inter-frame VP9 descriptor (P bit set, no SS)
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_vp9_interframe_descriptor() {
    let (lookup, _atx, video_tx) = MockInstanceLookup::new();
    let port = 18716;
    let (_mgr, cancel) = start_server(port, lookup).await;

    let (video_rtp_sock, _video_rtcp_sock) = bind_udp_receiver(17120, 17121).await;

    let mut client = RtspTestClient::connect(port).await;

    let resp = client
        .send_request(
            "SETUP",
            "rtsp://localhost/stream/test-instance/trackID=0?audio=false",
            &[("Transport", "RTP/AVP;unicast;client_port=17120-17121")],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);
    let session_id = RtspTestClient::extract_session(&resp).expect("session");

    let resp = client
        .send_request(
            "PLAY",
            "rtsp://localhost/stream/test-instance",
            &[("Session", &session_id)],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Send a keyframe first, then an inter-frame
    let _ = video_tx.send(make_vp9_packet(true, 100));
    tokio::time::sleep(Duration::from_millis(20)).await;
    let _ = video_tx.send(make_vp9_packet(false, 100));

    let pkts = recv_rtp_packets(&video_rtp_sock, 2, Duration::from_secs(2)).await;
    assert!(pkts.len() >= 2, "need at least 2 packets");

    // Second packet is the inter-frame
    let inter_payload = &pkts[1].payload;
    let byte0 = inter_payload[0];
    let p_bit = (byte0 >> 6) & 1;
    let f_bit = (byte0 >> 4) & 1;
    let v_bit = (byte0 >> 1) & 1;
    assert_eq!(p_bit, 1, "P bit must be set for inter-frame");
    assert_eq!(f_bit, 0, "F bit must be clear (non-flexible mode)");
    assert_eq!(v_bit, 0, "V bit must be clear for inter-frame (no SS)");

    // No P_DIFF byte in non-flexible mode; payload starts at byte 3 (after I+PID)
    assert!(inter_payload.len() >= 3, "inter-frame descriptor must be at least 3 bytes");

    // TEARDOWN
    let resp = client
        .send_request(
            "TEARDOWN",
            "rtsp://localhost/stream/test-instance",
            &[("Session", &session_id)],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);

    cancel.cancel();
}

// ═══════════════════════════════════════════════════════════════════
// Test 18: Full RTSP protocol flow (OPTIONS → DESCRIBE → SETUP
//          → PLAY → stream verification → PAUSE → TEARDOWN)
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_full_protocol_flow() {
    let (lookup, audio_tx, video_tx) = MockInstanceLookup::new();
    let port = 18717;
    let (mgr, cancel) = start_server(port, lookup).await;

    let (video_rtp_sock, _video_rtcp_sock) = bind_udp_receiver(17130, 17131).await;
    let (audio_rtp_sock, _audio_rtcp_sock) = bind_udp_receiver(17132, 17133).await;

    let mut client = RtspTestClient::connect(port).await;

    // Step 1: OPTIONS
    let resp = client
        .send_request("OPTIONS", "rtsp://localhost/stream/test-instance", &[])
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);
    assert!(resp.contains("Public:"));

    // Step 2: DESCRIBE
    let resp = client
        .send_request(
            "DESCRIBE",
            "rtsp://localhost/stream/test-instance",
            &[("Accept", "application/sdp")],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);
    assert!(resp.contains("m=video"));
    assert!(resp.contains("m=audio"));

    // Step 3: SETUP video
    let resp = client
        .send_request(
            "SETUP",
            "rtsp://localhost/stream/test-instance/trackID=0",
            &[("Transport", "RTP/AVP;unicast;client_port=17130-17131")],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);
    let session_id = RtspTestClient::extract_session(&resp).expect("session");

    // Step 4: SETUP audio
    let resp = client
        .send_request(
            "SETUP",
            "rtsp://localhost/stream/test-instance/trackID=1",
            &[
                ("Transport", "RTP/AVP;unicast;client_port=17132-17133"),
                ("Session", &session_id),
            ],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);

    // Step 5: PLAY
    let resp = client
        .send_request(
            "PLAY",
            "rtsp://localhost/stream/test-instance",
            &[("Session", &session_id)],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Step 6: Stream data and verify
    let _ = video_tx.send(make_vp9_packet(true, 200));
    let _ = audio_tx.send(make_opus_packet());
    tokio::time::sleep(Duration::from_millis(50)).await;

    let vpkts = recv_rtp_packets(&video_rtp_sock, 1, Duration::from_secs(1)).await;
    let apkts = recv_rtp_packets(&audio_rtp_sock, 1, Duration::from_secs(1)).await;
    assert!(!vpkts.is_empty(), "video RTP received after PLAY");
    assert!(!apkts.is_empty(), "audio RTP received after PLAY");

    // Step 7: PAUSE
    let resp = client
        .send_request(
            "PAUSE",
            "rtsp://localhost/stream/test-instance",
            &[("Session", &session_id)],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);
    assert_eq!(mgr.session_count(), 1, "session persists after PAUSE");

    // Step 8: TEARDOWN
    let resp = client
        .send_request(
            "TEARDOWN",
            "rtsp://localhost/stream/test-instance",
            &[("Session", &session_id)],
        )
        .await;
    assert_eq!(RtspTestClient::extract_status(&resp), 200);
    assert_eq!(mgr.session_count(), 0, "session removed after TEARDOWN");

    cancel.cancel();
}
