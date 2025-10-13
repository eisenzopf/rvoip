//! Session cleanup and resource management

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::interval;
use serde::{Serialize, Deserialize};
use tracing::{info, debug};
use crate::types::CallState;
use super::{SessionStore, SessionState};

/// Configuration for automatic cleanup
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupConfig {
    /// How often to run cleanup
    pub interval: Duration,
    
    /// TTL for terminated sessions
    pub terminated_ttl: Duration,
    
    /// TTL for failed sessions
    pub failed_ttl: Duration,
    
    /// Maximum idle time before cleanup
    pub max_idle_time: Duration,
    
    /// Maximum session age
    pub max_session_age: Duration,
    
    /// Enable automatic cleanup
    pub enabled: bool,
    
    /// Maximum memory usage before aggressive cleanup (in bytes)
    pub max_memory_bytes: Option<usize>,
    
    /// Maximum number of sessions before cleanup
    pub max_sessions: Option<usize>,
}

impl Default for CleanupConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(60),
            terminated_ttl: Duration::from_secs(300),      // 5 minutes
            failed_ttl: Duration::from_secs(600),          // 10 minutes
            max_idle_time: Duration::from_secs(3600),      // 1 hour
            max_session_age: Duration::from_secs(86400),   // 24 hours
            enabled: true,
            max_memory_bytes: None,
            max_sessions: None,
        }
    }
}

/// Statistics from cleanup run
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CleanupStats {
    pub sessions_checked: usize,
    pub total_removed: usize,
    pub terminated_removed: usize,
    pub failed_removed: usize,
    pub idle_removed: usize,
    pub aged_removed: usize,
    pub memory_pressure_removed: usize,
    pub active_preserved: usize,
    pub duration: Duration,
}

/// Resource limits for the session store
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Maximum number of concurrent sessions
    pub max_sessions: Option<usize>,
    
    /// Maximum memory per session (in bytes)
    pub max_memory_per_session: Option<usize>,
    
    /// Maximum history entries per session
    pub max_history_per_session: usize,
    
    /// Rate limit for new sessions (per second)
    pub max_sessions_per_second: Option<f64>,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_sessions: None,
            max_memory_per_session: Some(10 * 1024 * 1024), // 10MB
            max_history_per_session: 100,
            max_sessions_per_second: None,
        }
    }
}

/// Overall resource usage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceUsageReport {
    pub total_sessions: usize,
    pub active_sessions: usize,
    pub idle_sessions: usize,
    pub total_memory_bytes: usize,
    pub average_memory_per_session: usize,
    pub history_entries_total: usize,
    pub oldest_session_age: Option<Duration>,
    pub most_idle_session: Option<Duration>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
enum RemovalReason {
    Terminated,
    Failed,
    Idle,
    Aged,
    MemoryPressure,
}

