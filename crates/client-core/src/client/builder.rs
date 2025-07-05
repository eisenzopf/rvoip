//! Client builder for creating SIP clients
//! 
//! This module provides a fluent builder interface for constructing SIP clients
//! with comprehensive configuration options. The builder pattern allows for
//! readable, flexible client configuration while providing sensible defaults.
//! 
//! # Architecture
//! 
//! The `ClientBuilder` uses the builder pattern to construct a `ClientManager`
//! with all necessary configuration. It supports:
//! 
//! - Network configuration (local addresses, ports)
//! - Media configuration (codecs, SRTP, echo cancellation)
//! - SIP settings (user agent, domain)
//! - Resource limits (concurrent calls, bandwidth)
//! - Fluent sub-builders for complex configuration
//! 
//! # Examples
//! 
//! ## Basic Client Setup
//! 
//! ```rust
//! use rvoip_client_core::ClientBuilder;
//! 
//! # tokio_test::block_on(async {
//! let client = ClientBuilder::new()
//!     .local_address("127.0.0.1:5060".parse().unwrap())
//!     .domain("example.com")
//!     .user_agent("MyApp/1.0")
//!     .build()
//!     .await
//!     .expect("Failed to build client");
//! # })
//! ```
//! 
//! ## Advanced Media Configuration
//! 
//! ```rust
//! use rvoip_client_core::{ClientBuilder, MediaPreset};
//! 
//! # tokio_test::block_on(async {
//! let client = ClientBuilder::new()
//!     .local_address("0.0.0.0:5060".parse().unwrap())
//!     .media_address("192.168.1.100:7000".parse().unwrap())
//!     .with_media(|m| m
//!         .codecs(vec!["opus", "G722", "PCMU"])
//!         .require_srtp(true)
//!         .echo_cancellation(true)
//!         .noise_suppression(true)
//!         .max_bandwidth_kbps(256)
//!         .rtp_ports(7000..8000)
//!     )
//!     .max_concurrent_calls(10)
//!     .build()
//!     .await
//!     .expect("Failed to build client");
//! # })
//! ```

use std::sync::Arc;
use crate::{ClientConfig, ClientResult, client::ClientManager};
use super::config::{MediaConfig, MediaPreset};
use super::media_builder::MediaConfigBuilder;

/// Fluent builder for creating SIP clients with comprehensive configuration
/// 
/// The `ClientBuilder` provides a chainable interface for constructing SIP clients
/// with all necessary configuration options. It encapsulates the complexity of
/// client setup while providing sensible defaults and validation.
/// 
/// # Design Principles
/// 
/// - **Fluent Interface**: All configuration methods return `Self` for chaining
/// - **Sensible Defaults**: Works out-of-the-box with minimal configuration
/// - **Type Safety**: Compile-time validation of configuration parameters
/// - **Async Ready**: Built for async/await patterns with tokio integration
/// - **Flexible Media**: Supports both simple and advanced media configuration
/// 
/// # Configuration Categories
/// 
/// ## Network Configuration
/// - Local SIP and media addresses
/// - Port ranges for RTP traffic
/// - Domain and transport settings
/// 
/// ## Media Configuration  
/// - Codec preferences and capabilities
/// - Audio processing (echo cancellation, noise suppression)
/// - Security (SRTP requirements)
/// - Bandwidth and quality settings
/// 
/// ## SIP Configuration
/// - User agent identification
/// - Protocol compliance settings
/// - Extension support
/// 
/// ## Resource Management
/// - Concurrent call limits
/// - Memory and CPU constraints
/// - Network resource allocation
/// 
/// # Examples
/// 
/// ## Simple Desktop Client
/// 
/// ```rust
/// use rvoip_client_core::ClientBuilder;
/// 
/// # tokio_test::block_on(async {
/// let client = ClientBuilder::new()
///     .local_address("127.0.0.1:5060".parse().unwrap())
///     .domain("sip.example.com")
///     .echo_cancellation(true)
///     .build()
///     .await.unwrap();
/// # })
/// ```
/// 
/// ## Enterprise Server Setup
/// 
/// ```rust
/// use rvoip_client_core::{ClientBuilder, MediaPreset};
/// 
/// # tokio_test::block_on(async {
/// let client = ClientBuilder::new()
///     .local_address("0.0.0.0:5060".parse().unwrap())
///     .media_address("192.168.1.100:0".parse().unwrap())
///     .domain("enterprise.example.com")
///     .user_agent("EnterpriseVoIP/2.1")
///     .media_preset(MediaPreset::VoiceOptimized)
///     .max_concurrent_calls(100)
///     .rtp_ports(10000, 20000)
///     .build()
///     .await.unwrap();
/// # })
/// ```
/// 
/// ## WebRTC-Compatible Client
/// 
/// ```rust
/// use rvoip_client_core::ClientBuilder;
/// 
/// # tokio_test::block_on(async {
/// let client = ClientBuilder::new()
///     .local_address("127.0.0.1:5060".parse().unwrap())
///     .with_media(|m| m
///         .codecs(vec!["opus", "G722"])
///         .require_srtp(true)
///         .echo_cancellation(true)
///         .auto_gain_control(true)
///         .noise_suppression(true)
///     )
///     .build()
///     .await.unwrap();
/// # })
/// ```
pub struct ClientBuilder {
    config: ClientConfig,
}

