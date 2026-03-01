use std::sync::atomic::{AtomicBool, Ordering};

use bytes::Bytes;
use vpx_sys;
use tracing::{debug, trace};
use vscreen_core::config::VideoConfig;
use vscreen_core::error::VideoError;
use vscreen_core::frame::{EncodedPacket, I420Buffer, VideoCodec};

/// Low-level wrapper around the libvpx C encoder.
///
/// All unsafe FFI calls are confined to this struct.
struct VpxCtx {
    ctx: vpx_sys::vpx_codec_ctx_t,
    width: u32,
    height: u32,
}

// SAFETY: VpxCtx owns all its FFI resources (vpx_codec_ctx_t) exclusively.
// It is only accessed via &mut self from a single thread (the dedicated
// encode thread spawned in supervisor::start). No aliased pointers are
// shared across threads. The underlying libvpx state is not thread-safe,
// but exclusive ownership via &mut guarantees single-threaded access.
unsafe impl Send for VpxCtx {}

impl VpxCtx {
    fn new(config: &VideoConfig) -> Result<Self, VideoError> {
        unsafe {
            let iface = vpx_sys::vpx_codec_vp9_cx();
            if iface.is_null() {
                return Err(VideoError::EncodeFailed("vpx_codec_vp9_cx returned null".into()));
            }

            let mut cfg = std::mem::MaybeUninit::<vpx_sys::vpx_codec_enc_cfg_t>::zeroed().assume_init();
            let ret = vpx_sys::vpx_codec_enc_config_default(iface, &mut cfg, 0);
            if ret != vpx_sys::VPX_CODEC_OK {
                return Err(VideoError::EncodeFailed(format!("config default: {ret:?}")));
            }

            cfg.g_w = config.width;
            cfg.g_h = config.height;
            cfg.g_timebase.num = 1;
            cfg.g_timebase.den = config.framerate as i32;
            cfg.rc_target_bitrate = config.bitrate_kbps;
            cfg.g_threads = 8;
            cfg.g_error_resilient = vpx_sys::VPX_ERROR_RESILIENT_DEFAULT;
            cfg.g_lag_in_frames = 0;
            cfg.rc_end_usage = vpx_sys::vpx_rc_mode::VPX_CBR;
            cfg.kf_mode = vpx_sys::vpx_kf_mode::VPX_KF_AUTO;
            cfg.kf_max_dist = config.keyframe_interval;

            let mut ctx = std::mem::MaybeUninit::<vpx_sys::vpx_codec_ctx_t>::zeroed().assume_init();
            let ret = vpx_sys::vpx_codec_enc_init_ver(
                &mut ctx,
                iface,
                &cfg,
                0,
                vpx_sys::VPX_ENCODER_ABI_VERSION as i32,
            );
            if ret != vpx_sys::VPX_CODEC_OK {
                return Err(VideoError::EncodeFailed(format!("enc init: {ret:?}")));
            }

            // Set cpu-used for realtime performance
            let cpu_used = config.cpu_used as std::os::raw::c_int;
            vpx_sys::vpx_codec_control_(
                &mut ctx,
                vpx_sys::vp8e_enc_control_id::VP8E_SET_CPUUSED as _,
                cpu_used,
            );
            // Enable row-level multi-threading
            vpx_sys::vpx_codec_control_(
                &mut ctx,
                vpx_sys::vp8e_enc_control_id::VP9E_SET_ROW_MT as _,
                1 as std::os::raw::c_int,
            );
            // Use 4 tile columns (2^2) for parallel encoding of 1080p+
            vpx_sys::vpx_codec_control_(
                &mut ctx,
                vpx_sys::vp8e_enc_control_id::VP9E_SET_TILE_COLUMNS as _,
                2 as std::os::raw::c_int,
            );
            // Lower static threshold to bias toward encoding motion over static regions
            vpx_sys::vpx_codec_control_(
                &mut ctx,
                vpx_sys::vp8e_enc_control_id::VP8E_SET_STATIC_THRESHOLD as _,
                100 as std::os::raw::c_uint,
            );

            Ok(Self {
                ctx,
                width: config.width,
                height: config.height,
            })
        }
    }

