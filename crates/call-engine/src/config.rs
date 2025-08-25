use std::time::Duration;
use std::net::SocketAddr;
use serde::{Deserialize, Serialize};

/// Comprehensive call center configuration
///
/// This is the main configuration structure that encompasses all aspects of call center
/// operation, from basic networking settings to advanced routing algorithms and monitoring.
///
/// # Configuration Sections
///
/// - [`general`]: Basic system settings like networking, timeouts, and limits
/// - [`agents`]: Agent management configuration including skills and availability
/// - [`queues`]: Call queuing behavior, priorities, and overflow handling
/// - [`routing`]: Call routing strategies, load balancing, and geographic routing
/// - [`monitoring`]: Real-time monitoring, metrics collection, and quality tracking
/// - [`database`]: Persistent storage configuration and backup settings
///
/// # Examples
///
/// ## Default Configuration
///
/// ```
/// use rvoip_call_engine::prelude::CallCenterConfig;
/// 
/// let config = CallCenterConfig::default();
/// assert_eq!(config.general.max_concurrent_calls, 1000);
/// assert_eq!(config.agents.default_max_concurrent_calls, 3);
/// ```
///
/// ## Custom Configuration
///
/// ```
/// use rvoip_call_engine::prelude::{CallCenterConfig, RoutingStrategy, LoadBalanceStrategy};
/// 
/// let mut config = CallCenterConfig::default();
/// 
/// // Configure high-capacity setup
/// config.general.max_concurrent_calls = 5000;
/// config.general.max_agents = 1000;
/// 
/// // Enable advanced routing
/// config.routing.default_strategy = RoutingStrategy::SkillBased;
/// config.routing.load_balance_strategy = LoadBalanceStrategy::LeastBusy;
/// 
/// // Validate configuration
/// config.validate().expect("Configuration should be valid");
/// ```
///
/// # Validation
///
/// Configuration can be validated using the [`validate`] method:
///
/// ```
/// use rvoip_call_engine::prelude::CallCenterConfig;
/// 
/// let config = CallCenterConfig::default();
/// match config.validate() {
///     Ok(()) => println!("Configuration is valid"),
///     Err(e) => tracing::error!("Configuration error: {}", e),
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallCenterConfig {
    /// General call center settings including networking and system limits
    pub general: GeneralConfig,
    
    /// Agent management configuration including skills and availability tracking
    pub agents: AgentConfig,
    
    /// Queue management configuration including priorities and overflow policies
    pub queues: QueueConfig,
    
    /// Call routing configuration including strategies and load balancing
    pub routing: RoutingConfig,
    
    /// Monitoring and metrics configuration
    pub monitoring: MonitoringConfig,
    
    /// Database configuration for persistent storage
    pub database: DatabaseConfig,
}

