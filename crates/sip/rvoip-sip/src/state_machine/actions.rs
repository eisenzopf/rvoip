use crate::adapters::dialog_adapter::{
    RegisterAttemptContext, RegisterAttemptOutcome, RegistrationPostCommitEffect,
};
use crate::adapters::outbound_request_tracker::{TrackedInDialogMethod, TrackedInDialogOptions};
use crate::session_registry::SessionRegistryHandle;
use crate::state_machine::executor::{
    InboundResponseStateInput, PendingOptionsSlot, PendingOptionsSlotKind, StageDispatchClaim,
};
use crate::state_table::types::{EventType, SessionId};
use rvoip_sip_core::types::{HeaderName, TypedHeader};
use rvoip_sip_dialog::api::unified::{ReInviteRequestOptions, ReferRequestOptions};
use std::fmt;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

use crate::{
    adapters::{dialog_adapter::DialogAdapter, media_adapter::MediaAdapter},
    api::events::Event,
    cleanup_diag::{self, CleanupStage},
    session_store::SessionState,
    state_table::{Action, Condition},
};

const SIP_RESPONSE_DISPATCH_JOIN_FAILURE: &str = "SIP response dispatch task failed (class=join)";
const DIALOG_CLEANUP_JOIN_FAILURE: &str = "SIP dialog cleanup task failed (class=join)";

fn exact_dialog_cleanup_handle(
    session: &SessionState,
) -> crate::errors::Result<SessionRegistryHandle> {
    session.lifecycle_handle.clone().ok_or_else(|| {
        crate::errors::SessionError::InvalidTransition(
            "SIP dialog cleanup requires exact session authority".to_string(),
        )
    })
}

fn retire_lane_owned_dialog_identity(session: &mut SessionState) {
    session.dialog_id = None;
    session.dialog_established = false;
}

fn exact_request_tracker_handle(
    session: &SessionState,
) -> crate::errors::Result<&SessionRegistryHandle> {
    session.lifecycle_handle.as_ref().ok_or_else(|| {
        crate::errors::SessionError::InvalidTransition(
            "outbound request tracking requires exact session authority".to_string(),
        )
    })
}

fn next_session_refresh_generation(session: &mut SessionState) -> u64 {
    let next = session.session_refresh_timer_generation.wrapping_add(1);
    session.session_refresh_timer_generation = if next == 0 { 1 } else { next };
    session.session_refresh_timer_generation
}

fn session_refresh_retry_effect(
    session: &mut SessionState,
    kind: SessionRefreshDeadlineKind,
) -> ActionOutcome {
    let generation = next_session_refresh_generation(session);
    ActionOutcome::with_deferred_effect(DeferredActionEffect::SessionRefreshTimer(
        SessionRefreshTimerEffect {
            generation,
            delay: std::time::Duration::from_secs(1),
            kind,
        },
    ))
}

fn session_refresh_immediate_effect(
    session: &SessionState,
    kind: SessionRefreshDeadlineKind,
) -> ActionOutcome {
    ActionOutcome::with_deferred_effect(DeferredActionEffect::SessionRefreshTimer(
        SessionRefreshTimerEffect {
            generation: session.session_refresh_timer_generation,
            delay: std::time::Duration::ZERO,
            kind,
        },
    ))
}

fn session_refresh_transaction_deadline_effect(
    session: &SessionState,
    delay: std::time::Duration,
    kind: SessionRefreshDeadlineKind,
) -> ActionOutcome {
    ActionOutcome::with_deferred_effect(DeferredActionEffect::SessionRefreshTimer(
        SessionRefreshTimerEffect {
            generation: session.session_refresh_timer_generation,
            delay,
            kind,
        },
    ))
}

fn session_refresh_headers(session: &SessionState) -> crate::errors::Result<Vec<TypedHeader>> {
    use rvoip_sip_core::types::session_expires::{Refresher, SessionExpires};
    use rvoip_sip_core::types::{min_se::MinSE, supported::Supported};

    let interval = session.session_refresh_interval_secs.ok_or_else(|| {
        crate::errors::SessionError::InvalidTransition(
            "RFC 4028 refresh has no negotiated Session-Expires interval".to_string(),
        )
    })?;
    let refresher = match session.role {
        crate::state_table::Role::UAC => Refresher::Uac,
        crate::state_table::Role::UAS => Refresher::Uas,
        crate::state_table::Role::Both => {
            return Err(crate::errors::SessionError::InvalidTransition(
                "RFC 4028 refresh requires a concrete dialog role".to_string(),
            ));
        }
    };
    Ok(vec![
        TypedHeader::SessionExpires(SessionExpires::new(interval, Some(refresher))),
        TypedHeader::MinSE(MinSE::new(interval.min(90).max(1))),
        TypedHeader::Supported(Supported::new(vec!["timer".to_string()])),
    ])
}

fn prepare_session_refresh_update(
    session: &mut SessionState,
    dialog_adapter: &DialogAdapter,
) -> crate::errors::Result<ActionOutcome> {
    use crate::session_store::state::SessionRefreshPhase;

    let handle = exact_request_tracker_handle(session)?;
    if session.pending_reinvite.is_some()
        || session.pending_reinvite_options.is_some()
        || session.pending_update_options.is_some()
        || dialog_adapter
            .outbound_request_tracker
            .has_request(handle, TrackedInDialogMethod::Update)
    {
        return Ok(session_refresh_retry_effect(
            session,
            SessionRefreshDeadlineKind::UpdateDue,
        ));
    }
    session.pending_update_options = Some(Arc::new(
        rvoip_sip_dialog::api::unified::UpdateRequestOptions {
            sdp: None,
            session_timer_refresh: true,
            extra_headers: session_refresh_headers(session)?,
        },
    ));
    session.session_refresh_phase = SessionRefreshPhase::UpdateInFlight;
    Ok(ActionOutcome::with_event(EventType::SendOutboundUpdate))
}

fn prepare_session_refresh_reinvite(
    session: &mut SessionState,
) -> crate::errors::Result<ActionOutcome> {
    use crate::session_store::state::SessionRefreshPhase;

    if session.pending_reinvite.is_some() || session.pending_reinvite_options.is_some() {
        session.session_refresh_phase = SessionRefreshPhase::Idle;
        return Ok(session_refresh_retry_effect(
            session,
            SessionRefreshDeadlineKind::ReinviteDue,
        ));
    }
    session.pending_reinvite_options = Some(Arc::new(ReInviteRequestOptions {
        sdp: None,
        session_timer_refresh: true,
        precomputed_authorization: None,
        extra_headers: session_refresh_headers(session)?,
    }));
    session.session_refresh_phase = SessionRefreshPhase::ReinviteInFlight;
    Ok(ActionOutcome::with_event(EventType::SendOutboundReInvite))
}

/// Retire every lifecycle attachment derived from one media allocation in the
/// lane-owned working state. The executor publishes this mutation exactly once
/// after all ordered actions have finished.
fn retire_lane_owned_media_identity(session: &mut SessionState) {
    session.media_session_id = None;
    session.media_session_ready = false;
    session.sdp_negotiated = false;
    session.local_sdp = None;
    session.negotiated_config = None;
}

/// Run lower cleanup without publishing `SessionStore`, then retire the
/// event-local identity even if lower cleanup reports an error. In the error
/// case the exact lifecycle resource remains responsible for quiesced
/// teardown; recommitting a stale media identity would only recreate the
/// duplicate-writer race this lane is intended to prevent.
async fn cleanup_lane_owned_media(
    session: &mut SessionState,
    media_adapter: &Arc<MediaAdapter>,
) -> crate::errors::Result<()> {
    let cleanup = media_adapter.cleanup_session_lane_owned(session).await;
    retire_lane_owned_media_identity(session);
    cleanup
}

/// Release every lower-layer resource represented by the lane-owned working
/// state. All state-table cleanup spellings delegate here so dialog/media
/// teardown has one ordering rule and one identity-retirement rule.
async fn release_lane_owned_resources(
    session: &mut SessionState,
    dialog_adapter: &Arc<DialogAdapter>,
    media_adapter: &Arc<MediaAdapter>,
) -> crate::errors::Result<()> {
    let handle = exact_dialog_cleanup_handle(session)?;
    dialog_adapter
        .cleanup_session_exact_lane_owned(&handle)
        .await?;
    retire_lane_owned_dialog_identity(session);
    cleanup_lane_owned_media(session, media_adapter).await
}

#[cfg(test)]
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

#[cfg(test)]
async fn join_sip_response_task(
    task: AbortSipResponseTaskOnDrop<crate::errors::Result<()>>,
) -> crate::errors::Result<()> {
    task.join().await.map_err(|_| {
        crate::errors::SessionError::InternalError(SIP_RESPONSE_DISPATCH_JOIN_FAILURE.to_string())
    })?
}

/// Action-layer wrapper that retains dialog/transaction-core's authoritative
/// final-response wire classification across type erasure in the executor.
#[derive(Debug)]
pub(crate) struct ExactSipResponseActionError {
    disposition: rvoip_sip_dialog::FinalResponseCompletionDisposition,
    source: Box<dyn std::error::Error + Send + Sync>,
}

impl ExactSipResponseActionError {
    fn new(
        disposition: rvoip_sip_dialog::FinalResponseCompletionDisposition,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            disposition,
            source: Box::new(source),
        }
    }
}

impl fmt::Display for ExactSipResponseActionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "exact SIP response failed ({:?}): {}",
            self.disposition, self.source
        )
    }
}

impl std::error::Error for ExactSipResponseActionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

pub(crate) fn exact_sip_response_failure_disposition(
    error: &(dyn std::error::Error + Send + Sync + 'static),
) -> Option<rvoip_sip_dialog::FinalResponseCompletionDisposition> {
    error
        .downcast_ref::<ExactSipResponseActionError>()
        .map(|error| error.disposition)
}

type ExactSipResponseResult = Result<
    rvoip_sip_dialog::FinalResponseCompletionDisposition,
    Box<dyn std::error::Error + Send + Sync>,
>;

async fn send_exact_sip_response_on_fresh_task(
    dialog_adapter: Arc<DialogAdapter>,
    session_id: SessionId,
    transaction_id: rvoip_sip_dialog::transaction::TransactionKey,
    code: u16,
    sdp: Option<String>,
    extra_headers: Option<Vec<rvoip_sip_core::types::TypedHeader>>,
) -> ExactSipResponseResult {
    let task = AbortSipResponseTaskOnDrop::new(tokio::spawn(async move {
        if let Some(scripted) =
            scripted_exact_sip_response(&session_id, &transaction_id, code).await
        {
            return scripted;
        }
        let result = match extra_headers {
            Some(extra_headers) => {
                dialog_adapter
                    .send_response_with_options_for_transaction_classified(
                        &session_id,
                        &transaction_id,
                        code,
                        sdp,
                        extra_headers,
                    )
                    .await
            }
            None => {
                dialog_adapter
                    .send_response_for_transaction_classified(
                        &session_id,
                        &transaction_id,
                        code,
                        sdp,
                    )
                    .await
            }
        };
        result.map_err(|error| {
            Box::new(ExactSipResponseActionError::new(error.disposition, error))
                as Box<dyn std::error::Error + Send + Sync>
        })
    }));

    task.join().await.map_err(|_| {
        Box::new(ExactSipResponseActionError::new(
            rvoip_sip_dialog::FinalResponseCompletionDisposition::WireUnknownErrorTerminal,
            crate::errors::SessionError::InternalError(
                SIP_RESPONSE_DISPATCH_JOIN_FAILURE.to_string(),
            ),
        )) as Box<dyn std::error::Error + Send + Sync>
    })?
}

async fn send_exact_refer_response_on_fresh_task(
    dialog_adapter: Arc<DialogAdapter>,
    session_id: SessionId,
    transaction_id: rvoip_sip_dialog::transaction::TransactionKey,
    transaction_id_text: String,
    code: u16,
) -> ExactSipResponseResult {
    let task = AbortSipResponseTaskOnDrop::new(tokio::spawn(async move {
        if let Some(scripted) =
            scripted_exact_sip_response(&session_id, &transaction_id, code).await
        {
            return scripted;
        }
        dialog_adapter
            .send_refer_response_classified(&transaction_id_text, code)
            .await
            .map_err(|error| {
                Box::new(ExactSipResponseActionError::new(error.disposition, error))
                    as Box<dyn std::error::Error + Send + Sync>
            })
    }));

    task.join().await.map_err(|_| {
        Box::new(ExactSipResponseActionError::new(
            rvoip_sip_dialog::FinalResponseCompletionDisposition::WireUnknownErrorTerminal,
            crate::errors::SessionError::InternalError(
                SIP_RESPONSE_DISPATCH_JOIN_FAILURE.to_string(),
            ),
        )) as Box<dyn std::error::Error + Send + Sync>
    })?
}

async fn send_exact_provisional_sip_response_on_fresh_task(
    dialog_adapter: Arc<DialogAdapter>,
    session_id: SessionId,
    transaction_id: rvoip_sip_dialog::transaction::TransactionKey,
    code: u16,
    sdp: Option<String>,
    extra_headers: Option<Vec<rvoip_sip_core::types::TypedHeader>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let task = AbortSipResponseTaskOnDrop::new(tokio::spawn(async move {
        if let Some(scripted) =
            scripted_exact_sip_response(&session_id, &transaction_id, code).await
        {
            return match scripted {
                Ok(
                    rvoip_sip_dialog::FinalResponseCompletionDisposition::WrittenSuccessTerminal,
                ) => Ok(()),
                Ok(rvoip_sip_dialog::FinalResponseCompletionDisposition::ZeroWireRetryable) => {
                    Err(zero_wire_exact_response_error(
                        "exact provisional response stopped before transport write",
                    ))
                }
                Ok(
                    rvoip_sip_dialog::FinalResponseCompletionDisposition::WireUnknownErrorTerminal,
                ) => Err(Box::new(ExactSipResponseActionError::new(
                    rvoip_sip_dialog::FinalResponseCompletionDisposition::WireUnknownErrorTerminal,
                    crate::errors::SessionError::DialogError(
                        "exact provisional response crossed an unknown transport boundary"
                            .to_string(),
                    ),
                ))
                    as Box<dyn std::error::Error + Send + Sync>),
                Err(error) => Err(error),
            };
        }

        let result = match extra_headers {
            Some(extra_headers) => {
                dialog_adapter
                    .send_response_with_options_for_transaction(
                        &session_id,
                        &transaction_id,
                        code,
                        sdp,
                        extra_headers,
                    )
                    .await
            }
            None => {
                dialog_adapter
                    .send_response_for_transaction(&session_id, &transaction_id, code, sdp)
                    .await
            }
        };
        result.map_err(|error| {
            Box::new(ExactSipResponseActionError::new(
                // The unclassified provisional surface cannot prove that a
                // transport error preceded its first write. Preserve the
                // authority for a later final response, but never auto-retry
                // this provisional response as though it were proven zero-wire.
                rvoip_sip_dialog::FinalResponseCompletionDisposition::WireUnknownErrorTerminal,
                error,
            )) as Box<dyn std::error::Error + Send + Sync>
        })
    }));

    task.join().await.map_err(|_| {
        Box::new(ExactSipResponseActionError::new(
            rvoip_sip_dialog::FinalResponseCompletionDisposition::WireUnknownErrorTerminal,
            crate::errors::SessionError::InternalError(
                SIP_RESPONSE_DISPATCH_JOIN_FAILURE.to_string(),
            ),
        )) as Box<dyn std::error::Error + Send + Sync>
    })?
}

struct ExactInitialInviteTerminal {
    transaction_id: rvoip_sip_dialog::transaction::TransactionKey,
    written_success: bool,
    terminal_error: Option<Box<dyn std::error::Error + Send + Sync>>,
}

fn zero_wire_exact_response_error(
    detail: impl Into<String>,
) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(ExactSipResponseActionError::new(
        rvoip_sip_dialog::FinalResponseCompletionDisposition::ZeroWireRetryable,
        crate::errors::SessionError::InvalidTransition(detail.into()),
    ))
}

