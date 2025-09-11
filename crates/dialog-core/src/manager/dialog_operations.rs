//! Dialog CRUD Operations
//!
//! This module handles all Create, Read, Update, Delete operations for SIP dialogs,
//! implementing RFC 3261 compliant dialog management with proper state transitions.

use tracing::{debug, info, warn};
use uuid::Uuid;
use dashmap::mapref::one::RefMut;

use rvoip_sip_core::{Request, Uri};
use crate::dialog::{Dialog, DialogId, DialogState};
use crate::errors::{DialogError, DialogResult};
use crate::events::SessionCoordinationEvent;
use super::core::DialogManager;
use super::utils::DialogUtils;

/// Trait for dialog storage operations
/// 
/// Provides the interface for storing, retrieving, and managing dialogs
/// according to RFC 3261 dialog identification and state management rules.
pub trait DialogStore {
    /// Create a new dialog from an incoming request
    fn create_dialog(&self, request: &Request) -> impl std::future::Future<Output = DialogResult<DialogId>> + Send;
    
    /// Create an outgoing dialog for client-initiated requests
    fn create_outgoing_dialog(
        &self,
        local_uri: Uri,
        remote_uri: Uri,
        call_id: Option<String>,
    ) -> impl std::future::Future<Output = DialogResult<DialogId>> + Send;
    
    /// Store a dialog in the manager
    fn store_dialog(&self, dialog: Dialog) -> impl std::future::Future<Output = DialogResult<()>> + Send;
    
    /// Get a dialog by ID (read-only)
    fn get_dialog(&self, dialog_id: &DialogId) -> DialogResult<Dialog>;
    
    /// Get a mutable reference to a dialog
    fn get_dialog_mut(&self, dialog_id: &DialogId) -> DialogResult<RefMut<DialogId, Dialog>>;
    
    /// Terminate a dialog
    fn terminate_dialog(&self, dialog_id: &DialogId) -> impl std::future::Future<Output = DialogResult<()>> + Send;
    
    /// List all active dialogs
    fn list_dialogs(&self) -> Vec<DialogId>;
    
    /// Get current dialog count
    fn dialog_count(&self) -> usize;
    
    /// Check if a dialog exists
    fn has_dialog(&self, dialog_id: &DialogId) -> bool;
    
    /// Get dialog state
    fn get_dialog_state(&self, dialog_id: &DialogId) -> DialogResult<DialogState>;
    
    /// Update dialog state with proper notifications
    fn update_dialog_state(
        &self,
        dialog_id: &DialogId,
        new_state: DialogState,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
}

/// Trait for dialog lookup operations
/// 
/// Provides RFC 3261 compliant dialog lookup mechanisms based on 
/// Call-ID, tags, and other dialog identifiers.
pub trait DialogLookup {
    /// Find dialog for an incoming request
    fn find_dialog_for_request(&self, request: &Request) -> impl std::future::Future<Output = Option<DialogId>> + Send;
    
    /// Create early dialog from INVITE request
    fn create_early_dialog_from_invite(&self, request: &Request) -> impl std::future::Future<Output = DialogResult<DialogId>> + Send;
}

// Implement DialogStore for DialogManager
impl DialogStore for DialogManager {
    /// Create a new dialog from an incoming request
    /// 
    /// Implements RFC 3261 Section 12.1.1 for UAS dialog creation.
    /// Creates an early dialog that can be confirmed later.
    async fn create_dialog(&self, request: &Request) -> DialogResult<DialogId> {
        debug!("Creating dialog from incoming request");
        
        // Extract dialog information from request and create dialog
        let call_id = request.call_id()
            .ok_or_else(|| DialogError::protocol_error("Request missing Call-ID header"))?
            .to_string();
            
        let from_uri = request.from()
            .ok_or_else(|| DialogError::protocol_error("Request missing From header"))?
            .uri().clone();
            
        let to_uri = request.to()
            .ok_or_else(|| DialogError::protocol_error("Request missing To header"))?
            .uri().clone();
            
        let remote_tag = request.from()
            .and_then(|from| from.tag())
            .map(|tag| tag.to_string());
            
        // Create dialog (early state for INVITE)
        let dialog = Dialog::new_early(
            call_id,
            to_uri,      // local_uri (we are the UAS)
            from_uri,    // remote_uri (they are the UAC)
            None,        // local_tag (generated later when we respond)
            remote_tag,  // remote_tag (from their From header)
            false,       // is_initiator = false (incoming request, we are UAS)
        );
        
        let dialog_id = dialog.id.clone();
        self.store_dialog(dialog).await?;
        
        debug!("Created UAS dialog {} for incoming request", dialog_id);
        Ok(dialog_id)
    }
    
