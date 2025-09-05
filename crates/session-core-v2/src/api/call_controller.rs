//! Call Controller - High-level call control operations
//!
//! This module provides the main call control functionality including
//! making/receiving calls, hold/resume, transfers, and DTMF.

use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info};

use crate::types::{
    SessionId, DialogId, MediaSessionId, SessionEvent,
    CallDirection, IncomingCallInfo, MediaDirection,
};
use crate::state_table::types::CallState;
use crate::session_registry::SessionRegistry;
use crate::api::session_manager::SessionManager;
use crate::adapters::dialog_adapter::DialogAdapter;
use crate::adapters::media_adapter::MediaAdapter;
use crate::adapters::signaling_interceptor::{SignalingInterceptor, SignalingHandler};
use crate::errors::{Result, SessionError};

/// Call information for active calls
#[derive(Debug, Clone)]
pub struct ActiveCall {
    pub session_id: SessionId,
    pub dialog_id: Option<DialogId>,
    pub media_id: Option<MediaSessionId>,
    pub state: CallState,
    pub direction: CallDirection,
    pub from: String,
    pub to: String,
    pub is_held: bool,
    pub is_muted: bool,
}

/// Call controller for managing call operations
pub struct CallController {
    /// Session manager
    session_manager: Arc<SessionManager>,
    /// Session registry
    registry: Arc<SessionRegistry>,
    /// Dialog adapter
    dialog_adapter: Arc<DialogAdapter>,
    /// Media adapter  
    media_adapter: Arc<MediaAdapter>,
    /// Signaling interceptor
    signaling_interceptor: Arc<RwLock<SignalingInterceptor>>,
    /// Incoming call receiver
    incoming_call_rx: Arc<RwLock<mpsc::Receiver<IncomingCallInfo>>>,
    /// Active calls
    active_calls: Arc<RwLock<Vec<ActiveCall>>>,
}

impl CallController {
    /// Create a new call controller
    pub fn new(
        session_manager: Arc<SessionManager>,
        registry: Arc<SessionRegistry>,
        dialog_adapter: Arc<DialogAdapter>,
        media_adapter: Arc<MediaAdapter>,
    ) -> (Self, mpsc::Sender<IncomingCallInfo>) {
        let (incoming_tx, incoming_rx) = mpsc::channel(100);
        let (event_tx, _event_rx) = mpsc::channel(1000);
        
        let signaling_interceptor = SignalingInterceptor::with_default_handler(
            (*registry).clone(),
            incoming_tx.clone(),
            event_tx,
        );

        let controller = Self {
            session_manager,
            registry,
            dialog_adapter,
            media_adapter,
            signaling_interceptor: Arc::new(RwLock::new(signaling_interceptor)),
            incoming_call_rx: Arc::new(RwLock::new(incoming_rx)),
            active_calls: Arc::new(RwLock::new(Vec::new())),
        };

        (controller, incoming_tx)
    }

    /// Set a custom signaling handler
    pub async fn set_signaling_handler(&self, handler: Box<dyn SignalingHandler>) {
        self.signaling_interceptor.write().await.set_handler(handler);
    }

    /// Make an outgoing call
    pub async fn make_call(&self, from: String, to: String) -> Result<SessionId> {
        info!("Making call from {} to {}", from, to);

        // Create session
        let session_id = self.session_manager
            .create_session(from.clone(), to.clone(), CallDirection::Outgoing)
            .await?;

        // Create dialog
        let rvoip_dialog_id = self.dialog_adapter.create_dialog(&from, &to).await?;
        
        // Convert RvoipDialogId to our DialogId type
        let dialog_id: crate::types::DialogId = rvoip_dialog_id.into();
        
        // Map dialog to session
        self.session_manager.map_dialog(session_id.clone(), dialog_id.clone());

        // Create media session
        let media_id = self.media_adapter.create_media_session().await?;
        
        // Map media to session
        self.session_manager.map_media(session_id.clone(), media_id.clone());

        // Add to active calls
        let call = ActiveCall {
            session_id: session_id.clone(),
            dialog_id: Some(dialog_id.clone()),
            media_id: Some(media_id),
            state: CallState::Initiating,
            direction: CallDirection::Outgoing,
            from,
            to,
            is_held: false,
            is_muted: false,
        };
        self.active_calls.write().await.push(call);

        // Initiate the call
        self.dialog_adapter.send_invite(dialog_id).await?;

        // Update state
        self.session_manager
            .update_session_state(&session_id, CallState::Initiating)
            .await?;

        Ok(session_id)
    }

