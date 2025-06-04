//! Session Group Management
//! 
//! This module provides sophisticated group management for related sessions:
//! 
//! - Conference session groups with dynamic membership
//! - Transfer session groups for call transfer coordination
//! - Bridge session groups for multi-party calls
//! - Custom session groups with flexible policies
//! - Group lifecycle management and state coordination
//! - Group event propagation and synchronization
//! 
//! ## Integration with Existing Bridge Infrastructure
//! 
//! This module **enhances and coordinates with** the existing `bridge.rs` infrastructure:
//! 
//! ### Two-Layer Architecture:
//! 
//! **Layer 1: Media Bridge (`bridge.rs`)**
//! - Low-level media routing and audio mixing
//! - RTP packet handling and media transport
//! - Audio bridging between sessions
//! - Technical media infrastructure
//! 
//! **Layer 2: Session Coordination (`groups.rs`)**  
//! - High-level session relationship management
//! - Call coordination patterns (conference, transfer, consultation)
//! - Session lifecycle synchronization
//! - Business logic and call flow management
//! 
//! ### Integration Example:
//! ```rust
//! // Create a conference call:
//! // 1. Session group manages participants and coordination
//! let group_id = group_manager.create_group(GroupType::Conference, config).await?;
//! 
//! // 2. Media bridge handles actual audio routing  
//! let bridge = SessionBridge::new(bridge_config);
//! group.set_bridge_id(bridge.id.clone());
//! 
//! // 3. Add participants to both layers
//! group_manager.add_session_with_bridge(group_id, session_id, "participant", 
//!     |bridge_id, session_id| bridge.add_session(session_id)).await?;
//! ```
//! 
//! ### Benefits of Integration:
//! - **Separation of Concerns**: Media vs coordination logic
//! - **Flexibility**: Groups can exist without media bridges (transfers, queues)
//! - **Consistency**: Coordinated session and media management
//! - **Scalability**: Independent scaling of coordination and media layers

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use dashmap::DashMap;
use tokio::sync::RwLock;
use tracing::{debug, info, warn, error};
use serde::{Serialize, Deserialize};
use uuid::Uuid;

use crate::session::{SessionId, SessionState};
use crate::errors::{Error, ErrorContext};
use crate::session::bridge::{SessionBridge, BridgeId, BridgeConfig};

/// Types of session groups
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GroupType {
    /// Conference group (multiple participants)
    Conference,
    
    /// Transfer group (source, target, consultation sessions)
    Transfer,
    
    /// Bridge group (sessions connected via media bridge)
    Bridge,
    
    /// Consultation group (main call + consultation call)
    Consultation,
    
    /// Queue group (sessions waiting in a queue)
    Queue,
    
    /// Hunt group (sessions for hunting/forwarding)
    Hunt,
    
    /// Custom group with user-defined behavior
    Custom,
}

impl std::fmt::Display for GroupType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GroupType::Conference => write!(f, "Conference"),
            GroupType::Transfer => write!(f, "Transfer"),
            GroupType::Bridge => write!(f, "Bridge"),
            GroupType::Consultation => write!(f, "Consultation"),
            GroupType::Queue => write!(f, "Queue"),
            GroupType::Hunt => write!(f, "Hunt"),
            GroupType::Custom => write!(f, "Custom"),
        }
    }
}

/// State of a session group
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GroupState {
    /// Group is being initialized
    Initializing,
    
    /// Group is active and coordinating sessions
    Active,
    
    /// Group is temporarily suspended
    Suspended,
    
    /// Group is being terminated
    Terminating,
    
    /// Group has been terminated
    Terminated,
    
    /// Group failed to operate properly
    Failed,
}

impl std::fmt::Display for GroupState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GroupState::Initializing => write!(f, "Initializing"),
            GroupState::Active => write!(f, "Active"),
            GroupState::Suspended => write!(f, "Suspended"),
            GroupState::Terminating => write!(f, "Terminating"),
            GroupState::Terminated => write!(f, "Terminated"),
            GroupState::Failed => write!(f, "Failed"),
        }
    }
}

