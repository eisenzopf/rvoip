use std::collections::HashMap;
use std::future::Future;
use std::hash::Hash;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, Weak};
use std::time::Duration;

use tokio::sync::{oneshot, watch, Mutex, OwnedSemaphorePermit, Semaphore};
use tokio::task::{AbortHandle, JoinHandle};
use tokio::time::Instant;

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

/// Direction is part of every connection-pool key. An inbound flow may be a
/// valid route for a response, but it must never satisfy an outbound dial for a
/// different authenticated authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ConnectionDirection {
    Inbound,
    Outbound,
}

/// Resource policy derived from the public handshake policy.
///
/// Keeping these values derived preserves the existing public constructor
/// surface while adding hard bounds to every resource created by a dial. A
/// short handshake timeout in deterministic tests scales the idle/lifetime
/// windows down without changing production defaults.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ConnectionLifecycleConfig {
    pub(crate) max_pending_dials: usize,
    pub(crate) max_established_per_direction: usize,
    pub(crate) idle_timeout: Duration,
    pub(crate) authentication_lifetime: Duration,
    pub(crate) failure_backoff: Duration,
    pub(crate) write_timeout: Duration,
    pub(crate) writer_queue_capacity: usize,
}

impl ConnectionLifecycleConfig {
    pub(crate) fn from_handshake(handshake: HandshakeAdmissionConfig) -> Self {
        let idle_timeout = handshake.timeout.checked_mul(30).unwrap_or(Duration::MAX);
        let authentication_lifetime = idle_timeout.checked_mul(12).unwrap_or(Duration::MAX);
        Self {
            max_pending_dials: handshake.max_concurrent.saturating_mul(4).max(1),
            // This is a distinct semaphore from handshake admission. A peer
            // cannot recycle one handshake permit into unlimited live flows.
            max_established_per_direction: handshake.max_concurrent,
            idle_timeout,
            authentication_lifetime,
            failure_backoff: handshake.timeout.min(Duration::from_millis(250)),
            write_timeout: handshake.timeout,
            writer_queue_capacity: handshake.max_concurrent.clamp(16, 1_024),
        }
    }

    pub(crate) fn next_deadline(&self, last_activity: Instant, established_at: Instant) -> Instant {
        (last_activity + self.idle_timeout).min(established_at + self.authentication_lifetime)
    }
}

static NEXT_TRUST_CONTEXT: AtomicU64 = AtomicU64::new(1);

/// Allocate an opaque identity for one configured TLS/plaintext trust context.
/// It is intentionally process-local and diagnostic-free: pool equality, not
/// serialization, is its only purpose.
pub(crate) fn next_trust_context() -> u64 {
    NEXT_TRUST_CONTEXT.fetch_add(1, Ordering::Relaxed)
}

/// Cloneable, redacted dial failure shared by every singleflight follower.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SharedDialFailure {
    TransportClosed,
    Capacity,
    Timeout(SocketAddr),
    Connect(SocketAddr, io::ErrorKind),
    TlsHandshake,
    TlsCertificate,
    WebSocketHandshake,
    Cancelled,
    Other,
}

impl SharedDialFailure {
    pub(crate) fn capture(error: &Error) -> Self {
        match error {
            Error::TransportClosed => Self::TransportClosed,
            Error::ConnectionPoolExhausted | Error::ConnectionLimitReached => Self::Capacity,
            Error::ConnectionTimeout(address) => Self::Timeout(*address),
            Error::ConnectFailed(address, error) => Self::Connect(*address, error.kind()),
            Error::TlsHandshakeFailed(_) => Self::TlsHandshake,
            Error::TlsCertificateError(_) => Self::TlsCertificate,
            Error::WebSocketHandshakeFailed(_) => Self::WebSocketHandshake,
            _ => Self::Other,
        }
    }

    pub(crate) fn into_error(self) -> Error {
        match self {
            Self::TransportClosed => Error::TransportClosed,
            Self::Capacity => Error::ConnectionPoolExhausted,
            Self::Timeout(address) => Error::ConnectionTimeout(address),
            Self::Connect(address, kind) => {
                Error::ConnectFailed(address, io::Error::new(kind, "shared outbound dial failed"))
            }
            Self::TlsHandshake => {
                Error::TlsHandshakeFailed("shared outbound TLS dial failed".into())
            }
            Self::TlsCertificate => {
                Error::TlsCertificateError("shared outbound TLS identity check failed".into())
            }
            Self::WebSocketHandshake => {
                Error::WebSocketHandshakeFailed("shared outbound WebSocket dial failed".into())
            }
            Self::Cancelled => Error::Other("outbound dial leader was cancelled".into()),
            Self::Other => Error::Other("shared outbound dial failed".into()),
        }
    }
}

pub(crate) type SharedDialOutcome = std::result::Result<(), SharedDialFailure>;

pub(crate) struct InFlightDial {
    outcome: watch::Sender<Option<SharedDialOutcome>>,
}

