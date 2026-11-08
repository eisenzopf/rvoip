//! Bookkeeping for the orphan session sweeper.
//!
//! A SIP call session normally has an owner that will end it: the adapter
//! route that maps it to an orchestrator connection, an in-flight inbound
//! rejection, or a retained reclaim. A setup abandoned on an unanticipated
//! path can lose every owner and stay in the session store for good, along
//! with its media port. The sweeper finds call sessions that have had no
//! owner and received no RTP for
//! [`SessionCleanupConfig::orphan_session_ttl`](crate::api::unified::SessionCleanupConfig::orphan_session_ttl)
//! and hands each one to a bounded reclaim.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;

use crate::session_registry::SessionRegistryHandle;
use crate::types::CallState;
use crate::SessionId;

/// Reclaims the sweeper may run at once; the rest wait for a later pass.
pub(crate) const ORPHAN_RECLAIM_CONCURRENCY: usize = 256;

/// The sweeper schedule for one orphan TTL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OrphanSweepConfig {
    /// How long a session must be ownerless, with no new RTP, before it is
    /// reclaimed.
    pub ttl: Duration,
    /// Pass interval: a sixth of the TTL, clamped to 1 s to 10 s, so a session
    /// is reclaimed no later than TTL plus one interval after it lost its
    /// owner.
    pub interval: Duration,
}

impl OrphanSweepConfig {
    pub(crate) fn with_ttl(ttl: Duration) -> Self {
        Self {
            ttl,
            interval: (ttl / 6).clamp(Duration::from_secs(1), Duration::from_secs(10)),
        }
    }
}

/// Whether a session in this state is a call the sweeper may reclaim.
///
/// Registration, subscription and messaging lifetimes are never calls, and
/// `Idle` is how a registration session starts and ends, so all of them are
/// left alone whatever their routing.
pub(crate) fn sweepable_state(state: CallState) -> bool {
    !matches!(
        state,
        CallState::Idle
            | CallState::Registering
            | CallState::Registered
            | CallState::Unregistering
            | CallState::Subscribing
            | CallState::Subscribed
            | CallState::Publishing
            | CallState::Authenticating
            | CallState::Messaging
    )
}

/// Established states hold an ACKed dialog, so the reclaim sends a BYE first
/// and the peer does not keep a call nobody serves.
pub(crate) fn established_state(state: CallState) -> bool {
    matches!(
        state,
        CallState::Active
            | CallState::HoldPending
            | CallState::OnHold
            | CallState::Resuming
            | CallState::Muted
            | CallState::Bridged
            | CallState::Transferring
            | CallState::TransferringCall
            | CallState::ConsultationCall
    )
}

/// Stable label for a lifecycle state (`Failed(_)` drops its reason).
pub(crate) fn state_label(state: CallState) -> &'static str {
    match state {
        CallState::Idle => "Idle",
        CallState::Initiating => "Initiating",
        CallState::CancelPending => "CancelPending",
        CallState::Cancelling => "Cancelling",
        CallState::Ringing => "Ringing",
        CallState::Answering => "Answering",
        CallState::AnsweringHangupPending => "AnsweringHangupPending",
        CallState::EarlyMedia => "EarlyMedia",
        CallState::Active => "Active",
        CallState::HoldPending => "HoldPending",
        CallState::OnHold => "OnHold",
        CallState::Resuming => "Resuming",
        CallState::Muted => "Muted",
        CallState::Bridged => "Bridged",
        CallState::Transferring => "Transferring",
        CallState::TransferringCall => "TransferringCall",
        CallState::ConsultationCall => "ConsultationCall",
        CallState::Terminating => "Terminating",
        CallState::Terminated => "Terminated",
        CallState::Cancelled => "Cancelled",
        CallState::Failed(_) => "Failed",
        CallState::Registering => "Registering",
        CallState::Registered => "Registered",
        CallState::Unregistering => "Unregistering",
        CallState::Subscribing => "Subscribing",
        CallState::Subscribed => "Subscribed",
        CallState::Publishing => "Publishing",
        CallState::Authenticating => "Authenticating",
        CallState::Messaging => "Messaging",
    }
}

/// One current session-store lifetime, as the sweeper sees it.
#[derive(Debug, Clone)]
pub(crate) struct SweepEntry {
    pub session_id: SessionId,
    pub handle: SessionRegistryHandle,
    pub call_state: CallState,
}

#[derive(Debug, Clone, Copy)]
struct OrphanObservation {
    since: tokio::time::Instant,
    rtp_packets_received: Option<u64>,
}

/// What one pass found.
#[derive(Debug, Default)]
pub(crate) struct OrphanSweepPass {
    /// Ownerless lifetimes past the TTL with no new RTP, oldest first.
    pub due: Vec<(SessionRegistryHandle, CallState)>,
    /// Ownerless call sessions seen this pass, by lifecycle state.
    pub orphans_by_state: BTreeMap<&'static str, u64>,
}

impl OrphanSweepPass {
    pub(crate) fn orphans(&self) -> u64 {
        self.orphans_by_state.values().sum()
    }
}