impl ClientBuilder {
    /// Create a new client builder with default configuration
    /// 
    /// Initializes a new `ClientBuilder` with sensible default values suitable
    /// for most use cases. The defaults include:
    /// 
    /// - Local SIP address: `127.0.0.1:5060`
    /// - Local media address: `127.0.0.1:0` (auto-assigned port)
    /// - User agent: Generated from crate name and version
    /// - Media configuration: Basic codecs with standard settings
    /// - No domain specified (must be set for registration)
    /// - Maximum concurrent calls: 10
    /// 
    /// # Returns
    /// 
    /// A new `ClientBuilder` instance ready for configuration chaining.
    /// 
    /// # Examples
    /// 
    /// ## Basic Initialization
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// let builder = ClientBuilder::new();
    /// 
    /// // Builder is ready for configuration
    /// # tokio_test::block_on(async {
    /// let client = builder
    ///     .local_address("127.0.0.1:5060".parse().unwrap())
    ///     .build()
    ///     .await.unwrap();
    /// # })
    /// ```
    /// 
    /// ## Chained Configuration
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("0.0.0.0:5060".parse().unwrap())
    ///     .domain("sip.example.com")
    ///     .user_agent("CustomApp/1.0")
    ///     .max_concurrent_calls(50)
    ///     .build()
    ///     .await.unwrap();
    /// # })
    /// ```
    pub fn new() -> Self {
        Self {
            config: ClientConfig::default(),
        }
    }
    
    /// Set the local SIP address for binding the SIP transport
    /// 
    /// Configures the local socket address where the SIP client will bind to
    /// listen for incoming SIP messages and send outgoing requests. This address
    /// is used for all SIP protocol communication.
    /// 
    /// # Arguments
    /// 
    /// * `addr` - The socket address (IP and port) to bind for SIP communication
    /// 
    /// # Network Considerations
    /// 
    /// - **IP Address**: Use `0.0.0.0` to bind to all interfaces, or a specific IP for single-interface binding
    /// - **Port**: Standard SIP port is 5060 (UDP/TCP) or 5061 (TLS)
    /// - **Firewall**: Ensure the specified port is accessible for incoming connections
    /// - **NAT**: Consider using STUN/TURN for NAT traversal in production environments
    /// 
    /// # Examples
    /// 
    /// ## Standard SIP Port
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("127.0.0.1:5060".parse().unwrap())
    ///     .build()
    ///     .await.unwrap();
    /// # })
    /// ```
    /// 
    /// ## All Interfaces Binding
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("0.0.0.0:5060".parse().unwrap())
    ///     .domain("sip.provider.com")
    ///     .build()
    ///     .await.unwrap();
    /// # })
    /// ```
    /// 
    /// ## Custom Port
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("127.0.0.1:15060".parse().unwrap())
    ///     .build()
    ///     .await.unwrap();
    /// # })
    /// ```
    pub fn local_address(mut self, addr: std::net::SocketAddr) -> Self {
        self.config.local_sip_addr = addr;
        self
    }
    
