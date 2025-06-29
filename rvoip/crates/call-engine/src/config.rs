use std::time::Duration;
use std::net::SocketAddr;
use serde::{Deserialize, Serialize};

/// Call center configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallCenterConfig {
    /// General call center settings
    pub general: GeneralConfig,
    
    /// Agent management configuration
    pub agents: AgentConfig,
    
    /// Queue management configuration
    pub queues: QueueConfig,
    
    /// Routing configuration
    pub routing: RoutingConfig,
    
    /// Monitoring configuration
    pub monitoring: MonitoringConfig,
    
    /// Database configuration
    pub database: DatabaseConfig,
}

/// General call center configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    /// Maximum number of concurrent calls
    pub max_concurrent_calls: usize,
    
    /// Maximum number of agents
    pub max_agents: usize,
    
    /// Default call timeout in seconds
    pub default_call_timeout: u64,
    
    /// Session cleanup interval
    pub cleanup_interval: Duration,
    
    /// Local signaling address
    pub local_signaling_addr: SocketAddr,
    
    /// Local media address range start
    pub local_media_addr: SocketAddr,
    
    /// User agent string
    pub user_agent: String,
    
    /// Domain name
    pub domain: String,
    
    /// Local IP address for SIP URIs (replaces hardcoded 127.0.0.1)
    pub local_ip: String,
    
    /// Registrar domain for agent registration
    pub registrar_domain: String,
    
    /// Call center service URI prefix
    pub call_center_service: String,
    
    /// PHASE 0.24: BYE timeout configuration (seconds)
    pub bye_timeout_seconds: u64,
    
    /// PHASE 0.24: BYE retry attempts
    pub bye_retry_attempts: u32,
    
    /// PHASE 0.24: Race condition prevention delay (milliseconds)
    pub bye_race_delay_ms: u64,
}

/// Agent management configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Default maximum concurrent calls per agent
    pub default_max_concurrent_calls: u32,
    
    /// Agent availability timeout (seconds)
    pub availability_timeout: u64,
    
    /// Auto-logout timeout for idle agents (seconds)
    pub auto_logout_timeout: u64,
    
    /// Enable skill-based routing
    pub enable_skill_based_routing: bool,
    
    /// Default skills for new agents
    pub default_skills: Vec<String>,
}

/// Queue configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueConfig {
    /// Default maximum wait time in queue (seconds)
    pub default_max_wait_time: u64,
    
    /// Maximum queue size
    pub max_queue_size: usize,
    
    /// Enable queue priorities
    pub enable_priorities: bool,
    
    /// Enable overflow routing
    pub enable_overflow: bool,
    
    /// Queue announcement interval (seconds)
    pub announcement_interval: u64,
}

/// Routing configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingConfig {
    /// Default routing strategy
    pub default_strategy: RoutingStrategy,
    
    /// Enable load balancing
    pub enable_load_balancing: bool,
    
    /// Load balancing strategy
    pub load_balance_strategy: LoadBalanceStrategy,
    
    /// Enable geographic routing
    pub enable_geographic_routing: bool,
    
    /// Enable time-based routing
    pub enable_time_based_routing: bool,
}

/// Routing strategy enumeration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RoutingStrategy {
    RoundRobin,
    LeastRecentlyUsed,
    SkillBased,
    Random,
    Priority,
}

/// Load balancing strategy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LoadBalanceStrategy {
    EqualDistribution,
    WeightedDistribution,
    LeastBusy,
    MostExperienced,
}

/// Monitoring configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringConfig {
    /// Enable real-time monitoring
    pub enable_realtime_monitoring: bool,
    
    /// Metrics collection interval (seconds)
    pub metrics_interval: u64,
    
    /// Enable call recording
    pub enable_call_recording: bool,
    
    /// Enable quality monitoring
    pub enable_quality_monitoring: bool,
    
    /// Dashboard update interval (seconds)
    pub dashboard_update_interval: u64,
}

/// Database configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// Database file path (empty for in-memory)
    pub database_path: String,
    
    /// Enable database connection pooling
    pub enable_connection_pooling: bool,
    
    /// Maximum database connections
    pub max_connections: u32,
    
    /// Database query timeout (seconds)
    pub query_timeout: u64,
    
    /// Enable automatic backups
    pub enable_auto_backup: bool,
    
    /// Backup interval (seconds)
    pub backup_interval: u64,
}

