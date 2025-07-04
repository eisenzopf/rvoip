//! Configuration for Dialog-Core API
//!
//! This module provides configuration types for dialog-core API operations,
//! supporting both simple use cases and advanced customization scenarios.
//!
//! ## Quick Start
//!
//! ### Simple Client Configuration
//!
//! ```rust
//! use rvoip_dialog_core::api::ClientConfig;
//!
//! // Basic client setup
//! let config = ClientConfig::new("127.0.0.1:0".parse().unwrap())
//!     .with_from_uri("sip:alice@example.com");
//!
//! println!("Client will bind to: {}", config.dialog.local_address);
//! println!("Default From URI: {}", config.from_uri.unwrap());
//! ```
//!
//! ### Simple Server Configuration
//!
//! ```rust
//! use rvoip_dialog_core::api::ServerConfig;
//!
//! // Basic server setup
//! let config = ServerConfig::new("0.0.0.0:5060".parse().unwrap())
//!     .with_domain("sip.example.com")
//!     .with_auto_options();
//!
//! println!("Server will listen on: {}", config.dialog.local_address);
//! println!("Server domain: {}", config.domain.unwrap());
//! println!("Auto OPTIONS: {}", config.auto_options_response);
//! ```
//!
//! ## Configuration Architecture
//!
//! The configuration system is hierarchical:
//!
//! ```text
//! ClientConfig / ServerConfig
//!        │
//!        └── DialogConfig (shared settings)
//!             │
//!             ├── Network settings (local_address)
//!             ├── Protocol settings (user_agent, timeouts)
//!             ├── Resource limits (max_dialogs)
//!             └── Cleanup behavior (auto_cleanup, intervals)
//! ```
//!
//! ## Advanced Configuration Examples
//!
//! ### Production Client Configuration
//!
//! ```rust
//! use rvoip_dialog_core::api::ClientConfig;
//! use std::time::Duration;
//!
//! let config = ClientConfig::new("192.168.1.100:5060".parse().unwrap())
//!     .with_from_uri("sip:user@domain.com")
//!     .with_auth("username", "secret_password");
//!
//! // Customize dialog behavior for production
//! let mut prod_config = config;
//! prod_config.dialog = prod_config.dialog
//!     .with_timeout(Duration::from_secs(300))        // 5 minute timeout
//!     .with_max_dialogs(50000)                       // High capacity
//!     .with_user_agent("MyApp/2.1.0 (Production)"); // Custom UA
//!
//! // Validate before use
//! assert!(prod_config.validate().is_ok());
//! ```
//!
//! ### High-Performance Server Configuration
//!
//! ```rust
//! use rvoip_dialog_core::api::ServerConfig;
//! use std::time::Duration;
//!
//! let mut config = ServerConfig::new("0.0.0.0:5060".parse().unwrap())
//!     .with_domain("sip.mycompany.com")
//!     .with_auto_options()
//!     .with_auto_register();
//!
//! // Optimize for high-performance scenarios
//! config.dialog = config.dialog
//!     .with_timeout(Duration::from_secs(120))        // Shorter timeout
//!     .with_max_dialogs(100000)                      // Very high capacity
//!     .without_auto_cleanup();                       // Manual cleanup control
//!
//! assert!(config.validate().is_ok());
//! ```
//!
//! ### Development/Testing Configuration
//!
//! ```rust
//! use rvoip_dialog_core::api::{ClientConfig, ServerConfig};
//! use std::time::Duration;
//!
//! // Quick development setup
//! let dev_client = ClientConfig::new("127.0.0.1:0".parse().unwrap())
//!     .with_from_uri("sip:test@localhost");
//!
//! let dev_server = ServerConfig::new("127.0.0.1:5060".parse().unwrap())
//!     .with_domain("localhost")
//!     .with_auto_options();
//!
//! // Both use fast timeouts for testing
//! let mut quick_config = dev_client;
//! quick_config.dialog = quick_config.dialog
//!     .with_timeout(Duration::from_secs(30));        // Fast timeout for tests
//! ```

use std::net::SocketAddr;
use std::time::Duration;
use serde::{Deserialize, Serialize};

