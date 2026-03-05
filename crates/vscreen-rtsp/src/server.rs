use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::handler::{HandlerContext, InstanceLookup, handle_request};
use crate::parser::RtspRequest;
use crate::session::RtspSessionManager;
use crate::transport::SharedTcpWriter;

/// RTSP server that listens on a TCP port and handles RTSP connections.
pub struct RtspServer {
    port: u16,
    session_manager: Arc<RtspSessionManager>,
    handler_ctx: Arc<HandlerContext>,
    cancel: CancellationToken,
}

impl std::fmt::Debug for RtspServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RtspServer")
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

impl RtspServer {
    /// Create a new RTSP server.
    #[must_use]
    pub fn new(
        port: u16,
        server_ip: IpAddr,
        instance_lookup: Arc<dyn InstanceLookup>,
        cancel: CancellationToken,
    ) -> Self {
        let session_manager = Arc::new(RtspSessionManager::new(cancel.clone()));

        let handler_ctx = Arc::new(HandlerContext {
            session_manager: Arc::clone(&session_manager),
            server_ip,
            instance_lookup,
        });

        Self {
            port,
            session_manager,
            handler_ctx,
            cancel,
        }
    }

    /// Get a reference to the session manager for external queries.
    #[must_use]
    pub fn session_manager(&self) -> &Arc<RtspSessionManager> {
        &self.session_manager
    }

