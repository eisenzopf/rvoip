// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Bounded, transport-owned retention for completed MOQT objects.
//!
//! Objects are first committed to a pending group. A pending group counts
//! against every configured store bound, but is invisible to snapshots until
//! [`RetainedTrack::complete_group`] atomically publishes the whole group.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use bytes::{Bytes, BytesMut};
use thiserror::Error;

use crate::coding::{Encode, Location};
use crate::data::ExtensionHeaders;
use crate::message::GroupOrder;
use crate::watch::State;

/// Aggregate byte limits shared by all retained tracks in a session and in
/// the process. Bytes are charged lazily when Objects are committed, not when
/// a subscription allocates an empty cache.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetentionBudgetLimits {
    pub max_session_bytes: usize,
    pub max_process_bytes: usize,
}

impl Default for RetentionBudgetLimits {
    fn default() -> Self {
        Self {
            max_session_bytes: 64 * 1024 * 1024,
            max_process_bytes: 512 * 1024 * 1024,
        }
    }
}

#[derive(Debug)]
struct RetentionBudgetState {
    used: usize,
    limit: usize,
}

/// Process-owned retention budget. Derive one session view per MOQT session.
#[derive(Clone, Debug)]
pub struct RetentionBudgetPool {
    process: Arc<Mutex<RetentionBudgetState>>,
    limits: RetentionBudgetLimits,
}

impl RetentionBudgetPool {
    pub fn new(limits: RetentionBudgetLimits) -> Result<Self, RetentionBudgetError> {
        if limits.max_session_bytes == 0
            || limits.max_process_bytes == 0
            || limits.max_session_bytes > limits.max_process_bytes
        {
            return Err(RetentionBudgetError::InvalidLimits);
        }
        Ok(Self {
            process: Arc::new(Mutex::new(RetentionBudgetState {
                used: 0,
                limit: limits.max_process_bytes,
            })),
            limits,
        })
    }

    pub fn session(&self) -> RetentionBudget {
        RetentionBudget {
            process: self.process.clone(),
            session: Arc::new(Mutex::new(RetentionBudgetState {
                used: 0,
                limit: self.limits.max_session_bytes,
            })),
            limits: self.limits,
        }
    }

    pub fn limits(&self) -> RetentionBudgetLimits {
        self.limits
    }

    /// Return a point-in-time process-wide retained-byte gauge.
    ///
    /// Clones of this pool share the same process counter, so a long-lived
    /// runtime can retain one clone and poll this snapshot after individual
    /// sessions have been moved into their serving tasks.
    pub fn stats(&self) -> RetentionBudgetPoolStats {
        RetentionBudgetPoolStats {
            process_bytes: lock(&self.process).used,
            max_process_bytes: self.limits.max_process_bytes,
        }
    }
}

impl Default for RetentionBudgetPool {
    fn default() -> Self {
        Self::new(RetentionBudgetLimits::default())
            .expect("default retention budget limits are valid")
    }
}

/// One session's view over both the session-local and process-wide byte
/// counters. Clones share the same counters.
#[derive(Clone, Debug)]
pub struct RetentionBudget {
    process: Arc<Mutex<RetentionBudgetState>>,
    session: Arc<Mutex<RetentionBudgetState>>,
    limits: RetentionBudgetLimits,
}

impl RetentionBudget {
    fn try_reserve(
        &self,
        bytes: usize,
    ) -> Result<Arc<RetentionBudgetReservation>, RetentionBudgetError> {
        // Every acquisition and release uses process -> session lock order.
        let mut process = lock(&self.process);
        let mut session = lock(&self.session);
        let process_total = process
            .used
            .checked_add(bytes)
            .ok_or(RetentionBudgetError::ProcessBytes)?;
        if process_total > process.limit {
            return Err(RetentionBudgetError::ProcessBytes);
        }
        let session_total = session
            .used
            .checked_add(bytes)
            .ok_or(RetentionBudgetError::SessionBytes)?;
        if session_total > session.limit {
            return Err(RetentionBudgetError::SessionBytes);
        }

        // All checks completed before either counter is mutated, so a failed
        // process or session admission cannot strand partial capacity.
        process.used = process_total;
        session.used = session_total;
        Ok(Arc::new(RetentionBudgetReservation {
            process: self.process.clone(),
            session: self.session.clone(),
            bytes,
        }))
    }

    pub fn stats(&self) -> RetentionBudgetStats {
        let process_bytes = lock(&self.process).used;
        let session_bytes = lock(&self.session).used;
        RetentionBudgetStats {
            session_bytes,
            process_bytes,
            limits: self.limits,
        }
    }

    pub fn limits(&self) -> RetentionBudgetLimits {
        self.limits
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetentionBudgetStats {
    pub session_bytes: usize,
    pub process_bytes: usize,
    pub limits: RetentionBudgetLimits,
}

/// Process-wide retained-byte gauge exposed by [`RetentionBudgetPool`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetentionBudgetPoolStats {
    pub process_bytes: usize,
    pub max_process_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum RetentionBudgetError {
    #[error("retention budget limits are invalid")]
    InvalidLimits,
    #[error("session retained-byte budget exhausted")]
    SessionBytes,
    #[error("process retained-byte budget exhausted")]
    ProcessBytes,
}

#[derive(Debug)]
struct RetentionBudgetReservation {
    process: Arc<Mutex<RetentionBudgetState>>,
    session: Arc<Mutex<RetentionBudgetState>>,
    bytes: usize,
}

impl Drop for RetentionBudgetReservation {
    fn drop(&mut self) {
        let mut process = lock(&self.process);
        let mut session = lock(&self.session);
        debug_assert!(process.used >= self.bytes);
        debug_assert!(session.used >= self.bytes);
        process.used = process.used.saturating_sub(self.bytes);
        session.used = session.used.saturating_sub(self.bytes);
    }
}

/// Hard bounds for retained objects, groups, and active snapshots.
///
/// Byte limits include both payload bytes and the encoded extension-header
/// bytes retained with each object.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetentionLimits {
    pub max_object_bytes: usize,
    pub max_group_bytes: usize,
    pub max_objects_per_group: usize,
    pub max_groups: usize,
    pub max_store_bytes: usize,
    pub max_store_objects: usize,
    pub max_active_snapshots: usize,
}

impl Default for RetentionLimits {
    fn default() -> Self {
        Self {
            max_object_bytes: 256 * 1024,
            max_group_bytes: 4 * 1024 * 1024,
            max_objects_per_group: 1024,
            max_groups: 2,
            // Two completed groups plus one full pending group leaves enough
            // headroom for an atomic rotation at every per-group maximum.
            max_store_bytes: 12 * 1024 * 1024,
            max_store_objects: 3072,
            max_active_snapshots: 32,
        }
    }
}

impl RetentionLimits {
    pub fn validate(self) -> Result<Self, RetentionError> {
        let nonzero = self.max_object_bytes > 0
            && self.max_group_bytes > 0
            && self.max_objects_per_group > 0
            && self.max_groups > 0
            && self.max_store_bytes > 0
            && self.max_store_objects > 0
            && self.max_active_snapshots > 0;
        let consistent = self.max_object_bytes <= self.max_group_bytes
            && self.max_group_bytes <= self.max_store_bytes
            && self.max_objects_per_group <= self.max_store_objects;

        if nonzero && consistent {
            Ok(self)
        } else {
            Err(RetentionError::InvalidRange(
                RetentionRangeError::InvalidLimits,
            ))
        }
    }
}

/// The bound that rejected an otherwise valid retention operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetentionLimit {
    ObjectBytes,
    GroupBytes,
    GroupObjects,
    StoreBytes,
    StoreObjects,
    ActiveSnapshots,
    PinnedGroups,
    SessionBytes,
    ProcessBytes,
}