/// Main configuration for dialog operations
///
/// This is the foundational configuration struct that controls core dialog
/// behavior including network settings, timeouts, resource limits, and cleanup.
/// It's embedded in both ClientConfig and ServerConfig to provide consistent
/// dialog management across client and server deployments.
///
/// ## Examples
///
/// ### Basic Usage
///
/// ```rust
/// use rvoip_dialog_core::api::DialogConfig;
/// use std::time::Duration;
///
/// let config = DialogConfig::new("127.0.0.1:5060".parse().unwrap())
///     .with_timeout(Duration::from_secs(180))
///     .with_max_dialogs(1000)
///     .with_user_agent("MyApp/1.0");
///
/// assert_eq!(config.dialog_timeout, Duration::from_secs(180));
/// assert_eq!(config.max_dialogs, Some(1000));
/// ```
///
/// ### Resource Management
///
/// ```rust
/// use rvoip_dialog_core::api::DialogConfig;
/// use std::time::Duration;
///
/// // Configure for high-load scenarios
/// let high_load_config = DialogConfig::new("0.0.0.0:5060".parse().unwrap())
///     .with_max_dialogs(50000)                       // High capacity
///     .with_timeout(Duration::from_secs(120))        // Aggressive timeout
///     .without_auto_cleanup();                       // Manual control
///
/// assert_eq!(high_load_config.max_dialogs, Some(50000));
/// assert_eq!(high_load_config.auto_cleanup, false);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DialogConfig {
    /// Local address for SIP communication
    ///
    /// The socket address that this dialog instance will bind to for
    /// sending and receiving SIP messages. Use "0.0.0.0:5060" for servers
    /// that should accept connections from any interface, or specific
    /// addresses like "192.168.1.100:5060" for targeted binding.
    pub local_address: SocketAddr,
    
    /// User agent string to include in SIP messages
    ///
    /// This appears in the User-Agent header of outgoing SIP requests
    /// and Server header of outgoing responses. Useful for debugging
    /// and identifying your application in SIP traces.
    pub user_agent: Option<String>,
    
    /// Default timeout for dialog operations
    ///
    /// Maximum time to wait for dialog-related operations to complete.
    /// This includes waiting for responses to requests and dialog
    /// establishment timeouts. Shorter timeouts free up resources faster
    /// but may cause premature failures on slow networks.
    pub dialog_timeout: Duration,
    
    /// Maximum number of concurrent dialogs
    ///
    /// Limits the total number of dialogs that can be active simultaneously.
    /// This prevents resource exhaustion attacks and helps manage memory
    /// usage. Set to None for unlimited dialogs (not recommended for production).
    pub max_dialogs: Option<usize>,
    
    /// Enable automatic dialog cleanup
    ///
    /// When true, terminated dialogs are automatically cleaned up at
    /// regular intervals. When false, cleanup must be performed manually
    /// which gives more control but requires careful resource management.
    pub auto_cleanup: bool,
    
    /// Cleanup interval for terminated dialogs
    ///
    /// How often to run the automatic cleanup process for terminated
    /// dialogs. Shorter intervals keep memory usage low but use more CPU.
    /// Longer intervals are more efficient but allow terminated dialogs
    /// to accumulate temporarily.
    pub cleanup_interval: Duration,
}

impl Default for DialogConfig {
    fn default() -> Self {
        Self {
            local_address: "0.0.0.0:5060".parse().unwrap(),
            user_agent: Some("RVOIP-Dialog/1.0".to_string()),
            dialog_timeout: Duration::from_secs(180), // 3 minutes
            max_dialogs: Some(10000),
            auto_cleanup: true,
            cleanup_interval: Duration::from_secs(30),
        }
    }
}

impl DialogConfig {
    /// Create a new configuration with a specific local address
    ///
    /// Creates a DialogConfig with the specified local address and
    /// sensible defaults for all other settings.
    ///
    /// # Arguments
    /// * `local_address` - Socket address to bind to
    ///
    /// # Returns
    /// New DialogConfig with defaults
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_dialog_core::api::DialogConfig;
    ///
    /// let config = DialogConfig::new("192.168.1.100:5060".parse().unwrap());
    /// assert_eq!(config.local_address.port(), 5060);
    /// assert!(config.user_agent.is_some());
    /// assert_eq!(config.auto_cleanup, true);
    /// ```
    pub fn new(local_address: SocketAddr) -> Self {
        Self {
            local_address,
            ..Default::default()
        }
    }
    
