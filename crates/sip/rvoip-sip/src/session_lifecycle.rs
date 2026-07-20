//! Generation-qualified SIP session admission and lifetime fencing.
//!
//! This module is intentionally crate-internal. Lower SIP crates receive only
//! opaque identifiers derived from `SessionKey`; they do not participate in
//! admission or decide when a public session identifier may be reused.

use crate::state_table::SessionId;
use dashmap::mapref::entry::Entry as DashEntry;
use dashmap::DashMap;
use futures::FutureExt;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::future::Future;
use std::num::{NonZeroU64, NonZeroUsize};
use std::ops::{Deref, DerefMut};
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard as StdMutexGuard, Weak};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::sync::{oneshot, watch, Notify, OwnedSemaphorePermit, Semaphore};

const DEFAULT_LIFECYCLE_CAPACITY: usize = 65_536;
const DEFAULT_ANTI_REUSE_HORIZON: Duration = Duration::from_secs(64);
const DEFAULT_SUPERVISOR_ABORT_GRACE: Duration = Duration::from_secs(5);
const DEFAULT_RESOURCE_CAPACITY_PER_SESSION: usize = 1_024;
/// Maximum eager reservation for the two active-lifetime lookup indexes.
///
/// `capacity` is an enforced concurrency bound, not proof that every slot is
/// live at startup. Reserving a large production bound independently in both
/// sharded maps allocates one large bucket table per shard even when the
/// endpoint has only a small active working set. DashMap grows these tables as
/// needed, while the semaphore and admission index continue to enforce the
/// complete configured bound.
const MAX_EAGER_ACTIVE_LIFETIME_INDEX_CAPACITY: usize = 4_096;
const RETAINED_FENCE_CHURN_PER_SECOND: usize = 10;
const RETAINED_FENCE_BURST_MULTIPLIER: usize = 4;
/// Maximum retired admission fences removed in one scheduler turn.
///
/// A qualified high-CPS endpoint can have more than one hundred thousand
/// fences become reusable together. Expiring them in bounded waves prevents
/// the authority's single admission index from monopolizing a Tokio worker,
/// while the immediate follow-up turn still converges without new traffic.
const REUSABLE_FENCE_EXPIRY_BATCH: usize = 4_096;

fn eager_active_lifetime_index_capacity(capacity: usize) -> usize {
    capacity.min(MAX_EAGER_ACTIVE_LIFETIME_INDEX_CAPACITY)
}

fn retained_lifecycle_capacity(active_capacity: usize, anti_reuse_horizon: Duration) -> usize {
    let horizon_seconds = usize::try_from(anti_reuse_horizon.as_secs())
        .unwrap_or(usize::MAX)
        .saturating_add(usize::from(anti_reuse_horizon.subsec_nanos() != 0));
    let qualified_churn = RETAINED_FENCE_CHURN_PER_SECOND
        .saturating_mul(horizon_seconds)
        .saturating_mul(RETAINED_FENCE_BURST_MULTIPLIER);
    active_capacity
        .saturating_add(qualified_churn)
        .max(active_capacity.saturating_mul(2))
}

type ClockSleep = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

trait LifecycleClock: Send + Sync {
    fn now(&self) -> Instant;
    fn sleep_until(&self, deadline: Instant) -> ClockSleep;
}

struct SystemLifecycleClock;

