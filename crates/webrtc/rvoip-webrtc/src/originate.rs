//! Typed, bounded options for one target-contacting WebRTC origination.
//!
//! [`rvoip_core::adapter::OriginateRequest::target`] predates adapter-owned
//! originate contexts and remains available to legacy local-offer callers.
//! New outbound signaling uses [`WebRtcOriginateContext`]: construction
//! validates and freezes the exact endpoint, signaling protocol, ICE policy,
//! credential provider, and network boundary before the request reaches the
//! adapter.

use std::collections::BTreeSet;
use std::fmt;
use std::net::{IpAddr, Ipv6Addr};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rvoip_core::DataReliability;
#[cfg(feature = "tls-rustls")]
use sha2::{Digest, Sha256};
use thiserror::Error;
use url::Url;
use zeroize::Zeroize;

use crate::config::IceServerConfig;

/// Maximum serialized signaling target accepted at the adapter boundary.
pub const MAX_WEBRTC_TARGET_BYTES: usize = 2_048;
/// Maximum bearer credential size accepted from a provider.
pub const MAX_WEBRTC_BEARER_BYTES: usize = 4_096;
/// Maximum opaque credential-partition identifier size.
pub const MAX_CREDENTIAL_PARTITION_BYTES: usize = 128;
/// Maximum STUN/TURN entries scoped to one outbound peer.
pub const MAX_WEBRTC_ORIGINATE_ICE_SERVERS: usize = 16;
/// Maximum URLs retained by one scoped STUN/TURN entry.
pub const MAX_WEBRTC_ORIGINATE_ICE_URLS: usize = 8;
/// Maximum UTF-8 size of one STUN/TURN URL.
pub const MAX_WEBRTC_ORIGINATE_ICE_URL_BYTES: usize = 2_048;
/// Maximum UTF-8 size of a TURN username or credential.
pub const MAX_WEBRTC_ORIGINATE_ICE_CREDENTIAL_BYTES: usize = 4_096;
/// Maximum distinct audio codecs admitted by one outbound profile.
pub const MAX_WEBRTC_ORIGINATE_AUDIO_CODECS: usize = 8;
/// Maximum caller-requested DataChannels created before the initial offer.
///
/// The adapter also retains its historical `rvoip-messages` bootstrap
/// channel, keeping the complete route at the 64-channel operational cap.
pub const MAX_WEBRTC_ORIGINATE_PREOPENED_DATA_CHANNELS: usize = 63;
/// Maximum PEM trust bundle accepted for one outbound signaling context.
#[cfg(feature = "tls-rustls")]
pub const MAX_WEBRTC_TLS_TRUST_BYTES: usize = 256 * 1024;
/// Maximum certificates accepted in one outbound signaling trust bundle.
#[cfg(feature = "tls-rustls")]
pub const MAX_WEBRTC_TLS_TRUST_CERTIFICATES: usize = 64;

/// Explicit signaling exchange used by one outbound WebRTC leg.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum WebRtcSignalingMode {
    /// One persistent `rvoip.webrtc.v1` WebSocket session carries the offer,
    /// answer, trickle candidates, and terminal BYE.
    WebSocket,
    /// RFC 9725 WHIP resource creation and lifecycle.
    Whip,
    /// WHEP draft-04 subscriber exchange.
    ///
    /// Supports both the direct `201 Created` SDP-answer path and the
    /// `406 Not Acceptable` server counter-offer path. Playback media is
    /// negotiated receive-only and the retained session resource is deleted
    /// during route teardown.
    Whep,
}

/// ICE delivery policy for one signaling exchange.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum WebRtcIceExchangePolicy {
    /// Wait for a complete local SDP before contacting the target.
    #[default]
    FullGather,
    /// Send candidates incrementally over the retained signaling resource.
    Trickle,
}

/// Audio codecs which an immutable outbound WebRTC profile may admit.
///
/// Keeping this typed prevents a profile from silently advertising a codec
/// that the rvoip WebRTC media engine cannot actually negotiate.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum WebRtcAudioCodec {
    Opus,
    Pcmu,
    Pcma,
}

impl WebRtcAudioCodec {
    pub(crate) fn matches_name(self, name: &str) -> bool {
        match self {
            Self::Opus => name.eq_ignore_ascii_case("opus"),
            Self::Pcmu => {
                name.eq_ignore_ascii_case("g.711-mu") || name.eq_ignore_ascii_case("pcmu")
            }
            Self::Pcma => name.eq_ignore_ascii_case("g.711-a") || name.eq_ignore_ascii_case("pcma"),
        }
    }
}

/// Fixed, payload-free validation failures for outbound WebRTC options.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum WebRtcOriginateContextError {
    #[error("the WebRTC signaling target is invalid")]
    InvalidTarget,
    #[error("the WebRTC signaling target is too large")]
    TargetTooLarge,
    #[error("the WebRTC signaling target contains credentials")]
    CredentialsInTarget,
    #[error("the WebRTC signaling target contains a query or fragment")]
    TargetMetadataForbidden,
    #[error("the WebRTC signaling scheme does not match the selected mode")]
    SignalingModeMismatch,
    #[error("the WebRTC signaling target requires a secure transport")]
    InsecureTransportForbidden,
    #[error("the WebRTC signaling target port is not allowed")]
    PortForbidden,
    #[error("the WebRTC signaling target address is not allowed")]
    AddressForbidden,
    #[error("the WebRTC signaling target resolved to too many addresses")]
    TooManyResolvedAddresses,
    #[error("the WebRTC credential partition is invalid")]
    InvalidCredentialPartition,
    #[error("the WebRTC bearer credential is invalid")]
    InvalidBearerCredential,
    #[error("the WebRTC bearer credential is too large")]
    BearerCredentialTooLarge,
    #[error("the WebRTC bearer credential provider failed")]
    CredentialProviderFailed,
    #[error("the WebRTC outbound timeout is invalid")]
    InvalidTimeout,
    #[error("the WebRTC candidate bound is invalid")]
    InvalidCandidateBound,
    #[error("the WebRTC TLS trust bundle is invalid")]
    InvalidTlsTrust,
    #[error("the WebRTC ICE server override is invalid")]
    InvalidIceServers,
    #[error("the WebRTC audio codec policy is invalid")]
    InvalidAudioCodecs,
    #[error("the WebRTC preopened DataChannel descriptor is invalid")]
    InvalidPreopenedDataChannel,
    #[error("the WebRTC preopened DataChannel descriptor is duplicated")]
    DuplicatePreopenedDataChannel,
    #[error("the WebRTC preopened DataChannel descriptor limit was exceeded")]
    TooManyPreopenedDataChannels,
    #[error("preopened WebRTC DataChannels require DataChannels to be enabled")]
    PreopenedDataChannelsDisabled,
    #[error("remote admission readiness is available only for WebSocket signaling")]
    RemoteAdmissionReadyUnsupported,
}

