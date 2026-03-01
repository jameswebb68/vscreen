//! H.264 RTP payload format per RFC 6184.
//!
//! Implements Single NAL Unit mode and FU-A fragmentation for NAL units
//! that exceed the MTU. SPS/PPS are sent as individual NAL unit packets
//! alongside keyframes (the openh264 encoder emits them inline in Annex-B).

const DEFAULT_MTU: usize = 1200;
const RTP_HEADER_SIZE: usize = 12;

/// Annex-B start code patterns.
const START_CODE_3: [u8; 3] = [0x00, 0x00, 0x01];
const START_CODE_4: [u8; 4] = [0x00, 0x00, 0x00, 0x01];

/// H.264 RTP packetizer that fragments encoded H.264 NAL units into
/// RTP-sized payloads per RFC 6184.
///
/// Each call to [`packetize`] takes a raw Annex-B bitstream (as produced
/// by openh264), splits it into individual NAL units, and returns one or
/// more RTP payloads. NAL units that fit within the MTU are sent as
/// Single NAL Unit packets; larger ones are fragmented using FU-A.
#[derive(Debug)]
pub struct H264Packetizer {
    mtu: usize,
}

impl H264Packetizer {
    #[must_use]
    pub fn new() -> Self {
        Self { mtu: DEFAULT_MTU }
    }

    #[must_use]
    pub fn with_mtu(mtu: usize) -> Self {
        Self {
            mtu: mtu.max(100),
        }
    }

    /// Packetize an Annex-B H.264 bitstream into RTP payloads.
    ///
    /// Returns a `Vec<Vec<u8>>` where each inner `Vec` is the RTP payload
    /// (without the 12-byte RTP header). The caller sets the marker bit
    /// on the last packet of the access unit.
    pub fn packetize(&self, annex_b: &[u8]) -> Vec<Vec<u8>> {
        let nals = split_annex_b(annex_b);
        let max_payload = self.mtu.saturating_sub(RTP_HEADER_SIZE);

        let mut payloads = Vec::new();

        for nal in &nals {
            if nal.is_empty() {
                continue;
            }

            if nal.len() <= max_payload {
                payloads.push(nal.to_vec());
            } else {
                self.fragment_fu_a(nal, max_payload, &mut payloads);
            }
        }

        payloads
    }

    /// Fragment a single NAL unit into FU-A packets per RFC 6184 Section 5.8.
    ///
    /// FU indicator byte:  F | NRI (2 bits) | Type=28 (FU-A)
    /// FU header byte:     S | E | R | NAL-Type (5 bits)
    fn fragment_fu_a(
        &self,
        nal: &[u8],
        max_payload: usize,
        payloads: &mut Vec<Vec<u8>>,
    ) {
        let nal_header = nal[0];
        let f_nri = nal_header & 0xE0; // F bit + NRI
        let nal_type = nal_header & 0x1F;

        // FU indicator: same F+NRI as original, type=28 (FU-A)
        let fu_indicator = f_nri | 28;

        // 2 bytes for FU indicator + FU header
        let frag_max = max_payload.saturating_sub(2);
        let data = &nal[1..]; // skip original NAL header
        let mut offset = 0;

        while offset < data.len() {
            let is_first = offset == 0;
            let remaining = data.len() - offset;
            let chunk_size = remaining.min(frag_max);
            let is_last = offset + chunk_size >= data.len();

            let mut fu_header = nal_type;
            if is_first {
                fu_header |= 0x80; // S bit
            }
            if is_last {
                fu_header |= 0x40; // E bit
            }

            let mut pkt = Vec::with_capacity(2 + chunk_size);
            pkt.push(fu_indicator);
            pkt.push(fu_header);
            pkt.extend_from_slice(&data[offset..offset + chunk_size]);
            payloads.push(pkt);

            offset += chunk_size;
        }
    }
}

impl Default for H264Packetizer {
    fn default() -> Self {
        Self::new()
    }
}

