
use crate::lifecycle::component::Component;
use std::collections::HashMap;
use std::fmt::{self, Display};
use std::time::{Duration, Instant};
use async_trait::async_trait;

/// Status of a health check
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    /// Component is healthy
    Healthy,
    /// Component is degraded but functional
    Degraded,
    /// Component is unhealthy
    Unhealthy,
    /// Component health is unknown
    Unknown,
}

impl Display for HealthStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HealthStatus::Healthy => write!(f, "healthy"),
            HealthStatus::Degraded => write!(f, "degraded"),
            HealthStatus::Unhealthy => write!(f, "unhealthy"),
            HealthStatus::Unknown => write!(f, "unknown"),
        }
    }
}

/// Health check details
#[derive(Debug, Clone)]
pub struct HealthCheck {
    /// Component name
    pub component: String,
    /// Health status
    pub status: HealthStatus,
    /// Detailed message about the health status
    pub message: Option<String>,
    /// Time when the check was performed
    pub timestamp: Instant,
    /// Duration the check took to complete
    pub duration: Duration,
}

impl HealthCheck {
    /// Create a new healthy health check
    pub fn healthy(component: &str) -> Self {
        HealthCheck {
            component: component.to_string(),
            status: HealthStatus::Healthy,
            message: None,
            timestamp: Instant::now(),
            duration: Duration::from_secs(0),
        }
    }
    
    /// Create a new degraded health check
    pub fn degraded(component: &str, message: &str) -> Self {
        HealthCheck {
            component: component.to_string(),
            status: HealthStatus::Degraded,
            message: Some(message.to_string()),
            timestamp: Instant::now(),
            duration: Duration::from_secs(0),
        }
    }
    
    /// Create a new unhealthy health check
    pub fn unhealthy(component: &str, message: &str) -> Self {
        HealthCheck {
            component: component.to_string(),
            status: HealthStatus::Unhealthy,
            message: Some(message.to_string()),
            timestamp: Instant::now(),
            duration: Duration::from_secs(0),
        }
    }
    
    /// Create a new unknown health check
    pub fn unknown(component: &str) -> Self {
        HealthCheck {
            component: component.to_string(),
            status: HealthStatus::Unknown,
            message: None,
            timestamp: Instant::now(),
            duration: Duration::from_secs(0),
        }
    }
    
    /// Set the duration of the health check
    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }
    
    /// Add a message to the health check
    pub fn with_message(mut self, message: &str) -> Self {
        self.message = Some(message.to_string());
        self
    }
}

/// Trait for components that can perform health checks
#[async_trait]
pub trait HealthChecker: Send + Sync {
    /// Check the health of this component
    async fn check_health(&self) -> HealthCheck;
}

/// Manages health checks for a set of components
#[derive(Debug, Default)]
pub struct HealthCheckManager {
    /// Last health check result for each component
    results: HashMap<String, HealthCheck>,
}

impl HealthCheckManager {
    /// Create a new health check manager
    pub fn new() -> Self {
        HealthCheckManager {
            results: HashMap::new(),
        }
    }
    
    /// Perform a health check on a component
    pub async fn check_component<C: Component + ?Sized>(&mut self, component: &C) -> HealthCheck {
        let component_name = component.name();
        let start = Instant::now();
        
        let check = match component.health_check().await {
            Ok(()) => HealthCheck::healthy(component_name),
            Err(e) => HealthCheck::unhealthy(component_name, &e.to_string()),
        };
        
        let duration = start.elapsed();
        let result = check.with_duration(duration);
        
        self.results.insert(component_name.to_string(), result.clone());
        result
    }
    
    /// Get the latest health check result for a component
    pub fn get_result(&self, component: &str) -> Option<&HealthCheck> {
        self.results.get(component)
    }
    
    /// Get all health check results
    pub fn get_all_results(&self) -> &HashMap<String, HealthCheck> {
        &self.results
    }
    
    /// Check if all components are healthy
    pub fn is_system_healthy(&self) -> bool {
        self.results.values().all(|check| check.status == HealthStatus::Healthy)
    }
} 