/// Configuration for session group behavior
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupConfig {
    /// Maximum number of sessions in the group
    pub max_sessions: Option<usize>,
    
    /// Minimum number of sessions required for group to be active
    pub min_sessions: usize,
    
    /// Whether to auto-terminate group when membership falls below minimum
    pub auto_terminate_on_min: bool,
    
    /// Whether to allow dynamic membership changes
    pub allow_dynamic_membership: bool,
    
    /// Whether to synchronize session states within the group
    pub synchronize_states: bool,
    
    /// Whether to propagate events across group members
    pub propagate_events: bool,
    
    /// Timeout for group operations
    pub operation_timeout: Duration,
    
    /// Group-specific metadata
    pub metadata: HashMap<String, String>,
}

impl Default for GroupConfig {
    fn default() -> Self {
        Self {
            max_sessions: Some(100),
            min_sessions: 1,
            auto_terminate_on_min: false,
            allow_dynamic_membership: true,
            synchronize_states: false,
            propagate_events: true,
            operation_timeout: Duration::from_secs(30),
            metadata: HashMap::new(),
        }
    }
}

/// Session membership information in a group
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMembership {
    /// Session ID
    pub session_id: SessionId,
    
    /// Role of the session in the group
    pub role: String,
    
    /// When the session joined the group
    pub joined_at: SystemTime,
    
    /// Last activity time
    pub last_activity: SystemTime,
    
    /// Whether the session is active in the group
    pub active: bool,
    
    /// Session-specific metadata within the group
    pub metadata: HashMap<String, String>,
}

impl SessionMembership {
    /// Create a new session membership
    pub fn new(session_id: SessionId, role: String) -> Self {
        let now = SystemTime::now();
        Self {
            session_id,
            role,
            joined_at: now,
            last_activity: now,
            active: true,
            metadata: HashMap::new(),
        }
    }
    
    /// Update the last activity time
    pub fn update_activity(&mut self) {
        self.last_activity = SystemTime::now();
    }
    
    /// Set session as active/inactive
    pub fn set_active(&mut self, active: bool) {
        self.active = active;
        self.update_activity();
    }
    
    /// Add metadata
    pub fn add_metadata(&mut self, key: String, value: String) {
        self.metadata.insert(key, value);
        self.update_activity();
    }
}

/// Group event types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GroupEvent {
    /// Group was created
    GroupCreated {
        group_id: String,
        group_type: GroupType,
    },
    
    /// Session joined the group
    SessionJoined {
        group_id: String,
        session_id: SessionId,
        role: String,
    },
    
    /// Session left the group
    SessionLeft {
        group_id: String,
        session_id: SessionId,
        reason: String,
    },
    
    /// Group state changed
    StateChanged {
        group_id: String,
        old_state: GroupState,
        new_state: GroupState,
    },
    
    /// Group was terminated
    GroupTerminated {
        group_id: String,
        reason: String,
    },
}

/// Represents a group of related sessions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionGroup {
    /// Unique group identifier
    pub id: String,
    
    /// Type of group
    pub group_type: GroupType,
    
    /// Current state of the group
    pub state: GroupState,
    
    /// Group configuration
    pub config: GroupConfig,
    
    /// Sessions in the group
    pub members: HashMap<SessionId, SessionMembership>,
    
    /// When the group was created
    pub created_at: SystemTime,
    
    /// When the group was last updated
    pub updated_at: SystemTime,
    
    /// Group-wide metadata
    pub metadata: HashMap<String, String>,
    
    /// Group leader session (if applicable)
    pub leader: Option<SessionId>,
    
    /// Group statistics
    pub stats: GroupStatistics,
    
    /// **INTEGRATION**: Associated media bridge ID (for Bridge-type groups)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bridge_id: Option<BridgeId>,
}