    /// Set the user agent string
    ///
    /// Configures the User-Agent header that will be included in outgoing
    /// SIP requests. This is useful for identifying your application in
    /// SIP logs and debugging scenarios.
    ///
    /// # Arguments
    /// * `user_agent` - User agent string (e.g., "MyApp/1.0")
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_dialog_core::api::DialogConfig;
    ///
    /// let config = DialogConfig::new("127.0.0.1:5060".parse().unwrap())
    ///     .with_user_agent("CustomApp/2.1.0 (Linux)");
    ///
    /// assert_eq!(config.user_agent.unwrap(), "CustomApp/2.1.0 (Linux)");
    /// ```
    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = Some(user_agent.into());
        self
    }
    
    /// Set the dialog timeout
    ///
    /// Configures how long to wait for dialog operations to complete.
    /// This affects response timeouts, connection establishment, and
    /// other dialog lifecycle operations.
    ///
    /// # Arguments
    /// * `timeout` - Maximum time to wait for operations
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_dialog_core::api::DialogConfig;
    /// use std::time::Duration;
    ///
    /// let config = DialogConfig::new("127.0.0.1:5060".parse().unwrap())
    ///     .with_timeout(Duration::from_secs(60));
    ///
    /// assert_eq!(config.dialog_timeout, Duration::from_secs(60));
    /// ```
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.dialog_timeout = timeout;
        self
    }
    
    /// Set the maximum number of dialogs
    ///
    /// Limits the total number of simultaneous dialogs to prevent
    /// resource exhaustion. Essential for production deployments.
    ///
    /// # Arguments
    /// * `max` - Maximum number of concurrent dialogs
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_dialog_core::api::DialogConfig;
    ///
    /// let config = DialogConfig::new("127.0.0.1:5060".parse().unwrap())
    ///     .with_max_dialogs(5000);
    ///
    /// assert_eq!(config.max_dialogs, Some(5000));
    /// ```
    pub fn with_max_dialogs(mut self, max: usize) -> Self {
        self.max_dialogs = Some(max);
        self
    }
    
    /// Disable automatic cleanup
    ///
    /// Turns off automatic cleanup of terminated dialogs, giving the
    /// application full control over resource management. Requires
    /// manual cleanup to prevent memory leaks.
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_dialog_core::api::DialogConfig;
    ///
    /// let config = DialogConfig::new("127.0.0.1:5060".parse().unwrap())
    ///     .without_auto_cleanup();
    ///
    /// assert_eq!(config.auto_cleanup, false);
    /// ```
    pub fn without_auto_cleanup(mut self) -> Self {
        self.auto_cleanup = false;
        self
    }
    
    /// Validate the configuration
    ///
    /// Checks that all configuration values are valid and consistent.
    /// Should be called before using the configuration to create dialog
    /// managers or API instances.
    ///
    /// # Returns
    /// Ok(()) if valid, Err(message) if invalid
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_dialog_core::api::DialogConfig;
    /// use std::time::Duration;
    ///
    /// let valid_config = DialogConfig::new("127.0.0.1:5060".parse().unwrap());
    /// assert!(valid_config.validate().is_ok());
    ///
    /// let invalid_config = DialogConfig::new("127.0.0.1:5060".parse().unwrap())
    ///     .with_timeout(Duration::from_secs(0));  // Invalid timeout
    /// assert!(invalid_config.validate().is_err());
    /// ```
    pub fn validate(&self) -> Result<(), String> {
        if self.dialog_timeout.as_secs() == 0 {
            return Err("Dialog timeout must be greater than 0".to_string());
        }
        
        if let Some(max) = self.max_dialogs {
            if max == 0 {
                return Err("Max dialogs must be greater than 0".to_string());
            }
        }
        
        if self.cleanup_interval.as_secs() == 0 {
            return Err("Cleanup interval must be greater than 0".to_string());
        }
        
        Ok(())
    }
}