/// Explicit additional roots trusted by one outbound HTTPS/WSS signaling
/// context.
///
/// The trust bundle is deliberately attached to the immutable originate
/// context instead of changing process-global TLS state. This keeps private
/// PKI roots scoped to one configured signaling authority and lets pooled WSS
/// connections partition by the exact trust profile.
#[cfg(feature = "tls-rustls")]
#[derive(Clone)]
pub struct WebRtcTlsClientTrust {
    certificates: Arc<[rustls::pki_types::CertificateDer<'static>]>,
    profile_identity: [u8; 32],
}

#[cfg(feature = "tls-rustls")]
impl WebRtcTlsClientTrust {
    /// Parse a bounded PEM bundle containing one or more X.509 trust anchors.
    pub fn from_pem(pem: &[u8]) -> Result<Self, WebRtcOriginateContextError> {
        if pem.is_empty() || pem.len() > MAX_WEBRTC_TLS_TRUST_BYTES {
            return Err(WebRtcOriginateContextError::InvalidTlsTrust);
        }

        let mut reader = std::io::BufReader::new(pem);
        let certificates = rustls_pemfile::certs(&mut reader)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| WebRtcOriginateContextError::InvalidTlsTrust)?;
        if certificates.is_empty() || certificates.len() > MAX_WEBRTC_TLS_TRUST_CERTIFICATES {
            return Err(WebRtcOriginateContextError::InvalidTlsTrust);
        }

        // Validate every retained DER value as a usable trust anchor now so
        // HTTP and WebSocket clients cannot disagree later about the profile.
        let mut roots = rustls::RootCertStore::empty();
        for certificate in &certificates {
            roots
                .add(certificate.clone())
                .map_err(|_| WebRtcOriginateContextError::InvalidTlsTrust)?;
        }

        let mut digest = Sha256::new();
        for certificate in &certificates {
            digest.update((certificate.as_ref().len() as u64).to_be_bytes());
            digest.update(certificate.as_ref());
        }
        Ok(Self {
            certificates: certificates.into(),
            profile_identity: digest.finalize().into(),
        })
    }

    pub(crate) fn certificates(&self) -> &[rustls::pki_types::CertificateDer<'static>] {
        &self.certificates
    }

    #[cfg(feature = "signaling-ws")]
    pub(crate) fn profile_identity(&self) -> [u8; 32] {
        self.profile_identity
    }
}

#[cfg(feature = "tls-rustls")]
impl fmt::Debug for WebRtcTlsClientTrust {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebRtcTlsClientTrust")
            .field("certificate_count", &self.certificates.len())
            .field("profile", &"[redacted]")
            .finish()
    }
}

/// A bearer credential whose value is redacted and zeroized on drop.
#[derive(Clone, Eq, PartialEq)]
pub struct WebRtcBearerCredential(String);

impl WebRtcBearerCredential {
    pub fn new(value: impl Into<String>) -> Result<Self, WebRtcOriginateContextError> {
        let mut value = value.into();
        if value.len() > MAX_WEBRTC_BEARER_BYTES {
            value.zeroize();
            return Err(WebRtcOriginateContextError::BearerCredentialTooLarge);
        }
        // WebSocket compatibility authentication uses `token.<value>` as a
        // private subprotocol. Restrict the retained value to RFC 7230 token
        // characters so it can never split or inject a header value.
        if value.is_empty() || !value.is_ascii() || !value.bytes().all(is_http_token_byte) {
            value.zeroize();
            return Err(WebRtcOriginateContextError::InvalidBearerCredential);
        }
        Ok(Self(value))
    }

    /// Explicit access for a signaling request builder. Diagnostics stay
    /// redacted and callers should keep any derived copy as short-lived as
    /// possible.
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for WebRtcBearerCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("WebRtcBearerCredential([redacted])")
    }
}

impl fmt::Display for WebRtcBearerCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[redacted]")
    }
}

impl Drop for WebRtcBearerCredential {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// Async source for short-lived signaling credentials.
#[async_trait]
pub trait WebRtcBearerCredentialProvider: Send + Sync {
    async fn credential(
        &self,
    ) -> Result<Option<WebRtcBearerCredential>, WebRtcOriginateContextError>;
}

/// Credential provider for deployments which rotate credentials outside the
/// process. The retained copy is redacted and zeroized.
pub struct StaticWebRtcBearerCredentialProvider {
    credential: WebRtcBearerCredential,
}

impl StaticWebRtcBearerCredentialProvider {
    pub fn new(credential: WebRtcBearerCredential) -> Self {
        Self { credential }
    }
}

impl fmt::Debug for StaticWebRtcBearerCredentialProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StaticWebRtcBearerCredentialProvider")
            .field("credential", &self.credential)
            .finish()
    }
}

#[async_trait]
impl WebRtcBearerCredentialProvider for StaticWebRtcBearerCredentialProvider {
    async fn credential(
        &self,
    ) -> Result<Option<WebRtcBearerCredential>, WebRtcOriginateContextError> {
        Ok(Some(self.credential.clone()))
    }
}

/// Bounded SSRF, timeout, and backpressure policy for one target exchange.
#[derive(Clone)]
pub struct WebRtcTargetPolicy {
    allowed_ports: BTreeSet<u16>,
    allow_insecure: bool,
    allow_loopback: bool,
    allow_private_networks: bool,
    max_resolved_addresses: usize,
    max_buffered_candidates: usize,
    connect_timeout: Duration,
    signaling_timeout: Duration,
    credential_partition: String,
}

impl Default for WebRtcTargetPolicy {
    fn default() -> Self {
        Self {
            allowed_ports: BTreeSet::from([443]),
            allow_insecure: false,
            allow_loopback: false,
            allow_private_networks: false,
            max_resolved_addresses: 16,
            max_buffered_candidates: 64,
            connect_timeout: Duration::from_secs(10),
            signaling_timeout: Duration::from_secs(20),
            credential_partition: "default".to_owned(),
        }
    }
}

