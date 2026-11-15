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
//! use rvoip_sip_dialog::api::ClientConfig;
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
//! use rvoip_sip_dialog::api::ServerConfig;
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
//! use rvoip_sip_dialog::api::ClientConfig;
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
//! use rvoip_sip_dialog::api::ServerConfig;
//! use std::time::Duration;
//!
//! let mut config = ServerConfig::new("0.0.0.0:5060".parse().unwrap())
//!     .with_domain("sip.mycompany.com")
//!     .with_auto_options();
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
//! use rvoip_sip_dialog::api::{ClientConfig, ServerConfig};
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

use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::SocketAddr;
use std::time::Duration;

/// Maximum number of [`SipTlsProxyFlowPolicy`] entries a [`DialogConfig`]
/// accepts.
pub const MAX_TLS_PROXY_FLOW_POLICIES: usize = 64;

/// Same-dialog TLS flow reuse for one trusted record-routing proxy.
///
/// A contact center or gateway that sits behind a carrier's record-routing
/// SIP proxy usually receives the carrier's INVITE on a TLS connection the
/// proxy opened, with a `Record-Route` naming the proxy's logical TLS
/// listener. RFC 3261 sends every later in-dialog request (BYE, re-INVITE,
/// UPDATE, INFO) to that Record-Route, so by default this stack dials a new
/// TLS connection to the listener. Many carrier proxies only accept requests
/// for a dialog on the connection they opened, and the listener's certificate
/// is often issued for a DNS name rather than the IP address that appears in
/// the Record-Route, so the new connection fails or the request is rejected.
///
/// A policy lets in-dialog requests go back on the accepted TLS flow instead.
/// The flow is bound to an inbound dialog only when all of the following
/// hold:
///
/// - the dialog-forming INVITE was admitted with an authenticated principal
///   (see `rvoip-sip`'s `SipListenerAuthPolicy`) whose `issuer`, `tenant`
///   and `subject` equal this policy's, and which has not expired;
/// - the INVITE arrived over TLS, on an identified flow, from
///   `proxy_endpoint`'s IP address;
/// - the dialog's first route is a loose-routing (`;lr`) TLS URI whose host
///   is an IP literal and whose host and port are exactly `proxy_endpoint`
///   (`sip:` URIs need `transport=tls`; a `sips:` URI without a port means
///   5061).
///
/// Only a request whose next hop is still exactly that route uses the flow.
/// The flow is never inferred from a SIP header or source address alone,
/// and this is not an RFC 5923 connection-alias table: no other dialog or
/// request may use it. If the flow has closed, the request fails rather than
/// dialling the proxy, unless [`Self::renewal_authority`] is set.
///
/// Policies are configured only through [`DialogConfig::tls_proxy_flow_policies`]
/// (or `rvoip_sip::Config::sip_tls_proxy_flow_policies`); the default is none,
/// which leaves routing unchanged.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_dialog::api::config::SipTlsProxyFlowPolicy;
///
/// let policy = SipTlsProxyFlowPolicy::new(
///     "listener-trusted-cidr",
///     "carrier-tenant",
///     "carrier-proxy",
///     "192.0.2.10:5061".parse().unwrap(),
/// )
/// .with_renewal_authority("sip.carrier.example");
/// assert!(policy.validate().is_ok());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct SipTlsProxyFlowPolicy {
    /// Exact `AuthenticatedPrincipal::issuer` of the dialog-forming INVITE.
    pub issuer: String,
    /// Exact `AuthenticatedPrincipal::tenant` of the dialog-forming INVITE.
    pub tenant: String,
    /// Exact `AuthenticatedPrincipal::subject` of the dialog-forming INVITE.
    pub subject: String,
    /// The proxy's logical TLS listener, as it appears in Record-Route. Its
    /// IP address must also be the source address of the accepted flow; the
    /// flow's source port may differ.
    pub proxy_endpoint: SocketAddr,
    /// DNS name the proxy's certificate is verified against when the
    /// accepted flow has closed. When set, a request for a bound dialog whose
    /// flow has closed opens one new certificate-verified TLS connection to
    /// `proxy_endpoint`, verified against this name, and the dialog then
    /// keeps using that connection. `None` (the default) makes such a request
    /// fail instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renewal_authority: Option<String>,
}

