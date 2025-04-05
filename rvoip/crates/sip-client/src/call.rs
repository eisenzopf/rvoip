use std::sync::Arc;
use std::net::SocketAddr;
use std::time::Duration;

use tokio::sync::{mpsc, RwLock, watch, Mutex};
use tokio::time::Instant;
use tracing::{debug, error, info, warn};
use uuid::Uuid;
use bytes::Bytes;
use futures::future::BoxFuture;
use futures::FutureExt;

use rvoip_sip_core::{
    Request, Response, Message, Method, StatusCode, Uri, 
    Header, HeaderName, HeaderValue
};
use rvoip_session_core::sdp::{SessionDescription, extract_rtp_port_from_sdp};
use rvoip_transaction_core::TransactionManager;

use crate::config::CallConfig;
use crate::error::{Error, Result};
use crate::media::{MediaSession, MediaType};

/// Direction of the call
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallDirection {
    /// Outgoing call (we initiated it)
    Outgoing,
    /// Incoming call (we received it)
    Incoming,
}

/// State of a call
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallState {
    /// Initial state
    Initial,
    /// Outgoing call: INVITE sent, waiting for response
    /// Incoming call: Ringing, 180 Ringing sent
    Ringing,
    /// Outgoing call: Received final response, waiting for ACK
    /// Incoming call: 200 OK sent, waiting for ACK
    Connecting,
    /// Call is established (media flowing)
    Established,
    /// Call is being terminated
    Terminating,
    /// Call is terminated
    Terminated,
}

impl std::fmt::Display for CallState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallState::Initial => write!(f, "Initial"),
            CallState::Ringing => write!(f, "Ringing"),
            CallState::Connecting => write!(f, "Connecting"),
            CallState::Established => write!(f, "Established"),
            CallState::Terminating => write!(f, "Terminating"),
            CallState::Terminated => write!(f, "Terminated"),
        }
    }
}

/// Call events
#[derive(Debug, Clone)]
pub enum CallEvent {
    /// Incoming call received
    IncomingCall(Arc<Call>),
    /// Call state changed
    StateChanged {
        /// Call instance
        call: Arc<Call>,
        /// Previous state
        previous: CallState,
        /// New state
        current: CallState,
    },
    /// Media added to call
    MediaAdded {
        /// Call instance
        call: Arc<Call>,
        /// Media type
        media_type: MediaType,
    },
    /// Media removed from call
    MediaRemoved {
        /// Call instance
        call: Arc<Call>,
        /// Media type
        media_type: MediaType,
    },
    /// DTMF digit received
    DtmfReceived {
        /// Call instance
        call: Arc<Call>,
        /// DTMF digit
        digit: char,
    },
    /// Call terminated
    Terminated {
        /// Call instance
        call: Arc<Call>,
        /// Reason for termination
        reason: String,
    },
    /// Error occurred
    Error {
        /// Call instance
        call: Arc<Call>,
        /// Error description
        error: String,
    },
}

/// Call information and control
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

    /// State change watcher
    state_watcher: watch::Receiver<CallState>,

    /// Call start time
    start_time: Option<Instant>,

    /// Call connect time
    connect_time: Option<Instant>,

    /// Call end time
    end_time: Option<Instant>,

    /// Active media sessions
    media_sessions: Arc<RwLock<Vec<MediaSession>>>,

    /// Call event sender
    event_tx: mpsc::Sender<CallEvent>,

    /// Local SDP
    local_sdp: Arc<RwLock<Option<SessionDescription>>>,

    /// Remote SDP
    remote_sdp: Arc<RwLock<Option<SessionDescription>>>,
}

impl Call {
    /// Create a new call
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
        let (state_tx, state_rx) = watch::channel(CallState::Initial);

        let call = Arc::new(Self {
            id: Uuid::new_v4().to_string(),
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
            state_watcher: state_rx,
            start_time: Some(Instant::now()),
            connect_time: None,
            end_time: None,
            media_sessions: Arc::new(RwLock::new(Vec::new())),
            event_tx,
            local_sdp: Arc::new(RwLock::new(None)),
            remote_sdp: Arc::new(RwLock::new(None)),
        });

