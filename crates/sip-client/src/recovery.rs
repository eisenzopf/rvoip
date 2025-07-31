//! Error recovery and resilience mechanisms for the SIP client
//!
//! This module provides comprehensive error recovery, automatic reconnection,
//! and graceful degradation capabilities.

use crate::{
    error::{SipClientError, SipClientResult},
    events::{SipClientEvent, EventEmitter},
    types::{CallId, CallState},
};
use std::{
    sync::Arc,
    time::Duration,
    collections::HashMap,
};
use tokio::{
    sync::{RwLock, Mutex},
    time::{interval, sleep, Instant},
};
use tracing::{debug, error, info, warn};

/// Configuration for error recovery behavior
#[derive(Debug, Clone)]
pub struct RecoveryConfig {
    /// Enable automatic reconnection
    pub auto_reconnect: bool,
    
    /// Maximum number of reconnection attempts
    pub max_reconnect_attempts: u32,
    
    /// Initial reconnection delay
    pub initial_reconnect_delay: Duration,
    
    /// Maximum reconnection delay (for exponential backoff)
    pub max_reconnect_delay: Duration,
    
    /// Backoff multiplier for reconnection delays
    pub backoff_multiplier: f64,
    
    /// Enable graceful degradation
    pub enable_degradation: bool,
    
    /// Timeout for operations before considering them failed
    pub operation_timeout: Duration,
    
    /// Enable automatic codec fallback
    pub auto_codec_fallback: bool,
    
    /// Enable quality adaptation based on network conditions
    pub adaptive_quality: bool,
}

impl Default for RecoveryConfig {
    fn default() -> Self {
        Self {
            auto_reconnect: true,
            max_reconnect_attempts: 5,
            initial_reconnect_delay: Duration::from_secs(1),
            max_reconnect_delay: Duration::from_secs(60),
            backoff_multiplier: 2.0,
            enable_degradation: true,
            operation_timeout: Duration::from_secs(30),
            auto_codec_fallback: true,
            adaptive_quality: true,
        }
    }
}

/// Recovery state for a specific component
#[derive(Debug, Clone)]
pub struct RecoveryState {
    /// Current number of retry attempts
    pub retry_count: u32,
    
    /// Last error that occurred
    pub last_error: Option<String>,
    
    /// Time of last failure
    pub last_failure: Option<Instant>,
    
    /// Current backoff delay
    pub current_backoff: Duration,
    
    /// Whether recovery is in progress
    pub recovering: bool,
}

impl Default for RecoveryState {
    fn default() -> Self {
        Self {
            retry_count: 0,
            last_error: None,
            last_failure: None,
            current_backoff: Duration::from_secs(1),
            recovering: false,
        }
    }
}

/// Error recovery manager
pub struct RecoveryManager {
    /// Recovery configuration
    config: RecoveryConfig,
    
    /// Recovery states for different components
    states: Arc<RwLock<HashMap<String, RecoveryState>>>,
    
    /// Event emitter for recovery events
    event_emitter: EventEmitter,
    
    /// Active recovery tasks
    recovery_tasks: Arc<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>>,
}

