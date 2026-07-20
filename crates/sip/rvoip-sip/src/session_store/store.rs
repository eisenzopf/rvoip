use crate::cleanup_diag::{self, CleanupStage};
use crate::session_lifecycle::{SessionLeaseAuthority, SessionOperationKind, TeardownOutcome};
use crate::session_registry::{SessionRegistry, SessionRegistryHandle};
use crate::state_table::{CallId, DialogId, MediaSessionId, Role, SessionId};
use crate::types::CallState;
use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard as StdMutexGuard};
use std::time::Duration;
use tracing::{debug, info};

use super::state::{SessionHangupControl, SessionState, SessionStateCell, SessionStateSnapshot};

const SESSION_TEARDOWN_TIMEOUT: Duration = Duration::from_secs(10);
/// Maximum eager reserve for each active-session lookup index.
///
/// The authority owns the complete logical admission limit. Reserving that
/// limit independently in four sharded maps multiplies idle memory and leaves
/// allocator high water behind after churn; the maps grow normally from this
/// warm working set.
const MAX_EAGER_SESSION_STORE_INDEX_CAPACITY: usize = 4_096;

#[cfg(test)]
type IndexDeltaCommitHook = Arc<dyn Fn() + Send + Sync>;

fn same_optional_arc<T>(left: &Option<Arc<T>>, right: &Option<Arc<T>>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => Arc::ptr_eq(left, right),
        (None, None) => true,
        _ => false,
    }
}

/// Exact secondary-index changes for one session revision.
///
/// Keeping a per-index delta is both a correctness and hot-path property:
/// changing a media identifier must never transiently withdraw an unchanged
/// dialog or Call-ID route. The store's mutation boundary reserves the new
/// values, this receipt commits only those values, and the same receipt can
/// compensate the exact changes if lifecycle commit loses a teardown race.
#[derive(Debug)]
struct SessionIndexDelta {
    dialog: Option<(Option<DialogId>, Option<DialogId>)>,
    media: Option<(Option<MediaSessionId>, Option<MediaSessionId>)>,
    call: Option<(Option<CallId>, Option<CallId>)>,
}

impl SessionIndexDelta {
    fn between(old: &SessionState, new: &SessionState) -> Self {
        Self {
            dialog: (old.dialog_id != new.dialog_id).then(|| (old.dialog_id, new.dialog_id)),
            media: (old.media_session_id != new.media_session_id)
                .then(|| (old.media_session_id.clone(), new.media_session_id.clone())),
            call: (old.call_id != new.call_id).then(|| (old.call_id.clone(), new.call_id.clone())),
        }
    }

    fn is_empty(&self) -> bool {
        self.dialog.is_none() && self.media.is_none() && self.call.is_none()
    }
}

/// Compatibility name retained while media/event call sites migrate. The
/// token is now the registry-owned exact authority/slot handle rather than a
/// store-local generation.
pub(crate) type SessionLifecycleToken = SessionRegistryHandle;

/// Flexible session storage for rvoip-sip.
///
/// Identity, admission, anti-reuse, and quiesce belong exclusively to the
/// injected `SessionLeaseAuthority`. This store retains the exact
/// `SessionRegistryHandle` in every state and derived index; it never
/// allocates a second generation or tombstone.
pub struct SessionStore {
    pub(crate) sessions: Arc<DashMap<SessionId, Arc<SessionStateCell>>>,
    pub(crate) by_dialog: Arc<DashMap<DialogId, SessionRegistryHandle>>,
    pub(crate) by_call_id: Arc<DashMap<CallId, SessionRegistryHandle>>,
    pub(crate) by_media_id: Arc<DashMap<MediaSessionId, SessionRegistryHandle>>,
    authority: Arc<SessionLeaseAuthority>,
    registry: Arc<SessionRegistry>,
    created_total: AtomicU64,
    removed_total: AtomicU64,
    remove_missing_total: AtomicU64,
    update_missing_total: AtomicU64,
    /// Short structural boundary for admission/removal and cross-session
    /// secondary-index ownership. Ordinary reads and non-indexed updates use
    /// only the exact session cell and never take this lock.
    mutations: StdMutex<()>,
    #[cfg(test)]
    after_index_delta_commit: StdMutex<Option<IndexDeltaCommitHook>>,
}

impl SessionStore {
    pub fn new() -> Self {
        Self::standalone(None)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self::standalone(Some(capacity))
    }

    fn standalone(capacity: Option<usize>) -> Self {
        let authority = capacity.map_or_else(SessionLeaseAuthority::new, |capacity| {
            SessionLeaseAuthority::with_capacity(capacity.max(1))
        });
        let registry = Arc::new(SessionRegistry::with_authority(Arc::clone(&authority)));
        Self::with_lifecycle(authority, registry, capacity)
    }

    pub(crate) fn with_lifecycle(
        authority: Arc<SessionLeaseAuthority>,
        registry: Arc<SessionRegistry>,
        capacity: Option<usize>,
    ) -> Self {
        assert!(
            Arc::ptr_eq(&authority, registry.authority()),
            "session store and registry must share one lifecycle authority"
        );
        let initial_index_capacity = capacity
            .unwrap_or(0)
            .min(MAX_EAGER_SESSION_STORE_INDEX_CAPACITY);
        Self {
            sessions: Arc::new(DashMap::with_capacity(initial_index_capacity)),
            by_dialog: Arc::new(DashMap::with_capacity(initial_index_capacity)),
            by_call_id: Arc::new(DashMap::with_capacity(initial_index_capacity)),
            by_media_id: Arc::new(DashMap::with_capacity(initial_index_capacity)),
            authority,
            registry,
            created_total: AtomicU64::new(0),
            removed_total: AtomicU64::new(0),
            remove_missing_total: AtomicU64::new(0),
            update_missing_total: AtomicU64::new(0),
            mutations: StdMutex::new(()),
            #[cfg(test)]
            after_index_delta_commit: StdMutex::new(None),
        }
    }

    pub(crate) fn authority(&self) -> &Arc<SessionLeaseAuthority> {
        &self.authority
    }

    pub(crate) fn registry(&self) -> &Arc<SessionRegistry> {
        &self.registry
    }

    fn lock_mutations(&self) -> StdMutexGuard<'_, ()> {
        self.mutations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn handle_for_state(session: &SessionState) -> Option<&SessionRegistryHandle> {
        session.lifecycle_handle.as_ref()
    }

    fn state_matches_handle(session: &SessionState, handle: &SessionRegistryHandle) -> bool {
        Self::handle_for_state(session) == Some(handle)
            && session.session_id == *handle.session_id()
    }

    fn cell_for_handle(&self, handle: &SessionRegistryHandle) -> Option<Arc<SessionStateCell>> {
        let cell = self
            .sessions
            .get(handle.session_id())
            .map(|entry| Arc::clone(entry.value()))?;
        Self::state_matches_handle(cell.snapshot().state(), handle).then_some(cell)
    }

