//! Lock-free server call admission
//! ([`Config::server_call_admission_lock_free`](crate::api::unified::Config::server_call_admission_lock_free)).
//!
//! The default admission path takes one handler-wide mutex for every inbound
//! INVITE to read `sessions + pending`, apply the overload hysteresis and
//! reserve a pending slot. The lock-free path reserves the slot first with
//! one atomic add and returns it when the attempt does not admit. A
//! concurrent attempt therefore always counts every earlier reservation, so
//! the hard limit holds without the lock.
//!
//! The one difference is at the hard-limit boundary: an attempt can also
//! count a concurrent attempt that is itself about to be rejected, so it may
//! be rejected (503) where the serialized path would have admitted it. It
//! never admits more. Sequential decisions are identical (see the tests).

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Limits of one server admission decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AdmissionLimits {
    pub(crate) hard: usize,
    /// `min(configured soft, hard)`, or `hard` without a soft limit.
    pub(crate) soft: usize,
    /// The configured soft limit, when a pacing delay is configured as well.
    pub(crate) pacing_threshold: Option<usize>,
}

/// One step of the admission loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AdmissionStep {
    /// A pending slot is held; the caller wraps it in its guard.
    Admit {
        observed_sessions: usize,
    },
    Reject {
        observed_sessions: usize,
    },
    /// Sleep for the pacing delay once, then decide again.
    Pace {
        observed_sessions: usize,
    },
}

/// The decision of the serialized admission loop body, given
/// `sessions + pending` observed by this attempt (excluding itself).
pub(crate) fn decide(
    observed_sessions: usize,
    limits: AdmissionLimits,
    overloaded: &AtomicBool,
    paced_once: bool,
) -> AdmissionStep {
    if overloaded.load(Ordering::Relaxed) {
        if observed_sessions < limits.soft {
            overloaded.store(false, Ordering::Relaxed);
        } else {
            return AdmissionStep::Reject { observed_sessions };
        }
    }
    if observed_sessions >= limits.hard {
        overloaded.store(true, Ordering::Relaxed);
        return AdmissionStep::Reject { observed_sessions };
    }
    if !paced_once {
        if let Some(threshold) = limits.pacing_threshold {
            if observed_sessions >= threshold {
                return AdmissionStep::Pace { observed_sessions };
            }
        }
    }
    AdmissionStep::Admit { observed_sessions }
}

