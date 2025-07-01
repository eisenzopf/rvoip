//! Registration management for SIP client
//!
//! This module provides registration information structures and configuration for
//! SIP registration with registrar servers. All actual SIP registration operations
//! are delegated to session-core.
//!
//! # Architecture
//!
//! PROPER LAYER SEPARATION:
//! client-core -> session-core -> {transaction-core, media-core, sip-transport, sip-core}
//!
//! # Key Components
//!
//! - **RegistrationConfig** - Configuration for SIP registration
//! - **RegistrationStatus** - Current state of a registration
//! - **RegistrationInfo** - Complete registration details and metadata
//! - **RegistrationStats** - Aggregate statistics about registrations
//!
//! # SIP Registration Process
//!
//! 1. **Configuration** - Define server, credentials, and parameters
//! 2. **Registration** - Send REGISTER request to SIP server
//! 3. **Authentication** - Handle authentication challenge if required
//! 4. **Maintenance** - Periodically refresh registration before expiry
//! 5. **Termination** - Unregister when shutting down
//!
//! # Usage Examples
//!
//! ## Creating Registration Configuration
//!
//! ```rust
//! use rvoip_client_core::registration::RegistrationConfig;
//!
//! // Basic registration without authentication
//! let config = RegistrationConfig::new(
//!     "sip:registrar.example.com".to_string(),
//!     "sip:alice@example.com".to_string(),
//!     "sip:alice@192.168.1.100:5060".to_string(),
//! );
//!
//! // Registration with authentication
//! let auth_config = RegistrationConfig::new(
//!     "sip:registrar.example.com".to_string(),
//!     "sip:alice@example.com".to_string(),
//!     "sip:alice@192.168.1.100:5060".to_string(),
//! )
//! .with_credentials("alice".to_string(), "secret123".to_string())
//! .with_realm("example.com".to_string())
//! .with_expires(1800); // 30 minutes
//!
//! assert_eq!(auth_config.expires, 1800);
//! assert_eq!(auth_config.username, Some("alice".to_string()));
//! ```
//!
//! ## Working with Registration Status
//!
//! ```rust
//! use rvoip_client_core::registration::RegistrationStatus;
//!
//! let status = RegistrationStatus::Active;
//! println!("Registration status: {}", status);
//!
//! assert_eq!(status, RegistrationStatus::Active);
//! assert_ne!(status, RegistrationStatus::Failed);
//! ```
//!
//! ## Creating Registration Information
//!
//! ```rust
//! use rvoip_client_core::registration::{RegistrationInfo, RegistrationStatus};
//! use chrono::Utc;
//! use uuid::Uuid;
//!
//! let reg_info = RegistrationInfo {
//!     id: Uuid::new_v4(),
//!     server_uri: "sip:registrar.example.com".to_string(),
//!     from_uri: "sip:alice@example.com".to_string(),
//!     contact_uri: "sip:alice@192.168.1.100:5060".to_string(),
//!     expires: 3600,
//!     status: RegistrationStatus::Active,
//!     registration_time: Utc::now(),
//!     refresh_time: Some(Utc::now()),
//!     handle: None,
//! };
//!
//! assert_eq!(reg_info.status, RegistrationStatus::Active);
//! assert_eq!(reg_info.expires, 3600);
//! ```

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use rvoip_session_core::api::RegistrationHandle;

/// Registration configuration for SIP server registration
/// 
/// Contains all necessary parameters to register with a SIP registrar server,
/// including server details, user identity, authentication credentials,
/// and registration timing parameters.
/// 
/// # Examples
/// 
/// ```rust
/// use rvoip_client_core::registration::RegistrationConfig;
/// 
/// let config = RegistrationConfig::new(
///     "sip:registrar.example.com".to_string(),
///     "sip:alice@example.com".to_string(), 
///     "sip:alice@192.168.1.100:5060".to_string(),
/// )
/// .with_credentials("alice".to_string(), "password123".to_string())
/// .with_expires(1800);
/// 
/// assert_eq!(config.server_uri, "sip:registrar.example.com");
/// assert_eq!(config.username, Some("alice".to_string()));
/// assert_eq!(config.expires, 1800);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrationConfig {
    /// SIP registrar server URI (e.g., "sip:registrar.example.com")
    /// 
    /// The URI of the SIP registrar server where this client will register.
    /// This should include the scheme (sip: or sips:) and may include port.
    pub server_uri: String,
    
    /// From URI representing the user identity (e.g., "sip:alice@example.com")
    /// 
    /// The SIP address that identifies this user. This appears in the From
    /// header of REGISTER requests and represents the user's public identity.
    pub from_uri: String,
    
    /// Contact URI for this client (e.g., "sip:alice@192.168.1.100:5060")
    /// 
    /// The SIP URI where this client can be reached for incoming calls.
    /// Typically includes the client's current IP address and port.
    pub contact_uri: String,
    
    /// Registration expiration time in seconds
    /// 
    /// How long the registration should remain valid. The client will
    /// automatically refresh the registration before this time expires.
    /// Common values: 3600 (1 hour), 1800 (30 minutes), 300 (5 minutes).
    pub expires: u32,
    
    /// Authentication username (optional)
    /// 
    /// Username for SIP digest authentication. Required if the registrar
    /// server requires authentication (most production servers do).
    pub username: Option<String>,
    
    /// Authentication password (optional)
    /// 
    /// Password for SIP digest authentication. Should be kept secure
    /// and not logged or displayed in plain text.
    pub password: Option<String>,
    
    /// Authentication realm (optional)
    /// 
    /// Authentication realm provided by the server. Often the same as
    /// the domain portion of the server URI. Usually provided by the
    /// server in authentication challenges.
    pub realm: Option<String>,
}

