//! Common configuration types
//!
//! This module defines configuration types shared between client and server APIs.

use crate::api::common::frame::MediaFrameType;
use std::net::SocketAddr;

/// Security mode for transport
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityMode {
    /// No security (plain RTP)
    None,

    /// SRTP with pre-shared keys
    Srtp,

    /// Retained DTLS-SRTP mode identifier; unavailable in 0.3.5.
    DtlsSrtp,

    /// SDES-SRTP (keys exchanged via SDP Security Descriptions)
    SdesSrtp,

    /// Retained MIKEY-SRTP mode identifier; unavailable in 0.3.5.
    MikeySrtp,

    /// Retained ZRTP-SRTP mode identifier; unavailable in 0.3.5.
    ZrtpSrtp,
}

/// Key exchange method for SRTP security
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyExchangeMethod {
    /// Retained DTLS key-exchange identifier; unavailable in 0.3.5.
    DtlsSrtp,
    /// SDP Security Descriptions (SIP style)
    Sdes,
    /// Retained MIKEY identifier; unavailable in 0.3.5.
    Mikey,
    /// Retained ZRTP identifier; unavailable in 0.3.5.
    Zrtp,
    /// Pre-shared key (no key exchange)
    PreSharedKey,
}

impl From<SecurityMode> for Option<KeyExchangeMethod> {
    fn from(mode: SecurityMode) -> Self {
        match mode {
            SecurityMode::None => None,
            SecurityMode::Srtp => Some(KeyExchangeMethod::PreSharedKey),
            SecurityMode::DtlsSrtp => Some(KeyExchangeMethod::DtlsSrtp),
            SecurityMode::SdesSrtp => Some(KeyExchangeMethod::Sdes),
            SecurityMode::MikeySrtp => Some(KeyExchangeMethod::Mikey),
            SecurityMode::ZrtpSrtp => Some(KeyExchangeMethod::Zrtp),
        }
    }
}

impl KeyExchangeMethod {
    /// Get the security mode for this key exchange method
    pub fn to_security_mode(&self) -> SecurityMode {
        match self {
            KeyExchangeMethod::DtlsSrtp => SecurityMode::DtlsSrtp,
            KeyExchangeMethod::Sdes => SecurityMode::SdesSrtp,
            KeyExchangeMethod::Mikey => SecurityMode::MikeySrtp,
            KeyExchangeMethod::Zrtp => SecurityMode::ZrtpSrtp,
            KeyExchangeMethod::PreSharedKey => SecurityMode::Srtp,
        }
    }

    /// Check if this method requires network-based key exchange
    pub fn requires_network_exchange(&self) -> bool {
        match self {
            KeyExchangeMethod::DtlsSrtp
            | KeyExchangeMethod::Sdes
            | KeyExchangeMethod::Mikey
            | KeyExchangeMethod::Zrtp => true,
            KeyExchangeMethod::PreSharedKey => false,
        }
    }

    /// Check if this method exchanges keys via signaling (SDP)
    pub fn uses_signaling_exchange(&self) -> bool {
        match self {
            KeyExchangeMethod::Sdes => true,
            KeyExchangeMethod::DtlsSrtp
            | KeyExchangeMethod::Mikey
            | KeyExchangeMethod::Zrtp
            | KeyExchangeMethod::PreSharedKey => false,
        }
    }

    /// Check if this method exchanges keys via media path
    pub fn uses_media_exchange(&self) -> bool {
        match self {
            KeyExchangeMethod::Zrtp => true,
            KeyExchangeMethod::DtlsSrtp
            | KeyExchangeMethod::Sdes
            | KeyExchangeMethod::Mikey
            | KeyExchangeMethod::PreSharedKey => false,
        }
    }
}

impl SecurityMode {
    /// Check if security is enabled
    pub fn is_enabled(&self) -> bool {
        match self {
            SecurityMode::None => false,
            _ => true,
        }
    }

    /// Check if this mode requires SRTP
    pub fn requires_srtp(&self) -> bool {
        match self {
            SecurityMode::None => false,
            SecurityMode::Srtp
            | SecurityMode::DtlsSrtp
            | SecurityMode::SdesSrtp
            | SecurityMode::MikeySrtp
            | SecurityMode::ZrtpSrtp => true,
        }
    }

    /// Get the key exchange method for this security mode
    pub fn key_exchange_method(&self) -> Option<KeyExchangeMethod> {
        (*self).into()
    }
}

impl Default for SecurityMode {
    fn default() -> Self {
        SecurityMode::None
    }
}

