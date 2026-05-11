//! UAS reliable-provisional retransmission (RFC 3262 §3).
//!
//! When the UAS sends a reliable provisional (typically 183 Session Progress
//! with SDP), the response must be retransmitted with T1 backoff until the
//! UAC acknowledges it with PRACK (§7.2) or 64·T1 elapses. This module
//! spawns a tokio task per outstanding reliable provisional; the task is
//! registered on `DialogManager::reliable_provisional_tasks` so the PRACK
//! handler can abort it when the acknowledgment arrives, and the dialog
//! cleanup path can abort any survivors.
//!
//! Retransmits re-enter `TransactionManager::send_response` which sends the
//! same 18x bytes through the transport each tick. This preserves the
//! original Via/To/From/CSeq/RSeq — we do not mutate the cached response.

use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use tokio::task::AbortHandle;
use tracing::{debug, warn};

use crate::dialog::DialogId;
use crate::transaction::{TransactionKey, TransactionManager};
use rvoip_sip_core::Response;

/// RFC 3261 T1 base interval — 500 ms.
pub const T1: Duration = Duration::from_millis(500);
/// RFC 3261 T2 cap on retransmit interval — 4 s.
pub const T2: Duration = Duration::from_secs(4);
/// RFC 3262 §3 abandon window — 64·T1 = 32 s.
pub const ABANDON_WINDOW: Duration = Duration::from_millis(32_000);

/// Spawn a tokio task that retransmits `response` through
/// `transaction_manager` until PRACK arrives (task aborted externally) or
/// 64·T1 has elapsed. Registers its `AbortHandle` in the shared map keyed by
/// `(dialog_id, rseq)` so the PRACK handler can cancel it.
pub fn spawn_reliable_provisional_retransmit(
    dialog_id: DialogId,
    rseq: u32,
    transaction_id: TransactionKey,
    response: Response,
    transaction_manager: Arc<TransactionManager>,
    tracker: Arc<DashMap<(DialogId, u32), AbortHandle>>,
) {
    let key = (dialog_id.clone(), rseq);
    let cleanup_tracker = tracker.clone();
    let cleanup_key = key.clone();

    let handle = tokio::spawn(async move {
        let start = Instant::now();
        let mut interval = T1;

        loop {
            tokio::time::sleep(interval).await;
            if start.elapsed() >= ABANDON_WINDOW {
                warn!(
                    "Reliable 18x (dialog={}, rseq={}) unacknowledged after 64·T1 — abandoning retransmits",
                    dialog_id, rseq
                );
                break;
            }

            match transaction_manager
                .send_response(&transaction_id, response.clone())
                .await
            {
                Ok(_) => {
                    debug!(
                        "Retransmitted reliable 18x (dialog={}, rseq={}) after {:?}",
                        dialog_id, rseq, interval
                    );
                }
                Err(e) => {
                    warn!(
                        "Retransmit of reliable 18x (dialog={}, rseq={}) failed: {} — stopping",
                        dialog_id, rseq, e
                    );
                    break;
                }
            }

            interval = (interval * 2).min(T2);
        }

        cleanup_tracker.remove(&cleanup_key);
    });

    tracker.insert(key, handle.abort_handle());
}
