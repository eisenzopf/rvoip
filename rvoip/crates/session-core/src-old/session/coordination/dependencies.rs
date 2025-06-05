//! Session Dependency Tracking
//! 
//! This module provides sophisticated dependency tracking for complex call scenarios:
//! 
//! - Parent-child session relationships (transfers, consultations)
//! - Dependency state management and lifecycle coordination
//! - Automatic dependency cleanup and orphan handling
//! - Dependency validation and constraint enforcement
//! - Cross-session resource coordination

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use dashmap::DashMap;
use tokio::sync::RwLock;
use tracing::{debug, info, warn, error};
use serde::{Serialize, Deserialize};

use crate::session::{SessionId, SessionState};
use crate::errors::{Error, ErrorContext};

/// Types of dependencies between sessions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DependencyType {
    /// Parent-child relationship (child depends on parent)
    ParentChild,
    
    /// Consultation relationship (consultation session for transfer)
    Consultation,
    
    /// Conference relationship (session is part of conference)
    Conference,
    
    /// Transfer relationship (source and target sessions)
    Transfer,
    
    /// Bridge relationship (sessions connected via bridge)
    Bridge,
    
    /// Sequential relationship (sessions must be processed in order)
    Sequential,
    
    /// Mutual dependency (sessions depend on each other)
    Mutual,
    
    /// Resource sharing (sessions share common resources)
    ResourceSharing,
}

impl std::fmt::Display for DependencyType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DependencyType::ParentChild => write!(f, "ParentChild"),
            DependencyType::Consultation => write!(f, "Consultation"),
            DependencyType::Conference => write!(f, "Conference"),
            DependencyType::Transfer => write!(f, "Transfer"),
            DependencyType::Bridge => write!(f, "Bridge"),
            DependencyType::Sequential => write!(f, "Sequential"),
            DependencyType::Mutual => write!(f, "Mutual"),
            DependencyType::ResourceSharing => write!(f, "ResourceSharing"),
        }
    }
}

/// State of a dependency relationship
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DependencyState {
    /// Dependency is being established
    Establishing,
    
    /// Dependency is active and enforced
    Active,
    
    /// Dependency is temporarily suspended
    Suspended,
    
    /// Dependency is being terminated
    Terminating,
    
    /// Dependency has been terminated
    Terminated,
    
    /// Dependency failed to establish or maintain
    Failed,
}

impl std::fmt::Display for DependencyState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DependencyState::Establishing => write!(f, "Establishing"),
            DependencyState::Active => write!(f, "Active"),
            DependencyState::Suspended => write!(f, "Suspended"),
            DependencyState::Terminating => write!(f, "Terminating"),
            DependencyState::Terminated => write!(f, "Terminated"),
            DependencyState::Failed => write!(f, "Failed"),
        }
    }
}

/// Configuration for dependency behavior
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyConfig {
    /// Whether to automatically cleanup dependencies when sessions terminate
    pub auto_cleanup: bool,
    
    /// Whether to cascade termination from parent to children
    pub cascade_termination: bool,
    
    /// Whether to prevent parent termination if children exist
    pub prevent_parent_termination: bool,
    
    /// Maximum dependency depth to prevent cycles
    pub max_dependency_depth: usize,
    
    /// Timeout for dependency establishment
    pub establishment_timeout: Duration,
    
    /// Timeout for dependency termination
    pub termination_timeout: Duration,
    
    /// Whether to validate dependency constraints
    pub validate_constraints: bool,
    
    /// Whether to track dependency metrics
    pub track_metrics: bool,
}

impl Default for DependencyConfig {
    fn default() -> Self {
        Self {
            auto_cleanup: true,
            cascade_termination: true,
            prevent_parent_termination: false,
            max_dependency_depth: 10,
            establishment_timeout: Duration::from_secs(30),
            termination_timeout: Duration::from_secs(10),
            validate_constraints: true,
            track_metrics: true,
        }
    }
}