/// Identity validation mechanism
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityValidation {
    /// No validation (use with caution)
    None,
    /// Fingerprint validation (DTLS)
    Fingerprint,
    /// Certificate validation (DTLS)
    Certificate,
    /// Custom validation
    Custom,
}

/// SRTP profiles for negotiation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SrtpProfile {
    /// AES_CM_128_HMAC_SHA1_80 (most common)
    AesCm128HmacSha1_80,
    /// AES_CM_128_HMAC_SHA1_32 (reduced auth tag for bandwidth savings)
    AesCm128HmacSha1_32,
    /// AEAD_AES_128_GCM identity (not implemented in this release)
    AesGcm128,
    /// AEAD_AES_256_GCM identity (not implemented in this release)
    AesGcm256,
}

impl SrtpProfile {
    /// Whether this profile has a complete cryptographic implementation.
    pub const fn is_supported(self) -> bool {
        matches!(self, Self::AesCm128HmacSha1_80 | Self::AesCm128HmacSha1_32)
    }

    /// Reject profiles that must not be advertised or negotiated.
    pub fn ensure_supported(self) -> crate::Result<()> {
        if self.is_supported() {
            Ok(())
        } else {
            Err(crate::Error::UnsupportedFeature(format!(
                "SRTP profile {self:?} is not implemented"
            )))
        }
    }

    /// Return the SDP/DTLS name only for a profile implemented in this release.
    pub fn advertised_name(self) -> crate::Result<&'static str> {
        match self {
            Self::AesCm128HmacSha1_80 => Ok("AES_CM_128_HMAC_SHA1_80"),
            Self::AesCm128HmacSha1_32 => Ok("AES_CM_128_HMAC_SHA1_32"),
            Self::AesGcm128 | Self::AesGcm256 => Err(crate::Error::UnsupportedFeature(format!(
                "SRTP profile {self:?} is not implemented"
            ))),
        }
    }

    /// Convert an API profile to the exact implemented cryptographic suite.
    pub fn crypto_suite(self) -> crate::Result<crate::srtp::SrtpCryptoSuite> {
        match self {
            Self::AesCm128HmacSha1_80 => Ok(crate::srtp::SRTP_AES128_CM_SHA1_80),
            Self::AesCm128HmacSha1_32 => Ok(crate::srtp::SRTP_AES128_CM_SHA1_32),
            Self::AesGcm128 | Self::AesGcm256 => Err(crate::Error::UnsupportedFeature(format!(
                "SRTP profile {self:?} is not implemented"
            ))),
        }
    }
}

/// Validate and convert a complete profile list without dropping entries.
pub(crate) fn implemented_srtp_suites(
    profiles: &[SrtpProfile],
) -> Result<Vec<crate::srtp::SrtpCryptoSuite>, crate::api::common::error::SecurityError> {
    if profiles.is_empty() {
        return Err(crate::api::common::error::SecurityError::Configuration(
            "at least one implemented SRTP profile is required".to_string(),
        ));
    }

    profiles
        .iter()
        .copied()
        .map(|profile| profile.crypto_suite().map_err(Into::into))
        .collect()
}

/// Resolve the single exact suite required by direct pre-shared-key transports.
///
/// Direct PSK transports do not perform a profile negotiation. Accepting a
/// preference list here would make the advertised suite and the on-wire suite
/// diverge, so ambiguity is rejected before keys are installed.
pub(crate) fn implemented_single_srtp_suite(
    profiles: &[SrtpProfile],
) -> Result<crate::srtp::SrtpCryptoSuite, crate::api::common::error::SecurityError> {
    if profiles.len() != 1 {
        return Err(crate::api::common::error::SecurityError::Configuration(
            format!(
                "direct pre-shared-key SRTP requires exactly one implemented profile, got {}",
                profiles.len()
            ),
        ));
    }

    profiles[0].crypto_suite().map_err(Into::into)
}

/// Resolve one exact advertised suite name without falling back to a default.
pub(crate) fn implemented_single_srtp_suite_from_names(
    profiles: &[String],
) -> Result<crate::srtp::SrtpCryptoSuite, crate::api::common::error::SecurityError> {
    if profiles.len() != 1 {
        return Err(crate::api::common::error::SecurityError::Configuration(
            format!(
                "direct pre-shared-key SRTP requires exactly one implemented profile, got {}",
                profiles.len()
            ),
        ));
    }

    match profiles[0].as_str() {
        "AES_CM_128_HMAC_SHA1_80" => Ok(crate::srtp::SRTP_AES128_CM_SHA1_80),
        "AES_CM_128_HMAC_SHA1_32" => Ok(crate::srtp::SRTP_AES128_CM_SHA1_32),
        name => Err(
            crate::api::common::error::SecurityError::UnsupportedFeature(format!(
                "SRTP profile {name} is not implemented"
            )),
        ),
    }
}