impl SipTlsProxyFlowPolicy {
    /// Create a policy for the proxy at `proxy_endpoint`, trusted when the
    /// dialog-forming INVITE's principal has exactly this `issuer`, `tenant`
    /// and `subject`. Renewal is off; see [`Self::with_renewal_authority`].
    pub fn new(
        issuer: impl Into<String>,
        tenant: impl Into<String>,
        subject: impl Into<String>,
        proxy_endpoint: SocketAddr,
    ) -> Self {
        Self {
            issuer: issuer.into(),
            tenant: tenant.into(),
            subject: subject.into(),
            proxy_endpoint,
            renewal_authority: None,
        }
    }

    /// Allow a closed flow to be replaced by a new TLS connection to
    /// `proxy_endpoint` whose certificate is verified against `name`.
    pub fn with_renewal_authority(mut self, name: impl Into<String>) -> Self {
        self.renewal_authority = Some(name.into());
        self
    }

    /// Check the policy.
    ///
    /// `issuer`, `tenant` and `subject` must be 1 to 128 characters with no
    /// surrounding whitespace or control characters. `proxy_endpoint` must
    /// have a nonzero port and a unicast, specified IP address.
    /// `renewal_authority`, when set, must be a DNS name (not an IP
    /// literal) of at most 253 characters made of 1 to 63 character
    /// letter/digit/hyphen labels that neither start nor end with a hyphen.
    pub fn validate(&self) -> Result<(), String> {
        let valid_identity = |value: &str| {
            !value.is_empty()
                && value.len() <= 128
                && value.trim() == value
                && !value.chars().any(char::is_control)
        };
        if !valid_identity(&self.issuer)
            || !valid_identity(&self.tenant)
            || !valid_identity(&self.subject)
        {
            return Err(
                "TLS proxy-flow policy issuer, tenant and subject must be 1-128 characters \
                 without surrounding whitespace or control characters"
                    .to_string(),
            );
        }
        if self.proxy_endpoint.port() == 0
            || self.proxy_endpoint.ip().is_unspecified()
            || self.proxy_endpoint.ip().is_multicast()
        {
            return Err(
                "TLS proxy-flow policy proxy_endpoint must be a unicast address with a nonzero port"
                    .to_string(),
            );
        }
        if let Some(authority) = &self.renewal_authority {
            let valid = !authority.is_empty()
                && authority.len() <= 253
                && authority.parse::<std::net::IpAddr>().is_err()
                && authority.split('.').all(|label| {
                    !label.is_empty()
                        && label.len() <= 63
                        && !label.starts_with('-')
                        && !label.ends_with('-')
                        && label
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                });
            if !valid {
                return Err(
                    "TLS proxy-flow policy renewal_authority must be a DNS name".to_string()
                );
            }
        }
        Ok(())
    }
}

/// Policy for the SIP `100rel` reliable-provisional-response extension
/// (RFC 3262).
///
/// Controls how outgoing INVITEs advertise (or demand) reliable provisional
/// responses. Inbound 18x with `Require: 100rel` are auto-PRACKed regardless
/// of this setting; this only affects what we put on our own outgoing INVITE.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelUsage {
    /// Don't advertise 100rel at all. Carriers that mandate reliable 18x will
    /// reject the INVITE.
    NotSupported,
    /// Advertise via `Supported: 100rel`. The peer may use reliable 18x but
    /// is not required to. This is the safe default.
    Supported,
    /// Demand via `Require: 100rel`. The peer must support 100rel or the
    /// call setup fails with 420 Bad Extension.
    Required,
}

