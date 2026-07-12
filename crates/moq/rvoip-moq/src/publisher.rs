use std::net::SocketAddr;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::FutureExt;
use rvoip_core_traits::broadcast::{
    BroadcastDescriptor, BroadcastDrainDescriptor, BroadcastDrainRequest, BroadcastDrainState,
    BroadcastEndpoint, BroadcastHealthDescriptor, BroadcastHealthIssue, BroadcastHealthStatus,
    BroadcastLifecycleDescriptor, BroadcastLifecycleState, BroadcastProtocolDescriptor,
    BroadcastProtocolFamily, BroadcastPublisher, BroadcastRelayHop, BroadcastRelayRole,
    BroadcastResource, BroadcastSubstrate, BroadcastTransport,
};
use rvoip_core_traits::capability::CodecInfo;
use rvoip_core_traits::error::Result as RvoipResult;
use rvoip_core_traits::stream::MediaFrame;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::wire::{
    self, WirePublication, WirePublicationHandle, WireRelayClient, WireRelayPublication,
    WireRelayTermination, WireTlsMode,
};
use crate::{
    InMemoryMoqGroupIdAllocator, LocError, LocOpusPacketizer, MoqCompatibility, MoqError,
    MoqGroupIdAllocator, MoqNamespace, MoqProtocolVersion, MoqRelayFailure, MsfCatalog,
    AUDIO_TRACK, CATALOG_TRACK, LOC_DRAFT, MOQT_DRAFT, MSF_DRAFT,
};

#[derive(Clone, Debug)]
pub struct MoqPublisherConfig {
    pub tenant_id: String,
    pub broadcast_id: String,
    pub bitrate: u32,
    pub language: Option<String>,
    pub queue_frames: usize,
}

/// Substrate selection applied independently from the canonical `moqt://`
/// relay target.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum MoqRelaySubstratePolicy {
    /// Offer both supported substrates and retain what the peer negotiates.
    Auto,
    /// Require native MOQT over QUIC.
    #[default]
    RawQuic,
    /// Require MOQT over WebTransport.
    WebTransport,
}

/// Bounded TLS identity diagnostics for an external MOQT relay.
///
/// Certificate bodies and subject names are deliberately omitted. The full
/// leaf fingerprint is retained so an application can bind admission policy
/// to a reviewed relay identity without depending on moq-rs types.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
#[non_exhaustive]
pub enum MoqRelayPeerIdentity {
    /// No peer certificate was available.
    Anonymous,
    /// A certificate was presented while verification was explicitly disabled.
    UnverifiedCertificate {
        leaf_sha256: String,
        chain_len: u8,
        total_der_bytes: u32,
    },
    /// The peer certificate chain was verified by TLS.
    VerifiedCertificate {
        leaf_sha256: String,
        chain_len: u8,
        total_der_bytes: u32,
    },
}

impl MoqRelayPeerIdentity {
    /// Whether TLS authenticated this relay certificate chain.
    pub const fn is_authenticated(&self) -> bool {
        matches!(self, Self::VerifiedCertificate { .. })
    }
}

impl MoqPublisherConfig {
    /// Exact namespace representation retained for source compatibility.
    /// Validation is performed by [`Self::try_namespace`] and publisher
    /// construction; no sanitization or normalization occurs.
    pub fn namespace(&self) -> String {
        format!("{}/{}", self.tenant_id, self.broadcast_id)
    }

    pub fn try_namespace(&self) -> Result<MoqNamespace, MoqError> {
        Ok(MoqNamespace::new(
            self.tenant_id.clone(),
            self.broadcast_id.clone(),
        )?)
    }
}

/// MediaGraph-compatible MOQT publisher with an rvoip-owned public surface.
///
/// Dropping the final publisher handle transfers owned tasks to bounded
/// cleanup reapers. Prefer [`BroadcastPublisher::drain`] or
/// [`BroadcastPublisher::close`] when shutdown completion must be observed.
pub struct MoqBroadcastPublisher {
    config: MoqPublisherConfig,
    namespace: MoqNamespace,
    frame_tx: mpsc::Sender<MediaFrame>,
    wire: Arc<WirePublication>,
    frame_graceful_stop: CancellationToken,
    frame_abort: CancellationToken,
    frame_cleanup: SharedTaskCleanup,
    _group_id_allocator: Arc<dyn MoqGroupIdAllocator>,
    management: Arc<PublisherManagement>,
    runtime: tokio::runtime::Handle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FramePumpExit {
    Stopped,
    Aborted,
}

impl MoqBroadcastPublisher {
    pub fn new(config: MoqPublisherConfig) -> Result<Arc<Self>, MoqError> {
        static DEFAULT_GROUP_IDS: OnceLock<Arc<InMemoryMoqGroupIdAllocator>> = OnceLock::new();
        let allocator = Arc::clone(
            DEFAULT_GROUP_IDS.get_or_init(|| Arc::new(InMemoryMoqGroupIdAllocator::new())),
        );
        let allocator: Arc<dyn MoqGroupIdAllocator> = allocator;
        Self::new_with_group_id_allocator(config, allocator)
    }

    /// Construct a publisher with an application-owned Group ID allocator.
    ///
    /// Clustered and restartable origins must inject an allocator whose
    /// reservations are persisted atomically before they are returned.
    pub fn new_with_group_id_allocator(
        config: MoqPublisherConfig,
        group_id_allocator: Arc<dyn MoqGroupIdAllocator>,
    ) -> Result<Arc<Self>, MoqError> {
        let namespace = config.try_namespace()?;
        MoqCompatibility::PINNED.require(MoqProtocolVersion::PINNED)?;
        let catalog = MsfCatalog::opus_audio(
            &namespace,
            config.bitrate,
            config.language.clone(),
            unix_time_millis(),
        )?;
        let catalog_payload = catalog.to_json_bytes()?;
        let runtime =
            tokio::runtime::Handle::try_current().map_err(|_| MoqError::RuntimeUnavailable)?;
        let live_catalog_group =
            group_id_allocator.reserve_next_group(&namespace, CATALOG_TRACK)?;
        let (wire, mut catalog, audio) = WirePublication::new(&namespace)?;
        catalog.write_live(live_catalog_group, catalog_payload)?;
        let wire = Arc::new(wire);

        let (frame_tx, mut frame_rx) = mpsc::channel::<MediaFrame>(config.queue_frames.max(1));
        let frame_graceful_stop = CancellationToken::new();
        let frame_abort = CancellationToken::new();
        let frame_cleanup = SharedTaskCleanup::new(runtime.clone());
        let task_graceful_stop = frame_graceful_stop.clone();
        let task_abort = frame_abort.clone();
        let management = Arc::new(PublisherManagement::new());
        let task_management = Arc::clone(&management);
        let task_wire = Arc::clone(&wire);
        let task_allocator = Arc::clone(&group_id_allocator);
        let task_namespace = namespace.clone();
        let task = runtime.spawn(async move {
            let pump = AssertUnwindSafe(async move {
                let mut packetizer = LocOpusPacketizer::new();
                let mut audio = audio;
                let outcome: Result<FramePumpExit, MoqError> = loop {
                    let frame = tokio::select! {
                        biased;
                        () = task_abort.cancelled() => break Ok(FramePumpExit::Aborted),
                        () = task_graceful_stop.cancelled() => {
                            frame_rx.close();
                            loop {
                                let frame = tokio::select! {
                                    biased;
                                    () = task_abort.cancelled() => None,
                                    frame = frame_rx.recv() => frame,
                                };
                                let Some(frame) = frame else {
                                    break;
                                };
                                publish_audio_frame(
                                    &mut packetizer,
                                    &mut audio,
                                    task_allocator.as_ref(),
                                    &task_namespace,
                                    frame,
                                )?;
                            }
                            if task_abort.is_cancelled() {
                                break Ok(FramePumpExit::Aborted);
                            }
                            audio.finish()?;
                            let terminal = MsfCatalog::permanently_completed(unix_time_millis());
                            let terminal_group = task_allocator
                                .reserve_next_group(&task_namespace, CATALOG_TRACK)?;
                            catalog.write_terminal(terminal_group, terminal.to_json_bytes()?)?;
                            catalog.finish()?;
                            break Ok(FramePumpExit::Stopped);
                        },
                        frame = frame_rx.recv() => match frame {
                            Some(frame) => frame,
                            None => break Ok(FramePumpExit::Aborted),
                        },
                    };
                    publish_audio_frame(
                        &mut packetizer,
                        &mut audio,
                        task_allocator.as_ref(),
                        &task_namespace,
                        frame,
                    )?;
                };
                outcome
            })
            .catch_unwind()
            .await;
            if !matches!(
                pump,
                Ok(Ok(FramePumpExit::Stopped | FramePumpExit::Aborted))
            ) {
                fail_local_publication(task_management, task_wire).await;
            }
        });
        frame_cleanup.install(task);

        Ok(Arc::new(Self {
            config,
            namespace,
            frame_tx,
            wire,
            frame_graceful_stop,
            frame_abort,
            frame_cleanup,
            _group_id_allocator: group_id_allocator,
            management,
            runtime,
        }))
    }

    pub fn namespace(&self) -> &MoqNamespace {
        &self.namespace
    }

    pub fn config(&self) -> &MoqPublisherConfig {
        &self.config
    }

    pub const fn protocol_version(&self) -> MoqProtocolVersion {
        MoqProtocolVersion::PINNED
    }

    /// MOQT-specific aggregate health.
    pub fn moq_health(&self) -> MoqRelayHealthSnapshot {
        MoqRelayHealthSnapshot {
            common: self.management.health(),
            issues: Vec::new(),
        }
    }

    /// Announce this publisher to an external raw-QUIC or WebTransport MOQT
    /// relay. The handle closes both protocol tasks when dropped.
    pub async fn publish_to_relay(
        &self,
        client: &MoqRelayClient,
        relay: &Url,
    ) -> Result<MoqRelayPublication, MoqError> {
        let status = Arc::new(RelayStatus::new());
        let cancel = CancellationToken::new();
        let drain = CancellationToken::new();
        let control = Arc::new(RelayControl::new(
            Arc::clone(&status),
            cancel.clone(),
            drain.clone(),
            self.runtime.clone(),
        ));
        if let Err(error) = self.management.register(&control) {
            control.complete_without_task();
            return Err(error);
        }
        let publication = self.wire.tracks_handle();

        let mut connection = match connect_once(
            client.connector.as_ref(),
            &publication,
            relay,
            client.policy.attempt_timeout,
            client.policy.substrate,
            client.policy.publish_namespace_acceptance_timeout,
            &cancel,
        )
        .await
        {
            Ok(connection) => connection,
            Err(error) => {
                if matches!(error, MoqError::Closed) {
                    status.transition(BroadcastLifecycleState::Closed, None, None);
                    control.complete_without_task();
                    return Err(error);
                }
                let failure = relay_failure(&error);
                status.transition(BroadcastLifecycleState::Failed, Some(failure), None);
                metrics::counter!(
                    "rvoip_moq_relay_connect_attempts_total",
                    "result" => failure.metric_label()
                )
                .increment(1);
                control.complete_without_task();
                return Err(error);
            }
        };
        if cancel.is_cancelled() {
            connection.close().await;
            status.transition(BroadcastLifecycleState::Closed, None, None);
            control.complete_without_task();
            return Err(MoqError::Closed);
        }
        let diagnostics = RelayDiagnostics::from_connection(connection.as_ref());
        let connection_id = diagnostics.connection_id.clone();
        let relay_path = diagnostics.relay_path;
        status.transition(
            BroadcastLifecycleState::Ready,
            None,
            Some(diagnostics.clone()),
        );
        metrics::counter!(
            "rvoip_moq_relay_publications_total",
            "path" => relay_path
        )
        .increment(1);
        metrics::counter!(
            "rvoip_moq_protocol_acceptance_total",
            "result" => "accepted"
        )
        .increment(1);
        let supervisor_status = Arc::clone(&status);
        let connector = Arc::clone(&client.connector);
        let policy = client.policy.clone();
        let relay = relay.clone();
        let supervisor_cancel = cancel.clone();
        let supervisor_drain = drain.clone();
        let panic_status = Arc::clone(&supervisor_status);
        let task = tokio::spawn(async move {
            let outcome = AssertUnwindSafe(supervise_relay(
                connection,
                connector,
                publication,
                relay,
                policy,
                supervisor_cancel,
                supervisor_drain,
                supervisor_status,
            ))
            .catch_unwind()
            .await;
            if outcome.is_err() {
                panic_status.transition(
                    BroadcastLifecycleState::Failed,
                    Some(MoqRelayFailure::TaskFailed),
                    None,
                );
            }
        });
        control.install(task);
        let installed = control.status.snapshot();
        if installed.lifecycle == BroadcastLifecycleState::Failed {
            let failure = installed.failure.unwrap_or(MoqRelayFailure::TaskFailed);
            control.abort_and_reap().await;
            return Err(MoqError::RelayFailure(failure));
        }
        if cancel.is_cancelled() || installed.lifecycle == BroadcastLifecycleState::Closed {
            control.abort_and_reap().await;
            return Err(MoqError::Closed);
        }
        Ok(MoqRelayPublication {
            connection_id,
            relay_path,
            endpoint_uri: diagnostics.endpoint_uri,
            substrate: diagnostics.substrate,
            negotiated_protocol: diagnostics.negotiated_protocol,
            peer_identity: diagnostics.peer_identity,
            control,
        })
    }
}

impl Drop for MoqBroadcastPublisher {
    fn drop(&mut self) {
        self.management.begin_draining();
        // Drop is a hard stop: do not emit a permanently-completed catalog
        // because queued audio and relay delivery were not drained.
        self.frame_abort.cancel();
        self.wire.close();
        for control in self.management.active_relays() {
            control.start_cleanup_reaper();
        }
        spawn_frame_cleanup_reaper(self.frame_cleanup.clone());
        self.management.set_local(BroadcastLifecycleState::Closed);
    }
}

/// Bounded connection and reconnect behavior for one relay publication.
#[derive(Clone, Debug)]
pub struct MoqRelayConnectionPolicy {
    pub attempt_timeout: Duration,
    /// Maximum time to wait for the relay's explicit `REQUEST_OK` response to
    /// `PUBLISH_NAMESPACE` after transport and session setup complete.
    pub publish_namespace_acceptance_timeout: Duration,
    /// Substrate selection independent from the canonical relay target URI.
    pub substrate: MoqRelaySubstratePolicy,
    pub max_reconnect_attempts: u32,
    pub reconnect_initial_backoff: Duration,
    pub reconnect_max_backoff: Duration,
    pub reconnect_deadline: Duration,
    pub jitter_percent: u8,
}

impl Default for MoqRelayConnectionPolicy {
    fn default() -> Self {
        Self {
            attempt_timeout: Duration::from_secs(10),
            publish_namespace_acceptance_timeout: Duration::from_secs(5),
            substrate: MoqRelaySubstratePolicy::RawQuic,
            max_reconnect_attempts: 5,
            reconnect_initial_backoff: Duration::from_millis(100),
            reconnect_max_backoff: Duration::from_secs(5),
            reconnect_deadline: Duration::from_secs(30),
            jitter_percent: 20,
        }
    }
}

impl MoqRelayConnectionPolicy {
    fn validate(&self) -> Result<(), MoqError> {
        if self.attempt_timeout.is_zero() {
            return Err(MoqError::InvalidConfig(
                "relay attempt_timeout must be greater than zero",
            ));
        }
        if self.publish_namespace_acceptance_timeout.is_zero()
            || self.publish_namespace_acceptance_timeout >= self.attempt_timeout
        {
            return Err(MoqError::InvalidConfig(
                "relay publish_namespace_acceptance_timeout must be non-zero and shorter than attempt_timeout",
            ));
        }
        if self.max_reconnect_attempts == 0 || self.reconnect_deadline.is_zero() {
            return Err(MoqError::InvalidConfig(
                "relay reconnect attempts and deadline must be bounded and non-zero",
            ));
        }
        if self.reconnect_initial_backoff > self.reconnect_max_backoff {
            return Err(MoqError::InvalidConfig(
                "relay initial reconnect backoff cannot exceed its maximum",
            ));
        }
        if self.jitter_percent > 100 {
            return Err(MoqError::InvalidConfig(
                "relay reconnect jitter_percent cannot exceed 100",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct MoqRelayTlsConfig {
    /// PEM trust roots. An empty list uses verified system roots.
    pub root_certificates: Vec<PathBuf>,
    /// PEM client certificate chain. Required by production binding.
    pub client_certificate: Option<PathBuf>,
    /// PEM client private key. Required by production binding.
    pub client_private_key: Option<PathBuf>,
    /// Development-only escape hatch accepted solely by feature-gated
    /// development binding APIs.
    #[cfg(feature = "insecure-development")]
    pub disable_verification: bool,
}

impl std::fmt::Debug for MoqRelayTlsConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = formatter.debug_struct("MoqRelayTlsConfig");
        debug
            .field("root_certificate_count", &self.root_certificates.len())
            .field("has_client_certificate", &self.client_certificate.is_some())
            .field("has_client_private_key", &self.client_private_key.is_some());
        #[cfg(feature = "insecure-development")]
        debug.field("disable_verification", &self.disable_verification);
        debug.finish()
    }
}

/// Explicitly development-only alternatives to production mutual TLS.
#[cfg(feature = "insecure-development")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MoqRelayDevelopmentMode {
    /// Verify the relay certificate but do not present a client certificate.
    ServerAuthenticated,
    /// Disable relay certificate verification. Never use outside local tests.
    Insecure,
}

/// Bounded MOQT-specific health issues not representable by the common
/// transport health vocabulary.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MoqRelayHealthIssue {
    /// Compatibility value retained for readers of pre-acceptance diagnostics.
    /// Current publications never emit it because `REQUEST_OK` is observable.
    ProtocolAcceptanceUnobservable,
}

/// MOQT-specific health plus the common broadcast health snapshot.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct MoqRelayHealthSnapshot {
    pub common: BroadcastHealthDescriptor,
    pub issues: Vec<MoqRelayHealthIssue>,
}

/// Reusable MOQT relay client. Production binding requires origin-to-relay
/// mutual TLS and verified server roots.
#[derive(Clone)]
pub struct MoqRelayClient {
    connector: Arc<dyn RelayConnector>,
    policy: MoqRelayConnectionPolicy,
}

impl MoqRelayClient {
    pub fn bind(bind: SocketAddr, tls: MoqRelayTlsConfig) -> Result<Self, MoqError> {
        Self::bind_with_policy(bind, tls, MoqRelayConnectionPolicy::default())
    }

