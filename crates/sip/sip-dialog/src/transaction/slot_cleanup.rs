//! Accounting for the transaction manager's slot-cleanup decisions.
//!
//! Under a cleanup burst, one explicit termination can wait on a saturated
//! transaction-user (TU) queue for its terminal events. Closing admission at
//! that point refuses every later inbound request and every local BYE, so
//! ended sessions can no longer release their slots. Each decision that
//! replaces that behaviour is counted per manager, so an application can
//! reconcile a run from [`TransactionManager::slot_cleanup_stats`] alone.
//!
//! [`TransactionManager::slot_cleanup_stats`]:
//!     crate::transaction::TransactionManager::slot_cleanup_stats

use std::sync::atomic::{AtomicU64, Ordering};

/// Snapshot of a transaction manager's slot-cleanup counters.
///
/// Counters are monotonic except
/// [`terminal_publications_detached`](Self::terminal_publications_detached),
/// which is a gauge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SlotCleanupStats {
    /// Explicit-termination terminal publications that outlived the inline
    /// wait and continued in the background instead of closing admission.
    pub terminal_publication_handoffs: u64,
    /// Handed-off publications still waiting for TU queue capacity (gauge).
    pub terminal_publications_detached: u64,
    /// Times the manager closed new-transaction admission because a terminal
    /// event could not reach the TU at all (its receiver was closed).
    pub admission_fail_closed: u64,
    /// New dialog-forming INVITEs answered with a stateless 503 because the
    /// TU reported its session slots full.
    pub new_invite_capacity_rejections: u64,
    /// New dialog-forming INVITEs answered with a stateless 503 while the
    /// manager was draining, instead of being dropped unanswered.
    pub draining_invite_rejections: u64,
}

#[derive(Debug, Default)]
pub(crate) struct SlotCleanupCounters {
    terminal_publication_handoffs: AtomicU64,
    terminal_publications_detached: AtomicU64,
    admission_fail_closed: AtomicU64,
    new_invite_capacity_rejections: AtomicU64,
    draining_invite_rejections: AtomicU64,
}

impl SlotCleanupCounters {
    /// Count one hand-off. Returns the running count when this occurrence
    /// should be logged.
    pub(crate) fn record_terminal_publication_handoff(&self) -> Option<u64> {
        self.terminal_publications_detached
            .fetch_add(1, Ordering::Relaxed);
        crate::diagnostics::thin_log(&self.terminal_publication_handoffs)
    }

    pub(crate) fn record_detached_terminal_publication_finished(&self) {
        let _ = self.terminal_publications_detached.fetch_update(
            Ordering::Relaxed,
            Ordering::Relaxed,
            |detached| Some(detached.saturating_sub(1)),
        );
    }

    pub(crate) fn terminal_publications_detached(&self) -> u64 {
        self.terminal_publications_detached.load(Ordering::Relaxed)
    }

    pub(crate) fn record_admission_fail_closed(&self) {
        self.admission_fail_closed.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_new_invite_capacity_rejection(&self) {
        self.new_invite_capacity_rejections
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_draining_invite_rejection(&self) {
        self.draining_invite_rejections
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn snapshot(&self) -> SlotCleanupStats {
        SlotCleanupStats {
            terminal_publication_handoffs: self
                .terminal_publication_handoffs
                .load(Ordering::Relaxed),
            terminal_publications_detached: self.terminal_publications_detached(),
            admission_fail_closed: self.admission_fail_closed.load(Ordering::Relaxed),
            new_invite_capacity_rejections: self
                .new_invite_capacity_rejections
                .load(Ordering::Relaxed),
            draining_invite_rejections: self.draining_invite_rejections.load(Ordering::Relaxed),
        }
    }
}
