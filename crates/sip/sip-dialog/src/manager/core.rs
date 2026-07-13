//! Core Dialog Manager Implementation
//!
//! This module contains the main DialogManager struct and its core lifecycle methods.
//! It serves as the central coordinator for SIP dialog management.

use dashmap::DashMap;
use std::collections::{hash_map::DefaultHasher, HashSet};
use std::hash::{Hash, Hasher};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{debug, error, info, trace, warn};

use crate::transaction::transport::multiplexed::select_transport_for_uri;
use crate::transaction::{TransactionEvent, TransactionKey, TransactionManager, TransactionState};
use rvoip_infra_common::events::cross_crate::SipTransportContext;
use rvoip_sip_core::{Method, Request, Response, Uri};
use rvoip_sip_transport::transport::TransportType;

use crate::config::DialogManagerConfig;
use crate::diagnostics::safe_log::method_class;
use crate::dialog::{Dialog, DialogId, DialogState};
use crate::errors::{DialogError, DialogResult};
use crate::events::{DialogEvent, FlowFailureReason, SessionCoordinationEvent};
use crate::manager::outbound_flow::OutboundFlow;
use crate::manager::utils::DialogUtils;
use crate::subscription::SubscriptionManager;

// STIR/SHAKEN — the SIP-agnostic enum lives in infra-common so the
// cross-crate bus stays free of rvoip types; alias it here for
// readability at the publish-decision call site.
use rvoip_infra_common::events::cross_crate::IdentityVerificationStatus;

/// Outcome of [`DialogManager::run_identity_verification`].
///
/// - `Publish(Some(status))` — verifier ran and produced an outcome
///   the policy did not reject. Attach `status` to the published
///   cross-crate event.
/// - `Publish(None)` — no verifier installed (or event was not an
///   `IncomingCall`). Publish without an identity-verification
///   field.
/// - `Drop` — policy rejected the outcome and the matching RFC 8224
///   §6.2.2 4xx response has already been sent through the
///   transaction layer. Caller must drop the event so it never
///   reaches session-core.
#[derive(Debug)]
pub enum IdentityVerificationDecision {
    Publish(Option<IdentityVerificationStatus>),
    Drop,
}

const TERMINATED_BYE_TOMBSTONE_TTL: Duration = Duration::from_secs(32);
const MIN_TERMINATED_BYE_LOOKUP_HARD_MAX: usize = 65_536;
const TERMINATED_BYE_LOOKUP_HARD_MAX_MULTIPLIER: usize = 16;
const TERMINATED_BYE_PRUNE_INTERVAL: usize = 8_192;
const MIN_DIALOG_INDEX_CAPACITY: usize = 1024;
const DEFAULT_DIALOG_EVENT_DISPATCH_WORKERS: usize = 1;
const MIN_INVITE_FAILOVER_ATTEMPT_CAPACITY: usize = 65_536;
const INVITE_FAILOVER_ATTEMPT_CAPACITY_MULTIPLIER: usize = 16;

/// Retained dialog-manager state counts used by release-gate leak checks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DialogManagerRetentionCounts {
    pub dialogs: usize,
    pub dialog_lookup: usize,
    pub early_dialog_lookup: usize,
    pub terminated_bye_lookup: usize,
    pub transaction_to_dialog: usize,
    pub transaction_dialog_route_hash: usize,
    pub dialog_invite_transactions: usize,
    pub invite_failover_plans: usize,
    pub active_invite_failover_by_dialog: usize,
    pub invite_failover_attempts: usize,
    pub invite_failover_plan_reservations: usize,
    pub invite_failover_attempt_reservations: usize,
    pub dialog_server_transactions: usize,
    pub pending_response_transaction_by_dialog: usize,
    pub session_to_dialog: usize,
    pub dialog_to_session: usize,
    pub reliable_provisional_tasks: usize,
    pub session_refresh_tasks: usize,
    pub outbound_flows: usize,
    pub outbound_flow_tasks: usize,
    pub flow_by_destination: usize,
    pub flow_by_aor: usize,
}

fn dialog_index_capacity(max_dialogs: Option<usize>) -> usize {
    max_dialogs.unwrap_or(10_000).max(MIN_DIALOG_INDEX_CAPACITY)
}

fn terminated_bye_lookup_hard_max(index_capacity: usize) -> usize {
    index_capacity
        .saturating_mul(TERMINATED_BYE_LOOKUP_HARD_MAX_MULTIPLIER)
        .max(MIN_TERMINATED_BYE_LOOKUP_HARD_MAX)
}

fn invite_failover_attempt_capacity(index_capacity: usize) -> usize {
    index_capacity
        .saturating_mul(INVITE_FAILOVER_ATTEMPT_CAPACITY_MULTIPLIER)
        .max(MIN_INVITE_FAILOVER_ATTEMPT_CAPACITY)
}

pub(crate) fn outbound_request_key(request: &Request) -> Option<String> {
    let call_id = request.call_id()?.value();
    let cseq = request.cseq()?;
    Some(format!("{}:{}:{}", call_id, cseq.method, cseq.sequence()))
}

fn outbound_response_key(response: &Response) -> Option<String> {
    let call_id = response.call_id()?.value();
    let cseq = response.cseq()?;
    Some(format!("{}:{}:{}", call_id, cseq.method, cseq.sequence()))
}

#[derive(Debug, Clone, Copy)]
enum DialogTransactionEventKind {
    Invite,
    Ack,
    Bye,
    Cancel,
    Other,
}

impl DialogTransactionEventKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Invite => "invite",
            Self::Ack => "ack",
            Self::Bye => "bye",
            Self::Cancel => "cancel",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DialogRouteSource {
    Request,
    Stored,
    TransactionKey,
    Fallback,
}

impl DialogRouteSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Stored => "stored",
            Self::TransactionKey => "transaction_key",
            Self::Fallback => "fallback",
        }
    }
}

struct QueuedDialogTransactionEvent {
    event: TransactionEvent,
    kind: Option<DialogTransactionEventKind>,
    queued_at: Option<Instant>,
}

fn request_dialog_route_hash(request: &Request) -> Option<u64> {
    let call_id = request.call_id()?;
    let from_tag = request.from_tag()?;
    let mut hasher = DefaultHasher::new();
    call_id.value().hash(&mut hasher);
    from_tag.hash(&mut hasher);
    Some(hasher.finish())
}

fn transaction_key_route_hash(transaction_id: &TransactionKey) -> u64 {
    let mut hasher = DefaultHasher::new();
    transaction_id.hash(&mut hasher);
    hasher.finish()
}

fn transaction_event_key(event: &TransactionEvent) -> Option<&TransactionKey> {
    match event {
        TransactionEvent::AckReceived { transaction_id, .. }
        | TransactionEvent::CancelReceived { transaction_id, .. }
        | TransactionEvent::ProvisionalResponse { transaction_id, .. }
        | TransactionEvent::SuccessResponse { transaction_id, .. }
        | TransactionEvent::FailureResponse { transaction_id, .. }
        | TransactionEvent::ProvisionalResponseSent { transaction_id, .. }
        | TransactionEvent::FinalResponseSent { transaction_id, .. }
        | TransactionEvent::TransactionTimeout { transaction_id }
        | TransactionEvent::AckTimeout { transaction_id }
        | TransactionEvent::TransportError { transaction_id }
        | TransactionEvent::TransactionTerminated { transaction_id }
        | TransactionEvent::StateChanged { transaction_id, .. }
        | TransactionEvent::TimerTriggered { transaction_id, .. }
        | TransactionEvent::CancelRequest { transaction_id, .. }
        | TransactionEvent::AckRequest { transaction_id, .. }
        | TransactionEvent::InviteRequest { transaction_id, .. }
        | TransactionEvent::NonInviteRequest { transaction_id, .. } => Some(transaction_id),
        TransactionEvent::Error {
            transaction_id: Some(transaction_id),
            ..
        } => Some(transaction_id),
        _ => None,
    }
}

fn transaction_event_request(event: &TransactionEvent) -> Option<&Request> {
    match event {
        TransactionEvent::AckReceived { request, .. }
        | TransactionEvent::CancelReceived {
            cancel_request: request,
            ..
        }
        | TransactionEvent::CancelRequest { request, .. }
        | TransactionEvent::AckRequest { request, .. }
        | TransactionEvent::InviteRequest { request, .. }
        | TransactionEvent::NonInviteRequest { request, .. }
        | TransactionEvent::StrayRequest { request, .. }
        | TransactionEvent::StrayAck { request, .. }
        | TransactionEvent::StrayCancel { request, .. }
        | TransactionEvent::StrayAckRequest { request, .. } => Some(request),
        _ => None,
    }
}

fn dialog_event_request_route_hash(event: &TransactionEvent) -> Option<u64> {
    if let Some(request) = transaction_event_request(event) {
        if let Some(hash) = request_dialog_route_hash(request) {
            return Some(hash);
        }
    }

    None
}

fn dialog_event_kind(event: &TransactionEvent) -> DialogTransactionEventKind {
    if let Some(request) = transaction_event_request(event) {
        return match request.method() {
            Method::Invite => DialogTransactionEventKind::Invite,
            Method::Ack => DialogTransactionEventKind::Ack,
            Method::Bye => DialogTransactionEventKind::Bye,
            Method::Cancel => DialogTransactionEventKind::Cancel,
            _ => DialogTransactionEventKind::Other,
        };
    }

    match event {
        TransactionEvent::AckReceived { .. } | TransactionEvent::AckRequest { .. } => {
            DialogTransactionEventKind::Ack
        }
        TransactionEvent::CancelReceived { .. } | TransactionEvent::CancelRequest { .. } => {
            DialogTransactionEventKind::Cancel
        }
        TransactionEvent::InviteRequest { .. } => DialogTransactionEventKind::Invite,
        _ => DialogTransactionEventKind::Other,
    }
}

fn dialog_event_route_kind(event: &TransactionEvent) -> &'static str {
    match event {
        TransactionEvent::StateChanged { .. } | TransactionEvent::TransactionTerminated { .. } => {
            "lifecycle"
        }
        _ => dialog_event_kind(event).as_str(),
    }
}

fn session_coordination_event_kind(event: &SessionCoordinationEvent) -> &'static str {
    match event {
        SessionCoordinationEvent::IncomingCall { .. } => "incoming_call",
        SessionCoordinationEvent::AckReceived { .. } => "ack_received",
        SessionCoordinationEvent::ByeReceived { .. } => "bye_received",
        _ => "other",
    }
}

fn verification_outcome_class(outcome: &crate::manager::VerificationOutcome) -> &'static str {
    match outcome {
        crate::manager::VerificationOutcome::Valid { .. } => "valid",
        crate::manager::VerificationOutcome::Stale { .. } => "stale",
        crate::manager::VerificationOutcome::BadSignature => "bad_signature",
        crate::manager::VerificationOutcome::BadChain { .. } => "bad_chain",
        crate::manager::VerificationOutcome::ClaimMismatch { .. } => "claim_mismatch",
        crate::manager::VerificationOutcome::BadInfo { .. } => "bad_info",
        crate::manager::VerificationOutcome::NoIdentity => "no_identity",
    }
}

#[cfg(test)]
mod verification_diagnostic_tests {
    use super::verification_outcome_class;
    use crate::manager::VerificationOutcome;

