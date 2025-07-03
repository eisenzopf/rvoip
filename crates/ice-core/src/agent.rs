use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use rand::random;
use tokio::net::{TcpSocket, UdpSocket};
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::time::{self, sleep};
use tracing::{debug, error, info, trace, warn};

use crate::candidate::{Candidate, CandidateType, IceCandidate, TransportType, UdpCandidate, TcpCandidate};
use crate::config::{GatheringPolicy, IceComponent, IceConfig, IceRole, IceServerConfig};
use crate::error::{Error, Result};
use crate::stun::{StunAttribute, StunMessage, StunMessageType};

/// ICE agent state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IceAgentState {
    /// Initial state
    New,
    
    /// Gathering candidates
    Gathering,
    
    /// All candidates gathered
    Complete,
    
    /// Checking connectivity
    Checking,
    
    /// Connected and usable
    Connected,
    
    /// Connection failed
    Failed,
    
    /// Connection disconnected
    Disconnected,
    
    /// Connection closed
    Closed,
}

impl std::fmt::Display for IceAgentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::New => write!(f, "new"),
            Self::Gathering => write!(f, "gathering"),
            Self::Complete => write!(f, "complete"),
            Self::Checking => write!(f, "checking"),
            Self::Connected => write!(f, "connected"),
            Self::Failed => write!(f, "failed"),
            Self::Disconnected => write!(f, "disconnected"),
            Self::Closed => write!(f, "closed"),
        }
    }
}

/// ICE agent events
#[derive(Debug, Clone)]
pub enum IceAgentEvent {
    /// State change
    StateChange(IceAgentState),
    
    /// New local candidate found
    NewCandidate(IceCandidate),
    
    /// Gathering state change (true = gathering complete)
    GatheringStateChange(bool),
    
    /// Selected pair changed
    SelectedPairChange {
        local: IceCandidate,
        remote: IceCandidate,
    },
    
    /// Data received
    DataReceived(Bytes),
}

/// Candidate pair state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CandidatePairState {
    /// Initial state (not checked yet)
    Frozen,
    
    /// Waiting for turn (in the check list)
    Waiting,
    
    /// Currently being checked
    InProgress,
    
    /// Check succeeded
    Succeeded,
    
    /// Check failed
    Failed,
}

/// Candidate pair for connectivity checks
#[derive(Debug, Clone)]
struct CandidatePair {
    /// Local candidate
    local: IceCandidate,
    
    /// Remote candidate
    remote: IceCandidate,
    
    /// Priority of the pair
    priority: u64,
    
    /// Current state
    state: CandidatePairState,
    
    /// Is this the nominated pair?
    nominated: bool,
    
    /// Last time this pair was checked
    last_checked: Option<Instant>,
}

impl CandidatePair {
    /// Create a new candidate pair
    fn new(local: IceCandidate, remote: IceCandidate, role: IceRole) -> Self {
        // Compute the pair priority (RFC 8445 Section 6.1.2.3)
        let g = match role {
            IceRole::Controlling => 1,
            IceRole::Controlled => 0,
        };
        
        let local_priority = local.priority as u64;
        let remote_priority = remote.priority as u64;
        
        // Compute pair priority (RFC 8445 Section 6.1.2.3)
        // pairPriority = 2^32*MIN(G,D) + 2*MAX(G,D) + (G>D?1:0)
        let min_priority = std::cmp::min(local_priority, remote_priority);
        let max_priority = std::cmp::max(local_priority, remote_priority);
        
        let priority = (1 << 32) * min_priority + 2 * max_priority + (if local_priority > remote_priority { 1 } else { 0 });
        
        Self {
            local,
            remote,
            priority,
            state: CandidatePairState::Frozen,
            nominated: false,
            last_checked: None,
        }
    }
}

/// ICE agent for NAT traversal
pub struct IceAgent {
    /// ICE configuration
    config: IceConfig,
    
    /// Agent role (controlling or controlled)
    role: Arc<RwLock<IceRole>>,
    
