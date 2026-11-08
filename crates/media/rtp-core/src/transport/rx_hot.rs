//! Hot call state ([`MediaExecutorConfig::hot_state`]): the receive loop's
//! per-datagram control entry, kept in its lane's slab.
//!
//! Per datagram the receive loop loads atomics that live in separate
//! allocations (separate cache lines, cold again 20 ms later): the
//! transport's `active` flag, the symmetric-RTP generation, the inbound SRTP
//! epoch and the latched RTP SSRC. With hot state the loop reads the
//! transport's [`HotDoorbell`](crate::hot_state::HotDoorbell) instead and
//! runs each original check only when the bell moved since that check last
//! ran ([`RxHotEntry`] keeps one snapshot per check, so a check skipped for
//! one datagram — RTCP, STUN, a dropped packet — is never lost). The latched
//! RTP SSRC, which only this loop writes, is remembered instead of loaded.
//!
//! The symmetric-RTP learner (the latch state) moves into the same entry.
//! On a dedicated current-thread media lane the entry lives in that lane's
//! [`HotSlab`]: lane-owned, cache-line aligned, dense, addressed by a small
//! generation-checked handle. Elsewhere (a multi-thread runtime, where a
//! poller task may move between workers) it stays local to the loop. Either
//! way every decision is the one the plain learner makes, in the same order.
//!
//! [`MediaExecutorConfig::hot_state`]: crate::executor::MediaExecutorConfig::hot_state

use std::cell::RefCell;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use super::symmetric::{SymmetricRtpDecision, SymmetricRtpLearner};
use super::SymmetricRtpPolicy;
use crate::hot_state::{gate, HotHandle, HotSlab, NEVER_SEEN};

/// What the receive loop checks and learns per datagram.
pub(crate) struct RxHotEntry {
    /// Bell at the last `active` check (top of the loop).
    bell_top: u64,
    /// Bell at the last inbound SRTP epoch check.
    bell_srtp: u64,
    /// Bell at the last symmetric-RTP generation check.
    bell_generation: u64,
    /// The generation the learner state belongs to.
    observed_generation: u64,
    /// The SSRC this loop last stored to (or found in) the transport's
    /// latched RTP SSRC; [`NEVER_SEEN`] until the first admitted packet.
    latched_ssrc: u64,
    learner: SymmetricRtpLearner,
}

impl RxHotEntry {
    fn new(policy: SymmetricRtpPolicy) -> Self {
        Self {
            bell_top: NEVER_SEEN,
            bell_srtp: NEVER_SEEN,
            bell_generation: NEVER_SEEN,
            // The first packet adopts the current generation; resetting the
            // fresh learner then is a no-op, exactly like a fresh loop.
            observed_generation: NEVER_SEEN,
            latched_ssrc: NEVER_SEEN,
            learner: SymmetricRtpLearner::new(policy),
        }
    }
}

/// One RTP packet's symmetric-RTP verdict from [`RxHot::observe`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RxHotVerdict {
    pub(crate) decision: SymmetricRtpDecision,
    /// The transport's latched RTP SSRC already holds this packet's SSRC
    /// (this loop is its only writer), so admitting the packet needs no load.
    pub(crate) latched_ssrc_known: bool,
}

thread_local! {
    /// The lane's slab of receive-loop entries. Only loops polled on this
    /// thread's current-thread runtime insert here.
    static LANE_RX_SLAB: RefCell<HotSlab<RxHotEntry>> = RefCell::new(HotSlab::new());
}

enum Home {
    Lane { slab: u64, handle: HotHandle },
    Local(RxHotEntry),
}

/// One receive loop's control entry (see the module docs).
pub(crate) struct RxHot {
    policy: SymmetricRtpPolicy,
    home: Home,
}

impl RxHot {
    /// Place the entry: in this lane's slab when called from a task on a
    /// current-thread runtime (a dedicated media lane, whose tasks never
    /// leave its thread), locally otherwise. Called on the loop's first
    /// poll, never per packet.
    pub(crate) fn new(policy: SymmetricRtpPolicy) -> Self {
        let current_thread = tokio::runtime::Handle::try_current().is_ok_and(|runtime| {
            runtime.runtime_flavor() == tokio::runtime::RuntimeFlavor::CurrentThread
        });
        if current_thread {
            let placed = LANE_RX_SLAB.try_with(|cell| {
                let mut lane = cell.try_borrow_mut().ok()?;
                let handle = lane.insert(RxHotEntry::new(policy));
                Some((lane.id(), handle))
            });
            if let Ok(Some((slab, handle))) = placed {
                crate::hot_state::record_hot_rx_entry(true);
                return Self {
                    policy,
                    home: Home::Lane { slab, handle },
                };
            }
        }
        crate::hot_state::record_hot_rx_entry(false);
        Self::local(policy)
    }

    fn local(policy: SymmetricRtpPolicy) -> Self {
        Self {
            policy,
            home: Home::Local(RxHotEntry::new(policy)),
        }
    }

