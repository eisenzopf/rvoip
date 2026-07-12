//! Common lifetime supervisor for one substrate peer.
//!
//! QUIC, WebTransport, and WebSocket each run an inbound pump, outbound pump,
//! and coordinator-event translator. The first pump exit is terminal for that
//! peer: the coordinator is drained and every sibling task is aborted so
//! routes, channels, and capacity permits cannot remain orphaned.

use std::sync::Arc;
use std::time::Duration;

use futures::future::select_all;
use rvoip_core::adapter::{AdapterEvent, OrchestratorAdapterEvent};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::UctpCoordinator;

/// Nonblocking adapter-event delivery policy for peer event pumps. Quality
/// snapshots and native diagnostics are lossy under pressure; lifecycle,
/// authentication, control, and application data are critical and force peer
/// teardown when the consumer cannot accept them immediately.
pub fn try_deliver_adapter_event(
    sender: &mpsc::Sender<AdapterEvent>,
    event: AdapterEvent,
    transport: &'static str,
) -> bool {
    let best_effort = matches!(
        &event,
        AdapterEvent::Quality { .. } | AdapterEvent::Native { .. }
    );
    match sender.try_send(event) {
        Ok(()) => true,
        Err(mpsc::error::TrySendError::Full(_)) if best_effort => {
            metrics::counter!(
                "uctp_adapter_events_dropped_total",
                "transport" => transport,
                "class" => "best-effort"
            )
            .increment(1);
            true
        }
        Err(mpsc::error::TrySendError::Full(_)) => {
            metrics::counter!(
                "uctp_adapter_event_backpressure_total",
                "transport" => transport
            )
            .increment(1);
            false
        }
        Err(mpsc::error::TrySendError::Closed(_)) => false,
    }
}

/// Atomic counterpart used by first-party adapters. Authenticated inbound
/// handoff remains a single queue item; public best-effort events retain the
/// same loss policy as [`try_deliver_adapter_event`].
pub fn try_deliver_orchestrator_event(
    sender: &mpsc::Sender<OrchestratorAdapterEvent>,
    event: OrchestratorAdapterEvent,
    transport: &'static str,
) -> bool {
    let best_effort = matches!(
        &event,
        OrchestratorAdapterEvent::Public(
            AdapterEvent::Quality { .. } | AdapterEvent::Native { .. }
        )
    );
    match sender.try_send(event) {
        Ok(()) => true,
        Err(mpsc::error::TrySendError::Full(_)) if best_effort => {
            metrics::counter!(
                "uctp_adapter_events_dropped_total",
                "transport" => transport,
                "class" => "best-effort"
            )
            .increment(1);
            true
        }
        Err(mpsc::error::TrySendError::Full(_)) => {
            metrics::counter!(
                "uctp_adapter_event_backpressure_total",
                "transport" => transport
            )
            .increment(1);
            false
        }
        Err(mpsc::error::TrySendError::Closed(_)) => false,
    }
}

/// Maximum time a newly established substrate peer may remain
/// unauthenticated before its whole session is torn down.
pub const DEFAULT_AUTHENTICATION_DEADLINE: Duration = Duration::from_secs(10);

/// Couple authentication lifetime to the peer supervisor. The task remains
/// pending while the retained principal is active, and exits when the initial
/// authentication deadline is missed or the principal expires. Because it is
/// supervised alongside the I/O pumps, either condition tears down signaling,
/// media, routes, and the admission permit together.
pub fn spawn_auth_lifecycle_guard(
    coordinator: Arc<UctpCoordinator>,
    authentication_deadline: Duration,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + authentication_deadline;
        let mut interval = tokio::time::interval(Duration::from_millis(250));
        loop {
            interval.tick().await;
            match coordinator.authenticated_principal() {
                Some(principal) if principal.is_expired() => return,
                Some(_) => {}
                None if tokio::time::Instant::now() >= deadline => return,
                None => {}
            }
        }
    })
}

/// Result of supervising a substrate peer's coupled tasks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PeerSessionExit {
    /// The first task ended because it panicked or was cancelled.
    pub first_task_failed: bool,
    /// Coordinator drain exceeded the configured grace period.
    pub drain_timed_out: bool,
}

