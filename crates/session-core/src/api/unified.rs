//! Lower-level session orchestration API.
//!
//! [`UnifiedCoordinator`] is the shared engine underneath [`StreamPeer`] and
//! [`CallbackPeer`]. It exposes explicit [`SessionId`] values and direct
//! methods for call creation, incoming-call resolution, registration, event
//! subscription, transfer primitives, audio bridging, and media control.
//!
//! Use this module directly when you are building an application framework on
//! top of `session-core`: B2BUA logic, gateways, carrier-facing services,
//! custom peer abstractions, or multi-leg call orchestration. For ordinary
//! client/test code, [`StreamPeer`] is usually more ergonomic. For reactive
//! server endpoints, [`CallbackPeer`] is usually the better starting point.
//!
//! # Example
//!
//! ```rust,no_run
//! use rvoip_session_core::{Config, Event, Result, UnifiedCoordinator};
//!
//! # async fn example() -> Result<()> {
//! let coordinator = UnifiedCoordinator::new(Config::local("app", 5060)).await?;
//! let mut events = coordinator.events().await?;
//!
//! let call_id = coordinator
//!     .make_call("sip:app@127.0.0.1:5060", "sip:bob@127.0.0.1:5070")
//!     .await?;
//!
//! while let Some(event) = events.next().await {
//!     match event {
//!         Event::CallAnswered { call_id: id, .. } if id == call_id => {
//!             coordinator.send_dtmf(&call_id, '1').await?;
//!             coordinator.hangup(&call_id).await?;
//!         }
//!         Event::CallEnded { call_id: id, .. } if id == call_id => break,
//!         Event::CallFailed { call_id: id, .. } if id == call_id => break,
//!         _ => {}
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! [`CallbackPeer`]: crate::api::callback_peer::CallbackPeer
//! [`StreamPeer`]: crate::api::stream_peer::StreamPeer

use crate::adapters::{DialogAdapter, MediaAdapter};
use crate::errors::{Result, SessionError};
use crate::session_registry::SessionRegistry;
use crate::session_store::SessionStore;
use crate::state_machine::{StateMachine, StateMachineHelpers};
use crate::state_table::types::{Action, EventType, SessionId};
use crate::types::CallState;
use crate::types::{IncomingCallInfo, SessionInfo};
// Callback system removed - using event-driven approach
use rvoip_infra_common::events::coordinator::GlobalEventCoordinator;
use rvoip_media_core::types::AudioFrame;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

pub use rvoip_dialog_core::api::RelUsage;
pub use rvoip_media_core::relay::controller::{AudioSource, BridgeError, BridgeHandle};

/// SIP TLS operating mode for signalling transports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SipTlsMode {
    /// Disable SIP TLS transport.
    Disabled,
    /// Dial outbound TLS connections only. This is the normal mode for
    /// registering to an upstream proxy/B2BUA such as Asterisk; no local
    /// certificate/key is required.
    ClientOnly,
    /// Bind a SIP TLS listener only. Requires a local certificate/key.
    ServerOnly,
    /// Bind a listener and support outbound TLS dials. Requires a local
    /// certificate/key for the listener side.
    ClientAndServer,
}

impl Default for SipTlsMode {
    fn default() -> Self {
        Self::Disabled
    }
}

/// How this UA expects SIP peers to reach the Contact it advertises.
///
/// This is intentionally separate from [`SipTlsMode`]. The TLS mode controls
/// sockets; the contact mode controls the SIP registration/routing contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SipContactMode {
    /// Advertise a Contact address that peers can dial directly. For SIP TLS
    /// this normally means a local TLS listener and listener certificate/key.
    ReachableContact,
    /// RFC 5626 SIP Outbound: advertise outbound Contact parameters and
    /// receive inbound requests over the registered connection-oriented flow.
    RegisteredFlowRfc5626,
    /// Asterisk/PBX symmetric transport style: keep the registration flow
    /// alive and accept inbound requests on that flow without requiring the
    /// registrar to echo RFC 5626 Contact parameters.
    RegisteredFlowSymmetric,
}

impl Default for SipContactMode {
    fn default() -> Self {
        Self::ReachableContact
    }
}

/// Runtime configuration for [`UnifiedCoordinator`].
///
/// `Config` controls SIP and media binding, advertised addresses, TLS,
/// registration Contact behavior, SRTP policy, session timers, reliable
/// provisionals, caller identity headers, outbound INVITE routing, NAT/media
/// address discovery, and codec negotiation.
///
/// Start with [`Config::local`] for loopback examples, [`Config::on`] for a
/// specific LAN/host address, then adjust the feature-specific fields for the
/// deployment profile.
///
/// # Examples
///
/// ```rust
/// use rvoip_session_core::{Config, SipContactMode, SipTlsMode};
///
/// let lan = Config::on("alice", "192.168.1.50".parse().unwrap(), 5060);
/// assert_eq!(lan.local_uri, "sip:alice@192.168.1.50:5060");
///
/// let tls_registered = Config::local("alice", 5060)
///     .tls_registered_flow_symmetric("urn:uuid:00000000-0000-0000-0000-000000000001");
/// assert_eq!(tls_registered.sip_tls_mode, SipTlsMode::ClientOnly);
/// assert_eq!(
///     tls_registered.sip_contact_mode,
///     SipContactMode::RegisteredFlowSymmetric
/// );
/// ```
#[derive(Debug, Clone)]
pub struct Config {
    /// Local IP address for media
    pub local_ip: IpAddr,
    /// SIP port
    pub sip_port: u16,
    /// Starting port for media
    pub media_port_start: u16,
    /// Ending port for media
    pub media_port_end: u16,
    /// Bind address for SIP
    pub bind_addr: SocketAddr,
    /// Optional advertised address for SIP Via sent-by and fallback Contact
    /// generation. This is distinct from [`Config::bind_addr`]: bind can be
    /// `0.0.0.0`, while the advertised address must be routable by peers.
    pub sip_advertised_addr: Option<SocketAddr>,
    /// Optional path to custom state table YAML
    /// Priority: 1) This config path, 2) RVOIP_STATE_TABLE env var, 3) Embedded default
    pub state_table_path: Option<String>,
    /// Local SIP URI (e.g., "sip:alice@127.0.0.1:5060")
    pub local_uri: String,
    /// Policy for RFC 3262 `100rel` reliable provisionals on outgoing INVITE.
    ///
    /// Default is `Supported` — advertise capability without demanding it,
    /// which is the safe setting for interop and unchanged wire behavior.
    /// Set to `Required` when connecting to a carrier that mandates 100rel,
    /// or `NotSupported` to omit the tag entirely.
    pub use_100rel: RelUsage,

    /// RFC 4028 `Session-Expires` value in seconds to advertise on outgoing
    /// INVITEs. `None` disables session timers entirely. Common carrier
    /// value is 1800 (30 min).
    pub session_timer_secs: Option<u32>,

    /// Minimum-session-expires (`Min-SE:`) we're willing to accept, in
    /// seconds. Default 90 per RFC 4028 §5.
    pub session_timer_min_se: u32,

    /// Default credentials to apply to every outgoing call for RFC 3261 §22.2
    /// INVITE digest auth retry. When the server responds 401/407 to our
    /// INVITE, session-core looks here (or at per-call credentials passed
    /// via `PeerControl::call_with_auth`) to compute the digest response. When
    /// `None`, a 401/407 on INVITE surfaces as `CallFailed` instead of
    /// retrying. Default: `None`.
    pub credentials: Option<crate::types::Credentials>,

    /// Default `P-Asserted-Identity` URI (RFC 3325 §9.1) to attach to every
    /// outgoing INVITE. Carrier trunks (Twilio, Vonage, Bandwidth, most PBX
    /// trunks) require PAI for caller-ID assertion on outbound trunk calls;
    /// without it the call is often hard-rejected or stripped of caller ID.
    /// `None` (the default) suppresses the header entirely. Per-call override
    /// is available via [`UnifiedCoordinator::make_call_with_pai`].
    pub pai_uri: Option<String>,

    /// Outbound proxy URI (RFC 3261 §8.1.2). When set, a `Route:
    /// <outbound-proxy-uri;lr>` header is pre-loaded as the first Route on
    /// every outgoing INVITE this UA originates, forcing the dialog-
    /// initiating request through the specified proxy. Typical values:
    /// `sip:sbc.example.com;lr`, `sips:sbc.example.com:5061;lr`.
    ///
    /// The URI should carry the `;lr` parameter to signal a loose-routing
    /// proxy (RFC 3261 §16.12.1.1). Session-core does **not** auto-add `;lr`
    /// — set it explicitly in the URI string.
    ///
    /// **Current scope**: applied to outgoing INVITEs via the `extra_headers`
    /// path. Outbound REGISTER is not yet routed through this proxy; the
    /// RFC 5626 SIP Outbound work (A5) will integrate REGISTER + keep-alive
    /// flow-tokens separately. `None` (the default) suppresses the header
    /// entirely. Per-INVITE override is not yet exposed.
    pub outbound_proxy_uri: Option<String>,

    /// Enable RFC 5626 "SIP Outbound" behaviour on outgoing REGISTERs.
    ///
    /// When `true` and [`Config::sip_instance`] is set, the REGISTER Contact
    /// carries the outbound-aware parameters:
    ///
    /// - `+sip.instance="<urn:...>"` (RFC 5626 §4.1) — UA-stable instance
    ///   URN, so the registrar can associate a binding with a specific
    ///   physical device across flow failures.
    /// - `reg-id=1` (RFC 5626 §4.2) — flow identifier. Multi-flow support
    ///   will bump this; today we always register flow 1.
    /// - The Contact URI gets a `;ob` flag (RFC 5626 §5.4) signalling that
    ///   the UA wants the registrar to preserve the flow association.
    ///
    /// Enable this for carriers and SBCs that assume RFC 5626 — most
    /// modern carrier infra does. Default: `false` (pre-5626 REGISTER
    /// behaviour for backwards compatibility).
    ///
    /// When the registrar echoes the outbound Contact in a 2xx REGISTER,
    /// dialog-core starts CRLFCRLF keep-alive pings on the registration
    /// flow. Flow failure is surfaced back into session-core so the
    /// registration can be refreshed.
    pub sip_outbound_enabled: bool,

