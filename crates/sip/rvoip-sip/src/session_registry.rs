//! Generation-qualified SIP session mappings.
//!
//! Admission, identifier reuse, and lifetime fencing belong exclusively to
//! `SessionLeaseAuthority`. This registry stores mappings for admitted
//! `SessionKey` values; it deliberately has no tombstone, timeout, capacity,
//! or generation policy of its own.

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex as StdMutex, MutexGuard as StdMutexGuard};

use rvoip_core_traits::identity::AuthenticatedPrincipal;
use thiserror::Error;

use crate::auth::SipTransportSecurityContext;
use crate::session_lifecycle::{
    OperationGuard, SessionKey, SessionLeaseAuthority, SessionOperationError, SessionOperationKind,
};
use crate::types::{DialogId, IncomingCallInfo, MediaSessionId, SessionId};

/// Monotonic identity for one registry slot.
///
/// The revision is intentionally separate from the authority generation. A
/// generation may be removed and accidentally targeted by delayed cleanup;
/// requiring both values makes that cleanup conditional on the exact slot it
/// observed.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct RegistrySlotRevision(u64);

/// Retained exact identity for one generation-qualified registry slot.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SessionRegistryHandle {
    key: SessionKey,
    slot_revision: RegistrySlotRevision,
}

impl SessionRegistryHandle {
    pub(crate) fn key(&self) -> &SessionKey {
        &self.key
    }

    pub(crate) fn session_id(&self) -> &SessionId {
        &self.key.session_id
    }

    pub(crate) fn slot_revision(&self) -> RegistrySlotRevision {
        self.slot_revision
    }

    /// Test-only construction of a later registry incarnation for the same
    /// authority generation. Production revisions remain registry-owned.
    #[cfg(test)]
    pub(crate) fn with_next_slot_revision_for_test(&self) -> Self {
        Self {
            key: self.key.clone(),
            slot_revision: RegistrySlotRevision(self.slot_revision.0.wrapping_add(1)),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct IndexOwner {
    key: SessionKey,
    slot_revision: RegistrySlotRevision,
}

#[derive(Default)]
struct MutationGate {
    occupied: StdMutex<bool>,
    available: Condvar,
}

impl MutationGate {
    fn acquire(self: &Arc<Self>) -> Result<MutationPermit, SessionRegistryError> {
        let mut occupied = self
            .occupied
            .lock()
            .map_err(|_| SessionRegistryError::Poisoned)?;
        while *occupied {
            occupied = self
                .available
                .wait(occupied)
                .map_err(|_| SessionRegistryError::Poisoned)?;
        }
        *occupied = true;
        Ok(MutationPermit {
            gate: Arc::clone(self),
        })
    }
}

/// Owned serialization permit for one exact session slot.
///
/// Authority operations may overlap, but their registry publications must
/// finalize or compensate in installation order. Otherwise a later failed
/// mutation could restore an earlier speculative value after that earlier
/// operation had already skipped its conditional rollback.
struct MutationPermit {
    gate: Arc<MutationGate>,
}

impl Drop for MutationPermit {
    fn drop(&mut self) {
        let mut occupied = match self.gate.occupied.lock() {
            Ok(occupied) => occupied,
            Err(poisoned) => poisoned.into_inner(),
        };
        *occupied = false;
        self.gate.available.notify_one();
    }
}

/// All inbound INVITE material is updated and observed as one session-bound
/// value. Fields remain optional for compatibility with event paths that do
/// not retain every item.
#[derive(Clone, Default)]
pub(crate) struct PendingInboundBundle {
    pub(crate) info: Option<IncomingCallInfo>,
    pub(crate) request: Option<Arc<rvoip_sip_core::Request>>,
    pub(crate) transport: Option<Arc<SipTransportSecurityContext>>,
    pub(crate) principal: Option<AuthenticatedPrincipal>,
}

#[derive(Clone)]
struct RegistryEntry {
    slot_revision: RegistrySlotRevision,
    mutation_gate: Arc<MutationGate>,
    dialog_id: Option<DialogId>,
    dialog_mutation_revision: u64,
    media_id: Option<MediaSessionId>,
    media_mutation_revision: u64,
    pending: PendingInboundBundle,
    pending_mutation_revision: u64,
}

#[derive(Default)]
struct RegistryState {
    entries: HashMap<SessionKey, RegistryEntry>,
    by_dialog: HashMap<DialogId, IndexOwner>,
    by_media: HashMap<MediaSessionId, IndexOwner>,
    next_slot_revision: u64,
    next_mutation_revision: u64,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub(crate) enum SessionRegistryError {
    #[error("session registry mutex is poisoned")]
    Poisoned,
    #[error("session generation is no longer current")]
    StaleSession,
    #[error("session registry slot is missing")]
    SlotMissing,
    #[error("session registry slot revision does not match")]
    RevisionMismatch,
    #[error("session registry slot revision sequence is exhausted")]
    RevisionExhausted,
    #[error("dialog identifier is already owned by another registry slot")]
    DialogCollision,
    #[error("media identifier is already owned by another registry slot")]
    MediaCollision,
    #[error("session registry authority operation failed: {0}")]
    AuthorityOperation(SessionOperationError),
    #[error("session registry external-map compensation panicked")]
    ExternalCompensationPanicked,
}

/// Exact compensation returned by an external compatibility-map commit.
///
/// Production mutations should return a closure that removes only the values
/// installed by that commit. The `()` implementation exists solely to keep
/// legacy call sites source-compatible while they migrate; it cannot undo
/// external publication and must not be used on production authority paths.
pub(crate) trait ExternalMapCompensation {
    fn compensate(self);
}

impl ExternalMapCompensation for () {
    fn compensate(self) {}
}

impl<F> ExternalMapCompensation for F
where
    F: FnOnce(),
{
    fn compensate(self) {
        self();
    }
}

/// Exact, concurrent mappings for admitted SIP sessions.
///
/// A single synchronous mutex provides the commit boundary between an entry
/// bundle and both secondary indexes. Methods retain their historical async
/// signatures where compatibility requires it, but never hold this mutex
/// across an await.
#[cfg(test)]
type ExactMutationHook = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone)]
pub struct SessionRegistry {
    authority: Arc<SessionLeaseAuthority>,
    state: Arc<StdMutex<RegistryState>>,
    dialog_mapped_total: Arc<AtomicU64>,
    media_mapped_total: Arc<AtomicU64>,
    removed_total: Arc<AtomicU64>,
    remove_missing_total: Arc<AtomicU64>,
    #[cfg(test)]
    after_exact_mutation: Arc<StdMutex<Option<ExactMutationHook>>>,
}

/// Compensating receipt for a registry-side inbound-dialog commit.
///
/// External compatibility maps are intentionally updated only after the
/// registry mutex is released. The caller must finalize the receipt after
/// those updates succeed; dropping it restores the exact prior registry value
/// when no newer exact mutation superseded it.
#[must_use = "finalize the external commit or allow the receipt to compensate it"]
pub(crate) struct InboundDialogCommitReceipt {
    registry: SessionRegistry,
    key: SessionKey,
    revision: RegistrySlotRevision,
    installed_dialog: DialogId,
    installed_mutation_revision: u64,
    previous_dialog: Option<DialogId>,
    previous_dialog_mutation_revision: u64,
    previous_info: Option<IncomingCallInfo>,
    previous_pending_mutation_revision: u64,
    guard: Option<OperationGuard>,
    _mutation_permit: Option<MutationPermit>,
    armed: bool,
}

impl InboundDialogCommitReceipt {
    #[allow(dead_code)]
    pub(crate) fn finalize(self) -> Result<(), SessionRegistryError> {
        self.finalize_with_compensation(())
    }

    pub(crate) fn finalize_with_compensation<C>(
        mut self,
        compensation: C,
    ) -> Result<(), SessionRegistryError>
    where
        C: ExternalMapCompensation,
    {
        let guard = self.guard.take().expect("armed receipt authority guard");
        match guard.finish() {
            Ok(()) => {
                self.armed = false;
                Ok(())
            }
            Err(failure) => {
                let error = failure.error();
                let rollback = self.registry.rollback_inbound_commit(
                    &self.key,
                    self.revision,
                    self.installed_dialog,
                    self.installed_mutation_revision,
                    self.previous_dialog,
                    self.previous_dialog_mutation_revision,
                    self.previous_info.take(),
                    self.previous_pending_mutation_revision,
                );
                let external = catch_unwind(AssertUnwindSafe(|| compensation.compensate()));
                failure.into_guard().finish_rollback();
                self.armed = false;
                rollback?;
                if external.is_err() {
                    return Err(SessionRegistryError::ExternalCompensationPanicked);
                }
                Err(SessionRegistryError::AuthorityOperation(error))
            }
        }
    }

    #[allow(dead_code)]
    pub(crate) fn rollback(self) -> Result<bool, SessionRegistryError> {
        self.rollback_with_compensation(())
    }

    pub(crate) fn rollback_with_compensation<C>(
        mut self,
        compensation: C,
    ) -> Result<bool, SessionRegistryError>
    where
        C: ExternalMapCompensation,
    {
        self.armed = false;
        let rollback = self.registry.rollback_inbound_commit(
            &self.key,
            self.revision,
            self.installed_dialog,
            self.installed_mutation_revision,
            self.previous_dialog,
            self.previous_dialog_mutation_revision,
            self.previous_info.take(),
            self.previous_pending_mutation_revision,
        );
        let external = catch_unwind(AssertUnwindSafe(|| compensation.compensate()));
        if let Some(guard) = self.guard.take() {
            guard.finish_rollback();
        }
        let rolled_back = rollback?;
        if external.is_err() {
            return Err(SessionRegistryError::ExternalCompensationPanicked);
        }
        Ok(rolled_back)
    }
}

impl Drop for InboundDialogCommitReceipt {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.registry.rollback_inbound_commit(
                &self.key,
                self.revision,
                self.installed_dialog,
                self.installed_mutation_revision,
                self.previous_dialog,
                self.previous_dialog_mutation_revision,
                self.previous_info.take(),
                self.previous_pending_mutation_revision,
            );
            if let Some(guard) = self.guard.take() {
                guard.finish_rollback();
            }
            self.armed = false;
        }
    }
}