/// Convert a complete profile list to names without dropping unsupported entries.
pub(crate) fn implemented_srtp_profile_names(
    profiles: &[SrtpProfile],
) -> Result<Vec<String>, crate::api::common::error::SecurityError> {
    profiles
        .iter()
        .copied()
        .map(|profile| {
            profile
                .advertised_name()
                .map(str::to_string)
                .map_err(Into::into)
        })
        .collect()
}

/// Network condition preset for buffer configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkPreset {
    /// Minimal latency, good for LAN
    LowLatency,

    /// Balanced preset, good for stable broadband
    Balanced,

    /// Resilient preset, good for mobile or unstable networks
    Resilient,

    /// Maximum protection, for very unstable networks
    HighProtection,
}

/// Base transport configuration shared by client and server
#[derive(Debug, Clone)]
pub struct BaseTransportConfig {
    /// Local address to bind to
    pub local_address: Option<SocketAddr>,
    /// Whether to use RTCP multiplexing (RTP and RTCP on same port)
    pub rtcp_mux: bool,
    /// Media types enabled for this transport
    pub media_types: Vec<MediaFrameType>,
    /// Maximum transmission unit size
    pub mtu: usize,
}

/// Security information for SDP exchange
#[derive(Debug, Clone)]
pub struct SecurityInfo {
    /// Security mode (None, Srtp, DtlsSrtp)
    pub mode: SecurityMode,

    /// DTLS fingerprint (for DtlsSrtp)
    pub fingerprint: Option<String>,

    /// Fingerprint algorithm (for DtlsSrtp)
    pub fingerprint_algorithm: Option<String>,

    /// Crypto suites in order of preference
    pub crypto_suites: Vec<String>,

    /// Key parameters (for Srtp)
    pub key_params: Option<String>,

    /// Selected SRTP profile
    pub srtp_profile: Option<String>,
}

impl Default for SecurityInfo {
    fn default() -> Self {
        Self {
            mode: SecurityMode::None,
            fingerprint: None,
            fingerprint_algorithm: None,
            crypto_suites: Vec::new(),
            key_params: None,
            srtp_profile: None,
        }
    }
}

/// Predefined security profiles for common use cases
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityProfile {
    /// No security - plain RTP
    Unsecured,

    /// Basic SRTP with pre-shared key (for simple deployments)
    SrtpBasic,

    /// Retained DTLS-SRTP self-signed profile identifier.
    ///
    /// This variant remains for source compatibility. DTLS-SRTP is unavailable
    /// in 0.3.5 and [`SecurityConfig::validate`] rejects it.
    DtlsSrtpSelfSigned,

    /// Retained DTLS-SRTP certificate profile identifier.
    ///
    /// This variant remains for source compatibility. DTLS-SRTP is unavailable
    /// in 0.3.5 and [`SecurityConfig::validate`] rejects it.
    DtlsSrtpCertificate,

    /// SDES-SRTP for SIP/SDP key exchange (telephony systems)
    SdesSrtp,

    /// Retained MIKEY pre-shared-key profile identifier.
    ///
    /// This variant remains for source compatibility. MIKEY is unavailable in
    /// 0.3.5 and [`SecurityConfig::validate`] rejects it.
    MikeyPsk,

    /// Retained MIKEY public-key profile identifier.
    ///
    /// This variant remains for source compatibility. MIKEY is unavailable in
    /// 0.3.5 and [`SecurityConfig::validate`] rejects it.
    MikeyPke,

    /// Retained ZRTP profile identifier.
    ///
    /// This variant remains for source compatibility. ZRTP is unavailable in
    /// 0.3.5 and [`SecurityConfig::validate`] rejects it.
    ZrtpP2P,

    /// Custom configuration (use the detailed SecurityConfig)
    Custom,
}

impl Default for SecurityProfile {
    fn default() -> Self {
        // Security must be selected explicitly so an incomplete key exchange
        // cannot be enabled merely by constructing a default configuration.
        SecurityProfile::Unsecured
    }
}

/// Complete security configuration with reasonable defaults
/// This struct makes it easy to configure security without understanding
/// all the underlying details of DTLS-SRTP, SRTP, etc.
#[derive(Debug, Clone)]
pub struct SecurityConfig {
    /// Security profile (for common configurations)
    pub profile: SecurityProfile,

    /// Security mode (None, SRTP, DTLS-SRTP)
    pub mode: SecurityMode,

    /// Whether security is required (fail if not available)
    pub required: bool,

