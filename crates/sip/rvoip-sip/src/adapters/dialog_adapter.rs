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
use crate::session_store::{SessionState, SessionStore};
use crate::sip_data_message::{
    build_sip_data_request, SipDataMessage, SipDataMessageDispatchLanes,
};
use crate::state_table::{
    types::{DialogId, SessionId},
    EventType,
};
use dashmap::DashMap;
use rvoip_infra_common::events::coordinator::GlobalEventCoordinator;
use rvoip_sip_core::{Method, Response, StatusCode, Uri};
use rvoip_sip_dialog::{
    api::unified::{
        ByeRequestOptions, CancelRequestOptions, InfoRequestOptions, MessageRequestOptions,
        NotifyRequestOptions, OptionsRequestOptions, ReferRequestOptions, RegisterRequestOptions,
        SubscribeRequestOptions, UnifiedDialogApi, UpdateRequestOptions,
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

fn spawn_api_event_observation(
    coordinator: Arc<GlobalEventCoordinator>,
    api_event: crate::api::events::Event,
) -> tokio::task::JoinHandle<()> {
    let wrapped = crate::adapters::SessionApiCrossCrateEvent::new(
        crate::adapters::sanitize_session_api_observation(&api_event),
    );
    tokio::spawn(async move {
        if let Err(error) = coordinator.publish_observational(wrapped).await {
            tracing::warn!(%error, "Failed to publish app-level dialog adapter event");
        }
    })
}

/// One immutable snapshot for every transaction-oriented standalone request.
///
/// MESSAGE, OPTIONS, and SUBSCRIBE intentionally remain outside the call
/// state machine. Keeping their request identity in this private enum lets the
/// coordinator share one authentication/retry implementation without
/// manufacturing compatibility sessions.
#[derive(Clone)]
pub(crate) enum StandaloneRequestOptions {
    Message(MessageRequestOptions),
    Options(OptionsRequestOptions),
    Subscribe {
        target: String,
        options: SubscribeRequestOptions,
    },
}

impl StandaloneRequestOptions {
    pub(crate) fn method(&self) -> rvoip_sip_core::Method {
        match self {
            Self::Message(_) => rvoip_sip_core::Method::Message,
            Self::Options(_) => rvoip_sip_core::Method::Options,
            Self::Subscribe { .. } => rvoip_sip_core::Method::Subscribe,
        }
    }

    pub(crate) fn request_uri(&self) -> &str {
        match self {
            Self::Message(options) => &options.to_uri,
            Self::Options(options) => &options.to_uri,
            Self::Subscribe { target, .. } => target,
        }
    }

    pub(crate) fn body(&self) -> Option<&[u8]> {
        match self {
            Self::Message(options) if !options.body.is_empty() => Some(options.body.as_ref()),
            _ => None,
        }
    }

    pub(crate) fn with_request_identity(
        mut self,
        cseq: u32,
        call_id: Option<String>,
        from_tag: Option<String>,
    ) -> Self {
        match &mut self {
            Self::Message(options) => {
                options.cseq = Some(cseq);
                options.call_id = call_id;
                options.from_tag = from_tag;
            }
            Self::Options(options) => {
                options.cseq = Some(cseq);
                options.call_id = call_id;
                options.from_tag = from_tag;
            }
            Self::Subscribe { options, .. } => {
                options.cseq = Some(cseq);
                options.call_id = call_id;
                options.from_tag = from_tag;
            }
        }
        self
    }
}

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
    handle: SessionRegistryHandle,
    generation: u64,
    cancel: tokio::sync::oneshot::Sender<()>,
}

struct RegistrationRefreshCompletion {
    admission: Arc<StdMutex<()>>,
    tasks: Arc<DashMap<SessionId, RegistrationRefreshTask>>,
    handle: SessionRegistryHandle,
    generation: u64,
}

impl Drop for RegistrationRefreshCompletion {
    fn drop(&mut self) {
        let _admission = self
            .admission
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.tasks
            .remove_if(self.handle.session_id(), |_, current| {
                current.handle == self.handle && current.generation == self.generation
            });
    }
}

#[cfg(test)]
struct RegistrationRefreshDispatchPause {
    entered: std::sync::atomic::AtomicBool,
    entered_notify: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

#[cfg(test)]
impl RegistrationRefreshDispatchPause {
    fn new() -> Self {
        Self {
            entered: std::sync::atomic::AtomicBool::new(false),
            entered_notify: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        }
    }

    async fn pause(&self) {
        self.entered.store(true, Ordering::Release);
        self.entered_notify.notify_waiters();
        self.release.notified().await;
    }

    async fn wait_entered(&self) {
        while !self.entered.load(Ordering::Acquire) {
            self.entered_notify.notified().await;
        }
    }

    fn release(&self) {
        self.release.notify_one();
    }
}

#[derive(Clone)]
struct DataMessageAuthChallenge {
    status: u16,
    value: String,
}

#[derive(Clone)]
struct OutboundByeTransaction {
    handle: SessionRegistryHandle,
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

/// Cancellation owner for the gap between BYE dispatch and exact response
/// confirmation. Terminal cleanup may preserve a completed receipt only while
/// at least one owner exists; dropping the last owner exact-removes any newer
/// generation so cancelled builder/adapter futures cannot retain it forever.
pub(crate) struct OutgoingByeWaitOwner {
    handle: SessionRegistryHandle,
    wait_intents: Arc<DashMap<SessionRegistryHandle, OutgoingByeWaitIntentState>>,
    transactions: Arc<DashMap<SessionRegistryHandle, OutboundByeTransaction>>,
    generation_watch: Arc<DashMap<SessionRegistryHandle, tokio::sync::watch::Sender<u64>>>,
}

#[derive(Clone)]
struct OutgoingByeWaitIntentState {
    handle: SessionRegistryHandle,
    owners: usize,
    min_after_generation: u64,
}

impl Drop for OutgoingByeWaitOwner {
    fn drop(&mut self) {
        use dashmap::mapref::entry::Entry;

        if let Entry::Occupied(mut entry) = self.wait_intents.entry(self.handle.clone()) {
            if entry.get().handle != self.handle {
                return;
            }
            if entry.get().owners > 1 {
                entry.get_mut().owners -= 1;
                return;
            }

            // Keep the per-session intent entry occupied through reclamation.
            // A new owner therefore cannot publish a later generation between
            // the last-owner decision and remove_if (the same-session ABA).
            let min_after_generation = entry.get().min_after_generation;
            let removed = self.transactions.remove_if(&self.handle, |_, current| {
                current.handle == self.handle && current.generation > min_after_generation
            });
            if removed.is_some() || !self.transactions.contains_key(&self.handle) {
                self.generation_watch.remove(&self.handle);
            }
            entry.remove();
        }
    }
}

enum OutgoingByeGenerationWake {
    UseExactOutcome(
        rvoip_sip_dialog::transaction::TransactionResult<Option<ClientTransactionOutcome>>,
    ),
    FollowNewerGeneration,
    RetryCurrentGeneration,
    CleanupInterrupted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutgoingByeFinalDisposition {
    Confirmed,
    AuthenticationChallenge,
    PeerAlreadyTerminated,
    Rejected,
}

fn classify_outgoing_bye_final_response(status_code: u16) -> OutgoingByeFinalDisposition {
    match status_code {
        200..=299 => OutgoingByeFinalDisposition::Confirmed,
        401 | 407 => OutgoingByeFinalDisposition::AuthenticationChallenge,
        // RFC 3261 §15.1.1: a 481 response to BYE means the peer no
        // longer has the dialog, and the UAC MUST still consider the session
        // and dialog terminated. This is the ordinary result when two BYEs
        // cross and the peer completes its locally initiated teardown first.
        481 => OutgoingByeFinalDisposition::PeerAlreadyTerminated,
        _ => OutgoingByeFinalDisposition::Rejected,
    }
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

/// Return whether exact cleanup must interrupt an incomplete local-BYE
/// confirmation. A terminal completion is the handoff receipt for the
/// retained hangup owner: response-driven cleanup can run before the sending
/// state-machine action unwinds, so removing that receipt here would make the
/// later waiter sleep until Timer F despite an already-recorded wire result.
fn outgoing_bye_cleanup_should_interrupt_waiter(
    current_outcome: rvoip_sip_dialog::transaction::TransactionResult<
        Option<ClientTransactionOutcome>,
    >,
    wait_intent_registered: bool,
) -> bool {
    matches!(current_outcome, Ok(None)) || !wait_intent_registered
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

/// Publish one retained-auth mutation while the caller owns this exact
/// session's state-machine lane.
///
/// The lane—not a snapshot revision—is the serialization authority for Digest
/// nonce counts and retained protection spaces. The generation-qualified
/// handle still rejects replacement lifetimes; callers must not invoke this
/// helper from a MESSAGE-lane-only or otherwise unowned context.
fn update_retained_auth_lane_owned_exact<R>(
    store: &SessionStore,
    handle: &SessionRegistryHandle,
    unavailable: &'static str,
    update: impl FnOnce(&mut crate::session_store::SessionState) -> Result<R>,
) -> Result<R> {
    store
        .update_session_exact_with(handle, None, update)
        .map_err(|_| SessionError::InvalidTransition(unavailable.to_string()))?
}

/// Capture a raw public session identifier once, wait on that exact cell's
/// signaling lane, and revalidate the same generation after the wait.
///
/// Executor actions must never call this helper: they already own the lane and
/// use the lane-owned dispatch variants below.
async fn lock_and_load_exact_current_session(
    store: &SessionStore,
    session_id: &SessionId,
) -> Option<(
    SessionRegistryHandle,
    tokio::sync::OwnedMutexGuard<()>,
    SessionState,
)> {
    let (handle, lane) = store.state_machine_lane(session_id)?;
    let guard = lane.lock_owned().await;
    let snapshot = store.get_session_snapshot_exact(&handle).ok()?;
    Some((handle, guard, snapshot.state().clone()))
}

fn resolve_dialog_for_handle_exact(
    store: &SessionStore,
    handle: &SessionRegistryHandle,
) -> Option<RvoipDialogId> {
    store
        .registry()
        .get_dialog_handle_exact(handle)
        .map(Into::into)
}

/// Resolve the canonical dialog while the caller owns this session's exact
/// state-machine lane. The state-local identity and registry mapping must name
/// the same dialog before any in-dialog wire operation is allowed.
fn resolve_dialog_for_lane_owned_session(
    store: &SessionStore,
    session: &SessionState,
) -> Result<RvoipDialogId> {
    let handle = session.lifecycle_handle.as_ref().ok_or_else(|| {
        SessionError::InvalidTransition(
            "SIP request exact session has no lifecycle authority".to_string(),
        )
    })?;
    if handle.session_id() != &session.session_id {
        return Err(SessionError::InvalidTransition(
            "SIP request lifecycle owner does not match its session".to_string(),
        ));
    }
    let dialog_id = resolve_dialog_for_handle_exact(store, handle).ok_or_else(|| {
        SessionError::InvalidTransition(
            "SIP request exact session has no current dialog".to_string(),
        )
    })?;
    let exact_dialog_id: crate::types::DialogId = dialog_id.clone().into();
    if session.dialog_id.as_ref() != Some(&exact_dialog_id) {
        return Err(SessionError::InvalidTransition(
            "SIP request exact dialog no longer owns its session".to_string(),
        ));
    }
    Ok(dialog_id)
}

/// Resolve one legacy Dialog-ID convenience call through the canonical exact
/// registry owner, then acquire and revalidate that owner's state-machine
/// lane. Every missing or stale owner is an error: compatibility entry points
/// must never report wire success when no exact dialog can be reached.
async fn lock_and_load_exact_legacy_dialog_session(
    store: &SessionStore,
    dialog_id: &RvoipDialogId,
) -> Result<(
    SessionRegistryHandle,
    tokio::sync::OwnedMutexGuard<()>,
    SessionState,
)> {
    let exact_dialog_id: crate::types::DialogId = dialog_id.clone().into();
    let handle = store
        .registry()
        .get_handle_by_dialog_exact(&exact_dialog_id)
        .ok_or_else(|| {
            SessionError::SessionNotFound(format!("No exact session owns dialog {}", dialog_id))
        })?;
    let lane = store.state_machine_lane_exact(&handle).ok_or_else(|| {
        SessionError::SessionNotFound(format!(
            "Exact session for dialog {} is no longer available",
            dialog_id
        ))
    })?;
    let guard = lane.lock_owned().await;
    let snapshot = store.get_session_snapshot_exact(&handle).map_err(|_| {
        SessionError::SessionNotFound(format!(
            "Exact session for dialog {} is no longer current",
            dialog_id
        ))
    })?;
    if store.registry().get_dialog_handle_exact(&handle) != Some(exact_dialog_id) {
        return Err(SessionError::InvalidTransition(format!(
            "Dialog {} no longer owns its exact session",
            dialog_id
        )));
    }
    Ok((handle, guard, snapshot.state().clone()))
}

/// Resolve and acquire the state-machine lane owned by one canonical exact
/// dialog mapping. The registry supplies the generation-qualified owner;
/// compatibility maps are deliberately not consulted. Revalidation after the
/// await prevents cleanup or raw-ID reuse from redirecting a queued attempt.
async fn lock_and_load_exact_data_message_session(
    store: &SessionStore,
    dialog_id: &RvoipDialogId,
) -> Result<(
    SessionRegistryHandle,
    tokio::sync::OwnedMutexGuard<()>,
    SessionState,
)> {
    let exact_dialog_id: crate::types::DialogId = dialog_id.clone().into();
    let handle = store
        .registry()
        .get_handle_by_dialog_exact(&exact_dialog_id)
        .ok_or_else(|| {
            SessionError::InvalidTransition(
                "SIP MESSAGE exact dialog has no owning session".to_string(),
            )
        })?;
    let lane = store.state_machine_lane_exact(&handle).ok_or_else(|| {
        SessionError::InvalidTransition(
            "SIP MESSAGE exact session is no longer available".to_string(),
        )
    })?;

    let guard = lane.lock_owned().await;
    let snapshot = store.get_session_snapshot_exact(&handle).map_err(|_| {
        SessionError::InvalidTransition(
            "SIP MESSAGE exact session is no longer available".to_string(),
        )
    })?;
    if store.registry().get_dialog_handle_exact(&handle).as_ref() != Some(&exact_dialog_id) {
        return Err(SessionError::InvalidTransition(
            "SIP MESSAGE exact dialog no longer owns its session".to_string(),
        ));
    }
    Ok((handle, guard, snapshot.state().clone()))
}

/// Select how the canonical in-dialog MESSAGE driver obtains session-state
/// authority.
///
/// Public/external callers capture an exact generation and let the driver
/// acquire that generation's lane for every wire attempt. State-machine
/// actions already own the lane, so they lend their working state directly
/// and must never try to reacquire it.
enum DataMessageStateLane<'a> {
    AcquireExact {
        expected_handle: Option<SessionRegistryHandle>,
    },
    AlreadyOwned(&'a mut SessionState),
}

enum DataMessageAttemptState<'a> {
    AcquiredExact {
        handle: SessionRegistryHandle,
        _guard: tokio::sync::OwnedMutexGuard<()>,
        session: Box<SessionState>,
    },
    AlreadyOwned {
        handle: SessionRegistryHandle,
        session: &'a mut SessionState,
    },
}

fn legacy_text_data_message(body: String) -> SipDataMessage {
    SipDataMessage {
        content_type: "text/plain".to_string(),
        bytes: bytes::Bytes::from(body),
        extra_headers: Vec::new(),
    }
}

#[derive(Clone, Copy)]
enum RetainedDialogAuthMode<'a> {
    Request,
    DataMessage {
        fresh_challenge: Option<&'a DataMessageAuthChallenge>,
    },
}

impl<'a> RetainedDialogAuthMode<'a> {
    fn fresh_challenge(self) -> Option<&'a DataMessageAuthChallenge> {
        match self {
            Self::Request => None,
            Self::DataMessage { fresh_challenge } => fresh_challenge,
        }
    }

    fn requires_active_dialog(self) -> bool {
        matches!(self, Self::DataMessage { .. })
    }

    fn dialog_ownership_error(self) -> &'static str {
        match self {
            Self::Request => "SIP request exact dialog no longer owns its session",
            Self::DataMessage { .. } => "SIP MESSAGE exact dialog no longer owns its session",
        }
    }

    fn missing_credentials_error(self) -> &'static str {
        match self {
            Self::Request => "SIP dialog retained a challenge without route credentials",
            Self::DataMessage { .. } => {
                "SIP MESSAGE dialog retained a challenge without route credentials"
            }
        }
    }

    fn challenge_mismatch_error(self) -> &'static str {
        match self {
            Self::Request => {
                "SIP request challenge no longer matches the exact dialog protection space"
            }
            Self::DataMessage { .. } => {
                "SIP MESSAGE challenge no longer matches the exact dialog protection space"
            }
        }
    }

    fn wire_validation_error(self) -> &'static str {
        match self {
            Self::Request => "SIP request authorization failed wire-safety validation",
            Self::DataMessage { .. } => "SIP MESSAGE authorization failed wire-safety validation",
        }
    }
}

struct RetainedDialogAuthRoute<'a> {
    next_hop: &'a str,
    transport: &'a crate::auth::SipTransportSecurityContext,
}