impl SessionRegistry {
    /// Compatibility constructor.
    ///
    /// Production coordinators must use [`Self::with_authority`] so admission
    /// and registry resolution share the same authority. A registry created by
    /// this constructor remains empty until its private authority admits a
    /// session, so legacy write calls fail closed rather than inventing a
    /// second lifetime.
    pub fn new() -> Self {
        Self::with_authority(SessionLeaseAuthority::new())
    }

    pub(crate) fn with_authority(authority: Arc<SessionLeaseAuthority>) -> Self {
        Self {
            authority,
            state: Arc::new(StdMutex::new(RegistryState::default())),
            dialog_mapped_total: Arc::new(AtomicU64::new(0)),
            media_mapped_total: Arc::new(AtomicU64::new(0)),
            removed_total: Arc::new(AtomicU64::new(0)),
            remove_missing_total: Arc::new(AtomicU64::new(0)),
            #[cfg(test)]
            after_exact_mutation: Arc::new(StdMutex::new(None)),
        }
    }

    pub(crate) fn authority(&self) -> &Arc<SessionLeaseAuthority> {
        &self.authority
    }

    fn lock_state(&self) -> Result<StdMutexGuard<'_, RegistryState>, SessionRegistryError> {
        self.state
            .lock()
            .map_err(|_| SessionRegistryError::Poisoned)
    }

    fn allocate_revision_locked(
        state: &mut RegistryState,
    ) -> Result<RegistrySlotRevision, SessionRegistryError> {
        state.next_slot_revision = state
            .next_slot_revision
            .checked_add(1)
            .ok_or(SessionRegistryError::RevisionExhausted)?;
        Ok(RegistrySlotRevision(state.next_slot_revision))
    }

    fn allocate_mutation_revision_locked(
        state: &mut RegistryState,
    ) -> Result<u64, SessionRegistryError> {
        state.next_mutation_revision = state
            .next_mutation_revision
            .checked_add(1)
            .ok_or(SessionRegistryError::RevisionExhausted)?;
        Ok(state.next_mutation_revision)
    }

    #[cfg(test)]
    fn set_after_exact_mutation_hook(&self, hook: Arc<dyn Fn() + Send + Sync>) {
        *self
            .after_exact_mutation
            .lock()
            .expect("exact mutation hook") = Some(hook);
    }

    #[cfg(test)]
    fn fire_after_exact_mutation_hook(&self) {
        let hook = self
            .after_exact_mutation
            .lock()
            .expect("exact mutation hook")
            .take();
        if let Some(hook) = hook {
            hook();
        }
    }

    #[cfg(not(test))]
    fn fire_after_exact_mutation_hook(&self) {}

    fn finish_unmutated_operation<T>(
        guard: OperationGuard,
        value: T,
    ) -> Result<T, SessionRegistryError> {
        match guard.finish() {
            Ok(()) => Ok(value),
            Err(failure) => {
                let error = failure.error();
                failure.into_guard().finish_rollback();
                Err(SessionRegistryError::AuthorityOperation(error))
            }
        }
    }

    fn finish_exact_mutation<T>(
        &self,
        guard: OperationGuard,
        value: T,
        rollback: impl FnOnce(&mut RegistryState) -> Result<(), SessionRegistryError>,
    ) -> Result<T, SessionRegistryError> {
        self.fire_after_exact_mutation_hook();
        match guard.finish() {
            Ok(()) => Ok(value),
            Err(failure) => {
                let error = failure.error();
                let rollback_result = self.lock_state().and_then(|mut state| rollback(&mut state));
                failure.into_guard().finish_rollback();
                rollback_result?;
                Err(SessionRegistryError::AuthorityOperation(error))
            }
        }
    }

    fn validate_entry<'a>(
        state: &'a RegistryState,
        key: &SessionKey,
        revision: RegistrySlotRevision,
    ) -> Result<&'a RegistryEntry, SessionRegistryError> {
        let entry = state
            .entries
            .get(key)
            .ok_or(SessionRegistryError::SlotMissing)?;
        if entry.slot_revision != revision {
            return Err(SessionRegistryError::RevisionMismatch);
        }
        Ok(entry)
    }

    fn validate_entry_mut<'a>(
        state: &'a mut RegistryState,
        key: &SessionKey,
        revision: RegistrySlotRevision,
    ) -> Result<&'a mut RegistryEntry, SessionRegistryError> {
        let entry = state
            .entries
            .get_mut(key)
            .ok_or(SessionRegistryError::SlotMissing)?;
        if entry.slot_revision != revision {
            return Err(SessionRegistryError::RevisionMismatch);
        }
        Ok(entry)
    }

    fn acquire_mutation_permit(
        &self,
        key: &SessionKey,
        revision: RegistrySlotRevision,
    ) -> Result<MutationPermit, SessionRegistryError> {
        let gate = {
            let state = self.lock_state()?;
            Arc::clone(&Self::validate_entry(&state, key, revision)?.mutation_gate)
        };
        gate.acquire()
    }

    fn owner_matches(owner: &IndexOwner, key: &SessionKey, revision: RegistrySlotRevision) -> bool {
        owner.key == *key && owner.slot_revision == revision
    }

    fn authority_start_error(error: SessionOperationError) -> SessionRegistryError {
        match error {
            SessionOperationError::StaleGeneration => SessionRegistryError::StaleSession,
            other => SessionRegistryError::AuthorityOperation(other),
        }
    }

    /// Ensure one exact admitted generation has a registry slot.
    pub(crate) fn register_exact(
        &self,
        key: &SessionKey,
    ) -> Result<RegistrySlotRevision, SessionRegistryError> {
        let guard = self
            .authority
            .try_operation_exact(key, SessionOperationKind::StateTransition)
            .map_err(Self::authority_start_error)?;
        let mut state = match self.lock_state() {
            Ok(state) => state,
            Err(error) => {
                guard.finish_rollback();
                return Err(error);
            }
        };
        if let Some(entry) = state.entries.get(key) {
            let slot_revision = entry.slot_revision;
            drop(state);
            return Self::finish_unmutated_operation(guard, slot_revision);
        }
        let slot_revision = match Self::allocate_revision_locked(&mut state) {
            Ok(revision) => revision,
            Err(error) => {
                drop(state);
                guard.finish_rollback();
                return Err(error);
            }
        };
        state.entries.insert(
            key.clone(),
            RegistryEntry {
                slot_revision,
                mutation_gate: Arc::new(MutationGate::default()),
                dialog_id: None,
                dialog_mutation_revision: 0,
                media_id: None,
                media_mutation_revision: 0,
                pending: PendingInboundBundle::default(),
                pending_mutation_revision: 0,
            },
        );

        // Keep the state mutex across authority finalization. A concurrent
        // registration must never observe an inserted slot before this exact
        // admission operation has either committed or removed it.
        self.fire_after_exact_mutation_hook();
        match guard.finish() {
            Ok(()) => Ok(slot_revision),
            Err(failure) => {
                let error = failure.error();
                if state
                    .entries
                    .get(key)
                    .is_some_and(|entry| entry.slot_revision == slot_revision)
                {
                    state.entries.remove(key);
                }
                drop(state);
                failure.into_guard().finish_rollback();
                Err(SessionRegistryError::AuthorityOperation(error))
            }
        }
    }

    pub(crate) fn register_handle_exact(
        &self,
        key: &SessionKey,
    ) -> Result<SessionRegistryHandle, SessionRegistryError> {
        self.register_exact(key)
            .map(|slot_revision| SessionRegistryHandle {
                key: key.clone(),
                slot_revision,
            })
    }

    pub(crate) fn map_dialog_handle(
        &self,
        handle: &SessionRegistryHandle,
        dialog_id: DialogId,
    ) -> Result<(), SessionRegistryError> {
        self.map_dialog_exact(&handle.key, handle.slot_revision, dialog_id)
    }

    /// Clear only the exact dialog mapping installed for a retained slot.
    ///
    /// Managed-resource rollback may run after authority quiesce, so this
    /// deliberately validates the registry slot revision instead of requiring
    /// the generation to remain Active. The expected Dialog-ID makes delayed
    /// rollback conditional and unable to erase a later replacement mapping.
    pub(crate) fn clear_dialog_handle_retained(
        &self,
        handle: &SessionRegistryHandle,
        expected_dialog_id: DialogId,
    ) -> Result<bool, SessionRegistryError> {
        let _mutation_permit = self.acquire_mutation_permit(&handle.key, handle.slot_revision)?;
        let mut state = self.lock_state()?;
        let current = Self::validate_entry(&state, &handle.key, handle.slot_revision)?;
        if current.dialog_id != Some(expected_dialog_id) {
            return Ok(false);
        }

        let mutation_revision = Self::allocate_mutation_revision_locked(&mut state)?;
        if state
            .by_dialog
            .get(&expected_dialog_id)
            .is_some_and(|owner| Self::owner_matches(owner, &handle.key, handle.slot_revision))
        {
            state.by_dialog.remove(&expected_dialog_id);
        }
        let entry = Self::validate_entry_mut(&mut state, &handle.key, handle.slot_revision)?;
        entry.dialog_id = None;
        entry.dialog_mutation_revision = mutation_revision;
        Ok(true)
    }

    #[allow(dead_code)]
    pub(crate) fn map_media_handle(
        &self,
        handle: &SessionRegistryHandle,
        media_id: MediaSessionId,
    ) -> Result<(), SessionRegistryError> {
        self.map_media_exact(&handle.key, handle.slot_revision, media_id)
    }

    pub(crate) fn remove_handle(
        &self,
        handle: &SessionRegistryHandle,
    ) -> Result<bool, SessionRegistryError> {
        self.remove_if(&handle.key, handle.slot_revision)
    }

    fn current_handle(
        &self,
        session_id: &SessionId,
    ) -> Result<(SessionKey, RegistrySlotRevision), SessionRegistryError> {
        let key = self
            .authority
            .current_key(session_id)
            .ok_or(SessionRegistryError::StaleSession)?;
        let revision = self.register_exact(&key)?;
        Ok((key, revision))
    }

    fn existing_current_handle(
        &self,
        session_id: &SessionId,
    ) -> Result<(SessionKey, RegistrySlotRevision), SessionRegistryError> {
        let key = self
            .authority
            .current_key(session_id)
            .ok_or(SessionRegistryError::StaleSession)?;
        let state = self.lock_state()?;
        let revision = state
            .entries
            .get(&key)
            .map(|entry| entry.slot_revision)
            .ok_or(SessionRegistryError::SlotMissing)?;
        Ok((key, revision))
    }

    pub(crate) fn map_dialog_exact(
        &self,
        key: &SessionKey,
        revision: RegistrySlotRevision,
        dialog_id: DialogId,
    ) -> Result<(), SessionRegistryError> {
        let guard = self
            .authority
            .try_operation_exact(key, SessionOperationKind::Signaling)
            .map_err(Self::authority_start_error)?;
        let mutation_permit = match self.acquire_mutation_permit(key, revision) {
            Ok(permit) => permit,
            Err(error) => {
                guard.finish_rollback();
                return Err(error);
            }
        };
        if let Err(error) = guard.ensure_current() {
            guard.finish_rollback();
            return Err(SessionRegistryError::AuthorityOperation(error));
        }
        let mutation = (|| {
            let mut state = self.lock_state()?;
            Self::validate_entry(&state, key, revision)?;
            if let Some(owner) = state.by_dialog.get(&dialog_id) {
                if !Self::owner_matches(owner, key, revision) {
                    return Err(SessionRegistryError::DialogCollision);
                }
            }
            let (previous, previous_mutation_revision) = state
                .entries
                .get(key)
                .map(|entry| (entry.dialog_id, entry.dialog_mutation_revision))
                .expect("entry was validated under the same mutex");
            let installed_revision = Self::allocate_mutation_revision_locked(&mut state)?;
            if let Some(previous) = previous.filter(|previous| *previous != dialog_id) {
                if state
                    .by_dialog
                    .get(&previous)
                    .is_some_and(|owner| Self::owner_matches(owner, key, revision))
                {
                    state.by_dialog.remove(&previous);
                }
            }
            state.by_dialog.insert(
                dialog_id,
                IndexOwner {
                    key: key.clone(),
                    slot_revision: revision,
                },
            );
            let entry = Self::validate_entry_mut(&mut state, key, revision)?;
            entry.dialog_id = Some(dialog_id);
            entry.dialog_mutation_revision = installed_revision;
            Ok((previous, previous_mutation_revision, installed_revision))
        })();
        let (previous, previous_mutation_revision, installed_revision) = match mutation {
            Ok(result) => result,
            Err(error) => {
                guard.finish_rollback();
                return Err(error);
            }
        };
        self.dialog_mapped_total.fetch_add(1, Ordering::Relaxed);
        let rollback_key = key.clone();
        let result = self.finish_exact_mutation(guard, (), move |state| {
            let current = Self::validate_entry(state, &rollback_key, revision)?;
            if current.dialog_mutation_revision != installed_revision {
                return Ok(());
            }
            if state
                .by_dialog
                .get(&dialog_id)
                .is_some_and(|owner| Self::owner_matches(owner, &rollback_key, revision))
            {
                state.by_dialog.remove(&dialog_id);
            }
            let restored = previous.filter(|previous_dialog| {
                state
                    .by_dialog
                    .get(previous_dialog)
                    .is_none_or(|owner| Self::owner_matches(owner, &rollback_key, revision))
            });
            if let Some(previous_dialog) = restored {
                state.by_dialog.insert(
                    previous_dialog,
                    IndexOwner {
                        key: rollback_key.clone(),
                        slot_revision: revision,
                    },
                );
            }
            let entry = Self::validate_entry_mut(state, &rollback_key, revision)?;
            entry.dialog_id = restored;
            entry.dialog_mutation_revision = previous_mutation_revision;
            Ok(())
        });
        drop(mutation_permit);
        result
    }

    pub(crate) fn map_media_exact(
        &self,
        key: &SessionKey,
        revision: RegistrySlotRevision,
        media_id: MediaSessionId,
    ) -> Result<(), SessionRegistryError> {
        let guard = self
            .authority
            .try_operation_exact(key, SessionOperationKind::Media)
            .map_err(Self::authority_start_error)?;
        let mutation_permit = match self.acquire_mutation_permit(key, revision) {
            Ok(permit) => permit,
            Err(error) => {
                guard.finish_rollback();
                return Err(error);
            }
        };
        if let Err(error) = guard.ensure_current() {
            guard.finish_rollback();
            return Err(SessionRegistryError::AuthorityOperation(error));
        }
        let mutation = (|| {
            let mut state = self.lock_state()?;
            Self::validate_entry(&state, key, revision)?;
            if let Some(owner) = state.by_media.get(&media_id) {
                if !Self::owner_matches(owner, key, revision) {
                    return Err(SessionRegistryError::MediaCollision);
                }
            }
            let (previous, previous_mutation_revision) = state
                .entries
                .get(key)
                .map(|entry| (entry.media_id.clone(), entry.media_mutation_revision))
                .expect("entry was validated under the same mutex");
            let installed_revision = Self::allocate_mutation_revision_locked(&mut state)?;
            if let Some(previous) = previous.as_ref().filter(|previous| *previous != &media_id) {
                if state
                    .by_media
                    .get(previous)
                    .is_some_and(|owner| Self::owner_matches(owner, key, revision))
                {
                    state.by_media.remove(previous);
                }
            }
            state.by_media.insert(
                media_id.clone(),
                IndexOwner {
                    key: key.clone(),
                    slot_revision: revision,
                },
            );
            let entry = Self::validate_entry_mut(&mut state, key, revision)?;
            entry.media_id = Some(media_id.clone());
            entry.media_mutation_revision = installed_revision;
            Ok((previous, previous_mutation_revision, installed_revision))
        })();
        let (previous, previous_mutation_revision, installed_revision) = match mutation {
            Ok(result) => result,
            Err(error) => {
                guard.finish_rollback();
                return Err(error);
            }
        };
        self.media_mapped_total.fetch_add(1, Ordering::Relaxed);
        let rollback_key = key.clone();
        let result = self.finish_exact_mutation(guard, (), move |state| {
            let current = Self::validate_entry(state, &rollback_key, revision)?;
            if current.media_mutation_revision != installed_revision {
                return Ok(());
            }
            if state
                .by_media
                .get(&media_id)
                .is_some_and(|owner| Self::owner_matches(owner, &rollback_key, revision))
            {
                state.by_media.remove(&media_id);
            }
            let restored = previous.filter(|previous_media| {
                state
                    .by_media
                    .get(previous_media)
                    .is_none_or(|owner| Self::owner_matches(owner, &rollback_key, revision))
            });
            if let Some(previous_media) = restored.as_ref() {
                state.by_media.insert(
                    previous_media.clone(),
                    IndexOwner {
                        key: rollback_key.clone(),
                        slot_revision: revision,
                    },
                );
            }
            let entry = Self::validate_entry_mut(state, &rollback_key, revision)?;
            entry.media_id = restored;
            entry.media_mutation_revision = previous_mutation_revision;
            Ok(())
        });
        drop(mutation_permit);
        result
    }

    pub(crate) fn commit_inbound_dialog_exact(
        &self,
        key: &SessionKey,
        revision: RegistrySlotRevision,
        dialog_id: DialogId,
        info: IncomingCallInfo,
    ) -> Result<InboundDialogCommitReceipt, SessionRegistryError> {
        let guard = self
            .authority
            .try_operation_exact(key, SessionOperationKind::StateTransition)
            .map_err(Self::authority_start_error)?;
        let mutation_permit = match self.acquire_mutation_permit(key, revision) {
            Ok(permit) => permit,
            Err(error) => {
                guard.finish_rollback();
                return Err(error);
            }
        };
        let mutation = (|| {
            guard
                .ensure_current()
                .map_err(SessionRegistryError::AuthorityOperation)?;
            let mut state = self.lock_state()?;
            Self::validate_entry(&state, key, revision)?;
            if let Some(owner) = state.by_dialog.get(&dialog_id) {
                if !Self::owner_matches(owner, key, revision) {
                    return Err(SessionRegistryError::DialogCollision);
                }
            }
            let (
                previous_dialog,
                previous_dialog_mutation_revision,
                previous_info,
                previous_pending_mutation_revision,
            ) = state
                .entries
                .get(key)
                .map(|entry| {
                    (
                        entry.dialog_id,
                        entry.dialog_mutation_revision,
                        entry.pending.info.clone(),
                        entry.pending_mutation_revision,
                    )
                })
                .expect("entry was validated under the same mutex");
            let installed_mutation_revision = Self::allocate_mutation_revision_locked(&mut state)?;
            if let Some(previous) = previous_dialog.filter(|previous| *previous != dialog_id) {
                if state
                    .by_dialog
                    .get(&previous)
                    .is_some_and(|owner| Self::owner_matches(owner, key, revision))
                {
                    state.by_dialog.remove(&previous);
                }
            }
            state.by_dialog.insert(
                dialog_id,
                IndexOwner {
                    key: key.clone(),
                    slot_revision: revision,
                },
            );
            let entry = Self::validate_entry_mut(&mut state, key, revision)?;
            entry.dialog_id = Some(dialog_id);
            entry.dialog_mutation_revision = installed_mutation_revision;
            entry.pending.info = Some(info);
            entry.pending_mutation_revision = installed_mutation_revision;
            self.dialog_mapped_total.fetch_add(1, Ordering::Relaxed);
            Ok((
                previous_dialog,
                previous_dialog_mutation_revision,
                previous_info,
                previous_pending_mutation_revision,
                installed_mutation_revision,
            ))
        })();
        let (
            previous_dialog,
            previous_dialog_mutation_revision,
            previous_info,
            previous_pending_mutation_revision,
            installed_mutation_revision,
        ) = match mutation {
            Ok(previous) => previous,
            Err(error) => {
                guard.finish_rollback();
                return Err(error);
            }
        };
        Ok(InboundDialogCommitReceipt {
            registry: self.clone(),
            key: key.clone(),
            revision,
            installed_dialog: dialog_id,
            installed_mutation_revision,
            previous_dialog,
            previous_dialog_mutation_revision,
            previous_info,
            previous_pending_mutation_revision,
            guard: Some(guard),
            _mutation_permit: Some(mutation_permit),
            armed: true,
        })
    }

    // The flat argument list is the exact immutable rollback snapshot captured
    // before the transactional registry mutation.
    #[allow(clippy::too_many_arguments)]
    fn rollback_inbound_commit(
        &self,
        key: &SessionKey,
        revision: RegistrySlotRevision,
        installed_dialog: DialogId,
        installed_mutation_revision: u64,
        previous_dialog: Option<DialogId>,
        previous_dialog_mutation_revision: u64,
        previous_info: Option<IncomingCallInfo>,
        previous_pending_mutation_revision: u64,
    ) -> Result<bool, SessionRegistryError> {
        let mut state = self.lock_state()?;
        let entry = Self::validate_entry(&state, key, revision)?;
        let rollback_dialog = entry.dialog_mutation_revision == installed_mutation_revision;
        let rollback_pending = entry.pending_mutation_revision == installed_mutation_revision;
        if !rollback_dialog && !rollback_pending {
            return Ok(false);
        }
        let restored_dialog = if rollback_dialog {
            if state
                .by_dialog
                .get(&installed_dialog)
                .is_some_and(|owner| Self::owner_matches(owner, key, revision))
            {
                state.by_dialog.remove(&installed_dialog);
            }
            let restored =
                previous_dialog.filter(|dialog_id| match state.by_dialog.get(dialog_id) {
                    Some(owner) => Self::owner_matches(owner, key, revision),
                    None => true,
                });
            if let Some(dialog_id) = restored {
                state.by_dialog.insert(
                    dialog_id,
                    IndexOwner {
                        key: key.clone(),
                        slot_revision: revision,
                    },
                );
            }
            restored
        } else {
            None
        };
        let entry = Self::validate_entry_mut(&mut state, key, revision)?;
        if rollback_dialog {
            entry.dialog_id = restored_dialog;
            entry.dialog_mutation_revision = previous_dialog_mutation_revision;
        }
        if rollback_pending {
            entry.pending.info = previous_info;
            entry.pending_mutation_revision = previous_pending_mutation_revision;
        }
        Ok(true)
    }

    pub(crate) fn store_pending_bundle_exact(
        &self,
        key: &SessionKey,
        revision: RegistrySlotRevision,
        pending: PendingInboundBundle,
    ) -> Result<(), SessionRegistryError> {
        self.mutate_pending_exact(key, revision, move |current| *current = pending)
    }

    fn mutate_pending_exact<R>(
        &self,
        key: &SessionKey,
        revision: RegistrySlotRevision,
        mutate: impl FnOnce(&mut PendingInboundBundle) -> R,
    ) -> Result<R, SessionRegistryError> {
        let guard = self
            .authority
            .try_operation_exact(key, SessionOperationKind::EventDispatch)
            .map_err(Self::authority_start_error)?;
        let mutation_permit = match self.acquire_mutation_permit(key, revision) {
            Ok(permit) => permit,
            Err(error) => {
                guard.finish_rollback();
                return Err(error);
            }
        };
        if let Err(error) = guard.ensure_current() {
            guard.finish_rollback();
            return Err(SessionRegistryError::AuthorityOperation(error));
        }
        let mutation = (|| {
            let mut state = self.lock_state()?;
            let entry = Self::validate_entry(&state, key, revision)?;
            let previous = entry.pending.clone();
            let previous_mutation_revision = entry.pending_mutation_revision;
            let installed_revision = Self::allocate_mutation_revision_locked(&mut state)?;
            let entry = Self::validate_entry_mut(&mut state, key, revision)?;
            let value = mutate(&mut entry.pending);
            entry.pending_mutation_revision = installed_revision;
            Ok((
                value,
                previous,
                previous_mutation_revision,
                installed_revision,
            ))
        })();
        let (value, previous, previous_mutation_revision, installed_revision) = match mutation {
            Ok(result) => result,
            Err(error) => {
                guard.finish_rollback();
                return Err(error);
            }
        };
        let rollback_key = key.clone();
        let result = self.finish_exact_mutation(guard, value, move |state| {
            let current = Self::validate_entry(state, &rollback_key, revision)?;
            if current.pending_mutation_revision != installed_revision {
                return Ok(());
            }
            let entry = Self::validate_entry_mut(state, &rollback_key, revision)?;
            entry.pending = previous;
            entry.pending_mutation_revision = previous_mutation_revision;
            Ok(())
        });
        drop(mutation_permit);
        result
    }

    pub(crate) fn pending_bundle_exact(
        &self,
        key: &SessionKey,
        revision: RegistrySlotRevision,
    ) -> Result<PendingInboundBundle, SessionRegistryError> {
        if !self.authority.is_current(key) {
            return Err(SessionRegistryError::StaleSession);
        }
        let state = self.lock_state()?;
        Ok(Self::validate_entry(&state, key, revision)?.pending.clone())
    }

    pub(crate) fn get_key_by_dialog_exact(&self, dialog_id: &DialogId) -> Option<SessionKey> {
        let owner = self.lock_state().ok()?.by_dialog.get(dialog_id).cloned()?;
        if !self.authority.is_current(&owner.key) {
            return None;
        }
        let state = self.lock_state().ok()?;
        let entry = state.entries.get(&owner.key)?;
        (entry.slot_revision == owner.slot_revision && entry.dialog_id.as_ref() == Some(dialog_id))
            .then_some(owner.key)
    }

    pub(crate) fn get_key_by_media_exact(&self, media_id: &MediaSessionId) -> Option<SessionKey> {
        let owner = self.lock_state().ok()?.by_media.get(media_id).cloned()?;
        if !self.authority.is_current(&owner.key) {
            return None;
        }
        let state = self.lock_state().ok()?;
        let entry = state.entries.get(&owner.key)?;
        (entry.slot_revision == owner.slot_revision && entry.media_id.as_ref() == Some(media_id))
            .then_some(owner.key)
    }

    pub(crate) fn get_dialog_exact(
        &self,
        key: &SessionKey,
        revision: RegistrySlotRevision,
    ) -> Option<DialogId> {
        if !self.authority.is_current(key) {
            return None;
        }
        let state = self.lock_state().ok()?;
        Self::validate_entry(&state, key, revision).ok()?.dialog_id
    }

    pub(crate) fn get_media_exact(
        &self,
        key: &SessionKey,
        revision: RegistrySlotRevision,
    ) -> Option<MediaSessionId> {
        if !self.authority.is_current(key) {
            return None;
        }
        let state = self.lock_state().ok()?;
        Self::validate_entry(&state, key, revision)
            .ok()?
            .media_id
            .clone()
    }

    /// Remove only the exact slot observed by its caller. This is valid after
    /// authority quiesce, so it intentionally does not require `is_current`.
    pub(crate) fn remove_if(
        &self,
        key: &SessionKey,
        revision: RegistrySlotRevision,
    ) -> Result<bool, SessionRegistryError> {
        let mutation_gate = {
            let state = self.lock_state()?;
            let Some(entry) = state.entries.get(key) else {
                self.remove_missing_total.fetch_add(1, Ordering::Relaxed);
                return Ok(false);
            };
            if entry.slot_revision != revision {
                self.remove_missing_total.fetch_add(1, Ordering::Relaxed);
                return Ok(false);
            }
            Arc::clone(&entry.mutation_gate)
        };
        let _mutation_permit = mutation_gate.acquire()?;
        let mut state = self.lock_state()?;
        let Some(entry) = state.entries.get(key) else {
            self.remove_missing_total.fetch_add(1, Ordering::Relaxed);
            return Ok(false);
        };
        if entry.slot_revision != revision {
            self.remove_missing_total.fetch_add(1, Ordering::Relaxed);
            return Ok(false);
        }
        let entry = state
            .entries
            .remove(key)
            .expect("entry was checked under the same mutex");
        if let Some(dialog_id) = entry.dialog_id {
            if state
                .by_dialog
                .get(&dialog_id)
                .is_some_and(|owner| Self::owner_matches(owner, key, revision))
            {
                state.by_dialog.remove(&dialog_id);
            }
        }
        if let Some(media_id) = entry.media_id {
            if state
                .by_media
                .get(&media_id)
                .is_some_and(|owner| Self::owner_matches(owner, key, revision))
            {
                state.by_media.remove(&media_id);
            }
        }
        self.removed_total.fetch_add(1, Ordering::Relaxed);
        Ok(true)
    }

    fn unique_active_handle(&self) -> Option<(SessionKey, RegistrySlotRevision)> {
        let candidates: Vec<_> = self
            .lock_state()
            .ok()?
            .entries
            .iter()
            .map(|(key, entry)| (key.clone(), entry.slot_revision))
            .collect();
        let mut active = candidates
            .into_iter()
            .filter(|(key, _)| self.authority.is_current(key));
        let only = active.next()?;
        active.next().is_none().then_some(only)
    }

    // --- Compatibility surface. Raw identifiers resolve through the shared
    // authority and therefore fail closed after quiesce or generation reuse.

    pub async fn map_dialog(&self, session_id: SessionId, dialog_id: DialogId) {
        if !self.map_dialog_checked(session_id, dialog_id).await {
            tracing::warn!("session registry rejected non-current dialog mapping");
        }
    }

    pub(crate) async fn map_dialog_checked(
        &self,
        session_id: SessionId,
        dialog_id: DialogId,
    ) -> bool {
        let Ok((key, revision)) = self.current_handle(&session_id) else {
            return false;
        };
        self.map_dialog_exact(&key, revision, dialog_id).is_ok()
    }

    /// Legacy source-compatible external-map commit.
    ///
    /// Because the compensator is produced *after* the callback returns, a
    /// callback panic cannot compensate partial external mutation. Production
    /// authority paths must use [`Self::commit_inbound_dialog_compensated`],
    /// which arms the exact compensator before invoking external code.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn commit_inbound_dialog<F, C>(
        &self,
        session_id: SessionId,
        dialog_id: DialogId,
        info: IncomingCallInfo,
        commit_external_maps: F,
    ) -> bool
    where
        F: FnOnce() -> C,
        C: ExternalMapCompensation,
    {
        let Ok((key, revision)) = self.current_handle(&session_id) else {
            return false;
        };
        let Ok(receipt) = self.commit_inbound_dialog_exact(&key, revision, dialog_id, info) else {
            return false;
        };
        match catch_unwind(AssertUnwindSafe(commit_external_maps)) {
            Ok(compensation) => match receipt.finalize_with_compensation(compensation) {
                Ok(()) => true,
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "session registry authority rejected external inbound-map finalize"
                    );
                    false
                }
            },
            Err(_) => {
                drop(receipt);
                tracing::error!(
                    "session registry external inbound-map commit panicked; exact registry mutation was compensated"
                );
                false
            }
        }
    }

    /// Commit an inbound dialog with a pre-armed exact external compensator.
    ///
    /// `compensation` must remove only the external values installed by
    /// `commit_external_maps`. It runs on callback panic and whenever
    /// authority commit loses to quiesce, while the authority operation guard
    /// is still retained.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn commit_inbound_dialog_compensated<F, C>(
        &self,
        session_id: SessionId,
        dialog_id: DialogId,
        info: IncomingCallInfo,
        commit_external_maps: F,
        compensation: C,
    ) -> bool
    where
        F: FnOnce(),
        C: ExternalMapCompensation,
    {
        let Ok((key, revision)) = self.current_handle(&session_id) else {
            return false;
        };
        let Ok(receipt) = self.commit_inbound_dialog_exact(&key, revision, dialog_id, info) else {
            return false;
        };
        match catch_unwind(AssertUnwindSafe(commit_external_maps)) {
            Ok(()) => match receipt.finalize_with_compensation(compensation) {
                Ok(()) => true,
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "session registry authority rejected compensated inbound-map finalize"
                    );
                    false
                }
            },
            Err(_) => {
                if let Err(error) = receipt.rollback_with_compensation(compensation) {
                    tracing::error!(
                        error = %error,
                        "session registry failed to compensate panicked external inbound-map commit"
                    );
                }
                false
            }
        }
    }

    #[allow(dead_code)]
    pub(crate) fn with_live_session<R, C>(
        &self,
        session_id: &SessionId,
        commit: impl FnOnce() -> (R, C),
    ) -> Option<R>
    where
        C: ExternalMapCompensation,
    {
        let (key, revision) = self.existing_current_handle(session_id).ok()?;
        let guard = self
            .authority
            .try_operation_exact(&key, SessionOperationKind::StateTransition)
            .ok()?;
        {
            let Ok(state) = self.lock_state() else {
                guard.finish_rollback();
                return None;
            };
            if Self::validate_entry(&state, &key, revision).is_err() {
                drop(state);
                guard.finish_rollback();
                return None;
            }
        }
        match catch_unwind(AssertUnwindSafe(commit)) {
            Ok((value, compensation)) => match guard.finish() {
                Ok(()) => Some(value),
                Err(failure) => {
                    let _ = catch_unwind(AssertUnwindSafe(|| compensation.compensate()));
                    failure.into_guard().finish_rollback();
                    None
                }
            },
            Err(_) => {
                guard.finish_rollback();
                None
            }
        }
    }

    pub async fn map_media(&self, session_id: SessionId, media_id: MediaSessionId) {
        if !self.map_media_checked(session_id, media_id).await {
            tracing::warn!("session registry rejected non-current media mapping");
        }
    }

    pub(crate) async fn map_media_checked(
        &self,
        session_id: SessionId,
        media_id: MediaSessionId,
    ) -> bool {
        let Ok((key, revision)) = self.current_handle(&session_id) else {
            return false;
        };
        self.map_media_exact(&key, revision, media_id).is_ok()
    }

    pub async fn get_session_by_dialog(&self, dialog_id: &DialogId) -> Option<SessionId> {
        self.get_key_by_dialog_exact(dialog_id)
            .map(|key| key.session_id)
    }

    pub async fn get_session_by_media(&self, media_id: &MediaSessionId) -> Option<SessionId> {
        self.get_key_by_media_exact(media_id)
            .map(|key| key.session_id)
    }

    pub async fn get_dialog_by_session(&self, session_id: &SessionId) -> Option<DialogId> {
        let (key, revision) = self.existing_current_handle(session_id).ok()?;
        self.get_dialog_exact(&key, revision)
    }

    pub async fn get_media_by_session(&self, session_id: &SessionId) -> Option<MediaSessionId> {
        let (key, revision) = self.existing_current_handle(session_id).ok()?;
        self.get_media_exact(&key, revision)
    }

    pub async fn remove_session(&self, session_id: &SessionId) {
        let Ok((key, revision)) = self.existing_current_handle(session_id) else {
            self.remove_missing_total.fetch_add(1, Ordering::Relaxed);
            return;
        };
        let _ = self.remove_if(&key, revision);
    }

    /// Transitional compatibility wrapper. Retirement now belongs solely to
    /// `SessionLeaseAuthority`; this method only conditionally removes the
    /// current registry slot and reports whether it owned that removal.
    #[allow(dead_code)]
    pub(crate) fn retire_session_for_cleanup(&self, session_id: &SessionId) -> Option<bool> {
        match self.existing_current_handle(session_id) {
            Ok((key, revision)) => self.remove_if(&key, revision).ok(),
            Err(SessionRegistryError::Poisoned) => None,
            Err(_) => Some(false),
        }
    }

    /// No-op compatibility wrapper. Cleanup ownership moved to the authority.
    #[allow(dead_code)]
    pub(crate) fn release_cleanup_retirement(&self, _session_id: &SessionId) {}

    /// The registry has no retirement records by design.
    #[allow(dead_code)]
    pub(crate) fn retired_session_count(&self) -> usize {
        0
    }

    #[cfg(feature = "perf-tests")]
    pub(crate) fn perf_lifecycle_counts(&self) -> serde_json::Value {
        serde_json::json!({
            "dialog_mapped_total": self.dialog_mapped_total.load(Ordering::Relaxed),
            "media_mapped_total": self.media_mapped_total.load(Ordering::Relaxed),
            "removed_total": self.removed_total.load(Ordering::Relaxed),
            "remove_missing_total": self.remove_missing_total.load(Ordering::Relaxed),
        })
    }

    pub async fn contains_session(&self, session_id: &SessionId) -> bool {
        self.existing_current_handle(session_id).is_ok()
    }

    pub async fn session_count(&self) -> usize {
        let Ok(state) = self.lock_state() else {
            return 0;
        };
        let keys: Vec<_> = state.entries.keys().cloned().collect();
        drop(state);
        keys.iter()
            .filter(|key| self.authority.is_current(key))
            .count()
    }

    pub async fn clear(&self) {
        let Ok(mut state) = self.lock_state() else {
            return;
        };
        let removed = state.entries.len() as u64;
        state.entries.clear();
        state.by_dialog.clear();
        state.by_media.clear();
        self.removed_total.fetch_add(removed, Ordering::Relaxed);
    }

    pub async fn store_pending_incoming_call(
        &self,
        session_id: SessionId,
        info: IncomingCallInfo,
    ) -> bool {
        let Ok((key, revision)) = self.existing_current_handle(&session_id) else {
            return false;
        };
        self.mutate_pending_exact(&key, revision, move |pending| pending.info = Some(info))
            .is_ok()
    }

    pub async fn take_pending_incoming_call(
        &self,
        session_id: &SessionId,
    ) -> Option<IncomingCallInfo> {
        let (key, revision) = self.existing_current_handle(session_id).ok()?;
        self.mutate_pending_exact(&key, revision, |pending| pending.info.take())
            .ok()
            .flatten()
    }

    pub async fn store_pending_incoming_request(
        &self,
        session_id: &SessionId,
        request: Arc<rvoip_sip_core::Request>,
    ) -> bool {
        let Ok((key, revision)) = self.existing_current_handle(session_id) else {
            return false;
        };
        self.mutate_pending_exact(&key, revision, move |pending| {
            pending.request = Some(request)
        })
        .is_ok()
    }

    pub async fn store_pending_incoming_transport(
        &self,
        session_id: &SessionId,
        transport: SipTransportSecurityContext,
    ) -> bool {
        let Ok((key, revision)) = self.existing_current_handle(session_id) else {
            return false;
        };
        self.mutate_pending_exact(&key, revision, move |pending| {
            pending.transport = Some(Arc::new(transport))
        })
        .is_ok()
    }

    #[allow(dead_code)]
    pub(crate) async fn store_pending_incoming_principal(
        &self,
        session_id: &SessionId,
        principal: AuthenticatedPrincipal,
    ) -> bool {
        let Ok((key, revision)) = self.existing_current_handle(session_id) else {
            return false;
        };
        self.mutate_pending_exact(&key, revision, move |pending| {
            pending.principal = Some(principal)
        })
        .is_ok()
    }

    pub async fn peek_pending_incoming_request(&self) -> Option<Arc<rvoip_sip_core::Request>> {
        let (key, revision) = self.unique_active_handle()?;
        self.pending_bundle_exact(&key, revision).ok()?.request
    }

    pub async fn peek_pending_incoming_transport(
        &self,
    ) -> Option<Arc<SipTransportSecurityContext>> {
        let (key, revision) = self.unique_active_handle()?;
        self.pending_bundle_exact(&key, revision).ok()?.transport
    }

    #[allow(dead_code)]
    pub(crate) async fn peek_pending_incoming_principal(&self) -> Option<AuthenticatedPrincipal> {
        let (key, revision) = self.unique_active_handle()?;
        self.pending_bundle_exact(&key, revision).ok()?.principal
    }

    pub async fn take_pending_incoming_request(&self) -> Option<Arc<rvoip_sip_core::Request>> {
        let (key, revision) = self.unique_active_handle()?;
        self.mutate_pending_exact(&key, revision, |pending| pending.request.take())
            .ok()
            .flatten()
    }

    pub async fn take_pending_incoming_transport(
        &self,
    ) -> Option<Arc<SipTransportSecurityContext>> {
        let (key, revision) = self.unique_active_handle()?;
        self.mutate_pending_exact(&key, revision, |pending| pending.transport.take())
            .ok()
            .flatten()
    }

    #[allow(dead_code)]
    pub(crate) async fn take_pending_incoming_principal(&self) -> Option<AuthenticatedPrincipal> {
        let (key, revision) = self.unique_active_handle()?;
        self.mutate_pending_exact(&key, revision, |pending| pending.principal.take())
            .ok()
            .flatten()
    }
}

