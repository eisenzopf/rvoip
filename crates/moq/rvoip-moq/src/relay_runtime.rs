//! Managed, production-oriented MOQT relay runtime.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::FutureExt;
use moq_relay_ietf::{
    AdmissionDecision, CertificateFingerprintAdmission, Coordinator, CoordinatorError,
    CoordinatorResult, ListenerSecurityPolicy, Locals, NamespaceInfo, NamespaceOrigin,
    NamespaceRegistration, NamespaceSubscription, NamespaceUpdate, NamespaceUpdateSender, Relay,
    RelayCapacityLimitSet, RelayCapacityLimits, RelayConfig, RelayDiagnostics, RemoteManagerLimits,
    ScopeInfo, ScopePermissions, SessionAdmission,
};
use moq_transport::coding::TrackNamespace;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::{MoqProtocolVersion, MoqRelayAdmissionSubstrate, RvoipMoqRelayAdmission};

/// How the relay runtime is hosted by its owning application.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum MoqRelayDeploymentMode {
    /// Relay tasks share an application process (for example all-in-one mode).
    #[default]
    Embedded,
    /// Relay tasks are the primary workload of a separately managed process.
    Standalone,
}

/// Public, non-sensitive listener role used in diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum MoqRelayListenerKind {
    PublisherMutualTls,
    RelaySubscriberMutualTls,
    SubscriberWebTransport,
    SubscriberRawQuic,
}

/// One reviewed certificate-to-scope binding for an mTLS listener role.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoqRelayCertificateBinding {
    /// Lower- or upper-case SHA-256 fingerprint of the client leaf certificate.
    pub certificate_sha256: String,
    /// Exact path scope this certificate may use, beginning with `/`.
    ///
    /// The enclosing listener role determines whether the binding grants
    /// publish-only or subscribe-only access. A binding never grants both.
    pub scope: String,
}

/// Backward-compatible name for a publisher certificate binding.
pub type MoqRelayPublisherBinding = MoqRelayCertificateBinding;

/// Production admission posture for one relay listener.
///
/// Publisher and subscriber capabilities intentionally cannot share a
/// listener. Token roles require a different TLS posture, while the two mTLS
/// roles use distinct least-privilege certificate claims. Start separate
/// runtimes when more than one role is needed.
#[non_exhaustive]
pub enum MoqRelayRuntimeSecurity {
    PublisherMutualTls {
        bindings: Vec<MoqRelayCertificateBinding>,
        max_active_sessions_per_certificate: usize,
    },
    /// Subscribe-only raw-QUIC listener for a downstream relay.
    ///
    /// This role is intentionally separate from publisher mTLS. Its admitted
    /// certificates may subscribe and fetch within an exact scope but cannot
    /// publish into the upstream relay.
    RelaySubscriberMutualTls {
        bindings: Vec<MoqRelayCertificateBinding>,
        max_active_sessions_per_certificate: usize,
    },
    SubscriberWebTransport {
        admission: Arc<RvoipMoqRelayAdmission>,
    },
    SubscriberRawQuic {
        admission: Arc<RvoipMoqRelayAdmission>,
    },
}

impl MoqRelayRuntimeSecurity {
    pub const fn listener_kind(&self) -> MoqRelayListenerKind {
        match self {
            Self::PublisherMutualTls { .. } => MoqRelayListenerKind::PublisherMutualTls,
            Self::RelaySubscriberMutualTls { .. } => MoqRelayListenerKind::RelaySubscriberMutualTls,
            Self::SubscriberWebTransport { .. } => MoqRelayListenerKind::SubscriberWebTransport,
            Self::SubscriberRawQuic { .. } => MoqRelayListenerKind::SubscriberRawQuic,
        }
    }
}

impl std::fmt::Debug for MoqRelayRuntimeSecurity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PublisherMutualTls {
                bindings,
                max_active_sessions_per_certificate,
            } => formatter
                .debug_struct("PublisherMutualTls")
                .field("binding_count", &bindings.len())
                .field(
                    "max_active_sessions_per_certificate",
                    max_active_sessions_per_certificate,
                )
                .finish(),
            Self::RelaySubscriberMutualTls {
                bindings,
                max_active_sessions_per_certificate,
            } => formatter
                .debug_struct("RelaySubscriberMutualTls")
                .field("binding_count", &bindings.len())
                .field(
                    "max_active_sessions_per_certificate",
                    max_active_sessions_per_certificate,
                )
                .finish(),
            Self::SubscriberWebTransport { .. } => {
                formatter.write_str("SubscriberWebTransport { admission: <redacted> }")
            }
            Self::SubscriberRawQuic { .. } => {
                formatter.write_str("SubscriberRawQuic { admission: <redacted> }")
            }
        }
    }
}

/// File-backed TLS inputs for one relay server and its relay-to-relay client.
#[derive(Clone, Default)]
pub struct MoqRelayServerTlsConfig {
    /// Server certificate chains in PEM format.
    pub server_certificates: Vec<PathBuf>,
    /// Private keys paired positionally with `server_certificates`.
    pub server_private_keys: Vec<PathBuf>,
    /// Roots used to verify outbound origin or relay servers. Empty uses the
    /// operating-system trust store.
    pub server_root_certificates: Vec<PathBuf>,
    /// Client certificate presented for outbound relay-to-relay connections.
    pub outbound_client_certificate: Option<PathBuf>,
    /// Private key for `outbound_client_certificate`.
    pub outbound_client_private_key: Option<PathBuf>,
    /// Explicit roots used to verify inbound publisher or relay-subscriber
    /// client certificates. This must be non-empty only for an mTLS listener.
    ///
    /// The field retains its original publisher-specific name for source
    /// compatibility; listener authorization remains role-specific.
    pub publisher_client_ca_certificates: Vec<PathBuf>,
}

impl std::fmt::Debug for MoqRelayServerTlsConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MoqRelayServerTlsConfig")
            .field("server_certificate_count", &self.server_certificates.len())
            .field("server_private_key_count", &self.server_private_keys.len())
            .field(
                "server_root_certificate_count",
                &self.server_root_certificates.len(),
            )
            .field(
                "has_outbound_client_certificate",
                &self.outbound_client_certificate.is_some(),
            )
            .field(
                "has_outbound_client_private_key",
                &self.outbound_client_private_key.is_some(),
            )
            .field(
                "mutual_tls_client_ca_count",
                &self.publisher_client_ca_certificates.len(),
            )
            .finish()
    }
}

/// Hierarchical limits for retained relay requests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MoqRelayResourceLimits {
    pub total: usize,
    pub publish_namespaces: usize,
    pub publish_tracks: usize,
    pub subscribes: usize,
    pub track_statuses: usize,
    pub fetches: usize,
}

impl From<MoqRelayResourceLimits> for RelayCapacityLimitSet {
    fn from(value: MoqRelayResourceLimits) -> Self {
        Self {
            total: value.total,
            publish_namespaces: value.publish_namespaces,
            publish_tracks: value.publish_tracks,
            subscribes: value.subscribes,
            track_statuses: value.track_statuses,
            fetches: value.fetches,
        }
    }
}

