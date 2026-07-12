//! Managed public lifecycle for an authenticated MOQT catalog subscriber.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::FutureExt;
use rvoip_core_traits::broadcast::{
    BroadcastHealthDescriptor, BroadcastHealthIssue, BroadcastHealthStatus,
};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::catalog_subscriber_wire::{
    WireCatalogFailure, WireCatalogObject, WireCatalogSubscriberClient, WireCatalogSubscription,
};
use crate::{
    MoqCatalogApplyOutcome, MoqCatalogObject, MoqCatalogStateMachine, MoqCatalogSubscriberConfig,
    MoqCatalogSubscriberFailure, MoqCatalogSubscriberLifecycle, MoqCatalogSubscriptionSnapshot,
    MoqEndOfGroupEvidence, MoqError, MoqProtocolVersion, MoqSubscriberCredential,
    MoqSubscriberCredentialError, MoqSubscriberCredentialProvider, MsfCatalogState, CATALOG_TRACK,
};

const DROP_CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone)]
struct CatalogConnectionDiagnostics {
    endpoint_uri: String,
    substrate: rvoip_core_traits::broadcast::BroadcastSubstrate,
    negotiated_protocol: String,
    peer_identity: crate::MoqRelayPeerIdentity,
}

struct ManagedCatalogObject {
    group_id: u64,
    subgroup_id: u64,
    object_id: u64,
    first_object: bool,
    end_of_group: MoqEndOfGroupEvidence,
    extension_header_count: usize,
    declared_payload_len: u64,
    payload: bytes::Bytes,
}

#[async_trait]
trait CatalogConnector: Send + Sync {
    async fn connect(
        &self,
        config: &MoqCatalogSubscriberConfig,
        credential: MoqSubscriberCredential,
    ) -> Result<Box<dyn CatalogConnection>, WireCatalogFailure>;
}

#[async_trait]
trait CatalogConnection: Send {
    fn diagnostics(&self) -> CatalogConnectionDiagnostics;

    async fn next_object(
        &mut self,
        max_catalog_bytes: usize,
    ) -> Result<Option<ManagedCatalogObject>, WireCatalogFailure>;

    async fn close(self: Box<Self>, reason: &'static str);
}

#[async_trait]
impl CatalogConnector for WireCatalogSubscriberClient {
    async fn connect(
        &self,
        config: &MoqCatalogSubscriberConfig,
        credential: MoqSubscriberCredential,
    ) -> Result<Box<dyn CatalogConnection>, WireCatalogFailure> {
        Ok(Box::new(self.connect(config, credential).await?))
    }
}

#[async_trait]
impl CatalogConnection for WireCatalogSubscription {
    fn diagnostics(&self) -> CatalogConnectionDiagnostics {
        CatalogConnectionDiagnostics {
            endpoint_uri: self.endpoint_uri.clone(),
            substrate: self.substrate,
            negotiated_protocol: self.negotiated_protocol.clone(),
            peer_identity: self.peer_identity.clone(),
        }
    }

    async fn next_object(
        &mut self,
        max_catalog_bytes: usize,
    ) -> Result<Option<ManagedCatalogObject>, WireCatalogFailure> {
        Ok(
            WireCatalogSubscription::next_object(self, max_catalog_bytes)
                .await?
                .map(ManagedCatalogObject::from),
        )
    }

    async fn close(self: Box<Self>, reason: &'static str) {
        (*self).close(reason).await;
    }
}

impl From<WireCatalogObject> for ManagedCatalogObject {
    fn from(object: WireCatalogObject) -> Self {
        Self {
            group_id: object.group_id,
            subgroup_id: object.subgroup_id,
            object_id: object.object_id,
            first_object: object.first_object,
            end_of_group: object.end_of_group,
            extension_header_count: object.extension_header_count,
            declared_payload_len: object.declared_payload_len,
            payload: object.payload,
        }
    }
}

/// Production TLS roots for an outbound catalog subscriber.
///
/// This type intentionally has no client-identity or verification-disable
/// fields: bearer-token subscribers verify the relay certificate and present
/// no TLS client certificate.
#[derive(Clone, Default)]
pub struct MoqCatalogSubscriberTlsConfig {
    /// PEM trust roots. An empty list uses verified system roots.
    pub root_certificates: Vec<PathBuf>,
}

impl std::fmt::Debug for MoqCatalogSubscriberTlsConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MoqCatalogSubscriberTlsConfig")
            .field("root_certificate_count", &self.root_certificates.len())
            .finish()
    }
}

/// Managed, reconnecting MSF catalog subscriber.
///
/// All public state is rvoip-owned. Draft-specific MOQT session, request, and
/// stream handles remain inside the private adapter.
#[must_use = "retain the subscriber or close it to observe cleanup"]
pub struct MoqCatalogSubscriber {
    control: Arc<CatalogSubscriberControl>,
}