/// General call center system configuration
///
/// This structure contains the core system settings that affect the overall
/// operation of the call center, including network configuration, system limits,
/// and timeout settings.
///
/// # Network Configuration
///
/// The call center requires proper network configuration for SIP signaling and RTP media:
///
/// - `local_signaling_addr`: Address for SIP signaling (typically port 5060)
/// - `local_media_addr`: Starting address for RTP media streams
/// - `local_ip`: IP address used in SIP URIs and contact headers
/// - `domain`: SIP domain for the call center
///
/// # Examples
///
/// ```
/// use rvoip_call_engine::prelude::GeneralConfig;
/// 
/// let config = GeneralConfig::default();
/// 
/// // Generate URIs for agents
/// let agent_uri = config.agent_sip_uri("alice");
/// assert_eq!(agent_uri, "sip:alice@127.0.0.1");
/// 
/// let call_center_uri = config.call_center_uri();
/// assert_eq!(call_center_uri, "sip:call-center@call-center.local");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    /// Maximum number of concurrent calls the system can handle
    ///
    /// This includes both active calls and calls in queue. When this limit is reached,
    /// new incoming calls will be rejected with a busy signal.
    pub max_concurrent_calls: usize,
    
    /// Maximum number of agents that can be registered with the system
    ///
    /// This limit helps prevent resource exhaustion and ensures predictable
    /// system performance.
    pub max_agents: usize,
    
    /// Default call timeout in seconds
    ///
    /// How long to wait for call setup operations before timing out.
    /// This applies to INVITE transactions and bridge setup operations.
    pub default_call_timeout: u64,
    
    /// Session cleanup interval for removing stale sessions
    ///
    /// How often the system checks for and removes abandoned or stale sessions.
    pub cleanup_interval: Duration,
    
    /// Local signaling address for SIP messages
    ///
    /// The address and port where the call center listens for incoming SIP messages.
    /// Typically this is port 5060 for standard SIP.
    pub local_signaling_addr: SocketAddr,
    
    /// Local media address range start for RTP streams
    ///
    /// The starting address and port for RTP media streams. The system will allocate
    /// port pairs starting from this address for each call.
    pub local_media_addr: SocketAddr,
    
    /// User agent string sent in SIP messages
    ///
    /// Identifies the call center software in SIP headers.
    pub user_agent: String,
    
    /// SIP domain name for the call center
    ///
    /// Used in SIP URIs and routing decisions.
    pub domain: String,
    
    /// Local IP address for SIP URIs
    ///
    /// The IP address used in SIP URIs and contact headers. Should be reachable
    /// by agents and external SIP endpoints.
    pub local_ip: String,
    
    /// Registrar domain for agent registration
    ///
    /// The domain that agents should use when registering with the call center.
    pub registrar_domain: String,
    
    /// Call center service URI prefix
    ///
    /// The service name used in call center SIP URIs.
    pub call_center_service: String,
    
    /// BYE timeout configuration in seconds
    ///
    /// How long to wait for BYE responses before considering the request failed.
    /// Prevents hanging connections when call termination fails.
    pub bye_timeout_seconds: u64,
    
    /// BYE retry attempts
    ///
    /// Number of times to retry sending BYE requests when they fail or timeout.
    pub bye_retry_attempts: u32,
    
    /// Race condition prevention delay in milliseconds
    ///
    /// Small delay to prevent race conditions during call termination.
    pub bye_race_delay_ms: u64,
}

/// Agent management configuration
///
/// Controls how agents are managed, including registration, availability tracking,
/// skill-based routing, and automatic state management.
///
/// # Skill-Based Routing
///
/// When `enable_skill_based_routing` is true, the system will match incoming
/// calls with agents based on required skills. Agents without matching skills
/// will not be considered for call assignment.
///
/// # Examples
///
/// ```
/// use rvoip_call_engine::prelude::AgentConfig;
/// 
/// let config = AgentConfig {
///     enable_skill_based_routing: true,
///     default_skills: vec!["english".to_string(), "general".to_string()],
///     default_max_concurrent_calls: 5,
///     ..Default::default()
/// };
/// assert_eq!(config.default_max_concurrent_calls, 5);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Default maximum concurrent calls per agent
    ///
    /// When agents register without specifying their capacity, this value
    /// is used as their maximum concurrent call limit.
    pub default_max_concurrent_calls: u32,
    
    /// Agent availability timeout in seconds
    ///
    /// How long an agent can remain idle before being considered unavailable.
    /// After this timeout, the agent may be automatically logged out.
    pub availability_timeout: u64,
    
    /// Auto-logout timeout for idle agents in seconds
    ///
    /// Agents who remain idle (no calls or activity) for this duration
    /// will be automatically logged out to free up system resources.
    pub auto_logout_timeout: u64,
    
    /// Enable skill-based routing for call assignment
    ///
    /// When true, incoming calls will only be routed to agents whose skills
    /// match the call requirements. When false, calls can be routed to any
    /// available agent.
    pub enable_skill_based_routing: bool,
    
    /// Default skills assigned to new agents
    ///
    /// When agents register without specifying skills, they receive these
    /// default skills. Should include basic skills like language support.
    pub default_skills: Vec<String>,
}