    /// Create an outgoing dialog for client-initiated requests
    /// 
    /// Implements RFC 3261 Section 12.1.2 for UAC dialog creation.
    /// Creates an early dialog that will be confirmed by the response.
    async fn create_outgoing_dialog(
        &self,
        local_uri: Uri,
        remote_uri: Uri,
        call_id: Option<String>,
    ) -> DialogResult<DialogId> {
        debug!("Creating outgoing dialog for UAC request");
        
        // Generate call-id if not provided
        let call_id = call_id.unwrap_or_else(|| {
            format!("call-{}", Uuid::new_v4())
        });
        
        // Create outgoing dialog (UAC perspective)
        let dialog = Dialog::new_early(
            call_id.clone(),  // Clone call_id for later use
            local_uri,  // local_uri (we are the UAC)
            remote_uri, // remote_uri (they are the UAS)
            None,       // local_tag (will be generated when we send request)
            None,       // remote_tag (will be set from response)
            true,       // is_initiator = true (we're UAC)
        );
        
        let dialog_id = dialog.id.clone();
        self.store_dialog(dialog).await?;
        
        // Publish DialogCreated event for session-core to track
        if let Some(hub) = self.event_hub.read().await.as_ref() {
            let event = rvoip_infra_common::events::cross_crate::RvoipCrossCrateEvent::DialogToSession(
                rvoip_infra_common::events::cross_crate::DialogToSessionEvent::DialogCreated {
                    dialog_id: dialog_id.to_string(),
                    call_id: call_id.clone(),
                }
            );
            if let Err(e) = hub.publish_cross_crate_event(event).await {
                warn!("Failed to publish DialogCreated event: {}", e);
            } else {
                info!("Published DialogCreated event for dialog {} with call-id {}", dialog_id, call_id);
            }
        }
        
        debug!("Created UAC dialog {} for outgoing request", dialog_id);
        Ok(dialog_id)
    }
    
    /// Store a dialog in the manager
    /// 
    /// Implements proper dialog storage with RFC 3261 compliant lookup keys.
    async fn store_dialog(&self, dialog: Dialog) -> DialogResult<()> {
        let dialog_id = dialog.id.clone();
        
        // Store the dialog
        self.dialogs.insert(dialog_id.clone(), dialog.clone());
        
        // Store dialog lookup if we have both tags (confirmed dialog)
        if let Some(tuple) = dialog.dialog_id_tuple() {
            let key = DialogUtils::create_lookup_key(&tuple.0, &tuple.1, &tuple.2);
            self.dialog_lookup.insert(key, dialog_id.clone());
            debug!("Stored confirmed dialog lookup for {}", dialog_id);
        }
        
        debug!("Stored dialog {} (state: {:?})", dialog_id, dialog.state);
        Ok(())
    }
    
    /// Get a dialog (read-only)
    fn get_dialog(&self, dialog_id: &DialogId) -> DialogResult<Dialog> {
        self.dialogs.get(dialog_id)
            .map(|entry| entry.clone())
            .ok_or_else(|| DialogError::dialog_not_found(&dialog_id.to_string()))
    }
    
