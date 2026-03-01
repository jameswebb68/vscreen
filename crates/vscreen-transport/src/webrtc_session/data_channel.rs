use tokio::sync::mpsc;
use tracing::{debug, warn};
use vscreen_core::error::TransportError;
use vscreen_core::event::{InputEvent, PeerInputEvent};
use vscreen_core::instance::PeerId;

const INPUT_CHANNEL_CAPACITY: usize = 100;

/// Manages the input DataChannel for a peer.
///
/// Parses compact input events from the DataChannel and sends them
/// to the instance's input handler via a bounded mpsc channel.
#[derive(Debug)]
pub struct DataChannelHandler {
    peer_id: PeerId,
    input_tx: mpsc::Sender<PeerInputEvent>,
}

impl DataChannelHandler {
    /// Create a new handler and return the receiver for input events.
    #[must_use]
    pub fn new(peer_id: PeerId) -> (Self, mpsc::Receiver<PeerInputEvent>) {
        let (input_tx, input_rx) = mpsc::channel(INPUT_CHANNEL_CAPACITY);
        (Self { peer_id, input_tx }, input_rx)
    }

    /// Create a handler with an existing sender (for shared input channels).
    #[must_use]
    pub fn with_sender(peer_id: PeerId, input_tx: mpsc::Sender<PeerInputEvent>) -> Self {
        Self { peer_id, input_tx }
    }

    /// Handle a raw message from the DataChannel.
    ///
    /// # Errors
    /// Returns `TransportError` if the message cannot be parsed.
    pub fn handle_message(&self, data: &[u8]) -> Result<(), TransportError> {
        let text = std::str::from_utf8(data)
            .map_err(|e| TransportError::DataChannel(format!("invalid UTF-8: {e}")))?;

        let event: InputEvent = serde_json::from_str(text)
            .map_err(|e| TransportError::DataChannel(format!("parse error: {e}")))?;

        let peer_event = PeerInputEvent {
            peer_id: self.peer_id,
            event,
        };

        match self.input_tx.try_send(peer_event) {
            Ok(()) => {
                debug!(peer = %self.peer_id, "forwarded input event");
                Ok(())
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                warn!(peer = %self.peer_id, "input channel full, dropping event");
                Ok(()) // Drop is acceptable for input events
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                Err(TransportError::DataChannel("input channel closed".into()))
            }
        }
    }

    /// Peer ID of this handler.
    #[must_use]
    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }
}

/// Parse a raw DataChannel message into an `InputEvent`.
///
/// # Errors
/// Returns `TransportError` if parsing fails.
pub fn parse_input_message(data: &[u8]) -> Result<InputEvent, TransportError> {
    let text = std::str::from_utf8(data)
        .map_err(|e| TransportError::DataChannel(format!("invalid UTF-8: {e}")))?;

    serde_json::from_str(text)
        .map_err(|e| TransportError::DataChannel(format!("parse error: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mouse_move() {
        let data = br#"{"t":"mm","x":100,"y":200,"m":0}"#;
        let event = parse_input_message(data).expect("parse");
        assert!(event.is_mouse());
    }

    #[test]
    fn parse_key_down() {
        let data = br#"{"t":"kd","key":"a","code":"KeyA","m":0}"#;
        let event = parse_input_message(data).expect("parse");
        assert!(event.is_keyboard());
    }

    #[test]
    fn reject_invalid_utf8() {
        let data = &[0xFF, 0xFE];
        assert!(parse_input_message(data).is_err());
    }

    #[test]
    fn reject_invalid_json() {
        let data = b"not json";
        assert!(parse_input_message(data).is_err());
    }

    #[tokio::test]
    async fn handler_forwards_events() {
        let peer = PeerId::new();
        let (handler, mut rx) = DataChannelHandler::new(peer);

        let data = br#"{"t":"mm","x":50,"y":75,"m":0}"#;
        handler.handle_message(data).expect("handle");

        let event = rx.try_recv().expect("receive");
        assert_eq!(event.peer_id, peer);
        assert!(event.event.is_mouse());
    }

    #[tokio::test]
    async fn handler_drops_on_full_channel() {
        let peer = PeerId::new();
        let (tx, _rx) = mpsc::channel(1); // Tiny channel
        let handler = DataChannelHandler::with_sender(peer, tx);

        // Fill the channel
        let data = br#"{"t":"mm","x":0,"y":0,"m":0}"#;
        handler.handle_message(data).expect("first");

        // This should succeed (drop policy)
        let result = handler.handle_message(data);
        assert!(result.is_ok());
    }
}