impl MoqCatalogSubscriber {
    pub fn bind(
        bind: SocketAddr,
        config: MoqCatalogSubscriberConfig,
        tls: MoqCatalogSubscriberTlsConfig,
        credentials: Arc<dyn MoqSubscriberCredentialProvider>,
    ) -> Result<Arc<Self>, MoqError> {
        config.validate()?;
        let wire = WireCatalogSubscriberClient::bind(bind, tls.root_certificates)?;
        Self::start_with_connector(config, credentials, Arc::new(wire))
    }

    fn start_with_connector(
        config: MoqCatalogSubscriberConfig,
        credentials: Arc<dyn MoqSubscriberCredentialProvider>,
        connector: Arc<dyn CatalogConnector>,
    ) -> Result<Arc<Self>, MoqError> {
        config.validate()?;
        let state = MoqCatalogStateMachine::new(&config)?;
        let runtime =
            tokio::runtime::Handle::try_current().map_err(|_| MoqError::RuntimeUnavailable)?;
        let status = Arc::new(CatalogSubscriberStatus::new(&config));
        let cancel = CancellationToken::new();
        let task_status = Arc::clone(&status);
        let task_cancel = cancel.clone();
        let task = runtime.spawn(async move {
            let result = AssertUnwindSafe(run_subscriber(
                config,
                connector,
                credentials,
                state,
                Arc::clone(&task_status),
                task_cancel,
            ))
            .catch_unwind()
            .await;
            if result.is_err() {
                task_status.failed(MoqCatalogSubscriberFailure::TaskFailed);
            }
        });
        Ok(Arc::new(Self {
            control: Arc::new(CatalogSubscriberControl {
                status,
                cancel,
                task: Mutex::new(Some(task)),
                runtime,
            }),
        }))
    }

    pub fn snapshot(&self) -> MoqCatalogSubscriptionSnapshot {
        self.control.status.snapshot()
    }

    /// Subscribe to latest-state updates without exposing wire-engine types.
    pub fn updates(&self) -> watch::Receiver<MoqCatalogSubscriptionSnapshot> {
        self.control.status.subscribe()
    }

    /// Wait for permanent completion, explicit close, or terminal failure.
    pub async fn wait(&self) -> Result<(), MoqError> {
        let snapshot = self.control.status.wait_terminal().await;
        terminal_result(&snapshot)
    }

    /// Stop reconnecting, close the active network session, and join cleanup.
    pub async fn close(&self) -> Result<(), MoqError> {
        self.control.cancel.cancel();
        let snapshot = self.control.status.wait_terminal().await;
        self.control.join_task().await;
        terminal_result(&snapshot)
    }

    /// Close by a fixed deadline. Returns false on timeout or terminal failure.
    pub async fn drain(&self, deadline: DateTime<Utc>) -> bool {
        self.control.cancel.cancel();
        let remaining = deadline
            .signed_duration_since(Utc::now())
            .to_std()
            .unwrap_or(Duration::ZERO);
        let completed = tokio::time::timeout(remaining, async {
            let snapshot = self.control.status.wait_terminal().await;
            self.control.join_task().await;
            terminal_result(&snapshot).is_ok()
        })
        .await;
        match completed {
            Ok(clean) => clean,
            Err(_) => {
                self.control.abort_and_reap().await;
                false
            }
        }
    }
}

impl Drop for MoqCatalogSubscriber {
    fn drop(&mut self) {
        self.control.start_cleanup_reaper();
    }
}

struct CatalogSubscriberControl {
    status: Arc<CatalogSubscriberStatus>,
    cancel: CancellationToken,
    task: Mutex<Option<JoinHandle<()>>>,
    runtime: tokio::runtime::Handle,
}

impl CatalogSubscriberControl {
    async fn join_task(&self) {
        let task = self
            .task
            .lock()
            .expect("MOQT catalog subscriber task lock poisoned")
            .take();
        if let Some(task) = task {
            let _ = task.await;
        }
    }

    async fn abort_and_reap(&self) {
        let task = self
            .task
            .lock()
            .expect("MOQT catalog subscriber task lock poisoned")
            .take();
        if let Some(task) = task {
            task.abort();
            let _ = task.await;
        }
        if !self.status.snapshot().is_terminal() {
            self.status.failed(MoqCatalogSubscriberFailure::TaskFailed);
        }
    }

    fn start_cleanup_reaper(&self) {
        self.cancel.cancel();
        let task = self
            .task
            .lock()
            .expect("MOQT catalog subscriber task lock poisoned")
            .take();
        let Some(mut task) = task else {
            return;
        };
        let _cleanup = self.runtime.spawn(async move {
            if tokio::time::timeout(DROP_CLEANUP_TIMEOUT, &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
            }
        });
    }
}

struct CatalogSubscriberStatus {
    sender: watch::Sender<MoqCatalogSubscriptionSnapshot>,
}

