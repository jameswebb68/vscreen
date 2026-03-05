use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Health state of a single RTSP audio stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthState {
    /// Stream is operating normally.
    Healthy,
    /// Stream shows degradation (moderate packet loss or jitter).
    Degraded,
    /// Stream has failed or is unresponsive.
    Failed,
}

impl std::fmt::Display for HealthState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Healthy => write!(f, "healthy"),
            Self::Degraded => write!(f, "degraded"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

/// Per-stream health and statistics.
#[derive(Debug, Clone, Serialize)]
pub struct StreamHealth {
    pub state: HealthState,
    pub packets_sent: u64,
    pub bytes_sent: u64,
    /// Client-reported packet loss ratio (0.0 - 1.0), from RTCP RR.
    pub client_packet_loss: f64,
    /// Client-reported jitter in milliseconds, from RTCP RR.
    pub client_jitter_ms: f64,
    #[serde(skip)]
    pub last_rtp_sent: Option<Instant>,
    #[serde(skip)]
    pub last_rtcp_received: Option<Instant>,
    #[serde(skip)]
    pub created_at: Instant,
    /// Number of consecutive watchdog evaluations where the stream was stale.
    #[serde(skip)]
    pub consecutive_stale: u32,
}

impl Default for StreamHealth {
    fn default() -> Self {
        Self {
            state: HealthState::Healthy,
            packets_sent: 0,
            bytes_sent: 0,
            client_packet_loss: 0.0,
            client_jitter_ms: 0.0,
            last_rtp_sent: None,
            last_rtcp_received: None,
            created_at: Instant::now(),
            consecutive_stale: 0,
        }
    }
}

impl StreamHealth {
    /// How long this stream has been alive.
    #[must_use]
    pub fn uptime(&self) -> Duration {
        self.created_at.elapsed()
    }

    /// Serializable uptime in seconds.
    #[must_use]
    pub fn uptime_secs(&self) -> f64 {
        self.uptime().as_secs_f64()
    }

    /// Record that an RTP packet was sent.
    pub fn record_packet_sent(&mut self, bytes: u64) {
        self.packets_sent += 1;
        self.bytes_sent += bytes;
        self.last_rtp_sent = Some(Instant::now());
    }

    /// Update health from an RTCP Receiver Report.
    pub fn update_from_rtcp_rr(&mut self, packet_loss: f64, jitter_ms: f64) {
        self.client_packet_loss = packet_loss;
        self.client_jitter_ms = jitter_ms;
        self.last_rtcp_received = Some(Instant::now());
    }

    /// Evaluate health state based on current metrics.
    ///
    /// Staleness (no recent RTP sends) is reported as `Degraded` but never
    /// triggers `Failed`.  For screencast-based video the screen may be
    /// completely static for long periods, producing no new frames — this is
    /// normal and must not tear down the session.
    ///
    /// `Failed` is only declared when the *client* reports severe problems
    /// via RTCP Receiver Reports (very high packet loss).  Session cleanup
    /// for truly dead connections is handled by the RTSP session idle-timeout
    /// and client-initiated TEARDOWN.
    pub fn evaluate(&mut self) {
        if self.is_idle() {
            self.consecutive_stale += 1;
            // Stale → Degraded only, never Failed.
            // Session expiry handles truly dead connections.
            self.state = HealthState::Degraded;
        } else {
            self.consecutive_stale = 0;
            if self.client_packet_loss > 0.50 {
                self.state = HealthState::Failed;
            } else if self.client_packet_loss > 0.10 || self.client_jitter_ms > 100.0 {
                self.state = HealthState::Degraded;
            } else {
                self.state = HealthState::Healthy;
            }
        }
    }

    /// Whether no RTP packet has been sent recently.
    ///
    /// Uses a 10-second timeout for active streams and a 30-second grace
    /// period after creation before declaring idle.
    #[must_use]
    fn is_idle(&self) -> bool {
        match self.last_rtp_sent {
            Some(t) => t.elapsed() > Duration::from_secs(10),
            None => self.created_at.elapsed() > Duration::from_secs(30),
        }
    }
}

/// Aggregated health across all RTSP streams for an instance.
#[derive(Debug, Clone, Serialize)]
pub struct AggregatedHealth {
    pub total_sessions: u32,
    pub healthy: u32,
    pub degraded: u32,
    pub failed: u32,
    pub total_packets_sent: u64,
    pub total_bytes_sent: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_health_is_healthy() {
        let h = StreamHealth::default();
        assert_eq!(h.state, HealthState::Healthy);
        assert_eq!(h.packets_sent, 0);
    }

    #[test]
    fn record_packet() {
        let mut h = StreamHealth::default();
        h.record_packet_sent(100);
        assert_eq!(h.packets_sent, 1);
        assert_eq!(h.bytes_sent, 100);
        assert!(h.last_rtp_sent.is_some());
    }

    #[test]
    fn degraded_on_high_loss() {
        let mut h = StreamHealth::default();
        h.record_packet_sent(100);
        h.update_from_rtcp_rr(0.15, 20.0);
        h.evaluate();
        assert_eq!(h.state, HealthState::Degraded);
    }

    #[test]
    fn healthy_on_good_metrics() {
        let mut h = StreamHealth::default();
        h.record_packet_sent(100);
        h.update_from_rtcp_rr(0.01, 5.0);
        h.evaluate();
        assert_eq!(h.state, HealthState::Healthy);
    }

    #[test]
    fn health_display() {
        assert_eq!(HealthState::Healthy.to_string(), "healthy");
        assert_eq!(HealthState::Degraded.to_string(), "degraded");
        assert_eq!(HealthState::Failed.to_string(), "failed");
    }
}
