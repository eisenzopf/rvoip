//! One Orchestrator-facing SIP adapter backed by isolated egress profiles.
//!
//! SIP TLS client identity/trust, SRTP posture, and codec offers are
//! coordinator-wide today. This adapter keeps those settings isolated in one
//! child [`SipAdapter`] per immutable profile revision while presenting the
//! single `Transport::Sip` registration required by `rvoip-core`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, Weak};
use std::time::Duration;

use async_trait::async_trait;
use dashmap::{mapref::entry::Entry, DashMap};
use futures::future::join_all;
use rvoip_core::adapter::{
    legacy_normalized_event_receiver, AdapterEvent, AdapterKind, AdapterLifecycleCapabilities,
    AdapterLifecycleSink, AdapterLifecycleSinkSlot, ConnectionAdapter, ConnectionHandle, EndReason,
    InboundConnectionContext, OrchestratorAdapterEvent, OriginateRequest, OutboundActivation,
    PlaybackHandle, RejectReason, SignatureHeaders, TransferTarget,
};
use rvoip_core::capability::{
    CapabilityDescriptor, IdentityAssuranceRequirement, NegotiatedCodecs,
};
use rvoip_core::commands::{AudioSource, MuteDirection};
use rvoip_core::connection::Transport;
use rvoip_core::error::{Result as CoreResult, RvoipError};
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::{ConnectionId, TransferAttemptId};
use rvoip_core::message::Message;
use rvoip_core::stream::MediaStream;
use rvoip_core::DataMessage;
use thiserror::Error;
use tokio::sync::{mpsc, Mutex as AsyncMutex};

use crate::api::unified::Config;
use crate::{
    SipAdapter, SipNatConfig, SipOriginateContext, SipProfileRevision, UnifiedCoordinator,
};

/// Hard admission bound for independently configured SIP egress profiles.
pub const MAX_INSTALLED_SIP_EGRESS_PROFILES: usize = 64;
const PROFILED_SIP_EVENT_CAPACITY: usize = 256;

/// Coordinator-wide SRTP posture retained in a profile diagnostic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SipProfileSrtpPolicy {
    /// Do not offer or accept SDES-SRTP for this child.
    Disabled,
    /// Offer SDES-SRTP but permit a plaintext RTP answer.
    Preferred,
    /// Require successful SDES-SRTP negotiation.
    Required,
}

/// Non-secret description of the exact child configuration selected by one
/// opaque profile revision.
#[derive(Clone, Eq, PartialEq)]
pub struct SipEgressProfilePolicy {
    offered_codecs: Arc<[u8]>,
    srtp: SipProfileSrtpPolicy,
    tls_extra_roots: bool,
    tls_client_identity: bool,
    allowed_initial_headers: Arc<[String]>,
    sip_message: bool,
}

impl SipEgressProfilePolicy {
    fn from_config(
        config: &Config,
        allowed_initial_headers: impl IntoIterator<Item = String>,
        sip_message: bool,
    ) -> Result<Self, SipProfiledAdapterError> {
        let mut headers = allowed_initial_headers
            .into_iter()
            .map(|header| header.to_ascii_lowercase())
            .collect::<BTreeSet<_>>();
        if headers.len() > crate::MAX_SIP_INITIAL_HEADERS {
            return Err(SipProfiledAdapterError::InvalidPolicy);
        }
        for header in &headers {
            crate::SipInitialHeaders::new([(header.as_str(), "validation")])
                .map_err(|_| SipProfiledAdapterError::InvalidPolicy)?;
        }
        let srtp = match (config.offer_srtp, config.srtp_required) {
            (false, false) => SipProfileSrtpPolicy::Disabled,
            (true, false) => SipProfileSrtpPolicy::Preferred,
            (true, true) => SipProfileSrtpPolicy::Required,
            (false, true) => return Err(SipProfiledAdapterError::InvalidPolicy),
        };
        if config.offered_codecs.is_empty() {
            return Err(SipProfiledAdapterError::InvalidPolicy);
        }
        let tls_client_identity = match (
            config.tls_client_cert_path.is_some(),
            config.tls_client_key_path.is_some(),
        ) {
            (false, false) => false,
            (true, true) => true,
            _ => return Err(SipProfiledAdapterError::InvalidPolicy),
        };
        Ok(Self {
            offered_codecs: config.offered_codecs.clone().into(),
            srtp,
            tls_extra_roots: config.tls_extra_ca_path.is_some(),
            tls_client_identity,
            allowed_initial_headers: std::mem::take(&mut headers)
                .into_iter()
                .collect::<Vec<_>>()
                .into(),
            sip_message,
        })
    }