impl fmt::Debug for WebRtcTargetPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebRtcTargetPolicy")
            .field("allowed_port_count", &self.allowed_ports.len())
            .field("allow_insecure", &self.allow_insecure)
            .field("allow_loopback", &self.allow_loopback)
            .field("allow_private_networks", &self.allow_private_networks)
            .field("max_resolved_addresses", &self.max_resolved_addresses)
            .field("max_buffered_candidates", &self.max_buffered_candidates)
            .field("connect_timeout", &self.connect_timeout)
            .field("signaling_timeout", &self.signaling_timeout)
            .field("credential_partition", &"[redacted]")
            .finish()
    }
}

impl WebRtcTargetPolicy {
    pub fn allow_port(mut self, port: u16) -> Self {
        if port != 0 {
            self.allowed_ports.insert(port);
        }
        self
    }

    /// Permit plaintext `ws`/`http`. Intended for explicitly bounded local
    /// development networks; production should retain the secure default.
    pub fn allow_insecure(mut self, allow: bool) -> Self {
        self.allow_insecure = allow;
        self
    }

    pub fn allow_loopback(mut self, allow: bool) -> Self {
        self.allow_loopback = allow;
        self
    }

    pub fn allow_private_networks(mut self, allow: bool) -> Self {
        self.allow_private_networks = allow;
        self
    }

    pub fn with_max_resolved_addresses(
        mut self,
        maximum: usize,
    ) -> Result<Self, WebRtcOriginateContextError> {
        if maximum == 0 || maximum > 256 {
            return Err(WebRtcOriginateContextError::TooManyResolvedAddresses);
        }
        self.max_resolved_addresses = maximum;
        Ok(self)
    }

    pub fn with_max_buffered_candidates(
        mut self,
        maximum: usize,
    ) -> Result<Self, WebRtcOriginateContextError> {
        if maximum == 0 || maximum > 4_096 {
            return Err(WebRtcOriginateContextError::InvalidCandidateBound);
        }
        self.max_buffered_candidates = maximum;
        Ok(self)
    }

    pub fn with_timeouts(
        mut self,
        connect_timeout: Duration,
        signaling_timeout: Duration,
    ) -> Result<Self, WebRtcOriginateContextError> {
        const MAX_TIMEOUT: Duration = Duration::from_secs(120);
        if connect_timeout.is_zero()
            || signaling_timeout.is_zero()
            || connect_timeout > MAX_TIMEOUT
            || signaling_timeout > MAX_TIMEOUT
        {
            return Err(WebRtcOriginateContextError::InvalidTimeout);
        }
        self.connect_timeout = connect_timeout;
        self.signaling_timeout = signaling_timeout;
        Ok(self)
    }

    pub fn with_credential_partition(
        mut self,
        partition: impl Into<String>,
    ) -> Result<Self, WebRtcOriginateContextError> {
        let mut partition = partition.into();
        if partition.is_empty()
            || partition.len() > MAX_CREDENTIAL_PARTITION_BYTES
            || !partition.is_ascii()
            || !partition.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':')
            })
        {
            partition.zeroize();
            return Err(WebRtcOriginateContextError::InvalidCredentialPartition);
        }
        self.credential_partition.zeroize();
        self.credential_partition = partition;
        Ok(self)
    }

    #[cfg(any(feature = "signaling-ws", feature = "signaling-whip"))]
    pub(crate) fn connect_timeout(&self) -> Duration {
        self.connect_timeout
    }

    pub(crate) fn signaling_timeout(&self) -> Duration {
        self.signaling_timeout
    }

    #[cfg(any(feature = "signaling-ws", feature = "signaling-whip"))]
    pub(crate) fn max_resolved_addresses(&self) -> usize {
        self.max_resolved_addresses
    }

    #[cfg(any(feature = "signaling-ws", feature = "signaling-whip"))]
    pub(crate) fn max_buffered_candidates(&self) -> usize {
        self.max_buffered_candidates
    }

    #[cfg(feature = "signaling-ws")]
    pub(crate) fn credential_partition(&self) -> &str {
        &self.credential_partition
    }

    #[cfg(feature = "signaling-ws")]
    pub(crate) fn allows_loopback(&self) -> bool {
        self.allow_loopback
    }

    #[cfg(feature = "signaling-ws")]
    pub(crate) fn allows_private_networks(&self) -> bool {
        self.allow_private_networks
    }

    pub(crate) fn address_allowed(&self, address: IpAddr) -> bool {
        if is_unconditionally_forbidden(address) {
            return false;
        }
        if address.is_loopback() {
            return self.allow_loopback;
        }
        if is_private(address) {
            return self.allow_private_networks;
        }
        true
    }

    fn validate(&self) -> Result<(), WebRtcOriginateContextError> {
        if self.allowed_ports.is_empty() {
            return Err(WebRtcOriginateContextError::PortForbidden);
        }
        if self.max_resolved_addresses == 0 || self.max_resolved_addresses > 256 {
            return Err(WebRtcOriginateContextError::TooManyResolvedAddresses);
        }
        if self.max_buffered_candidates == 0 || self.max_buffered_candidates > 4_096 {
            return Err(WebRtcOriginateContextError::InvalidCandidateBound);
        }
        if self.connect_timeout.is_zero() || self.signaling_timeout.is_zero() {
            return Err(WebRtcOriginateContextError::InvalidTimeout);
        }
        validate_credential_partition(&self.credential_partition)
    }
}

impl Drop for WebRtcTargetPolicy {
    fn drop(&mut self) {
        self.credential_partition.zeroize();
    }
}

/// Immutable WebRTC-specific context carried opaquely by `OriginateRequest`.
#[derive(Clone)]
pub struct WebRtcOriginateContext {
    endpoint: Url,
    signaling_mode: WebRtcSignalingMode,
    ice_policy: WebRtcIceExchangePolicy,
    target_policy: WebRtcTargetPolicy,
    bearer_provider: Option<Arc<dyn WebRtcBearerCredentialProvider>>,
    ice_servers: Option<Arc<[IceServerConfig]>>,
    audio_codecs: Option<Arc<[WebRtcAudioCodec]>>,
    data_channels: bool,
    preopened_data_channels: Arc<[WebRtcPreopenedDataChannel]>,
    /// Require the target application to explicitly admit the exact
    /// request/connection pair before outbound lifecycle activation commits.
    ///
    /// This is negotiated with the `offer-ready` extension of
    /// `rvoip.webrtc.v1`; keeping the default false preserves the original
    /// `offer` exchange for older clients and servers.
    require_remote_admission_ready: bool,
    #[cfg(feature = "tls-rustls")]
    tls_trust: Option<Arc<WebRtcTlsClientTrust>>,
}

