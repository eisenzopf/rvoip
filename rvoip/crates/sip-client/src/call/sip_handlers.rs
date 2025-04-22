use std::str::FromStr;
use std::time::Instant;

use tracing::{debug, error, info, warn};

use rvoip_sip_core::{
    Request, Response, Message, Method, StatusCode, Header, HeaderName
};
use rvoip_session_core::dialog::{Dialog, DialogState, extract_tag, extract_uri};

use crate::error::{Error, Result};

use super::call_struct::Call;
use super::types::CallState;

impl Call {
    /// Handle an incoming SIP request
    pub async fn handle_request(&self, request: Request) -> Result<Option<Response>> {
        match request.method {
            Method::Invite => {
                // Store the original INVITE request
                self.store_invite_request(request.clone()).await?;
                
                // For an incoming call, we don't create the dialog yet - it will be created 
                // when we send a 2xx response during answer()
                
                // For now, just acknowledge receipt of INVITE
                // In a real implementation, we would generate a provisional response (e.g., 180 Ringing)
                let mut response = Response::new(StatusCode::Ringing);
                
                // Copy necessary headers
                for header in &request.headers {
                    match header.name {
                        HeaderName::CallId | HeaderName::From | HeaderName::Via => {
                            response.headers.push(header.clone());
                        },
                        _ => {},
                    }
                }
                
                // Add To header with tag for dialog establishment
                if let Some(to_header) = request.header(&HeaderName::To) {
                    if let Some(to_value) = to_header.value.as_text() {
                        // Use our local tag for the To header
                        let to_with_tag = if to_value.contains("tag=") {
                            to_value.to_string()
                        } else {
                            format!("{};tag={}", to_value, self.local_tag_str())
                        };
                        response.headers.push(Header::text(HeaderName::To, to_with_tag));
                    }
                }
                
                // Add Contact header
                let contact = format!("<sip:{}@{}>", 
                    self.local_uri_ref().username().unwrap_or("anonymous"),
                    self.local_addr_ref().to_string()
                );
                response.headers.push(Header::text(HeaderName::Contact, contact));
                
                // Add Content-Length (0 for provisional response)
                response.headers.push(Header::text(HeaderName::ContentLength, "0"));
                
                // Update call state
                self.transition_to(CallState::Ringing).await?;
                
                // Return the provisional response
                Ok(Some(response))
            },
            Method::Ack => {
                // ACK for a 2xx response confirms dialog establishment
                if let Some(mut dialog) = self.dialog_ref().read().await.clone() {
                    if dialog.state == DialogState::Early {
                        // Update dialog state to confirmed
                        dialog.state = DialogState::Confirmed;
                        
                        // Update the dialog
                        self.set_dialog(dialog).await?;
                    }
                } else if self.state_ref().read().await.clone() == CallState::Established {
                    // We may have received an ACK for a dialog-less call (unusual but possible)
                    debug!("Received ACK for a dialog-less established call");
                }
                
                // ACK doesn't have a response in SIP
                Ok(None)
            },
            Method::Bye => {
                // Other side is hanging up
                info!("Received BYE request, terminating call");
                
                // Update call state
                self.transition_to(CallState::Terminated).await?;
                
                // Set end time
                *self.end_time_ref().write().await = Some(Instant::now());
                
                // Update dialog state if we have one
                if let Some(mut dialog) = self.dialog_ref().read().await.clone() {
                    dialog.state = DialogState::Terminated;
                    self.set_dialog(dialog).await?;
                }
                
                // Acknowledge BYE with 200 OK
                let mut response = Response::new(StatusCode::Ok);
                
                // Copy necessary headers
                for header in &request.headers {
                    match header.name {
                        HeaderName::CallId | HeaderName::CSeq | HeaderName::From | HeaderName::Via | HeaderName::To => {
                            response.headers.push(header.clone());
                        },
                        _ => {},
                    }
                }
                
                // Add Content-Length
                response.headers.push(Header::text(HeaderName::ContentLength, "0"));
                
                Ok(Some(response))
            },
            // For other request methods, add handling as needed
            _ => {
                debug!("Received {} request, not handled", request.method);
                Ok(None)
            }
        }
    }
    