    /// Ordered RTP payload types installed on the child coordinator.
    pub fn offered_codecs(&self) -> &[u8] {
        &self.offered_codecs
    }

    /// Coordinator-wide SRTP posture for this exact child.
    pub const fn srtp(&self) -> SipProfileSrtpPolicy {
        self.srtp
    }

    /// Whether this child has a private TLS trust bundle.
    pub const fn has_tls_extra_roots(&self) -> bool {
        self.tls_extra_roots
    }

    /// Whether this child presents a TLS client certificate and key.
    pub const fn has_tls_client_identity(&self) -> bool {
        self.tls_client_identity
    }

    /// Lowercase initial-INVITE header names admitted by this profile.
    pub fn allowed_initial_headers(&self) -> &[String] {
        &self.allowed_initial_headers
    }

    /// Whether established routes may send transport-neutral data as SIP MESSAGE.
    pub const fn sip_message_allowed(&self) -> bool {
        self.sip_message
    }

    fn allows_initial_headers(&self, context: &SipOriginateContext) -> bool {
        context.initial_headers().iter().all(|(name, _)| {
            self.allowed_initial_headers
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(name.as_str()))
        })
    }
}

impl fmt::Debug for SipEgressProfilePolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SipEgressProfilePolicy")
            .field("offered_codecs", &self.offered_codecs)
            .field("srtp", &self.srtp)
            .field("tls_extra_roots", &self.tls_extra_roots)
            .field("tls_client_identity", &self.tls_client_identity)
            .field(
                "allowed_initial_header_count",
                &self.allowed_initial_headers.len(),
            )
            .field("sip_message", &self.sip_message)
            .finish()
    }
}

/// One prebuilt, independently secured outbound SIP child.
pub struct SipEgressProfileRegistration {
    revision: SipProfileRevision,
    policy: SipEgressProfilePolicy,
    adapter: Arc<SipAdapter>,
}

impl SipEgressProfileRegistration {
    /// Construct the child from the same `Config` used to derive its retained
    /// policy. Callers cannot pair a descriptor with a differently configured
    /// adapter.
    pub async fn from_config(
        revision: SipProfileRevision,
        config: Config,
        allowed_initial_headers: impl IntoIterator<Item = String>,
        sip_message: bool,
    ) -> Result<Self, SipProfiledAdapterError> {
        Self::from_config_and_nat(
            revision,
            config,
            SipNatConfig::default(),
            allowed_initial_headers,
            sip_message,
        )
        .await
    }

    /// Construct a child with an explicit RTP/NAT policy while deriving the
    /// retained profile policy from the same signaling/media config.
    pub async fn from_config_and_nat(
        revision: SipProfileRevision,
        config: Config,
        nat: SipNatConfig,
        allowed_initial_headers: impl IntoIterator<Item = String>,
        sip_message: bool,
    ) -> Result<Self, SipProfiledAdapterError> {
        let policy =
            SipEgressProfilePolicy::from_config(&config, allowed_initial_headers, sip_message)?;
        let coordinator = UnifiedCoordinator::new_with_nat(config, nat)
            .await
            .map_err(|_| SipProfiledAdapterError::ChildConstruction)?;
        let adapter = SipAdapter::new(coordinator)
            .await
            .map_err(|_| SipProfiledAdapterError::ChildConstruction)?;
        Ok(Self {
            revision,
            policy,
            adapter,
        })
    }

    /// Opaque immutable revision selecting this registration.
    pub fn revision(&self) -> &SipProfileRevision {
        &self.revision
    }

    /// Non-secret policy derived from the child coordinator's exact config.
    pub fn policy(&self) -> &SipEgressProfilePolicy {
        &self.policy
    }

    /// Stop a child that was constructed successfully but could not be
    /// installed into a composite adapter because a later startup step
    /// failed. This keeps partial profile startup from leaking signaling or
    /// media tasks.
    pub async fn shutdown(self, timeout: Duration) -> CoreResult<()> {
        let drain = self.adapter.drain().await;
        let shutdown = self
            .adapter
            .coordinator()
            .shutdown_gracefully(Some(timeout))
            .await
            .map_err(|_| RvoipError::Adapter("SIP profile shutdown failed".into()));
        drain.and(shutdown)
    }
}