/// One validated channel that the offerer creates before its initial SDP.
///
/// The descriptor is intentionally private to the immutable originate
/// context. Applications use [`WebRtcOriginateContext::with_preopened_data_channel`]
/// so labels and RFC 8832 reliability are validated and deduplicated before
/// any peer or network resource exists.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct WebRtcPreopenedDataChannel {
    label: String,
    reliability: DataReliability,
}

impl fmt::Debug for WebRtcPreopenedDataChannel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebRtcPreopenedDataChannel")
            .field("label_bytes", &self.label.len())
            .field("reliability", &self.reliability)
            .finish()
    }
}

impl WebRtcPreopenedDataChannel {
    pub(crate) fn label(&self) -> &str {
        &self.label
    }

    pub(crate) fn reliability(&self) -> &DataReliability {
        &self.reliability
    }

    fn cache_key(&self) -> Result<String, WebRtcOriginateContextError> {
        crate::data_message::cache_key_parts(&self.label, &self.reliability)
            .map_err(|_| WebRtcOriginateContextError::InvalidPreopenedDataChannel)
    }
}

impl WebRtcOriginateContext {
    pub fn new(
        target: impl AsRef<str>,
        signaling_mode: WebRtcSignalingMode,
        ice_policy: WebRtcIceExchangePolicy,
        target_policy: WebRtcTargetPolicy,
        bearer_provider: Option<Arc<dyn WebRtcBearerCredentialProvider>>,
    ) -> Result<Self, WebRtcOriginateContextError> {
        target_policy.validate()?;
        let endpoint = validate_target(target.as_ref(), signaling_mode, &target_policy)?;
        Ok(Self {
            endpoint,
            signaling_mode,
            ice_policy,
            target_policy,
            bearer_provider,
            ice_servers: None,
            audio_codecs: None,
            data_channels: true,
            preopened_data_channels: Arc::from([]),
            require_remote_admission_ready: false,
            #[cfg(feature = "tls-rustls")]
            tls_trust: None,
        })
    }

    pub fn websocket(
        target: impl AsRef<str>,
        target_policy: WebRtcTargetPolicy,
    ) -> Result<Self, WebRtcOriginateContextError> {
        Self::new(
            target,
            WebRtcSignalingMode::WebSocket,
            WebRtcIceExchangePolicy::Trickle,
            target_policy,
            None,
        )
    }

    pub fn with_bearer_provider(
        mut self,
        provider: Arc<dyn WebRtcBearerCredentialProvider>,
    ) -> Self {
        self.bearer_provider = Some(provider);
        self
    }

    /// Require an exact `ready` outcome from a WebSocket target before this
    /// outbound route may publish `Connected`.
    ///
    /// The target binds the outcome to both the client request id and its
    /// assigned connection id. A rejection, BYE, closed socket, or signaling
    /// timeout before `ready` fails activation. This is deliberately opt-in
    /// so existing `rvoip.webrtc.v1` services remain source- and wire-
    /// compatible. An older server rejects `offer-ready`, so a required route
    /// fails closed rather than falling back to answer-based activation.
    pub fn require_remote_admission_ready(mut self) -> Result<Self, WebRtcOriginateContextError> {
        if self.signaling_mode != WebRtcSignalingMode::WebSocket {
            return Err(WebRtcOriginateContextError::RemoteAdmissionReadyUnsupported);
        }
        self.require_remote_admission_ready = true;
        Ok(self)
    }

    /// Whether WebSocket activation is gated on the remote application's
    /// explicit, request-bound admission outcome.
    pub fn remote_admission_ready_required(&self) -> bool {
        self.require_remote_admission_ready
    }

    /// Override STUN/TURN discovery for this exact outbound peer.
    ///
    /// Credentials remain scoped to the immutable originate context and are
    /// redacted from diagnostics. An explicit empty collection disables the
    /// adapter's process-wide ICE servers for this peer.
    pub fn with_ice_servers(
        mut self,
        ice_servers: Vec<IceServerConfig>,
    ) -> Result<Self, WebRtcOriginateContextError> {
        validate_ice_servers(&ice_servers)?;
        self.ice_servers = Some(ice_servers.into());
        Ok(self)
    }

    /// Restrict the exact outbound peer to this codec allowlist.
    ///
    /// `None` inherits the adapter's configured codecs. An explicit policy
    /// must be non-empty and is retained on the immutable originate context.
    pub fn with_audio_codecs(
        mut self,
        codecs: impl IntoIterator<Item = WebRtcAudioCodec>,
    ) -> Result<Self, WebRtcOriginateContextError> {
        let codecs = codecs.into_iter().collect::<BTreeSet<_>>();
        if codecs.is_empty() || codecs.len() > MAX_WEBRTC_ORIGINATE_AUDIO_CODECS {
            return Err(WebRtcOriginateContextError::InvalidAudioCodecs);
        }
        self.audio_codecs = Some(codecs.into_iter().collect::<Vec<_>>().into());
        Ok(self)
    }

    /// Enable or disable SCTP/DataChannel negotiation for this exact peer.
    ///
    /// A disabled policy is enforced at offer construction and at both data
    /// send/receive boundaries; it is not merely a capability hint.
    pub fn with_data_channels(mut self, enabled: bool) -> Self {
        self.data_channels = enabled;
        self
    }

    /// Create an arbitrary labeled DataChannel before the initial offer.
    ///
    /// Preopening is required when an application must send on a non-legacy
    /// label while target activation is still waiting for request-bound
    /// remote admission. Exact `(label, reliability)` duplicates—including
    /// the retained legacy bootstrap channel—are rejected. The bounded
    /// descriptor is frozen into this context and contains no message bytes.
    pub fn with_preopened_data_channel(
        mut self,
        label: impl Into<String>,
        reliability: DataReliability,
    ) -> Result<Self, WebRtcOriginateContextError> {
        if !self.data_channels {
            return Err(WebRtcOriginateContextError::PreopenedDataChannelsDisabled);
        }
        let descriptor = WebRtcPreopenedDataChannel {
            label: label.into(),
            reliability,
        };
        let key = descriptor.cache_key()?;
        let legacy_key = crate::data_message::cache_key_parts(
            crate::adapter::OUTBOUND_MESSAGE_CHANNEL_LABEL,
            &DataReliability::ReliableOrdered,
        )
        .map_err(|_| WebRtcOriginateContextError::InvalidPreopenedDataChannel)?;
        if key == legacy_key
            || self
                .preopened_data_channels
                .iter()
                .any(|existing| existing.cache_key().is_ok_and(|existing| existing == key))
        {
            return Err(WebRtcOriginateContextError::DuplicatePreopenedDataChannel);
        }
        if self.preopened_data_channels.len() >= MAX_WEBRTC_ORIGINATE_PREOPENED_DATA_CHANNELS {
            return Err(WebRtcOriginateContextError::TooManyPreopenedDataChannels);
        }
        let mut descriptors = self.preopened_data_channels.to_vec();
        descriptors.push(descriptor);
        self.preopened_data_channels = descriptors.into();
        Ok(self)
    }

