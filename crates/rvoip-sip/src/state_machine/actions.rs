use crate::adapters::dialog_adapter::RegisterAttemptOutcome;
use crate::state_table::types::{EventType, SessionId};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

use crate::{
    adapters::{dialog_adapter::DialogAdapter, media_adapter::MediaAdapter},
    api::events::Event,
    session_store::{SessionState, SessionStore},
    state_table::{Action, Condition},
};

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
    let credentials = match mode {
        RegisterActionMode::Register => None,
        RegisterActionMode::RegisterWithAuth | RegisterActionMode::Unregister => {
            session.credentials.clone()
        }
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
                credentials.as_ref(),
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
                *session = session_store.get_session(&session_id).await?;
                return Ok(ActionOutcome::with_event(EventType::Registration200OK));
            }
            RegisterAttemptOutcome::Unregistered => {
                if mode == RegisterActionMode::Unregister {
                    dialog_adapter
                        .apply_unregistration_success(&session_id, &registrar_uri)
                        .await?;
                    *session = session_store.get_session(&session_id).await?;
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
                *session = session_store.get_session(&session_id).await?;
                return Ok(ActionOutcome::with_event(EventType::RegistrationFailed(
                    200,
                )));
            }
            RegisterAttemptOutcome::AuthChallenge {
                status_code,
                challenge,
            } => {
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
                    *session = session_store.get_session(&session_id).await?;
                    return Ok(ActionOutcome::with_event(EventType::UnregistrationFailed));
                }

                let retry_count = session_store
                    .get_session(&session_id)
                    .await?
                    .registration_retry_count;
                if retry_count >= 1 {
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
                    *session = session_store.get_session(&session_id).await?;
                    return Ok(ActionOutcome::with_event(EventType::RegistrationFailed(
                        status_code,
                    )));
                }

                let mut latest = session_store.get_session(&session_id).await?;
                latest.registration_retry_count += 1;
                session_store.update_session(latest.clone()).await?;
                *session = latest;
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
                    *session = session_store.get_session(&session_id).await?;
                    return Ok(ActionOutcome::with_event(EventType::UnregistrationFailed));
                }

                let latest = session_store.get_session(&session_id).await?;
                if latest.registration_retry_count >= 2 {
                    tracing::error!(
                        "❌ Registration failed with repeated 423 — giving up (retry count {})",
                        latest.registration_retry_count
                    );
                    dialog_adapter
                        .apply_registration_failure(
                            &session_id,
                            &registrar_uri,
                            423,
                            "Registration failed with repeated 423 Interval Too Brief responses",
                        )
                        .await?;
                    *session = session_store.get_session(&session_id).await?;
                    return Ok(ActionOutcome::with_event(EventType::RegistrationFailed(
                        423,
                    )));
                }

                tracing::info!(
                    "🔄 423 Interval Too Brief — retrying REGISTER with Expires={} (server required min)",
                    min_expires
                );
                let mut latest = latest;
                latest.registration_expires = Some(min_expires);
                latest.registration_retry_count += 1;
                session_store.update_session(latest.clone()).await?;
                *session = latest;
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
                    *session = session_store.get_session(&session_id).await?;
                    return Ok(ActionOutcome::with_event(EventType::UnregistrationFailed));
                }

                dialog_adapter
                    .apply_registration_failure(&session_id, &registrar_uri, status_code, reason)
                    .await?;
                *session = session_store.get_session(&session_id).await?;
                return Ok(ActionOutcome::with_event(EventType::RegistrationFailed(
                    status_code,
                )));
            }
        }
    }
}

