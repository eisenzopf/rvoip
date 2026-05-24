//! Lower-level session orchestration API.
//!
//! [`UnifiedCoordinator`] is the shared engine underneath [`StreamPeer`] and
//! [`CallbackPeer`]. It exposes explicit [`SessionId`] values and direct
//! methods for call creation, incoming-call resolution, registration
//! lifecycle management, event subscription, transfer primitives, audio
//! bridging, and media control.
//!
//! Use this module directly when you are building an application framework on
//! top of `rvoip-sip`: B2BUA logic, gateways, carrier-facing services,
//! custom peer abstractions, or multi-leg call orchestration. It is also the
//! surface that exposes deterministic registration shutdown and metadata such
//! as registrar-accepted expiry, refresh timing, Service-Route, and GRUU. For
//! ordinary client/test code, [`StreamPeer`] is usually more ergonomic. For
//! reactive server endpoints, [`CallbackPeer`] is usually the better starting
//! point.
//!
//! Outbound calls flow through one builder, [`UnifiedCoordinator::invite`],
//! with chainable modifiers — `.with_credentials(...)` for per-call digest
//! auth, `.with_pai(...)` for per-call `P-Asserted-Identity`, and
//! `.with_extra_headers(...)` for caller-supplied typed headers on the
//! first INVITE. Terminate the chain with `.send()`.
//!
//! # Example
//!
//! ```rust,no_run
//! use rvoip_sip::{Config, Event, Result, UnifiedCoordinator};
//!
//! # async fn example() -> Result<()> {
//! let coordinator = UnifiedCoordinator::new(Config::local("app", 5060)).await?;
//! let mut events = coordinator.events().await?;
//!
//! let call_id = coordinator
//!     .invite(Some("sip:app@127.0.0.1:5060".to_string()), "sip:bob@127.0.0.1:5070")
//!     .send()
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

#![deny(missing_docs)]

use crate::adapters::{DialogAdapter, MediaAdapter};
use crate::api::lifecycle::{CallLifecycleSnapshot, LifecycleIndex, SessionEventPublisher};
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
use rvoip_rtp_core::transport::{
    DEFAULT_RTP_PORT_RANGE_END, DEFAULT_RTP_PORT_RANGE_START, MIN_PORT,
};
use rvoip_sip_core::types::sdp::CryptoSuite;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};

pub use rvoip_media_core::relay::controller::{AudioSource, BridgeError, BridgeHandle};
pub use rvoip_sip_dialog::api::RelUsage;

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

/// Named SRTP suite offer policies for common PBX/carrier interop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SrtpSuitePolicy {
    /// Conservative default: AES-128 CM suites, strongest auth tag first.
    Default,
    /// FreeSWITCH-compatible SDES policy: offer AES-256 CM and AES-128 CM
    /// suites in a deliberate preference order while avoiding AEAD-GCM until
    /// rtp-core supports it end to end.
    FreeSwitchCompatible,
}

impl SrtpSuitePolicy {
    /// Suites to advertise for this policy, in local preference order.
    pub fn suites(self) -> Vec<CryptoSuite> {
        match self {
            Self::Default => vec![
                CryptoSuite::AesCm128HmacSha1_80,
                CryptoSuite::AesCm128HmacSha1_32,
            ],
            Self::FreeSwitchCompatible => vec![
                CryptoSuite::AesCm256HmacSha1_80,
                CryptoSuite::AesCm128HmacSha1_80,
                CryptoSuite::AesCm256HmacSha1_32,
                CryptoSuite::AesCm128HmacSha1_32,
            ],
        }
    }
}

/// Media allocation behavior for SIP sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaMode {
    /// Allocate media-core RTP sessions and RTP ports normally.
    Enabled,
    /// Skip media-core RTP allocation while still emitting SDP.
    ///
    /// The configured `sdp_rtp_port` is advertised unless
    /// [`Config::media_public_addr`] carries a nonzero port, in which case the
    /// explicit public media port is advertised.
    SignalingOnly {
        /// RTP port to advertise in SDP when no public media port override is set.
        sdp_rtp_port: u16,
    },
}

/// Runtime configuration for [`UnifiedCoordinator`].
///
/// `Config` controls SIP and media binding, advertised addresses, TLS,
/// registration Contact behavior, registration refresh/unregister policy, SRTP
/// policy, session timers, reliable provisionals, caller identity headers,
/// outbound proxy routing for INVITEs and REGISTERs, NAT/media address
/// discovery, and codec negotiation.
///
/// Start with [`Config::local`] for loopback examples, [`Config::on`] for a
/// specific LAN/host address, then adjust the feature-specific fields for the
/// deployment profile. The profile constructors are conservative starting
/// points for common interop targets; they do not imply carrier certification
/// or full RFC 5626 multi-flow behavior.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip::{Config, SipContactMode, SipTlsMode};
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
    /// Requested media port range capacity when configured by capacity.
    ///
    /// `None` means the explicit start/end range is authoritative. When set,
    /// validation checks that the configured range can satisfy the requested
    /// number of RTP ports.
    pub media_port_capacity: Option<usize>,
    /// Bind address for SIP
    pub bind_addr: SocketAddr,
    /// Optional advertised address for SIP Via sent-by and fallback Contact
    /// generation. This is distinct from [`Config::bind_addr`]: bind can be
    /// `0.0.0.0`, while the advertised address must be routable by peers.
    pub sip_advertised_addr: Option<SocketAddr>,
    /// Optional path to custom state table YAML
    /// Priority: 1) this config path, 2) embedded default
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

    /// Whether the default inbound INVITE path sends automatic `180 Ringing`.
    ///
    /// Default: `true`, which is appropriate for PBX-style ringing behavior
    /// and preserves previous API behavior. Set to `false` for IVR,
    /// contact-center, and auto-answer services that immediately send a final
    /// response and do not need the extra provisional response.
    pub auto_180_ringing: bool,

    /// Whether INVITE server transactions arm the automatic RFC 3261
    /// `100 Trying` timer.
    ///
    /// Default: `true`, which preserves RFC-friendly behavior for ordinary
    /// endpoints. Set to `false` only for fixed fast-answer services that
    /// immediately send a final response and do not need one timer task per
    /// inbound INVITE.
    pub auto_100_trying: bool,

    /// Whether inbound INVITEs are accepted immediately in the session event
    /// path before application callback dispatch.
    ///
    /// Default: `false`, preserving normal app-controlled accept/reject/defer
    /// behavior. Set to `true` only for fixed auto-answer services and
    /// high-CPS benchmarks where the app callback must not sit on the first
    /// final response path.
    pub fast_auto_accept_incoming_calls: bool,

    /// RFC 4028 `Session-Expires` value in seconds to advertise on outgoing
    /// INVITEs. `None` disables session timers entirely. Common carrier
    /// value is 1800 (30 min).
    pub session_timer_secs: Option<u32>,

    /// Minimum-session-expires (`Min-SE:`) we're willing to accept, in
    /// seconds. Default 90 per RFC 4028 §5.
    pub session_timer_min_se: u32,

    /// Default credentials to apply to every outgoing call for RFC 3261 §22.2
    /// INVITE digest auth retry. When the server responds 401/407 to our
    /// INVITE, rvoip-sip looks here (or at per-call credentials passed
    /// via `control.invite(...).with_credentials(...)`) to compute the digest response. When
    /// `None`, a 401/407 on INVITE surfaces as `CallFailed` instead of
    /// retrying. Default: `None`.
    pub credentials: Option<crate::types::Credentials>,

    /// Default `P-Asserted-Identity` URI (RFC 3325 §9.1) to attach to every
    /// outgoing INVITE. Carrier trunks (Twilio, Vonage, Bandwidth, most PBX
    /// trunks) require PAI for caller-ID assertion on outbound trunk calls;
    /// without it the call is often hard-rejected or stripped of caller ID.
    /// `None` (the default) suppresses the header entirely. Per-call override
    /// is available via [`UnifiedCoordinator::invite`] + `.with_pai(...)`.
    pub pai_uri: Option<String>,

    /// Optional SIP transport-boundary tracing for diagnostics.
    pub sip_trace: crate::api::events::SipTraceConfig,

    /// SIP_API_DESIGN_2 §12.4 — pluggable trace-output redaction. When
    /// set, every header value passing through the trace sink is run
    /// through the redactor before logging; the wire form is
    /// unaffected. `None` (the default) keeps the legacy
    /// log-verbatim behaviour. See
    /// [`crate::api::trace_redactor::TraceRedactor`] for the policy
    /// hook contract.
    pub trace_redaction: Option<std::sync::Arc<dyn crate::api::trace_redactor::TraceRedactor>>,

    /// Outbound proxy URI (RFC 3261 §8.1.2). When set, a `Route:
    /// <outbound-proxy-uri;lr>` header is pre-loaded as the first Route on
    /// every outgoing INVITE this UA originates, forcing the dialog-
    /// initiating request through the specified proxy. Typical values:
    /// `sip:sbc.example.com;lr`, `sips:sbc.example.com:5061;lr`.
    ///
    /// The URI should carry the `;lr` parameter to signal a loose-routing
    /// proxy (RFC 3261 §16.12.1.1). rvoip-sip does **not** auto-add `;lr`
    /// — set it explicitly in the URI string.
    ///
    /// Applied to outgoing INVITEs and REGISTERs. For REGISTER, the registrar
    /// remains the SIP Request-URI while the network destination is the
    /// outbound proxy and a loose Route header is included. `None` (the
    /// default) suppresses the header entirely. Per-request override is not
    /// yet exposed.
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
    /// flow. Flow failure is surfaced back into rvoip-sip so the
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

    /// Automatically refresh successful registrations before they expire.
    ///
    /// When enabled, rvoip-sip schedules a re-REGISTER after a successful
    /// REGISTER 2xx using the registrar-accepted expiry. Default: `true`.
    pub registration_auto_refresh: bool,

    /// Maximum percentage of the refresh interval to subtract as jitter.
    ///
    /// The base refresh interval is 85% of the accepted expiry. Jitter is
    /// applied earlier, never later, so a value of 5 means the refresh fires
    /// between 80.75% and 85% of the accepted expiry. Default: `5`.
    pub registration_refresh_jitter_percent: u8,

    /// Timeout for best-effort unregister during graceful shutdown.
    ///
    /// `0` disables unregister-on-shutdown. Default: `3` seconds.
    pub unregister_on_shutdown_timeout_secs: u64,

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
    pub srtp_offered_suites: Vec<CryptoSuite>,

    /// Override the RTP-side public address advertised in SDP `c=` /
    /// `o=` and `m=audio <port>` lines. Use when:
    ///
    /// - The rvoip-sip process runs behind a 1:1 NAT or IP alias
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

    /// Media allocation behavior.
    ///
    /// Default: [`MediaMode::Enabled`], which allocates real media-core RTP
    /// sessions and RTP ports. [`MediaMode::SignalingOnly`] skips media-core
    /// RTP allocation but still emits SDP; useful for signaling-only services
    /// and controlled tests.
    pub media_mode: MediaMode,

    /// Optional capacity hint for media-core session and RTP port indexes.
    ///
    /// This is intentionally separate from [`Config::server_call_capacity`]:
    /// high-CPS media servers may want RTP/media preallocation without
    /// inflating SIP dialog and transaction indexes.
    pub media_session_capacity: Option<usize>,

    /// STUN server (RFC 8489 §14) to probe for the RTP-side public
    /// mapping at coordinator boot. Format: `"host:port"` or `"host"`
    /// (default port 3478). Common public servers:
    /// `stun.l.google.com:19302`, `stun.cloudflare.com:3478`.
    ///
    /// The probe runs once at startup using a fresh UDP socket bound to
    /// [`Config::local_ip`]. This is best-effort address discovery: it is
    /// useful for simple cone-NAT labs, but it does not guarantee the exact
    /// mapping of a later per-call RTP socket. Symmetric NATs and production
    /// Internet edges should use a static [`Config::media_public_addr`] today
    /// or ICE in a future WebRTC/edge layer. Failure mode: probe timeout /
    /// unreachable / unparseable response → log a warning and fall back to
    /// the local interface address. STUN is intentionally soft-fail — the
    /// call path is never blocked on it.
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

    /// NEXT_STEPS C2 — RTP payload types this UA advertises in
    /// outgoing offers and accepts in answers. Default `[0, 8, 101]`
    /// (PCMU + PCMA + telephone-event) preserves the pre-C2 wire
    /// shape. Add `111` to advertise Opus (`opus/48000/2`); `9` for
    /// G.722; `18` for G.729. CN (PT 13) is folded in automatically
    /// when `comfort_noise_enabled = true`.
    ///
    /// Note: the rvoip-sip SDP builder will advertise any PT listed
    /// here, but actually encoding/decoding the codec requires
    /// media-core to be built with the matching feature flag
    /// (e.g. `--features opus`). Advertising a codec that media-core
    /// can't process surfaces as a negotiated session that drops
    /// audio rather than a build-time error.
    ///
    /// Default: `vec![0, 8, 101]`.
    pub offered_codecs: Vec<u8>,

    /// Capacity for the legacy incoming-call compatibility channel.
    ///
    /// Modern [`CallbackPeer`](crate::api::callback_peer::CallbackPeer) and
    /// [`StreamPeer`](crate::api::stream_peer::StreamPeer) consumers receive
    /// incoming calls through the app event publisher. The compatibility
    /// receiver exposed by [`UnifiedCoordinator::next_incoming_call`] is still
    /// kept for lower-level callers, so the buffer must be large enough for
    /// bursts without becoming a hidden backpressure point in the dialog event
    /// handler. Default: `1000`.
    pub incoming_call_channel_capacity: usize,

    /// Capacity for the internal state-machine event channel.
    ///
    /// State transitions publish lightweight internal events that the session
    /// event handler consumes and maps onto public API events where needed.
    /// This buffer must absorb short bursts so SIP request processing does not
    /// block behind event fan-out during load tests. Default: `1000`.
    pub state_event_channel_capacity: usize,

    /// Capacity for SIP transport event channels.
    ///
    /// This controls the per-transport receive queue (UDP/TCP/WS) and the
    /// combined transport-manager queue feeding the transaction layer. It is
    /// intentionally larger than the app-facing queues because one call setup
    /// produces multiple SIP messages and retransmission bursts can otherwise
    /// backpressure the UDP receive loop. Default: `10000`.
    pub sip_transport_channel_capacity: usize,

    /// Optional SIP transport-manager forwarding worker count.
    ///
    /// `None` preserves the single per-transport event bridge. Values above
    /// `1` enable keyed sharding between transport receive/parse and
    /// transaction-manager ingress.
    pub sip_transport_dispatch_workers: Option<usize>,

    /// Optional SIP transport-manager forwarding queue capacity.
    ///
    /// `None` uses [`Config::sip_transport_channel_capacity`]. When dispatch
    /// workers are enabled, this capacity is divided across workers.
    pub sip_transport_dispatch_queue_capacity: Option<usize>,

    /// Optional SIP UDP receive socket buffer size (`SO_RCVBUF`) in bytes.
    ///
    /// `None` preserves the OS default, which is appropriate for clients and
    /// small servers. High-CPS server profiles should set this alongside the
    /// transport channel capacity so kernel UDP bursts do not overflow before
    /// the async receive loop can drain them.
    pub sip_udp_recv_buffer_size: Option<usize>,

    /// Optional SIP UDP send socket buffer size (`SO_SNDBUF`) in bytes.
    ///
    /// `None` preserves the OS default. Server deployments with large reply
    /// bursts can set this to match the receive-side sizing policy.
    pub sip_udp_send_buffer_size: Option<usize>,

    /// Optional UDP parse worker count for the SIP UDP receive path.
    ///
    /// `None` keeps the transport default. High-CPS UDP servers can set this
    /// when parsing/dispatch work behind the socket receive loop needs more
    /// parallelism.
    pub sip_udp_parse_workers: Option<usize>,

    /// Optional per-worker UDP parse queue capacity.
    ///
    /// `None` uses the SIP transport channel capacity. When set, this bounds
    /// how many datagrams each UDP parse worker may buffer before overload is
    /// counted and dropped explicitly.
    pub sip_udp_parse_queue_capacity: Option<usize>,

    /// Optional UDP parse worker dispatch strategy.
    ///
    /// `None` preserves the transport default (`SourceHash`). High-CPS perf
    /// tests can opt into `RoundRobin` when the traffic generator sends all
    /// calls from a single source socket and source hashing cannot fan out.
    pub sip_udp_parse_dispatch: Option<rvoip_sip_transport::UdpParseDispatch>,

    /// Capacity for the transaction-manager event channel consumed by dialog
    /// core.
    ///
    /// A small transaction event queue can block transaction processing while
    /// dialog/session cleanup catches up. Default: `10000`.
    pub transaction_event_channel_capacity: usize,

    /// Optional transaction-manager ingress worker count.
    ///
    /// `None` preserves the single receive/handle loop used by clients and
    /// ordinary endpoints. High-CPS servers can set this above `1` to fan out
    /// transaction handling by a stable call/transaction key while preserving
    /// per-call request ordering.
    pub sip_transaction_dispatch_workers: Option<usize>,

    /// Optional transaction-manager ingress queue capacity.
    ///
    /// `None` uses [`Config::transaction_event_channel_capacity`]. When
    /// dispatch workers are enabled, this capacity is divided across workers.
    pub sip_transaction_dispatch_queue_capacity: Option<usize>,

    /// Optional priority-lane burst limit for transaction ingress workers.
    ///
    /// This applies only when [`Config::sip_transaction_dispatch_workers`] is
    /// greater than `1`. The transaction dispatcher lets ACK and BYE requests
    /// jump ahead of older normal-lane work on their assigned worker, then
    /// gives one ready normal item a turn after this many consecutive priority
    /// items. `None` uses the transaction-layer default (`64`). Lower this
    /// when INVITE/CANCEL/response work must make progress during teardown
    /// storms; raise it when ACK/BYE latency is the dominant failure mode and
    /// normal-lane delay is acceptable.
    pub sip_transaction_dispatch_priority_burst_max: Option<usize>,

    /// Optional cached INVITE `2xx` retransmission maintenance budget.
    ///
    /// The transaction manager keeps a short-lived cache of successful INVITE
    /// responses so duplicate INVITEs can be answered without rebuilding the
    /// SIP response. This limit controls how many cached `2xx` responses the
    /// proactive maintenance task may retransmit per 100 ms tick. `None` uses
    /// the transaction-layer default (`2048`). Lower this to pace retransmit
    /// storms when UDP send pressure starves teardown work; raise it when the
    /// host send path has headroom and UAC timeout/dead-call volume is driven
    /// by uncleared INVITE `2xx` loss bursts.
    pub sip_invite_2xx_retransmit_max_due_per_tick: Option<usize>,

    /// Optional dialog-core transaction-event dispatch worker count.
    ///
    /// `None` preserves the single dialog event processor. High-CPS servers can
    /// set this above `1` to fan out transaction events by stable call key while
    /// preserving per-call dialog ordering.
    pub sip_dialog_dispatch_workers: Option<usize>,

    /// Optional dialog-core transaction-event dispatch queue capacity.
    ///
    /// `None` uses the dialog max-dialog capacity hint. When dispatch workers
    /// are enabled, this capacity is divided across workers.
    pub sip_dialog_dispatch_queue_capacity: Option<usize>,

    /// Capacity for the infra-common global cross-crate event bus used inside
    /// this coordinator.
    ///
    /// ACK, BYE, media, and app-session events all cross this bus before they
    /// reach their local consumers. High-CPS server profiles should size it
    /// with the other signaling queues so the event bridge does not drop
    /// cleanup-driving events. Default: `10000`.
    pub global_event_channel_capacity: usize,

    /// Number of async workers used to publish app-level session events onto
    /// the global infra-common event bus.
    ///
    /// Internal per-call waits use the lifecycle index directly, but public
    /// event subscribers still receive events through the global bus. This
    /// worker pool avoids spawning one task per non-terminal event under
    /// server load. Default: logical CPU count capped at 16.
    pub session_event_dispatcher_workers: usize,

    /// Per-worker queue capacity for app-level session event publication.
    ///
    /// This queue sits in front of the global event coordinator only for
    /// non-terminal fire-and-forget publishes. Terminal publishes still use
    /// the synchronous `publish_now` path so session cleanup happens after
    /// the terminal event is visible. Default: `10000`.
    pub session_event_dispatcher_channel_capacity: usize,

    /// Expected server-side active call capacity for hot lookup indexes.
    ///
    /// `None` keeps client/small-endpoint behavior lazy and uses existing
    /// channel-derived defaults. Server/high-CPS profiles should set this to
    /// the expected active-call burst size so dialog, transaction, session,
    /// lifecycle, and media indexes can reserve capacity up front without
    /// tying that memory reservation to the larger event-queue capacities.
    pub server_call_capacity: Option<usize>,

    /// Enable SIP UDP transport and duplicate-recovery diagnostics.
    ///
    /// This is a Config-owned replacement for benchmark-only diagnostic env
    /// toggles. It enables UDP receive/send counters and SIP duplicate
    /// INVITE/BYE cache counters for this process.
    pub sip_udp_diagnostics: bool,

    /// Enable high-cardinality transaction timing diagnostics.
    ///
    /// This records per-message transaction dispatch, handler, transaction
    /// creation, existing-transaction dispatch, and event-send histograms. It
    /// is intentionally separate from [`Config::sip_udp_diagnostics`] because
    /// it adds hot-path timestamp and atomic work under high CPS.
    pub sip_transaction_timing_diagnostics: bool,

    /// Enable high-cardinality dialog timing diagnostics.
    ///
    /// This records transaction-event-to-dialog queueing, dialog handler,
    /// dialog lookup, and dialog-to-session publish histograms. It is separate
    /// from transaction timing so 20k CPS tests can isolate the current hot
    /// layer.
    pub sip_dialog_timing_diagnostics: bool,

    /// Enable media setup/teardown timing diagnostics.
    ///
    /// This records media start/stop, RTP port allocation, RTP session
    /// creation, event subscription, and handler-spawn timing.
    pub media_setup_diagnostics: bool,

    /// Enable cleanup-stage timing diagnostics.
    ///
    /// This records cleanup and high-rate call-progress subpath counters used
    /// by the perf listener and high-CPS investigations.
    pub cleanup_diagnostics: bool,

    /// Enable per-operation cleanup diagnostic event logs.
    ///
    /// This is intentionally separate from [`Config::cleanup_diagnostics`]
    /// because it emits one log line per measured operation and is much more
    /// expensive under load.
    pub cleanup_diagnostic_events: bool,

    /// Enable SRTP negotiation diagnostic log lines.
    pub srtp_diagnostics: bool,

    /// Enable RTP packet diagnostic log lines.
    pub rtp_diagnostics: bool,

    /// Enable SDP media diagnostic log lines.
    pub media_sdp_diagnostics: bool,

    /// SIP_API_DESIGN_2 §7.4 — application-supplied headers stamped
    /// on every outbound message the state machine emits
    /// **automatically** (session-timer auto-BYE, dialog-terminated-
    /// during-INVITE auto-CANCEL, REFER-completion auto-NOTIFY).
    ///
    /// Stack-managed names (`Call-ID`, `CSeq`, `Via`, `Max-Forwards`,
    /// `Content-Length`, `Record-Route`) are rejected at
    /// [`Config::validate`] time. Method-shaped names that have a
    /// dedicated builder setter (e.g. `Authorization`) are accepted
    /// here — auto-emit messages have no per-call builder to route
    /// through.
    ///
    /// Applies to auto-emitted messages only; application-initiated
    /// builders inherit `Config` defaults through the §6.1 merge
    /// table, not through this field. The §7.4 precedence rule is:
    /// the state machine's auto-emit handler checks
    /// `pending_<method>_options` stash first; if populated those
    /// win and `auto_emit_extra_headers` is **not** appended.
    pub auto_emit_extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
}

