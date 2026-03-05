use std::sync::OnceLock;

/// Prometheus metrics handle for rendering /metrics endpoint.
static PROM_HANDLE: OnceLock<metrics_exporter_prometheus::PrometheusHandle> = OnceLock::new();

/// Initialize the Prometheus metrics recorder.
/// Call once at startup.
pub fn init_metrics() {
    let builder = metrics_exporter_prometheus::PrometheusBuilder::new();
    match builder.install_recorder() {
        Ok(handle) => {
            PROM_HANDLE.set(handle).ok();
            register_descriptions();
            tracing::info!("Prometheus metrics initialized");
        }
        Err(e) => {
            tracing::warn!(%e, "failed to install Prometheus recorder");
        }
    }
}

fn register_descriptions() {
    metrics::describe_counter!("vscreen_frames_encoded_total", "Total video frames encoded");
    metrics::describe_counter!("vscreen_frames_dropped_total", "Total video frames dropped");
    metrics::describe_counter!("vscreen_audio_frames_total", "Total audio frames encoded");
    metrics::describe_counter!("vscreen_frames_skipped_total", "Video frames skipped (stale)");
    metrics::describe_gauge!("vscreen_active_peers", "Current active WebRTC peers");
    metrics::describe_gauge!("vscreen_active_instances", "Current active instances");
    metrics::describe_histogram!("vscreen_encode_duration_seconds", "VP9 encode time per frame");

    // RTSP metrics
    metrics::describe_gauge!("vscreen_rtsp_sessions_active", "Current active RTSP sessions");
    metrics::describe_counter!("vscreen_rtsp_connections_total", "Total RTSP TCP connections accepted");
    metrics::describe_counter!("vscreen_rtsp_sessions_expired_total", "RTSP sessions expired by reaper");
    metrics::describe_counter!("vscreen_rtsp_watchdog_teardowns_total", "RTSP sessions torn down by watchdog");
    metrics::describe_counter!("vscreen_rtsp_packets_sent_total", "Total RTP audio packets sent via RTSP");
    metrics::describe_counter!("vscreen_rtsp_bytes_sent_total", "Total RTP audio bytes sent via RTSP");
    metrics::describe_counter!("vscreen_rtsp_video_packets_sent_total", "Total RTP video packets sent via RTSP");
    metrics::describe_counter!("vscreen_rtsp_video_bytes_sent_total", "Total RTP video bytes sent via RTSP");
    metrics::describe_gauge!("vscreen_rtsp_transcode_active", "Active RTSP transcoding sessions");
    metrics::describe_gauge!("vscreen_rtsp_client_packet_loss_ratio", "Client-reported packet loss ratio (RTCP RR)");
    metrics::describe_gauge!("vscreen_rtsp_client_jitter_ms", "Client-reported jitter in ms (RTCP RR)");
}

/// Render current metrics as Prometheus text exposition format.
pub fn render() -> String {
    PROM_HANDLE
        .get()
        .map(|h| h.render())
        .unwrap_or_default()
}