    pub fn bind_with_policy(
        bind: SocketAddr,
        tls: MoqRelayTlsConfig,
        policy: MoqRelayConnectionPolicy,
    ) -> Result<Self, MoqError> {
        Self::bind_mode(bind, tls, policy, WireTlsMode::ProductionMutualTls)
    }

    /// Bind a relay client with an explicitly development-only TLS posture.
    #[cfg(feature = "insecure-development")]
    pub fn bind_development(
        bind: SocketAddr,
        tls: MoqRelayTlsConfig,
        mode: MoqRelayDevelopmentMode,
    ) -> Result<Self, MoqError> {
        Self::bind_development_with_policy(bind, tls, mode, MoqRelayConnectionPolicy::default())
    }

    /// Development-only binding with an explicit connection policy.
    #[cfg(feature = "insecure-development")]
    pub fn bind_development_with_policy(
        bind: SocketAddr,
        tls: MoqRelayTlsConfig,
        mode: MoqRelayDevelopmentMode,
        policy: MoqRelayConnectionPolicy,
    ) -> Result<Self, MoqError> {
        let mode = match mode {
            MoqRelayDevelopmentMode::ServerAuthenticated => {
                WireTlsMode::DevelopmentServerAuthenticated
            }
            MoqRelayDevelopmentMode::Insecure => WireTlsMode::DevelopmentInsecure,
        };
        Self::bind_mode(bind, tls, policy, mode)
    }

    fn bind_mode(
        bind: SocketAddr,
        tls: MoqRelayTlsConfig,
        policy: MoqRelayConnectionPolicy,
        mode: WireTlsMode,
    ) -> Result<Self, MoqError> {
        policy.validate()?;
        #[cfg(feature = "insecure-development")]
        let disable_verification = tls.disable_verification;
        #[cfg(not(feature = "insecure-development"))]
        let disable_verification = false;
        Ok(Self {
            connector: Arc::new(WireRelayClient::bind(
                bind,
                tls.root_certificates,
                tls.client_certificate,
                tls.client_private_key,
                disable_verification,
                mode,
            )?),
            policy,
        })
    }
}

/// Running publication to one relay.
///
/// Dropping the handle starts a bounded runtime cleanup reaper. Call
/// [`Self::drain`] when the caller must observe completion or its deadline.
#[must_use = "retain the handle or call drain to observe relay cleanup"]
pub struct MoqRelayPublication {
    /// Connection ID whose `PUBLISH_NAMESPACE` received `REQUEST_OK`.
    pub connection_id: String,
    /// Compatibility substrate label for the initial accepted connection.
    pub relay_path: &'static str,
    /// Canonical network `moqt://` endpoint used by the initial connection.
    pub endpoint_uri: String,
    /// Substrate actually negotiated by the initial connection.
    pub substrate: BroadcastSubstrate,
    /// MOQT protocol identifier actually negotiated by the initial connection.
    pub negotiated_protocol: String,
    /// Bounded TLS identity for the initial accepted relay connection.
    pub peer_identity: MoqRelayPeerIdentity,
    control: Arc<RelayControl>,
}

impl MoqRelayPublication {
    pub fn lifecycle(&self) -> BroadcastLifecycleDescriptor {
        self.control.status.lifecycle()
    }

    pub fn health(&self) -> BroadcastHealthDescriptor {
        self.control.status.health()
    }

    /// MOQT-specific health.
    pub fn moq_health(&self) -> MoqRelayHealthSnapshot {
        MoqRelayHealthSnapshot {
            common: self.health(),
            issues: Vec::new(),
        }
    }

    pub fn last_failure(&self) -> Option<MoqRelayFailure> {
        self.control.status.snapshot().failure
    }

    /// Most recent protocol-accepted connection ID, including a reconnect.
    pub fn current_connection_id(&self) -> Option<String> {
        self.control
            .status
            .snapshot()
            .diagnostics
            .map(|diagnostics| diagnostics.connection_id)
    }

    /// Compatibility substrate label for the most recent accepted connection.
    pub fn current_relay_path(&self) -> Option<&'static str> {
        self.control
            .status
            .snapshot()
            .diagnostics
            .map(|diagnostics| diagnostics.relay_path)
    }

    /// Canonical network endpoint used by the most recent accepted connection.
    pub fn current_endpoint_uri(&self) -> Option<String> {
        self.control
            .status
            .snapshot()
            .diagnostics
            .map(|diagnostics| diagnostics.endpoint_uri)
    }

    /// Substrate used by the most recent accepted connection.
    pub fn current_substrate(&self) -> Option<BroadcastSubstrate> {
        self.control
            .status
            .snapshot()
            .diagnostics
            .map(|diagnostics| diagnostics.substrate)
    }

    /// MOQT protocol identifier negotiated by the most recent accepted connection.
    pub fn current_negotiated_protocol(&self) -> Option<String> {
        self.control
            .status
            .snapshot()
            .diagnostics
            .map(|diagnostics| diagnostics.negotiated_protocol)
    }

    /// TLS identity of the most recent accepted relay connection.
    pub fn current_peer_identity(&self) -> Option<MoqRelayPeerIdentity> {
        self.control
            .status
            .snapshot()
            .diagnostics
            .map(|diagnostics| diagnostics.peer_identity)
    }

    /// Wait for terminal closure and surface terminal relay failures.
    pub async fn wait(&self) -> Result<(), MoqError> {
        let snapshot = self.control.status.wait_terminal().await;
        if snapshot.lifecycle == BroadcastLifecycleState::Failed {
            return Err(MoqError::RelayFailure(
                snapshot.failure.unwrap_or(MoqRelayFailure::TaskFailed),
            ));
        }
        Ok(())
    }

    /// Gracefully close this relay publication by the supplied deadline.
    pub async fn drain(&self, deadline: DateTime<Utc>) -> bool {
        if !terminal_lifecycle(self.control.status.snapshot().lifecycle) {
            self.control
                .status
                .transition(BroadcastLifecycleState::Draining, None, None);
        }
        // A per-relay drain must traverse the same publication FIN and peer
        // acknowledgment barrier as publisher-wide drain. The hard-cancel
        // token is reserved for abort, failure, and dropped-handle cleanup.
        self.control.drain.cancel();
        match self.control.wait_until(deadline).await {
            TaskWaitOutcome::Completed => true,
            TaskWaitOutcome::Failed | TaskWaitOutcome::TimedOut => {
                self.control.abort_and_reap().await;
                false
            }
        }
    }
}

impl Drop for MoqRelayPublication {
    fn drop(&mut self) {
        self.control.start_cleanup_reaper();
    }
}

#[async_trait]
trait RelayConnection: Send {
    fn connection_id(&self) -> &str;
    fn relay_path(&self) -> &'static str;
    fn endpoint_uri(&self) -> &str;
    fn substrate(&self) -> BroadcastSubstrate;
    fn negotiated_protocol(&self) -> &str;
    fn peer_identity(&self) -> MoqRelayPeerIdentity;
    async fn terminated(&mut self) -> WireRelayTermination;
    async fn graceful_finish(&mut self) -> WireRelayTermination;
    async fn close(&mut self);
}

#[async_trait]
impl RelayConnection for WireRelayPublication {
    fn connection_id(&self) -> &str {
        &self.connection_id
    }

    fn relay_path(&self) -> &'static str {
        self.relay_path
    }

    fn endpoint_uri(&self) -> &str {
        &self.endpoint_uri
    }

    fn substrate(&self) -> BroadcastSubstrate {
        self.substrate
    }

    fn negotiated_protocol(&self) -> &str {
        &self.negotiated_protocol
    }

    fn peer_identity(&self) -> MoqRelayPeerIdentity {
        self.peer_identity.clone()
    }

    async fn terminated(&mut self) -> WireRelayTermination {
        WireRelayPublication::terminated(self).await
    }

    async fn graceful_finish(&mut self) -> WireRelayTermination {
        WireRelayPublication::terminated(self).await
    }

    async fn close(&mut self) {
        WireRelayPublication::close(self).await;
    }
}

#[async_trait]
trait RelayConnector: Send + Sync {
    async fn connect(
        &self,
        publication: &WirePublicationHandle,
        relay: &Url,
        substrate: MoqRelaySubstratePolicy,
        acceptance_timeout: Duration,
    ) -> Result<Box<dyn RelayConnection>, MoqError>;
}

#[async_trait]
impl RelayConnector for WireRelayClient {
    async fn connect(
        &self,
        publication: &WirePublicationHandle,
        relay: &Url,
        substrate: MoqRelaySubstratePolicy,
        acceptance_timeout: Duration,
    ) -> Result<Box<dyn RelayConnection>, MoqError> {
        Ok(Box::new(
            wire::publish_to_relay(publication, self, relay, substrate, acceptance_timeout).await?,
        ))
    }
}

#[derive(Clone, Debug)]
struct RelayDiagnostics {
    connection_id: String,
    relay_path: &'static str,
    endpoint_uri: String,
    substrate: BroadcastSubstrate,
    negotiated_protocol: String,
    peer_identity: MoqRelayPeerIdentity,
}

impl RelayDiagnostics {
    fn from_connection(connection: &dyn RelayConnection) -> Self {
        Self {
            connection_id: connection.connection_id().to_owned(),
            relay_path: connection.relay_path(),
            endpoint_uri: connection.endpoint_uri().to_owned(),
            substrate: connection.substrate(),
            negotiated_protocol: connection.negotiated_protocol().to_owned(),
            peer_identity: connection.peer_identity(),
        }
    }
}

#[derive(Clone, Debug)]
struct RelaySnapshot {
    lifecycle: BroadcastLifecycleState,
    since: DateTime<Utc>,
    failure: Option<MoqRelayFailure>,
    diagnostics: Option<RelayDiagnostics>,
}

struct RelayStatus {
    snapshot: watch::Sender<RelaySnapshot>,
}

impl RelayStatus {
    fn new() -> Self {
        let (snapshot, _) = watch::channel(RelaySnapshot {
            lifecycle: BroadcastLifecycleState::Starting,
            since: Utc::now(),
            failure: None,
            diagnostics: None,
        });
        Self { snapshot }
    }

    fn snapshot(&self) -> RelaySnapshot {
        self.snapshot.borrow().clone()
    }

    fn lifecycle(&self) -> BroadcastLifecycleDescriptor {
        let snapshot = self.snapshot();
        BroadcastLifecycleDescriptor {
            state: snapshot.lifecycle,
            since: Some(snapshot.since),
        }
    }

    fn health(&self) -> BroadcastHealthDescriptor {
        health_for_lifecycle(self.snapshot().lifecycle)
    }

    fn transition(
        &self,
        lifecycle: BroadcastLifecycleState,
        failure: Option<MoqRelayFailure>,
        diagnostics: Option<RelayDiagnostics>,
    ) {
        let previous = self.snapshot();
        if terminal_lifecycle(previous.lifecycle)
            || (previous.lifecycle == BroadcastLifecycleState::Draining
                && !terminal_lifecycle(lifecycle))
        {
            return;
        }
        let lifecycle = if lifecycle == BroadcastLifecycleState::Ready && diagnostics.is_none() {
            tracing::error!("refusing to mark an MOQT relay ready without accepted diagnostics");
            BroadcastLifecycleState::Degraded
        } else {
            lifecycle
        };
        let protocol_ready = lifecycle == BroadcastLifecycleState::Ready && diagnostics.is_some();
        let snapshot = RelaySnapshot {
            lifecycle,
            since: Utc::now(),
            failure: if protocol_ready
                || matches!(
                    lifecycle,
                    BroadcastLifecycleState::Ready | BroadcastLifecycleState::Closed
                ) {
                None
            } else {
                failure.or(previous.failure)
            },
            diagnostics: diagnostics.or(previous.diagnostics),
        };
        self.snapshot.send_replace(snapshot);
        metrics::counter!(
            "rvoip_moq_relay_lifecycle_transitions_total",
            "state" => lifecycle_label(lifecycle)
        )
        .increment(1);
    }