    #[cfg(test)]
    fn on_lane(&self) -> bool {
        matches!(self.home, Home::Lane { .. })
    }

    /// Run `op` on the entry. A lane entry that cannot be reached (it never
    /// happens on a dedicated lane: the loop is only polled on its thread)
    /// is replaced by a fresh local one, so the loop keeps working: every
    /// snapshot is then [`NEVER_SEEN`] (all checks run again) and the learner
    /// starts over, as after a generation reset.
    fn entry_op<R>(&mut self, op: impl Fn(&mut RxHotEntry) -> R) -> R {
        if let Home::Lane { slab, handle } = self.home {
            let done = LANE_RX_SLAB.try_with(|cell| {
                let mut lane = cell.try_borrow_mut().ok()?;
                if lane.id() != slab {
                    return None;
                }
                lane.get_mut(handle).map(&op)
            });
            if let Ok(Some(result)) = done {
                return result;
            }
            crate::hot_state::record_hot_rx_fallback();
            *self = Self::local(self.policy);
        }
        match &mut self.home {
            Home::Local(entry) => op(entry),
            // Unreachable: replaced just above. Stateless rather than a panic.
            Home::Lane { .. } => op(&mut RxHotEntry::new(self.policy)),
        }
    }

    /// Top of the loop: whether to check the transport's `active` flag.
    #[inline]
    pub(crate) fn gate_top(&mut self, bell: u64) -> bool {
        self.entry_op(|entry| gate(&mut entry.bell_top, bell))
    }

    /// Before the inbound SRTP context is used: whether to check its epoch.
    #[inline]
    pub(crate) fn gate_srtp(&mut self, bell: u64) -> bool {
        self.entry_op(|entry| gate(&mut entry.bell_srtp, bell))
    }

    /// The plain loop's generation check and `SymmetricRtpLearner::observe`
    /// for one authenticated RTP packet; the generation is loaded only when
    /// the bell moved since the last check. When the decision admits the
    /// packet the remembered latched SSRC becomes its SSRC, which is what
    /// the admission stores.
    #[inline]
    pub(crate) fn observe(
        &mut self,
        bell: u64,
        generation: &AtomicU64,
        source: SocketAddr,
        ssrc: u32,
        sequence: u16,
        now: Instant,
    ) -> RxHotVerdict {
        self.entry_op(|entry| {
            if gate(&mut entry.bell_generation, bell) {
                let current = generation.load(Ordering::Acquire);
                if current != entry.observed_generation {
                    entry.learner.reset();
                    entry.observed_generation = current;
                }
            }
            let decision = entry.learner.observe(source, ssrc, sequence, now);
            let latched_ssrc_known = entry.latched_ssrc == u64::from(ssrc);
            if matches!(
                decision,
                SymmetricRtpDecision::Accept
                    | SymmetricRtpDecision::LatchInitial
                    | SymmetricRtpDecision::Rebind
            ) {
                entry.latched_ssrc = u64::from(ssrc);
            }
            RxHotVerdict {
                decision,
                latched_ssrc_known,
            }
        })
    }
}

impl Drop for RxHot {
    fn drop(&mut self) {
        if let Home::Lane { slab, handle } = self.home {
            let removed = LANE_RX_SLAB
                .try_with(|cell| {
                    let Ok(mut lane) = cell.try_borrow_mut() else {
                        return false;
                    };
                    lane.id() == slab && lane.remove(handle).is_some()
                })
                .unwrap_or(false);
            if !removed {
                // Dropped off its lane thread (runtime teardown): the entry
                // stays in that lane's slab until the thread exits.
                crate::hot_state::record_hot_rx_foreign_drop();
            }
        }
    }
}

