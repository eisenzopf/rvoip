use std::time::SystemTime;

use super::manager::DialogManager;
use super::dialog_id::DialogId;
use crate::errors::{Error, ErrorContext, ErrorCategory, ErrorSeverity, RecoveryAction};
use crate::events::SessionEvent;

impl DialogManager {
    /// Update dialog SDP state with a local SDP offer
    /// 
    /// This is used when sending an SDP offer in a request, to track
    /// the SDP negotiation state.
    pub async fn update_dialog_with_local_sdp_offer(
        &self,
        dialog_id: &DialogId,
        offer: crate::sdp::SessionDescription
    ) -> Result<(), Error> {
        let mut dialog = self.dialogs.get_mut(dialog_id)
            .ok_or_else(|| Error::DialogNotFoundWithId(
                dialog_id.to_string(),
                ErrorContext {
                    category: ErrorCategory::Dialog,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    dialog_id: Some(dialog_id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some(format!("Cannot update SDP - dialog {} not found", dialog_id)),
                    ..Default::default()
                }
            ))?;
            
        dialog.update_with_local_sdp_offer(offer);
        
        // Publish SDP offer event
        if let Some(session_id) = self.dialog_to_session.get(dialog_id) {
            let sdp_event = crate::events::SdpEvent::OfferSent {
                session_id: session_id.to_string(),
                dialog_id: dialog_id.to_string(),
            };
            self.event_bus.publish(sdp_event.into());
        }
        
        Ok(())
    }
    
    /// Update dialog SDP state with a local SDP answer
    /// 
    /// This is used when sending an SDP answer in a response, to track
    /// the SDP negotiation state.
    pub async fn update_dialog_with_local_sdp_answer(
        &self,
        dialog_id: &DialogId,
        answer: crate::sdp::SessionDescription
    ) -> Result<(), Error> {
        let mut dialog = self.dialogs.get_mut(dialog_id)
            .ok_or_else(|| Error::DialogNotFoundWithId(
                dialog_id.to_string(),
                ErrorContext {
                    category: ErrorCategory::Dialog,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    dialog_id: Some(dialog_id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some(format!("Cannot update SDP - dialog {} not found", dialog_id)),
                    ..Default::default()
                }
            ))?;
            
        dialog.update_with_local_sdp_answer(answer);
        
        // Publish SDP answer event
        if let Some(session_id) = self.dialog_to_session.get(dialog_id) {
            let sdp_event = crate::events::SdpEvent::AnswerSent {
                session_id: session_id.to_string(),
                dialog_id: dialog_id.to_string(),
            };
            self.event_bus.publish(sdp_event.into());
        }
        
        Ok(())
    }
    
    /// Update dialog for re-negotiation (re-INVITE)
    /// 
    /// This resets the SDP negotiation state to prepare for a new
    /// offer/answer exchange.
    pub async fn prepare_dialog_sdp_renegotiation(
        &self,
        dialog_id: &DialogId
    ) -> Result<(), Error> {
        let mut dialog = self.dialogs.get_mut(dialog_id)
            .ok_or_else(|| Error::DialogNotFoundWithId(
                dialog_id.to_string(),
                ErrorContext {
                    category: ErrorCategory::Dialog,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    dialog_id: Some(dialog_id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some(format!("Cannot prepare for renegotiation - dialog {} not found", dialog_id)),
                    ..Default::default()
                }
            ))?;
            
        dialog.prepare_sdp_renegotiation();
        Ok(())
    }
} 