    /// Run the RTSP server. Blocks until cancelled.
    ///
    /// # Errors
    /// Returns an I/O error if the TCP listener cannot be bound.
    pub async fn run(&self) -> Result<(), std::io::Error> {
        let addr = SocketAddr::new(
            IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
            self.port,
        );
        let listener = TcpListener::bind(addr).await?;

        info!(port = self.port, "RTSP server listening");

        // Spawn the session reaper
        let session_mgr = Arc::clone(&self.session_manager);
        let reaper_cancel = self.cancel.clone();
        tokio::spawn(async move {
            session_mgr.run_reaper().await;
            debug!("RTSP session reaper stopped");
            drop(reaper_cancel);
        });

        // Spawn the watchdog
        let watchdog_session_mgr = Arc::clone(&self.session_manager);
        let watchdog_cancel = self.cancel.clone();
        tokio::spawn(async move {
            run_watchdog(watchdog_session_mgr, watchdog_cancel).await;
        });

        loop {
            tokio::select! {
                () = self.cancel.cancelled() => {
                    info!("RTSP server shutting down");
                    self.session_manager.teardown_all();
                    break;
                }
                result = listener.accept() => {
                    match result {
                        Ok((stream, peer_addr)) => {
                            debug!(%peer_addr, "RTSP connection accepted");
                            metrics::counter!("vscreen_rtsp_connections_total").increment(1);

                            let ctx = Arc::clone(&self.handler_ctx);
                            let cancel = self.cancel.clone();

                            tokio::spawn(async move {
                                if let Err(e) = handle_connection(stream, peer_addr, ctx, cancel).await {
                                    warn!(%peer_addr, error = %e, "RTSP connection error");
                                }
                                debug!(%peer_addr, "RTSP connection closed");
                            });
                        }
                        Err(e) => {
                            error!(error = %e, "failed to accept RTSP connection");
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

/// Handle a single RTSP TCP connection.
///
/// The writer is wrapped in `Arc<Mutex>` so it can be shared with TCP interleaved
/// RTP sender tasks spawned during PLAY.
async fn handle_connection(
    stream: tokio::net::TcpStream,
    peer_addr: SocketAddr,
    ctx: Arc<HandlerContext>,
    cancel: CancellationToken,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (reader, writer) = stream.into_split();
    let shared_writer: SharedTcpWriter =
        Arc::new(tokio::sync::Mutex::new(writer));
    let mut reader = BufReader::new(reader);
    let mut request_buf = String::new();
    // Track the session associated with this TCP connection so we can
    // refresh its keepalive when interleaved RTCP data arrives.
    let mut current_session_id: Option<String> = None;

    loop {
        request_buf.clear();

        // Peek at the first byte to distinguish RTSP text from interleaved binary ($).
        let first_byte = {
            let buf = tokio::select! {
                () = cancel.cancelled() => { return Ok(()); }
                result = reader.fill_buf() => { result? }
            };
            if buf.is_empty() {
                return Ok(());
            }
            buf[0]
        };

        // Handle interleaved binary data from client (e.g. RTCP RR sent back).
        // Treat this as a keepalive — the client is still alive and sending data.
        if first_byte == 0x24 {
            let mut header = [0u8; 4];
            reader.read_exact(&mut header).await?;
            let len = u16::from_be_bytes([header[2], header[3]]) as usize;
            let mut data = vec![0u8; len];
            reader.read_exact(&mut data).await?;
            if let Some(ref sid) = current_session_id {
                ctx.session_manager.touch(sid);
            }
            continue;
        }

        // Read RTSP request headers line by line
        let mut content_length: usize = 0;

        loop {
            let mut line = String::new();

            tokio::select! {
                () = cancel.cancelled() => {
                    return Ok(());
                }
                result = reader.read_line(&mut line) => {
                    match result {
                        Ok(0) => return Ok(()),
                        Ok(_) => {
                            if let Some(rest) = line.to_lowercase().strip_prefix("content-length:") {
                                if let Ok(cl) = rest.trim().parse::<usize>() {
                                    content_length = cl;
                                }
                            }

                            request_buf.push_str(&line);

                            if line == "\r\n" || line == "\n" {
                                break;
                            }
                        }
                        Err(e) => return Err(e.into()),
                    }
                }
            }
        }

        if request_buf.is_empty() {
            continue;
        }

        if content_length > 0 {
            let mut body_buf = vec![0u8; content_length];
            tokio::select! {
                () = cancel.cancelled() => {
                    return Ok(());
                }
                result = reader.read_exact(&mut body_buf) => {
                    result?;
                    request_buf.push_str(&String::from_utf8_lossy(&body_buf));
                }
            }
        }

        let request = match RtspRequest::parse(request_buf.as_bytes()) {
            Ok(req) => req,
            Err(e) => {
                warn!(%peer_addr, error = %e, "failed to parse RTSP request");
                let resp = crate::parser::RtspResponse::new(400, "Bad Request", 0);
                let mut w = shared_writer.lock().await;
                w.write_all(&resp.serialize()).await?;
                continue;
            }
        };

        debug!(
            %peer_addr,
            method = %request.method,
            url = %request.url,
            cseq = request.cseq,
            "RTSP request"
        );

        let response = handle_request(
            &ctx,
            &request,
            peer_addr,
            Some(Arc::clone(&shared_writer)),
        )
        .await;

        // Track the session ID from responses so interleaved RTCP can
        // refresh the keepalive without an RTSP-level lookup.
        if let Some(ref sid) = response.session {
            current_session_id = Some(sid.0.clone());
        }

        debug!(
            %peer_addr,
            status = response.status,
            cseq = response.cseq,
            "RTSP response"
        );

        let mut w = shared_writer.lock().await;
        w.write_all(&response.serialize()).await?;
    }
}

/// Periodic watchdog that evaluates stream health metrics and reaps expired
/// sessions.
///
/// The watchdog does **not** tear down sessions based on health alone.
/// For screencast-based video the screen may be completely static for long
/// periods, producing zero RTP packets — this is normal, not a failure.
///
/// Session cleanup is handled by:
/// - **Session expiry:** no RTSP keepalive within the timeout window
/// - **Client TEARDOWN:** explicit client disconnect
/// - **Cancellation:** server shutdown
///
/// The watchdog updates health metrics (for monitoring/API) and logs
/// warnings when tracks are degraded or have client-reported failures.
async fn run_watchdog(session_manager: Arc<RtspSessionManager>, cancel: CancellationToken) {
    let interval = std::time::Duration::from_secs(10);

    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                debug!("RTSP watchdog shutting down");
                break;
            }
            () = tokio::time::sleep(interval) => {
                let sessions = session_manager.all_sessions();
                for info in &sessions {
                    if let Some(mut session) = session_manager.get_mut(&info.session_id) {
                        for track in &mut session.tracks {
                            track.health.evaluate();
                            match track.health.state {
                                crate::health::HealthState::Failed => {
                                    warn!(
                                        session_id = %info.session_id,
                                        instance_id = %info.instance_id,
                                        track_id = track.track_id,
                                        media_type = %track.media_type,
                                        packets_sent = track.health.packets_sent,
                                        packet_loss = track.health.client_packet_loss,
                                        "watchdog: track reporting high packet loss"
                                    );
                                }
                                crate::health::HealthState::Degraded => {
                                    debug!(
                                        session_id = %info.session_id,
                                        track_id = track.track_id,
                                        media_type = %track.media_type,
                                        packets_sent = track.health.packets_sent,
                                        consecutive_idle = track.health.consecutive_stale,
                                        "watchdog: track idle or degraded"
                                    );
                                }
                                crate::health::HealthState::Healthy => {}
                            }
                        }
                    }
                }

                // Reap sessions that haven't sent an RTSP keepalive
                session_manager.reap_expired();

                metrics::gauge!("vscreen_rtsp_sessions_active")
                    .set(session_manager.session_count() as f64);
            }
        }
    }
}
