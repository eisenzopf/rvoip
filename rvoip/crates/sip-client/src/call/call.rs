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
use crate::media::{MediaSession, MediaType};

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
    pub(crate) async fn setup_local_sdp(&self) -> Result<()> {
        // Implementation would go here
        // For now, we'll leave this as a stub to be filled in later
        Ok(())
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
        *self.state_watcher.borrow()
    }
    
    /// Get the remote URI
    pub fn remote_uri(&self) -> &Uri {
        &self.remote_uri
    }
    
    /// Get the remote display name
    pub async fn remote_display_name(&self) -> Option<String> {
        self.remote_display_name.read().await.clone()
    }
    
    /// Get the caller ID (display name or URI)
    pub async fn caller_id(&self) -> String {
        if let Some(name) = self.remote_display_name().await {
            name
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
        // Implementation would go here
        // For now, we'll leave this as a stub to be filled in later
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
        // Implementation would go here
        // For now, we'll leave this as a stub to be filled in later
        Ok(None)
    }
    
    /// Handle a SIP response
    pub(crate) async fn handle_response(&self, response: &Response) -> Result<()> {
        // Implementation would go here
        // For now, we'll leave this as a stub to be filled in later
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
    pub fn registry(&self) -> Option<Arc<dyn CallRegistryInterface + Send + Sync>> {
        match self.registry.try_read() {
            Ok(guard) => guard.clone(),
            Err(_) => None,
        }
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
    
    /// Setup media from SDP
    pub async fn setup_media_from_sdp(&self, sdp: &SessionDescription) -> Result<()> {
        // Implementation would go here
        // For now, we'll leave this as a stub to be filled in later
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
    
    /// Save call dialog to registry
    async fn save_dialog_to_registry(&self) -> Result<()> {
        // Implementation would go here
        // For now, we'll leave this as a stub to be filled in later
        Ok(())
    }
} 