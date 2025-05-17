// Helper to convert transaction error to session error
use std::time::SystemTime;
use crate::errors::{Error, ErrorContext, ErrorCategory, ErrorSeverity, RecoveryAction};
use crate::dialog::{DialogId, DialogManager, Dialog, DialogState};
use crate::session::{SessionManager, SessionConfig, SessionDirection, SessionState, Session, SessionId};
use crate::sdp::SessionDescription;
use rvoip_sip_core::{Request, Response, Method, Header, Uri, StatusCode, HeaderName, TypedHeader};
use rvoip_sip_core::types::content_type::ContentType;
use rvoip_transaction_core::{TransactionKey, TransactionKind};
use std::sync::Arc;
use bytes::Bytes;
use std::str::FromStr;
use dashmap::DashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use std::collections::HashMap;
use tracing::{debug, info, warn, error};
use uuid::Uuid;
use rand::Rng;
use crate::dialog::dialog_utils::uri_resolver;
use crate::events::{EventBus, SessionEvent};
use rvoip_sip_core::types::address::Address;
use rvoip_sip_core::types::from::From as FromHeader;
use rvoip_sip_core::types::to::To as ToHeader;
use rvoip_sip_core::types::param::Param;
use rvoip_transaction_core::{TransactionManager, TransactionEvent};

/// Helper function to create a simple test SDP
#[cfg(test)]
pub fn create_test_sdp() -> SessionDescription {
    // Create a basic SDP with minimal settings for testing
    let origin = rvoip_sip_core::Origin {
        username: "test".to_string(),
        sess_id: "1234567890".to_string(), 
        sess_version: "1".to_string(),
        net_type: "IN".to_string(),
        addr_type: "IP4".to_string(),
        unicast_address: "127.0.0.1".to_string(),
    };
    
    let session_name = "Test Session";
    let mut sdp = SessionDescription::new(origin, session_name);
    
    // Add connection info
    let connection = rvoip_sip_core::ConnectionData {
        net_type: "IN".to_string(),
        addr_type: "IP4".to_string(),
        connection_address: "127.0.0.1".to_string(),
        ttl: None,
        multicast_count: None,
    };
    sdp.connection_info = Some(connection);
    
    // Add a time description
    let time = rvoip_sip_core::TimeDescription {
        start_time: "0".to_string(),
        stop_time: "0".to_string(),
        repeat_times: vec![],
    };
    sdp.time_descriptions.push(time);
    
    // Add an audio media description
    let mut media = rvoip_sip_core::MediaDescription {
        media: "audio".to_string(),
        port: 49170,
        protocol: "RTP/AVP".to_string(),
        formats: vec!["0".to_string(), "8".to_string()],
        ptime: None,
        direction: Some(rvoip_sip_core::MediaDirection::SendRecv),
        connection_info: None,
        generic_attributes: vec![],
    };
    
    // Add the media section
    sdp.media_descriptions.push(media);
    
    sdp
}

// Helper to extract content type from a request
trait RequestExt {
    fn content_type(&self) -> Result<String, ()>;
}

impl RequestExt for Request {
    fn content_type(&self) -> Result<String, ()> {
        if let Some(TypedHeader::ContentType(content_type)) = self.header(&HeaderName::ContentType) {
            return Ok(content_type.to_string());
        }
        Err(())
    }
}

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
    let mut dialog = dialog_manager.get_dialog(dialog_id)?;
    
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