impl SessionStore {
    /// Run cleanup on sessions based on config
    pub async fn cleanup_sessions(&self, config: CleanupConfig) -> CleanupStats {
        let start = Instant::now();
        let mut stats = CleanupStats::default();
        
        // Check if cleanup is enabled
        if !config.enabled {
            return stats; // Return empty stats if disabled
        }
        
        let sessions = self.sessions.read().await;
        let total_sessions = sessions.len();
        stats.sessions_checked = total_sessions;
        
        let mut to_remove = Vec::new();
        let now = Instant::now();
        
        // Phase 1: Identify sessions to remove
        for (id, session) in sessions.iter() {
            let session_age = now.duration_since(session.created_at);
            let time_in_state = now.duration_since(session.entered_state_at);
            
            let should_remove = match session.call_state {
                CallState::Terminated => {
                    if time_in_state > config.terminated_ttl {
                        stats.terminated_removed += 1;
                        true
                    } else {
                        false
                    }
                }
                CallState::Failed(_) => {
                    if time_in_state > config.failed_ttl {
                        stats.failed_removed += 1;
                        true
                    } else {
                        false
                    }
                }
                CallState::Active | CallState::OnHold | CallState::Bridged => {
                    // Never auto-remove active calls
                    stats.active_preserved += 1;
                    false
                }
                _ => {
                    // Check idle time for other states
                    if let Some(history) = &session.history {
                        if history.idle_time() > config.max_idle_time {
                            stats.idle_removed += 1;
                            true
                        } else if session_age > config.max_session_age {
                            stats.aged_removed += 1;
                            true
                        } else {
                            false
                        }
                    } else if session_age > config.max_session_age {
                        stats.aged_removed += 1;
                        true
                    } else {
                        false
                    }
                }
            };
            
            if should_remove {
                to_remove.push(id.clone());
            }
        }
        
        // Phase 2: Check resource limits
        if let Some(max_sessions) = config.max_sessions {
            if total_sessions > max_sessions {
                let excess = total_sessions - max_sessions;
                // Sort by age and remove oldest idle sessions
                let mut idle_sessions: Vec<_> = sessions.iter()
                    .filter(|(id, s)| {
                        !to_remove.contains(id) && 
                        !matches!(s.call_state, CallState::Active | CallState::OnHold | CallState::Bridged)
                    })
                    .map(|(id, s)| (id.clone(), s.created_at))
                    .collect();
                
                idle_sessions.sort_by_key(|(_, created)| *created);
                
                for (id, _) in idle_sessions.iter().take(excess) {
                    to_remove.push(id.clone());
                    stats.memory_pressure_removed += 1;
                }
            }
        }
        
        // Phase 3: Check memory pressure
        if let Some(max_bytes) = config.max_memory_bytes {
            let estimated_size = total_sessions * std::mem::size_of::<SessionState>();
            if estimated_size > max_bytes {
                // Remove more idle sessions if needed
                let mut idle_sessions: Vec<_> = sessions.iter()
                    .filter(|(id, s)| {
                        !to_remove.contains(id) && 
                        !matches!(s.call_state, CallState::Active | CallState::OnHold | CallState::Bridged)
                    })
                    .map(|(id, s)| (id.clone(), s.created_at))
                    .collect();
                
                idle_sessions.sort_by_key(|(_, created)| *created);
                
                // Remove oldest sessions until under memory limit
                for (id, _) in idle_sessions {
                    to_remove.push(id);
                    stats.memory_pressure_removed += 1;
                    
                    let new_size = (total_sessions - to_remove.len()) * std::mem::size_of::<SessionState>();
                    if new_size < max_bytes {
                        break;
                    }
                }
            }
        }
        
        drop(sessions); // Release read lock
        
        // Phase 4: Actually remove sessions
        if !to_remove.is_empty() {
            let mut sessions = self.sessions.write().await;
            for id in &to_remove {
                sessions.remove(id);
            }
            stats.total_removed = to_remove.len();
        }
        
        stats.duration = start.elapsed();
        stats
    }
    
    /// Start automatic cleanup task
    pub fn start_cleanup_task(self: Arc<Self>, config: CleanupConfig) -> tokio::task::JoinHandle<()> {
        if !config.enabled {
            info!("Automatic cleanup is disabled");
            return tokio::spawn(async {});
        }
        
        info!("Starting automatic cleanup task with interval: {:?}", config.interval);
        
        return tokio::spawn(async move {
            let mut cleanup_interval = interval(config.interval);
            let mut _consecutive_failures = 0;
            
            loop {
                cleanup_interval.tick().await;
                
                let stats = self.cleanup_sessions(config.clone()).await;
                
                _consecutive_failures = 0;
                
                if stats.total_removed > 0 {
                    info!(
                        "Cleanup completed: removed {} of {} sessions in {:?}",
                        stats.total_removed,
                        stats.sessions_checked,
                        stats.duration
                    );
                    
                    debug!(
                        "Cleanup details: terminated={}, failed={}, idle={}, aged={}, memory={}",
                        stats.terminated_removed,
                        stats.failed_removed,
                        stats.idle_removed,
                        stats.aged_removed,
                        stats.memory_pressure_removed
                    );
                } else {
                    debug!(
                        "Cleanup completed: no sessions removed (examined {} in {:?})",
                        stats.sessions_checked,
                        stats.duration
                    );
                }
            }
        });
    }
    