impl CatalogSubscriberStatus {
    fn new(config: &MoqCatalogSubscriberConfig) -> Self {
        let now = Utc::now();
        let snapshot = MoqCatalogSubscriptionSnapshot {
            endpoint_uri: config.endpoint.to_string(),
            namespace: config.namespace.clone(),
            protocol_version: MoqProtocolVersion::PINNED,
            lifecycle: MoqCatalogSubscriberLifecycle::Starting,
            lifecycle_since: now,
            health: health_for(MoqCatalogSubscriberLifecycle::Starting, None, now),
            latest: None,
            failure: None,
            reconnects: 0,
            substrate: None,
            negotiated_protocol: None,
            peer_identity: None,
        };
        let (sender, _) = watch::channel(snapshot);
        Self { sender }
    }

    fn snapshot(&self) -> MoqCatalogSubscriptionSnapshot {
        self.sender.borrow().clone()
    }

    fn subscribe(&self) -> watch::Receiver<MoqCatalogSubscriptionSnapshot> {
        self.sender.subscribe()
    }

    fn transition(
        &self,
        lifecycle: MoqCatalogSubscriberLifecycle,
        failure: Option<MoqCatalogSubscriberFailure>,
    ) {
        let now = Utc::now();
        self.sender.send_modify(|snapshot| {
            if snapshot.lifecycle.is_terminal() {
                return;
            }
            snapshot.lifecycle = lifecycle;
            snapshot.lifecycle_since = now;
            snapshot.failure = failure;
            snapshot.health = health_for(lifecycle, failure, now);
        });
    }

    fn connected(&self, diagnostics: CatalogConnectionDiagnostics) {
        let now = Utc::now();
        self.sender.send_modify(|snapshot| {
            if snapshot.lifecycle.is_terminal() {
                return;
            }
            snapshot.endpoint_uri = diagnostics.endpoint_uri;
            snapshot.lifecycle = MoqCatalogSubscriberLifecycle::Subscribing;
            snapshot.lifecycle_since = now;
            snapshot.health = health_for(MoqCatalogSubscriberLifecycle::Subscribing, None, now);
            snapshot.failure = None;
            snapshot.substrate = Some(diagnostics.substrate);
            snapshot.negotiated_protocol = Some(diagnostics.negotiated_protocol);
            snapshot.peer_identity = Some(diagnostics.peer_identity);
        });
    }

    fn update(&self, update: crate::MoqCatalogUpdate) {
        let lifecycle = match update.catalog.state() {
            MsfCatalogState::Live => MoqCatalogSubscriberLifecycle::Live,
            MsfCatalogState::PermanentlyCompleted => {
                MoqCatalogSubscriberLifecycle::PermanentlyCompleted
            }
        };
        let now = Utc::now();
        self.sender.send_modify(|snapshot| {
            if snapshot.lifecycle.is_terminal() {
                return;
            }
            snapshot.lifecycle = lifecycle;
            snapshot.lifecycle_since = now;
            snapshot.health = health_for(lifecycle, None, now);
            snapshot.failure = None;
            snapshot.latest = Some(update);
        });
    }

    fn reconnecting(&self, reconnects: u32, failure: MoqCatalogSubscriberFailure) {
        self.transition(MoqCatalogSubscriberLifecycle::Reconnecting, Some(failure));
        self.sender.send_modify(|snapshot| {
            snapshot.reconnects = reconnects;
        });
    }

    fn failed(&self, failure: MoqCatalogSubscriberFailure) {
        self.transition(MoqCatalogSubscriberLifecycle::Failed, Some(failure));
    }

    fn draining(&self) {
        self.transition(MoqCatalogSubscriberLifecycle::Draining, None);
    }

    fn closed(&self) {
        self.transition(MoqCatalogSubscriberLifecycle::Closed, None);
    }

    async fn wait_terminal(&self) -> MoqCatalogSubscriptionSnapshot {
        let mut receiver = self.subscribe();
        loop {
            let snapshot = receiver.borrow().clone();
            if snapshot.is_terminal() {
                return snapshot;
            }
            if receiver.changed().await.is_err() {
                return snapshot;
            }
        }
    }
}