/// Server-specific configuration
///
/// Extends DialogConfig with server-specific settings including automatic
/// response handling, domain configuration, and server behavior options.
/// Used by DialogServer to configure server-side SIP operations.
///
/// ## Examples
///
/// ### Basic Server Setup
///
/// ```rust
/// use rvoip_dialog_core::api::ServerConfig;
///
/// let config = ServerConfig::new("0.0.0.0:5060".parse().unwrap())
///     .with_domain("sip.example.com")
///     .with_auto_options();
///
/// assert_eq!(config.domain.unwrap(), "sip.example.com");
/// assert!(config.auto_options_response);
/// ```
///
/// ### Production Server Configuration
///
/// ```rust
/// use rvoip_dialog_core::api::ServerConfig;
/// use std::time::Duration;
///
/// let mut config = ServerConfig::new("0.0.0.0:5060".parse().unwrap())
///     .with_domain("sip.production.com")
///     .with_auto_options()
///     .with_auto_register();
///
/// // Customize for production load
/// config.dialog = config.dialog
///     .with_timeout(Duration::from_secs(300))
///     .with_max_dialogs(100000)
///     .with_user_agent("ProductionSIP/1.0");
///
/// assert!(config.validate().is_ok());
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Base dialog configuration
    pub dialog: DialogConfig,
    
    /// Enable automatic response to OPTIONS requests
    ///
    /// When true, the server automatically responds to OPTIONS requests
    /// with a 200 OK including supported methods. When false, OPTIONS
    /// requests are forwarded to the application for custom handling.
    pub auto_options_response: bool,
    
    /// Enable automatic response to REGISTER requests
    ///
    /// When true, the server automatically handles REGISTER requests
    /// for basic registration functionality. When false, REGISTER
    /// requests are forwarded to the application.
    pub auto_register_response: bool,
    
    /// Server domain name
    ///
    /// The domain name this server represents. Used in Contact headers
    /// and for routing decisions. Should match your SIP domain.
    pub domain: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            dialog: DialogConfig::default(),
            auto_options_response: true,
            auto_register_response: false,
            domain: None,
        }
    }
}

impl ServerConfig {
    /// Create a new server configuration with a local address
    ///
    /// Creates a ServerConfig with the specified local address and
    /// sensible defaults for server operation.
    ///
    /// # Arguments
    /// * `local_address` - Address to bind the server to
    ///
    /// # Returns
    /// New ServerConfig with defaults
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_dialog_core::api::ServerConfig;
    ///
    /// let config = ServerConfig::new("0.0.0.0:5060".parse().unwrap());
    /// assert_eq!(config.dialog.local_address.port(), 5060);
    /// assert!(config.auto_options_response);  // Default enabled
    /// assert!(!config.auto_register_response); // Default disabled
    /// ```
    pub fn new(local_address: SocketAddr) -> Self {
        Self {
            dialog: DialogConfig::new(local_address),
            ..Default::default()
        }
    }
    
    /// Set the server domain
    ///
    /// Configures the domain name that this server represents.
    /// Used for Contact header generation and routing decisions.
    ///
    /// # Arguments
    /// * `domain` - Server domain name (e.g., "sip.example.com")
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_dialog_core::api::ServerConfig;
    ///
    /// let config = ServerConfig::new("0.0.0.0:5060".parse().unwrap())
    ///     .with_domain("sip.mycompany.com");
    ///
    /// assert_eq!(config.domain.unwrap(), "sip.mycompany.com");
    /// ```
    pub fn with_domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = Some(domain.into());
        self
    }
    
    /// Enable automatic OPTIONS response
    ///
    /// Configures the server to automatically respond to OPTIONS requests
    /// with a 200 OK including supported SIP methods. This is useful for
    /// capability discovery and keep-alive mechanisms.
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_dialog_core::api::ServerConfig;
    ///
    /// let config = ServerConfig::new("0.0.0.0:5060".parse().unwrap())
    ///     .with_auto_options();
    ///
    /// assert!(config.auto_options_response);
    /// ```
    pub fn with_auto_options(mut self) -> Self {
        self.auto_options_response = true;
        self
    }
    
    /// Enable automatic REGISTER response
    ///
    /// Configures the server to automatically handle REGISTER requests
    /// for basic SIP registration functionality. Use this for simple
    /// registrar services or disable for custom registration handling.
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_dialog_core::api::ServerConfig;
    ///
    /// let config = ServerConfig::new("0.0.0.0:5060".parse().unwrap())
    ///     .with_auto_register();
    ///
    /// assert!(config.auto_register_response);
    /// ```
    pub fn with_auto_register(mut self) -> Self {
        self.auto_register_response = true;
        self
    }
    
    /// Validate the server configuration
    ///
    /// Validates both the server-specific settings and the underlying
    /// dialog configuration to ensure everything is properly configured.
    ///
    /// # Returns
    /// Ok(()) if valid, Err(message) if invalid
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_dialog_core::api::ServerConfig;
    ///
    /// let config = ServerConfig::new("0.0.0.0:5060".parse().unwrap())
    ///     .with_domain("test.com");
    ///
    /// assert!(config.validate().is_ok());
    /// ```
    pub fn validate(&self) -> Result<(), String> {
        self.dialog.validate()
    }
}