/// Statistics for session groups
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GroupStatistics {
    /// Total sessions that have joined
    pub total_sessions_joined: u64,
    
    /// Total sessions that have left
    pub total_sessions_left: u64,
    
    /// Current active session count
    pub current_active_sessions: usize,
    
    /// Peak session count
    pub peak_session_count: usize,
    
    /// Average session duration in group
    pub average_session_duration: Duration,
    
    /// Total group lifetime
    pub total_lifetime: Duration,
    
    /// Number of state changes
    pub state_changes: u64,
}

impl SessionGroup {
    /// Create a new session group
    pub fn new(group_type: GroupType, config: GroupConfig) -> Self {
        let now = SystemTime::now();
        Self {
            id: Uuid::new_v4().to_string(),
            group_type,
            state: GroupState::Initializing,
            config,
            members: HashMap::new(),
            created_at: now,
            updated_at: now,
            metadata: HashMap::new(),
            leader: None,
            stats: GroupStatistics::default(),
            bridge_id: None,
        }
    }
    
    /// Add a session to the group
    pub fn add_session(&mut self, session_id: SessionId, role: String) -> Result<(), Error> {
        // Check if we're at max capacity
        if let Some(max) = self.config.max_sessions {
            if self.members.len() >= max {
                return Err(Error::InternalError(
                    format!("Group {} has reached maximum capacity of {}", self.id, max),
                    ErrorContext::default().with_message("Group capacity exceeded")
                ));
            }
        }
        
        // Check if session is already in the group
        if self.members.contains_key(&session_id) {
            return Err(Error::InternalError(
                format!("Session {} is already in group {}", session_id, self.id),
                ErrorContext::default().with_message("Duplicate session membership")
            ));
        }
        
        // Add the session
        let membership = SessionMembership::new(session_id, role.clone());
        self.members.insert(session_id, membership);
        
        // Update statistics
        self.stats.total_sessions_joined += 1;
        self.stats.current_active_sessions = self.get_active_session_count();
        self.stats.peak_session_count = self.stats.peak_session_count.max(self.stats.current_active_sessions);
        
        // Set leader if this is the first session
        if self.leader.is_none() && role == "leader" {
            self.leader = Some(session_id);
        }
        
        self.updated_at = SystemTime::now();
        
        info!("✅ Added session {} to group {} with role '{}'", session_id, self.id, role);
        
        Ok(())
    }
    
    /// Remove a session from the group
    pub fn remove_session(&mut self, session_id: SessionId) -> Result<(), Error> {
        if let Some(_membership) = self.members.remove(&session_id) {
            // Update statistics
            self.stats.total_sessions_left += 1;
            self.stats.current_active_sessions = self.get_active_session_count();
            
            // Handle leader removal
            if self.leader == Some(session_id) {
                self.elect_new_leader();
            }
            
            self.updated_at = SystemTime::now();
            
            info!("🗑️ Removed session {} from group {}", session_id, self.id);
            
            // Check if group should be terminated
            if self.config.auto_terminate_on_min && self.members.len() < self.config.min_sessions {
                self.state = GroupState::Terminating;
                info!("🔚 Group {} marked for termination (below minimum sessions)", self.id);
            }
            
            Ok(())
        } else {
            Err(Error::InternalError(
                format!("Session {} not found in group {}", session_id, self.id),
                ErrorContext::default().with_message("Session not in group")
            ))
        }
    }
    
    /// Update group state
    pub fn update_state(&mut self, new_state: GroupState) {
        if self.state != new_state {
            let old_state = self.state;
            self.state = new_state;
            self.updated_at = SystemTime::now();
            self.stats.state_changes += 1;
            
            info!("🔄 Group {} state changed: {} → {}", self.id, old_state, new_state);
        }
    }
    
    /// Get active session count
    pub fn get_active_session_count(&self) -> usize {
        self.members.values()
            .filter(|m| m.active)
            .count()
    }
    