impl LifecycleClock for SystemLifecycleClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep_until(&self, deadline: Instant) -> ClockSleep {
        Box::pin(tokio::time::sleep_until(tokio::time::Instant::from_std(
            deadline,
        )))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SessionLifecycleConfig {
    capacity: NonZeroUsize,
    retained_capacity: NonZeroUsize,
    anti_reuse_horizon: Duration,
    supervisor_abort_grace: Duration,
    resource_capacity_per_session: NonZeroUsize,
}

impl SessionLifecycleConfig {
    pub(crate) fn new(
        capacity: usize,
        anti_reuse_horizon: Duration,
    ) -> Result<Self, SessionLifecycleConfigError> {
        let capacity =
            NonZeroUsize::new(capacity).ok_or(SessionLifecycleConfigError::ZeroCapacity)?;
        if anti_reuse_horizon.is_zero() {
            return Err(SessionLifecycleConfigError::ZeroAntiReuseHorizon);
        }
        Ok(Self {
            capacity,
            retained_capacity: NonZeroUsize::new(retained_lifecycle_capacity(
                capacity.get(),
                anti_reuse_horizon,
            ))
            .expect("active capacity makes retained capacity nonzero"),
            anti_reuse_horizon,
            supervisor_abort_grace: DEFAULT_SUPERVISOR_ABORT_GRACE,
            resource_capacity_per_session: NonZeroUsize::new(DEFAULT_RESOURCE_CAPACITY_PER_SESSION)
                .expect("default resource capacity is nonzero"),
        })
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn with_supervisor_abort_grace(
        mut self,
        supervisor_abort_grace: Duration,
    ) -> Result<Self, SessionLifecycleConfigError> {
        if supervisor_abort_grace.is_zero() {
            return Err(SessionLifecycleConfigError::ZeroSupervisorAbortGrace);
        }
        self.supervisor_abort_grace = supervisor_abort_grace;
        Ok(self)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn with_resource_capacity_per_session(
        mut self,
        capacity: usize,
    ) -> Result<Self, SessionLifecycleConfigError> {
        self.resource_capacity_per_session =
            NonZeroUsize::new(capacity).ok_or(SessionLifecycleConfigError::ZeroResourceCapacity)?;
        Ok(self)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn with_retained_capacity(
        mut self,
        retained_capacity: usize,
    ) -> Result<Self, SessionLifecycleConfigError> {
        let retained_capacity = NonZeroUsize::new(retained_capacity)
            .ok_or(SessionLifecycleConfigError::ZeroRetainedCapacity)?;
        if retained_capacity.get() < self.capacity.get() {
            return Err(SessionLifecycleConfigError::RetainedCapacityBelowActive);
        }
        self.retained_capacity = retained_capacity;
        Ok(self)
    }
}

impl Default for SessionLifecycleConfig {
    fn default() -> Self {
        Self {
            capacity: NonZeroUsize::new(DEFAULT_LIFECYCLE_CAPACITY)
                .expect("default lifecycle capacity is nonzero"),
            retained_capacity: NonZeroUsize::new(retained_lifecycle_capacity(
                DEFAULT_LIFECYCLE_CAPACITY,
                DEFAULT_ANTI_REUSE_HORIZON,
            ))
            .expect("default retained lifecycle capacity is nonzero"),
            anti_reuse_horizon: DEFAULT_ANTI_REUSE_HORIZON,
            supervisor_abort_grace: DEFAULT_SUPERVISOR_ABORT_GRACE,
            resource_capacity_per_session: NonZeroUsize::new(DEFAULT_RESOURCE_CAPACITY_PER_SESSION)
                .expect("default resource capacity is nonzero"),
        }
    }
}

// These explicit names are part of diagnostic matching in package tests.
#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub(crate) enum SessionLifecycleConfigError {
    #[error("session lifecycle capacity must be nonzero")]
    ZeroCapacity,
    #[error("retained session lifecycle capacity must be nonzero")]
    ZeroRetainedCapacity,
    #[error("retained session lifecycle capacity must cover active capacity")]
    RetainedCapacityBelowActive,
    #[error("session anti-reuse horizon must be nonzero")]
    ZeroAntiReuseHorizon,
    #[error("session supervisor abort grace must be nonzero")]
    #[cfg_attr(not(test), allow(dead_code))]
    ZeroSupervisorAbortGrace,
    #[error("per-session managed resource capacity must be nonzero")]
    #[cfg_attr(not(test), allow(dead_code))]
    ZeroResourceCapacity,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct AuthorityEpoch(u128);

impl AuthorityEpoch {
    fn random() -> Self {
        Self(rand::random())
    }

    #[cfg(test)]
    fn fixed(value: u128) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SessionGeneration {
    pub(crate) authority_epoch: AuthorityEpoch,
    pub(crate) sequence: NonZeroU64,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SessionKey {
    pub(crate) session_id: SessionId,
    pub(crate) generation: SessionGeneration,
}

impl SessionKey {
    /// Stable, process-unique suffix for lower-layer resource identities.
    ///
    /// Raw SIP session identifiers may be admitted again after the anti-reuse
    /// horizon.  Lower layers that outlive an adapter call must therefore use
    /// the authority epoch and generation sequence as part of their identity;
    /// otherwise delayed cleanup for an old lifetime can target a newly
    /// admitted session with the same application identifier.
    pub(crate) fn resource_generation_suffix(&self) -> String {
        format!(
            "{:032x}-{}",
            self.generation.authority_epoch.0,
            self.generation.sequence.get()
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionPhase {
    Active,
    Quiescing { deadline: Instant },
    Releasing,
    Quarantined { since: Instant },
    Retired { retired_at: Instant },
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum SessionOperationKind {
    StateTransition,
    Signaling,
    Media,
    EventDispatch,
    #[cfg_attr(not(test), allow(dead_code))]
    Test(&'static str),
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct OperationId(NonZeroU64);

#[derive(Clone, Debug)]
struct OperationMeta {
    #[cfg_attr(not(test), allow(dead_code))]
    kind: SessionOperationKind,
    #[cfg_attr(not(test), allow(dead_code))]
    hard_deadline: Option<Instant>,
    commit_revision: Option<CommitRevision>,
    resource_ids: HashSet<ResourceId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CommitRevision(NonZeroU64);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct ResourceId(NonZeroU64);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ResourceDescriptor {
    pub(crate) kind: &'static str,
    pub(crate) identity: String,
}

impl ResourceDescriptor {
    pub(crate) fn new(kind: &'static str, identity: impl Into<String>) -> Self {
        Self {
            kind,
            identity: identity.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResourceSpec {
    descriptor: ResourceDescriptor,
    /// Runtime dependencies. Dependents are released before their
    /// dependencies, producing reverse-topological release waves.
    release_dependencies: Vec<ResourceId>,
    release_timeout: Duration,
}

impl ResourceSpec {
    pub(crate) fn new(
        descriptor: ResourceDescriptor,
        mut release_dependencies: Vec<ResourceId>,
        release_timeout: Duration,
    ) -> Result<Self, ResourceRegistryError> {
        if release_timeout.is_zero() {
            return Err(ResourceRegistryError::InvalidReleaseTimeout);
        }
        release_dependencies.sort_unstable();
        release_dependencies.dedup();
        Ok(Self {
            descriptor,
            release_dependencies,
            release_timeout,
        })
    }

    #[allow(dead_code)]
    pub(crate) fn descriptor(&self) -> &ResourceDescriptor {
        &self.descriptor
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("managed resource release failed: {code}")]
pub(crate) struct ManagedResourceReleaseError {
    code: &'static str,
}

impl ManagedResourceReleaseError {
    pub(crate) fn new(code: &'static str) -> Self {
        Self { code }
    }
}

type ResourceReleaseFuture =
    Pin<Box<dyn Future<Output = Result<(), ManagedResourceReleaseError>> + Send + 'static>>;

pub(crate) trait ManagedSessionResource: Send + Sync + 'static {
    fn descriptor(&self) -> ResourceDescriptor;
    fn cancel(&self);
    fn release(&self) -> ResourceReleaseFuture;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResourceOrphanReason {
    InstallAttemptDropped,
    DispatchPermitDropped,
    DescriptorMismatch,
    CancelPanicked,
    ReleaseFailed,
    ReleasePanicked,
    ReleaseDeadline,
    ReleaseDriverDropped,
    DependencyCycle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResourceStateKind {
    Reserved,
    Installing,
    Captured,
    Live,
    Releasing,
    Orphaned,
}

enum ManagedResourceState {
    Reserved,
    Installing,
    Captured(Arc<dyn ManagedSessionResource>),
    Live(Arc<dyn ManagedSessionResource>),
    Releasing(Arc<dyn ManagedSessionResource>),
    Orphaned {
        resource: Option<Arc<dyn ManagedSessionResource>>,
        reason: ResourceOrphanReason,
        retry_ready: bool,
    },
}

impl ManagedResourceState {
    fn kind(&self) -> ResourceStateKind {
        match self {
            Self::Reserved => ResourceStateKind::Reserved,
            Self::Installing => ResourceStateKind::Installing,
            Self::Captured(_) => ResourceStateKind::Captured,
            Self::Live(_) => ResourceStateKind::Live,
            Self::Releasing(_) => ResourceStateKind::Releasing,
            Self::Orphaned { .. } => ResourceStateKind::Orphaned,
        }
    }

    fn resource(&self) -> Option<Arc<dyn ManagedSessionResource>> {
        match self {
            Self::Captured(resource) | Self::Live(resource) | Self::Releasing(resource) => {
                Some(Arc::clone(resource))
            }
            Self::Orphaned { resource, .. } => resource.as_ref().map(Arc::clone),
            Self::Reserved | Self::Installing => None,
        }
    }
}

impl std::fmt::Debug for ManagedResourceState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Orphaned {
                resource,
                reason,
                retry_ready,
            } => formatter
                .debug_struct("Orphaned")
                .field("has_resource", &resource.is_some())
                .field("reason", reason)
                .field("retry_ready", retry_ready)
                .finish(),
            state => formatter
                .debug_tuple("ResourceState")
                .field(&state.kind())
                .finish(),
        }
    }
}

#[derive(Debug)]
struct ManagedResourceEntry {
    spec: ResourceSpec,
    owner_operation: Option<OperationId>,
    state: ManagedResourceState,
    state_since: Instant,
}

/// Created only after the authority observes successful release of the exact
/// captured descriptor. It is private so callers cannot forge cleanup proof.
struct RollbackReceipt {
    key: SessionKey,
    resource_id: ResourceId,
    descriptor: ResourceDescriptor,
}

struct ResourceReleaseWork {
    resource_id: ResourceId,
    spec: ResourceSpec,
    resource: Arc<dyn ManagedSessionResource>,
}

struct ResourceReleaseDropBomb<'a> {
    authority: &'a SessionLeaseAuthority,
    key: SessionKey,
    resource_id: ResourceId,
    armed: bool,
}

impl ResourceReleaseDropBomb<'_> {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ResourceReleaseDropBomb<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.authority.mark_resource_orphan(
                &self.key,
                self.resource_id,
                ResourceOrphanReason::ReleaseDriverDropped,
                false,
            );
        }
    }
}

enum ResourceReleaseAttempt<'a> {
    Released {
        receipt: RollbackReceipt,
        drop_bomb: ResourceReleaseDropBomb<'a>,
    },
    Orphaned {
        resource_id: ResourceId,
        reason: ResourceOrphanReason,
        drop_bomb: ResourceReleaseDropBomb<'a>,
    },
    AuthorityFatal {
        reason: AuthorityFatalReason,
        drop_bomb: ResourceReleaseDropBomb<'a>,
    },
}

enum ResourceReleasePlan {
    Complete,
    Wave(Vec<ResourceReleaseWork>),
    InFlight,
    Wait(QuarantineReason),
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub(crate) enum ResourceRegistryError {
    #[error("session lifetime is no longer current")]
    StaleGeneration,
    #[error("session lifetime is not accepting resource changes")]
    NotActive,
    #[error("managed resource capacity exhausted")]
    CapacityExhausted,
    #[error("managed resource sequence exhausted")]
    ResourceSequenceExhausted,
    #[error("managed resource release timeout must be nonzero")]
    InvalidReleaseTimeout,
    #[error("managed resource dependency is not registered in this lifetime")]
    UnknownDependency,
    #[error("managed resource dependency is not committed or captured by this operation")]
    InvalidDependencyState,
    #[error("managed resource is not in the required state")]
    InvalidState,
    #[error("captured resource descriptor does not match its reservation")]
    DescriptorMismatch,
    #[error("session lifecycle authority failed closed: {0:?}")]
    AuthorityFatal(AuthorityFatalReason),
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AuthorityFatalReason {
    IndexPoisoned = 1,
    CellPoisoned = 2,
    SupervisorRegistryPoisoned = 3,
    RetirementDeadlineOverflow = 4,
    InvariantViolation = 5,
    DrainDeadlineOverflow = 6,
    ResourceDeadlineOverflow = 7,
}

impl AuthorityFatalReason {
    fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::IndexPoisoned),
            2 => Some(Self::CellPoisoned),
            3 => Some(Self::SupervisorRegistryPoisoned),
            4 => Some(Self::RetirementDeadlineOverflow),
            5 => Some(Self::InvariantViolation),
            6 => Some(Self::DrainDeadlineOverflow),
            7 => Some(Self::ResourceDeadlineOverflow),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QuarantineReason {
    OperationDeadline,
    OperationPanicked,
    OperationAbandoned,
    CommittedAfterCancellation,
    SupervisorAbandoned,
    QuiesceDeadline,
    #[allow(dead_code)]
    AuthorityFatal,
    ResourceInstallOrphaned,
    ResourceDescriptorMismatch,
    ResourceCancelPanicked,
    ResourceReleaseFailed,
    ResourceReleasePanicked,
    ResourceReleaseDeadline,
    ResourceReleaseAmbiguous,
    ResourceDependencyCycle,
    ResourceRollbackIncomplete,
}

impl QuarantineReason {
    fn is_recoverable(self) -> bool {
        matches!(
            self,
            Self::OperationDeadline
                | Self::QuiesceDeadline
                | Self::ResourceInstallOrphaned
                | Self::ResourceDescriptorMismatch
                | Self::ResourceCancelPanicked
                | Self::ResourceReleaseFailed
                | Self::ResourceReleasePanicked
                | Self::ResourceReleaseDeadline
                | Self::ResourceReleaseAmbiguous
                | Self::ResourceDependencyCycle
                | Self::ResourceRollbackIncomplete
        )
    }
}

fn merge_quarantine_reason(
    current: Option<QuarantineReason>,
    proposed: QuarantineReason,
) -> QuarantineReason {
    match current {
        Some(existing) if !existing.is_recoverable() => existing,
        Some(existing) if proposed.is_recoverable() => existing,
        Some(_) | None => proposed,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TeardownOutcome {
    Retired {
        key: SessionKey,
    },
    Quarantined {
        key: SessionKey,
        reason: QuarantineReason,
    },
}

#[derive(Clone, Debug)]
enum AdmissionSlot {
    Live(SessionKey),
    NonReusable(SessionKey),
    Retired {
        generation: SessionGeneration,
        retired_at: Instant,
        reusable_at: Instant,
        reported_quarantine: Option<QuarantineReason>,
    },
}

/// Why one exact session generation remains unavailable for admission.
///
/// Non-reusable lifetimes still own a full [`SessionCell`] because teardown or
/// quarantine recovery may need its operations and managed resources. A
/// retired fence is the complete compact proof that those registries were
/// empty, the active permit was returned, and only the SIP anti-reuse horizon
/// remains. Retired fences deliberately do not retain a `SessionCell`.
impl AdmissionSlot {
    fn retains_exact_cell(&self, key: &SessionKey) -> bool {
        match self {
            Self::Live(current) | Self::NonReusable(current) => current == key,
            Self::Retired { .. } => false,
        }
    }
}

/// One generation-qualified anti-reuse deadline.
///
/// `BinaryHeap` is a max-heap, so the ordering is reversed to keep the
/// earliest deadline at the top. Generation is a process-unique tie breaker;
/// stale entries are harmless because expiry also compares the complete key
/// and deadline with the current admission slot before removing anything.
#[derive(Clone, Debug)]
struct ReusableFenceDeadline {
    reusable_at: Instant,
    session_id: Arc<SessionId>,
    generation: SessionGeneration,
}

impl PartialEq for ReusableFenceDeadline {
    fn eq(&self, other: &Self) -> bool {
        self.reusable_at == other.reusable_at && self.generation == other.generation
    }
}

impl Eq for ReusableFenceDeadline {}

impl PartialOrd for ReusableFenceDeadline {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ReusableFenceDeadline {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other
            .reusable_at
            .cmp(&self.reusable_at)
            .then_with(|| {
                other
                    .generation
                    .authority_epoch
                    .0
                    .cmp(&self.generation.authority_epoch.0)
            })
            .then_with(|| other.generation.sequence.cmp(&self.generation.sequence))
    }
}

#[derive(Debug)]
struct AdmissionIndex {
    /// Raw identifiers are allocated once and shared with their deadline
    /// entries. Live/non-reusable slots retain a full generation-qualified
    /// key, while retired slots retain only the generation and timestamps.
    slots: HashMap<Arc<SessionId>, AdmissionSlot>,
    reusable_deadlines: BinaryHeap<ReusableFenceDeadline>,
    #[cfg(test)]
    reuse_deadline_inspections: u64,
}

impl AdmissionIndex {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            slots: HashMap::with_capacity(capacity),
            reusable_deadlines: BinaryHeap::new(),
            #[cfg(test)]
            reuse_deadline_inspections: 0,
        }
    }

    fn block_non_reusable(&mut self, key: SessionKey) {
        if let Some(slot) = self.slots.get_mut(&key.session_id) {
            *slot = AdmissionSlot::NonReusable(key);
        } else {
            // A missing admitted slot is an upstream invariant violation, but
            // retaining the exact key still fails closed until the authority
            // detects and latches that violation at its next exact check.
            self.slots.insert(
                Arc::new(key.session_id.clone()),
                AdmissionSlot::NonReusable(key),
            );
        }
    }

    fn retire_until(
        &mut self,
        key: SessionKey,
        retired_at: Instant,
        reusable_at: Instant,
        reported_quarantine: Option<QuarantineReason>,
    ) -> bool {
        let advances_next_deadline = self
            .reusable_deadlines
            .peek()
            .is_none_or(|deadline| reusable_at < deadline.reusable_at);
        let session_id = self
            .slots
            .get_key_value(&key.session_id)
            .map(|(session_id, _)| Arc::clone(session_id))
            .unwrap_or_else(|| Arc::new(key.session_id.clone()));
        self.slots.insert(
            Arc::clone(&session_id),
            AdmissionSlot::Retired {
                generation: key.generation,
                retired_at,
                reusable_at,
                reported_quarantine,
            },
        );
        self.reusable_deadlines.push(ReusableFenceDeadline {
            reusable_at,
            session_id,
            generation: key.generation,
        });
        advances_next_deadline
    }
}

impl Deref for AdmissionIndex {
    type Target = HashMap<Arc<SessionId>, AdmissionSlot>;

    fn deref(&self) -> &Self::Target {
        &self.slots
    }
}

impl DerefMut for AdmissionIndex {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.slots
    }
}

#[derive(Debug)]
struct CellState {
    phase: SessionPhase,
    active_permit: Option<OwnedSemaphorePermit>,
    next_operation: u64,
    next_commit_revision: u64,
    next_resource: u64,
    operations: HashMap<OperationId, OperationMeta>,
    resources: HashMap<ResourceId, ManagedResourceEntry>,
    sticky_failure: Option<QuarantineReason>,
    teardown: Option<Arc<TeardownControl>>,
    teardown_driver_complete: bool,
}

#[derive(Debug)]
struct TeardownControl {
    result: watch::Sender<Option<TeardownOutcome>>,
    done: watch::Sender<bool>,
}

#[derive(Debug)]
struct SessionCell {
    state: StdMutex<CellState>,
    changed: Notify,
    cancel: watch::Sender<bool>,
}

impl SessionCell {
    fn active(active_permit: OwnedSemaphorePermit) -> Self {
        let (cancel, _) = watch::channel(false);
        Self {
            state: StdMutex::new(CellState {
                phase: SessionPhase::Active,
                active_permit: Some(active_permit),
                next_operation: 1,
                next_commit_revision: 1,
                next_resource: 1,
                operations: HashMap::new(),
                resources: HashMap::new(),
                sticky_failure: None,
                teardown: None,
                teardown_driver_complete: false,
            }),
            changed: Notify::new(),
            cancel,
        }
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub(crate) enum SessionAdmissionError {
    #[error("session identifier already has an active lifetime")]
    AlreadyActive,
    #[error("session identifier remains inside its anti-reuse horizon")]
    ReuseBlocked,
    #[error("session lifecycle capacity exhausted")]
    CapacityExhausted,
    #[error("retained session lifecycle capacity exhausted")]
    RetainedCapacityExhausted,
    #[error("session generation sequence exhausted")]
    GenerationExhausted,
    #[error("session lifetime is no longer current")]
    #[cfg_attr(not(test), allow(dead_code))]
    StaleGeneration,
    #[error("session lifecycle authority failed closed: {0:?}")]
    AuthorityFatal(AuthorityFatalReason),
    #[error("session lifecycle authority is draining")]
    AuthorityDraining,
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub(crate) enum SessionOperationError {
    #[error("session lifetime is no longer current")]
    StaleGeneration,
    #[error("session operation sequence exhausted")]
    OperationSequenceExhausted,
    #[error("session operation supervisor requires a Tokio runtime")]
    SupervisorUnavailable,
    #[error("session operation timeout must be nonzero")]
    InvalidTimeout,
    #[error("session operation deadline exceeds the runtime clock range")]
    DeadlineOverflow,
    #[error("session lifecycle authority failed closed: {0:?}")]
    AuthorityFatal(AuthorityFatalReason),
    #[error("session lifecycle authority is draining")]
    AuthorityDraining,
    #[error("session operation still owns unresolved managed resources")]
    ResourcesUnresolved,
    #[error("managed resource rollback failed")]
    ResourceRollbackFailed,
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub(crate) enum OwnedOperationError {
    #[error("owned session operation exceeded its hard deadline")]
    DeadlineExceeded,
    #[error("owned session operation panicked")]
    Panicked,
    #[error("owned session operation supervisor ended without a result")]
    SupervisorDropped,
}

/// Explicit proof that owned work either committed generation-bound state or
/// completed its exact rollback before releasing the operation registration.
pub(crate) struct OwnedOperationCompletion<T> {
    disposition: CompletionDisposition,
    value: T,
}

impl<T> OwnedOperationCompletion<T> {
    fn into_inner(self) -> T {
        self.value
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CompletionDisposition {
    Committed(CommitRevision),
    RolledBack,
}

#[derive(Clone, Copy)]
enum OperationRelease {
    Completed(CompletionDisposition),
    Failed(QuarantineReason),
}

fn classify_operation_release(
    state: &CellState,
    operation_id: OperationId,
    release: OperationRelease,
) -> Option<(Vec<ResourceId>, Option<QuarantineReason>)> {
    let meta = state.operations.get(&operation_id)?;
    let recorded_commit = meta.commit_revision;
    let operation_resources: Vec<_> = meta.resource_ids.iter().copied().collect();
    let committed_resources_valid = operation_resources.iter().all(|resource_id| {
        state.resources.get(resource_id).is_some_and(|entry| {
            entry.owner_operation.is_none() && matches!(entry.state, ManagedResourceState::Live(_))
        })
    });
    let failure = match release {
        OperationRelease::Failed(reason) => Some(reason),
        OperationRelease::Completed(CompletionDisposition::Committed(revision))
            if recorded_commit != Some(revision) || !committed_resources_valid =>
        {
            Some(QuarantineReason::CommittedAfterCancellation)
        }
        OperationRelease::Completed(CompletionDisposition::RolledBack)
            if !operation_resources.is_empty() =>
        {
            Some(QuarantineReason::ResourceRollbackIncomplete)
        }
        OperationRelease::Completed(
            CompletionDisposition::Committed(_) | CompletionDisposition::RolledBack,
        ) => None,
    };
    Some((operation_resources, failure))
}

pub(crate) struct OperationWaiter<T> {
    result: oneshot::Receiver<Result<T, OwnedOperationError>>,
}

impl<T> Future for OperationWaiter<T> {
    type Output = Result<T, OwnedOperationError>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.result).poll(context) {
            Poll::Ready(Ok(result)) => Poll::Ready(result),
            Poll::Ready(Err(_)) => Poll::Ready(Err(OwnedOperationError::SupervisorDropped)),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[derive(Clone)]
pub(crate) struct TeardownWaiter {
    result: watch::Receiver<Option<TeardownOutcome>>,
    done: watch::Receiver<bool>,
    authority: Arc<SessionLeaseAuthority>,
}

impl TeardownWaiter {
    pub(crate) async fn wait(mut self) -> Result<TeardownOutcome, SessionOperationError> {
        loop {
            if let Some(outcome) = self.result.borrow_and_update().clone() {
                return Ok(outcome);
            }
            if let Some(reason) = self.authority.fatal_reason() {
                return Err(SessionOperationError::AuthorityFatal(reason));
            }
            let fatal = self.authority.fatal_changed.notified();
            tokio::pin!(fatal);
            fatal.as_mut().enable();
            if let Some(reason) = self.authority.fatal_reason() {
                return Err(SessionOperationError::AuthorityFatal(reason));
            }
            tokio::select! {
                changed = self.result.changed() => {
                    changed.map_err(|_| SessionOperationError::StaleGeneration)?;
                }
                () = &mut fatal => {}
            }
        }
    }

    pub(crate) async fn wait_supervisor(mut self) -> Result<(), SessionOperationError> {
        loop {
            if *self.done.borrow_and_update() {
                return Ok(());
            }
            if let Some(reason) = self.authority.fatal_reason() {
                return Err(SessionOperationError::AuthorityFatal(reason));
            }
            let fatal = self.authority.fatal_changed.notified();
            tokio::pin!(fatal);
            fatal.as_mut().enable();
            if let Some(reason) = self.authority.fatal_reason() {
                return Err(SessionOperationError::AuthorityFatal(reason));
            }
            tokio::select! {
                changed = self.done.changed() => {
                    changed.map_err(|_| SessionOperationError::StaleGeneration)?;
                }
                () = &mut fatal => {}
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct SupervisorId(NonZeroU64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SupervisorKind {
    OwnedOperation,
    Teardown,
}

#[derive(Debug)]
struct SupervisorEntry {
    #[cfg_attr(not(test), allow(dead_code))]
    key: SessionKey,
    #[cfg_attr(not(test), allow(dead_code))]
    kind: SupervisorKind,
    abort: Option<tokio::task::AbortHandle>,
    abort_requested: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CurrentSupervisor {
    epoch: AuthorityEpoch,
    id: SupervisorId,
}

tokio::task_local! {
    static CURRENT_LIFECYCLE_SUPERVISOR: CurrentSupervisor;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct SupervisorDrainReport {
    pub(crate) deadline_reached: bool,
    pub(crate) abort_requested: usize,
    pub(crate) stragglers: usize,
    pub(crate) excluded_current: bool,
    pub(crate) fatal_reason: Option<AuthorityFatalReason>,
}

struct SupervisorRegistration {
    authority: Arc<SessionLeaseAuthority>,
    id: Option<SupervisorId>,
}

impl SupervisorRegistration {
    fn new(authority: Arc<SessionLeaseAuthority>, id: SupervisorId) -> Self {
        Self {
            authority,
            id: Some(id),
        }
    }

    fn unregister(&mut self) {
        if let Some(id) = self.id.take() {
            self.authority.unregister_supervisor(id);
        }
    }
}

impl Drop for SupervisorRegistration {
    fn drop(&mut self) {
        self.unregister();
    }
}

struct TeardownSupervisorGuard {
    authority: Arc<SessionLeaseAuthority>,
    key: SessionKey,
    registration: Option<SupervisorRegistration>,
    armed: bool,
}

impl TeardownSupervisorGuard {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TeardownSupervisorGuard {
    fn drop(&mut self) {
        if self.armed {
            // This transition and its result publication are one cell-locked
            // operation. If the driver already committed a terminal state,
            // this is a no-op and cannot overwrite that result.
            let _ = self
                .authority
                .transition_terminal_quarantine(&self.key, QuarantineReason::SupervisorAbandoned);
        }
        drop(self.registration.take());
    }
}

/// Reservation token created synchronously before any lower-layer install
/// await. Dropping it without an exact unused confirmation charges an orphan.
pub(crate) struct ResourceInstallAttempt {
    authority: Arc<SessionLeaseAuthority>,
    key: SessionKey,
    operation_id: OperationId,
    resource_id: ResourceId,
    armed: bool,
}

impl ResourceInstallAttempt {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn id(&self) -> ResourceId {
        self.resource_id
    }

    pub(crate) fn dispatch(mut self) -> Result<ResourceDispatchPermit, ResourceRegistryError> {
        if let Err(error) = self.authority.transition_resource_installing(
            &self.key,
            self.operation_id,
            self.resource_id,
        ) {
            // No lower-layer install was dispatched. Retire the reservation
            // synchronously when the exact operation is still retained (the
            // common reserve-vs-quiesce race), so callers can roll back as a
            // normal lifecycle-class error instead of manufacturing an
            // unresolved orphan. Authority-fatal failures remain armed and
            // fail closed through Drop.
            if self
                .authority
                .confirm_resource_unused(&self.key, self.operation_id, self.resource_id)
                .is_ok()
            {
                self.armed = false;
            }
            return Err(error);
        }
        self.armed = false;
        Ok(ResourceDispatchPermit {
            authority: Arc::clone(&self.authority),
            key: self.key.clone(),
            operation_id: self.operation_id,
            resource_id: self.resource_id,
            armed: true,
        })
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn confirm_unused(mut self) -> Result<(), ResourceRegistryError> {
        self.authority
            .confirm_resource_unused(&self.key, self.operation_id, self.resource_id)?;
        self.armed = false;
        Ok(())
    }
}

impl Drop for ResourceInstallAttempt {
    fn drop(&mut self) {
        if self.armed {
            self.authority.orphan_install_token(
                &self.key,
                self.operation_id,
                self.resource_id,
                ResourceOrphanReason::InstallAttemptDropped,
            );
        }
    }
}

/// Single-use permission to perform one lower-layer install. It must resolve
/// to either an exact captured resource or `confirm_unused`.
pub(crate) struct ResourceDispatchPermit {
    authority: Arc<SessionLeaseAuthority>,
    key: SessionKey,
    operation_id: OperationId,
    resource_id: ResourceId,
    armed: bool,
}

impl ResourceDispatchPermit {
    #[allow(dead_code)]
    pub(crate) fn id(&self) -> ResourceId {
        self.resource_id
    }

    /// Transfer the single-use dispatch proof into the lower-layer install
    /// callback. The lower layer must call `capture_at_install` at the exact
    /// point where its externally visible mutation succeeds; callers must not
    /// wait for an install future to return and capture afterward.
    pub(crate) fn into_installation_sink(mut self) -> ResourceInstallationSink {
        self.armed = false;
        ResourceInstallationSink {
            authority: Arc::clone(&self.authority),
            key: self.key.clone(),
            operation_id: self.operation_id,
            resource_id: self.resource_id,
            armed: true,
        }
    }
}

impl Drop for ResourceDispatchPermit {
    fn drop(&mut self) {
        if self.armed {
            self.authority.orphan_install_token(
                &self.key,
                self.operation_id,
                self.resource_id,
                ResourceOrphanReason::DispatchPermitDropped,
            );
        }
    }
}

/// Uncloneable lower-layer callback that resolves an install at its mutation
/// linearization point. If the install future/task is cancelled before it can
/// prove either outcome, dropping this sink retains the reservation as a
/// non-expiring orphan instead of guessing that no resource exists.
pub(crate) struct ResourceInstallationSink {
    authority: Arc<SessionLeaseAuthority>,
    key: SessionKey,
    operation_id: OperationId,
    resource_id: ResourceId,
    armed: bool,
}

impl ResourceInstallationSink {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn id(&self) -> ResourceId {
        self.resource_id
    }

    pub(crate) fn capture_at_install(
        mut self,
        resource: Arc<dyn ManagedSessionResource>,
    ) -> Result<ResourceId, ResourceRegistryError> {
        let result = self.authority.capture_resource(
            &self.key,
            self.operation_id,
            self.resource_id,
            resource,
        );
        // capture_resource retains mismatched resources as orphans, so this
        // permit is resolved on both success and descriptor mismatch.
        if result.is_ok() || result == Err(ResourceRegistryError::DescriptorMismatch) {
            self.armed = false;
        }
        result.map(|()| self.resource_id)
    }

    /// Resolve the attempted install only when the lower layer can prove that
    /// it made no external mutation and created no resource.
    #[allow(dead_code)]
    pub(crate) fn confirm_unused(mut self) -> Result<(), ResourceRegistryError> {
        self.authority
            .confirm_resource_unused(&self.key, self.operation_id, self.resource_id)?;
        self.armed = false;
        Ok(())
    }
}

impl Drop for ResourceInstallationSink {
    fn drop(&mut self) {
        if self.armed {
            self.authority.orphan_install_token(
                &self.key,
                self.operation_id,
                self.resource_id,
                ResourceOrphanReason::DispatchPermitDropped,
            );
        }
    }
}

/// Uncloneable ownership token for one supervised operation.
pub(crate) struct OwnedOperation {
    context: OperationContext,
}

impl OwnedOperation {
    #[allow(dead_code)]
    pub(crate) fn key(&self) -> &SessionKey {
        &self.context.key
    }

    #[allow(dead_code)]
    pub(crate) fn cancellation(&self) -> Option<watch::Receiver<bool>> {
        self.context.cancellation()
    }

    pub(crate) fn reserve_resource(
        &mut self,
        spec: ResourceSpec,
    ) -> Result<ResourceInstallAttempt, ResourceRegistryError> {
        self.context
            .authority
            .reserve_resource(&self.context.key, self.context.operation_id, spec)
    }

    pub(crate) fn commit(self) -> Result<CommittedOperation, OwnedCommitFailure> {
        match self
            .context
            .authority
            .record_operation_commit(&self.context.key, self.context.operation_id)
        {
            Ok(revision) => Ok(CommittedOperation {
                context: self.context,
                revision,
            }),
            Err(error) => Err(OwnedCommitFailure {
                operation: self,
                error,
            }),
        }
    }

    pub(crate) async fn rollback<T>(
        self,
        value: T,
    ) -> Result<OwnedOperationCompletion<T>, SessionOperationError> {
        self.context
            .authority
            .release_operation_resources(&self.context.key, self.context.operation_id)
            .await?;
        Ok(OwnedOperationCompletion {
            disposition: CompletionDisposition::RolledBack,
            value,
        })
    }
}

pub(crate) struct OwnedCommitFailure {
    operation: OwnedOperation,
    error: SessionOperationError,
}

impl OwnedCommitFailure {
    #[allow(dead_code)]
    pub(crate) fn error(&self) -> SessionOperationError {
        self.error
    }

    pub(crate) fn into_operation(self) -> OwnedOperation {
        self.operation
    }
}

impl std::fmt::Debug for OwnedCommitFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OwnedCommitFailure")
            .field("error", &self.error)
            .finish()
    }
}

/// Proof that operation commit and Captured->Live promotion linearized while
/// the session was Active. Completion may safely arrive after quiescing.
pub(crate) struct CommittedOperation {
    #[allow(dead_code)]
    context: OperationContext,
    revision: CommitRevision,
}

impl CommittedOperation {
    pub(crate) fn complete<T>(self, value: T) -> OwnedOperationCompletion<T> {
        OwnedOperationCompletion {
            disposition: CompletionDisposition::Committed(self.revision),
            value,
        }
    }
}

#[derive(Clone)]
pub(crate) struct OperationContext {
    authority: Arc<SessionLeaseAuthority>,
    key: SessionKey,
    operation_id: OperationId,
}

impl OperationContext {
    #[allow(dead_code)]
    pub(crate) fn key(&self) -> &SessionKey {
        &self.key
    }

    #[allow(dead_code)]
    pub(crate) fn operation_id(&self) -> OperationId {
        self.operation_id
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn cancellation(&self) -> Option<watch::Receiver<bool>> {
        self.authority.require_healthy().ok()?;
        self.authority
            .cells
            .get(&self.key)
            .map(|cell| cell.cancel.subscribe())
    }

    pub(crate) fn ensure_current(&self) -> Result<(), SessionOperationError> {
        self.authority
            .ensure_operation_current(&self.key, self.operation_id)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn prepare_commit(&self) -> Result<CommitPermit, SessionOperationError> {
        self.ensure_current()?;
        Ok(CommitPermit {
            operation: self.clone(),
            _not_send: std::marker::PhantomData,
        })
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct CommitPermit {
    operation: OperationContext,
    _not_send: std::marker::PhantomData<Rc<()>>,
}

impl CommitPermit {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn finish(self) -> Result<(), SessionOperationError> {
        self.operation
            .authority
            .record_operation_commit(&self.operation.key, self.operation.operation_id)
            .map(|_| ())
    }
}

struct RegisteredOperation {
    context: OperationContext,
    released: bool,
}

impl RegisteredOperation {
    fn context(&self) -> OperationContext {
        self.context.clone()
    }

    fn finish(mut self, disposition: CompletionDisposition) {
        self.context.authority.complete_operation(
            &self.context.key,
            self.context.operation_id,
            OperationRelease::Completed(disposition),
        );
        self.released = true;
    }

    fn fail(mut self, reason: QuarantineReason) {
        self.context.authority.complete_operation(
            &self.context.key,
            self.context.operation_id,
            OperationRelease::Failed(reason),
        );
        self.released = true;
    }

    fn latch_failure(&self, reason: QuarantineReason) {
        self.context.authority.latch_operation_failure(
            &self.context.key,
            self.context.operation_id,
            reason,
        );
    }
}

impl Drop for RegisteredOperation {
    fn drop(&mut self) {
        if !self.released {
            self.context.authority.complete_operation(
                &self.context.key,
                self.context.operation_id,
                OperationRelease::Failed(QuarantineReason::OperationAbandoned),
            );
            self.released = true;
        }
    }
}

/// Couples an owned operation's generation registration to its tracked task.
/// On task abort, operation failure is recorded before the supervisor registry
/// can reach zero.
struct OwnedTaskRegistration {
    operation: Option<RegisteredOperation>,
    supervisor: Option<SupervisorRegistration>,
}

impl OwnedTaskRegistration {
    fn context(&self) -> OperationContext {
        self.operation
            .as_ref()
            .expect("owned operation registration")
            .context()
    }

    fn latch_failure(&self, reason: QuarantineReason) {
        if let Some(operation) = self.operation.as_ref() {
            operation.latch_failure(reason);
        }
    }

    fn finish(&mut self, disposition: CompletionDisposition) {
        if let Some(operation) = self.operation.take() {
            operation.finish(disposition);
        }
    }

    fn fail(&mut self, reason: QuarantineReason) {
        if let Some(operation) = self.operation.take() {
            operation.fail(reason);
        }
    }
}

impl Drop for OwnedTaskRegistration {
    fn drop(&mut self) {
        // RegisteredOperation's drop-bomb must run before the task is removed
        // from the drain-visible supervisor registry.
        drop(self.operation.take());
        drop(self.supervisor.take());
    }
}

/// A synchronous-only operation guard.
///
/// The `Rc` marker deliberately makes this type `!Send`; side-effectful or
/// awaited work must use [`SessionLease::spawn_owned`] so caller cancellation
/// cannot masquerade as successful completion.
pub(crate) struct OperationGuard {
    registered: Option<RegisteredOperation>,
    _not_send: std::marker::PhantomData<Rc<()>>,
}

pub(crate) struct OperationFinishFailure {
    guard: OperationGuard,
    error: SessionOperationError,
}

impl OperationFinishFailure {
    pub(crate) fn error(&self) -> SessionOperationError {
        self.error
    }

    pub(crate) fn into_guard(self) -> OperationGuard {
        self.guard
    }
}

impl std::fmt::Debug for OperationFinishFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OperationFinishFailure")
            .field("error", &self.error)
            .finish()
    }
}

impl OperationGuard {
    #[allow(dead_code)]
    pub(crate) fn key(&self) -> &SessionKey {
        &self.registered.as_ref().expect("live guard").context.key
    }

    #[allow(dead_code)]
    pub(crate) fn operation_id(&self) -> OperationId {
        self.registered
            .as_ref()
            .expect("live guard")
            .context
            .operation_id
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn cancellation(&self) -> Option<watch::Receiver<bool>> {
        self.registered
            .as_ref()
            .expect("live guard")
            .context
            .cancellation()
    }

    pub(crate) fn ensure_current(&self) -> Result<(), SessionOperationError> {
        self.registered
            .as_ref()
            .expect("live guard")
            .context
            .ensure_current()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn prepare_commit(&self) -> Result<CommitPermit, SessionOperationError> {
        self.registered
            .as_ref()
            .expect("live guard")
            .context
            .prepare_commit()
    }

    pub(crate) fn finish(mut self) -> Result<(), OperationFinishFailure> {
        let registered = self.registered.as_ref().expect("live guard");
        let revision = match registered
            .context
            .authority
            .record_operation_commit(&registered.context.key, registered.context.operation_id)
        {
            Ok(revision) => revision,
            Err(error) => return Err(OperationFinishFailure { guard: self, error }),
        };
        self.registered
            .take()
            .expect("live guard")
            .finish(CompletionDisposition::Committed(revision));
        Ok(())
    }

    pub(crate) fn finish_rollback(mut self) {
        self.registered
            .take()
            .expect("live guard")
            .finish(CompletionDisposition::RolledBack);
    }
}

#[derive(Clone)]
pub(crate) struct SessionLease {
    #[cfg_attr(not(test), allow(dead_code))]
    authority: Arc<SessionLeaseAuthority>,
    key: SessionKey,
}

impl SessionLease {
    pub(crate) fn key(&self) -> &SessionKey {
        &self.key
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn is_current(&self) -> bool {
        self.authority.is_current(&self.key)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn try_operation(
        &self,
        kind: SessionOperationKind,
    ) -> Result<OperationGuard, SessionOperationError> {
        self.authority.try_operation_exact(&self.key, kind)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn spawn_owned<T, F, Fut>(
        &self,
        kind: SessionOperationKind,
        hard_timeout: Duration,
        operation: F,
    ) -> Result<OperationWaiter<T>, SessionOperationError>
    where
        T: Send + 'static,
        F: FnOnce(OwnedOperation) -> Fut + Send + 'static,
        Fut: Future<Output = OwnedOperationCompletion<T>> + Send + 'static,
    {
        self.authority
            .spawn_owned(&self.key, kind, hard_timeout, operation)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct OperationDiagnostics {
    pub(crate) total: usize,
    pub(crate) with_hard_deadline: usize,
    pub(crate) state_transition: usize,
    pub(crate) signaling: usize,
    pub(crate) media: usize,
    pub(crate) event_dispatch: usize,
    pub(crate) test: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct QuarantineDiagnostics {
    pub(crate) operation_deadline: usize,
    pub(crate) operation_panicked: usize,
    pub(crate) operation_abandoned: usize,
    pub(crate) committed_after_cancellation: usize,
    pub(crate) supervisor_abandoned: usize,
    pub(crate) quiesce_deadline: usize,
    pub(crate) authority_fatal: usize,
    pub(crate) resource_install_orphaned: usize,
    pub(crate) resource_descriptor_mismatch: usize,
    pub(crate) resource_cancel_panicked: usize,
    pub(crate) resource_release_failed: usize,
    pub(crate) resource_release_panicked: usize,
    pub(crate) resource_release_deadline: usize,
    pub(crate) resource_release_ambiguous: usize,
    pub(crate) resource_dependency_cycle: usize,
    pub(crate) resource_rollback_incomplete: usize,
}

impl QuarantineDiagnostics {
    fn record(&mut self, reason: QuarantineReason) {
        match reason {
            QuarantineReason::OperationDeadline => self.operation_deadline += 1,
            QuarantineReason::OperationPanicked => self.operation_panicked += 1,
            QuarantineReason::OperationAbandoned => self.operation_abandoned += 1,
            QuarantineReason::CommittedAfterCancellation => {
                self.committed_after_cancellation += 1;
            }
            QuarantineReason::SupervisorAbandoned => self.supervisor_abandoned += 1,
            QuarantineReason::QuiesceDeadline => self.quiesce_deadline += 1,
            QuarantineReason::AuthorityFatal => self.authority_fatal += 1,
            QuarantineReason::ResourceInstallOrphaned => self.resource_install_orphaned += 1,
            QuarantineReason::ResourceDescriptorMismatch => {
                self.resource_descriptor_mismatch += 1;
            }
            QuarantineReason::ResourceCancelPanicked => self.resource_cancel_panicked += 1,
            QuarantineReason::ResourceReleaseFailed => self.resource_release_failed += 1,
            QuarantineReason::ResourceReleasePanicked => self.resource_release_panicked += 1,
            QuarantineReason::ResourceReleaseDeadline => self.resource_release_deadline += 1,
            QuarantineReason::ResourceReleaseAmbiguous => self.resource_release_ambiguous += 1,
            QuarantineReason::ResourceDependencyCycle => self.resource_dependency_cycle += 1,
            QuarantineReason::ResourceRollbackIncomplete => {
                self.resource_rollback_incomplete += 1;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct ResourceOrphanDiagnostics {
    pub(crate) install_attempt_dropped: usize,
    pub(crate) dispatch_permit_dropped: usize,
    pub(crate) descriptor_mismatch: usize,
    pub(crate) cancel_panicked: usize,
    pub(crate) release_failed: usize,
    pub(crate) release_panicked: usize,
    pub(crate) release_deadline: usize,
    pub(crate) release_driver_dropped: usize,
    pub(crate) dependency_cycle: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct ResourceDiagnostics {
    /// All registry states remain charged and are included in `total`.
    pub(crate) total: usize,
    pub(crate) reserved: usize,
    pub(crate) installing: usize,
    pub(crate) captured: usize,
    pub(crate) live: usize,
    pub(crate) releasing: usize,
    pub(crate) orphaned: usize,
    pub(crate) oldest_orphan_age: Option<Duration>,
    pub(crate) orphan_reasons: ResourceOrphanDiagnostics,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct SessionLifecycleDiagnostics {
    /// False means a poisoned authority prevented a complete snapshot.
    pub(crate) complete: bool,
    pub(crate) fatal_reason: Option<AuthorityFatalReason>,
    pub(crate) capacity: usize,
    pub(crate) active_capacity_in_use: usize,
    pub(crate) retained_capacity: usize,
    pub(crate) resource_capacity_per_session: usize,
    pub(crate) lifecycle_count: usize,
    pub(crate) admission_index_capacity: usize,
    pub(crate) reusable_deadline_capacity: usize,
    pub(crate) retained_identifier_payload_bytes: usize,
    pub(crate) current_index_capacity: usize,
    pub(crate) exact_cell_index_capacity: usize,
    pub(crate) index_live: usize,
    pub(crate) index_blocked: usize,
    pub(crate) active: usize,
    pub(crate) quiescing: usize,
    pub(crate) releasing: usize,
    pub(crate) quarantined: usize,
    pub(crate) retired: usize,
    pub(crate) oldest_quarantine_age: Option<Duration>,
    pub(crate) oldest_retired_age: Option<Duration>,
    pub(crate) operations: OperationDiagnostics,
    pub(crate) resources: ResourceDiagnostics,
    pub(crate) quarantine_reasons: QuarantineDiagnostics,
    pub(crate) active_supervisors: usize,
    pub(crate) owned_operation_supervisors: usize,
    pub(crate) teardown_supervisors: usize,
}

pub(crate) struct SessionLeaseAuthority {
    epoch: AuthorityEpoch,
    index: StdMutex<AdmissionIndex>,
    /// Test instrumentation proving that exact-lifetime hot paths do not
    /// regress into raw-ID admission serialization.
    #[cfg(test)]
    admission_index_lock_acquisitions: AtomicU64,
    /// Lock-free raw-ID lookup for the currently active generation. The
    /// admission index remains authoritative for reuse and retained fences,
    /// but ordinary registry resolution never serializes on its mutex.
    current: DashMap<SessionId, SessionKey>,
    cells: DashMap<SessionKey, Arc<SessionCell>>,
    active_slots: Arc<Semaphore>,
    next_generation: AtomicU64,
    next_supervisor: AtomicU64,
    supervisors: StdMutex<HashMap<SupervisorId, SupervisorEntry>>,
    supervisors_changed: Notify,
    config: SessionLifecycleConfig,
    clock: Arc<dyn LifecycleClock>,
    reuse_pruner_started: AtomicBool,
    reuse_pruner_changed: Arc<Notify>,
    fatal: AtomicU8,
    fatal_changed: Notify,
    draining: AtomicBool,
}

impl Drop for SessionLeaseAuthority {
    fn drop(&mut self) {
        // The deadline task holds only a weak authority reference. Wake an
        // idle task so it can observe final authority destruction promptly.
        self.reuse_pruner_changed.notify_waiters();
    }
}

impl SessionLeaseAuthority {
    pub(crate) fn new() -> Arc<Self> {
        Self::with_config(
            AuthorityEpoch::random(),
            SessionLifecycleConfig::default(),
            Arc::new(SystemLifecycleClock),
        )
    }

    /// Construct the process authority with an explicit active-lifecycle
    /// capacity while retaining separately bounded anti-reuse fences and the
    /// production supervisor defaults. Runtime coordinators use this instead
    /// of maintaining a second active-capacity counter in their session store.
    pub(crate) fn with_capacity(capacity: usize) -> Arc<Self> {
        let config = SessionLifecycleConfig::new(capacity.max(1), DEFAULT_ANTI_REUSE_HORIZON)
            .expect("normalized lifecycle capacity and production horizon are valid");
        Self::with_config(
            AuthorityEpoch::random(),
            config,
            Arc::new(SystemLifecycleClock),
        )
    }

    /// Construct an authority with independently bounded active lifetimes and
    /// retained anti-reuse fences.
    ///
    /// Active call concurrency and completed-call churn are different sizing
    /// dimensions. High-CPS servers can retain many more retired identifiers
    /// during the SIP anti-reuse horizon than they have simultaneous active
    /// calls, so callers that know their workload may configure that bound
    /// explicitly without changing the conservative library default.
    pub(crate) fn with_capacities(
        active_capacity: usize,
        retained_capacity: usize,
    ) -> Result<Arc<Self>, SessionLifecycleConfigError> {
        let config = SessionLifecycleConfig::new(active_capacity, DEFAULT_ANTI_REUSE_HORIZON)?
            .with_retained_capacity(retained_capacity)?;
        Ok(Self::with_config(
            AuthorityEpoch::random(),
            config,
            Arc::new(SystemLifecycleClock),
        ))
    }

    fn with_config(
        epoch: AuthorityEpoch,
        config: SessionLifecycleConfig,
        clock: Arc<dyn LifecycleClock>,
    ) -> Arc<Self> {
        let capacity = config.capacity.get();
        let initial_active_index_capacity = eager_active_lifetime_index_capacity(capacity);
        Arc::new(Self {
            epoch,
            // The admission index enforces the complete logical capacity, but
            // it does not need one eagerly allocated bucket for every possible
            // active or retained lifetime. Keep its warm reserve aligned with
            // the two sharded exact-lifetime indexes and grow from observed
            // concurrency/churn.
            index: StdMutex::new(AdmissionIndex::with_capacity(initial_active_index_capacity)),
            #[cfg(test)]
            admission_index_lock_acquisitions: AtomicU64::new(0),
            current: DashMap::with_capacity(initial_active_index_capacity),
            cells: DashMap::with_capacity(initial_active_index_capacity),
            active_slots: Arc::new(Semaphore::new(capacity)),
            next_generation: AtomicU64::new(1),
            next_supervisor: AtomicU64::new(1),
            supervisors: StdMutex::new(HashMap::new()),
            supervisors_changed: Notify::new(),
            config,
            clock,
            reuse_pruner_started: AtomicBool::new(false),
            reuse_pruner_changed: Arc::new(Notify::new()),
            fatal: AtomicU8::new(0),
            fatal_changed: Notify::new(),
            draining: AtomicBool::new(false),
        })
    }

    fn fatal_reason(&self) -> Option<AuthorityFatalReason> {
        AuthorityFatalReason::from_u8(self.fatal.load(Ordering::Acquire))
    }

    fn latch_fatal(&self, reason: AuthorityFatalReason) -> AuthorityFatalReason {
        let _ = self
            .fatal
            .compare_exchange(0, reason as u8, Ordering::AcqRel, Ordering::Acquire);
        // Cancellation senders are independent of the possibly poisoned
        // authority mutexes. Broadcast best-effort before waking diagnostics
        // and waiters so retained creators stop external side effects quickly.
        for cell in self.cells.iter() {
            cell.cancel.send_replace(true);
            cell.changed.notify_waiters();
        }
        self.fatal_changed.notify_waiters();
        self.fatal_reason().unwrap_or(reason)
    }

    fn require_healthy(&self) -> Result<(), AuthorityFatalReason> {
        self.fatal_reason().map_or(Ok(()), Err)
    }

    fn lock_index(&self) -> Result<StdMutexGuard<'_, AdmissionIndex>, AuthorityFatalReason> {
        self.require_healthy()?;
        #[cfg(test)]
        self.admission_index_lock_acquisitions
            .fetch_add(1, Ordering::Relaxed);
        self.index
            .lock()
            .map_err(|_| self.latch_fatal(AuthorityFatalReason::IndexPoisoned))
    }

    #[cfg(test)]
    fn admission_index_lock_acquisitions(&self) -> u64 {
        self.admission_index_lock_acquisitions
            .load(Ordering::Relaxed)
    }

    fn lock_cell<'a>(
        &self,
        cell: &'a SessionCell,
    ) -> Result<StdMutexGuard<'a, CellState>, AuthorityFatalReason> {
        self.require_healthy()?;
        cell.state
            .lock()
            .map_err(|_| self.latch_fatal(AuthorityFatalReason::CellPoisoned))
    }

    /// Fence one exact generation from new raw-ID lookups while the caller
    /// holds the admission index. Exact in-flight operations remain attached
    /// to their cell and finish under that cell's lock.
    fn block_non_reusable_locked(&self, index: &mut AdmissionIndex, key: &SessionKey) {
        self.current
            .remove_if(&key.session_id, |_, current| current == key);
        index.block_non_reusable(key.clone());
    }

    fn lock_supervisors(
        &self,
    ) -> Result<StdMutexGuard<'_, HashMap<SupervisorId, SupervisorEntry>>, AuthorityFatalReason>
    {
        self.require_healthy()?;
        self.lock_supervisors_for_cleanup()
    }

    /// Cleanup must remain able to abort and account for tasks when an
    /// unrelated index/cell invariant has already failed closed.
    fn lock_supervisors_for_cleanup(
        &self,
    ) -> Result<StdMutexGuard<'_, HashMap<SupervisorId, SupervisorEntry>>, AuthorityFatalReason>
    {
        self.supervisors
            .lock()
            .map_err(|_| self.latch_fatal(AuthorityFatalReason::SupervisorRegistryPoisoned))
    }

    fn operation_deadline(&self, timeout: Duration) -> Result<Instant, SessionOperationError> {
        if timeout.is_zero() {
            return Err(SessionOperationError::InvalidTimeout);
        }
        self.require_healthy()
            .map_err(SessionOperationError::AuthorityFatal)?;
        self.clock
            .now()
            .checked_add(timeout)
            .ok_or(SessionOperationError::DeadlineOverflow)
    }

    fn retirement_deadline(&self, retired_at: Instant) -> Result<Instant, AuthorityFatalReason> {
        retired_at
            .checked_add(self.config.anti_reuse_horizon)
            .ok_or_else(|| self.latch_fatal(AuthorityFatalReason::RetirementDeadlineOverflow))
    }

    /// Start the one authority-owned retained-fence deadline worker.
    ///
    /// Construction and some compatibility APIs may run outside Tokio, so a
    /// failed start is deliberately retryable from the next retirement.
    /// Admission still performs an exact synchronous purge as a fail-safe;
    /// the worker exists to make retained storage converge during otherwise
    /// idle retention windows.
    fn ensure_reuse_pruner(self: &Arc<Self>) {
        if self
            .reuse_pruner_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }

        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            self.reuse_pruner_started.store(false, Ordering::Release);
            return;
        };

        let authority = Arc::downgrade(self);
        let changed = Arc::clone(&self.reuse_pruner_changed);
        let clock = Arc::clone(&self.clock);
        runtime.spawn(Self::run_reuse_pruner(authority, changed, clock));
    }

    async fn run_reuse_pruner(
        authority: Weak<Self>,
        changed: Arc<Notify>,
        clock: Arc<dyn LifecycleClock>,
    ) {
        loop {
            // Register the wake before inspecting the heap. A retirement that
            // races this inspection then either appears in the heap or leaves
            // a stored permit on this notification.
            let notified = changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            let Some(authority) = authority.upgrade() else {
                return;
            };
            let now = clock.now();
            let next_deadline = {
                let mut index = match authority.lock_index() {
                    Ok(index) => index,
                    Err(_) => return,
                };
                if authority
                    .purge_reusable_locked_bounded(&mut index, now, REUSABLE_FENCE_EXPIRY_BATCH)
                    .is_err()
                {
                    return;
                }
                index
                    .reusable_deadlines
                    .peek()
                    .map(|deadline| deadline.reusable_at)
            };
            drop(authority);

            match next_deadline {
                // A bounded batch left more already-due work. Yield once so
                // signaling work can run, then immediately drain another
                // bounded wave without waiting for external traffic.
                Some(deadline) if deadline <= now => tokio::task::yield_now().await,
                Some(deadline) => {
                    let sleep = clock.sleep_until(deadline);
                    tokio::pin!(sleep);
                    tokio::select! {
                        () = &mut sleep => {}
                        () = &mut notified => {}
                    }
                }
                None => notified.await,
            }
        }
    }

    #[cfg(test)]
    fn purge_reusable_locked(
        &self,
        index: &mut AdmissionIndex,
        now: Instant,
    ) -> Result<(), AuthorityFatalReason> {
        self.purge_reusable_locked_bounded(index, now, usize::MAX)
    }

    fn purge_reusable_locked_bounded(
        &self,
        index: &mut AdmissionIndex,
        now: Instant,
        limit: usize,
    ) -> Result<(), AuthorityFatalReason> {
        let had_reusable_deadlines = !index.reusable_deadlines.is_empty();
        let mut inspected = 0_usize;
        loop {
            if inspected >= limit {
                break;
            }
            let Some(reusable_at) = index
                .reusable_deadlines
                .peek()
                .map(|deadline| deadline.reusable_at)
            else {
                break;
            };
            #[cfg(test)]
            {
                index.reuse_deadline_inspections =
                    index.reuse_deadline_inspections.saturating_add(1);
            }
            if now < reusable_at {
                break;
            }
            let deadline = index
                .reusable_deadlines
                .pop()
                .expect("peeked reusable deadline must remain present");
            inspected = inspected.saturating_add(1);
            let exact_slot = matches!(
                index.slots.get(deadline.session_id.as_ref()),
                Some(AdmissionSlot::Retired {
                    generation,
                    reusable_at,
                    ..
                }) if *generation == deadline.generation && *reusable_at == deadline.reusable_at
            );
            if !exact_slot {
                continue;
            }
            let exact_key = SessionKey {
                session_id: deadline.session_id.as_ref().clone(),
                generation: deadline.generation,
            };
            if self.cells.contains_key(&exact_key) {
                // A retired fence is published only after the exact cell has
                // reached Retired, every operation/resource is gone, waiter
                // outcomes are visible, and the cell is removed. Any exact
                // cell still registered under that compact fence is therefore
                // an impossible split-brain lifetime. Fail closed rather than
                // make the public identifier reusable.
                return Err(self.latch_fatal(AuthorityFatalReason::InvariantViolation));
            }
            index.slots.remove(deadline.session_id.as_ref());
        }
        if had_reusable_deadlines && index.reusable_deadlines.is_empty() {
            // The logical retention bound must not become a permanent heap or
            // hash-table reservation after an idle horizon. Preserve only the
            // bounded eager working set; the logical admission limit remains
            // independently enforced by the semaphore and retained-capacity
            // check.
            if index.reusable_deadlines.capacity() > REUSABLE_FENCE_EXPIRY_BATCH {
                index.reusable_deadlines.shrink_to_fit();
            }
            index.slots.shrink_to(eager_active_lifetime_index_capacity(
                self.config.capacity.get(),
            ));

            // Reclaim sharded-map high water only at a truly idle generation
            // boundary. Shrinking a live map would trade retained memory for
            // allocation churn on the signaling hot path; when all three
            // indexes are empty there is no exact lifetime that can be delayed
            // or redirected by the shard-local rehash.
            if index.slots.is_empty() && self.current.is_empty() && self.cells.is_empty() {
                if self.current.capacity() > MAX_EAGER_ACTIVE_LIFETIME_INDEX_CAPACITY {
                    self.current.shrink_to_fit();
                }
                if self.cells.capacity() > MAX_EAGER_ACTIVE_LIFETIME_INDEX_CAPACITY {
                    self.cells.shrink_to_fit();
                }
            }
        }
        Ok(())
    }

    /// Remove only the exact full cell whose terminal retirement was just
    /// published. The index lock is held by every caller, so admission cannot
    /// observe an incomplete transition between the full and compact forms.
    fn remove_exact_retired_cell(
        &self,
        key: &SessionKey,
        expected: &Arc<SessionCell>,
    ) -> Result<(), AuthorityFatalReason> {
        let Some((_, removed)) = self.cells.remove(key) else {
            return Err(self.latch_fatal(AuthorityFatalReason::InvariantViolation));
        };
        if !Arc::ptr_eq(&removed, expected) {
            return Err(self.latch_fatal(AuthorityFatalReason::InvariantViolation));
        }
        Ok(())
    }

    fn allocate_generation(&self) -> Result<SessionGeneration, SessionAdmissionError> {
        let sequence = self
            .next_generation
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .map_err(|_| SessionAdmissionError::GenerationExhausted)?;
        let sequence =
            NonZeroU64::new(sequence).ok_or(SessionAdmissionError::GenerationExhausted)?;
        Ok(SessionGeneration {
            authority_epoch: self.epoch,
            sequence,
        })
    }

    pub(crate) fn admit(
        self: &Arc<Self>,
        session_id: SessionId,
    ) -> Result<SessionLease, SessionAdmissionError> {
        if self.draining.load(Ordering::Acquire) {
            return Err(SessionAdmissionError::AuthorityDraining);
        }
        let now = self.clock.now();
        let mut index = self
            .lock_index()
            .map_err(SessionAdmissionError::AuthorityFatal)?;
        if self.draining.load(Ordering::Acquire) {
            return Err(SessionAdmissionError::AuthorityDraining);
        }
        self.purge_reusable_locked_bounded(&mut index, now, REUSABLE_FENCE_EXPIRY_BATCH)
            .map_err(SessionAdmissionError::AuthorityFatal)?;
        let due_work_remains = index
            .reusable_deadlines
            .peek()
            .is_some_and(|deadline| deadline.reusable_at <= now);
        if due_work_remains {
            // Admission is a signaling hot path and must never inherit an
            // entire same-horizon retirement burst. The authority worker
            // drains subsequent bounded waves; this stored notification also
            // covers a worker that has not started yet.
            self.ensure_reuse_pruner();
            self.reuse_pruner_changed.notify_one();
        }

        if let Some(slot) = index.get(&session_id).cloned() {
            match slot {
                AdmissionSlot::Live(_) => return Err(SessionAdmissionError::AlreadyActive),
                AdmissionSlot::NonReusable(_) => return Err(SessionAdmissionError::ReuseBlocked),
                AdmissionSlot::Retired {
                    generation,
                    reusable_at,
                    ..
                } => {
                    if now < reusable_at {
                        return Err(SessionAdmissionError::ReuseBlocked);
                    }
                    let exact_key = SessionKey {
                        session_id: session_id.clone(),
                        generation,
                    };
                    if self.cells.contains_key(&exact_key) {
                        return Err(SessionAdmissionError::AuthorityFatal(
                            self.latch_fatal(AuthorityFatalReason::InvariantViolation),
                        ));
                    }
                    index.remove(&session_id);
                }
            }
        }

        if index.len() >= self.config.retained_capacity.get() {
            return Err(SessionAdmissionError::RetainedCapacityExhausted);
        }
        let active_permit = Arc::clone(&self.active_slots)
            .try_acquire_owned()
            .map_err(|_| SessionAdmissionError::CapacityExhausted)?;
        let key = SessionKey {
            session_id: session_id.clone(),
            generation: self.allocate_generation()?,
        };
        self.cells
            .insert(key.clone(), Arc::new(SessionCell::active(active_permit)));
        index.insert(
            Arc::new(session_id.clone()),
            AdmissionSlot::Live(key.clone()),
        );
        match self.current.entry(session_id.clone()) {
            DashEntry::Vacant(entry) => {
                entry.insert(key.clone());
            }
            DashEntry::Occupied(_) => {
                index.remove(&session_id);
                self.cells.remove(&key);
                return Err(SessionAdmissionError::AuthorityFatal(
                    self.latch_fatal(AuthorityFatalReason::InvariantViolation),
                ));
            }
        }
        Ok(SessionLease {
            authority: Arc::clone(self),
            key,
        })
    }

    pub(crate) fn current_key(&self, session_id: &SessionId) -> Option<SessionKey> {
        let key = self.current.get(session_id).map(|entry| entry.clone())?;
        let cell = self.cells.get(&key).map(|cell| Arc::clone(cell.value()))?;
        let active = {
            let state = self.lock_cell(&cell).ok()?;
            matches!(state.phase, SessionPhase::Active)
        };
        active.then_some(key)
    }

    /// Test-only clock seam for exercising raw-ID reuse without sleeping for
    /// the production anti-reuse horizon. This changes only the retirement
    /// deadline owned by this authority; admission still performs its normal
    /// retired-phase validation and exact cell removal.
    #[cfg(test)]
    pub(crate) fn elapse_reuse_horizon_for_test(self: &Arc<Self>, session_id: &SessionId) -> bool {
        let now = self.clock.now();
        let Ok(mut index) = self.lock_index() else {
            return false;
        };
        let Some((generation, retired_at)) = index.get(session_id).and_then(|slot| match slot {
            AdmissionSlot::Retired {
                generation,
                retired_at,
                ..
            } => Some((*generation, *retired_at)),
            AdmissionSlot::Live(_) | AdmissionSlot::NonReusable(_) => None,
        }) else {
            return false;
        };
        let reported_quarantine = match index.get(session_id) {
            Some(AdmissionSlot::Retired {
                reported_quarantine,
                ..
            }) => *reported_quarantine,
            Some(AdmissionSlot::Live(_)) | Some(AdmissionSlot::NonReusable(_)) | None => {
                return false
            }
        };
        let key = SessionKey {
            session_id: session_id.clone(),
            generation,
        };
        let advances_next_deadline = index.retire_until(key, retired_at, now, reported_quarantine);
        drop(index);
        if advances_next_deadline || !self.reuse_pruner_started.load(Ordering::Acquire) {
            self.ensure_reuse_pruner();
        }
        if advances_next_deadline {
            self.reuse_pruner_changed.notify_one();
        }
        true
    }

    pub(crate) fn is_current(&self, key: &SessionKey) -> bool {
        let Some(cell) = self.cells.get(key).map(|cell| Arc::clone(cell.value())) else {
            return false;
        };
        self.lock_cell(&cell)
            .is_ok_and(|state| matches!(state.phase, SessionPhase::Active))
    }

    pub(crate) fn phase(&self, key: &SessionKey) -> Option<SessionPhase> {
        // Exact, live generations are resolved directly.  The admission index
        // is only needed after the heavy cell has retired into its compact
        // anti-reuse fence.
        if let Some(cell) = self.cells.get(key).map(|cell| Arc::clone(cell.value())) {
            return self.lock_cell(&cell).ok().map(|state| state.phase);
        }
        let index = self.lock_index().ok()?;
        match index.get(&key.session_id) {
            Some(AdmissionSlot::Retired {
                generation,
                retired_at,
                ..
            }) if *generation == key.generation => {
                return Some(SessionPhase::Retired {
                    retired_at: *retired_at,
                });
            }
            Some(AdmissionSlot::Live(_))
            | Some(AdmissionSlot::NonReusable(_))
            | Some(AdmissionSlot::Retired { .. })
            | None => {
                return None;
            }
        }
    }

    /// Acquire a non-forgeable synchronous ownership guard for this exact
    /// generation. The generation-qualified cell is the synchronization
    /// point, so ordinary exact operations never serialize on the raw-ID
    /// admission index.
    pub(crate) fn try_operation_exact(
        self: &Arc<Self>,
        key: &SessionKey,
        kind: SessionOperationKind,
    ) -> Result<OperationGuard, SessionOperationError> {
        self.try_operation(key, kind, None)
    }

    /// Spawn retained asynchronous work for an already captured exact
    /// lifetime. Unlike a raw-ID lookup, this can never redirect delayed work
    /// to a later admission that reused the same application identifier.
    pub(crate) fn spawn_owned_exact<T, F, Fut>(
        self: &Arc<Self>,
        key: &SessionKey,
        kind: SessionOperationKind,
        hard_timeout: Duration,
        operation: F,
    ) -> Result<OperationWaiter<T>, SessionOperationError>
    where
        T: Send + 'static,
        F: FnOnce(OwnedOperation) -> Fut + Send + 'static,
        Fut: Future<Output = OwnedOperationCompletion<T>> + Send + 'static,
    {
        self.spawn_owned(key, kind, hard_timeout, operation)
    }

    #[cfg(test)]
    pub(crate) fn retire(self: &Arc<Self>, key: &SessionKey) -> Result<(), SessionAdmissionError> {
        let now = self.clock.now();
        let reusable_at = self
            .retirement_deadline(now)
            .map_err(SessionAdmissionError::AuthorityFatal)?;
        let mut index = self
            .lock_index()
            .map_err(SessionAdmissionError::AuthorityFatal)?;
        match index.get(&key.session_id) {
            Some(AdmissionSlot::Live(current)) if current == key => {}
            _ => return Err(SessionAdmissionError::StaleGeneration),
        }
        let Some(cell) = self.cells.get(key).map(|cell| Arc::clone(cell.value())) else {
            return Err(SessionAdmissionError::StaleGeneration);
        };
        let mut state = self
            .lock_cell(&cell)
            .map_err(SessionAdmissionError::AuthorityFatal)?;
        if !matches!(state.phase, SessionPhase::Active)
            || !state.operations.is_empty()
            || !state.resources.is_empty()
        {
            return Err(SessionAdmissionError::StaleGeneration);
        }
        state.phase = SessionPhase::Retired { retired_at: now };
        state.active_permit.take();
        drop(state);
        self.current
            .remove_if(&key.session_id, |_, current| current == key);
        let advances_next_deadline = index.retire_until(key.clone(), now, reusable_at, None);
        self.remove_exact_retired_cell(key, &cell)
            .map_err(SessionAdmissionError::AuthorityFatal)?;
        drop(index);
        if advances_next_deadline || !self.reuse_pruner_started.load(Ordering::Acquire) {
            self.ensure_reuse_pruner();
        }
        if advances_next_deadline {
            self.reuse_pruner_changed.notify_one();
        }
        Ok(())
    }

    fn register_operation(
        self: &Arc<Self>,
        key: &SessionKey,
        kind: SessionOperationKind,
        hard_deadline: Option<Instant>,
    ) -> Result<RegisteredOperation, SessionOperationError> {
        let cell = self
            .cells
            .get(key)
            .map(|cell| Arc::clone(cell.value()))
            .ok_or(SessionOperationError::StaleGeneration)?;
        let mut state = self
            .lock_cell(&cell)
            .map_err(SessionOperationError::AuthorityFatal)?;
        if !matches!(state.phase, SessionPhase::Active) {
            // Preserve the former admission-index semantics: once teardown
            // has fenced this generation, it is stale for new operations.
            return Err(SessionOperationError::StaleGeneration);
        }
        let operation_id = NonZeroU64::new(state.next_operation)
            .map(OperationId)
            .ok_or(SessionOperationError::OperationSequenceExhausted)?;
        state.next_operation = state
            .next_operation
            .checked_add(1)
            .ok_or(SessionOperationError::OperationSequenceExhausted)?;
        state.operations.insert(
            operation_id,
            OperationMeta {
                kind,
                hard_deadline,
                commit_revision: None,
                resource_ids: HashSet::new(),
            },
        );
        drop(state);
        Ok(RegisteredOperation {
            context: OperationContext {
                authority: Arc::clone(self),
                key: key.clone(),
                operation_id,
            },
            released: false,
        })
    }

    fn try_operation(
        self: &Arc<Self>,
        key: &SessionKey,
        kind: SessionOperationKind,
        hard_deadline: Option<Instant>,
    ) -> Result<OperationGuard, SessionOperationError> {
        Ok(OperationGuard {
            registered: Some(self.register_operation(key, kind, hard_deadline)?),
            _not_send: std::marker::PhantomData,
        })
    }

    fn ensure_operation_current(
        &self,
        key: &SessionKey,
        operation_id: OperationId,
    ) -> Result<(), SessionOperationError> {
        let cell = self
            .cells
            .get(key)
            .map(|cell| Arc::clone(cell.value()))
            .ok_or(SessionOperationError::StaleGeneration)?;
        let state = self
            .lock_cell(&cell)
            .map_err(SessionOperationError::AuthorityFatal)?;
        if !matches!(state.phase, SessionPhase::Active) {
            return Err(SessionOperationError::StaleGeneration);
        }
        state
            .operations
            .contains_key(&operation_id)
            .then_some(())
            .ok_or(SessionOperationError::StaleGeneration)
    }

    fn reserve_resource(
        self: &Arc<Self>,
        key: &SessionKey,
        operation_id: OperationId,
        spec: ResourceSpec,
    ) -> Result<ResourceInstallAttempt, ResourceRegistryError> {
        let now = self.clock.now();
        // The generation-qualified cell is the authority for ordinary active
        // resource mutations. Teardown takes this same cell lock before it
        // changes Active, and structural removal cannot occur while an
        // operation/resource remains registered. The raw-ID admission index
        // is therefore neither needed nor allowed on this hot path.
        let cell = self
            .cells
            .get(key)
            .map(|cell| Arc::clone(cell.value()))
            .ok_or(ResourceRegistryError::StaleGeneration)?;
        let mut state = self
            .lock_cell(&cell)
            .map_err(ResourceRegistryError::AuthorityFatal)?;
        if !matches!(state.phase, SessionPhase::Active) {
            return Err(ResourceRegistryError::NotActive);
        }
        if !state.operations.contains_key(&operation_id) {
            return Err(ResourceRegistryError::StaleGeneration);
        }
        if state.resources.len() >= self.config.resource_capacity_per_session.get() {
            return Err(ResourceRegistryError::CapacityExhausted);
        }
        // An operation may depend only on its own already-captured resources
        // or on committed live resources. It may never observe another
        // operation's staged resource: otherwise that owner could roll back a
        // parent beneath a committed child.
        for dependency in &spec.release_dependencies {
            let Some(entry) = state.resources.get(dependency) else {
                return Err(ResourceRegistryError::UnknownDependency);
            };
            let valid = (entry.owner_operation == Some(operation_id)
                && matches!(entry.state, ManagedResourceState::Captured(_)))
                || (entry.owner_operation.is_none()
                    && matches!(entry.state, ManagedResourceState::Live(_)));
            if !valid {
                return Err(ResourceRegistryError::InvalidDependencyState);
            }
        }
        let resource_id = NonZeroU64::new(state.next_resource)
            .map(ResourceId)
            .ok_or(ResourceRegistryError::ResourceSequenceExhausted)?;
        state.next_resource = state
            .next_resource
            .checked_add(1)
            .ok_or(ResourceRegistryError::ResourceSequenceExhausted)?;
        state.resources.insert(
            resource_id,
            ManagedResourceEntry {
                spec,
                owner_operation: Some(operation_id),
                state: ManagedResourceState::Reserved,
                state_since: now,
            },
        );
        let Some(operation) = state.operations.get_mut(&operation_id) else {
            let reason = self.latch_fatal(AuthorityFatalReason::InvariantViolation);
            return Err(ResourceRegistryError::AuthorityFatal(reason));
        };
        operation.resource_ids.insert(resource_id);
        Ok(ResourceInstallAttempt {
            authority: Arc::clone(self),
            key: key.clone(),
            operation_id,
            resource_id,
            armed: true,
        })
    }

    fn transition_resource_installing(
        &self,
        key: &SessionKey,
        operation_id: OperationId,
        resource_id: ResourceId,
    ) -> Result<(), ResourceRegistryError> {
        let now = self.clock.now();
        let cell = self
            .cells
            .get(key)
            .map(|cell| Arc::clone(cell.value()))
            .ok_or(ResourceRegistryError::StaleGeneration)?;
        let mut state = self
            .lock_cell(&cell)
            .map_err(ResourceRegistryError::AuthorityFatal)?;
        if !matches!(state.phase, SessionPhase::Active) {
            return Err(ResourceRegistryError::NotActive);
        }
        let Some(entry) = state.resources.get_mut(&resource_id) else {
            return Err(ResourceRegistryError::InvalidState);
        };
        if entry.owner_operation != Some(operation_id)
            || !matches!(entry.state, ManagedResourceState::Reserved)
        {
            return Err(ResourceRegistryError::InvalidState);
        }
        entry.state = ManagedResourceState::Installing;
        entry.state_since = now;
        Ok(())
    }

    fn capture_resource(
        &self,
        key: &SessionKey,
        operation_id: OperationId,
        resource_id: ResourceId,
        resource: Arc<dyn ManagedSessionResource>,
    ) -> Result<(), ResourceRegistryError> {
        let now = self.clock.now();
        let actual_descriptor =
            std::panic::catch_unwind(AssertUnwindSafe(|| resource.descriptor())).ok();
        let cell = self
            .cells
            .get(key)
            .map(|cell| Arc::clone(cell.value()))
            .ok_or(ResourceRegistryError::StaleGeneration)?;
        let mut state = self
            .lock_cell(&cell)
            .map_err(ResourceRegistryError::AuthorityFatal)?;
        let Some(entry) = state.resources.get_mut(&resource_id) else {
            return Err(ResourceRegistryError::InvalidState);
        };
        if entry.owner_operation != Some(operation_id)
            || !matches!(entry.state, ManagedResourceState::Installing)
        {
            return Err(ResourceRegistryError::InvalidState);
        }
        if actual_descriptor.as_ref() != Some(&entry.spec.descriptor) {
            // Descriptor mismatch changes the raw-ID admission fence. Drop
            // the cell lock and reacquire in the structural index->cell order,
            // then revalidate the exact entry before publishing quarantine.
            drop(state);
            let mut index = self
                .lock_index()
                .map_err(ResourceRegistryError::AuthorityFatal)?;
            if !index
                .get(&key.session_id)
                .is_some_and(|slot| slot.retains_exact_cell(key))
            {
                return Err(ResourceRegistryError::StaleGeneration);
            }
            let current_cell = self
                .cells
                .get(key)
                .map(|entry| Arc::clone(entry.value()))
                .ok_or(ResourceRegistryError::StaleGeneration)?;
            if !Arc::ptr_eq(&cell, &current_cell) {
                let reason = self.latch_fatal(AuthorityFatalReason::InvariantViolation);
                return Err(ResourceRegistryError::AuthorityFatal(reason));
            }
            let mut state = self
                .lock_cell(&cell)
                .map_err(ResourceRegistryError::AuthorityFatal)?;
            let Some(entry) = state.resources.get_mut(&resource_id) else {
                return Err(ResourceRegistryError::InvalidState);
            };
            if entry.owner_operation != Some(operation_id)
                || !matches!(entry.state, ManagedResourceState::Installing)
            {
                return Err(ResourceRegistryError::InvalidState);
            }
            entry.state = ManagedResourceState::Orphaned {
                resource: Some(resource),
                reason: ResourceOrphanReason::DescriptorMismatch,
                retry_ready: true,
            };
            entry.state_since = now;
            state.sticky_failure = Some(merge_quarantine_reason(
                state.sticky_failure,
                QuarantineReason::ResourceDescriptorMismatch,
            ));
            state.phase = SessionPhase::Quarantined { since: now };
            self.block_non_reusable_locked(&mut index, key);
            drop(state);
            drop(index);
            cell.cancel.send_replace(true);
            cell.changed.notify_waiters();
            return Err(ResourceRegistryError::DescriptorMismatch);
        }
        entry.state = ManagedResourceState::Captured(resource);
        entry.state_since = now;
        drop(state);
        cell.changed.notify_waiters();
        Ok(())
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn confirm_resource_unused(
        &self,
        key: &SessionKey,
        operation_id: OperationId,
        resource_id: ResourceId,
    ) -> Result<(), ResourceRegistryError> {
        let cell = self
            .cells
            .get(key)
            .map(|cell| Arc::clone(cell.value()))
            .ok_or(ResourceRegistryError::StaleGeneration)?;
        let mut state = self
            .lock_cell(&cell)
            .map_err(ResourceRegistryError::AuthorityFatal)?;
        let Some(entry) = state.resources.get(&resource_id) else {
            return Err(ResourceRegistryError::InvalidState);
        };
        if entry.owner_operation != Some(operation_id)
            || !matches!(
                entry.state,
                ManagedResourceState::Reserved | ManagedResourceState::Installing
            )
        {
            return Err(ResourceRegistryError::InvalidState);
        }
        state.resources.remove(&resource_id);
        let Some(operation) = state.operations.get_mut(&operation_id) else {
            let reason = self.latch_fatal(AuthorityFatalReason::InvariantViolation);
            return Err(ResourceRegistryError::AuthorityFatal(reason));
        };
        operation.resource_ids.remove(&resource_id);
        drop(state);
        cell.changed.notify_waiters();
        Ok(())
    }

    fn orphan_install_token(
        &self,
        key: &SessionKey,
        operation_id: OperationId,
        resource_id: ResourceId,
        orphan_reason: ResourceOrphanReason,
    ) {
        let now = self.clock.now();
        let Ok(mut index) = self.lock_index() else {
            return;
        };
        let exact_slot = index
            .get(&key.session_id)
            .is_some_and(|slot| slot.retains_exact_cell(key));
        if !exact_slot {
            return;
        }
        let Some(cell) = self.cells.get(key).map(|cell| Arc::clone(cell.value())) else {
            self.latch_fatal(AuthorityFatalReason::InvariantViolation);
            return;
        };
        let Ok(mut state) = self.lock_cell(&cell) else {
            return;
        };
        let Some(entry) = state.resources.get_mut(&resource_id) else {
            return;
        };
        if entry.owner_operation != Some(operation_id)
            || !matches!(
                entry.state,
                ManagedResourceState::Reserved | ManagedResourceState::Installing
            )
        {
            return;
        }
        entry.state = ManagedResourceState::Orphaned {
            resource: None,
            reason: orphan_reason,
            retry_ready: false,
        };
        entry.state_since = now;
        let reason = merge_quarantine_reason(
            state.sticky_failure,
            QuarantineReason::ResourceInstallOrphaned,
        );
        state.sticky_failure = Some(reason);
        state.phase = SessionPhase::Quarantined { since: now };
        self.block_non_reusable_locked(&mut index, key);
        if let Some(control) = state.teardown.as_ref() {
            control
                .result
                .send_replace(Some(TeardownOutcome::Quarantined {
                    key: key.clone(),
                    reason,
                }));
        }
        drop(state);
        drop(index);
        cell.cancel.send_replace(true);
        cell.changed.notify_waiters();
    }

    fn resource_quarantine_reason(orphan: ResourceOrphanReason) -> QuarantineReason {
        match orphan {
            ResourceOrphanReason::InstallAttemptDropped
            | ResourceOrphanReason::DispatchPermitDropped => {
                QuarantineReason::ResourceInstallOrphaned
            }
            ResourceOrphanReason::DescriptorMismatch => {
                QuarantineReason::ResourceDescriptorMismatch
            }
            ResourceOrphanReason::CancelPanicked => QuarantineReason::ResourceCancelPanicked,
            ResourceOrphanReason::ReleaseFailed => QuarantineReason::ResourceReleaseFailed,
            ResourceOrphanReason::ReleasePanicked => QuarantineReason::ResourceReleasePanicked,
            ResourceOrphanReason::ReleaseDeadline => QuarantineReason::ResourceReleaseDeadline,
            ResourceOrphanReason::ReleaseDriverDropped => {
                QuarantineReason::ResourceReleaseAmbiguous
            }
            ResourceOrphanReason::DependencyCycle => QuarantineReason::ResourceDependencyCycle,
        }
    }

    fn cancel_resources(&self, key: &SessionKey, owner: Option<OperationId>) {
        let resources: Vec<_> = {
            let Ok(index) = self.lock_index() else {
                return;
            };
            let exact_slot = index
                .get(&key.session_id)
                .is_some_and(|slot| slot.retains_exact_cell(key));
            if !exact_slot {
                return;
            }
            let Some(cell) = self.cells.get(key).map(|cell| Arc::clone(cell.value())) else {
                self.latch_fatal(AuthorityFatalReason::InvariantViolation);
                return;
            };
            let Ok(state) = self.lock_cell(&cell) else {
                return;
            };
            state
                .resources
                .iter()
                .filter(|(_, entry)| owner.is_none() || entry.owner_operation == owner)
                .filter_map(|(resource_id, entry)| {
                    entry
                        .state
                        .resource()
                        .map(|resource| (*resource_id, resource))
                })
                .collect()
        };

        for (resource_id, resource) in resources {
            if std::panic::catch_unwind(AssertUnwindSafe(|| resource.cancel())).is_err() {
                self.mark_resource_orphan(
                    key,
                    resource_id,
                    ResourceOrphanReason::CancelPanicked,
                    true,
                );
            }
        }
    }

    fn plan_resource_release(
        &self,
        key: &SessionKey,
        owner: Option<OperationId>,
    ) -> Result<ResourceReleasePlan, SessionOperationError> {
        let now = self.clock.now();
        let index = self
            .lock_index()
            .map_err(SessionOperationError::AuthorityFatal)?;
        let exact_slot = index
            .get(&key.session_id)
            .is_some_and(|slot| slot.retains_exact_cell(key));
        if !exact_slot {
            return Err(SessionOperationError::StaleGeneration);
        }
        let cell = self
            .cells
            .get(key)
            .map(|cell| Arc::clone(cell.value()))
            .ok_or(SessionOperationError::StaleGeneration)?;
        let mut state = self
            .lock_cell(&cell)
            .map_err(SessionOperationError::AuthorityFatal)?;
        let target_ids: HashSet<_> = state
            .resources
            .iter()
            .filter(|(_, entry)| owner.is_none() || entry.owner_operation == owner)
            .map(|(resource_id, _)| *resource_id)
            .collect();
        if target_ids.is_empty() {
            return Ok(ResourceReleasePlan::Complete);
        }

        let unresolved = target_ids.iter().find_map(|resource_id| {
            let entry = state.resources.get(resource_id)?;
            match &entry.state {
                ManagedResourceState::Reserved | ManagedResourceState::Installing => {
                    Some(QuarantineReason::ResourceInstallOrphaned)
                }
                ManagedResourceState::Orphaned {
                    resource: None,
                    reason,
                    ..
                } => Some(Self::resource_quarantine_reason(*reason)),
                _ => None,
            }
        });
        if let Some(reason) = unresolved {
            return Ok(ResourceReleasePlan::Wait(reason));
        }

        let mut ready = Vec::new();
        for resource_id in &target_ids {
            let Some(entry) = state.resources.get(resource_id) else {
                continue;
            };
            let releasable_state = match &entry.state {
                ManagedResourceState::Captured(_) | ManagedResourceState::Live(_) => true,
                ManagedResourceState::Orphaned {
                    resource: Some(_),
                    retry_ready,
                    ..
                } => *retry_ready,
                ManagedResourceState::Reserved
                | ManagedResourceState::Installing
                | ManagedResourceState::Releasing(_)
                | ManagedResourceState::Orphaned { resource: None, .. } => false,
            };
            if !releasable_state {
                continue;
            }
            let has_dependent = target_ids.iter().any(|other_id| {
                other_id != resource_id
                    && state
                        .resources
                        .get(other_id)
                        .is_some_and(|other| other.spec.release_dependencies.contains(resource_id))
            });
            if !has_dependent {
                ready.push(*resource_id);
            }
        }

        if ready.is_empty() {
            if target_ids.iter().any(|resource_id| {
                state
                    .resources
                    .get(resource_id)
                    .is_some_and(|entry| matches!(entry.state, ManagedResourceState::Releasing(_)))
            }) {
                return Ok(ResourceReleasePlan::InFlight);
            }
            if let Some(reason) = target_ids.iter().find_map(|resource_id| {
                let entry = state.resources.get(resource_id)?;
                match &entry.state {
                    ManagedResourceState::Orphaned {
                        reason,
                        retry_ready: false,
                        ..
                    } => Some(Self::resource_quarantine_reason(*reason)),
                    _ => None,
                }
            }) {
                return Ok(ResourceReleasePlan::Wait(reason));
            }

            for resource_id in &target_ids {
                let Some(entry) = state.resources.get_mut(resource_id) else {
                    continue;
                };
                let resource = entry.state.resource();
                entry.state = ManagedResourceState::Orphaned {
                    resource,
                    reason: ResourceOrphanReason::DependencyCycle,
                    retry_ready: false,
                };
                entry.state_since = now;
            }
            return Ok(ResourceReleasePlan::Wait(
                QuarantineReason::ResourceDependencyCycle,
            ));
        }

        let mut wave = Vec::with_capacity(ready.len());
        for resource_id in ready {
            let Some(entry) = state.resources.get_mut(&resource_id) else {
                let reason = self.latch_fatal(AuthorityFatalReason::InvariantViolation);
                return Err(SessionOperationError::AuthorityFatal(reason));
            };
            let Some(resource) = entry.state.resource() else {
                let reason = self.latch_fatal(AuthorityFatalReason::InvariantViolation);
                return Err(SessionOperationError::AuthorityFatal(reason));
            };
            entry.state = ManagedResourceState::Releasing(Arc::clone(&resource));
            entry.state_since = now;
            wave.push(ResourceReleaseWork {
                resource_id,
                spec: entry.spec.clone(),
                resource,
            });
        }
        Ok(ResourceReleasePlan::Wave(wave))
    }

    async fn release_one_resource(
        &self,
        key: SessionKey,
        work: ResourceReleaseWork,
    ) -> ResourceReleaseAttempt<'_> {
        let mut drop_bomb = ResourceReleaseDropBomb {
            authority: self,
            key: key.clone(),
            resource_id: work.resource_id,
            armed: true,
        };
        let Some(deadline) = self.clock.now().checked_add(work.spec.release_timeout) else {
            self.mark_resource_orphan(
                &key,
                work.resource_id,
                ResourceOrphanReason::ReleaseDeadline,
                false,
            );
            drop_bomb.disarm();
            return ResourceReleaseAttempt::AuthorityFatal {
                reason: self.latch_fatal(AuthorityFatalReason::ResourceDeadlineOverflow),
                drop_bomb,
            };
        };
        let resource = Arc::clone(&work.resource);
        let release = AssertUnwindSafe(async move { resource.release().await }).catch_unwind();
        tokio::pin!(release);
        let timeout = self.clock.sleep_until(deadline);
        tokio::pin!(timeout);
        let result = tokio::select! {
            result = &mut release => {
                match result {
                    Ok(Ok(())) => Ok(RollbackReceipt {
                        key,
                        resource_id: work.resource_id,
                        descriptor: work.spec.descriptor,
                    }),
                    Ok(Err(_)) => Err(ResourceOrphanReason::ReleaseFailed),
                    Err(_) => Err(ResourceOrphanReason::ReleasePanicked),
                }
            }
            () = &mut timeout => Err(ResourceOrphanReason::ReleaseDeadline),
        };
        match result {
            Ok(receipt) => ResourceReleaseAttempt::Released { receipt, drop_bomb },
            Err(reason) => ResourceReleaseAttempt::Orphaned {
                resource_id: work.resource_id,
                reason,
                drop_bomb,
            },
        }
    }

    fn apply_rollback_receipt(
        &self,
        receipt: RollbackReceipt,
    ) -> Result<(), SessionOperationError> {
        let index = self
            .lock_index()
            .map_err(SessionOperationError::AuthorityFatal)?;
        let exact_slot = index
            .get(&receipt.key.session_id)
            .is_some_and(|slot| slot.retains_exact_cell(&receipt.key));
        if !exact_slot {
            return Err(SessionOperationError::StaleGeneration);
        }
        let cell = self
            .cells
            .get(&receipt.key)
            .map(|cell| Arc::clone(cell.value()))
            .ok_or(SessionOperationError::StaleGeneration)?;
        let mut state = self
            .lock_cell(&cell)
            .map_err(SessionOperationError::AuthorityFatal)?;
        let Some(entry) = state.resources.get(&receipt.resource_id) else {
            return Err(SessionOperationError::ResourceRollbackFailed);
        };
        if entry.spec.descriptor != receipt.descriptor
            || !matches!(entry.state, ManagedResourceState::Releasing(_))
        {
            return Err(SessionOperationError::ResourceRollbackFailed);
        }
        let owner = entry.owner_operation;
        state.resources.remove(&receipt.resource_id);
        if let Some(operation_id) = owner {
            let Some(operation) = state.operations.get_mut(&operation_id) else {
                let reason = self.latch_fatal(AuthorityFatalReason::InvariantViolation);
                return Err(SessionOperationError::AuthorityFatal(reason));
            };
            operation.resource_ids.remove(&receipt.resource_id);
        }
        drop(state);
        drop(index);
        cell.changed.notify_waiters();
        Ok(())
    }

    fn mark_resource_orphan(
        &self,
        key: &SessionKey,
        resource_id: ResourceId,
        orphan_reason: ResourceOrphanReason,
        retry_ready: bool,
    ) {
        let now = self.clock.now();
        let resource = {
            let Ok(index) = self.lock_index() else {
                return;
            };
            let exact_slot = index
                .get(&key.session_id)
                .is_some_and(|slot| slot.retains_exact_cell(key));
            if !exact_slot {
                return;
            }
            let Some(cell) = self.cells.get(key).map(|cell| Arc::clone(cell.value())) else {
                self.latch_fatal(AuthorityFatalReason::InvariantViolation);
                return;
            };
            let Ok(mut state) = self.lock_cell(&cell) else {
                return;
            };
            let Some(entry) = state.resources.get_mut(&resource_id) else {
                return;
            };
            let resource = entry.state.resource();
            entry.state = ManagedResourceState::Orphaned {
                resource: resource.as_ref().map(Arc::clone),
                reason: orphan_reason,
                retry_ready,
            };
            entry.state_since = now;
            resource
        };
        let _ = resource;
        let _ = self.mark_quarantined(key, Self::resource_quarantine_reason(orphan_reason));
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn retry_orphaned_resources(
        &self,
        key: &SessionKey,
    ) -> Result<usize, SessionOperationError> {
        let index = self
            .lock_index()
            .map_err(SessionOperationError::AuthorityFatal)?;
        let exact_slot = index
            .get(&key.session_id)
            .is_some_and(|slot| slot.retains_exact_cell(key));
        if !exact_slot {
            return Err(SessionOperationError::StaleGeneration);
        }
        let cell = self
            .cells
            .get(key)
            .map(|cell| Arc::clone(cell.value()))
            .ok_or(SessionOperationError::StaleGeneration)?;
        let mut state = self
            .lock_cell(&cell)
            .map_err(SessionOperationError::AuthorityFatal)?;
        let mut count = 0;
        for entry in state.resources.values_mut() {
            if let ManagedResourceState::Orphaned {
                resource: Some(_),
                retry_ready,
                ..
            } = &mut entry.state
            {
                if !*retry_ready {
                    *retry_ready = true;
                    count += 1;
                }
            }
        }
        drop(state);
        drop(index);
        cell.changed.notify_waiters();
        Ok(count)
    }

    async fn release_resources(
        &self,
        key: &SessionKey,
        owner: Option<OperationId>,
    ) -> Result<(), SessionOperationError> {
        self.cancel_resources(key, owner);
        loop {
            let cell = self
                .cells
                .get(key)
                .map(|cell| Arc::clone(cell.value()))
                .ok_or(SessionOperationError::StaleGeneration)?;
            let changed = cell.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            match self.plan_resource_release(key, owner)? {
                ResourceReleasePlan::Complete => return Ok(()),
                ResourceReleasePlan::InFlight => changed.await,
                ResourceReleasePlan::Wait(reason) => {
                    let _ = self.mark_quarantined(key, reason);
                    changed.await;
                }
                ResourceReleasePlan::Wave(wave) => {
                    let attempts = futures::future::join_all(
                        wave.into_iter()
                            .map(|work| self.release_one_resource(key.clone(), work)),
                    )
                    .await;
                    for attempt in attempts {
                        match attempt {
                            ResourceReleaseAttempt::Released {
                                receipt,
                                mut drop_bomb,
                            } => {
                                self.apply_rollback_receipt(receipt)?;
                                drop_bomb.disarm();
                            }
                            ResourceReleaseAttempt::Orphaned {
                                resource_id,
                                reason,
                                mut drop_bomb,
                            } => {
                                self.mark_resource_orphan(key, resource_id, reason, false);
                                drop_bomb.disarm();
                            }
                            ResourceReleaseAttempt::AuthorityFatal { reason, drop_bomb } => {
                                // Keep the bomb armed: returning before exact
                                // application must never strand Releasing.
                                drop(drop_bomb);
                                return Err(SessionOperationError::AuthorityFatal(reason));
                            }
                        }
                    }
                }
            }
        }
    }

    async fn release_operation_resources(
        &self,
        key: &SessionKey,
        operation_id: OperationId,
    ) -> Result<(), SessionOperationError> {
        self.release_resources(key, Some(operation_id)).await
    }

    /// Linearize an operation's external commit while the exact generation is
    /// still Active. Teardown takes the same cell lock before quiescing, so a
    /// recorded revision unambiguously precedes quiescing even if task
    /// completion is delivered later.
    fn record_operation_commit(
        &self,
        key: &SessionKey,
        operation_id: OperationId,
    ) -> Result<CommitRevision, SessionOperationError> {
        let cell = self
            .cells
            .get(key)
            .map(|cell| Arc::clone(cell.value()))
            .ok_or(SessionOperationError::StaleGeneration)?;
        let mut state = self
            .lock_cell(&cell)
            .map_err(SessionOperationError::AuthorityFatal)?;
        if !matches!(state.phase, SessionPhase::Active) {
            return Err(SessionOperationError::StaleGeneration);
        }
        let Some(meta) = state.operations.get(&operation_id) else {
            return Err(SessionOperationError::StaleGeneration);
        };
        if let Some(revision) = meta.commit_revision {
            return Ok(revision);
        }
        let resource_ids: Vec<_> = meta.resource_ids.iter().copied().collect();
        for resource_id in &resource_ids {
            let Some(resource) = state.resources.get(resource_id) else {
                let reason = self.latch_fatal(AuthorityFatalReason::InvariantViolation);
                return Err(SessionOperationError::AuthorityFatal(reason));
            };
            if resource.owner_operation != Some(operation_id)
                || !matches!(resource.state, ManagedResourceState::Captured(_))
            {
                return Err(SessionOperationError::ResourcesUnresolved);
            }
        }
        let revision = NonZeroU64::new(state.next_commit_revision)
            .map(CommitRevision)
            .ok_or(SessionOperationError::OperationSequenceExhausted)?;
        state.next_commit_revision = state
            .next_commit_revision
            .checked_add(1)
            .ok_or(SessionOperationError::OperationSequenceExhausted)?;
        let Some(meta) = state.operations.get_mut(&operation_id) else {
            let reason = self.latch_fatal(AuthorityFatalReason::InvariantViolation);
            return Err(SessionOperationError::AuthorityFatal(reason));
        };
        meta.commit_revision = Some(revision);
        for resource_id in resource_ids {
            let Some(entry) = state.resources.get_mut(&resource_id) else {
                let reason = self.latch_fatal(AuthorityFatalReason::InvariantViolation);
                return Err(SessionOperationError::AuthorityFatal(reason));
            };
            let resource = match &entry.state {
                ManagedResourceState::Captured(resource) => Arc::clone(resource),
                _ => {
                    let reason = self.latch_fatal(AuthorityFatalReason::InvariantViolation);
                    return Err(SessionOperationError::AuthorityFatal(reason));
                }
            };
            entry.state = ManagedResourceState::Live(resource);
            entry.owner_operation = None;
            entry.state_since = self.clock.now();
        }
        Ok(revision)
    }

    fn complete_operation(
        &self,
        key: &SessionKey,
        operation_id: OperationId,
        release: OperationRelease,
    ) {
        let now = self.clock.now();
        let Some(cell) = self.cells.get(key).map(|cell| Arc::clone(cell.value())) else {
            return;
        };
        let Ok(mut state) = self.lock_cell(&cell) else {
            return;
        };
        let Some((_, failure)) = classify_operation_release(&state, operation_id, release) else {
            return;
        };

        // Successful commit/rollback is the overwhelmingly common path. The
        // exact cell already linearizes it with teardown, so it must not take
        // the raw-ID admission lock.
        if failure.is_none() {
            state.operations.remove(&operation_id);
            drop(state);
            cell.changed.notify_waiters();
            return;
        }

        // Quarantine changes the raw-ID admission fence. Reacquire locks in
        // the authority's structural index->cell order, then revalidate the
        // exact operation before publishing that exceptional transition.
        drop(state);
        let Ok(mut index) = self.lock_index() else {
            return;
        };
        if !index
            .get(&key.session_id)
            .is_some_and(|slot| slot.retains_exact_cell(key))
        {
            return;
        }
        let Some(current_cell) = self.cells.get(key).map(|entry| Arc::clone(entry.value())) else {
            return;
        };
        if !Arc::ptr_eq(&cell, &current_cell) {
            self.latch_fatal(AuthorityFatalReason::InvariantViolation);
            return;
        }
        let Ok(mut state) = self.lock_cell(&cell) else {
            return;
        };
        let Some((operation_resources, failure)) =
            classify_operation_release(&state, operation_id, release)
        else {
            return;
        };
        let Some(reason) = failure else {
            state.operations.remove(&operation_id);
            drop(state);
            drop(index);
            cell.changed.notify_waiters();
            return;
        };
        let reason = merge_quarantine_reason(state.sticky_failure, reason);
        state.sticky_failure = Some(reason);
        state.phase = SessionPhase::Quarantined { since: now };
        self.block_non_reusable_locked(&mut index, key);
        if let Some(control) = state.teardown.as_ref() {
            control
                .result
                .send_replace(Some(TeardownOutcome::Quarantined {
                    key: key.clone(),
                    reason,
                }));
        }
        cell.cancel.send_replace(true);
        // The failing task no longer owns an executable rollback path.
        // Transfer every exact registry entry to the session teardown driver;
        // never discard it with the operation metadata.
        for resource_id in &operation_resources {
            if let Some(entry) = state.resources.get_mut(resource_id) {
                if entry.owner_operation == Some(operation_id) {
                    entry.owner_operation = None;
                }
            }
        }
        state.operations.remove(&operation_id);
        drop(state);
        drop(index);
        cell.changed.notify_waiters();
    }

    fn latch_operation_failure(
        &self,
        key: &SessionKey,
        operation_id: OperationId,
        reason: QuarantineReason,
    ) {
        let now = self.clock.now();
        let Ok(mut index) = self.lock_index() else {
            return;
        };
        let exact_slot = index
            .get(&key.session_id)
            .is_some_and(|slot| slot.retains_exact_cell(key));
        if !exact_slot {
            return;
        }
        let Some(cell) = self.cells.get(key).map(|cell| Arc::clone(cell.value())) else {
            return;
        };
        let Ok(mut state) = self.lock_cell(&cell) else {
            return;
        };
        if !state.operations.contains_key(&operation_id) {
            return;
        }
        let reason = merge_quarantine_reason(state.sticky_failure, reason);
        state.sticky_failure = Some(reason);
        state.phase = SessionPhase::Quarantined { since: now };
        self.block_non_reusable_locked(&mut index, key);
        if let Some(control) = state.teardown.as_ref() {
            control
                .result
                .send_replace(Some(TeardownOutcome::Quarantined {
                    key: key.clone(),
                    reason,
                }));
        }
        drop(state);
        drop(index);
        cell.cancel.send_replace(true);
        cell.changed.notify_waiters();
    }

    fn register_supervisor(
        self: &Arc<Self>,
        key: &SessionKey,
        kind: SupervisorKind,
    ) -> Result<SupervisorRegistration, SessionOperationError> {
        if self.draining.load(Ordering::Acquire) {
            return Err(SessionOperationError::AuthorityDraining);
        }
        let sequence = self
            .next_supervisor
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .map_err(|_| SessionOperationError::OperationSequenceExhausted)?;
        let id = NonZeroU64::new(sequence)
            .map(SupervisorId)
            .ok_or(SessionOperationError::OperationSequenceExhausted)?;
        let mut supervisors = self
            .lock_supervisors()
            .map_err(SessionOperationError::AuthorityFatal)?;
        if self.draining.load(Ordering::Acquire) {
            return Err(SessionOperationError::AuthorityDraining);
        }
        supervisors.insert(
            id,
            SupervisorEntry {
                key: key.clone(),
                kind,
                abort: None,
                abort_requested: false,
            },
        );
        Ok(SupervisorRegistration::new(Arc::clone(self), id))
    }

    fn set_supervisor_abort(
        &self,
        id: SupervisorId,
        abort: tokio::task::AbortHandle,
    ) -> Result<(), AuthorityFatalReason> {
        let mut supervisors = self.lock_supervisors_for_cleanup()?;
        // A very short task can unregister before its JoinHandle is returned.
        let mut abort_now = false;
        if let Some(entry) = supervisors.get_mut(&id) {
            abort_now = entry.abort_requested;
            entry.abort = Some(abort.clone());
        }
        drop(supervisors);
        self.supervisors_changed.notify_waiters();
        if abort_now {
            abort.abort();
        }
        Ok(())
    }

    fn unregister_supervisor(&self, id: SupervisorId) {
        let Ok(mut supervisors) = self.lock_supervisors_for_cleanup() else {
            return;
        };
        supervisors.remove(&id);
        drop(supervisors);
        self.supervisors_changed.notify_waiters();
    }

    /// Wait until every lifecycle-owned task future and its drop guard have
    /// been destroyed. This is the authority-local hook a later coordinator
    /// drain must await. It does not yet prove cleanup of external SIP/media
    /// resources; that requires the separate exact-resource registry gate.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) async fn wait_for_supervisors(&self) -> Result<(), SessionOperationError> {
        let current = CURRENT_LIFECYCLE_SUPERVISOR
            .try_with(|current| *current)
            .ok()
            .filter(|current| current.epoch == self.epoch)
            .map(|current| current.id);
        loop {
            let changed = self.supervisors_changed.notified();
            let fatal = self.fatal_changed.notified();
            tokio::pin!(changed);
            tokio::pin!(fatal);
            changed.as_mut().enable();
            fatal.as_mut().enable();
            if let Some(reason) = self.fatal_reason() {
                return Err(SessionOperationError::AuthorityFatal(reason));
            }
            {
                let supervisors = self
                    .lock_supervisors_for_cleanup()
                    .map_err(SessionOperationError::AuthorityFatal)?;
                if supervisors.keys().all(|id| current.as_ref() == Some(id)) {
                    return Ok(());
                }
            }
            tokio::select! {
                () = &mut changed => {}
                () = &mut fatal => {
                    if let Some(reason) = self.fatal_reason() {
                        return Err(SessionOperationError::AuthorityFatal(reason));
                    }
                }
            }
        }
    }

    /// Stop new admissions/supervisors, wait until `deadline`, abort every
    /// remaining lifecycle task except the caller itself, then join through
    /// registry removal. The current task is excluded when drain is invoked
    /// from a supervised operation, preventing a self-join deadlock.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) async fn drain_supervisors(&self, deadline: Instant) -> SupervisorDrainReport {
        self.draining.store(true, Ordering::Release);
        let current = CURRENT_LIFECYCLE_SUPERVISOR
            .try_with(|current| *current)
            .ok()
            .filter(|current| current.epoch == self.epoch)
            .map(|current| current.id);
        let mut report = SupervisorDrainReport {
            excluded_current: current.is_some(),
            ..SupervisorDrainReport::default()
        };

        // Synchronize with admissions that passed the first draining check.
        match self.lock_index() {
            Ok(index) => drop(index),
            Err(reason) => {
                report.fatal_reason = Some(reason);
            }
        }

        let mut last_remaining = 0;
        while report.fatal_reason.is_none() {
            let changed = self.supervisors_changed.notified();
            let fatal = self.fatal_changed.notified();
            tokio::pin!(changed);
            tokio::pin!(fatal);
            changed.as_mut().enable();
            fatal.as_mut().enable();
            last_remaining = {
                let supervisors = match self.lock_supervisors_for_cleanup() {
                    Ok(supervisors) => supervisors,
                    Err(reason) => {
                        report.fatal_reason = Some(reason);
                        report.stragglers = last_remaining;
                        return report;
                    }
                };
                supervisors
                    .keys()
                    .filter(|id| current.as_ref() != Some(*id))
                    .count()
            };
            if last_remaining == 0 {
                return report;
            }
            if self.clock.now() >= deadline {
                break;
            }
            let deadline_sleep = self.clock.sleep_until(deadline);
            tokio::pin!(deadline_sleep);
            tokio::select! {
                () = &mut changed => {}
                () = &mut fatal => {}
                () = &mut deadline_sleep => {}
            }
            if let Some(reason) = self.fatal_reason() {
                report.fatal_reason = Some(reason);
                break;
            }
        }

        report.deadline_reached = self.clock.now() >= deadline;
        let abort_observation_deadline = match self
            .clock
            .now()
            .checked_add(self.config.supervisor_abort_grace)
        {
            Some(deadline) => deadline,
            None => {
                report.fatal_reason =
                    Some(self.latch_fatal(AuthorityFatalReason::DrainDeadlineOverflow));
                self.clock.now()
            }
        };
        let handles = match self.lock_supervisors_for_cleanup() {
            Ok(mut supervisors) => {
                let mut handles = Vec::new();
                for (id, entry) in supervisors.iter_mut() {
                    if current.as_ref() == Some(id) {
                        continue;
                    }
                    if !entry.abort_requested {
                        entry.abort_requested = true;
                        report.abort_requested += 1;
                    }
                    if let Some(abort) = entry.abort.clone() {
                        handles.push(abort);
                    }
                }
                handles
            }
            Err(reason) => {
                report.fatal_reason = Some(reason);
                report.stragglers = last_remaining;
                return report;
            }
        };
        for handle in handles {
            handle.abort();
        }

        loop {
            let changed = self.supervisors_changed.notified();
            let fatal = self.fatal_changed.notified();
            tokio::pin!(changed);
            tokio::pin!(fatal);
            changed.as_mut().enable();
            fatal.as_mut().enable();
            last_remaining = {
                let supervisors = match self.lock_supervisors_for_cleanup() {
                    Ok(supervisors) => supervisors,
                    Err(reason) => {
                        report.fatal_reason = Some(reason);
                        report.stragglers = last_remaining;
                        return report;
                    }
                };
                supervisors
                    .keys()
                    .filter(|id| current.as_ref() != Some(*id))
                    .count()
            };
            if last_remaining == 0 {
                report.stragglers = 0;
                return report;
            }
            if self.clock.now() >= abort_observation_deadline {
                report.stragglers = last_remaining;
                return report;
            }
            let grace_sleep = self.clock.sleep_until(abort_observation_deadline);
            tokio::pin!(grace_sleep);
            tokio::select! {
                () = &mut changed => {}
                () = &mut fatal => {}
                () = &mut grace_sleep => {}
            }
            if let Some(reason) = self.fatal_reason() {
                report.fatal_reason = Some(reason);
            }
        }
    }

    #[cfg(test)]
    fn abort_supervisors_for_key(
        &self,
        key: &SessionKey,
        kind: SupervisorKind,
    ) -> Result<usize, SessionOperationError> {
        let handles: Vec<_> = self
            .lock_supervisors_for_cleanup()
            .map_err(SessionOperationError::AuthorityFatal)?
            .values()
            .filter(|entry| &entry.key == key && entry.kind == kind)
            .filter_map(|entry| entry.abort.clone())
            .collect();
        let count = handles.len();
        for handle in handles {
            handle.abort();
        }
        Ok(count)
    }

    fn spawn_owned<T, F, Fut>(
        self: &Arc<Self>,
        key: &SessionKey,
        kind: SessionOperationKind,
        hard_timeout: Duration,
        operation: F,
    ) -> Result<OperationWaiter<T>, SessionOperationError>
    where
        T: Send + 'static,
        F: FnOnce(OwnedOperation) -> Fut + Send + 'static,
        Fut: Future<Output = OwnedOperationCompletion<T>> + Send + 'static,
    {
        tokio::runtime::Handle::try_current()
            .map_err(|_| SessionOperationError::SupervisorUnavailable)?;
        let hard_deadline = self.operation_deadline(hard_timeout)?;
        let registered = self.register_operation(key, kind, Some(hard_deadline))?;
        let supervisor = self.register_supervisor(key, SupervisorKind::OwnedOperation)?;
        let supervisor_id = supervisor.id.expect("new supervisor registration");
        let mut task = OwnedTaskRegistration {
            operation: Some(registered),
            supervisor: Some(supervisor),
        };
        let operation_token = OwnedOperation {
            context: task.context(),
        };
        let clock = Arc::clone(&self.clock);
        let current = CurrentSupervisor {
            epoch: self.epoch,
            id: supervisor_id,
        };
        let (result_tx, result) = oneshot::channel();
        let execution_task = async move {
            let execution =
                AssertUnwindSafe(async move { operation(operation_token).await }).catch_unwind();
            tokio::pin!(execution);
            let deadline = clock.sleep_until(hard_deadline);
            tokio::pin!(deadline);
            tokio::select! {
                execution = &mut execution => {
                    match execution {
                        Ok(completion) => {
                            let disposition = completion.disposition;
                            task.finish(disposition);
                            let _ = result_tx.send(Ok(completion.into_inner()));
                        }
                        Err(_) => {
                            task.fail(QuarantineReason::OperationPanicked);
                            let _ = result_tx.send(Err(OwnedOperationError::Panicked));
                        }
                    }
                }
                _ = &mut deadline => {
                    // Latch failure while the operation registration is still
                    // present, publish the deadline to the caller, then retain
                    // the creator future until it explicitly commits or rolls
                    // back. Dropping the waiter never cancels this supervisor.
                    task.latch_failure(QuarantineReason::OperationDeadline);
                    let _ = result_tx.send(Err(OwnedOperationError::DeadlineExceeded));
                    match execution.await {
                        Ok(completion) => {
                            let disposition = completion.disposition;
                            drop(completion.into_inner());
                            task.finish(disposition);
                        }
                        Err(_) => task.fail(QuarantineReason::OperationPanicked),
                    }
                }
            }
        };
        let handle = tokio::spawn(CURRENT_LIFECYCLE_SUPERVISOR.scope(current, execution_task));
        let abort = handle.abort_handle();
        if let Err(reason) = self.set_supervisor_abort(supervisor_id, abort.clone()) {
            abort.abort();
            return Err(SessionOperationError::AuthorityFatal(reason));
        }
        Ok(OperationWaiter { result })
    }

    fn mark_quarantined(&self, key: &SessionKey, reason: QuarantineReason) -> bool {
        let now = self.clock.now();
        let Ok(mut index) = self.lock_index() else {
            return false;
        };
        let exact_slot = index
            .get(&key.session_id)
            .is_some_and(|slot| slot.retains_exact_cell(key));
        if !exact_slot {
            return false;
        }
        let Some(cell) = self.cells.get(key).map(|cell| Arc::clone(cell.value())) else {
            self.latch_fatal(AuthorityFatalReason::InvariantViolation);
            return false;
        };
        let Ok(mut state) = self.lock_cell(&cell) else {
            return false;
        };
        if matches!(state.phase, SessionPhase::Retired { .. }) || state.teardown_driver_complete {
            return false;
        }
        let reason = merge_quarantine_reason(state.sticky_failure, reason);
        if state.sticky_failure == Some(reason)
            && matches!(state.phase, SessionPhase::Quarantined { .. })
        {
            // Idempotent quarantine calls must not wake their own retry loop;
            // only a real state/retry change should release a waiter.
            return false;
        }
        state.sticky_failure = Some(reason);
        state.phase = SessionPhase::Quarantined { since: now };
        self.block_non_reusable_locked(&mut index, key);
        if let Some(control) = state.teardown.as_ref() {
            control
                .result
                .send_replace(Some(TeardownOutcome::Quarantined {
                    key: key.clone(),
                    reason,
                }));
        }
        drop(state);
        drop(index);
        cell.cancel.send_replace(true);
        cell.changed.notify_waiters();
        true
    }

    fn transition_terminal_quarantine(&self, key: &SessionKey, proposed: QuarantineReason) -> bool {
        let now = self.clock.now();
        let Ok(mut index) = self.lock_index() else {
            return false;
        };
        let exact_slot = index
            .get(&key.session_id)
            .is_some_and(|slot| slot.retains_exact_cell(key));
        if !exact_slot {
            return false;
        }
        let Some(cell) = self.cells.get(key).map(|cell| Arc::clone(cell.value())) else {
            self.latch_fatal(AuthorityFatalReason::InvariantViolation);
            return false;
        };
        let Ok(mut state) = self.lock_cell(&cell) else {
            return false;
        };
        if matches!(state.phase, SessionPhase::Retired { .. }) || state.teardown_driver_complete {
            return false;
        }
        let reason = merge_quarantine_reason(state.sticky_failure, proposed);
        state.sticky_failure = Some(reason);
        state.phase = SessionPhase::Quarantined { since: now };
        state.teardown_driver_complete = true;
        self.block_non_reusable_locked(&mut index, key);
        if let Some(control) = state.teardown.as_ref() {
            control
                .result
                .send_replace(Some(TeardownOutcome::Quarantined {
                    key: key.clone(),
                    reason,
                }));
            control.done.send_replace(true);
        }
        drop(state);
        drop(index);
        cell.cancel.send_replace(true);
        cell.changed.notify_waiters();
        true
    }

    pub(crate) fn teardown(
        self: &Arc<Self>,
        key: &SessionKey,
        timeout: Duration,
    ) -> Result<TeardownWaiter, SessionOperationError> {
        tokio::runtime::Handle::try_current()
            .map_err(|_| SessionOperationError::SupervisorUnavailable)?;
        let deadline = self.operation_deadline(timeout)?;
        let mut index = self
            .lock_index()
            .map_err(SessionOperationError::AuthorityFatal)?;
        match index.get(&key.session_id) {
            Some(AdmissionSlot::Retired {
                generation,
                reported_quarantine,
                ..
            }) if *generation == key.generation => {
                let outcome = (*reported_quarantine).map_or_else(
                    || TeardownOutcome::Retired { key: key.clone() },
                    |reason| TeardownOutcome::Quarantined {
                        key: key.clone(),
                        reason,
                    },
                );
                let (_result_tx, result) = watch::channel(Some(outcome));
                let (_done_tx, done) = watch::channel(true);
                return Ok(TeardownWaiter {
                    result,
                    done,
                    authority: Arc::clone(self),
                });
            }
            Some(AdmissionSlot::Live(current)) if current == key => {}
            Some(AdmissionSlot::NonReusable(current)) if current == key => {}
            Some(AdmissionSlot::Live(_))
            | Some(AdmissionSlot::NonReusable(_))
            | Some(AdmissionSlot::Retired { .. })
            | None => {
                return Err(SessionOperationError::StaleGeneration);
            }
        }
        let cell = self
            .cells
            .get(key)
            .map(|cell| Arc::clone(cell.value()))
            .ok_or(SessionOperationError::StaleGeneration)?;
        let mut state = self
            .lock_cell(&cell)
            .map_err(SessionOperationError::AuthorityFatal)?;
        if let Some(existing) = state.teardown.as_ref() {
            return Ok(TeardownWaiter {
                result: existing.result.subscribe(),
                done: existing.done.subscribe(),
                authority: Arc::clone(self),
            });
        }
        let (result_tx, result) = watch::channel(None);
        let (done_tx, done) = watch::channel(false);
        let control = Arc::new(TeardownControl {
            result: result_tx,
            done: done_tx,
        });
        state.teardown = Some(Arc::clone(&control));
        let drive = match state.phase {
            SessionPhase::Active => {
                state.phase = SessionPhase::Quiescing { deadline };
                self.block_non_reusable_locked(&mut index, key);
                true
            }
            SessionPhase::Quiescing { .. } | SessionPhase::Releasing => true,
            SessionPhase::Quarantined { .. } => {
                let reason = state
                    .sticky_failure
                    .unwrap_or(QuarantineReason::QuiesceDeadline);
                control
                    .result
                    .send_replace(Some(TeardownOutcome::Quarantined {
                        key: key.clone(),
                        reason,
                    }));
                // Even a permanent non-reuse fence may own captured resources.
                // Start a cleanup driver and mark it terminal only after the
                // exact registry is empty.
                true
            }
            SessionPhase::Retired { .. } => {
                state.teardown_driver_complete = true;
                control
                    .result
                    .send_replace(Some(TeardownOutcome::Retired { key: key.clone() }));
                control.done.send_replace(true);
                false
            }
        };
        drop(state);
        drop(index);
        cell.cancel.send_replace(true);
        if drive {
            let supervisor = match self.register_supervisor(key, SupervisorKind::Teardown) {
                Ok(supervisor) => supervisor,
                Err(error) => {
                    let _ = self
                        .transition_terminal_quarantine(key, QuarantineReason::SupervisorAbandoned);
                    return Err(error);
                }
            };
            let Some(supervisor_id) = supervisor.id else {
                let reason = self.latch_fatal(AuthorityFatalReason::InvariantViolation);
                return Err(SessionOperationError::AuthorityFatal(reason));
            };
            let authority = Arc::clone(self);
            let owned_key = key.clone();
            let current = CurrentSupervisor {
                epoch: self.epoch,
                id: supervisor_id,
            };
            let mut supervisor_guard = TeardownSupervisorGuard {
                authority: Arc::clone(self),
                key: key.clone(),
                registration: Some(supervisor),
                armed: true,
            };
            let teardown_task = async move {
                let driver = Arc::clone(&authority).drive_teardown(
                    owned_key.clone(),
                    cell,
                    deadline,
                    control,
                );
                if AssertUnwindSafe(driver).catch_unwind().await.is_err() {
                    let _ = authority.transition_terminal_quarantine(
                        &owned_key,
                        QuarantineReason::OperationPanicked,
                    );
                }
                supervisor_guard.disarm();
            };
            let handle = tokio::spawn(CURRENT_LIFECYCLE_SUPERVISOR.scope(current, teardown_task));
            let abort = handle.abort_handle();
            if let Err(reason) = self.set_supervisor_abort(supervisor_id, abort.clone()) {
                abort.abort();
                return Err(SessionOperationError::AuthorityFatal(reason));
            }
        }
        Ok(TeardownWaiter {
            result,
            done,
            authority: Arc::clone(self),
        })
    }

    async fn drive_teardown(
        self: Arc<Self>,
        key: SessionKey,
        cell: Arc<SessionCell>,
        deadline: Instant,
        control: Arc<TeardownControl>,
    ) {
        let mut reported_quarantine = match control.result.borrow().as_ref() {
            Some(TeardownOutcome::Quarantined { reason, .. }) => Some(*reason),
            Some(TeardownOutcome::Retired { .. }) | None => None,
        };
        loop {
            let notified = cell.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let Ok(mut state) = self.lock_cell(&cell) else {
                    return;
                };
                if let Some(reason) = state.sticky_failure {
                    if reported_quarantine != Some(reason) {
                        control
                            .result
                            .send_replace(Some(TeardownOutcome::Quarantined {
                                key: key.clone(),
                                reason,
                            }));
                        reported_quarantine = Some(reason);
                    }
                }
                if state.operations.is_empty() {
                    state.phase = SessionPhase::Releasing;
                    break;
                }
            }
            if reported_quarantine.is_some() {
                notified.await;
            } else {
                let deadline_sleep = self.clock.sleep_until(deadline);
                tokio::pin!(deadline_sleep);
                tokio::select! {
                    () = notified => {}
                    () = &mut deadline_sleep => {
                        if self.mark_quarantined(&key, QuarantineReason::QuiesceDeadline) {
                            reported_quarantine = Some(QuarantineReason::QuiesceDeadline);
                        }
                    }
                }
            }
        }

        // Cancellation is broadcast before any release begins. This driver
        // remains registered for the entire non-expiring orphan/retry loop, so
        // an ambiguous install or failed release cannot silently free capacity
        // or make the public session identifier reusable.
        if self.release_resources(&key, None).await.is_err() {
            return;
        }

        let terminal_reason = {
            let Ok(state) = self.lock_cell(&cell) else {
                return;
            };
            if !state.operations.is_empty() || !state.resources.is_empty() {
                self.latch_fatal(AuthorityFatalReason::InvariantViolation);
                return;
            }
            state
                .sticky_failure
                .filter(|reason| !reason.is_recoverable())
        };
        if let Some(reason) = terminal_reason {
            // Unknown side effects from an abandoned/panicked operation remain
            // a permanent non-reuse fence even after every registered resource
            // has been released exactly.
            let _ = self.transition_terminal_quarantine(&key, reason);
            return;
        }

        let retired_at = self.clock.now();
        let Ok(reusable_at) = self.retirement_deadline(retired_at) else {
            return;
        };
        let Ok(mut index) = self.lock_index() else {
            return;
        };
        let exact_slot = index
            .get(&key.session_id)
            .is_some_and(|slot| slot.retains_exact_cell(&key));
        if !exact_slot {
            self.latch_fatal(AuthorityFatalReason::InvariantViolation);
            return;
        }
        let Ok(mut state) = self.lock_cell(&cell) else {
            return;
        };
        if !state.operations.is_empty() || !state.resources.is_empty() {
            self.latch_fatal(AuthorityFatalReason::InvariantViolation);
            return;
        }
        state.phase = SessionPhase::Retired { retired_at };
        state.active_permit.take();
        state.teardown_driver_complete = true;
        self.current
            .remove_if(&key.session_id, |_, current| current == &key);
        let advances_next_deadline =
            index.retire_until(key.clone(), retired_at, reusable_at, reported_quarantine);
        if reported_quarantine.is_none() {
            control
                .result
                .send_replace(Some(TeardownOutcome::Retired { key: key.clone() }));
        }
        control.done.send_replace(true);
        drop(state);
        if self.remove_exact_retired_cell(&key, &cell).is_err() {
            return;
        }
        drop(index);
        if advances_next_deadline || !self.reuse_pruner_started.load(Ordering::Acquire) {
            self.ensure_reuse_pruner();
        }
        if advances_next_deadline {
            self.reuse_pruner_changed.notify_one();
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn diagnostics(&self) -> SessionLifecycleDiagnostics {
        let now = self.clock.now();
        let mut snapshot = SessionLifecycleDiagnostics {
            complete: true,
            fatal_reason: self.fatal_reason(),
            capacity: self.config.capacity.get(),
            active_capacity_in_use: self
                .config
                .capacity
                .get()
                .saturating_sub(self.active_slots.available_permits()),
            retained_capacity: self.config.retained_capacity.get(),
            resource_capacity_per_session: self.config.resource_capacity_per_session.get(),
            current_index_capacity: self.current.capacity(),
            exact_cell_index_capacity: self.cells.capacity(),
            ..SessionLifecycleDiagnostics::default()
        };
        if snapshot.fatal_reason.is_some() {
            snapshot.complete = false;
            return snapshot;
        }

        let index = match self.index.lock() {
            Ok(index) => index,
            Err(_) => {
                snapshot.fatal_reason = Some(self.latch_fatal(AuthorityFatalReason::IndexPoisoned));
                snapshot.complete = false;
                return snapshot;
            }
        };
        snapshot.admission_index_capacity = index.slots.capacity();
        snapshot.reusable_deadline_capacity = index.reusable_deadlines.capacity();
        snapshot.retained_identifier_payload_bytes = index
            .slots
            .keys()
            .map(|session_id| session_id.0.capacity())
            .sum();
        snapshot.lifecycle_count = index.len();
        let mut cells = Vec::with_capacity(index.len().min(self.config.capacity.get()));
        for slot in index.values() {
            match slot {
                AdmissionSlot::Live(key) => {
                    snapshot.index_live += 1;
                    let Some(cell) = self.cells.get(key).map(|cell| Arc::clone(cell.value()))
                    else {
                        snapshot.fatal_reason =
                            Some(self.latch_fatal(AuthorityFatalReason::InvariantViolation));
                        snapshot.complete = false;
                        return snapshot;
                    };
                    cells.push(cell);
                }
                AdmissionSlot::NonReusable(key) => {
                    snapshot.index_blocked += 1;
                    let Some(cell) = self.cells.get(key).map(|cell| Arc::clone(cell.value()))
                    else {
                        snapshot.fatal_reason =
                            Some(self.latch_fatal(AuthorityFatalReason::InvariantViolation));
                        snapshot.complete = false;
                        return snapshot;
                    };
                    cells.push(cell);
                }
                AdmissionSlot::Retired {
                    retired_at,
                    reusable_at: _,
                    reported_quarantine,
                    ..
                } => {
                    snapshot.index_blocked += 1;
                    snapshot.retired += 1;
                    let age = now.saturating_duration_since(*retired_at);
                    snapshot.oldest_retired_age = Some(
                        snapshot
                            .oldest_retired_age
                            .map_or(age, |current| current.max(age)),
                    );
                    if let Some(reason) = reported_quarantine {
                        snapshot.quarantine_reasons.record(*reason);
                    }
                }
            }
        }
        if self.current.len() != snapshot.index_live
            || self.current.iter().any(|entry| {
                !matches!(
                    index.get(entry.key()),
                    Some(AdmissionSlot::Live(key)) if key == entry.value()
                )
            })
        {
            snapshot.fatal_reason =
                Some(self.latch_fatal(AuthorityFatalReason::InvariantViolation));
            snapshot.complete = false;
            return snapshot;
        }
        if self.cells.len() != cells.len() {
            snapshot.fatal_reason =
                Some(self.latch_fatal(AuthorityFatalReason::InvariantViolation));
            snapshot.complete = false;
            return snapshot;
        }
        drop(index);

        for cell in cells {
            let state = match cell.state.lock() {
                Ok(state) => state,
                Err(_) => {
                    snapshot.fatal_reason =
                        Some(self.latch_fatal(AuthorityFatalReason::CellPoisoned));
                    snapshot.complete = false;
                    return snapshot;
                }
            };
            match state.phase {
                SessionPhase::Active => snapshot.active += 1,
                SessionPhase::Quiescing { .. } => snapshot.quiescing += 1,
                SessionPhase::Releasing => snapshot.releasing += 1,
                SessionPhase::Quarantined { since } => {
                    snapshot.quarantined += 1;
                    let age = now.saturating_duration_since(since);
                    snapshot.oldest_quarantine_age = Some(
                        snapshot
                            .oldest_quarantine_age
                            .map_or(age, |current| current.max(age)),
                    );
                }
                SessionPhase::Retired { retired_at } => {
                    snapshot.retired += 1;
                    let age = now.saturating_duration_since(retired_at);
                    snapshot.oldest_retired_age = Some(
                        snapshot
                            .oldest_retired_age
                            .map_or(age, |current| current.max(age)),
                    );
                }
            }
            for meta in state.operations.values() {
                snapshot.operations.total += 1;
                snapshot.operations.with_hard_deadline += usize::from(meta.hard_deadline.is_some());
                match meta.kind {
                    SessionOperationKind::StateTransition => {
                        snapshot.operations.state_transition += 1
                    }
                    SessionOperationKind::Signaling => snapshot.operations.signaling += 1,
                    SessionOperationKind::Media => snapshot.operations.media += 1,
                    SessionOperationKind::EventDispatch => snapshot.operations.event_dispatch += 1,
                    SessionOperationKind::Test(_) => snapshot.operations.test += 1,
                }
            }
            for entry in state.resources.values() {
                snapshot.resources.total += 1;
                match &entry.state {
                    ManagedResourceState::Reserved => snapshot.resources.reserved += 1,
                    ManagedResourceState::Installing => snapshot.resources.installing += 1,
                    ManagedResourceState::Captured(_) => snapshot.resources.captured += 1,
                    ManagedResourceState::Live(_) => snapshot.resources.live += 1,
                    ManagedResourceState::Releasing(_) => snapshot.resources.releasing += 1,
                    ManagedResourceState::Orphaned { reason, .. } => {
                        snapshot.resources.orphaned += 1;
                        let age = now.saturating_duration_since(entry.state_since);
                        snapshot.resources.oldest_orphan_age = Some(
                            snapshot
                                .resources
                                .oldest_orphan_age
                                .map_or(age, |current| current.max(age)),
                        );
                        match reason {
                            ResourceOrphanReason::InstallAttemptDropped => {
                                snapshot.resources.orphan_reasons.install_attempt_dropped += 1
                            }
                            ResourceOrphanReason::DispatchPermitDropped => {
                                snapshot.resources.orphan_reasons.dispatch_permit_dropped += 1
                            }
                            ResourceOrphanReason::DescriptorMismatch => {
                                snapshot.resources.orphan_reasons.descriptor_mismatch += 1
                            }
                            ResourceOrphanReason::CancelPanicked => {
                                snapshot.resources.orphan_reasons.cancel_panicked += 1
                            }
                            ResourceOrphanReason::ReleaseFailed => {
                                snapshot.resources.orphan_reasons.release_failed += 1
                            }
                            ResourceOrphanReason::ReleasePanicked => {
                                snapshot.resources.orphan_reasons.release_panicked += 1
                            }
                            ResourceOrphanReason::ReleaseDeadline => {
                                snapshot.resources.orphan_reasons.release_deadline += 1
                            }
                            ResourceOrphanReason::ReleaseDriverDropped => {
                                snapshot.resources.orphan_reasons.release_driver_dropped += 1
                            }
                            ResourceOrphanReason::DependencyCycle => {
                                snapshot.resources.orphan_reasons.dependency_cycle += 1
                            }
                        }
                    }
                }
            }
            if let Some(reason) = state.sticky_failure {
                snapshot.quarantine_reasons.record(reason);
            }
        }

        let supervisors = match self.supervisors.lock() {
            Ok(supervisors) => supervisors,
            Err(_) => {
                snapshot.fatal_reason =
                    Some(self.latch_fatal(AuthorityFatalReason::SupervisorRegistryPoisoned));
                snapshot.complete = false;
                return snapshot;
            }
        };
        snapshot.active_supervisors = supervisors.len();
        for supervisor in supervisors.values() {
            match supervisor.kind {
                SupervisorKind::OwnedOperation => snapshot.owned_operation_supervisors += 1,
                SupervisorKind::Teardown => snapshot.teardown_supervisors += 1,
            }
        }
        snapshot
    }

    #[cfg(test)]
    fn lifecycle_count(&self) -> usize {
        self.index
            .lock()
            .map(|index| index.len())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    struct ManualClockState {
        now: StdMutex<Instant>,
        changed: Notify,
    }

    #[derive(Clone)]
    struct ManualClock(Arc<ManualClockState>);

    impl ManualClock {
        fn new() -> Self {
            Self(Arc::new(ManualClockState {
                now: StdMutex::new(Instant::now()),
                changed: Notify::new(),
            }))
        }

        fn advance(&self, duration: Duration) {
            let mut now = self.0.now.lock().expect("manual clock lock");
            *now = now.checked_add(duration).expect("manual clock advance");
            drop(now);
            self.0.changed.notify_waiters();
        }
    }

    impl LifecycleClock for ManualClock {
        fn now(&self) -> Instant {
            *self.0.now.lock().expect("manual clock lock")
        }

        fn sleep_until(&self, deadline: Instant) -> ClockSleep {
            let clock = self.clone();
            Box::pin(async move {
                loop {
                    let changed = clock.0.changed.notified();
                    tokio::pin!(changed);
                    changed.as_mut().enable();
                    if clock.now() >= deadline {
                        return;
                    }
                    changed.await;
                }
            })
        }
    }

    #[derive(Clone, Copy)]
    enum TestReleaseOutcome {
        Ok,
        Failed,
        Panicked,
        Pending,
    }

    struct TestResource {
        descriptor: ResourceDescriptor,
        events: Arc<StdMutex<Vec<String>>>,
        release_outcomes: Arc<StdMutex<VecDeque<TestReleaseOutcome>>>,
        released: Arc<Notify>,
        release_attempts: Arc<AtomicU64>,
        release_gate: Arc<StdMutex<Option<oneshot::Receiver<()>>>>,
    }

    impl ManagedSessionResource for TestResource {
        fn descriptor(&self) -> ResourceDescriptor {
            self.descriptor.clone()
        }

        fn cancel(&self) {
            self.events
                .lock()
                .expect("resource events")
                .push(format!("cancel:{}", self.descriptor.identity));
        }

        fn release(&self) -> ResourceReleaseFuture {
            let identity = self.descriptor.identity.clone();
            let events = Arc::clone(&self.events);
            let outcomes = Arc::clone(&self.release_outcomes);
            let released = Arc::clone(&self.released);
            let release_attempts = Arc::clone(&self.release_attempts);
            let release_gate = self.release_gate.lock().expect("release gate").take();
            Box::pin(async move {
                release_attempts.fetch_add(1, Ordering::SeqCst);
                events
                    .lock()
                    .expect("resource events")
                    .push(format!("release:{identity}"));
                released.notify_waiters();
                if let Some(release_gate) = release_gate {
                    let _ = release_gate.await;
                }
                let outcome = {
                    outcomes
                        .lock()
                        .expect("release outcomes")
                        .pop_front()
                        .unwrap_or(TestReleaseOutcome::Ok)
                };
                match outcome {
                    TestReleaseOutcome::Ok => Ok(()),
                    TestReleaseOutcome::Failed => Err(ManagedResourceReleaseError::new("injected")),
                    TestReleaseOutcome::Panicked => panic!("injected resource release panic"),
                    TestReleaseOutcome::Pending => futures::future::pending().await,
                }
            })
        }
    }

    fn test_resource(
        identity: &str,
        events: Arc<StdMutex<Vec<String>>>,
        outcomes: impl IntoIterator<Item = TestReleaseOutcome>,
    ) -> Arc<TestResource> {
        Arc::new(TestResource {
            descriptor: ResourceDescriptor::new("test", identity),
            events,
            release_outcomes: Arc::new(StdMutex::new(outcomes.into_iter().collect())),
            released: Arc::new(Notify::new()),
            release_attempts: Arc::new(AtomicU64::new(0)),
            release_gate: Arc::new(StdMutex::new(None)),
        })
    }

    fn gated_test_resource(
        identity: &str,
        events: Arc<StdMutex<Vec<String>>>,
    ) -> (oneshot::Sender<()>, Arc<TestResource>) {
        let (release_tx, release_rx) = oneshot::channel();
        let resource = test_resource(identity, events, []);
        *resource.release_gate.lock().expect("release gate") = Some(release_rx);
        (release_tx, resource)
    }

    fn resource_spec(identity: &str, dependencies: Vec<ResourceId>) -> ResourceSpec {
        ResourceSpec::new(
            ResourceDescriptor::new("test", identity),
            dependencies,
            Duration::from_secs(5),
        )
        .expect("resource spec")
    }

    fn authority(clock: &ManualClock, capacity: usize) -> Arc<SessionLeaseAuthority> {
        authority_with_grace(clock, capacity, DEFAULT_SUPERVISOR_ABORT_GRACE)
    }

    fn authority_with_grace(
        clock: &ManualClock,
        capacity: usize,
        supervisor_abort_grace: Duration,
    ) -> Arc<SessionLeaseAuthority> {
        SessionLeaseAuthority::with_config(
            AuthorityEpoch::fixed(7),
            SessionLifecycleConfig::new(capacity, Duration::from_secs(64))
                .and_then(|config| config.with_supervisor_abort_grace(supervisor_abort_grace))
                .expect("test config"),
            Arc::new(clock.clone()),
        )
    }

    fn take_reuse_deadline_inspections(authority: &SessionLeaseAuthority) -> u64 {
        let mut index = authority.lock_index().expect("admission index");
        std::mem::take(&mut index.reuse_deadline_inspections)
    }

    #[test]
    fn active_identifier_cannot_be_admitted_twice() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 2);
        let id = SessionId::from("same-id");
        let first = authority.admit(id.clone()).expect("first admission");

        assert_eq!(
            authority.admit(id).err(),
            Some(SessionAdmissionError::AlreadyActive)
        );
        assert!(first.is_current());
        assert_eq!(authority.lifecycle_count(), 1);
    }

    #[test]
    fn explicit_retained_capacity_covers_qualified_high_cps_horizon() {
        const ACTIVE_CAPACITY: usize = 20_000;
        const RETAINED_CAPACITY: usize = 160_000;
        const QUALIFIED_CPS: usize = 2_000;
        const HORIZON_SECONDS: usize = 64;

        let config = SessionLifecycleConfig::new(
            ACTIVE_CAPACITY,
            Duration::from_secs(HORIZON_SECONDS as u64),
        )
        .and_then(|config| config.with_retained_capacity(RETAINED_CAPACITY))
        .expect("qualified high-CPS lifecycle config");

        assert!(
            config.retained_capacity.get() >= ACTIVE_CAPACITY + QUALIFIED_CPS * HORIZON_SECONDS
        );

        let authority =
            SessionLeaseAuthority::with_capacities(ACTIVE_CAPACITY, config.retained_capacity.get())
                .expect("explicit production capacities");
        let diagnostics = authority.diagnostics();
        assert_eq!(diagnostics.capacity, ACTIVE_CAPACITY);
        assert_eq!(diagnostics.retained_capacity, RETAINED_CAPACITY);
        assert!(diagnostics.admission_index_capacity < ACTIVE_CAPACITY);
        assert!(diagnostics.current_index_capacity < ACTIVE_CAPACITY);
        assert!(diagnostics.exact_cell_index_capacity < ACTIVE_CAPACITY);
    }

    #[test]
    fn active_lookup_indexes_grow_lazily_without_lowering_the_logical_limit() {
        const ACTIVE_CAPACITY: usize = 20_000;

        let clock = ManualClock::new();
        let authority = authority(&clock, ACTIVE_CAPACITY);
        let initial = authority.diagnostics();

        assert_eq!(initial.capacity, ACTIVE_CAPACITY);
        assert_eq!(initial.active_capacity_in_use, 0);
        assert!(initial.current_index_capacity < ACTIVE_CAPACITY);
        assert!(initial.exact_cell_index_capacity < ACTIVE_CAPACITY);

        // Cross the actual allocator-rounded initial table capacity. Both
        // indexes must grow normally while the independent semaphore still
        // exposes the complete configured admission limit.
        let grow_to = initial
            .current_index_capacity
            .max(initial.exact_cell_index_capacity)
            + 1;
        let leases = (0..grow_to)
            .map(|sequence| {
                authority
                    .admit(SessionId::from(format!("lazy-index-{sequence}")))
                    .expect("logical capacity remains available past eager reserve")
            })
            .collect::<Vec<_>>();

        assert_eq!(leases.len(), grow_to);
        assert_eq!(authority.current.len(), grow_to);
        assert_eq!(authority.cells.len(), grow_to);
        assert!(authority.current.capacity() > initial.current_index_capacity);
        assert!(authority.cells.capacity() > initial.exact_cell_index_capacity);
        assert_eq!(
            authority.diagnostics().active_capacity_in_use,
            grow_to,
            "lazy allocation must not change enforced active admission accounting"
        );
    }

    #[test]
    fn retired_identifier_is_blocked_through_anti_reuse_horizon() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 2);
        let id = SessionId::from("reused-id");
        let first = authority.admit(id.clone()).expect("first admission");
        authority.retire(first.key()).expect("retire first");
        assert!(
            !authority.cells.contains_key(first.key()),
            "retirement must compact the full lifecycle cell immediately"
        );
        assert!(matches!(
            authority.phase(first.key()),
            Some(SessionPhase::Retired { .. })
        ));
        let retired = authority.diagnostics();
        assert_eq!(retired.lifecycle_count, 1);
        assert_eq!(retired.retired, 1);
        assert_eq!(retired.active_capacity_in_use, 0);

        clock.advance(Duration::from_secs(63));
        assert_eq!(
            authority.admit(id.clone()).err(),
            Some(SessionAdmissionError::ReuseBlocked)
        );
        clock.advance(Duration::from_secs(1));
        let second = authority.admit(id).expect("reuse after horizon");

        assert_ne!(first.key().generation, second.key().generation);
        assert_eq!(authority.lifecycle_count(), 1);
    }

    #[test]
    fn compact_retired_slot_and_deadline_share_one_identifier_allocation() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 1);
        let lease = authority
            .admit(SessionId::from("compact-retired-id"))
            .expect("admit compact retained lifecycle");
        let generation = lease.key().generation;
        authority
            .retire(lease.key())
            .expect("retire compact lifecycle");

        let index = authority.lock_index().expect("admission index");
        let (stored_id, slot) = index
            .slots
            .get_key_value(&SessionId::from("compact-retired-id"))
            .expect("compact retired slot");
        assert!(matches!(
            slot,
            AdmissionSlot::Retired {
                generation: stored_generation,
                ..
            } if *stored_generation == generation
        ));
        let deadline = index
            .reusable_deadlines
            .peek()
            .expect("compact retained deadline");
        assert!(Arc::ptr_eq(stored_id, &deadline.session_id));
        assert_eq!(deadline.generation, generation);
        assert_eq!(Arc::strong_count(stored_id), 2);
    }

    #[tokio::test]
    async fn retired_fences_expire_autonomously_in_bounded_idle_batches() {
        const RETIRED_FENCES: usize = REUSABLE_FENCE_EXPIRY_BATCH + 1;

        let clock = ManualClock::new();
        let config = SessionLifecycleConfig::new(1, Duration::from_secs(64))
            .and_then(|config| config.with_retained_capacity(RETIRED_FENCES))
            .expect("idle expiry test config");
        let authority = SessionLeaseAuthority::with_config(
            AuthorityEpoch::fixed(17),
            config,
            Arc::new(clock.clone()),
        );

        for sequence in 0..RETIRED_FENCES {
            let lease = authority
                .admit(SessionId::from(format!("idle-retained-{sequence}")))
                .expect("admit idle retained lifecycle");
            authority
                .retire(lease.key())
                .expect("retire idle retained lifecycle");
        }
        assert_eq!(authority.lifecycle_count(), RETIRED_FENCES);

        // The worker may wake or be notified arbitrarily often, but it must
        // never release an identifier before the complete horizon.
        clock.advance(Duration::from_secs(63));
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
        assert_eq!(authority.lifecycle_count(), RETIRED_FENCES);

        // No admission, diagnostics snapshot, or other lifecycle operation
        // drives expiry after this point. The single authority worker drains
        // one bounded batch, yields, and drains the remainder autonomously.
        clock.advance(Duration::from_secs(1));
        for _ in 0..32 {
            if authority.lifecycle_count() == 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(authority.lifecycle_count(), 0);
        assert!(authority
            .index
            .lock()
            .expect("admission index")
            .reusable_deadlines
            .is_empty());
    }

    #[test]
    fn admission_at_retention_boundary_is_bounded_and_reuses_exact_target() {
        const RETIRED_FENCES: usize = REUSABLE_FENCE_EXPIRY_BATCH + 1;

        let clock = ManualClock::new();
        let config = SessionLifecycleConfig::new(1, Duration::from_secs(64))
            .and_then(|config| config.with_retained_capacity(RETIRED_FENCES))
            .expect("bounded admission test config");
        let authority = SessionLeaseAuthority::with_config(
            AuthorityEpoch::fixed(18),
            config,
            Arc::new(clock.clone()),
        );
        let target = SessionId::from(format!("bounded-admission-{}", RETIRED_FENCES - 1));

        for sequence in 0..RETIRED_FENCES {
            let session_id = SessionId::from(format!("bounded-admission-{sequence}"));
            let lease = authority
                .admit(session_id)
                .expect("admit retained lifecycle");
            authority
                .retire(lease.key())
                .expect("retire retained lifecycle");
        }
        assert_eq!(authority.lifecycle_count(), RETIRED_FENCES);

        clock.advance(Duration::from_secs(64));
        let _ = take_reuse_deadline_inspections(&authority);
        let replacement = authority
            .admit(target.clone())
            .expect("exact expired target remains reusable at full retained capacity");
        let inspected = take_reuse_deadline_inspections(&authority);

        assert_eq!(replacement.key().session_id, target);
        assert!(
            inspected <= REUSABLE_FENCE_EXPIRY_BATCH as u64,
            "admission inspected {inspected} deadlines, exceeding one bounded wave"
        );
        assert_eq!(
            authority.lifecycle_count(),
            1,
            "one bounded wave plus exact-target removal must leave only the replacement"
        );
    }

    #[test]
    fn idle_retention_boundary_reclaims_exact_index_high_water() {
        const ACTIVE_CAPACITY: usize = MAX_EAGER_ACTIVE_LIFETIME_INDEX_CAPACITY + 1;

        let clock = ManualClock::new();
        let authority = authority(&clock, ACTIVE_CAPACITY);
        let leases = (0..ACTIVE_CAPACITY)
            .map(|sequence| {
                authority
                    .admit(SessionId::from(format!("shrink-safe-{sequence}")))
                    .expect("logical active capacity remains available")
            })
            .collect::<Vec<_>>();
        let current_high_water = authority.current.capacity();
        let cell_high_water = authority.cells.capacity();
        assert!(current_high_water > MAX_EAGER_ACTIVE_LIFETIME_INDEX_CAPACITY);
        assert!(cell_high_water > MAX_EAGER_ACTIVE_LIFETIME_INDEX_CAPACITY);

        for lease in &leases {
            authority
                .retire(lease.key())
                .expect("retire exact lifetime before idle horizon");
        }
        assert!(authority.current.is_empty());
        assert!(authority.cells.is_empty());

        clock.advance(Duration::from_secs(64));
        {
            let mut index = authority.lock_index().expect("admission index");
            authority
                .purge_reusable_locked(&mut index, clock.now())
                .expect("purge complete idle horizon");
            assert!(index.is_empty());
            assert!(index.reusable_deadlines.is_empty());
            assert!(
                index.capacity()
                    <= HashMap::<Arc<SessionId>, AdmissionSlot>::with_capacity(
                        MAX_EAGER_ACTIVE_LIFETIME_INDEX_CAPACITY,
                    )
                    .capacity()
            );
        }

        assert!(authority.current.capacity() < current_high_water);
        assert!(authority.cells.capacity() < cell_high_water);
        assert_eq!(authority.diagnostics().capacity, ACTIVE_CAPACITY);
    }

    #[test]
    fn stale_generation_cannot_retire_current_lifetime() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 2);
        let id = SessionId::from("stale-id");
        let first = authority.admit(id.clone()).expect("first admission");
        authority.retire(first.key()).expect("retire first");
        clock.advance(Duration::from_secs(64));
        let second = authority.admit(id).expect("second admission");

        assert_eq!(
            authority.retire(first.key()),
            Err(SessionAdmissionError::StaleGeneration)
        );
        assert!(second.is_current());
    }

    #[test]
    fn stale_reuse_deadline_cannot_remove_a_new_generation() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 2);
        let id = SessionId::from("stale-deadline-id");
        let first = authority.admit(id.clone()).expect("first admission");
        let first_key = first.key().clone();
        authority.retire(first.key()).expect("retire first");

        assert!(authority.elapse_reuse_horizon_for_test(&id));
        let second = authority.admit(id.clone()).expect("second admission");
        let second_key = second.key().clone();
        authority.retire(second.key()).expect("retire second");
        assert_ne!(first_key.generation, second_key.generation);

        // Both generations now have a deadline at this instant: the original
        // stale entry for generation one and the current entry for generation
        // two. The stale heap entry must never remove the exact newer slot.
        clock.advance(Duration::from_secs(64));
        let third = authority.admit(id).expect("third admission");
        assert_ne!(second_key.generation, third.key().generation);
        assert!(third.is_current());
        assert_eq!(authority.lifecycle_count(), 1);
    }

    #[test]
    fn expired_deadline_for_non_retired_cell_latches_fatal_and_cancels() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 2);
        let lease = authority
            .admit(SessionId::from("invalid-expired-fence"))
            .expect("admission");
        let key = lease.key().clone();
        let cell = authority
            .cells
            .get(&key)
            .map(|cell| Arc::clone(cell.value()))
            .expect("lifecycle cell");
        let cancellation = cell.cancel.subscribe();
        authority.retire(&key).expect("retirement");

        assert!(!authority.cells.contains_key(&key));
        // Inject the impossible split-brain state directly: production code
        // removes the exact full cell before releasing the index lock that
        // publishes a compact retired fence.
        cell.state.lock().expect("cell state").phase = SessionPhase::Active;
        authority.cells.insert(key.clone(), Arc::clone(&cell));
        clock.advance(Duration::from_secs(64));

        assert_eq!(
            authority
                .admit(SessionId::from("detect-invalid-expired-fence"))
                .err(),
            Some(SessionAdmissionError::AuthorityFatal(
                AuthorityFatalReason::InvariantViolation
            ))
        );
        assert!(*cancellation.borrow());
        assert_eq!(
            authority.diagnostics().fatal_reason,
            Some(AuthorityFatalReason::InvariantViolation)
        );
    }

    #[test]
    fn retained_index_admission_examines_only_the_next_or_expired_deadlines() {
        const RETAINED: usize = 1_024;
        const ADDITIONAL: usize = 64;

        let clock = ManualClock::new();
        let config = SessionLifecycleConfig::new(1, Duration::from_secs(64))
            .and_then(|config| config.with_retained_capacity(RETAINED + ADDITIONAL + 1))
            .expect("retained-index operation-count config");
        let authority = SessionLeaseAuthority::with_config(
            AuthorityEpoch::fixed(11),
            config,
            Arc::new(clock.clone()),
        );

        for sequence in 0..RETAINED {
            let lease = authority
                .admit(SessionId::from(format!("retained-{sequence}")))
                .expect("populate retained fence");
            authority
                .retire(lease.key())
                .expect("retire populated fence");
        }
        let _ = take_reuse_deadline_inspections(&authority);

        // A large retained set must add only one earliest-deadline inspection
        // to each admission while no fence is expired. The old full-map scan
        // performed RETAINED inspections per admission here.
        for sequence in 0..ADDITIONAL {
            let lease = authority
                .admit(SessionId::from(format!("additional-{sequence}")))
                .expect("admit with large retained index");
            authority
                .retire(lease.key())
                .expect("retire additional fence");
        }
        assert_eq!(
            take_reuse_deadline_inspections(&authority),
            ADDITIONAL as u64
        );

        // Once the horizon elapses, one admission examines and removes only
        // the deadlines that actually expired. It does not rescan the map.
        clock.advance(Duration::from_secs(64));
        authority
            .admit(SessionId::from("after-expiry"))
            .expect("admit after global expiry");
        assert_eq!(
            take_reuse_deadline_inspections(&authority),
            (RETAINED + ADDITIONAL) as u64
        );
        assert_eq!(authority.lifecycle_count(), 1);
    }

    #[test]
    fn retired_fences_do_not_consume_active_capacity_or_allow_early_same_id_reuse() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 1);
        let first = authority
            .admit(SessionId::from("first"))
            .expect("first admission");
        authority.retire(first.key()).expect("retire first");

        let second = authority
            .admit(SessionId::from("second"))
            .expect("retired fence does not consume the active slot");
        assert!(second.is_current());
        assert_eq!(
            authority.admit(SessionId::from("first")).err(),
            Some(SessionAdmissionError::ReuseBlocked)
        );
    }

    #[test]
    fn active_capacity_recycles_while_retained_fences_stay_bounded_and_exact() {
        let clock = ManualClock::new();
        let config = SessionLifecycleConfig::new(1, Duration::from_secs(64))
            .and_then(|config| config.with_retained_capacity(3))
            .expect("separate active/retained test capacities");
        let authority = SessionLeaseAuthority::with_config(
            AuthorityEpoch::fixed(9),
            config,
            Arc::new(clock.clone()),
        );

        let first = authority
            .admit(SessionId::from("retained-first"))
            .expect("first active slot");
        let first_key = first.key().clone();
        authority.retire(first.key()).expect("retire first");
        for id in ["retained-second", "retained-third"] {
            let lease = authority
                .admit(SessionId::from(id))
                .expect("active slot recycles while prior fences remain");
            authority
                .retire(lease.key())
                .expect("retire churn lifetime");
        }

        let full = authority.diagnostics();
        assert_eq!(full.capacity, 1);
        assert_eq!(full.active_capacity_in_use, 0);
        assert_eq!(full.retained_capacity, 3);
        assert_eq!(full.lifecycle_count, 3);
        assert_eq!(full.retired, 3);
        assert_eq!(
            authority.admit(SessionId::from("retained-fourth")).err(),
            Some(SessionAdmissionError::RetainedCapacityExhausted)
        );
        assert_eq!(
            authority.admit(SessionId::from("retained-first")).err(),
            Some(SessionAdmissionError::ReuseBlocked)
        );

        clock.advance(Duration::from_secs(64));
        let fourth = authority
            .admit(SessionId::from("retained-fourth"))
            .expect("expired fences are globally pruned");
        authority.retire(fourth.key()).expect("retire fourth");
        let first_again = authority
            .admit(SessionId::from("retained-first"))
            .expect("same identifier becomes reusable only after its horizon");
        assert_ne!(first_key.generation, first_again.key().generation);
        assert!(authority.lifecycle_count() <= 3);
    }

    #[test]
    fn epochs_make_restart_generations_distinct() {
        let clock = ManualClock::new();
        let first = SessionLeaseAuthority::with_config(
            AuthorityEpoch::fixed(1),
            SessionLifecycleConfig::new(1, Duration::from_nanos(1)).expect("config"),
            Arc::new(clock.clone()),
        );
        let second = SessionLeaseAuthority::with_config(
            AuthorityEpoch::fixed(2),
            SessionLifecycleConfig::new(1, Duration::from_nanos(1)).expect("config"),
            Arc::new(clock.clone()),
        );

        let first_key = first
            .admit(SessionId::from("restart-id"))
            .expect("first authority")
            .key()
            .clone();
        let second_key = second
            .admit(SessionId::from("restart-id"))
            .expect("second authority")
            .key()
            .clone();

        assert_ne!(first_key.generation, second_key.generation);
    }

    #[test]
    fn capacity_reaps_an_expired_retirement_for_a_different_identifier() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 1);
        let first = authority
            .admit(SessionId::from("expired-first"))
            .expect("first admission");
        authority.retire(first.key()).expect("retire first");
        clock.advance(Duration::from_secs(64));

        authority
            .admit(SessionId::from("unrelated-second"))
            .expect("global retirement reap frees capacity");
        assert_eq!(authority.lifecycle_count(), 1);
    }

    #[test]
    fn exact_generation_operations_do_not_wait_for_the_admission_index() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 1);
        let lease = authority
            .admit(SessionId::from("exact-cell-no-index"))
            .expect("admission");
        let key = lease.key().clone();
        let expected_key = key.clone();
        let held_index = authority.lock_index().expect("admission index");
        let worker_authority = Arc::clone(&authority);
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let resolved = worker_authority.current_key(&key.session_id);
            let current = worker_authority.is_current(&key);
            let phase = worker_authority.phase(&key);
            let operation = worker_authority
                .try_operation_exact(&key, SessionOperationKind::Test("direct-cell"));
            let registered = operation.is_ok();
            if let Ok(operation) = operation {
                operation.finish_rollback();
            }
            result_tx
                .send((resolved, current, phase, registered))
                .expect("result receiver");
        });

        let result = result_rx.recv_timeout(Duration::from_secs(1));
        drop(held_index);
        worker.join().expect("exact-cell worker");
        assert_eq!(
            result.expect("exact access must not wait for the admission index"),
            (Some(expected_key), true, Some(SessionPhase::Active), true)
        );
    }

    #[test]
    fn exact_resource_transitions_do_not_acquire_the_admission_index() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 1);
        let lease = authority
            .admit(SessionId::from("exact-resource-no-index"))
            .expect("admission");
        let key = lease.key().clone();
        let operation = authority
            .register_operation(
                &key,
                SessionOperationKind::Test("direct-resource-cell"),
                None,
            )
            .expect("operation");
        let operation_id = operation.context.operation_id;
        let events = Arc::new(StdMutex::new(Vec::new()));
        let resource = test_resource("captured-without-index", events, []);

        // Hold the raw-ID admission lock for the complete transition sequence.
        // A regression to lock_index would block the worker and increment the
        // instrumentation counter before attempting the mutex acquisition.
        let held_index = authority.lock_index().expect("admission index");
        let acquisitions_before = authority.admission_index_lock_acquisitions();
        let worker_authority = Arc::clone(&authority);
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let reserved = worker_authority
                .reserve_resource(
                    &key,
                    operation_id,
                    resource_spec("reserved-without-index", Vec::new()),
                )
                .and_then(|attempt| attempt.confirm_unused());
            let installed_unused = worker_authority
                .reserve_resource(
                    &key,
                    operation_id,
                    resource_spec("installed-unused-without-index", Vec::new()),
                )
                .and_then(ResourceInstallAttempt::dispatch)
                .map(ResourceDispatchPermit::into_installation_sink)
                .and_then(ResourceInstallationSink::confirm_unused);
            let captured = worker_authority
                .reserve_resource(
                    &key,
                    operation_id,
                    resource_spec("captured-without-index", Vec::new()),
                )
                .and_then(ResourceInstallAttempt::dispatch)
                .map(ResourceDispatchPermit::into_installation_sink)
                .and_then(|sink| sink.capture_at_install(resource).map(|_| ()));
            let committed = captured.and_then(|()| {
                worker_authority
                    .record_operation_commit(&key, operation_id)
                    .map_err(|error| match error {
                        SessionOperationError::AuthorityFatal(reason) => {
                            ResourceRegistryError::AuthorityFatal(reason)
                        }
                        SessionOperationError::StaleGeneration => {
                            ResourceRegistryError::StaleGeneration
                        }
                        SessionOperationError::OperationSequenceExhausted
                        | SessionOperationError::SupervisorUnavailable
                        | SessionOperationError::InvalidTimeout
                        | SessionOperationError::DeadlineOverflow
                        | SessionOperationError::AuthorityDraining
                        | SessionOperationError::ResourcesUnresolved
                        | SessionOperationError::ResourceRollbackFailed => {
                            ResourceRegistryError::InvalidState
                        }
                    })
            });
            if let Ok(revision) = committed {
                operation.finish(CompletionDisposition::Committed(revision));
            }
            result_tx
                .send((reserved, installed_unused, committed.map(|_| ())))
                .expect("result receiver");
        });

        let result = result_rx.recv_timeout(Duration::from_secs(1));
        let acquisitions_after = authority.admission_index_lock_acquisitions();
        drop(held_index);
        worker.join().expect("exact resource worker");

        assert_eq!(
            result.expect("exact resource transitions must not wait for admission"),
            (Ok(()), Ok(()), Ok(()))
        );
        assert_eq!(
            acquisitions_after, acquisitions_before,
            "normal exact resource transitions must not request the admission lock"
        );
    }

    #[test]
    fn stale_resource_owner_cannot_target_a_reused_raw_identifier() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 1);
        let session_id = SessionId::from("resource-owner-reuse");
        let first = authority
            .admit(session_id.clone())
            .expect("first admission");
        let stale_key = first.key().clone();
        let stale_operation = authority
            .register_operation(
                &stale_key,
                SessionOperationKind::Test("stale-resource-owner"),
                None,
            )
            .expect("first operation");
        let stale_operation_id = stale_operation.context.operation_id;
        stale_operation.finish(CompletionDisposition::RolledBack);
        authority.retire(&stale_key).expect("retire first lifetime");
        clock.advance(Duration::from_secs(64));

        let second = authority
            .admit(session_id)
            .expect("reuse after anti-reuse horizon");
        assert_ne!(stale_key.generation, second.key().generation);
        let current_operation = authority
            .register_operation(
                second.key(),
                SessionOperationKind::Test("current-resource-owner"),
                None,
            )
            .expect("current operation");
        let current_operation_id = current_operation.context.operation_id;
        let current_attempt = authority
            .reserve_resource(
                second.key(),
                current_operation_id,
                resource_spec("current-generation", Vec::new()),
            )
            .expect("current reservation");

        assert_eq!(
            authority
                .reserve_resource(
                    &stale_key,
                    stale_operation_id,
                    resource_spec("stale-generation", Vec::new()),
                )
                .err(),
            Some(ResourceRegistryError::StaleGeneration)
        );
        assert_eq!(
            authority
                .confirm_resource_unused(&stale_key, stale_operation_id, current_attempt.id(),),
            Err(ResourceRegistryError::StaleGeneration)
        );

        current_attempt
            .confirm_unused()
            .expect("current owner remains authoritative");
        current_operation.finish(CompletionDisposition::RolledBack);
        assert_eq!(authority.diagnostics().resources.total, 0);
        assert!(authority.is_current(second.key()));
    }

    #[tokio::test]
    async fn quiesce_is_an_atomic_admission_barrier_and_teardown_is_single_flight() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 2);
        let lease = authority
            .admit(SessionId::from("quiesce-barrier"))
            .expect("admission");
        let guard = authority
            .try_operation_exact(lease.key(), SessionOperationKind::Test("barrier"))
            .expect("operation");
        let cancellation = guard.cancellation().expect("cancellation receiver");

        let first = authority
            .teardown(lease.key(), Duration::from_secs(1))
            .expect("first teardown");
        let second = authority
            .teardown(lease.key(), Duration::from_secs(1))
            .expect("coalesced teardown");
        assert!(*cancellation.borrow());
        assert!(!lease.is_current());
        assert_eq!(
            authority
                .try_operation_exact(lease.key(), SessionOperationKind::Test("too-late"))
                .err(),
            Some(SessionOperationError::StaleGeneration)
        );

        guard.finish_rollback();
        let (first, second) = tokio::join!(first.wait(), second.wait());
        assert_eq!(
            first.expect("first outcome"),
            second.expect("second outcome")
        );
        assert!(matches!(
            authority.phase(lease.key()),
            Some(SessionPhase::Retired { .. })
        ));
        authority
            .wait_for_supervisors()
            .await
            .expect("teardown supervisor cleanup");
        assert!(
            !authority.cells.contains_key(lease.key()),
            "completed teardown must retain only the compact fence"
        );

        let repeated = authority
            .teardown(lease.key(), Duration::from_secs(1))
            .expect("retired teardown remains idempotent");
        let repeated_supervisor = repeated.clone();
        assert!(matches!(
            repeated.wait().await.expect("retired outcome"),
            TeardownOutcome::Retired { .. }
        ));
        repeated_supervisor
            .wait_supervisor()
            .await
            .expect("compact fence has no supervisor");
    }

    #[tokio::test]
    async fn commit_permit_fails_after_quiesce_and_exact_rollback_can_release() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 1);
        let lease = authority
            .admit(SessionId::from("commit-barrier"))
            .expect("admission");
        let guard = lease
            .try_operation(SessionOperationKind::Test("commit"))
            .expect("operation");
        let permit = guard.prepare_commit().expect("prepare while active");
        let teardown = authority
            .teardown(lease.key(), Duration::from_secs(1))
            .expect("teardown");

        assert_eq!(permit.finish(), Err(SessionOperationError::StaleGeneration));
        guard.finish_rollback();
        assert!(matches!(
            teardown.wait().await.expect("teardown outcome"),
            TeardownOutcome::Retired { .. }
        ));
    }

    #[tokio::test]
    async fn failed_guard_finish_preserves_registration_until_compensation_rolls_back() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 1);
        let lease = authority
            .admit(SessionId::from("guard-finish-compensation"))
            .expect("admission");
        let guard = authority
            .try_operation_exact(
                lease.key(),
                SessionOperationKind::Test("compensating-finish"),
            )
            .expect("operation");
        let teardown = authority
            .teardown(lease.key(), Duration::from_secs(1))
            .expect("teardown");

        let failure = guard.finish().expect_err("commit must lose to quiesce");
        assert_eq!(failure.error(), SessionOperationError::StaleGeneration);
        assert_eq!(authority.diagnostics().operations.total, 1);
        let mut outcome = Box::pin(teardown.clone().wait());
        assert!(futures::poll!(&mut outcome).is_pending());

        // A registry receipt compensates its local/external mappings here,
        // while the failure still owns the authority registration.
        failure.into_guard().finish_rollback();
        assert!(matches!(
            outcome.await.expect("retirement after compensation"),
            TeardownOutcome::Retired { .. }
        ));
    }

    #[tokio::test]
    async fn dropping_owned_waiter_does_not_cancel_the_supervised_operation() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 1);
        let lease = authority
            .admit(SessionId::from("detached-waiter"))
            .expect("admission");
        let (release_tx, release_rx) = oneshot::channel::<()>();
        let waiter = lease
            .spawn_owned(
                SessionOperationKind::Test("detached"),
                Duration::from_secs(1),
                move |operation| async move {
                    let _ = release_rx.await;
                    operation.rollback(()).await.expect("exact rollback")
                },
            )
            .expect("owned operation");
        drop(waiter);
        let teardown = authority
            .teardown(lease.key(), Duration::from_secs(1))
            .expect("teardown");
        release_tx.send(()).expect("release operation");

        assert!(matches!(
            teardown.wait().await.expect("teardown outcome"),
            TeardownOutcome::Retired { .. }
        ));
    }

    #[tokio::test]
    async fn operation_deadline_reports_quarantine_but_retains_exact_rollback() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 1);
        let lease = authority
            .admit(SessionId::from("deadline-rollback"))
            .expect("admission");
        let (release_tx, release_rx) = oneshot::channel::<()>();
        let waiter = lease
            .spawn_owned(
                SessionOperationKind::Test("deadline"),
                Duration::from_secs(1),
                move |operation| async move {
                    let _ = release_rx.await;
                    operation.rollback(()).await.expect("exact rollback")
                },
            )
            .expect("owned operation");
        tokio::task::yield_now().await;
        clock.advance(Duration::from_secs(1));
        assert_eq!(waiter.await, Err(OwnedOperationError::DeadlineExceeded));
        let teardown = authority
            .teardown(lease.key(), Duration::from_millis(10))
            .expect("teardown");
        assert!(matches!(
            teardown.wait().await.expect("quarantine outcome"),
            TeardownOutcome::Quarantined {
                reason: QuarantineReason::OperationDeadline,
                ..
            }
        ));
        release_tx.send(()).expect("finish exact rollback");
        authority
            .wait_for_supervisors()
            .await
            .expect("retained rollback reaper");
        assert!(matches!(
            authority.phase(lease.key()),
            Some(SessionPhase::Retired { .. })
        ));
        assert!(!authority.cells.contains_key(lease.key()));
        let late = authority
            .teardown(lease.key(), Duration::from_millis(10))
            .expect("late idempotent teardown");
        assert!(matches!(
            late.wait().await.expect("preserved quarantine outcome"),
            TeardownOutcome::Quarantined {
                reason: QuarantineReason::OperationDeadline,
                ..
            }
        ));
        assert_eq!(
            authority
                .diagnostics()
                .quarantine_reasons
                .operation_deadline,
            1
        );
    }