/// A deterministic validation error for a retention or range operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetentionRangeError {
    InvalidLimits,
    IncompleteObject,
    InvalidObjectProperties,
    EmptyOrReversed,
    NoObjects,
    StartAfterLargest,
    NotRetained,
    UnsupportedGroupOrder,
    NonIncreasingGroup,
    PendingGroupMismatch,
    NoPendingGroup,
    ConflictingObject,
}

/// Errors returned by the bounded retention layer.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum RetentionError {
    #[error("retention limit exceeded: {0:?}")]
    ExcessiveLoad(RetentionLimit),
    #[error("invalid retention range or operation: {0:?}")]
    InvalidRange(RetentionRangeError),
}

/// Immutable metadata retained with an object.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetainedObjectMetadata {
    pub location: Location,
    /// `None` identifies a datagram object; `Some` identifies its subgroup.
    pub subgroup_id: Option<u64>,
    pub publisher_priority: u8,
    pub properties: ExtensionHeaders,
    /// Whether the original live subgroup header asserted END_OF_GROUP.
    /// Standard draft-19 FETCH cannot serialize this field.
    pub end_of_group: bool,
}

/// A complete immutable object whose payload remains reference-counted.
#[derive(Clone, Debug)]
pub struct RetainedObject {
    metadata: RetainedObjectMetadata,
    payload: Bytes,
    retained_bytes: usize,
    budget: Option<Arc<RetentionBudgetReservation>>,
}

impl PartialEq for RetainedObject {
    fn eq(&self, other: &Self) -> bool {
        self.metadata == other.metadata
            && self.payload == other.payload
            && self.retained_bytes == other.retained_bytes
    }
}

impl Eq for RetainedObject {}

impl RetainedObject {
    pub fn new(metadata: RetainedObjectMetadata, payload: Bytes) -> Result<Self, RetentionError> {
        let mut encoded_properties = BytesMut::new();
        metadata
            .properties
            .encode(&mut encoded_properties)
            .map_err(|_| {
                RetentionError::InvalidRange(RetentionRangeError::InvalidObjectProperties)
            })?;
        let retained_bytes = payload
            .len()
            .checked_add(encoded_properties.len())
            .ok_or(RetentionError::ExcessiveLoad(RetentionLimit::ObjectBytes))?;

        Ok(Self {
            metadata,
            payload,
            retained_bytes,
            budget: None,
        })
    }

    pub fn metadata(&self) -> &RetainedObjectMetadata {
        &self.metadata
    }

    pub fn location(&self) -> Location {
        self.metadata.location
    }

    pub fn group_id(&self) -> u64 {
        self.metadata.location.group_id
    }

    pub fn object_id(&self) -> u64 {
        self.metadata.location.object_id
    }

    pub fn subgroup_id(&self) -> Option<u64> {
        self.metadata.subgroup_id
    }

    pub fn publisher_priority(&self) -> u8 {
        self.metadata.publisher_priority
    }

    pub fn properties(&self) -> &ExtensionHeaders {
        &self.metadata.properties
    }

    pub fn end_of_group(&self) -> bool {
        self.metadata.end_of_group
    }

    pub fn payload(&self) -> &Bytes {
        &self.payload
    }

    pub fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }
}

/// Builder that keeps an incomplete object outside the retained track.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetainedObjectBuilder {
    metadata: RetainedObjectMetadata,
    expected_payload_length: usize,
}

impl RetainedObjectBuilder {
    pub fn new(metadata: RetainedObjectMetadata, expected_payload_length: usize) -> Self {
        Self {
            metadata,
            expected_payload_length,
        }
    }

    pub fn finish(self, payload: Bytes) -> Result<RetainedObject, RetentionError> {
        if payload.len() != self.expected_payload_length {
            return Err(RetentionError::InvalidRange(
                RetentionRangeError::IncompleteObject,
            ));
        }

        RetainedObject::new(self.metadata, payload)
    }
}

#[derive(Debug)]
struct PendingGroup {
    group_id: u64,
    objects: BTreeMap<u64, RetainedObject>,
    bytes: usize,
}

#[derive(Debug)]
struct RetainedGroup {
    objects: Vec<RetainedObject>,
    bytes: usize,
}

#[derive(Debug)]
struct TrackState {
    limits: RetentionLimits,
    groups: BTreeMap<u64, Arc<RetainedGroup>>,
    pending: Option<PendingGroup>,
    completed_objects: usize,
    completed_bytes: usize,
    active_snapshots: usize,
}

/// A bounded retention store for one transport track.
#[derive(Clone, Debug)]
pub struct RetainedTrack {
    state: Arc<Mutex<TrackState>>,
    revision: State<u64>,
    budget: RetentionBudget,
}

