use std::sync::Arc;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use std::str::FromStr;
use std::collections::HashMap;

use tokio::sync::{mpsc, RwLock, watch, Mutex};
use tracing::{debug, error, info, warn};
use uuid::Uuid;
use bytes::Bytes;

use rvoip_sip_core::{
    Request, Response, Message, Method, StatusCode, Uri, 
    Header, HeaderName, HeaderValue
};
use rvoip_session_core::sdp::{SessionDescription, extract_rtp_port_from_sdp};
use rvoip_session_core::dialog::{Dialog, DialogState, DialogId, extract_tag, extract_uri};
use rvoip_transaction_core::TransactionManager;

use crate::config::CallConfig;
use crate::error::{Error, Result};
use crate::media::{MediaSession, MediaType, SdpHandler};
use crate::DEFAULT_SIP_PORT;
use crate::config::{DEFAULT_RTP_PORT_MIN, DEFAULT_RTP_PORT_MAX};

use super::types::{CallDirection, CallState, StateChangeError};
use super::events::CallEvent;
use super::registry_interface::CallRegistryInterface;
use super::weak_call::WeakCall;
use super::utils::is_valid_state_transition;

/// Call information and control
#[derive(Debug, Clone)]
pub struct Call {
    /// Unique call ID
    id: String,

    /// Call direction
    direction: CallDirection,

    /// Call configuration
    config: CallConfig,

    /// SIP call ID
    sip_call_id: String,

    /// Local tag
    local_tag: String,

    /// Remote tag
    remote_tag: Arc<RwLock<Option<String>>>,

    /// CSeq counter
    cseq: Arc<Mutex<u32>>,

    /// Local URI
    local_uri: Uri,

    /// Remote URI
    remote_uri: Uri,

    /// Remote display name
    remote_display_name: Arc<RwLock<Option<String>>>,

    /// Remote address
    remote_addr: SocketAddr,
    
    /// Local address
    local_addr: SocketAddr,

    /// SIP transaction manager
    transaction_manager: Arc<TransactionManager>,

    /// Current call state
    state: Arc<RwLock<CallState>>,

    /// State change watcher (receiver)
    state_watcher: watch::Receiver<CallState>,
    
    /// State change sender
    state_sender: Arc<watch::Sender<CallState>>,

    /// Call start time
    start_time: Option<Instant>,

    /// Call connect time
    connect_time: Arc<RwLock<Option<Instant>>>,

    /// Call end time
    end_time: Arc<RwLock<Option<Instant>>>,

    /// Active media sessions
    media_sessions: Arc<RwLock<Vec<MediaSession>>>,

    /// Call event sender
    event_tx: mpsc::Sender<CallEvent>,

    /// Local SDP
    local_sdp: Arc<RwLock<Option<SessionDescription>>>,

    /// Remote SDP
    remote_sdp: Arc<RwLock<Option<SessionDescription>>>,

    /// Dialog
    dialog: Arc<RwLock<Option<Dialog>>>,

    /// Last response received
    last_response: Arc<RwLock<Option<Response>>>,

    /// Store the original INVITE request
    original_invite: Arc<RwLock<Option<Request>>>,
    
    /// Store the transaction ID of the INVITE request
    invite_transaction_id: Arc<RwLock<Option<String>>>,
    
    /// Call registry reference
    registry: Arc<RwLock<Option<Arc<dyn CallRegistryInterface + Send + Sync>>>>,
}

