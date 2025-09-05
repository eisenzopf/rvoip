//! Event Router - Connects adapters to state machine
//!
//! Routes events from dialog/media adapters to the state machine,
//! and routes actions from the state machine to the appropriate adapter.

use std::sync::Arc;
use tokio::sync::mpsc;
use crate::{
    state_table::types::{SessionId, EventType, Action},
    state_machine::executor::StateMachine as StateMachineExecutor,
    session_store::SessionStore,
    errors::Result,
};
use super::{
    dialog_adapter::DialogAdapter,
    media_adapter::MediaAdapter,
};

/// Routes events and actions between adapters and state machine
pub struct EventRouter {
    /// State machine executor
    state_machine: Arc<StateMachineExecutor>,
    
    /// Session store
    store: Arc<SessionStore>,
    
    /// Dialog adapter
    pub dialog_adapter: Arc<DialogAdapter>,
    
    /// Media adapter
    media_adapter: Arc<MediaAdapter>,
    
    /// Event receiver from adapters
    event_rx: Option<mpsc::Receiver<(SessionId, EventType)>>,
    
    /// Event sender for adapters
    event_tx: mpsc::Sender<(SessionId, EventType)>,
}

impl EventRouter {
    /// Create a new event router
    pub fn new(
        state_machine: Arc<StateMachineExecutor>,
        store: Arc<SessionStore>,
        dialog_adapter: Arc<DialogAdapter>,
        media_adapter: Arc<MediaAdapter>,
    ) -> Self {
        let (event_tx, event_rx) = mpsc::channel(1000);
        
        Self {
            state_machine,
            store,
            dialog_adapter,
            media_adapter,
            event_rx: Some(event_rx),
            event_tx,
        }
    }
    
    /// Get the event sender for adapters to use
    pub fn event_sender(&self) -> mpsc::Sender<(SessionId, EventType)> {
        self.event_tx.clone()
    }
    
    /// Start the event router
    pub async fn start(&self) -> Result<()> {
        // Start adapters
        self.dialog_adapter.start_event_loop().await?;
        self.media_adapter.start_event_loop().await?;
        
        // Note: Event processing would normally happen here, but since we're
        // using a different architecture with adapters publishing events directly
        // through GlobalEventCoordinator, we don't need the event loop here.
        
        Ok(())
    }
    
    /// Route an event to the state machine
    async fn route_event(&self, session_id: SessionId, event: EventType) -> Result<()> {
        tracing::debug!("Routing event {:?} for session {}", event, session_id);
        
        // Process through state machine
        let transition_result = self.state_machine.process_event(&session_id, event).await?;
        
        // Execute any actions from the transition
        for action in &transition_result.actions_executed {
            self.execute_action(&session_id, action).await?;
        }
        
        Ok(())
    }
    
