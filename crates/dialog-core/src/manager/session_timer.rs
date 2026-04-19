//! RFC 4028 session-timer refresh scheduling.
//!
//! When a session timer is negotiated on a dialog, the refresher MUST
//! re-send an UPDATE (or re-INVITE, §9) at `interval / 2` to keep the
//! session from being torn down. We **subscribe to the refresh
//! transaction's outcome** (success, failure, timeout, transport error) and
//! only fire `SessionRefreshed` on a 2xx. If the refresh fails (4xx/5xx/6xx
//! response, timeout, or UPDATE returning 501), we fall back to a re-INVITE
//! per §9; if that also fails, the dialog is terminated with
//! `BYE + Reason: SIP ;cause=408;text="Session expired"` per §10 and
//! `SessionRefreshFailed` is notified up to the session layer.
//!
//! This module owns the per-dialog refresh task. Cancellation is via the
//! `AbortHandle` stored on `DialogManager::session_refresh_tasks`; both
//! `rx.recv()` and `tokio::time::timeout` are cancel-safe, so abort is
//! clean wherever the task happens to be awaiting.

use std::sync::Arc;
use std::time::Duration;

use tracing::{debug, info, warn};

use rvoip_sip_core::Method;
use rvoip_sip_core::types::reason::Reason;

use crate::dialog::DialogId;
use crate::events::SessionCoordinationEvent;
use crate::manager::{core::DialogManager, SessionCoordinator};
use crate::transaction::{TransactionEvent, TransactionKey, TransactionManager};

/// Outcome of awaiting a session-timer refresh transaction. Anything other
/// than `Success` is treated as a refresh failure per RFC 4028 §10.
#[derive(Debug)]
enum RefreshOutcome {
    Success,
    FailureStatus(u16),
    Timeout,
    Transport,
    Terminated,
}

impl RefreshOutcome {
    fn describe(&self, method: Method) -> String {
        match self {
            RefreshOutcome::Success => format!("{} 2xx", method),
            RefreshOutcome::FailureStatus(code) => format!("{} returned {}", method, code),
            RefreshOutcome::Timeout => format!("{} timed out", method),
            RefreshOutcome::Transport => format!("{} transport error", method),
            RefreshOutcome::Terminated => format!("{} terminated without final response", method),
        }
    }
}

/// Subscribe to the given client transaction and wait up to `deadline` for
/// its final outcome. Handles the race where the transaction completes
/// between `send_request` returning and the subscription being created —
/// falls back to the transaction's cached last response in that case.
async fn await_tx_outcome(
    tx_mgr: &Arc<TransactionManager>,
    key: &TransactionKey,
    deadline: Duration,
) -> RefreshOutcome {
    // Subscribe FIRST so events fired between the `last_response` check and
    // the `recv` loop below aren't lost. Then peek at the cached last
    // response: for the common case where the peer answered between
    // `send_request` returning and us getting here, the 2xx is already
    // sitting in the transaction's state and the subscription would never
    // see a fresh `SuccessResponse` event because it was broadcast before
    // we existed. Without this peek the task would sit until the 34 s
    // deadline expires and then fall through to a re-INVITE retry.
    let mut rx = match tx_mgr.subscribe_to_transaction(key).await {
        Ok(rx) => rx,
        Err(_) => {
            return match tx_mgr.last_response(key).await {
                Ok(Some(resp)) if (resp.status().as_u16() / 100) == 2 => {
                    RefreshOutcome::Success
                }
                Ok(Some(resp)) => RefreshOutcome::FailureStatus(resp.status().as_u16()),
                _ => RefreshOutcome::Terminated,
            };
        }
    };

    if let Ok(Some(resp)) = tx_mgr.last_response(key).await {
        let code = resp.status().as_u16();
        if (200..300).contains(&code) {
            return RefreshOutcome::Success;
        }
        if code >= 300 {
            return RefreshOutcome::FailureStatus(code);
        }
        // 1xx provisional — keep waiting for a final response.
    }

    let waited = tokio::time::timeout(deadline, async {
        loop {
            match rx.recv().await {
                Some(TransactionEvent::SuccessResponse { .. }) => {
                    return RefreshOutcome::Success;
                }
                Some(TransactionEvent::FailureResponse { response, .. }) => {
                    return RefreshOutcome::FailureStatus(response.status().as_u16());
                }
                Some(TransactionEvent::TransactionTimeout { .. }) => {
                    return RefreshOutcome::Timeout;
                }
                Some(TransactionEvent::TransportError { .. }) => {
                    return RefreshOutcome::Transport;
                }
                Some(TransactionEvent::TransactionTerminated { .. }) => {
                    return RefreshOutcome::Terminated;
                }
                None => return RefreshOutcome::Terminated,
                _ => continue,
            }
        }
    })
    .await;

    waited.unwrap_or(RefreshOutcome::Timeout)
}

