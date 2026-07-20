use crate::adapters::dialog_adapter::RegisterAttemptOutcome;
use crate::adapters::outbound_request_tracker::{TrackedInDialogMethod, TrackedInDialogOptions};
use crate::state_machine::executor::{PendingOptionsSlot, StageDispatchClaim};
use crate::state_table::types::{EventType, SessionId};
use rvoip_sip_core::types::{HeaderName, TypedHeader};
use std::fmt;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

use crate::{
    adapters::{dialog_adapter::DialogAdapter, media_adapter::MediaAdapter},
    api::events::Event,
    cleanup_diag::{self, CleanupStage},
    session_store::{SessionState, SessionStore},
    state_table::{Action, Condition},
};

const SIP_RESPONSE_DISPATCH_JOIN_FAILURE: &str = "SIP response dispatch task failed (class=join)";
const DIALOG_CLEANUP_JOIN_FAILURE: &str = "SIP dialog cleanup task failed (class=join)";

fn negotiated_audio_shape(codec: &str) -> (u32, u8) {
    if codec.eq_ignore_ascii_case("opus") {
        // The SIP SDP profile advertises `opus/48000/2`; preserve that exact
        // negotiated clock/channel shape in durable session state.
        (48_000, 2)
    } else {
        (8_000, 1)
    }
}

/// Owns a spawned SIP response task and cancels it unless it has been joined.
///
/// Awaiting the response on a fresh Tokio task gives the deeply nested dialog,
/// transaction, transport, and TLS poll chain a fresh worker stack. The handle
/// remains structurally owned by the state-machine action so cancellation never
/// detaches response I/O.
struct AbortSipResponseTaskOnDrop<T> {
    handle: tokio::task::JoinHandle<T>,
    armed: bool,
}

impl<T> AbortSipResponseTaskOnDrop<T> {
    fn new(handle: tokio::task::JoinHandle<T>) -> Self {
        Self {
            handle,
            armed: true,
        }
    }

    async fn join(mut self) -> std::result::Result<T, tokio::task::JoinError> {
        let result = (&mut self.handle).await;
        self.armed = false;
        result
    }
}

impl<T> Drop for AbortSipResponseTaskOnDrop<T> {
    fn drop(&mut self) {
        if self.armed {
            self.handle.abort();
        }
    }
}

enum SipResponseTarget {
    Session,
    Transaction(rvoip_sip_dialog::transaction::TransactionKey),
}

async fn join_sip_response_task(
    task: AbortSipResponseTaskOnDrop<crate::errors::Result<()>>,
) -> crate::errors::Result<()> {
    task.join().await.map_err(|_| {
        crate::errors::SessionError::InternalError(SIP_RESPONSE_DISPATCH_JOIN_FAILURE.to_string())
    })?
}

async fn send_sip_response_on_fresh_task(
    dialog_adapter: Arc<DialogAdapter>,
    session_id: SessionId,
    target: SipResponseTarget,
    code: u16,
    sdp: Option<String>,
) -> crate::errors::Result<()> {
    let task = AbortSipResponseTaskOnDrop::new(tokio::spawn(async move {
        match target {
            SipResponseTarget::Session => {
                dialog_adapter.send_response(&session_id, code, sdp).await
            }
            SipResponseTarget::Transaction(transaction_id) => {
                dialog_adapter
                    .send_response_for_transaction(&session_id, &transaction_id, code, sdp)
                    .await
            }
        }
    }));
    join_sip_response_task(task).await
}

async fn cleanup_dialog_on_fresh_task(
    dialog_adapter: Arc<DialogAdapter>,
    session_id: SessionId,
) -> crate::errors::Result<()> {
    let task = AbortSipResponseTaskOnDrop::new(tokio::spawn(async move {
        dialog_adapter.cleanup_session(&session_id).await
    }));
    task.join().await.map_err(|_| {
        crate::errors::SessionError::InternalError(DIALOG_CLEANUP_JOIN_FAILURE.to_string())
    })?
}

/// Result of a state-table action.
///
/// Actions may enqueue internal follow-up events, but they must not call
/// `StateMachine::process_event` directly. The executor drains these events
/// after the current transition has fully unwound and saved its state.
#[derive(Debug, Clone, Default)]
pub(crate) struct ActionOutcome {
    pub(crate) follow_up_events: Vec<EventType>,
}

impl ActionOutcome {
    fn with_event(event: EventType) -> Self {
        Self {
            follow_up_events: vec![event],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegisterActionMode {
    Register,
    RegisterWithAuth,
    Unregister,
}

/// Narrow copy of fields that dialog-side REGISTER processing may mutate
/// while an action awaits the wire response.
///
/// The executor owns an event-local `SessionState`, while `DialogAdapter`
/// atomically advances registration identity, digest nonce-count, transport,
/// and outcome fields in the store.  Reloading the complete session after
/// every attempt cloned all call/media/SDP/history state.  This projection
/// keeps the two views coherent without paying that unrelated cost.
struct RegistrationStateProjection {
    registration_expires: Option<u32>,
    registration_call_id: Option<String>,
    registration_cseq: u32,
    registration_accepted_expires: Option<u32>,
    registration_registered_at: Option<std::time::Instant>,
    registration_next_refresh_at: Option<std::time::Instant>,
    registration_last_failure: Option<String>,
    registration_service_route: Option<Vec<String>>,
    registration_pub_gruu: Option<String>,
    registration_temp_gruu: Option<String>,
    is_registered: bool,
    registration_retry_count: u32,
    pending_auth_transport: Option<crate::auth::SipTransportSecurityContext>,
    digest_nc: std::collections::HashMap<(String, String), u32>,
}

impl RegistrationStateProjection {
    fn capture(session: &SessionState) -> Self {
        Self {
            registration_expires: session.registration_expires,
            registration_call_id: session.registration_call_id.clone(),
            registration_cseq: session.registration_cseq,
            registration_accepted_expires: session.registration_accepted_expires,
            registration_registered_at: session.registration_registered_at,
            registration_next_refresh_at: session.registration_next_refresh_at,
            registration_last_failure: session.registration_last_failure.clone(),
            registration_service_route: session.registration_service_route.clone(),
            registration_pub_gruu: session.registration_pub_gruu.clone(),
            registration_temp_gruu: session.registration_temp_gruu.clone(),
            is_registered: session.is_registered,
            registration_retry_count: session.registration_retry_count,
            pending_auth_transport: session.pending_auth_transport.clone(),
            digest_nc: session.digest_nc.clone(),
        }
    }

    fn apply(self, session: &mut SessionState) {
        session.registration_expires = self.registration_expires;
        session.registration_call_id = self.registration_call_id;
        session.registration_cseq = self.registration_cseq;
        session.registration_accepted_expires = self.registration_accepted_expires;
        session.registration_registered_at = self.registration_registered_at;
        session.registration_next_refresh_at = self.registration_next_refresh_at;
        session.registration_last_failure = self.registration_last_failure;
        session.registration_service_route = self.registration_service_route;
        session.registration_pub_gruu = self.registration_pub_gruu;
        session.registration_temp_gruu = self.registration_temp_gruu;
        session.is_registered = self.is_registered;
        session.registration_retry_count = self.registration_retry_count;
        session.pending_auth_transport = self.pending_auth_transport;
        session.digest_nc = self.digest_nc;
    }
}

fn sync_registration_state(
    store: &SessionStore,
    session_id: &SessionId,
    session: &mut SessionState,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    store
        .with_session(session_id, RegistrationStateProjection::capture)?
        .apply(session);
    Ok(())
}

/// Redacted validation error for SIP-owned INVITE option materialization.
///
/// Neither `Display` nor derived `Debug` retains the rejected value or the
/// parser's source error. Diagnostics expose only a fixed field label, whether
/// the field was present, its byte length, and a validation class.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InviteOptionsMaterializationError {
    InvalidPAssertedIdentityUri { bytes: usize },
    InvalidOutboundProxyUri { bytes: usize },
}

impl fmt::Display for InviteOptionsMaterializationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (field, bytes) = match self {
            Self::InvalidPAssertedIdentityUri { bytes } => ("p_asserted_identity", bytes),
            Self::InvalidOutboundProxyUri { bytes } => ("outbound_proxy", bytes),
        };
        write!(
            formatter,
            "INVITE option validation failed (field={field}, present=true, bytes={bytes}, class=invalid-uri)"
        )
    }
}

impl std::error::Error for InviteOptionsMaterializationError {}

/// Value-free endpoint metadata used by outbound INVITE log records.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct InviteEndpointDiagnostics {
    from_present: bool,
    from_bytes: usize,
    target_present: bool,
    target_bytes: usize,
    sdp_present: bool,
}

impl InviteEndpointDiagnostics {
    fn new(from: Option<&str>, target: Option<&str>, sdp_present: bool) -> Self {
        Self {
            from_present: from.is_some(),
            from_bytes: from.map_or(0, str::len),
            target_present: target.is_some(),
            target_bytes: target.map_or(0, str::len),
            sdp_present,
        }
    }
}

async fn execute_register_action(
    session: &mut SessionState,
    dialog_adapter: &Arc<DialogAdapter>,
    session_store: &Arc<SessionStore>,
    mode: RegisterActionMode,
) -> Result<ActionOutcome, Box<dyn std::error::Error + Send + Sync>> {
    let session_id = session.session_id.clone();
    let from_uri = session
        .local_uri
        .clone()
        .ok_or_else(|| "local_uri not set for registration".to_string())?;
    let registrar_uri = match mode {
        RegisterActionMode::Unregister => session
            .registrar_uri
            .clone()
            .ok_or_else(|| "registrar_uri not set for unregistration".to_string())?,
        RegisterActionMode::Register | RegisterActionMode::RegisterWithAuth => session
            .registrar_uri
            .clone()
            .or_else(|| session.remote_uri.clone())
            .ok_or_else(|| "registrar_uri not set for registration".to_string())?,
    };
    let contact_uri = match mode {
        RegisterActionMode::Unregister => session
            .registration_contact
            .clone()
            .ok_or_else(|| "contact_uri not set for unregistration".to_string())?,
        RegisterActionMode::Register | RegisterActionMode::RegisterWithAuth => session
            .registration_contact
            .clone()
            .or_else(|| session.local_uri.clone())
            .ok_or_else(|| "contact_uri not set for registration".to_string())?,
    };
    let auth = match mode {
        RegisterActionMode::Register => None,
        RegisterActionMode::RegisterWithAuth | RegisterActionMode::Unregister => session
            .auth
            .clone()
            .or_else(|| session.credentials.clone().map(Into::into)),
    };
    let mut expires = match mode {
        RegisterActionMode::Unregister => 0,
        RegisterActionMode::Register | RegisterActionMode::RegisterWithAuth => {
            session.registration_expires.unwrap_or(3600)
        }
    };

    // SIP_API_DESIGN_2 §7.3 — preserve builder-staged extras across the
    // 401/407 retry hop. We `clone()` (not `take()`) so the stash persists
    // for the auth-retry pass; `Action::ClearPendingREGISTEROptions` (or
    // the Terminated backstop) clears it on final response.
    let staged_extras: Vec<rvoip_sip_core::types::TypedHeader> = session
        .pending_register_options
        .as_ref()
        .map(|opts| opts.extra_headers.clone())
        .unwrap_or_default();

    loop {
        let outcome = dialog_adapter
            .send_register(
                &session_id,
                &registrar_uri,
                &from_uri,
                &contact_uri,
                expires,
                auth.as_ref(),
                staged_extras.clone(),
            )
            .await?;

        match outcome {
            RegisterAttemptOutcome::Registered {
                accepted_expires,
                metadata,
            } => {
                dialog_adapter
                    .apply_registration_success(
                        &session_id,
                        &registrar_uri,
                        &from_uri,
                        &contact_uri,
                        accepted_expires,
                        metadata,
                    )
                    .await?;
                sync_registration_state(session_store, &session_id, session)?;
                return Ok(ActionOutcome::with_event(EventType::Registration200OK));
            }
            RegisterAttemptOutcome::Unregistered => {
                if mode == RegisterActionMode::Unregister {
                    dialog_adapter
                        .apply_unregistration_success(&session_id, &registrar_uri)
                        .await?;
                    sync_registration_state(session_store, &session_id, session)?;
                    return Ok(ActionOutcome::with_event(EventType::Unregistration200OK));
                }

                dialog_adapter
                    .apply_registration_failure(
                        &session_id,
                        &registrar_uri,
                        200,
                        "REGISTER returned an unregistration success while registering",
                    )
                    .await?;
                sync_registration_state(session_store, &session_id, session)?;
                return Ok(ActionOutcome::with_event(EventType::RegistrationFailed(
                    200,
                )));
            }
            RegisterAttemptOutcome::AuthChallenge {
                status_code,
                challenge,
            } => {
                let challenge_details =
                    rvoip_auth_core::DigestAuthenticator::parse_challenge_details(&challenge).ok();
                if mode == RegisterActionMode::Unregister {
                    dialog_adapter
                        .apply_unregistration_failure(
                            &session_id,
                            &registrar_uri,
                            format!(
                                "unregistration received {} authentication challenge",
                                status_code
                            ),
                        )
                        .await?;
                    sync_registration_state(session_store, &session_id, session)?;
                    return Ok(ActionOutcome::with_event(EventType::UnregistrationFailed));
                }

                let (retry_count, previous_nonce) =
                    session_store.with_session(&session_id, |latest| {
                        (
                            latest.registration_retry_count,
                            latest
                                .auth_challenge
                                .as_ref()
                                .map(|challenge| challenge.nonce.clone()),
                        )
                    })?;
                let stale_recovery = retry_count == 1
                    && challenge_details
                        .as_ref()
                        .is_some_and(|details| details.stale)
                    && previous_nonce.as_deref().is_some_and(|nonce| {
                        challenge_details
                            .as_ref()
                            .is_some_and(|details| nonce != details.challenge.nonce)
                    });
                if retry_count >= 1 && !stale_recovery {
                    tracing::error!(
                        "❌ REGISTER auth failed (retry count {}); invalid credentials",
                        retry_count
                    );
                    dialog_adapter
                        .apply_registration_failure(
                            &session_id,
                            &registrar_uri,
                            status_code,
                            "REGISTER authentication failed",
                        )
                        .await?;
                    sync_registration_state(session_store, &session_id, session)?;
                    return Ok(ActionOutcome::with_event(EventType::RegistrationFailed(
                        status_code,
                    )));
                }

                let retry_count = session_store
                    .update_session_with(&session_id, |latest| {
                        latest.registration_retry_count += 1;
                        latest.registration_retry_count
                    })
                    .await?;
                sync_registration_state(session_store, &session_id, session)?;
                session.registration_retry_count = retry_count;
                return Ok(ActionOutcome::with_event(EventType::AuthRequired {
                    status_code,
                    challenge,
                    method: "REGISTER".to_string(),
                }));
            }
            RegisterAttemptOutcome::IntervalTooBrief { min_expires } => {
                if mode == RegisterActionMode::Unregister {
                    dialog_adapter
                        .apply_unregistration_failure(
                            &session_id,
                            &registrar_uri,
                            format!(
                                "unregistration received 423 Interval Too Brief Min-Expires={}",
                                min_expires
                            ),
                        )
                        .await?;
                    sync_registration_state(session_store, &session_id, session)?;
                    return Ok(ActionOutcome::with_event(EventType::UnregistrationFailed));
                }

                let retry_count = session_store
                    .with_session(&session_id, |latest| latest.registration_retry_count)?;
                if retry_count >= 2 {
                    tracing::error!(
                        "❌ Registration failed with repeated 423 — giving up (retry count {})",
                        retry_count
                    );
                    dialog_adapter
                        .apply_registration_failure(
                            &session_id,
                            &registrar_uri,
                            423,
                            "Registration failed with repeated 423 Interval Too Brief responses",
                        )
                        .await?;
                    sync_registration_state(session_store, &session_id, session)?;
                    return Ok(ActionOutcome::with_event(EventType::RegistrationFailed(
                        423,
                    )));
                }

                tracing::info!(
                    "🔄 423 Interval Too Brief — retrying REGISTER with Expires={} (server required min)",
                    min_expires
                );
                let (retry_count, registration_expires) = session_store
                    .update_session_with(&session_id, |latest| {
                        latest.registration_expires = Some(min_expires);
                        latest.registration_retry_count += 1;
                        (latest.registration_retry_count, latest.registration_expires)
                    })
                    .await?;
                session.registration_retry_count = retry_count;
                session.registration_expires = registration_expires;
                expires = min_expires;
            }
            RegisterAttemptOutcome::Failure {
                status_code,
                reason,
            } => {
                if mode == RegisterActionMode::Unregister {
                    dialog_adapter
                        .apply_unregistration_failure(
                            &session_id,
                            &registrar_uri,
                            format!("{} (status {})", reason, status_code),
                        )
                        .await?;
                    sync_registration_state(session_store, &session_id, session)?;
                    return Ok(ActionOutcome::with_event(EventType::UnregistrationFailed));
                }

                dialog_adapter
                    .apply_registration_failure(&session_id, &registrar_uri, status_code, reason)
                    .await?;
                sync_registration_state(session_store, &session_id, session)?;
                return Ok(ActionOutcome::with_event(EventType::RegistrationFailed(
                    status_code,
                )));
            }
        }
    }
}

/// Materialize the per-call INVITE override set from a staged
/// [`OutboundCallOptionsSnapshot`](crate::api::send::outbound_call::OutboundCallOptionsSnapshot)
/// into a dialog-core [`InviteRequestOptions`] plus whether the global
/// outbound-proxy `Route` should be suppressed.
///
/// SIP_API_DESIGN_2 Phase B. Shared by the initial dispatch
/// ([`Action::SendINVITEWithOptions`]) and the 401/407 retry
/// ([`Action::SendINVITEWithAuth`]) so the authenticated retry's wire form
/// matches the initial INVITE — the root cause of per-call overrides vanishing
/// on the challenge retry. `P-Asserted-Identity` / `Subject` ride
/// `extra_headers`; outbound proxy, `From` display name, `Contact`, and
/// pre-computed `Authorization` are typed structural fields.
fn authoritative_invite_sdp(
    snapshot: Option<&crate::api::send::outbound_call::OutboundCallOptionsSnapshot>,
    generated_sdp: Option<&str>,
) -> Option<String> {
    snapshot
        .and_then(|options| options.sdp.clone())
        .or_else(|| generated_sdp.map(str::to_owned))
}