    #[test]
    fn verification_outcome_class_does_not_reflect_reason() {
        const SECRET: &str = "verification-secret-canary\r\nX-Leak: yes";
        for outcome in [
            VerificationOutcome::BadChain {
                reason: SECRET.to_string(),
            },
            VerificationOutcome::BadInfo {
                reason: SECRET.to_string(),
            },
        ] {
            let class = verification_outcome_class(&outcome);
            assert!(matches!(class, "bad_chain" | "bad_info"));
            assert!(!class.contains(SECRET));
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TerminatedByeTombstone {
    pub(crate) cseq: u32,
    created_at: Instant,
}

#[derive(Clone)]
pub struct DialogManager {
    /// Reference to transaction manager (handles transport for us)
    pub(crate) transaction_manager: Arc<TransactionManager>,

    /// Local bind address for this dialog manager. Via sent-by uses this
    /// only when no advertised address is configured.
    pub(crate) local_address: SocketAddr,

    /// **NEW**: Optional unified configuration for behavioral modes
    /// When present, enables mode-specific behavior (auto-responses, etc.).
    ///
    /// Wrapped in `Arc<RwLock<...>>` so that `set_config` propagates to every
    /// `DialogManager` clone — notably the background event-processor task
    /// spawned during construction, which otherwise would never see the
    /// config set later by `UnifiedDialogManager` (RFC 3262 420 + RFC 4028
    /// negotiation both rely on this config on the incoming-request path).
    pub(crate) config: Arc<std::sync::RwLock<Option<DialogManagerConfig>>>,

    /// Active dialogs by dialog ID
    pub(crate) dialogs: Arc<DashMap<DialogId, Dialog>>,

    /// Dialog lookup by call-id + tags (key: "call-id:local-tag:remote-tag")
    pub(crate) dialog_lookup: Arc<DashMap<String, DialogId>>,

    /// Early dialog lookup by call-id + remote tag. Avoids scanning all
    /// dialogs for new INVITEs that do not have a To tag.
    pub(crate) early_dialog_lookup: Arc<DashMap<String, DialogId>>,

    /// Recently terminated BYE lookup by call-id + tags. This preserves
    /// idempotent 200 OK handling for late BYE retransmits after the full
    /// dialog record has been removed from the hot lookup maps.
    pub(crate) terminated_bye_lookup: Arc<DashMap<String, TerminatedByeTombstone>>,

    /// Approximate insert counter used to prune terminated BYE tombstones
    /// without scanning the full map on every call.
    terminated_bye_insert_count: Arc<AtomicUsize>,

    /// Capacity-derived safety cap for the terminated BYE tombstone index.
    /// Server/high-CPS deployments increase this with their dialog index
    /// capacity so late retransmits are not evicted while still within TTL.
    terminated_bye_lookup_hard_max: usize,

    /// Transaction to dialog mapping
    pub(crate) transaction_to_dialog: Arc<DashMap<TransactionKey, DialogId>>,

    /// Outbound transport context recorded after `send_request` succeeds,
    /// keyed by transaction id. This is used by higher layers when a later
    /// 401/407 challenge needs transport-truth policy input for Basic/Bearer.
    pub(crate) outbound_transport_by_transaction: Arc<DashMap<TransactionKey, SipTransportContext>>,

    /// Same outbound transport context keyed by SIP request identity
    /// (`Call-ID`, `CSeq` method, `CSeq` number). Non-dialog sends such as
    /// REGISTER return only a response to public callers, so this index lets
    /// the response locate the request transport without changing API shape.
    pub(crate) outbound_transport_by_request_key: Arc<DashMap<String, SipTransportContext>>,

    /// Transaction to call-affinity route hash. Dialog dispatch records the
    /// call route from request-bearing events so later lifecycle events for
    /// the same transaction do not get sent to a different dialog worker.
    transaction_dialog_route_hash: Arc<DashMap<TransactionKey, u64>>,

    /// Dialog to INVITE transaction mapping. This is the reverse hot-path
    /// index for CANCEL and authenticated-INVITE retry handling; callers
    /// should not scan `transaction_to_dialog` to rediscover INVITEs.
    pub(crate) dialog_invite_transactions: Arc<DashMap<DialogId, Vec<TransactionKey>>>,

    /// Active and recently-completed logical initial-INVITE plans. Attempts
    /// remain indexed for a bounded late-2xx window after a candidate is
    /// superseded or the logical operation completes.
    pub(crate) invite_failover_plans: Arc<
        DashMap<u64, Arc<tokio::sync::Mutex<super::transaction_integration::InviteFailoverPlan>>>,
    >,
    pub(crate) active_invite_failover_by_dialog: Arc<DashMap<DialogId, u64>>,
    pub(crate) invite_failover_attempts:
        Arc<DashMap<TransactionKey, super::transaction_integration::InviteFailoverAttemptIndex>>,
    pub(crate) invite_failover_plan_reservations: Arc<AtomicUsize>,
    pub(crate) invite_failover_attempt_reservations: Arc<AtomicUsize>,
    pub(crate) next_invite_failover_plan_id: Arc<AtomicU64>,
    pub(crate) invite_failover_insert_count: Arc<AtomicUsize>,
    pub(crate) invite_failover_plan_capacity: usize,
    pub(crate) invite_failover_attempt_capacity: usize,

    /// Dialog to server transaction mapping. Session-level response APIs
    /// need this to select the pending UAS transaction without scanning the
    /// many-to-one `transaction_to_dialog` map under high CPS load.
    pub(crate) dialog_server_transactions: Arc<DashMap<DialogId, Vec<TransactionKey>>>,

    /// Dialog to currently pending server transaction response. This lets
    /// session-core answer the initial INVITE without scanning
    /// transaction_to_dialog under high call volume.
    pub(crate) pending_response_transaction_by_dialog: Arc<DashMap<DialogId, TransactionKey>>,

    /// Session to dialog mapping for cross-crate coordination
    pub(crate) session_to_dialog: Arc<DashMap<String, DialogId>>,

    /// Dialog to session mapping
    pub(crate) dialog_to_session: Arc<DashMap<DialogId, String>>,

    /// Event hub for global event coordination
    pub(crate) event_hub: Arc<tokio::sync::RwLock<Option<Arc<crate::events::DialogEventHub>>>>,

    /// Channel for sending session coordination events to session-core
    pub(crate) session_coordinator:
        Arc<tokio::sync::RwLock<Option<mpsc::Sender<SessionCoordinationEvent>>>>,

    /// Channel for sending dialog events to external consumers (session-core)
    pub(crate) dialog_event_sender: Arc<tokio::sync::RwLock<Option<mpsc::Sender<DialogEvent>>>>,

    /// Channel for receiving dialog events (for shutdown coordination).
    /// Retained so a future "consume remaining events on shutdown"
    /// path can drain the channel; today the manager just drops the
    /// receiver to signal completion.
    #[allow(dead_code)]
    pub(crate) dialog_event_receiver: Arc<tokio::sync::RwLock<Option<mpsc::Receiver<DialogEvent>>>>,

    /// Shutdown signal for global event processor
    pub(crate) shutdown_signal: Arc<tokio::sync::Notify>,

    /// Subscription manager for handling SUBSCRIBE/NOTIFY
    pub(crate) subscription_manager: Option<Arc<SubscriptionManager>>,

    /// Abort handles for in-flight UAS reliable-provisional retransmit tasks
    /// (RFC 3262 §3). Keyed by `(dialog_id, rseq)`. On PRACK arrival the
    /// matching entry is removed and aborted so the 18x stops retransmitting;
    /// on dialog termination every entry for that dialog is aborted.
    pub(crate) reliable_provisional_tasks: Arc<DashMap<(DialogId, u32), tokio::task::AbortHandle>>,

    /// Abort handles for per-dialog RFC 4028 session-timer refresh tasks.
    /// Populated when the UAC or UAS is designated refresher; one entry per
    /// dialog. Aborted on dialog termination.
    pub(crate) session_refresh_tasks: Arc<DashMap<DialogId, tokio::task::AbortHandle>>,

    /// Discovered public address from RFC 3581 `received=` / `rport=`
    /// echoed back on responses.
    ///
    /// On every inbound response we peek at the top `Via` header; when
    /// it carries `received=<ip>` plus a populated `rport=<port>` (set
    /// because we put `;rport` on the outgoing Via per RFC 3581 §4),
    /// we treat that as our externally-visible address as observed by
    /// the immediate hop. This lets a UA behind NAT discover its
    /// public address without STUN, then advertise it in subsequent
    /// `Contact:` headers (RFC 5626 §5).
    ///
    /// `None` until the first qualifying response arrives. Most-
    /// recent observation wins — if multiple peers see us through
    /// different NAT mappings, the latest update is authoritative.
    /// (Per-peer mapping would be a richer model; not yet justified
    /// by real-world traffic.)
    pub(crate) nat_discovered_addr: Arc<tokio::sync::RwLock<Option<SocketAddr>>>,

    /// Registrar-returned Service-Route (RFC 3608) keyed by AoR.
    ///
    /// Populated on successful REGISTER 2xx responses: the registrar
    /// echoes the ordered list of URIs that the UA MUST pre-load as
    /// Route headers for subsequent out-of-dialog requests within the
    /// registration binding. The key is the AoR (To URI, which for a
    /// UAC-originated REGISTER equals the From URI) normalized to its
    /// string form.
    ///
    /// Most recent REGISTER 2xx wins per AoR. Empty `Vec` means "we
    /// saw a REGISTER 2xx without Service-Route" (distinct from "no
    /// registration yet"); callers that care about the distinction
    /// should use `service_route_for_aor` and match on `None`.
    pub(crate) service_route_by_aor: Arc<
        tokio::sync::RwLock<
            std::collections::HashMap<String, Vec<rvoip_sip_core::types::uri::Uri>>,
        >,
    >,

    /// Registrar-assigned GRUU URIs (RFC 5627 §5.3) keyed by AoR.
    ///
    /// Populated on successful REGISTER 2xx responses when the
    /// registrar echoes the Contact with `pub-gruu="..."` and/or
    /// `temp-gruu="..."` parameters. Most recent REGISTER 2xx wins
    /// per AoR. `None` from
    /// [`Self::gruu_for_aor`](crate::manager::DialogManager::gruu_for_aor)
    /// means "no REGISTER 2xx with GRUU observed yet" (distinct from
    /// "registrar didn't assign a GRUU on this binding"). A registrar
    /// may assign only `pub-gruu` or only `temp-gruu` — the cached
    /// `GruuContactParams` carries `Option`s for each independently.
    pub(crate) gruu_by_aor: Arc<
        tokio::sync::RwLock<
            std::collections::HashMap<String, rvoip_sip_core::types::outbound::GruuContactParams>,
        >,
    >,

    /// RFC 5626 §3.5.1 outbound-flow state machines, keyed by
    /// `(AoR, reg-id, instance-id)` per RFC 5626 §4.2.
    ///
    /// Each successful outbound-aware REGISTER 2xx spawns one task (see
    /// [`Self::outbound_flow_tasks`]) that pings every
    /// [`outbound_keepalive_interval`](Self::outbound_keepalive_interval)
    /// and monitors the pong window. A pong timeout, `ConnectionClosed`
    /// event, or send error flips the [`OutboundFlow`] into
    /// `FlowState::Failed` and emits a
    /// [`SessionCoordinationEvent::OutboundFlowFailed`] (once) so
    /// session-core can trigger a fresh REGISTER (RFC 5626 §4.4.1).
    ///
    /// Idempotent: starting a flow for a key that already has one stops
    /// the prior flow first.
    pub(crate) outbound_flows: Arc<DashMap<(String, u32, String), Arc<OutboundFlow>>>,

    /// Abort handles for the spawned ping/monitor task of each entry in
    /// [`Self::outbound_flows`]. Split from the flow state so the state
    /// can be inspected (e.g. by pong/close handlers) without touching
    /// the task handle.
    pub(crate) outbound_flow_tasks: Arc<DashMap<(String, u32, String), tokio::task::AbortHandle>>,

    /// Secondary index mapping destination `SocketAddr` →
    /// `(aor, reg_id, instance)` flow keys, populated when
    /// `start_outbound_ping` installs a flow. Lets transport-side events
    /// (`KeepAlivePongReceived`, `ConnectionClosed`) — which arrive
    /// keyed only by IP:port — locate the flow(s) to update in O(1).
    pub(crate) flow_by_destination: Arc<DashMap<SocketAddr, Vec<(String, u32, String)>>>,

    /// Secondary outbound-flow index keyed by AoR. Used by registration
    /// policy checks that only need to know whether an AoR has active flow
    /// state, without scanning all outbound flows.
    pub(crate) flow_by_aor: Arc<DashMap<String, Vec<(String, u32, String)>>>,

    /// Keep-alive interval for RFC 5626 outbound flows, threaded from
    /// `session-core::Config::outbound_keepalive_interval_secs`. `None`
    /// disables keep-alive entirely — `start_outbound_ping` becomes a
    /// no-op.
    pub(crate) outbound_keepalive_interval: Arc<std::sync::RwLock<Option<std::time::Duration>>>,

    /// Pluggable RFC 8224 STIR/SHAKEN PASSporT verifier. When `Some`,
    /// the inbound event adapter runs verification on every inbound
    /// request that carries an `Identity:` header before the
    /// cross-crate `IncomingCall` event is published. Reference impl
    /// lives in `rvoip-stir-shaken`. Application sets this via
    /// [`DialogManager::set_identity_verifier`].
    pub(crate) identity_verifier: Arc<std::sync::RwLock<Option<crate::manager::SharedVerifier>>>,

    /// Pluggable RFC 8224 STIR/SHAKEN PASSporT signer. When `Some`,
    /// the outbound request lifecycle attaches an `Identity:` header
    /// to dialog-creating requests before they hit the wire.
    /// Reference impl lives in `rvoip-stir-shaken`.
    pub(crate) identity_signer: Arc<std::sync::RwLock<Option<crate::manager::SharedSigner>>>,

    /// Policy for what to do when verification fails or `Identity:` is
    /// absent. Defaults to [`VerificationPolicy::Annotate`] (forward
    /// outcome to session-core without rejecting). See
    /// [`VerificationPolicy`] for the full semantics.
    pub(crate) verification_policy: Arc<std::sync::RwLock<crate::manager::VerificationPolicy>>,

    /// Pluggable RFC 3263 URI → next-hop resolver. When `Some`, the
    /// manager consults this resolver to translate destination URIs into
    /// `SocketAddr`s on its outbound paths (INVITE send and friends).
    /// When `None` (default), resolution falls back to the process-wide
    /// system `HickoryResolver`, preserving pre-Phase-5 behaviour.
    /// Application sets this via [`DialogManager::set_resolver`].
    pub(crate) resolver:
        Arc<std::sync::RwLock<Option<Arc<dyn rvoip_sip_transport::resolver::Resolver>>>>,
}

impl std::fmt::Debug for DialogManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DialogManager")
            .field("local_address", &self.local_address)
            .field("dialogs_len", &self.dialogs.len())
            .field("early_dialog_lookup_len", &self.early_dialog_lookup.len())
            .field(
                "terminated_bye_lookup_len",
                &self.terminated_bye_lookup.len(),
            )
            .field(
                "terminated_bye_lookup_hard_max",
                &self.terminated_bye_lookup_hard_max,
            )
            .field(
                "pending_response_transaction_by_dialog_len",
                &self.pending_response_transaction_by_dialog.len(),
            )
            .field(
                "identity_verifier",
                &self
                    .identity_verifier
                    .read()
                    .ok()
                    .map(|g| if g.is_some() { "Some" } else { "None" })
                    .unwrap_or("?"),
            )
            .field(
                "identity_signer",
                &self
                    .identity_signer
                    .read()
                    .ok()
                    .map(|g| if g.is_some() { "Some" } else { "None" })
                    .unwrap_or("?"),
            )
            .field(
                "resolver",
                &self
                    .resolver
                    .read()
                    .ok()
                    .map(|g| if g.is_some() { "Some" } else { "None" })
                    .unwrap_or("?"),
            )
            .finish_non_exhaustive()
    }
}

fn start_dialog_event_dispatch_workers(
    manager: DialogManager,
    worker_count: usize,
    queue_capacity: usize,
) -> Arc<Vec<mpsc::Sender<QueuedDialogTransactionEvent>>> {
    let worker_count = worker_count.clamp(1, super::MAX_DIALOG_EVENT_DISPATCH_WORKERS);
    let per_worker_capacity = (queue_capacity / worker_count).max(1);
    let mut senders = Vec::with_capacity(worker_count);

    for worker_id in 0..worker_count {
        let (tx, mut rx) = mpsc::channel::<QueuedDialogTransactionEvent>(per_worker_capacity);
        let manager_for_worker = manager.clone();
        tokio::spawn(async move {
            while let Some(queued) = rx.recv().await {
                if let Some(queued_at) = queued.queued_at {
                    crate::diagnostics::record_dialog_event_dispatch_queue_delay(
                        queued_at.elapsed(),
                    );
                }
                manager_for_worker
                    .process_timed_global_transaction_event(queued.event)
                    .await;
            }
            debug!(
                worker_id,
                "Dialog transaction-event dispatch worker terminated"
            );
        });
        senders.push(tx);
    }

    info!(
        workers = worker_count,
        per_worker_capacity, "Dialog transaction-event dispatch workers enabled"
    );

    Arc::new(senders)
}

async fn dispatch_dialog_transaction_event(
    manager: &DialogManager,
    event: TransactionEvent,
    dispatch_senders: &Arc<Vec<mpsc::Sender<QueuedDialogTransactionEvent>>>,
    fallback_worker: &AtomicUsize,
) {
    let worker_index =
        manager.dialog_event_dispatch_worker_index(&event, dispatch_senders.len(), fallback_worker);
    let timing_enabled = crate::diagnostics::dialog_timing_enabled();
    let kind = timing_enabled.then(|| dialog_event_kind(&event));
    let queued = QueuedDialogTransactionEvent {
        event,
        kind,
        queued_at: timing_enabled.then(Instant::now),
    };

    match dispatch_senders[worker_index].try_send(queued) {
        Ok(()) => {}
        Err(mpsc::error::TrySendError::Full(queued)) => {
            warn!(
                worker_index,
                kind = queued.kind.map(|kind| kind.as_str()).unwrap_or("unknown"),
                "Dialog transaction-event dispatch queue full; applying backpressure"
            );
            let backpressure_started = timing_enabled.then(Instant::now);
            if dispatch_senders[worker_index].send(queued).await.is_err() {
                warn!(
                    worker_index,
                    "Dialog transaction-event dispatch worker channel closed"
                );
            }
            if let Some(started) = backpressure_started {
                crate::diagnostics::record_dialog_event_dispatch_backpressure(started.elapsed());
            }
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {
            warn!(
                worker_index,
                "Dialog transaction-event dispatch worker channel closed"
            );
        }
    }
}

impl DialogManager {
    /// Create a new dialog manager
    ///
    /// **ARCHITECTURE**: dialog-core receives TransactionManager via dependency injection.
    /// The application level is responsible for creating the transaction layer.
    ///
    /// # Arguments
    /// * `transaction_manager` - The transaction manager to use for SIP message reliability
    /// * `local_address` - The local address to use in Via headers and Contact headers
    ///
    /// # Returns
    /// A new DialogManager instance ready for use
    pub async fn new(
        transaction_manager: Arc<TransactionManager>,
        local_address: SocketAddr,
    ) -> DialogResult<Self> {
        Self::new_with_index_capacity(
            transaction_manager,
            local_address,
            dialog_index_capacity(None),
        )
        .await
    }

    pub async fn new_with_index_capacity(
        transaction_manager: Arc<TransactionManager>,
        local_address: SocketAddr,
        index_capacity: usize,
    ) -> DialogResult<Self> {
        info!(
            "Creating new DialogManager with local address {}",
            local_address
        );

        // Create shared stores
        let index_capacity = index_capacity.max(MIN_DIALOG_INDEX_CAPACITY);
        let dialogs = Arc::new(DashMap::with_capacity(index_capacity));
        let dialog_lookup = Arc::new(DashMap::with_capacity(index_capacity.saturating_mul(2)));

        // Create dialog event channel for subscription manager
        let (event_tx, _) = mpsc::channel(100);

        // Create subscription manager with shared stores
        let subscription_manager =
            SubscriptionManager::new(dialogs.clone(), dialog_lookup.clone(), event_tx);

        Ok(Self {
            transaction_manager,
            local_address,
            config: Arc::new(std::sync::RwLock::new(None)),
            dialogs,
            dialog_lookup,
            early_dialog_lookup: Arc::new(DashMap::with_capacity(index_capacity)),
            terminated_bye_lookup: Arc::new(DashMap::with_capacity(index_capacity)),
            terminated_bye_insert_count: Arc::new(AtomicUsize::new(0)),
            terminated_bye_lookup_hard_max: terminated_bye_lookup_hard_max(index_capacity),
            transaction_to_dialog: Arc::new(DashMap::with_capacity(index_capacity)),
            outbound_transport_by_transaction: Arc::new(DashMap::with_capacity(index_capacity)),
            outbound_transport_by_request_key: Arc::new(DashMap::with_capacity(index_capacity)),
            transaction_dialog_route_hash: Arc::new(DashMap::with_capacity(index_capacity)),
            dialog_invite_transactions: Arc::new(DashMap::with_capacity(index_capacity)),
            invite_failover_plans: Arc::new(DashMap::with_capacity(index_capacity)),
            active_invite_failover_by_dialog: Arc::new(DashMap::with_capacity(index_capacity)),
            invite_failover_attempts: Arc::new(DashMap::with_capacity(
                invite_failover_attempt_capacity(index_capacity),
            )),
            invite_failover_plan_reservations: Arc::new(AtomicUsize::new(0)),
            invite_failover_attempt_reservations: Arc::new(AtomicUsize::new(0)),
            next_invite_failover_plan_id: Arc::new(AtomicU64::new(1)),
            invite_failover_insert_count: Arc::new(AtomicUsize::new(0)),
            invite_failover_plan_capacity: index_capacity,
            invite_failover_attempt_capacity: invite_failover_attempt_capacity(index_capacity),
            dialog_server_transactions: Arc::new(DashMap::with_capacity(index_capacity)),
            pending_response_transaction_by_dialog: Arc::new(DashMap::with_capacity(
                index_capacity,
            )),
            session_to_dialog: Arc::new(DashMap::with_capacity(index_capacity)),
            dialog_to_session: Arc::new(DashMap::with_capacity(index_capacity)),
            event_hub: Arc::new(tokio::sync::RwLock::new(None)),
            session_coordinator: Arc::new(tokio::sync::RwLock::new(None)),
            dialog_event_sender: Arc::new(tokio::sync::RwLock::new(None)),
            dialog_event_receiver: Arc::new(tokio::sync::RwLock::new(None)),
            shutdown_signal: Arc::new(tokio::sync::Notify::new()),
            subscription_manager: Some(Arc::new(subscription_manager)),
            reliable_provisional_tasks: Arc::new(DashMap::with_capacity(index_capacity)),
            session_refresh_tasks: Arc::new(DashMap::with_capacity(index_capacity)),
            nat_discovered_addr: Arc::new(tokio::sync::RwLock::new(None)),
            service_route_by_aor: Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            gruu_by_aor: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            outbound_flows: Arc::new(DashMap::with_capacity(index_capacity)),
            outbound_flow_tasks: Arc::new(DashMap::with_capacity(index_capacity)),
            flow_by_destination: Arc::new(DashMap::with_capacity(index_capacity)),
            flow_by_aor: Arc::new(DashMap::with_capacity(index_capacity)),
            outbound_keepalive_interval: Arc::new(std::sync::RwLock::new(None)),
            identity_verifier: Arc::new(std::sync::RwLock::new(None)),
            identity_signer: Arc::new(std::sync::RwLock::new(None)),
            verification_policy: Arc::new(std::sync::RwLock::new(
                crate::manager::VerificationPolicy::default(),
            )),
            resolver: Arc::new(std::sync::RwLock::new(None)),
        })
    }

    /// Configure the RFC 5626 §3.5.1 keep-alive interval for this
    /// DialogManager. `None` (or not calling this at all) disables
    /// outbound keep-alive; subsequent REGISTER 2xx responses will not
    /// spawn ping tasks. The session-core coordinator wires this from
    /// its `outbound_keepalive_interval_secs` config at boot.
    pub fn set_outbound_keepalive_interval(&self, interval: Option<std::time::Duration>) {
        if let Ok(mut guard) = self.outbound_keepalive_interval.write() {
            *guard = interval;
        }
    }

    /// Read the currently-configured RFC 5626 keep-alive interval.
    pub fn outbound_keepalive_interval(&self) -> Option<std::time::Duration> {
        self.outbound_keepalive_interval
            .read()
            .ok()
            .and_then(|g| *g)
    }

    /// Install a pluggable RFC 8224 STIR/SHAKEN PASSporT verifier.
    ///
    /// When set, the inbound event adapter runs verification on every
    /// inbound request that carries an `Identity:` header before the
    /// cross-crate `IncomingCall` event is published. The reference
    /// implementation lives in `rvoip-stir-shaken`.
    ///
    /// Pass `None` to disable verification — inbound `Identity:`
    /// headers will be carried through as raw bytes for downstream
    /// consumers without semantic checking.
    pub fn set_identity_verifier(&self, verifier: Option<crate::manager::SharedVerifier>) {
        if let Ok(mut guard) = self.identity_verifier.write() {
            *guard = verifier;
        }
    }

    /// Read the currently-installed PASSporT verifier (if any).
    pub fn identity_verifier(&self) -> Option<crate::manager::SharedVerifier> {
        self.identity_verifier.read().ok().and_then(|g| g.clone())
    }

    /// Install a pluggable RFC 8224 STIR/SHAKEN PASSporT signer.
    ///
    /// When set, the outbound request lifecycle attaches an
    /// `Identity:` header to dialog-creating requests before they hit
    /// the wire. The reference implementation lives in
    /// `rvoip-stir-shaken`.
    pub fn set_identity_signer(&self, signer: Option<crate::manager::SharedSigner>) {
        if let Ok(mut guard) = self.identity_signer.write() {
            *guard = signer;
        }
    }

    /// Read the currently-installed PASSporT signer (if any).
    pub fn identity_signer(&self) -> Option<crate::manager::SharedSigner> {
        self.identity_signer.read().ok().and_then(|g| g.clone())
    }

    /// Set the policy that decides how the dialog layer reacts to a
    /// failed PASSporT verification or a missing `Identity:` header.
    /// Defaults to `Annotate` — the outcome is forwarded to
    /// session-core without rejecting.
    pub fn set_verification_policy(&self, policy: crate::manager::VerificationPolicy) {
        if let Ok(mut guard) = self.verification_policy.write() {
            *guard = policy;
        }
    }

    /// Read the currently-configured verification policy.
    pub fn verification_policy(&self) -> crate::manager::VerificationPolicy {
        self.verification_policy
            .read()
            .ok()
            .map(|g| *g)
            .unwrap_or_default()
    }

    /// Install a pluggable RFC 3263 URI resolver.
    ///
    /// When set, the manager consults this resolver to translate
    /// destination URIs into `SocketAddr`s before passing them to the
    /// transaction layer. Pass `None` to revert to the process-wide
    /// default (system `HickoryResolver`).
    pub fn set_resolver(&self, resolver: Option<Arc<dyn rvoip_sip_transport::resolver::Resolver>>) {
        if let Ok(mut guard) = self.resolver.write() {
            *guard = resolver;
        }
    }

    /// Read the currently-installed RFC 3263 resolver (if any).
    pub fn resolver(&self) -> Option<Arc<dyn rvoip_sip_transport::resolver::Resolver>> {
        self.resolver.read().ok().and_then(|g| g.clone())
    }

    /// Resolve a destination URI to a `SocketAddr`, consulting the
    /// configured [`Resolver`](rvoip_sip_transport::resolver::Resolver)
    /// when one is installed and falling back to the process-wide
    /// `HickoryResolver` otherwise. IP-literal URIs short-circuit
    /// without touching DNS so this works in sandboxed environments
    /// even when no system resolver is available.
    ///
    /// Returns `None` when the URI cannot be resolved.
    pub async fn resolve_uri_to_socketaddr(
        &self,
        uri: &rvoip_sip_core::Uri,
    ) -> Option<std::net::SocketAddr> {
        if let Some(resolver) = self.resolver() {
            match resolver.resolve(uri).await {
                Ok(candidates) => return candidates.into_iter().next().map(|t| t.addr),
                Err(_error) => {
                    tracing::debug!("Configured resolver returned an error");
                    return None;
                }
            }
        }
        crate::dialog::dialog_utils::resolve_uri_to_socketaddr(uri).await
    }

    /// Resolve a destination URI to the FULL candidate list (RFC 3263
    /// §4). Callers iterate candidates in returned order, trying the
    /// next one on transport-level failure per RFC 3263 §4.3.
    ///
    /// Consults the configured [`Resolver`](rvoip_sip_transport::resolver::Resolver)
    /// when one is installed; falls back to the process-wide default
    /// otherwise. IP-literal URIs short-circuit to a single-element
    /// vector.
    pub async fn resolve_uri_to_candidates(
        &self,
        uri: &rvoip_sip_core::Uri,
    ) -> Vec<rvoip_sip_transport::resolver::ResolvedTarget> {
        if let Some(resolver) = self.resolver() {
            match resolver.resolve(uri).await {
                Ok(candidates) => return candidates,
                Err(_error) => {
                    tracing::debug!("Configured resolver returned an error");
                    return Vec::new();
                }
            }
        }
        crate::dialog::dialog_utils::resolve_uri_to_candidates(uri).await
    }

    /// Decision returned by [`Self::run_identity_verification`].
    ///
    /// Owned by [`crate::manager`] (re-exported) so both publish paths
    /// — [`DialogEventAdapter`](crate::events::DialogEventAdapter) and
    /// [`DialogEventHub`](crate::events::DialogEventHub) — apply the
    /// same RFC 8224 §6.2.2 reject contract.
    pub async fn run_identity_verification(
        &self,
        event: &crate::events::SessionCoordinationEvent,
        raw_request: &Option<bytes::Bytes>,
    ) -> IdentityVerificationDecision {
        // No verifier installed → annotate as `None`, never reject.
        let verifier = match self.identity_verifier() {
            Some(v) => v,
            None => return IdentityVerificationDecision::Publish(None),
        };

        // Only IncomingCall events carry an inbound INVITE worth
        // verifying; other coordination events ride through untouched.
        let request = match event {
            crate::events::SessionCoordinationEvent::IncomingCall { request, .. } => {
                request.clone()
            }
            _ => return IdentityVerificationDecision::Publish(None),
        };

        // Extract the typed Identity header.
        let identity_opt = request.headers.iter().find_map(|h| match h {
            rvoip_sip_core::types::TypedHeader::Identity(id) => Some(id.clone()),
            _ => None,
        });

        // Resolve the byte-exact upstream form. Fall back to
        // re-serialising the parsed Request only when the transport
        // cache missed (synthetic / mock transport paths).
        let raw_bytes = raw_request.clone().unwrap_or_else(|| {
            bytes::Bytes::from(rvoip_sip_core::Message::Request(request.clone()).to_bytes())
        });

        let outcome = match identity_opt {
            Some(identity) => verifier.verify(&raw_bytes, &identity, &request).await,
            None => crate::manager::VerificationOutcome::NoIdentity,
        };

        let policy = self.verification_policy();
        if outcome.should_reject(policy) {
            // Wire the RFC 8224 §6.2.2 4xx response back through the
            // server transaction. Identical to the adapter's reject
            // helper; lives here so both publish paths share one
            // implementation.
            self.reject_inbound_identity_internal(event, &outcome).await;
            return IdentityVerificationDecision::Drop;
        }

        let cc_status = match &outcome {
            crate::manager::VerificationOutcome::Valid { .. } => IdentityVerificationStatus::Valid,
            crate::manager::VerificationOutcome::Stale { .. } => IdentityVerificationStatus::Stale,
            crate::manager::VerificationOutcome::BadSignature => {
                IdentityVerificationStatus::BadSignature
            }
            crate::manager::VerificationOutcome::BadChain { .. } => {
                IdentityVerificationStatus::BadChain
            }
            crate::manager::VerificationOutcome::ClaimMismatch { .. } => {
                IdentityVerificationStatus::ClaimMismatch
            }
            crate::manager::VerificationOutcome::BadInfo { .. } => {
                IdentityVerificationStatus::BadInfo
            }
            crate::manager::VerificationOutcome::NoIdentity => {
                IdentityVerificationStatus::NoIdentity
            }
        };
        IdentityVerificationDecision::Publish(Some(cc_status))
    }

    /// Send the RFC 8224 §6.2.2 4xx response back through the server
    /// transaction. Internal helper used by [`Self::run_identity_verification`];
    /// kept on `DialogManager` so both publish paths reach the same
    /// code.
    async fn reject_inbound_identity_internal(
        &self,
        event: &crate::events::SessionCoordinationEvent,
        outcome: &crate::manager::VerificationOutcome,
    ) {
        use rvoip_sip_core::builder::SimpleResponseBuilder;
        use rvoip_sip_core::types::StatusCode;

        let (transaction_id, request) = match event {
            crate::events::SessionCoordinationEvent::IncomingCall {
                transaction_id,
                request,
                ..
            } => (transaction_id.clone(), request.clone()),
            _ => return,
        };

        let status_u16 = outcome.reject_status().unwrap_or(428);
        let reason = match status_u16 {
            403 => "Stale Date",
            428 => "Use Identity Header",
            436 => "Bad Identity Info",
            437 => "Unsupported Credential",
            438 => "Invalid Identity Header",
            _ => "Forbidden",
        };
        let status = StatusCode::from_u16(status_u16).unwrap_or(StatusCode::Forbidden);

        tracing::info!(
            "STIR/SHAKEN reject: outcome_class={} status={} on transaction",
            verification_outcome_class(outcome),
            status_u16
        );

        let response =
            SimpleResponseBuilder::response_from_request(&request, status, Some(reason)).build();

        if let Err(_error) = self
            .transaction_manager
            .send_response(&transaction_id, response)
            .await
        {
            tracing::error!("Failed to send STIR/SHAKEN reject response on transaction");
        }
    }

    /// Spawn (or replace) a RFC 5626 §3.5.1 CRLFCRLF keep-alive flow
    /// targeting `destination` via the DialogManager's transport.
    ///
    /// `flow_key = (AoR, reg-id, instance-id)` is the outbound flow
    /// identity per RFC 5626 §4.2; a second call for the same key
    /// stops the prior flow first (idempotent refresh on re-REGISTER).
    ///
    /// Phase 2c: the spawned task drives an `OutboundFlow` state
    /// machine — after each ping it arms a pong deadline, and on
    /// pong-timeout / connection-closed / send-error it emits a single
    /// [`SessionCoordinationEvent::OutboundFlowFailed`] so session-core
    /// can trigger a fresh REGISTER without waiting for registration
    /// expiry.
    ///
    /// No-op when `outbound_keepalive_interval` is `None`.
    pub fn start_outbound_ping(&self, flow_key: (String, u32, String), destination: SocketAddr) {
        let _ = flow_key;
        warn!(
            destination = %destination,
            "refusing address-only outbound keep-alive; retain the transaction's exact transport route"
        );
    }

    /// Spawn a keep-alive monitor bound to the exact established transport
    /// flow represented by `route`.
    pub fn start_outbound_ping_on_route(
        &self,
        flow_key: (String, u32, String),
        route: rvoip_sip_transport::TransportRoute,
    ) -> bool {
        let Some(interval) = self.outbound_keepalive_interval() else {
            return false;
        };
        if interval.is_zero() {
            return false;
        }

        let Some(flow_id) = route.flow_id else {
            warn!(
                destination = %route.destination,
                "refusing flowless outbound keep-alive route"
            );
            return false;
        };
        if !matches!(
            route.transport_type,
            Some(
                rvoip_sip_transport::transport::TransportType::Tcp
                    | rvoip_sip_transport::transport::TransportType::Tls
                    | rvoip_sip_transport::transport::TransportType::Ws
                    | rvoip_sip_transport::transport::TransportType::Wss
            )
        ) {
            warn!(
                destination = %route.destination,
                flow_id = flow_id.as_u64(),
                "refusing outbound CRLF keep-alive on a non-stream route"
            );
            return false;
        }

        // Replace any prior flow for this key (idempotent on re-REGISTER).
        self.stop_outbound_ping(&flow_key);

        let transport = self.transaction_manager.transport().clone();
        let destination = route.destination;
        let flow = Arc::new(OutboundFlow::new_with_route(
            flow_key.clone(),
            route,
            interval,
        ));
        let manager = self.clone();
        let flow_for_task = flow.clone();

        let handle = tokio::spawn(async move {
            run_outbound_flow_loop(manager, flow_for_task, transport).await;
        })
        .abort_handle();

        self.outbound_flows.insert(flow_key.clone(), flow);
        self.outbound_flow_tasks.insert(flow_key.clone(), handle);
        self.index_outbound_flow_key(flow_key, destination);
        true
    }

    /// Stop (and forget) the RFC 5626 keep-alive flow for this key, if
    /// any. Aborts the monitor task and tears down both the primary
    /// flow map and the destination secondary index. Does **not** emit
    /// an `OutboundFlowFailed` event — explicit teardown is not a flow
    /// failure; callers that want the failure event must call
    /// `mark_failed` on the `OutboundFlow` first.
    pub fn stop_outbound_ping(&self, flow_key: &(String, u32, String)) {
        if let Some((_, handle)) = self.outbound_flow_tasks.remove(flow_key) {
            handle.abort();
        }
        if let Some((_, flow)) = self.outbound_flows.remove(flow_key) {
            self.remove_outbound_flow_indexes(flow_key, flow.destination);
        }
    }

    pub(crate) fn index_outbound_flow_key(
        &self,
        flow_key: (String, u32, String),
        destination: SocketAddr,
    ) {
        let aor = flow_key.0.clone();

        let mut destination_keys = self
            .flow_by_destination
            .entry(destination)
            .or_insert_with(Vec::new);
        if !destination_keys.iter().any(|key| key == &flow_key) {
            destination_keys.push(flow_key.clone());
        }

        let mut aor_keys = self.flow_by_aor.entry(aor).or_insert_with(Vec::new);
        if !aor_keys.iter().any(|key| key == &flow_key) {
            aor_keys.push(flow_key);
        }
    }

    fn remove_outbound_flow_indexes(
        &self,
        flow_key: &(String, u32, String),
        destination: SocketAddr,
    ) {
        if let Some(mut entry) = self.flow_by_destination.get_mut(&destination) {
            entry.value_mut().retain(|key| key != flow_key);
        }
        self.flow_by_destination
            .remove_if(&destination, |_, keys| keys.is_empty());

        if let Some(mut entry) = self.flow_by_aor.get_mut(&flow_key.0) {
            entry.value_mut().retain(|key| key != flow_key);
        }
        self.flow_by_aor
            .remove_if(&flow_key.0, |_, keys| keys.is_empty());
    }

    /// Transport reported `KeepAlivePongReceived` from `source`. Update
    /// every outbound flow that's aimed at that peer so the pong is
    /// treated as an answer to the in-flight ping (if any). No-op when
    /// no flow is registered for the address.
    pub async fn on_pong_received(&self, source: SocketAddr) {
        self.on_pong_received_on_flow(source, None).await;
    }

    /// Transport pong callback with an exact connection identity.
    pub async fn on_pong_received_on_flow(
        &self,
        source: SocketAddr,
        flow_id: Option<rvoip_sip_transport::TransportFlowId>,
    ) {
        let keys: Vec<(String, u32, String)> = match self.flow_by_destination.get(&source) {
            Some(entry) => entry.value().clone(),
            None => return,
        };
        for key in keys {
            if let Some(flow) = self.outbound_flows.get(&key).map(|e| e.value().clone()) {
                if flow.route.flow_id != flow_id {
                    continue;
                }
                flow.on_pong().await;
                tracing::trace!(
                    src = %source,
                    "RFC 5626 pong received — flow reset to Idle"
                );
            }
        }
    }

    /// Transport reported `ConnectionClosed` to `remote_addr`. Every
    /// outbound flow aimed at that peer is marked failed (once), emits
    /// an `OutboundFlowFailed` event with `ConnectionClosed` reason,
    /// and has its monitor task torn down. The peer reconnect is
    /// session-core's problem (trigger re-REGISTER) — dialog-core only
    /// reports the flow death.
    pub async fn on_connection_closed(&self, remote_addr: SocketAddr) {
        self.on_connection_closed_on_flow(remote_addr, None).await;
    }

    /// Transport close callback with an exact connection identity.
    pub async fn on_connection_closed_on_flow(
        &self,
        remote_addr: SocketAddr,
        flow_id: Option<rvoip_sip_transport::TransportFlowId>,
    ) {
        let keys: Vec<(String, u32, String)> = match self.flow_by_destination.get(&remote_addr) {
            Some(entry) => entry.value().clone(),
            None => return,
        };
        for key in keys {
            let flow = match self.outbound_flows.get(&key).map(|e| e.value().clone()) {
                Some(f) => f,
                None => continue,
            };
            if flow.route.flow_id != flow_id {
                continue;
            }
            if flow.mark_failed().await {
                tracing::info!(
                    dest = %remote_addr,
                    "RFC 5626 connection closed — flow failed"
                );
                self.emit_outbound_flow_failed(&flow, FlowFailureReason::ConnectionClosed)
                    .await;
            }
            // Explicit stop — the monitor task would have exited on its
            // own once it noticed `Failed`, but we don't need to wait.
            self.stop_outbound_ping(&key);
        }
    }

    /// Emit `SessionCoordinationEvent::OutboundFlowFailed` for a flow
    /// that just transitioned to `FlowState::Failed`. Callers are
    /// responsible for the idempotency check — only the thread that
    /// observed `mark_failed() == true` should call this.
    pub(crate) async fn emit_outbound_flow_failed(
        &self,
        flow: &OutboundFlow,
        reason: FlowFailureReason,
    ) {
        let (aor, reg_id, instance) = flow.key.clone();
        self.emit_session_coordination_event(SessionCoordinationEvent::OutboundFlowFailed {
            aor,
            reg_id,
            instance,
            reason,
        })
        .await;
    }

    /// Create a new dialog manager with global transaction events (RECOMMENDED)
    ///
    /// This constructor follows the working pattern from transaction-core examples
    /// by receiving global transaction events for proper event consumption.
    ///
    /// # Arguments
    /// * `transaction_manager` - The transaction manager to use for SIP message reliability
    /// * `transaction_events` - Global transaction event receiver
    /// * `local_address` - The local address to use in Via headers and Contact headers
    ///
    /// # Returns
    /// A new DialogManager instance with proper event consumption
    pub async fn with_global_events(
        transaction_manager: Arc<TransactionManager>,
        transaction_events: mpsc::Receiver<TransactionEvent>,
        local_address: SocketAddr,
    ) -> DialogResult<Self> {
        Self::with_global_events_and_index_capacity(
            transaction_manager,
            transaction_events,
            local_address,
            dialog_index_capacity(None),
        )
        .await
    }

    pub async fn with_global_events_and_index_capacity(
        transaction_manager: Arc<TransactionManager>,
        transaction_events: mpsc::Receiver<TransactionEvent>,
        local_address: SocketAddr,
        index_capacity: usize,
    ) -> DialogResult<Self> {
        Self::with_global_events_and_index_capacity_and_config(
            transaction_manager,
            transaction_events,
            local_address,
            index_capacity,
            None,
        )
        .await
    }

    pub async fn with_global_events_and_index_capacity_and_config(
        transaction_manager: Arc<TransactionManager>,
        transaction_events: mpsc::Receiver<TransactionEvent>,
        local_address: SocketAddr,
        index_capacity: usize,
        initial_config: Option<DialogManagerConfig>,
    ) -> DialogResult<Self> {
        info!(
            "Creating new DialogManager with global transaction events and local address {}",
            local_address
        );

        // Create shared stores
        let index_capacity = index_capacity.max(MIN_DIALOG_INDEX_CAPACITY);
        let dialogs = Arc::new(DashMap::with_capacity(index_capacity));
        let dialog_lookup = Arc::new(DashMap::with_capacity(index_capacity.saturating_mul(2)));

        // Create dialog event channel for subscription manager
        let (event_tx, _) = mpsc::channel(100);

        // Create subscription manager with shared stores
        let subscription_manager =
            SubscriptionManager::new(dialogs.clone(), dialog_lookup.clone(), event_tx);

        let manager = Self {
            transaction_manager,
            local_address,
            config: Arc::new(std::sync::RwLock::new(initial_config)),
            dialogs,
            dialog_lookup,
            early_dialog_lookup: Arc::new(DashMap::with_capacity(index_capacity)),
            terminated_bye_lookup: Arc::new(DashMap::with_capacity(index_capacity)),
            terminated_bye_insert_count: Arc::new(AtomicUsize::new(0)),
            terminated_bye_lookup_hard_max: terminated_bye_lookup_hard_max(index_capacity),
            transaction_to_dialog: Arc::new(DashMap::with_capacity(index_capacity)),
            outbound_transport_by_transaction: Arc::new(DashMap::with_capacity(index_capacity)),
            outbound_transport_by_request_key: Arc::new(DashMap::with_capacity(index_capacity)),
            transaction_dialog_route_hash: Arc::new(DashMap::with_capacity(index_capacity)),
            dialog_invite_transactions: Arc::new(DashMap::with_capacity(index_capacity)),
            invite_failover_plans: Arc::new(DashMap::with_capacity(index_capacity)),
            active_invite_failover_by_dialog: Arc::new(DashMap::with_capacity(index_capacity)),
            invite_failover_attempts: Arc::new(DashMap::with_capacity(
                invite_failover_attempt_capacity(index_capacity),
            )),
            invite_failover_plan_reservations: Arc::new(AtomicUsize::new(0)),
            invite_failover_attempt_reservations: Arc::new(AtomicUsize::new(0)),
            next_invite_failover_plan_id: Arc::new(AtomicU64::new(1)),
            invite_failover_insert_count: Arc::new(AtomicUsize::new(0)),
            invite_failover_plan_capacity: index_capacity,
            invite_failover_attempt_capacity: invite_failover_attempt_capacity(index_capacity),
            dialog_server_transactions: Arc::new(DashMap::with_capacity(index_capacity)),
            pending_response_transaction_by_dialog: Arc::new(DashMap::with_capacity(
                index_capacity,
            )),
            session_to_dialog: Arc::new(DashMap::with_capacity(index_capacity)),
            dialog_to_session: Arc::new(DashMap::with_capacity(index_capacity)),
            event_hub: Arc::new(tokio::sync::RwLock::new(None)),
            session_coordinator: Arc::new(tokio::sync::RwLock::new(None)),
            dialog_event_sender: Arc::new(tokio::sync::RwLock::new(None)),
            dialog_event_receiver: Arc::new(tokio::sync::RwLock::new(None)),
            shutdown_signal: Arc::new(tokio::sync::Notify::new()),
            subscription_manager: Some(Arc::new(subscription_manager)),
            reliable_provisional_tasks: Arc::new(DashMap::with_capacity(index_capacity)),
            session_refresh_tasks: Arc::new(DashMap::with_capacity(index_capacity)),
            nat_discovered_addr: Arc::new(tokio::sync::RwLock::new(None)),
            service_route_by_aor: Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            gruu_by_aor: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            outbound_flows: Arc::new(DashMap::with_capacity(index_capacity)),
            outbound_flow_tasks: Arc::new(DashMap::with_capacity(index_capacity)),
            flow_by_destination: Arc::new(DashMap::with_capacity(index_capacity)),
            flow_by_aor: Arc::new(DashMap::with_capacity(index_capacity)),
            outbound_keepalive_interval: Arc::new(std::sync::RwLock::new(None)),
            identity_verifier: Arc::new(std::sync::RwLock::new(None)),
            identity_signer: Arc::new(std::sync::RwLock::new(None)),
            verification_policy: Arc::new(std::sync::RwLock::new(
                crate::manager::VerificationPolicy::default(),
            )),
            resolver: Arc::new(std::sync::RwLock::new(None)),
        };

        // Spawn global transaction event processor
        let event_processor = manager.clone();
        tokio::spawn(async move {
            event_processor
                .process_global_transaction_events(transaction_events)
                .await;
        });

        // Wire up the RFC 5626 flow-event channel: the transaction
        // manager forwards transport-side pong + connection-closed
        // events into `flow_rx`; a dedicated consumer task then drives
        // the per-flow state machines in the dialog manager. The
        // channel is modest (64) — one event per flow per ping or
        // close, and the consumer is lightweight.
        let (flow_tx, mut flow_rx) =
            mpsc::channel::<crate::manager::outbound_flow::FlowTransportEvent>(64);
        manager
            .transaction_manager
            .set_flow_event_sender(flow_tx)
            .await;
        let flow_consumer = manager.clone();
        tokio::spawn(async move {
            while let Some(event) = flow_rx.recv().await {
                match event {
                    crate::manager::outbound_flow::FlowTransportEvent::PongReceived {
                        source,
                        flow_id,
                    } => {
                        flow_consumer
                            .on_pong_received_on_flow(source, flow_id)
                            .await;
                    }
                    crate::manager::outbound_flow::FlowTransportEvent::ConnectionClosed {
                        remote_addr,
                        flow_id,
                    } => {
                        flow_consumer
                            .on_connection_closed_on_flow(remote_addr, flow_id)
                            .await;
                    }
                }
            }
            debug!("RFC 5626 flow-event consumer channel closed");
        });

        Ok(manager)
    }

    /// Process global transaction events (similar to working transaction-core examples)
    ///
    /// This follows the exact pattern from working examples that use global event consumption
    /// instead of individual transaction subscriptions.
    async fn process_global_transaction_events(
        &self,
        mut events: mpsc::Receiver<TransactionEvent>,
    ) {
        info!("🔄 Starting global transaction event processor for dialog-core");

        let dispatch_workers = self.dialog_event_dispatch_worker_count();
        if dispatch_workers > DEFAULT_DIALOG_EVENT_DISPATCH_WORKERS {
            self.process_global_transaction_events_sharded(
                events,
                dispatch_workers,
                self.dialog_event_dispatch_queue_capacity(),
            )
            .await;
            info!("🏁 Global transaction event processor for dialog-core stopped");
            return;
        }

        let mut maintenance_interval = tokio::time::interval(Duration::from_secs(1));
        maintenance_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                // Process transaction events
                event = events.recv() => {
                    match event {
                        Some(event) => {
                            self.process_timed_global_transaction_event(event).await;
                        },
                        None => {
                            // Channel closed
                            debug!("Transaction events channel closed");
                            break;
                        }
                    }
                },

                _ = maintenance_interval.tick() => {
                    self.prune_terminated_bye_lookup();
                    self.prune_invite_failover_state().await;
                },

                // Wait for shutdown signal
                _ = self.shutdown_signal.notified() => {
                    info!("🛑 Global transaction event processor received shutdown signal");
                    break;
                }
            }
        }

        info!("🏁 Global transaction event processor for dialog-core stopped");
    }