    /// Current state
    state: Arc<RwLock<IceAgentState>>,
    
    /// Local candidates
    local_candidates: Arc<RwLock<Vec<IceCandidate>>>,
    
    /// Remote candidates
    remote_candidates: Arc<RwLock<Vec<IceCandidate>>>,
    
    /// Candidate pairs for connectivity checks
    candidate_pairs: Arc<RwLock<Vec<CandidatePair>>>,
    
    /// Selected pair
    selected_pair: Arc<RwLock<Option<(IceCandidate, IceCandidate)>>>,
    
    /// Local UDP candidates with sockets
    udp_candidates: Arc<RwLock<HashMap<String, Arc<UdpCandidate>>>>,
    
    /// Local TCP candidates with sockets
    tcp_candidates: Arc<RwLock<HashMap<String, Arc<TcpCandidate>>>>,
    
    /// ICE tiebreaker (for role conflicts)
    tiebreaker: u64,
    
    /// Event sender
    event_tx: Arc<Mutex<Option<mpsc::Sender<IceAgentEvent>>>>,
    
    /// Data channel
    data_tx: Arc<Mutex<Option<mpsc::Sender<Bytes>>>>,
}

impl IceAgent {
    /// Create a new ICE agent
    pub async fn new(config: IceConfig, initial_role: IceRole) -> Result<(Self, mpsc::Receiver<IceAgentEvent>, mpsc::Receiver<Bytes>)> {
        debug!("Creating ICE agent with role {:?}", initial_role);
        
        // Create channels for events
        let (event_tx, event_rx) = mpsc::channel(100);
        let (data_tx, data_rx) = mpsc::channel(100);
        
        // Generate a random tiebreaker
        let tiebreaker = random::<u64>();
        
        // Create the agent
        let ice_agent = Self {
            config,
            role: Arc::new(RwLock::new(initial_role)),
            state: Arc::new(RwLock::new(IceAgentState::New)),
            local_candidates: Arc::new(RwLock::new(Vec::new())),
            remote_candidates: Arc::new(RwLock::new(Vec::new())),
            candidate_pairs: Arc::new(RwLock::new(Vec::new())),
            selected_pair: Arc::new(RwLock::new(None)),
            udp_candidates: Arc::new(RwLock::new(HashMap::new())),
            tcp_candidates: Arc::new(RwLock::new(HashMap::new())),
            tiebreaker,
            event_tx: Arc::new(Mutex::new(Some(event_tx))),
            data_tx: Arc::new(Mutex::new(Some(data_tx))),
        };
        
        Ok((ice_agent, event_rx, data_rx))
    }
    
    /// Set the ICE role
    pub async fn set_role(&self, role: IceRole) {
        let mut role_guard = self.role.write().await;
        debug!("Changing ICE role from {:?} to {:?}", *role_guard, role);
        *role_guard = role;
    }
    
    /// Get the current ICE role
    pub async fn role(&self) -> IceRole {
        *self.role.read().await
    }
    
    /// Get current state
    pub async fn state(&self) -> IceAgentState {
        *self.state.read().await
    }
    
    /// Set the ICE agent state
    async fn set_state(&self, new_state: IceAgentState) {
        let current_state = {
            let mut state_guard = self.state.write().await;
            let old_state = *state_guard;
            *state_guard = new_state;
            old_state
        };
        
        if current_state != new_state {
            debug!("ICE agent state changed: {} -> {}", current_state, new_state);
            self.emit_event(IceAgentEvent::StateChange(new_state)).await;
        }
    }
    
    /// Emit an event to listeners
    async fn emit_event(&self, event: IceAgentEvent) {
        let event_tx = {
            let guard = self.event_tx.lock().await;
            guard.clone()
        };
        
        if let Some(tx) = event_tx {
            if tx.send(event).await.is_err() {
                error!("Failed to send event - receiver dropped");
            }
        }
    }
    
