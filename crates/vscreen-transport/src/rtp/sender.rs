use std::net::SocketAddr;

use tokio::net::UdpSocket;
use tracing::{debug, info};
use vscreen_core::config::RtpOutputConfig;
use vscreen_core::error::TransportError;
use vscreen_core::frame::EncodedPacket;

/// RTP/UDP sender for multicast audio output.
#[derive(Debug)]
pub struct RtpSender {
    config: RtpOutputConfig,
    socket: Option<UdpSocket>,
    target_addr: SocketAddr,
    packets_sent: u64,
    sequence_number: u16,
    ssrc: u32,
}

impl RtpSender {
    /// Create a new RTP sender.
    ///
    /// # Errors
    /// Returns `TransportError` if the target address is invalid.
    pub fn new(config: RtpOutputConfig) -> Result<Self, TransportError> {
        let target_addr: SocketAddr = format!("{}:{}", config.address, config.port)
            .parse()
            .map_err(|e| TransportError::RtpSend(format!("invalid address: {e}")))?;

        Ok(Self {
            config,
            socket: None,
            target_addr,
            packets_sent: 0,
            sequence_number: 0,
            ssrc: rand_ssrc(),
        })
    }

    /// Bind the UDP socket and prepare for sending.
    ///
    /// # Errors
    /// Returns `TransportError` if the socket cannot be bound.
    pub async fn bind(&mut self) -> Result<(), TransportError> {
        let socket = UdpSocket::bind("0.0.0.0:0")
            .await
            .map_err(|e| TransportError::RtpSend(format!("bind failed: {e}")))?;

        if self.config.multicast {
            debug!(addr = %self.target_addr, "RTP sender configured for multicast");
        }

        info!(
            target = %self.target_addr,
            ssrc = self.ssrc,
            "RTP sender bound"
        );

        self.socket = Some(socket);
        Ok(())
    }

    /// Send an encoded packet as RTP.
    ///
    /// # Errors
    /// Returns `TransportError` if the send fails.
    pub async fn send(&mut self, packet: &EncodedPacket) -> Result<(), TransportError> {
        let socket = self
            .socket
            .as_ref()
            .ok_or_else(|| TransportError::RtpSend("socket not bound".into()))?;

        // Build a minimal RTP header + payload
        let rtp_packet = self.build_rtp_packet(packet);

        socket
            .send_to(&rtp_packet, self.target_addr)
            .await
            .map_err(|e| TransportError::RtpSend(format!("send failed: {e}")))?;

        self.packets_sent += 1;
        self.sequence_number = self.sequence_number.wrapping_add(1);

        Ok(())
    }

    /// Build a minimal RTP packet (12-byte header + payload).
    fn build_rtp_packet(&self, packet: &EncodedPacket) -> Vec<u8> {
        let mut rtp = Vec::with_capacity(12 + packet.data.len());

        // RTP header (12 bytes)
        // V=2, P=0, X=0, CC=0
        rtp.push(0x80);
        // M (marker) + PT (payload type 111 for Opus)
        let marker: u8 = if packet.is_keyframe { 0x80 } else { 0x00 };
        rtp.push(marker | 111);
        // Sequence number (big-endian)
        rtp.extend_from_slice(&self.sequence_number.to_be_bytes());
        // Timestamp (big-endian, use pts * 48000/1000 for audio)
        let timestamp = (packet.pts * 48000 / 1000) as u32;
        rtp.extend_from_slice(&timestamp.to_be_bytes());
        // SSRC (big-endian)
        rtp.extend_from_slice(&self.ssrc.to_be_bytes());
        // Payload
        rtp.extend_from_slice(&packet.data);

        rtp
    }

    /// Total packets sent.
    #[must_use]
    pub fn packets_sent(&self) -> u64 {
        self.packets_sent
    }
}

/// Generate a random SSRC per RFC 3550.
fn rand_ssrc() -> u32 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = RandomState::new().build_hasher();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    hasher.write_u128(nanos);
    hasher.write_usize(&hasher as *const _ as usize);
    hasher.finish() as u32
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;

    #[test]
    fn create_sender() {
        let config = RtpOutputConfig {
            address: "239.0.0.1".into(),
            port: 5004,
            multicast: true,
        };
        let sender = RtpSender::new(config).expect("create");
        assert_eq!(sender.packets_sent(), 0);
    }

    #[test]
    fn reject_invalid_address() {
        let config = RtpOutputConfig {
            address: "not-an-address".into(),
            port: 5004,
            multicast: false,
        };
        assert!(RtpSender::new(config).is_err());
    }

    #[test]
    fn rtp_packet_structure() {
        let config = RtpOutputConfig {
            address: "127.0.0.1".into(),
            port: 5004,
            multicast: false,
        };
        let sender = RtpSender::new(config).expect("create");

        let packet = EncodedPacket {
            data: Bytes::from(vec![1, 2, 3, 4]),
            is_keyframe: false,
            pts: 0,
            duration: Some(20),
            codec: None,
        };

        let rtp = sender.build_rtp_packet(&packet);
        assert_eq!(rtp.len(), 12 + 4); // 12-byte header + 4-byte payload
        assert_eq!(rtp[0], 0x80); // V=2
        assert_eq!(rtp[1] & 0x7F, 111); // PT=111
    }

    #[tokio::test]
    async fn bind_and_send() {
        let config = RtpOutputConfig {
            address: "127.0.0.1".into(),
            port: 0, // Won't actually deliver, but won't error on send
            multicast: false,
        };
        // Just verify bind works - can't easily test UDP delivery in unit test
        let mut sender = RtpSender::new(config).expect("create");

        // Note: sending to port 0 may fail on some systems, so we just test bind
        let bind_result = sender.bind().await;
        // bind to 0.0.0.0:0 should always succeed
        assert!(bind_result.is_ok());
    }
}