/// Represents a dependency relationship between sessions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDependency {
    /// Source session (the one that depends)
    pub source_session: SessionId,
    
    /// Target session (the one being depended upon)
    pub target_session: SessionId,
    
    /// Type of dependency
    pub dependency_type: DependencyType,
    
    /// Current state of the dependency
    pub state: DependencyState,
    
    /// When the dependency was created
    pub created_at: SystemTime,
    
    /// When the dependency was last updated
    pub updated_at: SystemTime,
    
    /// Optional metadata about the dependency
    pub metadata: HashMap<String, String>,
    
    /// Whether this dependency is bidirectional
    pub bidirectional: bool,
    
    /// Priority of this dependency (higher values take precedence)
    pub priority: u32,
}

impl SessionDependency {
    /// Create a new session dependency
    pub fn new(
        source: SessionId,
        target: SessionId,
        dependency_type: DependencyType,
    ) -> Self {
        let now = SystemTime::now();
        Self {
            source_session: source,
            target_session: target,
            dependency_type,
            state: DependencyState::Establishing,
            created_at: now,
            updated_at: now,
            metadata: HashMap::new(),
            bidirectional: false,
            priority: 0,
        }
    }
    
    /// Update the dependency state
    pub fn update_state(&mut self, new_state: DependencyState) {
        self.state = new_state;
        self.updated_at = SystemTime::now();
    }
    
    /// Add metadata to the dependency
    pub fn add_metadata(&mut self, key: String, value: String) {
        self.metadata.insert(key, value);
        self.updated_at = SystemTime::now();
    }
    
    /// Check if the dependency involves a specific session
    pub fn involves_session(&self, session_id: SessionId) -> bool {
        self.source_session == session_id || self.target_session == session_id
    }
    
    /// Get the other session in the dependency relationship
    pub fn get_other_session(&self, session_id: SessionId) -> Option<SessionId> {
        if self.source_session == session_id {
            Some(self.target_session)
        } else if self.target_session == session_id {
            Some(self.source_session)
        } else {
            None
        }
    }
    
    /// Check if the dependency is active
    pub fn is_active(&self) -> bool {
        self.state == DependencyState::Active
    }
    
    /// Check if the dependency is terminal
    pub fn is_terminal(&self) -> bool {
        matches!(self.state, DependencyState::Terminated | DependencyState::Failed)
    }
}

/// Metrics for dependency tracking
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DependencyMetrics {
    /// Total number of dependencies created
    pub total_dependencies_created: u64,
    
    /// Total number of dependencies terminated
    pub total_dependencies_terminated: u64,
    
    /// Current number of active dependencies
    pub active_dependencies: u64,
    
    /// Dependencies by type
    pub dependencies_by_type: HashMap<DependencyType, u64>,
    
    /// Average dependency lifetime
    pub average_dependency_lifetime: Duration,
    
    /// Number of dependency validation failures
    pub validation_failures: u64,
    
    /// Number of cascaded terminations
    pub cascaded_terminations: u64,
    
    /// Number of orphaned sessions handled
    pub orphaned_sessions_handled: u64,
}

/// Manages session dependencies and relationships
pub struct SessionDependencyTracker {
    /// Configuration for dependency tracking
    config: DependencyConfig,
    
    /// Active dependencies by ID
    dependencies: Arc<DashMap<String, SessionDependency>>,
    
    /// Dependencies by source session
    dependencies_by_source: Arc<DashMap<SessionId, HashSet<String>>>,
    
    /// Dependencies by target session
    dependencies_by_target: Arc<DashMap<SessionId, HashSet<String>>>,
    
    /// Dependency metrics
    metrics: Arc<RwLock<DependencyMetrics>>,
    
    /// Session state cache for dependency validation
    session_states: Arc<DashMap<SessionId, SessionState>>,
}