impl Config {
    /// Default RTP media port range start.
    pub const DEFAULT_MEDIA_PORT_START: u16 = DEFAULT_RTP_PORT_RANGE_START;

    /// Default RTP media port range end.
    pub const DEFAULT_MEDIA_PORT_END: u16 = DEFAULT_RTP_PORT_RANGE_END;

    /// Create a config for local development/testing on 127.0.0.1.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip::Config;
    /// let config = Config::local("alice", 5060);
    /// assert_eq!(config.local_uri, "sip:alice@127.0.0.1:5060");
    /// ```
    pub fn local(name: &str, port: u16) -> Self {
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        Self {
            local_ip: ip,
            sip_port: port,
            media_port_start: Self::DEFAULT_MEDIA_PORT_START,
            media_port_end: Self::DEFAULT_MEDIA_PORT_END,
            media_port_capacity: None,
            bind_addr: SocketAddr::new(ip, port),
            sip_advertised_addr: None,
            state_table_path: None,
            local_uri: format!("sip:{}@{}:{}", name, ip, port),
            use_100rel: RelUsage::default(),
            auto_180_ringing: true,
            auto_100_trying: true,
            fast_auto_accept_incoming_calls: false,
            session_timer_secs: None,
            session_timer_min_se: 90,
            credentials: None,
            pai_uri: None,
            sip_trace: crate::api::events::SipTraceConfig::default(),
            trace_redaction: None,
            outbound_proxy_uri: None,
            sip_outbound_enabled: false,
            sip_instance: None,
            outbound_keepalive_interval_secs: 25,
            registration_auto_refresh: true,
            registration_refresh_jitter_percent: 5,
            unregister_on_shutdown_timeout_secs: 3,
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
            srtp_offered_suites: SrtpSuitePolicy::Default.suites(),
            media_public_addr: None,
            media_mode: MediaMode::Enabled,
            media_session_capacity: None,
            stun_server: None,
            comfort_noise_enabled: false,
            strict_codec_matching: true,
            offered_codecs: vec![0, 8, 101],
            incoming_call_channel_capacity: 1000,
            state_event_channel_capacity: 1000,
            sip_transport_channel_capacity: 10_000,
            sip_transport_dispatch_workers: None,
            sip_transport_dispatch_queue_capacity: None,
            sip_udp_recv_buffer_size: None,
            sip_udp_send_buffer_size: None,
            sip_udp_parse_workers: None,
            sip_udp_parse_queue_capacity: None,
            sip_udp_parse_dispatch: None,
            transaction_event_channel_capacity: 10_000,
            sip_transaction_dispatch_workers: None,
            sip_transaction_dispatch_queue_capacity: None,
            sip_transaction_dispatch_priority_burst_max: None,
            sip_invite_2xx_retransmit_max_due_per_tick: None,
            sip_dialog_dispatch_workers: None,
            sip_dialog_dispatch_queue_capacity: None,
            global_event_channel_capacity: 10_000,
            session_event_dispatcher_workers: default_session_event_dispatcher_workers(),
            session_event_dispatcher_channel_capacity: 10_000,
            server_call_capacity: None,
            sip_udp_diagnostics: false,
            sip_transaction_timing_diagnostics: false,
            sip_dialog_timing_diagnostics: false,
            media_setup_diagnostics: false,
            cleanup_diagnostics: false,
            cleanup_diagnostic_events: false,
            srtp_diagnostics: false,
            rtp_diagnostics: false,
            media_sdp_diagnostics: false,
            auto_emit_extra_headers: Vec::new(),
        }
    }

    /// Create a config bound to a specific IP address (e.g. for LAN or production).
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip::Config;
    /// let config = Config::on("alice", "192.168.1.50".parse().unwrap(), 5060);
    /// assert_eq!(config.local_uri, "sip:alice@192.168.1.50:5060");
    /// ```
    pub fn on(name: &str, ip: IpAddr, port: u16) -> Self {
        Self {
            local_ip: ip,
            sip_port: port,
            media_port_start: Self::DEFAULT_MEDIA_PORT_START,
            media_port_end: Self::DEFAULT_MEDIA_PORT_END,
            media_port_capacity: None,
            bind_addr: SocketAddr::new(ip, port),
            sip_advertised_addr: None,
            state_table_path: None,
            local_uri: format!("sip:{}@{}:{}", name, ip, port),
            use_100rel: RelUsage::default(),
            auto_180_ringing: true,
            auto_100_trying: true,
            fast_auto_accept_incoming_calls: false,
            session_timer_secs: None,
            session_timer_min_se: 90,
            credentials: None,
            pai_uri: None,
            sip_trace: crate::api::events::SipTraceConfig::default(),
            trace_redaction: None,
            outbound_proxy_uri: None,
            sip_outbound_enabled: false,
            sip_instance: None,
            outbound_keepalive_interval_secs: 25,
            registration_auto_refresh: true,
            registration_refresh_jitter_percent: 5,
            unregister_on_shutdown_timeout_secs: 3,
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
            srtp_offered_suites: SrtpSuitePolicy::Default.suites(),
            media_public_addr: None,
            media_mode: MediaMode::Enabled,
            media_session_capacity: None,
            stun_server: None,
            comfort_noise_enabled: false,
            strict_codec_matching: true,
            offered_codecs: vec![0, 8, 101],
            incoming_call_channel_capacity: 1000,
            state_event_channel_capacity: 1000,
            sip_transport_channel_capacity: 10_000,
            sip_transport_dispatch_workers: None,
            sip_transport_dispatch_queue_capacity: None,
            sip_udp_recv_buffer_size: None,
            sip_udp_send_buffer_size: None,
            sip_udp_parse_workers: None,
            sip_udp_parse_queue_capacity: None,
            sip_udp_parse_dispatch: None,
            transaction_event_channel_capacity: 10_000,
            sip_transaction_dispatch_workers: None,
            sip_transaction_dispatch_queue_capacity: None,
            sip_transaction_dispatch_priority_burst_max: None,
            sip_invite_2xx_retransmit_max_due_per_tick: None,
            sip_dialog_dispatch_workers: None,
            sip_dialog_dispatch_queue_capacity: None,
            global_event_channel_capacity: 10_000,
            session_event_dispatcher_workers: default_session_event_dispatcher_workers(),
            session_event_dispatcher_channel_capacity: 10_000,
            server_call_capacity: None,
            sip_udp_diagnostics: false,
            sip_transaction_timing_diagnostics: false,
            sip_dialog_timing_diagnostics: false,
            media_setup_diagnostics: false,
            cleanup_diagnostics: false,
            cleanup_diagnostic_events: false,
            srtp_diagnostics: false,
            rtp_diagnostics: false,
            media_sdp_diagnostics: false,
            auto_emit_extra_headers: Vec::new(),
        }
    }