    /// Get all session IDs in the group
    pub fn get_session_ids(&self) -> Vec<SessionId> {
        self.members.keys().copied().collect()
    }
    
    /// Get active session IDs in the group
    pub fn get_active_session_ids(&self) -> Vec<SessionId> {
        self.members.iter()
            .filter(|(_, m)| m.active)
            .map(|(id, _)| *id)
            .collect()
    }
    
    /// Check if the group contains a session
    pub fn contains_session(&self, session_id: SessionId) -> bool {
        self.members.contains_key(&session_id)
    }
    
    /// Get session role in the group
    pub fn get_session_role(&self, session_id: SessionId) -> Option<String> {
        self.members.get(&session_id).map(|m| m.role.clone())
    }
    
    /// Elect a new leader from current members
    fn elect_new_leader(&mut self) {
        // Simple election: pick the oldest active member
        self.leader = self.members.iter()
            .filter(|(_, m)| m.active)
            .min_by_key(|(_, m)| m.joined_at)
            .map(|(id, _)| *id);
            
        if let Some(new_leader) = self.leader {
            info!("👑 Elected new leader {} for group {}", new_leader, self.id);
        }
    }
    
    /// Check if the group is active
    pub fn is_active(&self) -> bool {
        self.state == GroupState::Active
    }
    
    /// Check if the group is terminal
    pub fn is_terminal(&self) -> bool {
        matches!(self.state, GroupState::Terminated | GroupState::Failed)
    }
    
    /// Add metadata to the group
    pub fn add_metadata(&mut self, key: String, value: String) {
        self.metadata.insert(key, value);
        self.updated_at = SystemTime::now();
    }
    
    /// **INTEGRATION**: Associate a media bridge with this group (for Bridge-type groups)
    pub fn set_bridge_id(&mut self, bridge_id: BridgeId) {
        let bridge_id_for_log = bridge_id.clone(); // Clone for logging
        self.bridge_id = Some(bridge_id);
        self.updated_at = SystemTime::now();
        info!("🔗 Associated media bridge {} with session group {}", bridge_id_for_log, self.id);
    }
    
    /// **INTEGRATION**: Get the associated media bridge ID
    pub fn get_bridge_id(&self) -> Option<BridgeId> {
        self.bridge_id.clone()
    }
    
    /// **INTEGRATION**: Check if this group has an associated media bridge
    pub fn has_media_bridge(&self) -> bool {
        self.bridge_id.is_some()
    }
    
    /// **INTEGRATION**: Create bridge configuration from group configuration
    pub fn create_bridge_config(&self) -> BridgeConfig {
        let mut bridge_config = BridgeConfig::default();
        
        // Map group config to bridge config
        if let Some(max_sessions) = self.config.max_sessions {
            bridge_config.max_sessions = max_sessions;
        }
        
        // Use group name if available
        if let Some(name) = self.metadata.get("name") {
            bridge_config.name = Some(name.clone());
        }
        
        // Enable mixing for conference-type groups
        bridge_config.enable_mixing = matches!(self.group_type, GroupType::Conference | GroupType::Bridge);
        
        bridge_config
    }
}

/// Manages session groups and their lifecycle
pub struct SessionGroupManager {
    /// Active groups by ID
    groups: Arc<DashMap<String, SessionGroup>>,
    
    /// Session to group mappings
    session_to_groups: Arc<DashMap<SessionId, HashSet<String>>>,
    
    /// Group metrics
    metrics: Arc<RwLock<GroupManagerMetrics>>,
    
    /// Configuration for group management
    config: GroupManagerConfig,
}

/// Configuration for the group manager
#[derive(Debug, Clone)]
pub struct GroupManagerConfig {
    /// Maximum number of concurrent groups
    pub max_concurrent_groups: Option<usize>,
    
    /// Whether to track detailed metrics
    pub track_metrics: bool,
    