/// Client-specific configuration
///
/// Extends DialogConfig with client-specific settings including default
/// URI configuration, authentication settings, and client behavior options.
/// Used by DialogClient to configure client-side SIP operations.
///
/// ## Examples
///
/// ### Basic Client Setup
///
/// ```rust
/// use rvoip_dialog_core::api::ClientConfig;
///
/// let config = ClientConfig::new("127.0.0.1:0".parse().unwrap())
///     .with_from_uri("sip:alice@example.com");
///
/// assert_eq!(config.from_uri.unwrap(), "sip:alice@example.com");
/// assert!(!config.auto_auth);  // Default disabled
/// ```
///
/// ### Client with Authentication
///
/// ```rust
/// use rvoip_dialog_core::api::ClientConfig;
///
/// let config = ClientConfig::new("192.168.1.100:5060".parse().unwrap())
///     .with_from_uri("sip:user@domain.com")
///     .with_auth("username", "password");
///
/// assert!(config.auto_auth);
/// assert!(config.credentials.is_some());
/// assert!(config.validate().is_ok());
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    /// Base dialog configuration
    pub dialog: DialogConfig,
    
    /// Default from URI for outgoing requests
    ///
    /// The URI that will be used in the From header of outgoing requests
    /// when not explicitly specified. Should represent the client's
    /// SIP identity (e.g., "sip:alice@example.com").
    pub from_uri: Option<String>,
    
    /// Enable automatic authentication
    ///
    /// When true, the client automatically handles 401/407 authentication
    /// challenges using the configured credentials. When false,
    /// authentication challenges are forwarded to the application.
    pub auto_auth: bool,
    
    /// Default credentials for authentication
    ///
    /// Username and password used for automatic authentication when
    /// auto_auth is enabled. Should be set when enabling auto_auth.
    pub credentials: Option<Credentials>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            dialog: DialogConfig::default(),
            from_uri: None,
            auto_auth: false,
            credentials: None,
        }
    }
}

impl ClientConfig {
    /// Create a new client configuration with a local address
    ///
    /// Creates a ClientConfig with the specified local address and
    /// sensible defaults for client operation.
    ///
    /// # Arguments
    /// * `local_address` - Local address for the client to bind to
    ///
    /// # Returns
    /// New ClientConfig with defaults
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_dialog_core::api::ClientConfig;
    ///
    /// let config = ClientConfig::new("127.0.0.1:0".parse().unwrap());
    /// assert_eq!(config.dialog.local_address.ip().to_string(), "127.0.0.1");
    /// assert!(!config.auto_auth);  // Default disabled
    /// ```
    pub fn new(local_address: SocketAddr) -> Self {
        Self {
            dialog: DialogConfig::new(local_address),
            ..Default::default()
        }
    }
    
    /// Set the default from URI
    ///
    /// Configures the default From header URI for outgoing requests.
    /// This should represent the client's SIP identity.
    ///
    /// # Arguments
    /// * `from_uri` - SIP URI (e.g., "sip:alice@example.com")
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_dialog_core::api::ClientConfig;
    ///
    /// let config = ClientConfig::new("127.0.0.1:0".parse().unwrap())
    ///     .with_from_uri("sip:test@localhost");
    ///
    /// assert_eq!(config.from_uri.unwrap(), "sip:test@localhost");
    /// ```
    pub fn with_from_uri(mut self, from_uri: impl Into<String>) -> Self {
        self.from_uri = Some(from_uri.into());
        self
    }
    
