//! Configuration system for the Global Event Coordinator
//!
//! Supports both monolithic (single process) and distributed (multi-process)
//! deployment configurations.

use std::collections::HashMap;
use serde::{Serialize, Deserialize};

/// Main configuration for the event coordinator
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventCoordinatorConfig {
    /// The deployment mode configuration
    pub deployment: DeploymentConfig,
    /// Name of this service (e.g., "session-core", "media-core")
    pub service_name: String,
}

impl Default for EventCoordinatorConfig {
    fn default() -> Self {
        Self {
            deployment: DeploymentConfig::Monolithic,
            service_name: "rvoip-monolithic".to_string(),
        }
    }
}

/// Deployment configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DeploymentConfig {
    /// Single process deployment - all components in one binary
    Monolithic,
    
    /// Multi-process deployment - components communicate over network
    #[serde(rename = "distributed")]
    Distributed {
        /// Network transport configuration
        transport: TransportConfig,
        /// Service discovery configuration
        discovery: ServiceDiscoveryConfig,
    },
}

/// Network transport configuration for distributed deployments
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum TransportConfig {
    /// NATS messaging system
    Nats {
        /// NATS server URLs
        servers: Vec<String>,
        /// Optional cluster name
        cluster: Option<String>,
    },
    
    /// gRPC transport
    Grpc {
        /// Listen endpoint for this service
        endpoint: String,
        /// TLS configuration
        tls: Option<TlsConfig>,
    },
    
    /// Redis pub/sub (future)
    Redis {
        /// Redis connection URL
        url: String,
    },
}

/// TLS configuration for secure transports
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsConfig {
    /// Path to certificate file
    pub cert_path: String,
    /// Path to key file
    pub key_path: String,
    /// Path to CA certificate
    pub ca_path: Option<String>,
}

/// Service discovery configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ServiceDiscoveryConfig {
    /// Static endpoint configuration
    Static {
        /// Map of service name to endpoint
        endpoints: HashMap<String, String>,
    },
    
    /// Consul service discovery (future)
    Consul {
        /// Consul endpoint
        endpoint: String,
        /// Service prefix
        service_prefix: Option<String>,
    },
    
    /// Kubernetes service discovery (future)
    Kubernetes {
        /// Namespace to search
        namespace: String,
        /// Label selector
        label_selector: Option<String>,
    },
}

impl EventCoordinatorConfig {
    /// Create a new monolithic configuration
    pub fn monolithic() -> Self {
        Self::default()
    }
    
    /// Create a new distributed configuration with NATS
    pub fn distributed_nats(
        service_name: impl Into<String>,
        servers: Vec<String>,
        endpoints: HashMap<String, String>,
    ) -> Self {
        Self {
            deployment: DeploymentConfig::Distributed {
                transport: TransportConfig::Nats {
                    servers,
                    cluster: None,
                },
                discovery: ServiceDiscoveryConfig::Static { endpoints },
            },
            service_name: service_name.into(),
        }
    }
    
    /// Load configuration from environment variables
    ///
    /// Supported variables:
    /// - `RVOIP_DISTRIBUTED`: Set to enable distributed mode
    /// - `RVOIP_NATS_SERVERS`: Comma-separated NATS server URLs
    /// - `RVOIP_SERVICE_NAME`: Service name for this instance
    /// - `RVOIP_SERVICE_ENDPOINTS`: JSON map of service name to endpoint
    pub fn from_env() -> Result<Self, ConfigError> {
        // Check if distributed mode is enabled
        if std::env::var("RVOIP_DISTRIBUTED").is_ok() {
            let service_name = std::env::var("RVOIP_SERVICE_NAME")
                .unwrap_or_else(|_| "rvoip-distributed".to_string());

            let nats_servers: Vec<String> = std::env::var("RVOIP_NATS_SERVERS")
                .unwrap_or_else(|_| "nats://localhost:4222".to_string())
                .split(',')
                .map(|s| s.trim().to_string())
                .collect();

            let endpoints: std::collections::HashMap<String, String> =
                std::env::var("RVOIP_SERVICE_ENDPOINTS")
                    .ok()
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default();

            return Ok(Self::distributed_nats(service_name, nats_servers, endpoints));
        }

        // Default to monolithic
        Ok(Self::monolithic())
    }

    /// Load configuration from a file
    ///
    /// Currently supports JSON configuration files. The file must contain
    /// a valid `EventCoordinatorConfig` serialized as JSON.
    pub fn from_file(path: &str) -> Result<Self, ConfigError> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| ConfigError::Io(e.to_string()))?;

        if path.ends_with(".json") {
            serde_json::from_str(&contents)
                .map_err(|e| ConfigError::Parse(e.to_string()))
        } else {
            // For non-JSON files, attempt JSON parsing as a fallback since
            // the content format may not match the extension
            serde_json::from_str(&contents)
                .map_err(|e| ConfigError::Parse(
                    format!("Failed to parse configuration (only JSON is currently supported): {}", e)
                ))
        }
    }
}

/// Configuration errors
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(String),
    
    #[error("Parse error: {0}")]
    Parse(String),
    
    #[error("Not implemented: {0}")]
    NotImplemented(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_default_config() {
        let config = EventCoordinatorConfig::default();
        assert!(matches!(config.deployment, DeploymentConfig::Monolithic));
        assert_eq!(config.service_name, "rvoip-monolithic");
    }
    
    #[test]
    fn test_distributed_nats_config() {
        let endpoints = HashMap::from([
            ("media-core".to_string(), "grpc://media:50051".to_string()),
            ("dialog-core".to_string(), "grpc://dialog:50052".to_string()),
        ]);
        
        let config = EventCoordinatorConfig::distributed_nats(
            "session-core",
            vec!["nats://localhost:4222".to_string()],
            endpoints.clone(),
        );
        
        assert_eq!(config.service_name, "session-core");
        match config.deployment {
            DeploymentConfig::Distributed { transport, discovery } => {
                assert!(matches!(transport, TransportConfig::Nats { .. }));
                assert!(matches!(discovery, ServiceDiscoveryConfig::Static { .. }));
            }
            _ => panic!("Expected distributed config"),
        }
    }
}
