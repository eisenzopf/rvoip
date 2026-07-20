//! BYE Request Handler for Dialog-Core
//!
//! This module handles BYE requests according to RFC 3261 Section 15.
//! BYE requests terminate established SIP dialogs and clean up associated resources.
//!
//! ## BYE Processing Steps
//!
//! 1. **Dialog Identification**: Match BYE to existing dialog using Call-ID and tags
//! 2. **Authorization Check**: Verify BYE is from dialog participant
//! 3. **State Validation**: Ensure dialog is in confirmable state for termination
//! 4. **Resource Cleanup**: Terminate dialog and clean up associated state
//! 5. **Response Generation**: Send 200 OK to acknowledge BYE receipt
//!
//! ## Error Handling
//!
//! - **481 Call/Transaction Does Not Exist**: No matching dialog found
//! - **403 Forbidden**: BYE from unauthorized party
//! - **500 Server Internal Error**: Processing failures

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, Weak};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use tokio::sync::{mpsc, Notify, OwnedSemaphorePermit};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::diagnostics;
use crate::dialog::{Dialog, DialogId, DialogState};
use crate::errors::{DialogError, DialogResult};
#[cfg(test)]
use crate::events::SessionCoordinationEvent;
use crate::manager::core::TerminatedByeDialogKey;
use crate::manager::utils::DialogUtils;
use crate::manager::{DialogManager, SourceExtractor};
use crate::transaction::{utils::response_builders, TransactionKey};
use rvoip_sip_core::{HeaderName, Request, StatusCode, TypedHeader};

#[cfg(not(test))]
const BYE_CLEANUP_COMPLETION_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(test)]
const BYE_CLEANUP_COMPLETION_TIMEOUT: Duration = Duration::from_millis(50);
#[cfg(not(test))]
const BYE_CLEANUP_PHASE_RETRY_LIMIT: usize = 8;
#[cfg(test)]
const BYE_CLEANUP_PHASE_RETRY_LIMIT: usize = 2;
static NEXT_BYE_CLEANUP_TOKEN: AtomicU64 = AtomicU64::new(1);

type ByeCleanupFuture = Pin<Box<dyn Future<Output = bool> + Send + 'static>>;

struct ByeCleanupWork {
    registry: Weak<ByeCleanupTaskRegistry>,
    key: ByeCleanupKey,
    token: u64,
    cleanup: Option<ByeCleanupFuture>,
    capacity: Option<OwnedSemaphorePermit>,
    retire_on_drop: bool,
}

impl ByeCleanupWork {
    async fn run(mut self) {
        let succeeded = self
            .cleanup
            .take()
            .expect("queued BYE cleanup owns its future")
            .await;
        if !succeeded {
            if let Some(registry) = self.registry.upgrade() {
                registry.failed.store(true, Ordering::Release);
                let capacity = self
                    .capacity
                    .take()
                    .expect("failed BYE cleanup retains its capacity");
                registry.quarantined.insert(
                    self.key.clone(),
                    ByeCleanupQuarantine {
                        _capacity: capacity,
                    },
                );
                self.retire_on_drop = false;
                registry.drained.notify_waiters();
            }
        } else if let Some(registry) = self.registry.upgrade() {
            registry.record_success();
        }
    }
}

impl Drop for ByeCleanupWork {
    fn drop(&mut self) {
        if self.retire_on_drop {
            if let Some(registry) = self.registry.upgrade() {
                registry.remove_exact(&self.key, self.token);
            }
        }
    }
}

#[derive(Debug)]
struct ByeCleanupQuarantine {
    _capacity: OwnedSemaphorePermit,
}

enum ByeCleanupCommand {
    Run(ByeCleanupWork),
    Shutdown,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum ByeCleanupKey {
    Dialog(DialogId),
    Transaction(TransactionKey),
}

#[derive(Debug)]
struct ByeCleanupAdmission {
    accepting: bool,
}

#[derive(Debug)]
pub(crate) struct ByeCleanupTaskRegistry {
    tasks: DashMap<ByeCleanupKey, u64>,
    admission: StdMutex<ByeCleanupAdmission>,
    sender: StdMutex<Option<mpsc::Sender<ByeCleanupCommand>>>,
    workers: tokio::sync::Mutex<Vec<JoinHandle<()>>>,
    capacity: Arc<tokio::sync::Semaphore>,
    quarantined: DashMap<ByeCleanupKey, ByeCleanupQuarantine>,
    drained: Notify,
    failed: AtomicBool,
    #[cfg(test)]
    worker_tasks_spawned: AtomicU64,
    #[cfg(test)]
    cleanup_jobs_enqueued: AtomicU64,
    #[cfg(test)]
    cleanup_jobs_completed: AtomicU64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ByeCleanupError {
    RegistryClosed,
    Duplicate,
    CompletionTimeout,
    TaskFailed,
}

impl fmt::Display for ByeCleanupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RegistryClosed => formatter.write_str("BYE cleanup registry is closed"),
            Self::Duplicate => formatter.write_str("BYE cleanup already registered"),
            Self::CompletionTimeout => formatter.write_str("BYE cleanup task did not complete"),
            Self::TaskFailed => formatter.write_str("BYE cleanup task failed"),
        }
    }
}