impl RetainedTrack {
    pub fn new(limits: RetentionLimits) -> Result<Self, RetentionError> {
        let budget = RetentionBudgetPool::new(RetentionBudgetLimits {
            max_session_bytes: limits.max_store_bytes,
            max_process_bytes: limits.max_store_bytes,
        })
        .map_err(|_| RetentionError::InvalidRange(RetentionRangeError::InvalidLimits))?
        .session();
        Self::new_with_budget(limits, budget)
    }

    pub fn new_with_budget(
        limits: RetentionLimits,
        budget: RetentionBudget,
    ) -> Result<Self, RetentionError> {
        let limits = limits.validate()?;
        Ok(Self {
            state: Arc::new(Mutex::new(TrackState {
                limits,
                groups: BTreeMap::new(),
                pending: None,
                completed_objects: 0,
                completed_bytes: 0,
                active_snapshots: 0,
            })),
            revision: State::default(),
            budget,
        })
    }

    /// Commit one complete object to the pending group.
    ///
    /// Objects may arrive out of object-ID order, but only one group may be
    /// pending. An exact duplicate is idempotent.
    pub fn commit(&self, mut object: RetainedObject) -> Result<(), RetentionError> {
        let mut state = lock(&self.state);
        let limits = state.limits;

        if object.retained_bytes() > limits.max_object_bytes {
            return Err(RetentionError::ExcessiveLoad(RetentionLimit::ObjectBytes));
        }

        let group_id = object.group_id();
        if let Some(pending) = &state.pending {
            if pending.group_id != group_id {
                return Err(RetentionError::InvalidRange(
                    RetentionRangeError::PendingGroupMismatch,
                ));
            }
            if let Some(existing) = pending.objects.get(&object.object_id()) {
                return if existing == &object {
                    Ok(())
                } else {
                    Err(RetentionError::InvalidRange(
                        RetentionRangeError::ConflictingObject,
                    ))
                };
            }
        } else if state
            .groups
            .last_key_value()
            .is_some_and(|(largest, _)| group_id <= *largest)
        {
            return Err(RetentionError::InvalidRange(
                RetentionRangeError::NonIncreasingGroup,
            ));
        }

        let pending_objects = state
            .pending
            .as_ref()
            .map_or(0, |group| group.objects.len());
        let pending_bytes = state.pending.as_ref().map_or(0, |group| group.bytes);

        if pending_objects >= limits.max_objects_per_group {
            return Err(RetentionError::ExcessiveLoad(RetentionLimit::GroupObjects));
        }
        checked_bound(
            pending_bytes,
            object.retained_bytes(),
            limits.max_group_bytes,
            RetentionLimit::GroupBytes,
        )?;
        checked_bound(
            state.completed_objects,
            pending_objects.saturating_add(1),
            limits.max_store_objects,
            RetentionLimit::StoreObjects,
        )?;
        checked_bound(
            state.completed_bytes,
            pending_bytes
                .checked_add(object.retained_bytes())
                .ok_or(RetentionError::ExcessiveLoad(RetentionLimit::StoreBytes))?,
            limits.max_store_bytes,
            RetentionLimit::StoreBytes,
        )?;

        let reservation = self
            .budget
            .try_reserve(object.retained_bytes())
            .map_err(|error| match error {
                RetentionBudgetError::SessionBytes => {
                    RetentionError::ExcessiveLoad(RetentionLimit::SessionBytes)
                }
                RetentionBudgetError::ProcessBytes => {
                    RetentionError::ExcessiveLoad(RetentionLimit::ProcessBytes)
                }
                RetentionBudgetError::InvalidLimits => {
                    RetentionError::InvalidRange(RetentionRangeError::InvalidLimits)
                }
            })?;
        object.budget = Some(reservation);

        let pending = state.pending.get_or_insert_with(|| PendingGroup {
            group_id,
            objects: BTreeMap::new(),
            bytes: 0,
        });
        pending.bytes += object.retained_bytes();
        pending.objects.insert(object.object_id(), object);
        Ok(())
    }

    /// Atomically promote the pending group and rotate unpinned old groups.
    pub fn complete_group(&self, group_id: u64) -> Result<(), RetentionError> {
        let mut state = lock(&self.state);
        let pending = state.pending.as_ref().ok_or(RetentionError::InvalidRange(
            RetentionRangeError::NoPendingGroup,
        ))?;
        if pending.group_id != group_id {
            return Err(RetentionError::InvalidRange(
                RetentionRangeError::PendingGroupMismatch,
            ));
        }

        let required_evictions = state
            .groups
            .len()
            .saturating_add(1)
            .saturating_sub(state.limits.max_groups);
        let evictions: Vec<u64> = state
            .groups
            .iter()
            .filter_map(|(id, group)| (Arc::strong_count(group) == 1).then_some(*id))
            .take(required_evictions)
            .collect();
        if evictions.len() != required_evictions {
            return Err(RetentionError::ExcessiveLoad(RetentionLimit::PinnedGroups));
        }

        // All fallible checks happen above. Mutation below is one critical
        // section, so readers observe either the old or new completed set.
        let pending = state.pending.take().expect("pending group checked above");
        for id in evictions {
            let removed = state.groups.remove(&id).expect("planned group exists");
            state.completed_objects -= removed.objects.len();
            state.completed_bytes -= removed.bytes;
        }

        let group = Arc::new(RetainedGroup {
            objects: pending.objects.into_values().collect(),
            bytes: pending.bytes,
        });
        state.completed_objects += group.objects.len();
        state.completed_bytes += group.bytes;
        state.groups.insert(group_id, group);
        drop(state);
        if let Some(mut revision) = self.revision.lock_mut() {
            *revision = revision.saturating_add(1);
        }
        Ok(())
    }

    /// Discard an incomplete Group after the live path determined it cannot
    /// be represented by the bounded FETCH profile. Completed snapshots are
    /// unaffected.
    pub(crate) fn discard_pending(&self, group_id: u64) {
        let mut state = lock(&self.state);
        if state
            .pending
            .as_ref()
            .is_some_and(|pending| pending.group_id == group_id)
        {
            state.pending = None;
        }
    }

