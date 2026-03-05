use std::net::IpAddr;

use vscreen_core::frame::VideoCodec;

use crate::quality::QualityTier;
use crate::session::MediaConfig;

/// Generate an SDP description for a media stream.
///
/// The SDP follows RFC 4566 and describes video (VP9 or H.264) and/or audio
/// (Opus) tracks suitable for RTP delivery per the RTSP DESCRIBE response.
#[must_use]
pub fn generate_sdp(params: &SdpParams) -> String {
    let ip = params.server_ip;
    let ip_ver = if ip.is_ipv6() { "IP6" } else { "IP4" };

    let mut sdp = String::with_capacity(512);

    // Session-level fields
    sdp.push_str("v=0\r\n");
    sdp.push_str(&format!(
        "o=vscreen {session_id} 1 IN {ip_ver} {ip}\r\n",
        session_id = params.session_version,
    ));
    sdp.push_str(&format!(
        "s=vscreen stream ({instance_id})\r\n",
        instance_id = params.instance_id,
    ));
    sdp.push_str(&format!("c=IN {ip_ver} {ip}\r\n"));
    sdp.push_str("t=0 0\r\n");

    // Video track (trackID=0)
    if params.media.video {
        sdp.push_str("m=video 0 RTP/AVP 96\r\n");

        match params.video_codec {
            VideoCodec::Vp9 => {
                sdp.push_str("a=rtpmap:96 VP9/90000\r\n");
                sdp.push_str(&format!(
                    "a=framesize:96 {}-{}\r\n",
                    params.video_width, params.video_height,
                ));
            }
            VideoCodec::H264 => {
                sdp.push_str("a=rtpmap:96 H264/90000\r\n");
                // Baseline profile, level 3.1 (1080p30)
                sdp.push_str("a=fmtp:96 profile-level-id=42c01f;packetization-mode=1\r\n");
            }
        }

        sdp.push_str(&format!("a=framerate:{}\r\n", params.framerate));
        sdp.push_str("a=control:trackID=0\r\n");
    }

    // Audio track (trackID=1)
    if params.media.audio {
        let bitrate = params.tier.bitrate_bps();
        sdp.push_str("m=audio 0 RTP/AVP 111\r\n");
        sdp.push_str("a=rtpmap:111 opus/48000/2\r\n");
        sdp.push_str(&format!(
            "a=fmtp:111 minptime=10;useinbandfec=1;maxaveragebitrate={bitrate}\r\n",
        ));
        sdp.push_str(&format!("a=ptime:{}\r\n", params.ptime_ms));
        sdp.push_str("a=control:trackID=1\r\n");
    }

    sdp
}

/// Parameters for SDP generation.
#[derive(Debug)]
pub struct SdpParams<'a> {
    pub instance_id: &'a str,
    pub server_ip: IpAddr,
    pub session_version: u64,
    pub tier: QualityTier,
    /// Packet time in milliseconds (typically 20).
    pub ptime_ms: u32,
    /// Which media tracks to include.
    pub media: MediaConfig,
    /// Video codec.
    pub video_codec: VideoCodec,
    /// Video width in pixels.
    pub video_width: u32,
    /// Video height in pixels.
    pub video_height: u32,
    /// Video framerate in fps.
    pub framerate: u32,
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    fn default_params() -> SdpParams<'static> {
        SdpParams {
            instance_id: "dev",
            server_ip: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)),
            session_version: 1,
            tier: QualityTier::Standard,
            ptime_ms: 20,
            media: MediaConfig::default(),
            video_codec: VideoCodec::Vp9,
            video_width: 1920,
            video_height: 1080,
            framerate: 30,
        }
    }

    #[test]
    fn generates_vp9_video_and_audio_sdp() {
        let sdp = generate_sdp(&default_params());

        assert!(sdp.starts_with("v=0\r\n"));
        assert!(sdp.contains("s=vscreen stream (dev)"));
        // Video track
        assert!(sdp.contains("m=video 0 RTP/AVP 96"));
        assert!(sdp.contains("a=rtpmap:96 VP9/90000"));
        assert!(sdp.contains("a=framesize:96 1920-1080"));
        assert!(sdp.contains("a=framerate:30"));
        assert!(sdp.contains("a=control:trackID=0"));
        // Audio track
        assert!(sdp.contains("m=audio 0 RTP/AVP 111"));
        assert!(sdp.contains("a=rtpmap:111 opus/48000/2"));
        assert!(sdp.contains("maxaveragebitrate=128000"));
        assert!(sdp.contains("a=ptime:20"));
        assert!(sdp.contains("a=control:trackID=1"));
    }

    #[test]
    fn generates_h264_video_sdp() {
        let mut params = default_params();
        params.video_codec = VideoCodec::H264;
        let sdp = generate_sdp(&params);

        assert!(sdp.contains("a=rtpmap:96 H264/90000"));
        assert!(sdp.contains("a=fmtp:96 profile-level-id=42c01f;packetization-mode=1"));
        assert!(!sdp.contains("VP9"));
        assert!(!sdp.contains("a=framesize"));
    }

    #[test]
    fn audio_only_sdp() {
        let mut params = default_params();
        params.media = MediaConfig::audio_only();
        let sdp = generate_sdp(&params);

        assert!(!sdp.contains("m=video"));
        assert!(sdp.contains("m=audio 0 RTP/AVP 111"));
        assert!(sdp.contains("a=control:trackID=1"));
    }

    #[test]
    fn video_only_sdp() {
        let mut params = default_params();
        params.media = MediaConfig { video: true, audio: false };
        let sdp = generate_sdp(&params);

        assert!(sdp.contains("m=video 0 RTP/AVP 96"));
        assert!(sdp.contains("a=control:trackID=0"));
        assert!(!sdp.contains("m=audio"));
    }

    #[test]
    fn sdp_reflects_tier_bitrate() {
        let mut params = default_params();
        params.tier = QualityTier::High;
        let sdp = generate_sdp(&params);
        assert!(sdp.contains("maxaveragebitrate=256000"));
    }

    #[test]
    fn sdp_reflects_resolution() {
        let mut params = default_params();
        params.video_width = 1280;
        params.video_height = 720;
        params.framerate = 60;
        let sdp = generate_sdp(&params);
        assert!(sdp.contains("a=framesize:96 1280-720"));
        assert!(sdp.contains("a=framerate:60"));
    }
}