async fn run_subscriber(
    config: MoqCatalogSubscriberConfig,
    connector: Arc<dyn CatalogConnector>,
    credentials: Arc<dyn MoqSubscriberCredentialProvider>,
    mut catalog: MoqCatalogStateMachine,
    status: Arc<CatalogSubscriberStatus>,
    cancel: CancellationToken,
) {
    let mut used_credentials = HashSet::new();
    let mut attempt = 0_u32;
    let mut reconnect_deadline = None;

    loop {
        if cancel.is_cancelled() {
            status.draining();
            status.closed();
            return;
        }
        status.transition(MoqCatalogSubscriberLifecycle::Connecting, None);
        let request = match config.credential_request(attempt) {
            Ok(request) => request,
            Err(_) => {
                status.failed(MoqCatalogSubscriberFailure::ReconnectExhausted);
                return;
            }
        };
        let connect = async {
            let credential = credentials
                .issue(request)
                .await
                .map_err(map_credential_failure)?;
            if !used_credentials.insert(credential.fingerprint()) {
                return Err(MoqCatalogSubscriberFailure::CredentialReused);
            }
            connector
                .connect(&config, credential)
                .await
                .map_err(map_wire_failure)
        };
        let connected = tokio::select! {
            () = cancel.cancelled() => {
                status.draining();
                status.closed();
                return;
            }
            result = tokio::time::timeout(config.attempt_timeout, connect) => {
                match result {
                    Ok(result) => result,
                    Err(_) => Err(MoqCatalogSubscriberFailure::ConnectTimeout),
                }
            }
        };

        let mut subscription = match connected {
            Ok(subscription) => {
                reconnect_deadline = None;
                status.connected(subscription.diagnostics());
                subscription
            }
            Err(failure) => {
                if !retry_or_wait(
                    &config,
                    &status,
                    &cancel,
                    &mut attempt,
                    &mut reconnect_deadline,
                    failure,
                )
                .await
                {
                    return;
                }
                continue;
            }
        };

        let failure = loop {
            let next = tokio::select! {
                () = cancel.cancelled() => {
                    status.draining();
                    subscription.close("rvoip catalog subscriber closed").await;
                    status.closed();
                    return;
                }
                result = subscription.next_object(config.max_catalog_bytes) => result,
            };
            let object = match next {
                Ok(Some(object)) => object,
                Ok(None) => break MoqCatalogSubscriberFailure::StreamEnded,
                Err(error) => break map_wire_failure(error),
            };
            let applied = catalog.apply(MoqCatalogObject {
                namespace: config.namespace.as_str(),
                track: CATALOG_TRACK,
                group_id: object.group_id,
                subgroup_id: object.subgroup_id,
                object_id: object.object_id,
                first_object: object.first_object,
                end_of_group: object.end_of_group,
                extension_header_count: object.extension_header_count,
                declared_payload_len: object.declared_payload_len,
                payload: &object.payload,
                received_at: Utc::now(),
            });
            match applied {
                Ok(MoqCatalogApplyOutcome::Duplicate) => {}
                Ok(MoqCatalogApplyOutcome::Update(update)) => {
                    let completed = update.catalog.state() == MsfCatalogState::PermanentlyCompleted;
                    status.update(update);
                    if completed {
                        subscription
                            .close("rvoip catalog publication completed")
                            .await;
                        return;
                    }
                }
                Err(error) => break map_validation_failure(&error),
            }
        };
        subscription
            .close("rvoip catalog subscriber reconnecting")
            .await;
        if !retry_or_wait(
            &config,
            &status,
            &cancel,
            &mut attempt,
            &mut reconnect_deadline,
            failure,
        )
        .await
        {
            return;
        }
    }
}

async fn retry_or_wait(
    config: &MoqCatalogSubscriberConfig,
    status: &CatalogSubscriberStatus,
    cancel: &CancellationToken,
    attempt: &mut u32,
    reconnect_deadline: &mut Option<Instant>,
    failure: MoqCatalogSubscriberFailure,
) -> bool {
    if !retryable(failure) || *attempt >= config.max_reconnect_attempts {
        status.failed(if retryable(failure) {
            MoqCatalogSubscriberFailure::ReconnectExhausted
        } else {
            failure
        });
        return false;
    }
    let deadline =
        *reconnect_deadline.get_or_insert_with(|| Instant::now() + config.reconnect_deadline);
    let backoff = reconnect_backoff(config, *attempt);
    let Some(wakeup) = Instant::now().checked_add(backoff) else {
        status.failed(MoqCatalogSubscriberFailure::ReconnectExhausted);
        return false;
    };
    if wakeup > deadline {
        status.failed(MoqCatalogSubscriberFailure::ReconnectExhausted);
        return false;
    }
    *attempt = attempt.saturating_add(1);
    status.reconnecting(*attempt, failure);
    tokio::select! {
        () = cancel.cancelled() => {
            status.draining();
            status.closed();
            false
        }
        () = tokio::time::sleep(backoff) => true,
    }
}

fn reconnect_backoff(config: &MoqCatalogSubscriberConfig, attempt: u32) -> Duration {
    let factor = 1_u32.checked_shl(attempt.min(31)).unwrap_or(u32::MAX);
    config
        .reconnect_initial_backoff
        .checked_mul(factor)
        .unwrap_or(config.reconnect_max_backoff)
        .min(config.reconnect_max_backoff)
}

const fn retryable(failure: MoqCatalogSubscriberFailure) -> bool {
    matches!(
        failure,
        MoqCatalogSubscriberFailure::CredentialUnavailable
            | MoqCatalogSubscriberFailure::ConnectFailed
            | MoqCatalogSubscriberFailure::ConnectTimeout
            | MoqCatalogSubscriberFailure::SetupFailed
            | MoqCatalogSubscriberFailure::SubscribeFailed
            | MoqCatalogSubscriberFailure::StreamEnded
            | MoqCatalogSubscriberFailure::TaskFailed
    )
}

const fn map_credential_failure(
    error: MoqSubscriberCredentialError,
) -> MoqCatalogSubscriberFailure {
    match error {
        MoqSubscriberCredentialError::Unavailable => {
            MoqCatalogSubscriberFailure::CredentialUnavailable
        }
        MoqSubscriberCredentialError::Denied
        | MoqSubscriberCredentialError::Empty
        | MoqSubscriberCredentialError::TooLong { .. } => {
            MoqCatalogSubscriberFailure::CredentialDenied
        }
    }
}

