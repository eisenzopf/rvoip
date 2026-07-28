//! Retired dialog-owned RFC 4028 scheduler compatibility surface.
//!
//! Dialog-core remains responsible for parsing and negotiating session-timer
//! headers. Scheduling, fallback decisions and lifecycle termination are owned
//! by rvoip-sip's generation-qualified session lane. The old public spawn
//! facade therefore fails closed and this registry remains empty in production;
//! its cancellation shape is retained so existing manager teardown signatures
//! do not change.

use std::collections::HashSet;
use std::fmt;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use dashmap::DashMap;
use tokio::sync::{watch, Mutex};
use tokio::task::AbortHandle;
use tracing::debug;

use crate::dialog::DialogId;
use crate::manager::core::DialogManager;

const REFRESH_TASK_COMPLETION_TIMEOUT: Duration = Duration::from_secs(1);

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

#[cfg(test)]
struct SessionRefreshTaskCompletion {
    registry: Arc<SessionRefreshTaskRegistry>,
    dialog_id: DialogId,
    token: u64,
    completion: Option<watch::Sender<bool>>,
}

#[cfg(test)]
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

/// Retired compatibility facade.
///
/// RFC 4028 lifecycle work must be admitted by rvoip-sip's exact session
/// authority. Calling this lower-layer facade can provide neither that exact
/// lifetime nor the YAML transition capability, so it always fails closed.
pub async fn spawn_refresh_task(
    _manager: DialogManager,
    _dialog_id: DialogId,
    _interval_secs: u32,
    _is_refresher: bool,
) -> Result<(), SessionRefreshTaskError> {
    Err(SessionRefreshTaskError::RegistryClosed)
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

    #[test]
    fn retired_module_has_no_refresh_wire_driver() {
        let source = include_str!("session_timer.rs");
        for forbidden in [
            concat!(".send_", "request("),
            concat!("send_bye_", "with_reason("),
            concat!("tokio::time::", "sleep("),
            concat!("SessionCoordinationEvent::", "SessionRefreshed"),
        ] {
            assert!(
                !source.contains(forbidden),
                "retired dialog timer regained a signaling hot path: {forbidden}"
            );
        }
    }
}
