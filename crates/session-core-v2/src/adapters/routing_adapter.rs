//! Routing adapter for B2BUA call interception
//!
//! This module provides routing decisions for incoming calls, determining
//! whether they should be handled as B2BUA calls or passed through directly.

use std::sync::Arc;
use tokio::sync::RwLock;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use crate::errors::{Result, SessionError};

/// Routing decision for incoming calls
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RoutingDecision {
    /// Handle as B2BUA call with specified target
    B2bua {
        target: String,
        media_mode: MediaMode,
    },

    /// Pass through directly to endpoint
    Direct {
        endpoint: String
    },

    /// Reject the call with reason
    Reject {
        reason: String,
        status_code: u16,
    },

    /// Queue for later processing
    Queue {
        priority: u8,
        queue_name: String,
    },

    /// Load balance across multiple targets
    LoadBalance {
        targets: Vec<String>,
        algorithm: LoadBalanceAlgorithm,
    },
}

/// Media handling mode for B2BUA
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum MediaMode {
    /// Just forward RTP packets (lowest latency, lowest CPU)
    Relay,

    /// Full decode/encode (needed for recording, transcoding, etc.)
    FullProcessing,
}

/// Load balancing algorithm
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LoadBalanceAlgorithm {
    RoundRobin,
    Random,
    LeastConnections,
    WeightedRoundRobin { weights: Vec<u32> },
}

/// Routing rule configuration
#[derive(Debug, Clone)]
pub struct RoutingRule {
    /// Unique rule ID
    pub id: String,

    /// Pattern to match (supports wildcards)
    pub pattern: String,

    /// Match type
    pub match_type: MatchType,

    /// Decision to apply when rule matches
    pub decision: RoutingDecision,

    /// Priority (higher = evaluated first)
    pub priority: i32,

    /// Whether rule is enabled
    pub enabled: bool,
}

/// What to match against
#[derive(Clone)]
pub enum MatchType {
    /// Match against From URI
    From,
    /// Match against To URI
    To,
    /// Match against both From and To
    Both { from: String, to: String },
    /// Custom match function
    Custom(Arc<dyn Fn(&str, &str) -> bool + Send + Sync>),
}

impl std::fmt::Debug for MatchType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MatchType::From => write!(f, "From"),
            MatchType::To => write!(f, "To"),
            MatchType::Both { from, to } => write!(f, "Both {{ from: {}, to: {} }}", from, to),
            MatchType::Custom(_) => write!(f, "Custom(function)"),
        }
    }
}

/// Failover configuration for backends
#[derive(Debug, Clone)]
pub struct FailoverConfig {
    /// Primary target
    pub primary: String,

    /// Backup targets in order of preference
    pub backups: Vec<String>,

    /// Health check configuration
    pub health_check: HealthCheckConfig,
}

/// Health check configuration
#[derive(Debug, Clone)]
pub struct HealthCheckConfig {
    /// Interval between health checks
    pub interval_secs: u64,

    /// Timeout for health check response
    pub timeout_secs: u64,

    /// Number of failures before marking backend down
    pub failure_threshold: u32,

    /// Number of successes before marking backend up
    pub success_threshold: u32,
}

/// Backend health status
#[derive(Debug, Clone)]
pub struct BackendHealth {
    /// Backend URI
    pub uri: String,

    /// Current state
    pub state: BackendState,

    /// Consecutive failures
    pub failure_count: u32,

    /// Consecutive successes
    pub success_count: u32,

    /// Last check time
    pub last_check: std::time::Instant,

    /// Circuit breaker threshold
    pub failure_threshold: u32,
}

/// Backend state for circuit breaker
#[derive(Debug, Clone, Copy)]
pub enum BackendState {
    /// Backend is healthy
    Healthy,

    /// Backend is temporarily down (circuit open)
    Down { since: std::time::Instant },

    /// Testing if backend recovered (circuit half-open)
    Testing,
}

/// Routing adapter that intercepts calls before session creation
pub struct RoutingAdapter {
    /// Routing rules
    rules: Arc<RwLock<Vec<RoutingRule>>>,

    /// Default decision if no rules match
    default_decision: Arc<RwLock<RoutingDecision>>,

    /// Backend health tracking
    backend_health: Arc<DashMap<String, BackendHealth>>,

    /// Failover configurations
    failover_configs: Arc<DashMap<String, FailoverConfig>>,

    /// Round-robin counters for load balancing
    round_robin_counters: Arc<DashMap<String, usize>>,
}