    async fn wait_terminal(&self) -> RelaySnapshot {
        let mut receiver = self.snapshot.subscribe();
        loop {
            let snapshot = receiver.borrow_and_update().clone();
            if terminal_lifecycle(snapshot.lifecycle) {
                return snapshot;
            }
            if receiver.changed().await.is_err() {
                return self.snapshot();
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TaskCleanupResult {
    Completed,
    TaskFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TaskWaitOutcome {
    Completed,
    Failed,
    TimedOut,
}

#[derive(Clone)]
struct SharedTaskCleanup {
    inner: Arc<SharedTaskCleanupInner>,
}

struct SharedTaskCleanupInner {
    state: Mutex<SharedTaskCleanupState>,
    completed: watch::Sender<Option<TaskCleanupResult>>,
    runtime: tokio::runtime::Handle,
    starts: AtomicU64,
}

struct SharedTaskCleanupState {
    installed: bool,
    requested: bool,
    started: bool,
    abort_requested: bool,
    task: Option<JoinHandle<()>>,
    abort: Option<tokio::task::AbortHandle>,
    result: Option<TaskCleanupResult>,
}

impl SharedTaskCleanup {
    fn new(runtime: tokio::runtime::Handle) -> Self {
        let (completed, _) = watch::channel(None);
        Self {
            inner: Arc::new(SharedTaskCleanupInner {
                state: Mutex::new(SharedTaskCleanupState {
                    installed: false,
                    requested: false,
                    started: false,
                    abort_requested: false,
                    task: None,
                    abort: None,
                    result: None,
                }),
                completed,
                runtime,
                starts: AtomicU64::new(0),
            }),
        }
    }

    fn install(&self, task: JoinHandle<()>) {
        let abort = task.abort_handle();
        let abort_now = {
            let mut state = self.inner.state.lock().expect("MOQT cleanup lock poisoned");
            assert!(!state.installed, "MOQT cleanup task already installed");
            assert!(state.result.is_none(), "MOQT cleanup already completed");
            state.installed = true;
            state.abort = Some(abort.clone());
            state.task = Some(task);
            state.abort_requested
        };
        if abort_now {
            abort.abort();
        }
        self.maybe_start();
    }

    fn complete_without_task(&self) {
        {
            let mut state = self.inner.state.lock().expect("MOQT cleanup lock poisoned");
            if state.result.is_some() {
                return;
            }
            state.installed = true;
            state.result = Some(TaskCleanupResult::Completed);
        }
        self.inner
            .completed
            .send_replace(Some(TaskCleanupResult::Completed));
    }

    /// Complete cleanup when the owner is being destroyed before a task was
    /// installed. Final owner destruction proves that no later installation is
    /// possible, so leaving the cleanup pending would strand its reaper.
    fn complete_if_uninstalled(&self) {
        let completed = {
            let mut state = self.inner.state.lock().expect("MOQT cleanup lock poisoned");
            if state.installed || state.result.is_some() {
                false
            } else {
                state.installed = true;
                state.result = Some(TaskCleanupResult::Completed);
                true
            }
        };
        if completed {
            self.inner
                .completed
                .send_replace(Some(TaskCleanupResult::Completed));
        }
    }

    fn request(&self, abort: bool) {
        let abort_handle = {
            let mut state = self.inner.state.lock().expect("MOQT cleanup lock poisoned");
            state.requested = true;
            if abort {
                state.abort_requested = true;
                state.abort.clone()
            } else {
                None
            }
        };
        if let Some(abort_handle) = abort_handle {
            abort_handle.abort();
        }
        self.maybe_start();
    }

    fn maybe_start(&self) {
        let task = {
            let mut state = self.inner.state.lock().expect("MOQT cleanup lock poisoned");
            if !state.requested || !state.installed || state.started || state.result.is_some() {
                return;
            }
            state.started = true;
            state.task.take()
        };
        let Some(task) = task else {
            self.inner.complete(TaskCleanupResult::Completed);
            return;
        };
        self.inner.starts.fetch_add(1, Ordering::AcqRel);
        let inner = Arc::clone(&self.inner);
        let _cleanup = self.inner.runtime.spawn(async move {
            let result = if task.await.is_ok() {
                TaskCleanupResult::Completed
            } else {
                TaskCleanupResult::TaskFailed
            };
            inner.complete(result);
        });
    }

    async fn finish(&self, abort: bool) -> TaskCleanupResult {
        self.request(abort);
        self.wait().await
    }

    async fn wait(&self) -> TaskCleanupResult {
        let mut completed = self.inner.completed.subscribe();
        loop {
            if let Some(result) = *completed.borrow_and_update() {
                return result;
            }
            if completed.changed().await.is_err() {
                return TaskCleanupResult::TaskFailed;
            }
        }
    }

    fn runtime(&self) -> &tokio::runtime::Handle {
        &self.inner.runtime
    }

    #[cfg(test)]
    fn start_count(&self) -> u64 {
        self.inner.starts.load(Ordering::Acquire)
    }
}

impl SharedTaskCleanupInner {
    fn complete(&self, result: TaskCleanupResult) {
        {
            let mut state = self.state.lock().expect("MOQT cleanup lock poisoned");
            if state.result.is_some() {
                return;
            }
            state.result = Some(result);
            state.abort = None;
        }
        self.completed.send_replace(Some(result));
    }
}

struct RelayControl {
    status: Arc<RelayStatus>,
    cancel: CancellationToken,
    drain: CancellationToken,
    cleanup: SharedTaskCleanup,
    drop_reaper_started: AtomicBool,
}

impl RelayControl {
    fn new(
        status: Arc<RelayStatus>,
        cancel: CancellationToken,
        drain: CancellationToken,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        Self {
            status,
            cancel,
            drain,
            cleanup: SharedTaskCleanup::new(runtime),
            drop_reaper_started: AtomicBool::new(false),
        }
    }

    fn install(&self, task: JoinHandle<()>) {
        self.cleanup.install(task);
    }

    fn complete_without_task(&self) {
        self.cleanup.complete_without_task();
    }

    async fn wait_until(&self, deadline: DateTime<Utc>) -> TaskWaitOutcome {
        let Some(remaining) = remaining_until(deadline) else {
            return TaskWaitOutcome::TimedOut;
        };
        self.cleanup.request(false);
        match tokio::time::timeout(remaining, self.cleanup.wait()).await {
            Ok(TaskCleanupResult::Completed) => match self.status.snapshot().lifecycle {
                BroadcastLifecycleState::Closed if !self.cancel.is_cancelled() => {
                    TaskWaitOutcome::Completed
                }
                BroadcastLifecycleState::Closed => TaskWaitOutcome::Failed,
                BroadcastLifecycleState::Failed => TaskWaitOutcome::Failed,
                _ => {
                    self.status.transition(
                        BroadcastLifecycleState::Failed,
                        Some(MoqRelayFailure::TaskFailed),
                        None,
                    );
                    TaskWaitOutcome::Failed
                }
            },
            Ok(TaskCleanupResult::TaskFailed) => {
                self.status.transition(
                    BroadcastLifecycleState::Failed,
                    Some(MoqRelayFailure::TaskFailed),
                    None,
                );
                TaskWaitOutcome::Failed
            }
            Err(_) => TaskWaitOutcome::TimedOut,
        }
    }

    async fn abort_and_reap(&self) {
        if !terminal_lifecycle(self.status.snapshot().lifecycle) {
            self.status.transition(
                BroadcastLifecycleState::Failed,
                Some(MoqRelayFailure::TaskFailed),
                None,
            );
        }
        self.cancel.cancel();
        let _ = self.cleanup.finish(true).await;
    }

    fn start_cleanup_reaper(&self) {
        if self.drop_reaper_started.swap(true, Ordering::AcqRel) {
            return;
        }
        self.cancel.cancel();
        spawn_relay_cleanup_reaper(self.cleanup.clone(), Arc::clone(&self.status));
    }
}

impl Drop for RelayControl {
    fn drop(&mut self) {
        // A publish future can be cancelled while its connector is pending,
        // before the supervisor task is installed. This is the final control
        // owner, so explicitly close that no-task lifecycle before starting the
        // cleanup reaper.
        self.cleanup.complete_if_uninstalled();
        self.start_cleanup_reaper();
    }
}

struct PublisherManagement {
    admitting: AtomicBool,
    local: Mutex<BroadcastLifecycleDescriptor>,
    relays: Mutex<Vec<Weak<RelayControl>>>,
}

impl PublisherManagement {
    fn new() -> Self {
        Self {
            admitting: AtomicBool::new(true),
            local: Mutex::new(BroadcastLifecycleDescriptor {
                state: BroadcastLifecycleState::Ready,
                since: Some(Utc::now()),
            }),
            relays: Mutex::new(Vec::new()),
        }
    }

    fn set_local(&self, state: BroadcastLifecycleState) {
        let mut local = self.local.lock().expect("MOQT lifecycle lock poisoned");
        if terminal_lifecycle(local.state) {
            return;
        }
        *local = BroadcastLifecycleDescriptor {
            state,
            since: Some(Utc::now()),
        };
        metrics::counter!(
            "rvoip_moq_publisher_lifecycle_transitions_total",
            "state" => lifecycle_label(state)
        )
        .increment(1);
    }

    fn begin_draining(&self) {
        let relays = self.relays.lock().expect("MOQT relay registry poisoned");
        self.admitting.store(false, Ordering::Release);
        let relay_failed = relays
            .iter()
            .filter_map(Weak::upgrade)
            .any(|relay| relay.status.snapshot().lifecycle == BroadcastLifecycleState::Failed);
        drop(relays);
        self.set_local(if relay_failed {
            BroadcastLifecycleState::Failed
        } else {
            BroadcastLifecycleState::Draining
        });
    }

    fn register(&self, control: &Arc<RelayControl>) -> Result<(), MoqError> {
        let mut relays = self.relays.lock().expect("MOQT relay registry poisoned");
        if !self.admitting.load(Ordering::Acquire) {
            return Err(MoqError::Closed);
        }
        relays.retain(|relay| relay.strong_count() > 0);
        relays.push(Arc::downgrade(control));
        Ok(())
    }

    fn active_relays(&self) -> Vec<Arc<RelayControl>> {
        let mut relays = self.relays.lock().expect("MOQT relay registry poisoned");
        let active = relays.iter().filter_map(Weak::upgrade).collect::<Vec<_>>();
        relays.retain(|relay| relay.strong_count() > 0);
        active
    }

    fn lifecycle(&self) -> BroadcastLifecycleDescriptor {
        let local = self
            .local
            .lock()
            .expect("MOQT lifecycle lock poisoned")
            .clone();
        if local.state != BroadcastLifecycleState::Ready {
            return local;
        }
        let snapshots = self
            .active_relays()
            .into_iter()
            .map(|relay| relay.status.snapshot())
            .filter(|snapshot| snapshot.lifecycle != BroadcastLifecycleState::Closed)
            .collect::<Vec<_>>();
        if snapshots.is_empty() {
            return local;
        }
        aggregate_relay_lifecycle(&snapshots, local.since)
    }

    fn health(&self) -> BroadcastHealthDescriptor {
        health_for_lifecycle(self.lifecycle().state)
    }

    fn accepted_relay_diagnostics(&self) -> Option<RelayDiagnostics> {
        self.active_relays()
            .into_iter()
            .map(|relay| relay.status.snapshot())
            .find(|snapshot| snapshot.lifecycle == BroadcastLifecycleState::Ready)
            .and_then(|snapshot| snapshot.diagnostics)
    }
}

fn aggregate_relay_lifecycle(
    snapshots: &[RelaySnapshot],
    fallback_since: Option<DateTime<Utc>>,
) -> BroadcastLifecycleDescriptor {
    let state_since = |state| {
        snapshots
            .iter()
            .filter(move |snapshot| snapshot.lifecycle == state)
            .map(|snapshot| snapshot.since)
    };
    if let Some(since) = state_since(BroadcastLifecycleState::Reconnecting).min() {
        return BroadcastLifecycleDescriptor {
            state: BroadcastLifecycleState::Reconnecting,
            since: Some(since),
        };
    }
    if let Some(since) = state_since(BroadcastLifecycleState::Starting).min() {
        return BroadcastLifecycleDescriptor {
            state: BroadcastLifecycleState::Starting,
            since: Some(since),
        };
    }
    if let Some(since) = state_since(BroadcastLifecycleState::Draining).min() {
        return BroadcastLifecycleDescriptor {
            state: BroadcastLifecycleState::Draining,
            since: Some(since),
        };
    }

    let failed = state_since(BroadcastLifecycleState::Failed).collect::<Vec<_>>();
    let live = snapshots
        .iter()
        .filter(|snapshot| {
            matches!(
                snapshot.lifecycle,
                BroadcastLifecycleState::Ready | BroadcastLifecycleState::Degraded
            )
        })
        .map(|snapshot| snapshot.since)
        .collect::<Vec<_>>();
    if !failed.is_empty() && live.is_empty() {
        return BroadcastLifecycleDescriptor {
            state: BroadcastLifecycleState::Failed,
            // The aggregate became failed when the last relay failed.
            since: failed.into_iter().max().or(fallback_since),
        };
    }
    if !failed.is_empty() && !live.is_empty() {
        let first_failure = failed.into_iter().min();
        let first_live = live.into_iter().min();
        return BroadcastLifecycleDescriptor {
            state: BroadcastLifecycleState::Degraded,
            // Mixed health began only once both sides of the mix existed.
            since: first_failure
                .zip(first_live)
                .map(|(failure, live)| failure.max(live))
                .or(fallback_since),
        };
    }
    if let Some(since) = state_since(BroadcastLifecycleState::Degraded).min() {
        return BroadcastLifecycleDescriptor {
            state: BroadcastLifecycleState::Degraded,
            since: Some(since),
        };
    }
    BroadcastLifecycleDescriptor {
        state: BroadcastLifecycleState::Ready,
        // All relays became ready when the last one entered Ready.
        since: state_since(BroadcastLifecycleState::Ready)
            .max()
            .or(fallback_since),
    }
}

async fn connect_once(
    connector: &dyn RelayConnector,
    publication: &WirePublicationHandle,
    relay: &Url,
    attempt_timeout: Duration,
    substrate: MoqRelaySubstratePolicy,
    acceptance_timeout: Duration,
    cancel: &CancellationToken,
) -> Result<Box<dyn RelayConnection>, MoqError> {
    tokio::select! {
        () = cancel.cancelled() => Err(MoqError::Closed),
        result = tokio::time::timeout(
            attempt_timeout,
            connector.connect(publication, relay, substrate, acceptance_timeout),
        ) => match result {
            Ok(result) => result,
            Err(_) => Err(MoqError::RelayFailure(MoqRelayFailure::ConnectTimeout)),
        },
    }
}

async fn supervise_relay(
    mut connection: Box<dyn RelayConnection>,
    connector: Arc<dyn RelayConnector>,
    publication: WirePublicationHandle,
    relay: Url,
    policy: MoqRelayConnectionPolicy,
    cancel: CancellationToken,
    drain: CancellationToken,
    status: Arc<RelayStatus>,
) {
    loop {
        let termination = tokio::select! {
            () = cancel.cancelled() => {
                connection.close().await;
                status.transition(BroadcastLifecycleState::Closed, None, None);
                return;
            }
            termination = connection.terminated() => termination,
            () = drain.cancelled() => connection.graceful_finish().await,
        };
        let failure = match termination {
            WireRelayTermination::Completed => {
                status.transition(BroadcastLifecycleState::Closed, None, None);
                return;
            }
            WireRelayTermination::Failed(failure) => failure,
        };
        if status.snapshot().lifecycle == BroadcastLifecycleState::Draining {
            status.transition(BroadcastLifecycleState::Failed, Some(failure), None);
            return;
        }
        metrics::counter!(
            "rvoip_moq_relay_failures_total",
            "reason" => failure.metric_label()
        )
        .increment(1);
        status.transition(BroadcastLifecycleState::Reconnecting, Some(failure), None);

        let Some(reconnect_deadline) = Instant::now().checked_add(policy.reconnect_deadline) else {
            status.transition(
                BroadcastLifecycleState::Failed,
                Some(MoqRelayFailure::ReconnectExhausted),
                None,
            );
            return;
        };
        let mut reconnected = None;
        for attempt in 1..=policy.max_reconnect_attempts {
            let delay = reconnect_delay(&policy, attempt, jitter_sample());
            let remaining = reconnect_deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() || delay >= remaining {
                break;
            }
            tokio::select! {
                () = cancel.cancelled() => {
                    status.transition(BroadcastLifecycleState::Closed, None, None);
                    return;
                }
                () = tokio::time::sleep(delay) => {}
            }
            let remaining = reconnect_deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let attempt_timeout = policy.attempt_timeout.min(remaining);
            if attempt_timeout <= policy.publish_namespace_acceptance_timeout {
                break;
            }
            match connect_once(
                connector.as_ref(),
                &publication,
                &relay,
                attempt_timeout,
                policy.substrate,
                policy.publish_namespace_acceptance_timeout,
                &cancel,
            )
            .await
            {
                Ok(next) => {
                    metrics::counter!(
                        "rvoip_moq_reconnect_attempts_total",
                        "result" => "accepted"
                    )
                    .increment(1);
                    metrics::counter!(
                        "rvoip_moq_protocol_acceptance_total",
                        "result" => "accepted"
                    )
                    .increment(1);
                    reconnected = Some(next);
                    break;
                }
                Err(MoqError::Closed) => {
                    status.transition(BroadcastLifecycleState::Closed, None, None);
                    return;
                }
                Err(error) => {
                    metrics::counter!(
                        "rvoip_moq_reconnect_attempts_total",
                        "result" => relay_failure(&error).metric_label()
                    )
                    .increment(1);
                }
            }
        }

        let Some(next) = reconnected else {
            status.transition(
                BroadcastLifecycleState::Failed,
                Some(MoqRelayFailure::ReconnectExhausted),
                None,
            );
            return;
        };
        connection = next;
        status.transition(
            BroadcastLifecycleState::Ready,
            None,
            Some(RelayDiagnostics::from_connection(connection.as_ref())),
        );
    }
}

fn reconnect_delay(policy: &MoqRelayConnectionPolicy, attempt: u32, sample: u64) -> Duration {
    let exponent = attempt.saturating_sub(1).min(31);
    let base = policy
        .reconnect_initial_backoff
        .saturating_mul(1_u32 << exponent)
        .min(policy.reconnect_max_backoff);
    if policy.jitter_percent == 0 || base.is_zero() {
        return base;
    }
    let base_nanos = base.as_nanos();
    let spread = base_nanos.saturating_mul(u128::from(policy.jitter_percent)) / 100;
    let low = base_nanos.saturating_sub(spread);
    let width = spread.saturating_mul(2).saturating_add(1);
    let offset = u128::from(sample) % width;
    duration_from_nanos(low.saturating_add(offset))
}

fn duration_from_nanos(nanos: u128) -> Duration {
    let seconds = (nanos / 1_000_000_000).min(u128::from(u64::MAX)) as u64;
    let subsec = (nanos % 1_000_000_000) as u32;
    Duration::new(seconds, subsec)
}

fn jitter_sample() -> u64 {
    static SAMPLE: AtomicU64 = AtomicU64::new(0);
    let counter = SAMPLE.fetch_add(0x9e37_79b9_7f4a_7c15, Ordering::Relaxed);
    let clock = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let mut value = counter ^ clock ^ u64::from(std::process::id());
    value ^= value >> 12;
    value ^= value << 25;
    value ^ (value >> 27)
}

fn relay_failure(error: &MoqError) -> MoqRelayFailure {
    match error {
        MoqError::RelayFailure(failure) => *failure,
        MoqError::PublishNamespaceRejected { .. } => MoqRelayFailure::PublishNamespaceRejected,
        MoqError::PublishNamespaceAcceptanceTimedOut { .. } => {
            MoqRelayFailure::PublishNamespaceAcceptanceTimedOut
        }
        MoqError::PublishNamespaceResponseStreamClosed => {
            MoqRelayFailure::PublishNamespaceResponseStreamClosed
        }
        MoqError::NegotiatedProtocolMismatch { .. } => MoqRelayFailure::NegotiatedProtocolMismatch,
        MoqError::RelayPeerUnauthenticated => MoqRelayFailure::RelayPeerUnauthenticated,
        _ => MoqRelayFailure::ConnectFailed,
    }
}

async fn fail_local_publication(management: Arc<PublisherManagement>, wire: Arc<WirePublication>) {
    management.set_local(BroadcastLifecycleState::Failed);
    let controls = management.active_relays();
    for control in &controls {
        control.cancel.cancel();
    }
    wire.close();
    let deadline = Utc::now() + chrono::Duration::seconds(5);
    for control in controls {
        if control.wait_until(deadline).await != TaskWaitOutcome::Completed {
            control.abort_and_reap().await;
        }
    }
}

fn remaining_until(deadline: DateTime<Utc>) -> Option<Duration> {
    (deadline - Utc::now())
        .to_std()
        .ok()
        .filter(|duration| !duration.is_zero())
}

async fn finish_shared_cleanup_until(
    cleanup: &SharedTaskCleanup,
    deadline: DateTime<Utc>,
) -> TaskWaitOutcome {
    cleanup.request(false);
    let Some(remaining) = remaining_until(deadline) else {
        let _ = cleanup.finish(true).await;
        return TaskWaitOutcome::TimedOut;
    };
    match tokio::time::timeout(remaining, cleanup.wait()).await {
        Ok(TaskCleanupResult::Completed) => TaskWaitOutcome::Completed,
        Ok(TaskCleanupResult::TaskFailed) => TaskWaitOutcome::Failed,
        Err(_) => {
            let _ = cleanup.finish(true).await;
            TaskWaitOutcome::TimedOut
        }
    }
}

fn spawn_relay_cleanup_reaper(cleanup: SharedTaskCleanup, status: Arc<RelayStatus>) {
    let runtime = cleanup.runtime().clone();
    let _cleanup = runtime.spawn(async move {
        cleanup.request(false);
        let outcome = tokio::time::timeout(Duration::from_secs(5), cleanup.wait()).await;
        match outcome {
            Ok(TaskCleanupResult::Completed) if terminal_lifecycle(status.snapshot().lifecycle) => {
            }
            Ok(_) => status.transition(
                BroadcastLifecycleState::Failed,
                Some(MoqRelayFailure::TaskFailed),
                None,
            ),
            Err(_) => {
                status.transition(
                    BroadcastLifecycleState::Failed,
                    Some(MoqRelayFailure::TaskFailed),
                    None,
                );
                let _ = cleanup.finish(true).await;
            }
        }
    });
}

fn spawn_frame_cleanup_reaper(cleanup: SharedTaskCleanup) {
    let runtime = cleanup.runtime().clone();
    let _cleanup = runtime.spawn(async move {
        cleanup.request(false);
        if tokio::time::timeout(Duration::from_secs(5), cleanup.wait())
            .await
            .is_err()
        {
            let _ = cleanup.finish(true).await;
        }
    });
}

fn terminal_lifecycle(state: BroadcastLifecycleState) -> bool {
    matches!(
        state,
        BroadcastLifecycleState::Closed | BroadcastLifecycleState::Failed
    )
}

fn lifecycle_label(state: BroadcastLifecycleState) -> &'static str {
    match state {
        BroadcastLifecycleState::Starting => "starting",
        BroadcastLifecycleState::Ready => "ready",
        BroadcastLifecycleState::Degraded => "degraded",
        BroadcastLifecycleState::Reconnecting => "reconnecting",
        BroadcastLifecycleState::Draining => "draining",
        BroadcastLifecycleState::Closed => "closed",
        BroadcastLifecycleState::Failed => "failed",
        _ => "unknown",
    }
}

fn health_for_lifecycle(state: BroadcastLifecycleState) -> BroadcastHealthDescriptor {
    let (status, issues) = match state {
        BroadcastLifecycleState::Ready => (BroadcastHealthStatus::Healthy, Vec::new()),
        BroadcastLifecycleState::Starting => (
            BroadcastHealthStatus::Degraded,
            vec![BroadcastHealthIssue::TransportUnavailable],
        ),
        BroadcastLifecycleState::Degraded => (
            BroadcastHealthStatus::Degraded,
            vec![BroadcastHealthIssue::RelayUnavailable],
        ),
        BroadcastLifecycleState::Reconnecting => (
            BroadcastHealthStatus::Degraded,
            vec![BroadcastHealthIssue::Reconnecting],
        ),
        BroadcastLifecycleState::Draining => (
            BroadcastHealthStatus::Degraded,
            vec![BroadcastHealthIssue::Draining],
        ),
        BroadcastLifecycleState::Closed => (BroadcastHealthStatus::Closed, Vec::new()),
        BroadcastLifecycleState::Failed => (
            BroadcastHealthStatus::Unhealthy,
            vec![BroadcastHealthIssue::RelayUnavailable],
        ),
        _ => (
            BroadcastHealthStatus::Unhealthy,
            vec![BroadcastHealthIssue::TransportUnavailable],
        ),
    };
    BroadcastHealthDescriptor {
        status,
        issues,
        active_subscribers: None,
        subscriber_capacity: None,
        checked_at: Utc::now(),
    }
}

#[async_trait]
impl BroadcastPublisher for MoqBroadcastPublisher {
    fn descriptor(&self) -> BroadcastDescriptor {
        BroadcastDescriptor {
            transport: BroadcastTransport::Moqt,
            namespace: self.namespace.to_string(),
            audio_track: AUDIO_TRACK.into(),
            catalog_track: Some(CATALOG_TRACK.into()),
            protocol_version: MoqProtocolVersion::PINNED.to_string(),
        }
    }

    fn codec(&self) -> CodecInfo {
        CodecInfo::from_name_with_defaults("opus")
    }

    fn frames_out(&self) -> mpsc::Sender<MediaFrame> {
        self.frame_tx.clone()
    }

    fn endpoint(&self) -> BroadcastEndpoint {
        let diagnostics = self.management.accepted_relay_diagnostics();
        let uri = diagnostics
            .as_ref()
            .map(|diagnostics| diagnostics.endpoint_uri.clone());
        let relay_path = diagnostics
            .map(|diagnostics| {
                vec![BroadcastRelayHop {
                    role: BroadcastRelayRole::Relay,
                    uri: diagnostics.endpoint_uri,
                }]
            })
            .unwrap_or_default();
        BroadcastEndpoint {
            uri,
            resource: BroadcastResource::Moqt {
                namespace: self.namespace.to_string(),
                audio_track: AUDIO_TRACK.into(),
                catalog_track: Some(CATALOG_TRACK.into()),
                events_track: None,
            },
            relay_path,
        }
    }

    fn protocol(&self) -> BroadcastProtocolDescriptor {
        let diagnostics = self.management.accepted_relay_diagnostics();
        BroadcastProtocolDescriptor {
            family: BroadcastProtocolFamily::Moqt,
            substrate: diagnostics.map(|diagnostics| diagnostics.substrate),
            transport_version: MOQT_DRAFT.into(),
            media_format_version: Some(MSF_DRAFT.into()),
            object_format_version: Some(LOC_DRAFT.into()),
            media_profile: Some("opus/48000/1; frame-duration=20ms".into()),
        }
    }

    fn lifecycle(&self) -> BroadcastLifecycleDescriptor {
        self.management.lifecycle()
    }

    fn health(&self) -> BroadcastHealthDescriptor {
        self.management.health()
    }

    async fn drain(
        self: Arc<Self>,
        request: BroadcastDrainRequest,
    ) -> RvoipResult<BroadcastDrainDescriptor> {
        let started_at = Utc::now();
        self.management.begin_draining();
        // Close media admission and finish the local track/cache before
        // disconnecting relays, so the terminal catalog can be observed.
        self.frame_graceful_stop.cancel();
        let mut deadline_exceeded = started_at > request.deadline;
        let mut failed = self.management.lifecycle().state == BroadcastLifecycleState::Failed;
        match finish_shared_cleanup_until(&self.frame_cleanup, request.deadline).await {
            TaskWaitOutcome::Completed => {}
            TaskWaitOutcome::Failed => {
                failed = true;
                self.management.set_local(BroadcastLifecycleState::Failed);
            }
            TaskWaitOutcome::TimedOut => {
                deadline_exceeded = true;
                self.frame_abort.cancel();
            }
        }
        failed |= self.management.lifecycle().state == BroadcastLifecycleState::Failed;
        // Ending the catalog track and then the namespace makes the complete
        // publication visible to relay serving tasks before they are drained.
        self.wire.close();

        let controls = self.management.active_relays();
        for control in &controls {
            if !terminal_lifecycle(control.status.snapshot().lifecycle) {
                control
                    .status
                    .transition(BroadcastLifecycleState::Draining, None, None);
                control.drain.cancel();
            }
        }

        for control in &controls {
            match control.wait_until(request.deadline).await {
                TaskWaitOutcome::Completed => {}
                TaskWaitOutcome::Failed => {
                    failed = true;
                    control.abort_and_reap().await;
                }
                TaskWaitOutcome::TimedOut => {
                    deadline_exceeded = true;
                    control.abort_and_reap().await;
                }
            }
        }
        self.management.set_local(BroadcastLifecycleState::Closed);
        failed |= self.management.lifecycle().state == BroadcastLifecycleState::Failed;
        let drain_incomplete = deadline_exceeded || failed;
        metrics::counter!(
            "rvoip_moq_drains_total",
            "result" => if failed { "failed" } else if deadline_exceeded { "deadline-exceeded" } else { "drained" }
        )
        .increment(1);
        Ok(BroadcastDrainDescriptor {
            state: if drain_incomplete {
                BroadcastDrainState::DeadlineExceeded
            } else {
                BroadcastDrainState::Drained
            },
            reason: request.reason,
            started_at,
            deadline: request.deadline,
            completed_at: Some(Utc::now()),
            remaining_subscribers: 0,
        })
    }

    async fn close(self: Arc<Self>) -> RvoipResult<()> {
        self.management.begin_draining();
        self.frame_graceful_stop.cancel();
        let cleanup_deadline = Utc::now() + chrono::Duration::seconds(5);
        let mut failed = self.management.lifecycle().state == BroadcastLifecycleState::Failed;
        match finish_shared_cleanup_until(&self.frame_cleanup, cleanup_deadline).await {
            TaskWaitOutcome::Completed => {}
            TaskWaitOutcome::Failed => {
                failed = true;
                self.management.set_local(BroadcastLifecycleState::Failed);
            }
            TaskWaitOutcome::TimedOut => {
                failed = true;
                self.frame_abort.cancel();
            }
        }
        failed |= self.management.lifecycle().state == BroadcastLifecycleState::Failed;
        self.wire.close();

        for control in self.management.active_relays() {
            if !terminal_lifecycle(control.status.snapshot().lifecycle) {
                control
                    .status
                    .transition(BroadcastLifecycleState::Draining, None, None);
                control.drain.cancel();
            }
            let cleanup_deadline = Utc::now() + chrono::Duration::seconds(5);
            match control.wait_until(cleanup_deadline).await {
                TaskWaitOutcome::Completed => {}
                TaskWaitOutcome::Failed | TaskWaitOutcome::TimedOut => {
                    failed = true;
                    control.abort_and_reap().await;
                }
            }
        }
        self.management.set_local(BroadcastLifecycleState::Closed);
        failed |= self.management.lifecycle().state == BroadcastLifecycleState::Failed;
        if failed {
            Err(MoqError::PublicationFailed.into())
        } else {
            Ok(())
        }
    }
}

fn publish_audio_frame(
    packetizer: &mut LocOpusPacketizer,
    audio: &mut wire::WireAudioWriter,
    group_id_allocator: &dyn MoqGroupIdAllocator,
    namespace: &MoqNamespace,
    frame: MediaFrame,
) -> Result<(), MoqError> {
    let mut packetized = match packetizer.packetize(&frame) {
        Ok(packetized) => packetized,
        Err(error) => {
            metrics::counter!(
                "rvoip_moq_invalid_frames_total",
                "reason" => loc_error_label(&error)
            )
            .increment(1);
            tracing::warn!(%error, "dropping frame outside the MOQT LOC profile");
            return Ok(());
        }
    };
    if let Some(discontinuity) = packetized.discontinuity {
        metrics::counter!("rvoip_moq_timestamp_discontinuities_total").increment(1);
        tracing::warn!(
            expected_rtp_timestamp = discontinuity.expected_rtp_timestamp,
            actual_rtp_timestamp = discontinuity.actual_rtp_timestamp,
            loc_timestamp = packetized.object.timestamp,
            "publishing the first valid frame after an RTP timestamp discontinuity"
        );
    }
    // Reserve before touching the wire. Even a failed write consumes the ID,
    // preventing reuse after restart.
    packetized.object.group_id = group_id_allocator.reserve_next_group(namespace, AUDIO_TRACK)?;
    audio.write(packetized.object)?;
    metrics::counter!("rvoip_moq_objects_total", "track" => "audio").increment(1);
    Ok(())
}

fn loc_error_label(error: &LocError) -> &'static str {
    match error {
        LocError::NotAudio => "not_audio",
        LocError::EmptyPacket => "empty_packet",
        LocError::StereoPacket => "stereo",
        LocError::MissingFrameCount | LocError::InvalidFrameCount { .. } => "frame_count",
        LocError::PacketDuration { .. } => "duration",
        LocError::TimestampOverflow => "timestamp_overflow",
        LocError::GroupIdExhausted => "group_id_exhausted",
    }
}

fn unix_time_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::future::pending;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex as StdMutex;

    use bytes::Bytes;
    use chrono::Utc;
    use rvoip_core_traits::broadcast::BroadcastPublisher;
    use rvoip_core_traits::ids::StreamId;
    use rvoip_core_traits::stream::StreamKind;
    use tokio::sync::{oneshot, Notify};

    #[cfg(feature = "insecure-development")]
    use moq_native_ietf::quic;
    #[cfg(feature = "insecure-development")]
    use moq_transport::coding::{TrackName, TrackNamespace, Value};
    #[cfg(feature = "insecure-development")]
    use moq_transport::serve::{
        ServeError, SubgroupsReader, Track, TrackReader, TrackReaderMode, Tracks,
    };
    #[cfg(feature = "insecure-development")]
    use moq_transport::session::{Publisher, SessionTarget, Subscribe, Subscriber, Transport};

    use super::*;

    fn config() -> MoqPublisherConfig {
        MoqPublisherConfig {
            tenant_id: "tenant-a".into(),
            broadcast_id: "broadcast-1".into(),
            bitrate: 24_000,
            language: Some("en".into()),
            queue_frames: 10,
        }
    }

    #[derive(Default)]
    struct RecordingGroupIdAllocator {
        inner: InMemoryMoqGroupIdAllocator,
        reservations: StdMutex<Vec<(String, u64)>>,
    }

    impl RecordingGroupIdAllocator {
        fn reservations_for(&self, track: &str) -> Vec<u64> {
            self.reservations
                .lock()
                .unwrap()
                .iter()
                .filter_map(|(reserved_track, group_id)| {
                    (reserved_track == track).then_some(*group_id)
                })
                .collect()
        }
    }

    impl MoqGroupIdAllocator for RecordingGroupIdAllocator {
        fn reserve_next_group(
            &self,
            namespace: &MoqNamespace,
            track: &str,
        ) -> Result<u64, crate::MoqGroupIdAllocationError> {
            let group_id = self.inner.reserve_next_group(namespace, track)?;
            self.reservations
                .lock()
                .unwrap()
                .push((track.to_owned(), group_id));
            Ok(group_id)
        }

        fn recover_above(
            &self,
            namespace: &MoqNamespace,
            track: &str,
            previous_group_id: u64,
        ) -> Result<(), crate::MoqGroupIdAllocationError> {
            self.inner
                .recover_above(namespace, track, previous_group_id)
        }
    }

    #[derive(Default)]
    struct TerminalCatalogFailureAllocator {
        inner: InMemoryMoqGroupIdAllocator,
        catalog_reservations: AtomicUsize,
    }

    impl MoqGroupIdAllocator for TerminalCatalogFailureAllocator {
        fn reserve_next_group(
            &self,
            namespace: &MoqNamespace,
            track: &str,
        ) -> Result<u64, crate::MoqGroupIdAllocationError> {
            if track == CATALOG_TRACK
                && self.catalog_reservations.fetch_add(1, Ordering::AcqRel) == 1
            {
                return Err(crate::MoqGroupIdAllocationError::Unavailable);
            }
            self.inner.reserve_next_group(namespace, track)
        }

        fn recover_above(
            &self,
            namespace: &MoqNamespace,
            track: &str,
            previous_group_id: u64,
        ) -> Result<(), crate::MoqGroupIdAllocationError> {
            self.inner
                .recover_above(namespace, track, previous_group_id)
        }
    }

    fn opus_frame(timestamp_rtp: u32) -> MediaFrame {
        MediaFrame {
            stream_id: StreamId::new(),
            kind: StreamKind::Audio,
            payload: Bytes::from_static(&[0x78, 0x00]),
            timestamp_rtp,
            captured_at: Utc::now(),
            payload_type: Some(111),
        }
    }

    #[cfg(feature = "insecure-development")]
    const MINI_RELAY_TIMEOUT: Duration = Duration::from_secs(10);

    #[cfg(feature = "insecure-development")]
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum MiniRelayStep {
        LiveCatalog,
        Audio,
        AudioEnded,
        TerminalCatalog,
        CatalogEnded,
        NamespaceCompleted,
    }

    #[cfg(feature = "insecure-development")]
    struct MiniRelayPki {
        directory: PathBuf,
        certificate: PathBuf,
        private_key: PathBuf,
    }

    #[cfg(feature = "insecure-development")]
    impl MiniRelayPki {
        fn new() -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let directory = std::env::temp_dir().join(format!(
                "rvoip-moq-mini-relay-{}-{nonce}",
                std::process::id()
            ));
            std::fs::create_dir_all(&directory).unwrap();
            let generated = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
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
    }

    #[cfg(feature = "insecure-development")]
    impl Drop for MiniRelayPki {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    #[cfg(feature = "insecure-development")]
    async fn subgroup_reader(track: &TrackReader) -> SubgroupsReader {
        match tokio::time::timeout(MINI_RELAY_TIMEOUT, track.mode())
            .await
            .expect("track mode timed out")
            .expect("track mode failed")
        {
            TrackReaderMode::Subgroups(reader) => reader,
            _ => panic!("MSF/LOC tracks must use subgroup streams"),
        }
    }

    #[cfg(feature = "insecure-development")]
    async fn read_catalog_group(
        groups: &mut SubgroupsReader,
        context: &'static str,
    ) -> (u64, Bytes) {
        let mut subgroup = tokio::time::timeout(MINI_RELAY_TIMEOUT, groups.next())
            .await
            .unwrap_or_else(|_| panic!("{context} subgroup timed out"))
            .unwrap_or_else(|error| panic!("{context} subgroup failed: {error}"))
            .unwrap_or_else(|| panic!("{context} ended before the expected object"));
        assert_eq!(subgroup.subgroup_id, 0);
        assert!(subgroup.first_object);
        assert!(subgroup.end_of_group);
        let mut object = subgroup
            .next()
            .await
            .expect("catalog object failed")
            .expect("catalog subgroup was empty");
        assert_eq!(object.object_id, 0);
        assert!(object.extension_headers.is_empty());
        let payload = object.read_all().await.expect("catalog payload failed");
        assert!(subgroup
            .next()
            .await
            .expect("catalog subgroup FIN failed")
            .is_none());
        (subgroup.group_id, payload)
    }

    #[cfg(feature = "insecure-development")]
    async fn read_audio_group(groups: &mut SubgroupsReader) -> u64 {
        let mut subgroup = tokio::time::timeout(MINI_RELAY_TIMEOUT, groups.next())
            .await
            .expect("audio subgroup timed out")
            .expect("audio subgroup failed")
            .expect("audio track ended before the expected object");
        assert_eq!(subgroup.subgroup_id, 0);
        assert!(subgroup.first_object);
        assert!(subgroup.end_of_group);
        let mut object = subgroup
            .next()
            .await
            .expect("audio object failed")
            .expect("audio subgroup was empty");
        assert_eq!(object.object_id, 0);
        assert_eq!(
            object
                .extension_headers
                .get(crate::LOC_TIMESTAMP_PROPERTY)
                .expect("LOC timestamp missing")
                .value,
            Value::IntValue(960)
        );
        assert_eq!(
            object
                .extension_headers
                .get(crate::LOC_TIMESCALE_PROPERTY)
                .expect("LOC timescale missing")
                .value,
            Value::IntValue(48_000)
        );
        assert_eq!(
            object.read_all().await.unwrap(),
            Bytes::from_static(&[0x78, 0x00])
        );
        assert!(subgroup
            .next()
            .await
            .expect("audio subgroup FIN failed")
            .is_none());
        subgroup.group_id
    }

    #[cfg(feature = "insecure-development")]
    async fn wait_for_clean_subscription_end(subscription: &Subscribe, context: &'static str) {
        let result = tokio::time::timeout(MINI_RELAY_TIMEOUT, subscription.closed())
            .await
            .unwrap_or_else(|_| panic!("{context} timed out"));
        assert!(
            matches!(result, Ok(()) | Err(ServeError::Done)),
            "{context} failed: {result:?}"
        );
    }

    #[cfg(feature = "insecure-development")]
    fn assert_live_catalog(payload: &[u8]) {
        let catalog: serde_json::Value = serde_json::from_slice(payload).unwrap();
        assert_eq!(catalog["version"], crate::MSF_CATALOG_VERSION);
        assert!(catalog.get("isComplete").is_none());
        assert_eq!(catalog["tracks"].as_array().unwrap().len(), 1);
        assert_eq!(catalog["tracks"][0]["name"], AUDIO_TRACK);
    }

    #[cfg(feature = "insecure-development")]
    fn assert_terminal_catalog(payload: &[u8]) {
        let catalog: serde_json::Value = serde_json::from_slice(payload).unwrap();
        assert_eq!(catalog["version"], crate::MSF_CATALOG_VERSION);
        assert_eq!(catalog["isComplete"], true);
        assert!(catalog["tracks"].as_array().unwrap().is_empty());
    }

    #[cfg(feature = "insecure-development")]
    async fn assert_two_hop_mini_relay(
        relay_substrate: MoqRelaySubstratePolicy,
        native_substrate: quic::SubstratePolicy,
        expected_transport: Transport,
    ) {
        let pki = MiniRelayPki::new();
        let tls = moq_native_ietf::tls::Args {
            cert: vec![pki.certificate.clone()],
            key: vec![pki.private_key.clone()],
            disable_verify: true,
            ..Default::default()
        }
        .load()
        .unwrap();
        let endpoint = quic::Endpoint::new(
            quic::Config::new("127.0.0.1:0".parse().unwrap(), None, tls)
                .unwrap()
                .with_stateless_retry(false),
        )
        .unwrap();
        let quic::Endpoint {
            client: downstream_client,
            server,
            ..
        } = endpoint;
        let mut server = server.expect("mini relay must expose a server");
        let server_address = server.local_addr().unwrap();
        let target: SessionTarget = format!("moqt://localhost:{}/relay", server_address.port())
            .parse()
            .unwrap();
        let relay_url = Url::parse(&target.to_string()).unwrap();
        let wire_namespace = TrackNamespace::from_utf8_path("tenant-a/broadcast-1");
        let relay_wire_namespace = wire_namespace.clone();
        let relay_terminal_observed = Arc::new(AtomicBool::new(false));
        let relay_terminal_observer = Arc::clone(&relay_terminal_observed);
        let (origin_cached, origin_cached_rx) = oneshot::channel();
        let (origin_namespace_fin, origin_namespace_fin_rx) = oneshot::channel();
        let (release_origin_ack, release_origin_ack_rx) = oneshot::channel();

        let relay = tokio::spawn(async move {
            let origin_connection =
                tokio::time::timeout(MINI_RELAY_TIMEOUT, server.accept_connection())
                    .await
                    .expect("origin connection timed out")
                    .expect("mini relay stopped accepting the origin");
            assert_eq!(origin_connection.negotiated.substrate, expected_transport);
            let (origin_session, mut origin_subscriber) =
                Subscriber::accept(origin_connection.session, origin_connection.negotiated)
                    .await
                    .unwrap();
            let origin_session = tokio::spawn(origin_session.run());
            let mut origin_namespace =
                tokio::time::timeout(MINI_RELAY_TIMEOUT, origin_subscriber.published_namespace())
                    .await
                    .expect("origin namespace timed out")
                    .expect("origin session ended before PUBLISH_NAMESPACE");
            assert_eq!(origin_namespace.namespace, relay_wire_namespace);
            origin_namespace.ok().unwrap();

            let (mut relay_tracks_writer, relay_tracks_request, mut relay_tracks_reader) =
                Tracks::new(origin_namespace.namespace.clone()).produce();
            let relay_catalog_writer = relay_tracks_writer
                .create(TrackName::from(CATALOG_TRACK))
                .expect("failed to create relay catalog cache");
            let relay_catalog_reader = relay_tracks_reader
                .get_track_reader(&origin_namespace.namespace, TrackName::from(CATALOG_TRACK))
                .expect("relay catalog cache missing");
            let catalog_subscription = origin_subscriber
                .subscribe_open(relay_catalog_writer)
                .await
                .expect("origin catalog SUBSCRIBE failed");
            let mut relay_catalog = subgroup_reader(&relay_catalog_reader).await;
            let (live_group, live_payload) =
                read_catalog_group(&mut relay_catalog, "relay live catalog").await;
            assert_live_catalog(&live_payload);
            let mut observations = vec![MiniRelayStep::LiveCatalog];

            let relay_audio_writer = relay_tracks_writer
                .create(TrackName::from(AUDIO_TRACK))
                .expect("failed to create relay audio cache");
            let relay_audio_reader = relay_tracks_reader
                .get_track_reader(&origin_namespace.namespace, TrackName::from(AUDIO_TRACK))
                .expect("relay audio cache missing");
            let audio_subscription = origin_subscriber
                .subscribe_open(relay_audio_writer)
                .await
                .expect("origin audio SUBSCRIBE failed");
            origin_cached
                .send(())
                .expect("downstream test stopped before relay cache became live");

            let downstream_connection =
                tokio::time::timeout(MINI_RELAY_TIMEOUT, server.accept_connection())
                    .await
                    .expect("downstream connection timed out")
                    .expect("mini relay stopped accepting the downstream client");
            assert_eq!(
                downstream_connection.negotiated.substrate,
                expected_transport
            );
            let (downstream_session, mut downstream_publisher) = Publisher::accept(
                downstream_connection.session,
                downstream_connection.negotiated,
            )
            .await
            .unwrap();
            let downstream_session = tokio::spawn(downstream_session.run());
            let downstream_publish = tokio::spawn(async move {
                downstream_publisher
                    .publish_namespace(relay_tracks_reader)
                    .await
            });

            let mut relay_audio = subgroup_reader(&relay_audio_reader).await;
            let _audio_group = read_audio_group(&mut relay_audio).await;
            observations.push(MiniRelayStep::Audio);
            wait_for_clean_subscription_end(&audio_subscription, "origin audio completion").await;
            observations.push(MiniRelayStep::AudioEnded);
            let (terminal_group, terminal_payload) =
                read_catalog_group(&mut relay_catalog, "relay terminal catalog").await;
            assert!(terminal_group > live_group);
            assert_terminal_catalog(&terminal_payload);
            relay_terminal_observer.store(true, Ordering::Release);
            observations.push(MiniRelayStep::TerminalCatalog);
            wait_for_clean_subscription_end(&catalog_subscription, "origin catalog completion")
                .await;
            observations.push(MiniRelayStep::CatalogEnded);
            tokio::time::timeout(MINI_RELAY_TIMEOUT, origin_namespace.closed())
                .await
                .expect("origin namespace completion timed out")
                .expect("origin namespace failed");
            origin_namespace_fin
                .send(())
                .expect("test stopped before observing the origin namespace FIN");
            release_origin_ack_rx
                .await
                .expect("test stopped before releasing the origin namespace acknowledgment");
            observations.push(MiniRelayStep::NamespaceCompleted);

            // The acknowledged namespace handle is the draft-19 flush
            // barrier. Only release it after both subscribed tracks have
            // completed, then release the cache producers so the downstream
            // PUBLISH_NAMESPACE can finish in the same order.
            drop(origin_namespace);
            drop(relay_tracks_request);
            drop(relay_tracks_writer);
            tokio::time::timeout(MINI_RELAY_TIMEOUT, downstream_publish)
                .await
                .expect("downstream PUBLISH_NAMESPACE completion timed out")
                .expect("downstream PUBLISH_NAMESPACE task failed")
                .expect("downstream PUBLISH_NAMESPACE failed");

            origin_session.abort();
            downstream_session.abort();
            let _ = origin_session.await;
            let _ = downstream_session.await;
            observations
        });

        let policy = MoqRelayConnectionPolicy {
            attempt_timeout: Duration::from_secs(5),
            publish_namespace_acceptance_timeout: Duration::from_secs(3),
            substrate: relay_substrate,
            max_reconnect_attempts: 1,
            reconnect_initial_backoff: Duration::from_millis(10),
            reconnect_max_backoff: Duration::from_millis(10),
            reconnect_deadline: Duration::from_secs(1),
            jitter_percent: 0,
        };
        let client = MoqRelayClient::bind_development_with_policy(
            "127.0.0.1:0".parse().unwrap(),
            MoqRelayTlsConfig {
                disable_verification: true,
                ..Default::default()
            },
            MoqRelayDevelopmentMode::Insecure,
            policy,
        )
        .unwrap();
        let publisher = MoqBroadcastPublisher::new(config()).unwrap();
        let relay_publication = Arc::new(
            publisher
                .publish_to_relay(&client, &relay_url)
                .await
                .unwrap(),
        );
        assert_eq!(
            relay_publication.substrate,
            match expected_transport {
                Transport::RawQuic => BroadcastSubstrate::RawQuic,
                Transport::WebTransport => BroadcastSubstrate::WebTransport,
            }
        );
        origin_cached_rx
            .await
            .expect("mini relay ended before caching the live catalog");

        let downstream_connection = tokio::time::timeout(
            MINI_RELAY_TIMEOUT,
            downstream_client.connect_target(&target, native_substrate, Some(server_address)),
        )
        .await
        .expect("downstream transport connection timed out")
        .expect("downstream transport connection failed");
        assert_eq!(
            downstream_connection.negotiated.substrate,
            expected_transport
        );
        let (downstream_session, mut downstream_subscriber) = Subscriber::connect(
            downstream_connection.session,
            downstream_connection.negotiated,
        )
        .await
        .unwrap();
        let downstream_session = tokio::spawn(downstream_session.run());
        let mut downstream_namespace = tokio::time::timeout(
            MINI_RELAY_TIMEOUT,
            downstream_subscriber.published_namespace(),
        )
        .await
        .expect("downstream namespace timed out")
        .expect("downstream session ended before PUBLISH_NAMESPACE");
        assert_eq!(downstream_namespace.namespace, wire_namespace);
        downstream_namespace.ok().unwrap();

        let (downstream_catalog_writer, downstream_catalog_reader) = Track::new(
            downstream_namespace.namespace.clone(),
            TrackName::from(CATALOG_TRACK),
        )
        .produce();
        let catalog_subscription = downstream_subscriber
            .subscribe_open(downstream_catalog_writer)
            .await
            .expect("downstream catalog SUBSCRIBE failed");
        let mut downstream_catalog = subgroup_reader(&downstream_catalog_reader).await;
        let (live_group, live_payload) =
            read_catalog_group(&mut downstream_catalog, "downstream live catalog").await;
        assert_live_catalog(&live_payload);
        let mut downstream_observations = vec![MiniRelayStep::LiveCatalog];

        let (downstream_audio_writer, downstream_audio_reader) = Track::new(
            downstream_namespace.namespace.clone(),
            TrackName::from(AUDIO_TRACK),
        )
        .produce();
        let audio_subscription = downstream_subscriber
            .subscribe_open(downstream_audio_writer)
            .await
            .expect("downstream audio SUBSCRIBE failed");

        publisher.frames_out().send(opus_frame(960)).await.unwrap();
        let mut downstream_audio = subgroup_reader(&downstream_audio_reader).await;
        let _audio_group = read_audio_group(&mut downstream_audio).await;
        downstream_observations.push(MiniRelayStep::Audio);

        let drain_relay = Arc::clone(&relay_publication);
        let (relay_drain_polled, relay_drain_polled_rx) = oneshot::channel();
        let mut relay_drain = tokio::spawn(async move {
            let drain = drain_relay.drain(Utc::now() + chrono::Duration::seconds(8));
            tokio::pin!(drain);
            match futures::poll!(drain.as_mut()) {
                std::task::Poll::Ready(result) => {
                    let _ = relay_drain_polled.send(false);
                    result
                }
                std::task::Poll::Pending => {
                    let _ = relay_drain_polled.send(true);
                    drain.await
                }
            }
        });
        assert!(
            relay_drain_polled_rx
                .await
                .expect("per-relay drain task ended before reporting its first poll"),
            "per-relay drain completed on its first poll while the source remained live"
        );
        let drain_publisher = Arc::clone(&publisher);
        let drain = tokio::spawn(async move {
            drain_publisher
                .drain(BroadcastDrainRequest {
                    reason: rvoip_core_traits::broadcast::BroadcastDrainReason::Shutdown,
                    deadline: Utc::now() + chrono::Duration::seconds(8),
                })
                .await
        });
        wait_for_clean_subscription_end(&audio_subscription, "downstream audio completion").await;
        downstream_observations.push(MiniRelayStep::AudioEnded);
        let (terminal_group, terminal_payload) =
            read_catalog_group(&mut downstream_catalog, "downstream terminal catalog").await;
        assert!(terminal_group > live_group);
        assert_terminal_catalog(&terminal_payload);
        downstream_observations.push(MiniRelayStep::TerminalCatalog);
        wait_for_clean_subscription_end(&catalog_subscription, "downstream catalog completion")
            .await;
        downstream_observations.push(MiniRelayStep::CatalogEnded);
        tokio::time::timeout(MINI_RELAY_TIMEOUT, origin_namespace_fin_rx)
            .await
            .expect("relay did not observe the origin namespace FIN")
            .expect("relay stopped before observing the origin namespace FIN");
        assert!(relay_terminal_observed.load(Ordering::Acquire));
        tokio::select! {
            biased;
            completed = &mut relay_drain => {
                panic!("per-relay drain completed before peer acknowledgment: {completed:?}");
            }
            () = tokio::task::yield_now() => {}
        }
        assert!(
            !relay_drain.is_finished(),
            "per-relay drain completed before the peer acknowledgment barrier was released"
        );
        release_origin_ack
            .send(())
            .expect("relay stopped before the origin namespace acknowledgment was released");
        tokio::time::timeout(MINI_RELAY_TIMEOUT, downstream_namespace.closed())
            .await
            .expect("downstream namespace completion timed out")
            .expect("downstream namespace failed");
        downstream_observations.push(MiniRelayStep::NamespaceCompleted);
        drop(downstream_namespace);

        let relay_observations = tokio::time::timeout(MINI_RELAY_TIMEOUT, relay)
            .await
            .expect("mini relay task timed out")
            .expect("mini relay task failed");
        let drained = tokio::time::timeout(MINI_RELAY_TIMEOUT, drain)
            .await
            .expect("publisher drain timed out")
            .expect("publisher drain task failed")
            .expect("publisher drain failed");
        assert_eq!(drained.state, BroadcastDrainState::Drained);
        assert_eq!(publisher.lifecycle().state, BroadcastLifecycleState::Closed);
        let relay_drained = tokio::time::timeout(MINI_RELAY_TIMEOUT, relay_drain)
            .await
            .expect("per-relay drain timed out")
            .expect("per-relay drain task failed");
        assert!(relay_drained);
        assert!(
            relay_terminal_observed.load(Ordering::Acquire),
            "per-relay drain completed before the relay observed the terminal catalog"
        );
        assert!(!relay_publication.control.cancel.is_cancelled());
        relay_publication.wait().await.unwrap();
        downstream_session.abort();
        let _ = downstream_session.await;

        let expected = vec![
            MiniRelayStep::LiveCatalog,
            MiniRelayStep::Audio,
            MiniRelayStep::AudioEnded,
            MiniRelayStep::TerminalCatalog,
            MiniRelayStep::CatalogEnded,
            MiniRelayStep::NamespaceCompleted,
        ];
        assert_eq!(relay_observations, expected);
        assert_eq!(downstream_observations, expected);
    }

    #[cfg(feature = "insecure-development")]
    #[tokio::test]
    async fn raw_quic_and_webtransport_per_relay_drain_waits_for_terminal_delivery() {
        assert_two_hop_mini_relay(
            MoqRelaySubstratePolicy::RawQuic,
            quic::SubstratePolicy::RawQuic,
            Transport::RawQuic,
        )
        .await;
        assert_two_hop_mini_relay(
            MoqRelaySubstratePolicy::WebTransport,
            quic::SubstratePolicy::WebTransport,
            Transport::WebTransport,
        )
        .await;
    }

    #[tokio::test]
    async fn graceful_restart_persists_monotonic_groups_and_emits_terminal_catalogs() {
        let allocator = Arc::new(RecordingGroupIdAllocator::default());

        let first = MoqBroadcastPublisher::new_with_group_id_allocator(
            config(),
            Arc::clone(&allocator) as Arc<dyn MoqGroupIdAllocator>,
        )
        .unwrap();
        let first_wire = Arc::clone(&first.wire);
        first.frames_out().send(opus_frame(0)).await.unwrap();
        Arc::clone(&first).close().await.unwrap();
        assert!(first_wire.is_cleanly_completed_for_test());

        let second = MoqBroadcastPublisher::new_with_group_id_allocator(
            config(),
            Arc::clone(&allocator) as Arc<dyn MoqGroupIdAllocator>,
        )
        .unwrap();
        let second_wire = Arc::clone(&second.wire);
        second.frames_out().send(opus_frame(960)).await.unwrap();
        Arc::clone(&second).close().await.unwrap();
        assert!(second_wire.is_cleanly_completed_for_test());

        assert_eq!(allocator.reservations_for(AUDIO_TRACK), vec![0, 1]);
        assert_eq!(allocator.reservations_for(CATALOG_TRACK), vec![0, 1, 2, 3]);
    }

    #[tokio::test]
    async fn hard_wire_failure_consumes_audio_group_without_claiming_completion() {
        let allocator = Arc::new(RecordingGroupIdAllocator::default());
        let publisher = MoqBroadcastPublisher::new_with_group_id_allocator(
            config(),
            Arc::clone(&allocator) as Arc<dyn MoqGroupIdAllocator>,
        )
        .unwrap();
        publisher.wire.fail_writes_for_test();
        publisher.frames_out().send(opus_frame(0)).await.unwrap();

        tokio::time::timeout(Duration::from_secs(1), async {
            while publisher.lifecycle().state != BroadcastLifecycleState::Failed {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("wire failure must become terminal");

        assert_eq!(allocator.reservations_for(AUDIO_TRACK), vec![0]);
        assert_eq!(allocator.reservations_for(CATALOG_TRACK), vec![0]);
        assert!(!publisher.wire.is_cleanly_completed_for_test());
        assert!(Arc::clone(&publisher).close().await.is_err());
        assert_eq!(publisher.lifecycle().state, BroadcastLifecycleState::Failed);
        assert!(!publisher.wire.is_cleanly_completed_for_test());
    }

    #[tokio::test]
    async fn terminal_catalog_allocation_failure_makes_drain_and_close_nonclean() {
        let allocator = Arc::new(TerminalCatalogFailureAllocator::default());
        let publisher = MoqBroadcastPublisher::new_with_group_id_allocator(
            config(),
            allocator as Arc<dyn MoqGroupIdAllocator>,
        )
        .unwrap();
        publisher.frames_out().send(opus_frame(0)).await.unwrap();

        let drained = Arc::clone(&publisher)
            .drain(BroadcastDrainRequest {
                reason: rvoip_core_traits::broadcast::BroadcastDrainReason::Shutdown,
                deadline: Utc::now() + chrono::Duration::seconds(1),
            })
            .await
            .unwrap();

        assert_eq!(drained.state, BroadcastDrainState::DeadlineExceeded);
        assert_eq!(publisher.lifecycle().state, BroadcastLifecycleState::Failed);
        assert!(!publisher.wire.is_cleanly_completed_for_test());
        assert!(Arc::clone(&publisher).close().await.is_err());
        assert_eq!(publisher.lifecycle().state, BroadcastLifecycleState::Failed);
    }

    #[tokio::test]
    async fn audio_wire_failure_racing_drain_never_reports_clean_completion() {
        let publisher = MoqBroadcastPublisher::new(config()).unwrap();
        publisher.wire.fail_writes_for_test();
        publisher.frames_out().send(opus_frame(0)).await.unwrap();

        let drained = Arc::clone(&publisher)
            .drain(BroadcastDrainRequest {
                reason: rvoip_core_traits::broadcast::BroadcastDrainReason::Shutdown,
                deadline: Utc::now() + chrono::Duration::seconds(1),
            })
            .await
            .unwrap();

        assert_eq!(drained.state, BroadcastDrainState::DeadlineExceeded);
        assert_eq!(publisher.lifecycle().state, BroadcastLifecycleState::Failed);
        assert!(!publisher.wire.is_cleanly_completed_for_test());
        assert!(Arc::clone(&publisher).close().await.is_err());
        assert_eq!(publisher.lifecycle().state, BroadcastLifecycleState::Failed);
    }

    fn verified_test_peer_identity() -> MoqRelayPeerIdentity {
        verified_test_peer_identity_for("accepted")
    }

    fn verified_test_peer_identity_for(connection_id: &str) -> MoqRelayPeerIdentity {
        let marker = connection_id
            .bytes()
            .fold(0u8, |accumulator, byte| accumulator.wrapping_add(byte));
        MoqRelayPeerIdentity::VerifiedCertificate {
            leaf_sha256: format!("{marker:02x}").repeat(32),
            chain_len: 1,
            total_der_bytes: 512,
        }
    }

    struct MockConnection {
        id: String,
        termination: Option<oneshot::Receiver<MoqRelayFailure>>,
        closed: Arc<AtomicBool>,
    }

    struct PanickingConnection {
        panic_gate: Option<oneshot::Receiver<()>>,
    }

    struct BlockingCloseConnection {
        close_started: Arc<Notify>,
        allow_close: Arc<Notify>,
    }

    #[async_trait]
    impl RelayConnection for PanickingConnection {
        fn connection_id(&self) -> &str {
            "panic-connection"
        }

        fn relay_path(&self) -> &'static str {
            "raw-quic"
        }

        fn endpoint_uri(&self) -> &str {
            "moqt://relay.invalid/"
        }

        fn substrate(&self) -> BroadcastSubstrate {
            BroadcastSubstrate::RawQuic
        }

        fn negotiated_protocol(&self) -> &str {
            "moqt-19"
        }

        fn peer_identity(&self) -> MoqRelayPeerIdentity {
            verified_test_peer_identity()
        }

        async fn terminated(&mut self) -> WireRelayTermination {
            let _ = self
                .panic_gate
                .take()
                .expect("panic gate already consumed")
                .await;
            panic!("injected relay task panic");
        }

        async fn graceful_finish(&mut self) -> WireRelayTermination {
            self.terminated().await
        }

        async fn close(&mut self) {}
    }

    #[async_trait]
    impl RelayConnection for BlockingCloseConnection {
        fn connection_id(&self) -> &str {
            "blocking-close"
        }

        fn relay_path(&self) -> &'static str {
            "raw-quic"
        }

        fn endpoint_uri(&self) -> &str {
            "moqt://relay.invalid/"
        }

        fn substrate(&self) -> BroadcastSubstrate {
            BroadcastSubstrate::RawQuic
        }

        fn negotiated_protocol(&self) -> &str {
            "moqt-19"
        }

        fn peer_identity(&self) -> MoqRelayPeerIdentity {
            verified_test_peer_identity()
        }

        async fn terminated(&mut self) -> WireRelayTermination {
            pending().await
        }

        async fn graceful_finish(&mut self) -> WireRelayTermination {
            self.close().await;
            WireRelayTermination::Completed
        }

        async fn close(&mut self) {
            self.close_started.notify_one();
            self.allow_close.notified().await;
        }
    }

    #[async_trait]
    impl RelayConnection for MockConnection {
        fn connection_id(&self) -> &str {
            &self.id
        }

        fn relay_path(&self) -> &'static str {
            "raw-quic"
        }

        fn endpoint_uri(&self) -> &str {
            "moqt://relay.invalid/"
        }

        fn substrate(&self) -> BroadcastSubstrate {
            BroadcastSubstrate::RawQuic
        }

        fn negotiated_protocol(&self) -> &str {
            "moqt-19"
        }

        fn peer_identity(&self) -> MoqRelayPeerIdentity {
            verified_test_peer_identity_for(&self.id)
        }

        async fn terminated(&mut self) -> WireRelayTermination {
            match self.termination.take() {
                Some(receiver) => WireRelayTermination::Failed(
                    receiver.await.unwrap_or(MoqRelayFailure::TaskFailed),
                ),
                None => pending().await,
            }
        }

        async fn graceful_finish(&mut self) -> WireRelayTermination {
            self.closed.store(true, Ordering::Release);
            WireRelayTermination::Completed
        }

        async fn close(&mut self) {
            self.closed.store(true, Ordering::Release);
        }
    }

    enum MockPlan {
        Ready(MockConnection),
        Panicking(PanickingConnection),
        BlockingClose(BlockingCloseConnection),
        Failed(MoqRelayFailure),
    }

    struct MockConnector {
        plans: Mutex<VecDeque<MockPlan>>,
        attempts: AtomicUsize,
    }

    impl MockConnector {
        fn new(plans: impl IntoIterator<Item = MockPlan>) -> Self {
            Self {
                plans: Mutex::new(plans.into_iter().collect()),
                attempts: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl RelayConnector for MockConnector {
        async fn connect(
            &self,
            _publication: &WirePublicationHandle,
            _relay: &Url,
            _substrate: MoqRelaySubstratePolicy,
            _acceptance_timeout: Duration,
        ) -> Result<Box<dyn RelayConnection>, MoqError> {
            self.attempts.fetch_add(1, Ordering::AcqRel);
            match self
                .plans
                .lock()
                .expect("mock plans poisoned")
                .pop_front()
                .expect("unexpected mock connection attempt")
            {
                MockPlan::Ready(connection) => Ok(Box::new(connection)),
                MockPlan::Panicking(connection) => Ok(Box::new(connection)),
                MockPlan::BlockingClose(connection) => Ok(Box::new(connection)),
                MockPlan::Failed(failure) => Err(MoqError::RelayFailure(failure)),
            }
        }
    }

    struct GatedConnector {
        ready: Arc<Notify>,
        closed: Arc<AtomicBool>,
    }

    struct PendingConnector {
        entered: Arc<Notify>,
    }

    struct ErrorGatedConnector {
        entered: Arc<Notify>,
        release: Arc<Notify>,
        error: Mutex<Option<MoqError>>,
    }

    #[async_trait]
    impl RelayConnector for GatedConnector {
        async fn connect(
            &self,
            _publication: &WirePublicationHandle,
            _relay: &Url,
            _substrate: MoqRelaySubstratePolicy,
            _acceptance_timeout: Duration,
        ) -> Result<Box<dyn RelayConnection>, MoqError> {
            self.ready.notified().await;
            Ok(Box::new(MockConnection {
                id: "ready-connection".into(),
                termination: None,
                closed: Arc::clone(&self.closed),
            }))
        }
    }

    #[async_trait]
    impl RelayConnector for PendingConnector {
        async fn connect(
            &self,
            _publication: &WirePublicationHandle,
            _relay: &Url,
            _substrate: MoqRelaySubstratePolicy,
            _acceptance_timeout: Duration,
        ) -> Result<Box<dyn RelayConnection>, MoqError> {
            self.entered.notify_one();
            pending().await
        }
    }

    #[async_trait]
    impl RelayConnector for ErrorGatedConnector {
        async fn connect(
            &self,
            _publication: &WirePublicationHandle,
            _relay: &Url,
            _substrate: MoqRelaySubstratePolicy,
            _acceptance_timeout: Duration,
        ) -> Result<Box<dyn RelayConnection>, MoqError> {
            self.entered.notify_one();
            self.release.notified().await;
            Err(self
                .error
                .lock()
                .expect("gated error poisoned")
                .take()
                .expect("gated error already consumed"))
        }
    }

    fn test_policy() -> MoqRelayConnectionPolicy {
        MoqRelayConnectionPolicy {
            attempt_timeout: Duration::from_secs(1),
            publish_namespace_acceptance_timeout: Duration::from_millis(100),
            substrate: MoqRelaySubstratePolicy::RawQuic,
            max_reconnect_attempts: 2,
            reconnect_initial_backoff: Duration::ZERO,
            reconnect_max_backoff: Duration::ZERO,
            reconnect_deadline: Duration::from_secs(1),
            jitter_percent: 0,
        }
    }

    fn test_client(
        connector: Arc<dyn RelayConnector>,
        policy: MoqRelayConnectionPolicy,
    ) -> MoqRelayClient {
        policy.validate().unwrap();
        MoqRelayClient { connector, policy }
    }

    fn relay_url() -> Url {
        Url::parse("moqt://relay.invalid:443").unwrap()
    }

    async fn gated_connect_error(error: MoqError) -> (MoqError, RelaySnapshot) {
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let client = Arc::new(test_client(
            Arc::new(ErrorGatedConnector {
                entered: Arc::clone(&entered),
                release: Arc::clone(&release),
                error: Mutex::new(Some(error)),
            }),
            test_policy(),
        ));
        let publisher = MoqBroadcastPublisher::new(config()).unwrap();
        let publish_publisher = Arc::clone(&publisher);
        let publish = tokio::spawn(async move {
            publish_publisher
                .publish_to_relay(&client, &relay_url())
                .await
        });

        entered.notified().await;
        let control = publisher
            .management
            .active_relays()
            .into_iter()
            .next()
            .expect("pending publication must register relay lifecycle");
        release.notify_one();
        let error = match publish.await.unwrap() {
            Err(error) => error,
            Ok(_) => panic!("gated connector error unexpectedly produced a publication"),
        };
        let snapshot = control.status.snapshot();
        assert_eq!(snapshot.lifecycle, BroadcastLifecycleState::Failed);
        assert_eq!(control.cleanup.wait().await, TaskCleanupResult::Completed);
        assert!(Arc::clone(&publisher).close().await.is_err());
        assert_eq!(publisher.lifecycle().state, BroadcastLifecycleState::Failed);
        (error, snapshot)
    }

    #[tokio::test]
    async fn publishes_through_the_transport_neutral_contract() {
        let publisher = MoqBroadcastPublisher::new(config()).unwrap();
        assert_eq!(publisher.namespace().as_str(), "tenant-a/broadcast-1");
        assert_eq!(
            publisher.descriptor().protocol_version,
            "draft-ietf-moq-transport-19; draft-ietf-moq-msf-01; draft-ietf-moq-loc-03"
        );
        assert_eq!(publisher.protocol().transport_version, MOQT_DRAFT);
        assert_eq!(
            publisher.protocol().media_format_version.as_deref(),
            Some(MSF_DRAFT)
        );
        assert_eq!(
            publisher.protocol().object_format_version.as_deref(),
            Some(LOC_DRAFT)
        );

        publisher
            .frames_out()
            .send(MediaFrame {
                stream_id: StreamId::new(),
                kind: StreamKind::Audio,
                payload: Bytes::from_static(&[0x78, 0x00]),
                timestamp_rtp: 960,
                captured_at: Utc::now(),
                payload_type: Some(111),
            })
            .await
            .unwrap();
        publisher.close().await.unwrap();
    }

    #[tokio::test]
    async fn explicit_acceptance_transitions_to_ready_with_negotiated_diagnostics() {
        let ready = Arc::new(Notify::new());
        let closed = Arc::new(AtomicBool::new(false));
        let client = Arc::new(test_client(
            Arc::new(GatedConnector {
                ready: Arc::clone(&ready),
                closed: Arc::clone(&closed),
            }),
            test_policy(),
        ));
        let publisher = MoqBroadcastPublisher::new(config()).unwrap();
        let publish_publisher = Arc::clone(&publisher);
        let task = tokio::spawn(async move {
            publish_publisher
                .publish_to_relay(&client, &relay_url())
                .await
        });

        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(!task.is_finished());
        assert_eq!(
            publisher.lifecycle().state,
            BroadcastLifecycleState::Starting
        );

        ready.notify_one();
        let publication = task.await.unwrap().unwrap();
        assert_eq!(
            publication.lifecycle().state,
            BroadcastLifecycleState::Ready
        );
        assert_eq!(publication.health().status, BroadcastHealthStatus::Healthy);
        assert!(publication.moq_health().issues.is_empty());
        assert!(publisher.moq_health().issues.is_empty());
        assert_eq!(publication.endpoint_uri, "moqt://relay.invalid/");
        assert_eq!(publication.substrate, BroadcastSubstrate::RawQuic);
        assert_eq!(publication.negotiated_protocol, "moqt-19");
        assert_eq!(
            publication.peer_identity,
            verified_test_peer_identity_for("ready-connection")
        );
        assert_eq!(
            publication.current_endpoint_uri().as_deref(),
            Some("moqt://relay.invalid/")
        );
        assert_eq!(
            publication.current_negotiated_protocol().as_deref(),
            Some("moqt-19")
        );
        assert_eq!(
            publication.current_peer_identity(),
            Some(verified_test_peer_identity_for("ready-connection"))
        );
        assert_eq!(
            publisher.endpoint().uri.as_deref(),
            Some("moqt://relay.invalid/")
        );
        assert_eq!(
            publisher.protocol().substrate,
            Some(BroadcastSubstrate::RawQuic)
        );
        publication
            .drain(Utc::now() + chrono::Duration::seconds(1))
            .await;
        assert!(closed.load(Ordering::Acquire));
        publisher.close().await.unwrap();
    }

    #[tokio::test]
    async fn namespace_rejection_is_typed_and_marks_the_lifecycle_failed() {
        let (error, snapshot) = gated_connect_error(MoqError::PublishNamespaceRejected {
            error_code: 0x1,
            retry_interval: 250,
            reason: "denied".into(),
        })
        .await;

        assert!(matches!(
            error,
            MoqError::PublishNamespaceRejected {
                error_code: 0x1,
                retry_interval: 250,
                reason,
            } if reason == "denied"
        ));
        assert_eq!(
            snapshot.failure,
            Some(MoqRelayFailure::PublishNamespaceRejected)
        );
    }

    #[tokio::test]
    async fn silent_namespace_acceptance_is_a_typed_timeout_and_failed_lifecycle() {
        let timeout = Duration::from_millis(25);
        let (error, snapshot) =
            gated_connect_error(MoqError::PublishNamespaceAcceptanceTimedOut { timeout }).await;

        assert!(matches!(
            error,
            MoqError::PublishNamespaceAcceptanceTimedOut { timeout: actual }
                if actual == timeout
        ));
        assert_eq!(
            snapshot.failure,
            Some(MoqRelayFailure::PublishNamespaceAcceptanceTimedOut)
        );
    }

    #[tokio::test]
    async fn response_stream_disconnect_is_typed_and_marks_the_lifecycle_failed() {
        let (error, snapshot) =
            gated_connect_error(MoqError::PublishNamespaceResponseStreamClosed).await;

        assert!(matches!(
            error,
            MoqError::PublishNamespaceResponseStreamClosed
        ));
        assert_eq!(
            snapshot.failure,
            Some(MoqRelayFailure::PublishNamespaceResponseStreamClosed)
        );
    }

    #[tokio::test]
    async fn unauthenticated_relay_identity_is_typed_and_marks_the_lifecycle_failed() {
        let (error, snapshot) = gated_connect_error(MoqError::RelayPeerUnauthenticated).await;

        assert!(matches!(error, MoqError::RelayPeerUnauthenticated));
        assert_eq!(
            snapshot.failure,
            Some(MoqRelayFailure::RelayPeerUnauthenticated)
        );
    }

    #[tokio::test]
    async fn cancelling_pending_relay_publish_completes_uninstalled_cleanup() {
        let entered = Arc::new(Notify::new());
        let client = Arc::new(test_client(
            Arc::new(PendingConnector {
                entered: Arc::clone(&entered),
            }),
            test_policy(),
        ));
        let publisher = MoqBroadcastPublisher::new(config()).unwrap();
        let publish_publisher = Arc::clone(&publisher);
        let publish = tokio::spawn(async move {
            publish_publisher
                .publish_to_relay(&client, &relay_url())
                .await
        });

        entered.notified().await;
        let control = publisher
            .management
            .active_relays()
            .into_iter()
            .next()
            .expect("pending publication must register its relay control");
        let cleanup = control.cleanup.clone();
        publish.abort();
        assert!(matches!(publish.await, Err(error) if error.is_cancelled()));

        drop(control);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), cleanup.wait())
                .await
                .expect("cancelled pending publication cleanup must complete"),
            TaskCleanupResult::Completed
        );
        assert_eq!(cleanup.start_count(), 0);
        assert!(publisher.management.active_relays().is_empty());
        publisher.close().await.unwrap();
    }

    #[tokio::test]
    async fn asynchronous_failure_is_retried_then_exposed() {
        let (terminate, termination) = oneshot::channel();
        let connector = Arc::new(MockConnector::new([
            MockPlan::Ready(MockConnection {
                id: "initial".into(),
                termination: Some(termination),
                closed: Arc::new(AtomicBool::new(false)),
            }),
            MockPlan::Failed(MoqRelayFailure::ConnectFailed),
            MockPlan::Failed(MoqRelayFailure::ConnectTimeout),
        ]));
        let client = test_client(connector.clone(), test_policy());
        let publisher = MoqBroadcastPublisher::new(config()).unwrap();
        let publication = publisher
            .publish_to_relay(&client, &relay_url())
            .await
            .unwrap();
        assert_eq!(
            publication.current_peer_identity(),
            Some(verified_test_peer_identity_for("initial"))
        );

        terminate.send(MoqRelayFailure::SessionEnded).unwrap();
        let error = publication.wait().await.unwrap_err();
        assert!(matches!(
            error,
            MoqError::RelayFailure(MoqRelayFailure::ReconnectExhausted)
        ));
        assert_eq!(
            publication.lifecycle().state,
            BroadcastLifecycleState::Failed
        );
        assert_eq!(
            publication.health().status,
            BroadcastHealthStatus::Unhealthy
        );
        assert_eq!(connector.attempts.load(Ordering::Acquire), 3);
        assert!(Arc::clone(&publisher).close().await.is_err());
        assert_eq!(publisher.lifecycle().state, BroadcastLifecycleState::Failed);
    }

    #[tokio::test]
    async fn panicking_supervisor_becomes_task_failed_and_unblocks_wait() {
        let (panic_now, panic_gate) = oneshot::channel();
        let connector = Arc::new(MockConnector::new([MockPlan::Panicking(
            PanickingConnection {
                panic_gate: Some(panic_gate),
            },
        )]));
        let client = test_client(connector, test_policy());
        let publisher = MoqBroadcastPublisher::new(config()).unwrap();
        let publication = publisher
            .publish_to_relay(&client, &relay_url())
            .await
            .unwrap();

        panic_now.send(()).unwrap();
        let error = tokio::time::timeout(Duration::from_secs(1), publication.wait())
            .await
            .expect("wait must not hang after a task panic")
            .unwrap_err();
        assert!(matches!(
            error,
            MoqError::RelayFailure(MoqRelayFailure::TaskFailed)
        ));
        assert!(Arc::clone(&publisher).close().await.is_err());
        assert_eq!(publisher.lifecycle().state, BroadcastLifecycleState::Failed);
    }

    #[tokio::test]
    async fn forced_task_cancellation_is_terminal_task_failed() {
        let connector = Arc::new(MockConnector::new([MockPlan::Ready(MockConnection {
            id: "cancelled".into(),
            termination: None,
            closed: Arc::new(AtomicBool::new(false)),
        })]));
        let client = test_client(connector, test_policy());
        let publisher = MoqBroadcastPublisher::new(config()).unwrap();
        let publication = publisher
            .publish_to_relay(&client, &relay_url())
            .await
            .unwrap();

        publication.control.abort_and_reap().await;
        assert!(matches!(
            publication.wait().await,
            Err(MoqError::RelayFailure(MoqRelayFailure::TaskFailed))
        ));
        assert!(publication.moq_health().issues.is_empty());
        let drained = Arc::clone(&publisher)
            .drain(BroadcastDrainRequest {
                reason: rvoip_core_traits::broadcast::BroadcastDrainReason::Shutdown,
                deadline: Utc::now() + chrono::Duration::seconds(1),
            })
            .await
            .unwrap();
        assert_eq!(drained.state, BroadcastDrainState::DeadlineExceeded);
        assert!(Arc::clone(&publisher).close().await.is_err());
        assert_eq!(publisher.lifecycle().state, BroadcastLifecycleState::Failed);
    }

    #[tokio::test]
    async fn successful_reconnect_updates_the_observable_connection() {
        let (terminate, termination) = oneshot::channel();
        let reconnected_closed = Arc::new(AtomicBool::new(false));
        let connector = Arc::new(MockConnector::new([
            MockPlan::Ready(MockConnection {
                id: "initial".into(),
                termination: Some(termination),
                closed: Arc::new(AtomicBool::new(false)),
            }),
            MockPlan::Ready(MockConnection {
                id: "reconnected".into(),
                termination: None,
                closed: Arc::clone(&reconnected_closed),
            }),
        ]));
        let client = test_client(connector, test_policy());
        let publisher = MoqBroadcastPublisher::new(config()).unwrap();
        let publication = publisher
            .publish_to_relay(&client, &relay_url())
            .await
            .unwrap();
        assert_eq!(
            publication.current_peer_identity(),
            Some(verified_test_peer_identity_for("initial"))
        );

        terminate.send(MoqRelayFailure::SessionEnded).unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if publication.current_connection_id().as_deref() == Some("reconnected") {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            publication.lifecycle().state,
            BroadcastLifecycleState::Ready
        );
        assert_eq!(publication.current_relay_path(), Some("raw-quic"));
        assert_eq!(
            publication.current_peer_identity(),
            Some(verified_test_peer_identity_for("reconnected"))
        );

        publication
            .drain(Utc::now() + chrono::Duration::seconds(1))
            .await;
        assert!(reconnected_closed.load(Ordering::Acquire));
        publisher.close().await.unwrap();
    }

    #[tokio::test]
    async fn drain_stops_admission_and_reaps_relay_tasks() {
        let closed = Arc::new(AtomicBool::new(false));
        let connector = Arc::new(MockConnector::new([MockPlan::Ready(MockConnection {
            id: "initial".into(),
            termination: None,
            closed: Arc::clone(&closed),
        })]));
        let client = test_client(connector, test_policy());
        let publisher = MoqBroadcastPublisher::new(config()).unwrap();
        let publication = publisher
            .publish_to_relay(&client, &relay_url())
            .await
            .unwrap();

        let drained = Arc::clone(&publisher)
            .drain(BroadcastDrainRequest {
                reason: rvoip_core_traits::broadcast::BroadcastDrainReason::Shutdown,
                deadline: Utc::now() + chrono::Duration::seconds(1),
            })
            .await
            .unwrap();
        assert_eq!(drained.state, BroadcastDrainState::Drained);
        assert!(closed.load(Ordering::Acquire));
        assert_eq!(publisher.lifecycle().state, BroadcastLifecycleState::Closed);
        assert_eq!(
            publication.lifecycle().state,
            BroadcastLifecycleState::Closed
        );
        assert!(publication.moq_health().issues.is_empty());
        assert!(matches!(
            publisher.publish_to_relay(&client, &relay_url()).await,
            Err(MoqError::Closed)
        ));
        assert!(publisher.frames_out().send(opus_frame(0)).await.is_err());
    }

    #[tokio::test]
    async fn concurrent_relay_drains_share_one_cleanup_completion() {
        let close_started = Arc::new(Notify::new());
        let allow_close = Arc::new(Notify::new());
        let connector = Arc::new(MockConnector::new([MockPlan::BlockingClose(
            BlockingCloseConnection {
                close_started: Arc::clone(&close_started),
                allow_close: Arc::clone(&allow_close),
            },
        )]));
        let client = test_client(connector, test_policy());
        let publisher = MoqBroadcastPublisher::new(config()).unwrap();
        let publication = Arc::new(
            publisher
                .publish_to_relay(&client, &relay_url())
                .await
                .unwrap(),
        );
        let first_publication = Arc::clone(&publication);
        let second_publication = Arc::clone(&publication);
        let deadline = Utc::now() + chrono::Duration::seconds(2);
        let first = tokio::spawn(async move { first_publication.drain(deadline).await });
        let second = tokio::spawn(async move { second_publication.drain(deadline).await });

        close_started.notified().await;
        assert!(!first.is_finished());
        assert!(!second.is_finished());
        allow_close.notify_one();
        assert!(first.await.unwrap());
        assert!(second.await.unwrap());
        assert_eq!(publication.control.cleanup.start_count(), 1);
        assert_eq!(
            publication.control.cleanup.wait().await,
            TaskCleanupResult::Completed
        );
        publisher.close().await.unwrap();
    }

    #[tokio::test]
    async fn hard_cancel_cannot_satisfy_a_graceful_per_relay_drain() {
        let closed = Arc::new(AtomicBool::new(false));
        let connector = Arc::new(MockConnector::new([MockPlan::Ready(MockConnection {
            id: "hard-cancel".into(),
            termination: None,
            closed: Arc::clone(&closed),
        })]));
        let client = test_client(connector, test_policy());
        let publisher = MoqBroadcastPublisher::new(config()).unwrap();
        let publication = publisher
            .publish_to_relay(&client, &relay_url())
            .await
            .unwrap();

        publication.control.cancel.cancel();
        let graceful = publication
            .drain(Utc::now() + chrono::Duration::seconds(1))
            .await;

        assert!(!graceful);
        assert!(publication.control.cancel.is_cancelled());
        assert!(closed.load(Ordering::Acquire));
        assert_eq!(
            publication.control.cleanup.wait().await,
            TaskCleanupResult::Completed
        );
        assert!(publisher.close().await.is_err());
    }

    #[tokio::test]
    async fn concurrent_publisher_drain_and_close_share_all_cleanup() {
        let close_started = Arc::new(Notify::new());
        let allow_close = Arc::new(Notify::new());
        let connector = Arc::new(MockConnector::new([MockPlan::BlockingClose(
            BlockingCloseConnection {
                close_started: Arc::clone(&close_started),
                allow_close: Arc::clone(&allow_close),
            },
        )]));
        let client = test_client(connector, test_policy());
        let publisher = MoqBroadcastPublisher::new(config()).unwrap();
        let publication = publisher
            .publish_to_relay(&client, &relay_url())
            .await
            .unwrap();
        let drain_publisher = Arc::clone(&publisher);
        let close_publisher = Arc::clone(&publisher);
        let drain = tokio::spawn(async move {
            drain_publisher
                .drain(BroadcastDrainRequest {
                    reason: rvoip_core_traits::broadcast::BroadcastDrainReason::Shutdown,
                    deadline: Utc::now() + chrono::Duration::seconds(2),
                })
                .await
        });
        let close = tokio::spawn(async move { close_publisher.close().await });

        close_started.notified().await;
        assert!(!drain.is_finished());
        assert!(!close.is_finished());
        allow_close.notify_one();
        assert_eq!(
            drain.await.unwrap().unwrap().state,
            BroadcastDrainState::Drained
        );
        close.await.unwrap().unwrap();
        assert_eq!(publication.control.cleanup.start_count(), 1);
        assert_eq!(publisher.frame_cleanup.start_count(), 1);
        assert_eq!(
            publication.control.cleanup.wait().await,
            TaskCleanupResult::Completed
        );
        assert_eq!(
            publisher.frame_cleanup.wait().await,
            TaskCleanupResult::Completed
        );
    }

    #[tokio::test]
    async fn relay_handle_drop_hands_task_to_cleanup_reaper() {
        let closed = Arc::new(AtomicBool::new(false));
        let connector = Arc::new(MockConnector::new([MockPlan::Ready(MockConnection {
            id: "drop-cleanup".into(),
            termination: None,
            closed: Arc::clone(&closed),
        })]));
        let client = test_client(connector, test_policy());
        let publisher = MoqBroadcastPublisher::new(config()).unwrap();
        let publication = publisher
            .publish_to_relay(&client, &relay_url())
            .await
            .unwrap();
        let control = Arc::clone(&publication.control);

        drop(publication);
        let terminal = tokio::time::timeout(Duration::from_secs(1), control.status.wait_terminal())
            .await
            .expect("drop cleanup must terminate");
        assert_eq!(terminal.lifecycle, BroadcastLifecycleState::Closed);
        assert_eq!(control.cleanup.wait().await, TaskCleanupResult::Completed);
        assert_eq!(control.cleanup.start_count(), 1);
        assert!(closed.load(Ordering::Acquire));
        assert!(publisher.close().await.is_err());
    }

    #[tokio::test]
    async fn publisher_drop_reaps_frame_and_relay_tasks() {
        let closed = Arc::new(AtomicBool::new(false));
        let connector = Arc::new(MockConnector::new([MockPlan::Ready(MockConnection {
            id: "publisher-drop".into(),
            termination: None,
            closed: Arc::clone(&closed),
        })]));
        let client = test_client(connector, test_policy());
        let publisher = MoqBroadcastPublisher::new(config()).unwrap();
        let publication = publisher
            .publish_to_relay(&client, &relay_url())
            .await
            .unwrap();
        let control = Arc::clone(&publication.control);

        drop(publisher);
        let terminal = tokio::time::timeout(Duration::from_secs(1), control.status.wait_terminal())
            .await
            .expect("publisher drop cleanup must terminate");
        assert_eq!(terminal.lifecycle, BroadcastLifecycleState::Closed);
        assert!(closed.load(Ordering::Acquire));
        assert_eq!(control.cleanup.wait().await, TaskCleanupResult::Completed);
        assert_eq!(control.cleanup.start_count(), 1);
        drop(publication);
    }

    #[tokio::test]
    async fn local_wire_failure_closes_wire_and_reaps_relays() {
        let closed = Arc::new(AtomicBool::new(false));
        let connector = Arc::new(MockConnector::new([MockPlan::Ready(MockConnection {
            id: "local-failure".into(),
            termination: None,
            closed: Arc::clone(&closed),
        })]));
        let client = test_client(connector, test_policy());
        let publisher = MoqBroadcastPublisher::new(config()).unwrap();
        let publication = publisher
            .publish_to_relay(&client, &relay_url())
            .await
            .unwrap();

        publisher.wire.fail_writes_for_test();
        publisher
            .frames_out()
            .send(MediaFrame {
                stream_id: StreamId::new(),
                kind: StreamKind::Audio,
                payload: Bytes::from_static(&[0x78, 0x00]),
                timestamp_rtp: 960,
                captured_at: Utc::now(),
                payload_type: Some(111),
            })
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if publisher.lifecycle().state == BroadcastLifecycleState::Failed {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("local wire failure must become terminal");
        assert!(closed.load(Ordering::Acquire));
        assert!(publisher.wire.is_closed_for_test());
        assert_eq!(
            publication.lifecycle().state,
            BroadcastLifecycleState::Closed
        );
        assert!(Arc::clone(&publisher).close().await.is_err());
        assert_eq!(publisher.lifecycle().state, BroadcastLifecycleState::Failed);
    }

    #[test]
    fn production_tls_is_mutual_and_debug_output_is_redacted() {
        let tls = MoqRelayTlsConfig {
            root_certificates: vec![PathBuf::from("/secret/relay-ca.pem")],
            client_certificate: None,
            client_private_key: None,
            #[cfg(feature = "insecure-development")]
            disable_verification: false,
        };
        let rendered = format!("{tls:?}");
        assert!(!rendered.contains("/secret"));
        assert!(matches!(
            MoqRelayClient::bind("127.0.0.1:0".parse().unwrap(), tls),
            Err(MoqError::TlsConfiguration(_))
        ));
    }

    #[test]
    fn reconnect_backoff_is_exponential_capped_and_deterministic() {
        let policy = MoqRelayConnectionPolicy {
            reconnect_initial_backoff: Duration::from_millis(100),
            reconnect_max_backoff: Duration::from_millis(250),
            jitter_percent: 0,
            ..test_policy()
        };
        assert_eq!(
            reconnect_delay(&policy, 1, u64::MAX),
            Duration::from_millis(100)
        );
        assert_eq!(
            reconnect_delay(&policy, 2, u64::MAX),
            Duration::from_millis(200)
        );
        assert_eq!(
            reconnect_delay(&policy, 3, u64::MAX),
            Duration::from_millis(250)
        );
    }

    #[test]
    fn acceptance_deadline_must_precede_the_outer_attempt_deadline() {
        let mut policy = test_policy();
        policy.publish_namespace_acceptance_timeout = policy.attempt_timeout;
        assert!(matches!(policy.validate(), Err(MoqError::InvalidConfig(_))));

        policy.publish_namespace_acceptance_timeout = Duration::ZERO;
        assert!(matches!(policy.validate(), Err(MoqError::InvalidConfig(_))));
    }

    #[test]
    fn relay_lifecycle_requires_accepted_diagnostics_before_ready() {
        let status = RelayStatus::new();
        assert_eq!(status.lifecycle().state, BroadcastLifecycleState::Starting);
        assert_eq!(status.health().status, BroadcastHealthStatus::Degraded);

        status.transition(BroadcastLifecycleState::Ready, None, None);
        assert_eq!(status.lifecycle().state, BroadcastLifecycleState::Degraded);
        assert_eq!(status.health().status, BroadcastHealthStatus::Degraded);

        let accepted = RelayStatus::new();
        accepted.transition(
            BroadcastLifecycleState::Ready,
            None,
            Some(RelayDiagnostics {
                connection_id: "accepted".into(),
                relay_path: "raw-quic",
                endpoint_uri: "moqt://relay.invalid/".into(),
                substrate: BroadcastSubstrate::RawQuic,
                negotiated_protocol: "moqt-19".into(),
                peer_identity: verified_test_peer_identity(),
            }),
        );
        assert_eq!(accepted.lifecycle().state, BroadcastLifecycleState::Ready);
        assert_eq!(accepted.health().status, BroadcastHealthStatus::Healthy);

        for (state, expected_health) in [
            (
                BroadcastLifecycleState::Degraded,
                BroadcastHealthStatus::Degraded,
            ),
            (
                BroadcastLifecycleState::Reconnecting,
                BroadcastHealthStatus::Degraded,
            ),
            (
                BroadcastLifecycleState::Draining,
                BroadcastHealthStatus::Degraded,
            ),
            (
                BroadcastLifecycleState::Closed,
                BroadcastHealthStatus::Closed,
            ),
        ] {
            status.transition(state, None, None);
            assert_eq!(status.lifecycle().state, state);
            assert_eq!(status.health().status, expected_health);
        }

        // Terminal state is immutable even if a late reconnect completion races
        // with cancellation.
        status.transition(BroadcastLifecycleState::Degraded, None, None);
        assert_eq!(status.lifecycle().state, BroadcastLifecycleState::Closed);

        let failed = RelayStatus::new();
        failed.transition(
            BroadcastLifecycleState::Failed,
            Some(MoqRelayFailure::ReconnectExhausted),
            None,
        );
        assert_eq!(failed.health().status, BroadcastHealthStatus::Unhealthy);
    }

    #[test]
    fn aggregate_since_tracks_when_mixed_health_actually_began() {
        let base = Utc::now() - chrono::Duration::seconds(30);
        let snapshot = |lifecycle, offset| RelaySnapshot {
            lifecycle,
            since: base + chrono::Duration::seconds(offset),
            failure: None,
            diagnostics: None,
        };
        let mixed = aggregate_relay_lifecycle(
            &[
                snapshot(BroadcastLifecycleState::Ready, 2),
                snapshot(BroadcastLifecycleState::Failed, 5),
                snapshot(BroadcastLifecycleState::Degraded, 8),
            ],
            Some(base),
        );
        assert_eq!(mixed.state, BroadcastLifecycleState::Degraded);
        assert_eq!(mixed.since, Some(base + chrono::Duration::seconds(5)));

        let all_failed = aggregate_relay_lifecycle(
            &[
                snapshot(BroadcastLifecycleState::Failed, 3),
                snapshot(BroadcastLifecycleState::Failed, 9),
            ],
            Some(base),
        );
        assert_eq!(all_failed.state, BroadcastLifecycleState::Failed);
        assert_eq!(all_failed.since, Some(base + chrono::Duration::seconds(9)));
    }

    #[tokio::test]
    async fn rejects_namespaces_instead_of_collapsing_them() {
        let mut invalid = config();
        invalid.tenant_id = "tenant/a".into();
        assert!(matches!(
            MoqBroadcastPublisher::new(invalid),
            Err(MoqError::Namespace(_))
        ));
    }

    #[test]
    fn publisher_and_relay_public_types_are_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<MoqBroadcastPublisher>();
        assert_send_sync::<MoqRelayClient>();
    }

    #[test]
    fn construction_without_a_runtime_returns_an_explicit_error() {
        assert!(matches!(
            MoqBroadcastPublisher::new(config()),
            Err(MoqError::RuntimeUnavailable)
        ));
    }
}
