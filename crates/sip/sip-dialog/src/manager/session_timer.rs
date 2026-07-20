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
//! `AbortHandle` stored on `DialogManager::session_refresh_tasks`; exact
//! completion waits are cancel-safe, so abort is clean wherever the task
//! happens to be awaiting.

use std::collections::HashSet;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use dashmap::DashMap;
use tokio::sync::{oneshot, watch, Mutex};
use tokio::task::AbortHandle;
use tracing::{debug, info, warn};

use rvoip_sip_core::types::reason::Reason;
use rvoip_sip_core::Method;

use crate::dialog::{DialogId, DialogState};
use crate::events::SessionCoordinationEvent;
use crate::manager::{core::DialogManager, SessionCoordinator};
use crate::transaction::{
    ClientTransactionFailure, ClientTransactionOutcome, TransactionKey, TransactionManager,
};

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

const REFRESH_TASK_COMPLETION_TIMEOUT: Duration = Duration::from_secs(1);
static NEXT_REFRESH_TASK_TOKEN: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
struct SessionRefreshTask {
    token: u64,
    abort: AbortHandle,
    completion: watch::Receiver<bool>,
}

#[derive(Debug)]
struct SessionRefreshAdmission {
    accepting: bool,
    closed_dialogs: HashSet<DialogId>,
}

#[derive(Debug)]
pub(crate) struct SessionRefreshTaskRegistry {
    tasks: DashMap<DialogId, Arc<SessionRefreshTask>>,
    operation_gate: Mutex<()>,
    admission: StdMutex<SessionRefreshAdmission>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionRefreshTaskError {
    RegistryClosed,
    CompletionTimeout,
}

impl fmt::Display for SessionRefreshTaskError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RegistryClosed => formatter.write_str("session refresh registry is closed"),
            Self::CompletionTimeout => {
                formatter.write_str("session refresh task did not complete after cancellation")
            }
        }
    }
}

impl SessionRefreshTaskRegistry {
    pub(crate) fn with_capacity(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            tasks: DashMap::with_capacity(capacity),
            operation_gate: Mutex::new(()),
            admission: StdMutex::new(SessionRefreshAdmission {
                accepting: true,
                closed_dialogs: HashSet::new(),
            }),
        })
    }

    pub(crate) fn len(&self) -> usize {
        self.tasks.len()
    }

    pub(crate) fn has_task(&self, dialog_id: &DialogId) -> bool {
        self.tasks.contains_key(dialog_id)
    }

    pub(crate) fn fence_dialog(&self, dialog_id: &DialogId) {
        self.admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .closed_dialogs
            .insert(dialog_id.clone());
    }

    pub(crate) fn begin_close_dialog(&self, dialog_id: &DialogId) {
        self.fence_dialog(dialog_id);
        if let Some(task) = self.tasks.get(dialog_id) {
            task.abort.abort();
        }
    }

    fn remove_exact(&self, dialog_id: &DialogId, token: u64) {
        self.tasks
            .remove_if(dialog_id, |_, current| current.token == token);
    }

    async fn cancel_record(
        &self,
        dialog_id: &DialogId,
        task: &Arc<SessionRefreshTask>,
    ) -> Result<(), SessionRefreshTaskError> {
        task.abort.abort();
        if wait_for_refresh_task_completion(task).await {
            self.remove_exact(dialog_id, task.token);
            Ok(())
        } else {
            Err(SessionRefreshTaskError::CompletionTimeout)
        }
    }

    pub(crate) async fn cancel_dialog(
        &self,
        dialog_id: &DialogId,
    ) -> Result<(), SessionRefreshTaskError> {
        let _operation = self.operation_gate.lock().await;
        self.admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .closed_dialogs
            .insert(dialog_id.clone());
        if let Some(task) = self
            .tasks
            .get(dialog_id)
            .map(|entry| Arc::clone(entry.value()))
        {
            self.cancel_record(dialog_id, &task).await?;
        }
        Ok(())
    }

    pub(crate) fn release_dialog(&self, dialog_id: &DialogId) {
        let mut admission = self
            .admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.tasks.contains_key(dialog_id) {
            admission.closed_dialogs.remove(dialog_id);
        }
    }

    pub(crate) async fn close_all(&self) -> Result<(), SessionRefreshTaskError> {
        let _operation = self.operation_gate.lock().await;
        self.admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .accepting = false;
        let records = self
            .tasks
            .iter()
            .map(|entry| (entry.key().clone(), Arc::clone(entry.value())))
            .collect::<Vec<_>>();
        for (_, task) in &records {
            task.abort.abort();
        }
        let deadline = tokio::time::Instant::now() + REFRESH_TASK_COMPLETION_TIMEOUT;
        let mut incomplete = false;
        for (dialog_id, task) in records {
            if wait_for_refresh_task_completion_until(&task, deadline).await {
                self.remove_exact(&dialog_id, task.token);
            } else {
                incomplete = true;
            }
        }
        if incomplete {
            Err(SessionRefreshTaskError::CompletionTimeout)
        } else {
            Ok(())
        }
    }
}