    /* Duplicate - removed
    /// Perform cleanup of stale sessions
    pub async fn cleanup_stale_sessions_old(&self, config: &CleanupConfig) -> Result<CleanupStats> {
        let start = Instant::now();
        let mut stats = CleanupStats::default();
        
        let now = Instant::now();
        let mut sessions_to_remove = Vec::new();
        
        // Phase 1: Identify sessions to remove
        {
            let sessions = self.sessions.read().await;
            stats.sessions_examined = sessions.len();
            
            for (id, state) in sessions.iter() {
                let (should_remove, reason) = self.should_remove_session(state, config, now);
                
                if should_remove {
                    sessions_to_remove.push((id.clone(), reason));
                    stats.memory_freed_bytes += std::mem::size_of_val(state);
                    
                    match reason {
                        RemovalReason::Terminated => stats.terminated_removed += 1,
                        RemovalReason::Failed => stats.failed_removed += 1,
                        RemovalReason::Idle => stats.idle_removed += 1,
                        RemovalReason::Aged => stats.aged_removed += 1,
                        RemovalReason::MemoryPressure => stats.memory_pressure_removed += 1,
                    }
                }
            }
        }
        
        // Phase 2: Remove identified sessions
        for (session_id, reason) in sessions_to_remove {
            self.remove_session(&session_id, reason).await?;
            stats.sessions_removed += 1;
        }
        
        // Phase 3: Check memory pressure
        if let Some(max_memory) = config.max_memory_bytes {
            let current_memory = self.estimate_memory_usage().await;
            if current_memory > max_memory {
                debug!(
                    "Memory pressure detected: {} bytes > {} bytes limit",
                    current_memory, max_memory
                );
                let additional = self.cleanup_for_memory(max_memory).await?;
                stats.sessions_removed += additional;
                stats.memory_pressure_removed += additional;
            }
        }
        
        stats.cleanup_duration_ms = start.elapsed().as_millis() as u64;
        Ok(stats)
    }
    */
    