        (call, state_tx)
    }

    /// Get the call ID
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

    /// Get the caller ID (formatted with display name if available)
    pub async fn caller_id(&self) -> String {
        if let Some(name) = self.remote_display_name().await {
            format!("{} <{}>", name, self.remote_uri)
        } else {
            self.remote_uri.to_string()
        }
    }

    /// Get the call duration
    pub fn duration(&self) -> Option<Duration> {
        let start = self.connect_time?;
        let end = self.end_time.unwrap_or_else(Instant::now);
        Some(end.duration_since(start))
    }

    /// Get the active media sessions
    pub async fn media_sessions(&self) -> Vec<MediaSession> {
        self.media_sessions.read().await.clone()
    }

    /// Answer an incoming call
    pub async fn answer(&self) -> Result<()> {
        if self.direction != CallDirection::Incoming {
            return Err(Error::Call("Cannot answer an outgoing call".into()));
        }

        let current_state = self.state().await;
        if current_state != CallState::Ringing {
            return Err(Error::Call(
                format!("Cannot answer call in {} state", current_state)
            ));
        }

        // Implementation will be filled in later
        debug!("Answering call {} not implemented yet", self.id);

        Ok(())
    }

    /// Reject an incoming call
    pub async fn reject(&self, status: StatusCode) -> Result<()> {
        if self.direction != CallDirection::Incoming {
            return Err(Error::Call("Cannot reject an outgoing call".into()));
        }

        let current_state = self.state().await;
        if current_state != CallState::Ringing && current_state != CallState::Initial {
            return Err(Error::Call(
                format!("Cannot reject call in {} state", current_state)
            ));
        }

        // Implementation will be filled in later
        debug!("Rejecting call {} not implemented yet", self.id);

        Ok(())
    }

    /// Hang up a call
    pub async fn hangup(&self) -> Result<()> {
        let current_state = self.state().await;
        if current_state == CallState::Terminated || current_state == CallState::Terminating {
            return Ok(());
        }

        // Implementation will be filled in later
        debug!("Hanging up call {} not implemented yet", self.id);

        Ok(())
    }

    /// Send DTMF digit
    pub async fn send_dtmf(&self, digit: char) -> Result<()> {
        let current_state = self.state().await;
        if current_state != CallState::Established {
            return Err(Error::Call(
                format!("Cannot send DTMF in {} state", current_state)
            ));
        }

        // Implementation will be filled in later
        debug!("Sending DTMF {} for call {} not implemented yet", digit, self.id);

        Ok(())
    }

    /// Wait until the call is established or terminated
    pub async fn wait_until_established(&self) -> Result<()> {
        loop {
            let state = *self.state_watcher.borrow();
            match state {
                CallState::Established => return Ok(()),
                CallState::Terminated => {
                    return Err(Error::Call("Call terminated before being established".into()));
                }
                _ => {
                    // Wait for state change
                    let cloned_watcher = self.state_watcher.clone();
                    if cloned_watcher.changed().await.is_err() {
                        return Err(Error::Call("Call state watcher closed".into()));
                    }
                }
            }
        }
    }

    /// Wait until the call is terminated
    pub async fn wait_until_terminated(&self) -> Result<()> {
        loop {
            let state = *self.state_watcher.borrow();
            if state == CallState::Terminated {
                return Ok(());
            }

            // Wait for state change
            let cloned_watcher = self.state_watcher.clone();
            if cloned_watcher.changed().await.is_err() {
                return Err(Error::Call("Call state watcher closed".into()));
            }
        }
    }

    /// Handle an incoming request for this call
    pub(crate) async fn handle_request(&self, request: Request) -> Result<Option<Response>> {
        // Implementation will be filled in later
        debug!("Handling incoming request {} for call {}", request.method, self.id);
        
        Ok(None)
    }

    /// Handle an incoming response for this call
    pub(crate) async fn handle_response(&self, response: Response) -> Result<()> {
        // Implementation will be filled in later
        debug!("Handling incoming response {} for call {}", response.status, self.id);
        
        Ok(())
    }
} 