    /// SRTP profiles in order of preference
    pub srtp_profiles: Vec<SrtpProfile>,

    /// Certificate file path (PEM format)
    pub certificate_path: Option<String>,

    /// Private key file path (PEM format)
    pub private_key_path: Option<String>,

    /// Fingerprint algorithm for DTLS
    pub fingerprint_algorithm: String,

    /// Pre-shared key for SRTP (used when mode is Srtp)
    pub srtp_key: Option<Vec<u8>>,

    /// Require client certificate validation
    pub require_client_certificate: bool,

    /// Remote fingerprint (if known, e.g. from SDP)
    pub remote_fingerprint: Option<String>,

    /// Remote fingerprint algorithm
    pub remote_fingerprint_algorithm: Option<String>,

    /// Certificate data in DER format (for MIKEY-PKE)
    pub certificate_data: Option<Vec<u8>>,

    /// Private key data in PKCS#8 DER format (for MIKEY-PKE)
    pub private_key_data: Option<Vec<u8>>,

    /// Peer certificate data in DER format (for MIKEY-PKE)
    pub peer_certificate_data: Option<Vec<u8>>,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            profile: SecurityProfile::default(),
            mode: SecurityMode::None,
            required: false,
            srtp_profiles: Vec::new(),
            certificate_path: None,
            private_key_path: None,
            fingerprint_algorithm: "sha-256".to_string(),
            srtp_key: None,
            require_client_certificate: false,
            remote_fingerprint: None,
            remote_fingerprint_algorithm: None,
            certificate_data: None,
            private_key_data: None,
            peer_certificate_data: None,
        }
    }
}

impl SecurityConfig {
    /// Whether this configuration retains material that only the unavailable
    /// certificate/fingerprint key-exchange paths could consume.
    fn has_certificate_or_fingerprint_material(&self) -> bool {
        self.certificate_path.is_some()
            || self.private_key_path.is_some()
            || self.fingerprint_algorithm != "sha-256"
            || self.require_client_certificate
            || self.remote_fingerprint.is_some()
            || self.remote_fingerprint_algorithm.is_some()
            || self.certificate_data.is_some()
            || self.private_key_data.is_some()
            || self.peer_certificate_data.is_some()
    }

    /// Validate all configured profiles before any security method is selected.
    pub fn validate(&self) -> Result<(), crate::api::common::error::SecurityError> {
        if self.mode == SecurityMode::DtlsSrtp
            || matches!(
                self.profile,
                SecurityProfile::DtlsSrtpSelfSigned | SecurityProfile::DtlsSrtpCertificate
            )
        {
            return Err(
                crate::api::common::error::SecurityError::UnsupportedFeature(
                    "DTLS-SRTP is not complete and is unavailable".to_string(),
                ),
            );
        }
        if self.mode == SecurityMode::MikeySrtp
            || matches!(
                self.profile,
                SecurityProfile::MikeyPsk | SecurityProfile::MikeyPke
            )
        {
            return Err(
                crate::api::common::error::SecurityError::UnsupportedFeature(
                    "MIKEY key exchange is not complete and is unavailable".to_string(),
                ),
            );
        }
        if self.mode == SecurityMode::ZrtpSrtp || self.profile == SecurityProfile::ZrtpP2P {
            return Err(
                crate::api::common::error::SecurityError::UnsupportedFeature(
                    "ZRTP key exchange is not complete and is unavailable".to_string(),
                ),
            );
        }
        if let Some(profile) = self
            .srtp_profiles
            .iter()
            .copied()
            .find(|profile| !profile.is_supported())
        {
            return Err(
                crate::api::common::error::SecurityError::UnsupportedFeature(format!(
                    "SRTP profile {profile:?} is not implemented"
                )),
            );
        }

        let expected_mode = match self.profile {
            SecurityProfile::Unsecured => Some(SecurityMode::None),
            SecurityProfile::SrtpBasic => Some(SecurityMode::Srtp),
            SecurityProfile::DtlsSrtpSelfSigned | SecurityProfile::DtlsSrtpCertificate => {
                Some(SecurityMode::DtlsSrtp)
            }
            SecurityProfile::SdesSrtp => Some(SecurityMode::SdesSrtp),
            SecurityProfile::MikeyPsk | SecurityProfile::MikeyPke => Some(SecurityMode::MikeySrtp),
            SecurityProfile::ZrtpP2P => Some(SecurityMode::ZrtpSrtp),
            SecurityProfile::Custom => None,
        };
        if expected_mode.is_some_and(|expected| expected != self.mode) {
            return Err(crate::api::common::error::SecurityError::Configuration(
                format!(
                    "security profile {:?} is incompatible with mode {:?}",
                    self.profile, self.mode
                ),
            ));
        }

        if self.mode == SecurityMode::None {
            if self.required
                || !self.srtp_profiles.is_empty()
                || self.srtp_key.is_some()
                || self.has_certificate_or_fingerprint_material()
            {
                return Err(crate::api::common::error::SecurityError::Configuration(
                    "unsecured mode cannot require security or retain key, certificate, or fingerprint material"
                        .to_string(),
                ));
            }
            return Ok(());
        }

        if matches!(self.mode, SecurityMode::Srtp | SecurityMode::SdesSrtp)
            && self.has_certificate_or_fingerprint_material()
        {
            return Err(crate::api::common::error::SecurityError::Configuration(
                format!(
                    "{:?} mode cannot retain unused certificate or fingerprint material",
                    self.mode
                ),
            ));
        }

        if self.mode == SecurityMode::SdesSrtp && self.srtp_key.is_some() {
            return Err(crate::api::common::error::SecurityError::Configuration(
                "SDES mode cannot retain an unused direct-SRTP key".to_string(),
            ));
        }

        implemented_srtp_suites(&self.srtp_profiles)?;

        if self.mode == SecurityMode::Srtp {
            implemented_single_srtp_suite(&self.srtp_profiles)?;
            let key = self.srtp_key.as_ref().ok_or_else(|| {
                crate::api::common::error::SecurityError::Configuration(
                    "SRTP mode requires a 16-byte key and 14-byte salt".to_string(),
                )
            })?;
            if key.len() != 30 {
                return Err(crate::api::common::error::SecurityError::Configuration(
                    format!(
                        "AES-128 SRTP key material must be exactly 30 bytes, got {}",
                        key.len()
                    ),
                ));
            }
        }

        Ok(())
    }