    /// Whether to auto-cleanup terminated groups
    pub auto_cleanup: bool,
    
    /// Cleanup interval for terminated groups
    pub cleanup_interval: Duration,
}

impl Default for GroupManagerConfig {
    fn default() -> Self {
        Self {
            max_concurrent_groups: Some(1000),
            track_metrics: true,
            auto_cleanup: true,
            cleanup_interval: Duration::from_secs(300), // 5 minutes
        }
    }
}

/// Metrics for group management
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GroupManagerMetrics {
    /// Total groups created
    pub total_groups_created: u64,
    
    /// Total groups terminated
    pub total_groups_terminated: u64,
    
    /// Current active groups
    pub active_groups: u64,
    
    /// Groups by type
    pub groups_by_type: HashMap<GroupType, u64>,
    
    /// Average group lifetime
    pub average_group_lifetime: Duration,
    
    /// Peak concurrent groups
    pub peak_concurrent_groups: u64,
    
    /// Session membership operations
    pub total_membership_operations: u64,
}

impl SessionGroupManager {
    /// Create a new session group manager
    pub fn new(config: GroupManagerConfig) -> Self {
        Self {
            groups: Arc::new(DashMap::new()),
            session_to_groups: Arc::new(DashMap::new()),
            metrics: Arc::new(RwLock::new(GroupManagerMetrics::default())),
            config,
        }
    }
    
    /// Create a new session group
    pub async fn create_group(
        &self,
        group_type: GroupType,
        config: GroupConfig,
    ) -> Result<String, Error> {
        // Check concurrent group limit
        if let Some(max) = self.config.max_concurrent_groups {
            if self.groups.len() >= max {
                return Err(Error::InternalError(
                    format!("Maximum concurrent groups limit {} reached", max),
                    ErrorContext::default().with_message("Group creation limit exceeded")
                ));
            }
        }
        
        let mut group = SessionGroup::new(group_type, config);
        group.update_state(GroupState::Active);
        let group_id = group.id.clone();
        
        // Store the group
        self.groups.insert(group_id.clone(), group);
        
        // Update metrics
        if self.config.track_metrics {
            let mut metrics = self.metrics.write().await;
            metrics.total_groups_created += 1;
            metrics.active_groups += 1;
            *metrics.groups_by_type.entry(group_type).or_insert(0) += 1;
            metrics.peak_concurrent_groups = metrics.peak_concurrent_groups.max(metrics.active_groups);
        }
        
        info!("✅ Created session group {} of type {}", group_id, group_type);
        
        Ok(group_id)
    }
    
    /// Add a session to a group
    pub async fn add_session_to_group(
        &self,
        group_id: &str,
        session_id: SessionId,
        role: String,
    ) -> Result<(), Error> {
        // Get and update the group
        if let Some(mut group) = self.groups.get_mut(group_id) {
            group.add_session(session_id, role)?;
            
            // Update session-to-group mapping
            self.session_to_groups
                .entry(session_id)
                .or_insert_with(HashSet::new)
                .insert(group_id.to_string());
            
            // Update metrics
            if self.config.track_metrics {
                let mut metrics = self.metrics.write().await;
                metrics.total_membership_operations += 1;
            }
            
            Ok(())
        } else {
            Err(Error::InternalError(
                format!("Group {} not found", group_id),
                ErrorContext::default().with_message("Group not found")
            ))
        }
    }
    
    /// Remove a session from a group
    pub async fn remove_session_from_group(
        &self,
        group_id: &str,
        session_id: SessionId,
    ) -> Result<(), Error> {
        // Get and update the group
        if let Some(mut group) = self.groups.get_mut(group_id) {
            group.remove_session(session_id)?;
            
            // Update session-to-group mapping
            if let Some(mut session_groups) = self.session_to_groups.get_mut(&session_id) {
                session_groups.remove(group_id);
                if session_groups.is_empty() {
                    drop(session_groups);
                    self.session_to_groups.remove(&session_id);
                }
            }
            
            // Update metrics
            if self.config.track_metrics {
                let mut metrics = self.metrics.write().await;
                metrics.total_membership_operations += 1;
            }
            
            Ok(())
        } else {
            Err(Error::InternalError(
                format!("Group {} not found", group_id),
                ErrorContext::default().with_message("Group not found")
            ))
        }
    }
    
