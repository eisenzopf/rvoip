//! Session Resource Management
//! 
//! This module provides enhanced session resource tracking, cleanup automation,
//! and monitoring capabilities for session-core. It implements:
//! 
//! - Granular resource tracking by user/endpoint
//! - Session aging and timeout management  
//! - Automatic cleanup of terminated sessions
//! - Resource metrics and monitoring
//! - Per-user session limits
//! - Session health monitoring
//! - Resource leak detection

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, Instant};
use dashmap::DashMap;
use tokio::sync::RwLock;
use tokio::time::interval;
use tracing::{debug, info, warn, error};
use serde::{Serialize, Deserialize};

use crate::session::{SessionId, SessionState};
use crate::errors::{Error, ErrorCategory, ErrorContext, ErrorSeverity, RecoveryAction};

/// Configuration for session resource management
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionResourceConfig {
    /// Maximum sessions per user (None = unlimited)
    pub max_sessions_per_user: Option<usize>,
    
    /// Maximum total concurrent sessions (None = unlimited)
    pub max_total_sessions: Option<usize>,
    
    /// Session timeout for inactive sessions
    pub session_timeout: Duration,
    
    /// How often to run cleanup operations
    pub cleanup_interval: Duration,
    
    /// How long to keep terminated sessions for metrics
    pub terminated_session_retention: Duration,
    
    /// Maximum memory usage per session (bytes)
    pub max_memory_per_session: Option<usize>,
    
    /// Enable resource leak detection
    pub enable_leak_detection: bool,
    
    /// Health check interval
    pub health_check_interval: Duration,
}

impl Default for SessionResourceConfig {
    fn default() -> Self {
        Self {
            max_sessions_per_user: Some(10),
            max_total_sessions: Some(1000),
            session_timeout: Duration::from_secs(3600), // 1 hour
            cleanup_interval: Duration::from_secs(300), // 5 minutes
            terminated_session_retention: Duration::from_secs(1800), // 30 minutes
            max_memory_per_session: Some(10 * 1024 * 1024), // 10MB
            enable_leak_detection: true,
            health_check_interval: Duration::from_secs(60), // 1 minute
        }
    }
}

/// Per-user session limits and tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserSessionLimits {
    /// User identifier (typically SIP URI)
    pub user_id: String,
    
    /// Maximum concurrent sessions for this user
    pub max_concurrent_sessions: usize,
    
    /// Current active session count
    pub active_session_count: usize,
    
    /// Total sessions created for this user
    pub total_sessions_created: u64,
    
    /// Last activity timestamp
    pub last_activity: SystemTime,
}

/// Resource metrics for a session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionResourceMetrics {
    /// Session ID
    pub session_id: SessionId,
    
    /// Session state
    pub state: SessionState,
    
    /// User identifier (if available)
    pub user_id: Option<String>,
    
    /// Remote endpoint
    pub remote_endpoint: Option<SocketAddr>,
    
    /// Session creation time
    pub created_at: SystemTime,
    
    /// Last activity time
    pub last_activity: SystemTime,
    
    /// Session state duration in current state
    pub state_duration: Duration,
    
    /// Total session duration
    pub total_duration: Duration,
    
    /// Estimated memory usage (bytes)
    pub memory_usage: usize,
    
    /// Number of dialogs associated with this session
    pub dialog_count: usize,
    
    /// Number of media sessions
    pub media_session_count: usize,
    
    /// Whether session is healthy
    pub is_healthy: bool,
    
    /// Health check timestamp
    pub last_health_check: SystemTime,
    
    /// Resource warnings (if any)
    pub warnings: Vec<String>,
}

/// Session resource tracking entry
#[derive(Debug)]
struct SessionResourceEntry {
    /// Session metrics
    pub metrics: SessionResourceMetrics,
    
    /// State change history (last 10 states)
    pub state_history: Vec<(SessionState, SystemTime)>,
    
    /// Timeout handle for cleanup
    pub timeout_handle: Option<tokio::task::JoinHandle<()>>,
}

/// Session Resource Manager
/// 
/// Provides comprehensive resource management for sessions including:
/// - Resource tracking and metrics
/// - Automatic cleanup and aging
/// - Health monitoring
/// - User-based session limits
/// - Resource leak detection
pub struct SessionResourceManager {
    /// Configuration
    config: SessionResourceConfig,
    
    /// Session resource tracking
    session_resources: Arc<DashMap<SessionId, SessionResourceEntry>>,
    
    /// User session tracking
    user_sessions: Arc<DashMap<String, UserSessionLimits>>,
    
