//! VP9 RTP payload format per RFC 9628.
//!
//! Implements flexible-mode packetization for single-layer (non-SVC) VP9 streams
//! with proper frame fragmentation across MTU-sized RTP packets.

const DEFAULT_MTU: usize = 1200;

/// VP9 RTP packetizer that fragments encoded VP9 frames into RTP-sized payloads.
///
/// Each call to [`packetize`] increments the picture ID and produces one or more
/// payload buffers (VP9 payload descriptor + VP9 bitstream data) ready to be
/// wrapped in RTP headers by the caller.
#[derive(Debug)]
pub struct Vp9Packetizer {
    picture_id: u16,
    mtu: usize,
    width: u16,
    height: u16,
}

impl Vp9Packetizer {
    /// Create a new packetizer with the given resolution and default MTU (1200).
    #[must_use]
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            picture_id: 0,
            mtu: DEFAULT_MTU,
            width,
            height,
        }
    }

    /// Create a new packetizer with a custom MTU.
    #[must_use]
    pub fn with_mtu(width: u16, height: u16, mtu: usize) -> Self {
        Self {
            picture_id: 0,
            mtu: mtu.max(100),
            width,
            height,
        }
    }

    /// Update the resolution (used when the encoder reconfigures).
    pub fn set_resolution(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
    }

    /// Current picture ID.
    #[must_use]
    pub fn picture_id(&self) -> u16 {
        self.picture_id
    }

    /// Packetize a VP9 frame into one or more RTP payload buffers.
    ///
    /// Returns a `Vec` of payloads. Each payload is a VP9 payload descriptor
    /// followed by a chunk of the VP9 bitstream. The caller is responsible for
    /// prepending the 12-byte RTP header and setting the marker bit on the last
    /// packet.
    pub fn packetize(&mut self, frame: &[u8], is_keyframe: bool) -> Vec<Vec<u8>> {
        let pid = self.picture_id;
        self.picture_id = (self.picture_id + 1) & 0x7FFF;

        if frame.is_empty() {
            return vec![self.build_descriptor(is_keyframe, true, true, true, pid)];
        }

        let first_descriptor = self.build_descriptor(is_keyframe, true, false, is_keyframe, pid);
        let first_max_payload = self.mtu.saturating_sub(first_descriptor.len());

        if frame.len() <= first_max_payload {
            // Single packet: B=1, E=1
            let desc = self.build_descriptor(is_keyframe, true, true, is_keyframe, pid);
            let mut pkt = desc;
            pkt.extend_from_slice(frame);
            return vec![pkt];
        }

        let mut packets = Vec::new();
        let mut offset = 0;

        // First packet: B=1, E=0
        let chunk_size = first_max_payload.min(frame.len());
        let mut pkt = first_descriptor;
        pkt.extend_from_slice(&frame[..chunk_size]);
        packets.push(pkt);
        offset += chunk_size;

        // Continuation/last packets
        while offset < frame.len() {
            let is_last = frame.len() - offset <= self.continuation_max_payload(is_keyframe, pid);
            let desc = self.build_descriptor(is_keyframe, false, is_last, false, pid);
            let max_payload = self.mtu.saturating_sub(desc.len());
            let end = (offset + max_payload).min(frame.len());
            let mut pkt = desc;
            pkt.extend_from_slice(&frame[offset..end]);
            packets.push(pkt);
            offset = end;
        }

        packets
    }

    fn continuation_max_payload(&self, is_keyframe: bool, pid: u16) -> usize {
        let desc = self.build_descriptor(is_keyframe, false, true, false, pid);
        self.mtu.saturating_sub(desc.len())
    }

    /// Build a VP9 payload descriptor (non-flexible mode for broad compatibility).
    ///
    /// Layout:
    /// ```text
    ///   Byte 0: I|P|L|F|B|E|V|Z
    ///   Byte 1: M=1 | PID[14:8]    (15-bit PID)
    ///   Byte 2: PID[7:0]
    ///   [If V=1]: SS header (N_S[7:5]|Y[4]|G[3]|RES[2:0]), WIDTH(16), HEIGHT(16)
    /// ```
    ///
    /// F=0 (non-flexible mode) is used for compatibility with ffplay/VLC
    /// depacketizers that implement the older draft spec. In non-flexible mode
    /// no P_DIFF reference bytes are sent; the P bit alone signals inter-frames.
    fn build_descriptor(
        &self,
        is_keyframe: bool,
        is_start: bool,
        is_end: bool,
        include_ss: bool,
        pid: u16,
    ) -> Vec<u8> {
        let i = 1u8; // PID present
        let p = if is_keyframe { 0u8 } else { 1u8 };
        let l = 0u8; // no layer indices
        let f = 0u8; // non-flexible mode (compatible with ffplay/VLC)
        let b = u8::from(is_start);
        let e = u8::from(is_end);
        let v = if include_ss { 1u8 } else { 0u8 };
        let z = 0u8;

        let byte0 = (i << 7) | (p << 6) | (l << 5) | (f << 4) | (b << 3) | (e << 2) | (v << 1) | z;

        let cap = 3 + if include_ss { 5 } else { 0 };
        let mut desc = Vec::with_capacity(cap);
        desc.push(byte0);

        // PID: M=1, 15-bit
        desc.push(0x80 | ((pid >> 8) & 0x7F) as u8);
        desc.push((pid & 0xFF) as u8);

        // Scalability Structure for keyframes
        if include_ss {
            // N_S=0 (bits 7-5 = 000), Y=1 (bit 4), G=0 (bit 3), RES=0 (bits 2-0)
            desc.push(0b0001_0000);
            desc.push((self.width >> 8) as u8);
            desc.push((self.width & 0xFF) as u8);
            desc.push((self.height >> 8) as u8);
            desc.push((self.height & 0xFF) as u8);
        }

        desc
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_packet_keyframe() {
        let mut p = Vp9Packetizer::new(1920, 1080);
        let frame = vec![0xAA; 100];
        let packets = p.packetize(&frame, true);

        assert_eq!(packets.len(), 1);
        let pkt = &packets[0];

        // Byte 0: I=1,P=0,L=0,F=0,B=1,E=1,V=1,Z=0
        assert_eq!(pkt[0] & 0b1111_1110, 0b1000_1110);
        // M=1, PID=0
        assert_eq!(pkt[1], 0x80);
        assert_eq!(pkt[2], 0x00);
        // SS: N_S=0 (bits 7-5=000), Y=1 (bit 4), G=0 (bit 3)
        assert_eq!(pkt[3], 0b0001_0000);
        // Width=1920
        assert_eq!(u16::from_be_bytes([pkt[4], pkt[5]]), 1920);
        // Height=1080
        assert_eq!(u16::from_be_bytes([pkt[6], pkt[7]]), 1080);
        // Payload follows
        assert_eq!(&pkt[8..], &frame[..]);
    }

    #[test]
    fn single_packet_interframe() {
        let mut p = Vp9Packetizer::new(1920, 1080);
        // Advance PID past keyframe
        let _ = p.packetize(&[0u8; 10], true);

        let frame = vec![0xBB; 50];
        let packets = p.packetize(&frame, false);

        assert_eq!(packets.len(), 1);
        let pkt = &packets[0];

        // Byte 0: I=1,P=1,L=0,F=0,B=1,E=1,V=0,Z=0 (no P_DIFF in non-flexible mode)
        assert_eq!(pkt[0], 0b1100_1100);
        // M=1, PID=1
        assert_eq!(pkt[1], 0x80);
        assert_eq!(pkt[2], 0x01);
        // Payload follows directly (no P_DIFF byte)
        assert_eq!(&pkt[3..], &frame[..]);
    }

    #[test]
    fn fragmentation() {
        let mut p = Vp9Packetizer::with_mtu(640, 480, 200);
        // Frame larger than MTU minus descriptor
        let frame = vec![0xCC; 500];
        let packets = p.packetize(&frame, false);

        assert!(packets.len() > 1, "should fragment: got {} packets", packets.len());

        // First packet: B=1, E=0
        assert_eq!(packets[0][0] & 0b0000_1100, 0b0000_1000);

        // Middle packets (if any): B=0, E=0
        for pkt in &packets[1..packets.len() - 1] {
            assert_eq!(pkt[0] & 0b0000_1100, 0b0000_0000);
        }

        // Last packet: B=0, E=1
        let last = packets.last().unwrap();
        assert_eq!(last[0] & 0b0000_1100, 0b0000_0100);

        // Reassembled payload should match original (descriptor is 3 bytes for inter in non-flex)
        let reassembled: Vec<u8> = packets
            .iter()
            .flat_map(|pkt| {
                &pkt[3..]
            })
            .copied()
            .collect();
        assert_eq!(reassembled, frame);
    }

    #[test]
    fn keyframe_fragmentation_first_has_ss() {
        let mut p = Vp9Packetizer::with_mtu(1280, 720, 200);
        let frame = vec![0xDD; 500];
        let packets = p.packetize(&frame, true);

        assert!(packets.len() > 1);

        // First packet has V=1 (SS present)
        assert_ne!(packets[0][0] & 0b0000_0010, 0);

        // Subsequent packets have V=0
        for pkt in &packets[1..] {
            assert_eq!(pkt[0] & 0b0000_0010, 0);
        }
    }

    #[test]
    fn pid_wrapping() {
        let mut p = Vp9Packetizer::new(320, 240);
        // Set PID near wrap point
        p.picture_id = 0x7FFE;

        let _ = p.packetize(&[0u8; 10], false);
        assert_eq!(p.picture_id, 0x7FFF);

        let _ = p.packetize(&[0u8; 10], false);
        assert_eq!(p.picture_id, 0x0000);
    }

    #[test]
    fn empty_frame() {
        let mut p = Vp9Packetizer::new(640, 480);
        let packets = p.packetize(&[], true);
        assert_eq!(packets.len(), 1);
        // B=1, E=1
        assert_eq!(packets[0][0] & 0b0000_1100, 0b0000_1100);
    }

    #[test]
    fn resolution_update() {
        let mut p = Vp9Packetizer::new(640, 480);
        p.set_resolution(1920, 1080);

        let packets = p.packetize(&[0u8; 50], true);
        let pkt = &packets[0];
        // SS data: resolution bytes
        // Descriptor is 3 (mandatory) + 1 (SS header) + 4 (resolution) = but SS header
        // is at byte 3, followed by 2+2 resolution bytes
        assert_eq!(u16::from_be_bytes([pkt[4], pkt[5]]), 1920);
        assert_eq!(u16::from_be_bytes([pkt[6], pkt[7]]), 1080);
    }

    #[test]
    fn mtu_enforcement() {
        let mtu = 300;
        let mut p = Vp9Packetizer::with_mtu(1920, 1080, mtu);
        let frame = vec![0xEE; 2000];
        let packets = p.packetize(&frame, true);

        for pkt in &packets {
            assert!(
                pkt.len() <= mtu,
                "packet {} bytes exceeds MTU {}",
                pkt.len(),
                mtu
            );
        }
    }
}
