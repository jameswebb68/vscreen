use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Path, State, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures_util::SinkExt;
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, error, info, warn};
use vscreen_core::event::PeerInputEvent;
use vscreen_core::instance::{InstanceId, PeerId};
use vscreen_transport::webrtc_session::session::PeerSession;
use vscreen_transport::webrtc_session::signaling::SignalingMessage;

use crate::state::AppState;

/// WebSocket upgrade handler for signaling.
pub async fn ws_signal(
    State(state): State<AppState>,
    Path(instance_id): Path<String>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let id = InstanceId::from(instance_id.as_str());

    match state.registry.get(&id) {
        Ok(entry) => {
            let current_state = entry.state_rx.borrow().clone();
            if !current_state.can_accept_peers() && !matches!(current_state, vscreen_core::instance::InstanceState::Created) {
                let error_msg = SignalingMessage::Error {
                    code: "INVALID_STATE".into(),
                    message: format!("instance {} is in state: {}", id, current_state),
                };
                return ws.on_upgrade(move |mut socket| async move {
                    let json = serde_json::to_string(&error_msg).unwrap_or_default();
                    let _ = socket.send(Message::Text(json.into())).await;
                    let _ = socket.close().await;
                });
            }
        }
        Err(_) => {
            let error_msg = SignalingMessage::Error {
                code: "INSTANCE_NOT_FOUND".into(),
                message: format!("instance {instance_id} not found"),
            };
            return ws.on_upgrade(move |mut socket| async move {
                let json = serde_json::to_string(&error_msg).unwrap_or_default();
                let _ = socket.send(Message::Text(json.into())).await;
                let _ = socket.close().await;
            });
        }
    }

    ws.on_upgrade(move |socket| handle_signaling(socket, id, state))
}

