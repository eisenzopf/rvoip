//! UAS reliable-provisional retransmission (RFC 3262 §3).
//!
//! A reliable provisional is reserved in the registry before its initial wire
//! write.  The retransmit task remains behind a start barrier until that write
//! succeeds.  An exact final-response close therefore rejects new reservations
//! and drains both pending and active retransmit generations before the final
//! response is allowed onto the wire.

use std::collections::HashSet;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use tokio::sync::{oneshot, watch};
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
const TASK_COMPLETION_TIMEOUT: Duration = Duration::from_secs(1);
static NEXT_RELIABLE_TASK_TOKEN: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ReliableProvisionalKey {
    dialog_id: DialogId,
    transaction_id: TransactionKey,
    rseq: u32,
}

#[derive(Debug)]
struct ReliableProvisionalTask {
    token: u64,
    abort: AbortHandle,
    completion: watch::Receiver<bool>,
}

#[derive(Debug)]
struct ReliableRegistryAdmission {
    accepting: bool,
    closed_transactions: HashSet<(DialogId, TransactionKey)>,
    closed_dialogs: HashSet<DialogId>,
}

/// A lifecycle failure which must prevent a final response or shutdown from
/// claiming that reliable-provisional work has drained.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReliableProvisionalError {
    RegistryClosed,
    DuplicateReservation,
    TaskCompletionTimeout { incomplete: usize },
}

impl fmt::Display for ReliableProvisionalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RegistryClosed => formatter.write_str("reliable provisional phase is closed"),
            Self::DuplicateReservation => {
                formatter.write_str("reliable provisional reservation already exists")
            }
            Self::TaskCompletionTimeout { incomplete } => write!(
                formatter,
                "{incomplete} reliable provisional task(s) did not complete after cancellation"
            ),
        }
    }
}

/// Exact reliable-provisional lifecycle registry.
///
/// `admission` is deliberately a short synchronous critical section. It makes
/// reservation and exact transaction close linearizable without placing an
/// async lock on the retransmission hot path.
pub(crate) struct ReliableProvisionalRegistry {
    tasks: DashMap<ReliableProvisionalKey, Arc<ReliableProvisionalTask>>,
    admission: StdMutex<ReliableRegistryAdmission>,
}

impl fmt::Debug for ReliableProvisionalRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReliableProvisionalRegistry")
            .field("tasks", &self.tasks.len())
            .finish()
    }
}

