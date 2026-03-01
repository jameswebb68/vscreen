use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use vscreen_core::frame::EncodedPacket;

use vscreen_rtsp::health::HealthState;
use vscreen_rtsp::quality::QualityTier;
use vscreen_rtsp::server::RtspServer;
use vscreen_rtsp::session::{MediaConfig, RtspSessionManager};
use vscreen_rtsp::transcoder::OpusTranscoder;

// ───────────────────── Mock Instance Lookup ───────────────────────

struct MockInstanceLookup {
    audio_tx: broadcast::Sender<EncodedPacket>,
    video_tx: broadcast::Sender<EncodedPacket>,
}

impl MockInstanceLookup {
    fn new() -> (Self, broadcast::Sender<EncodedPacket>, broadcast::Sender<EncodedPacket>) {
        let (audio_tx, _) = broadcast::channel::<EncodedPacket>(16);
        let (video_tx, _) = broadcast::channel::<EncodedPacket>(16);
        let atx = audio_tx.clone();
        let vtx = video_tx.clone();
        (Self { audio_tx, video_tx }, atx, vtx)
    }
}

impl vscreen_rtsp::InstanceLookup for MockInstanceLookup {
    fn instance_exists(&self, instance_id: &str) -> bool {
        instance_id == "test-instance"
    }

    fn subscribe_audio(
        &self,
        instance_id: &str,
    ) -> Option<broadcast::Receiver<EncodedPacket>> {
        if instance_id == "test-instance" {
            Some(self.audio_tx.subscribe())
        } else {
            None
        }
    }

    fn subscribe_video(
        &self,
        instance_id: &str,
    ) -> Option<broadcast::Receiver<EncodedPacket>> {
        if instance_id == "test-instance" {
            Some(self.video_tx.subscribe())
        } else {
            None
        }
    }

    fn video_resolution(&self, instance_id: &str) -> Option<(u32, u32)> {
        if instance_id == "test-instance" {
            Some((1920, 1080))
        } else {
            None
        }
    }

    fn video_framerate(&self, instance_id: &str) -> Option<u32> {
        if instance_id == "test-instance" {
            Some(30)
        } else {
            None
        }
    }

    fn request_keyframe(&self, _instance_id: &str) {}

    fn video_codec(&self, id: &str) -> vscreen_core::frame::VideoCodec {
        let _ = id;
        vscreen_core::frame::VideoCodec::Vp9
    }
}