/// Get the media configuration from a dialog
///
/// This is a convenience wrapper that extracts media configuration from 
/// negotiated SDP in a dialog.
pub fn get_dialog_media_config(
    dialog_manager: &Arc<DialogManager>,
    dialog_id: &DialogId
) -> Result<crate::media::MediaConfig, Error> {
    // Get the dialog
    let dialog = dialog_manager.get_dialog(dialog_id)?;
    
    // Check if SDP negotiation is complete
    if dialog.sdp_context.state != crate::sdp::NegotiationState::Complete {
        return Err(Error::InvalidDialogState {
            current: dialog.sdp_context.state.to_string(),
            expected: crate::sdp::NegotiationState::Complete.to_string(),
            context: ErrorContext {
                category: ErrorCategory::Dialog,
                severity: ErrorSeverity::Error,
                recovery: RecoveryAction::None,
                retryable: false,
                dialog_id: Some(dialog_id.to_string()),
                timestamp: SystemTime::now(),
                details: Some("Cannot extract media config: SDP negotiation not complete".to_string()),
                ..Default::default()
            }
        });
    }
    
    // Get local and remote SDP
    let local_sdp = dialog.sdp_context.local_sdp
        .as_ref()
        .ok_or_else(|| Error::InternalError(
            "Local SDP missing in complete negotiation".to_string(),
            ErrorContext {
                category: ErrorCategory::Internal,
                severity: ErrorSeverity::Error,
                recovery: RecoveryAction::None,
                retryable: false,
                dialog_id: Some(dialog_id.to_string()),
                timestamp: SystemTime::now(),
                details: Some("SDP negotiation state is Complete but local SDP is missing".to_string()),
                ..Default::default()
            }
        ))?;
        
    let remote_sdp = dialog.sdp_context.remote_sdp
        .as_ref()
        .ok_or_else(|| Error::InternalError(
            "Remote SDP missing in complete negotiation".to_string(),
            ErrorContext {
                category: ErrorCategory::Internal,
                severity: ErrorSeverity::Error,
                recovery: RecoveryAction::None,
                retryable: false,
                dialog_id: Some(dialog_id.to_string()),
                timestamp: SystemTime::now(),
                details: Some("SDP negotiation state is Complete but remote SDP is missing".to_string()),
                ..Default::default()
            }
        ))?;
    
    // Extract media config using the SDP utility function
    crate::sdp::extract_media_config(local_sdp, remote_sdp)
        .map_err(|e| Error::InternalError(
            format!("Failed to extract media config: {}", e),
            ErrorContext {
                category: ErrorCategory::Media,
                severity: ErrorSeverity::Error,
                recovery: RecoveryAction::None,
                retryable: false,
                dialog_id: Some(dialog_id.to_string()),
                timestamp: SystemTime::now(),
                details: Some(format!("Error extracting media config: {}", e)),
                ..Default::default()
            }
        ))
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
    
    // Use the new dialog manager method to create the dialog directly
    let dialog_id = dialog_manager.create_dialog_directly(
        dialog_id,
        call_id,
        local_uri,
        remote_uri,
        local_tag,
        remote_tag,
        true // Assume we're the initiator by default
    );
    
    // Associate with session and emit created event
    dialog_manager.associate_and_notify(&dialog_id, session_id)?;
    
    Ok(dialog_id)
}

/// Refresh a session dialog using re-INVITE
///
/// This is a convenience wrapper that sends a re-INVITE to refresh a dialog
/// without changing media parameters. This is useful for refreshing sessions
/// that have been established for a long time, or for updating NAT bindings.
pub async fn refresh_dialog(
    dialog_manager: &Arc<DialogManager>,
    dialog_id: &DialogId
) -> Result<(), Error> {
    // Get a reference to the dialog to verify it exists
    let dialog = dialog_manager.get_dialog(dialog_id)?;
    
    // Verify the dialog is in a state where we can send a re-INVITE
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
                details: Some("Cannot refresh dialog in non-confirmed state".to_string()),
                ..Default::default()
            }
        });
    }
    
    // Prepare for SDP renegotiation
    dialog_manager.prepare_dialog_sdp_renegotiation(dialog_id).await?;
    
    // Get the current SDP if available
    let local_sdp = match dialog.sdp_context.local_sdp {
        Some(ref sdp) => sdp.clone(),
        None => {
            // If no SDP is available, we can't do a refresh with media
            return Err(Error::InvalidMediaState {
                context: ErrorContext {
                    category: ErrorCategory::Media,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    dialog_id: Some(dialog_id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some("Cannot refresh dialog with no local SDP available".to_string()),
                    ..Default::default()
                }
            });
        }
    };
    
    // Create a refreshed SDP by manually creating a new one with updated values
    let mut refreshed_sdp = local_sdp.clone();
    
    // Update the origin version number if available
    let origin = &mut refreshed_sdp.origin;
    // Parse and increment the version
    if let Ok(version) = origin.sess_version.parse::<u64>() {
        origin.sess_version = (version + 1).to_string();
    }
    
    // Update any time fields
    if !refreshed_sdp.time_descriptions.is_empty() {
        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        
        refreshed_sdp.time_descriptions[0].start_time = current_time.to_string();
    }
    
    // Update dialog with new SDP offer
    dialog_manager.update_dialog_with_local_sdp_offer(dialog_id, refreshed_sdp).await?;
    
    // Create and send the re-INVITE request
    let transaction_id = send_dialog_request(
        dialog_manager, 
        dialog_id, 
        Method::Invite,
        None
    ).await?;
    
    Ok(())
}