async fn send_exact_initial_invite_final_response(
    session: &SessionState,
    dialog_adapter: &Arc<DialogAdapter>,
    status: u16,
    body: Option<String>,
    extra_headers: Option<Vec<rvoip_sip_core::types::TypedHeader>>,
) -> Result<ExactInitialInviteTerminal, Box<dyn std::error::Error + Send + Sync>> {
    let transaction_id = session
        .pending_inbound_invite_transaction_id
        .clone()
        .ok_or_else(|| {
            zero_wire_exact_response_error(
                "initial INVITE final response has no exact inbound transaction",
            )
        })?;
    let dispatch = send_exact_sip_response_on_fresh_task(
        Arc::clone(dialog_adapter),
        session.session_id.clone(),
        transaction_id.clone(),
        status,
        body,
        extra_headers,
    )
    .await;
    let (written_success, terminal_error) = match dispatch {
        Ok(rvoip_sip_dialog::FinalResponseCompletionDisposition::WrittenSuccessTerminal) => {
            (true, None)
        }
        Ok(rvoip_sip_dialog::FinalResponseCompletionDisposition::ZeroWireRetryable) => {
            return Err(zero_wire_exact_response_error(
                "exact initial INVITE response stopped before transport write",
            ));
        }
        Ok(rvoip_sip_dialog::FinalResponseCompletionDisposition::WireUnknownErrorTerminal) => (
            false,
            Some(Box::new(ExactSipResponseActionError::new(
                rvoip_sip_dialog::FinalResponseCompletionDisposition::WireUnknownErrorTerminal,
                crate::errors::SessionError::DialogError(
                    "exact initial INVITE response crossed an unknown transport boundary"
                        .to_string(),
                ),
            )) as Box<dyn std::error::Error + Send + Sync>),
        ),
        Err(error) => match exact_sip_response_failure_disposition(error.as_ref()) {
            Some(rvoip_sip_dialog::FinalResponseCompletionDisposition::ZeroWireRetryable) => {
                return Err(error);
            }
            Some(rvoip_sip_dialog::FinalResponseCompletionDisposition::WrittenSuccessTerminal) => {
                (true, None)
            }
            Some(
                rvoip_sip_dialog::FinalResponseCompletionDisposition::WireUnknownErrorTerminal,
            ) => (false, Some(error)),
            None => return Err(error),
        },
    };
    Ok(ExactInitialInviteTerminal {
        transaction_id,
        written_success,
        terminal_error,
    })
}

fn validate_initial_invite_event_response_authority(
    session: &SessionState,
    inbound_response: Option<&InboundResponseStateInput>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let event_transaction = inbound_response
        .ok_or_else(|| {
            zero_wire_exact_response_error(
                "initial INVITE event has no event-local response authority",
            )
        })?
        .transaction_id()
        .map_err(|error| zero_wire_exact_response_error(error.to_string()))?;
    let retained_transaction = session
        .pending_inbound_invite_transaction_id
        .as_ref()
        .ok_or_else(|| {
            zero_wire_exact_response_error(
                "initial INVITE event has no retained response authority",
            )
        })?;
    if event_transaction != retained_transaction {
        return Err(zero_wire_exact_response_error(
            "initial INVITE event-local and retained response authorities differ",
        ));
    }
    Ok(())
}

async fn send_exact_initial_invite_provisional_response(
    session: &SessionState,
    dialog_adapter: &Arc<DialogAdapter>,
    status: u16,
    body: Option<String>,
    extra_headers: Option<Vec<rvoip_sip_core::types::TypedHeader>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if !(100..=199).contains(&status) {
        return Err(zero_wire_exact_response_error(
            "initial INVITE provisional response is not 1xx",
        ));
    }
    let transaction_id = session
        .pending_inbound_invite_transaction_id
        .clone()
        .ok_or_else(|| {
            zero_wire_exact_response_error(
                "initial INVITE provisional response has no exact inbound transaction",
            )
        })?;
    send_exact_provisional_sip_response_on_fresh_task(
        Arc::clone(dialog_adapter),
        session.session_id.clone(),
        transaction_id,
        status,
        body,
        extra_headers,
    )
    .await
}

pub(crate) struct ExactInboundResponseTerminal {
    pub(crate) terminal_error: Option<Box<dyn std::error::Error + Send + Sync>>,
}

pub(crate) struct ExactReferResponseTerminal {
    pub(crate) terminal_error: Option<Box<dyn std::error::Error + Send + Sync>>,
}

/// Author one REFER final through its immutable inbound transaction. Only a
/// proven zero-wire result is retryable. Written and wire-unknown results are
/// terminal for the response authority, so callers must clear the matching
/// lane-owned REFER state before surfacing `terminal_error`.
pub(crate) async fn send_exact_refer_final_response(
    session_id: &SessionId,
    transaction_id_text: &str,
    dialog_adapter: &Arc<DialogAdapter>,
    status: u16,
) -> Result<ExactReferResponseTerminal, Box<dyn std::error::Error + Send + Sync>> {
    if !(200..=699).contains(&status) {
        return Err(zero_wire_exact_response_error(
            "REFER response status is not final",
        ));
    }
    let transaction_id = transaction_id_text
        .parse::<rvoip_sip_dialog::transaction::TransactionKey>()
        .map_err(|_| zero_wire_exact_response_error("REFER transaction authority is invalid"))?;
    if !transaction_id.is_server() || transaction_id.method() != &rvoip_sip_core::Method::Refer {
        return Err(zero_wire_exact_response_error(
            "REFER response authority is not an inbound REFER transaction",
        ));
    }
    let dispatch = send_exact_refer_response_on_fresh_task(
        Arc::clone(dialog_adapter),
        session_id.clone(),
        transaction_id,
        transaction_id_text.to_string(),
        status,
    )
    .await;
    let terminal_error = match dispatch {
        Ok(rvoip_sip_dialog::FinalResponseCompletionDisposition::WrittenSuccessTerminal) => None,
        Ok(rvoip_sip_dialog::FinalResponseCompletionDisposition::ZeroWireRetryable) => {
            return Err(zero_wire_exact_response_error(
                "exact REFER response stopped before transport write",
            ));
        }
        Ok(rvoip_sip_dialog::FinalResponseCompletionDisposition::WireUnknownErrorTerminal) => {
            Some(Box::new(ExactSipResponseActionError::new(
                rvoip_sip_dialog::FinalResponseCompletionDisposition::WireUnknownErrorTerminal,
                crate::errors::SessionError::DialogError(
                    "exact REFER response crossed an unknown transport boundary".to_string(),
                ),
            )) as Box<dyn std::error::Error + Send + Sync>)
        }
        Err(error) => match exact_sip_response_failure_disposition(error.as_ref()) {
            Some(rvoip_sip_dialog::FinalResponseCompletionDisposition::ZeroWireRetryable) => {
                return Err(error);
            }
            Some(rvoip_sip_dialog::FinalResponseCompletionDisposition::WrittenSuccessTerminal) => {
                None
            }
            Some(
                rvoip_sip_dialog::FinalResponseCompletionDisposition::WireUnknownErrorTerminal,
            ) => Some(error),
            None => return Err(error),
        },
    };
    Ok(ExactReferResponseTerminal { terminal_error })
}

/// Author one final response using only the transaction captured from this
/// inbound event. Written and wire-unknown outcomes consume the transient
/// authority; a zero-wire outcome deliberately leaves it retryable.
pub(crate) async fn send_exact_inbound_final_response(
    session_id: &SessionId,
    inbound_response: Option<&mut InboundResponseStateInput>,
    dialog_adapter: &Arc<DialogAdapter>,
    status: u16,
    body: Option<String>,
    extra_headers: Option<Vec<rvoip_sip_core::types::TypedHeader>>,
) -> Result<ExactInboundResponseTerminal, Box<dyn std::error::Error + Send + Sync>> {
    if !(200..=699).contains(&status) {
        return Err(zero_wire_exact_response_error(
            "inbound re-INVITE/UPDATE response is not final",
        ));
    }
    let inbound_response = inbound_response.ok_or_else(|| {
        zero_wire_exact_response_error(
            "inbound re-INVITE/UPDATE final response has no event-local transaction authority",
        )
    })?;
    let transaction_id = inbound_response
        .transaction_id()
        .map_err(|error| zero_wire_exact_response_error(error.to_string()))?
        .clone();
    let dispatch = send_exact_sip_response_on_fresh_task(
        Arc::clone(dialog_adapter),
        session_id.clone(),
        transaction_id,
        status,
        body,
        extra_headers,
    )
    .await;
    let terminal_error = match dispatch {
        Ok(rvoip_sip_dialog::FinalResponseCompletionDisposition::WrittenSuccessTerminal) => None,
        Ok(rvoip_sip_dialog::FinalResponseCompletionDisposition::ZeroWireRetryable) => {
            return Err(zero_wire_exact_response_error(
                "exact inbound re-INVITE/UPDATE response stopped before transport write",
            ));
        }
        Ok(rvoip_sip_dialog::FinalResponseCompletionDisposition::WireUnknownErrorTerminal) => {
            Some(Box::new(ExactSipResponseActionError::new(
                rvoip_sip_dialog::FinalResponseCompletionDisposition::WireUnknownErrorTerminal,
                crate::errors::SessionError::DialogError(
                    "exact inbound re-INVITE/UPDATE response crossed an unknown transport boundary"
                        .to_string(),
                ),
            )) as Box<dyn std::error::Error + Send + Sync>)
        }
        Err(error) => match exact_sip_response_failure_disposition(error.as_ref()) {
            Some(rvoip_sip_dialog::FinalResponseCompletionDisposition::ZeroWireRetryable) => {
                return Err(error);
            }
            Some(rvoip_sip_dialog::FinalResponseCompletionDisposition::WrittenSuccessTerminal) => {
                None
            }
            Some(
                rvoip_sip_dialog::FinalResponseCompletionDisposition::WireUnknownErrorTerminal,
            ) => Some(error),
            None => return Err(error),
        },
    };
    inbound_response.consume_terminal();
    Ok(ExactInboundResponseTerminal { terminal_error })
}

async fn send_exact_inbound_provisional_response(
    session_id: &SessionId,
    inbound_response: Option<&mut InboundResponseStateInput>,
    dialog_adapter: &Arc<DialogAdapter>,
    status: u16,
    body: Option<String>,
    extra_headers: Option<Vec<rvoip_sip_core::types::TypedHeader>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if !(100..=199).contains(&status) {
        return Err(zero_wire_exact_response_error(
            "inbound response is not provisional",
        ));
    }
    let inbound_response = inbound_response.ok_or_else(|| {
        zero_wire_exact_response_error(
            "inbound provisional response has no event-local transaction authority",
        )
    })?;
    let transaction_id = inbound_response
        .transaction_id()
        .map_err(|error| zero_wire_exact_response_error(error.to_string()))?
        .clone();
    send_exact_provisional_sip_response_on_fresh_task(
        Arc::clone(dialog_adapter),
        session_id.clone(),
        transaction_id,
        status,
        body,
        extra_headers,
    )
    .await
}

fn consume_exact_initial_invite_response_authority(
    session: &mut SessionState,
    dialog_adapter: &DialogAdapter,
    terminal: &ExactInitialInviteTerminal,
    record_invite_200_metrics: bool,
) {
    // The transaction/timing pair is one immutable response authority. It is
    // retired together only after the transaction classifies the response as
    // written or wire-unknown terminal.
    let response_started_at = session.incoming_invite_received_at.take();
    session.pending_inbound_invite_transaction_id.take();
    let udp_receive_timing = dialog_adapter
        .dialog_api
        .dialog_manager()
        .core()
        .transaction_manager()
        .take_inbound_timing(&terminal.transaction_id);
    if terminal.written_success && record_invite_200_metrics {
        if let Some(timing) = udp_receive_timing {
            if let Some(received_at) = timing.received_at {
                rvoip_sip_dialog::diagnostics::record_udp_receive_to_invite_200(
                    received_at.elapsed(),
                );
            }
        }
        if let Some(started_at) = response_started_at {
            rvoip_sip_dialog::diagnostics::record_200_ok_invite_first();
            rvoip_sip_dialog::diagnostics::record_first_invite_to_200(started_at.elapsed());
        }
    }
}