fn invite_proxy_protection_target(
    snapshot: Option<&crate::api::send::outbound_call::OutboundCallOptionsSnapshot>,
    dialog_adapter: &DialogAdapter,
    request_uri: &str,
) -> String {
    match snapshot.map(|snapshot| &snapshot.outbound_proxy_override) {
        Some(crate::api::send::ProxyOverride::Use(uri)) => uri.clone(),
        Some(crate::api::send::ProxyOverride::Suppress) => request_uri.to_string(),
        Some(crate::api::send::ProxyOverride::Default) | None => dialog_adapter
            .outbound_proxy_uri
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| request_uri.to_string()),
    }
}

fn retained_invite_authorization_headers(
    session: &SessionState,
    origin_target: &str,
    proxy_target: &str,
) -> Result<Vec<TypedHeader>, crate::errors::SessionError> {
    use crate::session_store::state::InviteCredentialKind;

    session
        .invite_authorization_credentials
        .iter()
        .filter(|credential| match credential.kind {
            InviteCredentialKind::Origin => credential.protection_target == origin_target,
            InviteCredentialKind::Proxy => credential.protection_target == proxy_target,
        })
        .map(|credential| {
            let name = match credential.kind {
                InviteCredentialKind::Origin => HeaderName::Authorization,
                InviteCredentialKind::Proxy => HeaderName::ProxyAuthorization,
            };
            rvoip_sip_core::validation::validated_authorization_header(
                name,
                credential.value.clone(),
            )
            .map_err(|_| {
                crate::errors::SessionError::ProtocolError(
                    "retained INVITE authorization failed validation".to_string(),
                )
            })
        })
        .collect()
}

pub(crate) fn materialize_invite_options(
    snapshot: &crate::api::send::outbound_call::OutboundCallOptionsSnapshot,
    session_pai_uri: Option<&str>,
    sdp_for_wire: Option<String>,
) -> Result<
    (rvoip_sip_dialog::api::unified::InviteRequestOptions, bool),
    InviteOptionsMaterializationError,
> {
    use crate::api::send::ProxyOverride;
    use rvoip_sip_core::types::TypedHeader;
    use std::str::FromStr;

    let mut extras = snapshot.extra_headers.clone();

    // P-Asserted-Identity (RFC 3325) — from `session.pai_uri`, set by the
    // builder's `with_pai(uri)` or the `Config.pai_uri` fallback.
    if let Some(pai) = session_pai_uri {
        use rvoip_sip_core::types::{p_asserted_identity::PAssertedIdentity, uri::Uri};
        match Uri::from_str(pai) {
            Ok(uri) => extras.insert(
                0,
                TypedHeader::PAssertedIdentity(PAssertedIdentity::with_uri(uri)),
            ),
            Err(_) => {
                return Err(
                    InviteOptionsMaterializationError::InvalidPAssertedIdentityUri {
                        bytes: pai.len(),
                    },
                )
            }
        }
    }

    // Per-call outbound proxy is structural. It must stay ahead of any
    // REGISTER-learned Service-Route and survive authenticated retries.
    let outbound_proxy_uri = match &snapshot.outbound_proxy_override {
        ProxyOverride::Use(uri_str) => {
            use rvoip_sip_core::types::uri::Uri;
            match Uri::from_str(uri_str) {
                Ok(uri) => Some(uri),
                Err(_) => {
                    return Err(InviteOptionsMaterializationError::InvalidOutboundProxyUri {
                        bytes: uri_str.len(),
                    })
                }
            }
        }
        ProxyOverride::Default | ProxyOverride::Suppress => None,
    };
    let suppress_global_proxy = matches!(
        &snapshot.outbound_proxy_override,
        ProxyOverride::Suppress | ProxyOverride::Use(_)
    );

    // Subject — a first-class header appended via the application channel.
    if let Some(subject) = snapshot.subject.as_ref() {
        use rvoip_sip_core::types::subject::Subject;
        extras.push(TypedHeader::Subject(Subject::new(subject.clone())));
    }

    let opts = rvoip_sip_dialog::api::unified::InviteRequestOptions {
        from_uri: snapshot.from.clone().unwrap_or_default(),
        to_uri: snapshot.to.clone(),
        sdp: sdp_for_wire,
        call_id: None,
        from_display: snapshot.from_display.clone(),
        contact_uri: snapshot.contact_uri.clone(),
        precomputed_authorization: snapshot.precomputed_auth.clone(),
        outbound_proxy_uri,
        supported_100rel: snapshot.supported_100rel,
        extra_headers: extras,
    };
    Ok((opts, suppress_global_proxy))
}

/// Execute an action from the state table
async fn claim_tracked_request_staging(
    session: &mut SessionState,
    session_store: &SessionStore,
    method: TrackedInDialogMethod,
    dispatch_claim: Option<&StageDispatchClaim>,
) -> crate::errors::Result<TrackedInDialogOptions> {
    let fallback_slot = match method {
        TrackedInDialogMethod::Refer => session
            .pending_refer_options
            .as_ref()
            .map(|options| PendingOptionsSlot::Refer(Arc::clone(options))),
        TrackedInDialogMethod::Notify => session
            .pending_notify_options
            .as_ref()
            .map(|options| PendingOptionsSlot::Notify(Arc::clone(options))),
        TrackedInDialogMethod::Info => session
            .pending_info_options
            .as_ref()
            .map(|options| PendingOptionsSlot::Info(Arc::clone(options))),
        TrackedInDialogMethod::Update => session
            .pending_update_options
            .as_ref()
            .map(|options| PendingOptionsSlot::Update(Arc::clone(options))),
    };
    let fallback_claim = fallback_slot.map(StageDispatchClaim::new);
    let claim = dispatch_claim.or(fallback_claim.as_ref()).ok_or_else(|| {
        crate::errors::SessionError::InvalidTransition(format!(
            "outbound {} dispatch requires exact staged options",
            method.as_sip_method()
        ))
    })?;
    if claim.method() != method.as_sip_method() {
        return Err(crate::errors::SessionError::InvalidTransition(format!(
            "outbound {} dispatch received a mismatched stage claim",
            method.as_sip_method()
        )));
    }

    let claimed = session_store
        .update_session_with(&session.session_id, |stored| claim.claim_exact(stored))
        .await
        .map_err(|_| {
            crate::errors::SessionError::InternalError(
                "failed to atomically claim outbound request staging".to_string(),
            )
        })??;
    claimed.clear_if_exact(session);

    match (method, claimed) {
        (TrackedInDialogMethod::Refer, PendingOptionsSlot::Refer(options)) => {
            Ok(TrackedInDialogOptions::Refer(options))
        }
        (TrackedInDialogMethod::Notify, PendingOptionsSlot::Notify(options)) => {
            Ok(TrackedInDialogOptions::Notify(options))
        }
        (TrackedInDialogMethod::Info, PendingOptionsSlot::Info(options)) => {
            Ok(TrackedInDialogOptions::Info(options))
        }
        (TrackedInDialogMethod::Update, PendingOptionsSlot::Update(options)) => {
            Ok(TrackedInDialogOptions::Update(options))
        }
        _ => Err(crate::errors::SessionError::InvalidTransition(
            "outbound request stage claim changed method".to_string(),
        )),
    }
}

async fn advance_tracked_auth_owner(
    session: &mut SessionState,
    session_store: &SessionStore,
    method: TrackedInDialogMethod,
    challenged_transaction: &rvoip_sip_dialog::transaction::TransactionKey,
    retry_transaction: &rvoip_sip_dialog::transaction::TransactionKey,
    request_uri: &str,
) {
    let challenged_id = challenged_transaction.to_string();
    let retry_id = retry_transaction.to_string();
    let method_name = method.as_sip_method().to_string();

    session.pending_auth_transaction_id = Some(retry_id.clone());
    session.pending_auth_request_uri = Some(request_uri.to_string());
    session.pending_auth_method = Some(method_name.clone());

    // Do not overwrite a concurrently recorded challenge for another exact
    // request. The retry is already on the wire, so its tracker entry remains
    // authoritative; this compatibility projection advances only while it
    // still owns the challenged transaction.
    let _ = session_store
        .update_session_with(&session.session_id, |stored| {
            if stored.pending_auth_transaction_id.as_deref() == Some(challenged_id.as_str()) {
                stored.pending_auth_transaction_id = Some(retry_id);
                stored.pending_auth_request_uri = Some(request_uri.to_string());
                stored.pending_auth_method = Some(method_name);
            }
        })
        .await;
}