/// Handle a session refresh request (incoming re-INVITE)
///
/// This is a convenience wrapper that processes an incoming re-INVITE
/// to refresh a session, responding with appropriate SDP to maintain
/// the media session.
pub async fn accept_refresh_request(
    dialog_manager: &Arc<DialogManager>,
    dialog_id: &DialogId,
    transaction_id: &TransactionKey,
    request: &Request
) -> Result<(), Error> {
    // Get a reference to the dialog to verify it exists
    let dialog = dialog_manager.get_dialog(dialog_id)?;
    
    // Verify the dialog is in a state where we can accept a refresh
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
                details: Some("Cannot accept refresh request in non-confirmed state".to_string()),
                ..Default::default()
            }
        });
    }
    
    // Extract SDP from request if available
    let remote_sdp = if let Ok(content_type) = request.content_type() {
        if content_type == "application/sdp" {
            if let Ok(sdp_str) = std::str::from_utf8(&request.body) {
                match crate::sdp::SessionDescription::from_str(sdp_str) {
                    Ok(sdp) => Some(sdp),
                    Err(_) => None
                }
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };
    
    // Process remote SDP if available
    if let Some(remote_sdp) = remote_sdp {
        // If we have a local SDP, we can create an answer
        if let Some(ref local_sdp) = dialog.sdp_context.local_sdp {
            // Create a refreshed SDP answer based on original local SDP
            let mut refreshed_sdp = local_sdp.clone();
            
            // Update the origin version number if available
            let origin = &mut refreshed_sdp.origin;
            // Parse and increment the version
            if let Ok(version) = origin.sess_version.parse::<u64>() {
                origin.sess_version = (version + 1).to_string();
            }
            
            // Update any time fields
            if !refreshed_sdp.time_descriptions.is_empty() {
                let current_time = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                
                refreshed_sdp.time_descriptions[0].start_time = current_time.to_string();
            }
            
            // Update dialog with remote SDP offer
            let mut dialog_context = dialog.sdp_context.clone();
            dialog_context.update_with_remote_offer(remote_sdp);
            
            // Create a response with our SDP answer
            let mut response = Response::new(StatusCode::Ok);
            
            // Add required headers for response
            // Call-ID, From, To, CSeq will be added by the transaction layer
            
            // Add Contact header
            let contact_uri = dialog.local_uri.clone();
            let contact_addr = rvoip_sip_core::types::address::Address::new(contact_uri);
            let contact_param = rvoip_sip_core::types::contact::ContactParamInfo { address: contact_addr };
            let contact = rvoip_sip_core::types::contact::Contact::new_params(vec![contact_param]);
            response.headers.push(rvoip_sip_core::TypedHeader::Contact(contact));
            
            // Add SDP body and Content-Type
            response.body = Bytes::from(refreshed_sdp.to_string().into_bytes());
            response.headers.push(rvoip_sip_core::TypedHeader::ContentType(
                ContentType::from_str("application/sdp").unwrap()
            ));
            
            // Send the response
            if let Err(e) = dialog_manager.send_response(transaction_id, response).await {
                return Err(Error::TransactionError(
                    e,
                    ErrorContext {
                        category: ErrorCategory::Protocol,
                        severity: ErrorSeverity::Error,
                        recovery: RecoveryAction::None,
                        retryable: true,
                        dialog_id: Some(dialog_id.to_string()),
                        transaction_id: Some(transaction_id.to_string()),
                        timestamp: SystemTime::now(),
                        details: Some("Failed to send response to re-INVITE".to_string()),
                        ..Default::default()
                    }
                ));
            }
            
            // Update dialog with local SDP answer
            dialog_manager.update_dialog_with_local_sdp_answer(dialog_id, refreshed_sdp).await?;
            
            Ok(())
        } else {
            // No local SDP, send error response
            let response = Response::new(StatusCode::NotAcceptable);
            
            if let Err(e) = dialog_manager.send_response(transaction_id, response).await {
                return Err(Error::TransactionError(
                    e,
                    ErrorContext {
                        category: ErrorCategory::Protocol,
                        severity: ErrorSeverity::Error,
                        recovery: RecoveryAction::None,
                        retryable: true,
                        dialog_id: Some(dialog_id.to_string()),
                        transaction_id: Some(transaction_id.to_string()),
                        timestamp: SystemTime::now(),
                        details: Some("Failed to send error response to re-INVITE".to_string()),
                        ..Default::default()
                    }
                ));
            }
            
            Err(Error::InvalidMediaState {
                context: ErrorContext {
                    category: ErrorCategory::Media,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    dialog_id: Some(dialog_id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some("Cannot accept refresh with no local SDP available".to_string()),
                    ..Default::default()
                }
            })
        }
    } else {
        // No SDP in request, send 200 OK
        let response = Response::new(StatusCode::Ok);
        
        if let Err(e) = dialog_manager.send_response(transaction_id, response).await {
            return Err(Error::TransactionError(
                e,
                ErrorContext {
                    category: ErrorCategory::Protocol,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: true,
                    dialog_id: Some(dialog_id.to_string()),
                    transaction_id: Some(transaction_id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some("Failed to send response to re-INVITE".to_string()),
                    ..Default::default()
                }
            ));
        }
        
        Ok(())
    }
}

/// Attempt to recover a dialog after a network failure
///
/// This function will initiate recovery of a dialog that has encountered network connectivity
/// issues. It uses the built-in recovery mechanism to attempt to re-establish the dialog.
///
/// # Arguments
///
/// * `dialog_manager` - The dialog manager instance
/// * `dialog_id` - The ID of the dialog to recover
/// * `reason` - A description of why recovery is needed
///
/// # Returns
///
/// A Result indicating success or failure of initiating the recovery process
///
/// # Example
///
/// ```no_run
/// use rvoip_session_core::helpers::attempt_dialog_recovery;
/// use rvoip_session_core::dialog::{DialogManager, DialogId};
///
/// async fn recover_dialog(dialog_manager: &DialogManager, dialog_id: &DialogId) {
///     match attempt_dialog_recovery(dialog_manager, dialog_id, "Network connectivity loss").await {
///         Ok(_) => println!("Recovery process started"),
///         Err(e) => println!("Failed to start recovery: {}", e),
///     }
/// }
/// ```
pub async fn attempt_dialog_recovery(
    dialog_manager: &DialogManager,
    dialog_id: &DialogId,
    reason: &str
) -> Result<(), Error> {
    // Check if dialog is in a state where it can be recovered
    if dialog_manager.needs_recovery(dialog_id).await {
        // Initiate recovery process
        dialog_manager.recover_dialog(dialog_id, reason).await
    } else {
        // Dialog can't be recovered
        Err(Error::InvalidDialogState {
            current: "Unknown".to_string(),
            expected: "Confirmed or Early".to_string(),
            context: ErrorContext {
                category: ErrorCategory::Dialog,
                severity: ErrorSeverity::Warning,
                recovery: RecoveryAction::None,
                retryable: false,
                dialog_id: Some(dialog_id.to_string()),
                timestamp: std::time::SystemTime::now(),
                details: Some("Dialog is not in a recoverable state".to_string()),
                ..Default::default()
            }
        })
    }
}

/// Send an UPDATE request to modify an established dialog without alerting the user
///
/// This function sends an UPDATE request as defined in RFC 3311 to modify an established dialog.
/// Unlike re-INVITE, UPDATE doesn't alert the user and can be used for mid-dialog session modifications.
/// It's particularly useful for refreshing session timers or modifying media parameters during a call.
///
/// # Arguments
///
/// * `dialog_manager` - The dialog manager instance
/// * `dialog_id` - The ID of the dialog to update
/// * `sdp` - Optional new SDP description for media modification (if None, no media changes)
///
/// # Returns
///
/// A Result containing the transaction key if successful, or error information
///
/// # Example
///
/// ```no_run
/// use rvoip_session_core::helpers::send_update_request;
/// use rvoip_session_core::dialog::{DialogManager, DialogId};
/// use rvoip_session_core::sdp::SessionDescription;
///
/// async fn modify_session(dialog_manager: &DialogManager, dialog_id: &DialogId, sdp: SessionDescription) {
///     match send_update_request(dialog_manager, dialog_id, Some(sdp)).await {
///         Ok(tx) => println!("UPDATE sent with transaction {}", tx),
///         Err(e) => println!("Failed to send UPDATE: {}", e),
///     }
/// }
/// ```
pub async fn send_update_request(
    dialog_manager: &DialogManager,
    dialog_id: &DialogId,
    sdp: Option<SessionDescription>
) -> Result<TransactionKey, Error> {
    // Verify the dialog is in a state where we can send UPDATE
    let mut dialog = dialog_manager.get_dialog(dialog_id)?;
    
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
                timestamp: std::time::SystemTime::now(),
                details: Some("Cannot send UPDATE in non-confirmed dialog".to_string()),
                ..Default::default()
            }
        });
    }
    
    // Get the base dialog request - using Method::Update directly instead of Method::Invite
    let base_request = dialog.create_request(Method::Update);
    
    // Create an UPDATE request using the transaction-core utilities
    let mut update_request = match rvoip_transaction_core::method::update::create_update_request(
        &base_request,
        &"0.0.0.0:0".parse().unwrap(), // Local address (not used for internal dialog)
        None // No SDP initially
    ) {
        Ok(req) => req,
        Err(e) => return Err(Error::TransactionError(
            e,
            ErrorContext {
                category: ErrorCategory::Dialog,
                severity: ErrorSeverity::Error,
                recovery: RecoveryAction::None,
                retryable: false,
                dialog_id: Some(dialog_id.to_string()),
                timestamp: std::time::SystemTime::now(),
                details: Some("Failed to create UPDATE request".to_string()),
                ..Default::default()
            }
        )),
    };
    
    // If SDP is provided, include it in the request and update dialog state
    if let Some(sdp_desc) = sdp {
        // Update dialog with local SDP offer
        dialog_manager.update_dialog_with_local_sdp_offer(dialog_id, sdp_desc.clone()).await?;
        
        // Add SDP to the UPDATE request
        let sdp_str = sdp_desc.to_string();
        
        // Add Content-Type header for SDP
        update_request.headers.push(TypedHeader::ContentType(
            rvoip_sip_core::types::content_type::ContentType::from_str("application/sdp").unwrap()
        ));
        
        // Add Content-Length header
        update_request.headers.push(TypedHeader::ContentLength(
            rvoip_sip_core::types::content_length::ContentLength::new(sdp_str.len() as u32)
        ));
        
        // Set the body to the SDP
        update_request.body = Bytes::from(sdp_str.into_bytes());
    } else {
        // No SDP, set Content-Length to 0
        update_request.headers.push(TypedHeader::ContentLength(
            rvoip_sip_core::types::content_length::ContentLength::new(0)
        ));
    }
    
    // Resolve the URI to get the destination address
    let remote_target = dialog.remote_target.clone();
    let destination = match uri_resolver::resolve_uri_to_socketaddr(&remote_target).await {
        Some(addr) => addr,
        None => return Err(Error::network_unreachable(&remote_target.to_string())),
    };
    
    // Create a client transaction for this request using dialog_manager's internal APIs
    // We'd ideally want to use the public methods, but for now we'll use send_dialog_request directly
    
    // Send the request via the dialog manager
    dialog_manager.send_dialog_request(dialog_id, Method::Update).await
}