    /// Start gathering ICE candidates
    pub async fn gather_candidates(&self) -> Result<()> {
        // Set state to gathering
        self.set_state(IceAgentState::Gathering).await;
        
        // Emit gathering state change event (started)
        self.emit_event(IceAgentEvent::GatheringStateChange(false)).await;
        
        // Use a local variable to track discovery
        let gathering_policy = self.config.gathering_policy;
        let mut discovery_complete = false;
        
        // Gather host candidates if enabled
        if (gathering_policy == GatheringPolicy::All || 
            gathering_policy == GatheringPolicy::HostOnly) && 
           self.config.gather_host {
            self.gather_host_candidates().await?;
        }
        
        // For this simplified implementation, we'll skip actual STUN/TURN gathering
        // and just set a timeout after which we consider gathering complete
        let max_gathering_time = Duration::from_millis(self.config.max_gathering_time_ms);
        
        // Spawn a task to handle gathering timeout
        let self_arc = Arc::new(self.clone());
        tokio::spawn(async move {
            sleep(max_gathering_time).await;
            
            if !discovery_complete {
                // Set gathering complete
                self_arc.set_state(IceAgentState::Complete).await;
                
                // Emit gathering state change event (completed)
                self_arc.emit_event(IceAgentEvent::GatheringStateChange(true)).await;
            }
        });
        
        Ok(())
    }
    
    /// Gather host candidates
    async fn gather_host_candidates(&self) -> Result<()> {
        debug!("Gathering host candidates");
        
        // Simplified address discovery for host candidates
        // In a real implementation, we would discover all network interfaces
        
        // For this demo, we'll create a few dummy host candidates
        if self.config.use_udp {
            // Create a UDP socket and candidate for component 1 (RTP)
            let socket = UdpSocket::bind("0.0.0.0:0").await?;
            let candidate = UdpCandidate::new(
                socket,
                1, // RTP component
                CandidateType::Host,
                None,
            ).await?;
            
            // Add the candidate
            self.add_local_candidate_from_udp(Arc::new(candidate)).await?;
        }
        
        if self.config.use_tcp {
            // Create a TCP socket for component 1 (RTP)
            let socket = TcpSocket::new_v4()?;
            socket.bind(SocketAddr::new(IpAddr::from([0, 0, 0, 0]), 0))?;
            
            let candidate = TcpCandidate::new(
                socket,
                TransportType::TcpPassive,
                1, // RTP component
                CandidateType::Host,
                None,
            ).await?;
            
            // Add the candidate
            self.add_local_candidate_from_tcp(Arc::new(candidate)).await?;
        }
        
        Ok(())
    }
    
    /// Add a UDP candidate to the agent
    async fn add_local_candidate_from_udp(&self, candidate: Arc<UdpCandidate>) -> Result<()> {
        let foundation = candidate.get_info().foundation.clone();
        
        // Store candidate
        {
            // Store the socket in the udp_candidates map
            let mut candidates_map = self.udp_candidates.write().await;
            candidates_map.insert(foundation.clone(), candidate.clone());
            
            // Add the candidate info to local_candidates
            let mut local_candidates = self.local_candidates.write().await;
            local_candidates.push(candidate.get_info().clone());
        }
        
        // Emit new candidate event
        self.emit_event(IceAgentEvent::NewCandidate(candidate.get_info().clone())).await;
        
        // Setup receiver for incoming data
        let self_arc = Arc::new(self.clone());
        let candidate_clone = candidate.clone();
        let candidate_info = candidate.get_info().clone();
        
        tokio::spawn(async move {
            // Get a new receiver for this task
            let mut receiver = candidate_clone.get_data_receiver();
            while let Some((data, remote_addr)) = receiver.recv().await {
                if let Err(e) = self_arc.handle_incoming_data(&data, remote_addr, &candidate_info).await {
                    error!("Error handling incoming data: {}", e);
                }
            }
        });
        
        Ok(())
    }
    