    #[tokio::test]
    async fn panic_after_deadline_remains_sticky_and_non_reusable() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 1);
        let id = SessionId::from("deadline-panic");
        let lease = authority.admit(id.clone()).expect("admission");
        let (panic_tx, panic_rx) = oneshot::channel::<()>();
        let waiter: OperationWaiter<()> = lease
            .spawn_owned(
                SessionOperationKind::Test("panic-after-deadline"),
                Duration::from_secs(1),
                |operation| async move {
                    // Keep the exact operation guard owned by the future
                    // until the injected panic unwinds it.
                    let _operation = operation;
                    let _ = panic_rx.await;
                    panic!("post-deadline creator panic")
                },
            )
            .expect("owned operation");
        tokio::task::yield_now().await;
        clock.advance(Duration::from_secs(1));
        assert_eq!(waiter.await, Err(OwnedOperationError::DeadlineExceeded));
        let teardown = authority
            .teardown(lease.key(), Duration::from_millis(5))
            .expect("teardown");
        assert!(matches!(
            teardown.wait().await.expect("quarantine outcome"),
            TeardownOutcome::Quarantined { .. }
        ));
        panic_tx.send(()).expect("trigger retained panic");
        authority
            .wait_for_supervisors()
            .await
            .expect("panic supervisors end");
        assert!(matches!(
            authority.phase(lease.key()),
            Some(SessionPhase::Quarantined { .. })
        ));
        clock.advance(Duration::from_secs(128));
        assert_eq!(
            authority.admit(id).err(),
            Some(SessionAdmissionError::ReuseBlocked)
        );
    }

    #[tokio::test]
    async fn abandoned_synchronous_guard_is_fail_closed() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 1);
        let lease = authority
            .admit(SessionId::from("abandoned-sync"))
            .expect("admission");
        let guard = lease
            .try_operation(SessionOperationKind::Test("abandoned"))
            .expect("operation");
        drop(guard);

        let teardown = authority
            .teardown(lease.key(), Duration::from_millis(10))
            .expect("teardown");
        assert!(matches!(
            teardown.wait().await.expect("quarantine outcome"),
            TeardownOutcome::Quarantined {
                reason: QuarantineReason::OperationAbandoned,
                ..
            }
        ));
    }

    #[test]
    fn lifecycle_configuration_rejects_zero_limits() {
        assert_eq!(
            SessionLifecycleConfig::new(0, Duration::from_secs(1)),
            Err(SessionLifecycleConfigError::ZeroCapacity)
        );
        assert_eq!(
            SessionLifecycleConfig::new(1, Duration::ZERO),
            Err(SessionLifecycleConfigError::ZeroAntiReuseHorizon)
        );
        assert_eq!(
            SessionLifecycleConfig::new(1, Duration::from_secs(1))
                .expect("base config")
                .with_supervisor_abort_grace(Duration::ZERO),
            Err(SessionLifecycleConfigError::ZeroSupervisorAbortGrace)
        );
        assert_eq!(
            SessionLifecycleConfig::new(1, Duration::from_secs(1))
                .expect("base config")
                .with_resource_capacity_per_session(0),
            Err(SessionLifecycleConfigError::ZeroResourceCapacity)
        );
        assert_eq!(
            SessionLifecycleConfig::new(1, Duration::from_secs(1))
                .expect("base config")
                .with_retained_capacity(0),
            Err(SessionLifecycleConfigError::ZeroRetainedCapacity)
        );
        assert_eq!(
            SessionLifecycleConfig::new(2, Duration::from_secs(1))
                .expect("base config")
                .with_retained_capacity(1),
            Err(SessionLifecycleConfigError::RetainedCapacityBelowActive)
        );
    }

    #[tokio::test]
    async fn timeout_arithmetic_is_checked() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 1);
        let lease = authority
            .admit(SessionId::from("deadline-overflow"))
            .expect("admission");

        assert_eq!(
            lease
                .spawn_owned(
                    SessionOperationKind::Test("overflow"),
                    Duration::MAX,
                    |operation| async move {
                        operation.rollback(()).await.expect("exact rollback")
                    },
                )
                .err(),
            Some(SessionOperationError::DeadlineOverflow)
        );
        assert_eq!(
            authority.teardown(lease.key(), Duration::ZERO).err(),
            Some(SessionOperationError::InvalidTimeout)
        );
    }

    #[test]
    fn retirement_horizon_overflow_latches_authority_fatal() {
        let clock = ManualClock::new();
        let authority = SessionLeaseAuthority::with_config(
            AuthorityEpoch::fixed(7),
            SessionLifecycleConfig::new(1, Duration::MAX).expect("large horizon config"),
            Arc::new(clock),
        );
        let lease = authority
            .admit(SessionId::from("retirement-overflow"))
            .expect("admission");

        assert_eq!(
            authority.retire(lease.key()),
            Err(SessionAdmissionError::AuthorityFatal(
                AuthorityFatalReason::RetirementDeadlineOverflow
            ))
        );
        assert_eq!(
            authority
                .admit(SessionId::from("rejected-after-overflow"))
                .err(),
            Some(SessionAdmissionError::AuthorityFatal(
                AuthorityFatalReason::RetirementDeadlineOverflow
            ))
        );
    }

    #[test]
    fn poisoned_index_fails_closed_broadcasts_cancel_and_drop_does_not_panic() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 2);
        let lease = authority
            .admit(SessionId::from("poisoned-index"))
            .expect("admission");
        let guard = lease
            .try_operation(SessionOperationKind::Test("poisoned-drop"))
            .expect("operation");
        let cancellation = guard.cancellation().expect("cancellation");

        let poisoned = std::panic::catch_unwind(AssertUnwindSafe({
            let authority = Arc::clone(&authority);
            move || {
                let _index = authority.index.lock().expect("index lock");
                panic!("poison index");
            }
        }));
        assert!(poisoned.is_err());
        assert_eq!(
            authority.admit(SessionId::from("detect-poison")).err(),
            Some(SessionAdmissionError::AuthorityFatal(
                AuthorityFatalReason::IndexPoisoned
            ))
        );
        assert!(*cancellation.borrow());
        assert!(std::panic::catch_unwind(AssertUnwindSafe(|| drop(guard))).is_ok());

        let diagnostics = authority.diagnostics();
        assert!(!diagnostics.complete);
        assert_eq!(
            diagnostics.fatal_reason,
            Some(AuthorityFatalReason::IndexPoisoned)
        );
    }

    #[tokio::test]
    async fn quiesce_timeout_reports_then_exact_rollback_retires() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 1);
        let lease = authority
            .admit(SessionId::from("quiesce-timeout"))
            .expect("admission");
        let guard = lease
            .try_operation(SessionOperationKind::Test("slow-rollback"))
            .expect("operation");
        let teardown = authority
            .teardown(lease.key(), Duration::from_secs(1))
            .expect("teardown");
        let supervisor = teardown.clone();

        tokio::task::yield_now().await;
        clock.advance(Duration::from_secs(1));
        assert!(matches!(
            teardown.wait().await.expect("quarantine outcome"),
            TeardownOutcome::Quarantined {
                reason: QuarantineReason::QuiesceDeadline,
                ..
            }
        ));
        guard.finish_rollback();
        supervisor
            .wait_supervisor()
            .await
            .expect("retained teardown supervisor");
        assert!(matches!(
            authority.phase(lease.key()),
            Some(SessionPhase::Retired { .. })
        ));
    }

    #[tokio::test]
    async fn aborted_teardown_supervisor_runs_drop_bomb_before_registry_removal() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 1);
        let lease = authority
            .admit(SessionId::from("abort-teardown"))
            .expect("admission");
        let guard = lease
            .try_operation(SessionOperationKind::Test("hold-teardown"))
            .expect("operation");
        let teardown = authority
            .teardown(lease.key(), Duration::from_secs(10))
            .expect("teardown");

        assert_eq!(
            authority
                .abort_supervisors_for_key(lease.key(), SupervisorKind::Teardown)
                .expect("abort lookup"),
            1
        );
        assert!(matches!(
            teardown.wait().await.expect("bomb outcome"),
            TeardownOutcome::Quarantined {
                reason: QuarantineReason::SupervisorAbandoned,
                ..
            }
        ));
        authority
            .wait_for_supervisors()
            .await
            .expect("registry removal");
        guard.finish_rollback();
        assert!(matches!(
            authority.phase(lease.key()),
            Some(SessionPhase::Quarantined { .. })
        ));
    }

    #[tokio::test]
    async fn teardown_bomb_cannot_overwrite_an_atomic_retirement() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 1);
        let lease = authority
            .admit(SessionId::from("terminal-window"))
            .expect("admission");
        let teardown = authority
            .teardown(lease.key(), Duration::from_secs(1))
            .expect("teardown");
        let supervisor = teardown.clone();
        assert!(matches!(
            teardown.wait().await.expect("retired outcome"),
            TeardownOutcome::Retired { .. }
        ));
        supervisor.wait_supervisor().await.expect("driver complete");

        let registration = authority
            .register_supervisor(lease.key(), SupervisorKind::Teardown)
            .expect("synthetic terminal-window supervisor");
        drop(TeardownSupervisorGuard {
            authority: Arc::clone(&authority),
            key: lease.key().clone(),
            registration: Some(registration),
            armed: true,
        });

        assert!(matches!(
            authority.phase(lease.key()),
            Some(SessionPhase::Retired { .. })
        ));
        assert!(
            !authority.cells.contains_key(lease.key()),
            "a stale teardown bomb cannot recreate a compacted cell"
        );
        let replay = authority
            .teardown(lease.key(), Duration::from_secs(1))
            .expect("retired outcome remains published");
        assert!(matches!(
            replay.wait().await.expect("retired replay outcome"),
            TeardownOutcome::Retired { .. }
        ));
    }

    #[tokio::test]
    async fn committed_before_quiesce_can_complete_after_quiesce() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 1);
        let lease = authority
            .admit(SessionId::from("commit-linearization"))
            .expect("admission");
        let (committed_tx, committed_rx) = oneshot::channel::<()>();
        let (complete_tx, complete_rx) = oneshot::channel::<()>();
        let operation = lease
            .spawn_owned(
                SessionOperationKind::StateTransition,
                Duration::from_secs(10),
                move |operation| async move {
                    let completion = operation
                        .commit()
                        .expect("atomic commit proof")
                        .complete("committed");
                    let _ = committed_tx.send(());
                    let _ = complete_rx.await;
                    completion
                },
            )
            .expect("owned operation");
        committed_rx.await.expect("commit linearized");

        let teardown = authority
            .teardown(lease.key(), Duration::from_secs(1))
            .expect("teardown");
        complete_tx.send(()).expect("release completion");
        assert_eq!(operation.await, Ok("committed"));
        assert!(matches!(
            teardown.wait().await.expect("retirement"),
            TeardownOutcome::Retired { .. }
        ));
    }

    #[tokio::test]
    async fn resource_reservation_is_visible_before_lower_install_await() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 1);
        let lease = authority
            .admit(SessionId::from("resource-reserve-before-await"))
            .expect("admission");
        let (reserved_tx, reserved_rx) = oneshot::channel();
        let (continue_tx, continue_rx) = oneshot::channel();
        let operation = lease
            .spawn_owned(
                SessionOperationKind::Signaling,
                Duration::from_secs(10),
                move |mut operation| async move {
                    let attempt = operation
                        .reserve_resource(resource_spec("reserved", Vec::new()))
                        .expect("reserve before install");
                    let _ = reserved_tx.send(attempt.id());
                    let _ = continue_rx.await;
                    attempt.confirm_unused().expect("proved unused");
                    operation.rollback(()).await.expect("exact rollback")
                },
            )
            .expect("owned operation");

        let _resource_id = reserved_rx.await.expect("reservation visible");
        let diagnostics = authority.diagnostics();
        assert_eq!(diagnostics.resource_capacity_per_session, 1_024);
        assert_eq!(diagnostics.resources.total, 1);
        assert_eq!(diagnostics.resources.reserved, 1);
        continue_tx.send(()).expect("continue install");
        assert_eq!(operation.await, Ok(()));
        assert_eq!(authority.diagnostics().resources.total, 0);
    }

    #[tokio::test]
    async fn managed_resource_capacity_is_a_recoverable_operation_error() {
        let clock = ManualClock::new();
        let authority = SessionLeaseAuthority::with_config(
            AuthorityEpoch::fixed(7),
            SessionLifecycleConfig::new(1, Duration::from_secs(64))
                .and_then(|config| config.with_resource_capacity_per_session(1))
                .expect("one-resource configuration"),
            Arc::new(clock),
        );
        let lease = authority
            .admit(SessionId::from("resource-capacity-recoverable"))
            .expect("admission");
        let operation = lease
            .spawn_owned(
                SessionOperationKind::Media,
                Duration::from_secs(10),
                move |mut operation| async move {
                    let first = operation
                        .reserve_resource(resource_spec("capacity-first", Vec::new()))
                        .expect("first reservation");
                    let second =
                        operation.reserve_resource(resource_spec("capacity-overload", Vec::new()));
                    first.confirm_unused().expect("release first reservation");
                    operation
                        .rollback(second.err())
                        .await
                        .expect("recoverable rollback")
                },
            )
            .expect("owned operation");

        assert_eq!(
            operation.await,
            Ok(Some(ResourceRegistryError::CapacityExhausted))
        );
        assert_eq!(authority.diagnostics().resources.total, 0);
    }

    #[tokio::test]
    async fn pre_dispatch_quiesce_confirms_reservation_unused_for_clean_rollback() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 1);
        let lease = authority
            .admit(SessionId::from("resource-dispatch-versus-quiesce"))
            .expect("admission");
        let (reserved_tx, reserved_rx) = oneshot::channel();
        let (dispatch_tx, dispatch_rx) = oneshot::channel();
        let operation = lease
            .spawn_owned(
                SessionOperationKind::Media,
                Duration::from_secs(10),
                move |mut operation| async move {
                    let attempt = operation
                        .reserve_resource(resource_spec("dispatch-race", Vec::new()))
                        .expect("reserve before quiesce");
                    let _ = reserved_tx.send(());
                    let _ = dispatch_rx.await;
                    let error = match attempt.dispatch() {
                        Ok(_) => panic!("dispatch must lose to quiesce"),
                        Err(error) => error,
                    };
                    operation
                        .rollback(error)
                        .await
                        .expect("clean exact rollback")
                },
            )
            .expect("owned operation");
        reserved_rx.await.expect("reservation visible");

        let teardown = authority
            .teardown(lease.key(), Duration::from_secs(1))
            .expect("teardown");
        dispatch_tx.send(()).expect("attempt dispatch");

        assert_eq!(operation.await, Ok(ResourceRegistryError::NotActive));
        assert!(matches!(
            teardown.wait().await.expect("retirement"),
            TeardownOutcome::Retired { .. }
        ));
        assert_eq!(authority.diagnostics().resources.total, 0);
    }

    #[tokio::test]
    async fn installation_sink_can_capture_after_quiesce_at_lower_mutation_point() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 1);
        let lease = authority
            .admit(SessionId::from("install-versus-quiesce"))
            .expect("admission");
        let events = Arc::new(StdMutex::new(Vec::new()));
        let resource = test_resource("late-capture", Arc::clone(&events), []);
        let (installing_tx, installing_rx) = oneshot::channel();
        let (mutate_tx, mutate_rx) = oneshot::channel();
        let operation = lease
            .spawn_owned(
                SessionOperationKind::Signaling,
                Duration::from_secs(10),
                move |mut operation| async move {
                    let sink = operation
                        .reserve_resource(resource_spec("late-capture", Vec::new()))
                        .expect("reservation")
                        .dispatch()
                        .expect("dispatch")
                        .into_installation_sink();
                    let _ = installing_tx.send(sink.id());
                    let _ = mutate_rx.await;
                    sink.capture_at_install(resource)
                        .expect("capture at lower mutation");
                    operation.rollback(()).await.expect("exact rollback")
                },
            )
            .expect("owned operation");
        let _ = installing_rx.await.expect("install dispatched");

        let teardown = authority
            .teardown(lease.key(), Duration::from_secs(1))
            .expect("teardown");
        assert_eq!(authority.diagnostics().resources.installing, 1);
        mutate_tx.send(()).expect("perform lower mutation");
        assert_eq!(operation.await, Ok(()));
        assert!(matches!(
            teardown.wait().await.expect("teardown outcome"),
            TeardownOutcome::Retired { .. }
        ));
        assert!(events
            .lock()
            .expect("events")
            .iter()
            .any(|event| event == "release:late-capture"));
    }

    #[tokio::test]
    async fn committed_resource_promotes_before_quiesce_and_releases_after_completion() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 1);
        let lease = authority
            .admit(SessionId::from("resource-commit-linearization"))
            .expect("admission");
        let events = Arc::new(StdMutex::new(Vec::new()));
        let resource = test_resource("committed", Arc::clone(&events), []);
        let (committed_tx, committed_rx) = oneshot::channel();
        let (complete_tx, complete_rx) = oneshot::channel();
        let operation = lease
            .spawn_owned(
                SessionOperationKind::StateTransition,
                Duration::from_secs(10),
                move |mut operation| async move {
                    operation
                        .reserve_resource(resource_spec("committed", Vec::new()))
                        .expect("reservation")
                        .dispatch()
                        .expect("dispatch")
                        .into_installation_sink()
                        .capture_at_install(resource)
                        .expect("capture");
                    let completion = operation
                        .commit()
                        .expect("atomic resource commit")
                        .complete("committed");
                    let _ = committed_tx.send(());
                    let _ = complete_rx.await;
                    completion
                },
            )
            .expect("owned operation");
        committed_rx.await.expect("commit linearized");
        assert_eq!(authority.diagnostics().resources.live, 1);

        let teardown = authority
            .teardown(lease.key(), Duration::from_secs(1))
            .expect("teardown");
        complete_tx.send(()).expect("complete operation");
        assert_eq!(operation.await, Ok("committed"));
        assert!(matches!(
            teardown.wait().await.expect("retirement"),
            TeardownOutcome::Retired { .. }
        ));
        assert_eq!(authority.diagnostics().resources.total, 0);
        assert!(events
            .lock()
            .expect("events")
            .iter()
            .any(|event| event == "release:committed"));
    }

    #[tokio::test]
    async fn abort_after_lower_capture_retains_and_releases_exact_resource() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 1);
        let lease = authority
            .admit(SessionId::from("abort-after-resource-capture"))
            .expect("admission");
        let events = Arc::new(StdMutex::new(Vec::new()));
        let resource = test_resource("captured-before-abort", Arc::clone(&events), []);
        let (captured_tx, captured_rx) = oneshot::channel();
        let operation = lease
            .spawn_owned(
                SessionOperationKind::Signaling,
                Duration::from_secs(30),
                move |mut operation| async move {
                    operation
                        .reserve_resource(resource_spec("captured-before-abort", Vec::new()))
                        .expect("reservation")
                        .dispatch()
                        .expect("dispatch")
                        .into_installation_sink()
                        .capture_at_install(resource)
                        .expect("capture at mutation");
                    let _ = captured_tx.send(());
                    futures::future::pending::<OwnedOperationCompletion<()>>().await
                },
            )
            .expect("owned operation");
        captured_rx.await.expect("resource captured");
        let teardown = authority
            .teardown(lease.key(), Duration::from_secs(10))
            .expect("teardown");
        let supervisor = teardown.clone();

        assert_eq!(
            authority
                .abort_supervisors_for_key(lease.key(), SupervisorKind::OwnedOperation)
                .expect("abort operation"),
            1
        );
        assert_eq!(operation.await, Err(OwnedOperationError::SupervisorDropped));
        assert!(matches!(
            teardown.wait().await.expect("permanent fence reported"),
            TeardownOutcome::Quarantined {
                reason: QuarantineReason::OperationAbandoned,
                ..
            }
        ));
        supervisor
            .wait_supervisor()
            .await
            .expect("resource cleanup driver");
        assert_eq!(authority.diagnostics().resources.total, 0);
        assert!(events
            .lock()
            .expect("events")
            .iter()
            .any(|event| event == "release:captured-before-abort"));
    }

    #[tokio::test]
    async fn rollback_receipt_is_exact_and_wrong_descriptor_cannot_discharge_capacity() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 1);
        let lease = authority
            .admit(SessionId::from("rollback-receipt-isolation"))
            .expect("admission");
        let events = Arc::new(StdMutex::new(Vec::new()));
        let resource = test_resource("receipt", Arc::clone(&events), []);
        let operation = lease
            .spawn_owned(
                SessionOperationKind::StateTransition,
                Duration::from_secs(10),
                move |mut operation| async move {
                    operation
                        .reserve_resource(resource_spec("receipt", Vec::new()))
                        .expect("reservation")
                        .dispatch()
                        .expect("dispatch")
                        .into_installation_sink()
                        .capture_at_install(resource)
                        .expect("capture");
                    operation.commit().expect("commit").complete(())
                },
            )
            .expect("owned operation");
        assert_eq!(operation.await, Ok(()));

        let work = match authority
            .plan_resource_release(lease.key(), None)
            .expect("release plan")
        {
            ResourceReleasePlan::Wave(mut wave) => {
                assert_eq!(wave.len(), 1);
                wave.pop().expect("release work")
            }
            _ => panic!("expected release wave"),
        };
        let resource_id = work.resource_id;
        assert_eq!(
            authority.apply_rollback_receipt(RollbackReceipt {
                key: lease.key().clone(),
                resource_id,
                descriptor: ResourceDescriptor::new("test", "wrong"),
            }),
            Err(SessionOperationError::ResourceRollbackFailed)
        );
        let diagnostics = authority.diagnostics();
        assert_eq!(diagnostics.resources.total, 1);
        assert_eq!(diagnostics.resources.releasing, 1);

        let (receipt, mut drop_bomb) = match authority
            .release_one_resource(lease.key().clone(), work)
            .await
        {
            ResourceReleaseAttempt::Released { receipt, drop_bomb } => (receipt, drop_bomb),
            _ => panic!("exact release must produce private receipt"),
        };
        authority
            .apply_rollback_receipt(receipt)
            .expect("exact receipt accepted");
        drop_bomb.disarm();
        assert_eq!(authority.diagnostics().resources.total, 0);
        authority.retire(lease.key()).expect("retire after release");
    }

    #[tokio::test]
    async fn concurrent_release_drivers_invoke_exact_resource_once() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 1);
        let lease = authority
            .admit(SessionId::from("concurrent-resource-release"))
            .expect("admission");
        let events = Arc::new(StdMutex::new(Vec::new()));
        let (release_tx, resource) = gated_test_resource("single-release", Arc::clone(&events));
        let attempts = Arc::clone(&resource.release_attempts);
        let operation = lease
            .spawn_owned(
                SessionOperationKind::StateTransition,
                Duration::from_secs(10),
                move |mut operation| async move {
                    operation
                        .reserve_resource(resource_spec("single-release", Vec::new()))
                        .expect("reservation")
                        .dispatch()
                        .expect("dispatch")
                        .into_installation_sink()
                        .capture_at_install(resource)
                        .expect("capture");
                    operation.commit().expect("commit").complete(())
                },
            )
            .expect("owned operation");
        assert_eq!(operation.await, Ok(()));

        let mut first = Box::pin(authority.release_resources(lease.key(), None));
        assert!(futures::poll!(&mut first).is_pending());
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        assert_eq!(authority.diagnostics().resources.releasing, 1);
        let mut second = Box::pin(authority.release_resources(lease.key(), None));
        assert!(futures::poll!(&mut second).is_pending());
        assert_eq!(attempts.load(Ordering::SeqCst), 1);

        release_tx.send(()).expect("finish exact release");
        let (first, second) = tokio::join!(first, second);
        assert_eq!(first, Ok(()));
        assert_eq!(second, Ok(()));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        assert_eq!(authority.diagnostics().resources.total, 0);
        authority.retire(lease.key()).expect("retire after release");
    }

    #[tokio::test]
    async fn aborting_blocked_release_driver_converts_releasing_to_charged_orphan() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 1);
        let lease = authority
            .admit(SessionId::from("aborted-resource-release-driver"))
            .expect("admission");
        let events = Arc::new(StdMutex::new(Vec::new()));
        let (release_tx, resource) = gated_test_resource("blocked-release", Arc::clone(&events));
        let attempts = Arc::clone(&resource.release_attempts);
        let (captured_tx, captured_rx) = oneshot::channel();
        let operation = lease
            .spawn_owned(
                SessionOperationKind::Signaling,
                Duration::from_secs(30),
                move |mut operation| async move {
                    operation
                        .reserve_resource(resource_spec("blocked-release", Vec::new()))
                        .expect("reservation")
                        .dispatch()
                        .expect("dispatch")
                        .into_installation_sink()
                        .capture_at_install(resource)
                        .expect("capture");
                    let _ = captured_tx.send(());
                    operation.rollback(()).await.expect("release rollback")
                },
            )
            .expect("owned operation");
        captured_rx.await.expect("resource captured");
        while attempts.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        assert_eq!(authority.diagnostics().resources.releasing, 1);
        let teardown = authority
            .teardown(lease.key(), Duration::from_secs(10))
            .expect("teardown");
        let supervisor = teardown.clone();

        assert_eq!(
            authority
                .abort_supervisors_for_key(lease.key(), SupervisorKind::OwnedOperation)
                .expect("abort release driver"),
            1
        );
        assert_eq!(operation.await, Err(OwnedOperationError::SupervisorDropped));
        let _ = release_tx.send(());
        loop {
            let diagnostics = authority.diagnostics();
            if diagnostics.resources.orphaned == 1 {
                assert_eq!(diagnostics.resources.releasing, 0);
                assert_eq!(diagnostics.resources.total, 1);
                assert_eq!(
                    diagnostics.resources.orphan_reasons.release_driver_dropped,
                    1
                );
                assert_eq!(diagnostics.teardown_supervisors, 1);
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(matches!(
            teardown.wait().await.expect("permanent operation fence"),
            TeardownOutcome::Quarantined { .. }
        ));

        assert_eq!(
            authority
                .retry_orphaned_resources(lease.key())
                .expect("explicit ambiguous-release retry"),
            1
        );
        supervisor
            .wait_supervisor()
            .await
            .expect("retained teardown driver completed cleanup");
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert_eq!(authority.diagnostics().resources.total, 0);
    }

    #[tokio::test]
    async fn dependency_release_is_reverse_topological() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 1);
        let lease = authority
            .admit(SessionId::from("resource-dependency-order"))
            .expect("admission");
        let events = Arc::new(StdMutex::new(Vec::new()));
        let parent = test_resource("parent", Arc::clone(&events), []);
        let child = test_resource("child", Arc::clone(&events), []);
        let operation = lease
            .spawn_owned(
                SessionOperationKind::Signaling,
                Duration::from_secs(10),
                move |mut operation| async move {
                    let parent_attempt = operation
                        .reserve_resource(resource_spec("parent", Vec::new()))
                        .expect("parent reservation");
                    let parent_id = parent_attempt.id();
                    parent_attempt
                        .dispatch()
                        .expect("parent dispatch")
                        .into_installation_sink()
                        .capture_at_install(parent)
                        .expect("parent capture");
                    operation
                        .reserve_resource(resource_spec("child", vec![parent_id]))
                        .expect("child reservation")
                        .dispatch()
                        .expect("child dispatch")
                        .into_installation_sink()
                        .capture_at_install(child)
                        .expect("child capture");
                    operation.rollback(()).await.expect("ordered rollback")
                },
            )
            .expect("owned operation");
        assert_eq!(operation.await, Ok(()));
        let releases: Vec<_> = events
            .lock()
            .expect("events")
            .iter()
            .filter(|event| event.starts_with("release:"))
            .cloned()
            .collect();
        assert_eq!(releases, ["release:child", "release:parent"]);
        assert_eq!(authority.diagnostics().resources.total, 0);
    }

    #[tokio::test]
    async fn committed_operation_cannot_depend_on_another_operations_staged_parent() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 1);
        let lease = authority
            .admit(SessionId::from("cross-operation-resource-dependency"))
            .expect("admission");
        let events = Arc::new(StdMutex::new(Vec::new()));
        let parent = test_resource("staged-parent", Arc::clone(&events), []);
        let (parent_tx, parent_rx) = oneshot::channel();
        let (rollback_tx, rollback_rx) = oneshot::channel();
        let parent_operation = lease
            .spawn_owned(
                SessionOperationKind::Signaling,
                Duration::from_secs(30),
                move |mut operation| async move {
                    let attempt = operation
                        .reserve_resource(resource_spec("staged-parent", Vec::new()))
                        .expect("parent reservation");
                    let parent_id = attempt.id();
                    attempt
                        .dispatch()
                        .expect("parent dispatch")
                        .into_installation_sink()
                        .capture_at_install(parent)
                        .expect("parent capture");
                    let _ = parent_tx.send(parent_id);
                    let _ = rollback_rx.await;
                    operation.rollback(()).await.expect("parent rollback")
                },
            )
            .expect("parent operation");
        let parent_id = parent_rx.await.expect("captured parent id");

        let child_operation = lease
            .spawn_owned(
                SessionOperationKind::StateTransition,
                Duration::from_secs(10),
                move |mut operation| async move {
                    let error = match operation
                        .reserve_resource(resource_spec("forbidden-child", vec![parent_id]))
                    {
                        Ok(_) => panic!("cross-operation staged dependency must be rejected"),
                        Err(error) => error,
                    };
                    operation.commit().expect("empty commit").complete(error)
                },
            )
            .expect("child operation");
        assert_eq!(
            child_operation.await,
            Ok(ResourceRegistryError::InvalidDependencyState)
        );

        rollback_tx.send(()).expect("release staged parent");
        assert_eq!(parent_operation.await, Ok(()));
        assert_eq!(authority.diagnostics().resources.total, 0);
        assert_eq!(
            events
                .lock()
                .expect("events")
                .iter()
                .filter(|event| event.as_str() == "release:staged-parent")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn failed_dependent_release_preserves_dependency_until_explicit_retry() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 1);
        let lease = authority
            .admit(SessionId::from("resource-release-retry"))
            .expect("admission");
        let events = Arc::new(StdMutex::new(Vec::new()));
        let parent = test_resource("retry-parent", Arc::clone(&events), []);
        let child = test_resource(
            "retry-child",
            Arc::clone(&events),
            [TestReleaseOutcome::Failed, TestReleaseOutcome::Ok],
        );
        let child_attempts = Arc::clone(&child.release_attempts);
        let (captured_tx, captured_rx) = oneshot::channel();
        let operation = lease
            .spawn_owned(
                SessionOperationKind::Signaling,
                Duration::from_secs(30),
                move |mut operation| async move {
                    let parent_attempt = operation
                        .reserve_resource(resource_spec("retry-parent", Vec::new()))
                        .expect("parent reservation");
                    let parent_id = parent_attempt.id();
                    parent_attempt
                        .dispatch()
                        .expect("parent dispatch")
                        .into_installation_sink()
                        .capture_at_install(parent)
                        .expect("parent capture");
                    operation
                        .reserve_resource(resource_spec("retry-child", vec![parent_id]))
                        .expect("child reservation")
                        .dispatch()
                        .expect("child dispatch")
                        .into_installation_sink()
                        .capture_at_install(child)
                        .expect("child capture");
                    let _ = captured_tx.send(());
                    operation.rollback(()).await.expect("retry rollback")
                },
            )
            .expect("owned operation");
        captured_rx.await.expect("resources captured");
        while child_attempts.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        let diagnostics = authority.diagnostics();
        assert_eq!(diagnostics.resources.total, 2);
        assert_eq!(diagnostics.resources.orphaned, 1);
        assert_eq!(diagnostics.resources.captured, 1);
        let releases_before_retry: Vec<_> = events
            .lock()
            .expect("events")
            .iter()
            .filter(|event| event.starts_with("release:"))
            .cloned()
            .collect();
        assert_eq!(releases_before_retry, ["release:retry-child"]);

        assert_eq!(
            authority
                .retry_orphaned_resources(lease.key())
                .expect("authorize retry"),
            1
        );
        assert_eq!(operation.await, Ok(()));
        let releases: Vec<_> = events
            .lock()
            .expect("events")
            .iter()
            .filter(|event| event.starts_with("release:"))
            .cloned()
            .collect();
        assert_eq!(
            releases,
            [
                "release:retry-child",
                "release:retry-child",
                "release:retry-parent"
            ]
        );
        assert_eq!(authority.diagnostics().resources.total, 0);
    }

    #[tokio::test]
    async fn panicked_and_timed_out_releases_remain_charged_until_retry_succeeds() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 1);
        let lease = authority
            .admit(SessionId::from("resource-panic-timeout-retry"))
            .expect("admission");
        let events = Arc::new(StdMutex::new(Vec::new()));
        let panicked = test_resource(
            "panicked-release",
            Arc::clone(&events),
            [TestReleaseOutcome::Panicked, TestReleaseOutcome::Ok],
        );
        let pending = test_resource(
            "timed-out-release",
            Arc::clone(&events),
            [TestReleaseOutcome::Pending, TestReleaseOutcome::Ok],
        );
        let panic_attempts = Arc::clone(&panicked.release_attempts);
        let timeout_attempts = Arc::clone(&pending.release_attempts);
        let (captured_tx, captured_rx) = oneshot::channel();
        let operation = lease
            .spawn_owned(
                SessionOperationKind::Signaling,
                Duration::from_secs(30),
                move |mut operation| async move {
                    operation
                        .reserve_resource(resource_spec("panicked-release", Vec::new()))
                        .expect("panic reservation")
                        .dispatch()
                        .expect("panic dispatch")
                        .into_installation_sink()
                        .capture_at_install(panicked)
                        .expect("panic capture");
                    operation
                        .reserve_resource(resource_spec("timed-out-release", Vec::new()))
                        .expect("timeout reservation")
                        .dispatch()
                        .expect("timeout dispatch")
                        .into_installation_sink()
                        .capture_at_install(pending)
                        .expect("timeout capture");
                    let _ = captured_tx.send(());
                    operation.rollback(()).await.expect("retry rollback")
                },
            )
            .expect("owned operation");
        captured_rx.await.expect("resources captured");
        while panic_attempts.load(Ordering::SeqCst) == 0
            || timeout_attempts.load(Ordering::SeqCst) == 0
        {
            tokio::task::yield_now().await;
        }
        clock.advance(Duration::from_secs(5));
        loop {
            let diagnostics = authority.diagnostics();
            if diagnostics.resources.orphaned == 2 {
                assert_eq!(diagnostics.resources.total, 2);
                assert_eq!(diagnostics.resources.orphan_reasons.release_panicked, 1);
                assert_eq!(diagnostics.resources.orphan_reasons.release_deadline, 1);
                break;
            }
            tokio::task::yield_now().await;
        }

        assert_eq!(
            authority
                .retry_orphaned_resources(lease.key())
                .expect("authorize both retries"),
            2
        );
        assert_eq!(operation.await, Ok(()));
        assert_eq!(panic_attempts.load(Ordering::SeqCst), 2);
        assert_eq!(timeout_attempts.load(Ordering::SeqCst), 2);
        assert_eq!(authority.diagnostics().resources.total, 0);
    }

    #[tokio::test]
    async fn ambiguous_install_retains_capacity_and_non_expiring_cleanup_drivers() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 1);
        let lease = authority
            .admit(SessionId::from("ambiguous-resource-install"))
            .expect("admission");
        let (orphaned_tx, orphaned_rx) = oneshot::channel();
        let operation = lease
            .spawn_owned(
                SessionOperationKind::Signaling,
                Duration::from_secs(30),
                move |mut operation| async move {
                    let sink = operation
                        .reserve_resource(resource_spec("ambiguous", Vec::new()))
                        .expect("reservation")
                        .dispatch()
                        .expect("dispatch")
                        .into_installation_sink();
                    drop(sink);
                    let _ = orphaned_tx.send(());
                    operation.rollback(()).await.expect("cannot guess cleanup")
                },
            )
            .expect("owned operation");
        orphaned_rx.await.expect("install became ambiguous");
        let teardown = authority
            .teardown(lease.key(), Duration::from_secs(1))
            .expect("teardown");
        assert!(matches!(
            teardown.clone().wait().await.expect("orphan reported"),
            TeardownOutcome::Quarantined {
                reason: QuarantineReason::ResourceInstallOrphaned,
                ..
            }
        ));
        tokio::task::yield_now().await;
        let diagnostics = authority.diagnostics();
        assert_eq!(diagnostics.resources.total, 1);
        assert_eq!(diagnostics.resources.orphaned, 1);
        assert_eq!(
            diagnostics.resources.orphan_reasons.dispatch_permit_dropped,
            1
        );
        assert_eq!(diagnostics.owned_operation_supervisors, 1);
        assert_eq!(diagnostics.teardown_supervisors, 1);
        let mut operation = Box::pin(operation);
        assert!(futures::poll!(&mut operation).is_pending());
        let mut driver = Box::pin(teardown.clone().wait_supervisor());
        assert!(futures::poll!(&mut driver).is_pending());

        assert_eq!(
            authority
                .abort_supervisors_for_key(lease.key(), SupervisorKind::OwnedOperation)
                .expect("abort operation"),
            1
        );
        assert_eq!(operation.await, Err(OwnedOperationError::SupervisorDropped));
        tokio::task::yield_now().await;
        assert_eq!(
            authority
                .abort_supervisors_for_key(lease.key(), SupervisorKind::Teardown)
                .expect("abort teardown"),
            1
        );
        driver.await.expect("teardown drop bomb completed");
        authority
            .wait_for_supervisors()
            .await
            .expect("all test supervisors removed");
        assert_eq!(authority.diagnostics().resources.total, 1);
    }

    #[tokio::test]
    async fn notify_enable_before_check_prevents_teardown_lost_wakeups() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 32);
        for sequence in 0..32 {
            let lease = authority
                .admit(SessionId::from(format!("lost-wakeup-{sequence}")))
                .expect("admission");
            let guard = lease
                .try_operation(SessionOperationKind::Test("lost-wakeup"))
                .expect("operation");
            let teardown = authority
                .teardown(lease.key(), Duration::from_secs(1))
                .expect("teardown");
            // Complete before the newly spawned driver necessarily polls.
            guard.finish_rollback();
            assert!(matches!(
                teardown.wait().await.expect("retirement"),
                TeardownOutcome::Retired { .. }
            ));
        }
        authority
            .wait_for_supervisors()
            .await
            .expect("all teardown supervisors");
    }

    #[tokio::test]
    async fn diagnostics_report_operation_metadata_reasons_and_oldest_ages() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 4);

        let quarantined = authority
            .admit(SessionId::from("diagnostic-quarantine"))
            .expect("quarantine admission");
        drop(
            quarantined
                .try_operation(SessionOperationKind::Test("abandon"))
                .expect("abandoned operation"),
        );
        let retired = authority
            .admit(SessionId::from("diagnostic-retired"))
            .expect("retired admission");
        authority.retire(retired.key()).expect("retire");

        let active = authority
            .admit(SessionId::from("diagnostic-active"))
            .expect("active admission");
        let signaling = active
            .try_operation(SessionOperationKind::Signaling)
            .expect("signaling operation");
        let (release_tx, release_rx) = oneshot::channel::<()>();
        let media = active
            .spawn_owned(
                SessionOperationKind::Media,
                Duration::from_secs(30),
                move |operation| async move {
                    let _ = release_rx.await;
                    operation.rollback(()).await.expect("exact rollback")
                },
            )
            .expect("media operation");

        clock.advance(Duration::from_secs(7));
        let diagnostics = authority.diagnostics();
        assert!(diagnostics.complete);
        assert_eq!(diagnostics.capacity, 4);
        assert_eq!(diagnostics.lifecycle_count, 3);
        assert_eq!(diagnostics.quarantined, 1);
        assert_eq!(diagnostics.retired, 1);
        assert_eq!(
            diagnostics.oldest_quarantine_age,
            Some(Duration::from_secs(7))
        );
        assert_eq!(diagnostics.oldest_retired_age, Some(Duration::from_secs(7)));
        assert_eq!(diagnostics.operations.total, 2);
        assert_eq!(diagnostics.operations.with_hard_deadline, 1);
        assert_eq!(diagnostics.operations.signaling, 1);
        assert_eq!(diagnostics.operations.media, 1);
        assert_eq!(diagnostics.owned_operation_supervisors, 1);
        assert_eq!(diagnostics.quarantine_reasons.operation_abandoned, 1);

        signaling.finish_rollback();
        release_tx.send(()).expect("release media");
        assert_eq!(media.await, Ok(()));
        authority
            .wait_for_supervisors()
            .await
            .expect("media supervisor cleanup");
    }

    #[tokio::test]
    async fn nested_drain_excludes_current_supervisor_without_self_join() {
        let clock = ManualClock::new();
        let authority = authority(&clock, 1);
        let lease = authority
            .admit(SessionId::from("nested-drain"))
            .expect("admission");
        let deadline = clock
            .now()
            .checked_add(Duration::from_secs(10))
            .expect("deadline");
        let nested_authority = Arc::clone(&authority);
        let (report_tx, report_rx) = oneshot::channel();
        let operation = lease
            .spawn_owned(
                SessionOperationKind::Test("nested-drain"),
                Duration::from_secs(30),
                move |operation| async move {
                    let report = nested_authority.drain_supervisors(deadline).await;
                    let _ = report_tx.send(report);
                    operation.rollback(()).await.expect("exact rollback")
                },
            )
            .expect("operation");

        let report = report_rx.await.expect("nested drain report");
        assert!(report.excluded_current);
        assert!(!report.deadline_reached);
        assert_eq!(report.abort_requested, 0);
        assert_eq!(report.stragglers, 0);
        assert_eq!(operation.await, Ok(()));
        authority
            .wait_for_supervisors()
            .await
            .expect("current supervisor eventually unregisters");
        assert_eq!(
            authority.admit(SessionId::from("after-drain")).err(),
            Some(SessionAdmissionError::AuthorityDraining)
        );
    }

    #[tokio::test]
    async fn deadline_drain_aborts_and_joins_a_retained_operation() {
        let clock = ManualClock::new();
        let authority = authority_with_grace(&clock, 1, Duration::from_secs(1));
        let lease = authority
            .admit(SessionId::from("deadline-drain"))
            .expect("admission");
        let (_hold_tx, hold_rx) = oneshot::channel::<()>();
        let (started_tx, started_rx) = oneshot::channel::<()>();
        let operation = lease
            .spawn_owned(
                SessionOperationKind::Test("drain-abort"),
                Duration::from_secs(30),
                move |operation| async move {
                    let _ = started_tx.send(());
                    let _ = hold_rx.await;
                    operation.rollback(()).await.expect("exact rollback")
                },
            )
            .expect("operation");
        started_rx.await.expect("operation started");
        let deadline = clock
            .now()
            .checked_add(Duration::from_secs(1))
            .expect("deadline");
        let mut drain = Box::pin(authority.drain_supervisors(deadline));
        assert!(futures::poll!(&mut drain).is_pending());
        clock.advance(Duration::from_secs(1));
        let report = drain.await;

        assert!(report.deadline_reached);
        assert_eq!(report.abort_requested, 1);
        assert_eq!(report.stragglers, 0);
        assert_eq!(report.fatal_reason, None);
        assert_eq!(operation.await, Err(OwnedOperationError::SupervisorDropped));
        assert!(matches!(
            authority.phase(lease.key()),
            Some(SessionPhase::Quarantined { .. })
        ));
    }

    #[tokio::test]
    async fn late_abort_handle_observes_prior_drain_request() {
        let clock = ManualClock::new();
        let authority = authority_with_grace(&clock, 1, Duration::from_secs(1));
        let lease = authority
            .admit(SessionId::from("late-abort-handle"))
            .expect("admission");
        let registration = authority
            .register_supervisor(lease.key(), SupervisorKind::OwnedOperation)
            .expect("registration");
        let supervisor_id = registration.id.expect("supervisor id");
        let task = tokio::spawn(async move {
            let _registration = registration;
            futures::future::pending::<()>().await;
        });
        let abort = task.abort_handle();
        let mut drain = Box::pin(authority.drain_supervisors(clock.now()));
        assert!(futures::poll!(&mut drain).is_pending());

        authority
            .set_supervisor_abort(supervisor_id, abort)
            .expect("late handle installation");
        let report = drain.await;
        assert_eq!(report.abort_requested, 1);
        assert_eq!(report.stragglers, 0);
        assert!(task.await.expect_err("task aborted").is_cancelled());
    }

    #[tokio::test]
    async fn drain_reports_noncooperative_registration_after_bounded_grace() {
        let clock = ManualClock::new();
        let authority = authority_with_grace(&clock, 1, Duration::from_secs(1));
        let lease = authority
            .admit(SessionId::from("drain-straggler"))
            .expect("admission");
        let registration = authority
            .register_supervisor(lease.key(), SupervisorKind::OwnedOperation)
            .expect("registration without task handle");
        let mut drain = Box::pin(authority.drain_supervisors(clock.now()));
        assert!(futures::poll!(&mut drain).is_pending());
        clock.advance(Duration::from_secs(1));

        let report = drain.await;
        assert!(report.deadline_reached);
        assert_eq!(report.abort_requested, 1);
        assert_eq!(report.stragglers, 1);
        drop(registration);
        authority
            .wait_for_supervisors()
            .await
            .expect("manual registration removed");
    }

    #[tokio::test]
    async fn fatal_drain_still_aborts_through_cleanup_registry() {
        let clock = ManualClock::new();
        let authority = authority_with_grace(&clock, 1, Duration::from_secs(1));
        let lease = authority
            .admit(SessionId::from("fatal-drain"))
            .expect("admission");
        let (_hold_tx, hold_rx) = oneshot::channel::<()>();
        let (started_tx, started_rx) = oneshot::channel::<()>();
        let operation = lease
            .spawn_owned(
                SessionOperationKind::Test("fatal-drain"),
                Duration::from_secs(30),
                move |operation| async move {
                    let _ = started_tx.send(());
                    let _ = hold_rx.await;
                    operation.rollback(()).await.expect("exact rollback")
                },
            )
            .expect("operation");
        started_rx.await.expect("operation started");
        let _ = std::panic::catch_unwind(AssertUnwindSafe({
            let authority = Arc::clone(&authority);
            move || {
                let _index = authority.index.lock().expect("index lock");
                panic!("poison before drain");
            }
        }));
        assert!(matches!(
            authority.admit(SessionId::from("detect-fatal")).err(),
            Some(SessionAdmissionError::AuthorityFatal(
                AuthorityFatalReason::IndexPoisoned
            ))
        ));
        assert_eq!(
            authority.wait_for_supervisors().await,
            Err(SessionOperationError::AuthorityFatal(
                AuthorityFatalReason::IndexPoisoned
            ))
        );

        let report = authority.drain_supervisors(clock.now()).await;
        assert_eq!(
            report.fatal_reason,
            Some(AuthorityFatalReason::IndexPoisoned)
        );
        assert_eq!(report.abort_requested, 1);
        assert_eq!(report.stragglers, 0);
        assert_eq!(operation.await, Err(OwnedOperationError::SupervisorDropped));
    }
}