    /// Resolve one current raw identifier directly to its immutable cell
    /// revision.  This is the common closure-read path, so it deliberately
    /// avoids resolving the handle and then looking up the same map entry a
    /// second time.
    fn current_snapshot(&self, session_id: &SessionId) -> Option<Arc<SessionStateSnapshot>> {
        let cell = self
            .sessions
            .get(session_id)
            .map(|entry| Arc::clone(entry.value()))?;
        let snapshot = cell.snapshot();
        let handle = snapshot.lifecycle_handle.as_ref()?;
        (snapshot.session_id == *session_id
            && handle.session_id() == session_id
            && self.authority.is_current(handle.key()))
        .then_some(snapshot)
    }

    /// Resolve the lazy hangup-control lane for this exact current lifetime.
    /// Callers retain the returned `Arc` while awaiting the lane, so removal
    /// of the raw-ID map entry cannot redirect queued teardown to a reused ID.
    pub(crate) fn hangup_control(
        &self,
        session_id: &SessionId,
    ) -> Option<(SessionRegistryHandle, Arc<SessionHangupControl>)> {
        let cell = self
            .sessions
            .get(session_id)
            .map(|entry| Arc::clone(entry.value()))?;
        let snapshot = cell.snapshot();
        let handle = snapshot.lifecycle_handle.as_ref()?;
        (snapshot.session_id == *session_id
            && handle.session_id() == session_id
            && self.authority.is_current(handle.key()))
        .then(|| (handle.clone(), cell.hangup_control()))
    }

    /// Resolve the async state-machine lane for this exact current lifetime.
    ///
    /// Callers retain both the generation-qualified handle and lane while
    /// waiting. They must revalidate the handle after acquiring the lane so a
    /// queued event cannot cross raw-ID reuse into a replacement cell.
    pub(crate) fn state_machine_lane(
        &self,
        session_id: &SessionId,
    ) -> Option<(SessionRegistryHandle, Arc<tokio::sync::Mutex<()>>)> {
        let cell = self
            .sessions
            .get(session_id)
            .map(|entry| Arc::clone(entry.value()))?;
        let snapshot = cell.snapshot();
        let handle = snapshot.lifecycle_handle.as_ref()?;
        (snapshot.session_id == *session_id
            && handle.session_id() == session_id
            && self.authority.is_current(handle.key()))
        .then(|| (handle.clone(), cell.state_machine_lane()))
    }

    fn map_owns_cell(&self, handle: &SessionRegistryHandle, cell: &Arc<SessionStateCell>) -> bool {
        self.sessions
            .get(handle.session_id())
            .is_some_and(|entry| Arc::ptr_eq(entry.value(), cell))
    }

    fn reserve_index_delta(
        &self,
        delta: &SessionIndexDelta,
        handle: &SessionRegistryHandle,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if delta
            .dialog
            .as_ref()
            .and_then(|(_, new)| new.as_ref())
            .and_then(|id| self.by_dialog.get(id))
            .is_some_and(|owner| owner.value() != handle)
        {
            return Err("dialog identifier is already owned by another session lifetime".into());
        }
        if delta
            .media
            .as_ref()
            .and_then(|(_, new)| new.as_ref())
            .and_then(|id| self.by_media_id.get(id))
            .is_some_and(|owner| owner.value() != handle)
        {
            return Err("media identifier is already owned by another session lifetime".into());
        }
        if delta
            .call
            .as_ref()
            .and_then(|(_, new)| new.as_ref())
            .and_then(|id| self.by_call_id.get(id))
            .is_some_and(|owner| owner.value() != handle)
        {
            return Err("call identifier is already owned by another session lifetime".into());
        }
        Ok(())
    }

    fn commit_index_delta(&self, delta: &SessionIndexDelta, handle: &SessionRegistryHandle) {
        if let Some((old, new)) = &delta.dialog {
            if let Some(old) = old {
                self.by_dialog.remove_if(old, |_, owner| owner == handle);
            }
            if let Some(new) = new {
                self.by_dialog.insert(*new, handle.clone());
            }
        }
        if let Some((old, new)) = &delta.media {
            if let Some(old) = old {
                self.by_media_id.remove_if(old, |_, owner| owner == handle);
            }
            if let Some(new) = new {
                self.by_media_id.insert(new.clone(), handle.clone());
            }
        }
        if let Some((old, new)) = &delta.call {
            if let Some(old) = old {
                self.by_call_id.remove_if(old, |_, owner| owner == handle);
            }
            if let Some(new) = new {
                self.by_call_id.insert(new.clone(), handle.clone());
            }
        }
    }

    fn rollback_index_delta(&self, delta: &SessionIndexDelta, handle: &SessionRegistryHandle) {
        if let Some((old, new)) = &delta.dialog {
            if let Some(new) = new {
                self.by_dialog.remove_if(new, |_, owner| owner == handle);
            }
            if let Some(old) = old {
                self.by_dialog.insert(*old, handle.clone());
            }
        }
        if let Some((old, new)) = &delta.media {
            if let Some(new) = new {
                self.by_media_id.remove_if(new, |_, owner| owner == handle);
            }
            if let Some(old) = old {
                self.by_media_id.insert(old.clone(), handle.clone());
            }
        }
        if let Some((old, new)) = &delta.call {
            if let Some(new) = new {
                self.by_call_id.remove_if(new, |_, owner| owner == handle);
            }
            if let Some(old) = old {
                self.by_call_id.insert(old.clone(), handle.clone());
            }
        }
    }

    fn index_delta_committed(&self) {
        #[cfg(test)]
        if let Some(hook) = self
            .after_index_delta_commit
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
        {
            hook();
        }
    }

    fn remove_indexes_for_session(&self, session: &SessionState, handle: &SessionRegistryHandle) {
        if let Some(dialog_id) = &session.dialog_id {
            self.by_dialog
                .remove_if(dialog_id, |_, owner| owner == handle);
        }
        if let Some(media_id) = &session.media_session_id {
            self.by_media_id
                .remove_if(media_id, |_, owner| owner == handle);
        }
        if let Some(call_id) = &session.call_id {
            self.by_call_id
                .remove_if(call_id, |_, owner| owner == handle);
        }
    }

    async fn retire_unpublished_lifetime(&self, handle: &SessionRegistryHandle) {
        let _ = self.registry.remove_handle(handle);
        if let Ok(waiter) = self
            .authority
            .teardown(handle.key(), SESSION_TEARDOWN_TIMEOUT)
        {
            let supervisor = waiter.clone();
            let _ = waiter.wait().await;
            let _ = supervisor.wait_supervisor().await;
        }
    }

