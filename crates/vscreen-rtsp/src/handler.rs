use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use tracing::{debug, info, warn};
use vscreen_core::frame::{EncodedPacket, VideoCodec};
use vscreen_core::instance::InstanceId;

use crate::parser::{
    Method, RtspRequest, RtspResponse, TransportHeader, TransportMode,
    extract_instance_id, extract_query_params, extract_track_id, parse_media_config,
};
use crate::quality::QualityTier;
use crate::sdp::{SdpParams, generate_sdp};
use crate::session::{MediaType, RtspSessionManager};
use crate::transcoder::OpusTranscoder;
use crate::transport::{RtpUnicastStream, RtpVideoStream, SharedTcpWriter, allocate_port_pair};

/// Shared context for RTSP request handlers.
pub struct HandlerContext {
    pub session_manager: Arc<RtspSessionManager>,
    pub server_ip: IpAddr,
    pub instance_lookup: Arc<dyn InstanceLookup>,
}

impl std::fmt::Debug for HandlerContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HandlerContext")
            .field("server_ip", &self.server_ip)
            .finish_non_exhaustive()
    }
}

/// Trait for looking up instances and subscribing to their media streams.
pub trait InstanceLookup: Send + Sync + 'static {
    /// Check if an instance exists.
    fn instance_exists(&self, instance_id: &str) -> bool;

    /// Subscribe to an instance's audio broadcast channel.
    fn subscribe_audio(
        &self,
        instance_id: &str,
    ) -> Option<tokio::sync::broadcast::Receiver<EncodedPacket>>;

    /// Subscribe to an instance's video broadcast channel.
    fn subscribe_video(
        &self,
        instance_id: &str,
    ) -> Option<tokio::sync::broadcast::Receiver<EncodedPacket>>;

    /// Get the configured video resolution for an instance.
    fn video_resolution(&self, instance_id: &str) -> Option<(u32, u32)>;

    /// Get the configured video framerate for an instance.
    fn video_framerate(&self, instance_id: &str) -> Option<u32>;

    /// Request the next video frame to be encoded as a keyframe.
    fn request_keyframe(&self, instance_id: &str);

    /// Get the configured video codec for an instance.
    fn video_codec(&self, instance_id: &str) -> VideoCodec {
        let _ = instance_id;
        VideoCodec::H264
    }
}

/// Dispatch an RTSP request to the appropriate handler.
///
/// `tcp_writer` is provided for TCP interleaved sessions so RTP data can be
/// multiplexed on the same connection as RTSP control messages.
pub async fn handle_request(
    ctx: &HandlerContext,
    request: &RtspRequest,
    client_addr: SocketAddr,
    tcp_writer: Option<SharedTcpWriter>,
) -> RtspResponse {
    let cseq = request.cseq;

    match request.method {
        Method::Options => handle_options(cseq),
        Method::Describe => handle_describe(ctx, request, cseq),
        Method::Setup => handle_setup(ctx, request, cseq, client_addr).await,
        Method::Play => handle_play(ctx, request, cseq, tcp_writer),
        Method::Pause => handle_pause(ctx, request, cseq),
        Method::Teardown => handle_teardown(ctx, request, cseq),
        Method::GetParameter => handle_get_parameter(ctx, request, cseq),
    }
}

fn handle_options(cseq: u32) -> RtspResponse {
    let mut resp = RtspResponse::ok(cseq);
    resp.header(
        "Public",
        "OPTIONS, DESCRIBE, SETUP, PLAY, PAUSE, TEARDOWN, GET_PARAMETER",
    );
    resp
}