pub(crate) async fn execute_action(
    action: &Action,
    triggering_event: &EventType,
    session: &mut SessionState,
    dialog_adapter: &Arc<DialogAdapter>,
    media_adapter: &Arc<MediaAdapter>,
    session_store: &Arc<SessionStore>,
    auto_180_ringing: bool,
    _simple_peer_event_tx: &Option<tokio::sync::mpsc::Sender<Event>>, // Unused - events handled by SessionCrossCrateEventHandler
    stage_claim: Option<&StageDispatchClaim>,
) -> Result<ActionOutcome, Box<dyn std::error::Error + Send + Sync>> {
    debug!("Executing action: {:?}", action);

    match action {
        // Dialog actions
        Action::CreateDialog => {
            info!("Action::CreateDialog for session {}", session.session_id);
            let from = session
                .local_uri
                .as_deref()
                .ok_or_else(|| "local_uri not set for session".to_string())?;
            let to = session
                .remote_uri
                .as_deref()
                .ok_or_else(|| "remote_uri not set for session".to_string())?;
            info!(
                "Preparing dialog: {:?}",
                InviteEndpointDiagnostics::new(Some(from), Some(to), session.local_sdp.is_some())
            );
            // Don't create dialog here - it will be created when we send INVITE
            // Just log that we're preparing to create a dialog
            info!("Dialog will be created when INVITE is sent");
        }
        Action::CreateMediaSession => {
            info!(
                "Action::CreateMediaSession for session {}",
                session.session_id
            );
            #[cfg(feature = "perf-call-setup-diagnostics")]
            let started = std::time::Instant::now();
            let media_id = media_adapter.create_session(&session.session_id).await?;
            #[cfg(feature = "perf-call-setup-diagnostics")]
            crate::call_setup_diag::record_stage(
                &session.session_id,
                "action.create_media_session",
                started.elapsed(),
            );
            session.media_session_id = Some(media_id.clone());
            info!("Created media session ID: {:?}", media_id);
        }
        Action::GenerateLocalSDP => {
            #[cfg(feature = "perf-call-setup-diagnostics")]
            let started = std::time::Instant::now();
            let guard = cleanup_diag::stage_guard(
                CleanupStage::ActionGenerateLocalSdp,
                &session.session_id.0,
            );
            // Skip generation if a caller-supplied SDP is already in place
            // (e.g. `UnifiedCoordinator::accept_call_with_sdp` populated it
            // before dispatching `AcceptCall`). This lets b2bua hand the
            // outbound-leg answer SDP through to the inbound-leg 200 OK
            // without us re-negotiating against the local media stack.
            if session.sdp_negotiated && session.local_sdp.is_some() {
                info!(
                    "Action::GenerateLocalSDP for session {}: using pre-set SDP",
                    session.session_id
                );
            } else {
                info!(
                    "Action::GenerateLocalSDP for session {}",
                    session.session_id
                );
                let sdp = media_adapter
                    .generate_local_sdp(&session.session_id)
                    .await?;
                session.local_sdp = Some(sdp.clone());
                info!("Generated SDP with {} bytes", sdp.len());
            }
            // Persist before SendINVITE. A fast 401/407 can re-enter the
            // state machine while SendINVITE is still awaiting, and the auth
            // retry needs the original SDP offer from the store.
            session_store
                .update_session_with(&session.session_id, |stored| {
                    stored.local_sdp = session.local_sdp.clone();
                })
                .await?;
            #[cfg(feature = "perf-call-setup-diagnostics")]
            crate::call_setup_diag::record_stage(
                &session.session_id,
                "action.generate_local_sdp",
                started.elapsed(),
            );
            guard.finish_success();
        }
        Action::SendRejectResponse => {
            let status = session.reject_status.unwrap_or(486);
            info!(
                "Action::SendRejectResponse for session {} with status {}",
                session.session_id, status
            );
            // SIP_API_DESIGN_2 §3.4 — when the application built the
            // 4xx/6xx via `RejectBuilder` / `AuthChallengeBuilder`, the
            // staged extras (`Retry-After`, `Warning`,
            // `WWW-Authenticate`, custom `X-*`, …) ride here. The
            // builder writes to `reject_response_extras` BEFORE
            // dispatching the state-machine `RejectCall` event, so we
            // consume the stash on the first SendRejectResponse and
            // clear it so a follow-up reject_call (e.g. cleanup) does
            // not pick up stale headers.
            let extras = session.reject_response_extras.take();
            if let Some(extras) = extras {
                dialog_adapter
                    .send_response_with_options(&session.session_id, status, None, extras)
                    .await?;
            } else {
                dialog_adapter
                    .send_response(&session.session_id, status, None)
                    .await?;
            }
        }
        Action::SendRedirectResponse => {
            let status = session.redirect_response_status.unwrap_or(302);
            let contacts = session.redirect_response_contacts.clone();
            info!(
                "Action::SendRedirectResponse for session {} with status {} and {} contact(s)",
                session.session_id,
                status,
                contacts.len()
            );
            if contacts.is_empty() {
                return Err(format!(
                    "SendRedirectResponse for session {} with no contacts",
                    session.session_id
                )
                .into());
            }
            dialog_adapter
                .send_redirect_response(&session.session_id, status, contacts)
                .await?;
        }
        Action::SendSIPResponse(code, _reason) => {
            if *code == 180 && !auto_180_ringing {
                debug!(
                    "Suppressing automatic 180 Ringing for session {} via Config::auto_180_ringing=false",
                    session.session_id
                );
                return Ok(ActionOutcome::default());
            }
            let guard = (*code == 200).then(|| {
                cleanup_diag::stage_guard(CleanupStage::ActionSend200Ok, &session.session_id.0)
            });
            if *code == 200 {
                let response_started_at = session.incoming_invite_received_at.take();
                if let Some(transaction_id) = session.pending_inbound_invite_transaction_id.take() {
                    let udp_receive_timing = dialog_adapter
                        .dialog_api
                        .dialog_manager()
                        .core()
                        .transaction_manager()
                        .take_inbound_timing(&transaction_id);
                    send_sip_response_on_fresh_task(
                        Arc::clone(dialog_adapter),
                        session.session_id.clone(),
                        SipResponseTarget::Transaction(transaction_id),
                        *code,
                        session.local_sdp.clone(),
                    )
                    .await?;
                    if let Some(timing) = udp_receive_timing {
                        if let Some(received_at) = timing.received_at {
                            rvoip_sip_dialog::diagnostics::record_udp_receive_to_invite_200(
                                received_at.elapsed(),
                            );
                        }
                    }
                } else {
                    send_sip_response_on_fresh_task(
                        Arc::clone(dialog_adapter),
                        session.session_id.clone(),
                        SipResponseTarget::Session,
                        *code,
                        session.local_sdp.clone(),
                    )
                    .await?;
                }
                if let Some(started_at) = response_started_at {
                    rvoip_sip_dialog::diagnostics::record_200_ok_invite_first();
                    rvoip_sip_dialog::diagnostics::record_first_invite_to_200(started_at.elapsed());
                }
            } else {
                send_sip_response_on_fresh_task(
                    Arc::clone(dialog_adapter),
                    session.session_id.clone(),
                    SipResponseTarget::Session,
                    *code,
                    session.local_sdp.clone(),
                )
                .await?;
            }
            // RFC 3261: Dialog is established when UAS sends 200 OK to INVITE
            if *code == 200 {
                session.dialog_established = true;
                info!(
                    "Dialog established (UAS sent 200 OK) for session {}",
                    session.session_id
                );
            }
            if let Some(guard) = guard {
                guard.finish_success();
            }
        }
        Action::SendINVITE => {
            info!("Action::SendINVITE for session {}", session.session_id);
            // Get session details for send_invite_with_details
            let from = session
                .local_uri
                .clone()
                .ok_or_else(|| "local_uri not set for session".to_string())?;
            let to = session
                .remote_uri
                .clone()
                .ok_or_else(|| "remote_uri not set for session".to_string())?;
            info!(
                "Sending INVITE: {:?}",
                InviteEndpointDiagnostics::new(Some(&from), Some(&to), session.local_sdp.is_some())
            );

            // Build any extra typed headers that travel with the very first
            // INVITE. The synthesized `P-Asserted-Identity` (RFC 3325 §9.1)
            // is appended first when `SessionState.pai_uri` is set;
            // caller-supplied headers from the `_with_headers` API variants
            // follow. The outbound-proxy Route prepended inside
            // `DialogAdapter::send_invite_with_extra_headers` runs after
            // this, so a configured outbound proxy still ends up first on
            // the wire.
            let mut extras: Vec<rvoip_sip_core::types::TypedHeader> = Vec::new();
            if let Some(pai) = session.pai_uri.as_ref() {
                use rvoip_sip_core::types::{
                    p_asserted_identity::PAssertedIdentity, uri::Uri, TypedHeader,
                };
                use std::str::FromStr;
                match Uri::from_str(pai) {
                    Ok(uri) => {
                        extras.push(TypedHeader::PAssertedIdentity(PAssertedIdentity::with_uri(
                            uri,
                        )));
                    }
                    Err(_) => {
                        // Reject upstream rather than silently dropping — the
                        // app set a malformed PAI and would otherwise wonder
                        // why the carrier rejects with 403.
                        return Err(
                            InviteOptionsMaterializationError::InvalidPAssertedIdentityUri {
                                bytes: pai.len(),
                            }
                            .into(),
                        );
                    }
                }
            }
            if !session.extra_headers.is_empty() {
                extras.extend(session.extra_headers.iter().cloned());
            }

            // This will create the real dialog in dialog-core.
            // Route through `send_invite_with_extra_headers` whenever we have
            // extras OR an outbound proxy is configured (E4 — that path
            // injects the pre-loaded Route header at the adapter layer).
            let use_extra_path = !extras.is_empty() || dialog_adapter.outbound_proxy_uri.is_some();
            if !use_extra_path {
                dialog_adapter
                    .send_invite_with_details(
                        &session.session_id,
                        &from,
                        &to,
                        session.local_sdp.clone(),
                    )
                    .await?;
            } else {
                dialog_adapter
                    .send_invite_with_extra_headers(
                        &session.session_id,
                        &from,
                        &to,
                        session.local_sdp.clone(),
                        extras,
                    )
                    .await?;
            }

            // Now get the real dialog ID that was created
            if let Some(real_dialog_id) = dialog_adapter.session_to_dialog.get(&session.session_id)
            {
                // Convert RvoipDialogId to our DialogId type
                let dialog_id: crate::types::DialogId = real_dialog_id.value().clone().into();
                session.dialog_id = Some(dialog_id);
                info!("INVITE sent successfully with dialog ID {:?}", dialog_id);
            } else {
                warn!("Failed to get dialog ID after sending INVITE");
                info!("INVITE sent successfully");
            }
        }
        Action::ClearPendingReinvite => {
            session.pending_reinvite = None;
            session.reinvite_retry_attempts = 0;
            debug!(
                "Cleared pending_reinvite for session {} (glare resolved by peer)",
                session.session_id
            );
        }
        Action::ScheduleReinviteRetry => {
            // RFC 3261 §14.1 — glare avoidance. The "owner" of the Call-ID
            // (the UAC that originated the dialog) waits 2.1–4.0 s; the
            // non-owner waits 0–2.0 s. Splitting the ranges ensures the
            // non-owner retries first on every round, breaking the glare
            // deterministically instead of letting both sides keep racing
            // until the retry cap trips.
            use crate::session_store::state::PendingReinvite;
            use crate::state_table::types::Role;
            const MAX_GLARE_RETRIES: u8 = 3;
            if session.reinvite_retry_attempts >= MAX_GLARE_RETRIES {
                session.pending_reinvite = None;
                return Err(format!(
                    "491 glare retry limit ({}) exceeded for session {}",
                    MAX_GLARE_RETRIES, session.session_id
                )
                .into());
            }
            let kind = match session.pending_reinvite.clone() {
                Some(k) => k,
                None => {
                    warn!(
                        "ScheduleReinviteRetry with no pending_reinvite for session {}; noop",
                        session.session_id
                    );
                    return Ok(ActionOutcome::default());
                }
            };
            session.reinvite_retry_attempts += 1;

            // UAC = Call-ID owner → 2.1–4.0 s. UAS = non-owner → 0–2.0 s.
            // `Role::Both` is a table-wildcard never stored on a session;
            // default to the owner range if it ever appears.
            let millis: u64 = match session.role {
                Role::UAS => rand::random::<u64>() % 2000,
                Role::UAC | Role::Both => 2100 + (rand::random::<u64>() % 1900),
            };
            let backoff = std::time::Duration::from_millis(millis);
            info!(
                "⏳ 491 glare: sleeping {:?} before retrying {:?} for session {} (attempt {}/{})",
                backoff,
                kind,
                session.session_id,
                session.reinvite_retry_attempts,
                MAX_GLARE_RETRIES
            );
            tokio::time::sleep(backoff).await;

            let sdp = match kind {
                PendingReinvite::Hold => media_adapter
                    .create_hold_sdp_for_session(&session.session_id)
                    .await
                    .map_err(|e| format!("create_hold_sdp failed: {}", e))?,
                PendingReinvite::Resume => media_adapter
                    .create_active_sdp_for_session(&session.session_id)
                    .await
                    .map_err(|e| format!("create_active_sdp failed: {}", e))?,
                PendingReinvite::SdpUpdate(sdp) => sdp,
            };
            session.local_sdp = Some(sdp.clone());
            dialog_adapter
                .send_reinvite_session(&session.session_id, sdp)
                .await?;
        }
        Action::RetryWithContact => {
            // RFC 3261 §8.1.3.4 / §19.1.5 — follow a 3xx redirect's Contact URI.
            // The executor pre-process has already pushed the response's targets
            // onto session.redirect_targets. Cap total follow-ups at 5 hops per
            // RFC-recommended loop breaker so misconfigured redirect chains fail.
            const MAX_REDIRECTS: u8 = 5;
            if session.redirect_attempts >= MAX_REDIRECTS {
                return Err(format!(
                    "Exceeded max {} redirect hops for session {}",
                    MAX_REDIRECTS, session.session_id
                )
                .into());
            }
            let next_target =
                session.redirect_targets.first().cloned().ok_or_else(|| {
                    "RetryWithContact: no redirect targets on session".to_string()
                })?;
            session.redirect_targets.remove(0);
            session.redirect_attempts += 1;
            session.remote_uri = Some(next_target.clone());

            // A redirect changes the origin protection target. Never replay
            // an Authorization value (including caller-supplied precomputed
            // Basic/Bearer/Digest material) to the new Contact. Proxy auth is
            // also cleared and may be re-established by a fresh 407.
            session.invite_authorization_credentials.clear();
            session.invite_auth_retry_count = 0;

            // Reset readiness flags so the state machine treats this as a fresh
            // call attempt (media session was already cleaned up by CleanupMedia
            // earlier in this transition's action sequence).
            session.dialog_established = false;
            session.sdp_negotiated = false;
            session.dialog_id = None;

            let from = session
                .local_uri
                .clone()
                .ok_or_else(|| "local_uri not set for redirect retry".to_string())?;
            info!(
                attempt = session.redirect_attempts,
                max_attempts = MAX_REDIRECTS,
                from_bytes = from.len(),
                target_bytes = next_target.len(),
                "Following 3xx redirect"
            );

            let (invite_opts, apply_global_proxy) = if let Some(snapshot) =
                session.pending_invite_options.as_ref()
            {
                let mut redirected = (**snapshot).clone();
                redirected.to = next_target.clone();
                redirected.precomputed_auth = None;
                let sdp = authoritative_invite_sdp(Some(&redirected), session.local_sdp.as_deref());
                let (options, suppress_global_proxy) =
                    materialize_invite_options(&redirected, session.pai_uri.as_deref(), sdp)?;
                session.pending_invite_options = Some(Arc::new(redirected));
                (options, !suppress_global_proxy)
            } else {
                (
                    rvoip_sip_dialog::api::unified::InviteRequestOptions {
                        from_uri: from,
                        to_uri: next_target,
                        sdp: session.local_sdp.clone(),
                        ..Default::default()
                    },
                    true,
                )
            };

            dialog_adapter
                .send_invite_with_options(&session.session_id, invite_opts, apply_global_proxy)
                .await?;
            if let Some(real_dialog_id) = dialog_adapter.session_to_dialog.get(&session.session_id)
            {
                let dialog_id: crate::types::DialogId = real_dialog_id.value().clone().into();
                session.dialog_id = Some(dialog_id);
            }
        }
        Action::SendACK => {
            // NO-OP for SIP: dialog-core sends ACK automatically per RFC 3261
            // However, we still set dialog_established = true here because for UAC,
            // the dialog is considered established when ACK is sent
            session.dialog_established = true;
            info!(
                "SendACK action: dialog-core handles ACK sending, dialog marked as established for UAC session {}",
                session.session_id
            );
        }
        Action::SendBYE => {
            // Materialize one immutable BYE snapshot before the first wire
            // write. The state transition has already published Terminating,
            // so a fast 401/407 can re-enter the state machine immediately;
            // persist the snapshot first so that retry observes the exact
            // headers/reason used by this generation.
            let reason = session.pending_bye_reason.take();
            let snapshot = if let Some(opts) = session.pending_bye_options.as_ref() {
                (**opts).clone()
            } else {
                let mut extra_headers = dialog_adapter.auto_emit_extra_headers.clone();
                if let Some((protocol, cause, text)) = reason {
                    extra_headers.push(TypedHeader::Reason(
                        rvoip_sip_core::types::reason::Reason::new(protocol, cause, text),
                    ));
                }
                let materialized = Arc::new(rvoip_sip_dialog::api::unified::ByeRequestOptions {
                    reason: None,
                    extra_headers,
                });
                // If a builder raced legacy hangup before Terminating became
                // visible, its already-staged immutable snapshot wins rather
                // than being overwritten by automatic options.
                let retained = session_store
                    .update_session_with(&session.session_id, |stored| {
                        stored
                            .pending_bye_options
                            .get_or_insert_with(|| Arc::clone(&materialized))
                            .clone()
                    })
                    .await?;
                session.pending_bye_options = Some(Arc::clone(&retained));
                (*retained).clone()
            };
            if let Err(error) = dialog_adapter
                .send_bye_with_options(&session.session_id, snapshot)
                .await
            {
                // An immediate zero-wire failure has no exact final-response
                // owner to release the retained builder slot.
                session.pending_bye_options = None;
                let _ = session_store
                    .update_session_with(&session.session_id, |stored| {
                        stored.pending_bye_options = None;
                    })
                    .await;
                return Err(error.into());
            }
            // Retain through 401/407 and release with exact BYE finalization.
        }
        // Action::SendCANCEL deleted per SIP_API_DESIGN_2.md Phase 5 —
        // consolidated into Action::SendCANCELWithOptions which honors
        // stash-precedence and auto-emit fallback identically. YAML
        // emit rows updated to reference SendCANCELWithOptions.

        // Call control actions
        Action::HoldCall => {
            // Send re-INVITE with sendonly SDP. Record that this is a Hold so
            // RFC 3261 §14.1 glare (491) retry can reissue the correct kind.
            let hold_sdp = media_adapter
                .create_hold_sdp_for_session(&session.session_id)
                .await
                .map_err(|e| format!("create_hold_sdp failed: {}", e))?;
            session.local_sdp = Some(hold_sdp.clone());
            session.pending_reinvite = Some(crate::session_store::state::PendingReinvite::Hold);
            dialog_adapter
                .send_reinvite_session(&session.session_id, hold_sdp)
                .await?;
        }
        Action::ResumeCall => {
            // Send re-INVITE with sendrecv SDP.
            let active_sdp = media_adapter
                .create_active_sdp_for_session(&session.session_id)
                .await
                .map_err(|e| format!("create_active_sdp failed: {}", e))?;
            session.local_sdp = Some(active_sdp.clone());
            session.pending_reinvite = Some(crate::session_store::state::PendingReinvite::Resume);
            dialog_adapter
                .send_reinvite_session(&session.session_id, active_sdp)
                .await?;
        }
        Action::TransferCall(target) => {
            session.transfer_target = Some(target.clone());
            session.transfer_state = crate::session_store::state::TransferState::TransferInitiated;
            dialog_adapter
                .send_refer_session(&session.session_id, target)
                .await?;
        }
        Action::StartRecording => {
            // Start recording the media session
            media_adapter.start_recording(&session.session_id).await?;
        }
        Action::StopRecording => {
            // Stop recording the media session
            media_adapter.stop_recording(&session.session_id).await?;
        }

        // Media actions
        Action::StartMediaSession => {
            media_adapter.start_session(&session.session_id).await?;
            // Mark media as ready after successfully starting
            session.media_session_ready = true;
            info!(
                "Media session started and marked as ready for session {}",
                session.session_id
            );
        }
        Action::SwitchToPassThroughOnActive => {
            // On EarlyMedia → Active, make sure any app-installed
            // ringback / announcement source gets replaced by PassThrough so
            // bidirectional audio flows. For calls that never set a source
            // the transmitter is already in PassThrough (established by
            // `establish_media_flow`), so this is a benign no-op swap.
            //
            // Swallow errors — the transmitter may not be active yet on
            // pre-negotiated-SDP flows (e.g. `accept_call_with_sdp`), and in
            // that case there's nothing to switch. The normal PassThrough
            // setup will happen when media flow is established later.
            use crate::api::unified::AudioSource;
            if let Err(e) = media_adapter
                .set_audio_source(&session.session_id, AudioSource::PassThrough)
                .await
            {
                debug!(
                    "SwitchToPassThroughOnActive: no-op for session {} ({})",
                    session.session_id, e
                );
            } else {
                debug!(
                    "SwitchToPassThroughOnActive: transmitter switched for session {}",
                    session.session_id
                );
            }
        }
        Action::NegotiateSDPAsUAC => {
            if let Some(remote_sdp) = &session.remote_sdp {
                let config = media_adapter
                    .negotiate_sdp_as_uac(&session.session_id, remote_sdp)
                    .await?;

                // Convert to session_store NegotiatedConfig
                let (sample_rate, channels) = negotiated_audio_shape(&config.codec);
                let session_config = crate::session_store::state::NegotiatedConfig {
                    local_addr: config.local_addr,
                    remote_addr: config.remote_addr,
                    codec: config.codec,
                    sample_rate,
                    channels,
                };
                session.negotiated_config = Some(session_config);
                session.local_media_direction = config.local_direction;
                session.remote_media_direction = config.remote_direction;
                session.sdp_negotiated = true;
                info!("SDP negotiated as UAC for session {}", session.session_id);
            }
        }
        Action::NegotiateSDPAsUAS => {
            let guard = cleanup_diag::stage_guard(
                CleanupStage::ActionNegotiateSdpUas,
                &session.session_id.0,
            );
            // Skip negotiation when caller supplied the answer SDP ahead of
            // time via `accept_call_with_sdp`. Same reasoning as
            // `GenerateLocalSDP` above.
            if session.sdp_negotiated && session.local_sdp.is_some() {
                info!(
                    "Action::NegotiateSDPAsUAS for session {}: using pre-set SDP",
                    session.session_id
                );
            } else if let Some(remote_sdp) = &session.remote_sdp {
                let (local_sdp, config) = media_adapter
                    .negotiate_sdp_as_uas(&session.session_id, remote_sdp)
                    .await?;

                // Convert to session_store NegotiatedConfig
                let (sample_rate, channels) = negotiated_audio_shape(&config.codec);
                let session_config = crate::session_store::state::NegotiatedConfig {
                    local_addr: config.local_addr,
                    remote_addr: config.remote_addr,
                    codec: config.codec,
                    sample_rate,
                    channels,
                };
                session.local_sdp = Some(local_sdp);
                session.negotiated_config = Some(session_config);
                session.local_media_direction = config.local_direction;
                session.remote_media_direction = config.remote_direction;
                session.sdp_negotiated = true;
                info!("SDP negotiated as UAS for session {}", session.session_id);
            }
            guard.finish_success();
        }
        Action::PrepareEarlyMediaSDP => {
            if let Some(sdp) = session.early_media_sdp.take() {
                session.local_sdp = Some(sdp);
                session.sdp_negotiated = true;
                info!(
                    "PrepareEarlyMediaSDP: using caller-supplied SDP for session {}",
                    session.session_id
                );
            } else if let Some(remote_sdp) = session.remote_sdp.clone() {
                let (local_sdp, config) = media_adapter
                    .negotiate_sdp_as_uas(&session.session_id, &remote_sdp)
                    .await?;
                let (sample_rate, channels) = negotiated_audio_shape(&config.codec);
                let session_config = crate::session_store::state::NegotiatedConfig {
                    local_addr: config.local_addr,
                    remote_addr: config.remote_addr,
                    codec: config.codec,
                    sample_rate,
                    channels,
                };
                session.local_sdp = Some(local_sdp);
                session.negotiated_config = Some(session_config);
                session.local_media_direction = config.local_direction;
                session.remote_media_direction = config.remote_direction;
                session.sdp_negotiated = true;
                info!(
                    "PrepareEarlyMediaSDP: auto-negotiated SDP answer for session {}",
                    session.session_id
                );
            } else {
                return Err(format!(
                    "PrepareEarlyMediaSDP: no caller-supplied SDP and no remote offer on record for session {}",
                    session.session_id
                ).into());
            }
        }

        // State updates
        Action::SetCondition(condition, value) => {
            match condition {
                Condition::DialogEstablished => session.dialog_established = *value,
                Condition::MediaSessionReady => session.media_session_ready = *value,
                Condition::SDPNegotiated => session.sdp_negotiated = *value,
            }
            info!("Set condition {:?} = {}", condition, value);
        }
        Action::StoreLocalSDP => {
            // Already handled by negotiate actions
        }
        Action::StoreRemoteSDP => {
            // Remote SDP should already be stored by the event processor
            // This action just confirms it's there and logs it
            if let Some(remote_sdp) = &session.remote_sdp {
                info!(
                    "Remote SDP stored for session {} ({} bytes)",
                    session.session_id,
                    remote_sdp.len()
                );
                // Parse and log the remote RTP port for debugging
                if let Some(port_match) = remote_sdp
                    .lines()
                    .find(|line| line.starts_with("m=audio"))
                    .and_then(|line| line.split_whitespace().nth(1))
                {
                    info!("Remote RTP port: {}", port_match);
                }
            } else {
                warn!(
                    "StoreRemoteSDP action called but no remote SDP found for session {}",
                    session.session_id
                );
            }
        }
        Action::StoreNegotiatedConfig => {
            // Already handled by negotiate actions
        }

        // Callbacks
        Action::TriggerCallEstablished => {
            session.call_established_triggered = true;
            info!("Call established for session {}", session.session_id);
        }
        Action::TriggerCallTerminated => {
            info!("Call terminated for session {}", session.session_id);
        }

        // Cleanup
        Action::StartDialogCleanup => {
            dialog_adapter.cleanup_session(&session.session_id).await?;
            debug!(
                "Dialog cleanup completed for session {}",
                session.session_id
            );
        }
        Action::StartMediaCleanup => {
            media_adapter.cleanup_session(&session.session_id).await?;
            debug!("Media cleanup completed for session {}", session.session_id);
        }

        // New actions for extended functionality
        Action::SendReINVITE => {
            use crate::session_store::state::PendingReinvite;
            use crate::types::CallState;
            // Pick SDP direction from the *target* state — the executor commits
            // `next_state` before running actions, so `session.call_state`
            // reflects the state we're entering. Also record `pending_reinvite`
            // so RFC 3261 §14.1 glare retry (`ScheduleReinviteRetry`) can
            // reissue the correct kind.
            let (hold_direction, kind) = match session.call_state {
                CallState::HoldPending => (true, PendingReinvite::Hold),
                CallState::Resuming => (false, PendingReinvite::Resume),
                other => {
                    // SendReINVITE fired from an unexpected state. Default to
                    // "preserve current direction" (sendrecv) to avoid lying
                    // on the wire, but log — this indicates a YAML bug.
                    warn!(
                        "SendReINVITE dispatched from state {:?} for session {} — no hold/resume intent inferred",
                        other, session.session_id
                    );
                    (false, PendingReinvite::Resume)
                }
            };

            let sdp = if hold_direction {
                media_adapter
                    .create_hold_sdp_for_session(&session.session_id)
                    .await
                    .map_err(|e| format!("create_hold_sdp failed: {}", e))?
            } else {
                media_adapter
                    .create_active_sdp_for_session(&session.session_id)
                    .await
                    .map_err(|e| format!("create_active_sdp failed: {}", e))?
            };
            session.local_sdp = Some(sdp.clone());
            session.pending_reinvite = Some(kind);
            // Persist pending_reinvite before awaiting the wire send — the
            // 491/ReinviteGlare response races with our await, and the glare
            // handler's `ScheduleReinviteRetry` reads `pending_reinvite` from
            // the store to know what kind of re-INVITE to reissue.
            session_store
                .update_session_with(&session.session_id, |stored| {
                    stored.local_sdp = session.local_sdp.clone();
                    stored.pending_reinvite = session.pending_reinvite.clone();
                })
                .await
                .map_err(|e| format!("persist pending_reinvite failed: {}", e))?;
            debug!(
                "Sending re-INVITE for session {} (hold={})",
                session.session_id, hold_direction
            );
            dialog_adapter
                .send_reinvite_session(&session.session_id, sdp)
                .await?;
        }

        Action::PlayAudioFile(file) => {
            debug!(
                "Playing audio file {} for session {}",
                file, session.session_id
            );
            media_adapter
                .play_audio_file(&session.session_id, file)
                .await?;
        }

        Action::StartRecordingMedia => {
            debug!("Starting recording for session {}", session.session_id);
            let recording_path = media_adapter.start_recording(&session.session_id).await?;
            info!("Recording started at: {}", recording_path);
        }

        Action::StopRecordingMedia => {
            debug!("Stopping recording for session {}", session.session_id);
            media_adapter.stop_recording(&session.session_id).await?;
        }

        Action::CreateBridge(other_session) => {
            debug!(
                "Creating bridge between {} and {}",
                session.session_id, other_session
            );
            media_adapter
                .create_bridge(&session.session_id, other_session)
                .await?;
            // Update session state
            session.bridged_to = Some(other_session.clone());
        }

        Action::DestroyBridge => {
            debug!("Destroying bridge for session {}", session.session_id);
            media_adapter.destroy_bridge(&session.session_id).await?;
            session.bridged_to = None;
        }

        // InitiateBlindTransfer and InitiateAttendedTransfer actions removed

        // Conference actions
        Action::CreateAudioMixer => {
            debug!("Creating audio mixer for conference");
            let mixer_id = media_adapter.create_audio_mixer().await?;
            session.conference_mixer_id = Some(mixer_id);
        }

        Action::RedirectToMixer => {
            debug!("Redirecting session {} to mixer", session.session_id);
            if let Some(mixer_id) = &session.conference_mixer_id {
                if let Some(media_id) = &session.media_session_id {
                    media_adapter
                        .redirect_to_mixer(media_id.clone(), mixer_id.clone())
                        .await?;
                }
            }
        }

        Action::ConnectToMixer => {
            debug!(
                "Connecting session {} to conference mixer",
                session.session_id
            );
            // This would connect to an existing conference mixer
            // Implementation depends on media adapter capabilities
        }

        Action::DisconnectFromMixer => {
            debug!("Disconnecting session {} from mixer", session.session_id);
            if let Some(_media_id) = &session.media_session_id {
                // TODO: Implement restore_direct_media
                warn!("restore_direct_media not implemented yet");
            }
        }

        Action::MuteToMixer => {
            debug!("Muting session {} to mixer", session.session_id);
            if let Some(media_id) = &session.media_session_id {
                media_adapter.set_mute(media_id.clone(), true).await?;
            }
        }

        Action::UnmuteToMixer => {
            debug!("Unmuting session {} to mixer", session.session_id);
            if let Some(media_id) = &session.media_session_id {
                media_adapter.set_mute(media_id.clone(), false).await?;
            }
        }

        Action::DestroyMixer => {
            debug!("Destroying conference mixer");
            if let Some(mixer_id) = &session.conference_mixer_id {
                media_adapter.destroy_mixer(mixer_id.clone()).await?;
                session.conference_mixer_id = None;
            }
        }

        // Media direction actions
        Action::UpdateMediaDirection { direction } => {
            debug!("Updating media direction to {:?}", direction);
            if let Some(media_id) = &session.media_session_id {
                // Convert from state_table::types::MediaDirection to crate::types::MediaDirection
                let media_direction = match direction {
                    crate::state_table::types::MediaDirection::SendRecv => {
                        crate::types::MediaDirection::SendRecv
                    }
                    crate::state_table::types::MediaDirection::SendOnly => {
                        crate::types::MediaDirection::SendOnly
                    }
                    crate::state_table::types::MediaDirection::RecvOnly => {
                        crate::types::MediaDirection::RecvOnly
                    }
                    crate::state_table::types::MediaDirection::Inactive => {
                        crate::types::MediaDirection::Inactive
                    }
                };
                media_adapter
                    .set_media_direction(media_id.clone(), media_direction)
                    .await?;
            }
        }

        // Additional call control
        // SendREFER and SendREFERWithReplaces actions removed

        // Mute/Unmute actions previously lived here (Action::MuteLocalAudio /
        // Action::UnmuteLocalAudio). They bypassed the state machine as
        // direct MediaAdapter calls. Per the architectural rule in
        // `docs/ARCHITECTURE_OVERVIEW.md#media-plane-side-effects`, media-plane
        // side effects do not belong in the state-machine action set — they
        // invoke the adapter directly from `UnifiedCoordinator`.

        // SendDTMFTone previously lived here for the same reason. Outbound
        // DTMF is dispatched through `UnifiedCoordinator::send_dtmf` →
        // `MediaAdapter::send_dtmf_rfc4733` directly.
        Action::StartRecordingMixer => {
            debug!("Starting recording of conference mixer");
            if let Some(mixer_id) = &session.conference_mixer_id {
                let mixer_session_id = SessionId(format!("mixer-{}", mixer_id.as_str()));
                media_adapter.start_recording(&mixer_session_id).await?;
            }
        }

        Action::StopRecordingMixer => {
            debug!("Stopping recording of conference mixer");
            if let Some(mixer_id) = &session.conference_mixer_id {
                let mixer_session_id = SessionId(format!("mixer-{}", mixer_id.as_str()));
                media_adapter.stop_recording(&mixer_session_id).await?;
            }
        }

        Action::ReleaseAllResources => {
            debug!("Releasing all resources for session {}", session.session_id);
            // Final cleanup - both dialog and media
            dialog_adapter.cleanup_session(&session.session_id).await?;
            media_adapter.cleanup_session(&session.session_id).await?;
        }

        Action::StartEmergencyCleanup => {
            error!(
                "Starting emergency cleanup for session {}",
                session.session_id
            );
            // Best-effort cleanup on error
            let _ = dialog_adapter.cleanup_session(&session.session_id).await;
            let _ = media_adapter.cleanup_session(&session.session_id).await;
        }

        Action::AttemptMediaRecovery => {
            warn!(
                "Attempting media recovery for session {}",
                session.session_id
            );
            // Try to recover from media errors
            if let Some(_media_id) = &session.media_session_id {
                // TODO: Implement attempt_recovery
                warn!("attempt_recovery not implemented yet");
            }
        }

        Action::Custom(action_name) => {
            debug!(
                "Custom action '{}' for session {}",
                action_name, session.session_id
            );
            // Handle custom SIP actions
            match action_name.as_str() {
                "Send180Ringing" => {
                    info!("Sending 180 Ringing for session {}", session.session_id);
                    dialog_adapter
                        .send_response_session(&session.session_id, 180, "Ringing")
                        .await?;
                }
                "Send200OK" => {
                    let guard = cleanup_diag::stage_guard(
                        CleanupStage::ActionSend200Ok,
                        &session.session_id.0,
                    );
                    info!("Sending 200 OK for session {}", session.session_id);
                    // For UAS, include SDP in 200 OK
                    if session.role == crate::state_table::Role::UAS {
                        if let Some(local_sdp) = &session.local_sdp {
                            dialog_adapter
                                .send_response_with_sdp(&session.session_id, 200, "OK", local_sdp)
                                .await?;
                        } else {
                            dialog_adapter
                                .send_response_session(&session.session_id, 200, "OK")
                                .await?;
                        }
                    } else {
                        dialog_adapter
                            .send_response_session(&session.session_id, 200, "OK")
                            .await?;
                    }
                    guard.finish_success();
                }
                "SuspendMedia" => {
                    if let Some(media_id) = &session.media_session_id {
                        let direction = crate::types::MediaDirection::SendOnly;
                        media_adapter
                            .set_media_direction(media_id.clone(), direction)
                            .await?;
                        session.local_media_direction = direction;
                    }
                }
                "ResumeMedia" => {
                    if let Some(media_id) = &session.media_session_id {
                        let direction = crate::types::MediaDirection::SendRecv;
                        media_adapter
                            .set_media_direction(media_id.clone(), direction)
                            .await?;
                        session.local_media_direction = direction;
                    }
                }
                _ => {
                    // Other custom actions
                }
            }
        }

        // Missing actions that need implementation
        Action::BridgeToMixer => {
            debug!("Bridging session {} to mixer", session.session_id);
            // TODO: Implement bridge to mixer functionality
            warn!("BridgeToMixer not implemented yet");
        }

        Action::RestoreDirectMedia => {
            debug!("Restoring direct media for session {}", session.session_id);
            // Alias for RestoreMediaFlow
            if let Some(media_id) = &session.media_session_id {
                use crate::types::MediaDirection;
                let active_direction = MediaDirection::SendRecv;
                media_adapter
                    .set_media_direction(media_id.clone(), active_direction)
                    .await?;
            }

            // Send re-INVITE with sendrecv
            let active_sdp = media_adapter
                .create_active_sdp_for_session(&session.session_id)
                .await
                .map_err(|e| format!("create_active_sdp failed: {}", e))?;
            session.local_sdp = Some(active_sdp.clone());
            dialog_adapter
                .send_reinvite_session(&session.session_id, active_sdp)
                .await?;
            info!("Media flow restored for session {}", session.session_id);
        }

        Action::RestoreMediaFlow => {
            debug!("Restoring media flow (unhold)");
            if let Some(media_id) = &session.media_session_id {
                use crate::types::MediaDirection;
                let active_direction = MediaDirection::SendRecv;
                media_adapter
                    .set_media_direction(media_id.clone(), active_direction)
                    .await?;
            }

            // Send re-INVITE with sendrecv
            let active_sdp = media_adapter
                .create_active_sdp_for_session(&session.session_id)
                .await
                .map_err(|e| format!("create_active_sdp failed: {}", e))?;
            session.local_sdp = Some(active_sdp.clone());
            dialog_adapter
                .send_reinvite_session(&session.session_id, active_sdp)
                .await?;
            info!("Media flow restored for session {}", session.session_id);
        }

        Action::HoldCurrentCall => {
            debug!("Putting current call on hold for transfer");

            // Update media direction to sendonly (we can hear them, they hear hold music/silence)
            if let Some(media_id) = &session.media_session_id {
                use crate::types::MediaDirection;
                let hold_direction = MediaDirection::SendOnly;
                media_adapter
                    .set_media_direction(media_id.clone(), hold_direction)
                    .await?;
            }

            // Send re-INVITE with sendonly SDP
            let hold_sdp = media_adapter
                .create_hold_sdp_for_session(&session.session_id)
                .await
                .map_err(|e| format!("create_hold_sdp failed: {}", e))?;
            session.local_sdp = Some(hold_sdp.clone());
            dialog_adapter
                .send_reinvite_session(&session.session_id, hold_sdp)
                .await?;

            info!("Call {} put on hold", session.session_id);
        }

        Action::CleanupResources => {
            debug!("Cleaning up resources for session {}", session.session_id);
            // TODO: Implement resource cleanup
            warn!("CleanupResources not implemented yet");
        }

        // Registration actions
        Action::SendREGISTER => {
            info!("Action::SendREGISTER for session {}", session.session_id);
            return execute_register_action(
                session,
                dialog_adapter,
                session_store,
                RegisterActionMode::Register,
            )
            .await;
        }

        Action::SendREGISTERWithAuth => {
            info!(
                "Action::SendREGISTERWithAuth for session {}",
                session.session_id
            );
            return execute_register_action(
                session,
                dialog_adapter,
                session_store,
                RegisterActionMode::RegisterWithAuth,
            )
            .await;
        }

        Action::SendUnREGISTER => {
            info!("Action::SendUnREGISTER for session {}", session.session_id);
            return execute_register_action(
                session,
                dialog_adapter,
                session_store,
                RegisterActionMode::Unregister,
            )
            .await;
        }

        Action::StoreAuthChallenge => {
            debug!(
                "Action::StoreAuthChallenge for session {}",
                session.session_id
            );
            // Store the challenge payload stashed in session.pending_auth by the
            // executor (for AuthRequired events). Digest challenges are also
            // parsed into session.auth_challenge for nonce-count/stale handling;
            // non-Digest schemes use the raw challenge string.
            //
            // Fallback: the legacy REGISTER shortcut in DialogAdapter may have
            // already populated session.auth_challenge directly (Phase 2 will
            // remove that path). If pending_auth is None and auth_challenge is
            // already set, treat this action as a no-op.
            if let Some((_, challenge_str)) = session.pending_auth.clone() {
                let previous_nonce = session
                    .auth_challenge
                    .as_ref()
                    .map(|challenge| challenge.nonce.clone());
                session.auth_challenge_raw = Some(challenge_str.clone());
                if let Ok(parsed) =
                    rvoip_auth_core::DigestAuthenticator::parse_challenge_details(&challenge_str)
                {
                    info!(
                        "Stored digest auth challenge for session {} (realm_present={}, realm_bytes={}, nonce_present={}, nonce_bytes={})",
                        session.session_id,
                        !parsed.challenge.realm.is_empty(),
                        parsed.challenge.realm.len(),
                        !parsed.challenge.nonce.is_empty(),
                        parsed.challenge.nonce.len()
                    );
                    session.auth_challenge_stale = parsed.stale;
                    session.auth_challenge_replaces_nonce = previous_nonce;
                    session.auth_challenge = Some(parsed.challenge);
                } else {
                    info!(
                        "Stored non-digest auth challenge for session {}",
                        session.session_id
                    );
                    session.auth_challenge_stale = false;
                    session.auth_challenge_replaces_nonce = previous_nonce;
                    session.auth_challenge = None;
                }
                // Persist so the next action — `SendREGISTERWithAuth` or
                // `SendINVITEWithAuth` — sees the challenge when it re-reads
                // the session from the store inside the dialog adapter.
                // Actions share a mutable local `session`, while the adapter
                // reads the persisted exact cell. Publish only the fields
                // this action owns so concurrent transport/session metadata
                // is not replaced by a stale full-session clone.
                session_store
                    .update_session_with(&session.session_id, |stored| {
                        stored.auth_challenge_raw = session.auth_challenge_raw.clone();
                        stored.auth_challenge_stale = session.auth_challenge_stale;
                        stored.auth_challenge_replaces_nonce =
                            session.auth_challenge_replaces_nonce.clone();
                        stored.auth_challenge = session.auth_challenge.clone();
                    })
                    .await?;
            } else if session.auth_challenge.is_some() {
                debug!("Auth challenge already stored (legacy path); continuing");
            } else {
                return Err(format!(
                    "StoreAuthChallenge: no pending_auth on session {} and no prior challenge",
                    session.session_id
                )
                .into());
            }
        }
        Action::SendINVITEWithAuth => {
            Box::pin(async {
                // RFC 3261 §22.2 — compute an Authorization header and
                // re-issue the INVITE on the same dialog (same Call-ID, bumped
                // CSeq) via DialogAdapter::resend_invite_with_auth. Origin and
                // proxy protection spaces are tracked independently so a 407 may
                // be followed by a 401 while retaining both credentials.
                info!(
                    "Action::SendINVITEWithAuth for session {}",
                    session.session_id
                );
                let (status, challenge_raw) = session.pending_auth.clone().unwrap_or_else(|| {
                    (401, session.auth_challenge_raw.clone().unwrap_or_default())
                });
                if challenge_raw.is_empty() {
                    return Err(format!(
                        "SendINVITEWithAuth: no auth challenge on session {}",
                        session.session_id
                    )
                    .into());
                }
                let request_uri = session.remote_uri.clone().ok_or_else(|| {
                    format!(
                        "SendINVITEWithAuth: no remote_uri on session {}",
                        session.session_id
                    )
                })?;
                let invite_snapshot = session
                    .pending_invite_options
                    .as_ref()
                    .map(|snapshot| (**snapshot).clone());

                use crate::session_store::state::{
                    InviteAuthorizationCredential, InviteCredentialKind,
                };
                let credential_kind = if status == 407 {
                    InviteCredentialKind::Proxy
                } else {
                    InviteCredentialKind::Origin
                };
                let proxy_target = invite_proxy_protection_target(
                    invite_snapshot.as_ref(),
                    dialog_adapter,
                    &request_uri,
                );
                let protection_target = if credential_kind == InviteCredentialKind::Proxy {
                    proxy_target.clone()
                } else {
                    request_uri.clone()
                };
                let auth = session
                    .auth
                    .clone()
                    .or_else(|| session.credentials.clone().map(Into::into))
                    .ok_or_else(|| {
                        Box::new(crate::errors::SessionError::MissingCredentialsForInviteAuth)
                            as Box<dyn std::error::Error + Send + Sync>
                    })?;
                // The builder-supplied body snapshot is the wire authority. SDP
                // generation may also populate `session.local_sdp`, but an
                // auth-int retry must hash and retransmit the exact original bytes.
                let body_owned = authoritative_invite_sdp(
                    invite_snapshot.as_ref(),
                    session.local_sdp.as_deref(),
                );
                let body_bytes = body_owned.as_deref().map(|s| s.as_bytes());
                let transport_context =
                    session.pending_auth_transport.clone().unwrap_or_else(|| {
                        dialog_adapter.outbound_transport_context_for_uri(&request_uri)
                    });

                // Select the challenge first. A response can advertise several
                // schemes and several Digest algorithms; session.auth_challenge is
                // only a legacy parse cache and may describe a different member of
                // that set. Protection-space, stale, and nonce-count bookkeeping
                // must follow the challenge actually selected by SipClientAuth.
                let preview_auth = auth
                    .authorization_for_challenge_with_transport_context(
                        &challenge_raw,
                        "INVITE",
                        &request_uri,
                        1,
                        body_bytes,
                        &transport_context,
                    )
                    .map_err(redacted_invite_auth_error)?;
                let realm = selected_invite_auth_realm(&preview_auth);
                let challenge_nonce = preview_auth
                    .digest_challenge
                    .as_ref()
                    .map(|challenge| challenge.nonce.clone());
                let existing_credential = invite_credential_slot_for_challenge(
                    &session.invite_authorization_credentials,
                    credential_kind,
                    &protection_target,
                    &realm,
                    challenge_nonce.as_deref(),
                    preview_auth.stale,
                )
                .map_err(|()| crate::errors::SessionError::InviteAuthRetryExhausted)?;

                // RFC 7616 §3.4.5 — increment the counter for the selected
                // (realm, nonce), not whichever challenge happened to be parsed
                // first by the state-machine cache.
                let nc_value = if let Some(challenge) = preview_auth.digest_challenge.as_ref() {
                    let nc_key = (challenge.realm.clone(), challenge.nonce.clone());
                    *session
                        .digest_nc
                        .entry(nc_key)
                        .and_modify(|n| *n += 1)
                        .or_insert(1)
                } else {
                    1
                };
                let selected_auth = if preview_auth.digest_challenge.is_some() && nc_value != 1 {
                    auth.authorization_for_challenge_with_transport_context(
                        &challenge_raw,
                        "INVITE",
                        &request_uri,
                        nc_value,
                        body_bytes,
                        &transport_context,
                    )
                    .map_err(redacted_invite_auth_error)?
                } else {
                    preview_auth
                };
                session.invite_auth_retry_count = session.invite_auth_retry_count.saturating_add(1);
                let header_value = selected_auth.value;

                let stale_refreshes = existing_credential
                    .map(|index| {
                        session.invite_authorization_credentials[index]
                            .stale_refreshes
                            .saturating_add(1)
                    })
                    .unwrap_or(0);
                let credential = InviteAuthorizationCredential {
                    kind: credential_kind,
                    protection_target,
                    challenge_raw: challenge_raw.clone(),
                    realm,
                    nonce: challenge_nonce,
                    stale_refreshes,
                    value: header_value,
                };
                if let Some(index) = existing_credential {
                    session.invite_authorization_credentials[index] = credential;
                } else {
                    session.invite_authorization_credentials.push(credential);
                }

                session.pending_auth.take();
                session.pending_auth_transport = None;
                let header_name = if status == 407 {
                    "Proxy-Authorization"
                } else {
                    "Authorization"
                };

                // SIP_API_DESIGN_2 §7.3 / Phase B — rebuild the FULL per-call
                // override set from the persisted INVITE stash so the authenticated
                // retry's wire form matches the initial INVITE. The snapshot
                // survives the auth-retry hop (the stash isn't consumed until the
                // final response), so we re-run the same `materialize_invite_options`
                // mapping rather than forwarding raw `extra_headers` alone — which
                // is what used to drop with_pai / with_subject / with_from_display /
                // with_contact_uri on the 401/407 retry that actually completes the
                // call. Transfer-leg / internal paths leave the stash empty.
                let mut authorization_headers =
                    retained_invite_authorization_headers(session, &request_uri, &proxy_target)?;

                let invite_opts = match invite_snapshot.as_ref() {
                    Some(snapshot) => {
                        if !session
                            .invite_authorization_credentials
                            .iter()
                            .any(|credential| {
                                credential.kind == InviteCredentialKind::Origin
                                    && credential.protection_target == request_uri
                            })
                            && snapshot.to == request_uri
                        {
                            if let Some(precomputed) = snapshot.precomputed_auth.clone() {
                                authorization_headers.push(
                                    rvoip_sip_core::validation::validated_authorization_header(
                                        rvoip_sip_core::types::HeaderName::Authorization,
                                        precomputed,
                                    )
                                    .map_err(|_| {
                                        crate::errors::SessionError::ProtocolError(
                                            "precomputed INVITE authorization failed validation"
                                                .to_string(),
                                        )
                                    })?,
                                );
                            }
                        }
                        materialize_invite_options(
                            snapshot,
                            session.pai_uri.as_deref(),
                            body_owned.clone(),
                        )?
                        .0
                    }
                    None => rvoip_sip_dialog::api::unified::InviteRequestOptions {
                        sdp: body_owned.clone(),
                        ..Default::default()
                    },
                };
                let apply_global_proxy = invite_snapshot.as_ref().is_none_or(|snapshot| {
                    matches!(
                        snapshot.outbound_proxy_override,
                        crate::api::send::ProxyOverride::Default
                    )
                });

                dialog_adapter
                    .resend_invite_with_auth(
                        &session.session_id,
                        rvoip_sip_dialog::api::unified::InviteAuthRetryOptions {
                            sdp: body_owned,
                            authorization_headers,
                            extra_headers: invite_opts.extra_headers,
                            from_display: invite_opts.from_display,
                            contact_uri: invite_opts.contact_uri,
                            outbound_proxy_uri: invite_opts.outbound_proxy_uri,
                            supported_100rel: invite_opts.supported_100rel,
                        },
                        apply_global_proxy,
                    )
                    .await?;
                info!(
                    "Auth-retry INVITE sent for session {} (retry #{}, header {})",
                    session.session_id, session.invite_auth_retry_count, header_name
                );
                Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
            })
            .await?;
        }

        Action::SendRequestWithAuth => {
            // SIP_API_DESIGN_2 R2 — auth-retry for non-INVITE/non-REGISTER
            // methods. Reads `session.pending_auth_method` to discriminate
            // which `pending_<method>_options` to re-issue (falls back to
            // inspecting which stash is set when method is missing or
            // empty), computes the selected auth scheme, and dispatches via
            // the matching `DialogAdapter::send_<method>_with_auth`.
            info!(
                "Action::SendRequestWithAuth for session {} (method={})",
                session.session_id,
                session
                    .pending_auth_method
                    .as_deref()
                    .map(safe_outbound_auth_method_label)
                    .unwrap_or("unset")
            );
            const CAP: u8 = 1;
            // Resolve exact method ownership before evaluating the retry
            // budget. INFO/REFER/NOTIFY/UPDATE carry an independent budget in
            // their tracker entry; BYE and legacy OOB methods retain the
            // compatibility session-level counter.
            let method = resolve_auth_method(session);
            let tracked_method = TrackedInDialogMethod::from_label(&method);
            let challenged_transaction = if tracked_method.is_some() {
                let transaction_id = session.pending_auth_transaction_id.as_deref().ok_or_else(
                    || {
                        crate::errors::SessionError::InvalidTransition(format!(
                            "SendRequestWithAuth({method}): exact challenged transaction is unavailable"
                        ))
                    },
                )?;
                Some(
                    transaction_id
                        .parse::<rvoip_sip_dialog::transaction::TransactionKey>()
                        .map_err(|_| {
                            crate::errors::SessionError::InvalidTransition(format!(
                                "SendRequestWithAuth({method}): challenged transaction is invalid"
                            ))
                        })?,
                )
            } else {
                None
            };
            let (retry_count, tracked_last_nonce) =
                if let (Some(tracked_method), Some(transaction)) =
                    (tracked_method, challenged_transaction.as_ref())
                {
                    dialog_adapter
                        .outbound_request_tracker
                        .auth_retry_state_for_transaction(
                            &session.session_id,
                            tracked_method,
                            transaction,
                        )?
                } else {
                    (session.request_auth_retry_count, None)
                };
            let replaces_nonce = if tracked_method.is_some() {
                tracked_last_nonce.as_deref()
            } else {
                session.auth_challenge_replaces_nonce.as_deref()
            };
            if !auth_retry_allowed(
                retry_count,
                CAP,
                session.auth_challenge.as_ref(),
                session.auth_challenge_stale,
                replaces_nonce,
            ) {
                return Err(Box::new(
                    crate::errors::SessionError::RequestAuthRetryExhausted {
                        method: auth_method_for_error(&method),
                    },
                ));
            }
            if tracked_method.is_none() {
                session.request_auth_retry_count += 1;
            }

            let (status, challenge_raw) = session
                .pending_auth
                .clone()
                .unwrap_or_else(|| (401, session.auth_challenge_raw.clone().unwrap_or_default()));
            if challenge_raw.is_empty() {
                return Err(format!(
                    "SendRequestWithAuth: no auth challenge on session {}",
                    session.session_id
                )
                .into());
            }
            let auth = session
                .auth
                .clone()
                .or_else(|| session.credentials.clone().map(Into::into))
                .ok_or_else(|| {
                    Box::new(
                        crate::errors::SessionError::MissingCredentialsForRequestAuth {
                            method: auth_method_for_error(&method),
                        },
                    ) as Box<dyn std::error::Error + Send + Sync>
                })?;

            session.pending_auth.take();
            let header_name = if status == 407 {
                "Proxy-Authorization"
            } else {
                "Authorization"
            };

            // Digest HA2 must use the exact challenged request URI. The typed
            // dialog event supplies it for every tracked in-dialog request and
            // BYE; never reconstruct those targets from mutable dialog/session
            // metadata. OOB compatibility methods retain their target in the
            // authoritative builder stash.
            let request_uri = if tracked_method.is_some() || method == "BYE" {
                session.pending_auth_request_uri.clone().ok_or_else(|| {
                    crate::errors::SessionError::InvalidTransition(format!(
                        "SendRequestWithAuth({method}): exact challenged request URI is unavailable"
                    ))
                })?
            } else {
                resolve_auth_request_uri(session, &method).ok_or_else(|| {
                    format!(
                        "SendRequestWithAuth: no request_uri for method {} on session {}",
                        method, session.session_id
                    )
                })?
            };

            // RFC 7616 §3.4.5 — per-(realm, nonce) NC counter.
            let digest_challenge_for_nc = session.auth_challenge.clone().or_else(|| {
                rvoip_auth_core::DigestAuthenticator::parse_challenge(&challenge_raw).ok()
            });
            let nc_value = if let Some(challenge) = digest_challenge_for_nc.as_ref() {
                let nc_key = (challenge.realm.clone(), challenge.nonce.clone());
                *session
                    .digest_nc
                    .entry(nc_key)
                    .and_modify(|n| *n += 1)
                    .or_insert(1)
            } else {
                1
            };

            // RFC 7616 auth-int signs the exact challenged entity body. For
            // tracked in-dialog requests read immutable INFO/NOTIFY/UPDATE
            // bytes from the exact transaction-owned snapshot; never rebuild
            // them from mutable SessionState. Legacy OOB MESSAGE retains its
            // authoritative body in its compatibility stash.
            let body_bytes_owned: Option<bytes::Bytes> =
                if let (Some(tracked_method), Some(transaction)) =
                    (tracked_method, challenged_transaction.as_ref())
                {
                    dialog_adapter
                        .outbound_request_tracker
                        .request_body_for_transaction(
                            &session.session_id,
                            tracked_method,
                            transaction,
                        )?
                } else {
                    match method.as_str() {
                        "MESSAGE" => session
                            .pending_message_options
                            .as_ref()
                            .map(|options| options.body.clone()),
                        _ => None,
                    }
                };
            let body_bytes_ref = body_bytes_owned.as_deref();

            let transport_context = session
                .pending_auth_transport
                .clone()
                .unwrap_or_else(|| dialog_adapter.outbound_transport_context_for_uri(&request_uri));
            let selected_auth = auth
                .authorization_for_challenge_with_transport_context(
                    &challenge_raw,
                    &method,
                    &request_uri,
                    nc_value,
                    body_bytes_ref,
                    &transport_context,
                )
                .map_err(|error| {
                    crate::errors::redacted_outbound_auth_error(
                        crate::errors::OutboundAuthOperation::Request,
                        error,
                    )
                })?;
            let header_value = selected_auth.value;
            let challenge_nonce = session
                .auth_challenge
                .as_ref()
                .map(|challenge| challenge.nonce.clone());
            session.pending_auth_transport = None;

            // Dispatch per method. Each branch reads the matching
            // `pending_<method>_options` stash so the application
            // extras / typed parameters ride the retry.
            match method.as_str() {
                "BYE" => {
                    let opts = session
                        .pending_bye_options
                        .as_ref()
                        .map(|a| (**a).clone())
                        .ok_or_else(|| {
                            format!(
                                "SendRequestWithAuth(BYE): no pending_bye_options for session {}",
                                session.session_id
                            )
                        })?;
                    dialog_adapter
                        .send_bye_with_auth(&session.session_id, opts, header_name, header_value)
                        .await?;
                }
                "REFER" => {
                    let challenged_transaction = challenged_transaction.as_ref().ok_or_else(|| {
                        crate::errors::SessionError::InvalidTransition(
                            "SendRequestWithAuth(REFER): exact challenged transaction is unavailable"
                                .to_string(),
                        )
                    })?;
                    let (lease, options) = dialog_adapter.outbound_request_tracker.prepare_retry(
                        &session.session_id,
                        TrackedInDialogMethod::Refer,
                        challenged_transaction,
                        challenge_nonce.clone(),
                    )?;
                    let TrackedInDialogOptions::Refer(options) = options else {
                        return Err(
                            "SendRequestWithAuth(REFER): tracker option type mismatch".into()
                        );
                    };
                    let transaction_id = dialog_adapter
                        .send_refer_with_auth(
                            &session.session_id,
                            (*options).clone(),
                            header_name,
                            header_value,
                        )
                        .await?;
                    dialog_adapter
                        .outbound_request_tracker
                        .activate(lease, transaction_id.clone())?;
                    advance_tracked_auth_owner(
                        session,
                        session_store,
                        TrackedInDialogMethod::Refer,
                        challenged_transaction,
                        &transaction_id,
                        &request_uri,
                    )
                    .await;
                }
                "NOTIFY" => {
                    let challenged_transaction = challenged_transaction.as_ref().ok_or_else(|| {
                        crate::errors::SessionError::InvalidTransition(
                            "SendRequestWithAuth(NOTIFY): exact challenged transaction is unavailable"
                                .to_string(),
                        )
                    })?;
                    let (lease, options) = dialog_adapter.outbound_request_tracker.prepare_retry(
                        &session.session_id,
                        TrackedInDialogMethod::Notify,
                        challenged_transaction,
                        challenge_nonce.clone(),
                    )?;
                    let TrackedInDialogOptions::Notify(options) = options else {
                        return Err(
                            "SendRequestWithAuth(NOTIFY): tracker option type mismatch".into()
                        );
                    };
                    let transaction_id = dialog_adapter
                        .send_notify_with_auth(
                            &session.session_id,
                            (*options).clone(),
                            header_name,
                            header_value,
                        )
                        .await?;
                    dialog_adapter
                        .outbound_request_tracker
                        .activate(lease, transaction_id.clone())?;
                    advance_tracked_auth_owner(
                        session,
                        session_store,
                        TrackedInDialogMethod::Notify,
                        challenged_transaction,
                        &transaction_id,
                        &request_uri,
                    )
                    .await;
                }
                "INFO" => {
                    let challenged_transaction =
                        challenged_transaction.as_ref().ok_or_else(|| {
                            crate::errors::SessionError::InvalidTransition(
                            "SendRequestWithAuth(INFO): exact challenged transaction is unavailable"
                                .to_string(),
                        )
                        })?;
                    let (lease, options) = dialog_adapter.outbound_request_tracker.prepare_retry(
                        &session.session_id,
                        TrackedInDialogMethod::Info,
                        challenged_transaction,
                        challenge_nonce.clone(),
                    )?;
                    let TrackedInDialogOptions::Info(options) = options else {
                        return Err(
                            "SendRequestWithAuth(INFO): tracker option type mismatch".into()
                        );
                    };
                    let transaction_id = dialog_adapter
                        .send_info_with_auth(
                            &session.session_id,
                            (*options).clone(),
                            header_name,
                            header_value,
                        )
                        .await?;
                    dialog_adapter
                        .outbound_request_tracker
                        .activate(lease, transaction_id.clone())?;
                    advance_tracked_auth_owner(
                        session,
                        session_store,
                        TrackedInDialogMethod::Info,
                        challenged_transaction,
                        &transaction_id,
                        &request_uri,
                    )
                    .await;
                }
                "UPDATE" => {
                    let challenged_transaction = challenged_transaction.as_ref().ok_or_else(|| {
                        crate::errors::SessionError::InvalidTransition(
                            "SendRequestWithAuth(UPDATE): exact challenged transaction is unavailable"
                                .to_string(),
                        )
                    })?;
                    let (lease, options) = dialog_adapter.outbound_request_tracker.prepare_retry(
                        &session.session_id,
                        TrackedInDialogMethod::Update,
                        challenged_transaction,
                        challenge_nonce,
                    )?;
                    let TrackedInDialogOptions::Update(options) = options else {
                        return Err(
                            "SendRequestWithAuth(UPDATE): tracker option type mismatch".into()
                        );
                    };
                    let transaction_id = dialog_adapter
                        .send_update_with_auth(
                            &session.session_id,
                            (*options).clone(),
                            header_name,
                            header_value,
                        )
                        .await?;
                    dialog_adapter
                        .outbound_request_tracker
                        .activate(lease, transaction_id.clone())?;
                    advance_tracked_auth_owner(
                        session,
                        session_store,
                        TrackedInDialogMethod::Update,
                        challenged_transaction,
                        &transaction_id,
                        &request_uri,
                    )
                    .await;
                }
                "MESSAGE" => {
                    let opts = session
                        .pending_message_options
                        .as_ref()
                        .map(|a| (**a).clone())
                        .ok_or_else(|| {
                            format!(
                                "SendRequestWithAuth(MESSAGE): no pending_message_options for session {}",
                                session.session_id
                            )
                        })?;
                    let _resp = dialog_adapter
                        .send_message_oob_with_auth(opts, header_name, header_value)
                        .await?;
                }
                "OPTIONS" => {
                    let opts = session
                        .pending_options_options
                        .as_ref()
                        .map(|a| (**a).clone())
                        .ok_or_else(|| {
                            format!(
                                "SendRequestWithAuth(OPTIONS): no pending_options_options for session {}",
                                session.session_id
                            )
                        })?;
                    let _resp = dialog_adapter
                        .send_options_oob_with_auth(opts, header_name, header_value)
                        .await?;
                }
                "SUBSCRIBE" => {
                    let opts_arc =
                        session.pending_subscribe_options.as_ref().ok_or_else(|| {
                            format!(
                                "SendRequestWithAuth(SUBSCRIBE): no pending_subscribe_options for session {}",
                                session.session_id
                            )
                        })?;
                    let target = session.remote_uri.clone().ok_or_else(|| {
                        format!(
                            "SendRequestWithAuth(SUBSCRIBE): no remote_uri on session {}",
                            session.session_id
                        )
                    })?;
                    let opts = (**opts_arc).clone();
                    let _resp = dialog_adapter
                        .send_subscribe_oob_with_auth(&target, opts, header_name, header_value)
                        .await?;
                }
                other => {
                    return Err(format!(
                        "SendRequestWithAuth: unsupported method {} for session {}",
                        other, session.session_id
                    )
                    .into());
                }
            }

            info!(
                "Auth-retry {} sent for session {} (retry #{}, header {})",
                method,
                session.session_id,
                retry_count.saturating_add(1),
                header_name
            );
        }

        Action::SendINVITEWithBumpedSessionExpires => {
            Box::pin(async {
            // RFC 4028 §6 — on 422 Session Interval Too Small the UAS's
            // `Min-SE` header dictates the required floor. Bump the retry
            // counter, enforce the 2-attempt cap, and re-issue the INVITE
            // with the peer's Min-SE as both our Session-Expires and Min-SE.
            // Mirrors the 423 REGISTER retry at
            // `adapters/dialog_adapter.rs:756-800` but goes through the state
            // machine (INVITE interacts with call state in ways REGISTER
            // doesn't). Errors out when the cap is exceeded so the failure
            // path surfaces a clean `CallFailed(422)` to the app.
            const CAP: u8 = 2;
            if session.session_timer_retry_count >= CAP {
                return Err(format!(
                    "422 session-timer retry cap ({}) exceeded for session {}",
                    CAP, session.session_id
                )
                .into());
            }

            let min_se = session.session_timer_min_se.ok_or_else(|| {
                format!(
                    "SendINVITEWithBumpedSessionExpires: no Min-SE cached on session {}",
                    session.session_id
                )
            })?;

            session.session_timer_retry_count += 1;
            info!(
                "🔄 422 Session Interval Too Small — retrying INVITE for session {} with Session-Expires={}s / Min-SE={}s (attempt {}/{})",
                session.session_id, min_se, min_se, session.session_timer_retry_count, CAP
            );

            let request_uri = session.remote_uri.clone().ok_or_else(|| {
                format!(
                    "SendINVITEWithBumpedSessionExpires: no remote_uri on session {}",
                    session.session_id
                )
            })?;
            let snapshot = session
                .pending_invite_options
                .as_ref()
                .map(|snapshot| (**snapshot).clone());
            let body = authoritative_invite_sdp(snapshot.as_ref(), session.local_sdp.as_deref());
            let proxy_target =
                invite_proxy_protection_target(snapshot.as_ref(), dialog_adapter, &request_uri);
            let mut authorization_headers =
                retained_invite_authorization_headers(session, &request_uri, &proxy_target)?;
            let invite_opts = if let Some(snapshot) = snapshot.as_ref() {
                if !session
                    .invite_authorization_credentials
                    .iter()
                    .any(|credential| {
                        credential.kind == crate::session_store::state::InviteCredentialKind::Origin
                            && credential.protection_target == request_uri
                    })
                    && snapshot.to == request_uri
                {
                    if let Some(precomputed) = snapshot.precomputed_auth.clone() {
                        authorization_headers.push(
                            rvoip_sip_core::validation::validated_authorization_header(
                                HeaderName::Authorization,
                                precomputed,
                            )
                            .map_err(|_| {
                                crate::errors::SessionError::ProtocolError(
                                    "precomputed INVITE authorization failed validation"
                                        .to_string(),
                                )
                            })?,
                        );
                    }
                }
                materialize_invite_options(snapshot, session.pai_uri.as_deref(), body.clone())?.0
            } else {
                rvoip_sip_dialog::api::unified::InviteRequestOptions {
                    sdp: body.clone(),
                    ..Default::default()
                }
            };
            let apply_global_proxy = snapshot.as_ref().is_none_or(|snapshot| {
                matches!(
                    snapshot.outbound_proxy_override,
                    crate::api::send::ProxyOverride::Default
                )
            });

            dialog_adapter
                .resend_invite_with_session_timer_override(
                    &session.session_id,
                    rvoip_sip_dialog::api::unified::InviteAuthRetryOptions {
                        sdp: body,
                        authorization_headers,
                        extra_headers: invite_opts.extra_headers,
                        from_display: invite_opts.from_display,
                        contact_uri: invite_opts.contact_uri,
                        outbound_proxy_uri: invite_opts.outbound_proxy_uri,
                        supported_100rel: invite_opts.supported_100rel,
                    },
                    apply_global_proxy,
                    min_se,
                    min_se,
                )
                .await?;
            Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
            })
            .await?;
        }
        Action::ProcessRegistrationResponse => {
            debug!(
                "Processing registration response for session {}",
                session.session_id
            );
            // Response processing is handled by events from dialog adapter
            // This action is a placeholder for any additional processing needed
        }

        // Subscription actions
        Action::SendSUBSCRIBE => {
            info!("Action::SendSUBSCRIBE for session {}", session.session_id);
            let from_uri = session
                .local_uri
                .as_deref()
                .ok_or_else(|| "local_uri not set for subscription".to_string())?;
            let to_uri = session
                .remote_uri
                .as_deref()
                .ok_or_else(|| "to_uri not set for subscription".to_string())?;
            let event_package = "presence"; // Default to presence, could be stored in session
            let expires = 3600; // Default 1 hour subscription
            if let Some(follow_up) = dialog_adapter
                .send_subscribe(
                    &session.session_id,
                    from_uri,
                    to_uri,
                    event_package,
                    expires,
                )
                .await?
            {
                return Ok(ActionOutcome::with_event(follow_up));
            }
        }
        Action::ProcessNOTIFY => {
            debug!("Processing NOTIFY for session {}", session.session_id);
            // NOTIFY processing is handled by events from dialog adapter
            // This action is a placeholder for any additional processing needed
        }
        // Action::SendNOTIFY deleted per SIP_API_DESIGN_2.md Phase 5 —
        // consolidated into Action::SendNOTIFYWithOptions. YAML emit
        // rows updated to reference SendNOTIFYWithOptions.

        // Message actions
        Action::SendMESSAGE => {
            info!("Action::SendMESSAGE for session {}", session.session_id);
            let from_uri = session
                .local_uri
                .as_deref()
                .ok_or_else(|| "local_uri not set for message".to_string())?;
            let to_uri = session
                .remote_uri
                .as_deref()
                .ok_or_else(|| "to_uri not set for message".to_string())?;
            // Get message body from session (could be stored in a specific field)
            let body = session
                .local_sdp
                .clone()
                .unwrap_or_else(|| "Test message".to_string());
            let in_dialog = session.dialog_id.is_some(); // Send in-dialog if we have a dialog
            if let Some(follow_up) = dialog_adapter
                .send_message(&session.session_id, from_uri, to_uri, body, in_dialog)
                .await?
            {
                return Ok(ActionOutcome::with_event(follow_up));
            }
        }
        Action::ProcessMESSAGE => {
            debug!("Processing MESSAGE for session {}", session.session_id);
            // MESSAGE processing is handled by events from dialog adapter
            // This action is a placeholder for any additional processing needed
        }

        // Generic cleanup actions
        Action::CleanupDialog => {
            debug!("Cleaning up dialog for session {}", session.session_id);
            if session.dialog_id.is_some() {
                cleanup_dialog_on_fresh_task(
                    Arc::clone(dialog_adapter),
                    session.session_id.clone(),
                )
                .await?;
            }
        }
        Action::CleanupMedia => {
            // NEXT_STEPS B.1 diag — promoted from debug! to info! so the
            // perf_listener log shows whether this action fires at all.
            // If the listener prints `cleaned_total=0` but this line is
            // present in the log we know the action ran but
            // cleanup_session bailed; if both are absent the BYE event
            // never matched the {Active, DialogBYE} row.
            info!(
                "Action::CleanupMedia firing for session {} (media_session_id={:?})",
                session.session_id, session.media_session_id
            );
            // Always call cleanup_session — the adapter is idempotent and
            // media-core may still have state even when our `media_session_id`
            // field looks empty (e.g. a previous cleanup cleared the field
            // but stop_media hasn't landed yet).
            media_adapter.cleanup_session(&session.session_id).await?;
            // Reset field so the subsequent CreateMediaSession (in a redirect
            // transition) doesn't trip the idempotency guard that now lives
            // in GenerateLocalSDP / NegotiateSDPAsUAS (added for
            // accept_call_with_sdp).
            session.media_session_id = None;
            session.media_session_ready = false;
            session.sdp_negotiated = false;
            session.local_sdp = None;
            session.negotiated_config = None;
        }

        // ===== REFER Response Action =====
        Action::SendReferAccepted => {
            debug!("Sending 202 Accepted for REFER request");

            let transaction_id = session
                .refer_transaction_id
                .clone()
                .unwrap_or_else(|| "unknown".to_string());

            // Send ReferResponse event back to dialog-core via global event bus
            let refer_response =
                rvoip_infra_common::events::cross_crate::SessionToDialogEvent::ReferResponse {
                    transaction_id,
                    accept: true,
                    status_code: 202,
                    reason: "Accepted".to_string(),
                };

            let event =
                rvoip_infra_common::events::cross_crate::RvoipCrossCrateEvent::SessionToDialog(
                    refer_response,
                );

            // Get global coordinator from dialog adapter
            if dialog_adapter
                .global_coordinator
                .publish(Arc::new(event))
                .await
                .is_err()
            {
                error!("Failed to publish ReferResponse event (class=coordination)");
            } else {
                debug!("Published ReferResponse (202 Accepted) event to dialog-core");
            }
        }

        // ===== RFC 3515 §2.4.5 Transfer-Progress NOTIFYs =====
        Action::SendRefer100Trying => {
            // Fires on the REFER-receiving session's OWN dialog (not via
            // transferor linkage — the receiver and transferor are the
            // same session in this arm). RFC 3515 §2.4.5: "The transferee
            // SHOULD send a NOTIFY with a `message/sipfrag` body of
            // `SIP/2.0 100 Trying` upon accepting the REFER" — this is
            // the acceptance ack of the implicit subscription, not a
            // dialog-progress NOTIFY, so it has no linkage dependency.
            debug!("SendRefer100Trying on session {}", session.session_id);
            if dialog_adapter
                .send_refer_notify(&session.session_id, 100, "Trying")
                .await
                .is_err()
            {
                warn!(session = %session.session_id, "Failed to send 100 Trying NOTIFY");
            }
        }

        Action::SendTransferNotifyRinging => {
            if let Some(transferor) = session.transferor_session_id.clone() {
                debug!(
                    "SendTransferNotifyRinging: leg {} -> transferor {}",
                    session.session_id, transferor
                );
                if dialog_adapter
                    .send_refer_notify(&transferor, 180, "Ringing")
                    .await
                    .is_err()
                {
                    warn!(transferor = %transferor, "Failed to send 180 Ringing NOTIFY");
                }
                publish_transfer_event(
                    dialog_adapter,
                    Event::ReferNotify {
                        call_id: transferor.clone(),
                        status_code: 180,
                        reason: "Ringing".to_string(),
                        subscription_state: None,
                        body: Some("SIP/2.0 180 Ringing\r\n".to_string()),
                    },
                );
                publish_transfer_event(
                    dialog_adapter,
                    Event::ReferProgress {
                        call_id: transferor,
                        status_code: 180,
                        reason: "Ringing".to_string(),
                    },
                );
            } else {
                debug!(
                    "SendTransferNotifyRinging on non-transfer session {} — no-op",
                    session.session_id
                );
            }
        }

        Action::SendTransferNotifySuccess => {
            if let Some(transferor) = session.transferor_session_id.clone() {
                debug!(
                    "SendTransferNotifySuccess: leg {} -> transferor {}",
                    session.session_id, transferor
                );
                if dialog_adapter
                    .send_refer_notify(&transferor, 200, "OK")
                    .await
                    .is_err()
                {
                    warn!(transferor = %transferor, "Failed to send 200 OK NOTIFY");
                }
                publish_transfer_event(
                    dialog_adapter,
                    Event::ReferNotify {
                        call_id: transferor.clone(),
                        status_code: 200,
                        reason: "OK".to_string(),
                        subscription_state: None,
                        body: Some("SIP/2.0 200 OK\r\n".to_string()),
                    },
                );
                publish_transfer_event(
                    dialog_adapter,
                    Event::TransferTargetAnswered {
                        transfer_call_id: transferor.clone(),
                        target_uri: session.remote_uri.clone().unwrap_or_default(),
                        evidence: crate::api::events::TransferTargetEvidence::LocalTargetLeg {
                            call_id: session.session_id.clone(),
                        },
                    },
                );
                publish_transfer_event(
                    dialog_adapter,
                    Event::ReferCompleted {
                        call_id: transferor,
                        target: session.remote_uri.clone().unwrap_or_default(),
                        status_code: 200,
                        reason: "OK".to_string(),
                    },
                );
            } else {
                debug!(
                    "SendTransferNotifySuccess on non-transfer session {} — no-op",
                    session.session_id
                );
            }
        }

        Action::SendTransferNotifyFailure => {
            if let Some(transferor) = session.transferor_session_id.clone() {
                // We don't currently stash the non-2xx status code on
                // `SessionState` mid-failure, so the progress NOTIFY
                // carries a coarse 500. The transferor still gets a
                // terminal `TransferFailed` signal; the b2bua crate can
                // narrow the reason once SessionState grows a
                // `last_failure_status` field.
                let status_code: u16 = 500;
                let reason = "Transfer leg failed".to_string();
                debug!(
                    "SendTransferNotifyFailure: leg {} -> transferor {} ({} {})",
                    session.session_id, transferor, status_code, reason
                );
                if dialog_adapter
                    .send_refer_notify(&transferor, status_code, &reason)
                    .await
                    .is_err()
                {
                    warn!(
                        status_code,
                        transferor = %transferor,
                        "Failed to send transfer-failure NOTIFY"
                    );
                }
                publish_transfer_event(
                    dialog_adapter,
                    Event::ReferNotify {
                        call_id: transferor.clone(),
                        status_code,
                        reason: reason.clone(),
                        subscription_state: None,
                        body: Some(format!("SIP/2.0 {} {}\r\n", status_code, reason)),
                    },
                );
                publish_transfer_event(
                    dialog_adapter,
                    Event::TransferFailed {
                        call_id: transferor,
                        reason,
                        status_code,
                    },
                );
            } else {
                debug!(
                    "SendTransferNotifyFailure on non-transfer session {} — no-op",
                    session.session_id
                );
            }
        }

        // ──────────────────────────────────────────────────────────────
        // SIP_API_DESIGN_2 §7.1 / §7.3 — Unified outbound dispatch
        // through the option stash.
        //
        // Each handler reads `session.pending_<method>_options` with
        // `.take()`, so the stash is consumed-on-dispatch. This
        // matches the Phase 2 lifecycle: builder `.send()` stages the
        // slot (with the §7.3 invariant #5 conflict guard), the
        // matching `EventType::SendOutbound<METHOD>` queues, the
        // handler below dispatches via the dialog-adapter mirror and
        // the slot returns to `None`. A second `.send()` for the same
        // method is then immediately allowed — concurrent overlaps
        // are still rejected by the conflict guard at stage time.
        //
        // Phase 4 (auth-retry) will reintroduce `.clone()` semantics
        // alongside per-method response correlation so the same
        // snapshot can drive a 401 retransmit. Until that lands, the
        // `Send<METHOD>WithAuth` actions read their own session state
        // (auth_challenge / credentials) rather than the stash.
        //
        // §7.4 precedence (stash wins over auto-emit) on BYE / NOTIFY /
        // CANCEL lives in the auto-emit handlers above
        // (`Action::SendBYE`, `Action::SendCANCEL`, `Action::SendNOTIFY`).
        // ──────────────────────────────────────────────────────────────
        // SIP_API_DESIGN_2 §7.3 — R2: snapshot-then-clear-after-dispatch.
        // Mirrors `execute_register_action`'s `.as_ref().clone()` pattern
        // so the application-staged extras stay readable for the entire
        // duration of `send_X_with_options(...)`. Today these dialog
        // adapter calls do not internally drive 401/407 retries for the
        // non-INVITE/non-REGISTER methods; when that auth-retry plumbing
        // lands the snapshot will already be available. The post-dispatch
        // `= None` mirrors today's `.take()` semantics for the success
        // path, and the `Terminated` backstop in `executor.rs:533-546`
        // still sweeps the slot on session teardown if a dispatch errors
        // out unexpectedly.
        Action::SendBYEWithOptions => {
            let snapshot = session
                .pending_bye_options
                .as_ref()
                .map(|opts| (**opts).clone())
                .ok_or_else(|| {
                    format!(
                        "SendBYEWithOptions: no pending_bye_options for session {}",
                        session.session_id
                    )
                })?;
            if let Err(error) = dialog_adapter
                .send_bye_with_options(&session.session_id, snapshot)
                .await
            {
                // No exact transaction exists to drive terminal cleanup when
                // dispatch itself fails. Release the builder slot immediately.
                session.pending_bye_options = None;
                let _ = session_store
                    .update_session_with(&session.session_id, |stored| {
                        stored.pending_bye_options = None;
                    })
                    .await;
                return Err(error.into());
            }
            // Keep the immutable options until the exact BYE final-response
            // owner releases the session. A 401/407 retry must reproduce the
            // same application extras before adding stack-owned auth.
        }
        Action::SendCANCELWithOptions => {
            // Phase 5 — single CANCEL action: stash wins; otherwise fall
            // back to `Config.auto_emit_extra_headers` (operators stamp
            // tenant/trace headers on every CANCEL); else legacy fast
            // path. Consolidated from the deleted `Action::SendCANCEL`.
            if let Some(opts_arc) = session.pending_cancel_options.as_ref() {
                let opts = (**opts_arc).clone();
                dialog_adapter
                    .send_cancel_with_options(&session.session_id, opts)
                    .await?;
                session.pending_cancel_options = None;
            } else {
                let auto_extras = dialog_adapter.auto_emit_extra_headers.clone();
                if auto_extras.is_empty() {
                    dialog_adapter.send_cancel(&session.session_id).await?;
                } else {
                    let opts = rvoip_sip_dialog::api::unified::CancelRequestOptions {
                        reason: None,
                        extra_headers: auto_extras,
                    };
                    dialog_adapter
                        .send_cancel_with_options(&session.session_id, opts)
                        .await?;
                }
            }
        }
        Action::SendREFERWithOptions => {
            let TrackedInDialogOptions::Refer(options) = claim_tracked_request_staging(
                session,
                session_store,
                TrackedInDialogMethod::Refer,
                stage_claim,
            )
            .await?
            else {
                return Err(crate::errors::SessionError::InvalidTransition(
                    "SendREFERWithOptions claimed the wrong method".to_string(),
                )
                .into());
            };
            let lease = dialog_adapter.outbound_request_tracker.prepare(
                &session.session_id,
                TrackedInDialogOptions::Refer(Arc::clone(&options)),
            )?;
            let transaction_id = dialog_adapter
                .send_refer_with_options(&session.session_id, (*options).clone())
                .await?;
            dialog_adapter
                .outbound_request_tracker
                .activate(lease, transaction_id)?;
        }
        Action::SendNOTIFYWithOptions => {
            // Phase 5 — single NOTIFY action: stash wins; otherwise
            // consult `Config.auto_emit_extra_headers` so operator
            // headers ride every stack-emitted NOTIFY. Consolidated from
            // the deleted `Action::SendNOTIFY`.
            if stage_claim.is_some() || session.pending_notify_options.is_some() {
                let TrackedInDialogOptions::Notify(options) = claim_tracked_request_staging(
                    session,
                    session_store,
                    TrackedInDialogMethod::Notify,
                    stage_claim,
                )
                .await?
                else {
                    return Err(crate::errors::SessionError::InvalidTransition(
                        "SendNOTIFYWithOptions claimed the wrong method".to_string(),
                    )
                    .into());
                };
                let lease = dialog_adapter.outbound_request_tracker.prepare(
                    &session.session_id,
                    TrackedInDialogOptions::Notify(Arc::clone(&options)),
                )?;
                let transaction_id = dialog_adapter
                    .send_notify_with_options(&session.session_id, (*options).clone())
                    .await?;
                dialog_adapter
                    .outbound_request_tracker
                    .activate(lease, transaction_id)?;
            } else if matches!(triggering_event, EventType::SendOutboundNotify) {
                return Err(crate::errors::SessionError::InvalidTransition(
                    "SendNOTIFYWithOptions requires exact staged options".to_string(),
                )
                .into());
            } else {
                let auto_extras = dialog_adapter.auto_emit_extra_headers.clone();
                let event_package = "presence";
                let body = session.local_sdp.clone();
                if auto_extras.is_empty() {
                    let _ = dialog_adapter
                        .send_notify(&session.session_id, event_package, body, None)
                        .await?;
                } else {
                    let opts = rvoip_sip_dialog::api::unified::NotifyRequestOptions {
                        event: event_package.to_string(),
                        subscription_state: String::new(),
                        content_type: None,
                        body: body.map(bytes::Bytes::from),
                        subscription_id: None,
                        extra_headers: auto_extras,
                    };
                    let _ = dialog_adapter
                        .send_notify_with_options(&session.session_id, opts)
                        .await?;
                }
            }
        }
        Action::SendINFOWithOptions => {
            let TrackedInDialogOptions::Info(options) = claim_tracked_request_staging(
                session,
                session_store,
                TrackedInDialogMethod::Info,
                stage_claim,
            )
            .await?
            else {
                return Err(crate::errors::SessionError::InvalidTransition(
                    "SendINFOWithOptions claimed the wrong method".to_string(),
                )
                .into());
            };
            let lease = dialog_adapter.outbound_request_tracker.prepare(
                &session.session_id,
                TrackedInDialogOptions::Info(Arc::clone(&options)),
            )?;
            let transaction_id = dialog_adapter
                .send_info_with_options(&session.session_id, (*options).clone())
                .await?;
            dialog_adapter
                .outbound_request_tracker
                .activate(lease, transaction_id)?;
        }
        Action::SendUPDATEWithOptions => {
            let TrackedInDialogOptions::Update(options) = claim_tracked_request_staging(
                session,
                session_store,
                TrackedInDialogMethod::Update,
                stage_claim,
            )
            .await?
            else {
                return Err(crate::errors::SessionError::InvalidTransition(
                    "SendUPDATEWithOptions claimed the wrong method".to_string(),
                )
                .into());
            };
            let lease = dialog_adapter.outbound_request_tracker.prepare(
                &session.session_id,
                TrackedInDialogOptions::Update(Arc::clone(&options)),
            )?;
            let transaction_id = dialog_adapter
                .send_update_with_options(&session.session_id, (*options).clone())
                .await?;
            dialog_adapter
                .outbound_request_tracker
                .activate(lease, transaction_id)?;
        }
        Action::SendReINVITEWithOptions => {
            if let Some(opts) = session.pending_reinvite_options.as_ref() {
                let snapshot = (**opts).clone();
                // RFC 3261 §14.1 — track the in-flight builder-API
                // re-INVITE so `HasPendingReinvite` fires the UAS-side
                // glare path if the peer's re-INVITE arrives before our
                // final response. Hold/Resume set this in their own
                // action handlers; the builder API needs the same
                // treatment. Cleared on terminal response by the
                // Active+Dialog{200OK,4xx,5xx,6xx,Timeout}+HasPendingReinvite
                // YAML rows.
                let sdp_snapshot = snapshot.sdp.clone().unwrap_or_default();
                session.pending_reinvite = Some(
                    crate::session_store::state::PendingReinvite::SdpUpdate(sdp_snapshot),
                );
                dialog_adapter
                    .send_reinvite_with_options(&session.session_id, snapshot)
                    .await?;
                session.pending_reinvite_options = None;
            }
        }
        Action::SendMESSAGEWithOptions => {
            if let Some(opts) = session.pending_message_options.as_ref() {
                let snapshot = (**opts).clone();
                dialog_adapter
                    .send_message_oob_with_options(snapshot)
                    .await?;
                session.pending_message_options = None;
            }
        }
        Action::SendOPTIONSWithOptions => {
            if let Some(opts) = session.pending_options_options.as_ref() {
                let snapshot = (**opts).clone();
                dialog_adapter
                    .send_options_oob_with_options(snapshot)
                    .await?;
                session.pending_options_options = None;
            }
        }
        Action::SendSUBSCRIBEWithOptions => {
            if let Some(opts) = session.pending_subscribe_options.as_ref() {
                // Out-of-dialog SUBSCRIBE uses the target as the
                // request URI; falls back to the session's remote
                // URI for in-dialog refresh.
                let target = session.remote_uri.clone().unwrap_or_default();
                let snapshot = (**opts).clone();
                dialog_adapter
                    .send_subscribe_oob_with_options(&target, snapshot)
                    .await?;
                session.pending_subscribe_options = None;
            }
        }
        Action::SendREGISTERWithOptions => {
            if let Some(opts) = session.pending_register_options.clone() {
                // SIP_API_DESIGN_2 §7.1 — REGISTER dispatch through the
                // unified options surface, routed through the
                // DialogAdapter mirror so HeaderPolicy::validate_outbound
                // and prepend_outbound_proxy_route run on the application
                // extras. The legacy automatic refresh path (driven by
                // Config.registration_auto_refresh) remains; this Action
                // covers builder dispatch (initial + manual refresh) and
                // consults `opts.refresh` for Call-ID / CSeq reuse
                // semantics in dialog-core.
                let opts = (*opts).clone();
                let refresh_flag = opts.refresh;
                let registrar_uri = opts.registrar_uri.clone();
                let from_uri = opts.aor_uri.clone();
                let contact_uri = opts.contact_uri.clone();
                let requested_expires = opts.expires;
                let session_id = session.session_id.clone();
                let (response, register_route) = dialog_adapter
                    .send_register_with_options_and_route(opts.clone())
                    .await
                    .map_err(|e| {
                        Box::<dyn std::error::Error + Send + Sync>::from(format!(
                            "SendREGISTERWithOptions: {}",
                            e
                        ))
                    })?;
                debug!(
                    "SendREGISTERWithOptions (refresh={}) on session {}: status={}",
                    refresh_flag,
                    session_id,
                    response.status_code()
                );

                match DialogAdapter::register_attempt_outcome_from_response(
                    &response,
                    &contact_uri,
                    requested_expires,
                ) {
                    RegisterAttemptOutcome::Registered {
                        accepted_expires,
                        mut metadata,
                    } => {
                        metadata.transport_route = Some(register_route);
                        dialog_adapter
                            .apply_registration_success(
                                &session_id,
                                &registrar_uri,
                                &from_uri,
                                &contact_uri,
                                accepted_expires,
                                metadata,
                            )
                            .await?;
                        sync_registration_state(session_store, &session_id, session)?;
                        session.pending_register_options = None;
                        return Ok(ActionOutcome::with_event(EventType::Registration200OK));
                    }
                    RegisterAttemptOutcome::Unregistered => {
                        dialog_adapter
                            .apply_unregistration_success(&session_id, &registrar_uri)
                            .await?;
                        sync_registration_state(session_store, &session_id, session)?;
                        session.pending_register_options = None;
                        return Ok(ActionOutcome::with_event(EventType::Unregistration200OK));
                    }
                    RegisterAttemptOutcome::AuthChallenge {
                        status_code,
                        challenge,
                    } => {
                        let retry_count = session_store
                            .with_session(&session_id, |latest| latest.registration_retry_count)?;
                        if retry_count >= 1 {
                            dialog_adapter
                                .apply_registration_failure(
                                    &session_id,
                                    &registrar_uri,
                                    status_code,
                                    "REGISTER authentication failed",
                                )
                                .await?;
                            sync_registration_state(session_store, &session_id, session)?;
                            session.pending_register_options = None;
                            return Ok(ActionOutcome::with_event(EventType::RegistrationFailed(
                                status_code,
                            )));
                        }

                        let retry_count = session_store
                            .update_session_with(&session_id, |latest| {
                                latest.registration_retry_count += 1;
                                latest.registration_retry_count
                            })
                            .await?;
                        session.registration_retry_count = retry_count;
                        return Ok(ActionOutcome::with_event(EventType::AuthRequired {
                            status_code,
                            challenge,
                            method: "REGISTER".to_string(),
                        }));
                    }
                    RegisterAttemptOutcome::IntervalTooBrief { min_expires } => {
                        let retry_count = session_store
                            .with_session(&session_id, |latest| latest.registration_retry_count)?;
                        if retry_count >= 2 {
                            dialog_adapter
                                .apply_registration_failure(
                                    &session_id,
                                    &registrar_uri,
                                    423,
                                    "Registration failed with repeated 423 Interval Too Brief responses",
                                )
                                .await?;
                            sync_registration_state(session_store, &session_id, session)?;
                            session.pending_register_options = None;
                            return Ok(ActionOutcome::with_event(EventType::RegistrationFailed(
                                423,
                            )));
                        }

                        let mut retry_opts = opts;
                        retry_opts.expires = min_expires;
                        let retry_opts = Arc::new(retry_opts);
                        let retry_count = session_store
                            .update_session_with(&session_id, |latest| {
                                latest.registration_expires = Some(min_expires);
                                latest.registration_retry_count += 1;
                                latest.pending_register_options = Some(Arc::clone(&retry_opts));
                                latest.registration_retry_count
                            })
                            .await?;
                        session.registration_expires = Some(min_expires);
                        session.registration_retry_count = retry_count;
                        session.pending_register_options = Some(retry_opts);
                        return Ok(ActionOutcome::with_event(EventType::SendOutboundRegister));
                    }
                    RegisterAttemptOutcome::Failure {
                        status_code,
                        reason,
                    } => {
                        if requested_expires == 0 {
                            dialog_adapter
                                .apply_unregistration_failure(
                                    &session_id,
                                    &registrar_uri,
                                    format!("{} (status {})", reason, status_code),
                                )
                                .await?;
                            sync_registration_state(session_store, &session_id, session)?;
                            session.pending_register_options = None;
                            return Ok(ActionOutcome::with_event(EventType::UnregistrationFailed));
                        }

                        dialog_adapter
                            .apply_registration_failure(
                                &session_id,
                                &registrar_uri,
                                status_code,
                                reason,
                            )
                            .await?;
                        sync_registration_state(session_store, &session_id, session)?;
                        session.pending_register_options = None;
                        return Ok(ActionOutcome::with_event(EventType::RegistrationFailed(
                            status_code,
                        )));
                    }
                }
            }
        }
        Action::SendINVITEWithOptions => {
            // INVITE uses `.clone()` (not `.take()`) so the snapshot
            // persists through the 401/407 auth-retry hop —
            // `Action::SendINVITEWithAuth` reads from the same stash to
            // preserve application extras on the retry. The slot is
            // cleared on final response by
            // `Action::ClearPendingINVITEOptions` emitted from the
            // Initiating → Active (Dialog200OK) and Initiating → Failed
            // (Dialog4xx/5xx/6xx/Timeout) transitions in YAML, and
            // backstopped by the executor's `Terminated` sweep.
            if let Some(opts) = session.pending_invite_options.clone() {
                let snapshot = (*opts).clone();
                // SDP precedence: builder-supplied `snapshot.sdp` wins;
                // otherwise fall back to `session.local_sdp` populated by the
                // preceding `GenerateLocalSDP` action.
                let sdp_for_wire =
                    authoritative_invite_sdp(Some(&snapshot), session.local_sdp.as_deref());

                // `with_topology_hiding(true)` is a no-op on the fresh-INVITE
                // build path (Via/Contact are stamped from scratch); the flag
                // is plumbed for proxy-style forward paths only.
                if snapshot.topology_hiding {
                    debug!(
                        "topology_hiding requested for session {} — fresh INVITE path stamps a clean Via/Contact by construction (no-op)",
                        session.session_id
                    );
                }

                // SIP_API_DESIGN_2 Phase B — map the staged snapshot to a
                // structured `InviteRequestOptions`. Per-call From display /
                // Contact / pre-computed Authorization travel as typed fields;
                // PAI / Route / Subject ride `extra_headers`. The very same
                // mapping feeds `SendINVITEWithAuth`, so the authenticated
                // retry carries identical overrides.
                let (invite_opts, suppress_global_proxy) = materialize_invite_options(
                    &snapshot,
                    session.pai_uri.as_deref(),
                    sdp_for_wire,
                )?;

                #[cfg(feature = "perf-call-setup-diagnostics")]
                let started = std::time::Instant::now();
                dialog_adapter
                    .send_invite_with_options(
                        &session.session_id,
                        invite_opts,
                        !suppress_global_proxy,
                    )
                    .await?;
                if let Some(real_dialog_id) =
                    dialog_adapter.session_to_dialog.get(&session.session_id)
                {
                    session.dialog_id = Some(real_dialog_id.value().clone().into());
                } else {
                    return Err(crate::errors::SessionError::InternalError(
                        "initial INVITE committed without an exact dialog mapping".to_string(),
                    )
                    .into());
                }
                #[cfg(feature = "perf-call-setup-diagnostics")]
                crate::call_setup_diag::record_stage(
                    &session.session_id,
                    "action.send_invite_with_options",
                    started.elapsed(),
                );
                debug!(
                    "SendINVITEWithOptions dispatched for session {}: {:?}",
                    session.session_id,
                    InviteEndpointDiagnostics::new(
                        snapshot.from.as_deref(),
                        Some(&snapshot.to),
                        snapshot.sdp.is_some()
                    )
                );
            }
        }

        // ──────────────────────────────────────────────────────────────
        // SIP_API_DESIGN_2 §7.3 invariant #2 — stash clear actions.
        // YAML emits the matching variant on the final-response
        // transition (200 / 4xx / 5xx / 6xx / timeout) so the slot is
        // ready for the next builder dispatch. Idempotent: clearing an
        // already-`None` slot is a no-op.
        // ──────────────────────────────────────────────────────────────
        Action::ClearPendingINVITEOptions => {
            session.pending_invite_options = None;
            // Keep the credentials negotiated by the successful initial
            // INVITE for method-specific requests in this exact dialog. BYE,
            // MESSAGE, and other listener-authenticated requests cannot reuse
            // the INVITE header verbatim, but they must retain its challenge
            // protection space so the adapter can recompute HA2 for the new
            // method/URI/body. Redirect and terminal cleanup remain the
            // authorities that zeroize these credentials.
            session.invite_auth_retry_count = 0;
        }
        Action::ClearPendingReINVITEOptions => {
            session.pending_reinvite_options = None;
        }
        Action::ClearPendingREGISTEROptions => {
            session.pending_register_options = None;
        }
        Action::ClearPendingSUBSCRIBEOptions => {
            session.pending_subscribe_options = None;
        }
        Action::ClearPendingMESSAGEOptions => {
            session.pending_message_options = None;
        }
        Action::ClearPendingNOTIFYOptions => {
            session.pending_notify_options = None;
        }
        Action::ClearPendingBYEOptions => {
            session.pending_bye_options = None;
        }
        Action::ClearPendingCANCELOptions => {
            session.pending_cancel_options = None;
        }
        Action::ClearPendingREFEROptions => {
            session.pending_refer_options = None;
        }
        Action::ClearPendingINFOOptions => {
            session.pending_info_options = None;
        }
        Action::ClearPendingUPDATEOptions => {
            session.pending_update_options = None;
        }
        Action::ClearPendingOPTIONSOptions => {
            session.pending_options_options = None;
        }
    }

    Ok(ActionOutcome::default())
}