/// Fixed, payload-free construction failures for the composite adapter.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum SipProfiledAdapterError {
    /// The bounded installed-profile limit was exceeded.
    #[error("too many SIP egress profiles are installed")]
    TooManyProfiles,
    /// Two child registrations used the same opaque revision.
    #[error("a SIP egress profile revision is installed more than once")]
    DuplicateProfile,
    /// A child config or metadata policy was internally inconsistent.
    #[error("the SIP egress profile policy is invalid")]
    InvalidPolicy,
    /// Building an isolated SIP child coordinator failed.
    #[error("a SIP egress child could not be constructed")]
    ChildConstruction,
    /// A child receiver or lifecycle fallback was already consumed elsewhere.
    #[error("a SIP child event stream or lifecycle sink is already owned")]
    ChildAlreadyOwned,
}

struct ProfileChild {
    generation: u64,
    adapter: Arc<SipAdapter>,
    policy: Option<SipEgressProfilePolicy>,
    allow_inbound: bool,
}

#[derive(Clone)]
struct ConnectionOwner {
    generation: u64,
    child: Arc<ProfileChild>,
}

/// Composite SIP adapter registered once with an Orchestrator.
pub struct ProfiledSipAdapter {
    default: Arc<ProfileChild>,
    profiles: BTreeMap<SipProfileRevision, Arc<ProfileChild>>,
    children: Vec<Arc<ProfileChild>>,
    owners: DashMap<ConnectionId, ConnectionOwner>,
    events_tx: mpsc::Sender<OrchestratorAdapterEvent>,
    events_rx: StdMutex<Option<mpsc::Receiver<OrchestratorAdapterEvent>>>,
    lifecycle: AdapterLifecycleSinkSlot,
    draining: AtomicBool,
    drain_gate: AsyncMutex<()>,
}

impl ProfiledSipAdapter {
    /// Build one composite around the inbound/legacy child and zero or more
    /// independently configured outbound children.
    pub fn new(
        default: Arc<SipAdapter>,
        registrations: impl IntoIterator<Item = SipEgressProfileRegistration>,
    ) -> Result<Arc<Self>, SipProfiledAdapterError> {
        let registrations = registrations.into_iter().collect::<Vec<_>>();
        if registrations.len() > MAX_INSTALLED_SIP_EGRESS_PROFILES {
            return Err(SipProfiledAdapterError::TooManyProfiles);
        }

        let default = Arc::new(ProfileChild {
            generation: 1,
            adapter: default,
            policy: None,
            allow_inbound: true,
        });
        let mut profiles = BTreeMap::new();
        let mut children = vec![Arc::clone(&default)];
        for (offset, registration) in registrations.into_iter().enumerate() {
            let child = Arc::new(ProfileChild {
                generation: offset as u64 + 2,
                adapter: registration.adapter,
                policy: Some(registration.policy),
                allow_inbound: false,
            });
            if profiles
                .insert(registration.revision, Arc::clone(&child))
                .is_some()
            {
                return Err(SipProfiledAdapterError::DuplicateProfile);
            }
            children.push(child);
        }

        let (events_tx, events_rx) = mpsc::channel(PROFILED_SIP_EVENT_CAPACITY);
        let adapter = Arc::new(Self {
            default,
            profiles,
            children,
            owners: DashMap::new(),
            events_tx,
            events_rx: StdMutex::new(Some(events_rx)),
            lifecycle: AdapterLifecycleSinkSlot::default(),
            draining: AtomicBool::new(false),
            drain_gate: AsyncMutex::new(()),
        });

        for child in &adapter.children {
            child
                .adapter
                .install_lifecycle_sink(Arc::new(ProfileChildLifecycleSink {
                    pool: Arc::downgrade(&adapter),
                    generation: child.generation,
                }))
                .map_err(|_| SipProfiledAdapterError::ChildAlreadyOwned)?;
            let receiver = child
                .adapter
                .try_subscribe_atomic_events()
                .map_err(|_| SipProfiledAdapterError::ChildAlreadyOwned)?;
            tokio::spawn(run_child_events(
                Arc::downgrade(&adapter),
                Arc::clone(child),
                receiver,
            ));
        }
        Ok(adapter)
    }