/// Execute an action from the state table
pub(crate) async fn execute_action(
    action: &Action,
    session: &mut SessionState,
    dialog_adapter: &Arc<DialogAdapter>,
    media_adapter: &Arc<MediaAdapter>,
    session_store: &Arc<SessionStore>,
    _simple_peer_event_tx: &Option<tokio::sync::mpsc::Sender<Event>>, // Unused - events handled by SessionCrossCrateEventHandler
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
            info!("Creating dialog from {} to {}", from, to);
            // Don't create dialog here - it will be created when we send INVITE
            // Just log that we're preparing to create a dialog
            info!("Dialog will be created when INVITE is sent");
        }
        Action::CreateMediaSession => {
            info!(
                "Action::CreateMediaSession for session {}",
                session.session_id
            );
            let media_id = media_adapter.create_session(&session.session_id).await?;
            session.media_session_id = Some(media_id.clone());
            info!("Created media session ID: {:?}", media_id);
        }
        Action::GenerateLocalSDP => {
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
            session_store.update_session(session.clone()).await?;
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
            dialog_adapter
                .send_response(&session.session_id, *code, session.local_sdp.clone())
                .await?;
            // RFC 3261: Dialog is established when UAS sends 200 OK to INVITE
            if *code == 200 {
                session.dialog_established = true;
                info!(
                    "Dialog established (UAS sent 200 OK) for session {}",
                    session.session_id
                );
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
                "Sending INVITE from {} to {} with SDP: {}",
                from,
                to,
                session.local_sdp.is_some()
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
                    Err(e) => {
                        // Reject upstream rather than silently dropping — the
                        // app set a malformed PAI and would otherwise wonder
                        // why the carrier rejects with 403.
                        return Err(format!(
                            "SessionState.pai_uri ({}) is not a valid URI: {}",
                            pai, e
                        )
                        .into());
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
                session.dialog_id = Some(dialog_id.clone());
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
                "🔀 Following 3xx redirect (attempt {}/{}) from {} to {}",
                session.redirect_attempts, MAX_REDIRECTS, from, next_target
            );

            dialog_adapter
                .send_invite_with_details(
                    &session.session_id,
                    &from,
                    &next_target,
                    session.local_sdp.clone(),
                )
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
            info!("SendACK action: dialog-core handles ACK sending, dialog marked as established for UAC session {}", session.session_id);
        }
        Action::SendBYE => {
            // SIP_API_DESIGN_2 §7.4 — application-staged
            // `pending_bye_options` (from `coord.bye(..).send()`) wins
            // over the auto-emit headers; when the stash is empty we
            // fall back to `Config.auto_emit_extra_headers` (operators
            // use it to stamp tenant identifiers / trace headers onto
            // every teardown the stack initiates).
            let reason = session.pending_bye_reason.take();

            if let Some(opts_arc) = session.pending_bye_options.take() {
                // Stash wins — extras already validated at the
                // DialogAdapter mirror boundary when the builder
                // dispatched.
                let opts = (*opts_arc).clone();
                dialog_adapter
                    .send_bye_with_options(&session.session_id, opts)
                    .await?;
            } else {
                let auto_extras = dialog_adapter.auto_emit_extra_headers.clone();
                if auto_extras.is_empty() {
                    // Preserve the legacy fast path so the existing flat
                    // helpers stay observable as the canonical entries.
                    if let Some((protocol, cause, text)) = reason {
                        let reason =
                            rvoip_sip_core::types::reason::Reason::new(protocol, cause, text);
                        dialog_adapter
                            .send_bye_session_with_reason(&session.session_id, reason)
                            .await?;
                    } else {
                        dialog_adapter.send_bye_session(&session.session_id).await?;
                    }
                } else {
                    let opts = rvoip_sip_dialog::api::unified::ByeRequestOptions {
                        reason: reason.and_then(|(_p, _c, text)| text),
                        extra_headers: auto_extras,
                    };
                    dialog_adapter
                        .send_bye_with_options(&session.session_id, opts)
                        .await?;
                }
            }
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
                let session_config = crate::session_store::state::NegotiatedConfig {
                    local_addr: config.local_addr,
                    remote_addr: config.remote_addr,
                    codec: config.codec,
                    sample_rate: 8000, // Default for PCMU
                    channels: 1,
                };
                session.negotiated_config = Some(session_config);
                session.local_media_direction = config.local_direction;
                session.remote_media_direction = config.remote_direction;
                session.sdp_negotiated = true;
                info!("SDP negotiated as UAC for session {}", session.session_id);
            }
        }
        Action::NegotiateSDPAsUAS => {
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
                let session_config = crate::session_store::state::NegotiatedConfig {
                    local_addr: config.local_addr,
                    remote_addr: config.remote_addr,
                    codec: config.codec,
                    sample_rate: 8000, // Default for PCMU
                    channels: 1,
                };
                session.local_sdp = Some(local_sdp);
                session.negotiated_config = Some(session_config);
                session.local_media_direction = config.local_direction;
                session.remote_media_direction = config.remote_direction;
                session.sdp_negotiated = true;
                info!("SDP negotiated as UAS for session {}", session.session_id);
            }
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
                let session_config = crate::session_store::state::NegotiatedConfig {
                    local_addr: config.local_addr,
                    remote_addr: config.remote_addr,
                    codec: config.codec,
                    sample_rate: 8000,
                    channels: 1,
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
                .update_session(session.clone())
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
            // Parse the challenge payload stashed in session.pending_auth by the
            // executor (for AuthRequired events) and write the parsed
            // `DigestChallenge` into session.auth_challenge. Both INVITE and
            // REGISTER auth paths consume this field.
            //
            // Fallback: the legacy REGISTER shortcut in DialogAdapter may have
            // already populated session.auth_challenge directly (Phase 2 will
            // remove that path). If pending_auth is None and auth_challenge is
            // already set, treat this action as a no-op.
            if let Some((_, ref challenge_str)) = session.pending_auth {
                let parsed = rvoip_auth_core::DigestAuthenticator::parse_challenge(challenge_str)?;
                info!(
                    "Stored auth challenge for session {} (realm={}, nonce={})",
                    session.session_id, parsed.realm, parsed.nonce
                );
                session.auth_challenge = Some(parsed);
                // Persist so the next action — `SendREGISTERWithAuth` or
                // `SendINVITEWithAuth` — sees the challenge when it re-reads
                // the session from the store inside the dialog adapter.
                // Actions share a mutable local `session` but the adapter
                // calls `store.get_session` which reads the persisted copy.
                session_store.update_session(session.clone()).await?;
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
            // RFC 3261 §22.2 — compute a digest Authorization header and
            // re-issue the INVITE on the same dialog (same Call-ID, bumped
            // CSeq) via DialogAdapter::resend_invite_with_auth. Capped at one
            // retry to prevent loops when credentials are wrong.
            info!(
                "Action::SendINVITEWithAuth for session {}",
                session.session_id
            );
            const CAP: u8 = 1;
            if session.invite_auth_retry_count >= CAP {
                return Err(Box::new(
                    crate::errors::SessionError::InviteAuthRetryExhausted,
                ));
            }
            session.invite_auth_retry_count += 1;

            let challenge = session.auth_challenge.clone().ok_or_else(|| {
                format!(
                    "SendINVITEWithAuth: no auth_challenge on session {}",
                    session.session_id
                )
            })?;
            let creds = session.credentials.clone().ok_or_else(|| {
                Box::new(crate::errors::SessionError::MissingCredentialsForInviteAuth)
                    as Box<dyn std::error::Error + Send + Sync>
            })?;
            let request_uri = session.remote_uri.clone().ok_or_else(|| {
                format!(
                    "SendINVITEWithAuth: no remote_uri on session {}",
                    session.session_id
                )
            })?;

            // RFC 7616 §3.4.5 — increment the per-(realm, nonce) NC
            // counter before computing. A carrier reusing the same
            // nonce across multiple requests rejects `nc` repeats as
            // replays, so the counter must monotonically grow.
            let nc_key = (challenge.realm.clone(), challenge.nonce.clone());
            let nc_value = *session
                .digest_nc
                .entry(nc_key)
                .and_modify(|n| *n += 1)
                .or_insert(1);
            // INVITE body is the local SDP offer — fold it into HA2
            // when the server's challenge offers `qop=auth-int`. RFC
            // 7616 §3.4.3.
            let body_owned = session.local_sdp.clone();
            let body_bytes = body_owned.as_deref().map(|s| s.as_bytes());
            let computed = rvoip_auth_core::DigestClient::compute_response_with_state(
                &creds.username,
                &creds.password,
                &challenge,
                "INVITE",
                &request_uri,
                nc_value,
                body_bytes,
            )?;
            let header_value = rvoip_auth_core::DigestClient::format_authorization_with_state(
                &creds.username,
                &challenge,
                &request_uri,
                &computed,
            );

            let (status, _) = session.pending_auth.take().unwrap_or((401, String::new()));
            let header_name = if status == 407 {
                "Proxy-Authorization"
            } else {
                "Authorization"
            };

            // SIP_API_DESIGN_2 §7.3 — pull application extras from the
            // INVITE stash so they ride the 401/407 retry. The snapshot
            // persists across the auth-retry hop because the stash is
            // not consumed by `Action::SendINVITEWithOptions` until the
            // final response (200 / 4xx-after-retry / 5xx / 6xx).
            // The OutboundCallBuilder path fills this; transfer-leg and
            // internal helper paths leave it empty, in which case we
            // send an empty extras vector.
            let auth_extras: Vec<rvoip_sip_core::types::TypedHeader> = session
                .pending_invite_options
                .as_ref()
                .map(|snapshot| snapshot.extra_headers.clone())
                .unwrap_or_default();

            dialog_adapter
                .resend_invite_with_auth(
                    &session.session_id,
                    session.local_sdp.clone(),
                    header_name,
                    header_value,
                    auth_extras,
                )
                .await?;
            info!(
                "Auth-retry INVITE sent for session {} (retry #{}, header {})",
                session.session_id, session.invite_auth_retry_count, header_name
            );
        }

        Action::SendRequestWithAuth => {
            // SIP_API_DESIGN_2 R2 — auth-retry for non-INVITE/non-REGISTER
            // methods. Reads `session.pending_auth_method` to discriminate
            // which `pending_<method>_options` to re-issue (falls back to
            // inspecting which stash is set when method is missing or
            // empty), computes the digest via auth-core, and dispatches
            // via the matching `DialogAdapter::send_<method>_with_auth`.
            info!(
                "Action::SendRequestWithAuth for session {} (method={:?})",
                session.session_id, session.pending_auth_method
            );
            const CAP: u8 = 1;
            if session.request_auth_retry_count >= CAP {
                return Err(format!(
                    "request auth retry cap ({}) exceeded for session {}",
                    CAP, session.session_id
                )
                .into());
            }
            session.request_auth_retry_count += 1;

            let challenge = session.auth_challenge.clone().ok_or_else(|| {
                format!(
                    "SendRequestWithAuth: no auth_challenge on session {}",
                    session.session_id
                )
            })?;
            let creds = session.credentials.clone().ok_or_else(|| {
                Box::new(crate::errors::SessionError::MissingCredentialsForInviteAuth)
                    as Box<dyn std::error::Error + Send + Sync>
            })?;

            // Resolve the method. Prefer the explicit field; fall back
            // to inspecting which stash is set. The conflict guard
            // guarantees at most one non-INVITE/non-REGISTER stash is
            // populated per session.
            let method = resolve_auth_method(session);

            let (status, _) = session.pending_auth.take().unwrap_or((401, String::new()));
            let header_name = if status == 407 {
                "Proxy-Authorization"
            } else {
                "Authorization"
            };

            // Method-specific request URI. In-dialog methods use the
            // remote URI (= the remote target URI per RFC 3261); OOB
            // methods (SUBSCRIBE, MESSAGE, OPTIONS) use the stash's
            // explicit target.
            let request_uri = resolve_auth_request_uri(session, &method).ok_or_else(|| {
                format!(
                    "SendRequestWithAuth: no request_uri for method {} on session {}",
                    method, session.session_id
                )
            })?;

            // RFC 7616 §3.4.5 — per-(realm, nonce) NC counter.
            let nc_key = (challenge.realm.clone(), challenge.nonce.clone());
            let nc_value = *session
                .digest_nc
                .entry(nc_key)
                .and_modify(|n| *n += 1)
                .or_insert(1);

            // Most non-INVITE methods don't carry a body that's folded
            // into HA2 under qop=auth-int. The exceptions are MESSAGE
            // (which has a body) and re-INVITE (handled by the INVITE
            // path). Pull the body from the stash for MESSAGE.
            let body_bytes_owned: Option<Vec<u8>> = match method.as_str() {
                "MESSAGE" => session
                    .pending_message_options
                    .as_ref()
                    .map(|opts| opts.body.to_vec())
                    .filter(|b| !b.is_empty()),
                _ => None,
            };
            let body_bytes_ref = body_bytes_owned.as_deref();

            let computed = rvoip_auth_core::DigestClient::compute_response_with_state(
                &creds.username,
                &creds.password,
                &challenge,
                &method,
                &request_uri,
                nc_value,
                body_bytes_ref,
            )?;
            let header_value = rvoip_auth_core::DigestClient::format_authorization_with_state(
                &creds.username,
                &challenge,
                &request_uri,
                &computed,
            );

            // Dispatch per method. Each branch reads the matching
            // `pending_<method>_options` stash so the application
            // extras / typed parameters ride the retry.
            match method.as_str() {
                "BYE" => {
                    let opts = session
                        .pending_bye_options
                        .as_ref()
                        .map(|a| (**a).clone())
                        .unwrap_or_default();
                    dialog_adapter
                        .send_bye_with_auth(
                            &session.session_id,
                            opts,
                            header_name,
                            header_value,
                        )
                        .await?;
                }
                "REFER" => {
                    let opts = session
                        .pending_refer_options
                        .as_ref()
                        .map(|a| (**a).clone())
                        .ok_or_else(|| {
                            format!(
                                "SendRequestWithAuth(REFER): no pending_refer_options for session {}",
                                session.session_id
                            )
                        })?;
                    dialog_adapter
                        .send_refer_with_auth(
                            &session.session_id,
                            opts,
                            header_name,
                            header_value,
                        )
                        .await?;
                }
                "NOTIFY" => {
                    let opts = session
                        .pending_notify_options
                        .as_ref()
                        .map(|a| (**a).clone())
                        .ok_or_else(|| {
                            format!(
                                "SendRequestWithAuth(NOTIFY): no pending_notify_options for session {}",
                                session.session_id
                            )
                        })?;
                    dialog_adapter
                        .send_notify_with_auth(
                            &session.session_id,
                            opts,
                            header_name,
                            header_value,
                        )
                        .await?;
                }
                "INFO" => {
                    let opts = session
                        .pending_info_options
                        .as_ref()
                        .map(|a| (**a).clone())
                        .ok_or_else(|| {
                            format!(
                                "SendRequestWithAuth(INFO): no pending_info_options for session {}",
                                session.session_id
                            )
                        })?;
                    dialog_adapter
                        .send_info_with_auth(
                            &session.session_id,
                            opts,
                            header_name,
                            header_value,
                        )
                        .await?;
                }
                "UPDATE" => {
                    let opts = session
                        .pending_update_options
                        .as_ref()
                        .map(|a| (**a).clone())
                        .ok_or_else(|| {
                            format!(
                                "SendRequestWithAuth(UPDATE): no pending_update_options for session {}",
                                session.session_id
                            )
                        })?;
                    dialog_adapter
                        .send_update_with_auth(
                            &session.session_id,
                            opts,
                            header_name,
                            header_value,
                        )
                        .await?;
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
                method, session.session_id, session.request_auth_retry_count, header_name
            );
        }

        Action::SendINVITEWithBumpedSessionExpires => {
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
                session.session_id, min_se, min_se,
                session.session_timer_retry_count, CAP
            );

            dialog_adapter
                .resend_invite_with_session_timer_override(
                    &session.session_id,
                    session.local_sdp.clone(),
                    min_se,
                    min_se,
                )
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
            dialog_adapter
                .send_subscribe(
                    &session.session_id,
                    from_uri,
                    to_uri,
                    event_package,
                    expires,
                )
                .await?;
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
            dialog_adapter
                .send_message(&session.session_id, from_uri, to_uri, body, in_dialog)
                .await?;
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
                dialog_adapter.cleanup_session(&session.session_id).await?;
            }
        }
        Action::CleanupMedia => {
            debug!(
                "Cleaning up media for session {} (media_session_id={:?})",
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
            if let Err(e) = dialog_adapter
                .global_coordinator
                .publish(Arc::new(event))
                .await
            {
                error!("Failed to publish ReferResponse event: {}", e);
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
            if let Err(e) = dialog_adapter
                .send_refer_notify(&session.session_id, 100, "Trying")
                .await
            {
                warn!(
                    "Failed to send 100 Trying NOTIFY on session {}: {}",
                    session.session_id, e
                );
            }
        }

        Action::SendTransferNotifyRinging => {
            if let Some(transferor) = session.transferor_session_id.clone() {
                debug!(
                    "SendTransferNotifyRinging: leg {} -> transferor {}",
                    session.session_id, transferor
                );
                if let Err(e) = dialog_adapter
                    .send_refer_notify(&transferor, 180, "Ringing")
                    .await
                {
                    warn!(
                        "Failed to send 180 Ringing NOTIFY to transferor {}: {}",
                        transferor, e
                    );
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
                if let Err(e) = dialog_adapter
                    .send_refer_notify(&transferor, 200, "OK")
                    .await
                {
                    warn!(
                        "Failed to send 200 OK NOTIFY to transferor {}: {}",
                        transferor, e
                    );
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
                if let Err(e) = dialog_adapter
                    .send_refer_notify(&transferor, status_code, &reason)
                    .await
                {
                    warn!(
                        "Failed to send {} {} NOTIFY to transferor {}: {}",
                        status_code, reason, transferor, e
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
            if let Some(opts) = session.pending_bye_options.as_ref() {
                let snapshot = (**opts).clone();
                dialog_adapter
                    .send_bye_with_options(&session.session_id, snapshot)
                    .await?;
                session.pending_bye_options = None;
            }
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
            if let Some(opts) = session.pending_refer_options.as_ref() {
                let snapshot = (**opts).clone();
                dialog_adapter
                    .send_refer_with_options(&session.session_id, snapshot)
                    .await?;
                session.pending_refer_options = None;
            }
        }
        Action::SendNOTIFYWithOptions => {
            // Phase 5 — single NOTIFY action: stash wins; otherwise
            // consult `Config.auto_emit_extra_headers` so operator
            // headers ride every stack-emitted NOTIFY. Consolidated from
            // the deleted `Action::SendNOTIFY`.
            if let Some(opts_arc) = session.pending_notify_options.as_ref() {
                let opts = (**opts_arc).clone();
                dialog_adapter
                    .send_notify_with_options(&session.session_id, opts)
                    .await?;
                session.pending_notify_options = None;
            } else {
                let auto_extras = dialog_adapter.auto_emit_extra_headers.clone();
                let event_package = "presence";
                let body = session.local_sdp.clone();
                if auto_extras.is_empty() {
                    dialog_adapter
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
                    dialog_adapter
                        .send_notify_with_options(&session.session_id, opts)
                        .await?;
                }
            }
        }
        Action::SendINFOWithOptions => {
            if let Some(opts) = session.pending_info_options.as_ref() {
                let snapshot = (**opts).clone();
                dialog_adapter
                    .send_info_with_options(&session.session_id, snapshot)
                    .await?;
                session.pending_info_options = None;
            }
        }
        Action::SendUPDATEWithOptions => {
            if let Some(opts) = session.pending_update_options.as_ref() {
                let snapshot = (**opts).clone();
                dialog_adapter
                    .send_update_with_options(&session.session_id, snapshot)
                    .await?;
                session.pending_update_options = None;
            }
        }
        Action::SendReINVITEWithOptions => {
            if let Some(opts) = session.pending_reinvite_options.as_ref() {
                let snapshot = (**opts).clone();
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
            if let Some(opts) = session.pending_register_options.take() {
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
                let response = dialog_adapter
                    .send_register_with_options(opts)
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
                    session.session_id,
                    response.status_code()
                );
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
                let from = snapshot.from.clone().unwrap_or_else(String::new);

                // Mirror the legacy `CreateDialog` action's PAI plumbing
                // (actions.rs:407) so the builder dispatch path stamps
                // `P-Asserted-Identity` on the wire INVITE when
                // `session.pai_uri` is set (either from the builder's
                // `with_pai(uri)` or via `Config.pai_uri` fall-through
                // resolved at builder-staging time). Without this, the
                // builder path silently drops PAI even though the legacy
                // path honors it.
                let mut extras = snapshot.extra_headers.clone();
                if let Some(pai) = session.pai_uri.as_ref() {
                    use rvoip_sip_core::types::{
                        p_asserted_identity::PAssertedIdentity, uri::Uri, TypedHeader,
                    };
                    use std::str::FromStr;
                    match Uri::from_str(pai) {
                        Ok(uri) => {
                            extras.insert(
                                0,
                                TypedHeader::PAssertedIdentity(
                                    PAssertedIdentity::with_uri(uri),
                                ),
                            );
                        }
                        Err(e) => {
                            return Err(format!(
                                "SendINVITEWithOptions: SessionState.pai_uri \
                                 ({}) is not a valid URI: {}",
                                pai, e
                            )
                            .into());
                        }
                    }
                }

                // SIP_API_DESIGN_2 §6.1 — per-call outbound proxy
                // override. `dialog_adapter.send_invite_with_extra_headers`
                // applies the global `Config.outbound_proxy_uri` via
                // `prepend_outbound_proxy_route`. To honor the builder's
                // `with_outbound_proxy(uri)` override we prepend a
                // `Route:` ahead of any global one; `Suppress` just
                // omits both. The global path will still try to
                // prepend its own Route at the adapter, so on Use we
                // route through the override below and skip the adapter's
                // global default by passing `None` for the adapter call
                // when applicable.
                use crate::api::send::ProxyOverride;
                let route_override = match &snapshot.outbound_proxy_override {
                    ProxyOverride::Use(uri) => Some(uri.clone()),
                    ProxyOverride::Default | ProxyOverride::Suppress => None,
                };
                if let Some(uri_str) = route_override {
                    use rvoip_sip_core::types::{route::Route, uri::Uri, TypedHeader};
                    use std::str::FromStr;
                    match Uri::from_str(&uri_str) {
                        Ok(uri) => {
                            extras
                                .insert(0, TypedHeader::Route(Route::with_uri(uri)));
                        }
                        Err(e) => {
                            return Err(format!(
                                "SendINVITEWithOptions: outbound_proxy override \
                                 ({}) is not a valid URI: {}",
                                uri_str, e
                            )
                            .into());
                        }
                    }
                }
                let suppress_global_proxy = matches!(
                    &snapshot.outbound_proxy_override,
                    ProxyOverride::Suppress | ProxyOverride::Use(_)
                );

                // SIP_API_DESIGN_2 §7.2 — per-call Contact override.
                // The builder's `with_contact_uri(uri)` stages a value into
                // `snapshot.contact_uri`; emit it as a typed `Contact` in
                // extras so dialog-core honors it instead of stamping the
                // default socket-derived Contact. Prepended so it sits
                // ahead of application extras, deterministic on the wire.
                if let Some(contact_uri) = snapshot.contact_uri.as_ref() {
                    use rvoip_sip_core::types::address::Address;
                    use rvoip_sip_core::types::contact::{
                        Contact, ContactParamInfo,
                    };
                    use rvoip_sip_core::types::{uri::Uri, TypedHeader};
                    use std::str::FromStr;
                    match Uri::from_str(contact_uri) {
                        Ok(uri) => {
                            let address = Address::new(uri);
                            let contact = Contact::new_params(vec![
                                ContactParamInfo { address },
                            ]);
                            extras.insert(0, TypedHeader::Contact(contact));
                        }
                        Err(e) => {
                            return Err(format!(
                                "SendINVITEWithOptions: contact_uri override \
                                 ({}) is not a valid URI: {}",
                                contact_uri, e
                            )
                            .into());
                        }
                    }
                }

                // SDP precedence: builder-supplied snapshot.sdp wins;
                // otherwise fall back to `session.local_sdp` populated by
                // the preceding `GenerateLocalSDP` action. Mirrors the
                // legacy `Action::SendINVITE` shape so the new builder
                // path negotiates media identically.
                let sdp_for_wire = snapshot.sdp.clone().or_else(|| session.local_sdp.clone());

                if suppress_global_proxy {
                    dialog_adapter
                        .send_invite_with_extra_headers_no_global_proxy(
                            &session.session_id,
                            &from,
                            &snapshot.to,
                            sdp_for_wire,
                            extras,
                        )
                        .await?;
                } else {
                    dialog_adapter
                        .send_invite_with_extra_headers(
                            &session.session_id,
                            &from,
                            &snapshot.to,
                            sdp_for_wire,
                            extras,
                        )
                        .await?;
                }
                debug!(
                    "SendINVITEWithOptions dispatched for session {}: to={}",
                    session.session_id, snapshot.to
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
            return m.to_ascii_uppercase();
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
        if let Err(e) = coordinator.publish(wrapped).await {
            tracing::warn!("Failed to publish Transfer* event: {}", e);
        }
    });
}