    /// Terminate a group
    pub async fn terminate_group(&self, group_id: &str, reason: &str) -> Result<(), Error> {
        if let Some(mut group) = self.groups.get_mut(group_id) {
            group.update_state(GroupState::Terminated);
            
            // Remove all session mappings
            for session_id in group.get_session_ids() {
                if let Some(mut session_groups) = self.session_to_groups.get_mut(&session_id) {
                    session_groups.remove(group_id);
                    if session_groups.is_empty() {
                        drop(session_groups);
                        self.session_to_groups.remove(&session_id);
                    }
                }
            }
            
            info!("🔚 Terminated group {} - {}", group_id, reason);
            
            // Update metrics
            if self.config.track_metrics {
                let mut metrics = self.metrics.write().await;
                metrics.total_groups_terminated += 1;
                if metrics.active_groups > 0 {
                    metrics.active_groups -= 1;
                }
            }
            
            Ok(())
        } else {
            Err(Error::InternalError(
                format!("Group {} not found", group_id),
                ErrorContext::default().with_message("Group not found")
            ))
        }
    }
    
    /// Get a group by ID
    pub async fn get_group(&self, group_id: &str) -> Option<SessionGroup> {
        self.groups.get(group_id).map(|g| g.value().clone())
    }
    
    /// Get all groups containing a session
    pub async fn get_session_groups(&self, session_id: SessionId) -> Vec<SessionGroup> {
        let mut groups = Vec::new();
        
        if let Some(group_ids) = self.session_to_groups.get(&session_id) {
            for group_id in group_ids.iter() {
                if let Some(group) = self.groups.get(group_id) {
                    groups.push(group.value().clone());
                }
            }
        }
        
        groups
    }
    
    /// Get all active groups
    pub async fn get_active_groups(&self) -> Vec<SessionGroup> {
        self.groups.iter()
            .filter(|entry| entry.value().is_active())
            .map(|entry| entry.value().clone())
            .collect()
    }
    
    /// Handle session termination across all groups
    pub async fn handle_session_termination(&self, session_id: SessionId) -> Result<(), Error> {
        info!("🔄 Handling session termination for {} across all groups", session_id);
        
        let group_ids: Vec<String> = if let Some(session_groups) = self.session_to_groups.get(&session_id) {
            session_groups.iter().cloned().collect()
        } else {
            Vec::new()
        };
        
        for group_id in group_ids {
            if let Err(e) = self.remove_session_from_group(&group_id, session_id).await {
                warn!("Failed to remove session {} from group {}: {}", session_id, group_id, e);
            }
        }
        
        info!("✅ Completed group cleanup for session {}", session_id);
        Ok(())
    }
    
    /// Cleanup terminated groups
    pub async fn cleanup_terminated_groups(&self) -> Result<usize, Error> {
        let mut cleanup_count = 0;
        let mut to_remove = Vec::new();
        
        for entry in self.groups.iter() {
            if entry.value().is_terminal() {
                to_remove.push(entry.key().clone());
            }
        }
        
        for group_id in to_remove {
            self.groups.remove(&group_id);
            cleanup_count += 1;
        }
        
        if cleanup_count > 0 {
            info!("🧹 Cleaned up {} terminated groups", cleanup_count);
        }
        
        Ok(cleanup_count)
    }
    
    /// Get group manager metrics
    pub async fn get_metrics(&self) -> GroupManagerMetrics {
        self.metrics.read().await.clone()
    }
    