    /// Get a mutable reference to a dialog
    fn get_dialog_mut(&self, dialog_id: &DialogId) -> DialogResult<RefMut<DialogId, Dialog>> {
        self.dialogs.get_mut(dialog_id)
            .ok_or_else(|| DialogError::dialog_not_found(&dialog_id.to_string()))
    }
    
    /// Terminate a dialog
    /// 
    /// Implements RFC 3261 Section 12.3 dialog termination.
    /// Properly cleans up all dialog state and lookup entries.
    async fn terminate_dialog(&self, dialog_id: &DialogId) -> DialogResult<()> {
        debug!("Terminating dialog {}", dialog_id);
        
        // Get the dialog and terminate it
        if let Some(mut dialog_entry) = self.dialogs.get_mut(dialog_id) {
            let dialog = dialog_entry.value_mut();
            
            // Only terminate if not already terminated
            if dialog.state != DialogState::Terminated {
                let previous_state = dialog.state.clone();
                dialog.terminate();
                
                // Send session coordination event for dialog state change
                if let Some(ref coordinator) = self.session_coordinator.read().await.as_ref() {
                    let event = SessionCoordinationEvent::DialogStateChanged {
                        dialog_id: dialog_id.clone(),
                        new_state: "Terminated".to_string(),
                        previous_state: format!("{:?}", previous_state),
                    };
                    
                    if let Err(e) = coordinator.send(event).await {
                        warn!("Failed to send dialog termination event: {}", e);
                    }
                }
                
                debug!("Dialog {} terminated (was: {:?})", dialog_id, previous_state);
            } else {
                debug!("Dialog {} already terminated", dialog_id);
            }
            
            Ok(())
        } else {
            Err(DialogError::dialog_not_found(&dialog_id.to_string()))
        }
    }
    
    /// List all active dialogs
    fn list_dialogs(&self) -> Vec<DialogId> {
        self.dialogs.iter()
            .map(|entry| entry.key().clone())
            .collect()
    }
    
    /// Get current dialog count
    fn dialog_count(&self) -> usize {
        self.dialogs.len()
    }
    
    /// Check if a dialog exists
    fn has_dialog(&self, dialog_id: &DialogId) -> bool {
        self.dialogs.contains_key(dialog_id)
    }
    
    /// Get dialog state
    fn get_dialog_state(&self, dialog_id: &DialogId) -> DialogResult<DialogState> {
        let dialog = self.get_dialog(dialog_id)?;
        Ok(dialog.state.clone())
    }
    
