//! RFC 4028 session-timer refresh scheduling.
//!
//! When a session timer is negotiated on a dialog, the refresher MUST
//! re-send an UPDATE (or re-INVITE, §9) at `interval / 2` to keep the
//! session from being torn down. If a refresh fails — a 4xx/5xx/6xx
//! response, timeout, or UPDATE returning 501 — the dialog is terminated
//! with `BYE + Reason: SIP ;cause=408;text="Session expired"` per §10.
//!
//! This module owns the per-dialog refresh task. Cancellation is via the
//! `AbortHandle` stored on `DialogManager::session_refresh_tasks`.

use std::sync::Arc;
use std::time::Duration;

use tokio::task::AbortHandle;
use tracing::{debug, info, warn};

use rvoip_sip_core::Method;

use crate::dialog::DialogId;
use crate::events::SessionCoordinationEvent;
use crate::manager::{core::DialogManager, SessionCoordinator};

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

    let handle = tokio::spawn(async move {
        loop {
            tokio::time::sleep(half).await;

            // Try UPDATE first (lighter weight, RFC 4028 §9 preferred).
            let update_result = manager
                .send_request(&dialog_for_task, Method::Update, None)
                .await;

            let refresh_ok = match update_result {
                Ok(_) => {
                    info!("Session refresh (UPDATE) sent for dialog {}", dialog_for_task);
                    true
                }
                Err(e) => {
                    warn!(
                        "Session refresh UPDATE failed for dialog {}: {} — retrying as re-INVITE",
                        dialog_for_task, e
                    );
                    match manager
                        .send_request(&dialog_for_task, Method::Invite, None)
                        .await
                    {
                        Ok(_) => {
                            info!("Session refresh (re-INVITE) sent for dialog {}", dialog_for_task);
                            true
                        }
                        Err(e2) => {
                            warn!(
                                "Session refresh re-INVITE also failed for dialog {}: {} — tearing down with 408",
                                dialog_for_task, e2
                            );
                            false
                        }
                    }
                }
            };

            if !refresh_ok {
                // Tear down the dialog per RFC 4028 §10. We send a BYE; the
                // Reason header isn't carried by the current `send_bye`
                // helper — the 408 cause is noted in logs and the session
                // layer is notified via SessionRefreshFailed so apps see
                // the distinct reason. Producing a Reason: header on the
                // BYE is a nice-to-have left for follow-on work.
                let _ = manager
                    .send_request(&dialog_for_task, Method::Bye, None)
                    .await;
                let _ = manager
                    .notify_session_layer(SessionCoordinationEvent::SessionRefreshFailed {
                        dialog_id: dialog_for_task.clone(),
                        reason: "Session expired (RFC 4028 §10 — cause=408)".to_string(),
                    })
                    .await;
                break;
            }

            let _ = manager
                .notify_session_layer(SessionCoordinationEvent::SessionRefreshed {
                    dialog_id: dialog_for_task.clone(),
                    expires_secs: interval_secs,
                })
                .await;
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