    /// Add a bounded collection of `(label, reliability)` descriptors.
    ///
    /// Validation is atomic from the caller's perspective because this
    /// builder consumes the context: an invalid, duplicate, or over-limit
    /// collection returns no partially configured context.
    pub fn with_preopened_data_channels<I, L>(
        mut self,
        descriptors: I,
    ) -> Result<Self, WebRtcOriginateContextError>
    where
        I: IntoIterator<Item = (L, DataReliability)>,
        L: Into<String>,
    {
        for (label, reliability) in descriptors {
            self = self.with_preopened_data_channel(label, reliability)?;
        }
        Ok(self)
    }

    /// Add private trust anchors for this exact HTTPS/WSS signaling context.
    /// System roots remain available; certificate and hostname validation are
    /// never disabled.
    #[cfg(feature = "tls-rustls")]
    pub fn with_tls_trust(mut self, trust: Arc<WebRtcTlsClientTrust>) -> Self {
        self.tls_trust = Some(trust);
        self
    }

    pub fn signaling_mode(&self) -> WebRtcSignalingMode {
        self.signaling_mode
    }

    pub fn ice_policy(&self) -> WebRtcIceExchangePolicy {
        self.ice_policy
    }

    /// Canonical, credential-free signaling target. Treat path components as
    /// operationally sensitive even though credentials and query strings are
    /// forbidden.
    pub fn endpoint(&self) -> &Url {
        &self.endpoint
    }

    pub fn target_policy(&self) -> &WebRtcTargetPolicy {
        &self.target_policy
    }

    /// Replace the bounded target policy and revalidate the frozen endpoint.
    ///
    /// This is intentionally explicit: callers cannot relax the default
    /// public-network boundary without constructing a policy that names the
    /// exception, and every replacement is checked against the already
    /// retained signaling mode and endpoint before it becomes observable.
    /// Bearer, TLS-trust, and per-peer ICE material remain unchanged.
    pub fn with_target_policy(
        mut self,
        target_policy: WebRtcTargetPolicy,
    ) -> Result<Self, WebRtcOriginateContextError> {
        target_policy.validate()?;
        let endpoint =
            validate_target(self.endpoint.as_str(), self.signaling_mode, &target_policy)?;
        if endpoint != self.endpoint {
            return Err(WebRtcOriginateContextError::InvalidTarget);
        }
        self.target_policy = target_policy;
        Ok(self)
    }

    /// Per-peer STUN/TURN override. `None` inherits the adapter configuration;
    /// `Some([])` deliberately disables external ICE servers.
    pub fn ice_servers_override(&self) -> Option<&[IceServerConfig]> {
        self.ice_servers.as_deref()
    }

    /// Exact per-peer codec allowlist, or `None` to inherit adapter policy.
    pub fn audio_codecs_override(&self) -> Option<&[WebRtcAudioCodec]> {
        self.audio_codecs.as_deref()
    }

    /// Whether this peer may negotiate and exchange DataChannel messages.
    pub fn data_channels_allowed(&self) -> bool {
        self.data_channels
    }

    pub(crate) fn preopened_data_channels(&self) -> &[WebRtcPreopenedDataChannel] {
        &self.preopened_data_channels
    }

    #[cfg(any(feature = "signaling-ws", feature = "signaling-whip"))]
    pub(crate) async fn bearer_credential(
        &self,
    ) -> Result<Option<WebRtcBearerCredential>, WebRtcOriginateContextError> {
        match self.bearer_provider.as_ref() {
            Some(provider) => provider.credential().await,
            None => Ok(None),
        }
    }

    #[cfg(feature = "signaling-ws")]
    pub(crate) fn bearer_provider_identity(&self) -> usize {
        self.bearer_provider
            .as_ref()
            .map(|provider| Arc::as_ptr(provider).cast::<()>() as usize)
            .unwrap_or_default()
    }

    #[cfg(feature = "tls-rustls")]
    pub(crate) fn tls_trust(&self) -> Option<&WebRtcTlsClientTrust> {
        self.tls_trust.as_deref()
    }

    #[cfg(all(feature = "tls-rustls", feature = "signaling-ws"))]
    pub(crate) fn tls_trust_profile_identity(&self) -> Option<[u8; 32]> {
        self.tls_trust
            .as_deref()
            .map(WebRtcTlsClientTrust::profile_identity)
    }

    /// Revalidate retained values at the opaque adapter boundary.
    pub fn validate(&self) -> Result<(), WebRtcOriginateContextError> {
        self.target_policy.validate()?;
        let validated = validate_target(
            self.endpoint.as_str(),
            self.signaling_mode,
            &self.target_policy,
        )?;
        if validated != self.endpoint {
            return Err(WebRtcOriginateContextError::InvalidTarget);
        }
        #[cfg(feature = "tls-rustls")]
        if self.tls_trust.is_some() && !matches!(self.endpoint.scheme(), "https" | "wss") {
            return Err(WebRtcOriginateContextError::InvalidTlsTrust);
        }
        if let Some(ice_servers) = self.ice_servers.as_deref() {
            validate_ice_servers(ice_servers)?;
        }
        if self.audio_codecs.as_ref().is_some_and(|codecs| {
            codecs.is_empty() || codecs.len() > MAX_WEBRTC_ORIGINATE_AUDIO_CODECS
        }) {
            return Err(WebRtcOriginateContextError::InvalidAudioCodecs);
        }
        if self.require_remote_admission_ready
            && self.signaling_mode != WebRtcSignalingMode::WebSocket
        {
            return Err(WebRtcOriginateContextError::RemoteAdmissionReadyUnsupported);
        }
        if !self.preopened_data_channels.is_empty() && !self.data_channels {
            return Err(WebRtcOriginateContextError::PreopenedDataChannelsDisabled);
        }
        if self.preopened_data_channels.len() > MAX_WEBRTC_ORIGINATE_PREOPENED_DATA_CHANNELS {
            return Err(WebRtcOriginateContextError::TooManyPreopenedDataChannels);
        }
        let legacy_key = crate::data_message::cache_key_parts(
            crate::adapter::OUTBOUND_MESSAGE_CHANNEL_LABEL,
            &DataReliability::ReliableOrdered,
        )
        .map_err(|_| WebRtcOriginateContextError::InvalidPreopenedDataChannel)?;
        let mut channel_keys = BTreeSet::from([legacy_key]);
        for descriptor in self.preopened_data_channels.iter() {
            let key = descriptor.cache_key()?;
            if !channel_keys.insert(key) {
                return Err(WebRtcOriginateContextError::DuplicatePreopenedDataChannel);
            }
        }
        Ok(())
    }