struct SessionRefreshTaskCompletion {
    registry: Arc<SessionRefreshTaskRegistry>,
    dialog_id: DialogId,
    token: u64,
    completion: Option<watch::Sender<bool>>,
}

impl Drop for SessionRefreshTaskCompletion {
    fn drop(&mut self) {
        if let Some(completion) = self.completion.take() {
            let _ = completion.send(true);
        }
        self.registry.remove_exact(&self.dialog_id, self.token);
    }
}

async fn wait_for_refresh_task_completion_until(
    task: &SessionRefreshTask,
    deadline: tokio::time::Instant,
) -> bool {
    let mut completion = task.completion.clone();
    if *completion.borrow() {
        return true;
    }
    match tokio::time::timeout_at(deadline, async {
        loop {
            if *completion.borrow() {
                return true;
            }
            if completion.changed().await.is_err() {
                return *completion.borrow();
            }
        }
    })
    .await
    {
        Ok(completed) => completed,
        Err(_) => false,
    }
}

async fn wait_for_refresh_task_completion(task: &SessionRefreshTask) -> bool {
    wait_for_refresh_task_completion_until(
        task,
        tokio::time::Instant::now() + REFRESH_TASK_COMPLETION_TIMEOUT,
    )
    .await
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

/// Wait on the transaction's exact completion authority. This closes both the
/// response-before-wait and response-versus-removal races without allocating
/// a global observational subscription.
async fn await_tx_outcome(
    tx_mgr: &Arc<TransactionManager>,
    key: &TransactionKey,
    deadline: Duration,
) -> RefreshOutcome {
    match tx_mgr
        .wait_for_client_transaction_outcome(key, deadline)
        .await
    {
        Ok(Some(ClientTransactionOutcome::FinalResponse(response))) => {
            let code = response.status().as_u16();
            if (200..300).contains(&code) {
                RefreshOutcome::Success
            } else {
                RefreshOutcome::FailureStatus(code)
            }
        }
        Ok(Some(ClientTransactionOutcome::Failure(ClientTransactionFailure::Timeout)))
        | Ok(None) => RefreshOutcome::Timeout,
        Ok(Some(ClientTransactionOutcome::Failure(ClientTransactionFailure::Transport))) => {
            RefreshOutcome::Transport
        }
        Ok(Some(ClientTransactionOutcome::Failure(
            ClientTransactionFailure::Internal
            | ClientTransactionFailure::Cancelled
            | ClientTransactionFailure::Terminated,
        )))
        | Err(_) => RefreshOutcome::Terminated,
    }
}

/// Spawn a session-timer refresh task for the dialog. Replaces any
/// previously-spawned task for the same dialog (aborting it first).
///
/// `interval_secs` is the negotiated `Session-Expires` value; the refresh
/// fires at half that. `is_refresher` indicates whether *this* side is
/// responsible for sending the refresh — if `false`, no task is spawned
/// (the peer refreshes for us, and mid-dialog re-INVITE/UPDATE reception
/// already updates the dialog's liveness timer via the normal handlers).
pub async fn spawn_refresh_task(
    manager: DialogManager,
    dialog_id: DialogId,
    interval_secs: u32,
    is_refresher: bool,
) -> Result<(), SessionRefreshTaskError> {
    let registry = Arc::clone(&manager.session_refresh_tasks);
    let _operation = registry.operation_gate.lock().await;
    {
        let admission = registry
            .admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !admission.accepting
            || admission.closed_dialogs.contains(&dialog_id)
            || !manager.is_accepting_work()
        {
            return Err(SessionRefreshTaskError::RegistryClosed);
        }
    }

    // Replacement is serialized with cancellation and stop. The prior exact
    // generation must complete before the new task is registered.
    if let Some(prior) = registry
        .tasks
        .get(&dialog_id)
        .map(|entry| Arc::clone(entry.value()))
    {
        registry.cancel_record(&dialog_id, &prior).await?;
    }

    if !is_refresher {
        debug!(
            "Session timer active on dialog {} but peer is the refresher — no task needed",
            dialog_id
        );
        return Ok(());
    }
    if interval_secs == 0 {
        return Ok(());
    }

    let half = Duration::from_secs(interval_secs as u64 / 2);
    let dialog_for_task = dialog_id.clone();
    let task_registry = Arc::clone(&registry);

    // Budget for awaiting the refresh transaction's final outcome. Timer F
    // drives the transaction timeout; add a small slack so the subscription
    // path definitely sees the TransactionTimeout event before we bail.
    let tx_deadline = manager
        .transaction_manager
        .timer_settings()
        .transaction_timeout
        + Duration::from_secs(2);

    // Revalidate at the exact insertion boundary. Synchronous terminal
    // cleanup fences admission before removing storage, so it either sees
    // this record or this reservation observes the fence/removed dialog.
    let final_admission = registry
        .admission
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dialog_refreshable = manager
        .get_dialog(&dialog_id)
        .map(|dialog| dialog.state != DialogState::Terminated)
        .unwrap_or(false);
    if !final_admission.accepting
        || final_admission.closed_dialogs.contains(&dialog_id)
        || !manager.is_accepting_work()
        || !dialog_refreshable
    {
        return Err(SessionRefreshTaskError::RegistryClosed);
    }

    let token = NEXT_REFRESH_TASK_TOKEN.fetch_add(1, Ordering::Relaxed);
    let (start_tx, start_rx) = oneshot::channel();
    let (completion_tx, completion_rx) = watch::channel(false);
    let completion_guard = SessionRefreshTaskCompletion {
        registry: Arc::clone(&task_registry),
        dialog_id: dialog_for_task.clone(),
        token,
        completion: Some(completion_tx),
    };
    let handle = tokio::spawn(async move {
        let _completion = completion_guard;
        if start_rx.await.is_err() {
            return;
        }
        loop {
            tokio::time::sleep(half).await;
            if !manager.is_accepting_work() {
                break;
            }

            // Attempt 1: UPDATE (RFC 4028 §9 preferred for refresh).
            let update_outcome = match manager
                .send_request(&dialog_for_task, Method::Update, None)
                .await
            {
                Ok(key) => {
                    info!(
                        "Session refresh UPDATE sent for dialog {}; awaiting response",
                        dialog_for_task
                    );
                    await_tx_outcome(&manager.transaction_manager, &key, tx_deadline).await
                }
                Err(_error) => {
                    warn!(
                        "Session refresh UPDATE send failed for dialog {}",
                        dialog_for_task
                    );
                    RefreshOutcome::Transport
                }
            };

            if matches!(update_outcome, RefreshOutcome::Success) {
                if !manager.is_accepting_work() {
                    break;
                }
                let _ = manager
                    .notify_session_layer(SessionCoordinationEvent::SessionRefreshed {
                        dialog_id: dialog_for_task.clone(),
                        expires_secs: interval_secs,
                    })
                    .await;
                continue;
            }

            warn!(
                "Session refresh UPDATE failed for dialog {}; falling back to re-INVITE",
                dialog_for_task
            );

            // Attempt 2: re-INVITE fallback.
            if !manager.is_accepting_work() {
                break;
            }
            let invite_outcome = match manager
                .send_request(&dialog_for_task, Method::Invite, None)
                .await
            {
                Ok(key) => {
                    info!(
                        "Session refresh re-INVITE sent for dialog {}; awaiting response",
                        dialog_for_task
                    );
                    await_tx_outcome(&manager.transaction_manager, &key, tx_deadline).await
                }
                Err(_error) => {
                    warn!(
                        "Session refresh re-INVITE send failed for dialog {}",
                        dialog_for_task
                    );
                    RefreshOutcome::Transport
                }
            };

            if matches!(invite_outcome, RefreshOutcome::Success) {
                if !manager.is_accepting_work() {
                    break;
                }
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
                "Session refresh failed for dialog {}; sending BYE with Reason",
                dialog_for_task
            );

            if !manager.is_accepting_work() {
                break;
            }
            let reason = Reason::new("SIP", 408, Some("Session expired"));
            if let Err(_error) = manager.send_bye_with_reason(&dialog_for_task, reason).await {
                warn!(
                    "Failed to send BYE-with-Reason for dialog {}",
                    dialog_for_task
                );
            }

            if manager.is_accepting_work() {
                let _ = manager
                    .notify_session_layer(SessionCoordinationEvent::SessionRefreshFailed {
                        dialog_id: dialog_for_task.clone(),
                        reason: format!(
                            "Session expired (RFC 4028 §10 — cause=408): {}",
                            failure_reason
                        ),
                    })
                    .await;
            }
            break;
        }
    });
    let task = Arc::new(SessionRefreshTask {
        token,
        abort: handle.abort_handle(),
        completion: completion_rx,
    });
    drop(handle);
    registry.tasks.insert(dialog_id, task);
    drop(final_admission);
    if start_tx.send(()).is_err() {
        return Err(SessionRefreshTaskError::CompletionTimeout);
    }
    Ok(())
}

/// Abort the refresh task (if any) for a dialog — called from the dialog
/// cleanup path when the dialog terminates via BYE or any other reason.
pub async fn cancel_refresh_task(
    manager: &DialogManager,
    dialog_id: &DialogId,
) -> Result<(), SessionRefreshTaskError> {
    manager
        .session_refresh_tasks
        .cancel_dialog(dialog_id)
        .await?;
    debug!("Cancelled session refresh task for dialog {}", dialog_id);
    Ok(())
}

/// Wrapper taking an `Arc<DialogManager>` for call sites that only have a
/// shared reference.
pub async fn spawn_refresh_task_for(
    manager: Arc<DialogManager>,
    dialog_id: DialogId,
    interval_secs: u32,
    is_refresher: bool,
) -> Result<(), SessionRefreshTaskError> {
    spawn_refresh_task((*manager).clone(), dialog_id, interval_secs, is_refresher).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
    use rvoip_sip_core::prelude::{Message, Request};
    use rvoip_sip_core::StatusCode;
    use rvoip_sip_transport::transport::TransportType;
    use rvoip_sip_transport::{Transport, TransportEvent};
    use tokio::sync::mpsc;

    #[derive(Debug)]
    struct TestTransport {
        local_addr: SocketAddr,
    }

    fn install_pending_refresh(
        registry: Arc<SessionRefreshTaskRegistry>,
        dialog_id: DialogId,
        token: u64,
    ) -> Arc<SessionRefreshTask> {
        let (completion_tx, completion_rx) = watch::channel(false);
        let completion_guard = SessionRefreshTaskCompletion {
            registry: Arc::clone(&registry),
            dialog_id: dialog_id.clone(),
            token,
            completion: Some(completion_tx),
        };
        let handle = tokio::spawn(async move {
            let _completion = completion_guard;
            std::future::pending::<()>().await;
        });
        let task = Arc::new(SessionRefreshTask {
            token,
            abort: handle.abort_handle(),
            completion: completion_rx,
        });
        drop(handle);
        registry.tasks.insert(dialog_id, Arc::clone(&task));
        task
    }

    #[tokio::test]
    async fn terminal_cancel_fences_and_joins_never_polled_refresh() {
        let registry = SessionRefreshTaskRegistry::with_capacity(2);
        let dialog_id = DialogId::new();
        install_pending_refresh(Arc::clone(&registry), dialog_id.clone(), 1);

        registry
            .cancel_dialog(&dialog_id)
            .await
            .expect("terminal cancellation must observe completion");
        assert_eq!(registry.len(), 0);
        assert!(registry
            .admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .closed_dialogs
            .contains(&dialog_id));
        registry.release_dialog(&dialog_id);
        assert!(!registry
            .admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .closed_dialogs
            .contains(&dialog_id));
    }

    #[tokio::test]
    async fn stale_refresh_completion_cannot_remove_replacement_and_stop_closes_admission() {
        let registry = SessionRefreshTaskRegistry::with_capacity(2);
        let dialog_id = DialogId::new();
        install_pending_refresh(Arc::clone(&registry), dialog_id.clone(), 2);
        let (completion_tx, _completion_rx) = watch::channel(false);
        drop(SessionRefreshTaskCompletion {
            registry: Arc::clone(&registry),
            dialog_id: dialog_id.clone(),
            token: 1,
            completion: Some(completion_tx),
        });
        assert!(registry.has_task(&dialog_id));

        registry.close_all().await.expect("stop drain");
        assert_eq!(registry.len(), 0);
        assert!(
            !registry
                .admission
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .accepting
        );
    }

    #[async_trait::async_trait]
    impl Transport for TestTransport {
        async fn send_message(
            &self,
            _message: Message,
            _destination: SocketAddr,
        ) -> std::result::Result<(), rvoip_sip_transport::Error> {
            Ok(())
        }

        fn local_addr(&self) -> std::result::Result<SocketAddr, rvoip_sip_transport::Error> {
            Ok(self.local_addr)
        }

        async fn close(&self) -> std::result::Result<(), rvoip_sip_transport::Error> {
            Ok(())
        }

        fn is_closed(&self) -> bool {
            false
        }
    }

    fn refresh_request() -> Request {
        SimpleRequestBuilder::new(Method::Update, "sip:bob@example.com")
            .expect("UPDATE builder")
            .from("Alice", "sip:alice@example.com", Some("alice-tag"))
            .to("Bob", "sip:bob@example.com", Some("bob-tag"))
            .call_id("session-timer-exact-outcome")
            .cseq(1)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK.session-timer-exact"))
            .max_forwards(70)
            .build()
    }

    #[tokio::test]
    async fn refresh_wait_uses_exact_completion_without_subscribers() {
        let transport = Arc::new(TestTransport {
            local_addr: "127.0.0.1:5060".parse().unwrap(),
        });
        let (_transport_tx, transport_rx) = mpsc::channel(8);
        let (manager, _primary_events) =
            TransactionManager::new(transport.clone(), transport_rx, Some(8))
                .await
                .expect("transaction manager");
        let manager = Arc::new(manager);
        let request = refresh_request();
        let peer: SocketAddr = "192.0.2.21:5060".parse().unwrap();
        let key = manager
            .create_client_transaction(request.clone(), peer)
            .await
            .expect("client transaction");
        manager.send_request(&key).await.expect("initial write");

        let response =
            SimpleResponseBuilder::response_from_request(&request, StatusCode::Ok, Some("OK"))
                .to("Bob", "sip:bob@example.com", Some("bob-tag"))
                .build();
        manager
            .handle_transport_event(TransportEvent::MessageReceived {
                message: Message::Response(response),
                source: peer,
                destination: transport.local_addr().unwrap(),
                transport_type: TransportType::Udp,
                flow_id: None,
                raw_bytes: None,
                timing: None,
                connection_metadata: None,
            })
            .await
            .expect("response dispatch");

        assert!(matches!(
            await_tx_outcome(&manager, &key, Duration::from_secs(1)).await,
            RefreshOutcome::Success
        ));
        assert_eq!(manager.retention_counts().event_subscribers, 0);
        manager.shutdown().await;
    }
}