    /// UA-stable instance URN advertised on outbound REGISTERs (RFC 5626
    /// §4.1). Typically a `urn:uuid:<uuid>` generated once per device and
    /// persisted across process restarts. Without this, the registrar
    /// cannot tell a restarted UA apart from a different device with the
    /// same AoR — flow stickiness breaks.
    ///
    /// When [`Config::sip_outbound_enabled`] is `true` and this is `None`,
    /// a warning is logged and outbound-aware parameters are suppressed
    /// on the REGISTER (falling back to pre-5626 behaviour). Callers
    /// SHOULD supply a stable URN explicitly; leaving it `None` is only
    /// appropriate for single-shot dev / lab usage.
    pub sip_instance: Option<String>,

    /// Interval in seconds between RFC 5626 §5.1 CRLFCRLF keep-alive pings
    /// on long-lived TCP / TLS flows. Default 25 s per the RFC
    /// recommendation (ping every 25 s, flow declared dead after 30 s
    /// without a response).
    ///
    /// This is honored when outbound registration flow support is
    /// enabled with a stable [`Config::sip_instance`].
    pub outbound_keepalive_interval_secs: u64,

    /// SIP TLS signalling mode.
    pub sip_tls_mode: SipTlsMode,

    /// SIP Contact reachability strategy.
    ///
    /// [`SipContactMode::ReachableContact`] is the classic SIP UA model:
    /// the Contact URI is directly reachable by the peer. For SIP TLS that
    /// means this process usually also runs a TLS listener. The registered
    /// flow modes are for proxy/B2BUA deployments where inbound requests are
    /// expected on the existing outbound registration flow.
    pub sip_contact_mode: SipContactMode,

    /// Optional local SIP TLS listener address. Used for
    /// [`SipTlsMode::ServerOnly`] and [`SipTlsMode::ClientAndServer`].
    /// When unset, dialog-core retains its legacy default of deriving the
    /// TLS listener address from [`Config::bind_addr`] by adding 1 to the
    /// port.
    pub tls_bind_addr: Option<SocketAddr>,

    /// Optional advertised address for SIP TLS Via sent-by and fallback
    /// Contact generation. This is distinct from [`Config::tls_bind_addr`]:
    /// bind can be `0.0.0.0`, while the advertised address must be routable
    /// by peers.
    pub tls_advertised_addr: Option<SocketAddr>,

    /// Optional Contact URI override used by dialog-core for
    /// dialog-creating and target-refresh requests. Registrations can
    /// still override Contact per REGISTER via [`Registration`].
    pub contact_uri: Option<String>,

    /// Path to the PEM-encoded TLS listener certificate (RFC 3261
    /// §26.2 / RFC 5630). Required only for [`SipTlsMode::ServerOnly`]
    /// and [`SipTlsMode::ClientAndServer`]. It is not required for
    /// [`SipTlsMode::ClientOnly`], where this endpoint connects to a
    /// remote TLS server and verifies that server's certificate.
    pub tls_cert_path: Option<std::path::PathBuf>,

    /// Path to the PEM-encoded PKCS#8 listener private key matching
    /// [`Config::tls_cert_path`].
    pub tls_key_path: Option<std::path::PathBuf>,

    /// Optional PEM-encoded client certificate chain for mutual TLS.
    /// Leave unset for normal server-authenticated SIP TLS.
    pub tls_client_cert_path: Option<std::path::PathBuf>,

    /// Optional PEM-encoded PKCS#8 private key matching
    /// [`Config::tls_client_cert_path`].
    pub tls_client_key_path: Option<std::path::PathBuf>,

    /// Optional path to a PEM-encoded CA bundle to *add to* the system
    /// trust store on the client side. Used for enterprise PKI / private
    /// carriers where the server cert is signed by a private CA not in
    /// the system root store. Default: `None` (system roots only).
    pub tls_extra_ca_path: Option<std::path::PathBuf>,

    /// **Dev only.** When `true`, server certs are accepted without
    /// identity verification. Required for self-signed test certs. The
    /// TLS handshake still runs end-to-end (encrypted), but a malicious
    /// peer can MITM. Default: `false`. **Must not** be enabled in
    /// production.
    ///
    /// Gated behind the `dev-insecure-tls` Cargo feature — production
    /// builds physically cannot access this field. The matching
    /// `InsecureCertVerifier` in `sip-transport` is also feature-gated,
    /// so even with the feature enabled the verifier type only exists
    /// in the dev-build binary.
    #[cfg(feature = "dev-insecure-tls")]
    pub tls_insecure_skip_verify: bool,

    /// Offer RFC 4568 SDES-SRTP on outgoing INVITEs.
    ///
    /// When `true`:
    ///
    /// - The `m=audio` line in the offer uses `RTP/SAVP` (RFC 4568
    ///   §3.1.4) instead of `RTP/AVP`.
    /// - One `a=crypto:` line per suite in
    ///   [`Config::srtp_offered_suites`] is attached, each with a
    ///   freshly-generated master key (RFC 4568 §6.1).
    /// - When the answer accepts SRTP, paired `SrtpContext`s are
    ///   installed on the outgoing+incoming RTP transport before the
    ///   first packet flows. All RTP payload is then AES-encrypted
    ///   end-to-end per RFC 3711.
    ///
    /// Enable this when targeting:
    /// - **Cloud SIP carriers** (Twilio, Vonage, Bandwidth, Telnyx)
    ///   on production tier — they typically require `srtp=mandatory`.
    /// - **Modern Asterisk / FreeSWITCH** trunks configured with
    ///   `srtp=mandatory`.
    /// - **Microsoft Teams Direct Routing** (which also requires TLS
    ///   for signalling — see [`Config::tls_cert_path`]).
    ///
    /// Leave disabled (the default) for:
    /// - LAN-only PBX deployments where carriers don't enforce SRTP.
    /// - Dev / lab setups exercising the RTP path without crypto
    ///   overhead.
    /// - Codec / RTP profile experiments where SRTP would obscure
    ///   the wire bytes.
    ///
    /// See [`Config::srtp_required`] for the strict-mode variant.
    pub offer_srtp: bool,

    /// Refuse to fall back to plaintext RTP when SRTP can't be
    /// negotiated.
    ///
    /// - **UAC**: a remote SDP answer without an acceptable
    ///   `a=crypto:` line causes the call to surface as
    ///   [`Event::CallFailed`](crate::api::events::Event::CallFailed)
    ///   rather than silently downgrading.
    /// - **UAS**: an offer without `a=crypto:` is rejected with
    ///   `488 Not Acceptable Here`.
    ///
    /// Mirrors the RFC 3261 `Require:` header semantic — fail
    /// loudly rather than silently downgrade a security guarantee.
    /// Pair with [`Config::offer_srtp`] = `true` for the canonical
    /// "I require encrypted media" stance.
    ///
    /// Default: `false` — soft-prefer SRTP but accept plaintext.
    pub srtp_required: bool,

    /// SRTP crypto suites to advertise on outgoing offers, in
    /// preference order. The answerer picks the first suite it
    /// supports.
    ///
    /// Default:
    /// `[AesCm128HmacSha1_80, AesCm128HmacSha1_32]` —
    /// RFC 4568 §6.2.1 MTI suite first (`_80`, ubiquitous), then
    /// `_32` (smaller auth tag for bandwidth-conscious carriers).
    /// Modify when a specific carrier requires a non-default
    /// preference.
    pub srtp_offered_suites: Vec<rvoip_sip_core::types::sdp::CryptoSuite>,

    /// Override the RTP-side public address advertised in SDP `c=` /
    /// `o=` and `m=audio <port>` lines. Use when:
    ///
    /// - The session-core process runs behind a 1:1 NAT or IP alias
    ///   and the operator already knows the external IP/port.
    /// - The deployment uses an SBC that performs media latching, and
    ///   we want to advertise the SBC's public IP rather than rely on
    ///   STUN.
    ///
    /// Mutually exclusive with [`Config::stun_server`]. If both are
    /// set, the static override wins and a warning is logged.
    /// Default: `None` — advertise the local interface address (today's
    /// behaviour).
    pub media_public_addr: Option<SocketAddr>,

    /// STUN server (RFC 8489 §14) to probe for the RTP-side public
    /// mapping at coordinator boot. Format: `"host:port"` or `"host"`
    /// (default port 3478). Common public servers:
    /// `stun.l.google.com:19302`, `stun.cloudflare.com:3478`.
    ///
    /// The probe runs once at startup using the bound RTP socket so
    /// the discovered NAT binding matches the binding outgoing audio
    /// will traverse. Failure mode: probe timeout / unreachable /
    /// unparseable response → log a warning and fall back to the
    /// local interface address. STUN is intentionally soft-fail —
    /// the call path is never blocked on it.
    ///
    /// Default: `None` — no probe runs (today's behaviour).
    pub stun_server: Option<String>,

    /// RFC 3389 Comfort Noise (PT 13) advertisement.
    ///
    /// When `true`, outgoing offers and answers carry `13` in the
    /// `m=audio` format list plus `a=rtpmap:13 CN/8000` so peers know
    /// we accept Comfort Noise during silence periods. The session
    /// also enables media-core comfort-noise support so callers can drive CN
    /// packets through their chosen media-control path.
    ///
    /// Default: `false` — peers see the pre-Sprint-3 PCMU + PCMA +
    /// telephone-event format set with no CN.
    pub comfort_noise_enabled: bool,

    /// RFC 3264 §6 strict codec matching for SDP answers.
    ///
    /// When `true` (default), the SDP answer's format list is the
    /// strict intersection of the offer's formats and our supported
    /// set, in offerer-preference order. RFC-correct: a peer that
    /// offered `0 101` (PCMU + telephone-event only) gets answered
    /// with `0 101`, not `0 8 101`.
    ///
    /// When `false`, the answer always advertises our full supported
    /// set regardless of offer (the pre-Sprint-3.5 permissive
    /// behaviour). Set to `false` for deployments where a carrier or
    /// PBX accidentally relied on the legacy "always full set"
    /// answer shape — this provides a one-line escape hatch back to
    /// the prior behaviour without code changes.
    ///
    /// Default: `true`.
    pub strict_codec_matching: bool,
}

