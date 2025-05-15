// Helper to convert transaction error to session error
use std::time::SystemTime;
use crate::errors::{Error, ErrorContext, ErrorCategory, ErrorSeverity, RecoveryAction};
use crate::dialog::{DialogId, DialogManager, Dialog, DialogState};
use crate::session::{SessionManager, SessionConfig, SessionDirection, SessionState, Session, SessionId};
use crate::sdp::SessionDescription;
use rvoip_sip_core::{Request, Response, Method, Header, Uri};
use rvoip_sip_core::types::content_type::ContentType;
use rvoip_transaction_core::TransactionKey;
use std::sync::Arc;
use bytes::Bytes;
use std::str::FromStr;

/// Create a dialog not found error with proper context
pub fn dialog_not_found_error(dialog_id: &DialogId) -> Error {
    Error::DialogNotFoundWithId(
        dialog_id.to_string(),
        ErrorContext {
            category: ErrorCategory::Dialog,
            severity: ErrorSeverity::Error,
            recovery: RecoveryAction::None,
            retryable: false,
            dialog_id: Some(dialog_id.to_string()),
            timestamp: SystemTime::now(),
            details: Some(format!("Dialog {} not found", dialog_id)),
            ..Default::default()
        }
    )
}

/// Create a transaction creation error
pub fn transaction_creation_error(method: &str, error_msg: &str) -> Error {
    Error::TransactionCreationFailed(
        method.to_string(),
        None,
        ErrorContext {
            category: ErrorCategory::Protocol,
            severity: ErrorSeverity::Error,
            recovery: RecoveryAction::Retry,
            retryable: true,
            timestamp: SystemTime::now(),
            details: Some(error_msg.to_string()),
            ..Default::default()
        }
    )
}

/// Create a transaction send error
pub fn transaction_send_error(error_msg: &str, transaction_id: &str) -> Error {
    Error::TransactionError(
        rvoip_transaction_core::Error::Other(error_msg.to_string()),
        ErrorContext {
            category: ErrorCategory::Protocol,
            severity: ErrorSeverity::Error,
            recovery: RecoveryAction::Retry,
            retryable: true,
            transaction_id: Some(transaction_id.to_string()),
            timestamp: SystemTime::now(),
            details: Some(error_msg.to_string()),
            ..Default::default()
        }
    )
}

/// Create a network unreachable error
pub fn network_unreachable_error(target: &str) -> Error {
    Error::NetworkUnreachable(
        target.to_string(),
        ErrorContext {
            category: ErrorCategory::Network,
            severity: ErrorSeverity::Error,
            recovery: RecoveryAction::Wait(std::time::Duration::from_secs(5)),
            retryable: true,
            timestamp: SystemTime::now(),
            details: Some(format!("Network unreachable: {}", target)),
            ..Default::default()
        }
    )
}

/// Create a new outgoing call (convenience wrapper)
pub async fn make_call(
    session_manager: &Arc<SessionManager>,
    destination: Uri
) -> Result<Arc<crate::Session>, Error> {
    // Create a new outgoing session
    let session = session_manager.create_outgoing_session().await?;
    
    // Set initial state
    let _ = session.set_state(SessionState::Dialing).await?;
    
    // Return the session
    Ok(session)
}

/// Answer an incoming call (convenience wrapper)
pub async fn answer_call(
    session: &Arc<crate::Session>
) -> Result<(), Error> {
    // Check current state
    let current_state = session.state().await;
    if current_state != SessionState::Ringing {
        return Err(Error::InvalidSessionStateTransition { 
            from: current_state.to_string(), 
            to: SessionState::Connected.to_string(),
            context: crate::errors::ErrorContext::default()
        });
    }
    
    // Set connected state
    session.set_state(SessionState::Connected).await?;
    
    Ok(())
}

/// End a call (convenience wrapper)
pub async fn end_call(
    session: &Arc<crate::Session>
) -> Result<(), Error> {
    // Set terminating state
    let _ = session.set_state(SessionState::Terminating).await;
    
    // Then set terminated state
    session.set_state(SessionState::Terminated).await?;
    
    Ok(())
}

/// Create a dialog from an incoming INVITE request
/// 
/// This is a convenience wrapper that creates a dialog from an INVITE request
/// and associates it with the given session.
pub async fn create_dialog_from_invite(
    dialog_manager: &Arc<DialogManager>,
    transaction_id: &TransactionKey,
    request: &Request,
    response: &Response,
    session_id: &SessionId,
    is_initiator: bool
) -> Result<DialogId, Error> {
    // Attempt to create the dialog
    if let Some(dialog_id) = dialog_manager.create_dialog_from_transaction(
        transaction_id,
        request,
        response,
        is_initiator
    ).await {
        // Associate with session
        dialog_manager.associate_with_session(&dialog_id, session_id)?;
        
        Ok(dialog_id)
    } else {
        Err(Error::DialogCreationFailed(
            "Failed to create dialog from transaction".to_string(),
            ErrorContext {
                category: ErrorCategory::Dialog,
                severity: ErrorSeverity::Error,
                recovery: RecoveryAction::None,
                retryable: false,
                transaction_id: Some(transaction_id.to_string()),
                timestamp: SystemTime::now(),
                details: Some("Failed to create dialog from transaction".to_string()),
                ..Default::default()
            }
        ))
    }
}