impl SessionDependencyTracker {
    /// Create a new dependency tracker
    pub fn new(config: DependencyConfig) -> Self {
        Self {
            config,
            dependencies: Arc::new(DashMap::new()),
            dependencies_by_source: Arc::new(DashMap::new()),
            dependencies_by_target: Arc::new(DashMap::new()),
            metrics: Arc::new(RwLock::new(DependencyMetrics::default())),
            session_states: Arc::new(DashMap::new()),
        }
    }
    
    /// Create a new dependency between sessions
    pub async fn create_dependency(
        &self,
        source: SessionId,
        target: SessionId,
        dependency_type: DependencyType,
    ) -> Result<String, Error> {
        // Validate the dependency before creating
        self.validate_dependency(source, target, dependency_type).await?;
        
        let dependency_id = format!("{}:{}:{}", source, target, dependency_type);
        let mut dependency = SessionDependency::new(source, target, dependency_type);
        
        // Check for bidirectional dependencies
        if matches!(dependency_type, DependencyType::Mutual | DependencyType::Conference) {
            dependency.bidirectional = true;
        }
        
        // Store the dependency
        self.dependencies.insert(dependency_id.clone(), dependency.clone());
        
        // Update indexes
        self.dependencies_by_source
            .entry(source)
            .or_insert_with(HashSet::new)
            .insert(dependency_id.clone());
            
        self.dependencies_by_target
            .entry(target)
            .or_insert_with(HashSet::new)
            .insert(dependency_id.clone());
        
        // Mark as active
        if let Some(mut dep) = self.dependencies.get_mut(&dependency_id) {
            dep.update_state(DependencyState::Active);
        }
        
        // Update metrics
        if self.config.track_metrics {
            let mut metrics = self.metrics.write().await;
            metrics.total_dependencies_created += 1;
            metrics.active_dependencies += 1;
            *metrics.dependencies_by_type.entry(dependency_type).or_insert(0) += 1;
        }
        
        info!("✅ Created dependency {} between sessions {} and {}", 
            dependency_type, source, target);
        
        Ok(dependency_id)
    }
    
    /// Remove a dependency between sessions
    pub async fn remove_dependency(&self, dependency_id: &str) -> Result<(), Error> {
        if let Some((_, dependency)) = self.dependencies.remove(dependency_id) {
            // Update indexes
            if let Some(mut source_deps) = self.dependencies_by_source.get_mut(&dependency.source_session) {
                source_deps.remove(dependency_id);
            }
            
            if let Some(mut target_deps) = self.dependencies_by_target.get_mut(&dependency.target_session) {
                target_deps.remove(dependency_id);
            }
            
            // Update metrics
            if self.config.track_metrics {
                let mut metrics = self.metrics.write().await;
                metrics.total_dependencies_terminated += 1;
                if metrics.active_dependencies > 0 {
                    metrics.active_dependencies -= 1;
                }
            }
            
            info!("🗑️ Removed dependency {} between sessions {} and {}", 
                dependency.dependency_type, dependency.source_session, dependency.target_session);
            
            Ok(())
        } else {
            Err(Error::InternalError(
                format!("Dependency {} not found", dependency_id),
                ErrorContext::default().with_message("Dependency removal failed")
            ))
        }
    }
    
    /// Get all dependencies for a session
    pub async fn get_session_dependencies(&self, session_id: SessionId) -> Vec<SessionDependency> {
        let mut dependencies = Vec::new();
        
        // Get dependencies where this session is the source
        if let Some(source_deps) = self.dependencies_by_source.get(&session_id) {
            for dep_id in source_deps.iter() {
                if let Some(dep) = self.dependencies.get(dep_id) {
                    dependencies.push(dep.value().clone());
                }
            }
        }
        
        // Get dependencies where this session is the target
        if let Some(target_deps) = self.dependencies_by_target.get(&session_id) {
            for dep_id in target_deps.iter() {
                if let Some(dep) = self.dependencies.get(dep_id) {
                    dependencies.push(dep.value().clone());
                }
            }
        }
        
        dependencies
    }
    