/// Per-lifetime orphan clocks, owned by the single sweeper loop.
///
/// A lifetime's clock starts on the first pass that finds it ownerless and
/// restarts whenever it regains an owner or receives RTP. A lifetime handed
/// to a reclaim is owned while the reclaim runs, so if the reclaim does not
/// remove it, its clock restarts and the next attempt is one TTL later.
#[derive(Debug, Default)]
pub(crate) struct OrphanTracker {
    observed: HashMap<SessionRegistryHandle, OrphanObservation>,
}

impl OrphanTracker {
    #[cfg(test)]
    pub(crate) fn tracked(&self) -> usize {
        self.observed.len()
    }

    /// Classify `entries` and return the lifetimes due for reclaim.
    ///
    /// `owned` says whether a lifetime has an owner that will end it (an
    /// adapter route, an in-flight rejection or reclaim). `rtp_packets`
    /// reads the lifetime's RTP receive counter; it is read only when a clock
    /// starts and when it expires.
    pub(crate) async fn pass<O, R, F>(
        &mut self,
        entries: Vec<SweepEntry>,
        now: tokio::time::Instant,
        ttl: Duration,
        owned: O,
        rtp_packets: R,
    ) -> OrphanSweepPass
    where
        O: Fn(&SessionId, &SessionRegistryHandle) -> bool,
        R: Fn(SessionId) -> F,
        F: std::future::Future<Output = Option<u64>>,
    {
        let mut pass = OrphanSweepPass::default();
        let mut live = HashSet::with_capacity(self.observed.len());
        let mut due = Vec::new();
        for entry in entries {
            if !sweepable_state(entry.call_state) || owned(&entry.session_id, &entry.handle) {
                continue;
            }
            *pass
                .orphans_by_state
                .entry(state_label(entry.call_state))
                .or_default() += 1;
            live.insert(entry.handle.clone());
            let Some(observation) = self.observed.get(&entry.handle).copied() else {
                let packets = rtp_packets(entry.session_id.clone()).await;
                self.observed.insert(
                    entry.handle,
                    OrphanObservation {
                        since: now,
                        rtp_packets_received: packets,
                    },
                );
                continue;
            };
            if now.saturating_duration_since(observation.since) < ttl {
                continue;
            }
            let packets = rtp_packets(entry.session_id.clone()).await;
            if media_advanced(observation.rtp_packets_received, packets) {
                self.observed.insert(
                    entry.handle,
                    OrphanObservation {
                        since: now,
                        rtp_packets_received: packets,
                    },
                );
                continue;
            }
            due.push((observation.since, entry.handle, entry.call_state));
        }
        self.observed.retain(|handle, _| live.contains(handle));
        due.sort_by_key(|(since, _, _)| *since);
        pass.due = due
            .into_iter()
            .map(|(_, handle, state)| (handle, state))
            .collect();
        pass
    }

    /// Forget a lifetime handed to a reclaim. A later pass starts its clock
    /// again if it is still present and ownerless.
    pub(crate) fn forget(&mut self, handle: &SessionRegistryHandle) {
        self.observed.remove(handle);
    }
}

fn media_advanced(before: Option<u64>, after: Option<u64>) -> bool {
    match (before, after) {
        (Some(before), Some(after)) => after > before,
        (None, Some(after)) => after > 0,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweep_interval_is_a_sixth_of_the_ttl_within_bounds() {
        assert_eq!(
            OrphanSweepConfig::with_ttl(Duration::from_secs(60)),
            OrphanSweepConfig {
                ttl: Duration::from_secs(60),
                interval: Duration::from_secs(10),
            }
        );
        assert_eq!(
            OrphanSweepConfig::with_ttl(Duration::from_secs(5)).interval,
            Duration::from_secs(1)
        );
        assert_eq!(
            OrphanSweepConfig::with_ttl(Duration::from_secs(3_600)).interval,
            Duration::from_secs(10)
        );
    }

    #[test]
    fn sweeper_never_touches_registration_or_idle_lifetimes() {
        for state in [
            CallState::Idle,
            CallState::Registering,
            CallState::Registered,
            CallState::Unregistering,
            CallState::Subscribing,
            CallState::Subscribed,
            CallState::Publishing,
            CallState::Authenticating,
            CallState::Messaging,
        ] {
            assert!(!sweepable_state(state), "{state:?}");
        }
        for state in [
            CallState::Ringing,
            CallState::Answering,
            CallState::AnsweringHangupPending,
            CallState::Active,
            CallState::Terminating,
            CallState::Terminated,
            CallState::Failed(crate::types::FailureReason::Timeout),
        ] {
            assert!(sweepable_state(state), "{state:?}");
        }
        assert!(established_state(CallState::Active));
        assert!(!established_state(CallState::AnsweringHangupPending));
        assert_eq!(
            state_label(CallState::Failed(crate::types::FailureReason::Timeout)),
            "Failed"
        );
    }

    #[test]
    fn media_activity_restarts_the_orphan_clock() {
        assert!(media_advanced(Some(10), Some(11)));
        assert!(media_advanced(None, Some(1)));
        assert!(!media_advanced(Some(10), Some(10)));
        assert!(!media_advanced(None, None));
        assert!(!media_advanced(Some(10), None));
    }
}