    /// Deployment profile for local examples and integration tests.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip::Config;
    /// let config = Config::local_lab("alice", 5060);
    /// assert_eq!(config.local_uri, "sip:alice@127.0.0.1:5060");
    /// ```
    pub fn local_lab(name: &str, port: u16) -> Self {
        Self::local(name, port)
    }

    /// Deployment profile for a directly reachable LAN PBX endpoint.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip::Config;
    /// let bind = "0.0.0.0:5060".parse().unwrap();
    /// let advertised = "192.168.1.50:5060".parse().unwrap();
    /// let config = Config::lan_pbx("alice", bind, advertised);
    /// assert_eq!(config.sip_advertised_addr, Some(advertised));
    /// ```
    pub fn lan_pbx(name: &str, bind_addr: SocketAddr, advertised_addr: SocketAddr) -> Self {
        let mut config = Self::on(name, bind_addr.ip(), bind_addr.port());
        config.bind_addr = bind_addr;
        config.sip_advertised_addr = Some(advertised_addr);
        config.media_public_addr = Some(SocketAddr::new(advertised_addr.ip(), 0));
        config
    }

    /// Deployment profile for Asterisk TLS + SDES-SRTP with registered-flow
    /// reuse over the outbound registration connection.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip::{Config, SipTlsMode};
    /// let bind = "0.0.0.0:5061".parse().unwrap();
    /// let config = Config::asterisk_tls_registered_flow(
    ///     "alice",
    ///     bind,
    ///     "urn:uuid:00000000-0000-0000-0000-000000000001",
    /// );
    /// assert_eq!(config.sip_tls_mode, SipTlsMode::ClientOnly);
    /// assert!(config.srtp_required);
    /// ```
    pub fn asterisk_tls_registered_flow(
        name: &str,
        bind_addr: SocketAddr,
        sip_instance: impl Into<String>,
    ) -> Self {
        let mut config = Self::on(name, bind_addr.ip(), bind_addr.port())
            .tls_registered_flow_symmetric(sip_instance);
        config.bind_addr = bind_addr;
        config.offer_srtp = true;
        config.srtp_required = true;
        config
    }

    /// Deployment profile for FreeSWITCH/Sofia's internal LAN profile.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip::Config;
    /// let bind = "192.168.1.50:5060".parse().unwrap();
    /// let config = Config::freeswitch_internal("alice", bind);
    /// assert!(config.strict_codec_matching);
    /// ```
    pub fn freeswitch_internal(name: &str, bind_addr: SocketAddr) -> Self {
        let mut config = Self::on(name, bind_addr.ip(), bind_addr.port());
        config.bind_addr = bind_addr;
        config.strict_codec_matching = true;
        config
    }

    /// Deployment profile for FreeSWITCH TLS + mandatory SDES-SRTP with a
    /// directly reachable TLS Contact.
    ///
    /// The profile enables SIP TLS listener mode, mandatory SRTP, strict codec
    /// matching, and the FreeSWITCH-compatible SDES suite policy. It does not
    /// pin a single crypto suite; SDP offer/answer decides the negotiated
    /// suite.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip::Config;
    /// let udp_bind = "0.0.0.0:5060".parse().unwrap();
    /// let tls_bind = "0.0.0.0:5061".parse().unwrap();
    /// let config = Config::freeswitch_tls_srtp_reachable_contact(
    ///     "alice",
    ///     udp_bind,
    ///     tls_bind,
    ///     "cert.pem",
    ///     "key.pem",
    /// );
    /// assert!(config.offer_srtp);
    /// assert!(config.srtp_required);
    /// ```
    pub fn freeswitch_tls_srtp_reachable_contact(
        name: &str,
        bind_addr: SocketAddr,
        tls_bind_addr: SocketAddr,
        cert_path: impl Into<std::path::PathBuf>,
        key_path: impl Into<std::path::PathBuf>,
    ) -> Self {
        let mut config = Self::freeswitch_internal(name, bind_addr)
            .tls_reachable_contact(tls_bind_addr, cert_path, key_path)
            .with_srtp_suite_policy(SrtpSuitePolicy::FreeSwitchCompatible);
        config.offer_srtp = true;
        config.srtp_required = true;
        config
    }

    /// Deployment profile for carrier/SBC style outbound proxy operation.
    ///
    /// This is a conservative starting point: TLS client mode, registered-flow
    /// Contact behavior, mandatory SDES-SRTP, explicit public media address,
    /// and a preloaded outbound proxy route for INVITEs. REGISTER proxy,
    /// Service-Route/Path, SRV/NAPTR, and ICE remain separate hardening work.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip::{Config, SipContactMode};
    /// let bind = "0.0.0.0:5061".parse().unwrap();
    /// let public = "203.0.113.10:5061".parse().unwrap();
    /// let config = Config::carrier_sbc(
    ///     "alice",
    ///     bind,
    ///     public,
    ///     "sips:sbc.example.com:5061;lr",
    ///     "urn:uuid:00000000-0000-0000-0000-000000000001",
    /// );
    /// assert_eq!(config.sip_contact_mode, SipContactMode::RegisteredFlowRfc5626);
    /// assert!(config.srtp_required);
    /// ```
    pub fn carrier_sbc(
        name: &str,
        bind_addr: SocketAddr,
        public_addr: SocketAddr,
        outbound_proxy_uri: impl Into<String>,
        sip_instance: impl Into<String>,
    ) -> Self {
        let mut config = Self::on(name, bind_addr.ip(), bind_addr.port())
            .tls_registered_flow_rfc5626(sip_instance);
        config.bind_addr = bind_addr;
        config.sip_advertised_addr = Some(public_addr);
        config.tls_advertised_addr = Some(public_addr);
        config.media_public_addr = Some(SocketAddr::new(public_addr.ip(), 0));
        config.outbound_proxy_uri = Some(outbound_proxy_uri.into());
        config.offer_srtp = true;
        config.srtp_required = true;
        config
    }

    /// Placeholder deployment profile for a SIP proxy plus RTPengine lab.
    ///
    /// The signaling side preloads the outbound proxy route; media relay
    /// integration remains explicit because RTPengine control belongs above
    /// rvoip-sip.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip::Config;
    /// let bind = "0.0.0.0:5060".parse().unwrap();
    /// let advertised = "192.168.1.50:5060".parse().unwrap();
    /// let config = Config::proxy_rtpengine(
    ///     "alice",
    ///     bind,
    ///     advertised,
    ///     "sip:proxy.example.com;lr",
    /// );
    /// assert_eq!(config.outbound_proxy_uri.as_deref(), Some("sip:proxy.example.com;lr"));
    /// ```
    pub fn proxy_rtpengine(
        name: &str,
        bind_addr: SocketAddr,
        advertised_addr: SocketAddr,
        outbound_proxy_uri: impl Into<String>,
    ) -> Self {
        let mut config = Self::lan_pbx(name, bind_addr, advertised_addr);
        config.outbound_proxy_uri = Some(outbound_proxy_uri.into());
        config
    }

    /// Replace the SDES-SRTP offer suite list with a named policy.
    ///
    /// This only changes the advertised suite order/list. Callers still choose
    /// whether SRTP is offered or mandatory with [`Config::offer_srtp`] and
    /// [`Config::srtp_required`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip::{Config, SrtpSuitePolicy};
    /// let config = Config::local("alice", 5060)
    ///     .with_srtp_suite_policy(SrtpSuitePolicy::FreeSwitchCompatible);
    /// assert_eq!(config.srtp_offered_suites.len(), 4);
    /// ```
    pub fn with_srtp_suite_policy(mut self, policy: SrtpSuitePolicy) -> Self {
        self.srtp_offered_suites = policy.suites();
        self
    }

    /// Set the legacy incoming-call compatibility channel capacity.
    ///
    /// The default is `1000`, which is enough for normal bursty call-arrival
    /// workloads while still bounding memory. Values below `1` are rejected by
    /// [`Config::validate`].
    pub fn with_incoming_call_channel_capacity(mut self, capacity: usize) -> Self {
        self.incoming_call_channel_capacity = capacity;
        self
    }

    /// Set SIP signaling channel capacities from one expected-concurrency knob.
    ///
    /// `capacity` is the expected number of concurrent or burst-arriving calls.
    /// Per-call queues use that value directly; lower-level transport and
    /// transaction event queues use `capacity * 10` because a single call
    /// generates multiple SIP messages and transaction lifecycle events.
    /// Values below `1` are rejected by [`Config::validate`].
    pub fn with_channel_capacity(mut self, capacity: usize) -> Self {
        let event_capacity = capacity.saturating_mul(10);
        self.incoming_call_channel_capacity = capacity;
        self.state_event_channel_capacity = capacity;
        self.sip_transport_channel_capacity = event_capacity;
        self.transaction_event_channel_capacity = event_capacity;
        self.global_event_channel_capacity = event_capacity;
        self.session_event_dispatcher_channel_capacity = event_capacity;
        self
    }

    /// Set a server-side active-call capacity profile.
    ///
    /// This reserves hot lookup indexes for `capacity` active or
    /// burst-arriving calls without changing event queue sizes. Clients should
    /// usually leave this unset and use the defaults.
    pub fn with_server_capacity(mut self, capacity: usize) -> Self {
        self.server_call_capacity = Some(capacity);
        self
    }

    /// Apply a high-CPS UDP auto-answer profile.
    ///
    /// This keeps media enabled, suppresses automatic provisional responses,
    /// sizes SIP event queues from `capacity`, and configures the UDP receive
    /// path for a single fast parse worker with a queue sized to the same
    /// burst capacity. It disables automatic `100 Trying` because fixed
    /// immediate-answer services should send the final response before Timer
    /// 100 would fire, and avoiding the timer task/message reduces hot-path
    /// work. It does not enable the fused fast-auto-accept path yet; that
    /// remains an explicit opt-in until the 8000 CPS cleanup/retransmit target
    /// is stable. It does not enlarge socket buffers and does not set
    /// [`Config::server_call_capacity`]. It also leaves
    /// [`Config::sip_transaction_dispatch_priority_burst_max`] and
    /// [`Config::sip_invite_2xx_retransmit_max_due_per_tick`] unset so load
    /// tests can tune dispatch fairness and retransmit pacing explicitly.
    pub fn with_high_cps_udp_auto_answer(mut self, capacity: usize) -> Self {
        self = self.with_channel_capacity(capacity);
        self.auto_180_ringing = false;
        self.auto_100_trying = false;
        self.sip_udp_parse_workers = Some(1);
        self.sip_udp_parse_queue_capacity = Some(capacity);
        self.media_mode = MediaMode::Enabled;
        self
    }

    fn dialog_index_capacity_hint(&self) -> usize {
        self.server_call_capacity
            .unwrap_or(self.transaction_event_channel_capacity)
            .max(1)
    }

    fn transaction_index_capacity_hint(&self) -> usize {
        self.server_call_capacity
            // INVITE server transactions, BYE server transactions, ACK
            // indexes, and retransmission caches can remain live beyond the
            // active-call count. Size for the short transaction-retention
            // window, not just simultaneous dialogs.
            .map(|capacity| capacity.saturating_mul(16))
            .unwrap_or(self.transaction_event_channel_capacity)
            .max(1)
    }

    /// Set the RTP media port range.
    ///
    /// The default range is [`Config::DEFAULT_MEDIA_PORT_START`] through
    /// [`Config::DEFAULT_MEDIA_PORT_END`]. Values are checked by
    /// [`Config::validate`].
    pub fn with_media_ports(mut self, start: u16, end: u16) -> Self {
        self.media_port_start = start;
        self.media_port_end = end;
        self.media_port_capacity = None;
        self
    }

    /// Set the RTP media port range by start port and requested capacity.
    ///
    /// Validation rejects capacity `0`, start ports below [`MIN_PORT`], and
    /// requested capacities that do not fit in the `u16` port space.
    pub fn with_media_port_capacity(mut self, start: u16, capacity: usize) -> Self {
        self.media_port_start = start;
        self.media_port_end = capacity
            .checked_sub(1)
            .and_then(|offset| (start as usize).checked_add(offset))
            .and_then(|end| u16::try_from(end).ok())
            .unwrap_or(u16::MAX);
        self.media_port_capacity = Some(capacity);
        self
    }

    /// Enable or disable automatic `180 Ringing` on inbound INVITEs.
    ///
    /// `true` is the PBX-friendly default. `false` is useful for IVR,
    /// call-center, and benchmark listeners that answer immediately with a
    /// final response.
    pub fn with_auto_180_ringing(mut self, enabled: bool) -> Self {
        self.auto_180_ringing = enabled;
        self
    }

    /// Enable or disable the automatic RFC 3261 `100 Trying` timer.
    ///
    /// The default is `true`. High-CPS immediate-answer services can set this
    /// to `false` to avoid spawning a timer task for every INVITE when a final
    /// response is expected well before Timer 100 would fire.
    pub fn with_auto_100_trying(mut self, enabled: bool) -> Self {
        self.auto_100_trying = enabled;
        self
    }

    /// Enable or disable immediate session-path accept for inbound INVITEs.
    ///
    /// This is intentionally separate from [`Config::auto_180_ringing`]:
    /// disabling 180 only removes the provisional response, while enabling
    /// this option sends the final answer before app callbacks run.
    pub fn with_fast_auto_accept_incoming_calls(mut self, enabled: bool) -> Self {
        self.fast_auto_accept_incoming_calls = enabled;
        self
    }

    /// Set media allocation behavior.
    pub fn with_media_mode(mut self, mode: MediaMode) -> Self {
        self.media_mode = mode;
        self
    }

    /// Set the media-core session and RTP allocator capacity hint.
    pub fn with_media_session_capacity(mut self, capacity: usize) -> Self {
        self.media_session_capacity = Some(capacity);
        self
    }

    /// Enable or disable real media-core RTP allocation.
    ///
    /// Disabling media switches to [`MediaMode::SignalingOnly`] with SDP port
    /// `9`, the discard port convention used for signaling-only tests.
    pub fn with_media_enabled(mut self, enabled: bool) -> Self {
        self.media_mode = if enabled {
            MediaMode::Enabled
        } else {
            MediaMode::SignalingOnly { sdp_rtp_port: 9 }
        };
        self
    }

    /// Skip media-core RTP allocation while still generating SDP.
    pub fn with_signaling_only_media(mut self, sdp_rtp_port: u16) -> Self {
        self.media_mode = MediaMode::SignalingOnly { sdp_rtp_port };
        self
    }

    /// Set the internal state-machine event channel capacity.
    ///
    /// The default is `1000`. Values below `1` are rejected by
    /// [`Config::validate`].
    pub fn with_state_event_channel_capacity(mut self, capacity: usize) -> Self {
        self.state_event_channel_capacity = capacity;
        self
    }

    /// Set the SIP transport event channel capacity.
    ///
    /// The default is `10000`. Values below `1` are rejected by
    /// [`Config::validate`].
    pub fn with_sip_transport_channel_capacity(mut self, capacity: usize) -> Self {
        self.sip_transport_channel_capacity = capacity;
        self
    }

    /// Set the SIP transport-manager forwarding worker count.
    ///
    /// Values above `1` enable keyed sharding between transport receive/parse
    /// and transaction-manager ingress. Values below `1` are rejected by
    /// [`Config::validate`].
    pub fn with_sip_transport_dispatch_workers(mut self, workers: usize) -> Self {
        self.sip_transport_dispatch_workers = Some(workers);
        self
    }

