use rvoip_core::capability::CapabilityDescriptor;
use serde::{Deserialize, Serialize};

use crate::identity::DtlsFingerprint;

/// Policy applied to inbound trickle ICE candidates whose hostname ends in
/// `.local` (RFC 8839 mDNS-style anonymized candidates).
///
/// Server-side reachability of `.local` hostnames requires an mDNS resolver
/// reachable on the same broadcast domain as the client — usually false for
/// a hosted server. Default is `Drop`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MdnsCandidatePolicy {
    /// Silently drop `.local` candidates. Safe default for hosted servers.
    #[default]
    Drop,
    /// Forward `.local` candidates to webrtc-rs verbatim (will fail to
    /// resolve unless mDNS reachability is in place).
    Pass,
}

impl MdnsCandidatePolicy {
    /// Returns `true` if the candidate string contains a `.local` hostname
    /// in the typical RFC 8839 position (`candidate:<...> typ host`).
    pub fn is_mdns_candidate(candidate: &str) -> bool {
        // Quick substring match — the canonical form is
        //   "candidate:<foundation> <component> <proto> <prio> <hostname> <port> typ host ..."
        // and the hostname always ends in ".local" for mDNS candidates.
        candidate
            .split_whitespace()
            .any(|tok| tok.ends_with(".local") || tok.ends_with(".local."))
    }
}

/// STUN/TURN server entry with optional long-term credentials.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct IceServerConfig {
    pub urls: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
}

impl IceServerConfig {
    pub fn stun(url: impl Into<String>) -> Self {
        Self {
            urls: vec![url.into()],
            username: None,
            credential: None,
        }
    }

    pub fn turn(url: impl Into<String>, username: impl Into<String>, credential: impl Into<String>) -> Self {
        Self {
            urls: vec![url.into()],
            username: Some(username.into()),
            credential: Some(credential.into()),
        }
    }
}