impl RoutingAdapter {
    /// Create a new routing adapter
    pub fn new() -> Self {
        Self {
            rules: Arc::new(RwLock::new(Vec::new())),
            default_decision: Arc::new(RwLock::new(
                RoutingDecision::Reject {
                    reason: "No route found".to_string(),
                    status_code: 404,
                }
            )),
            backend_health: Arc::new(DashMap::new()),
            failover_configs: Arc::new(DashMap::new()),
            round_robin_counters: Arc::new(DashMap::new()),
        }
    }

    /// Add a routing rule
    pub async fn add_rule(&self, rule: RoutingRule) -> Result<()> {
        let mut rules = self.rules.write().await;

        // Check for duplicate ID
        if rules.iter().any(|r| r.id == rule.id) {
            return Err(SessionError::ConfigurationError(
                format!("Rule with ID '{}' already exists", rule.id)
            ));
        }

        rules.push(rule);
        rules.sort_by_key(|r| -r.priority); // Sort by priority descending
        Ok(())
    }

    /// Remove a routing rule by ID
    pub async fn remove_rule(&self, rule_id: &str) -> Result<()> {
        let mut rules = self.rules.write().await;
        let original_len = rules.len();
        rules.retain(|r| r.id != rule_id);

        if rules.len() == original_len {
            Err(SessionError::SessionNotFound(format!("Rule '{}' not found", rule_id)))
        } else {
            Ok(())
        }
    }

    /// Update a routing rule
    pub async fn update_rule(&self, rule: RoutingRule) -> Result<()> {
        let mut rules = self.rules.write().await;

        if let Some(existing) = rules.iter_mut().find(|r| r.id == rule.id) {
            *existing = rule;
            rules.sort_by_key(|r| -r.priority);
            Ok(())
        } else {
            Err(SessionError::SessionNotFound(format!("Rule '{}' not found", rule.id)))
        }
    }

    /// Set default routing decision
    pub async fn set_default_decision(&self, decision: RoutingDecision) {
        *self.default_decision.write().await = decision;
    }

    /// Process incoming INVITE to determine routing
    pub async fn route_invite(
        &self,
        from: &str,
        to: &str,
        call_id: &str,
    ) -> Result<RoutingDecision> {
        tracing::debug!("Routing INVITE - From: {}, To: {}, Call-ID: {}", from, to, call_id);

        // Check rules in priority order
        let rules = self.rules.read().await;

        for rule in rules.iter() {
            if !rule.enabled {
                continue;
            }

            if self.matches_rule(rule, from, to) {
                tracing::info!("Rule '{}' matched for Call-ID: {}", rule.id, call_id);

                // Handle load balancing and failover
                let decision = match &rule.decision {
                    RoutingDecision::LoadBalance { targets, algorithm } => {
                        self.apply_load_balancing(targets, algorithm, &rule.id).await
                    }
                    RoutingDecision::B2bua { target, media_mode } => {
                        // Check if target is healthy
                        if let Some(healthy_target) = self.get_healthy_target(target).await {
                            RoutingDecision::B2bua {
                                target: healthy_target,
                                media_mode: *media_mode,
                            }
                        } else {
                            // Try failover if configured
                            if let Some(failover) = self.failover_configs.get(target) {
                                if let Some(backup) = self.get_first_healthy_backup(&failover.backups).await {
                                    RoutingDecision::B2bua {
                                        target: backup,
                                        media_mode: *media_mode,
                                    }
                                } else {
                                    RoutingDecision::Reject {
                                        reason: "All backends unavailable".to_string(),
                                        status_code: 503,
                                    }
                                }
                            } else {
                                RoutingDecision::Reject {
                                    reason: format!("Backend {} unavailable", target),
                                    status_code: 503,
                                }
                            }
                        }
                    }
                    other => other.clone(),
                };

                return Ok(decision);
            }
        }

        // No rules matched, return default decision
        Ok(self.default_decision.read().await.clone())
    }

    /// Check if a rule matches the given from/to URIs
    fn matches_rule(&self, rule: &RoutingRule, from: &str, to: &str) -> bool {
        match &rule.match_type {
            MatchType::From => self.matches_pattern(&rule.pattern, from),
            MatchType::To => self.matches_pattern(&rule.pattern, to),
            MatchType::Both { from: from_pattern, to: to_pattern } => {
                self.matches_pattern(from_pattern, from) &&
                self.matches_pattern(to_pattern, to)
            }
            MatchType::Custom(matcher) => matcher(from, to),
        }
    }