    /// Get the next incoming call
    pub async fn get_incoming_call(&self) -> Option<IncomingCallInfo> {
        self.incoming_call_rx.write().await.recv().await
    }

    /// Accept an incoming call
    pub async fn accept_call(&self, session_id: &SessionId) -> Result<()> {
        info!("Accepting call {}", session_id);

        // Get dialog ID from registry
        let dialog_id = self.registry.get_dialog_by_session(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.to_string()))?;

        // Create media session
        let media_id = self.media_adapter.create_media_session().await?;
        self.session_manager.map_media(session_id.clone(), media_id.clone());

        // Send acceptance
        self.dialog_adapter.send_response_by_dialog(dialog_id.clone(), 200, "OK").await?;

        // Update state
        self.session_manager
            .update_session_state(session_id, CallState::Active)
            .await?;

        // Add to active calls
        if let Some(metadata) = self.session_manager.get_session(session_id).await {
            let call = ActiveCall {
                session_id: session_id.clone(),
                dialog_id: Some(dialog_id),
                media_id: Some(media_id),
                state: CallState::Active,
                direction: CallDirection::Incoming,
                from: metadata.from,
                to: metadata.to,
                is_held: false,
                is_muted: false,
            };
            self.active_calls.write().await.push(call);
        }

        Ok(())
    }

    /// Reject an incoming call
    pub async fn reject_call(&self, session_id: &SessionId, reason: &str) -> Result<()> {
        info!("Rejecting call {} with reason: {}", session_id, reason);

        // Get dialog ID from registry
        let dialog_id = self.registry.get_dialog_by_session(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.to_string()))?;

        // Send rejection
        self.dialog_adapter.send_response_by_dialog(dialog_id, 486, reason).await?;

        // Update state
        self.session_manager
            .update_session_state(session_id, CallState::Failed(crate::state_table::types::FailureReason::Rejected))
            .await?;

        // Terminate session
        self.session_manager
            .terminate_session(session_id, Some(reason.to_string()))
            .await?;

        Ok(())
    }

    /// Hang up a call
    pub async fn hangup(&self, session_id: &SessionId) -> Result<()> {
        info!("Hanging up call {}", session_id);

        // Get dialog ID from registry
        if let Some(dialog_id) = self.registry.get_dialog_by_session(session_id) {
            // Send BYE
            self.dialog_adapter.send_bye(dialog_id).await?;
        }

        // Stop media session
        if let Some(media_id) = self.registry.get_media_by_session(session_id) {
            self.media_adapter.stop_media_session(media_id).await?;
        }

        // Update state
        self.session_manager
            .update_session_state(session_id, CallState::Terminated)
            .await?;

        // Remove from active calls
        self.active_calls.write().await.retain(|c| c.session_id != *session_id);

        // Terminate session
        self.session_manager
            .terminate_session(session_id, Some("Normal hangup".to_string()))
            .await?;

        Ok(())
    }

    /// Put a call on hold
    pub async fn hold(&self, session_id: &SessionId) -> Result<()> {
        info!("Putting call {} on hold", session_id);

        // Get dialog ID
        let dialog_id = self.registry.get_dialog_by_session(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.to_string()))?;

        // Create hold SDP
        let hold_sdp = self.media_adapter.create_hold_sdp().await?;

        // Send re-INVITE with hold SDP
        self.dialog_adapter.send_reinvite(dialog_id, hold_sdp).await?;

        // Update media direction
        if let Some(media_id) = self.registry.get_media_by_session(session_id) {
            self.media_adapter.set_media_direction(media_id, MediaDirection::SendOnly).await?;
        }

        // Update state
        self.session_manager
            .update_session_state(session_id, CallState::OnHold)
            .await?;

        // Update active call
        let mut calls = self.active_calls.write().await;
        if let Some(call) = calls.iter_mut().find(|c| c.session_id == *session_id) {
            call.is_held = true;
            call.state = CallState::OnHold;
        }

        Ok(())
    }

    /// Resume a call from hold
    pub async fn resume(&self, session_id: &SessionId) -> Result<()> {
        info!("Resuming call {} from hold", session_id);

        // Get dialog ID
        let dialog_id = self.registry.get_dialog_by_session(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.to_string()))?;

        // Create active SDP
        let active_sdp = self.media_adapter.create_active_sdp().await?;

        // Send re-INVITE with active SDP
        self.dialog_adapter.send_reinvite(dialog_id, active_sdp).await?;

        // Update media direction
        if let Some(media_id) = self.registry.get_media_by_session(session_id) {
            self.media_adapter.set_media_direction(media_id, MediaDirection::SendRecv).await?;
        }

        // Update state
        self.session_manager
            .update_session_state(session_id, CallState::Active)
            .await?;

        // Update active call
        let mut calls = self.active_calls.write().await;
        if let Some(call) = calls.iter_mut().find(|c| c.session_id == *session_id) {
            call.is_held = false;
            call.state = CallState::Active;
        }

        Ok(())
    }

    /// Transfer a call (blind transfer)
    pub async fn transfer_blind(&self, session_id: &SessionId, target: &str) -> Result<()> {
        info!("Blind transferring call {} to {}", session_id, target);

        // Get dialog ID
        let dialog_id = self.registry.get_dialog_by_session(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.to_string()))?;

        // Send REFER
        self.dialog_adapter.send_refer(dialog_id, target, false).await?;

        // Update state
        self.session_manager
            .update_session_state(session_id, CallState::Transferring)
            .await?;

        Ok(())
    }

    /// Transfer a call (attended transfer)
    pub async fn transfer_attended(
        &self,
        session_id: &SessionId,
        target_session_id: &SessionId,
    ) -> Result<()> {
        info!("Attended transferring call {} to session {}", session_id, target_session_id);

        // Get dialog IDs
        let dialog_id = self.registry.get_dialog_by_session(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.to_string()))?;
        
        let target_dialog_id = self.registry.get_dialog_by_session(target_session_id)
            .ok_or_else(|| SessionError::SessionNotFound(target_session_id.to_string()))?;

        // Get target dialog info
        let target_uri = self.dialog_adapter.get_remote_uri(target_dialog_id.clone()).await?;

        // Send REFER with Replaces header
        self.dialog_adapter.send_refer(dialog_id, &target_uri, true).await?;

        // Update states
        self.session_manager
            .update_session_state(session_id, CallState::Transferring)
            .await?;

        Ok(())
    }

    /// Send DTMF digit
    pub async fn send_dtmf(&self, session_id: &SessionId, digit: char) -> Result<()> {
        debug!("Sending DTMF digit {} for call {}", digit, session_id);

        // Validate digit
        if !digit.is_ascii_digit() && digit != '*' && digit != '#' {
            return Err(SessionError::InvalidInput(format!("Invalid DTMF digit: {}", digit)));
        }

        // Get media session
        let media_id = self.registry.get_media_by_session(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.to_string()))?;

        // Send DTMF through media adapter
        self.media_adapter.send_dtmf(media_id, digit).await?;

        Ok(())
    }

    /// Mute the microphone
    pub async fn mute(&self, session_id: &SessionId) -> Result<()> {
        debug!("Muting call {}", session_id);

        // Get media session
        let media_id = self.registry.get_media_by_session(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.to_string()))?;

        // Mute through media adapter
        self.media_adapter.set_mute(media_id, true).await?;

        // Update active call
        let mut calls = self.active_calls.write().await;
        if let Some(call) = calls.iter_mut().find(|c| c.session_id == *session_id) {
            call.is_muted = true;
        }

        Ok(())
    }

    /// Unmute the microphone
    pub async fn unmute(&self, session_id: &SessionId) -> Result<()> {
        debug!("Unmuting call {}", session_id);

        // Get media session
        let media_id = self.registry.get_media_by_session(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.to_string()))?;

        // Unmute through media adapter
        self.media_adapter.set_mute(media_id, false).await?;

        // Update active call
        let mut calls = self.active_calls.write().await;
        if let Some(call) = calls.iter_mut().find(|c| c.session_id == *session_id) {
            call.is_muted = false;
        }

        Ok(())
    }

    /// Get all active calls
    pub async fn get_active_calls(&self) -> Vec<ActiveCall> {
        self.active_calls.read().await.clone()
    }

    /// Get a specific active call
    pub async fn get_call(&self, session_id: &SessionId) -> Option<ActiveCall> {
        self.active_calls.read().await
            .iter()
            .find(|c| c.session_id == *session_id)
            .cloned()
    }

    /// Handle an incoming invite (called by DialogAdapter)
    pub async fn handle_incoming_invite(
        &self,
        from: &str,
        to: &str,
        call_id: &str,
        dialog_id: DialogId,
        sdp: Option<String>,
    ) -> Result<()> {
        let event = SessionEvent::IncomingCall {
            from: from.to_string(),
            to: to.to_string(),
            call_id: call_id.to_string(),
            dialog_id,
            sdp,
        };

        self.signaling_interceptor.read().await
            .handle_signaling_event(event)
            .await
            .map_err(|e| SessionError::Other(e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::unified::{UnifiedCoordinator, Config};

    async fn create_test_controller() -> (CallController, mpsc::Sender<IncomingCallInfo>) {
        create_test_controller_with_port(15061).await
    }
    
    async fn create_test_controller_with_port(port: u16) -> (CallController, mpsc::Sender<IncomingCallInfo>) {
        let config = Config {
            sip_port: port,
            media_port_start: 17000 + (port - 15061) * 1000,
            media_port_end: 18000 + (port - 15061) * 1000,
            local_ip: "127.0.0.1".parse().unwrap(),
            bind_addr: format!("127.0.0.1:{}", port).parse().unwrap(),
            state_table_path: None,
        };
        
        let coordinator = UnifiedCoordinator::new(config).await.unwrap();
        let session_manager = coordinator.session_manager().await.unwrap();
        let dialog_adapter = coordinator.dialog_adapter();
        let media_adapter = coordinator.media_adapter();
        let registry = coordinator.session_registry();
        
        CallController::new(
            session_manager,
            registry,
            dialog_adapter,
            media_adapter,
        )
    }

    #[tokio::test]
    async fn test_make_call() {
        let (controller, _) = create_test_controller_with_port(15072).await;
        
        let session_id = controller.make_call(
            "alice@example.com".to_string(),
            "bob@example.com".to_string(),
        ).await.unwrap();

        let calls = controller.get_active_calls().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].session_id, session_id);
        assert_eq!(calls[0].direction, CallDirection::Outgoing);
    }

    #[tokio::test]
    async fn test_hold_resume() {
        let (controller, _) = create_test_controller_with_port(15073).await;
        
        let session_id = controller.make_call(
            "alice@example.com".to_string(),
            "bob@example.com".to_string(),
        ).await.unwrap();

        // Put on hold
        controller.hold(&session_id).await.unwrap();
        
        let call = controller.get_call(&session_id).await.unwrap();
        assert!(call.is_held);
        assert_eq!(call.state, CallState::OnHold);

        // Resume
        controller.resume(&session_id).await.unwrap();
        
        let call = controller.get_call(&session_id).await.unwrap();
        assert!(!call.is_held);
        assert_eq!(call.state, CallState::Active);
    }

    #[tokio::test]
    async fn test_mute_unmute() {
        let (controller, _) = create_test_controller_with_port(15074).await;
        
        let session_id = controller.make_call(
            "alice@example.com".to_string(),
            "bob@example.com".to_string(),
        ).await.unwrap();

        // Mute
        controller.mute(&session_id).await.unwrap();
        
        let call = controller.get_call(&session_id).await.unwrap();
        assert!(call.is_muted);

        // Unmute
        controller.unmute(&session_id).await.unwrap();
        
        let call = controller.get_call(&session_id).await.unwrap();
        assert!(!call.is_muted);
    }

    #[tokio::test]
    async fn test_send_dtmf() {
        let (controller, _) = create_test_controller_with_port(15075).await;
        
        let session_id = controller.make_call(
            "alice@example.com".to_string(),
            "bob@example.com".to_string(),
        ).await.unwrap();

        // Test valid digits
        controller.send_dtmf(&session_id, '1').await.unwrap();
        controller.send_dtmf(&session_id, '*').await.unwrap();
        controller.send_dtmf(&session_id, '#').await.unwrap();

        // Test invalid digit
        assert!(controller.send_dtmf(&session_id, 'X').await.is_err());
    }
}