    /// Set the SIP transport-manager forwarding queue capacity.
    ///
    /// `None` uses [`Config::sip_transport_channel_capacity`]. Values below
    /// `1` are rejected by [`Config::validate`].
    pub fn with_sip_transport_dispatch_queue_capacity(mut self, capacity: usize) -> Self {
        self.sip_transport_dispatch_queue_capacity = Some(capacity);
        self
    }

    /// Set SIP transport-manager forwarding worker and queue overrides
    /// together.
    pub fn with_sip_transport_dispatch_config(
        mut self,
        workers: Option<usize>,
        queue_capacity: Option<usize>,
    ) -> Self {
        self.sip_transport_dispatch_workers = workers;
        self.sip_transport_dispatch_queue_capacity = queue_capacity;
        self
    }

    /// Set SIP UDP socket receive/send buffer sizes in bytes.
    ///
    /// Use this for high-CPS server profiles where the kernel UDP queue must
    /// absorb bursts while application queues drain. Pass `None` for either
    /// side to keep that side at the OS default.
    pub fn with_sip_udp_socket_buffers(
        mut self,
        recv_buffer_size: Option<usize>,
        send_buffer_size: Option<usize>,
    ) -> Self {
        self.sip_udp_recv_buffer_size = recv_buffer_size;
        self.sip_udp_send_buffer_size = send_buffer_size;
        self
    }

    /// Set the SIP UDP receive socket buffer size (`SO_RCVBUF`) in bytes.
    pub fn with_sip_udp_recv_buffer_size(mut self, size: usize) -> Self {
        self.sip_udp_recv_buffer_size = Some(size);
        self
    }

    /// Set the SIP UDP send socket buffer size (`SO_SNDBUF`) in bytes.
    pub fn with_sip_udp_send_buffer_size(mut self, size: usize) -> Self {
        self.sip_udp_send_buffer_size = Some(size);
        self
    }

    /// Set the UDP parse worker count.
    pub fn with_sip_udp_parse_workers(mut self, workers: usize) -> Self {
        self.sip_udp_parse_workers = Some(workers);
        self
    }

    /// Set the per-worker UDP parse queue capacity.
    pub fn with_sip_udp_parse_queue_capacity(mut self, capacity: usize) -> Self {
        self.sip_udp_parse_queue_capacity = Some(capacity);
        self
    }

    /// Set the UDP parse worker dispatch strategy.
    pub fn with_sip_udp_parse_dispatch(
        mut self,
        dispatch: rvoip_sip_transport::UdpParseDispatch,
    ) -> Self {
        self.sip_udp_parse_dispatch = Some(dispatch);
        self
    }

    /// Set UDP parse worker and queue overrides together.
    pub fn with_sip_udp_parse_config(
        mut self,
        workers: Option<usize>,
        queue_capacity: Option<usize>,
    ) -> Self {
        self.sip_udp_parse_workers = workers;
        self.sip_udp_parse_queue_capacity = queue_capacity;
        self
    }

    /// Set the transaction-manager event channel capacity.
    ///
    /// The default is `10000`. Values below `1` are rejected by
    /// [`Config::validate`].
    pub fn with_transaction_event_channel_capacity(mut self, capacity: usize) -> Self {
        self.transaction_event_channel_capacity = capacity;
        self
    }

    /// Set the transaction-manager ingress dispatch worker count.
    ///
    /// Values above `1` enable keyed sharding of incoming transport events.
    /// Values below `1` are rejected by [`Config::validate`].
    pub fn with_sip_transaction_dispatch_workers(mut self, workers: usize) -> Self {
        self.sip_transaction_dispatch_workers = Some(workers);
        self
    }

    /// Set the transaction-manager ingress dispatch queue capacity.
    ///
    /// `None` uses [`Config::transaction_event_channel_capacity`]. Values below
    /// `1` are rejected by [`Config::validate`].
    pub fn with_sip_transaction_dispatch_queue_capacity(mut self, capacity: usize) -> Self {
        self.sip_transaction_dispatch_queue_capacity = Some(capacity);
        self
    }

    /// Set the transaction-manager ACK/BYE priority burst limit.
    ///
    /// This only affects multi-worker transaction dispatch. After this many
    /// consecutive priority-lane ACK/BYE events, a worker processes one ready
    /// normal-lane item before resuming priority work. Use lower values when
    /// INVITE/CANCEL/response work is being starved; use higher values when
    /// BYE/ACK latency is the bottleneck. Values below `1` are rejected by
    /// [`Config::validate`].
    pub fn with_sip_transaction_dispatch_priority_burst_max(mut self, max_burst: usize) -> Self {
        self.sip_transaction_dispatch_priority_burst_max = Some(max_burst);
        self
    }

    /// Set the cached INVITE `2xx` retransmission maintenance budget.
    ///
    /// The value is the maximum number of cached INVITE `2xx` responses the
    /// transaction manager may proactively resend per 100 ms tick. Lower values
    /// pace UDP send bursts; higher values clear retransmission backlog faster
    /// when the host send path has capacity. Values below `1` are rejected by
    /// [`Config::validate`].
    pub fn with_sip_invite_2xx_retransmit_max_due_per_tick(
        mut self,
        max_due_per_tick: usize,
    ) -> Self {
        self.sip_invite_2xx_retransmit_max_due_per_tick = Some(max_due_per_tick);
        self
    }

    /// Set transaction-manager ingress worker and queue overrides together.
    pub fn with_sip_transaction_dispatch_config(
        mut self,
        workers: Option<usize>,
        queue_capacity: Option<usize>,
    ) -> Self {
        self.sip_transaction_dispatch_workers = workers;
        self.sip_transaction_dispatch_queue_capacity = queue_capacity;
        self
    }

    /// Set the dialog-core transaction-event dispatch worker count.
    ///
    /// Values above `1` enable keyed sharding of transaction events before
    /// dialog protocol handling. Values below `1` are rejected by
    /// [`Config::validate`].
    pub fn with_sip_dialog_dispatch_workers(mut self, workers: usize) -> Self {
        self.sip_dialog_dispatch_workers = Some(workers);
        self
    }

    /// Set the dialog-core transaction-event dispatch queue capacity.
    ///
    /// `None` uses the dialog max-dialog capacity hint. Values below `1` are
    /// rejected by [`Config::validate`].
    pub fn with_sip_dialog_dispatch_queue_capacity(mut self, capacity: usize) -> Self {
        self.sip_dialog_dispatch_queue_capacity = Some(capacity);
        self
    }

    /// Set dialog-core transaction-event dispatch worker and queue overrides
    /// together.
    pub fn with_sip_dialog_dispatch_config(
        mut self,
        workers: Option<usize>,
        queue_capacity: Option<usize>,
    ) -> Self {
        self.sip_dialog_dispatch_workers = workers;
        self.sip_dialog_dispatch_queue_capacity = queue_capacity;
        self
    }

    /// Set the infra-common global event bus channel capacity.
    ///
    /// The default is `10000`. Values below `1` are rejected by
    /// [`Config::validate`].
    pub fn with_global_event_channel_capacity(mut self, capacity: usize) -> Self {
        self.global_event_channel_capacity = capacity;
        self
    }

    /// Set the app-session event dispatcher worker count.
    ///
    /// Values below `1` are rejected by [`Config::validate`].
    pub fn with_session_event_dispatcher_workers(mut self, workers: usize) -> Self {
        self.session_event_dispatcher_workers = workers;
        self
    }

    /// Set the per-worker app-session event dispatcher queue capacity.
    ///
    /// Values below `1` are rejected by [`Config::validate`].
    pub fn with_session_event_dispatcher_channel_capacity(mut self, capacity: usize) -> Self {
        self.session_event_dispatcher_channel_capacity = capacity;
        self
    }

    /// Enable or disable SIP UDP transport and duplicate-recovery diagnostics.
    pub fn with_sip_udp_diagnostics(mut self, enabled: bool) -> Self {
        self.sip_udp_diagnostics = enabled;
        self
    }

    /// Enable or disable high-cardinality transaction timing diagnostics.
    pub fn with_sip_transaction_timing_diagnostics(mut self, enabled: bool) -> Self {
        self.sip_transaction_timing_diagnostics = enabled;
        self
    }

    /// Enable or disable high-cardinality dialog timing diagnostics.
    pub fn with_sip_dialog_timing_diagnostics(mut self, enabled: bool) -> Self {
        self.sip_dialog_timing_diagnostics = enabled;
        self
    }

    /// Enable or disable media setup/teardown timing diagnostics.
    pub fn with_media_setup_diagnostics(mut self, enabled: bool) -> Self {
        self.media_setup_diagnostics = enabled;
        self
    }

    /// Enable or disable cleanup-stage timing diagnostics.
    pub fn with_cleanup_diagnostics(mut self, enabled: bool) -> Self {
        self.cleanup_diagnostics = enabled;
        self
    }

    /// Enable or disable per-operation cleanup diagnostic event logs.
    pub fn with_cleanup_diagnostic_events(mut self, enabled: bool) -> Self {
        self.cleanup_diagnostic_events = enabled;
        self
    }

    /// Enable or disable SRTP negotiation diagnostic log lines.
    pub fn with_srtp_diagnostics(mut self, enabled: bool) -> Self {
        self.srtp_diagnostics = enabled;
        self
    }

    /// Enable or disable RTP packet diagnostic log lines.
    pub fn with_rtp_diagnostics(mut self, enabled: bool) -> Self {
        self.rtp_diagnostics = enabled;
        self
    }

    /// Enable or disable SDP media diagnostic log lines.
    pub fn with_media_sdp_diagnostics(mut self, enabled: bool) -> Self {
        self.media_sdp_diagnostics = enabled;
        self
    }

    /// Configure SIP TLS as a directly reachable Contact listener.
    ///
    /// The UA will both dial outbound TLS and listen on `tls_bind_addr` for
    /// inbound TLS requests sent to its advertised Contact.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip::{Config, SipContactMode, SipTlsMode};
    /// let tls_addr = "0.0.0.0:5061".parse().unwrap();
    /// let config = Config::local("alice", 5060)
    ///     .tls_reachable_contact(tls_addr, "cert.pem", "key.pem");
    /// assert_eq!(config.sip_tls_mode, SipTlsMode::ClientAndServer);
    /// assert_eq!(config.sip_contact_mode, SipContactMode::ReachableContact);
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip::{Config, SipContactMode};
    /// let config = Config::local("alice", 5060)
    ///     .tls_registered_flow_rfc5626("urn:uuid:00000000-0000-0000-0000-000000000001");
    /// assert_eq!(config.sip_contact_mode, SipContactMode::RegisteredFlowRfc5626);
    /// assert!(config.sip_outbound_enabled);
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip::{Config, SipContactMode};
    /// let config = Config::local("alice", 5060)
    ///     .tls_registered_flow_symmetric("urn:uuid:00000000-0000-0000-0000-000000000001");
    /// assert_eq!(config.sip_contact_mode, SipContactMode::RegisteredFlowSymmetric);
    /// ```
    pub fn tls_registered_flow_symmetric(mut self, sip_instance: impl Into<String>) -> Self {
        self.sip_tls_mode = SipTlsMode::ClientOnly;
        self.sip_contact_mode = SipContactMode::RegisteredFlowSymmetric;
        self.sip_instance = Some(sip_instance.into());
        self
    }