    /// Add a TCP candidate to the agent
    async fn add_local_candidate_from_tcp(&self, candidate: Arc<TcpCandidate>) -> Result<()> {
        let foundation = candidate.get_info().foundation.clone();
        
        // Store candidate
        {
            // Store the socket in the tcp_candidates map
            let mut candidates_map = self.tcp_candidates.write().await;
            candidates_map.insert(foundation.clone(), candidate.clone());
            
            // Add the candidate info to local_candidates
            let mut local_candidates = self.local_candidates.write().await;
            local_candidates.push(candidate.get_info().clone());
        }
        
        // Emit new candidate event
        self.emit_event(IceAgentEvent::NewCandidate(candidate.get_info().clone())).await;
        
        // Setup receiver for incoming data
        let self_arc = Arc::new(self.clone());
        let candidate_clone = candidate.clone();
        let candidate_info = candidate.get_info().clone();
        
        tokio::spawn(async move {
            // Get a new receiver for this task
            let mut receiver = candidate_clone.get_data_receiver();
            while let Some((data, remote_addr)) = receiver.recv().await {
                if let Err(e) = self_arc.handle_incoming_data(&data, remote_addr, &candidate_info).await {
                    error!("Error handling incoming data: {}", e);
                }
            }
        });
        
        Ok(())
    }
    
    /// Handle incoming data from a socket
    async fn handle_incoming_data(&self, data: &[u8], remote_addr: SocketAddr, local_candidate: &IceCandidate) -> Result<()> {
        // Check if the data might be a STUN message
        if data.len() >= 20 && (data[0] & 0xC0) == 0 {
            // Try to parse as STUN
            match StunMessage::decode(data) {
                Ok(stun_msg) => {
                    self.handle_stun_message(stun_msg, remote_addr, local_candidate).await?;
                    return Ok(());
                }
                Err(e) => {
                    trace!("Failed to parse as STUN: {}", e);
                    // Fall through and treat as data
                }
            }
        }
        
        // If we're here, it's application data
        if self.state().await == IceAgentState::Connected {
            // Forward to the data channel
            let data_tx = {
                let guard = self.data_tx.lock().await;
                guard.clone()
            };
            
            if let Some(tx) = data_tx {
                // Clone the data for sending
                let data_bytes = Bytes::copy_from_slice(data);
                if tx.send(data_bytes).await.is_err() {
                    error!("Failed to send data - receiver dropped");
                }
            }
            
            // Also emit as an event for backward compatibility
            let data_bytes = Bytes::copy_from_slice(data);
            self.emit_event(IceAgentEvent::DataReceived(data_bytes)).await;
        } else {
            debug!("Received data but not in Connected state, ignoring");
        }
        
        Ok(())
    }
    
    /// Handle STUN message
    async fn handle_stun_message(&self, msg: StunMessage, remote_addr: SocketAddr, local_candidate: &IceCandidate) -> Result<()> {
        match msg.msg_type {
            StunMessageType::BindingRequest => {
                self.handle_binding_request(msg, remote_addr, local_candidate).await?;
            }
            StunMessageType::BindingResponse => {
                self.handle_binding_response(msg, remote_addr, local_candidate).await?;
            }
            StunMessageType::BindingErrorResponse => {
                warn!("Received binding error response");
                // Handle error response
            }
            _ => {
                debug!("Received unhandled STUN message type: {:?}", msg.msg_type);
            }
        }
        
        Ok(())
    }
    