/// Wait for the first coupled peer task to end, drain the coordinator, then
/// abort all siblings. `tasks` must contain only peer-lifetime tasks; periodic
/// samplers that may intentionally return early should be managed separately.
pub async fn supervise_peer_tasks(
    coordinator: Arc<UctpCoordinator>,
    tasks: Vec<JoinHandle<()>>,
    drain_grace: Duration,
) -> PeerSessionExit {
    supervise_peer_tasks_inner(coordinator, tasks, drain_grace, None).await
}

/// Variant that immediately cancels peer media when any supervised task exits,
/// while still allowing the coordinator its bounded signaling drain window.
pub async fn supervise_peer_tasks_with_media_cancel(
    coordinator: Arc<UctpCoordinator>,
    tasks: Vec<JoinHandle<()>>,
    drain_grace: Duration,
    media_cancel: CancellationToken,
) -> PeerSessionExit {
    supervise_peer_tasks_inner(coordinator, tasks, drain_grace, Some(media_cancel)).await
}

async fn supervise_peer_tasks_inner(
    coordinator: Arc<UctpCoordinator>,
    mut tasks: Vec<JoinHandle<()>>,
    drain_grace: Duration,
    media_cancel: Option<CancellationToken>,
) -> PeerSessionExit {
    assert!(
        !tasks.is_empty(),
        "peer supervisor requires at least one task"
    );
    let cancellation_coordinator = Arc::clone(&coordinator);
    tasks.push(tokio::spawn(async move {
        cancellation_coordinator.cancelled().await;
    }));
    let (first_result, _, remaining) = select_all(tasks).await;
    let first_task_failed = first_result.is_err();
    if let Some(media_cancel) = media_cancel {
        media_cancel.cancel();
    }
    let drain_timed_out = tokio::time::timeout(drain_grace, coordinator.shutdown())
        .await
        .is_err();
    if drain_timed_out {
        coordinator.abort().await;
    }
    for task in &remaining {
        task.abort();
    }
    let _ = tokio::time::timeout(drain_grace, futures::future::join_all(remaining)).await;
    metrics::counter!(
        "uctp_peer_sessions_ended_total",
        "first_task_failed" => first_task_failed.to_string(),
        "drain_timed_out" => drain_timed_out.to_string()
    )
    .increment(1);
    PeerSessionExit {
        first_task_failed,
        drain_timed_out,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ENVELOPE_CHANNEL_CAP;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn first_task_exit_drains_coordinator_and_aborts_sibling() {
        let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
        let (out_tx, _out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
        let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
        let coordinator = UctpCoordinator::start(
            "test",
            in_rx,
            out_tx,
            events_tx,
            rvoip_auth_core::bearer_stub(),
        );
        let first = tokio::spawn(async {});
        let sibling = tokio::spawn(async { futures::future::pending::<()>().await });
        let result = supervise_peer_tasks(
            Arc::clone(&coordinator),
            vec![first, sibling],
            Duration::from_secs(1),
        )
        .await;
        drop(in_tx);
        assert_eq!(
            result,
            PeerSessionExit {
                first_task_failed: false,
                drain_timed_out: false,
            }
        );
        assert!(coordinator.authenticated_principal().is_none());
    }

    #[tokio::test]
    async fn unauthenticated_guard_exits_at_deadline() {
        let (_in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
        let (out_tx, _out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
        let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
        let coordinator = UctpCoordinator::start(
            "test",
            in_rx,
            out_tx,
            events_tx,
            rvoip_auth_core::bearer_stub(),
        );
        tokio::time::timeout(
            Duration::from_secs(1),
            spawn_auth_lifecycle_guard(coordinator, Duration::from_millis(20)),
        )
        .await
        .expect("unauthenticated lifecycle guard should expire")
        .expect("guard task should not panic");
    }

    #[tokio::test]
    async fn coordinator_cancellation_wakes_blocked_peer_tasks() {
        let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
        let (out_tx, _out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
        let (events_tx, _events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
        let coordinator = UctpCoordinator::start(
            "test",
            in_rx,
            out_tx,
            events_tx,
            rvoip_auth_core::bearer_stub(),
        );
        let supervisor_coordinator = Arc::clone(&coordinator);
        let supervisor = tokio::spawn(async move {
            supervise_peer_tasks(
                supervisor_coordinator,
                vec![tokio::spawn(futures::future::pending())],
                Duration::from_secs(1),
            )
            .await
        });

        coordinator.abort().await;
        drop(in_tx);
        tokio::time::timeout(Duration::from_secs(1), supervisor)
            .await
            .expect("cancellation must wake the peer supervisor")
            .expect("supervisor task should not panic");
    }
}