    /// User to sessions mapping
    user_to_sessions: Arc<DashMap<String, Vec<SessionId>>>,
    
    /// Endpoint to sessions mapping
    endpoint_to_sessions: Arc<DashMap<SocketAddr, Vec<SessionId>>>,
    
    /// Terminated sessions (for metrics and analysis)
    terminated_sessions: Arc<RwLock<HashMap<SessionId, SessionResourceMetrics>>>,
    
    /// Global resource metrics
    global_metrics: Arc<RwLock<GlobalResourceMetrics>>,
    
    /// Running flag
    running: Arc<std::sync::atomic::AtomicBool>,
}

/// Global resource metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalResourceMetrics {
    /// Total active sessions
    pub active_sessions: usize,
    
    /// Total sessions created since start
    pub total_sessions_created: u64,
    
    /// Total sessions terminated
    pub total_sessions_terminated: u64,
    
    /// Total memory usage estimate (bytes)
    pub total_memory_usage: usize,
    
    /// Average session duration
    pub average_session_duration: Duration,
    
    /// Active users count
    pub active_users: usize,
    
    /// Sessions per state
    pub sessions_per_state: HashMap<SessionState, usize>,
    
    /// Resource warnings count
    pub resource_warnings: u64,
    
    /// Last cleanup time
    pub last_cleanup: SystemTime,
    
    /// Last health check time
    pub last_health_check: SystemTime,
}

impl Default for GlobalResourceMetrics {
    fn default() -> Self {
        let now = SystemTime::now();
        Self {
            active_sessions: 0,
            total_sessions_created: 0,
            total_sessions_terminated: 0,
            total_memory_usage: 0,
            average_session_duration: Duration::from_secs(0),
            active_users: 0,
            sessions_per_state: HashMap::new(),
            resource_warnings: 0,
            last_cleanup: now,
            last_health_check: now,
        }
    }
}