    fn encode(&mut self, pts: i64, i420_data: &[u8], force_keyframe: bool) -> Result<Vec<EncodedFrame>, VideoError> {
        let expected_size = (self.width as usize * self.height as usize * 3) / 2;
        if i420_data.len() < expected_size {
            return Err(VideoError::EncodeFailed(format!(
                "I420 buffer too small: got {} bytes, need at least {}",
                i420_data.len(),
                expected_size
            )));
        }

        const MAX_FRAME_SIZE: usize = 64 * 1024 * 1024;

        unsafe {
            let mut image = std::mem::MaybeUninit::<vpx_sys::vpx_image_t>::zeroed().assume_init();
            let ptr = vpx_sys::vpx_img_wrap(
                &mut image,
                vpx_sys::vpx_img_fmt::VPX_IMG_FMT_I420,
                self.width,
                self.height,
                1,
                i420_data.as_ptr() as *mut _,
            );
            if ptr.is_null() {
                return Err(VideoError::EncodeFailed("vpx_img_wrap failed".into()));
            }

            let flags: std::os::raw::c_ulong = if force_keyframe {
                vpx_sys::VPX_EFLAG_FORCE_KF as std::os::raw::c_ulong
            } else {
                0
            };

            let ret = vpx_sys::vpx_codec_encode(
                &mut self.ctx,
                &image,
                pts,
                1,
                flags as i64,
                vpx_sys::VPX_DL_REALTIME as std::os::raw::c_ulong,
            );
            if ret != vpx_sys::VPX_CODEC_OK {
                return Err(VideoError::EncodeFailed(format!("encode: {ret:?}")));
            }

            let mut frames = Vec::new();
            let mut iter: vpx_sys::vpx_codec_iter_t = std::ptr::null();
            loop {
                let pkt = vpx_sys::vpx_codec_get_cx_data(&mut self.ctx, &mut iter);
                if pkt.is_null() {
                    break;
                }
                if (*pkt).kind == vpx_sys::vpx_codec_cx_pkt_kind::VPX_CODEC_CX_FRAME_PKT {
                    let f = &(*pkt).data.frame;
                    if f.sz > MAX_FRAME_SIZE {
                        return Err(VideoError::EncodeFailed(format!(
                            "encoded frame size {} exceeds maximum {}",
                            f.sz, MAX_FRAME_SIZE
                        )));
                    }
                    let data = std::slice::from_raw_parts(f.buf as *const u8, f.sz);
                    frames.push(EncodedFrame {
                        data: data.to_vec(),
                        key: (f.flags & vpx_sys::VPX_FRAME_IS_KEY) != 0,
                    });
                }
            }

            Ok(frames)
        }
    }
}

impl Drop for VpxCtx {
    fn drop(&mut self) {
        unsafe {
            vpx_sys::vpx_codec_destroy(&mut self.ctx);
        }
    }
}

struct EncodedFrame {
    data: Vec<u8>,
    key: bool,
}

/// VP9 encoder using libvpx with full control over keyframes and configuration.
pub struct Vp9Encoder {
    ctx: VpxCtx,
    config: VideoConfig,
    frame_count: u64,
    keyframe_requested: AtomicBool,
    i420_buf: Vec<u8>,
}

impl std::fmt::Debug for Vp9Encoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vp9Encoder")
            .field("config", &self.config)
            .field("frame_count", &self.frame_count)
            .finish_non_exhaustive()
    }
}

impl Vp9Encoder {
    /// Create a new VP9 encoder with the given config.
    ///
    /// # Errors
    /// Returns `VideoError` if the configuration is invalid or libvpx init fails.
    pub fn new(config: VideoConfig) -> Result<Self, VideoError> {
        if config.width == 0 || config.height == 0 {
            return Err(VideoError::InvalidResolution {
                width: config.width,
                height: config.height,
            });
        }
        if config.width % 2 != 0 || config.height % 2 != 0 {
            return Err(VideoError::InvalidResolution {
                width: config.width,
                height: config.height,
            });
        }

        let ctx = VpxCtx::new(&config)?;

        let buf_size = I420Buffer::y_size(config.width, config.height)
            + 2 * I420Buffer::uv_size(config.width, config.height);

        debug!(
            width = config.width,
            height = config.height,
            bitrate_kbps = config.bitrate_kbps,
            framerate = config.framerate,
            cpu_used = config.cpu_used,
            "VP9 encoder initialized (libvpx)"
        );

        Ok(Self {
            ctx,
            config,
            frame_count: 0,
            keyframe_requested: AtomicBool::new(true),
            i420_buf: vec![0u8; buf_size],
        })
    }

    fn flatten_i420(&mut self, frame: &I420Buffer) {
        let y_len = frame.y.len();
        let u_len = frame.u.len();
        self.i420_buf[..y_len].copy_from_slice(&frame.y);
        self.i420_buf[y_len..y_len + u_len].copy_from_slice(&frame.u);
        self.i420_buf[y_len + u_len..y_len + u_len + frame.v.len()].copy_from_slice(&frame.v);
    }
}

impl vscreen_core::traits::VideoEncoder for Vp9Encoder {
    fn encode(&mut self, frame: &I420Buffer) -> Result<EncodedPacket, VideoError> {
        if frame.width != self.config.width || frame.height != self.config.height {
            return Err(VideoError::InvalidResolution {
                width: frame.width,
                height: frame.height,
            });
        }

        self.flatten_i420(frame);

        let pts = i64::try_from(self.frame_count).unwrap_or(0);
        let force_kf = self.keyframe_requested.swap(false, Ordering::SeqCst);

        let frames = self.ctx.encode(pts, &self.i420_buf, force_kf)?;

        let mut encoded_data = Vec::new();
        let mut is_keyframe = false;

        for f in &frames {
            encoded_data.extend_from_slice(&f.data);
            if f.key {
                is_keyframe = true;
            }
        }

        if encoded_data.is_empty() {
            is_keyframe = self.frame_count == 0;
        }

        let packet = EncodedPacket {
            data: Bytes::from(encoded_data),
            is_keyframe,
            pts: self.frame_count,
            duration: Some(1000 / u64::from(self.config.framerate)),
            codec: Some(VideoCodec::Vp9),
        };

        self.frame_count += 1;

        trace!(
            pts = packet.pts,
            is_keyframe = packet.is_keyframe,
            size = packet.size(),
            "encoded VP9 frame"
        );

        Ok(packet)
    }