    /// Handle a SIP response
    pub async fn handle_response(&self, response: &Response) -> Result<()> {
        // Store the last response
        self.store_last_response(response.clone()).await?;
        
        // Get the original INVITE request if this is a response to an INVITE
        let original_invite = self.original_invite_ref().read().await.clone();
        
        debug!("Handling response: status={}, headers={:?}", response.status, response.headers);
        
        // Extract CSeq to determine what we're handling
        let cseq_header = match response.header(&HeaderName::CSeq) {
            Some(h) => h,
            None => {
                error!("Response missing CSeq header");
                return Err(Error::Protocol("Response missing CSeq header".into()));
            }
        };
        
        let cseq_text = match cseq_header.value.as_text() {
            Some(t) => t,
            None => {
                error!("CSeq header value is not text");
                return Err(Error::Protocol("CSeq header value is not text".into()));
            }
        };
        
        let cseq_parts: Vec<&str> = cseq_text.splitn(2, ' ').collect();
        if cseq_parts.len() < 2 {
            error!("Invalid CSeq format: {}", cseq_text);
            return Err(Error::Protocol(format!("Invalid CSeq format: {}", cseq_text)));
        }
        
        let method_str = cseq_parts[1];
        let method = Method::from_str(method_str).map_err(|_| {
            Error::Protocol(format!("Invalid method in CSeq: {}", method_str))
        })?;
        
        debug!("Processing response for method: {}, status: {}", method, response.status);
        
        // Handle based on method and response code
        match (method, response.status) {
            // Handle 200 OK to INVITE - establish dialog
            (Method::Invite, status) if status.is_success() => {
                info!("Handling 200 OK response to INVITE for call {}", self.sip_call_id());
                debug!("Response content: body_len={}, call_id={}, complete response={:?}", 
                       response.body.len(), self.sip_call_id(), response);
                
                // If we have the original INVITE, create a dialog
                if let Some(invite) = original_invite {
                    debug!("Original INVITE found for call {}, creating dialog", self.sip_call_id());
                    // Try to create a dialog from the response
                    match Dialog::from_2xx_response(&invite, response, true) {
                        Some(dialog) => {
                            info!("Created dialog from 2xx response: {} for call {}", dialog.id, self.sip_call_id());
                            debug!("Dialog details: local_tag={:?}, remote_tag={:?}, state={:?}", 
                                  dialog.local_tag, dialog.remote_tag, dialog.state);
                            
                            // Set the dialog
                            if let Err(e) = self.set_dialog(dialog).await {
                                error!("Failed to set dialog: {}", e);
                                return Err(e);
                            }
                            debug!("Dialog set successfully for call {}", self.sip_call_id());
                            
                            // Transition call state to Established
                            info!("Transitioning call {} state to Established", self.sip_call_id());
                            if let Err(e) = self.transition_to(CallState::Established).await {
                                error!("Failed to transition to Established state: {}", e);
                                return Err(e);
                            }
                            debug!("State successfully transitioned to Established for call {}", self.sip_call_id());
                            
                            // Set connection time
                            *self.connect_time_ref().write().await = Some(Instant::now());
                            debug!("Set connection time for established call {}", self.sip_call_id());
                            
                            // Check if we need to send an ACK
                            debug!("Sending ACK for 200 OK response for call {}", self.sip_call_id());
                            if let Err(e) = self.send_ack().await {
                                error!("Failed to send ACK: {}", e);
                                return Err(e);
                            }
                            info!("ACK sent successfully for call {}", self.sip_call_id());
                        },
                        None => {
                            warn!("Failed to create dialog from 2xx response for call {}", self.sip_call_id());
                            debug!("Dialog creation failed. Response: {:?}", response);
                            // We can still proceed with the call, but it will be dialog-less
                        }
                    }
                } else {
                    warn!("No original INVITE stored, cannot create dialog for call {}", self.sip_call_id());
                    debug!("Original INVITE missing. Call ID: {}", self.sip_call_id());
                }
            },
            
            // Handle 1xx responses to INVITE - early dialog
            (Method::Invite, status) if (100..200).contains(&status.as_u16()) => {
                debug!("Handling 1xx response to INVITE: {}", status);
                
                // If we have the original INVITE and status > 100, create an early dialog
                if let Some(invite) = original_invite.clone() {
                    if status.as_u16() > 100 {
                        // Try to create an early dialog from the provisional response
                        match Dialog::from_provisional_response(&invite, response, true) {
                            Some(dialog) => {
                                info!("Created early dialog from {} response: {}", status, dialog.id);
                                
                                // Check if we already have a dialog
                                let existing_dialog = self.dialog_ref().read().await.clone();
                                
                                if existing_dialog.is_none() {
                                    // Set the early dialog
                                    self.set_dialog(dialog).await?;
                                }
                                
                                // Update call state based on response
                                if status == StatusCode::Ringing {
                                    self.transition_to(CallState::Ringing).await?;
                                } else if status.as_u16() >= 180 {
                                    self.transition_to(CallState::Progress).await?;
                                }
                            },
                            None => {
                                debug!("Could not create early dialog from {} response", status);
                            }
                        }
                    }
                }
            },
            
            // Handle failure responses to INVITE
            (Method::Invite, status) if (400..700).contains(&status.as_u16()) => {
                warn!("INVITE failed with status {}", status);
                self.transition_to(CallState::Failed).await?;
            },
            
            // Handle success responses to BYE
            (Method::Bye, status) if status.is_success() => {
                info!("BYE completed successfully with status {}", status);
                self.transition_to(CallState::Terminated).await?;
                
                // Set end time
                *self.end_time_ref().write().await = Some(Instant::now());
                
                // Update the dialog state to terminated if we have one
                if let Some(mut dialog) = self.dialog_ref().read().await.clone() {
                    dialog.state = DialogState::Terminated;
                    self.set_dialog(dialog).await?;
                }
            },
            
            // Other responses
            (method, status) => {
                debug!("Received {} response to {}", status, method);
            }
        }
        
        Ok(())
    }
    
