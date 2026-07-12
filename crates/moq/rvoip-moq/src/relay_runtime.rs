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
    CertificateFingerprintAdmission, Coordinator, CoordinatorError, CoordinatorResult,
    ListenerSecurityPolicy, NamespaceOrigin, NamespaceRegistration, Relay, RelayCapacityLimitSet,
    RelayCapacityLimits, RelayConfig, RelayDiagnostics, RemoteManagerLimits, SessionAdmission,
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
    SubscriberWebTransport,
    SubscriberRawQuic,
}

/// One reviewed publisher certificate-to-scope binding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoqRelayPublisherBinding {
    /// Lower- or upper-case SHA-256 fingerprint of the client leaf certificate.
    pub certificate_sha256: String,
    /// Exact path scope this certificate may publish, beginning with `/`.
    pub scope: String,
}

/// Production admission posture for one relay listener.
///
/// Publisher and subscriber roles intentionally cannot share a listener: they
/// require different TLS client-authentication postures. Start separate
/// runtimes when more than one role is needed.
pub enum MoqRelayRuntimeSecurity {
    PublisherMutualTls {
        bindings: Vec<MoqRelayPublisherBinding>,
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
    /// Explicit roots used to verify inbound publisher client certificates.
    /// This must be non-empty only for a publisher mTLS listener.
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
                "publisher_client_ca_count",
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
        validate_config(&config)?;
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| MoqRelayRuntimeError::RuntimeUnavailable)?;
        let listener = config.security.listener_kind();
        let tls = load_tls(&config)?;
        let (listener_security, admission) = admission_for(&config.security)?;
        let coordinator = Arc::new(LocalCoordinator::new(
            config.advertised_endpoint.clone(),
            config.advertised_socket_addr,
            config.limits.max_coordinated_namespaces,
        ));
        let relay = Relay::new(RelayConfig {
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
        })
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
        MoqRelayRuntimeSnapshot {
            deployment: self.inner.deployment,
            listener: self.inner.listener,
            lifecycle: self.lifecycle(),
            protocol: MoqProtocolVersion::PINNED,
            active_resource_leases: wire.capacity.active,
            principal_capacity_buckets: wire.capacity.principal_buckets,
            scope_capacity_buckets: wire.capacity.scope_buckets,
            coordinated_namespaces: self.inner.coordinator.len(),
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
    if config.advertised_endpoint.scheme() != "moqt"
        || !config.advertised_endpoint.username().is_empty()
        || config.advertised_endpoint.password().is_some()
        || config.advertised_endpoint.query().is_some()
        || config.advertised_endpoint.fragment().is_some()
        || !matches!(config.advertised_endpoint.path(), "" | "/")
    {
        return Err(MoqRelayRuntimeError::InvalidConfig(
            "advertised endpoint must be a credential-free authority-only moqt:// URL",
        ));
    }
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

fn load_tls(
    config: &MoqRelayRuntimeConfig,
) -> Result<moq_native_ietf::tls::Config, MoqRelayRuntimeError> {
    let client_auth = match config.security {
        MoqRelayRuntimeSecurity::PublisherMutualTls { .. } => {
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

struct LocalCoordinatorState {
    namespaces: HashMap<NamespaceKey, LocalOrigin>,
}

struct LocalCoordinator {
    state: Arc<Mutex<LocalCoordinatorState>>,
    advertised_endpoint: Url,
    advertised_socket_addr: Option<SocketAddr>,
    next_registration_id: AtomicU64,
    max_namespaces: usize,
}

impl LocalCoordinator {
    fn new(
        advertised_endpoint: Url,
        advertised_socket_addr: Option<SocketAddr>,
        max_namespaces: usize,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(LocalCoordinatorState {
                namespaces: HashMap::new(),
            })),
            advertised_endpoint,
            advertised_socket_addr,
            next_registration_id: AtomicU64::new(1),
            max_namespaces,
        }
    }

    fn len(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .namespaces
            .len()
    }
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
        }
    }
}

#[async_trait]
impl Coordinator for LocalCoordinator {
    async fn register_namespace(
        &self,
        scope: Option<&str>,
        namespace: &TrackNamespace,
    ) -> CoordinatorResult<NamespaceRegistration> {
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
        if state.namespaces.len() >= self.max_namespaces {
            return Err(CoordinatorError::CapacityExhausted {
                resource: "local_namespaces",
            });
        }
        state.namespaces.insert(
            key.clone(),
            LocalOrigin {
                registration_id,
                url: self.advertised_endpoint.clone(),
                addr: self.advertised_socket_addr,
            },
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
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .namespaces
            .remove(&key);
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

    async fn shutdown(&self) -> CoordinatorResult<()> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .namespaces
            .clear();
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
        assert!(limits.max_active_sessions > 0);
        assert!(limits.max_pending_admissions > 0);
        assert!(limits.max_coordinated_namespaces > 0);
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
            1,
        );
        let namespace = TrackNamespace::from_utf8_path("tenant/broadcast");
        let registration = coordinator
            .register_namespace(Some("/tenant/broadcast"), &namespace)
            .await
            .unwrap();
        assert_eq!(coordinator.len(), 1);
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
        assert_eq!(coordinator.len(), 0);
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
