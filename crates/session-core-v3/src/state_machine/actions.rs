use std::sync::Arc;
use tracing::{info, debug, warn, error};
use crate::state_table::types::SessionId;

use crate::{
    state_table::{Action, Condition},
    session_store::{SessionState, SessionStore},
    adapters::{dialog_adapter::DialogAdapter, media_adapter::MediaAdapter},
    api::events::Event,
};

/// Execute an action from the state table
pub async fn execute_action(
    action: &Action,
    session: &mut SessionState,
    dialog_adapter: &Arc<DialogAdapter>,
    media_adapter: &Arc<MediaAdapter>,
    _session_store: &Arc<SessionStore>,
    simple_peer_event_tx: &Option<tokio::sync::mpsc::Sender<Event>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    debug!("Executing action: {:?}", action);
    
    match action {
        // Dialog actions
        Action::CreateDialog => {
            info!("Action::CreateDialog for session {}", session.session_id);
            let from = session.local_uri.as_deref()
                .ok_or_else(|| "local_uri not set for session".to_string())?;
            let to = session.remote_uri.as_deref()
                .ok_or_else(|| "remote_uri not set for session".to_string())?;
            info!("Creating dialog from {} to {}", from, to);
            // Don't create dialog here - it will be created when we send INVITE
            // Just log that we're preparing to create a dialog
            info!("Dialog will be created when INVITE is sent");
        }
        Action::CreateMediaSession => {
            info!("Action::CreateMediaSession for session {}", session.session_id);
            let media_id = media_adapter.create_session(&session.session_id).await?;
            session.media_session_id = Some(media_id.clone());
            info!("Created media session ID: {:?}", media_id);
        }
        Action::GenerateLocalSDP => {
            info!("Action::GenerateLocalSDP for session {}", session.session_id);
            let sdp = media_adapter.generate_local_sdp(&session.session_id).await?;
            session.local_sdp = Some(sdp.clone());
            info!("Generated SDP with {} bytes", sdp.len());
        }
        Action::SendSIPResponse(code, _reason) => {
            dialog_adapter.send_response(&session.session_id, *code, session.local_sdp.clone()).await?;
            // RFC 3261: Dialog is established when UAS sends 200 OK to INVITE
            if *code == 200 {
                session.dialog_established = true;
                info!("Dialog established (UAS sent 200 OK) for session {}", session.session_id);
            }
        }
        Action::SendINVITE => {
            info!("Action::SendINVITE for session {}", session.session_id);
            // Get session details for send_invite_with_details
            let from = session.local_uri.clone()
                .ok_or_else(|| "local_uri not set for session".to_string())?;
            let to = session.remote_uri.clone()
                .ok_or_else(|| "remote_uri not set for session".to_string())?;
            info!("Sending INVITE from {} to {} with SDP: {}", from, to, session.local_sdp.is_some());
            
            // This will create the real dialog in dialog-core
            dialog_adapter.send_invite_with_details(&session.session_id, &from, &to, session.local_sdp.clone()).await?;
            
            // Now get the real dialog ID that was created
            if let Some(real_dialog_id) = dialog_adapter.session_to_dialog.get(&session.session_id) {
                // Convert RvoipDialogId to our DialogId type
                let dialog_id: crate::types::DialogId = real_dialog_id.value().clone().into();
                session.dialog_id = Some(dialog_id.clone());
                info!("INVITE sent successfully with dialog ID {:?}", dialog_id);
            } else {
                warn!("Failed to get dialog ID after sending INVITE");
                info!("INVITE sent successfully");
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
            // Send re-INVITE with sendonly SDP
            if let Some(hold_sdp) = media_adapter.create_hold_sdp().await.ok() {
                session.local_sdp = Some(hold_sdp.clone());
                dialog_adapter.send_reinvite_session(&session.session_id, hold_sdp).await?;
            }
        }
        Action::ResumeCall => {
            // Send re-INVITE with sendrecv SDP
            if let Some(active_sdp) = media_adapter.create_active_sdp().await.ok() {
                session.local_sdp = Some(active_sdp.clone());
                dialog_adapter.send_reinvite_session(&session.session_id, active_sdp).await?;
            }
        }
        Action::TransferCall(target) => {
            // Send REFER for blind transfer
            dialog_adapter.send_refer_session(&session.session_id, target).await?;
        }
        Action::SendDTMF(digit) => {
            // Send DTMF through media session
            {
                let media_id = crate::types::MediaSessionId::new();
                media_adapter.send_dtmf(media_id, *digit).await?;
            }
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
            info!("Media session started and marked as ready for session {}", session.session_id);
        }
        Action::StopMediaSession => {
            media_adapter.stop_session(&session.session_id).await?;
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
                session.sdp_negotiated = true;
                info!("SDP negotiated as UAC for session {}", session.session_id);
            }
        }
        Action::NegotiateSDPAsUAS => {
            if let Some(remote_sdp) = &session.remote_sdp {
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
                session.sdp_negotiated = true;
                info!("SDP negotiated as UAS for session {}", session.session_id);
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
                info!("Remote SDP stored for session {} ({} bytes)", session.session_id, remote_sdp.len());
                // Parse and log the remote RTP port for debugging
                if let Some(port_match) = remote_sdp.lines()
                    .find(|line| line.starts_with("m=audio"))
                    .and_then(|line| line.split_whitespace().nth(1)) {
                    info!("Remote RTP port: {}", port_match);
                }
            } else {
                warn!("StoreRemoteSDP action called but no remote SDP found for session {}", session.session_id);
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
            debug!("Dialog cleanup completed for session {}", session.session_id);
        }
        Action::StartMediaCleanup => {
            media_adapter.cleanup_session(&session.session_id).await?;
            debug!("Media cleanup completed for session {}", session.session_id);
        }
        
        // New actions for extended functionality
        Action::SendReINVITE => {
            debug!("Sending re-INVITE for session {}", session.session_id);
            
            // Generate SDP based on current state
            let sdp = if session.call_state == crate::types::CallState::Active {
                // Going to hold - use sendonly
                session.local_sdp.as_ref().map(|sdp| {
                    // Modify SDP to include sendonly attribute
                    if sdp.contains("a=sendrecv") {
                        sdp.replace("a=sendrecv", "a=sendonly")
                    } else {
                        format!("{}\na=sendonly\r\n", sdp.trim_end())
                    }
                })
            } else {
                // Resuming from hold - use sendrecv
                session.local_sdp.as_ref().map(|sdp| {
                    // Modify SDP to include sendrecv attribute
                    if sdp.contains("a=sendonly") {
                        sdp.replace("a=sendonly", "a=sendrecv")
                    } else if !sdp.contains("a=sendrecv") {
                        format!("{}\na=sendrecv\r\n", sdp.trim_end())
                    } else {
                        sdp.clone()
                    }
                })
            };
            
            if let Some(sdp_data) = sdp {
                dialog_adapter.send_reinvite_session(&session.session_id, sdp_data).await?;
            }
        }
        
        Action::PlayAudioFile(file) => {
            debug!("Playing audio file {} for session {}", file, session.session_id);
            media_adapter.play_audio_file(&session.session_id, file).await?;
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
            debug!("Creating bridge between {} and {}", session.session_id, other_session);
            media_adapter.create_bridge(&session.session_id, other_session).await?;
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
                    media_adapter.redirect_to_mixer(media_id.clone(), mixer_id.clone()).await?;
                }
            }
        }
        
        Action::ConnectToMixer => {
            debug!("Connecting session {} to conference mixer", session.session_id);
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
                    crate::state_table::types::MediaDirection::SendRecv => crate::types::MediaDirection::SendRecv,
                    crate::state_table::types::MediaDirection::SendOnly => crate::types::MediaDirection::SendOnly,
                    crate::state_table::types::MediaDirection::RecvOnly => crate::types::MediaDirection::RecvOnly,
                    crate::state_table::types::MediaDirection::Inactive => crate::types::MediaDirection::Inactive,
                };
                media_adapter.set_media_direction(media_id.clone(), media_direction).await?;
            }
        }
        
        // Additional call control
        // SendREFER and SendREFERWithReplaces actions removed
        
        Action::MuteLocalAudio => {
            debug!("Muting local audio");
            if let Some(media_id) = &session.media_session_id {
                media_adapter.set_mute(media_id.clone(), true).await?;
            }
        }
        
        Action::UnmuteLocalAudio => {
            debug!("Unmuting local audio");
            if let Some(media_id) = &session.media_session_id {
                media_adapter.set_mute(media_id.clone(), false).await?;
            }
        }
        
        // CreateConsultationCall action removed
        
        // TerminateConsultationCall action removed
        
        Action::SendDTMFTone => {
            debug!("Sending DTMF tone");
            if let Some(digits) = &session.dtmf_digits {
                if let Some(media_id) = &session.media_session_id {
                    for digit in digits.chars() {
                        media_adapter.send_dtmf(media_id.clone(), digit).await?;
                    }
                }
            }
        }
        
        Action::StartRecordingMixer => {
            debug!("Starting recording of conference mixer");
            if let Some(mixer_id) = &session.conference_mixer_id {
                let mixer_session_id = SessionId(format!("mixer-{}", mixer_id.0));
                media_adapter.start_recording(&mixer_session_id).await?;
            }
        }
        
        Action::StopRecordingMixer => {
            debug!("Stopping recording of conference mixer");
            if let Some(mixer_id) = &session.conference_mixer_id {
                let mixer_session_id = SessionId(format!("mixer-{}", mixer_id.0));
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
            error!("Starting emergency cleanup for session {}", session.session_id);
            // Best-effort cleanup on error
            let _ = dialog_adapter.cleanup_session(&session.session_id).await;
            let _ = media_adapter.cleanup_session(&session.session_id).await;
        }
        
        Action::AttemptMediaRecovery => {
            warn!("Attempting media recovery for session {}", session.session_id);
            // Try to recover from media errors
            if let Some(_media_id) = &session.media_session_id {
                // TODO: Implement attempt_recovery
                warn!("attempt_recovery not implemented yet");
            }
        }
        
        Action::Custom(action_name) => {
            debug!("Custom action '{}' for session {}", action_name, session.session_id);
            // Handle custom SIP actions
            match action_name.as_str() {
                "Send180Ringing" => {
                    info!("Sending 180 Ringing for session {}", session.session_id);
                    dialog_adapter.send_response_session(&session.session_id, 180, "Ringing").await?;
                }
                "Send200OK" => {
                    info!("Sending 200 OK for session {}", session.session_id);
                    // For UAS, include SDP in 200 OK
                    if session.role == crate::state_table::Role::UAS {
                        if let Some(local_sdp) = &session.local_sdp {
                            dialog_adapter.send_response_with_sdp(&session.session_id, 200, "OK", local_sdp).await?;
                        } else {
                            dialog_adapter.send_response_session(&session.session_id, 200, "OK").await?;
                        }
                    } else {
                        dialog_adapter.send_response_session(&session.session_id, 200, "OK").await?;
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
                media_adapter.set_media_direction(media_id.clone(), active_direction).await?;
            }

            // Send re-INVITE with sendrecv
            if let Some(active_sdp) = media_adapter.create_active_sdp().await.ok() {
                session.local_sdp = Some(active_sdp.clone());
                dialog_adapter.send_reinvite_session(&session.session_id, active_sdp).await?;
            }
            info!("Media flow restored for session {}", session.session_id);
        }

        Action::RestoreMediaFlow => {
            debug!("Restoring media flow (unhold)");
            if let Some(media_id) = &session.media_session_id {
                use crate::types::MediaDirection;
                let active_direction = MediaDirection::SendRecv;
                media_adapter.set_media_direction(media_id.clone(), active_direction).await?;
            }

            // Send re-INVITE with sendrecv
            if let Some(active_sdp) = media_adapter.create_active_sdp().await.ok() {
                session.local_sdp = Some(active_sdp.clone());
                dialog_adapter.send_reinvite_session(&session.session_id, active_sdp).await?;
            }
            info!("Media flow restored for session {}", session.session_id);
        }
        
        Action::HoldCurrentCall => {
            debug!("Putting current call on hold for transfer");

            // Update media direction to sendonly (we can hear them, they hear hold music/silence)
            if let Some(media_id) = &session.media_session_id {
                use crate::types::MediaDirection;
                let hold_direction = MediaDirection::SendOnly;
                media_adapter.set_media_direction(media_id.clone(), hold_direction).await?;
            }

            // Send re-INVITE with sendonly SDP
            if let Some(hold_sdp) = media_adapter.create_hold_sdp().await.ok() {
                session.local_sdp = Some(hold_sdp.clone());
                dialog_adapter.send_reinvite_session(&session.session_id, hold_sdp).await?;
            }

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
            let from_uri = session.local_uri.as_deref()
                .ok_or_else(|| "local_uri not set for registration".to_string())?;
            let registrar_uri = session.remote_uri.as_deref()
                .ok_or_else(|| "registrar_uri not set for registration".to_string())?;
            let expires = 3600; // Default 1 hour registration
            dialog_adapter.send_register(&session.session_id, from_uri, registrar_uri, expires).await?;
        }
        Action::ProcessRegistrationResponse => {
            debug!("Processing registration response for session {}", session.session_id);
            // Response processing is handled by events from dialog adapter
            // This action is a placeholder for any additional processing needed
        }

        // Subscription actions
        Action::SendSUBSCRIBE => {
            info!("Action::SendSUBSCRIBE for session {}", session.session_id);
            let from_uri = session.local_uri.as_deref()
                .ok_or_else(|| "local_uri not set for subscription".to_string())?;
            let to_uri = session.remote_uri.as_deref()
                .ok_or_else(|| "to_uri not set for subscription".to_string())?;
            let event_package = "presence"; // Default to presence, could be stored in session
            let expires = 3600; // Default 1 hour subscription
            dialog_adapter.send_subscribe(&session.session_id, from_uri, to_uri, event_package, expires).await?;
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
            dialog_adapter.send_notify(&session.session_id, event_package, body, None).await?;
        }

        // Message actions
        Action::SendMESSAGE => {
            info!("Action::SendMESSAGE for session {}", session.session_id);
            let from_uri = session.local_uri.as_deref()
                .ok_or_else(|| "local_uri not set for message".to_string())?;
            let to_uri = session.remote_uri.as_deref()
                .ok_or_else(|| "to_uri not set for message".to_string())?;
            // Get message body from session (could be stored in a specific field)
            let body = session.local_sdp.clone()
                .unwrap_or_else(|| "Test message".to_string());
            let in_dialog = session.dialog_id.is_some(); // Send in-dialog if we have a dialog
            dialog_adapter.send_message(&session.session_id, from_uri, to_uri, body, in_dialog).await?;
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
            debug!("Cleaning up media for session {}", session.session_id);
            if session.media_session_id.is_some() {
                media_adapter.cleanup_session(&session.session_id).await?;
            }
        }

        // ===== Event Publishing Actions =====

        Action::PublishReferEvent => {
            debug!("Publishing REFER event for session {}", session.session_id);
            
            if let Some(event_tx) = simple_peer_event_tx {
                let event = Event::ReferReceived {
                    call_id: session.session_id.clone(),
                    refer_to: session.transfer_target.clone().unwrap_or_default(),
                    referred_by: session.referred_by.clone(),
                    replaces: session.replaces_header.clone(),
                };
                
                if let Err(e) = event_tx.send(event).await {
                    error!("Failed to publish REFER event to SimplePeer: {}", e);
                } else {
                    debug!("Published REFER event to SimplePeer");
                }
            } else {
                debug!("No SimplePeer event channel - REFER event not published");
            }
        }

        Action::SendReferAccepted => {
            debug!("Sending 202 Accepted for REFER request");
            
            let transaction_id = session.refer_transaction_id.clone()
                .unwrap_or_else(|| "unknown".to_string());
            
            // Send ReferResponse event back to dialog-core via global event bus
            let refer_response = rvoip_infra_common::events::cross_crate::SessionToDialogEvent::ReferResponse {
                transaction_id,
                accept: true,
                status_code: 202,
                reason: "Accepted".to_string(),
            };
            
            let event = rvoip_infra_common::events::cross_crate::RvoipCrossCrateEvent::SessionToDialog(refer_response);
            
            // Get global coordinator from dialog adapter
            if let Err(e) = dialog_adapter.global_coordinator.publish(Arc::new(event)).await {
                error!("Failed to publish ReferResponse event: {}", e);
            } else {
                debug!("Published ReferResponse (202 Accepted) event to dialog-core");
            }
        }

        Action::PublishIncomingCallEvent => {
            debug!("Publishing incoming call event for session {}", session.session_id);
            
            if let Some(event_tx) = simple_peer_event_tx {
                let event = Event::IncomingCall {
                    call_id: session.session_id.clone(),
                    from: session.remote_uri.clone().unwrap_or_default(),
                    to: session.local_uri.clone().unwrap_or_default(),
                    sdp: session.remote_sdp.clone(),
                };
                
                if let Err(e) = event_tx.send(event).await {
                    error!("Failed to publish incoming call event to SimplePeer: {}", e);
                } else {
                    debug!("Published incoming call event to SimplePeer");
                }
            }
        }

        Action::PublishCallEndedEvent => {
            debug!("Publishing call ended event for session {}", session.session_id);
            
            if let Some(event_tx) = simple_peer_event_tx {
                let event = Event::CallEnded {
                    call_id: session.session_id.clone(),
                    reason: "Call terminated".to_string(),
                };
                
                let _ = event_tx.send(event).await;
            }
        }

        Action::PublishCallAnsweredEvent => {
            debug!("Publishing call answered event for session {}", session.session_id);
            
            if let Some(event_tx) = simple_peer_event_tx {
                let event = Event::CallAnswered {
                    call_id: session.session_id.clone(),
                    sdp: session.remote_sdp.clone(),
                };
                
                let _ = event_tx.send(event).await;
            }
        }

        Action::PublishCallOnHoldEvent => {
            debug!("Publishing call on hold event for session {}", session.session_id);
            
            if let Some(event_tx) = simple_peer_event_tx {
                let event = Event::CallOnHold {
                    call_id: session.session_id.clone(),
                };
                
                let _ = event_tx.send(event).await;
            }
        }

        Action::PublishCallResumedEvent => {
            debug!("Publishing call resumed event for session {}", session.session_id);
            
            if let Some(event_tx) = simple_peer_event_tx {
                let event = Event::CallResumed {
                    call_id: session.session_id.clone(),
                };
                
                let _ = event_tx.send(event).await;
            }
        }

        Action::PublishDtmfReceivedEvent => {
            debug!("Publishing DTMF received event for session {}", session.session_id);
            // TODO: Extract DTMF digit from session state
            warn!("PublishDtmfReceivedEvent not fully implemented yet");
        }
    }
    
    Ok(())
}