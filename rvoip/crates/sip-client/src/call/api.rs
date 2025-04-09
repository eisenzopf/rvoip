use std::sync::Arc;
use std::time::Instant;

use tokio::time;
use tracing::{debug, error, info, warn};

use rvoip_sip_core::{
    Request, Response, Message, Method, StatusCode, Header, HeaderName
};
use rvoip_session_core::dialog::{Dialog, DialogState};
use rvoip_session_core::sdp::SessionDescription;

use crate::error::{Error, Result};
use crate::media::SdpHandler;
use crate::config::{DEFAULT_RTP_PORT_MIN, DEFAULT_RTP_PORT_MAX};
use crate::DEFAULT_SIP_PORT;

use super::call_struct::Call;
use super::types::CallState;

impl Call {
    /// Answer an incoming call
    pub async fn answer(&self) -> Result<()> {
        // Verify this is an incoming call
        if self.direction() != super::types::CallDirection::Incoming {
            return Err(Error::Call("Cannot answer an outgoing call".into()));
        }
        
        // Get the original INVITE request
        let invite = match self.original_invite_ref().read().await.clone() {
            Some(invite) => invite,
            None => {
                return Err(Error::Call("No INVITE request found to answer".into()));
            }
        };
        
        // Create a 200 OK response
        let mut response = Response::new(StatusCode::Ok);
        
        // Copy necessary headers from request
        for header in &invite.headers {
            match header.name {
                HeaderName::CallId | HeaderName::From | HeaderName::CSeq | HeaderName::Via => {
                    response.headers.push(header.clone());
                },
                _ => {},
            }
        }
        
        // Add To header with tag for dialog establishment
        if let Some(to_header) = invite.header(&HeaderName::To) {
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
        
        // Get local IP from transaction manager or use a reasonable default
        let local_ip = match self.transaction_manager_ref().transport().local_addr() {
            Ok(addr) => addr.ip(),
            Err(_) => {
                warn!("Could not get local IP from transport, using 127.0.0.1");
                "127.0.0.1".parse().unwrap()
            }
        };
        
        // Process SDP if it exists in the INVITE
        let mut media_session = None;
        if !invite.body.is_empty() {
            // Extract content type
            let content_type = invite.header(&HeaderName::ContentType)
                .and_then(|h| h.value.as_text());
                
            // Parse the SDP
            let sdp_str = std::str::from_utf8(&invite.body)
                .map_err(|_| Error::SipProtocol("Invalid UTF-8 in SDP".into()))?;
                
            let remote_sdp = SessionDescription::parse(sdp_str)
                .map_err(|e| Error::SdpParsing(format!("Invalid SDP: {}", e)))?;
            
            // Store remote SDP
            *self.remote_sdp_ref().write().await = Some(remote_sdp.clone());
            
            // Create SDP handler
            let sdp_handler = SdpHandler::new(
                local_ip,
                self.config_ref().rtp_port_range_start.unwrap_or(DEFAULT_RTP_PORT_MIN),
                self.config_ref().rtp_port_range_end.unwrap_or(DEFAULT_RTP_PORT_MAX),
                self.config_ref().clone(),
                self.local_sdp_ref().clone(),
                self.remote_sdp_ref().clone(),
            );
            
            // Setup media from SDP
            if let Err(e) = self.setup_media_from_sdp(&remote_sdp).await {
                warn!("Error setting up media from SDP: {}", e);
            }
            
            // Process using SDP handler for response
            match sdp_handler.process_remote_sdp(&remote_sdp).await {
                Ok(Some(session)) => {
                    media_session = Some(session);
                },
                Ok(None) => {
                    warn!("No compatible media found in SDP");
                },
                Err(e) => {
                    warn!("Failed to process remote SDP: {}", e);
                }
            }
        } else {
            // No body in INVITE, add empty Content-Length
            response.headers.push(Header::text(HeaderName::ContentLength, "0"));
        }
        
        // Add Contact header
        let contact = format!("<sip:{}@{}>", 
            self.local_uri_ref().username().unwrap_or("anonymous"),
            match self.transaction_manager_ref().transport().local_addr() {
                Ok(addr) => addr.to_string(),
                Err(_) => format!("{}:{}", local_ip, DEFAULT_SIP_PORT)
            }
        );
        response.headers.push(Header::text(HeaderName::Contact, contact));
        
        // Create dialog from 2xx response
        let dialog = Dialog::from_2xx_response(&invite, &response, false);
        
        if let Some(dialog) = dialog {
            info!("Created dialog for incoming call: {}", dialog.id);
            // Save dialog to call and registry
            self.set_dialog(dialog).await?;
        } else {
            warn!("Failed to create dialog for incoming call");
        }
        
        // Send the response
        self.transaction_manager_ref().transport()
            .send_message(Message::Response(response), *self.remote_addr_ref())
            .await
            .map_err(|e| Error::Transport(e.to_string()))?;
        
        // Update call state
        self.transition_to(CallState::Established).await?;
        
        // Set connection time
        *self.connect_time_ref().write().await = Some(Instant::now());
        
        // If we have a media session, save it
        if let Some(session) = media_session {
            debug!("Starting media session for call {}", self.id());
            // Save the media session
            self.media_sessions_ref().write().await.push(session);
        }
        
        // Update call state in registry if available
        if let Some(registry) = self.registry_ref().read().await.clone() {
            debug!("Updating call state in registry after answer");
            let call_id = self.sip_call_id().to_string();
            if let Err(e) = registry.update_call_state(&call_id, CallState::Ringing, CallState::Established).await {
                warn!("Failed to update call state in registry: {}", e);
            } else {
                debug!("Successfully updated call state in registry to Established");
            }
        }
        
        Ok(())
    }
    
    /// Reject an incoming call
    pub async fn reject(&self, status: StatusCode) -> Result<()> {
        // Implementation would go here
        // For now, we'll leave this as a stub to be filled in later
        Ok(())
    }
    
    /// Hang up a call
    pub async fn hangup(&self) -> Result<()> {
        debug!("Starting to hang up call {}", self.id());
        
        // Check current state
        let current_state = self.state_ref().read().await.clone();
        debug!("Current call state before hangup: {}", current_state);
        
        // Only established calls can be hung up
        if current_state != CallState::Established {
            return Err(Error::Call(format!("Cannot hang up call in {} state", current_state)));
        }
        
        // Get the dialog
        let dialog = match self.dialog_ref().read().await.clone() {
            Some(dialog) => dialog,
            None => {
                warn!("No dialog found for hanging up call");
                return Err(Error::Call("Cannot hang up: no dialog found".into()));
            }
        };
        
        debug!("Dialog found for hangup: id={}, state={:?}", dialog.id, dialog.state);
        
        // Create BYE request
        let mut bye = Request::new(Method::Bye, dialog.remote_target.clone());
        
        // Copy Call-ID from dialog
        bye.headers.push(Header::text(HeaderName::CallId, dialog.call_id.clone()));
        
        // Create CSeq header
        let cseq = *self.cseq_ref().lock().await;
        let cseq_header = format!("{} BYE", cseq);
        bye.headers.push(Header::text(HeaderName::CSeq, cseq_header));
        
        // Add From header with tag - use unwrap_or_default for Option
        let from = format!("<{}>;tag={}", self.local_uri_ref(), dialog.local_tag.clone().unwrap_or_default());
        bye.headers.push(Header::text(HeaderName::From, from));
        
        // Add To header with tag - use unwrap_or_default for Option
        let to = format!("<{}>;tag={}", dialog.remote_uri, dialog.remote_tag.clone().unwrap_or_default());
        bye.headers.push(Header::text(HeaderName::To, to));
        
        // Add Via header
        let via = format!("SIP/2.0/UDP {};branch=z9hG4bK-{}", self.local_addr_ref(), uuid::Uuid::new_v4());
        bye.headers.push(Header::text(HeaderName::Via, via));
        
        // Add Max-Forwards
        bye.headers.push(Header::text(HeaderName::MaxForwards, "70"));
        
        // Add Contact header
        let contact = format!("<sip:{}@{}>", 
            self.local_uri_ref().username().unwrap_or("anonymous"),
            self.local_addr_ref()
        );
        bye.headers.push(Header::text(HeaderName::Contact, contact));
        
        // Add Content-Length (0 for BYE)
        bye.headers.push(Header::text(HeaderName::ContentLength, "0"));
        
        // Update call state to terminating
        self.transition_to(CallState::Terminating).await?;
        
        // Send the BYE request directly through the transport
        debug!("Sending BYE request");
        self.transaction_manager_ref().transport()
            .send_message(Message::Request(bye), *self.remote_addr_ref())
            .await
            .map_err(|e| Error::Transport(e.to_string()))?;
        
        debug!("BYE request sent successfully");
        
        // Set a timeout to wait for response
        let timeout = time::sleep(std::time::Duration::from_secs(5));
        
        tokio::pin!(timeout);
        
        // We're not going to wait for the response - just transition to terminated
        self.transition_to(CallState::Terminated).await?;
        
        // Set end time
        *self.end_time_ref().write().await = Some(Instant::now());
        
        // Update dialog state
        let mut updated_dialog = dialog.clone();
        updated_dialog.state = DialogState::Terminated;
        self.set_dialog(updated_dialog).await?;
        
        debug!("Call {} successfully terminated", self.id());
        
        Ok(())
    }
    
    /// Send a DTMF digit
    pub async fn send_dtmf(&self, digit: char) -> Result<()> {
        // Implementation would go here
        // For now, we'll leave this as a stub to be filled in later
        Ok(())
    }
    
    /// Wait until the call is established or fails
    pub async fn wait_until_established(&self) -> Result<()> {
        // Implementation would go here
        // For now, we'll leave this as a stub to be filled in later
        Ok(())
    }
    
    /// Wait until the call is terminated
    pub async fn wait_until_terminated(&self) -> Result<()> {
        // Implementation would go here
        // For now, we'll leave this as a stub to be filled in later
        Ok(())
    }
} 