fn map_wire_failure(error: WireCatalogFailure) -> MoqCatalogSubscriberFailure {
    match error {
        WireCatalogFailure::ConnectFailed => MoqCatalogSubscriberFailure::ConnectFailed,
        WireCatalogFailure::PeerUnauthenticated => MoqCatalogSubscriberFailure::PeerUnauthenticated,
        WireCatalogFailure::ProtocolMismatch => MoqCatalogSubscriberFailure::ProtocolMismatch,
        WireCatalogFailure::SetupFailed => MoqCatalogSubscriberFailure::SetupFailed,
        WireCatalogFailure::SubscribeFailed => MoqCatalogSubscriberFailure::SubscribeFailed,
        WireCatalogFailure::InvalidTrack => MoqCatalogSubscriberFailure::InvalidTrack,
        WireCatalogFailure::InvalidCatalog(error) => map_validation_failure(&error),
        WireCatalogFailure::PayloadTooLarge => MoqCatalogSubscriberFailure::PayloadTooLarge,
        WireCatalogFailure::StreamEnded | WireCatalogFailure::SessionEnded => {
            MoqCatalogSubscriberFailure::StreamEnded
        }
        WireCatalogFailure::TaskFailed => MoqCatalogSubscriberFailure::TaskFailed,
    }
}

const fn map_validation_failure(
    error: &crate::MoqCatalogValidationError,
) -> MoqCatalogSubscriberFailure {
    match error {
        crate::MoqCatalogValidationError::NamespaceMismatch
        | crate::MoqCatalogValidationError::TrackMismatch => {
            MoqCatalogSubscriberFailure::InvalidTrack
        }
        crate::MoqCatalogValidationError::PayloadTooLarge { .. } => {
            MoqCatalogSubscriberFailure::PayloadTooLarge
        }
        _ => MoqCatalogSubscriberFailure::InvalidCatalog,
    }
}

fn terminal_result(snapshot: &MoqCatalogSubscriptionSnapshot) -> Result<(), MoqError> {
    if snapshot.lifecycle == MoqCatalogSubscriberLifecycle::Failed {
        return Err(MoqError::CatalogSubscriber(
            snapshot
                .failure
                .unwrap_or(MoqCatalogSubscriberFailure::TaskFailed),
        ));
    }
    Ok(())
}

fn health_for(
    lifecycle: MoqCatalogSubscriberLifecycle,
    failure: Option<MoqCatalogSubscriberFailure>,
    checked_at: DateTime<Utc>,
) -> BroadcastHealthDescriptor {
    let (status, issues) = match lifecycle {
        MoqCatalogSubscriberLifecycle::Live => (BroadcastHealthStatus::Healthy, Vec::new()),
        MoqCatalogSubscriberLifecycle::PermanentlyCompleted
        | MoqCatalogSubscriberLifecycle::Closed => (BroadcastHealthStatus::Closed, Vec::new()),
        MoqCatalogSubscriberLifecycle::Failed => (
            BroadcastHealthStatus::Unhealthy,
            vec![health_issue_for_failure(failure)],
        ),
        MoqCatalogSubscriberLifecycle::Reconnecting => (
            BroadcastHealthStatus::Degraded,
            vec![BroadcastHealthIssue::Reconnecting],
        ),
        MoqCatalogSubscriberLifecycle::Draining => (
            BroadcastHealthStatus::Degraded,
            vec![BroadcastHealthIssue::Draining],
        ),
        MoqCatalogSubscriberLifecycle::Starting
        | MoqCatalogSubscriberLifecycle::Connecting
        | MoqCatalogSubscriberLifecycle::Subscribing => (
            BroadcastHealthStatus::Degraded,
            vec![BroadcastHealthIssue::TransportUnavailable],
        ),
    };
    BroadcastHealthDescriptor {
        status,
        issues,
        active_subscribers: None,
        subscriber_capacity: None,
        checked_at,
    }
}