    /// Create an INVITE request
    pub async fn create_invite_request(&self) -> Result<Request> {
        // Implementation would go here
        // For now, we'll leave this as a stub that returns an error
        Err(Error::Call("Not implemented".into()))
    }
    
    /// Send ACK for a response
    pub async fn send_ack(&self) -> Result<()> {
        debug!("Starting to send ACK for call {}", self.id());
        
        // Get the dialog
        let dialog = match self.dialog_ref().read().await.clone() {
            Some(dialog) => dialog,
            None => {
                warn!("No dialog found for sending ACK");
                return Err(Error::Call("No dialog found for sending ACK".into()));
            }
        };
        
        // Make sure we have the original INVITE
        let invite = match self.original_invite_ref().read().await.clone() {
            Some(invite) => invite,
            None => {
                warn!("No original INVITE found for sending ACK");
                return Err(Error::Call("No original INVITE found for sending ACK".into()));
            }
        };
        
        // Get the last response
        let response = match self.last_response_ref().read().await.clone() {
            Some(response) => response,
            None => {
                warn!("No response found for sending ACK");
                return Err(Error::Call("No response found for sending ACK".into()));
            }
        };
        
        // Create ACK request
        let mut ack = Request::new(Method::Ack, dialog.remote_target.clone());
        
        // Copy Call-ID from dialog
        ack.headers.push(Header::text(HeaderName::CallId, dialog.call_id.clone()));
        
        // Create CSeq header
        // For ACK, we use the same CSeq number as the INVITE
        let cseq_header = match invite.header(&HeaderName::CSeq) {
            Some(header) => {
                if let Some(text) = header.value.as_text() {
                    if let Some(value) = text.split_whitespace().next() {
                        format!("{} ACK", value)
                    } else {
                        "1 ACK".to_string()
                    }
                } else {
                    "1 ACK".to_string()
                }
            },
            None => "1 ACK".to_string(),
        };
        ack.headers.push(Header::text(HeaderName::CSeq, cseq_header));
        
        // Add From header with tag
        let from = match invite.header(&HeaderName::From) {
            Some(header) => {
                if let Some(text) = header.value.as_text() {
                    text.to_string()
                } else {
                    format!("<sip:{}@{}>", self.local_uri_ref().username().unwrap_or("anonymous"), "127.0.0.1")
                }
            },
            None => format!("<sip:{}@{}>", self.local_uri_ref().username().unwrap_or("anonymous"), "127.0.0.1"),
        };
        ack.headers.push(Header::text(HeaderName::From, from));
        
        // Add To header with tag from the dialog
        let to = match response.header(&HeaderName::To) {
            Some(header) => {
                if let Some(text) = header.value.as_text() {
                    text.to_string()
                } else {
                    format!("<{}>", dialog.remote_uri)
                }
            },
            None => format!("<{}>", dialog.remote_uri),
        };
        ack.headers.push(Header::text(HeaderName::To, to));
        
        // Add Via header
        let via = match invite.header(&HeaderName::Via) {
            Some(header) => {
                if let Some(text) = header.value.as_text() {
                    let parts: Vec<&str> = text.splitn(2, ';').collect();
                    if parts.len() > 1 {
                        // Replace branch parameter
                        format!("{};branch=z9hG4bK-{}", parts[0], uuid::Uuid::new_v4())
                    } else {
                        // Add branch parameter
                        format!("{};branch=z9hG4bK-{}", text, uuid::Uuid::new_v4())
                    }
                } else {
                    format!("SIP/2.0/UDP {};branch=z9hG4bK-{}", self.local_addr_ref(), uuid::Uuid::new_v4())
                }
            },
            None => format!("SIP/2.0/UDP {};branch=z9hG4bK-{}", self.local_addr_ref(), uuid::Uuid::new_v4()),
        };
        ack.headers.push(Header::text(HeaderName::Via, via));
        
        // Add Max-Forwards
        ack.headers.push(Header::text(HeaderName::MaxForwards, "70"));
        
        // Add Contact header
        let contact = format!("<sip:{}@{}>", 
            self.local_uri_ref().username().unwrap_or("anonymous"),
            self.local_addr_ref()
        );
        ack.headers.push(Header::text(HeaderName::Contact, contact));
        
        // Add Content-Length (0 for ACK)
        ack.headers.push(Header::text(HeaderName::ContentLength, "0"));
        
        // Send the ACK
        debug!("Sending ACK request");
        self.transaction_manager_ref().transport()
            .send_message(Message::Request(ack), *self.remote_addr_ref())
            .await
            .map_err(|e| Error::Transport(e.to_string()))?;
            
        debug!("ACK sent successfully for call {}", self.id());
            
        Ok(())
    }
} 