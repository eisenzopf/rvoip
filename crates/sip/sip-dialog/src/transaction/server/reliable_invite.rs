//! UAS reliable-provisional retransmission (RFC 3262 §3).
//!
//! A reliable provisional is reserved in the registry before its initial wire
//! write.  The retransmit task remains behind a start barrier until that write
//! succeeds.  An exact final-response close therefore rejects new reservations
//! and drains both pending and active retransmit generations before the final
//! response is allowed onto the wire.

use std::collections::{HashMap, HashSet};
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
    /// Exact INVITE transaction fences, grouped by dialog so that dialog
    /// cleanup releases all of a dialog's fences with one removal.
    closed_transactions: HashMap<DialogId, HashSet<TransactionKey>>,
    closed_dialogs: HashSet<DialogId>,
}

impl ReliableRegistryAdmission {
    fn transaction_closed(&self, dialog_id: &DialogId, transaction_id: &TransactionKey) -> bool {
        self.closed_transactions
            .get(dialog_id)
            .is_some_and(|transactions| transactions.contains(transaction_id))
    }

    fn close_transaction(&mut self, dialog_id: &DialogId, transaction_id: &TransactionKey) {
        self.closed_transactions
            .entry(dialog_id.clone())
            .or_default()
            .insert(transaction_id.clone());
    }