async fn handle_signaling(mut socket: WebSocket, instance_id: InstanceId, state: AppState) {
    let peer_id = PeerId::new();
    metrics::gauge!("vscreen_active_peers").increment(1.0);
    info!(%instance_id, %peer_id, "signaling WebSocket connected");

    // H5: re-check instance existence after WebSocket upgrade (TOCTOU fix)
    if state.registry.get(&instance_id).is_err() {
        let err_msg = SignalingMessage::Error {
            code: "INSTANCE_NOT_FOUND".into(),
            message: format!("instance {} was removed during WebSocket upgrade", instance_id),
        };
        if let Ok(json) = serde_json::to_string(&err_msg) {
            let _ = socket.send(Message::Text(json.into())).await;
        }
        metrics::gauge!("vscreen_active_peers").decrement(1.0);
        return;
    }

    let (input_tx, mut video_rx, mut audio_rx, mut clipboard_rx, video_codec) = {
        if let Some(sup) = state.get_supervisor(&instance_id) {
            (
                sup.input_sender(),
                Some(sup.video_receiver()),
                Some(sup.audio_receiver()),
                Some(sup.clipboard_receiver()),
                sup.video_codec(),
            )
        } else {
            let (tx, _) = mpsc::channel::<PeerInputEvent>(100);
            (tx, None, None, None, vscreen_core::frame::VideoCodec::default())
        }
    };

    let (mut session, mut signaling_rx) = match PeerSession::new(peer_id, input_tx, video_codec).await {
        Ok(s) => s,
        Err(e) => {
            error!(?e, %peer_id, "failed to create peer session");
            let err_msg = SignalingMessage::Error {
                code: "SESSION_ERROR".into(),
                message: e.to_string(),
            };
            if let Ok(json) = serde_json::to_string(&err_msg) {
                let _ = socket.send(Message::Text(json.into())).await;
            }
            metrics::gauge!("vscreen_active_peers").decrement(1.0);
            return;
        }
    };

    let connected = SignalingMessage::Connected {
        peer_id: peer_id.to_string(),
    };
    if let Ok(json) = serde_json::to_string(&connected) {
        let _ = socket.send(Message::Text(json.into())).await;
    }

    loop {
        tokio::select! {
            biased;

            // Check for server shutdown
            () = state.cancel.cancelled() => {
                let disc_msg = SignalingMessage::Disconnected {
                    reason: "server shutting down".into(),
                };
                if let Ok(json) = serde_json::to_string(&disc_msg) {
                    let _ = socket.send(Message::Text(json.into())).await;
                }
                break;
            }
            // Forward server-side signaling (ICE candidates) to the client
            Some(sig_msg) = signaling_rx.recv() => {
                if let Ok(json) = sig_msg.to_json() {
                    if let Err(e) = socket.send(Message::Text(json.into())).await {
                        error!(?e, %peer_id, "failed to send signaling to client");
                        break;
                    }
                }
            }
            // Forward encoded video to the peer's WebRTC track
            video = async {
                if let Some(ref mut rx) = video_rx {
                    rx.recv().await
                } else {
                    std::future::pending().await
                }
            } => {
                match video {
                    Ok(packet) => {
                        if let Err(e) = session.write_video(&packet).await {
                            warn!(?e, %peer_id, "video write failed");
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!(%peer_id, n, "peer video lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        info!(%peer_id, "video broadcast closed");
                        video_rx = None;
                    }
                }
            }
            // Forward encoded audio to the peer's WebRTC track
            audio = async {
                if let Some(ref mut rx) = audio_rx {
                    rx.recv().await
                } else {
                    std::future::pending().await
                }
            } => {
                match audio {
                    Ok(packet) => {
                        if let Err(e) = session.write_audio(&packet).await {
                            warn!(?e, %peer_id, "audio write failed");
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!(%peer_id, n, "peer audio lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        info!(%peer_id, "audio broadcast closed");
                        audio_rx = None;
                    }
                }
            }
            // Forward clipboard content to the client
            clipboard = async {
                if let Some(ref mut rx) = clipboard_rx {
                    rx.recv().await
                } else {
                    std::future::pending().await
                }
            } => {
                match clipboard {
                    Ok(text) => {
                        let msg = SignalingMessage::Clipboard { text };
                        if let Ok(json) = msg.to_json() {
                            if let Err(e) = socket.send(Message::Text(json.into())).await {
                                error!(?e, %peer_id, "failed to send clipboard to client");
                                break;
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => {
                        info!(%peer_id, "clipboard broadcast closed");
                        clipboard_rx = None;
                    }
                }
            }
            // Process incoming messages from the client
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        match SignalingMessage::from_json(&text) {
                            Ok(sig_msg) => {
                                debug!(%peer_id, "received signaling message");
                                match session.handle_signaling(sig_msg).await {
                                    Ok(Some(response)) => {
                                        if let Ok(json) = response.to_json() {
                                            if let Err(e) = socket.send(Message::Text(json.into())).await {
                                                error!(?e, %peer_id, "failed to send signaling response");
                                                break;
                                            }
                                        }
                                        if matches!(response, SignalingMessage::Answer { .. }) {
                                            if let Some(sup) = state.get_supervisor(&instance_id) {
                                                sup.request_keyframe();
                                            }
                                        }
                                    }
                                    Ok(None) => {}
                                    Err(e) => {
                                        warn!(?e, %peer_id, "signaling error");
                                        let err_msg = SignalingMessage::Error {
                                            code: "SIGNALING_ERROR".into(),
                                            message: e.to_string(),
                                        };
                                        if let Ok(json) = serde_json::to_string(&err_msg) {
                                            let _ = socket.send(Message::Text(json.into())).await;
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                warn!(?e, %peer_id, "invalid signaling message");
                                let err_msg = SignalingMessage::Error {
                                    code: "PARSE_ERROR".into(),
                                    message: e.to_string(),
                                };
                                if let Ok(json) = serde_json::to_string(&err_msg) {
                                    let _ = socket.send(Message::Text(json.into())).await;
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        debug!(%peer_id, "signaling WebSocket closed by client");
                        break;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        warn!(?e, %peer_id, "signaling WebSocket error");
                        break;
                    }
                    None => {
                        debug!(%peer_id, "signaling WebSocket stream ended");
                        break;
                    }
                }
            }
        }
    }

    session.close().await;
    session.set_disconnected();
    metrics::gauge!("vscreen_active_peers").decrement(1.0);
    info!(%instance_id, %peer_id, "signaling WebSocket disconnected");
}