/// Queue configuration for call queuing and waiting
///
/// Controls how calls are queued when no agents are immediately available,
/// including wait times, queue sizes, priorities, and overflow handling.
///
/// # Queue Priorities
///
/// When `enable_priorities` is true, calls can be assigned different priority
/// levels. Higher priority calls will be served before lower priority calls,
/// even if they arrived later.
///
/// # Overflow Handling
///
/// When `enable_overflow` is true and a queue reaches capacity or maximum
/// wait time, calls can be routed to alternate destinations or handled
/// with special policies.
///
/// # Examples
///
/// ```
/// use rvoip_call_engine::prelude::QueueConfig;
/// 
/// let config = QueueConfig {
///     default_max_wait_time: 180, // 3 minutes
///     max_queue_size: 20,
///     enable_priorities: true,
///     enable_overflow: true,
///     announcement_interval: 15, // Every 15 seconds
/// };
/// assert_eq!(config.default_max_wait_time, 180);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueConfig {
    /// Default maximum wait time in queue (seconds)
    ///
    /// How long calls will wait in queue before being redirected, transferred
    /// to voicemail, or handled by overflow policies.
    pub default_max_wait_time: u64,
    
    /// Maximum queue size (number of calls)
    ///
    /// When a queue reaches this size, new calls will be rejected or handled
    /// by overflow policies rather than being added to the queue.
    pub max_queue_size: usize,
    
    /// Enable queue priorities for different call types
    ///
    /// Allows calls to be queued with different priority levels, ensuring
    /// high-priority calls are handled first.
    pub enable_priorities: bool,
    
    /// Enable overflow routing when queues are full
    ///
    /// When enabled, calls that cannot be queued will be routed to alternate
    /// destinations or handled with special policies.
    pub enable_overflow: bool,
    
    /// Queue announcement interval (seconds)
    ///
    /// How often to play queue position and wait time announcements to
    /// callers waiting in queue.
    pub announcement_interval: u64,
}

/// Call routing configuration
///
/// Controls how incoming calls are routed to agents, including routing strategies,
/// load balancing algorithms, geographic routing, and time-based routing rules.
///
/// # Routing Strategies
///
/// Different strategies determine how calls are distributed among available agents:
///
/// - `RoundRobin`: Distributes calls evenly among agents in rotation
/// - `LeastRecentlyUsed`: Routes to the agent who hasn't handled a call in the longest time
/// - `SkillBased`: Routes based on matching agent skills with call requirements
/// - `Random`: Randomly selects from available agents
/// - `Priority`: Routes to agents based on priority rankings
///
/// # Load Balancing
///
/// When `enable_load_balancing` is true, the system considers agent workload
/// when making routing decisions to ensure even distribution of calls.
///
/// # Examples
///
/// ```
/// use rvoip_call_engine::prelude::{RoutingConfig, RoutingStrategy, LoadBalanceStrategy};
/// 
/// let config = RoutingConfig {
///     default_strategy: RoutingStrategy::SkillBased,
///     enable_load_balancing: true,
///     load_balance_strategy: LoadBalanceStrategy::LeastBusy,
///     enable_geographic_routing: false,
///     enable_time_based_routing: true,
/// };
/// assert!(config.enable_load_balancing);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingConfig {
    /// Default routing strategy for call distribution
    ///
    /// The primary algorithm used to select agents for incoming calls.
    pub default_strategy: RoutingStrategy,
    
    /// Enable load balancing across agents
    ///
    /// When true, routing decisions consider current agent workload to
    /// ensure even distribution of calls.
    pub enable_load_balancing: bool,
    
    /// Load balancing strategy to use
    ///
    /// The algorithm used for load balancing when enabled.
    pub load_balance_strategy: LoadBalanceStrategy,
    
    /// Enable geographic-based routing
    ///
    /// When true, calls can be routed based on geographic location of
    /// callers and agents (requires additional configuration).
    pub enable_geographic_routing: bool,
    
    /// Enable time-based routing rules
    ///
    /// When true, routing decisions can consider time of day, day of week,
    /// and business hours in routing logic.
    pub enable_time_based_routing: bool,
}

