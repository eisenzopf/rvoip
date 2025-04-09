use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use rvoip_sip_core::{
    Request, Response, Message, Method, StatusCode,
    Uri, Header, HeaderName, HeaderValue
};
use rvoip_transaction_core::TransactionManager;
use rvoip_session_core::sdp::SessionDescription;
use rvoip_session_core::dialog::DialogState;

use crate::call::{Call, CallState, CallEvent, CallDirection};
use crate::config::ClientConfig;
use crate::error::{Error, Result};
use crate::call_registry::CallRegistry;

use super::utils::{add_response_headers, extract_uri_from_header};

/// Handle an incoming SIP request
pub async fn handle_incoming_request(
    request: Request,
    source: SocketAddr,
    transaction_manager: Arc<TransactionManager>,
    active_calls: Arc<RwLock<HashMap<String, Arc<Call>>>>,
    event_tx: mpsc::Sender<CallEvent>,
    config: &ClientConfig,
    call_registry: Arc<CallRegistry>,
) -> Result<()> {
    debug!("Handling incoming {} request from {}", request.method, source);
    
    // Extract Call-ID
    let call_id = match request.call_id() {
        Some(id) => id.to_string(),
        None => return Err(Error::SipProtocol("Request missing Call-ID".into())),
    };
    
    // Log message receipt for debugging
    debug!("Received {} for call {}: {:?}", request.method, call_id, request);
    
    // Check for existing call using the SIP call ID
    let calls_read = active_calls.read().await;
    let existing_call = calls_read.get(&call_id).cloned();
    
    if existing_call.is_none() {
        debug!("No existing call found with call_id={}, known calls: {:?}", 
               call_id, calls_read.keys().collect::<Vec<_>>());
    } else {
        debug!("Found existing call with call_id={}", call_id);
    }
    
    drop(calls_read);
    
    // Handling INVITE requests
    if request.method == Method::Invite && existing_call.is_none() {
        debug!("Processing new INVITE request from {}", source);

        // Extract From URI for caller identification
        let from_uri = match extract_uri_from_header(&request, HeaderName::From) {
            Some(uri) => uri,
            None => return Err(Error::SipProtocol("Missing From URI".into())),
        };

        // Extract From tag (IMPORTANT - extract it from the header for dialog setup)
        let from_tag = match request.headers.iter()
            .find(|h| h.name == HeaderName::From)
            .and_then(|h| h.value.as_text())
            .and_then(|v| rvoip_session_core::dialog::extract_tag(v)) {
            Some(tag) => tag,
            None => return Err(Error::SipProtocol("Missing From tag".into())),
        };

        debug!("Extracted From tag: {}", from_tag);

        // Extract To URI
        let to_uri = match extract_uri_from_header(&request, HeaderName::To) {
            Some(uri) => uri,
            None => return Err(Error::SipProtocol("Missing To URI".into())),
        };

        // Get To header value for tag
        let to_header_value = match request.headers.iter()
            .find(|h| h.name == HeaderName::To)
            .and_then(|h| h.value.as_text()) {
            Some(value) => value.to_string(),
            None => return Err(Error::SipProtocol("Missing To header".into())),
        };

        // Generate tag for To header
        let to_tag = format!("tag-{}", Uuid::new_v4());
        let to_with_tag = format!("{};tag={}", to_header_value, to_tag);

        info!("Processing INVITE for call {}", call_id);

        // Debug SDP content if present
        if !request.body.is_empty() {
            if let Ok(sdp_str) = std::str::from_utf8(&request.body) {
                debug!("Received SDP in INVITE:\n{}", sdp_str);
            } else {
                warn!("INVITE contains body but it's not valid UTF-8");
            }
        } else {
            warn!("INVITE request has no SDP body");
        }

        // Create call config from client config
        let call_config = crate::config::CallConfig {
            audio_enabled: config.media.rtp_enabled,
            video_enabled: false,
            dtmf_enabled: true,
            auto_answer: false,
            auto_answer_delay: std::time::Duration::from_secs(0),
            call_timeout: std::time::Duration::from_secs(60),
            media: Some(config.media.clone()),
            auth_username: None,
            auth_password: None,
            display_name: None,
            rtp_port_range_start: config.media.rtp_port_min.into(),
            rtp_port_range_end: config.media.rtp_port_max.into(),
        };

        // Create call with auto-generated ID
        let (call, state_tx) = Call::new(
            CallDirection::Incoming,
            call_config,
            call_id.clone(),
            to_tag,
            to_uri,
            from_uri.clone(),
            source,
            transaction_manager.clone(),
            event_tx.clone(),
        );

        // Set the remote tag extracted from the From header
        call.set_remote_tag(from_tag).await;

        // Send a ringing response
        let mut ringing_response = Response::new(StatusCode::Ringing);
        add_response_headers(&request, &mut ringing_response);

        // Add To header with tag
        ringing_response.headers.push(Header::text(HeaderName::To, to_with_tag.clone()));

        debug!("Sending 180 Ringing for call {}", call_id);

        // Send 180 Ringing
        if let Err(e) = transaction_manager.transport().send_message(
            Message::Response(ringing_response),
            source
        ).await {
            warn!("Failed to send 180 Ringing: {}", e);
        }

        // Update call state to ringing using proper transition method
        if let Err(e) = call.transition_to(CallState::Ringing).await {
            error!("Failed to update call state to Ringing: {}", e);
        } else {
            debug!("Call {} state updated to Ringing", call_id);
        }

        // Store call - important that we register the call before sending events
        // First add to active calls - use SIP call ID for consistent lookup
        let sip_call_id = call.sip_call_id().to_string();
        active_calls.write().await.insert(sip_call_id.clone(), call.clone());

        // Before sending the IncomingCall event, manually register with call registry to avoid race conditions
        debug!("Registering call with registry: id={}, sip_call_id={}", call.id(), sip_call_id);
        if let Err(e) = call_registry.register_call(call.clone()).await {
            error!("Failed to register call in registry: {}", e);
        }

        // Store the original INVITE request for later answering
        if let Err(e) = call.store_invite_request(request.clone()).await {
            warn!("Failed to store INVITE request: {}", e);
        } else {
            debug!("Stored original INVITE request for later answering");
        }

        // Send event - this will trigger registry update via event handler
        debug!("About to send IncomingCall event for call {} to application", call_id);
        if let Err(e) = event_tx.send(CallEvent::IncomingCall(call.clone())).await
            .map_err(|_| Error::Call("Failed to send call event".into())) {
            error!("Failed to send IncomingCall event: {}", e);
        } else {
            debug!("Sent IncomingCall event for call {} to application", call_id);
            debug!("Storing weak reference to call {}", call_id);
            let weak_call = call.weak_clone();
        }

        // If auto-answer is enabled, answer the call after sending the event
        if config.media.auto_answer {
            debug!("Auto-answer is enabled in config, will proceed to answer call {}", call_id);
            
            // Extract remote SDP
            if !request.body.is_empty() {
                match std::str::from_utf8(&request.body)
                    .map_err(|_| Error::SipProtocol("Invalid UTF-8 in SDP".into()))
                    .and_then(|sdp_str| SessionDescription::parse(sdp_str)
                        .map_err(|e| Error::SipProtocol(format!("Invalid SDP: {}", e))))
                {
                    Ok(remote_sdp) => {
                        debug!("Successfully parsed SDP from INVITE");
                        
                        // Store SDP in the call for media setup
                        if let Err(e) = call.setup_media_from_sdp(&remote_sdp).await {
                            warn!("Error setting up media from SDP: {}", e);
                        }
                    },
                    Err(e) => {
                        warn!("Failed to parse SDP: {}", e);
                        debug!("SDP content that failed to parse: {:?}", 
                            String::from_utf8_lossy(&request.body));
                    }
                }
            } else {
                debug!("No SDP body in INVITE, skipping SDP parsing");
            }
            
            // Give application a chance to handle the IncomingCall event first
            debug!("Waiting before auto-answering to allow application time to process event");
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            
            // Check if call is still in ringing state (not answered or rejected by application)
            let current_state = call.state().await;
            debug!("Pre-answer call state: {}", current_state);
            
            if current_state == CallState::Ringing {
                debug!("Call still in Ringing state, proceeding with auto-answer");
                
                // Ensure we have the SIP call ID before answering
                let sip_call_id = call.sip_call_id().to_string();
                debug!("About to auto-answer call with SIP ID: {}", sip_call_id);
                
                // Directly answer the call ourselves
                match call.answer().await {
                    Ok(_) => {
                        info!("Call {} auto-answered by user agent", call_id);
                        debug!("Auto-answer succeeded, 200 OK sent to {}", source);
                        
                        // Double-check the call state after answering
                        match call.state().await {
                            CallState::Established => {
                                info!("Call successfully established at {}", call.id());
                            },
                            other_state => {
                                warn!("Call not in expected Established state after auto-answer, state: {}", other_state);
                                // Force state transition to established if needed
                                if other_state != CallState::Established {
                                    info!("Forcing transition to Established state");
                                    if let Err(e) = call.transition_to(CallState::Established).await {
                                        error!("Failed to force transition to Established: {}", e);
                                    } else {
                                        info!("Successfully forced transition to Established");
                                    }
                                }
                            }
                        }
                        
                        // Emit another state change event to ensure client is updated
                        debug!("Emitting explicit state change event");
                        if let Err(e) = event_tx.send(CallEvent::StateChanged {
                            call: call.clone(),
                            previous: CallState::Ringing,
                            current: CallState::Established,
                        }).await {
                            error!("Failed to send state change event: {}", e);
                        }
                    },
                    Err(e) => {
                        error!("User agent auto-answer failed: {}", e);
                    }
                }
            } else {
                debug!("Call not in Ringing state (current: {}), not auto-answering", current_state);
            }
        } else {
            debug!("Auto-answer not enabled in config for call {}", call_id);
        }
        
        return Ok(());
    }
    
    // Handle request for existing call
    if let Some(call) = existing_call {
        debug!("Processing {} request for existing call {}", request.method, call_id);
        
        // For INFO requests, verify dialog parameters match
        if request.method == Method::Info {
            debug!("Verifying dialog parameters for INFO request");
            
            // Extract From tag
            let from_tag = request.headers.iter()
                .find(|h| h.name == HeaderName::From)
                .and_then(|h| h.value.as_text())
                .and_then(|v| rvoip_session_core::dialog::extract_tag(v));
                
            // Extract To tag
            let to_tag = request.headers.iter()
                .find(|h| h.name == HeaderName::To)
                .and_then(|h| h.value.as_text())
                .and_then(|v| rvoip_session_core::dialog::extract_tag(v));
                
            // Get current dialog parameters from call
            let call_has_dialog = call.dialog().await.is_some();
            let remote_tag = call.remote_tag().await;
            
            debug!("INFO request tags - From: {:?}, To: {:?}", from_tag, to_tag);
            debug!("Call dialog info - Has dialog: {}, Remote tag: {:?}", call_has_dialog, remote_tag);
            
            // If call is in Established state, allow INFO even with dialog issues
            // This helps with implementations that might not follow dialog rules strictly
            let current_state = call.state().await;
            if current_state == CallState::Established {
                debug!("Call is Established, proceeding with INFO request despite potential dialog issues");
            }
            // Only enforce dialog validation if the call is not established or we have no remote tag
            else if !call_has_dialog || remote_tag.is_none() {
                debug!("Call has no dialog established, rejecting INFO with 481");
                let mut response = Response::new(StatusCode::CallOrTransactionDoesNotExist);
                add_response_headers(&request, &mut response);
                
                // Send response
                transaction_manager.transport().send_message(
                    Message::Response(response),
                    source
                ).await.map_err(|e| Error::Transport(e.to_string()))?;
                
                return Ok(());
            }
        }
        
        // Handle ACK to 200 OK specially for state transition
        if request.method == Method::Ack {
            // When we receive an ACK after sending 200 OK, the call is now established
            let current_state = call.state().await;
            info!("Received ACK for call {} in state {}", call_id, current_state);
            
            if current_state == CallState::Connecting {
                info!("Transitioning call {} from Connecting to Established after ACK", call_id);
                
                // Directly update the call's state to Established
                if let Err(e) = call.transition_to(CallState::Established).await {
                    warn!("Failed to update call state to Established: {}", e);
                } else {
                    info!("Call {} established successfully after ACK", call_id);
                    
                    // Check if dialog state is properly updated
                    if let Some(dialog) = call.dialog().await {
                        info!("Dialog state after ACK: {}", dialog.state);
                        if dialog.state != DialogState::Confirmed {
                            warn!("Dialog state not updated to Confirmed after ACK!");
                        }
                    } else {
                        warn!("No dialog found after ACK processing!");
                    }
                }
                
                return Ok(());
            } else {
                debug!("Received ACK for call {} in state {}, not transitioning", call_id, current_state);
            }
        }
        
        // Let the call handle other requests
        return match call.handle_request(request).await? {
            Some(response) => {
                debug!("Sending response {} for call {}", response.status, call_id);
                
                // Send response
                transaction_manager.transport().send_message(
                    Message::Response(response),
                    source
                ).await.map_err(|e| Error::Transport(e.to_string()))?;
                
                Ok(())
            },
            None => Ok(()),
        };
    }
    
    debug!("No matching call for {} request with Call-ID {}", request.method, call_id);
    
    // No matching call, reject with 481 Call/Transaction Does Not Exist
    if request.method != Method::Ack {
        let mut response = Response::new(StatusCode::CallOrTransactionDoesNotExist);
        add_response_headers(&request, &mut response);
        
        debug!("Sending 481 Call/Transaction Does Not Exist for {}", call_id);
        
        // Send response
        transaction_manager.transport().send_message(
            Message::Response(response),
            source
        ).await.map_err(|e| Error::Transport(e.to_string()))?;
    }
    
    Ok(())
} 