enum DialState {
    InFlight(Arc<InFlightDial>),
    Backoff {
        until: Instant,
        failure: SharedDialFailure,
    },
}

/// Result of bounded dial admission. Only `Leader` is allowed to create a
/// managed transport task; followers wait on the leader's shared result.
pub(crate) enum DialAdmission<K: Clone + Eq + Hash> {
    Leader {
        key: K,
        flight: Arc<InFlightDial>,
        _pending: OwnedSemaphorePermit,
        cancellation: DialCancellation<K>,
    },
    Follower {
        outcome: watch::Receiver<Option<SharedDialOutcome>>,
        _pending: OwnedSemaphorePermit,
    },
}

/// Bounded, cancellation-aware outbound dial singleflight.
///
/// Admission is acquired synchronously with `try_acquire_owned` before the
/// caller can create a task. Failures are cached for a short bounded backoff so
/// an outage produces one network attempt, not one attempt per waiter.
pub(crate) struct OutboundDialCoordinator<K> {
    pending: Arc<Semaphore>,
    handshakes: Arc<Semaphore>,
    states: StdMutex<HashMap<K, DialState>>,
    max_states: usize,
    failure_backoff: Duration,
    closed: AtomicBool,
}

impl<K> OutboundDialCoordinator<K>
where
    K: Clone + Eq + Hash,
{
    pub(crate) fn new(
        max_handshakes: usize,
        max_pending: usize,
        failure_backoff: Duration,
    ) -> Arc<Self> {
        Arc::new(Self {
            pending: Arc::new(Semaphore::new(max_pending)),
            handshakes: Arc::new(Semaphore::new(max_handshakes)),
            states: StdMutex::new(HashMap::new()),
            max_states: max_pending.saturating_mul(2).max(1),
            failure_backoff,
            closed: AtomicBool::new(false),
        })
    }

    pub(crate) fn begin(self: &Arc<Self>, key: K) -> Result<DialAdmission<K>> {
        if self.closed.load(Ordering::Acquire) {
            return Err(Error::TransportClosed);
        }
        let pending = self
            .pending
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::ConnectionPoolExhausted)?;
        let now = Instant::now();
        let mut states = self
            .states
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.closed.load(Ordering::Acquire) {
            return Err(Error::TransportClosed);
        }
        states
            .retain(|_, state| !matches!(state, DialState::Backoff { until, .. } if *until <= now));
        match states.get(&key) {
            Some(DialState::InFlight(flight)) => {
                return Ok(DialAdmission::Follower {
                    outcome: flight.outcome.subscribe(),
                    _pending: pending,
                });
            }
            Some(DialState::Backoff { failure, .. }) => {
                return Err(failure.clone().into_error());
            }
            None => {}
        }
        if states.len() >= self.max_states {
            return Err(Error::ConnectionPoolExhausted);
        }
        let (outcome, _) = watch::channel(None);
        let flight = Arc::new(InFlightDial { outcome });
        states.insert(key.clone(), DialState::InFlight(flight.clone()));
        let cancellation = DialCancellation {
            coordinator: Arc::downgrade(self),
            key: Some(key.clone()),
            flight: flight.clone(),
        };
        Ok(DialAdmission::Leader {
            key,
            flight,
            _pending: pending,
            cancellation,
        })
    }

    pub(crate) async fn acquire_handshake(
        &self,
        deadline: Instant,
        destination: SocketAddr,
    ) -> Result<OwnedSemaphorePermit> {
        tokio::time::timeout_at(deadline, self.handshakes.clone().acquire_owned())
            .await
            .map_err(|_| Error::ConnectionTimeout(destination))?
            .map_err(|_| Error::TransportClosed)
    }

    pub(crate) fn complete<T>(
        &self,
        key: &K,
        flight: &Arc<InFlightDial>,
        result: &Result<T>,
        cancellation: &mut DialCancellation<K>,
    ) {
        let outcome = result
            .as_ref()
            .map(|_| ())
            .map_err(SharedDialFailure::capture);
        self.finish(key, flight, outcome);
        cancellation.disarm();
    }

    fn finish(&self, key: &K, flight: &Arc<InFlightDial>, outcome: SharedDialOutcome) {
        let mut states = self
            .states
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let same_flight = matches!(
            states.get(key),
            Some(DialState::InFlight(current)) if Arc::ptr_eq(current, flight)
        );
        if !same_flight {
            return;
        }
        if let Err(failure) = &outcome {
            states.insert(
                key.clone(),
                DialState::Backoff {
                    until: Instant::now() + self.failure_backoff,
                    failure: failure.clone(),
                },
            );
        } else {
            states.remove(key);
        }
        drop(states);
        flight.outcome.send_replace(Some(outcome));
    }

    pub(crate) async fn wait(
        mut outcome: watch::Receiver<Option<SharedDialOutcome>>,
        deadline: Instant,
        destination: SocketAddr,
    ) -> Result<()> {
        loop {
            if let Some(result) = outcome.borrow().clone() {
                return result.map_err(SharedDialFailure::into_error);
            }
            tokio::time::timeout_at(deadline, outcome.changed())
                .await
                .map_err(|_| Error::ConnectionTimeout(destination))?
                .map_err(|_| Error::TransportClosed)?;
        }
    }

    pub(crate) fn close(&self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        self.pending.close();
        self.handshakes.close();
        let mut states = self
            .states
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for state in states.values() {
            if let DialState::InFlight(flight) = state {
                flight
                    .outcome
                    .send_replace(Some(Err(SharedDialFailure::TransportClosed)));
            }
        }
        states.clear();
    }
}