    /// Admit and publish a new exact session lifetime.
    pub async fn create_session(
        &self,
        session_id: SessionId,
        role: Role,
        with_history: bool,
    ) -> Result<SessionState, Box<dyn std::error::Error + Send + Sync>> {
        let lease = self.authority.admit(session_id.clone())?;
        let handle = match self.registry.register_handle_exact(lease.key()) {
            Ok(handle) => handle,
            Err(error) => {
                if let Ok(waiter) = self
                    .authority
                    .teardown(lease.key(), SESSION_TEARDOWN_TIMEOUT)
                {
                    let supervisor = waiter.clone();
                    let _ = waiter.wait().await;
                    let _ = supervisor.wait_supervisor().await;
                }
                return Err(Box::new(error));
            }
        };

        let mut session = if with_history {
            use crate::session_store::HistoryConfig;
            SessionState::with_history(session_id.clone(), role, HistoryConfig::default())
        } else {
            SessionState::new(session_id.clone(), role)
        };
        session.lifecycle_handle = Some(handle.clone());

        let inserted = {
            let _mutation = self.lock_mutations();
            if self.sessions.contains_key(&session_id) {
                false
            } else {
                self.sessions.insert(
                    session_id.clone(),
                    Arc::new(SessionStateCell::new(session.clone())),
                );
                true
            }
        };
        if !inserted {
            self.retire_unpublished_lifetime(&handle).await;
            return Err(format!("Session {session_id} already exists").into());
        }

        self.created_total.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "perf-infra-memory-diagnostics")]
        rvoip_infra_common::memory_diagnostics::record_created(
            "sip.session_store.session",
            std::mem::size_of::<SessionState>(),
        );
        info!(session_id = %session_id, ?role, "created exact SIP session lifetime");
        Ok(session)
    }

    pub(crate) fn lifecycle_handle(&self, session_id: &SessionId) -> Option<SessionRegistryHandle> {
        self.current_snapshot(session_id)?.lifecycle_handle.clone()
    }

    pub(crate) fn lifecycle_token(&self, session_id: &SessionId) -> Option<SessionLifecycleToken> {
        self.lifecycle_handle(session_id)
    }

    pub async fn get_session(
        &self,
        session_id: &SessionId,
    ) -> Result<SessionState, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.get_session_snapshot(session_id).await?.state().clone())
    }

    /// Load one immutable revision without retaining a DashMap shard guard.
    pub async fn get_session_snapshot(
        &self,
        session_id: &SessionId,
    ) -> Result<Arc<SessionStateSnapshot>, Box<dyn std::error::Error + Send + Sync>> {
        self.current_snapshot(session_id)
            .ok_or_else(|| format!("Session {session_id} not found").into())
    }

    pub(crate) async fn get_session_exact(
        &self,
        handle: &SessionRegistryHandle,
    ) -> Result<SessionState, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.get_session_snapshot_exact(handle)?.state().clone())
    }

    pub(crate) fn get_session_snapshot_exact(
        &self,
        handle: &SessionRegistryHandle,
    ) -> Result<Arc<SessionStateSnapshot>, Box<dyn std::error::Error + Send + Sync>> {
        if !self.authority.is_current(handle.key()) {
            return Err("session lifetime is no longer current".into());
        }
        self.get_session_retained_snapshot_exact(handle)
    }

    /// Read an exact retained state during teardown. Unlike
    /// `get_session_exact`, this deliberately accepts a quiescing or retired
    /// authority phase, but still requires the same generation and registry
    /// slot retained in the store.
    pub(crate) async fn get_session_retained_exact(
        &self,
        handle: &SessionRegistryHandle,
    ) -> Result<SessionState, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self
            .get_session_retained_snapshot_exact(handle)?
            .state()
            .clone())
    }

    pub(crate) fn get_session_retained_snapshot_exact(
        &self,
        handle: &SessionRegistryHandle,
    ) -> Result<Arc<SessionStateSnapshot>, Box<dyn std::error::Error + Send + Sync>> {
        let cell = self
            .cell_for_handle(handle)
            .ok_or_else(|| format!("Session {} not found", handle.session_id()))?;
        let snapshot = cell.snapshot();
        if !Self::state_matches_handle(snapshot.state(), handle) {
            return Err(format!("Session {} not found", handle.session_id()).into());
        }
        Ok(snapshot)
    }

    /// Evaluate a read-only closure against one stable immutable revision.
    pub fn with_session<R>(
        &self,
        session_id: &SessionId,
        read: impl FnOnce(&SessionState) -> R,
    ) -> Result<R, Box<dyn std::error::Error + Send + Sync>> {
        let snapshot = self
            .current_snapshot(session_id)
            .ok_or_else(|| format!("Session {session_id} not found"))?;
        Ok(read(snapshot.state()))
    }

    /// Clear the retained media identity only when this exact lifetime still
    /// owns the expected lower-layer dialog.
    ///
    /// Managed media cleanup may run after quiescing, when a normal state
    /// transition operation is no longer admissible. The mutation lock and
    /// generation-qualified handle make this cleanup safe without reopening
    /// the lifetime: stale release can neither edit a reused raw identifier
    /// nor remove a newer media index entry.
    pub(crate) fn clear_media_session_retained_exact(
        &self,
        handle: &SessionRegistryHandle,
        expected_media_id: &MediaSessionId,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let Some(cell) = self.cell_for_handle(handle) else {
            return Ok(false);
        };
        let _cell_update = cell.lock_update();
        let current = cell.snapshot();
        if !Self::state_matches_handle(current.state(), handle) {
            return Ok(false);
        }
        let mut session = current.state().clone();
        if session.media_session_id.as_ref() != Some(expected_media_id) {
            return Ok(false);
        }

        let _mutation = self.lock_mutations();
        if !self.map_owns_cell(handle, &cell) {
            return Ok(false);
        }
        self.by_media_id
            .remove_if(expected_media_id, |_, owner| owner == handle);
        session.media_session_id = None;
        session.media_session_ready = false;
        let _ = cell.publish(session);
        Ok(true)
    }

    /// Clear the retained dialog identity only when this exact lifetime still
    /// owns the expected lower-layer dialog.
    ///
    /// Initial-INVITE ownership publishes `dialog_id` before wire dispatch so
    /// a synchronous response cannot clone an incomplete session revision.
    /// Managed rollback may subsequently run after quiescing, when ordinary
    /// state-transition admission is closed. This retained exact mutation
    /// compensates that pre-wire publication without touching a replacement
    /// lifetime or a newer dialog identity.
    pub(crate) fn clear_dialog_session_retained_exact(
        &self,
        handle: &SessionRegistryHandle,
        expected_dialog_id: &DialogId,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let Some(cell) = self.cell_for_handle(handle) else {
            return Ok(false);
        };
        let _cell_update = cell.lock_update();
        let current = cell.snapshot();
        if !Self::state_matches_handle(current.state(), handle) {
            return Ok(false);
        }
        let mut session = current.state().clone();
        if session.dialog_id.as_ref() != Some(expected_dialog_id) {
            return Ok(false);
        }

        let _mutation = self.lock_mutations();
        if !self.map_owns_cell(handle, &cell) {
            return Ok(false);
        }
        self.by_dialog
            .remove_if(expected_dialog_id, |_, owner| owner == handle);
        session.dialog_id = None;
        let _ = cell.publish(session);
        Ok(true)
    }

    pub async fn update_session(
        &self,
        session: SessionState,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.update_session_and_snapshot(session).map(|_| ())
    }

    /// Publish an owned state and return the exact immutable revision that was
    /// installed.
    ///
    /// State-machine hot paths use this after their final event-local update:
    /// they can move the large `SessionState` into the cell, then use this
    /// stable `Arc` for event materialization instead of cloning the state and
    /// loading it again.  The existing owned `update_session` API remains the
    /// compatibility wrapper.
    pub(crate) fn update_session_and_snapshot(
        &self,
        session: SessionState,
    ) -> Result<Arc<SessionStateSnapshot>, Box<dyn std::error::Error + Send + Sync>> {
        let handle = session
            .lifecycle_handle
            .clone()
            .ok_or("session state has no admitted lifecycle handle")?;
        if session.session_id != *handle.session_id() {
            return Err("session state identifier does not match its lifecycle handle".into());
        }
        self.replace_session_exact(&handle, session)
    }

    /// Publish state-machine-owned event state without resurrecting stale
    /// tracked-request staging slots.
    ///
    /// Builders stage these four slots through atomic cell updates while
    /// different-method state-machine actions may execute concurrently. The
    /// action transfers its own slot into the request tracker before clearing
    /// the stored slot. Merging the cell's current values while its update lock
    /// is held preserves a newer builder stage without changing the semantics
    /// of the public owned-state compatibility replacement APIs.
    pub(crate) fn update_state_machine_session_and_snapshot(
        &self,
        session: SessionState,
        auth_required_event: bool,
    ) -> Result<Arc<SessionStateSnapshot>, Box<dyn std::error::Error + Send + Sync>> {
        let handle = session
            .lifecycle_handle
            .clone()
            .ok_or("session state has no admitted lifecycle handle")?;
        if session.session_id != *handle.session_id() {
            return Err("session state identifier does not match its lifecycle handle".into());
        }
        self.replace_session_exact_inner(&handle, session, true, !auth_required_event)
    }

    /// Publish an already-owned compatibility state without first cloning the
    /// current state into a throwaway mutable working copy. The state-machine
    /// executor still uses this API for its event-local working state, so the
    /// avoided clone is on the call-setup hot path.
    fn replace_session_exact(
        &self,
        handle: &SessionRegistryHandle,
        session: SessionState,
    ) -> Result<Arc<SessionStateSnapshot>, Box<dyn std::error::Error + Send + Sync>> {
        self.replace_session_exact_inner(handle, session, false, false)
    }

    fn replace_session_exact_inner(
        &self,
        handle: &SessionRegistryHandle,
        mut session: SessionState,
        merge_tracked_request_staging: bool,
        preserve_auth_coordination: bool,
    ) -> Result<Arc<SessionStateSnapshot>, Box<dyn std::error::Error + Send + Sync>> {
        if session.session_id != *handle.session_id()
            || session.lifecycle_handle.as_ref() != Some(handle)
        {
            return Err("session update changed its exact lifecycle identity".into());
        }

        let guard = self
            .authority
            .try_operation_exact(handle.key(), SessionOperationKind::StateTransition)?;
        let Some(cell) = self.cell_for_handle(handle) else {
            self.update_missing_total.fetch_add(1, Ordering::Relaxed);
            guard.finish_rollback();
            return Err("session lifetime is not present in the store".into());
        };

        let _cell_update = cell.lock_update();
        let old_snapshot = cell.snapshot();
        if !Self::state_matches_handle(old_snapshot.state(), handle) {
            self.update_missing_total.fetch_add(1, Ordering::Relaxed);
            guard.finish_rollback();
            return Err("session lifetime is not present in the store".into());
        }

        let old_session = old_snapshot.state();
        let final_state = session.call_state.is_final();
        let tracked_staging_changed = merge_tracked_request_staging
            && !final_state
            && (!same_optional_arc(
                &session.pending_refer_options,
                &old_session.pending_refer_options,
            ) || !same_optional_arc(
                &session.pending_notify_options,
                &old_session.pending_notify_options,
            ) || !same_optional_arc(
                &session.pending_info_options,
                &old_session.pending_info_options,
            ) || !same_optional_arc(
                &session.pending_update_options,
                &old_session.pending_update_options,
            ));
        if tracked_staging_changed {
            session.pending_refer_options = old_session.pending_refer_options.clone();
            session.pending_notify_options = old_session.pending_notify_options.clone();
            session.pending_info_options = old_session.pending_info_options.clone();
            session.pending_update_options = old_session.pending_update_options.clone();
        }
        let auth_coordination_changed = preserve_auth_coordination
            && !final_state
            && (session.pending_auth != old_session.pending_auth
                || session.pending_auth_method != old_session.pending_auth_method
                || session.pending_auth_transport != old_session.pending_auth_transport
                || session.pending_auth_transaction_id != old_session.pending_auth_transaction_id
                || session.pending_auth_request_uri != old_session.pending_auth_request_uri
                || session.request_auth_retry_count != old_session.request_auth_retry_count
                || session.auth_challenge != old_session.auth_challenge
                || session.auth_challenge_raw != old_session.auth_challenge_raw
                || session.auth_challenge_stale != old_session.auth_challenge_stale
                || session.auth_challenge_replaces_nonce
                    != old_session.auth_challenge_replaces_nonce
                || session.digest_nc != old_session.digest_nc);
        if auth_coordination_changed {
            session.pending_auth = old_session.pending_auth.clone();
            session.pending_auth_method = old_session.pending_auth_method.clone();
            session.pending_auth_transport = old_session.pending_auth_transport.clone();
            session.pending_auth_transaction_id = old_session.pending_auth_transaction_id.clone();
            session.pending_auth_request_uri = old_session.pending_auth_request_uri.clone();
            session.request_auth_retry_count = old_session.request_auth_retry_count;
            session.auth_challenge = old_session.auth_challenge.clone();
            session.auth_challenge_raw = old_session.auth_challenge_raw.clone();
            session.auth_challenge_stale = old_session.auth_challenge_stale;
            session.auth_challenge_replaces_nonce =
                old_session.auth_challenge_replaces_nonce.clone();
            session.digest_nc = old_session.digest_nc.clone();
        }
        let index_delta = SessionIndexDelta::between(old_session, &session);
        let _mutation = (!index_delta.is_empty()).then(|| self.lock_mutations());
        if !self.map_owns_cell(handle, &cell) {
            self.update_missing_total.fetch_add(1, Ordering::Relaxed);
            guard.finish_rollback();
            return Err("session lifetime is not present in the store".into());
        }
        if !index_delta.is_empty() {
            if let Err(error) = self.reserve_index_delta(&index_delta, handle) {
                guard.finish_rollback();
                return Err(error);
            }
            self.commit_index_delta(&index_delta, handle);
            self.index_delta_committed();
        }

        let (previous, published) = cell.publish(session);
        match guard.finish() {
            Ok(()) => {
                debug!(session_id = %handle.session_id(), "updated exact SIP session lifetime");
                Ok(published)
            }
            Err(failure) => {
                if !index_delta.is_empty() {
                    self.rollback_index_delta(&index_delta, handle);
                }
                cell.restore(previous);
                let error = failure.error();
                failure.into_guard().finish_rollback();
                Err(Box::new(error))
            }
        }
    }

    /// Atomically read, mutate, and publish one session revision.
    ///
    /// This avoids the compatibility API's clone/modify/update window and is
    /// the preferred form for concurrent state-machine helpers. Sessions that
    /// do not change dialog/call/media identities never contend on the global
    /// structural lock.
    pub async fn update_session_with<R>(
        &self,
        session_id: &SessionId,
        update: impl FnOnce(&mut SessionState) -> R,
    ) -> Result<R, Box<dyn std::error::Error + Send + Sync>> {
        let handle = self
            .lifecycle_handle(session_id)
            .ok_or_else(|| format!("Session {session_id} not found"))?;
        self.update_session_exact_with(&handle, None, update)
    }

    /// Conditionally mutate the exact revision represented by `snapshot`.
    /// A concurrent writer produces a clear stale-revision error rather than
    /// silently overwriting its changes.
    pub async fn update_session_snapshot_with<R>(
        &self,
        snapshot: &SessionStateSnapshot,
        update: impl FnOnce(&mut SessionState) -> R,
    ) -> Result<R, Box<dyn std::error::Error + Send + Sync>> {
        let handle = snapshot
            .lifecycle_handle
            .as_ref()
            .ok_or("session snapshot has no admitted lifecycle handle")?;
        self.update_session_exact_with(handle, Some(snapshot.revision()), update)
    }

    /// Synchronously clear a builder staging field only when the caller's
    /// pointer-qualified value is still installed on this exact lifetime.
    ///
    /// The supplied closure performs the field-specific `Arc::ptr_eq` check.
    /// Keeping the cell mutation synchronous lets an ownership guard call it
    /// from `Drop`, including after the Tokio runtime has shut down.
    pub(crate) fn clear_staged_options_exact(
        &self,
        handle: &SessionRegistryHandle,
        clear_if_exact: impl FnOnce(&mut SessionState) -> bool,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        self.update_session_exact_with(handle, None, clear_if_exact)
    }

    /// Synchronously mutate one generation-qualified session cell.
    ///
    /// This is crate-visible for cancellation guards that run from `Drop`:
    /// they cannot await or spawn best-effort cleanup, but still need the same
    /// authority fence, exact-cell lock, and rollback behavior as normal
    /// session updates.
    pub(crate) fn update_session_exact_with<R>(
        &self,
        handle: &SessionRegistryHandle,
        expected_revision: Option<u64>,
        update: impl FnOnce(&mut SessionState) -> R,
    ) -> Result<R, Box<dyn std::error::Error + Send + Sync>> {
        let guard = self
            .authority
            .try_operation_exact(handle.key(), SessionOperationKind::StateTransition)?;
        let Some(cell) = self.cell_for_handle(handle) else {
            self.update_missing_total.fetch_add(1, Ordering::Relaxed);
            guard.finish_rollback();
            return Err("session lifetime is not present in the store".into());
        };

        let _cell_update = cell.lock_update();
        let old_snapshot = cell.snapshot();
        if !Self::state_matches_handle(old_snapshot.state(), handle) {
            self.update_missing_total.fetch_add(1, Ordering::Relaxed);
            guard.finish_rollback();
            return Err("session lifetime is not present in the store".into());
        }
        if expected_revision.is_some_and(|revision| revision != old_snapshot.revision()) {
            guard.finish_rollback();
            return Err("session snapshot revision is stale".into());
        }
        let old_session = old_snapshot.state();
        let mut session = old_session.clone();
        let result = update(&mut session);
        if session.session_id != *handle.session_id()
            || session.lifecycle_handle.as_ref() != Some(handle)
        {
            guard.finish_rollback();
            return Err("session update changed its exact lifecycle identity".into());
        }

        let index_delta = SessionIndexDelta::between(old_session, &session);
        let _mutation = (!index_delta.is_empty()).then(|| self.lock_mutations());
        if !self.map_owns_cell(handle, &cell) {
            self.update_missing_total.fetch_add(1, Ordering::Relaxed);
            guard.finish_rollback();
            return Err("session lifetime is not present in the store".into());
        }
        if !index_delta.is_empty() {
            if let Err(error) = self.reserve_index_delta(&index_delta, handle) {
                guard.finish_rollback();
                return Err(error);
            }
            self.commit_index_delta(&index_delta, handle);
            self.index_delta_committed();
        }

        let (previous, _published) = cell.publish(session);

        match guard.finish() {
            Ok(()) => {
                debug!(session_id = %handle.session_id(), "updated exact SIP session lifetime");
                Ok(result)
            }
            Err(failure) => {
                if !index_delta.is_empty() {
                    self.rollback_index_delta(&index_delta, handle);
                }
                cell.restore(previous);
                let error = failure.error();
                failure.into_guard().finish_rollback();
                Err(Box::new(error))
            }
        }
    }

    pub(crate) async fn quiesce_session_exact(
        &self,
        handle: &SessionRegistryHandle,
    ) -> Result<TeardownOutcome, Box<dyn std::error::Error + Send + Sync>> {
        if self.cell_for_handle(handle).is_none() {
            return Err("exact session lifetime is not present in the store".into());
        }
        let waiter = self
            .authority
            .teardown(handle.key(), SESSION_TEARDOWN_TIMEOUT)?;
        let supervisor = waiter.clone();
        let outcome = waiter.wait().await?;
        supervisor.wait_supervisor().await?;
        Ok(outcome)
    }

    pub(crate) fn remove_quiesced_session_exact(
        &self,
        handle: &SessionRegistryHandle,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !matches!(
            self.authority.phase(handle.key()),
            Some(crate::session_lifecycle::SessionPhase::Retired { .. })
        ) {
            return Err("session lifetime must be retired before exact removal".into());
        }
        let guard =
            cleanup_diag::stage_guard(CleanupStage::SessionStoreRemoval, &handle.session_id().0);
        let Some(cell) = self.cell_for_handle(handle) else {
            self.remove_missing_total.fetch_add(1, Ordering::Relaxed);
            return Err("exact session lifetime is not present in the store".into());
        };
        let _cell_update = cell.lock_update();
        let snapshot = cell.snapshot();
        if !Self::state_matches_handle(snapshot.state(), handle) {
            self.remove_missing_total.fetch_add(1, Ordering::Relaxed);
            return Err("exact session lifetime is not present in the store".into());
        }
        let _mutation = self.lock_mutations();
        if !self.map_owns_cell(handle, &cell) {
            self.remove_missing_total.fetch_add(1, Ordering::Relaxed);
            return Err("exact session lifetime is not present in the store".into());
        }
        if !self.registry.remove_handle(handle)? {
            return Err("exact registry slot was missing during store removal".into());
        }
        self.sessions
            .remove_if(handle.session_id(), |_, candidate| {
                Arc::ptr_eq(candidate, &cell)
            });
        self.remove_indexes_for_session(snapshot.state(), handle);
        self.removed_total.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "perf-infra-memory-diagnostics")]
        rvoip_infra_common::memory_diagnostics::record_dropped(
            "sip.session_store.session",
            std::mem::size_of::<SessionState>(),
        );
        guard.finish_success();
        info!(session_id = %handle.session_id(), "removed exact SIP session lifetime");
        Ok(())
    }

    pub(crate) async fn remove_session_exact(
        &self,
        handle: &SessionRegistryHandle,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        match self.quiesce_session_exact(handle).await? {
            TeardownOutcome::Retired { .. } => self.remove_quiesced_session_exact(handle),
            TeardownOutcome::Quarantined { reason, .. } => {
                Err(format!("session teardown quarantined: {reason:?}").into())
            }
        }
    }

    /// Compatibility removal captures the current exact handle immediately.
    /// Production delayed cleanup paths are migrated to `remove_session_exact`.
    pub async fn remove_session(
        &self,
        session_id: &SessionId,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let handle = self
            .lifecycle_handle(session_id)
            .ok_or_else(|| format!("Session {session_id} not found"))?;
        self.remove_session_exact(&handle).await
    }

    #[cfg(feature = "perf-tests")]
    pub(crate) fn perf_lifecycle_counts(&self) -> serde_json::Value {
        let created = self.created_total.load(Ordering::Relaxed);
        let removed = self.removed_total.load(Ordering::Relaxed);
        let authority = self.authority.diagnostics();
        serde_json::json!({
            "created_total": created,
            "removed_total": removed,
            "remove_missing_total": self.remove_missing_total.load(Ordering::Relaxed),
            "update_missing_total": self.update_missing_total.load(Ordering::Relaxed),
            "net_created_minus_removed": created.saturating_sub(removed),
            "authority": {
                "active_capacity": authority.capacity,
                "active_capacity_in_use": authority.active_capacity_in_use,
                "retained_capacity": authority.retained_capacity,
                "retained_total": authority.lifecycle_count,
                "admission_index_capacity": authority.admission_index_capacity,
                "reusable_deadline_capacity": authority.reusable_deadline_capacity,
                "retained_identifier_payload_bytes": authority.retained_identifier_payload_bytes,
                "current_index_capacity": authority.current_index_capacity,
                "exact_cell_index_capacity": authority.exact_cell_index_capacity,
                "record_inline_bytes": {
                    "session_id": std::mem::size_of::<crate::state_table::SessionId>(),
                    "session_key": std::mem::size_of::<crate::session_lifecycle::SessionKey>(),
                },
                "active": authority.active,
                "quiescing": authority.quiescing,
                "releasing": authority.releasing,
                "quarantined": authority.quarantined,
                "retired": authority.retired,
                "index_live": authority.index_live,
                "index_blocked": authority.index_blocked,
            },
        })
    }

    pub async fn find_by_dialog(&self, dialog_id: &DialogId) -> Option<SessionState> {
        let handle = self.by_dialog.get(dialog_id)?.value().clone();
        self.get_session_exact(&handle)
            .await
            .ok()
            .filter(|session| session.dialog_id.as_ref() == Some(dialog_id))
    }

    pub async fn find_by_media_id(&self, media_id: &MediaSessionId) -> Option<SessionState> {
        let handle = self.by_media_id.get(media_id)?.value().clone();
        self.get_session_exact(&handle)
            .await
            .ok()
            .filter(|session| session.media_session_id.as_ref() == Some(media_id))
    }

    pub async fn find_by_call_id(&self, call_id: &CallId) -> Option<SessionState> {
        let handle = self.by_call_id.get(call_id)?.value().clone();
        self.get_session_exact(&handle)
            .await
            .ok()
            .filter(|session| session.call_id.as_ref() == Some(call_id))
    }

    pub async fn get_all_sessions(&self) -> Vec<SessionState> {
        self.sessions
            .iter()
            .filter_map(|entry| {
                let snapshot = entry.value().snapshot();
                let session = snapshot.state();
                let handle = session.lifecycle_handle.as_ref()?;
                self.authority
                    .is_current(handle.key())
                    .then(|| session.clone())
            })
            .collect()
    }

    pub async fn has_session(&self) -> bool {
        self.sessions.iter().any(|entry| {
            entry
                .value()
                .snapshot()
                .lifecycle_handle
                .as_ref()
                .is_some_and(|handle| self.authority.is_current(handle.key()))
        })
    }

    pub async fn get_current_session_id(&self) -> Option<SessionId> {
        self.sessions
            .iter()
            .filter_map(|entry| {
                let snapshot = entry.value().snapshot();
                let handle = snapshot.lifecycle_handle.as_ref()?;
                self.authority
                    .is_current(handle.key())
                    .then(|| (entry.key().clone(), snapshot.created_at))
            })
            .max_by_key(|(_, created_at)| *created_at)
            .map(|(session_id, _)| session_id)
    }

    pub async fn clear(&self) {
        let handles: Vec<_> = self
            .sessions
            .iter()
            .filter_map(|entry| entry.value().snapshot().lifecycle_handle.clone())
            .collect();
        for handle in handles {
            let _ = self.remove_session_exact(&handle).await;
        }
        info!("cleared all exact SIP session lifetimes");
    }

    pub async fn get_stats(&self) -> SessionStats {
        let mut stats = SessionStats::default();
        for session in self.get_all_sessions().await {
            stats.total += 1;
            match session.call_state {
                CallState::Idle | CallState::Registered | CallState::Subscribed => stats.idle += 1,
                CallState::Initiating
                | CallState::CancelPending
                | CallState::Registering
                | CallState::Subscribing
                | CallState::Publishing
                | CallState::Authenticating => stats.initiating += 1,
                CallState::Ringing | CallState::Answering => stats.ringing += 1,
                CallState::Cancelling
                | CallState::AnsweringHangupPending
                | CallState::Terminating
                | CallState::Unregistering => stats.terminating += 1,
                CallState::Terminated | CallState::Cancelled => stats.terminated += 1,
                CallState::Failed(_) => stats.failed += 1,
                CallState::OnHold => stats.on_hold += 1,
                CallState::EarlyMedia
                | CallState::Active
                | CallState::HoldPending
                | CallState::Resuming
                | CallState::Bridged
                | CallState::Transferring
                | CallState::TransferringCall
                | CallState::Muted
                | CallState::ConsultationCall
                | CallState::Messaging => stats.active += 1,
            }
        }
        stats
    }
}