impl Config {
    /// Create a config for local development/testing on 127.0.0.1.
    ///
    /// ```
    /// # use rvoip_session_core::Config;
    /// let config = Config::local("alice", 5060);
    /// assert_eq!(config.local_uri, "sip:alice@127.0.0.1:5060");
    /// ```
    pub fn local(name: &str, port: u16) -> Self {
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        Self {
            local_ip: ip,
            sip_port: port,
            media_port_start: 16000,
            media_port_end: 17000,
            bind_addr: SocketAddr::new(ip, port),
            sip_advertised_addr: None,
            state_table_path: None,
            local_uri: format!("sip:{}@{}:{}", name, ip, port),
            use_100rel: RelUsage::default(),
            session_timer_secs: None,
            session_timer_min_se: 90,
            credentials: None,
            pai_uri: None,
            outbound_proxy_uri: None,
            sip_outbound_enabled: false,
            sip_instance: None,
            outbound_keepalive_interval_secs: 25,
            sip_tls_mode: SipTlsMode::Disabled,
            sip_contact_mode: SipContactMode::ReachableContact,
            tls_bind_addr: None,
            tls_advertised_addr: None,
            contact_uri: None,
            tls_cert_path: None,
            tls_key_path: None,
            tls_client_cert_path: None,
            tls_client_key_path: None,
            tls_extra_ca_path: None,
            #[cfg(feature = "dev-insecure-tls")]
            tls_insecure_skip_verify: false,
            offer_srtp: false,
            srtp_required: false,
            srtp_offered_suites: vec![
                rvoip_sip_core::types::sdp::CryptoSuite::AesCm128HmacSha1_80,
                rvoip_sip_core::types::sdp::CryptoSuite::AesCm128HmacSha1_32,
            ],
            media_public_addr: None,
            stun_server: None,
            comfort_noise_enabled: false,
            strict_codec_matching: true,
        }
    }

    /// Create a config bound to a specific IP address (e.g. for LAN or production).
    ///
    /// ```
    /// # use rvoip_session_core::Config;
    /// let config = Config::on("alice", "192.168.1.50".parse().unwrap(), 5060);
    /// assert_eq!(config.local_uri, "sip:alice@192.168.1.50:5060");
    /// ```
    pub fn on(name: &str, ip: IpAddr, port: u16) -> Self {
        Self {
            local_ip: ip,
            sip_port: port,
            media_port_start: 16000,
            media_port_end: 17000,
            bind_addr: SocketAddr::new(ip, port),
            sip_advertised_addr: None,
            state_table_path: None,
            local_uri: format!("sip:{}@{}:{}", name, ip, port),
            use_100rel: RelUsage::default(),
            session_timer_secs: None,
            session_timer_min_se: 90,
            credentials: None,
            pai_uri: None,
            outbound_proxy_uri: None,
            sip_outbound_enabled: false,
            sip_instance: None,
            outbound_keepalive_interval_secs: 25,
            sip_tls_mode: SipTlsMode::Disabled,
            sip_contact_mode: SipContactMode::ReachableContact,
            tls_bind_addr: None,
            tls_advertised_addr: None,
            contact_uri: None,
            tls_cert_path: None,
            tls_key_path: None,
            tls_client_cert_path: None,
            tls_client_key_path: None,
            tls_extra_ca_path: None,
            #[cfg(feature = "dev-insecure-tls")]
            tls_insecure_skip_verify: false,
            offer_srtp: false,
            srtp_required: false,
            srtp_offered_suites: vec![
                rvoip_sip_core::types::sdp::CryptoSuite::AesCm128HmacSha1_80,
                rvoip_sip_core::types::sdp::CryptoSuite::AesCm128HmacSha1_32,
            ],
            media_public_addr: None,
            stun_server: None,
            comfort_noise_enabled: false,
            strict_codec_matching: true,
        }
    }

    /// Configure SIP TLS as a directly reachable Contact listener.
    ///
    /// The UA will both dial outbound TLS and listen on `tls_bind_addr` for
    /// inbound TLS requests sent to its advertised Contact.
    pub fn tls_reachable_contact(
        mut self,
        tls_bind_addr: SocketAddr,
        cert_path: impl Into<std::path::PathBuf>,
        key_path: impl Into<std::path::PathBuf>,
    ) -> Self {
        self.sip_tls_mode = SipTlsMode::ClientAndServer;
        self.sip_contact_mode = SipContactMode::ReachableContact;
        self.tls_bind_addr = Some(tls_bind_addr);
        if self.tls_advertised_addr.is_none() && !tls_bind_addr.ip().is_unspecified() {
            self.tls_advertised_addr = Some(tls_bind_addr);
        }
        self.tls_cert_path = Some(cert_path.into());
        self.tls_key_path = Some(key_path.into());
        self
    }

    /// Configure SIP TLS for RFC 5626 registered-flow reuse.
    ///
    /// No TLS listener certificate/key is required because inbound requests
    /// are expected on the outbound registration flow.
    pub fn tls_registered_flow_rfc5626(mut self, sip_instance: impl Into<String>) -> Self {
        self.sip_tls_mode = SipTlsMode::ClientOnly;
        self.sip_contact_mode = SipContactMode::RegisteredFlowRfc5626;
        self.sip_outbound_enabled = true;
        self.sip_instance = Some(sip_instance.into());
        self
    }

    /// Configure SIP TLS for PBX symmetric-transport registered-flow reuse.
    ///
    /// This mode keeps the registration flow alive but does not require the
    /// registrar to echo RFC 5626 Contact parameters.
    pub fn tls_registered_flow_symmetric(mut self, sip_instance: impl Into<String>) -> Self {
        self.sip_tls_mode = SipTlsMode::ClientOnly;
        self.sip_contact_mode = SipContactMode::RegisteredFlowSymmetric;
        self.sip_instance = Some(sip_instance.into());
        self
    }

    /// Validate the SIP TLS/contact-mode configuration.
    pub fn validate(&self) -> Result<()> {
        let effective_tls_mode = self.effective_tls_mode();

        if self.tls_cert_path.is_some() ^ self.tls_key_path.is_some() {
            return Err(SessionError::ConfigError(
                "TLS listener certificate and key must be provided together".to_string(),
            ));
        }
        if self.tls_client_cert_path.is_some() ^ self.tls_client_key_path.is_some() {
            return Err(SessionError::ConfigError(
                "TLS client certificate and key must be provided together".to_string(),
            ));
        }
        if matches!(
            effective_tls_mode,
            SipTlsMode::ServerOnly | SipTlsMode::ClientAndServer
        ) && (self.tls_cert_path.is_none() || self.tls_key_path.is_none())
        {
            return Err(SessionError::ConfigError(
                "SIP TLS listener modes require tls_cert_path and tls_key_path".to_string(),
            ));
        }

        match self.sip_contact_mode {
            SipContactMode::ReachableContact => match effective_tls_mode {
                SipTlsMode::Disabled => {}
                SipTlsMode::ClientOnly => {
                    if self.contact_uri.is_none() {
                        return Err(SessionError::ConfigError(
                            "reachable TLS Contact mode with ClientOnly requires an explicit external contact_uri".to_string(),
                        ));
                    }
                }
                SipTlsMode::ServerOnly | SipTlsMode::ClientAndServer => {
                    if self.tls_bind_addr.is_none() {
                        return Err(SessionError::ConfigError(
                            "reachable TLS Contact mode requires tls_bind_addr".to_string(),
                        ));
                    }
                    if self.tls_cert_path.is_none() || self.tls_key_path.is_none() {
                        return Err(SessionError::ConfigError(
                            "reachable TLS Contact mode requires tls_cert_path and tls_key_path"
                                .to_string(),
                        ));
                    }
                }
            },
            SipContactMode::RegisteredFlowRfc5626 => {
                if !matches!(
                    effective_tls_mode,
                    SipTlsMode::ClientOnly | SipTlsMode::ClientAndServer
                ) {
                    return Err(SessionError::ConfigError(
                        "RFC 5626 registered-flow mode requires SIP TLS ClientOnly or ClientAndServer".to_string(),
                    ));
                }
                if !self.sip_outbound_enabled {
                    return Err(SessionError::ConfigError(
                        "RFC 5626 registered-flow mode requires sip_outbound_enabled=true"
                            .to_string(),
                    ));
                }
                if self
                    .sip_instance
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_none()
                {
                    return Err(SessionError::ConfigError(
                        "RFC 5626 registered-flow mode requires a stable sip_instance URN"
                            .to_string(),
                    ));
                }
            }
            SipContactMode::RegisteredFlowSymmetric => {
                if !matches!(
                    effective_tls_mode,
                    SipTlsMode::ClientOnly | SipTlsMode::ClientAndServer
                ) {
                    return Err(SessionError::ConfigError(
                        "symmetric registered-flow mode requires SIP TLS ClientOnly or ClientAndServer".to_string(),
                    ));
                }
            }
        }

        Ok(())
    }

    fn effective_tls_mode(&self) -> SipTlsMode {
        if self.sip_tls_mode == SipTlsMode::Disabled
            && self.tls_cert_path.is_some()
            && self.tls_key_path.is_some()
        {
            SipTlsMode::ClientAndServer
        } else {
            self.sip_tls_mode
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config::local("user", 5060)
    }
}

/// Lower-level coordinator for SIP sessions, registrations, media, and events.
///
/// `UnifiedCoordinator` is intentionally explicit: most methods take or return
/// a [`SessionId`], and event consumers choose whether to subscribe to all
/// events or filter by session. This makes it suitable for applications that
/// manage more than one call leg at a time.
///
/// Use higher-level wrappers when possible:
///
/// - [`StreamPeer`](crate::api::stream_peer::StreamPeer) for sequential clients
///   and tests.
/// - [`CallbackPeer`](crate::api::callback_peer::CallbackPeer) for reactive
///   servers.
#[allow(dead_code)]
pub struct UnifiedCoordinator {
    /// State machine helpers
    pub(crate) helpers: Arc<StateMachineHelpers>,

    /// Media adapter for audio operations
    media_adapter: Arc<MediaAdapter>,

    /// Dialog adapter for SIP operations
    dialog_adapter: Arc<DialogAdapter>,

    /// Incoming call receiver
    incoming_rx: Arc<RwLock<mpsc::Receiver<IncomingCallInfo>>>,

    /// Global event coordinator — used to publish and subscribe to session API events.
    /// Events are published to the "session_to_app" channel.
    pub(crate) global_coordinator: Arc<GlobalEventCoordinator>,