    /* Commenting out old helper functions
    fn should_remove_session(
        &self,
        state: &SessionState,
        config: &CleanupConfig,
        now: Instant,
    ) -> (bool, RemovalReason) {
        let state_age = now.saturating_duration_since(state.entered_state_at);
        
        // Check terminated sessions
        if state.call_state == CallState::Terminated {
            if state_age > config.terminated_ttl {
                return (true, RemovalReason::Terminated);
            }
        }
        
        // Check failed sessions
        if matches!(state.call_state, CallState::Failed(_)) {
            if state_age > config.failed_ttl {
                return (true, RemovalReason::Failed);
            }
        }
        
        // Check idle time (only for non-active states)
        if !matches!(state.call_state, CallState::Active | CallState::OnHold | CallState::Bridged) {
            if let Some(history) = &state.history {
                let idle_time = now.saturating_duration_since(history.last_activity);
                if idle_time > config.max_idle_time {
                    return (true, RemovalReason::Idle);
                }
            }
        }
        
        // Check total age
        let total_age = now.saturating_duration_since(state.created_at);
        if total_age > config.max_session_age {
            return (true, RemovalReason::Aged);
        }
        
        (false, RemovalReason::Terminated) // Dummy reason, won't be used
    }
    
    async fn remove_session(&self, session_id: &SessionId, reason: RemovalReason) -> Result<()> {
        debug!("Removing session {} (reason: {:?})", session_id, reason);
        
        // Clean up related resources
        self.cleanup_session_resources(session_id).await?;
        
        // Remove from all indexes
        self.sessions.write().await.remove(session_id);
        self.by_dialog.write().await.retain(|_, v| v != session_id);
        self.by_call_id.write().await.retain(|_, v| v != session_id);
        self.by_media_id.write().await.retain(|_, v| v != session_id);
        
        Ok(())
    }
    
    async fn cleanup_session_resources(&self, _session_id: &SessionId) -> Result<()> {
        // Clean up any associated resources
        // TODO: When dialog and media adapters are available:
        // - Call dialog adapter cleanup
        // - Call media adapter cleanup
        // - Cancel any pending timers
        // - Clear any message queues
        
        Ok(())
    }
    
    async fn estimate_memory_usage(&self) -> usize {
        let sessions = self.sessions.read().await;
        sessions.values().map(|s| std::mem::size_of_val(s)).sum()
    }
    
    async fn cleanup_for_memory(&self, target_bytes: usize) -> Result<usize> {
        // Remove oldest idle sessions until under memory target
        let mut removed = 0;
        
        // Get sessions sorted by last activity
        let mut idle_sessions: Vec<_> = {
            let sessions = self.sessions.read().await;
            sessions
                .iter()
                .filter_map(|(id, state)| {
                    // Only consider non-active sessions for memory cleanup
                    if !matches!(state.call_state, CallState::Active | CallState::OnHold | CallState::Bridged) {
                        state.history.as_ref().map(|h| (id.clone(), h.last_activity))
                    } else {
                        None
                    }
                })
                .collect()
        };
        
        // Sort by last activity (oldest first)
        idle_sessions.sort_by_key(|(_, last_activity)| *last_activity);
        
        for (session_id, _) in idle_sessions {
            if self.estimate_memory_usage().await <= target_bytes {
                break;
            }
            
            self.remove_session(&session_id, RemovalReason::MemoryPressure).await?;
            removed += 1;
        }
        
        if removed > 0 {
            info!("Removed {} sessions due to memory pressure", removed);
        }
        
        Ok(removed)
    }
    
    /// Check if we can create a new session (resource limits)
    pub async fn can_create_session(&self, limits: &ResourceLimits) -> Result<()> {
        // Check max sessions limit
        if let Some(max) = limits.max_sessions {
            let count = self.sessions.read().await.len();
            if count >= max {
                return Err(SessionError::ResourceExhausted(
                    format!("Maximum sessions ({}) reached", max)
                ));
            }
        }
        
        // Check rate limit
        if let Some(_rate) = limits.max_sessions_per_second {
            // TODO: Implement rate limiting with token bucket or similar
            // For now, just allow it
        }
        
        Ok(())
    }
    
    /// Get current resource usage
    pub async fn get_resource_usage(&self) -> ResourceUsageReport {
        let sessions = self.sessions.read().await;
        
        let mut active = 0;
        let mut idle = 0;
        let mut total_memory = 0;
        let mut history_total = 0;
        let mut oldest_session: Option<Instant> = None;
        let mut most_idle: Option<Instant> = None;
        
        for state in sessions.values() {
            // Count active vs idle
            if matches!(state.call_state, CallState::Active | CallState::OnHold | CallState::Bridged) {
                active += 1;
            } else {
                idle += 1;
            }
            
            // Calculate memory
            total_memory += std::mem::size_of_val(state);
            
            // Count history entries
            if let Some(history) = &state.history {
                history_total += history.total_transitions as usize;
                
                // Track most idle session
                if most_idle.is_none() || history.last_activity < most_idle.unwrap() {
                    most_idle = Some(history.last_activity);
                }
            }
            
            // Track oldest session
            if oldest_session.is_none() || state.created_at < oldest_session.unwrap() {
                oldest_session = Some(state.created_at);
            }
        }
        
        let total = sessions.len();
        let avg_memory = if total > 0 { total_memory / total } else { 0 };
        
        ResourceUsageReport {
            total_sessions: total,
            active_sessions: active,
            idle_sessions: idle,
            total_memory_bytes: total_memory,
            average_memory_per_session: avg_memory,
            history_entries_total: history_total,
            oldest_session_age: oldest_session.map(|t| t.elapsed()),
            most_idle_session: most_idle.map(|t| t.elapsed()),
        }
    }
    
    /// Manually trigger cleanup
    pub async fn manual_cleanup(&self, config: &CleanupConfig) -> Result<CleanupStats> {
        info!("Manual cleanup triggered");
        self.cleanup_sessions(config.clone()).await.map_err(|e| anyhow::anyhow!("{}", e))
    }
    
    /// Force remove a specific session
    pub async fn force_remove_session(&self, session_id: &SessionId) -> Result<()> {
        info!("Force removing session {}", session_id);
        self.remove_session(session_id).await.map_err(|e| anyhow::anyhow!("{}", e))
    }
    */
}