impl Default for SessionStore {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Default, Clone)]
pub struct SessionStats {
    pub total: usize,
    pub idle: usize,
    pub initiating: usize,
    pub ringing: usize,
    pub active: usize,
    pub on_hold: usize,
    pub terminating: usize,
    pub terminated: usize,
    pub failed: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_capacity_is_logical_and_store_indexes_reserve_lazily() {
        const LOGICAL_CAPACITY: usize = 20_000;

        let store = SessionStore::with_capacity(LOGICAL_CAPACITY);
        assert_eq!(store.authority.diagnostics().capacity, LOGICAL_CAPACITY);
        for capacity in [
            store.sessions.capacity(),
            store.by_dialog.capacity(),
            store.by_call_id.capacity(),
            store.by_media_id.capacity(),
        ] {
            assert!(capacity < LOGICAL_CAPACITY);
        }
    }

    #[tokio::test]
    async fn retained_exact_dialog_clear_is_expected_value_fenced() {
        let store = SessionStore::new();
        let session_id = SessionId::new();
        let created = store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create retained dialog session");
        let handle = created
            .lifecycle_handle
            .clone()
            .expect("session has exact registry handle");
        let dialog_id = DialogId::new();
        store
            .update_session_with(&session_id, |session| {
                session.dialog_id = Some(dialog_id);
            })
            .await
            .expect("publish exact dialog identity");

        assert!(!store
            .clear_dialog_session_retained_exact(&handle, &DialogId::new())
            .expect("mismatched retained clear is a no-op"));
        assert_eq!(
            store
                .get_session_retained_snapshot_exact(&handle)
                .expect("read retained dialog after mismatch")
                .dialog_id,
            Some(dialog_id)
        );
        assert!(store
            .clear_dialog_session_retained_exact(&handle, &dialog_id)
            .expect("clear exact retained dialog"));
        assert_eq!(
            store
                .get_session_retained_snapshot_exact(&handle)
                .expect("read retained dialog after clear")
                .dialog_id,
            None
        );
        assert!(!store.by_dialog.contains_key(&dialog_id));
    }

