use std::sync::Arc;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use std::collections::HashMap;

use tokio::sync::{mpsc, RwLock, watch, Mutex};
use tracing::{debug, error, info, warn};
use uuid::Uuid;
use bytes::Bytes;

use rvoip_sip_core::{
    Request, Response, Message, Method, StatusCode, Uri, 
    Header, HeaderName, HeaderValue
};
use rvoip_session_core::sdp::SessionDescription;
use rvoip_session_core::dialog::{Dialog, DialogState, DialogId};
use rvoip_transaction_core::TransactionManager;

use crate::config::CallConfig;
use crate::error::{Error, Result};
use crate::media::MediaSession;
use crate::DEFAULT_SIP_PORT;

use super::types::{CallDirection, CallState};
use super::events::CallEvent;
use super::registry_interface::CallRegistryInterface;
use super::weak_call::WeakCall;

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

    /// Create a weak reference to this call
    pub fn weak_clone(&self) -> WeakCall {
        WeakCall {
            id: self.id.clone(),
            direction: self.direction(),
            sip_call_id: self.sip_call_id.clone(),
            local_uri: self.local_uri_ref().clone(),
            remote_uri: self.remote_uri.clone(),
            remote_addr: *self.remote_addr_ref(),
            state_watcher: self.state_watcher.clone(),
            
            // Create weak references
            remote_tag: Arc::downgrade(&self.remote_tag),
            state: Arc::downgrade(&self.state),
            connect_time: Arc::downgrade(&self.connect_time),
            end_time: Arc::downgrade(&self.end_time),
            
            // Registry reference
            registry: Arc::downgrade(&self.registry),
            
            // Transaction manager reference (keep strong reference)
            transaction_manager: self.transaction_manager_ref().clone(),
        }
    }

    /// Get the original invite
    pub fn original_invite_ref(&self) -> &Arc<RwLock<Option<Request>>> {
        &self.original_invite
    }
    
    /// Get the local tag
    pub fn local_tag_str(&self) -> &str {
        &self.local_tag
    }
    
    /// Get the remote tag
    pub fn remote_tag_ref(&self) -> &Arc<RwLock<Option<String>>> {
        &self.remote_tag
    }
    
    /// Get the CSeq counter
    pub fn cseq_ref(&self) -> &Arc<Mutex<u32>> {
        &self.cseq
    }
    
    /// Get the local URI
    pub fn local_uri_ref(&self) -> &Uri {
        &self.local_uri
    }
    
    /// Get the local address
    pub fn local_addr_ref(&self) -> &SocketAddr {
        &self.local_addr
    }
    
    /// Get the remote address
    pub fn remote_addr_ref(&self) -> &SocketAddr {
        &self.remote_addr
    }
    
    /// Get the transaction manager
    pub fn transaction_manager_ref(&self) -> &Arc<TransactionManager> {
        &self.transaction_manager
    }
    
    /// Get the call state
    pub fn state_ref(&self) -> &Arc<RwLock<CallState>> {
        &self.state
    }
    
    /// Get the state sender
    pub fn state_sender_ref(&self) -> &Arc<watch::Sender<CallState>> {
        &self.state_sender
    }
    
    /// Get the connect time
    pub fn connect_time_ref(&self) -> &Arc<RwLock<Option<Instant>>> {
        &self.connect_time
    }
    
    /// Get the end time
    pub fn end_time_ref(&self) -> &Arc<RwLock<Option<Instant>>> {
        &self.end_time
    }
    
    /// Get the media sessions
    pub fn media_sessions_ref(&self) -> &Arc<RwLock<Vec<MediaSession>>> {
        &self.media_sessions
    }
    
    /// Get the event transmitter
    pub fn event_tx_ref(&self) -> &mpsc::Sender<CallEvent> {
        &self.event_tx
    }
    
    /// Get the local SDP
    pub fn local_sdp_ref(&self) -> &Arc<RwLock<Option<SessionDescription>>> {
        &self.local_sdp
    }
    
    /// Get the remote SDP
    pub fn remote_sdp_ref(&self) -> &Arc<RwLock<Option<SessionDescription>>> {
        &self.remote_sdp
    }
    
    /// Get the dialog
    pub fn dialog_ref(&self) -> &Arc<RwLock<Option<Dialog>>> {
        &self.dialog
    }
    
    /// Get the last response
    pub fn last_response_ref(&self) -> &Arc<RwLock<Option<Response>>> {
        &self.last_response
    }
    
    /// Get the invite transaction ID
    pub fn invite_transaction_id_ref(&self) -> &Arc<RwLock<Option<String>>> {
        &self.invite_transaction_id
    }
    
    /// Get the call registry
    pub fn registry_ref(&self) -> &Arc<RwLock<Option<Arc<dyn CallRegistryInterface + Send + Sync>>>> {
        &self.registry
    }
    
    /// Get the call config
    pub fn config_ref(&self) -> &CallConfig {
        &self.config
    }
} 