    /// Set the local media address for RTP/RTCP traffic
    /// 
    /// Configures the local address where the client will bind for media traffic.
    /// This address is used for RTP (Real-time Transport Protocol) audio streams
    /// and RTCP (Real-time Control Protocol) feedback messages.
    /// 
    /// # Arguments
    /// 
    /// * `addr` - The socket address (IP and port) to bind for media communication
    /// 
    /// # Media Network Considerations
    /// 
    /// - **Separate from SIP**: Media traffic is independent of SIP signaling
    /// - **Port Zero**: When set to port 0, uses automatic port allocation via GlobalPortAllocator
    /// - **Firewall**: RTP requires a range of UDP ports to be accessible
    /// - **Quality of Service**: Consider network QoS settings for media traffic
    /// - **NAT Handling**: Media traffic often requires additional NAT traversal
    /// 
    /// # Examples
    /// 
    /// ## Auto-assigned Media Port
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("127.0.0.1:5060".parse().unwrap())
    ///     .media_address("127.0.0.1:0".parse().unwrap()) // Port auto-assigned
    ///     .build()
    ///     .await.unwrap();
    /// # })
    /// ```
    /// 
    /// ## Specific Media Interface
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("127.0.0.1:5060".parse().unwrap())
    ///     .media_address("127.0.0.1:7000".parse().unwrap())
    ///     .rtp_ports(7000, 8000) // Define RTP port range
    ///     .build()
    ///     .await.unwrap();
    /// # })
    /// ```
    /// 
    /// ## Multi-homed Configuration
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// # tokio_test::block_on(async {
    /// // SIP on one port, media on another
    /// let client = ClientBuilder::new()
    ///     .local_address("127.0.0.1:5060".parse().unwrap())
    ///     .media_address("127.0.0.1:0".parse().unwrap())
    ///     .build()
    ///     .await.unwrap();
    /// # })
    /// ```
    pub fn media_address(mut self, addr: std::net::SocketAddr) -> Self {
        self.config.local_media_addr = addr;
        self
    }
    