/// ICE / media configuration shared by peer connections and the adapter.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WebRtcConfig {
    /// UDP bind address passed to `PeerConnectionBuilder::with_udp_addrs`.
    /// Use `"0.0.0.0:0"` or `"127.0.0.1:0"` for ephemeral ports.
    pub udp_bind: String,

    /// STUN/TURN servers (username/credential for TURN relay).
    #[serde(default)]
    pub ice_servers: Vec<IceServerConfig>,

    /// Maximum time to wait for ICE gathering to complete (full SDP, no trickle).
    #[serde(default = "default_gather_timeout_secs")]
    pub gather_timeout_secs: u64,

    /// Default capabilities advertised by [`crate::adapter::WebRtcAdapter`].
    #[serde(default = "default_capabilities")]
    pub capabilities: CapabilityDescriptor,

    /// Capacity of the per-peer internal handler event channels (ICE, track, DC, state).
    /// A larger value tolerates bigger bursts before dropping; default is 256.
    #[serde(default = "default_handler_channel_capacity")]
    pub handler_channel_capacity: usize,

    /// Deadline applied to inbound media-pump sends. If a downstream consumer can't
    /// drain a `MediaFrame` within this window the frame is dropped and a counter
    /// is incremented (avoids stalling the inbound RTP task forever).
    #[serde(default = "default_inbound_send_deadline_ms")]
    pub inbound_send_deadline_ms: u64,

    /// Time after which a failed or closed route is reaped by the background
    /// session reaper. `0` disables the reaper.
    #[serde(default = "default_session_idle_ttl_secs")]
    pub session_idle_ttl_secs: u64,

    /// Trickle-ICE mode. When `true`:
    /// - `create_offer_and_gather` / `create_answer_and_gather` return as soon
    ///   as the local description is set, without waiting for ICE gathering to
    ///   complete (the SDP will contain few or zero candidates inline).
    /// - The signaler is expected to forward subsequent candidates via the
    ///   trickle channel (WS `ice-candidate` JSON or WHIP `PATCH
    ///   application/trickle-ice-sdpfrag`).
    ///
    /// Defaults to `false` for backward compatibility (UCTP v0 / full-gather).
    #[serde(default)]
    pub trickle_ice: bool,

    /// When `true`, `crate::adapter::WebRtcAdapter::hold` / `resume` not only
    /// flip the transceiver direction but also produce a fresh local SDP via
    /// renegotiation. Remote peers that ignore mute will still stop sending.
    /// Defaults to `true`.
    #[serde(default = "default_hold_renegotiate")]
    pub hold_renegotiate: bool,

    /// Max concurrent inbound WebRTC sessions the adapter will accept.
    /// `originate` and `apply_remote_offer` return an error when at cap.
    /// `0` disables the cap.
    #[serde(default = "default_max_concurrent_sessions")]
    pub max_concurrent_sessions: usize,

    /// CORS allow-list for the WHIP HTTP server. Empty = no CORS headers
    /// (assume reverse proxy or same-origin client). `["*"]` allows any.
    #[serde(default)]
    pub cors_origins: Vec<String>,

    /// Maximum WebSocket text frame size in bytes (anti-DoS).
    #[serde(default = "default_ws_max_message_size")]
    pub ws_max_message_size: usize,

    /// Interval between server-driven WebSocket pings. `0` disables.
    #[serde(default = "default_ws_keepalive_secs")]
    pub ws_keepalive_secs: u64,

    /// Inbound WHIP POSTs allowed per source IP per minute. `0` disables
    /// rate limiting.
    #[serde(default = "default_whip_per_ip_per_min")]
    pub whip_per_ip_per_min: u32,

    /// How to handle inbound trickle ICE candidates whose hostname ends in
    /// `.local` (RFC 8839 mDNS-style). Defaults to `Drop`.
    #[serde(default)]
    pub mdns_candidate_policy: MdnsCandidatePolicy,

    /// Restrict ICE candidate gathering to relay (TURN) candidates only.
    /// Equivalent to W3C `iceTransportPolicy = "relay"`. Useful when
    /// host/srflx candidates must be hidden for privacy or NAT topology
    /// reasons. Default: `All` (gather everything).
    #[serde(default)]
    pub ice_transport_policy: IceTransportPolicy,

    /// G12 — Opus codec tuning knobs reflected into the `a=fmtp:111` line
    /// at media-engine build time.
    #[serde(default)]
    pub opus_settings: OpusSettings,

    /// D2 — Static list of DTLS-SRTP certificate fingerprints the adapter
    /// will accept as remote peer identities. Default empty = no pinning
    /// (every fingerprint is allowed; current behavior). When non-empty,
    /// `apply_remote_offer` / `apply_remote_answer` reject any peer whose
    /// `a=fingerprint:` doesn't match an entry here — see also the
    /// runtime-set
    /// [`FingerprintPolicyHook`](crate::adapter::FingerprintPolicyHook)
    /// for per-route overrides (e.g. multi-tenant pinning).
    #[serde(default)]
    pub pinned_fingerprints: Vec<DtlsFingerprint>,

    /// When `true`, outbound peers created via `crate::WebRtcAdapter::originate`
    /// (WHEP `POST`, orchestrator-driven outbound) get a local video track
    /// attached *before* the offer SDP is generated, so the resulting offer
    /// advertises an `m=video` section. Default `false`, which preserves
    /// the historical audio-only outbound offer shape.
    #[serde(default)]
    pub originate_include_video: bool,
}

/// G12 — Opus encoder/decoder hints carried in the SDP fmtp line for
/// PT 111. Default values match the H1–H7 hard-coded fmtp
/// (`minptime=10;useinbandfec=1`).
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct OpusSettings {
    /// Inband FEC — sender encodes a low-bitrate copy of the previous
    /// frame in the next packet for loss concealment. Default `true`.
    pub use_in_band_fec: bool,
    /// DTX — discontinuous transmission. Skip frames during silence.
    /// Default `false` (most browser senders leave this off).
    pub use_dtx: bool,
    /// Minimum packet duration (ms). Default 10.
    pub min_ptime_ms: u32,
    /// Maximum average bitrate in bits/sec, when non-zero. Default `0`
    /// (let Opus pick — typically 32 kbit/s mono / 64 kbit/s stereo).
    pub max_average_bitrate_bps: u32,
    /// Stereo flag (sdp fmtp `stereo=1`). Default `false`.
    pub stereo: bool,
}

impl Default for OpusSettings {
    fn default() -> Self {
        Self {
            use_in_band_fec: true,
            use_dtx: false,
            min_ptime_ms: 10,
            max_average_bitrate_bps: 0,
            stereo: false,
        }
    }
}