    /// Wait until the requested immutable range is retained.
    ///
    /// Only transient "not published yet" range errors wait for another
    /// completed-group revision. Structural range and capacity errors remain
    /// immediate so a malformed FETCH cannot retain an unbounded waiter.
    pub async fn snapshot_when_available(
        &self,
        range: RetainedRange,
        group_order: GroupOrder,
    ) -> Result<RetainedSnapshot, RetentionError> {
        loop {
            let revision = self.revision.lock();
            match self.snapshot(range, group_order) {
                Ok(snapshot) => return Ok(snapshot),
                Err(RetentionError::InvalidRange(
                    RetentionRangeError::NoObjects
                    | RetentionRangeError::StartAfterLargest
                    | RetentionRangeError::NotRetained,
                )) => {}
                Err(error) => return Err(error),
            }
            let modified = revision.modified().ok_or(RetentionError::InvalidRange(
                RetentionRangeError::NotRetained,
            ))?;
            modified.await;
        }
    }

    /// Create an immutable range snapshot and pin every selected group.
    pub fn snapshot(
        &self,
        range: RetainedRange,
        group_order: GroupOrder,
    ) -> Result<RetainedSnapshot, RetentionError> {
        if group_order == GroupOrder::Publisher {
            return Err(RetentionError::InvalidRange(
                RetentionRangeError::UnsupportedGroupOrder,
            ));
        }
        if range.start >= range.end {
            return Err(RetentionError::InvalidRange(
                RetentionRangeError::EmptyOrReversed,
            ));
        }

        let mut state = lock(&self.state);
        let largest = largest_location(&state)
            .ok_or(RetentionError::InvalidRange(RetentionRangeError::NoObjects))?;
        if range.start > largest {
            return Err(RetentionError::InvalidRange(
                RetentionRangeError::StartAfterLargest,
            ));
        }

        let last_group = if range.end.object_id == 0 {
            range
                .end
                .group_id
                .checked_sub(1)
                .ok_or(RetentionError::InvalidRange(
                    RetentionRangeError::NotRetained,
                ))?
        } else {
            range.end.group_id
        };
        if last_group < range.start.group_id {
            return Err(RetentionError::InvalidRange(
                RetentionRangeError::EmptyOrReversed,
            ));
        }

        let expected_u64 = last_group
            .checked_sub(range.start.group_id)
            .and_then(|difference| difference.checked_add(1))
            .ok_or(RetentionError::InvalidRange(
                RetentionRangeError::NotRetained,
            ))?;
        let expected = usize::try_from(expected_u64)
            .map_err(|_| RetentionError::InvalidRange(RetentionRangeError::NotRetained))?;

        let selected: Vec<(u64, Arc<RetainedGroup>)> = state
            .groups
            .range(range.start.group_id..=last_group)
            .map(|(id, group)| (*id, Arc::clone(group)))
            .collect();
        let contiguous = selected.len() == expected
            && selected.iter().enumerate().all(|(offset, (id, _))| {
                u64::try_from(offset)
                    .ok()
                    .and_then(|offset| range.start.group_id.checked_add(offset))
                    == Some(*id)
            });
        if !contiguous {
            return Err(RetentionError::InvalidRange(
                RetentionRangeError::NotRetained,
            ));
        }
        if state.active_snapshots >= state.limits.max_active_snapshots {
            return Err(RetentionError::ExcessiveLoad(
                RetentionLimit::ActiveSnapshots,
            ));
        }

        let mut groups: Vec<Arc<RetainedGroup>> =
            selected.into_iter().map(|(_, group)| group).collect();
        if group_order == GroupOrder::Descending {
            groups.reverse();
        }
        let (objects, retained_bytes) = groups
            .iter()
            .flat_map(|group| group.objects.iter())
            .filter(|object| range.contains(object.location()))
            .fold((0usize, 0usize), |(objects, bytes), object| {
                (objects + 1, bytes + object.retained_bytes())
            });

        state.active_snapshots += 1;
        Ok(RetainedSnapshot {
            owner: Arc::downgrade(&self.state),
            range,
            group_order,
            groups,
            objects,
            retained_bytes,
        })
    }

    pub fn stats(&self) -> RetainedTrackStats {
        let state = lock(&self.state);
        let pending_objects = state
            .pending
            .as_ref()
            .map_or(0, |group| group.objects.len());
        let pending_bytes = state.pending.as_ref().map_or(0, |group| group.bytes);
        RetainedTrackStats {
            completed_groups: state.groups.len(),
            completed_objects: state.completed_objects,
            completed_bytes: state.completed_bytes,
            pending_group: state.pending.as_ref().map(|group| group.group_id),
            pending_objects,
            pending_bytes,
            store_objects: state.completed_objects + pending_objects,
            store_bytes: state.completed_bytes + pending_bytes,
            active_snapshots: state.active_snapshots,
            largest_location: largest_location(&state),
        }
    }

    pub fn limits(&self) -> RetentionLimits {
        lock(&self.state).limits
    }
}

/// Half-open object range `[start, end)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetainedRange {
    pub start: Location,
    pub end: Location,
}

impl RetainedRange {
    pub fn new(start: Location, end: Location) -> Self {
        Self { start, end }
    }

    pub fn contains(&self, location: Location) -> bool {
        self.start <= location && location < self.end
    }
}

/// Immutable snapshot of retained groups and the selected objects within them.
///
/// The snapshot pins whole groups for its lifetime. It intentionally is not
/// `Clone`; callers that need shared ownership can wrap it in an [`Arc`].
#[derive(Debug)]
pub struct RetainedSnapshot {
    owner: Weak<Mutex<TrackState>>,
    range: RetainedRange,
    group_order: GroupOrder,
    groups: Vec<Arc<RetainedGroup>>,
    objects: usize,
    retained_bytes: usize,
}

impl RetainedSnapshot {
    pub fn range(&self) -> RetainedRange {
        self.range
    }

    pub fn group_order(&self) -> GroupOrder {
        self.group_order
    }

    pub fn len(&self) -> usize {
        self.objects
    }

    pub fn is_empty(&self) -> bool {
        self.objects == 0
    }

    pub fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }

    pub fn pinned_group_count(&self) -> usize {
        self.groups.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &RetainedObject> {
        self.groups
            .iter()
            .flat_map(|group| group.objects.iter())
            .filter(|object| self.range.contains(object.location()))
    }
}

impl Drop for RetainedSnapshot {
    fn drop(&mut self) {
        if let Some(owner) = self.owner.upgrade() {
            let mut state = lock(&owner);
            debug_assert!(state.active_snapshots > 0);
            state.active_snapshots = state.active_snapshots.saturating_sub(1);
        }
    }
}