/// All explicit bounds applied by an rvoip relay runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoqRelayRuntimeLimits {
    pub max_pending_admissions: usize,
    pub max_active_sessions: usize,
    pub process: MoqRelayResourceLimits,
    pub per_principal: MoqRelayResourceLimits,
    pub per_scope: MoqRelayResourceLimits,
    pub max_cached_tracks_per_namespace: usize,
    pub max_pending_track_requests_per_namespace: usize,
    pub max_upstream_connections: usize,
    pub max_upstream_tracks: usize,
    pub max_coordinated_namespaces: usize,
}

impl Default for MoqRelayRuntimeLimits {
    fn default() -> Self {
        Self {
            max_pending_admissions: 256,
            max_active_sessions: 2_048,
            process: MoqRelayResourceLimits {
                total: 20_000,
                publish_namespaces: 4_096,
                publish_tracks: 8_192,
                subscribes: 10_000,
                track_statuses: 2_048,
                fetches: 4_096,
            },
            per_principal: MoqRelayResourceLimits {
                total: 2_048,
                publish_namespaces: 256,
                publish_tracks: 1_024,
                subscribes: 1_024,
                track_statuses: 256,
                fetches: 512,
            },
            per_scope: MoqRelayResourceLimits {
                total: 8_192,
                publish_namespaces: 2_048,
                publish_tracks: 4_096,
                subscribes: 4_096,
                track_statuses: 1_024,
                fetches: 2_048,
            },
            max_cached_tracks_per_namespace: 4_096,
            max_pending_track_requests_per_namespace: 1_024,
            max_upstream_connections: 128,
            max_upstream_tracks: 4_096,
            max_coordinated_namespaces: 100_000,
        }
    }
}

/// Explicit bounds for the shared namespace-routing topology.
///
/// These limits are separate from per-listener request limits because several
/// role-specific listeners may share one topology.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MoqRelayTopologyLimits {
    /// Maximum simultaneously registered publisher namespaces.
    pub max_namespaces: usize,
    /// Maximum long-lived namespace-prefix subscriptions.
    pub max_namespace_subscriptions: usize,
    /// Per-subscription capacity for nonblocking Added/Removed updates.
    pub namespace_update_queue_capacity: usize,
}

impl Default for MoqRelayTopologyLimits {
    fn default() -> Self {
        Self {
            max_namespaces: 100_000,
            max_namespace_subscriptions: 4_096,
            namespace_update_queue_capacity: 256,
        }
    }
}

impl MoqRelayTopologyLimits {
    fn with_namespace_limit(max_namespaces: usize) -> Self {
        Self {
            max_namespaces,
            ..Self::default()
        }
    }

    fn validate(self) -> Result<Self, MoqRelayRuntimeError> {
        if self.max_namespaces == 0
            || self.max_namespace_subscriptions == 0
            || self.namespace_update_queue_capacity == 0
        {
            return Err(MoqRelayRuntimeError::InvalidConfig(
                "relay topology limits must be greater than zero",
            ));
        }
        Ok(self)
    }
}

/// Bounded relay operation and shutdown deadlines.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MoqRelayRuntimeTimeouts {
    pub setup: Duration,
    pub admission: Duration,
    pub pre_admission_cleanup: Duration,
    pub admission_session_close: Duration,
    pub token_revalidation_interval: Duration,
    pub upstream_track_idle: Duration,
    pub upstream_connection_idle: Duration,
    /// Final-handle drop cleanup bound.
    pub drop_cleanup: Duration,
}

impl Default for MoqRelayRuntimeTimeouts {
    fn default() -> Self {
        Self {
            setup: Duration::from_secs(5),
            admission: Duration::from_secs(5),
            pre_admission_cleanup: Duration::from_secs(2),
            admission_session_close: Duration::from_secs(5),
            token_revalidation_interval: Duration::from_secs(15),
            upstream_track_idle: Duration::from_secs(30),
            upstream_connection_idle: Duration::from_secs(60),
            drop_cleanup: Duration::from_secs(10),
        }
    }
}

/// Complete configuration for one role-specific relay listener.
#[derive(Debug)]
pub struct MoqRelayRuntimeConfig {
    pub deployment: MoqRelayDeploymentMode,
    pub bind: SocketAddr,
    /// Canonical public `moqt://` endpoint advertised for this relay.
    pub advertised_endpoint: Url,
    /// Optional routable address used when DNS should be bypassed internally.
    pub advertised_socket_addr: Option<SocketAddr>,
    pub tls: MoqRelayServerTlsConfig,
    pub security: MoqRelayRuntimeSecurity,
    pub limits: MoqRelayRuntimeLimits,
    pub timeouts: MoqRelayRuntimeTimeouts,
}

/// Shared in-process routing state for role-separated relay listeners.
///
/// Production publisher listeners require mutual TLS while subscriber
/// listeners require receive-only SETUP tokens, so those roles cannot safely
/// share one public socket. A topology lets the role-specific runtimes share
/// namespace registration and lookup without exposing `moq-rs` coordinator
/// types to applications.
#[derive(Clone)]
pub struct MoqRelayTopology {
    coordinator: Arc<LocalCoordinator>,
    locals: Locals,
}

impl MoqRelayTopology {
    /// Create a bounded topology whose registered origins route back to the
    /// publisher listener at `publisher_endpoint`.
    pub fn new(
        publisher_endpoint: Url,
        publisher_socket_addr: Option<SocketAddr>,
        max_namespaces: usize,
    ) -> Result<Self, MoqRelayRuntimeError> {
        Self::with_limits(
            publisher_endpoint,
            publisher_socket_addr,
            MoqRelayTopologyLimits::with_namespace_limit(max_namespaces),
        )
    }

    /// Create a topology with explicit namespace and discovery-stream limits.
    pub fn with_limits(
        publisher_endpoint: Url,
        publisher_socket_addr: Option<SocketAddr>,
        limits: MoqRelayTopologyLimits,
    ) -> Result<Self, MoqRelayRuntimeError> {
        validate_authority_endpoint(&publisher_endpoint)?;
        let limits = limits.validate()?;
        Ok(Self {
            coordinator: Arc::new(LocalCoordinator::new(
                publisher_endpoint,
                publisher_socket_addr,
                limits,
            )),
            locals: Locals::new(),
        })
    }

    /// Aggregate-safe count of namespaces currently registered by publishers.
    pub fn coordinated_namespaces(&self) -> usize {
        self.coordinator.snapshot().namespaces
    }

    /// Aggregate-safe count of active namespace-prefix subscriptions.
    pub fn namespace_subscriptions(&self) -> usize {
        self.coordinator.snapshot().namespace_subscriptions
    }
}

impl std::fmt::Debug for MoqRelayTopology {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MoqRelayTopology")
            .field("coordinated_namespaces", &self.coordinated_namespaces())
            .field("namespace_subscriptions", &self.namespace_subscriptions())
            .finish_non_exhaustive()
    }
}

/// Managed relay lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum MoqRelayRuntimeLifecycle {
    Starting,
    Ready,
    Draining,
    Stopped,
    Failed,
}

impl MoqRelayRuntimeLifecycle {
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Stopped | Self::Failed)
    }
}