    /// Create a security configuration from a predefined profile
    pub fn from_profile(profile: SecurityProfile) -> Self {
        match profile {
            SecurityProfile::Unsecured => Self {
                profile,
                mode: SecurityMode::None,
                required: false,
                srtp_profiles: vec![],
                certificate_path: None,
                private_key_path: None,
                fingerprint_algorithm: "sha-256".to_string(),
                srtp_key: None,
                require_client_certificate: false,
                remote_fingerprint: None,
                remote_fingerprint_algorithm: None,
                certificate_data: None,
                private_key_data: None,
                peer_certificate_data: None,
            },

            SecurityProfile::SrtpBasic => {
                Self {
                    profile,
                    mode: SecurityMode::Srtp,
                    required: true,
                    srtp_profiles: vec![SrtpProfile::AesCm128HmacSha1_80],
                    certificate_path: None,
                    private_key_path: None,
                    fingerprint_algorithm: "sha-256".to_string(),
                    // Default key will need to be set by the user
                    srtp_key: None,
                    require_client_certificate: false,
                    remote_fingerprint: None,
                    remote_fingerprint_algorithm: None,
                    certificate_data: None,
                    private_key_data: None,
                    peer_certificate_data: None,
                }
            }

            SecurityProfile::DtlsSrtpSelfSigned => {
                Self {
                    profile,
                    mode: SecurityMode::DtlsSrtp,
                    required: true,
                    srtp_profiles: vec![SrtpProfile::AesCm128HmacSha1_80],
                    certificate_path: None, // Will use self-signed
                    private_key_path: None, // Will use self-signed
                    fingerprint_algorithm: "sha-256".to_string(),
                    srtp_key: None, // Not needed for DTLS-SRTP
                    require_client_certificate: false,
                    remote_fingerprint: None,
                    remote_fingerprint_algorithm: None,
                    certificate_data: None,
                    private_key_data: None,
                    peer_certificate_data: None,
                }
            }

            SecurityProfile::DtlsSrtpCertificate => {
                Self {
                    profile,
                    mode: SecurityMode::DtlsSrtp,
                    required: true,
                    srtp_profiles: vec![SrtpProfile::AesCm128HmacSha1_80],
                    // Paths need to be set by user
                    certificate_path: None,
                    private_key_path: None,
                    fingerprint_algorithm: "sha-256".to_string(),
                    srtp_key: None,                    // Not needed for DTLS-SRTP
                    require_client_certificate: false, // Optional in most deployments
                    remote_fingerprint: None,
                    remote_fingerprint_algorithm: None,
                    certificate_data: None,
                    private_key_data: None,
                    peer_certificate_data: None,
                }
            }

            SecurityProfile::SdesSrtp => Self {
                profile,
                mode: SecurityMode::SdesSrtp,
                required: true,
                srtp_profiles: vec![SrtpProfile::AesCm128HmacSha1_80],
                certificate_path: None,
                private_key_path: None,
                fingerprint_algorithm: "sha-256".to_string(),
                srtp_key: None,
                require_client_certificate: false,
                remote_fingerprint: None,
                remote_fingerprint_algorithm: None,
                certificate_data: None,
                private_key_data: None,
                peer_certificate_data: None,
            },

            SecurityProfile::MikeyPsk => Self {
                profile,
                mode: SecurityMode::MikeySrtp,
                required: true,
                srtp_profiles: vec![SrtpProfile::AesCm128HmacSha1_80],
                certificate_path: None,
                private_key_path: None,
                fingerprint_algorithm: "sha-256".to_string(),
                srtp_key: None,
                require_client_certificate: false,
                remote_fingerprint: None,
                remote_fingerprint_algorithm: None,
                certificate_data: None,
                private_key_data: None,
                peer_certificate_data: None,
            },

            SecurityProfile::MikeyPke => Self {
                profile,
                mode: SecurityMode::MikeySrtp,
                required: true,
                srtp_profiles: vec![SrtpProfile::AesCm128HmacSha1_80],
                certificate_path: None,
                private_key_path: None,
                fingerprint_algorithm: "sha-256".to_string(),
                srtp_key: None,
                require_client_certificate: false,
                remote_fingerprint: None,
                remote_fingerprint_algorithm: None,
                certificate_data: None,
                private_key_data: None,
                peer_certificate_data: None,
            },

            SecurityProfile::ZrtpP2P => Self {
                profile,
                mode: SecurityMode::ZrtpSrtp,
                required: true,
                srtp_profiles: vec![SrtpProfile::AesCm128HmacSha1_80],
                certificate_path: None,
                private_key_path: None,
                fingerprint_algorithm: "sha-256".to_string(),
                srtp_key: None,
                require_client_certificate: false,
                remote_fingerprint: None,
                remote_fingerprint_algorithm: None,
                certificate_data: None,
                private_key_data: None,
                peer_certificate_data: None,
            },

            SecurityProfile::Custom => {
                let mut config = Self::default();
                config.profile = SecurityProfile::Custom;
                config
            }
        }
    }

