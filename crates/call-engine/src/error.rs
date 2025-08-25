use thiserror::Error;

/// Comprehensive error types for call center operations
///
/// This enum covers all possible error conditions that can occur during call center
/// operations, from session management to database operations and business logic failures.
///
/// # Examples
///
/// ```
/// use rvoip_call_engine::{CallCenterError, Result};
/// 
/// fn process_call() -> Result<()> {
///     // Simulate an agent error
///     Err(CallCenterError::agent("Agent not available"))
/// }
/// 
/// match process_call() {
///     Ok(_) => println!("Call processed successfully"),
///     Err(CallCenterError::Agent(msg)) => println!("Agent error: {}", msg),
///     Err(e) => println!("Other error: {}", e),
/// }
/// ```
#[derive(Error, Debug)]
pub enum CallCenterError {
    /// Session-related errors from the underlying SIP session layer
    ///
    /// These errors originate from rvoip-session-core and indicate problems
    /// with SIP session management, call setup, or media negotiation.
    #[error("Session error: {0}")]
    Session(#[from] rvoip_session_core::api::SessionError),
    
    /// Database operation errors
    ///
    /// Includes connection failures, SQL errors, transaction problems,
    /// and data consistency issues with the Limbo SQLite database.
    ///
    /// # Examples
    /// - Connection timeout
    /// - SQL syntax errors
    /// - Foreign key constraint violations
    /// - Database file corruption
    #[error("Database error: {0}")]
    Database(String),
    
    /// Agent-related errors
    ///
    /// Covers agent registration, authentication, skill validation,
    /// availability tracking, and agent state management issues.
    ///
    /// # Examples
    /// - Agent already registered
    /// - Invalid agent credentials
    /// - Agent skills don't match requirements
    /// - Agent unavailable for assignment
    #[error("Agent error: {0}")]
    Agent(String),
    
    /// Queue-related errors
    ///
    /// Issues with call queuing, queue management, overflow handling,
    /// and queue policy enforcement.
    ///
    /// # Examples
    /// - Queue full
    /// - Invalid queue configuration
    /// - Queue processing failures
    /// - Overflow policy violations
    #[error("Queue error: {0}")]
    Queue(String),
    
    /// Call routing errors
    ///
    /// Problems with call routing logic, skill matching, load balancing,
    /// and routing decision enforcement.
    ///
    /// # Examples
    /// - No available agents match skills
    /// - Routing rules misconfigured
    /// - Load balancing algorithm failures
    /// - Geographic routing unavailable
    #[error("Routing error: {0}")]
    Routing(String),
    
    /// SIP bridge operation errors
    ///
    /// Issues with creating, managing, or tearing down SIP bridges
    /// between agents and customers.
    ///
    /// # Examples
    /// - Bridge creation failed
    /// - Bridge audio problems
    /// - Bridge tear-down timeout
    /// - Bridge resource exhaustion
    #[error("Bridge error: {0}")]
    Bridge(String),
    
    /// Call center orchestration errors
    ///
    /// High-level coordination problems that don't fit into specific
    /// subsystem categories. Usually indicates system-wide issues.
    ///
    /// # Examples
    /// - System overload
    /// - Configuration conflicts
    /// - Resource exhaustion
    /// - Inter-component communication failures
    #[error("Orchestration error: {0}")]
    Orchestration(String),
    
    /// Configuration validation and parsing errors
    ///
    /// Problems with call center configuration, including invalid values,
    /// missing required settings, and configuration file parsing errors.
    ///
    /// # Examples
    /// - Invalid IP address format
    /// - Port numbers out of range
    /// - Missing required configuration
    /// - Configuration file not found
    #[error("Configuration error: {0}")]
    Configuration(String),
    
    /// Authentication failures
    ///
    /// Agent or supervisor authentication problems, including invalid
    /// credentials, expired tokens, and authentication service failures.
    ///
    /// # Examples
    /// - Invalid username/password
    /// - Expired authentication token
    /// - Authentication service unavailable
    /// - Multi-factor authentication failed
    #[error("Authentication error: {0}")]
    Authentication(String),
    
    /// Authorization failures
    ///
    /// Permission and access control errors when agents or supervisors
    /// attempt operations they're not authorized to perform.
    ///
    /// # Examples
    /// - Insufficient privileges
    /// - Access denied to supervisor features
    /// - Department restrictions violated
    /// - Time-based access restrictions
    #[error("Authorization error: {0}")]
    Authorization(String),
    
    /// Resource unavailable errors
    ///
    /// System resources (memory, CPU, network, disk) are temporarily
    /// unavailable or exhausted.
    ///
    /// # Examples
    /// - Maximum concurrent calls reached
    /// - Database connection pool exhausted
    /// - Network bandwidth exceeded
    /// - Disk space full
    #[error("Resource unavailable: {0}")]
    ResourceUnavailable(String),
    
    /// Invalid input validation errors
    ///
    /// User-provided input failed validation checks, including format
    /// validation, range checks, and business rule violations.
    ///
    /// # Examples
    /// - Invalid phone number format
    /// - Agent ID contains invalid characters
    /// - Skill name too long
    /// - Queue priority out of range
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    
    /// Resource not found errors
    ///
    /// Requested agents, calls, queues, or other resources could not
    /// be located in the system.
    ///
    /// # Examples
    /// - Agent ID not found
    /// - Call session doesn't exist
    /// - Queue not configured
    /// - Bridge ID invalid
    #[error("Not found: {0}")]
    NotFound(String),
    
    /// Resource already exists errors
    ///
    /// Attempt to create a resource that already exists in the system,
    /// typically during registration or initialization operations.
    ///
    /// # Examples
    /// - Agent already registered
    /// - Queue already exists
    /// - Duplicate session ID
    /// - Bridge already created
    #[error("Already exists: {0}")]
    AlreadyExists(String),
    
    /// Operation timeout errors
    ///
    /// Operations that failed to complete within the specified time limit,
    /// including network operations, database queries, and call setup.
    ///
    /// # Examples
    /// - Database query timeout
    /// - SIP INVITE timeout
    /// - Agent response timeout
    /// - Bridge setup timeout
    #[error("Operation timed out: {0}")]
    Timeout(String),
    
    /// Internal system errors
    ///
    /// Unexpected internal errors that indicate bugs or system corruption.
    /// These should be logged and may require system restart.
    ///
    /// # Examples
    /// - Assertion failures
    /// - Memory corruption detected
    /// - Invalid internal state
    /// - Unreachable code executed
    #[error("Internal error: {0}")]
    Internal(String),
    
    /// Integration layer errors
    ///
    /// Problems with integrating with external systems, including
    /// session-core, databases, and third-party services.
    ///
    /// # Examples
    /// - Session-core API changes
    /// - Database schema mismatch
    /// - External service unavailable
    /// - Protocol version mismatch
    #[error("Integration error: {0}")]
    Integration(String),
    
    /// Data validation errors
    ///
    /// Problems with data consistency, integrity checks, and business
    /// rule validation across the call center system.
    ///
    /// # Examples
    /// - Agent skills don't match database
    /// - Call state inconsistent
    /// - Queue statistics invalid
    /// - Configuration conflicts detected
    #[error("Validation error: {0}")]
    Validation(String),
}

impl CallCenterError {
    /// Create a new Agent error with the provided message
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_call_engine::CallCenterError;
    /// 
    /// let error = CallCenterError::agent("Agent not available for assignment");
    /// println!("{}", error);  // Prints: Agent error: Agent not available for assignment
    /// ```
    pub fn agent<S: Into<String>>(msg: S) -> Self {
        Self::Agent(msg.into())
    }
    