/// Aggregate-safe relay diagnostics. No principal, tenant, namespace, URL, or
/// credential values are included.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct MoqRelayRuntimeSnapshot {
    pub deployment: MoqRelayDeploymentMode,
    pub listener: MoqRelayListenerKind,
    pub lifecycle: MoqRelayRuntimeLifecycle,
    pub protocol: MoqProtocolVersion,
    pub active_resource_leases: usize,
    pub principal_capacity_buckets: usize,
    pub scope_capacity_buckets: usize,
    pub coordinated_namespaces: usize,
    pub namespace_subscriptions: usize,
    pub cached_upstream_connections: usize,
    pub retained_upstream_connections: usize,
    pub retained_upstream_tracks: usize,
    pub supervised_upstream_tasks: usize,
    pub retained_process_bytes: usize,
    pub max_retained_process_bytes: usize,
}

impl MoqRelayRuntimeSnapshot {
    pub const fn ready(&self) -> bool {
        matches!(self.lifecycle, MoqRelayRuntimeLifecycle::Ready)
    }
}

/// Sanitized relay construction or lifecycle failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum MoqRelayRuntimeError {
    #[error("invalid MOQT relay runtime configuration: {0}")]
    InvalidConfig(&'static str),
    #[error("MOQT relay TLS configuration could not be loaded")]
    TlsConfiguration,
    #[error("MOQT relay listener could not be started")]
    StartFailed,
    #[error("MOQT relay runtime requires an active Tokio runtime")]
    RuntimeUnavailable,
    #[error("MOQT relay runtime failed")]
    RuntimeFailed,
    #[error("MOQT relay drain timed out")]
    DrainTimedOut,
}

/// Cloneable managed relay handle used by embedded and standalone processes.
#[derive(Clone)]
pub struct MoqRelayRuntime {
    inner: Arc<RuntimeInner>,
}

struct RuntimeInner {
    deployment: MoqRelayDeploymentMode,
    listener: MoqRelayListenerKind,
    diagnostics: RelayDiagnostics,
    coordinator: Arc<LocalCoordinator>,
    lifecycle: Arc<RuntimeLifecycle>,
    shutdown: CancellationToken,
    task: Mutex<Option<JoinHandle<()>>>,
    runtime: tokio::runtime::Handle,
    drop_cleanup: Duration,
}

struct RuntimeLifecycle {
    tx: watch::Sender<MoqRelayRuntimeLifecycle>,
}

impl RuntimeLifecycle {
    fn new() -> Self {
        let (tx, _) = watch::channel(MoqRelayRuntimeLifecycle::Starting);
        Self { tx }
    }

    fn current(&self) -> MoqRelayRuntimeLifecycle {
        *self.tx.borrow()
    }

    fn transition(&self, next: MoqRelayRuntimeLifecycle) {
        let current = self.current();
        if current.is_terminal() || current == next {
            return;
        }
        self.tx.send_replace(next);
        metrics::counter!(
            "rvoip_moq_relay_runtime_transitions_total",
            "state" => lifecycle_label(next)
        )
        .increment(1);
    }

    async fn wait_terminal(&self) -> MoqRelayRuntimeLifecycle {
        let mut rx = self.tx.subscribe();
        loop {
            let current = *rx.borrow_and_update();
            if current.is_terminal() {
                return current;
            }
            if rx.changed().await.is_err() {
                return self.current();
            }
        }
    }
}

impl MoqRelayRuntime {
    /// Bind and supervise one production relay listener.
    pub fn start(config: MoqRelayRuntimeConfig) -> Result<Self, MoqRelayRuntimeError> {
        let topology = MoqRelayTopology::new(
            config.advertised_endpoint.clone(),
            config.advertised_socket_addr,
            config.limits.max_coordinated_namespaces,
        )?;
        Self::start_with_topology(config, topology)
    }