    /// Create an unsecured configuration (plain RTP)
    pub fn unsecured() -> Self {
        Self::from_profile(SecurityProfile::Unsecured)
    }

    /// Create a basic SRTP configuration with a pre-shared key
    pub fn srtp_with_key(key: Vec<u8>) -> Self {
        let mut config = Self::from_profile(SecurityProfile::SrtpBasic);
        config.srtp_key = Some(key);
        config
    }

    /// Create a retained DTLS-SRTP self-signed configuration template.
    ///
    /// This constructor remains for source compatibility. DTLS-SRTP is
    /// unavailable in 0.3.5 and [`Self::validate`] returns an
    /// unsupported-feature error for this configuration.
    pub fn webrtc_compatible() -> Self {
        Self::from_profile(SecurityProfile::DtlsSrtpSelfSigned)
    }

    /// Create a retained DTLS-SRTP certificate configuration template.
    ///
    /// This constructor remains for source compatibility. DTLS-SRTP is
    /// unavailable in 0.3.5 and [`Self::validate`] returns an
    /// unsupported-feature error for this configuration.
    pub fn dtls_with_certificate(cert_path: String, key_path: String) -> Self {
        let mut config = Self::from_profile(SecurityProfile::DtlsSrtpCertificate);
        config.certificate_path = Some(cert_path);
        config.private_key_path = Some(key_path);
        config
    }

    /// Create an SDES-SRTP configuration for SIP/SDP key exchange
    pub fn sdes_srtp() -> Self {
        Self::from_profile(SecurityProfile::SdesSrtp)
    }

    /// Create a retained MIKEY pre-shared-key configuration template.
    ///
    /// This constructor remains for source compatibility. MIKEY is unavailable
    /// in 0.3.5 and [`Self::validate`] returns an unsupported-feature error for
    /// this configuration.
    pub fn mikey_psk() -> Self {
        Self::from_profile(SecurityProfile::MikeyPsk)
    }

    /// Create a MIKEY-SRTP public-key configuration template.
    ///
    /// This constructor is retained for source compatibility. The incomplete
    /// MIKEY-PKE protocol is unavailable and [`Self::validate`] returns an
    /// unsupported-feature error for this configuration.
    pub fn mikey_pke() -> Self {
        Self::from_profile(SecurityProfile::MikeyPke)
    }