    /// Handle STUN binding request
    async fn handle_binding_request(&self, msg: StunMessage, remote_addr: SocketAddr, local_candidate: &IceCandidate) -> Result<()> {
        debug!("Handling STUN binding request from {}", remote_addr);
        
        // Create a binding response
        let mut response = StunMessage::binding_response();
        response.transaction_id = msg.transaction_id;
        
        // Add XOR-MAPPED-ADDRESS attribute
        let attr = StunAttribute::xor_mapped_address(remote_addr, &msg.transaction_id);
        response.add_attribute(attr);
        
        // Add SOFTWARE attribute
        let software = format!("rvoip-ice-core/{}", env!("CARGO_PKG_VERSION"));
        let attr = StunAttribute::software(&software);
        response.add_attribute(attr);
        
        // Find the candidate with the matching foundation
        let candidate_foundation = local_candidate.foundation.clone();
        
        // Send the response
        let encoded = response.encode();
        
        // If UDP
        if !local_candidate.transport.is_tcp() {
            let udp_candidates = self.udp_candidates.read().await;
            if let Some(candidate) = udp_candidates.get(&candidate_foundation) {
                candidate.send_to(&encoded, remote_addr).await?;
            } else {
                return Err(Error::IceError("Local candidate not found".to_string()));
            }
        } else {
            // If TCP
            let tcp_candidates = self.tcp_candidates.read().await;
            if let Some(candidate) = tcp_candidates.get(&candidate_foundation) {
                candidate.send_to(&encoded, remote_addr).await?;
            } else {
                return Err(Error::IceError("Local candidate not found".to_string()));
            }
        }
        
        Ok(())
    }
    
    /// Handle STUN binding response
    async fn handle_binding_response(&self, msg: StunMessage, remote_addr: SocketAddr, local_candidate: &IceCandidate) -> Result<()> {
        debug!("Handling STUN binding response from {}", remote_addr);
        
        // Extract XOR-MAPPED-ADDRESS from the response
        if let Some(attr) = msg.get_attribute(crate::stun::StunAttributeType::XorMappedAddress) {
            let mapped_addr = attr.get_xor_mapped_address(&msg.transaction_id)?;
            debug!("Our address as seen by the remote side: {}", mapped_addr);
            
            // This could be used to create a server-reflexive candidate
            // For now, we'll skip that in this simplified implementation
        }
        
        Ok(())
    }
    
    /// Add a remote candidate
    pub async fn add_remote_candidate(&self, candidate: IceCandidate) -> Result<()> {
        debug!("Adding remote candidate: {}", candidate.to_sdp_string());
        
        // Store candidate
        {
            let mut candidates = self.remote_candidates.write().await;
            
            // Check for duplicates
            if candidates.iter().any(|c| c.foundation == candidate.foundation) {
                debug!("Ignoring duplicate remote candidate");
                return Ok(());
            }
            
            candidates.push(candidate.clone());
        }
        
        // Form pairs with local candidates
        self.form_candidate_pairs(candidate).await?;
        
        Ok(())
    }
    
    /// Form candidate pairs with a new remote candidate
    async fn form_candidate_pairs(&self, remote_candidate: IceCandidate) -> Result<()> {
        let local_candidates = self.local_candidates.read().await;
        let current_role = self.role().await;
        
        let mut new_pairs = Vec::new();
        
        for local_candidate in local_candidates.iter() {
            // Only pair candidates with matching component IDs
            if local_candidate.component != remote_candidate.component {
                continue;
            }
            
            // Create a new candidate pair
            let pair = CandidatePair::new(local_candidate.clone(), remote_candidate.clone(), current_role);
            new_pairs.push(pair);
        }
        
        // Add the new pairs to the checklist
        let mut checklist = self.candidate_pairs.write().await;
        checklist.extend(new_pairs);
        
        // Sort the checklist by priority (highest first)
        checklist.sort_by(|a, b| b.priority.cmp(&a.priority));
        
        Ok(())
    }
    
    /// Start connectivity checks
    pub async fn start_connectivity_checks(&self) -> Result<()> {
        if self.state().await == IceAgentState::New || self.state().await == IceAgentState::Complete {
            // Set state to checking
            self.set_state(IceAgentState::Checking).await;
            
            // Start checking candidate pairs
            self.check_candidate_pairs().await?;
        } else {
            debug!("Cannot start connectivity checks in current state: {:?}", self.state().await);
        }
        
        Ok(())
    }
    
