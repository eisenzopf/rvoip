//! Deployment configuration for federated planes
//!
//! Supports flexible deployment modes from monolithic to fully distributed

use serde::{Deserialize, Serialize};
use std::time::Duration;
use std::collections::HashMap;

/// Overall deployment mode for the RVOIP system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DeploymentMode {
    /// All planes run in the same process (P2P clients)
    Monolithic,
    
    /// Transport plane is remote (edge deployment)
    TransportDistributed,
    
    /// Media plane is remote (cloud media processing)
    MediaDistributed,
    
    /// Signaling plane is remote (centralized control)
    SignalingDistributed,
    
    /// All planes run as separate services (microservices)
    FullyDistributed,
    
    /// Custom deployment configuration
    Custom(CustomDeployment),
}

impl DeploymentMode {
    /// Check if this is a monolithic deployment
    pub fn is_monolithic(&self) -> bool {
        matches!(self, DeploymentMode::Monolithic)
    }
    
    /// Check if any plane is distributed
    pub fn is_distributed(&self) -> bool {
        !self.is_monolithic()
    }
}

/// Custom deployment configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomDeployment {
    pub transport: PlaneConfig,
    pub media: PlaneConfig,
    pub signaling: PlaneConfig,
}

/// Configuration for individual plane deployment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PlaneConfig {
    /// Plane runs in the local process
    Local,
    
    /// Plane runs as a remote service
    Remote {
        /// Service endpoints for the plane
        endpoints: Vec<String>,
        /// Failover strategy for multiple endpoints
        failover_strategy: FailoverStrategy,
        /// Health check interval
        health_check_interval: Duration,
    },
    
    /// Hybrid deployment with both local and remote components
    Hybrid {
        /// Weight for local processing (0-100)
        local_weight: u32,
        /// Remote service endpoints
        remote_endpoints: Vec<String>,
        /// Load distribution strategy
        load_strategy: LoadStrategy,
        /// Fallback to local if remote fails
        fallback_to_local: bool,
    },
}

impl PlaneConfig {
    /// Check if this plane is local
    pub fn is_local(&self) -> bool {
        matches!(self, PlaneConfig::Local)
    }
    
    /// Check if this plane has remote components
    pub fn has_remote(&self) -> bool {
        !self.is_local()
    }
    
    /// Get remote endpoints if any
    pub fn endpoints(&self) -> Vec<String> {
        match self {
            PlaneConfig::Local => vec![],
            PlaneConfig::Remote { endpoints, .. } => endpoints.clone(),
            PlaneConfig::Hybrid { remote_endpoints, .. } => remote_endpoints.clone(),
        }
    }
}

/// Failover strategy for distributed planes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FailoverStrategy {
    /// Try endpoints in order
    Sequential,
    /// Round-robin between endpoints
    RoundRobin,
    /// Choose endpoint with lowest latency
    LowestLatency,
    /// Choose endpoint with least load
    LeastLoad,
    /// Weighted random selection
    WeightedRandom(HashMap<String, u32>),
}

/// Load distribution strategy for hybrid deployments
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LoadStrategy {
    /// Process locally up to weight percentage
    LocalFirst,
    /// Process remotely up to weight percentage
    RemoteFirst,
    /// Distribute based on resource availability
    ResourceBased,
    /// Route based on session affinity
    SessionAffinity,
}

/// Complete deployment configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentConfig {
    /// Overall deployment mode
    pub deployment_mode: DeploymentMode,
    
    /// Individual plane configurations
    pub plane_configs: PlaneConfigs,
    
    /// Network configuration for distributed deployment
    pub networking: NetworkConfig,
    
    /// Service discovery configuration
    pub discovery: ServiceDiscoveryConfig,
    
    /// Performance tuning parameters
    pub performance: PerformanceConfig,
}

impl DeploymentConfig {
    /// Create default monolithic configuration for P2P
    pub fn monolithic() -> Self {
        Self {
            deployment_mode: DeploymentMode::Monolithic,
            plane_configs: PlaneConfigs::all_local(),
            networking: NetworkConfig::disabled(),
            discovery: ServiceDiscoveryConfig::none(),
            performance: PerformanceConfig::low_resource(),
        }
    }
    
    /// Create fully distributed configuration
    pub fn fully_distributed() -> Self {
        Self {
            deployment_mode: DeploymentMode::FullyDistributed,
            plane_configs: PlaneConfigs::all_remote_default(),
            networking: NetworkConfig::multi_protocol(),
            discovery: ServiceDiscoveryConfig::kubernetes(),
            performance: PerformanceConfig::high_throughput(),
        }
    }
    
    /// Create edge deployment (local signaling/media, remote transport)
    pub fn edge_deployment() -> Self {
        Self {
            deployment_mode: DeploymentMode::TransportDistributed,
            plane_configs: PlaneConfigs {
                transport: PlaneConfig::Remote {
                    endpoints: vec!["transport.example.com:8080".to_string()],
                    failover_strategy: FailoverStrategy::LowestLatency,
                    health_check_interval: Duration::from_secs(30),
                },
                media: PlaneConfig::Local,
                signaling: PlaneConfig::Local,
            },
            networking: NetworkConfig::single_protocol("grpc"),
            discovery: ServiceDiscoveryConfig::static_config(),
            performance: PerformanceConfig::balanced(),
        }
    }
}