    /// Get children of a session (sessions that depend on this one)
    pub async fn get_session_children(&self, parent_session: SessionId) -> Vec<SessionId> {
        let mut children = Vec::new();
        
        if let Some(target_deps) = self.dependencies_by_target.get(&parent_session) {
            for dep_id in target_deps.iter() {
                if let Some(dep) = self.dependencies.get(dep_id) {
                    if matches!(dep.dependency_type, DependencyType::ParentChild) && dep.is_active() {
                        children.push(dep.source_session);
                    }
                }
            }
        }
        
        children
    }
    
    /// Get parent of a session
    pub async fn get_session_parent(&self, child_session: SessionId) -> Option<SessionId> {
        if let Some(source_deps) = self.dependencies_by_source.get(&child_session) {
            for dep_id in source_deps.iter() {
                if let Some(dep) = self.dependencies.get(dep_id) {
                    if matches!(dep.dependency_type, DependencyType::ParentChild) && dep.is_active() {
                        return Some(dep.target_session);
                    }
                }
            }
        }
        
        None
    }
    
    /// Handle session termination with dependency cleanup
    pub async fn handle_session_termination(&self, session_id: SessionId) -> Result<(), Error> {
        info!("🔄 Handling session termination for {} with dependency cleanup", session_id);
        
        let dependencies = self.get_session_dependencies(session_id).await;
        
        for dependency in dependencies {
            if dependency.involves_session(session_id) {
                // Handle different dependency types
                match dependency.dependency_type {
                    DependencyType::ParentChild => {
                        if dependency.target_session == session_id {
                            // Parent session terminating
                            if self.config.cascade_termination {
                                let children = self.get_session_children(session_id).await;
                                for child in children {
                                    info!("🔗 Cascading termination to child session {}", child);
                                    // Note: Actual session termination would be handled by SessionManager
                                }
                            }
                        }
                    },
                    DependencyType::Consultation => {
                        // Cleanup consultation relationships
                        if let Some(other_session) = dependency.get_other_session(session_id) {
                            info!("🔄 Cleaning up consultation relationship with session {}", other_session);
                        }
                    },
                    _ => {
                        // Generic cleanup for other dependency types
                        debug!("🧹 Cleaning up {} dependency", dependency.dependency_type);
                    }
                }
                
                // Remove the dependency
                let dep_id = format!("{}:{}:{}", dependency.source_session, dependency.target_session, dependency.dependency_type);
                self.remove_dependency(&dep_id).await?;
            }
        }
        
        // Remove session from state cache
        self.session_states.remove(&session_id);
        
        info!("✅ Completed dependency cleanup for session {}", session_id);
        Ok(())
    }
    
    /// Update session state in the tracker
    pub async fn update_session_state(&self, session_id: SessionId, state: SessionState) {
        self.session_states.insert(session_id, state);
        
        // Check for state-dependent dependency actions
        if state == SessionState::Terminated {
            if let Err(e) = self.handle_session_termination(session_id).await {
                error!("Failed to handle session termination for {}: {}", session_id, e);
            }
        }
    }
    
    /// Validate a potential dependency
    async fn validate_dependency(
        &self,
        source: SessionId,
        target: SessionId,
        dependency_type: DependencyType,
    ) -> Result<(), Error> {
        if !self.config.validate_constraints {
            return Ok(());
        }
        
        // Check for self-dependency
        if source == target {
            return Err(Error::InternalError(
                "Cannot create dependency from session to itself".to_string(),
                ErrorContext::default().with_message("Self-dependency not allowed")
            ));
        }
        
        // Check for existing dependency
        let dependency_id = format!("{}:{}:{}", source, target, dependency_type);
        if self.dependencies.contains_key(&dependency_id) {
            return Err(Error::InternalError(
                format!("Dependency {} already exists", dependency_id),
                ErrorContext::default().with_message("Duplicate dependency")
            ));
        }
        
        // Check dependency depth to prevent cycles
        if self.would_create_cycle(source, target).await {
            return Err(Error::InternalError(
                "Creating this dependency would create a cycle".to_string(),
                ErrorContext::default().with_message("Dependency cycle detected")
            ));
        }
        
        // Check maximum depth
        let depth = self.get_dependency_depth(source).await;
        if depth >= self.config.max_dependency_depth {
            return Err(Error::InternalError(
                format!("Maximum dependency depth {} exceeded", self.config.max_dependency_depth),
                ErrorContext::default().with_message("Dependency depth limit exceeded")
            ));
        }
        
        Ok(())
    }
    