    /// Configuration
    config: Config,

    /// Shutdown signal — send `true` to stop all background tasks.
    shutdown_tx: tokio::sync::watch::Sender<bool>,
}

impl UnifiedCoordinator {
    /// Create and start a new coordinator.
    ///
    /// This validates [`Config`], initializes dialog and media adapters,
    /// starts the central event handler, and returns a shared coordinator
    /// handle. Background tasks are stopped by calling [`shutdown`](Self::shutdown)
    /// or by dropping all coordinator owners.
    pub async fn new(config: Config) -> Result<Arc<Self>> {
        config.validate()?;

        // Get the global event coordinator singleton
        let global_coordinator = rvoip_infra_common::events::global_coordinator()
            .await
            .clone();

        // Create core components
        let store = Arc::new(SessionStore::new());
        let registry = Arc::new(SessionRegistry::new());

        // Create adapters
        let dialog_api = Self::create_dialog_api(&config, global_coordinator.clone()).await?;

        // E4: parse the outbound proxy URI once up-front so a malformed
        // config fails loudly at coordinator boot, not per-call.
        let outbound_proxy_uri = if let Some(s) = config.outbound_proxy_uri.as_ref() {
            use std::str::FromStr;
            match rvoip_sip_core::types::uri::Uri::from_str(s) {
                Ok(u) => Some(u),
                Err(e) => {
                    return Err(crate::errors::SessionError::ConfigurationError(format!(
                        "Config.outbound_proxy_uri ({}) is not a valid SIP URI: {}",
                        s, e
                    )));
                }
            }
        } else {
            None
        };

        // Build RFC 5626 outbound Contact params from config. Require both
        // the outbound flag and a stable instance URN unless validation has
        // already made that mode mandatory.
        let outbound_contact_params = if config.sip_outbound_enabled
            || matches!(
                config.sip_contact_mode,
                SipContactMode::RegisteredFlowRfc5626
            ) {
            if let Some(instance) = config.sip_instance.as_ref() {
                Some(rvoip_sip_core::types::outbound::OutboundContactParams {
                    instance_urn: instance.clone(),
                    reg_id: 1,
                })
            } else {
                tracing::warn!(
                    "Config.sip_outbound_enabled is true but sip_instance is None; \
                     falling back to pre-5626 REGISTER Contact. Provide a stable \
                     urn:uuid:<uuid> in Config.sip_instance to enable RFC 5626."
                );
                None
            }
        } else {
            None
        };

        let symmetric_flow_params = if matches!(
            config.sip_contact_mode,
            SipContactMode::RegisteredFlowSymmetric
        ) {
            Some(rvoip_sip_core::types::outbound::OutboundContactParams {
                instance_urn: config
                    .sip_instance
                    .clone()
                    .unwrap_or_else(|| format!("symmetric:{}", config.local_uri)),
                reg_id: 1,
            })
        } else {
            None
        };

        // Thread the registered-flow keep-alive interval into the
        // DialogManager so REGISTER 2xx responses can spawn CRLFCRLF
        // ping tasks. RFC 5626 mode starts after outbound Contact echo;
        // symmetric mode starts after a successful REGISTER.
        if (outbound_contact_params.is_some() || symmetric_flow_params.is_some())
            && config.outbound_keepalive_interval_secs > 0
        {
            dialog_api
                .dialog_manager()
                .core()
                .set_outbound_keepalive_interval(Some(std::time::Duration::from_secs(
                    config.outbound_keepalive_interval_secs,
                )));
        }

        let dialog_adapter = Arc::new(DialogAdapter::new(
            dialog_api,
            store.clone(),
            global_coordinator.clone(),
            outbound_proxy_uri,
            outbound_contact_params,
            symmetric_flow_params,
        ));

        let media_controller =
            Self::create_media_controller(&config, global_coordinator.clone()).await?;
        let mut media_adapter_inner = MediaAdapter::new(
            media_controller,
            store.clone(),
            config.local_ip,
            config.media_port_start,
            config.media_port_end,
        );
        // Apply RFC 4568 SDES-SRTP policy from Config (Step 2B.1).
        media_adapter_inner.set_srtp_policy(
            config.offer_srtp,
            config.srtp_required,
            config.srtp_offered_suites.clone(),
        );
        // Sprint 3 C1 — propagate Comfort Noise opt-in.
        media_adapter_inner.set_comfort_noise(config.comfort_noise_enabled);
        // Sprint 3.5 — propagate strict codec matching policy.
        media_adapter_inner.set_strict_codec_matching(config.strict_codec_matching);
        let media_adapter = Arc::new(media_adapter_inner);

        // Sprint 3 A6 — resolve the public RTP address. Static
        // override wins over STUN; STUN failure is soft (warn + use
        // local IP). Probe runs once, here, before any session is
        // created.
        if let Some(static_addr) = config.media_public_addr {
            if config.stun_server.is_some() {
                tracing::warn!(
                    "Both Config::media_public_addr and Config::stun_server are set; \
                     using the static override and skipping the STUN probe"
                );
            }
            tracing::info!(
                "RTP public addr: {} (static override from Config::media_public_addr)",
                static_addr
            );
            media_adapter.set_public_rtp_addr(Some(static_addr));
        } else if let Some(ref stun_target) = config.stun_server {
            // Probe runs in the background to keep coordinator boot
            // snappy — but the soft-fail design means downstream code
            // doesn't block on the result. The first session created
            // *after* the probe lands picks up the override.
            let adapter_for_probe = media_adapter.clone();
            let stun_target = stun_target.clone();
            tokio::spawn(async move {
                if let Err(e) = run_stun_probe(adapter_for_probe, &stun_target).await {
                    tracing::warn!(
                        "STUN probe failed against '{}': {} — falling back to local IP",
                        stun_target,
                        e
                    );
                }
            });
        }
        // RFC 4733 DTMF bridge: adapter publishes `Event::DtmfReceived`
        // onto the API bus whenever media-core signals a DTMF event.
        media_adapter
            .set_global_coordinator(global_coordinator.clone())
            .await;

        // Load state table based on config
        let state_table = Arc::new(crate::state_table::load_state_table_with_config(
            config.state_table_path.as_deref(),
        ));

        let (state_event_tx, state_event_rx) =
            mpsc::channel::<crate::state_machine::executor::SessionEvent>(100);

        let state_machine = Arc::new(StateMachine::new_with_custom_table(
            state_table,
            store.clone(),
            dialog_adapter.clone(),
            media_adapter.clone(),
            state_event_tx,
        ));

        // Wire the state machine into the dialog adapter (for REGISTER
        // response handling). The adapter holds an `Arc<OnceLock<_>>`
        // internally so this post-construction init is sound without
        // `unsafe`.
        let _ = dialog_adapter.init_state_machine(state_machine.clone());

        // Create helpers
        let helpers = Arc::new(StateMachineHelpers::new(state_machine.clone()));

        // Create incoming call channel
        let (incoming_tx, incoming_rx) = mpsc::channel(100);
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        let coordinator = Arc::new(Self {
            helpers,
            media_adapter: media_adapter.clone(),
            dialog_adapter: dialog_adapter.clone(),
            incoming_rx: Arc::new(RwLock::new(incoming_rx)),
            global_coordinator: global_coordinator.clone(),
            config,
            shutdown_tx,
        });

        // Start the dialog adapter
        dialog_adapter.start().await?;

        // Create and start the centralized event handler.
        // Events are published to the global coordinator's "session_to_app" channel.
        let event_handler =
            crate::adapters::SessionCrossCrateEventHandler::with_event_broadcast_and_state_machine_events(
                state_machine.clone(),
                global_coordinator.clone(),
                dialog_adapter.clone(),
                media_adapter.clone(),
                registry.clone(),
                incoming_tx,
                state_event_rx,
            );

        // Start the event handler (sets up channels and subscriptions)
        event_handler.start(shutdown_rx).await?;

        Ok(coordinator)
    }

    /// Create a new coordinator with SimplePeer event integration.
    ///
    /// **Deprecated** — use [`UnifiedCoordinator::new()`] then [`subscribe_events()`][Self::subscribe_events].
    /// The `simple_peer_event_tx` parameter is ignored; events are now broadcast internally.
    #[deprecated(note = "Use UnifiedCoordinator::new() then subscribe_events()")]
    pub async fn with_simple_peer_events(
        config: Config,
        _simple_peer_event_tx: tokio::sync::mpsc::Sender<crate::api::events::Event>,
    ) -> Result<Arc<Self>> {
        Self::new(config).await
    }

    // ===== Shutdown =====

    /// Shut down this coordinator and all its background tasks.
    ///
    /// After calling this, the coordinator stops processing events. Existing
    /// call sessions are not explicitly terminated; use [`hangup`](Self::hangup)
    /// first if you need clean call teardown.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    /// Return a cloneable handle that can signal
    /// [`shutdown`](Self::shutdown) from another task. Mirrors
    /// [`CallbackPeer::shutdown_handle`].
    ///
    /// [`CallbackPeer::shutdown_handle`]: crate::api::callback_peer::CallbackPeer::shutdown_handle
    pub fn shutdown_handle(&self) -> crate::api::callback_peer::ShutdownHandle {
        crate::api::callback_peer::ShutdownHandle::from_sender(self.shutdown_tx.clone())
    }

    // ===== Event Subscription =====

    /// Subscribe to the raw cross-crate session API event stream.
    ///
    /// Returns an independent `mpsc::Receiver` for events published by this
    /// coordinator on the internal `"session_to_app"` channel. Most
    /// application code should prefer [`events`](Self::events), which wraps
    /// this raw receiver and yields typed [`Event`](crate::api::events::Event)
    /// values.
    ///
    /// Use this method only when building a custom peer type or diagnostic
    /// tool that needs access to the raw event envelope.
    pub async fn subscribe_events(
        &self,
    ) -> crate::errors::Result<
        tokio::sync::mpsc::Receiver<
            std::sync::Arc<dyn rvoip_infra_common::events::cross_crate::CrossCrateEvent>,
        >,
    > {
        self.global_coordinator
            .subscribe(crate::adapters::SESSION_TO_APP_CHANNEL)
            .await
            .map_err(|e| {
                crate::errors::SessionError::InternalError(format!(
                    "Failed to subscribe to session events: {}",
                    e
                ))
            })
    }

