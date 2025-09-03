use std::sync::Arc;
use tracing::{info, debug};

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
        Action::SendSIPResponse(code, _reason) => {
            dialog_adapter.send_response(&session.session_id, *code, session.local_sdp.clone()).await?;
        }
        Action::SendINVITE => {
            let from = session.local_uri.as_deref().unwrap_or("sip:user@localhost");
            let to = session.remote_uri.as_deref().unwrap_or("sip:target@localhost");
            dialog_adapter.send_invite(
                &session.session_id,
                from,
                to, 
                session.local_sdp.clone()
            ).await?;
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
            dialog_adapter.send_bye(&session.session_id).await?;
        }
        Action::SendCANCEL => {
            dialog_adapter.send_cancel(&session.session_id).await?;
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
                dialog_adapter.send_reinvite(&session.session_id, sdp_data).await?;
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
            dialog_adapter.send_refer(&session.session_id, target).await?;
        }
        
        Action::InitiateAttendedTransfer(target) => {
            debug!("Attended transfer from {} to {}", session.session_id, target);
            // For attended transfer, we first establish a consultation call
            // then send REFER with Replaces header
            // For now, just do a blind transfer as a fallback
            dialog_adapter.send_refer(&session.session_id, target).await?;
            info!("Attended transfer initiated (using blind transfer for now)");
        }
        
        Action::Custom(action_name) => {
            debug!("Custom action '{}' for session {}", action_name, session.session_id);
            // Custom actions are application-specific
        }
    }
    
    Ok(())
}