    /// Check if creating a dependency would create a cycle
    async fn would_create_cycle(&self, source: SessionId, target: SessionId) -> bool {
        // Simple cycle detection: check if target has a path back to source
        self.has_dependency_path(target, source).await
    }
    
    /// Check if there's a dependency path from one session to another
    async fn has_dependency_path(&self, from: SessionId, to: SessionId) -> bool {
        let mut visited = HashSet::new();
        let mut stack = vec![from];
        
        while let Some(current) = stack.pop() {
            if current == to {
                return true;
            }
            
            if visited.contains(&current) {
                continue;
            }
            visited.insert(current);
            
            // Add all sessions this one depends on
            if let Some(source_deps) = self.dependencies_by_source.get(&current) {
                for dep_id in source_deps.iter() {
                    if let Some(dep) = self.dependencies.get(dep_id) {
                        if dep.is_active() {
                            stack.push(dep.target_session);
                        }
                    }
                }
            }
        }
        
        false
    }
    
    /// Get the dependency depth for a session
    async fn get_dependency_depth(&self, session_id: SessionId) -> usize {
        let mut max_depth = 0;
        let mut visited = HashSet::new();
        self.calculate_depth_recursive(session_id, 0, &mut max_depth, &mut visited).await;
        max_depth
    }
    
    /// Recursively calculate dependency depth
    async fn calculate_depth_recursive(
        &self,
        session_id: SessionId,
        current_depth: usize,
        max_depth: &mut usize,
        visited: &mut HashSet<SessionId>,
    ) {
        if visited.contains(&session_id) {
            return;
        }
        visited.insert(session_id);
        
        *max_depth = (*max_depth).max(current_depth);
        
        // Check dependencies
        if let Some(source_deps) = self.dependencies_by_source.get(&session_id) {
            for dep_id in source_deps.iter() {
                if let Some(dep) = self.dependencies.get(dep_id) {
                    if dep.is_active() {
                        // Use Box::pin to handle async recursion
                        Box::pin(self.calculate_depth_recursive(
                            dep.target_session,
                            current_depth + 1,
                            max_depth,
                            visited,
                        )).await;
                    }
                }
            }
        }
    }
    
    /// Get dependency metrics
    pub async fn get_metrics(&self) -> DependencyMetrics {
        self.metrics.read().await.clone()
    }
    
    /// Get number of active dependencies
    pub async fn get_active_dependency_count(&self) -> usize {
        self.dependencies.iter()
            .filter(|dep| dep.is_active())
            .count()
    }
    
    /// Cleanup terminated dependencies
    pub async fn cleanup_terminated_dependencies(&self) -> Result<usize, Error> {
        let mut cleanup_count = 0;
        let mut to_remove = Vec::new();
        
        for entry in self.dependencies.iter() {
            if entry.value().is_terminal() {
                to_remove.push(entry.key().clone());
            }
        }
        
        for dep_id in to_remove {
            self.remove_dependency(&dep_id).await?;
            cleanup_count += 1;
        }
        
        if cleanup_count > 0 {
            info!("🧹 Cleaned up {} terminated dependencies", cleanup_count);
        }
        
        Ok(cleanup_count)
    }
} 