/// SIP_API_DESIGN_2 R2 — resolve the SIP method for a non-INVITE/
/// non-REGISTER auth retry. Prefers the explicit
/// `session.pending_auth_method` (populated by the cross-crate
/// `AuthRequired` event's `method` field, originally extracted from
/// the response `CSeq:`). Falls back to inspecting which
/// `pending_<method>_options` stash is set — the conflict guard
/// guarantees at most one is populated per session.
fn resolve_auth_method(session: &crate::session_store::SessionState) -> String {
    if let Some(m) = session.pending_auth_method.as_ref() {
        if !m.is_empty() {
            return safe_outbound_auth_method_label(m).to_string();
        }
    }
    if session.pending_bye_options.is_some() {
        return "BYE".to_string();
    }
    if session.pending_refer_options.is_some() {
        return "REFER".to_string();
    }
    if session.pending_notify_options.is_some() {
        return "NOTIFY".to_string();
    }
    if session.pending_info_options.is_some() {
        return "INFO".to_string();
    }
    if session.pending_update_options.is_some() {
        return "UPDATE".to_string();
    }
    if session.pending_message_options.is_some() {
        return "MESSAGE".to_string();
    }
    if session.pending_options_options.is_some() {
        return "OPTIONS".to_string();
    }
    if session.pending_subscribe_options.is_some() {
        return "SUBSCRIBE".to_string();
    }
    // Default fallback — caller will treat the unknown method as an
    // error.
    String::new()
}