    /// Get current active group count
    pub async fn get_active_group_count(&self) -> usize {
        self.groups.iter()
            .filter(|entry| entry.value().is_active())
            .count()
    }
    
    /// **INTEGRATION**: Create a bridge-type group with automatic media bridge creation
    pub async fn create_bridge_group(
        &self,
        config: GroupConfig,
        bridge_factory: impl Fn(BridgeConfig) -> Arc<SessionBridge>,
    ) -> Result<(String, BridgeId), Error> {
        // Create the session group
        let group_id = self.create_group(GroupType::Bridge, config.clone()).await?;
        
        // Create associated media bridge
        if let Some(mut group) = self.groups.get_mut(&group_id) {
            let bridge_config = group.create_bridge_config();
            let session_bridge = bridge_factory(bridge_config);
            let bridge_id = session_bridge.id.clone();
            
            // Associate the bridge with the group
            group.set_bridge_id(bridge_id.clone());
            
            info!("✅ Created bridge group {} with media bridge {}", group_id, bridge_id);
            
            Ok((group_id, bridge_id))
        } else {
            Err(Error::InternalError(
                format!("Failed to retrieve created group {}", group_id),
                ErrorContext::default().with_message("Group creation inconsistency")
            ))
        }
    }
    
    /// **INTEGRATION**: Add session to group and associated media bridge
    pub async fn add_session_with_bridge(
        &self,
        group_id: &str,
        session_id: SessionId,
        role: String,
        bridge_manager: impl Fn(BridgeId, SessionId) -> Result<(), crate::session::bridge::BridgeError>,
    ) -> Result<(), Error> {
        // Add to session group first
        self.add_session_to_group(group_id, session_id, role).await?;
        
        // Add to associated media bridge if exists
        if let Some(group) = self.groups.get(group_id) {
            if let Some(bridge_id) = group.get_bridge_id() {
                if let Err(bridge_error) = bridge_manager(bridge_id, session_id) {
                    // If bridge addition fails, remove from group to maintain consistency
                    warn!("Failed to add session {} to media bridge: {}", session_id, bridge_error);
                    let _ = self.remove_session_from_group(group_id, session_id).await;
                    
                    return Err(Error::InternalError(
                        format!("Failed to add session to media bridge: {}", bridge_error),
                        ErrorContext::default().with_message("Bridge coordination failed")
                    ));
                }
                
                info!("✅ Added session {} to group {} and media bridge", session_id, group_id);
            }
        }
        
        Ok(())
    }
    
    /// **INTEGRATION**: Remove session from group and associated media bridge
    pub async fn remove_session_with_bridge(
        &self,
        group_id: &str,
        session_id: SessionId,
        bridge_manager: impl Fn(BridgeId, SessionId) -> Result<(), crate::session::bridge::BridgeError>,
    ) -> Result<(), Error> {
        // Remove from associated media bridge first
        if let Some(group) = self.groups.get(group_id) {
            if let Some(bridge_id) = group.get_bridge_id() {
                if let Err(bridge_error) = bridge_manager(bridge_id, session_id) {
                    warn!("Failed to remove session {} from media bridge: {}", session_id, bridge_error);
                    // Continue with group removal even if bridge removal fails
                }
            }
        }
        
        // Remove from session group
        self.remove_session_from_group(group_id, session_id).await?;
        
        info!("✅ Removed session {} from group {} and media bridge", session_id, group_id);
        Ok(())
    }
    
    /// **INTEGRATION**: Get groups that have associated media bridges
    pub async fn get_bridge_groups(&self) -> Vec<(SessionGroup, BridgeId)> {
        let mut bridge_groups = Vec::new();
        
        for entry in self.groups.iter() {
            let group = entry.value();
            if let Some(bridge_id) = group.get_bridge_id() {
                bridge_groups.push((group.clone(), bridge_id));
            }
        }
        
        bridge_groups
    }
} 