    #[tokio::test]
    async fn exact_state_machine_lane_serializes_same_session_events() {
        let store = Arc::new(SessionStore::new());
        let session_id = SessionId::new();
        store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create state-machine lane session");
        let (_, lane) = store
            .state_machine_lane(&session_id)
            .expect("resolve exact state-machine lane");
        let (_, same_lane) = store
            .state_machine_lane(&session_id)
            .expect("resolve same exact state-machine lane");
        assert!(Arc::ptr_eq(&lane, &same_lane));

        let first = lane.lock_owned().await;
        let attempted = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let entered = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let waiting_store = Arc::clone(&store);
        let waiting_session = session_id.clone();
        let waiting_attempted = Arc::clone(&attempted);
        let waiting_entered = Arc::clone(&entered);
        let waiter = tokio::spawn(async move {
            let (_, lane) = waiting_store
                .state_machine_lane(&waiting_session)
                .expect("resolve queued state-machine lane");
            waiting_attempted.store(true, Ordering::Release);
            let _guard = lane.lock_owned().await;
            waiting_entered.store(true, Ordering::Release);
        });
        while !attempted.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
        tokio::task::yield_now().await;
        assert!(!entered.load(Ordering::Acquire));

        drop(first);
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("queued state-machine event remained blocked")
            .expect("queued state-machine task panicked");
        assert!(entered.load(Ordering::Acquire));
    }