impl RegistrationConfig {
    /// Create a new registration configuration with default settings
    /// 
    /// Creates a basic registration configuration with a default expiration
    /// of 3600 seconds (1 hour) and no authentication credentials.
    /// 
    /// # Arguments
    /// 
    /// * `server_uri` - URI of the SIP registrar server
    /// * `from_uri` - SIP URI representing the user identity
    /// * `contact_uri` - SIP URI where this client can be reached
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::registration::RegistrationConfig;
    /// 
    /// let config = RegistrationConfig::new(
    ///     "sip:registrar.example.com".to_string(),
    ///     "sip:alice@example.com".to_string(),
    ///     "sip:alice@192.168.1.100:5060".to_string(),
    /// );
    /// 
    /// assert_eq!(config.expires, 3600); // Default 1 hour
    /// assert_eq!(config.username, None); // No auth by default
    /// ```
    pub fn new(server_uri: String, from_uri: String, contact_uri: String) -> Self {
        Self {
            server_uri,
            from_uri,
            contact_uri,
            expires: 3600, // Default to 1 hour
            username: None,
            password: None,
            realm: None,
        }
    }
    
    /// Set authentication credentials for the registration
    /// 
    /// Configures username and password for SIP digest authentication.
    /// Most production SIP servers require authentication.
    /// 
    /// # Arguments
    /// 
    /// * `username` - Authentication username
    /// * `password` - Authentication password
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::registration::RegistrationConfig;
    /// 
    /// let config = RegistrationConfig::new(
    ///     "sip:registrar.example.com".to_string(),
    ///     "sip:alice@example.com".to_string(),
    ///     "sip:alice@192.168.1.100:5060".to_string(),
    /// )
    /// .with_credentials("alice".to_string(), "secret123".to_string());
    /// 
    /// assert_eq!(config.username, Some("alice".to_string()));
    /// assert_eq!(config.password, Some("secret123".to_string()));
    /// ```
    pub fn with_credentials(mut self, username: String, password: String) -> Self {
        self.username = Some(username);
        self.password = Some(password);
        self
    }
    
    /// Set the authentication realm for the registration
    /// 
    /// The realm is usually provided by the server during authentication
    /// challenges, but can be set in advance if known.
    /// 
    /// # Arguments
    /// 
    /// * `realm` - Authentication realm (often the server domain)
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::registration::RegistrationConfig;
    /// 
    /// let config = RegistrationConfig::new(
    ///     "sip:registrar.example.com".to_string(),
    ///     "sip:alice@example.com".to_string(),
    ///     "sip:alice@192.168.1.100:5060".to_string(),
    /// )
    /// .with_realm("example.com".to_string());
    /// 
    /// assert_eq!(config.realm, Some("example.com".to_string()));
    /// ```
    pub fn with_realm(mut self, realm: String) -> Self {
        self.realm = Some(realm);
        self
    }
    
    /// Set the registration expiration time
    /// 
    /// Controls how long the registration remains valid before requiring
    /// a refresh. Shorter times provide faster failover detection but
    /// increase network traffic.
    /// 
    /// # Arguments
    /// 
    /// * `expires` - Expiration time in seconds
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::registration::RegistrationConfig;
    /// 
    /// let config = RegistrationConfig::new(
    ///     "sip:registrar.example.com".to_string(),
    ///     "sip:alice@example.com".to_string(),
    ///     "sip:alice@192.168.1.100:5060".to_string(),
    /// )
    /// .with_expires(1800); // 30 minutes
    /// 
    /// assert_eq!(config.expires, 1800);
    /// ```
    pub fn with_expires(mut self, expires: u32) -> Self {
        self.expires = expires;
        self
    }
}