    pub(crate) fn request_target_matches(&self, request_target: &str) -> bool {
        request_target.is_empty()
            || Url::parse(request_target)
                .map(|target| target == self.endpoint)
                .unwrap_or(false)
    }
}

impl fmt::Debug for WebRtcOriginateContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebRtcOriginateContext")
            .field("endpoint", &"[redacted]")
            .field("signaling_mode", &self.signaling_mode)
            .field("ice_policy", &self.ice_policy)
            .field("target_policy", &self.target_policy)
            .field("bearer_provider_present", &self.bearer_provider.is_some())
            .field(
                "remote_admission_ready_required",
                &self.require_remote_admission_ready,
            )
            .field(
                "ice_server_override_count",
                &self.ice_servers.as_ref().map_or(0, |servers| servers.len()),
            )
            .field(
                "audio_codec_override_count",
                &self.audio_codecs.as_ref().map_or(0, |codecs| codecs.len()),
            )
            .field("data_channels", &self.data_channels)
            .field(
                "preopened_data_channel_count",
                &self.preopened_data_channels.len(),
            )
            .field("tls_trust_present", &{
                #[cfg(feature = "tls-rustls")]
                {
                    self.tls_trust.is_some()
                }
                #[cfg(not(feature = "tls-rustls"))]
                {
                    false
                }
            })
            .finish()
    }
}

fn validate_ice_servers(
    ice_servers: &[IceServerConfig],
) -> Result<(), WebRtcOriginateContextError> {
    if ice_servers.len() > MAX_WEBRTC_ORIGINATE_ICE_SERVERS {
        return Err(WebRtcOriginateContextError::InvalidIceServers);
    }
    for server in ice_servers {
        if server.urls.is_empty() || server.urls.len() > MAX_WEBRTC_ORIGINATE_ICE_URLS {
            return Err(WebRtcOriginateContextError::InvalidIceServers);
        }
        if server.username.is_some() != server.credential.is_some() {
            return Err(WebRtcOriginateContextError::InvalidIceServers);
        }
        for value in [server.username.as_deref(), server.credential.as_deref()]
            .into_iter()
            .flatten()
        {
            if value.is_empty()
                || value.len() > MAX_WEBRTC_ORIGINATE_ICE_CREDENTIAL_BYTES
                || value.chars().any(char::is_control)
            {
                return Err(WebRtcOriginateContextError::InvalidIceServers);
            }
        }
        for url in &server.urls {
            let scheme = url
                .split_once(':')
                .map(|(scheme, _)| scheme.to_ascii_lowercase())
                .ok_or(WebRtcOriginateContextError::InvalidIceServers)?;
            if url.is_empty()
                || url.len() > MAX_WEBRTC_ORIGINATE_ICE_URL_BYTES
                || url.chars().any(char::is_whitespace)
                || url.chars().any(char::is_control)
                || !matches!(scheme.as_str(), "stun" | "stuns" | "turn" | "turns")
            {
                return Err(WebRtcOriginateContextError::InvalidIceServers);
            }
        }
    }
    Ok(())
}

fn validate_target(
    raw: &str,
    mode: WebRtcSignalingMode,
    policy: &WebRtcTargetPolicy,
) -> Result<Url, WebRtcOriginateContextError> {
    if raw.len() > MAX_WEBRTC_TARGET_BYTES {
        return Err(WebRtcOriginateContextError::TargetTooLarge);
    }
    let endpoint = Url::parse(raw).map_err(|_| WebRtcOriginateContextError::InvalidTarget)?;
    if !endpoint.username().is_empty() || endpoint.password().is_some() {
        return Err(WebRtcOriginateContextError::CredentialsInTarget);
    }
    if endpoint.query().is_some() || endpoint.fragment().is_some() {
        return Err(WebRtcOriginateContextError::TargetMetadataForbidden);
    }
    if endpoint.host_str().is_none() {
        return Err(WebRtcOriginateContextError::InvalidTarget);
    }
    let scheme_allowed = match mode {
        WebRtcSignalingMode::WebSocket => matches!(endpoint.scheme(), "ws" | "wss"),
        WebRtcSignalingMode::Whip | WebRtcSignalingMode::Whep => {
            matches!(endpoint.scheme(), "http" | "https")
        }
    };
    if !scheme_allowed {
        return Err(WebRtcOriginateContextError::SignalingModeMismatch);
    }
    if matches!(endpoint.scheme(), "ws" | "http") && !policy.allow_insecure {
        return Err(WebRtcOriginateContextError::InsecureTransportForbidden);
    }
    let port = endpoint
        .port_or_known_default()
        .ok_or(WebRtcOriginateContextError::InvalidTarget)?;
    if !policy.allowed_ports.contains(&port) {
        return Err(WebRtcOriginateContextError::PortForbidden);
    }
    if let Ok(address) = endpoint
        .host_str()
        .ok_or(WebRtcOriginateContextError::InvalidTarget)?
        .parse::<IpAddr>()
    {
        if !policy.address_allowed(address) {
            return Err(WebRtcOriginateContextError::AddressForbidden);
        }
    }
    Ok(endpoint)
}

fn validate_credential_partition(value: &str) -> Result<(), WebRtcOriginateContextError> {
    if value.is_empty()
        || value.len() > MAX_CREDENTIAL_PARTITION_BYTES
        || !value.is_ascii()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
    {
        return Err(WebRtcOriginateContextError::InvalidCredentialPartition);
    }
    Ok(())
}

fn is_http_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

fn is_unconditionally_forbidden(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            address.is_unspecified()
                || address.is_broadcast()
                || address.is_multicast()
                || address.is_documentation()
                || address.is_link_local()
        }
        IpAddr::V6(address) => {
            address.is_unspecified()
                || address.is_multicast()
                || is_ipv6_documentation(address)
                || is_ipv6_link_local(address)
        }
    }
}

fn is_private(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => address.is_private(),
        IpAddr::V6(address) => is_ipv6_unique_local(address),
    }
}

fn is_ipv6_unique_local(address: Ipv6Addr) -> bool {
    address.segments()[0] & 0xfe00 == 0xfc00
}