impl ByeCleanupTaskRegistry {
    pub(crate) fn with_capacity(capacity: usize) -> Arc<Self> {
        let capacity = capacity.max(1);
        let (sender, receiver) = mpsc::channel(capacity);
        let receiver = Arc::new(tokio::sync::Mutex::new(receiver));
        let worker_count = capacity.min(4);
        let mut workers = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let receiver = Arc::clone(&receiver);
            workers.push(tokio::spawn(async move {
                loop {
                    let command = receiver.lock().await.recv().await;
                    match command {
                        Some(ByeCleanupCommand::Run(work)) => work.run().await,
                        Some(ByeCleanupCommand::Shutdown) | None => break,
                    }
                }
            }));
        }
        Arc::new(Self {
            tasks: DashMap::with_capacity(capacity),
            admission: StdMutex::new(ByeCleanupAdmission { accepting: true }),
            sender: StdMutex::new(Some(sender)),
            workers: tokio::sync::Mutex::new(workers),
            capacity: Arc::new(tokio::sync::Semaphore::new(capacity)),
            quarantined: DashMap::new(),
            drained: Notify::new(),
            failed: AtomicBool::new(false),
            #[cfg(test)]
            worker_tasks_spawned: AtomicU64::new(worker_count as u64),
            #[cfg(test)]
            cleanup_jobs_enqueued: AtomicU64::new(0),
            #[cfg(test)]
            cleanup_jobs_completed: AtomicU64::new(0),
        })
    }

    pub(crate) fn len(&self) -> usize {
        self.tasks.len()
    }

    fn record_success(&self) {
        #[cfg(test)]
        self.cleanup_jobs_completed.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn begin_close(&self) {
        let mut admission = self
            .admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        admission.accepting = false;
        self.capacity.close();
    }

    fn remove_exact(&self, key: &ByeCleanupKey, token: u64) {
        if self
            .tasks
            .remove_if(key, |_, current| *current == token)
            .is_some()
        {
            self.drained.notify_waiters();
        }
    }

    async fn reserve_capacity(&self) -> Result<OwnedSemaphorePermit, ByeCleanupError> {
        {
            let admission = self
                .admission
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !admission.accepting {
                return Err(ByeCleanupError::RegistryClosed);
            }
        }
        Arc::clone(&self.capacity)
            .acquire_owned()
            .await
            .map_err(|_| ByeCleanupError::RegistryClosed)
    }

    fn register(
        self: &Arc<Self>,
        key: ByeCleanupKey,
        capacity: OwnedSemaphorePermit,
        cleanup: ByeCleanupFuture,
        force_transaction_termination: Arc<AtomicBool>,
    ) -> Result<PreparedByeCleanup, ByeCleanupError> {
        let admission = self
            .admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Owning `capacity` is the exact pre-close admission token. Shutdown
        // closes the semaphore to wake non-admitted waiters, but a handler
        // that acquired this permit before the close boundary must still be
        // allowed to register after its dialog mutation.
        let token = NEXT_BYE_CLEANUP_TOKEN.fetch_add(1, Ordering::Relaxed);
        match self.tasks.entry(key.clone()) {
            dashmap::mapref::entry::Entry::Vacant(entry) => {
                entry.insert(token);
            }
            dashmap::mapref::entry::Entry::Occupied(_) => {
                return Err(ByeCleanupError::Duplicate);
            }
        }
        let work = ByeCleanupWork {
            registry: Arc::downgrade(self),
            key,
            token,
            cleanup: Some(cleanup),
            capacity: Some(capacity),
            retire_on_drop: true,
        };
        let sender = self
            .sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .ok_or(ByeCleanupError::RegistryClosed)?;
        drop(admission);

        Ok(PreparedByeCleanup {
            registry: Arc::clone(self),
            sender,
            work: Some(work),
            force_transaction_termination,
        })
    }

    fn enqueue(&self, sender: &mpsc::Sender<ByeCleanupCommand>, work: ByeCleanupWork) {
        match sender.try_send(ByeCleanupCommand::Run(work)) {
            Ok(()) => {
                #[cfg(test)]
                self.cleanup_jobs_enqueued.fetch_add(1, Ordering::Relaxed);
            }
            Err(mpsc::error::TrySendError::Closed(command))
            | Err(mpsc::error::TrySendError::Full(command)) => {
                // Every admitted work item owns one semaphore slot through
                // completion, while the queue is sized to that same bound.
                // Full is therefore unreachable unless those invariants are
                // broken; dropping the command still runs the exact cleanup
                // completion guard and makes shutdown fail closed.
                self.failed.store(true, Ordering::Release);
                drop(command);
            }
        }
    }

    async fn wait_empty_until(&self, deadline: tokio::time::Instant) -> bool {
        loop {
            let notified = self.drained.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.tasks.is_empty() {
                return true;
            }
            if self.failed.load(Ordering::Acquire) || !self.quarantined.is_empty() {
                return false;
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return self.tasks.is_empty();
            }
        }
    }

    pub(crate) async fn close_all(&self) -> Result<(), ByeCleanupError> {
        self.begin_close();
        let sender = {
            self.sender
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        };

        let grace_deadline = tokio::time::Instant::now() + BYE_CLEANUP_COMPLETION_TIMEOUT;
        let mut quarantined_failure = !self.quarantined.is_empty();
        if !quarantined_failure && !self.wait_empty_until(grace_deadline).await {
            quarantined_failure =
                self.failed.load(Ordering::Acquire) || !self.quarantined.is_empty();
        }
        if !quarantined_failure && !self.tasks.is_empty() {
            // Preserve the worker and its exact in-flight job. A later stop
            // retry resumes the same event send and cleanup instead of losing
            // or duplicating a ByeReceived notification.
            return Err(ByeCleanupError::CompletionTimeout);
        }
        if let Some(sender) = sender {
            let shutdown_count = self
                .workers
                .lock()
                .await
                .iter()
                .filter(|worker| !worker.is_finished())
                .count();
            for _ in 0..shutdown_count {
                match tokio::time::timeout_at(
                    grace_deadline,
                    sender.send(ByeCleanupCommand::Shutdown),
                )
                .await
                {
                    Ok(Ok(())) => {}
                    Ok(Err(_)) => break,
                    Err(_) => return Err(ByeCleanupError::CompletionTimeout),
                }
            }
        }

        let mut workers = self.workers.lock().await;
        let mut worker_failed = false;
        while !workers.is_empty() {
            match tokio::time::timeout_at(grace_deadline, &mut workers[0]).await {
                Ok(result) => {
                    worker_failed |= result.is_err();
                    drop(workers.swap_remove(0));
                }
                Err(_) => return Err(ByeCleanupError::CompletionTimeout),
            }
        }
        self.sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if worker_failed {
            self.failed.store(true, Ordering::Release);
        }
        if quarantined_failure {
            self.failed.store(true, Ordering::Release);
        }
        if self.failed.load(Ordering::Acquire) {
            Err(ByeCleanupError::TaskFailed)
        } else {
            Ok(())
        }
    }
}

struct PreparedByeCleanup {
    registry: Arc<ByeCleanupTaskRegistry>,
    sender: mpsc::Sender<ByeCleanupCommand>,
    work: Option<ByeCleanupWork>,
    force_transaction_termination: Arc<AtomicBool>,
}

impl PreparedByeCleanup {
    fn activate(mut self) {
        if let Some(work) = self.work.take() {
            self.registry.enqueue(&self.sender, work);
        }
    }

    fn disarm(mut self) {
        // The response API returned only after its Completed command was
        // accepted, so the transaction runner owns natural Timer J/K. Dropping
        // this unqueued fallback releases its exact reservation.
        self.work.take();
    }
}