fn auth_method_for_error(method: &str) -> rvoip_sip_core::Method {
    match method {
        "BYE" => rvoip_sip_core::Method::Bye,
        "REFER" => rvoip_sip_core::Method::Refer,
        "NOTIFY" => rvoip_sip_core::Method::Notify,
        "INFO" => rvoip_sip_core::Method::Info,
        "UPDATE" => rvoip_sip_core::Method::Update,
        "MESSAGE" => rvoip_sip_core::Method::Message,
        "OPTIONS" => rvoip_sip_core::Method::Options,
        "SUBSCRIBE" => rvoip_sip_core::Method::Subscribe,
        _ => rvoip_sip_core::Method::Extension("extension".to_string()),
    }
}

fn safe_outbound_auth_method_label(method: &str) -> &'static str {
    match method.trim().to_ascii_uppercase().as_str() {
        "INVITE" => "INVITE",
        "REGISTER" => "REGISTER",
        "BYE" => "BYE",
        "REFER" => "REFER",
        "NOTIFY" => "NOTIFY",
        "INFO" => "INFO",
        "UPDATE" => "UPDATE",
        "MESSAGE" => "MESSAGE",
        "OPTIONS" => "OPTIONS",
        "SUBSCRIBE" => "SUBSCRIBE",
        _ => "extension",
    }
}