fn is_ipv6_link_local(address: Ipv6Addr) -> bool {
    address.segments()[0] & 0xffc0 == 0xfe80
}

fn is_ipv6_documentation(address: Ipv6Addr) -> bool {
    let segments = address.segments();
    segments[0] == 0x2001 && segments[1] == 0x0db8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loopback_policy(port: u16) -> WebRtcTargetPolicy {
        WebRtcTargetPolicy::default()
            .allow_port(port)
            .allow_insecure(true)
            .allow_loopback(true)
    }

    #[test]
    fn rejects_target_credentials_query_fragment_and_mode_mismatch() {
        let policy = loopback_policy(8080);
        assert_eq!(
            WebRtcOriginateContext::websocket("ws://user:secret@127.0.0.1:8080/s", policy.clone())
                .unwrap_err(),
            WebRtcOriginateContextError::CredentialsInTarget
        );
        assert_eq!(
            WebRtcOriginateContext::websocket("ws://127.0.0.1:8080/s?token=secret", policy.clone())
                .unwrap_err(),
            WebRtcOriginateContextError::TargetMetadataForbidden
        );
        assert_eq!(
            WebRtcOriginateContext::new(
                "https://127.0.0.1:8080/s",
                WebRtcSignalingMode::WebSocket,
                WebRtcIceExchangePolicy::Trickle,
                policy,
                None,
            )
            .unwrap_err(),
            WebRtcOriginateContextError::SignalingModeMismatch
        );
    }

    #[test]
    fn secure_public_target_is_the_default_boundary() {
        let context = WebRtcOriginateContext::websocket(
            "wss://signal.example.test/call",
            WebRtcTargetPolicy::default(),
        )
        .expect("secure target");
        assert_eq!(context.signaling_mode(), WebRtcSignalingMode::WebSocket);
        assert_eq!(context.ice_policy(), WebRtcIceExchangePolicy::Trickle);

        assert_eq!(
            WebRtcOriginateContext::websocket(
                "ws://signal.example.test/call",
                WebRtcTargetPolicy::default(),
            )
            .unwrap_err(),
            WebRtcOriginateContextError::InsecureTransportForbidden
        );
        assert_eq!(
            WebRtcOriginateContext::websocket(
                "wss://127.0.0.1/call",
                WebRtcTargetPolicy::default(),
            )
            .unwrap_err(),
            WebRtcOriginateContextError::AddressForbidden
        );
    }

    #[test]
    fn remote_admission_readiness_is_explicit_and_websocket_only() {
        let legacy =
            WebRtcOriginateContext::websocket("ws://127.0.0.1:8080/signal", loopback_policy(8080))
                .expect("legacy-compatible context");
        assert!(!legacy.remote_admission_ready_required());
        assert!(legacy
            .require_remote_admission_ready()
            .expect("WebSocket readiness policy")
            .remote_admission_ready_required());

        let whip = WebRtcOriginateContext::new(
            "http://127.0.0.1:8080/whip",
            WebRtcSignalingMode::Whip,
            WebRtcIceExchangePolicy::Trickle,
            loopback_policy(8080),
            None,
        )
        .expect("WHIP context");
        assert_eq!(
            whip.require_remote_admission_ready().unwrap_err(),
            WebRtcOriginateContextError::RemoteAdmissionReadyUnsupported
        );
    }

    #[test]
    fn target_policy_override_revalidates_without_changing_the_default_boundary() {
        assert_eq!(
            WebRtcOriginateContext::websocket(
                "wss://127.0.0.1:8443/call",
                WebRtcTargetPolicy::default().allow_port(8443),
            )
            .unwrap_err(),
            WebRtcOriginateContextError::AddressForbidden,
            "the production default must continue to reject loopback targets"
        );

        let context = WebRtcOriginateContext::websocket(
            "wss://127.0.0.1:8443/call",
            WebRtcTargetPolicy::default()
                .allow_port(8443)
                .allow_loopback(true),
        )
        .expect("explicit loopback fixture policy");
        assert_eq!(
            context
                .clone()
                .with_target_policy(WebRtcTargetPolicy::default().allow_port(8443))
                .unwrap_err(),
            WebRtcOriginateContextError::AddressForbidden,
            "replacement policies must be checked against the frozen endpoint"
        );
        context
            .with_target_policy(
                WebRtcTargetPolicy::default()
                    .allow_port(8443)
                    .allow_loopback(true),
            )
            .expect("an explicit, still-valid fixture policy can be retained");
    }

    #[test]
    fn diagnostics_redact_target_policy_and_bearer() {
        let credential = WebRtcBearerCredential::new("canary-secret").expect("credential");
        let provider = Arc::new(StaticWebRtcBearerCredentialProvider::new(credential));
        let context = WebRtcOriginateContext::websocket(
            "ws://127.0.0.1:8080/private-call-path",
            loopback_policy(8080)
                .with_credential_partition("tenant-canary")
                .expect("partition"),
        )
        .expect("context")
        .with_bearer_provider(provider);
        let diagnostic = format!("{context:?}");
        assert!(!diagnostic.contains("private-call-path"));
        assert!(!diagnostic.contains("canary-secret"));
        assert!(!diagnostic.contains("tenant-canary"));
        assert!(diagnostic.contains("[redacted]"));
    }

    #[test]
    fn bearer_rejects_header_splitting_bytes() {
        assert_eq!(
            WebRtcBearerCredential::new("secret,other").unwrap_err(),
            WebRtcOriginateContextError::InvalidBearerCredential
        );
        assert_eq!(
            WebRtcBearerCredential::new("secret\r\nheader").unwrap_err(),
            WebRtcOriginateContextError::InvalidBearerCredential
        );
    }

    #[test]
    fn per_peer_ice_override_is_bounded_and_redacted() {
        let context =
            WebRtcOriginateContext::websocket("ws://127.0.0.1:8080/signal", loopback_policy(8080))
                .expect("context")
                .with_ice_servers(vec![IceServerConfig::turn(
                    "turns:turn.example.test:5349?transport=tcp",
                    "turn-user-canary",
                    "turn-secret-canary",
                )])
                .expect("valid ICE override");

        let retained = context.ice_servers_override().expect("override");
        assert_eq!(retained.len(), 1);
        assert_eq!(retained[0].urls.len(), 1);
        let diagnostic = format!("{context:?}");
        assert!(diagnostic.contains("ice_server_override_count: 1"));
        assert!(!diagnostic.contains("turn-user-canary"));
        assert!(!diagnostic.contains("turn-secret-canary"));
    }

    #[test]
    fn per_peer_ice_override_rejects_partial_credentials_and_bad_schemes() {
        let base = || {
            WebRtcOriginateContext::websocket("ws://127.0.0.1:8080/signal", loopback_policy(8080))
                .expect("context")
        };
        assert_eq!(
            base()
                .with_ice_servers(vec![IceServerConfig {
                    urls: vec!["turn:turn.example.test".into()],
                    username: Some("user".into()),
                    credential: None,
                }])
                .unwrap_err(),
            WebRtcOriginateContextError::InvalidIceServers
        );
        assert_eq!(
            base()
                .with_ice_servers(vec![IceServerConfig::stun(
                    "https://not-an-ice-server.example.test"
                )])
                .unwrap_err(),
            WebRtcOriginateContextError::InvalidIceServers
        );
    }

    #[test]
    fn preopened_data_channels_are_bounded_validated_and_redacted() {
        let context =
            WebRtcOriginateContext::websocket("ws://127.0.0.1:8080/signal", loopback_policy(8080))
                .expect("context")
                .with_preopened_data_channel(
                    "bridgefu.context.v1",
                    DataReliability::ReliableOrdered,
                )
                .expect("bounded preopened context channel");
        assert_eq!(context.preopened_data_channels().len(), 1);
        assert_eq!(
            context.preopened_data_channels()[0].label(),
            "bridgefu.context.v1"
        );
        assert_eq!(
            context.preopened_data_channels()[0].reliability(),
            &DataReliability::ReliableOrdered
        );
        let diagnostic = format!("{context:?}");
        assert!(diagnostic.contains("preopened_data_channel_count: 1"));
        assert!(!diagnostic.contains("bridgefu.context.v1"));
    }

    #[test]
    fn preopened_data_channels_reject_unsafe_duplicate_and_disabled_descriptors() {
        let base = || {
            WebRtcOriginateContext::websocket("ws://127.0.0.1:8080/signal", loopback_policy(8080))
                .expect("context")
        };
        assert_eq!(
            base()
                .with_preopened_data_channel(
                    "bridgefu.context.v1\r\nunsafe",
                    DataReliability::ReliableOrdered,
                )
                .unwrap_err(),
            WebRtcOriginateContextError::InvalidPreopenedDataChannel
        );
        assert_eq!(
            base()
                .with_preopened_data_channel(
                    "bridgefu.context.v1",
                    DataReliability::MaxLifetime {
                        ordered: true,
                        milliseconds: 0,
                    },
                )
                .unwrap_err(),
            WebRtcOriginateContextError::InvalidPreopenedDataChannel
        );

        let context = base()
            .with_preopened_data_channel("bridgefu.context.v1", DataReliability::ReliableOrdered)
            .expect("first descriptor");
        assert_eq!(
            context
                .with_preopened_data_channel(
                    "bridgefu.context.v1",
                    DataReliability::ReliableOrdered,
                )
                .unwrap_err(),
            WebRtcOriginateContextError::DuplicatePreopenedDataChannel
        );
        assert_eq!(
            base()
                .with_preopened_data_channel(
                    crate::adapter::OUTBOUND_MESSAGE_CHANNEL_LABEL,
                    DataReliability::ReliableOrdered,
                )
                .unwrap_err(),
            WebRtcOriginateContextError::DuplicatePreopenedDataChannel
        );
        assert_eq!(
            base()
                .with_data_channels(false)
                .with_preopened_data_channel(
                    "bridgefu.context.v1",
                    DataReliability::ReliableOrdered,
                )
                .unwrap_err(),
            WebRtcOriginateContextError::PreopenedDataChannelsDisabled
        );
        assert_eq!(
            base()
                .with_preopened_data_channel(
                    "bridgefu.context.v1",
                    DataReliability::ReliableOrdered,
                )
                .expect("preopened channel")
                .with_data_channels(false)
                .validate()
                .unwrap_err(),
            WebRtcOriginateContextError::PreopenedDataChannelsDisabled
        );
    }

    #[test]
    fn preopened_data_channel_limit_includes_the_legacy_bootstrap_slot() {
        let mut context =
            WebRtcOriginateContext::websocket("ws://127.0.0.1:8080/signal", loopback_policy(8080))
                .expect("context");
        for index in 0..MAX_WEBRTC_ORIGINATE_PREOPENED_DATA_CHANNELS {
            context = context
                .with_preopened_data_channel(
                    format!("application-channel-{index}"),
                    DataReliability::ReliableOrdered,
                )
                .expect("descriptor within bound");
        }
        assert_eq!(
            context
                .with_preopened_data_channel(
                    "one-channel-too-many",
                    DataReliability::ReliableOrdered,
                )
                .unwrap_err(),
            WebRtcOriginateContextError::TooManyPreopenedDataChannels
        );
    }

    #[cfg(feature = "tls-rustls")]
    #[test]
    fn tls_trust_is_bounded_validated_and_redacted() {
        let generated = rcgen::generate_simple_self_signed(vec!["localhost".into()])
            .expect("generate test trust anchor");
        let pem = generated.cert.pem();
        let trust = WebRtcTlsClientTrust::from_pem(pem.as_bytes()).expect("valid trust bundle");
        let diagnostic = format!("{trust:?}");
        assert!(diagnostic.contains("certificate_count: 1"));
        assert!(diagnostic.contains("[redacted]"));
        assert!(!diagnostic.contains("BEGIN CERTIFICATE"));

        assert_eq!(
            WebRtcTlsClientTrust::from_pem(b"").unwrap_err(),
            WebRtcOriginateContextError::InvalidTlsTrust
        );
        assert_eq!(
            WebRtcTlsClientTrust::from_pem(b"not a certificate").unwrap_err(),
            WebRtcOriginateContextError::InvalidTlsTrust
        );
    }

    #[cfg(feature = "tls-rustls")]
    #[test]
    fn custom_tls_trust_cannot_be_attached_to_plaintext_signaling() {
        let generated = rcgen::generate_simple_self_signed(vec!["localhost".into()])
            .expect("generate test trust anchor");
        let trust = Arc::new(
            WebRtcTlsClientTrust::from_pem(generated.cert.pem().as_bytes())
                .expect("valid trust bundle"),
        );
        let context =
            WebRtcOriginateContext::websocket("ws://127.0.0.1:8080/signal", loopback_policy(8080))
                .expect("plaintext test context")
                .with_tls_trust(trust);
        assert_eq!(
            context.validate().unwrap_err(),
            WebRtcOriginateContextError::InvalidTlsTrust
        );
    }
}