fn make_test_opus_packet() -> EncodedPacket {
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

fn make_test_vp9_packet(keyframe: bool) -> EncodedPacket {
    EncodedPacket {
        data: Bytes::from(vec![0xAA; 500]),
        is_keyframe: keyframe,
        pts: 0,
        duration: Some(33),
        codec: None,
    }
}


// ───────────────── RTSP Client Helper ─────────────────────────────

struct RtspTestClient {
    reader: BufReader<tokio::net::tcp::OwnedReadHalf>,
    writer: tokio::net::tcp::OwnedWriteHalf,
    cseq: u32,
}

impl RtspTestClient {
    async fn connect(port: u16) -> Self {
        let stream = TcpStream::connect(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            port,
        ))
        .await
        .expect("connect to RTSP server");

        let (reader, writer) = stream.into_split();
        Self {
            reader: BufReader::new(reader),
            writer,
            cseq: 0,
        }
    }

    async fn send_request(&mut self, method: &str, url: &str, extra_headers: &[(&str, &str)]) -> String {
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

    fn extract_status(&self, response: &str) -> u16 {
        response
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    }

    fn extract_session(&self, response: &str) -> Option<String> {
        response
            .lines()
            .find(|l| l.to_lowercase().starts_with("session:"))
            .and_then(|l| l.split(':').nth(1))
            .map(|v| v.trim().split(';').next().unwrap_or("").to_owned())
    }
}

// ───────────────── Tests ──────────────────────────────────────────

#[tokio::test]
async fn test_rtsp_options() {
    let cancel = CancellationToken::new();
    let (lookup, _atx, _vtx) = MockInstanceLookup::new();
    let port = 18600;

    let server = RtspServer::new(
        port,
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        Arc::new(lookup),
        cancel.clone(),
    );

    tokio::spawn(async move {
        let _ = server.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RtspTestClient::connect(port).await;
    let response = client
        .send_request("OPTIONS", "rtsp://localhost/stream/test-instance", &[])
        .await;

    assert_eq!(client.extract_status(&response), 200);
    assert!(response.contains("Public:"));
    assert!(response.contains("DESCRIBE"));
    assert!(response.contains("SETUP"));
    assert!(response.contains("PLAY"));

    cancel.cancel();
}

#[tokio::test]
async fn test_rtsp_describe_stream_both_tracks() {
    let cancel = CancellationToken::new();
    let (lookup, _atx, _vtx) = MockInstanceLookup::new();
    let port = 18601;

    let server = RtspServer::new(
        port,
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        Arc::new(lookup),
        cancel.clone(),
    );

    tokio::spawn(async move {
        let _ = server.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RtspTestClient::connect(port).await;
    let response = client
        .send_request(
            "DESCRIBE",
            "rtsp://localhost/stream/test-instance",
            &[("Accept", "application/sdp")],
        )
        .await;

    assert_eq!(client.extract_status(&response), 200);
    assert!(response.contains("Content-Type: application/sdp"));
    assert!(response.contains("v=0"));
    assert!(response.contains("m=video 0 RTP/AVP 96"));
    assert!(response.contains("VP9/90000"));
    assert!(response.contains("a=control:trackID=0"));
    assert!(response.contains("m=audio 0 RTP/AVP 111"));
    assert!(response.contains("opus/48000/2"));
    assert!(response.contains("a=control:trackID=1"));
    assert!(response.contains("test-instance"));

    cancel.cancel();
}

#[tokio::test]
async fn test_rtsp_describe_audio_only_legacy() {
    let cancel = CancellationToken::new();
    let (lookup, _atx, _vtx) = MockInstanceLookup::new();
    let port = 18620;

    let server = RtspServer::new(
        port,
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        Arc::new(lookup),
        cancel.clone(),
    );

    tokio::spawn(async move {
        let _ = server.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RtspTestClient::connect(port).await;
    let response = client
        .send_request(
            "DESCRIBE",
            "rtsp://localhost/audio/test-instance",
            &[("Accept", "application/sdp")],
        )
        .await;

    assert_eq!(client.extract_status(&response), 200);
    assert!(!response.contains("m=video"));
    assert!(response.contains("m=audio 0 RTP/AVP 111"));

    cancel.cancel();
}

#[tokio::test]
async fn test_rtsp_describe_video_disabled() {
    let cancel = CancellationToken::new();
    let (lookup, _atx, _vtx) = MockInstanceLookup::new();
    let port = 18621;

    let server = RtspServer::new(
        port,
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        Arc::new(lookup),
        cancel.clone(),
    );

    tokio::spawn(async move {
        let _ = server.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RtspTestClient::connect(port).await;
    let response = client
        .send_request(
            "DESCRIBE",
            "rtsp://localhost/stream/test-instance?video=false",
            &[("Accept", "application/sdp")],
        )
        .await;

    assert_eq!(client.extract_status(&response), 200);
    assert!(!response.contains("m=video"));
    assert!(response.contains("m=audio 0 RTP/AVP 111"));

    cancel.cancel();
}

#[tokio::test]
async fn test_rtsp_describe_nonexistent_instance() {
    let cancel = CancellationToken::new();
    let (lookup, _atx, _vtx) = MockInstanceLookup::new();
    let port = 18602;

    let server = RtspServer::new(
        port,
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        Arc::new(lookup),
        cancel.clone(),
    );

    tokio::spawn(async move {
        let _ = server.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RtspTestClient::connect(port).await;
    let response = client
        .send_request(
            "DESCRIBE",
            "rtsp://localhost/stream/nonexistent",
            &[("Accept", "application/sdp")],
        )
        .await;

    assert_eq!(client.extract_status(&response), 404);

    cancel.cancel();
}

#[tokio::test]
async fn test_rtsp_setup_and_play_audio() {
    let cancel = CancellationToken::new();
    let (lookup, audio_tx, _vtx) = MockInstanceLookup::new();
    let port = 18603;

    let server = RtspServer::new(
        port,
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        Arc::new(lookup),
        cancel.clone(),
    );
    let session_mgr = server.session_manager().clone();

    tokio::spawn(async move {
        let _ = server.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RtspTestClient::connect(port).await;

    // SETUP audio track
    let response = client
        .send_request(
            "SETUP",
            "rtsp://localhost/audio/test-instance/trackID=1",
            &[("Transport", "RTP/AVP;unicast;client_port=15000-15001")],
        )
        .await;

    assert_eq!(client.extract_status(&response), 200);
    assert!(response.contains("Transport:"));
    assert!(response.contains("server_port="));

    let session_id = client
        .extract_session(&response)
        .expect("session ID in SETUP response");

    assert_eq!(session_mgr.session_count(), 1);

    // PLAY
    let response = client
        .send_request(
            "PLAY",
            "rtsp://localhost/audio/test-instance",
            &[("Session", &session_id)],
        )
        .await;

    assert_eq!(client.extract_status(&response), 200);

    // Send a test audio packet
    let test_pkt = make_test_opus_packet();
    let _ = audio_tx.send(test_pkt);

    tokio::time::sleep(Duration::from_millis(100)).await;

    // TEARDOWN
    let response = client
        .send_request(
            "TEARDOWN",
            "rtsp://localhost/audio/test-instance",
            &[("Session", &session_id)],
        )
        .await;

    assert_eq!(client.extract_status(&response), 200);
    assert_eq!(session_mgr.session_count(), 0);

    cancel.cancel();
}

#[tokio::test]
async fn test_rtsp_multi_track_setup_play() {
    let cancel = CancellationToken::new();
    let (lookup, audio_tx, video_tx) = MockInstanceLookup::new();
    let port = 18622;

    let server = RtspServer::new(
        port,
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        Arc::new(lookup),
        cancel.clone(),
    );
    let session_mgr = server.session_manager().clone();

    tokio::spawn(async move {
        let _ = server.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RtspTestClient::connect(port).await;

    // SETUP video track (first SETUP creates session)
    let response = client
        .send_request(
            "SETUP",
            "rtsp://localhost/stream/test-instance/trackID=0",
            &[("Transport", "RTP/AVP;unicast;client_port=16000-16001")],
        )
        .await;

    assert_eq!(client.extract_status(&response), 200);
    let session_id = client
        .extract_session(&response)
        .expect("session ID in first SETUP");

    // SETUP audio track (same session)
    let response = client
        .send_request(
            "SETUP",
            "rtsp://localhost/stream/test-instance/trackID=1",
            &[
                ("Transport", "RTP/AVP;unicast;client_port=16002-16003"),
                ("Session", &session_id),
            ],
        )
        .await;

    assert_eq!(client.extract_status(&response), 200);
    assert_eq!(session_mgr.session_count(), 1);

    // Verify session has 2 tracks
    {
        let session = session_mgr.get(&session_id).expect("session exists");
        assert_eq!(session.tracks.len(), 2);
    }

    // PLAY
    let response = client
        .send_request(
            "PLAY",
            "rtsp://localhost/stream/test-instance",
            &[("Session", &session_id)],
        )
        .await;

    assert_eq!(client.extract_status(&response), 200);

    // Send test packets
    let _ = video_tx.send(make_test_vp9_packet(true));
    let _ = audio_tx.send(make_test_opus_packet());

    tokio::time::sleep(Duration::from_millis(100)).await;

    // TEARDOWN
    let response = client
        .send_request(
            "TEARDOWN",
            "rtsp://localhost/stream/test-instance",
            &[("Session", &session_id)],
        )
        .await;

    assert_eq!(client.extract_status(&response), 200);
    assert_eq!(session_mgr.session_count(), 0);

    cancel.cancel();
}

#[tokio::test]
async fn test_rtsp_session_not_found() {
    let cancel = CancellationToken::new();
    let (lookup, _atx, _vtx) = MockInstanceLookup::new();
    let port = 18604;

    let server = RtspServer::new(
        port,
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        Arc::new(lookup),
        cancel.clone(),
    );

    tokio::spawn(async move {
        let _ = server.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RtspTestClient::connect(port).await;

    let response = client
        .send_request(
            "PLAY",
            "rtsp://localhost/stream/test-instance",
            &[("Session", "nonexistent-session-id")],
        )
        .await;

    assert_eq!(client.extract_status(&response), 454);

    cancel.cancel();
}

#[tokio::test]
async fn test_rtsp_get_parameter_keepalive() {
    let cancel = CancellationToken::new();
    let (lookup, _atx, _vtx) = MockInstanceLookup::new();
    let port = 18605;

    let server = RtspServer::new(
        port,
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        Arc::new(lookup),
        cancel.clone(),
    );

    tokio::spawn(async move {
        let _ = server.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RtspTestClient::connect(port).await;

    let response = client
        .send_request(
            "GET_PARAMETER",
            "rtsp://localhost/stream/test-instance",
            &[],
        )
        .await;

    assert_eq!(client.extract_status(&response), 200);

    cancel.cancel();
}

#[tokio::test]
async fn test_rtsp_describe_with_quality_tier() {
    let cancel = CancellationToken::new();
    let (lookup, _atx, _vtx) = MockInstanceLookup::new();
    let port = 18606;

    let server = RtspServer::new(
        port,
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        Arc::new(lookup),
        cancel.clone(),
    );

    tokio::spawn(async move {
        let _ = server.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RtspTestClient::connect(port).await;

    let response = client
        .send_request(
            "DESCRIBE",
            "rtsp://localhost/stream/test-instance?tier=low",
            &[("Accept", "application/sdp")],
        )
        .await;

    assert_eq!(client.extract_status(&response), 200);
    assert!(response.contains("maxaveragebitrate=32000"));

    cancel.cancel();
}

// ───────── Session Manager Tests ──────────────────────────────────

#[test]
fn test_session_manager_aggregated_health() {
    let cancel = CancellationToken::new();
    let mgr = RtspSessionManager::new(cancel);

    mgr.create_session(
        vscreen_core::instance::InstanceId::from("inst1"),
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 5000),
        MediaConfig::default(),
        QualityTier::Standard,
    );
    mgr.create_session(
        vscreen_core::instance::InstanceId::from("inst1"),
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 5002),
        MediaConfig::default(),
        QualityTier::Low,
    );

    let health = mgr.aggregated_health("inst1");
    assert_eq!(health.total_sessions, 2);
}

// ───────── Transcoder Tests ───────────────────────────────────────

#[test]
fn test_transcoder_all_tiers() {
    let master_pkt = make_test_opus_packet();

    for tier in [QualityTier::Low, QualityTier::Medium, QualityTier::Standard] {
        let mut tc = OpusTranscoder::new(tier).expect("init transcoder");
        let result = tc.transcode(&master_pkt).expect("transcode");
        assert!(!result.data.is_empty(), "tier {tier} produced empty output");
    }
}

#[test]
fn test_transcoder_custom_bitrate() {
    let master_pkt = make_test_opus_packet();
    let tier = QualityTier::custom(96, Some(1));
    let mut tc = OpusTranscoder::new(tier).expect("init");
    let result = tc.transcode(&master_pkt).expect("transcode");
    assert!(!result.data.is_empty());
}

// ───────── Health Watchdog Tests ──────────────────────────────────

#[test]
fn test_health_evaluation_stale() {
    let mut health = vscreen_rtsp::StreamHealth::default();
    // Grace period for never-sent streams is 30s
    health.created_at = std::time::Instant::now() - Duration::from_secs(35);
    // Staleness only produces Degraded, never Failed (static screens are normal)
    health.evaluate();
    assert_eq!(health.state, HealthState::Degraded);
    health.evaluate();
    health.evaluate();
    health.evaluate();
    // Still Degraded even after many consecutive idle evaluations
    assert_eq!(health.state, HealthState::Degraded);
}

#[test]
fn test_health_failed_on_severe_packet_loss() {
    let mut health = vscreen_rtsp::StreamHealth::default();
    health.record_packet_sent(100);
    // Client reports >50% packet loss via RTCP RR
    health.update_from_rtcp_rr(0.55, 10.0);
    health.evaluate();
    assert_eq!(health.state, HealthState::Failed);
}

#[test]
fn test_health_evaluation_high_jitter() {
    let mut health = vscreen_rtsp::StreamHealth::default();
    health.record_packet_sent(100);
    health.update_from_rtcp_rr(0.02, 150.0);
    health.evaluate();
    assert_eq!(health.state, HealthState::Degraded);
}

// ───────── Session Timeout Tests ──────────────────────────────────

#[tokio::test]
async fn test_session_timeout_reaper() {
    let cancel = CancellationToken::new();
    let mgr = Arc::new(RtspSessionManager::with_timeout(
        cancel.clone(),
        Duration::from_millis(50),
    ));

    let _sid = mgr.create_session(
        vscreen_core::instance::InstanceId::from("test"),
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 5000),
        MediaConfig::default(),
        QualityTier::Standard,
    );

    assert_eq!(mgr.session_count(), 1);

    tokio::time::sleep(Duration::from_millis(100)).await;

    let expired: Vec<String> = mgr
        .all_sessions()
        .iter()
        .filter(|s| {
            mgr.get(&s.session_id)
                .map_or(false, |sess| sess.is_expired(Duration::from_millis(50)))
        })
        .map(|s| s.session_id.clone())
        .collect();

    for session_id in expired {
        mgr.remove(&session_id);
    }

    assert_eq!(mgr.session_count(), 0);

    cancel.cancel();
}