/// Current status of a SIP registration
/// 
/// Represents the various states a registration can be in during its lifecycle,
/// from initial setup through active registration to termination.
/// 
/// # State Transitions
/// 
/// Typical registration flow:
/// `Pending` → `Active` → `Expired`/`Failed`/`Cancelled`
/// 
/// # Examples
/// 
/// ```rust
/// use rvoip_client_core::registration::RegistrationStatus;
/// 
/// let status = RegistrationStatus::Active;
/// println!("Status: {}", status); // Prints "Status: Active"
/// 
/// assert_eq!(status, RegistrationStatus::Active);
/// assert_ne!(status, RegistrationStatus::Failed);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegistrationStatus {
    /// Registration request has been sent but no response received yet
    /// 
    /// Initial state when a REGISTER request is sent to the server.
    /// The client is waiting for the server's response.
    Pending,
    
    /// Registration is active and valid
    /// 
    /// The server has accepted the registration and it's currently valid.
    /// The client can receive incoming calls and the registration will
    /// be refreshed automatically before expiration.
    Active,
    
    /// Registration failed
    /// 
    /// The server rejected the registration request, typically due to
    /// authentication failure, invalid credentials, or server error.
    /// This is a terminal state that requires manual intervention.
    Failed,
    
    /// Registration expired
    /// 
    /// The registration validity period has elapsed and the registration
    /// is no longer active. This can happen if refresh attempts fail
    /// or if the client is disconnected for too long.
    Expired,
    
    /// Registration was cancelled by the client
    /// 
    /// The client explicitly unregistered by sending a REGISTER request
    /// with Expires: 0. This is the normal way to end a registration.
    Cancelled,
}

impl std::fmt::Display for RegistrationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistrationStatus::Pending => write!(f, "Pending"),
            RegistrationStatus::Active => write!(f, "Active"),
            RegistrationStatus::Failed => write!(f, "Failed"),
            RegistrationStatus::Expired => write!(f, "Expired"),
            RegistrationStatus::Cancelled => write!(f, "Cancelled"),
        }
    }
}

/// Comprehensive information about a SIP registration
/// 
/// Contains all details about a registration including configuration,
/// current status, timing information, and technical handles.
/// 
/// # Examples
/// 
/// ```rust
/// use rvoip_client_core::registration::{RegistrationInfo, RegistrationStatus};
/// use chrono::Utc;
/// use uuid::Uuid;
/// 
/// let reg_info = RegistrationInfo {
///     id: Uuid::new_v4(),
///     server_uri: "sip:registrar.example.com".to_string(),
///     from_uri: "sip:alice@example.com".to_string(),
///     contact_uri: "sip:alice@192.168.1.100:5060".to_string(),
///     expires: 3600,
///     status: RegistrationStatus::Active,
///     registration_time: Utc::now(),
///     refresh_time: Some(Utc::now()),
///     handle: None,
/// };
/// 
/// assert_eq!(reg_info.status, RegistrationStatus::Active);
/// assert_eq!(reg_info.server_uri, "sip:registrar.example.com");
/// ```
#[derive(Debug, Clone)]
pub struct RegistrationInfo {
    /// Unique registration identifier assigned by the client
    pub id: Uuid,
    
    /// SIP registrar server URI where this registration is active
    pub server_uri: String,
    
    /// From URI representing the registered user identity
    pub from_uri: String,
    
    /// Contact URI where this client can be reached for incoming calls
    pub contact_uri: String,
    
    /// Registration expiration time in seconds from last refresh
    pub expires: u32,
    
    /// Current status of this registration
    pub status: RegistrationStatus,
    
    /// When the registration was initially created
    pub registration_time: DateTime<Utc>,
    
    /// When the registration was last successfully refreshed (if applicable)
    pub refresh_time: Option<DateTime<Utc>>,
    
    /// Internal handle to the session-core registration (if active)
    pub handle: Option<RegistrationHandle>,
}

/// Statistics about SIP registrations in the system
/// 
/// Provides aggregate counts and metrics about registrations, useful for
/// monitoring, debugging, and user interface displays.
/// 
/// # Examples
/// 
/// ```rust
/// use rvoip_client_core::registration::RegistrationStats;
/// 
/// let stats = RegistrationStats {
///     total_registrations: 5,
///     active_registrations: 3,
///     failed_registrations: 1,
/// };
/// 
/// assert_eq!(stats.total_registrations, 5);
/// assert_eq!(stats.active_registrations, 3);
/// assert_eq!(stats.failed_registrations, 1);
/// 
/// // Calculate derived metrics
/// let pending_registrations = stats.total_registrations - stats.active_registrations - stats.failed_registrations;
/// assert_eq!(pending_registrations, 1);
/// ```
#[derive(Debug, Clone)]
pub struct RegistrationStats {
    /// Total number of registrations (all statuses)
    pub total_registrations: usize,
    /// Number of registrations in Active status
    pub active_registrations: usize,
    /// Number of registrations in Failed status
    pub failed_registrations: usize,
} 