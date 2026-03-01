use std::sync::atomic::{AtomicU64, Ordering};

use base64::Engine;
use bytes::Bytes;
use tracing::{debug, trace, warn};
use vscreen_core::error::CdpError;
use vscreen_core::frame::RawFrame;

use crate::protocol::{
    CdpRequest, ScreencastFrameAckParams, ScreencastFrameEvent, StartScreencastParams,
};

/// Manages screencast session state including generation tracking.
#[derive(Debug)]
pub struct ScreencastManager {
    generation: AtomicU64,
    active: std::sync::atomic::AtomicBool,
}

impl ScreencastManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            generation: AtomicU64::new(0),
            active: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Increment the generation counter (called on reconnect).
    pub fn bump_generation(&self) -> u64 {
        let new_gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        debug!(generation = new_gen, "screencast generation bumped");
        new_gen
    }

    /// Current generation value.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Build a `Page.startScreencast` request.
    #[must_use]
    pub fn start_request(&self, params: &StartScreencastParams) -> CdpRequest {
        self.active
            .store(true, std::sync::atomic::Ordering::SeqCst);
        CdpRequest::new(
            "Page.startScreencast",
            Some(serde_json::to_value(params).expect("infallible serialization")),
        )
    }

    /// Build a `Page.stopScreencast` request.
    #[must_use]
    pub fn stop_request(&self) -> CdpRequest {
        self.active
            .store(false, std::sync::atomic::Ordering::SeqCst);
        CdpRequest::new("Page.stopScreencast", None)
    }

    /// Build a `Page.screencastFrameAck` request.
    #[must_use]
    pub fn ack_request(session_id: u32) -> CdpRequest {
        let params = ScreencastFrameAckParams { session_id };
        CdpRequest::new(
            "Page.screencastFrameAck",
            Some(serde_json::to_value(&params).expect("infallible serialization")),
        )
    }

    /// Whether screencast is currently active.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.active.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Decode a screencast frame event into a `RawFrame`.
    ///
    /// Returns `None` if the frame belongs to a stale generation (after reconnect).
    ///
    /// # Errors
    /// Returns `CdpError` if base64 decoding fails.
    pub fn decode_frame(
        &self,
        event: &ScreencastFrameEvent,
        frame_generation: u64,
    ) -> Result<Option<RawFrame>, CdpError> {
        let current_gen = self.generation();
        if frame_generation < current_gen {
            trace!(
                frame_gen = frame_generation,
                current_gen,
                "dropping stale screencast frame"
            );
            return Ok(None);
        }

        let data = base64::engine::general_purpose::STANDARD
            .decode(&event.data)
            .map_err(|e| CdpError::Screencast(format!("base64 decode failed: {e}")))?;

        if data.is_empty() {
            warn!("received empty screencast frame");
            return Ok(None);
        }

        Ok(Some(RawFrame {
            data: Bytes::from(data),
            timestamp: std::time::Instant::now(),
            session_id: event.session_id,
        }))
    }
}

impl Default for ScreencastManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_tracking() {
        let mgr = ScreencastManager::new();
        assert_eq!(mgr.generation(), 0);
        assert_eq!(mgr.bump_generation(), 1);
        assert_eq!(mgr.bump_generation(), 2);
        assert_eq!(mgr.generation(), 2);
    }

    #[test]
    fn start_stop_request() {
        let mgr = ScreencastManager::new();
        assert!(!mgr.is_active());

        let req = mgr.start_request(&StartScreencastParams::default());
        assert_eq!(req.method, "Page.startScreencast");
        assert!(mgr.is_active());

        let req = mgr.stop_request();
        assert_eq!(req.method, "Page.stopScreencast");
        assert!(!mgr.is_active());
    }

    #[test]
    fn ack_request() {
        let req = ScreencastManager::ack_request(42);
        assert_eq!(req.method, "Page.screencastFrameAck");
        let params = req.params.expect("params");
        assert_eq!(params["sessionId"], 42);
    }

    #[test]
    fn decode_valid_frame() {
        let mgr = ScreencastManager::new();
        let jpeg_bytes = vec![0xFF, 0xD8, 0xFF, 0xE0]; // minimal JPEG header
        let b64 = base64::engine::general_purpose::STANDARD.encode(&jpeg_bytes);

        let event = ScreencastFrameEvent {
            data: b64,
            metadata: crate::protocol::ScreencastFrameMetadata {
                offset_top: 0.0,
                page_scale_factor: 1.0,
                device_width: 1920.0,
                device_height: 1080.0,
                scroll_offset_x: 0.0,
                scroll_offset_y: 0.0,
                timestamp: Some(0.0),
            },
            session_id: 1,
        };

        let frame = mgr
            .decode_frame(&event, 0)
            .expect("decode")
            .expect("frame");
        assert_eq!(frame.data.as_ref(), &jpeg_bytes);
        assert_eq!(frame.session_id, 1);
    }

    #[test]
    fn decode_stale_frame_dropped() {
        let mgr = ScreencastManager::new();
        mgr.bump_generation(); // gen = 1

        let event = ScreencastFrameEvent {
            data: base64::engine::general_purpose::STANDARD.encode([1, 2, 3]),
            metadata: crate::protocol::ScreencastFrameMetadata {
                offset_top: 0.0,
                page_scale_factor: 1.0,
                device_width: 1920.0,
                device_height: 1080.0,
                scroll_offset_x: 0.0,
                scroll_offset_y: 0.0,
                timestamp: None,
            },
            session_id: 1,
        };

        // frame_generation=0 < current_gen=1 → dropped
        let result = mgr.decode_frame(&event, 0).expect("decode");
        assert!(result.is_none());
    }

    #[test]
    fn decode_invalid_base64() {
        let mgr = ScreencastManager::new();
        let event = ScreencastFrameEvent {
            data: "!!!not-base64!!!".into(),
            metadata: crate::protocol::ScreencastFrameMetadata {
                offset_top: 0.0,
                page_scale_factor: 1.0,
                device_width: 1920.0,
                device_height: 1080.0,
                scroll_offset_x: 0.0,
                scroll_offset_y: 0.0,
                timestamp: None,
            },
            session_id: 1,
        };

        let result = mgr.decode_frame(&event, 0);
        assert!(result.is_err());
    }

    #[test]
    fn decode_empty_frame_returns_none() {
        let mgr = ScreencastManager::new();
        let event = ScreencastFrameEvent {
            data: base64::engine::general_purpose::STANDARD.encode([]),
            metadata: crate::protocol::ScreencastFrameMetadata {
                offset_top: 0.0,
                page_scale_factor: 1.0,
                device_width: 1920.0,
                device_height: 1080.0,
                scroll_offset_x: 0.0,
                scroll_offset_y: 0.0,
                timestamp: None,
            },
            session_id: 1,
        };

        let result = mgr.decode_frame(&event, 0).expect("decode");
        assert!(result.is_none());
    }
}