fn handle_describe(ctx: &HandlerContext, request: &RtspRequest, cseq: u32) -> RtspResponse {
    let instance_id = match extract_instance_id(&request.url) {
        Some(id) => id,
        None => {
            warn!(url = %request.url, "DESCRIBE: could not extract instance ID");
            return RtspResponse::not_found(cseq);
        }
    };

    if !ctx.instance_lookup.instance_exists(&instance_id) {
        warn!(instance_id, "DESCRIBE: instance not found");
        return RtspResponse::not_found(cseq);
    }

    let media = parse_media_config(&request.url);
    let params = extract_query_params(&request.url);
    let tier = resolve_quality_tier(&params);

    let (video_width, video_height) = ctx
        .instance_lookup
        .video_resolution(&instance_id)
        .unwrap_or((1920, 1080));
    let framerate = ctx
        .instance_lookup
        .video_framerate(&instance_id)
        .unwrap_or(30);

    let video_codec = ctx.instance_lookup.video_codec(&instance_id);

    let sdp = generate_sdp(&SdpParams {
        instance_id: &instance_id,
        server_ip: ctx.server_ip,
        session_version: 1,
        tier,
        ptime_ms: 20,
        media,
        video_codec,
        video_width,
        video_height,
        framerate,
    });

    let mut resp = RtspResponse::ok(cseq);
    resp.set_body("application/sdp", sdp);
    resp.header("Content-Base", &request.url);

    debug!(instance_id, ?media, "DESCRIBE response sent");
    resp
}

async fn handle_setup(
    ctx: &HandlerContext,
    request: &RtspRequest,
    cseq: u32,
    client_addr: SocketAddr,
) -> RtspResponse {
    let instance_id = match extract_instance_id(&request.url) {
        Some(id) => id,
        None => return RtspResponse::not_found(cseq),
    };

    if !ctx.instance_lookup.instance_exists(&instance_id) {
        return RtspResponse::not_found(cseq);
    }

    // Parse Transport header
    let transport_str = match request.headers.get("transport") {
        Some(t) => t,
        None => {
            warn!("SETUP: missing Transport header");
            return RtspResponse::unsupported_transport(cseq);
        }
    };

    let transport = match TransportHeader::parse(transport_str) {
        Ok(t) => t,
        Err(e) => {
            warn!(error = %e, "SETUP: invalid Transport header");
            return RtspResponse::unsupported_transport(cseq);
        }
    };

    // Allocate server ports only for UDP transport
    let (server_rtp_port, server_rtcp_port) = if transport.is_tcp_interleaved() {
        (0, 0)
    } else {
        match allocate_port_pair().await {
            Ok(pair) => pair,
            Err(e) => {
                warn!(error = %e, "SETUP: failed to allocate ports");
                return RtspResponse::internal_error(cseq);
            }
        }
    };

    let params = extract_query_params(&request.url);
    let quality = resolve_quality_tier(&params);
    let media_config = parse_media_config(&request.url);
    let track_id = extract_track_id(&request.url);

    // Determine the media type for this track
    let (resolved_track_id, media_type) = match track_id {
        Some(0) => (0, MediaType::Video),
        Some(1) => (1, MediaType::Audio),
        None => {
            if media_config.audio && !media_config.video {
                (1, MediaType::Audio)
            } else if media_config.video && !media_config.audio {
                (0, MediaType::Video)
            } else {
                (1, MediaType::Audio)
            }
        }
        Some(id) => {
            warn!(track_id = id, "SETUP: unknown track ID");
            return RtspResponse::not_found(cseq);
        }
    };

    // Helper closure to add the track based on transport mode
    let add_track_to_session = |session: &mut crate::session::RtspSession| {
        match &transport.mode {
            TransportMode::UdpUnicast { client_rtp_port, client_rtcp_port } => {
                session.add_track(
                    resolved_track_id,
                    media_type,
                    *client_rtp_port,
                    *client_rtcp_port,
                    server_rtp_port,
                    server_rtcp_port,
                );
            }
            TransportMode::TcpInterleaved { rtp_channel, rtcp_channel } => {
                session.add_interleaved_track(
                    resolved_track_id,
                    media_type,
                    *rtp_channel,
                    *rtcp_channel,
                );
            }
        }
    };

    // If session already exists (subsequent SETUP), add the track
    let session_id = if let Some(existing_session) = &request.session {
        if let Some(mut session) = ctx.session_manager.get_mut(&existing_session.0) {
            if session.instance_id.0 != instance_id {
                warn!("SETUP: session instance mismatch");
                return RtspResponse::session_not_found(cseq);
            }

            if session.track(resolved_track_id).is_some() {
                warn!(track_id = resolved_track_id, "SETUP: track already set up");
                return RtspResponse::new(455, "Method Not Valid in This State", cseq);
            }

            add_track_to_session(&mut session);

            existing_session.clone()
        } else {
            warn!(session_id = %existing_session, "SETUP: session not found");
            return RtspResponse::session_not_found(cseq);
        }
    } else {
        let sid = ctx.session_manager.create_session(
            InstanceId::from(instance_id.as_str()),
            client_addr,
            media_config,
            quality,
        );

        if let Some(mut session) = ctx.session_manager.get_mut(&sid.0) {
            add_track_to_session(&mut session);
        }

        sid
    };

    let resp_transport = transport.format_response(server_rtp_port, server_rtcp_port);

    let timeout = ctx.session_manager.timeout_secs();
    let mut resp = RtspResponse::ok(cseq);
    resp = resp.with_session(session_id, timeout);
    resp.header("Transport", &resp_transport);

    let is_tcp = transport.is_tcp_interleaved();
    info!(
        %client_addr,
        server_rtp_port,
        server_rtcp_port,
        track_id = resolved_track_id,
        %media_type,
        quality = %quality,
        tcp_interleaved = is_tcp,
        "SETUP complete"
    );

    resp
}