/// Call routing strategy enumeration
///
/// Defines the available algorithms for routing calls to agents.
/// Each strategy has different characteristics and use cases.
///
/// # Strategy Details
///
/// - **RoundRobin**: Simple fair distribution, good for balanced workloads
/// - **LeastRecentlyUsed**: Ensures all agents get calls, good for training
/// - **SkillBased**: Matches agent capabilities with call needs, best for specialized support
/// - **Random**: Unpredictable distribution, useful for testing
/// - **Priority**: Routes to highest priority agents first, good for tier-based support
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RoutingStrategy {
    /// Distribute calls evenly among agents in rotation
    RoundRobin,
    /// Route to the agent who hasn't handled a call in the longest time
    LeastRecentlyUsed,
    /// Route based on matching agent skills with call requirements
    SkillBased,
    /// Randomly select from available agents
    Random,
    /// Route to agents based on priority rankings
    Priority,
}

/// Load balancing strategy enumeration
///
/// Defines how the system balances call load across agents when
/// load balancing is enabled.
///
/// # Strategy Comparison
///
/// - **EqualDistribution**: Aims for exactly equal call counts
/// - **WeightedDistribution**: Considers agent capabilities and preferences
/// - **LeastBusy**: Routes to agents with the least current activity
/// - **MostExperienced**: Prioritizes experienced agents for complex calls
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LoadBalanceStrategy {
    /// Distribute calls equally among all agents
    EqualDistribution,
    /// Distribute calls based on agent weights and capabilities
    WeightedDistribution,
    /// Route to the least busy agents first
    LeastBusy,
    /// Route to the most experienced agents for complex calls
    MostExperienced,
}

/// Monitoring and metrics configuration
///
/// Controls real-time monitoring, metrics collection, call recording,
/// quality monitoring, and dashboard updates.
///
/// # Real-time Monitoring
///
/// When `enable_realtime_monitoring` is true, the system provides live
/// dashboards and real-time statistics updates for supervisors.
///
/// # Call Recording
///
/// When `enable_call_recording` is true, calls can be recorded for
/// quality assurance and training purposes (requires additional setup).
///
/// # Examples
///
/// ```
/// use rvoip_call_engine::prelude::MonitoringConfig;
/// 
/// let config = MonitoringConfig {
///     enable_realtime_monitoring: true,
///     metrics_interval: 5, // Collect metrics every 5 seconds
///     enable_call_recording: false, // Disabled for privacy
///     enable_quality_monitoring: true,
///     dashboard_update_interval: 2, // Update dashboard every 2 seconds
/// };
/// assert!(config.enable_realtime_monitoring);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringConfig {
    /// Enable real-time monitoring and dashboards
    ///
    /// Provides live statistics and monitoring capabilities for supervisors.
    pub enable_realtime_monitoring: bool,
    
    /// Metrics collection interval in seconds
    ///
    /// How often the system collects and updates performance metrics.
    /// Lower values provide more real-time data but use more resources.
    pub metrics_interval: u64,
    
    /// Enable call recording for quality assurance
    ///
    /// When enabled, calls can be recorded for training and quality purposes.
    /// Requires additional storage and legal compliance considerations.
    pub enable_call_recording: bool,
    
    /// Enable quality monitoring and alerts
    ///
    /// Monitors call quality metrics and generates alerts for poor quality calls.
    pub enable_quality_monitoring: bool,
    
    /// Dashboard update interval in seconds
    ///
    /// How often supervisor dashboards are refreshed with new data.
    pub dashboard_update_interval: u64,
}