/// Entries in this thread's lane slab.
#[cfg(test)]
pub(crate) fn lane_rx_entries() -> usize {
    LANE_RX_SLAB.with(|cell| cell.borrow().len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn addr(port: u16) -> SocketAddr {
        SocketAddr::from(([192, 0, 2, 10], port))
    }

    #[tokio::test]
    async fn a_current_thread_lane_keeps_the_entry_in_its_slab_until_drop() {
        let before = lane_rx_entries();
        let hot = RxHot::new(SymmetricRtpPolicy::default());
        assert!(hot.on_lane(), "a current-thread runtime is a lane");
        assert_eq!(lane_rx_entries(), before + 1);
        let second = RxHot::new(SymmetricRtpPolicy::default());
        assert_eq!(lane_rx_entries(), before + 2);
        drop(hot);
        assert_eq!(
            lane_rx_entries(),
            before + 1,
            "drop releases exactly its entry"
        );
        drop(second);
        assert_eq!(lane_rx_entries(), before);
    }

    #[test]
    fn outside_a_current_thread_runtime_the_entry_is_local() {
        assert!(!RxHot::new(SymmetricRtpPolicy::default()).on_lane());
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .build()
            .unwrap();
        let local = runtime.block_on(async {
            tokio::spawn(async { RxHot::new(SymmetricRtpPolicy::default()).on_lane() })
                .await
                .unwrap()
        });
        assert!(
            !local,
            "a multi-thread worker may move the poller: keep it local"
        );
    }

    /// The gated learner decides exactly what the plain learner decides for
    /// a latch, a probation, a rebind, a rejected spoof and a generation
    /// reset (a bell ring after the generation moved), lane and local alike;
    /// the remembered latched SSRC is exactly what the admissions stored.
    #[tokio::test]
    async fn gated_decisions_match_the_plain_learner_on_lane_and_local() {
        let policy = SymmetricRtpPolicy::default();
        let start = Instant::now();
        let script: Vec<(SocketAddr, u32, u16, bool)> = vec![
            (addr(20_000), 7, 100, false),
            (addr(20_000), 7, 101, false),
            (addr(30_000), 7, 102, false),
            (addr(30_000), 7, 103, false),
            (addr(30_000), 7, 104, false),
            (addr(40_000), 9, 105, false),
            (addr(30_000), 7, 106, false),
            (addr(30_000), 8, 107, false),
            (addr(30_000), 8, 108, false),
            // The control plane moves the generation and rings.
            (addr(50_000), 7, 109, true),
            (addr(50_000), 7, 110, false),
        ];
        for lane in [true, false] {
            let generation = AtomicU64::new(3);
            let bell = crate::hot_state::HotDoorbell::new();
            let mut hot = if lane {
                RxHot::new(policy)
            } else {
                RxHot::local(policy)
            };
            assert_eq!(hot.on_lane(), lane);
            let mut plain = SymmetricRtpLearner::new(policy);
            let mut plain_observed = generation.load(Ordering::Acquire);
            let mut stored_ssrc = NEVER_SEEN;
            for (index, &(source, ssrc, sequence, reset)) in script.iter().enumerate() {
                if reset {
                    generation.fetch_add(1, Ordering::AcqRel);
                    bell.ring();
                }
                let now = start + Duration::from_millis(index as u64 * 20);
                let current = generation.load(Ordering::Acquire);
                if current != plain_observed {
                    plain.reset();
                    plain_observed = current;
                }
                let expected = plain.observe(source, ssrc, sequence, now);
                let got = hot.observe(bell.read(), &generation, source, ssrc, sequence, now);
                assert_eq!(got.decision, expected, "lane={lane} packet {index}");
                assert_eq!(
                    got.latched_ssrc_known,
                    stored_ssrc == u64::from(ssrc),
                    "lane={lane} packet {index}"
                );
                if matches!(
                    expected,
                    SymmetricRtpDecision::Accept
                        | SymmetricRtpDecision::LatchInitial
                        | SymmetricRtpDecision::Rebind
                ) {
                    stored_ssrc = u64::from(ssrc);
                }
            }
        }
    }

    #[tokio::test]
    async fn each_check_has_its_own_snapshot() {
        let bell = crate::hot_state::HotDoorbell::new();
        let mut hot = RxHot::new(SymmetricRtpPolicy::default());
        assert!(
            hot.gate_top(bell.read()),
            "first datagram checks everything"
        );
        assert!(hot.gate_srtp(bell.read()));
        assert!(!hot.gate_top(bell.read()));
        assert!(!hot.gate_srtp(bell.read()));
        bell.ring();
        // A datagram that only reached the SRTP check (say RTCP) consumes
        // that check's ring, not the others'.
        assert!(hot.gate_srtp(bell.read()));
        assert!(!hot.gate_srtp(bell.read()));
        assert!(
            hot.gate_top(bell.read()),
            "the top check still sees the ring"
        );
    }

    #[tokio::test]
    async fn an_unreachable_lane_entry_falls_back_without_panicking() {
        let before = crate::hot_state::hot_state_counters().rx_fallbacks;
        let mut hot = RxHot::new(SymmetricRtpPolicy::default());
        let Home::Lane { handle, .. } = hot.home else {
            panic!("expected a lane entry");
        };
        LANE_RX_SLAB.with(|cell| cell.borrow_mut().remove(handle));
        let bell = crate::hot_state::HotDoorbell::new();
        assert!(
            hot.gate_top(bell.read()),
            "a fresh entry checks everything again"
        );
        assert!(!hot.on_lane());
        assert!(crate::hot_state::hot_state_counters().rx_fallbacks > before);
    }

    #[test]
    fn a_receive_loop_dropped_off_its_lane_thread_leaves_its_entry_and_counts_it() {
        let before = crate::hot_state::hot_state_counters().rx_foreign_drops;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let hot = runtime.block_on(async { RxHot::new(SymmetricRtpPolicy::default()) });
        assert!(hot.on_lane());
        std::thread::spawn(move || drop(hot)).join().unwrap();
        assert!(crate::hot_state::hot_state_counters().rx_foreign_drops > before);
    }
}