    /// Simple wildcard pattern matching
    fn matches_pattern(&self, pattern: &str, uri: &str) -> bool {
        // Handle exact match
        if pattern == uri {
            return true;
        }

        // Handle wildcard patterns
        if pattern.contains('*') {
            // Convert pattern to regex-like matching
            let parts: Vec<&str> = pattern.split('*').collect();

            if parts.is_empty() {
                return true; // Pattern is just "*"
            }

            let mut current_pos = 0;
            for (i, part) in parts.iter().enumerate() {
                if part.is_empty() {
                    continue;
                }

                // Check if this part exists in the URI at or after current position
                if let Some(found_pos) = uri[current_pos..].find(part) {
                    // If this is the first part and pattern doesn't start with *,
                    // it must be at the beginning
                    if i == 0 && !pattern.starts_with('*') && found_pos != 0 {
                        return false;
                    }
                    current_pos += found_pos + part.len();
                } else {
                    return false;
                }
            }

            // If pattern doesn't end with *, check that we've consumed the entire URI
            if !pattern.ends_with('*') && current_pos != uri.len() {
                return false;
            }

            true
        } else {
            false
        }
    }

    /// Apply load balancing algorithm to select target
    async fn apply_load_balancing(
        &self,
        targets: &[String],
        algorithm: &LoadBalanceAlgorithm,
        rule_id: &str,
    ) -> RoutingDecision {
        if targets.is_empty() {
            return RoutingDecision::Reject {
                reason: "No targets configured".to_string(),
                status_code: 503,
            };
        }

        // Filter healthy targets
        let mut healthy_targets = Vec::new();
        for target in targets {
            if self.is_backend_healthy(target).await {
                healthy_targets.push(target.clone());
            }
        }

        if healthy_targets.is_empty() {
            return RoutingDecision::Reject {
                reason: "No healthy backends available".to_string(),
                status_code: 503,
            };
        }

        let selected = match algorithm {
            LoadBalanceAlgorithm::RoundRobin => {
                let counter_key = format!("rr_{}", rule_id);
                let mut counter = self.round_robin_counters.entry(counter_key).or_insert(0);
                let idx = *counter % healthy_targets.len();
                *counter = counter.wrapping_add(1);
                healthy_targets[idx].clone()
            }
            LoadBalanceAlgorithm::Random => {
                use rand::Rng;
                let idx = rand::thread_rng().gen_range(0..healthy_targets.len());
                healthy_targets[idx].clone()
            }
            LoadBalanceAlgorithm::LeastConnections => {
                // For now, just use round-robin
                // TODO: Implement connection tracking
                healthy_targets[0].clone()
            }
            LoadBalanceAlgorithm::WeightedRoundRobin { weights: _ } => {
                // Simple weighted selection
                // TODO: Implement proper weighted round-robin
                healthy_targets[0].clone()
            }
        };

        RoutingDecision::B2bua {
            target: selected,
            media_mode: MediaMode::Relay, // Default to relay for load balanced calls
        }
    }

    /// Check if a backend is healthy
    async fn is_backend_healthy(&self, uri: &str) -> bool {
        if let Some(health) = self.backend_health.get(uri) {
            matches!(health.state, BackendState::Healthy | BackendState::Testing)
        } else {
            // No health info means we assume it's healthy (optimistic)
            true
        }
    }

    /// Get a healthy target, checking health status
    async fn get_healthy_target(&self, target: &str) -> Option<String> {
        if self.is_backend_healthy(target).await {
            Some(target.to_string())
        } else {
            None
        }
    }

    /// Get first healthy backup from list
    async fn get_first_healthy_backup(&self, backups: &[String]) -> Option<String> {
        for backup in backups {
            if self.is_backend_healthy(backup).await {
                return Some(backup.clone());
            }
        }
        None
    }

    /// Mark a backend as failed
    pub async fn mark_backend_failed(&self, uri: &str) {
        let mut health = self.backend_health.entry(uri.to_string()).or_insert_with(|| {
            BackendHealth {
                uri: uri.to_string(),
                state: BackendState::Healthy,
                failure_count: 0,
                success_count: 0,
                last_check: std::time::Instant::now(),
                failure_threshold: 3, // Default threshold
            }
        });

        health.failure_count += 1;
        health.success_count = 0;
        health.last_check = std::time::Instant::now();

        if health.failure_count >= health.failure_threshold {
            health.state = BackendState::Down {
                since: std::time::Instant::now()
            };
            tracing::warn!("Backend {} marked as DOWN after {} failures", uri, health.failure_count);
        }
    }

