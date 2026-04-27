use crate::state_table::types::SessionId;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

use crate::{
    adapters::{dialog_adapter::DialogAdapter, media_adapter::MediaAdapter},
    api::events::Event,
    session_store::{SessionState, SessionStore},
    state_table::{Action, Condition},
};

/// Execute an action from the state table
pub async fn execute_action(
    action: &Action,
    session: &mut SessionState,
    dialog_adapter: &Arc<DialogAdapter>,
    media_adapter: &Arc<MediaAdapter>,
    session_store: &Arc<SessionStore>,
    _simple_peer_event_tx: &Option<tokio::sync::mpsc::Sender<Event>>, // Unused - events handled by SessionCrossCrateEventHandler
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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
            dialog_adapter
                .send_response(&session.session_id, status, None)
                .await?;
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
            // INVITE. Today only `P-Asserted-Identity` (RFC 3325 §9.1) lands
            // here — when `SessionState.pai_uri` is set the typed header is
            // constructed and routed through dialog-core's
            // `make_call_with_extra_headers_for_session` entry point.
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
                    return Ok(());
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
            dialog_adapter.send_bye_session(&session.session_id).await?;
        }
        Action::SendCANCEL => {
            dialog_adapter.send_cancel(&session.session_id).await?;
        }

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
            // Send REFER for blind transfer
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
            let from_uri = session
                .local_uri
                .as_deref()
                .ok_or_else(|| "local_uri not set for registration".to_string())?;
            let registrar_uri = session
                .registrar_uri
                .as_deref()
                .or_else(|| session.remote_uri.as_deref())
                .ok_or_else(|| "registrar_uri not set for registration".to_string())?;
            let contact_uri = session
                .registration_contact
                .as_deref()
                .or_else(|| session.local_uri.as_deref())
                .ok_or_else(|| "contact_uri not set for registration".to_string())?;
            let expires = session.registration_expires.unwrap_or(3600);

            // Send REGISTER without authentication (first attempt)
            dialog_adapter
                .send_register(
                    &session.session_id,
                    registrar_uri,
                    from_uri,
                    contact_uri,
                    expires,
                    None, // No credentials on first attempt
                )
                .await?;
            // `send_register` awaits the response and may mutate the store
            // directly (set `is_registered`, bump retry counters, dispatch a
            // recursive AuthRequired). Reload so the executor's final save
            // doesn't overwrite those changes with a stale copy.
            *session = session_store.get_session(&session.session_id).await?;
        }

        Action::SendREGISTERWithAuth => {
            info!(
                "Action::SendREGISTERWithAuth for session {}",
                session.session_id
            );
            let from_uri = session
                .local_uri
                .as_deref()
                .ok_or_else(|| "local_uri not set for registration".to_string())?;
            let registrar_uri = session
                .registrar_uri
                .as_deref()
                .or_else(|| session.remote_uri.as_deref())
                .ok_or_else(|| "registrar_uri not set for registration".to_string())?;
            let contact_uri = session
                .registration_contact
                .as_deref()
                .or_else(|| session.local_uri.as_deref())
                .ok_or_else(|| "contact_uri not set for registration".to_string())?;
            let expires = session.registration_expires.unwrap_or(3600);

            // Send REGISTER with authentication
            dialog_adapter
                .send_register(
                    &session.session_id,
                    registrar_uri,
                    from_uri,
                    contact_uri,
                    expires,
                    session.credentials.as_ref(),
                )
                .await?;
            // Reload — see SendREGISTER above.
            *session = session_store.get_session(&session.session_id).await?;
        }

        Action::SendUnREGISTER => {
            info!("Action::SendUnREGISTER for session {}", session.session_id);
            let from_uri = session
                .local_uri
                .as_deref()
                .ok_or_else(|| "local_uri not set for unregistration".to_string())?;
            let registrar_uri = session
                .registrar_uri
                .as_deref()
                .ok_or_else(|| "registrar_uri not set for unregistration".to_string())?;
            let contact_uri = session
                .registration_contact
                .as_deref()
                .ok_or_else(|| "contact_uri not set for unregistration".to_string())?;

            // Send REGISTER with expires=0 to unregister
            dialog_adapter
                .send_register(
                    &session.session_id,
                    registrar_uri,
                    from_uri,
                    contact_uri,
                    0, // expires=0 means unregister
                    session.credentials.as_ref(),
                )
                .await?;
            // Reload — see SendREGISTER above.
            *session = session_store.get_session(&session.session_id).await?;
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
                return Err(format!(
                    "INVITE auth retry cap ({}) exceeded for session {}",
                    CAP, session.session_id
                )
                .into());
            }
            session.invite_auth_retry_count += 1;

            let challenge = session.auth_challenge.clone().ok_or_else(|| {
                format!(
                    "SendINVITEWithAuth: no auth_challenge on session {}",
                    session.session_id
                )
            })?;
            let creds = session.credentials.clone().ok_or_else(|| {
                format!(
                    "SendINVITEWithAuth: no credentials on session {} — set via StreamPeer::with_credentials",
                    session.session_id
                )
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

            dialog_adapter
                .resend_invite_with_auth(
                    &session.session_id,
                    session.local_sdp.clone(),
                    header_name,
                    header_value,
                )
                .await?;
            info!(
                "Auth-retry INVITE sent for session {} (retry #{}, header {})",
                session.session_id, session.invite_auth_retry_count, header_name
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
        Action::SendNOTIFY => {
            info!("Action::SendNOTIFY for session {}", session.session_id);
            // Get event package from session context (default to presence)
            let event_package = "presence";
            let body = session.local_sdp.clone(); // Use SDP field to store notify body temporarily
            dialog_adapter
                .send_notify(&session.session_id, event_package, body, None)
                .await?;
        }

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
                    Event::TransferProgress {
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
                    Event::TransferCompleted {
                        old_call_id: transferor.clone(),
                        new_call_id: transferor,
                        target: session.remote_uri.clone().unwrap_or_default(),
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
    }

    Ok(())
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