    fn release_transaction(&mut self, dialog_id: &DialogId, transaction_id: &TransactionKey) {
        if let Some(transactions) = self.closed_transactions.get_mut(dialog_id) {
            transactions.remove(transaction_id);
            if transactions.is_empty() {
                self.closed_transactions.remove(dialog_id);
            }
        }
    }
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
///
/// `by_dialog` indexes the keys of `tasks` by dialog. Every per-call lookup
/// (PRACK, exact final-response close, dialog cleanup) reads the dialog's
/// index entry instead of iterating every shard of `tasks`, so its cost does
/// not grow with the number of live calls. Lock order: a `by_dialog` guard may
/// be held while reading `tasks`, never the reverse.
pub(crate) struct ReliableProvisionalRegistry {
    tasks: DashMap<ReliableProvisionalKey, Arc<ReliableProvisionalTask>>,
    by_dialog: DashMap<DialogId, HashSet<ReliableProvisionalKey>>,
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
            by_dialog: DashMap::with_capacity(capacity),
            admission: StdMutex::new(ReliableRegistryAdmission {
                accepting: true,
                closed_transactions: HashMap::new(),
                closed_dialogs: HashSet::new(),
            }),
        })
    }

    pub(crate) fn len(&self) -> usize {
        self.tasks.len()
    }

    pub(crate) fn has_tasks_for_dialog(&self, dialog_id: &DialogId) -> bool {
        self.dialog_has_task(dialog_id, |_| true)
    }

    /// Whether `tasks` holds a key of `dialog_id` that matches `filter`.
    ///
    /// Each candidate from the index is confirmed against `tasks`, so the
    /// answer is the one a scan of `tasks` would give even while a concurrent
    /// removal has not yet unindexed its key.
    fn dialog_has_task(
        &self,
        dialog_id: &DialogId,
        filter: impl Fn(&ReliableProvisionalKey) -> bool,
    ) -> bool {
        self.by_dialog.get(dialog_id).is_some_and(|keys| {
            keys.iter()
                .any(|key| filter(key) && self.tasks.contains_key(key))
        })
    }

    /// The registered `(key, task)` records of `dialog_id` matching `filter`.
    fn dialog_records(
        &self,
        dialog_id: &DialogId,
        filter: impl Fn(&ReliableProvisionalKey) -> bool,
    ) -> Vec<(ReliableProvisionalKey, Arc<ReliableProvisionalTask>)> {
        let Some(keys) = self.by_dialog.get(dialog_id) else {
            return Vec::new();
        };
        keys.iter()
            .filter(|&key| filter(key))
            .filter_map(|key| {
                self.tasks
                    .get(key)
                    .map(|task| (key.clone(), Arc::clone(task.value())))
            })
            .collect()
    }

    /// Register a task and index its key. Callers hold `admission`.
    fn insert_task(
        &self,
        key: ReliableProvisionalKey,
        task: Arc<ReliableProvisionalTask>,
    ) -> Option<Arc<ReliableProvisionalTask>> {
        // Insert into `tasks` first: a concurrent `unindex` of an earlier
        // generation of the same key then finds it present and keeps the
        // index entry.
        let previous = self.tasks.insert(key.clone(), task);
        self.by_dialog
            .entry(key.dialog_id.clone())
            .or_default()
            .insert(key);
        previous
    }

    /// Drop `key` from the index once `tasks` no longer holds it.
    fn unindex(&self, key: &ReliableProvisionalKey) {
        if let dashmap::mapref::entry::Entry::Occupied(mut keys) =
            self.by_dialog.entry(key.dialog_id.clone())
        {
            // The entry guard serializes this with `insert_task`'s index
            // update; a re-reservation of the same key keeps its entry.
            if !self.tasks.contains_key(key) {
                keys.get_mut().remove(key);
            }
            if keys.get().is_empty() {
                keys.remove();
            }
        }
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
        for (_, task) in self.dialog_records(dialog_id, |_| true) {
            task.abort.abort();
        }
    }

    fn remove_exact(
        &self,
        key: &ReliableProvisionalKey,
        token: u64,
    ) -> Option<Arc<ReliableProvisionalTask>> {
        let removed = self
            .tasks
            .remove_if(key, |_, current| current.token == token)
            .map(|(_, task)| task);
        if removed.is_some() {
            self.unindex(key);
        }
        removed
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
        let records = self.dialog_records(dialog_id, |key| key.rseq == rseq);
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
            admission.close_transaction(dialog_id, transaction_id);
            self.dialog_records(dialog_id, |key| &key.transaction_id == transaction_id)
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
        if !self.dialog_has_task(dialog_id, |key| &key.transaction_id == transaction_id) {
            admission.release_transaction(dialog_id, transaction_id);
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
            self.dialog_records(dialog_id, |_| true)
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
        if self.has_tasks_for_dialog(dialog_id) {
            return;
        }
        admission.closed_dialogs.remove(dialog_id);
        admission.closed_transactions.remove(dialog_id);
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

    (tokio::time::timeout_at(deadline, async {
        loop {
            if *completion.borrow() {
                return true;
            }
            if completion.changed().await.is_err() {
                return *completion.borrow();
            }
        }
    })
    .await)
        .unwrap_or_default()
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
            let open = admission.accepting
                && !admission.closed_dialogs.contains(&self.key.dialog_id)
                && !admission.transaction_closed(&self.key.dialog_id, &self.key.transaction_id)
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
    if !admission.accepting
        || admission.closed_dialogs.contains(&key.dialog_id)
        || admission.transaction_closed(&key.dialog_id, &key.transaction_id)
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
    registry.insert_task(key.clone(), Arc::clone(&task));
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
            || admission.transaction_closed(&key.dialog_id, &key.transaction_id)
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
            .insert_task(key.clone(), Arc::clone(&task))
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

    /// Reference answer: the full scan of `tasks` the index replaces.
    fn scanned_keys(
        registry: &ReliableProvisionalRegistry,
        dialog_id: &DialogId,
        filter: impl Fn(&ReliableProvisionalKey) -> bool,
    ) -> HashSet<ReliableProvisionalKey> {
        registry
            .tasks
            .iter()
            .filter(|entry| &entry.key().dialog_id == dialog_id && filter(entry.key()))
            .map(|entry| entry.key().clone())
            .collect()
    }

    fn indexed_keys(
        registry: &ReliableProvisionalRegistry,
        dialog_id: &DialogId,
        filter: impl Fn(&ReliableProvisionalKey) -> bool,
    ) -> HashSet<ReliableProvisionalKey> {
        registry
            .dialog_records(dialog_id, filter)
            .into_iter()
            .map(|(key, _)| key)
            .collect()
    }

    /// Every index lookup agrees with the scan, and the index holds exactly
    /// the keys of `tasks` (no stale entries, no empty dialog sets).
    fn assert_index_matches_scan(
        registry: &ReliableProvisionalRegistry,
        dialogs: &[DialogId],
        transactions: &[TransactionKey],
    ) {
        for dialog_id in dialogs {
            assert_eq!(
                registry.has_tasks_for_dialog(dialog_id),
                !scanned_keys(registry, dialog_id, |_| true).is_empty()
            );
            assert_eq!(
                indexed_keys(registry, dialog_id, |_| true),
                scanned_keys(registry, dialog_id, |_| true)
            );
            for transaction_id in transactions {
                let same_transaction =
                    |key: &ReliableProvisionalKey| &key.transaction_id == transaction_id;
                assert_eq!(
                    registry.dialog_has_task(dialog_id, same_transaction),
                    !scanned_keys(registry, dialog_id, same_transaction).is_empty()
                );
                assert_eq!(
                    indexed_keys(registry, dialog_id, same_transaction),
                    scanned_keys(registry, dialog_id, same_transaction)
                );
            }
            for rseq in 1..=3 {
                let same_rseq = |key: &ReliableProvisionalKey| key.rseq == rseq;
                assert_eq!(
                    indexed_keys(registry, dialog_id, same_rseq),
                    scanned_keys(registry, dialog_id, same_rseq)
                );
            }
        }
        let indexed = registry
            .by_dialog
            .iter()
            .flat_map(|entry| {
                assert!(!entry.value().is_empty(), "empty dialog sets are removed");
                entry.value().iter().cloned().collect::<Vec<_>>()
            })
            .collect::<HashSet<_>>();
        let registered = registry
            .tasks
            .iter()
            .map(|entry| entry.key().clone())
            .collect::<HashSet<_>>();
        assert_eq!(indexed, registered);
    }

    #[tokio::test]
    async fn dialog_index_matches_a_full_scan_across_reserve_cancel_close_and_release() {
        let registry = ReliableProvisionalRegistry::with_capacity(16);
        let dialogs = [DialogId::new(), DialogId::new(), DialogId::new()];
        let transactions = [
            transaction("z9hG4bK-index-a"),
            transaction("z9hG4bK-index-b"),
        ];
        let mut prepared: Vec<PreparedReliableProvisional> = Vec::new();
        // Deterministic xorshift so a failure reproduces exactly.
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = |bound: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % bound as u64) as usize
        };

        for _ in 0..600 {
            let dialog_id = &dialogs[next(dialogs.len())];
            let transaction_id = &transactions[next(transactions.len())];
            let rseq = next(3) as u32 + 1;
            match next(8) {
                0..=2 => {
                    let key = ReliableProvisionalKey {
                        dialog_id: dialog_id.clone(),
                        transaction_id: transaction_id.clone(),
                        rseq,
                    };
                    // The helper replaces duplicates; reserve only free keys,
                    // as `prepare_reliable_provisional_retransmit` does.
                    if !registry.tasks.contains_key(&key) {
                        if let Ok(reservation) = reserve_test_task(
                            Arc::clone(&registry),
                            dialog_id.clone(),
                            transaction_id.clone(),
                            rseq,
                        ) {
                            prepared.push(reservation);
                        }
                    }
                }
                3 if !prepared.is_empty() => {
                    // Cancelling, then possibly re-reserving, the same key
                    // re-keys it under a new token.
                    let reservation = prepared.swap_remove(next(prepared.len()));
                    reservation.cancel().await.expect("cancel settles");
                }
                4 => {
                    registry
                        .cancel_prack(dialog_id, rseq)
                        .await
                        .expect("PRACK cancellation settles");
                }
                5 => {
                    registry
                        .close_transaction(dialog_id, transaction_id)
                        .await
                        .expect("exact close settles");
                    registry.release_transaction(dialog_id, transaction_id);
                }
                6 => {
                    registry
                        .close_dialog(dialog_id)
                        .await
                        .expect("dialog close settles");
                    registry.release_dialog(dialog_id);
                }
                _ => registry.release_dialog(dialog_id),
            }
            assert_index_matches_scan(&registry, &dialogs, &transactions);
        }

        registry.close_all().await.expect("teardown drains");
        for reservation in prepared {
            reservation
                .activate()
                .await
                .expect("drained reservation settles");
        }
        assert_index_matches_scan(&registry, &dialogs, &transactions);
        assert_eq!(registry.len(), 0);
        assert!(registry.by_dialog.is_empty());
    }

    #[tokio::test]
    async fn dialog_lookups_visit_only_that_dialogs_keys() {
        let registry = ReliableProvisionalRegistry::with_capacity(1024);
        let transaction_id = transaction("z9hG4bK-index-scale");
        let target = DialogId::new();
        let mut prepared = Vec::new();
        for _ in 0..1_000 {
            prepared.push(
                reserve_test_task(
                    Arc::clone(&registry),
                    DialogId::new(),
                    transaction_id.clone(),
                    1,
                )
                .expect("other dialog reservation"),
            );
        }
        for rseq in 1..=2 {
            prepared.push(
                reserve_test_task(
                    Arc::clone(&registry),
                    target.clone(),
                    transaction_id.clone(),
                    rseq,
                )
                .expect("target reservation"),
            );
        }

        // A lookup walks the dialog's index entry only: two keys out of
        // 1,002 registered tasks.
        assert_eq!(registry.len(), 1_002);
        assert_eq!(registry.by_dialog.get(&target).expect("indexed").len(), 2);
        assert_eq!(registry.dialog_records(&target, |_| true).len(), 2);
        assert!(!registry.has_tasks_for_dialog(&DialogId::new()));

        registry.close_dialog(&target).await.expect("dialog close");
        assert!(registry.by_dialog.get(&target).is_none());
        assert_eq!(registry.len(), 1_000);
        registry.close_all().await.expect("teardown drains");
        for reservation in prepared {
            reservation
                .activate()
                .await
                .expect("drained reservation settles");
        }
        assert!(registry.by_dialog.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_regeneration_of_the_same_keys_leaves_no_stale_index_entries() {
        let registry = ReliableProvisionalRegistry::with_capacity(16);
        let dialog_id = DialogId::new();
        let mut workers = Vec::new();
        for worker in 0..8u32 {
            let registry = Arc::clone(&registry);
            let dialog_id = dialog_id.clone();
            workers.push(tokio::spawn(async move {
                let transaction_id = transaction("z9hG4bK-index-race");
                for _ in 0..200 {
                    // Dropping an unsettled reservation aborts its task, whose
                    // completion guard removes the old generation on another
                    // thread while this worker reserves the same key again.
                    if let Ok(reservation) = reserve_test_task(
                        Arc::clone(&registry),
                        dialog_id.clone(),
                        transaction_id.clone(),
                        worker % 3 + 1,
                    ) {
                        drop(reservation);
                    }
                    tokio::task::yield_now().await;
                }
            }));
        }
        for worker in workers {
            worker.await.expect("worker");
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            while registry.len() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("aborted generations unregister");
        assert!(!registry.has_tasks_for_dialog(&dialog_id));
        assert!(registry.by_dialog.is_empty());
    }

    #[tokio::test]
    async fn closed_transaction_fences_are_grouped_by_dialog() {
        let registry = ReliableProvisionalRegistry::with_capacity(4);
        let dialog_id = DialogId::new();
        let other_dialog = DialogId::new();
        let first = transaction("z9hG4bK-fence-first");
        let second = transaction("z9hG4bK-fence-second");
        for transaction_id in [&first, &second] {
            registry
                .close_transaction(&dialog_id, transaction_id)
                .await
                .expect("close");
        }
        registry
            .close_transaction(&other_dialog, &first)
            .await
            .expect("close");
        {
            let admission = registry.admission.lock().expect("admission");
            assert!(admission.transaction_closed(&dialog_id, &first));
            assert!(admission.transaction_closed(&dialog_id, &second));
            assert_eq!(admission.closed_transactions.len(), 2);
        }

        registry.release_transaction(&dialog_id, &first);
        {
            let admission = registry.admission.lock().expect("admission");
            assert!(!admission.transaction_closed(&dialog_id, &first));
            assert!(admission.transaction_closed(&dialog_id, &second));
        }
        assert!(matches!(
            reserve_test_task(Arc::clone(&registry), dialog_id.clone(), second.clone(), 1),
            Err(ReliableProvisionalError::RegistryClosed)
        ));

        // Dialog cleanup drops only that dialog's fences, with one removal.
        registry.release_dialog(&dialog_id);
        let admission = registry.admission.lock().expect("admission");
        assert!(!admission.closed_transactions.contains_key(&dialog_id));
        assert!(admission.transaction_closed(&other_dialog, &first));
    }
}