/// Reserve, then decide. On `Admit` the caller owns one unit of `pending`
/// (released by its guard); on `Reject` and `Pace` the reservation has
/// already been returned.
pub(crate) fn reserve_and_decide(
    sessions: usize,
    pending: &AtomicUsize,
    overloaded: &AtomicBool,
    limits: AdmissionLimits,
    paced_once: bool,
) -> AdmissionStep {
    let earlier = pending.fetch_add(1, Ordering::AcqRel);
    let step = decide(
        sessions.saturating_add(earlier),
        limits,
        overloaded,
        paced_once,
    );
    if !matches!(step, AdmissionStep::Admit { .. }) {
        pending.fetch_sub(1, Ordering::AcqRel);
    }
    step
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Transliteration of the serialized loop body in
    /// `SessionCrossCrateEventHandler::acquire_server_call_admission`, which
    /// runs under the admission mutex: observe, decide, reserve on admit.
    fn serialized(
        sessions: usize,
        pending: &AtomicUsize,
        overloaded: &AtomicBool,
        limits: AdmissionLimits,
        paced_once: bool,
    ) -> AdmissionStep {
        let observed_sessions = sessions.saturating_add(pending.load(Ordering::Relaxed));
        if overloaded.load(Ordering::Relaxed) {
            if observed_sessions < limits.soft {
                overloaded.store(false, Ordering::Relaxed);
            } else {
                return AdmissionStep::Reject { observed_sessions };
            }
        }
        if observed_sessions >= limits.hard {
            overloaded.store(true, Ordering::Relaxed);
            return AdmissionStep::Reject { observed_sessions };
        }
        if !paced_once {
            if let Some(threshold) = limits.pacing_threshold {
                if observed_sessions >= threshold {
                    return AdmissionStep::Pace { observed_sessions };
                }
            }
        }
        pending.fetch_add(1, Ordering::Relaxed);
        AdmissionStep::Admit { observed_sessions }
    }

    fn limits(hard: usize, soft: Option<usize>, pacing: bool) -> AdmissionLimits {
        AdmissionLimits {
            hard,
            soft: soft.unwrap_or(hard).min(hard),
            pacing_threshold: if pacing { soft } else { None },
        }
    }

    #[test]
    fn hysteresis_clears_only_below_soft() {
        let limits = limits(4, Some(2), false);
        let (pending, overloaded) = (AtomicUsize::new(0), AtomicBool::new(true));
        let step = reserve_and_decide(2, &pending, &overloaded, limits, false);
        assert_eq!(
            step,
            AdmissionStep::Reject {
                observed_sessions: 2
            }
        );
        assert!(overloaded.load(Ordering::Relaxed));
        assert_eq!(pending.load(Ordering::Relaxed), 0);
        let step = reserve_and_decide(1, &pending, &overloaded, limits, false);
        assert_eq!(
            step,
            AdmissionStep::Admit {
                observed_sessions: 1
            }
        );
        assert!(!overloaded.load(Ordering::Relaxed));
        assert_eq!(pending.load(Ordering::Relaxed), 1);
    }

    /// Sequential attempts (no concurrency) decide identically on both
    /// paths, including the overload hysteresis and pacing, across a scripted
    /// sequence of session-store sizes and guard releases.
    #[test]
    fn sequential_decisions_match_the_serialized_path() {
        for limits in [
            limits(4, None, false),
            limits(6, Some(3), false),
            limits(6, Some(3), true),
            limits(1, Some(1), true),
        ] {
            let (old_pending, old_overloaded) = (AtomicUsize::new(0), AtomicBool::new(false));
            let (new_pending, new_overloaded) = (AtomicUsize::new(0), AtomicBool::new(false));
            // (sessions in the store, admitted guards released before this
            // attempt, paced_once)
            let script = [
                (0, 0, false),
                (0, 0, false),
                (1, 1, false),
                (1, 0, false),
                (2, 0, true),
                (3, 0, false),
                (4, 0, false),
                (5, 2, false),
                (5, 0, true),
                (2, 3, false),
                (1, 0, false),
                (0, 0, false),
                (0, 0, true),
                (6, 0, false),
                (2, 0, false),
            ];
            for (step, (sessions, released, paced_once)) in script.into_iter().enumerate() {
                for _ in 0..released {
                    if old_pending.load(Ordering::Relaxed) > 0 {
                        old_pending.fetch_sub(1, Ordering::Relaxed);
                        new_pending.fetch_sub(1, Ordering::Relaxed);
                    }
                }
                let old = serialized(sessions, &old_pending, &old_overloaded, limits, paced_once);
                let new =
                    reserve_and_decide(sessions, &new_pending, &new_overloaded, limits, paced_once);
                assert_eq!(old, new, "step {step} under {limits:?}");
                assert_eq!(
                    old_pending.load(Ordering::Relaxed),
                    new_pending.load(Ordering::Relaxed),
                    "pending after step {step}"
                );
                assert_eq!(
                    old_overloaded.load(Ordering::Relaxed),
                    new_overloaded.load(Ordering::Relaxed),
                    "hysteresis after step {step}"
                );
            }
        }
    }

    /// Concurrent attempts never admit past the hard limit, and every
    /// non-admitting attempt returns its reservation.
    #[test]
    fn concurrent_reservations_never_exceed_the_hard_limit() {
        let hard = 16;
        let pending = Arc::new(AtomicUsize::new(0));
        let overloaded = Arc::new(AtomicBool::new(false));
        let admitted = Arc::new(AtomicUsize::new(0));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let (pending, overloaded, admitted) = (
                    Arc::clone(&pending),
                    Arc::clone(&overloaded),
                    Arc::clone(&admitted),
                );
                std::thread::spawn(move || {
                    for _ in 0..200 {
                        if let AdmissionStep::Admit { .. } = reserve_and_decide(
                            0,
                            &pending,
                            &overloaded,
                            limits(hard, None, false),
                            false,
                        ) {
                            admitted.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        // No guard is ever released here, so admissions are capped and every
        // rejected reservation was returned.
        let concurrent = admitted.load(Ordering::Relaxed);
        assert!(
            concurrent <= hard,
            "admitted {concurrent} past hard limit {hard}"
        );
        assert_eq!(pending.load(Ordering::Relaxed), concurrent);
        // Uncontended attempts then fill exactly the remaining slots (the
        // hysteresis clears once a guard-free attempt observes fewer
        // sessions than the soft limit, which equals the hard limit here).
        let mut sequential = 0;
        while let AdmissionStep::Admit { .. } =
            reserve_and_decide(0, &pending, &overloaded, limits(hard, None, false), false)
        {
            sequential += 1;
            assert!(sequential <= hard);
        }
        assert_eq!(concurrent + sequential, hard);
        assert_eq!(pending.load(Ordering::Relaxed), hard);
    }
}