pub(crate) fn auth_retry_allowed(
    retry_count: u8,
    cap: u8,
    challenge: Option<&crate::auth::DigestChallenge>,
    challenge_stale: bool,
    replaces_nonce: Option<&str>,
) -> bool {
    if retry_count < cap {
        return true;
    }
    if retry_count != cap || !challenge_stale {
        return false;
    }
    let Some(challenge) = challenge else {
        return false;
    };
    replaces_nonce.is_some_and(|previous| previous != challenge.nonce)
}

const MAX_INVITE_PROTECTION_SPACES: usize = 8;

fn selected_invite_auth_realm(selected: &crate::auth::ClientAuthHeader) -> String {
    if let Some(challenge) = selected.digest_challenge.as_ref() {
        return challenge.realm.clone();
    }

    // Non-Digest ClientAuthHeader variants do not expose a parsed realm yet.
    // Keep schemes in distinct protection spaces and, critically, never use
    // the first token from the unselected aggregate challenge string.
    match &selected.scheme {
        crate::auth::SipAuthScheme::Digest => "digest".to_string(),
        crate::auth::SipAuthScheme::Bearer => "bearer".to_string(),
        crate::auth::SipAuthScheme::Basic => "basic".to_string(),
        crate::auth::SipAuthScheme::Aka => "aka".to_string(),
        crate::auth::SipAuthScheme::Other(_) => "other".to_string(),
    }
}