/// Aggregate-safe retention diagnostics for one track.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetainedTrackStats {
    pub completed_groups: usize,
    pub completed_objects: usize,
    pub completed_bytes: usize,
    pub pending_group: Option<u64>,
    pub pending_objects: usize,
    pub pending_bytes: usize,
    pub store_objects: usize,
    pub store_bytes: usize,
    pub active_snapshots: usize,
    pub largest_location: Option<Location>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn checked_bound(
    current: usize,
    additional: usize,
    maximum: usize,
    limit: RetentionLimit,
) -> Result<usize, RetentionError> {
    let total = current
        .checked_add(additional)
        .ok_or(RetentionError::ExcessiveLoad(limit))?;
    if total > maximum {
        Err(RetentionError::ExcessiveLoad(limit))
    } else {
        Ok(total)
    }
}

fn largest_location(state: &TrackState) -> Option<Location> {
    state
        .groups
        .last_key_value()
        .and_then(|(_, group)| group.objects.last())
        .map(RetainedObject::location)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn limits() -> RetentionLimits {
        RetentionLimits {
            max_object_bytes: 64,
            max_group_bytes: 256,
            max_objects_per_group: 8,
            max_groups: 2,
            max_store_bytes: 512,
            max_store_objects: 16,
            max_active_snapshots: 4,
        }
    }

    fn metadata(group_id: u64, object_id: u64, subgroup_id: Option<u64>) -> RetainedObjectMetadata {
        RetainedObjectMetadata {
            location: Location::new(group_id, object_id),
            subgroup_id,
            publisher_priority: 7,
            properties: ExtensionHeaders::new(),
            end_of_group: false,
        }
    }

    fn object(
        group_id: u64,
        object_id: u64,
        subgroup_id: Option<u64>,
        size: usize,
    ) -> RetainedObject {
        RetainedObject::new(
            metadata(group_id, object_id, subgroup_id),
            Bytes::from(vec![object_id as u8; size]),
        )
        .unwrap()
    }

    #[test]
    fn retained_object_preserves_live_end_of_group_metadata() {
        let mut metadata = metadata(1, 0, Some(0));
        metadata.end_of_group = true;
        let object = RetainedObject::new(metadata, Bytes::from_static(b"opus")).unwrap();
        assert!(object.end_of_group());
        assert!(object.metadata().end_of_group);
    }

    fn range(start: (u64, u64), end: (u64, u64)) -> RetainedRange {
        RetainedRange::new(Location::new(start.0, start.1), Location::new(end.0, end.1))
    }

    fn complete(track: &RetainedTrack, group_id: u64, ids: &[u64]) {
        for id in ids {
            track.commit(object(group_id, *id, None, 4)).unwrap();
        }
        track.complete_group(group_id).unwrap();
    }

    fn assert_error<T: std::fmt::Debug>(
        result: Result<T, RetentionError>,
        expected: RetentionError,
    ) {
        assert_eq!(result.err(), Some(expected));
    }

    fn budget(session_bytes: usize, process_bytes: usize) -> RetentionBudgetPool {
        RetentionBudgetPool::new(RetentionBudgetLimits {
            max_session_bytes: session_bytes,
            max_process_bytes: process_bytes,
        })
        .unwrap()
    }

    #[test]
    fn incomplete_object_is_rejected_and_invisible() {
        let track = RetainedTrack::new(limits()).unwrap();
        let builder = RetainedObjectBuilder::new(metadata(1, 0, None), 4);
        assert_eq!(
            builder.finish(Bytes::from_static(b"bad")),
            Err(RetentionError::InvalidRange(
                RetentionRangeError::IncompleteObject
            ))
        );
        assert_error(
            track.snapshot(range((1, 0), (2, 0)), GroupOrder::Ascending),
            RetentionError::InvalidRange(RetentionRangeError::NoObjects),
        );
        assert_eq!(track.stats().store_objects, 0);
    }

    #[test]
    fn bytes_are_shared_through_commit_and_snapshot() {
        let track = RetainedTrack::new(limits()).unwrap();
        let payload = Bytes::from_static(b"shared");
        let pointer = payload.as_ptr();
        track
            .commit(RetainedObject::new(metadata(1, 0, None), payload).unwrap())
            .unwrap();
        track.complete_group(1).unwrap();
        let snapshot = track
            .snapshot(range((1, 0), (2, 0)), GroupOrder::Ascending)
            .unwrap();
        assert_eq!(snapshot.iter().next().unwrap().payload().as_ptr(), pointer);
    }

    #[test]
    fn session_budget_rejects_n_plus_one_and_releases_on_cache_drop() {
        let charge = object(1, 0, Some(0), 4).retained_bytes();
        let pool = budget(charge * 2, charge * 8);
        let session = pool.session();
        let first = RetainedTrack::new_with_budget(limits(), session.clone()).unwrap();
        let second = RetainedTrack::new_with_budget(limits(), session.clone()).unwrap();
        let third = RetainedTrack::new_with_budget(limits(), session.clone()).unwrap();

        first.commit(object(1, 0, Some(0), 4)).unwrap();
        second.commit(object(2, 0, Some(0), 4)).unwrap();
        assert_eq!(
            third.commit(object(3, 0, Some(0), 4)),
            Err(RetentionError::ExcessiveLoad(RetentionLimit::SessionBytes))
        );
        assert_eq!(session.stats().session_bytes, charge * 2);

        drop(first);
        assert_eq!(session.stats().session_bytes, charge);
        third.commit(object(3, 0, Some(0), 4)).unwrap();
        assert_eq!(session.stats().session_bytes, charge * 2);
    }

    #[test]
    fn process_budget_failure_rolls_back_the_other_sessions_counter() {
        let charge = object(1, 0, Some(0), 4).retained_bytes();
        let pool = budget(charge * 2, charge * 2);
        let first_session = pool.session();
        let second_session = pool.session();
        let first = RetainedTrack::new_with_budget(limits(), first_session.clone()).unwrap();
        let second = RetainedTrack::new_with_budget(limits(), second_session.clone()).unwrap();

        first.commit(object(1, 0, Some(0), 4)).unwrap();
        first.commit(object(1, 1, Some(0), 4)).unwrap();
        assert_eq!(
            second.commit(object(2, 0, Some(0), 4)),
            Err(RetentionError::ExcessiveLoad(RetentionLimit::ProcessBytes))
        );
        assert_eq!(second_session.stats().session_bytes, 0);
        assert_eq!(second_session.stats().process_bytes, charge * 2);

        drop(first);
        assert_eq!(second_session.stats().process_bytes, 0);
        second.commit(object(2, 0, Some(0), 4)).unwrap();
        assert_eq!(second_session.stats().session_bytes, charge);
        assert_eq!(second_session.stats().process_bytes, charge);
    }

    #[test]
    fn process_pool_snapshot_aggregates_sessions_and_releases_raii_bytes() {
        let charge = object(1, 0, Some(0), 4).retained_bytes();
        let pool = budget(charge * 2, charge * 4);
        let first = RetainedTrack::new_with_budget(limits(), pool.session()).unwrap();
        let second = RetainedTrack::new_with_budget(limits(), pool.session()).unwrap();

        assert_eq!(
            pool.stats(),
            RetentionBudgetPoolStats {
                process_bytes: 0,
                max_process_bytes: charge * 4,
            }
        );
        first.commit(object(1, 0, Some(0), 4)).unwrap();
        second.commit(object(2, 0, Some(0), 4)).unwrap();
        assert_eq!(pool.stats().process_bytes, charge * 2);

        drop(first);
        assert_eq!(pool.stats().process_bytes, charge);
        drop(second);
        assert_eq!(pool.stats().process_bytes, 0);
    }

    #[test]
    fn completed_group_eviction_releases_shared_byte_capacity() {
        let charge = object(1, 0, Some(0), 4).retained_bytes();
        let pool = budget(charge * 3, charge * 3);
        let session = pool.session();
        let mut config = limits();
        config.max_groups = 1;
        let track = RetainedTrack::new_with_budget(config, session.clone()).unwrap();

        track.commit(object(1, 0, Some(0), 4)).unwrap();
        track.complete_group(1).unwrap();
        assert_eq!(session.stats().session_bytes, charge);
        track.commit(object(2, 0, Some(0), 4)).unwrap();
        assert_eq!(session.stats().session_bytes, charge * 2);
        track.complete_group(2).unwrap();
        assert_eq!(session.stats().session_bytes, charge);
    }

    #[test]
    fn pinned_snapshot_holds_budget_until_eviction_or_final_snapshot_drop() {
        let charge = object(1, 0, Some(0), 4).retained_bytes();
        let pool = budget(charge * 3, charge * 3);
        let session = pool.session();
        let mut config = limits();
        config.max_groups = 1;
        let track = RetainedTrack::new_with_budget(config, session.clone()).unwrap();

        track.commit(object(1, 0, Some(0), 4)).unwrap();
        track.complete_group(1).unwrap();
        let pinned = track
            .snapshot(range((1, 0), (2, 0)), GroupOrder::Ascending)
            .unwrap();
        track.commit(object(2, 0, Some(0), 4)).unwrap();
        assert_eq!(
            track.complete_group(2),
            Err(RetentionError::ExcessiveLoad(RetentionLimit::PinnedGroups))
        );
        assert_eq!(session.stats().session_bytes, charge * 2);
        drop(pinned);
        track.complete_group(2).unwrap();
        assert_eq!(session.stats().session_bytes, charge);

        let final_snapshot = track
            .snapshot(range((2, 0), (3, 0)), GroupOrder::Ascending)
            .unwrap();
        drop(track);
        assert_eq!(session.stats().session_bytes, charge);
        drop(final_snapshot);
        assert_eq!(session.stats().session_bytes, 0);
    }

    #[test]
    fn pending_group_is_invisible_until_atomic_completion() {
        let track = RetainedTrack::new(limits()).unwrap();
        track.commit(object(1, 1, None, 4)).unwrap();
        track.commit(object(1, 0, None, 4)).unwrap();
        assert_eq!(track.stats().pending_objects, 2);
        assert_error(
            track.snapshot(range((1, 0), (2, 0)), GroupOrder::Ascending),
            RetentionError::InvalidRange(RetentionRangeError::NoObjects),
        );

        track.complete_group(1).unwrap();
        let snapshot = track
            .snapshot(range((1, 0), (2, 0)), GroupOrder::Ascending)
            .unwrap();
        assert_eq!(
            snapshot
                .iter()
                .map(RetainedObject::object_id)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
    }

    #[tokio::test]
    async fn snapshot_waiter_wakes_only_after_atomic_group_completion() {
        let track = RetainedTrack::new(limits()).unwrap();
        let waiting = {
            let track = track.clone();
            tokio::spawn(async move {
                track
                    .snapshot_when_available(range((1, 0), (1, 2)), GroupOrder::Ascending)
                    .await
            })
        };
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());

        track.commit(object(1, 0, Some(0), 4)).unwrap();
        track.commit(object(1, 1, Some(0), 4)).unwrap();
        tokio::task::yield_now().await;
        assert!(
            !waiting.is_finished(),
            "pending Objects must stay invisible"
        );
        track.complete_group(1).unwrap();

        let snapshot = waiting.await.unwrap().unwrap();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(track.stats().active_snapshots, 1);
        drop(snapshot);
        assert_eq!(track.stats().active_snapshots, 0);
    }

    #[test]
    fn completed_group_rotation_occurs_only_at_completion() {
        let mut config = limits();
        config.max_groups = 1;
        let track = RetainedTrack::new(config).unwrap();
        complete(&track, 1, &[0]);
        track.commit(object(2, 0, None, 4)).unwrap();

        assert!(track
            .snapshot(range((1, 0), (2, 0)), GroupOrder::Ascending)
            .is_ok());
        assert_eq!(track.stats().pending_group, Some(2));

        track.complete_group(2).unwrap();
        assert_error(
            track.snapshot(range((1, 0), (2, 0)), GroupOrder::Ascending),
            RetentionError::InvalidRange(RetentionRangeError::NotRetained),
        );
        assert_eq!(
            track
                .snapshot(range((2, 0), (3, 0)), GroupOrder::Ascending)
                .unwrap()
                .iter()
                .map(RetainedObject::group_id)
                .collect::<Vec<_>>(),
            vec![2]
        );
    }

    #[test]
    fn snapshot_pins_rotation_then_retry_succeeds_after_drop() {
        let mut config = limits();
        config.max_groups = 1;
        let track = RetainedTrack::new(config).unwrap();
        complete(&track, 1, &[0]);
        let pinned = track
            .snapshot(range((1, 0), (2, 0)), GroupOrder::Ascending)
            .unwrap();
        track.commit(object(2, 0, None, 4)).unwrap();

        assert_eq!(
            track.complete_group(2),
            Err(RetentionError::ExcessiveLoad(RetentionLimit::PinnedGroups))
        );
        assert_eq!(track.stats().pending_group, Some(2));
        assert_eq!(pinned.iter().next().unwrap().group_id(), 1);

        drop(pinned);
        track.complete_group(2).unwrap();
        assert_eq!(track.stats().largest_location, Some(Location::new(2, 0)));
    }

    #[test]
    fn rotation_skips_pinned_oldest_and_evicts_oldest_unpinned() {
        let track = RetainedTrack::new(limits()).unwrap();
        complete(&track, 1, &[0]);
        let pinned = track
            .snapshot(range((1, 0), (2, 0)), GroupOrder::Ascending)
            .unwrap();
        complete(&track, 2, &[0]);
        track.commit(object(3, 0, None, 4)).unwrap();
        track.complete_group(3).unwrap();

        assert_eq!(pinned.iter().next().unwrap().group_id(), 1);
        assert_error(
            track.snapshot(range((2, 0), (3, 0)), GroupOrder::Ascending),
            RetentionError::InvalidRange(RetentionRangeError::NotRetained),
        );
        assert!(track
            .snapshot(range((3, 0), (4, 0)), GroupOrder::Ascending)
            .is_ok());
    }

    #[test]
    fn active_snapshot_bound_is_exact() {
        let mut config = limits();
        config.max_active_snapshots = 1;
        let track = RetainedTrack::new(config).unwrap();
        complete(&track, 1, &[0]);
        let first = track
            .snapshot(range((1, 0), (2, 0)), GroupOrder::Ascending)
            .unwrap();
        assert_eq!(track.stats().active_snapshots, 1);
        assert_error(
            track.snapshot(range((1, 0), (2, 0)), GroupOrder::Ascending),
            RetentionError::ExcessiveLoad(RetentionLimit::ActiveSnapshots),
        );
        drop(first);
        assert_eq!(track.stats().active_snapshots, 0);
    }

    #[test]
    fn object_group_and_store_bounds_have_specific_errors() {
        let mut object_config = limits();
        object_config.max_object_bytes = 4;
        let track = RetainedTrack::new(object_config).unwrap();
        assert_eq!(
            track.commit(object(1, 0, None, 4)),
            Err(RetentionError::ExcessiveLoad(RetentionLimit::ObjectBytes))
        );

        let mut group_object_config = limits();
        group_object_config.max_objects_per_group = 1;
        let track = RetainedTrack::new(group_object_config).unwrap();
        track.commit(object(1, 0, None, 4)).unwrap();
        assert_eq!(
            track.commit(object(1, 1, None, 4)),
            Err(RetentionError::ExcessiveLoad(RetentionLimit::GroupObjects))
        );

        let mut group_byte_config = limits();
        group_byte_config.max_object_bytes = 9;
        group_byte_config.max_group_bytes = 9;
        let track = RetainedTrack::new(group_byte_config).unwrap();
        track.commit(object(1, 0, None, 4)).unwrap();
        assert_eq!(
            track.commit(object(1, 1, None, 4)),
            Err(RetentionError::ExcessiveLoad(RetentionLimit::GroupBytes))
        );

        let mut store_object_config = limits();
        store_object_config.max_store_objects = 2;
        store_object_config.max_objects_per_group = 2;
        let track = RetainedTrack::new(store_object_config).unwrap();
        complete(&track, 1, &[0]);
        track.commit(object(2, 0, None, 4)).unwrap();
        assert_eq!(
            track.commit(object(2, 1, None, 4)),
            Err(RetentionError::ExcessiveLoad(RetentionLimit::StoreObjects))
        );

        let mut store_byte_config = limits();
        store_byte_config.max_store_bytes = 14;
        store_byte_config.max_group_bytes = 14;
        store_byte_config.max_object_bytes = 14;
        let track = RetainedTrack::new(store_byte_config).unwrap();
        complete(&track, 1, &[0]);
        track.commit(object(2, 0, None, 4)).unwrap();
        assert_eq!(
            track.commit(object(2, 1, None, 4)),
            Err(RetentionError::ExcessiveLoad(RetentionLimit::StoreBytes))
        );
    }

    #[test]
    fn invalid_limits_are_deterministic() {
        for invalid in [
            RetentionLimits {
                max_groups: 0,
                ..limits()
            },
            RetentionLimits {
                max_object_bytes: 65,
                max_group_bytes: 64,
                ..limits()
            },
            RetentionLimits {
                max_group_bytes: 513,
                max_store_bytes: 512,
                ..limits()
            },
            RetentionLimits {
                max_objects_per_group: 17,
                max_store_objects: 16,
                ..limits()
            },
        ] {
            assert_error(
                RetainedTrack::new(invalid),
                RetentionError::InvalidRange(RetentionRangeError::InvalidLimits),
            );
        }
    }

    #[test]
    fn ordering_is_by_group_and_object_never_subgroup() {
        let track = RetainedTrack::new(limits()).unwrap();
        track.commit(object(1, 9, Some(0), 4)).unwrap();
        track.commit(object(1, 1, Some(99), 4)).unwrap();
        track.commit(object(1, 5, None, 4)).unwrap();
        track.complete_group(1).unwrap();
        track.commit(object(2, 3, Some(1000), 4)).unwrap();
        track.commit(object(2, 0, Some(2), 4)).unwrap();
        track.complete_group(2).unwrap();

        let ascending = track
            .snapshot(range((1, 0), (3, 0)), GroupOrder::Ascending)
            .unwrap();
        assert_eq!(
            ascending
                .iter()
                .map(|object| (object.group_id(), object.object_id()))
                .collect::<Vec<_>>(),
            vec![(1, 1), (1, 5), (1, 9), (2, 0), (2, 3)]
        );

        let descending = track
            .snapshot(range((1, 0), (3, 0)), GroupOrder::Descending)
            .unwrap();
        assert_eq!(
            descending
                .iter()
                .map(|object| (object.group_id(), object.object_id()))
                .collect::<Vec<_>>(),
            vec![(2, 0), (2, 3), (1, 1), (1, 5), (1, 9)]
        );
    }

    #[test]
    fn sparse_range_filtering_is_half_open() {
        let track = RetainedTrack::new(limits()).unwrap();
        complete(&track, 4, &[0, 2, 7]);
        complete(&track, 5, &[1, 6]);
        let snapshot = track
            .snapshot(range((4, 2), (5, 6)), GroupOrder::Ascending)
            .unwrap();
        assert_eq!(
            snapshot
                .iter()
                .map(|object| (object.group_id(), object.object_id()))
                .collect::<Vec<_>>(),
            vec![(4, 2), (4, 7), (5, 1)]
        );
        assert_eq!(snapshot.len(), 3);
        assert_eq!(snapshot.retained_bytes(), 15);
    }

    #[test]
    fn invalid_ranges_and_missing_groups_are_distinct() {
        let track = RetainedTrack::new(limits()).unwrap();
        complete(&track, 1, &[0, 2]);
        track.commit(object(3, 0, None, 4)).unwrap();
        track.complete_group(3).unwrap();

        assert_error(
            track.snapshot(range((1, 0), (1, 0)), GroupOrder::Ascending),
            RetentionError::InvalidRange(RetentionRangeError::EmptyOrReversed),
        );
        assert_error(
            track.snapshot(range((4, 0), (5, 0)), GroupOrder::Ascending),
            RetentionError::InvalidRange(RetentionRangeError::StartAfterLargest),
        );
        assert_error(
            track.snapshot(range((1, 0), (4, 0)), GroupOrder::Ascending),
            RetentionError::InvalidRange(RetentionRangeError::NotRetained),
        );
        assert_error(
            track.snapshot(range((1, 0), (2, 0)), GroupOrder::Publisher),
            RetentionError::InvalidRange(RetentionRangeError::UnsupportedGroupOrder),
        );
    }

    #[test]
    fn duplicate_and_group_lifecycle_errors_are_deterministic() {
        let track = RetainedTrack::new(limits()).unwrap();
        let original = object(1, 0, None, 4);
        track.commit(original.clone()).unwrap();
        track.commit(original).unwrap();
        assert_eq!(track.stats().pending_objects, 1);

        assert_eq!(
            track.commit(object(1, 0, None, 5)),
            Err(RetentionError::InvalidRange(
                RetentionRangeError::ConflictingObject
            ))
        );
        assert_eq!(
            track.commit(object(2, 0, None, 4)),
            Err(RetentionError::InvalidRange(
                RetentionRangeError::PendingGroupMismatch
            ))
        );
        assert_eq!(
            track.complete_group(2),
            Err(RetentionError::InvalidRange(
                RetentionRangeError::PendingGroupMismatch
            ))
        );
        track.complete_group(1).unwrap();
        assert_eq!(
            track.complete_group(1),
            Err(RetentionError::InvalidRange(
                RetentionRangeError::NoPendingGroup
            ))
        );
        assert_eq!(
            track.commit(object(1, 1, None, 4)),
            Err(RetentionError::InvalidRange(
                RetentionRangeError::NonIncreasingGroup
            ))
        );
    }

    #[test]
    fn snapshot_outlives_track() {
        let snapshot = {
            let track = RetainedTrack::new(limits()).unwrap();
            complete(&track, 1, &[0, 1]);
            track
                .snapshot(range((1, 0), (2, 0)), GroupOrder::Ascending)
                .unwrap()
        };
        assert_eq!(
            snapshot
                .iter()
                .map(RetainedObject::object_id)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
    }

    #[test]
    fn stats_account_for_pending_completed_and_snapshots() {
        let track = RetainedTrack::new(limits()).unwrap();
        complete(&track, 1, &[0, 1]);
        track.commit(object(2, 0, None, 4)).unwrap();
        let snapshot = track
            .snapshot(range((1, 0), (2, 0)), GroupOrder::Ascending)
            .unwrap();
        let stats = track.stats();
        assert_eq!(stats.completed_groups, 1);
        assert_eq!(stats.completed_objects, 2);
        assert_eq!(stats.completed_bytes, 10);
        assert_eq!(stats.pending_group, Some(2));
        assert_eq!(stats.pending_objects, 1);
        assert_eq!(stats.pending_bytes, 5);
        assert_eq!(stats.store_objects, 3);
        assert_eq!(stats.store_bytes, 15);
        assert_eq!(stats.active_snapshots, 1);
        assert_eq!(stats.largest_location, Some(Location::new(1, 1)));
        drop(snapshot);
    }

    #[test]
    fn selected_groups_are_pinned_even_when_range_contains_no_object() {
        let track = RetainedTrack::new(limits()).unwrap();
        complete(&track, 1, &[0, 2]);
        complete(&track, 2, &[10]);
        let snapshot = track
            .snapshot(range((1, 1), (1, 2)), GroupOrder::Ascending)
            .unwrap();
        assert!(snapshot.is_empty());
        assert_eq!(snapshot.pinned_group_count(), 1);
    }

    #[test]
    fn every_group_in_range_must_be_retained() {
        let mut config = limits();
        config.max_groups = 3;
        let track = RetainedTrack::new(config).unwrap();
        complete(&track, 10, &[0]);
        complete(&track, 12, &[0]);
        assert_error(
            track.snapshot(range((10, 0), (13, 0)), GroupOrder::Ascending),
            RetentionError::InvalidRange(RetentionRangeError::NotRetained),
        );
    }

    #[test]
    fn no_subgroup_dimension_leaks_into_group_identity() {
        let track = RetainedTrack::new(limits()).unwrap();
        for (object_id, subgroup_id) in [(0, Some(7)), (1, Some(2)), (2, None)] {
            track.commit(object(1, object_id, subgroup_id, 4)).unwrap();
        }
        track.complete_group(1).unwrap();
        let snapshot = track
            .snapshot(range((1, 0), (2, 0)), GroupOrder::Ascending)
            .unwrap();
        let groups: BTreeSet<_> = snapshot.iter().map(RetainedObject::group_id).collect();
        assert_eq!(groups, BTreeSet::from([1]));
    }
}
