use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use dashmap::{DashMap, DashSet};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};
use vscreen_core::instance::{
    InstanceId, LockInfo, LockStatus, LockToken, LockType, SessionId, WaitQueueEntry,
};

// ---------------------------------------------------------------------------
// Lock state (per-instance, interior to DashMap)
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct InstanceLockState {
    exclusive: Option<LockInfo>,
    observers: Vec<LockInfo>,
    wait_queue: VecDeque<WaitEntry>,
}

#[derive(Debug)]
struct WaitEntry {
    session_id: SessionId,
    agent_name: Option<String>,
    requested_lock_type: LockType,
    queued_at: chrono::DateTime<Utc>,
    notify: oneshot::Sender<()>,
}

impl InstanceLockState {
    fn new() -> Self {
        Self {
            exclusive: None,
            observers: Vec::new(),
            wait_queue: VecDeque::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.exclusive.is_none() && self.observers.is_empty() && self.wait_queue.is_empty()
    }

    fn to_status(&self, instance_id: &InstanceId) -> LockStatus {
        LockStatus {
            instance_id: instance_id.clone(),
            exclusive_holder: self.exclusive.clone(),
            observers: self.observers.clone(),
            wait_queue: self
                .wait_queue
                .iter()
                .map(|w| WaitQueueEntry {
                    session_id: w.session_id.clone(),
                    agent_name: w.agent_name.clone(),
                    requested_lock_type: w.requested_lock_type,
                    queued_at: w.queued_at,
                })
                .collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// Public error type (lightweight, maps to MCP errors upstream)
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum LockError {
    InstanceLocked {
        holder_session: SessionId,
        holder_agent: Option<String>,
        expires_at: chrono::DateTime<Utc>,
    },
    NotHeld {
        instance_id: InstanceId,
    },
    Timeout {
        instance_id: InstanceId,
    },
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InstanceLocked {
                holder_session,
                holder_agent,
                expires_at,
            } => {
                let remaining = (*expires_at - Utc::now()).num_seconds().max(0);
                write!(
                    f,
                    "locked by session '{holder_session}'{}; expires in {remaining}s (at {expires_at})",
                    holder_agent
                        .as_deref()
                        .map(|n| format!(" (agent: '{n}')"))
                        .unwrap_or_default(),
                )
            }
            Self::NotHeld { instance_id } => {
                write!(f, "no lock held on instance '{instance_id}'")
            }
            Self::Timeout { instance_id } => {
                write!(f, "timed out waiting for lock on instance '{instance_id}'")
            }
        }
    }
}

// ---------------------------------------------------------------------------
// InstanceLockManager
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct InstanceLockManager {
    locks: DashMap<InstanceId, InstanceLockState>,
    active_sessions: DashSet<SessionId>,
}

impl InstanceLockManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            locks: DashMap::new(),
            active_sessions: DashSet::new(),
        }
    }

    /// Register a session as active (called when a new MCP session is created).
    pub fn register_session(&self, session_id: &SessionId) {
        self.active_sessions.insert(session_id.clone());
        debug!(session_id = %session_id, "session registered");
    }

    /// Unregister a session (called when session is dropped). Releases all locks.
    pub fn unregister_session(&self, session_id: &SessionId) {
        self.active_sessions.remove(session_id);
        self.release_all_for_session(session_id);
        info!(session_id = %session_id, "session unregistered, locks released");
    }

    /// Check whether a session is currently active.
    #[must_use]
    pub fn is_session_active(&self, session_id: &SessionId) -> bool {
        self.active_sessions.contains(session_id)
    }

    /// Attempt to acquire a lock immediately. Returns `Err` if blocked.
    pub fn acquire(
        &self,
        instance_id: &InstanceId,
        session_id: &SessionId,
        agent_name: Option<String>,
        lock_type: LockType,
        ttl: Duration,
    ) -> Result<LockInfo, LockError> {
        self.acquire_inner(instance_id, session_id, agent_name, lock_type, ttl, false)
    }

    /// Acquire a lock tagged as auto-acquired (can be stolen by new sessions).
    pub fn acquire_auto(
        &self,
        instance_id: &InstanceId,
        session_id: &SessionId,
        lock_type: LockType,
        ttl: Duration,
    ) -> Result<LockInfo, LockError> {
        self.acquire_inner(instance_id, session_id, None, lock_type, ttl, true)
    }

    fn acquire_inner(
        &self,
        instance_id: &InstanceId,
        session_id: &SessionId,
        agent_name: Option<String>,
        lock_type: LockType,
        ttl: Duration,
        auto_acquired: bool,
    ) -> Result<LockInfo, LockError> {
        let mut state = self
            .locks
            .entry(instance_id.clone())
            .or_insert_with(InstanceLockState::new);

        // If this session already holds the requested lock type, renew it.
        if let Some(ref mut exc) = state.exclusive {
            if exc.session_id == *session_id {
                exc.expires_at = Utc::now() + chrono::Duration::from_std(ttl).unwrap_or(chrono::Duration::seconds(120));
                if agent_name.is_some() {
                    exc.agent_name = agent_name;
                }
                return Ok(exc.clone());
            }
        }
        if lock_type == LockType::Observer {
            if let Some(existing) = state.observers.iter_mut().find(|o| o.session_id == *session_id) {
                existing.expires_at = Utc::now() + chrono::Duration::from_std(ttl).unwrap_or(chrono::Duration::seconds(120));
                if agent_name.is_some() {
                    existing.agent_name = agent_name;
                }
                return Ok(existing.clone());
            }
        }

        match lock_type {
            LockType::Exclusive => {
                if state.exclusive.is_some() || !state.observers.is_empty() {
                    // Allow same-session upgrade: if this session is the only
                    // observer and there's no exclusive holder, remove the
                    // observer lock and grant exclusive instead.
                    let is_self_upgrade = state.exclusive.is_none()
                        && state.observers.len() == 1
                        && state.observers[0].session_id == *session_id;

                    if is_self_upgrade {
                        debug!(
                            instance_id = %instance_id,
                            session = %session_id,
                            "upgrading observer lock to exclusive"
                        );
                        state.observers.clear();
                    } else {
                        let blocker = state
                            .exclusive
                            .as_ref()
                            .or(state.observers.first())
                            .unwrap();
                        return Err(LockError::InstanceLocked {
                            holder_session: blocker.session_id.clone(),
                            holder_agent: blocker.agent_name.clone(),
                            expires_at: blocker.expires_at,
                        });
                    }
                }
                let info = LockInfo {
                    session_id: session_id.clone(),
                    agent_name,
                    lock_type,
                    lock_token: LockToken::new(),
                    acquired_at: Utc::now(),
                    expires_at: Utc::now() + chrono::Duration::from_std(ttl).unwrap_or(chrono::Duration::seconds(120)),
                    auto_acquired,
                };
                state.exclusive = Some(info.clone());
                info!(instance_id = %instance_id, session = %session_id, token = %info.lock_token, auto = auto_acquired, "exclusive lock acquired");
                Ok(info)
            }
            LockType::Observer => {
                if state.exclusive.is_some() {
                    let exc = state.exclusive.as_ref().unwrap();
                    return Err(LockError::InstanceLocked {
                        holder_session: exc.session_id.clone(),
                        holder_agent: exc.agent_name.clone(),
                        expires_at: exc.expires_at,
                    });
                }
                let info = LockInfo {
                    session_id: session_id.clone(),
                    agent_name,
                    lock_type,
                    lock_token: LockToken::new(),
                    acquired_at: Utc::now(),
                    expires_at: Utc::now() + chrono::Duration::from_std(ttl).unwrap_or(chrono::Duration::seconds(120)),
                    auto_acquired,
                };
                state.observers.push(info.clone());
                debug!(instance_id = %instance_id, session = %session_id, "observer lock acquired");
                Ok(info)
            }
        }
    }

    /// Acquire a lock, or wait up to `wait_timeout` for it to become available.
    pub async fn acquire_or_wait(
        &self,
        instance_id: &InstanceId,
        session_id: &SessionId,
        agent_name: Option<String>,
        lock_type: LockType,
        ttl: Duration,
        wait_timeout: Duration,
    ) -> Result<LockInfo, LockError> {
        // Try immediate acquire first.
        match self.acquire(instance_id, session_id, agent_name.clone(), lock_type, ttl) {
            Ok(info) => return Ok(info),
            Err(LockError::InstanceLocked { .. }) => {}
            Err(e) => return Err(e),
        }

        // Enqueue a waiter.
        let (tx, rx) = oneshot::channel();
        {
            let mut state = self
                .locks
                .entry(instance_id.clone())
                .or_insert_with(InstanceLockState::new);
            state.wait_queue.push_back(WaitEntry {
                session_id: session_id.clone(),
                agent_name: agent_name.clone(),
                requested_lock_type: lock_type,
                queued_at: Utc::now(),
                notify: tx,
            });
        }

        // Wait for notification or timeout.
        match tokio::time::timeout(wait_timeout, rx).await {
            Ok(Ok(())) => {
                // We were notified -- try to acquire again.
                self.acquire(instance_id, session_id, agent_name, lock_type, ttl)
            }
            _ => {
                // Timeout or sender dropped -- remove from queue.
                self.remove_from_queue(instance_id, session_id);
                Err(LockError::Timeout {
                    instance_id: instance_id.clone(),
                })
            }
        }
    }

    /// Release all locks held by a session (or matching a lock token) on an instance.
    /// Promotes the next waiter if any.
    pub fn release(
        &self,
        instance_id: &InstanceId,
        session_id: &SessionId,
    ) -> Result<bool, LockError> {
        self.release_with_token(instance_id, session_id, None)
    }

    /// Release locks matching session_id or lock_token.
    pub fn release_with_token(
        &self,
        instance_id: &InstanceId,
        session_id: &SessionId,
        lock_token: Option<&LockToken>,
    ) -> Result<bool, LockError> {
        let mut entry = self
            .locks
            .entry(instance_id.clone())
            .or_insert_with(InstanceLockState::new);

        let matches = |info: &LockInfo| -> bool {
            info.session_id == *session_id
                || lock_token.map_or(false, |t| info.lock_token == *t)
        };

        let mut released = false;
        if let Some(ref exc) = entry.exclusive {
            if matches(exc) {
                let old_session = exc.session_id.clone();
                entry.exclusive = None;
                released = true;
                info!(instance_id = %instance_id, session = %old_session, "exclusive lock released");
            }
        }
        let before = entry.observers.len();
        entry.observers.retain(|o| !matches(o));
        if entry.observers.len() < before {
            released = true;
            debug!(instance_id = %instance_id, session = %session_id, "observer lock released");
        }

        if !released {
            return Err(LockError::NotHeld {
                instance_id: instance_id.clone(),
            });
        }

        let promoted = self.try_promote(&mut entry);

        if entry.is_empty() {
            drop(entry);
            self.locks.remove(instance_id);
        }

        Ok(promoted)
    }

    /// Renew (extend TTL) for a lock held by this session.
    pub fn renew(
        &self,
        instance_id: &InstanceId,
        session_id: &SessionId,
        ttl: Duration,
    ) -> Result<LockInfo, LockError> {
        self.renew_with_token(instance_id, session_id, None, ttl)
    }

    /// Renew by session ID or lock token.
    pub fn renew_with_token(
        &self,
        instance_id: &InstanceId,
        session_id: &SessionId,
        lock_token: Option<&LockToken>,
        ttl: Duration,
    ) -> Result<LockInfo, LockError> {
        let mut state = self.locks.get_mut(instance_id).ok_or(LockError::NotHeld {
            instance_id: instance_id.clone(),
        })?;

        let new_expiry = Utc::now() + chrono::Duration::from_std(ttl).unwrap_or(chrono::Duration::seconds(120));

        let matches = |info: &LockInfo| -> bool {
            info.session_id == *session_id
                || lock_token.map_or(false, |t| info.lock_token == *t)
        };

        if let Some(ref mut exc) = state.exclusive {
            if matches(exc) {
                exc.expires_at = new_expiry;
                return Ok(exc.clone());
            }
        }
        if let Some(obs) = state.observers.iter_mut().find(|o| matches(o)) {
            obs.expires_at = new_expiry;
            return Ok(obs.clone());
        }

        Err(LockError::NotHeld {
            instance_id: instance_id.clone(),
        })
    }

    /// Check whether `session_id` has adequate access for the requested `required` lock type.
    pub fn check_access(
        &self,
        instance_id: &InstanceId,
        session_id: &SessionId,
        required: LockType,
    ) -> Result<(), LockError> {
        self.check_access_with_token(instance_id, session_id, None, required)
    }

    /// Check access by session ID or lock token. Access is granted if either matches.
    pub fn check_access_with_token(
        &self,
        instance_id: &InstanceId,
        session_id: &SessionId,
        lock_token: Option<&LockToken>,
        required: LockType,
    ) -> Result<(), LockError> {
        let state = match self.locks.get(instance_id) {
            Some(s) => s,
            None => {
                return Err(LockError::NotHeld {
                    instance_id: instance_id.clone(),
                });
            }
        };

        let matches_session_or_token = |info: &LockInfo| -> bool {
            info.session_id == *session_id
                || lock_token.map_or(false, |t| info.lock_token == *t)
        };

        match required {
            LockType::Exclusive => {
                if let Some(ref exc) = state.exclusive {
                    if matches_session_or_token(exc) {
                        return Ok(());
                    }
                    return Err(LockError::InstanceLocked {
                        holder_session: exc.session_id.clone(),
                        holder_agent: exc.agent_name.clone(),
                        expires_at: exc.expires_at,
                    });
                }
                Err(LockError::NotHeld {
                    instance_id: instance_id.clone(),
                })
            }
            LockType::Observer => {
                if let Some(ref exc) = state.exclusive {
                    if matches_session_or_token(exc) {
                        return Ok(());
                    }
                    return Err(LockError::InstanceLocked {
                        holder_session: exc.session_id.clone(),
                        holder_agent: exc.agent_name.clone(),
                        expires_at: exc.expires_at,
                    });
                }
                if state.observers.iter().any(|o| matches_session_or_token(o)) {
                    return Ok(());
                }
                Err(LockError::NotHeld {
                    instance_id: instance_id.clone(),
                })
            }
        }
    }

    /// Check if a specific session holds any lock on an instance.
    #[must_use]
    pub fn is_held_by(&self, instance_id: &InstanceId, session_id: &SessionId) -> bool {
        self.locks.get(instance_id).map_or(false, |state| {
            state.exclusive.as_ref().map_or(false, |e| e.session_id == *session_id)
                || state.observers.iter().any(|o| o.session_id == *session_id)
        })
    }

    /// Check if the lock blocking access was auto-acquired.
    /// Checks the exclusive holder first, then falls back to observers.
    #[must_use]
    pub fn is_auto_acquired(&self, instance_id: &InstanceId) -> bool {
        self.locks.get(instance_id).map_or(false, |state| {
            if let Some(ref exc) = state.exclusive {
                return exc.auto_acquired;
            }
            state.observers.iter().all(|o| o.auto_acquired) && !state.observers.is_empty()
        })
    }

    /// Get lock status for a single instance.
    #[must_use]
    pub fn status(&self, instance_id: &InstanceId) -> LockStatus {
        match self.locks.get(instance_id) {
            Some(state) => state.to_status(instance_id),
            None => LockStatus {
                instance_id: instance_id.clone(),
                exclusive_holder: None,
                observers: vec![],
                wait_queue: vec![],
            },
        }
    }

    /// Get lock status for all instances that have any lock state.
    #[must_use]
    pub fn status_all(&self) -> Vec<LockStatus> {
        self.locks
            .iter()
            .map(|entry| entry.value().to_status(entry.key()))
            .collect()
    }

    /// Release all locks held by a session across all instances (cleanup on disconnect).
    pub fn release_all_for_session(&self, session_id: &SessionId) {
        let instance_ids: Vec<InstanceId> = self.locks.iter().map(|e| e.key().clone()).collect();
        for id in &instance_ids {
            let _ = self.release(id, session_id);
        }
        // Also purge any queue entries for this session.
        for mut entry in self.locks.iter_mut() {
            entry.value_mut().wait_queue.retain(|w| w.session_id != *session_id);
        }
    }

    /// Expire stale locks (TTL exceeded). Called periodically by the reaper task.
    pub fn reap_expired(&self) {
        let now = Utc::now();
        let instance_ids: Vec<InstanceId> = self.locks.iter().map(|e| e.key().clone()).collect();

        for id in &instance_ids {
            let mut needs_cleanup = false;
            if let Some(mut state) = self.locks.get_mut(id) {
                if let Some(ref exc) = state.exclusive {
                    if exc.expires_at <= now {
                        info!(
                            instance_id = %id,
                            session = %exc.session_id,
                            "exclusive lock expired (TTL)"
                        );
                        state.exclusive = None;
                        self.try_promote(&mut state);
                    }
                }

                let before = state.observers.len();
                state.observers.retain(|o| {
                    if o.expires_at <= now {
                        debug!(
                            instance_id = %id,
                            session = %o.session_id,
                            "observer lock expired (TTL)"
                        );
                        false
                    } else {
                        true
                    }
                });
                if state.observers.len() < before {
                    self.try_promote(&mut state);
                }

                needs_cleanup = state.is_empty();
            }

            if needs_cleanup {
                self.locks.remove(id);
            }
        }
    }

    /// Spawn the background reaper task that periodically expires stale locks.
    pub fn spawn_reaper(self: &Arc<Self>, cancel: CancellationToken) {
        let mgr = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => {
                        debug!("lock reaper shutting down");
                        break;
                    }
                    _ = interval.tick() => {
                        mgr.reap_expired();
                    }
                }
            }
        });
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn remove_from_queue(&self, instance_id: &InstanceId, session_id: &SessionId) {
        if let Some(mut state) = self.locks.get_mut(instance_id) {
            state.wait_queue.retain(|w| w.session_id != *session_id);
        }
    }

    /// Try to promote the next waiter(s) from the queue. Returns true if any were promoted.
    fn try_promote(&self, state: &mut InstanceLockState) -> bool {
        if state.exclusive.is_some() {
            return false;
        }

        let mut promoted = false;
        while let Some(front) = state.wait_queue.front() {
            match front.requested_lock_type {
                LockType::Exclusive => {
                    if !state.observers.is_empty() {
                        break;
                    }
                    let waiter = state.wait_queue.pop_front().unwrap();
                    let _ = waiter.notify.send(());
                    promoted = true;
                    break; // Only one exclusive at a time.
                }
                LockType::Observer => {
                    let waiter = state.wait_queue.pop_front().unwrap();
                    let _ = waiter.notify.send(());
                    promoted = true;
                    // Continue promoting observers until we hit an exclusive request.
                }
            }
        }
        promoted
    }
}

