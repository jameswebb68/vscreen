use std::net::SocketAddr;

use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use vscreen_core::config::AppConfig;
use vscreen_server::{build_router, AppState};

/// A test server harness that starts vscreen on a random port.
pub struct TestServer {
    pub addr: SocketAddr,
    pub state: AppState,
    pub cancel: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

impl TestServer {
    /// Start a test server on a random available port.
    pub async fn start() -> Self {
        Self::start_with_config(AppConfig::default()).await
    }

    /// Start a test server with a custom config.
    pub async fn start_with_config(mut config: AppConfig) -> Self {
        config.server.listen = "127.0.0.1:0".to_owned();

        let cancel = CancellationToken::new();
        let state = AppState::new(config, cancel.clone());
        let router = build_router(state.clone());

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let addr = listener.local_addr().expect("local addr");

        let cancel_clone = cancel.clone();
        let task = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(cancel_clone.cancelled_owned())
                .await
                .expect("test server");
        });

        Self {
            addr,
            state,
            cancel,
            task,
        }
    }

    /// Base URL for HTTP requests.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// WebSocket URL for signaling.
    pub fn ws_url(&self, instance_id: &str) -> String {
        format!("ws://{}/signal/{instance_id}", self.addr)
    }

    /// Stop the test server.
    pub async fn stop(self) {
        self.cancel.cancel();
        let _ = self.task.await;
    }
}