impl CallCenterConfig {
    /// Validate the configuration for consistency and correctness
    pub fn validate(&self) -> Result<(), String> {
        // Validate IP address format
        if self.general.local_ip.is_empty() {
            return Err("local_ip cannot be empty".to_string());
        }
        
        // Basic IP address validation (IPv4 or IPv6)
        if !self.general.local_ip.parse::<std::net::IpAddr>().is_ok() {
            return Err(format!("Invalid IP address format: {}", self.general.local_ip));
        }
        
        // Validate domain names are not empty
        if self.general.domain.is_empty() {
            return Err("domain cannot be empty".to_string());
        }
        
        if self.general.registrar_domain.is_empty() {
            return Err("registrar_domain cannot be empty".to_string());
        }
        
        if self.general.call_center_service.is_empty() {
            return Err("call_center_service cannot be empty".to_string());
        }
        
        // Validate numeric constraints
        if self.general.max_concurrent_calls == 0 {
            return Err("max_concurrent_calls must be greater than 0".to_string());
        }
        
        if self.general.max_agents == 0 {
            return Err("max_agents must be greater than 0".to_string());
        }
        
        // Validate queue configuration
        if self.queues.max_queue_size == 0 {
            return Err("max_queue_size must be greater than 0".to_string());
        }
        
        // PHASE 0.24: Validate BYE configuration
        if self.general.bye_timeout_seconds == 0 {
            return Err("bye_timeout_seconds must be greater than 0".to_string());
        }
        
        if self.general.bye_timeout_seconds > 300 {
            return Err("bye_timeout_seconds cannot exceed 300 seconds (5 minutes)".to_string());
        }
        
        if self.general.bye_retry_attempts > 10 {
            return Err("bye_retry_attempts cannot exceed 10".to_string());
        }
        
        if self.general.bye_race_delay_ms > 5000 {
            return Err("bye_race_delay_ms cannot exceed 5000ms (5 seconds)".to_string());
        }
        
        Ok(())
    }
}

impl Default for CallCenterConfig {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            agents: AgentConfig::default(),
            queues: QueueConfig::default(),
            routing: RoutingConfig::default(),
            monitoring: MonitoringConfig::default(),
            database: DatabaseConfig::default(),
        }
    }
}

impl GeneralConfig {
    /// Generate agent SIP URI from username
    pub fn agent_sip_uri(&self, username: &str) -> String {
        format!("sip:{}@{}", username, self.local_ip)
    }
    
    /// Generate call center SIP URI  
    pub fn call_center_uri(&self) -> String {
        format!("sip:{}@{}", self.call_center_service, self.domain)
    }
    
    /// Generate registrar URI
    pub fn registrar_uri(&self) -> String {
        format!("sip:registrar@{}", self.registrar_domain)
    }
    
    /// Generate contact URI for an agent with optional port
    pub fn agent_contact_uri(&self, username: &str, port: Option<u16>) -> String {
        match port {
            Some(port) => format!("sip:{}@{}:{}", username, self.local_ip, port),
            None => self.agent_sip_uri(username),
        }
    }
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            max_concurrent_calls: 1000,
            max_agents: 500,
            default_call_timeout: 300, // 5 minutes
            cleanup_interval: Duration::from_secs(60),
            local_signaling_addr: "0.0.0.0:5060".parse().unwrap(),
            local_media_addr: "0.0.0.0:10000".parse().unwrap(),
            user_agent: "rvoip-call-center/0.1.0".to_string(),
            domain: "call-center.local".to_string(),
            local_ip: "127.0.0.1".to_string(),  // Safe default for development
            registrar_domain: "call-center.local".to_string(),
            call_center_service: "call-center".to_string(),
            
            // PHASE 0.24: BYE handling configuration with production-ready defaults
            bye_timeout_seconds: 15,     // Increased from 5s to 15s for better reliability
            bye_retry_attempts: 3,       // Allow 3 retry attempts for failed BYEs
            bye_race_delay_ms: 100,      // 100ms delay to prevent race conditions
        }
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            default_max_concurrent_calls: 3,
            availability_timeout: 300, // 5 minutes
            auto_logout_timeout: 3600, // 1 hour
            enable_skill_based_routing: true,
            default_skills: vec!["general".to_string()],
        }
    }
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            default_max_wait_time: 600, // 10 minutes
            max_queue_size: 100,
            enable_priorities: true,
            enable_overflow: true,
            announcement_interval: 30, // 30 seconds
        }
    }
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            default_strategy: RoutingStrategy::SkillBased,
            enable_load_balancing: true,
            load_balance_strategy: LoadBalanceStrategy::LeastBusy,
            enable_geographic_routing: false,
            enable_time_based_routing: true,
        }
    }
}

impl Default for MonitoringConfig {
    fn default() -> Self {
        Self {
            enable_realtime_monitoring: true,
            metrics_interval: 10, // 10 seconds
            enable_call_recording: false,
            enable_quality_monitoring: true,
            dashboard_update_interval: 5, // 5 seconds
        }
    }
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            database_path: "call_center.db".to_string(),
            enable_connection_pooling: true,
            max_connections: 10,
            query_timeout: 30, // 30 seconds
            enable_auto_backup: false,
            backup_interval: 3600, // 1 hour
        }
    }
} 