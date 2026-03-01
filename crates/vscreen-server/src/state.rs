use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tracing::info;
use vscreen_core::config::AppConfig;
use vscreen_core::error::VScreenError;
use vscreen_core::instance::{InstanceConfig, InstanceId, InstanceState};

use vscreen_core::frame::EncodedPacket;

use crate::lock_manager::InstanceLockManager;
use crate::supervisor::InstanceSupervisor;
use crate::vision::VisionClient;

/// Shared application state for the HTTP server.
#[derive(Debug, Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub registry: Arc<InstanceRegistry>,
    pub cancel: CancellationToken,
    /// Supervisors use interior mutability — no outer Mutex needed (C2).
    pub supervisors: Arc<DashMap<InstanceId, Arc<InstanceSupervisor>>>,
    pub lock_manager: Arc<InstanceLockManager>,
    /// When true, all lock checks are bypassed (single-agent mode).
    pub single_agent_mode: bool,
    /// RTSP session manager (set when RTSP server is started).
    pub rtsp_session_manager: Option<Arc<vscreen_rtsp::RtspSessionManager>>,
    /// RTSP server port (for constructing stream URLs).
    pub rtsp_port: u16,
    /// Vision LLM client for screenshot-based verification (None if not configured).
    pub vision_client: Option<Arc<VisionClient>>,
}

impl AppState {
    #[must_use]
    pub fn new(config: AppConfig, cancel: CancellationToken) -> Self {
        Self {
            config: Arc::new(config),
            registry: Arc::new(InstanceRegistry::new()),
            cancel,
            supervisors: Arc::new(DashMap::new()),
            lock_manager: Arc::new(InstanceLockManager::new()),
            single_agent_mode: false,
            rtsp_session_manager: None,
            rtsp_port: 0,
            vision_client: None,
        }
    }

    pub fn set_supervisor(&self, id: &str, sup: InstanceSupervisor) {
        self.supervisors
            .insert(InstanceId::from(id), Arc::new(sup));
    }

    #[must_use]
    pub fn get_supervisor(&self, id: &InstanceId) -> Option<Arc<InstanceSupervisor>> {
        self.supervisors.get(id).map(|entry| entry.value().clone())
    }

    /// Remove supervisor, stopping it first (H1: stop before drop).
    pub async fn remove_supervisor(&self, id: &InstanceId) {
        if let Some((_, sup)) = self.supervisors.remove(id) {
            sup.stop().await;
        }
    }
}

/// Registry of active instances.
///
/// Uses `DashMap` for concurrent atomic insert/remove (R1).
#[derive(Debug)]
pub struct InstanceRegistry {
    instances: DashMap<InstanceId, InstanceEntry>,
}

/// An entry in the instance registry.
#[derive(Debug, Clone)]
pub struct InstanceEntry {
    pub config: InstanceConfig,
    pub state_tx: watch::Sender<InstanceState>,
    pub state_rx: watch::Receiver<InstanceState>,
}

impl InstanceRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            instances: DashMap::new(),
        }
    }

    /// Create a new instance. Returns error if it already exists (R1: atomic insert).
    ///
    /// # Errors
    /// Returns `VScreenError::InstanceAlreadyExists` if the ID is taken.
    /// Returns `VScreenError::LimitExceeded` if at capacity.
    pub fn create(
        &self,
        config: InstanceConfig,
        max_instances: u32,
    ) -> Result<InstanceEntry, VScreenError> {
        let id = config.instance_id.clone();

        if !id.is_url_safe() {
            return Err(VScreenError::InvalidConfig(format!(
                "instance ID '{}' is not URL-safe (only alphanumeric, hyphens, underscores, dots allowed; 1-128 chars)",
                id.0
            )));
        }

        if self.instances.len() >= max_instances as usize {
            return Err(VScreenError::LimitExceeded(format!(
                "max instances ({max_instances}) reached"
            )));
        }

        let (state_tx, state_rx) = watch::channel(InstanceState::Created);
        let entry = InstanceEntry {
            config,
            state_tx,
            state_rx,
        };

        use dashmap::mapref::entry::Entry;
        match self.instances.entry(id.clone()) {
            Entry::Occupied(_) => {
                Err(VScreenError::InstanceAlreadyExists(id.0))
            }
            Entry::Vacant(vacant) => {
                let entry_clone = entry.clone();
                vacant.insert(entry);
                info!(instance_id = %id, "instance created");
                Ok(entry_clone)
            }
        }
    }

    /// Remove an instance. Returns error if it doesn't exist.
    ///
    /// # Errors
    /// Returns `VScreenError::InstanceNotFound` if the ID is not registered.
    pub fn remove(&self, id: &InstanceId) -> Result<InstanceEntry, VScreenError> {
        self.instances
            .remove(id)
            .map(|(_, entry)| {
                info!(instance_id = %id, "instance removed");
                entry
            })
            .ok_or_else(|| VScreenError::InstanceNotFound(id.0.clone()))
    }

    /// Get instance info (state snapshot).
    ///
    /// # Errors
    /// Returns `VScreenError::InstanceNotFound` if the ID is not registered.
    pub fn get(&self, id: &InstanceId) -> Result<InstanceEntry, VScreenError> {
        self.instances
            .get(id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| VScreenError::InstanceNotFound(id.0.clone()))
    }

    /// List all instance IDs.
    #[must_use]
    pub fn list_ids(&self) -> Vec<InstanceId> {
        self.instances
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Current number of instances.
    #[must_use]
    pub fn len(&self) -> usize {
        self.instances.len()
    }

    /// Whether the registry is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }
}