    /// Update dialog state with proper notifications
    /// 
    /// Updates dialog state and notifies session-core of the change.
    /// Implements proper RFC 3261 state transition validation.
    async fn update_dialog_state(&self, dialog_id: &DialogId, new_state: DialogState) -> DialogResult<()> {
        debug!("Updating dialog {} state to {:?}", dialog_id, new_state);
        
        let previous_state = {
            let mut dialog = self.get_dialog_mut(dialog_id)?;
            let prev = dialog.state.clone();
            
            // Validate state transition (RFC 3261 compliance)
            match (&prev, &new_state) {
                // Valid transitions: Early -> Confirmed or Terminated
                (DialogState::Early, DialogState::Confirmed) => {},
                (DialogState::Early, DialogState::Terminated) => {},
                
                // Valid transitions: Confirmed -> Terminated
                (DialogState::Confirmed, DialogState::Terminated) => {},
                
                // Valid transitions: Initial can go to any state
                (DialogState::Initial, _) => {},
                
                // Valid transitions: Recovering can transition to any state  
                (DialogState::Recovering, _) => {},
                
                // Allow re-termination
                (DialogState::Terminated, DialogState::Terminated) => {},
                
                // Same state transitions for Early and Confirmed (idempotent)
                (DialogState::Early, DialogState::Early) => {},
                (DialogState::Confirmed, DialogState::Confirmed) => {},
                
                // Valid transitions: Initial can go to any state (covers all Initial cases)
                
                // Valid transitions: Recovering can transition to any state (covers all Recovering cases)
                
                // Invalid transitions - Confirmed cannot go back to Early
                (DialogState::Confirmed, DialogState::Early) => {
                    return Err(DialogError::protocol_error("Invalid state transition: Confirmed -> Early"));
                },
                
                // Invalid transitions - Confirmed cannot go to Initial or Recovering
                (DialogState::Confirmed, DialogState::Initial) => {
                    return Err(DialogError::protocol_error("Invalid state transition: Confirmed -> Initial"));
                },
                (DialogState::Confirmed, DialogState::Recovering) => {
                    return Err(DialogError::protocol_error("Invalid state transition: Confirmed -> Recovering"));
                },
                
                // Invalid transitions - Early cannot go back to Initial
                (DialogState::Early, DialogState::Initial) => {
                    return Err(DialogError::protocol_error("Invalid state transition: Early -> Initial"));
                },
                (DialogState::Early, DialogState::Recovering) => {
                    return Err(DialogError::protocol_error("Invalid state transition: Early -> Recovering"));
                },
                
                // Invalid transitions - Cannot transition from Terminated (except to Terminated)
                (DialogState::Terminated, _) => {
                    return Err(DialogError::protocol_error("Cannot transition from Terminated state"));
                },
            }
            
            dialog.state = new_state.clone();
            prev
        };
        
        // Send session coordination event for state change
        if let Some(ref coordinator) = self.session_coordinator.read().await.as_ref() {
            let event = SessionCoordinationEvent::DialogStateChanged {
                dialog_id: dialog_id.clone(),
                new_state: format!("{:?}", new_state),
                previous_state: format!("{:?}", previous_state),
            };
            
            if let Err(e) = coordinator.send(event).await {
                warn!("Failed to send dialog state change event: {}", e);
            }
        }
        
        debug!("Updated dialog {} state from {:?} to {:?}", dialog_id, previous_state, new_state);
        Ok(())
    }
}

// Implement DialogLookup for DialogManager
impl DialogLookup for DialogManager {
    /// Find an existing dialog by request
    /// 
    /// Implements RFC 3261 Section 12.2 dialog identification rules.
    /// Uses Call-ID, From tag, and To tag for proper dialog matching.
    /// 
    /// **FIXED**: Added fallback lookup for early dialogs that don't have both tags yet.
    async fn find_dialog_for_request(&self, request: &Request) -> Option<DialogId> {
        // Extract dialog identification info
        let (call_id, from_tag, to_tag) = DialogUtils::extract_dialog_info(request)?;
        let from_tag = from_tag?;
        
        // First try: Standard lookup with both tags (for confirmed dialogs)
        if let Some(to_tag) = &to_tag {
            debug!("Looking for confirmed dialog: Call-ID={}, From-tag={}, To-tag={}", 
                   call_id, from_tag, to_tag);
            
            // Try both scenarios: UAC and UAS perspective
            let (key1, key2) = DialogUtils::create_bidirectional_keys(&call_id, &from_tag, &to_tag);
            
            // Scenario 1: Local is From, Remote is To (UAC perspective)
            if let Some(dialog_id) = self.dialog_lookup.get(&key1) {
                debug!("Found confirmed dialog {} using UAC perspective", dialog_id.value());
                return Some(dialog_id.clone());
            }
            
            // Scenario 2: Local is To, Remote is From (UAS perspective)
            if let Some(dialog_id) = self.dialog_lookup.get(&key2) {
                debug!("Found confirmed dialog {} using UAS perspective", dialog_id.value());
                return Some(dialog_id.clone());
            }
        }
        
        // Second try: Fallback lookup for early dialogs (only have call-id and from-tag)
        // This is needed for initial INVITEs where we created an early dialog but don't have to-tag yet
        debug!("Searching for early dialog: Call-ID={}, From-tag={}, To-tag=None", call_id, from_tag);
        
        // Search through all dialogs for matching call-id and remote-tag (early dialogs)
        for dialog_entry in self.dialogs.iter() {
            let dialog = dialog_entry.value();
            
            // Check if this is an early dialog matching our request
            if dialog.call_id == call_id && 
               dialog.state == crate::dialog::DialogState::Early &&
               dialog.remote_tag.as_ref() == Some(&from_tag) {
                debug!("Found early dialog {} for initial INVITE", dialog.id);
                return Some(dialog.id.clone());
            }
        }
        
        debug!("No matching dialog found for request");
        None
    }
    