fn exact_redirect_response_headers(
    contacts: &[String],
    extra_headers: Option<Vec<rvoip_sip_core::types::TypedHeader>>,
) -> Result<Vec<rvoip_sip_core::types::TypedHeader>, Box<dyn std::error::Error + Send + Sync>> {
    use rvoip_sip_core::types::{
        address::Address,
        contact::{Contact, ContactParamInfo},
        uri::Uri,
        TypedHeader,
    };

    if contacts.is_empty() {
        return Err(zero_wire_exact_response_error(
            "initial INVITE redirect has no Contact URI",
        ));
    }
    let mut params = Vec::with_capacity(contacts.len());
    for contact in contacts {
        let uri = contact.parse::<Uri>().map_err(|_| {
            zero_wire_exact_response_error("initial INVITE redirect has an invalid Contact URI")
        })?;
        params.push(ContactParamInfo {
            address: Address::new(uri),
        });
    }
    let mut headers = vec![TypedHeader::Contact(Contact::new_params(params))];
    headers.extend(extra_headers.unwrap_or_default());
    Ok(headers)
}

async fn cleanup_dialog_on_fresh_task(
    dialog_adapter: Arc<DialogAdapter>,
    handle: SessionRegistryHandle,
) -> crate::errors::Result<()> {
    let task = AbortSipResponseTaskOnDrop::new(tokio::spawn(async move {
        dialog_adapter
            .cleanup_session_exact_lane_owned(&handle)
            .await
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
    pub(crate) deferred_effects: Vec<DeferredActionEffect>,
}

impl ActionOutcome {
    fn with_event(event: EventType) -> Self {
        Self {
            follow_up_events: vec![event],
            deferred_effects: Vec::new(),
        }
    }

    fn with_deferred_effect(effect: DeferredActionEffect) -> Self {
        Self {
            follow_up_events: Vec::new(),
            deferred_effects: vec![effect],
        }
    }

    fn with_event_and_deferred_effect(event: EventType, effect: DeferredActionEffect) -> Self {
        Self {
            follow_up_events: vec![event],
            deferred_effects: vec![effect],
        }
    }
}

/// Work that is admitted only after the transition's exact state publication.
///
/// Keeping this descriptor private to the state-machine implementation lets an
/// action declare delayed work without sleeping in the session lane or
/// manufacturing a second signaling dispatcher.
#[derive(Debug, Clone)]
pub(crate) enum DeferredActionEffect {
    ReinviteRetry(ReinviteRetryEffect),
    Registration(RegistrationPostCommitEffect),
    TransferNotify(TransferNotifyEffect),
    SessionRefreshTimer(SessionRefreshTimerEffect),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum SessionRefreshDeadlineKind {
    UpdateDue,
    ReinviteDue,
    PeerExpired,
    UpdateFailed,
    ReinviteFailed,
}

/// One generation-qualified RFC 4028 deadline admitted only after the YAML
/// transition that armed it has committed.
#[derive(Debug, Clone)]
pub(crate) struct SessionRefreshTimerEffect {
    pub(crate) generation: u64,
    pub(crate) delay: std::time::Duration,
    pub(crate) kind: SessionRefreshDeadlineKind,
}

#[derive(Debug, Clone)]
pub(crate) struct ReinviteRetryEffect {
    pub(crate) kind: crate::session_store::state::PendingReinvite,
    pub(crate) attempt: u8,
    pub(crate) backoff: std::time::Duration,
}

/// One post-commit REFER progress operation bound to the exact lifetime that
/// owns the implicit subscription dialog.
///
/// The target-leg transition constructs this descriptor while holding its own
/// lane. The executor later admits the wire NOTIFY on the transferor's lane,
/// preventing cross-session lock inversion and raw session-ID reuse.
#[derive(Debug, Clone)]
pub(crate) struct TransferNotifyEffect {
    pub(crate) transferor: SessionRegistryHandle,
    pub(crate) status_code: u16,
    pub(crate) reason: String,
    pub(crate) observations: Vec<Event>,
}

fn exact_transferor_link(
    session: &SessionState,
) -> crate::errors::Result<Option<(SessionId, SessionRegistryHandle)>> {
    match (
        session.transferor_session_id.as_ref(),
        session.transferor_lifecycle_handle.as_ref(),
    ) {
        (None, None) if !session.is_transfer_call => Ok(None),
        (Some(session_id), Some(handle))
            if session.is_transfer_call && handle.session_id() == session_id =>
        {
            Ok(Some((session_id.clone(), handle.clone())))
        }
        _ => Err(crate::errors::SessionError::InvalidTransition(
            "transfer progress requires one matching exact transferor lifetime".to_string(),
        )),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegisterActionMode {
    Register,
    RegisterWithAuth,
    Unregister,
}

fn record_registration_auth_retry(session: &mut SessionState) {
    session.registration_retry_count = session.registration_retry_count.saturating_add(1);
}

fn record_registration_interval_retry(session: &mut SessionState, min_expires: u32) {
    session.registration_expires = Some(min_expires);
    session.registration_retry_count = session.registration_retry_count.saturating_add(1);
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
    triggering_event: &EventType,
    mode: RegisterActionMode,
    stage_claim: Option<&StageDispatchClaim>,
) -> Result<ActionOutcome, Box<dyn std::error::Error + Send + Sync>> {
    let session_id = session.session_id.clone();
    let staged_options = claim_builder_request_staging(
        session,
        PendingOptionsSlotKind::Register,
        BuilderStageLifetime::RetainedForRetry,
        stage_claim,
    )?;

    let unregister = mode == RegisterActionMode::Unregister
        || matches!(triggering_event, EventType::StartUnregistration)
        || matches!(
            staged_options.as_ref(),
            Some(PendingOptionsSlot::Register(options)) if options.expires == 0
        );
    let authenticated = mode == RegisterActionMode::RegisterWithAuth
        || matches!(triggering_event, EventType::AuthRequired { .. })
        || unregister;

    let mut options = if let Some(PendingOptionsSlot::Register(options)) = staged_options {
        (*options).clone()
    } else {
        let registrar_uri = session
            .registrar_uri
            .clone()
            .or_else(|| session.remote_uri.clone())
            .ok_or_else(|| "registrar_uri not set for registration".to_string())?;
        let aor_uri = session
            .local_uri
            .clone()
            .ok_or_else(|| "local_uri not set for registration".to_string())?;
        let contact_uri = session
            .registration_contact
            .clone()
            .or_else(|| session.local_uri.clone())
            .ok_or_else(|| "contact_uri not set for registration".to_string())?;
        let refresh = unregister
            || matches!(triggering_event, EventType::RefreshRegistration)
            || (authenticated && session.is_registered);
        rvoip_sip_dialog::api::unified::RegisterRequestOptions {
            registrar_uri,
            aor_uri,
            contact_uri,
            expires: if unregister {
                0
            } else {
                session.registration_expires.unwrap_or(3600)
            },
            authorization: None,
            proxy_authorization: None,
            call_id: session.registration_call_id.clone(),
            // The dialog adapter advances from lane-owned state. A staged
            // manual refresh may carry an explicitly reserved next CSeq;
            // synthesized lifecycle sends deliberately do not.
            cseq: None,
            outbound_contact: None,
            outbound_proxy_uri: None,
            extra_headers: Vec::new(),
            refresh,
        }
    };
    if unregister {
        options.expires = 0;
        options.refresh = true;
    }

    let registrar_uri = options.registrar_uri.clone();
    let from_uri = options.aor_uri.clone();
    let contact_uri = options.contact_uri.clone();
    let auth = authenticated.then(|| {
        session
            .auth
            .clone()
            .or_else(|| session.credentials.clone().map(Into::into))
    });
    let auth = auth.flatten();

    loop {
        let attempt = dialog_adapter
            .send_register_attempt(
                &session_id,
                options,
                auth.as_ref(),
                RegisterAttemptContext::capture(session),
            )
            .await?;
        attempt.context.apply(session);
        options = attempt.request_options;
        session.pending_register_options = Some(Arc::new(options.clone()));

        match attempt.outcome {
            RegisterAttemptOutcome::Registered {
                accepted_expires,
                metadata,
            } => {
                let effect = dialog_adapter.record_registration_success(
                    session,
                    &registrar_uri,
                    &from_uri,
                    &contact_uri,
                    accepted_expires,
                    metadata,
                );
                session.pending_register_options = None;
                return Ok(ActionOutcome::with_event_and_deferred_effect(
                    EventType::Registration200OK,
                    DeferredActionEffect::Registration(effect),
                ));
            }
            RegisterAttemptOutcome::Unregistered => {
                if unregister {
                    let effect =
                        dialog_adapter.record_unregistration_success(session, &registrar_uri);
                    session.pending_register_options = None;
                    return Ok(ActionOutcome::with_event_and_deferred_effect(
                        EventType::Unregistration200OK,
                        DeferredActionEffect::Registration(effect),
                    ));
                }

                let effect = dialog_adapter.record_registration_failure(
                    session,
                    &registrar_uri,
                    200,
                    "REGISTER returned an unregistration success while registering",
                );
                session.pending_register_options = None;
                return Ok(ActionOutcome::with_event_and_deferred_effect(
                    EventType::RegistrationFailed(200),
                    DeferredActionEffect::Registration(effect),
                ));
            }
            RegisterAttemptOutcome::AuthChallenge {
                status_code,
                challenge,
            } => {
                let challenge_details =
                    rvoip_auth_core::DigestAuthenticator::parse_challenge_details(&challenge).ok();
                let retry_count = session.registration_retry_count;
                let previous_nonce = session
                    .auth_challenge
                    .as_ref()
                    .map(|challenge| challenge.nonce.clone());
                // Origin (401) and proxy (407) protection spaces are
                // independent. A response only repeats an already-attempted
                // space when the exact request snapshot carried that space's
                // credential; the other challenge may still be answered while
                // retaining the first header.
                let challenged_space_was_authenticated = if status_code == 407 {
                    options.proxy_authorization.is_some()
                } else {
                    options.authorization.is_some()
                };
                let has_prior_auth_retry = retry_count > 0 && challenged_space_was_authenticated;
                let stale_recovery = has_prior_auth_retry
                    && !session.auth_challenge_stale
                    && challenge_details
                        .as_ref()
                        .is_some_and(|details| details.stale)
                    && previous_nonce.as_deref().is_some_and(|nonce| {
                        challenge_details
                            .as_ref()
                            .is_some_and(|details| nonce != details.challenge.nonce)
                    });
                if has_prior_auth_retry && !stale_recovery {
                    tracing::error!(
                        "❌ REGISTER auth failed (retry count {}); invalid credentials",
                        retry_count
                    );
                    session.pending_register_options = None;
                    if unregister {
                        let effect = dialog_adapter.record_unregistration_failure(
                            session,
                            &registrar_uri,
                            "unregistration authentication failed",
                        );
                        return Ok(ActionOutcome::with_event_and_deferred_effect(
                            EventType::UnregistrationFailed,
                            DeferredActionEffect::Registration(effect),
                        ));
                    } else {
                        let effect = dialog_adapter.record_registration_failure(
                            session,
                            &registrar_uri,
                            status_code,
                            "REGISTER authentication failed",
                        );
                        return Ok(ActionOutcome::with_event_and_deferred_effect(
                            EventType::RegistrationFailed(status_code),
                            DeferredActionEffect::Registration(effect),
                        ));
                    }
                }

                record_registration_auth_retry(session);
                return Ok(ActionOutcome::with_event(EventType::AuthRequired {
                    status_code,
                    challenge,
                    method: "REGISTER".to_string(),
                }));
            }
            RegisterAttemptOutcome::IntervalTooBrief { min_expires } => {
                if unregister {
                    let effect = dialog_adapter.record_unregistration_failure(
                        session,
                        &registrar_uri,
                        format!(
                            "unregistration received 423 Interval Too Brief Min-Expires={}",
                            min_expires
                        ),
                    );
                    session.pending_register_options = None;
                    return Ok(ActionOutcome::with_event_and_deferred_effect(
                        EventType::UnregistrationFailed,
                        DeferredActionEffect::Registration(effect),
                    ));
                }

                let retry_count = session.registration_retry_count;
                if retry_count >= 2 {
                    tracing::error!(
                        "❌ Registration failed with repeated 423 — giving up (retry count {})",
                        retry_count
                    );
                    let effect = dialog_adapter.record_registration_failure(
                        session,
                        &registrar_uri,
                        423,
                        "Registration failed with repeated 423 Interval Too Brief responses",
                    );
                    session.pending_register_options = None;
                    return Ok(ActionOutcome::with_event_and_deferred_effect(
                        EventType::RegistrationFailed(423),
                        DeferredActionEffect::Registration(effect),
                    ));
                }

                tracing::info!(
                    "🔄 423 Interval Too Brief — retrying REGISTER with Expires={} (server required min)",
                    min_expires
                );
                record_registration_interval_retry(session, min_expires);
                options.expires = min_expires;
                session.pending_register_options = Some(Arc::new(options.clone()));
            }
            RegisterAttemptOutcome::Failure {
                status_code,
                reason,
            } => {
                if unregister {
                    let effect = dialog_adapter.record_unregistration_failure(
                        session,
                        &registrar_uri,
                        format!("{} (status {})", reason, status_code),
                    );
                    session.pending_register_options = None;
                    return Ok(ActionOutcome::with_event_and_deferred_effect(
                        EventType::UnregistrationFailed,
                        DeferredActionEffect::Registration(effect),
                    ));
                }

                let effect = dialog_adapter.record_registration_failure(
                    session,
                    &registrar_uri,
                    status_code,
                    reason,
                );
                session.pending_register_options = None;
                return Ok(ActionOutcome::with_event_and_deferred_effect(
                    EventType::RegistrationFailed(status_code),
                    DeferredActionEffect::Registration(effect),
                ));
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
fn claim_tracked_request_staging(
    session: &mut SessionState,
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
        TrackedInDialogMethod::Reinvite => session
            .pending_reinvite_options
            .as_ref()
            .map(|options| PendingOptionsSlot::ReInvite(Arc::clone(options))),
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

    // The executor holds this exact session's complete-event lane for the
    // entire transition. Claim the staged Arc from its one lane-owned working
    // state; publishing a second store revision here would create a competing
    // writer without adding any serialization.
    let claimed = claim.claim_exact(session)?;

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
        (TrackedInDialogMethod::Reinvite, PendingOptionsSlot::ReInvite(options)) => {
            Ok(TrackedInDialogOptions::Reinvite(options))
        }
        _ => Err(crate::errors::SessionError::InvalidTransition(
            "outbound request stage claim changed method".to_string(),
        )),
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum BuilderStageLifetime {
    /// The immutable snapshot remains staged until the final-response owner
    /// clears it because a protocol/authentication retry may need it.
    RetainedForRetry,
    /// The action takes the immutable snapshot before its first wire await.
    ConsumeBeforeWire,
}

fn pending_options_slot(
    session: &SessionState,
    kind: PendingOptionsSlotKind,
) -> Option<PendingOptionsSlot> {
    match kind {
        PendingOptionsSlotKind::Invite => session
            .pending_invite_options
            .as_ref()
            .map(|options| PendingOptionsSlot::Invite(Arc::clone(options))),
        PendingOptionsSlotKind::ReInvite => session
            .pending_reinvite_options
            .as_ref()
            .map(|options| PendingOptionsSlot::ReInvite(Arc::clone(options))),
        PendingOptionsSlotKind::Register => session
            .pending_register_options
            .as_ref()
            .map(|options| PendingOptionsSlot::Register(Arc::clone(options))),
        PendingOptionsSlotKind::Refer => session
            .pending_refer_options
            .as_ref()
            .map(|options| PendingOptionsSlot::Refer(Arc::clone(options))),
        PendingOptionsSlotKind::Bye => session
            .pending_bye_options
            .as_ref()
            .map(|options| PendingOptionsSlot::Bye(Arc::clone(options))),
        PendingOptionsSlotKind::Cancel => session
            .pending_cancel_options
            .as_ref()
            .map(|options| PendingOptionsSlot::Cancel(Arc::clone(options))),
        PendingOptionsSlotKind::Notify => session
            .pending_notify_options
            .as_ref()
            .map(|options| PendingOptionsSlot::Notify(Arc::clone(options))),
        PendingOptionsSlotKind::Subscribe => session
            .pending_subscribe_options
            .as_ref()
            .map(|options| PendingOptionsSlot::Subscribe(Arc::clone(options))),
        PendingOptionsSlotKind::Info => session
            .pending_info_options
            .as_ref()
            .map(|options| PendingOptionsSlot::Info(Arc::clone(options))),
        PendingOptionsSlotKind::Update => session
            .pending_update_options
            .as_ref()
            .map(|options| PendingOptionsSlot::Update(Arc::clone(options))),
        PendingOptionsSlotKind::Message => session
            .pending_message_options
            .as_ref()
            .map(|options| PendingOptionsSlot::Message(Arc::clone(options))),
        PendingOptionsSlotKind::Options => session
            .pending_options_options
            .as_ref()
            .map(|options| PendingOptionsSlot::Options(Arc::clone(options))),
    }
}

/// Transfer one builder's exact staged Arc into its action before the first
/// wire await. `stage_claim == None` is the public two-step compatibility
/// path: synthesize a claim around the already-staged exact Arc and apply the
/// same retained/consumed lifetime policy.
fn claim_builder_request_staging(
    session: &mut SessionState,
    kind: PendingOptionsSlotKind,
    lifetime: BuilderStageLifetime,
    stage_claim: Option<&StageDispatchClaim>,
) -> crate::errors::Result<Option<PendingOptionsSlot>> {
    let fallback_slot = pending_options_slot(session, kind);
    let fallback_claim = fallback_slot.map(StageDispatchClaim::new);
    let Some(claim) = stage_claim.or(fallback_claim.as_ref()) else {
        return Ok(None);
    };
    if claim.kind() != kind {
        return Err(crate::errors::SessionError::InvalidTransition(format!(
            "outbound staged-options kind mismatch: expected {kind:?}, received {:?}",
            claim.kind()
        )));
    }

    let claimed = match lifetime {
        BuilderStageLifetime::RetainedForRetry => claim.claim_retained_exact(session)?,
        BuilderStageLifetime::ConsumeBeforeWire => claim.claim_exact(session)?,
    };
    Ok(Some(claimed))
}

fn advance_tracked_auth_owner(
    session: &mut SessionState,
    method: TrackedInDialogMethod,
    retry_transaction: &rvoip_sip_dialog::transaction::TransactionKey,
    request_uri: &str,
) {
    let retry_id = retry_transaction.to_string();

    // The executor owns this mutable state for the complete AuthRequired
    // transition. Advancing the exact request owner here is therefore the
    // authoritative mutation; publishing a second partial SessionStore write
    // would recreate the race this lane exists to prevent.
    session.pending_auth_transaction_id = Some(retry_id);
    session.pending_auth_request_uri = Some(request_uri.to_string());
    session.pending_auth_method = Some(method.as_sip_method().to_string());
}

pub(crate) async fn execute_action(
    action: &Action,
    triggering_event: &EventType,
    session: &mut SessionState,
    dialog_adapter: &Arc<DialogAdapter>,
    media_adapter: &Arc<MediaAdapter>,
    _simple_peer_event_tx: &Option<tokio::sync::mpsc::Sender<Event>>, // Unused - events handled by SessionCrossCrateEventHandler
    stage_claim: Option<&StageDispatchClaim>,
    mut inbound_response: Option<&mut InboundResponseStateInput>,
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
            if session.role == crate::state_table::Role::UAS {
                // Dialog-core has already created and registered the inbound
                // UAS dialog before the IncomingCall event enters this exact
                // lane. Adopt that canonical identity into the working state
                // so later in-dialog actions can require registry/state
                // agreement without falling back to a raw-ID lookup.
                let handle = exact_dialog_cleanup_handle(session)?;
                let dialog_id = dialog_adapter
                    .store
                    .registry()
                    .get_dialog_handle_exact(&handle)
                    .ok_or_else(|| {
                        crate::errors::SessionError::InvalidTransition(
                            "inbound UAS session has no exact registry dialog".to_string(),
                        )
                    })?;
                if session
                    .dialog_id
                    .as_ref()
                    .is_some_and(|current| current != &dialog_id)
                {
                    return Err(crate::errors::SessionError::InvalidTransition(
                        "inbound UAS state dialog conflicts with its exact registry owner"
                            .to_string(),
                    )
                    .into());
                }
                session.dialog_id = Some(dialog_id);
                info!("Adopted inbound UAS dialog from exact registry owner");
            } else {
                // Outbound UAC dialog creation remains owned by the initial
                // INVITE dispatch, which installs the exact registry mapping.
                info!("Dialog will be created when INVITE is sent");
            }
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
                let sdp = media_adapter.generate_local_sdp_lane_owned(session).await?;
                session.local_sdp = Some(sdp.clone());
                info!("Generated SDP with {} bytes", sdp.len());
            }
            // A fast 401/407 is delivered through the same exact-session lane,
            // so it cannot run until this transition canonically publishes the
            // lane-owned SDP below.
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
            if !(400..=699).contains(&status) {
                return Err(zero_wire_exact_response_error(
                    "initial INVITE rejection status is not 4xx, 5xx, or 6xx",
                ));
            }
            let extras = session
                .reject_response_extras
                .clone()
                .filter(|extras| !extras.is_empty());
            let terminal = send_exact_initial_invite_final_response(
                session,
                dialog_adapter,
                status,
                None,
                extras,
            )
            .await?;
            consume_exact_initial_invite_response_authority(
                session,
                dialog_adapter,
                &terminal,
                false,
            );
            session.reject_response_extras.take();
            if let Some(error) = terminal.terminal_error {
                return Err(error);
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
            let extras = session
                .reject_response_extras
                .clone()
                .filter(|extras| !extras.is_empty());
            if !(300..=399).contains(&status) {
                return Err(zero_wire_exact_response_error(
                    "initial INVITE redirect status is not 3xx",
                ));
            }
            let headers = exact_redirect_response_headers(&contacts, extras)?;
            let terminal = send_exact_initial_invite_final_response(
                session,
                dialog_adapter,
                status,
                None,
                Some(headers),
            )
            .await?;
            consume_exact_initial_invite_response_authority(
                session,
                dialog_adapter,
                &terminal,
                false,
            );
            session.reject_response_extras.take();
            if let Some(error) = terminal.terminal_error {
                return Err(error);
            }
        }
        Action::SendSIPResponse(code, _reason) => {
            let response_code = session.pending_response_status_override.unwrap_or(*code);
            let extras = session
                .reject_response_extras
                .clone()
                .filter(|extras| !extras.is_empty());
            let guard = (response_code == 200).then(|| {
                cleanup_diag::stage_guard(CleanupStage::ActionSend200Ok, &session.session_id.0)
            });
            let response_is_final = (200..=699).contains(&response_code);
            let response_is_provisional = (100..=199).contains(&response_code);
            if !response_is_final && !response_is_provisional {
                return Err(zero_wire_exact_response_error(
                    "YAML UAS response status is outside the SIP response range",
                ));
            }

            let initial_invite_event = matches!(
                triggering_event,
                EventType::IncomingCall { .. } | EventType::IncomingCallAutoAccept { .. }
            );
            if initial_invite_event {
                validate_initial_invite_event_response_authority(
                    session,
                    inbound_response.as_deref(),
                )?;
            }
            // Event-local authority always wins. The retained initial-INVITE
            // capability is consulted only for a later application event that
            // has no inbound request attached (Accept/Reject/EarlyMedia).
            let initial_invite_response = initial_invite_event
                || (inbound_response.is_none()
                    && session.pending_inbound_invite_transaction_id.is_some());
            let mut terminal_error = None;
            if response_is_final && initial_invite_response {
                let terminal = send_exact_initial_invite_final_response(
                    session,
                    dialog_adapter,
                    response_code,
                    session.local_sdp.clone(),
                    extras,
                )
                .await?;
                consume_exact_initial_invite_response_authority(
                    session,
                    dialog_adapter,
                    &terminal,
                    response_code == 200,
                );
                terminal_error = terminal.terminal_error;
            } else if response_is_final {
                let terminal = send_exact_inbound_final_response(
                    &session.session_id,
                    inbound_response.as_deref_mut(),
                    dialog_adapter,
                    response_code,
                    session.local_sdp.clone(),
                    extras,
                )
                .await?;
                terminal_error = terminal.terminal_error;
            } else if initial_invite_response {
                send_exact_initial_invite_provisional_response(
                    session,
                    dialog_adapter,
                    response_code,
                    session.local_sdp.clone(),
                    extras,
                )
                .await?;
            } else {
                send_exact_inbound_provisional_response(
                    &session.session_id,
                    inbound_response.as_deref_mut(),
                    dialog_adapter,
                    response_code,
                    session.local_sdp.clone(),
                    extras,
                )
                .await?;
            }

            // The event-local response input remains intact across every
            // await. Consume it only after success or an explicitly terminal
            // wire-unknown classification.
            session.pending_response_status_override.take();
            session.reject_response_extras.take();
            // RFC 3261: Dialog is established when the UAS sends a 2xx final
            // response to the initial INVITE.
            if response_is_final && initial_invite_response && (200..300).contains(&response_code) {
                session.dialog_established = true;
                info!(
                    "Dialog established (UAS sent {} final response) for session {}",
                    response_code, session.session_id
                );
            }
            if let Some(error) = terminal_error {
                return Err(error);
            }
            if let Some(success_guard) = guard {
                success_guard.finish_success();
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
            if let Ok(dialog_id) = dialog_adapter.initial_invite_dialog_lane_owned(session) {
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
                "⏳ 491 glare: scheduling {:?} retry after {:?} for session {} (attempt {}/{})",
                kind,
                backoff,
                session.session_id,
                session.reinvite_retry_attempts,
                MAX_GLARE_RETRIES
            );
            return Ok(ActionOutcome::with_deferred_effect(
                DeferredActionEffect::ReinviteRetry(ReinviteRetryEffect {
                    kind,
                    attempt: session.reinvite_retry_attempts,
                    backoff,
                }),
            ));
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
            if let Ok(dialog_id) = dialog_adapter.initial_invite_dialog_lane_owned(session) {
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
            let mut snapshot = if let Some(opts) = session.pending_bye_options.as_ref() {
                (**opts).clone()
            } else {
                let materialized = Arc::new(rvoip_sip_dialog::api::unified::ByeRequestOptions {
                    reason: None,
                    extra_headers: dialog_adapter.auto_emit_extra_headers.clone(),
                });
                // Builder staging and this transition share the exact-session
                // lane. A snapshot staged first was selected above; otherwise
                // this transition owns and publishes the automatic snapshot.
                session.pending_bye_options = Some(Arc::clone(&materialized));
                (*materialized).clone()
            };
            // An automatic lifecycle reason is authoritative even when an
            // application snapshot was already staged. Preserve every other
            // immutable option, but materialize exactly one RFC 3326 Reason so
            // timer expiry cannot silently inherit an unrelated cause=200.
            if let Some((protocol, cause, text)) = reason {
                snapshot.reason = None;
                snapshot
                    .extra_headers
                    .retain(|header| !matches!(header, TypedHeader::Reason(_)));
                snapshot.extra_headers.push(TypedHeader::Reason(
                    rvoip_sip_core::types::reason::Reason::new(protocol, cause, text),
                ));
                session.pending_bye_options = Some(Arc::new(snapshot.clone()));
            }
            if let Err(error) = dialog_adapter
                .send_bye_with_options_lane_owned(session, snapshot)
                .await
            {
                // An immediate zero-wire failure has no exact final-response
                // owner to release the retained builder slot.
                session.pending_bye_options = None;
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
                .create_hold_sdp_for_session_lane_owned(session)
                .await
                .map_err(|e| format!("create_hold_sdp failed: {}", e))?;
            session.local_sdp = Some(hold_sdp.clone());
            session.pending_reinvite = Some(crate::session_store::state::PendingReinvite::Hold);
            dialog_adapter
                .send_reinvite_with_options_lane_owned(
                    session,
                    ReInviteRequestOptions {
                        sdp: Some(hold_sdp),
                        ..Default::default()
                    },
                )
                .await?;
        }
        Action::ResumeCall => {
            // Send re-INVITE with sendrecv SDP.
            let active_sdp = media_adapter
                .create_active_sdp_for_session_lane_owned(session)
                .await
                .map_err(|e| format!("create_active_sdp failed: {}", e))?;
            session.local_sdp = Some(active_sdp.clone());
            session.pending_reinvite = Some(crate::session_store::state::PendingReinvite::Resume);
            dialog_adapter
                .send_reinvite_with_options_lane_owned(
                    session,
                    ReInviteRequestOptions {
                        sdp: Some(active_sdp),
                        ..Default::default()
                    },
                )
                .await?;
        }
        Action::TransferCall(target) => {
            session.transfer_target = Some(target.clone());
            session.transfer_state = crate::session_store::state::TransferState::TransferInitiated;
            dialog_adapter
                .send_refer_with_options_lane_owned(
                    session,
                    ReferRequestOptions {
                        refer_to: target.clone(),
                        ..Default::default()
                    },
                )
                .await?;
            info!(
                session = %session.session_id.0,
                target_present = !target.is_empty(),
                target_bytes = target.len(),
                "Sent REFER"
            );
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
            if let Some(remote_sdp) = session.remote_sdp.clone() {
                let config = media_adapter
                    .negotiate_sdp_as_uac_lane_owned(session, &remote_sdp)
                    .await?;

                // Convert to session_store NegotiatedConfig
                let session_config = crate::session_store::state::NegotiatedConfig {
                    local_addr: config.local_addr,
                    remote_addr: config.remote_addr,
                    codec: config.codec,
                    payload_type: config.payload_type,
                    sample_rate: config.clock_rate,
                    channels: config.channels,
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
            } else if let Some(remote_sdp) = session.remote_sdp.clone() {
                let (local_sdp, config) = media_adapter
                    .negotiate_sdp_as_uas_lane_owned(session, &remote_sdp)
                    .await?;

                // Convert to session_store NegotiatedConfig
                let session_config = crate::session_store::state::NegotiatedConfig {
                    local_addr: config.local_addr,
                    remote_addr: config.remote_addr,
                    codec: config.codec,
                    payload_type: config.payload_type,
                    sample_rate: config.clock_rate,
                    channels: config.channels,
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
                    .negotiate_sdp_as_uas_lane_owned(session, &remote_sdp)
                    .await?;
                let session_config = crate::session_store::state::NegotiatedConfig {
                    local_addr: config.local_addr,
                    remote_addr: config.remote_addr,
                    codec: config.codec,
                    payload_type: config.payload_type,
                    sample_rate: config.clock_rate,
                    channels: config.channels,
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
            let handle = exact_dialog_cleanup_handle(session)?;
            dialog_adapter
                .cleanup_session_exact_lane_owned(&handle)
                .await?;
            retire_lane_owned_dialog_identity(session);
            debug!(
                "Dialog cleanup completed for session {}",
                session.session_id
            );
        }
        Action::StartMediaCleanup => {
            cleanup_lane_owned_media(session, media_adapter).await?;
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
                    .create_hold_sdp_for_session_lane_owned(session)
                    .await
                    .map_err(|e| format!("create_hold_sdp failed: {}", e))?
            } else {
                media_adapter
                    .create_active_sdp_for_session_lane_owned(session)
                    .await
                    .map_err(|e| format!("create_active_sdp failed: {}", e))?
            };
            session.local_sdp = Some(sdp.clone());
            session.pending_reinvite = Some(kind);
            // A 491/ReinviteGlare response is queued on the same exact-session
            // lane. It observes this SDP and retry intent only after the
            // executor's canonical transition publication.
            debug!(
                "Sending re-INVITE for session {} (hold={})",
                session.session_id, hold_direction
            );
            dialog_adapter
                .send_reinvite_with_options_lane_owned(
                    session,
                    ReInviteRequestOptions {
                        sdp: Some(sdp),
                        ..Default::default()
                    },
                )
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
            let mixer_id = session.conference_mixer_id.clone().ok_or_else(|| {
                crate::errors::SessionError::InvalidTransition(format!(
                    "RedirectToMixer requires an owned mixer for session {}",
                    session.session_id
                ))
            })?;
            let media_id = session.media_session_id.clone().ok_or_else(|| {
                crate::errors::SessionError::InvalidTransition(format!(
                    "RedirectToMixer requires an owned media session for session {}",
                    session.session_id
                ))
            })?;
            media_adapter.redirect_to_mixer(media_id, mixer_id).await?;
        }

        Action::ConnectToMixer => {
            return Err(crate::errors::SessionError::InvalidTransition(format!(
                "ConnectToMixer has no media-plane implementation for session {}; use an owned BridgeHandle",
                session.session_id
            ))
            .into());
        }

        Action::DisconnectFromMixer => {
            return Err(crate::errors::SessionError::InvalidTransition(format!(
                "DisconnectFromMixer has no media-plane implementation for session {}; release the owned BridgeHandle",
                session.session_id
            ))
            .into());
        }

        Action::MuteToMixer => {
            debug!("Muting session {} to mixer", session.session_id);
            if session.conference_mixer_id.is_none() {
                return Err(crate::errors::SessionError::InvalidTransition(format!(
                    "MuteToMixer requires an owned mixer for session {}",
                    session.session_id
                ))
                .into());
            }
            let media_id = session.media_session_id.clone().ok_or_else(|| {
                crate::errors::SessionError::InvalidTransition(format!(
                    "MuteToMixer requires an owned media session for session {}",
                    session.session_id
                ))
            })?;
            media_adapter.set_mute(media_id, true).await?;
        }

        Action::UnmuteToMixer => {
            debug!("Unmuting session {} to mixer", session.session_id);
            if session.conference_mixer_id.is_none() {
                return Err(crate::errors::SessionError::InvalidTransition(format!(
                    "UnmuteToMixer requires an owned mixer for session {}",
                    session.session_id
                ))
                .into());
            }
            let media_id = session.media_session_id.clone().ok_or_else(|| {
                crate::errors::SessionError::InvalidTransition(format!(
                    "UnmuteToMixer requires an owned media session for session {}",
                    session.session_id
                ))
            })?;
            media_adapter.set_mute(media_id, false).await?;
        }

        Action::DestroyMixer => {
            debug!("Destroying conference mixer");
            let mixer_id = session.conference_mixer_id.clone().ok_or_else(|| {
                crate::errors::SessionError::InvalidTransition(format!(
                    "DestroyMixer requires an owned mixer for session {}",
                    session.session_id
                ))
            })?;
            media_adapter.destroy_mixer(mixer_id).await?;
            session.conference_mixer_id = None;
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
            let mixer_id = session.conference_mixer_id.as_ref().ok_or_else(|| {
                crate::errors::SessionError::InvalidTransition(format!(
                    "StartRecordingMixer requires an owned mixer for session {}",
                    session.session_id
                ))
            })?;
            let mixer_session_id = SessionId(format!("mixer-{}", mixer_id.as_str()));
            media_adapter.start_recording(&mixer_session_id).await?;
        }

        Action::StopRecordingMixer => {
            debug!("Stopping recording of conference mixer");
            let mixer_id = session.conference_mixer_id.as_ref().ok_or_else(|| {
                crate::errors::SessionError::InvalidTransition(format!(
                    "StopRecordingMixer requires an owned mixer for session {}",
                    session.session_id
                ))
            })?;
            let mixer_session_id = SessionId(format!("mixer-{}", mixer_id.as_str()));
            media_adapter.stop_recording(&mixer_session_id).await?;
        }

        Action::ReleaseAllResources => {
            debug!("Releasing all resources for session {}", session.session_id);
            release_lane_owned_resources(session, dialog_adapter, media_adapter).await?;
        }

        Action::StartEmergencyCleanup => {
            error!(
                "Starting emergency cleanup for session {}",
                session.session_id
            );
            // Best-effort cleanup on error
            if let Ok(handle) = exact_dialog_cleanup_handle(session) {
                if dialog_adapter
                    .cleanup_session_exact_lane_owned(&handle)
                    .await
                    .is_ok()
                {
                    retire_lane_owned_dialog_identity(session);
                }
            }
            let _ = cleanup_lane_owned_media(session, media_adapter).await;
        }

        Action::AttemptMediaRecovery => {
            return Err(crate::errors::SessionError::InvalidTransition(format!(
                "AttemptMediaRecovery is not implemented for session {}; the state table must choose an explicit cleanup or retry transition",
                session.session_id
            ))
            .into());
        }

        Action::Custom(action_name) => {
            debug!(
                "Custom action '{}' for session {}",
                action_name, session.session_id
            );
            // Handle custom SIP actions
            match action_name.as_str() {
                "ArmSessionRefreshTimer" => {
                    use crate::session_store::state::SessionRefreshPhase;

                    let Some((interval_secs, local_refresher)) =
                        dialog_adapter.session_timer_settings_lane_owned(session)?
                    else {
                        next_session_refresh_generation(session);
                        session.session_refresh_interval_secs = None;
                        session.session_refresh_local_refresher = false;
                        session.session_refresh_phase = SessionRefreshPhase::Idle;
                        return Ok(ActionOutcome::default());
                    };
                    let generation = next_session_refresh_generation(session);
                    session.session_refresh_interval_secs = Some(interval_secs);
                    session.session_refresh_local_refresher = local_refresher;
                    session.session_refresh_phase = SessionRefreshPhase::Idle;
                    let delay_secs = if local_refresher {
                        (interval_secs / 2).max(1)
                    } else {
                        interval_secs.max(1)
                    };
                    let kind = if local_refresher {
                        SessionRefreshDeadlineKind::UpdateDue
                    } else {
                        SessionRefreshDeadlineKind::PeerExpired
                    };
                    return Ok(ActionOutcome::with_deferred_effect(
                        DeferredActionEffect::SessionRefreshTimer(SessionRefreshTimerEffect {
                            generation,
                            delay: std::time::Duration::from_secs(u64::from(delay_secs)),
                            kind,
                        }),
                    ));
                }
                "PrepareSessionRefreshUpdate" => {
                    return Ok(prepare_session_refresh_update(session, dialog_adapter)?);
                }
                "PrepareSessionRefreshReinvite" => {
                    return Ok(prepare_session_refresh_reinvite(session)?);
                }
                "PrepareSessionRefreshExpiry" => {
                    use crate::session_store::state::SessionRefreshPhase;

                    next_session_refresh_generation(session);
                    session.session_refresh_phase = SessionRefreshPhase::Idle;
                    session.pending_bye_reason =
                        Some(("SIP".to_string(), 408, Some("Session expired".to_string())));
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
                "CheckReadiness" => {
                    return Ok(ActionOutcome::with_event(EventType::CheckConditions));
                }
                _ => {
                    return Err(crate::errors::SessionError::InvalidTransition(format!(
                        "unsupported custom state-machine action '{}' for session {}",
                        action_name, session.session_id
                    ))
                    .into());
                }
            }
        }

        Action::BridgeToMixer => {
            return Err(crate::errors::SessionError::InvalidTransition(format!(
                "BridgeToMixer has no media-plane implementation for session {}; use an owned BridgeHandle",
                session.session_id
            ))
            .into());
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
                .create_active_sdp_for_session_lane_owned(session)
                .await
                .map_err(|e| format!("create_active_sdp failed: {}", e))?;
            session.local_sdp = Some(active_sdp.clone());
            dialog_adapter
                .send_reinvite_with_options_lane_owned(
                    session,
                    ReInviteRequestOptions {
                        sdp: Some(active_sdp),
                        ..Default::default()
                    },
                )
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
                .create_active_sdp_for_session_lane_owned(session)
                .await
                .map_err(|e| format!("create_active_sdp failed: {}", e))?;
            session.local_sdp = Some(active_sdp.clone());
            dialog_adapter
                .send_reinvite_with_options_lane_owned(
                    session,
                    ReInviteRequestOptions {
                        sdp: Some(active_sdp),
                        ..Default::default()
                    },
                )
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
                .create_hold_sdp_for_session_lane_owned(session)
                .await
                .map_err(|e| format!("create_hold_sdp failed: {}", e))?;
            session.local_sdp = Some(hold_sdp.clone());
            dialog_adapter
                .send_reinvite_with_options_lane_owned(
                    session,
                    ReInviteRequestOptions {
                        sdp: Some(hold_sdp),
                        ..Default::default()
                    },
                )
                .await?;

            info!("Call {} put on hold", session.session_id);
        }

        Action::CleanupResources => {
            debug!("Cleaning up resources for session {}", session.session_id);
            release_lane_owned_resources(session, dialog_adapter, media_adapter).await?;
        }

        // Registration actions
        Action::SendREGISTER => {
            info!("Action::SendREGISTER for session {}", session.session_id);
            return execute_register_action(
                session,
                dialog_adapter,
                triggering_event,
                RegisterActionMode::Register,
                stage_claim,
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
                triggering_event,
                RegisterActionMode::RegisterWithAuth,
                stage_claim,
            )
            .await;
        }

        Action::SendUnREGISTER => {
            info!("Action::SendUnREGISTER for session {}", session.session_id);
            return execute_register_action(
                session,
                dialog_adapter,
                triggering_event,
                RegisterActionMode::Unregister,
                stage_claim,
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
                // The following auth action consumes this same lane-owned
                // working state. The executor commits it once after the
                // transition, including on an action failure.
            } else {
                return Err(format!(
                    "StoreAuthChallenge: no admitted pending_auth on session {}",
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
                            exact_request_tracker_handle(session)?,
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
                            exact_request_tracker_handle(session)?,
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
                        .send_bye_with_auth_lane_owned(session, opts, header_name, header_value)
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
                        exact_request_tracker_handle(session)?,
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
                        .send_refer_with_auth_lane_owned(
                            session,
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
                        TrackedInDialogMethod::Refer,
                        &transaction_id,
                        &request_uri,
                    );
                }
                "NOTIFY" => {
                    let challenged_transaction = challenged_transaction.as_ref().ok_or_else(|| {
                        crate::errors::SessionError::InvalidTransition(
                            "SendRequestWithAuth(NOTIFY): exact challenged transaction is unavailable"
                                .to_string(),
                        )
                    })?;
                    let (lease, options) = dialog_adapter.outbound_request_tracker.prepare_retry(
                        exact_request_tracker_handle(session)?,
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
                        .send_notify_with_auth_lane_owned(
                            session,
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
                        TrackedInDialogMethod::Notify,
                        &transaction_id,
                        &request_uri,
                    );
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
                        exact_request_tracker_handle(session)?,
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
                        .send_info_with_auth_lane_owned(
                            session,
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
                        TrackedInDialogMethod::Info,
                        &transaction_id,
                        &request_uri,
                    );
                }
                "UPDATE" => {
                    let challenged_transaction = challenged_transaction.as_ref().ok_or_else(|| {
                        crate::errors::SessionError::InvalidTransition(
                            "SendRequestWithAuth(UPDATE): exact challenged transaction is unavailable"
                                .to_string(),
                        )
                    })?;
                    let (lease, options) = dialog_adapter.outbound_request_tracker.prepare_retry(
                        exact_request_tracker_handle(session)?,
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
                        .send_update_with_auth_lane_owned(
                            session,
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
                        TrackedInDialogMethod::Update,
                        &transaction_id,
                        &request_uri,
                    );
                }
                "INVITE" => {
                    let challenged_transaction = challenged_transaction.as_ref().ok_or_else(|| {
                        crate::errors::SessionError::InvalidTransition(
                            "SendRequestWithAuth(INVITE): exact challenged re-INVITE transaction is unavailable"
                                .to_string(),
                        )
                    })?;
                    let (lease, options) = dialog_adapter.outbound_request_tracker.prepare_retry(
                        exact_request_tracker_handle(session)?,
                        TrackedInDialogMethod::Reinvite,
                        challenged_transaction,
                        challenge_nonce,
                    )?;
                    let TrackedInDialogOptions::Reinvite(options) = options else {
                        return Err(
                            "SendRequestWithAuth(INVITE): tracker option type mismatch".into()
                        );
                    };
                    let transaction_id = dialog_adapter
                        .send_reinvite_with_auth_lane_owned(
                            session,
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
                        TrackedInDialogMethod::Reinvite,
                        &transaction_id,
                        &request_uri,
                    );
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
            // Dialog-core already validated and acknowledged the wire request.
            // The executor applies the private typed REFER-NOTIFY input to this
            // lane-owned working state before invoking the action; retaining an
            // explicit YAML action makes the canonical commit an auditable
            // prerequisite for derived application observations.
        }
        // Action::SendNOTIFY deleted per SIP_API_DESIGN_2.md Phase 5 —
        // consolidated into Action::SendNOTIFYWithOptions. YAML emit
        // rows updated to reference SendNOTIFYWithOptions.

        // Message actions
        Action::SendMESSAGE => {
            info!("Action::SendMESSAGE for session {}", session.session_id);
            let from_uri = session
                .local_uri
                .clone()
                .ok_or_else(|| "local_uri not set for message".to_string())?;
            let to_uri = session
                .remote_uri
                .clone()
                .ok_or_else(|| "to_uri not set for message".to_string())?;
            // Get message body from session (could be stored in a specific field)
            let body = session
                .local_sdp
                .clone()
                .unwrap_or_else(|| "Test message".to_string());
            let in_dialog = session.dialog_id.is_some(); // Send in-dialog if we have a dialog
            if let Some(follow_up) = dialog_adapter
                .send_message_lane_owned(session, &from_uri, &to_uri, body, in_dialog)
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
                let handle = exact_dialog_cleanup_handle(session)?;
                cleanup_dialog_on_fresh_task(Arc::clone(dialog_adapter), handle).await?;
            }
            // Cleanup and the replacement INVITE execute in the same exact
            // session lane. Retire the lane-owned identity immediately after
            // lower cleanup succeeds so a redirect cannot try to publish its
            // new dialog while the working snapshot still names the old one.
            // The executor remains the only publisher of this mutation.
            retire_lane_owned_dialog_identity(session);
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
            // Always run cleanup — the adapter is idempotent and
            // media-core may still have state even when our `media_session_id`
            // field looks empty (e.g. a previous cleanup cleared the field
            // but stop_media hasn't landed yet).
            cleanup_lane_owned_media(session, media_adapter).await?;
        }

        // ===== REFER Response Action =====
        Action::SendReferAccepted => {
            debug!("Sending 202 Accepted for REFER request");

            let transaction_id = session
                .refer_transaction_id
                .clone()
                .ok_or_else(|| "REFER acceptance has no pending transaction".to_string())?;
            let terminal = send_exact_refer_final_response(
                &session.session_id,
                &transaction_id,
                dialog_adapter,
                202,
            )
            .await?;
            if session.refer_transaction_id.as_deref() == Some(transaction_id.as_str()) {
                session.refer_transaction_id = None;
            }
            if let Some(error) = terminal.terminal_error {
                return Err(error);
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
            let handle = session.lifecycle_handle.clone().ok_or_else(|| {
                crate::errors::SessionError::InvalidTransition(
                    "REFER acceptance NOTIFY requires exact session authority".to_string(),
                )
            })?;
            if dialog_adapter
                .send_refer_notify_lane_owned(&handle, session, 100, "Trying")
                .await
                .is_err()
            {
                warn!(session = %session.session_id, "Failed to send 100 Trying NOTIFY");
            }
        }

        Action::SendTransferNotifyRinging => {
            if let Some((transferor, transferor_handle)) = exact_transferor_link(session)? {
                debug!(
                    "SendTransferNotifyRinging: leg {} -> transferor {}",
                    session.session_id, transferor
                );
                return Ok(ActionOutcome::with_deferred_effect(
                    DeferredActionEffect::TransferNotify(TransferNotifyEffect {
                        transferor: transferor_handle,
                        status_code: 180,
                        reason: "Ringing".to_string(),
                        observations: vec![
                            Event::ReferNotify {
                                call_id: transferor.clone(),
                                status_code: 180,
                                reason: "Ringing".to_string(),
                                subscription_state: None,
                                body: Some("SIP/2.0 180 Ringing\r\n".to_string()),
                            },
                            Event::ReferProgress {
                                call_id: transferor,
                                status_code: 180,
                                reason: "Ringing".to_string(),
                            },
                        ],
                    }),
                ));
            }
            // Shared dialog-progress rows also serve ordinary calls. `None`
            // means no transfer operation was requested, so there is no
            // transfer outcome to report. Any partial/stale transfer linkage
            // already failed closed in `exact_transferor_link` above.
        }

        Action::SendTransferNotifySuccess => {
            if let Some((transferor, transferor_handle)) = exact_transferor_link(session)? {
                debug!(
                    "SendTransferNotifySuccess: leg {} -> transferor {}",
                    session.session_id, transferor
                );
                return Ok(ActionOutcome::with_deferred_effect(
                    DeferredActionEffect::TransferNotify(TransferNotifyEffect {
                        transferor: transferor_handle,
                        status_code: 200,
                        reason: "OK".to_string(),
                        observations: vec![
                            Event::ReferNotify {
                                call_id: transferor.clone(),
                                status_code: 200,
                                reason: "OK".to_string(),
                                subscription_state: None,
                                body: Some("SIP/2.0 200 OK\r\n".to_string()),
                            },
                            Event::TransferTargetAnswered {
                                transfer_call_id: transferor.clone(),
                                target_uri: session.remote_uri.clone().unwrap_or_default(),
                                evidence:
                                    crate::api::events::TransferTargetEvidence::LocalTargetLeg {
                                        call_id: session.session_id.clone(),
                                    },
                            },
                            Event::ReferCompleted {
                                call_id: transferor,
                                target: session.remote_uri.clone().unwrap_or_default(),
                                status_code: 200,
                                reason: "OK".to_string(),
                            },
                        ],
                    }),
                ));
            }
            // Intentional conditional projection for an ordinary call; an
            // inconsistent transfer marker cannot reach this point.
        }

        Action::SendTransferNotifyFailure => {
            if let Some((transferor, transferor_handle)) = exact_transferor_link(session)? {
                let status_code = match triggering_event {
                    EventType::Dialog4xxFailure(code)
                    | EventType::Dialog5xxFailure(code)
                    | EventType::Dialog6xxFailure(code) => *code,
                    EventType::DialogTimeout => 408,
                    _ => 500,
                };
                let reason = "Transfer leg failed".to_string();
                debug!(
                    "SendTransferNotifyFailure: leg {} -> transferor {} ({} {})",
                    session.session_id, transferor, status_code, reason
                );
                return Ok(ActionOutcome::with_deferred_effect(
                    DeferredActionEffect::TransferNotify(TransferNotifyEffect {
                        transferor: transferor_handle,
                        status_code,
                        reason: reason.clone(),
                        observations: vec![
                            Event::ReferNotify {
                                call_id: transferor.clone(),
                                status_code,
                                reason: reason.clone(),
                                subscription_state: None,
                                body: Some(format!("SIP/2.0 {} {}\r\n", status_code, reason)),
                            },
                            Event::TransferFailed {
                                call_id: transferor,
                                reason,
                                status_code,
                            },
                        ],
                    }),
                ));
            }
            // Intentional conditional projection for an ordinary call; an
            // inconsistent transfer marker cannot reach this point.
        }

        // ──────────────────────────────────────────────────────────────
        // SIP_API_DESIGN_2 §7.1 / §7.3 — Unified outbound dispatch
        // through the option stash.
        //
        // Each handler reads `session.pending_<method>_options` with
        // `.take()`, so the stash is consumed-on-dispatch. This
        // matches the Phase 2 lifecycle: builder `.send()` stages the
        // slot (with the §7.3 invariant #5 conflict guard), the matching
        // `EventType::SendOutbound<METHOD>` queues, and the action claims the
        // exact staged Arc before its first wire await. One-shot requests
        // consume the slot during that claim. INVITE, REGISTER, and BYE retain
        // the pointer-exact immutable snapshot until their retry/final-response
        // owner clears it.
        //
        // §7.4 precedence (stash wins over auto-emit) on BYE / NOTIFY /
        // CANCEL lives in the auto-emit handlers above
        // (`Action::SendBYE`, `Action::SendCANCEL`, `Action::SendNOTIFY`).
        // ──────────────────────────────────────────────────────────────
        // SIP_API_DESIGN_2 §7.3 — R2: exact snapshot ownership transfer.
        Action::SendBYEWithOptions => {
            let Some(PendingOptionsSlot::Bye(options)) = claim_builder_request_staging(
                session,
                PendingOptionsSlotKind::Bye,
                BuilderStageLifetime::RetainedForRetry,
                stage_claim,
            )?
            else {
                return Err(format!(
                    "SendBYEWithOptions: no pending_bye_options for session {}",
                    session.session_id
                )
                .into());
            };
            let snapshot = (*options).clone();
            if let Err(error) = dialog_adapter
                .send_bye_with_options_lane_owned(session, snapshot)
                .await
            {
                // No exact transaction exists to drive terminal cleanup when
                // dispatch itself fails. Release the builder slot immediately.
                PendingOptionsSlot::Bye(options).clear_if_exact(session);
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
            if stage_claim.is_some() || session.pending_cancel_options.is_some() {
                let Some(PendingOptionsSlot::Cancel(options)) = claim_builder_request_staging(
                    session,
                    PendingOptionsSlotKind::Cancel,
                    BuilderStageLifetime::ConsumeBeforeWire,
                    stage_claim,
                )?
                else {
                    return Err(crate::errors::SessionError::InvalidTransition(
                        "SendCANCELWithOptions requires exact staged options".to_string(),
                    )
                    .into());
                };
                dialog_adapter
                    .send_cancel_with_options_lane_owned(session, (*options).clone())
                    .await?;
            } else {
                let auto_extras = dialog_adapter.auto_emit_extra_headers.clone();
                if auto_extras.is_empty() {
                    dialog_adapter
                        .send_cancel_with_options_lane_owned(
                            session,
                            rvoip_sip_dialog::api::unified::CancelRequestOptions::default(),
                        )
                        .await?;
                } else {
                    let opts = rvoip_sip_dialog::api::unified::CancelRequestOptions {
                        reason: None,
                        extra_headers: auto_extras,
                    };
                    dialog_adapter
                        .send_cancel_with_options_lane_owned(session, opts)
                        .await?;
                }
            }
        }
        Action::SendREFERWithOptions => {
            let TrackedInDialogOptions::Refer(options) =
                claim_tracked_request_staging(session, TrackedInDialogMethod::Refer, stage_claim)?
            else {
                return Err(crate::errors::SessionError::InvalidTransition(
                    "SendREFERWithOptions claimed the wrong method".to_string(),
                )
                .into());
            };
            let lease = dialog_adapter.outbound_request_tracker.prepare(
                exact_request_tracker_handle(session)?,
                TrackedInDialogOptions::Refer(Arc::clone(&options)),
            )?;
            let transaction_id = dialog_adapter
                .send_refer_with_options_lane_owned(session, (*options).clone())
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
                    TrackedInDialogMethod::Notify,
                    stage_claim,
                )?
                else {
                    return Err(crate::errors::SessionError::InvalidTransition(
                        "SendNOTIFYWithOptions claimed the wrong method".to_string(),
                    )
                    .into());
                };
                let lease = dialog_adapter.outbound_request_tracker.prepare(
                    exact_request_tracker_handle(session)?,
                    TrackedInDialogOptions::Notify(Arc::clone(&options)),
                )?;
                let transaction_id = dialog_adapter
                    .send_notify_with_options_lane_owned(session, (*options).clone())
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
                        .send_notify_with_options_lane_owned(
                            session,
                            rvoip_sip_dialog::api::unified::NotifyRequestOptions {
                                event: event_package.to_string(),
                                subscription_state: String::new(),
                                content_type: None,
                                body: body.map(bytes::Bytes::from),
                                subscription_id: None,
                                extra_headers: Vec::new(),
                            },
                        )
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
                        .send_notify_with_options_lane_owned(session, opts)
                        .await?;
                }
            }
        }
        Action::SendINFOWithOptions => {
            let TrackedInDialogOptions::Info(options) =
                claim_tracked_request_staging(session, TrackedInDialogMethod::Info, stage_claim)?
            else {
                return Err(crate::errors::SessionError::InvalidTransition(
                    "SendINFOWithOptions claimed the wrong method".to_string(),
                )
                .into());
            };
            let lease = dialog_adapter.outbound_request_tracker.prepare(
                exact_request_tracker_handle(session)?,
                TrackedInDialogOptions::Info(Arc::clone(&options)),
            )?;
            let transaction_id = dialog_adapter
                .send_info_with_options_lane_owned(session, (*options).clone())
                .await?;
            dialog_adapter
                .outbound_request_tracker
                .activate(lease, transaction_id)?;
        }
        Action::SendUPDATEWithOptions => {
            let TrackedInDialogOptions::Update(options) =
                claim_tracked_request_staging(session, TrackedInDialogMethod::Update, stage_claim)?
            else {
                return Err(crate::errors::SessionError::InvalidTransition(
                    "SendUPDATEWithOptions claimed the wrong method".to_string(),
                )
                .into());
            };
            let lease = dialog_adapter.outbound_request_tracker.prepare(
                exact_request_tracker_handle(session)?,
                TrackedInDialogOptions::Update(Arc::clone(&options)),
            )?;
            let transaction_id = match dialog_adapter
                .send_update_with_options_lane_owned(session, (*options).clone())
                .await
            {
                Ok(transaction_id) => transaction_id,
                Err(_) if options.session_timer_refresh => {
                    return Ok(session_refresh_immediate_effect(
                        session,
                        SessionRefreshDeadlineKind::UpdateFailed,
                    ));
                }
                Err(error) => return Err(error.into()),
            };
            dialog_adapter
                .outbound_request_tracker
                .activate(lease, transaction_id)?;
            if options.session_timer_refresh {
                return Ok(session_refresh_transaction_deadline_effect(
                    session,
                    dialog_adapter
                        .non_invite_transaction_timeout()
                        .saturating_add(std::time::Duration::from_secs(2)),
                    SessionRefreshDeadlineKind::UpdateFailed,
                ));
            }
        }
        Action::SendReINVITEWithOptions => {
            let TrackedInDialogOptions::Reinvite(options) = claim_tracked_request_staging(
                session,
                TrackedInDialogMethod::Reinvite,
                stage_claim,
            )?
            else {
                return Err(crate::errors::SessionError::InvalidTransition(
                    "SendReINVITEWithOptions claimed the wrong method".to_string(),
                )
                .into());
            };
            let lease = dialog_adapter.outbound_request_tracker.prepare(
                exact_request_tracker_handle(session)?,
                TrackedInDialogOptions::Reinvite(Arc::clone(&options)),
            )?;
            let snapshot = (*options).clone();
            // RFC 3261 §14.1 — track the in-flight builder-API
            // re-INVITE so `HasPendingReinvite` fires the UAS-side glare path.
            let sdp_snapshot = snapshot.sdp.clone().unwrap_or_default();
            session.pending_reinvite = Some(
                crate::session_store::state::PendingReinvite::SdpUpdate(sdp_snapshot),
            );
            let transaction_id = match dialog_adapter
                .send_reinvite_with_options_lane_owned(session, snapshot)
                .await
            {
                Ok(transaction_id) => transaction_id,
                Err(_) if options.session_timer_refresh => {
                    return Ok(session_refresh_immediate_effect(
                        session,
                        SessionRefreshDeadlineKind::ReinviteFailed,
                    ));
                }
                Err(error) => return Err(error.into()),
            };
            dialog_adapter
                .outbound_request_tracker
                .activate(lease, transaction_id)?;
            if options.session_timer_refresh {
                return Ok(session_refresh_transaction_deadline_effect(
                    session,
                    dialog_adapter
                        .non_invite_transaction_timeout()
                        .saturating_add(std::time::Duration::from_secs(2)),
                    SessionRefreshDeadlineKind::ReinviteFailed,
                ));
            }
        }
        Action::SendMESSAGEWithOptions => {
            if let Some(PendingOptionsSlot::Message(options)) = claim_builder_request_staging(
                session,
                PendingOptionsSlotKind::Message,
                BuilderStageLifetime::ConsumeBeforeWire,
                stage_claim,
            )? {
                dialog_adapter
                    .send_message_oob_with_options((*options).clone())
                    .await?;
            }
        }
        Action::SendOPTIONSWithOptions => {
            if let Some(PendingOptionsSlot::Options(options)) = claim_builder_request_staging(
                session,
                PendingOptionsSlotKind::Options,
                BuilderStageLifetime::ConsumeBeforeWire,
                stage_claim,
            )? {
                dialog_adapter
                    .send_options_oob_with_options((*options).clone())
                    .await?;
            }
        }
        Action::SendSUBSCRIBEWithOptions => {
            if let Some(PendingOptionsSlot::Subscribe(options)) = claim_builder_request_staging(
                session,
                PendingOptionsSlotKind::Subscribe,
                BuilderStageLifetime::ConsumeBeforeWire,
                stage_claim,
            )? {
                // Out-of-dialog SUBSCRIBE uses the target as the
                // request URI; falls back to the session's remote
                // URI for in-dialog refresh.
                let target = session.remote_uri.clone().unwrap_or_default();
                dialog_adapter
                    .send_subscribe_oob_with_options(&target, (*options).clone())
                    .await?;
            }
        }
        Action::SendREGISTERWithOptions => {
            // Canonical REGISTER lifecycle action. A builder snapshot is used
            // when present; initial registration, automatic refresh, and
            // unregister synthesize the same options type from lane-owned
            // state. Compatibility action variants above delegate to this
            // exact implementation as well.
            return execute_register_action(
                session,
                dialog_adapter,
                triggering_event,
                if matches!(triggering_event, EventType::StartUnregistration) {
                    RegisterActionMode::Unregister
                } else {
                    RegisterActionMode::Register
                },
                stage_claim,
            )
            .await;
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
            if let Some(PendingOptionsSlot::Invite(options)) = claim_builder_request_staging(
                session,
                PendingOptionsSlotKind::Invite,
                BuilderStageLifetime::RetainedForRetry,
                stage_claim,
            )? {
                let snapshot = (*options).clone();
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
                session.dialog_id = Some(dialog_adapter.initial_invite_dialog_lane_owned(session)?);
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

/// SIP_API_DESIGN_2 R2 — resolve the SIP method for a non-initial-INVITE/
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
    if session.pending_reinvite_options.is_some() {
        return "INVITE".to_string();
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
        "INVITE" => rvoip_sip_core::Method::Invite,
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
/// digest computation. In-dialog methods (re-INVITE, BYE, REFER, NOTIFY,
/// INFO, UPDATE) target `session.remote_uri`. OOB methods (MESSAGE,
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

#[cfg(not(test))]
async fn scripted_exact_sip_response(
    _session_id: &SessionId,
    _transaction_id: &rvoip_sip_dialog::transaction::TransactionKey,
    _code: u16,
) -> Option<ExactSipResponseResult> {
    None
}

#[cfg(test)]
async fn scripted_exact_sip_response(
    session_id: &SessionId,
    transaction_id: &rvoip_sip_dialog::transaction::TransactionKey,
    code: u16,
) -> Option<ExactSipResponseResult> {
    exact_response_dispatch_test_hook::dispatch(session_id, transaction_id, code).await
}

#[cfg(test)]
pub(crate) mod exact_response_dispatch_test_hook {
    use super::{ExactSipResponseActionError, ExactSipResponseResult};
    use crate::state_table::types::SessionId;
    use rvoip_sip_dialog::transaction::TransactionKey;
    use rvoip_sip_dialog::FinalResponseCompletionDisposition;
    use std::collections::{HashMap, VecDeque};
    use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
    use std::sync::{Arc, LazyLock, Mutex};
    use tokio::sync::Notify;

    static SCRIPTS: LazyLock<Mutex<HashMap<String, Arc<Script>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    #[derive(Clone)]
    pub(crate) enum Step {
        ZeroWire,
        Written,
        WireUnknown,
        Owned(Arc<OwnedCompletion>),
    }

    pub(crate) struct Script {
        steps: Mutex<VecDeque<Step>>,
        attempts: AtomicUsize,
        wire_authorships: AtomicUsize,
        dispatches: Mutex<Vec<(TransactionKey, u16)>>,
    }

    impl Script {
        pub(crate) fn attempts(&self) -> usize {
            self.attempts.load(Ordering::Acquire)
        }

        pub(crate) fn wire_authorships(&self) -> usize {
            self.wire_authorships.load(Ordering::Acquire)
        }

        pub(crate) fn dispatches(&self) -> Vec<(TransactionKey, u16)> {
            self.dispatches
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }
    }

    pub(crate) struct OwnedCompletion {
        authored: AtomicBool,
        entered: AtomicBool,
        entered_notify: Notify,
        outcome: AtomicU8,
        outcome_notify: Notify,
    }

    impl OwnedCompletion {
        pub(crate) fn new() -> Arc<Self> {
            Arc::new(Self {
                authored: AtomicBool::new(false),
                entered: AtomicBool::new(false),
                entered_notify: Notify::new(),
                outcome: AtomicU8::new(0),
                outcome_notify: Notify::new(),
            })
        }

        pub(crate) async fn wait_until_entered(&self) {
            loop {
                if self.entered.load(Ordering::Acquire) {
                    return;
                }
                let notified = self.entered_notify.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if self.entered.load(Ordering::Acquire) {
                    return;
                }
                notified.await;
            }
        }

        pub(crate) fn complete_written(&self) {
            self.outcome.store(1, Ordering::Release);
            self.outcome_notify.notify_waiters();
        }

        async fn wait(&self) -> ExactSipResponseResult {
            loop {
                let notified = self.outcome_notify.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                match self.outcome.load(Ordering::Acquire) {
                    0 => notified.await,
                    1 => {
                        return Ok(FinalResponseCompletionDisposition::WrittenSuccessTerminal);
                    }
                    2 => {
                        return Err(classified_error(
                            FinalResponseCompletionDisposition::ZeroWireRetryable,
                            "scripted owned response failed before transport write",
                        ))
                    }
                    3 => {
                        return Err(classified_error(
                            FinalResponseCompletionDisposition::WireUnknownErrorTerminal,
                            "scripted owned response crossed an unknown transport boundary",
                        ))
                    }
                    _ => unreachable!("invalid scripted exact-response outcome"),
                }
            }
        }
    }

    pub(crate) fn install(session_id: &SessionId, steps: Vec<Step>) -> Arc<Script> {
        let script = Arc::new(Script {
            steps: Mutex::new(steps.into()),
            attempts: AtomicUsize::new(0),
            wire_authorships: AtomicUsize::new(0),
            dispatches: Mutex::new(Vec::new()),
        });
        SCRIPTS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(session_id.0.clone(), Arc::clone(&script));
        script
    }

    pub(crate) fn remove(session_id: &SessionId) {
        SCRIPTS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&session_id.0);
    }

    fn classified_error(
        disposition: FinalResponseCompletionDisposition,
        detail: &str,
    ) -> Box<dyn std::error::Error + Send + Sync> {
        Box::new(ExactSipResponseActionError::new(
            disposition,
            crate::errors::SessionError::DialogError(detail.to_string()),
        ))
    }

    pub(super) async fn dispatch(
        session_id: &SessionId,
        transaction_id: &TransactionKey,
        code: u16,
    ) -> Option<ExactSipResponseResult> {
        let script = SCRIPTS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&session_id.0)
            .cloned()?;
        script.attempts.fetch_add(1, Ordering::AcqRel);
        script
            .dispatches
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((transaction_id.clone(), code));
        let step = script
            .steps
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop_front();
        let Some(step) = step else {
            return Some(Err(classified_error(
                FinalResponseCompletionDisposition::ZeroWireRetryable,
                "scripted exact-response sequence was exhausted",
            )));
        };
        Some(match step {
            Step::ZeroWire => Err(classified_error(
                FinalResponseCompletionDisposition::ZeroWireRetryable,
                "scripted exact response failed before transport write",
            )),
            Step::Written => {
                script.wire_authorships.fetch_add(1, Ordering::AcqRel);
                Ok(FinalResponseCompletionDisposition::WrittenSuccessTerminal)
            }
            Step::WireUnknown => {
                script.wire_authorships.fetch_add(1, Ordering::AcqRel);
                Err(classified_error(
                    FinalResponseCompletionDisposition::WireUnknownErrorTerminal,
                    "scripted exact response crossed an unknown transport boundary",
                ))
            }
            Step::Owned(operation) => {
                if !operation.authored.swap(true, Ordering::AcqRel) {
                    script.wire_authorships.fetch_add(1, Ordering::AcqRel);
                    operation.entered.store(true, Ordering::Release);
                    operation.entered_notify.notify_waiters();
                }
                operation.wait().await
            }
        })
    }
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
mod registration_lane_state_tests {
    use super::*;
    use crate::state_table::Role;

    #[test]
    fn fast_401_updates_only_the_lane_owned_retry_state() {
        let mut session = SessionState::new(SessionId::new(), Role::UAC);
        session.local_sdp = Some("v=0\r\n".into());

        record_registration_auth_retry(&mut session);

        assert_eq!(session.registration_retry_count, 1);
        assert_eq!(session.local_sdp.as_deref(), Some("v=0\r\n"));
    }

    #[test]
    fn fast_423_updates_expiry_and_retry_in_one_working_state() {
        let mut session = SessionState::new(SessionId::new(), Role::UAC);
        session.registration_expires = Some(60);

        record_registration_interval_retry(&mut session, 300);

        assert_eq!(session.registration_expires, Some(300));
        assert_eq!(session.registration_retry_count, 1);
    }

    #[test]
    fn registration_actions_have_no_store_reconciliation_path() {
        let source = include_str!("actions.rs");
        let removed_projection = ["RegistrationState", "Projection"].concat();
        let removed_sync = ["sync_registration", "_state"].concat();
        assert!(!source.contains(&removed_projection));
        assert!(!source.contains(&removed_sync));

        let register_action = source
            .split("async fn execute_register_action")
            .nth(1)
            .and_then(|tail| tail.split("/// Materialize the per-call INVITE").next())
            .expect("execute_register_action source");
        assert!(!register_action.contains("with_session("));
        assert!(!register_action.contains("update_session_with("));

        let options_action = source
            .split("Action::SendREGISTERWithOptions =>")
            .nth(1)
            .and_then(|tail| tail.split("Action::SendINVITEWithOptions =>").next())
            .expect("SendREGISTERWithOptions source");
        assert!(!options_action.contains("with_session("));
        assert!(!options_action.contains("update_session_with("));

        let production = source
            .split("#[cfg(test)]\nmod")
            .next()
            .expect("production source");
        assert_eq!(
            production.matches(".send_register_attempt(").count(),
            1,
            "all REGISTER action variants must share one wire implementation"
        );
        assert!(register_action.contains("claim_builder_request_staging("));
        for action in [
            "Action::SendREGISTER =>",
            "Action::SendREGISTERWithAuth =>",
            "Action::SendUnREGISTER =>",
            "Action::SendREGISTERWithOptions =>",
        ] {
            let body = production
                .split(action)
                .nth(1)
                .unwrap_or_else(|| panic!("missing {action}"));
            assert!(
                body.contains("execute_register_action("),
                "{action} bypasses the canonical REGISTER action"
            );
        }
    }
}

#[cfg(test)]
mod lane_owned_action_state_tests {
    use super::{retire_lane_owned_dialog_identity, retire_lane_owned_media_identity};

    #[test]
    fn dialog_cleanup_retires_the_working_identity_before_redirect_dispatch() {
        let mut session = crate::session_store::SessionState::new(
            crate::state_table::SessionId::new(),
            crate::state_table::Role::UAC,
        );
        session.dialog_id = Some(crate::types::DialogId::new());
        session.dialog_established = true;

        retire_lane_owned_dialog_identity(&mut session);

        assert!(session.dialog_id.is_none());
        assert!(!session.dialog_established);

        let source = include_str!("actions.rs");
        let cleanup = source
            .split("Action::CleanupDialog =>")
            .nth(1)
            .and_then(|tail| tail.split("Action::CleanupMedia =>").next())
            .expect("CleanupDialog action source");
        assert!(cleanup.contains("cleanup_dialog_on_fresh_task"));
        assert!(cleanup.contains("retire_lane_owned_dialog_identity(session)"));
        assert!(
            cleanup.find("cleanup_dialog_on_fresh_task")
                < cleanup.find("retire_lane_owned_dialog_identity(session)"),
            "the old lane-owned identity must retire only after lower cleanup succeeds"
        );
        let production = source
            .split("#[cfg(test)]\nmod")
            .next()
            .expect("production action source");
        assert_eq!(
            production
                .matches("retire_lane_owned_dialog_identity(session);")
                .count(),
            4,
            "every successful dialog cleanup action must retire its lane-owned identity"
        );
    }

    #[test]
    fn media_cleanup_retires_every_lane_owned_media_field() {
        let mut session = crate::session_store::SessionState::new(
            crate::state_table::SessionId::new(),
            crate::state_table::Role::UAC,
        );
        session.media_session_id = Some(crate::types::MediaSessionId::new("lane-media"));
        session.media_session_ready = true;
        session.sdp_negotiated = true;
        session.local_sdp = Some("v=0\r\n".to_string());
        session.negotiated_config = Some(crate::session_store::state::NegotiatedConfig {
            local_addr: "127.0.0.1:16000".parse().expect("local address"),
            remote_addr: "127.0.0.1:16002".parse().expect("remote address"),
            codec: "PCMU".to_string(),
            payload_type: 0,
            sample_rate: 8_000,
            channels: 1,
        });

        retire_lane_owned_media_identity(&mut session);

        assert!(session.media_session_id.is_none());
        assert!(!session.media_session_ready);
        assert!(!session.sdp_negotiated);
        assert!(session.local_sdp.is_none());
        assert!(session.negotiated_config.is_none());
    }

    #[test]
    fn every_state_machine_media_cleanup_uses_the_lane_owned_path() {
        let source = include_str!("actions.rs");
        let production = source
            .split("#[cfg(test)]\nmod")
            .next()
            .expect("production action source");

        assert_eq!(
            production
                .matches("cleanup_lane_owned_media(session, media_adapter)")
                .count(),
            4,
            "every media cleanup action must share the lane-owned implementation"
        );
        assert!(
            !production.contains("media_adapter.cleanup_session(&session.session_id)"),
            "state-machine actions must not enter retained-store media cleanup"
        );

        let helper = production
            .split("async fn cleanup_lane_owned_media")
            .nth(1)
            .and_then(|tail| tail.split("fn negotiated_audio_shape").next())
            .expect("lane-owned media cleanup helper");
        assert!(helper.contains("cleanup_session_lane_owned(session)"));
        assert!(helper.contains("retire_lane_owned_media_identity(session)"));
        assert!(!helper.contains("update_session"));
        assert!(!helper.contains("clear_media_session_retained_exact"));
    }

    #[test]
    fn yaml_uas_responses_have_no_session_or_dialog_transaction_fallback() {
        let source = include_str!("actions.rs");
        let production = source
            .split("#[cfg(test)]\nmod")
            .next()
            .expect("production action source");
        assert!(!production.contains("send_sip_response_on_fresh_task"));
        assert!(!production.contains(".send_response_with_options(&session_id"));
        assert!(!production.contains(".send_response(&session_id"));

        let response_action = production
            .split("Action::SendSIPResponse(code, _reason) =>")
            .nth(1)
            .and_then(|tail| tail.split("Action::SendINVITE =>").next())
            .expect("SendSIPResponse action source");
        assert!(response_action.contains("send_exact_initial_invite_provisional_response"));
        assert!(response_action.contains("send_exact_inbound_provisional_response"));
        assert!(response_action.contains("send_exact_initial_invite_final_response"));
        assert!(response_action.contains("send_exact_inbound_final_response"));
        assert!(!response_action.contains("dialog_adapter.send_response"));
    }

    #[test]
    fn unsupported_actions_fail_closed_and_cleanup_has_one_authority() {
        let source = include_str!("actions.rs");
        let production = source
            .split("#[cfg(test)]\nmod")
            .next()
            .expect("production action source");

        for retired_success_log in [
            "restore_direct_media not implemented yet",
            "attempt_recovery not implemented yet",
            "BridgeToMixer not implemented yet",
            "CleanupResources not implemented yet",
            "// Other custom actions",
        ] {
            assert!(
                !production.contains(retired_success_log),
                "retired false-success path remains: {retired_success_log}"
            );
        }

        for action in [
            "Action::ConnectToMixer =>",
            "Action::DisconnectFromMixer =>",
            "Action::AttemptMediaRecovery =>",
            "Action::BridgeToMixer =>",
        ] {
            let body = production
                .split(action)
                .nth(1)
                .unwrap_or_else(|| panic!("missing {action}"));
            assert!(
                body.contains("return Err("),
                "{action} must never report success without a real implementation"
            );
        }

        for (action, next) in [
            ("Action::RedirectToMixer =>", "Action::ConnectToMixer =>"),
            ("Action::MuteToMixer =>", "Action::UnmuteToMixer =>"),
            ("Action::UnmuteToMixer =>", "Action::DestroyMixer =>"),
            ("Action::DestroyMixer =>", "Action::UpdateMediaDirection"),
            (
                "Action::StartRecordingMixer =>",
                "Action::StopRecordingMixer =>",
            ),
            (
                "Action::StopRecordingMixer =>",
                "Action::ReleaseAllResources =>",
            ),
        ] {
            let body = production
                .split(action)
                .nth(1)
                .and_then(|tail| tail.split(next).next())
                .unwrap_or_else(|| panic!("missing {action}"));
            assert!(
                body.contains("SessionError::InvalidTransition"),
                "{action} must fail closed when its optional mixer/media authority is absent"
            );
        }

        assert_eq!(
            production
                .matches("release_lane_owned_resources(session, dialog_adapter, media_adapter)")
                .count(),
            2,
            "ReleaseAllResources and CleanupResources must share one implementation"
        );
    }

    #[test]
    fn auth_actions_have_no_direct_store_writer() {
        let source = include_str!("actions.rs");
        let production = source
            .split("#[cfg(test)]\nmod")
            .next()
            .expect("production action source");
        assert_eq!(
            production.matches(".update_session_with(").count(),
            0,
            "all actions must mutate only the lane-owned SessionState"
        );

        let auth_owner = production
            .split("fn advance_tracked_auth_owner")
            .nth(1)
            .and_then(|tail| tail.split("pub(crate) async fn execute_action").next())
            .expect("tracked auth owner source");
        assert!(!auth_owner.contains("update_session"));

        let auth_challenge = production
            .split("Action::StoreAuthChallenge =>")
            .nth(1)
            .and_then(|tail| tail.split("Action::SendINVITEWithAuth =>").next())
            .expect("StoreAuthChallenge source");
        assert!(!auth_challenge.contains("update_session"));
        assert!(!auth_challenge.contains("legacy REGISTER shortcut"));
        assert!(!auth_challenge.contains("auth_challenge.is_some()"));
    }

    #[test]
    fn every_builder_action_claims_its_exact_stage() {
        let source = include_str!("actions.rs");
        let production = source
            .split("#[cfg(test)]\nmod")
            .next()
            .expect("production action source");
        let executor = production
            .split("pub(crate) async fn execute_action")
            .nth(1)
            .expect("action executor source");

        for (action, next, claim_helper) in [
            (
                "Action::SendBYEWithOptions => {",
                "Action::SendCANCELWithOptions => {",
                "claim_builder_request_staging(",
            ),
            (
                "Action::SendCANCELWithOptions => {",
                "Action::SendREFERWithOptions => {",
                "claim_builder_request_staging(",
            ),
            (
                "Action::SendREFERWithOptions => {",
                "Action::SendNOTIFYWithOptions => {",
                "claim_tracked_request_staging(",
            ),
            (
                "Action::SendNOTIFYWithOptions => {",
                "Action::SendINFOWithOptions => {",
                "claim_tracked_request_staging(",
            ),
            (
                "Action::SendINFOWithOptions => {",
                "Action::SendUPDATEWithOptions => {",
                "claim_tracked_request_staging(",
            ),
            (
                "Action::SendUPDATEWithOptions => {",
                "Action::SendReINVITEWithOptions => {",
                "claim_tracked_request_staging(",
            ),
            (
                "Action::SendReINVITEWithOptions => {",
                "Action::SendMESSAGEWithOptions => {",
                "claim_tracked_request_staging(",
            ),
            (
                "Action::SendMESSAGEWithOptions => {",
                "Action::SendOPTIONSWithOptions => {",
                "claim_builder_request_staging(",
            ),
            (
                "Action::SendOPTIONSWithOptions => {",
                "Action::SendSUBSCRIBEWithOptions => {",
                "claim_builder_request_staging(",
            ),
            (
                "Action::SendSUBSCRIBEWithOptions => {",
                "Action::SendREGISTERWithOptions => {",
                "claim_builder_request_staging(",
            ),
            (
                "Action::SendREGISTERWithOptions => {",
                "Action::SendINVITEWithOptions => {",
                "execute_register_action(",
            ),
            (
                "Action::SendINVITEWithOptions => {",
                "Action::ClearPendingINVITEOptions => {",
                "claim_builder_request_staging(",
            ),
        ] {
            let action_source = executor
                .split(action)
                .nth(1)
                .and_then(|tail| tail.split(next).next())
                .unwrap_or_else(|| panic!("missing builder action source for {action}"));
            assert!(
                action_source.contains(claim_helper),
                "{action} bypasses the exact staged-options claim"
            );
        }
    }

    #[test]
    fn transfer_actions_defer_one_exact_notify_with_the_complete_observation_set() {
        let source = include_str!("actions.rs");
        let production = source
            .split("#[cfg(test)]\nmod")
            .next()
            .expect("production action source");
        assert!(!production.contains("fn publish_transfer_event"));

        for (action, next, observations) in [
            (
                "Action::SendTransferNotifyRinging =>",
                "Action::SendTransferNotifySuccess =>",
                2,
            ),
            (
                "Action::SendTransferNotifySuccess =>",
                "Action::SendTransferNotifyFailure =>",
                3,
            ),
            (
                "Action::SendTransferNotifyFailure =>",
                "// ──────────────────────────────────────────────────────────────",
                2,
            ),
        ] {
            let action_source = production
                .split(action)
                .nth(1)
                .and_then(|tail| tail.split(next).next())
                .expect("transfer action source");
            assert!(!action_source.contains(".send_refer_notify("));
            assert_eq!(
                action_source
                    .matches("DeferredActionEffect::TransferNotify")
                    .count(),
                1,
                "{action} must admit one exact transfer operation"
            );
            assert_eq!(
                action_source.matches("Event::").count(),
                observations,
                "{action} changed its post-wire public observation set"
            );
            assert!(!action_source.contains("publish_api_event"));
            assert!(!action_source.contains("publish_observational"));
            assert!(action_source.contains("exact_transferor_link(session)?"));
        }

        let transfer_link = production
            .split("fn exact_transferor_link")
            .nth(1)
            .and_then(|tail| tail.split("enum RegisterActionMode").next())
            .expect("exact transfer-link validator");
        assert!(transfer_link.contains("(None, None) if !session.is_transfer_call => Ok(None)"));
        assert!(transfer_link.contains("SessionError::InvalidTransition"));
        assert!(transfer_link.contains("matching exact transferor lifetime"));

        let trying = production
            .split("Action::SendRefer100Trying =>")
            .nth(1)
            .and_then(|tail| tail.split("Action::SendTransferNotifyRinging =>").next())
            .expect("100 Trying action source");
        assert!(trying.contains("send_refer_notify_lane_owned"));
        assert!(!trying.contains(".send_refer_notify("));
    }

    #[test]
    fn retired_string_selected_response_actions_cannot_reenter_production() {
        let source = include_str!("actions.rs");
        let production = source
            .split("#[cfg(test)]\nmod")
            .next()
            .expect("production action source");

        assert!(!production.contains("\"Send180Ringing\" =>"));
        assert!(!production.contains("\"Send200OK\" =>"));
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