    /// Validate the SIP TLS/contact-mode configuration.
    ///
    /// # Examples
    ///
    /// ```
    /// # use rvoip_sip::Config;
    /// let config = Config::local("alice", 5060);
    /// config.validate().unwrap();
    /// ```
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
        if self.registration_refresh_jitter_percent > 50 {
            return Err(SessionError::ConfigError(
                "registration_refresh_jitter_percent must be <= 50".to_string(),
            ));
        }
        if self.incoming_call_channel_capacity == 0 {
            return Err(SessionError::ConfigError(
                "incoming_call_channel_capacity must be at least 1".to_string(),
            ));
        }
        if self.state_event_channel_capacity == 0 {
            return Err(SessionError::ConfigError(
                "state_event_channel_capacity must be at least 1".to_string(),
            ));
        }
        if self.sip_transport_channel_capacity == 0 {
            return Err(SessionError::ConfigError(
                "sip_transport_channel_capacity must be at least 1".to_string(),
            ));
        }
        if matches!(self.sip_transport_dispatch_workers, Some(0)) {
            return Err(SessionError::ConfigError(
                "sip_transport_dispatch_workers must be at least 1 when set".to_string(),
            ));
        }
        if let Some(workers) = self.sip_transport_dispatch_workers {
            if workers
                > rvoip_sip_dialog::transaction::transport::MAX_TRANSPORT_EVENT_DISPATCH_WORKERS
            {
                return Err(SessionError::ConfigError(format!(
                    "sip_transport_dispatch_workers must be <= {} when set",
                    rvoip_sip_dialog::transaction::transport::MAX_TRANSPORT_EVENT_DISPATCH_WORKERS
                )));
            }
        }
        if matches!(self.sip_transport_dispatch_queue_capacity, Some(0)) {
            return Err(SessionError::ConfigError(
                "sip_transport_dispatch_queue_capacity must be at least 1 when set".to_string(),
            ));
        }
        if matches!(self.sip_udp_recv_buffer_size, Some(0)) {
            return Err(SessionError::ConfigError(
                "sip_udp_recv_buffer_size must be at least 1 when set".to_string(),
            ));
        }
        if matches!(self.sip_udp_send_buffer_size, Some(0)) {
            return Err(SessionError::ConfigError(
                "sip_udp_send_buffer_size must be at least 1 when set".to_string(),
            ));
        }
        if matches!(self.sip_udp_parse_workers, Some(0)) {
            return Err(SessionError::ConfigError(
                "sip_udp_parse_workers must be at least 1 when set".to_string(),
            ));
        }
        if let Some(workers) = self.sip_udp_parse_workers {
            if workers > rvoip_sip_transport::UdpParseConfig::MAX_WORKERS {
                return Err(SessionError::ConfigError(format!(
                    "sip_udp_parse_workers must be <= {} when set",
                    rvoip_sip_transport::UdpParseConfig::MAX_WORKERS
                )));
            }
        }
        if matches!(self.sip_udp_parse_queue_capacity, Some(0)) {
            return Err(SessionError::ConfigError(
                "sip_udp_parse_queue_capacity must be at least 1 when set".to_string(),
            ));
        }
        if self.transaction_event_channel_capacity == 0 {
            return Err(SessionError::ConfigError(
                "transaction_event_channel_capacity must be at least 1".to_string(),
            ));
        }
        if matches!(self.sip_transaction_dispatch_workers, Some(0)) {
            return Err(SessionError::ConfigError(
                "sip_transaction_dispatch_workers must be at least 1 when set".to_string(),
            ));
        }
        if let Some(workers) = self.sip_transaction_dispatch_workers {
            if workers > rvoip_sip_dialog::transaction::MAX_TRANSACTION_DISPATCH_WORKERS {
                return Err(SessionError::ConfigError(format!(
                    "sip_transaction_dispatch_workers must be <= {} when set",
                    rvoip_sip_dialog::transaction::MAX_TRANSACTION_DISPATCH_WORKERS
                )));
            }
        }
        if matches!(self.sip_transaction_dispatch_queue_capacity, Some(0)) {
            return Err(SessionError::ConfigError(
                "sip_transaction_dispatch_queue_capacity must be at least 1 when set".to_string(),
            ));
        }
        if matches!(self.sip_transaction_dispatch_priority_burst_max, Some(0)) {
            return Err(SessionError::ConfigError(
                "sip_transaction_dispatch_priority_burst_max must be at least 1 when set"
                    .to_string(),
            ));
        }
        if matches!(self.sip_invite_2xx_retransmit_max_due_per_tick, Some(0)) {
            return Err(SessionError::ConfigError(
                "sip_invite_2xx_retransmit_max_due_per_tick must be at least 1 when set"
                    .to_string(),
            ));
        }
        if matches!(self.sip_dialog_dispatch_workers, Some(0)) {
            return Err(SessionError::ConfigError(
                "sip_dialog_dispatch_workers must be at least 1 when set".to_string(),
            ));
        }
        if let Some(workers) = self.sip_dialog_dispatch_workers {
            if workers > rvoip_sip_dialog::manager::MAX_DIALOG_EVENT_DISPATCH_WORKERS {
                return Err(SessionError::ConfigError(format!(
                    "sip_dialog_dispatch_workers must be <= {} when set",
                    rvoip_sip_dialog::manager::MAX_DIALOG_EVENT_DISPATCH_WORKERS
                )));
            }
        }
        if matches!(self.sip_dialog_dispatch_queue_capacity, Some(0)) {
            return Err(SessionError::ConfigError(
                "sip_dialog_dispatch_queue_capacity must be at least 1 when set".to_string(),
            ));
        }
        if self.global_event_channel_capacity == 0 {
            return Err(SessionError::ConfigError(
                "global_event_channel_capacity must be at least 1".to_string(),
            ));
        }
        if self.session_event_dispatcher_workers == 0 {
            return Err(SessionError::ConfigError(
                "session_event_dispatcher_workers must be at least 1".to_string(),
            ));
        }
        if self.session_event_dispatcher_channel_capacity == 0 {
            return Err(SessionError::ConfigError(
                "session_event_dispatcher_channel_capacity must be at least 1".to_string(),
            ));
        }
        if matches!(self.server_call_capacity, Some(0)) {
            return Err(SessionError::ConfigError(
                "server_call_capacity must be at least 1 when set".to_string(),
            ));
        }
        if matches!(self.media_session_capacity, Some(0)) {
            return Err(SessionError::ConfigError(
                "media_session_capacity must be at least 1 when set".to_string(),
            ));
        }
        if matches!(self.media_port_capacity, Some(0)) {
            return Err(SessionError::ConfigError(
                "media_port_capacity must be at least 1 when set".to_string(),
            ));
        }
        if self.media_port_start < MIN_PORT {
            return Err(SessionError::ConfigError(format!(
                "media_port_start must be >= {}",
                MIN_PORT
            )));
        }
        if self.media_port_start > self.media_port_end {
            return Err(SessionError::ConfigError(
                "media_port_start must be <= media_port_end".to_string(),
            ));
        }
        if let Some(capacity) = self.media_port_capacity {
            let available = self.media_port_end as usize - self.media_port_start as usize + 1;
            if available < capacity {
                return Err(SessionError::ConfigError(format!(
                    "media port range {}-{} provides {} ports, below requested media_port_capacity {}",
                    self.media_port_start, self.media_port_end, available, capacity
                )));
            }
        }
        if let MediaMode::SignalingOnly { sdp_rtp_port: 0 } = self.media_mode {
            return Err(SessionError::ConfigError(
                "signaling-only media SDP RTP port must be at least 1".to_string(),
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

        // SIP_API_DESIGN_2 §7.4 — auto_emit_extra_headers stack-managed
        // rejection. The state machine's auto-emit paths can't route
        // through `HeaderPolicy::validate_outbound` (there's no
        // builder), so we hard-fail at Config-construction time when
        // an application stages a name that would desync the dialog
        // or transaction.
        for header in &self.auto_emit_extra_headers {
            if crate::api::headers::policy::forbidden_for_carry_through(&header.name()) {
                return Err(SessionError::ConfigError(format!(
                    "Config.auto_emit_extra_headers contains stack-managed header {:?} \
                     (Call-ID / CSeq / Via / Max-Forwards / Content-Length / Record-Route / \
                     Route are owned by the dialog/transaction layer)",
                    header.name()
                )));
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

fn default_session_event_dispatcher_workers() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 16)
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

    /// Per-call lifecycle index for deterministic late waiters.
    lifecycle: LifecycleIndex,

    /// App event publisher that updates lifecycle before global bus delivery.
    app_event_publisher: SessionEventPublisher,

    /// SIP_API_DESIGN_2 Phase A: shared session registry so the four
    /// public surfaces can fetch the parsed inbound `Arc<Request>` when
    /// constructing an `IncomingCall`.
    pub(crate) session_registry: Arc<SessionRegistry>,
}

impl UnifiedCoordinator {
    /// SIP_API_DESIGN_2 Phase C — read-only access to
    /// [`Config::local_uri`] for builder surfaces that need to
    /// pre-populate the `From` URI when the caller passes `None`.
    /// Kept inherent-impl so the surface adapter doesn't need access
    /// to the private `config` field.
    pub fn config_local_uri(&self) -> String {
        self.config.local_uri.clone()
    }

    /// SIP_API_DESIGN_2 §7.1 — read-only access to
    /// [`Config::pai_uri`] for outbound builders that need to resolve
    /// the per-call `P-Asserted-Identity` against
    /// [`PaiOverride::Default`](crate::api::send::PaiOverride::Default).
    pub fn config_pai_uri(&self) -> Option<String> {
        self.config.pai_uri.clone()
    }

    /// Read-only access to [`Config::credentials`] so outbound builders
    /// can fall back to the peer-level default when the application
    /// did not stage per-call credentials via `with_credentials(..)`.
    pub fn config_credentials(&self) -> Option<crate::types::Credentials> {
        self.config.credentials.clone()
    }

    /// SIP_API_DESIGN_2 Phase C — internal accessor to the
    /// [`DialogAdapter`] so send/respond builders can route their
    /// dispatch through the same translation layer used by the legacy
    /// flat methods. Crate-private so the builders are the only
    /// external consumers.
    pub(crate) fn dialog_adapter(&self) -> &Arc<DialogAdapter> {
        &self.dialog_adapter
    }

    // ──────────────────────────────────────────────────────────────────
    // SIP_API_DESIGN_2 Phase C — builder entry points.
    //
    // One verb-named entry per outbound method. Each returns a typed
    // builder implementing
    // [`SipRequestOptions`](crate::api::headers::SipRequestOptions),
    // so applications get a uniform `with_header / with_credentials /
    // with_headers_from / strip_header / .send()` shape.
    // ──────────────────────────────────────────────────────────────────

    /// Begin building an outbound INVITE.
    pub fn invite(
        self: &Arc<Self>,
        from: Option<String>,
        to: impl Into<String>,
    ) -> crate::api::send::OutboundCallBuilder {
        crate::api::send::OutboundCallBuilder::new(self.clone(), from, to)
    }

    /// Begin building an outbound BYE.
    pub fn bye(
        self: &Arc<Self>,
        session: &crate::api::handle::CallId,
    ) -> crate::api::send::ByeBuilder {
        crate::api::send::ByeBuilder::new(self.clone(), session.clone())
    }

    /// Begin building an outbound CANCEL.
    pub fn cancel(
        self: &Arc<Self>,
        session: &crate::api::handle::CallId,
    ) -> crate::api::send::CancelBuilder {
        crate::api::send::CancelBuilder::new(self.clone(), session.clone())
    }

    /// Begin building an outbound REFER.
    pub fn refer(
        self: &Arc<Self>,
        session: &crate::api::handle::CallId,
        refer_to: impl Into<String>,
    ) -> crate::api::send::ReferBuilder {
        crate::api::send::ReferBuilder::new(self.clone(), session.clone(), refer_to)
    }

    /// Begin building an outbound NOTIFY.
    pub fn notify(
        self: &Arc<Self>,
        session: &crate::api::handle::CallId,
        event_package: impl Into<String>,
    ) -> crate::api::send::NotifyBuilder {
        crate::api::send::NotifyBuilder::new(self.clone(), session.clone(), event_package)
    }

    /// Begin building an outbound INFO.
    pub fn info(
        self: &Arc<Self>,
        session: &crate::api::handle::CallId,
        content_type: impl Into<String>,
    ) -> crate::api::send::InfoBuilder {
        crate::api::send::InfoBuilder::new(self.clone(), session.clone(), content_type)
    }

    /// Begin building an outbound UPDATE.
    pub fn update(
        self: &Arc<Self>,
        session: &crate::api::handle::CallId,
    ) -> crate::api::send::UpdateBuilder {
        crate::api::send::UpdateBuilder::new(self.clone(), session.clone())
    }

    /// Begin building an outbound re-INVITE.
    pub fn reinvite(
        self: &Arc<Self>,
        session: &crate::api::handle::CallId,
    ) -> crate::api::send::ReInviteBuilder {
        crate::api::send::ReInviteBuilder::new(self.clone(), session.clone())
    }

    /// Begin building an out-of-dialog SUBSCRIBE.
    ///
    /// Canonical SUBSCRIBE-method verb-builder per SIP_API_DESIGN_2.md §3.3.
    /// The legacy state-machine-event observer that previously owned the
    /// bare name was renamed to [`on_session_events`](Self::on_session_events).
    pub fn subscribe(
        self: &Arc<Self>,
        target: impl Into<String>,
        event_package: impl Into<String>,
    ) -> crate::api::send::SubscribeBuilder {
        crate::api::send::SubscribeBuilder::new(self.clone(), target, event_package)
    }

    /// Begin building an out-of-dialog MESSAGE.
    pub fn message(
        self: &Arc<Self>,
        target: impl Into<String>,
    ) -> crate::api::send::MessageBuilder {
        crate::api::send::MessageBuilder::new(self.clone(), target)
    }

    /// Begin building an out-of-dialog OPTIONS.
    pub fn options(
        self: &Arc<Self>,
        target: impl Into<String>,
    ) -> crate::api::send::OptionsBuilder {
        crate::api::send::OptionsBuilder::new(self.clone(), target)
    }

    /// Begin building an accept response for an inbound INVITE.
    pub fn accept(
        self: &Arc<Self>,
        session: &crate::api::handle::CallId,
    ) -> crate::api::respond::AcceptBuilder {
        crate::api::respond::AcceptBuilder::new(self.clone(), session.clone())
    }

    /// Begin building a reject response for an inbound INVITE.
    pub fn reject(
        self: &Arc<Self>,
        session: &crate::api::handle::CallId,
    ) -> crate::api::respond::RejectBuilder {
        crate::api::respond::RejectBuilder::new(self.clone(), session.clone())
    }

    /// Begin building a redirect response for an inbound INVITE.
    pub fn redirect(
        self: &Arc<Self>,
        session: &crate::api::handle::CallId,
    ) -> crate::api::respond::RedirectBuilder {
        crate::api::respond::RedirectBuilder::new(self.clone(), session.clone())
    }

    /// Begin building a UAS-side auth challenge (401 / 407).
    pub fn challenge(
        self: &Arc<Self>,
        session: &crate::api::handle::CallId,
        scheme: crate::api::respond::AuthScheme,
    ) -> crate::api::respond::AuthChallengeBuilder {
        crate::api::respond::AuthChallengeBuilder::new(
            self.clone(),
            session.clone(),
            rvoip_sip_core::types::Method::Invite,
            scheme,
        )
    }

    /// Begin building an outbound REGISTER.
    ///
    /// Canonical REGISTER verb-builder per SIP_API_DESIGN_2.md §3.3. The
    /// legacy 6-arg `register(uri, from, contact, user, pw, exp)` method
    /// was deleted in Phase 12; use this builder entry with
    /// `.with_expires(...)`, `.with_extra_headers(...)`, etc. before
    /// terminating with `.send()`.
    pub fn register(
        self: &Arc<Self>,
        registrar: impl Into<String>,
        user: impl Into<String>,
        password: impl Into<String>,
    ) -> crate::api::send::RegisterBuilder {
        crate::api::send::RegisterBuilder::new(self.clone(), registrar, user, password)
    }

    /// Begin building a generic UAS response (3xx / 4xx / 5xx / 6xx)
    /// for the given session.
    pub fn respond(
        self: &Arc<Self>,
        session: &crate::api::handle::CallId,
        status: u16,
    ) -> crate::errors::Result<crate::api::respond::GenericResponseBuilder> {
        // The coordinator-level `respond` entry is reachable from session
        // state where the inbound INVITE is the only outstanding UAS
        // request; pass `Method::Invite` so the policy classifier
        // matches the UAS context.
        crate::api::respond::GenericResponseBuilder::new(
            self.clone(),
            session.clone(),
            rvoip_sip_core::types::Method::Invite,
            status,
        )
    }

    /// Begin building a reliable 1xx provisional response.
    pub fn send_provisional(
        self: &Arc<Self>,
        session: &crate::api::handle::CallId,
        code: u16,
    ) -> crate::api::respond::ProvisionalBuilder {
        crate::api::respond::ProvisionalBuilder::new(self.clone(), session.clone(), code)
    }
}

impl UnifiedCoordinator {
    /// Create and start a new coordinator.
    ///
    /// This validates [`Config`], initializes dialog and media adapters,
    /// starts the central event handler, and returns a shared coordinator
    /// handle. Background tasks are stopped by calling [`shutdown`](Self::shutdown)
    /// or by dropping all coordinator owners.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> rvoip_sip::Result<()> {
    /// use rvoip_sip::{Config, UnifiedCoordinator};
    ///
    /// let coordinator = UnifiedCoordinator::new(Config::local("alice", 5060)).await?;
    /// coordinator.shutdown_gracefully(None).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new(config: Config) -> Result<Arc<Self>> {
        config.validate()?;
        rvoip_sip_transport::diagnostics::set_enabled(config.sip_udp_diagnostics);
        rvoip_sip_dialog::diagnostics::set_enabled(config.sip_udp_diagnostics);
        rvoip_sip_dialog::diagnostics::set_transaction_timing_enabled(
            config.sip_transaction_timing_diagnostics,
        );
        rvoip_sip_dialog::diagnostics::set_dialog_timing_enabled(
            config.sip_dialog_timing_diagnostics,
        );
        rvoip_media_core::diagnostics::set_enabled(config.media_setup_diagnostics);
        crate::cleanup_diag::set_enabled(config.cleanup_diagnostics);
        crate::cleanup_diag::set_event_logs_enabled(config.cleanup_diagnostic_events);
        crate::adapters::media_adapter::set_sdp_diagnostics(
            config.srtp_diagnostics,
            config.media_sdp_diagnostics,
        );
        rvoip_rtp_core::transport::set_udp_diagnostics(
            config.srtp_diagnostics,
            config.rtp_diagnostics,
        );

        let global_event_config = rvoip_infra_common::events::EventCoordinatorConfig::monolithic()
            .with_channel_capacity(config.global_event_channel_capacity);
        let global_coordinator = Arc::new(
            rvoip_infra_common::events::GlobalEventCoordinator::new(global_event_config)
                .await
                .map_err(|e| {
                    SessionError::InternalError(format!(
                        "Failed to create global event coordinator: {}",
                        e
                    ))
                })?,
        );

        // Create core components
        let store = Arc::new(
            config
                .server_call_capacity
                .map(SessionStore::with_capacity)
                .unwrap_or_else(SessionStore::new),
        );
        let registry = Arc::new(SessionRegistry::new());

        let sip_trace_owner_id = config
            .sip_trace
            .enabled
            .then(|| format!("sip-trace-{}", uuid::Uuid::new_v4()));

        // Create adapters
        let dialog_api = Self::create_dialog_api(
            &config,
            global_coordinator.clone(),
            sip_trace_owner_id.clone(),
        )
        .await?;

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
            config.registration_auto_refresh,
            config.registration_refresh_jitter_percent,
            config.auto_emit_extra_headers.clone(),
            config.trace_redaction.clone(),
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
        media_adapter_inner.set_media_mode(config.media_mode);
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
        // NEXT_STEPS C2 — propagate the configured offered codec list.
        media_adapter_inner.set_offered_codecs(config.offered_codecs.clone());
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

        let (state_event_tx, state_event_rx) = mpsc::channel::<
            crate::state_machine::executor::SessionEvent,
        >(config.state_event_channel_capacity);

        let state_machine = Arc::new(StateMachine::new_with_custom_table(
            state_table,
            store.clone(),
            dialog_adapter.clone(),
            media_adapter.clone(),
            state_event_tx,
            config.auto_180_ringing,
        ));

        // Wire the state machine into the dialog adapter (for REGISTER
        // response handling). The adapter holds an `Arc<OnceLock<_>>`
        // internally so this post-construction init is sound without
        // `unsafe`.
        let _ = dialog_adapter.init_state_machine(state_machine.clone());

        // Create helpers
        let helpers = Arc::new(StateMachineHelpers::new(state_machine.clone()));

        // Create incoming call channel
        let (incoming_tx, incoming_rx) = mpsc::channel(config.incoming_call_channel_capacity);
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let lifecycle = config
            .server_call_capacity
            .map(LifecycleIndex::with_capacity)
            .unwrap_or_else(LifecycleIndex::new);
        let app_event_publisher = SessionEventPublisher::with_dispatcher(
            global_coordinator.clone(),
            lifecycle.clone(),
            config.session_event_dispatcher_workers,
            config.session_event_dispatcher_channel_capacity,
        );
        media_adapter
            .set_app_event_publisher(app_event_publisher.clone())
            .await;
        let fast_auto_accept_incoming_calls = config.fast_auto_accept_incoming_calls;
        let fast_auto_accept_queue_capacity = config.incoming_call_channel_capacity;

        let coordinator = Arc::new(Self {
            helpers,
            media_adapter: media_adapter.clone(),
            dialog_adapter: dialog_adapter.clone(),
            incoming_rx: Arc::new(RwLock::new(incoming_rx)),
            global_coordinator: global_coordinator.clone(),
            config,
            shutdown_tx,
            lifecycle: lifecycle.clone(),
            app_event_publisher: app_event_publisher.clone(),
            session_registry: registry.clone(),
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
                app_event_publisher.clone(),
                sip_trace_owner_id,
            )
            .with_fast_auto_accept_incoming_calls(
                fast_auto_accept_incoming_calls,
                fast_auto_accept_queue_capacity,
            );

        // SIP_API_DESIGN_2 Phase D — give the handler a weak handle
        // back to the coordinator so the bus-path `IncomingRegister`
        // branch can build a response-capable wrapper. Weak avoids
        // the circular ownership loop.
        event_handler.set_coordinator(&coordinator);

        // Start the event handler (sets up channels and subscriptions)
        event_handler.start(shutdown_rx).await?;

        Ok(coordinator)
    }

    pub(crate) fn fast_auto_accept_incoming_calls(&self) -> bool {
        self.config.fast_auto_accept_incoming_calls
    }

    // ===== Shutdown =====

    /// Shut down this coordinator and all its background tasks.
    ///
    /// This is a non-blocking best-effort shutdown. When
    /// [`Config::unregister_on_shutdown_timeout_secs`] is non-zero, active
    /// registrations are asked to unregister before the shutdown signal is
    /// sent. Use [`shutdown_gracefully`](Self::shutdown_gracefully) when the
    /// caller needs deterministic unregister completion.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>) {
    /// coordinator.shutdown();
    /// # }
    /// ```
    pub fn shutdown(&self) {
        let timeout = Duration::from_secs(self.config.unregister_on_shutdown_timeout_secs);
        let shutdown_tx = self.shutdown_tx.clone();
        let helpers = self.helpers.clone();
        let dialog_adapter = self.dialog_adapter.clone();

        if timeout.is_zero() {
            dialog_adapter.abort_all_registration_refreshes();
            let _ = shutdown_tx.send(true);
            return;
        }

        tokio::spawn(async move {
            Self::unregister_registered_sessions_best_effort(helpers, timeout).await;
            dialog_adapter.abort_all_registration_refreshes();
            let _ = shutdown_tx.send(true);
        });
    }

    /// Gracefully unregister active registrations, then stop background tasks.
    ///
    /// The timeout applies per registration. Pass `None` to use
    /// [`Config::unregister_on_shutdown_timeout_secs`]. A zero timeout skips
    /// unregister and behaves like an immediate shutdown.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>) -> rvoip_sip::Result<()> {
    /// coordinator.shutdown_gracefully(Some(std::time::Duration::from_secs(2))).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn shutdown_gracefully(&self, timeout: Option<Duration>) -> Result<()> {
        let timeout = timeout.unwrap_or_else(|| {
            Duration::from_secs(self.config.unregister_on_shutdown_timeout_secs)
        });
        if !timeout.is_zero() {
            self.unregister_registered_sessions(timeout).await;
        }
        self.dialog_adapter.abort_all_registration_refreshes();
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }

    async fn unregister_registered_sessions(&self, timeout: Duration) {
        let sessions = self.helpers.state_machine.store.get_all_sessions().await;
        for session in sessions {
            if !session.is_registered {
                continue;
            }
            let handle = RegistrationHandle {
                session_id: session.session_id.clone(),
            };
            if let Err(e) = self.unregister_and_wait(&handle, Some(timeout)).await {
                tracing::warn!(
                    "Graceful shutdown unregister failed for session {}: {}",
                    session.session_id,
                    e
                );
            }
        }
    }

    async fn unregister_registered_sessions_best_effort(
        helpers: Arc<StateMachineHelpers>,
        timeout: Duration,
    ) {
        let sessions = helpers.state_machine.store.get_all_sessions().await;
        for session in sessions {
            if !session.is_registered {
                continue;
            }
            let session_id = session.session_id.clone();
            let unregister = helpers
                .state_machine
                .process_event(&session_id, EventType::StartUnregistration);
            match tokio::time::timeout(timeout, unregister).await {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    tracing::warn!(
                        "Best-effort shutdown unregister failed for session {}: {}",
                        session_id,
                        e
                    );
                }
                Err(_) => {
                    tracing::warn!(
                        "Best-effort shutdown unregister timed out for session {}",
                        session_id
                    );
                }
            }
        }
    }

    /// Return a cloneable handle that can signal
    /// [`shutdown`](Self::shutdown) from another task. Mirrors
    /// [`CallbackPeer::shutdown_handle`].
    ///
    /// [`CallbackPeer::shutdown_handle`]: crate::api::callback_peer::CallbackPeer::shutdown_handle
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>) {
    /// let stop = coordinator.shutdown_handle();
    /// tokio::spawn(async move {
    ///     stop.shutdown();
    /// });
    /// # }
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>) -> rvoip_sip::Result<()> {
    /// let mut raw_events = coordinator.subscribe_events().await?;
    /// tokio::spawn(async move {
    ///     while let Some(_event) = raw_events.recv().await {
    ///         // Downcast to SessionApiCrossCrateEvent for diagnostics.
    ///     }
    /// });
    /// # Ok(())
    /// # }
    /// ```
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
    /// [`crate::api::events::Event`] values across all sessions and
    /// registration lifecycles owned by this coordinator.
    ///
    /// Use when a single consumer needs every session API event, for example
    /// a b2bua coordinator, activity log, or registration monitor. For
    /// per-leg call logic prefer [`events_for_session`][Self::events_for_session].
    ///
    /// The returned receiver already handles the downcast from the raw
    /// cross-crate broadcast and exposes filtering helpers like
    /// [`EventReceiver::next_dtmf`](crate::api::stream_peer::EventReceiver::next_dtmf),
    /// [`EventReceiver::next_incoming`](crate::api::stream_peer::EventReceiver::next_incoming),
    /// and
    /// [`EventReceiver::next_transfer`](crate::api::stream_peer::EventReceiver::next_transfer).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>) -> rvoip_sip::Result<()> {
    /// use rvoip_sip::Event;
    ///
    /// let mut events = coordinator.events().await?;
    /// if let Some(Event::RegistrationSuccess { registrar, .. }) = events.next().await {
    ///     println!("registered with {registrar}");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn events(&self) -> Result<crate::api::stream_peer::EventReceiver> {
        let rx = self.subscribe_events().await?;
        Ok(crate::api::stream_peer::EventReceiver::new(rx))
    }

    /// Return an [`EventReceiver`](crate::api::stream_peer::EventReceiver) that only yields events whose
    /// `call_id` matches `id`. Per-session filtering happens in the
    /// receiver's `next()` loop.
    ///
    /// Registration lifecycle events do not carry a call id, so they are only
    /// visible on [`events`](Self::events), not on per-session receivers.
    ///
    /// Intended for b2bua-style consumers that need to watch both legs of
    /// a bridged call independently:
    ///
    /// ```no_run
    /// # use rvoip_sip::{Event, SessionId, UnifiedCoordinator};
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
    ///
    /// # Examples
    ///
    /// See the b2bua-style event split above for a complete `tokio::select!`
    /// example.
    pub async fn events_for_session(
        &self,
        id: &SessionId,
    ) -> Result<crate::api::stream_peer::EventReceiver> {
        let rx = self.subscribe_events().await?;
        let mut receiver = crate::api::stream_peer::EventReceiver::filtered(rx, id.clone());

        // Race repair: SESSION_TO_APP_CHANNEL is broadcast — a subscriber
        // added here cannot observe events that fired before this call
        // returned. On a fast loopback, `invite → 200 OK → CallAnswered`
        // can complete in well under a millisecond, so callers that follow
        // the documented `invite().send() → events_for_session → wait for
        // CallAnswered` pattern would otherwise deadlock. Inspect the
        // session's *current* state and synthesize the events the caller
        // would have observed had they been subscribed earlier.
        if let Ok(state) = self.helpers.get_state(id).await {
            use crate::types::CallState;
            match state {
                CallState::Active
                | CallState::Bridged
                | CallState::OnHold
                | CallState::HoldPending
                | CallState::EarlyMedia
                | CallState::Resuming
                | CallState::Muted => {
                    receiver.prime(crate::api::events::Event::CallAnswered {
                        call_id: id.clone(),
                        sdp: None,
                    });
                }
                CallState::Failed(reason) => {
                    receiver.prime(crate::api::events::Event::CallFailed {
                        call_id: id.clone(),
                        reason: reason.to_string(),
                        status_code: 500,
                    });
                }
                CallState::Terminated | CallState::Cancelled => {
                    receiver.prime(crate::api::events::Event::CallEnded {
                        call_id: id.clone(),
                        reason: format!("session in state {state:?}"),
                    });
                }
                _ => {}
            }
        }

        Ok(receiver)
    }

    pub(crate) async fn lifecycle_snapshot(&self, id: &SessionId) -> CallLifecycleSnapshot {
        let (state, media_security) = match self.helpers.state_machine.store.get_session(id).await {
            Ok(session) => (Some(session.call_state), session.media_security),
            Err(_) => (None, None),
        };
        let mut snapshot = self.lifecycle.snapshot(id, state);
        if snapshot.media_security.is_none() {
            snapshot.media_security = media_security;
        }
        snapshot
    }

    pub(crate) fn lifecycle_watcher(&self, id: &SessionId) -> tokio::sync::watch::Receiver<u64> {
        self.lifecycle.watcher(id)
    }

    #[cfg(test)]
    pub(crate) async fn publish_app_event_for_test(
        &self,
        event: crate::api::events::Event,
    ) -> Result<()> {
        self.app_event_publisher.publish_now(event).await
    }

    // ===== Simple Call Operations =====

    /// Spawn an outbound leg linked to a transferor session for RFC 3515
    /// §2.4.5 progress reporting. The new leg's `SessionState` carries
    /// `transferor_session_id = Some(..)` before the state machine
    /// dispatches `MakeCall`, so every subsequent `Dialog180Ringing` /
    /// `Dialog200OK` / failure fires a progress NOTIFY back on the
    /// transferor's REFER subscription. This is the b2bua wrapper crate's
    /// primary REFER-forwarding entry point.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, transferor: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// let new_leg = coordinator.make_transfer_leg(
    ///     "sip:service@127.0.0.1:5060",
    ///     "sip:target@example.com",
    ///     &transferor,
    /// ).await?;
    /// # let _ = new_leg;
    /// # Ok(())
    /// # }
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, leg: rvoip_sip::SessionId, transferor: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// coordinator.set_transferor_session(&leg, &transferor).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn set_transferor_session(
        &self,
        leg_session_id: &SessionId,
        transferor_session_id: &SessionId,
    ) -> Result<()> {
        self.helpers
            .set_transferor_session(leg_session_id, transferor_session_id)
            .await
    }