    /// Return a typed, unfiltered [`EventReceiver`](crate::api::stream_peer::EventReceiver) that yields
    /// [`crate::api::events::Event`] values across all sessions.
    ///
    /// Use when a single consumer needs every session API event (b2bua
    /// coordinator, activity log). For per-leg logic prefer
    /// [`events_for_session`][Self::events_for_session].
    ///
    /// The returned receiver already handles the downcast from the raw
    /// cross-crate broadcast and exposes filtering helpers like
    /// [`EventReceiver::next_dtmf`](crate::api::stream_peer::EventReceiver::next_dtmf),
    /// [`EventReceiver::next_incoming`](crate::api::stream_peer::EventReceiver::next_incoming),
    /// and
    /// [`EventReceiver::next_transfer`](crate::api::stream_peer::EventReceiver::next_transfer).
    pub async fn events(&self) -> Result<crate::api::stream_peer::EventReceiver> {
        let rx = self.subscribe_events().await?;
        Ok(crate::api::stream_peer::EventReceiver::new(rx))
    }

    /// Return an [`EventReceiver`](crate::api::stream_peer::EventReceiver) that only yields events whose
    /// `call_id` matches `id`. Per-session filtering happens in the
    /// receiver's `next()` loop.
    ///
    /// Intended for b2bua-style consumers that need to watch both legs of
    /// a bridged call independently:
    ///
    /// ```no_run
    /// # use rvoip_session_core::{Event, SessionId, UnifiedCoordinator};
    /// # async fn example(coord: &UnifiedCoordinator, inbound: &SessionId, outbound: &SessionId) {
    /// let mut inbound_events = coord.events_for_session(inbound).await.unwrap();
    /// let mut outbound_events = coord.events_for_session(outbound).await.unwrap();
    /// tokio::select! {
    ///     Some(Event::CallEnded { .. }) = inbound_events.next() => {
    ///         // inbound leg ended — tear down the outbound leg
    ///     }
    ///     Some(Event::CallEnded { .. }) = outbound_events.next() => {
    ///         // outbound leg ended — tear down the inbound leg
    ///     }
    /// }
    /// # }
    /// ```
    ///
    /// **Caller contract:** open the receiver *before* any event of
    /// interest fires. Events are lost if no subscriber is attached at
    /// publish time. For incoming calls the safe pattern is:
    /// 1. Wait for an `IncomingCall` event on the unfiltered
    ///    [`events()`][Self::events] receiver.
    /// 2. Open `events_for_session(&id)` with the new `SessionId`.
    /// 3. Call `accept_call_with_sdp()` (post-acceptance events then
    ///    reach the filtered receiver).
    pub async fn events_for_session(
        &self,
        id: &SessionId,
    ) -> Result<crate::api::stream_peer::EventReceiver> {
        let rx = self.subscribe_events().await?;
        Ok(crate::api::stream_peer::EventReceiver::filtered(
            rx,
            id.clone(),
        ))
    }

    // ===== Simple Call Operations =====

    /// Make an outgoing call. If the `Config.credentials` default is set,
    /// those credentials are applied to the session before dispatch so the
    /// state machine can transparently retry on a 401/407 INVITE challenge
    /// (RFC 3261 §22.2). Likewise, if `Config.pai_uri` is set, a typed
    /// `P-Asserted-Identity` (RFC 3325) header is attached to the very
    /// first INVITE.
    pub async fn make_call(&self, from: &str, to: &str) -> Result<SessionId> {
        self.helpers
            .make_call_with_credentials_and_pai(
                from,
                to,
                self.config.credentials.clone(),
                self.config.pai_uri.clone(),
            )
            .await
    }

    /// Make an outgoing call with explicit credentials, overriding the
    /// per-peer default. Useful for multi-tenant clients where each call
    /// authenticates with a different user.
    pub async fn make_call_with_auth(
        &self,
        from: &str,
        to: &str,
        credentials: crate::types::Credentials,
    ) -> Result<SessionId> {
        self.helpers
            .make_call_with_credentials_and_pai(
                from,
                to,
                Some(credentials),
                self.config.pai_uri.clone(),
            )
            .await
    }

    /// Make an outgoing call attaching a per-call `P-Asserted-Identity` URI
    /// (RFC 3325 §9.1), overriding `Config::pai_uri`. Useful for
    /// multi-tenant trunking where each call asserts a different identity.
    /// Pass `None` for `pai` to suppress the header for this call only.
    pub async fn make_call_with_pai(
        &self,
        from: &str,
        to: &str,
        pai: Option<String>,
    ) -> Result<SessionId> {
        self.helpers
            .make_call_with_credentials_and_pai(from, to, self.config.credentials.clone(), pai)
            .await
    }

    /// Spawn an outbound leg linked to a transferor session for RFC 3515
    /// §2.4.5 progress reporting. The new leg's `SessionState` carries
    /// `transferor_session_id = Some(..)` before the state machine
    /// dispatches `MakeCall`, so every subsequent `Dialog180Ringing` /
    /// `Dialog200OK` / failure fires a progress NOTIFY back on the
    /// transferor's REFER subscription. This is the b2bua wrapper crate's
    /// primary REFER-forwarding entry point.
    pub async fn make_transfer_leg(
        &self,
        from: &str,
        to: &str,
        transferor_session_id: &SessionId,
    ) -> Result<SessionId> {
        self.helpers
            .make_transfer_leg(from, to, transferor_session_id)
            .await
    }

    /// Retroactively link an existing session as a transfer leg of
    /// `transferor_session_id`. Prefer [`make_transfer_leg`](Self::make_transfer_leg) — this
    /// lower-level primitive accepts a race window in which dialog
    /// events fired before the linkage is set silently drop their
    /// corresponding progress NOTIFY.
    pub async fn set_transferor_session(
        &self,
        leg_session_id: &SessionId,
        transferor_session_id: &SessionId,
    ) -> Result<()> {
        self.helpers
            .set_transferor_session(leg_session_id, transferor_session_id)
            .await
    }

    /// Accept an incoming call
    pub async fn accept_call(&self, session_id: &SessionId) -> Result<()> {
        self.helpers.accept_call(session_id).await
    }

    /// Accept an incoming call with a caller-supplied SDP answer. Bypasses
    /// local media negotiation — intended for b2bua flows where the answer
    /// body comes from the outbound leg's 200 OK. See
    /// [`StateMachineHelpers::accept_call_with_sdp`] for the mechanism.
    pub async fn accept_call_with_sdp(&self, session_id: &SessionId, sdp: String) -> Result<()> {
        self.helpers.accept_call_with_sdp(session_id, sdp).await
    }

    /// Reject an incoming call with a specific SIP status code and reason phrase.
    pub async fn reject_call(
        &self,
        session_id: &SessionId,
        status: u16,
        reason: &str,
    ) -> Result<()> {
        self.helpers.reject_call(session_id, status, reason).await
    }

    /// Redirect an incoming call to one or more alternate URIs (RFC 3261
    /// §8.1.3.4 / §21.3). Sends a 3xx response with a `Contact:` header
    /// listing the supplied URIs. `status` should be 300-399; `contacts`
    /// must be non-empty.
    pub async fn redirect_call(
        &self,
        session_id: &SessionId,
        status: u16,
        contacts: Vec<String>,
    ) -> Result<()> {
        self.helpers
            .redirect_call(session_id, status, contacts)
            .await
    }

    /// Hangup a call
    pub async fn hangup(&self, session_id: &SessionId) -> Result<()> {
        self.helpers.hangup(session_id).await
    }

    /// Bridge the RTP streams of two active sessions at the media layer.
    ///
    /// Transparent packet-level relay: inbound RTP from session A is
    /// forwarded as outbound RTP on session B and vice versa, without
    /// transcoding. Intended for b2bua-style consumers that need to connect
    /// two SIP legs without shuffling AudioFrames through app code.
    ///
    /// # Preconditions
    ///
    /// - Both sessions must exist and be in `CallState::Active` (i.e. have
    ///   a negotiated remote RTP address).
    /// - Both sessions must have negotiated the same codec payload type.
    ///   Codec mismatch returns [`BridgeError::CodecMismatch`].
    /// - Neither session may already be bridged.
    ///
    /// Dropping the returned [`BridgeHandle`] tears the bridge down. DTMF
    /// (RFC 2833) rides the RTP stream and is forwarded transparently;
    /// RTCP is not bridged — each leg keeps generating its own reports.
    pub async fn bridge(
        &self,
        session_a: &SessionId,
        session_b: &SessionId,
    ) -> std::result::Result<BridgeHandle, BridgeError> {
        self.media_adapter
            .bridge_rtp_sessions(session_a, session_b)
            .await
    }

    /// Send a reliable 183 Session Progress with early-media SDP (RFC 3262).
    ///
    /// - `sdp: Some(body)` sends the supplied SDP verbatim.
    /// - `sdp: None` generates an answer from the stored remote offer via
    ///   `MediaAdapter::negotiate_sdp_as_uas` (same path as `accept_call`).
    ///
    /// Fails fast with `UnreliableProvisionalsNotSupported` when the peer
    /// did not advertise `Supported: 100rel` on the INVITE. Transitions the
    /// session to `CallState::EarlyMedia`. Valid from `Ringing` and
    /// `EarlyMedia` (re-emission updates the SDP and bumps `RSeq`).
    pub async fn send_early_media(
        &self,
        session_id: &SessionId,
        sdp: Option<String>,
    ) -> Result<()> {
        if !self.dialog_adapter.peer_supports_100rel(session_id).await? {
            return Err(SessionError::UnreliableProvisionalsNotSupported);
        }
        self.helpers.send_early_media(session_id, sdp).await
    }

    /// Swap the audio source on the running transmitter for a session.
    ///
    /// Typical use: after [`send_early_media`][Self::send_early_media] has
    /// put the session into `EarlyMedia` (which starts a pass-through
    /// transmitter by default), call this to replace silence with a
    /// ringback tone, a "please hold" WAV, or any other
    /// [`AudioSource`] variant.
    ///
    /// On transition to `Active` (after `accept_call`), the state machine
    /// automatically swaps the transmitter back to `AudioSource::PassThrough`
    /// so bidirectional audio flows without further action from the app.
    /// Apps that want a *different* source after answer (e.g., continued
    /// announcement playback over an active call) should call this method
    /// again *after* the `CallEstablished` event fires.
    pub async fn set_audio_source(
        &self,
        session_id: &SessionId,
        source: AudioSource,
    ) -> Result<()> {
        self.media_adapter
            .set_audio_source(session_id, source)
            .await
    }

