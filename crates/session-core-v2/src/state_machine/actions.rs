use std::sync::Arc;
use tracing::{info, debug, warn, error};
use crate::state_table::types::SessionId;

use crate::{
    state_table::{Action, Condition},
    session_store::SessionState,
    adapters::{dialog_adapter::DialogAdapter, media_adapter::MediaAdapter},
};

/// Execute an action from the state table
pub async fn execute_action(
    action: &Action,
    session: &mut SessionState,
    dialog_adapter: &Arc<DialogAdapter>,
    media_adapter: &Arc<MediaAdapter>,
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
            let dialog_id = dialog_adapter.create_dialog(from, to).await?;
            session.dialog_id = Some(dialog_id);
            info!("Created dialog ID: {:?}", dialog_id);
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
        }
        Action::SendINVITE => {
            info!("Action::SendINVITE for session {}", session.session_id);
            // Get session details for send_invite_with_details
            let from = session.local_uri.clone()
                .ok_or_else(|| "local_uri not set for session".to_string())?;
            let to = session.remote_uri.clone()
                .ok_or_else(|| "remote_uri not set for session".to_string())?;
            info!("Sending INVITE from {} to {} with SDP: {}", from, to, session.local_sdp.is_some());
            dialog_adapter.send_invite_with_details(&session.session_id, &from, &to, session.local_sdp.clone()).await?;
            info!("INVITE sent successfully");
        }
        Action::SendACK => {
            // Use the stored 200 OK response if available
            let response = if let Some(serialized) = &session.last_200_ok {
                // Deserialize the stored response
                bincode::deserialize::<rvoip_sip_core::Response>(serialized)
                    .unwrap_or_else(|_| rvoip_sip_core::Response::new(rvoip_sip_core::StatusCode::Ok))
            } else {
                // Fallback to a dummy response if none stored
                tracing::warn!("No 200 OK response stored for ACK, using dummy response");
                rvoip_sip_core::Response::new(rvoip_sip_core::StatusCode::Ok)
            };
            dialog_adapter.send_ack(&session.session_id, &response).await?;
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
            // Remote SDP is stored when event is received
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
            let sdp = if session.call_state == crate::state_table::CallState::Active {
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
        
        Action::InitiateBlindTransfer(target) => {
            debug!("Blind transfer from {} to {}", session.session_id, target);
            dialog_adapter.send_refer_session(&session.session_id, target).await?;
        }
        
        Action::InitiateAttendedTransfer(target) => {
            debug!("Attended transfer from {} to {}", session.session_id, target);
            // For attended transfer, we first establish a consultation call
            // then send REFER with Replaces header
            // For now, just do a blind transfer as a fallback
            dialog_adapter.send_refer_session(&session.session_id, target).await?;
            info!("Attended transfer initiated (using blind transfer for now)");
        }
        
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
            if let Some(media_id) = &session.media_session_id {
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
        Action::SendREFER => {
            debug!("Sending REFER for transfer");
            // The target would be in session data
            if let Some(target) = &session.transfer_target {
                dialog_adapter.send_refer_session(&session.session_id, target).await?;
            }
        }
        
        Action::SendREFERWithReplaces => {
            debug!("Sending REFER with Replaces for attended transfer");
            if let Some(target) = &session.transfer_target {
                dialog_adapter.send_refer_with_replaces(&session.session_id, target).await?;
            }
        }
        
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
        
        Action::CreateConsultationCall => {
            debug!("Creating consultation call for attended transfer");
            // This would create a new session for consultation
            // Handled by state machine creating a new session
        }
        
        Action::TerminateConsultationCall => {
            debug!("Terminating consultation call");
            // Clean up the consultation session
        }
        
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
        
        Action::RestoreMediaFlow => {
            debug!("Restoring media flow after hold/resume");
            if let Some(media_id) = &session.media_session_id {
                // TODO: Implement restore_media_flow
                warn!("restore_media_flow not implemented yet");
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
            if let Some(media_id) = &session.media_session_id {
                // TODO: Implement attempt_recovery
                warn!("attempt_recovery not implemented yet");
            }
        }
        
        Action::Custom(action_name) => {
            debug!("Custom action '{}' for session {}", action_name, session.session_id);
            // Custom actions are application-specific
        }
        
        // Missing actions that need implementation
        Action::BridgeToMixer => {
            debug!("Bridging session {} to mixer", session.session_id);
            // TODO: Implement bridge to mixer functionality
            warn!("BridgeToMixer not implemented yet");
        }
        
        Action::RestoreDirectMedia => {
            debug!("Restoring direct media for session {}", session.session_id);
            if let Some(media_id) = &session.media_session_id {
                // TODO: Implement restore_direct_media in MediaAdapter
                warn!("RestoreDirectMedia not implemented yet");
            }
        }
        
        Action::HoldCurrentCall => {
            debug!("Holding current call for session {}", session.session_id);
            // TODO: Implement hold current call
            warn!("HoldCurrentCall not implemented yet");
        }
        
        Action::CleanupResources => {
            debug!("Cleaning up resources for session {}", session.session_id);
            // TODO: Implement resource cleanup
            warn!("CleanupResources not implemented yet");
        }
    }
    
    Ok(())
}