/// Individual plane configurations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaneConfigs {
    pub transport: PlaneConfig,
    pub media: PlaneConfig,
    pub signaling: PlaneConfig,
}

impl PlaneConfigs {
    /// All planes local
    pub fn all_local() -> Self {
        Self {
            transport: PlaneConfig::Local,
            media: PlaneConfig::Local,
            signaling: PlaneConfig::Local,
        }
    }
    
    /// All planes remote with default configuration
    pub fn all_remote_default() -> Self {
        Self {
            transport: PlaneConfig::Remote {
                endpoints: vec![],
                failover_strategy: FailoverStrategy::RoundRobin,
                health_check_interval: Duration::from_secs(30),
            },
            media: PlaneConfig::Remote {
                endpoints: vec![],
                failover_strategy: FailoverStrategy::LeastLoad,
                health_check_interval: Duration::from_secs(30),
            },
            signaling: PlaneConfig::Remote {
                endpoints: vec![],
                failover_strategy: FailoverStrategy::Sequential,
                health_check_interval: Duration::from_secs(30),
            },
        }
    }
}

/// Network configuration for distributed deployment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// Enable networking
    pub enabled: bool,
    
    /// Primary protocol for inter-plane communication
    pub primary_protocol: String,
    
    /// Backup protocols
    pub backup_protocols: Vec<String>,
    
    /// Enable TLS/mTLS
    pub use_tls: bool,
    
    /// Connection pool size
    pub connection_pool_size: usize,
    
    /// Request timeout
    pub request_timeout: Duration,
    
    /// Enable compression
    pub compression: CompressionConfig,
}

impl NetworkConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            primary_protocol: String::new(),
            backup_protocols: vec![],
            use_tls: false,
            connection_pool_size: 0,
            request_timeout: Duration::from_secs(30),
            compression: CompressionConfig::None,
        }
    }
    
    pub fn multi_protocol() -> Self {
        Self {
            enabled: true,
            primary_protocol: "grpc".to_string(),
            backup_protocols: vec!["http2".to_string(), "websocket".to_string()],
            use_tls: true,
            connection_pool_size: 100,
            request_timeout: Duration::from_secs(30),
            compression: CompressionConfig::Auto,
        }
    }
    
    pub fn single_protocol(protocol: &str) -> Self {
        Self {
            enabled: true,
            primary_protocol: protocol.to_string(),
            backup_protocols: vec![],
            use_tls: true,
            connection_pool_size: 50,
            request_timeout: Duration::from_secs(30),
            compression: CompressionConfig::Auto,
        }
    }
}

/// Compression configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CompressionConfig {
    None,
    Gzip,
    Lz4,
    Zstd,
    Auto, // Choose based on payload size
}

/// Service discovery configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServiceDiscoveryConfig {
    /// No service discovery (static configuration)
    None,
    
    /// Kubernetes service discovery
    Kubernetes {
        namespace: String,
        label_selector: String,
    },
    
    /// Consul service discovery
    Consul {
        datacenter: String,
        service_prefix: String,
    },
    
    /// Static configuration
    Static {
        endpoints: HashMap<String, Vec<String>>,
    },
    
    /// DNS-based discovery
    Dns {
        domain: String,
        srv_records: bool,
    },
}

impl ServiceDiscoveryConfig {
    pub fn none() -> Self {
        ServiceDiscoveryConfig::None
    }
    
    pub fn kubernetes() -> Self {
        ServiceDiscoveryConfig::Kubernetes {
            namespace: "default".to_string(),
            label_selector: "app=rvoip".to_string(),
        }
    }
    
    pub fn static_config() -> Self {
        ServiceDiscoveryConfig::Static {
            endpoints: HashMap::new(),
        }
    }
}

/// Performance configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceConfig {
    /// Maximum concurrent sessions
    pub max_sessions: usize,
    
    /// Event buffer size
    pub event_buffer_size: usize,
    
    /// Worker thread count
    pub worker_threads: usize,
    
    /// Enable CPU affinity
    pub cpu_affinity: bool,
    
    /// Memory pool size in MB
    pub memory_pool_mb: usize,
    
    /// Enable performance monitoring
    pub monitoring_enabled: bool,
}

impl PerformanceConfig {
    pub fn low_resource() -> Self {
        Self {
            max_sessions: 10,
            event_buffer_size: 1000,
            worker_threads: 2,
            cpu_affinity: false,
            memory_pool_mb: 100,
            monitoring_enabled: false,
        }
    }
    
    pub fn balanced() -> Self {
        Self {
            max_sessions: 100,
            event_buffer_size: 10000,
            worker_threads: 4,
            cpu_affinity: false,
            memory_pool_mb: 500,
            monitoring_enabled: true,
        }
    }
    
    pub fn high_throughput() -> Self {
        Self {
            max_sessions: 10000,
            event_buffer_size: 100000,
            worker_threads: 16,
            cpu_affinity: true,
            memory_pool_mb: 4000,
            monitoring_enabled: true,
        }
    }
}