    /// Put a call on hold
    pub async fn hold(&self, session_id: &SessionId) -> Result<()> {
        self.helpers
            .state_machine
            .process_event(session_id, EventType::HoldCall)
            .await?;
        Ok(())
    }

    /// Resume a call from hold
    pub async fn resume(&self, session_id: &SessionId) -> Result<()> {
        self.helpers
            .state_machine
            .process_event(session_id, EventType::ResumeCall)
            .await?;
        Ok(())
    }

    // ===== Conference Operations =====

    /// Create a conference from an active call
    pub async fn create_conference(&self, session_id: &SessionId, name: &str) -> Result<()> {
        self.helpers.create_conference(session_id, name).await
    }

    /// Add a participant to a conference
    pub async fn add_to_conference(
        &self,
        host_session_id: &SessionId,
        participant_session_id: &SessionId,
    ) -> Result<()> {
        self.helpers
            .add_to_conference(host_session_id, participant_session_id)
            .await
    }

    /// Join an existing conference
    pub async fn join_conference(&self, session_id: &SessionId, conference_id: &str) -> Result<()> {
        self.helpers
            .state_machine
            .process_event(
                session_id,
                EventType::JoinConference {
                    conference_id: conference_id.to_string(),
                },
            )
            .await?;
        Ok(())
    }

    // ===== Event System Integration =====
    // Callback registry removed - using event-driven approach via SimplePeer

    /// Terminate the current session (for single session constraint)
    pub async fn terminate_current_session(&self) -> Result<()> {
        // Get the current session ID
        if let Some(session_id) = self
            .helpers
            .state_machine
            .store
            .get_current_session_id()
            .await
        {
            self.hangup(&session_id).await
        } else {
            Ok(()) // No session to terminate
        }
    }

    /// Send REFER message to initiate transfer (this will trigger callback on recipient)
    pub async fn send_refer(&self, session_id: &SessionId, refer_to: &str) -> Result<()> {
        if let Ok(mut session) = self
            .helpers
            .state_machine
            .store
            .get_session(session_id)
            .await
        {
            session.transfer_target = Some(refer_to.to_string());
            session.transfer_state = crate::session_store::state::TransferState::TransferInitiated;
            self.helpers
                .state_machine
                .store
                .update_session(session)
                .await?;
        }

        self.dialog_adapter
            .send_refer_session(session_id, refer_to)
            .await
    }

    /// Accept a pending inbound REFER request and send RFC 3515 acceptance
    /// responses/NOTIFYs through the state machine.
    pub async fn accept_refer(&self, session_id: &SessionId) -> Result<()> {
        let session = self
            .helpers
            .state_machine
            .store
            .get_session(session_id)
            .await?;
        let refer_to = session.transfer_target.clone().ok_or_else(|| {
            SessionError::Other(format!(
                "No pending REFER target for session {}",
                session_id
            ))
        })?;
        let transaction_id = session.refer_transaction_id.clone().ok_or_else(|| {
            SessionError::Other(format!(
                "No pending REFER transaction for session {}",
                session_id
            ))
        })?;

        self.helpers
            .state_machine
            .process_event(
                session_id,
                EventType::TransferRequested {
                    refer_to,
                    transfer_type: "blind".to_string(),
                    transaction_id: transaction_id.clone(),
                },
            )
            .await?;

        if let Ok(mut session) = self
            .helpers
            .state_machine
            .store
            .get_session(session_id)
            .await
        {
            if session.refer_transaction_id.as_deref() == Some(transaction_id.as_str()) {
                session.refer_transaction_id = None;
                self.helpers
                    .state_machine
                    .store
                    .update_session(session)
                    .await?;
            }
        }

        Ok(())
    }

    /// Reject a pending inbound REFER request with a final response.
    pub async fn reject_refer(
        &self,
        session_id: &SessionId,
        status_code: u16,
        reason: &str,
    ) -> Result<()> {
        let session = self
            .helpers
            .state_machine
            .store
            .get_session(session_id)
            .await?;
        let transaction_id = session.refer_transaction_id.clone().ok_or_else(|| {
            SessionError::Other(format!(
                "No pending REFER transaction for session {}",
                session_id
            ))
        })?;

        let event = rvoip_infra_common::events::cross_crate::RvoipCrossCrateEvent::SessionToDialog(
            rvoip_infra_common::events::cross_crate::SessionToDialogEvent::ReferResponse {
                transaction_id: transaction_id.clone(),
                accept: false,
                status_code,
                reason: reason.to_string(),
            },
        );

        self.global_coordinator
            .publish(Arc::new(event))
            .await
            .map_err(|e| {
                SessionError::Other(format!("Failed to publish REFER rejection: {}", e))
            })?;

        if let Ok(mut session) = self
            .helpers
            .state_machine
            .store
            .get_session(session_id)
            .await
        {
            if session.refer_transaction_id.as_deref() == Some(transaction_id.as_str()) {
                session.refer_transaction_id = None;
                session.transfer_target = None;
                session.transfer_state = crate::session_store::state::TransferState::None;
                self.helpers
                    .state_machine
                    .store
                    .update_session(session)
                    .await?;
            }
        }

        Ok(())
    }

    /// Send an in-dialog INFO request (RFC 6086) with a caller-chosen
    /// `Content-Type`.
    ///
    /// Used for SIP-INFO DTMF (`application/dtmf-relay` — some carriers
    /// prefer this over in-band RFC 2833), fax flow control
    /// (`application/sipfrag`), and other application-level mid-dialog
    /// signalling.
    ///
    /// The call must already be in an established dialog (past `Active`).
    /// The supplied `body` is sent verbatim; the method does not transcode
    /// or validate it against the declared content type.
    pub async fn send_info(
        &self,
        session_id: &SessionId,
        content_type: &str,
        body: &[u8],
    ) -> Result<()> {
        self.dialog_adapter
            .send_info(session_id, content_type, body)
            .await
    }

    /// Send a general-purpose NOTIFY request (RFC 6665) on an established
    /// dialog. `event_package` populates the `Event:` header; the raw
    /// `subscription_state` string is forwarded verbatim to dialog-core,
    /// which parses it into a typed `Subscription-State:` header.
    ///
    /// RFC 3515 §2.4.5 REFER progress NOTIFYs are emitted automatically
    /// by the state machine for transfer legs created through
    /// [`UnifiedCoordinator::make_transfer_leg`]. This method is the
    /// escape hatch for other event packages (dialog, message-summary,
    /// presence, custom) and for non-standard REFER orchestration.
    pub async fn send_notify(
        &self,
        session_id: &SessionId,
        event_package: &str,
        body: Option<String>,
        subscription_state: Option<String>,
    ) -> Result<()> {
        self.dialog_adapter
            .send_notify(session_id, event_package, body, subscription_state)
            .await
    }

    /// Send NOTIFY message for REFER status (used after handling transfer)
    pub async fn send_refer_notify(
        &self,
        session_id: &SessionId,
        status_code: u16,
        reason: &str,
    ) -> Result<()> {
        self.dialog_adapter
            .send_refer_notify(session_id, status_code, reason)
            .await
    }

    /// Send REFER with a pre-built `Replaces` header value (RFC 3891).
    ///
    /// Primitive for attended-transfer orchestration: a caller managing two
    /// sessions (original + consultation) constructs the Replaces value from
    /// the consultation session's [`DialogIdentity`](crate::api::types::DialogIdentity) and passes it here for
    /// the original session to send.
    pub async fn send_refer_with_replaces(
        &self,
        session_id: &SessionId,
        target_uri: &str,
        replaces: &str,
    ) -> Result<()> {
        if let Ok(mut session) = self
            .helpers
            .state_machine
            .store
            .get_session(session_id)
            .await
        {
            session.transfer_target = Some(target_uri.to_string());
            session.replaces_header = Some(replaces.to_string());
            session.transfer_state = crate::session_store::state::TransferState::TransferInitiated;
            self.helpers
                .state_machine
                .store
                .update_session(session)
                .await?;
        }

        self.dialog_adapter
            .send_refer_with_replaces(session_id, target_uri, replaces)
            .await
    }