impl Call {
    /// Create a new Call instance
    pub(crate) fn new(
        direction: CallDirection,
        config: CallConfig,
        sip_call_id: String,
        local_tag: String,
        local_uri: Uri,
        remote_uri: Uri,
        remote_addr: SocketAddr,
        transaction_manager: Arc<TransactionManager>,
        event_tx: mpsc::Sender<CallEvent>,
    ) -> (Arc<Self>, watch::Sender<CallState>) {
        // Create state channels
        let (state_sender, state_watcher) = watch::channel(CallState::Initial);
        
        // Generate a unique ID for the call
        let id = Uuid::new_v4().to_string();
        
        // Get local address from transaction manager or use a fallback
        let local_addr = transaction_manager.transport().local_addr()
            .unwrap_or_else(|_| {
                warn!("Could not get local address from transport, using 127.0.0.1:{}", DEFAULT_SIP_PORT);
                format!("127.0.0.1:{}", DEFAULT_SIP_PORT).parse().unwrap()
            });
        
        // Create the call instance
        let call = Self {
            id,
            direction,
            config,
            sip_call_id,
            local_tag,
            remote_tag: Arc::new(RwLock::new(None)),
            cseq: Arc::new(Mutex::new(1)),
            local_uri,
            remote_uri,
            remote_display_name: Arc::new(RwLock::new(None)),
            remote_addr,
            local_addr,
            transaction_manager,
            state: Arc::new(RwLock::new(CallState::Initial)),
            state_watcher: state_watcher.clone(),
            state_sender: Arc::new(state_sender.clone()),
            start_time: Some(Instant::now()),
            connect_time: Arc::new(RwLock::new(None)),
            end_time: Arc::new(RwLock::new(None)),
            media_sessions: Arc::new(RwLock::new(Vec::new())),
            event_tx,
            local_sdp: Arc::new(RwLock::new(None)),
            remote_sdp: Arc::new(RwLock::new(None)),
            dialog: Arc::new(RwLock::new(None)),
            last_response: Arc::new(RwLock::new(None)),
            original_invite: Arc::new(RwLock::new(None)),
            invite_transaction_id: Arc::new(RwLock::new(None)),
            registry: Arc::new(RwLock::new(None)),
        };
        
        (Arc::new(call), state_sender)
    }
    
    /// Setup local SDP for the call
    pub(crate) async fn setup_local_sdp(&self) -> Result<Option<SessionDescription>> {
        debug!("Setting up local SDP for call {}", self.id);
        
        // Create an SDP handler
        let local_ip = if let Ok(addr) = self.transaction_manager.transport().local_addr() {
            addr.ip()
        } else {
            "127.0.0.1".parse().unwrap()
        };
        
        let sdp_handler = SdpHandler::new(
            local_ip,
            self.config.rtp_port_range_start.unwrap_or(DEFAULT_RTP_PORT_MIN),
            self.config.rtp_port_range_end.unwrap_or(DEFAULT_RTP_PORT_MAX),
            self.config.clone(),
            self.local_sdp.clone(),
            self.remote_sdp.clone(),
        );
        
        // Create a new local SDP
        let local_sdp = sdp_handler.create_local_sdp().await?;
        
        // Store the created SDP
        if let Some(sdp) = &local_sdp {
            *self.local_sdp.write().await = Some(sdp.clone());
        }
        
        Ok(local_sdp)
    }
    
    /// Get the unique call ID
    pub fn id(&self) -> &str {
        &self.id
    }
    
    /// Get the SIP call ID
    pub fn sip_call_id(&self) -> &str {
        &self.sip_call_id
    }
    
    /// Get the call direction
    pub fn direction(&self) -> CallDirection {
        self.direction
    }
    
    /// Get the current call state
    pub async fn state(&self) -> CallState {
        *self.state.read().await
    }
    
    /// Get the remote URI
    pub fn remote_uri(&self) -> &Uri {
        &self.remote_uri
    }
    
    /// Get the remote display name
    pub async fn remote_display_name(&self) -> Option<String> {
        self.remote_display_name.read().await.clone()
    }
    
    /// Get a caller ID string
    pub async fn caller_id(&self) -> String {
        if let Some(name) = self.remote_display_name.read().await.as_ref() {
            name.to_string()
        } else {
            self.remote_uri.to_string()
        }
    }
    