    /// Execute an action by routing to the appropriate adapter
    pub async fn execute_action(&self, session_id: &SessionId, action: &Action) -> Result<()> {
        tracing::debug!("Executing action {:?} for session {}", action, session_id);
        
        match action {
            // Dialog actions
            Action::SendINVITE => {
                // Get session to get from/to/sdp
                let session = self.store.get_session(session_id).await?;
                let from = session.local_uri.unwrap_or_else(|| "sip:user@localhost".to_string());
                let to = session.remote_uri.unwrap_or_else(|| "sip:remote@localhost".to_string());
                self.dialog_adapter.send_invite_with_details(session_id, &from, &to, session.local_sdp).await?;
            }
            
            Action::SendSIPResponse(code, _reason) => {
                let state = self.store.get_session(session_id).await?;
                self.dialog_adapter.send_response(
                    session_id,
                    *code,
                    state.local_sdp.clone(),
                ).await?;
            }
            
            Action::SendACK => {
                // Get the stored 200 OK response
                let state = self.store.get_session(session_id).await?;
                let response = if let Some(serialized) = &state.last_200_ok {
                    // Deserialize the stored response
                    bincode::deserialize::<rvoip_sip_core::Response>(serialized)
                        .unwrap_or_else(|_| rvoip_sip_core::Response::new(rvoip_sip_core::StatusCode::Ok))
                } else {
                    tracing::warn!("No 200 OK response stored for ACK, using dummy response");
                    rvoip_sip_core::Response::new(rvoip_sip_core::StatusCode::Ok)
                };
                self.dialog_adapter.send_ack(session_id, &response).await?;
            }
            
            Action::SendBYE => {
                // Send BYE using the dialog_id from session
                let session = self.store.get_session(session_id).await?;
                if let Some(dialog_id) = session.dialog_id {
                    self.dialog_adapter.send_bye(dialog_id).await?;
                }
            }
            
            Action::SendCANCEL => {
                self.dialog_adapter.send_cancel(session_id).await?;
            }
            
            Action::SendReINVITE => {
                let state = self.store.get_session(session_id).await?;
                if let Some(sdp) = state.local_sdp {
                    // Send re-INVITE using session_id
                    self.dialog_adapter.send_reinvite_session(session_id, sdp).await?;
                }
            }
            
            // Media actions
            Action::StartMediaSession => {
                self.media_adapter.start_session(session_id).await?;
            }
            
            Action::StopMediaSession => {
                self.media_adapter.stop_session(session_id).await?;
            }
            
            Action::NegotiateSDPAsUAC => {
                let mut state = self.store.get_session(session_id).await?;
                if let Some(remote_sdp) = state.remote_sdp.clone() {
                    let config = self.media_adapter
                        .negotiate_sdp_as_uac(session_id, &remote_sdp)
                        .await?;
                    
                    // Convert to session_store NegotiatedConfig
                    let session_config = crate::session_store::state::NegotiatedConfig {
                        local_addr: config.local_addr,
                        remote_addr: config.remote_addr,
                        codec: config.codec,
                        sample_rate: 8000, // Default for PCMU
                        channels: 1,
                    };
                    
                    // Update session state
                    state.negotiated_config = Some(session_config);
                    self.store.update_session(state).await?;
                }
            }
            
            Action::NegotiateSDPAsUAS => {
                let mut state = self.store.get_session(session_id).await?;
                if let Some(remote_sdp) = state.remote_sdp.clone() {
                    let (local_sdp, config) = self.media_adapter
                        .negotiate_sdp_as_uas(session_id, &remote_sdp)
                        .await?;
                    
                    // Convert to session_store NegotiatedConfig
                    let session_config = crate::session_store::state::NegotiatedConfig {
                        local_addr: config.local_addr,
                        remote_addr: config.remote_addr,
                        codec: config.codec,
                        sample_rate: 8000, // Default for PCMU
                        channels: 1,
                    };
                    
                    // Update session state
                    state.local_sdp = Some(local_sdp);
                    state.negotiated_config = Some(session_config);
                    self.store.update_session(state).await?;
                }
            }
            
            // Media control actions
            Action::PlayAudioFile(file) => {
                tracing::info!("Playing audio file {} for session {}", file, session_id);
                self.media_adapter.play_audio_file(session_id, file).await?;
            }
            
            Action::StartRecordingMedia => {
                tracing::info!("Starting recording for session {}", session_id);
                let recording_path = self.media_adapter.start_recording(session_id).await?;
                tracing::info!("Recording started at: {}", recording_path);
            }
            
            Action::StopRecordingMedia => {
                tracing::info!("Stopping recording for session {}", session_id);
                self.media_adapter.stop_recording(session_id).await?;
            }
            
            // Bridge/Transfer actions
            Action::CreateBridge(other_session) => {
                tracing::info!("Creating bridge between {} and {}", session_id, other_session);
                self.media_adapter.create_bridge(session_id, other_session).await?;
                // Update session state
                if let Ok(mut session) = self.store.get_session(session_id).await {
                    session.bridged_to = Some(other_session.clone());
                    let _ = self.store.update_session(session).await;
                }
            }
            
            Action::DestroyBridge => {
                tracing::info!("Destroying bridge for session {}", session_id);
                self.media_adapter.destroy_bridge(session_id).await?;
                // Update session state
                if let Ok(mut session) = self.store.get_session(session_id).await {
                    session.bridged_to = None;
                    let _ = self.store.update_session(session).await;
                }
            }
            
            Action::InitiateBlindTransfer(target) => {
                tracing::info!("Blind transfer from {} to {}", session_id, target);
                self.dialog_adapter.send_refer_session(session_id, target).await?;
            }
            
            Action::InitiateAttendedTransfer(target) => {
                tracing::info!("Attended transfer from {} to {}", session_id, target);
                // For attended transfer, we first establish a consultation call
                // then send REFER with Replaces header
                // For now, just do a blind transfer as a fallback
                self.dialog_adapter.send_refer_session(session_id, target).await?;
                tracing::info!("Attended transfer initiated (using blind transfer for now)");
            }
            
            // Cleanup actions
            Action::StartDialogCleanup => {
                self.dialog_adapter.cleanup_session(session_id).await?;
                tracing::debug!("Dialog cleanup completed for session {}", session_id);
            }
            
            Action::StartMediaCleanup => {
                self.media_adapter.cleanup_session(session_id).await?;
                tracing::debug!("Media cleanup completed for session {}", session_id);
            }
            
            // State updates (handled by state machine)
            Action::SetCondition(_, _) |
            Action::StoreLocalSDP |
            Action::StoreRemoteSDP |
            Action::StoreNegotiatedConfig |
            Action::TriggerCallEstablished |
            Action::TriggerCallTerminated => {
                // These are handled by the state machine itself
            }
            
            Action::Custom(name) => {
                tracing::debug!("Custom action '{}' for session {}", name, session_id);
                // Application-specific custom actions
            }
            
            // Call control actions
            Action::HoldCall => {
                tracing::info!("Putting call on hold for session {}", session_id);
                // TODO: Implement hold
            }
            
            Action::ResumeCall => {
                tracing::info!("Resuming call for session {}", session_id);
                // TODO: Implement resume
            }
            
            Action::TransferCall(target) => {
                tracing::info!("Transferring call to {} for session {}", target, session_id);
                // TODO: Implement transfer
            }
            
            Action::SendDTMF(digit) => {
                tracing::info!("Sending DTMF {} for session {}", digit, session_id);
                // TODO: Implement DTMF sending
            }
            
            Action::StartRecording => {
                tracing::info!("Starting recording for session {}", session_id);
                let _ = self.media_adapter.start_recording(session_id).await?;
            }
            
            Action::StopRecording => {
                tracing::info!("Stopping recording for session {}", session_id);
                self.media_adapter.stop_recording(session_id).await?;
            }
        }
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_event_router_creation() {
        // This would need mock adapters for proper testing
        // For now, just ensure the types compile correctly
    }
}