    async fn process_global_transaction_events_sharded(
        &self,
        mut events: mpsc::Receiver<TransactionEvent>,
        worker_count: usize,
        queue_capacity: usize,
    ) {
        let dispatch_senders =
            start_dialog_event_dispatch_workers(self.clone(), worker_count, queue_capacity);
        let fallback_worker = AtomicUsize::new(0);
        let mut maintenance_interval = tokio::time::interval(Duration::from_secs(1));
        maintenance_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                event = events.recv() => {
                    match event {
                        Some(event) => {
                            dispatch_dialog_transaction_event(
                                self,
                                event,
                                &dispatch_senders,
                                &fallback_worker,
                            )
                            .await;
                        }
                        None => {
                            debug!("Transaction events channel closed");
                            break;
                        }
                    }
                }

                _ = maintenance_interval.tick() => {
                    self.prune_terminated_bye_lookup();
                    self.prune_invite_failover_state().await;
                }

                _ = self.shutdown_signal.notified() => {
                    info!("🛑 Sharded global transaction event processor received shutdown signal");
                    break;
                }
            }
        }
    }

    fn dialog_event_dispatch_worker_count(&self) -> usize {
        self.config()
            .and_then(|config| config.dialog_config().event_dispatch_workers)
            .unwrap_or(DEFAULT_DIALOG_EVENT_DISPATCH_WORKERS)
            .clamp(1, super::MAX_DIALOG_EVENT_DISPATCH_WORKERS)
    }

    fn dialog_event_dispatch_queue_capacity(&self) -> usize {
        self.config()
            .and_then(|config| config.dialog_config().event_dispatch_queue_capacity)
            .or_else(|| {
                self.config()
                    .and_then(|config| config.dialog_config().max_dialogs)
            })
            .unwrap_or(10_000)
            .max(1)
    }

    fn dialog_event_route_hash(
        &self,
        event: &TransactionEvent,
    ) -> Option<(u64, DialogRouteSource, bool)> {
        let transaction_id = transaction_event_key(event);

        if let Some(hash) = dialog_event_request_route_hash(event) {
            if let Some(transaction_id) = transaction_id {
                let mismatch = self
                    .transaction_dialog_route_hash
                    .get(transaction_id)
                    .is_some_and(|existing| *existing.value() != hash);
                self.transaction_dialog_route_hash
                    .insert(transaction_id.clone(), hash);
                return Some((hash, DialogRouteSource::Request, mismatch));
            }
            return Some((hash, DialogRouteSource::Request, false));
        }

        if let Some(transaction_id) = transaction_id {
            if let Some(hash) = self
                .transaction_dialog_route_hash
                .get(transaction_id)
                .map(|entry| *entry.value())
            {
                return Some((hash, DialogRouteSource::Stored, false));
            }

            return Some((
                transaction_key_route_hash(transaction_id),
                DialogRouteSource::TransactionKey,
                false,
            ));
        }

        None
    }

    fn dialog_event_dispatch_worker_index(
        &self,
        event: &TransactionEvent,
        worker_count: usize,
        fallback_worker: &AtomicUsize,
    ) -> usize {
        if worker_count <= 1 {
            return 0;
        }

        let route_kind = dialog_event_route_kind(event);
        if let Some((hash, source, mismatch)) = self.dialog_event_route_hash(event) {
            crate::diagnostics::record_dialog_route(source.as_str(), route_kind, mismatch);
            return (hash as usize) % worker_count;
        }

        crate::diagnostics::record_dialog_route(
            DialogRouteSource::Fallback.as_str(),
            route_kind,
            false,
        );
        fallback_worker.fetch_add(1, Ordering::Relaxed) % worker_count
    }

    async fn process_timed_global_transaction_event(&self, event: TransactionEvent) {
        let timing_enabled = crate::diagnostics::dialog_timing_enabled();
        let kind = timing_enabled.then(|| dialog_event_kind(&event));
        let started = timing_enabled.then(Instant::now);
        self.process_global_transaction_event(event).await;
        if let Some(started) = started {
            crate::diagnostics::record_dialog_event_handler(
                kind.expect("timed dialog transaction event kind").as_str(),
                started.elapsed(),
            );
        }
    }

    async fn process_global_transaction_event(&self, event: TransactionEvent) {
        match &event {
            TransactionEvent::StateChanged {
                transaction_id,
                new_state: TransactionState::Terminated,
                ..
            }
            | TransactionEvent::TransactionTerminated { transaction_id } => {
                self.transaction_manager
                    .mark_transaction_terminated_indexed(transaction_id);
            }
            _ => {}
        }

        if matches!(
            self.handle_invite_failover_event(&event).await,
            super::transaction_integration::InviteFailoverEventDisposition::Consumed
        ) {
            return;
        }

        if matches!(
            &event,
            TransactionEvent::StateChanged { new_state, .. }
                if *new_state != TransactionState::Terminated
        ) {
            return;
        }

        let clear_route_after_processing = matches!(
            &event,
            TransactionEvent::TransactionTerminated { .. }
                | TransactionEvent::StateChanged {
                    new_state: TransactionState::Terminated,
                    ..
                }
                | TransactionEvent::AckReceived { .. }
                | TransactionEvent::AckRequest { .. }
        );

        // Extract transaction ID from the event
        let transaction_id = self.extract_transaction_id(&event);

        let lookup_started = crate::diagnostics::dialog_timing_enabled().then(Instant::now);
        let dialog_id = self.find_dialog_for_transaction_event(&transaction_id);
        if let Some(started) = lookup_started {
            crate::diagnostics::record_dialog_lookup(started.elapsed());
        }

        // Find the dialog associated with this transaction
        if let Some(dialog_id) = dialog_id {
            if let Err(_error) = self
                .process_transaction_event(&transaction_id, &dialog_id, event)
                .await
            {
                error!(
                    "Failed to process transaction event for dialog {}",
                    dialog_id
                );
            }
        } else {
            // No dialog found using transaction-to-dialog mapping

            // Special handling for AckReceived events: use dialog-based matching
            if let TransactionEvent::AckReceived { request, .. } = &event {
                // Find dialog using Call-ID, From tag, To tag from the ACK request
                let lookup_started = crate::diagnostics::dialog_timing_enabled().then(Instant::now);
                let dialog_id = self.find_dialog_for_request(request).await;
                if let Some(started) = lookup_started {
                    crate::diagnostics::record_dialog_lookup(started.elapsed());
                }
                if let Some(dialog_id) = dialog_id {
                    if let Err(_error) = self
                        .process_transaction_event(&transaction_id, &dialog_id, event)
                        .await
                    {
                        error!(
                            "Failed to process AckReceived event for dialog {}",
                            dialog_id
                        );
                    }
                } else {
                    // Still treat as unassociated event
                    if let Err(_error) = self
                        .handle_unassociated_transaction_event(&transaction_id, event)
                        .await
                    {
                        error!("Failed to handle unassociated AckReceived event");
                    }
                }
            } else {
                // Event for transaction not associated with any dialog
                // Check if this is a new incoming INVITE that should create a dialog
                if let Err(_error) = self
                    .handle_unassociated_transaction_event(&transaction_id, event)
                    .await
                {
                    error!("Failed to handle unassociated transaction event");
                }
            }
        }

        if clear_route_after_processing {
            self.transaction_dialog_route_hash.remove(&transaction_id);
        }
    }

    /// Extract transaction ID from any TransactionEvent variant
    fn extract_transaction_id(&self, event: &TransactionEvent) -> TransactionKey {
        match event {
            TransactionEvent::AckReceived { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::CancelReceived { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::ProvisionalResponse { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::SuccessResponse { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::FailureResponse { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::ProvisionalResponseSent { transaction_id, .. } => {
                transaction_id.clone()
            }
            TransactionEvent::FinalResponseSent { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::TransactionTimeout { transaction_id } => transaction_id.clone(),
            TransactionEvent::AckTimeout { transaction_id } => transaction_id.clone(),
            TransactionEvent::TransportError { transaction_id } => transaction_id.clone(),
            TransactionEvent::Error { transaction_id, .. } => transaction_id
                .clone()
                .unwrap_or_else(|| TransactionKey::new("unknown".to_string(), Method::Info, false)),
            TransactionEvent::TransactionTerminated { transaction_id } => transaction_id.clone(),
            TransactionEvent::StateChanged { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::TimerTriggered { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::CancelRequest { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::AckRequest { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::InviteRequest { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::NonInviteRequest { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::StrayRequest { .. } => {
                TransactionKey::new("stray".to_string(), Method::Info, false)
            }
            TransactionEvent::StrayResponse { .. } => {
                TransactionKey::new("stray".to_string(), Method::Info, false)
            }
            TransactionEvent::StrayAck { .. } => {
                TransactionKey::new("stray".to_string(), Method::Info, false)
            }
            TransactionEvent::StrayCancel { .. } => {
                TransactionKey::new("stray".to_string(), Method::Info, false)
            }
            TransactionEvent::StrayAckRequest { .. } => {
                TransactionKey::new("stray".to_string(), Method::Info, false)
            }

            // Shutdown events don't have transaction IDs
            TransactionEvent::ShutdownRequested
            | TransactionEvent::ShutdownReady
            | TransactionEvent::ShutdownNow
            | TransactionEvent::ShutdownComplete => {
                TransactionKey::new("shutdown".to_string(), Method::Info, false)
            }
        }
    }

    /// Find dialog associated with a transaction event
    fn find_dialog_for_transaction_event(
        &self,
        transaction_id: &TransactionKey,
    ) -> Option<DialogId> {
        self.transaction_to_dialog
            .get(transaction_id)
            .map(|entry| entry.clone())
    }

    pub(crate) fn link_transaction_to_dialog_indexed(
        &self,
        transaction_id: &TransactionKey,
        dialog_id: &DialogId,
    ) {
        self.transaction_to_dialog
            .insert(transaction_id.clone(), dialog_id.clone());

        if transaction_id.method() == &Method::Invite {
            let mut invite_transactions = self
                .dialog_invite_transactions
                .entry(dialog_id.clone())
                .or_insert_with(Vec::new);
            if !invite_transactions
                .iter()
                .any(|existing| existing == transaction_id)
            {
                invite_transactions.push(transaction_id.clone());
            }
        }

        if transaction_id.is_server() {
            let mut server_transactions = self
                .dialog_server_transactions
                .entry(dialog_id.clone())
                .or_insert_with(Vec::new);
            if !server_transactions
                .iter()
                .any(|existing| existing == transaction_id)
            {
                server_transactions.push(transaction_id.clone());
            }
        }
    }

    pub(crate) fn link_outbound_transaction_to_dialog_indexed(
        &self,
        transaction_id: &TransactionKey,
        dialog_id: &DialogId,
        request: &Request,
    ) {
        self.link_transaction_to_dialog_indexed(transaction_id, dialog_id);
        if let Some(route_hash) = request_dialog_route_hash(request) {
            self.transaction_dialog_route_hash
                .insert(transaction_id.clone(), route_hash);
        }
    }

    /// Record the transport context for an outbound request after the
    /// transaction manager accepted `send_request`. This is the point where
    /// RFC 3263 candidate selection and local transport availability have
    /// converged, so auth policy can later decide whether Basic/Bearer retry
    /// headers are allowed on the actual hop used for the challenged request.
    pub(crate) fn record_outbound_transport_context(
        &self,
        transaction_id: &TransactionKey,
        request_key: Option<String>,
        transport: TransportType,
        remote_addr: SocketAddr,
    ) {
        if self.outbound_transport_by_request_key.len() > 8192 {
            self.outbound_transport_by_request_key.clear();
        }

        let local_addr = self
            .transaction_manager
            .get_transport_info(transport)
            .and_then(|info| info.local_addr)
            .unwrap_or(self.local_address);
        let context = SipTransportContext::new(
            transport.to_string(),
            local_addr.to_string(),
            remote_addr.to_string(),
            matches!(transport, TransportType::Tls | TransportType::Wss),
        );

        self.outbound_transport_by_transaction
            .insert(transaction_id.clone(), context.clone());
        if let Some(key) = request_key {
            self.outbound_transport_by_request_key.insert(key, context);
        }
    }

    /// Transport context recorded for an outbound transaction, if available.
    pub fn outbound_transport_context_for_transaction(
        &self,
        transaction_id: &TransactionKey,
    ) -> Option<SipTransportContext> {
        self.outbound_transport_by_transaction
            .get(transaction_id)
            .map(|entry| entry.value().clone())
    }

    /// Transport context recorded for the request matched by a response's
    /// `Call-ID` and `CSeq`.
    pub fn outbound_transport_context_for_response(
        &self,
        response: &Response,
    ) -> Option<SipTransportContext> {
        outbound_response_key(response).and_then(|key| {
            self.outbound_transport_by_request_key
                .get(&key)
                .map(|entry| entry.value().clone())
        })
    }

    pub(crate) fn unlink_transaction_from_dialog_indexed(
        &self,
        transaction_id: &TransactionKey,
    ) -> Option<DialogId> {
        self.transaction_dialog_route_hash.remove(transaction_id);
        self.outbound_transport_by_transaction
            .remove(transaction_id);
        let removed_dialog_id = self
            .transaction_to_dialog
            .remove(transaction_id)
            .map(|(_, dialog_id)| dialog_id);

        if let Some(dialog_id) = removed_dialog_id.as_ref() {
            self.remove_dialog_invite_transaction(dialog_id, transaction_id);
            self.remove_dialog_server_transaction(dialog_id, transaction_id);
        }

        removed_dialog_id
    }

    fn remove_dialog_invite_transaction(
        &self,
        dialog_id: &DialogId,
        transaction_id: &TransactionKey,
    ) {
        if transaction_id.method() != &Method::Invite {
            return;
        }

        if let Some(mut entry) = self.dialog_invite_transactions.get_mut(dialog_id) {
            entry.value_mut().retain(|tx_id| tx_id != transaction_id);
        }
        self.dialog_invite_transactions
            .remove_if(dialog_id, |_, tx_ids| tx_ids.is_empty());
    }

    fn remove_dialog_server_transaction(
        &self,
        dialog_id: &DialogId,
        transaction_id: &TransactionKey,
    ) {
        if !transaction_id.is_server() {
            return;
        }

        if let Some(mut entry) = self.dialog_server_transactions.get_mut(dialog_id) {
            entry.value_mut().retain(|tx_id| tx_id != transaction_id);
        }
        self.dialog_server_transactions
            .remove_if(dialog_id, |_, tx_ids| tx_ids.is_empty());
    }

    /// Handle transaction events not associated with any existing dialog
    ///
    /// This handles new incoming requests that should create dialogs.
    async fn handle_unassociated_transaction_event(
        &self,
        transaction_id: &TransactionKey,
        event: TransactionEvent,
    ) -> DialogResult<()> {
        match event {
            TransactionEvent::InviteRequest {
                request, source, ..
            } => {
                // RFC 3261 §14: an INVITE on an existing dialog is a
                // re-INVITE. Every inbound INVITE spins up a fresh server
                // transaction, so the transaction-to-dialog mapping is
                // always empty at this point. We must dialog-match on
                // (Call-ID, From-tag, To-tag) before falling through to
                // initial INVITE handling. Same pattern as the REFER arm
                // below.
                if let Some(dialog_id) = self.find_dialog_for_request(&request).await {
                    debug!(
                        "INVITE request belongs to existing dialog {} — treating as re-INVITE",
                        dialog_id
                    );
                    self.handle_reinvite(transaction_id.clone(), request, dialog_id)
                        .await?;
                    return Ok(());
                }

                tracing::debug!(
                    "🎯 FOUND UNASSOCIATED INVITE: Processing new incoming INVITE from {}",
                    source
                );
                debug!("Processing new incoming INVITE request from transaction");

                // This is a new incoming INVITE - create dialog and process it
                self.handle_initial_invite(transaction_id.clone(), request, source)
                    .await?;

                debug!("Successfully processed new incoming INVITE from {}", source);
                Ok(())
            }

            TransactionEvent::NonInviteRequest {
                request, source, ..
            } => {
                debug!(
                    "Processing new incoming {} request from transaction",
                    method_class(&request.method())
                );

                if request.method() == Method::Bye {
                    return self
                        .handle_bye_with_transaction(transaction_id.clone(), request)
                        .await;
                }
                if request.method() == Method::Cancel {
                    let invite_tx_id = self
                        .transaction_manager
                        .find_invite_server_transaction_for_cancel(&request)
                        .await
                        .map_err(|_error| DialogError::TransactionError {
                            message: "Failed to find INVITE server transaction for CANCEL"
                                .to_string(),
                        })?;

                    if let Some(invite_tx_id) = invite_tx_id {
                        return self
                            .handle_cancel_request_event(transaction_id, &invite_tx_id, request)
                            .await;
                    }

                    let response = crate::transaction::utils::response_builders::create_response(
                        &request,
                        rvoip_sip_core::StatusCode::CallOrTransactionDoesNotExist,
                    );
                    self.transaction_manager
                        .send_response(transaction_id, response)
                        .await
                        .map_err(|_error| DialogError::TransactionError {
                            message: "Failed to send 481 response to CANCEL".to_string(),
                        })?;
                    let _ = self
                        .transaction_manager
                        .terminate_transaction(transaction_id)
                        .await;
                    return Ok(());
                }

                // For REFER requests, check if they belong to an existing dialog
                if request.method() == Method::Refer {
                    // Try to find the dialog using Call-ID, From tag, and To tag
                    if let Some(dialog_id) = self.find_dialog_for_request(&request).await {
                        debug!("REFER request belongs to existing dialog {}", dialog_id);

                        // Store the transaction-to-dialog mapping
                        self.link_transaction_to_dialog_indexed(transaction_id, &dialog_id);

                        // REFER within a dialog should be handled by the protocol handler
                        // which will emit the TransferRequest event to session-core
                        return self.handle_refer(request, source).await;
                    } else {
                        debug!("REFER request does not match any existing dialog");
                    }
                }

                // Handle non-INVITE requests (REGISTER, OPTIONS, etc.) or REFER without dialog
                self.handle_request(request, source).await
            }

            // UAS-side CANCEL. The transaction manager emits this when an
            // inbound CANCEL finds a matching INVITE server transaction.
            // The CANCEL request itself has no dialog mapping of its own,
            // so it arrives here as "unassociated" — route it to the
            // protocol handler so we send 200 OK to CANCEL, 487 to the
            // pending INVITE, and terminate the dialog.
            TransactionEvent::CancelRequest {
                request,
                target_transaction_id,
                ..
            } => {
                debug!("Processing unassociated CANCEL request from transaction");
                self.handle_cancel_request_event(transaction_id, &target_transaction_id, request)
                    .await
            }

            _ => {
                // Other unassociated events (responses, timeouts, etc.) - just log them
                debug!(
                    "Received unassociated transaction event class={}",
                    dialog_event_kind(&event).as_str()
                );
                Ok(())
            }
        }
    }

    async fn handle_cancel_request_event(
        &self,
        cancel_tx_id: &TransactionKey,
        invite_tx_id: &TransactionKey,
        request: Request,
    ) -> DialogResult<()> {
        let ok = crate::transaction::utils::response_builders::create_response(
            &request,
            rvoip_sip_core::StatusCode::Ok,
        );
        self.transaction_manager
            .send_response(cancel_tx_id, ok)
            .await
            .map_err(|_error| DialogError::TransactionError {
                message: "Failed to send 200 OK to CANCEL".to_string(),
            })?;
        let _ = self
            .transaction_manager
            .terminate_transaction(cancel_tx_id)
            .await;

        let original_invite = self
            .transaction_manager
            .get_server_transaction_request(invite_tx_id)
            .await
            .map_err(|_error| DialogError::TransactionError {
                message: "Failed to fetch pending INVITE for 487".to_string(),
            })?;
        let terminated = crate::transaction::utils::response_builders::create_response(
            &original_invite,
            rvoip_sip_core::StatusCode::RequestTerminated,
        );
        self.transaction_manager
            .send_response(invite_tx_id, terminated)
            .await
            .map_err(|_error| DialogError::TransactionError {
                message: "Failed to send 487 Request Terminated".to_string(),
            })?;

        self.terminate_dialog_for_tx_and_emit_cancelled(invite_tx_id, "CANCEL received")
            .await;

        debug!("CANCEL processed for INVITE server transaction (200 CANCEL, 487 INVITE sent)");
        Ok(())
    }

    /// Get the configured local address
    ///
    /// Returns the bind address that this DialogManager uses for sockets.
    /// Outbound Via sent-by and fallback Contact generation should use
    /// [`Self::local_address_for_uri`] so configured advertised addresses
    /// are honored.
    pub fn local_address(&self) -> SocketAddr {
        self.local_address
    }

    /// Configured SIP advertised sent-by address, if supplied.
    pub fn advertised_local_address(&self) -> Option<SocketAddr> {
        self.config
            .read()
            .ok()
            .and_then(|g| g.as_ref().and_then(|c| c.advertised_local_address()))
    }

    /// Configured local Contact URI, if the application supplied one.
    pub fn local_contact_uri(&self) -> Option<String> {
        self.config.read().ok().and_then(|g| {
            g.as_ref()
                .and_then(|c| c.local_contact_uri().map(str::to_string))
        })
    }

    /// Configured SIP TLS local address, if the application supplied
    /// one.
    pub fn tls_local_address(&self) -> Option<SocketAddr> {
        self.config
            .read()
            .ok()
            .and_then(|g| g.as_ref().and_then(|c| c.tls_local_address()))
    }

    /// Configured SIP TLS advertised sent-by address, if supplied.
    pub fn tls_advertised_local_address(&self) -> Option<SocketAddr> {
        self.config
            .read()
            .ok()
            .and_then(|g| g.as_ref().and_then(|c| c.tls_advertised_local_address()))
    }

    /// Local sent-by address for an outbound request targeting `uri`.
    /// TLS requests prefer the configured TLS advertised address, then the
    /// TLS bind address, then the base bind address. Other transports prefer
    /// the configured SIP advertised address, then the base bind address.
    pub fn local_address_for_uri(&self, uri: &Uri) -> SocketAddr {
        if select_transport_for_uri(uri) == TransportType::Tls {
            self.tls_advertised_local_address()
                .or_else(|| self.tls_local_address())
                .unwrap_or(self.local_address)
        } else {
            self.advertised_local_address()
                .unwrap_or(self.local_address)
        }
    }

    /// Advertised sent-by address for a resolver-selected transport
    /// candidate. Candidate failover can change the transport selected from
    /// the same SIP URI, so Via and stack-generated Contact values must be
    /// planned from the actual candidate rather than the URI default.
    pub fn local_address_for_transport(&self, transport: TransportType) -> SocketAddr {
        if matches!(transport, TransportType::Tls | TransportType::Wss) {
            self.tls_advertised_local_address()
                .or_else(|| self.tls_local_address())
                .unwrap_or(self.local_address)
        } else {
            self.advertised_local_address()
                .unwrap_or(self.local_address)
        }
    }

    /// Local sent-by address for an outbound request with an optional route
    /// set. The top Route URI is the next hop when present; otherwise the
    /// Request-URI determines the transport and advertised sent-by address.
    pub fn local_address_for_target_and_routes(
        &self,
        target_uri: &Uri,
        route_set: &[Uri],
    ) -> SocketAddr {
        self.local_address_for_uri(route_set.first().unwrap_or(target_uri))
    }

    // REMOVED: set_session_coordinator() - Use GlobalEventCoordinator instead
    // REMOVED: set_dialog_event_sender() - Use GlobalEventCoordinator instead
    // REMOVED: setup_dialog_event_channel() - Use GlobalEventCoordinator instead
    // REMOVED: process_dialog_events() and handle_shutdown_requested() - Use GlobalEventCoordinator instead
    // REMOVED: subscribe_to_dialog_events() - Use GlobalEventCoordinator instead

    /// Emit a dialog event to external consumers
    ///
    /// Sends dialog events to session-core for high-level dialog state management.
    /// This maintains the proper architectural separation where dialog-core handles
    /// SIP protocol details and session-core handles session logic.
    pub async fn emit_dialog_event(&self, event: DialogEvent) {
        // Try event hub first (new global event bus)
        let hub = self.event_hub.read().await.clone();
        if let Some(hub) = hub {
            if let Err(_error) = hub.publish_dialog_event(event.clone()).await {
                warn!("Failed to publish dialog event to global bus");
            } else {
                debug!("Published dialog event to global bus");
                return;
            }
        }

        // Fall back to channel (legacy)
        let sender = self.dialog_event_sender.read().await.clone();
        if let Some(sender) = sender {
            if let Err(_error) = sender.send(event.clone()).await {
                warn!("Failed to send dialog event to session-core");
            } else {
                debug!("Emitted dialog event");
            }
        }
    }

    /// Emit a session coordination event
    ///
    /// Sends session coordination events for legacy compatibility and specific
    /// session management operations.
    pub async fn emit_session_coordination_event(&self, event: SessionCoordinationEvent) {
        let timing_enabled = crate::diagnostics::dialog_timing_enabled();
        let publish_kind = timing_enabled.then(|| session_coordination_event_kind(&event));
        let publish_started = timing_enabled.then(Instant::now);
        trace!(
            "emit_session_coordination_event called with class={}",
            session_coordination_event_kind(&event)
        );

        // Try event hub first (new global event bus)
        let hub = self.event_hub.read().await.clone();
        if let Some(hub) = hub {
            trace!("Event hub exists, publishing session coordination event");
            if let Err(_error) = hub.publish_session_coordination_event(event.clone()).await {
                warn!("Failed to publish session coordination event to global bus");
            } else {
                trace!("Published session coordination event to global bus");
                if let Some(started) = publish_started {
                    crate::diagnostics::record_dialog_session_publish(
                        publish_kind.expect("timed session coordination event kind"),
                        started.elapsed(),
                    );
                }
                return;
            }
        } else {
            trace!("Event hub is None, trying legacy session channel");
        }

        // Fall back to channel (legacy)
        let sender = self.session_coordinator.read().await.clone();
        if let Some(sender) = sender {
            trace!("Legacy session channel exists, sending event");
            if let Err(_error) = sender.send(event.clone()).await {
                warn!("Failed to send session coordination event");
            } else {
                trace!("Emitted session coordination event to legacy channel");
            }
        } else {
            warn!("Both event hub and legacy channel are None - event not sent");
        }
        if let Some(started) = publish_started {
            crate::diagnostics::record_dialog_session_publish(
                publish_kind.expect("timed session coordination event kind"),
                started.elapsed(),
            );
        }
    }

    /// Try to emit a session coordination event and report whether any session
    /// consumer path accepted it. This is intentionally narrower than
    /// `emit_session_coordination_event`: protocol handlers that need a
    /// definite answer can use this to choose a local fallback, while existing
    /// fire-and-forget event paths keep their current behavior.
    pub(crate) async fn try_emit_session_coordination_event(
        &self,
        event: SessionCoordinationEvent,
    ) -> DialogResult<bool> {
        // Try the legacy in-process session_coordinator first — it
        // signals "definite consumer" because the receiver is held by
        // the application (rather than the event-hub fan-out which can
        // succeed even when no subscriber is listening). This is the
        // path the OPTIONS-fallback test relies on: in test setups
        // without a session_coordinator wired, the protocol handler
        // must observe `false` here and emit a basic 200 OK locally.
        let mut delivered = false;
        let sender = self.session_coordinator.read().await.clone();
        if let Some(sender) = sender {
            match sender.send(event.clone()).await {
                Ok(()) => delivered = true,
                Err(_error) => {
                    warn!("Failed to send session coordination event");
                }
            }
        }

        // Best-effort fan-out via the event hub. Success here is not
        // sufficient to claim "consumer exists" because the global bus
        // accepts publishes whether or not any subscriber is wired.
        let hub = self.event_hub.read().await.clone();
        if let Some(hub) = hub {
            match hub
                .try_publish_session_coordination_event(event.clone())
                .await
            {
                Ok(true) | Ok(false) => {
                    // either mapped or not; either way the in-process
                    // delivered flag above is the authoritative signal.
                }
                Err(_error) => {
                    warn!("Failed to publish session coordination event to global bus");
                }
            }
        }

        Ok(delivered)
    }

    /// **CENTRAL DISPATCHER**: Handle incoming SIP messages
    ///
    /// This is the main entry point for processing SIP messages in dialog-core.
    /// It routes messages to the appropriate method-specific handlers while maintaining
    /// RFC 3261 compliance for dialog state management.
    ///
    /// # Arguments
    /// * `message` - The SIP message (Request or Response)
    /// * `source` - Source address of the message
    ///
    /// # Returns
    /// Result indicating success or the specific error encountered
    pub async fn handle_message(
        &self,
        message: rvoip_sip_core::Message,
        source: SocketAddr,
    ) -> DialogResult<()> {
        match message {
            rvoip_sip_core::Message::Request(request) => self.handle_request(request, source).await,
            rvoip_sip_core::Message::Response(_response) => {
                // For responses, we need the transaction ID to route properly
                // This would typically come from the transaction layer
                warn!("Response handling requires transaction ID - use handle_response() directly");
                Err(DialogError::protocol_error(
                    "Response handling requires transaction context",
                ))
            }
        }
    }

    /// Handle incoming SIP requests
    ///
    /// Routes requests to appropriate method handlers based on the SIP method.
    /// Implements RFC 3261 Section 12 dialog handling requirements.
    ///
    /// # Arguments
    /// * `request` - The SIP request to handle
    /// * `source` - Source address of the request
    async fn handle_request(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        debug!(
            "Handling {} request from {}",
            method_class(&request.method()),
            source
        );

        // Dispatch request to appropriate handler based on method
        match request.method() {
            Method::Invite => self.handle_invite(request, source).await,
            Method::Bye => self.handle_bye(request).await,
            Method::Cancel => self.handle_cancel(request).await,
            Method::Ack => self.handle_ack(request).await,
            Method::Options => self.handle_options(request, source).await,
            Method::Register => self.handle_register(request, source).await,
            Method::Update => self.handle_update(request).await,
            Method::Info => self.handle_info(request, source).await,
            Method::Refer => self.handle_refer(request, source).await,
            Method::Subscribe => self.handle_subscribe(request, source).await,
            Method::Notify => self.handle_notify(request, source).await,
            Method::Prack => self.handle_prack(request).await,
            Method::Message => {
                // RFC 3428 — MESSAGE is a fire-and-forget transport that
                // dialog-core does not parse for application semantics.
                // Reply with a basic 200 OK so the transaction settles
                // and the application can observe the wire bytes via
                // the SIP trace channel. Out-of-dialog MESSAGE creates
                // no dialog state per RFC 3428 §4.
                debug!("Replying 200 OK to inbound MESSAGE from {}", source);
                let server_transaction = self
                    .transaction_manager
                    .create_server_transaction(request.clone(), source)
                    .await
                    .map_err(|_error| DialogError::TransactionError {
                        message: "Failed to create server transaction for MESSAGE".to_string(),
                    })?;
                let transaction_id = server_transaction.id().clone();
                let response = crate::transaction::utils::response_builders::create_response(
                    &request,
                    rvoip_sip_core::StatusCode::Ok,
                );
                if let Err(_error) = self
                    .transaction_manager
                    .send_response(&transaction_id, response)
                    .await
                {
                    debug!("Failed to send 200 OK for MESSAGE");
                }
                Ok(())
            }
            method => {
                // Demoted from warn — under the test harness this fires
                // for every method we haven't implemented yet (e.g.
                // PUBLISH); spurious error-level logs make pass output
                // noisier than the failure they're supposed to flag.
                debug!("Unsupported SIP method class={}", method_class(&method));
                Err(DialogError::protocol_error(&format!(
                    "Unsupported method class: {}",
                    method_class(&method)
                )))
            }
        }
    }

    /// Start the dialog manager
    ///
    /// Initializes the dialog manager for processing. This can include starting
    /// background tasks for dialog cleanup, recovery, and maintenance.
    pub async fn start(&self) -> DialogResult<()> {
        info!("DialogManager starting");

        // TODO: Start background processing tasks (cleanup, recovery, etc.)
        // - Dialog timeout monitoring
        // - Orphaned dialog cleanup
        // - Recovery coordination
        // - Statistics collection

        info!("DialogManager started successfully");
        Ok(())
    }

    /// Stop the dialog manager
    ///
    /// Gracefully shuts down the dialog manager in BOTTOM-UP order
    /// This is called when receiving ShutdownNow("DialogManager") event
    ///
    /// Shutdown order (bottom-up):
    /// 1. Shutdown transaction manager (which has already stopped transport)
    /// 2. Signal global event processor to stop
    /// 3. Terminate any remaining dialogs
    /// 4. Clear internal state
    /// 5. Report completion via event
    pub async fn stop(&self) -> DialogResult<()> {
        info!("DialogManager stopping gracefully - responding to shutdown event");

        // Step 0: Abort all RFC 5626 outbound-flow monitor tasks so
        // they don't try to emit `OutboundFlowFailed` against a
        // transport that's about to be torn down.
        let flow_keys: Vec<(String, u32, String)> = self
            .outbound_flow_tasks
            .iter()
            .map(|e| e.key().clone())
            .collect();
        for key in flow_keys {
            self.stop_outbound_ping(&key);
        }

        // Step 1: Shutdown the transaction manager
        // Note: Transport should already be stopped by now via events
        info!("Shutting down transaction manager...");
        self.transaction_manager.shutdown().await;
        debug!("Transaction manager shut down");

        // Step 2: Signal shutdown to global event processor
        self.shutdown_signal.notify_one();
        debug!("Sent shutdown signal to global event processor");

        // Give event processor time to process final messages
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Step 3: Now terminate any remaining dialogs
        let dialog_ids: Vec<DialogId> = self
            .dialogs
            .iter()
            .map(|entry| entry.key().clone())
            .collect();

        if !dialog_ids.is_empty() {
            debug!("Found {} remaining dialogs to clean up", dialog_ids.len());
            for dialog_id in dialog_ids {
                if let Some(_) = self.dialogs.remove(&dialog_id) {
                    debug!("Removed dialog {}", dialog_id);
                }
            }
        }

        // Step 4: Clear all mappings
        self.dialogs.clear();
        self.dialog_lookup.clear();
        self.early_dialog_lookup.clear();
        self.terminated_bye_lookup.clear();
        self.terminated_bye_insert_count.store(0, Ordering::Relaxed);
        self.transaction_to_dialog.clear();
        self.outbound_transport_by_transaction.clear();
        self.outbound_transport_by_request_key.clear();
        self.transaction_dialog_route_hash.clear();
        self.dialog_invite_transactions.clear();
        self.invite_failover_plans.clear();
        self.active_invite_failover_by_dialog.clear();
        self.invite_failover_attempts.clear();
        self.invite_failover_plan_reservations
            .store(0, Ordering::Relaxed);
        self.invite_failover_attempt_reservations
            .store(0, Ordering::Relaxed);
        self.invite_failover_insert_count
            .store(0, Ordering::Relaxed);
        self.dialog_server_transactions.clear();
        self.pending_response_transaction_by_dialog.clear();
        self.session_to_dialog.clear();
        self.dialog_to_session.clear();
        for entry in self.reliable_provisional_tasks.iter() {
            entry.value().abort();
        }
        self.reliable_provisional_tasks.clear();
        for entry in self.session_refresh_tasks.iter() {
            entry.value().abort();
        }
        self.session_refresh_tasks.clear();
        self.outbound_flows.clear();
        self.outbound_flow_tasks.clear();
        self.flow_by_destination.clear();
        self.flow_by_aor.clear();

        // Step 5: Report completion
        // Since we're in dialog-core, we emit DialogEvent::ShutdownComplete
        self.emit_dialog_event(DialogEvent::ShutdownComplete).await;

        info!("DialogManager stopped successfully");
        Ok(())
    }

    /// Get the transaction manager reference
    ///
    /// Provides access to the underlying transaction manager for cases where
    /// direct transaction operations are needed.
    pub fn transaction_manager(&self) -> &Arc<TransactionManager> {
        &self.transaction_manager
    }

    /// Get dialog count
    ///
    /// Returns the current number of active dialogs.
    pub fn dialog_count(&self) -> usize {
        self.dialogs.len()
    }

    /// Return retained dialog-manager state counts for perf leak gates.
    ///
    /// This prunes expired BYE tombstones first so idle post-drain samples
    /// reflect live retransmission protection, not expired cache residue.
    pub fn retention_counts(&self) -> DialogManagerRetentionCounts {
        self.prune_terminated_bye_lookup();

        DialogManagerRetentionCounts {
            dialogs: self.dialogs.len(),
            dialog_lookup: self.dialog_lookup.len(),
            early_dialog_lookup: self.early_dialog_lookup.len(),
            terminated_bye_lookup: self.terminated_bye_lookup.len(),
            transaction_to_dialog: self.transaction_to_dialog.len(),
            transaction_dialog_route_hash: self.transaction_dialog_route_hash.len(),
            dialog_invite_transactions: self.dialog_invite_transactions.len(),
            invite_failover_plans: self.invite_failover_plans.len(),
            active_invite_failover_by_dialog: self.active_invite_failover_by_dialog.len(),
            invite_failover_attempts: self.invite_failover_attempts.len(),
            invite_failover_plan_reservations: self
                .invite_failover_plan_reservations
                .load(Ordering::Acquire),
            invite_failover_attempt_reservations: self
                .invite_failover_attempt_reservations
                .load(Ordering::Acquire),
            dialog_server_transactions: self.dialog_server_transactions.len(),
            pending_response_transaction_by_dialog: self
                .pending_response_transaction_by_dialog
                .len(),
            session_to_dialog: self.session_to_dialog.len(),
            dialog_to_session: self.dialog_to_session.len(),
            reliable_provisional_tasks: self.reliable_provisional_tasks.len(),
            session_refresh_tasks: self.session_refresh_tasks.len(),
            outbound_flows: self.outbound_flows.len(),
            outbound_flow_tasks: self.outbound_flow_tasks.len(),
            flow_by_destination: self.flow_by_destination.len(),
            flow_by_aor: self.flow_by_aor.len(),
        }
    }

    /// Check if a dialog exists
    ///
    /// # Arguments
    /// * `dialog_id` - The dialog ID to check
    ///
    /// # Returns
    /// true if the dialog exists, false otherwise
    pub fn has_dialog(&self, dialog_id: &DialogId) -> bool {
        self.dialogs.contains_key(dialog_id)
    }

    /// Remove a completed dialog and all hot lookup/index entries.
    ///
    /// This is intended for upper layers that already emitted a terminal
    /// application event and are releasing per-call resources. It is
    /// idempotent: if dialog-core already removed the storage on an inbound
    /// BYE cleanup path, the method returns `false`.
    pub fn cleanup_dialog_storage(&self, dialog_id: &DialogId) -> bool {
        self.remove_dialog_storage(dialog_id).is_some()
    }

    /// Remove a dialog and force-release server transactions indexed to it.
    ///
    /// Session-level terminal cleanup can run after dialog-core has stopped
    /// making progress on a setup or teardown transaction. Snapshot the
    /// transaction indexes before removing dialog storage so those transactions
    /// can be woken and removed instead of becoming unowned transaction-runner
    /// tasks.
    pub async fn cleanup_dialog_storage_and_transactions(&self, dialog_id: &DialogId) -> bool {
        let mut transaction_ids = Vec::new();

        if let Some(transaction_id) = self.pending_response_transaction_for_dialog(dialog_id) {
            transaction_ids.push(transaction_id);
        }
        if let Some(entry) = self.dialog_server_transactions.get(dialog_id) {
            transaction_ids.extend(entry.value().iter().cloned());
        }
        if let Some(entry) = self.dialog_invite_transactions.get(dialog_id) {
            transaction_ids.extend(entry.value().iter().cloned());
        }

        let mut seen = HashSet::new();
        transaction_ids.retain(|transaction_id| seen.insert(transaction_id.clone()));

        for transaction_id in transaction_ids {
            if let Err(_error) = self
                .transaction_manager
                .terminate_transaction(&transaction_id)
                .await
            {
                debug!("cleanup_dialog_storage_and_transactions: transaction was already gone");
            }
            self.cleanup_transaction_receiver(&transaction_id);
            self.transaction_manager
                .remove_invite_2xx_response_cache(&transaction_id);
        }

        self.remove_dialog_storage(dialog_id).is_some()
    }

    /// Clean up completed transaction event receivers
    ///
    /// This method removes transaction-to-dialog mappings for completed transactions.
    ///
    /// # Arguments
    /// * `transaction_id` - The transaction ID to clean up
    pub fn cleanup_transaction_receiver(&self, transaction_id: &TransactionKey) {
        // Remove from transaction-to-dialog mapping if present
        if self
            .unlink_transaction_from_dialog_indexed(transaction_id)
            .is_some()
        {
            debug!("Cleaned up transaction-dialog mapping for completed transaction");
        }
    }

    pub(crate) fn pending_response_transaction_for_dialog(
        &self,
        dialog_id: &DialogId,
    ) -> Option<TransactionKey> {
        self.pending_response_transaction_by_dialog
            .get(dialog_id)
            .map(|entry| entry.value().clone())
    }

    pub(crate) fn server_transactions_for_dialog(
        &self,
        dialog_id: &DialogId,
    ) -> Vec<TransactionKey> {
        self.dialog_server_transactions
            .get(dialog_id)
            .map(|entry| entry.value().clone())
            .unwrap_or_default()
    }

    pub(crate) fn clear_pending_response_transaction(
        &self,
        dialog_id: &DialogId,
        transaction_id: &TransactionKey,
    ) {
        let should_remove = self
            .pending_response_transaction_by_dialog
            .get(dialog_id)
            .is_some_and(|entry| entry.value() == transaction_id);
        if should_remove {
            self.pending_response_transaction_by_dialog
                .remove(dialog_id);
        }
    }

    pub(crate) fn remove_dialog_storage(&self, dialog_id: &DialogId) -> Option<Dialog> {
        {
            if let Some(dialog) = self.dialogs.get(dialog_id) {
                self.insert_terminated_bye_tombstone(dialog.value());
            }
        }

        let (_, dialog) = self.dialogs.remove(dialog_id)?;

        if let Some(remote_tag) = dialog.remote_tag.as_ref() {
            let key = DialogUtils::create_early_lookup_key(&dialog.call_id, remote_tag);
            self.early_dialog_lookup.remove(&key);
        }

        if let Some((call_id, local_tag, remote_tag)) = dialog.dialog_id_tuple() {
            let key = DialogUtils::create_lookup_key(&call_id, &local_tag, &remote_tag);
            self.dialog_lookup.remove(&key);
            let reverse_key = DialogUtils::create_lookup_key(&call_id, &remote_tag, &local_tag);
            self.dialog_lookup.remove(&reverse_key);
        }

        if let Some((_, session_id)) = self.dialog_to_session.remove(dialog_id) {
            self.session_to_dialog.remove(&session_id);
        }
        self.pending_response_transaction_by_dialog
            .remove(dialog_id);
        self.remove_invite_failover_state_for_dialog(dialog_id);
        if let Some((_, invite_transactions)) = self.dialog_invite_transactions.remove(dialog_id) {
            for transaction_id in invite_transactions {
                self.transaction_manager
                    .remove_invite_2xx_response_cache(&transaction_id);
            }
        }
        self.dialog_server_transactions.remove(dialog_id);

        Some(dialog)
    }

    fn insert_terminated_bye_tombstone(&self, dialog: &Dialog) {
        if dialog.state != DialogState::Terminated || dialog.remote_cseq == 0 {
            return;
        }

        if let Some((call_id, local_tag, remote_tag)) = dialog.dialog_id_tuple() {
            let tombstone = TerminatedByeTombstone {
                cseq: dialog.remote_cseq,
                created_at: Instant::now(),
            };
            let key = DialogUtils::create_lookup_key(&call_id, &local_tag, &remote_tag);
            self.terminated_bye_lookup.insert(key, tombstone);
            let reverse_key = DialogUtils::create_lookup_key(&call_id, &remote_tag, &local_tag);
            self.terminated_bye_lookup.insert(reverse_key, tombstone);
            if crate::diagnostics::dialog_timing_enabled() {
                crate::diagnostics::record_bye_tombstone_observed_size(
                    self.terminated_bye_lookup.len(),
                );
            }

            let insert_count = self
                .terminated_bye_insert_count
                .fetch_add(2, Ordering::Relaxed)
                + 2;
            if insert_count % TERMINATED_BYE_PRUNE_INTERVAL == 0 {
                self.prune_terminated_bye_lookup();
            }
        }
    }

    fn prune_terminated_bye_lookup(&self) {
        let prune_started = crate::diagnostics::dialog_timing_enabled().then(Instant::now);
        let now = Instant::now();
        let expired_keys: Vec<_> = self
            .terminated_bye_lookup
            .iter()
            .filter(|entry| {
                now.duration_since(entry.value().created_at) >= TERMINATED_BYE_TOMBSTONE_TTL
            })
            .map(|entry| entry.key().clone())
            .collect();

        for key in expired_keys {
            self.terminated_bye_lookup.remove(&key);
        }

        let len = self.terminated_bye_lookup.len();
        crate::diagnostics::record_bye_tombstone_observed_size(len);

        if len > self.terminated_bye_lookup_hard_max {
            let overage = len - self.terminated_bye_lookup_hard_max;
            let overflow_keys: Vec<_> = self
                .terminated_bye_lookup
                .iter()
                .take(overage)
                .map(|entry| entry.key().clone())
                .collect();

            for key in overflow_keys {
                self.terminated_bye_lookup.remove(&key);
            }
        }

        if let Some(started) = prune_started {
            crate::diagnostics::record_bye_tombstone_prune(started.elapsed());
        }
    }

    /// Find the INVITE transaction associated with a dialog
    ///
    /// This is used for CANCEL operations to find the pending INVITE transaction
    /// that needs to be cancelled.
    ///
    /// # Arguments
    /// * `dialog_id` - The dialog ID to find the INVITE transaction for
    ///
    /// # Returns
    /// The transaction key for the INVITE if found, None otherwise
    pub fn find_invite_transaction_for_dialog(
        &self,
        dialog_id: &DialogId,
    ) -> Option<TransactionKey> {
        let Some(invite_transactions) = self.dialog_invite_transactions.get(dialog_id) else {
            debug!("No INVITE transaction found for dialog {}", dialog_id);
            return None;
        };

        for tx_key in invite_transactions.iter() {
            if self
                .transaction_to_dialog
                .get(tx_key)
                .is_some_and(|mapped_dialog_id| mapped_dialog_id.value() == dialog_id)
            {
                debug!("Found INVITE transaction for dialog {}", dialog_id);
                return Some(tx_key.clone());
            }
        }

        debug!("No INVITE transaction found for dialog {}", dialog_id);
        None
    }

    // ========================================
    // **NEW**: UNIFIED CONFIGURATION SUPPORT
    // ========================================

    /// Set the unified configuration for this DialogManager
    ///
    /// Enables mode-specific behavior based on configuration.
    /// This method allows the UnifiedDialogManager to inject configuration.
    ///
    /// # Arguments
    /// * `config` - Unified configuration determining behavior mode
    pub fn set_config(&mut self, config: DialogManagerConfig) {
        debug!(
            "Setting unified configuration to {:?} mode",
            Self::config_mode_name(&config)
        );
        if let Ok(mut guard) = self.config.write() {
            *guard = Some(config);
        }
    }

    /// Get a clone of the current configuration (if any).
    pub fn config(&self) -> Option<DialogManagerConfig> {
        self.config.read().ok().and_then(|g| g.clone())
    }

    /// Check if auto-response to OPTIONS requests is enabled
    pub fn should_auto_respond_to_options(&self) -> bool {
        self.config
            .read()
            .ok()
            .and_then(|g| g.as_ref().map(|c| c.auto_options_enabled()))
            .unwrap_or(false)
    }

    /// Check if auto-response to REGISTER requests is enabled
    pub fn should_auto_respond_to_register(&self) -> bool {
        self.config
            .read()
            .ok()
            .and_then(|g| g.as_ref().map(|c| c.auto_register_enabled()))
            .unwrap_or(false)
    }

    /// Check if outgoing calls are supported (defaults to true when no config).
    pub fn supports_outgoing_calls(&self) -> bool {
        self.config
            .read()
            .ok()
            .and_then(|g| g.as_ref().map(|c| c.supports_outgoing_calls()))
            .unwrap_or(true)
    }

    /// Check if incoming calls are supported (defaults to true when no config).
    pub fn supports_incoming_calls(&self) -> bool {
        self.config
            .read()
            .ok()
            .and_then(|g| g.as_ref().map(|c| c.supports_incoming_calls()))
            .unwrap_or(true)
    }

    /// Get configuration mode name for logging
    fn config_mode_name(config: &DialogManagerConfig) -> &'static str {
        match config {
            DialogManagerConfig::Client(_) => "Client",
            DialogManagerConfig::Server(_) => "Server",
            DialogManagerConfig::Hybrid(_) => "Hybrid",
        }
    }
}

// Forward declarations for methods that will be implemented in other modules
impl DialogManager {
    // Dialog Operations (delegated to dialog_operations.rs)
    pub async fn create_dialog(&self, request: &Request) -> DialogResult<DialogId> {
        <Self as super::dialog_operations::DialogStore>::create_dialog(self, request).await
    }

    pub async fn terminate_dialog(&self, dialog_id: &DialogId) -> DialogResult<()> {
        <Self as super::dialog_operations::DialogStore>::terminate_dialog(self, dialog_id).await
    }

    pub fn get_dialog(&self, dialog_id: &DialogId) -> DialogResult<Dialog> {
        <Self as super::dialog_operations::DialogStore>::get_dialog(self, dialog_id)
    }

    pub fn get_dialog_mut(
        &self,
        dialog_id: &DialogId,
    ) -> DialogResult<dashmap::mapref::one::RefMut<'_, DialogId, Dialog>> {
        <Self as super::dialog_operations::DialogStore>::get_dialog_mut(self, dialog_id)
    }

    pub async fn store_dialog(&self, dialog: Dialog) -> DialogResult<()> {
        <Self as super::dialog_operations::DialogStore>::store_dialog(self, dialog).await
    }

    pub fn list_dialogs(&self) -> Vec<DialogId> {
        <Self as super::dialog_operations::DialogStore>::list_dialogs(self)
    }

    pub fn get_dialog_state(&self, dialog_id: &DialogId) -> DialogResult<DialogState> {
        <Self as super::dialog_operations::DialogStore>::get_dialog_state(self, dialog_id)
    }

    pub async fn update_dialog_state(
        &self,
        dialog_id: &DialogId,
        new_state: DialogState,
    ) -> DialogResult<()> {
        <Self as super::dialog_operations::DialogStore>::update_dialog_state(
            self, dialog_id, new_state,
        )
        .await
    }

    pub async fn create_outgoing_dialog(
        &self,
        local_uri: rvoip_sip_core::Uri,
        remote_uri: rvoip_sip_core::Uri,
        call_id: Option<String>,
    ) -> DialogResult<DialogId> {
        <Self as super::dialog_operations::DialogStore>::create_outgoing_dialog(
            self, local_uri, remote_uri, call_id,
        )
        .await
    }

    /// Get a reference to the subscription manager if configured
    pub fn subscription_manager(&self) -> Option<&Arc<SubscriptionManager>> {
        self.subscription_manager.as_ref()
    }

    // ===== Event Hub Helper Methods =====

    /// Set the event hub for global event coordination
    pub async fn set_event_hub(&self, event_hub: Arc<crate::events::DialogEventHub>) {
        *self.event_hub.write().await = Some(event_hub);
    }

    /// Get session ID from dialog ID
    pub fn get_session_id(&self, dialog_id: &DialogId) -> Option<String> {
        self.dialog_to_session
            .get(dialog_id)
            .map(|e| e.value().clone())
    }

    /// Store dialog mapping for incoming call
    pub fn store_dialog_mapping(
        &self,
        session_id: &str,
        dialog_id: DialogId,
        transaction_id: TransactionKey,
        _request: rvoip_sip_core::Request,
        _source: SocketAddr,
    ) {
        self.session_to_dialog
            .insert(session_id.to_string(), dialog_id.clone());
        self.dialog_to_session
            .insert(dialog_id.clone(), session_id.to_string());
        self.link_transaction_to_dialog_indexed(&transaction_id, &dialog_id);
        self.pending_response_transaction_by_dialog
            .insert(dialog_id, transaction_id);
        // Store additional request data if needed
    }

    // Protocol Handlers (delegated to protocol_handlers.rs)
    pub async fn handle_invite(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        <Self as super::protocol_handlers::ProtocolHandlers>::handle_invite_method(
            self, request, source,
        )
        .await
    }

    pub async fn handle_bye(&self, request: Request) -> DialogResult<()> {
        <Self as super::protocol_handlers::ProtocolHandlers>::handle_bye_method(self, request).await
    }

    pub async fn handle_cancel(&self, request: Request) -> DialogResult<()> {
        <Self as super::protocol_handlers::ProtocolHandlers>::handle_cancel_method(self, request)
            .await
    }

    pub async fn handle_ack(&self, request: Request) -> DialogResult<()> {
        <Self as super::protocol_handlers::ProtocolHandlers>::handle_ack_method(self, request).await
    }

    pub async fn handle_options(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        <Self as super::protocol_handlers::ProtocolHandlers>::handle_options_method(
            self, request, source,
        )
        .await
    }

    pub async fn handle_register(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        <Self as super::protocol_handlers::MethodHandler>::handle_register_method(
            self, request, source,
        )
        .await
    }

    pub async fn handle_update(&self, request: Request) -> DialogResult<()> {
        <Self as super::protocol_handlers::ProtocolHandlers>::handle_update_method(self, request)
            .await
    }

    pub async fn handle_prack(&self, request: Request) -> DialogResult<()> {
        <Self as super::protocol_handlers::ProtocolHandlers>::handle_prack_method(self, request)
            .await
    }

    pub async fn handle_info(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        <Self as super::protocol_handlers::MethodHandler>::handle_info_method(self, request, source)
            .await
    }

    pub async fn handle_refer(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        <Self as super::protocol_handlers::MethodHandler>::handle_refer_method(
            self, request, source,
        )
        .await
    }

    pub async fn handle_subscribe(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        <Self as super::protocol_handlers::MethodHandler>::handle_subscribe_method(
            self, request, source,
        )
        .await
    }

    pub async fn handle_notify(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        <Self as super::protocol_handlers::MethodHandler>::handle_notify_method(
            self, request, source,
        )
        .await
    }

    /// Snapshot of the externally-visible address most recently
    /// learned from an inbound response's `Via: …;received=…;rport=…`
    /// (RFC 3581). Returns `None` until the first qualifying response
    /// arrives — i.e. before the first request goes on the wire, or
    /// when no NAT is in the path (in which case the discovered
    /// address would equal the local bind and we suppress the
    /// update).
    ///
    /// Callers can use this to rewrite outbound `Contact:` headers
    /// (RFC 5626 §5) so a registrar's stored binding routes through
    /// the discovered NAT mapping rather than the unreachable
    /// private bind address.
    pub async fn discovered_public_addr(&self) -> Option<SocketAddr> {
        *self.nat_discovered_addr.read().await
    }

    /// Returns the registrar-provided Service-Route (RFC 3608) for the
    /// given AoR, if a REGISTER 2xx has populated the cache. The
    /// returned URIs MUST be pre-loaded as Route headers on subsequent
    /// out-of-dialog requests from the UA for that AoR, in the order
    /// returned.
    ///
    /// `None` → no REGISTER 2xx observed for this AoR yet.
    /// `Some(empty vec)` → REGISTER 2xx observed, registrar declined to
    /// set a Service-Route (caller should not pre-load any Route).
    pub async fn service_route_for_aor(
        &self,
        aor: &str,
    ) -> Option<Vec<rvoip_sip_core::types::uri::Uri>> {
        self.service_route_by_aor.read().await.get(aor).cloned()
    }

    /// Returns the registrar-assigned GRUU URIs (RFC 5627 §5.3) for
    /// the given AoR, if a REGISTER 2xx has populated the cache.
    /// Either `pub_gruu` or `temp_gruu` may be `None` independently —
    /// a registrar may assign only one. `None` from this accessor
    /// means no REGISTER 2xx with GRUU has been observed for this AoR
    /// yet (or the registrar declined to assign either GRUU).
    pub async fn gruu_for_aor(
        &self,
        aor: &str,
    ) -> Option<rvoip_sip_core::types::outbound::GruuContactParams> {
        self.gruu_by_aor.read().await.get(aor).cloned()
    }

    /// Returns true when at least one RFC 5626 outbound-flow monitor is
    /// active for the given AoR.
    pub fn outbound_flow_active_for_aor(&self, aor: &str) -> bool {
        self.flow_by_aor
            .get(aor)
            .is_some_and(|flow_keys| !flow_keys.is_empty())
    }

    pub async fn handle_response(
        &self,
        response: Response,
        transaction_id: TransactionKey,
    ) -> DialogResult<()> {
        <Self as super::protocol_handlers::ProtocolHandlers>::handle_response_message(
            self,
            response,
            transaction_id,
        )
        .await
    }

    // Message Routing (delegated to message_routing.rs)
    pub async fn find_dialog_for_request(&self, request: &Request) -> Option<DialogId> {
        <Self as super::dialog_operations::DialogLookup>::find_dialog_for_request(self, request)
            .await
    }

    pub fn find_dialog_for_transaction(
        &self,
        transaction_id: &TransactionKey,
    ) -> DialogResult<DialogId> {
        <Self as super::message_routing::DialogMatcher>::match_transaction(self, transaction_id)
    }

    // Transaction Integration (delegated to transaction_integration.rs)
    pub async fn send_request(
        &self,
        dialog_id: &DialogId,
        method: Method,
        body: Option<bytes::Bytes>,
    ) -> DialogResult<TransactionKey> {
        <Self as super::transaction_integration::TransactionIntegration>::send_request_in_dialog(
            self, dialog_id, method, body,
        )
        .await
    }

    /// Send a BYE request carrying a `Reason:` header (RFC 3326).
    ///
    /// Used by the session-timer refresh-failure path (RFC 4028 §10) to
    /// communicate `Reason: SIP ;cause=408 ;text="Session expired"` on the
    /// BYE so peer observability is RFC-correct. Mirrors the transport
    /// plumbing of `send_request` for BYE but threads a typed `Reason`
    /// header through `bye_for_dialog`'s `extra_headers` param.
    pub async fn send_bye_with_reason(
        &self,
        dialog_id: &DialogId,
        reason: rvoip_sip_core::types::reason::Reason,
    ) -> DialogResult<TransactionKey> {
        use crate::transaction::dialog::quick as dialog_quick;
        use rvoip_sip_core::types::TypedHeader;

        debug!("Sending BYE with Reason header for dialog {}", dialog_id);

        let (candidates, request) = {
            let mut dialog = self.get_dialog_mut(dialog_id)?;

            let template = dialog.create_request_template(Method::Bye);

            let local_tag = match template.local_tag {
                Some(tag) if !tag.is_empty() => tag,
                _ => {
                    let new_tag = dialog.generate_local_tag();
                    dialog.local_tag = Some(new_tag.clone());
                    new_tag
                }
            };

            let remote_tag = template
                .remote_tag
                .filter(|t| !t.is_empty())
                .ok_or_else(|| {
                    DialogError::protocol_error("BYE requires remote tag in established dialog")
                })?;

            let request = dialog_quick::bye_for_dialog_with_request_uri(
                &template.call_id,
                &template.local_uri.to_string(),
                &local_tag,
                &template.remote_uri.to_string(),
                &remote_tag,
                &template.target_uri.to_string(),
                template.cseq_number,
                self.local_address_for_target_and_routes(&template.target_uri, &template.route_set),
                if template.route_set.is_empty() {
                    None
                } else {
                    Some(template.route_set.clone())
                },
                Some(vec![TypedHeader::Reason(reason)]),
            )
            .map_err(|_error| DialogError::InternalError {
                message: "Failed to build BYE request".to_string(),
                context: None,
            })?;

            let next_hop =
                crate::transaction::transport::multiplexed::exact_next_hop_uri_for_request(
                    &request,
                )
                .map_err(|_| DialogError::routing_error("BYE contains an unusable Route header"))?;
            let candidates = self.resolve_uri_to_candidates(&next_hop).await;
            if candidates.is_empty() {
                return Err(DialogError::routing_error(
                    "No address candidates for the exact BYE next hop",
                ));
            }

            (candidates, request)
        };

        let (transaction_id, _) = self
            .send_request_with_candidate_failover(request, candidates, Some(dialog_id))
            .await?;

        Ok(transaction_id)
    }

    /// Send an INFO request carrying a caller-chosen `Content-Type` (RFC 6086).
    ///
    /// The generic [`send_request_in_dialog`](Self::send_request) path always
    /// tags INFO bodies as `application/info`. This method lets the caller
    /// pick any content type — `application/dtmf-relay` for DTMF-over-INFO,
    /// `application/sipfrag` for fax flow control, etc.
    pub async fn send_info_with_content_type(
        &self,
        dialog_id: &DialogId,
        content_type: String,
        body: bytes::Bytes,
    ) -> DialogResult<TransactionKey> {
        use crate::transaction::dialog::quick as dialog_quick;

        debug!(
            "Sending INFO with Content-Type: {} for dialog {}",
            content_type, dialog_id
        );

        let (candidates, request) = {
            let mut dialog = self.get_dialog_mut(dialog_id)?;

            let template = dialog.create_request_template(Method::Info);

            let local_tag = match template.local_tag {
                Some(tag) if !tag.is_empty() => tag,
                _ => {
                    let new_tag = dialog.generate_local_tag();
                    dialog.local_tag = Some(new_tag.clone());
                    new_tag
                }
            };

            let remote_tag = template
                .remote_tag
                .filter(|t| !t.is_empty())
                .ok_or_else(|| {
                    DialogError::protocol_error("INFO requires remote tag in established dialog")
                })?;

            let body_str = String::from_utf8_lossy(&body).into_owned();
            let request = dialog_quick::info_for_dialog(
                &template.call_id,
                &template.local_uri.to_string(),
                &local_tag,
                &template.remote_uri.to_string(),
                &remote_tag,
                body_str,
                Some(content_type),
                template.cseq_number,
                self.local_address_for_target_and_routes(&template.target_uri, &template.route_set),
                if template.route_set.is_empty() {
                    None
                } else {
                    Some(template.route_set.clone())
                },
            )
            .map_err(|_error| DialogError::InternalError {
                message: "Failed to build INFO request".to_string(),
                context: None,
            })?;

            let next_hop =
                crate::transaction::transport::multiplexed::exact_next_hop_uri_for_request(
                    &request,
                )
                .map_err(|_| {
                    DialogError::routing_error("INFO contains an unusable Route header")
                })?;
            let candidates = self.resolve_uri_to_candidates(&next_hop).await;
            if candidates.is_empty() {
                return Err(DialogError::routing_error(
                    "No address candidates for the exact INFO next hop",
                ));
            }

            (candidates, request)
        };

        let (transaction_id, _) = self
            .send_request_with_candidate_failover(request, candidates, Some(dialog_id))
            .await?;

        Ok(transaction_id)
    }

    pub async fn send_response(
        &self,
        transaction_id: &TransactionKey,
        response: Response,
    ) -> DialogResult<()> {
        <Self as super::transaction_integration::TransactionIntegration>::send_transaction_response(
            self,
            transaction_id,
            response,
        )
        .await
    }

    pub fn associate_transaction_with_dialog(
        &self,
        transaction_id: &TransactionKey,
        dialog_id: &DialogId,
    ) {
        <Self as super::transaction_integration::TransactionHelpers>::link_transaction_to_dialog(
            self,
            transaction_id,
            dialog_id,
        )
    }

    pub async fn send_ack_for_2xx_response(
        &self,
        dialog_id: &DialogId,
        original_invite_tx_id: &TransactionKey,
        response: &Response,
    ) -> DialogResult<()> {
        debug!("Sending ACK for 2xx response for dialog {}", dialog_id);

        // Use transaction-core's send_ack_for_2xx method to actually send the ACK
        self.transaction_manager
            .send_ack_for_2xx(original_invite_tx_id, response)
            .await
            .map_err(|_error| crate::errors::DialogError::TransactionError {
                message: "Failed to send ACK for 2xx response".to_string(),
            })?;

        debug!(
            "Successfully sent ACK for 2xx response for dialog {}",
            dialog_id
        );
        Ok(())
    }

    pub async fn create_ack_for_2xx_response(
        &self,
        original_invite_tx_id: &TransactionKey,
        response: &Response,
    ) -> DialogResult<Request> {
        <Self as super::transaction_integration::TransactionHelpers>::create_ack_for_success_response(self, original_invite_tx_id, response).await
    }

    pub async fn find_transaction_by_message(
        &self,
        message: &rvoip_sip_core::Message,
    ) -> DialogResult<Option<TransactionKey>> {
        debug!("Finding transaction for message using transaction-core");

        self.transaction_manager
            .find_transaction_by_message(message)
            .await
            .map_err(|_error| DialogError::TransactionError {
                message: "Failed to find transaction by message".to_string(),
            })
    }
}

/// Drives a single [`OutboundFlow`] for its lifetime.
///
/// The loop alternates between sending CRLFCRLF pings on the keep-alive
/// interval and waiting for the per-ping pong deadline. Transport-side
/// events (`KeepAlivePongReceived`, `ConnectionClosed`) arrive out of
/// band via [`DialogManager::on_pong_received`] /
/// [`DialogManager::on_connection_closed`] and flip state on the shared
/// [`OutboundFlow`]; this task observes the updates when it next wakes
/// on the deadline arm.
///
/// The task exits — and cleans up its own registration in the manager's
/// maps — when it observes failure or when the manager aborts it.
async fn run_outbound_flow_loop(
    manager: DialogManager,
    flow: Arc<OutboundFlow>,
    transport: Arc<dyn rvoip_sip_transport::Transport>,
) {
    use bytes::Bytes;

    let mut ticker = tokio::time::interval(flow.interval);
    // The first tick fires immediately; skip it so the first ping goes
    // out at `interval` after REGISTER success, not right away (avoids
    // a thundering herd on bulk re-REGISTER).
    ticker.tick().await;

    // Pong deadline. Parked far in the future when no ping is
    // outstanding so the select arm effectively waits forever; reset to
    // `now + pong_timeout` after every successful ping.
    //
    // 365 days is well below `Instant` overflow on every platform we
    // support and low enough that pinning it here is harmless.
    let far_future =
        || tokio::time::Instant::now() + std::time::Duration::from_secs(365 * 24 * 3600);
    let sleep = tokio::time::sleep_until(far_future());
    tokio::pin!(sleep);
    let mut deadline_armed = false;

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                match transport
                    .send_raw_via(flow.route.clone(), Bytes::from_static(b"\r\n\r\n"))
                    .await
                {
                    Ok(()) => {
                        flow.record_ping_sent().await;
                        tracing::trace!(
                            dest = %flow.destination,
                            "RFC 5626 keep-alive ping sent"
                        );
                        let when = tokio::time::Instant::now() + flow.pong_timeout;
                        sleep.as_mut().reset(when);
                        deadline_armed = true;
                    }
                    Err(_error) => {
                        tracing::debug!(
                            dest = %flow.destination,
                            "RFC 5626 keep-alive send failed — marking flow failed"
                        );
                        if flow.mark_failed().await {
                            manager
                                .emit_outbound_flow_failed(&flow, FlowFailureReason::SendError)
                                .await;
                        }
                        break;
                    }
                }
            }
            _ = &mut sleep, if deadline_armed => {
                // Pong deadline fired. Re-check state under the flow's
                // internal locks — if a pong arrived during the wait
                // it already reset state to `Idle` and we just disarm.
                if flow.is_pong_overdue().await {
                    tracing::info!(
                        dest = %flow.destination,
                        pong_timeout_ms = flow.pong_timeout.as_millis() as u64,
                        "RFC 5626 pong timeout — marking flow failed"
                    );
                    if flow.mark_failed().await {
                        manager
                            .emit_outbound_flow_failed(&flow, FlowFailureReason::PongTimeout)
                            .await;
                    }
                    break;
                }
                sleep.as_mut().reset(far_future());
                deadline_armed = false;
            }
        }
    }

    // Clean up both the primary flow and the secondary index so a
    // future REGISTER 2xx for the same AoR can install a fresh flow.
    // Safe if `stop_outbound_ping` already removed us concurrently.
    manager.stop_outbound_ping(&flow.key);
}

#[cfg(test)]
mod outbound_flow_handler_tests {
    //! Tests for the `DialogManager` → `OutboundFlow` plumbing that
    //! lives across `on_pong_received`, `on_connection_closed`, and the
    //! `(outbound_flows, flow_by_destination)` pair. The state machine
    //! itself is unit-tested in `super::outbound_flow::tests`; these
    //! tests drive the handler entry points by pre-populating the maps
    //! so we don't have to boot a real transport or spawn the ping
    //! loop.
    use super::*;
    use crate::manager::outbound_flow::{FlowState, OutboundFlow};
    use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
    use rvoip_sip_transport::error::Result as TransportResult;
    use rvoip_sip_transport::{Transport, TransportEvent};
    use std::net::SocketAddr;
    use std::str::FromStr;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    #[test]
    fn outbound_transport_lookup_keys_match_response_identity() {
        let request = SimpleRequestBuilder::new(Method::Register, "sip:registrar.example.com")
            .unwrap()
            .from("Alice", "sip:alice@example.com", None)
            .to("Alice", "sip:alice@example.com", None)
            .call_id("register-call-id")
            .cseq(42)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK-register"))
            .max_forwards(70)
            .build();
        let response = SimpleResponseBuilder::response_from_request(
            &request,
            rvoip_sip_core::StatusCode::Unauthorized,
            None,
        )
        .build();

        assert_eq!(
            outbound_request_key(&request),
            outbound_response_key(&response)
        );
    }

    #[derive(Debug)]
    struct NoopTransport {
        addr: SocketAddr,
        closed: AtomicBool,
    }

    impl NoopTransport {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                addr: SocketAddr::from_str("127.0.0.1:5060").unwrap(),
                closed: AtomicBool::new(false),
            })
        }
    }

    #[async_trait::async_trait]
    impl Transport for NoopTransport {
        fn local_addr(&self) -> TransportResult<SocketAddr> {
            Ok(self.addr)
        }
        async fn send_message(
            &self,
            _m: rvoip_sip_core::Message,
            _dst: SocketAddr,
        ) -> TransportResult<()> {
            Ok(())
        }
        async fn close(&self) -> TransportResult<()> {
            self.closed.store(true, Ordering::SeqCst);
            Ok(())
        }
        fn is_closed(&self) -> bool {
            self.closed.load(Ordering::SeqCst)
        }
    }

    async fn make_manager() -> (DialogManager, mpsc::Receiver<SessionCoordinationEvent>) {
        let transport = NoopTransport::new();
        let (_tx, transport_rx) = mpsc::channel::<TransportEvent>(16);
        let (tm, _events_rx) = TransactionManager::new(transport, transport_rx, Some(16))
            .await
            .expect("build TransactionManager");
        let local = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let manager = DialogManager::new(Arc::new(tm), local)
            .await
            .expect("build DialogManager");

        // Install a legacy session-coordination channel so
        // `emit_session_coordination_event` delivers into the test.
        let (sc_tx, sc_rx) = mpsc::channel::<SessionCoordinationEvent>(16);
        *manager.session_coordinator.write().await = Some(sc_tx);

        (manager, sc_rx)
    }

    fn test_key(n: u8) -> (String, u32, String) {
        (
            format!("sip:alice{n}@example.com"),
            1,
            format!("urn:uuid:{n:032x}"),
        )
    }

    fn dest_addr(port: u16) -> SocketAddr {
        SocketAddr::from_str(&format!("127.0.0.1:{port}")).unwrap()
    }

    fn dispatch_request(method: Method, branch: &str, cseq: u32) -> Request {
        SimpleRequestBuilder::new(method.clone(), "sip:bob@example.com")
            .unwrap()
            .from("Alice", "sip:alice@example.com", Some("alice-dispatch-tag"))
            .to("Bob", "sip:bob@example.com", Some("bob-dispatch-tag"))
            .contact("sip:alice@127.0.0.1:5060", None)
            .call_id("dialog-dispatch-call-id")
            .cseq(cseq)
            .via("127.0.0.1:5060", "UDP", Some(branch))
            .max_forwards(70)
            .build()
    }

    #[tokio::test]
    async fn dialog_event_dispatch_routes_same_call_requests_to_same_worker() {
        let (manager, _rx) = make_manager().await;
        let fallback = AtomicUsize::new(0);
        let source = dest_addr(5070);
        let invite_tx = TransactionKey::new("z9hG4bK-invite".to_string(), Method::Invite, true);
        let bye_tx = TransactionKey::new("z9hG4bK-bye".to_string(), Method::Bye, true);
        let cancel_tx = TransactionKey::new("z9hG4bK-cancel".to_string(), Method::Cancel, true);

        let events = [
            TransactionEvent::InviteRequest {
                transaction_id: invite_tx.clone(),
                request: dispatch_request(Method::Invite, "z9hG4bK-invite", 1),
                source,
            },
            TransactionEvent::AckRequest {
                transaction_id: invite_tx.clone(),
                request: dispatch_request(Method::Ack, "z9hG4bK-ack", 1),
                source,
            },
            TransactionEvent::NonInviteRequest {
                transaction_id: bye_tx,
                request: dispatch_request(Method::Bye, "z9hG4bK-bye", 2),
                source,
            },
            TransactionEvent::CancelRequest {
                transaction_id: cancel_tx,
                target_transaction_id: invite_tx,
                request: dispatch_request(Method::Cancel, "z9hG4bK-cancel", 1),
                source,
            },
        ];

        let first = manager.dialog_event_dispatch_worker_index(&events[0], 8, &fallback);
        for event in &events[1..] {
            assert_eq!(
                manager.dialog_event_dispatch_worker_index(event, 8, &fallback),
                first
            );
        }
        assert_eq!(fallback.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn dialog_event_dispatch_routes_lifecycle_events_by_stored_call_route() {
        let (manager, _rx) = make_manager().await;
        let fallback = AtomicUsize::new(0);
        let source = dest_addr(5070);
        let invite_tx = TransactionKey::new("z9hG4bK-invite".to_string(), Method::Invite, true);

        let invite = TransactionEvent::InviteRequest {
            transaction_id: invite_tx.clone(),
            request: dispatch_request(Method::Invite, "z9hG4bK-invite", 1),
            source,
        };
        let state_changed = TransactionEvent::StateChanged {
            transaction_id: invite_tx.clone(),
            previous_state: TransactionState::Proceeding,
            new_state: TransactionState::Terminated,
        };
        let terminated = TransactionEvent::TransactionTerminated {
            transaction_id: invite_tx,
        };

        let first = manager.dialog_event_dispatch_worker_index(&invite, 8, &fallback);
        assert_eq!(
            manager.dialog_event_dispatch_worker_index(&state_changed, 8, &fallback),
            first
        );
        assert_eq!(
            manager.dialog_event_dispatch_worker_index(&terminated, 8, &fallback),
            first
        );
        assert_eq!(fallback.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn state_changed_terminated_clears_stored_dialog_route_hash() {
        let (manager, _rx) = make_manager().await;
        let fallback = AtomicUsize::new(0);
        let source = dest_addr(5070);
        let invite_tx =
            TransactionKey::new("z9hG4bK-route-cleanup".to_string(), Method::Invite, true);

        let invite = TransactionEvent::InviteRequest {
            transaction_id: invite_tx.clone(),
            request: dispatch_request(Method::Invite, "z9hG4bK-route-cleanup", 1),
            source,
        };
        manager.dialog_event_dispatch_worker_index(&invite, 8, &fallback);
        assert!(manager
            .transaction_dialog_route_hash
            .contains_key(&invite_tx));

        manager
            .process_global_transaction_event(TransactionEvent::StateChanged {
                transaction_id: invite_tx.clone(),
                previous_state: TransactionState::Proceeding,
                new_state: TransactionState::Terminated,
            })
            .await;

        assert!(!manager
            .transaction_dialog_route_hash
            .contains_key(&invite_tx));
    }

    #[tokio::test]
    async fn retained_invite_attempts_share_route_hash_and_prune_all_indexes() {
        use crate::manager::transaction_integration::{
            CandidateWirePlan, InviteFailoverAttempt, InviteFailoverAttemptIndex,
            InviteFailoverAttemptOutcome, InviteFailoverPlan, InviteFailoverPlanPhase,
        };

        let (manager, _rx) = make_manager().await;
        let dialog_id = DialogId::new();
        let first_transaction =
            TransactionKey::new("z9hG4bK-retained-first".to_string(), Method::Invite, false);
        let second_transaction =
            TransactionKey::new("z9hG4bK-retained-second".to_string(), Method::Invite, false);
        let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
            .unwrap()
            .from("Alice", "sip:alice@example.com", Some("alice-retained"))
            .to("Bob", "sip:bob@example.com", None)
            .contact("sip:alice@127.0.0.1:5060", None)
            .call_id("retained-plan-prune")
            .cseq(1)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK-retained-template"))
            .max_forwards(70)
            .build();

        manager.link_outbound_transaction_to_dialog_indexed(
            &first_transaction,
            &dialog_id,
            &request,
        );
        manager.link_outbound_transaction_to_dialog_indexed(
            &second_transaction,
            &dialog_id,
            &request,
        );
        assert_eq!(
            manager
                .transaction_dialog_route_hash
                .get(&first_transaction)
                .map(|entry| *entry.value()),
            manager
                .transaction_dialog_route_hash
                .get(&second_transaction)
                .map(|entry| *entry.value()),
            "all attempts in one logical INVITE must stay on one event shard"
        );

        let plan_id = 77;
        manager.invite_failover_plans.insert(
            plan_id,
            Arc::new(tokio::sync::Mutex::new(InviteFailoverPlan {
                id: plan_id,
                dialog_id: dialog_id.clone(),
                request,
                candidates: vec![rvoip_sip_transport::resolver::ResolvedTarget::immediate(
                    dest_addr(5090),
                    rvoip_sip_transport::transport::TransportType::Udp,
                )],
                wire_plan: CandidateWirePlan::default(),
                next_candidate_index: 1,
                current_transaction: Some(first_transaction.clone()),
                current_candidate_index: Some(0),
                provisional_seen: false,
                phase: InviteFailoverPlanPhase::Closed,
                attempts: vec![InviteFailoverAttempt {
                    transaction_id: first_transaction.clone(),
                    candidate_index: 0,
                    outcome: InviteFailoverAttemptOutcome::FinalResponse,
                }],
                accepted_transaction: None,
                accepted_to_tag: None,
                cleaned_fork_tags: std::collections::HashSet::new(),
                setup_deadline: Instant::now() + Duration::from_secs(32),
                expires_at: Instant::now() - Duration::from_millis(1),
            })),
        );
        manager
            .active_invite_failover_by_dialog
            .insert(dialog_id.clone(), plan_id);
        manager.invite_failover_attempts.insert(
            first_transaction.clone(),
            InviteFailoverAttemptIndex {
                plan_id,
                dialog_id: dialog_id.clone(),
                candidate_index: 0,
            },
        );

        manager.prune_invite_failover_state().await;

        assert!(!manager.invite_failover_plans.contains_key(&plan_id));
        assert!(!manager
            .active_invite_failover_by_dialog
            .contains_key(&dialog_id));
        assert!(!manager
            .invite_failover_attempts
            .contains_key(&first_transaction));
        assert!(!manager
            .transaction_to_dialog
            .contains_key(&first_transaction));
        assert!(!manager
            .transaction_dialog_route_hash
            .contains_key(&first_transaction));

        manager.unlink_transaction_from_dialog_indexed(&second_transaction);
    }

    #[tokio::test]
    async fn timer_b_advances_retained_invite_once_and_preserves_attempt_indexes() {
        use crate::manager::transaction_integration::{
            CandidateWirePlan, InviteFailoverAttemptOutcome, InviteFailoverPlanPhase,
        };

        let (manager, _rx) = make_manager().await;
        let dialog_id = DialogId::new();
        let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
            .unwrap()
            .from("Alice", "sip:alice@example.com", Some("alice-timeout"))
            .to("Bob", "sip:bob@example.com", None)
            .contact("sip:alice@127.0.0.1:5060", None)
            .call_id("retained-plan-timer-b")
            .cseq(1)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK-timeout-template"))
            .max_forwards(70)
            .build();
        let (first_transaction, _) = manager
            .send_request_with_candidate_wire_plan(
                request,
                vec![
                    rvoip_sip_transport::resolver::ResolvedTarget::immediate(
                        dest_addr(5091),
                        rvoip_sip_transport::transport::TransportType::Udp,
                    ),
                    rvoip_sip_transport::resolver::ResolvedTarget::immediate(
                        dest_addr(5092),
                        rvoip_sip_transport::transport::TransportType::Udp,
                    ),
                    rvoip_sip_transport::resolver::ResolvedTarget::immediate(
                        dest_addr(5093),
                        rvoip_sip_transport::transport::TransportType::Udp,
                    ),
                ],
                Some(&dialog_id),
                CandidateWirePlan::default(),
            )
            .await
            .expect("initial retained INVITE");
        let original_setup_deadline = {
            let plan_id = *manager
                .active_invite_failover_by_dialog
                .get(&dialog_id)
                .expect("active plan")
                .value();
            let plan = manager
                .invite_failover_plans
                .get(&plan_id)
                .expect("retained plan")
                .value()
                .clone();
            let setup_deadline = plan.lock().await.setup_deadline;
            setup_deadline
        };

        manager
            .process_global_transaction_event(TransactionEvent::TransactionTimeout {
                transaction_id: first_transaction.clone(),
            })
            .await;
        manager
            .process_global_transaction_event(TransactionEvent::TransactionTimeout {
                transaction_id: first_transaction.clone(),
            })
            .await;

        let plan_id = *manager
            .active_invite_failover_by_dialog
            .get(&dialog_id)
            .expect("plan remains active after failover")
            .value();
        let plan = manager
            .invite_failover_plans
            .get(&plan_id)
            .expect("retained plan")
            .value()
            .clone();
        let plan = plan.lock().await;
        let second_transaction = plan
            .current_transaction
            .clone()
            .expect("second attempt is current");
        assert_ne!(first_transaction, second_transaction);
        assert_eq!(plan.attempts.len(), 2);
        assert_eq!(plan.next_candidate_index, 2);
        assert_eq!(plan.setup_deadline, original_setup_deadline);
        assert_eq!(
            plan.attempts[0].outcome,
            InviteFailoverAttemptOutcome::TransactionTimeout
        );
        assert_eq!(
            plan.attempts[1].outcome,
            InviteFailoverAttemptOutcome::Active
        );
        drop(plan);

        manager
            .process_global_transaction_event(TransactionEvent::StateChanged {
                transaction_id: first_transaction.clone(),
                previous_state: TransactionState::Calling,
                new_state: TransactionState::Terminated,
            })
            .await;
        manager
            .process_global_transaction_event(TransactionEvent::TransactionTerminated {
                transaction_id: first_transaction.clone(),
            })
            .await;

        assert!(manager
            .invite_failover_attempts
            .contains_key(&first_transaction));
        assert!(manager
            .invite_failover_attempts
            .contains_key(&second_transaction));
        assert_eq!(
            manager
                .transaction_to_dialog
                .get(&second_transaction)
                .map(|entry| entry.value().clone()),
            Some(dialog_id.clone()),
            "terminal events from an old attempt cannot unlink the current attempt"
        );
        assert_eq!(
            manager
                .transaction_dialog_route_hash
                .get(&first_transaction)
                .map(|entry| *entry.value()),
            manager
                .transaction_dialog_route_hash
                .get(&second_transaction)
                .map(|entry| *entry.value())
        );

        assert_eq!(
            manager
                .invite_failover_plan_reservations
                .load(Ordering::Acquire),
            1
        );
        assert_eq!(
            manager
                .invite_failover_attempt_reservations
                .load(Ordering::Acquire),
            3
        );
        let retained_plan = manager
            .invite_failover_plans
            .get(&plan_id)
            .expect("retained plan before TTL prune")
            .value()
            .clone();
        let mut retained_plan_guard = retained_plan.lock().await;
        retained_plan_guard.phase = InviteFailoverPlanPhase::Closed;
        retained_plan_guard.expires_at = Instant::now() - Duration::from_millis(1);
        manager.prune_invite_failover_state().await;
        assert!(
            manager.invite_failover_plans.contains_key(&plan_id),
            "maintenance must skip a busy plan instead of blocking its event shard"
        );
        drop(retained_plan_guard);
        manager.prune_invite_failover_state().await;

        assert!(!manager.invite_failover_plans.contains_key(&plan_id));
        assert!(!manager
            .invite_failover_attempts
            .contains_key(&first_transaction));
        assert!(!manager
            .invite_failover_attempts
            .contains_key(&second_transaction));
        assert_eq!(
            manager
                .invite_failover_plan_reservations
                .load(Ordering::Acquire),
            0
        );
        assert_eq!(
            manager
                .invite_failover_attempt_reservations
                .load(Ordering::Acquire),
            0
        );
    }

    #[tokio::test]
    async fn cancel_and_timer_b_serialize_on_one_exact_current_attempt() {
        use crate::manager::transaction_integration::{
            CandidateWirePlan, InviteFailoverAttemptOutcome, InviteFailoverPlanPhase,
        };

        let (manager, _rx) = make_manager().await;
        let manager = Arc::new(manager);
        let dialog_id = DialogId::new();
        let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
            .unwrap()
            .from("Alice", "sip:alice@example.com", Some("alice-race"))
            .to("Bob", "sip:bob@example.com", None)
            .contact("sip:alice@127.0.0.1:5060", None)
            .call_id("retained-plan-cancel-timeout-race")
            .cseq(1)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK-race-template"))
            .max_forwards(70)
            .build();
        let (first_transaction, _) = manager
            .send_request_with_candidate_wire_plan(
                request,
                vec![
                    rvoip_sip_transport::resolver::ResolvedTarget::immediate(
                        dest_addr(5095),
                        rvoip_sip_transport::transport::TransportType::Udp,
                    ),
                    rvoip_sip_transport::resolver::ResolvedTarget::immediate(
                        dest_addr(5096),
                        rvoip_sip_transport::transport::TransportType::Udp,
                    ),
                    rvoip_sip_transport::resolver::ResolvedTarget::immediate(
                        dest_addr(5097),
                        rvoip_sip_transport::transport::TransportType::Udp,
                    ),
                ],
                Some(&dialog_id),
                CandidateWirePlan::default(),
            )
            .await
            .expect("initial race INVITE");
        let timeout_manager = manager.clone();
        let timeout_transaction = first_transaction.clone();
        let cancel_manager = manager.clone();
        let cancel_transaction = first_transaction.clone();

        let (_, cancel_result) = tokio::join!(
            async move {
                timeout_manager
                    .process_global_transaction_event(TransactionEvent::TransactionTimeout {
                        transaction_id: timeout_transaction,
                    })
                    .await;
            },
            async move {
                cancel_manager
                    .cancel_invite_transaction_with_dialog(&cancel_transaction)
                    .await
            }
        );
        cancel_result.expect("serialized CANCEL succeeds");

        let plan_id = manager
            .invite_failover_attempts
            .get(&first_transaction)
            .expect("first attempt remains indexed")
            .plan_id;
        let plan = manager
            .invite_failover_plans
            .get(&plan_id)
            .expect("race plan retained")
            .value()
            .clone();
        let plan = plan.lock().await;
        assert_eq!(plan.phase, InviteFailoverPlanPhase::Cancelled);
        assert!((1..=2).contains(&plan.attempts.len()));
        let current_transaction = plan
            .current_transaction
            .as_ref()
            .expect("cancelled current transaction");
        assert_eq!(
            plan.attempts
                .iter()
                .find(|attempt| &attempt.transaction_id == current_transaction)
                .expect("current attempt")
                .outcome,
            InviteFailoverAttemptOutcome::Cancelled
        );
        if plan.attempts.len() == 2 {
            assert_eq!(
                plan.attempts[0].outcome,
                InviteFailoverAttemptOutcome::TransactionTimeout
            );
        }
        drop(plan);
        assert!(!manager
            .active_invite_failover_by_dialog
            .contains_key(&dialog_id));

        // Exercise the opposite serialized outcome explicitly: once CANCEL
        // owns the plan, a queued Timer-B event cannot create another leg.
        let cancel_first_dialog = DialogId::new();
        let cancel_first_request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
            .unwrap()
            .from("Alice", "sip:alice@example.com", Some("alice-cancel-first"))
            .to("Bob", "sip:bob@example.com", None)
            .contact("sip:alice@127.0.0.1:5060", None)
            .call_id("retained-plan-cancel-first")
            .cseq(1)
            .via(
                "127.0.0.1:5060",
                "UDP",
                Some("z9hG4bK-cancel-first-template"),
            )
            .max_forwards(70)
            .build();
        let (cancel_first_transaction, _) = manager
            .send_request_with_candidate_wire_plan(
                cancel_first_request,
                vec![
                    rvoip_sip_transport::resolver::ResolvedTarget::immediate(
                        dest_addr(5100),
                        rvoip_sip_transport::transport::TransportType::Udp,
                    ),
                    rvoip_sip_transport::resolver::ResolvedTarget::immediate(
                        dest_addr(5101),
                        rvoip_sip_transport::transport::TransportType::Udp,
                    ),
                ],
                Some(&cancel_first_dialog),
                CandidateWirePlan::default(),
            )
            .await
            .expect("cancel-first INVITE");
        manager
            .cancel_invite_transaction_with_dialog(&cancel_first_transaction)
            .await
            .expect("cancel wins serialization");
        manager
            .process_global_transaction_event(TransactionEvent::TransactionTimeout {
                transaction_id: cancel_first_transaction.clone(),
            })
            .await;
        let cancel_first_plan_id = manager
            .invite_failover_attempts
            .get(&cancel_first_transaction)
            .expect("cancel-first attempt remains indexed")
            .plan_id;
        let cancel_first_plan = manager
            .invite_failover_plans
            .get(&cancel_first_plan_id)
            .expect("cancel-first plan")
            .value()
            .clone();
        let cancel_first_plan = cancel_first_plan.lock().await;
        assert_eq!(cancel_first_plan.phase, InviteFailoverPlanPhase::Cancelled);
        assert_eq!(cancel_first_plan.attempts.len(), 1);
        assert_eq!(
            cancel_first_plan.attempts[0].outcome,
            InviteFailoverAttemptOutcome::Cancelled
        );
    }

    #[tokio::test]
    async fn retained_invite_plan_hard_cap_rejects_admission_and_stop_drains_all_state() {
        use crate::manager::transaction_integration::{
            CandidateWirePlan, InviteFailoverAttempt, InviteFailoverAttemptIndex,
            InviteFailoverAttemptOutcome, InviteFailoverPlan, InviteFailoverPlanPhase,
        };

        let (manager, _rx) = make_manager().await;
        let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
            .unwrap()
            .from("Alice", "sip:alice@example.com", Some("alice-cap"))
            .to("Bob", "sip:bob@example.com", None)
            .contact("sip:alice@127.0.0.1:5060", None)
            .call_id("retained-plan-cap")
            .cseq(1)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK-cap-template"))
            .max_forwards(70)
            .build();
        let retained_transaction =
            TransactionKey::new("z9hG4bK-cap-retained".into(), Method::Invite, false);
        let retained_dialog = DialogId::new();
        let now = Instant::now();

        for offset in 0..manager.invite_failover_plan_capacity {
            let plan_id = offset as u64 + 1;
            let dialog_id = if offset == 0 {
                retained_dialog.clone()
            } else {
                DialogId::new()
            };
            let attempts = if offset == 0 {
                vec![InviteFailoverAttempt {
                    transaction_id: retained_transaction.clone(),
                    candidate_index: 0,
                    outcome: InviteFailoverAttemptOutcome::Active,
                }]
            } else {
                Vec::new()
            };
            manager.invite_failover_plans.insert(
                plan_id,
                Arc::new(tokio::sync::Mutex::new(InviteFailoverPlan {
                    id: plan_id,
                    dialog_id,
                    request: request.clone(),
                    candidates: Vec::new(),
                    wire_plan: CandidateWirePlan::default(),
                    next_candidate_index: 0,
                    current_transaction: (offset == 0).then(|| retained_transaction.clone()),
                    current_candidate_index: (offset == 0).then_some(0),
                    provisional_seen: false,
                    phase: InviteFailoverPlanPhase::Active,
                    attempts,
                    accepted_transaction: None,
                    accepted_to_tag: None,
                    cleaned_fork_tags: std::collections::HashSet::new(),
                    setup_deadline: now + Duration::from_secs(32),
                    expires_at: now + Duration::from_secs(90),
                })),
            );
        }
        manager
            .active_invite_failover_by_dialog
            .insert(retained_dialog.clone(), 1);
        manager.invite_failover_attempts.insert(
            retained_transaction.clone(),
            InviteFailoverAttemptIndex {
                plan_id: 1,
                dialog_id: retained_dialog.clone(),
                candidate_index: 0,
            },
        );
        manager.link_outbound_transaction_to_dialog_indexed(
            &retained_transaction,
            &retained_dialog,
            &request,
        );

        let admission_dialog = DialogId::new();
        let rejected = manager
            .send_request_with_candidate_wire_plan(
                request.clone(),
                vec![rvoip_sip_transport::resolver::ResolvedTarget::immediate(
                    dest_addr(5098),
                    rvoip_sip_transport::transport::TransportType::Udp,
                )],
                Some(&admission_dialog),
                CandidateWirePlan::default(),
            )
            .await
            .expect_err("hard plan cap rejects new setup");
        assert_eq!(rejected.diagnostic_class(), "transaction");

        manager.invite_failover_plans.clear();
        manager.active_invite_failover_by_dialog.clear();
        let oversized_candidates = vec![
            rvoip_sip_transport::resolver::ResolvedTarget::immediate(
                dest_addr(5099),
                rvoip_sip_transport::transport::TransportType::Udp,
            );
            manager.invite_failover_attempt_capacity + 1
        ];
        let attempt_rejected = manager
            .send_request_with_candidate_wire_plan(
                request,
                oversized_candidates,
                Some(&admission_dialog),
                CandidateWirePlan::default(),
            )
            .await
            .expect_err("hard attempt reservation cap rejects new setup");
        assert_eq!(attempt_rejected.diagnostic_class(), "transaction");

        manager.stop().await.expect("manager stop");
        assert!(manager.invite_failover_plans.is_empty());
        assert!(manager.active_invite_failover_by_dialog.is_empty());
        assert!(manager.invite_failover_attempts.is_empty());
        assert_eq!(
            manager
                .invite_failover_plan_reservations
                .load(Ordering::Relaxed),
            0
        );
        assert_eq!(
            manager
                .invite_failover_attempt_reservations
                .load(Ordering::Relaxed),
            0
        );
        assert!(manager.transaction_to_dialog.is_empty());
        assert!(manager.transaction_dialog_route_hash.is_empty());
        let retention = manager.retention_counts();
        assert_eq!(retention.invite_failover_plans, 0);
        assert_eq!(retention.active_invite_failover_by_dialog, 0);
        assert_eq!(retention.invite_failover_attempts, 0);
        assert_eq!(retention.invite_failover_plan_reservations, 0);
        assert_eq!(retention.invite_failover_attempt_reservations, 0);
    }

    #[tokio::test]
    async fn duplicate_selected_invite_success_is_reacked_without_duplicate_answer_event() {
        use crate::manager::transaction_integration::CandidateWirePlan;

        let (manager, mut session_events) = make_manager().await;
        let mut dialog = Dialog::new(
            "retained-selected-duplicate".to_string(),
            "sip:alice@example.com".parse().unwrap(),
            "sip:bob@example.com".parse().unwrap(),
            Some("alice-selected".to_string()),
            None,
            true,
        );
        dialog.state = DialogState::Early;
        let dialog_id = dialog.id.clone();
        manager.store_dialog(dialog).await.expect("store dialog");
        let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
            .unwrap()
            .from("Alice", "sip:alice@example.com", Some("alice-selected"))
            .to("Bob", "sip:bob@example.com", None)
            .contact("sip:alice@127.0.0.1:5060", None)
            .call_id("retained-selected-duplicate")
            .cseq(1)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK-selected-template"))
            .max_forwards(70)
            .build();
        let (transaction_id, _) = manager
            .send_request_with_candidate_wire_plan(
                request,
                vec![rvoip_sip_transport::resolver::ResolvedTarget::immediate(
                    dest_addr(5094),
                    rvoip_sip_transport::transport::TransportType::Udp,
                )],
                Some(&dialog_id),
                CandidateWirePlan::default(),
            )
            .await
            .expect("send selected INVITE");
        let sent_request = manager
            .transaction_manager
            .original_request(&transaction_id)
            .await
            .expect("request lookup")
            .expect("request retained");
        let response = SimpleResponseBuilder::response_from_request(
            &sent_request,
            rvoip_sip_core::StatusCode::Ok,
            None,
        )
        .to("Bob", "sip:bob@example.com", Some("bob-selected"))
        .contact("sip:bob@127.0.0.1:5094", None)
        .body(bytes::Bytes::from_static(b"v=0\r\n"))
        .build();
        let event = TransactionEvent::SuccessResponse {
            transaction_id: transaction_id.clone(),
            response: response.clone(),
            need_ack: true,
            source: dest_addr(5094),
        };

        manager.process_global_transaction_event(event).await;
        manager
            .process_global_transaction_event(TransactionEvent::SuccessResponse {
                transaction_id,
                response,
                need_ack: true,
                source: dest_addr(5094),
            })
            .await;

        let mut answer_events = 0;
        while let Ok(event) = session_events.try_recv() {
            if matches!(event, SessionCoordinationEvent::CallAnswered { .. }) {
                answer_events += 1;
            }
        }
        assert_eq!(answer_events, 1);
        assert_eq!(
            manager.get_dialog_state(&dialog_id).expect("dialog state"),
            DialogState::Confirmed
        );
    }

    #[tokio::test]
    async fn ack_received_clears_stored_dialog_route_hash_after_processing() {
        let (manager, _rx) = make_manager().await;
        let fallback = AtomicUsize::new(0);
        let invite_tx =
            TransactionKey::new("z9hG4bK-ack-cleanup".to_string(), Method::Invite, true);

        let ack = TransactionEvent::AckReceived {
            transaction_id: invite_tx.clone(),
            request: dispatch_request(Method::Ack, "z9hG4bK-ack-cleanup", 1),
        };
        manager.dialog_event_dispatch_worker_index(&ack, 8, &fallback);
        assert!(manager
            .transaction_dialog_route_hash
            .contains_key(&invite_tx));

        manager.process_global_transaction_event(ack).await;

        assert!(!manager
            .transaction_dialog_route_hash
            .contains_key(&invite_tx));
    }

    #[tokio::test]
    async fn dialog_event_dispatch_round_robins_unkeyed_events() {
        let (manager, _rx) = make_manager().await;
        let fallback = AtomicUsize::new(0);
        let events = [
            TransactionEvent::ShutdownRequested,
            TransactionEvent::ShutdownReady,
            TransactionEvent::ShutdownNow,
        ];

        assert_eq!(
            manager.dialog_event_dispatch_worker_index(&events[0], 3, &fallback),
            0
        );
        assert_eq!(
            manager.dialog_event_dispatch_worker_index(&events[1], 3, &fallback),
            1
        );
        assert_eq!(
            manager.dialog_event_dispatch_worker_index(&events[2], 3, &fallback),
            2
        );
    }

    #[test]
    fn terminated_bye_lookup_hard_max_scales_with_index_capacity() {
        assert_eq!(
            terminated_bye_lookup_hard_max(MIN_DIALOG_INDEX_CAPACITY),
            MIN_TERMINATED_BYE_LOOKUP_HARD_MAX
        );
        assert_eq!(terminated_bye_lookup_hard_max(100_000), 1_600_000);
    }

    #[tokio::test]
    async fn remove_dialog_storage_indexes_terminated_bye_tombstone() {
        let (manager, _rx) = make_manager().await;
        let mut dialog = Dialog::new(
            "terminated-bye-index-test".to_string(),
            "sip:alice@example.com".parse().unwrap(),
            "sip:bob@example.com".parse().unwrap(),
            Some("alice-tag".to_string()),
            Some("bob-tag".to_string()),
            true,
        );
        dialog.state = DialogState::Terminated;
        dialog.remote_cseq = 42;
        let dialog_id = dialog.id.clone();

        manager.store_dialog(dialog).await.expect("store dialog");
        let removed = manager
            .remove_dialog_storage(&dialog_id)
            .expect("dialog removed");
        let (call_id, local_tag, remote_tag) =
            removed.dialog_id_tuple().expect("full dialog tuple");
        let key = DialogUtils::create_lookup_key(&call_id, &local_tag, &remote_tag);
        let reverse_key = DialogUtils::create_lookup_key(&call_id, &remote_tag, &local_tag);

        assert_eq!(
            manager
                .terminated_bye_lookup
                .get(&key)
                .expect("forward tombstone")
                .cseq,
            42
        );
        assert_eq!(
            manager
                .terminated_bye_lookup
                .get(&reverse_key)
                .expect("reverse tombstone")
                .cseq,
            42
        );
        assert!(!manager.dialogs.contains_key(&dialog_id));
    }

    #[tokio::test]
    async fn cleanup_dialog_storage_and_transactions_terminates_indexed_server_transactions() {
        use crate::transaction::runner::HasLifecycle;
        use crate::transaction::state::TransactionLifecycle;

        let (manager, _rx) = make_manager().await;
        let source = dest_addr(5070);
        let mut dialog = Dialog::new(
            "cleanup-dialog-transaction-test".to_string(),
            "sip:alice@example.com".parse().unwrap(),
            "sip:bob@example.com".parse().unwrap(),
            Some("alice-tag".to_string()),
            Some("bob-tag".to_string()),
            false,
        );
        dialog.state = DialogState::Early;
        let dialog_id = dialog.id.clone();
        manager.store_dialog(dialog).await.expect("store dialog");

        let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
            .unwrap()
            .from("Alice", "sip:alice@example.com", Some("alice-tag"))
            .to("Bob", "sip:bob@example.com", None)
            .contact("sip:alice@127.0.0.1:5070", None)
            .call_id("cleanup-dialog-transaction-test")
            .cseq(1)
            .via("127.0.0.1:5070", "UDP", Some("z9hG4bK-cleanup-dialog-tx"))
            .max_forwards(70)
            .build();
        let transaction = manager
            .transaction_manager()
            .create_server_transaction(request, source)
            .await
            .expect("server transaction");
        let transaction_id = transaction.id().clone();
        manager.link_transaction_to_dialog_indexed(&transaction_id, &dialog_id);
        manager
            .pending_response_transaction_by_dialog
            .insert(dialog_id.clone(), transaction_id.clone());

        assert_eq!(manager.transaction_manager().transaction_count().await, 1);
        assert!(
            manager
                .cleanup_dialog_storage_and_transactions(&dialog_id)
                .await
        );

        for _ in 0..20 {
            if transaction.data().get_lifecycle() == TransactionLifecycle::Destroyed {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        assert_eq!(manager.transaction_manager().transaction_count().await, 0);
        assert_eq!(
            transaction.data().get_lifecycle(),
            TransactionLifecycle::Destroyed
        );
        assert!(!manager.dialogs.contains_key(&dialog_id));
        assert!(!manager.transaction_to_dialog.contains_key(&transaction_id));
        assert!(manager
            .server_transactions_for_dialog(&dialog_id)
            .is_empty());
        assert!(!manager
            .pending_response_transaction_by_dialog
            .contains_key(&dialog_id));
    }

    #[tokio::test]
    async fn non_invite_bye_uses_existing_server_transaction() {
        let (manager, _rx) = make_manager().await;
        let source = dest_addr(5070);
        let mut dialog = Dialog::new(
            "bye-existing-transaction".to_string(),
            "sip:alice@example.com".parse().unwrap(),
            "sip:bob@example.com".parse().unwrap(),
            Some("alice-tag".to_string()),
            Some("bob-tag".to_string()),
            false,
        );
        dialog.state = DialogState::Confirmed;
        dialog.remote_cseq = 1;
        let dialog_id = dialog.id.clone();
        manager.store_dialog(dialog).await.expect("store dialog");

        let request = SimpleRequestBuilder::new(Method::Bye, "sip:alice@example.com")
            .unwrap()
            .from("Bob", "sip:bob@example.com", Some("bob-tag"))
            .to("Alice", "sip:alice@example.com", Some("alice-tag"))
            .call_id("bye-existing-transaction")
            .cseq(2)
            .via("127.0.0.1:5070", "UDP", Some("z9hG4bK-bye-existing"))
            .max_forwards(70)
            .build();
        let transaction = manager
            .transaction_manager()
            .create_server_transaction(request.clone(), source)
            .await
            .expect("existing BYE server transaction");
        let transaction_id = transaction.id().clone();

        manager
            .handle_unassociated_transaction_event(
                &transaction_id,
                TransactionEvent::NonInviteRequest {
                    transaction_id: transaction_id.clone(),
                    request,
                    source,
                },
            )
            .await
            .expect("BYE handled through existing transaction");

        assert_eq!(manager.transaction_manager().transaction_count().await, 0);
        let stored = manager
            .get_dialog(&dialog_id)
            .expect("dialog still indexed");
        assert_eq!(stored.state, DialogState::Terminated);
    }

    fn install_flow(
        manager: &DialogManager,
        key: (String, u32, String),
        dest: SocketAddr,
    ) -> Arc<OutboundFlow> {
        let flow = Arc::new(OutboundFlow::new(
            key.clone(),
            dest,
            Duration::from_secs(25),
        ));
        manager.outbound_flows.insert(key.clone(), flow.clone());
        manager.index_outbound_flow_key(key, dest);
        flow
    }

    #[tokio::test]
    async fn transaction_dialog_indexes_track_server_and_invite_keys() {
        let (manager, _rx) = make_manager().await;
        let dialog_id = DialogId(uuid::Uuid::new_v4());
        let server_invite =
            TransactionKey::new("z9hG4bK-server-invite".to_string(), Method::Invite, true);
        let server_bye = TransactionKey::new("z9hG4bK-server-bye".to_string(), Method::Bye, true);
        let client_invite =
            TransactionKey::new("z9hG4bK-client-invite".to_string(), Method::Invite, false);

        manager.link_transaction_to_dialog_indexed(&server_invite, &dialog_id);
        manager.link_transaction_to_dialog_indexed(&server_bye, &dialog_id);
        manager.link_transaction_to_dialog_indexed(&client_invite, &dialog_id);

        let server_transactions = manager.server_transactions_for_dialog(&dialog_id);
        assert!(server_transactions.contains(&server_invite));
        assert!(server_transactions.contains(&server_bye));
        assert!(!server_transactions.contains(&client_invite));
        assert_eq!(
            manager.find_invite_transaction_for_dialog(&dialog_id),
            Some(server_invite.clone())
        );

        manager.unlink_transaction_from_dialog_indexed(&server_invite);
        let server_transactions = manager.server_transactions_for_dialog(&dialog_id);
        assert!(!server_transactions.contains(&server_invite));
        assert!(server_transactions.contains(&server_bye));
        assert_eq!(
            manager.find_invite_transaction_for_dialog(&dialog_id),
            Some(client_invite.clone())
        );

        manager.unlink_transaction_from_dialog_indexed(&server_bye);
        assert!(manager
            .server_transactions_for_dialog(&dialog_id)
            .is_empty());

        manager.unlink_transaction_from_dialog_indexed(&client_invite);
        assert_eq!(manager.find_invite_transaction_for_dialog(&dialog_id), None);
    }

    #[tokio::test]
    async fn on_pong_received_resets_existing_flow_state() {
        let (manager, _rx) = make_manager().await;
        let key = test_key(1);
        let dest = dest_addr(5080);
        let flow = install_flow(&manager, key.clone(), dest);
        flow.record_ping_sent().await;
        assert_eq!(flow.state().await, FlowState::AwaitingPong);

        manager.on_pong_received(dest).await;

        assert_eq!(flow.state().await, FlowState::Idle);
    }

    #[tokio::test]
    async fn on_pong_received_is_noop_for_unknown_destination() {
        let (manager, _rx) = make_manager().await;
        let key = test_key(1);
        let flow = install_flow(&manager, key.clone(), dest_addr(5081));
        flow.record_ping_sent().await;

        // A pong from a peer we don't have a flow for must not disturb
        // the existing flow.
        manager.on_pong_received(dest_addr(9999)).await;

        assert_eq!(flow.state().await, FlowState::AwaitingPong);
    }

    #[tokio::test]
    async fn on_connection_closed_emits_event_once_and_clears_maps() {
        let (manager, mut rx) = make_manager().await;
        let key = test_key(2);
        let dest = dest_addr(5082);
        let flow = install_flow(&manager, key.clone(), dest);

        manager.on_connection_closed(dest).await;

        // Exactly one OutboundFlowFailed for this key with
        // `ConnectionClosed` reason.
        let event = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .expect("event must arrive")
            .expect("channel open");
        match event {
            SessionCoordinationEvent::OutboundFlowFailed {
                aor,
                reg_id,
                instance,
                reason,
            } => {
                assert_eq!(aor, key.0);
                assert_eq!(reg_id, key.1);
                assert_eq!(instance, key.2);
                assert_eq!(reason, FlowFailureReason::ConnectionClosed);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        // Flow state flipped to Failed.
        assert_eq!(flow.state().await, FlowState::Failed);

        // Maps were cleared.
        assert!(!manager.outbound_flows.contains_key(&key));
        assert!(manager.flow_by_destination.get(&dest).is_none());

        // Idempotent: a second close for the same destination must not
        // emit another event (the flow is gone + state is Failed
        // anyway).
        manager.on_connection_closed(dest).await;
        assert!(
            tokio::time::timeout(Duration::from_millis(100), rx.recv())
                .await
                .is_err(),
            "no additional event after second close"
        );
    }

    #[tokio::test]
    async fn stop_outbound_ping_does_not_emit_failure_event() {
        // Explicit teardown is not a flow failure; no event should fire.
        let (manager, mut rx) = make_manager().await;
        let key = test_key(3);
        let dest = dest_addr(5083);
        let _flow = install_flow(&manager, key.clone(), dest);

        manager.stop_outbound_ping(&key);

        assert!(!manager.outbound_flows.contains_key(&key));
        assert!(manager.flow_by_destination.get(&dest).is_none());
        assert!(
            tokio::time::timeout(Duration::from_millis(50), rx.recv())
                .await
                .is_err(),
            "stop must not emit OutboundFlowFailed"
        );
    }

    #[tokio::test]
    async fn outbound_keepalive_rejects_address_only_and_flowless_routes() {
        let (manager, _rx) = make_manager().await;
        manager.set_outbound_keepalive_interval(Some(Duration::from_secs(30)));
        let key = test_key(4);
        let destination = dest_addr(5084);

        manager.start_outbound_ping(key.clone(), destination);
        assert!(!manager.outbound_flows.contains_key(&key));
        assert!(!manager.start_outbound_ping_on_route(
            key.clone(),
            rvoip_sip_transport::TransportRoute::new(destination)
                .with_transport_type(rvoip_sip_transport::transport::TransportType::Tcp),
        ));
        assert!(!manager.outbound_flows.contains_key(&key));
    }
}