/// Spawn a session-timer refresh task for the dialog. Replaces any
/// previously-spawned task for the same dialog (aborting it first).
///
/// `interval_secs` is the negotiated `Session-Expires` value; the refresh
/// fires at half that. `is_refresher` indicates whether *this* side is
/// responsible for sending the refresh — if `false`, no task is spawned
/// (the peer refreshes for us, and mid-dialog re-INVITE/UPDATE reception
/// already updates the dialog's liveness timer via the normal handlers).
pub fn spawn_refresh_task(
    manager: DialogManager,
    dialog_id: DialogId,
    interval_secs: u32,
    is_refresher: bool,
) {
    if !is_refresher {
        debug!(
            "Session timer active on dialog {} but peer is the refresher — no task needed",
            dialog_id
        );
        return;
    }
    if interval_secs == 0 {
        return;
    }

    // Abort any prior task for the same dialog (e.g. on re-INVITE
    // renegotiation with a different interval).
    if let Some((_, prior)) = manager.session_refresh_tasks.remove(&dialog_id) {
        prior.abort();
    }

    let half = Duration::from_secs(interval_secs as u64 / 2);
    let dialog_for_task = dialog_id.clone();
    let tracker = manager.session_refresh_tasks.clone();
    let task_tracker = tracker.clone();
    let manager_for_insert = manager.clone();

    // Budget for awaiting the refresh transaction's final outcome. Timer F
    // drives the transaction timeout; add a small slack so the subscription
    // path definitely sees the TransactionTimeout event before we bail.
    let tx_deadline =
        manager.transaction_manager.timer_settings().transaction_timeout + Duration::from_secs(2);

    let handle = tokio::spawn(async move {
        loop {
            tokio::time::sleep(half).await;

            // Attempt 1: UPDATE (RFC 4028 §9 preferred for refresh).
            let update_outcome = match manager
                .send_request(&dialog_for_task, Method::Update, None)
                .await
            {
                Ok(key) => {
                    info!(
                        "Session refresh UPDATE sent for dialog {} (tx={}); awaiting response",
                        dialog_for_task, key
                    );
                    await_tx_outcome(&manager.transaction_manager, &key, tx_deadline).await
                }
                Err(e) => {
                    warn!(
                        "Session refresh UPDATE send failed for dialog {}: {}",
                        dialog_for_task, e
                    );
                    RefreshOutcome::Transport
                }
            };

            if matches!(update_outcome, RefreshOutcome::Success) {
                let _ = manager
                    .notify_session_layer(SessionCoordinationEvent::SessionRefreshed {
                        dialog_id: dialog_for_task.clone(),
                        expires_secs: interval_secs,
                    })
                    .await;
                continue;
            }

            warn!(
                "Session refresh UPDATE outcome for dialog {}: {} — falling back to re-INVITE",
                dialog_for_task,
                update_outcome.describe(Method::Update)
            );

            // Attempt 2: re-INVITE fallback.
            let invite_outcome = match manager
                .send_request(&dialog_for_task, Method::Invite, None)
                .await
            {
                Ok(key) => {
                    info!(
                        "Session refresh re-INVITE sent for dialog {} (tx={}); awaiting response",
                        dialog_for_task, key
                    );
                    await_tx_outcome(&manager.transaction_manager, &key, tx_deadline).await
                }
                Err(e) => {
                    warn!(
                        "Session refresh re-INVITE send failed for dialog {}: {}",
                        dialog_for_task, e
                    );
                    RefreshOutcome::Transport
                }
            };

            if matches!(invite_outcome, RefreshOutcome::Success) {
                let _ = manager
                    .notify_session_layer(SessionCoordinationEvent::SessionRefreshed {
                        dialog_id: dialog_for_task.clone(),
                        expires_secs: interval_secs,
                    })
                    .await;
                continue;
            }

            // Both paths failed — tear the dialog down per RFC 4028 §10.
            let failure_reason = format!(
                "{}; {}",
                update_outcome.describe(Method::Update),
                invite_outcome.describe(Method::Invite)
            );
            warn!(
                "Session refresh failed for dialog {}: {} — sending BYE with Reason",
                dialog_for_task, failure_reason
            );

            let reason = Reason::new("SIP", 408, Some("Session expired"));
            if let Err(e) = manager.send_bye_with_reason(&dialog_for_task, reason).await {
                warn!(
                    "Failed to send BYE-with-Reason for dialog {}: {}",
                    dialog_for_task, e
                );
            }

            let _ = manager
                .notify_session_layer(SessionCoordinationEvent::SessionRefreshFailed {
                    dialog_id: dialog_for_task.clone(),
                    reason: format!("Session expired (RFC 4028 §10 — cause=408): {}", failure_reason),
                })
                .await;
            break;
        }

        task_tracker.remove(&dialog_for_task);
    });

    manager_for_insert
        .session_refresh_tasks
        .insert(dialog_id, handle.abort_handle());
    let _ = tracker; // silence unused binding
}

/// Abort the refresh task (if any) for a dialog — called from the dialog
/// cleanup path when the dialog terminates via BYE or any other reason.
pub fn cancel_refresh_task(manager: &DialogManager, dialog_id: &DialogId) {
    if let Some((_, abort)) = manager.session_refresh_tasks.remove(dialog_id) {
        abort.abort();
        debug!("Cancelled session refresh task for dialog {}", dialog_id);
    }
}

/// Wrapper taking an `Arc<DialogManager>` for call sites that only have a
/// shared reference.
pub fn spawn_refresh_task_for(
    manager: Arc<DialogManager>,
    dialog_id: DialogId,
    interval_secs: u32,
    is_refresher: bool,
) {
    spawn_refresh_task((*manager).clone(), dialog_id, interval_secs, is_refresher);
}