    /// Accept an incoming call.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, incoming: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// coordinator.accept_call(&incoming).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn accept_call(&self, session_id: &SessionId) -> Result<()> {
        self.helpers.accept_call(session_id).await
    }

    /// Accept an incoming call with a caller-supplied SDP answer. Bypasses
    /// local media negotiation — intended for b2bua flows where the answer
    /// body comes from the outbound leg's 200 OK. See
    /// [`StateMachineHelpers::accept_call_with_sdp`] for the mechanism.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, incoming: rvoip_sip::SessionId, answer_sdp: String) -> rvoip_sip::Result<()> {
    /// coordinator.accept_call_with_sdp(&incoming, answer_sdp).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn accept_call_with_sdp(&self, session_id: &SessionId, sdp: String) -> Result<()> {
        self.helpers.accept_call_with_sdp(session_id, sdp).await
    }

    /// Hang up or cancel a call.
    ///
    /// Established calls send BYE. Ringing or early-media outbound calls send
    /// CANCEL and do not publish `CallCancelled` until the INVITE reaches a
    /// terminal outcome. If the outbound INVITE has not received a provisional
    /// response yet, cancel intent is recorded and CANCEL is sent only if it
    /// later becomes legal; a fast 200 OK is ACKed and immediately BYE-cleaned.
    /// Use [`SessionHandle::hangup_and_wait`](crate::api::handle::SessionHandle::hangup_and_wait)
    /// when the caller needs to wait for the terminal API event.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// let mut events = coordinator.events_for_session(&call_id).await?;
    /// coordinator.hangup(&call_id).await?;
    /// // Wait for Event::CallEnded / CallFailed / CallCancelled if needed.
    /// # let _ = events.next().await;
    /// # Ok(())
    /// # }
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, a: rvoip_sip::SessionId, b: rvoip_sip::SessionId) -> Result<(), rvoip_sip::BridgeError> {
    /// let bridge = coordinator.bridge(&a, &b).await?;
    /// // Keep `bridge` alive for as long as the RTP relay should run.
    /// drop(bridge);
    /// # Ok(())
    /// # }
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, incoming: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// coordinator.send_early_media(&incoming, None).await?;
    /// coordinator.accept_call(&incoming).await?;
    /// # Ok(())
    /// # }
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// use rvoip_sip::AudioSource;
    ///
    /// coordinator.set_audio_source(
    ///     &call_id,
    ///     AudioSource::Tone { frequency: 440.0, amplitude: 0.4 },
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn set_audio_source(
        &self,
        session_id: &SessionId,
        source: AudioSource,
    ) -> Result<()> {
        self.media_adapter
            .set_audio_source(session_id, source)
            .await
    }

    /// Put a call on hold.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// coordinator.hold(&call_id).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn hold(&self, session_id: &SessionId) -> Result<()> {
        self.helpers
            .state_machine
            .process_event(session_id, EventType::HoldCall)
            .await?;
        Ok(())
    }

    /// Resume a call from hold.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// coordinator.resume(&call_id).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn resume(&self, session_id: &SessionId) -> Result<()> {
        self.helpers
            .state_machine
            .process_event(session_id, EventType::ResumeCall)
            .await?;
        Ok(())
    }

    // ===== Conference Operations =====

    /// Create a conference from an active call.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, host: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// coordinator.create_conference(&host, "support-bridge").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn create_conference(&self, session_id: &SessionId, name: &str) -> Result<()> {
        self.helpers.create_conference(session_id, name).await
    }

    /// Add a participant to a conference hosted by another active session.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, host: rvoip_sip::SessionId, participant: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// coordinator.add_to_conference(&host, &participant).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn add_to_conference(
        &self,
        host_session_id: &SessionId,
        participant_session_id: &SessionId,
    ) -> Result<()> {
        self.helpers
            .add_to_conference(host_session_id, participant_session_id)
            .await
    }

    /// Join an existing conference by conference id.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// coordinator.join_conference(&call_id, "support-bridge").await?;
    /// # Ok(())
    /// # }
    /// ```
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

    /// Materialize a [`SessionHandle`](crate::api::handle::SessionHandle)
    /// for an existing call_id.
    ///
    /// Returns a handle for invoking control APIs (hangup, hold, resume,
    /// DTMF, …) on a session created via the canonical builder chain
    /// (`coord.invite(...).send()` returns a [`CallId`](crate::api::handle::CallId);
    /// pair it with this helper to get the rich `SessionHandle` for
    /// in-call control).
    pub fn session(
        self: &Arc<Self>,
        call_id: &crate::api::handle::CallId,
    ) -> crate::api::handle::SessionHandle {
        crate::api::handle::SessionHandle::new(call_id.clone(), self.clone())
    }

    /// Terminate the current session tracked by the session store.
    ///
    /// This is an advanced compatibility helper for single-session flows. New
    /// code should usually hold the specific [`SessionId`] and call
    /// [`hangup`](Self::hangup).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>) -> rvoip_sip::Result<()> {
    /// coordinator.terminate_current_session().await?;
    /// # Ok(())
    /// # }
    /// ```
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

    /// Accept a pending inbound REFER request and send RFC 3515 acceptance
    /// responses/NOTIFYs through the state machine.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// // Call this after receiving Event::ReferReceived for `call_id`.
    /// coordinator.accept_refer(&call_id).await?;
    /// # Ok(())
    /// # }
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// coordinator.reject_refer(&call_id, 603, "Decline").await?;
    /// # Ok(())
    /// # }
    /// ```
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

    /// Send a REFER progress NOTIFY with a SIP status code and reason.
    ///
    /// This is the low-level helper for custom REFER orchestration. Transfer
    /// legs created with [`make_transfer_leg`](Self::make_transfer_leg)
    /// emit ordinary REFER progress automatically.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// coordinator.send_refer_notify(&call_id, 180, "Ringing").await?;
    /// # Ok(())
    /// # }
    /// ```
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

    /// Fetch the SIP-level identity (`Call-ID`, local/remote tags) of a
    /// session's dialog. Returns `None` if the dialog isn't established
    /// yet or has already been cleaned up.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// if let Some(identity) = coordinator.dialog_identity(&call_id).await? {
    ///     if let Some(replaces) = identity.to_replaces_value() {
    ///         println!("Replaces value: {replaces}");
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// coordinator.send_dtmf(&call_id, '5').await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_dtmf(&self, session_id: &SessionId, digit: char) -> Result<()> {
        self.media_adapter
            .send_dtmf_rfc4733(session_id, digit, 100)
            .await
    }

    // ===== Recording Operations =====

    /// Start recording a call.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// coordinator.start_recording(&call_id).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn start_recording(&self, session_id: &SessionId) -> Result<()> {
        self.helpers
            .state_machine
            .process_event(session_id, EventType::StartRecording)
            .await?;
        Ok(())
    }

    /// Stop recording a call.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// coordinator.stop_recording(&call_id).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn stop_recording(&self, session_id: &SessionId) -> Result<()> {
        self.helpers
            .state_machine
            .process_event(session_id, EventType::StopRecording)
            .await?;
        Ok(())
    }

    // ===== Query Operations =====

    /// Get detailed session information.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// let info = coordinator.get_session_info(&call_id).await?;
    /// println!("session state: {:?}", info.state);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_session_info(&self, session_id: &SessionId) -> Result<SessionInfo> {
        self.helpers.get_session_info(session_id).await
    }

    /// List all active sessions.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>) {
    /// let sessions = coordinator.list_sessions().await;
    /// println!("active sessions: {}", sessions.len());
    /// # }
    /// ```
    pub async fn list_sessions(&self) -> Vec<SessionInfo> {
        self.helpers.list_sessions().await
    }

    /// Get the current state of a session.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// let state = coordinator.get_state(&call_id).await?;
    /// println!("call state: {state:?}");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_state(&self, session_id: &SessionId) -> Result<CallState> {
        self.helpers.get_state(session_id).await
    }

    /// Check whether a session is in a conference.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// if coordinator.is_in_conference(&call_id).await? {
    ///     println!("call is in a conference");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn is_in_conference(&self, session_id: &SessionId) -> Result<bool> {
        self.helpers.is_in_conference(session_id).await
    }

    // ===== Audio Operations =====

    /// Subscribe to decoded audio frames for a session.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// let mut audio = coordinator.subscribe_to_audio(&call_id).await?;
    /// tokio::spawn(async move {
    ///     while let Some(frame) = audio.receiver.recv().await {
    ///         println!("received {} samples", frame.samples.len());
    ///     }
    /// });
    /// # Ok(())
    /// # }
    /// ```
    pub async fn subscribe_to_audio(
        &self,
        session_id: &SessionId,
    ) -> Result<crate::types::AudioFrameSubscriber> {
        self.media_adapter
            .subscribe_to_audio_frames(session_id)
            .await
    }

    /// Send an encoded/decoded audio frame to a session's media path.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) -> rvoip_sip::Result<()> {
    /// let frame = rvoip_media_core::types::AudioFrame::new(vec![0i16; 160], 8000, 1, 0);
    /// coordinator.send_audio(&call_id, frame).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_audio(&self, session_id: &SessionId, frame: AudioFrame) -> Result<()> {
        self.media_adapter.send_audio_frame(session_id, frame).await
    }

    // ===== Event Subscriptions =====

    /// Subscribe a callback to low-level state-machine session events.
    ///
    /// This is an advanced compatibility hook. New application code should
    /// prefer [`events`](Self::events) or [`events_for_session`](Self::events_for_session).
    ///
    /// Renamed from `subscribe(...)` per SIP_API_DESIGN_2.md Phase 12: the
    /// bare `subscribe` entry now names the SUBSCRIBE-method verb-builder.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) {
    /// coordinator.on_session_events(call_id, |_event| {
    ///     // Observe low-level state-machine events.
    /// }).await;
    /// # }
    /// ```
    pub async fn on_session_events<F>(&self, session_id: SessionId, callback: F)
    where
        F: Fn(crate::state_machine::helpers::SessionEvent) + Send + Sync + 'static,
    {
        self.helpers.subscribe(session_id, callback).await
    }

    /// Unsubscribe from low-level state-machine events for a session.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, call_id: rvoip_sip::SessionId) {
    /// coordinator.unsubscribe(&call_id).await;
    /// # }
    /// ```
    pub async fn unsubscribe(&self, session_id: &SessionId) {
        self.helpers.unsubscribe(session_id).await
    }

    // ===== Incoming Call Handling =====

    /// Get the next low-level incoming call notification.
    ///
    /// This is the coordinator primitive underneath
    /// [`StreamPeer::wait_for_incoming`](crate::api::stream_peer::StreamPeer::wait_for_incoming).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>) {
    /// if let Some(incoming) = coordinator.get_incoming_call().await {
    ///     println!("incoming call from {}", incoming.from);
    /// }
    /// # }
    /// ```
    pub async fn get_incoming_call(&self) -> Option<IncomingCallInfo> {
        self.incoming_rx.write().await.recv().await
    }

    // ===== Auto-Transfer Handling =====

    /// Enable automatic blind transfer handling - DISABLED
    /// Auto-transfer now handled in SessionEventHandler to avoid event stealing
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>) {
    /// coordinator.enable_auto_transfer();
    /// # }
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>) -> rvoip_sip::Result<()> {
    /// let users = std::collections::HashMap::from([
    ///     ("alice".to_string(), "secret".to_string()),
    /// ]);
    /// let registrar = coordinator.start_registration_server("example.com", users).await?;
    /// # let _ = registrar;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn start_registration_server(
        &self,
        realm: &str,
        users: std::collections::HashMap<String, String>,
    ) -> Result<Arc<rvoip_sip_registrar::RegistrarService>> {
        use crate::adapters::RegistrationAdapter;
        use rvoip_sip_registrar::{api::ServiceMode, types::RegistrarConfig, RegistrarService};

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
        sip_trace_owner_id: Option<String>,
    ) -> Result<Arc<rvoip_sip_dialog::api::unified::UnifiedDialogApi>> {
        use rvoip_sip_dialog::api::unified::UnifiedDialogApi;
        use rvoip_sip_dialog::config::DialogManagerConfig;
        use rvoip_sip_dialog::transaction::{
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
                rvoip_sip_dialog::transaction::transport::TlsRole::ClientAndServer
            }
            SipTlsMode::ClientOnly => rvoip_sip_dialog::transaction::transport::TlsRole::ClientOnly,
            SipTlsMode::ServerOnly => rvoip_sip_dialog::transaction::transport::TlsRole::ServerOnly,
            SipTlsMode::ClientAndServer => {
                rvoip_sip_dialog::transaction::transport::TlsRole::ClientAndServer
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
            default_channel_capacity: config.sip_transport_channel_capacity,
            udp_recv_buffer_size: config.sip_udp_recv_buffer_size,
            udp_send_buffer_size: config.sip_udp_send_buffer_size,
            udp_parse_workers: config.sip_udp_parse_workers,
            udp_parse_queue_capacity: config.sip_udp_parse_queue_capacity,
            udp_parse_dispatch: config.sip_udp_parse_dispatch,
            transport_event_dispatch_workers: config.sip_transport_dispatch_workers,
            transport_event_dispatch_queue_capacity: config.sip_transport_dispatch_queue_capacity,
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

        if let Some(owner_id) = sip_trace_owner_id {
            // SIP_API_DESIGN_2 §12.4 — when a TraceRedactor is configured,
            // build a closure that walks each rendered SIP message header
            // line and delegates per-header decisions to the redactor.
            // The transform runs at the trace boundary in
            // SipTraceRuntime::publish; the wire form is unaffected.
            let redactor_fn: Option<rvoip_sip_dialog::transaction::transport::TraceRedactorFn> =
                config.trace_redaction.as_ref().map(|redactor| {
                    let redactor = redactor.clone();
                    let f: rvoip_sip_dialog::transaction::transport::TraceRedactorFn =
                        Arc::new(move |raw: &str| -> String {
                            crate::api::trace_redactor::apply_message_redactor(
                                redactor.as_ref(),
                                raw,
                            )
                        });
                    f
                });

            transport_manager.enable_sip_trace_with_redactor(
                owner_id,
                config.sip_trace.clone(),
                global_coordinator.clone(),
                redactor_fn,
            );
        }

        // Initialize the transport manager
        transport_manager.initialize().await.map_err(|e| {
            SessionError::InternalError(format!("Failed to initialize transport: {}", e))
        })?;

        // Create transaction manager using transport manager
        let (transaction_manager, event_rx) =
            TransactionManager::with_transport_manager_and_index_capacity_and_dispatch(
                transport_manager,
                transport_event_rx,
                Some(config.transaction_event_channel_capacity),
                Some(config.transaction_index_capacity_hint()),
                config.sip_transaction_dispatch_workers,
                config.sip_transaction_dispatch_queue_capacity,
            )
            .await
            .map_err(|e| {
                SessionError::InternalError(format!("Failed to create transaction manager: {}", e))
            })?;

        let mut transaction_manager = transaction_manager;
        transaction_manager.set_auto_100_trying(config.auto_100_trying);
        if let Some(max_burst) = config.sip_transaction_dispatch_priority_burst_max {
            transaction_manager.set_transaction_dispatch_priority_burst_max(max_burst);
        }
        if let Some(max_due_per_tick) = config.sip_invite_2xx_retransmit_max_due_per_tick {
            transaction_manager.set_invite_2xx_retransmit_max_due_per_tick(max_due_per_tick);
        }
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
                dialog.max_dialogs = Some(config.dialog_index_capacity_hint());
                dialog.event_dispatch_workers = config.sip_dialog_dispatch_workers;
                dialog.event_dispatch_queue_capacity = config.sip_dialog_dispatch_queue_capacity;
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
        let controller = Arc::new(MediaSessionController::with_port_range_and_capacity(
            config.media_port_start,
            config.media_port_end,
            config
                .media_session_capacity
                .or(config.server_call_capacity)
                .unwrap_or(0),
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

/// SIP_API_DESIGN_2 §7.1 — builder → state-machine dispatch helpers.
///
/// These two thin wrappers form the canonical builder send path. Builders
/// call [`UnifiedCoordinator::stage_outbound_options`] to write the matching
/// `pending_<method>_options` slot under the §7.3 invariant #5 conflict
/// guard, then call [`UnifiedCoordinator::dispatch_outbound`] with the
/// matching `EventType::SendOutbound<METHOD>` to drive the state table.
/// The action handler reads from the stash; the final-response transition
/// (`ClearPending*Options`) drops it.
impl UnifiedCoordinator {
    /// Stage a pending-options snapshot on the session and check the
    /// single-in-flight conflict guard. Returns
    /// `Err(SessionError::Conflict { method })` when a prior `.send()`
    /// for the same method on the same session has not yet reached its
    /// final response. See
    /// [`StateMachine::stage_outbound_options`](crate::state_machine::executor::StateMachine::stage_outbound_options).
    pub async fn stage_outbound_options(
        &self,
        session_id: &SessionId,
        slot: crate::state_machine::executor::PendingOptionsSlot,
    ) -> Result<()> {
        self.helpers
            .state_machine
            .stage_outbound_options(session_id, slot)
            .await
            .map_err(|e| {
                if let Ok(typed) = e.downcast::<SessionError>() {
                    *typed
                } else {
                    SessionError::InternalError(
                        "stage_outbound_options: state-machine error".to_string(),
                    )
                }
            })
    }

    /// Queue a state-machine event on the session's event queue and run
    /// the resulting transition. Thin wrapper over
    /// [`StateMachine::process_event`].
    pub async fn dispatch_outbound(
        &self,
        session_id: &SessionId,
        event: crate::state_table::EventType,
    ) -> Result<crate::state_machine::executor::ProcessEventResult> {
        self.helpers
            .state_machine
            .process_event(session_id, event)
            .await
            .map_err(|e| SessionError::InternalError(format!("dispatch_outbound: {}", e)))
    }

    /// Crate-internal accessor: read the current `SessionState` snapshot for
    /// the given session id. Used by refresh-style builders that need to
    /// reuse registration / dialog identifiers from the original send.
    pub(crate) async fn session_state(
        &self,
        session_id: &SessionId,
    ) -> Result<crate::session_store::SessionState> {
        self.helpers
            .state_machine
            .store
            .get_session(session_id)
            .await
            .map_err(|_| SessionError::SessionNotFound(session_id.to_string()))
    }

    /// Crate-internal: write back a modified `SessionState`. Used by
    /// response builders to stash extras (`Retry-After`, `Warning`,
    /// `WWW-Authenticate`, …) on the session before firing the
    /// state-machine event that consumes them.
    pub(crate) async fn update_session_state(
        &self,
        session: crate::session_store::SessionState,
    ) -> Result<()> {
        self.helpers
            .state_machine
            .store
            .update_session(session)
            .await
            .map_err(|e| SessionError::InternalError(format!("update_session: {}", e)))
    }
}

/// Simple helper to create a session and make a call
impl UnifiedCoordinator {
    /// Quick method to create a UAC session and make a call
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>) -> rvoip_sip::Result<()> {
    /// let call_id = coordinator
    ///     .quick_call("sip:alice@127.0.0.1:5060", "sip:bob@127.0.0.1:5070")
    ///     .await?;
    /// # let _ = call_id;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn quick_call(self: &Arc<Self>, from: &str, to: &str) -> Result<SessionId> {
        self.invite(Some(from.to_string()), to.to_string())
            .send()
            .await
    }
}

/// Registration API
impl UnifiedCoordinator {
    /// Internal REGISTER dispatch used by
    /// [`RegisterBuilder`](crate::api::send::RegisterBuilder).
    ///
    /// When `extra_headers` is non-empty we follow SIP_API_DESIGN_2 §10 #19
    /// and stash a `RegisterRequestOptions` on the session *before* the
    /// `StartRegistration` event fires, so `execute_register_action`
    /// (`state_machine/actions.rs`) reads the slice on the very first
    /// dispatch (not just on the 401-retry).
    ///
    /// When `extra_headers` is empty we skip the stash entirely. Stashing
    /// here would occupy `pending_register_options`; the slot is only
    /// cleared once the first REGISTER reaches a final response, so a
    /// caller that fires a `RegisterRefreshBuilder::send` before that
    /// would race the stash check in
    /// [`stage_outbound_options`](crate::state_machine::executor::StateMachineExecutor::stage_outbound_options)
    /// and get back `SessionError::Conflict { method: Register }`.
    pub(crate) async fn register_with_extras(
        &self,
        registrar_uri: &str,
        from_uri: &str,
        contact_uri: &str,
        username: &str,
        password: &str,
        expires: u32,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> Result<RegistrationHandle> {
        let session_id = SessionId::new();
        self.helpers
            .create_session(
                session_id.clone(),
                from_uri.to_string(),
                registrar_uri.to_string(),
                crate::state_table::types::Role::UAC,
            )
            .await?;

        let credentials = crate::types::Credentials::new(username, password);

        let session_store = &self.helpers.state_machine.store;
        let mut session = session_store.get_session(&session_id).await?;
        session.credentials = Some(credentials);
        session.registrar_uri = Some(registrar_uri.to_string());
        session.registration_contact = Some(contact_uri.to_string());
        session.registration_expires = Some(expires);

        if !extra_headers.is_empty() {
            session.pending_register_options = Some(std::sync::Arc::new(
                rvoip_sip_dialog::api::unified::RegisterRequestOptions {
                    registrar_uri: registrar_uri.to_string(),
                    aor_uri: from_uri.to_string(),
                    contact_uri: contact_uri.to_string(),
                    expires,
                    authorization: None,
                    call_id: None,
                    cseq: None,
                    outbound_contact: None,
                    outbound_proxy_uri: None,
                    extra_headers,
                    refresh: false,
                },
            ));
        }
        session_store.update_session(session).await?;

        let _ = self
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

    /// Unregister from the SIP server.
    ///
    /// Sends REGISTER with `Expires: 0` to remove the binding and aborts any
    /// pending automatic refresh task when the registrar confirms success.
    /// This method returns after the state machine accepts the request. Use
    /// [`unregister_and_wait`](Self::unregister_and_wait) when the caller must
    /// wait for `UnregistrationSuccess` or `UnregistrationFailed`.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, handle: rvoip_sip::RegistrationHandle) -> rvoip_sip::Result<()> {
    /// coordinator.unregister(&handle).await?;
    /// # Ok(())
    /// # }
    /// ```
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

    /// Refresh registration before it expires.
    ///
    /// Sends a new REGISTER request using the stored registration expiry and
    /// registration Call-ID. Successful refresh responses replace the stored
    /// accepted expiry and next automatic refresh time.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, handle: rvoip_sip::RegistrationHandle) -> rvoip_sip::Result<()> {
    /// coordinator.refresh_registration(&handle).await?;
    /// # Ok(())
    /// # }
    /// ```
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

    /// SIP_API_DESIGN_2 §3.3 — Begin a manual REGISTER refresh on the
    /// given registration handle. Returns a `RegisterRefreshBuilder`
    /// that supports `.with_expires(n)` and ships via `.send().await`.
    pub fn refresh(
        self: &Arc<Self>,
        handle: &RegistrationHandle,
    ) -> crate::api::send::RegisterRefreshBuilder {
        crate::api::send::RegisterRefreshBuilder::new(self.clone(), handle.clone())
    }

    /// Return whether the registration is currently marked registered.
    ///
    /// This is a coarse boolean for simple clients. Use
    /// [`registration_info`](Self::registration_info) for status, accepted
    /// expiry, next refresh timing, failure metadata, Service-Route, GRUU, and
    /// outbound-flow information.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, handle: rvoip_sip::RegistrationHandle) -> rvoip_sip::Result<()> {
    /// if coordinator.is_registered(&handle).await? {
    ///     println!("registration is active");
    /// }
    /// # Ok(())
    /// # }
    /// ```
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

    /// Return detailed registration lifecycle information for a handle.
    ///
    /// `accepted_expires_secs`, `registered_at`, and `next_refresh_at` are
    /// populated from successful REGISTER responses. `service_route`,
    /// `pub_gruu`, and `temp_gruu` are populated when supplied by the
    /// registrar. Failure and unregister paths clear refresh metadata and keep
    /// a stable status snapshot for diagnostics.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, handle: rvoip_sip::RegistrationHandle) -> rvoip_sip::Result<()> {
    /// let info = coordinator.registration_info(&handle).await?;
    /// println!("status: {:?}", info.status);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn registration_info(&self, handle: &RegistrationHandle) -> Result<RegistrationInfo> {
        let session = self
            .helpers
            .state_machine
            .store
            .get_session(&handle.session_id)
            .await?;

        let status = if session.is_registered {
            RegistrationStatus::Registered
        } else {
            match session.call_state {
                CallState::Registering => RegistrationStatus::Registering,
                CallState::Unregistering => RegistrationStatus::Unregistering,
                _ if session.registration_last_failure.is_some() => RegistrationStatus::Failed,
                _ if session.registration_retry_count > 0 => RegistrationStatus::Failed,
                _ => RegistrationStatus::Unregistered,
            }
        };

        let next_refresh_in = session
            .registration_next_refresh_at
            .map(|when| when.saturating_duration_since(Instant::now()));

        let last_failure = if let Some(reason) = session.registration_last_failure.clone() {
            Some(reason)
        } else if matches!(status, RegistrationStatus::Failed) {
            Some(format!(
                "registration failed after {} retry attempt(s)",
                session.registration_retry_count
            ))
        } else {
            None
        };

        let aor = session.local_uri.clone();
        let (dialog_service_route, dialog_pub_gruu, dialog_temp_gruu, outbound_flow_active) =
            if let Some(aor) = aor.as_deref() {
                let dialog_service_route = self
                    .dialog_adapter
                    .dialog_api
                    .service_route_for_aor(aor)
                    .await
                    .map(|uris| uris.into_iter().map(|uri| uri.to_string()).collect());
                let gruu = self.dialog_adapter.dialog_api.gruu_for_aor(aor).await;
                let outbound_flow_active = self
                    .dialog_adapter
                    .dialog_api
                    .outbound_flow_active_for_aor(aor);
                (
                    dialog_service_route,
                    gruu.as_ref().and_then(|params| params.pub_gruu.clone()),
                    gruu.and_then(|params| params.temp_gruu),
                    outbound_flow_active,
                )
            } else {
                (None, None, None, false)
            };
        let service_route = session
            .registration_service_route
            .clone()
            .or(dialog_service_route);
        let pub_gruu = session.registration_pub_gruu.clone().or(dialog_pub_gruu);
        let temp_gruu = session.registration_temp_gruu.clone().or(dialog_temp_gruu);

        Ok(RegistrationInfo {
            session_id: handle.session_id.clone(),
            status,
            registrar: session.registrar_uri.clone(),
            contact: session.registration_contact.clone(),
            expires_secs: session.registration_expires,
            next_refresh_in,
            retry_count: session.registration_retry_count,
            last_failure,
            accepted_expires_secs: session.registration_accepted_expires,
            registered_at: session.registration_registered_at,
            next_refresh_at: session.registration_next_refresh_at,
            service_route,
            pub_gruu,
            temp_gruu,
            outbound_flow_active,
        })
    }

    /// Unregister and wait for the matching registration lifecycle event.
    ///
    /// This subscribes to the coordinator event stream before sending
    /// unregister, then returns after `UnregistrationSuccess` or converts
    /// `UnregistrationFailed` into an error. Registration events are global
    /// coordinator events, not per-registration handle streams.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>, handle: rvoip_sip::RegistrationHandle) -> rvoip_sip::Result<()> {
    /// coordinator
    ///     .unregister_and_wait(&handle, Some(std::time::Duration::from_secs(3)))
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn unregister_and_wait(
        &self,
        handle: &RegistrationHandle,
        timeout: Option<std::time::Duration>,
    ) -> Result<()> {
        let registrar = self
            .registration_info(handle)
            .await?
            .registrar
            .unwrap_or_default();
        let mut events = self.events().await?;
        self.unregister(handle).await?;

        let fut = async {
            loop {
                match events.next().await {
                    Some(crate::api::events::Event::UnregistrationSuccess { registrar: r })
                        if registrar.is_empty() || r == registrar =>
                    {
                        return Ok(());
                    }
                    Some(crate::api::events::Event::UnregistrationFailed {
                        registrar: r,
                        reason,
                    }) if registrar.is_empty() || r == registrar => {
                        return Err(SessionError::Other(format!(
                            "Unregistration failed for {}: {}",
                            r, reason
                        )));
                    }
                    Some(_) => {}
                    None => {
                        return Err(SessionError::Other(
                            "Event channel closed while waiting for unregister".to_string(),
                        ));
                    }
                }
            }
        };

        match timeout {
            Some(duration) => tokio::time::timeout(duration, fut)
                .await
                .map_err(|_| SessionError::Timeout("unregister_and_wait timed out".to_string()))?,
            None => fut.await,
        }
    }
}

/// Handle for managing a registration.
///
/// Registration lifecycle events are emitted through
/// [`UnifiedCoordinator::events`] and [`UnifiedCoordinator::events_for_session`].
/// This handle deliberately does not expose a separate event stream today,
/// because doing so cleanly would require a per-registration event bus split.
#[derive(Debug, Clone)]
pub struct RegistrationHandle {
    /// Session id backing this registration lifecycle.
    pub session_id: SessionId,
}

/// Coarse registration lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationStatus {
    /// REGISTER has been sent and the registrar response is still pending.
    Registering,
    /// Registrar accepted the binding and the contact is currently active.
    Registered,
    /// REGISTER with `Expires: 0` has been sent and the registrar response is pending.
    Unregistering,
    /// No active binding is known for this registration handle.
    Unregistered,
    /// The most recent registration or refresh attempt failed.
    Failed,
}

/// Query result for a registration handle.
///
/// This is a snapshot of the current client-side registration lifecycle. It
/// combines rvoip-sip state with metadata learned from dialog-core REGISTER
/// responses.
#[derive(Debug, Clone)]
pub struct RegistrationInfo {
    /// Session id backing this registration lifecycle.
    pub session_id: SessionId,
    /// Coarse lifecycle status.
    pub status: RegistrationStatus,
    /// Registrar URI originally used for REGISTER.
    pub registrar: Option<String>,
    /// Contact URI currently associated with the registration.
    pub contact: Option<String>,
    /// Last expiry value rvoip-sip will request on refresh.
    pub expires_secs: Option<u32>,
    /// Duration until the currently scheduled automatic refresh.
    pub next_refresh_in: Option<Duration>,
    /// Number of retry attempts used by the current/last registration flow.
    pub retry_count: u32,
    /// Last failure summary, if the lifecycle is failed.
    pub last_failure: Option<String>,
    /// Expiry accepted by the registrar in the most recent successful 2xx.
    pub accepted_expires_secs: Option<u32>,
    /// Local time when the most recent successful registration completed.
    pub registered_at: Option<Instant>,
    /// Local time when automatic refresh is scheduled.
    pub next_refresh_at: Option<Instant>,
    /// Registrar-provided Service-Route URIs, if supplied.
    pub service_route: Option<Vec<String>>,
    /// Registrar-assigned public GRUU, if supplied.
    pub pub_gruu: Option<String>,
    /// Registrar-assigned temporary GRUU, if supplied.
    pub temp_gruu: Option<String>,
    /// Whether dialog-core currently has an RFC 5626 outbound flow monitor.
    pub outbound_flow_active: bool,
}

/// Configuration for SIP registration.
///
/// Use [`Registration::new()`] for the common case where `from_uri` and
/// `contact_uri` are derived from the peer's [`Config`].
///
/// # Example
///
/// ```
/// use rvoip_sip::Registration;
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
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_sip::Registration;
    ///
    /// let reg = Registration::new("sip:registrar.example.com", "alice", "secret");
    /// assert_eq!(reg.expires, 3600);
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_sip::Registration;
    ///
    /// let reg = Registration::new("sip:registrar.example.com", "alice", "secret")
    ///     .expires(600);
    /// assert_eq!(reg.expires, 600);
    /// ```
    pub fn expires(mut self, secs: u32) -> Self {
        self.expires = secs;
        self
    }

    /// Override the From URI (defaults to the peer's local URI).
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_sip::Registration;
    ///
    /// let reg = Registration::new("sip:registrar.example.com", "alice", "secret")
    ///     .from_uri("sip:alice@example.com");
    /// assert_eq!(reg.from_uri.as_deref(), Some("sip:alice@example.com"));
    /// ```
    pub fn from_uri(mut self, uri: impl Into<String>) -> Self {
        self.from_uri = Some(uri.into());
        self
    }

    /// Override the Contact URI (defaults to the peer's local URI).
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_sip::Registration;
    ///
    /// let reg = Registration::new("sip:registrar.example.com", "alice", "secret")
    ///     .contact_uri("sip:alice@192.168.1.50:5060");
    /// assert_eq!(reg.contact_uri.as_deref(), Some("sip:alice@192.168.1.50:5060"));
    /// ```
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