/// Split an Annex-B bitstream into individual NAL units (without start codes).
fn split_annex_b(data: &[u8]) -> Vec<&[u8]> {
    let mut nals = Vec::new();
    let mut i = 0;

    // Find the first start code
    while i < data.len() {
        if i + 4 <= data.len() && data[i..i + 4] == START_CODE_4 {
            i += 4;
            break;
        }
        if i + 3 <= data.len() && data[i..i + 3] == START_CODE_3 {
            i += 3;
            break;
        }
        i += 1;
    }

    let mut nal_start = i;

    while i < data.len() {
        let found_4 = i + 4 <= data.len() && data[i..i + 4] == START_CODE_4;
        let found_3 = !found_4 && i + 3 <= data.len() && data[i..i + 3] == START_CODE_3;

        if found_4 || found_3 {
            if nal_start < i {
                nals.push(&data[nal_start..i]);
            }
            i += if found_4 { 4 } else { 3 };
            nal_start = i;
        } else {
            i += 1;
        }
    }

    if nal_start < data.len() {
        nals.push(&data[nal_start..]);
    }

    nals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_annex_b_basic() {
        let data = [
            0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0xC0, 0x1F, // SPS (4-byte start code)
            0x00, 0x00, 0x00, 0x01, 0x68, 0xCE, 0x38, 0x80, // PPS
            0x00, 0x00, 0x01, 0x65, 0xAA, 0xBB,             // IDR (3-byte start code)
        ];
        let nals = split_annex_b(&data);
        assert_eq!(nals.len(), 3);
        assert_eq!(nals[0][0] & 0x1F, 7); // SPS
        assert_eq!(nals[1][0] & 0x1F, 8); // PPS
        assert_eq!(nals[2][0] & 0x1F, 5); // IDR
    }

    #[test]
    fn single_nal_packet() {
        let p = H264Packetizer::new();
        // Small NAL that fits in one packet
        let mut annex_b = vec![0x00, 0x00, 0x00, 0x01];
        let nal = vec![0x65, 0xAA, 0xBB, 0xCC]; // IDR slice
        annex_b.extend_from_slice(&nal);

        let payloads = p.packetize(&annex_b);
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0], nal);
    }

    #[test]
    fn fu_a_fragmentation() {
        let p = H264Packetizer::with_mtu(50);
        let mut annex_b = vec![0x00, 0x00, 0x00, 0x01];
        let mut nal = vec![0x65]; // IDR type=5
        nal.extend(vec![0xAA; 200]); // 201 bytes total
        annex_b.extend_from_slice(&nal);

        let payloads = p.packetize(&annex_b);
        assert!(payloads.len() > 1, "should fragment into multiple FU-A packets");

        // First fragment: FU indicator type=28, FU header S=1
        let first = &payloads[0];
        assert_eq!(first[0] & 0x1F, 28); // FU-A type
        assert_ne!(first[1] & 0x80, 0);   // S bit set
        assert_eq!(first[1] & 0x40, 0);   // E bit clear
        assert_eq!(first[1] & 0x1F, 5);   // original NAL type (IDR)

        // Last fragment: E=1
        let last = payloads.last().expect("has last");
        assert_eq!(last[0] & 0x1F, 28);   // FU-A type
        assert_eq!(last[1] & 0x80, 0);     // S bit clear
        assert_ne!(last[1] & 0x40, 0);     // E bit set

        // Reassemble: skip FU-A headers, reconstruct original NAL
        let mut reassembled = Vec::new();
        for (i, pkt) in payloads.iter().enumerate() {
            if i == 0 {
                // Reconstruct NAL header from FU indicator NRI + FU header type
                let nal_header = (pkt[0] & 0xE0) | (pkt[1] & 0x1F);
                reassembled.push(nal_header);
            }
            reassembled.extend_from_slice(&pkt[2..]);
        }
        assert_eq!(reassembled, nal);
    }

    #[test]
    fn multiple_nals() {
        let p = H264Packetizer::new();
        let mut annex_b = Vec::new();
        // SPS
        annex_b.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1F]);
        // PPS
        annex_b.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x68, 0xCE, 0x38, 0x80]);
        // IDR slice
        annex_b.extend_from_slice(&[0x00, 0x00, 0x01, 0x65, 0xAA, 0xBB]);

        let payloads = p.packetize(&annex_b);
        assert_eq!(payloads.len(), 3);
    }

    #[test]
    fn empty_input() {
        let p = H264Packetizer::new();
        let payloads = p.packetize(&[]);
        assert!(payloads.is_empty());
    }

    #[test]
    fn mtu_enforcement() {
        let mtu = 100;
        let p = H264Packetizer::with_mtu(mtu);
        let mut annex_b = vec![0x00, 0x00, 0x00, 0x01];
        let mut nal = vec![0x65];
        nal.extend(vec![0xCC; 500]);
        annex_b.extend_from_slice(&nal);

        let payloads = p.packetize(&annex_b);
        let max_payload = mtu - RTP_HEADER_SIZE;
        for pkt in &payloads {
            assert!(
                pkt.len() <= max_payload,
                "payload {} bytes exceeds max payload {} (MTU {})",
                pkt.len(),
                max_payload,
                mtu
            );
        }
    }
}