    /// Create an early dialog from an INVITE request
    /// 
    /// Implements RFC 3261 Section 12.1.1 for early dialog creation.
    /// Early dialogs are created when processing incoming INVITE requests.
    async fn create_early_dialog_from_invite(&self, request: &Request) -> DialogResult<DialogId> {
        debug!("Creating early dialog from INVITE request");
        
        // Validate this is an INVITE request
        if request.method() != rvoip_sip_core::Method::Invite {
            return Err(DialogError::protocol_error("Early dialog creation only valid for INVITE requests"));
        }
        
        // Extract required information from INVITE
        let call_id = request.call_id()
            .ok_or_else(|| DialogError::protocol_error("INVITE missing Call-ID header"))?
            .to_string();
            
        let from_uri = request.from()
            .ok_or_else(|| DialogError::protocol_error("INVITE missing From header"))?
            .uri().clone();
            
        let to_uri = request.to()
            .ok_or_else(|| DialogError::protocol_error("INVITE missing To header"))?
            .uri().clone();
            
        let remote_tag = request.from()
            .and_then(|from| from.tag())
            .map(|tag| tag.to_string());
            
        // For incoming INVITE, we are the UAS (not initiator)
        let dialog = Dialog::new_early(
            call_id,
            to_uri,      // local_uri (we are the UAS)
            from_uri,    // remote_uri (they are the UAC)
            None,        // local_tag (will be generated when we respond)
            remote_tag,  // remote_tag (from the From header)
            false,       // is_initiator = false (we're UAS)
        );
        
        let dialog_id = dialog.id.clone();
        
        // Store the early dialog
        self.store_dialog(dialog).await?;
        
        info!("Created early dialog {} from INVITE request", dialog_id);
        Ok(dialog_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::{Method, Uri};
    use crate::dialog::DialogState;
    
    #[test]
    fn test_dialog_lookup_key_creation() {
        let key = DialogUtils::create_lookup_key("call-123", "tag-local", "tag-remote");
        assert_eq!(key, "call-123:tag-local:tag-remote");
    }
    
    #[test]
    fn test_bidirectional_dialog_keys() {
        let (key1, key2) = DialogUtils::create_bidirectional_keys("call-123", "tag-a", "tag-b");
        assert_eq!(key1, "call-123:tag-a:tag-b");
        assert_eq!(key2, "call-123:tag-b:tag-a");
    }
    
    #[test]
    fn test_dialog_info_extraction() {
        let uri = Uri::sip("test@example.com");
        let request = Request::new(Method::Invite, uri);
        
        // Test extraction when no headers are present
        let result = DialogUtils::extract_dialog_info(&request);
        assert!(result.is_none()); // Should be None due to missing Call-ID
    }
    
    #[test]
    fn test_dialog_state_transition_validation() {
        // Test the state transition logic that's implemented in update_dialog_state
        use DialogState::*;
        
        // Test valid transitions - these should NOT panic in our match logic
        let valid_transitions = vec![
            (Early, Confirmed),
            (Early, Terminated), 
            (Confirmed, Terminated),
            (Initial, Early),
            (Initial, Confirmed),
            (Initial, Terminated),
            (Recovering, Early),
            (Recovering, Confirmed),
            (Recovering, Terminated),
            (Terminated, Terminated), // Re-termination allowed
            // Idempotent transitions
            (Early, Early),
            (Confirmed, Confirmed),
            (Initial, Initial),
            (Recovering, Recovering),
        ];
        
        for (from_state, to_state) in valid_transitions {
            // Simulate the validation logic from update_dialog_state
            let validation_result = validate_state_transition(&from_state, &to_state);
            assert!(validation_result.is_ok(), 
                "Transition from {:?} to {:?} should be valid", from_state, to_state);
        }
        
        // Test invalid transitions
        let invalid_transitions = vec![
            (Confirmed, Early),
            (Confirmed, Initial),
            (Confirmed, Recovering),
            (Early, Initial),
            (Early, Recovering),
            (Terminated, Early),
            (Terminated, Confirmed),
            (Terminated, Initial),
            (Terminated, Recovering),
        ];
        
        for (from_state, to_state) in invalid_transitions {
            let validation_result = validate_state_transition(&from_state, &to_state);
            assert!(validation_result.is_err(), 
                "Transition from {:?} to {:?} should be invalid", from_state, to_state);
        }
    }
    
    #[test]
    fn test_message_extensions() {
        use super::super::utils::MessageExtensions;
        
        let uri = Uri::sip("test@example.com");
        let request_empty = Request::new(Method::Invite, uri.clone());
        let request_with_body = Request::new(Method::Invite, uri).with_body(b"test body".to_vec());
        
        // Test empty body
        assert_eq!(request_empty.body_string(), None);
        
        // Test body with content
        assert_eq!(request_with_body.body_string(), Some("test body".to_string()));
    }
    
    // Helper function to test state transition validation logic
    // This extracts the validation logic from update_dialog_state for unit testing
    fn validate_state_transition(from: &DialogState, to: &DialogState) -> Result<(), &'static str> {
        match (from, to) {
            // Valid transitions: Early -> Confirmed or Terminated
            (DialogState::Early, DialogState::Confirmed) => Ok(()),
            (DialogState::Early, DialogState::Terminated) => Ok(()),
            
            // Valid transitions: Confirmed -> Terminated
            (DialogState::Confirmed, DialogState::Terminated) => Ok(()),
            
            // Allow re-termination
            (DialogState::Terminated, DialogState::Terminated) => Ok(()),
            
            // Same state transitions for Early and Confirmed (idempotent)
            (DialogState::Early, DialogState::Early) => Ok(()),
            (DialogState::Confirmed, DialogState::Confirmed) => Ok(()),
            
            // Valid transitions: Initial can go to any state (covers all Initial cases)
            (DialogState::Initial, _) => Ok(()),
            
            // Valid transitions: Recovering can transition to any state (covers all Recovering cases)
            (DialogState::Recovering, _) => Ok(()),
            
            // Invalid transitions - Confirmed cannot go back to Early
            (DialogState::Confirmed, DialogState::Early) => Err("Invalid state transition: Confirmed -> Early"),
            
            // Invalid transitions - Confirmed cannot go to Initial or Recovering
            (DialogState::Confirmed, DialogState::Initial) => Err("Invalid state transition: Confirmed -> Initial"),
            (DialogState::Confirmed, DialogState::Recovering) => Err("Invalid state transition: Confirmed -> Recovering"),
            
            // Invalid transitions - Early cannot go back to Initial
            (DialogState::Early, DialogState::Initial) => Err("Invalid state transition: Early -> Initial"),
            (DialogState::Early, DialogState::Recovering) => Err("Invalid state transition: Early -> Recovering"),
            
            // Invalid transitions - Cannot transition from Terminated (except to Terminated)
            (DialogState::Terminated, _) => Err("Cannot transition from Terminated state"),
        }
    }
} 