    /// Mark a backend as successful
    pub async fn mark_backend_success(&self, uri: &str) {
        let mut health = self.backend_health.entry(uri.to_string()).or_insert_with(|| {
            BackendHealth {
                uri: uri.to_string(),
                state: BackendState::Healthy,
                failure_count: 0,
                success_count: 0,
                last_check: std::time::Instant::now(),
                failure_threshold: 3,
            }
        });

        health.success_count += 1;
        health.failure_count = 0;
        health.last_check = std::time::Instant::now();

        // Recover from down state
        if matches!(health.state, BackendState::Down { .. }) {
            if health.success_count >= 2 {
                health.state = BackendState::Healthy;
                tracing::info!("Backend {} recovered and marked as HEALTHY", uri);
            } else {
                health.state = BackendState::Testing;
            }
        }
    }

    /// Configure failover for a backend
    pub async fn configure_failover(&self, primary: String, config: FailoverConfig) {
        self.failover_configs.insert(primary, config);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wildcard_pattern_matching() {
        let adapter = RoutingAdapter::new();

        // Test exact match
        assert!(adapter.matches_pattern("sip:alice@example.com", "sip:alice@example.com"));
        assert!(!adapter.matches_pattern("sip:alice@example.com", "sip:bob@example.com"));

        // Test wildcard at end
        assert!(adapter.matches_pattern("sip:alice@*", "sip:alice@example.com"));
        assert!(adapter.matches_pattern("sip:alice@*", "sip:alice@test.org"));
        assert!(!adapter.matches_pattern("sip:alice@*", "sip:bob@example.com"));

        // Test wildcard at beginning
        assert!(adapter.matches_pattern("*@example.com", "sip:alice@example.com"));
        assert!(adapter.matches_pattern("*@example.com", "sip:bob@example.com"));
        assert!(!adapter.matches_pattern("*@example.com", "sip:alice@test.org"));

        // Test wildcard in middle
        assert!(adapter.matches_pattern("sip:*@example.com", "sip:alice@example.com"));
        assert!(adapter.matches_pattern("sip:*@example.com", "sip:bob@example.com"));

        // Test multiple wildcards
        assert!(adapter.matches_pattern("sip:*@*.com", "sip:alice@example.com"));
        assert!(adapter.matches_pattern("sip:*@*.com", "sip:bob@test.com"));
        assert!(!adapter.matches_pattern("sip:*@*.com", "sip:alice@example.org"));
    }

    #[tokio::test]
    async fn test_routing_priority() {
        let adapter = RoutingAdapter::new();

        // Add rules with different priorities
        adapter.add_rule(RoutingRule {
            id: "rule1".to_string(),
            pattern: "sip:alice@*".to_string(),
            match_type: MatchType::From,
            decision: RoutingDecision::B2bua {
                target: "sip:backend1@server.com".to_string(),
                media_mode: MediaMode::Relay,
            },
            priority: 10,
            enabled: true,
        }).await.unwrap();

        adapter.add_rule(RoutingRule {
            id: "rule2".to_string(),
            pattern: "*".to_string(),
            match_type: MatchType::From,
            decision: RoutingDecision::B2bua {
                target: "sip:backend2@server.com".to_string(),
                media_mode: MediaMode::Relay,
            },
            priority: 5,
            enabled: true,
        }).await.unwrap();

        // Test that higher priority rule matches first
        let decision = adapter.route_invite(
            "sip:alice@client.com",
            "sip:service@server.com",
            "call-123"
        ).await.unwrap();

        match decision {
            RoutingDecision::B2bua { target, .. } => {
                assert_eq!(target, "sip:backend1@server.com");
            }
            _ => panic!("Expected B2bua decision"),
        }
    }

    #[tokio::test]
    async fn test_backend_health_tracking() {
        let adapter = RoutingAdapter::new();

        // Mark backend as failed multiple times
        adapter.mark_backend_failed("sip:backend@server.com").await;
        adapter.mark_backend_failed("sip:backend@server.com").await;
        adapter.mark_backend_failed("sip:backend@server.com").await;

        // Should be unhealthy after 3 failures
        assert!(!adapter.is_backend_healthy("sip:backend@server.com").await);

        // Mark as successful
        adapter.mark_backend_success("sip:backend@server.com").await;
        adapter.mark_backend_success("sip:backend@server.com").await;

        // Should be healthy again
        assert!(adapter.is_backend_healthy("sip:backend@server.com").await);
    }
}