    /// Source-compatible single-child wrapper. No profile key is required.
    pub fn single(default: Arc<SipAdapter>) -> Result<Arc<Self>, SipProfiledAdapterError> {
        Self::new(default, std::iter::empty())
    }

    /// Inspect the non-secret policy for one installed exact revision.
    pub fn profile_policy(&self, revision: &SipProfileRevision) -> Option<&SipEgressProfilePolicy> {
        self.profiles
            .get(revision)
            .and_then(|child| child.policy.as_ref())
    }

    fn select_outbound_child(&self, request: &OriginateRequest) -> CoreResult<Arc<ProfileChild>> {
        if self.profiles.is_empty() {
            return Ok(Arc::clone(&self.default));
        }
        let context = request
            .context
            .downcast_arc::<SipOriginateContext>()
            .ok_or(RvoipError::AdmissionRejected(
                "profiled SIP origination requires a typed context",
            ))?;
        let revision = context
            .profile_revision()
            .ok_or(RvoipError::AdmissionRejected(
                "profiled SIP origination requires an installed profile revision",
            ))?;
        let child = self
            .profiles
            .get(revision)
            .cloned()
            .ok_or(RvoipError::AdmissionRejected(
                "SIP egress profile revision is not installed",
            ))?;
        let policy = child.policy.as_ref().ok_or(RvoipError::InvalidState(
            "profiled SIP child has no retained policy",
        ))?;
        if !policy.allows_initial_headers(&context) {
            return Err(RvoipError::AdmissionRejected(
                "SIP initial headers are forbidden by the selected profile",
            ));
        }
        Ok(child)
    }

    fn owner(&self, connection_id: &ConnectionId) -> CoreResult<ConnectionOwner> {
        self.owners
            .get(connection_id)
            .map(|owner| owner.clone())
            .ok_or_else(|| RvoipError::ConnectionNotFound(connection_id.clone()))
    }

    fn claim_owner(&self, connection_id: ConnectionId, child: Arc<ProfileChild>) -> CoreResult<()> {
        match self.owners.entry(connection_id) {
            Entry::Vacant(entry) => {
                entry.insert(ConnectionOwner {
                    generation: child.generation,
                    child,
                });
                Ok(())
            }
            Entry::Occupied(entry) if entry.get().generation == child.generation => Ok(()),
            Entry::Occupied(_) => Err(RvoipError::AdmissionRejected(
                "SIP connection identifier is already owned by another profile generation",
            )),
        }
    }

    fn remove_owner_exact(&self, connection_id: &ConnectionId, generation: u64) {
        if let Entry::Occupied(entry) = self.owners.entry(connection_id.clone()) {
            if entry.get().generation == generation {
                entry.remove();
            }
        }
    }