impl SessionResourceManager {
    /// Create a new session resource manager
    pub fn new(config: SessionResourceConfig) -> Self {
        Self {
            config,
            session_resources: Arc::new(DashMap::new()),
            user_sessions: Arc::new(DashMap::new()),
            user_to_sessions: Arc::new(DashMap::new()),
            endpoint_to_sessions: Arc::new(DashMap::new()),
            terminated_sessions: Arc::new(RwLock::new(HashMap::new())),
            global_metrics: Arc::new(RwLock::new(GlobalResourceMetrics::default())),
            running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
    
    /// Start the resource manager background tasks
    pub async fn start(&self) -> Result<(), Error> {
        self.running.store(true, std::sync::atomic::Ordering::SeqCst);
        
        // Start cleanup task
        let cleanup_manager = self.clone();
        tokio::spawn(async move {
            cleanup_manager.cleanup_task().await;
        });
        
        // Start health monitoring task
        let health_manager = self.clone();
        tokio::spawn(async move {
            health_manager.health_monitoring_task().await;
        });
        
        info!("Session resource manager started with config: {:?}", self.config);
        Ok(())
    }
    
    /// Stop the resource manager
    pub async fn stop(&self) {
        self.running.store(false, std::sync::atomic::Ordering::SeqCst);
        info!("Session resource manager stopped");
    }
    
    /// Register a new session for resource tracking
    pub async fn register_session(
        &self,
        session_id: SessionId,
        user_id: Option<String>,
        remote_endpoint: Option<SocketAddr>,
        initial_state: SessionState,
    ) -> Result<(), Error> {
        let now = SystemTime::now();
        
        // Check global session limits
        if let Some(max_total) = self.config.max_total_sessions {
            if self.session_resources.len() >= max_total {
                return Err(Error::ResourceLimitExceededDetailed {
                    resource: "total_sessions".to_string(),
                    limit: max_total,
                    current: self.session_resources.len(),
                    context: ErrorContext {
                        category: ErrorCategory::Session,
                        severity: ErrorSeverity::Error,
                        recovery: RecoveryAction::WaitAndRetry(Duration::from_secs(30)),
                        retryable: true,
                        session_id: Some(session_id.to_string()),
                        timestamp: now,
                        details: Some("Maximum total sessions limit reached".to_string()),
                        ..Default::default()
                    }
                });
            }
        }
        
        // Check per-user session limits
        if let Some(user_id) = &user_id {
            self.check_user_session_limits(user_id, &session_id).await?;
        }
        
        // Create session metrics
        let metrics = SessionResourceMetrics {
            session_id: session_id.clone(),
            state: initial_state,
            user_id: user_id.clone(),
            remote_endpoint,
            created_at: now,
            last_activity: now,
            state_duration: Duration::from_secs(0),
            total_duration: Duration::from_secs(0),
            memory_usage: 1024, // Base memory usage estimate
            dialog_count: 0,
            media_session_count: 0,
            is_healthy: true,
            last_health_check: now,
            warnings: Vec::new(),
        };
        
        // Create resource entry
        let entry = SessionResourceEntry {
            metrics,
            state_history: vec![(initial_state, now)],
            timeout_handle: None,
        };
        
        // Register session
        self.session_resources.insert(session_id.clone(), entry);
        
        // Update user tracking
        if let Some(user_id) = &user_id {
            self.update_user_session_count(user_id, 1).await;
            
            // Add to user-to-sessions mapping
            let mut user_sessions = self.user_to_sessions.entry(user_id.clone()).or_insert_with(Vec::new);
            user_sessions.push(session_id.clone());
        }
        
        // Update endpoint tracking
        if let Some(endpoint) = remote_endpoint {
            let mut endpoint_sessions = self.endpoint_to_sessions.entry(endpoint).or_insert_with(Vec::new);
            endpoint_sessions.push(session_id.clone());
        }
        
        // Update global metrics
        self.update_global_metrics_on_session_creation(initial_state).await;
        
        debug!("Session {} registered for resource tracking (user: {:?}, endpoint: {:?})", 
            session_id, user_id, remote_endpoint);
        
        Ok(())
    }
    
    /// Update session state
    pub async fn update_session_state(&self, session_id: &SessionId, new_state: SessionState) -> Result<(), Error> {
        if let Some(mut entry) = self.session_resources.get_mut(session_id) {
            let now = SystemTime::now();
            let old_state = entry.metrics.state;
            
            // Update state duration
            if let Ok(duration) = now.duration_since(entry.metrics.last_activity) {
                entry.metrics.state_duration = duration;
            }
            
            // Update state and activity
            entry.metrics.state = new_state;
            entry.metrics.last_activity = now;
            
            // Add to state history (keep last 10)
            entry.state_history.push((new_state, now));
            if entry.state_history.len() > 10 {
                entry.state_history.remove(0);
            }
            
            // Update global state metrics
            self.update_global_state_metrics(old_state, new_state).await;
            
            debug!("Session {} state updated: {} → {}", session_id, old_state, new_state);
            
            // Handle termination
            if new_state == SessionState::Terminated {
                self.handle_session_termination(session_id).await?;
            }
        }
        
        Ok(())
    }
    
    /// Update session resource usage
    pub async fn update_session_resources(
        &self,
        session_id: &SessionId,
        dialog_count: Option<usize>,
        media_session_count: Option<usize>,
        memory_usage: Option<usize>,
    ) -> Result<(), Error> {
        if let Some(mut entry) = self.session_resources.get_mut(session_id) {
            if let Some(dialogs) = dialog_count {
                entry.metrics.dialog_count = dialogs;
            }
            
            if let Some(media_sessions) = media_session_count {
                entry.metrics.media_session_count = media_sessions;
            }
            
            if let Some(memory) = memory_usage {
                entry.metrics.memory_usage = memory;
                
                // Check memory limits
                if let Some(max_memory) = self.config.max_memory_per_session {
                    if memory > max_memory {
                        entry.metrics.warnings.push(format!(
                            "Session memory usage ({} bytes) exceeds limit ({} bytes)",
                            memory, max_memory
                        ));
                        
                        warn!("Session {} memory usage warning: {} bytes > {} bytes", 
                            session_id, memory, max_memory);
                    }
                }
            }
            
            entry.metrics.last_activity = SystemTime::now();
        }
        
        Ok(())
    }
    
    /// Get session resource metrics
    pub async fn get_session_metrics(&self, session_id: &SessionId) -> Option<SessionResourceMetrics> {
        self.session_resources.get(session_id).map(|entry| {
            let mut metrics = entry.metrics.clone();
            
            // Update total duration
            if let Ok(duration) = SystemTime::now().duration_since(metrics.created_at) {
                metrics.total_duration = duration;
            }
            
            metrics
        })
    }
    
    /// Get metrics for all active sessions
    pub async fn get_all_session_metrics(&self) -> Vec<SessionResourceMetrics> {
        let now = SystemTime::now();
        
        self.session_resources.iter().map(|entry| {
            let mut metrics = entry.metrics.clone();
            
            // Update total duration
            if let Ok(duration) = now.duration_since(metrics.created_at) {
                metrics.total_duration = duration;
            }
            
            metrics
        }).collect()
    }
    
    /// Get global resource metrics
    pub async fn get_global_metrics(&self) -> GlobalResourceMetrics {
        let mut global = self.global_metrics.read().await.clone();
        
        // Update real-time metrics
        global.active_sessions = self.session_resources.len();
        
        global
    }
    
    /// Get user session limits and current usage
    pub async fn get_user_limits(&self, user_id: &str) -> Option<UserSessionLimits> {
        self.user_sessions.get(user_id).map(|entry| entry.clone())
    }
    
    /// Cleanup terminated sessions periodically
    async fn cleanup_task(&self) {
        let mut interval = interval(self.config.cleanup_interval);
        
        while self.running.load(std::sync::atomic::Ordering::SeqCst) {
            interval.tick().await;
            
            if let Err(e) = self.cleanup_terminated_sessions().await {
                error!("Failed to cleanup terminated sessions: {}", e);
            }
        }
    }
    
    /// Cleanup terminated sessions
    pub async fn cleanup_terminated_sessions(&self) -> Result<usize, Error> {
        let now = SystemTime::now();
        let mut cleaned_count = 0;
        
        // Collect sessions to remove
        let sessions_to_remove: Vec<SessionId> = self.session_resources
            .iter()
            .filter_map(|entry| {
                let session_id = entry.key().clone();
                let metrics = &entry.metrics;
                
                // Remove terminated sessions older than retention period
                if metrics.state == SessionState::Terminated {
                    if let Ok(since_termination) = now.duration_since(metrics.last_activity) {
                        if since_termination > self.config.terminated_session_retention {
                            return Some(session_id);
                        }
                    }
                }
                
                // Remove very old sessions that might be stuck
                if let Ok(total_duration) = now.duration_since(metrics.created_at) {
                    if total_duration > (self.config.session_timeout * 2) {
                        return Some(session_id);
                    }
                }
                
                None
            })
            .collect();
        
        // Remove sessions
        for session_id in sessions_to_remove {
            if let Some((_, entry)) = self.session_resources.remove(&session_id) {
                // Move to terminated sessions for historical analysis
                let mut terminated = self.terminated_sessions.write().await;
                terminated.insert(session_id.clone(), entry.metrics.clone());
                
                // Cleanup user mappings
                if let Some(user_id) = &entry.metrics.user_id {
                    self.update_user_session_count(user_id, -1).await;
                    
                    if let Some(mut user_sessions) = self.user_to_sessions.get_mut(user_id) {
                        user_sessions.retain(|id| *id != session_id);
                    }
                }
                
                // Cleanup endpoint mappings
                if let Some(endpoint) = entry.metrics.remote_endpoint {
                    if let Some(mut endpoint_sessions) = self.endpoint_to_sessions.get_mut(&endpoint) {
                        endpoint_sessions.retain(|id| *id != session_id);
                    }
                }
                
                cleaned_count += 1;
            }
        }
        
        // Update global metrics
        let mut global = self.global_metrics.write().await;
        global.last_cleanup = now;
        
        if cleaned_count > 0 {
            info!("Cleaned up {} terminated sessions", cleaned_count);
        } else {
            debug!("Session cleanup: no sessions to clean");
        }
        
        Ok(cleaned_count)
    }
    
    /// Health monitoring task
    async fn health_monitoring_task(&self) {
        let mut interval = interval(self.config.health_check_interval);
        
        while self.running.load(std::sync::atomic::Ordering::SeqCst) {
            interval.tick().await;
            
            if let Err(e) = self.perform_health_checks().await {
                error!("Failed to perform health checks: {}", e);
            }
        }
    }
    
    /// Perform health checks on all sessions
    pub async fn perform_health_checks(&self) -> Result<usize, Error> {
        let now = SystemTime::now();
        let mut unhealthy_count = 0;
        
        for mut entry in self.session_resources.iter_mut() {
            let session_id = entry.key().clone();
            let metrics = &mut entry.metrics;
            
            let mut is_healthy = true;
            let mut warnings = Vec::new();
            
            // Check session age
            if let Ok(age) = now.duration_since(metrics.created_at) {
                if age > self.config.session_timeout {
                    is_healthy = false;
                    warnings.push(format!("Session age ({:?}) exceeds timeout ({:?})", age, self.config.session_timeout));
                }
            }
            
            // Check last activity
            if let Ok(inactive_time) = now.duration_since(metrics.last_activity) {
                if inactive_time > (self.config.session_timeout / 2) {
                    warnings.push(format!("Session inactive for {:?}", inactive_time));
                }
            }
            
            // Check memory usage
            if let Some(max_memory) = self.config.max_memory_per_session {
                if metrics.memory_usage > max_memory {
                    is_healthy = false;
                    warnings.push(format!("Memory usage {} exceeds limit {}", metrics.memory_usage, max_memory));
                }
            }
            
            // Check for stuck states
            if let Ok(state_duration) = now.duration_since(metrics.last_activity) {
                match metrics.state {
                    SessionState::Terminating if state_duration > Duration::from_secs(30) => {
                        is_healthy = false;
                        warnings.push("Session stuck in Terminating state".to_string());
                    },
                    SessionState::Dialing if state_duration > Duration::from_secs(60) => {
                        warnings.push("Session stuck in Dialing state".to_string());
                    },
                    _ => {}
                }
            }
            
            if !is_healthy {
                unhealthy_count += 1;
            }
            
            // Update health status
            metrics.is_healthy = is_healthy;
            metrics.warnings = warnings;
            metrics.last_health_check = now;
        }
        
        // Update global metrics
        let mut global = self.global_metrics.write().await;
        global.last_health_check = now;
        
        if unhealthy_count > 0 {
            warn!("Health check found {} unhealthy sessions", unhealthy_count);
        } else {
            debug!("Health check: all sessions healthy");
        }
        
        Ok(unhealthy_count)
    }
    
    /// Check user session limits
    async fn check_user_session_limits(&self, user_id: &str, session_id: &SessionId) -> Result<(), Error> {
        let max_per_user = self.config.max_sessions_per_user.unwrap_or(usize::MAX);
        
        let current_count = self.user_to_sessions.get(user_id)
            .map(|sessions| sessions.len())
            .unwrap_or(0);
        
        if current_count >= max_per_user {
            return Err(Error::ResourceLimitExceededDetailed {
                resource: format!("user_sessions:{}", user_id),
                limit: max_per_user,
                current: current_count,
                context: ErrorContext {
                    category: ErrorCategory::Session,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::WaitAndRetry(Duration::from_secs(60)),
                    retryable: true,
                    session_id: Some(session_id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some(format!("User {} has reached session limit of {}", user_id, max_per_user)),
                    ..Default::default()
                }
            });
        }
        
        Ok(())
    }
    
    /// Update user session count
    async fn update_user_session_count(&self, user_id: &str, delta: i32) {
        let mut user_limits = self.user_sessions.entry(user_id.to_string()).or_insert_with(|| {
            UserSessionLimits {
                user_id: user_id.to_string(),
                max_concurrent_sessions: self.config.max_sessions_per_user.unwrap_or(usize::MAX),
                active_session_count: 0,
                total_sessions_created: 0,
                last_activity: SystemTime::now(),
            }
        });
        
        if delta > 0 {
            user_limits.active_session_count += delta as usize;
            user_limits.total_sessions_created += 1;
        } else if delta < 0 && user_limits.active_session_count > 0 {
            user_limits.active_session_count -= (-delta) as usize;
        }
        
        user_limits.last_activity = SystemTime::now();
    }
    
    /// Handle session termination
    async fn handle_session_termination(&self, session_id: &SessionId) -> Result<(), Error> {
        // Session will be moved to terminated_sessions during cleanup
        // Update global metrics
        let mut global = self.global_metrics.write().await;
        global.total_sessions_terminated += 1;
        
        debug!("Session {} marked for termination cleanup", session_id);
        Ok(())
    }
    
    /// Update global metrics on session creation
    async fn update_global_metrics_on_session_creation(&self, initial_state: SessionState) {
        let mut global = self.global_metrics.write().await;
        global.total_sessions_created += 1;
        
        // Update state counts
        *global.sessions_per_state.entry(initial_state).or_insert(0) += 1;
    }
    
    /// Update global state metrics
    async fn update_global_state_metrics(&self, old_state: SessionState, new_state: SessionState) {
        let mut global = self.global_metrics.write().await;
        
        // Update state counts
        if let Some(count) = global.sessions_per_state.get_mut(&old_state) {
            if *count > 0 {
                *count -= 1;
            }
        }
        
        *global.sessions_per_state.entry(new_state).or_insert(0) += 1;
    }
}

impl Clone for SessionResourceManager {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            session_resources: self.session_resources.clone(),
            user_sessions: self.user_sessions.clone(),
            user_to_sessions: self.user_to_sessions.clone(),
            endpoint_to_sessions: self.endpoint_to_sessions.clone(),
            terminated_sessions: self.terminated_sessions.clone(),
            global_metrics: self.global_metrics.clone(),
            running: self.running.clone(),
        }
    }
} 