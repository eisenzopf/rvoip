//! Simplified Dialog Adapter for rvoip-sip
//!
//! Thin translation layer between dialog-core and state machine.
//! Focuses only on essential dialog operations and events.
//!
//! ## API Design
//!
//! This adapter provides a clean interface for dialog operations:
//!
//! ### Primary Methods
//! - `send_invite_with_details()` - Creates dialog and sends INVITE in one atomic operation
//! - `send_response()` - Sends SIP responses for incoming calls
//! - `send_bye()` - Terminates calls
//! - `send_ack()` - Acknowledges responses
//!
//! ### Removed Methods
//! The following methods were removed to avoid confusion:
//! - `create_dialog()` - Did not actually create a dialog in dialog-core
//! - `send_invite()` - Did not actually send an INVITE
//!
//! All dialog creation is now done through `send_invite_with_details()` which
//! properly creates the dialog in dialog-core and sends the INVITE.

use crate::adapters::outbound_request_tracker::{
    OutboundInDialogRequestTracker, TrackedInDialogOptions,
};
use crate::api::types::DialogIdentity;
use crate::cleanup_diag::{self, CleanupStage};
use crate::errors::{Result, SessionError};
use crate::retained_tasks::RetainedTasks;
use crate::session_lifecycle::{
    ManagedResourceReleaseError, ManagedSessionResource, OwnedOperation, OwnedOperationCompletion,
    ResourceDescriptor, ResourceSpec, SessionOperationKind,
};
use crate::session_registry::{SessionRegistry, SessionRegistryError, SessionRegistryHandle};
use crate::session_store::SessionStore;
use crate::sip_data_message::{
    build_sip_data_request, SipDataMessage, SipDataMessageDispatchLanes,
};
use crate::state_table::types::{DialogId, SessionId};
use dashmap::DashMap;
use rvoip_infra_common::events::{
    coordinator::GlobalEventCoordinator, cross_crate::RvoipCrossCrateEvent,
};
use rvoip_sip_core::{Response, StatusCode, Uri};
use rvoip_sip_dialog::{
    api::unified::{
        ByeRequestOptions, CancelRequestOptions, InfoRequestOptions, MessageRequestOptions,
        NotifyRequestOptions, ReferRequestOptions, SubscribeRequestOptions, UnifiedDialogApi,
        UpdateRequestOptions,
    },
    transaction::{
        dialog::DialogRequestTemplate, transport::multiplexed::exact_next_hop_uri_for_request,
        ClientTransactionCompletionHandle, ClientTransactionOutcome, TransactionKey,
    },
    DialogId as RvoipDialogId, DialogState, InitialInviteOwner, InitialInviteWireOutcome,
};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex as StdMutex, Weak};
use std::time::{Duration, Instant};

const INITIAL_INVITE_OWNED_OPERATION_TIMEOUT: Duration = Duration::from_secs(30);
const INITIAL_INVITE_RESOURCE_RELEASE_TIMEOUT: Duration = Duration::from_secs(12);
const INITIAL_INVITE_PROTOCOL_DRAIN_TIMEOUT: Duration = Duration::from_secs(10);
const DATA_MESSAGE_FINAL_RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);
const REGISTRATION_REFRESH_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

const OWNED_INVITE_INSTALLED: u8 = 0;
const OWNED_INVITE_SENT: u8 = 1;
const OWNED_INVITE_WIRE_UNKNOWN: u8 = 2;
const OWNED_INVITE_ZERO_WIRE: u8 = 3;

fn exact_response_method_class(method: &rvoip_sip_core::Method) -> &'static str {
    use rvoip_sip_core::Method;

    match method {
        Method::Invite => "INVITE",
        Method::Ack => "ACK",
        Method::Bye => "BYE",
        Method::Cancel => "CANCEL",
        Method::Register => "REGISTER",
        Method::Options => "OPTIONS",
        Method::Subscribe => "SUBSCRIBE",
        Method::Notify => "NOTIFY",
        Method::Update => "UPDATE",
        Method::Refer => "REFER",
        Method::Info => "INFO",
        Method::Message => "MESSAGE",
        Method::Prack => "PRACK",
        Method::Publish => "PUBLISH",
        Method::Extension(_) => "extension",
    }
}

fn exact_response_transaction_diagnostics(
    transaction_id: &TransactionKey,
) -> (&'static str, &'static str) {
    let direction = if transaction_id.is_server() {
        "server"
    } else {
        "client"
    };
    (
        exact_response_method_class(transaction_id.method()),
        direction,
    )
}

struct RegistrationRefreshTask {
    generation: u64,
    cancel: tokio::sync::oneshot::Sender<()>,
}

struct RegistrationRefreshCompletion {
    admission: Arc<StdMutex<()>>,
    tasks: Arc<DashMap<SessionId, RegistrationRefreshTask>>,
    session_id: SessionId,
    generation: u64,
}

impl Drop for RegistrationRefreshCompletion {
    fn drop(&mut self) {
        let _admission = self
            .admission
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.tasks.remove_if(&self.session_id, |_, current| {
            current.generation == self.generation
        });
    }
}

#[derive(Clone)]
struct DataMessageAuthChallenge {
    status: u16,
    value: String,
}

#[derive(Clone)]
struct OutboundByeTransaction {
    generation: u64,
    transaction_id: TransactionKey,
    /// Completion captured atomically with transaction creation. This exact
    /// authority remains valid after the manager retires the key-indexed
    /// transaction entry.
    completion: ClientTransactionCompletionHandle,
    /// Request-URI captured from the dialog remote target immediately before
    /// this generation is built. Local teardown owns the dialog from that
    /// point, so target-refresh requests are no longer admitted; retaining the
    /// value also keeps Digest retry independent of compact transaction
    /// tombstones, which intentionally do not retain non-INVITE request wire.
    request_uri: String,
}

enum OutgoingByeGenerationWake {
    UseExactOutcome(
        rvoip_sip_dialog::transaction::TransactionResult<Option<ClientTransactionOutcome>>,
    ),
    FollowNewerGeneration,
    RetryCurrentGeneration,
    CleanupInterrupted,
}

fn resolve_outgoing_bye_generation_wake(
    current_outcome: rvoip_sip_dialog::transaction::TransactionResult<
        Option<ClientTransactionOutcome>,
    >,
    newer_generation_exists: bool,
    generation_watch_closed: bool,
    retained_transaction_exists: bool,
) -> OutgoingByeGenerationWake {
    match current_outcome {
        current @ Ok(Some(_)) | current @ Err(_) => {
            OutgoingByeGenerationWake::UseExactOutcome(current)
        }
        Ok(None) if newer_generation_exists => OutgoingByeGenerationWake::FollowNewerGeneration,
        Ok(None) if generation_watch_closed && !retained_transaction_exists => {
            OutgoingByeGenerationWake::CleanupInterrupted
        }
        Ok(None) => OutgoingByeGenerationWake::RetryCurrentGeneration,
    }
}

fn data_message_auth_realm(selected: &crate::auth::ClientAuthHeader) -> String {
    if let Some(challenge) = selected.digest_challenge.as_ref() {
        return challenge.realm.clone();
    }

    match &selected.scheme {
        crate::auth::SipAuthScheme::Digest => "digest",
        crate::auth::SipAuthScheme::Bearer => "bearer",
        crate::auth::SipAuthScheme::Basic => "basic",
        crate::auth::SipAuthScheme::Aka => "aka",
        crate::auth::SipAuthScheme::Other(_) => "other",
    }
    .to_string()
}

/// Mutate retained authentication state on the latest revision of one exact
/// session lifetime.
///
/// Authentication callers intentionally capture only the generation-qualified
/// lifecycle handle from their read snapshot.  Using the snapshot revision as
/// a compare-and-swap fence would reject a valid request whenever an unrelated
/// state-machine update lands between that read and Digest bookkeeping.  The
/// exact handle still rejects replacement lifetimes, while the per-session
/// update lock serializes nonce-count and credential changes with the latest
/// benign state revision.
fn update_retained_auth_exact<R>(
    store: &SessionStore,
    handle: &SessionRegistryHandle,
    unavailable: &'static str,
    update: impl FnOnce(&mut crate::session_store::SessionState) -> Result<R>,
) -> Result<R> {
    store
        .update_session_exact_with(handle, None, update)
        .map_err(|_| SessionError::InvalidTransition(unavailable.to_string()))?
}

#[derive(Clone)]
struct OutboundInitialInviteBinding {
    handle: SessionRegistryHandle,
    owner: InitialInviteOwner,
    resource: Weak<OutboundInitialInviteResource>,
}

impl OutboundInitialInviteBinding {
    fn matches(&self, handle: &SessionRegistryHandle, owner: &InitialInviteOwner) -> bool {
        self.handle == *handle && self.owner == *owner
    }
}

/// Exact lower-layer ownership retained by the session lifecycle authority.
///
/// Adapter maps still expose their historical raw-ID shapes, so a separate
/// exact binding fences every mutation and rollback. The weak self-reference
/// lets explicit CANCEL/BYE paths mark protocol teardown without creating a
/// resource/map reference cycle.
struct OutboundInitialInviteResource {
    dialog_api: Arc<UnifiedDialogApi>,
    registry: Arc<SessionRegistry>,
    handle: SessionRegistryHandle,
    owner: InitialInviteOwner,
    bindings: Arc<DashMap<SessionId, OutboundInitialInviteBinding>>,
    session_to_dialog: Arc<DashMap<SessionId, RvoipDialogId>>,
    dialog_to_session: Arc<DashMap<RvoipDialogId, SessionId>>,
    callid_to_session: Arc<DashMap<String, SessionId>>,
    outgoing_invite_tx: Arc<DashMap<SessionId, TransactionKey>>,
    phase: AtomicU8,
    protocol_teardown_owned_by_upper: AtomicBool,
    session_map_installed: AtomicBool,
    dialog_map_installed: AtomicBool,
    call_id_map_installed: AtomicBool,
    registry_map_installed: AtomicBool,
    transaction_map_installed: AtomicBool,
}

/// Deterministic SIP Call-ID used by every outbound dialog construction path.
/// Media routing may derive its lookup key before dialog-core returns, so this
/// function is the single source of truth shared with the adapter layer.
pub(crate) fn deterministic_outbound_call_id(session_id: &SessionId) -> String {
    format!("{}@rvoip-sip", session_id.0)
}

/// Registrar metadata returned on a successful REGISTER 2xx response.
#[derive(Debug, Clone, Default)]
pub(crate) struct RegistrationResponseMetadata {
    pub(crate) service_route: Option<Vec<String>>,
    pub(crate) pub_gruu: Option<String>,
    pub(crate) temp_gruu: Option<String>,
    /// Exact flow-bearing route used by the successful REGISTER attempt.
    pub(crate) transport_route: Option<rvoip_sip_transport::TransportRoute>,
}

/// Outcome for a single REGISTER wire attempt.
///
/// This deliberately does not encode state-machine lifecycle decisions. The
/// dialog adapter sends one request, parses one response, and returns the SIP
/// result; the state-machine action decides which internal event to enqueue.
#[derive(Debug, Clone)]
pub(crate) enum RegisterAttemptOutcome {
    Registered {
        accepted_expires: u32,
        metadata: RegistrationResponseMetadata,
    },
    Unregistered,
    AuthChallenge {
        status_code: u16,
        challenge: String,
    },
    IntervalTooBrief {
        min_expires: u32,
    },
    Failure {
        status_code: u16,
        reason: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InviteDispatchFailure {
    Initial,
    InitialWithExtraHeaders,
    InitialWithOptions,
    AuthRetry,
    SessionTimerRetry,
    LegacyUpdateReinvite,
    ReinviteWithOptions,
    ReinviteInDialog,
}

impl InviteDispatchFailure {
    fn diagnostic(self) -> &'static str {
        match self {
            Self::Initial => "Failed to make call (class=invite-dispatch)",
            Self::InitialWithExtraHeaders => {
                "Failed to make call with extra headers (class=invite-dispatch)"
            }
            Self::InitialWithOptions => {
                "Failed to send INVITE with options (class=dialog-dispatch)"
            }
            Self::AuthRetry => "resend_invite_with_auth failed (class=invite-auth-retry)",
            Self::SessionTimerRetry => {
                "resend_invite_with_session_timer_override failed (class=invite-timer-retry)"
            }
            Self::LegacyUpdateReinvite => {
                "Failed to send legacy UPDATE re-INVITE (class=invite-dispatch)"
            }
            Self::ReinviteWithOptions => {
                "Failed to send re-INVITE with options (class=invite-dispatch)"
            }
            Self::ReinviteInDialog => "Failed to send in-dialog re-INVITE (class=invite-dispatch)",
        }
    }
}

fn redacted_invite_dispatch_error<E>(failure: InviteDispatchFailure, _source: E) -> SessionError {
    // Dialog-layer failures can retain a parser or validation source
    // containing caller-owned URI/header/auth material. Preserve only the
    // operation and fixed failure class at this public wrapper.
    SessionError::DialogError(failure.diagnostic().to_string())
}

fn redacted_dialog_operation_error<E>(operation: &'static str, _source: E) -> SessionError {
    SessionError::DialogError(format!("{operation} failed (class=dialog-dispatch)"))
}

fn register_auth_scheme_class(scheme: &crate::auth::SipAuthScheme) -> &'static str {
    match scheme {
        crate::auth::SipAuthScheme::Digest => "digest",
        crate::auth::SipAuthScheme::Bearer => "bearer",
        crate::auth::SipAuthScheme::Basic => "basic",
        crate::auth::SipAuthScheme::Aka => "aka",
        crate::auth::SipAuthScheme::Other(_) => "other",
    }
}

impl OutboundInitialInviteResource {
    fn new(
        adapter: &DialogAdapter,
        handle: SessionRegistryHandle,
        owner: InitialInviteOwner,
    ) -> Arc<Self> {
        Arc::new(Self {
            dialog_api: Arc::clone(&adapter.dialog_api),
            registry: Arc::clone(adapter.store.registry()),
            handle,
            owner,
            bindings: Arc::clone(&adapter.outbound_initial_invites),
            session_to_dialog: Arc::clone(&adapter.session_to_dialog),
            dialog_to_session: Arc::clone(&adapter.dialog_to_session),
            callid_to_session: Arc::clone(&adapter.callid_to_session),
            outgoing_invite_tx: Arc::clone(&adapter.outgoing_invite_tx),
            phase: AtomicU8::new(OWNED_INVITE_INSTALLED),
            protocol_teardown_owned_by_upper: AtomicBool::new(false),
            session_map_installed: AtomicBool::new(false),
            dialog_map_installed: AtomicBool::new(false),
            call_id_map_installed: AtomicBool::new(false),
            registry_map_installed: AtomicBool::new(false),
            transaction_map_installed: AtomicBool::new(false),
        })
    }

    fn install_adapter_bindings(self: &Arc<Self>) -> std::result::Result<(), &'static str> {
        use dashmap::mapref::entry::Entry;