/// Re-author one exact dialog's retained origin/proxy protection spaces.
///
/// This is deliberately a pure working-state mutation: it performs no store,
/// lane, dialog, transaction, or transport I/O. Callers retain their current
/// ownership and lock ordering while sharing one standards-sensitive Digest
/// implementation. Credentials and nonce counts are copied first and are
/// published to `session` together only after every selected header passes
/// protection-space and wire-safety validation.
fn mutate_retained_dialog_auth(
    session: &mut SessionState,
    dialog_id: &RvoipDialogId,
    mode: RetainedDialogAuthMode<'_>,
    method: &str,
    request_uri: &str,
    route: RetainedDialogAuthRoute<'_>,
    body: Option<&[u8]>,
) -> Result<Vec<rvoip_sip_core::types::TypedHeader>> {
    use crate::session_store::state::{InviteAuthorizationCredential, InviteCredentialKind};
    let RetainedDialogAuthRoute {
        next_hop,
        transport,
    } = route;

    if session
        .dialog_id
        .as_ref()
        .is_none_or(|current| current.as_uuid() != &dialog_id.0)
    {
        return Err(SessionError::InvalidTransition(
            mode.dialog_ownership_error().to_string(),
        ));
    }
    if mode.requires_active_dialog()
        && (!session.dialog_established
            || session.call_state == crate::types::CallState::Terminating
            || session.call_state.is_final())
    {
        return Err(SessionError::InvalidTransition(
            "SIP MESSAGE exact dialog is no longer active".to_string(),
        ));
    }

    let fresh_challenge = mode.fresh_challenge();
    if session.invite_authorization_credentials.is_empty() && fresh_challenge.is_none() {
        return Ok(Vec::new());
    }

    let auth = session
        .auth
        .clone()
        .or_else(|| session.credentials.clone().map(Into::into))
        .ok_or_else(|| SessionError::AuthError(mode.missing_credentials_error().to_string()))?;
    let origin_target = session
        .remote_uri
        .clone()
        .unwrap_or_else(|| request_uri.to_string());
    let mut credentials = session.invite_authorization_credentials.clone();
    let mut digest_nc = session.digest_nc.clone();

    if let Some(fresh) = fresh_challenge {
        let kind = if fresh.status == 407 {
            InviteCredentialKind::Proxy
        } else {
            InviteCredentialKind::Origin
        };
        let preview = auth
            .authorization_for_challenge_with_transport_context(
                &fresh.value,
                method,
                request_uri,
                1,
                body,
                transport,
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
        let existing = credentials
            .iter()
            .position(|credential| credential.kind == kind && credential.realm == realm);
        if existing.is_none() && credentials.iter().any(|credential| credential.kind == kind) {
            return Err(SessionError::AuthError(
                "SIP MESSAGE challenge changed the exact dialog protection space".to_string(),
            ));
        }
        let (protection_target, stale_refreshes) = if let Some(index) = existing {
            let credential = &credentials[index];
            if !preview.stale || credential.nonce == nonce || credential.stale_refreshes >= 1 {
                return Err(SessionError::AuthError(
                    "SIP MESSAGE repeated a non-refreshing authentication challenge".to_string(),
                ));
            }
            (
                credential.protection_target.clone(),
                credential.stale_refreshes.saturating_add(1),
            )
        } else {
            if credentials.len() >= 8 {
                return Err(SessionError::AuthError(
                    "SIP MESSAGE authentication protection-space limit was reached".to_string(),
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
        let credential = InviteAuthorizationCredential {
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
            InviteCredentialKind::Origin => credential.protection_target == origin_target,
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
                transport,
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
                    mode.challenge_mismatch_error().to_string(),
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
                transport,
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
            InviteCredentialKind::Origin => rvoip_sip_core::types::HeaderName::Authorization,
            InviteCredentialKind::Proxy => rvoip_sip_core::types::HeaderName::ProxyAuthorization,
        };
        headers.push(
            rvoip_sip_core::validation::validated_authorization_header(name, selected.value)
                .map_err(|_| SessionError::AuthError(mode.wire_validation_error().to_string()))?,
        );
    }

    session.invite_authorization_credentials = credentials;
    session.digest_nc = digest_nc;
    Ok(headers)
}

/// Apply retained MESSAGE authentication while the caller owns the exact
/// state-machine lane. The unauthenticated path validates against the local
/// lane-owned snapshot without publishing; credential or fresh-challenge
/// bookkeeping is committed through the narrow exact-cell updater.
#[allow(clippy::too_many_arguments)]
fn authorize_data_message_lane_owned_exact(
    store: &SessionStore,
    handle: &SessionRegistryHandle,
    session: &mut SessionState,
    dialog_id: &RvoipDialogId,
    request: &mut rvoip_sip_core::Request,
    next_hop: &str,
    fresh_challenge: Option<&DataMessageAuthChallenge>,
    transport: &crate::auth::SipTransportSecurityContext,
) -> Result<()> {
    let request_uri = request.uri.to_string();
    let body = Some(request.body.as_ref());
    let authorize = |target: &mut SessionState| {
        mutate_retained_dialog_auth(
            target,
            dialog_id,
            RetainedDialogAuthMode::DataMessage { fresh_challenge },
            "MESSAGE",
            &request_uri,
            RetainedDialogAuthRoute {
                next_hop,
                transport,
            },
            body,
        )
    };
    let headers =
        if !session.invite_authorization_credentials.is_empty() || fresh_challenge.is_some() {
            update_retained_auth_lane_owned_exact(
                store,
                handle,
                "SIP MESSAGE exact session changed during authentication",
                authorize,
            )?
        } else {
            authorize(session)?
        };
    request.headers.extend(headers);
    Ok(())
}

/// Publish the exact lower-layer dialog identity before the initial INVITE is
/// allowed onto the wire.
///
/// A loopback peer can answer synchronously during dispatch.  The response
/// handler must therefore never be able to clone a session revision whose
/// exact registry mapping is installed but whose state-local `dialog_id` is
/// still empty.  Publishing here closes that gap; the ordinary state-machine
/// action later observes and republishes the same identity.
fn publish_initial_invite_dialog_exact(
    store: &SessionStore,
    handle: &SessionRegistryHandle,
    dialog_id: &RvoipDialogId,
) -> Result<()> {
    let session_dialog_id: crate::types::DialogId = dialog_id.clone().into();
    let registry_dialog_id = store.registry().get_dialog_handle_exact(handle);
    match store.update_session_exact_with(handle, None, |session| -> Result<()> {
        match (session.dialog_id.as_ref(), registry_dialog_id.as_ref()) {
            (Some(current), _) if current == &session_dialog_id => return Ok(()),
            // Redirect cleanup retires the old registry owner inside the same
            // executor transition before this replacement INVITE is
            // dispatched. The store still contains the transition's
            // pre-commit dialog snapshot, so replace it only when the exact
            // registry already proves the new owner.
            (Some(_), Some(registry)) if registry == &session_dialog_id => {}
            (Some(_), _) => {
                return Err(SessionError::InvalidTransition(
                    "initial INVITE exact dialog changed before wire dispatch".to_string(),
                ));
            }
            (None, _) => {}
        }
        session.dialog_id = Some(session_dialog_id);
        Ok(())
    }) {
        Ok(result) => result,
        Err(_) => Err(SessionError::InternalError(
            "initial INVITE exact dialog publication failed (class=lifecycle)".to_string(),
        )),
    }
}

/// Resolve the dialog owned by the current exact session lifetime.
///
/// INVITE retries are responses to an already-installed initial transaction,
/// so the registry mapping is authoritative and must already exist. Capturing
/// the generation-qualified handle before the synchronous exact lookup makes
/// raw-ID reuse unable to redirect a retry to another call.
fn resolve_exact_invite_retry_dialog(
    store: &SessionStore,
    session_id: &SessionId,
) -> Result<RvoipDialogId> {
    let handle = store
        .lifecycle_handle(session_id)
        .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?;
    resolve_dialog_for_handle_exact(store, &handle)
        .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))
}

#[derive(Clone)]
struct OutboundInitialInviteBinding {
    handle: SessionRegistryHandle,
    owner: InitialInviteOwner,
    resource: Weak<OutboundInitialInviteResource>,
}

#[derive(Clone)]
struct OutboundInviteTransaction {
    handle: SessionRegistryHandle,
    dialog_id: RvoipDialogId,
    transaction_id: TransactionKey,
}

impl OutboundInviteTransaction {
    fn matches(&self, handle: &SessionRegistryHandle, dialog_id: &RvoipDialogId) -> bool {
        self.handle == *handle && self.dialog_id == *dialog_id
    }
}

impl OutboundInitialInviteBinding {
    fn matches(&self, handle: &SessionRegistryHandle, owner: &InitialInviteOwner) -> bool {
        self.handle == *handle && self.owner == *owner
    }
}

/// Exact lower-layer ownership retained by the session lifecycle authority.
///
/// The remaining raw-ID transaction and resource trackers carry an exact
/// handle beside every value, fencing every mutation and rollback. The weak
/// self-reference lets explicit CANCEL/BYE paths mark protocol teardown
/// without creating a resource/map reference cycle.
struct OutboundInitialInviteResource {
    dialog_api: Arc<UnifiedDialogApi>,
    store: Arc<SessionStore>,
    registry: Arc<SessionRegistry>,
    handle: SessionRegistryHandle,
    owner: InitialInviteOwner,
    bindings: Arc<DashMap<SessionId, OutboundInitialInviteBinding>>,
    outgoing_invite_tx: Arc<DashMap<SessionId, OutboundInviteTransaction>>,
    phase: AtomicU8,
    protocol_teardown_owned_by_upper: AtomicBool,
    state_dialog_published: AtomicBool,
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

/// Dialog-owned REGISTER mechanics returned to the state-machine lane after
/// one wire attempt.
///
/// This is not a second stored projection. It is an input/output value for a
/// single dialog operation: the action captures it from its lane-owned
/// `SessionState`, dialog code advances Call-ID/CSeq, Digest nonce-count, and
/// the response transport context, and the action applies it back to that same
/// working state before the executor's canonical commit.
#[derive(Clone)]
pub(crate) struct RegisterAttemptContext {
    registration_call_id: Option<String>,
    registration_cseq: u32,
    auth_challenge_raw: Option<String>,
    auth_challenge: Option<crate::auth::DigestChallenge>,
    pending_auth_transport: Option<crate::auth::SipTransportSecurityContext>,
    pending_auth_status: Option<u16>,
    digest_nc: std::collections::HashMap<(String, String), u32>,
}

impl RegisterAttemptContext {
    pub(crate) fn capture(session: &SessionState) -> Self {
        Self {
            registration_call_id: session.registration_call_id.clone(),
            registration_cseq: session.registration_cseq,
            auth_challenge_raw: session.auth_challenge_raw.clone(),
            auth_challenge: session.auth_challenge.clone(),
            pending_auth_transport: session.pending_auth_transport.clone(),
            pending_auth_status: session.pending_auth.as_ref().map(|(status, _)| *status),
            digest_nc: session.digest_nc.clone(),
        }
    }

    pub(crate) fn apply(self, session: &mut SessionState) {
        session.registration_call_id = self.registration_call_id;
        session.registration_cseq = self.registration_cseq;
        session.pending_auth_transport = self.pending_auth_transport;
        session.digest_nc = self.digest_nc;
    }
}

/// Result of one REGISTER wire attempt, including the dialog-owned mechanics
/// that the caller must fold into its lane-owned working state.
pub(crate) struct RegisterAttemptResult {
    pub(crate) outcome: RegisterAttemptOutcome,
    pub(crate) context: RegisterAttemptContext,
    /// Exact request snapshot written for this attempt. The state-machine
    /// retains it across 401/407 and 423 so every retry preserves application
    /// headers, routing, registration identity, and refresh intent.
    pub(crate) request_options: RegisterRequestOptions,
}

/// Non-state side effects admitted only after the executor commits the
/// registration outcome. Observer callbacks and refresh timers therefore
/// cannot observe the pre-outcome session snapshot.
#[derive(Debug, Clone)]
pub(crate) enum RegistrationPostCommitEffect {
    Registered {
        registrar_uri: String,
        from_uri: String,
        contact_uri: String,
        accepted_expires: u32,
        next_refresh_at: Option<Instant>,
        transport_route: Option<rvoip_sip_transport::TransportRoute>,
    },
    Unregistered {
        registrar_uri: String,
    },
    RegistrationFailed {
        registrar_uri: String,
        status_code: u16,
        failure_summary: String,
    },
    UnregistrationFailed {
        registrar_uri: String,
        reason: String,
    },
}

struct RegistrationSuccessStateInput<'a> {
    registrar_uri: &'a str,
    from_uri: &'a str,
    contact_uri: &'a str,
    accepted_expires: u32,
    now: Instant,
    next_refresh_at: Option<Instant>,
    metadata: RegistrationResponseMetadata,
}

fn record_registration_success_state(
    session: &mut SessionState,
    input: RegistrationSuccessStateInput<'_>,
) -> RegistrationPostCommitEffect {
    let RegistrationSuccessStateInput {
        registrar_uri,
        from_uri,
        contact_uri,
        accepted_expires,
        now,
        next_refresh_at,
        metadata,
    } = input;
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

    RegistrationPostCommitEffect::Registered {
        registrar_uri: registrar_uri.to_string(),
        from_uri: from_uri.to_string(),
        contact_uri: contact_uri.to_string(),
        accepted_expires,
        next_refresh_at,
        transport_route: metadata.transport_route,
    }
}

fn record_unregistration_success_state(
    session: &mut SessionState,
    registrar_uri: &str,
) -> RegistrationPostCommitEffect {
    session.is_registered = false;
    session.registration_accepted_expires = None;
    session.registration_registered_at = None;
    session.registration_next_refresh_at = None;
    session.registration_last_failure = None;
    session.registration_retry_count = 0;
    session.registration_service_route = None;
    session.registration_pub_gruu = None;
    session.registration_temp_gruu = None;

    RegistrationPostCommitEffect::Unregistered {
        registrar_uri: registrar_uri.to_string(),
    }
}

fn record_registration_failure_state(
    session: &mut SessionState,
    registrar_uri: &str,
    status_code: u16,
    reason: String,
) -> RegistrationPostCommitEffect {
    let failure_summary = if session.registration_retry_count > 0 {
        format!(
            "{} after {} retry attempt(s)",
            reason, session.registration_retry_count
        )
    } else {
        reason
    };
    session.is_registered = false;
    session.registration_accepted_expires = None;
    session.registration_registered_at = None;
    session.registration_next_refresh_at = None;
    session.registration_last_failure = Some(failure_summary.clone());
    session.registration_service_route = None;
    session.registration_pub_gruu = None;
    session.registration_temp_gruu = None;

    RegistrationPostCommitEffect::RegistrationFailed {
        registrar_uri: registrar_uri.to_string(),
        status_code,
        failure_summary,
    }
}

fn record_unregistration_failure_state(
    session: &mut SessionState,
    registrar_uri: &str,
    reason: String,
) -> RegistrationPostCommitEffect {
    session.is_registered = false;
    session.registration_accepted_expires = None;
    session.registration_registered_at = None;
    session.registration_next_refresh_at = None;
    session.registration_last_failure = Some(reason.clone());
    session.registration_retry_count = 0;
    session.registration_service_route = None;
    session.registration_pub_gruu = None;
    session.registration_temp_gruu = None;

    RegistrationPostCommitEffect::UnregistrationFailed {
        registrar_uri: registrar_uri.to_string(),
        reason,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InviteDispatchFailure {
    Initial,
    InitialWithExtraHeaders,
    InitialWithOptions,
    AuthRetry,
    SessionTimerRetry,
    ReinviteWithOptions,
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
            Self::ReinviteWithOptions => {
                "Failed to send re-INVITE with options (class=invite-dispatch)"
            }
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
            store: Arc::clone(&adapter.store),
            registry: Arc::clone(adapter.store.registry()),
            handle,
            owner,
            bindings: Arc::clone(&adapter.outbound_initial_invites),
            outgoing_invite_tx: Arc::clone(&adapter.outgoing_invite_tx),
            phase: AtomicU8::new(OWNED_INVITE_INSTALLED),
            protocol_teardown_owned_by_upper: AtomicBool::new(false),
            state_dialog_published: AtomicBool::new(false),
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

        self.registry
            .install_dialog_identity_handle(
                &self.handle,
                self.owner.dialog_id().clone().into(),
                self.owner.call_id().to_string(),
            )
            .map_err(|_| "exact session registry dialog mapping failed")?;
        self.registry_map_installed.store(true, Ordering::Release);
        Ok(())
    }

    fn record_state_dialog_published(&self) {
        self.state_dialog_published.store(true, Ordering::Release);
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
                entry.insert(OutboundInviteTransaction {
                    handle: self.handle.clone(),
                    dialog_id: self.owner.dialog_id().clone(),
                    transaction_id,
                });
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
        store: Arc<SessionStore>,
        registry: Arc<SessionRegistry>,
        handle: SessionRegistryHandle,
        owner: InitialInviteOwner,
        bindings: Arc<DashMap<SessionId, OutboundInitialInviteBinding>>,
        outgoing_invite_tx: Arc<DashMap<SessionId, OutboundInviteTransaction>>,
        phase: u8,
        protocol_teardown_owned_by_upper: bool,
        state_dialog_published: bool,
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

        if state_dialog_published
            && store
                .clear_dialog_session_retained_exact(&handle, &owner.dialog_id().clone().into())
                .is_err()
        {
            return Err(ManagedResourceReleaseError::new(
                "invite-session-dialog-release-failed",
            ));
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

        if transaction_map_installed {
            outgoing_invite_tx.remove_if(&session_id, |_, transaction| {
                transaction.matches(&handle, owner.dialog_id())
            });
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
        let store = Arc::clone(&self.store);
        let registry = Arc::clone(&self.registry);
        let handle = self.handle.clone();
        let owner = self.owner.clone();
        let bindings = Arc::clone(&self.bindings);
        let outgoing_invite_tx = Arc::clone(&self.outgoing_invite_tx);
        let phase = self.phase.load(Ordering::Acquire);
        let protocol_teardown_owned_by_upper = self
            .protocol_teardown_owned_by_upper
            .load(Ordering::Acquire);
        let state_dialog_published = self.state_dialog_published.load(Ordering::Acquire);
        let registry_map_installed = self.registry_map_installed.load(Ordering::Acquire);
        let transaction_map_installed = self.transaction_map_installed.load(Ordering::Acquire);
        Box::pin(Self::release_exact(
            dialog_api,
            store,
            registry,
            handle,
            owner,
            bindings,
            outgoing_invite_tx,
            phase,
            protocol_teardown_owned_by_upper,
            state_dialog_published,
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

    /// Generation-qualified outgoing INVITE transaction ownership retained for
    /// automatic ACK correlation and exact response cleanup.
    outgoing_invite_tx: Arc<DashMap<SessionId, OutboundInviteTransaction>>,

    /// Latest in-dialog BYE transaction for each exact live session. The
    /// generation lets a 401/407-driven retry supersede the challenged
    /// transaction without a late initial dispatch overwriting it.
    outgoing_bye_tx: Arc<DashMap<SessionRegistryHandle, OutboundByeTransaction>>,
    /// Exact per-session generation notification. Authentication retries wake
    /// the owning BYE waiter directly instead of requiring 10 ms polling.
    outgoing_bye_generation_watch:
        Arc<DashMap<SessionRegistryHandle, tokio::sync::watch::Sender<u64>>>,
    /// Number of cancellation owners spanning dispatch through confirmation.
    /// A terminal receipt is preserved across response-driven cleanup only
    /// while this exact session has at least one such owner.
    outgoing_bye_wait_intents: Arc<DashMap<SessionRegistryHandle, OutgoingByeWaitIntentState>>,
    next_outgoing_bye_generation: Arc<AtomicU64>,
    /// Timer F / configured non-INVITE transaction horizon used by the
    /// retained local-BYE cleanup owner.
    non_invite_transaction_timeout: Duration,

    /// Exact in-dialog request ownership for methods whose builder futures
    /// return after first transport write while authentication/final response
    /// arrives asynchronously.
    pub(crate) outbound_request_tracker: OutboundInDialogRequestTracker,

    /// Exact owner for each staged outbound initial INVITE.
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
    #[cfg(test)]
    registration_refresh_dispatch_pause:
        Arc<StdMutex<Option<Arc<RegistrationRefreshDispatchPause>>>>,

    /// Perf diagnostics for dialog mapping cleanup balance.
    cleanup_attempt_total: Arc<AtomicU64>,
    cleanup_mapped_total: Arc<AtomicU64>,
    cleanup_missing_total: Arc<AtomicU64>,
    cleanup_outgoing_invite_removed_total: Arc<AtomicU64>,

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
            outgoing_invite_tx: Arc::new(DashMap::new()),
            outgoing_bye_tx: Arc::new(DashMap::new()),
            outgoing_bye_generation_watch: Arc::new(DashMap::new()),
            outgoing_bye_wait_intents: Arc::new(DashMap::new()),
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
            #[cfg(test)]
            registration_refresh_dispatch_pause: Arc::new(StdMutex::new(None)),
            cleanup_attempt_total: Arc::new(AtomicU64::new(0)),
            cleanup_mapped_total: Arc::new(AtomicU64::new(0)),
            cleanup_missing_total: Arc::new(AtomicU64::new(0)),
            cleanup_outgoing_invite_removed_total: Arc::new(AtomicU64::new(0)),
            trace_redactor,
        }
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

    /// Route a retained adapter compatibility facade through the same atomic
    /// staged-options executor used by crate-owned builders. The adapter never
    /// invokes a BYE or CANCEL wire method directly from a raw public ID.
    async fn dispatch_state_machine_options_exact(
        &self,
        handle: &SessionRegistryHandle,
        event: EventType,
        slot: crate::state_machine::executor::PendingOptionsSlot,
    ) -> Result<()> {
        let state_machine = self.state_machine.get().cloned().ok_or_else(|| {
            SessionError::InvalidTransition(
                "SIP signaling state-machine authority is not installed".to_string(),
            )
        })?;
        self.store
            .get_session_snapshot_exact(handle)
            .map_err(|_| SessionError::SessionNotFound(handle.session_id().to_string()))?;
        let claim = Arc::new(crate::state_machine::executor::StageDispatchClaim::new(
            slot.clone(),
        ));
        state_machine
            .process_event_with_staged_options_exact(handle, event, slot, claim, None)
            .await
            .map(|_| ())
            .map_err(|error| SessionError::InternalError(error.to_string()))
    }

    /// Admit a sanitized application observation without placing observer
    /// completion or backpressure on the signaling caller.
    pub(crate) fn publish_api_event(&self, api_event: crate::api::events::Event) {
        if self
            .state_machine
            .get()
            .is_some_and(|state_machine| state_machine.publish_api_event(api_event.clone()))
        {
            return;
        }
        let _observation =
            spawn_api_event_observation(Arc::clone(&self.global_coordinator), api_event);
    }

    /// Publish a committed application event through the installed private
    /// exact-control path. Isolated adapters without that publisher emit only
    /// a sanitized public observation and never place authority on the bus.
    pub(crate) fn publish_api_event_exact(
        &self,
        lifecycle_handle: &SessionRegistryHandle,
        api_event: crate::api::events::Event,
    ) {
        if self.state_machine.get().is_some_and(|state_machine| {
            state_machine.publish_api_event_exact(lifecycle_handle, api_event.clone())
        }) {
            return;
        }
        let _observation =
            spawn_api_event_observation(Arc::clone(&self.global_coordinator), api_event);
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

    pub(crate) fn abort_registration_refresh_exact(&self, handle: &SessionRegistryHandle) {
        let session_id = handle.session_id();
        let guard = cleanup_diag::stage_guard(CleanupStage::TimerTaskShutdown, &session_id.0);
        let _admission = self
            .registration_refresh_admission
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((_, task)) = self
            .registration_refresh_tasks
            .remove_if(session_id, |_, current| current.handle == *handle)
        {
            let _ = task.cancel.send(());
        }
        guard.finish_success();
    }

    /// Feature-gated retained-object counts for perf leak investigations.
    #[cfg(any(test, feature = "perf-tests"))]
    pub(crate) fn perf_diagnostic_counts(&self) -> serde_json::Value {
        serde_json::json!({
            "outgoing_invite_tx": self.outgoing_invite_tx.len(),
            "outgoing_bye_tx": self.outgoing_bye_tx.len(),
            "outgoing_bye_generation_watch": self.outgoing_bye_generation_watch.len(),
            "outgoing_bye_wait_intents": self.outgoing_bye_wait_intents.len(),
            "outbound_initial_invites": self.outbound_initial_invites.len(),
            "outbound_request_tracker": {
                "live_requests": self.outbound_request_tracker.live_request_count(),
                "deferred_events": self.outbound_request_tracker.deferred_event_count(),
            },
            "registration_refresh_tasks": self.registration_refresh_tasks.len(),
            "registration_refresh_retained_tasks": self.registration_refresh_retained.count(),
            "lifecycle": {
                "cleanup_attempt_total": self.cleanup_attempt_total.load(Ordering::Relaxed),
                "cleanup_mapped_total": self.cleanup_mapped_total.load(Ordering::Relaxed),
                "cleanup_missing_total": self.cleanup_missing_total.load(Ordering::Relaxed),
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
        handle: SessionRegistryHandle,
        next_refresh_at: Option<Instant>,
    ) {
        let session_id = handle.session_id().clone();
        let state_machine = self.state_machine.get().cloned();
        let _admission = self
            .registration_refresh_admission
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        // Post-commit effects run outside the exact state-machine lane. A
        // delayed generation-A effect must not cancel or publish over a
        // generation-B timer that now owns the same raw SessionId.
        if self.store.lifecycle_handle(&session_id).as_ref() != Some(&handle) {
            tracing::debug!(
                session_id = %session_id,
                "ignored stale automatic registration refresh schedule"
            );
            return;
        }

        // Every map removal compares the complete task identity observed
        // under the admission gate. A current generation may reclaim a stale
        // predecessor, but an old callback can never remove its successor.
        let previous_identity = self
            .registration_refresh_tasks
            .get(&session_id)
            .map(|current| (current.handle.clone(), current.generation));
        if let Some((previous_handle, previous_generation)) = previous_identity {
            if let Some((_, previous)) =
                self.registration_refresh_tasks
                    .remove_if(&session_id, |_, current| {
                        current.handle == previous_handle
                            && current.generation == previous_generation
                    })
            {
                let _ = previous.cancel.send(());
            }
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
        if self.store.lifecycle_handle(&session_id).as_ref() != Some(&handle) {
            return;
        }

        let generation = self
            .next_registration_refresh_generation
            .fetch_add(1, Ordering::Relaxed);
        let (cancel, mut cancelled) = tokio::sync::oneshot::channel();
        let session_id_for_task = session_id.clone();
        let handle_for_task = handle.clone();
        let completion_handle = handle.clone();
        let completion_admission = Arc::clone(&self.registration_refresh_admission);
        let completion_tasks = Arc::clone(&self.registration_refresh_tasks);
        #[cfg(test)]
        let dispatch_pause = Arc::clone(&self.registration_refresh_dispatch_pause);
        let spawned = self.registration_refresh_retained.spawn(async move {
            // Construct inside the spawned future. If admission rejects and
            // drops the unpolled future while the caller owns the gate, there
            // is no completion destructor trying to reacquire that gate.
            let _completion = RegistrationRefreshCompletion {
                admission: completion_admission,
                tasks: completion_tasks,
                handle: completion_handle,
                generation,
            };
            let stage_claim = Arc::new(
                crate::state_machine::executor::StageDispatchClaim::new_deferred(
                    Method::Register,
                    crate::state_machine::executor::PendingOptionsSlotKind::Register,
                ),
            );
            let refresh_claim = Arc::clone(&stage_claim);
            let refresh = async move {
                tokio::time::sleep_until(tokio::time::Instant::from_std(next_refresh_at)).await;

                #[cfg(test)]
                let pause = {
                    dispatch_pause
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .clone()
                };
                #[cfg(test)]
                if let Some(pause) = pause {
                    pause.pause().await;
                }

                match state_machine
                    .process_registration_refresh_exact(
                        &handle_for_task,
                        crate::state_table::types::EventType::RefreshRegistration,
                        None,
                        Vec::new(),
                        refresh_claim,
                    )
                    .await
                {
                    Ok(result) if result.transition.is_some() => {}
                    Ok(_) => {
                        tracing::warn!(
                            session_id = %session_id_for_task,
                            generation,
                            "automatic registration refresh has no state-table transition; no REGISTER dispatched"
                        );
                    }
                    Err(e) => {
                        tracing::debug!(
                            session_id = %session_id_for_task,
                            generation,
                            error = %e,
                            "exact automatic registration refresh was cancelled or stale"
                        );
                    }
                }
            };
            tokio::pin!(refresh);
            tokio::select! {
                biased;
                _ = &mut cancelled => {
                    if stage_claim.cancel_before_claim() {
                        return;
                    }
                    refresh.await;
                },
                _ = &mut refresh => {}
            }
        });
        if spawned {
            let replaced = self.registration_refresh_tasks.insert(
                session_id,
                RegistrationRefreshTask {
                    handle,
                    generation,
                    cancel,
                },
            );
            debug_assert!(replaced.is_none());
        }
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

    pub(crate) fn register_response_transport_context(
        &self,
        response: &Response,
    ) -> Option<crate::auth::SipTransportSecurityContext> {
        self.dialog_api
            .outbound_transport_context_for_response(response)
            .map(|context| {
                crate::auth::SipTransportSecurityContext::from_transport_context(&context)
            })
    }

    /// Apply a successful REGISTER outcome to the executor's lane-owned
    /// working state and return the non-state work that must run after commit.
    pub(crate) fn record_registration_success(
        &self,
        session: &mut SessionState,
        registrar_uri: &str,
        from_uri: &str,
        contact_uri: &str,
        accepted_expires: u32,
        metadata: RegistrationResponseMetadata,
    ) -> RegistrationPostCommitEffect {
        let now = Instant::now();
        let next_refresh_at = if self.registration_auto_refresh && accepted_expires > 0 {
            Some(self.compute_registration_refresh_at(now, accepted_expires))
        } else {
            None
        };

        record_registration_success_state(
            session,
            RegistrationSuccessStateInput {
                registrar_uri,
                from_uri,
                contact_uri,
                accepted_expires,
                now,
                next_refresh_at,
                metadata,
            },
        )
    }

    pub(crate) fn record_unregistration_success(
        &self,
        session: &mut SessionState,
        registrar_uri: &str,
    ) -> RegistrationPostCommitEffect {
        record_unregistration_success_state(session, registrar_uri)
    }

    pub(crate) fn record_registration_failure(
        &self,
        session: &mut SessionState,
        registrar_uri: &str,
        status_code: u16,
        reason: impl Into<String>,
    ) -> RegistrationPostCommitEffect {
        record_registration_failure_state(session, registrar_uri, status_code, reason.into())
    }

    pub(crate) fn record_unregistration_failure(
        &self,
        session: &mut SessionState,
        registrar_uri: &str,
        reason: impl Into<String>,
    ) -> RegistrationPostCommitEffect {
        record_unregistration_failure_state(session, registrar_uri, reason.into())
    }

    /// Run observer/timer/flow effects after the canonical state commit.
    pub(crate) fn complete_registration_post_commit(
        &self,
        handle: &SessionRegistryHandle,
        effect: RegistrationPostCommitEffect,
    ) {
        let session_id = handle.session_id();
        match effect {
            RegistrationPostCommitEffect::Registered {
                registrar_uri,
                from_uri,
                contact_uri,
                accepted_expires,
                next_refresh_at,
                transport_route,
            } => {
                tracing::info!(
                    "✅ Registration successful - session {} marked as registered",
                    session_id.0
                );
                self.publish_api_event(crate::api::events::Event::RegistrationSuccess {
                    registrar: registrar_uri,
                    expires: accepted_expires,
                    contact: contact_uri,
                });
                self.schedule_registration_refresh(handle.clone(), next_refresh_at);
                self.start_symmetric_registration_keepalive(&from_uri, transport_route);
            }
            RegistrationPostCommitEffect::Unregistered { registrar_uri } => {
                self.abort_registration_refresh_exact(handle);
                tracing::info!(
                    "✅ Unregistration successful - session {} marked as unregistered",
                    session_id.0
                );
                self.publish_api_event(crate::api::events::Event::UnregistrationSuccess {
                    registrar: registrar_uri,
                });
            }
            RegistrationPostCommitEffect::RegistrationFailed {
                registrar_uri,
                status_code,
                failure_summary,
            } => {
                self.abort_registration_refresh_exact(handle);
                self.publish_api_event(crate::api::events::Event::RegistrationFailed {
                    registrar: registrar_uri,
                    status_code,
                    reason: failure_summary,
                });
            }
            RegistrationPostCommitEffect::UnregistrationFailed {
                registrar_uri,
                reason,
            } => {
                self.abort_registration_refresh_exact(handle);
                self.publish_api_event(crate::api::events::Event::UnregistrationFailed {
                    registrar: registrar_uri,
                    reason,
                });
            }
        }
    }

    // ===== Direct Dialog Operations =====
    // NOTE: Removed confusing create_dialog() and send_invite() methods
    // Use send_invite_with_details() to create a dialog and send INVITE in one operation

    /// Send a response
    pub async fn send_response_by_dialog(
        &self,
        _dialog_id: DialogId,
        _status_code: u16,
        _reason: &str,
    ) -> Result<()> {
        Err(SessionError::InvalidTransition(
            "dialog-scoped response dispatch has no exact inbound transaction authority"
                .to_string(),
        ))
    }

    /// Compatibility facade for a dialog-addressed BYE. The exact registry
    /// owner is captured once, then the request is staged and dispatched by
    /// the YAML executor before this method joins the transaction result.
    pub async fn send_bye(&self, dialog_id: crate::types::DialogId) -> Result<()> {
        let rvoip_dialog_id: RvoipDialogId = dialog_id.into();
        let exact_dialog_id: crate::types::DialogId = rvoip_dialog_id.clone().into();
        let handle = self
            .store
            .registry()
            .get_handle_by_dialog_exact(&exact_dialog_id)
            .ok_or_else(|| {
                SessionError::SessionNotFound(format!(
                    "No exact session owns dialog {}",
                    rvoip_dialog_id
                ))
            })?;
        let _wait_owner = self.begin_outgoing_bye_wait_exact(&handle)?;
        self.dispatch_state_machine_options_exact(
            &handle,
            EventType::SendOutboundBye,
            crate::state_machine::executor::PendingOptionsSlot::Bye(Arc::new(
                ByeRequestOptions::default(),
            )),
        )
        .await?;
        self.wait_for_outgoing_bye_final_response_exact(&handle)
            .await
    }

    /// Send re-INVITE with new SDP
    pub async fn send_reinvite(
        &self,
        dialog_id: crate::types::DialogId,
        sdp: String,
    ) -> Result<()> {
        let rvoip_dialog_id: RvoipDialogId = dialog_id.into();
        let (_handle, _exact_lane, session) =
            lock_and_load_exact_legacy_dialog_session(self.store.as_ref(), &rvoip_dialog_id)
                .await?;

        let session_id = session.session_id.clone();
        self.send_reinvite_with_options_lane_owned(
            &session,
            rvoip_sip_dialog::api::unified::ReInviteRequestOptions {
                sdp: Some(sdp),
                ..Default::default()
            },
        )
        .await?;

        tracing::info!("Sent re-INVITE for session {}", session_id.0);

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
        let (_handle, _exact_lane, session) =
            lock_and_load_exact_legacy_dialog_session(self.store.as_ref(), &rvoip_dialog_id)
                .await?;

        let session_id = session.session_id.clone();
        // Attended-transfer Replaces belongs in `ReferRequestOptions.replaces`
        // (an RFC 3891 header param on Refer-To), not as a REFER body. The
        // legacy boolean never had an on-wire effect.
        let _ = attended;
        self.send_refer_with_options_lane_owned(
            &session,
            ReferRequestOptions {
                refer_to: target.to_string(),
                ..Default::default()
            },
        )
        .await?;

        tracing::info!(
            session = %session_id.0,
            target_present = !target.is_empty(),
            target_bytes = target.len(),
            "Sent REFER"
        );

        Ok(())
    }

    /// Get remote URI for a dialog
    pub async fn get_remote_uri(&self, dialog_id: crate::types::DialogId) -> Result<String> {
        let rvoip_dialog_id: RvoipDialogId = dialog_id.into();
        self.dialog_api
            .get_dialog_info(&rvoip_dialog_id)
            .await
            .map(|dialog| dialog.remote_uri.to_string())
            .map_err(|_| {
                SessionError::DialogError(
                    "Failed to resolve remote URI for exact dialog (class=dialog)".to_string(),
                )
            })
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
        let dialog_id = resolve_exact_invite_retry_dialog(self.store.as_ref(), session_id)?;

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
        let dialog_id = resolve_exact_invite_retry_dialog(self.store.as_ref(), session_id)?;

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

    /// Read the dialog installed by a completed initial-INVITE dispatch while
    /// the executor retains this exact session lane. The owned operation has
    /// already published the same identity into SessionStore before wire; the
    /// executor's working copy may still contain `None` and adopts it at its
    /// canonical event commit.
    pub(crate) fn initial_invite_dialog_lane_owned(
        &self,
        session: &SessionState,
    ) -> Result<crate::types::DialogId> {
        let handle = session.lifecycle_handle.as_ref().ok_or_else(|| {
            SessionError::InvalidTransition(
                "initial INVITE exact session has no lifecycle authority".to_string(),
            )
        })?;
        if handle.session_id() != &session.session_id {
            return Err(SessionError::InvalidTransition(
                "initial INVITE lifecycle owner does not match its session".to_string(),
            ));
        }
        let dialog_id = self
            .store
            .registry()
            .get_dialog_handle_exact(handle)
            .ok_or_else(|| {
                SessionError::InternalError(
                    "initial INVITE committed without an exact dialog mapping".to_string(),
                )
            })?;
        if session
            .dialog_id
            .as_ref()
            .is_some_and(|current| current != &dialog_id)
        {
            return Err(SessionError::InvalidTransition(
                "initial INVITE exact dialog changed before executor commit".to_string(),
            ));
        }
        Ok(dialog_id)
    }

    /// Does the remote peer support RFC 3262 100rel? Used to gate
    /// `send_early_media` — we only emit a reliable 183 when the caller
    /// advertised `Supported: 100rel` (or `Require: 100rel`) on the INVITE.
    /// Returns `SessionNotFound` if the session has no dialog yet.
    pub async fn peer_supports_100rel(&self, session_id: &SessionId) -> Result<bool> {
        let handle = self
            .store
            .lifecycle_handle(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?;
        let dialog_id = resolve_dialog_for_handle_exact(self.store.as_ref(), &handle)
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

        if resolve_dialog_for_handle_exact(self.store.as_ref(), &handle).as_ref()
            != Some(&dialog_id)
        {
            return Err(SessionError::SessionNotFound(session_id.0.clone()));
        }

        Ok(dialog.peer_supports_100rel)
    }

    // ===== Outbound Actions (from state machine) =====

    /// Record that the accepted upper-layer transition owns BYE teardown.
    ///
    /// Call this immediately before awaiting BYE dispatch. A loopback
    /// peer can return a final response during that await and drive exact
    /// session release before the send future unwinds. Marking afterward loses
    /// the retained initial-INVITE owner and can make shutdown supervise a
    /// teardown that the upper layer already sent. CANCEL remains owned by the
    /// dialog lifecycle because its transaction layer can distinguish a
    /// proven zero-wire failure from a write-started ambiguous failure.
    pub(crate) fn mark_initial_invite_protocol_teardown_exact(
        &self,
        handle: &SessionRegistryHandle,
    ) {
        let session_id = handle.session_id();
        let Some(binding) = self.outbound_initial_invites.get(session_id) else {
            return;
        };
        if binding.handle != *handle {
            return;
        }
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
        let store = Arc::clone(&self.store);
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

                    // Installation publishes the exact registry mapping.
                    // Mirror that identity into SessionState before
                    // dispatch: a synchronous 200 OK may otherwise publish an
                    // Active snapshot cloned from the pre-install revision and
                    // make an immediate BYE fail its ownership check.
                    if let Err(error) = publish_initial_invite_dialog_exact(
                        store.as_ref(),
                        &operation_resource.handle,
                        operation_resource.owner.dialog_id(),
                    ) {
                        drop(installed);
                        return rollback_owned_invite(operation, Err(error)).await;
                    }
                    operation_resource.record_state_dialog_published();

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
            registered_flow_routes: Vec::new(),
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

    /// Retained compatibility signature. Session-scoped redirect dispatch
    /// fails closed because it has no exact inbound transaction capability.
    pub async fn send_redirect_response_with_options(
        &self,
        _session_id: &SessionId,
        _status: u16,
        _contacts: Vec<String>,
        _extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> Result<()> {
        Err(SessionError::InvalidTransition(
            "session-scoped redirect dispatch has no exact inbound transaction authority"
                .to_string(),
        ))
    }

    /// Retained compatibility signature for a session-scoped redirect.
    pub async fn send_redirect_response(
        &self,
        _session_id: &SessionId,
        _status: u16,
        _contacts: Vec<String>,
    ) -> Result<()> {
        Err(SessionError::InvalidTransition(
            "session-scoped redirect dispatch has no exact inbound transaction authority"
                .to_string(),
        ))
    }

    /// Retained compatibility signature for a session-scoped UAS response
    /// with application headers. Exact transaction methods remain supported.
    pub async fn send_response_with_options(
        &self,
        _session_id: &SessionId,
        _code: u16,
        _sdp: Option<String>,
        _extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> Result<()> {
        Err(SessionError::InvalidTransition(
            "session-scoped response dispatch has no exact inbound transaction authority"
                .to_string(),
        ))
    }

    /// Send response (for UAS)
    pub async fn send_response(
        &self,
        _session_id: &SessionId,
        _code: u16,
        _sdp: Option<String>,
    ) -> Result<()> {
        Err(SessionError::InvalidTransition(
            "session-scoped response dispatch has no exact inbound transaction authority"
                .to_string(),
        ))
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

    /// Send a REFER final response through the exact inbound server
    /// transaction. This is the causal path; the cross-crate event bus only
    /// observes outcomes and is not required for wire progress.
    pub(crate) async fn send_refer_response_classified(
        &self,
        transaction_id: &str,
        code: u16,
    ) -> std::result::Result<
        rvoip_sip_dialog::FinalResponseCompletionDisposition,
        rvoip_sip_dialog::ExactResponseSendError,
    > {
        let transaction_id = transaction_id.parse::<TransactionKey>().map_err(|_| {
            rvoip_sip_dialog::ExactResponseSendError {
                source: rvoip_sip_dialog::ApiError::Protocol {
                    message: "pending REFER transaction identifier is invalid".to_string(),
                },
                disposition:
                    rvoip_sip_dialog::FinalResponseCompletionDisposition::ZeroWireRetryable,
            }
        })?;
        if transaction_id.method() != &rvoip_sip_core::Method::Refer || !transaction_id.is_server()
        {
            return Err(rvoip_sip_dialog::ExactResponseSendError {
                source: rvoip_sip_dialog::ApiError::Protocol {
                    message: "pending REFER response does not identify a server REFER transaction"
                        .to_string(),
                },
                disposition:
                    rvoip_sip_dialog::FinalResponseCompletionDisposition::ZeroWireRetryable,
            });
        }
        let status =
            StatusCode::from_u16(code).map_err(|_| rvoip_sip_dialog::ExactResponseSendError {
                source: rvoip_sip_dialog::ApiError::Protocol {
                    message: "pending REFER response has an invalid final status".to_string(),
                },
                disposition:
                    rvoip_sip_dialog::FinalResponseCompletionDisposition::ZeroWireRetryable,
            })?;
        self.dialog_api
            .send_status_response_classified(&transaction_id, status, None)
            .await
    }

    /// Send a transaction-oriented REGISTER response directly through the
    /// dialog/transaction API. The response event type remains available for
    /// compatibility observation, but wire correctness does not depend on it.
    pub(crate) async fn send_register_response_fields(
        &self,
        fields: &crate::api::respond::register_response::RegisterResponseEventFields,
    ) -> Result<()> {
        match self.send_register_response_fields_classified(fields).await {
            Ok(rvoip_sip_dialog::FinalResponseCompletionDisposition::WrittenSuccessTerminal)
            | Ok(rvoip_sip_dialog::FinalResponseCompletionDisposition::WireUnknownErrorTerminal) => {
                Ok(())
            }
            Ok(rvoip_sip_dialog::FinalResponseCompletionDisposition::ZeroWireRetryable) => {
                Err(SessionError::DialogError(
                    "failed to send exact REGISTER response before transport write".to_string(),
                ))
            }
            Err(error)
                if error.disposition
                    != rvoip_sip_dialog::FinalResponseCompletionDisposition::ZeroWireRetryable =>
            {
                tracing::warn!(
                    status_code = fields.status_code,
                    disposition = ?error.disposition,
                    "REGISTER final response became terminal at the transport boundary; suppressing duplicate fallback"
                );
                Ok(())
            }
            Err(_) => Err(SessionError::DialogError(
                "failed to send exact REGISTER response before transport write".to_string(),
            )),
        }
    }

    /// Classified exact-transaction form used by the built-in registrar.
    /// No session or dialog index participates because REGISTER is a
    /// transaction-oriented method.
    pub(crate) async fn send_register_response_fields_classified(
        &self,
        fields: &crate::api::respond::register_response::RegisterResponseEventFields,
    ) -> std::result::Result<
        rvoip_sip_dialog::FinalResponseCompletionDisposition,
        rvoip_sip_dialog::ExactResponseSendError,
    > {
        let transaction_id = fields
            .transaction_id
            .parse::<TransactionKey>()
            .map_err(|_| rvoip_sip_dialog::ExactResponseSendError {
                source: rvoip_sip_dialog::ApiError::Protocol {
                    message: "pending REGISTER transaction identifier is invalid".to_string(),
                },
                disposition:
                    rvoip_sip_dialog::FinalResponseCompletionDisposition::ZeroWireRetryable,
            })?;
        if transaction_id.method() != &rvoip_sip_core::Method::Register
            || !transaction_id.is_server()
        {
            return Err(rvoip_sip_dialog::ExactResponseSendError {
                source: rvoip_sip_dialog::ApiError::Protocol {
                    message: "REGISTER response does not identify a server REGISTER transaction"
                        .to_string(),
                },
                disposition:
                    rvoip_sip_dialog::FinalResponseCompletionDisposition::ZeroWireRetryable,
            });
        }
        self.dialog_api
            .send_register_response_with_extras_classified(
                &transaction_id,
                fields.status_code,
                &fields.reason,
                fields.www_authenticate.as_deref(),
                fields.contact.as_deref(),
                fields.expires,
                fields.min_expires,
                &fields.service_route,
                fields.path_echo,
                &fields.associated_uri,
                &fields.extra_headers,
            )
            .await
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

    /// ACK generation for INVITE 2xx is owned by dialog/transaction response
    /// processing. Retain the established method signature, but reject a
    /// second application-authored ACK attempt.
    pub async fn send_ack(&self, session_id: &SessionId, _response: &Response) -> Result<()> {
        Err(SessionError::InvalidTransition(format!(
            "ACK for session {} is generated automatically by the exact INVITE transaction",
            session_id.0
        )))
    }

    pub(crate) async fn send_delayed_offer_ack_exact(
        &self,
        handle: &SessionRegistryHandle,
        transaction_id: &TransactionKey,
        response: &Response,
        sdp_answer: &str,
    ) -> Result<()> {
        self.store
            .get_session_snapshot_exact(handle)
            .map_err(|_| SessionError::SessionNotFound(handle.session_id().0.clone()))?;
        self.dialog_api
            .send_delayed_offer_ack_for_session_transaction(
                &handle.session_id().0,
                transaction_id,
                response,
                sdp_answer,
            )
            .await
            .map_err(|_| {
                SessionError::DialogError(
                    "Delayed-offer ACK failed exact validation or transport write".to_string(),
                )
            })
    }

    pub(crate) async fn send_invite_2xx_ack_exact(
        &self,
        handle: &SessionRegistryHandle,
        transaction_id: &TransactionKey,
        response: &Response,
    ) -> Result<()> {
        self.store
            .get_session_snapshot_exact(handle)
            .map_err(|_| SessionError::SessionNotFound(handle.session_id().0.clone()))?;
        self.dialog_api
            .send_invite_2xx_ack_for_session_transaction(
                &handle.session_id().0,
                transaction_id,
                response,
            )
            .await
            .map_err(|_| {
                SessionError::DialogError(
                    "INVITE 2xx ACK failed exact validation or transport write".to_string(),
                )
            })
    }

    /// Compatibility facade for a default established-call BYE.
    pub async fn send_bye_session(&self, session_id: &SessionId) -> Result<()> {
        self.send_bye_with_options(session_id, ByeRequestOptions::default())
            .await
    }

    /// Compatibility facade for an established BYE with RFC 3326 Reason.
    pub async fn send_bye_session_with_reason(
        &self,
        session_id: &SessionId,
        reason: rvoip_sip_core::types::reason::Reason,
    ) -> Result<()> {
        self.send_bye_with_options(
            session_id,
            ByeRequestOptions {
                reason: Some(reason.to_string()),
                ..Default::default()
            },
        )
        .await
    }

    /// SIP_API_DESIGN_2 Phase C — UPDATE (RFC 3311) dispatch routed
    /// through the new dialog-core options surface. SDP is optional;
    /// when present it rides on the UPDATE body. The builder layer
    /// supplies a fully-populated `UpdateRequestOptions`.
    pub async fn send_update_with_options(
        &self,
        session_id: &SessionId,
        opts: rvoip_sip_dialog::api::unified::UpdateRequestOptions,
    ) -> Result<TransactionKey> {
        let Some((_handle, _lane, session)) =
            lock_and_load_exact_current_session(self.store.as_ref(), session_id).await
        else {
            return Err(SessionError::SessionNotFound(session_id.0.clone()));
        };
        self.dispatch_update_with_options(&session, opts, None)
            .await
    }

    pub(crate) async fn send_update_with_options_lane_owned(
        &self,
        session: &SessionState,
        opts: UpdateRequestOptions,
    ) -> Result<TransactionKey> {
        self.dispatch_update_with_options(session, opts, None).await
    }

    async fn dispatch_update_with_options(
        &self,
        session: &SessionState,
        mut opts: UpdateRequestOptions,
        auth_header: Option<(&str, String)>,
    ) -> Result<TransactionKey> {
        opts.extra_headers = match auth_header {
            Some((name, value)) => apply_outbound_extras_policy_with_auth(
                rvoip_sip_core::types::Method::Update,
                opts.extra_headers,
                self.outbound_proxy_uri.as_ref(),
                name,
                value,
            )?,
            None => apply_outbound_extras_policy(
                rvoip_sip_core::types::Method::Update,
                opts.extra_headers,
                self.outbound_proxy_uri.as_ref(),
            )?,
        };
        let dialog_id = resolve_dialog_for_lane_owned_session(self.store.as_ref(), session)?;
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
        opts: rvoip_sip_dialog::api::unified::ReInviteRequestOptions,
    ) -> Result<()> {
        let Some((_handle, _lane, session)) =
            lock_and_load_exact_current_session(self.store.as_ref(), session_id).await
        else {
            return Err(SessionError::SessionNotFound(session_id.0.clone()));
        };
        self.dispatch_reinvite_with_options(&session, opts, None)
            .await
            .map(|_| ())
    }

    pub(crate) async fn send_reinvite_with_options_lane_owned(
        &self,
        session: &SessionState,
        opts: rvoip_sip_dialog::api::unified::ReInviteRequestOptions,
    ) -> Result<TransactionKey> {
        self.dispatch_reinvite_with_options(session, opts, None)
            .await
    }

    pub(crate) async fn send_reinvite_with_auth_lane_owned(
        &self,
        session: &SessionState,
        opts: rvoip_sip_dialog::api::unified::ReInviteRequestOptions,
        header_name: &str,
        header_value: String,
    ) -> Result<TransactionKey> {
        self.dispatch_reinvite_with_options(session, opts, Some((header_name, header_value)))
            .await
    }

    async fn dispatch_reinvite_with_options(
        &self,
        session: &SessionState,
        mut opts: rvoip_sip_dialog::api::unified::ReInviteRequestOptions,
        auth_header: Option<(&str, String)>,
    ) -> Result<TransactionKey> {
        // A challenged precomputed credential is replaced, not appended.
        // Keeping it would cause dialog-core to materialize a second
        // Authorization field beside the exact retry header below.
        if auth_header.is_some() {
            opts.precomputed_authorization = None;
        }
        opts.extra_headers = match auth_header {
            Some((name, value)) => apply_outbound_extras_policy_with_auth(
                rvoip_sip_core::types::Method::Invite,
                opts.extra_headers,
                self.outbound_proxy_uri.as_ref(),
                name,
                value,
            )?,
            None => apply_outbound_extras_policy(
                rvoip_sip_core::types::Method::Invite,
                opts.extra_headers,
                self.outbound_proxy_uri.as_ref(),
            )?,
        };
        let dialog_id = resolve_dialog_for_lane_owned_session(self.store.as_ref(), session)?;
        self.dialog_api
            .send_reinvite_with_options(&dialog_id, opts)
            .await
            .map_err(|error| {
                redacted_invite_dispatch_error(InviteDispatchFailure::ReinviteWithOptions, error)
            })
    }

    /// SIP_API_DESIGN_2 Phase C — REFER dispatch through the new
    /// dialog-core options surface; carries the full RFC 3891
    /// `Replaces`, RFC 3892 `Referred-By`, RFC 4538 `Target-Dialog`
    /// trio.
    pub async fn send_refer_with_options(
        &self,
        session_id: &SessionId,
        opts: rvoip_sip_dialog::api::unified::ReferRequestOptions,
    ) -> Result<TransactionKey> {
        let Some((_handle, _lane, session)) =
            lock_and_load_exact_current_session(self.store.as_ref(), session_id).await
        else {
            return Err(SessionError::SessionNotFound(session_id.0.clone()));
        };
        self.dispatch_refer_with_options(&session, opts, None).await
    }

    pub(crate) async fn send_refer_with_options_lane_owned(
        &self,
        session: &SessionState,
        opts: ReferRequestOptions,
    ) -> Result<TransactionKey> {
        self.dispatch_refer_with_options(session, opts, None).await
    }

    async fn dispatch_refer_with_options(
        &self,
        session: &SessionState,
        mut opts: ReferRequestOptions,
        auth_header: Option<(&str, String)>,
    ) -> Result<TransactionKey> {
        opts.extra_headers = match auth_header {
            Some((name, value)) => apply_outbound_extras_policy_with_auth(
                rvoip_sip_core::types::Method::Refer,
                opts.extra_headers,
                self.outbound_proxy_uri.as_ref(),
                name,
                value,
            )?,
            None => apply_outbound_extras_policy(
                rvoip_sip_core::types::Method::Refer,
                opts.extra_headers,
                self.outbound_proxy_uri.as_ref(),
            )?,
        };
        let dialog_id = resolve_dialog_for_lane_owned_session(self.store.as_ref(), session)?;
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
        opts: rvoip_sip_dialog::api::unified::InfoRequestOptions,
    ) -> Result<TransactionKey> {
        let Some((_handle, _lane, session)) =
            lock_and_load_exact_current_session(self.store.as_ref(), session_id).await
        else {
            return Err(SessionError::SessionNotFound(session_id.0.clone()));
        };
        self.dispatch_info_with_options(&session, opts, None).await
    }

    pub(crate) async fn send_info_with_options_lane_owned(
        &self,
        session: &SessionState,
        opts: InfoRequestOptions,
    ) -> Result<TransactionKey> {
        self.dispatch_info_with_options(session, opts, None).await
    }

    async fn dispatch_info_with_options(
        &self,
        session: &SessionState,
        mut opts: InfoRequestOptions,
        auth_header: Option<(&str, String)>,
    ) -> Result<TransactionKey> {
        opts.extra_headers = match auth_header {
            Some((name, value)) => apply_outbound_extras_policy_with_auth(
                rvoip_sip_core::types::Method::Info,
                opts.extra_headers,
                self.outbound_proxy_uri.as_ref(),
                name,
                value,
            )?,
            None => apply_outbound_extras_policy(
                rvoip_sip_core::types::Method::Info,
                opts.extra_headers,
                self.outbound_proxy_uri.as_ref(),
            )?,
        };
        let dialog_id = resolve_dialog_for_lane_owned_session(self.store.as_ref(), session)?;
        self.dialog_api
            .send_info_with_options(&dialog_id, opts)
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to send INFO: {}", e)))
    }

    /// Preserve the adapter's established options signature while routing the
    /// request through one exact YAML action.
    pub async fn send_bye_with_options(
        &self,
        session_id: &SessionId,
        opts: rvoip_sip_dialog::api::unified::ByeRequestOptions,
    ) -> Result<()> {
        let handle = self
            .store
            .lifecycle_handle(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.to_string()))?;
        self.dispatch_state_machine_options_exact(
            &handle,
            EventType::SendOutboundBye,
            crate::state_machine::executor::PendingOptionsSlot::Bye(Arc::new(opts)),
        )
        .await
    }

    /// Dispatch a first-attempt BYE while the executor already owns this
    /// session's exact state-machine lane. Retained authorization bookkeeping
    /// stays in the executor's working state and is published by its one
    /// canonical event commit.
    pub(crate) async fn send_bye_with_options_lane_owned(
        &self,
        session: &mut SessionState,
        mut opts: ByeRequestOptions,
    ) -> Result<()> {
        opts.extra_headers = apply_outbound_extras_policy(
            rvoip_sip_core::types::Method::Bye,
            opts.extra_headers,
            self.outbound_proxy_uri.as_ref(),
        )?;
        let dialog_id = resolve_dialog_for_lane_owned_session(self.store.as_ref(), session)
            .map_err(|_| SessionError::SessionNotFound(session.session_id.0.clone()))?;
        let dialog = self
            .dialog_api
            .dialog_manager()
            .core()
            .get_dialog(&dialog_id)
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
        let transport = self.outbound_transport_context_for_uri(&next_hop);
        let headers = mutate_retained_dialog_auth(
            session,
            &dialog_id,
            RetainedDialogAuthMode::Request,
            "BYE",
            &request_uri,
            RetainedDialogAuthRoute {
                next_hop: &next_hop,
                transport: &transport,
            },
            None,
        )?;
        opts.extra_headers.extend(headers);

        let handle = session.lifecycle_handle.as_ref().ok_or_else(|| {
            SessionError::InvalidTransition(
                "SIP BYE exact session has no lifecycle authority".to_string(),
            )
        })?;
        self.dispatch_bye_with_options(handle, &dialog_id, opts, request_uri)
            .await
    }

    /// Canonical BYE dispatch for both first attempts and challenge retries.
    /// The exact transaction generation and completion handle are installed
    /// here, beside the only dialog-layer BYE wire call.
    async fn dispatch_bye_with_options(
        &self,
        handle: &SessionRegistryHandle,
        dialog_id: &RvoipDialogId,
        opts: ByeRequestOptions,
        request_uri: String,
    ) -> Result<()> {
        self.mark_initial_invite_protocol_teardown_exact(handle);
        let generation = self.next_outgoing_bye_generation();
        let (transaction_id, completion) = self
            .dialog_api
            .send_bye_with_options_and_completion(dialog_id, opts)
            .await
            .map_err(|error| redacted_dialog_operation_error("SIP BYE", error))?;
        self.retain_outgoing_bye_transaction(
            handle,
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

    /// Publish cancellation-safe intent before any BYE wire dispatch. Exact
    /// response cleanup can then preserve a terminal receipt across the small
    /// dispatch-to-wait gap, while dropping the last owner reclaims a receipt
    /// if its public future is cancelled or exits on a bookkeeping error.
    pub(crate) fn begin_outgoing_bye_wait_exact(
        &self,
        handle: &SessionRegistryHandle,
    ) -> Result<OutgoingByeWaitOwner> {
        self.store.get_session_snapshot_exact(handle).map_err(|_| {
            SessionError::SessionNotFound(format!(
                "Session {} exact BYE lifetime is unavailable",
                handle.session_id().0
            ))
        })?;
        let after_generation = self
            .outgoing_bye_tx
            .get(handle)
            .filter(|transaction| transaction.handle == *handle)
            .map_or(0, |transaction| transaction.generation);
        self.outgoing_bye_wait_intents
            .entry(handle.clone())
            .and_modify(|intent| {
                if intent.handle == *handle {
                    intent.owners = intent.owners.saturating_add(1);
                    intent.min_after_generation = intent.min_after_generation.min(after_generation);
                } else {
                    *intent = OutgoingByeWaitIntentState {
                        handle: handle.clone(),
                        owners: 1,
                        min_after_generation: after_generation,
                    };
                }
            })
            .or_insert(OutgoingByeWaitIntentState {
                handle: handle.clone(),
                owners: 1,
                min_after_generation: after_generation,
            });
        Ok(OutgoingByeWaitOwner {
            handle: handle.clone(),
            wait_intents: Arc::clone(&self.outgoing_bye_wait_intents),
            transactions: Arc::clone(&self.outgoing_bye_tx),
            generation_watch: Arc::clone(&self.outgoing_bye_generation_watch),
        })
    }

    fn retain_outgoing_bye_transaction(
        &self,
        handle: &SessionRegistryHandle,
        generation: u64,
        transaction_id: TransactionKey,
        completion: ClientTransactionCompletionHandle,
        request_uri: String,
    ) {
        let transaction = OutboundByeTransaction {
            handle: handle.clone(),
            generation,
            transaction_id,
            completion,
            request_uri,
        };
        self.outgoing_bye_tx
            .entry(handle.clone())
            .and_modify(|current| {
                if current.handle != transaction.handle
                    || current.generation < transaction.generation
                {
                    *current = transaction.clone();
                }
            })
            .or_insert(transaction);
        let sender = self
            .outgoing_bye_generation_watch
            .entry(handle.clone())
            .or_insert_with(|| tokio::sync::watch::channel(0).0)
            .clone();
        sender.send_replace(generation);
    }

    fn latest_outgoing_bye_transaction(
        &self,
        handle: &SessionRegistryHandle,
        after_generation: u64,
    ) -> Option<OutboundByeTransaction> {
        self.outgoing_bye_tx
            .get(handle)
            .map(|entry| entry.value().clone())
            .filter(|transaction| {
                transaction.handle == *handle && transaction.generation > after_generation
            })
    }

    /// Capture the exact retained-BYE generation, if any, before a state
    /// machine dispatch. A caller can later prove that this dispatch reached
    /// the wire by observing a strictly newer generation for the same
    /// session; an unrelated session cannot satisfy that proof.
    pub(crate) fn outgoing_bye_generation_exact(
        &self,
        handle: &SessionRegistryHandle,
    ) -> Option<u64> {
        self.outgoing_bye_tx
            .get(handle)
            .filter(|transaction| transaction.handle == *handle)
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
    pub(crate) fn has_outgoing_bye_after_exact(
        &self,
        handle: &SessionRegistryHandle,
        generation: u64,
    ) -> bool {
        self.outgoing_bye_tx.get(handle).is_some_and(|transaction| {
            transaction.handle == *handle && transaction.generation > generation
        })
    }

    /// Prove that an authentication event owns the latest retained BYE
    /// generation for this exact session. Stale or cross-session challenge
    /// events must not be allowed to consume the immutable BYE retry stash.
    pub(crate) fn outgoing_bye_transaction_matches_exact(
        &self,
        handle: &SessionRegistryHandle,
        transaction_id: &TransactionKey,
    ) -> bool {
        self.outgoing_bye_tx.get(handle).is_some_and(|transaction| {
            transaction.handle == *handle && transaction.transaction_id == *transaction_id
        })
    }

    /// Admit a terminal BYE completion when it names the latest retained
    /// attempt, or when a cancelled public waiter has already reclaimed that
    /// receipt. A retained newer authentication retry fences a stale outcome.
    pub(crate) fn outgoing_bye_completion_is_current_or_unretained_exact(
        &self,
        handle: &SessionRegistryHandle,
        transaction_id: &TransactionKey,
    ) -> bool {
        self.outgoing_bye_tx.get(handle).is_none_or(|transaction| {
            transaction.handle == *handle && transaction.transaction_id == *transaction_id
        })
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
    pub(crate) async fn outgoing_bye_request_uri_exact(
        &self,
        handle: &SessionRegistryHandle,
    ) -> Result<String> {
        self.outgoing_bye_tx
            .get(handle)
            .filter(|transaction| transaction.handle == *handle)
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

    fn clear_outgoing_bye_transaction(&self, transaction: &OutboundByeTransaction) {
        let removed = self
            .outgoing_bye_tx
            .remove_if(&transaction.handle, |_, current| {
                current.handle == transaction.handle
                    && current.generation == transaction.generation
                    && current.transaction_id == transaction.transaction_id
            });
        if removed.is_some()
            && !self
                .outgoing_bye_tx
                .get(&transaction.handle)
                .is_some_and(|current| current.handle == transaction.handle)
        {
            self.outgoing_bye_generation_watch
                .remove(&transaction.handle);
        }
    }

    /// Wait until the latest BYE attempt reaches an authoritative terminal
    /// outcome. A 2xx confirms receipt, while RFC 3261 §15.1.1 defines 481
    /// as an already-terminated peer dialog and therefore a graceful BYE
    /// result. A 401/407 is not terminal: the generic request-auth flow may
    /// install one newer transaction, which this loop follows. Every other
    /// non-2xx, timeout, or unobservable transaction fails closed.
    pub(crate) async fn wait_for_outgoing_bye_final_response_exact(
        &self,
        handle: &SessionRegistryHandle,
    ) -> Result<()> {
        let deadline = tokio::time::Instant::now() + self.non_invite_transaction_timeout;
        let mut after_generation = 0;
        let mut last_transaction = None;
        let mut generation_changes = self
            .outgoing_bye_generation_watch
            .entry(handle.clone())
            .or_insert_with(|| tokio::sync::watch::channel(0).0)
            .subscribe();
        loop {
            let transaction = loop {
                if let Some(transaction) =
                    self.latest_outgoing_bye_transaction(handle, after_generation)
                {
                    break transaction;
                }
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    if let Some(transaction) = last_transaction.as_ref() {
                        self.clear_outgoing_bye_transaction(transaction);
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
                        self.clear_outgoing_bye_transaction(transaction);
                    }
                    return Err(SessionError::Timeout(
                        "SIP BYE transaction was not available before its deadline".to_string(),
                    ));
                }
            };
            last_transaction = Some(transaction.clone());
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                self.clear_outgoing_bye_transaction(&transaction);
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
                        .latest_outgoing_bye_transaction(handle, transaction.generation)
                        .is_some();
                    let retained_transaction_exists =
                        self.outgoing_bye_tx.get(handle).is_some_and(|retained| {
                            retained.handle == *handle
                        });
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
                .latest_outgoing_bye_transaction(handle, transaction.generation)
                .is_some();
            match response {
                Ok(Some(ClientTransactionOutcome::FinalResponse(response))) => {
                    match classify_outgoing_bye_final_response(response.status_code()) {
                        OutgoingByeFinalDisposition::Confirmed => {
                            self.clear_outgoing_bye_transaction(&transaction);
                            return Ok(());
                        }
                        OutgoingByeFinalDisposition::PeerAlreadyTerminated => {
                            tracing::debug!(
                                session = %handle.session_id(),
                                "SIP BYE peer already terminated the exact dialog"
                            );
                            self.clear_outgoing_bye_transaction(&transaction);
                            return Ok(());
                        }
                        OutgoingByeFinalDisposition::AuthenticationChallenge => {
                            after_generation = transaction.generation;
                        }
                        OutgoingByeFinalDisposition::Rejected => {
                            self.clear_outgoing_bye_transaction(&transaction);
                            return Err(SessionError::ProtocolError(
                                "SIP BYE received a non-success final response".to_string(),
                            ));
                        }
                    }
                }
                Ok(Some(ClientTransactionOutcome::Failure(_))) | Ok(None) => {
                    if newer_transaction {
                        after_generation = transaction.generation;
                        continue;
                    }
                    self.clear_outgoing_bye_transaction(&transaction);
                    return Err(SessionError::Timeout(
                        "SIP BYE final response timed out".to_string(),
                    ));
                }
                Err(_) => {
                    if newer_transaction {
                        after_generation = transaction.generation;
                        continue;
                    }
                    self.clear_outgoing_bye_transaction(&transaction);
                    return Err(SessionError::DialogError(
                        "SIP BYE final response could not be observed".to_string(),
                    ));
                }
            }
        }
    }

    /// SIP_API_DESIGN_2 Phase C — NOTIFY dispatch through the new
    /// dialog-core options surface.
    pub async fn send_notify_with_options(
        &self,
        session_id: &SessionId,
        opts: rvoip_sip_dialog::api::unified::NotifyRequestOptions,
    ) -> Result<TransactionKey> {
        let Some((_handle, _lane, session)) =
            lock_and_load_exact_current_session(self.store.as_ref(), session_id).await
        else {
            return Err(SessionError::SessionNotFound(session_id.0.clone()));
        };
        self.dispatch_notify_with_options(&session, opts, None)
            .await
    }

    pub(crate) async fn send_notify_with_options_lane_owned(
        &self,
        session: &SessionState,
        opts: NotifyRequestOptions,
    ) -> Result<TransactionKey> {
        self.dispatch_notify_with_options(session, opts, None).await
    }

    async fn dispatch_notify_with_options(
        &self,
        session: &SessionState,
        mut opts: NotifyRequestOptions,
        auth_header: Option<(&str, String)>,
    ) -> Result<TransactionKey> {
        opts.extra_headers = match auth_header {
            Some((name, value)) => apply_outbound_extras_policy_with_auth(
                rvoip_sip_core::types::Method::Notify,
                opts.extra_headers,
                self.outbound_proxy_uri.as_ref(),
                name,
                value,
            )?,
            None => apply_outbound_extras_policy(
                rvoip_sip_core::types::Method::Notify,
                opts.extra_headers,
                self.outbound_proxy_uri.as_ref(),
            )?,
        };
        let dialog_id = resolve_dialog_for_lane_owned_session(self.store.as_ref(), session)?;
        self.dialog_api
            .send_notify_with_options(&dialog_id, opts)
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to send NOTIFY: {}", e)))
    }

    /// Dispatch one standalone request snapshot, optionally adding the
    /// authorization header computed for that same snapshot.
    pub(crate) async fn send_standalone_request(
        &self,
        request: StandaloneRequestOptions,
        auth_header: Option<(&str, String)>,
    ) -> Result<rvoip_sip_core::Response> {
        let authenticated = auth_header.is_some();
        match request {
            StandaloneRequestOptions::Message(mut options) => {
                options.extra_headers = match auth_header {
                    Some((name, value)) => apply_outbound_extras_policy_with_auth(
                        rvoip_sip_core::Method::Message,
                        options.extra_headers,
                        self.outbound_proxy_uri.as_ref(),
                        name,
                        value,
                    )?,
                    None => apply_outbound_extras_policy(
                        rvoip_sip_core::Method::Message,
                        options.extra_headers,
                        self.outbound_proxy_uri.as_ref(),
                    )?,
                };
                self.dialog_api
                    .send_message_out_of_dialog_with_options(options)
                    .await
                    .map_err(|error| {
                        let operation = if authenticated {
                            "MESSAGE with auth"
                        } else {
                            "MESSAGE"
                        };
                        SessionError::DialogError(format!("Failed to send {operation}: {error}"))
                    })
            }
            StandaloneRequestOptions::Options(mut options) => {
                options.extra_headers = match auth_header {
                    Some((name, value)) => apply_outbound_extras_policy_with_auth(
                        rvoip_sip_core::Method::Options,
                        options.extra_headers,
                        self.outbound_proxy_uri.as_ref(),
                        name,
                        value,
                    )?,
                    None => apply_outbound_extras_policy(
                        rvoip_sip_core::Method::Options,
                        options.extra_headers,
                        self.outbound_proxy_uri.as_ref(),
                    )?,
                };
                self.dialog_api
                    .send_options_out_of_dialog_with_options(options)
                    .await
                    .map_err(|error| {
                        let operation = if authenticated {
                            "OPTIONS with auth"
                        } else {
                            "OPTIONS"
                        };
                        SessionError::DialogError(format!("Failed to send {operation}: {error}"))
                    })
            }
            StandaloneRequestOptions::Subscribe {
                target,
                mut options,
            } => {
                options.extra_headers = match auth_header {
                    Some((name, value)) => apply_outbound_extras_policy_with_auth(
                        rvoip_sip_core::Method::Subscribe,
                        options.extra_headers,
                        self.outbound_proxy_uri.as_ref(),
                        name,
                        value,
                    )?,
                    None => apply_outbound_extras_policy(
                        rvoip_sip_core::Method::Subscribe,
                        options.extra_headers,
                        self.outbound_proxy_uri.as_ref(),
                    )?,
                };
                self.dialog_api
                    .send_subscribe_with_options(&target, options)
                    .await
                    .map_err(|error| {
                        let operation = if authenticated {
                            "SUBSCRIBE with auth"
                        } else {
                            "SUBSCRIBE"
                        };
                        SessionError::DialogError(format!("Failed to send {operation}: {error}"))
                    })
            }
        }
    }

    /// SIP_API_DESIGN_2 Phase C — out-of-dialog MESSAGE dispatch
    /// through the new dialog-core options surface. Returns the
    /// registrar's `Response` so the caller can inspect 200 OK vs
    /// 401 auth-challenge vs 404. No session_id is required because
    /// MESSAGE is fire-and-forget per RFC 3428.
    pub async fn send_message_oob_with_options(
        &self,
        opts: rvoip_sip_dialog::api::unified::MessageRequestOptions,
    ) -> Result<rvoip_sip_core::Response> {
        self.send_standalone_request(StandaloneRequestOptions::Message(opts), None)
            .await
    }

    /// SIP_API_DESIGN_2 Phase C — out-of-dialog OPTIONS dispatch.
    /// Today returns the wire-`Response` when dialog-core ships the
    /// transaction-authorship; until then dialog-core's stub returns
    /// `NotImplemented` and that error bubbles through unchanged.
    pub async fn send_options_oob_with_options(
        &self,
        opts: rvoip_sip_dialog::api::unified::OptionsRequestOptions,
    ) -> Result<rvoip_sip_core::Response> {
        self.send_standalone_request(StandaloneRequestOptions::Options(opts), None)
            .await
    }

    /// SIP_API_DESIGN_2 Phase C — out-of-dialog SUBSCRIBE dispatch.
    /// Returns the registrar's `Response` so callers can inspect
    /// `Expires`, `Min-Expires`, or 401 challenge.
    pub async fn send_subscribe_oob_with_options(
        &self,
        target: &str,
        opts: rvoip_sip_dialog::api::unified::SubscribeRequestOptions,
    ) -> Result<rvoip_sip_core::Response> {
        self.send_standalone_request(
            StandaloneRequestOptions::Subscribe {
                target: target.to_string(),
                options: opts,
            },
            None,
        )
        .await
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

    /// Authentication retry is an executor-owned continuation of an exact
    /// challenged BYE. External callers cannot safely supply only a header
    /// without the retained transaction/challenge generation.
    pub async fn send_bye_with_auth(
        &self,
        session_id: &SessionId,
        _opts: rvoip_sip_dialog::api::unified::ByeRequestOptions,
        _auth_header_name: &str,
        _auth_header_value: String,
    ) -> Result<()> {
        Err(SessionError::InvalidTransition(format!(
            "BYE authentication retry for session {} requires the exact state-machine challenge owner",
            session_id.0
        )))
    }

    /// Dispatch an authentication retry while the executor already owns the
    /// exact session lane. Digest state was computed in the lane-owned action;
    /// this path only adds that immutable header to the retained request
    /// snapshot and therefore has no competing store writer.
    pub(crate) async fn send_bye_with_auth_lane_owned(
        &self,
        session: &mut SessionState,
        mut opts: ByeRequestOptions,
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
        let dialog_id = resolve_dialog_for_lane_owned_session(self.store.as_ref(), session)?;
        // Authentication retries must sign and retain the challenged
        // generation's exact Request-URI.
        let handle = session.lifecycle_handle.as_ref().ok_or_else(|| {
            SessionError::InvalidTransition(
                "SIP BYE exact session has no lifecycle authority".to_string(),
            )
        })?;
        let request_uri = self.outgoing_bye_request_uri_exact(handle).await?;
        self.dispatch_bye_with_options(handle, &dialog_id, opts, request_uri)
            .await
    }

    pub async fn send_refer_with_auth(
        &self,
        session_id: &SessionId,
        opts: rvoip_sip_dialog::api::unified::ReferRequestOptions,
        auth_header_name: &str,
        auth_header_value: String,
    ) -> Result<TransactionKey> {
        let Some((_handle, _lane, session)) =
            lock_and_load_exact_current_session(self.store.as_ref(), session_id).await
        else {
            return Err(SessionError::SessionNotFound(session_id.0.clone()));
        };
        self.send_refer_with_auth_lane_owned(&session, opts, auth_header_name, auth_header_value)
            .await
    }

    pub(crate) async fn send_refer_with_auth_lane_owned(
        &self,
        session: &SessionState,
        opts: ReferRequestOptions,
        auth_header_name: &str,
        auth_header_value: String,
    ) -> Result<TransactionKey> {
        self.dispatch_refer_with_options(session, opts, Some((auth_header_name, auth_header_value)))
            .await
    }

    pub async fn send_notify_with_auth(
        &self,
        session_id: &SessionId,
        opts: rvoip_sip_dialog::api::unified::NotifyRequestOptions,
        auth_header_name: &str,
        auth_header_value: String,
    ) -> Result<TransactionKey> {
        let Some((_handle, _lane, session)) =
            lock_and_load_exact_current_session(self.store.as_ref(), session_id).await
        else {
            return Err(SessionError::SessionNotFound(session_id.0.clone()));
        };
        self.send_notify_with_auth_lane_owned(&session, opts, auth_header_name, auth_header_value)
            .await
    }

    pub(crate) async fn send_notify_with_auth_lane_owned(
        &self,
        session: &SessionState,
        opts: NotifyRequestOptions,
        auth_header_name: &str,
        auth_header_value: String,
    ) -> Result<TransactionKey> {
        self.dispatch_notify_with_options(
            session,
            opts,
            Some((auth_header_name, auth_header_value)),
        )
        .await
    }

    pub async fn send_info_with_auth(
        &self,
        session_id: &SessionId,
        opts: rvoip_sip_dialog::api::unified::InfoRequestOptions,
        auth_header_name: &str,
        auth_header_value: String,
    ) -> Result<TransactionKey> {
        let Some((_handle, _lane, session)) =
            lock_and_load_exact_current_session(self.store.as_ref(), session_id).await
        else {
            return Err(SessionError::SessionNotFound(session_id.0.clone()));
        };
        self.send_info_with_auth_lane_owned(&session, opts, auth_header_name, auth_header_value)
            .await
    }

    pub(crate) async fn send_info_with_auth_lane_owned(
        &self,
        session: &SessionState,
        opts: InfoRequestOptions,
        auth_header_name: &str,
        auth_header_value: String,
    ) -> Result<TransactionKey> {
        self.dispatch_info_with_options(session, opts, Some((auth_header_name, auth_header_value)))
            .await
    }

    pub async fn send_update_with_auth(
        &self,
        session_id: &SessionId,
        opts: rvoip_sip_dialog::api::unified::UpdateRequestOptions,
        auth_header_name: &str,
        auth_header_value: String,
    ) -> Result<TransactionKey> {
        let Some((_handle, _lane, session)) =
            lock_and_load_exact_current_session(self.store.as_ref(), session_id).await
        else {
            return Err(SessionError::SessionNotFound(session_id.0.clone()));
        };
        self.send_update_with_auth_lane_owned(&session, opts, auth_header_name, auth_header_value)
            .await
    }

    pub(crate) async fn send_update_with_auth_lane_owned(
        &self,
        session: &SessionState,
        opts: UpdateRequestOptions,
        auth_header_name: &str,
        auth_header_value: String,
    ) -> Result<TransactionKey> {
        self.dispatch_update_with_options(
            session,
            opts,
            Some((auth_header_name, auth_header_value)),
        )
        .await
    }

    pub async fn send_message_oob_with_auth(
        &self,
        opts: rvoip_sip_dialog::api::unified::MessageRequestOptions,
        auth_header_name: &str,
        auth_header_value: String,
    ) -> Result<rvoip_sip_core::Response> {
        self.send_standalone_request(
            StandaloneRequestOptions::Message(opts),
            Some((auth_header_name, auth_header_value)),
        )
        .await
    }

    pub async fn send_options_oob_with_auth(
        &self,
        opts: rvoip_sip_dialog::api::unified::OptionsRequestOptions,
        auth_header_name: &str,
        auth_header_value: String,
    ) -> Result<rvoip_sip_core::Response> {
        self.send_standalone_request(
            StandaloneRequestOptions::Options(opts),
            Some((auth_header_name, auth_header_value)),
        )
        .await
    }

    pub async fn send_subscribe_oob_with_auth(
        &self,
        target: &str,
        opts: rvoip_sip_dialog::api::unified::SubscribeRequestOptions,
        auth_header_name: &str,
        auth_header_value: String,
    ) -> Result<rvoip_sip_core::Response> {
        self.send_standalone_request(
            StandaloneRequestOptions::Subscribe {
                target: target.to_string(),
                options: opts,
            },
            Some((auth_header_name, auth_header_value)),
        )
        .await
    }

    /// Compatibility facade for cancelling the exact pending INVITE.
    pub async fn send_cancel(&self, session_id: &SessionId) -> Result<()> {
        self.send_cancel_with_options(session_id, CancelRequestOptions::default())
            .await
    }

    /// SIP_API_DESIGN_2 §7.1 — REGISTER dispatch through the new
    /// dialog-core options surface with policy validation and outbound-
    /// proxy routing. Both the public options method and the state-machine's
    /// canonical REGISTER attempt use this exact wire boundary.
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
        // An explicit options-level proxy is rendered structurally by
        // dialog-core. Only prepend the coordinator default when no explicit
        // route was materialized, otherwise the request would carry two Route
        // headers for the same hop.
        let default_proxy = opts
            .outbound_proxy_uri
            .is_none()
            .then_some(self.outbound_proxy_uri.as_ref())
            .flatten();
        opts.extra_headers = apply_outbound_extras_policy(
            rvoip_sip_core::types::Method::Register,
            opts.extra_headers,
            default_proxy,
        )?;
        self.dialog_api
            .send_register_with_options_and_route(opts)
            .await
            .map_err(|error| redacted_dialog_operation_error("send REGISTER", error))
    }

    /// Preserve the adapter options signature while routing CANCEL through
    /// the initiating-state YAML action and its one dialog transaction owner.
    pub async fn send_cancel_with_options(
        &self,
        session_id: &SessionId,
        opts: rvoip_sip_dialog::api::unified::CancelRequestOptions,
    ) -> Result<()> {
        let handle = self
            .store
            .lifecycle_handle(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.to_string()))?;
        self.dispatch_state_machine_options_exact(
            &handle,
            EventType::SendOutboundCancel,
            crate::state_machine::executor::PendingOptionsSlot::Cancel(Arc::new(opts)),
        )
        .await
    }

    pub(crate) async fn send_cancel_with_options_lane_owned(
        &self,
        session: &SessionState,
        opts: CancelRequestOptions,
    ) -> Result<()> {
        self.dispatch_cancel_with_options(session, opts).await
    }

    async fn dispatch_cancel_with_options(
        &self,
        session: &SessionState,
        mut opts: CancelRequestOptions,
    ) -> Result<()> {
        opts.extra_headers = apply_outbound_extras_policy(
            rvoip_sip_core::types::Method::Cancel,
            opts.extra_headers,
            self.outbound_proxy_uri.as_ref(),
        )?;
        let dialog_id = resolve_dialog_for_lane_owned_session(self.store.as_ref(), session)?;
        let _handle = session.lifecycle_handle.as_ref().ok_or_else(|| {
            SessionError::InvalidTransition(
                "SIP CANCEL exact session has no lifecycle authority".to_string(),
            )
        })?;

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
        self.send_info_with_options(
            session_id,
            InfoRequestOptions {
                content_type: content_type.to_string(),
                body: bytes::Bytes::copy_from_slice(body),
                ..Default::default()
            },
        )
        .await?;

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
        self.send_refer_with_options(
            session_id,
            ReferRequestOptions {
                refer_to: refer_to.to_string(),
                ..Default::default()
            },
        )
        .await?;

        tracing::info!(
            session = %session_id.0,
            target_present = !refer_to.is_empty(),
            target_bytes = refer_to.len(),
            "Sent REFER"
        );
        Ok(())
    }

    /// Read the negotiated RFC 4028 timer only for the dialog owned by the
    /// lane-held exact session. Dialog-core remains the authority for header
    /// parsing and refresher negotiation; session-core uses this immutable
    /// result solely to arm its generation-qualified lifecycle deadline.
    pub(crate) fn session_timer_settings_lane_owned(
        &self,
        session: &SessionState,
    ) -> Result<Option<(u32, bool)>> {
        let dialog_id = resolve_dialog_for_lane_owned_session(self.store.as_ref(), session)?;
        let dialog = self
            .dialog_api
            .dialog_manager()
            .core()
            .get_dialog(&dialog_id)
            .map_err(|_| {
                SessionError::InvalidTransition(
                    "RFC 4028 timer requires the exact dialog owner".to_string(),
                )
            })?;
        Ok(dialog
            .session_expires_secs
            .filter(|interval| *interval > 0)
            .map(|interval| (interval, dialog.is_session_refresher)))
    }

    /// Fetch the SIP-level dialog identity (`Call-ID`, `local_tag`, `remote_tag`)
    /// for a session. Returns `None` if the session has no dialog yet
    /// (e.g., the INVITE hasn't been sent) or the dialog was lost.
    ///
    /// Callers use this to construct a Replaces header value when driving
    /// attended transfer from a higher layer.
    pub async fn dialog_identity(&self, session_id: &SessionId) -> Result<Option<DialogIdentity>> {
        let handle = match self.store.lifecycle_handle(session_id) {
            Some(handle) => handle,
            None => return Ok(None),
        };
        self.dialog_identity_exact(&handle).await
    }

    /// Fetch dialog identity only for a captured session generation.
    pub(crate) async fn dialog_identity_exact(
        &self,
        handle: &SessionRegistryHandle,
    ) -> Result<Option<DialogIdentity>> {
        let dialog_id = match resolve_dialog_for_handle_exact(self.store.as_ref(), handle) {
            Some(dialog_id) => dialog_id,
            None => return Ok(None),
        };

        let dialog = match self.dialog_api.get_dialog_info(&dialog_id).await {
            Ok(d) => d,
            Err(_) => return Ok(None),
        };

        if resolve_dialog_for_handle_exact(self.store.as_ref(), handle).as_ref() != Some(&dialog_id)
        {
            return Ok(None);
        }

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
        self.send_reinvite_with_options(
            session_id,
            rvoip_sip_dialog::api::unified::ReInviteRequestOptions {
                sdp: Some(sdp),
                ..Default::default()
            },
        )
        .await
    }

    /// Clean up all mappings and resources for a session
    pub async fn cleanup_session(&self, session_id: &SessionId) -> Result<()> {
        let (handle, lane) = self.store.state_machine_lane(session_id).ok_or_else(|| {
            SessionError::SessionNotFound(format!(
                "Session {} has no current cleanup lane",
                session_id.0
            ))
        })?;
        let _state_machine_lane = lane.lock_owned().await;
        self.store
            .get_session_snapshot_exact(&handle)
            .map_err(|_| {
                SessionError::SessionNotFound(format!(
                    "Session {} exact cleanup lifetime is unavailable",
                    session_id.0
                ))
            })?;
        self.cleanup_session_exact_lane_owned(&handle).await
    }

    /// Retire only rvoip-sip's generation-qualified adapter copies for a
    /// failed inbound initial INVITE while preserving dialog-core's exact
    /// dialog and server transaction for its causal 503 fallback.
    ///
    /// This boundary must never call a lower dialog cleanup API. The caller
    /// returns a negative processing ACK only after this completes; dialog-
    /// core still owns `(dialog_id, transaction_id)` and retires that lower
    /// route after a classified Written/WireUnknown final response.
    pub(crate) async fn cleanup_failed_inbound_session_preserving_lower_route_exact(
        &self,
        handle: &SessionRegistryHandle,
    ) -> Result<()> {
        let lane = self
            .store
            .state_machine_lane_retained_exact(handle)
            .ok_or_else(|| {
                SessionError::SessionNotFound(format!(
                    "Session {} exact failed-inbound cleanup lane is unavailable",
                    handle.session_id().0
                ))
            })?;
        let _state_machine_lane = lane.lock_owned().await;
        self.store
            .get_session_retained_snapshot_exact(handle)
            .map_err(|_| {
                SessionError::SessionNotFound(format!(
                    "Session {} exact failed-inbound lifetime is unavailable",
                    handle.session_id().0
                ))
            })?;

        let session_id = handle.session_id();
        let dialog_id = self
            .store
            .registry()
            .get_dialog_retained_exact(handle)
            .map(Into::into);

        self.outgoing_invite_tx
            .remove_if(session_id, |_, current| current.handle == *handle);
        self.outbound_initial_invites
            .remove_if(session_id, |_, current| current.handle == *handle);
        if let Some(transaction) = self
            .outgoing_bye_tx
            .get(handle)
            .map(|current| current.value().clone())
        {
            self.clear_outgoing_bye_transaction(&transaction);
        }
        self.outgoing_bye_wait_intents.remove(handle);
        self.outgoing_bye_generation_watch.remove(handle);
        self.outbound_request_tracker.clear_exact(handle);

        if let Some(dialog_id) = dialog_id.as_ref() {
            let lane = self.data_message_dispatch_lanes.lane(dialog_id);
            self.data_message_dispatch_lanes
                .remove_exact(dialog_id, &lane);
            self.store
                .registry()
                .clear_dialog_handle_retained(handle, dialog_id.clone().into())
                .map_err(|_| {
                    SessionError::InternalError(
                        "failed-inbound registry retirement failed (class=lifecycle)".to_string(),
                    )
                })?;
        }

        Ok(())
    }

    /// Clean up only the dialog resources owned by one retained session
    /// lifetime. The exact store check happens before any lower mutation, so a
    /// delayed cleanup cannot target a later call that reused the raw ID.
    pub(crate) async fn cleanup_session_exact(&self, handle: &SessionRegistryHandle) -> Result<()> {
        let lane = self
            .store
            .state_machine_lane_retained_exact(handle)
            .ok_or_else(|| {
                SessionError::SessionNotFound(format!(
                    "Session {} exact cleanup lane is unavailable",
                    handle.session_id().0
                ))
            })?;
        let _state_machine_lane = lane.lock_owned().await;
        self.store
            .get_session_retained_snapshot_exact(handle)
            .map_err(|_| {
                SessionError::SessionNotFound(format!(
                    "Session {} exact cleanup lifetime is unavailable",
                    handle.session_id().0
                ))
            })?;
        self.cleanup_session_exact_lane_owned(handle).await
    }

    /// Execute exact dialog cleanup while the caller already owns this
    /// session's state-machine lane. State-table actions use this entry to
    /// avoid recursive acquisition; public and shutdown paths acquire and
    /// revalidate the same exact lane in their compatibility facades above.
    pub(crate) async fn cleanup_session_exact_lane_owned(
        &self,
        handle: &SessionRegistryHandle,
    ) -> Result<()> {
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
            .store
            .registry()
            .get_dialog_retained_exact(handle)
            .map(Into::into);
        let outgoing_invite = self
            .outgoing_invite_tx
            .get(session_id)
            .map(|entry| entry.value().clone())
            .filter(|transaction| {
                dialog_id
                    .as_ref()
                    .is_some_and(|dialog_id| transaction.matches(handle, dialog_id))
            });
        let outgoing_bye = self
            .outgoing_bye_tx
            .get(handle)
            .map(|entry| entry.value().clone())
            .filter(|transaction| transaction.handle == *handle);
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

        // Incomplete confirmation/auth work cannot outlive exact terminal
        // cleanup. Drop that owner before any lower cleanup suspension so its
        // retained supervisor wakes instead of sleeping until Timer F.
        //
        // Preserve an already-terminal completion, however. A fast peer
        // response can trigger this cleanup before SendBYE unwinds and before
        // the hangup supervisor enters its confirmation wait. The completion
        // is then the only exact handoff receipt proving the wire outcome; the
        // supervisor consumes and clears it as soon as dispatch unwinds.
        if let Some(transaction) = outgoing_bye.as_ref() {
            if outgoing_bye_cleanup_should_interrupt_waiter(
                transaction.completion.current_outcome(),
                self.outgoing_bye_wait_intents
                    .get(handle)
                    .is_some_and(|intent| intent.handle == *handle),
            ) {
                self.clear_outgoing_bye_transaction(transaction);
            }
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
            if let Some(lane) = data_message_lane.as_ref() {
                self.data_message_dispatch_lanes
                    .remove_exact(dialog_id, lane);
            }
        } else {
            self.cleanup_missing_total.fetch_add(1, Ordering::Relaxed);
        }

        if outgoing_invite.is_some_and(|transaction| {
            self.outgoing_invite_tx
                .remove_if(session_id, |_, mapped| {
                    mapped.handle == transaction.handle
                        && mapped.dialog_id == transaction.dialog_id
                        && mapped.transaction_id == transaction.transaction_id
                })
                .is_some()
        }) {
            self.cleanup_outgoing_invite_removed_total
                .fetch_add(1, Ordering::Relaxed);
        }
        self.outbound_request_tracker.clear_exact(handle);

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

    /// Send one REGISTER from an immutable options snapshot and return the
    /// exact snapshot used on the wire.
    ///
    /// This is the sole dialog-owned REGISTER implementation. Initial sends,
    /// manual and automatic refreshes, authentication retries, 423 retries,
    /// and the compatibility actions all enter here with the same options
    /// shape. The state-machine remains responsible for lifecycle decisions
    /// and retry limits; this method owns request identity, Digest mechanics,
    /// transport selection, and response classification.
    pub(crate) async fn send_register_attempt(
        &self,
        session_id: &SessionId,
        mut options: RegisterRequestOptions,
        auth: Option<&crate::auth::SipClientAuth>,
        mut context: RegisterAttemptContext,
    ) -> Result<RegisterAttemptResult> {
        let registrar_uri = options.registrar_uri.clone();
        let from_uri = options.aor_uri.clone();
        let requested_contact_uri = options.contact_uri.clone();
        let expires = options.expires;
        tracing::info!(
            session_present = !session_id.0.is_empty(),
            session_bytes = session_id.0.len(),
            registrar_present = !options.registrar_uri.is_empty(),
            registrar_bytes = registrar_uri.len(),
            aor_present = !options.aor_uri.is_empty(),
            aor_bytes = from_uri.len(),
            contact_present = !options.contact_uri.is_empty(),
            contact_bytes = options.contact_uri.len(),
            expires,
            refresh = options.refresh,
            "outbound REGISTER starting"
        );

        // Build authorization header if auth material provided.
        let (authorization, proxy_authorization) = if let Some(auth) = auth {
            let auth_state = context
                .auth_challenge_raw
                .clone()
                .or_else(|| {
                    context.auth_challenge.as_ref().map(|challenge| {
                        rvoip_auth_core::DigestAuthenticator::new(challenge.realm.clone())
                            .format_www_authenticate(challenge)
                    })
                })
                .map(|challenge_raw| {
                    let digest_challenge = context.auth_challenge.clone().or_else(|| {
                        rvoip_auth_core::DigestAuthenticator::parse_challenge(&challenge_raw).ok()
                    });
                    let nc_value = if let Some(challenge) = digest_challenge.as_ref() {
                        let nc_key = (challenge.realm.clone(), challenge.nonce.clone());
                        *context
                            .digest_nc
                            .entry(nc_key)
                            .and_modify(|n| *n += 1)
                            .or_insert(1)
                    } else {
                        1
                    };
                    let transport_context = context
                        .pending_auth_transport
                        .take()
                        .unwrap_or_else(|| self.outbound_transport_context_for_uri(&registrar_uri));
                    let status = context.pending_auth_status.unwrap_or(401);
                    (challenge_raw, nc_value, transport_context, status)
                });
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
                        &registrar_uri,
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
                    (options.authorization.clone(), Some(selected.value))
                } else {
                    (Some(selected.value), options.proxy_authorization.clone())
                }
            } else {
                tracing::debug!("No challenge stored, sending without auth");
                (
                    options.authorization.clone(),
                    options.proxy_authorization.clone(),
                )
            }
        } else {
            (
                options.authorization.clone(),
                options.proxy_authorization.clone(),
            )
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
            let rewritten = rewrite_contact_host(&requested_contact_uri, public);
            if rewritten != requested_contact_uri {
                tracing::info!(
                    contact_present = !requested_contact_uri.is_empty(),
                    contact_bytes = requested_contact_uri.len(),
                    rewritten_contact_present = !rewritten.is_empty(),
                    rewritten_contact_bytes = rewritten.len(),
                    public_address_family = if public.is_ipv4() { "ipv4" } else { "ipv6" },
                    public_port = public.port(),
                    "outbound REGISTER Contact rewritten from NAT discovery"
                );
            }
            rewritten
        } else {
            requested_contact_uri.clone()
        };

        // Reserve registration identity for this new logical REGISTER
        // transaction. This is registration-scoped only; dialog-core still owns
        // all in-dialog CSeq state and transaction-layer retransmissions reuse
        // the request created below.
        let registration_call_id = options
            .call_id
            .clone()
            .or_else(|| context.registration_call_id.clone())
            .unwrap_or_else(|| format!("reg-{}", uuid::Uuid::new_v4()));
        context.registration_call_id = Some(registration_call_id.clone());

        // A builder may reserve the next CSeq before it enters the lane. Use
        // that value when it is newer than the committed registration CSeq;
        // otherwise this is an auth/423/automatic retry and must advance once.
        let requested_cseq = options.cseq.unwrap_or_default();
        let registration_cseq = if requested_cseq > context.registration_cseq {
            requested_cseq
        } else {
            context.registration_cseq.saturating_add(1).max(1)
        };
        context.registration_cseq = registration_cseq;

        options.contact_uri = rewritten_contact;
        options.authorization = authorization;
        options.proxy_authorization = proxy_authorization;
        options.call_id = Some(registration_call_id);
        options.cseq = Some(registration_cseq);
        if options.outbound_contact.is_none() {
            options.outbound_contact = self.outbound_contact_params.clone();
        }
        if options.outbound_proxy_uri.is_none() {
            options.outbound_proxy_uri = self.outbound_proxy_uri.clone();
        }

        // Send REGISTER through dialog-core API and get response.
        // A5 Phase 2a: when the coordinator is configured for RFC 5626 SIP
        // Outbound, route through the outbound-aware REGISTER so the Contact
        // carries `+sip.instance` + `reg-id` + `;ob`.
        let (response, register_route) = self
            .send_register_with_options_and_route(options.clone())
            .await
            .map_err(|error| redacted_dialog_operation_error("send REGISTER", error))?;

        tracing::info!(
            "REGISTER response received: {} for session {}",
            response.status_code(),
            session_id.0
        );

        context.pending_auth_transport = self.register_response_transport_context(&response);

        let mut outcome = Self::register_attempt_outcome_from_response(
            &response,
            &requested_contact_uri,
            expires,
        );
        if let RegisterAttemptOutcome::Registered { metadata, .. } = &mut outcome {
            metadata.transport_route = Some(register_route);
        }

        Ok(RegisterAttemptResult {
            outcome,
            context,
            request_options: options,
        })
    }

    pub async fn send_subscribe(
        &self,
        session_id: &SessionId,
        from_uri: &str,
        to_uri: &str,
        event_package: &str,
        expires: u32,
    ) -> Result<Option<EventType>> {
        tracing::info!(
            "Sending SUBSCRIBE for session {} from {} to {} for event {}",
            session_id.0,
            from_uri,
            to_uri,
            event_package
        );

        // Externally configured state tables may still select SendSUBSCRIBE.
        // Keep that action as a facade over the same standalone request
        // implementation used by the public builder.
        let response = self
            .send_standalone_request(
                StandaloneRequestOptions::Subscribe {
                    target: to_uri.to_string(),
                    options: SubscribeRequestOptions {
                        event: event_package.to_string(),
                        expires,
                        from_uri: Some(from_uri.to_string()),
                        contact_uri: Some(from_uri.to_string()),
                        ..Default::default()
                    },
                },
                None,
            )
            .await?;

        tracing::info!(
            "SUBSCRIBE response: {} for session {}",
            response.status_code(),
            session_id.0
        );

        // Return the response-driven follow-up to the state-machine queue.
        // Publishing it synchronously through the global bus would re-enter
        // this exact session while its complete-event lane is still held.
        let follow_up = if response.status_code() == 200 || response.status_code() == 202 {
            Some(EventType::SubscriptionAccepted)
        } else if response.status_code() >= 400 {
            Some(EventType::SubscriptionFailed(response.status_code()))
        } else {
            None
        };

        Ok(follow_up)
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

        self.send_notify_with_options(
            session_id,
            NotifyRequestOptions {
                event: event_package.to_string(),
                subscription_state: subscription_state.unwrap_or_default(),
                body: body.map(bytes::Bytes::from),
                ..Default::default()
            },
        )
        .await?;

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
        let Some((handle, _lane, session)) =
            lock_and_load_exact_current_session(self.store.as_ref(), session_id).await
        else {
            return Err(SessionError::SessionNotFound(session_id.0.clone()));
        };

        self.send_refer_notify_lane_owned(&handle, &session, status_code, reason)
            .await
    }

    /// Send one REFER progress NOTIFY while the caller owns the exact
    /// session's state-machine lane.
    ///
    /// State-machine actions must use this entry point instead of the public
    /// raw-ID facade above. Requiring both the retained registry handle and
    /// its lane-owned snapshot prevents a delayed transfer callback from
    /// crossing session-ID reuse into a replacement dialog.
    pub(crate) async fn send_refer_notify_lane_owned(
        &self,
        handle: &SessionRegistryHandle,
        session: &SessionState,
        status_code: u16,
        reason: &str,
    ) -> Result<()> {
        if session.lifecycle_handle.as_ref() != Some(handle)
            || &session.session_id != handle.session_id()
        {
            return Err(SessionError::InvalidTransition(
                "REFER NOTIFY requires the matching exact session authority".to_string(),
            ));
        }

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
        let lease = self
            .outbound_request_tracker
            .prepare(handle, TrackedInDialogOptions::Notify(Arc::clone(&options)))?;
        let transaction_id = self
            .send_notify_with_options_lane_owned(session, (*options).clone())
            .await
            .map_err(|error| redacted_dialog_operation_error("REFER NOTIFY", error))?;
        self.outbound_request_tracker
            .activate(lease, transaction_id)?;

        tracing::info!(
            "REFER NOTIFY sent successfully for session {}",
            handle.session_id().0
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
        self.send_data_message_on_dialog_driver(
            dialog_id,
            message,
            DataMessageStateLane::AcquireExact {
                expected_handle: None,
            },
        )
        .await
    }

    /// Canonical in-dialog MESSAGE owner.
    ///
    /// Both the byte-preserving data-message API and the legacy `String`
    /// facade enter this driver. It alone materializes the request, allocates
    /// CSeq, writes to the transaction layer, observes the final response, and
    /// performs the single bounded fresh-challenge retry.
    async fn send_data_message_on_dialog_driver(
        &self,
        dialog_id: &RvoipDialogId,
        message: SipDataMessage,
        mut state_lane: DataMessageStateLane<'_>,
    ) -> Result<()> {
        let manager = self.dialog_api.dialog_manager().core();
        let mut fresh_challenge = None;

        // One initial attempt plus one bounded retry for a fresh 401/407.
        // Creating a new dialog template on each pass advances CSeq as RFC
        // 3261 requires for the challenged replacement request.
        for attempt in 0..=1 {
            // DNS is intentionally outside both serialization lanes. Capture
            // the dialog routing inputs, resolve them, then acquire exact
            // state -> MESSAGE and revalidate the same route before allocating
            // a CSeq. A target refresh restarts this preflight without consuming
            // a sequence number; cleanup can win at any point and makes the
            // later exact-state resolution fail closed.
            let (mut attempt_state, dispatch_guard, mut request, next_hop, candidates) = 'route_preflight: loop {
                let route_snapshot = manager.get_dialog(dialog_id).map_err(|_| {
                    SessionError::InvalidTransition(
                        "SIP MESSAGE exact dialog is no longer available".to_string(),
                    )
                })?;
                if route_snapshot.state != DialogState::Confirmed {
                    return Err(SessionError::InvalidTransition(
                        "SIP MESSAGE requires a confirmed dialog".to_string(),
                    ));
                }
                let expected_target = route_snapshot.remote_target;
                let expected_routes = route_snapshot.route_set;
                let expected_next_hop = expected_routes
                    .first()
                    .cloned()
                    .unwrap_or_else(|| expected_target.clone());
                let candidates = manager.resolve_uri_to_candidates(&expected_next_hop).await;
                if candidates.is_empty() {
                    return Err(SessionError::DialogError(
                        "SIP MESSAGE exact next hop is unavailable".to_string(),
                    ));
                }

                // Cleanup uses this same exact state -> MESSAGE order. The
                // state lane remains owned only through active-dialog and auth
                // publication; the MESSAGE lane owns this attempt through its
                // wire/final-response outcome.
                let attempt_state = match &mut state_lane {
                    DataMessageStateLane::AcquireExact { expected_handle } => {
                        let (handle, guard, session) = lock_and_load_exact_data_message_session(
                            self.store.as_ref(),
                            dialog_id,
                        )
                        .await?;
                        match expected_handle.as_ref() {
                            Some(expected) if expected != &handle => {
                                return Err(SessionError::InvalidTransition(
                                    "SIP MESSAGE exact session generation changed before dispatch"
                                        .to_string(),
                                ));
                            }
                            None => *expected_handle = Some(handle.clone()),
                            Some(_) => {}
                        }
                        DataMessageAttemptState::AcquiredExact {
                            handle,
                            _guard: guard,
                            session: Box::new(session),
                        }
                    }
                    DataMessageStateLane::AlreadyOwned(session) => {
                        let exact_dialog =
                            resolve_dialog_for_lane_owned_session(self.store.as_ref(), session)?;
                        if &exact_dialog != dialog_id {
                            return Err(SessionError::InvalidTransition(
                                "SIP MESSAGE lane-owned dialog identity changed before dispatch"
                                    .to_string(),
                            ));
                        }
                        let handle = session.lifecycle_handle.clone().ok_or_else(|| {
                            SessionError::InvalidTransition(
                                "SIP MESSAGE lane-owned session has no lifecycle authority"
                                    .to_string(),
                            )
                        })?;
                        DataMessageAttemptState::AlreadyOwned { handle, session }
                    }
                };
                let dispatch_lane = self.data_message_dispatch_lanes.lane(dialog_id);
                let dispatch_guard = Arc::clone(&dispatch_lane).lock_owned().await;
                let template = {
                    let mut dialog = match manager.get_dialog_mut(dialog_id) {
                        Ok(dialog) => dialog,
                        Err(_) => {
                            // Cleanup may have removed the original lane before
                            // this already-captured sender entered. Remove only
                            // this exact replacement lane; a concurrently
                            // installed successor remains independently owned.
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
                    if dialog.remote_target != expected_target
                        || dialog.route_set != expected_routes
                    {
                        drop(dialog);
                        drop(dispatch_guard);
                        drop(attempt_state);
                        continue 'route_preflight;
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
                                "SIP MESSAGE confirmed dialog is missing its remote tag"
                                    .to_string(),
                            )
                        })?;
                    let local_address = manager.local_address_for_target_and_routes(
                        &template.target_uri,
                        &template.route_set,
                    );
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

                let request = build_sip_data_request(&template, message.clone()).map_err(|_| {
                    SessionError::InvalidInput(
                        "SIP data message failed local request construction".to_string(),
                    )
                })?;
                let next_hop = exact_next_hop_uri_for_request(&request).map_err(|_| {
                    SessionError::InvalidInput(
                        "SIP data message has an unusable exact next hop".to_string(),
                    )
                })?;
                if next_hop != expected_next_hop {
                    return Err(SessionError::InvalidTransition(
                        "SIP MESSAGE exact route changed during request construction".to_string(),
                    ));
                }
                break 'route_preflight (
                    attempt_state,
                    dispatch_guard,
                    request,
                    next_hop,
                    candidates,
                );
            };

            let next_hop_text = next_hop.to_string();
            let transport = self.outbound_transport_context_for_uri(&next_hop_text);
            match &mut attempt_state {
                DataMessageAttemptState::AcquiredExact {
                    handle, session, ..
                } => authorize_data_message_lane_owned_exact(
                    self.store.as_ref(),
                    handle,
                    session,
                    dialog_id,
                    &mut request,
                    &next_hop_text,
                    fresh_challenge.as_ref(),
                    &transport,
                )?,
                DataMessageAttemptState::AlreadyOwned { handle, session } => {
                    if self
                        .store
                        .registry()
                        .get_dialog_handle_exact(handle)
                        .as_ref()
                        != Some(&dialog_id.clone().into())
                    {
                        return Err(SessionError::InvalidTransition(
                            "SIP MESSAGE lane-owned exact dialog changed during authentication"
                                .to_string(),
                        ));
                    }
                    let request_uri = request.uri.to_string();
                    let headers = mutate_retained_dialog_auth(
                        session,
                        dialog_id,
                        RetainedDialogAuthMode::DataMessage {
                            fresh_challenge: fresh_challenge.as_ref(),
                        },
                        "MESSAGE",
                        &request_uri,
                        RetainedDialogAuthRoute {
                            next_hop: &next_hop_text,
                            transport: &transport,
                        },
                        Some(request.body.as_ref()),
                    )?;
                    request.headers.extend(headers);
                }
            }

            // The exact-locking facade releases its acquired state lane before
            // wire I/O. The state-machine facade merely releases this borrow;
            // its caller necessarily retains the already-owned executor lane.
            // Both retain the MESSAGE lane through the final-response outcome.
            drop(attempt_state);
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
                    let challenge = DataMessageAuthChallenge {
                        status,
                        value: values.join(", "),
                    };
                    // A retry releases this attempt's MESSAGE lane before
                    // returning to the top of the loop. Exact-locking callers
                    // have also released their acquired state lane; an
                    // executor caller still owns the lane it lent us and
                    // revalidates its exact mapping on the next pass.
                    drop(dispatch_guard);
                    fresh_challenge = Some(challenge);
                    continue;
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

    /// Send a MESSAGE request (can be in-dialog or out-of-dialog)
    pub async fn send_message(
        &self,
        session_id: &SessionId,
        from_uri: &str,
        to_uri: &str,
        body: String,
        in_dialog: bool,
    ) -> Result<Option<EventType>> {
        tracing::info!(
            "Sending MESSAGE for session {} from {} to {} (in_dialog: {})",
            session_id.0,
            from_uri,
            to_uri,
            in_dialog
        );

        if in_dialog {
            let Some((handle, lane, session)) =
                lock_and_load_exact_current_session(self.store.as_ref(), session_id).await
            else {
                return Err(SessionError::DialogError(
                    "No dialog for session".to_string(),
                ));
            };
            let dialog_id = resolve_dialog_for_lane_owned_session(self.store.as_ref(), &session)
                .map_err(|_| SessionError::DialogError("No dialog for session".to_string()))?;
            drop(lane);
            self.send_data_message_on_dialog_driver(
                &dialog_id,
                legacy_text_data_message(body),
                DataMessageStateLane::AcquireExact {
                    expected_handle: Some(handle),
                },
            )
            .await?;
            return Ok(None);
        }

        self.send_message_out_of_dialog_legacy(session_id, from_uri, to_uri, body)
            .await
    }

    pub(crate) async fn send_message_lane_owned(
        &self,
        session: &mut SessionState,
        from_uri: &str,
        to_uri: &str,
        body: String,
        in_dialog: bool,
    ) -> Result<Option<EventType>> {
        if in_dialog {
            self.send_message_in_dialog_lane_owned(session, body).await
        } else {
            self.send_message_out_of_dialog_legacy(&session.session_id, from_uri, to_uri, body)
                .await
        }
    }

    async fn send_message_in_dialog_lane_owned(
        &self,
        session: &mut SessionState,
        body: String,
    ) -> Result<Option<EventType>> {
        let dialog_id = resolve_dialog_for_lane_owned_session(self.store.as_ref(), session)
            .map_err(|_| SessionError::DialogError("No dialog for session".to_string()))?;
        self.send_data_message_on_dialog_driver(
            &dialog_id,
            legacy_text_data_message(body),
            DataMessageStateLane::AlreadyOwned(session),
        )
        .await?;
        tracing::info!(
            "MESSAGE sent successfully for session {}",
            session.session_id.0
        );
        Ok(None)
    }

    async fn send_message_out_of_dialog_legacy(
        &self,
        session_id: &SessionId,
        from_uri: &str,
        to_uri: &str,
        body: String,
    ) -> Result<Option<EventType>> {
        // Externally configured state tables may still select SendMESSAGE.
        // Keep that action as a facade over the same standalone request
        // implementation used by the public builder.
        let response = self
            .send_standalone_request(
                StandaloneRequestOptions::Message(MessageRequestOptions {
                    from_uri: from_uri.to_string(),
                    to_uri: to_uri.to_string(),
                    content_type: String::from("text/plain"),
                    body: bytes::Bytes::from(body),
                    ..Default::default()
                }),
                None,
            )
            .await?;

        let follow_up = if response.status_code() == 200 {
            Some(EventType::MessageDelivered)
        } else if response.status_code() >= 400 {
            Some(EventType::MessageFailed(response.status_code()))
        } else {
            None
        };
        tracing::info!("MESSAGE sent successfully for session {}", session_id.0);
        Ok(follow_up)
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
            outgoing_invite_tx: self.outgoing_invite_tx.clone(),
            outgoing_bye_tx: self.outgoing_bye_tx.clone(),
            outgoing_bye_generation_watch: self.outgoing_bye_generation_watch.clone(),
            outgoing_bye_wait_intents: self.outgoing_bye_wait_intents.clone(),
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
            #[cfg(test)]
            registration_refresh_dispatch_pause: self.registration_refresh_dispatch_pause.clone(),
            cleanup_attempt_total: self.cleanup_attempt_total.clone(),
            cleanup_mapped_total: self.cleanup_mapped_total.clone(),
            cleanup_missing_total: self.cleanup_missing_total.clone(),
            cleanup_outgoing_invite_removed_total: self
                .cleanup_outgoing_invite_removed_total
                .clone(),
            trace_redactor: self.trace_redactor.clone(),
        }
    }
}

#[cfg(test)]
// Policy helpers remain below this focused diagnostic module so the production
// API stays grouped with its documentation.
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use crate::state_table::Role;

    #[test]
    fn failed_inbound_adapter_cleanup_preserves_lower_response_authority() {
        let source = include_str!("dialog_adapter.rs");
        let cleanup = source
            .split(
                "    pub(crate) async fn cleanup_failed_inbound_session_preserving_lower_route_exact(",
            )
            .nth(1)
            .and_then(|tail| tail.split("    /// Clean up only the dialog resources").next())
            .expect("failed-inbound preserving cleanup source");

        for forbidden in [
            "cleanup_dialog_storage_and_transactions",
            "cleanup_transaction_receiver",
            "terminate_transaction",
            "finish_initial_invite_teardown",
            "remove_dialog_storage",
        ] {
            assert!(
                !cleanup.contains(forbidden),
                "upper failed-inbound cleanup retired lower response authority via {forbidden}"
            );
        }
        assert!(cleanup.contains("clear_dialog_handle_retained"));
        assert!(cleanup.contains("outbound_request_tracker.clear_exact"));
        assert!(cleanup.contains("data_message_dispatch_lanes"));
    }

    fn transfer_observation(id: usize) -> crate::api::events::Event {
        crate::api::events::Event::ReferProgress {
            call_id: SessionId(format!("transfer-observer-{id}")),
            status_code: 180,
            reason: "Ringing".to_string(),
        }
    }

    #[tokio::test]
    async fn post_commit_transfer_observation_ignores_absent_and_full_observers() {
        use rvoip_infra_common::events::EventCoordinatorConfig;

        let absent = Arc::new(
            GlobalEventCoordinator::new(
                EventCoordinatorConfig::monolithic().with_channel_capacity(1),
            )
            .await
            .expect("create coordinator without observer"),
        );
        tokio::time::timeout(
            Duration::from_secs(1),
            spawn_api_event_observation(Arc::clone(&absent), transfer_observation(1)),
        )
        .await
        .expect("absent observer blocked detached publication")
        .expect("absent-observer publication task panicked");
        absent.shutdown().await.expect("shutdown absent fixture");

        let saturated = Arc::new(
            GlobalEventCoordinator::new(
                EventCoordinatorConfig::monolithic().with_channel_capacity(1),
            )
            .await
            .expect("create saturated-observer coordinator"),
        );
        let observer = saturated
            .subscribe("session_to_app")
            .await
            .expect("subscribe bounded app observer");
        tokio::time::timeout(
            Duration::from_secs(1),
            spawn_api_event_observation(Arc::clone(&saturated), transfer_observation(2)),
        )
        .await
        .expect("first observation publication stalled")
        .expect("first observation task panicked");
        tokio::time::timeout(Duration::from_secs(1), async {
            while observer.is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("first observation never filled bounded observer queue");
        assert_eq!(observer.len(), 1, "observer queue must be full");

        for id in 3..=4 {
            tokio::time::timeout(
                Duration::from_secs(1),
                spawn_api_event_observation(Arc::clone(&saturated), transfer_observation(id)),
            )
            .await
            .expect("full observer affected detached publication")
            .expect("full-observer publication task panicked");
        }
        assert_eq!(observer.len(), 1, "test must retain observer pressure");

        drop(observer);
        saturated
            .shutdown()
            .await
            .expect("shutdown saturated fixture");
    }

    #[test]
    fn fast_register_200_updates_one_lane_owned_state() {
        let mut session = SessionState::new(SessionId::new(), Role::UAC);
        session.registration_retry_count = 2;
        session.registration_last_failure = Some("old failure".into());
        session.local_sdp = Some("v=0\r\n".into());
        let now = Instant::now();

        let effect = record_registration_success_state(
            &mut session,
            RegistrationSuccessStateInput {
                registrar_uri: "sip:registrar.example.test",
                from_uri: "sip:alice@example.test",
                contact_uri: "sip:alice@192.0.2.10:5060",
                accepted_expires: 600,
                now,
                next_refresh_at: None,
                metadata: RegistrationResponseMetadata {
                    service_route: Some(vec!["sip:edge.example.test;lr".into()]),
                    pub_gruu: Some("sip:alice@example.test;gr=public".into()),
                    temp_gruu: Some("sip:opaque@example.test;gr=temp".into()),
                    transport_route: None,
                },
            },
        );

        assert!(session.is_registered);
        assert_eq!(session.registration_expires, Some(600));
        assert_eq!(session.registration_accepted_expires, Some(600));
        assert_eq!(session.registration_registered_at, Some(now));
        assert_eq!(session.registration_retry_count, 0);
        assert!(session.registration_last_failure.is_none());
        assert_eq!(session.local_sdp.as_deref(), Some("v=0\r\n"));
        assert!(matches!(
            effect,
            RegistrationPostCommitEffect::Registered {
                accepted_expires: 600,
                ..
            }
        ));
    }

    #[test]
    fn register_wire_helper_has_no_session_store_writer() {
        let source = include_str!("dialog_adapter.rs");
        let helper = source
            .split("pub(crate) async fn send_register_attempt(")
            .nth(1)
            .and_then(|tail| tail.split("pub async fn send_subscribe(").next())
            .expect("send_register_attempt source");
        assert!(!helper.contains("self.store"));
        assert!(!helper.contains("update_session_with"));
    }

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
    fn bye_final_response_classifies_crossed_481_as_graceful_terminal() {
        assert_eq!(
            classify_outgoing_bye_final_response(200),
            OutgoingByeFinalDisposition::Confirmed
        );
        assert_eq!(
            classify_outgoing_bye_final_response(299),
            OutgoingByeFinalDisposition::Confirmed
        );
        assert_eq!(
            classify_outgoing_bye_final_response(401),
            OutgoingByeFinalDisposition::AuthenticationChallenge
        );
        assert_eq!(
            classify_outgoing_bye_final_response(407),
            OutgoingByeFinalDisposition::AuthenticationChallenge
        );
        assert_eq!(
            classify_outgoing_bye_final_response(481),
            OutgoingByeFinalDisposition::PeerAlreadyTerminated
        );
        for rejected in [300, 403, 408, 480, 482, 500, 603] {
            assert_eq!(
                classify_outgoing_bye_final_response(rejected),
                OutgoingByeFinalDisposition::Rejected,
                "unexpected graceful classification for {rejected}"
            );
        }
    }

    #[test]
    fn recorded_bye_success_survives_cleanup_before_waiter_lookup() {
        let outcome = Ok(Some(ClientTransactionOutcome::FinalResponse(
            rvoip_sip_core::Response::new(rvoip_sip_core::StatusCode::Ok),
        )));

        assert!(!outgoing_bye_cleanup_should_interrupt_waiter(outcome, true));
        assert!(outgoing_bye_cleanup_should_interrupt_waiter(Ok(None), true));
        assert!(outgoing_bye_cleanup_should_interrupt_waiter(
            Ok(Some(ClientTransactionOutcome::FinalResponse(
                rvoip_sip_core::Response::new(rvoip_sip_core::StatusCode::Ok),
            ))),
            false,
        ));
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
    async fn initial_invite_publishes_exact_dialog_before_wire_dispatch() {
        let store = SessionStore::new();
        let session_id = SessionId::new();
        let created = store
            .create_session(session_id.clone(), crate::state_table::Role::UAC, false)
            .await
            .expect("create exact initial-INVITE session");
        let handle = created
            .lifecycle_handle
            .clone()
            .expect("initial INVITE has exact lifecycle handle");
        let dialog_id = RvoipDialogId::new();

        publish_initial_invite_dialog_exact(&store, &handle, &dialog_id)
            .expect("publish exact dialog before dispatch");
        let published = store
            .get_session_snapshot(&session_id)
            .await
            .expect("read pre-wire session publication");
        assert_eq!(
            published
                .dialog_id
                .as_ref()
                .map(crate::types::DialogId::as_uuid),
            Some(&dialog_id.0)
        );

        publish_initial_invite_dialog_exact(&store, &handle, &dialog_id)
            .expect("same exact dialog publication is idempotent");
        let replacement = RvoipDialogId::new();
        assert!(matches!(
            publish_initial_invite_dialog_exact(&store, &handle, &replacement),
            Err(SessionError::InvalidTransition(_))
        ));
        assert_eq!(
            store
                .get_session_snapshot(&session_id)
                .await
                .expect("read exact dialog after rejected replacement")
                .dialog_id
                .as_ref()
                .map(crate::types::DialogId::as_uuid),
            Some(&dialog_id.0)
        );

        store
            .registry()
            .map_dialog_exact(
                handle.key(),
                handle.slot_revision(),
                dialog_id.clone().into(),
            )
            .expect("install initial exact registry dialog");
        assert!(store
            .registry()
            .clear_dialog_handle_retained(&handle, dialog_id.clone().into())
            .expect("retire initial exact registry dialog"));
        store
            .registry()
            .map_dialog_exact(
                handle.key(),
                handle.slot_revision(),
                replacement.clone().into(),
            )
            .expect("install registry-proven redirect replacement");
        publish_initial_invite_dialog_exact(&store, &handle, &replacement)
            .expect("adopt registry-proven redirect replacement");
        assert_eq!(
            store
                .get_session_snapshot(&session_id)
                .await
                .expect("read registry-proven replacement")
                .dialog_id
                .as_ref()
                .map(crate::types::DialogId::as_uuid),
            Some(&replacement.0)
        );
    }

    #[tokio::test]
    async fn invite_retry_exact_resolver_uses_registry_without_compatibility_map() {
        let store = SessionStore::new();
        let session_id = SessionId::new();
        let created = store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create exact INVITE-retry session");
        let handle = created
            .lifecycle_handle
            .clone()
            .expect("INVITE-retry exact handle");

        assert!(matches!(
            resolve_exact_invite_retry_dialog(&store, &session_id),
            Err(SessionError::SessionNotFound(_))
        ));

        // No DialogAdapter (and therefore no compatibility DashMap) exists in
        // this fixture. The exact registry mapping is sufficient by design.
        let dialog_id = RvoipDialogId::new();
        store
            .registry()
            .map_dialog_handle(&handle, dialog_id.clone().into())
            .expect("install canonical exact dialog mapping");
        assert_eq!(
            resolve_exact_invite_retry_dialog(&store, &session_id)
                .expect("resolve exact registry-owned dialog"),
            dialog_id
        );
    }

    #[tokio::test]
    async fn invite_retry_exact_resolver_cannot_cross_raw_id_reuse() {
        let store = SessionStore::new();
        let session_id = SessionId("invite-retry-reused-session".to_string());
        let generation_a = store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create INVITE-retry generation A");
        let handle_a = generation_a
            .lifecycle_handle
            .clone()
            .expect("generation A exact handle");
        let stale_key = handle_a.key().clone();
        let stale_revision = handle_a.slot_revision();
        let dialog_a = RvoipDialogId::new();
        store
            .registry()
            .map_dialog_handle(&handle_a, dialog_a.clone().into())
            .expect("map generation A dialog");
        assert_eq!(
            resolve_exact_invite_retry_dialog(&store, &session_id)
                .expect("resolve generation A dialog"),
            dialog_a
        );

        store
            .remove_session_exact(&handle_a)
            .await
            .expect("retire INVITE-retry generation A");
        assert!(matches!(
            resolve_exact_invite_retry_dialog(&store, &session_id),
            Err(SessionError::SessionNotFound(_))
        ));
        drop(handle_a);
        drop(generation_a);
        assert!(store.authority().elapse_reuse_horizon_for_test(&session_id));

        let generation_b = store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create INVITE-retry generation B");
        let handle_b = generation_b
            .lifecycle_handle
            .clone()
            .expect("generation B exact handle");
        let dialog_b = RvoipDialogId::new();
        store
            .registry()
            .map_dialog_handle(&handle_b, dialog_b.clone().into())
            .expect("map generation B dialog");

        assert!(store
            .registry()
            .get_dialog_exact(&stale_key, stale_revision)
            .is_none());
        assert_eq!(
            resolve_exact_invite_retry_dialog(&store, &session_id)
                .expect("raw ID resolves only current generation B"),
            dialog_b
        );
    }

    #[test]
    fn invite_retry_resolution_has_no_polling_or_compatibility_fallback() {
        let source = include_str!("dialog_adapter.rs");
        let resolver = rust_function_source(source, "resolve_exact_invite_retry_dialog");
        assert_eq!(resolver.matches(".lifecycle_handle(").count(), 1);
        assert_eq!(
            resolver.matches("resolve_dialog_for_handle_exact(").count(),
            1
        );
        assert!(!resolver.contains("session_to_dialog"));

        for (retry, lower_wire) in [
            ("resend_invite_with_auth", ".send_invite_with_auth_options("),
            (
                "resend_invite_with_session_timer_override",
                ".send_invite_with_session_timer_options(",
            ),
        ] {
            let body = rust_function_source(source, retry);
            assert_eq!(
                body.matches("resolve_exact_invite_retry_dialog(").count(),
                1,
                "{retry} must resolve its exact dialog once"
            );
            for forbidden in [
                "session_to_dialog",
                "tokio::time::sleep",
                "Duration::",
                "Instant::",
                "loop {",
            ] {
                assert!(
                    !body.contains(forbidden),
                    "{retry} regained obsolete polling/fallback token {forbidden}"
                );
            }
            let exact_resolution = body
                .find("resolve_exact_invite_retry_dialog(")
                .expect("exact retry resolution");
            let wire = body.find(lower_wire).expect("retry lower wire dispatch");
            assert!(exact_resolution < wire);
        }

        let initial_dispatch = rust_function_source(source, "send_initial_invite_staged");
        let install = initial_dispatch
            .find("install_adapter_bindings")
            .expect("exact mapping installation");
        let publish = initial_dispatch
            .find("publish_initial_invite_dialog_exact")
            .expect("exact state publication");
        let wire = initial_dispatch
            .find("dispatch_initial_invite")
            .expect("initial/redirect wire dispatch");
        assert!(
            install < publish && publish < wire,
            "initial and redirect INVITEs must install mapping, publish state, then dispatch wire"
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
        let session_id = SessionId::new();
        let created = adapter
            .store
            .create_session(session_id, Role::UAC, false)
            .await
            .expect("refresh session");
        let handle = created
            .lifecycle_handle
            .clone()
            .expect("refresh exact handle");
        adapter
            .schedule_registration_refresh(handle, Some(Instant::now() + Duration::from_secs(60)));
        assert_eq!(adapter.registration_refresh_tasks.len(), 1);
        assert_eq!(adapter.registration_refresh_retained.count(), 1);
        assert_eq!(
            adapter.perf_diagnostic_counts()["registration_refresh_retained_tasks"],
            1
        );

        adapter
            .abort_all_registration_refreshes_and_wait()
            .await
            .expect("cooperative refresh cancellation drained");
        assert!(adapter.registration_refresh_tasks.is_empty());
        assert_eq!(adapter.registration_refresh_retained.count(), 0);
        assert!(!adapter.registration_refresh_retained.panicked());
        assert_eq!(
            adapter.perf_diagnostic_counts()["registration_refresh_retained_tasks"],
            0
        );

        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("coordinator shutdown remains idempotent");
    }

    #[tokio::test]
    async fn paused_refresh_cannot_target_reused_session_generation() {
        let coordinator = crate::api::unified::UnifiedCoordinator::new(
            crate::api::unified::Config::local("refresh-generation", 0),
        )
        .await
        .expect("coordinator");
        let adapter = Arc::clone(coordinator.dialog_adapter());
        let session_id = SessionId::new();
        let created_a = adapter
            .store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("generation A");
        let handle_a = created_a
            .lifecycle_handle
            .clone()
            .expect("generation A exact handle");
        let pause = Arc::new(RegistrationRefreshDispatchPause::new());
        *adapter
            .registration_refresh_dispatch_pause
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::clone(&pause));

        adapter.schedule_registration_refresh(handle_a.clone(), Some(Instant::now()));
        tokio::time::timeout(Duration::from_secs(1), pause.wait_entered())
            .await
            .expect("refresh reached pre-dispatch pause");

        adapter
            .store
            .remove_session_exact(&handle_a)
            .await
            .expect("retire generation A");
        // Production deliberately quarantines a retired raw identifier for
        // the anti-reuse horizon. Advance only that test-owned deadline so B
        // can coexist with the still-paused exact-A refresh; releasing the
        // pause below then proves the stale handle cannot enter B's lane.
        assert!(adapter
            .store
            .authority()
            .elapse_reuse_horizon_for_test(&session_id));
        let created_b = adapter
            .store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("generation B");
        let handle_b = created_b
            .lifecycle_handle
            .clone()
            .expect("generation B exact handle");
        adapter
            .store
            .update_session_exact_with(&handle_b, None, |session| {
                session.registration_retry_count = 77;
            })
            .expect("mark generation B");
        let before = adapter
            .store
            .get_session_snapshot_exact(&handle_b)
            .expect("generation B before stale wake");

        *adapter
            .registration_refresh_dispatch_pause
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        pause.release();
        tokio::time::timeout(
            Duration::from_secs(1),
            adapter.registration_refresh_retained.wait_idle(),
        )
        .await
        .expect("stale refresh completed");

        let after = adapter
            .store
            .get_session_snapshot_exact(&handle_b)
            .expect("generation B after stale wake");
        assert_eq!(after.revision(), before.revision());
        assert_eq!(after.registration_retry_count, 77);
        assert_eq!(after.registration_call_id, None);

        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("coordinator shutdown");
    }

    #[tokio::test]
    async fn stale_refresh_schedule_cannot_replace_reused_generation_task() {
        let coordinator = crate::api::unified::UnifiedCoordinator::new(
            crate::api::unified::Config::local("refresh-stale-schedule", 0),
        )
        .await
        .expect("coordinator");
        let adapter = Arc::clone(coordinator.dialog_adapter());
        let session_id = SessionId::new();
        let created_a = adapter
            .store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("generation A");
        let handle_a = created_a
            .lifecycle_handle
            .clone()
            .expect("generation A exact handle");
        adapter
            .store
            .remove_session_exact(&handle_a)
            .await
            .expect("retire generation A");
        assert!(adapter
            .store
            .authority()
            .elapse_reuse_horizon_for_test(&session_id));
        let created_b = adapter
            .store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("generation B");
        let handle_b = created_b
            .lifecycle_handle
            .clone()
            .expect("generation B exact handle");

        let refresh_at = Instant::now() + Duration::from_secs(60);
        adapter.schedule_registration_refresh(handle_b.clone(), Some(refresh_at));
        let generation_b = {
            let task = adapter
                .registration_refresh_tasks
                .get(&session_id)
                .expect("generation B refresh task");
            assert_eq!(task.handle, handle_b);
            task.generation
        };

        // Simulate a delayed generation-A post-commit effect arriving after B
        // has published its own refresh owner.
        adapter.schedule_registration_refresh(handle_a, Some(refresh_at));

        let task = adapter
            .registration_refresh_tasks
            .get(&session_id)
            .expect("generation B refresh survives stale schedule");
        assert_eq!(task.handle, handle_b);
        assert_eq!(task.generation, generation_b);
        drop(task);
        assert_eq!(adapter.registration_refresh_retained.count(), 1);

        adapter
            .abort_all_registration_refreshes_and_wait()
            .await
            .expect("refresh tasks drained");
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("coordinator shutdown");
    }

    #[tokio::test]
    async fn stale_refresh_abort_cannot_cancel_reused_generation_task() {
        let coordinator = crate::api::unified::UnifiedCoordinator::new(
            crate::api::unified::Config::local("refresh-stale-abort", 0),
        )
        .await
        .expect("coordinator");
        let adapter = Arc::clone(coordinator.dialog_adapter());
        let session_id = SessionId::new();
        let created_a = adapter
            .store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("generation A");
        let handle_a = created_a
            .lifecycle_handle
            .clone()
            .expect("generation A exact handle");
        adapter
            .store
            .remove_session_exact(&handle_a)
            .await
            .expect("retire generation A");
        assert!(adapter
            .store
            .authority()
            .elapse_reuse_horizon_for_test(&session_id));
        let created_b = adapter
            .store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("generation B");
        let handle_b = created_b
            .lifecycle_handle
            .clone()
            .expect("generation B exact handle");

        adapter.schedule_registration_refresh(
            handle_b.clone(),
            Some(Instant::now() + Duration::from_secs(60)),
        );
        let generation_b = adapter
            .registration_refresh_tasks
            .get(&session_id)
            .expect("generation B refresh task")
            .generation;

        // Simulate generation A's delayed failure/unregistration completion.
        adapter.abort_registration_refresh_exact(&handle_a);

        let task = adapter
            .registration_refresh_tasks
            .get(&session_id)
            .expect("generation B refresh survives stale abort");
        assert_eq!(task.handle, handle_b);
        assert_eq!(task.generation, generation_b);
        drop(task);
        assert_eq!(adapter.registration_refresh_retained.count(), 1);

        adapter.abort_registration_refresh_exact(&handle_b);
        tokio::time::timeout(
            Duration::from_secs(1),
            adapter.registration_refresh_retained.wait_idle(),
        )
        .await
        .expect("generation B refresh cancellation drained");
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("coordinator shutdown");
    }

    #[tokio::test]
    async fn refresh_without_yaml_transition_does_not_write_or_dispatch() {
        let coordinator = crate::api::unified::UnifiedCoordinator::new(
            crate::api::unified::Config::local("refresh-no-transition", 0),
        )
        .await
        .expect("coordinator");
        let adapter = Arc::clone(coordinator.dialog_adapter());
        let session_id = SessionId::new();
        let created = adapter
            .store
            .create_session(session_id, Role::UAC, false)
            .await
            .expect("idle registration session");
        let handle = created.lifecycle_handle.clone().expect("idle exact handle");
        let before = adapter
            .store
            .get_session_snapshot_exact(&handle)
            .expect("snapshot before no-transition refresh");

        adapter.schedule_registration_refresh(handle.clone(), Some(Instant::now()));
        tokio::time::timeout(
            Duration::from_secs(1),
            adapter.registration_refresh_retained.wait_idle(),
        )
        .await
        .expect("no-transition refresh completed");

        let after = adapter
            .store
            .get_session_snapshot_exact(&handle)
            .expect("snapshot after no-transition refresh");
        assert_eq!(after.revision(), before.revision());
        assert_eq!(after.registration_call_id, None);
        assert_eq!(after.registration_cseq, 0);
        assert_eq!(after.registration_retry_count, 0);

        let source = include_str!("dialog_adapter.rs");
        let removed_fallback = ["send_registration_refresh", "_direct"].concat();
        assert!(!source.contains(&removed_fallback));

        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("coordinator shutdown");
    }

    const RETAINED_AUTH_PASSWORD: &str = "retained-auth-password";

    fn retained_auth_challenge(realm: &str, nonce: &str, stale: bool, qop: &str) -> String {
        let stale = if stale { ", stale=true" } else { "" };
        format!("Digest realm=\"{realm}\", nonce=\"{nonce}\", algorithm=MD5, qop=\"{qop}\"{stale}")
    }

    fn retained_auth_session(dialog_id: &RvoipDialogId, origin_target: &str) -> SessionState {
        let mut session = SessionState::new(SessionId::new(), Role::UAC);
        session.dialog_id = Some(dialog_id.clone().into());
        session.dialog_established = true;
        session.call_state = crate::types::CallState::Active;
        session.remote_uri = Some(origin_target.to_string());
        session.auth = Some(crate::auth::SipClientAuth::digest(
            "retained-auth-user",
            RETAINED_AUTH_PASSWORD,
        ));
        session
    }

    fn retain_digest_credential(
        session: &mut SessionState,
        kind: crate::session_store::state::InviteCredentialKind,
        protection_target: &str,
        realm: &str,
        nonce: &str,
        qop: &str,
    ) {
        session.invite_authorization_credentials.push(
            crate::session_store::state::InviteAuthorizationCredential {
                kind,
                protection_target: protection_target.to_string(),
                challenge_raw: retained_auth_challenge(realm, nonce, false, qop),
                realm: realm.to_string(),
                nonce: Some(nonce.to_string()),
                stale_refreshes: 0,
                value: String::new(),
            },
        );
    }

    fn typed_auth_value(header: &rvoip_sip_core::types::TypedHeader) -> (&'static str, String) {
        match header {
            rvoip_sip_core::types::TypedHeader::Authorization(value) => {
                ("origin", value.to_string())
            }
            rvoip_sip_core::types::TypedHeader::ProxyAuthorization(value) => {
                ("proxy", value.to_string())
            }
            rvoip_sip_core::types::TypedHeader::Other(
                rvoip_sip_core::types::HeaderName::Authorization,
                rvoip_sip_core::types::headers::HeaderValue::Raw(value),
            ) => (
                "origin",
                String::from_utf8(value.clone()).expect("validated origin auth text"),
            ),
            rvoip_sip_core::types::TypedHeader::Other(
                rvoip_sip_core::types::HeaderName::ProxyAuthorization,
                rvoip_sip_core::types::headers::HeaderValue::Raw(value),
            ) => (
                "proxy",
                String::from_utf8(value.clone()).expect("validated proxy auth text"),
            ),
            other => panic!("unexpected retained auth header: {other:?}"),
        }
    }

    async fn install_active_data_message_session(
        store: &SessionStore,
        session_id: SessionId,
        dialog_id: &RvoipDialogId,
        with_route_auth: bool,
    ) -> SessionRegistryHandle {
        let created = store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create exact MESSAGE session");
        let handle = created
            .lifecycle_handle
            .clone()
            .expect("MESSAGE session has an exact lifecycle handle");
        store
            .update_session_exact_with(&handle, None, |session| {
                session.dialog_id = Some(dialog_id.clone().into());
                session.dialog_established = true;
                session.call_state = crate::types::CallState::Active;
                session.remote_uri = Some("sip:bob@example.test".to_string());
                session.auth = with_route_auth.then(|| {
                    crate::auth::SipClientAuth::digest("retained-auth-user", RETAINED_AUTH_PASSWORD)
                });
            })
            .expect("publish active exact MESSAGE state");
        store
            .registry()
            .map_dialog_handle(&handle, dialog_id.clone().into())
            .expect("install canonical exact MESSAGE dialog mapping");
        handle
    }

    #[tokio::test]
    async fn retained_message_auth_is_write_free_until_challenged_and_retries_immutably() {
        const REALM: &str = "message-lane-owned";
        const NONCE: &str = "message-lane-owned-nonce";
        const REQUEST_URI: &str = "sip:bob@contact.example.test";
        const BODY: &[u8] = b"\x00immutable\xffmessage-body";

        let store = SessionStore::new();
        let dialog_id = RvoipDialogId::new();
        let handle =
            install_active_data_message_session(&store, SessionId::new(), &dialog_id, true).await;
        let lanes = SipDataMessageDispatchLanes::default();
        let immutable_body = bytes::Bytes::from_static(BODY);
        let transport = crate::auth::SipTransportSecurityContext::from_transport_name("UDP");

        let (first_handle, first_state_guard, mut first_session) =
            lock_and_load_exact_data_message_session(&store, &dialog_id)
                .await
                .expect("load first exact MESSAGE attempt");
        assert_eq!(first_handle, handle);
        let first_message_lane = lanes.lane(&dialog_id);
        let first_message_guard = Arc::clone(&first_message_lane).lock_owned().await;
        let before_unauth = store
            .get_session_snapshot_exact(&handle)
            .expect("snapshot before unauthenticated MESSAGE")
            .revision();
        let mut unauthenticated = rvoip_sip_core::Request::new(
            rvoip_sip_core::Method::Message,
            Uri::from_str(REQUEST_URI).expect("MESSAGE request URI"),
        );
        unauthenticated.body = immutable_body.clone();
        authorize_data_message_lane_owned_exact(
            &store,
            &first_handle,
            &mut first_session,
            &dialog_id,
            &mut unauthenticated,
            REQUEST_URI,
            None,
            &transport,
        )
        .expect("prepare unauthenticated MESSAGE attempt");
        assert!(unauthenticated.headers.is_empty());
        assert_eq!(unauthenticated.body.as_ref(), BODY);
        assert_eq!(
            store
                .get_session_snapshot_exact(&handle)
                .expect("snapshot after unauthenticated MESSAGE")
                .revision(),
            before_unauth,
            "an attempt without retained credentials or a challenge must not publish"
        );
        drop(first_state_guard);
        drop(first_message_guard);

        let challenge = DataMessageAuthChallenge {
            status: 401,
            value: retained_auth_challenge(REALM, NONCE, false, "auth-int"),
        };
        let (retry_handle, retry_state_guard, mut retry_session) =
            lock_and_load_exact_data_message_session(&store, &dialog_id)
                .await
                .expect("reacquire exact state for challenged MESSAGE retry");
        let retry_message_guard = lanes.lane(&dialog_id).lock_owned().await;
        let mut retry = rvoip_sip_core::Request::new(
            rvoip_sip_core::Method::Message,
            Uri::from_str(REQUEST_URI).expect("MESSAGE retry URI"),
        );
        retry.body = immutable_body.clone();
        authorize_data_message_lane_owned_exact(
            &store,
            &retry_handle,
            &mut retry_session,
            &dialog_id,
            &mut retry,
            REQUEST_URI,
            Some(&challenge),
            &transport,
        )
        .expect("authorize challenged MESSAGE retry");
        assert!(unauthenticated.headers.is_empty());
        assert_eq!(unauthenticated.body.as_ref(), BODY);
        assert_eq!(retry.body.as_ref(), BODY);
        assert_eq!(retry.headers.len(), 1);
        let (_, authorization) = typed_auth_value(&retry.headers[0]);
        let parsed = crate::auth::DigestAuthenticator::parse_authorization(&authorization)
            .expect("parse challenged MESSAGE authorization");
        assert_eq!(parsed.nc.as_deref(), Some("00000001"));
        assert!(crate::auth::DigestAuthenticator::new(REALM)
            .validate_response_with_body(&parsed, "MESSAGE", RETAINED_AUTH_PASSWORD, Some(BODY),)
            .expect("validate immutable auth-int MESSAGE retry body"));
        drop(retry_state_guard);
        drop(retry_message_guard);

        let (next_handle, next_state_guard, mut next_session) =
            lock_and_load_exact_data_message_session(&store, &dialog_id)
                .await
                .expect("reacquire exact state for the next MESSAGE attempt");
        let next_message_guard = lanes.lane(&dialog_id).lock_owned().await;
        let mut next = rvoip_sip_core::Request::new(
            rvoip_sip_core::Method::Message,
            Uri::from_str(REQUEST_URI).expect("next MESSAGE URI"),
        );
        next.body = immutable_body;
        authorize_data_message_lane_owned_exact(
            &store,
            &next_handle,
            &mut next_session,
            &dialog_id,
            &mut next,
            REQUEST_URI,
            None,
            &transport,
        )
        .expect("reuse retained MESSAGE protection space");
        let (_, authorization) = typed_auth_value(&next.headers[0]);
        let parsed = crate::auth::DigestAuthenticator::parse_authorization(&authorization)
            .expect("parse next retained MESSAGE authorization");
        assert_eq!(parsed.nc.as_deref(), Some("00000002"));
        assert!(crate::auth::DigestAuthenticator::new(REALM)
            .validate_response_with_body(&parsed, "MESSAGE", RETAINED_AUTH_PASSWORD, Some(BODY),)
            .expect("validate exact body after monotonic nonce advance"));
        drop(next_state_guard);
        drop(next_message_guard);

        let published = store
            .get_session_snapshot_exact(&handle)
            .expect("read retained MESSAGE auth publication");
        assert_eq!(
            published
                .digest_nc
                .get(&(REALM.to_string(), NONCE.to_string())),
            Some(&2)
        );
    }

    #[tokio::test]
    async fn retained_message_cleanup_acquires_state_then_waits_for_attempt_lane() {
        let store = Arc::new(SessionStore::new());
        let lanes = Arc::new(SipDataMessageDispatchLanes::default());
        let dialog_id = RvoipDialogId::new();
        let handle = install_active_data_message_session(
            store.as_ref(),
            SessionId::new(),
            &dialog_id,
            false,
        )
        .await;

        let (_, state_guard, _) =
            lock_and_load_exact_data_message_session(store.as_ref(), &dialog_id)
                .await
                .expect("load active MESSAGE attempt");
        let attempt_lane = lanes.lane(&dialog_id);
        let attempt_guard = Arc::clone(&attempt_lane).lock_owned().await;
        drop(state_guard);

        let cleanup_state_lane = store
            .state_machine_lane_exact(&handle)
            .expect("cleanup resolves the exact state lane");
        let cleanup_message_lane = Arc::clone(&attempt_lane);
        let cleanup_store = Arc::clone(&store);
        let cleanup_lanes = Arc::clone(&lanes);
        let cleanup_handle = handle.clone();
        let cleanup_dialog = dialog_id.clone();
        let state_entered = Arc::new(tokio::sync::Notify::new());
        let cleanup_state_entered = Arc::clone(&state_entered);
        let message_entered = Arc::new(AtomicBool::new(false));
        let cleanup_message_entered = Arc::clone(&message_entered);
        let cleanup = tokio::spawn(async move {
            let _cleanup_state_guard = cleanup_state_lane.lock_owned().await;
            cleanup_state_entered.notify_one();
            let _cleanup_message_guard = cleanup_message_lane.lock_owned().await;
            cleanup_message_entered.store(true, Ordering::Release);
            cleanup_store
                .registry()
                .clear_dialog_handle_retained(&cleanup_handle, cleanup_dialog.clone().into())
                .expect("clear canonical exact dialog during cleanup");
            cleanup_lanes.remove_exact(&cleanup_dialog, &attempt_lane);
        });

        tokio::time::timeout(Duration::from_secs(1), state_entered.notified())
            .await
            .expect("cleanup entered exact state lane");
        tokio::task::yield_now().await;
        assert!(
            !message_entered.load(Ordering::Acquire),
            "cleanup crossed a MESSAGE attempt that still owns its per-wire lane"
        );

        drop(attempt_guard);
        tokio::time::timeout(Duration::from_secs(1), cleanup)
            .await
            .expect("cleanup acquired MESSAGE lane after attempt release")
            .expect("cleanup lock-order task did not panic");
        assert!(message_entered.load(Ordering::Acquire));
        assert!(matches!(
            lock_and_load_exact_data_message_session(store.as_ref(), &dialog_id).await,
            Err(SessionError::InvalidTransition(_))
        ));
    }

    #[tokio::test]
    async fn retained_message_exact_resolution_cannot_cross_raw_session_id_reuse() {
        let store = SessionStore::new();
        let session_id = SessionId("message-reused-session".to_string());
        let dialog_a = RvoipDialogId::new();
        let handle_a =
            install_active_data_message_session(&store, session_id.clone(), &dialog_a, false).await;
        let stale_key = handle_a.key().clone();
        let stale_revision = handle_a.slot_revision();
        let (resolved_a, guard_a, _) = lock_and_load_exact_data_message_session(&store, &dialog_a)
            .await
            .expect("resolve MESSAGE generation A");
        assert_eq!(resolved_a, handle_a);
        drop(guard_a);

        store
            .remove_session_exact(&handle_a)
            .await
            .expect("retire MESSAGE generation A");
        assert!(matches!(
            lock_and_load_exact_data_message_session(&store, &dialog_a).await,
            Err(SessionError::InvalidTransition(_))
        ));
        drop(resolved_a);
        drop(handle_a);
        assert!(store.authority().elapse_reuse_horizon_for_test(&session_id));

        let dialog_b = RvoipDialogId::new();
        let handle_b =
            install_active_data_message_session(&store, session_id, &dialog_b, false).await;
        assert!(store
            .registry()
            .get_dialog_exact(&stale_key, stale_revision)
            .is_none());
        assert!(matches!(
            lock_and_load_exact_data_message_session(&store, &dialog_a).await,
            Err(SessionError::InvalidTransition(_))
        ));
        let (resolved_b, guard_b, _) = lock_and_load_exact_data_message_session(&store, &dialog_b)
            .await
            .expect("resolve only current MESSAGE generation B");
        assert_eq!(resolved_b, handle_b);
        drop(guard_b);
    }

    #[test]
    fn retained_message_lock_order_and_auth_publication_have_one_authority() {
        let source = include_str!("dialog_adapter.rs");
        let resolver = rust_function_source(source, "lock_and_load_exact_data_message_session");
        let registry_owner = resolver
            .find("get_handle_by_dialog_exact")
            .expect("MESSAGE resolves its canonical registry owner");
        let lane_resolution = resolver
            .find("state_machine_lane_exact(&handle)")
            .expect("MESSAGE resolves the owner's exact state lane");
        let state_lock = resolver
            .find("lock_owned().await")
            .expect("MESSAGE acquires the exact state lane");
        let snapshot_revalidation = resolver
            .find("get_session_snapshot_exact")
            .expect("MESSAGE revalidates exact state after lane acquisition");
        let dialog_revalidation = resolver
            .find("get_dialog_handle_exact")
            .expect("MESSAGE revalidates canonical dialog ownership");
        assert!(
            registry_owner < lane_resolution
                && lane_resolution < state_lock
                && state_lock < snapshot_revalidation
                && snapshot_revalidation < dialog_revalidation
        );
        assert!(!resolver.contains("dialog_to_session"));
        assert!(!resolver.contains("session_to_dialog"));

        let cleanup = rust_function_source(source, "cleanup_session_exact_lane_owned");
        let canonical_cleanup_dialog = cleanup
            .find("get_dialog_retained_exact(handle)")
            .expect("cleanup resolves its MESSAGE lane from canonical exact ownership");
        let cleanup_message_lane = cleanup
            .find("data_message_dispatch_lanes.lane")
            .expect("cleanup serializes with the canonical dialog's MESSAGE lane");
        assert!(canonical_cleanup_dialog < cleanup_message_lane);
        assert!(!cleanup.contains("session_to_dialog.get"));

        let external_facade = rust_function_source(source, "send_data_message_on_dialog");
        assert_eq!(
            external_facade
                .matches("send_data_message_on_dialog_driver(")
                .count(),
            1,
            "the exact-locking facade delegates once to the canonical driver"
        );
        assert!(!external_facade.contains("build_sip_data_request"));
        assert!(!external_facade.contains("send_request_with_candidate_failover"));
        assert!(!external_facade.contains("wait_for_final_response"));

        let lane_owned_facade = rust_function_source(source, "send_message_in_dialog_lane_owned");
        assert_eq!(
            lane_owned_facade
                .matches("send_data_message_on_dialog_driver(")
                .count(),
            1,
            "the already-lane-owned facade delegates once to the canonical driver"
        );
        assert!(lane_owned_facade.contains("DataMessageStateLane::AlreadyOwned(session)"));
        assert!(!lane_owned_facade.contains("send_request_in_dialog"));
        assert!(!lane_owned_facade.contains("build_sip_data_request"));
        assert!(!lane_owned_facade.contains("send_request_with_candidate_failover"));
        assert!(!lane_owned_facade.contains("wait_for_final_response"));

        let legacy_message = rust_function_source(source, "legacy_text_data_message");
        assert!(legacy_message.contains("content_type: \"text/plain\""));
        assert!(legacy_message.contains("extra_headers: Vec::new()"));
        let legacy_value = legacy_text_data_message("legacy body".to_string());
        assert_eq!(legacy_value.content_type, "text/plain");
        assert_eq!(legacy_value.bytes.as_ref(), b"legacy body");
        assert!(legacy_value.extra_headers.is_empty());

        let public_legacy_facade = rust_function_source(source, "send_message");
        assert_eq!(
            public_legacy_facade
                .matches("send_data_message_on_dialog_driver(")
                .count(),
            1,
            "the public in-dialog String facade delegates once to the canonical driver"
        );
        assert!(public_legacy_facade.contains("expected_handle: Some(handle)"));
        assert!(!public_legacy_facade.contains("send_request_in_dialog"));

        let dispatch = rust_function_source(source, "send_data_message_on_dialog_driver");
        let route_snapshot = dispatch
            .find("manager.get_dialog(dialog_id)")
            .expect("MESSAGE snapshots dialog routing before DNS");
        let candidate_resolution = dispatch
            .find("resolve_uri_to_candidates")
            .expect("MESSAGE resolves its exact route");
        let zero_candidate_gate = dispatch
            .find("if candidates.is_empty()")
            .expect("MESSAGE preserves zero-candidate no-write behavior");
        let state_attempt = dispatch
            .find("lock_and_load_exact_data_message_session")
            .expect("MESSAGE acquires exact state after DNS preflight");
        let message_attempt = dispatch
            .find("data_message_dispatch_lanes.lane")
            .expect("MESSAGE attempt acquires the per-dialog wire lane second");
        let route_revalidation = dispatch
            .find("dialog.remote_target != expected_target")
            .expect("MESSAGE revalidates the DNS route under both lanes");
        let cseq_allocation = dispatch
            .find("create_request_template")
            .expect("MESSAGE allocates one CSeq after route revalidation");
        let auth_publish = dispatch
            .find("authorize_data_message_lane_owned_exact")
            .expect("MESSAGE authorizes while exact state is owned");
        let state_release = dispatch[auth_publish..]
            .find("drop(attempt_state)")
            .map(|offset| auth_publish + offset)
            .expect("MESSAGE releases the per-attempt state binding before wire I/O");
        let wire = dispatch
            .find("send_request_with_candidate_failover")
            .expect("MESSAGE wire dispatch");
        let final_wait = dispatch
            .find("wait_for_final_response")
            .expect("MESSAGE final-response wait");
        assert!(
            route_snapshot < candidate_resolution
                && candidate_resolution < zero_candidate_gate
                && zero_candidate_gate < state_attempt
                && state_attempt < message_attempt
                && message_attempt < route_revalidation
                && route_revalidation < cseq_allocation
                && cseq_allocation < auth_publish
                && auth_publish < state_release
                && state_release < wire
                && wire < final_wait
        );
        let message_release = dispatch[final_wait..]
            .find("drop(dispatch_guard)")
            .map(|offset| final_wait + offset)
            .expect("challenge retry releases its MESSAGE lane");
        let retain_challenge = dispatch
            .find("fresh_challenge = Some(challenge)")
            .expect("challenge retained for the bounded retry");
        let retry = dispatch[retain_challenge..]
            .find("continue;")
            .map(|offset| retain_challenge + offset)
            .expect("challenge retry returns to exact state acquisition");
        assert!(message_release < retain_challenge && retain_challenge < retry);
        assert_eq!(
            dispatch.matches("data_message_dispatch_lanes.lane").count(),
            1,
            "each loop pass acquires one per-attempt MESSAGE lane"
        );
        assert_eq!(
            dispatch.matches("build_sip_data_request(").count(),
            1,
            "one request materializer owns every in-dialog MESSAGE shape"
        );
        assert_eq!(
            dispatch
                .matches("send_request_with_candidate_failover(")
                .count(),
            1,
            "one wire owner dispatches every in-dialog MESSAGE"
        );
        assert_eq!(
            dispatch.matches("wait_for_final_response(").count(),
            1,
            "one final-response owner observes every in-dialog MESSAGE"
        );
        assert!(source.contains(
            "const DATA_MESSAGE_FINAL_RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);"
        ));

        let auth = rust_function_source(source, "authorize_data_message_lane_owned_exact");
        assert_eq!(auth.matches("mutate_retained_dialog_auth(").count(), 1);
        assert_eq!(
            auth.matches("update_retained_auth_lane_owned_exact(")
                .count(),
            1
        );
        assert!(!auth.contains("dialog_to_session"));
        assert!(!auth.contains("session_to_dialog"));

        for authority_source in [
            source,
            include_str!("../session_store/store.rs"),
            include_str!("../state_machine/executor.rs"),
        ] {
            for deleted in [
                ["auth_required", "_event"].concat(),
                ["preserve_auth", "_coordination"].concat(),
                ["auth_coordination", "_changed"].concat(),
            ] {
                assert!(
                    !authority_source.contains(&deleted),
                    "deleted competing auth authority returned: {deleted}"
                );
            }
        }
    }

    #[test]
    fn retained_auth_pure_bye_binds_exact_method_and_dialog_target() {
        use crate::session_store::state::InviteCredentialKind;

        const REALM: &str = "bye-retained";
        const NONCE: &str = "bye-retained-nonce";
        const ORIGIN_TARGET: &str = "sip:bob@example.test";
        const REQUEST_URI: &str = "sip:bob@contact.example.test";

        let dialog_id = RvoipDialogId::new();
        let mut session = retained_auth_session(&dialog_id, ORIGIN_TARGET);
        retain_digest_credential(
            &mut session,
            InviteCredentialKind::Origin,
            ORIGIN_TARGET,
            REALM,
            NONCE,
            "auth",
        );

        let headers = mutate_retained_dialog_auth(
            &mut session,
            &dialog_id,
            RetainedDialogAuthMode::Request,
            "BYE",
            REQUEST_URI,
            RetainedDialogAuthRoute {
                next_hop: REQUEST_URI,
                transport: &crate::auth::SipTransportSecurityContext::from_transport_name("UDP"),
            },
            None,
        )
        .expect("pure BYE retained auth");
        assert_eq!(headers.len(), 1);
        let (kind, value) = typed_auth_value(&headers[0]);
        assert_eq!(kind, "origin");
        let parsed = crate::auth::DigestAuthenticator::parse_authorization(&value)
            .expect("parse retained BYE authorization");
        assert_eq!(parsed.uri, REQUEST_URI);
        assert!(crate::auth::DigestAuthenticator::new(REALM)
            .validate_response_with_body(&parsed, "BYE", RETAINED_AUTH_PASSWORD, None)
            .expect("validate BYE method binding"));
        assert!(!crate::auth::DigestAuthenticator::new(REALM)
            .validate_response_with_body(&parsed, "INVITE", RETAINED_AUTH_PASSWORD, None)
            .expect("reject INVITE method substitution"));
        assert_eq!(session.invite_authorization_credentials[0].value, value);
    }

    #[test]
    fn retained_auth_pure_message_installs_one_fresh_and_one_stale_refresh() {
        const REALM: &str = "message-retained";
        const OLD_NONCE: &str = "message-old-nonce";
        const FRESH_NONCE: &str = "message-fresh-nonce";
        const THIRD_NONCE: &str = "message-third-nonce";
        const ORIGIN_TARGET: &str = "sip:bob@example.test";
        const REQUEST_URI: &str = "sip:bob@contact.example.test";

        let dialog_id = RvoipDialogId::new();
        let mut session = retained_auth_session(&dialog_id, ORIGIN_TARGET);
        let initial = DataMessageAuthChallenge {
            status: 401,
            value: retained_auth_challenge(REALM, OLD_NONCE, false, "auth"),
        };
        mutate_retained_dialog_auth(
            &mut session,
            &dialog_id,
            RetainedDialogAuthMode::DataMessage {
                fresh_challenge: Some(&initial),
            },
            "MESSAGE",
            REQUEST_URI,
            RetainedDialogAuthRoute {
                next_hop: REQUEST_URI,
                transport: &crate::auth::SipTransportSecurityContext::from_transport_name("UDP"),
            },
            Some(b"fresh-message"),
        )
        .expect("install initial MESSAGE protection space");
        assert_eq!(session.invite_authorization_credentials.len(), 1);
        assert_eq!(
            session.invite_authorization_credentials[0].nonce.as_deref(),
            Some(OLD_NONCE)
        );
        assert_eq!(
            session.invite_authorization_credentials[0].stale_refreshes,
            0
        );

        let stale = DataMessageAuthChallenge {
            status: 401,
            value: retained_auth_challenge(REALM, FRESH_NONCE, true, "auth"),
        };
        mutate_retained_dialog_auth(
            &mut session,
            &dialog_id,
            RetainedDialogAuthMode::DataMessage {
                fresh_challenge: Some(&stale),
            },
            "MESSAGE",
            REQUEST_URI,
            RetainedDialogAuthRoute {
                next_hop: REQUEST_URI,
                transport: &crate::auth::SipTransportSecurityContext::from_transport_name("UDP"),
            },
            Some(b"fresh-message"),
        )
        .expect("refresh MESSAGE protection space once");
        assert_eq!(
            session.invite_authorization_credentials[0].nonce.as_deref(),
            Some(FRESH_NONCE)
        );
        assert_eq!(
            session.invite_authorization_credentials[0].stale_refreshes,
            1
        );
        assert_eq!(
            session
                .digest_nc
                .get(&(REALM.to_string(), FRESH_NONCE.to_string())),
            Some(&1)
        );

        let counts_before_rejection = session.digest_nc.clone();
        let rejected = DataMessageAuthChallenge {
            status: 401,
            value: retained_auth_challenge(REALM, THIRD_NONCE, true, "auth"),
        };
        let error = mutate_retained_dialog_auth(
            &mut session,
            &dialog_id,
            RetainedDialogAuthMode::DataMessage {
                fresh_challenge: Some(&rejected),
            },
            "MESSAGE",
            REQUEST_URI,
            RetainedDialogAuthRoute {
                next_hop: REQUEST_URI,
                transport: &crate::auth::SipTransportSecurityContext::from_transport_name("UDP"),
            },
            Some(b"fresh-message"),
        )
        .expect_err("a second stale refresh must fail");
        assert!(matches!(error, SessionError::AuthError(_)));
        assert_eq!(session.digest_nc, counts_before_rejection);
        assert_eq!(
            session.invite_authorization_credentials[0].nonce.as_deref(),
            Some(FRESH_NONCE),
            "rejected refresh must not partially replace retained state"
        );
    }

    #[test]
    fn retained_auth_pure_message_keeps_origin_and_proxy_targets_isolated() {
        const ORIGIN_REALM: &str = "message-origin";
        const PROXY_REALM: &str = "message-proxy";
        const ORIGIN_TARGET: &str = "sip:bob@example.test";
        const REQUEST_URI: &str = "sip:bob@contact.example.test";
        const PROXY_TARGET: &str = "sip:proxy.example.test;lr";

        let dialog_id = RvoipDialogId::new();
        let mut session = retained_auth_session(&dialog_id, ORIGIN_TARGET);
        let origin = DataMessageAuthChallenge {
            status: 401,
            value: retained_auth_challenge(ORIGIN_REALM, "origin-nonce", false, "auth"),
        };
        mutate_retained_dialog_auth(
            &mut session,
            &dialog_id,
            RetainedDialogAuthMode::DataMessage {
                fresh_challenge: Some(&origin),
            },
            "MESSAGE",
            REQUEST_URI,
            RetainedDialogAuthRoute {
                next_hop: PROXY_TARGET,
                transport: &crate::auth::SipTransportSecurityContext::from_transport_name("UDP"),
            },
            Some(b"origin-and-proxy"),
        )
        .expect("install origin protection space");

        let proxy = DataMessageAuthChallenge {
            status: 407,
            value: retained_auth_challenge(PROXY_REALM, "proxy-nonce", false, "auth"),
        };
        let headers = mutate_retained_dialog_auth(
            &mut session,
            &dialog_id,
            RetainedDialogAuthMode::DataMessage {
                fresh_challenge: Some(&proxy),
            },
            "MESSAGE",
            REQUEST_URI,
            RetainedDialogAuthRoute {
                next_hop: PROXY_TARGET,
                transport: &crate::auth::SipTransportSecurityContext::from_transport_name("UDP"),
            },
            Some(b"origin-and-proxy"),
        )
        .expect("install independent proxy protection space");
        assert_eq!(headers.len(), 2);
        let mut kinds = headers
            .iter()
            .map(typed_auth_value)
            .map(|(kind, _)| kind)
            .collect::<Vec<_>>();
        kinds.sort_unstable();
        assert_eq!(kinds, vec!["origin", "proxy"]);

        let proxy_count = session
            .digest_nc
            .get(&(PROXY_REALM.to_string(), "proxy-nonce".to_string()))
            .copied();
        let changed_hop_headers = mutate_retained_dialog_auth(
            &mut session,
            &dialog_id,
            RetainedDialogAuthMode::DataMessage {
                fresh_challenge: None,
            },
            "MESSAGE",
            REQUEST_URI,
            RetainedDialogAuthRoute {
                next_hop: "sip:other-proxy.example.test;lr",
                transport: &crate::auth::SipTransportSecurityContext::from_transport_name("UDP"),
            },
            Some(b"origin-and-proxy"),
        )
        .expect("reauthor exact changed next hop");
        assert_eq!(changed_hop_headers.len(), 1);
        assert_eq!(typed_auth_value(&changed_hop_headers[0]).0, "origin");
        assert_eq!(
            session
                .digest_nc
                .get(&(PROXY_REALM.to_string(), "proxy-nonce".to_string()))
                .copied(),
            proxy_count,
            "a proxy credential must not advance for a different next hop"
        );
    }

    #[test]
    fn retained_auth_pure_digest_nonce_count_is_monotonic_and_body_exact() {
        use crate::session_store::state::InviteCredentialKind;

        const REALM: &str = "message-auth-int";
        const NONCE: &str = "message-auth-int-nonce";
        const ORIGIN_TARGET: &str = "sip:bob@example.test";
        const REQUEST_URI: &str = "sip:bob@contact.example.test";
        const BODY: &[u8] = b"\x00binary\xffbody";

        let dialog_id = RvoipDialogId::new();
        let mut session = retained_auth_session(&dialog_id, ORIGIN_TARGET);
        retain_digest_credential(
            &mut session,
            InviteCredentialKind::Origin,
            ORIGIN_TARGET,
            REALM,
            NONCE,
            "auth-int",
        );
        let transport = crate::auth::SipTransportSecurityContext::from_transport_name("UDP");

        for expected in 1..=2 {
            let headers = mutate_retained_dialog_auth(
                &mut session,
                &dialog_id,
                RetainedDialogAuthMode::DataMessage {
                    fresh_challenge: None,
                },
                "MESSAGE",
                REQUEST_URI,
                RetainedDialogAuthRoute {
                    next_hop: REQUEST_URI,
                    transport: &transport,
                },
                Some(BODY),
            )
            .expect("advance retained Digest nonce count");
            let (_, value) = typed_auth_value(&headers[0]);
            let parsed = crate::auth::DigestAuthenticator::parse_authorization(&value)
                .expect("parse retained auth-int response");
            let expected_nc = format!("{expected:08x}");
            assert_eq!(parsed.nc.as_deref(), Some(expected_nc.as_str()));
            assert!(
                crate::auth::DigestAuthenticator::new(REALM)
                    .validate_response_with_body(
                        &parsed,
                        "MESSAGE",
                        RETAINED_AUTH_PASSWORD,
                        Some(BODY),
                    )
                    .expect("validate exact auth-int body")
            );
        }
        assert_eq!(
            session
                .digest_nc
                .get(&(REALM.to_string(), NONCE.to_string())),
            Some(&2)
        );
    }

    #[test]
    fn retained_dialog_auth_wrappers_share_one_pure_implementation() {
        let source = include_str!("dialog_adapter.rs");
        let implementation_marker = ["fn mutate_retained_dialog_auth", "("].concat();
        assert_eq!(
            source.matches(&implementation_marker).count(),
            1,
            "retained dialog authentication must have one pure implementation"
        );
        for wrapper in [
            "send_bye_with_options_lane_owned",
            "authorize_data_message_lane_owned_exact",
        ] {
            let body = rust_function_source(source, wrapper);
            assert_eq!(
                body.matches("mutate_retained_dialog_auth(").count(),
                1,
                "{wrapper} must delegate exactly once to the shared mutator"
            );
            assert!(!body.contains("authorization_for_challenge_with_transport_context"));
            assert!(!body.contains("invite_authorization_credentials ="));
            assert!(!body.contains("digest_nc.entry"));
        }
    }

    #[tokio::test]
    async fn legacy_dialog_resolution_fails_closed_and_fences_raw_id_reuse() {
        let store = Arc::new(SessionStore::new());
        let missing_dialog = RvoipDialogId::new();
        assert!(
            matches!(
                lock_and_load_exact_legacy_dialog_session(store.as_ref(), &missing_dialog).await,
                Err(SessionError::SessionNotFound(_))
            ),
            "an unmapped legacy Dialog-ID must fail closed"
        );

        let session_id = SessionId("legacy-dialog-reused-session".to_string());
        let dialog_a = RvoipDialogId::new();
        let handle_a = install_active_data_message_session(
            store.as_ref(),
            session_id.clone(),
            &dialog_a,
            false,
        )
        .await;
        let held_lane = store
            .state_machine_lane_exact(&handle_a)
            .expect("generation A exact lane")
            .lock_owned()
            .await;
        let waiting_store = Arc::clone(&store);
        let waiting_dialog = dialog_a.clone();
        let waiting = tokio::spawn(async move {
            lock_and_load_exact_legacy_dialog_session(waiting_store.as_ref(), &waiting_dialog).await
        });
        tokio::task::yield_now().await;
        assert!(
            !waiting.is_finished(),
            "legacy resolver bypassed its exact lane"
        );

        store
            .remove_session_exact(&handle_a)
            .await
            .expect("retire legacy dialog generation A");
        drop(held_lane);
        let stale = tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .expect("stale legacy resolution remained blocked")
            .expect("stale legacy resolution task panicked");
        assert!(
            matches!(
                stale,
                Err(SessionError::SessionNotFound(_)) | Err(SessionError::InvalidTransition(_))
            ),
            "a queued legacy operation survived exact retirement"
        );

        drop(handle_a);
        assert!(store.authority().elapse_reuse_horizon_for_test(&session_id));
        let dialog_b = RvoipDialogId::new();
        let handle_b =
            install_active_data_message_session(store.as_ref(), session_id, &dialog_b, false).await;
        assert!(
            matches!(
                lock_and_load_exact_legacy_dialog_session(store.as_ref(), &dialog_a).await,
                Err(SessionError::SessionNotFound(_))
            ),
            "generation A dialog resolved through raw session-ID reuse"
        );
        let (resolved_b, lane_b, _) =
            lock_and_load_exact_legacy_dialog_session(store.as_ref(), &dialog_b)
                .await
                .expect("generation B dialog resolves its exact owner");
        assert_eq!(resolved_b, handle_b);
        drop(lane_b);
    }

    #[tokio::test]
    async fn signaling_and_quiesced_cleanup_serialize_on_the_same_exact_lane() {
        let store = Arc::new(SessionStore::new());
        let session_id = SessionId::new();
        let created = store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create BYE/cleanup serialization session");
        let handle = created
            .lifecycle_handle
            .clone()
            .expect("BYE/cleanup exact handle");
        let (_, signaling_lane) = store
            .state_machine_lane(&session_id)
            .expect("active signaling lane");
        let signaling_guard = Arc::clone(&signaling_lane).lock_owned().await;

        store
            .quiesce_session_exact(&handle)
            .await
            .expect("quiesce while signaling lane is held");
        let cleanup_lane = store
            .state_machine_lane_retained_exact(&handle)
            .expect("shutdown retained cleanup lane");
        assert!(Arc::ptr_eq(&signaling_lane, &cleanup_lane));

        let attempted = Arc::new(AtomicBool::new(false));
        let entered = Arc::new(AtomicBool::new(false));
        let waiting_attempted = Arc::clone(&attempted);
        let waiting_entered = Arc::clone(&entered);
        let cleanup_waiter = tokio::spawn(async move {
            waiting_attempted.store(true, Ordering::Release);
            let _cleanup_guard = cleanup_lane.lock_owned().await;
            waiting_entered.store(true, Ordering::Release);
        });
        while !attempted.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
        tokio::task::yield_now().await;
        assert!(
            !entered.load(Ordering::Acquire),
            "shutdown cleanup entered while signaling owned the exact lane"
        );

        drop(signaling_guard);
        tokio::time::timeout(Duration::from_secs(1), cleanup_waiter)
            .await
            .expect("shutdown cleanup remained blocked after BYE released lane")
            .expect("shutdown cleanup serialization task panicked");
        assert!(entered.load(Ordering::Acquire));
    }

    #[test]
    fn bye_uses_only_the_executor_lane_and_canonical_wire_dispatch() {
        let source = include_str!("dialog_adapter.rs");
        for lane_owned in [
            "send_bye_with_options_lane_owned",
            "send_bye_with_auth_lane_owned",
            "dispatch_bye_with_options",
        ] {
            let body = rust_function_source(source, lane_owned);
            assert!(
                !body.contains("data_message_dispatch_lanes"),
                "{lane_owned} must not acquire the MESSAGE/cleanup lane"
            );
        }

        let first_attempt = rust_function_source(source, "send_bye_with_options_lane_owned");
        assert!(first_attempt.contains("mutate_retained_dialog_auth("));
        assert!(!first_attempt.contains("update_retained_auth_lane_owned_exact"));
        assert!(!first_attempt.contains("update_session_and_snapshot"));

        let public_cleanup = rust_function_source(source, "cleanup_session");
        assert!(public_cleanup.contains("state_machine_lane(session_id)"));
        let exact_cleanup = rust_function_source(source, "cleanup_session_exact");
        let cleanup_lock = exact_cleanup
            .find("lock_owned().await")
            .expect("shutdown cleanup locks retained exact lane");
        let cleanup_revalidation = exact_cleanup
            .find("get_session_retained_snapshot_exact")
            .expect("shutdown cleanup revalidates retained exact cell");
        assert!(cleanup_lock < cleanup_revalidation);
        assert!(exact_cleanup.contains("state_machine_lane_retained_exact"));
        assert!(!exact_cleanup.contains("data_message_dispatch_lanes"));

        let lane_owned_cleanup = rust_function_source(source, "cleanup_session_exact_lane_owned");
        assert!(lane_owned_cleanup.contains("data_message_dispatch_lanes.lane"));

        let actions = include_str!("../state_machine/actions.rs");
        assert_eq!(
            actions
                .matches(".send_bye_with_options_lane_owned(")
                .count(),
            2,
            "both first-attempt BYE actions must reuse the executor lane"
        );
        assert_eq!(
            actions.matches(".send_bye_with_auth_lane_owned(").count(),
            1,
            "BYE auth retry must reuse the executor lane"
        );
        assert!(!actions.contains(".send_bye_with_options(&session.session_id"));
        assert!(!actions.contains(".send_bye_with_auth(&session.session_id"));
        assert_eq!(
            actions
                .matches(".cleanup_session_exact_lane_owned(")
                .count(),
            4,
            "every state-machine dialog cleanup path must reuse its held exact lane"
        );
        assert!(!actions.contains("dialog_adapter.cleanup_session("));
        assert!(!actions.contains("dialog_adapter.cleanup_session_exact("));
    }

    #[tokio::test]
    async fn retained_auth_exact_update_runs_under_the_exact_state_lane() {
        let store = SessionStore::new();
        let session_id = SessionId::new();
        let dialog_id = DialogId::new();
        store
            .create_session(session_id.clone(), crate::state_table::Role::UAC, false)
            .await
            .expect("create exact retained-auth session");
        store
            .update_session_with(&session_id, |session| {
                session.dialog_id = Some(dialog_id);
                session.dialog_established = true;
                session.call_state = crate::types::CallState::Active;
                session.remote_uri = Some("sip:peer@example.test".to_string());
            })
            .await
            .expect("publish established dialog");

        store
            .update_session_with(&session_id, |session| {
                session.call_established_triggered = true;
                session.sdp_origin_version = session.sdp_origin_version.saturating_add(1);
            })
            .await
            .expect("publish benign concurrent mutation");
        let (handle, lane) = store
            .state_machine_lane(&session_id)
            .expect("resolve retained-auth exact lane");
        let _state_lane_guard = lane.lock_owned().await;
        let auth_read_revision = store
            .get_session_snapshot_exact(&handle)
            .expect("capture lane-owned auth revision")
            .revision();

        let observed_origin_version = update_retained_auth_lane_owned_exact(
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
            InviteDispatchFailure::ReinviteWithOptions,
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

    fn rust_function_source<'a>(source: &'a str, name: &str) -> &'a str {
        let marker = format!("fn {name}(");
        let start = source
            .find(&marker)
            .unwrap_or_else(|| panic!("missing Rust function {name}"));
        let body_start = start
            + source[start..]
                .find('{')
                .unwrap_or_else(|| panic!("missing body for Rust function {name}"));
        let mut depth = 0usize;
        for (offset, byte) in source.as_bytes()[body_start..].iter().enumerate() {
            match byte {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return &source[start..=body_start + offset];
                    }
                }
                _ => {}
            }
        }
        panic!("unterminated Rust function {name}")
    }

    #[test]
    fn retained_in_dialog_methods_have_one_lower_wire_authority() {
        let source = include_str!("dialog_adapter.rs");
        for (authority, lower_wire_call) in [
            (
                "dispatch_bye_with_options",
                ".send_bye_with_options_and_completion(",
            ),
            ("dispatch_cancel_with_options", ".send_cancel_with_options("),
            (
                "dispatch_reinvite_with_options",
                ".send_reinvite_with_options(",
            ),
            ("dispatch_refer_with_options", ".send_refer_with_options("),
            ("dispatch_notify_with_options", ".send_notify_with_options("),
            ("dispatch_info_with_options", ".send_info_with_options("),
            ("dispatch_update_with_options", ".send_update_with_options("),
        ] {
            let body = rust_function_source(source, authority);
            assert_eq!(
                body.matches(lower_wire_call).count(),
                1,
                "{authority} must retain exactly one {lower_wire_call} wire call"
            );
        }
    }

    #[test]
    fn legacy_dialog_facades_have_no_false_success_or_placeholder_path() {
        let source = include_str!("dialog_adapter.rs");
        let response = rust_function_source(source, "send_response_by_dialog");
        assert!(response.contains("SessionError::InvalidTransition"));
        assert!(response.contains("no exact inbound transaction authority"));
        assert!(!response.contains("self.dialog_api"));

        for facade in ["send_reinvite", "send_refer"] {
            let body = rust_function_source(source, facade);
            assert!(body.contains("lock_and_load_exact_legacy_dialog_session"));
            assert!(body.contains(".await?"));
            assert!(!body.contains("let Some("));
            assert!(!body.contains("No session found for dialog"));
        }

        let remote_uri = rust_function_source(source, "get_remote_uri");
        assert!(remote_uri.contains("get_dialog_info"));
        assert!(remote_uri.contains("dialog.remote_uri.to_string()"));
        assert!(!remote_uri.contains("sip:remote@example.com"));
    }

    #[test]
    fn session_scoped_response_facades_fail_closed_without_transaction_rediscovery() {
        let source = include_str!("dialog_adapter.rs");
        for facade in [
            "send_response_by_dialog",
            "send_redirect_response_with_options",
            "send_redirect_response",
            "send_response_with_options",
            "send_response",
        ] {
            let body = rust_function_source(source, facade);
            assert!(body.contains("SessionError::InvalidTransition"), "{facade}");
            assert!(
                body.contains("no exact inbound transaction authority"),
                "{facade}"
            );
            assert!(!body.contains("self.dialog_api"), "{facade}");
            assert!(!body.contains("pending_response"), "{facade}");
        }
    }

    #[test]
    fn retained_in_dialog_facades_do_not_bypass_canonical_dispatch() {
        let source = include_str!("dialog_adapter.rs");
        for facade in [
            "send_ack",
            "send_bye",
            "send_bye_session",
            "send_bye_session_with_reason",
            "send_bye_with_options",
            "send_bye_with_auth",
            "send_cancel",
            "send_cancel_with_options",
            "send_reinvite",
            "send_reinvite_session",
            "send_reinvite_with_options",
            "send_refer",
            "send_refer_session",
            "send_refer_with_options",
            "send_refer_with_auth",
            "send_notify",
            "send_notify_with_options",
            "send_notify_with_auth",
            "send_info",
            "send_info_with_options",
            "send_info_with_auth",
            "send_update_with_options",
            "send_update_with_auth",
        ] {
            let body = rust_function_source(source, facade);
            assert!(
                !body.contains("self.dialog_api"),
                "{facade} bypasses its canonical in-dialog dispatcher"
            );
        }

        let ack = rust_function_source(source, "send_ack");
        assert!(ack.contains("generated automatically by the exact INVITE transaction"));
        let bye = rust_function_source(source, "send_bye_with_options");
        assert!(bye.contains("dispatch_state_machine_options_exact"));
        assert!(bye.contains("EventType::SendOutboundBye"));
        let cancel = rust_function_source(source, "send_cancel_with_options");
        assert!(cancel.contains("dispatch_state_machine_options_exact"));
        assert!(cancel.contains("EventType::SendOutboundCancel"));

        assert!(
            !rust_function_source(source, "send_reinvite").contains("send_update_with_options"),
            "the re-INVITE compatibility facade must not regain an UPDATE wire path"
        );
        assert!(
            !rust_function_source(source, "send_reinvite_session")
                .contains("send_request_in_dialog"),
            "the session re-INVITE facade must not regain a generic wire path"
        );
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