    /// Bind one role-specific listener into shared in-process routing state.
    ///
    /// Publisher and subscriber listeners that belong to the same embedded
    /// relay deployment must use the same topology. The topology's publisher
    /// endpoint must match the publisher runtime's advertised endpoint.
    pub fn start_with_topology(
        config: MoqRelayRuntimeConfig,
        topology: MoqRelayTopology,
    ) -> Result<Self, MoqRelayRuntimeError> {
        validate_config(&config)?;
        if matches!(
            &config.security,
            MoqRelayRuntimeSecurity::PublisherMutualTls { .. }
        ) && (topology.coordinator.advertised_endpoint != config.advertised_endpoint
            || topology.coordinator.advertised_socket_addr != config.advertised_socket_addr)
        {
            return Err(MoqRelayRuntimeError::InvalidConfig(
                "publisher runtime endpoint must match its shared relay topology",
            ));
        }
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| MoqRelayRuntimeError::RuntimeUnavailable)?;
        let listener = config.security.listener_kind();
        let tls = load_tls(&config)?;
        let (listener_security, admission) = admission_for(&config.security)?;
        let coordinator = topology.coordinator;
        let relay = Relay::new_with_locals(
            RelayConfig {
                bind: Some(config.bind),
                endpoints: Vec::new(),
                tls,
                qlog_dir: None,
                mlog_dir: None,
                announce: None,
                node: Some(config.advertised_endpoint),
                coordinator: coordinator.clone(),
                admission,
                development: false,
                listener_security,
                setup_timeout: config.timeouts.setup,
                admission_timeout: config.timeouts.admission,
                cleanup_timeout: config.timeouts.pre_admission_cleanup,
                session_close_timeout: config.timeouts.admission_session_close,
                max_pending_admissions: config.limits.max_pending_admissions,
                max_active_sessions: config.limits.max_active_sessions,
                token_revalidation_interval: config.timeouts.token_revalidation_interval,
                capacity_limits: RelayCapacityLimits {
                    process: config.limits.process.into(),
                    per_principal: config.limits.per_principal.into(),
                    per_scope: config.limits.per_scope.into(),
                },
                remote_limits: RemoteManagerLimits {
                    max_connections: config.limits.max_upstream_connections,
                    max_tracks: config.limits.max_upstream_tracks,
                    track_idle_timeout: config.timeouts.upstream_track_idle,
                    connection_idle_timeout: config.timeouts.upstream_connection_idle,
                },
                tracks_limits: moq_transport::serve::TracksLimits {
                    max_cached_tracks: config.limits.max_cached_tracks_per_namespace,
                    max_pending_requests: config.limits.max_pending_track_requests_per_namespace,
                },
                request_limits: moq_transport::session::RequestLimits::default(),
            },
            topology.locals,
        )
        .map_err(|_| MoqRelayRuntimeError::StartFailed)?;
        let diagnostics = relay.diagnostics();
        let lifecycle = Arc::new(RuntimeLifecycle::new());
        let shutdown = CancellationToken::new();
        let task_lifecycle = lifecycle.clone();
        let task_shutdown = shutdown.clone();
        let (start_tx, start_rx) = tokio::sync::oneshot::channel();
        let task = runtime.spawn(async move {
            let _ = start_rx.await;
            let result = AssertUnwindSafe(relay.run_until(task_shutdown.clone()))
                .catch_unwind()
                .await;
            match result {
                Ok(Ok(())) => task_lifecycle.transition(MoqRelayRuntimeLifecycle::Stopped),
                Ok(Err(_)) | Err(_) => {
                    metrics::counter!("rvoip_moq_relay_runtime_failures_total").increment(1);
                    task_lifecycle.transition(MoqRelayRuntimeLifecycle::Failed);
                }
            }
        });
        let inner = Arc::new(RuntimeInner {
            deployment: config.deployment,
            listener,
            diagnostics,
            coordinator,
            lifecycle,
            shutdown,
            task: Mutex::new(Some(task)),
            runtime,
            drop_cleanup: config.timeouts.drop_cleanup,
        });
        inner.lifecycle.transition(MoqRelayRuntimeLifecycle::Ready);
        let _ = start_tx.send(());
        Ok(Self { inner })
    }

    pub fn lifecycle(&self) -> MoqRelayRuntimeLifecycle {
        self.inner.lifecycle.current()
    }

    pub const fn protocol_version(&self) -> MoqProtocolVersion {
        MoqProtocolVersion::PINNED
    }

    /// Capture bounded aggregate diagnostics from the running relay.
    pub async fn snapshot(&self) -> MoqRelayRuntimeSnapshot {
        let wire = self.inner.diagnostics.snapshot().await;
        let topology = self.inner.coordinator.snapshot();
        MoqRelayRuntimeSnapshot {
            deployment: self.inner.deployment,
            listener: self.inner.listener,
            lifecycle: self.lifecycle(),
            protocol: MoqProtocolVersion::PINNED,
            active_resource_leases: wire.capacity.active,
            principal_capacity_buckets: wire.capacity.principal_buckets,
            scope_capacity_buckets: wire.capacity.scope_buckets,
            coordinated_namespaces: topology.namespaces,
            namespace_subscriptions: topology.namespace_subscriptions,
            cached_upstream_connections: wire.remotes.cached_connections,
            retained_upstream_connections: wire.remotes.retained_connections,
            retained_upstream_tracks: wire.remotes.retained_tracks,
            supervised_upstream_tasks: wire.remotes.supervised_tasks,
            retained_process_bytes: wire.retained_process_bytes,
            max_retained_process_bytes: wire.max_retained_process_bytes,
        }
    }

    /// Wait until the relay reaches a terminal state.
    pub async fn wait(&self) -> Result<(), MoqRelayRuntimeError> {
        match self.inner.lifecycle.wait_terminal().await {
            MoqRelayRuntimeLifecycle::Stopped => Ok(()),
            MoqRelayRuntimeLifecycle::Failed => Err(MoqRelayRuntimeError::RuntimeFailed),
            _ => Err(MoqRelayRuntimeError::RuntimeFailed),
        }
    }

    /// Stop accepting sessions and await bounded admitted-session cleanup.
    pub async fn drain(&self, timeout: Duration) -> Result<(), MoqRelayRuntimeError> {
        if timeout.is_zero() {
            return Err(MoqRelayRuntimeError::InvalidConfig(
                "relay drain timeout must be greater than zero",
            ));
        }
        if !self.lifecycle().is_terminal() {
            self.inner
                .lifecycle
                .transition(MoqRelayRuntimeLifecycle::Draining);
            self.inner.shutdown.cancel();
        }
        match tokio::time::timeout(timeout, self.inner.lifecycle.wait_terminal()).await {
            Ok(MoqRelayRuntimeLifecycle::Stopped) => {
                self.reap_completed_task().await;
                Ok(())
            }
            Ok(MoqRelayRuntimeLifecycle::Failed) => {
                self.reap_completed_task().await;
                Err(MoqRelayRuntimeError::RuntimeFailed)
            }
            Ok(_) => Err(MoqRelayRuntimeError::RuntimeFailed),
            Err(_) => {
                self.abort_and_reap_task().await;
                self.inner
                    .lifecycle
                    .transition(MoqRelayRuntimeLifecycle::Failed);
                Err(MoqRelayRuntimeError::DrainTimedOut)
            }
        }
    }

    async fn reap_completed_task(&self) {
        let task = take_task(&self.inner.task);
        if let Some(task) = task {
            let _ = task.await;
        }
    }

    async fn abort_and_reap_task(&self) {
        let task = take_task(&self.inner.task);
        if let Some(task) = task {
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for RuntimeInner {
    fn drop(&mut self) {
        if !self.lifecycle.current().is_terminal() {
            self.lifecycle
                .transition(MoqRelayRuntimeLifecycle::Draining);
        }
        self.shutdown.cancel();
        let Some(mut task) = take_task(&self.task) else {
            return;
        };
        let timeout = self.drop_cleanup;
        self.runtime.spawn(async move {
            if tokio::time::timeout(timeout, &mut task).await.is_err() {
                task.abort();
                let _ = task.await;
            }
        });
    }
}

fn take_task(task: &Mutex<Option<JoinHandle<()>>>) -> Option<JoinHandle<()>> {
    task.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
}

fn lifecycle_label(lifecycle: MoqRelayRuntimeLifecycle) -> &'static str {
    match lifecycle {
        MoqRelayRuntimeLifecycle::Starting => "starting",
        MoqRelayRuntimeLifecycle::Ready => "ready",
        MoqRelayRuntimeLifecycle::Draining => "draining",
        MoqRelayRuntimeLifecycle::Stopped => "stopped",
        MoqRelayRuntimeLifecycle::Failed => "failed",
    }
}

fn validate_config(config: &MoqRelayRuntimeConfig) -> Result<(), MoqRelayRuntimeError> {
    validate_authority_endpoint(&config.advertised_endpoint)?;
    if config.tls.server_certificates.is_empty()
        || config.tls.server_certificates.len() != config.tls.server_private_keys.len()
    {
        return Err(MoqRelayRuntimeError::InvalidConfig(
            "relay TLS requires an equal non-zero number of server certificates and keys",
        ));
    }
    if config.tls.outbound_client_certificate.is_some()
        != config.tls.outbound_client_private_key.is_some()
    {
        return Err(MoqRelayRuntimeError::InvalidConfig(
            "outbound relay client certificate and key must be configured together",
        ));
    }
    for timeout in [
        config.timeouts.setup,
        config.timeouts.admission,
        config.timeouts.pre_admission_cleanup,
        config.timeouts.admission_session_close,
        config.timeouts.token_revalidation_interval,
        config.timeouts.upstream_track_idle,
        config.timeouts.upstream_connection_idle,
        config.timeouts.drop_cleanup,
    ] {
        if timeout.is_zero() {
            return Err(MoqRelayRuntimeError::InvalidConfig(
                "relay timeouts must be greater than zero",
            ));
        }
    }
    if config.limits.max_coordinated_namespaces == 0
        || config.limits.max_cached_tracks_per_namespace == 0
        || config.limits.max_pending_track_requests_per_namespace == 0
        || config.limits.max_upstream_connections == 0
        || config.limits.max_upstream_tracks == 0
    {
        return Err(MoqRelayRuntimeError::InvalidConfig(
            "relay retained-state limits must be greater than zero",
        ));
    }
    match &config.security {
        MoqRelayRuntimeSecurity::PublisherMutualTls {
            bindings,
            max_active_sessions_per_certificate,
        } => {
            if bindings.is_empty() || *max_active_sessions_per_certificate == 0 {
                return Err(MoqRelayRuntimeError::InvalidConfig(
                    "publisher mTLS requires bindings and a positive per-certificate session limit",
                ));
            }
            if config.tls.publisher_client_ca_certificates.is_empty() {
                return Err(MoqRelayRuntimeError::InvalidConfig(
                    "publisher mTLS requires at least one explicit client CA",
                ));
            }
        }
        MoqRelayRuntimeSecurity::RelaySubscriberMutualTls {
            bindings,
            max_active_sessions_per_certificate,
        } => {
            if bindings.is_empty() || *max_active_sessions_per_certificate == 0 {
                return Err(MoqRelayRuntimeError::InvalidConfig(
                    "relay-subscriber mTLS requires bindings and a positive per-certificate session limit",
                ));
            }
            if config.tls.publisher_client_ca_certificates.is_empty() {
                return Err(MoqRelayRuntimeError::InvalidConfig(
                    "relay-subscriber mTLS requires at least one explicit client CA",
                ));
            }
        }
        MoqRelayRuntimeSecurity::SubscriberWebTransport { admission } => {
            if admission.config().subscriber_substrate != MoqRelayAdmissionSubstrate::WebTransport {
                return Err(MoqRelayRuntimeError::InvalidConfig(
                    "WebTransport listener requires WebTransport admission",
                ));
            }
            if !config.tls.publisher_client_ca_certificates.is_empty() {
                return Err(MoqRelayRuntimeError::InvalidConfig(
                    "subscriber listeners cannot configure publisher client CAs",
                ));
            }
        }
        MoqRelayRuntimeSecurity::SubscriberRawQuic { admission } => {
            if admission.config().subscriber_substrate != MoqRelayAdmissionSubstrate::RawQuic {
                return Err(MoqRelayRuntimeError::InvalidConfig(
                    "raw QUIC listener requires raw QUIC admission",
                ));
            }
            if !config.tls.publisher_client_ca_certificates.is_empty() {
                return Err(MoqRelayRuntimeError::InvalidConfig(
                    "subscriber listeners cannot configure publisher client CAs",
                ));
            }
        }
    }
    Ok(())
}

fn validate_authority_endpoint(endpoint: &Url) -> Result<(), MoqRelayRuntimeError> {
    if endpoint.scheme() != "moqt"
        || endpoint.host_str().is_none_or(str::is_empty)
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || !matches!(endpoint.path(), "" | "/")
    {
        return Err(MoqRelayRuntimeError::InvalidConfig(
            "advertised endpoint must be a credential-free authority-only moqt:// URL",
        ));
    }
    Ok(())
}

fn load_tls(
    config: &MoqRelayRuntimeConfig,
) -> Result<moq_native_ietf::tls::Config, MoqRelayRuntimeError> {
    let client_auth = match config.security {
        MoqRelayRuntimeSecurity::PublisherMutualTls { .. }
        | MoqRelayRuntimeSecurity::RelaySubscriberMutualTls { .. } => {
            moq_native_ietf::tls::ClientAuthMode::Required
        }
        MoqRelayRuntimeSecurity::SubscriberWebTransport { .. }
        | MoqRelayRuntimeSecurity::SubscriberRawQuic { .. } => {
            moq_native_ietf::tls::ClientAuthMode::Disabled
        }
    };
    moq_native_ietf::tls::Args {
        cert: config.tls.server_certificates.clone(),
        key: config.tls.server_private_keys.clone(),
        root: config.tls.server_root_certificates.clone(),
        client_cert: config.tls.outbound_client_certificate.clone(),
        client_key: config.tls.outbound_client_private_key.clone(),
        client_auth,
        client_ca: config.tls.publisher_client_ca_certificates.clone(),
        disable_verify: false,
    }
    .load()
    .map_err(|_| MoqRelayRuntimeError::TlsConfiguration)
}

fn admission_for(
    security: &MoqRelayRuntimeSecurity,
) -> Result<(ListenerSecurityPolicy, Arc<dyn SessionAdmission>), MoqRelayRuntimeError> {
    match security {
        MoqRelayRuntimeSecurity::PublisherMutualTls {
            bindings,
            max_active_sessions_per_certificate,
        } => {
            let bindings = bindings
                .iter()
                .map(|binding| format!("{}={}", binding.certificate_sha256, binding.scope));
            let admission = CertificateFingerprintAdmission::new_bindings_with_limit(
                bindings,
                *max_active_sessions_per_certificate,
            )
            .map_err(|_| MoqRelayRuntimeError::InvalidConfig("invalid publisher mTLS binding"))?;
            Ok((ListenerSecurityPolicy::MutualTlsPublisher, admission))
        }
        MoqRelayRuntimeSecurity::RelaySubscriberMutualTls {
            bindings,
            max_active_sessions_per_certificate,
        } => {
            let bindings = bindings
                .iter()
                .map(|binding| format!("{}={}", binding.certificate_sha256, binding.scope));
            let admission =
                CertificateFingerprintAdmission::new_relay_subscriber_bindings_with_limit(
                    bindings,
                    *max_active_sessions_per_certificate,
                )
                .map_err(|_| {
                    MoqRelayRuntimeError::InvalidConfig("invalid relay-subscriber mTLS binding")
                })?;
            Ok((ListenerSecurityPolicy::MutualTlsRelaySubscriber, admission))
        }
        MoqRelayRuntimeSecurity::SubscriberWebTransport { admission } => {
            Ok((ListenerSecurityPolicy::TokenSubscriber, admission.clone()))
        }
        MoqRelayRuntimeSecurity::SubscriberRawQuic { admission } => Ok((
            ListenerSecurityPolicy::RawQuicTokenSubscriber,
            admission.clone(),
        )),
    }
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct NamespaceKey {
    scope: Option<String>,
    namespace: TrackNamespace,
}

struct LocalOrigin {
    registration_id: u64,
    url: Url,
    addr: Option<SocketAddr>,
}

struct LocalNamespaceSubscription {
    scope: Option<String>,
    prefix: TrackNamespace,
    updates: NamespaceUpdateSender,
}

impl LocalNamespaceSubscription {
    fn matches(&self, scope: Option<&str>, namespace: &TrackNamespace) -> bool {
        self.scope.as_deref() == scope
            && namespace.fields.len() >= self.prefix.fields.len()
            && self
                .prefix
                .fields
                .iter()
                .zip(&namespace.fields)
                .all(|(expected, actual)| expected == actual)
    }
}

#[derive(Clone, Copy)]
enum LocalNamespaceUpdateKind {
    Added,
    Removed,
}

struct LocalCoordinatorState {
    namespaces: HashMap<NamespaceKey, LocalOrigin>,
    namespace_subscriptions: HashMap<u64, LocalNamespaceSubscription>,
}

impl LocalCoordinatorState {
    fn notify_namespace_change(
        &mut self,
        scope: Option<&str>,
        namespace: &TrackNamespace,
        kind: LocalNamespaceUpdateKind,
    ) {
        self.namespace_subscriptions.retain(|_, subscription| {
            if !subscription.matches(scope, namespace) {
                return true;
            }
            let info = NamespaceInfo::new(namespace.clone());
            let update = match kind {
                LocalNamespaceUpdateKind::Added => NamespaceUpdate::Added(info),
                LocalNamespaceUpdateKind::Removed => NamespaceUpdate::Removed(info),
            };
            // The fork's bounded sender permanently marks an overflowing
            // stream failed. Removing it here also releases topology capacity
            // immediately instead of retaining stale discovery state.
            subscription.updates.try_send(update).is_ok()
        });
    }
}

struct LocalCoordinator {
    state: Arc<Mutex<LocalCoordinatorState>>,
    advertised_endpoint: Url,
    advertised_socket_addr: Option<SocketAddr>,
    next_registration_id: AtomicU64,
    next_subscription_id: AtomicU64,
    limits: MoqRelayTopologyLimits,
}

impl LocalCoordinator {
    fn new(
        advertised_endpoint: Url,
        advertised_socket_addr: Option<SocketAddr>,
        limits: MoqRelayTopologyLimits,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(LocalCoordinatorState {
                namespaces: HashMap::new(),
                namespace_subscriptions: HashMap::new(),
            })),
            advertised_endpoint,
            advertised_socket_addr,
            next_registration_id: AtomicU64::new(1),
            next_subscription_id: AtomicU64::new(1),
            limits,
        }
    }

    fn snapshot(&self) -> LocalCoordinatorSnapshot {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        LocalCoordinatorSnapshot {
            namespaces: state.namespaces.len(),
            namespace_subscriptions: state.namespace_subscriptions.len(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LocalCoordinatorSnapshot {
    namespaces: usize,
    namespace_subscriptions: usize,
}

struct LocalRegistration {
    state: Arc<Mutex<LocalCoordinatorState>>,
    key: NamespaceKey,
    registration_id: u64,
}

impl Drop for LocalRegistration {
    fn drop(&mut self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state
            .namespaces
            .get(&self.key)
            .is_some_and(|origin| origin.registration_id == self.registration_id)
        {
            state.namespaces.remove(&self.key);
            state.notify_namespace_change(
                self.key.scope.as_deref(),
                &self.key.namespace,
                LocalNamespaceUpdateKind::Removed,
            );
        }
    }
}

struct LocalNamespaceSubscriptionRegistration {
    state: Arc<Mutex<LocalCoordinatorState>>,
    subscription_id: u64,
}

impl Drop for LocalNamespaceSubscriptionRegistration {
    fn drop(&mut self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .namespace_subscriptions
            .remove(&self.subscription_id);
    }
}

#[async_trait]
impl Coordinator for LocalCoordinator {
    async fn resolve_admitted_scope(
        &self,
        admission: &AdmissionDecision,
        connection_path: Option<&str>,
    ) -> CoordinatorResult<Option<ScopeInfo>> {
        let path = connection_path.ok_or_else(|| {
            CoordinatorError::from(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "role-separated relay sessions require a namespace path",
            ))
        })?;
        if !path.starts_with('/')
            || path.len() > moq_relay_ietf::AdmissionClaims::MAX_SCOPE_BYTES
            || path.contains(['?', '#'])
            || (admission.principal.method == moq_relay_ietf::AuthenticationMethod::MutualTls
                && admission.claims.scope.as_deref() != Some(path))
        {
            return Err(CoordinatorError::from(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid admitted relay scope",
            )));
        }
        let permissions = match (admission.claims.publish, admission.claims.subscribe) {
            (true, false) => ScopePermissions::ReadWrite,
            (false, true) => ScopePermissions::ReadOnly,
            _ => {
                return Err(CoordinatorError::from(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "relay admission must grant exactly one listener role",
                )));
            }
        };
        Ok(Some(ScopeInfo {
            scope_id: path.to_owned(),
            permissions,
        }))
    }

    async fn register_namespace(
        &self,
        scope: Option<&str>,
        namespace: &TrackNamespace,
    ) -> CoordinatorResult<NamespaceRegistration> {
        let namespace_path = namespace.to_utf8_path();
        if scope != Some(namespace_path.as_str()) {
            return Err(CoordinatorError::from(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "publisher scope does not match its namespace",
            )));
        }
        let key = NamespaceKey {
            scope: scope.map(str::to_owned),
            namespace: namespace.clone(),
        };
        let registration_id = self.next_registration_id.fetch_add(1, Ordering::Relaxed);
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.namespaces.contains_key(&key) {
            return Err(CoordinatorError::NamespaceAlreadyRegistered);
        }
        if state.namespaces.len() >= self.limits.max_namespaces {
            return Err(CoordinatorError::CapacityExhausted {
                resource: "local_namespaces",
            });
        }
        let mut url = self.advertised_endpoint.clone();
        url.set_path(&namespace_path);
        state.namespaces.insert(
            key.clone(),
            LocalOrigin {
                registration_id,
                url,
                addr: self.advertised_socket_addr,
            },
        );
        state.notify_namespace_change(
            key.scope.as_deref(),
            &key.namespace,
            LocalNamespaceUpdateKind::Added,
        );
        drop(state);
        Ok(NamespaceRegistration::new(LocalRegistration {
            state: self.state.clone(),
            key,
            registration_id,
        }))
    }

    async fn unregister_namespace(
        &self,
        scope: Option<&str>,
        namespace: &TrackNamespace,
    ) -> CoordinatorResult<()> {
        let key = NamespaceKey {
            scope: scope.map(str::to_owned),
            namespace: namespace.clone(),
        };
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.namespaces.remove(&key).is_some() {
            state.notify_namespace_change(
                key.scope.as_deref(),
                &key.namespace,
                LocalNamespaceUpdateKind::Removed,
            );
        }
        Ok(())
    }

    async fn lookup(
        &self,
        scope: Option<&str>,
        namespace: &TrackNamespace,
    ) -> CoordinatorResult<(NamespaceOrigin, Option<moq_native_ietf::quic::Client>)> {
        let key = NamespaceKey {
            scope: scope.map(str::to_owned),
            namespace: namespace.clone(),
        };
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let origin = state
            .namespaces
            .get(&key)
            .ok_or(CoordinatorError::NamespaceNotFound)?;
        Ok((
            NamespaceOrigin::new(namespace.clone(), origin.url.clone(), origin.addr),
            None,
        ))
    }

    async fn subscribe_namespace(
        &self,
        scope: Option<&str>,
        prefix: &TrackNamespace,
    ) -> CoordinatorResult<NamespaceSubscription> {
        let subscription_id = self.next_subscription_id.fetch_add(1, Ordering::Relaxed);
        let registration = LocalNamespaceSubscriptionRegistration {
            state: self.state.clone(),
            subscription_id,
        };
        let (mut subscription, updates) = NamespaceSubscription::bounded(
            Vec::new(),
            registration,
            self.limits.namespace_update_queue_capacity,
        )
        .map_err(|error| CoordinatorError::Other(error.into()))?;

        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.namespace_subscriptions.len() >= self.limits.max_namespace_subscriptions {
            // Drop the subscription only after releasing the mutex because its
            // RAII registration removes itself through the same mutex.
            drop(state);
            drop(subscription);
            return Err(CoordinatorError::CapacityExhausted {
                resource: "local_namespace_subscriptions",
            });
        }

        let scope = scope.map(str::to_owned);
        let candidate = LocalNamespaceSubscription {
            scope,
            prefix: prefix.clone(),
            updates,
        };
        let mut existing = state
            .namespaces
            .keys()
            .filter(|key| candidate.matches(key.scope.as_deref(), &key.namespace))
            .map(|key| NamespaceInfo::new(key.namespace.clone()))
            .collect::<Vec<_>>();
        existing.sort_by_key(|info| info.namespace.to_utf8_path());
        subscription.existing_namespaces = existing;
        state
            .namespace_subscriptions
            .insert(subscription_id, candidate);
        drop(state);
        Ok(subscription)
    }

    async fn unsubscribe_namespace(
        &self,
        scope: Option<&str>,
        prefix: &TrackNamespace,
    ) -> CoordinatorResult<()> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .namespace_subscriptions
            .retain(|_, subscription| {
                subscription.scope.as_deref() != scope || subscription.prefix != *prefix
            });
        Ok(())
    }

    async fn shutdown(&self) -> CoordinatorResult<()> {
        // Runtime listeners can share this coordinator. Their namespace
        // registration handles remove exact entries as publisher sessions
        // close; stopping a subscriber listener must not erase live origins.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestFiles {
        directory: PathBuf,
        certificate: PathBuf,
        private_key: PathBuf,
    }

    impl TestFiles {
        fn new() -> Self {
            static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);
            let generated = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
            let directory = std::env::temp_dir().join(format!(
                "rvoip-moq-relay-runtime-{}-{}",
                std::process::id(),
                NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&directory).unwrap();
            let certificate = directory.join("identity.pem");
            let private_key = directory.join("identity.key");
            std::fs::write(&certificate, generated.cert.pem()).unwrap();
            std::fs::write(&private_key, generated.signing_key.serialize_pem()).unwrap();
            Self {
                directory,
                certificate,
                private_key,
            }
        }

        fn tls(&self) -> MoqRelayServerTlsConfig {
            MoqRelayServerTlsConfig {
                server_certificates: vec![self.certificate.clone()],
                server_private_keys: vec![self.private_key.clone()],
                server_root_certificates: vec![self.certificate.clone()],
                outbound_client_certificate: Some(self.certificate.clone()),
                outbound_client_private_key: Some(self.private_key.clone()),
                publisher_client_ca_certificates: vec![self.certificate.clone()],
            }
        }
    }

    impl Drop for TestFiles {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn defaults_are_bounded_and_protocol_is_explicit() {
        let limits = MoqRelayRuntimeLimits::default();
        let topology_limits = MoqRelayTopologyLimits::default();
        assert!(limits.max_active_sessions > 0);
        assert!(limits.max_pending_admissions > 0);
        assert!(limits.max_coordinated_namespaces > 0);
        assert!(topology_limits.max_namespaces > 0);
        assert!(topology_limits.max_namespace_subscriptions > 0);
        assert!(topology_limits.namespace_update_queue_capacity > 0);
        assert_eq!(crate::MOQT_NEGOTIATED_PROTOCOL, "moqt-19");
        assert_eq!(MoqProtocolVersion::PINNED.transport, 19);
    }

    #[test]
    fn tls_debug_never_prints_paths() {
        let tls = MoqRelayServerTlsConfig {
            server_certificates: vec![PathBuf::from("/secret/server.pem")],
            server_private_keys: vec![PathBuf::from("/secret/server.key")],
            outbound_client_certificate: Some(PathBuf::from("/secret/client.pem")),
            outbound_client_private_key: Some(PathBuf::from("/secret/client.key")),
            ..MoqRelayServerTlsConfig::default()
        };
        let debug = format!("{tls:?}");
        assert!(!debug.contains("/secret"));
        assert!(debug.contains("server_certificate_count: 1"));
    }

    #[tokio::test]
    async fn local_coordinator_is_bounded_scoped_and_raii_cleaned() {
        let coordinator = LocalCoordinator::new(
            Url::parse("moqt://relay.test:443").unwrap(),
            Some("127.0.0.1:443".parse().unwrap()),
            MoqRelayTopologyLimits {
                max_namespaces: 1,
                max_namespace_subscriptions: 2,
                namespace_update_queue_capacity: 2,
            },
        );
        let namespace = TrackNamespace::from_utf8_path("tenant/broadcast");
        let registration = coordinator
            .register_namespace(Some("/tenant/broadcast"), &namespace)
            .await
            .unwrap();
        assert_eq!(coordinator.snapshot().namespaces, 1);
        assert!(coordinator
            .lookup(Some("/other/broadcast"), &namespace)
            .await
            .is_err());
        let other = TrackNamespace::from_utf8_path("tenant/other");
        assert!(matches!(
            coordinator
                .register_namespace(Some("/tenant/other"), &other)
                .await,
            Err(CoordinatorError::CapacityExhausted { .. })
        ));
        drop(registration);
        assert_eq!(coordinator.snapshot().namespaces, 0);
    }

    #[tokio::test]
    async fn namespace_subscriptions_are_scoped_live_and_raii_cleaned() {
        let coordinator = LocalCoordinator::new(
            Url::parse("moqt://relay.test:443").unwrap(),
            None,
            MoqRelayTopologyLimits {
                max_namespaces: 4,
                max_namespace_subscriptions: 2,
                namespace_update_queue_capacity: 2,
            },
        );
        let namespace = TrackNamespace::from_utf8_path("tenant/broadcast");
        let cross_scope = TrackNamespace::from_utf8_path("other/broadcast");
        let registration = coordinator
            .register_namespace(Some("/tenant/broadcast"), &namespace)
            .await
            .unwrap();
        let _cross_scope_registration = coordinator
            .register_namespace(Some("/other/broadcast"), &cross_scope)
            .await
            .unwrap();

        let prefix = TrackNamespace::from_utf8_path("tenant");
        let mut subscription = coordinator
            .subscribe_namespace(Some("/tenant/broadcast"), &prefix)
            .await
            .unwrap();
        assert_eq!(
            subscription.existing_namespaces,
            vec![NamespaceInfo::new(namespace.clone())]
        );
        assert_eq!(coordinator.snapshot().namespace_subscriptions, 1);

        drop(registration);
        assert!(matches!(
            subscription.next_update().await.unwrap(),
            NamespaceUpdate::Removed(info) if info.namespace == namespace
        ));
        let _replacement = coordinator
            .register_namespace(Some("/tenant/broadcast"), &namespace)
            .await
            .unwrap();
        assert!(matches!(
            subscription.next_update().await.unwrap(),
            NamespaceUpdate::Added(info) if info.namespace == namespace
        ));

        drop(subscription);
        assert_eq!(coordinator.snapshot().namespace_subscriptions, 0);
    }

    #[tokio::test]
    async fn namespace_subscription_capacity_and_overflow_fail_closed() {
        let coordinator = LocalCoordinator::new(
            Url::parse("moqt://relay.test:443").unwrap(),
            None,
            MoqRelayTopologyLimits {
                max_namespaces: 2,
                max_namespace_subscriptions: 1,
                namespace_update_queue_capacity: 1,
            },
        );
        let namespace = TrackNamespace::from_utf8_path("tenant/broadcast");
        let prefix = TrackNamespace::from_utf8_path("tenant");
        let mut subscription = coordinator
            .subscribe_namespace(Some("/tenant/broadcast"), &prefix)
            .await
            .unwrap();
        assert!(matches!(
            coordinator
                .subscribe_namespace(Some("/tenant/broadcast"), &prefix)
                .await,
            Err(CoordinatorError::CapacityExhausted {
                resource: "local_namespace_subscriptions"
            })
        ));

        let registration = coordinator
            .register_namespace(Some("/tenant/broadcast"), &namespace)
            .await
            .unwrap();
        drop(registration);
        assert_eq!(coordinator.snapshot().namespace_subscriptions, 0);
        assert!(matches!(
            subscription.next_update().await,
            Err(CoordinatorError::CapacityExhausted {
                resource: "namespace_update_stream"
            })
        ));
    }

    #[test]
    fn mutual_tls_listener_roles_map_to_distinct_least_privilege_policies() {
        let binding = MoqRelayCertificateBinding {
            certificate_sha256: "ab".repeat(32),
            scope: "/tenant/broadcast".to_string(),
        };
        let publisher = MoqRelayRuntimeSecurity::PublisherMutualTls {
            bindings: vec![binding.clone()],
            max_active_sessions_per_certificate: 2,
        };
        let relay_subscriber = MoqRelayRuntimeSecurity::RelaySubscriberMutualTls {
            bindings: vec![binding],
            max_active_sessions_per_certificate: 2,
        };

        let (publisher_policy, _) = admission_for(&publisher).unwrap();
        let (relay_policy, _) = admission_for(&relay_subscriber).unwrap();
        assert_eq!(
            publisher.listener_kind(),
            MoqRelayListenerKind::PublisherMutualTls
        );
        assert_eq!(
            relay_subscriber.listener_kind(),
            MoqRelayListenerKind::RelaySubscriberMutualTls
        );
        assert_eq!(publisher_policy, ListenerSecurityPolicy::MutualTlsPublisher);
        assert_eq!(
            relay_policy,
            ListenerSecurityPolicy::MutualTlsRelaySubscriber
        );
        assert_ne!(publisher_policy, relay_policy);
    }

    #[tokio::test]
    async fn admitted_scope_preserves_role_and_exact_certificate_scope() {
        let coordinator = LocalCoordinator::new(
            Url::parse("moqt://relay.test:443").unwrap(),
            None,
            MoqRelayTopologyLimits::default(),
        );
        let decision = |publish, subscribe, scope: &str| {
            moq_relay_ietf::AdmissionDecision::new(
                moq_relay_ietf::AdmissionPrincipal::new(
                    "certificate-sha256:test",
                    moq_relay_ietf::AuthenticationMethod::MutualTls,
                )
                .unwrap(),
                moq_relay_ietf::AdmissionClaims {
                    scope: Some(scope.to_string()),
                    publish,
                    subscribe,
                    expires_at_unix_seconds: None,
                    token_id: None,
                },
            )
            .unwrap()
        };

        let publisher = coordinator
            .resolve_admitted_scope(
                &decision(true, false, "/tenant/broadcast"),
                Some("/tenant/broadcast"),
            )
            .await
            .unwrap()
            .unwrap();
        let relay_subscriber = coordinator
            .resolve_admitted_scope(
                &decision(false, true, "/tenant/broadcast"),
                Some("/tenant/broadcast"),
            )
            .await
            .unwrap()
            .unwrap();
        assert!(publisher.permissions.can_publish());
        assert!(!relay_subscriber.permissions.can_publish());
        assert!(relay_subscriber.permissions.can_subscribe());
        assert!(coordinator
            .resolve_admitted_scope(
                &decision(false, true, "/tenant/broadcast"),
                Some("/other/broadcast"),
            )
            .await
            .is_err());
        assert!(coordinator
            .resolve_admitted_scope(
                &decision(true, true, "/tenant/broadcast"),
                Some("/tenant/broadcast"),
            )
            .await
            .is_err());
    }

    #[tokio::test]
    async fn relay_subscriber_runtime_requires_mtls_and_reports_its_role() {
        let files = TestFiles::new();
        let mut missing_client_ca = files.tls();
        missing_client_ca.publisher_client_ca_certificates.clear();
        let make_config = |tls| MoqRelayRuntimeConfig {
            deployment: MoqRelayDeploymentMode::Standalone,
            bind: "127.0.0.1:0".parse().unwrap(),
            advertised_endpoint: Url::parse("moqt://localhost:4444").unwrap(),
            advertised_socket_addr: None,
            tls,
            security: MoqRelayRuntimeSecurity::RelaySubscriberMutualTls {
                bindings: vec![MoqRelayCertificateBinding {
                    certificate_sha256: "cd".repeat(32),
                    scope: "/tenant/broadcast".to_string(),
                }],
                max_active_sessions_per_certificate: 2,
            },
            limits: MoqRelayRuntimeLimits::default(),
            timeouts: MoqRelayRuntimeTimeouts::default(),
        };
        assert!(matches!(
            MoqRelayRuntime::start(make_config(missing_client_ca)),
            Err(MoqRelayRuntimeError::InvalidConfig(_))
        ));

        let runtime = MoqRelayRuntime::start(make_config(files.tls())).unwrap();
        let snapshot = runtime.snapshot().await;
        assert!(snapshot.ready());
        assert_eq!(
            snapshot.listener,
            MoqRelayListenerKind::RelaySubscriberMutualTls
        );
        assert_eq!(snapshot.namespace_subscriptions, 0);
        runtime.drain(Duration::from_secs(2)).await.unwrap();
    }

    #[tokio::test]
    async fn managed_runtime_starts_snapshots_and_drains() {
        let files = TestFiles::new();
        let config = MoqRelayRuntimeConfig {
            deployment: MoqRelayDeploymentMode::Embedded,
            bind: "127.0.0.1:0".parse().unwrap(),
            advertised_endpoint: Url::parse("moqt://localhost:4443").unwrap(),
            advertised_socket_addr: Some("127.0.0.1:4443".parse().unwrap()),
            tls: files.tls(),
            security: MoqRelayRuntimeSecurity::PublisherMutualTls {
                bindings: vec![MoqRelayPublisherBinding {
                    certificate_sha256: "00".repeat(32),
                    scope: "/tenant/broadcast".to_string(),
                }],
                max_active_sessions_per_certificate: 2,
            },
            limits: MoqRelayRuntimeLimits::default(),
            timeouts: MoqRelayRuntimeTimeouts::default(),
        };
        let runtime = MoqRelayRuntime::start(config).unwrap();
        let snapshot = runtime.snapshot().await;
        assert!(snapshot.ready());
        assert_eq!(snapshot.listener, MoqRelayListenerKind::PublisherMutualTls);
        assert_eq!(snapshot.protocol, MoqProtocolVersion::PINNED);
        runtime.drain(Duration::from_secs(2)).await.unwrap();
        assert_eq!(runtime.lifecycle(), MoqRelayRuntimeLifecycle::Stopped);
    }
}