        let session_id = self.handle.session_id().clone();
        match self.bindings.entry(session_id.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(OutboundInitialInviteBinding {
                    handle: self.handle.clone(),
                    owner: self.owner.clone(),
                    resource: Arc::downgrade(self),
                });
            }
            Entry::Occupied(_) => return Err("exact outbound INVITE binding already exists"),
        }

        match self.session_to_dialog.entry(session_id.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(self.owner.dialog_id().clone());
                self.session_map_installed.store(true, Ordering::Release);
            }
            Entry::Occupied(_) => return Err("session already owns a dialog mapping"),
        }
        match self.dialog_to_session.entry(self.owner.dialog_id().clone()) {
            Entry::Vacant(entry) => {
                entry.insert(session_id.clone());
                self.dialog_map_installed.store(true, Ordering::Release);
            }
            Entry::Occupied(_) => return Err("dialog already owns a session mapping"),
        }
        match self
            .callid_to_session
            .entry(self.owner.call_id().to_string())
        {
            Entry::Vacant(entry) => {
                entry.insert(session_id);
                self.call_id_map_installed.store(true, Ordering::Release);
            }
            Entry::Occupied(_) => return Err("Call-ID already owns a session mapping"),
        }

        self.registry
            .map_dialog_handle(&self.handle, self.owner.dialog_id().clone().into())
            .map_err(|_| "exact session registry dialog mapping failed")?;
        self.registry_map_installed.store(true, Ordering::Release);
        Ok(())
    }

    fn record_wire_outcome(&self, outcome: InitialInviteWireOutcome) {
        let phase = match outcome {
            InitialInviteWireOutcome::ZeroWire => OWNED_INVITE_ZERO_WIRE,
            InitialInviteWireOutcome::Sent => OWNED_INVITE_SENT,
            InitialInviteWireOutcome::Unknown => OWNED_INVITE_WIRE_UNKNOWN,
        };
        self.phase.store(phase, Ordering::Release);
    }

    fn install_transaction(&self, transaction_id: TransactionKey) -> bool {
        use dashmap::mapref::entry::Entry;

        let session_id = self.handle.session_id().clone();
        if !self
            .bindings
            .get(&session_id)
            .is_some_and(|binding| binding.matches(&self.handle, &self.owner))
        {
            return false;
        }
        match self.outgoing_invite_tx.entry(session_id) {
            Entry::Vacant(entry) => {
                entry.insert(transaction_id);
                self.transaction_map_installed
                    .store(true, Ordering::Release);
                true
            }
            Entry::Occupied(_) => false,
        }
    }

    fn mark_protocol_teardown_owned_by_upper(&self) {
        self.protocol_teardown_owned_by_upper
            .store(true, Ordering::Release);
    }

    // The arguments are an immutable teardown snapshot. Keeping them flat
    // makes the retained cleanup task independent of the owning guard.
    #[allow(clippy::too_many_arguments)]
    async fn release_exact(
        dialog_api: Arc<UnifiedDialogApi>,
        registry: Arc<SessionRegistry>,
        handle: SessionRegistryHandle,
        owner: InitialInviteOwner,
        bindings: Arc<DashMap<SessionId, OutboundInitialInviteBinding>>,
        session_to_dialog: Arc<DashMap<SessionId, RvoipDialogId>>,
        dialog_to_session: Arc<DashMap<RvoipDialogId, SessionId>>,
        callid_to_session: Arc<DashMap<String, SessionId>>,
        outgoing_invite_tx: Arc<DashMap<SessionId, TransactionKey>>,
        phase: u8,
        protocol_teardown_owned_by_upper: bool,
        session_map_installed: bool,
        dialog_map_installed: bool,
        call_id_map_installed: bool,
        registry_map_installed: bool,
        transaction_map_installed: bool,
    ) -> std::result::Result<(), ManagedResourceReleaseError> {
        let retained = dialog_api.initial_invite_owner_is_retained(&owner);
        if retained {
            match phase {
                OWNED_INVITE_INSTALLED => {
                    let _ = dialog_api.compensate_initial_invite(&owner).await;
                }
                OWNED_INVITE_SENT => {
                    let active = dialog_api
                        .list_active_dialogs()
                        .await
                        .iter()
                        .any(|dialog_id| dialog_id == owner.dialog_id());
                    // A peer-originated BYE can retire the dialog before this
                    // exact resource release runs. Missing from the manager's
                    // active-dialog index is then a confirmed terminal
                    // condition, not an uncertain sent INVITE that needs a
                    // synthetic BYE/CANCEL supervisor. If the dialog races
                    // away between the index read and state read, recheck the
                    // authoritative index; a state-read failure on a still-
                    // live dialog remains fail-closed.
                    let terminal = if !active {
                        true
                    } else {
                        match dialog_api.get_dialog_state(owner.dialog_id()).await {
                            Ok(DialogState::Terminated) => true,
                            Ok(_) => false,
                            Err(_) => !dialog_api
                                .list_active_dialogs()
                                .await
                                .iter()
                                .any(|dialog_id| dialog_id == owner.dialog_id()),
                        }
                    };
                    if protocol_teardown_owned_by_upper || terminal {
                        let _ = dialog_api.finish_initial_invite_teardown(&owner).await;
                    } else {
                        let _ = dialog_api.supervise_initial_invite_teardown(&owner);
                    }
                }
                OWNED_INVITE_WIRE_UNKNOWN => {
                    let _ = dialog_api.supervise_initial_invite_teardown(&owner);
                }
                OWNED_INVITE_ZERO_WIRE => {
                    let _ = dialog_api.compensate_initial_invite(&owner).await;
                }
                _ => return Err(ManagedResourceReleaseError::new("invite-phase-invalid")),
            }
        }

        if dialog_api.initial_invite_owner_is_retained(&owner) {
            let deadline = tokio::time::Instant::now() + INITIAL_INVITE_PROTOCOL_DRAIN_TIMEOUT;
            loop {
                if !dialog_api.initial_invite_owner_is_retained(&owner) {
                    break;
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err(ManagedResourceReleaseError::new(
                        "invite-protocol-teardown-pending",
                    ));
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }

        let session_id = handle.session_id().clone();
        let binding_matches = bindings
            .get(&session_id)
            .is_some_and(|binding| binding.matches(&handle, &owner));
        if !binding_matches {
            return Ok(());
        }

        if registry_map_installed {
            match registry.clear_dialog_handle_retained(&handle, owner.dialog_id().clone().into()) {
                Ok(_)
                | Err(SessionRegistryError::SlotMissing)
                | Err(SessionRegistryError::RevisionMismatch) => {}
                Err(_) => {
                    return Err(ManagedResourceReleaseError::new(
                        "invite-registry-release-failed",
                    ));
                }
            }
        }

        if bindings
            .remove_if(&session_id, |_, binding| binding.matches(&handle, &owner))
            .is_none()
        {
            return Ok(());
        }

        if session_map_installed {
            session_to_dialog.remove_if(&session_id, |_, dialog_id| dialog_id == owner.dialog_id());
        }
        if dialog_map_installed {
            dialog_to_session.remove_if(owner.dialog_id(), |_, mapped_session| {
                mapped_session == &session_id
            });
        }
        if call_id_map_installed {
            callid_to_session.remove_if(owner.call_id(), |_, mapped_session| {
                mapped_session == &session_id
            });
        }
        if transaction_map_installed {
            outgoing_invite_tx.remove(&session_id);
        }
        Ok(())
    }
}

impl ManagedSessionResource for OutboundInitialInviteResource {
    fn descriptor(&self) -> ResourceDescriptor {
        ResourceDescriptor::new("sip-initial-invite", self.owner.dialog_id().to_string())
    }

    fn cancel(&self) {
        // The authority's owned operation/dispatch supervisors remain retained
        // across caller cancellation. Phase-specific protocol work belongs in
        // the async release path where its outcome can be observed.
    }

    fn release(
        &self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = std::result::Result<(), ManagedResourceReleaseError>>
                + Send
                + 'static,
        >,
    > {
        let dialog_api = Arc::clone(&self.dialog_api);
        let registry = Arc::clone(&self.registry);
        let handle = self.handle.clone();
        let owner = self.owner.clone();
        let bindings = Arc::clone(&self.bindings);
        let session_to_dialog = Arc::clone(&self.session_to_dialog);
        let dialog_to_session = Arc::clone(&self.dialog_to_session);
        let callid_to_session = Arc::clone(&self.callid_to_session);
        let outgoing_invite_tx = Arc::clone(&self.outgoing_invite_tx);
        let phase = self.phase.load(Ordering::Acquire);
        let protocol_teardown_owned_by_upper = self
            .protocol_teardown_owned_by_upper
            .load(Ordering::Acquire);
        let session_map_installed = self.session_map_installed.load(Ordering::Acquire);
        let dialog_map_installed = self.dialog_map_installed.load(Ordering::Acquire);
        let call_id_map_installed = self.call_id_map_installed.load(Ordering::Acquire);
        let registry_map_installed = self.registry_map_installed.load(Ordering::Acquire);
        let transaction_map_installed = self.transaction_map_installed.load(Ordering::Acquire);
        Box::pin(Self::release_exact(
            dialog_api,
            registry,
            handle,
            owner,
            bindings,
            session_to_dialog,
            dialog_to_session,
            callid_to_session,
            outgoing_invite_tx,
            phase,
            protocol_teardown_owned_by_upper,
            session_map_installed,
            dialog_map_installed,
            call_id_map_installed,
            registry_map_installed,
            transaction_map_installed,
        ))
    }
}

async fn rollback_owned_invite<T>(
    operation: OwnedOperation,
    value: T,
) -> OwnedOperationCompletion<T> {
    operation
        .rollback(value)
        .await
        .unwrap_or_else(|_| panic!("initial INVITE exact rollback failed"))
}

async fn commit_owned_invite<T>(
    operation: OwnedOperation,
    value: T,
) -> OwnedOperationCompletion<T> {
    match operation.commit() {
        Ok(committed) => committed.complete(value),
        Err(failure) => rollback_owned_invite(failure.into_operation(), value).await,
    }
}

/// Minimal dialog adapter - just translates between dialog-core and state machine
pub struct DialogAdapter {
    /// Dialog-core unified API
    pub(crate) dialog_api: Arc<UnifiedDialogApi>,

    /// Session store for updating IDs
    pub(crate) store: Arc<SessionStore>,

    /// Simple mapping of session IDs to dialog IDs
    pub(crate) session_to_dialog: Arc<DashMap<SessionId, RvoipDialogId>>,
    pub(crate) dialog_to_session: Arc<DashMap<RvoipDialogId, SessionId>>,

    /// Store Call-ID to session mapping for correlation
    pub(crate) callid_to_session: Arc<DashMap<String, SessionId>>,

    /// Store outgoing INVITE transaction IDs for UAC ACK sending
    pub(crate) outgoing_invite_tx: Arc<DashMap<SessionId, TransactionKey>>,

    /// Latest in-dialog BYE transaction for each exact live session. The
    /// generation lets a 401/407-driven retry supersede the challenged
    /// transaction without a late initial dispatch overwriting it.
    outgoing_bye_tx: Arc<DashMap<SessionId, OutboundByeTransaction>>,
    /// Exact per-session generation notification. Authentication retries wake
    /// the owning BYE waiter directly instead of requiring 10 ms polling.
    outgoing_bye_generation_watch: Arc<DashMap<SessionId, tokio::sync::watch::Sender<u64>>>,
    next_outgoing_bye_generation: Arc<AtomicU64>,
    /// Timer F / configured non-INVITE transaction horizon used by the
    /// retained local-BYE cleanup owner.
    non_invite_transaction_timeout: Duration,

    /// Exact in-dialog request ownership for methods whose builder futures
    /// return after first transport write while authentication/final response
    /// arrives asynchronously.
    pub(crate) outbound_request_tracker: OutboundInDialogRequestTracker,

    /// Exact owner for each staged outbound initial INVITE. Raw compatibility
    /// maps are mutated or removed only while this binding still matches.
    outbound_initial_invites: Arc<DashMap<SessionId, OutboundInitialInviteBinding>>,

    /// FIFO serialization for reliable-ordered SIP DataMessages. A lane is
    /// scoped to an exact dialog ID and removed by exact dialog cleanup.
    data_message_dispatch_lanes: Arc<SipDataMessageDispatchLanes>,

    /// SIP_API_DESIGN_2 §7.4 — application-supplied headers stamped on
    /// every outbound message the state machine emits automatically
    /// (auto-BYE on session-timer expiry, auto-CANCEL on
    /// dialog-terminated-during-INVITE, auto-NOTIFY on REFER
    /// completion). Populated at construction from
    /// [`crate::Config::auto_emit_extra_headers`]; empty by default.
    pub(crate) auto_emit_extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,

    /// Global event coordinator for publishing events
    pub(crate) global_coordinator: Arc<GlobalEventCoordinator>,

    /// State machine reference for triggering events (needed for REGISTER
    /// response handling). Wired post-construction via
    /// [`DialogAdapter::init_state_machine`] because the `StateMachine`
    /// transitively depends on this adapter — classic circular init. The
    /// `OnceLock` makes the initialization soundly observable by any task
    /// without requiring `&mut self`.
    pub(crate) state_machine: Arc<std::sync::OnceLock<Arc<crate::state_machine::StateMachine>>>,

    /// RFC 3261 §8.1.2 outbound proxy URI, validated at construction. When
    /// `Some`, `send_invite_with_extra_headers` prepends a `Route:
    /// <proxy-uri;lr>` header so dialog-initiating requests traverse the
    /// configured proxy. `None` → no Route pre-loading. Populated from
    /// [`crate::Config::outbound_proxy_uri`] during coordinator setup.
    pub(crate) outbound_proxy_uri: Option<rvoip_sip_core::types::uri::Uri>,

    /// RFC 5626 §4 outbound registration params (`+sip.instance` URN +
    /// `reg-id`) applied to REGISTER Contact headers, together with the
    /// `;ob` URI flag. `None` → pre-5626 behaviour. Populated at
    /// construction from
    /// [`crate::Config::sip_outbound_enabled`]+[`crate::Config::sip_instance`].
    pub(crate) outbound_contact_params:
        Option<rvoip_sip_core::types::outbound::OutboundContactParams>,

    /// Symmetric registered-flow keep-alive identity. Unlike RFC 5626 mode,
    /// this starts after REGISTER success even if the registrar does not echo
    /// outbound Contact parameters.
    pub(crate) symmetric_flow_params:
        Option<rvoip_sip_core::types::outbound::OutboundContactParams>,

    /// Automatic registration refresh settings and task registry.
    registration_auto_refresh: bool,
    registration_refresh_jitter_percent: u8,
    registration_refresh_admission: Arc<StdMutex<()>>,
    registration_refresh_tasks: Arc<DashMap<SessionId, RegistrationRefreshTask>>,
    registration_refresh_retained: Arc<RetainedTasks>,
    next_registration_refresh_generation: Arc<AtomicU64>,

    /// Perf diagnostics for dialog mapping cleanup balance.
    cleanup_attempt_total: Arc<AtomicU64>,
    cleanup_mapped_total: Arc<AtomicU64>,
    cleanup_missing_total: Arc<AtomicU64>,
    cleanup_call_ids_removed_total: Arc<AtomicU64>,
    cleanup_outgoing_invite_removed_total: Arc<AtomicU64>,

    #[cfg(test)]
    cleanup_pause: Arc<std::sync::Mutex<Option<Arc<DialogCleanupPause>>>>,

    /// SIP_API_DESIGN_2 §12.4 — pluggable trace-output redactor. When
    /// `Some`, the trace path consults this hook before emitting each
    /// header to the trace sink so PII / carrier tokens can be
    /// scrubbed without affecting the wire form. Populated at
    /// construction from
    /// [`crate::Config::trace_redaction`]; `None` resolves to the
    /// production-safe default policy before construction. See
    /// [`crate::TraceRedactor`] for the policy contract.
    pub(crate) trace_redactor: Option<Arc<dyn crate::api::trace_redactor::TraceRedactor>>,
}

impl DialogAdapter {
    /// Create a new dialog adapter.
    ///
    /// `outbound_proxy_uri` is the RFC 3261 §8.1.2 outbound proxy, if any.
    /// Pass `None` for no pre-loaded Route. When `Some`, the URI MUST parse
    /// as a valid SIP URI — typically `sip:sbc.example.com;lr`.
    ///
    /// `outbound_contact_params` is the RFC 5626 §4 instance + reg-id pair
    /// attached to REGISTER Contact headers when outbound registration is
    /// enabled. Pass `None` for pre-5626 REGISTER Contact shape.
    // Preserve the established public constructor while the builder API is
    // introduced separately.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        dialog_api: Arc<UnifiedDialogApi>,
        store: Arc<SessionStore>,
        global_coordinator: Arc<GlobalEventCoordinator>,
        outbound_proxy_uri: Option<rvoip_sip_core::types::uri::Uri>,
        outbound_contact_params: Option<rvoip_sip_core::types::outbound::OutboundContactParams>,
        symmetric_flow_params: Option<rvoip_sip_core::types::outbound::OutboundContactParams>,
        registration_auto_refresh: bool,
        registration_refresh_jitter_percent: u8,
        auto_emit_extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
        trace_redactor: Option<Arc<dyn crate::api::trace_redactor::TraceRedactor>>,
    ) -> Self {
        let non_invite_transaction_timeout = dialog_api
            .dialog_manager()
            .core()
            .transaction_manager()
            .timer_settings()
            .transaction_timeout;
        Self {
            dialog_api,
            store,
            session_to_dialog: Arc::new(DashMap::new()),
            dialog_to_session: Arc::new(DashMap::new()),
            callid_to_session: Arc::new(DashMap::new()),
            outgoing_invite_tx: Arc::new(DashMap::new()),
            outgoing_bye_tx: Arc::new(DashMap::new()),
            outgoing_bye_generation_watch: Arc::new(DashMap::new()),
            next_outgoing_bye_generation: Arc::new(AtomicU64::new(1)),
            non_invite_transaction_timeout,
            outbound_request_tracker: OutboundInDialogRequestTracker::new(
                non_invite_transaction_timeout,
            ),
            outbound_initial_invites: Arc::new(DashMap::new()),
            data_message_dispatch_lanes: Arc::new(SipDataMessageDispatchLanes::default()),
            auto_emit_extra_headers,
            global_coordinator,
            state_machine: Arc::new(std::sync::OnceLock::new()),
            outbound_proxy_uri,
            outbound_contact_params,
            symmetric_flow_params,
            registration_auto_refresh,
            registration_refresh_jitter_percent,
            registration_refresh_admission: Arc::new(StdMutex::new(())),
            registration_refresh_tasks: Arc::new(DashMap::new()),
            registration_refresh_retained: RetainedTasks::new(),
            next_registration_refresh_generation: Arc::new(AtomicU64::new(1)),
            cleanup_attempt_total: Arc::new(AtomicU64::new(0)),
            cleanup_mapped_total: Arc::new(AtomicU64::new(0)),
            cleanup_missing_total: Arc::new(AtomicU64::new(0)),
            cleanup_call_ids_removed_total: Arc::new(AtomicU64::new(0)),
            cleanup_outgoing_invite_removed_total: Arc::new(AtomicU64::new(0)),
            #[cfg(test)]
            cleanup_pause: Arc::new(std::sync::Mutex::new(None)),
            trace_redactor,
        }
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn pause_next_cleanup_before_core_for_test(&self) -> Arc<DialogCleanupPause> {
        let pause = Arc::new(DialogCleanupPause::new());
        *self
            .cleanup_pause
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::clone(&pause));
        pause
    }

    /// Wire the state machine after construction. Idempotent — subsequent
    /// calls are silently ignored (returns `Err` if already set, which
    /// callers may choose to ignore or treat as a programming error).
    pub fn init_state_machine(
        &self,
        state_machine: Arc<crate::state_machine::StateMachine>,
    ) -> std::result::Result<(), Arc<crate::state_machine::StateMachine>> {
        self.state_machine.set(state_machine)
    }

    fn publish_api_event(&self, api_event: crate::api::events::Event) {
        let wrapped = crate::adapters::SessionApiCrossCrateEvent::new(api_event);
        let coordinator = self.global_coordinator.clone();
        tokio::spawn(async move {
            if let Err(e) = coordinator.publish(wrapped).await {
                tracing::warn!("Failed to publish app-level dialog adapter event: {}", e);
            }
        });
    }

    pub(crate) fn outbound_transport_context_for_uri(
        &self,
        request_uri: &str,
    ) -> crate::auth::SipTransportSecurityContext {
        let Ok(uri) = Uri::from_str(request_uri) else {
            return crate::auth::SipTransportSecurityContext::from_request_uri_transport_hint(
                request_uri,
            );
        };
        let transaction_manager = self
            .dialog_api
            .dialog_manager()
            .core()
            .transaction_manager();
        let transport = transaction_manager.get_best_transport_for_uri(&uri);
        let mut context =
            crate::auth::SipTransportSecurityContext::from_transport_name(transport.to_string());
        if let Some(info) = transaction_manager.get_transport_info(transport) {
            context.local_addr = info.local_addr.map(|addr| addr.to_string());
        }
        context
    }

    pub(crate) fn outbound_transport_context_for_response(
        &self,
        response: &Response,
        fallback_request_uri: &str,
    ) -> crate::auth::SipTransportSecurityContext {
        self.dialog_api
            .outbound_transport_context_for_response(response)
            .map(|context| {
                crate::auth::SipTransportSecurityContext::from_transport_context(&context)
            })
            .unwrap_or_else(|| self.outbound_transport_context_for_uri(fallback_request_uri))
    }

    pub(crate) fn abort_registration_refresh(&self, session_id: &SessionId) {
        let guard = cleanup_diag::stage_guard(CleanupStage::TimerTaskShutdown, &session_id.0);
        let _admission = self
            .registration_refresh_admission
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((_, task)) = self.registration_refresh_tasks.remove(session_id) {
            let _ = task.cancel.send(());
        }
        guard.finish_success();
    }

    /// Feature-gated retained-object counts for perf leak investigations.
    #[cfg(feature = "perf-tests")]
    pub(crate) fn perf_diagnostic_counts(&self) -> serde_json::Value {
        serde_json::json!({
            "session_to_dialog": self.session_to_dialog.len(),
            "dialog_to_session": self.dialog_to_session.len(),
            "callid_to_session": self.callid_to_session.len(),
            "outgoing_invite_tx": self.outgoing_invite_tx.len(),
            "outgoing_bye_tx": self.outgoing_bye_tx.len(),
            "outgoing_bye_generation_watch": self.outgoing_bye_generation_watch.len(),
            "outbound_initial_invites": self.outbound_initial_invites.len(),
            "registration_refresh_tasks": self.registration_refresh_tasks.len(),
            "lifecycle": {
                "cleanup_attempt_total": self.cleanup_attempt_total.load(Ordering::Relaxed),
                "cleanup_mapped_total": self.cleanup_mapped_total.load(Ordering::Relaxed),
                "cleanup_missing_total": self.cleanup_missing_total.load(Ordering::Relaxed),
                "cleanup_call_ids_removed_total": self.cleanup_call_ids_removed_total.load(Ordering::Relaxed),
                "cleanup_outgoing_invite_removed_total": self.cleanup_outgoing_invite_removed_total.load(Ordering::Relaxed),
            },
        })
    }

    pub(crate) async fn abort_all_registration_refreshes_and_wait(&self) -> Result<()> {
        let cleanup_guard = cleanup_diag::stage_guard(CleanupStage::TimerTaskShutdown, "all");
        {
            // This short synchronous gate makes close, replacement and task
            // publication one ordered admission history. Network work never
            // runs while the gate is held.
            let _admission = self
                .registration_refresh_admission
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            self.registration_refresh_retained.close();
            let session_ids: Vec<_> = self
                .registration_refresh_tasks
                .iter()
                .map(|entry| entry.key().clone())
                .collect();
            for session_id in session_ids {
                if let Some((_, task)) = self.registration_refresh_tasks.remove(&session_id) {
                    let _ = task.cancel.send(());
                }
            }
        }
        cleanup_guard.finish_success();
        if tokio::time::timeout(
            REGISTRATION_REFRESH_DRAIN_TIMEOUT,
            self.registration_refresh_retained.wait_idle(),
        )
        .await
        .is_err()
        {
            return Err(SessionError::InternalError(format!(
                "registration refresh drain timed out with {} retained tasks",
                self.registration_refresh_retained.count()
            )));
        }
        if self.registration_refresh_retained.panicked() {
            return Err(SessionError::InternalError(
                "registration refresh task panicked during drain".to_string(),
            ));
        }
        Ok(())
    }

    fn compute_registration_refresh_at(&self, now: Instant, accepted_expires: u32) -> Instant {
        let base_secs = ((accepted_expires as f64) * 0.85).floor().max(1.0) as u64;
        let jitter_cap_secs =
            (base_secs * u64::from(self.registration_refresh_jitter_percent)) / 100;
        let jitter_secs = if jitter_cap_secs == 0 {
            0
        } else {
            use rand::Rng;
            rand::thread_rng().gen_range(0..=jitter_cap_secs)
        };
        now + Duration::from_secs(base_secs.saturating_sub(jitter_secs).max(1))
    }

    fn schedule_registration_refresh(
        &self,
        session_id: SessionId,
        next_refresh_at: Option<Instant>,
    ) {
        let state_machine = self.state_machine.get().cloned();
        let _admission = self
            .registration_refresh_admission
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if let Some((_, previous)) = self.registration_refresh_tasks.remove(&session_id) {
            let _ = previous.cancel.send(());
        }
        if !self.registration_auto_refresh {
            return;
        }
        let Some(next_refresh_at) = next_refresh_at else {
            return;
        };
        let Some(state_machine) = state_machine else {
            return;
        };

        let generation = self
            .next_registration_refresh_generation
            .fetch_add(1, Ordering::Relaxed);
        let (cancel, mut cancelled) = tokio::sync::oneshot::channel();
        let session_id_for_task = session_id.clone();
        let completion_admission = Arc::clone(&self.registration_refresh_admission);
        let completion_tasks = Arc::clone(&self.registration_refresh_tasks);
        let adapter = self.clone();
        let spawned = self.registration_refresh_retained.spawn(async move {
            // Construct inside the spawned future. If admission rejects and
            // drops the unpolled future while the caller owns the gate, there
            // is no completion destructor trying to reacquire that gate.
            let _completion = RegistrationRefreshCompletion {
                admission: completion_admission,
                tasks: completion_tasks,
                session_id: session_id_for_task.clone(),
                generation,
            };
            let refresh = async {
                tokio::time::sleep_until(tokio::time::Instant::from_std(next_refresh_at)).await;

                match state_machine
                    .process_event(
                        &session_id_for_task,
                        crate::state_table::types::EventType::RefreshRegistration,
                    )
                    .await
                {
                    Ok(result) if result.transition.is_some() => {}
                    Ok(_) => {
                        tracing::warn!(
                            "Automatic registration refresh had no state-table transition for session {}; falling back to direct REGISTER",
                            session_id_for_task
                        );
                        if let Err(e) = adapter
                            .send_registration_refresh_direct(&session_id_for_task)
                            .await
                        {
                            tracing::warn!(
                                "Automatic direct registration refresh failed for session {}: {}",
                                session_id_for_task,
                                e
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Automatic registration refresh failed for session {}: {}",
                            session_id_for_task,
                            e
                        );
                    }
                }
            };
            tokio::select! {
                biased;
                _ = &mut cancelled => return,
                _ = refresh => {}
            }
        });
        if spawned {
            let replaced = self
                .registration_refresh_tasks
                .insert(session_id, RegistrationRefreshTask { generation, cancel });
            debug_assert!(replaced.is_none());
        }
    }

    async fn send_registration_refresh_direct(&self, session_id: &SessionId) -> Result<()> {
        let refresh = self.store.with_session(session_id, |session| {
            if !session.is_registered {
                return Ok::<_, SessionError>(None);
            }
            let from_uri = session.local_uri.clone().ok_or_else(|| {
                SessionError::InternalError("local_uri not set for refresh".into())
            })?;
            let registrar_uri = session
                .registrar_uri
                .clone()
                .or_else(|| session.remote_uri.clone())
                .ok_or_else(|| {
                    SessionError::InternalError("registrar_uri not set for refresh".into())
                })?;
            let contact_uri = session
                .registration_contact
                .clone()
                .or_else(|| session.local_uri.clone())
                .ok_or_else(|| {
                    SessionError::InternalError("contact_uri not set for refresh".into())
                })?;
            let expires = session.registration_expires.unwrap_or(3600);
            let auth = session
                .auth
                .clone()
                .or_else(|| session.credentials.clone().map(Into::into));
            Ok::<_, SessionError>(Some((from_uri, registrar_uri, contact_uri, expires, auth)))
        })??;
        let Some((from_uri, registrar_uri, contact_uri, expires, auth)) = refresh else {
            tracing::debug!(
                "Skipping automatic registration refresh for unregistered session {}",
                session_id
            );
            return Ok(());
        };

        let mut attempt_expires = expires;
        for _ in 0..=2 {
            match self
                .send_register(
                    session_id,
                    &registrar_uri,
                    &from_uri,
                    &contact_uri,
                    attempt_expires,
                    auth.as_ref(),
                    Vec::new(),
                )
                .await?
            {
                RegisterAttemptOutcome::Registered {
                    accepted_expires,
                    metadata,
                } => {
                    return self
                        .apply_registration_success(
                            session_id,
                            &registrar_uri,
                            &from_uri,
                            &contact_uri,
                            accepted_expires,
                            metadata,
                        )
                        .await;
                }
                RegisterAttemptOutcome::IntervalTooBrief { min_expires } => {
                    self.store
                        .update_session_with(session_id, |session| {
                            session.registration_expires = Some(min_expires);
                            session.registration_retry_count += 1;
                        })
                        .await?;
                    attempt_expires = min_expires;
                }
                RegisterAttemptOutcome::Failure {
                    status_code,
                    reason,
                } => {
                    return self
                        .apply_registration_failure(session_id, &registrar_uri, status_code, reason)
                        .await;
                }
                RegisterAttemptOutcome::AuthChallenge { status_code, .. } => {
                    return self
                        .apply_registration_failure(
                            session_id,
                            &registrar_uri,
                            status_code,
                            "automatic registration refresh received a new auth challenge",
                        )
                        .await;
                }
                RegisterAttemptOutcome::Unregistered => {
                    return self
                        .apply_unregistration_success(session_id, &registrar_uri)
                        .await;
                }
            }
        }

        self.apply_registration_failure(
            session_id,
            &registrar_uri,
            423,
            "registration refresh failed with repeated 423 Interval Too Brief responses",
        )
        .await
    }

    pub(crate) fn accepted_registration_expires(
        response: &Response,
        requested_contact_uri: &str,
        fallback_expires: u32,
    ) -> u32 {
        use rvoip_sip_core::types::headers::HeaderAccess;
        use rvoip_sip_core::types::{header::HeaderName, TypedHeader};

        let requested = requested_contact_uri.trim().trim_matches(['<', '>']);

        let mut first_contact_expires = None;
        for contact in response.headers.iter().filter_map(|header| match header {
            TypedHeader::Contact(contact) => Some(contact),
            _ => None,
        }) {
            for address in contact.addresses() {
                let expires = address
                    .get_param("expires")
                    .flatten()
                    .and_then(|value| value.parse::<u32>().ok());
                if first_contact_expires.is_none() {
                    first_contact_expires = expires;
                }
                if address.uri.to_string() == requested {
                    if let Some(expires) = expires {
                        return expires;
                    }
                }
            }
        }

        first_contact_expires
            .or_else(|| {
                response
                    .raw_header_value(&HeaderName::Expires)
                    .and_then(|value| value.trim().parse::<u32>().ok())
            })
            .unwrap_or(fallback_expires)
    }

    pub(crate) fn response_registration_metadata(
        response: &Response,
    ) -> RegistrationResponseMetadata {
        use rvoip_sip_core::types::outbound::read_gruu_contact_params;
        use rvoip_sip_core::types::TypedHeader;

        let service_route = {
            let routes: Vec<String> = response
                .headers
                .iter()
                .filter_map(|header| match header {
                    TypedHeader::ServiceRoute(route) => Some(route.uris()),
                    _ => None,
                })
                .flatten()
                .map(|uri| uri.to_string())
                .collect();
            if routes.is_empty() {
                None
            } else {
                Some(routes)
            }
        };

        let mut pub_gruu = None;
        let mut temp_gruu = None;
        for contact in response.headers.iter().filter_map(|header| match header {
            TypedHeader::Contact(contact) => Some(contact),
            _ => None,
        }) {
            for address in contact.addresses() {
                let params = read_gruu_contact_params(address);
                if pub_gruu.is_none() {
                    pub_gruu = params.pub_gruu;
                }
                if temp_gruu.is_none() {
                    temp_gruu = params.temp_gruu;
                }
            }
        }

        RegistrationResponseMetadata {
            service_route,
            pub_gruu,
            temp_gruu,
            transport_route: None,
        }
    }

    pub(crate) fn register_attempt_outcome_from_response(
        response: &Response,
        contact_uri: &str,
        expires: u32,
    ) -> RegisterAttemptOutcome {
        match response.status_code() {
            200..=299 => {
                if expires == 0 {
                    RegisterAttemptOutcome::Unregistered
                } else {
                    RegisterAttemptOutcome::Registered {
                        accepted_expires: Self::accepted_registration_expires(
                            response,
                            contact_uri,
                            expires,
                        ),
                        metadata: Self::response_registration_metadata(response),
                    }
                }
            }
            401 | 407 => {
                use rvoip_sip_core::types::headers::HeaderAccess;
                let header_name = if response.status_code() == 407 {
                    rvoip_sip_core::types::header::HeaderName::ProxyAuthenticate
                } else {
                    rvoip_sip_core::types::header::HeaderName::WwwAuthenticate
                };
                if let Some(challenge) = response.raw_header_value(&header_name) {
                    RegisterAttemptOutcome::AuthChallenge {
                        status_code: response.status_code(),
                        challenge,
                    }
                } else {
                    RegisterAttemptOutcome::Failure {
                        status_code: response.status_code(),
                        reason: "REGISTER challenge response did not include challenge header"
                            .to_string(),
                    }
                }
            }
            423 => {
                use rvoip_sip_core::types::headers::HeaderAccess;
                match response
                    .raw_header_value(&rvoip_sip_core::types::header::HeaderName::MinExpires)
                    .and_then(|s| s.trim().parse::<u32>().ok())
                {
                    Some(min_expires) if min_expires > 0 && min_expires <= 7200 => {
                        RegisterAttemptOutcome::IntervalTooBrief { min_expires }
                    }
                    Some(min_expires) => RegisterAttemptOutcome::Failure {
                        status_code: response.status_code(),
                        reason: format!(
                            "423 Interval Too Brief included invalid Min-Expires={}",
                            min_expires
                        ),
                    },
                    None => RegisterAttemptOutcome::Failure {
                        status_code: response.status_code(),
                        reason: "423 Interval Too Brief without Min-Expires header".to_string(),
                    },
                }
            }
            _ => RegisterAttemptOutcome::Failure {
                status_code: response.status_code(),
                reason: response.reason_phrase().to_string(),
            },
        }
    }

    pub(crate) async fn apply_registration_success(
        &self,
        session_id: &SessionId,
        registrar_uri: &str,
        from_uri: &str,
        contact_uri: &str,
        accepted_expires: u32,
        metadata: RegistrationResponseMetadata,
    ) -> Result<()> {
        let now = Instant::now();
        let next_refresh_at = if self.registration_auto_refresh && accepted_expires > 0 {
            Some(self.compute_registration_refresh_at(now, accepted_expires))
        } else {
            None
        };

        let registration_route = metadata.transport_route;
        self.store
            .update_session_with(session_id, |session| {
                session.is_registered = true;
                session.registration_expires = Some(accepted_expires);
                session.registration_accepted_expires = Some(accepted_expires);
                session.registration_registered_at = Some(now);
                session.registration_next_refresh_at = next_refresh_at;
                session.registration_last_failure = None;
                session.registration_retry_count = 0;
                session.registration_service_route = metadata.service_route;
                session.registration_pub_gruu = metadata.pub_gruu;
                session.registration_temp_gruu = metadata.temp_gruu;
            })
            .await?;

        tracing::info!(
            "✅ Registration successful - session {} marked as registered",
            session_id.0
        );
        self.publish_api_event(crate::api::events::Event::RegistrationSuccess {
            registrar: registrar_uri.to_string(),
            expires: accepted_expires,
            contact: contact_uri.to_string(),
        });
        self.schedule_registration_refresh(session_id.clone(), next_refresh_at);
        self.start_symmetric_registration_keepalive(from_uri, registration_route);
        Ok(())
    }

    pub(crate) async fn apply_unregistration_success(
        &self,
        session_id: &SessionId,
        registrar_uri: &str,
    ) -> Result<()> {
        self.abort_registration_refresh(session_id);
        self.store
            .update_session_with(session_id, |session| {
                session.is_registered = false;
                session.registration_accepted_expires = None;
                session.registration_registered_at = None;
                session.registration_next_refresh_at = None;
                session.registration_last_failure = None;
                session.registration_retry_count = 0;
                session.registration_service_route = None;
                session.registration_pub_gruu = None;
                session.registration_temp_gruu = None;
            })
            .await?;

        tracing::info!(
            "✅ Unregistration successful - session {} marked as unregistered",
            session_id.0
        );
        self.publish_api_event(crate::api::events::Event::UnregistrationSuccess {
            registrar: registrar_uri.to_string(),
        });
        Ok(())
    }

    pub(crate) async fn apply_registration_failure(
        &self,
        session_id: &SessionId,
        registrar_uri: &str,
        status_code: u16,
        reason: impl Into<String>,
    ) -> Result<()> {
        self.abort_registration_refresh(session_id);
        let reason = reason.into();
        let failure_summary = self
            .store
            .update_session_with(session_id, |session| {
                let failure_summary = if session.registration_retry_count > 0 {
                    format!(
                        "{} after {} retry attempt(s)",
                        reason, session.registration_retry_count
                    )
                } else {
                    reason.clone()
                };
                session.is_registered = false;
                session.registration_accepted_expires = None;
                session.registration_registered_at = None;
                session.registration_next_refresh_at = None;
                session.registration_last_failure = Some(failure_summary.clone());
                session.registration_service_route = None;
                session.registration_pub_gruu = None;
                session.registration_temp_gruu = None;
                failure_summary
            })
            .await?;

        self.publish_api_event(crate::api::events::Event::RegistrationFailed {
            registrar: registrar_uri.to_string(),
            status_code,
            reason: failure_summary,
        });
        Ok(())
    }

    pub(crate) async fn apply_unregistration_failure(
        &self,
        session_id: &SessionId,
        registrar_uri: &str,
        reason: impl Into<String>,
    ) -> Result<()> {
        self.abort_registration_refresh(session_id);
        let reason = reason.into();
        self.store
            .update_session_with(session_id, |session| {
                session.is_registered = false;
                session.registration_accepted_expires = None;
                session.registration_registered_at = None;
                session.registration_next_refresh_at = None;
                session.registration_last_failure = Some(reason.clone());
                session.registration_retry_count = 0;
                session.registration_service_route = None;
                session.registration_pub_gruu = None;
                session.registration_temp_gruu = None;
            })
            .await?;

        self.publish_api_event(crate::api::events::Event::UnregistrationFailed {
            registrar: registrar_uri.to_string(),
            reason,
        });
        Ok(())
    }

    // ===== Direct Dialog Operations =====
    // NOTE: Removed confusing create_dialog() and send_invite() methods
    // Use send_invite_with_details() to create a dialog and send INVITE in one operation

    /// Send a response
    pub async fn send_response_by_dialog(
        &self,
        _dialog_id: DialogId,
        status_code: u16,
        _reason: &str,
    ) -> Result<()> {
        // We can't really convert a string to RvoipDialogId which wraps a UUID
        // This method needs to be rethought - for now just return Ok
        // since this is called from places where we have only our DialogId
        tracing::warn!(
            "send_response_by_dialog called but conversion not implemented - status: {}",
            status_code
        );
        Ok(())
    }

    /// Send BYE for a specific dialog
    pub async fn send_bye(&self, dialog_id: crate::types::DialogId) -> Result<()> {
        // Convert our DialogId to RvoipDialogId
        let rvoip_dialog_id: RvoipDialogId = dialog_id.into();

        // Find session ID from dialog
        if let Some(entry) = self.dialog_to_session.get(&rvoip_dialog_id) {
            let session_id = entry.value().clone();
            drop(entry);

            self.mark_initial_invite_protocol_teardown(&session_id);

            // Send BYE through dialog API
            let request_uri = self
                .dialog_api
                .dialog_manager()
                .core()
                .get_dialog(&rvoip_dialog_id)
                .map_err(|_| {
                    SessionError::InvalidTransition(
                        "SIP BYE exact dialog is no longer available".to_string(),
                    )
                })?
                .remote_target
                .to_string();
            let generation = self.next_outgoing_bye_generation();
            let (transaction_id, completion) = self
                .dialog_api
                .send_bye_with_options_and_completion(
                    &rvoip_dialog_id,
                    ByeRequestOptions::default(),
                )
                .await
                .map_err(|e| SessionError::DialogError(format!("Failed to send BYE: {}", e)))?;
            self.retain_outgoing_bye_transaction(
                &session_id,
                generation,
                transaction_id,
                completion,
                request_uri,
            );
            self.wait_for_outgoing_bye_final_response(&session_id)
                .await?;

            tracing::info!("Sent BYE for session {}", session_id.0);
        } else {
            tracing::warn!("No session found for dialog {}", dialog_id);
        }

        Ok(())
    }

    /// Send re-INVITE with new SDP
    pub async fn send_reinvite(
        &self,
        dialog_id: crate::types::DialogId,
        sdp: String,
    ) -> Result<()> {
        // Convert our DialogId to RvoipDialogId
        let rvoip_dialog_id: RvoipDialogId = dialog_id.into();

        // Find session ID from dialog
        if let Some(entry) = self.dialog_to_session.get(&rvoip_dialog_id) {
            let session_id = entry.value().clone();
            drop(entry);

            // Use UPDATE method for re-INVITE
            self.dialog_api
                .send_update_with_options(
                    &rvoip_dialog_id,
                    UpdateRequestOptions {
                        sdp: Some(sdp),
                        ..Default::default()
                    },
                )
                .await
                .map_err(|error| {
                    redacted_invite_dispatch_error(
                        InviteDispatchFailure::LegacyUpdateReinvite,
                        error,
                    )
                })?;

            tracing::info!("Sent re-INVITE for session {}", session_id.0);
        } else {
            tracing::warn!("No session found for dialog {}", dialog_id);
        }

        Ok(())
    }

    /// Send REFER for transfers
    pub async fn send_refer(
        &self,
        dialog_id: crate::types::DialogId,
        target: &str,
        attended: bool,
    ) -> Result<()> {
        // Convert our DialogId to RvoipDialogId
        let rvoip_dialog_id: RvoipDialogId = dialog_id.into();

        // Find session ID from dialog
        if let Some(entry) = self.dialog_to_session.get(&rvoip_dialog_id) {
            let session_id = entry.value().clone();
            drop(entry);

            // Send REFER through dialog API. Attended-transfer Replaces
            // belongs in `ReferRequestOptions.replaces` (an RFC 3891
            // header param on Refer-To), not as a REFER body — the
            // legacy code passed the literal "attended" string, which
            // had no on-wire effect. Attended transfers route through
            // `send_refer_with_options` paths that compute Replaces
            // from the held session's dialog identifiers.
            let _ = attended;
            self.dialog_api
                .send_refer_with_options(
                    &rvoip_dialog_id,
                    ReferRequestOptions {
                        refer_to: target.to_string(),
                        ..Default::default()
                    },
                )
                .await
                .map_err(|error| redacted_dialog_operation_error("REFER", error))?;

            tracing::info!(
                session = %session_id.0,
                target_present = !target.is_empty(),
                target_bytes = target.len(),
                "Sent REFER"
            );
        } else {
            tracing::warn!("No session found for dialog {}", dialog_id);
        }

        Ok(())
    }

    /// Get remote URI for a dialog
    pub async fn get_remote_uri(&self, _dialog_id: crate::types::DialogId) -> Result<String> {
        // For now, return a placeholder
        Ok("sip:remote@example.com".to_string())
    }

    /// RFC 3261 §22.2 — resend an INVITE with `Authorization` (or
    /// `Proxy-Authorization`) header on the same dialog after the server
    /// challenged with 401/407. The `SendINVITEWithAuth` state-machine action
    /// owns auth header computation; this is a thin passthrough to dialog-core.
    ///
    /// Both REGISTER and INVITE 401/407 challenges flow through the state
    /// machine via `DialogToSessionEvent::AuthRequired` → `EventType::AuthRequired`;
    /// the previous inline REGISTER-auth shortcut (`handle_401_challenge`) was
    /// retired when INVITE auth landed. See `default.yaml`'s `Initiating` /
    /// `Registering` + `AuthRequired` transitions.
    pub async fn resend_invite_with_auth(
        &self,
        session_id: &SessionId,
        mut opts: rvoip_sip_dialog::api::unified::InviteAuthRetryOptions,
        apply_global_proxy: bool,
    ) -> Result<()> {
        // Compatibility wait: the staged initial-INVITE path now installs
        // this mapping before wire dispatch, but retained responses from an
        // older call path may still race a rolling upgrade. Poll briefly so
        // the retry can reuse the exact dialog instead of failing spuriously.
        use tokio::time::{Duration, Instant};
        let start = Instant::now();
        let dialog_id = loop {
            if let Some(entry) = self.session_to_dialog.get(session_id) {
                break entry.value().clone();
            }
            if start.elapsed() >= Duration::from_secs(1) {
                return Err(SessionError::SessionNotFound(session_id.0.clone()));
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };

        // Legacy/internal paths may not have a persisted per-call override.
        // In that case retain the configured global proxy structurally; never
        // synthesize it as a transient application Route header.
        if apply_global_proxy && opts.outbound_proxy_uri.is_none() {
            opts.outbound_proxy_uri = self.outbound_proxy_uri.clone();
        }
        self.dialog_api
            .send_invite_with_auth_options(&dialog_id, opts)
            .await
            .map_err(|error| {
                redacted_invite_dispatch_error(InviteDispatchFailure::AuthRetry, error)
            })?;
        Ok(())
    }

    /// RFC 4028 §6 — resend an INVITE with a bumped `Session-Expires` /
    /// `Min-SE` after a 422 Session Interval Too Small. The UAS's Min-SE
    /// floor is supplied by the caller (parsed from the 422 response by
    /// dialog-core). The timer headers bypass
    /// [`DialogManagerConfig`](rvoip_sip_dialog::config::DialogManagerConfig)'s
    /// global values and use these overrides verbatim.
    pub async fn resend_invite_with_session_timer_override(
        &self,
        session_id: &SessionId,
        mut opts: rvoip_sip_dialog::api::unified::InviteAuthRetryOptions,
        apply_global_proxy: bool,
        session_secs: u32,
        min_se: u32,
    ) -> Result<()> {
        // Compatibility wait for retained responses from the pre-staged
        // initial-INVITE path during rolling upgrades. New calls install the
        // exact s2d mapping before wire dispatch. Cap the wait at 1s; timeout
        // propagates as `SessionNotFound` and becomes terminal `CallFailed`.
        use tokio::time::{Duration, Instant};
        let start = Instant::now();
        let dialog_id = loop {
            if let Some(entry) = self.session_to_dialog.get(session_id) {
                break entry.value().clone();
            }
            if start.elapsed() >= Duration::from_secs(1) {
                return Err(SessionError::SessionNotFound(session_id.0.clone()));
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };

        if apply_global_proxy && opts.outbound_proxy_uri.is_none() {
            opts.outbound_proxy_uri = self.outbound_proxy_uri.clone();
        }
        self.dialog_api
            .send_invite_with_session_timer_options(&dialog_id, opts, session_secs, min_se)
            .await
            .map_err(|error| {
                redacted_invite_dispatch_error(InviteDispatchFailure::SessionTimerRetry, error)
            })?;
        Ok(())
    }

    /// Does the remote peer support RFC 3262 100rel? Used to gate
    /// `send_early_media` — we only emit a reliable 183 when the caller
    /// advertised `Supported: 100rel` (or `Require: 100rel`) on the INVITE.
    /// Returns `SessionNotFound` if the session has no dialog yet.
    pub async fn peer_supports_100rel(&self, session_id: &SessionId) -> Result<bool> {
        let dialog_id = self
            .session_to_dialog
            .get(session_id)
            .map(|e| e.value().clone())
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?;

        let dialog = self
            .dialog_api
            .get_dialog_info(&dialog_id)
            .await
            .map_err(|e| {
                SessionError::DialogError(format!(
                    "peer_supports_100rel: failed to read dialog {}: {}",
                    dialog_id, e
                ))
            })?;

        Ok(dialog.peer_supports_100rel)
    }

    // ===== Outbound Actions (from state machine) =====

    /// Record that the accepted upper-layer transition owns protocol teardown.
    ///
    /// Call this immediately before awaiting CANCEL/BYE dispatch. A loopback
    /// peer can return a final response during that await and drive exact
    /// session release before the send future unwinds. Marking afterward loses
    /// the retained initial-INVITE owner and can make shutdown supervise a
    /// teardown that the upper layer already sent. A dispatch error remains an
    /// ambiguous at-most-once attempt, so lower cleanup must not synthesize a
    /// competing CANCEL/BYE in that case either.
    pub(crate) fn mark_initial_invite_protocol_teardown(&self, session_id: &SessionId) {
        let Some(binding) = self.outbound_initial_invites.get(session_id) else {
            return;
        };
        if let Some(resource) = binding.resource.upgrade() {
            resource.mark_protocol_teardown_owned_by_upper();
        }
    }

    async fn send_initial_invite_staged(
        &self,
        session_id: &SessionId,
        opts: rvoip_sip_dialog::api::unified::InviteRequestOptions,
        failure: InviteDispatchFailure,
    ) -> Result<()> {
        tracing::trace!(
            session_id = %session_id,
            operation = failure.diagnostic(),
            "staged initial INVITE entering planner"
        );
        let handle = self.store.lifecycle_handle(session_id).ok_or_else(|| {
            SessionError::SessionNotFound(format!(
                "Session {} has no current lifecycle handle",
                session_id.0
            ))
        })?;
        let plan = self
            .dialog_api
            .plan_initial_invite(Some(session_id.0.clone()), opts)
            .await
            .map_err(|error| redacted_invite_dispatch_error(failure, error))?;
        tracing::trace!(
            session_id = %session_id,
            operation = failure.diagnostic(),
            "staged initial INVITE plan ready"
        );
        let resource =
            OutboundInitialInviteResource::new(self, handle.clone(), plan.owner().clone());
        let dialog_api = Arc::clone(&self.dialog_api);
        let authority = Arc::clone(self.store.authority());
        let operation_resource = Arc::clone(&resource);

        let waiter = authority
            .spawn_owned_exact(
                handle.key(),
                SessionOperationKind::Signaling,
                INITIAL_INVITE_OWNED_OPERATION_TIMEOUT,
                move |mut operation| async move {
                    tracing::trace!("staged initial INVITE owned operation started");
                    let spec = ResourceSpec::new(
                        operation_resource.descriptor(),
                        Vec::new(),
                        INITIAL_INVITE_RESOURCE_RELEASE_TIMEOUT,
                    )
                    .unwrap_or_else(|_| panic!("initial INVITE resource spec is invalid"));
                    let attempt = match operation.reserve_resource(spec) {
                        Ok(attempt) => attempt,
                        Err(_) => {
                            return rollback_owned_invite(
                                operation,
                                Err(SessionError::InternalError(
                                    "initial INVITE resource reservation failed (class=lifecycle)"
                                        .to_string(),
                                )),
                            )
                            .await;
                        }
                    };
                    let installation_sink = attempt
                        .dispatch()
                        .unwrap_or_else(|_| panic!("initial INVITE dispatch permit failed"))
                        .into_installation_sink();
                    // `install_initial_invite_with_sink` may reject a plan
                    // before invoking its sink (for example, when an exact
                    // session mapping is still occupied). Keep the sink in a
                    // shared single-use slot so that path can prove the
                    // reservation unused instead of dropping it as an
                    // unresolvable lifecycle orphan.
                    let installation_sink = Arc::new(std::sync::Mutex::new(Some(
                        installation_sink,
                    )));
                    let callback_installation_sink = Arc::clone(&installation_sink);
                    let sink_resource = Arc::clone(&operation_resource);
                    let installed = match dialog_api.install_initial_invite_with_sink(
                        plan,
                        move |_installed| {
                            let installation_sink = callback_installation_sink
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                .take()
                                .ok_or_else(|| rvoip_sip_dialog::ApiError::Dialog {
                                    message: "Initial INVITE lifecycle sink was already resolved"
                                        .to_string(),
                                })?;
                            installation_sink
                                .capture_at_install(
                                    Arc::clone(&sink_resource)
                                        as Arc<dyn ManagedSessionResource>,
                                )
                                .map_err(|_| rvoip_sip_dialog::ApiError::Dialog {
                                    message: "Initial INVITE lifecycle capture failed".to_string(),
                                })?;
                            sink_resource.install_adapter_bindings().map_err(|_| {
                                rvoip_sip_dialog::ApiError::Dialog {
                                    message: "Initial INVITE adapter binding failed".to_string(),
                                }
                            })
                        },
                    ) {
                        Ok(installed) => {
                            tracing::trace!("staged initial INVITE installed");
                            installed
                        }
                        Err(error) => {
                            tracing::trace!("staged initial INVITE installation rejected");
                            let unused_sink = installation_sink
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                .take();
                            if let Some(unused_sink) = unused_sink {
                                if unused_sink.confirm_unused().is_err() {
                                    return rollback_owned_invite(
                                        operation,
                                        Err(SessionError::InternalError(
                                            "initial INVITE unused reservation confirmation failed (class=lifecycle)"
                                                .to_string(),
                                        )),
                                    )
                                    .await;
                                }
                            }
                            return rollback_owned_invite(
                                operation,
                                Err(redacted_invite_dispatch_error(failure, error)),
                            )
                            .await;
                        }
                    };

                    tracing::trace!("staged initial INVITE dispatch starting");
                    let completion = dialog_api.dispatch_initial_invite(installed).wait().await;
                    tracing::trace!("staged initial INVITE dispatch completed");
                    let wire_outcome = completion.wire_outcome();
                    operation_resource.record_wire_outcome(wire_outcome);
                    match completion.into_result() {
                        Ok((owner, transaction_id)) => {
                            if owner != operation_resource.owner {
                                panic!("initial INVITE dispatch returned a different exact owner");
                            }
                            if !operation_resource.install_transaction(transaction_id) {
                                return commit_owned_invite(
                                    operation,
                                    Err(SessionError::InternalError(
                                        "initial INVITE transaction binding failed (class=lifecycle)"
                                            .to_string(),
                                    )),
                                )
                                .await;
                            }
                            commit_owned_invite(operation, Ok(())).await
                        }
                        Err(error) => {
                            let value = Err(redacted_invite_dispatch_error(failure, error));
                            match wire_outcome {
                                InitialInviteWireOutcome::ZeroWire => {
                                    rollback_owned_invite(operation, value).await
                                }
                                InitialInviteWireOutcome::Sent
                                | InitialInviteWireOutcome::Unknown => {
                                    commit_owned_invite(operation, value).await
                                }
                            }
                        }
                    }
                },
            )
            .map_err(|_| {
                SessionError::InternalError(
                    "initial INVITE owned operation admission failed (class=lifecycle)".to_string(),
                )
            })?;

        let result = waiter.await.map_err(|_| {
            SessionError::InternalError(
                "initial INVITE owned operation failed (class=lifecycle)".to_string(),
            )
        })?;
        if result.is_ok() {
            tracing::debug!(
                session_id = %session_id,
                dialog_id = %resource.owner.dialog_id(),
                "staged initial INVITE committed with exact lifecycle ownership"
            );
        }
        result
    }

    /// Send INVITE for UAC - this is the primary method for initiating calls
    ///
    /// This method:
    /// 1. Creates a dialog in dialog-core
    /// 2. Sends the INVITE request
    /// 3. Stores the session-to-dialog mapping
    ///
    /// # Arguments
    /// * `session_id` - The session ID from the state machine
    /// * `from` - The From URI (e.g., "sip:alice@example.com")
    /// * `to` - The To URI (e.g., "sip:bob@example.com")
    /// * `sdp` - Optional SDP offer
    pub async fn send_invite_with_details(
        &self,
        session_id: &SessionId,
        from: &str,
        to: &str,
        sdp: Option<String>,
    ) -> Result<()> {
        let call_id = deterministic_outbound_call_id(session_id);
        self.send_initial_invite_staged(
            session_id,
            rvoip_sip_dialog::api::unified::InviteRequestOptions {
                from_uri: from.to_string(),
                to_uri: to.to_string(),
                sdp,
                call_id: Some(call_id),
                ..Default::default()
            },
            InviteDispatchFailure::Initial,
        )
        .await
    }

    /// Like [`send_invite_with_details`](Self::send_invite_with_details) but appends caller-supplied extra
    /// headers to the outgoing INVITE. Routes through dialog-core's
    /// `make_call_with_extra_headers_for_session` so the extras (typically
    /// `P-Asserted-Identity` per RFC 3325) ride on the very first wire
    /// transmission rather than being added in a follow-up.
    ///
    /// Used by the `SendINVITE` action when `SessionState.pai_uri` is set;
    /// the action handler builds the typed PAI header from the URI and
    /// passes it through here.
    pub async fn send_invite_with_extra_headers(
        &self,
        session_id: &SessionId,
        from: &str,
        to: &str,
        sdp: Option<String>,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> Result<()> {
        self.send_invite_with_extra_headers_inner(
            session_id,
            from,
            to,
            sdp,
            extra_headers,
            true, // apply global outbound-proxy Route
        )
        .await
    }

    /// SIP_API_DESIGN_2 §6.1 — variant used by builder dispatch when
    /// the builder has set its own per-call `with_outbound_proxy(uri)`
    /// structural override in `Action::SendINVITEWithOptions`. Skips the global
    /// `Config.outbound_proxy_uri` so the wire doesn't end up with two
    /// stacked proxy Routes when the caller meant to override the
    /// default.
    pub async fn send_invite_with_extra_headers_no_global_proxy(
        &self,
        session_id: &SessionId,
        from: &str,
        to: &str,
        sdp: Option<String>,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> Result<()> {
        self.send_invite_with_extra_headers_inner(session_id, from, to, sdp, extra_headers, false)
            .await
    }

    async fn send_invite_with_extra_headers_inner(
        &self,
        session_id: &SessionId,
        from: &str,
        to: &str,
        sdp: Option<String>,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
        apply_global_proxy: bool,
    ) -> Result<()> {
        let call_id = deterministic_outbound_call_id(session_id);

        // The proxy is a structural first hop, not an application Route
        // header. This guarantees it precedes REGISTER Service-Route entries
        // and lets dialog-core reject caller-controlled Route injection.
        let outbound_proxy_uri = if apply_global_proxy {
            self.outbound_proxy_uri.clone()
        } else {
            None
        };
        if apply_global_proxy && self.outbound_proxy_uri.is_some() {
            tracing::debug!(
                "E4 outbound proxy: staged structural first-hop route for INVITE session {}",
                session_id.0
            );
        }

        let opts = rvoip_sip_dialog::api::unified::InviteRequestOptions {
            from_uri: from.to_string(),
            to_uri: to.to_string(),
            sdp,
            call_id: Some(call_id),
            from_display: None,
            contact_uri: None,
            precomputed_authorization: None,
            outbound_proxy_uri,
            supported_100rel: false,
            extra_headers,
        };
        self.send_initial_invite_staged(
            session_id,
            opts,
            InviteDispatchFailure::InitialWithExtraHeaders,
        )
        .await
    }

    /// SIP_API_DESIGN_2 Phase B — structured initial-INVITE dispatch. The
    /// rvoip-sip counterpart of dialog-core `send_invite_with_options`: carries
    /// the `From` display name and `Contact` as typed fields instead of
    /// smuggling them through `extra_headers`. `apply_global_proxy` follows the
    /// same rule as [`Self::send_invite_with_extra_headers`] — skip the global
    /// `Config.outbound_proxy_uri` when the builder set a structural per-call
    /// override in `opts.outbound_proxy_uri`.
    pub async fn send_invite_with_options(
        &self,
        session_id: &SessionId,
        mut opts: rvoip_sip_dialog::api::unified::InviteRequestOptions,
        apply_global_proxy: bool,
    ) -> Result<()> {
        let call_id = deterministic_outbound_call_id(session_id);

        if apply_global_proxy && opts.outbound_proxy_uri.is_none() {
            opts.outbound_proxy_uri = self.outbound_proxy_uri.clone();
            if opts.outbound_proxy_uri.is_some() {
                tracing::debug!(
                    "E4 outbound proxy: staged structural first-hop route for INVITE session {}",
                    session_id.0
                );
            }
        }
        opts.call_id = Some(call_id);
        self.send_initial_invite_staged(session_id, opts, InviteDispatchFailure::InitialWithOptions)
            .await
    }

    /// Send 200 OK response
    pub async fn send_200_ok(&self, session_id: &SessionId, sdp: Option<String>) -> Result<()> {
        self.send_response(session_id, 200, sdp).await
    }

    /// Send response with SDP
    pub async fn send_response_with_sdp(
        &self,
        session_id: &SessionId,
        code: u16,
        _reason: &str,
        sdp: &str,
    ) -> Result<()> {
        self.send_response(session_id, code, Some(sdp.to_string()))
            .await
    }

    /// Send response without SDP
    pub async fn send_response_session(
        &self,
        session_id: &SessionId,
        code: u16,
        _reason: &str,
    ) -> Result<()> {
        self.send_response(session_id, code, None).await
    }

    /// Send error response
    pub async fn send_error_response(
        &self,
        session_id: &SessionId,
        code: StatusCode,
        _reason: &str,
    ) -> Result<()> {
        self.send_response(session_id, code.as_u16(), None).await
    }

    /// SIP_API_DESIGN_2 Phase D — 3xx redirect dispatch with
    /// application-staged extras (e.g., a registrar's 305 Use Proxy
    /// with `Retry-After:`).
    pub async fn send_redirect_response_with_options(
        &self,
        session_id: &SessionId,
        status: u16,
        contacts: Vec<String>,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> Result<()> {
        tracing::info!(
            "DialogAdapter sending {} redirect for session {} with {} contact(s), {} staged extras",
            status,
            session_id.0,
            contacts.len(),
            extra_headers.len()
        );
        self.dialog_api
            .send_redirect_response_with_extras_for_session(
                &session_id.0,
                status,
                contacts,
                extra_headers,
            )
            .await
            .map_err(|e| {
                tracing::error!(
                    "Failed to send redirect (with extras) for session {}: {}",
                    session_id.0,
                    e
                );
                SessionError::DialogError(format!("Failed to send redirect: {}", e))
            })
    }

    /// Send a 3xx redirect response with one or more `Contact:` URIs
    /// (RFC 3261 §8.1.3.4). Thin wrapper over
    /// `UnifiedDialogApi::send_redirect_response_for_session`.
    pub async fn send_redirect_response(
        &self,
        session_id: &SessionId,
        status: u16,
        contacts: Vec<String>,
    ) -> Result<()> {
        tracing::info!(
            "DialogAdapter sending {} redirect for session {} with {} contact(s)",
            status,
            session_id.0,
            contacts.len()
        );
        self.dialog_api
            .send_redirect_response_for_session(&session_id.0, status, contacts)
            .await
            .map_err(|e| {
                tracing::error!(
                    "Failed to send redirect for session {}: {}",
                    session_id.0,
                    e
                );
                SessionError::DialogError(format!("Failed to send redirect: {}", e))
            })
    }

    /// SIP_API_DESIGN_2 Phase D — UAS response dispatch that
    /// threads application-staged headers to the wire. The session's
    /// pending UAS transaction is resolved internally by
    /// `UnifiedDialogApi::send_response_with_extras_for_session`,
    /// which appends `extra_headers` *after* the stack-managed
    /// `From` / `To` / `Via` / `Call-ID` / `CSeq` / `Content-Length`
    /// / `Contact` / `Record-Route` are stamped.
    pub async fn send_response_with_options(
        &self,
        session_id: &SessionId,
        code: u16,
        sdp: Option<String>,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> Result<()> {
        tracing::info!(
            "DialogAdapter sending {} response for session {} with {} staged extras",
            code,
            session_id.0,
            extra_headers.len()
        );
        self.dialog_api
            .send_response_with_extras_for_session(&session_id.0, code, sdp, extra_headers)
            .await
            .map_err(|e| {
                tracing::error!(
                    "Failed to send response (with extras) for session {}: {}",
                    session_id.0,
                    e
                );
                SessionError::DialogError(format!("Failed to send response: {}", e))
            })
    }

    /// Send response (for UAS)
    pub async fn send_response(
        &self,
        session_id: &SessionId,
        code: u16,
        sdp: Option<String>,
    ) -> Result<()> {
        tracing::info!(
            "DialogAdapter sending {} response for session {} with SDP: {}",
            code,
            session_id.0,
            sdp.is_some()
        );

        // Use dialog-core's session-based response method
        self.dialog_api
            .send_response_for_session(&session_id.0, code, sdp)
            .await
            .map_err(|e| {
                tracing::error!(
                    "Failed to send response for session {}: {}",
                    session_id.0,
                    e
                );
                SessionError::DialogError(format!("Failed to send response: {}", e))
            })
    }

    /// Send a UAS response through a known inbound server transaction.
    pub async fn send_response_for_transaction(
        &self,
        session_id: &SessionId,
        transaction_id: &TransactionKey,
        code: u16,
        sdp: Option<String>,
    ) -> Result<()> {
        let (transaction_method, transaction_direction) =
            exact_response_transaction_diagnostics(transaction_id);
        tracing::info!(
            session_id = %session_id.0,
            status_code = code,
            transaction_method,
            transaction_direction,
            sdp_present = sdp.is_some(),
            "DialogAdapter sending exact SIP response"
        );

        self.dialog_api
            .send_response_for_session_transaction(&session_id.0, transaction_id, code, sdp)
            .await
            .map_err(|_| {
                tracing::error!(
                    session_id = %session_id.0,
                    status_code = code,
                    transaction_method,
                    transaction_direction,
                    error_class = "dialog",
                    "Failed to send exact SIP response"
                );
                SessionError::DialogError(
                    "Failed to send exact SIP response (class=dialog)".to_string(),
                )
            })
    }

    /// Send an exact final response while preserving the transaction layer's
    /// authoritative transport-write disposition for cancellation recovery.
    pub(crate) async fn send_response_for_transaction_classified(
        &self,
        session_id: &SessionId,
        transaction_id: &TransactionKey,
        code: u16,
        sdp: Option<String>,
    ) -> std::result::Result<
        rvoip_sip_dialog::FinalResponseCompletionDisposition,
        rvoip_sip_dialog::ExactResponseSendError,
    > {
        self.dialog_api
            .send_response_for_session_transaction_classified(
                &session_id.0,
                transaction_id,
                code,
                sdp,
            )
            .await
    }

    /// Send a UAS response with application headers through a known inbound
    /// server transaction. Dialog-core verifies that the transaction belongs
    /// to the dialog resolved from `session_id` before writing anything.
    pub async fn send_response_with_options_for_transaction(
        &self,
        session_id: &SessionId,
        transaction_id: &TransactionKey,
        code: u16,
        body: Option<String>,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> Result<()> {
        self.dialog_api
            .send_response_with_extras_for_session_transaction(
                &session_id.0,
                transaction_id,
                code,
                body,
                extra_headers,
            )
            .await
            .map_err(|_| {
                SessionError::DialogError("Failed to send exact in-dialog response".to_string())
            })
    }

    /// Classified exact response variant that also preserves application
    /// response headers.
    pub(crate) async fn send_response_with_options_for_transaction_classified(
        &self,
        session_id: &SessionId,
        transaction_id: &TransactionKey,
        code: u16,
        body: Option<String>,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> std::result::Result<
        rvoip_sip_dialog::FinalResponseCompletionDisposition,
        rvoip_sip_dialog::ExactResponseSendError,
    > {
        self.dialog_api
            .send_response_with_extras_for_session_transaction_classified(
                &session_id.0,
                transaction_id,
                code,
                body,
                extra_headers,
            )
            .await
    }

    /// Send ACK (for UAC after 200 OK)
    pub async fn send_ack(&self, session_id: &SessionId, response: &Response) -> Result<()> {
        // Get the dialog ID for this session
        let dialog_id = self
            .session_to_dialog
            .get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();

        // Check if we have the original INVITE transaction ID stored
        if let Some(tx_id) = self.outgoing_invite_tx.get(session_id) {
            // Use the proper ACK method with transaction ID
            self.dialog_api
                .send_ack_for_2xx_response(&dialog_id, &tx_id, response)
                .await
                .map_err(|e| SessionError::DialogError(format!("Failed to send ACK: {}", e)))?;

            // Clean up the stored transaction ID after successful ACK
            self.outgoing_invite_tx.remove(session_id);
        } else {
            // Fallback: Try to send ACK without transaction ID (may not work properly)
            tracing::debug!(
                "No transaction ID stored for session {}, ACK may fail",
                session_id.0
            );
            // The dialog-core API doesn't have a direct send_ack without transaction ID
            // so we'll need to handle this case differently in production
        }

        Ok(())
    }

    /// Send BYE to terminate call (for state machine)
    pub async fn send_bye_session(&self, session_id: &SessionId) -> Result<()> {
        let dialog_id = {
            let entry = self
                .session_to_dialog
                .get(session_id)
                .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?;
            entry.value().clone()
        };

        self.mark_initial_invite_protocol_teardown(session_id);
        self.send_bye_with_retained_dialog_auth(
            session_id,
            &dialog_id,
            ByeRequestOptions::default(),
        )
        .await?;

        Ok(())
    }

    /// Send BYE with an RFC 3326 Reason header to terminate a call.
    pub async fn send_bye_session_with_reason(
        &self,
        session_id: &SessionId,
        reason: rvoip_sip_core::types::reason::Reason,
    ) -> Result<()> {
        let dialog_id = {
            let entry = self
                .session_to_dialog
                .get(session_id)
                .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?;
            entry.value().clone()
        };

        self.mark_initial_invite_protocol_teardown(session_id);
        self.send_bye_with_retained_dialog_auth(
            session_id,
            &dialog_id,
            ByeRequestOptions {
                reason: Some(reason.to_string()),
                ..Default::default()
            },
        )
        .await?;

        Ok(())
    }

    /// SIP_API_DESIGN_2 Phase C — UPDATE (RFC 3311) dispatch routed
    /// through the new dialog-core options surface. SDP is optional;
    /// when present it rides on the UPDATE body. The builder layer
    /// supplies a fully-populated `UpdateRequestOptions`.
    pub async fn send_update_with_options(
        &self,
        session_id: &SessionId,
        mut opts: rvoip_sip_dialog::api::unified::UpdateRequestOptions,
    ) -> Result<TransactionKey> {
        opts.extra_headers = apply_outbound_extras_policy(
            rvoip_sip_core::types::Method::Update,
            opts.extra_headers,
            self.outbound_proxy_uri.as_ref(),
        )?;
        let dialog_id = self
            .session_to_dialog
            .get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();
        self.dialog_api
            .send_update_with_options(&dialog_id, opts)
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to send UPDATE: {}", e)))
    }

    /// SIP_API_DESIGN_2 Phase C — re-INVITE dispatch routed through
    /// the new dialog-core options surface so applications can
    /// attach precomputed `Authorization:` or stage extra headers.
    pub async fn send_reinvite_with_options(
        &self,
        session_id: &SessionId,
        mut opts: rvoip_sip_dialog::api::unified::ReInviteRequestOptions,
    ) -> Result<()> {
        opts.extra_headers = apply_outbound_extras_policy(
            rvoip_sip_core::types::Method::Invite,
            opts.extra_headers,
            self.outbound_proxy_uri.as_ref(),
        )?;
        let dialog_id = self
            .session_to_dialog
            .get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();
        self.dialog_api
            .send_reinvite_with_options(&dialog_id, opts)
            .await
            .map_err(|error| {
                redacted_invite_dispatch_error(InviteDispatchFailure::ReinviteWithOptions, error)
            })?;
        Ok(())
    }

    /// SIP_API_DESIGN_2 Phase C — REFER dispatch through the new
    /// dialog-core options surface; carries the full RFC 3891
    /// `Replaces`, RFC 3892 `Referred-By`, RFC 4538 `Target-Dialog`
    /// trio.
    pub async fn send_refer_with_options(
        &self,
        session_id: &SessionId,
        mut opts: rvoip_sip_dialog::api::unified::ReferRequestOptions,
    ) -> Result<TransactionKey> {
        opts.extra_headers = apply_outbound_extras_policy(
            rvoip_sip_core::types::Method::Refer,
            opts.extra_headers,
            self.outbound_proxy_uri.as_ref(),
        )?;
        let dialog_id = self
            .session_to_dialog
            .get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();
        self.dialog_api
            .send_refer_with_options(&dialog_id, opts)
            .await
            .map_err(|error| redacted_dialog_operation_error("REFER", error))
    }

    /// SIP_API_DESIGN_2 Phase C — INFO dispatch through the new
    /// dialog-core options surface, replacing the legacy
    /// `send_info(content_type, body)` path.
    pub async fn send_info_with_options(
        &self,
        session_id: &SessionId,
        mut opts: rvoip_sip_dialog::api::unified::InfoRequestOptions,
    ) -> Result<TransactionKey> {
        opts.extra_headers = apply_outbound_extras_policy(
            rvoip_sip_core::types::Method::Info,
            opts.extra_headers,
            self.outbound_proxy_uri.as_ref(),
        )?;
        let dialog_id = self
            .session_to_dialog
            .get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();
        self.dialog_api
            .send_info_with_options(&dialog_id, opts)
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to send INFO: {}", e)))
    }

    /// SIP_API_DESIGN_2 Phase C — BYE dispatch through the new
    /// dialog-core options surface; carries the optional RFC 3326
    /// `Reason:` header.
    pub async fn send_bye_with_options(
        &self,
        session_id: &SessionId,
        mut opts: rvoip_sip_dialog::api::unified::ByeRequestOptions,
    ) -> Result<()> {
        opts.extra_headers = apply_outbound_extras_policy(
            rvoip_sip_core::types::Method::Bye,
            opts.extra_headers,
            self.outbound_proxy_uri.as_ref(),
        )?;
        let dialog_id = self
            .session_to_dialog
            .get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();
        self.mark_initial_invite_protocol_teardown(session_id);
        self.send_bye_with_retained_dialog_auth(session_id, &dialog_id, opts)
            .await?;
        Ok(())
    }

    /// Apply credentials negotiated by this exact dialog's initial INVITE to
    /// a locally generated BYE. Listener policies commonly authenticate every
    /// request, not only INVITE; sending an unauthenticated BYE and relying on
    /// the Active-state generic retry is unsafe because local hangup has
    /// already moved the session into its terminal transition. Serialize with
    /// application MESSAGE so Digest nonce-count updates cannot race.
    async fn send_bye_with_retained_dialog_auth(
        &self,
        session_id: &SessionId,
        dialog_id: &RvoipDialogId,
        mut opts: ByeRequestOptions,
    ) -> Result<()> {
        let dispatch_lane = self.data_message_dispatch_lanes.lane(dialog_id);
        let _dispatch_guard = Arc::clone(&dispatch_lane).lock_owned().await;
        let dialog = self
            .dialog_api
            .dialog_manager()
            .core()
            .get_dialog(dialog_id)
            .map_err(|_| {
                SessionError::InvalidTransition(
                    "SIP BYE exact dialog is no longer available".to_string(),
                )
            })?;
        let request_uri = dialog.remote_target.to_string();
        let next_hop = dialog
            .route_set
            .first()
            .unwrap_or(&dialog.remote_target)
            .to_string();
        let headers = self
            .retained_dialog_authorization_headers(
                session_id,
                dialog_id,
                "BYE",
                &request_uri,
                &next_hop,
                None,
            )
            .await?;
        opts.extra_headers.extend(headers);
        let generation = self.next_outgoing_bye_generation();
        let (transaction_id, completion) = self
            .dialog_api
            .send_bye_with_options_and_completion(dialog_id, opts)
            .await
            .map_err(|error| redacted_dialog_operation_error("SIP BYE", error))?;
        self.retain_outgoing_bye_transaction(
            session_id,
            generation,
            transaction_id,
            completion,
            request_uri,
        );
        Ok(())
    }

    fn next_outgoing_bye_generation(&self) -> u64 {
        self.next_outgoing_bye_generation
            .fetch_add(1, Ordering::Relaxed)
    }

    fn retain_outgoing_bye_transaction(
        &self,
        session_id: &SessionId,
        generation: u64,
        transaction_id: TransactionKey,
        completion: ClientTransactionCompletionHandle,
        request_uri: String,
    ) {
        let transaction = OutboundByeTransaction {
            generation,
            transaction_id,
            completion,
            request_uri,
        };
        self.outgoing_bye_tx
            .entry(session_id.clone())
            .and_modify(|current| {
                if current.generation < transaction.generation {
                    *current = transaction.clone();
                }
            })
            .or_insert(transaction);
        let sender = self
            .outgoing_bye_generation_watch
            .entry(session_id.clone())
            .or_insert_with(|| tokio::sync::watch::channel(0).0)
            .clone();
        sender.send_replace(generation);
    }

    fn latest_outgoing_bye_transaction(
        &self,
        session_id: &SessionId,
        after_generation: u64,
    ) -> Option<OutboundByeTransaction> {
        self.outgoing_bye_tx
            .get(session_id)
            .map(|entry| entry.value().clone())
            .filter(|transaction| transaction.generation > after_generation)
    }

    /// Capture the exact retained-BYE generation, if any, before a state
    /// machine dispatch. A caller can later prove that this dispatch reached
    /// the wire by observing a strictly newer generation for the same
    /// session; an unrelated session cannot satisfy that proof.
    pub(crate) fn outgoing_bye_generation(&self, session_id: &SessionId) -> Option<u64> {
        self.outgoing_bye_tx
            .get(session_id)
            .map(|transaction| transaction.generation)
    }

    /// Configured Timer F horizon for non-INVITE client transactions.
    pub(crate) fn non_invite_transaction_timeout(&self) -> Duration {
        self.non_invite_transaction_timeout
    }

    /// Return whether this exact session retained a BYE after `generation`.
    /// This is side-effect evidence, not an error-string classification: the
    /// coordinator uses it only to join the already-required final-response
    /// confirmation after post-send bookkeeping loses a concurrent race.
    pub(crate) fn has_outgoing_bye_after(&self, session_id: &SessionId, generation: u64) -> bool {
        self.outgoing_bye_tx
            .get(session_id)
            .is_some_and(|transaction| transaction.generation > generation)
    }

    /// Prove that an authentication event owns the latest retained BYE
    /// generation for this exact session. Stale or cross-session challenge
    /// events must not be allowed to consume the immutable BYE retry stash.
    pub(crate) fn outgoing_bye_transaction_matches(
        &self,
        session_id: &SessionId,
        transaction_id: &TransactionKey,
    ) -> bool {
        self.outgoing_bye_tx
            .get(session_id)
            .is_some_and(|transaction| transaction.transaction_id == *transaction_id)
    }

    /// Return the Request-URI from this session's latest exact outbound BYE.
    ///
    /// Digest HA2 must use the URI that was actually placed on the challenged
    /// request line. An established dialog's remote target can differ from the
    /// original To URI after Contact/target refresh processing, so rebuilding
    /// it from session metadata would produce invalid credentials.
    /// Non-INVITE transaction tombstones deliberately do not retain request
    /// wire. The adapter therefore captures the dialog remote target at the
    /// local-teardown dispatch fence and carries it with the exact generation.
    /// A 401/407 retry reuses that captured value rather than consulting
    /// mutable session metadata or challenge text.
    pub(crate) async fn outgoing_bye_request_uri(&self, session_id: &SessionId) -> Result<String> {
        self.outgoing_bye_tx
            .get(session_id)
            .map(|entry| entry.request_uri.clone())
            .ok_or_else(|| {
                tracing::warn!(
                    error_class = "missing-retained-transaction",
                    "SIP BYE authentication retry could not recover its exact request URI"
                );
                SessionError::InvalidTransition(
                    "SIP BYE authentication retry has no retained transaction".to_string(),
                )
            })
    }

    fn clear_outgoing_bye_transaction(
        &self,
        session_id: &SessionId,
        transaction: &OutboundByeTransaction,
    ) {
        let removed = self.outgoing_bye_tx.remove_if(session_id, |_, current| {
            current.generation == transaction.generation
                && current.transaction_id == transaction.transaction_id
        });
        if removed.is_some() {
            self.outgoing_bye_generation_watch.remove(session_id);
        }
    }

    /// Wait until the latest BYE attempt receives a successful final
    /// response. A 401/407 is not success: the generic request-auth flow may
    /// install one newer transaction, which this loop follows. Every other
    /// non-2xx, timeout, or unobservable transaction fails closed.
    pub(crate) async fn wait_for_outgoing_bye_final_response(
        &self,
        session_id: &SessionId,
    ) -> Result<()> {
        let deadline = tokio::time::Instant::now() + self.non_invite_transaction_timeout;
        let mut after_generation = 0;
        let mut last_transaction = None;
        let mut generation_changes = self
            .outgoing_bye_generation_watch
            .entry(session_id.clone())
            .or_insert_with(|| tokio::sync::watch::channel(0).0)
            .subscribe();
        loop {
            let transaction = loop {
                if let Some(transaction) =
                    self.latest_outgoing_bye_transaction(session_id, after_generation)
                {
                    break transaction;
                }
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    if let Some(transaction) = last_transaction.as_ref() {
                        self.clear_outgoing_bye_transaction(session_id, transaction);
                    }
                    return Err(SessionError::Timeout(
                        "SIP BYE transaction was not available before its deadline".to_string(),
                    ));
                }
                if !matches!(
                    tokio::time::timeout(remaining, generation_changes.changed()).await,
                    Ok(Ok(()))
                ) {
                    if let Some(transaction) = last_transaction.as_ref() {
                        self.clear_outgoing_bye_transaction(session_id, transaction);
                    }
                    return Err(SessionError::Timeout(
                        "SIP BYE transaction was not available before its deadline".to_string(),
                    ));
                }
            };
            last_transaction = Some(transaction.clone());
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                self.clear_outgoing_bye_transaction(session_id, &transaction);
                return Err(SessionError::Timeout(
                    "SIP BYE final response timed out".to_string(),
                ));
            }
            let response = tokio::select! {
                response = transaction.completion.wait_for_outcome(remaining) => response,
                generation_change = generation_changes.changed() => {
                    // Response processing records the exact completion before
                    // publishing any event that can run terminal cleanup. If
                    // both futures become ready together, the watch branch is
                    // allowed to win the select; re-read the completion cell so
                    // cleanup cannot turn a successful BYE into a false timeout.
                    let newer_generation_exists = self
                        .latest_outgoing_bye_transaction(session_id, transaction.generation)
                        .is_some();
                    let retained_transaction_exists =
                        self.outgoing_bye_tx.get(session_id).is_some();
                    match resolve_outgoing_bye_generation_wake(
                        transaction.completion.current_outcome(),
                        newer_generation_exists,
                        generation_change.is_err(),
                        retained_transaction_exists,
                    ) {
                        OutgoingByeGenerationWake::UseExactOutcome(current) => current,
                        OutgoingByeGenerationWake::FollowNewerGeneration => {
                            after_generation = transaction.generation;
                            continue;
                        }
                        OutgoingByeGenerationWake::RetryCurrentGeneration => continue,
                        OutgoingByeGenerationWake::CleanupInterrupted => {
                            return Err(SessionError::Timeout(
                                "SIP BYE confirmation ended during exact local cleanup".to_string(),
                            ));
                        }
                    }
                }
            };
            let newer_transaction = self
                .latest_outgoing_bye_transaction(session_id, transaction.generation)
                .is_some();
            match response {
                Ok(Some(ClientTransactionOutcome::FinalResponse(response)))
                    if (200..=299).contains(&response.status_code()) =>
                {
                    self.clear_outgoing_bye_transaction(session_id, &transaction);
                    return Ok(());
                }
                Ok(Some(ClientTransactionOutcome::FinalResponse(response)))
                    if matches!(response.status_code(), 401 | 407) =>
                {
                    after_generation = transaction.generation;
                }
                Ok(Some(ClientTransactionOutcome::FinalResponse(_))) => {
                    self.clear_outgoing_bye_transaction(session_id, &transaction);
                    return Err(SessionError::ProtocolError(
                        "SIP BYE received a non-success final response".to_string(),
                    ));
                }
                Ok(Some(ClientTransactionOutcome::Failure(_))) | Ok(None) => {
                    if newer_transaction {
                        after_generation = transaction.generation;
                        continue;
                    }
                    self.clear_outgoing_bye_transaction(session_id, &transaction);
                    return Err(SessionError::Timeout(
                        "SIP BYE final response timed out".to_string(),
                    ));
                }
                Err(_) => {
                    if newer_transaction {
                        after_generation = transaction.generation;
                        continue;
                    }
                    self.clear_outgoing_bye_transaction(session_id, &transaction);
                    return Err(SessionError::DialogError(
                        "SIP BYE final response could not be observed".to_string(),
                    ));
                }
            }
        }
    }

    async fn retained_dialog_authorization_headers(
        &self,
        session_id: &SessionId,
        dialog_id: &RvoipDialogId,
        method: &'static str,
        request_uri: &str,
        next_hop: &str,
        body: Option<&[u8]>,
    ) -> Result<Vec<rvoip_sip_core::types::TypedHeader>> {
        use crate::session_store::state::InviteCredentialKind;

        let snapshot = self
            .store
            .get_session_snapshot(session_id)
            .await
            .map_err(|_| {
                SessionError::InvalidTransition(
                    "SIP request exact session is no longer available".to_string(),
                )
            })?;
        let handle = snapshot.state().lifecycle_handle.clone().ok_or_else(|| {
            SessionError::InvalidTransition(
                "SIP request exact session has no lifecycle authority".to_string(),
            )
        })?;
        if snapshot
            .state()
            .dialog_id
            .as_ref()
            .is_none_or(|current| current.as_uuid() != &dialog_id.0)
        {
            return Err(SessionError::InvalidTransition(
                "SIP request exact dialog no longer owns its session".to_string(),
            ));
        }
        // The ordinary unauthenticated BYE path must remain read-only.  The
        // exact-cell update below clones and publishes a complete session
        // revision, so entering it when there is no retained credential would
        // add a full hot-path state write to every successful call teardown.
        if snapshot.state().invite_authorization_credentials.is_empty() {
            return Ok(Vec::new());
        }
        let transport = self.outbound_transport_context_for_uri(next_hop);
        update_retained_auth_exact(
            self.store.as_ref(),
            &handle,
            "SIP request exact session changed during authentication",
            |session| -> Result<Vec<_>> {
                if session
                    .dialog_id
                    .as_ref()
                    .is_none_or(|current| current.as_uuid() != &dialog_id.0)
                {
                    return Err(SessionError::InvalidTransition(
                        "SIP request exact dialog no longer owns its session".to_string(),
                    ));
                }
                if session.invite_authorization_credentials.is_empty() {
                    return Ok(Vec::new());
                }
                let auth = session
                    .auth
                    .clone()
                    .or_else(|| session.credentials.clone().map(Into::into))
                    .ok_or_else(|| {
                        SessionError::AuthError(
                            "SIP dialog retained a challenge without route credentials".to_string(),
                        )
                    })?;
                let origin_target = session
                    .remote_uri
                    .clone()
                    .unwrap_or_else(|| request_uri.to_string());
                let mut credentials = session.invite_authorization_credentials.clone();
                let mut digest_nc = session.digest_nc.clone();
                let mut headers = Vec::with_capacity(credentials.len());
                for credential in &mut credentials {
                    let applies_to_exact_target = match credential.kind {
                        InviteCredentialKind::Origin => {
                            credential.protection_target == origin_target
                        }
                        InviteCredentialKind::Proxy => credential.protection_target == next_hop,
                    };
                    if !applies_to_exact_target {
                        continue;
                    }
                    let preview = auth
                        .authorization_for_challenge_with_transport_context(
                            &credential.challenge_raw,
                            method,
                            request_uri,
                            1,
                            body,
                            &transport,
                        )
                        .map_err(|error| {
                            crate::errors::redacted_outbound_auth_error(
                                crate::errors::OutboundAuthOperation::Request,
                                error,
                            )
                        })?;
                    if let Some(challenge) = preview.digest_challenge.as_ref() {
                        if challenge.realm != credential.realm
                            || credential.nonce.as_deref() != Some(challenge.nonce.as_str())
                        {
                            return Err(SessionError::AuthError(
                                "SIP request challenge no longer matches the exact dialog protection space"
                                    .to_string(),
                            ));
                        }
                    }
                    let nonce_count = if let Some(challenge) = preview.digest_challenge.as_ref() {
                        let key = (challenge.realm.clone(), challenge.nonce.clone());
                        *digest_nc
                            .entry(key)
                            .and_modify(|count| *count = count.saturating_add(1))
                            .or_insert(1)
                    } else {
                        1
                    };
                    let selected = if preview.digest_challenge.is_some() && nonce_count != 1 {
                        auth.authorization_for_challenge_with_transport_context(
                            &credential.challenge_raw,
                            method,
                            request_uri,
                            nonce_count,
                            body,
                            &transport,
                        )
                        .map_err(|error| {
                            crate::errors::redacted_outbound_auth_error(
                                crate::errors::OutboundAuthOperation::Request,
                                error,
                            )
                        })?
                    } else {
                        preview
                    };
                    credential.value = selected.value.clone();
                    let name = match credential.kind {
                        InviteCredentialKind::Origin => {
                            rvoip_sip_core::types::HeaderName::Authorization
                        }
                        InviteCredentialKind::Proxy => {
                            rvoip_sip_core::types::HeaderName::ProxyAuthorization
                        }
                    };
                    headers.push(
                        rvoip_sip_core::validation::validated_authorization_header(
                            name,
                            selected.value,
                        )
                        .map_err(|_| {
                            SessionError::AuthError(
                                "SIP request authorization failed wire-safety validation"
                                    .to_string(),
                            )
                        })?,
                    );
                }
                session.invite_authorization_credentials = credentials;
                session.digest_nc = digest_nc;
                Ok(headers)
            },
        )
    }

    /// SIP_API_DESIGN_2 Phase C — NOTIFY dispatch through the new
    /// dialog-core options surface.
    pub async fn send_notify_with_options(
        &self,
        session_id: &SessionId,
        mut opts: rvoip_sip_dialog::api::unified::NotifyRequestOptions,
    ) -> Result<TransactionKey> {
        opts.extra_headers = apply_outbound_extras_policy(
            rvoip_sip_core::types::Method::Notify,
            opts.extra_headers,
            self.outbound_proxy_uri.as_ref(),
        )?;
        let dialog_id = self
            .session_to_dialog
            .get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();
        self.dialog_api
            .send_notify_with_options(&dialog_id, opts)
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to send NOTIFY: {}", e)))
    }

    /// SIP_API_DESIGN_2 Phase C — out-of-dialog MESSAGE dispatch
    /// through the new dialog-core options surface. Returns the
    /// registrar's `Response` so the caller can inspect 200 OK vs
    /// 401 auth-challenge vs 404. No session_id is required because
    /// MESSAGE is fire-and-forget per RFC 3428.
    pub async fn send_message_oob_with_options(
        &self,
        mut opts: rvoip_sip_dialog::api::unified::MessageRequestOptions,
    ) -> Result<rvoip_sip_core::Response> {
        opts.extra_headers = apply_outbound_extras_policy(
            rvoip_sip_core::types::Method::Message,
            opts.extra_headers,
            self.outbound_proxy_uri.as_ref(),
        )?;
        self.dialog_api
            .send_message_out_of_dialog_with_options(opts)
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to send MESSAGE: {}", e)))
    }

    /// SIP_API_DESIGN_2 Phase C — out-of-dialog OPTIONS dispatch.
    /// Today returns the wire-`Response` when dialog-core ships the
    /// transaction-authorship; until then dialog-core's stub returns
    /// `NotImplemented` and that error bubbles through unchanged.
    pub async fn send_options_oob_with_options(
        &self,
        mut opts: rvoip_sip_dialog::api::unified::OptionsRequestOptions,
    ) -> Result<rvoip_sip_core::Response> {
        opts.extra_headers = apply_outbound_extras_policy(
            rvoip_sip_core::types::Method::Options,
            opts.extra_headers,
            self.outbound_proxy_uri.as_ref(),
        )?;
        self.dialog_api
            .send_options_out_of_dialog_with_options(opts)
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to send OPTIONS: {}", e)))
    }

    /// SIP_API_DESIGN_2 Phase C — out-of-dialog SUBSCRIBE dispatch.
    /// Returns the registrar's `Response` so callers can inspect
    /// `Expires`, `Min-Expires`, or 401 challenge.
    pub async fn send_subscribe_oob_with_options(
        &self,
        target: &str,
        mut opts: rvoip_sip_dialog::api::unified::SubscribeRequestOptions,
    ) -> Result<rvoip_sip_core::Response> {
        opts.extra_headers = apply_outbound_extras_policy(
            rvoip_sip_core::types::Method::Subscribe,
            opts.extra_headers,
            self.outbound_proxy_uri.as_ref(),
        )?;
        self.dialog_api
            .send_subscribe_with_options(target, opts)
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to send SUBSCRIBE: {}", e)))
    }

    // ─────────────────────────────────────────────────────────────────────
    // SIP_API_DESIGN_2 R2 — auth-retry mirrors for non-INVITE/non-REGISTER
    // methods. Each `send_<method>_with_auth` takes the same options
    // struct as its non-auth sibling plus a pre-computed `Authorization:`
    // (or `Proxy-Authorization:`) header, validates the application
    // extras via `apply_outbound_extras_policy_with_auth`, then injects
    // the stack-computed auth header at the end before handing off to
    // dialog-core. Called by `Action::SendRequestWithAuth` after the
    // matching client auth scheme computes the response for the cached
    // challenge.
    // ─────────────────────────────────────────────────────────────────────

    pub async fn send_bye_with_auth(
        &self,
        session_id: &SessionId,
        mut opts: rvoip_sip_dialog::api::unified::ByeRequestOptions,
        auth_header_name: &str,
        auth_header_value: String,
    ) -> Result<()> {
        opts.extra_headers = apply_outbound_extras_policy_with_auth(
            rvoip_sip_core::types::Method::Bye,
            opts.extra_headers,
            self.outbound_proxy_uri.as_ref(),
            auth_header_name,
            auth_header_value,
        )?;
        let dialog_id = self
            .session_to_dialog
            .get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();
        // Authentication retries must sign and retain the challenged
        // generation's request target, not reconstruct it from To/session
        // metadata. Local teardown fences target-refresh processing while the
        // new BYE is built from the same confirmed dialog.
        let request_uri = self.outgoing_bye_request_uri(session_id).await?;
        self.mark_initial_invite_protocol_teardown(session_id);
        let generation = self.next_outgoing_bye_generation();
        let (transaction_id, completion) = self
            .dialog_api
            .send_bye_with_options_and_completion(&dialog_id, opts)
            .await
            .map_err(|e| {
                SessionError::DialogError(format!("Failed to send BYE with auth: {}", e))
            })?;
        self.retain_outgoing_bye_transaction(
            session_id,
            generation,
            transaction_id,
            completion,
            request_uri,
        );
        Ok(())
    }

    pub async fn send_refer_with_auth(
        &self,
        session_id: &SessionId,
        mut opts: rvoip_sip_dialog::api::unified::ReferRequestOptions,
        auth_header_name: &str,
        auth_header_value: String,
    ) -> Result<TransactionKey> {
        opts.extra_headers = apply_outbound_extras_policy_with_auth(
            rvoip_sip_core::types::Method::Refer,
            opts.extra_headers,
            self.outbound_proxy_uri.as_ref(),
            auth_header_name,
            auth_header_value,
        )?;
        let dialog_id = self
            .session_to_dialog
            .get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();
        self.dialog_api
            .send_refer_with_options(&dialog_id, opts)
            .await
            .map_err(|e| {
                SessionError::DialogError(format!("Failed to send REFER with auth: {}", e))
            })
    }

    pub async fn send_notify_with_auth(
        &self,
        session_id: &SessionId,
        mut opts: rvoip_sip_dialog::api::unified::NotifyRequestOptions,
        auth_header_name: &str,
        auth_header_value: String,
    ) -> Result<TransactionKey> {
        opts.extra_headers = apply_outbound_extras_policy_with_auth(
            rvoip_sip_core::types::Method::Notify,
            opts.extra_headers,
            self.outbound_proxy_uri.as_ref(),
            auth_header_name,
            auth_header_value,
        )?;
        let dialog_id = self
            .session_to_dialog
            .get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();
        self.dialog_api
            .send_notify_with_options(&dialog_id, opts)
            .await
            .map_err(|e| {
                SessionError::DialogError(format!("Failed to send NOTIFY with auth: {}", e))
            })
    }

    pub async fn send_info_with_auth(
        &self,
        session_id: &SessionId,
        mut opts: rvoip_sip_dialog::api::unified::InfoRequestOptions,
        auth_header_name: &str,
        auth_header_value: String,
    ) -> Result<TransactionKey> {
        opts.extra_headers = apply_outbound_extras_policy_with_auth(
            rvoip_sip_core::types::Method::Info,
            opts.extra_headers,
            self.outbound_proxy_uri.as_ref(),
            auth_header_name,
            auth_header_value,
        )?;
        let dialog_id = self
            .session_to_dialog
            .get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();
        self.dialog_api
            .send_info_with_options(&dialog_id, opts)
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to send INFO with auth: {}", e)))
    }

    pub async fn send_update_with_auth(
        &self,
        session_id: &SessionId,
        mut opts: rvoip_sip_dialog::api::unified::UpdateRequestOptions,
        auth_header_name: &str,
        auth_header_value: String,
    ) -> Result<TransactionKey> {
        opts.extra_headers = apply_outbound_extras_policy_with_auth(
            rvoip_sip_core::types::Method::Update,
            opts.extra_headers,
            self.outbound_proxy_uri.as_ref(),
            auth_header_name,
            auth_header_value,
        )?;
        let dialog_id = self
            .session_to_dialog
            .get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();
        self.dialog_api
            .send_update_with_options(&dialog_id, opts)
            .await
            .map_err(|e| {
                SessionError::DialogError(format!("Failed to send UPDATE with auth: {}", e))
            })
    }

    pub async fn send_message_oob_with_auth(
        &self,
        mut opts: rvoip_sip_dialog::api::unified::MessageRequestOptions,
        auth_header_name: &str,
        auth_header_value: String,
    ) -> Result<rvoip_sip_core::Response> {
        opts.extra_headers = apply_outbound_extras_policy_with_auth(
            rvoip_sip_core::types::Method::Message,
            opts.extra_headers,
            self.outbound_proxy_uri.as_ref(),
            auth_header_name,
            auth_header_value,
        )?;
        self.dialog_api
            .send_message_out_of_dialog_with_options(opts)
            .await
            .map_err(|e| {
                SessionError::DialogError(format!("Failed to send MESSAGE with auth: {}", e))
            })
    }

    pub async fn send_options_oob_with_auth(
        &self,
        mut opts: rvoip_sip_dialog::api::unified::OptionsRequestOptions,
        auth_header_name: &str,
        auth_header_value: String,
    ) -> Result<rvoip_sip_core::Response> {
        opts.extra_headers = apply_outbound_extras_policy_with_auth(
            rvoip_sip_core::types::Method::Options,
            opts.extra_headers,
            self.outbound_proxy_uri.as_ref(),
            auth_header_name,
            auth_header_value,
        )?;
        self.dialog_api
            .send_options_out_of_dialog_with_options(opts)
            .await
            .map_err(|e| {
                SessionError::DialogError(format!("Failed to send OPTIONS with auth: {}", e))
            })
    }

    pub async fn send_subscribe_oob_with_auth(
        &self,
        target: &str,
        mut opts: rvoip_sip_dialog::api::unified::SubscribeRequestOptions,
        auth_header_name: &str,
        auth_header_value: String,
    ) -> Result<rvoip_sip_core::Response> {
        opts.extra_headers = apply_outbound_extras_policy_with_auth(
            rvoip_sip_core::types::Method::Subscribe,
            opts.extra_headers,
            self.outbound_proxy_uri.as_ref(),
            auth_header_name,
            auth_header_value,
        )?;
        self.dialog_api
            .send_subscribe_with_options(target, opts)
            .await
            .map_err(|e| {
                SessionError::DialogError(format!("Failed to send SUBSCRIBE with auth: {}", e))
            })
    }

    /// Send CANCEL to cancel pending INVITE
    pub async fn send_cancel(&self, session_id: &SessionId) -> Result<()> {
        let dialog_id = {
            let entry = self
                .session_to_dialog
                .get(session_id)
                .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?;
            entry.value().clone()
        };

        self.mark_initial_invite_protocol_teardown(session_id);
        self.dialog_api
            .send_cancel_with_options(&dialog_id, CancelRequestOptions::default())
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to send CANCEL: {}", e)))?;

        Ok(())
    }

    /// SIP_API_DESIGN_2 Phase C — CANCEL dispatch through the new
    /// dialog-core options surface. Carries the optional RFC 3326
    /// `Reason:` and any application-staged extras to the wire.
    /// SIP_API_DESIGN_2 §7.1 — REGISTER dispatch through the new
    /// dialog-core options surface with policy validation and outbound-
    /// proxy route prepended. Used by `Action::SendREGISTERWithOptions`
    /// for the unified builder path; the legacy `send_register` path
    /// stays separate because its extras vector is empty and the route
    /// goes through `outbound_proxy_uri` directly on the options struct.
    pub async fn send_register_with_options(
        &self,
        opts: rvoip_sip_dialog::api::unified::RegisterRequestOptions,
    ) -> Result<rvoip_sip_core::Response> {
        self.send_register_with_options_and_route(opts)
            .await
            .map(|(response, _route)| response)
    }

    pub(crate) async fn send_register_with_options_and_route(
        &self,
        mut opts: rvoip_sip_dialog::api::unified::RegisterRequestOptions,
    ) -> Result<(
        rvoip_sip_core::Response,
        rvoip_sip_transport::TransportRoute,
    )> {
        opts.extra_headers = apply_outbound_extras_policy(
            rvoip_sip_core::types::Method::Register,
            opts.extra_headers,
            self.outbound_proxy_uri.as_ref(),
        )?;
        self.dialog_api
            .send_register_with_options_and_route(opts)
            .await
            .map_err(|error| redacted_dialog_operation_error("send REGISTER", error))
    }

    pub async fn send_cancel_with_options(
        &self,
        session_id: &SessionId,
        mut opts: rvoip_sip_dialog::api::unified::CancelRequestOptions,
    ) -> Result<()> {
        opts.extra_headers = apply_outbound_extras_policy(
            rvoip_sip_core::types::Method::Cancel,
            opts.extra_headers,
            self.outbound_proxy_uri.as_ref(),
        )?;
        let dialog_id = {
            let entry = self
                .session_to_dialog
                .get(session_id)
                .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?;
            entry.value().clone()
        };

        self.mark_initial_invite_protocol_teardown(session_id);
        self.dialog_api
            .send_cancel_with_options(&dialog_id, opts)
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to send CANCEL: {}", e)))?;

        Ok(())
    }

    /// Send an in-dialog INFO request (RFC 6086) with a caller-chosen
    /// `Content-Type`. Used for SIP-INFO DTMF (`application/dtmf-relay`),
    /// fax flow control (`application/sipfrag`), and other application-level
    /// mid-dialog signalling.
    pub async fn send_info(
        &self,
        session_id: &SessionId,
        content_type: &str,
        body: &[u8],
    ) -> Result<()> {
        let dialog_id = self
            .session_to_dialog
            .get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();

        self.dialog_api
            .send_info_with_options(
                &dialog_id,
                InfoRequestOptions {
                    content_type: content_type.to_string(),
                    body: bytes::Bytes::copy_from_slice(body),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to send INFO: {}", e)))?;

        tracing::debug!(
            session = %session_id.0,
            content_type = %content_type,
            body_len = body.len(),
            "Sent INFO"
        );
        Ok(())
    }

    /// Send REFER for blind transfer (for state machine)
    pub async fn send_refer_session(&self, session_id: &SessionId, refer_to: &str) -> Result<()> {
        let dialog_id = self
            .session_to_dialog
            .get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();

        // Send REFER through dialog API
        self.dialog_api
            .send_refer_with_options(
                &dialog_id,
                ReferRequestOptions {
                    refer_to: refer_to.to_string(),
                    ..Default::default()
                },
            )
            .await
            .map_err(|error| redacted_dialog_operation_error("REFER", error))?;

        tracing::info!(
            session = %session_id.0,
            target_present = !refer_to.is_empty(),
            target_bytes = refer_to.len(),
            "Sent REFER"
        );
        Ok(())
    }

    /// Fetch the SIP-level dialog identity (`Call-ID`, `local_tag`, `remote_tag`)
    /// for a session. Returns `None` if the session has no dialog yet
    /// (e.g., the INVITE hasn't been sent) or the dialog was lost.
    ///
    /// Callers use this to construct a Replaces header value when driving
    /// attended transfer from a higher layer.
    pub async fn dialog_identity(&self, session_id: &SessionId) -> Result<Option<DialogIdentity>> {
        let dialog_id = match self.session_to_dialog.get(session_id) {
            Some(entry) => entry.clone(),
            None => return Ok(None),
        };

        let dialog = match self.dialog_api.get_dialog_info(&dialog_id).await {
            Ok(d) => d,
            Err(_) => return Ok(None),
        };

        Ok(Some(DialogIdentity {
            call_id: dialog.call_id,
            local_tag: dialog.local_tag,
            remote_tag: dialog.remote_tag,
        }))
    }

    /// Send a re-INVITE for hold/resume or mid-call SDP updates.
    ///
    /// RFC 3261 §14 — re-INVITE is the standard mechanism for modifying an
    /// established dialog's session parameters (SDP direction attributes for
    /// hold/resume, codec changes, etc.). This previously routed through
    /// UPDATE (RFC 3311) which caused Timer F timeouts when the remote
    /// didn't answer an UPDATE promptly; re-INVITE is both more widely
    /// supported and the RFC-recommended method here.
    pub async fn send_reinvite_session(&self, session_id: &SessionId, sdp: String) -> Result<()> {
        use rvoip_sip_core::Method;

        let dialog_id = self
            .session_to_dialog
            .get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();

        self.dialog_api
            .send_request_in_dialog(&dialog_id, Method::Invite, Some(bytes::Bytes::from(sdp)))
            .await
            .map_err(|error| {
                redacted_invite_dispatch_error(InviteDispatchFailure::ReinviteInDialog, error)
            })?;

        Ok(())
    }

    /// Clean up all mappings and resources for a session
    pub async fn cleanup_session(&self, session_id: &SessionId) -> Result<()> {
        let handle = self.store.lifecycle_handle(session_id).ok_or_else(|| {
            SessionError::SessionNotFound(format!(
                "Session {} has no current lifecycle handle",
                session_id.0
            ))
        })?;
        self.cleanup_session_exact(&handle).await
    }

    /// Clean up only the dialog resources owned by one retained session
    /// lifetime. The exact store check happens before any lower mutation, so a
    /// delayed cleanup cannot target a later call that reused the raw ID.
    pub(crate) async fn cleanup_session_exact(&self, handle: &SessionRegistryHandle) -> Result<()> {
        self.store
            .get_session_retained_exact(handle)
            .await
            .map_err(|_| {
                SessionError::SessionNotFound(format!(
                    "Session {} exact lifetime is unavailable",
                    handle.session_id().0
                ))
            })?;
        let session_id = handle.session_id();
        let guard = cleanup_diag::stage_guard(CleanupStage::DialogCleanup, &session_id.0);
        self.cleanup_attempt_total.fetch_add(1, Ordering::Relaxed);
        // Capture exact identifiers without removing them. The lower core
        // cleanup suspends; retaining the mappings until it returns makes a
        // timeout-cancelled caller retryable with the same dialog identity.
        let dialog_id = self
            .session_to_dialog
            .get(session_id)
            .map(|entry| entry.value().clone());
        let call_ids_to_remove: Vec<_> = self
            .callid_to_session
            .iter()
            .filter(|entry| entry.value() == session_id)
            .map(|entry| entry.key().clone())
            .collect();
        let outgoing_invite = self
            .outgoing_invite_tx
            .get(session_id)
            .map(|entry| entry.value().clone());
        let outgoing_bye = self
            .outgoing_bye_tx
            .get(session_id)
            .map(|entry| entry.value().clone());
        let outbound_initial_invite = self
            .outbound_initial_invites
            .get(session_id)
            .map(|entry| entry.value().clone())
            .filter(|binding| {
                binding.handle == *handle
                    && dialog_id
                        .as_ref()
                        .is_some_and(|dialog_id| binding.owner.dialog_id() == dialog_id)
            });

        // Exact terminal cleanup supersedes further BYE confirmation/auth
        // work. Drop the retained completion owner before any lower cleanup
        // suspension so a bounded adapter fallback wakes the retained hangup
        // supervisor instead of leaving it asleep until Timer F.
        if let Some(transaction) = outgoing_bye.as_ref() {
            self.clear_outgoing_bye_transaction(session_id, transaction);
        }

        // Serialize cleanup with exact-dialog DataMessage dispatch. Holding
        // this owner through lower cleanup and mapping removal makes queued
        // senders observe an unavailable dialog rather than send after close.
        let data_message_lane = dialog_id
            .as_ref()
            .map(|dialog_id| self.data_message_dispatch_lanes.lane(dialog_id));
        let _data_message_cleanup_guard = match data_message_lane.as_ref() {
            Some(lane) => Some(Arc::clone(lane).lock_owned().await),
            None => None,
        };

        if let Some(dialog_id) = dialog_id.as_ref() {
            self.cleanup_mapped_total.fetch_add(1, Ordering::Relaxed);
            #[cfg(test)]
            let pause = {
                self.cleanup_pause
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .take()
            };
            #[cfg(test)]
            if let Some(pause) = pause {
                pause.entered.store(true, Ordering::Release);
                pause.entered_notify.notify_waiters();
                pause.release.notified().await;
            }
            self.dialog_api
                .dialog_manager()
                .core()
                .cleanup_dialog_storage_and_transactions(dialog_id)
                .await;

            // A final response (including a 3xx followed by a fresh INVITE)
            // terminates this exact initial-INVITE owner. Retire its lower
            // ownership and exact registry mapping before allowing a
            // replacement dialog to install for the same session lifetime.
            // The managed resource itself remains registered with the
            // lifecycle authority until whole-session teardown; its exact
            // release observes that this binding has been superseded and is
            // therefore harmless.
            if let Some(binding) = outbound_initial_invite.as_ref() {
                if self
                    .dialog_api
                    .initial_invite_owner_is_retained(&binding.owner)
                    && !self
                        .dialog_api
                        .finish_initial_invite_teardown(&binding.owner)
                        .await
                {
                    return Err(SessionError::InternalError(
                        "initial INVITE exact retirement failed (class=lifecycle)".to_string(),
                    ));
                }
                self.store
                    .registry()
                    .clear_dialog_handle_retained(handle, dialog_id.clone().into())
                    .map_err(|_| {
                        SessionError::InternalError(
                            "initial INVITE registry retirement failed (class=lifecycle)"
                                .to_string(),
                        )
                    })?;
                self.outbound_initial_invites
                    .remove_if(session_id, |_, current| {
                        current.matches(handle, &binding.owner)
                    });
            }
            self.session_to_dialog
                .remove_if(session_id, |_, mapped| mapped == dialog_id);
            self.dialog_to_session
                .remove_if(dialog_id, |_, mapped| mapped == session_id);
            if let Some(lane) = data_message_lane.as_ref() {
                self.data_message_dispatch_lanes
                    .remove_exact(dialog_id, lane);
            }
        } else {
            self.cleanup_missing_total.fetch_add(1, Ordering::Relaxed);
        }

        for call_id in call_ids_to_remove {
            if self
                .callid_to_session
                .remove_if(&call_id, |_, mapped| mapped == session_id)
                .is_some()
            {
                self.cleanup_call_ids_removed_total
                    .fetch_add(1, Ordering::Relaxed);
            }
        }

        if outgoing_invite.is_some_and(|transaction| {
            self.outgoing_invite_tx
                .remove_if(session_id, |_, mapped| mapped == &transaction)
                .is_some()
        }) {
            self.cleanup_outgoing_invite_removed_total
                .fetch_add(1, Ordering::Relaxed);
        }
        self.outbound_request_tracker.clear_session(session_id);

        tracing::debug!(
            "Cleaned up dialog adapter mappings for session {}",
            session_id.0
        );
        guard.finish_success();
        Ok(())
    }

    // ===== Registration Methods =====

    fn start_symmetric_registration_keepalive(
        &self,
        from_uri: &str,
        route: Option<rvoip_sip_transport::TransportRoute>,
    ) {
        let Some(params) = self.symmetric_flow_params.as_ref() else {
            return;
        };
        let Some(route) = route else {
            tracing::warn!(
                "symmetric registered-flow: successful REGISTER did not retain an exact route"
            );
            return;
        };
        let destination = route.destination;

        if self
            .dialog_api
            .dialog_manager()
            .core()
            .start_outbound_ping_on_route(
                (
                    from_uri.to_string(),
                    params.reg_id,
                    params.instance_urn.clone(),
                ),
                route,
            )
        {
            tracing::info!(
                aor_present = !from_uri.is_empty(),
                aor_bytes = from_uri.len(),
                reg_id = params.reg_id,
                instance_present = !params.instance_urn.is_empty(),
                instance_bytes = params.instance_urn.len(),
                destination = %destination,
                "symmetric registered-flow: keep-alive ping started"
            );
        }
    }

    /// Send REGISTER request and process response.
    ///
    /// `extras` carry application-staged additional headers (e.g. from
    /// `coord.register(..).with_header(..)`); they ride alongside the
    /// stack-managed headers per SIP_API_DESIGN_2.md §7.3. On auth retry
    /// the caller passes the same extras snapshot so headers survive the
    /// 401/407 hop.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn send_register(
        &self,
        session_id: &SessionId,
        registrar_uri: &str,
        from_uri: &str,
        contact_uri: &str,
        expires: u32,
        auth: Option<&crate::auth::SipClientAuth>,
        extras: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> Result<RegisterAttemptOutcome> {
        tracing::info!(
            session_present = !session_id.0.is_empty(),
            session_bytes = session_id.0.len(),
            registrar_present = !registrar_uri.is_empty(),
            registrar_bytes = registrar_uri.len(),
            aor_present = !from_uri.is_empty(),
            aor_bytes = from_uri.len(),
            contact_present = !contact_uri.is_empty(),
            contact_bytes = contact_uri.len(),
            expires,
            "outbound REGISTER starting"
        );

        // Build authorization header if auth material provided.
        let (authorization, proxy_authorization) = if let Some(auth) = auth {
            let auth_state = self
                .store
                .update_session_with(session_id, |session| {
                    let challenge_raw = session.auth_challenge_raw.clone().or_else(|| {
                        session.auth_challenge.as_ref().map(|challenge| {
                            rvoip_auth_core::DigestAuthenticator::new(challenge.realm.clone())
                                .format_www_authenticate(challenge)
                        })
                    })?;

                    let digest_challenge = session.auth_challenge.clone().or_else(|| {
                        rvoip_auth_core::DigestAuthenticator::parse_challenge(&challenge_raw).ok()
                    });
                    let nc_value = if let Some(challenge) = digest_challenge.as_ref() {
                        let nc_key = (challenge.realm.clone(), challenge.nonce.clone());
                        *session
                            .digest_nc
                            .entry(nc_key)
                            .and_modify(|n| *n += 1)
                            .or_insert(1)
                    } else {
                        1
                    };
                    let transport_context = session
                        .pending_auth_transport
                        .take()
                        .unwrap_or_else(|| self.outbound_transport_context_for_uri(registrar_uri));
                    let status = session
                        .pending_auth
                        .as_ref()
                        .map(|(status, _)| *status)
                        .unwrap_or(401);
                    Some((challenge_raw, nc_value, transport_context, status))
                })
                .await?;
            if let Some((challenge_raw, nc_value, transport_context, status)) = auth_state {
                // RFC 7616 §3.4.5 — bump the per-(realm, nonce) NC
                // counter before computing. REGISTER reuses one nonce
                // across many refreshes, so this is exactly the path
                // where carriers reject `nc=00000001` repeats.
                tracing::info!(
                    registrar_present = !registrar_uri.is_empty(),
                    registrar_bytes = registrar_uri.len(),
                    nonce_count = nc_value,
                    "outbound REGISTER authentication computing"
                );

                // REGISTER body is empty; pass `None` so the qop
                // selector picks `auth` (or legacy if no qop offered)
                // rather than `auth-int`.
                let selected = auth
                    .authorization_for_challenge_with_transport_context(
                        &challenge_raw,
                        "REGISTER",
                        registrar_uri,
                        nc_value,
                        None,
                        &transport_context,
                    )
                    .map_err(|error| {
                        crate::errors::redacted_outbound_auth_error(
                            crate::errors::OutboundAuthOperation::Register,
                            error,
                        )
                    })?;

                tracing::info!(
                    auth_scheme = register_auth_scheme_class(&selected.scheme),
                    "outbound REGISTER authentication computed"
                );

                if status == 407 {
                    (None, Some(selected.value))
                } else {
                    (Some(selected.value), None)
                }
            } else {
                tracing::debug!("No challenge stored, sending without auth");
                (None, None)
            }
        } else {
            (None, None)
        };

        // RFC 3581 NAT discovery: if the dialog manager has learned a
        // public address from a prior response's `Via:
        // …;received=…;rport=…`, rewrite the host:port portion of the
        // Contact URI so the registrar binds the new registration to
        // the externally-routable address (RFC 5626 §5). First
        // REGISTER goes out with the bind-address Contact; the
        // response carries `received=`/`rport=` which populates the
        // discovery cache; subsequent REGISTERs (refresh, auth retry)
        // use the discovered address.
        let rewritten_contact = if let Some(public) = self.dialog_api.discovered_public_addr().await
        {
            let rewritten = rewrite_contact_host(contact_uri, public);
            if rewritten != contact_uri {
                tracing::info!(
                    contact_present = !contact_uri.is_empty(),
                    contact_bytes = contact_uri.len(),
                    rewritten_contact_present = !rewritten.is_empty(),
                    rewritten_contact_bytes = rewritten.len(),
                    public_address_family = if public.is_ipv4() { "ipv4" } else { "ipv6" },
                    public_port = public.port(),
                    "outbound REGISTER Contact rewritten from NAT discovery"
                );
            }
            rewritten
        } else {
            contact_uri.to_string()
        };

        // Reserve registration identity for this new logical REGISTER
        // transaction. This is registration-scoped only; dialog-core still owns
        // all in-dialog CSeq state and transaction-layer retransmissions reuse
        // the request created below.
        let (registration_call_id, registration_cseq) = self
            .store
            .update_session_with(session_id, |session| {
                let call_id = session
                    .registration_call_id
                    .get_or_insert_with(|| format!("reg-{}", uuid::Uuid::new_v4()))
                    .clone();
                session.registration_cseq = session.registration_cseq.saturating_add(1);
                (call_id, session.registration_cseq)
            })
            .await?;

        // Send REGISTER through dialog-core API and get response.
        // A5 Phase 2a: when the coordinator is configured for RFC 5626 SIP
        // Outbound, route through the outbound-aware REGISTER so the Contact
        // carries `+sip.instance` + `reg-id` + `;ob`.
        let (response, register_route) = self
            .dialog_api
            .send_register_with_options_and_route(
                rvoip_sip_dialog::api::unified::RegisterRequestOptions {
                    registrar_uri: registrar_uri.to_string(),
                    aor_uri: from_uri.to_string(),
                    contact_uri: rewritten_contact,
                    expires,
                    authorization,
                    proxy_authorization,
                    call_id: Some(registration_call_id),
                    cseq: Some(registration_cseq),
                    outbound_contact: self.outbound_contact_params.clone(),
                    outbound_proxy_uri: self.outbound_proxy_uri.clone(),
                    extra_headers: extras,
                    refresh: false,
                },
            )
            .await
            .map_err(|error| redacted_dialog_operation_error("send REGISTER", error))?;

        tracing::info!(
            "REGISTER response received: {} for session {}",
            response.status_code(),
            session_id.0
        );

        let register_transport = self
            .dialog_api
            .outbound_transport_context_for_response(&response)
            .map(|context| {
                crate::auth::SipTransportSecurityContext::from_transport_context(&context)
            });
        if self.store.with_session(session_id, |_| ()).is_ok() {
            self.store
                .update_session_with(session_id, |session| {
                    session.pending_auth_transport = register_transport;
                })
                .await?;
        }

        match response.status_code() {
            200..=299 => {
                let is_unregister = expires == 0;
                if is_unregister {
                    Ok(RegisterAttemptOutcome::Unregistered)
                } else {
                    let accepted_expires =
                        Self::accepted_registration_expires(&response, contact_uri, expires);
                    let mut metadata = Self::response_registration_metadata(&response);
                    metadata.transport_route = Some(register_route);
                    Ok(RegisterAttemptOutcome::Registered {
                        accepted_expires,
                        metadata,
                    })
                }
            }
            401 | 407 => {
                // RFC 3261 §22.2 — auth challenge on REGISTER. The adapter
                // only extracts the challenge; the state-machine action owns
                // retry limits and the follow-up `AuthRequired` event.
                use rvoip_sip_core::types::headers::HeaderAccess;
                let header_name = if response.status_code() == 407 {
                    rvoip_sip_core::types::header::HeaderName::ProxyAuthenticate
                } else {
                    rvoip_sip_core::types::header::HeaderName::WwwAuthenticate
                };
                let challenges = response
                    .raw_headers(&header_name)
                    .into_iter()
                    .filter_map(|bytes| String::from_utf8(bytes).ok())
                    .collect::<Vec<_>>();
                if !challenges.is_empty() {
                    Ok(RegisterAttemptOutcome::AuthChallenge {
                        status_code: response.status_code(),
                        challenge: challenges.join(", "),
                    })
                } else {
                    tracing::warn!(
                        "REGISTER {} without challenge header",
                        response.status_code()
                    );
                    Ok(RegisterAttemptOutcome::Failure {
                        status_code: response.status_code(),
                        reason: "REGISTER challenge response did not include challenge header"
                            .to_string(),
                    })
                }
            }
            423 => {
                // RFC 3261 §10.2.8 — Interval Too Brief. The registrar requires
                // a minimum expiry; it MUST include a Min-Expires header with
                // its minimum acceptable value. The action owns the bounded
                // retry and re-enters this method iteratively.
                use rvoip_sip_core::types::headers::HeaderAccess;
                let min_expires = response
                    .raw_header_value(&rvoip_sip_core::types::header::HeaderName::MinExpires)
                    .and_then(|s| s.trim().parse::<u32>().ok());

                match min_expires {
                    Some(min_expires) if min_expires > 0 && min_expires <= 7200 => {
                        Ok(RegisterAttemptOutcome::IntervalTooBrief { min_expires })
                    }
                    Some(min_expires) => Ok(RegisterAttemptOutcome::Failure {
                        status_code: response.status_code(),
                        reason: format!(
                            "423 Interval Too Brief included invalid Min-Expires={}",
                            min_expires
                        ),
                    }),
                    None => Ok(RegisterAttemptOutcome::Failure {
                        status_code: response.status_code(),
                        reason: "423 Interval Too Brief without Min-Expires header".to_string(),
                    }),
                }
            }
            _ => {
                tracing::warn!(
                    "❌ Registration failed with status {}",
                    response.status_code()
                );
                Ok(RegisterAttemptOutcome::Failure {
                    status_code: response.status_code(),
                    reason: response.reason_phrase().to_string(),
                })
            }
        }
    }

    pub async fn send_subscribe(
        &self,
        session_id: &SessionId,
        from_uri: &str,
        to_uri: &str,
        event_package: &str,
        expires: u32,
    ) -> Result<()> {
        tracing::info!(
            "Sending SUBSCRIBE for session {} from {} to {} for event {}",
            session_id.0,
            from_uri,
            to_uri,
            event_package
        );

        // Send as non-dialog request (creates dialog on 2xx). dialog-core
        // owns the wire-ready SIP request construction.
        let response = self
            .dialog_api
            .send_subscribe_with_options(
                to_uri,
                SubscribeRequestOptions {
                    event: event_package.to_string(),
                    expires,
                    from_uri: Some(from_uri.to_string()),
                    contact_uri: Some(from_uri.to_string()),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to send SUBSCRIBE: {}", e)))?;

        tracing::info!(
            "SUBSCRIBE response: {} for session {}",
            response.status_code(),
            session_id.0
        );

        // Handle response and potentially store dialog ID
        if response.status_code() == 200 || response.status_code() == 202 {
            // Extract dialog ID from response if present
            // This would normally come from the response headers
            // For now, emit subscription accepted event
            let event = RvoipCrossCrateEvent::DialogToSession(
                rvoip_infra_common::events::cross_crate::DialogToSessionEvent::SubscriptionAccepted {
                    session_id: session_id.0.clone(),
                }
            );
            let _ = self.global_coordinator.publish(Arc::new(event)).await;
        } else if response.status_code() >= 400 {
            let event = RvoipCrossCrateEvent::DialogToSession(
                rvoip_infra_common::events::cross_crate::DialogToSessionEvent::SubscriptionFailed {
                    session_id: session_id.0.clone(),
                    status_code: response.status_code(),
                },
            );
            let _ = self.global_coordinator.publish(Arc::new(event)).await;
        }

        Ok(())
    }

    /// Send a NOTIFY request within a subscription dialog
    pub async fn send_notify(
        &self,
        session_id: &SessionId,
        event_package: &str,
        body: Option<String>,
        subscription_state: Option<String>,
    ) -> Result<()> {
        tracing::info!(
            "Sending NOTIFY for session {} with event {} and state {:?}",
            session_id.0,
            event_package,
            subscription_state
        );

        // Get dialog ID for this session
        let dialog_id = self
            .session_to_dialog
            .get(session_id)
            .ok_or_else(|| SessionError::DialogError("No dialog for session".to_string()))?
            .clone();

        // Send NOTIFY within the dialog
        self.dialog_api
            .send_notify_with_options(
                &dialog_id,
                NotifyRequestOptions {
                    event: event_package.to_string(),
                    subscription_state: subscription_state.unwrap_or_default(),
                    body: body.map(bytes::Bytes::from),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to send NOTIFY: {}", e)))?;

        tracing::info!("NOTIFY sent successfully for session {}", session_id.0);
        Ok(())
    }

    /// Send NOTIFY for REFER implicit subscription (RFC 3515)
    ///
    /// Convenience method that automatically formats NOTIFY for transfer progress
    pub async fn send_refer_notify(
        &self,
        session_id: &SessionId,
        status_code: u16,
        reason: &str,
    ) -> Result<()> {
        tracing::info!(
            session = %session_id.0,
            status_code,
            reason_present = !reason.is_empty(),
            reason_bytes = reason.len(),
            "Sending REFER NOTIFY"
        );

        // Build REFER progress through the same exact, tracked NOTIFY path as
        // application-generated requests. Keeping the immutable options in
        // the tracker preserves the sipfrag body for Digest auth-int retry,
        // correlates the final response to this transaction, and guarantees
        // automatic operator headers are applied without exposing them in
        // diagnostics.
        let subscription_state = if status_code >= 200 {
            "terminated;reason=noresource"
        } else {
            "active;expires=60"
        };
        let options = Arc::new(NotifyRequestOptions {
            event: "refer".to_string(),
            subscription_state: subscription_state.to_string(),
            content_type: Some("message/sipfrag".to_string()),
            body: Some(bytes::Bytes::from(format!(
                "SIP/2.0 {} {}",
                status_code, reason
            ))),
            subscription_id: None,
            extra_headers: self.auto_emit_extra_headers.clone(),
        });
        let lease = self.outbound_request_tracker.prepare(
            session_id,
            TrackedInDialogOptions::Notify(Arc::clone(&options)),
        )?;
        let transaction_id = self
            .send_notify_with_options(session_id, (*options).clone())
            .await
            .map_err(|error| redacted_dialog_operation_error("REFER NOTIFY", error))?;
        self.outbound_request_tracker
            .activate(lease, transaction_id)?;

        tracing::info!(
            "REFER NOTIFY sent successfully for session {}",
            session_id.0
        );
        Ok(())
    }

    // ===== MESSAGE Methods =====

    /// Send a validated, byte-preserving MESSAGE on one exact dialog.
    ///
    /// `send_request_in_dialog` currently converts generic bodies through a
    /// UTF-8 `String`. Build the request with an equal-length placeholder and
    /// replace only its public `Bytes` body before dispatch, preserving the
    /// builder's exact Content-Length while retaining arbitrary binary bytes.
    pub(crate) async fn send_data_message_on_dialog(
        &self,
        dialog_id: &RvoipDialogId,
        message: SipDataMessage,
    ) -> Result<()> {
        let dispatch_lane = self.data_message_dispatch_lanes.lane(dialog_id);
        let _dispatch_guard = Arc::clone(&dispatch_lane).lock_owned().await;
        let manager = self.dialog_api.dialog_manager().core();
        let mut fresh_challenge = None;

        // One initial attempt plus one bounded retry for a fresh 401/407.
        // Creating a new dialog template on each pass advances CSeq as RFC
        // 3261 requires for the challenged replacement request.
        for attempt in 0..=1 {
            let template = {
                let mut dialog = match manager.get_dialog_mut(dialog_id) {
                    Ok(dialog) => dialog,
                    Err(_) => {
                        // Cleanup may have removed the original lane before this
                        // already-captured sender entered. Remove only this exact
                        // replacement lane; a concurrently installed successor is
                        // left for its own failed dispatch to release.
                        self.data_message_dispatch_lanes
                            .remove_exact(dialog_id, &dispatch_lane);
                        return Err(SessionError::InvalidTransition(
                            "SIP MESSAGE exact dialog is no longer available".to_string(),
                        ));
                    }
                };
                if dialog.state != DialogState::Confirmed {
                    return Err(SessionError::InvalidTransition(
                        "SIP MESSAGE requires a confirmed dialog".to_string(),
                    ));
                }
                let template = dialog.create_request_template(rvoip_sip_core::Method::Message);
                let local_tag = template
                    .local_tag
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        SessionError::InvalidTransition(
                            "SIP MESSAGE confirmed dialog is missing its local tag".to_string(),
                        )
                    })?;
                let remote_tag = template
                    .remote_tag
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        SessionError::InvalidTransition(
                            "SIP MESSAGE confirmed dialog is missing its remote tag".to_string(),
                        )
                    })?;
                let local_address = manager
                    .local_address_for_target_and_routes(&template.target_uri, &template.route_set);
                DialogRequestTemplate {
                    call_id: template.call_id,
                    from_uri: template.local_uri.to_string(),
                    from_tag: local_tag,
                    to_uri: template.remote_uri.to_string(),
                    to_tag: remote_tag,
                    request_uri: template.target_uri.to_string(),
                    cseq: template.cseq_number,
                    local_address,
                    route_set: template.route_set,
                    contact: None,
                }
            };

            let mut request = build_sip_data_request(&template, message.clone()).map_err(|_| {
                SessionError::InvalidInput(
                    "SIP data message failed local request construction".to_string(),
                )
            })?;

            let next_hop = exact_next_hop_uri_for_request(&request).map_err(|_| {
                SessionError::InvalidInput(
                    "SIP data message has an unusable exact next hop".to_string(),
                )
            })?;
            let candidates = manager.resolve_uri_to_candidates(&next_hop).await;
            if candidates.is_empty() {
                return Err(SessionError::DialogError(
                    "SIP MESSAGE exact next hop is unavailable".to_string(),
                ));
            }
            self.authorize_data_message_on_dialog(
                dialog_id,
                &mut request,
                &next_hop.to_string(),
                fresh_challenge.as_ref(),
            )
            .await?;
            let (transaction_id, _) = manager
                // This operation owns its final response and bounded auth
                // retry. Deliberately do not publish the transaction through
                // the generic session AuthRequired path, which otherwise
                // would race a second MESSAGE retry from the state machine.
                .send_request_with_candidate_failover(request, candidates, None)
                .await
                .map_err(|error| redacted_dialog_operation_error("SIP MESSAGE", error))?;
            let response = manager
                .transaction_manager()
                .wait_for_final_response(&transaction_id, DATA_MESSAGE_FINAL_RESPONSE_TIMEOUT)
                .await
                .map_err(|_| {
                    SessionError::DialogError(
                        "SIP MESSAGE final response could not be observed".to_string(),
                    )
                })?
                .ok_or_else(|| {
                    SessionError::DialogError("SIP MESSAGE final response timed out".to_string())
                })?;
            match response.status_code() {
                200..=299 => return Ok(()),
                status @ (401 | 407) if attempt == 0 => {
                    use rvoip_sip_core::types::headers::HeaderAccess;

                    let header_name = if status == 407 {
                        rvoip_sip_core::types::header::HeaderName::ProxyAuthenticate
                    } else {
                        rvoip_sip_core::types::header::HeaderName::WwwAuthenticate
                    };
                    let values = response
                        .raw_headers(&header_name)
                        .into_iter()
                        .map(|value| String::from_utf8(value).map_err(|_| ()))
                        .collect::<std::result::Result<Vec<_>, _>>()
                        .map_err(|_| {
                            SessionError::AuthError(
                                "SIP MESSAGE challenge is not valid header text".to_string(),
                            )
                        })?;
                    if values.is_empty() {
                        return Err(SessionError::AuthError(
                            "SIP MESSAGE challenge response omitted its challenge".to_string(),
                        ));
                    }
                    fresh_challenge = Some(DataMessageAuthChallenge {
                        status,
                        value: values.join(", "),
                    });
                }
                401 | 407 => {
                    return Err(SessionError::AuthError(
                        "SIP MESSAGE authentication retry was rejected".to_string(),
                    ));
                }
                status => {
                    return Err(SessionError::ProtocolError(format!(
                        "SIP MESSAGE peer rejected delivery with status {status}"
                    )));
                }
            }
        }

        Err(SessionError::AuthError(
            "SIP MESSAGE authentication retry was exhausted".to_string(),
        ))
    }

    /// Re-author method-specific origin/proxy credentials negotiated by this
    /// exact dialog's initial INVITE. Digest credentials cannot be copied from
    /// INVITE because HA2 binds the request method, URI, and optional body;
    /// recompute them for MESSAGE while advancing the retained nonce counter.
    /// No challenge or credential is taken from another session or target.
    async fn authorize_data_message_on_dialog(
        &self,
        dialog_id: &RvoipDialogId,
        request: &mut rvoip_sip_core::Request,
        next_hop: &str,
        fresh_challenge: Option<&DataMessageAuthChallenge>,
    ) -> Result<()> {
        use crate::session_store::state::InviteCredentialKind;

        let session_id = self
            .dialog_to_session
            .get(dialog_id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| {
                SessionError::InvalidTransition(
                    "SIP MESSAGE exact dialog has no owning session".to_string(),
                )
            })?;
        let snapshot = self
            .store
            .get_session_snapshot(&session_id)
            .await
            .map_err(|_| {
                SessionError::InvalidTransition(
                    "SIP MESSAGE exact session is no longer available".to_string(),
                )
            })?;
        let handle = snapshot.state().lifecycle_handle.clone().ok_or_else(|| {
            SessionError::InvalidTransition(
                "SIP MESSAGE exact session has no lifecycle authority".to_string(),
            )
        })?;
        if snapshot
            .state()
            .dialog_id
            .as_ref()
            .is_none_or(|current| current.as_uuid() != &dialog_id.0)
        {
            return Err(SessionError::InvalidTransition(
                "SIP MESSAGE exact dialog no longer owns its session".to_string(),
            ));
        }
        if !snapshot.state().dialog_established
            || snapshot.state().call_state == crate::types::CallState::Terminating
            || snapshot.state().call_state.is_final()
        {
            return Err(SessionError::InvalidTransition(
                "SIP MESSAGE exact dialog is no longer active".to_string(),
            ));
        }
        // Preserve the no-write fast path for ordinary unauthenticated
        // MESSAGE delivery.  A fresh challenge must still enter the exact
        // update so installing its protection-space credential and nonce
        // counter is atomic with any concurrent benign session mutation.
        if snapshot.state().invite_authorization_credentials.is_empty() && fresh_challenge.is_none()
        {
            return Ok(());
        }
        let request_uri = request.uri.to_string();
        let transport = self.outbound_transport_context_for_uri(next_hop);
        let body = Some(request.body.as_ref());
        let headers = update_retained_auth_exact(
            self.store.as_ref(),
            &handle,
            "SIP MESSAGE exact session changed during authentication",
            |session| -> Result<Vec<_>> {
                if session
                    .dialog_id
                    .as_ref()
                    .is_none_or(|current| current.as_uuid() != &dialog_id.0)
                {
                    return Err(SessionError::InvalidTransition(
                        "SIP MESSAGE exact dialog no longer owns its session".to_string(),
                    ));
                }
                if !session.dialog_established
                    || session.call_state == crate::types::CallState::Terminating
                    || session.call_state.is_final()
                {
                    return Err(SessionError::InvalidTransition(
                        "SIP MESSAGE exact dialog is no longer active".to_string(),
                    ));
                }
                if session.invite_authorization_credentials.is_empty() && fresh_challenge.is_none()
                {
                    return Ok(Vec::new());
                }
                let auth = session
                    .auth
                    .clone()
                    .or_else(|| session.credentials.clone().map(Into::into))
                    .ok_or_else(|| {
                        SessionError::AuthError(
                            "SIP MESSAGE dialog retained a challenge without route credentials"
                                .to_string(),
                        )
                    })?;
                let mut credentials = session.invite_authorization_credentials.clone();
                let mut digest_nc = session.digest_nc.clone();
                let origin_target = session
                    .remote_uri
                    .clone()
                    .unwrap_or_else(|| request_uri.clone());

                if let Some(fresh) = fresh_challenge {
                    let kind = if fresh.status == 407 {
                        InviteCredentialKind::Proxy
                    } else {
                        InviteCredentialKind::Origin
                    };
                    let preview = auth
                        .authorization_for_challenge_with_transport_context(
                            &fresh.value,
                            "MESSAGE",
                            &request_uri,
                            1,
                            body,
                            &transport,
                        )
                        .map_err(|error| {
                            crate::errors::redacted_outbound_auth_error(
                                crate::errors::OutboundAuthOperation::Request,
                                error,
                            )
                        })?;
                    let realm = data_message_auth_realm(&preview);
                    let nonce = preview
                        .digest_challenge
                        .as_ref()
                        .map(|challenge| challenge.nonce.clone());
                    let existing = credentials.iter().position(|credential| {
                        credential.kind == kind && credential.realm == realm
                    });
                    if existing.is_none()
                        && credentials.iter().any(|credential| credential.kind == kind)
                    {
                        return Err(SessionError::AuthError(
                            "SIP MESSAGE challenge changed the exact dialog protection space"
                                .to_string(),
                        ));
                    }
                    let (protection_target, stale_refreshes) = if let Some(index) = existing {
                        let credential = &credentials[index];
                        if !preview.stale
                            || credential.nonce == nonce
                            || credential.stale_refreshes >= 1
                        {
                            return Err(SessionError::AuthError(
                                "SIP MESSAGE repeated a non-refreshing authentication challenge"
                                    .to_string(),
                            ));
                        }
                        (
                            credential.protection_target.clone(),
                            credential.stale_refreshes.saturating_add(1),
                        )
                    } else {
                        if credentials.len() >= 8 {
                            return Err(SessionError::AuthError(
                                "SIP MESSAGE authentication protection-space limit was reached"
                                    .to_string(),
                            ));
                        }
                        (
                            if kind == InviteCredentialKind::Origin {
                                origin_target.clone()
                            } else {
                                next_hop.to_string()
                            },
                            0,
                        )
                    };
                    let credential = crate::session_store::state::InviteAuthorizationCredential {
                        kind,
                        protection_target,
                        challenge_raw: fresh.value.clone(),
                        realm,
                        nonce,
                        stale_refreshes,
                        value: String::new(),
                    };
                    if let Some(index) = existing {
                        credentials[index] = credential;
                    } else {
                        credentials.push(credential);
                    }
                }

                let mut headers = Vec::with_capacity(credentials.len());
                for credential in &mut credentials {
                    let applies_to_exact_target = match credential.kind {
                        InviteCredentialKind::Origin => {
                            credential.protection_target == origin_target
                        }
                        InviteCredentialKind::Proxy => credential.protection_target == next_hop,
                    };
                    if !applies_to_exact_target {
                        continue;
                    }
                    let preview = auth
                        .authorization_for_challenge_with_transport_context(
                            &credential.challenge_raw,
                            "MESSAGE",
                            &request_uri,
                            1,
                            body,
                            &transport,
                        )
                        .map_err(|error| {
                            crate::errors::redacted_outbound_auth_error(
                                crate::errors::OutboundAuthOperation::Request,
                                error,
                            )
                        })?;
                    if let Some(challenge) = preview.digest_challenge.as_ref() {
                        if challenge.realm != credential.realm
                            || credential.nonce.as_deref() != Some(challenge.nonce.as_str())
                        {
                            return Err(SessionError::AuthError(
                                "SIP MESSAGE challenge no longer matches the exact dialog protection space"
                                    .to_string(),
                            ));
                        }
                    }
                    let nonce_count = if let Some(challenge) = preview.digest_challenge.as_ref() {
                        let key = (challenge.realm.clone(), challenge.nonce.clone());
                        *digest_nc
                            .entry(key)
                            .and_modify(|count| *count = count.saturating_add(1))
                            .or_insert(1)
                    } else {
                        1
                    };
                    let selected = if preview.digest_challenge.is_some() && nonce_count != 1 {
                        auth.authorization_for_challenge_with_transport_context(
                            &credential.challenge_raw,
                            "MESSAGE",
                            &request_uri,
                            nonce_count,
                            body,
                            &transport,
                        )
                        .map_err(|error| {
                            crate::errors::redacted_outbound_auth_error(
                                crate::errors::OutboundAuthOperation::Request,
                                error,
                            )
                        })?
                    } else {
                        preview
                    };
                    credential.value = selected.value.clone();
                    let name = match credential.kind {
                        InviteCredentialKind::Origin => {
                            rvoip_sip_core::types::HeaderName::Authorization
                        }
                        InviteCredentialKind::Proxy => {
                            rvoip_sip_core::types::HeaderName::ProxyAuthorization
                        }
                    };
                    headers.push(
                        rvoip_sip_core::validation::validated_authorization_header(
                            name,
                            selected.value,
                        )
                        .map_err(|_| {
                            SessionError::AuthError(
                                "SIP MESSAGE authorization failed wire-safety validation"
                                    .to_string(),
                            )
                        })?,
                    );
                }
                session.invite_authorization_credentials = credentials;
                session.digest_nc = digest_nc;
                Ok(headers)
            },
        )?;
        request.headers.extend(headers);
        Ok(())
    }

    /// Send a MESSAGE request (can be in-dialog or out-of-dialog)
    pub async fn send_message(
        &self,
        session_id: &SessionId,
        from_uri: &str,
        to_uri: &str,
        body: String,
        in_dialog: bool,
    ) -> Result<()> {
        tracing::info!(
            "Sending MESSAGE for session {} from {} to {} (in_dialog: {})",
            session_id.0,
            from_uri,
            to_uri,
            in_dialog
        );

        if in_dialog {
            // Send MESSAGE within existing dialog
            let dialog_id = self
                .session_to_dialog
                .get(session_id)
                .ok_or_else(|| SessionError::DialogError("No dialog for session".to_string()))?
                .clone();

            self.dialog_api
                .send_request_in_dialog(
                    &dialog_id,
                    rvoip_sip_core::Method::Message,
                    Some(bytes::Bytes::from(body)),
                )
                .await
                .map_err(|e| {
                    SessionError::DialogError(format!("Failed to send MESSAGE in dialog: {}", e))
                })?;
        } else {
            // Send MESSAGE as standalone (no dialog). dialog-core owns the
            // wire-ready SIP request construction.
            let response = self
                .dialog_api
                .send_message_out_of_dialog_with_options(MessageRequestOptions {
                    from_uri: from_uri.to_string(),
                    to_uri: to_uri.to_string(),
                    content_type: String::from("text/plain"),
                    body: bytes::Bytes::from(body),
                    ..Default::default()
                })
                .await
                .map_err(|e| SessionError::DialogError(format!("Failed to send MESSAGE: {}", e)))?;

            // Handle response
            if response.status_code() == 200 {
                let event = RvoipCrossCrateEvent::DialogToSession(
                    rvoip_infra_common::events::cross_crate::DialogToSessionEvent::MessageDelivered {
                        session_id: session_id.0.clone(),
                    }
                );
                let _ = self.global_coordinator.publish(Arc::new(event)).await;
            } else if response.status_code() >= 400 {
                let event = RvoipCrossCrateEvent::DialogToSession(
                    rvoip_infra_common::events::cross_crate::DialogToSessionEvent::MessageFailed {
                        session_id: session_id.0.clone(),
                        status_code: response.status_code(),
                    },
                );
                let _ = self.global_coordinator.publish(Arc::new(event)).await;
            }
        }

        tracing::info!("MESSAGE sent successfully for session {}", session_id.0);
        Ok(())
    }

    // ===== Helper Methods =====

    // ===== Inbound Events (from dialog-core) =====

    /// Start the dialog API (no event handling here)
    pub async fn start(&self) -> Result<()> {
        // Start the dialog API
        self.dialog_api
            .start()
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to start dialog API: {}", e)))?;

        Ok(())
    }

    /// Stop the dialog API and release its transaction transports.
    pub async fn stop(&self) -> Result<()> {
        self.dialog_api
            .stop()
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to stop dialog API: {}", e)))?;

        Ok(())
    }
}

impl Clone for DialogAdapter {
    fn clone(&self) -> Self {
        Self {
            dialog_api: self.dialog_api.clone(),
            store: self.store.clone(),
            session_to_dialog: self.session_to_dialog.clone(),
            dialog_to_session: self.dialog_to_session.clone(),
            callid_to_session: self.callid_to_session.clone(),
            outgoing_invite_tx: self.outgoing_invite_tx.clone(),
            outgoing_bye_tx: self.outgoing_bye_tx.clone(),
            outgoing_bye_generation_watch: self.outgoing_bye_generation_watch.clone(),
            next_outgoing_bye_generation: self.next_outgoing_bye_generation.clone(),
            non_invite_transaction_timeout: self.non_invite_transaction_timeout,
            outbound_request_tracker: self.outbound_request_tracker.clone(),
            outbound_initial_invites: self.outbound_initial_invites.clone(),
            data_message_dispatch_lanes: self.data_message_dispatch_lanes.clone(),
            auto_emit_extra_headers: self.auto_emit_extra_headers.clone(),
            global_coordinator: self.global_coordinator.clone(),
            state_machine: self.state_machine.clone(),
            outbound_proxy_uri: self.outbound_proxy_uri.clone(),
            outbound_contact_params: self.outbound_contact_params.clone(),
            symmetric_flow_params: self.symmetric_flow_params.clone(),
            registration_auto_refresh: self.registration_auto_refresh,
            registration_refresh_jitter_percent: self.registration_refresh_jitter_percent,
            registration_refresh_admission: self.registration_refresh_admission.clone(),
            registration_refresh_tasks: self.registration_refresh_tasks.clone(),
            registration_refresh_retained: self.registration_refresh_retained.clone(),
            next_registration_refresh_generation: self.next_registration_refresh_generation.clone(),
            cleanup_attempt_total: self.cleanup_attempt_total.clone(),
            cleanup_mapped_total: self.cleanup_mapped_total.clone(),
            cleanup_missing_total: self.cleanup_missing_total.clone(),
            cleanup_call_ids_removed_total: self.cleanup_call_ids_removed_total.clone(),
            cleanup_outgoing_invite_removed_total: self
                .cleanup_outgoing_invite_removed_total
                .clone(),
            #[cfg(test)]
            cleanup_pause: self.cleanup_pause.clone(),
            trace_redactor: self.trace_redactor.clone(),
        }
    }
}

#[cfg(test)]
pub(crate) struct DialogCleanupPause {
    entered: std::sync::atomic::AtomicBool,
    entered_notify: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

#[cfg(test)]
#[allow(dead_code)]
impl DialogCleanupPause {
    fn new() -> Self {
        Self {
            entered: std::sync::atomic::AtomicBool::new(false),
            entered_notify: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        }
    }

    pub(crate) async fn wait_entered(&self) {
        while !self.entered.load(Ordering::Acquire) {
            self.entered_notify.notified().await;
        }
    }

    pub(crate) fn release(&self) {
        self.release.notify_waiters();
    }
}

#[cfg(test)]
// Policy helpers remain below this focused diagnostic module so the production
// API stays grouped with its documentation.
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

    #[test]
    fn recorded_bye_success_wins_simultaneous_cleanup_watch_close() {
        let outcome = Ok(Some(ClientTransactionOutcome::FinalResponse(
            rvoip_sip_core::Response::new(rvoip_sip_core::StatusCode::Ok),
        )));

        match resolve_outgoing_bye_generation_wake(outcome, false, true, false) {
            OutgoingByeGenerationWake::UseExactOutcome(Ok(Some(
                ClientTransactionOutcome::FinalResponse(response),
            ))) => assert_eq!(response.status_code(), 200),
            _ => panic!("the recorded exact BYE response must outrank cleanup watch closure"),
        }
    }

    #[test]
    fn exact_response_transaction_diagnostics_are_bounded_and_redacted() {
        const SECRET_BRANCH: &str = "z9hG4bK-exact-response-secret-branch";
        const SECRET_METHOD: &str = "X-EXACT-RESPONSE-SECRET-METHOD";
        let transaction = TransactionKey::new(
            SECRET_BRANCH.to_string(),
            rvoip_sip_core::Method::Extension(SECRET_METHOD.to_string()),
            true,
        );

        let diagnostics = exact_response_transaction_diagnostics(&transaction);
        let rendered = format!("{diagnostics:?}");
        assert_eq!(diagnostics, ("extension", "server"));
        assert!(!rendered.contains(SECRET_BRANCH));
        assert!(!rendered.contains(SECRET_METHOD));

        let source = include_str!("dialog_adapter.rs");
        let forbidden_raw_format = ["via transaction ", "{}"].concat();
        assert!(
            !source.contains(&forbidden_raw_format),
            "exact response diagnostics regained raw transaction formatting"
        );
    }

    #[tokio::test]
    async fn registration_refresh_shutdown_cooperatively_cancels_long_sleep() {
        let coordinator = crate::api::unified::UnifiedCoordinator::new(
            crate::api::unified::Config::local("refresh-drain", 0),
        )
        .await
        .expect("coordinator");
        let adapter = Arc::clone(coordinator.dialog_adapter());
        adapter.schedule_registration_refresh(
            SessionId::new(),
            Some(Instant::now() + Duration::from_secs(60)),
        );
        assert_eq!(adapter.registration_refresh_tasks.len(), 1);
        assert_eq!(adapter.registration_refresh_retained.count(), 1);

        adapter
            .abort_all_registration_refreshes_and_wait()
            .await
            .expect("cooperative refresh cancellation drained");
        assert!(adapter.registration_refresh_tasks.is_empty());
        assert_eq!(adapter.registration_refresh_retained.count(), 0);
        assert!(!adapter.registration_refresh_retained.panicked());

        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("coordinator shutdown remains idempotent");
    }

    #[tokio::test]
    async fn retained_auth_exact_update_survives_benign_revision_advance() {
        let store = SessionStore::new();
        let session_id = SessionId::new();
        let dialog_id = DialogId::new();
        store
            .create_session(session_id.clone(), crate::state_table::Role::UAC, false)
            .await
            .expect("create exact retained-auth session");
        store
            .update_session_with(&session_id, |session| {
                session.dialog_id = Some(dialog_id.clone());
                session.dialog_established = true;
                session.call_state = crate::types::CallState::Active;
                session.remote_uri = Some("sip:peer@example.test".to_string());
            })
            .await
            .expect("publish established dialog");

        // Model the old race deterministically: auth captures one exact
        // lifetime, then an unrelated state-machine field advances the
        // session revision before Digest nonce bookkeeping runs.
        let auth_read = store
            .get_session_snapshot(&session_id)
            .await
            .expect("capture auth read");
        let auth_read_revision = auth_read.revision();
        let handle = auth_read
            .state()
            .lifecycle_handle
            .clone()
            .expect("exact retained-auth handle");
        store
            .update_session_with(&session_id, |session| {
                session.call_established_triggered = true;
                session.sdp_origin_version = session.sdp_origin_version.saturating_add(1);
            })
            .await
            .expect("publish benign concurrent mutation");

        let observed_origin_version = update_retained_auth_exact(
            &store,
            &handle,
            "retained auth exact session changed",
            |session| {
                assert_eq!(session.dialog_id.as_ref(), Some(&dialog_id));
                let nonce_count = session
                    .digest_nc
                    .entry(("dialog-realm".to_string(), "dialog-nonce".to_string()))
                    .or_insert(0);
                *nonce_count = nonce_count.saturating_add(1);
                Ok(session.sdp_origin_version)
            },
        )
        .expect("latest exact session revision accepts retained auth update");

        let after = store
            .get_session_snapshot(&session_id)
            .await
            .expect("read retained-auth result");
        assert!(after.revision() > auth_read_revision);
        assert!(after.state().call_established_triggered);
        assert_eq!(after.state().sdp_origin_version, observed_origin_version);
        assert_eq!(
            after
                .state()
                .digest_nc
                .get(&("dialog-realm".to_string(), "dialog-nonce".to_string())),
            Some(&1)
        );
    }

    #[test]
    fn invite_dispatch_errors_do_not_relay_lower_sources() {
        const SECRET: &str = "lower-dialog-option-secret-canary";
        for failure in [
            InviteDispatchFailure::Initial,
            InviteDispatchFailure::InitialWithExtraHeaders,
            InviteDispatchFailure::InitialWithOptions,
            InviteDispatchFailure::AuthRetry,
            InviteDispatchFailure::SessionTimerRetry,
            InviteDispatchFailure::LegacyUpdateReinvite,
            InviteDispatchFailure::ReinviteWithOptions,
            InviteDispatchFailure::ReinviteInDialog,
        ] {
            let error = redacted_invite_dispatch_error(
                failure,
                format!(
                    "invalid From sip:{SECRET}@from.invalid; target=sip:{SECRET}@target.invalid; Authorization: Bearer {SECRET}; X-App: {SECRET}"
                ),
            );
            let display = error.to_string();
            let debug = format!("{error:?}");
            for rendered in [&display, &debug] {
                assert!(!rendered.contains(SECRET), "source leaked: {rendered}");
                assert!(!rendered.contains("sip:"));
                assert!(!rendered.contains("Authorization"));
                assert!(!rendered.contains("X-App"));
            }
            let SessionError::DialogError(detail) = &error else {
                panic!("unexpected invite error class: {error:?}");
            };
            assert!(detail.contains("class="));
            assert!(detail.contains(failure.diagnostic()));
        }
    }

    #[test]
    fn invite_wrapper_source_has_no_lower_error_relay_templates() {
        let source = include_str!("dialog_adapter.rs");
        for forbidden in [
            ["Failed to make call", ": {}"].concat(),
            ["Failed to make call with extra headers", ": {}"].concat(),
            ["Failed to send INVITE with options", ": {}"].concat(),
            ["resend_invite_with_auth failed for session {}", ": {}"].concat(),
            [
                "resend_invite_with_session_timer_override failed for session {}",
                ": {}",
            ]
            .concat(),
            ["Failed to send re-INVITE", ": {}"].concat(),
        ] {
            assert!(
                !source.contains(&forbidden),
                "lower error relay template returned: {forbidden}"
            );
        }
    }

    #[test]
    fn register_diagnostics_never_format_live_uri_contact_or_scheme_values() {
        let source = include_str!("dialog_adapter.rs");
        for forbidden in [
            ["Sending REGISTER for session {}", " to {}"].concat(),
            ["Computing auth for REGISTER", " uri={}"].concat(),
            ["Computed REGISTER auth", " using {:?}"].concat(),
            ["rewriting REGISTER Contact", " {}"].concat(),
            ["Failed to send REGISTER", ": {}"].concat(),
        ] {
            assert!(
                !source.contains(&forbidden),
                "live REGISTER diagnostic template returned: {forbidden}"
            );
        }

        let scheme_canary = "SCHEME_CANARY_SECRET_19cf";
        assert_eq!(
            register_auth_scheme_class(&crate::auth::SipAuthScheme::Other(
                scheme_canary.to_string()
            )),
            "other"
        );
        assert!(
            !register_auth_scheme_class(&crate::auth::SipAuthScheme::Other(
                scheme_canary.to_string()
            ))
            .contains(scheme_canary)
        );
    }

    // ---- NAT-aware Contact rewrite (Sprint 1.A3) -------------------

    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    fn pub_addr() -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7)), 54321)
    }

    #[test]
    fn rewrite_contact_swaps_host_port_after_user() {
        // Standard `sip:user@host:port` form — host:port replaced.
        let input = "sip:alice@192.168.1.10:5060";
        assert_eq!(
            rewrite_contact_host(input, pub_addr()),
            "sip:alice@203.0.113.7:54321"
        );
    }

    #[test]
    fn rewrite_contact_preserves_uri_params() {
        let input = "sip:alice@192.168.1.10:5060;transport=tcp";
        assert_eq!(
            rewrite_contact_host(input, pub_addr()),
            "sip:alice@203.0.113.7:54321;transport=tcp"
        );
    }

    #[test]
    fn rewrite_contact_handles_no_port_in_input() {
        let input = "sip:alice@192.168.1.10";
        assert_eq!(
            rewrite_contact_host(input, pub_addr()),
            "sip:alice@203.0.113.7:54321"
        );
    }

    #[test]
    fn rewrite_contact_handles_no_user() {
        // Some Contacts omit the user-part — rewrite host:port anyway.
        let input = "sip:192.168.1.10:5060";
        assert_eq!(
            rewrite_contact_host(input, pub_addr()),
            "sip:203.0.113.7:54321"
        );
    }

    #[test]
    fn rewrite_contact_passes_through_sips_scheme() {
        let input = "sips:alice@192.168.1.10:5061;transport=tls";
        assert_eq!(
            rewrite_contact_host(input, pub_addr()),
            "sips:alice@203.0.113.7:54321;transport=tls"
        );
    }

    // ---- E4 outbound proxy pre-loaded Route ---------------------------

    use rvoip_sip_core::types::{uri::Uri, TypedHeader};
    use std::str::FromStr;

    #[test]
    fn prepend_outbound_proxy_route_with_proxy_adds_first_route() {
        let proxy = Uri::from_str("sip:sbc.example.com;lr").unwrap();
        let headers = prepend_outbound_proxy_route(Vec::new(), Some(&proxy));
        assert_eq!(headers.len(), 1);
        match &headers[0] {
            TypedHeader::Route(route) => {
                assert_eq!(route.len(), 1);
                assert_eq!(route[0].0.uri.to_string(), "sip:sbc.example.com;lr");
            }
            other => panic!("expected TypedHeader::Route, got {:?}", other),
        }
    }

    #[test]
    fn prepend_outbound_proxy_route_without_proxy_is_identity() {
        let pai_uri = Uri::from_str("sip:alice@pai.example.com").unwrap();
        let existing = vec![TypedHeader::PAssertedIdentity(
            rvoip_sip_core::types::p_asserted_identity::PAssertedIdentity::with_uri(pai_uri),
        )];
        let headers = prepend_outbound_proxy_route(existing.clone(), None);
        assert_eq!(headers.len(), existing.len());
        assert!(matches!(headers[0], TypedHeader::PAssertedIdentity(_)));
    }

    #[test]
    fn prepend_outbound_proxy_route_preserves_existing_before_route() {
        // Route goes FIRST, caller extras preserved after.
        let proxy = Uri::from_str("sip:sbc.example.com;lr").unwrap();
        let pai_uri = Uri::from_str("sip:alice@pai.example.com").unwrap();
        let existing = vec![TypedHeader::PAssertedIdentity(
            rvoip_sip_core::types::p_asserted_identity::PAssertedIdentity::with_uri(pai_uri),
        )];
        let headers = prepend_outbound_proxy_route(existing, Some(&proxy));
        assert_eq!(headers.len(), 2);
        assert!(matches!(headers[0], TypedHeader::Route(_)));
        assert!(matches!(headers[1], TypedHeader::PAssertedIdentity(_)));
    }

    #[test]
    fn auth_retry_policy_rejects_line_smuggling_for_401_and_407_headers() {
        for header_name in ["Authorization", "Proxy-Authorization"] {
            let canary = format!("Bearer safe\r\nX-Injected-{header_name}: yes");
            let error = apply_outbound_extras_policy_with_auth(
                rvoip_sip_core::types::Method::Invite,
                Vec::new(),
                None,
                header_name,
                canary.clone(),
            )
            .expect_err("auth retry controls must fail before header insertion");
            assert!(!error.to_string().contains(&canary));
        }
    }

    #[test]
    fn auth_retry_policy_preserves_valid_values_for_401_and_407_headers() {
        use rvoip_sip_core::types::headers::{HeaderName, HeaderValue};

        for (wire_name, expected_name) in [
            ("Authorization", HeaderName::Authorization),
            ("Proxy-Authorization", HeaderName::ProxyAuthorization),
        ] {
            let value = "Digest username=\"alice\", response=\"safe\"";
            let headers = apply_outbound_extras_policy_with_auth(
                rvoip_sip_core::types::Method::Invite,
                Vec::new(),
                None,
                wire_name,
                value.to_string(),
            )
            .expect("valid retry header");
            assert!(matches!(
                headers.as_slice(),
                [TypedHeader::Other(name, HeaderValue::Raw(bytes))]
                    if *name == expected_name && bytes.as_slice() == value.as_bytes()
            ));
        }
    }

    #[test]
    fn auth_retry_policy_accepts_case_aliases_and_rejects_unknown_names() {
        let value = "Digest username=\"alice\", response=\"safe\"".to_string();
        for name in ["authorization", "PROXY-authorization"] {
            apply_outbound_extras_policy_with_auth(
                rvoip_sip_core::types::Method::Invite,
                Vec::new(),
                None,
                name,
                value.clone(),
            )
            .expect("case-insensitive credential header name");
        }
        for name in ["", "Proxy-Authenticate", "Authorization ", "X-Auth"] {
            let error = apply_outbound_extras_policy_with_auth(
                rvoip_sip_core::types::Method::Invite,
                Vec::new(),
                None,
                name,
                value.clone(),
            )
            .expect_err("unknown credential header names must fail closed");
            assert!(error.to_string().contains("unsupported"));
        }
    }
}

/// Rewrite the host (and port) portion of a SIP URI in a `Contact:`
/// value with the supplied public address. Preserves the scheme,
/// user-part (if any), and any URI parameters.
///
/// Used by `DialogAdapter::send_register` to redirect the registrar's
/// stored binding to the NAT-discovered public address (RFC 5626 §5).
/// Pure / sync so the rewrite is trivially testable without standing
/// up the full adapter.
///
/// Format we handle: `<scheme>:[<user>@]<host>[:<port>][;<params>]`.
/// We deliberately don't lean on a full URI parser here — the input
/// is always a Contact value we built ourselves earlier in the
/// pipeline, so the structure is predictable.
pub(crate) fn rewrite_contact_host(input: &str, public: std::net::SocketAddr) -> String {
    // Split off any URI params (`;name=value` after the host[:port]).
    let (host_section, params_suffix) = match input.find(';') {
        Some(idx) => (&input[..idx], &input[idx..]),
        None => (input, ""),
    };

    // Split scheme: prefix (`sip:` or `sips:`).
    let (scheme_prefix, after_scheme) = match host_section.find(':') {
        Some(idx) => (&host_section[..=idx], &host_section[idx + 1..]),
        None => return input.to_string(), // No `:` — not a SIP URI we recognise.
    };

    // Split optional `<user>@`.
    let (user_at, _existing_host_port) = match after_scheme.find('@') {
        Some(idx) => (&after_scheme[..=idx], &after_scheme[idx + 1..]),
        None => ("", after_scheme),
    };

    format!("{}{}{}{}", scheme_prefix, user_at, public, params_suffix)
}

/// SBC topology hiding (RFC 3261 §16-style) — strip every `Via:`
/// header below the topmost one.
///
/// Used when an SBC or stateless proxy mutates an inbound request
/// in-place before forwarding it, and wants to hide upstream hop
/// identities from the downstream peer. The top Via is preserved so
/// that the response can route back to *some* sender (typically the
/// SBC itself after it re-stamps the top Via with its own sent-by).
///
/// **NOT used by the B2BUA pattern in this codebase** — the standard
/// `coord.invite(...)` path builds a fresh outbound Request with the
/// SBC's own Via stamped fresh, so there's nothing to strip. This
/// helper is meaningful for proxy-style flows on top of
/// `Transport::send_message_raw` (i.e. the helpers planned for Phase
/// 8.5 stateless-proxy support).
///
/// Returns the number of Via headers removed (0 if there was only
/// one to begin with — common for endpoints that talk directly to
/// the SBC without intermediate proxies).
pub fn strip_via_below_top(request: &mut rvoip_sip_core::Request) -> usize {
    use rvoip_sip_core::types::TypedHeader;
    let mut seen_first_via = false;
    let mut removed = 0;
    request.headers.retain(|h| {
        if matches!(h, TypedHeader::Via(_)) {
            if seen_first_via {
                removed += 1;
                false
            } else {
                seen_first_via = true;
                true
            }
        } else {
            true
        }
    });
    removed
}

/// SBC topology hiding — strip every `Record-Route:` header whose
/// host does NOT match the supplied `self_host` (the SBC's own
/// public-facing host).
///
/// RFC 3261 §16.6 requires proxies to insert their own Record-Route
/// before forwarding so subsequent in-dialog requests come back
/// through them. An SBC doing topology hiding wants downstream to
/// see ONLY the SBC's own entry, not the upstream proxies that
/// previously inserted theirs.
///
/// `self_host` is matched against `Address.uri.host` as a case-
/// insensitive string. Pass the SBC's externally-visible host (e.g.
/// `"sbc.example.com"` or `"203.0.113.5"`) — typically what's also
/// used in `rewrite_contact_host`.
///
/// Returns the number of Record-Route entries (across all headers)
/// removed.
pub fn strip_record_route_below_self(
    request: &mut rvoip_sip_core::Request,
    self_host: &str,
) -> usize {
    use rvoip_sip_core::types::TypedHeader;
    let self_lower = self_host.to_ascii_lowercase();
    let mut removed = 0;

    // First pass: filter each RecordRoute header's entries.
    for header in request.headers.iter_mut() {
        if let TypedHeader::RecordRoute(rr) = header {
            let before = rr.0.len();
            rr.0.retain(|entry| {
                let host = entry.0.uri.host.to_string().to_ascii_lowercase();
                host == self_lower
            });
            removed += before - rr.0.len();
        }
    }

    // Second pass: drop any RecordRoute headers that became empty.
    request.headers.retain(|h| match h {
        TypedHeader::RecordRoute(rr) => !rr.0.is_empty(),
        _ => true,
    });

    removed
}

/// E4 / RFC 3261 §8.1.2: produce the full `extra_headers` list for an
/// outgoing INVITE, prepending a pre-loaded `Route` header when an outbound
/// proxy is configured on the `DialogAdapter`.
///
/// Pure so the "which headers travel on the wire" decision can be validated
/// without constructing a dialog_api / transport stack. Callers:
/// `DialogAdapter::send_invite_with_extra_headers`.
pub(crate) fn prepend_outbound_proxy_route(
    extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    outbound_proxy_uri: Option<&rvoip_sip_core::types::uri::Uri>,
) -> Vec<rvoip_sip_core::types::TypedHeader> {
    let mut headers = extra_headers;
    if let Some(uri) = outbound_proxy_uri {
        use rvoip_sip_core::types::{route::Route, TypedHeader};
        headers.insert(0, TypedHeader::Route(Route::with_uri(uri.clone())));
    }
    headers
}

/// SIP_API_DESIGN_2 §5.4 + §6.1 — the canonical pre-dispatch step for
/// every `send_*_with_options` mirror on [`DialogAdapter`]. Runs
/// [`crate::api::headers::policy::validate_outbound`] against the
/// application extras (catches stack-managed names that bypassed the
/// builder's strictness gate), then prepends the configured outbound
/// proxy's `Route:` header via [`prepend_outbound_proxy_route`].
///
/// Returns the rewritten extras vector or a typed `SessionError` if
/// the header policy rejects the staged set. The dialog-adapter
/// mirror passes the returned vector through to dialog-core.
pub(crate) fn apply_outbound_extras_policy(
    method: rvoip_sip_core::types::Method,
    extras: Vec<rvoip_sip_core::types::TypedHeader>,
    outbound_proxy_uri: Option<&rvoip_sip_core::types::uri::Uri>,
) -> Result<Vec<rvoip_sip_core::types::TypedHeader>> {
    if let Err(violations) = crate::api::headers::policy::validate_outbound(method, &extras) {
        // Map the first violation to SessionError::HeaderPolicy; the
        // policy returns the StackManaged-in-extras case as a
        // MissingRequiredHeader-shaped violation today.
        let first = violations.into_iter().next().expect("non-empty on Err");
        return Err(SessionError::HeaderPolicy {
            method: first.method,
            header: first.name,
            reason: crate::api::headers::ViolationReason::StackManaged,
        });
    }
    Ok(prepend_outbound_proxy_route(extras, outbound_proxy_uri))
}

/// SIP_API_DESIGN_2 R2 — auth-retry mirror of
/// [`apply_outbound_extras_policy`]. Runs the same policy validation
/// on the application extras, then **appends** the
/// `Authorization:` / `Proxy-Authorization:` header *after* policy
/// validation. The auth header bypasses the policy because:
///
/// 1. The HeaderPolicy classifies `Authorization` as `MethodShaped`
///    for INVITE / REGISTER / SUBSCRIBE / MESSAGE / OPTIONS / REFER,
///    meaning application code can't stage it via `with_raw_header`.
/// 2. But the state machine *itself* stages it on the auth-retry hop
///    via `Action::SendRequestWithAuth`, computed from the digest
///    challenge. That's a stack-managed injection, not an application
///    one, so the policy guard is intentionally bypassed.
///
/// `auth_header_name` is the raw wire name (`"Authorization"` or
/// `"Proxy-Authorization"`); `auth_header_value` is the rendered
/// `Digest username="..", ...` body.
pub(crate) fn apply_outbound_extras_policy_with_auth(
    method: rvoip_sip_core::types::Method,
    extras: Vec<rvoip_sip_core::types::TypedHeader>,
    outbound_proxy_uri: Option<&rvoip_sip_core::types::uri::Uri>,
    auth_header_name: &str,
    auth_header_value: String,
) -> Result<Vec<rvoip_sip_core::types::TypedHeader>> {
    let mut validated = apply_outbound_extras_policy(method, extras, outbound_proxy_uri)?;
    let header_name = rvoip_sip_core::validation::authorization_header_name(auth_header_name)
        .map_err(|_| {
            crate::errors::SessionError::AuthError(
                "unsupported outbound SIP authorization header name".to_string(),
            )
        })?;
    let authorization =
        rvoip_sip_core::validation::validated_authorization_header(header_name, auth_header_value)
            .map_err(|_| {
                crate::errors::SessionError::AuthError(
                    "outbound SIP authorization header failed wire-safety validation".to_string(),
                )
            })?;
    validated.push(authorization);
    Ok(validated)
}