impl Drop for PreparedByeCleanup {
    fn drop(&mut self) {
        // A response-send error or cancellation after the dialog became
        // terminal must not strand cleanup. Capacity was reserved before that
        // mutation, so dispatch here is synchronous and cannot be lost to
        // queue backpressure.
        if let Some(work) = self.work.take() {
            self.force_transaction_termination
                .store(true, Ordering::Release);
            self.registry.enqueue(&self.sender, work);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ByeSequenceDisposition {
    Fresh,
    DuplicateTerminated,
    TerminatedMismatch,
}

fn classify_bye_sequence(
    dialog: &Dialog,
    request: &Request,
) -> DialogResult<ByeSequenceDisposition> {
    let new_seq = bye_cseq(request)?;

    if dialog.state == DialogState::Terminated {
        if dialog.remote_cseq != 0 && new_seq == dialog.remote_cseq {
            Ok(ByeSequenceDisposition::DuplicateTerminated)
        } else {
            Ok(ByeSequenceDisposition::TerminatedMismatch)
        }
    } else {
        Ok(ByeSequenceDisposition::Fresh)
    }
}

fn bye_cseq(request: &Request) -> DialogResult<u32> {
    match request.header(&HeaderName::CSeq) {
        Some(TypedHeader::CSeq(cseq)) => Ok(cseq.sequence()),
        _ => Err(DialogError::protocol_error("Request missing CSeq header")),
    }
}

fn matches_terminated_bye_retransmit(manager: &DialogManager, request: &Request) -> bool {
    let Ok(cseq) = bye_cseq(request) else {
        return false;
    };
    let Some((call_id, Some(from_tag), Some(to_tag))) = DialogUtils::extract_dialog_info(request)
    else {
        return false;
    };

    let key = TerminatedByeDialogKey::canonical(&call_id, &from_tag, &to_tag);
    manager
        .terminated_bye_lookup
        .get(&key)
        .is_some_and(|entry| entry.value().cseq == cseq)
}

/// BYE-specific handling operations
pub trait ByeHandler {
    /// Handle BYE requests (dialog-terminating)
    fn handle_bye_method(
        &self,
        request: Request,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
}

/// Implementation of BYE handling for DialogManager
impl ByeHandler for DialogManager {
    /// Handle BYE requests according to RFC 3261 Section 15
    ///
    /// Terminates the dialog and sends appropriate responses.
    async fn handle_bye_method(&self, request: Request) -> DialogResult<()> {
        debug!("Processing BYE request");

        let source = SourceExtractor::extract_from_request(&request);

        // Create server transaction
        let server_transaction = self
            .transaction_manager
            .create_server_transaction(request.clone(), source)
            .await
            .map_err(|_error| DialogError::TransactionError {
                message: "Failed to create server transaction for BYE".to_string(),
            })?;

        let transaction_id = server_transaction.id().clone();

        self.handle_bye_with_transaction(transaction_id, request)
            .await
    }
}

/// BYE-specific helper methods for DialogManager
impl DialogManager {
    async fn settle_failed_bye_transaction(&self, transaction_id: &TransactionKey) -> bool {
        for attempt in 0..BYE_CLEANUP_PHASE_RETRY_LIMIT {
            if self
                .transaction_manager
                .recover_bye_final_response_lifecycle(transaction_id)
                .await
            {
                return true;
            }
            if self
                .transaction_manager
                .bye_final_response_may_have_reached_wire(transaction_id)
            {
                warn!(transaction=%transaction_id, "Wire-unknown BYE response has no recoverable runner; retaining exact transaction for operator-visible quarantine");
                return false;
            }
            let _ = self
                .transaction_manager
                .terminate_transaction(transaction_id)
                .await;
            if self
                .transaction_manager
                .recover_bye_final_response_lifecycle(transaction_id)
                .await
            {
                return true;
            }
            if attempt + 1 < BYE_CLEANUP_PHASE_RETRY_LIMIT {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        warn!(transaction=%transaction_id, "BYE transaction did not reach a proven terminal lifecycle; quarantining cleanup");
        false
    }

    fn prepare_bye_transaction_recovery(
        &self,
        transaction_id: TransactionKey,
        capacity: OwnedSemaphorePermit,
    ) -> Result<PreparedByeCleanup, ByeCleanupError> {
        let registry = Arc::clone(&self.bye_cleanup_tasks);
        let manager = self.clone();
        let cleanup_key = ByeCleanupKey::Transaction(transaction_id.clone());
        let force_transaction_termination = Arc::new(AtomicBool::new(false));
        let cleanup_force_transaction_termination = Arc::clone(&force_transaction_termination);
        let cleanup = Box::pin(async move {
            if cleanup_force_transaction_termination.load(Ordering::Acquire) {
                return manager.settle_failed_bye_transaction(&transaction_id).await;
            }
            true
        });
        registry.register(
            cleanup_key,
            capacity,
            cleanup,
            force_transaction_termination,
        )
    }

    fn prepare_bye_cleanup(
        &self,
        transaction_id: TransactionKey,
        dialog_id: DialogId,
        capacity: OwnedSemaphorePermit,
    ) -> Result<PreparedByeCleanup, ByeCleanupError> {
        let registry = Arc::clone(&self.bye_cleanup_tasks);
        let cleanup_dialog_id = dialog_id.clone();
        let manager = self.clone();
        let force_transaction_termination = Arc::new(AtomicBool::new(false));
        let cleanup_force_transaction_termination = Arc::clone(&force_transaction_termination);
        let cleanup = Box::pin(async move {
            if cleanup_force_transaction_termination.load(Ordering::Acquire) {
                // A cancellation can race after the final write but before
                // the Completed command is accepted. Preserve natural Timer J
                // whenever the exact wire fence advanced; only a proven
                // pre-wire failure is force-terminated.
                if !manager.settle_failed_bye_transaction(&transaction_id).await {
                    return false;
                }
            }

            let mut event_emitted = false;
            let mut producer_failures = 0usize;
            let mut event_failures = 0usize;
            let mut storage_failures = 0usize;
            loop {
                let refresh_closed = manager
                    .session_refresh_tasks
                    .cancel_dialog(&cleanup_dialog_id)
                    .await
                    .is_ok();
                let reliable_closed = manager
                    .reliable_provisional_tasks
                    .close_dialog(&cleanup_dialog_id)
                    .await
                    .is_ok();
                if !(refresh_closed && reliable_closed) {
                    producer_failures += 1;
                    warn!(dialog=%cleanup_dialog_id, "Protocol producer did not join during BYE cleanup; retaining exact job for retry");
                    if producer_failures >= BYE_CLEANUP_PHASE_RETRY_LIMIT {
                        return false;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    continue;
                }

                if !event_emitted {
                    let emit_started = diagnostics::dialog_timing_enabled().then(Instant::now);
                    if manager
                        .deliver_bye_received_authoritative(&cleanup_dialog_id)
                        .await
                        .is_err()
                    {
                        event_failures += 1;
                        warn!(dialog=%cleanup_dialog_id, "Authoritative ByeReceived delivery failed; retaining exact job for retry");
                        if event_failures >= BYE_CLEANUP_PHASE_RETRY_LIMIT {
                            return false;
                        }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                        continue;
                    }
                    if let Some(started) = emit_started {
                        diagnostics::record_bye_path_cleanup_emit(started.elapsed());
                    }
                    diagnostics::record_bye_cleanup_event_emitted();
                    event_emitted = true;
                }

                let remove_started = diagnostics::dialog_timing_enabled().then(Instant::now);
                let removed = manager.remove_dialog_storage(&cleanup_dialog_id).is_some();
                let already_removed = !manager.has_dialog(&cleanup_dialog_id);
                if let Some(started) = remove_started {
                    diagnostics::record_bye_path_cleanup_remove_storage(started.elapsed());
                }
                if removed || already_removed {
                    debug!(dialog=%cleanup_dialog_id, "BYE cleanup completed");
                    return true;
                }

                storage_failures += 1;
                warn!(dialog=%cleanup_dialog_id, "BYE cleanup could not remove dialog storage; retaining exact job for retry");
                if storage_failures >= BYE_CLEANUP_PHASE_RETRY_LIMIT {
                    return false;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });

        registry.register(
            ByeCleanupKey::Dialog(dialog_id),
            capacity,
            cleanup,
            force_transaction_termination,
        )
    }

    /// Handle a BYE using the server transaction already created by the
    /// transaction manager.
    pub async fn handle_bye_with_transaction(
        &self,
        transaction_id: TransactionKey,
        request: Request,
    ) -> DialogResult<()> {
        let handler_started = diagnostics::dialog_timing_enabled().then(Instant::now);
        self.record_bye_handler_entry(&transaction_id, handler_started);

        // Find the dialog for this BYE
        let lookup_started = diagnostics::dialog_timing_enabled().then(Instant::now);
        let dialog_id = self.find_dialog_for_request(&request).await;
        if let Some(started) = lookup_started {
            diagnostics::record_dialog_lookup(started.elapsed());
        }
        if let Some(dialog_id) = dialog_id {
            self.process_bye_in_dialog(transaction_id, request, dialog_id)
                .await
        } else {
            let _cleanup_operation =
                self.enter_bye_cleanup_operation()
                    .ok_or_else(|| DialogError::InvalidState {
                        expected: "running dialog manager".to_string(),
                        actual: "dialog manager is draining".to_string(),
                    })?;
            let tombstone_lookup_started = diagnostics::dialog_timing_enabled().then(Instant::now);
            let tombstone_match = matches_terminated_bye_retransmit(self, &request);
            if let Some(started) = tombstone_lookup_started {
                diagnostics::record_bye_tombstone_lookup(started.elapsed());
                diagnostics::record_bye_tombstone_observed_size(self.terminated_bye_lookup.len());
            }
            let status_code = if tombstone_match {
                debug!("BYE retransmit matched recently terminated dialog");
                diagnostics::record_duplicate_bye_tombstone_hit();
                StatusCode::Ok
            } else {
                diagnostics::record_duplicate_bye_tombstone_miss();
                StatusCode::CallOrTransactionDoesNotExist
            };

            // RFC 3261 §15.1.2: a BYE that does not match an existing dialog
            // gets a 481 Call/Transaction Does Not Exist. This happens in
            // normal operation when a peer retransmits a BYE past our dialog
            // teardown (e.g. its 200 OK was lost), so it is not an error.
            // Recently terminated BYEs keep a compact tombstone so late
            // retransmits still receive the original idempotent 200 OK.
            let cleanup_capacity =
                self.bye_cleanup_tasks
                    .reserve_capacity()
                    .await
                    .map_err(|_error| DialogError::InternalError {
                        message: "Failed to reserve BYE response recovery capacity".to_string(),
                        context: None,
                    })?;
            let prepared_recovery = self
                .prepare_bye_transaction_recovery(transaction_id.clone(), cleanup_capacity)
                .map_err(|_error| DialogError::InternalError {
                    message: "Failed to register BYE response recovery".to_string(),
                    context: None,
                })?;
            let response = response_builders::create_response(&request, status_code);
            let send_started = diagnostics::dialog_timing_enabled().then(Instant::now);
            if let (Some(handler_started), Some(send_started)) = (handler_started, send_started) {
                diagnostics::record_bye_path_handler_to_send_start(
                    send_started.duration_since(handler_started),
                );
            }
            let send_result = self
                .transaction_manager
                .send_response(&transaction_id, response)
                .await;
            if let Some(started) = send_started {
                diagnostics::record_bye_path_send_response(started.elapsed());
            }
            if send_result.is_err() {
                return Err(DialogError::TransactionError {
                    message: "Failed to send response to BYE".to_string(),
                });
            }
            prepared_recovery.disarm();
            if status_code == StatusCode::Ok {
                diagnostics::record_200_ok_bye_tombstone();
                self.record_bye_receive_to_200(&transaction_id);
            }

            debug!(
                "BYE processed with {} response (no dialog found)",
                status_code
            );
            Ok(())
        }
    }

    /// Process BYE within a dialog
    pub async fn process_bye_in_dialog(
        &self,
        transaction_id: TransactionKey,
        request: Request,
        dialog_id: DialogId,
    ) -> DialogResult<()> {
        let _cleanup_operation =
            self.enter_bye_cleanup_operation()
                .ok_or_else(|| DialogError::InvalidState {
                    expected: "running dialog manager".to_string(),
                    actual: "dialog manager is draining".to_string(),
                })?;
        debug!("Processing BYE for dialog {}", dialog_id);
        let handler_started = diagnostics::dialog_timing_enabled().then(Instant::now);

        // Recognize a terminated generation before waiting on cleanup
        // capacity. A different CSeq is not another teardown owner; it gets a
        // deterministic 481 while the original exact cleanup remains sole.
        let initial_disposition = match self.dialogs.get(&dialog_id) {
            Some(dialog) => classify_bye_sequence(dialog.value(), &request)?,
            None => {
                if matches_terminated_bye_retransmit(self, &request) {
                    ByeSequenceDisposition::DuplicateTerminated
                } else {
                    ByeSequenceDisposition::TerminatedMismatch
                }
            }
        };

        // Reserve bounded manager-owned cleanup capacity before changing the
        // dialog. Cancellation while this await is pending leaves the dialog
        // untouched; after it succeeds registration and fallback dispatch are
        // synchronous, so terminal cleanup cannot be lost.
        let cleanup_capacity =
            self.bye_cleanup_tasks
                .reserve_capacity()
                .await
                .map_err(|_error| DialogError::InternalError {
                    message: "Failed to reserve BYE cleanup capacity".to_string(),
                    context: None,
                })?;

        // Revalidate under the exact dialog cell after the asynchronous
        // reservation. Cleanup may have removed the dialog, or another BYE may
        // have terminated it, between lookup and this mutation.
        let mut sequence_error = None;
        let disposition = if initial_disposition == ByeSequenceDisposition::Fresh {
            match self.get_dialog_mut(&dialog_id) {
                Ok(mut dialog) => match classify_bye_sequence(&dialog, &request) {
                    Ok(ByeSequenceDisposition::Fresh) => {
                        match dialog.update_remote_sequence(&request) {
                            Ok(()) => {
                                dialog.terminate();
                                ByeSequenceDisposition::Fresh
                            }
                            Err(error) => {
                                sequence_error = Some(error);
                                ByeSequenceDisposition::TerminatedMismatch
                            }
                        }
                    }
                    Ok(disposition) => disposition,
                    Err(error) => {
                        sequence_error = Some(error);
                        ByeSequenceDisposition::TerminatedMismatch
                    }
                },
                Err(_) if matches_terminated_bye_retransmit(self, &request) => {
                    ByeSequenceDisposition::DuplicateTerminated
                }
                Err(_) => ByeSequenceDisposition::TerminatedMismatch,
            }
        } else {
            initial_disposition
        };

        if let Some(error) = sequence_error {
            let prepared_recovery = self
                .prepare_bye_transaction_recovery(transaction_id.clone(), cleanup_capacity)
                .map_err(|_error| DialogError::InternalError {
                    message: "Failed to register invalid BYE recovery".to_string(),
                    context: None,
                })?;
            drop(prepared_recovery);
            return Err(error);
        }

        let fresh = disposition == ByeSequenceDisposition::Fresh;
        let status_code = match disposition {
            ByeSequenceDisposition::Fresh | ByeSequenceDisposition::DuplicateTerminated => {
                StatusCode::Ok
            }
            ByeSequenceDisposition::TerminatedMismatch => StatusCode::CallOrTransactionDoesNotExist,
        };
        let prepared_cleanup = if fresh {
            self.session_refresh_tasks.begin_close_dialog(&dialog_id);
            self.reliable_provisional_tasks
                .begin_close_dialog(&dialog_id);
            self.prepare_bye_cleanup(transaction_id.clone(), dialog_id.clone(), cleanup_capacity)
                .map_err(|_error| DialogError::InternalError {
                    message: "Failed to register BYE cleanup".to_string(),
                    context: None,
                })?
        } else {
            self.prepare_bye_transaction_recovery(transaction_id.clone(), cleanup_capacity)
                .map_err(|_error| DialogError::InternalError {
                    message: "Failed to register BYE response recovery".to_string(),
                    context: None,
                })?
        };

        let response = response_builders::create_response(&request, status_code);
        let send_started = diagnostics::dialog_timing_enabled().then(Instant::now);
        if let (Some(handler_started), Some(send_started)) = (handler_started, send_started) {
            diagnostics::record_bye_path_handler_to_send_start(
                send_started.duration_since(handler_started),
            );
        }
        let send_result = self
            .transaction_manager
            .send_response(&transaction_id, response)
            .await;
        if let Some(started) = send_started {
            diagnostics::record_bye_path_send_response(started.elapsed());
        }
        if send_result.is_err() {
            // Dropping the prepared owner queues exact recovery: preserve
            // Timer J when the wire fence advanced, otherwise terminate the
            // pre-wire generation. Fresh-dialog cleanup runs in the same job.
            return Err(DialogError::TransactionError {
                message: format!("Failed to send {} response to BYE", status_code),
            });
        }

        if status_code == StatusCode::Ok {
            diagnostics::record_bye_200_sent();
            if fresh {
                diagnostics::record_200_ok_bye_fresh();
            } else {
                diagnostics::record_200_ok_bye_duplicate_terminated();
                diagnostics::record_duplicate_bye_terminated_dialog();
            }
            self.record_bye_receive_to_200(&transaction_id);
        }

        if fresh {
            prepared_cleanup.activate();
            info!("BYE processed for dialog {}", dialog_id);
        } else {
            prepared_cleanup.disarm();
            debug!(
                "BYE processed idempotently with {} for terminated dialog {}",
                status_code, dialog_id
            );
        }
        Ok(())
    }

    fn record_bye_receive_to_200(&self, transaction_id: &TransactionKey) {
        if let Some(timing) = self.transaction_manager.take_inbound_timing(transaction_id) {
            if let Some(received_at) = timing.received_at {
                diagnostics::record_bye_receive_to_200(received_at.elapsed());
            }
        }
    }

    fn record_bye_handler_entry(
        &self,
        transaction_id: &TransactionKey,
        handler_at: Option<Instant>,
    ) {
        let Some(handler_at) = handler_at else {
            return;
        };
        if let Some(timing) = self.transaction_manager.peek_inbound_timing(transaction_id) {
            if let Some(received_at) = timing.received_at {
                diagnostics::record_bye_path_udp_to_handler(handler_at.duration_since(received_at));
            }
            if let Some(transaction_manager_received_at) = timing.transaction_manager_received_at {
                diagnostics::record_bye_path_tx_received_to_handler(
                    handler_at.duration_since(transaction_manager_received_at),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialog::{Dialog, DialogState};
    use crate::manager::core::DialogManagerLifecycle;
    use crate::transaction::{TransactionManager, TransactionState};
    use rvoip_sip_core::builder::SimpleRequestBuilder;
    use rvoip_sip_core::{Message, Method, Request};
    use rvoip_sip_transport::transport::TransportType;
    use rvoip_sip_transport::{Transport, TransportEvent, TransportRoute};
    use std::net::SocketAddr;
    use tokio::sync::mpsc;

    #[derive(Debug)]
    struct NoopTransport {
        local_addr: SocketAddr,
        transport_type: TransportType,
        sent: Arc<StdMutex<Vec<Message>>>,
        fail_send: bool,
    }

    #[async_trait::async_trait]
    impl Transport for NoopTransport {
        async fn send_message(
            &self,
            message: Message,
            _destination: SocketAddr,
        ) -> Result<(), rvoip_sip_transport::Error> {
            if self.fail_send {
                return Err(rvoip_sip_transport::Error::TransportClosed);
            }
            self.sent
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(message);
            Ok(())
        }

        fn local_addr(&self) -> Result<SocketAddr, rvoip_sip_transport::Error> {
            Ok(self.local_addr)
        }

        async fn close(&self) -> Result<(), rvoip_sip_transport::Error> {
            Ok(())
        }

        fn is_closed(&self) -> bool {
            false
        }

        fn supports_tcp(&self) -> bool {
            true
        }

        fn default_transport_type(&self) -> TransportType {
            self.transport_type
        }
    }

    fn dialog_with_state(state: DialogState, remote_cseq: u32) -> Dialog {
        let mut dialog = Dialog::new(
            "bye-sequence-test".to_string(),
            "sip:alice@example.com".parse().unwrap(),
            "sip:bob@example.com".parse().unwrap(),
            Some("alice-tag".to_string()),
            Some("bob-tag".to_string()),
            true,
        );
        dialog.state = state;
        dialog.remote_cseq = remote_cseq;
        dialog
    }

    fn bye_request(cseq: u32) -> Request {
        bye_request_with_branch(cseq, "z9hG4bK-bye-sequence")
    }

    fn bye_request_with_branch(cseq: u32, branch: &str) -> Request {
        SimpleRequestBuilder::new(Method::Bye, "sip:alice@example.com")
            .unwrap()
            .from("Bob", "sip:bob@example.com", Some("bob-tag"))
            .to("Alice", "sip:alice@example.com", Some("alice-tag"))
            .call_id("bye-sequence-test")
            .cseq(cseq)
            .via("127.0.0.1:5060", "UDP", Some(branch))
            .max_forwards(70)
            .build()
    }

    async fn install_test_cleanup(
        registry: Arc<ByeCleanupTaskRegistry>,
        dialog_id: DialogId,
        gate: Option<Arc<tokio::sync::Semaphore>>,
    ) -> PreparedByeCleanup {
        let capacity = registry
            .reserve_capacity()
            .await
            .expect("test cleanup capacity");
        let cleanup = Box::pin(async move {
            if let Some(gate) = gate {
                gate.acquire().await.expect("test cleanup release").forget();
            }
            true
        });
        Arc::clone(&registry)
            .register(
                ByeCleanupKey::Dialog(dialog_id),
                capacity,
                cleanup,
                Arc::new(AtomicBool::new(false)),
            )
            .expect("test cleanup registration")
    }

    #[tokio::test]
    async fn close_all_observes_prepared_cleanup_before_activation() {
        let registry = ByeCleanupTaskRegistry::with_capacity(2);
        let dialog_id = DialogId::new();
        let prepared = install_test_cleanup(Arc::clone(&registry), dialog_id, None).await;
        let stop_registry = Arc::clone(&registry);
        let close = tokio::spawn(async move { stop_registry.close_all().await });
        tokio::task::yield_now().await;
        assert_eq!(registry.len(), 1);
        prepared.activate();
        close
            .await
            .expect("close task joined")
            .expect("prepared cleanup drained");
        assert_eq!(registry.len(), 0);
    }

    #[tokio::test]
    async fn preclose_capacity_permit_authorizes_registration_and_close_wakes_waiters() {
        let registry = ByeCleanupTaskRegistry::with_capacity(1);
        let admitted = registry
            .reserve_capacity()
            .await
            .expect("pre-close admission");
        let waiting_registry = Arc::clone(&registry);
        let waiting = tokio::spawn(async move { waiting_registry.reserve_capacity().await });
        tokio::task::yield_now().await;

        registry.begin_close();
        assert!(matches!(
            waiting.await.expect("capacity waiter joined"),
            Err(ByeCleanupError::RegistryClosed)
        ));

        let prepared = Arc::clone(&registry)
            .register(
                ByeCleanupKey::Dialog(DialogId::new()),
                admitted,
                Box::pin(async { true }),
                Arc::new(AtomicBool::new(false)),
            )
            .expect("pre-close permit remains exact registration authority");
        prepared.activate();
        registry
            .close_all()
            .await
            .expect("already-admitted cleanup drains after close");
    }

    #[tokio::test]
    async fn failed_cleanup_is_quarantined_and_never_reports_later_success() {
        let registry = ByeCleanupTaskRegistry::with_capacity(1);
        let capacity = registry.reserve_capacity().await.expect("cleanup capacity");
        Arc::clone(&registry)
            .register(
                ByeCleanupKey::Dialog(DialogId::new()),
                capacity,
                Box::pin(async { false }),
                Arc::new(AtomicBool::new(false)),
            )
            .expect("cleanup registration")
            .activate();
        tokio::task::yield_now().await;

        assert_eq!(registry.close_all().await, Err(ByeCleanupError::TaskFailed));
        assert_eq!(registry.close_all().await, Err(ByeCleanupError::TaskFailed));
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.quarantined.len(), 1);
        assert_eq!(registry.cleanup_jobs_completed.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn stale_bye_cleanup_completion_cannot_remove_replacement() {
        let registry = ByeCleanupTaskRegistry::with_capacity(2);
        let dialog_id = DialogId::new();
        let prepared = install_test_cleanup(Arc::clone(&registry), dialog_id.clone(), None).await;
        let key = ByeCleanupKey::Dialog(dialog_id);
        let live_token = *registry.tasks.get(&key).expect("live token");
        registry.remove_exact(&key, live_token.wrapping_add(1));
        assert_eq!(registry.len(), 1);
        prepared.activate();
        registry.close_all().await.expect("replacement drained");
    }

    #[tokio::test]
    async fn blocked_bye_cleanup_is_retained_and_retryable_after_deadline() {
        let registry = ByeCleanupTaskRegistry::with_capacity(2);
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let prepared = install_test_cleanup(
            Arc::clone(&registry),
            DialogId::new(),
            Some(Arc::clone(&gate)),
        )
        .await;
        prepared.activate();
        assert_eq!(
            registry.close_all().await,
            Err(ByeCleanupError::CompletionTimeout)
        );
        assert_eq!(registry.len(), 1);
        gate.add_permits(1);
        registry
            .close_all()
            .await
            .expect("explicit retry re-audits drained registry");
    }

    #[tokio::test]
    async fn many_blocked_bye_cleanups_share_one_grace_deadline_and_remain_owned() {
        let registry = ByeCleanupTaskRegistry::with_capacity(32);
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        for _ in 1..=32 {
            install_test_cleanup(
                Arc::clone(&registry),
                DialogId::new(),
                Some(Arc::clone(&gate)),
            )
            .await
            .activate();
        }

        let started = tokio::time::Instant::now();
        assert_eq!(
            registry.close_all().await,
            Err(ByeCleanupError::CompletionTimeout)
        );
        assert!(
            started.elapsed() < Duration::from_millis(300),
            "batch close exceeded shared-deadline bound: {:?}",
            started.elapsed()
        );
        assert_eq!(registry.len(), 32);
        gate.add_permits(32);
        registry
            .close_all()
            .await
            .expect("retry after joined batch");
    }

    #[tokio::test]
    async fn cleanup_jobs_share_fixed_worker_pool_and_complete_exactly_once() {
        let registry = ByeCleanupTaskRegistry::with_capacity(8);
        let completed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        for _ in 0..8 {
            let capacity = registry.reserve_capacity().await.expect("cleanup capacity");
            let completed = Arc::clone(&completed);
            let cleanup = Box::pin(async move {
                completed.fetch_add(1, Ordering::Relaxed);
                true
            });
            Arc::clone(&registry)
                .register(
                    ByeCleanupKey::Dialog(DialogId::new()),
                    capacity,
                    cleanup,
                    Arc::new(AtomicBool::new(false)),
                )
                .expect("cleanup registration")
                .activate();
        }

        registry.close_all().await.expect("cleanup queue drained");
        assert_eq!(completed.load(Ordering::Relaxed), 8);
        assert_eq!(registry.len(), 0);
        assert_eq!(registry.worker_tasks_spawned.load(Ordering::Relaxed), 4);
        assert_eq!(registry.cleanup_jobs_enqueued.load(Ordering::Relaxed), 8);
        assert_eq!(registry.cleanup_jobs_completed.load(Ordering::Relaxed), 8);
    }

    async fn test_dialog_manager(
        transport_type: TransportType,
    ) -> (
        Arc<DialogManager>,
        Arc<StdMutex<Vec<Message>>>,
        mpsc::Receiver<crate::transaction::TransactionEvent>,
    ) {
        let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
        let sent = Arc::new(StdMutex::new(Vec::new()));
        let transport = Arc::new(NoopTransport {
            local_addr,
            transport_type,
            sent: Arc::clone(&sent),
            fail_send: false,
        });
        let (_transport_tx, transport_rx) = mpsc::channel::<TransportEvent>(64);
        let (transaction_manager, events) =
            TransactionManager::new(transport, transport_rx, Some(64))
                .await
                .expect("transaction manager");
        let manager = Arc::new(
            DialogManager::new(Arc::new(transaction_manager), local_addr)
                .await
                .expect("dialog manager"),
        );
        (manager, sent, events)
    }

    async fn wait_for_bye_cleanup(manager: &DialogManager, dialog_id: &DialogId) {
        tokio::time::timeout(Duration::from_secs(1), async {
            while manager.has_dialog(dialog_id) || manager.bye_cleanup_tasks.len() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("BYE cleanup queue converged");
    }

    #[tokio::test]
    async fn fresh_udp_bye_keeps_natural_compact_timer_j_and_uses_no_per_bye_task() {
        let (manager, sent, _events) = test_dialog_manager(TransportType::Udp).await;
        let dialog = dialog_with_state(DialogState::Confirmed, 1);
        let dialog_id = dialog.id.clone();
        manager.store_dialog(dialog).await.expect("store dialog");

        let request = bye_request(2);
        let route = TransportRoute::new("127.0.0.1:5090".parse().unwrap())
            .with_transport_type(TransportType::Udp);
        let transaction = manager
            .transaction_manager
            .create_server_transaction_on_route(request.clone(), route)
            .await
            .expect("BYE server transaction");
        let transaction_id = transaction.id().clone();
        manager
            .process_bye_in_dialog(transaction_id.clone(), request, dialog_id.clone())
            .await
            .expect("BYE response path");
        wait_for_bye_cleanup(&manager, &dialog_id).await;

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if matches!(
                    manager
                        .transaction_manager
                        .transaction_state(&transaction_id)
                        .await,
                    Ok(TransactionState::Completed)
                ) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("UDP BYE entered compact Timer J");
        assert!(
            manager
                .transaction_manager
                .transaction_exists(&transaction_id)
                .await,
            "UDP BYE must remain replayable for Timer J"
        );
        assert_eq!(
            manager
                .bye_cleanup_tasks
                .worker_tasks_spawned
                .load(Ordering::Relaxed),
            4
        );
        assert_eq!(
            manager
                .bye_cleanup_tasks
                .cleanup_jobs_enqueued
                .load(Ordering::Relaxed),
            1
        );
        assert_eq!(
            manager
                .bye_cleanup_tasks
                .cleanup_jobs_completed
                .load(Ordering::Relaxed),
            1
        );
        let sent = sent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(
            matches!(sent.as_slice(), [Message::Response(response)] if response.status() == StatusCode::Ok)
        );
    }

    #[tokio::test]
    async fn reliable_bye_uses_zero_timer_j_and_cleanup_queue_still_converges() {
        let (manager, _sent, _events) = test_dialog_manager(TransportType::Tcp).await;
        let dialog = dialog_with_state(DialogState::Confirmed, 1);
        let dialog_id = dialog.id.clone();
        manager.store_dialog(dialog).await.expect("store dialog");

        let request = bye_request(2);
        let route = TransportRoute::new("127.0.0.1:5090".parse().unwrap())
            .with_transport_type(TransportType::Tcp);
        let transaction = manager
            .transaction_manager
            .create_server_transaction_on_route(request.clone(), route)
            .await
            .expect("BYE server transaction");
        let transaction_id = transaction.id().clone();
        manager
            .process_bye_in_dialog(transaction_id.clone(), request, dialog_id.clone())
            .await
            .expect("BYE response path");
        wait_for_bye_cleanup(&manager, &dialog_id).await;

        tokio::time::timeout(Duration::from_secs(1), async {
            while manager
                .transaction_manager
                .transaction_exists(&transaction_id)
                .await
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("reliable BYE zero Timer J converged");
        assert_eq!(
            manager
                .bye_cleanup_tasks
                .worker_tasks_spawned
                .load(Ordering::Relaxed),
            4
        );
        assert_eq!(
            manager
                .bye_cleanup_tasks
                .cleanup_jobs_completed
                .load(Ordering::Relaxed),
            1
        );
    }

    #[tokio::test]
    async fn wire_unknown_bye_response_preserves_timer_j_and_keeps_cleanup_exact() {
        let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
        let transport = Arc::new(NoopTransport {
            local_addr,
            transport_type: TransportType::Udp,
            sent: Arc::new(StdMutex::new(Vec::new())),
            fail_send: true,
        });
        let (_transport_tx, transport_rx) = mpsc::channel::<TransportEvent>(64);
        let (transaction_manager, _events) =
            TransactionManager::new(transport, transport_rx, Some(64))
                .await
                .expect("transaction manager");
        let manager = Arc::new(
            DialogManager::new(Arc::new(transaction_manager), local_addr)
                .await
                .expect("dialog manager"),
        );
        let dialog = dialog_with_state(DialogState::Confirmed, 1);
        let dialog_id = dialog.id.clone();
        manager.store_dialog(dialog).await.expect("store dialog");

        let request = bye_request(2);
        let transaction = manager
            .transaction_manager
            .create_server_transaction(request.clone(), "127.0.0.1:5090".parse().unwrap())
            .await
            .expect("BYE server transaction");
        let transaction_id = transaction.id().clone();
        manager
            .process_bye_in_dialog(transaction_id.clone(), request, dialog_id.clone())
            .await
            .expect_err("transport failure must reach caller");
        wait_for_bye_cleanup(&manager, &dialog_id).await;

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if matches!(
                    manager
                        .transaction_manager
                        .transaction_state(&transaction_id)
                        .await,
                    Ok(TransactionState::Completed)
                ) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("wire-unknown response entered compact Timer J");
        assert!(
            manager
                .transaction_manager
                .transaction_exists(&transaction_id)
                .await,
            "a write-boundary error must retain replay ownership"
        );
        assert_eq!(
            manager
                .bye_cleanup_tasks
                .cleanup_jobs_enqueued
                .load(Ordering::Relaxed),
            1
        );
        assert_eq!(
            manager
                .bye_cleanup_tasks
                .cleanup_jobs_completed
                .load(Ordering::Relaxed),
            1
        );
    }

    #[tokio::test]
    async fn terminated_dialog_tombstone_replies_200_without_duplicate_cleanup() {
        let (manager, sent, _events) = test_dialog_manager(TransportType::Udp).await;
        let dialog = dialog_with_state(DialogState::Confirmed, 1);
        let dialog_id = dialog.id.clone();
        manager.store_dialog(dialog).await.expect("store dialog");
        let peer: SocketAddr = "127.0.0.1:5090".parse().unwrap();

        let first = bye_request_with_branch(2, "z9hG4bK-bye-first");
        let first_transaction = manager
            .transaction_manager
            .create_server_transaction_on_route(
                first.clone(),
                TransportRoute::new(peer).with_transport_type(TransportType::Udp),
            )
            .await
            .expect("first BYE transaction");
        manager
            .process_bye_in_dialog(first_transaction.id().clone(), first, dialog_id.clone())
            .await
            .expect("first BYE");
        wait_for_bye_cleanup(&manager, &dialog_id).await;

        let duplicate = bye_request_with_branch(2, "z9hG4bK-bye-retry");
        let duplicate_transaction = manager
            .transaction_manager
            .create_server_transaction_on_route(
                duplicate.clone(),
                TransportRoute::new(peer).with_transport_type(TransportType::Udp),
            )
            .await
            .expect("duplicate BYE transaction");
        manager
            .handle_bye_with_transaction(duplicate_transaction.id().clone(), duplicate)
            .await
            .expect("tombstone BYE response");

        assert_eq!(
            manager
                .bye_cleanup_tasks
                .cleanup_jobs_enqueued
                .load(Ordering::Relaxed),
            1
        );
        assert_eq!(
            manager
                .bye_cleanup_tasks
                .cleanup_jobs_completed
                .load(Ordering::Relaxed),
            1
        );
        let response_statuses = sent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter_map(|message| match message {
                Message::Response(response) => Some(response.status()),
                Message::Request(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(response_statuses, vec![StatusCode::Ok, StatusCode::Ok]);
    }

    #[tokio::test]
    async fn blocked_real_bye_cleanup_remains_owned_and_delivers_exact_event_on_retry() {
        let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
        let transport = Arc::new(NoopTransport {
            local_addr,
            transport_type: TransportType::Udp,
            sent: Arc::new(StdMutex::new(Vec::new())),
            fail_send: false,
        });
        let (_transport_tx, transport_rx) = mpsc::channel::<TransportEvent>(8);
        let (transaction_manager, _events) =
            TransactionManager::new(transport, transport_rx, Some(8))
                .await
                .expect("transaction manager");
        let manager = Arc::new(
            DialogManager::new(Arc::new(transaction_manager), local_addr)
                .await
                .expect("dialog manager"),
        );

        let mut dialog = dialog_with_state(DialogState::Confirmed, 1);
        let dialog_id = dialog.id.clone();
        dialog.session_expires_secs = Some(120);
        dialog.is_session_refresher = true;
        manager.store_dialog(dialog).await.expect("store dialog");
        crate::manager::session_timer::spawn_refresh_task(
            (*manager).clone(),
            dialog_id.clone(),
            120,
            true,
        )
        .await
        .expect("refresh producer");
        assert_eq!(manager.session_refresh_tasks.len(), 1);

        let (session_tx, mut session_rx) = mpsc::channel(1);
        session_tx
            .send(SessionCoordinationEvent::ByeReceived {
                dialog_id: DialogId::new(),
            })
            .await
            .expect("prefill session channel");
        manager
            .session_to_dialog
            .insert("blocked-cleanup-session".to_string(), dialog_id.clone());
        manager
            .dialog_to_session
            .insert(dialog_id.clone(), "blocked-cleanup-session".to_string());
        *manager.session_coordinator.write().await = Some(session_tx);

        let request = bye_request(2);
        let source: SocketAddr = "127.0.0.1:5090".parse().unwrap();
        let server_transaction = manager
            .transaction_manager
            .create_server_transaction(request.clone(), source)
            .await
            .expect("BYE server transaction");
        manager
            .process_bye_in_dialog(server_transaction.id().clone(), request, dialog_id.clone())
            .await
            .expect("BYE 200 path");

        let first_stop = tokio::time::timeout(Duration::from_secs(1), manager.stop())
            .await
            .expect("first stop bounded")
            .expect_err("blocked cleanup must fail before ShutdownComplete");
        assert!(matches!(first_stop, DialogError::InternalError { .. }));
        assert_eq!(manager.lifecycle(), DialogManagerLifecycle::Draining);
        assert!(manager.has_dialog(&dialog_id));
        assert_eq!(manager.bye_cleanup_tasks.len(), 1);

        let _prefill = session_rx.try_recv().expect("prefill remains queued");
        let delivered = tokio::time::timeout(Duration::from_secs(1), session_rx.recv())
            .await
            .expect("retained cleanup resumed")
            .expect("session receiver remains open");
        assert!(matches!(
            delivered,
            SessionCoordinationEvent::ByeReceived { dialog_id: delivered_id }
                if delivered_id == dialog_id
        ));
        wait_for_bye_cleanup(&manager, &dialog_id).await;

        tokio::time::timeout(Duration::from_secs(1), manager.stop())
            .await
            .expect("retry stop bounded")
            .expect("retry re-audits and completes");
        assert_eq!(manager.lifecycle(), DialogManagerLifecycle::Stopped);
        assert!(!manager.has_dialog(&dialog_id));
        assert_eq!(manager.bye_cleanup_tasks.len(), 0);
        assert_eq!(manager.session_refresh_tasks.len(), 0);
        assert_eq!(manager.reliable_provisional_tasks.len(), 0);

        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(matches!(
            session_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected)
        ));
    }

    #[test]
    fn duplicate_bye_on_terminated_dialog_is_idempotent() {
        let dialog = dialog_with_state(DialogState::Terminated, 2);
        let request = bye_request(2);

        assert_eq!(
            classify_bye_sequence(&dialog, &request).unwrap(),
            ByeSequenceDisposition::DuplicateTerminated
        );
    }

    #[test]
    fn same_cseq_bye_on_confirmed_dialog_remains_fresh_for_strict_validation() {
        let dialog = dialog_with_state(DialogState::Confirmed, 2);
        let request = bye_request(2);

        assert_eq!(
            classify_bye_sequence(&dialog, &request).unwrap(),
            ByeSequenceDisposition::Fresh
        );
    }

    #[test]
    fn missing_cseq_bye_still_fails_protocol_validation() {
        let dialog = dialog_with_state(DialogState::Terminated, 2);
        let request = Request::new(Method::Bye, "sip:alice@example.com".parse().unwrap());

        assert!(classify_bye_sequence(&dialog, &request).is_err());
    }
}