impl ReliableProvisionalRegistry {
    pub(crate) fn with_capacity(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            tasks: DashMap::with_capacity(capacity),
            admission: StdMutex::new(ReliableRegistryAdmission {
                accepting: true,
                closed_transactions: HashSet::new(),
                closed_dialogs: HashSet::new(),
            }),
        })
    }

    pub(crate) fn len(&self) -> usize {
        self.tasks.len()
    }

    pub(crate) fn has_tasks_for_dialog(&self, dialog_id: &DialogId) -> bool {
        self.tasks
            .iter()
            .any(|entry| &entry.key().dialog_id == dialog_id)
    }

    pub(crate) fn fence_dialog(&self, dialog_id: &DialogId) {
        self.admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .closed_dialogs
            .insert(dialog_id.clone());
    }

    pub(crate) fn begin_close_dialog(&self, dialog_id: &DialogId) {
        let mut admission = self
            .admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        admission.closed_dialogs.insert(dialog_id.clone());
        let tasks = self
            .tasks
            .iter()
            .filter(|entry| &entry.key().dialog_id == dialog_id)
            .map(|entry| Arc::clone(entry.value()))
            .collect::<Vec<_>>();
        for task in tasks {
            task.abort.abort();
        }
    }

    fn remove_exact(
        &self,
        key: &ReliableProvisionalKey,
        token: u64,
    ) -> Option<Arc<ReliableProvisionalTask>> {
        self.tasks
            .remove_if(key, |_, current| current.token == token)
            .map(|(_, task)| task)
    }

    fn owns(&self, key: &ReliableProvisionalKey, token: u64) -> bool {
        self.tasks
            .get(key)
            .is_some_and(|current| current.token == token)
    }

    async fn abort_and_wait(
        &self,
        records: Vec<(ReliableProvisionalKey, Arc<ReliableProvisionalTask>)>,
    ) -> Result<(), ReliableProvisionalError> {
        for (_, task) in &records {
            task.abort.abort();
        }

        let deadline = tokio::time::Instant::now() + TASK_COMPLETION_TIMEOUT;
        let mut incomplete = 0usize;
        for (key, task) in records {
            if wait_for_task_completion_until(&task, deadline).await {
                self.remove_exact(&key, task.token);
            } else {
                // Keep the exact record and abort handle registered. A later
                // final-response or stop retry must be able to observe it and
                // must not falsely claim successful drain.
                incomplete += 1;
            }
        }

        if incomplete == 0 {
            Ok(())
        } else {
            Err(ReliableProvisionalError::TaskCompletionTimeout { incomplete })
        }
    }

    pub(crate) async fn cancel_prack(
        &self,
        dialog_id: &DialogId,
        rseq: u32,
    ) -> Result<bool, ReliableProvisionalError> {
        let records = self
            .tasks
            .iter()
            .filter(|entry| &entry.key().dialog_id == dialog_id && entry.key().rseq == rseq)
            .map(|entry| (entry.key().clone(), Arc::clone(entry.value())))
            .collect::<Vec<_>>();
        if records.is_empty() {
            return Ok(false);
        }
        self.abort_and_wait(records).await?;
        Ok(true)
    }

    /// Atomically close the exact INVITE transaction generation and drain all
    /// pending/active reliable provisionals belonging to it.
    pub(crate) async fn close_transaction(
        &self,
        dialog_id: &DialogId,
        transaction_id: &TransactionKey,
    ) -> Result<(), ReliableProvisionalError> {
        let records = {
            let mut admission = self
                .admission
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            admission
                .closed_transactions
                .insert((dialog_id.clone(), transaction_id.clone()));
            self.tasks
                .iter()
                .filter(|entry| {
                    &entry.key().dialog_id == dialog_id
                        && &entry.key().transaction_id == transaction_id
                })
                .map(|entry| (entry.key().clone(), Arc::clone(entry.value())))
                .collect::<Vec<_>>()
        };
        self.abort_and_wait(records).await
    }

    /// Release only the exact transaction fence after its final response was
    /// written successfully. A later re-INVITE uses a different transaction
    /// key and is never affected by this generation.
    pub(crate) fn release_transaction(
        &self,
        dialog_id: &DialogId,
        transaction_id: &TransactionKey,
    ) {
        let mut admission = self
            .admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let has_tasks = self.tasks.iter().any(|entry| {
            &entry.key().dialog_id == dialog_id && &entry.key().transaction_id == transaction_id
        });
        if !has_tasks {
            admission
                .closed_transactions
                .remove(&(dialog_id.clone(), transaction_id.clone()));
        }
    }

    pub(crate) async fn close_dialog(
        &self,
        dialog_id: &DialogId,
    ) -> Result<(), ReliableProvisionalError> {
        let records = {
            let mut admission = self
                .admission
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            admission.closed_dialogs.insert(dialog_id.clone());
            self.tasks
                .iter()
                .filter(|entry| &entry.key().dialog_id == dialog_id)
                .map(|entry| (entry.key().clone(), Arc::clone(entry.value())))
                .collect::<Vec<_>>()
        };
        self.abort_and_wait(records).await
    }

    /// Drop the per-dialog admission fence after normal dialog cleanup. This
    /// keeps the registry bounded while the dialog store remains authoritative
    /// against any later operation on the removed dialog.
    pub(crate) fn release_dialog(&self, dialog_id: &DialogId) {
        let mut admission = self
            .admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self
            .tasks
            .iter()
            .any(|entry| &entry.key().dialog_id == dialog_id)
        {
            return;
        }
        admission.closed_dialogs.remove(dialog_id);
        admission
            .closed_transactions
            .retain(|(candidate, _)| candidate != dialog_id);
    }

    /// Stop admission and drain every pending/active generation. On timeout
    /// the records stay registered and callers must not continue shutdown.
    pub(crate) async fn close_all(&self) -> Result<(), ReliableProvisionalError> {
        let records = {
            let mut admission = self
                .admission
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            admission.accepting = false;
            self.tasks
                .iter()
                .map(|entry| (entry.key().clone(), Arc::clone(entry.value())))
                .collect::<Vec<_>>()
        };
        self.abort_and_wait(records).await
    }
}

struct ReliableTaskCompletion {
    registry: Arc<ReliableProvisionalRegistry>,
    key: ReliableProvisionalKey,
    token: u64,
    completion: Option<watch::Sender<bool>>,
}

impl Drop for ReliableTaskCompletion {
    fn drop(&mut self) {
        if let Some(completion) = self.completion.take() {
            let _ = completion.send(true);
        }
        self.registry.remove_exact(&self.key, self.token);
    }
}