    /// Fetch the SIP-level identity (`Call-ID`, local/remote tags) of a
    /// session's dialog. Returns `None` if the dialog isn't established
    /// yet or has already been cleaned up.
    pub async fn dialog_identity(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<crate::api::types::DialogIdentity>> {
        self.dialog_adapter.dialog_identity(session_id).await
    }

    // ===== DTMF Operations =====

    /// Send a single RFC 4733 DTMF digit over the active media session
    /// at a 100 ms default duration (suitable for interactive softphone
    /// use).
    ///
    /// Goes directly through [`MediaAdapter::send_dtmf_rfc4733`] rather
    /// than the state machine: DTMF is an in-call side-effect, not a
    /// state transition, and the state table does not (intentionally)
    /// enumerate a SendDTMF transition. The media adapter resolves
    /// `session_id → dialog_id`, encodes the RFC 4733 telephone-event
    /// payload, and transmits with PT 101 over the existing RTP
    /// session.
    pub async fn send_dtmf(&self, session_id: &SessionId, digit: char) -> Result<()> {
        self.media_adapter
            .send_dtmf_rfc4733(session_id, digit, 100)
            .await
    }

    // ===== Recording Operations =====

    /// Start recording a call
    pub async fn start_recording(&self, session_id: &SessionId) -> Result<()> {
        self.helpers
            .state_machine
            .process_event(session_id, EventType::StartRecording)
            .await?;
        Ok(())
    }

    /// Stop recording a call
    pub async fn stop_recording(&self, session_id: &SessionId) -> Result<()> {
        self.helpers
            .state_machine
            .process_event(session_id, EventType::StopRecording)
            .await?;
        Ok(())
    }

    // ===== Query Operations =====

    /// Get session information
    pub async fn get_session_info(&self, session_id: &SessionId) -> Result<SessionInfo> {
        self.helpers.get_session_info(session_id).await
    }

    /// List all active sessions
    pub async fn list_sessions(&self) -> Vec<SessionInfo> {
        self.helpers.list_sessions().await
    }

    /// Get current state of a session
    pub async fn get_state(&self, session_id: &SessionId) -> Result<CallState> {
        self.helpers.get_state(session_id).await
    }

    /// Check if session is in conference
    pub async fn is_in_conference(&self, session_id: &SessionId) -> Result<bool> {
        self.helpers.is_in_conference(session_id).await
    }

    // ===== Audio Operations =====

    /// Subscribe to audio frames for a session
    pub async fn subscribe_to_audio(
        &self,
        session_id: &SessionId,
    ) -> Result<crate::types::AudioFrameSubscriber> {
        self.media_adapter
            .subscribe_to_audio_frames(session_id)
            .await
    }

    /// Send audio frame to a session
    pub async fn send_audio(&self, session_id: &SessionId, frame: AudioFrame) -> Result<()> {
        self.media_adapter.send_audio_frame(session_id, frame).await
    }

    // ===== Event Subscriptions =====

    /// Subscribe to session events
    pub async fn subscribe<F>(&self, session_id: SessionId, callback: F)
    where
        F: Fn(crate::state_machine::helpers::SessionEvent) + Send + Sync + 'static,
    {
        self.helpers.subscribe(session_id, callback).await
    }

    /// Unsubscribe from session events
    pub async fn unsubscribe(&self, session_id: &SessionId) {
        self.helpers.unsubscribe(session_id).await
    }

    // ===== Incoming Call Handling =====

    /// Get the next incoming call
    pub async fn get_incoming_call(&self) -> Option<IncomingCallInfo> {
        self.incoming_rx.write().await.recv().await
    }

    // ===== Auto-Transfer Handling =====

    /// Enable automatic blind transfer handling - DISABLED
    /// Auto-transfer now handled in SessionEventHandler to avoid event stealing
    pub fn enable_auto_transfer(self: &Arc<Self>) {
        tracing::info!("🔄 Auto-transfer: handled by SessionEventHandler");
    }

    // extract_field method removed - no longer needed without transfer coordinator

    // ===== Server-Side Registration =====

    /// Start server-side registration handling
    ///
    /// This creates and starts a RegistrationAdapter that handles incoming REGISTER
    /// requests via the global event bus. The registrar service authenticates users
    /// and manages registrations.
    ///
    /// # Arguments
    /// * `realm` - The SIP realm for digest authentication (e.g., "example.com")
    /// * `users` - Map of username -> password for authentication
    ///
    /// # Returns
    ///
    /// `Arc<RegistrarService>` for inspecting and managing registrations.
    pub async fn start_registration_server(
        &self,
        realm: &str,
        users: std::collections::HashMap<String, String>,
    ) -> Result<Arc<rvoip_registrar_core::RegistrarService>> {
        use crate::adapters::RegistrationAdapter;
        use rvoip_registrar_core::{api::ServiceMode, types::RegistrarConfig, RegistrarService};

        tracing::info!(
            "🔐 Starting server-side registration handler with realm: {}",
            realm
        );

        // Create registrar service with authentication
        let registrar =
            RegistrarService::with_auth(ServiceMode::B2BUA, RegistrarConfig::default(), realm)
                .await
                .map_err(|e| {
                    SessionError::InternalError(format!("Failed to create registrar: {}", e))
                })?;

        // Add users to the registrar
        if let Some(user_store) = registrar.user_store() {
            for (username, password) in users {
                user_store.add_user(&username, &password).map_err(|e| {
                    SessionError::InternalError(format!("Failed to add user: {}", e))
                })?;
                tracing::debug!("Added user: {}", username);
            }
        }

        let registrar = Arc::new(registrar);

        // Get the global event coordinator
        let global_coordinator = rvoip_infra_common::events::global_coordinator()
            .await
            .clone();

        // Create and start the registration adapter
        let adapter = Arc::new(RegistrationAdapter::new(
            registrar.clone(),
            global_coordinator,
        ));

        adapter.start().await.map_err(|e| {
            SessionError::InternalError(format!("Failed to start registration adapter: {}", e))
        })?;

        tracing::info!("✅ Server-side registration handler started");

        Ok(registrar)
    }

    // ===== Internal Helpers =====

    async fn create_dialog_api(
        config: &Config,
        global_coordinator: Arc<GlobalEventCoordinator>,
    ) -> Result<Arc<rvoip_dialog_core::api::unified::UnifiedDialogApi>> {
        use rvoip_dialog_core::api::unified::UnifiedDialogApi;
        use rvoip_dialog_core::config::DialogManagerConfig;
        use rvoip_dialog_core::transaction::{
            transport::{TransportManager, TransportManagerConfig},
            TransactionManager,
        };

        // Create transport manager first (dialog-core's own transport manager).
        //
        // TCP is enabled by default — the URI-aware
        // `MultiplexedTransport` (`crates/dialog-core/src/transaction/transport/multiplexed.rs`)
        // routes outbound INVITEs to the right flavour based on the
        // Request-URI's scheme + `;transport=` parameter.
        //
        let effective_tls_mode = config.effective_tls_mode();
        let enable_tls = effective_tls_mode != SipTlsMode::Disabled;
        if config.tls_cert_path.is_some() ^ config.tls_key_path.is_some() {
            tracing::warn!(
                "session-core Config has tls_cert_path xor tls_key_path set; \
                 TLS listener roles require both"
            );
        }
        if matches!(
            effective_tls_mode,
            SipTlsMode::ServerOnly | SipTlsMode::ClientAndServer
        ) && (config.tls_cert_path.is_none() || config.tls_key_path.is_none())
        {
            return Err(SessionError::ConfigError(
                "SIP TLS listener modes require tls_cert_path and tls_key_path".to_string(),
            ));
        }
        if config.tls_client_cert_path.is_some() ^ config.tls_client_key_path.is_some() {
            return Err(SessionError::ConfigError(
                "TLS client certificate and key must be provided together".to_string(),
            ));
        }

        let tls_role = match effective_tls_mode {
            SipTlsMode::Disabled => {
                rvoip_dialog_core::transaction::transport::TlsRole::ClientAndServer
            }
            SipTlsMode::ClientOnly => {
                rvoip_dialog_core::transaction::transport::TlsRole::ClientOnly
            }
            SipTlsMode::ServerOnly => {
                rvoip_dialog_core::transaction::transport::TlsRole::ServerOnly
            }
            SipTlsMode::ClientAndServer => {
                rvoip_dialog_core::transaction::transport::TlsRole::ClientAndServer
            }
        };
        if matches!(effective_tls_mode, SipTlsMode::ClientOnly) {
            tracing::info!(
                "SIP TLS client-only mode enabled; no local endpoint certificate/key required"
            );
        }
        let transport_config = TransportManagerConfig {
            enable_udp: true,
            enable_tcp: true,
            enable_ws: false,
            enable_tls,
            tls_role,
            bind_addresses: vec![config.bind_addr],
            tls_bind_addresses: config.tls_bind_addr.into_iter().collect(),
            tls_cert_path: config
                .tls_cert_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            tls_key_path: config
                .tls_key_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            tls_client_cert_path: config
                .tls_client_cert_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            tls_client_key_path: config
                .tls_client_key_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            tls_extra_ca_path: config
                .tls_extra_ca_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            // Default build: `Config::tls_insecure_skip_verify` is not
            // compiled, so we always pass `false`. Only the
            // `dev-insecure-tls` build surfaces the field.
            #[cfg(feature = "dev-insecure-tls")]
            tls_insecure_skip_verify: config.tls_insecure_skip_verify,
            #[cfg(not(feature = "dev-insecure-tls"))]
            tls_insecure_skip_verify: false,
            ..Default::default()
        };

        let dialog_tls_local_address = config.tls_bind_addr.or_else(|| {
            if matches!(
                effective_tls_mode,
                SipTlsMode::ServerOnly | SipTlsMode::ClientAndServer
            ) {
                let mut tls_addr = config.bind_addr;
                if tls_addr.port() != 0 {
                    tls_addr.set_port(tls_addr.port().saturating_add(1));
                }
                Some(tls_addr)
            } else {
                None
            }
        });

        let (mut transport_manager, transport_event_rx) =
            TransportManager::new(transport_config).await.map_err(|e| {
                SessionError::InternalError(format!("Failed to create transport manager: {}", e))
            })?;

        // Initialize the transport manager
        transport_manager.initialize().await.map_err(|e| {
            SessionError::InternalError(format!("Failed to initialize transport: {}", e))
        })?;

        // Create transaction manager using transport manager
        let (transaction_manager, event_rx) = TransactionManager::with_transport_manager(
            transport_manager,
            transport_event_rx,
            None, // No max transactions limit
        )
        .await
        .map_err(|e| {
            SessionError::InternalError(format!("Failed to create transaction manager: {}", e))
        })?;

        let transaction_manager = Arc::new(transaction_manager);

        // Create dialog config - use hybrid mode to support both incoming and outgoing calls
        let dialog_config = DialogManagerConfig::hybrid(config.bind_addr)
            .with_from_uri(&config.local_uri)
            .with_auto_options()
            .with_100rel(config.use_100rel)
            .with_session_timer(config.session_timer_secs)
            .with_min_se(config.session_timer_min_se)
            .with_dialog_config(|mut dialog| {
                dialog.advertised_local_address = config.sip_advertised_addr;
                dialog.local_contact_uri = config.contact_uri.clone();
                dialog.tls_local_address = dialog_tls_local_address;
                dialog.tls_advertised_local_address = config.tls_advertised_addr;
                dialog
            })
            .build();

        // Create dialog API with global event coordination AND transaction events
        let dialog_api = Arc::new(
            UnifiedDialogApi::with_global_events_and_coordinator(
                transaction_manager,
                event_rx,
                dialog_config,
                global_coordinator.clone(),
            )
            .await
            .map_err(|e| {
                SessionError::InternalError(format!("Failed to create dialog API: {}", e))
            })?,
        );

        dialog_api.start().await.map_err(|e| {
            SessionError::InternalError(format!("Failed to start dialog API: {}", e))
        })?;

        Ok(dialog_api)
    }

    async fn create_media_controller(
        config: &Config,
        global_coordinator: Arc<GlobalEventCoordinator>,
    ) -> Result<Arc<rvoip_media_core::relay::controller::MediaSessionController>> {
        use rvoip_media_core::relay::controller::MediaSessionController;

        // Create media controller with port range
        let controller = Arc::new(MediaSessionController::with_port_range(
            config.media_port_start,
            config.media_port_end,
        ));

        // Create and set up the event hub
        let event_hub =
            rvoip_media_core::events::MediaEventHub::new(global_coordinator, controller.clone())
                .await
                .map_err(|e| {
                    SessionError::InternalError(format!("Failed to create media event hub: {}", e))
                })?;

        // Set the event hub on the media controller
        controller.set_event_hub(event_hub).await;

        Ok(controller)
    }
}

/// Simple helper to create a session and make a call
impl UnifiedCoordinator {
    /// Quick method to create a UAC session and make a call
    pub async fn quick_call(&self, from: &str, to: &str) -> Result<SessionId> {
        self.make_call(from, to).await
    }
}

/// Registration API
impl UnifiedCoordinator {
    /// Register with SIP server
    ///
    /// # Arguments
    /// * `registrar_uri` - URI of the registrar server (e.g., "sip:registrar.example.com")
    /// * `from_uri` - From URI (e.g., "sip:user@example.com")
    /// * `contact_uri` - Contact URI (e.g., "sip:user@192.168.1.100:5060")
    /// * `username` - Username for authentication
    /// * `password` - Password for digest authentication
    /// * `expires` - Registration expiry in seconds (typically 3600)
    ///
    /// # Returns
    /// A `RegistrationHandle` that can be used to unregister or refresh
    pub async fn register(
        &self,
        registrar_uri: &str,
        from_uri: &str,
        contact_uri: &str,
        username: &str,
        password: &str,
        expires: u32,
    ) -> Result<RegistrationHandle> {
        // Create registration session
        let session_id = SessionId::new();
        tracing::info!("📝 Created registration session: {}", session_id.0);
        self.helpers
            .create_session(
                session_id.clone(),
                from_uri.to_string(),
                registrar_uri.to_string(),
                crate::state_table::types::Role::UAC,
            )
            .await?;

        // Store credentials
        let credentials = crate::types::Credentials::new(username, password);

        // Get session store and update
        let session_store = &self.helpers.state_machine.store;
        let mut session = session_store.get_session(&session_id).await?;
        session.credentials = Some(credentials);
        session.registrar_uri = Some(registrar_uri.to_string());
        session.registration_contact = Some(contact_uri.to_string());
        session.registration_expires = Some(expires);
        session_store.update_session(session).await?;

        // Trigger registration via state machine
        let _result = self
            .helpers
            .state_machine
            .process_event(
                &session_id,
                crate::state_table::types::EventType::StartRegistration,
            )
            .await
            .map_err(|e| {
                SessionError::InternalError(format!("Failed to trigger registration: {}", e))
            })?;

        Ok(RegistrationHandle { session_id })
    }

