//! Media configuration builder for fluent API
//! 
//! Provides a convenient way to configure media preferences using the builder pattern.

use super::config::MediaConfig;

/// Builder for MediaConfig with fluent API
/// 
/// The MediaConfigBuilder provides a convenient, type-safe way to construct complex
/// media configurations using the builder pattern. It offers fluent method chaining
/// and sensible defaults for VoIP applications.
/// 
/// # Examples
/// 
/// ## Basic Voice Configuration
/// 
/// ```rust
/// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
/// 
/// let config = MediaConfigBuilder::new()
///     .codecs(vec!["opus", "PCMU"])
///     .echo_cancellation(true)
///     .noise_suppression(true)
///     .build();
/// 
/// assert_eq!(config.preferred_codecs, vec!["opus", "PCMU"]);
/// assert!(config.echo_cancellation);
/// assert!(config.noise_suppression);
/// ```
/// 
/// ## Secure Enterprise Configuration
/// 
/// ```rust
/// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
/// 
/// let config = MediaConfigBuilder::new()
///     .codecs(vec!["opus", "G722"])
///     .require_srtp(true)
///     .srtp_profiles(vec!["AES_CM_128_HMAC_SHA1_80", "AES_CM_128_HMAC_SHA1_32"])
///     .max_bandwidth_kbps(128)
///     .audio_processing(true)
///     .build();
/// 
/// assert!(config.require_srtp);
/// assert_eq!(config.srtp_profiles.len(), 2);
/// assert_eq!(config.max_bandwidth_kbps, Some(128));
/// ```
/// 
/// ## Mobile/Low-Bandwidth Configuration
/// 
/// ```rust
/// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
/// 
/// let config = MediaConfigBuilder::new()
///     .codecs(vec!["G.729", "GSM", "PCMU"])
///     .max_bandwidth_kbps(32)
///     .ptime(30)  // Longer packets for efficiency
///     .audio_processing(true)  // Keep quality despite bandwidth constraints
///     .build();
/// 
/// assert_eq!(config.max_bandwidth_kbps, Some(32));
/// assert_eq!(config.preferred_ptime, Some(30));
/// assert!(config.echo_cancellation);
/// ```
pub struct MediaConfigBuilder {
    config: MediaConfig,
}

impl Default for MediaConfigBuilder {
    fn default() -> Self {
        Self {
            config: MediaConfig::default(),
        }
    }
}