/// Accept an incoming UPDATE request with an optional SDP answer
///
/// This function generates a 200 OK response to an incoming UPDATE request and
/// accepts any proposed media changes by including an SDP answer if needed.
///
/// # Arguments
///
/// * `dialog_manager` - The dialog manager instance
/// * `transaction_id` - The transaction ID of the incoming UPDATE request
/// * `sdp` - Optional SDP answer (required if the UPDATE contained an SDP offer)
///
/// # Returns
///
/// A Result indicating success or failure
///
/// # Example
///
/// ```no_run
/// use rvoip_session_core::helpers::accept_update_request;
/// use rvoip_session_core::dialog::DialogManager;
/// use rvoip_transaction_core::TransactionKey;
/// use rvoip_session_core::sdp::SessionDescription;
///
/// async fn handle_update(
///     dialog_manager: &DialogManager, 
///     transaction_id: &TransactionKey,
///     sdp: SessionDescription
/// ) {
///     match accept_update_request(dialog_manager, transaction_id, Some(sdp)).await {
///         Ok(_) => println!("UPDATE accepted"),
///         Err(e) => println!("Failed to accept UPDATE: {}", e),
///     }
/// }
/// ```
pub async fn accept_update_request(
    dialog_manager: &DialogManager,
    transaction_id: &TransactionKey,
    sdp: Option<SessionDescription>
) -> Result<(), Error> {
    // Find dialog associated with this transaction
    // In an actual implementation, we would use dialog_manager's API to find the dialog
    // For now, this will need to wait until we refactor the DialogManager to expose this functionality
    
    // For placeholder implementation, let's create a 200 OK response to the UPDATE
    let mut response = Response::new(StatusCode::Ok);
    
    // If SDP answer is provided, add it to the response
    if let Some(sdp_answer) = sdp {
        // Add SDP body
        let sdp_str = sdp_answer.to_string();
        
        // Add Content-Type header for SDP
        response.headers.push(TypedHeader::ContentType(
            rvoip_sip_core::types::content_type::ContentType::from_str("application/sdp").unwrap()
        ));
        
        // Add Content-Length header
        response.headers.push(TypedHeader::ContentLength(
            rvoip_sip_core::types::content_length::ContentLength::new(sdp_str.len() as u32)
        ));
        
        // Set the body to the SDP
        response.body = Bytes::from(sdp_str.into_bytes());
    } else {
        // No SDP, set Content-Length to 0
        response.headers.push(TypedHeader::ContentLength(
            rvoip_sip_core::types::content_length::ContentLength::new(0)
        ));
    }
    
    // For now, we don't have direct access to send the response, but we'll create the interface
    // that we'd want to have. In the future, DialogManager should expose this functionality.
    Err(Error::Unsupported {
        feature: "UPDATE Method Response".to_string(),
        context: ErrorContext {
            category: ErrorCategory::Dialog,
            severity: ErrorSeverity::Warning,
            recovery: RecoveryAction::None,
            retryable: false,
            transaction_id: Some(transaction_id.to_string()),
            timestamp: std::time::SystemTime::now(),
            details: Some("UPDATE method support is not yet fully implemented".to_string()),
            ..Default::default()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use rvoip_sip_core::{Uri, Method};
    use crate::dialog::{DialogManager, DialogState};
    use crate::events::EventBus;
    use crate::session::{SessionId, SessionManager};
    use tokio::sync::mpsc;
    use std::fmt;
    
    // Create a simple mock transport for testing
    #[derive(Debug)]
    struct MockTransport {
        // Flag to indicate if sending messages should fail
        should_fail_send: Arc<AtomicBool>,
        // Channel to emit transport events
        transport_tx: Option<mpsc::Sender<rvoip_sip_transport::TransportEvent>>,
    }
    
    impl MockTransport {
        fn new() -> Self {
            Self {
                should_fail_send: Arc::new(AtomicBool::new(false)),
                transport_tx: None,
            }
        }
        
        fn with_send_failure(should_fail: bool) -> Self {
            Self {
                should_fail_send: Arc::new(AtomicBool::new(should_fail)),
                transport_tx: None,
            }
        }
        
        // Method to change failure behavior during tests
        fn set_send_failure(&self, should_fail: bool) {
            self.should_fail_send.store(should_fail, Ordering::SeqCst);
        }
        
        // Method to set a transport events channel
        fn with_transport_events(mut self, tx: mpsc::Sender<rvoip_sip_transport::TransportEvent>) -> Self {
            self.transport_tx = Some(tx);
            self
        }
    }
    
    #[async_trait::async_trait]
    impl rvoip_sip_transport::Transport for MockTransport {
        fn local_addr(&self) -> Result<std::net::SocketAddr, rvoip_sip_transport::error::Error> {
            Ok("127.0.0.1:5060".parse().unwrap())
        }
        
        async fn send_message(
            &self, 
            message: rvoip_sip_core::Message, 
            destination: std::net::SocketAddr
        ) -> Result<(), rvoip_sip_transport::error::Error> {
            if self.should_fail_send.load(Ordering::SeqCst) {
                // Create the error
                let error = rvoip_sip_transport::error::Error::ConnectionFailed(
                    "Simulated network failure for testing".into()
                );
                
                // Emit a transport error event if we have a channel
                if let Some(tx) = &self.transport_tx {
                    // Create transport error event with just the error message
                    let _ = tx.send(rvoip_sip_transport::TransportEvent::Error {
                        error: format!("Simulated network failure: {}", error),
                    }).await;
                }
                
                // Return the error
                Err(error)
            } else {
                Ok(())
            }
        }
        
        async fn close(&self) -> Result<(), rvoip_sip_transport::error::Error> {
            Ok(())
        }
        
        fn is_closed(&self) -> bool {
            false
        }
    }

    // Helper to create a test dialog manager
    async fn create_test_dialog_manager() -> Arc<DialogManager> {
        create_test_dialog_manager_with_options(false).await
    }
    
    // Helper to create a test dialog manager with custom transport options
    async fn create_test_dialog_manager_with_options(should_fail_send: bool) -> Arc<DialogManager> {
        let event_bus = EventBus::new(100);
        
        // Create a channel for transport events
        let (transport_tx, transport_rx) = mpsc::channel::<rvoip_sip_transport::TransportEvent>(10);
        
        // Create the transport with failure configuration and transport events channel
        let transport = Arc::new(
            MockTransport::with_send_failure(should_fail_send)
                .with_transport_events(transport_tx)
        );
        
        // Create the transaction manager with the transport events receiver
        let transaction_manager = rvoip_transaction_core::TransactionManager::new(
            transport,
            transport_rx,
            None, // Default max transactions
        ).await.unwrap().0;
        
        let tm = Arc::new(transaction_manager);
        
        Arc::new(DialogManager::new(tm, event_bus))
    }

    // Simple unit test for UPDATE method creation
    #[test]
    fn test_update_method_basics() {
        // Test basic UPDATE request creation
        let mut dialog = Dialog {
            id: DialogId::new(),
            state: DialogState::Confirmed,
            call_id: "test-call-update".to_string(),
            local_uri: Uri::sip("alice@example.com"),
            remote_uri: Uri::sip("bob@example.com"),
            local_tag: Some("alice-tag".to_string()),
            remote_tag: Some("bob-tag".to_string()),
            local_seq: 1,
            remote_seq: 0,
            remote_target: Uri::sip("bob@example.com"),
            route_set: Vec::new(),
            is_initiator: true,
            sdp_context: crate::sdp::SdpContext::new(),
            last_known_remote_addr: None,
            last_successful_transaction_time: None,
            recovery_attempts: 0,
            recovery_reason: None,
            recovered_at: None,
            recovery_start_time: None,
        };
        
        // Create an UPDATE request
        let update_req = dialog.create_request(Method::Update);
        
        // Verify it's an UPDATE method
        assert_eq!(update_req.method, Method::Update);
        
        // Verify it has the expected headers
        assert!(update_req.header(&rvoip_sip_core::HeaderName::CallId).is_some());
        assert!(update_req.header(&rvoip_sip_core::HeaderName::From).is_some());
        assert!(update_req.header(&rvoip_sip_core::HeaderName::To).is_some());
        
        // Verify CSeq method and number
        if let Some(rvoip_sip_core::TypedHeader::CSeq(cseq)) = update_req.header(&rvoip_sip_core::HeaderName::CSeq) {
            assert_eq!(cseq.method().to_string(), Method::Update.to_string());
            assert_eq!(cseq.sequence(), 2); // Should be incremented from 1
        } else {
            panic!("Missing CSeq header");
        }
    }
} 