/// Database configuration for persistent storage
///
/// Controls database connections, connection pooling, query timeouts,
/// and backup settings for the SQLite (Limbo) database.
///
/// # Database Path
///
/// - Empty string or ":memory:": In-memory database (not persistent)
/// - File path: Persistent database stored on disk
///
/// # Connection Pooling
///
/// When `enable_connection_pooling` is true, the system maintains a pool
/// of database connections for better performance under high load.
///
/// # Examples
///
/// ```
/// use rvoip_call_engine::prelude::DatabaseConfig;
/// 
/// // Persistent database with connection pooling
/// let config = DatabaseConfig {
///     database_path: "/var/lib/callcenter/data.db".to_string(),
///     enable_connection_pooling: true,
///     max_connections: 20,
///     query_timeout: 10,
///     enable_auto_backup: true,
///     backup_interval: 7200, // Backup every 2 hours
/// };
/// assert!(config.enable_connection_pooling);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// Database file path (empty string for in-memory)
    ///
    /// Specifies where to store the SQLite database file. Use ":memory:"
    /// or empty string for in-memory database (data not persisted).
    pub database_path: String,
    
    /// Enable database connection pooling
    ///
    /// Maintains multiple database connections for better performance
    /// under concurrent load.
    pub enable_connection_pooling: bool,
    
    /// Maximum number of database connections in pool
    ///
    /// Only used when connection pooling is enabled.
    pub max_connections: u32,
    
    /// Database query timeout in seconds
    ///
    /// How long to wait for database queries before timing out.
    pub query_timeout: u64,
    
    /// Enable automatic database backups
    ///
    /// When enabled, the system will periodically create backup copies
    /// of the database file.
    pub enable_auto_backup: bool,
    
    /// Backup interval in seconds
    ///
    /// How often to create automatic backups when enabled.
    pub backup_interval: u64,
}

impl CallCenterConfig {
    /// Validate the configuration for consistency and correctness
    ///
    /// Performs comprehensive validation of all configuration settings,
    /// checking for valid ranges, proper formatting, and logical consistency.
    ///
    /// # Returns
    ///
    /// - `Ok(())` if the configuration is valid
    /// - `Err(String)` with a description of the validation error
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_call_engine::prelude::CallCenterConfig;
    /// 
    /// let mut config = CallCenterConfig::default();
    /// 
    /// // This should pass validation
    /// assert!(config.validate().is_ok());
    /// 
    /// // This should fail validation
    /// config.general.max_concurrent_calls = 0;
    /// assert!(config.validate().is_err());
    /// ```
    ///
    /// # Validation Rules
    ///
    /// - IP addresses must be valid IPv4 or IPv6 format
    /// - Domain names cannot be empty
    /// - Numeric limits must be greater than zero
    /// - Timeout values must be reasonable (not too large)
    /// - Port numbers must be within valid range
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
    ///
    /// Creates a properly formatted SIP URI for an agent based on their username
    /// and the configured local IP address.
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_call_engine::prelude::GeneralConfig;
    /// 
    /// let config = GeneralConfig::default();
    /// let uri = config.agent_sip_uri("alice");
    /// assert_eq!(uri, "sip:alice@127.0.0.1");
    /// ```
    pub fn agent_sip_uri(&self, username: &str) -> String {
        format!("sip:{}@{}", username, self.local_ip)
    }
    
    /// Generate call center SIP URI
    ///
    /// Creates the main SIP URI for the call center service.
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_call_engine::prelude::GeneralConfig;
    /// 
    /// let config = GeneralConfig::default();
    /// let uri = config.call_center_uri();
    /// assert_eq!(uri, "sip:call-center@call-center.local");
    /// ```
    pub fn call_center_uri(&self) -> String {
        format!("sip:{}@{}", self.call_center_service, self.domain)
    }
    
    /// Generate registrar URI
    ///
    /// Creates the SIP URI for agent registration requests.
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_call_engine::prelude::GeneralConfig;
    /// 
    /// let config = GeneralConfig::default();
    /// let uri = config.registrar_uri();
    /// assert_eq!(uri, "sip:registrar@call-center.local");
    /// ```
    pub fn registrar_uri(&self) -> String {
        format!("sip:registrar@{}", self.registrar_domain)
    }
    
    /// Generate contact URI for an agent with optional port
    ///
    /// Creates a contact URI for an agent, optionally including a specific port number.
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_call_engine::prelude::GeneralConfig;
    /// 
    /// let config = GeneralConfig::default();
    /// 
    /// // Without port
    /// let uri1 = config.agent_contact_uri("alice", None);
    /// assert_eq!(uri1, "sip:alice@127.0.0.1");
    /// 
    /// // With port
    /// let uri2 = config.agent_contact_uri("alice", Some(5061));
    /// assert_eq!(uri2, "sip:alice@127.0.0.1:5061");
    /// ```
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