    /// Check candidate pairs
    async fn check_candidate_pairs(&self) -> Result<()> {
        debug!("Starting connectivity checks");
        
        // For this simplified implementation, we'll just pick the highest priority pair
        // and mark it as successful
        
        let mut selected_pair = None;
        
        {
            let mut pairs = self.candidate_pairs.write().await;
            if let Some(pair) = pairs.first_mut() {
                debug!("Selected highest priority pair for connectivity");
                
                // Mark the pair as successful
                pair.state = CandidatePairState::Succeeded;
                pair.nominated = true;
                pair.last_checked = Some(Instant::now());
                
                selected_pair = Some((pair.local.clone(), pair.remote.clone()));
            }
        }
        
        // If we found a pair, set it as selected
        if let Some((local, remote)) = selected_pair {
            debug!("Setting selected pair: {} <-> {}", local.address(), remote.address());
            
            // Store the selected pair
            {
                let mut selected = self.selected_pair.write().await;
                *selected = Some((local.clone(), remote.clone()));
            }
            
            // Emit selected pair change event
            self.emit_event(IceAgentEvent::SelectedPairChange {
                local,
                remote,
            }).await;
            
            // Transition to Connected state
            self.set_state(IceAgentState::Connected).await;
        } else {
            debug!("No candidate pairs available for checks");
            
            // Transition to Failed state
            self.set_state(IceAgentState::Failed).await;
        }
        
        Ok(())
    }
    
    /// Send data
    pub async fn send_data(&self, data: &[u8]) -> Result<()> {
        if self.state().await != IceAgentState::Connected {
            return Err(Error::InvalidState("Cannot send data when not connected".to_string()));
        }
        
        let selected_pair = {
            let selected = self.selected_pair.read().await;
            selected.clone()
        };
        
        if let Some((local, remote)) = selected_pair {
            let local_foundation = local.foundation.clone();
            
            if !local.transport.is_tcp() {
                // Send over UDP
                let udp_candidates = self.udp_candidates.read().await;
                if let Some(candidate) = udp_candidates.get(&local_foundation) {
                    candidate.send_to(data, remote.address()).await?;
                    return Ok(());
                }
            } else {
                // Send over TCP
                let tcp_candidates = self.tcp_candidates.read().await;
                if let Some(candidate) = tcp_candidates.get(&local_foundation) {
                    candidate.send_to(data, remote.address()).await?;
                    return Ok(());
                }
            }
            
            return Err(Error::IceError("Selected candidate not found".to_string()));
        }
        
        Err(Error::IceError("No selected candidate pair".to_string()))
    }
    
    /// Get local candidates
    pub async fn local_candidates(&self) -> Vec<IceCandidate> {
        self.local_candidates.read().await.clone()
    }
    
    /// Get remote candidates
    pub async fn remote_candidates(&self) -> Vec<IceCandidate> {
        self.remote_candidates.read().await.clone()
    }
    
    /// Get selected candidate pair
    pub async fn selected_pair(&self) -> Option<(IceCandidate, IceCandidate)> {
        self.selected_pair.read().await.clone()
    }
    
    /// Close the ICE agent
    pub async fn close(&self) -> Result<()> {
        debug!("Closing ICE agent");
        
        // Set state to closed
        self.set_state(IceAgentState::Closed).await;
        
        // Close event and data channels
        {
            let mut event_tx = self.event_tx.lock().await;
            *event_tx = None;
            
            let mut data_tx = self.data_tx.lock().await;
            *data_tx = None;
        }
        
        Ok(())
    }
}

impl Clone for IceAgent {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            role: self.role.clone(),
            state: self.state.clone(),
            local_candidates: self.local_candidates.clone(),
            remote_candidates: self.remote_candidates.clone(),
            candidate_pairs: self.candidate_pairs.clone(),
            selected_pair: self.selected_pair.clone(),
            udp_candidates: self.udp_candidates.clone(),
            tcp_candidates: self.tcp_candidates.clone(),
            tiebreaker: self.tiebreaker,
            event_tx: self.event_tx.clone(),
            data_tx: self.data_tx.clone(),
        }
    }
} 