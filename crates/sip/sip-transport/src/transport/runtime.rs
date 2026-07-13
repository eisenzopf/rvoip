use std::collections::HashMap;
use std::future::Future;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{oneshot, Mutex, OwnedMutexGuard, OwnedSemaphorePermit, Semaphore};
use tokio::task::{AbortHandle, JoinHandle};

use crate::error::{Error, Result};

/// Admission policy for TLS and WebSocket handshakes.
///
/// Inbound transports apply the limit before accepting another TCP socket,
/// leaving excess connections in the kernel backlog. Outbound transports use
/// the same policy for a global dial budget plus per-destination singleflight.
/// Bidirectional transports keep independent inbound and outbound budgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandshakeAdmissionConfig {
    /// Maximum time allowed for the complete transport handshake. For WSS this
    /// includes both TLS and the HTTP WebSocket upgrade.
    pub timeout: Duration,
    /// Maximum number of handshakes concurrently admitted in one direction.
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

/// Bounded admission and single-flight coordination for outbound handshakes.
///
/// A fixed set of destination locks avoids an attacker-controlled map whose
/// keys grow with every dial target. Hash collisions only serialize unrelated
/// destinations; they never permit two handshakes to the same destination.
pub(crate) struct OutboundHandshakeAdmission {
    global: Arc<Semaphore>,
    destination_locks: Box<[Arc<Mutex<()>>]>,
}

pub(crate) struct OutboundHandshakePermit {
    _destination: OwnedMutexGuard<()>,
    _global: OwnedSemaphorePermit,
}

impl OutboundHandshakeAdmission {
    const DESTINATION_LOCK_STRIPES: usize = 256;

    pub(crate) fn new(max_concurrent: usize) -> Arc<Self> {
        let destination_locks = (0..Self::DESTINATION_LOCK_STRIPES)
            .map(|_| Arc::new(Mutex::new(())))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Arc::new(Self {
            global: Arc::new(Semaphore::new(max_concurrent)),
            destination_locks,
        })
    }

    pub(crate) async fn acquire(&self, destination: SocketAddr) -> Result<OutboundHandshakePermit> {
        let mut hasher = DefaultHasher::new();
        destination.hash(&mut hasher);
        let index = hasher.finish() as usize % self.destination_locks.len();

        // Acquire destination ownership first so duplicate dials cannot occupy
        // all global permits while waiting behind their leader.
        let destination = self.destination_locks[index].clone().lock_owned().await;
        let global = self
            .global
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| Error::TransportClosed)?;
        Ok(OutboundHandshakePermit {
            _destination: destination,
            _global: global,
        })
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

    /// Run an operation as a managed task and return its result.
    ///
    /// This is used for caller-awaited connection establishment. The work is
    /// still owned by the transport, so `close()` aborts and joins it and the
    /// waiting caller deterministically receives `TransportClosed`.
    pub(crate) async fn run<F, T>(self: &Arc<Self>, future: F) -> Result<T>
    where
        F: Future<Output = Result<T>> + Send + 'static,
        T: Send + 'static,
    {
        let (result_tx, result_rx) = oneshot::channel();
        if self
            .spawn(async move {
                let _ = result_tx.send(future.await);
            })
            .await
            .is_none()
        {
            return Err(Error::TransportClosed);
        }
        result_rx.await.unwrap_or(Err(Error::TransportClosed))
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

    #[tokio::test]
    async fn managed_result_is_cancelled_by_close() {
        let tasks = TransportTaskSet::new();
        let runner = {
            let tasks = tasks.clone();
            tokio::spawn(async move {
                tasks
                    .run(async {
                        std::future::pending::<()>().await;
                        Ok::<_, Error>(())
                    })
                    .await
            })
        };
        tokio::task::yield_now().await;
        tasks.close().await;
        assert!(matches!(runner.await.unwrap(), Err(Error::TransportClosed)));
    }

    #[tokio::test]
    async fn outbound_admission_serializes_one_destination() {
        let admission = OutboundHandshakeAdmission::new(2);
        let first = admission
            .acquire("127.0.0.1:5061".parse().unwrap())
            .await
            .unwrap();
        let blocked = {
            let admission = admission.clone();
            tokio::spawn(async move { admission.acquire("127.0.0.1:5061".parse().unwrap()).await })
        };
        tokio::task::yield_now().await;
        assert!(!blocked.is_finished());
        drop(first);
        assert!(blocked.await.unwrap().is_ok());
    }
}