/// Send a request within an existing dialog
///
/// This is a convenience wrapper that creates and sends a request within
/// an existing dialog, handling the transaction creation.
pub async fn send_dialog_request(
    dialog_manager: &Arc<DialogManager>,
    dialog_id: &DialogId,
    method: Method,
    custom_headers: Option<Vec<rvoip_sip_core::TypedHeader>>
) -> Result<TransactionKey, Error> {
    // Get a reference to the dialog to verify it exists
    let _dialog = dialog_manager.get_dialog(dialog_id)?;
    
    // Create the request
    let mut request = dialog_manager.create_request(dialog_id, method.clone()).await?;
    
    // Add any custom headers if provided
    if let Some(headers) = custom_headers {
        for header in headers {
            request.headers.push(header);
        }
    }
    
    // Send the request through the dialog
    dialog_manager.send_dialog_request(dialog_id, method).await
}

/// Terminate a dialog with a specific reason
///
/// This is a convenience wrapper that terminates a dialog and
/// emits the appropriate events.
pub async fn terminate_dialog(
    dialog_manager: &Arc<DialogManager>,
    dialog_id: &DialogId,
    reason: Option<String>
) -> Result<(), Error> {
    // Terminate the dialog
    dialog_manager.terminate_dialog(dialog_id).await?;
    
    // Clean up any terminated dialogs
    let _ = dialog_manager.cleanup_terminated();
    
    Ok(())
}

/// Update dialog media parameters (for re-INVITE scenarios)
///
/// This is a convenience wrapper that sends a re-INVITE with new media parameters.
pub async fn update_dialog_media(
    dialog_manager: &Arc<DialogManager>,
    dialog_id: &DialogId,
    new_sdp: SessionDescription
) -> Result<TransactionKey, Error> {
    // Get a reference to the dialog
    let dialog = dialog_manager.get_dialog(dialog_id)?;
    
    // Verify the dialog is in a state where we can update media
    if dialog.state != DialogState::Confirmed {
        return Err(Error::InvalidDialogState {
            current: dialog.state.to_string(),
            expected: "Confirmed".to_string(),
            context: ErrorContext {
                category: ErrorCategory::Dialog,
                severity: ErrorSeverity::Error,
                recovery: RecoveryAction::None,
                retryable: false,
                dialog_id: Some(dialog_id.to_string()),
                timestamp: SystemTime::now(),
                details: Some("Cannot update media in non-confirmed dialog".to_string()),
                ..Default::default()
            }
        });
    }
    
    // Create a new INVITE request (re-INVITE)
    let mut request = dialog_manager.create_request(dialog_id, Method::Invite).await?;
    
    // Set the SDP as the message body with proper Content-Type
    let sdp_string = new_sdp.to_string();
    request.body = Bytes::from(sdp_string.into_bytes());
    request.headers.push(rvoip_sip_core::TypedHeader::ContentType(
        ContentType::from_str("application/sdp").unwrap()
    ));
    
    // Send the re-INVITE request
    dialog_manager.send_dialog_request(dialog_id, Method::Invite).await
}

/// Create a new dialog directly (for testing or advanced scenarios)
///
/// This is useful for creating dialogs outside the normal INVITE flow,
/// such as when reconstructing dialogs from stored state.
pub fn create_dialog(
    dialog_manager: &Arc<DialogManager>,
    call_id: String,
    local_uri: Uri,
    remote_uri: Uri,
    local_tag: Option<String>,
    remote_tag: Option<String>,
    session_id: &SessionId
) -> Result<DialogId, Error> {
    // Create a new dialog ID
    let dialog_id = DialogId::new();
    
    // Create a new dialog with remote URI cloned for the remote target field
    let remote_target = remote_uri.clone();
    
    // Create a new dialog
    let dialog = Dialog {
        id: dialog_id.clone(),
        state: DialogState::Confirmed,
        call_id,
        local_uri,
        remote_uri,
        local_tag,
        remote_tag,
        local_seq: 1,  // Initialize at 1 for first request
        remote_seq: 0, // Will be set when receiving a request
        remote_target, // Use remote URI as target initially
        route_set: Vec::new(),
        is_initiator: true, // Assume we're the initiator by default
    };
    
    // In a real implementation we'd add the dialog to the dialog manager directly
    // but since we don't have a public method for that, we'll return an error 
    Err(Error::InternalError(
        "The create_dialog function is not fully implemented yet".to_string(),
        ErrorContext {
            category: ErrorCategory::Internal,
            severity: ErrorSeverity::Error,
            recovery: RecoveryAction::None,
            retryable: false,
            timestamp: SystemTime::now(),
            details: Some("Use create_dialog_from_invite with a request and response instead".to_string()),
            ..Default::default()
        }
    ))
} 