fn handle_play(
    ctx: &HandlerContext,
    request: &RtspRequest,
    cseq: u32,
    tcp_writer: Option<SharedTcpWriter>,
) -> RtspResponse {
    let session_id = match &request.session {
        Some(s) => s,
        None => {
            warn!("PLAY: missing Session header");
            return RtspResponse::session_not_found(cseq);
        }
    };

    let mut session = match ctx.session_manager.get_mut(&session_id.0) {
        Some(s) => s,
        None => {
            warn!(session_id = %session_id, "PLAY: session not found");
            return RtspResponse::session_not_found(cseq);
        }
    };

    if let Err(e) = session.play() {
        warn!(error = %e, "PLAY: state transition failed");
        return RtspResponse::new(455, "Method Not Valid in This State", cseq);
    }

    let instance_id = session.instance_id.0.clone();
    let cancel = session.cancel.clone();
    let session_id_str = session.id.0.clone();
    let session_mgr = Arc::clone(&ctx.session_manager);

    // Request a keyframe so new clients can start decoding immediately.
    // Without this, clients would have to wait for the next natural keyframe
    // and ffplay/VLC would report "unspecified size".
    if session.track_by_type(crate::session::MediaType::Video).is_some() {
        ctx.instance_lookup.request_keyframe(&instance_id);
    }

    // Spawn RTP sender tasks for each track that doesn't already have one
    for track_idx in 0..session.tracks.len() {
        if session.tracks[track_idx].rtp_task.is_some() {
            continue;
        }

        let track = &session.tracks[track_idx];
        let track_id = track.track_id;
        let media_type = track.media_type;
        let interleaved = track.interleaved_channels;
        let client_rtp_addr = SocketAddr::new(session.client_addr.ip(), track.client_rtp_port);
        let client_rtcp_addr = SocketAddr::new(session.client_addr.ip(), track.client_rtcp_port);
        let server_rtp_port = track.server_rtp_port;
        let server_rtcp_port = track.server_rtcp_port;
        let quality = session.quality;
        let cancel = cancel.clone();
        let sid_str = session_id_str.clone();
        let mgr = Arc::clone(&session_mgr);
        let writer = tcp_writer.clone();

        match media_type {
            MediaType::Audio => {
                let rx = ctx.instance_lookup.subscribe_audio(&instance_id);
                if let Some(mut audio_rx) = rx {
                    let task = tokio::spawn(async move {
                        let mut stream = if let Some((rtp_ch, rtcp_ch)) = interleaved {
                            let w = match writer {
                                Some(w) => w,
                                None => {
                                    warn!("TCP interleaved audio: no writer available");
                                    return;
                                }
                            };
                            RtpUnicastStream::new_interleaved(
                                w, rtp_ch, rtcp_ch, cancel.clone(),
                            )
                        } else {
                            match RtpUnicastStream::new(
                                server_rtp_port,
                                server_rtcp_port,
                                client_rtp_addr,
                                client_rtcp_addr,
                                cancel.clone(),
                            )
                            .await
                            {
                                Ok(s) => s,
                                Err(e) => {
                                    warn!(error = %e, "failed to create audio RTP stream");
                                    return;
                                }
                            }
                        };

                        let mut transcoder = if quality.needs_transcode() {
                            match OpusTranscoder::new(quality) {
                                Ok(tc) => Some(tc),
                                Err(e) => {
                                    warn!(error = %e, "failed to create transcoder");
                                    return;
                                }
                            }
                        } else {
                            None
                        };

                        let mut sr_interval =
                            tokio::time::interval(std::time::Duration::from_secs(5));
                        sr_interval
                            .set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

                        loop {
                            tokio::select! {
                                () = cancel.cancelled() => {
                                    debug!(session_id = %sid_str, "audio RTP sender cancelled");
                                    break;
                                }
                                result = audio_rx.recv() => {
                                    match result {
                                        Ok(packet) => {
                                            let send_data = if let Some(ref mut tc) = transcoder {
                                                match tc.transcode(&packet) {
                                                    Ok(transcoded) => transcoded.data,
                                                    Err(e) => {
                                                        warn!(error = %e, "transcode failed");
                                                        continue;
                                                    }
                                                }
                                            } else {
                                                packet.data
                                            };

                                            if let Err(e) = stream.send_rtp(&send_data).await {
                                                warn!(error = %e, "audio RTP send failed");
                                                break;
                                            }

                                            if let Some(mut s) = mgr.get_mut(&sid_str) {
                                                if let Some(t) = s.track_mut(track_id) {
                                                    t.health = stream.health.clone();
                                                }
                                            }
                                        }
                                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                            warn!(lagged = n, "audio broadcast lagged");
                                        }
                                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                            debug!("audio broadcast closed");
                                            break;
                                        }
                                    }
                                }
                                _ = sr_interval.tick() => {
                                    if let Err(e) = stream.send_sender_report().await {
                                        warn!(error = %e, "audio RTCP SR send failed");
                                    }
                                    stream.try_receive_rtcp().await;
                                    stream.health.evaluate();
                                    mgr.touch(&sid_str);
                                    if let Some(mut s) = mgr.get_mut(&sid_str) {
                                        if let Some(t) = s.track_mut(track_id) {
                                            t.health = stream.health.clone();
                                        }
                                    }
                                }
                            }
                        }
                    });

                    session.tracks[track_idx].rtp_task = Some(task);
                } else {
                    warn!("PLAY: could not subscribe to audio broadcast");
                }
            }

            MediaType::Video => {
                let rx = ctx.instance_lookup.subscribe_video(&instance_id);
                let resolution = ctx.instance_lookup.video_resolution(&instance_id);
                let fps = ctx.instance_lookup.video_framerate(&instance_id).unwrap_or(30);
                let codec = ctx.instance_lookup.video_codec(&instance_id);

                if let Some(mut video_rx) = rx {
                    let (w, h) = resolution.unwrap_or((1920, 1080));

                    let task = tokio::spawn(async move {
                        let mut stream = if let Some((rtp_ch, rtcp_ch)) = interleaved {
                            let wr = match writer {
                                Some(w) => w,
                                None => {
                                    warn!("TCP interleaved video: no writer available");
                                    return;
                                }
                            };
                            RtpVideoStream::new_interleaved(
                                wr, rtp_ch, rtcp_ch, cancel.clone(),
                                w as u16, h as u16, fps, codec,
                            )
                        } else {
                            match RtpVideoStream::new(
                                server_rtp_port,
                                server_rtcp_port,
                                client_rtp_addr,
                                client_rtcp_addr,
                                cancel.clone(),
                                w as u16,
                                h as u16,
                                fps,
                                codec,
                            )
                            .await
                            {
                                Ok(s) => s,
                                Err(e) => {
                                    warn!(error = %e, "failed to create video RTP stream");
                                    return;
                                }
                            }
                        };

                        let mut sr_interval =
                            tokio::time::interval(std::time::Duration::from_secs(5));
                        sr_interval
                            .set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

                        loop {
                            tokio::select! {
                                () = cancel.cancelled() => {
                                    debug!(session_id = %sid_str, "video RTP sender cancelled");
                                    break;
                                }
                                result = video_rx.recv() => {
                                    match result {
                                        Ok(packet) => {
                                            if let Err(e) = stream.send_frame(&packet.data, packet.is_keyframe).await {
                                                warn!(error = %e, "video RTP send failed");
                                                break;
                                            }

                                            if let Some(mut s) = mgr.get_mut(&sid_str) {
                                                if let Some(t) = s.track_mut(track_id) {
                                                    t.health = stream.health.clone();
                                                }
                                            }
                                        }
                                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                            warn!(lagged = n, "video broadcast lagged");
                                        }
                                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                            debug!("video broadcast closed");
                                            break;
                                        }
                                    }
                                }
                                _ = sr_interval.tick() => {
                                    if let Err(e) = stream.send_sender_report().await {
                                        warn!(error = %e, "video RTCP SR send failed");
                                    }
                                    stream.try_receive_rtcp().await;
                                    stream.health.evaluate();
                                    mgr.touch(&sid_str);
                                    if let Some(mut s) = mgr.get_mut(&sid_str) {
                                        if let Some(t) = s.track_mut(track_id) {
                                            t.health = stream.health.clone();
                                        }
                                    }
                                }
                            }
                        }
                    });

                    session.tracks[track_idx].rtp_task = Some(task);
                } else {
                    warn!("PLAY: could not subscribe to video broadcast");
                }
            }
        }
    }

    let timeout = ctx.session_manager.timeout_secs();
    let mut resp = RtspResponse::ok(cseq);
    resp = resp.with_session(session_id.clone(), timeout);

    info!(session_id = %session_id, "PLAY started");
    resp
}