    /// Create a ZRTP configuration template.
    ///
    /// This constructor is retained for source compatibility. The incomplete
    /// ZRTP protocol is unavailable and [`Self::validate`] returns an
    /// unsupported-feature error for this configuration.
    pub fn zrtp_p2p() -> Self {
        Self::from_profile(SecurityProfile::ZrtpP2P)
    }

    /// Create a configuration whose first method is its explicit selection.
    ///
    /// This does not silently fall back around unavailable methods. Call
    /// [`Self::validate`] and negotiate only a method it accepts.
    pub fn multi_method(methods: Vec<KeyExchangeMethod>) -> Self {
        let primary_method = methods.first().copied().unwrap_or(KeyExchangeMethod::Sdes);
        let mut config = Self::from_profile(SecurityProfile::Custom);
        config.mode = primary_method.to_security_mode();
        config.required = true;
        config.srtp_profiles = vec![SrtpProfile::AesCm128HmacSha1_80];
        config
    }

    // Predefined profile combinations for common SIP scenarios

    /// SIP enterprise configuration using the implemented SDES exchange.
    pub fn sip_enterprise() -> Self {
        Self::sdes_srtp()
    }

    /// SIP operator configuration (SDES with operator keys)
    pub fn sip_operator() -> Self {
        Self::sdes_srtp()
    }

    /// SIP peer-to-peer configuration using the implemented SDES exchange.
    pub fn sip_peer_to_peer() -> Self {
        Self::sdes_srtp()
    }

    /// SIP<->WebRTC bridge configuration using the implemented SDES exchange.
    pub fn sip_webrtc_bridge() -> Self {
        Self::multi_method(vec![KeyExchangeMethod::Sdes])
    }

    /// Set the remote party's fingerprint (e.g. from SDP)
    pub fn with_remote_fingerprint(
        mut self,
        fingerprint: String,
        algorithm: Option<String>,
    ) -> Self {
        self.remote_fingerprint = Some(fingerprint);
        self.remote_fingerprint_algorithm =
            algorithm.or_else(|| Some(self.fingerprint_algorithm.clone()));
        self
    }

    /// Mark an implemented security exchange as optional at the policy layer.
    ///
    /// This does not enable plaintext fallback for an unavailable method;
    /// [`Self::validate`] still returns an unsupported-feature error.
    pub fn with_optional_security(mut self) -> Self {
        self.required = false;
        self
    }

    /// Set certificate data for PKE mode (DER format)
    pub fn with_certificate_data(mut self, cert_data: Vec<u8>, private_key_data: Vec<u8>) -> Self {
        self.certificate_data = Some(cert_data);
        self.private_key_data = Some(private_key_data);
        self
    }

    /// Set peer certificate data for PKE mode (DER format)
    pub fn with_peer_certificate_data(mut self, peer_cert_data: Vec<u8>) -> Self {
        self.peer_certificate_data = Some(peer_cert_data);
        self
    }