    /// Create a new Queue error with the provided message
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_call_engine::CallCenterError;
    /// 
    /// let error = CallCenterError::queue("Queue full, cannot accept more calls");
    /// println!("{}", error);
    /// ```
    pub fn queue<S: Into<String>>(msg: S) -> Self {
        Self::Queue(msg.into())
    }
    
    /// Create a new Routing error with the provided message
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_call_engine::CallCenterError;
    /// 
    /// let error = CallCenterError::routing("No agents available with required skills");
    /// println!("{}", error);
    /// ```
    pub fn routing<S: Into<String>>(msg: S) -> Self {
        Self::Routing(msg.into())
    }
    
    /// Create a new Bridge error with the provided message
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_call_engine::CallCenterError;
    /// 
    /// let error = CallCenterError::bridge("Failed to create SIP bridge");
    /// println!("{}", error);
    /// ```
    pub fn bridge<S: Into<String>>(msg: S) -> Self {
        Self::Bridge(msg.into())
    }
    
    /// Create a new Orchestration error with the provided message
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_call_engine::CallCenterError;
    /// 
    /// let error = CallCenterError::orchestration("System overload detected");
    /// println!("{}", error);
    /// ```
    pub fn orchestration<S: Into<String>>(msg: S) -> Self {
        Self::Orchestration(msg.into())
    }
    