impl Default for RelUsage {
    fn default() -> Self {
        RelUsage::Supported
    }
}

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
/// use rvoip_sip_dialog::api::DialogConfig;
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
/// use rvoip_sip_dialog::api::DialogConfig;
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
#[non_exhaustive]
pub struct DialogConfig {
    /// Local bind address for SIP communication
    ///
    /// The socket address that this dialog instance will bind to for
    /// sending and receiving SIP messages. Use "0.0.0.0:5060" for servers
    /// that should accept connections from any interface, or specific
    /// addresses like "192.168.1.100:5060" for targeted binding.
    pub local_address: SocketAddr,

    /// Optional SIP address to advertise in Via sent-by and fallback
    /// Contact generation for non-TLS requests. When unset, dialog-core
    /// preserves the legacy behavior of using `local_address`.
    #[serde(default)]
    pub advertised_local_address: Option<SocketAddr>,

    /// Optional local Contact URI to advertise in dialog-creating and
    /// target-refresh messages. When unset, dialog-core falls back to
    /// method-specific defaults derived from the local address.
    #[serde(default)]
    pub local_contact_uri: Option<String>,

    /// Optional local bind address for SIP TLS listener mode. When unset,
    /// TLS listener modes derive their legacy bind address from
    /// `local_address`.
    #[serde(default)]
    pub tls_local_address: Option<SocketAddr>,

    /// Optional SIP TLS address to advertise in Via sent-by and fallback
    /// Contact generation. When unset, dialog-core uses
    /// `tls_local_address` and then `local_address` as a compatibility
    /// fallback.
    #[serde(default)]
    pub tls_advertised_local_address: Option<SocketAddr>,

    /// Accept explicit SIP TLS Contacts on SIPS INVITEs received over TLS.
    /// Off by default; remote targets are normalized to SIPS, never cleartext.
    #[serde(default)]
    pub allow_tls_contact_on_sips: bool,

    /// Trusted record-routing proxies whose accepted TLS flow carries the
    /// in-dialog requests of the dialogs they deliver. Empty by default,
    /// which leaves routing unchanged. At most
    /// [`MAX_TLS_PROXY_FLOW_POLICIES`] entries. See [`SipTlsProxyFlowPolicy`].
    #[serde(default)]
    pub tls_proxy_flow_policies: Vec<SipTlsProxyFlowPolicy>,

    /// Keep the in-dialog requests of a dialog this endpoint initiated over
    /// TLS on the certificate-verified connection its initial INVITE used.
    ///
    /// When `true` and the dialog-forming INVITE was sent over TLS, later
    /// requests whose next hop is the same loose route (or, without a route
    /// set, the same Contact) at the same address are sent on that
    /// connection, even when the next hop is an IP literal the peer's
    /// certificate does not name (for example a carrier that is dialled by
    /// DNS name but record-routes with its IP address). If that connection
    /// has closed, the request fails instead of opening a new one. Requests
    /// to any other next hop use normal routing.
    ///
    /// `false` (the default) keeps normal routing: each request is matched
    /// to a pooled connection by address and verified name, and a closed
    /// connection is redialled.
    #[serde(default)]
    pub retain_outbound_tls_flow: bool,

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

    /// Optional dialog transaction-event dispatch worker count.
    ///
    /// `None` preserves the single dialog event processor. Values above `1`
    /// enable keyed sharding between transaction-manager events and dialog
    /// protocol handling while preserving per-call ordering.
    #[serde(default)]
    pub event_dispatch_workers: Option<usize>,

    /// Optional dialog transaction-event dispatch queue capacity.
    ///
    /// `None` uses the transaction-event channel capacity selected by the
    /// embedding stack. When dispatch workers are enabled, this capacity is
    /// divided across workers.
    #[serde(default)]
    pub event_dispatch_queue_capacity: Option<usize>,

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

    /// Policy for the SIP 100rel (reliable provisional) extension on
    /// outgoing INVITE. Default is `Supported` (advertise capability, don't
    /// demand it).
    #[serde(default)]
    pub use_100rel: RelUsage,

    /// Session-Expires (RFC 4028) interval to request on outgoing INVITEs,
    /// in seconds. `None` disables session timers entirely (no header sent,
    /// no refresh scheduled). Common carrier value is 1800 (30 minutes).
    #[serde(default)]
    pub session_timer_secs: Option<u32>,