impl MediaConfigBuilder {
    /// Create a new media config builder
    /// 
    /// Initializes a MediaConfigBuilder with default settings suitable for most
    /// VoIP applications. The defaults include common codecs, standard RTP port
    /// ranges, and audio processing disabled.
    /// 
    /// # Returns
    /// 
    /// A new MediaConfigBuilder with sensible defaults
    /// 
    /// # Examples
    /// 
    /// ## Basic Builder Creation
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let builder = MediaConfigBuilder::new();
    /// let config = builder.build();
    /// 
    /// // Verify default settings
    /// assert_eq!(config.rtp_port_start, 10000);
    /// assert_eq!(config.rtp_port_end, 20000);
    /// assert!(config.echo_cancellation);  // Default is enabled
    /// assert!(!config.require_srtp);
    /// ```
    /// 
    /// ## Fluent Chaining
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .echo_cancellation(true)
    ///     .max_bandwidth_kbps(64)
    ///     .build();
    /// 
    /// assert!(config.echo_cancellation);
    /// assert_eq!(config.max_bandwidth_kbps, Some(64));
    /// ```
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Set preferred codecs in order of preference
    /// 
    /// Configures the list of preferred audio codecs for media negotiation.
    /// Codecs will be offered in the order specified, with the first codec
    /// being the most preferred. Common codecs include "opus", "PCMU", "PCMA",
    /// "G722", "G.729", and "GSM".
    /// 
    /// # Arguments
    /// 
    /// * `codecs` - An iterable of codec names (anything convertible to String)
    /// 
    /// # Examples
    /// 
    /// ## High Quality Configuration
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .codecs(vec!["opus", "G722", "PCMU"])
    ///     .build();
    /// 
    /// assert_eq!(config.preferred_codecs, vec!["opus", "G722", "PCMU"]);
    /// assert_eq!(config.preferred_codecs[0], "opus"); // First is most preferred
    /// ```
    /// 
    /// ## Legacy Compatibility
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .codecs(vec!["PCMU", "PCMA"])  // G.711 for maximum compatibility
    ///     .build();
    /// 
    /// assert_eq!(config.preferred_codecs.len(), 2);
    /// assert!(config.preferred_codecs.contains(&"PCMU".to_string()));
    /// assert!(config.preferred_codecs.contains(&"PCMA".to_string()));
    /// ```
    /// 
    /// ## Low Bandwidth Mobile
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .codecs(["G.729", "GSM", "PCMU"])  // Array syntax also works
    ///     .build();
    /// 
    /// assert_eq!(config.preferred_codecs[0], "G.729");  // Lowest bandwidth first
    /// assert_eq!(config.preferred_codecs.len(), 3);
    /// ```
    /// 
    /// ## String References and Owned Strings
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let opus = "opus".to_string();
    /// let config = MediaConfigBuilder::new()
    ///     .codecs(vec![opus, "PCMU".to_string(), "G722".to_string()])  // Mixed types work
    ///     .build();
    /// 
    /// assert_eq!(config.preferred_codecs.len(), 3);
    /// assert_eq!(config.preferred_codecs[0], "opus");
    /// ```
    pub fn codecs<I, S>(mut self, codecs: I) -> Self 
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.config.preferred_codecs = codecs.into_iter().map(Into::into).collect();
        self
    }
    
    /// Add a single codec to the preference list
    /// 
    /// Appends a codec to the end of the preferred codecs list. This is useful
    /// for incrementally building codec preferences or adding fallback codecs.
    /// The codec will be offered after all previously configured codecs.
    /// 
    /// # Arguments
    /// 
    /// * `codec` - The codec name to add (anything convertible to String)
    /// 
    /// # Examples
    /// 
    /// ## Building Codec List Incrementally
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .codecs(Vec::<&str>::new())  // Start with empty list
    ///     .add_codec("opus")        // Most preferred
    ///     .add_codec("G722")        // Second choice
    ///     .add_codec("PCMU")        // Fallback
    ///     .build();
    /// 
    /// assert_eq!(config.preferred_codecs, vec!["opus", "G722", "PCMU"]);
    /// assert_eq!(config.preferred_codecs.len(), 3);
    /// ```
    /// 
    /// ## Conditional Codec Addition
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let high_quality = true;
    /// let mut builder = MediaConfigBuilder::new()
    ///     .codecs(Vec::<&str>::new())  // Start with empty list
    ///     .add_codec("PCMU");      // Always include baseline
    /// 
    /// if high_quality {
    ///     builder = builder.add_codec("opus");  // Add high quality option
    /// }
    /// 
    /// let config = builder.build();
    /// assert!(config.preferred_codecs.contains(&"opus".to_string()));
    /// assert!(config.preferred_codecs.contains(&"PCMU".to_string()));
    /// ```
    /// 
    /// ## Combining with Codec List
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .codecs(vec!["opus", "G722"])     // Set initial list
    ///     .add_codec("PCMU")               // Add fallback
    ///     .add_codec("GSM")                // Add another fallback
    ///     .build();
    /// 
    /// assert_eq!(config.preferred_codecs, vec!["opus", "G722", "PCMU", "GSM"]);
    /// assert_eq!(config.preferred_codecs.len(), 4);
    /// ```
    /// 
    /// ## String Types
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let codec_name = "G.729".to_string();
    /// let config = MediaConfigBuilder::new()
    ///     .codecs(Vec::<&str>::new())  // Start with empty list
    ///     .add_codec(codec_name)       // Owned string
    ///     .add_codec("PCMU")           // String literal
    ///     .build();
    /// 
    /// assert_eq!(config.preferred_codecs[0], "G.729");
    /// assert_eq!(config.preferred_codecs[1], "PCMU");
    /// ```
    pub fn add_codec(mut self, codec: impl Into<String>) -> Self {
        self.config.preferred_codecs.push(codec.into());
        self
    }
    
    /// Enable or disable DTMF (Dual-Tone Multi-Frequency) support
    /// 
    /// DTMF is used for touch-tone signaling during calls (phone keypad tones).
    /// When enabled, the client can send and receive DTMF digits for IVR systems,
    /// voicemail navigation, and other interactive applications.
    /// 
    /// # Arguments
    /// 
    /// * `enabled` - Whether to enable DTMF support
    /// 
    /// # Examples
    /// 
    /// ## Enable DTMF for IVR Systems
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .dtmf(true)
    ///     .build();
    /// 
    /// assert!(config.dtmf_enabled);
    /// ```
    /// 
    /// ## Disable DTMF for Simple Voice Calls
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .dtmf(false)  // Explicitly disable
    ///     .build();
    /// 
    /// assert!(!config.dtmf_enabled);
    /// ```
    /// 
    /// ## Call Center Configuration
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .dtmf(true)                    // Need DTMF for IVR
    ///     .codecs(vec!["PCMU", "PCMA"])  // Legacy compatibility
    ///     .echo_cancellation(true)       // Clean audio for agents
    ///     .build();
    /// 
    /// assert!(config.dtmf_enabled);
    /// assert!(config.echo_cancellation);
    /// ```
    pub fn dtmf(mut self, enabled: bool) -> Self {
        self.config.dtmf_enabled = enabled;
        self
    }
    
    /// Enable or disable echo cancellation
    /// 
    /// Echo cancellation removes acoustic echo that occurs when audio from
    /// the speaker is picked up by the microphone. This is essential for
    /// hands-free calling and speakerphone usage.
    /// 
    /// # Arguments
    /// 
    /// * `enabled` - Whether to enable echo cancellation
    /// 
    /// # Examples
    /// 
    /// ## Hands-Free Configuration
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .echo_cancellation(true)  // Essential for speakerphone
    ///     .build();
    /// 
    /// assert!(config.echo_cancellation);
    /// ```
    /// 
    /// ## Headset Configuration
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .echo_cancellation(false)  // Not needed with good headsets
    ///     .noise_suppression(true)   // Still want noise reduction
    ///     .build();
    /// 
    /// assert!(!config.echo_cancellation);
    /// assert!(config.noise_suppression);
    /// ```
    pub fn echo_cancellation(mut self, enabled: bool) -> Self {
        self.config.echo_cancellation = enabled;
        self
    }
    
    /// Enable or disable noise suppression
    /// 
    /// Noise suppression reduces background noise such as keyboard typing,
    /// air conditioning, traffic, and other environmental sounds. This
    /// improves call quality and listener experience.
    /// 
    /// # Arguments
    /// 
    /// * `enabled` - Whether to enable noise suppression
    /// 
    /// # Examples
    /// 
    /// ## Office Environment
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .noise_suppression(true)   // Reduce office noise
    ///     .echo_cancellation(true)   // Handle speakerphone
    ///     .build();
    /// 
    /// assert!(config.noise_suppression);
    /// assert!(config.echo_cancellation);
    /// ```
    /// 
    /// ## Studio Quality Recording
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .noise_suppression(false)  // Preserve natural audio
    ///     .echo_cancellation(false)  // Controlled environment
    ///     .auto_gain_control(false)  // Manual levels
    ///     .build();
    /// 
    /// assert!(!config.noise_suppression);
    /// assert!(!config.echo_cancellation);
    /// assert!(!config.auto_gain_control);
    /// ```
    pub fn noise_suppression(mut self, enabled: bool) -> Self {
        self.config.noise_suppression = enabled;
        self
    }
    
    /// Enable or disable automatic gain control
    /// 
    /// Automatic Gain Control (AGC) automatically adjusts microphone levels
    /// to maintain consistent volume. It prevents audio from being too quiet
    /// or too loud, and adapts to different speakers and environments.
    /// 
    /// # Arguments
    /// 
    /// * `enabled` - Whether to enable automatic gain control
    /// 
    /// # Examples
    /// 
    /// ## Multi-User Environment
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .auto_gain_control(true)   // Handle different users
    ///     .noise_suppression(true)   // Clean up background
    ///     .build();
    /// 
    /// assert!(config.auto_gain_control);
    /// assert!(config.noise_suppression);
    /// ```
    /// 
    /// ## Professional Audio Setup
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .auto_gain_control(false)  // Manual control preferred
    ///     .noise_suppression(true)   // Still want noise reduction
    ///     .build();
    /// 
    /// assert!(!config.auto_gain_control);
    /// assert!(config.noise_suppression);
    /// ```
    /// 
    /// ## Mobile/Varied Environment
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .auto_gain_control(true)   // Adapt to changing conditions
    ///     .echo_cancellation(true)   // Handle speaker feedback
    ///     .noise_suppression(true)   // Clean up environment
    ///     .build();
    /// 
    /// assert!(config.auto_gain_control);
    /// assert!(config.echo_cancellation);
    /// assert!(config.noise_suppression);
    /// ```
    pub fn auto_gain_control(mut self, enabled: bool) -> Self {
        self.config.auto_gain_control = enabled;
        self
    }
    
    /// Set maximum bandwidth in kilobits per second
    /// 
    /// Constrains the total bandwidth used for media streams. This is useful
    /// for mobile networks, limited connections, or when bandwidth needs to
    /// be shared among multiple applications.
    /// 
    /// # Arguments
    /// 
    /// * `bandwidth` - Maximum bandwidth in kbps (kilobits per second)
    /// 
    /// # Examples
    /// 
    /// ## Mobile/3G Configuration
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .max_bandwidth_kbps(32)           // Low bandwidth for mobile
    ///     .codecs(vec!["G.729", "GSM"])     // Efficient codecs
    ///     .ptime(30)                       // Longer packets
    ///     .build();
    /// 
    /// assert_eq!(config.max_bandwidth_kbps, Some(32));
    /// ```
    /// 
    /// ## Standard Voice Quality
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .max_bandwidth_kbps(64)           // Standard voice quality
    ///     .codecs(vec!["PCMU", "PCMA"])     // G.711 codecs
    ///     .build();
    /// 
    /// assert_eq!(config.max_bandwidth_kbps, Some(64));
    /// ```
    /// 
    /// ## High Quality/Conference
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .max_bandwidth_kbps(128)          // High quality
    ///     .codecs(vec!["opus", "G722"])     // Wideband codecs
    ///     .build();
    /// 
    /// assert_eq!(config.max_bandwidth_kbps, Some(128));
    /// ```
    /// 
    /// ## Unlimited Bandwidth
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new().build();
    /// 
    /// // Default is no bandwidth limit
    /// assert_eq!(config.max_bandwidth_kbps, None);
    /// ```
    pub fn max_bandwidth_kbps(mut self, bandwidth: u32) -> Self {
        self.config.max_bandwidth_kbps = Some(bandwidth);
        self
    }
    
    /// Require SRTP encryption for secure media
    /// 
    /// SRTP (Secure Real-time Transport Protocol) encrypts media streams
    /// to prevent eavesdropping. When required, calls will only be established
    /// if both parties support SRTP encryption.
    /// 
    /// # Arguments
    /// 
    /// * `required` - Whether to require SRTP encryption
    /// 
    /// # Examples
    /// 
    /// ## Enterprise Security
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .require_srtp(true)               // Mandatory encryption
    ///     .srtp_profiles(vec![
    ///         "AES_CM_128_HMAC_SHA1_80",
    ///         "AES_CM_128_HMAC_SHA1_32"
    ///     ])
    ///     .build();
    /// 
    /// assert!(config.require_srtp);
    /// assert!(!config.srtp_profiles.is_empty());
    /// ```
    /// 
    /// ## Best-Effort Security
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .require_srtp(false)              // Optional encryption
    ///     .srtp_profiles(vec!["AES_CM_128_HMAC_SHA1_80"])
    ///     .build();
    /// 
    /// assert!(!config.require_srtp);       // Will try SRTP but allow unencrypted
    /// assert!(!config.srtp_profiles.is_empty());
    /// ```
    /// 
    /// ## No Encryption
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .require_srtp(false)              // No encryption requirement
    ///     .build();
    /// 
    /// assert!(!config.require_srtp);
    /// ```
    pub fn require_srtp(mut self, required: bool) -> Self {
        self.config.require_srtp = required;
        self
    }
    
    /// Set SRTP encryption profiles
    /// 
    /// Configures the list of supported SRTP encryption profiles in order
    /// of preference. These determine the encryption algorithms and key
    /// lengths used for secure media transmission.
    /// 
    /// # Arguments
    /// 
    /// * `profiles` - An iterable of SRTP profile names
    /// 
    /// # Common Profiles
    /// 
    /// - `AES_CM_128_HMAC_SHA1_80` - AES-128 with 80-bit authentication
    /// - `AES_CM_128_HMAC_SHA1_32` - AES-128 with 32-bit authentication
    /// - `AES_CM_256_HMAC_SHA1_80` - AES-256 with 80-bit authentication
    /// 
    /// # Examples
    /// 
    /// ## Standard Security
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .require_srtp(true)
    ///     .srtp_profiles(vec!["AES_CM_128_HMAC_SHA1_80"])
    ///     .build();
    /// 
    /// assert_eq!(config.srtp_profiles.len(), 1);
    /// assert_eq!(config.srtp_profiles[0], "AES_CM_128_HMAC_SHA1_80");
    /// ```
    /// 
    /// ## Multiple Profile Support
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .srtp_profiles(vec![
    ///         "AES_CM_128_HMAC_SHA1_80",    // Preferred
    ///         "AES_CM_128_HMAC_SHA1_32",    // Fallback
    ///     ])
    ///     .build();
    /// 
    /// assert_eq!(config.srtp_profiles.len(), 2);
    /// assert_eq!(config.srtp_profiles[0], "AES_CM_128_HMAC_SHA1_80");
    /// ```
    /// 
    /// ## High Security
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .require_srtp(true)
    ///     .srtp_profiles(vec!["AES_CM_256_HMAC_SHA1_80"])  // 256-bit encryption
    ///     .build();
    /// 
    /// assert!(config.require_srtp);
    /// assert_eq!(config.srtp_profiles[0], "AES_CM_256_HMAC_SHA1_80");
    /// ```
    /// 
    /// ## Government/Military Grade
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let profiles = vec![
    ///     "AES_CM_256_HMAC_SHA1_80".to_string(),  // Highest security first
    ///     "AES_CM_128_HMAC_SHA1_80".to_string(),  // Compatibility fallback
    /// ];
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .require_srtp(true)               // Mandatory
    ///     .srtp_profiles(profiles)
    ///     .build();
    /// 
    /// assert!(config.require_srtp);
    /// assert_eq!(config.srtp_profiles.len(), 2);
    /// ```
    pub fn srtp_profiles<I, S>(mut self, profiles: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.config.srtp_profiles = profiles.into_iter().map(Into::into).collect();
        self
    }
    
    /// Set RTP port range for media streams
    /// 
    /// Configures the range of UDP ports that can be used for RTP media streams.
    /// This is important for firewall configuration and network planning.
    /// Different ranges may be required for different network environments.
    /// 
    /// # Arguments
    /// 
    /// * `range` - Range of ports (start..end) to use for RTP media
    /// 
    /// # Examples
    /// 
    /// ## Firewall-Friendly Range
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .rtp_ports(16384..32768)          // Common firewall range
    ///     .build();
    /// 
    /// assert_eq!(config.rtp_port_start, 16384);
    /// assert_eq!(config.rtp_port_end, 32768);
    /// ```
    /// 
    /// ## Restricted Corporate Network
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .rtp_ports(50000..50100)          // Small range for corporate
    ///     .build();
    /// 
    /// assert_eq!(config.rtp_port_start, 50000);
    /// assert_eq!(config.rtp_port_end, 50100);
    /// assert_eq!(config.rtp_port_end - config.rtp_port_start, 100);
    /// ```
    /// 
    /// ## Wide Range for Busy Server
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .rtp_ports(10000..60000)          // Wide range for many calls
    ///     .build();
    /// 
    /// assert_eq!(config.rtp_port_start, 10000);
    /// assert_eq!(config.rtp_port_end, 60000);
    /// let available_ports = config.rtp_port_end - config.rtp_port_start;
    /// assert_eq!(available_ports, 50000);
    /// ```
    /// 
    /// ## Testing Environment
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .rtp_ports(20000..20010)          // Small range for testing
    ///     .build();
    /// 
    /// assert_eq!(config.rtp_port_start, 20000);
    /// assert_eq!(config.rtp_port_end, 20010);
    /// ```
    pub fn rtp_ports(mut self, range: std::ops::Range<u16>) -> Self {
        self.config.rtp_port_start = range.start;
        self.config.rtp_port_end = range.end;
        self
    }
    
    /// Set preferred packetization time (ptime)
    /// 
    /// Configures the preferred duration of audio data in each RTP packet,
    /// measured in milliseconds. Shorter ptime reduces latency but increases
    /// packet overhead. Longer ptime is more bandwidth-efficient but increases
    /// latency and may affect interactive applications.
    /// 
    /// # Arguments
    /// 
    /// * `ptime` - Preferred packetization time in milliseconds
    /// 
    /// # Common Values
    /// 
    /// - `10ms` - Very low latency, high overhead
    /// - `20ms` - Standard for most voice applications
    /// - `30ms` - Good for bandwidth-constrained networks
    /// - `40ms` - Maximum for most interactive applications
    /// 
    /// # Examples
    /// 
    /// ## Interactive/Gaming Applications
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .ptime(10)                        // Ultra-low latency
    ///     .codecs(vec!["opus"])             // Low-latency codec
    ///     .build();
    /// 
    /// assert_eq!(config.preferred_ptime, Some(10));
    /// ```
    /// 
    /// ## Standard Voice Calls
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .ptime(20)                        // Standard voice latency
    ///     .codecs(vec!["PCMU", "PCMA"])     // Standard codecs
    ///     .build();
    /// 
    /// assert_eq!(config.preferred_ptime, Some(20));
    /// ```
    /// 
    /// ## Bandwidth-Constrained Networks
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .ptime(30)                        // Efficient packetization
    ///     .max_bandwidth_kbps(32)           // Low bandwidth
    ///     .codecs(vec!["G.729", "GSM"])     // Efficient codecs
    ///     .build();
    /// 
    /// assert_eq!(config.preferred_ptime, Some(30));
    /// assert_eq!(config.max_bandwidth_kbps, Some(32));
    /// ```
    /// 
    /// ## Conference/Broadcast Applications
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .ptime(40)                        // Optimize for quality/bandwidth
    ///     .codecs(vec!["opus", "G722"])     // High-quality codecs
    ///     .build();
    /// 
    /// assert_eq!(config.preferred_ptime, Some(40));
    /// ```
    pub fn ptime(mut self, ptime: u8) -> Self {
        self.config.preferred_ptime = Some(ptime);
        self
    }
    
    /// Add a custom SDP attribute
    /// 
    /// Adds custom Session Description Protocol (SDP) attributes that will be
    /// included in media negotiation. This allows for vendor-specific extensions
    /// or advanced SDP features not directly supported by the builder.
    /// 
    /// # Arguments
    /// 
    /// * `key` - The SDP attribute key/name
    /// * `value` - The SDP attribute value
    /// 
    /// # Examples
    /// 
    /// ## Custom Tool Identification
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .custom_attribute("a=tool", "MyVoIPApp/1.0")
    ///     .build();
    /// 
    /// assert_eq!(
    ///     config.custom_sdp_attributes.get("a=tool"),
    ///     Some(&"MyVoIPApp/1.0".to_string())
    /// );
    /// ```
    /// 
    /// ## RTCP Configuration
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .custom_attribute("a=rtcp", "9 IN IP4 224.0.0.1")
    ///     .custom_attribute("a=rtcp-mux", "")
    ///     .build();
    /// 
    /// assert!(config.custom_sdp_attributes.contains_key("a=rtcp"));
    /// assert!(config.custom_sdp_attributes.contains_key("a=rtcp-mux"));
    /// ```
    /// 
    /// ## Vendor Extensions
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .custom_attribute("a=X-vendor-feature", "enabled")
    ///     .custom_attribute("a=X-quality-level", "high")
    ///     .custom_attribute("a=X-priority", "1")
    ///     .build();
    /// 
    /// assert_eq!(config.custom_sdp_attributes.len(), 3);
    /// assert_eq!(
    ///     config.custom_sdp_attributes.get("a=X-quality-level"),
    ///     Some(&"high".to_string())
    /// );
    /// ```
    /// 
    /// ## Multiple Attributes
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let version = "2.1.0".to_string();
    /// let config = MediaConfigBuilder::new()
    ///     .custom_attribute("a=tool", format!("RVoIP/{}", version))
    ///     .custom_attribute("a=orient", "portrait")
    ///     .custom_attribute("a=framerate", "30")
    ///     .build();
    /// 
    /// assert_eq!(
    ///     config.custom_sdp_attributes.get("a=tool"),
    ///     Some(&"RVoIP/2.1.0".to_string())
    /// );
    /// assert!(config.custom_sdp_attributes.len() >= 3);
    /// ```
    pub fn custom_attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.config.custom_sdp_attributes.insert(key.into(), value.into());
        self
    }
    
    /// Enable or disable all audio processing options at once
    /// 
    /// This is a convenience method that enables or disables echo cancellation,
    /// noise suppression, and automatic gain control simultaneously. This is
    /// useful when you want consistent audio processing behavior.
    /// 
    /// # Arguments
    /// 
    /// * `enabled` - Whether to enable all audio processing features
    /// 
    /// # Examples
    /// 
    /// ## Enable All Audio Processing
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .audio_processing(true)
    ///     .build();
    /// 
    /// assert!(config.echo_cancellation);
    /// assert!(config.noise_suppression);
    /// assert!(config.auto_gain_control);
    /// ```
    /// 
    /// ## Disable All Audio Processing
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .audio_processing(false)
    ///     .build();
    /// 
    /// assert!(!config.echo_cancellation);
    /// assert!(!config.noise_suppression);
    /// assert!(!config.auto_gain_control);
    /// ```
    /// 
    /// ## Selective Override
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .audio_processing(true)           // Enable all
    ///     .auto_gain_control(false)         // But disable AGC specifically
    ///     .build();
    /// 
    /// assert!(config.echo_cancellation);   // Still enabled
    /// assert!(config.noise_suppression);   // Still enabled
    /// assert!(!config.auto_gain_control);  // Disabled by override
    /// ```
    /// 
    /// ## Environment-Based Configuration
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let is_noisy_environment = true;
    /// let config = MediaConfigBuilder::new()
    ///     .audio_processing(is_noisy_environment)
    ///     .build();
    /// 
    /// if is_noisy_environment {
    ///     assert!(config.echo_cancellation);
    ///     assert!(config.noise_suppression);
    ///     assert!(config.auto_gain_control);
    /// }
    /// ```
    pub fn audio_processing(mut self, enabled: bool) -> Self {
        self.config.echo_cancellation = enabled;
        self.config.noise_suppression = enabled;
        self.config.auto_gain_control = enabled;
        self
    }
    
    /// Build the final MediaConfig
    /// 
    /// Consumes the builder and returns the configured MediaConfig instance.
    /// This is the final step in the builder pattern and produces the
    /// configuration object that can be used with VoIP clients.
    /// 
    /// # Returns
    /// 
    /// A `MediaConfig` instance with all the configured settings
    /// 
    /// # Examples
    /// 
    /// ## Basic Configuration
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .codecs(vec!["opus", "PCMU"])
    ///     .echo_cancellation(true)
    ///     .build();
    /// 
    /// // Use the built configuration
    /// assert_eq!(config.preferred_codecs, vec!["opus", "PCMU"]);
    /// assert!(config.echo_cancellation);
    /// ```
    /// 
    /// ## Complex Enterprise Configuration
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .codecs(vec!["opus", "G722", "PCMU"])
    ///     .require_srtp(true)
    ///     .srtp_profiles(vec!["AES_CM_128_HMAC_SHA1_80"])
    ///     .max_bandwidth_kbps(128)
    ///     .audio_processing(true)
    ///     .rtp_ports(16384..32768)
    ///     .ptime(20)
    ///     .dtmf(true)
    ///     .custom_attribute("a=tool", "Enterprise-VoIP/1.0")
    ///     .build();
    /// 
    /// // Verify comprehensive configuration
    /// assert_eq!(config.preferred_codecs.len(), 3);
    /// assert!(config.require_srtp);
    /// assert_eq!(config.max_bandwidth_kbps, Some(128));
    /// assert!(config.echo_cancellation);
    /// assert_eq!(config.rtp_port_start, 16384);
    /// assert_eq!(config.preferred_ptime, Some(20));
    /// assert!(config.dtmf_enabled);
    /// assert!(!config.custom_sdp_attributes.is_empty());
    /// ```
    /// 
    /// ## Configuration Validation
    /// 
    /// ```rust
    /// use rvoip_client_core::client::media_builder::MediaConfigBuilder;
    /// 
    /// let config = MediaConfigBuilder::new()
    ///     .rtp_ports(10000..20000)
    ///     .max_bandwidth_kbps(64)
    ///     .build();
    /// 
    /// // Verify port range is valid
    /// assert!(config.rtp_port_start < config.rtp_port_end);
    /// let port_range = config.rtp_port_end - config.rtp_port_start;
    /// assert!(port_range > 0);
    /// 
    /// // Verify bandwidth is reasonable
    /// assert!(config.max_bandwidth_kbps.unwrap() > 0);
    /// ```
    pub fn build(self) -> MediaConfig {
        self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_media_config_builder() {
        let config = MediaConfigBuilder::new()
            .codecs(vec!["opus", "PCMU"])
            .require_srtp(true)
            .echo_cancellation(true)
            .max_bandwidth_kbps(128)
            .rtp_ports(30000..40000)
            .ptime(20)
            .custom_attribute("a=tool", "rvoip-client")
            .build();
        
        assert_eq!(config.preferred_codecs, vec!["opus", "PCMU"]);
        assert!(config.require_srtp);
        assert!(config.echo_cancellation);
        assert_eq!(config.max_bandwidth_kbps, Some(128));
        assert_eq!(config.rtp_port_start, 30000);
        assert_eq!(config.rtp_port_end, 40000);
        assert_eq!(config.preferred_ptime, Some(20));
        assert_eq!(config.custom_sdp_attributes.get("a=tool"), Some(&"rvoip-client".to_string()));
    }
    
    #[test]
    fn test_audio_processing_shortcut() {
        let config = MediaConfigBuilder::new()
            .audio_processing(false)
            .build();
        
        assert!(!config.echo_cancellation);
        assert!(!config.noise_suppression);
        assert!(!config.auto_gain_control);
    }
} 