const fn health_issue_for_failure(
    failure: Option<MoqCatalogSubscriberFailure>,
) -> BroadcastHealthIssue {
    match failure {
        Some(
            MoqCatalogSubscriberFailure::CredentialUnavailable
            | MoqCatalogSubscriberFailure::CredentialDenied
            | MoqCatalogSubscriberFailure::CredentialReused,
        ) => BroadcastHealthIssue::AuthenticationUnavailable,
        Some(MoqCatalogSubscriberFailure::ProtocolMismatch) => {
            BroadcastHealthIssue::VersionMismatch
        }
        _ => BroadcastHealthIssue::RelayUnavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::future::pending;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::sync::Notify;

    struct SequenceCredentials {
        values: Mutex<VecDeque<Result<Vec<u8>, MoqSubscriberCredentialError>>>,
        calls: AtomicUsize,
    }

    impl SequenceCredentials {
        fn new(
            values: impl IntoIterator<Item = Result<Vec<u8>, MoqSubscriberCredentialError>>,
        ) -> Self {
            Self {
                values: Mutex::new(values.into_iter().collect()),
                calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl MoqSubscriberCredentialProvider for SequenceCredentials {
        async fn issue(
            &self,
            _request: crate::MoqSubscriberCredentialRequest,
        ) -> Result<MoqSubscriberCredential, MoqSubscriberCredentialError> {
            self.calls.fetch_add(1, Ordering::AcqRel);
            let value = self
                .values
                .lock()
                .expect("credential sequence poisoned")
                .pop_front()
                .unwrap_or(Err(MoqSubscriberCredentialError::Unavailable))?;
            MoqSubscriberCredential::new(value)
        }
    }

    enum MockAttempt {
        Fail(WireCatalogFailure),
        Pending {
            started: Arc<Notify>,
            dropped: Arc<AtomicUsize>,
        },
        Connected(MockStream),
    }

    enum MockStream {
        Pending { started: Arc<Notify> },
        Events(VecDeque<Result<Option<ManagedCatalogObject>, WireCatalogFailure>>),
    }

    struct MockConnector {
        attempts: Mutex<VecDeque<MockAttempt>>,
        calls: AtomicUsize,
        fingerprints: Mutex<Vec<[u8; 32]>>,
        closes: Arc<AtomicUsize>,
        close_notify: Arc<Notify>,
    }

    impl MockConnector {
        fn new(attempts: impl IntoIterator<Item = MockAttempt>) -> Self {
            Self {
                attempts: Mutex::new(attempts.into_iter().collect()),
                calls: AtomicUsize::new(0),
                fingerprints: Mutex::new(Vec::new()),
                closes: Arc::new(AtomicUsize::new(0)),
                close_notify: Arc::new(Notify::new()),
            }
        }
    }

    struct PendingAttemptGuard(Arc<AtomicUsize>);

    impl Drop for PendingAttemptGuard {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::AcqRel);
        }
    }

    #[async_trait]
    impl CatalogConnector for MockConnector {
        async fn connect(
            &self,
            _config: &MoqCatalogSubscriberConfig,
            credential: MoqSubscriberCredential,
        ) -> Result<Box<dyn CatalogConnection>, WireCatalogFailure> {
            self.calls.fetch_add(1, Ordering::AcqRel);
            self.fingerprints
                .lock()
                .expect("fingerprints poisoned")
                .push(credential.fingerprint());
            let attempt = self
                .attempts
                .lock()
                .expect("attempt sequence poisoned")
                .pop_front()
                .unwrap_or(MockAttempt::Fail(WireCatalogFailure::ConnectFailed));
            match attempt {
                MockAttempt::Fail(failure) => Err(failure),
                MockAttempt::Pending { started, dropped } => {
                    let _guard = PendingAttemptGuard(dropped);
                    started.notify_one();
                    pending().await
                }
                MockAttempt::Connected(stream) => Ok(Box::new(MockConnection {
                    stream,
                    closes: Arc::clone(&self.closes),
                    close_notify: Arc::clone(&self.close_notify),
                })),
            }
        }
    }

    struct MockConnection {
        stream: MockStream,
        closes: Arc<AtomicUsize>,
        close_notify: Arc<Notify>,
    }

    #[async_trait]
    impl CatalogConnection for MockConnection {
        fn diagnostics(&self) -> CatalogConnectionDiagnostics {
            CatalogConnectionDiagnostics {
                endpoint_uri: "moqt://relay.example/tenant/broadcast".into(),
                substrate: rvoip_core_traits::broadcast::BroadcastSubstrate::WebTransport,
                negotiated_protocol: crate::MOQT_NEGOTIATED_PROTOCOL.into(),
                peer_identity: crate::MoqRelayPeerIdentity::VerifiedCertificate {
                    leaf_sha256: "aa".repeat(32),
                    chain_len: 1,
                    total_der_bytes: 512,
                },
            }
        }

        async fn next_object(
            &mut self,
            _max_catalog_bytes: usize,
        ) -> Result<Option<ManagedCatalogObject>, WireCatalogFailure> {
            match &mut self.stream {
                MockStream::Pending { started } => {
                    started.notify_one();
                    pending().await
                }
                MockStream::Events(events) => events
                    .pop_front()
                    .unwrap_or(Err(WireCatalogFailure::StreamEnded)),
            }
        }

        async fn close(self: Box<Self>, _reason: &'static str) {
            self.closes.fetch_add(1, Ordering::AcqRel);
            self.close_notify.notify_one();
        }
    }

    fn config(max_reconnect_attempts: u32) -> MoqCatalogSubscriberConfig {
        let mut config = MoqCatalogSubscriberConfig::new(
            url::Url::parse("moqt://relay.example/tenant/broadcast").unwrap(),
            crate::MoqNamespace::new("tenant", "broadcast").unwrap(),
        );
        config.attempt_timeout = Duration::from_millis(20);
        config.max_reconnect_attempts = max_reconnect_attempts;
        config.reconnect_initial_backoff = Duration::from_millis(1);
        config.reconnect_max_backoff = Duration::from_millis(2);
        config.reconnect_deadline = Duration::from_millis(100);
        config
    }

    fn terminal_object(end_of_group: MoqEndOfGroupEvidence) -> ManagedCatalogObject {
        let payload = crate::MsfCatalog::permanently_completed(1)
            .to_json_bytes()
            .unwrap();
        ManagedCatalogObject {
            group_id: 7,
            subgroup_id: 0,
            object_id: 0,
            first_object: true,
            end_of_group,
            extension_header_count: 0,
            declared_payload_len: payload.len() as u64,
            payload: payload.into(),
        }
    }

    fn start_mock(
        config: MoqCatalogSubscriberConfig,
        credentials: Arc<SequenceCredentials>,
        connector: Arc<MockConnector>,
    ) -> Arc<MoqCatalogSubscriber> {
        let credentials: Arc<dyn MoqSubscriberCredentialProvider> = credentials;
        let connector: Arc<dyn CatalogConnector> = connector;
        MoqCatalogSubscriber::start_with_connector(config, credentials, connector).unwrap()
    }

    #[test]
    fn backoff_is_bounded_and_failure_health_has_stable_categories() {
        let config = MoqCatalogSubscriberConfig::new(
            url::Url::parse("moqt://relay.example/tenant/broadcast").unwrap(),
            crate::MoqNamespace::new("tenant", "broadcast").unwrap(),
        );
        assert_eq!(reconnect_backoff(&config, 0), Duration::from_millis(100));
        assert_eq!(reconnect_backoff(&config, 30), Duration::from_secs(5));
        assert_eq!(
            health_issue_for_failure(Some(MoqCatalogSubscriberFailure::CredentialDenied)),
            BroadcastHealthIssue::AuthenticationUnavailable
        );
        assert_eq!(
            health_issue_for_failure(Some(MoqCatalogSubscriberFailure::ProtocolMismatch)),
            BroadcastHealthIssue::VersionMismatch
        );
    }

    #[tokio::test]
    async fn status_is_watchable_and_terminal_transitions_are_immutable() {
        let config = MoqCatalogSubscriberConfig::new(
            url::Url::parse("moqt://relay.example/tenant/broadcast").unwrap(),
            crate::MoqNamespace::new("tenant", "broadcast").unwrap(),
        );
        let status = CatalogSubscriberStatus::new(&config);
        status.failed(MoqCatalogSubscriberFailure::CredentialDenied);
        status.closed();
        let snapshot = status.wait_terminal().await;
        assert_eq!(snapshot.lifecycle, MoqCatalogSubscriberLifecycle::Failed);
        assert_eq!(
            snapshot.failure,
            Some(MoqCatalogSubscriberFailure::CredentialDenied)
        );
    }

    #[tokio::test]
    async fn every_attempt_uses_a_fresh_credential_and_exhausts_a_bounded_budget() {
        let credentials = Arc::new(SequenceCredentials::new([
            Ok(b"attempt-one".to_vec()),
            Ok(b"attempt-two".to_vec()),
        ]));
        let connector = Arc::new(MockConnector::new([
            MockAttempt::Fail(WireCatalogFailure::ConnectFailed),
            MockAttempt::Fail(WireCatalogFailure::ConnectFailed),
        ]));
        let subscriber = start_mock(config(1), Arc::clone(&credentials), Arc::clone(&connector));
        let result = tokio::time::timeout(Duration::from_secs(1), subscriber.wait())
            .await
            .unwrap();
        assert!(matches!(
            result,
            Err(MoqError::CatalogSubscriber(
                MoqCatalogSubscriberFailure::ReconnectExhausted
            ))
        ));
        assert_eq!(credentials.calls.load(Ordering::Acquire), 2);
        assert_eq!(connector.calls.load(Ordering::Acquire), 2);
        let fingerprints = connector
            .fingerprints
            .lock()
            .expect("fingerprints poisoned");
        assert_eq!(fingerprints.len(), 2);
        assert_ne!(fingerprints[0], fingerprints[1]);
    }

    #[tokio::test]
    async fn repeated_credential_fails_before_a_second_network_attempt() {
        let credentials = Arc::new(SequenceCredentials::new([
            Ok(b"reused-token".to_vec()),
            Ok(b"reused-token".to_vec()),
        ]));
        let connector = Arc::new(MockConnector::new([
            MockAttempt::Fail(WireCatalogFailure::ConnectFailed),
            MockAttempt::Connected(MockStream::Events(VecDeque::new())),
        ]));
        let subscriber = start_mock(config(1), Arc::clone(&credentials), Arc::clone(&connector));
        let result = tokio::time::timeout(Duration::from_secs(1), subscriber.wait())
            .await
            .unwrap();
        assert!(matches!(
            result,
            Err(MoqError::CatalogSubscriber(
                MoqCatalogSubscriberFailure::CredentialReused
            ))
        ));
        assert_eq!(credentials.calls.load(Ordering::Acquire), 2);
        assert_eq!(connector.calls.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn connection_timeouts_cancel_attempts_and_exhaust_after_one_retry() {
        let first_dropped = Arc::new(AtomicUsize::new(0));
        let second_dropped = Arc::new(AtomicUsize::new(0));
        let credentials = Arc::new(SequenceCredentials::new([
            Ok(b"one".to_vec()),
            Ok(b"two".to_vec()),
        ]));
        let connector = Arc::new(MockConnector::new([
            MockAttempt::Pending {
                started: Arc::new(Notify::new()),
                dropped: Arc::clone(&first_dropped),
            },
            MockAttempt::Pending {
                started: Arc::new(Notify::new()),
                dropped: Arc::clone(&second_dropped),
            },
        ]));
        let subscriber = start_mock(config(1), credentials, connector);
        let result = tokio::time::timeout(Duration::from_secs(1), subscriber.wait())
            .await
            .unwrap();
        assert!(matches!(
            result,
            Err(MoqError::CatalogSubscriber(
                MoqCatalogSubscriberFailure::ReconnectExhausted
            ))
        ));
        assert_eq!(first_dropped.load(Ordering::Acquire), 1);
        assert_eq!(second_dropped.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn close_cancels_an_in_flight_connection_attempt() {
        let started = Arc::new(Notify::new());
        let dropped = Arc::new(AtomicUsize::new(0));
        let credentials = Arc::new(SequenceCredentials::new([Ok(b"one".to_vec())]));
        let connector = Arc::new(MockConnector::new([MockAttempt::Pending {
            started: Arc::clone(&started),
            dropped: Arc::clone(&dropped),
        }]));
        let subscriber = start_mock(config(5), credentials, connector);
        started.notified().await;
        tokio::time::timeout(Duration::from_secs(1), subscriber.close())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            subscriber.snapshot().lifecycle,
            MoqCatalogSubscriberLifecycle::Closed
        );
        assert_eq!(dropped.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn close_cancels_stream_read_and_closes_the_connection() {
        let stream_started = Arc::new(Notify::new());
        let credentials = Arc::new(SequenceCredentials::new([Ok(b"one".to_vec())]));
        let connector = Arc::new(MockConnector::new([MockAttempt::Connected(
            MockStream::Pending {
                started: Arc::clone(&stream_started),
            },
        )]));
        let subscriber = start_mock(config(0), credentials, Arc::clone(&connector));
        stream_started.notified().await;
        subscriber.close().await.unwrap();
        assert_eq!(connector.closes.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn terminal_catalog_completes_and_closes_without_reconnect() {
        let credentials = Arc::new(SequenceCredentials::new([Ok(b"one".to_vec())]));
        let connector = Arc::new(MockConnector::new([MockAttempt::Connected(
            MockStream::Events(VecDeque::from([Ok(Some(terminal_object(
                MoqEndOfGroupEvidence::Signaled,
            )))])),
        )]));
        let subscriber = start_mock(config(0), credentials, Arc::clone(&connector));
        subscriber.wait().await.unwrap();
        let snapshot = subscriber.snapshot();
        assert_eq!(
            snapshot.lifecycle,
            MoqCatalogSubscriberLifecycle::PermanentlyCompleted
        );
        assert_eq!(
            snapshot.latest.unwrap().catalog.state(),
            MsfCatalogState::PermanentlyCompleted
        );
        assert_eq!(connector.closes.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn retained_fetch_with_unknown_group_end_is_accepted_without_fabrication() {
        let credentials = Arc::new(SequenceCredentials::new([Ok(b"one".to_vec())]));
        let connector = Arc::new(MockConnector::new([MockAttempt::Connected(
            MockStream::Events(VecDeque::from([Ok(Some(terminal_object(
                MoqEndOfGroupEvidence::UnknownFromFetch,
            )))])),
        )]));
        let subscriber = start_mock(config(0), credentials, connector);
        subscriber.wait().await.unwrap();
        let snapshot = subscriber.snapshot();
        assert_eq!(
            snapshot.lifecycle,
            MoqCatalogSubscriberLifecycle::PermanentlyCompleted
        );
        assert_eq!(snapshot.failure, None);
    }

    #[tokio::test]
    async fn dropping_the_final_handle_reaps_an_active_stream() {
        let stream_started = Arc::new(Notify::new());
        let credentials = Arc::new(SequenceCredentials::new([Ok(b"one".to_vec())]));
        let connector = Arc::new(MockConnector::new([MockAttempt::Connected(
            MockStream::Pending {
                started: Arc::clone(&stream_started),
            },
        )]));
        let subscriber = start_mock(config(0), credentials, Arc::clone(&connector));
        stream_started.notified().await;
        let close_notified = connector.close_notify.notified();
        drop(subscriber);
        tokio::time::timeout(Duration::from_secs(1), close_notified)
            .await
            .expect("drop cleanup did not close the active stream");
        assert_eq!(connector.closes.load(Ordering::Acquire), 1);
    }
}