    /// Enable automatic authentication with credentials
    ///
    /// Configures the client to automatically handle SIP authentication
    /// challenges using the provided username and password.
    ///
    /// # Arguments
    /// * `username` - Authentication username
    /// * `password` - Authentication password
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_dialog_core::api::ClientConfig;
    ///
    /// let config = ClientConfig::new("127.0.0.1:0".parse().unwrap())
    ///     .with_auth("user123", "secret456");
    ///
    /// assert!(config.auto_auth);
    /// assert!(config.credentials.is_some());
    /// assert_eq!(config.credentials.unwrap().username, "user123");
    /// ```
    pub fn with_auth(mut self, username: impl Into<String>, password: impl Into<String>) -> Self {
        self.auto_auth = true;
        self.credentials = Some(Credentials {
            username: username.into(),
            password: password.into(),
            realm: None,
        });
        self
    }
    
    /// Validate the client configuration
    ///
    /// Validates both the client-specific settings and the underlying
    /// dialog configuration to ensure everything is properly configured.
    ///
    /// # Returns
    /// Ok(()) if valid, Err(message) if invalid
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_dialog_core::api::ClientConfig;
    ///
    /// let valid_config = ClientConfig::new("127.0.0.1:0".parse().unwrap())
    ///     .with_from_uri("sip:test@example.com");
    /// assert!(valid_config.validate().is_ok());
    ///
    /// let mut invalid_config = ClientConfig::new("127.0.0.1:0".parse().unwrap())
    ///     .with_auth("user", "pass");
    /// // Note: with_auth() actually sets up credentials properly,
    /// // so this example shows a working auth configuration
    /// assert!(invalid_config.validate().is_ok());
    /// ```
    pub fn validate(&self) -> Result<(), String> {
        self.dialog.validate()?;
        
        if self.auto_auth && self.credentials.is_none() {
            return Err("Auto auth enabled but no credentials provided".to_string());
        }
        
        Ok(())
    }
}

/// Authentication credentials
///
/// Stores username, password, and optional realm for SIP authentication.
/// Used with ClientConfig when auto_auth is enabled to automatically
/// respond to authentication challenges.
///
/// ## Examples
///
/// ### Basic Credentials
///
/// ```rust
/// use rvoip_dialog_core::api::config::Credentials;
///
/// let creds = Credentials::new("alice", "secret123");
/// assert_eq!(creds.username, "alice");
/// assert_eq!(creds.password, "secret123");
/// assert!(creds.realm.is_none());
/// ```
///
/// ### Credentials with Realm
///
/// ```rust
/// use rvoip_dialog_core::api::config::Credentials;
///
/// let creds = Credentials::new("bob", "password456")
///     .with_realm("example.com");
/// assert_eq!(creds.realm.unwrap(), "example.com");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    /// Username for authentication
    pub username: String,
    
    /// Password for authentication  
    pub password: String,
    
    /// Realm (optional, will be extracted from challenge if not provided)
    ///
    /// The authentication realm. If not provided, it will be extracted
    /// from the authentication challenge. Setting it explicitly can be
    /// useful for pre-configured authentication scenarios.
    pub realm: Option<String>,
}

impl Credentials {
    /// Create new credentials
    ///
    /// Creates credentials with the specified username and password.
    /// The realm will be extracted from authentication challenges if needed.
    ///
    /// # Arguments
    /// * `username` - Authentication username
    /// * `password` - Authentication password
    ///
    /// # Returns
    /// New Credentials instance
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_dialog_core::api::config::Credentials;
    ///
    /// let creds = Credentials::new("testuser", "testpass");
    /// assert_eq!(creds.username, "testuser");
    /// assert_eq!(creds.password, "testpass");
    /// ```
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            password: password.into(),
            realm: None,
        }
    }
    
    /// Set the realm
    ///
    /// Configures a specific realm for authentication. Useful when you
    /// know the realm in advance or want to restrict authentication to
    /// a specific realm.
    ///
    /// # Arguments
    /// * `realm` - Authentication realm
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_dialog_core::api::config::Credentials;
    ///
    /// let creds = Credentials::new("user", "pass")
    ///     .with_realm("secure.example.com");
    /// assert_eq!(creds.realm.unwrap(), "secure.example.com");
    /// ```
    pub fn with_realm(mut self, realm: impl Into<String>) -> Self {
        self.realm = Some(realm.into());
        self
    }
} 