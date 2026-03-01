use std::path::Path;

use serde::Deserialize;

use crate::error::ConfigError;
use crate::frame::VideoCodec;

// ---------------------------------------------------------------------------
// Top-level config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    #[serde(default = "ServerConfig::default")]
    pub server: ServerConfig,
    #[serde(default)]
    pub webrtc: WebRtcConfig,
    #[serde(default)]
    pub defaults: DefaultsConfig,
    #[serde(default)]
    pub limits: LimitsConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            webrtc: WebRtcConfig::default(),
            defaults: DefaultsConfig::default(),
            limits: LimitsConfig::default(),
            logging: LoggingConfig::default(),
        }
    }
}

impl AppConfig {
    /// Load config from a TOML file, falling back to defaults for missing fields.
    ///
    /// # Errors
    /// Returns `ConfigError` if the file cannot be read or parsed.
    pub fn from_file(path: &Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            ConfigError::File(format!("{}: {e}", path.display()))
        })?;
        let config: Self =
            toml::from_str(&content).map_err(|e| ConfigError::Parse(e.to_string()))?;
        Ok(config)
    }

    /// Load from a TOML string (useful for tests and env-var overrides).
    ///
    /// # Errors
    /// Returns `ConfigError` if the content cannot be parsed.
    pub fn from_str(content: &str) -> Result<Self, ConfigError> {
        toml::from_str(content).map_err(|e| ConfigError::Parse(e.to_string()))
    }

    /// Validate all config values.
    ///
    /// # Errors
    /// Returns a list of validation errors.
    pub fn validate(&self) -> Result<(), Vec<ConfigError>> {
        let mut errors = Vec::new();

        if self.server.listen.is_empty() {
            errors.push(ConfigError::InvalidValue {
                field: "server.listen",
                reason: "must not be empty",
            });
        }

        if self.limits.max_instances == 0 {
            errors.push(ConfigError::InvalidValue {
                field: "limits.max_instances",
                reason: "must be > 0",
            });
        }

        if self.limits.max_peers_per_instance == 0 {
            errors.push(ConfigError::InvalidValue {
                field: "limits.max_peers_per_instance",
                reason: "must be > 0",
            });
        }

        if self.limits.frame_queue_depth < 1 || self.limits.frame_queue_depth > 30 {
            errors.push(ConfigError::OutOfRange {
                field: "limits.frame_queue_depth",
                min: 1,
                max: 30,
            });
        }

        if self.defaults.video.framerate == 0 || self.defaults.video.framerate > 60 {
            errors.push(ConfigError::OutOfRange {
                field: "defaults.video.framerate",
                min: 1,
                max: 60,
            });
        }

        if self.defaults.video.bitrate_kbps == 0 {
            errors.push(ConfigError::InvalidValue {
                field: "defaults.video.bitrate_kbps",
                reason: "must be > 0",
            });
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

// ---------------------------------------------------------------------------
// Sub-configs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    #[serde(default = "default_listen")]
    pub listen: String,
    #[serde(default)]
    pub auth_token: Option<String>,
    #[serde(default)]
    pub tls: Option<TlsConfig>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen: default_listen(),
            auth_token: None,
            tls: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TlsConfig {
    pub cert_path: String,
    pub key_path: String,
}

fn default_listen() -> String {
    "0.0.0.0:8450".to_owned()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebRtcConfig {
    #[serde(default = "default_stun_servers")]
    pub stun_servers: Vec<String>,
    #[serde(default)]
    pub turn_servers: Vec<TurnServer>,
}

impl Default for WebRtcConfig {
    fn default() -> Self {
        Self {
            stun_servers: default_stun_servers(),
            turn_servers: Vec::new(),
        }
    }
}

fn default_stun_servers() -> Vec<String> {
    vec!["stun:stun.l.google.com:19302".to_owned()]
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnServer {
    pub url: String,
    pub username: String,
    pub credential: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefaultsConfig {
    #[serde(default)]
    pub video: VideoConfig,
    #[serde(default)]
    pub audio: AudioConfig,
    #[serde(default)]
    pub rtp_output: Option<RtpOutputConfig>,
}

impl Default for DefaultsConfig {
    fn default() -> Self {
        Self {
            video: VideoConfig::default(),
            audio: AudioConfig::default(),
            rtp_output: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct VideoConfig {
    #[serde(default = "default_width")]
    pub width: u32,
    #[serde(default = "default_height")]
    pub height: u32,
    #[serde(default = "default_framerate")]
    pub framerate: u32,
    #[serde(default = "default_bitrate_kbps")]
    pub bitrate_kbps: u32,
    #[serde(default = "default_keyframe_interval")]
    pub keyframe_interval: u32,
    #[serde(default = "default_cpu_used")]
    pub cpu_used: u32,
    #[serde(default = "default_min_bitrate_kbps")]
    pub min_bitrate_kbps: u32,
    #[serde(default = "default_max_bitrate_kbps")]
    pub max_bitrate_kbps: u32,
    #[serde(default)]
    pub codec: VideoCodec,
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            width: default_width(),
            height: default_height(),
            framerate: default_framerate(),
            bitrate_kbps: default_bitrate_kbps(),
            keyframe_interval: default_keyframe_interval(),
            cpu_used: default_cpu_used(),
            min_bitrate_kbps: default_min_bitrate_kbps(),
            max_bitrate_kbps: default_max_bitrate_kbps(),
            codec: VideoCodec::default(),
        }
    }
}

const fn default_width() -> u32 {
    1920
}
const fn default_height() -> u32 {
    1080
}
const fn default_framerate() -> u32 {
    30
}
const fn default_bitrate_kbps() -> u32 {
    6000
}
const fn default_keyframe_interval() -> u32 {
    60
}
const fn default_min_bitrate_kbps() -> u32 {
    1000
}
const fn default_max_bitrate_kbps() -> u32 {
    10000
}
const fn default_cpu_used() -> u32 {
    8
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct AudioConfig {
    #[serde(default = "default_sample_rate")]
    pub sample_rate: u32,
    #[serde(default = "default_channels")]
    pub channels: u16,
    #[serde(default = "default_audio_bitrate")]
    pub bitrate_kbps: u32,
    #[serde(default = "default_frame_duration_ms")]
    pub frame_duration_ms: u32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: default_sample_rate(),
            channels: default_channels(),
            bitrate_kbps: default_audio_bitrate(),
            frame_duration_ms: default_frame_duration_ms(),
        }
    }
}

const fn default_sample_rate() -> u32 {
    48000
}
const fn default_channels() -> u16 {
    2
}
const fn default_audio_bitrate() -> u32 {
    128
}
const fn default_frame_duration_ms() -> u32 {
    20
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct RtpOutputConfig {
    pub address: String,
    pub port: u16,
    #[serde(default)]
    pub multicast: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitsConfig {
    #[serde(default = "default_max_instances")]
    pub max_instances: u32,
    #[serde(default = "default_max_peers")]
    pub max_peers_per_instance: u32,
    #[serde(default = "default_frame_queue")]
    pub frame_queue_depth: usize,
    #[serde(default = "default_max_frame_size")]
    pub max_frame_size_bytes: usize,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            max_instances: default_max_instances(),
            max_peers_per_instance: default_max_peers(),
            frame_queue_depth: default_frame_queue(),
            max_frame_size_bytes: default_max_frame_size(),
        }
    }
}

const fn default_max_instances() -> u32 {
    16
}
const fn default_max_peers() -> u32 {
    8
}
const fn default_frame_queue() -> usize {
    3
}
const fn default_max_frame_size() -> usize {
    2 * 1024 * 1024 // 2 MB
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default)]
    pub json: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            json: false,
        }
    }
}

fn default_log_level() -> String {
    "info".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_validates() {
        let config = AppConfig::default();
        config.validate().expect("default config should be valid");
    }

    #[test]
    fn parse_minimal_toml() {
        let toml = "";
        let config = AppConfig::from_str(toml).expect("parse empty toml");
        assert_eq!(config.server.listen, "0.0.0.0:8450");
        assert_eq!(config.defaults.video.framerate, 30);
    }

    #[test]
    fn parse_full_toml() {
        let toml = r#"
[server]
listen = "127.0.0.1:9000"

[webrtc]
stun_servers = ["stun:stun.example.com:3478"]

[defaults.video]
width = 1280
height = 720
framerate = 25
bitrate_kbps = 1500

[defaults.audio]
sample_rate = 48000
channels = 1

[limits]
max_instances = 8
max_peers_per_instance = 4
frame_queue_depth = 5

[logging]
level = "debug"
json = true
"#;
        let config = AppConfig::from_str(toml).expect("parse full toml");
        assert_eq!(config.server.listen, "127.0.0.1:9000");
        assert_eq!(config.defaults.video.width, 1280);
        assert_eq!(config.defaults.video.framerate, 25);
        assert_eq!(config.defaults.audio.channels, 1);
        assert_eq!(config.limits.max_instances, 8);
        assert!(config.logging.json);
    }

    #[test]
    fn validation_catches_zero_max_instances() {
        let mut config = AppConfig::default();
        config.limits.max_instances = 0;
        let errors = config.validate().unwrap_err();
        assert!(errors.iter().any(|e| e.to_string().contains("max_instances")));
    }

    #[test]
    fn validation_catches_out_of_range_frame_queue() {
        let mut config = AppConfig::default();
        config.limits.frame_queue_depth = 50;
        let errors = config.validate().unwrap_err();
        assert!(errors
            .iter()
            .any(|e| e.to_string().contains("frame_queue_depth")));
    }

    #[test]
    fn validation_catches_zero_framerate() {
        let mut config = AppConfig::default();
        config.defaults.video.framerate = 0;
        let errors = config.validate().unwrap_err();
        assert!(errors.iter().any(|e| e.to_string().contains("framerate")));
    }

    #[test]
    fn validation_catches_zero_bitrate() {
        let mut config = AppConfig::default();
        config.defaults.video.bitrate_kbps = 0;
        let errors = config.validate().unwrap_err();
        assert!(errors.iter().any(|e| e.to_string().contains("bitrate")));
    }

    #[test]
    fn unknown_field_rejected() {
        let toml = r#"
[server]
listen = "0.0.0.0:8450"
unknown_field = true
"#;
        let result = AppConfig::from_str(toml);
        assert!(result.is_err());
    }

    #[test]
    fn video_config_defaults() {
        let vc = VideoConfig::default();
        assert_eq!(vc.width, 1920);
        assert_eq!(vc.height, 1080);
        assert_eq!(vc.framerate, 30);
        assert_eq!(vc.bitrate_kbps, 6000);
    }

    #[test]
    fn audio_config_defaults() {
        let ac = AudioConfig::default();
        assert_eq!(ac.sample_rate, 48000);
        assert_eq!(ac.channels, 2);
        assert_eq!(ac.bitrate_kbps, 128);
    }

    #[test]
    fn limits_config_defaults() {
        let lc = LimitsConfig::default();
        assert_eq!(lc.max_instances, 16);
        assert_eq!(lc.max_peers_per_instance, 8);
        assert_eq!(lc.frame_queue_depth, 3);
        assert_eq!(lc.max_frame_size_bytes, 2 * 1024 * 1024);
    }
}