/* Old tests - commented out
#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_table::Role;
    use crate::session_store::history::HistoryConfig;
    
    #[tokio::test]
    async fn test_cleanup_terminated_sessions() {
        let store = Arc::new(SessionStore::new());
        
        // Create a terminated session
        let session_id = SessionId::new();
        let mut state = SessionState::with_history(
            session_id.clone(),
            Role::UAC,
            HistoryConfig::default()
        );
        state.call_state = CallState::Terminated;
        state.entered_state_at = Instant::now() - Duration::from_secs(400); // Old enough
        
        store.sessions.write().await.insert(session_id.clone(), state);
        
        // Run cleanup with short TTL
        let config = CleanupConfig {
            terminated_ttl: Duration::from_secs(300),
            ..Default::default()
        };
        
        let stats = store.cleanup_stale_sessions(&config).await.unwrap();
        
        assert_eq!(stats.sessions_examined, 1);
        assert_eq!(stats.sessions_removed, 1);
        assert_eq!(stats.terminated_removed, 1);
        assert!(store.sessions.read().await.is_empty());
    }
    
    #[tokio::test]
    async fn test_cleanup_idle_sessions() {
        let store = Arc::new(SessionStore::new());
        
        // Create an idle session
        let session_id = SessionId::new();
        let mut state = SessionState::with_history(
            session_id.clone(),
            Role::UAS,
            HistoryConfig::default()
        );
        state.call_state = CallState::Initiating;
        
        // Make it idle
        if let Some(history) = &mut state.history {
            history.last_activity = Instant::now() - Duration::from_secs(7200); // 2 hours ago
        }
        
        store.sessions.write().await.insert(session_id.clone(), state);
        
        // Run cleanup
        let config = CleanupConfig {
            max_idle_time: Duration::from_secs(3600), // 1 hour
            ..Default::default()
        };
        
        let stats = store.cleanup_stale_sessions(&config).await.unwrap();
        
        assert_eq!(stats.sessions_examined, 1);
        assert_eq!(stats.sessions_removed, 1);
        assert_eq!(stats.idle_removed, 1);
    }
    
    #[tokio::test]
    async fn test_cleanup_preserves_active_sessions() {
        let store = Arc::new(SessionStore::new());
        
        // Create an active session (should not be removed)
        let session_id = SessionId::new();
        let mut state = SessionState::with_history(
            session_id.clone(),
            Role::UAC,
            HistoryConfig::default()
        );
        state.call_state = CallState::Active;
        state.created_at = Instant::now() - Duration::from_secs(7200); // Old but active
        
        store.sessions.write().await.insert(session_id.clone(), state);
        
        // Run aggressive cleanup
        let config = CleanupConfig {
            max_idle_time: Duration::from_secs(60),
            max_session_age: Duration::from_secs(3600),
            ..Default::default()
        };
        
        let stats = store.cleanup_stale_sessions(&config).await.unwrap();
        
        // Active session should not be removed despite being old
        assert_eq!(stats.sessions_examined, 1);
        assert_eq!(stats.sessions_removed, 0);
        assert_eq!(store.sessions.read().await.len(), 1);
    }
    
    #[tokio::test]
    async fn test_resource_limits() {
        let store = Arc::new(SessionStore::new());
        let limits = ResourceLimits {
            max_sessions: Some(2),
            ..Default::default()
        };
        
        // Create 2 sessions (at limit)
        for i in 0..2 {
            let session_id = SessionId::new();
            let state = SessionState::new(session_id.clone(), Role::UAC);
            store.sessions.write().await.insert(session_id, state);
        }
        
        // Should be able to create at limit
        assert!(store.can_create_session(&limits).await.is_ok());
        
        // Add one more to exceed limit
        let session_id = SessionId::new();
        let state = SessionState::new(session_id.clone(), Role::UAC);
        store.sessions.write().await.insert(session_id, state);
        
        // Should not be able to create beyond limit
        assert!(store.can_create_session(&limits).await.is_err());
    }
}
*/