    fn request_keyframe(&mut self) {
        self.keyframe_requested.store(true, Ordering::SeqCst);
        debug!("keyframe requested for next encode");
    }

    fn reconfigure(&mut self, config: VideoConfig) -> Result<(), VideoError> {
        if config.width == 0 || config.height == 0 {
            return Err(VideoError::InvalidResolution {
                width: config.width,
                height: config.height,
            });
        }
        if config.width % 2 != 0 || config.height % 2 != 0 {
            return Err(VideoError::InvalidResolution {
                width: config.width,
                height: config.height,
            });
        }

        self.ctx = VpxCtx::new(&config)?;

        let buf_size = I420Buffer::y_size(config.width, config.height)
            + 2 * I420Buffer::uv_size(config.width, config.height);
        self.i420_buf.resize(buf_size, 0);

        debug!(
            width = config.width,
            height = config.height,
            bitrate_kbps = config.bitrate_kbps,
            "VP9 encoder reconfigured"
        );

        self.config = config;
        self.keyframe_requested.store(true, Ordering::SeqCst);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use vscreen_core::traits::VideoEncoder;

    use super::*;

    fn make_i420(width: u32, height: u32) -> I420Buffer {
        I420Buffer {
            y: vec![128u8; I420Buffer::y_size(width, height)],
            u: vec![128u8; I420Buffer::uv_size(width, height)],
            v: vec![128u8; I420Buffer::uv_size(width, height)],
            width,
            height,
            timestamp: Instant::now(),
        }
    }

    #[test]
    fn encode_produces_packet() {
        let config = VideoConfig::default();
        let mut encoder = Vp9Encoder::new(config).expect("init");
        let frame = make_i420(1920, 1080);
        let packet = encoder.encode(&frame).expect("encode");
        assert!(packet.is_keyframe);
        assert!(!packet.data.is_empty());
    }

    #[test]
    fn second_frame_not_keyframe() {
        let config = VideoConfig::default();
        let mut encoder = Vp9Encoder::new(config).expect("init");
        let frame = make_i420(1920, 1080);
        let _ = encoder.encode(&frame).expect("encode 1");
        let pkt2 = encoder.encode(&frame).expect("encode 2");
        assert!(!pkt2.is_keyframe);
    }

    #[test]
    fn keyframe_request() {
        let config = VideoConfig::default();
        let mut encoder = Vp9Encoder::new(config).expect("init");
        let frame = make_i420(1920, 1080);
        let _ = encoder.encode(&frame).expect("encode 1");
        encoder.request_keyframe();
        let pkt = encoder.encode(&frame).expect("encode 2");
        assert!(pkt.is_keyframe);
    }

    #[test]
    fn reject_zero_resolution() {
        let mut config = VideoConfig::default();
        config.width = 0;
        assert!(Vp9Encoder::new(config).is_err());
    }

    #[test]
    fn reject_mismatched_frame() {
        let config = VideoConfig::default();
        let mut encoder = Vp9Encoder::new(config).expect("init");
        let frame = make_i420(640, 480);
        assert!(encoder.encode(&frame).is_err());
    }

    #[test]
    fn reconfigure() {
        let config = VideoConfig::default();
        let mut encoder = Vp9Encoder::new(config).expect("init");

        let new_config = VideoConfig {
            width: 1280,
            height: 720,
            ..VideoConfig::default()
        };
        encoder.reconfigure(new_config).expect("reconfig");

        let frame = make_i420(1280, 720);
        let pkt = encoder.encode(&frame).expect("encode");
        assert!(pkt.is_keyframe);
    }

    #[test]
    fn pts_increments() {
        let config = VideoConfig::default();
        let mut encoder = Vp9Encoder::new(config).expect("init");
        let frame = make_i420(1920, 1080);

        let p0 = encoder.encode(&frame).expect("e0");
        let p1 = encoder.encode(&frame).expect("e1");
        let p2 = encoder.encode(&frame).expect("e2");

        assert_eq!(p0.pts, 0);
        assert_eq!(p1.pts, 1);
        assert_eq!(p2.pts, 2);
    }

    #[test]
    fn reject_odd_dimensions() {
        let config = VideoConfig {
            width: 1921,
            height: 1080,
            ..VideoConfig::default()
        };
        assert!(Vp9Encoder::new(config).is_err());
    }
}