impl Default for SessionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use rvoip_core_traits::identity::{AuthenticationMethod, IdentityAssurance};
    use rvoip_sip_core::{Method, Request, Uri};

    use super::*;

    struct Admitted {
        authority: Arc<SessionLeaseAuthority>,
        registry: SessionRegistry,
        _leases: Vec<crate::session_lifecycle::SessionLease>,
    }

    impl Admitted {
        fn sessions(ids: &[&str]) -> (Self, Vec<SessionKey>) {
            let authority = SessionLeaseAuthority::new();
            let registry = SessionRegistry::with_authority(Arc::clone(&authority));
            let leases: Vec<_> = ids
                .iter()
                .map(|id| authority.admit(SessionId::from(*id)).expect("admission"))
                .collect();
            let keys = leases.iter().map(|lease| lease.key().clone()).collect();
            (
                Self {
                    authority,
                    registry,
                    _leases: leases,
                },
                keys,
            )
        }
    }

    fn incoming(session_id: SessionId, dialog_id: DialogId, marker: &str) -> IncomingCallInfo {
        IncomingCallInfo {
            session_id,
            dialog_id,
            from: format!("sip:{marker}@example.test"),
            to: "sip:bridge@example.test".to_string(),
            call_id: format!("{marker}@example.test"),
            p_asserted_identity: None,
        }
    }

    fn request(marker: &str) -> Arc<Request> {
        let uri = Uri::from_str(&format!("sip:{marker}@example.test")).expect("URI");
        Arc::new(Request::new(Method::Invite, uri))
    }

    fn principal(marker: &str) -> AuthenticatedPrincipal {
        AuthenticatedPrincipal {
            subject: marker.to_string(),
            tenant: Some(format!("tenant-{marker}")),
            scopes: vec!["call:attach".to_string()],
            issuer: Some("registry-test".to_string()),
            expires_at: None,
            method: AuthenticationMethod::Bearer,
            assurance: IdentityAssurance::Anonymous,
        }
    }

    #[tokio::test]
    async fn two_interleaved_calls_remain_isolated() {
        let (fixture, keys) = Admitted::sessions(&["call-a", "call-b"]);
        let registry = &fixture.registry;
        let revision_a = registry.register_exact(&keys[0]).expect("slot A");
        let revision_b = registry.register_exact(&keys[1]).expect("slot B");
        let dialog_a = DialogId::new();
        let dialog_b = DialogId::new();
        let media_a = MediaSessionId::new_v4();
        let media_b = MediaSessionId::new_v4();

        registry
            .map_dialog_exact(&keys[0], revision_a, dialog_a)
            .expect("dialog A");
        registry
            .map_media_exact(&keys[1], revision_b, media_b.clone())
            .expect("media B");
        registry
            .map_dialog_exact(&keys[1], revision_b, dialog_b)
            .expect("dialog B");
        registry
            .map_media_exact(&keys[0], revision_a, media_a.clone())
            .expect("media A");

        assert_eq!(
            registry.get_key_by_dialog_exact(&dialog_a),
            Some(keys[0].clone())
        );
        assert_eq!(
            registry.get_key_by_dialog_exact(&dialog_b),
            Some(keys[1].clone())
        );
        assert_eq!(
            registry.get_key_by_media_exact(&media_a),
            Some(keys[0].clone())
        );
        assert_eq!(
            registry.get_key_by_media_exact(&media_b),
            Some(keys[1].clone())
        );
    }

    #[test]
    fn retained_dialog_clear_is_exact_and_preserves_a_replacement() {
        let (fixture, keys) = Admitted::sessions(&["clear-exact"]);
        let registry = &fixture.registry;
        let revision = registry.register_exact(&keys[0]).expect("slot");
        let handle = SessionRegistryHandle {
            key: keys[0].clone(),
            slot_revision: revision,
        };
        let first = DialogId::new();
        let replacement = DialogId::new();

        registry
            .map_dialog_handle(&handle, first)
            .expect("first dialog");
        assert!(registry
            .clear_dialog_handle_retained(&handle, first)
            .expect("exact clear"));
        assert_eq!(registry.get_dialog_exact(&keys[0], revision), None);
        assert!(!registry
            .clear_dialog_handle_retained(&handle, first)
            .expect("idempotent clear"));

        registry
            .map_dialog_handle(&handle, replacement)
            .expect("replacement dialog");
        assert!(!registry
            .clear_dialog_handle_retained(&handle, first)
            .expect("stale clear"));
        assert_eq!(
            registry.get_dialog_exact(&keys[0], revision),
            Some(replacement)
        );
    }

    #[tokio::test]
    async fn removing_a_leaves_b_intact() {
        let (fixture, keys) = Admitted::sessions(&["remove-a", "remove-b"]);
        let registry = &fixture.registry;
        let revision_a = registry.register_exact(&keys[0]).expect("slot A");
        let revision_b = registry.register_exact(&keys[1]).expect("slot B");
        let dialog_a = DialogId::new();
        let dialog_b = DialogId::new();
        registry
            .map_dialog_exact(&keys[0], revision_a, dialog_a)
            .expect("dialog A");
        registry
            .map_dialog_exact(&keys[1], revision_b, dialog_b)
            .expect("dialog B");

        assert!(registry.remove_if(&keys[0], revision_a).expect("remove A"));
        assert_eq!(registry.get_key_by_dialog_exact(&dialog_a), None);
        assert_eq!(
            registry.get_key_by_dialog_exact(&dialog_b),
            Some(keys[1].clone())
        );
        assert_eq!(registry.session_count().await, 1);
    }

    #[tokio::test]
    async fn no_arg_pending_wrappers_fail_closed_with_two_active_entries() {
        let (fixture, keys) = Admitted::sessions(&["pending-a", "pending-b"]);
        let registry = &fixture.registry;
        for (index, key) in keys.iter().enumerate() {
            let revision = registry.register_exact(key).expect("slot");
            registry
                .store_pending_bundle_exact(
                    key,
                    revision,
                    PendingInboundBundle {
                        request: Some(request(&format!("pending-{index}"))),
                        ..PendingInboundBundle::default()
                    },
                )
                .expect("pending bundle");
        }

        assert!(registry.peek_pending_incoming_request().await.is_none());
        assert!(registry.take_pending_incoming_request().await.is_none());
    }

    #[tokio::test]
    async fn no_arg_pending_wrappers_fail_closed_once_authority_quiesces() {
        let (fixture, keys) = Admitted::sessions(&["pending-quiesce"]);
        let registry = &fixture.registry;
        let revision = registry.register_exact(&keys[0]).expect("slot");
        registry
            .store_pending_bundle_exact(
                &keys[0],
                revision,
                PendingInboundBundle {
                    request: Some(request("pending-quiesce")),
                    ..PendingInboundBundle::default()
                },
            )
            .expect("pending bundle");
        let teardown = fixture
            .authority
            .teardown(&keys[0], std::time::Duration::from_secs(1))
            .expect("begin quiesce");

        assert!(!fixture.authority.is_current(&keys[0]));
        assert!(registry.peek_pending_incoming_request().await.is_none());
        assert!(registry.take_pending_incoming_request().await.is_none());
        teardown.wait().await.expect("finish teardown");
    }

    #[tokio::test]
    async fn compatibility_external_commit_is_reentrant_and_panic_safe() {
        let (fixture, keys) = Admitted::sessions(&["commit-callback"]);
        let registry = &fixture.registry;
        let key = &keys[0];
        let revision = registry.register_exact(key).expect("slot");
        let first_dialog = DialogId::new();
        let first_info = incoming(key.session_id.clone(), first_dialog, "first");
        let callback_registry = registry.clone();
        let callback_key = key.clone();
        assert!(registry.commit_inbound_dialog(
            key.session_id.clone(),
            first_dialog,
            first_info,
            move || {
                assert_eq!(
                    callback_registry.get_key_by_dialog_exact(&first_dialog),
                    Some(callback_key)
                );
            },
        ));

        let panic_dialog = DialogId::new();
        let panic_info = incoming(key.session_id.clone(), panic_dialog, "panic");
        assert!(!registry.commit_inbound_dialog(
            key.session_id.clone(),
            panic_dialog,
            panic_info,
            || -> () { panic!("external mapping panic") },
        ));
        assert_eq!(registry.get_dialog_exact(key, revision), Some(first_dialog));
        assert_eq!(
            registry.get_key_by_dialog_exact(&panic_dialog),
            None,
            "panicked external commit must compensate its registry mapping"
        );
        registry
            .map_dialog_exact(key, revision, DialogId::new())
            .expect("registry mutex remains usable");

        let published = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let publish_for_commit = Arc::clone(&published);
        let publish_for_rollback = Arc::clone(&published);
        let compensated_dialog = DialogId::new();
        assert!(!registry.commit_inbound_dialog_compensated(
            key.session_id.clone(),
            compensated_dialog,
            incoming(
                key.session_id.clone(),
                compensated_dialog,
                "compensated-panic",
            ),
            move || {
                publish_for_commit.store(true, std::sync::atomic::Ordering::SeqCst);
                panic!("panic after external publication");
            },
            move || {
                publish_for_rollback.store(false, std::sync::atomic::Ordering::SeqCst);
            },
        ));
        assert!(
            !published.load(std::sync::atomic::Ordering::SeqCst),
            "pre-armed compensation must remove publication after callback panic"
        );
        assert_eq!(registry.get_key_by_dialog_exact(&compensated_dialog), None);
    }

    #[tokio::test]
    async fn quiesce_during_external_commit_compensates_publication() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let (fixture, keys) = Admitted::sessions(&["commit-quiesce"]);
        let registry = &fixture.registry;
        let key = &keys[0];
        let revision = registry.register_exact(key).expect("slot");
        let dialog_id = DialogId::new();
        let info = incoming(key.session_id.clone(), dialog_id, "quiesce");
        let published = Arc::new(AtomicBool::new(false));
        let publish_for_commit = Arc::clone(&published);
        let publish_for_rollback = Arc::clone(&published);
        let authority = Arc::clone(&fixture.authority);
        let teardown_key = key.clone();
        let teardown_slot = Arc::new(StdMutex::new(None));
        let teardown_slot_for_commit = Arc::clone(&teardown_slot);

        assert!(!registry.commit_inbound_dialog_compensated(
            key.session_id.clone(),
            dialog_id,
            info,
            move || {
                publish_for_commit.store(true, Ordering::SeqCst);
                let teardown = authority
                    .teardown(&teardown_key, std::time::Duration::from_secs(1))
                    .expect("quiesce during external commit");
                *teardown_slot_for_commit.lock().expect("teardown slot") = Some(teardown);
            },
            move || publish_for_rollback.store(false, Ordering::SeqCst),
        ));
        assert!(
            !published.load(Ordering::SeqCst),
            "external publication must be compensated when authority finalize loses to quiesce"
        );
        {
            let state = registry.lock_state().expect("registry state");
            assert_eq!(
                state.entries.get(key).and_then(|entry| entry.dialog_id),
                None,
                "registry mutation must be compensated with the external publication"
            );
        }
        let teardown = teardown_slot
            .lock()
            .expect("teardown slot")
            .take()
            .expect("teardown waiter");
        teardown.wait().await.expect("finish teardown");
        assert!(registry
            .remove_if(key, revision)
            .expect("exact cleanup remains possible"));
    }

    #[tokio::test]
    async fn registration_rolls_back_when_quiesce_wins_after_the_exact_mutation() {
        let (fixture, keys) = Admitted::sessions(&["register-quiesce"]);
        let registry = &fixture.registry;
        let key = &keys[0];
        let authority = Arc::clone(&fixture.authority);
        let teardown_key = key.clone();
        let teardown_slot = Arc::new(StdMutex::new(None));
        let teardown_slot_for_hook = Arc::clone(&teardown_slot);
        registry.set_after_exact_mutation_hook(Arc::new(move || {
            let waiter = authority
                .teardown(&teardown_key, std::time::Duration::from_secs(1))
                .expect("quiesce after slot insertion");
            *teardown_slot_for_hook.lock().expect("teardown slot") = Some(waiter);
        }));

        assert!(matches!(
            registry.register_exact(key),
            Err(SessionRegistryError::AuthorityOperation(_))
        ));
        assert!(
            !registry
                .lock_state()
                .expect("registry state")
                .entries
                .contains_key(key),
            "a slot inserted after the last authority check must be removed when quiesce wins"
        );
        let teardown = teardown_slot
            .lock()
            .expect("teardown slot")
            .take()
            .expect("teardown waiter");
        teardown.wait().await.expect("finish teardown");
    }

    #[tokio::test]
    async fn dialog_mapping_rolls_back_when_quiesce_wins_after_the_exact_mutation() {
        let (fixture, keys) = Admitted::sessions(&["dialog-quiesce"]);
        let registry = &fixture.registry;
        let key = &keys[0];
        let revision = registry.register_exact(key).expect("slot");
        let dialog_id = DialogId::new();
        let authority = Arc::clone(&fixture.authority);
        let teardown_key = key.clone();
        let teardown_slot = Arc::new(StdMutex::new(None));
        let teardown_slot_for_hook = Arc::clone(&teardown_slot);
        registry.set_after_exact_mutation_hook(Arc::new(move || {
            let waiter = authority
                .teardown(&teardown_key, std::time::Duration::from_secs(1))
                .expect("quiesce after dialog mapping");
            *teardown_slot_for_hook.lock().expect("teardown slot") = Some(waiter);
        }));

        assert!(matches!(
            registry.map_dialog_exact(key, revision, dialog_id),
            Err(SessionRegistryError::AuthorityOperation(_))
        ));
        {
            let state = registry.lock_state().expect("registry state");
            assert_eq!(
                state.entries.get(key).and_then(|entry| entry.dialog_id),
                None,
                "the slot must restore its prior dialog value"
            );
            assert!(
                !state.by_dialog.contains_key(&dialog_id),
                "the exact dialog index publication must also be removed"
            );
        }
        let teardown = teardown_slot
            .lock()
            .expect("teardown slot")
            .take()
            .expect("teardown waiter");
        teardown.wait().await.expect("finish teardown");
        assert!(registry
            .remove_if(key, revision)
            .expect("exact cleanup remains possible"));
    }

    #[tokio::test]
    async fn secondary_collisions_reject_and_same_slot_can_reassign() {
        let (fixture, keys) = Admitted::sessions(&["collision-a", "collision-b"]);
        let registry = &fixture.registry;
        let revision_a = registry.register_exact(&keys[0]).expect("slot A");
        let revision_b = registry.register_exact(&keys[1]).expect("slot B");
        let first = DialogId::new();
        let replacement = DialogId::new();
        let first_media = MediaSessionId::new_v4();
        let replacement_media = MediaSessionId::new_v4();
        assert_eq!(
            registry.map_dialog_exact(&keys[0], revision_b, DialogId::new()),
            Err(SessionRegistryError::RevisionMismatch)
        );
        assert!(!registry
            .remove_if(&keys[0], revision_b)
            .expect("wrong-revision removal is harmless"));
        registry
            .map_dialog_exact(&keys[0], revision_a, first)
            .expect("first mapping");
        assert_eq!(
            registry.map_dialog_exact(&keys[1], revision_b, first),
            Err(SessionRegistryError::DialogCollision)
        );
        registry
            .map_dialog_exact(&keys[0], revision_a, replacement)
            .expect("same-slot replacement");
        assert_eq!(registry.get_key_by_dialog_exact(&first), None);
        assert_eq!(
            registry.get_key_by_dialog_exact(&replacement),
            Some(keys[0].clone())
        );
        registry
            .map_media_exact(&keys[0], revision_a, first_media.clone())
            .expect("first media mapping");
        assert_eq!(
            registry.map_media_exact(&keys[1], revision_b, first_media.clone()),
            Err(SessionRegistryError::MediaCollision)
        );
        registry
            .map_media_exact(&keys[0], revision_a, replacement_media.clone())
            .expect("same-slot media replacement");
        assert_eq!(registry.get_key_by_media_exact(&first_media), None);
        assert_eq!(
            registry.get_key_by_media_exact(&replacement_media),
            Some(keys[0].clone())
        );
    }

    #[tokio::test]
    async fn late_old_generation_write_and_remove_cannot_touch_reused_id() {
        let raw_id = SessionId::from("reused-id");
        let old_authority = SessionLeaseAuthority::new();
        let old_lease = old_authority.admit(raw_id.clone()).expect("old admission");
        let old_key = old_lease.key().clone();
        let old_registry = SessionRegistry::with_authority(Arc::clone(&old_authority));
        let old_revision = old_registry.register_exact(&old_key).expect("old slot");
        assert!(old_registry
            .remove_if(&old_key, old_revision)
            .expect("old cleanup"));

        // Model an authority restart/re-admission while retaining the mapping
        // store. The new epoch makes the key distinct even if the raw ID is the
        // same.
        let new_authority = SessionLeaseAuthority::new();
        let new_lease = new_authority.admit(raw_id).expect("new admission");
        let new_key = new_lease.key().clone();
        let new_registry = SessionRegistry {
            authority: Arc::clone(&new_authority),
            state: Arc::clone(&old_registry.state),
            dialog_mapped_total: Arc::clone(&old_registry.dialog_mapped_total),
            media_mapped_total: Arc::clone(&old_registry.media_mapped_total),
            removed_total: Arc::clone(&old_registry.removed_total),
            remove_missing_total: Arc::clone(&old_registry.remove_missing_total),
            after_exact_mutation: Arc::clone(&old_registry.after_exact_mutation),
        };
        let new_revision = new_registry.register_exact(&new_key).expect("new slot");
        let new_dialog = DialogId::new();
        new_registry
            .map_dialog_exact(&new_key, new_revision, new_dialog)
            .expect("new mapping");

        assert_eq!(
            new_registry.map_dialog_exact(&old_key, old_revision, DialogId::new()),
            Err(SessionRegistryError::StaleSession)
        );
        assert!(!new_registry
            .remove_if(&old_key, old_revision)
            .expect("stale cleanup is harmless"));
        assert_eq!(
            new_registry.get_key_by_dialog_exact(&new_dialog),
            Some(new_key)
        );
    }

    #[tokio::test]
    async fn pending_request_transport_principal_are_one_exact_bundle() {
        let (fixture, keys) = Admitted::sessions(&["bundle"]);
        let registry = &fixture.registry;
        let key = &keys[0];
        let revision = registry.register_exact(key).expect("slot");
        let dialog_id = DialogId::new();
        let info = incoming(key.session_id.clone(), dialog_id, "bundle");
        let parsed = request("bundle");
        let transport = Arc::new(SipTransportSecurityContext::secure("TLS"));
        let principal = principal("bundle");
        registry
            .store_pending_bundle_exact(
                key,
                revision,
                PendingInboundBundle {
                    info: Some(info.clone()),
                    request: Some(Arc::clone(&parsed)),
                    transport: Some(Arc::clone(&transport)),
                    principal: Some(principal.clone()),
                },
            )
            .expect("bundle commit");

        let observed = registry
            .pending_bundle_exact(key, revision)
            .expect("bundle snapshot");
        assert_eq!(observed.info.expect("info").call_id, info.call_id);
        assert!(Arc::ptr_eq(&observed.request.expect("request"), &parsed));
        assert_eq!(
            observed.transport.expect("transport").transport,
            transport.transport
        );
        let observed_principal = observed.principal.expect("principal");
        assert_eq!(observed_principal.subject, principal.subject);
        assert_eq!(observed_principal.tenant, principal.tenant);
        assert_eq!(observed_principal.issuer, principal.issuer);
    }

    #[test]
    fn poisoned_registry_fails_closed() {
        let (fixture, keys) = Admitted::sessions(&["poison"]);
        let state = Arc::clone(&fixture.registry.state);
        let _ = std::thread::spawn(move || {
            let _guard = state.lock().expect("initial lock");
            panic!("poison registry for deterministic test");
        })
        .join();

        assert_eq!(
            fixture.registry.register_exact(&keys[0]),
            Err(SessionRegistryError::Poisoned)
        );
        assert!(fixture
            .registry
            .get_key_by_dialog_exact(&DialogId::new())
            .is_none());
    }
}