async fn wait_for_task_completion_until(
    task: &ReliableProvisionalTask,
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

async fn wait_for_task_completion(task: &ReliableProvisionalTask) -> bool {
    wait_for_task_completion_until(task, tokio::time::Instant::now() + TASK_COMPLETION_TIMEOUT)
        .await
}

/// A reservation installed before the initial reliable provisional wire write.
/// It owns the task start barrier until the write either succeeds or fails.
pub(crate) struct PreparedReliableProvisional {
    registry: Arc<ReliableProvisionalRegistry>,
    key: ReliableProvisionalKey,
    task: Arc<ReliableProvisionalTask>,
    start: Option<oneshot::Sender<()>>,
    settled: bool,
}

impl PreparedReliableProvisional {
    pub(crate) async fn activate(mut self) -> Result<(), ReliableProvisionalError> {
        let activated = {
            let admission = self
                .registry
                .admission
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let transaction_key = (self.key.dialog_id.clone(), self.key.transaction_id.clone());
            let open = admission.accepting
                && !admission.closed_dialogs.contains(&self.key.dialog_id)
                && !admission.closed_transactions.contains(&transaction_key)
                && self.registry.owns(&self.key, self.task.token);
            open && self
                .start
                .take()
                .is_some_and(|start| start.send(()).is_ok())
        };

        if activated {
            self.settled = true;
            Ok(())
        } else {
            // PRACK or an exact final close may legitimately win immediately
            // after the initial 18x write and before this activation step.
            // Once that cancellation has settled the already-written response
            // remains a successful send; only a completion timeout is fatal.
            self.cancel().await?;
            Ok(())
        }
    }

    pub(crate) async fn cancel(mut self) -> Result<(), ReliableProvisionalError> {
        self.start.take();
        self.task.abort.abort();
        if wait_for_task_completion(&self.task).await {
            self.registry.remove_exact(&self.key, self.task.token);
            self.settled = true;
            Ok(())
        } else {
            Err(ReliableProvisionalError::TaskCompletionTimeout { incomplete: 1 })
        }
    }
}

impl Drop for PreparedReliableProvisional {
    fn drop(&mut self) {
        if !self.settled {
            self.start.take();
            self.task.abort.abort();
        }
    }
}

/// Reserve a retransmit generation synchronously before the initial 18x write.
pub(crate) fn prepare_reliable_provisional_retransmit<F>(
    dialog_id: DialogId,
    rseq: u32,
    transaction_id: TransactionKey,
    response: Response,
    transaction_manager: Arc<TransactionManager>,
    registry: Arc<ReliableProvisionalRegistry>,
    dialog_is_live: F,
) -> Result<PreparedReliableProvisional, ReliableProvisionalError>
where
    F: FnOnce() -> bool,
{
    let key = ReliableProvisionalKey {
        dialog_id,
        transaction_id,
        rseq,
    };

    let admission = registry
        .admission
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let transaction_key = (key.dialog_id.clone(), key.transaction_id.clone());
    if !admission.accepting
        || admission.closed_dialogs.contains(&key.dialog_id)
        || admission.closed_transactions.contains(&transaction_key)
        || !dialog_is_live()
    {
        return Err(ReliableProvisionalError::RegistryClosed);
    }
    if registry.tasks.contains_key(&key) {
        return Err(ReliableProvisionalError::DuplicateReservation);
    }

    let token = NEXT_RELIABLE_TASK_TOKEN.fetch_add(1, Ordering::Relaxed);
    let (start_tx, start_rx) = oneshot::channel();
    let (completion_tx, completion_rx) = watch::channel(false);
    let task_registry = Arc::clone(&registry);
    let task_key = key.clone();
    let completion_guard = ReliableTaskCompletion {
        registry: task_registry,
        key: task_key.clone(),
        token,
        completion: Some(completion_tx),
    };
    let handle = tokio::spawn(async move {
        let _completion = completion_guard;
        if start_rx.await.is_err() {
            return;
        }

        let start = Instant::now();
        let mut interval = T1;
        loop {
            tokio::time::sleep(interval).await;
            if start.elapsed() >= ABANDON_WINDOW {
                warn!(
                    dialog=%task_key.dialog_id,
                    rseq=task_key.rseq,
                    "Reliable 18x unacknowledged after 64·T1; abandoning retransmits"
                );
                break;
            }

            match transaction_manager
                .send_response(&task_key.transaction_id, response.clone())
                .await
            {
                Ok(_) => {
                    debug!(
                        dialog=%task_key.dialog_id,
                        rseq=task_key.rseq,
                        ?interval,
                        "Retransmitted reliable 18x"
                    );
                }
                Err(error) => {
                    warn!(
                        dialog=%task_key.dialog_id,
                        rseq=task_key.rseq,
                        error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&error),
                        "Retransmit of reliable 18x failed; stopping"
                    );
                    break;
                }
            }
            interval = (interval * 2).min(T2);
        }
    });

    let task = Arc::new(ReliableProvisionalTask {
        token,
        abort: handle.abort_handle(),
        completion: completion_rx,
    });
    drop(handle);
    registry.tasks.insert(key.clone(), Arc::clone(&task));
    drop(admission);

    Ok(PreparedReliableProvisional {
        registry,
        key,
        task,
        start: Some(start_tx),
        settled: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::Method;

    fn transaction(branch: &str) -> TransactionKey {
        TransactionKey::new(branch.to_string(), Method::Invite, true)
    }

    fn reserve_test_task(
        registry: Arc<ReliableProvisionalRegistry>,
        dialog_id: DialogId,
        transaction_id: TransactionKey,
        rseq: u32,
    ) -> Result<PreparedReliableProvisional, ReliableProvisionalError> {
        let key = ReliableProvisionalKey {
            dialog_id,
            transaction_id,
            rseq,
        };
        let admission = registry
            .admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !admission.accepting
            || admission.closed_dialogs.contains(&key.dialog_id)
            || admission
                .closed_transactions
                .contains(&(key.dialog_id.clone(), key.transaction_id.clone()))
        {
            return Err(ReliableProvisionalError::RegistryClosed);
        }

        let token = NEXT_RELIABLE_TASK_TOKEN.fetch_add(1, Ordering::Relaxed);
        let (start_tx, start_rx) = oneshot::channel();
        let (completion_tx, completion_rx) = watch::channel(false);
        let task_registry = Arc::clone(&registry);
        let task_key = key.clone();
        let completion_guard = ReliableTaskCompletion {
            registry: task_registry,
            key: task_key.clone(),
            token,
            completion: Some(completion_tx),
        };
        let handle = tokio::spawn(async move {
            let _completion = completion_guard;
            if start_rx.await.is_ok() {
                std::future::pending::<()>().await;
            }
        });
        let task = Arc::new(ReliableProvisionalTask {
            token,
            abort: handle.abort_handle(),
            completion: completion_rx,
        });
        drop(handle);
        if registry
            .tasks
            .insert(key.clone(), Arc::clone(&task))
            .is_some()
        {
            task.abort.abort();
            return Err(ReliableProvisionalError::DuplicateReservation);
        }
        drop(admission);
        Ok(PreparedReliableProvisional {
            registry,
            key,
            task,
            start: Some(start_tx),
            settled: false,
        })
    }

    #[tokio::test]
    async fn pending_pre_wire_reservation_drains_before_exact_final() {
        let registry = ReliableProvisionalRegistry::with_capacity(4);
        let dialog_id = DialogId::new();
        let transaction_id = transaction("z9hG4bK-pending-close");
        let prepared = reserve_test_task(
            Arc::clone(&registry),
            dialog_id.clone(),
            transaction_id.clone(),
            1,
        )
        .expect("pre-wire reservation");

        registry
            .close_transaction(&dialog_id, &transaction_id)
            .await
            .expect("pending task must settle");
        assert_eq!(registry.len(), 0);
        prepared
            .activate()
            .await
            .expect("a final close winning before activation is a settled race");
    }

    #[tokio::test]
    async fn immediate_prack_before_activation_is_successful_completion() {
        let registry = ReliableProvisionalRegistry::with_capacity(4);
        let dialog_id = DialogId::new();
        let prepared = reserve_test_task(
            Arc::clone(&registry),
            dialog_id.clone(),
            transaction("z9hG4bK-prack-before-activate"),
            9,
        )
        .expect("pre-wire reservation");

        assert!(registry
            .cancel_prack(&dialog_id, 9)
            .await
            .expect("PRACK cancellation must settle"));
        prepared
            .activate()
            .await
            .expect("already acknowledged initial response remains successful");
        assert_eq!(registry.len(), 0);
    }

    #[tokio::test]
    async fn exact_transaction_close_preserves_reinvite_and_stop_closes_admission() {
        let registry = ReliableProvisionalRegistry::with_capacity(4);
        let dialog_id = DialogId::new();
        let first_transaction = transaction("z9hG4bK-first-invite");
        let first = reserve_test_task(
            Arc::clone(&registry),
            dialog_id.clone(),
            first_transaction.clone(),
            1,
        )
        .expect("first reservation");
        registry
            .close_transaction(&dialog_id, &first_transaction)
            .await
            .expect("first generation close");
        first.activate().await.expect("settled first generation");
        registry.release_transaction(&dialog_id, &first_transaction);

        let second_transaction = transaction("z9hG4bK-reinvite");
        let _second = reserve_test_task(Arc::clone(&registry), dialog_id, second_transaction, 1)
            .expect("later re-INVITE has an independent exact key");
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
}
