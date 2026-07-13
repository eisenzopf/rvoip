use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{oneshot, Mutex};
use tokio::task::{AbortHandle, JoinHandle};

use crate::error::{Error, Result};

/// Admission policy for unauthenticated TLS and WebSocket handshakes.
///
/// The limit applies before accepting another TCP socket, leaving excess
/// connections in the kernel backlog instead of allocating an unbounded task
/// and userspace buffers for each slow peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandshakeAdmissionConfig {
    /// Maximum time allowed for the complete transport handshake. For WSS this
    /// includes both TLS and the HTTP WebSocket upgrade.
    pub timeout: Duration,
    /// Maximum number of handshakes concurrently admitted by one listener.
    pub max_concurrent: usize,
}

impl HandshakeAdmissionConfig {
    /// Construct an explicit handshake admission policy.
    pub const fn new(timeout: Duration, max_concurrent: usize) -> Self {
        Self {
            timeout,
            max_concurrent,
        }
    }

    pub(crate) fn validate(self, transport: &str) -> Result<Self> {
        if self.timeout.is_zero() {
            return Err(Error::InvalidState(format!(
                "{transport} handshake timeout must be greater than zero"
            )));
        }
        if self.max_concurrent == 0 {
            return Err(Error::InvalidState(format!(
                "{transport} concurrent handshake limit must be greater than zero"
            )));
        }
        Ok(self)
    }
}

impl Default for HandshakeAdmissionConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(10),
            max_concurrent: 128,
        }
    }
}

/// Join-handle owner shared by connection-oriented transports.
///
/// Every task is registered before it starts. `close()` first prevents new
/// registrations, then aborts and joins every registered task. This makes the
/// post-close boundary deterministic: listeners and streams have been dropped
/// and no managed task can emit another transport event.
pub(crate) struct TransportTaskSet {
    closing: AtomicBool,
    next_id: AtomicU64,
    tasks: Mutex<HashMap<u64, JoinHandle<()>>>,
    close_gate: Mutex<()>,
}

impl TransportTaskSet {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            closing: AtomicBool::new(false),
            next_id: AtomicU64::new(1),
            tasks: Mutex::new(HashMap::new()),
            close_gate: Mutex::new(()),
        })
    }

    pub(crate) fn is_closing(&self) -> bool {
        self.closing.load(Ordering::Acquire)
    }

    /// Register and start one managed task. Returns `None` after shutdown has
    /// begun; in that case the supplied future is dropped without being run.
    pub(crate) async fn spawn<F>(self: &Arc<Self>, future: F) -> Option<AbortHandle>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (start_tx, start_rx) = oneshot::channel();
        let weak_tasks = Arc::downgrade(self);
        let handle = tokio::spawn(async move {
            if start_rx.await.is_err() {
                return;
            }
            future.await;
            if let Some(tasks) = weak_tasks.upgrade() {
                tasks.tasks.lock().await.remove(&id);
            }
        });
        let abort_handle = handle.abort_handle();

        let mut tasks = self.tasks.lock().await;
        if self.is_closing() {
            drop(tasks);
            handle.abort();
            let _ = handle.await;
            return None;
        }
        tasks.retain(|_, task| !task.is_finished());
        tasks.insert(id, handle);
        drop(tasks);

        // The receiver cannot disappear before the task is started or
        // aborted. Treat a failed send like a rejected registration.
        if start_tx.send(()).is_err() {
            abort_handle.abort();
            return None;
        }
        Some(abort_handle)
    }

    /// Idempotently stop and join all managed tasks.
    pub(crate) async fn close(&self) {
        let _close_guard = self.close_gate.lock().await;
        self.closing.store(true, Ordering::Release);

        loop {
            let handles: Vec<_> = {
                let mut tasks = self.tasks.lock().await;
                tasks.drain().map(|(_, handle)| handle).collect()
            };
            if handles.is_empty() {
                break;
            }
            for handle in &handles {
                handle.abort();
            }
            for handle in handles {
                let _ = handle.await;
            }
        }
    }
}

impl Drop for TransportTaskSet {
    fn drop(&mut self) {
        if let Ok(mut tasks) = self.tasks.try_lock() {
            for (_, handle) in tasks.drain() {
                handle.abort();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_admission_rejects_zero_values() {
        assert!(HandshakeAdmissionConfig::new(Duration::ZERO, 1)
            .validate("test")
            .is_err());
        assert!(HandshakeAdmissionConfig::new(Duration::from_secs(1), 0)
            .validate("test")
            .is_err());
    }

    #[tokio::test]
    async fn task_set_close_is_idempotent_and_joins_tasks() {
        let tasks = TransportTaskSet::new();
        let (dropped_tx, dropped_rx) = oneshot::channel::<()>();
        struct DropSignal(Option<oneshot::Sender<()>>);
        impl Drop for DropSignal {
            fn drop(&mut self) {
                if let Some(sender) = self.0.take() {
                    let _ = sender.send(());
                }
            }
        }

        let signal = DropSignal(Some(dropped_tx));
        tasks
            .spawn(async move {
                let _signal = signal;
                std::future::pending::<()>().await;
            })
            .await
            .expect("task admitted");
        tasks.close().await;
        dropped_rx
            .await
            .expect("task future dropped before close returned");
        tasks.close().await;
        assert!(tasks.spawn(async {}).await.is_none());
    }
}