fn redacted_invite_auth_error<E>(source: E) -> crate::errors::SessionError {
    crate::errors::redacted_outbound_auth_error(
        crate::errors::OutboundAuthOperation::Invite,
        source,
    )
}

fn invite_credential_slot_for_challenge(
    credentials: &[crate::session_store::state::InviteAuthorizationCredential],
    kind: crate::session_store::state::InviteCredentialKind,
    protection_target: &str,
    realm: &str,
    nonce: Option<&str>,
    stale: bool,
) -> std::result::Result<Option<usize>, ()> {
    let existing = credentials.iter().position(|credential| {
        credential.kind == kind
            && credential.protection_target == protection_target
            && credential.realm == realm
    });
    match existing {
        Some(index) => {
            let credential = &credentials[index];
            if stale && credential.stale_refreshes == 0 && credential.nonce.as_deref() != nonce {
                Ok(Some(index))
            } else {
                Err(())
            }
        }
        None if credentials.len() >= MAX_INVITE_PROTECTION_SPACES => Err(()),
        None => Ok(None),
    }
}

/// SIP_API_DESIGN_2 R2 — pick the request-URI to fold into HA2 for the
/// digest computation. In-dialog methods (BYE, REFER, NOTIFY, INFO,
/// UPDATE) target `session.remote_uri`. OOB methods (MESSAGE,
/// OPTIONS) carry their target on the options struct; SUBSCRIBE
/// targets `session.remote_uri` (which the builder stashes there
/// before dispatch).
fn resolve_auth_request_uri(
    session: &crate::session_store::SessionState,
    method: &str,
) -> Option<String> {
    match method {
        "MESSAGE" => session
            .pending_message_options
            .as_ref()
            .map(|opts| opts.to_uri.clone()),
        "OPTIONS" => session
            .pending_options_options
            .as_ref()
            .map(|opts| opts.to_uri.clone()),
        _ => session.remote_uri.clone(),
    }
}