    /// Get the call duration
    pub async fn duration(&self) -> Option<Duration> {
        let end_time = *self.end_time.read().await;
        let connect_time = *self.connect_time.read().await;
        
        match (connect_time, end_time) {
            (Some(connect), Some(end)) => Some(end.duration_since(connect)),
            (Some(connect), None) => Some(Instant::now().duration_since(connect)),
            _ => None,
        }
    }
    
    /// Get the active media sessions
    pub async fn media_sessions(&self) -> Vec<MediaSession> {
        self.media_sessions.read().await.clone()
    }
    
    /// Get the SIP dialog
    pub async fn dialog(&self) -> Option<Dialog> {
        self.dialog.read().await.clone()
    }
    
    /// Answer an incoming call
    pub async fn answer(&self) -> Result<()> {
        // Verify this is an incoming call
        if self.direction != CallDirection::Incoming {
            return Err(Error::Call("Cannot answer an outgoing call".into()));
        }
        
        // Get the original INVITE request
        let invite = match self.original_invite.read().await.clone() {
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
                    format!("{};tag={}", to_value, self.local_tag)
                };
                response.headers.push(Header::text(HeaderName::To, to_with_tag));
            }
        }
        
        // Get local IP from transaction manager or use a reasonable default
        let local_ip = match self.transaction_manager.transport().local_addr() {
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
            *self.remote_sdp.write().await = Some(remote_sdp.clone());
            
            // Create SDP handler
            let sdp_handler = SdpHandler::new(
                local_ip,
                self.config.rtp_port_range_start.unwrap_or(DEFAULT_RTP_PORT_MIN),
                self.config.rtp_port_range_end.unwrap_or(DEFAULT_RTP_PORT_MAX),
                self.config.clone(),
                self.local_sdp.clone(),
                self.remote_sdp.clone(),
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
            self.local_uri.username().unwrap_or("anonymous"),
            match self.transaction_manager.transport().local_addr() {
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
        self.transaction_manager.transport()
            .send_message(Message::Response(response), self.remote_addr)
            .await
            .map_err(|e| Error::Transport(e.to_string()))?;
        
        // Update call state
        self.transition_to(CallState::Established).await?;
        
        // Set connection time
        *self.connect_time.write().await = Some(Instant::now());
        
        // If we have a media session, save it
        if let Some(session) = media_session {
            debug!("Starting media session for call {}", self.id);
            // Save the media session
            self.media_sessions.write().await.push(session);
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
        // Implementation would go here
        // For now, we'll leave this as a stub to be filled in later
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
    
    /// Handle an incoming SIP request
    pub(crate) async fn handle_request(&self, request: Request) -> Result<Option<Response>> {
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
                            format!("{};tag={}", to_value, self.local_tag)
                        };
                        response.headers.push(Header::text(HeaderName::To, to_with_tag));
                    }
                }
                
                // Add Contact header
                let contact = format!("<sip:{}@{}>", 
                    self.local_uri.username().unwrap_or("anonymous"),
                    self.local_addr.to_string()
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
                if let Some(mut dialog) = self.dialog.read().await.clone() {
                    if dialog.state == DialogState::Early {
                        // Update dialog state to confirmed
                        dialog.state = DialogState::Confirmed;
                        
                        // Update the dialog
                        self.set_dialog(dialog).await?;
                    }
                } else if self.state.read().await.clone() == CallState::Established {
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
                *self.end_time.write().await = Some(Instant::now());
                
                // Update dialog state if we have one
                if let Some(mut dialog) = self.dialog.read().await.clone() {
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
    pub(crate) async fn handle_response(&self, response: &Response) -> Result<()> {
        // Store the last response
        self.store_last_response(response.clone()).await?;
        
        // Get the original INVITE request if this is a response to an INVITE
        let original_invite = self.original_invite.read().await.clone();
        
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
        
        // Handle based on method and response code
        match (method, response.status) {
            // Handle 200 OK to INVITE - establish dialog
            (Method::Invite, status) if status.is_success() => {
                info!("Handling 200 OK response to INVITE");
                
                // If we have the original INVITE, create a dialog
                if let Some(invite) = original_invite {
                    // Try to create a dialog from the response
                    match Dialog::from_2xx_response(&invite, response, true) {
                        Some(dialog) => {
                            info!("Created dialog from 2xx response: {}", dialog.id);
                            
                            // Set the dialog
                            self.set_dialog(dialog).await?;
                            
                            // Transition call state to Established
                            self.transition_to(CallState::Established).await?;
                            
                            // Set connection time
                            *self.connect_time.write().await = Some(Instant::now());
                        },
                        None => {
                            warn!("Failed to create dialog from 2xx response");
                            // We can still proceed with the call, but it will be dialog-less
                        }
                    }
                } else {
                    warn!("No original INVITE stored, cannot create dialog");
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
                                let existing_dialog = self.dialog.read().await.clone();
                                
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
                *self.end_time.write().await = Some(Instant::now());
                
                // Update the dialog state to terminated if we have one
                if let Some(mut dialog) = self.dialog.read().await.clone() {
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
    pub(crate) async fn create_invite_request(&self) -> Result<Request> {
        // Implementation would go here
        // For now, we'll leave this as a stub that returns an error
        Err(Error::Call("Not implemented".into()))
    }
    
    /// Send ACK for a response
    pub(crate) async fn send_ack(&self) -> Result<()> {
        // Implementation would go here
        // For now, we'll leave this as a stub to be filled in later
        Ok(())
    }
    
    /// Get the call registry
    pub async fn registry(&self) -> Option<Arc<dyn CallRegistryInterface + Send + Sync>> {
        self.registry.read().await.clone()
    }
    
    /// Set the call registry
    pub async fn set_registry(&self, registry: Arc<dyn CallRegistryInterface + Send + Sync>) {
        *self.registry.write().await = Some(registry);
    }
    
    /// Store the INVITE transaction ID
    pub async fn store_invite_transaction_id(&self, transaction_id: String) -> Result<()> {
        *self.invite_transaction_id.write().await = Some(transaction_id);
        Ok(())
    }
    
    /// Update the call state
    pub async fn update_state(&self, new_state: CallState) -> Result<()> {
        let current_state = *self.state.read().await;
        
        if !is_valid_state_transition(current_state, new_state) {
            return Err(Error::Call(format!(
                "Invalid state transition from {} to {}",
                current_state, new_state
            )));
        }
        
        // Update state
        *self.state.write().await = new_state;
        
        // Update state watcher
        if let Err(e) = self.state_sender.send(new_state) {
            warn!("Failed to update state watcher: {}", e);
        }
        
        // Handle state-specific actions
        match new_state {
            CallState::Connecting => {
                // Set connect time when transitioning to Connecting state
                *self.connect_time.write().await = Some(Instant::now());
            }
            CallState::Terminated | CallState::Failed => {
                // Set end time when the call is terminated or failed
                *self.end_time.write().await = Some(Instant::now());
            }
            _ => {}
        }
        
        // Send state change event
        if let Err(e) = self.event_tx.send(CallEvent::StateChanged {
            call: Arc::new(self.clone()),
            previous: current_state,
            current: new_state,
        }).await {
            warn!("Failed to send state change event: {}", e);
        }
        
        Ok(())
    }
    
    /// Simple state transition, just forwards to update_state
    pub async fn transition_to(&self, new_state: CallState) -> Result<()> {
        self.update_state(new_state).await
    }
    
    /// Store the original INVITE request
    pub async fn store_invite_request(&self, request: Request) -> Result<()> {
        *self.original_invite.write().await = Some(request);
        Ok(())
    }
    
    /// Store the last response received
    pub async fn store_last_response(&self, response: Response) -> Result<()> {
        *self.last_response.write().await = Some(response);
        Ok(())
    }
    
    /// Setup media from remote SDP
    pub async fn setup_media_from_sdp(&self, sdp: &SessionDescription) -> Result<()> {
        debug!("Setting up media from SDP for call {}", self.id);
        
        // Update our remote SDP
        *self.remote_sdp.write().await = Some(sdp.clone());
        
        // Create SDP handler
        let local_ip = if let Ok(addr) = self.transaction_manager.transport().local_addr() {
            addr.ip()
        } else {
            "127.0.0.1".parse().unwrap()
        };
        
        let sdp_handler = SdpHandler::new(
            local_ip,
            self.config.rtp_port_range_start.unwrap_or(DEFAULT_RTP_PORT_MIN),
            self.config.rtp_port_range_end.unwrap_or(DEFAULT_RTP_PORT_MAX),
            self.config.clone(),
            self.local_sdp.clone(),
            self.remote_sdp.clone(),
        );
        
        // Setup the media based on remote SDP
        let media_session = sdp_handler.setup_media(sdp).await?;
        
        // Store the media session
        self.media_sessions.write().await.push(media_session);
        
        Ok(())
    }
    
    /// Get the remote tag
    pub async fn remote_tag(&self) -> Option<String> {
        self.remote_tag.read().await.clone()
    }
    
    /// Set the remote tag
    pub async fn set_remote_tag(&self, tag: String) {
        *self.remote_tag.write().await = Some(tag);
    }
    
    /// Create a weak reference to this call
    pub fn weak_clone(&self) -> WeakCall {
        WeakCall {
            id: self.id.clone(),
            direction: self.direction,
            sip_call_id: self.sip_call_id.clone(),
            local_uri: self.local_uri.clone(),
            remote_uri: self.remote_uri.clone(),
            remote_addr: self.remote_addr,
            state_watcher: self.state_watcher.clone(),
            
            // Create weak references
            remote_tag: Arc::downgrade(&self.remote_tag),
            state: Arc::downgrade(&self.state),
            connect_time: Arc::downgrade(&self.connect_time),
            end_time: Arc::downgrade(&self.end_time),
            
            // Registry reference
            registry: Arc::downgrade(&self.registry),
            
            // Transaction manager reference (keep strong reference)
            transaction_manager: self.transaction_manager.clone(),
        }
    }
    
    /// Save dialog information to the registry
    async fn save_dialog_to_registry(&self) -> Result<()> {
        if let Some(registry) = self.registry.read().await.clone() {
            if let Some(dialog) = self.dialog.read().await.clone() {
                debug!("Saving dialog {} to registry", dialog.id);
                
                // Get dialog sequence numbers and target
                let local_seq = *self.cseq.lock().await;
                let remote_seq = 0; // TODO: Get remote sequence number from dialog
                
                // Use remote target from dialog or fall back to remote URI
                let remote_target = dialog.remote_uri.clone(); // No optional remote_target field

                // Find call registry interface to update dialog
                registry.update_dialog_info(
                    &dialog.id.to_string(),
                    Some(dialog.call_id.clone()),
                    Some(dialog.state.to_string()),
                    Some(dialog.local_tag.clone().unwrap_or_default()),
                    Some(dialog.remote_tag.clone().unwrap_or_default()),
                    Some(local_seq),
                    Some(remote_seq),
                    None, // route_set
                    Some(remote_target.to_string()),
                    Some(dialog.local_uri.scheme.to_string() == "sips")
                ).await?;
                
                return Ok(());
            }
            
            // No dialog found for this call
            debug!("No dialog found for call {}, not saving to registry", self.id);
            Ok(())
        } else {
            // No registry set
            debug!("No registry set for call {}, not saving dialog", self.id);
            Ok(())
        }
    }
    
    /// Set the dialog for this call and save it to the registry
    pub async fn set_dialog(&self, dialog: Dialog) -> Result<()> {
        // Update the dialog field
        *self.dialog.write().await = Some(dialog);
        
        // Save dialog to registry
        self.save_dialog_to_registry().await?;
        
        Ok(())
    }
} 