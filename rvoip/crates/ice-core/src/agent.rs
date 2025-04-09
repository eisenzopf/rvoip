use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, RwLock, mpsc};
use tokio::net::UdpSocket;
use webrtc_ice::agent::{Agent, AgentConfig};
use webrtc_ice::url::Url;
use webrtc_ice::state::ConnectionState;
use webrtc_ice::network_type::NetworkType;
use tracing::{debug, error, info, warn};

use crate::candidate::{IceCandidate, CandidateType};
use crate::config::{IceConfig, IceServerConfig};
use crate::error::{Error, Result};

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

impl From<ConnectionState> for IceAgentState {
    fn from(state: ConnectionState) -> Self {
        match state {
            ConnectionState::New => IceAgentState::New,
            ConnectionState::Checking => IceAgentState::Checking,
            ConnectionState::Connected => IceAgentState::Connected,
            ConnectionState::Completed => IceAgentState::Connected, // map to Connected
            ConnectionState::Failed => IceAgentState::Failed,
            ConnectionState::Disconnected => IceAgentState::Disconnected,
            ConnectionState::Closed => IceAgentState::Closed,
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
    
    /// Gathering state change
    GatheringStateChange(bool),
    
    /// Selected pair changed
    SelectedPairChange {
        local: IceCandidate,
        remote: IceCandidate,
    },
    
    /// Data received
    DataReceived(Vec<u8>),
}

/// ICE agent for NAT traversal
pub struct IceAgent {
    /// WebRTC ICE agent
    agent: Arc<Agent>,
    
    /// ICE configuration
    config: IceConfig,
    
    /// Current state
    state: Arc<RwLock<IceAgentState>>,
    
    /// Local candidates
    local_candidates: Arc<RwLock<Vec<IceCandidate>>>,
    
    /// Remote candidates
    remote_candidates: Arc<RwLock<Vec<IceCandidate>>>,
    
    /// Selected pair
    selected_pair: Arc<RwLock<Option<(IceCandidate, IceCandidate)>>>,
    
    /// Event sender
    event_tx: Arc<Mutex<Option<mpsc::Sender<IceAgentEvent>>>>,
}

impl IceAgent {
    /// Create a new ICE agent
    pub async fn new(config: IceConfig) -> Result<(Self, mpsc::Receiver<IceAgentEvent>)> {
        // Create ICE URLs
        let mut urls = Vec::new();
        for server in &config.servers {
            let url = Url::parse(&server.url)
                .map_err(|e| Error::ConfigError(format!("Failed to parse ICE server URL: {}", e)))?;
            
            // Add authentication if necessary (for TURN)
            let url = if url.scheme == "turn" || url.scheme == "turns" {
                if let (Some(username), Some(credential)) = (&server.username, &server.credential) {
                    url.with_username(username.clone())
                        .map_err(|e| Error::ConfigError(format!("Failed to set username: {}", e)))?
                        .with_password(credential.clone())
                        .map_err(|e| Error::ConfigError(format!("Failed to set credential: {}", e)))?
                } else {
                    url
                }
            } else {
                url
            };
            
            urls.push(url);
        }
        
        // Create agent configuration
        let mut agent_config = AgentConfig::default();
        
        // Configure network types based on config
        let mut network_types = Vec::new();
        
        if config.use_udp {
            network_types.push(NetworkType::Udp4);
            network_types.push(NetworkType::Udp6);
        }
        
        if config.use_tcp {
            network_types.push(NetworkType::Tcp4);
            network_types.push(NetworkType::Tcp6);
        }
        
        agent_config.network_types = network_types;
        agent_config.udp_network_timeout = Some(config.timeout);
        agent_config.max_binding_requests = Some(7); // RFC recommended value
        
        // Create the ICE agent
        let agent = Agent::new(agent_config)
            .await
            .map_err(|e| Error::IceError(format!("Failed to create ICE agent: {}", e)))?;
        
        // Add ICE servers
        for url in urls {
            agent.add_url(url).await
                .map_err(|e| Error::IceError(format!("Failed to add ICE server: {}", e)))?;
        }
        
        // Create a channel for events
        let (event_tx, event_rx) = mpsc::channel(100);
        
        // Create the agent
        let ice_agent = Self {
            agent: Arc::new(agent),
            config,
            state: Arc::new(RwLock::new(IceAgentState::New)),
            local_candidates: Arc::new(RwLock::new(Vec::new())),
            remote_candidates: Arc::new(RwLock::new(Vec::new())),
            selected_pair: Arc::new(RwLock::new(None)),
            event_tx: Arc::new(Mutex::new(Some(event_tx))),
        };
        
        // Setup event handling
        ice_agent.setup_event_handlers().await?;
        
        Ok((ice_agent, event_rx))
    }
    
    /// Setup event handlers for the ICE agent
    async fn setup_event_handlers(&self) -> Result<()> {
        // Clone references
        let agent = self.agent.clone();
        let state = self.state.clone();
        let local_candidates = self.local_candidates.clone();
        let selected_pair = self.selected_pair.clone();
        let event_tx = self.event_tx.clone();
        
        // Handle state changes
        let state_tx = {
            let tx = event_tx.lock().await.clone();
            let state_clone = state.clone();
            agent.on_connection_state_change(Box::new(move |s| {
                let state_change = IceAgentState::from(s);
                
                // Update state
                if let Ok(mut current_state) = state_clone.write() {
                    *current_state = state_change;
                }
                
                // Send event
                if let Some(tx) = &tx {
                    let _ = tx.try_send(IceAgentEvent::StateChange(state_change));
                }
                
                Box::pin(async {})
            })).await;
            tx
        };
        
        // Handle candidate gathering state changes
        let gathering_tx = {
            let tx = state_tx.clone();
            agent.on_gathering_state_change(Box::new(move |s| {
                let complete = s.is_complete();
                
                // Send event
                if let Some(tx) = &tx {
                    let _ = tx.try_send(IceAgentEvent::GatheringStateChange(complete));
                }
                
                Box::pin(async {})
            })).await;
            tx
        };
        
        // Handle new candidates
        let candidate_tx = {
            let tx = gathering_tx.clone();
            let local_candidates_clone = local_candidates.clone();
            agent.on_candidate(Box::new(move |c| {
                if let Some(c) = c {
                    // Convert to our candidate type
                    let candidate = convert_webrtc_to_candidate(&c);
                    
                    // Store candidate
                    if let Ok(mut candidates) = local_candidates_clone.write() {
                        candidates.push(candidate.clone());
                    }
                    
                    // Send event
                    if let Some(tx) = &tx {
                        let _ = tx.try_send(IceAgentEvent::NewCandidate(candidate));
                    }
                }
                
                Box::pin(async {})
            })).await;
            tx
        };
        
        // Handle selected candidate pair
        let selected_tx = {
            let tx = candidate_tx.clone();
            let selected_pair_clone = selected_pair.clone();
            agent.on_selected_candidate_pair_change(Box::new(move |p| {
                if let Some((local, remote)) = p {
                    // Convert to our candidate types
                    let local_candidate = convert_webrtc_to_candidate(&local);
                    let remote_candidate = convert_webrtc_to_candidate(&remote);
                    
                    // Store pair
                    if let Ok(mut pair) = selected_pair_clone.write() {
                        *pair = Some((local_candidate.clone(), remote_candidate.clone()));
                    }
                    
                    // Send event
                    if let Some(tx) = &tx {
                        let _ = tx.try_send(IceAgentEvent::SelectedPairChange {
                            local: local_candidate,
                            remote: remote_candidate,
                        });
                    }
                }
                
                Box::pin(async {})
            })).await;
            tx
        };
        
        // Handle data reception
        let _ = {
            let tx = selected_tx;
            agent.on_data(Box::new(move |data| {
                // Send event
                if let Some(tx) = &tx {
                    let _ = tx.try_send(IceAgentEvent::DataReceived(data.to_vec()));
                }
                
                Box::pin(async {})
            })).await;
        };
        
        Ok(())
    }
    
    /// Start gathering ICE candidates
    pub async fn gather_candidates(&self) -> Result<()> {
        // Start gathering
        self.agent.gather_candidates()
            .await
            .map_err(|e| Error::IceError(format!("Failed to gather candidates: {}", e)))?;
        
        Ok(())
    }
    
    /// Add a remote candidate
    pub async fn add_remote_candidate(&self, candidate: IceCandidate) -> Result<()> {
        // Convert to webrtc-ice candidate
        let webrtc_candidate = convert_candidate_to_webrtc(&candidate)
            .map_err(|e| Error::InvalidCandidate(e))?;
        
        // Add candidate to agent
        self.agent.add_remote_candidate(&webrtc_candidate)
            .await
            .map_err(|e| Error::IceError(format!("Failed to add remote candidate: {}", e)))?;
        
        // Store candidate
        {
            let mut candidates = self.remote_candidates.write().await;
            candidates.push(candidate);
        }
        
        Ok(())
    }
    
    /// Start connectivity checks
    pub async fn start_connectivity_checks(&self) -> Result<()> {
        // Start connecting
        self.agent.connect()
            .await
            .map_err(|e| Error::IceError(format!("Failed to start connectivity checks: {}", e)))?;
        
        Ok(())
    }
    
    /// Send data
    pub async fn send_data(&self, data: &[u8]) -> Result<()> {
        // Send data
        self.agent.write(data)
            .await
            .map_err(|e| Error::IceError(format!("Failed to send data: {}", e)))?;
        
        Ok(())
    }
    
    /// Get current state
    pub async fn state(&self) -> IceAgentState {
        *self.state.read().await
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
        // Close the agent
        self.agent.close()
            .await
            .map_err(|e| Error::IceError(format!("Failed to close ICE agent: {}", e)))?;
        
        // Clear event sender
        *self.event_tx.lock().await = None;
        
        Ok(())
    }
}

/// Convert our candidate type to webrtc-ice candidate
fn convert_candidate_to_webrtc(candidate: &IceCandidate) -> std::result::Result<webrtc_ice::candidate::Candidate, String> {
    let candidate_type = match candidate.candidate_type {
        CandidateType::Host => webrtc_ice::candidate::CandidateType::Host,
        CandidateType::ServerReflexive => webrtc_ice::candidate::CandidateType::ServerReflexive,
        CandidateType::PeerReflexive => webrtc_ice::candidate::CandidateType::PeerReflexive,
        CandidateType::Relay => webrtc_ice::candidate::CandidateType::Relay,
    };
    
    let address = SocketAddr::new(candidate.ip, candidate.port);
    let related_address = if let (Some(ip), Some(port)) = (candidate.related_address, candidate.related_port) {
        Some(SocketAddr::new(ip, port))
    } else {
        None
    };
    
    // Create candidate
    let mut webrtc_candidate = webrtc_ice::candidate::Candidate::new(
        None, // agent will fill this
        None, // agent will fill this
        None, // agent will fill this
        None, // agent will fill this
        Some(candidate.foundation.clone()),
        Some(candidate.priority),
        Some(address),
        Some(candidate_type),
        Some(related_address),
    );
    
    // Set protocol
    match candidate.protocol.to_lowercase().as_str() {
        "udp" => webrtc_candidate.transport = "UDP".to_string(),
        "tcp" => webrtc_candidate.transport = "TCP".to_string(),
        "tls" => webrtc_candidate.transport = "TLS".to_string(),
        "dtls" => webrtc_candidate.transport = "DTLS".to_string(),
        _ => return Err(format!("Unknown protocol: {}", candidate.protocol)),
    }
    
    Ok(webrtc_candidate)
}

/// Convert webrtc-ice candidate to our candidate type
fn convert_webrtc_to_candidate(candidate: &webrtc_ice::candidate::Candidate) -> IceCandidate {
    let candidate_type = match candidate.candidate_type {
        webrtc_ice::candidate::CandidateType::Host => CandidateType::Host,
        webrtc_ice::candidate::CandidateType::ServerReflexive => CandidateType::ServerReflexive,
        webrtc_ice::candidate::CandidateType::PeerReflexive => CandidateType::PeerReflexive,
        webrtc_ice::candidate::CandidateType::Relay => CandidateType::Relay,
        _ => CandidateType::Host, // Default to host for unknown types
    };
    
    IceCandidate {
        foundation: candidate.foundation.clone(),
        component: candidate.component as u32,
        protocol: candidate.transport.to_string(),
        priority: candidate.priority,
        ip: candidate.address.ip(),
        port: candidate.address.port(),
        candidate_type,
        related_address: candidate.related_address.as_ref().map(|a| a.ip()),
        related_port: candidate.related_address.as_ref().map(|a| a.port()),
        tcp_type: None, // Not available in webrtc-ice
    }
} 