impl Default for InstanceLockManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sid() -> SessionId {
        SessionId::new()
    }

    fn iid(name: &str) -> InstanceId {
        InstanceId::from(name)
    }

    #[test]
    fn acquire_exclusive_succeeds_when_unlocked() {
        let mgr = InstanceLockManager::new();
        let s = sid();
        let result = mgr.acquire(&iid("dev"), &s, None, LockType::Exclusive, Duration::from_secs(60));
        assert!(result.is_ok());
        let info = result.unwrap();
        assert_eq!(info.lock_type, LockType::Exclusive);
        assert_eq!(info.session_id, s);
    }

    #[test]
    fn acquire_exclusive_blocks_second_exclusive() {
        let mgr = InstanceLockManager::new();
        let s1 = sid();
        let s2 = sid();
        mgr.acquire(&iid("dev"), &s1, Some("agent-1".into()), LockType::Exclusive, Duration::from_secs(60)).unwrap();
        let result = mgr.acquire(&iid("dev"), &s2, None, LockType::Exclusive, Duration::from_secs(60));
        assert!(matches!(result, Err(LockError::InstanceLocked { .. })));
    }

    #[test]
    fn acquire_observer_coexists_with_observer() {
        let mgr = InstanceLockManager::new();
        let s1 = sid();
        let s2 = sid();
        mgr.acquire(&iid("dev"), &s1, None, LockType::Observer, Duration::from_secs(60)).unwrap();
        let result = mgr.acquire(&iid("dev"), &s2, None, LockType::Observer, Duration::from_secs(60));
        assert!(result.is_ok());
    }

    #[test]
    fn exclusive_blocks_observer() {
        let mgr = InstanceLockManager::new();
        let s1 = sid();
        let s2 = sid();
        mgr.acquire(&iid("dev"), &s1, None, LockType::Exclusive, Duration::from_secs(60)).unwrap();
        let result = mgr.acquire(&iid("dev"), &s2, None, LockType::Observer, Duration::from_secs(60));
        assert!(matches!(result, Err(LockError::InstanceLocked { .. })));
    }

    #[test]
    fn observer_blocks_exclusive() {
        let mgr = InstanceLockManager::new();
        let s1 = sid();
        let s2 = sid();
        mgr.acquire(&iid("dev"), &s1, None, LockType::Observer, Duration::from_secs(60)).unwrap();
        let result = mgr.acquire(&iid("dev"), &s2, None, LockType::Exclusive, Duration::from_secs(60));
        assert!(matches!(result, Err(LockError::InstanceLocked { .. })));
    }

    #[test]
    fn release_and_reacquire() {
        let mgr = InstanceLockManager::new();
        let s1 = sid();
        let s2 = sid();
        mgr.acquire(&iid("dev"), &s1, None, LockType::Exclusive, Duration::from_secs(60)).unwrap();
        mgr.release(&iid("dev"), &s1).unwrap();
        let result = mgr.acquire(&iid("dev"), &s2, None, LockType::Exclusive, Duration::from_secs(60));
        assert!(result.is_ok());
    }

    #[test]
    fn release_not_held_returns_error() {
        let mgr = InstanceLockManager::new();
        let s = sid();
        let result = mgr.release(&iid("dev"), &s);
        assert!(matches!(result, Err(LockError::NotHeld { .. })));
    }

    #[test]
    fn renew_extends_ttl() {
        let mgr = InstanceLockManager::new();
        let s = sid();
        let info1 = mgr.acquire(&iid("dev"), &s, None, LockType::Exclusive, Duration::from_secs(30)).unwrap();
        let info2 = mgr.renew(&iid("dev"), &s, Duration::from_secs(300)).unwrap();
        assert!(info2.expires_at > info1.expires_at);
    }

    #[test]
    fn renew_not_held_returns_error() {
        let mgr = InstanceLockManager::new();
        let s = sid();
        let result = mgr.renew(&iid("dev"), &s, Duration::from_secs(60));
        assert!(matches!(result, Err(LockError::NotHeld { .. })));
    }

    #[test]
    fn check_access_exclusive_holder_passes() {
        let mgr = InstanceLockManager::new();
        let s = sid();
        mgr.acquire(&iid("dev"), &s, None, LockType::Exclusive, Duration::from_secs(60)).unwrap();
        assert!(mgr.check_access(&iid("dev"), &s, LockType::Exclusive).is_ok());
        assert!(mgr.check_access(&iid("dev"), &s, LockType::Observer).is_ok());
    }

    #[test]
    fn check_access_observer_cannot_do_exclusive() {
        let mgr = InstanceLockManager::new();
        let s = sid();
        mgr.acquire(&iid("dev"), &s, None, LockType::Observer, Duration::from_secs(60)).unwrap();
        assert!(mgr.check_access(&iid("dev"), &s, LockType::Observer).is_ok());
        assert!(matches!(
            mgr.check_access(&iid("dev"), &s, LockType::Exclusive),
            Err(LockError::NotHeld { .. })
        ));
    }

    #[test]
    fn check_access_different_session_blocked() {
        let mgr = InstanceLockManager::new();
        let s1 = sid();
        let s2 = sid();
        mgr.acquire(&iid("dev"), &s1, None, LockType::Exclusive, Duration::from_secs(60)).unwrap();
        assert!(matches!(
            mgr.check_access(&iid("dev"), &s2, LockType::Exclusive),
            Err(LockError::InstanceLocked { .. })
        ));
    }

    #[test]
    fn status_returns_correct_info() {
        let mgr = InstanceLockManager::new();
        let s = sid();
        mgr.acquire(&iid("dev"), &s, Some("my-agent".into()), LockType::Exclusive, Duration::from_secs(60)).unwrap();
        let status = mgr.status(&iid("dev"));
        assert!(status.exclusive_holder.is_some());
        assert_eq!(status.exclusive_holder.as_ref().unwrap().agent_name.as_deref(), Some("my-agent"));
    }

    #[test]
    fn status_unlocked_instance() {
        let mgr = InstanceLockManager::new();
        let status = mgr.status(&iid("dev"));
        assert!(status.exclusive_holder.is_none());
        assert!(status.observers.is_empty());
    }

    #[test]
    fn release_all_for_session_cleans_up() {
        let mgr = InstanceLockManager::new();
        let s = sid();
        mgr.acquire(&iid("a"), &s, None, LockType::Exclusive, Duration::from_secs(60)).unwrap();
        mgr.acquire(&iid("b"), &s, None, LockType::Observer, Duration::from_secs(60)).unwrap();
        mgr.release_all_for_session(&s);
        assert!(mgr.status(&iid("a")).exclusive_holder.is_none());
        assert!(mgr.status(&iid("b")).observers.is_empty());
    }

    #[test]
    fn reap_expired_removes_stale_locks() {
        let mgr = InstanceLockManager::new();
        let s = sid();
        // Acquire with zero TTL (already expired).
        mgr.acquire(&iid("dev"), &s, None, LockType::Exclusive, Duration::ZERO).unwrap();
        // Give chrono a moment to expire.
        std::thread::sleep(Duration::from_millis(10));
        mgr.reap_expired();
        assert!(mgr.status(&iid("dev")).exclusive_holder.is_none());
    }

    #[test]
    fn reacquire_same_session_renews() {
        let mgr = InstanceLockManager::new();
        let s = sid();
        let info1 = mgr.acquire(&iid("dev"), &s, None, LockType::Exclusive, Duration::from_secs(30)).unwrap();
        let info2 = mgr.acquire(&iid("dev"), &s, None, LockType::Exclusive, Duration::from_secs(120)).unwrap();
        assert!(info2.expires_at >= info1.expires_at);
    }

    #[test]
    fn different_instances_independent() {
        let mgr = InstanceLockManager::new();
        let s1 = sid();
        let s2 = sid();
        mgr.acquire(&iid("a"), &s1, None, LockType::Exclusive, Duration::from_secs(60)).unwrap();
        let result = mgr.acquire(&iid("b"), &s2, None, LockType::Exclusive, Duration::from_secs(60));
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn acquire_or_wait_timeout() {
        let mgr = Arc::new(InstanceLockManager::new());
        let s1 = sid();
        let s2 = sid();
        mgr.acquire(&iid("dev"), &s1, None, LockType::Exclusive, Duration::from_secs(60)).unwrap();

        let result = mgr
            .acquire_or_wait(
                &iid("dev"),
                &s2,
                None,
                LockType::Exclusive,
                Duration::from_secs(60),
                Duration::from_millis(100),
            )
            .await;
        assert!(matches!(result, Err(LockError::Timeout { .. })));
    }

    #[tokio::test]
    async fn acquire_or_wait_succeeds_after_release() {
        let mgr = Arc::new(InstanceLockManager::new());
        let s1 = sid();
        let s2 = sid();
        let id = iid("dev");
        mgr.acquire(&id, &s1, None, LockType::Exclusive, Duration::from_secs(60)).unwrap();

        let mgr2 = Arc::clone(&mgr);
        let id2 = id.clone();
        let s1c = s1.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            mgr2.release(&id2, &s1c).unwrap();
        });

        let result = mgr
            .acquire_or_wait(
                &id,
                &s2,
                None,
                LockType::Exclusive,
                Duration::from_secs(60),
                Duration::from_secs(2),
            )
            .await;
        assert!(result.is_ok());
    }

    #[test]
    fn lock_error_display() {
        let err = LockError::InstanceLocked {
            holder_session: sid(),
            holder_agent: Some("test-agent".into()),
            expires_at: Utc::now() + chrono::Duration::seconds(45),
        };
        let msg = err.to_string();
        assert!(msg.contains("test-agent"));
        assert!(msg.contains("45s") || msg.contains("44s"));
    }

    #[test]
    fn status_all_returns_all_locked() {
        let mgr = InstanceLockManager::new();
        let s1 = sid();
        let s2 = sid();
        mgr.acquire(&iid("a"), &s1, None, LockType::Exclusive, Duration::from_secs(60)).unwrap();
        mgr.acquire(&iid("b"), &s2, None, LockType::Observer, Duration::from_secs(60)).unwrap();
        let all = mgr.status_all();
        assert_eq!(all.len(), 2);
    }

    // -----------------------------------------------------------------------
    // Lock token tests
    // -----------------------------------------------------------------------

    #[test]
    fn acquire_returns_lock_token() {
        let mgr = InstanceLockManager::new();
        let s = sid();
        let info = mgr.acquire(&iid("dev"), &s, None, LockType::Exclusive, Duration::from_secs(60)).unwrap();
        assert!(!info.lock_token.0.is_nil());
    }

    #[test]
    fn check_access_with_token_works() {
        let mgr = InstanceLockManager::new();
        let s1 = sid();
        let s2 = sid();
        let info = mgr.acquire(&iid("dev"), &s1, None, LockType::Exclusive, Duration::from_secs(60)).unwrap();
        // s2 doesn't hold the lock, but has the token
        assert!(mgr.check_access_with_token(&iid("dev"), &s2, Some(&info.lock_token), LockType::Exclusive).is_ok());
        // Without the token, s2 is blocked
        assert!(mgr.check_access_with_token(&iid("dev"), &s2, None, LockType::Exclusive).is_err());
    }

    #[test]
    fn release_with_token_works() {
        let mgr = InstanceLockManager::new();
        let s1 = sid();
        let s2 = sid();
        let info = mgr.acquire(&iid("dev"), &s1, None, LockType::Exclusive, Duration::from_secs(60)).unwrap();
        // s2 can release using the token
        assert!(mgr.release_with_token(&iid("dev"), &s2, Some(&info.lock_token)).is_ok());
        assert!(mgr.status(&iid("dev")).exclusive_holder.is_none());
    }

    #[test]
    fn renew_with_token_works() {
        let mgr = InstanceLockManager::new();
        let s1 = sid();
        let s2 = sid();
        let info1 = mgr.acquire(&iid("dev"), &s1, None, LockType::Exclusive, Duration::from_secs(30)).unwrap();
        // s2 can renew using the token
        let info2 = mgr.renew_with_token(&iid("dev"), &s2, Some(&info1.lock_token), Duration::from_secs(300)).unwrap();
        assert!(info2.expires_at > info1.expires_at);
    }

    // -----------------------------------------------------------------------
    // Session registry tests
    // -----------------------------------------------------------------------

    #[test]
    fn register_and_unregister_session() {
        let mgr = InstanceLockManager::new();
        let s = sid();
        mgr.register_session(&s);
        assert!(mgr.is_session_active(&s));
        mgr.unregister_session(&s);
        assert!(!mgr.is_session_active(&s));
    }

    #[test]
    fn unregister_session_releases_locks() {
        let mgr = InstanceLockManager::new();
        let s = sid();
        mgr.register_session(&s);
        mgr.acquire(&iid("dev"), &s, None, LockType::Exclusive, Duration::from_secs(60)).unwrap();
        assert!(mgr.status(&iid("dev")).exclusive_holder.is_some());
        mgr.unregister_session(&s);
        assert!(mgr.status(&iid("dev")).exclusive_holder.is_none());
    }

    #[test]
    fn is_held_by_works() {
        let mgr = InstanceLockManager::new();
        let s1 = sid();
        let s2 = sid();
        mgr.acquire(&iid("dev"), &s1, None, LockType::Exclusive, Duration::from_secs(60)).unwrap();
        assert!(mgr.is_held_by(&iid("dev"), &s1));
        assert!(!mgr.is_held_by(&iid("dev"), &s2));
    }

    #[test]
    fn same_session_upgrades_observer_to_exclusive() {
        let mgr = InstanceLockManager::new();
        let s1 = sid();
        mgr.acquire(&iid("dev"), &s1, None, LockType::Observer, Duration::from_secs(60)).unwrap();
        assert_eq!(mgr.status(&iid("dev")).observers.len(), 1);
        assert!(mgr.status(&iid("dev")).exclusive_holder.is_none());

        let result = mgr.acquire(&iid("dev"), &s1, None, LockType::Exclusive, Duration::from_secs(60));
        assert!(result.is_ok(), "same session should upgrade observer to exclusive");
        assert!(mgr.status(&iid("dev")).exclusive_holder.is_some());
        assert!(mgr.status(&iid("dev")).observers.is_empty(), "observer should be removed after upgrade");
    }

    #[test]
    fn different_session_observer_still_blocks_exclusive() {
        let mgr = InstanceLockManager::new();
        let s1 = sid();
        let s2 = sid();
        mgr.acquire(&iid("dev"), &s1, None, LockType::Observer, Duration::from_secs(60)).unwrap();
        let result = mgr.acquire(&iid("dev"), &s2, None, LockType::Exclusive, Duration::from_secs(60));
        assert!(matches!(result, Err(LockError::InstanceLocked { .. })),
            "different session's observer should still block exclusive");
    }

    #[test]
    fn is_auto_acquired_checks_observers() {
        let mgr = InstanceLockManager::new();
        let s1 = sid();
        mgr.acquire_auto(&iid("dev"), &s1, LockType::Observer, Duration::from_secs(60)).unwrap();
        assert!(mgr.is_auto_acquired(&iid("dev")),
            "auto-acquired observer should be detected");
    }
}