    /// Register with a SIP server using a [`Registration`] builder.
    ///
    /// This is the preferred way to register — `from_uri` and `contact_uri`
    /// default to the peer's `local_uri` from [`Config`].
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_session_core::UnifiedCoordinator>) -> rvoip_session_core::Result<()> {
    /// use rvoip_session_core::Registration;
    ///
    /// let handle = coordinator.register_with(
    ///     Registration::new("sip:registrar.example.com", "alice", "secret123")
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn register_with(&self, reg: Registration) -> Result<RegistrationHandle> {
        let from_uri = reg.from_uri.as_deref().unwrap_or(&self.config.local_uri);
        let contact_uri = reg
            .contact_uri
            .as_deref()
            .or(self.config.contact_uri.as_deref())
            .unwrap_or(&self.config.local_uri);
        self.register(
            &reg.registrar,
            from_uri,
            contact_uri,
            &reg.username,
            &reg.password,
            reg.expires,
        )
        .await
    }

    /// Unregister from SIP server
    ///
    /// Sends REGISTER with expires=0 to remove registration
    pub async fn unregister(&self, handle: &RegistrationHandle) -> Result<()> {
        // Trigger unregistration via state machine
        let result = self
            .helpers
            .state_machine
            .process_event(
                &handle.session_id,
                crate::state_table::types::EventType::StartUnregistration,
            )
            .await
            .map_err(|e| {
                SessionError::InternalError(format!("Failed to trigger unregistration: {}", e))
            })?;
        if result.transition.is_none() {
            return Err(SessionError::InvalidTransition(format!(
                "Cannot unregister session {} from state {:?}",
                handle.session_id.0, result.old_state
            )));
        }
        if !result
            .actions_executed
            .iter()
            .any(|action| matches!(action, Action::SendUnREGISTER))
        {
            return Err(SessionError::InternalError(format!(
                "Unregistration transition for session {} did not send REGISTER Expires: 0",
                handle.session_id.0
            )));
        }
        Ok(())
    }

    /// Refresh registration before it expires
    ///
    /// Sends a new REGISTER request with the same expiry time
    pub async fn refresh_registration(&self, handle: &RegistrationHandle) -> Result<()> {
        // Trigger refresh via state machine
        let _result = self
            .helpers
            .state_machine
            .process_event(
                &handle.session_id,
                crate::state_table::types::EventType::RefreshRegistration,
            )
            .await
            .map_err(|e| {
                SessionError::InternalError(format!("Failed to trigger refresh: {}", e))
            })?;
        Ok(())
    }

    /// Get registration status
    pub async fn is_registered(&self, handle: &RegistrationHandle) -> Result<bool> {
        let session = self
            .helpers
            .state_machine
            .store
            .get_session(&handle.session_id)
            .await?;
        tracing::info!(
            "🔍 Checking registration for session {}: is_registered={}, retry_count={}",
            handle.session_id.0,
            session.is_registered,
            session.registration_retry_count
        );
        Ok(session.is_registered)
    }
}

/// Handle for managing a registration
#[derive(Debug, Clone)]
pub struct RegistrationHandle {
    pub session_id: SessionId,
}

/// Configuration for SIP registration.
///
/// Use [`Registration::new()`] for the common case where `from_uri` and
/// `contact_uri` are derived from the peer's [`Config`].
///
/// # Example
///
/// ```
/// use rvoip_session_core::Registration;
///
/// let reg = Registration::new("sip:registrar.example.com", "alice", "secret123")
///     .expires(1800);
/// ```
#[derive(Debug, Clone)]
pub struct Registration {
    /// SIP URI of the registrar server (e.g. `sip:registrar.example.com`)
    pub registrar: String,
    /// Username for digest authentication
    pub username: String,
    /// Password for digest authentication
    pub password: String,
    /// Registration expiry in seconds (default: 3600)
    pub expires: u32,
    /// Override the From URI (defaults to the peer's local_uri)
    pub from_uri: Option<String>,
    /// Override the Contact URI (defaults to the peer's local_uri)
    pub contact_uri: Option<String>,
}

impl Registration {
    /// Create a registration with the minimum required fields.
    ///
    /// `from_uri` and `contact_uri` will be derived from the peer's config.
    pub fn new(
        registrar: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self {
            registrar: registrar.into(),
            username: username.into(),
            password: password.into(),
            expires: 3600,
            from_uri: None,
            contact_uri: None,
        }
    }

    /// Set the registration expiry in seconds (default: 3600).
    pub fn expires(mut self, secs: u32) -> Self {
        self.expires = secs;
        self
    }

    /// Override the From URI (defaults to the peer's local_uri).
    pub fn from_uri(mut self, uri: impl Into<String>) -> Self {
        self.from_uri = Some(uri.into());
        self
    }

    /// Override the Contact URI (defaults to the peer's local_uri).
    pub fn contact_uri(mut self, uri: impl Into<String>) -> Self {
        self.contact_uri = Some(uri.into());
        self
    }
}

/// Sprint 3 A6 — best-effort STUN probe for the RTP-side public
/// mapping.
///
/// **Caveat.** The probe binds a fresh ephemeral UDP socket on the
/// configured `local_ip` and asks the STUN server what mapping it
/// sees. For typical cone NATs (most consumer routers, AWS / GCP
/// NAT gateways) the mapping is keyed by source IP only, so the
/// discovered address matches what the actual RTP path will see
/// later. For symmetric NATs the mapping is per-(source IP, source
/// port) and the result will be wrong — those deployments need ICE
/// (Sprint 4 D3). For Sprint 3 the simple shape is the right
/// trade-off; a deployment that breaks here can fall back to
/// `Config::media_public_addr` (static override).
async fn run_stun_probe(adapter: Arc<MediaAdapter>, stun_target: &str) -> Result<()> {
    use std::sync::Arc as StdArc;
    use tokio::net::UdpSocket as TokioUdpSocket;

    // Normalise "host" → "host:3478"; "host:port" passes through.
    let target_str = if stun_target.contains(':') {
        stun_target.to_string()
    } else {
        format!("{}:3478", stun_target)
    };

    // Resolve via tokio's DNS — STUN servers are typically fronted by
    // SRV in production but the public ones (Google, Cloudflare) all
    // expose plain A records.
    let server_addr = tokio::net::lookup_host(&target_str)
        .await
        .map_err(|e| {
            SessionError::ConfigError(format!("STUN resolve '{}' failed: {}", target_str, e))
        })?
        .next()
        .ok_or_else(|| {
            SessionError::ConfigError(format!("STUN '{}' resolved to nothing", target_str))
        })?;

    // Bind a probe socket on the same interface as the SIP/media
    // bind. Random ephemeral port; the cone-NAT-mapping caveat above
    // applies.
    let bind_local = std::net::SocketAddr::new(adapter.local_ip(), 0);
    let probe_sock = TokioUdpSocket::bind(bind_local).await.map_err(|e| {
        SessionError::ConfigError(format!("STUN probe bind {} failed: {}", bind_local, e))
    })?;
    let probe_sock = StdArc::new(probe_sock);

    let client = rvoip_rtp_core::network::stun::StunClient::new(probe_sock, server_addr);
    let discovered = client
        .discover()
        .await
        .map_err(|e| SessionError::ConfigError(format!("STUN probe failed: {}", e)))?;

    tracing::info!(
        "RTP public addr: {} (STUN-discovered via {})",
        discovered,
        target_str
    );
    adapter.set_public_rtp_addr(Some(discovered));
    Ok(())
}
