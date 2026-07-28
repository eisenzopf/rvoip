use std::collections::HashMap;
use std::sync::Mutex;

use crate::MoqNamespace;

/// Atomically reserves monotonically increasing MOQT Group IDs.
///
/// A production implementation must durably advance its stored next value
/// before returning from [`Self::reserve_next_group`]. A successfully
/// reserved ID is consumed even if the subsequent object write fails. This
/// contract prevents a restarted origin from reusing an ID that a relay or
/// subscriber may already have observed.
pub trait MoqGroupIdAllocator: Send + Sync {
    /// Reserve and durably consume the next Group ID for one track.
    fn reserve_next_group(
        &self,
        namespace: &MoqNamespace,
        track: &str,
    ) -> Result<u64, MoqGroupIdAllocationError>;

    /// Recover a track after externally persisted history.
    ///
    /// Future reservations are guaranteed to be strictly greater than
    /// `previous_group_id`. Implementations must never move their stored next
    /// value backwards.
    fn recover_above(
        &self,
        namespace: &MoqNamespace,
        track: &str,
        previous_group_id: u64,
    ) -> Result<(), MoqGroupIdAllocationError>;
}

/// Process-local allocator for development and tests.
///
/// Reuse the same instance across publisher reconstruction to preserve group
/// monotonicity in one process. Clustered deployments should inject a durable
/// implementation of [`MoqGroupIdAllocator`].
#[derive(Default)]
pub struct InMemoryMoqGroupIdAllocator {
    // `None` is a durable-in-process exhaustion tombstone, not an absent key.
    next_by_track: Mutex<HashMap<(String, String), Option<u64>>>,
}

impl InMemoryMoqGroupIdAllocator {
    pub fn new() -> Self {
        Self::default()
    }
}

impl MoqGroupIdAllocator for InMemoryMoqGroupIdAllocator {
    fn reserve_next_group(
        &self,
        namespace: &MoqNamespace,
        track: &str,
    ) -> Result<u64, MoqGroupIdAllocationError> {
        let mut next_by_track = self
            .next_by_track
            .lock()
            .map_err(|_| MoqGroupIdAllocationError::Unavailable)?;
        let next = next_by_track
            .entry((namespace.to_string(), track.to_owned()))
            .or_insert(Some(0));
        let reserved = next.ok_or(MoqGroupIdAllocationError::Exhausted)?;
        // u64::MAX is a valid draft-19 vi64 Group ID. Reserve it once,
        // leaving an exhaustion tombstone for every subsequent call.
        *next = reserved.checked_add(1);
        Ok(reserved)
    }

    fn recover_above(
        &self,
        namespace: &MoqNamespace,
        track: &str,
        previous_group_id: u64,
    ) -> Result<(), MoqGroupIdAllocationError> {
        let mut next_by_track = self
            .next_by_track
            .lock()
            .map_err(|_| MoqGroupIdAllocationError::Unavailable)?;
        let next = next_by_track
            .entry((namespace.to_string(), track.to_owned()))
            .or_insert(Some(0));
        let Some(recovered_next) = previous_group_id.checked_add(1) else {
            *next = None;
            return Err(MoqGroupIdAllocationError::Exhausted);
        };
        if let Some(current) = next {
            *current = (*current).max(recovered_next);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MoqGroupIdAllocationError {
    #[error("MOQT Group ID space is exhausted")]
    Exhausted,
    #[error("MOQT Group ID allocator is unavailable")]
    Unavailable,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservations_are_per_track_monotonic_and_recovery_never_moves_backwards() {
        let namespace = MoqNamespace::new("tenant", "broadcast").unwrap();
        let allocator = InMemoryMoqGroupIdAllocator::new();

        assert_eq!(
            allocator
                .reserve_next_group(&namespace, "audio/main")
                .unwrap(),
            0
        );
        assert_eq!(
            allocator
                .reserve_next_group(&namespace, "audio/main")
                .unwrap(),
            1
        );
        assert_eq!(
            allocator.reserve_next_group(&namespace, "catalog").unwrap(),
            0
        );

        allocator
            .recover_above(&namespace, "audio/main", 40)
            .unwrap();
        allocator
            .recover_above(&namespace, "audio/main", 2)
            .unwrap();
        assert_eq!(
            allocator
                .reserve_next_group(&namespace, "audio/main")
                .unwrap(),
            41
        );
    }

    #[test]
    fn recovery_rejects_an_exhausted_prior_id() {
        let namespace = MoqNamespace::new("tenant", "broadcast").unwrap();
        let allocator = InMemoryMoqGroupIdAllocator::new();
        assert_eq!(
            allocator
                .recover_above(&namespace, "audio/main", u64::MAX)
                .unwrap_err(),
            MoqGroupIdAllocationError::Exhausted
        );
        assert_eq!(
            allocator
                .reserve_next_group(&namespace, "audio/main")
                .unwrap_err(),
            MoqGroupIdAllocationError::Exhausted
        );
    }

    #[test]
    fn maximum_group_id_is_reserved_once_before_exhaustion() {
        let namespace = MoqNamespace::new("tenant", "broadcast").unwrap();
        let allocator = InMemoryMoqGroupIdAllocator::new();
        allocator
            .recover_above(&namespace, "audio/main", u64::MAX - 1)
            .unwrap();
        assert_eq!(
            allocator
                .reserve_next_group(&namespace, "audio/main")
                .unwrap(),
            u64::MAX
        );
        assert_eq!(
            allocator
                .reserve_next_group(&namespace, "audio/main")
                .unwrap_err(),
            MoqGroupIdAllocationError::Exhausted
        );
    }
}