fn handle_pause(ctx: &HandlerContext, request: &RtspRequest, cseq: u32) -> RtspResponse {
    let session_id = match &request.session {
        Some(s) => s,
        None => return RtspResponse::session_not_found(cseq),
    };

    let mut session = match ctx.session_manager.get_mut(&session_id.0) {
        Some(s) => s,
        None => return RtspResponse::session_not_found(cseq),
    };

    if let Err(e) = session.pause() {
        warn!(error = %e, "PAUSE: state transition failed");
        return RtspResponse::new(455, "Method Not Valid in This State", cseq);
    }

    // Cancel all running RTP tasks
    session.cancel.cancel();
    for track in &mut session.tracks {
        if let Some(task) = track.rtp_task.take() {
            task.abort();
        }
    }
    // Reset cancel token for potential resume
    session.cancel = ctx.session_manager.cancel_token_child();

    let timeout = ctx.session_manager.timeout_secs();
    let mut resp = RtspResponse::ok(cseq);
    resp = resp.with_session(session_id.clone(), timeout);

    info!(session_id = %session_id, "PAUSE");
    resp
}

fn handle_teardown(ctx: &HandlerContext, request: &RtspRequest, cseq: u32) -> RtspResponse {
    let session_id = match &request.session {
        Some(s) => s,
        None => return RtspResponse::session_not_found(cseq),
    };

    if ctx.session_manager.remove(&session_id.0).is_none() {
        return RtspResponse::session_not_found(cseq);
    }

    info!(session_id = %session_id, "TEARDOWN");
    RtspResponse::ok(cseq)
}