    /// Stop admission, drain every child concurrently, then stop every child
    /// coordinator. The operation is one-way and idempotent.
    pub async fn drain(&self, timeout: Duration) -> CoreResult<()> {
        let _gate = self.drain_gate.lock().await;
        if self.draining.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let drains = self.children.iter().map(|child| {
            let child = Arc::clone(child);
            async move {
                let drain = child.adapter.drain().await;
                let shutdown = child
                    .adapter
                    .coordinator()
                    .shutdown_gracefully(Some(timeout))
                    .await
                    .map_err(|_| RvoipError::Adapter("SIP child shutdown failed".into()));
                drain.and(shutdown)
            }
        });
        let mut failure = None;
        for result in join_all(drains).await {
            if let Err(error) = result {
                failure.get_or_insert(error);
            }
        }
        self.owners.clear();
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

struct ProfileChildLifecycleSink {
    pool: Weak<ProfiledSipAdapter>,
    generation: u64,
}

#[async_trait]
impl AdapterLifecycleSink for ProfileChildLifecycleSink {
    async fn deliver_terminal(&self, event: AdapterEvent) {
        let Some(pool) = self.pool.upgrade() else {
            return;
        };
        let Some(connection_id) = adapter_event_connection_id(&event).cloned() else {
            return;
        };
        let owned = pool
            .owners
            .get(&connection_id)
            .is_some_and(|owner| owner.generation == self.generation);
        if !owned {
            return;
        }
        pool.lifecycle.deliver_terminal(event).await;
        pool.remove_owner_exact(&connection_id, self.generation);
    }
}

async fn run_child_events(
    pool: Weak<ProfiledSipAdapter>,
    child: Arc<ProfileChild>,
    mut receiver: mpsc::Receiver<OrchestratorAdapterEvent>,
) {
    while let Some(event) = receiver.recv().await {
        let Some(pool) = pool.upgrade() else {
            return;
        };
        let connection_id = orchestrator_event_connection_id(&event).cloned();
        let inbound = is_inbound_event(&event);
        if inbound {
            let Some(connection_id) = connection_id.as_ref() else {
                continue;
            };
            if !child.allow_inbound {
                let _ = child
                    .adapter
                    .reject(connection_id.clone(), RejectReason::Forbidden)
                    .await;
                continue;
            }
            if pool
                .claim_owner(connection_id.clone(), Arc::clone(&child))
                .is_err()
            {
                let _ = child
                    .adapter
                    .reject(connection_id.clone(), RejectReason::ServerError)
                    .await;
                continue;
            }
        } else if let Some(connection_id) = connection_id.as_ref() {
            let owned = pool
                .owners
                .get(connection_id)
                .is_some_and(|owner| owner.generation == child.generation);
            if !owned {
                continue;
            }
        }

        let terminal = terminal_event(&event);
        let delivered = if let Some(terminal) = terminal.clone() {
            !matches!(
                pool.lifecycle
                    .queue_or_deliver_orchestrator_terminal(&pool.events_tx, terminal)
                    .await,
                rvoip_core::adapter::TerminalDelivery::Undeliverable
            )
        } else {
            pool.events_tx.send(event).await.is_ok()
        };
        if !delivered {
            if let Some(connection_id) = connection_id.as_ref() {
                let _ = child
                    .adapter
                    .end(
                        connection_id.clone(),
                        EndReason::Failed {
                            detail: "profiled SIP event delivery failed".into(),
                        },
                    )
                    .await;
                pool.remove_owner_exact(connection_id, child.generation);
            }
            return;
        }
        if terminal.is_some() {
            if let Some(connection_id) = connection_id.as_ref() {
                pool.remove_owner_exact(connection_id, child.generation);
            }
        }
    }
}

fn is_inbound_event(event: &OrchestratorAdapterEvent) -> bool {
    matches!(
        event,
        OrchestratorAdapterEvent::AuthenticatedInboundConnection { .. }
            | OrchestratorAdapterEvent::Public(AdapterEvent::InboundConnection { .. })
    )
}

fn terminal_event(event: &OrchestratorAdapterEvent) -> Option<AdapterEvent> {
    match event {
        OrchestratorAdapterEvent::Public(
            event @ (AdapterEvent::Ended { .. } | AdapterEvent::Failed { .. }),
        ) => Some(event.clone()),
        _ => None,
    }
}

fn orchestrator_event_connection_id(event: &OrchestratorAdapterEvent) -> Option<&ConnectionId> {
    match event {
        OrchestratorAdapterEvent::AuthenticatedInboundConnection { connection, .. } => {
            Some(&connection.id)
        }
        OrchestratorAdapterEvent::Public(event) => adapter_event_connection_id(event),
        _ => None,
    }
}

fn adapter_event_connection_id(event: &AdapterEvent) -> Option<&ConnectionId> {
    match event {
        AdapterEvent::InboundConnection { connection } => Some(&connection.id),
        AdapterEvent::Connected { connection_id }
        | AdapterEvent::Progress { connection_id, .. }
        | AdapterEvent::Authenticated { connection_id, .. }
        | AdapterEvent::PrincipalAuthenticated { connection_id, .. }
        | AdapterEvent::Ended { connection_id, .. }
        | AdapterEvent::Failed { connection_id, .. }
        | AdapterEvent::Dtmf { connection_id, .. }
        | AdapterEvent::Quality { connection_id, .. }
        | AdapterEvent::Message { connection_id, .. }
        | AdapterEvent::DataMessage { connection_id, .. }
        | AdapterEvent::TransferStatus { connection_id, .. }
        | AdapterEvent::StepUpResponse { connection_id, .. } => Some(connection_id),
        AdapterEvent::Native { .. } => None,
        _ => None,
    }
}

#[async_trait]
impl ConnectionAdapter for ProfiledSipAdapter {
    fn transport(&self) -> Transport {
        Transport::Sip
    }

    fn kind(&self) -> AdapterKind {
        AdapterKind::Interop
    }

    fn lifecycle_capabilities(&self) -> AdapterLifecycleCapabilities {
        AdapterLifecycleCapabilities {
            authoritative_liveness: true,
            atomic_inbound_handoff: true,
            terminal_fallback: true,
            staged_outbound_activation: true,
        }
    }

    fn supports_inbound_admission_confirmation(&self) -> bool {
        self.default
            .adapter
            .supports_inbound_admission_confirmation()
    }

    fn notify_inbound_admission_outcome(
        &self,
        connection_id: &ConnectionId,
        lifecycle_generation: u64,
        accepted: bool,
    ) {
        if let Ok(owner) = self.owner(connection_id) {
            owner.adapter().notify_inbound_admission_outcome(
                connection_id,
                lifecycle_generation,
                accepted,
            );
        }
    }

    fn install_lifecycle_sink(&self, sink: Arc<dyn AdapterLifecycleSink>) -> CoreResult<()> {
        self.lifecycle
            .install(sink)
            .map_err(|_| RvoipError::InvalidState("profiled SIP lifecycle sink already installed"))
    }

    fn is_connection_live(&self, connection_id: &ConnectionId) -> bool {
        self.owner(connection_id)
            .is_ok_and(|owner| owner.adapter().is_connection_live(connection_id))
    }

    fn take_inbound_context(
        &self,
        connection_id: &ConnectionId,
    ) -> Option<InboundConnectionContext> {
        self.owner(connection_id)
            .ok()
            .and_then(|owner| owner.adapter().take_inbound_context(connection_id))
    }

    async fn originate(&self, request: OriginateRequest) -> CoreResult<ConnectionHandle> {
        if self.draining.load(Ordering::Acquire) {
            return Err(RvoipError::InvalidState("profiled SIP adapter is draining"));
        }
        let child = self.select_outbound_child(&request)?;
        let handle = child.adapter.originate(request).await?;
        let connection_id = handle.connection.id.clone();
        if let Err(error) = self.claim_owner(connection_id.clone(), Arc::clone(&child)) {
            let _ = child
                .adapter
                .end(
                    connection_id,
                    EndReason::Failed {
                        detail: "profiled SIP owner claim failed".into(),
                    },
                )
                .await;
            return Err(error);
        }
        Ok(handle)
    }

    async fn activate_outbound(&self, connection_id: ConnectionId) -> CoreResult<()> {
        self.owner(&connection_id)?
            .adapter()
            .activate_outbound(connection_id)
            .await
    }

    async fn activate_outbound_with_receipt(
        &self,
        connection_id: ConnectionId,
    ) -> CoreResult<OutboundActivation> {
        self.owner(&connection_id)?
            .adapter()
            .activate_outbound_with_receipt(connection_id)
            .await
    }

    async fn start_inbound_early_media(&self, connection_id: ConnectionId) -> CoreResult<()> {
        self.owner(&connection_id)?
            .adapter()
            .start_inbound_early_media(connection_id)
            .await
    }

    async fn accept(&self, connection_id: ConnectionId) -> CoreResult<()> {
        self.owner(&connection_id)?
            .adapter()
            .accept(connection_id)
            .await
    }

    async fn reject(&self, connection_id: ConnectionId, reason: RejectReason) -> CoreResult<()> {
        self.owner(&connection_id)?
            .adapter()
            .reject(connection_id, reason)
            .await
    }

    async fn end(&self, connection_id: ConnectionId, reason: EndReason) -> CoreResult<()> {
        self.owner(&connection_id)?
            .adapter()
            .end(connection_id, reason)
            .await
    }

    async fn hold(&self, connection_id: ConnectionId) -> CoreResult<()> {
        self.owner(&connection_id)?
            .adapter()
            .hold(connection_id)
            .await
    }

    async fn resume(&self, connection_id: ConnectionId) -> CoreResult<()> {
        self.owner(&connection_id)?
            .adapter()
            .resume(connection_id)
            .await
    }

    async fn transfer(
        &self,
        connection_id: ConnectionId,
        target: TransferTarget,
    ) -> CoreResult<()> {
        self.owner(&connection_id)?
            .adapter()
            .transfer(connection_id, target)
            .await
    }

    async fn transfer_with_attempt(
        &self,
        connection_id: ConnectionId,
        attempt_id: TransferAttemptId,
        target: TransferTarget,
    ) -> CoreResult<()> {
        self.owner(&connection_id)?
            .adapter()
            .transfer_with_attempt(connection_id, attempt_id, target)
            .await
    }

    async fn streams(&self, connection_id: ConnectionId) -> CoreResult<Vec<Arc<dyn MediaStream>>> {
        self.owner(&connection_id)?
            .adapter()
            .streams(connection_id)
            .await
    }

    async fn send_message(&self, connection_id: ConnectionId, message: Message) -> CoreResult<()> {
        self.owner(&connection_id)?
            .adapter()
            .send_message(connection_id, message)
            .await
    }

    async fn send_data_message(
        &self,
        connection_id: ConnectionId,
        message: DataMessage,
    ) -> CoreResult<()> {
        let owner = self.owner(&connection_id)?;
        if owner
            .child
            .policy
            .as_ref()
            .is_some_and(|policy| !policy.sip_message_allowed())
        {
            return Err(RvoipError::AdmissionRejected(
                "SIP MESSAGE is disabled by the selected profile",
            ));
        }
        owner
            .adapter()
            .send_data_message(connection_id, message)
            .await
    }

    async fn send_dtmf(
        &self,
        connection_id: ConnectionId,
        digits: &str,
        duration_ms: u32,
    ) -> CoreResult<()> {
        self.owner(&connection_id)?
            .adapter()
            .send_dtmf(connection_id, digits, duration_ms)
            .await
    }

    async fn renegotiate_media(
        &self,
        connection_id: ConnectionId,
        capabilities: CapabilityDescriptor,
    ) -> CoreResult<NegotiatedCodecs> {
        self.owner(&connection_id)?
            .adapter()
            .renegotiate_media(connection_id, capabilities)
            .await
    }

    async fn mute(&self, connection_id: ConnectionId, direction: MuteDirection) -> CoreResult<()> {
        self.owner(&connection_id)?
            .adapter()
            .mute(connection_id, direction)
            .await
    }

    async fn unmute(
        &self,
        connection_id: ConnectionId,
        direction: MuteDirection,
    ) -> CoreResult<()> {
        self.owner(&connection_id)?
            .adapter()
            .unmute(connection_id, direction)
            .await
    }

    async fn play_audio(
        &self,
        connection_id: ConnectionId,
        source: AudioSource,
    ) -> CoreResult<PlaybackHandle> {
        self.owner(&connection_id)?
            .adapter()
            .play_audio(connection_id, source)
            .await
    }

    async fn send_step_up_request(
        &self,
        connection_id: ConnectionId,
        required: IdentityAssuranceRequirement,
        allowed_methods: Vec<String>,
        reason: Option<String>,
    ) -> CoreResult<()> {
        self.owner(&connection_id)?
            .adapter()
            .send_step_up_request(connection_id, required, allowed_methods, reason)
            .await
    }

    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent> {
        legacy_normalized_event_receiver(
            self.subscribe_orchestrator_events(),
            PROFILED_SIP_EVENT_CAPACITY,
        )
    }

    fn subscribe_orchestrator_events(&self) -> mpsc::Receiver<OrchestratorAdapterEvent> {
        self.events_rx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .expect("ProfiledSipAdapter event stream already consumed")
    }

    fn capabilities(&self) -> CapabilityDescriptor {
        self.default.adapter.capabilities()
    }

    async fn verify_request_signature(
        &self,
        connection_id: ConnectionId,
        signature: SignatureHeaders,
    ) -> CoreResult<IdentityAssurance> {
        self.owner(&connection_id)?
            .adapter()
            .verify_request_signature(connection_id, signature)
            .await
    }
}

impl ConnectionOwner {
    fn adapter(&self) -> &SipAdapter {
        &self.child.adapter
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_core::connection::Direction;
    use rvoip_core::ids::{ParticipantId, SessionId};

    fn revision(value: char) -> SipProfileRevision {
        SipProfileRevision::new(value.to_string().repeat(64)).expect("revision")
    }

    #[test]
    fn policy_snapshot_keeps_conflicting_transport_security_isolated() {
        let mut preferred = Config::local("preferred", 0);
        preferred.offer_srtp = true;
        preferred.srtp_required = false;
        preferred.offered_codecs = vec![0, 101];
        preferred.tls_extra_ca_path = Some("/private/ca-a.pem".into());

        let mut required = Config::local("required", 0);
        required.offer_srtp = true;
        required.srtp_required = true;
        required.offered_codecs = vec![8, 111, 101];
        required.tls_client_cert_path = Some("/private/client-b.pem".into());
        required.tls_client_key_path = Some("/private/client-b-key.pem".into());

        let left =
            SipEgressProfilePolicy::from_config(&preferred, ["X-Correlation-Id".into()], false)
                .expect("preferred policy");
        let right = SipEgressProfilePolicy::from_config(&required, ["X-Account-Tier".into()], true)
            .expect("required policy");

        assert_eq!(left.srtp(), SipProfileSrtpPolicy::Preferred);
        assert_eq!(right.srtp(), SipProfileSrtpPolicy::Required);
        assert_eq!(left.offered_codecs(), &[0, 101]);
        assert_eq!(right.offered_codecs(), &[8, 111, 101]);
        assert!(left.has_tls_extra_roots());
        assert!(!left.has_tls_client_identity());
        assert!(!right.has_tls_extra_roots());
        assert!(right.has_tls_client_identity());
        assert!(!left.sip_message_allowed());
        assert!(right.sip_message_allowed());
    }

    #[tokio::test]
    async fn explicit_nat_policy_is_validated_by_profile_child_construction() {
        let mut nat = SipNatConfig::default();
        nat.symmetric_rtp.probation_packets = 0;
        let error = match SipEgressProfileRegistration::from_config_and_nat(
            revision('n'),
            Config::local("invalid-nat", 0),
            nat,
            std::iter::empty(),
            false,
        )
        .await
        {
            Ok(_) => panic!("invalid per-profile NAT policy must fail closed"),
            Err(error) => error,
        };
        assert_eq!(error, SipProfiledAdapterError::ChildConstruction);
    }

    #[tokio::test]
    async fn exact_revision_selects_child_and_denies_sip_message_by_profile() {
        let default = SipAdapter::from_config(Config::local("default", 0))
            .await
            .expect("default adapter");

        let mut first_config = Config::local("first", 0);
        first_config.offer_srtp = true;
        first_config.offered_codecs = vec![0, 101];
        let first_revision = revision('a');
        let first = SipEgressProfileRegistration::from_config(
            first_revision.clone(),
            first_config,
            ["X-Correlation-Id".into()],
            false,
        )
        .await
        .expect("first profile");

        let mut second_config = Config::local("second", 0);
        second_config.offer_srtp = true;
        second_config.srtp_required = true;
        second_config.offered_codecs = vec![8, 101];
        let second_revision = revision('b');
        let second = SipEgressProfileRegistration::from_config(
            second_revision.clone(),
            second_config,
            ["X-Account-Tier".into()],
            true,
        )
        .await
        .expect("second profile");

        let pool = ProfiledSipAdapter::new(default, [first, second]).expect("profiled adapter");
        let headers =
            crate::SipInitialHeaders::new([("X-Correlation-Id", "corr-1")]).expect("headers");
        let context = SipOriginateContext::new()
            .with_profile_revision(first_revision)
            .with_initial_headers(headers);
        let handle = pool
            .originate(
                OriginateRequest::new(
                    SessionId::new(),
                    ParticipantId::new(),
                    "sip:agent@example.test",
                    Direction::Outbound,
                    CapabilityDescriptor::default(),
                )
                .with_context(context),
            )
            .await
            .expect("prepared first route");
        let connection_id = handle.connection.id;
        assert!(matches!(
            pool.send_data_message(
                connection_id.clone(),
                DataMessage::reliable("context", "application/json", b"{}".as_slice()),
            )
            .await,
            Err(RvoipError::AdmissionRejected(_))
        ));

        pool.end(connection_id, EndReason::Normal)
            .await
            .expect("end prepared route");
        pool.drain(Duration::from_secs(2))
            .await
            .expect("drain every child");

        assert_eq!(
            pool.profile_policy(&second_revision)
                .expect("second policy")
                .srtp(),
            SipProfileSrtpPolicy::Required
        );
    }
}