impl Default for InstanceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// InstanceLookup implementation for RTSP server integration
// ---------------------------------------------------------------------------

impl vscreen_rtsp::InstanceLookup for AppState {
    fn instance_exists(&self, instance_id: &str) -> bool {
        self.registry
            .get(&InstanceId::from(instance_id))
            .is_ok()
    }

    fn subscribe_audio(
        &self,
        instance_id: &str,
    ) -> Option<tokio::sync::broadcast::Receiver<EncodedPacket>> {
        self.get_supervisor(&InstanceId::from(instance_id))
            .map(|sup| sup.audio_broadcast().subscribe())
    }

    fn subscribe_video(
        &self,
        instance_id: &str,
    ) -> Option<tokio::sync::broadcast::Receiver<EncodedPacket>> {
        self.get_supervisor(&InstanceId::from(instance_id))
            .map(|sup| sup.video_broadcast().subscribe())
    }

    fn video_resolution(&self, instance_id: &str) -> Option<(u32, u32)> {
        self.get_supervisor(&InstanceId::from(instance_id))
            .map(|sup| sup.video_resolution())
    }

    fn video_framerate(&self, instance_id: &str) -> Option<u32> {
        self.get_supervisor(&InstanceId::from(instance_id))
            .map(|sup| sup.video_framerate())
    }

    fn request_keyframe(&self, instance_id: &str) {
        if let Some(sup) = self.get_supervisor(&InstanceId::from(instance_id)) {
            sup.request_keyframe();
        }
    }

    fn video_codec(&self, instance_id: &str) -> vscreen_core::frame::VideoCodec {
        self.get_supervisor(&InstanceId::from(instance_id))
            .map(|sup| sup.video_codec())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vscreen_core::config::{AudioConfig, VideoConfig};

    fn make_config(id: &str) -> InstanceConfig {
        InstanceConfig {
            instance_id: InstanceId::from(id),
            cdp_endpoint: "ws://localhost:9222/devtools/page/TEST".into(),
            pulse_source: "test.monitor".into(),
            display: None,
            video: VideoConfig::default(),
            audio: AudioConfig::default(),
            rtp_output: None,
        }
    }

    #[test]
    fn create_and_get() {
        let reg = InstanceRegistry::new();
        let config = make_config("test-1");
        let entry = reg.create(config, 16).expect("create");
        assert_eq!(*entry.state_rx.borrow(), InstanceState::Created);

        let fetched = reg.get(&InstanceId::from("test-1")).expect("get");
        assert_eq!(*fetched.state_rx.borrow(), InstanceState::Created);
    }

    #[test]
    fn create_duplicate_rejected() {
        let reg = InstanceRegistry::new();
        let config = make_config("test-1");
        reg.create(config, 16).expect("create 1");

        let config2 = make_config("test-1");
        let result = reg.create(config2, 16);
        assert!(matches!(result, Err(VScreenError::InstanceAlreadyExists(_))));
    }

    #[test]
    fn create_at_limit_rejected() {
        let reg = InstanceRegistry::new();
        reg.create(make_config("a"), 1).expect("create");
        let result = reg.create(make_config("b"), 1);
        assert!(matches!(result, Err(VScreenError::LimitExceeded(_))));
    }

    #[test]
    fn remove_existing() {
        let reg = InstanceRegistry::new();
        reg.create(make_config("test-1"), 16).expect("create");
        let _entry = reg.remove(&InstanceId::from("test-1")).expect("remove");
        assert!(reg.is_empty());
    }

    #[test]
    fn remove_nonexistent() {
        let reg = InstanceRegistry::new();
        let result = reg.remove(&InstanceId::from("nope"));
        assert!(matches!(result, Err(VScreenError::InstanceNotFound(_))));
    }

    #[test]
    fn list_ids() {
        let reg = InstanceRegistry::new();
        reg.create(make_config("a"), 16).expect("create a");
        reg.create(make_config("b"), 16).expect("create b");
        let ids = reg.list_ids();
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn len_and_is_empty() {
        let reg = InstanceRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);

        reg.create(make_config("a"), 16).expect("create");
        assert!(!reg.is_empty());
        assert_eq!(reg.len(), 1);
    }

    // -----------------------------------------------------------------------
    // Supervisor registry tests
    // -----------------------------------------------------------------------

    fn make_state() -> AppState {
        AppState::new(
            vscreen_core::config::AppConfig::default(),
            CancellationToken::new(),
        )
    }

    #[test]
    fn supervisor_get_missing_returns_none() {
        let state = make_state();
        assert!(state.get_supervisor(&InstanceId::from("nope")).is_none());
    }

    #[test]
    fn supervisor_set_and_get() {
        let state = make_state();
        assert!(state.get_supervisor(&InstanceId::from("test")).is_none());
        assert_eq!(state.supervisors.len(), 0);
    }

    #[tokio::test]
    async fn supervisor_remove() {
        let state = make_state();
        state.remove_supervisor(&InstanceId::from("absent")).await;
        assert_eq!(state.supervisors.len(), 0);
    }

    #[test]
    fn app_state_clone_shares_supervisors() {
        let state = make_state();
        let state2 = state.clone();
        // Both share the same DashMap
        assert!(std::sync::Arc::ptr_eq(&state.supervisors, &state2.supervisors));
    }

    #[test]
    fn app_state_clone_shares_registry() {
        let state = make_state();
        let state2 = state.clone();
        assert!(std::sync::Arc::ptr_eq(&state.registry, &state2.registry));
    }
}