/// Publish an app-level `Event` to the global coordinator's session-to-app
/// channel, using the same fire-and-forget spawn pattern as
/// `session_event_handler::publish_api_event`. Errors are logged, not
/// propagated — a progress-NOTIFY transport failure should not roll back
/// the dialog transition that triggered it.
fn publish_transfer_event(dialog_adapter: &Arc<DialogAdapter>, api_event: Event) {
    let wrapped = crate::adapters::SessionApiCrossCrateEvent::new(api_event);
    let coordinator = dialog_adapter.global_coordinator.clone();
    tokio::spawn(async move {
        if coordinator.publish(wrapped).await.is_err() {
            tracing::warn!("Failed to publish Transfer* event (class=coordination)");
        }
    });
}

#[cfg(test)]
mod negotiated_audio_shape_tests {
    use super::negotiated_audio_shape;

    #[test]
    fn negotiated_shape_preserves_opus_and_g711_clocks() {
        assert_eq!(negotiated_audio_shape("PCMU"), (8_000, 1));
        assert_eq!(negotiated_audio_shape("PCMA"), (8_000, 1));
        assert_eq!(negotiated_audio_shape("opus"), (48_000, 2));
        assert_eq!(negotiated_audio_shape("OPUS"), (48_000, 2));
    }
}

#[cfg(test)]
mod registration_projection_tests {
    use super::*;
    use crate::state_table::Role;
    use crate::types::CallState;

    #[test]
    fn registration_projection_does_not_replace_unrelated_call_state() {
        let mut stored = SessionState::new(SessionId::new(), Role::UAC);
        stored.registration_call_id = Some("registration-call".into());
        stored.registration_cseq = 17;
        stored.registration_retry_count = 2;
        stored.is_registered = true;

        let mut event_local = SessionState::new(stored.session_id.clone(), Role::UAC);
        event_local.call_state = CallState::Active;
        event_local.local_sdp = Some("v=0\r\n".into());

        RegistrationStateProjection::capture(&stored).apply(&mut event_local);

        assert_eq!(event_local.call_state, CallState::Active);
        assert_eq!(event_local.local_sdp.as_deref(), Some("v=0\r\n"));
        assert_eq!(
            event_local.registration_call_id.as_deref(),
            Some("registration-call")
        );
        assert_eq!(event_local.registration_cseq, 17);
        assert_eq!(event_local.registration_retry_count, 2);
        assert!(event_local.is_registered);
    }
}

#[cfg(test)]
mod sip_response_task_tests {
    use super::*;
    use tokio::sync::oneshot;

    struct DropSignal(Option<oneshot::Sender<()>>);

    impl Drop for DropSignal {
        fn drop(&mut self) {
            if let Some(signal) = self.0.take() {
                let _ = signal.send(());
            }
        }
    }

    #[tokio::test]
    async fn dropping_response_join_aborts_the_owned_io_task() {
        let (started_tx, started_rx) = oneshot::channel();
        let (dropped_tx, dropped_rx) = oneshot::channel();
        let task = AbortSipResponseTaskOnDrop::new(tokio::spawn(async move {
            let _drop_signal = DropSignal(Some(dropped_tx));
            let _ = started_tx.send(());
            std::future::pending::<crate::errors::Result<()>>().await
        }));

        started_rx.await.expect("response task started");
        let join = Box::pin(task.join());
        drop(join);

        tokio::time::timeout(std::time::Duration::from_secs(1), dropped_rx)
            .await
            .expect("cancelled response task did not stop")
            .expect("response task drop signal closed");
    }

    #[tokio::test]
    async fn response_task_panics_map_to_a_fixed_internal_error_class() {
        let task = AbortSipResponseTaskOnDrop::new(tokio::spawn(async {
            panic!("synthetic response dispatch panic");
            #[allow(unreachable_code)]
            crate::errors::Result::<()>::Ok(())
        }));

        let error = join_sip_response_task(task)
            .await
            .expect_err("panicked response task must fail");
        match error {
            crate::errors::SessionError::InternalError(detail) => {
                assert_eq!(detail, SIP_RESPONSE_DISPATCH_JOIN_FAILURE);
            }
            other => panic!("unexpected response task error: {other:?}"),
        }
    }
}

#[cfg(test)]
mod invite_option_diagnostic_tests {
    use super::*;
    use crate::api::send::outbound_call::{OutboundCallOptionsSnapshot, ProxyOverride};
    use crate::auth::SipClientAuth;
    use crate::types::Credentials;
    use rvoip_sip_core::types::{headers::HeaderValue, HeaderName, TypedHeader};

    const SECRET: &str = "invite-option-secret-canary";

    fn secret_snapshot() -> OutboundCallOptionsSnapshot {
        OutboundCallOptionsSnapshot {
            from: Some(format!("sip:{SECRET}@from.invalid")),
            to: format!("sip:{SECRET}@target.invalid"),
            credentials: Some(Credentials::new(SECRET, SECRET)),
            auth: Some(SipClientAuth::bearer_token(SECRET)),
            contact_uri: Some(format!("sip:{SECRET}@contact.invalid")),
            subject: Some(SECRET.to_string()),
            from_display: Some(SECRET.to_string()),
            precomputed_auth: Some(format!("Bearer {SECRET}")),
            extra_headers: vec![TypedHeader::Other(
                HeaderName::Other("X-Application-Context".to_string()),
                HeaderValue::Raw(SECRET.as_bytes().to_vec()),
            )],
            ..OutboundCallOptionsSnapshot::default()
        }
    }

    fn assert_redacted(error: InviteOptionsMaterializationError) {
        let display = error.to_string();
        let debug = format!("{error:?}");
        for rendered in [&display, &debug] {
            assert!(!rendered.contains(SECRET), "secret leaked: {rendered}");
            assert!(!rendered.contains("sip:"), "URI leaked: {rendered}");
            assert!(
                !rendered.contains("Bearer"),
                "credential leaked: {rendered}"
            );
        }
        assert!(display.contains("present="));
        assert!(display.contains("class="));
        assert!(display.contains("bytes="));
    }

    #[test]
    fn invite_endpoint_log_metadata_never_formats_values() {
        let from = format!("sip:{SECRET}@from.invalid");
        let target = format!("sip:{SECRET}@target.invalid");
        let rendered = format!(
            "{:?}",
            InviteEndpointDiagnostics::new(Some(&from), Some(&target), true)
        );
        assert!(!rendered.contains(SECRET));
        assert!(!rendered.contains("sip:"));
        assert!(rendered.contains(&format!("from_bytes: {}", from.len())));
        assert!(rendered.contains(&format!("target_bytes: {}", target.len())));
        assert!(rendered.contains("sdp_present: true"));
    }

    #[test]
    fn invite_option_source_has_no_value_bearing_error_or_log_templates() {
        let source = include_str!("actions.rs");
        for forbidden in [
            ["Creating dialog from ", "{} to {}"].concat(),
            ["Sending INVITE from ", "{} to {}"].concat(),
            ["SendINVITEWithOptions dispatched for session {}: ", "to={}"].concat(),
            ["pai_uri (", "{}) is not a valid URI"].concat(),
            ["outbound_proxy override (", "{}) is not a valid URI"].concat(),
            ["SessionState.pai_uri (", "{}) is not a valid URI"].concat(),
        ] {
            assert!(
                !source.contains(&forbidden),
                "value-bearing diagnostic template returned: {forbidden}"
            );
        }
    }

    #[test]
    fn pai_materialization_error_exposes_only_safe_extent_and_class() {
        let error = materialize_invite_options(
            &secret_snapshot(),
            Some(&format!("sip:{SECRET}\r\nX-Injected: yes")),
            None,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            InviteOptionsMaterializationError::InvalidPAssertedIdentityUri { .. }
        ));
        assert_redacted(error);
    }

    #[test]
    fn proxy_materialization_error_exposes_only_safe_extent_and_class() {
        let mut proxy = secret_snapshot();
        proxy.outbound_proxy_override =
            ProxyOverride::Use(format!("sip:{SECRET}\r\nX-Injected: yes"));
        let error = materialize_invite_options(&proxy, None, None).unwrap_err();
        assert!(matches!(
            error,
            InviteOptionsMaterializationError::InvalidOutboundProxyUri { .. }
        ));
        assert_redacted(error);
    }

    #[test]
    fn successful_materialization_preserves_legacy_values_and_header_order() {
        let mut snapshot = secret_snapshot();
        // A missing From is intentionally left for the existing dialog path;
        // this diagnostic closure must not introduce new admission behavior.
        snapshot.from = None;
        snapshot.outbound_proxy_override =
            ProxyOverride::Use("sip:proxy.example.com;lr".to_string());

        let (options, suppress_global_proxy) = materialize_invite_options(
            &snapshot,
            Some("sip:identity@example.com"),
            Some("v=0\r\n".to_string()),
        )
        .expect("valid options");

        assert!(options.from_uri.is_empty());
        assert_eq!(options.to_uri, format!("sip:{SECRET}@target.invalid"));
        assert_eq!(options.sdp.as_deref(), Some("v=0\r\n"));
        assert!(suppress_global_proxy);
        assert_eq!(
            options.outbound_proxy_uri.as_ref().map(ToString::to_string),
            Some("sip:proxy.example.com;lr".to_string())
        );
        assert_eq!(options.extra_headers.len(), 3);
        assert_eq!(
            options.extra_headers[0].name(),
            HeaderName::PAssertedIdentity
        );
        assert_eq!(
            options.extra_headers[1].name(),
            HeaderName::Other("X-Application-Context".to_string())
        );
        assert_eq!(options.extra_headers[2].name(), HeaderName::Subject);
    }

    #[test]
    fn caller_sdp_is_the_authoritative_initial_and_auth_int_body() {
        let mut snapshot = OutboundCallOptionsSnapshot {
            sdp: Some("v=0\r\na=x-caller-byte-for-byte\r\n".into()),
            ..Default::default()
        };
        let generated = "v=0\r\na=x-generated-different\r\n";
        assert_eq!(
            authoritative_invite_sdp(Some(&snapshot), Some(generated)),
            snapshot.sdp.clone()
        );

        snapshot.sdp = None;
        assert_eq!(
            authoritative_invite_sdp(Some(&snapshot), Some(generated)).as_deref(),
            Some(generated)
        );
    }

    #[test]
    fn extension_auth_method_is_classified_before_errors_and_dispatch() {
        const METHOD_SECRET: &str = "X-AUTH-METHOD-PROVIDER-SECRET-CANARY";
        let mut session = crate::session_store::SessionState::new(
            crate::state_table::SessionId::new(),
            crate::state_table::Role::UAC,
        );
        session.pending_auth_method = Some(METHOD_SECRET.to_string());

        let method = resolve_auth_method(&session);
        assert_eq!(method, "extension");
        assert_eq!(safe_outbound_auth_method_label(METHOD_SECRET), "extension");

        let missing = crate::errors::SessionError::MissingCredentialsForRequestAuth {
            method: auth_method_for_error(&method),
        };
        let exhausted = crate::errors::SessionError::RequestAuthRetryExhausted {
            method: auth_method_for_error(&method),
        };
        let no_uri = format!(
            "SendRequestWithAuth: no request_uri for method {} on session",
            method
        );
        let unsupported = format!(
            "SendRequestWithAuth: unsupported method {} for session",
            method
        );
        for rendered in [
            missing.to_string(),
            exhausted.to_string(),
            no_uri,
            unsupported,
        ] {
            assert!(rendered.contains("extension"));
            assert!(!rendered.contains(METHOD_SECRET));
        }
    }

    #[test]
    fn invite_auth_slots_are_independent_bounded_and_allow_one_stale_refresh() {
        use crate::session_store::state::{InviteAuthorizationCredential, InviteCredentialKind};

        let proxy = InviteAuthorizationCredential {
            kind: InviteCredentialKind::Proxy,
            protection_target: "proxy.example".into(),
            challenge_raw: "Digest realm=\"edge\", nonce=\"nonce-one\"".into(),
            realm: "edge".into(),
            nonce: Some("nonce-one".into()),
            stale_refreshes: 0,
            value: "redacted".into(),
        };
        let credentials = vec![proxy];
        assert_eq!(
            invite_credential_slot_for_challenge(
                &credentials,
                InviteCredentialKind::Origin,
                "origin.example",
                "uas",
                Some("origin-nonce"),
                false,
            ),
            Ok(None),
            "a proxy credential must not consume the origin retry slot"
        );
        assert_eq!(
            invite_credential_slot_for_challenge(
                &credentials,
                InviteCredentialKind::Proxy,
                "proxy.example",
                "edge",
                Some("nonce-two"),
                true,
            ),
            Ok(Some(0))
        );
        assert!(invite_credential_slot_for_challenge(
            &credentials,
            InviteCredentialKind::Proxy,
            "proxy.example",
            "edge",
            Some("nonce-one"),
            true,
        )
        .is_err());

        let saturated = (0..MAX_INVITE_PROTECTION_SPACES)
            .map(|index| InviteAuthorizationCredential {
                kind: InviteCredentialKind::Origin,
                protection_target: format!("target-{index}"),
                challenge_raw: format!("Digest realm=\"realm-{index}\""),
                realm: format!("realm-{index}"),
                nonce: None,
                stale_refreshes: 0,
                value: "redacted".into(),
            })
            .collect::<Vec<_>>();
        assert!(invite_credential_slot_for_challenge(
            &saturated,
            InviteCredentialKind::Proxy,
            "new-target",
            "new-realm",
            None,
            false,
        )
        .is_err());
    }

    #[test]
    fn invite_auth_protection_space_uses_the_selected_digest_challenge() {
        let selected = crate::auth::SipClientAuth::digest("alice", "secret")
            .authorization_for_challenge(
                r#"Basic realm="legacy", Digest realm="weak-realm", nonce="weak", algorithm=MD5, Digest realm="strong-realm", nonce="strong", algorithm=SHA-512-256, qop="auth""#,
                "INVITE",
                "sip:bob@example.test",
                1,
                Some(b"v=0\r\n"),
                false,
            )
            .expect("select strongest Digest challenge");

        assert_eq!(selected_invite_auth_realm(&selected), "strong-realm");
        assert_eq!(
            selected
                .digest_challenge
                .as_ref()
                .map(|challenge| challenge.nonce.as_str()),
            Some("strong")
        );
    }
}
