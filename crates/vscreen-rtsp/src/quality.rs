use std::fmt;

use serde::{Deserialize, Serialize};

/// Predefined audio quality tiers with standard bitrate/channel configurations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QualityTier {
    /// 32 kbps, mono
    Low,
    /// 64 kbps, mono
    Medium,
    /// 128 kbps, stereo
    Standard,
    /// 256 kbps, stereo (master quality, no transcoding needed)
    High,
    /// Arbitrary bitrate and channel count
    Custom {
        kbps: u32,
        channels: u16,
    },
}

impl QualityTier {
    /// Bitrate in bits per second.
    #[must_use]
    pub const fn bitrate_bps(&self) -> u32 {
        match self {
            Self::Low => 32_000,
            Self::Medium => 64_000,
            Self::Standard => 128_000,
            Self::High => 256_000,
            Self::Custom { kbps, .. } => *kbps * 1000,
        }
    }

    /// Bitrate in kilobits per second.
    #[must_use]
    pub const fn bitrate_kbps(&self) -> u32 {
        match self {
            Self::Low => 32,
            Self::Medium => 64,
            Self::Standard => 128,
            Self::High => 256,
            Self::Custom { kbps, .. } => *kbps,
        }
    }

    /// Number of audio channels for this tier.
    #[must_use]
    pub const fn channels(&self) -> u16 {
        match self {
            Self::Low | Self::Medium => 1,
            Self::Standard | Self::High => 2,
            Self::Custom { channels, .. } => *channels,
        }
    }

    /// Whether this tier matches the master encoder output (no transcoding).
    #[must_use]
    pub const fn is_master(&self) -> bool {
        matches!(self, Self::High)
    }

    /// Whether transcoding is required for this tier.
    #[must_use]
    pub const fn needs_transcode(&self) -> bool {
        !self.is_master()
    }

    /// Parse a tier from a string name (e.g., from RTSP URL query params).
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_lowercase().as_str() {
            "low" => Some(Self::Low),
            "medium" | "med" => Some(Self::Medium),
            "standard" | "std" => Some(Self::Standard),
            "high" | "hq" => Some(Self::High),
            _ => None,
        }
    }

    /// Create a custom tier from a kbps value and optional channel count.
    #[must_use]
    pub fn custom(kbps: u32, channels: Option<u16>) -> Self {
        let channels = channels.unwrap_or(if kbps >= 96 { 2 } else { 1 });
        Self::Custom { kbps, channels }
    }
}

impl Default for QualityTier {
    fn default() -> Self {
        Self::Standard
    }
}

impl fmt::Display for QualityTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Low => write!(f, "low (32kbps mono)"),
            Self::Medium => write!(f, "medium (64kbps mono)"),
            Self::Standard => write!(f, "standard (128kbps stereo)"),
            Self::High => write!(f, "high (256kbps stereo)"),
            Self::Custom { kbps, channels } => {
                let ch = if *channels == 1 { "mono" } else { "stereo" };
                write!(f, "custom ({kbps}kbps {ch})")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_bitrates() {
        assert_eq!(QualityTier::Low.bitrate_kbps(), 32);
        assert_eq!(QualityTier::Medium.bitrate_kbps(), 64);
        assert_eq!(QualityTier::Standard.bitrate_kbps(), 128);
        assert_eq!(QualityTier::High.bitrate_kbps(), 256);
        assert_eq!(QualityTier::custom(192, None).bitrate_kbps(), 192);
    }

    #[test]
    fn tier_channels() {
        assert_eq!(QualityTier::Low.channels(), 1);
        assert_eq!(QualityTier::Medium.channels(), 1);
        assert_eq!(QualityTier::Standard.channels(), 2);
        assert_eq!(QualityTier::High.channels(), 2);
    }

    #[test]
    fn tier_transcode() {
        assert!(QualityTier::Low.needs_transcode());
        assert!(!QualityTier::High.needs_transcode());
    }

    #[test]
    fn tier_from_name() {
        assert_eq!(QualityTier::from_name("low"), Some(QualityTier::Low));
        assert_eq!(QualityTier::from_name("HIGH"), Some(QualityTier::High));
        assert_eq!(QualityTier::from_name("hq"), Some(QualityTier::High));
        assert!(QualityTier::from_name("invalid").is_none());
    }

    #[test]
    fn custom_auto_channels() {
        assert_eq!(QualityTier::custom(48, None).channels(), 1);
        assert_eq!(QualityTier::custom(128, None).channels(), 2);
        assert_eq!(QualityTier::custom(128, Some(1)).channels(), 1);
    }
}