impl RecoveryManager {
    /// Create a new recovery manager
    pub fn new(config: RecoveryConfig, event_emitter: EventEmitter) -> Self {
        Self {
            config,
            states: Arc::new(RwLock::new(HashMap::new())),
            event_emitter,
            recovery_tasks: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    
    /// Handle an error and initiate recovery if needed
    pub async fn handle_error(
        &self,
        component: &str,
        error: &SipClientError,
        recovery_action: impl Fn() -> futures::future::BoxFuture<'static, SipClientResult<()>> + Send + Sync + 'static,
    ) -> SipClientResult<()> {
        warn!("Error in component {}: {:?}", component, error);
        
        // Check and update recovery state
        let (should_recover, recovery_state) = {
            let mut states = self.states.write().await;
            let state = states.entry(component.to_string()).or_default();
            
            state.last_error = Some(error.to_string());
            state.last_failure = Some(Instant::now());
            
            // Check if we should attempt recovery with current state
            let can_recover = !state.recovering && state.retry_count < self.config.max_reconnect_attempts;
            
            if can_recover {
                // Update state for recovery
                state.recovering = true;
                state.retry_count += 1;
                (true, state.clone())
            } else {
                (false, state.clone())
            }
        };
        
        if !should_recover {
            error!("Recovery not possible for {} (recovering: {}, retry_count: {})", 
                   component, recovery_state.recovering, recovery_state.retry_count);
            return Err(SipClientError::Internal {
                message: format!("Recovery failed for {}: {}", component, error),
            });
        }
        
        let component_name = component.to_string();
        let config = self.config.clone();
        let states_arc = self.states.clone();
        let event_emitter = self.event_emitter.clone();
        let recovery_action = Arc::new(recovery_action);
        
        // Spawn recovery task
        let task = tokio::spawn(async move {
            Self::perform_recovery(
                component_name,
                recovery_state,
                config,
                states_arc,
                event_emitter,
                recovery_action,
            ).await;
        });
        
        // Store the recovery task
        let mut tasks = self.recovery_tasks.lock().await;
        tasks.insert(component.to_string(), task);
        
        Ok(())
    }
    
    /// Check if recovery should be attempted
    async fn should_recover(&self, component: &str, state: &RecoveryState) -> bool {
        // Don't recover if already recovering
        if state.recovering {
            debug!("Recovery already in progress for {}", component);
            return false;
        }
        
        // Check retry limit
        if state.retry_count >= self.config.max_reconnect_attempts {
            return false;
        }
        
        // Check if enough time has passed since last failure
        if let Some(last_failure) = state.last_failure {
            let elapsed = Instant::now().duration_since(last_failure);
            if elapsed < state.current_backoff {
                debug!("Backoff period not elapsed for {}", component);
                return false;
            }
        }
        
        true
    }
    
    /// Perform the actual recovery
    async fn perform_recovery(
        component: String,
        mut state: RecoveryState,
        config: RecoveryConfig,
        states: Arc<RwLock<HashMap<String, RecoveryState>>>,
        event_emitter: EventEmitter,
        recovery_action: Arc<dyn Fn() -> futures::future::BoxFuture<'static, SipClientResult<()>> + Send + Sync>,
    ) {
        info!("Starting recovery for {} (attempt {})", component, state.retry_count);
        
        // Wait for backoff period
        sleep(state.current_backoff).await;
        
        // Attempt recovery
        match recovery_action().await {
            Ok(()) => {
                info!("Recovery successful for {}", component);
                
                // Reset recovery state
                let mut states = states.write().await;
                states.insert(component.clone(), RecoveryState::default());
                
                // Emit recovery success event
                event_emitter.emit(SipClientEvent::RecoverySucceeded {
                    component: component.clone(),
                    attempts: state.retry_count,
                });
            }
            Err(e) => {
                warn!("Recovery attempt {} failed for {}: {}", state.retry_count, component, e);
                
                // Update backoff
                state.current_backoff = std::cmp::min(
                    Duration::from_secs_f64(
                        state.current_backoff.as_secs_f64() * config.backoff_multiplier
                    ),
                    config.max_reconnect_delay,
                );
                
                // Update state
                state.recovering = false;
                let mut states = states.write().await;
                states.insert(component.clone(), state.clone());
                
                // Emit recovery failure event
                event_emitter.emit(SipClientEvent::RecoveryFailed {
                    component: component.clone(),
                    error: e.to_string(),
                    attempts: state.retry_count,
                });
            }
        }
    }
    
    /// Stop all recovery attempts
    pub async fn stop_all_recovery(&self) {
        let mut tasks = self.recovery_tasks.lock().await;
        for (component, task) in tasks.drain() {
            debug!("Cancelling recovery for {}", component);
            task.abort();
        }
        
        // Clear all recovery states
        let mut states = self.states.write().await;
        states.clear();
    }
    
    /// Get recovery state for a component
    pub async fn get_recovery_state(&self, component: &str) -> Option<RecoveryState> {
        let states = self.states.read().await;
        states.get(component).cloned()
    }
}

/// Graceful degradation manager
pub struct DegradationManager {
    /// Current degradation level (0 = normal, higher = more degraded)
    degradation_level: Arc<RwLock<u8>>,
    
    /// Event emitter
    event_emitter: EventEmitter,
    
    /// Configuration
    config: RecoveryConfig,
}

impl DegradationManager {
    /// Create a new degradation manager
    pub fn new(config: RecoveryConfig, event_emitter: EventEmitter) -> Self {
        Self {
            degradation_level: Arc::new(RwLock::new(0)),
            event_emitter,
            config,
        }
    }
    
    /// Apply degradation based on current conditions
    pub async fn apply_degradation(&self, metrics: &NetworkMetrics) -> DegradationActions {
        if !self.config.enable_degradation {
            return DegradationActions::default();
        }
        
        let mut actions = DegradationActions::default();
        let mut level = self.degradation_level.write().await;
        
        // Determine new degradation level based on metrics
        let new_level = self.calculate_degradation_level(metrics);
        
        if new_level != *level {
            info!("Changing degradation level from {} to {}", *level, new_level);
            *level = new_level;
            
            // Apply degradation actions based on level
            match new_level {
                0 => {
                    // Normal operation
                    actions.codec_downgrade = false;
                    actions.reduce_quality = false;
                    actions.disable_video = false;
                }
                1 => {
                    // Mild degradation
                    actions.reduce_quality = true;
                    actions.target_bitrate = Some(32000); // 32 kbps
                }
                2 => {
                    // Moderate degradation
                    actions.codec_downgrade = true;
                    actions.reduce_quality = true;
                    actions.target_bitrate = Some(16000); // 16 kbps
                    actions.disable_enhancements = true;
                }
                3.. => {
                    // Severe degradation
                    actions.codec_downgrade = true;
                    actions.reduce_quality = true;
                    actions.target_bitrate = Some(8000); // 8 kbps
                    actions.disable_enhancements = true;
                    actions.disable_video = true;
                    actions.reduce_frame_rate = true;
                }
            }
            
            self.event_emitter.emit(SipClientEvent::DegradationApplied {
                level: new_level,
                actions: actions.clone(),
            });
        }
        
        actions
    }
    
    /// Calculate degradation level based on network metrics
    fn calculate_degradation_level(&self, metrics: &NetworkMetrics) -> u8 {
        let mut score = 0;
        
        // Packet loss thresholds
        if metrics.packet_loss_percent > 10.0 {
            score += 3;
        } else if metrics.packet_loss_percent > 5.0 {
            score += 2;
        } else if metrics.packet_loss_percent > 2.0 {
            score += 1;
        }
        
        // Jitter thresholds
        if metrics.jitter_ms > 100.0 {
            score += 2;
        } else if metrics.jitter_ms > 50.0 {
            score += 1;
        }
        
        // RTT thresholds
        if metrics.rtt_ms > 300.0 {
            score += 2;
        } else if metrics.rtt_ms > 150.0 {
            score += 1;
        }
        
        // Bandwidth thresholds
        if let Some(bandwidth) = metrics.available_bandwidth_bps {
            if bandwidth < 16000 {
                score += 3;
            } else if bandwidth < 32000 {
                score += 2;
            } else if bandwidth < 64000 {
                score += 1;
            }
        }
        
        // Map score to degradation level
        match score {
            0..=1 => 0,
            2..=3 => 1,
            4..=5 => 2,
            _ => 3,
        }
    }
}

/// Network metrics for degradation decisions
#[derive(Debug, Clone)]
pub struct NetworkMetrics {
    /// Packet loss percentage
    pub packet_loss_percent: f64,
    
    /// Jitter in milliseconds
    pub jitter_ms: f64,
    
    /// Round-trip time in milliseconds
    pub rtt_ms: f64,
    
    /// Available bandwidth in bits per second
    pub available_bandwidth_bps: Option<u64>,
    
    /// Number of consecutive errors
    pub consecutive_errors: u32,
}

/// Actions to take for graceful degradation
#[derive(Debug, Clone)]
pub struct DegradationActions {
    /// Downgrade to lower quality codec
    pub codec_downgrade: bool,
    
    /// Reduce audio quality settings
    pub reduce_quality: bool,
    
    /// Target bitrate if quality reduction is needed
    pub target_bitrate: Option<u32>,
    
    /// Disable audio enhancements (echo cancellation, noise suppression)
    pub disable_enhancements: bool,
    
    /// Disable video if applicable
    pub disable_video: bool,
    
    /// Reduce frame rate
    pub reduce_frame_rate: bool,
}

/// Connection monitor for health checking
pub struct ConnectionMonitor {
    /// Event emitter
    event_emitter: EventEmitter,
    
    /// Monitoring interval
    check_interval: Duration,
    
    /// Health check callback
    health_check: Arc<dyn Fn() -> futures::future::BoxFuture<'static, bool> + Send + Sync>,
}

impl ConnectionMonitor {
    /// Create a new connection monitor
    pub fn new(
        event_emitter: EventEmitter,
        check_interval: Duration,
        health_check: impl Fn() -> futures::future::BoxFuture<'static, bool> + Send + Sync + 'static,
    ) -> Self {
        Self {
            event_emitter,
            check_interval,
            health_check: Arc::new(health_check),
        }
    }
    
    /// Start monitoring connection health
    pub async fn start_monitoring(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = interval(self.check_interval);
            let mut consecutive_failures = 0;
            
            loop {
                interval.tick().await;
                
                let is_healthy = (self.health_check)().await;
                
                if is_healthy {
                    if consecutive_failures > 0 {
                        info!("Connection restored after {} failures", consecutive_failures);
                        self.event_emitter.emit(SipClientEvent::ConnectionRestored);
                    }
                    consecutive_failures = 0;
                } else {
                    consecutive_failures += 1;
                    warn!("Connection health check failed (count: {})", consecutive_failures);
                    
                    if consecutive_failures == 1 {
                        self.event_emitter.emit(SipClientEvent::ConnectionLost {
                            reason: "Health check failed".to_string(),
                        });
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_recovery_config_defaults() {
        let config = RecoveryConfig::default();
        assert!(config.auto_reconnect);
        assert_eq!(config.max_reconnect_attempts, 5);
        assert_eq!(config.initial_reconnect_delay, Duration::from_secs(1));
    }
    
    #[tokio::test]
    async fn test_degradation_level_calculation() {
        let config = RecoveryConfig::default();
        let emitter = EventEmitter::default();
        let manager = DegradationManager::new(config, emitter);
        
        // Test with good metrics
        let good_metrics = NetworkMetrics {
            packet_loss_percent: 0.5,
            jitter_ms: 20.0,
            rtt_ms: 50.0,
            available_bandwidth_bps: Some(128000),
            consecutive_errors: 0,
        };
        
        let level = manager.calculate_degradation_level(&good_metrics);
        assert_eq!(level, 0);
        
        // Test with poor metrics
        let poor_metrics = NetworkMetrics {
            packet_loss_percent: 15.0,
            jitter_ms: 150.0,
            rtt_ms: 400.0,
            available_bandwidth_bps: Some(8000),
            consecutive_errors: 5,
        };
        
        let level = manager.calculate_degradation_level(&poor_metrics);
        assert_eq!(level, 3);
    }
    
    #[tokio::test]
    async fn test_recovery_state_updates() {
        let config = RecoveryConfig::default();
        let emitter = EventEmitter::default();
        let manager = RecoveryManager::new(config, emitter);
        
        // First error - should trigger recovery
        let result = manager.handle_error(
            "test_component",
            &SipClientError::Network { message: "Test error".to_string() },
            || Box::pin(async { Ok(()) }),
        ).await;
        
        // handle_error returns Ok(()) when recovery is initiated
        assert!(result.is_ok());
        
        // Check recovery state immediately - it should be updated synchronously
        let state = manager.get_recovery_state("test_component").await;
        assert!(state.is_some());
        
        let state = state.unwrap();
        assert_eq!(state.retry_count, 1);
        assert!(state.recovering);
        
        // Second error while recovering - should fail
        let result2 = manager.handle_error(
            "test_component",
            &SipClientError::Network { message: "Test error 2".to_string() },
            || Box::pin(async { Ok(()) }),
        ).await;
        
        // Should fail because recovery is already in progress
        assert!(result2.is_err());
        
        // Test max retries
        let mut config2 = RecoveryConfig::default();
        config2.max_reconnect_attempts = 1;
        let manager2 = RecoveryManager::new(config2, EventEmitter::default());
        
        // First attempt
        let _ = manager2.handle_error(
            "test_component2",
            &SipClientError::Network { message: "Test error".to_string() },
            || Box::pin(async { Err(SipClientError::Network { message: "Still failing".to_string() }) }),
        ).await;
        
        // Wait for recovery to fail
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        
        // Second attempt should fail because max retries reached
        let final_result = manager2.handle_error(
            "test_component2",
            &SipClientError::Network { message: "Test error 3".to_string() },
            || Box::pin(async { Ok(()) }),
        ).await;
        
        assert!(final_result.is_err());
    }
}