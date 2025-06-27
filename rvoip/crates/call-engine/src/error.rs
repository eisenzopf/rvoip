use thiserror::Error;

/// Call center engine errors
#[derive(Error, Debug)]
pub enum CallCenterError {
    /// Session-related errors
    #[error("Session error: {0}")]
    Session(#[from] rvoip_session_core::api::SessionError),
    
    /// Database errors
    #[error("Database error: {0}")]
    Database(String),
    
    /// Agent-related errors
    #[error("Agent error: {0}")]
    Agent(String),
    
    /// Queue-related errors
    #[error("Queue error: {0}")]
    Queue(String),
    
    /// Routing errors
    #[error("Routing error: {0}")]
    Routing(String),
    
    /// Bridge errors
    #[error("Bridge error: {0}")]
    Bridge(String),
    
    /// Orchestration errors
    #[error("Orchestration error: {0}")]
    Orchestration(String),
    
    /// Configuration errors
    #[error("Configuration error: {0}")]
    Configuration(String),
    
    /// Authentication errors
    #[error("Authentication error: {0}")]
    Authentication(String),
    
    /// Authorization errors
    #[error("Authorization error: {0}")]
    Authorization(String),
    
    /// Resource unavailable
    #[error("Resource unavailable: {0}")]
    ResourceUnavailable(String),
    
    /// Invalid input
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    
    /// Not found
    #[error("Not found: {0}")]
    NotFound(String),
    
    /// Already exists
    #[error("Already exists: {0}")]
    AlreadyExists(String),
    
    /// Timeout
    #[error("Operation timed out: {0}")]
    Timeout(String),
    
    /// Internal error
    #[error("Internal error: {0}")]
    Internal(String),
    
    /// Integration errors
    #[error("Integration error: {0}")]
    Integration(String),
    
    /// Validation errors
    #[error("Validation error: {0}")]
    Validation(String),
}

impl CallCenterError {
    /// Create a new Agent error
    pub fn agent<S: Into<String>>(msg: S) -> Self {
        Self::Agent(msg.into())
    }
    
    /// Create a new Queue error
    pub fn queue<S: Into<String>>(msg: S) -> Self {
        Self::Queue(msg.into())
    }
    
    /// Create a new Routing error
    pub fn routing<S: Into<String>>(msg: S) -> Self {
        Self::Routing(msg.into())
    }
    
    /// Create a new Bridge error
    pub fn bridge<S: Into<String>>(msg: S) -> Self {
        Self::Bridge(msg.into())
    }
    
    /// Create a new Orchestration error
    pub fn orchestration<S: Into<String>>(msg: S) -> Self {
        Self::Orchestration(msg.into())
    }
    
    /// Create a new Database error
    pub fn database<S: Into<String>>(msg: S) -> Self {
        Self::Database(msg.into())
    }
    
    /// Create a new Configuration error
    pub fn configuration<S: Into<String>>(msg: S) -> Self {
        Self::Configuration(msg.into())
    }
    
    /// Create a new NotFound error
    pub fn not_found<S: Into<String>>(msg: S) -> Self {
        Self::NotFound(msg.into())
    }
    
    /// Create a new Internal error
    pub fn internal<S: Into<String>>(msg: S) -> Self {
        Self::Internal(msg.into())
    }
    
    /// Create a new Integration error
    pub fn integration<S: Into<String>>(msg: S) -> Self {
        Self::Integration(msg.into())
    }
    
    /// Create a new Validation error
    pub fn validation<S: Into<String>>(msg: S) -> Self {
        Self::Validation(msg.into())
    }
}

/// Result type for call center operations
pub type Result<T> = std::result::Result<T, CallCenterError>; 