    /// Minimum session-expires interval we're willing to accept (`Min-SE:`
    /// header per RFC 4028 §5). If the peer's `Min-SE` exceeds our
    /// `session_timer_secs`, we respond 422 Session Interval Too Small.
    /// Default is 90 s (the RFC's recommended minimum).
    #[serde(default = "default_min_se")]
    pub session_timer_min_se: u32,

    /// What this UAS implements: the methods, option tags, Request-URI
    /// schemes and session body types a request is checked against before
    /// it is processed (RFC 3261 §8.2: 405/501, 420, 416, 415, 406), and
    /// that the OPTIONS response advertises in Allow, Supported and Accept
    /// (RFC 3261 §11.2). List only what is implemented.
    #[serde(default)]
    pub uas_capabilities: rvoip_sip_core::validation::UasCapabilities,
}

fn default_min_se() -> u32 {
    90
}

impl Default for DialogConfig {
    fn default() -> Self {
        Self {
            local_address: "0.0.0.0:5060".parse().unwrap(),
            advertised_local_address: None,
            local_contact_uri: None,
            tls_local_address: None,
            tls_advertised_local_address: None,
            allow_tls_contact_on_sips: false,
            tls_proxy_flow_policies: Vec::new(),
            retain_outbound_tls_flow: false,
            user_agent: Some("RVOIP-Dialog/1.0".to_string()),
            dialog_timeout: Duration::from_secs(180), // 3 minutes
            max_dialogs: Some(10000),
            event_dispatch_workers: None,
            event_dispatch_queue_capacity: None,
            auto_cleanup: true,
            cleanup_interval: Duration::from_secs(30),
            use_100rel: RelUsage::default(),
            session_timer_secs: None,
            session_timer_min_se: default_min_se(),
            uas_capabilities: rvoip_sip_core::validation::UasCapabilities::default(),
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
    /// use rvoip_sip_dialog::api::DialogConfig;
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
    /// use rvoip_sip_dialog::api::DialogConfig;
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

    /// Set what this UAS implements (see [`DialogConfig::uas_capabilities`]).
    pub fn with_uas_capabilities(
        mut self,
        capabilities: rvoip_sip_core::validation::UasCapabilities,
    ) -> Self {
        self.uas_capabilities = capabilities;
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
    /// use rvoip_sip_dialog::api::DialogConfig;
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
    /// use rvoip_sip_dialog::api::DialogConfig;
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

    /// Set the dialog transaction-event dispatch worker count.
    pub fn with_event_dispatch_workers(mut self, workers: usize) -> Self {
        self.event_dispatch_workers = Some(workers);
        self
    }

    /// Set the dialog transaction-event dispatch queue capacity.
    pub fn with_event_dispatch_queue_capacity(mut self, capacity: usize) -> Self {
        self.event_dispatch_queue_capacity = Some(capacity);
        self
    }

    /// Set the 100rel (reliable provisional) policy (RFC 3262).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_dialog::api::{DialogConfig, config::RelUsage};
    ///
    /// let config = DialogConfig::new("127.0.0.1:5060".parse().unwrap())
    ///     .with_100rel(RelUsage::Required);
    /// assert_eq!(config.use_100rel, RelUsage::Required);
    /// ```
    pub fn with_100rel(mut self, policy: RelUsage) -> Self {
        self.use_100rel = policy;
        self
    }

    /// Enable RFC 4028 session timers with the given expiry in seconds.
    /// Pass `None` to disable.
    pub fn with_session_timer(mut self, secs: Option<u32>) -> Self {
        self.session_timer_secs = secs;
        self
    }

    /// Configure the minimum accepted session-expires interval (RFC 4028 §5).
    pub fn with_min_se(mut self, min_se: u32) -> Self {
        self.session_timer_min_se = min_se;
        self
    }

    /// Set the trusted record-routing proxies whose accepted TLS flow
    /// carries in-dialog requests. See [`SipTlsProxyFlowPolicy`].
    pub fn with_tls_proxy_flow_policies(
        mut self,
        policies: impl IntoIterator<Item = SipTlsProxyFlowPolicy>,
    ) -> Self {
        self.tls_proxy_flow_policies = policies.into_iter().collect();
        self
    }

    /// Keep in-dialog requests of locally initiated TLS dialogs on the
    /// initial INVITE's connection. See [`Self::retain_outbound_tls_flow`].
    pub fn with_retained_outbound_tls_flow(mut self, retain: bool) -> Self {
        self.retain_outbound_tls_flow = retain;
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
    /// use rvoip_sip_dialog::api::DialogConfig;
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
    /// use rvoip_sip_dialog::api::DialogConfig;
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

        if matches!(self.event_dispatch_workers, Some(0)) {
            return Err("Dialog event dispatch workers must be greater than 0".to_string());
        }

        if let Some(workers) = self.event_dispatch_workers {
            if workers > crate::manager::MAX_DIALOG_EVENT_DISPATCH_WORKERS {
                return Err(format!(
                    "Dialog event dispatch workers must be <= {}",
                    crate::manager::MAX_DIALOG_EVENT_DISPATCH_WORKERS
                ));
            }
        }

        if matches!(self.event_dispatch_queue_capacity, Some(0)) {
            return Err("Dialog event dispatch queue capacity must be greater than 0".to_string());
        }

        if self.cleanup_interval.as_secs() == 0 {
            return Err("Cleanup interval must be greater than 0".to_string());
        }

        if self.tls_proxy_flow_policies.len() > MAX_TLS_PROXY_FLOW_POLICIES {
            return Err(format!(
                "At most {MAX_TLS_PROXY_FLOW_POLICIES} TLS proxy-flow policies are allowed"
            ));
        }
        for policy in &self.tls_proxy_flow_policies {
            policy.validate()?;
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
/// use rvoip_sip_dialog::api::ServerConfig;
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
/// use rvoip_sip_dialog::api::ServerConfig;
/// use std::time::Duration;
///
/// let mut config = ServerConfig::new("0.0.0.0:5060".parse().unwrap())
///     .with_domain("sip.production.com")
///     .with_auto_options();
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

    /// Legacy automatic REGISTER-response flag.
    ///
    /// Dialog-core has no location-service binding store. When true it returns
    /// 501 rather than fabricating a successful registration. When false,
    /// REGISTER is delivered to the authoritative application registrar.
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
    /// use rvoip_sip_dialog::api::ServerConfig;
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
    /// use rvoip_sip_dialog::api::ServerConfig;
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
    /// use rvoip_sip_dialog::api::ServerConfig;
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

    /// Set the legacy automatic REGISTER-response flag.
    ///
    /// This does not create a registrar. REGISTER requests receive 501 while
    /// the flag is set; leave it disabled and install an application registrar
    /// to author standards-compliant binding responses.
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_dialog::api::ServerConfig;
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
    /// use rvoip_sip_dialog::api::ServerConfig;
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
/// use rvoip_sip_dialog::api::ClientConfig;
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
/// use rvoip_sip_dialog::api::ClientConfig;
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
    /// use rvoip_sip_dialog::api::ClientConfig;
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
    /// use rvoip_sip_dialog::api::ClientConfig;
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
    /// use rvoip_sip_dialog::api::ClientConfig;
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
    /// use rvoip_sip_dialog::api::ClientConfig;
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
/// use rvoip_sip_dialog::api::config::Credentials;
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
/// use rvoip_sip_dialog::api::config::Credentials;
///
/// let creds = Credentials::new("bob", "password456")
///     .with_realm("example.com");
/// assert_eq!(creds.realm.unwrap(), "example.com");
/// ```
#[derive(Clone, Serialize, Deserialize)]
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

impl fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Credentials")
            .field("username", &"[redacted]")
            .field("password", &"[redacted]")
            .field("has_realm", &self.realm.is_some())
            .finish()
    }
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
    /// use rvoip_sip_dialog::api::config::Credentials;
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
    /// use rvoip_sip_dialog::api::config::Credentials;
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

#[cfg(test)]
mod credential_debug_tests {
    use super::Credentials;

    #[test]
    fn credential_debug_redacts_username_password_and_realm() {
        let credentials =
            Credentials::new("dialog-user", "dialog-password").with_realm("dialog-realm");
        let debug = format!("{credentials:?}");
        for secret in ["dialog-user", "dialog-password", "dialog-realm"] {
            assert!(!debug.contains(secret));
        }
        assert!(debug.contains("has_realm: true"));
    }
}

#[cfg(test)]
mod tls_proxy_flow_policy_tests {
    use super::{DialogConfig, SipTlsProxyFlowPolicy, MAX_TLS_PROXY_FLOW_POLICIES};

    fn policy() -> SipTlsProxyFlowPolicy {
        SipTlsProxyFlowPolicy::new(
            "issuer",
            "tenant",
            "proxy",
            "192.0.2.10:5061".parse().unwrap(),
        )
    }

    #[test]
    fn policy_validation_rejects_bad_identity_endpoint_and_renewal_name() {
        assert!(policy().validate().is_ok());
        assert!(policy()
            .with_renewal_authority("sip.carrier.example")
            .validate()
            .is_ok());
        let long = "x".repeat(129);
        for identity in ["", " padded", "padded ", "line\nbreak", long.as_str()] {
            for field in 0..3 {
                let mut changed = policy();
                match field {
                    0 => changed.issuer = identity.into(),
                    1 => changed.tenant = identity.into(),
                    _ => changed.subject = identity.into(),
                }
                assert!(changed.validate().is_err(), "{field}: {identity:?}");
            }
        }
        for endpoint in [
            "192.0.2.10:0",
            "0.0.0.0:5061",
            "224.0.0.1:5061",
            "[::]:5061",
        ] {
            let mut changed = policy();
            changed.proxy_endpoint = endpoint.parse().unwrap();
            assert!(changed.validate().is_err(), "{endpoint}");
        }
        for name in [
            "",
            "192.0.2.10",
            "-bad.example",
            "bad-.example",
            "a..b",
            "under_score.example",
        ] {
            assert!(
                policy().with_renewal_authority(name).validate().is_err(),
                "{name:?}"
            );
        }
    }

    #[test]
    fn policies_are_default_empty_bounded_validated_and_serde_round_trip() {
        let config = DialogConfig::default();
        assert!(config.tls_proxy_flow_policies.is_empty());
        assert!(!config.retain_outbound_tls_flow);

        let config = DialogConfig::default()
            .with_tls_proxy_flow_policies([policy().with_renewal_authority("proxy.example")])
            .with_retained_outbound_tls_flow(true);
        assert!(config.validate().is_ok());
        let json = serde_json::to_string(&config).unwrap();
        let restored: DialogConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(
            restored.tls_proxy_flow_policies,
            config.tls_proxy_flow_policies
        );
        assert!(restored.retain_outbound_tls_flow);

        // A policy without renewal omits the field and parses back to None.
        let json = serde_json::to_string(&policy()).unwrap();
        assert!(!json.contains("renewal_authority"));
        assert_eq!(
            serde_json::from_str::<SipTlsProxyFlowPolicy>(&json).unwrap(),
            policy()
        );
        assert!(serde_json::from_str::<SipTlsProxyFlowPolicy>(
            r#"{"issuer":"i","tenant":"t","subject":"s","proxy_endpoint":"192.0.2.10:5061","unknown":1}"#
        )
        .is_err());

        let too_many = DialogConfig::default().with_tls_proxy_flow_policies(std::iter::repeat_n(
            policy(),
            MAX_TLS_PROXY_FLOW_POLICIES + 1,
        ));
        assert!(too_many.validate().is_err());
        let mut invalid = policy();
        invalid.subject.clear();
        assert!(DialogConfig::default()
            .with_tls_proxy_flow_policies([invalid])
            .validate()
            .is_err());
    }
}