    /// Create a new Database error with the provided message
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_call_engine::CallCenterError;
    /// 
    /// let error = CallCenterError::database("Connection to database failed");
    /// println!("{}", error);
    /// ```
    pub fn database<S: Into<String>>(msg: S) -> Self {
        Self::Database(msg.into())
    }
    
    /// Create a new Configuration error with the provided message
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_call_engine::CallCenterError;
    /// 
    /// let error = CallCenterError::configuration("Invalid port number in config");
    /// println!("{}", error);
    /// ```
    pub fn configuration<S: Into<String>>(msg: S) -> Self {
        Self::Configuration(msg.into())
    }
    
    /// Create a new NotFound error with the provided message
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_call_engine::CallCenterError;
    /// 
    /// let error = CallCenterError::not_found("Agent with ID 'agent-123' not found");
    /// println!("{}", error);
    /// ```
    pub fn not_found<S: Into<String>>(msg: S) -> Self {
        Self::NotFound(msg.into())
    }
    
    /// Create a new Internal error with the provided message
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_call_engine::CallCenterError;
    /// 
    /// let error = CallCenterError::internal("Unexpected state in call processing");
    /// println!("{}", error);
    /// ```
    pub fn internal<S: Into<String>>(msg: S) -> Self {
        Self::Internal(msg.into())
    }
    
    /// Create a new Integration error with the provided message
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_call_engine::CallCenterError;
    /// 
    /// let error = CallCenterError::integration("Session-core API call failed");
    /// println!("{}", error);
    /// ```
    pub fn integration<S: Into<String>>(msg: S) -> Self {
        Self::Integration(msg.into())
    }
    
    /// Create a new Validation error with the provided message
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_call_engine::CallCenterError;
    /// 
    /// let error = CallCenterError::validation("Agent skills validation failed");
    /// println!("{}", error);
    /// ```
    pub fn validation<S: Into<String>>(msg: S) -> Self {
        Self::Validation(msg.into())
    }
}

/// Result type for call center operations
///
/// This is a type alias for `std::result::Result<T, CallCenterError>` that simplifies
/// error handling throughout the call center codebase.
///
/// # Examples
///
/// ```
/// use rvoip_call_engine::{Result, CallCenterError};
/// 
/// fn register_agent(agent_id: &str) -> Result<String> {
///     if agent_id.is_empty() {
///         return Err(CallCenterError::InvalidInput("Agent ID cannot be empty".to_string()));
///     }
///     Ok(format!("session-{}", agent_id))
/// }
/// 
/// match register_agent("") {
///     Ok(session_id) => println!("Agent registered: {}", session_id),
///     Err(e) => tracing::error!("Registration failed: {}", e),
/// }
/// ```
pub type Result<T> = std::result::Result<T, CallCenterError>; 