    /// Create a retained MIKEY-PKE certificate configuration template.
    ///
    /// MIKEY-PKE is unavailable in 0.3.5 and [`Self::validate`] returns an
    /// unsupported-feature error for the returned configuration.
    pub fn mikey_pke_with_certificates(
        cert_data: Vec<u8>,
        private_key_data: Vec<u8>,
        peer_cert_data: Vec<u8>,
    ) -> Self {
        Self::mikey_pke()
            .with_certificate_data(cert_data, private_key_data)
            .with_peer_certificate_data(peer_cert_data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_psk_requires_one_exact_profile() {
        let mut config = SecurityConfig::srtp_with_key(vec![0x55; 30]);
        config.srtp_profiles = vec![
            SrtpProfile::AesCm128HmacSha1_80,
            SrtpProfile::AesCm128HmacSha1_32,
        ];

        assert!(matches!(
            config.validate(),
            Err(crate::api::common::error::SecurityError::Configuration(_))
        ));
    }

    #[test]
    fn incomplete_key_exchanges_fail_configuration_validation() {
        for config in [
            SecurityConfig::webrtc_compatible(),
            SecurityConfig::dtls_with_certificate("cert.pem".into(), "key.pem".into()),
            SecurityConfig::mikey_psk(),
            SecurityConfig::mikey_pke(),
            SecurityConfig::zrtp_p2p(),
        ] {
            assert!(matches!(
                config.validate(),
                Err(crate::api::common::error::SecurityError::UnsupportedFeature(_))
            ));
        }
    }

    #[test]
    fn built_in_defaults_and_sip_presets_advertise_only_available_methods() {
        let default = SecurityConfig::default();
        assert_eq!(default.profile, SecurityProfile::Unsecured);
        assert_eq!(default.mode, SecurityMode::None);
        assert!(!default.required);
        assert!(default.srtp_profiles.is_empty());
        assert!(default.validate().is_ok());

        for config in [
            SecurityConfig::sip_enterprise(),
            SecurityConfig::sip_operator(),
            SecurityConfig::sip_peer_to_peer(),
            SecurityConfig::sip_webrtc_bridge(),
        ] {
            assert_eq!(config.mode, SecurityMode::SdesSrtp);
            assert!(config.validate().is_ok());
        }
    }

    #[test]
    fn profile_mode_and_unsecured_material_must_be_coherent() {
        let mut secure_profile_in_plaintext = SecurityConfig::srtp_with_key(vec![0x44; 30]);
        secure_profile_in_plaintext.mode = SecurityMode::None;
        assert!(matches!(
            secure_profile_in_plaintext.validate(),
            Err(crate::api::common::error::SecurityError::Configuration(_))
        ));

        let mut plaintext_profile_in_srtp = SecurityConfig::unsecured();
        plaintext_profile_in_srtp.mode = SecurityMode::Srtp;
        plaintext_profile_in_srtp.required = true;
        plaintext_profile_in_srtp.srtp_profiles = vec![SrtpProfile::AesCm128HmacSha1_80];
        plaintext_profile_in_srtp.srtp_key = Some(vec![0x55; 30]);
        assert!(matches!(
            plaintext_profile_in_srtp.validate(),
            Err(crate::api::common::error::SecurityError::Configuration(_))
        ));

        for mut plaintext_with_material in [
            SecurityConfig::unsecured(),
            SecurityConfig::from_profile(SecurityProfile::Custom),
        ] {
            plaintext_with_material.srtp_profiles = vec![SrtpProfile::AesCm128HmacSha1_80];
            assert!(matches!(
                plaintext_with_material.validate(),
                Err(crate::api::common::error::SecurityError::Configuration(_))
            ));
            plaintext_with_material.srtp_profiles.clear();
            plaintext_with_material.srtp_key = Some(vec![0x66; 30]);
            assert!(matches!(
                plaintext_with_material.validate(),
                Err(crate::api::common::error::SecurityError::Configuration(_))
            ));
        }
    }

    fn configurations_with_certificate_or_fingerprint_material(
        base: SecurityConfig,
    ) -> Vec<SecurityConfig> {
        let mut configs = Vec::new();

        let mut config = base.clone();
        config.certificate_path = Some("certificate.pem".to_string());
        configs.push(config);
        let mut config = base.clone();
        config.private_key_path = Some("private-key.pem".to_string());
        configs.push(config);
        let mut config = base.clone();
        config.fingerprint_algorithm = "sha-512".to_string();
        configs.push(config);
        let mut config = base.clone();
        config.require_client_certificate = true;
        configs.push(config);
        let mut config = base.clone();
        config.remote_fingerprint = Some("AA:BB".to_string());
        configs.push(config);
        let mut config = base.clone();
        config.remote_fingerprint_algorithm = Some("sha-256".to_string());
        configs.push(config);
        let mut config = base.clone();
        config.certificate_data = Some(vec![1]);
        configs.push(config);
        let mut config = base.clone();
        config.private_key_data = Some(vec![2]);
        configs.push(config);
        let mut config = base;
        config.peer_certificate_data = Some(vec![3]);
        configs.push(config);

        configs
    }

    #[test]
    fn implemented_modes_reject_ignored_security_material() {
        for base in [
            SecurityConfig::unsecured(),
            SecurityConfig::srtp_with_key(vec![0x31; 30]),
            SecurityConfig::sdes_srtp(),
        ] {
            for config in configurations_with_certificate_or_fingerprint_material(base) {
                assert!(matches!(
                    config.validate(),
                    Err(crate::api::common::error::SecurityError::Configuration(_))
                ));
            }
        }

        let mut sdes_with_direct_key = SecurityConfig::sdes_srtp();
        sdes_with_direct_key.srtp_key = Some(vec![0x41; 30]);
        assert!(matches!(
            sdes_with_direct_key.validate(),
            Err(crate::api::common::error::SecurityError::Configuration(_))
        ));
    }

    #[test]
    fn direct_aes_128_key_and_salt_length_is_exact() {
        for length in [0, 16, 29, 31, 32] {
            assert!(matches!(
                SecurityConfig::srtp_with_key(vec![0x77; length]).validate(),
                Err(crate::api::common::error::SecurityError::Configuration(_))
            ));
        }
        assert!(SecurityConfig::srtp_with_key(vec![0x77; 30])
            .validate()
            .is_ok());
    }
}