    /// Set the User-Agent header value for SIP messages
    /// 
    /// Configures the User-Agent header that will be included in outgoing SIP
    /// requests and responses. This header identifies the client software and
    /// version to remote SIP endpoints and servers.
    /// 
    /// # Arguments
    /// 
    /// * `user_agent` - String identifying the client application and version
    /// 
    /// # SIP Protocol Considerations
    /// 
    /// - **RFC 3261 Compliance**: User-Agent header is recommended in SIP messages
    /// - **Identification**: Helps with debugging and interoperability testing
    /// - **Statistics**: SIP servers often collect User-Agent statistics
    /// - **Format**: Typically follows "ProductName/Version" convention
    /// 
    /// # Examples
    /// 
    /// ## Application Identification
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("127.0.0.1:5060".parse().unwrap())
    ///     .user_agent("MyVoIPApp/2.1.0")
    ///     .build()
    ///     .await.unwrap();
    /// # })
    /// ```
    /// 
    /// ## Platform-Specific Agent
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// # tokio_test::block_on(async {
    /// let platform = std::env::consts::OS;
    /// let user_agent = format!("EnterprisePhone/1.0 ({})", platform);
    /// 
    /// let client = ClientBuilder::new()
    ///     .local_address("127.0.0.1:5060".parse().unwrap())
    ///     .user_agent(user_agent)
    ///     .build()
    ///     .await.unwrap();
    /// # })
    /// ```
    /// 
    /// ## SDK Integration
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("127.0.0.1:5060".parse().unwrap())
    ///     .user_agent("CustomerApp/3.2 rvoip-client-core/1.0")
    ///     .build()
    ///     .await.unwrap();
    /// # })
    /// ```
    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.config.user_agent = user_agent.into();
        self
    }
    
    /// Set the SIP domain for registration and routing
    /// 
    /// Configures the SIP domain that this client belongs to. This domain is used
    /// for SIP registration, request routing, and URI construction. It typically
    /// corresponds to the SIP service provider's domain.
    /// 
    /// # Arguments
    /// 
    /// * `domain` - The SIP domain name (e.g., "sip.provider.com")
    /// 
    /// # SIP Domain Usage
    /// 
    /// - **Registration**: Used as the domain in REGISTER requests
    /// - **URI Construction**: Forms the domain part of SIP URIs (sip:user@domain)
    /// - **Routing**: Helps determine where to send outbound requests
    /// - **Authentication**: Often tied to authentication realm
    /// 
    /// # Examples
    /// 
    /// ## Service Provider Domain
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("127.0.0.1:5060".parse().unwrap())
    ///     .domain("sip.provider.com")
    ///     .build()
    ///     .await.unwrap();
    /// # })
    /// ```
    /// 
    /// ## Enterprise Domain
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("0.0.0.0:5060".parse().unwrap())
    ///     .domain("pbx.company.com")
    ///     .user_agent("CompanyPhone/1.0")
    ///     .build()
    ///     .await.unwrap();
    /// # })
    /// ```
    /// 
    /// ## Local Development
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("127.0.0.1:5060".parse().unwrap())
    ///     .domain("localhost")
    ///     .build()
    ///     .await.unwrap();
    /// # })
    /// ```
    pub fn domain(mut self, domain: impl Into<String>) -> Self {
        self.config.domain = Some(domain.into());
        self
    }
    
    /// Set preferred audio codecs in priority order
    /// 
    /// Configures the list of preferred audio codecs for media negotiation.
    /// Codecs are specified in order of preference, with the first codec being
    /// the most preferred. During SDP negotiation, the client will attempt to
    /// use codecs in the specified order.
    /// 
    /// # Arguments
    /// 
    /// * `codecs` - Iterator of codec names in preference order
    /// 
    /// # Codec Considerations
    /// 
    /// - **Quality vs Bandwidth**: Balance audio quality with network bandwidth
    /// - **Compatibility**: Ensure codecs are supported by remote endpoints
    /// - **Computational Load**: Consider CPU requirements for codec processing
    /// - **Network Conditions**: Some codecs handle packet loss better than others
    /// 
    /// # Common Codecs
    /// 
    /// - **opus**: Modern, high-quality codec with excellent packet loss resilience
    /// - **G722**: Wideband codec (7kHz) with good quality
    /// - **PCMU/PCMA**: Standard narrowband codecs with universal compatibility
    /// - **G729**: Low-bandwidth codec, good for limited networks
    /// - **iLBC**: Designed for packet loss resilience
    /// 
    /// # Examples
    /// 
    /// ## High-Quality Configuration
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("127.0.0.1:5060".parse().unwrap())
    ///     .codecs(vec!["opus", "G722", "PCMU"])
    ///     .build()
    ///     .await.unwrap();
    /// # })
    /// ```
    /// 
    /// ## Low-Bandwidth Configuration
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("127.0.0.1:5060".parse().unwrap())
    ///     .codecs(vec!["G729", "iLBC", "PCMU"])
    ///     .build()
    ///     .await.unwrap();
    /// # })
    /// ```
    /// 
    /// ## Universal Compatibility
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("127.0.0.1:5060".parse().unwrap())
    ///     .codecs(vec!["PCMU", "PCMA", "G722"])
    ///     .build()
    ///     .await.unwrap();
    /// # })
    /// ```
    pub fn codecs<I, S>(mut self, codecs: I) -> Self 
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.config.media.preferred_codecs = codecs.into_iter().map(Into::into).collect();
        self
    }
    
    /// Configure media settings using a fluent sub-builder interface
    /// 
    /// This method provides access to a comprehensive media configuration builder
    /// that allows fine-grained control over audio processing, codecs, security,
    /// and network settings. The closure receives a `MediaConfigBuilder` that
    /// can be chained to configure multiple media options.
    /// 
    /// # Arguments
    /// 
    /// * `f` - Closure that configures media settings via `MediaConfigBuilder`
    /// 
    /// # Media Configuration Options
    /// 
    /// The sub-builder provides access to:
    /// - **Codec Selection**: Preferred codecs and priority ordering
    /// - **Audio Processing**: Echo cancellation, noise suppression, AGC
    /// - **Security**: SRTP requirements and key management
    /// - **Network**: Bandwidth limits, port ranges, DSCP marking
    /// - **Quality**: Sample rates, frame sizes, packet times
    /// 
    /// # Examples
    /// 
    /// ## Professional Audio Setup
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("127.0.0.1:5060".parse().unwrap())
    ///     .with_media(|m| m
    ///         .codecs(vec!["opus", "G722", "PCMU"])
    ///         .echo_cancellation(true)
    ///         .noise_suppression(true)
    ///         .auto_gain_control(true)
    ///         .max_bandwidth_kbps(256)
    ///         .ptime(20)
    ///     )
    ///     .build()
    ///     .await.unwrap();
    /// # })
    /// ```
    /// 
    /// ## Secure Enterprise Configuration
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("0.0.0.0:5060".parse().unwrap())
    ///     .with_media(|m| m
    ///         .codecs(vec!["opus", "G722"])
    ///         .require_srtp(true)
    ///         .srtp_profiles(vec!["AES_CM_128_HMAC_SHA1_80"])
    ///         .rtp_ports(10000..20000)
    /// 
    ///         .echo_cancellation(true)
    ///     )
    ///     .build()
    ///     .await.unwrap();
    /// # })
    /// ```
    /// 
    /// ## Low-Bandwidth Mobile Setup
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("127.0.0.1:5060".parse().unwrap())
    ///     .with_media(|m| m
    ///         .codecs(vec!["opus", "iLBC", "G729"])
    ///         .max_bandwidth_kbps(64)
    ///         .ptime(40) // Larger packets for efficiency
    ///         .echo_cancellation(true)
    ///     )
    ///     .build()
    ///     .await.unwrap();
    /// # })
    /// ```
    /// 
    /// ## WebRTC-Style Configuration
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("127.0.0.1:5060".parse().unwrap())
    ///     .with_media(|m| m
    ///         .codecs(vec!["opus", "G722"])
    ///         .require_srtp(true)
    ///         .echo_cancellation(true)
    ///         .noise_suppression(true)
    ///         .auto_gain_control(true)
    ///     )
    ///     .build()
    ///     .await.unwrap();
    /// # })
    /// ```
    pub fn with_media<F>(mut self, f: F) -> Self 
    where
        F: FnOnce(MediaConfigBuilder) -> MediaConfigBuilder,
    {
        let builder = MediaConfigBuilder::new();
        self.config.media = f(builder).build();
        self
    }
    
    /// Set media configuration directly
    pub fn media_config(mut self, media: MediaConfig) -> Self {
        self.config.media = media;
        self
    }
    
    /// Apply a predefined media configuration preset
    /// 
    /// Media presets provide quick configuration for common use cases by applying
    /// a set of predefined media settings. This is convenient when you need
    /// standard configurations without detailed customization.
    /// 
    /// # Arguments
    /// 
    /// * `preset` - The media preset to apply
    /// 
    /// # Available Presets
    /// 
    /// - **`VoiceOptimized`**: Optimized for voice calls with echo cancellation and noise suppression
    /// - **`HighQuality`**: Premium audio quality with wideband codecs and advanced processing
    /// - **`LowLatency`**: Minimal processing delay for real-time applications
    /// - **`LowBandwidth`**: Optimized for limited network conditions
    /// - **`Conference`**: Multi-party conferencing with audio mixing support
    /// - **`WebRTCCompatible`**: Settings compatible with WebRTC endpoints
    /// 
    /// # Examples
    /// 
    /// ## Voice-Optimized Desktop Client
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientBuilder, MediaPreset};
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("127.0.0.1:5060".parse().unwrap())
    ///     .media_preset(MediaPreset::VoiceOptimized)
    ///     .build()
    ///     .await.unwrap();
    /// # })
    /// ```
    /// 
    /// ## Security-Focused System
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientBuilder, MediaPreset};
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("127.0.0.1:5060".parse().unwrap())
    ///     .media_preset(MediaPreset::Secure)
    ///     .domain("premium.voip.com")
    ///     .build()
    ///     .await
    ///     .unwrap();
    /// # })
    /// ```
    /// 
    /// ## Legacy-Compatible Application
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientBuilder, MediaPreset};
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("127.0.0.1:18060".parse().unwrap())
    ///     .media_preset(MediaPreset::Legacy)
    ///     .max_concurrent_calls(50)
    ///     .build()
    ///     .await
    ///     .unwrap();
    /// # })
    /// ```
    /// 
    /// ## Mobile/Limited Bandwidth
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientBuilder, MediaPreset};
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("127.0.0.1:5060".parse().unwrap())
    ///     .media_preset(MediaPreset::LowBandwidth)
    ///     .build()
    ///     .await
    ///     .unwrap();
    /// # })
    /// ```
    pub fn media_preset(mut self, preset: MediaPreset) -> Self {
        self.config.media = MediaConfig::from_preset(preset);
        self
    }
    
    /// Enable or disable acoustic echo cancellation (AEC)
    /// 
    /// Acoustic echo cancellation removes the echo of your own voice that
    /// can occur when the remote party's audio is played through speakers
    /// instead of headphones. This is essential for hands-free operation.
    /// 
    /// # Arguments
    /// 
    /// * `enabled` - Whether to enable echo cancellation processing
    /// 
    /// # Use Cases
    /// 
    /// - **Hands-free calling**: Essential when using speakers instead of headphones
    /// - **Conference rooms**: Critical for room-based conferencing systems
    /// - **Desktop applications**: Improves user experience in office environments
    /// - **Mobile devices**: Important for speakerphone functionality
    /// 
    /// # Performance Considerations
    /// 
    /// - **CPU Usage**: Echo cancellation requires additional processing power
    /// - **Latency**: May introduce small amounts of audio delay
    /// - **Quality**: Generally improves overall call quality significantly
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// # tokio_test::block_on(async {
    /// // Enable for desktop/conference use
    /// let client = ClientBuilder::new()
    ///     .local_address("127.0.0.1:5060".parse().unwrap())
    ///     .echo_cancellation(true)
    ///     .build()
    ///     .await.unwrap();
    /// # })
    /// ```
    pub fn echo_cancellation(mut self, enabled: bool) -> Self {
        self.config.media.echo_cancellation = enabled;
        self
    }
    
    /// Require or allow SRTP (Secure Real-time Transport Protocol)
    /// 
    /// SRTP provides encryption and authentication for RTP media streams.
    /// When required, the client will only establish calls that support
    /// encrypted media, rejecting unencrypted connections.
    /// 
    /// # Arguments
    /// 
    /// * `required` - Whether SRTP is mandatory for all media streams
    /// 
    /// # Security Considerations
    /// 
    /// - **Data Protection**: Encrypts audio streams end-to-end
    /// - **Integrity**: Protects against media tampering and injection
    /// - **Compliance**: May be required for regulatory compliance
    /// - **Performance**: Adds minimal computational overhead
    /// 
    /// # Compatibility
    /// 
    /// - **WebRTC**: SRTP is mandatory in WebRTC implementations
    /// - **Legacy Systems**: Some older SIP devices may not support SRTP
    /// - **Enterprise**: Most modern enterprise systems support SRTP
    /// 
    /// # Examples
    /// 
    /// ## Security-First Configuration
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("127.0.0.1:5060".parse().unwrap())
    ///     .require_srtp(true)
    ///     .domain("secure.voip.com")
    ///     .build()
    ///     .await.unwrap();
    /// # })
    /// ```
    /// 
    /// ## Flexible Security (SRTP preferred but not required)
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("127.0.0.1:5060".parse().unwrap())
    ///     .require_srtp(false) // Allow fallback to RTP
    ///     .build()
    ///     .await.unwrap();
    /// # })
    /// ```
    pub fn require_srtp(mut self, required: bool) -> Self {
        self.config.media.require_srtp = required;
        self
    }
    
    /// Set the UDP port range for RTP media traffic
    /// 
    /// Configures the range of UDP ports that will be used for RTP and RTCP
    /// media streams. The client will allocate ports within this range for
    /// each active call's media streams.
    /// 
    /// # Arguments
    /// 
    /// * `start` - First port in the range (inclusive)
    /// * `end` - Last port in the range (inclusive)
    /// 
    /// # Port Planning Considerations
    /// 
    /// - **Concurrent Calls**: Each call typically uses 2 ports (RTP + RTCP)
    /// - **Firewall Configuration**: All ports in range must be accessible
    /// - **Range Size**: Should accommodate maximum expected simultaneous calls
    /// - **Standard Ranges**: Many systems use 10000-20000 or 16384-32767
    /// 
    /// # Examples
    /// 
    /// ## Standard Enterprise Range
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("0.0.0.0:5060".parse().unwrap())
    ///     .rtp_ports(10000, 20000) // 10,000 ports for ~5,000 calls
    ///     .max_concurrent_calls(1000)
    ///     .build()
    ///     .await.unwrap();
    /// # })
    /// ```
    /// 
    /// ## Small Office Configuration
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("127.0.0.1:5060".parse().unwrap())
    ///     .rtp_ports(7000, 7100) // 100 ports for ~50 calls
    ///     .max_concurrent_calls(25)
    ///     .build()
    ///     .await.unwrap();
    /// # })
    /// ```
    /// 
    /// ## High-Density Server
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("0.0.0.0:5060".parse().unwrap())
    ///     .rtp_ports(16384, 32767) // ~16k ports for high capacity
    ///     .max_concurrent_calls(5000)
    ///     .build()
    ///     .await.unwrap();
    /// # })
    /// ```
    pub fn rtp_ports(mut self, start: u16, end: u16) -> Self {
        self.config.media.rtp_port_start = start;
        self.config.media.rtp_port_end = end;
        self
    }
    
    /// Set the maximum number of concurrent calls
    /// 
    /// Limits the number of simultaneous active calls that the client can handle.
    /// This prevents resource exhaustion and ensures predictable performance
    /// under load. New call attempts beyond this limit will be rejected.
    /// 
    /// # Arguments
    /// 
    /// * `max` - Maximum number of concurrent calls allowed
    /// 
    /// # Resource Planning
    /// 
    /// Consider the following resources when setting limits:
    /// - **CPU**: Audio processing scales with concurrent calls
    /// - **Memory**: Each call maintains state and buffers
    /// - **Network**: Bandwidth requirements multiply with call count
    /// - **Ports**: Each call requires RTP/RTCP port pairs
    /// 
    /// # Examples
    /// 
    /// ## Desktop Client
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("127.0.0.1:5060".parse().unwrap())
    ///     .max_concurrent_calls(5) // Typical for desktop use
    ///     .build()
    ///     .await.unwrap();
    /// # })
    /// ```
    /// 
    /// ## Small Business Server
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("0.0.0.0:5060".parse().unwrap())
    ///     .max_concurrent_calls(100)
    ///     .rtp_ports(10000, 10500) // Adequate port range
    ///     .build()
    ///     .await.unwrap();
    /// # })
    /// ```
    /// 
    /// ## High-Capacity System
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("0.0.0.0:5060".parse().unwrap())
    ///     .max_concurrent_calls(10000)
    ///     .rtp_ports(10000, 40000) // Large port range
    ///     .with_media(|m| m
    ///         .codecs(vec!["G729", "PCMU"]) // Low-bandwidth codecs
    ///         .max_bandwidth_kbps(64)
    ///     )
    ///     .build()
    ///     .await.unwrap();
    /// # })
    /// ```
    pub fn max_concurrent_calls(mut self, max: usize) -> Self {
        self.config.max_concurrent_calls = max;
        self
    }
    
    /// Build and initialize the SIP client
    /// 
    /// Consumes the builder and creates a fully configured `ClientManager` instance.
    /// This method performs all necessary initialization including network binding,
    /// media system setup, and internal component initialization.
    /// 
    /// # Returns
    /// 
    /// Returns a `ClientResult<Arc<ClientManager>>` which will be:
    /// - `Ok(client)` on successful initialization
    /// - `Err(error)` if initialization fails for any reason
    /// 
    /// # Initialization Process
    /// 
    /// The build process performs several critical steps:
    /// 1. **Configuration Validation**: Verifies all settings are valid and consistent
    /// 2. **Network Binding**: Binds to specified SIP and media addresses
    /// 3. **Media System Init**: Initializes audio processing and codec subsystems
    /// 4. **Component Startup**: Starts internal services and background tasks
    /// 5. **Resource Allocation**: Reserves ports, memory, and other resources
    /// 
    /// # Error Conditions
    /// 
    /// The build process can fail for various reasons:
    /// - **Network Errors**: Port already in use, invalid addresses, permission denied
    /// - **Resource Limits**: Insufficient system resources, file descriptor limits
    /// - **Configuration Errors**: Invalid settings, unsupported codec combinations
    /// - **System Errors**: Audio device access, network interface issues
    /// 
    /// # Examples
    /// 
    /// ## Basic Client Creation
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("127.0.0.1:5060".parse().unwrap())
    ///     .build()
    ///     .await.unwrap();
    /// 
    /// // Client is ready for use
    /// println!("Client initialized successfully");
    /// # })
    /// ```
    /// 
    /// ## Error Handling
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientBuilder;
    /// 
    /// # tokio_test::block_on(async {
    /// match ClientBuilder::new()
    ///     .local_address("127.0.0.1:5060".parse().unwrap())
    ///     .domain("sip.provider.com")
    ///     .build()
    ///     .await
    /// {
    ///     Ok(client) => {
    ///         println!("Client ready for SIP operations");
    ///         // Use client...
    ///     }
    ///     Err(error) => {
    ///         eprintln!("Failed to initialize client: {}", error);
    ///         // Handle initialization failure...
    ///     }
    /// }
    /// # })
    /// ```
    /// 
    /// ## Production Configuration
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientBuilder, MediaPreset};
    /// 
    /// # tokio_test::block_on(async {
    /// let client = ClientBuilder::new()
    ///     .local_address("0.0.0.0:5060".parse().unwrap())
    ///     .media_address("192.168.1.100:0".parse().unwrap())
    ///     .domain("enterprise.company.com")
    ///     .user_agent("CompanyPhone/3.1.0")
    ///     .media_preset(MediaPreset::VoiceOptimized)
    ///     .max_concurrent_calls(50)
    ///     .rtp_ports(10000, 20000)
    ///     .require_srtp(true)
    ///     .build()
    ///     .await.unwrap();
    /// 
    /// println!("Enterprise client initialized and ready");
    /// # })
    /// ```
    /// 
    /// # Thread Safety
    /// 
    /// The returned `ClientManager` is wrapped in an `Arc` for safe sharing
    /// across threads and async tasks. All client operations are thread-safe.
    /// 
    /// # Resource Management
    /// 
    /// The client automatically manages its resources and will clean up
    /// properly when dropped. For graceful shutdown, use the client's
    /// shutdown methods before dropping.
    pub async fn build(mut self) -> ClientResult<Arc<ClientManager>> {
        // If media address has default IP (127.0.0.1), update to match SIP IP but keep port
        let default_media_addr: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
        if self.config.local_media_addr.ip() == default_media_addr.ip() {
            let sip_ip = self.config.local_sip_addr.ip();
            let media_port = self.config.local_media_addr.port(); // Keep existing port (0 = auto)
            self.config.local_media_addr = format!("{}:{}", sip_ip, media_port).parse().unwrap();
        }
        ClientManager::new(self.config).await
    }
}

/// Default implementation for ClientBuilder
/// 
/// Creates a new `ClientBuilder` with default configuration settings.
/// This is equivalent to calling `ClientBuilder::new()`.
/// 
/// # Examples
/// 
/// ```rust
/// use rvoip_client_core::ClientBuilder;
/// 
/// // These are equivalent:
/// let builder1 = ClientBuilder::new();
/// let builder2 = ClientBuilder::default();
/// 
/// # tokio_test::block_on(async {
/// let client = ClientBuilder::default()
///     .local_address("127.0.0.1:5060".parse().unwrap())
///     .build()
///     .await.unwrap();
/// # })
/// ```
impl Default for ClientBuilder {
    fn default() -> Self {
        Self::new()
    }
} 