fn handle_get_parameter(ctx: &HandlerContext, request: &RtspRequest, cseq: u32) -> RtspResponse {
    if let Some(session_id) = &request.session {
        if let Some(mut session) = ctx.session_manager.get_mut(&session_id.0) {
            session.touch();
        }
    }

    RtspResponse::ok(cseq)
}

/// Resolve quality tier from URL query parameters.
fn resolve_quality_tier(params: &std::collections::HashMap<String, String>) -> QualityTier {
    if let Some(tier_name) = params.get("tier") {
        if let Some(tier) = QualityTier::from_name(tier_name) {
            return tier;
        }
    }

    if let Some(kbps_str) = params.get("kbps") {
        if let Ok(kbps) = kbps_str.parse::<u32>() {
            let channels = params
                .get("channels")
                .and_then(|c| c.parse::<u16>().ok());
            return QualityTier::custom(kbps, channels);
        }
    }

    QualityTier::High
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_tier_from_name() {
        let mut params = std::collections::HashMap::new();
        params.insert("tier".to_owned(), "low".to_owned());
        assert_eq!(resolve_quality_tier(&params), QualityTier::Low);
    }

    #[test]
    fn resolve_tier_from_kbps() {
        let mut params = std::collections::HashMap::new();
        params.insert("kbps".to_owned(), "192".to_owned());
        let tier = resolve_quality_tier(&params);
        assert_eq!(tier.bitrate_kbps(), 192);
    }

    #[test]
    fn resolve_default_tier() {
        let params = std::collections::HashMap::new();
        assert_eq!(resolve_quality_tier(&params), QualityTier::High);
    }
}