/// Synchronous cancellation cleanup for the narrow gap between singleflight
/// admission and managed-task registration. Once moved into the task it also
/// wakes followers if shutdown aborts or a panic drops the leader future.
pub(crate) struct DialCancellation<K>
where
    K: Clone + Eq + Hash,
{
    coordinator: Weak<OutboundDialCoordinator<K>>,
    key: Option<K>,
    flight: Arc<InFlightDial>,
}

impl<K> DialCancellation<K>
where
    K: Clone + Eq + Hash,
{
    fn disarm(&mut self) {
        self.key = None;
    }
}

impl<K> Drop for DialCancellation<K>
where
    K: Clone + Eq + Hash,
{
    fn drop(&mut self) {
        let Some(key) = self.key.take() else {
            return;
        };
        let outcome = Err(SharedDialFailure::Cancelled);
        if let Some(coordinator) = self.coordinator.upgrade() {
            coordinator.finish(&key, &self.flight, outcome);
        } else {
            self.flight.outcome.send_replace(Some(outcome));
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

    #[test]
    fn lifecycle_deadline_enforces_idle_and_authentication_expiry() {
        let lifecycle = ConnectionLifecycleConfig::from_handshake(HandshakeAdmissionConfig::new(
            Duration::from_secs(1),
            1,
        ));
        let established = Instant::now();
        assert_eq!(
            lifecycle.next_deadline(established, established),
            established + lifecycle.idle_timeout
        );
        let recently_active = established + lifecycle.authentication_lifetime;
        assert_eq!(
            lifecycle.next_deadline(recently_active, established),
            established + lifecycle.authentication_lifetime
        );
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
    async fn outbound_dials_share_failure_and_backoff() {
        let destination: SocketAddr = "127.0.0.1:5061".parse().unwrap();
        let coordinator = OutboundDialCoordinator::new(1, 2, Duration::from_secs(1));
        let (key, flight, leader_pending, mut cancellation) =
            match coordinator.begin("authority-a").unwrap() {
                DialAdmission::Leader {
                    key,
                    flight,
                    _pending,
                    cancellation,
                } => (key, flight, _pending, cancellation),
                DialAdmission::Follower { .. } => panic!("first dial must lead"),
            };
        let follower = match coordinator.begin("authority-a").unwrap() {
            DialAdmission::Follower { outcome, _pending } => (outcome, _pending),
            DialAdmission::Leader { .. } => panic!("second dial must follow"),
        };
        let failure = Err::<(), _>(Error::ConnectionTimeout(destination));
        coordinator.complete(&key, &flight, &failure, &mut cancellation);
        drop(leader_pending);
        assert!(matches!(
            OutboundDialCoordinator::<&str>::wait(
                follower.0,
                Instant::now() + Duration::from_secs(1),
                destination,
            )
            .await,
            Err(Error::ConnectionTimeout(address)) if address == destination
        ));
        drop(follower.1);
        assert!(matches!(
            coordinator.begin("authority-a"),
            Err(Error::ConnectionTimeout(address)) if address == destination
        ));
    }

    #[tokio::test]
    async fn outbound_pending_admission_fails_before_task_creation() {
        let coordinator = OutboundDialCoordinator::new(1, 1, Duration::from_millis(10));
        let _leader = coordinator.begin("authority-a").unwrap();
        assert!(matches!(
            coordinator.begin("authority-b"),
            Err(Error::ConnectionPoolExhausted)
        ));
    }

    #[tokio::test]
    async fn cancelled_leader_wakes_followers_and_enters_backoff() {
        let destination: SocketAddr = "127.0.0.1:5061".parse().unwrap();
        let coordinator = OutboundDialCoordinator::new(1, 2, Duration::from_secs(1));
        let leader = coordinator.begin("authority-a").unwrap();
        let follower = match coordinator.begin("authority-a").unwrap() {
            DialAdmission::Follower { outcome, _pending } => (outcome, _pending),
            DialAdmission::Leader { .. } => panic!("second dial must follow"),
        };
        drop(leader);

        assert!(matches!(
            OutboundDialCoordinator::<&str>::wait(
                follower.0,
                Instant::now() + Duration::from_secs(1),
                destination,
            )
            .await,
            Err(Error::Other(message)) if message.contains("cancelled")
        ));
        drop(follower.1);
        assert!(matches!(
            coordinator.begin("authority-a"),
            Err(Error::Other(message)) if message.contains("cancelled")
        ));
    }
}