    #[test]
    fn index_delta_reserves_only_identifiers_that_changed() {
        let session_id = SessionId::new();
        let mut old = SessionState::new(session_id, Role::UAC);
        old.dialog_id = Some(DialogId::new());
        old.media_session_id = Some(MediaSessionId::new("delta-old-media"));
        old.call_id = Some("delta-call".to_string());
        let mut new = old.clone();
        new.media_session_id = Some(MediaSessionId::new("delta-new-media"));

        let delta = SessionIndexDelta::between(&old, &new);

        assert!(
            delta.dialog.is_none(),
            "unchanged dialog route was reserved"
        );
        assert!(delta.media.is_some(), "changed media route was omitted");
        assert!(delta.call.is_none(), "unchanged Call-ID route was reserved");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unchanged_lookup_survives_index_commit_and_teardown_rolls_delta_back() {
        let store = Arc::new(SessionStore::new());
        let session_id = SessionId::new();
        let dialog_id = DialogId::new();
        let old_media_id = MediaSessionId::new("rollback-old-media");
        let new_media_id = MediaSessionId::new("rollback-new-media");
        let call_id = "rollback-call".to_string();
        let mut session = store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create exact session");
        session.dialog_id = Some(dialog_id);
        session.media_session_id = Some(old_media_id.clone());
        session.call_id = Some(call_id.clone());
        store
            .update_session(session)
            .await
            .expect("publish initial indexes");
        let handle = store
            .lifecycle_handle(&session_id)
            .expect("exact lifecycle handle");

        let committed = Arc::new(std::sync::Barrier::new(2));
        let release = Arc::new(std::sync::Barrier::new(2));
        let hook_committed = Arc::clone(&committed);
        let hook_release = Arc::clone(&release);
        *store.after_index_delta_commit.lock().expect("index hook") = Some(Arc::new(move || {
            hook_committed.wait();
            hook_release.wait();
        }));

        let mut changed = store
            .get_session(&session_id)
            .await
            .expect("current session");
        changed.media_session_id = Some(new_media_id.clone());
        let update_store = Arc::clone(&store);
        let update = tokio::spawn(async move { update_store.update_session(changed).await });

        tokio::task::spawn_blocking(move || committed.wait())
            .await
            .expect("wait for index commit");

        let during = store
            .find_by_dialog(&dialog_id)
            .await
            .expect("unchanged dialog route remains continuously resolvable");
        assert_eq!(during.session_id, session_id);
        assert_eq!(during.call_id.as_ref(), Some(&call_id));

        let teardown = store
            .authority
            .teardown(handle.key(), SESSION_TEARDOWN_TIMEOUT)
            .expect("quiesce exact lifetime after index commit");
        let teardown_supervisor = teardown.clone();
        tokio::task::spawn_blocking(move || release.wait())
            .await
            .expect("release index update");

        let update_error = update
            .await
            .expect("index update task")
            .expect_err("quiescing must reject lifecycle commit");
        assert!(update_error.to_string().contains("no longer current"));

        assert_eq!(
            store
                .by_dialog
                .get(&dialog_id)
                .expect("unchanged dialog index")
                .value(),
            &handle
        );
        assert_eq!(
            store
                .by_call_id
                .get(&call_id)
                .expect("unchanged Call-ID index")
                .value(),
            &handle
        );
        assert_eq!(
            store
                .by_media_id
                .get(&old_media_id)
                .expect("rolled-back media index")
                .value(),
            &handle
        );
        assert!(!store.by_media_id.contains_key(&new_media_id));
        assert_eq!(
            store
                .get_session_retained_exact(&handle)
                .await
                .expect("rolled-back exact state")
                .media_session_id
                .as_ref(),
            Some(&old_media_id)
        );

        let _ = teardown.wait().await.expect("terminal teardown outcome");
        teardown_supervisor
            .wait_supervisor()
            .await
            .expect("teardown supervisor");
        *store.after_index_delta_commit.lock().expect("index hook") = None;
    }

    #[tokio::test]
    async fn owned_publication_returns_the_exact_stable_revision() {
        let store = SessionStore::new();
        let session_id = SessionId::new();
        let mut session = store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .unwrap();
        session.call_state = CallState::Initiating;

        let published = store.update_session_and_snapshot(session).unwrap();
        assert_eq!(published.revision(), 2);
        assert_eq!(published.call_state, CallState::Initiating);

        store
            .update_session_with(&session_id, |session| {
                session.call_state = CallState::Active;
            })
            .await
            .unwrap();

        assert_eq!(published.call_state, CallState::Initiating);
        assert_eq!(
            store
                .get_session_snapshot(&session_id)
                .await
                .unwrap()
                .call_state,
            CallState::Active
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn state_machine_publication_preserves_concurrent_tracked_staging_slots() {
        let store = Arc::new(SessionStore::new());
        let session_id = SessionId::new();
        let mut event_local = store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .unwrap();

        let info_store = Arc::clone(&store);
        let info_session = session_id.clone();
        let notify_store = Arc::clone(&store);
        let notify_session = session_id.clone();
        let (info_result, notify_result) = tokio::join!(
            async move {
                info_store
                    .update_session_with(&info_session, |session| {
                        session.pending_info_options = Some(Arc::new(Default::default()));
                    })
                    .await
            },
            async move {
                notify_store
                    .update_session_with(&notify_session, |session| {
                        session.pending_notify_options = Some(Arc::new(Default::default()));
                    })
                    .await
            }
        );
        info_result.unwrap();
        notify_result.unwrap();

        event_local.call_state = CallState::Active;
        store
            .update_state_machine_session_and_snapshot(event_local, false)
            .unwrap();

        let current = store.get_session(&session_id).await.unwrap();
        assert!(current.pending_info_options.is_some());
        assert!(current.pending_notify_options.is_some());
        assert_eq!(current.call_state, CallState::Active);
    }

    #[tokio::test]
    async fn non_auth_publication_preserves_concurrent_exact_auth_coordination() {
        let store = Arc::new(SessionStore::new());
        let session_id = SessionId::new();
        let mut stale_non_auth_event = store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .unwrap();

        store
            .update_session_with(&session_id, |current| {
                current.pending_auth = Some((401, "Digest redacted".to_string()));
                current.pending_auth_method = Some("INFO".to_string());
                current.pending_auth_transaction_id = Some("exact-info-tx".to_string());
                current.pending_auth_request_uri = Some("sip:callee@example.invalid".to_string());
                current.request_auth_retry_count = 2;
                current.auth_challenge_raw = Some("Digest redacted".to_string());
                current.auth_challenge_stale = true;
                current.auth_challenge_replaces_nonce = Some("prior-nonce".to_string());
                current
                    .digest_nc
                    .insert(("realm".to_string(), "nonce".to_string()), 7);
            })
            .await
            .unwrap();

        stale_non_auth_event.call_state = CallState::Active;
        store
            .update_state_machine_session_and_snapshot(stale_non_auth_event, false)
            .unwrap();

        let current = store.get_session(&session_id).await.unwrap();
        assert_eq!(current.pending_auth_method.as_deref(), Some("INFO"));
        assert_eq!(
            current.pending_auth_transaction_id.as_deref(),
            Some("exact-info-tx")
        );
        assert_eq!(current.request_auth_retry_count, 2);
        assert!(current.auth_challenge_stale);
        assert_eq!(
            current
                .digest_nc
                .get(&("realm".to_string(), "nonce".to_string())),
            Some(&7)
        );
    }

    #[tokio::test]
    async fn auth_required_publication_is_authoritative_for_auth_coordination() {
        let store = Arc::new(SessionStore::new());
        let session_id = SessionId::new();
        let mut auth_event = store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .unwrap();
        auth_event.pending_auth = Some((407, "Digest new".to_string()));
        auth_event.pending_auth_method = Some("NOTIFY".to_string());
        auth_event.pending_auth_transaction_id = Some("new-notify-tx".to_string());
        auth_event.pending_auth_request_uri = Some("sip:new@example.invalid".to_string());
        auth_event
            .digest_nc
            .insert(("new-realm".to_string(), "new-nonce".to_string()), 1);

        store
            .update_session_with(&session_id, |current| {
                current.pending_auth_method = Some("INFO".to_string());
                current.pending_auth_transaction_id = Some("old-info-tx".to_string());
                current.digest_nc.clear();
            })
            .await
            .unwrap();

        store
            .update_state_machine_session_and_snapshot(auth_event, true)
            .unwrap();
        let current = store.get_session(&session_id).await.unwrap();
        assert_eq!(current.pending_auth_method.as_deref(), Some("NOTIFY"));
        assert_eq!(
            current.pending_auth_transaction_id.as_deref(),
            Some("new-notify-tx")
        );
        assert_eq!(
            current
                .digest_nc
                .get(&("new-realm".to_string(), "new-nonce".to_string())),
            Some(&1)
        );
    }

    #[tokio::test]
    async fn final_state_publication_does_not_restore_staging_or_auth() {
        let store = Arc::new(SessionStore::new());
        let session_id = SessionId::new();
        let mut final_event = store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .unwrap();

        store
            .update_session_with(&session_id, |current| {
                current.pending_info_options = Some(Arc::new(Default::default()));
                current.pending_auth = Some((401, "Digest redacted".to_string()));
                current.pending_auth_method = Some("INFO".to_string());
                current.pending_auth_transaction_id = Some("late-info-tx".to_string());
                current.auth_challenge_raw = Some("Digest redacted".to_string());
                current
                    .digest_nc
                    .insert(("realm".to_string(), "nonce".to_string()), 1);
            })
            .await
            .unwrap();

        final_event.call_state = CallState::Terminated;
        final_event.clear_pending_request_state_for_final_transition();
        store
            .update_state_machine_session_and_snapshot(final_event, false)
            .unwrap();

        let current = store.get_session(&session_id).await.unwrap();
        assert!(current.pending_info_options.is_none());
        assert!(current.pending_auth.is_none());
        assert!(current.pending_auth_method.is_none());
        assert!(current.pending_auth_transaction_id.is_none());
        assert!(current.auth_challenge_raw.is_none());
        assert!(current.digest_nc.is_empty());
    }
}