impl OpusSettings {
    /// Render the SDP fmtp tail (i.e. the part after `a=fmtp:111 `).
    pub fn to_fmtp_line(&self) -> String {
        let mut bits = Vec::with_capacity(5);
        bits.push(format!("minptime={}", self.min_ptime_ms));
        bits.push(format!(
            "useinbandfec={}",
            if self.use_in_band_fec { 1 } else { 0 }
        ));
        if self.use_dtx {
            bits.push("usedtx=1".into());
        }
        if self.max_average_bitrate_bps > 0 {
            bits.push(format!("maxaveragebitrate={}", self.max_average_bitrate_bps));
        }
        if self.stereo {
            bits.push("stereo=1".into());
            bits.push("sprop-stereo=1".into());
        }
        bits.join(";")
    }
}

/// Maps to [`webrtc::peer_connection::RTCIceTransportPolicy`] / W3C
/// `iceTransportPolicy`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IceTransportPolicy {
    /// Gather host, srflx, prflx, and relay candidates.
    #[default]
    All,
    /// Only gather relay (TURN) candidates.
    Relay,
}

fn default_gather_timeout_secs() -> u64 {
    5
}

fn default_capabilities() -> CapabilityDescriptor {
    crate::sdp::capability::default_webrtc_capabilities()
}

fn default_handler_channel_capacity() -> usize {
    256
}

fn default_inbound_send_deadline_ms() -> u64 {
    200
}

fn default_session_idle_ttl_secs() -> u64 {
    300
}

fn default_hold_renegotiate() -> bool {
    true
}

fn default_max_concurrent_sessions() -> usize {
    1000
}

fn default_ws_max_message_size() -> usize {
    1024 * 1024 // 1 MB — plenty for SDP + a candidate
}

fn default_ws_keepalive_secs() -> u64 {
    15
}

fn default_whip_per_ip_per_min() -> u32 {
    60
}

impl Default for WebRtcConfig {
    fn default() -> Self {
        Self {
            udp_bind: "0.0.0.0:0".into(),
            ice_servers: vec![IceServerConfig::stun("stun:stun.l.google.com:19302")],
            gather_timeout_secs: default_gather_timeout_secs(),
            capabilities: default_capabilities(),
            handler_channel_capacity: default_handler_channel_capacity(),
            inbound_send_deadline_ms: default_inbound_send_deadline_ms(),
            session_idle_ttl_secs: default_session_idle_ttl_secs(),
            trickle_ice: false,
            hold_renegotiate: default_hold_renegotiate(),
            max_concurrent_sessions: default_max_concurrent_sessions(),
            cors_origins: Vec::new(),
            ws_max_message_size: default_ws_max_message_size(),
            ws_keepalive_secs: default_ws_keepalive_secs(),
            whip_per_ip_per_min: default_whip_per_ip_per_min(),
            mdns_candidate_policy: MdnsCandidatePolicy::Drop,
            ice_transport_policy: IceTransportPolicy::All,
            opus_settings: OpusSettings::default(),
            pinned_fingerprints: Vec::new(),
            originate_include_video: false,
        }
    }
}

impl WebRtcConfig {
    pub fn loopback() -> Self {
        Self {
            udp_bind: "127.0.0.1:0".into(),
            ice_servers: vec![],
            gather_timeout_secs: 5,
            capabilities: crate::sdp::capability::default_webrtc_capabilities(),
            handler_channel_capacity: default_handler_channel_capacity(),
            inbound_send_deadline_ms: default_inbound_send_deadline_ms(),
            session_idle_ttl_secs: default_session_idle_ttl_secs(),
            trickle_ice: false,
            hold_renegotiate: default_hold_renegotiate(),
            max_concurrent_sessions: default_max_concurrent_sessions(),
            cors_origins: Vec::new(),
            ws_max_message_size: default_ws_max_message_size(),
            // Disable keepalive in loopback so deterministic short tests don't
            // observe ping frames mid-handshake. Production should set this
            // (default 15s via `WebRtcConfig::default()`).
            ws_keepalive_secs: 0,
            whip_per_ip_per_min: 0, // loopback tests should not be rate-limited
            mdns_candidate_policy: MdnsCandidatePolicy::Drop,
            ice_transport_policy: IceTransportPolicy::All,
            opus_settings: OpusSettings::default(),
            pinned_fingerprints: Vec::new(),
            originate_include_video: false,
        }
    }

    /// Configure a TURN relay (external server — this crate does not host TURN).
    pub fn with_turn(
        mut self,
        url: impl Into<String>,
        username: impl Into<String>,
        credential: impl Into<String>,
    ) -> Self {
        self.ice_servers
            .push(IceServerConfig::turn(url, username, credential));
        self
    }
}
