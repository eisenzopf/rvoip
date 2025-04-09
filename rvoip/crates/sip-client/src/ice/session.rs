use std::sync::Arc;
use tokio::sync::RwLock;
use anyhow::Result;
use tracing::{debug, info, warn, error};

use rvoip_ice_core::{
    IceAgent, IceAgentState, IceAgentEvent, IceCandidate, IceConfig,
    IceRole, IceComponent, TransportType, CandidateType
};

use super::candidate::SipIceCandidate;

/// ICE session for a SIP call
/// Manages the lifecycle of an ICE session following pjsip's model
pub struct IceSession {
    /// The underlying ICE agent
    agent: Arc<IceAgent>,
    
    /// The current state of the session
    state: Arc<RwLock<IceSessionState>>,
    
    /// Local candidates
    local_candidates: Arc<RwLock<Vec<IceCandidate>>>,
    
    /// Remote candidates
    remote_candidates: Arc<RwLock<Vec<IceCandidate>>>,
    
    /// Configuration
    config: IceConfig,
}

/// State of the ICE session
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IceSessionState {
    /// Session is not started
    New,
    
    /// Gathering local candidates
    Gathering,
    
    /// Local candidates have been gathered
    GatheringComplete,
    
    /// Checking connectivity with remote candidates
    Checking,
    
    /// Connection has been established
    Connected,
    
    /// Session has been terminated
    Terminated,
    
    /// Session has failed
    Failed,
}

impl IceSession {
    /// Create a new ICE session
    pub async fn new(config: IceConfig) -> Result<Self> {
        let (agent, _, _) = IceAgent::new(config.clone(), IceRole::Controlling).await?;
        
        Ok(Self {
            agent: Arc::new(agent),
            state: Arc::new(RwLock::new(IceSessionState::New)),
            local_candidates: Arc::new(RwLock::new(Vec::new())),
            remote_candidates: Arc::new(RwLock::new(Vec::new())),
            config,
        })
    }
    
    /// Start gathering local candidates
    pub async fn start_gathering(&self) -> Result<()> {
        *self.state.write().await = IceSessionState::Gathering;
        
        // Start the ICE agent to gather candidates
        self.agent.gather_candidates().await?;
        
        Ok(())
    }
    
    /// Add a remote candidate
    pub async fn add_remote_candidate(&self, candidate: IceCandidate) -> Result<()> {
        self.remote_candidates.write().await.push(candidate.clone());
        self.agent.add_remote_candidate(candidate).await?;
        
        Ok(())
    }
    
    /// Get all local candidates
    pub async fn local_candidates(&self) -> Vec<IceCandidate> {
        self.agent.local_candidates().await
    }
    
    /// Get the current session state
    pub async fn state(&self) -> IceSessionState {
        *self.state.read().await
    }
    
    /// Get the selected candidate pair if available
    pub async fn selected_pair(&self) -> Option<(IceCandidate, IceCandidate)> {
        self.agent.selected_pair().await
    }
    
    /// Handle agent events and update session state
    pub async fn handle_agent_event(&self, event: IceAgentEvent) -> Result<()> {
        match event {
            IceAgentEvent::StateChange(agent_state) => {
                match agent_state {
                    IceAgentState::Gathering => {
                        *self.state.write().await = IceSessionState::Gathering;
                    }
                    IceAgentState::Complete => {
                        *self.state.write().await = IceSessionState::GatheringComplete;
                    }
                    IceAgentState::Checking => {
                        *self.state.write().await = IceSessionState::Checking;
                    }
                    IceAgentState::Connected => {
                        *self.state.write().await = IceSessionState::Connected;
                    }
                    IceAgentState::Failed => {
                        *self.state.write().await = IceSessionState::Failed;
                    }
                    IceAgentState::Closed => {
                        *self.state.write().await = IceSessionState::Terminated;
                    }
                    _ => {}
                }
            }
            IceAgentEvent::NewCandidate(candidate) => {
                self.local_candidates.write().await.push(candidate);
            }
            _ => {}
        }
        
        Ok(())
    }
    
    /// Convert a list of candidates to SDP format for SIP messages
    pub fn candidates_to_sdp(candidates: &[IceCandidate]) -> Vec<String> {
        candidates.iter().map(|c| c.to_sdp_line()).collect()
    }
    
    /// Terminate the ICE session
    pub async fn terminate(&self) -> Result<()> {
        *self.state.write().await = IceSessionState::Terminated;
        self.agent.close().await?;
        
        Ok(())
    }
} 