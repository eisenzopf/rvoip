//! Unified PBX interop harness shared by the `pbx_endpoint`, `pbx_stream_peer`,
//! `pbx_callback_builder`, and `pbx_analyze` Cargo examples in this directory.
//!
//! The same scenario suite — registration/unregistration, basic call,
//! G.729, hold/resume, ring/cancel, DTMF, reject/busy, and blind transfer — is
//! exercised against both Asterisk and FreeSWITCH and through all three
//! public API surfaces ([`Endpoint`](rvoip_sip::Endpoint),
//! [`StreamPeer`](rvoip_sip::StreamPeer), and
//! [`CallbackPeer::builder`](rvoip_sip::CallbackPeerBuilder)) so provider
//! behaviour and surface ergonomics are validated in the same matrix.
//!
//! The runner (`examples/pbx/run.sh`) controls behaviour via these env vars:
//!
//! - `PBX_PROVIDER` (`asterisk`|`freeswitch`) — selects PBX defaults and SRTP
//!   policy
//! - `PBX_SCENARIO` (e.g. `registration`, `basic_call`, `g729_call`,
//!   `hold_resume`, `ring_cancel`, `dtmf`, `reject`, `blind_transfer`) —
//!   chooses the scenario
//! - `PBX_TRANSPORT` (`udp`|`tls`) — selects the transport leg
//! - `PBX_ROLE` — selects the participant (caller/callee/transfer-target/etc.)
//!
//! Per-provider tunables (`SIP_PORT`, `SIP_TLS_PORT`, `SIP_PASSWORD`,
//! `ASTERISK_TLS_CONTACT_MODE`, `FREESWITCH_UDP_ADDR`, etc.) come from the
//! `env/asterisk.env` and `env/freeswitch.env` files loaded by `run.sh`.
//!
//! See `examples/pbx/README.md` for the full scenario matrix, evidence layout,
//! and provider differences.

#![allow(dead_code)]

use std::fs::OpenOptions;
use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    Arc, Mutex, OnceLock,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rvoip_media_core::types::AudioFrame;
use rvoip_sip::{
    AudioSender, CallHandlerDecision, CallId, CallState, CallbackPeer, CallbackPeerControl, Config,
    Endpoint, EndpointAccount, EndpointProfile, Event, EventReceiver, MediaSecurityKeying,
    MediaSecurityProfile, MediaSecurityState, Registration, RegistrationHandle, SessionHandle,
    SessionId, SipAccount, SipContactMode, SrtpSuitePolicy, StreamPeer, TransferOutcome,
    TransferWaitMode, UnifiedCoordinator,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};

pub type ExampleResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub const SAMPLE_RATE: u32 = 8000;
pub const FRAME_SIZE: usize = 160;
pub const G729_FRAME_SIZE: usize = 80;

/// One 20 ms AMR frame, which is *not* the same for both variants: AMR-NB is
/// 160 samples at 8 kHz and AMR-WB is 320 at 16 kHz.
///
/// Feeding a wideband session 160-sample frames does not degrade gracefully —
/// the encoder refuses them outright, because a short frame silently
/// mis-framed would drift against the far end. So the recorder has to be told
/// which variant it is driving.
/// Matched exhaustively rather than with a catch-all: a wildcard here would
/// give any newly added wideband profile the narrowband rate silently, which is
/// the failure this function exists to prevent.
pub const fn amr_sample_rate(profile: CodecProfile) -> u32 {
    match profile {
        CodecProfile::AmrWb | CodecProfile::AmrWbBe => 16_000,
        CodecProfile::Default
        | CodecProfile::G729A
        | CodecProfile::G729AB
        | CodecProfile::AmrNb
        | CodecProfile::AmrNbBe
        | CodecProfile::Pcmu => 8_000,
    }
}

/// The highest mode index of an AMR profile — where our encoder opens, and
/// therefore what the peer sends until someone asks otherwise. Used by the
/// mode-switch step to prove the peer actually *moved*.
pub const fn amr_top_mode_index(profile: CodecProfile) -> u8 {
    match profile {
        CodecProfile::AmrWb | CodecProfile::AmrWbBe => 8,
        CodecProfile::Default
        | CodecProfile::G729A
        | CodecProfile::G729AB
        | CodecProfile::AmrNb
        | CodecProfile::AmrNbBe
        | CodecProfile::Pcmu => 7,
    }
}

/// Whether the operator asked the caller to exercise a mid-call codec mode
/// request (`PBX_AMR_MODE_SWITCH=1`).
fn amr_mode_switch_requested() -> bool {
    matches!(
        std::env::var("PBX_AMR_MODE_SWITCH").as_deref(),
        Ok("1") | Ok("true") | Ok("yes") | Ok("on")
    )
}

/// Whether the operator asked for AMR discontinuous transmission
/// (`PBX_AMR_DTX=1`).
///
/// Sender-side policy with nothing in the SDP, so a cell that enables it
/// looks identical on the signalling side and differs only in what the
/// encoder emits during silence.
fn amr_dtx_requested() -> bool {
    matches!(
        std::env::var("PBX_AMR_DTX").as_deref(),
        Ok("1") | Ok("true") | Ok("yes") | Ok("on")
    )
}

/// `PBX_AMR_MODE_SET=0,2,4` restricts the cell to those AMR modes.
///
/// Unlike `PBX_AMR_DTX` this one *is* visible in the SDP: it becomes
/// RFC 4867 `mode-set` on the offer, and the set is bi-directional, so it
/// governs what the PBX sends us as well as what we send it. That is what
/// makes a per-rate lab cell possible at all — without it every cell runs at
/// whatever mode the encoder opens at, which is the highest permitted one.
///
/// Unparseable entries are dropped rather than defaulted, so a typo narrows
/// the set or empties it rather than silently testing a different rate than
/// the label claims.
fn amr_mode_set_requested() -> Option<Vec<u8>> {
    let raw = std::env::var("PBX_AMR_MODE_SET").ok()?;
    let modes: Vec<u8> = raw
        .split(',')
        .filter_map(|part| part.trim().parse::<u8>().ok())
        .collect();
    (!modes.is_empty()).then_some(modes)
}

pub const fn amr_frame_size(profile: CodecProfile) -> usize {
    match profile {
        CodecProfile::AmrWb | CodecProfile::AmrWbBe => 320,
        CodecProfile::Default
        | CodecProfile::G729A
        | CodecProfile::G729AB
        | CodecProfile::AmrNb
        | CodecProfile::AmrNbBe
        | CodecProfile::Pcmu => 160,
    }
}
pub const TONE_FRAMES: usize = 150;
pub const ENDPOINT_2001_TONE_HZ: f32 = 440.0;
pub const ENDPOINT_2002_TONE_HZ: f32 = 880.0;
pub const ENDPOINT_1001_TONE_HZ: f32 = ENDPOINT_2001_TONE_HZ;
pub const ENDPOINT_1002_TONE_HZ: f32 = ENDPOINT_2002_TONE_HZ;
pub const ENDPOINT_1003_TONE_HZ: f32 = 660.0;
/// The evidence floor as a duration, which is what it always meant.
///
/// It used to be stated only as `MIN_RECEIVED_SAMPLES = 12_000`, a raw sample
/// count — 1.5 s at 8 kHz but 0.75 s at 16 kHz, so every wideband AMR run
/// quietly collected half the exercise of a narrowband one and nothing said
/// so.
pub const MIN_RECEIVED_MS: usize = 1_500;
pub const fn min_received_samples(sample_rate: u32) -> usize {
    sample_rate as usize * MIN_RECEIVED_MS / 1000
}
pub const MIN_RECEIVED_SAMPLES: usize = min_received_samples(SAMPLE_RATE);
// The caller controls call teardown from its local receive count, while the
// peer's recorder can trail it by several codec frames during PBX transcoding
// and TLS/SRTP scheduling. Capture another half-second before BYE so both
// independently recorded directions satisfy the unchanged evidence floor.
pub const G729_CALLER_CAPTURE_TARGET_SAMPLES: usize =
    MIN_RECEIVED_SAMPLES + SAMPLE_RATE as usize / 2;
/// The tone-analysis window is 200 ms *at any rate*.
///
/// Stated as a sample count it was 200 ms at 8 kHz but silently 100 ms at
/// 16 kHz. The duration is load-bearing twice over: every tone this harness
/// sends (440, 660, 880 Hz) lands on an exact Goertzel bin of a 200 ms window
/// at both rates, and an off-bin fundamental caps the measurable SNR near
/// 17 dB no matter how clean the audio is — see
/// `harness_tones_land_on_an_exact_goertzel_bin`.
pub const fn tone_analysis_window_samples(sample_rate: u32) -> usize {
    sample_rate as usize / 5
}
pub const TONE_ANALYSIS_WINDOW_SAMPLES: usize = tone_analysis_window_samples(SAMPLE_RATE);
/// One 20 ms frame at `sample_rate`: 160 at 8 kHz, 320 at 16 kHz.
pub const fn frame_samples(sample_rate: u32) -> usize {
    sample_rate as usize / 50
}

/// Peak amplitude of the tone this harness sends — `0.3 * 32767` in
/// [`generate_tone_at_rate`]. Every level threshold below is a fraction of it,
/// so a deliberate change to the sent level moves the thresholds with it.
pub const TONE_PEAK_AMPLITUDE: f32 = 0.3 * 32767.0;
pub const TONE_RMS: f32 = TONE_PEAK_AMPLITUDE / std::f32::consts::SQRT_2;

/// Per-window floor on fundamental-vs-residual power for an AMR capture.
///
/// Calibrated against real captures at the top modes (AMR-NB 12.2, AMR-WB
/// 23.85): the cleanest path's *worst* window measured +25.7 dB and a
/// degraded path's best *sustained* stretch at 15 dB lasted 0.46 s against
/// the 1 s required below. Lowering this below ~13 dB without lengthening
/// [`AMR_REQUIRED_TONE_SECS`] lets that degraded capture back in — the two
/// constants trade off, and the tradeoff was measured, not guessed. Lower
/// modes will code a tone worse; when a rate sweep lands, calibrate a
/// per-mode table from codec loopback rather than relaxing this blanket
/// figure.
pub const AMR_MIN_TONE_SNR_DB: f32 = 15.0;
/// Per-20 ms-frame RMS floor: a quarter of what we send. The cleanest capture
/// never dipped below 0.6× sent RMS; a degraded one spent whole frames near
/// 0.02×. Level is checked separately from SNR because attenuation preserves
/// spectral purity perfectly.
pub const AMR_MIN_FRAME_RMS: f32 = TONE_RMS / 4.0;
/// The unbroken stretch of passing windows an AMR capture must contain — the
/// same one-continuous-second guarantee `assert_audio_path` gives G.711 and
/// G.729.
pub const AMR_REQUIRED_TONE_SECS: f32 = 1.0;
pub const HOLD_RESUME_PRE_HOLD_FRAMES: usize = 100;
pub const HOLD_RESUME_HELD_FRAMES: usize = 50;
pub const HOLD_RESUME_POST_RESUME_FRAMES: usize = 200;
pub const DOMINANCE_RATIO: f32 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PbxProvider {
    Asterisk,
    FreeSwitch,
    /// Kamailio registrar-proxy with rtpengine relaying media
    /// (infra/release-runners/pbx/kamailio). A proxy, not a B2BUA: it routes
    /// by registered contact and rtpengine forwards payloads verbatim.
    Kamailio,
    /// OpenSIPS sibling of the Kamailio lab.
    OpenSips,
}

impl PbxProvider {
    pub fn from_env_or_args() -> ExampleResult<Self> {
        let mut value = std::env::var("PBX_PROVIDER")
            .or_else(|_| std::env::var("PBX"))
            .unwrap_or_else(|_| "asterisk".to_string());
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--pbx" | "--provider" => {
                    value = args
                        .next()
                        .ok_or_else(|| format!("{} requires a value", arg))?;
                }
                _ => {}
            }
        }
        match value.trim().to_ascii_lowercase().as_str() {
            "asterisk" | "ast" => Ok(Self::Asterisk),
            "kamailio" | "kam" => Ok(Self::Kamailio),
            "opensips" | "open-sips" | "osips" => Ok(Self::OpenSips),
            "freeswitch" | "free-switch" | "fs" => Ok(Self::FreeSwitch),
            other => Err(format!("unknown PBX provider '{}'", other).into()),
        }
    }

    pub fn env_name(self) -> &'static str {
        match self {
            Self::Asterisk => "asterisk",
            Self::FreeSwitch => "freeswitch",
            Self::Kamailio => "kamailio",
            Self::OpenSips => "opensips",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Asterisk => "Asterisk",
            Self::FreeSwitch => "FreeSWITCH",
            Self::Kamailio => "Kamailio",
            Self::OpenSips => "OpenSIPS",
        }
    }

    fn default_settle_secs(self) -> u64 {
        match self {
            Self::Asterisk => 5,
            Self::FreeSwitch => 2,
            // usrloc writes are synchronous and in-memory; nothing to settle.
            Self::Kamailio | Self::OpenSips => 1,
        }
    }

    fn default_retry_attempts(self) -> usize {
        match self {
            Self::Asterisk => 8,
            Self::FreeSwitch => 4,
            Self::Kamailio | Self::OpenSips => 4,
        }
    }

    fn expects_target_cancel(self) -> bool {
        match self {
            Self::Asterisk => env_bool("ASTERISK_EXPECT_TARGET_CANCEL", false).unwrap_or(false),
            Self::FreeSwitch => true,
            // A proxy relays CANCEL verbatim.
            Self::Kamailio | Self::OpenSips => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportMode {
    Udp,
    TlsSrtp,
}

impl TransportMode {
    pub fn from_env_or_args() -> ExampleResult<Self> {
        let mut value = std::env::var("PBX_TRANSPORT")
            .or_else(|_| std::env::var("SIP_TRANSPORT"))
            .unwrap_or_else(|_| "udp".to_string());
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            if arg.as_str() == "--transport" {
                value = args
                    .next()
                    .ok_or_else(|| "--transport requires a value".to_string())?;
            }
        }
        match value.trim().to_ascii_lowercase().as_str() {
            "udp" | "rtp" => Ok(Self::Udp),
            "tls" | "tls-srtp" | "srtp" => Ok(Self::TlsSrtp),
            other => Err(format!("unknown PBX transport '{}'", other).into()),
        }
    }

    pub fn is_tls(self) -> bool {
        self == Self::TlsSrtp
    }

    pub fn env_value(self) -> &'static str {
        match self {
            Self::Udp => "UDP",
            Self::TlsSrtp => "TLS",
        }
    }

    pub fn scenario_prefix(self) -> &'static str {
        match self {
            Self::Udp => "udp",
            Self::TlsSrtp => "tls",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scenario {
    Registration,
    BasicCall,
    G729Call,
    AmrCall,
    /// Caller and callee offer disjoint codecs, forcing the PBX to transcode
    /// — the scenario where a foreign AMR implementation must actually read
    /// our bitstream. See [`CodecPairing`].
    AmrTranscodeCall,
    /// rvoip is the B2BUA in the middle: caller → PBX → rvoip → PBX → target,
    /// with rvoip terminating both legs and bridging their payloads. Closes
    /// half the exit criterion in `AMR_IMPLEMENTATION_PLAN.md` — rvoip as the
    /// relaying B2BUA rather than an endpoint through someone else's bridge.
    B2buaCall,
    HoldResume,
    RingCancel,
    Dtmf,
    Reject,
    BlindTransfer,
}

impl Scenario {
    pub fn from_env_or_args() -> ExampleResult<Self> {
        let mut value = std::env::var("PBX_SCENARIO")
            .or_else(|_| std::env::var("CALLBACK_SCENARIO"))
            .unwrap_or_else(|_| "registration".to_string());
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            if arg.as_str() == "--scenario" {
                value = args
                    .next()
                    .ok_or_else(|| "--scenario requires a value".to_string())?;
            }
        }
        Self::parse(&value)
    }

    /// The pure name parser, so tests need not touch the environment.
    pub fn parse(value: &str) -> ExampleResult<Self> {
        let normalized = value
            .trim()
            .to_ascii_lowercase()
            .replace('-', "_")
            .replace("tls_", "")
            .replace("udp_", "");
        match normalized.as_str() {
            "registration" | "registration_tls" | "registration_udp" => Ok(Self::Registration),
            "basic" | "basic_call" | "call" | "udp_call" => Ok(Self::BasicCall),
            "g729" | "g729_call" | "g729ab" | "g729ab_call" => Ok(Self::G729Call),
            "amr" | "amr_call" | "amrwb" | "amrwb_call" | "amr_wb_call" => Ok(Self::AmrCall),
            "amr_transcode" | "amr_transcode_call" | "transcode" => Ok(Self::AmrTranscodeCall),
            "b2bua" | "b2bua_call" => Ok(Self::B2buaCall),
            "hold" | "hold_resume" => Ok(Self::HoldResume),
            "ring" | "ring_cancel" | "ring_remote" => Ok(Self::RingCancel),
            "dtmf" => Ok(Self::Dtmf),
            "reject" | "busy" => Ok(Self::Reject),
            "blind_transfer" | "blind_transfer_remote" | "transfer" => Ok(Self::BlindTransfer),
            other => Err(format!("unknown PBX scenario '{}'", other).into()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecProfile {
    Default,
    G729A,
    G729AB,
    /// AMR-NB, octet-aligned. The framing most PBXes default to, and the one
    /// whose fmtp has to survive the round trip for anything to be audible.
    AmrNb,
    /// AMR-WB, octet-aligned. Separate because it is 16 kHz with 320-sample
    /// frames, which is where a narrowband assumption shows up.
    AmrWb,
    /// AMR-NB, bandwidth-efficient. The framing whose payload boundaries do not
    /// fall on octets, so a peer that packs it wrongly still produces a payload
    /// of a plausible length — the error surfaces as noise, not as a refusal.
    ///
    /// It is also the only framing a bridged FreeSWITCH call can use end to
    /// end: FreeSWITCH offers `octet-align=0` on the outbound leg and relays
    /// payloads between the legs without re-framing them, so an octet-aligned
    /// inbound leg leaves both endpoints reading the framing they did not
    /// agree to.
    AmrNbBe,
    /// AMR-WB, bandwidth-efficient.
    AmrWbBe,
    /// PCMU alone — the far leg of a transcode pairing. PCMU only, no PCMA,
    /// for the same reason the AMR profiles offer one framing each: the
    /// negotiated codec must be provable from the profile name.
    Pcmu,
}

impl CodecProfile {
    pub fn from_env_or_scenario() -> ExampleResult<Self> {
        Self::for_endpoint(None, None)
    }

    /// The codec profile for one participant.
    ///
    /// Delegates to [`select_codec_profile`] with the process environment
    /// filled in; the precedence and every refusal live in that pure function
    /// where they are testable without env mutation.
    pub fn for_endpoint(username: Option<&str>, role: Option<Role>) -> ExampleResult<Self> {
        let endpoint_override = username
            .and_then(|user| std::env::var(format!("ENDPOINT_{}_CODEC_PROFILE", user)).ok());
        let global = std::env::var("PBX_CODEC_PROFILE").ok();
        let pairing = std::env::var("PBX_CODEC_PAIRING").ok();
        select_codec_profile(
            Scenario::from_env_or_args()?,
            role,
            endpoint_override.as_deref(),
            global.as_deref(),
            pairing.as_deref(),
        )
    }

    fn parse(value: &str) -> ExampleResult<Self> {
        match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "" | "default" | "pcmu_pcma" | "g711" => Ok(Self::Default),
            "g729a" | "g729_annex_a" | "annexb_no" => Ok(Self::G729A),
            "g729" | "g729ab" | "g729ba" | "annexb_yes" => Ok(Self::G729AB),
            "amr" | "amrnb" | "amr_nb" => Ok(Self::AmrNb),
            "amrwb" | "amr_wb" => Ok(Self::AmrWb),
            "amrnb_be" | "amr_nb_be" | "amrnbbe" => Ok(Self::AmrNbBe),
            "amrwb_be" | "amr_wb_be" | "amrwbbe" => Ok(Self::AmrWbBe),
            "pcmu" | "ulaw" => Ok(Self::Pcmu),
            other => Err(format!("unknown PBX codec profile '{}'", other).into()),
        }
    }

    /// The payload types this profile puts in the offer, `None` meaning the
    /// stack's defaults. Split from [`Self::apply`] so the disjointness of a
    /// [`CodecPairing`]'s two legs is a unit-testable property of the lists
    /// themselves, not of a fully-constructed `Config`.
    pub fn offered_codecs(self) -> Option<Vec<u8>> {
        match self {
            Self::Default => None,
            Self::G729A | Self::G729AB => Some(vec![18, 101]),
            // Octet-aligned only: offering both framings for one codec lets a
            // PBX pick the other one, and then a passing call would say
            // nothing about the framing under test. 107 is AMR-NB
            // octet-aligned and 105 is AMR-WB octet-aligned.
            Self::AmrNb => Some(vec![107, 101]),
            Self::AmrWb => Some(vec![105, 101]),
            // 106 and 104 are the same two codecs bandwidth-efficient, and are
            // offered alone for the same reason: a call that passes must have
            // used the framing named here.
            Self::AmrNbBe => Some(vec![106, 101]),
            Self::AmrWbBe => Some(vec![104, 101]),
            Self::Pcmu => Some(vec![0, 101]),
        }
    }

    fn apply(self, config: &mut Config) {
        if let Some(codecs) = self.offered_codecs() {
            config.offered_codecs = codecs;
        }
        match self {
            Self::G729A => config.g729_annex_b = false,
            Self::G729AB => config.g729_annex_b = true,
            _ => {}
        }
    }

    fn env_value(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::G729A => "g729a",
            Self::G729AB => "g729ab",
            Self::AmrNb => "amrnb",
            Self::AmrWb => "amrwb",
            Self::AmrNbBe => "amrnb_be",
            Self::AmrWbBe => "amrwb_be",
            Self::Pcmu => "pcmu",
        }
    }
}

/// A codec per leg, for the scenario whose whole point is that the two legs
/// cannot agree.
///
/// When both legs of a bridged call offer the same codec, both PBXes in this
/// lab relay the RTP payloads untouched — Asterisk switches to its native_rtp
/// bridge, FreeSWITCH forwards ingress bytes verbatim — and no foreign codec
/// ever touches our bitstream. A pairing offers *disjoint* codecs, so the PBX
/// physically cannot native-bridge: its own AMR implementation must decode
/// what we encoded and encode what we decode.
///
/// Named pairings rather than a `caller_callee` grammar because profile names
/// already contain underscores (`amrnb_be`), so a grammar would have to guess
/// the split.
// The shared `Amr` prefix is the point: every pairing names which AMR leg is
// under test, and the far leg after the underscore.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecPairing {
    /// AMR-NB caller, PCMU callee — the default. One foreign AMR codec in
    /// each direction, and a lossless-ish reference on the far side, so a
    /// tone failure names a suspect.
    AmrNbPcmu,
    /// AMR-WB caller, PCMU callee: the wideband decoder/encoder pair, plus
    /// the PBX's 16 kHz ↔ 8 kHz resampler.
    AmrWbPcmu,
    /// Bandwidth-efficient variants, the fallback if a PBX mishandles
    /// octet-aligned input on its transcoding path.
    AmrNbBePcmu,
    AmrWbBePcmu,
    /// Both our variants through the PBX's transcoder at once. The stretch
    /// case: four codecs in the path, so run it after a PCMU pairing is
    /// green, not instead of one.
    AmrNbAmrWb,
}

impl CodecPairing {
    pub fn parse(value: &str) -> ExampleResult<Self> {
        match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "amrnb_pcmu" => Ok(Self::AmrNbPcmu),
            "amrwb_pcmu" => Ok(Self::AmrWbPcmu),
            "amrnb_be_pcmu" => Ok(Self::AmrNbBePcmu),
            "amrwb_be_pcmu" => Ok(Self::AmrWbBePcmu),
            "amrnb_amrwb" => Ok(Self::AmrNbAmrWb),
            other => Err(format!("unknown PBX codec pairing '{}'", other).into()),
        }
    }

    pub fn env_value(self) -> &'static str {
        match self {
            Self::AmrNbPcmu => "amrnb_pcmu",
            Self::AmrWbPcmu => "amrwb_pcmu",
            Self::AmrNbBePcmu => "amrnb_be_pcmu",
            Self::AmrWbBePcmu => "amrwb_be_pcmu",
            Self::AmrNbAmrWb => "amrnb_amrwb",
        }
    }

    /// Which profile each leg offers. Only the two media roles have one:
    /// asking for a target/transferor profile is a wiring error, not a
    /// default.
    pub fn profile_for(self, role: Role) -> ExampleResult<CodecProfile> {
        let (caller, callee) = match self {
            Self::AmrNbPcmu => (CodecProfile::AmrNb, CodecProfile::Pcmu),
            Self::AmrWbPcmu => (CodecProfile::AmrWb, CodecProfile::Pcmu),
            Self::AmrNbBePcmu => (CodecProfile::AmrNbBe, CodecProfile::Pcmu),
            Self::AmrWbBePcmu => (CodecProfile::AmrWbBe, CodecProfile::Pcmu),
            Self::AmrNbAmrWb => (CodecProfile::AmrNb, CodecProfile::AmrWb),
        };
        match role {
            Role::Caller => Ok(caller),
            Role::Callee => Ok(callee),
            other => Err(format!(
                "codec pairing {} has no profile for role {:?}: only the caller and callee \
                 carry media in a transcode call",
                self.env_value(),
                other
            )
            .into()),
        }
    }
}

/// The one place codec-profile precedence is decided, pure so it is testable
/// without mutating the process environment:
///
/// 1. `ENDPOINT_{username}_CODEC_PROFILE` — the per-participant override
///    channel every other per-endpoint knob already uses;
/// 2. the pairing, for the transcode scenario;
/// 3. `PBX_CODEC_PROFILE` — **refused** for the transcode scenario, because
///    one profile cannot describe two legs, and accepting it would silently
///    collapse both legs onto one codec: the PBX would native-bridge again
///    and the scenario would pass while proving nothing;
/// 4. the per-scenario default.
fn select_codec_profile(
    scenario: Scenario,
    role: Option<Role>,
    endpoint_override: Option<&str>,
    global: Option<&str>,
    pairing: Option<&str>,
) -> ExampleResult<CodecProfile> {
    if let Some(value) = endpoint_override {
        return CodecProfile::parse(value);
    }
    if scenario == Scenario::AmrTranscodeCall {
        if global.is_some() {
            return Err(
                "PBX_CODEC_PROFILE is one profile and a transcode call needs one per leg; \
                 set PBX_CODEC_PAIRING (or ENDPOINT_{user}_CODEC_PROFILE) instead"
                    .into(),
            );
        }
        let pairing = match pairing {
            Some(value) => CodecPairing::parse(value)?,
            None => CodecPairing::AmrNbPcmu,
        };
        let role =
            role.ok_or("a transcode call resolves its codec per role, and no role was given")?;
        return pairing.profile_for(role);
    }
    if let Some(value) = global {
        return CodecProfile::parse(value);
    }
    match scenario {
        Scenario::G729Call => Ok(CodecProfile::G729AB),
        // Narrowband by default: it is the variant every AMR-capable PBX
        // has, and PBX_CODEC_PROFILE=amrwb selects the other.
        Scenario::AmrCall => Ok(CodecProfile::AmrNb),
        // The exit criterion names AMR-WB; the run.sh sweep pins the framing
        // per provider and adds a PCMU control cell.
        Scenario::B2buaCall => Ok(CodecProfile::AmrWb),
        _ => Ok(CodecProfile::Default),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Registration,
    Caller,
    Callee,
    Target,
    Transferor,
    Transferee,
    /// The B2BUA in the middle: registers, accepts an inbound leg, originates
    /// an outbound leg, and bridges the two.
    B2bua,
}

impl Role {
    pub fn from_env_or_args() -> ExampleResult<Self> {
        let mut value = std::env::var("PBX_ROLE").unwrap_or_else(|_| "registration".to_string());
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            if arg.as_str() == "--role" {
                value = args
                    .next()
                    .ok_or_else(|| "--role requires a value".to_string())?;
            }
        }
        match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "registration" | "register" => Ok(Self::Registration),
            "caller" | "uac" => Ok(Self::Caller),
            "callee" | "uas" => Ok(Self::Callee),
            "target" | "transfer_target" | "ring_target" => Ok(Self::Target),
            "transferor" => Ok(Self::Transferor),
            "transferee" => Ok(Self::Transferee),
            "b2bua" => Ok(Self::B2bua),
            other => Err(format!("unknown PBX role '{}'", other).into()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsContactMode {
    ReachableContact,
    RegisteredFlowRfc5626,
    RegisteredFlowSymmetric,
}

impl TlsContactMode {
    fn from_env(provider: PbxProvider) -> ExampleResult<Self> {
        if provider == PbxProvider::Asterisk && env_bool("ASTERISK_TLS_FLOW_REUSE", false)? {
            return Ok(Self::RegisteredFlowSymmetric);
        }
        let key = match provider {
            PbxProvider::Asterisk => "ASTERISK_TLS_CONTACT_MODE",
            PbxProvider::FreeSwitch => "FREESWITCH_TLS_CONTACT_MODE",
            // TLS is not wired for the proxy labs yet; the arms exist so the
            // match stays exhaustive and the key is ready when it is.
            PbxProvider::Kamailio => "KAMAILIO_TLS_CONTACT_MODE",
            PbxProvider::OpenSips => "OPENSIPS_TLS_CONTACT_MODE",
        };
        match env_string(key, "reachable-contact")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "reachable-contact" | "reachable" | "listener" | "uas" => Ok(Self::ReachableContact),
            "registered-flow" | "registered-flow-rfc5626" | "rfc5626" | "outbound" => {
                Ok(Self::RegisteredFlowRfc5626)
            }
            "registered-flow-symmetric" | "symmetric" | "symmetric-transport"
            | "flow-reuse" | "client-only" => Ok(Self::RegisteredFlowSymmetric),
            other => Err(format!(
                "{} must be reachable-contact, registered-flow-rfc5626, or registered-flow-symmetric, got '{}'",
                key, other
            )
            .into()),
        }
    }

    fn uses_listener(self) -> bool {
        self == Self::ReachableContact
    }

    fn sip_contact_mode(self) -> SipContactMode {
        match self {
            Self::ReachableContact => SipContactMode::ReachableContact,
            Self::RegisteredFlowRfc5626 => SipContactMode::RegisteredFlowRfc5626,
            Self::RegisteredFlowSymmetric => SipContactMode::RegisteredFlowSymmetric,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EndpointConfig {
    pub provider: PbxProvider,
    pub username: String,
    pub auth_username: String,
    pub password: String,
    pub sip_server: String,
    pub sip_port: u16,
    pub transport: TransportMode,
    pub local_ip: IpAddr,
    pub advertised_ip: IpAddr,
    pub media_advertised_ip: IpAddr,
    pub local_port: u16,
    pub tls_local_port: Option<u16>,
    pub tls_contact_mode: TlsContactMode,
    pub media_port_start: u16,
    pub media_port_end: u16,
    pub codec_profile: CodecProfile,
    pub output_dir: PathBuf,
}

impl EndpointConfig {
    pub fn new(
        provider: PbxProvider,
        username: &str,
        transport: TransportMode,
    ) -> ExampleResult<Self> {
        Self::new_for_role(provider, username, transport, None)
    }

    /// The role-aware constructor. The role only matters for scenarios whose
    /// two legs run different codecs; passing `None` keeps the historical
    /// behaviour everywhere else.
    pub fn new_for_role(
        provider: PbxProvider,
        username: &str,
        transport: TransportMode,
        role: Option<Role>,
    ) -> ExampleResult<Self> {
        let defaults = endpoint_defaults(provider, username, transport);
        let prefix = format!("ENDPOINT_{}", username);
        let (sip_server, sip_port) = match provider {
            PbxProvider::Asterisk => {
                let server = env_string("SIP_SERVER", "192.168.1.103");
                let port = if transport.is_tls() {
                    env_u16("SIP_TLS_PORT", 5061)?
                } else {
                    env_u16("SIP_PORT", 5060)?
                };
                (server, port)
            }
            PbxProvider::FreeSwitch => {
                let addr_key = if transport.is_tls() {
                    "FREESWITCH_TLS_ADDR"
                } else {
                    "FREESWITCH_UDP_ADDR"
                };
                let default_addr = if transport.is_tls() {
                    "127.0.0.1:5063"
                } else {
                    "127.0.0.1:5062"
                };
                split_host_port(&env_string(addr_key, default_addr))?
            }
            PbxProvider::Kamailio => {
                let addr_key = if transport.is_tls() {
                    "KAMAILIO_TLS_ADDR"
                } else {
                    "KAMAILIO_UDP_ADDR"
                };
                let default_addr = if transport.is_tls() {
                    "127.0.0.1:5067"
                } else {
                    "127.0.0.1:5066"
                };
                split_host_port(&env_string(addr_key, default_addr))?
            }
            PbxProvider::OpenSips => {
                let addr_key = if transport.is_tls() {
                    "OPENSIPS_TLS_ADDR"
                } else {
                    "OPENSIPS_UDP_ADDR"
                };
                let default_addr = if transport.is_tls() {
                    "127.0.0.1:5075"
                } else {
                    "127.0.0.1:5068"
                };
                split_host_port(&env_string(addr_key, default_addr))?
            }
        };
        let auth_username = auth_username_for(&prefix, username);
        let password = match provider {
            PbxProvider::Asterisk => std::env::var(format!("{}_PASSWORD", prefix))
                .or_else(|_| std::env::var("SIP_PASSWORD"))
                .unwrap_or_else(|_| "password123".to_string()),
            PbxProvider::FreeSwitch => std::env::var(format!("{}_PASSWORD", prefix))
                .or_else(|_| std::env::var("FREESWITCH_PASSWORD"))
                .or_else(|_| std::env::var("SIP_PASSWORD"))
                .unwrap_or_else(|_| "1234".to_string()),
            // The proxy labs run an accept-all registrar; the password is
            // carried but never challenged for.
            PbxProvider::Kamailio => std::env::var(format!("{}_PASSWORD", prefix))
                .or_else(|_| std::env::var("KAMAILIO_PASSWORD"))
                .or_else(|_| std::env::var("SIP_PASSWORD"))
                .unwrap_or_else(|_| "password123".to_string()),
            PbxProvider::OpenSips => std::env::var(format!("{}_PASSWORD", prefix))
                .or_else(|_| std::env::var("OPENSIPS_PASSWORD"))
                .or_else(|_| std::env::var("SIP_PASSWORD"))
                .unwrap_or_else(|_| "password123".to_string()),
        };
        let local_ip: IpAddr = match provider {
            PbxProvider::Asterisk => env_string("LOCAL_IP", "0.0.0.0").parse()?,
            PbxProvider::FreeSwitch | PbxProvider::Kamailio | PbxProvider::OpenSips => {
                std::env::var("RVOIP_LOCAL_IP")
                    .or_else(|_| std::env::var("LOCAL_IP"))
                    .unwrap_or_else(|_| "127.0.0.1".to_string())
                    .parse()?
            }
        };
        let advertised_ip = advertised_ip(provider, local_ip)?;
        let media_advertised_ip = media_advertised_ip(provider, advertised_ip)?;
        let local_port = env_u16(&format!("{}_LOCAL_PORT", prefix), defaults.local_port)?;
        let tls_contact_mode = TlsContactMode::from_env(provider)?;
        let tls_local_port = if transport.is_tls() {
            Some(env_u16(
                &format!("{}_TLS_LOCAL_PORT", prefix),
                defaults
                    .tls_local_port
                    .unwrap_or(defaults.local_port.saturating_add(1)),
            )?)
        } else {
            None
        };
        let media_port_start = env_u16(
            &format!("{}_MEDIA_PORT_START", prefix),
            defaults.media_port_start,
        )?;
        let media_port_end = env_u16(
            &format!("{}_MEDIA_PORT_END", prefix),
            defaults.media_port_end,
        )?;
        let codec_profile = CodecProfile::for_endpoint(Some(username), role)?;
        let output_dir = std::env::var("AUDIO_OUTPUT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("examples/pbx/output")
                    .join(provider.env_name())
            });

        Ok(Self {
            provider,
            username: username.to_string(),
            auth_username,
            password,
            sip_server,
            sip_port,
            transport,
            local_ip,
            advertised_ip,
            media_advertised_ip,
            local_port,
            tls_local_port,
            tls_contact_mode,
            media_port_start,
            media_port_end,
            codec_profile,
            output_dir,
        })
    }

    pub fn registrar_uri(&self) -> String {
        format!(
            "{}:{}:{}{}",
            self.uri_scheme(),
            self.sip_server,
            self.sip_port,
            transport_suffix(self.transport)
        )
    }

    pub fn aor_uri(&self) -> String {
        format!(
            "{}:{}@{}",
            self.uri_scheme(),
            self.username,
            self.sip_server
        )
    }

    pub fn contact_uri(&self) -> String {
        format!(
            "{}:{}@{}:{}{}",
            self.uri_scheme(),
            self.username,
            self.advertised_ip,
            self.contact_port(),
            transport_suffix(self.transport)
        )
    }

    pub fn call_uri(&self, target: &str) -> String {
        if self.transport.is_tls() || self.sip_port != default_pbx_port(self.transport) {
            format!(
                "{}:{}@{}:{}{}",
                self.uri_scheme(),
                target,
                self.sip_server,
                self.sip_port,
                transport_suffix(self.transport)
            )
        } else {
            format!("sip:{}@{}", target, self.sip_server)
        }
    }

    pub fn outbound_call_uri(&self, target: &str) -> String {
        let key = format!("ENDPOINT_{}_CALL_URI", self.username);
        std::env::var(&key)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| self.call_uri(target))
    }

    pub fn remote_user(&self) -> &'static str {
        if self.transport.is_tls() {
            "1003"
        } else {
            "2003"
        }
    }

    pub fn remote_call_uri(&self) -> String {
        let override_key = if self.transport.is_tls() {
            "REMOTE_TLS_CALL_URI"
        } else {
            "REMOTE_UDP_CALL_URI"
        };
        std::env::var(override_key)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| self.call_uri(self.remote_user()))
    }

    pub fn stream_config(&self) -> Config {
        let mut config = match self.provider {
            PbxProvider::Asterisk => Config::on(&self.username, self.local_ip, self.local_port),
            PbxProvider::FreeSwitch => Config::freeswitch_internal(
                &self.username,
                SocketAddr::new(self.local_ip, self.local_port),
            ),
            // Plain config: the proxies impose no FreeSWITCH-shaped quirks.
            PbxProvider::Kamailio | PbxProvider::OpenSips => {
                Config::on(&self.username, self.local_ip, self.local_port)
            }
        };
        config.local_uri = self.aor_uri();
        config.contact_uri = Some(self.contact_uri());
        config.sip_advertised_addr = Some(SocketAddr::new(self.advertised_ip, self.local_port));
        if self.transport.is_tls() {
            config.tls_advertised_addr =
                Some(SocketAddr::new(self.advertised_ip, self.contact_port()));
        }
        config.sip_contact_mode = if self.transport.is_tls() {
            self.tls_contact_mode.sip_contact_mode()
        } else {
            SipContactMode::ReachableContact
        };
        config.credentials = Some(self.sip_account().credentials());
        config.media_port_start = self.media_port_start;
        config.media_port_end = self.media_port_end;
        config.media_public_addr = Some(SocketAddr::new(self.media_advertised_ip, 0));
        // AMR DTX is sender-side policy with no SDP surface, so it is set on
        // whichever side the operator runs with PBX_AMR_DTX=1 and the other
        // side needs no matching setting to receive it.
        //
        // Set here rather than in `session_config`: that method returns this
        // one's result early for every non-TLS transport, so a knob applied to
        // its tail reaches TLS cells only — which is exactly how the first
        // version of this silently did nothing on UDP.
        config = config.with_amr_dtx(amr_dtx_requested());
        // Same placement reasoning as `amr_dtx` above, and the same trap: set
        // in `session_config` this would reach TLS cells only. An absent
        // request and an empty one are the same state — the builder maps an
        // empty slice to "no mode-set offered".
        config = config.with_amr_mode_set(&amr_mode_set_requested().unwrap_or_default());
        self.codec_profile.apply(&mut config);
        config
    }

    pub fn session_config(&self) -> ExampleResult<Config> {
        if !self.transport.is_tls() {
            return Ok(self.stream_config());
        }

        let mut config = self.stream_config();
        match self.tls_contact_mode {
            TlsContactMode::ReachableContact => {
                let tls_port = self.tls_local_port.ok_or_else(|| {
                    "TLS reachable-contact mode requires ENDPOINT_<user>_TLS_LOCAL_PORT".to_string()
                })?;
                config = config.tls_reachable_contact(
                    SocketAddr::new(self.local_ip, tls_port),
                    required_path("TLS_CERT_PATH")?,
                    required_path("TLS_KEY_PATH")?,
                );
            }
            TlsContactMode::RegisteredFlowRfc5626 => {
                config = config.tls_registered_flow_rfc5626(self.sip_instance_urn());
            }
            TlsContactMode::RegisteredFlowSymmetric => {
                config = config.tls_registered_flow_symmetric(self.sip_instance_urn());
            }
        }
        config.tls_extra_ca_path = optional_path("TLS_CA_PATH");
        config.tls_client_cert_path = optional_path("TLS_CLIENT_CERT_PATH");
        config.tls_client_key_path = optional_path("TLS_CLIENT_KEY_PATH");
        #[cfg(feature = "dev-insecure-tls")]
        {
            let default_insecure = self.provider == PbxProvider::FreeSwitch;
            config.tls_insecure_skip_verify = env_bool("TLS_INSECURE", default_insecure)?;
        }
        config.offer_srtp = true;
        config.srtp_required = match self.provider {
            PbxProvider::Asterisk => env_bool("ASTERISK_TLS_SRTP_REQUIRED", true)?,
            PbxProvider::FreeSwitch => env_bool("FREESWITCH_TLS_SRTP_REQUIRED", true)?,
            PbxProvider::Kamailio => env_bool("KAMAILIO_TLS_SRTP_REQUIRED", true)?,
            PbxProvider::OpenSips => env_bool("OPENSIPS_TLS_SRTP_REQUIRED", true)?,
        };
        if self.provider == PbxProvider::FreeSwitch {
            config = config.with_srtp_suite_policy(SrtpSuitePolicy::FreeSwitchCompatible);
        }
        Ok(config)
    }

    pub fn registration(&self) -> Registration {
        self.sip_account().registration()
    }

    pub fn endpoint_account(&self) -> EndpointAccount {
        self.sip_account().endpoint_account()
    }

    pub fn sip_account(&self) -> SipAccount {
        SipAccount::new(self.registrar_uri(), &self.username, &self.password)
            .auth_username(&self.auth_username)
            .from_uri(self.aor_uri())
            .contact_uri(self.contact_uri())
    }

    fn uri_scheme(&self) -> &'static str {
        if self.transport.is_tls() {
            "sips"
        } else {
            "sip"
        }
    }

    fn contact_port(&self) -> u16 {
        if self.transport.is_tls() && self.tls_contact_mode.uses_listener() {
            self.tls_local_port
                .unwrap_or(self.local_port.saturating_add(1))
        } else {
            self.local_port
        }
    }

    fn sip_instance_urn(&self) -> String {
        std::env::var(format!("ENDPOINT_{}_SIP_INSTANCE", self.username))
            .or_else(|_| std::env::var("SIP_INSTANCE"))
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| deterministic_sip_instance(&self.username))
    }
}

#[derive(Debug, Clone, Copy)]
struct EndpointDefaults {
    local_port: u16,
    tls_local_port: Option<u16>,
    media_port_start: u16,
    media_port_end: u16,
}

#[derive(Debug)]
pub struct ToneAnalysis {
    pub samples: usize,
    pub expected_hz: f32,
    pub rejected_hz: f32,
    pub expected_magnitude: f32,
    pub rejected_magnitude: f32,
    pub ratio: f32,
}

#[derive(Debug)]
struct ToneWindowScan {
    best: ToneAnalysis,
    total_windows: usize,
    passing_windows: usize,
    longest_passing_run: usize,
    required_passing_run: usize,
    analysis_window_samples: usize,
    step_samples: usize,
    /// Quality of the best-ratio window, plus the weakest figures seen in
    /// *any* window — a failure message must name the limiting window, not
    /// the flattering one. A degraded capture's best window can measure
    /// better than a clean capture's worst; only the weakest-window view
    /// discriminates.
    best_quality: ToneQuality,
    weakest_snr_db: f32,
    weakest_frame_rms: f32,
}

/// What a single analysis window must satisfy to count as passing.
///
/// The dominance ratio alone is scale-invariant and phase-blind: it passed a
/// capture that was 1-bit square-wave distortion, one that was attenuated
/// 100×, and one with half its frames zeroed. Each quality clause exists
/// because a measured failure defeated the others.
#[derive(Debug, Clone, Copy)]
struct WindowGate {
    min_ratio: f32,
    min_snr_db: Option<f32>,
    min_frame_rms: Option<f32>,
}

impl WindowGate {
    /// Exactly the historical predicate: the right tone dominates the wrong
    /// one. The scenarios that only ever asked that question keep asking it,
    /// bit-for-bit.
    fn tone_only() -> Self {
        Self {
            min_ratio: DOMINANCE_RATIO,
            min_snr_db: None,
            min_frame_rms: None,
        }
    }

    /// The AMR gate: the tone must dominate, must actually *be* a tone, and
    /// must be there at full level in every 20 ms frame.
    fn amr() -> Self {
        Self {
            min_ratio: DOMINANCE_RATIO,
            min_snr_db: Some(AMR_MIN_TONE_SNR_DB),
            min_frame_rms: Some(AMR_MIN_FRAME_RMS),
        }
    }

    fn admits(&self, analysis: &ToneAnalysis, quality: &ToneQuality) -> bool {
        analysis.ratio >= self.min_ratio
            && self.min_snr_db.is_none_or(|floor| quality.snr_db >= floor)
            && self
                .min_frame_rms
                .is_none_or(|floor| quality.min_frame_rms >= floor)
    }
}

pub struct ToneRecorder {
    running: Arc<AtomicBool>,
    /// Makes the send loop emit digital silence instead of the tone, so a DTX
    /// cell has something for the encoder's VAD to detect.
    sending_silence: Arc<AtomicBool>,
    send_task: JoinHandle<()>,
    recv_task: JoinHandle<()>,
    received_buf: Arc<Mutex<Vec<i16>>>,
    counters: Arc<RecorderCounters>,
    diag_output_dir: Option<PathBuf>,
    diag_name: String,
    /// The rate the far end's audio arrives at, carried so the saved file says
    /// so. Wideband recordings written with the narrowband default play an
    /// octave low at twice the duration, and nothing about them looks wrong.
    sample_rate: u32,
}

pub struct RecorderCounters {
    first_rx_elapsed_ms: AtomicU64,
    last_rx_elapsed_ms: AtomicU64,
    rx_frames: AtomicUsize,
    rx_samples: AtomicUsize,
}

impl RecorderCounters {
    fn new() -> Self {
        Self {
            first_rx_elapsed_ms: AtomicU64::new(0),
            last_rx_elapsed_ms: AtomicU64::new(0),
            rx_frames: AtomicUsize::new(0),
            rx_samples: AtomicUsize::new(0),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum IncomingMode {
    Accept,
    RejectBusy,
    Defer(Duration),
}

pub enum CallbackEvent {
    Incoming {
        call_id: CallId,
        from: String,
        to: String,
    },
    Established(SessionHandle),
    Progress {
        call_id: CallId,
        status_code: u16,
        reason: String,
        sdp: Option<String>,
    },
    Ended {
        call_id: CallId,
        reason: String,
    },
    Failed {
        call_id: CallId,
        status_code: u16,
        reason: String,
    },
    Cancelled {
        call_id: CallId,
    },
    Dtmf {
        call_id: CallId,
        digit: char,
    },
    MediaSecurity {
        call_id: CallId,
        state: MediaSecurityState,
    },
    LocalHold {
        call_id: CallId,
    },
    LocalResume {
        call_id: CallId,
    },
    RemoteHold {
        call_id: CallId,
    },
    RemoteResume {
        call_id: CallId,
    },
    TransferAccepted {
        call_id: CallId,
        refer_to: String,
    },
    ReferProgress {
        call_id: CallId,
        status_code: u16,
        reason: String,
    },
    ReferCompleted {
        call_id: CallId,
        target: String,
        status_code: u16,
        reason: String,
    },
    TransferFailed {
        call_id: CallId,
        status_code: u16,
        reason: String,
    },
    RegistrationSuccess {
        registrar: String,
        expires: u32,
        contact: String,
    },
    UnregistrationSuccess {
        registrar: String,
    },
}

pub struct CallbackRuntime {
    pub cfg: EndpointConfig,
    pub control: CallbackPeerControl,
    pub events: mpsc::UnboundedReceiver<CallbackEvent>,
    run_task: JoinHandle<rvoip_sip::Result<()>>,
}

impl CallbackRuntime {
    pub async fn shutdown(self) -> ExampleResult<()> {
        self.control.shutdown();
        let _ = timeout(Duration::from_secs(3), self.run_task).await;
        Ok(())
    }
}

pub fn load_env(provider: PbxProvider) {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    if let Ok(home) = std::env::var("HOME") {
        match provider {
            PbxProvider::Asterisk => {
                let _ = dotenvy::from_filename(
                    Path::new(&home)
                        .join("Developer")
                        .join("asterisk")
                        .join("rvoip-local.env"),
                );
            }
            PbxProvider::FreeSwitch => {
                let _ = dotenvy::from_filename(
                    Path::new(&home)
                        .join("Developer")
                        .join("freeswitch")
                        .join("freeswitch-local.env"),
                );
            }
            PbxProvider::Kamailio => {
                let _ = dotenvy::from_filename(
                    Path::new(&home)
                        .join("Developer")
                        .join("kamailio")
                        .join("kamailio-local.env"),
                );
            }
            PbxProvider::OpenSips => {
                let _ = dotenvy::from_filename(
                    Path::new(&home)
                        .join("Developer")
                        .join("opensips")
                        .join("opensips-local.env"),
                );
            }
        }
    }
    let _ = dotenvy::from_filename(
        manifest
            .join("examples/pbx/env")
            .join(format!("{}.env", provider.env_name())),
    );
    let _ = dotenvy::from_filename(manifest.join("examples/pbx/.env.local"));
    let _ = dotenvy::dotenv();
}

pub fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info,rvoip_sip_dialog=warn".into()),
        )
        .try_init();
}

fn pbx_diag_enabled() -> bool {
    matches!(
        std::env::var("PBX_DIAG")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn diag_start() -> &'static Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now)
}

fn diag_elapsed_ms() -> u64 {
    diag_start().elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

fn diag_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}

fn diag_role_name() -> String {
    std::env::var("PBX_ROLE").unwrap_or_else(|_| "unknown".to_string())
}

fn diag_output_dir_from_env() -> Option<PathBuf> {
    std::env::var("AUDIO_OUTPUT_DIR")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
}

fn diag_event_env(event: &str, fields: serde_json::Value) {
    if let Some(output_dir) = diag_output_dir_from_env() {
        diag_event(&output_dir, event, fields);
    }
}

fn diag_event(output_dir: &Path, event: &str, fields: serde_json::Value) {
    if !pbx_diag_enabled() {
        return;
    }
    let _ = std::fs::create_dir_all(output_dir);
    let role = diag_role_name();
    let mut record = serde_json::Map::new();
    record.insert("event".to_string(), serde_json::json!(event));
    record.insert("role".to_string(), serde_json::json!(role));
    record.insert(
        "elapsed_ms".to_string(),
        serde_json::json!(diag_elapsed_ms()),
    );
    record.insert("epoch_ms".to_string(), serde_json::json!(diag_epoch_ms()));
    if let serde_json::Value::Object(extra) = fields {
        for (key, value) in extra {
            record.insert(key, value);
        }
    }
    let path = output_dir.join(format!("{}_timeline.jsonl", role));
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{}", serde_json::Value::Object(record));
    }
}

fn diag_role_result(result: &ExampleResult<()>) {
    match result {
        Ok(()) => diag_event_env("role_exit", serde_json::json!({ "status": "ok" })),
        Err(error) => diag_event_env(
            "role_exit",
            serde_json::json!({
                "status": "error",
                "error": error.to_string()
            }),
        ),
    }
}

fn diag_call_id(handle: &SessionHandle) -> String {
    format!("{:?}", handle.id())
}

pub fn context() -> ExampleResult<(PbxProvider, Scenario, TransportMode, Role)> {
    let provider = PbxProvider::from_env_or_args()?;
    load_env(provider);
    init_tracing();
    Ok((
        provider,
        Scenario::from_env_or_args()?,
        TransportMode::from_env_or_args()?,
        Role::from_env_or_args()?,
    ))
}

pub fn username_for(transport: TransportMode, role: Role) -> &'static str {
    match (transport, role) {
        (TransportMode::TlsSrtp, Role::Caller | Role::Transferor | Role::Registration) => "1001",
        (TransportMode::TlsSrtp, Role::Callee | Role::Transferee | Role::B2bua) => "1002",
        (TransportMode::TlsSrtp, Role::Target) => "1003",
        (TransportMode::Udp, Role::Caller | Role::Transferor | Role::Registration) => "2001",
        (TransportMode::Udp, Role::Callee | Role::Transferee | Role::B2bua) => "2002",
        (TransportMode::Udp, Role::Target) => "2003",
    }
}

pub fn endpoint_config_for(
    provider: PbxProvider,
    transport: TransportMode,
    role: Role,
) -> ExampleResult<EndpointConfig> {
    EndpointConfig::new_for_role(
        provider,
        username_for(transport, role),
        transport,
        Some(role),
    )
}

pub async fn new_stream_peer(cfg: &EndpointConfig) -> ExampleResult<StreamPeer> {
    Ok(StreamPeer::with_config(cfg.session_config()?).await?)
}

pub async fn new_endpoint(cfg: &EndpointConfig) -> ExampleResult<Endpoint> {
    Ok(Endpoint::builder()
        .name(&cfg.username)
        .endpoint_account(cfg.endpoint_account())
        .profile(EndpointProfile::Custom(cfg.session_config()?))
        .build()
        .await?)
}

pub async fn callback_runtime(
    provider: PbxProvider,
    transport: TransportMode,
    role: Role,
    mode: IncomingMode,
) -> ExampleResult<CallbackRuntime> {
    let cfg = endpoint_config_for(provider, transport, role)?;
    let (tx, events) = mpsc::unbounded_channel();
    let incoming_tx = tx.clone();
    let established_tx = tx.clone();
    let progress_tx = tx.clone();
    let ended_tx = tx.clone();
    let failed_tx = tx.clone();
    let cancelled_tx = tx.clone();
    let dtmf_tx = tx.clone();
    let media_security_tx = tx.clone();
    let local_hold_tx = tx.clone();
    let local_resume_tx = tx.clone();
    let remote_hold_tx = tx.clone();
    let remote_resume_tx = tx.clone();
    let transfer_accepted_tx = tx.clone();
    let refer_progress_tx = tx.clone();
    let refer_completed_tx = tx.clone();
    let transfer_failed_tx = tx.clone();
    let registration_tx = tx.clone();
    let unregistration_tx = tx;

    let peer = CallbackPeer::builder(cfg.session_config()?)
        .on_incoming(move |call| {
            let tx = incoming_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::Incoming {
                    call_id: call.call_id.clone(),
                    from: call.from.clone(),
                    to: call.to.clone(),
                });
                match mode {
                    IncomingMode::Accept => CallHandlerDecision::Accept,
                    IncomingMode::RejectBusy => CallHandlerDecision::Reject {
                        status: 486,
                        reason: "Busy Here".to_string(),
                    },
                    IncomingMode::Defer(duration) => {
                        CallHandlerDecision::Defer(call.defer(duration))
                    }
                }
            }
        })
        .on_established(move |handle| {
            let tx = established_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::Established(handle));
                Ok(())
            }
        })
        .on_progress(move |handle, status_code, reason, sdp| {
            let tx = progress_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::Progress {
                    call_id: handle.id().clone(),
                    status_code,
                    reason,
                    sdp,
                });
                Ok(())
            }
        })
        .on_ended(move |call_id, reason| {
            let tx = ended_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::Ended {
                    call_id,
                    reason: format!("{reason:?}"),
                });
                Ok(())
            }
        })
        .on_failed(move |call_id, status_code, reason| {
            let tx = failed_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::Failed {
                    call_id,
                    status_code,
                    reason,
                });
                Ok(())
            }
        })
        .on_cancelled(move |call_id| {
            let tx = cancelled_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::Cancelled { call_id });
                Ok(())
            }
        })
        .on_dtmf(move |handle, digit| {
            let tx = dtmf_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::Dtmf {
                    call_id: handle.id().clone(),
                    digit,
                });
                Ok(())
            }
        })
        .on_media_security(move |handle, state| {
            let tx = media_security_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::MediaSecurity {
                    call_id: handle.id().clone(),
                    state,
                });
                Ok(())
            }
        })
        .on_refer_received(|handle, request| async move {
            println!(
                "[callback-transfer] accepting REFER on call {} (method={:?})",
                handle.id(),
                request.method
            );
            Ok(true)
        })
        .on_local_hold(move |handle| {
            let tx = local_hold_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::LocalHold {
                    call_id: handle.id().clone(),
                });
                Ok(())
            }
        })
        .on_local_resume(move |handle| {
            let tx = local_resume_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::LocalResume {
                    call_id: handle.id().clone(),
                });
                Ok(())
            }
        })
        .on_remote_hold(move |handle| {
            let tx = remote_hold_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::RemoteHold {
                    call_id: handle.id().clone(),
                });
                Ok(())
            }
        })
        .on_remote_resume(move |handle| {
            let tx = remote_resume_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::RemoteResume {
                    call_id: handle.id().clone(),
                });
                Ok(())
            }
        })
        .on_transfer_accepted(move |handle, refer_to| {
            let tx = transfer_accepted_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::TransferAccepted {
                    call_id: handle.id().clone(),
                    refer_to,
                });
                Ok(())
            }
        })
        .on_refer_progress(move |handle, status_code, reason| {
            let tx = refer_progress_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::ReferProgress {
                    call_id: handle.id().clone(),
                    status_code,
                    reason,
                });
                Ok(())
            }
        })
        .on_refer_completed(move |handle, target, status_code, reason| {
            let tx = refer_completed_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::ReferCompleted {
                    call_id: handle.id().clone(),
                    target,
                    status_code,
                    reason,
                });
                Ok(())
            }
        })
        .on_transfer_failed(move |handle, status_code, reason| {
            let tx = transfer_failed_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::TransferFailed {
                    call_id: handle.id().clone(),
                    status_code,
                    reason,
                });
                Ok(())
            }
        })
        .on_registration_success(move |registrar, expires, contact| {
            let tx = registration_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::RegistrationSuccess {
                    registrar,
                    expires,
                    contact,
                });
                Ok(())
            }
        })
        .on_unregistration_success(move |registrar| {
            let tx = unregistration_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::UnregistrationSuccess { registrar });
                Ok(())
            }
        })
        .build()
        .await?;
    let control = peer.control();
    let run_task = tokio::spawn(async move { peer.run().await });
    sleep(Duration::from_millis(100)).await;
    Ok(CallbackRuntime {
        cfg,
        control,
        events,
        run_task,
    })
}

pub async fn register_stream_peer(
    peer: &mut StreamPeer,
    cfg: &EndpointConfig,
) -> ExampleResult<RegistrationHandle> {
    print_registration_context(cfg);
    let handle = peer.register_account(&cfg.sip_account()).send().await?;
    wait_for_stream_registration(peer, &handle, &cfg.username).await?;
    println!("[{}] Registered.", cfg.username);
    diag_event(
        &cfg.output_dir,
        "registration_complete",
        serde_json::json!({ "username": cfg.username.as_str(), "surface": "stream_peer" }),
    );
    Ok(handle)
}

pub async fn register_endpoint_api(
    endpoint: &mut Endpoint,
    cfg: &EndpointConfig,
) -> ExampleResult<RegistrationHandle> {
    print_registration_context(cfg);
    let handle = endpoint.register().await?;
    for _ in 0..50 {
        if endpoint
            .control()
            .coordinator()
            .is_registered(&handle)
            .await?
        {
            println!("[{}] Registered.", cfg.username);
            diag_event(
                &cfg.output_dir,
                "registration_complete",
                serde_json::json!({ "username": cfg.username.as_str(), "surface": "endpoint" }),
            );
            return Ok(handle);
        }
        sleep(Duration::from_millis(200)).await;
    }
    Err(format!("endpoint {} did not register within 10s", cfg.username).into())
}

pub async fn register_callback_endpoint(
    runtime: &mut CallbackRuntime,
) -> ExampleResult<RegistrationHandle> {
    print_registration_context(&runtime.cfg);
    let handle = runtime
        .control
        .register_account(&runtime.cfg.sip_account())
        .send()
        .await?;
    for _ in 0..50 {
        if runtime.control.is_registered(&handle).await? {
            wait_for_registration_success(&mut runtime.events, Duration::from_secs(10)).await?;
            println!("[{}] Registered.", runtime.cfg.username);
            diag_event(
                &runtime.cfg.output_dir,
                "registration_complete",
                serde_json::json!({
                    "username": runtime.cfg.username.as_str(),
                    "surface": "callback"
                }),
            );
            return Ok(handle);
        }
        sleep(Duration::from_millis(200)).await;
    }
    Err(format!(
        "callback endpoint {} did not register within 10s",
        runtime.cfg.username
    )
    .into())
}

pub async fn unregister_callback_endpoint(
    runtime: &mut CallbackRuntime,
    handle: &RegistrationHandle,
) -> ExampleResult<()> {
    runtime.control.unregister(handle).await?;
    wait_for_unregistration_success(&mut runtime.events, Duration::from_secs(10)).await?;
    println!("[{}] Unregistered.", runtime.cfg.username);
    diag_event(
        &runtime.cfg.output_dir,
        "unregistration_complete",
        serde_json::json!({ "username": runtime.cfg.username.as_str() }),
    );
    Ok(())
}

pub async fn run_stream_peer_surface() -> ExampleResult<()> {
    let (provider, scenario, transport, role) = context()?;
    let result = run_stream_peer(provider, scenario, transport, role).await;
    diag_role_result(&result);
    result
}

pub async fn run_endpoint_surface() -> ExampleResult<()> {
    let (provider, scenario, transport, role) = context()?;
    let result = run_endpoint(provider, scenario, transport, role).await;
    diag_role_result(&result);
    result
}

pub async fn run_callback_builder_surface() -> ExampleResult<()> {
    let (provider, scenario, transport, role) = context()?;
    let result = run_callback(provider, scenario, transport, role).await;
    diag_role_result(&result);
    result
}

async fn run_stream_peer(
    provider: PbxProvider,
    scenario: Scenario,
    transport: TransportMode,
    role: Role,
) -> ExampleResult<()> {
    let cfg = endpoint_config_for(provider, transport, role)?;
    let mut peer = new_stream_peer(&cfg).await?;
    let registration = register_stream_peer(&mut peer, &cfg).await?;
    match scenario {
        Scenario::Registration => {
            sleep(idle_duration()).await;
        }
        Scenario::BasicCall
        | Scenario::G729Call
        | Scenario::AmrCall
        | Scenario::AmrTranscodeCall
        | Scenario::HoldResume
        | Scenario::RingCancel
        | Scenario::Dtmf
        | Scenario::Reject => {
            run_stream_peer_two_party(provider, scenario, transport, role, &cfg, &mut peer).await?;
        }
        Scenario::B2buaCall => {
            return Err("b2bua_call runs through the endpoint API only; use --api endpoint".into());
        }
        Scenario::BlindTransfer => {
            run_stream_peer_transfer(provider, transport, role, &cfg, &mut peer).await?;
        }
    }
    peer.unregister(&registration).await.ok();
    peer.shutdown().await.ok();
    Ok(())
}

async fn run_endpoint(
    provider: PbxProvider,
    scenario: Scenario,
    transport: TransportMode,
    role: Role,
) -> ExampleResult<()> {
    let cfg = endpoint_config_for(provider, transport, role)?;
    let mut endpoint = new_endpoint(&cfg).await?;
    register_endpoint_api(&mut endpoint, &cfg).await?;
    match scenario {
        Scenario::Registration => {
            sleep(idle_duration()).await;
        }
        Scenario::BasicCall
        | Scenario::G729Call
        | Scenario::AmrCall
        | Scenario::AmrTranscodeCall
        | Scenario::HoldResume
        | Scenario::RingCancel
        | Scenario::Dtmf
        | Scenario::Reject => {
            run_endpoint_two_party(provider, scenario, transport, role, &cfg, &mut endpoint)
                .await?;
        }
        Scenario::B2buaCall => {
            run_endpoint_b2bua(provider, transport, role, &cfg, &mut endpoint).await?;
        }
        Scenario::BlindTransfer => {
            run_endpoint_transfer(provider, transport, role, &cfg, &mut endpoint).await?;
        }
    }
    endpoint.unregister().await.ok();
    endpoint.shutdown().await.ok();
    Ok(())
}

async fn run_callback(
    provider: PbxProvider,
    scenario: Scenario,
    transport: TransportMode,
    role: Role,
) -> ExampleResult<()> {
    let mode = match (scenario, role) {
        (Scenario::Reject, Role::Callee) => IncomingMode::RejectBusy,
        (Scenario::RingCancel, Role::Target) => IncomingMode::Defer(Duration::from_secs(30)),
        (_, Role::Callee | Role::Target | Role::Transferee) => IncomingMode::Accept,
        _ => IncomingMode::RejectBusy,
    };
    let mut runtime = callback_runtime(provider, transport, role, mode).await?;
    let registration = register_callback_endpoint(&mut runtime).await?;
    match scenario {
        Scenario::Registration => {
            sleep(idle_duration()).await;
        }
        Scenario::BasicCall
        | Scenario::G729Call
        | Scenario::AmrCall
        | Scenario::AmrTranscodeCall
        | Scenario::HoldResume
        | Scenario::RingCancel
        | Scenario::Dtmf
        | Scenario::Reject => {
            run_callback_two_party(provider, scenario, transport, role, &mut runtime).await?;
        }
        Scenario::B2buaCall => {
            return Err("b2bua_call runs through the endpoint API only; use --api endpoint".into());
        }
        Scenario::BlindTransfer => {
            run_callback_transfer(transport, role, &mut runtime).await?;
        }
    }
    unregister_callback_endpoint(&mut runtime, &registration)
        .await
        .ok();
    runtime.shutdown().await
}

async fn run_stream_peer_two_party(
    provider: PbxProvider,
    scenario: Scenario,
    transport: TransportMode,
    role: Role,
    cfg: &EndpointConfig,
    peer: &mut StreamPeer,
) -> ExampleResult<()> {
    match (scenario, role) {
        (Scenario::BasicCall, Role::Caller) => {
            settle_after_register(provider).await;
            let target = cfg.outbound_call_uri(target_user_for(transport));
            let handle =
                call_with_answer_retry(peer, &target, remote_test_timeout(provider)?).await?;
            run_basic_caller(cfg, &handle, transport).await?;
        }
        (Scenario::BasicCall, Role::Callee) => {
            let incoming =
                timeout(remote_test_timeout(provider)?, peer.wait_for_incoming()).await??;
            let handle = incoming.accept().await?;
            run_basic_callee(provider, cfg, &handle, transport).await?;
        }
        (Scenario::G729Call, Role::Caller) => {
            settle_after_register(provider).await;
            let target = cfg.outbound_call_uri(target_user_for(transport));
            let handle =
                call_with_answer_retry(peer, &target, remote_test_timeout(provider)?).await?;
            run_g729_caller(cfg, &handle, transport).await?;
        }
        (Scenario::G729Call, Role::Callee) => {
            let incoming =
                timeout(remote_test_timeout(provider)?, peer.wait_for_incoming()).await??;
            let handle = incoming.accept().await?;
            run_g729_callee(provider, cfg, &handle, transport).await?;
        }
        (Scenario::AmrCall, Role::Caller) => {
            settle_after_register(provider).await;
            let target = cfg.outbound_call_uri(target_user_for(transport));
            let handle =
                call_with_answer_retry(peer, &target, remote_test_timeout(provider)?).await?;
            run_amr_caller(cfg, &handle, transport, amr_caller_wav(transport)).await?;
        }
        (Scenario::AmrCall, Role::Callee) => {
            let incoming =
                timeout(remote_test_timeout(provider)?, peer.wait_for_incoming()).await??;
            let handle = incoming.accept().await?;
            run_amr_callee(provider, cfg, &handle, transport, amr_callee_wav(transport)).await?;
        }
        (Scenario::AmrTranscodeCall, Role::Caller) => {
            settle_after_register(provider).await;
            let target = cfg.outbound_call_uri(target_user_for(transport));
            let handle =
                call_with_answer_retry(peer, &target, remote_test_timeout(provider)?).await?;
            let wav = amr_transcode_wav(cfg);
            run_amr_caller(cfg, &handle, transport, &wav).await?;
        }
        (Scenario::AmrTranscodeCall, Role::Callee) => {
            let incoming =
                timeout(remote_test_timeout(provider)?, peer.wait_for_incoming()).await??;
            let handle = incoming.accept().await?;
            let wav = amr_transcode_wav(cfg);
            run_amr_callee(provider, cfg, &handle, transport, &wav).await?;
        }
        (Scenario::HoldResume, Role::Caller) => {
            settle_after_register(provider).await;
            let target = cfg.outbound_call_uri(target_user_for(transport));
            let handle =
                call_with_answer_retry(peer, &target, remote_test_timeout(provider)?).await?;
            run_hold_on_handle(provider, cfg, &handle, transport).await?;
        }
        (Scenario::HoldResume, Role::Callee) => {
            let incoming =
                timeout(remote_test_timeout(provider)?, peer.wait_for_incoming()).await??;
            let handle = incoming.accept().await?;
            run_answering_tone_role(
                cfg,
                &handle,
                tone_for_callee(transport),
                hold_resume_callee_wav(transport),
                transport,
            )
            .await?;
        }
        (Scenario::RingCancel, Role::Caller) => {
            settle_after_register(provider).await;
            let handle = call_with_ringing_retry(
                peer,
                &cfg.remote_call_uri(),
                remote_test_timeout(provider)?,
            )
            .await?;
            let mut events = handle.events().await?;
            handle
                .hangup_and_wait(Some(Duration::from_secs(12)))
                .await?;
            wait_for_call_cancelled_on_events(&mut events, Duration::from_secs(12))
                .await
                .ok();
        }
        (Scenario::RingCancel, Role::Target) => run_deferred_target(provider, peer, cfg).await?,
        (Scenario::Dtmf, Role::Caller) => {
            settle_after_register(provider).await;
            let target = target_user_for(transport);
            let handle = call_with_answer_retry(
                peer,
                &cfg.outbound_call_uri(target),
                remote_test_timeout(provider)?,
            )
            .await?;
            run_dtmf_caller(cfg, &handle, transport).await?;
        }
        (Scenario::Dtmf, Role::Callee) => {
            let incoming =
                timeout(remote_test_timeout(provider)?, peer.wait_for_incoming()).await??;
            let handle = incoming.accept().await?;
            run_dtmf_callee(provider, cfg, &handle, transport).await?;
        }
        (Scenario::Reject, Role::Caller) => {
            settle_after_register(provider).await;
            let target = target_user_for(transport);
            let call_id = peer.invite(cfg.outbound_call_uri(target)).send().await?;
            let handle = peer.coordinator().session(&call_id);
            let mut events = handle.events().await?;
            let (status, _) =
                wait_for_call_failed_on_events(&mut events, remote_test_timeout(provider)?).await?;
            if status != 486 {
                return Err(format!("expected 486 Busy Here, got {}", status).into());
            }
        }
        (Scenario::Reject, Role::Callee) => {
            let incoming =
                timeout(remote_test_timeout(provider)?, peer.wait_for_incoming()).await??;
            incoming.reject(486, "Busy Here");
            sleep(Duration::from_secs(1)).await;
        }
        _ => {
            return Err(
                format!("unsupported StreamPeer role {:?} for {:?}", role, scenario).into(),
            );
        }
    }
    Ok(())
}

async fn run_endpoint_two_party(
    provider: PbxProvider,
    scenario: Scenario,
    transport: TransportMode,
    role: Role,
    cfg: &EndpointConfig,
    endpoint: &mut Endpoint,
) -> ExampleResult<()> {
    match (scenario, role) {
        (Scenario::BasicCall, Role::Caller) => {
            settle_after_register(provider).await;
            let handle = endpoint
                .call_and_wait(
                    target_user_for(transport),
                    Some(remote_test_timeout(provider)?),
                )
                .await?;
            run_basic_caller(cfg, handle.as_session_handle(), transport).await?;
        }
        (Scenario::BasicCall, Role::Callee) => {
            let incoming =
                timeout(remote_test_timeout(provider)?, endpoint.wait_for_incoming()).await??;
            let handle = incoming.accept().await?;
            run_basic_callee(provider, cfg, handle.as_session_handle(), transport).await?;
        }
        (Scenario::G729Call, Role::Caller) => {
            settle_after_register(provider).await;
            let handle = endpoint
                .call_and_wait(
                    target_user_for(transport),
                    Some(remote_test_timeout(provider)?),
                )
                .await?;
            run_g729_caller(cfg, handle.as_session_handle(), transport).await?;
        }
        (Scenario::G729Call, Role::Callee) => {
            let incoming =
                timeout(remote_test_timeout(provider)?, endpoint.wait_for_incoming()).await??;
            let handle = incoming.accept().await?;
            run_g729_callee(provider, cfg, handle.as_session_handle(), transport).await?;
        }
        (Scenario::AmrCall, Role::Caller) => {
            settle_after_register(provider).await;
            let handle = endpoint
                .call_and_wait(
                    target_user_for(transport),
                    Some(remote_test_timeout(provider)?),
                )
                .await?;
            run_amr_caller_toned(
                cfg,
                handle.as_session_handle(),
                transport,
                tone_for_caller(transport),
                tone_for_callee(transport),
                amr_caller_wav(transport),
                Some(endpoint.control().coordinator()),
            )
            .await?;
        }
        (Scenario::AmrCall, Role::Callee) => {
            let incoming =
                timeout(remote_test_timeout(provider)?, endpoint.wait_for_incoming()).await??;
            let handle = incoming.accept().await?;
            run_amr_callee(
                provider,
                cfg,
                handle.as_session_handle(),
                transport,
                amr_callee_wav(transport),
            )
            .await?;
        }
        (Scenario::AmrTranscodeCall, Role::Caller) => {
            settle_after_register(provider).await;
            let handle = endpoint
                .call_and_wait(
                    target_user_for(transport),
                    Some(remote_test_timeout(provider)?),
                )
                .await?;
            let wav = amr_transcode_wav(cfg);
            run_amr_caller(cfg, handle.as_session_handle(), transport, &wav).await?;
        }
        (Scenario::AmrTranscodeCall, Role::Callee) => {
            let incoming =
                timeout(remote_test_timeout(provider)?, endpoint.wait_for_incoming()).await??;
            let handle = incoming.accept().await?;
            let wav = amr_transcode_wav(cfg);
            run_amr_callee(provider, cfg, handle.as_session_handle(), transport, &wav).await?;
        }
        (Scenario::HoldResume, Role::Caller) => {
            settle_after_register(provider).await;
            let target = cfg.outbound_call_uri(target_user_for(transport));
            let call_id = endpoint.invite(&target)?.send().await?;
            let handle = endpoint
                .wrap_call(call_id)
                .wait_for_answered(Some(remote_test_timeout(provider)?))
                .await?;
            run_hold_on_handle(provider, cfg, handle.as_session_handle(), transport).await?;
        }
        (Scenario::HoldResume, Role::Callee) => {
            let incoming =
                timeout(remote_test_timeout(provider)?, endpoint.wait_for_incoming()).await??;
            let handle = incoming.accept().await?;
            run_answering_tone_role(
                cfg,
                handle.as_session_handle(),
                tone_for_callee(transport),
                hold_resume_callee_wav(transport),
                transport,
            )
            .await?;
        }
        (Scenario::RingCancel, Role::Caller) => {
            settle_after_register(provider).await;
            let call_id = endpoint.invite(&cfg.remote_call_uri())?.send().await?;
            let handle = endpoint.wrap_call(call_id);
            handle
                .as_session_handle()
                .wait_for_progress(
                    |event| {
                        matches!(
                            event,
                            Event::CallProgress {
                                status_code: 180 | 183,
                                ..
                            }
                        )
                    },
                    Some(remote_test_timeout(provider)?),
                )
                .await?;
            let mut events = handle.as_session_handle().events().await?;
            handle
                .hangup_and_wait(Some(Duration::from_secs(12)))
                .await?;
            wait_for_call_cancelled_on_events(&mut events, Duration::from_secs(12))
                .await
                .ok();
        }
        (Scenario::RingCancel, Role::Target) => {
            let incoming =
                timeout(remote_test_timeout(provider)?, endpoint.wait_for_incoming()).await??;
            let guard = incoming.defer(Duration::from_secs(30));
            let result = guard
                .wait_for_cancelled(Some(Duration::from_secs(12)))
                .await;
            if provider.expects_target_cancel() {
                result?;
            }
        }
        (Scenario::Dtmf, Role::Caller) => {
            settle_after_register(provider).await;
            let handle = endpoint
                .call_and_wait(
                    target_user_for(transport),
                    Some(remote_test_timeout(provider)?),
                )
                .await?;
            run_dtmf_caller(cfg, handle.as_session_handle(), transport).await?;
        }
        (Scenario::Dtmf, Role::Callee) => {
            let incoming =
                timeout(remote_test_timeout(provider)?, endpoint.wait_for_incoming()).await??;
            let handle = incoming.accept().await?;
            run_dtmf_callee(provider, cfg, handle.as_session_handle(), transport).await?;
        }
        (Scenario::Reject, Role::Caller) => {
            settle_after_register(provider).await;
            let call_id = endpoint.invite(target_user_for(transport))?.send().await?;
            let handle = endpoint.wrap_call(call_id);
            let mut events = handle.as_session_handle().events().await?;
            let (status, _) =
                wait_for_call_failed_on_events(&mut events, remote_test_timeout(provider)?).await?;
            if status != 486 {
                return Err(format!("expected 486 Busy Here, got {}", status).into());
            }
        }
        (Scenario::Reject, Role::Callee) => {
            let incoming =
                timeout(remote_test_timeout(provider)?, endpoint.wait_for_incoming()).await??;
            incoming.reject(486, "Busy Here").await?;
            sleep(Duration::from_secs(1)).await;
        }
        _ => return Err(format!("unsupported Endpoint role {:?} for {:?}", role, scenario).into()),
    }
    Ok(())
}

async fn run_callback_two_party(
    provider: PbxProvider,
    scenario: Scenario,
    transport: TransportMode,
    role: Role,
    runtime: &mut CallbackRuntime,
) -> ExampleResult<()> {
    match (scenario, role) {
        (Scenario::BasicCall, Role::Caller) => {
            settle_after_register(provider).await;
            let target = runtime.cfg.outbound_call_uri(target_user_for(transport));
            let handle =
                callback_call_with_answer_retry(runtime, &target, remote_test_timeout(provider)?)
                    .await?;
            run_basic_caller(&runtime.cfg, &handle, transport).await?;
        }
        (Scenario::BasicCall, Role::Callee) => {
            let handle =
                wait_for_next_established(&mut runtime.events, remote_test_timeout(provider)?)
                    .await?;
            run_basic_callee(provider, &runtime.cfg, &handle, transport).await?;
        }
        (Scenario::G729Call, Role::Caller) => {
            settle_after_register(provider).await;
            let target = runtime.cfg.outbound_call_uri(target_user_for(transport));
            let handle =
                callback_call_with_answer_retry(runtime, &target, remote_test_timeout(provider)?)
                    .await?;
            run_g729_caller(&runtime.cfg, &handle, transport).await?;
        }
        (Scenario::G729Call, Role::Callee) => {
            let handle =
                wait_for_next_established(&mut runtime.events, remote_test_timeout(provider)?)
                    .await?;
            run_g729_callee(runtime.cfg.provider, &runtime.cfg, &handle, transport).await?;
        }
        (Scenario::AmrCall, Role::Caller) => {
            settle_after_register(provider).await;
            let target = runtime.cfg.outbound_call_uri(target_user_for(transport));
            let handle =
                callback_call_with_answer_retry(runtime, &target, remote_test_timeout(provider)?)
                    .await?;
            run_amr_caller(&runtime.cfg, &handle, transport, amr_caller_wav(transport)).await?;
        }
        (Scenario::AmrCall, Role::Callee) => {
            let handle =
                wait_for_next_established(&mut runtime.events, remote_test_timeout(provider)?)
                    .await?;
            run_amr_callee(
                runtime.cfg.provider,
                &runtime.cfg,
                &handle,
                transport,
                amr_callee_wav(transport),
            )
            .await?;
        }
        (Scenario::AmrTranscodeCall, Role::Caller) => {
            settle_after_register(provider).await;
            let target = runtime.cfg.outbound_call_uri(target_user_for(transport));
            let handle =
                callback_call_with_answer_retry(runtime, &target, remote_test_timeout(provider)?)
                    .await?;
            let wav = amr_transcode_wav(&runtime.cfg);
            run_amr_caller(&runtime.cfg, &handle, transport, &wav).await?;
        }
        (Scenario::AmrTranscodeCall, Role::Callee) => {
            let handle =
                wait_for_next_established(&mut runtime.events, remote_test_timeout(provider)?)
                    .await?;
            let wav = amr_transcode_wav(&runtime.cfg);
            run_amr_callee(runtime.cfg.provider, &runtime.cfg, &handle, transport, &wav).await?;
        }
        (Scenario::HoldResume, Role::Caller) => {
            settle_after_register(provider).await;
            let target = runtime.cfg.outbound_call_uri(target_user_for(transport));
            let handle =
                callback_call_with_answer_retry(runtime, &target, remote_test_timeout(provider)?)
                    .await?;
            run_hold_on_handle(provider, &runtime.cfg, &handle, transport).await?;
            wait_for_local_hold_resume(&mut runtime.events, Duration::from_secs(15)).await?;
        }
        (Scenario::HoldResume, Role::Callee) => {
            let handle =
                wait_for_next_established(&mut runtime.events, remote_test_timeout(provider)?)
                    .await?;
            run_answering_tone_role(
                &runtime.cfg,
                &handle,
                tone_for_callee(transport),
                hold_resume_callee_wav(transport),
                transport,
            )
            .await?;
        }
        (Scenario::RingCancel, Role::Caller) => {
            settle_after_register(provider).await;
            let call_id = runtime
                .control
                .invite(runtime.cfg.remote_call_uri())
                .send()
                .await?;
            let handle = runtime.control.coordinator().session(&call_id);
            wait_for_callback_progress(
                &mut runtime.events,
                handle.id(),
                remote_test_timeout(provider)?,
            )
            .await?;
            handle
                .hangup_and_wait(Some(Duration::from_secs(12)))
                .await?;
            wait_for_cancelled(
                &mut runtime.events,
                Some(handle.id()),
                Duration::from_secs(12),
            )
            .await
            .ok();
        }
        (Scenario::RingCancel, Role::Target) => {
            let call_id =
                wait_for_incoming_notice(&mut runtime.events, remote_test_timeout(provider)?)
                    .await?;
            let result =
                wait_for_cancelled(&mut runtime.events, Some(&call_id), Duration::from_secs(12))
                    .await;
            if provider.expects_target_cancel() {
                result?;
            }
        }
        (Scenario::Dtmf, Role::Caller) => {
            settle_after_register(provider).await;
            let target = runtime.cfg.outbound_call_uri(target_user_for(transport));
            let handle =
                callback_call_with_answer_retry(runtime, &target, remote_test_timeout(provider)?)
                    .await?;
            run_dtmf_caller(&runtime.cfg, &handle, transport).await?;
        }
        (Scenario::Dtmf, Role::Callee) => {
            let handle =
                wait_for_next_established(&mut runtime.events, remote_test_timeout(provider)?)
                    .await?;
            // Unconditional for the same reason as run_dtmf_callee: the caller
            // holds its digits until it receives our tone.
            let recorder = start_tone_recorder(&handle, tone_for_callee(transport)).await?;
            wait_for_dtmf_sequence(
                &mut runtime.events,
                &remote_test_digits(provider),
                remote_test_timeout(provider)?,
            )
            .await?;
            handle
                .wait_for_end(Some(Duration::from_secs(15)))
                .await
                .ok();
            recorder
                .stop_and_save(&runtime.cfg.output_dir, dtmf_callee_wav(transport))
                .await?;
        }
        (Scenario::Reject, Role::Caller) => {
            settle_after_register(provider).await;
            let target = runtime.cfg.outbound_call_uri(target_user_for(transport));
            let call_id = runtime.control.invite(target).send().await?;
            let handle = runtime.control.coordinator().session(&call_id);
            wait_for_call_failed(
                &mut runtime.events,
                handle.id(),
                486,
                remote_test_timeout(provider)?,
            )
            .await?;
        }
        (Scenario::Reject, Role::Callee) => {
            let _call_id =
                wait_for_incoming_notice(&mut runtime.events, remote_test_timeout(provider)?)
                    .await?;
            sleep(Duration::from_secs(1)).await;
        }
        _ => return Err(format!("unsupported Callback role {:?} for {:?}", role, scenario).into()),
    }
    Ok(())
}

async fn run_stream_peer_transfer(
    provider: PbxProvider,
    transport: TransportMode,
    role: Role,
    cfg: &EndpointConfig,
    peer: &mut StreamPeer,
) -> ExampleResult<()> {
    match role {
        Role::Transferor => {
            settle_after_register(provider).await;
            let handle = call_with_answer_retry(
                peer,
                &cfg.outbound_call_uri(target_user_for(transport)),
                remote_test_timeout(provider)?,
            )
            .await?;
            run_transferor(provider, cfg, &handle, transport).await?;
        }
        Role::Transferee => {
            let incoming =
                timeout(remote_test_timeout(provider)?, peer.wait_for_incoming()).await??;
            let handle = incoming.accept().await?;
            run_transfer_answering_role(cfg, &handle, transport, true).await?;
        }
        Role::Target => {
            let incoming = timeout(Duration::from_secs(90), peer.wait_for_incoming()).await??;
            let handle = incoming.accept().await?;
            run_transfer_answering_role(cfg, &handle, transport, false).await?;
        }
        _ => return Err(format!("unsupported transfer role {:?}", role).into()),
    }
    Ok(())
}

async fn run_endpoint_transfer(
    provider: PbxProvider,
    transport: TransportMode,
    role: Role,
    cfg: &EndpointConfig,
    endpoint: &mut Endpoint,
) -> ExampleResult<()> {
    match role {
        Role::Transferor => {
            settle_after_register(provider).await;
            let handle = endpoint
                .call_and_wait(
                    target_user_for(transport),
                    Some(remote_test_timeout(provider)?),
                )
                .await?;
            run_transferor(provider, cfg, handle.as_session_handle(), transport).await?;
        }
        Role::Transferee => {
            let incoming =
                timeout(remote_test_timeout(provider)?, endpoint.wait_for_incoming()).await??;
            let handle = incoming.accept().await?;
            run_transfer_answering_role(cfg, handle.as_session_handle(), transport, true).await?;
        }
        Role::Target => {
            let incoming = timeout(Duration::from_secs(90), endpoint.wait_for_incoming()).await??;
            let handle = incoming.accept().await?;
            run_transfer_answering_role(cfg, handle.as_session_handle(), transport, false).await?;
        }
        _ => return Err(format!("unsupported transfer role {:?}", role).into()),
    }
    Ok(())
}

async fn run_callback_transfer(
    transport: TransportMode,
    role: Role,
    runtime: &mut CallbackRuntime,
) -> ExampleResult<()> {
    match role {
        Role::Transferor => {
            settle_after_register(runtime.cfg.provider).await;
            let target = runtime.cfg.outbound_call_uri(target_user_for(transport));
            let handle = callback_call_with_answer_retry(
                runtime,
                &target,
                remote_test_timeout(runtime.cfg.provider)?,
            )
            .await?;
            run_transferor(runtime.cfg.provider, &runtime.cfg, &handle, transport).await?;
        }
        Role::Transferee => {
            let handle = wait_for_next_established(
                &mut runtime.events,
                remote_test_timeout(runtime.cfg.provider)?,
            )
            .await?;
            run_transfer_answering_role(&runtime.cfg, &handle, transport, true).await?;
        }
        Role::Target => {
            let handle =
                wait_for_next_established(&mut runtime.events, Duration::from_secs(90)).await?;
            run_transfer_answering_role(&runtime.cfg, &handle, transport, false).await?;
        }
        _ => return Err(format!("unsupported transfer role {:?}", role).into()),
    }
    Ok(())
}

async fn run_hold_on_handle(
    _provider: PbxProvider,
    cfg: &EndpointConfig,
    handle: &SessionHandle,
    transport: TransportMode,
) -> ExampleResult<()> {
    if transport.is_tls() {
        assert_srtp_media_security(handle, Duration::from_secs(5)).await?;
    }
    let mut call_events = handle.events().await?;
    let audio = handle.audio().await?;
    let (sender, mut receiver) = audio.split();
    let received_buf = Arc::new(Mutex::new(Vec::<i16>::new()));
    let recv_buf = received_buf.clone();
    let recv_task = tokio::spawn(async move {
        while let Some(frame) = receiver.recv().await {
            if let Ok(mut buf) = recv_buf.lock() {
                buf.extend_from_slice(&frame.samples);
            }
        }
    });

    let mut frame_index = 0usize;
    send_tone_segment(
        &sender,
        ENDPOINT_1001_TONE_HZ,
        HOLD_RESUME_PRE_HOLD_FRAMES,
        &mut frame_index,
    )
    .await?;
    handle.hold().await?;
    wait_for_local_hold_on_events(&mut call_events, Duration::from_secs(8)).await?;
    send_tone_segment(&sender, 550.0, HOLD_RESUME_HELD_FRAMES, &mut frame_index).await?;
    sleep(Duration::from_millis(500)).await;
    handle.resume().await?;
    wait_for_local_resume_on_events(&mut call_events, Duration::from_secs(8)).await?;
    send_tone_segment(
        &sender,
        ENDPOINT_1003_TONE_HZ,
        HOLD_RESUME_POST_RESUME_FRAMES,
        &mut frame_index,
    )
    .await?;
    sleep(Duration::from_secs(2)).await;
    drop(sender);
    handle
        .hangup_and_wait(Some(Duration::from_secs(8)))
        .await
        .ok();
    stop_recv_task(recv_task).await;
    let received = received_buf.lock().map(|g| g.clone()).unwrap_or_default();
    save_wav(
        &cfg.output_dir,
        hold_resume_caller_wav(transport),
        &received,
    )?;
    Ok(())
}

async fn run_answering_tone_role(
    cfg: &EndpointConfig,
    handle: &SessionHandle,
    tone_hz: f32,
    wav_name: &str,
    transport: TransportMode,
) -> ExampleResult<()> {
    if transport.is_tls() {
        assert_srtp_media_security(handle, Duration::from_secs(5)).await?;
    }
    let recorder = start_tone_recorder(handle, tone_hz).await?;
    handle
        .wait_for_end(Some(Duration::from_secs(45)))
        .await
        .ok();
    recorder.stop_and_save(&cfg.output_dir, wav_name).await?;
    Ok(())
}

async fn run_basic_caller(
    cfg: &EndpointConfig,
    handle: &SessionHandle,
    transport: TransportMode,
) -> ExampleResult<()> {
    if transport.is_tls() {
        assert_srtp_media_security(handle, Duration::from_secs(5)).await?;
    }
    let recorder = start_tone_recorder(handle, tone_for_caller(transport)).await?;
    if let Err(error) = recorder
        .wait_for_received_samples(TONE_ANALYSIS_WINDOW_SAMPLES, Duration::from_secs(6))
        .await
    {
        handle
            .hangup_and_wait(Some(Duration::from_secs(8)))
            .await
            .ok();
        recorder
            .stop_and_save(&cfg.output_dir, g711_caller_wav(transport))
            .await
            .ok();
        return Err(error);
    }
    sleep(Duration::from_secs(4)).await;
    handle
        .hangup_and_wait(Some(Duration::from_secs(8)))
        .await
        .ok();
    recorder
        .stop_and_save(&cfg.output_dir, g711_caller_wav(transport))
        .await?;
    Ok(())
}

async fn run_basic_callee(
    provider: PbxProvider,
    cfg: &EndpointConfig,
    handle: &SessionHandle,
    transport: TransportMode,
) -> ExampleResult<()> {
    if transport.is_tls() {
        assert_srtp_media_security(handle, Duration::from_secs(5)).await?;
    }
    let recorder = start_tone_recorder(handle, tone_for_callee(transport)).await?;
    handle
        .wait_for_end(Some(remote_test_timeout(provider)?))
        .await
        .ok();
    recorder
        .stop_and_save(&cfg.output_dir, g711_callee_wav(transport))
        .await?;
    Ok(())
}

/// Stream a tone over a negotiated AMR call and keep what comes back.
///
/// The PBX is the peer here, not another rvoip endpoint — which is the whole
/// point. Asterisk parses our SDP, negotiates its own AMR framing, and either
/// relays or transcodes; none of that shares an assumption with our code.
async fn run_amr_caller(
    cfg: &EndpointConfig,
    handle: &SessionHandle,
    transport: TransportMode,
    wav_name: &str,
) -> ExampleResult<()> {
    run_amr_caller_toned(
        cfg,
        handle,
        transport,
        tone_for_caller(transport),
        tone_for_callee(transport),
        wav_name,
        None,
    )
    .await
}

/// The teardown-driving half of an AMR call, with the tones spelled out.
///
/// `send_hz` is what this side transmits, `expect_hz` what the far end sends
/// and this side must recover. The two-party `amr_call` uses the caller/callee
/// tones; `b2bua_call` reuses this with the far leg's tone, since its middle
/// node sends nothing of its own.
async fn run_amr_caller_toned(
    cfg: &EndpointConfig,
    handle: &SessionHandle,
    transport: TransportMode,
    send_hz: f32,
    expect_hz: f32,
    wav_name: &str,
    mode_switch: Option<&UnifiedCoordinator>,
) -> ExampleResult<()> {
    if transport.is_tls() {
        assert_srtp_media_security(handle, Duration::from_secs(5)).await?;
    }
    let sample_rate = amr_sample_rate(cfg.codec_profile);
    let recorder = start_tone_recorder_at_rate(
        handle,
        send_hz,
        amr_frame_size(cfg.codec_profile),
        sample_rate,
    )
    .await?;
    // The caller alone paces teardown, exactly as the G.729 pair does: it
    // captures the floor plus half a second and only then hangs up, so the
    // callee — which just waits for the call to end — necessarily holds at
    // least the floor by the time the BYE lands. The previous shape had both
    // roles racing to the same floor and hanging up, which was a coin flip
    // decided by scheduler slop; fixing the send clock made the slop small
    // enough that the loser came up exactly one frame short.
    let target = min_received_samples(sample_rate) + sample_rate as usize / 2;
    let outcome = recorder
        .wait_for_received_samples(target, Duration::from_secs(20))
        .await;
    // The mode switch runs only after the quality floor is secured, so the
    // gate's one continuous clean second exists regardless of how the lower
    // rate codes the tone. Sequenced, not concurrent: evidence first, then
    // the experiment on top of it.
    let mut switch_outcome: ExampleResult<()> = Ok(());
    if let (Some(coordinator), true) = (mode_switch, amr_mode_switch_requested()) {
        if outcome.is_ok() {
            switch_outcome = exercise_amr_mode_switch(coordinator, handle, cfg.codec_profile).await;
        }
    }
    // DTX, on the same sequencing principle as the mode switch: the quality
    // floor is already banked, so the silent window can only add evidence.
    let mut dtx_outcome: ExampleResult<()> = Ok(());
    if amr_dtx_requested() && outcome.is_ok() {
        let silence = Duration::from_secs(2);
        let received = recorder.hold_silence(silence).await;
        // The far end must keep delivering audio through our silence. DTX
        // replaces speech frames with SID updates and gaps on the wire, but
        // the *decoder* turns those into comfort noise, so the receive stream
        // stays continuous. A drop to nothing would mean the peer stopped
        // rather than went quiet -- the failure this scenario exists to catch.
        let expected = sample_rate as usize * silence.as_secs() as usize / 2;
        if received < expected {
            dtx_outcome = Err(format!(
                "dtx: only {received} samples arrived during a {}s silent window,                  expected at least {expected} (comfort noise should keep the                  stream continuous)",
                silence.as_secs()
            )
            .into());
        }
        diag_event(
            &cfg.output_dir,
            "amr_dtx_silence_window",
            serde_json::json!({
                "silence_secs": silence.as_secs(),
                "received_samples": received,
                "expected_at_least": expected,
            }),
        );
    }
    handle
        .hangup_and_wait(Some(Duration::from_secs(8)))
        .await
        .ok();
    let saved = recorder.stop_and_save(&cfg.output_dir, wav_name).await;
    outcome?;
    switch_outcome?;
    dtx_outcome?;
    let path = saved?;
    assert_amr_tone_quality(&path, sample_rate, expect_hz, send_hz)?;
    Ok(())
}

/// Ask the peer to drop to the lowest mode mid-call and prove it did.
///
/// Non-vacuous by construction: the peer must be observed at the profile's
/// *top* mode first (that is where every encoder opens), and at mode 0 after
/// — a stack that never emitted the CMR, or a peer that ignored it, fails
/// the second assertion; a test pointed at a stream that was already slow
/// fails the first.
async fn exercise_amr_mode_switch(
    coordinator: &UnifiedCoordinator,
    handle: &SessionHandle,
    profile: CodecProfile,
) -> ExampleResult<()> {
    let session = handle.id();
    let top = amr_top_mode_index(profile);
    let before = coordinator.peer_codec_mode(session).await;
    if before != Some(top) {
        return Err(format!(
            "mode switch: peer was sending mode {:?} before the request, expected the top mode {}",
            before, top
        )
        .into());
    }
    if !coordinator.request_peer_codec_mode(session, 0).await? {
        return Err("mode switch: session has no active media".into());
    }
    // The CMR rides the next outgoing payload (20 ms away); the peer's next
    // frame at the new rate needs one more round trip plus its encoder's
    // change policy. A second is generous; five covers a congested lab.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if coordinator.peer_codec_mode(session).await == Some(0) {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "mode switch: peer still sending mode {:?} five seconds after CMR 0",
                coordinator.peer_codec_mode(session).await
            )
            .into());
        }
        sleep(Duration::from_millis(40)).await;
    }
    // Half a second of audio at the new rate, so the switch is on the wire
    // long enough to appear in the capture and the PBX snapshots.
    sleep(Duration::from_millis(500)).await;
    println!(
        "[mode-switch] peer moved from mode {} to mode 0 on request",
        top
    );
    diag_event(
        &std::env::var("AUDIO_OUTPUT_DIR")
            .map(PathBuf::from)
            .unwrap_or_default(),
        "amr_mode_switch",
        serde_json::json!({ "from": top, "to": 0 }),
    );
    Ok(())
}

/// The answering half of [`run_amr_caller`].
async fn run_amr_callee(
    provider: PbxProvider,
    cfg: &EndpointConfig,
    handle: &SessionHandle,
    transport: TransportMode,
    wav_name: &str,
) -> ExampleResult<()> {
    run_amr_callee_toned(
        provider,
        cfg,
        handle,
        transport,
        tone_for_callee(transport),
        tone_for_caller(transport),
        wav_name,
    )
    .await
}

async fn run_amr_callee_toned(
    provider: PbxProvider,
    cfg: &EndpointConfig,
    handle: &SessionHandle,
    transport: TransportMode,
    send_hz: f32,
    expect_hz: f32,
    wav_name: &str,
) -> ExampleResult<()> {
    if transport.is_tls() {
        assert_srtp_media_security(handle, Duration::from_secs(5)).await?;
    }
    let sample_rate = amr_sample_rate(cfg.codec_profile);
    let recorder = start_tone_recorder_at_rate(
        handle,
        send_hz,
        amr_frame_size(cfg.codec_profile),
        sample_rate,
    )
    .await?;
    // No live sample wait and no hangup here: the caller stays up for the
    // floor plus a margin and drives teardown (see `run_amr_caller`). The
    // floor is asserted on the recording instead — the evidence bar is the
    // same, without two roles racing to hang up on each other.
    handle
        .wait_for_end(Some(remote_test_timeout(provider)?))
        .await
        .ok();
    let path = recorder.stop_and_save(&cfg.output_dir, wav_name).await?;
    let received = read_wav(&path)?;
    let floor = min_received_samples(sample_rate);
    if received.len() < floor {
        return Err(format!(
            "{}: {} samples received, below the {} floor ({} ms at {} Hz)",
            path.display(),
            received.len(),
            floor,
            MIN_RECEIVED_MS,
            sample_rate
        )
        .into());
    }
    assert_amr_tone_quality(&path, sample_rate, expect_hz, send_hz)?;
    Ok(())
}

/// Poll a session to a target state, bounded, exactly as
/// `examples/unified/04_b2bua_bridge/bridge_peer.rs` does.
async fn wait_for_call_state(
    coordinator: &UnifiedCoordinator,
    session: &SessionId,
    target: CallState,
    deadline: Duration,
) -> ExampleResult<()> {
    let end = tokio::time::Instant::now() + deadline;
    loop {
        let state = coordinator.get_state(session).await?;
        if state == target {
            return Ok(());
        }
        if tokio::time::Instant::now() >= end {
            return Err(format!(
                "session {} never reached {:?} (stuck at {:?})",
                session.0, target, state
            )
            .into());
        }
        sleep(Duration::from_millis(50)).await;
    }
}

/// rvoip as the B2BUA: accept the caller's inbound leg, originate an outbound
/// leg to the target through the PBX, and bridge the two.
///
/// This composes the same coordinator primitives the CI-proven
/// `04_b2bua_bridge` example does, in outbound-first order: the target leg is
/// answered before the caller's INVITE is accepted, so by the time the caller
/// sees the call established both legs already have media, which keeps the
/// caller's received-sample clock honest. It reaches the coordinator through
/// the endpoint's own control handle rather than `SipB2bua`, because the
/// harness needs both leg session ids to tear down and (under TLS) to assert
/// SRTP on each — `SipB2bua::handle_inbound` returns only a media-core bridge
/// handle.
///
/// The two legs share the caller's negotiated codec because the b2bua's own
/// `EndpointConfig` offers the same profile: `bridge` forwards payloads
/// without transcoding and refuses (`CodecMismatch` / a framing
/// `FormatMismatch`) if the two legs disagree, so a cell that passes is proof
/// the same codec crossed both legs and rvoip relayed it.
async fn run_b2bua_bridge_role(
    provider: PbxProvider,
    transport: TransportMode,
    cfg: &EndpointConfig,
    endpoint: &Endpoint,
) -> ExampleResult<()> {
    let coordinator = endpoint.control().coordinator().clone();
    let mut events = coordinator.events().await?;

    // Wait for the caller's INVITE. The unfiltered stream is only used to
    // learn the inbound session id; per-leg streams take over after.
    let inbound_id = timeout(remote_test_timeout(provider)?, async {
        loop {
            match events.next().await {
                Some(Event::IncomingCall { call_id, from, .. }) => {
                    diag_event(
                        &cfg.output_dir,
                        "b2bua_inbound",
                        serde_json::json!({ "from": from, "leg_a": call_id.0 }),
                    );
                    println!("[b2bua] inbound leg A = {} from {}", call_id.0, from);
                    return call_id;
                }
                Some(_) => continue,
                None => unreachable!("event stream closed before an inbound call"),
            }
        }
    })
    .await
    .map_err(|_| "b2bua never received the caller's INVITE")?;

    // Originate the outbound leg to the target (2003/1003) through the PBX.
    let outbound_id = coordinator
        .invite(Some(cfg.aor_uri()), cfg.remote_call_uri())
        .send()
        .await?;
    println!(
        "[b2bua] outbound leg B = {} to {}",
        outbound_id.0,
        cfg.remote_call_uri()
    );
    let mut outbound_events = coordinator.events_for_session(&outbound_id).await?;

    let answered = timeout(remote_test_timeout(provider)?, async {
        loop {
            match outbound_events.next().await? {
                Event::CallAnswered { .. } => return Some(()),
                Event::CallEnded { .. } | Event::CallFailed { .. } => return None,
                _ => continue,
            }
        }
    })
    .await;
    match answered {
        Ok(Some(())) => {}
        Ok(None) => return Err("b2bua outbound leg terminated before answering".into()),
        Err(_) => return Err("b2bua outbound leg answer timeout".into()),
    }

    coordinator.accept_call(&inbound_id).await?;
    wait_for_call_state(
        &coordinator,
        &inbound_id,
        CallState::Active,
        Duration::from_secs(10),
    )
    .await?;
    wait_for_call_state(
        &coordinator,
        &outbound_id,
        CallState::Active,
        Duration::from_secs(10),
    )
    .await?;

    if transport.is_tls() {
        // Each leg negotiates its own SRTP; both must be secured before we
        // relay decrypted payloads between them.
        assert_srtp_media_security(&coordinator.session(&inbound_id), Duration::from_secs(5))
            .await?;
        assert_srtp_media_security(&coordinator.session(&outbound_id), Duration::from_secs(5))
            .await?;
    }

    let bridge = coordinator.bridge(&inbound_id, &outbound_id).await?;
    diag_event(
        &cfg.output_dir,
        "b2bua_bridged",
        serde_json::json!({ "inbound": inbound_id.0, "outbound": outbound_id.0 }),
    );
    println!("[b2bua] bridged {} <-> {}", inbound_id.0, outbound_id.0);

    // The caller drives teardown once it has its evidence; the b2bua just
    // holds the bridge until the inbound leg ends, then closes the relay
    // before hanging up the outbound leg.
    coordinator
        .session(&inbound_id)
        .wait_for_end(Some(remote_test_timeout(provider)?))
        .await
        .ok();
    drop(bridge);
    let _ = coordinator.hangup(&outbound_id).await;
    let _ = timeout(Duration::from_secs(3), outbound_events.next()).await;
    Ok(())
}

async fn run_endpoint_b2bua(
    provider: PbxProvider,
    transport: TransportMode,
    role: Role,
    cfg: &EndpointConfig,
    endpoint: &mut Endpoint,
) -> ExampleResult<()> {
    match role {
        Role::Caller => {
            settle_after_register(provider).await;
            let handle = endpoint
                .call_and_wait(
                    target_user_for(transport),
                    Some(remote_test_timeout(provider)?),
                )
                .await?;
            run_amr_caller_toned(
                cfg,
                handle.as_session_handle(),
                transport,
                tone_for_caller(transport),
                tone_for_b2bua_far(transport),
                b2bua_caller_wav(transport),
                Some(endpoint.control().coordinator()),
            )
            .await?;
        }
        Role::B2bua => {
            run_b2bua_bridge_role(provider, transport, cfg, endpoint).await?;
        }
        Role::Target => {
            // The target answers, sends 660 Hz, and waits for the call to
            // end. It is `run_amr_callee_toned` with the far tone being the
            // caller's, two PBX hops away through the bridge.
            let incoming =
                timeout(remote_test_timeout(provider)?, endpoint.wait_for_incoming()).await??;
            let handle = incoming.accept().await?;
            run_amr_callee_toned(
                provider,
                cfg,
                handle.as_session_handle(),
                transport,
                tone_for_b2bua_far(transport),
                tone_for_caller(transport),
                b2bua_target_wav(transport),
            )
            .await?;
        }
        other => {
            return Err(format!("unsupported endpoint role {:?} for b2bua_call", other).into());
        }
    }
    Ok(())
}

async fn run_g729_caller(
    cfg: &EndpointConfig,
    handle: &SessionHandle,
    transport: TransportMode,
) -> ExampleResult<()> {
    if transport.is_tls() {
        assert_srtp_media_security(handle, Duration::from_secs(5)).await?;
    }
    let recorder =
        start_tone_recorder_with_frame_size(handle, tone_for_caller(transport), G729_FRAME_SIZE)
            .await?;
    if let Err(error) = recorder
        .wait_for_received_samples(G729_CALLER_CAPTURE_TARGET_SAMPLES, Duration::from_secs(15))
        .await
    {
        handle
            .hangup_and_wait(Some(Duration::from_secs(8)))
            .await
            .ok();
        recorder
            .stop_and_save(&cfg.output_dir, g729_caller_wav(transport))
            .await
            .ok();
        return Err(error);
    }
    handle
        .hangup_and_wait(Some(Duration::from_secs(8)))
        .await
        .ok();
    recorder
        .stop_and_save(&cfg.output_dir, g729_caller_wav(transport))
        .await?;
    Ok(())
}

async fn run_g729_callee(
    provider: PbxProvider,
    cfg: &EndpointConfig,
    handle: &SessionHandle,
    transport: TransportMode,
) -> ExampleResult<()> {
    if transport.is_tls() {
        assert_srtp_media_security(handle, Duration::from_secs(5)).await?;
    }
    let recorder =
        start_tone_recorder_with_frame_size(handle, tone_for_callee(transport), G729_FRAME_SIZE)
            .await?;
    handle
        .wait_for_end(Some(remote_test_timeout(provider)?))
        .await
        .ok();
    recorder
        .stop_and_save(&cfg.output_dir, g729_callee_wav(transport))
        .await?;
    Ok(())
}

async fn run_deferred_target(
    provider: PbxProvider,
    peer: &mut StreamPeer,
    _cfg: &EndpointConfig,
) -> ExampleResult<()> {
    let incoming = timeout(remote_test_timeout(provider)?, peer.wait_for_incoming()).await??;
    let guard = incoming.defer(Duration::from_secs(30));
    let result = guard
        .wait_for_cancelled(Some(Duration::from_secs(12)))
        .await;
    if provider.expects_target_cancel() {
        result?;
    }
    Ok(())
}

async fn run_dtmf_caller(
    cfg: &EndpointConfig,
    handle: &SessionHandle,
    transport: TransportMode,
) -> ExampleResult<()> {
    if transport.is_tls() {
        assert_srtp_media_security(handle, Duration::from_secs(5)).await?;
    }
    // Record on both transports: the tone stream is also how we prove the media
    // path is live before clocking out digits.
    let recorder = start_tone_recorder(handle, tone_for_caller(transport)).await?;
    // Only start the digit train once RTP is actually flowing both ways. A digit
    // handed to send_dtmf before the media path is up is scheduled against a
    // dialog that cannot transmit it, and it is dropped with no error — the
    // callee then waits for a digit that was never sent and only learns anything
    // is wrong when our BYE arrives ("call ended before DTMF completed"). The TLS
    // path got this gate for free from assert_srtp_media_security; UDP previously
    // started sending 500ms after answer and relied on that being enough.
    let media_ready = recorder
        .wait_for_received_samples(MIN_RECEIVED_SAMPLES, Duration::from_secs(6))
        .await;
    if let Err(error) = media_ready {
        handle
            .hangup_and_wait(Some(Duration::from_secs(8)))
            .await
            .ok();
        recorder
            .stop_and_save(&cfg.output_dir, dtmf_caller_wav(transport))
            .await
            .ok();
        return Err(error);
    }
    for digit in remote_test_digits(cfg.provider) {
        sleep(Duration::from_millis(500)).await;
        handle.send_dtmf(digit).await?;
    }
    sleep(Duration::from_secs(1)).await;
    handle.hangup_and_wait(Some(Duration::from_secs(8))).await?;
    recorder
        .stop_and_save(&cfg.output_dir, dtmf_caller_wav(transport))
        .await?;
    Ok(())
}

async fn run_dtmf_callee(
    provider: PbxProvider,
    cfg: &EndpointConfig,
    handle: &SessionHandle,
    transport: TransportMode,
) -> ExampleResult<()> {
    if transport.is_tls() {
        assert_srtp_media_security(handle, Duration::from_secs(5)).await?;
    }
    // Record on both transports. Beyond the capture, this is what feeds the
    // caller's media-readiness gate in run_dtmf_caller: it waits for our tone
    // before sending any digit, so a UDP callee that stayed silent would stall it.
    let recorder = start_tone_recorder(handle, tone_for_callee(transport)).await?;
    let mut events = handle.events().await?;
    wait_for_dtmf_sequence_on_events(
        &mut events,
        &remote_test_digits(provider),
        remote_test_timeout(provider)?,
    )
    .await?;
    handle
        .wait_for_end(Some(Duration::from_secs(15)))
        .await
        .ok();
    recorder
        .stop_and_save(&cfg.output_dir, dtmf_callee_wav(transport))
        .await?;
    Ok(())
}

async fn run_transferor(
    provider: PbxProvider,
    cfg: &EndpointConfig,
    handle: &SessionHandle,
    transport: TransportMode,
) -> ExampleResult<()> {
    diag_event(
        &cfg.output_dir,
        "call_established",
        serde_json::json!({
            "call_id": diag_call_id(handle),
            "role_detail": "transferor"
        }),
    );
    if transport.is_tls() {
        assert_srtp_media_security(handle, Duration::from_secs(5)).await?;
        diag_event(
            &cfg.output_dir,
            "srtp_asserted",
            serde_json::json!({ "call_id": diag_call_id(handle) }),
        );
    }
    let recorder = if transport.is_tls() {
        Some(start_tone_recorder(handle, ENDPOINT_1001_TONE_HZ).await?)
    } else {
        None
    };
    sleep(transfer_settle_duration(provider, transport)).await;
    diag_event(
        &cfg.output_dir,
        "refer_start",
        serde_json::json!({
            "call_id": diag_call_id(handle),
            "refer_to": cfg.remote_call_uri()
        }),
    );
    let transfer_outcome = handle
        .transfer_blind_and_wait_for_outcome(
            &cfg.remote_call_uri(),
            TransferWaitMode::NotifyFinal,
            Some(remote_test_timeout(provider)?),
        )
        .await?;
    match transfer_outcome {
        TransferOutcome::ReferCompleted {
            status_code,
            reason,
            ..
        } => {
            println!("[transfer] REFER completed: {} {}", status_code, reason);
            diag_event(
                &cfg.output_dir,
                "refer_outcome",
                serde_json::json!({
                    "call_id": diag_call_id(handle),
                    "outcome": "completed",
                    "status_code": status_code,
                    "reason": reason.as_str()
                }),
            );
        }
        TransferOutcome::Failed {
            status_code,
            reason,
            ..
        } => {
            diag_event(
                &cfg.output_dir,
                "refer_outcome",
                serde_json::json!({
                    "call_id": diag_call_id(handle),
                    "outcome": "failed",
                    "status_code": status_code,
                    "reason": reason.as_str()
                }),
            );
            return Err(format!("REFER failed: {} {}", status_code, reason).into());
        }
        other => return Err(format!("unexpected transfer outcome: {:?}", other).into()),
    }
    let default_post_refer_settle = match (provider, transport) {
        (PbxProvider::Asterisk, TransportMode::TlsSrtp) => 2,
        _ => 0,
    };
    let post_refer_settle = env_duration_secs(
        "PBX_TRANSFER_POST_REFER_SETTLE_SECS",
        default_post_refer_settle,
    );
    if !post_refer_settle.is_zero() {
        diag_event(
            &cfg.output_dir,
            "post_refer_settle_start",
            serde_json::json!({
                "call_id": diag_call_id(handle),
                "duration_ms": post_refer_settle.as_millis().min(u128::from(u64::MAX)) as u64
            }),
        );
        sleep(post_refer_settle).await;
        diag_event(
            &cfg.output_dir,
            "post_refer_settle_end",
            serde_json::json!({ "call_id": diag_call_id(handle) }),
        );
    }
    if let Some(recorder) = recorder {
        recorder
            .stop_and_save(&cfg.output_dir, transferor_wav(transport))
            .await?;
    }
    // RFC 5589 §6.1: after a successful blind transfer (final NOTIFY received),
    // the Transferor terminates its leg of the original call with BYE. Without
    // this the dialog stays open on the PBX, which eventually retransmits a
    // BYE that may land on whichever process binds the original Contact next.
    diag_event(
        &cfg.output_dir,
        "hangup_start",
        serde_json::json!({ "call_id": diag_call_id(handle) }),
    );
    handle
        .hangup_and_wait(Some(Duration::from_secs(8)))
        .await
        .ok();
    diag_event(
        &cfg.output_dir,
        "hangup_end",
        serde_json::json!({ "call_id": diag_call_id(handle) }),
    );
    Ok(())
}

async fn run_transfer_answering_role(
    cfg: &EndpointConfig,
    handle: &SessionHandle,
    transport: TransportMode,
    transferee: bool,
) -> ExampleResult<()> {
    diag_event(
        &cfg.output_dir,
        "call_established",
        serde_json::json!({
            "call_id": diag_call_id(handle),
            "role_detail": if transferee { "transferee" } else { "target" }
        }),
    );
    if transport.is_tls() {
        assert_srtp_media_security(handle, Duration::from_secs(5)).await?;
        diag_event(
            &cfg.output_dir,
            "srtp_asserted",
            serde_json::json!({ "call_id": diag_call_id(handle) }),
        );
    }
    let recorder = if transport.is_tls() {
        let tone = if transferee {
            ENDPOINT_1002_TONE_HZ
        } else {
            ENDPOINT_1003_TONE_HZ
        };
        Some(start_tone_recorder(handle, tone).await?)
    } else {
        None
    };
    let hold_duration = if transferee {
        let default = match (cfg.provider, transport) {
            (PbxProvider::Asterisk, TransportMode::TlsSrtp) => 14,
            _ => 12,
        };
        env_duration_secs("PBX_TRANSFER_TRANSFEREE_DURATION_SECS", default)
    } else {
        let default = match (cfg.provider, transport) {
            (PbxProvider::Asterisk, TransportMode::TlsSrtp) => 8,
            _ => 4,
        };
        env_duration_secs("PBX_TRANSFER_TARGET_DURATION_SECS", default)
    };
    diag_event(
        &cfg.output_dir,
        "transfer_role_hold_start",
        serde_json::json!({
            "call_id": diag_call_id(handle),
            "duration_ms": hold_duration.as_millis().min(u128::from(u64::MAX)) as u64
        }),
    );
    sleep(hold_duration).await;
    diag_event(
        &cfg.output_dir,
        "transfer_role_hold_end",
        serde_json::json!({ "call_id": diag_call_id(handle) }),
    );
    diag_event(
        &cfg.output_dir,
        "hangup_start",
        serde_json::json!({ "call_id": diag_call_id(handle) }),
    );
    handle
        .hangup_and_wait(Some(Duration::from_secs(8)))
        .await
        .ok();
    diag_event(
        &cfg.output_dir,
        "hangup_end",
        serde_json::json!({ "call_id": diag_call_id(handle) }),
    );
    if let Some(recorder) = recorder {
        let name = if transferee {
            transferee_wav(transport)
        } else {
            transfer_target_wav(transport)
        };
        recorder.stop_and_save(&cfg.output_dir, name).await?;
    }
    Ok(())
}

pub async fn run_analyze() -> ExampleResult<()> {
    let provider = PbxProvider::from_env_or_args()?;
    load_env(provider);
    init_tracing();
    let scenario = Scenario::from_env_or_args()?;
    let transport = TransportMode::from_env_or_args()?;
    let cfg = EndpointConfig::new(provider, username_for(transport, Role::Caller), transport)?;
    match scenario {
        Scenario::BasicCall => analyze_basic(&cfg, transport),
        Scenario::G729Call => analyze_g729(&cfg, transport),
        Scenario::AmrCall => analyze_amr(&cfg, transport),
        Scenario::B2buaCall => analyze_b2bua(&cfg, transport),
        Scenario::HoldResume => analyze_hold(&cfg, transport),
        Scenario::Dtmf if transport.is_tls() => analyze_dtmf(&cfg, transport),
        Scenario::BlindTransfer if transport.is_tls() => analyze_transfer(&cfg, transport),
        _ => {
            println!(
                "No WAV analysis is required for {:?} over {:?}.",
                scenario, transport
            );
            Ok(())
        }
    }
}

fn analyze_basic(cfg: &EndpointConfig, transport: TransportMode) -> ExampleResult<()> {
    write_audio_diagnostics(cfg, Scenario::BasicCall, transport);
    let caller_wav = cfg.output_dir.join(g711_caller_wav(transport));
    let callee_wav = cfg.output_dir.join(g711_callee_wav(transport));
    let caller = assert_audio_path(
        &caller_wav,
        tone_for_callee(transport),
        tone_for_caller(transport),
    )?;
    let callee = assert_audio_path(
        &callee_wav,
        tone_for_caller(transport),
        tone_for_callee(transport),
    )?;
    print_analysis("caller received callee G.711 tone", &caller_wav, &caller);
    print_analysis("callee received caller G.711 tone", &callee_wav, &callee);
    Ok(())
}

fn analyze_g729(cfg: &EndpointConfig, transport: TransportMode) -> ExampleResult<()> {
    write_audio_diagnostics(cfg, Scenario::G729Call, transport);
    let caller_wav = cfg.output_dir.join(g729_caller_wav(transport));
    let callee_wav = cfg.output_dir.join(g729_callee_wav(transport));
    let caller = assert_audio_path(
        &caller_wav,
        tone_for_callee(transport),
        tone_for_caller(transport),
    )?;
    let callee = assert_audio_path(
        &callee_wav,
        tone_for_caller(transport),
        tone_for_callee(transport),
    )?;
    print_analysis("caller received callee G.729 tone", &caller_wav, &caller);
    print_analysis("callee received caller G.729 tone", &callee_wav, &callee);
    Ok(())
}

/// Re-judge existing AMR captures without placing a call.
///
/// The interop WAVs are gitignored, so this analyzer is the only way to check
/// a threshold change against real captured evidence rather than re-running
/// two PBXes. `PBX_CODEC_PROFILE` selects the profile whose rate the capture
/// was made at, exactly as it selected it during the call.
fn analyze_amr(cfg: &EndpointConfig, transport: TransportMode) -> ExampleResult<()> {
    write_audio_diagnostics(cfg, Scenario::AmrCall, transport);
    let rate = amr_sample_rate(cfg.codec_profile);
    let caller_wav = cfg.output_dir.join(amr_caller_wav(transport));
    let callee_wav = cfg.output_dir.join(amr_callee_wav(transport));
    assert_amr_tone_quality(
        &caller_wav,
        rate,
        tone_for_callee(transport),
        tone_for_caller(transport),
    )?;
    assert_amr_tone_quality(
        &callee_wav,
        rate,
        tone_for_caller(transport),
        tone_for_callee(transport),
    )?;
    Ok(())
}

/// The caller recovers the target's 660 Hz and the target recovers the
/// caller's tone, each two PBX hops away through rvoip's bridge; the b2bua
/// middle node records nothing. Same quality gate as `analyze_amr`.
fn analyze_b2bua(cfg: &EndpointConfig, transport: TransportMode) -> ExampleResult<()> {
    write_audio_diagnostics(cfg, Scenario::B2buaCall, transport);
    let rate = amr_sample_rate(cfg.codec_profile);
    let caller_wav = cfg.output_dir.join(b2bua_caller_wav(transport));
    let target_wav = cfg.output_dir.join(b2bua_target_wav(transport));
    assert_amr_tone_quality(
        &caller_wav,
        rate,
        tone_for_b2bua_far(transport),
        tone_for_caller(transport),
    )?;
    assert_amr_tone_quality(
        &target_wav,
        rate,
        tone_for_caller(transport),
        tone_for_b2bua_far(transport),
    )?;
    Ok(())
}

fn analyze_hold(cfg: &EndpointConfig, transport: TransportMode) -> ExampleResult<()> {
    write_audio_diagnostics(cfg, Scenario::HoldResume, transport);
    let caller_wav = cfg.output_dir.join(hold_resume_caller_wav(transport));
    let callee_wav = cfg.output_dir.join(hold_resume_callee_wav(transport));
    let caller = assert_audio_path(
        &caller_wav,
        tone_for_callee(transport),
        ENDPOINT_1001_TONE_HZ,
    )?;
    let callee_samples = read_wav(&callee_wav)?;
    let pre_hold = assert_best_window_tone(
        "callee pre-hold caller tone",
        leading_third(&callee_samples),
        SAMPLE_RATE,
        SAMPLE_RATE as usize,
        FRAME_SIZE,
        ENDPOINT_1001_TONE_HZ,
        ENDPOINT_1003_TONE_HZ,
    )?;
    let post_resume = assert_best_window_tone(
        "callee post-resume caller tone",
        trailing_third(&callee_samples),
        SAMPLE_RATE,
        SAMPLE_RATE as usize,
        FRAME_SIZE,
        ENDPOINT_1003_TONE_HZ,
        ENDPOINT_1002_TONE_HZ,
    )?;
    print_analysis(
        "caller received callee reference tone",
        &caller_wav,
        &caller,
    );
    print_analysis("callee pre-hold caller tone", &callee_wav, &pre_hold);
    print_analysis("callee post-resume caller tone", &callee_wav, &post_resume);
    Ok(())
}

fn analyze_dtmf(cfg: &EndpointConfig, transport: TransportMode) -> ExampleResult<()> {
    write_audio_diagnostics(cfg, Scenario::Dtmf, transport);
    let caller_wav = cfg.output_dir.join(dtmf_caller_wav(transport));
    let callee_wav = cfg.output_dir.join(dtmf_callee_wav(transport));
    let caller = assert_audio_path(&caller_wav, ENDPOINT_1002_TONE_HZ, ENDPOINT_1001_TONE_HZ)?;
    let callee = assert_audio_path(&callee_wav, ENDPOINT_1001_TONE_HZ, ENDPOINT_1002_TONE_HZ)?;
    print_analysis("1001 received 1002 reference tone", &caller_wav, &caller);
    print_analysis("1002 received 1001 reference tone", &callee_wav, &callee);
    Ok(())
}

fn analyze_transfer(cfg: &EndpointConfig, transport: TransportMode) -> ExampleResult<()> {
    const WINDOW_SAMPLES: usize = SAMPLE_RATE as usize;
    const MIN_TRANSFEREE_SAMPLES: usize = WINDOW_SAMPLES * 2;

    write_audio_diagnostics(cfg, Scenario::BlindTransfer, transport);
    let transferor_wav = cfg.output_dir.join(transferor_wav(transport));
    let transferee_wav = cfg.output_dir.join(transferee_wav(transport));
    let target_wav = cfg.output_dir.join(transfer_target_wav(transport));
    let transferor = assert_audio_path(
        &transferor_wav,
        ENDPOINT_1002_TONE_HZ,
        ENDPOINT_1001_TONE_HZ,
    )?;
    let target = assert_audio_path(&target_wav, ENDPOINT_1002_TONE_HZ, ENDPOINT_1003_TONE_HZ)?;
    let transferee_samples = read_wav(&transferee_wav)?;
    if transferee_samples.len() < MIN_TRANSFEREE_SAMPLES {
        return Err(format!(
            "{} too short: {} samples (expected at least {})",
            transferee_wav.display(),
            transferee_samples.len(),
            MIN_TRANSFEREE_SAMPLES
        )
        .into());
    }
    let initial = assert_best_window_tone(
        "1002 initial leg received 1001 tone",
        leading_third(&transferee_samples),
        SAMPLE_RATE,
        WINDOW_SAMPLES,
        FRAME_SIZE,
        ENDPOINT_1001_TONE_HZ,
        ENDPOINT_1003_TONE_HZ,
    )?;
    let transferred = assert_best_window_tone(
        "1002 transferred leg received 1003 tone",
        trailing_third(&transferee_samples),
        SAMPLE_RATE,
        WINDOW_SAMPLES,
        FRAME_SIZE,
        ENDPOINT_1003_TONE_HZ,
        ENDPOINT_1001_TONE_HZ,
    )?;
    print_analysis(
        "1001 received 1002 initial-leg tone",
        &transferor_wav,
        &transferor,
    );
    print_analysis(
        "1003 received 1002 transferred-leg tone",
        &target_wav,
        &target,
    );
    print_analysis(
        "1002 initial window received 1001 tone",
        &transferee_wav,
        &initial,
    );
    print_analysis(
        "1002 final window received 1003 tone",
        &transferee_wav,
        &transferred,
    );
    Ok(())
}

fn write_audio_diagnostics(cfg: &EndpointConfig, scenario: Scenario, transport: TransportMode) {
    if !pbx_diag_enabled() {
        return;
    }
    if let Err(error) = write_audio_diagnostics_inner(cfg, scenario, transport) {
        let _ = std::fs::write(
            cfg.output_dir.join("audio-analysis-error.txt"),
            format!("{}\n", error),
        );
    }
}

fn write_audio_diagnostics_inner(
    cfg: &EndpointConfig,
    scenario: Scenario,
    transport: TransportMode,
) -> ExampleResult<()> {
    let mut files = Vec::new();
    let mut markdown = String::new();
    markdown.push_str("# PBX Audio Diagnostics\n\n");
    markdown.push_str(&format!("- scenario: {:?}\n", scenario));
    markdown.push_str(&format!("- transport: {:?}\n\n", transport));

    match scenario {
        Scenario::BasicCall => {
            add_audio_file_diagnostics(
                &mut files,
                &mut markdown,
                SAMPLE_RATE,
                "g711 caller",
                &cfg.output_dir.join(g711_caller_wav(transport)),
                &[(
                    "caller received callee G.711 tone",
                    WindowSelector::Stable,
                    tone_for_callee(transport),
                    tone_for_caller(transport),
                )],
            );
            add_audio_file_diagnostics(
                &mut files,
                &mut markdown,
                SAMPLE_RATE,
                "g711 callee",
                &cfg.output_dir.join(g711_callee_wav(transport)),
                &[(
                    "callee received caller G.711 tone",
                    WindowSelector::Stable,
                    tone_for_caller(transport),
                    tone_for_callee(transport),
                )],
            );
        }
        Scenario::G729Call => {
            add_audio_file_diagnostics(
                &mut files,
                &mut markdown,
                SAMPLE_RATE,
                "g729 caller",
                &cfg.output_dir.join(g729_caller_wav(transport)),
                &[(
                    "caller received callee G.729 tone",
                    WindowSelector::Stable,
                    tone_for_callee(transport),
                    tone_for_caller(transport),
                )],
            );
            add_audio_file_diagnostics(
                &mut files,
                &mut markdown,
                SAMPLE_RATE,
                "g729 callee",
                &cfg.output_dir.join(g729_callee_wav(transport)),
                &[(
                    "callee received caller G.729 tone",
                    WindowSelector::Stable,
                    tone_for_caller(transport),
                    tone_for_callee(transport),
                )],
            );
        }
        Scenario::HoldResume => {
            add_audio_file_diagnostics(
                &mut files,
                &mut markdown,
                SAMPLE_RATE,
                "hold caller",
                &cfg.output_dir.join(hold_resume_caller_wav(transport)),
                &[(
                    "caller received callee reference tone",
                    WindowSelector::Stable,
                    tone_for_callee(transport),
                    ENDPOINT_1001_TONE_HZ,
                )],
            );
            add_audio_file_diagnostics(
                &mut files,
                &mut markdown,
                SAMPLE_RATE,
                "hold callee",
                &cfg.output_dir.join(hold_resume_callee_wav(transport)),
                &[
                    (
                        "callee pre-hold caller tone",
                        WindowSelector::LeadingThird,
                        ENDPOINT_1001_TONE_HZ,
                        ENDPOINT_1003_TONE_HZ,
                    ),
                    (
                        "callee post-resume caller tone",
                        WindowSelector::TrailingThird,
                        ENDPOINT_1003_TONE_HZ,
                        ENDPOINT_1002_TONE_HZ,
                    ),
                ],
            );
        }
        Scenario::Dtmf => {
            add_audio_file_diagnostics(
                &mut files,
                &mut markdown,
                SAMPLE_RATE,
                "dtmf caller",
                &cfg.output_dir.join(dtmf_caller_wav(transport)),
                &[(
                    "1001 received 1002 reference tone",
                    WindowSelector::Stable,
                    ENDPOINT_1002_TONE_HZ,
                    ENDPOINT_1001_TONE_HZ,
                )],
            );
            add_audio_file_diagnostics(
                &mut files,
                &mut markdown,
                SAMPLE_RATE,
                "dtmf callee",
                &cfg.output_dir.join(dtmf_callee_wav(transport)),
                &[(
                    "1002 received 1001 reference tone",
                    WindowSelector::Stable,
                    ENDPOINT_1001_TONE_HZ,
                    ENDPOINT_1002_TONE_HZ,
                )],
            );
        }
        Scenario::BlindTransfer => {
            add_audio_file_diagnostics(
                &mut files,
                &mut markdown,
                SAMPLE_RATE,
                "transferor",
                &cfg.output_dir.join(transferor_wav(transport)),
                &[(
                    "1001 received 1002 initial-leg tone",
                    WindowSelector::Stable,
                    ENDPOINT_1002_TONE_HZ,
                    ENDPOINT_1001_TONE_HZ,
                )],
            );
            add_audio_file_diagnostics(
                &mut files,
                &mut markdown,
                SAMPLE_RATE,
                "transferee",
                &cfg.output_dir.join(transferee_wav(transport)),
                &[
                    (
                        "1002 initial leg received 1001 tone",
                        WindowSelector::LeadingThird,
                        ENDPOINT_1001_TONE_HZ,
                        ENDPOINT_1003_TONE_HZ,
                    ),
                    (
                        "1002 transferred leg received 1003 tone",
                        WindowSelector::TrailingThird,
                        ENDPOINT_1003_TONE_HZ,
                        ENDPOINT_1001_TONE_HZ,
                    ),
                ],
            );
            add_audio_file_diagnostics(
                &mut files,
                &mut markdown,
                SAMPLE_RATE,
                "target",
                &cfg.output_dir.join(transfer_target_wav(transport)),
                &[(
                    "1003 received 1002 transferred-leg tone",
                    WindowSelector::Stable,
                    ENDPOINT_1002_TONE_HZ,
                    ENDPOINT_1003_TONE_HZ,
                )],
            );
        }
        Scenario::AmrCall => {
            // The one scenario whose captures are not 8 kHz: the rate follows
            // the negotiated profile, and so do the file duration, the
            // analysis window and the Goertzel bins inside.
            let rate = amr_sample_rate(cfg.codec_profile);
            add_audio_file_diagnostics(
                &mut files,
                &mut markdown,
                rate,
                "amr caller",
                &cfg.output_dir.join(amr_caller_wav(transport)),
                &[(
                    "caller received callee AMR tone",
                    WindowSelector::Stable,
                    tone_for_callee(transport),
                    tone_for_caller(transport),
                )],
            );
            add_audio_file_diagnostics(
                &mut files,
                &mut markdown,
                rate,
                "amr callee",
                &cfg.output_dir.join(amr_callee_wav(transport)),
                &[(
                    "callee received caller AMR tone",
                    WindowSelector::Stable,
                    tone_for_caller(transport),
                    tone_for_callee(transport),
                )],
            );
        }
        Scenario::B2buaCall => {
            let rate = amr_sample_rate(cfg.codec_profile);
            add_audio_file_diagnostics(
                &mut files,
                &mut markdown,
                rate,
                "b2bua caller",
                &cfg.output_dir.join(b2bua_caller_wav(transport)),
                &[(
                    "caller received target tone through the bridge",
                    WindowSelector::Stable,
                    tone_for_b2bua_far(transport),
                    tone_for_caller(transport),
                )],
            );
            add_audio_file_diagnostics(
                &mut files,
                &mut markdown,
                rate,
                "b2bua target",
                &cfg.output_dir.join(b2bua_target_wav(transport)),
                &[(
                    "target received caller tone through the bridge",
                    WindowSelector::Stable,
                    tone_for_caller(transport),
                    tone_for_b2bua_far(transport),
                )],
            );
        }
        _ => {}
    }

    let report_sample_rate = match scenario {
        Scenario::AmrCall | Scenario::B2buaCall => amr_sample_rate(cfg.codec_profile),
        _ => SAMPLE_RATE,
    };
    let report = serde_json::json!({
        "scenario": format!("{:?}", scenario),
        "transport": format!("{:?}", transport),
        "sample_rate": report_sample_rate,
        "frame_size": frame_samples(report_sample_rate),
        "files": files,
    });
    std::fs::write(
        cfg.output_dir.join("audio-analysis.json"),
        serde_json::to_string_pretty(&report)?,
    )?;
    std::fs::write(cfg.output_dir.join("audio-analysis.md"), markdown)?;
    Ok(())
}

#[derive(Clone, Copy)]
enum WindowSelector {
    Stable,
    LeadingThird,
    TrailingThird,
}

fn add_audio_file_diagnostics(
    files: &mut Vec<serde_json::Value>,
    markdown: &mut String,
    sample_rate: u32,
    label: &str,
    path: &Path,
    windows: &[(&str, WindowSelector, f32, f32)],
) {
    markdown.push_str(&format!("## {}\n\n", label));
    match read_wav(path) {
        Ok(samples) => {
            let bounds = non_silence_bounds(&samples, 256);
            markdown.push_str(&format!("- path: `{}`\n", path.display()));
            markdown.push_str(&format!("- samples: {}\n", samples.len()));
            markdown.push_str(&format!(
                "- duration_secs: {:.3}\n",
                samples.len() as f64 / f64::from(sample_rate)
            ));
            if let Some((first, last)) = bounds {
                markdown.push_str(&format!("- first_non_silence_sample: {}\n", first));
                markdown.push_str(&format!("- last_non_silence_sample: {}\n", last));
            } else {
                markdown.push_str("- first_non_silence_sample: null\n");
                markdown.push_str("- last_non_silence_sample: null\n");
            }
            let mut window_values = Vec::new();
            for (window_label, selector, expected_hz, rejected_hz) in windows {
                let selected = match selector {
                    WindowSelector::Stable => {
                        analysis_slice_for_window(&samples, sample_rate as usize)
                    }
                    WindowSelector::LeadingThird => {
                        if samples.len() >= 3 {
                            leading_third(&samples)
                        } else {
                            &samples
                        }
                    }
                    WindowSelector::TrailingThird => {
                        if samples.len() >= 3 {
                            trailing_third(&samples)
                        } else {
                            &samples
                        }
                    }
                };
                let window_value = match best_window_tone_for_diag(
                    selected,
                    sample_rate,
                    sample_rate as usize,
                    frame_samples(sample_rate),
                    *expected_hz,
                    *rejected_hz,
                ) {
                    Ok(scan) => {
                        let analysis = &scan.best;
                        markdown.push_str(&format!(
                            "- {}: samples={} analysis_window_samples={} passing_windows={}/{} longest_passing_run={} required_passing_run={} expected_hz={:.0} rejected_hz={:.0} ratio={:.2}\n",
                            window_label,
                            analysis.samples,
                            scan.analysis_window_samples,
                            scan.passing_windows,
                            scan.total_windows,
                            scan.longest_passing_run,
                            scan.required_passing_run,
                            analysis.expected_hz,
                            analysis.rejected_hz,
                            analysis.ratio
                        ));
                        serde_json::json!({
                            "label": window_label,
                            "status": "ok",
                            "samples": analysis.samples,
                            "analysis_window_samples": scan.analysis_window_samples,
                            "step_samples": scan.step_samples,
                            "passing_windows": scan.passing_windows,
                            "total_windows": scan.total_windows,
                            "longest_passing_run": scan.longest_passing_run,
                            "required_passing_run": scan.required_passing_run,
                            "expected_hz": analysis.expected_hz,
                            "rejected_hz": analysis.rejected_hz,
                            "expected_magnitude": analysis.expected_magnitude,
                            "rejected_magnitude": analysis.rejected_magnitude,
                            "ratio": analysis.ratio,
                            "best_window_snr_db": scan.best_quality.snr_db,
                            "best_window_fundamental_amplitude": scan.best_quality.fundamental_amplitude,
                            "weakest_window_snr_db": scan.weakest_snr_db,
                            "weakest_frame_rms": scan.weakest_frame_rms
                        })
                    }
                    Err(error) => {
                        markdown.push_str(&format!("- {}: error: {}\n", window_label, error));
                        serde_json::json!({
                            "label": window_label,
                            "status": "error",
                            "error": error
                        })
                    }
                };
                window_values.push(window_value);
            }
            markdown.push('\n');
            files.push(serde_json::json!({
                "label": label,
                "path": path.display().to_string(),
                "status": "ok",
                "samples": samples.len(),
                "duration_secs": samples.len() as f64 / f64::from(sample_rate),
                "first_non_silence_sample": bounds.map(|(first, _)| first),
                "last_non_silence_sample": bounds.map(|(_, last)| last),
                "windows": window_values
            }));
        }
        Err(error) => {
            markdown.push_str(&format!("- path: `{}`\n", path.display()));
            markdown.push_str(&format!("- error: {}\n\n", error));
            files.push(serde_json::json!({
                "label": label,
                "path": path.display().to_string(),
                "status": "error",
                "error": error.to_string()
            }));
        }
    }
}

fn non_silence_bounds(samples: &[i16], threshold: i16) -> Option<(usize, usize)> {
    let first = samples
        .iter()
        .position(|sample| sample.saturating_abs() >= threshold)?;
    let last = samples
        .iter()
        .rposition(|sample| sample.saturating_abs() >= threshold)?;
    Some((first, last))
}

fn scan_tone_windows(
    samples: &[i16],
    sample_rate: u32,
    window_samples: usize,
    step_samples: usize,
    expected_hz: f32,
    rejected_hz: f32,
    gate: WindowGate,
) -> Result<ToneWindowScan, String> {
    if samples.len() < window_samples {
        return Err(format!(
            "{} samples available (expected at least {})",
            samples.len(),
            window_samples
        ));
    }

    let analysis_window = samples.len().min(
        tone_analysis_window_samples(sample_rate)
            .min(window_samples)
            .max(1),
    );
    let step = step_samples.max(1);
    let last_start = samples.len() - analysis_window;
    let mut start = 0usize;
    let mut best: Option<(ToneAnalysis, ToneQuality)> = None;
    let mut passing_windows = 0usize;
    let mut total_windows = 0usize;
    let mut current_passing_run = 0usize;
    let mut longest_passing_run = 0usize;
    let mut weakest_snr_db = f32::INFINITY;
    let mut weakest_frame_rms = f32::INFINITY;
    loop {
        let window = &samples[start..start + analysis_window];
        let analysis = analyze_tapered_samples(window, sample_rate, expected_hz, rejected_hz);
        // Quality is measured for every window regardless of whether the gate
        // enforces it, so the diagnostics report the same numbers a stricter
        // gate would judge — that is how a future threshold gets chosen from
        // evidence instead of re-running two PBXes.
        let quality = tone_quality(window, sample_rate, expected_hz);
        weakest_snr_db = weakest_snr_db.min(quality.snr_db);
        weakest_frame_rms = weakest_frame_rms.min(quality.min_frame_rms);
        let passes = gate.admits(&analysis, &quality);
        let is_best = best
            .as_ref()
            .map(|(current, _)| analysis.ratio > current.ratio)
            .unwrap_or(true);
        if is_best {
            best = Some((analysis, quality));
        }
        total_windows += 1;
        if passes {
            passing_windows += 1;
            current_passing_run += 1;
            longest_passing_run = longest_passing_run.max(current_passing_run);
        } else {
            current_passing_run = 0;
        }
        if start == last_start {
            break;
        }
        start = (start + step).min(last_start);
    }
    let remaining = window_samples.saturating_sub(analysis_window);
    let additional_windows = if remaining == 0 {
        0
    } else {
        remaining.div_ceil(step)
    };
    let required_passing_run = (additional_windows + 1).min(total_windows).max(1);
    let (best, best_quality) = best.ok_or_else(|| "no analysis window available".to_string())?;
    Ok(ToneWindowScan {
        best,
        total_windows,
        passing_windows,
        longest_passing_run,
        required_passing_run,
        analysis_window_samples: analysis_window,
        step_samples: step,
        best_quality,
        weakest_snr_db,
        weakest_frame_rms,
    })
}

fn best_window_tone_for_diag(
    samples: &[i16],
    sample_rate: u32,
    window_samples: usize,
    step_samples: usize,
    expected_hz: f32,
    rejected_hz: f32,
) -> Result<ToneWindowScan, String> {
    // Diagnostics gate on tone dominance alone so the pass/fail bookkeeping
    // stays comparable across scenarios, but the scan now measures quality
    // for every window regardless — the JSON below is where a future
    // threshold gets chosen from evidence.
    scan_tone_windows(
        samples,
        sample_rate,
        window_samples,
        step_samples,
        expected_hz,
        rejected_hz,
        WindowGate::tone_only(),
    )
}

pub fn generate_tone(freq: f32, frame_num: usize) -> Vec<i16> {
    generate_tone_with_frame_size(freq, frame_num, FRAME_SIZE)
}

pub fn generate_tone_with_frame_size(freq: f32, frame_num: usize, frame_size: usize) -> Vec<i16> {
    generate_tone_at_rate(freq, frame_num, frame_size, SAMPLE_RATE)
}

/// The same tone at an explicit sample rate.
///
/// Everything else in this harness is 8 kHz, so the rate was a constant.
/// AMR-WB is 16 kHz, and a frame carrying the wrong rate is refused by the
/// codec runtime rather than resampled — correctly, since a mis-declared rate
/// would otherwise play back at the wrong pitch.
pub fn generate_tone_at_rate(
    freq: f32,
    frame_num: usize,
    frame_size: usize,
    sample_rate: u32,
) -> Vec<i16> {
    (0..frame_size)
        .map(|j| {
            let t = (frame_num * frame_size + j) as f32 / sample_rate as f32;
            (0.3 * (2.0 * std::f32::consts::PI * freq * t).sin() * 32767.0) as i16
        })
        .collect()
}

pub async fn send_tone_segment(
    sender: &AudioSender,
    tone_hz: f32,
    frames: usize,
    frame_index: &mut usize,
) -> ExampleResult<()> {
    for _ in 0..frames {
        let frame = AudioFrame::new(
            generate_tone(tone_hz, *frame_index),
            SAMPLE_RATE,
            1,
            (*frame_index * FRAME_SIZE) as u32,
        );
        sender.send(frame).await?;
        *frame_index += 1;
        sleep(Duration::from_millis(20)).await;
    }
    Ok(())
}

pub async fn start_tone_recorder(
    handle: &SessionHandle,
    tone_hz: f32,
) -> ExampleResult<ToneRecorder> {
    start_tone_recorder_with_frame_size(handle, tone_hz, FRAME_SIZE).await
}

pub async fn start_tone_recorder_with_frame_size(
    handle: &SessionHandle,
    tone_hz: f32,
    frame_size: usize,
) -> ExampleResult<ToneRecorder> {
    start_tone_recorder_at_rate(handle, tone_hz, frame_size, SAMPLE_RATE).await
}

/// [`start_tone_recorder_with_frame_size`] at an explicit sample rate.
pub async fn start_tone_recorder_at_rate(
    handle: &SessionHandle,
    tone_hz: f32,
    frame_size: usize,
    sample_rate: u32,
) -> ExampleResult<ToneRecorder> {
    let audio = handle.audio().await?;
    let (sender, mut receiver) = audio.split();
    let received_buf = Arc::new(Mutex::new(Vec::<i16>::new()));
    let recv_buf = received_buf.clone();
    let counters = Arc::new(RecorderCounters::new());
    let recv_counters = counters.clone();
    let recorder_started = Instant::now();
    let diag_output_dir = diag_output_dir_from_env();
    let recv_diag_output_dir = diag_output_dir.clone();
    let diag_name = format!("tone_{:.0}hz", tone_hz);
    let recv_diag_name = diag_name.clone();
    if let Some(output_dir) = diag_output_dir.as_deref() {
        diag_event(
            output_dir,
            "recorder_start",
            serde_json::json!({
                "call_id": diag_call_id(handle),
                "recorder": diag_name,
                "tone_hz": tone_hz
            }),
        );
    }
    let recv_task = tokio::spawn(async move {
        while let Some(frame) = receiver.recv().await {
            let elapsed_ms = recorder_started
                .elapsed()
                .as_millis()
                .min(u128::from(u64::MAX)) as u64;
            let elapsed_ms = elapsed_ms.saturating_add(1);
            let previous_frames = recv_counters.rx_frames.fetch_add(1, Ordering::Relaxed);
            recv_counters
                .rx_samples
                .fetch_add(frame.samples.len(), Ordering::Relaxed);
            recv_counters
                .last_rx_elapsed_ms
                .store(elapsed_ms, Ordering::Relaxed);
            let _ = recv_counters.first_rx_elapsed_ms.compare_exchange(
                0,
                elapsed_ms,
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
            if let Ok(mut buf) = recv_buf.lock() {
                buf.extend_from_slice(&frame.samples);
            }
            let frame_count = previous_frames + 1;
            if frame_count.is_multiple_of(50) {
                if let Some(output_dir) = recv_diag_output_dir.as_deref() {
                    diag_event(
                        output_dir,
                        "recorder_rx_periodic",
                        serde_json::json!({
                            "recorder": recv_diag_name,
                            "rx_frames": frame_count,
                            "rx_samples": recv_counters.rx_samples.load(Ordering::Relaxed),
                            "last_rx_elapsed_ms": elapsed_ms
                        }),
                    );
                }
            }
        }
    });
    let running = Arc::new(AtomicBool::new(true));
    let send_running = running.clone();
    // Digital silence on demand. DTX only does anything when the encoder is
    // given silence to detect, and the tone source never is — so a DTX cell
    // without this proves only that the call still works.
    let sending_silence = Arc::new(AtomicBool::new(false));
    let send_silence = sending_silence.clone();
    let send_task = tokio::spawn(async move {
        let mut frame_index = 0usize;
        let frame_duration_ms = ((frame_size as u64) * 1000 / u64::from(sample_rate)).max(1);
        // An interval, not a trailing sleep. `sleep(20ms)` after generate+send
        // makes the real period 20 ms *plus* the work and scheduler slop —
        // measured at 21.5 ms/frame through a transparent relay, and 24 ms
        // under load. A PBX re-clocking the stream to a true 20 ms is starved
        // by such a source and has to stretch or conceal, which corrupts every
        // audio measurement downstream of it. The interval's default catch-up
        // (Burst) keeps the long-run rate exact even when one tick runs late.
        let mut ticker = tokio::time::interval(Duration::from_millis(frame_duration_ms));
        while send_running.load(Ordering::Relaxed) && sender.is_open() {
            ticker.tick().await;
            let samples = if send_silence.load(Ordering::Relaxed) {
                vec![0i16; frame_size]
            } else {
                generate_tone_at_rate(tone_hz, frame_index, frame_size, sample_rate)
            };
            let frame = AudioFrame::new(samples, sample_rate, 1, (frame_index * frame_size) as u32);
            if sender.send(frame).await.is_err() {
                break;
            }
            frame_index += 1;
        }
    });
    Ok(ToneRecorder {
        running,
        sending_silence,
        send_task,
        recv_task,
        received_buf,
        counters,
        diag_output_dir,
        diag_name,
        sample_rate,
    })
}

impl ToneRecorder {
    /// Send digital silence for `duration`, then resume the tone.
    ///
    /// Returns the number of samples received during the silent window, which
    /// is what makes the DTX assertion possible: with DTX on the far end
    /// still has to deliver a continuous stream (SID-driven comfort noise
    /// rather than a gap), so a receiver that goes silent is a bug, not the
    /// feature working.
    async fn hold_silence(&self, duration: Duration) -> usize {
        let before = self.counters.rx_samples.load(Ordering::Relaxed);
        self.sending_silence.store(true, Ordering::Relaxed);
        sleep(duration).await;
        self.sending_silence.store(false, Ordering::Relaxed);
        self.counters
            .rx_samples
            .load(Ordering::Relaxed)
            .saturating_sub(before)
    }

    async fn wait_for_received_samples(
        &self,
        minimum_samples: usize,
        timeout_duration: Duration,
    ) -> ExampleResult<()> {
        let wait = async {
            loop {
                let received_samples = self.counters.rx_samples.load(Ordering::Relaxed);
                if received_samples >= minimum_samples {
                    return;
                }
                sleep(Duration::from_millis(10)).await;
            }
        };

        timeout(timeout_duration, wait).await.map_err(|_| {
            format!(
                "timed out after {:?} waiting for {} to receive {} samples (received {})",
                timeout_duration,
                self.diag_name,
                minimum_samples,
                self.counters.rx_samples.load(Ordering::Relaxed)
            )
        })?;
        Ok(())
    }

    pub async fn stop_and_save(
        self,
        output_dir: &Path,
        output_name: &str,
    ) -> ExampleResult<PathBuf> {
        let ToneRecorder {
            running,
            sending_silence: _,
            send_task,
            recv_task,
            received_buf,
            counters,
            diag_output_dir,
            diag_name,
            sample_rate,
        } = self;
        running.store(false, Ordering::Relaxed);
        let _ = timeout(Duration::from_secs(2), send_task).await;
        stop_recv_task(recv_task).await;
        let received = received_buf.lock().map(|g| g.clone()).unwrap_or_default();
        let path = save_wav_at_rate(output_dir, output_name, &received, sample_rate)?;
        if let Some(diag_dir) = diag_output_dir.as_deref() {
            diag_event(
                diag_dir,
                "recorder_stop",
                serde_json::json!({
                    "recorder": diag_name,
                    "output_name": output_name,
                    "output_path": path.display().to_string(),
                    "rx_frames": counters.rx_frames.load(Ordering::Relaxed),
                    "rx_samples": counters.rx_samples.load(Ordering::Relaxed),
                    "first_rx_elapsed_ms": counters.first_rx_elapsed_ms.load(Ordering::Relaxed),
                    "last_rx_elapsed_ms": counters.last_rx_elapsed_ms.load(Ordering::Relaxed)
                }),
            );
        }
        Ok(path)
    }
}

pub async fn call_with_answer_retry(
    peer: &mut StreamPeer,
    target: &str,
    timeout_duration: Duration,
) -> ExampleResult<SessionHandle> {
    let attempts =
        call_retry_attempts(PbxProvider::from_env_or_args().unwrap_or(PbxProvider::Asterisk))
            .max(1);
    let mut last_error: Option<Box<dyn std::error::Error + Send + Sync>> = None;
    for attempt in 1..=attempts {
        let call_id = peer.invite(target).send().await?;
        let handle = peer.coordinator().session(&call_id);
        match handle.wait_for_answered(Some(timeout_duration)).await {
            Ok(answered) => return Ok(answered),
            Err(e) => {
                println!(
                    "[call] Attempt {}/{} to {} was not answered: {}",
                    attempt, attempts, target, e
                );
                last_error = Some(Box::new(e));
            }
        }
        if attempt < attempts {
            sleep(Duration::from_secs(2)).await;
        }
    }
    Err(last_error.unwrap_or_else(|| "call was not answered".into()))
}

pub async fn call_with_ringing_retry(
    peer: &mut StreamPeer,
    target: &str,
    timeout_duration: Duration,
) -> ExampleResult<SessionHandle> {
    let attempts =
        call_retry_attempts(PbxProvider::from_env_or_args().unwrap_or(PbxProvider::Asterisk))
            .max(1);
    let mut last_error: Option<Box<dyn std::error::Error + Send + Sync>> = None;
    for attempt in 1..=attempts {
        let call_id = peer.invite(target).send().await?;
        let handle = peer.coordinator().session(&call_id);
        match handle
            .wait_for_progress(
                |event| {
                    matches!(
                        event,
                        Event::CallProgress {
                            status_code: 180 | 183,
                            ..
                        }
                    )
                },
                Some(timeout_duration),
            )
            .await
        {
            Ok(_) => return Ok(handle),
            Err(e) => {
                println!(
                    "[call] Attempt {}/{} to {} did not ring: {}",
                    attempt, attempts, target, e
                );
                last_error = Some(Box::new(e));
            }
        }
        if attempt < attempts {
            sleep(Duration::from_secs(2)).await;
        }
    }
    Err(last_error.unwrap_or_else(|| "call did not reach ringing".into()))
}

pub async fn callback_call_with_answer_retry(
    runtime: &mut CallbackRuntime,
    target: &str,
    timeout_duration: Duration,
) -> ExampleResult<SessionHandle> {
    let attempts = call_retry_attempts(runtime.cfg.provider).max(1);
    let mut last_error: Option<String> = None;
    for attempt in 1..=attempts {
        let call_id = runtime.control.invite(target).send().await?;
        let handle = runtime.control.coordinator().session(&call_id);
        match wait_for_established(&mut runtime.events, handle.id(), timeout_duration).await {
            Ok(answered) => return Ok(answered),
            Err(e) => {
                println!(
                    "[call] Attempt {}/{} to {} was not answered: {}",
                    attempt, attempts, target, e
                );
                last_error = Some(e.to_string());
            }
        }
        if attempt < attempts {
            sleep(Duration::from_secs(2)).await;
        }
    }
    Err(last_error
        .unwrap_or_else(|| "call was not answered".into())
        .into())
}

pub async fn assert_srtp_media_security(
    handle: &SessionHandle,
    timeout_duration: Duration,
) -> ExampleResult<()> {
    let security = handle
        .wait_for_media_security(Some(timeout_duration))
        .await?;
    if security.keying != MediaSecurityKeying::Sdes {
        return Err(format!("expected SDES keying, got {:?}", security.keying).into());
    }
    if security.profile != MediaSecurityProfile::RtpSavp {
        return Err(format!("expected RTP/SAVP profile, got {:?}", security.profile).into());
    }
    if !security.contexts_installed {
        return Err("SRTP media security exists but contexts_installed=false".into());
    }
    println!(
        "[security] SRTP negotiated: keying=SDES suite={} profile=RTP/SAVP contexts_installed={}",
        security.suite, security.contexts_installed
    );
    Ok(())
}

pub async fn wait_for_stream_registration(
    peer: &StreamPeer,
    handle: &RegistrationHandle,
    username: &str,
) -> ExampleResult<()> {
    for _ in 0..50 {
        if peer.is_registered(handle).await? {
            return Ok(());
        }
        sleep(Duration::from_millis(200)).await;
    }
    Err(format!("endpoint {} did not register within 10s", username).into())
}

pub async fn wait_for_local_hold_on_events(
    events: &mut EventReceiver,
    timeout_duration: Duration,
) -> ExampleResult<()> {
    wait_for_named_event(events, timeout_duration, "CallOnHold", |event| {
        matches!(event, Event::CallOnHold { .. })
    })
    .await
}

pub async fn wait_for_local_resume_on_events(
    events: &mut EventReceiver,
    timeout_duration: Duration,
) -> ExampleResult<()> {
    wait_for_named_event(events, timeout_duration, "CallResumed", |event| {
        matches!(event, Event::CallResumed { .. })
    })
    .await
}

pub async fn wait_for_dtmf_sequence_on_events(
    events: &mut EventReceiver,
    expected: &[char],
    timeout_duration: Duration,
) -> ExampleResult<()> {
    let expected = expected.to_vec();
    timeout(timeout_duration, async {
        let mut index = 0usize;
        while index < expected.len() {
            match events.next().await {
                Some(Event::DtmfReceived { digit, .. }) if digit == expected[index] => {
                    index += 1;
                }
                Some(Event::DtmfReceived { digit, .. }) => {
                    return Err(format!(
                        "DTMF sequence mismatch at index {}: expected '{}', got '{}'",
                        index, expected[index], digit
                    )
                    .into());
                }
                Some(Event::CallEnded { reason, .. }) => {
                    return Err(format!("call ended before DTMF completed: {}", reason).into());
                }
                Some(Event::CallFailed {
                    status_code,
                    reason,
                    ..
                }) => {
                    return Err(format!(
                        "call failed before DTMF completed: {} {}",
                        status_code, reason
                    )
                    .into());
                }
                Some(_) => {}
                None => return Err("event stream closed while waiting for DTMF".into()),
            }
        }
        Ok(())
    })
    .await
    .map_err(|_| format!("timed out after {:?} waiting for DTMF", timeout_duration))?
}

pub async fn wait_for_call_cancelled_on_events(
    events: &mut EventReceiver,
    timeout_duration: Duration,
) -> ExampleResult<()> {
    timeout(timeout_duration, async {
        loop {
            match events.next().await {
                Some(Event::CallCancelled { .. }) => return Ok(()),
                Some(Event::CallEnded { reason, .. }) => {
                    return Err(
                        format!("call ended while waiting for CallCancelled: {}", reason).into(),
                    );
                }
                Some(Event::CallFailed {
                    status_code,
                    reason,
                    ..
                }) => {
                    return Err(format!(
                        "call failed while waiting for CallCancelled: {} {}",
                        status_code, reason
                    )
                    .into());
                }
                Some(_) => {}
                None => return Err("event stream closed while waiting for CallCancelled".into()),
            }
        }
    })
    .await
    .map_err(|_| {
        format!(
            "timed out after {:?} waiting for CallCancelled",
            timeout_duration
        )
    })?
}

pub async fn wait_for_call_failed_on_events(
    events: &mut EventReceiver,
    timeout_duration: Duration,
) -> ExampleResult<(u16, String)> {
    timeout(timeout_duration, async {
        loop {
            match events.next().await {
                Some(Event::CallFailed {
                    status_code,
                    reason,
                    ..
                }) => return Ok((status_code, reason)),
                Some(Event::CallEnded { reason, .. }) => {
                    return Err(
                        format!("call ended while waiting for CallFailed: {}", reason).into(),
                    );
                }
                Some(_) => {}
                None => return Err("event stream closed while waiting for CallFailed".into()),
            }
        }
    })
    .await
    .map_err(|_| {
        format!(
            "timed out after {:?} waiting for CallFailed",
            timeout_duration
        )
    })?
}

async fn wait_for_named_event<F>(
    events: &mut EventReceiver,
    timeout_duration: Duration,
    event_name: &str,
    mut predicate: F,
) -> ExampleResult<()>
where
    F: FnMut(&Event) -> bool,
{
    timeout(timeout_duration, async {
        loop {
            match events.next().await {
                Some(event) if predicate(&event) => return Ok(()),
                Some(Event::CallEnded { reason, .. }) => {
                    return Err(format!(
                        "call ended before {} was observed: {}",
                        event_name, reason
                    )
                    .into());
                }
                Some(Event::CallFailed {
                    status_code,
                    reason,
                    ..
                }) => {
                    return Err(format!(
                        "call failed before {} was observed: {} {}",
                        event_name, status_code, reason
                    )
                    .into());
                }
                Some(_) => {}
                None => {
                    return Err(
                        format!("event stream closed while waiting for {}", event_name).into(),
                    );
                }
            }
        }
    })
    .await
    .map_err(|_| {
        format!(
            "timed out after {:?} waiting for {}",
            timeout_duration, event_name
        )
    })?
}

pub async fn wait_for_established(
    events: &mut mpsc::UnboundedReceiver<CallbackEvent>,
    call_id: &CallId,
    timeout_duration: Duration,
) -> ExampleResult<SessionHandle> {
    timeout(timeout_duration, async {
        loop {
            match events.recv().await {
                Some(CallbackEvent::Established(handle)) if handle.id() == call_id => {
                    return Ok(handle);
                }
                Some(CallbackEvent::Failed {
                    call_id: failed_id,
                    status_code,
                    reason,
                }) if &failed_id == call_id => {
                    return Err(format!("call failed with {} {}", status_code, reason).into());
                }
                Some(CallbackEvent::Cancelled {
                    call_id: cancelled_id,
                }) if &cancelled_id == call_id => {
                    return Err("call cancelled before answer".into());
                }
                Some(_) => {}
                None => return Err("callback event channel closed".into()),
            }
        }
    })
    .await
    .map_err(|_| format!("timed out after {:?} waiting for answer", timeout_duration))?
}

pub async fn wait_for_next_established(
    events: &mut mpsc::UnboundedReceiver<CallbackEvent>,
    timeout_duration: Duration,
) -> ExampleResult<SessionHandle> {
    timeout(timeout_duration, async {
        loop {
            match events.recv().await {
                Some(CallbackEvent::Established(handle)) => return Ok(handle),
                Some(CallbackEvent::Failed {
                    status_code,
                    reason,
                    ..
                }) => {
                    return Err(format!("call failed with {} {}", status_code, reason).into());
                }
                Some(_) => {}
                None => return Err("callback event channel closed".into()),
            }
        }
    })
    .await
    .map_err(|_| {
        format!(
            "timed out after {:?} waiting for established call",
            timeout_duration
        )
    })?
}

pub async fn wait_for_call_failed(
    events: &mut mpsc::UnboundedReceiver<CallbackEvent>,
    call_id: &CallId,
    expected_status: u16,
    timeout_duration: Duration,
) -> ExampleResult<()> {
    timeout(timeout_duration, async {
        loop {
            match events.recv().await {
                Some(CallbackEvent::Failed {
                    call_id: failed_id,
                    status_code,
                    reason,
                }) if &failed_id == call_id => {
                    if status_code == expected_status {
                        return Ok(());
                    }
                    return Err(format!(
                        "expected failure {}, got {} {}",
                        expected_status, status_code, reason
                    )
                    .into());
                }
                Some(_) => {}
                None => return Err("callback event channel closed".into()),
            }
        }
    })
    .await
    .map_err(|_| {
        format!(
            "timed out after {:?} waiting for CallFailed",
            timeout_duration
        )
    })?
}

pub async fn wait_for_cancelled(
    events: &mut mpsc::UnboundedReceiver<CallbackEvent>,
    call_id: Option<&CallId>,
    timeout_duration: Duration,
) -> ExampleResult<()> {
    timeout(timeout_duration, async {
        loop {
            match events.recv().await {
                Some(CallbackEvent::Cancelled {
                    call_id: cancelled_id,
                }) if call_id.is_none_or(|expected| expected == &cancelled_id) => return Ok(()),
                Some(CallbackEvent::Failed {
                    call_id: failed_id,
                    status_code,
                    reason,
                }) if call_id.is_none_or(|expected| expected == &failed_id) => {
                    return Err(format!(
                        "call failed while waiting for cancellation: {} {}",
                        status_code, reason
                    )
                    .into());
                }
                Some(_) => {}
                None => return Err("callback event channel closed".into()),
            }
        }
    })
    .await
    .map_err(|_| {
        format!(
            "timed out after {:?} waiting for CallCancelled",
            timeout_duration
        )
    })?
}

pub async fn wait_for_callback_progress(
    events: &mut mpsc::UnboundedReceiver<CallbackEvent>,
    call_id: &CallId,
    timeout_duration: Duration,
) -> ExampleResult<()> {
    timeout(timeout_duration, async {
        loop {
            match events.recv().await {
                Some(CallbackEvent::Progress {
                    call_id: progress_id,
                    status_code: 180 | 183,
                    ..
                }) if &progress_id == call_id => return Ok(()),
                Some(CallbackEvent::Failed {
                    call_id: failed_id,
                    status_code,
                    reason,
                }) if &failed_id == call_id => {
                    return Err(format!("call failed with {} {}", status_code, reason).into());
                }
                Some(_) => {}
                None => return Err("callback event channel closed".into()),
            }
        }
    })
    .await
    .map_err(|_| {
        format!(
            "timed out after {:?} waiting for callback call progress",
            timeout_duration
        )
    })?
}

pub async fn wait_for_dtmf_sequence(
    events: &mut mpsc::UnboundedReceiver<CallbackEvent>,
    expected: &[char],
    timeout_duration: Duration,
) -> ExampleResult<()> {
    let expected = expected.to_vec();
    timeout(timeout_duration, async {
        let mut index = 0usize;
        while index < expected.len() {
            match events.recv().await {
                Some(CallbackEvent::Dtmf { digit, .. }) if digit == expected[index] => index += 1,
                Some(CallbackEvent::Dtmf { digit, .. }) => {
                    return Err(format!(
                        "DTMF sequence mismatch at index {}: expected '{}', got '{}'",
                        index, expected[index], digit
                    )
                    .into());
                }
                Some(_) => {}
                None => return Err("callback event channel closed".into()),
            }
        }
        Ok(())
    })
    .await
    .map_err(|_| format!("timed out after {:?} waiting for DTMF", timeout_duration))?
}

pub async fn wait_for_registration_success(
    events: &mut mpsc::UnboundedReceiver<CallbackEvent>,
    timeout_duration: Duration,
) -> ExampleResult<()> {
    timeout(timeout_duration, async {
        loop {
            match events.recv().await {
                Some(CallbackEvent::RegistrationSuccess { registrar, .. }) => {
                    println!("[callback-registration] registered with {}", registrar);
                    return Ok(());
                }
                Some(_) => {}
                None => return Err("callback event channel closed".into()),
            }
        }
    })
    .await
    .map_err(|_| {
        format!(
            "timed out after {:?} waiting for registration",
            timeout_duration
        )
    })?
}

pub async fn wait_for_unregistration_success(
    events: &mut mpsc::UnboundedReceiver<CallbackEvent>,
    timeout_duration: Duration,
) -> ExampleResult<()> {
    timeout(timeout_duration, async {
        loop {
            match events.recv().await {
                Some(CallbackEvent::UnregistrationSuccess { registrar }) => {
                    println!("[callback-registration] unregistered from {}", registrar);
                    return Ok(());
                }
                Some(_) => {}
                None => return Err("callback event channel closed".into()),
            }
        }
    })
    .await
    .map_err(|_| {
        format!(
            "timed out after {:?} waiting for unregistration",
            timeout_duration
        )
    })?
}

pub async fn wait_for_local_hold_resume(
    events: &mut mpsc::UnboundedReceiver<CallbackEvent>,
    timeout_duration: Duration,
) -> ExampleResult<()> {
    timeout(timeout_duration, async {
        let mut saw_hold = false;
        loop {
            match events.recv().await {
                Some(CallbackEvent::LocalHold { .. }) => saw_hold = true,
                Some(CallbackEvent::LocalResume { .. }) if saw_hold => return Ok(()),
                Some(_) => {}
                None => return Err("callback event channel closed".into()),
            }
        }
    })
    .await
    .map_err(|_| {
        format!(
            "timed out after {:?} waiting for hold/resume",
            timeout_duration
        )
    })?
}

async fn wait_for_incoming_notice(
    events: &mut mpsc::UnboundedReceiver<CallbackEvent>,
    timeout_duration: Duration,
) -> ExampleResult<CallId> {
    timeout(timeout_duration, async {
        loop {
            match events.recv().await {
                Some(CallbackEvent::Incoming { call_id, from, to }) => {
                    println!("[callback] incoming call {} -> {}", from, to);
                    return Ok(call_id);
                }
                Some(_) => {}
                None => return Err("callback event channel closed".into()),
            }
        }
    })
    .await
    .map_err(|_| {
        format!(
            "timed out after {:?} waiting for incoming call",
            timeout_duration
        )
    })?
}

pub fn save_wav(out_dir: &Path, name: &str, samples: &[i16]) -> ExampleResult<PathBuf> {
    save_wav_at_rate(out_dir, name, samples, SAMPLE_RATE)
}

/// Write a recording whose header states the rate the samples were actually
/// captured at.
///
/// The narrowband default is wrong for AMR-WB, and wrong in the way that is
/// hardest to notice: 16 kHz samples in an 8 kHz header still play, still have
/// a plausible RMS, and still pass a length check — they are simply an octave
/// low and twice as long. Every wideband recording this harness wrote before
/// this took the rate from `SAMPLE_RATE`, so their durations were double the
/// truth and their tones half the frequency.
pub fn save_wav_at_rate(
    out_dir: &Path,
    name: &str,
    samples: &[i16],
    sample_rate: u32,
) -> ExampleResult<PathBuf> {
    std::fs::create_dir_all(out_dir)?;
    let path = out_dir.join(name);
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&path, spec)?;
    for &s in samples {
        writer.write_sample(s)?;
    }
    writer.finalize()?;
    println!("Saved {} ({} samples)", path.display(), samples.len());
    Ok(path)
}

pub fn read_wav(path: &Path) -> ExampleResult<Vec<i16>> {
    Ok(read_wav_with_rate(path)?.0)
}

/// Read a recording *and* the rate its header claims, so a caller that knows
/// what rate the leg negotiated can catch a mislabeled capture instead of
/// silently analysing it an octave off.
pub fn read_wav_with_rate(path: &Path) -> ExampleResult<(Vec<i16>, u32)> {
    let mut reader = hound::WavReader::open(path)?;
    let rate = reader.spec().sample_rate;
    let samples = reader.samples::<i16>().collect::<Result<Vec<_>, _>>()?;
    Ok((samples, rate))
}

pub fn analyze_samples(
    samples: &[i16],
    expected_hz: f32,
    rejected_hz: f32,
) -> ExampleResult<ToneAnalysis> {
    let expected_magnitude = goertzel_magnitude(samples, SAMPLE_RATE as f32, expected_hz);
    let rejected_magnitude = goertzel_magnitude(samples, SAMPLE_RATE as f32, rejected_hz);
    let ratio = dominance_ratio(expected_magnitude, rejected_magnitude);
    Ok(ToneAnalysis {
        samples: samples.len(),
        expected_hz,
        rejected_hz,
        expected_magnitude,
        rejected_magnitude,
        ratio,
    })
}

pub fn assert_audio_path(
    path: &Path,
    expected_hz: f32,
    rejected_hz: f32,
) -> ExampleResult<ToneAnalysis> {
    let samples = read_wav(path)?;
    if samples.len() < MIN_RECEIVED_SAMPLES {
        return Err(format!(
            "{} too short: {} samples (expected at least {})",
            path.display(),
            samples.len(),
            MIN_RECEIVED_SAMPLES
        )
        .into());
    }
    let label = path.display().to_string();
    assert_best_window_tone(
        &label,
        &samples,
        SAMPLE_RATE,
        SAMPLE_RATE as usize,
        FRAME_SIZE,
        expected_hz,
        rejected_hz,
    )
}

/// Assert a recording holds a *clean* copy of the far end's tone.
///
/// Three independent things must hold, because each of the ways the old
/// dominance-only check actually failed defeats a different pair of them:
///
/// - the far end's tone dominates the one we sent (unchanged — rules out
///   loopback and crossed legs);
/// - fundamental power beats everything else by [`AMR_MIN_TONE_SNR_DB`],
///   which is what a decoder producing noise at the right pitch fails
///   (1-bit squaring, per-frame time reversal: full level, right bin,
///   single-digit SNR);
/// - no 20 ms frame falls below [`AMR_MIN_FRAME_RMS`], which is what
///   attenuation and dropouts fail while the spectrum stays perfect;
///
/// and all three must hold *continuously* for [`AMR_REQUIRED_TONE_SECS`] —
/// a degraded capture's best window can beat a clean capture's worst, so no
/// whole-capture figure discriminates. Everything runs at the capture's own
/// rate; a 16 kHz recording read at 8 kHz is a clean tone an octave low,
/// which nothing else in the harness notices.
fn assert_amr_tone_quality(
    path: &Path,
    sample_rate: u32,
    expected_hz: f32,
    rejected_hz: f32,
) -> ExampleResult<()> {
    let (samples, header_rate) = read_wav_with_rate(path)?;
    if header_rate != sample_rate {
        return Err(format!(
            "{}: WAV header says {} Hz but this leg negotiated {} Hz — the recorder \
             and the assertion disagree about what was captured",
            path.display(),
            header_rate,
            sample_rate
        )
        .into());
    }
    let label = path.display().to_string();
    assert_amr_tone_quality_samples(&label, &samples, sample_rate, expected_hz, rejected_hz)
}

/// The samples-level half of [`assert_amr_tone_quality`], separated so tests
/// need no files (the interop WAVs are gitignored and can never be fixtures)
/// and so the wrong-rate test proves the *analysis* catches a mislabeled
/// capture, not just the header check.
fn assert_amr_tone_quality_samples(
    label: &str,
    samples: &[i16],
    sample_rate: u32,
    expected_hz: f32,
    rejected_hz: f32,
) -> ExampleResult<()> {
    let window_samples = (sample_rate as f32 * AMR_REQUIRED_TONE_SECS) as usize;
    let analysis = assert_best_window_tone_gated(
        label,
        samples,
        sample_rate,
        window_samples,
        frame_samples(sample_rate),
        expected_hz,
        rejected_hz,
        WindowGate::amr(),
    )?;
    println!(
        "{}: {:.0}Hz dominant over {:.0}Hz by {:.1}x at {} Hz, {}s of windows above {:.0} dB SNR and frame RMS {:.0}",
        label,
        expected_hz,
        rejected_hz,
        analysis.ratio,
        sample_rate,
        AMR_REQUIRED_TONE_SECS,
        AMR_MIN_TONE_SNR_DB,
        AMR_MIN_FRAME_RMS,
    );
    Ok(())
}

pub fn assert_samples_tone(
    label: &str,
    samples: &[i16],
    expected_hz: f32,
    rejected_hz: f32,
) -> ExampleResult<ToneAnalysis> {
    let analysis = analyze_samples(samples, expected_hz, rejected_hz)?;
    if analysis.ratio < DOMINANCE_RATIO {
        return Err(format!(
            "{}: {:.0}Hz magnitude {:.1} vs {:.0}Hz magnitude {:.1}, ratio {:.2} (expected at least {:.2})",
            label,
            analysis.expected_hz,
            analysis.expected_magnitude,
            analysis.rejected_hz,
            analysis.rejected_magnitude,
            analysis.ratio,
            DOMINANCE_RATIO
        )
        .into());
    }
    Ok(analysis)
}

pub fn assert_best_window_tone(
    label: &str,
    samples: &[i16],
    sample_rate: u32,
    window_samples: usize,
    step_samples: usize,
    expected_hz: f32,
    rejected_hz: f32,
) -> ExampleResult<ToneAnalysis> {
    assert_best_window_tone_gated(
        label,
        samples,
        sample_rate,
        window_samples,
        step_samples,
        expected_hz,
        rejected_hz,
        WindowGate::tone_only(),
    )
}

#[allow(clippy::too_many_arguments)]
fn assert_best_window_tone_gated(
    label: &str,
    samples: &[i16],
    sample_rate: u32,
    window_samples: usize,
    step_samples: usize,
    expected_hz: f32,
    rejected_hz: f32,
    gate: WindowGate,
) -> ExampleResult<ToneAnalysis> {
    let scan = scan_tone_windows(
        samples,
        sample_rate,
        window_samples,
        step_samples,
        expected_hz,
        rejected_hz,
        gate,
    )
    .map_err(|error| format!("{}: {}", label, error))?;
    let analysis = scan.best;
    if scan.longest_passing_run < scan.required_passing_run {
        // Name the clause that actually bit: the weakest window's figures are
        // the diagnosis, the best window's ratio is only the headline.
        let mut clauses = format!("ratio threshold {:.2}", gate.min_ratio);
        if let Some(floor) = gate.min_snr_db {
            clauses.push_str(&format!(
                ", weakest window SNR {:.1} dB vs floor {:.1} dB",
                scan.weakest_snr_db, floor
            ));
        }
        if let Some(floor) = gate.min_frame_rms {
            clauses.push_str(&format!(
                ", weakest 20ms frame RMS {:.0} vs floor {:.0}",
                scan.weakest_frame_rms, floor
            ));
        }
        return Err(format!(
            "{}: {}/{} sampled windows matched, longest passing run {}/{}; best {:.0}Hz magnitude {:.1} vs {:.0}Hz magnitude {:.1}, ratio {:.2}, best-window SNR {:.1} dB (analysis window {} samples, step {} samples, {})",
            label,
            scan.passing_windows,
            scan.total_windows,
            scan.longest_passing_run,
            scan.required_passing_run,
            analysis.expected_hz,
            analysis.expected_magnitude,
            analysis.rejected_hz,
            analysis.rejected_magnitude,
            analysis.ratio,
            scan.best_quality.snr_db,
            scan.analysis_window_samples,
            scan.step_samples,
            clauses
        )
        .into());
    }
    Ok(analysis)
}

pub fn print_analysis(label: &str, path: &Path, analysis: &ToneAnalysis) {
    println!(
        "{}: {} samples, {:.0}Hz magnitude {:.1}, {:.0}Hz magnitude {:.1}, ratio {:.2}",
        label,
        analysis.samples,
        analysis.expected_hz,
        analysis.expected_magnitude,
        analysis.rejected_hz,
        analysis.rejected_magnitude,
        analysis.ratio
    );
    println!("{} WAV: {}", label, path.display());
}

pub fn goertzel_magnitude(samples: &[i16], sample_rate: f32, target_hz: f32) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let n = samples.len() as f32;
    let k = (0.5 + (n * target_hz) / sample_rate).floor();
    let omega = (2.0 * std::f32::consts::PI * k) / n;
    let coeff = 2.0 * omega.cos();
    let (mut q1, mut q2) = (0.0f32, 0.0f32);
    for &s in samples {
        let q0 = coeff * q1 - q2 + (s as f32);
        q2 = q1;
        q1 = q0;
    }
    (q1 * q1 + q2 * q2 - q1 * q2 * coeff).sqrt()
}

/// The integer Goertzel bin `target_hz` snaps to for a window of `len`.
fn goertzel_bin(len: usize, sample_rate: f32, target_hz: f32) -> usize {
    (0.5 + (len as f32 * target_hz) / sample_rate).floor() as usize
}

/// Hann-tapered Goertzel magnitude at an explicit bin, so a neighbouring bin
/// can be addressed directly. This is the DFT magnitude of the Hann-weighted
/// window at bin `k` — the quantity [`tone_quality`]'s power arithmetic is
/// calibrated against.
fn goertzel_magnitude_hann_bin(samples: &[i16], bin: usize) -> f32 {
    let n = samples.len() as f32;
    let omega = (2.0 * std::f32::consts::PI * bin as f32) / n;
    let coeff = 2.0 * omega.cos();
    let hann_denominator = (samples.len() - 1) as f32;
    let (mut q1, mut q2) = (0.0f32, 0.0f32);
    for (index, &sample) in samples.iter().enumerate() {
        let phase = (2.0 * std::f32::consts::PI * index as f32) / hann_denominator;
        let weight = 0.5 - 0.5 * phase.cos();
        let q0 = coeff * q1 - q2 + f32::from(sample) * weight;
        q2 = q1;
        q1 = q0;
    }
    (q1 * q1 + q2 * q2 - q1 * q2 * coeff).max(0.0).sqrt()
}

fn goertzel_magnitude_hann(samples: &[i16], sample_rate: f32, target_hz: f32) -> f32 {
    if samples.len() < 3 {
        return goertzel_magnitude(samples, sample_rate, target_hz);
    }
    goertzel_magnitude_hann_bin(samples, goertzel_bin(samples.len(), sample_rate, target_hz))
}

/// Energy of the Hann-weighted window, `Σ (x·w)²` — the denominator that makes
/// [`ToneQuality::fundamental_fraction`] a true power fraction.
fn hann_windowed_energy(samples: &[i16]) -> f64 {
    if samples.len() < 2 {
        return 0.0;
    }
    let hann_denominator = (samples.len() - 1) as f64;
    samples
        .iter()
        .enumerate()
        .map(|(index, &sample)| {
            let phase = (2.0 * std::f64::consts::PI * index as f64) / hann_denominator;
            let weight = 0.5 - 0.5 * phase.cos();
            let weighted = f64::from(sample) * weight;
            weighted * weighted
        })
        .sum()
}

/// What one analysis window actually contains, beyond which tone dominates.
///
/// A pure tone lets all of this be measured with no reference signal: the
/// fundamental's power against everything else *is* THD+N, inverted.
#[derive(Debug, Clone, Copy)]
pub struct ToneQuality {
    pub samples: usize,
    pub expected_hz: f32,
    /// Amplitude of the fundamental, directly comparable with
    /// [`TONE_PEAK_AMPLITUDE`]. Recovers a known amplitude to within 0.1%.
    pub fundamental_amplitude: f32,
    /// Share of the window's power in the fundamental bin and its two
    /// neighbours (the spread tolerates up to ±one bin of frequency drift —
    /// ±5 Hz at a 200 ms window — without charging the tone as noise).
    pub fundamental_fraction: f32,
    /// `10·log10(fundamental / everything else)`, in true dB: noise injected
    /// at a stated SNR reads back within 1 dB, which is what makes the
    /// threshold a physical quantity rather than a tuned index.
    pub snr_db: f32,
    pub rms: f32,
    /// The weakest 20 ms frame inside the window — the dropout detector.
    /// Attenuation and gating change this while leaving `snr_db` perfect.
    pub min_frame_rms: f32,
    pub dc_offset: f32,
}

pub fn tone_quality(samples: &[i16], sample_rate: u32, expected_hz: f32) -> ToneQuality {
    let n = samples.len();
    let sum: f64 = samples.iter().map(|&s| f64::from(s)).sum();
    let energy: f64 = samples.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    let rms = if n == 0 {
        0.0
    } else {
        (energy / n as f64).sqrt() as f32
    };
    let dc_offset = if n == 0 { 0.0 } else { (sum / n as f64) as f32 };

    let frame = frame_samples(sample_rate).max(1);
    let mut min_frame_rms = f32::INFINITY;
    let mut offset = 0usize;
    while offset + frame <= n {
        let frame_energy: f64 = samples[offset..offset + frame]
            .iter()
            .map(|&s| f64::from(s) * f64::from(s))
            .sum();
        min_frame_rms = min_frame_rms.min((frame_energy / frame as f64).sqrt() as f32);
        offset += frame;
    }
    if !min_frame_rms.is_finite() {
        min_frame_rms = rms;
    }

    let (fundamental_amplitude, fundamental_fraction, snr_db) = if n < 8 {
        (0.0, 0.0, -99.0)
    } else {
        let k = goertzel_bin(n, sample_rate as f32, expected_hz).clamp(2, n / 2 - 2);
        let center = f64::from(goertzel_magnitude_hann_bin(samples, k));
        let below = f64::from(goertzel_magnitude_hann_bin(samples, k - 1));
        let above = f64::from(goertzel_magnitude_hann_bin(samples, k + 1));
        let windowed = hann_windowed_energy(samples);
        // For a Hann window, an exact-bin tone of amplitude A yields
        // |X_k| = A·N/4 and |X_k±1| = A·N/8, and Parseval over the windowed
        // signal gives Σ|X_j|² = N·E_w. Both identities are pinned by
        // `tone_quality_reads_back_true_snr` and the amplitude check inside
        // `amr_quality_accepts_a_clean_capture_at_either_rate`.
        let amplitude = (4.0 * center / n as f64) as f32;
        let tone_power = 2.0 * (below * below + center * center + above * above);
        let fraction = if windowed <= 0.0 {
            0.0
        } else {
            (tone_power / (n as f64 * windowed)).clamp(0.0, 1.0)
        };
        let snr = if fraction <= 1e-9 {
            -99.0
        } else if fraction >= 1.0 - 1e-9 {
            99.0
        } else {
            (10.0 * (fraction / (1.0 - fraction)).log10()) as f32
        };
        (amplitude, fraction as f32, snr.clamp(-99.0, 99.0))
    };

    ToneQuality {
        samples: n,
        expected_hz,
        fundamental_amplitude,
        fundamental_fraction,
        snr_db,
        rms,
        min_frame_rms,
        dc_offset,
    }
}

fn analyze_tapered_samples(
    samples: &[i16],
    sample_rate: u32,
    expected_hz: f32,
    rejected_hz: f32,
) -> ToneAnalysis {
    let expected_magnitude = goertzel_magnitude_hann(samples, sample_rate as f32, expected_hz);
    let rejected_magnitude = goertzel_magnitude_hann(samples, sample_rate as f32, rejected_hz);
    let ratio = dominance_ratio(expected_magnitude, rejected_magnitude);
    ToneAnalysis {
        samples: samples.len(),
        expected_hz,
        rejected_hz,
        expected_magnitude,
        rejected_magnitude,
        ratio,
    }
}

fn dominance_ratio(expected_magnitude: f32, rejected_magnitude: f32) -> f32 {
    expected_magnitude / rejected_magnitude.max(1.0)
}

fn endpoint_defaults(
    provider: PbxProvider,
    username: &str,
    transport: TransportMode,
) -> EndpointDefaults {
    let base = match provider {
        // Asterisk's endpoints stay at 5070-5075 and 5080-5084, pinned by
        // `asterisk_defaults_preserve_existing_lab_ports`. That block is
        // load-bearing -- the local env files and the lab's PJSIP endpoint
        // configuration name these ports -- so anything that lands on it
        // moves, not this. The Kamailio lab did land on it (5072/5073) and
        // was moved to 5090/5091; see `infra/release-runners/pbx/kamailio/up.sh`.
        PbxProvider::Asterisk => 0,
        PbxProvider::FreeSwitch => 10_000,
        // 30k/40k, not 20k: 5070+20_000 = 25070 is the sip-proxy interop
        // suite's peer port, and its 25xxx block must stay clear so both
        // suites can run side by side (pinned by a unit test below).
        PbxProvider::Kamailio => 30_000,
        PbxProvider::OpenSips => 40_000,
    };
    match (transport, username) {
        (TransportMode::TlsSrtp, "1001") => EndpointDefaults {
            local_port: 5070 + base,
            tls_local_port: Some(5071 + base),
            media_port_start: 16000,
            media_port_end: 16100,
        },
        (TransportMode::TlsSrtp, "1002") => EndpointDefaults {
            local_port: 5072 + base,
            tls_local_port: Some(5073 + base),
            media_port_start: 16120,
            media_port_end: 16220,
        },
        (TransportMode::TlsSrtp, "1003") => EndpointDefaults {
            local_port: 5074 + base,
            tls_local_port: Some(5075 + base),
            media_port_start: 16240,
            media_port_end: 16340,
        },
        (TransportMode::Udp, "2001") => EndpointDefaults {
            local_port: 5080 + base,
            tls_local_port: None,
            media_port_start: 17000,
            media_port_end: 17100,
        },
        (TransportMode::Udp, "2002") => EndpointDefaults {
            local_port: 5082 + base,
            tls_local_port: None,
            media_port_start: 17120,
            media_port_end: 17220,
        },
        (TransportMode::Udp, "2003") => EndpointDefaults {
            local_port: 5084 + base,
            tls_local_port: None,
            media_port_start: 17240,
            media_port_end: 17340,
        },
        _ => EndpointDefaults {
            local_port: 5090 + base,
            tls_local_port: Some(5091 + base),
            media_port_start: 18000,
            media_port_end: 18100,
        },
    }
}

fn print_registration_context(cfg: &EndpointConfig) {
    println!("[{}] Provider:   {}", cfg.username, cfg.provider.label());
    println!(
        "[{}] Transport:  {}",
        cfg.username,
        cfg.transport.env_value()
    );
    println!("[{}] AOR:        {}", cfg.username, cfg.aor_uri());
    println!("[{}] Contact:    {}", cfg.username, cfg.contact_uri());
    println!("[{}] Registrar:  {}", cfg.username, cfg.registrar_uri());
    println!("[{}] Media SDP:  {}", cfg.username, cfg.media_advertised_ip);
    println!(
        "[{}] Codec:     {}",
        cfg.username,
        cfg.codec_profile.env_value()
    );
}

async fn settle_after_register(provider: PbxProvider) {
    let secs = std::env::var(match provider {
        PbxProvider::Asterisk => "ASTERISK_POST_REGISTER_SETTLE_SECS",
        PbxProvider::FreeSwitch => "FREESWITCH_POST_REGISTER_SETTLE_SECS",
        PbxProvider::Kamailio => "KAMAILIO_POST_REGISTER_SETTLE_SECS",
        PbxProvider::OpenSips => "OPENSIPS_POST_REGISTER_SETTLE_SECS",
    })
    .or_else(|_| std::env::var("POST_REGISTER_SETTLE_SECS"))
    .ok()
    .and_then(|value| value.parse().ok())
    .unwrap_or(provider.default_settle_secs());
    if secs > 0 {
        sleep(Duration::from_secs(secs)).await;
    }
}

fn idle_duration() -> Duration {
    env_duration_secs("IDLE_SECS", 2)
}

fn remote_test_timeout(provider: PbxProvider) -> ExampleResult<Duration> {
    let key = match provider {
        PbxProvider::Asterisk => "ASTERISK_TEST_TIMEOUT_SECS",
        PbxProvider::FreeSwitch => "FREESWITCH_TEST_TIMEOUT_SECS",
        PbxProvider::Kamailio => "KAMAILIO_TEST_TIMEOUT_SECS",
        PbxProvider::OpenSips => "OPENSIPS_TEST_TIMEOUT_SECS",
    };
    let secs = std::env::var(key)
        .or_else(|_| std::env::var("REMOTE_TEST_TIMEOUT_SECS"))
        .unwrap_or_else(|_| "60".to_string())
        .parse()?;
    Ok(Duration::from_secs(secs))
}

fn transfer_settle_duration(provider: PbxProvider, transport: TransportMode) -> Duration {
    let key = match provider {
        PbxProvider::Asterisk => "ASTERISK_TRANSFER_SETTLE_SECS",
        PbxProvider::FreeSwitch => "FREESWITCH_TRANSFER_SETTLE_SECS",
        PbxProvider::Kamailio => "KAMAILIO_TRANSFER_SETTLE_SECS",
        PbxProvider::OpenSips => "OPENSIPS_TRANSFER_SETTLE_SECS",
    };
    let tls_key = match provider {
        PbxProvider::Asterisk => "ASTERISK_TLS_TRANSFER_SETTLE_SECS",
        PbxProvider::FreeSwitch => "FREESWITCH_TLS_TRANSFER_SETTLE_SECS",
        PbxProvider::Kamailio => "KAMAILIO_TLS_TRANSFER_SETTLE_SECS",
        PbxProvider::OpenSips => "OPENSIPS_TLS_TRANSFER_SETTLE_SECS",
    };
    if transport.is_tls() {
        if let Some(duration) = optional_env_duration_secs(tls_key) {
            return duration;
        }
    }
    let default = match (provider, transport) {
        (PbxProvider::Asterisk, TransportMode::TlsSrtp) => 8,
        _ => 3,
    };
    env_duration_secs(key, default)
}

fn call_retry_attempts(provider: PbxProvider) -> usize {
    let key = match provider {
        PbxProvider::Asterisk => "ASTERISK_CALL_RETRY_ATTEMPTS",
        PbxProvider::FreeSwitch => "FREESWITCH_CALL_RETRY_ATTEMPTS",
        PbxProvider::Kamailio => "KAMAILIO_CALL_RETRY_ATTEMPTS",
        PbxProvider::OpenSips => "OPENSIPS_CALL_RETRY_ATTEMPTS",
    };
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(provider.default_retry_attempts())
}

fn remote_test_digits(provider: PbxProvider) -> Vec<char> {
    let key = match provider {
        PbxProvider::Asterisk => "ASTERISK_TEST_DIGITS",
        PbxProvider::FreeSwitch => "FREESWITCH_TEST_DIGITS",
        PbxProvider::Kamailio => "KAMAILIO_TEST_DIGITS",
        PbxProvider::OpenSips => "OPENSIPS_TEST_DIGITS",
    };
    std::env::var(key)
        .or_else(|_| std::env::var("REMOTE_TEST_DIGITS"))
        .unwrap_or_else(|_| "1234#".to_string())
        .chars()
        .collect()
}

fn target_user_for(transport: TransportMode) -> &'static str {
    if transport.is_tls() {
        "1002"
    } else {
        "2002"
    }
}

fn tone_for_caller(transport: TransportMode) -> f32 {
    if transport.is_tls() {
        ENDPOINT_1001_TONE_HZ
    } else {
        ENDPOINT_2001_TONE_HZ
    }
}

fn tone_for_callee(transport: TransportMode) -> f32 {
    if transport.is_tls() {
        ENDPOINT_1002_TONE_HZ
    } else {
        ENDPOINT_2002_TONE_HZ
    }
}

fn g711_caller_wav(transport: TransportMode) -> &'static str {
    if transport.is_tls() {
        "tls_srtp_g711_1001_received.wav"
    } else {
        "g711_2001_received.wav"
    }
}

fn g711_callee_wav(transport: TransportMode) -> &'static str {
    if transport.is_tls() {
        "tls_srtp_g711_1002_received.wav"
    } else {
        "g711_2002_received.wav"
    }
}

fn amr_caller_wav(transport: TransportMode) -> &'static str {
    if transport.is_tls() {
        "tls_srtp_amr_1001_received.wav"
    } else {
        "amr_2001_received.wav"
    }
}

fn amr_callee_wav(transport: TransportMode) -> &'static str {
    if transport.is_tls() {
        "tls_srtp_amr_1002_received.wav"
    } else {
        "amr_2002_received.wav"
    }
}

/// The recording name for one leg of a transcode call.
///
/// Unlike [`amr_caller_wav`], the name carries the leg's own codec profile:
/// the two legs of a transcode call write different codecs' audio into one
/// directory, and `amr_2002_received.wav` holding PCMU audio would be
/// actively misleading in an evidence bundle.
fn amr_transcode_wav(cfg: &EndpointConfig) -> String {
    format!(
        "amr_transcode_{}_{}_received.wav",
        cfg.username,
        cfg.codec_profile.env_value()
    )
}

/// The far end of a b2bua call — the target (2003/1003) — sends 660 Hz. The
/// caller sends its usual tone; the b2bua in the middle sends nothing, so
/// 880 Hz appearing anywhere in a b2bua recording would itself be a fault.
fn tone_for_b2bua_far(_transport: TransportMode) -> f32 {
    ENDPOINT_1003_TONE_HZ
}

fn b2bua_caller_wav(transport: TransportMode) -> &'static str {
    if transport.is_tls() {
        "tls_srtp_b2bua_1001_received.wav"
    } else {
        "b2bua_2001_received.wav"
    }
}

fn b2bua_target_wav(transport: TransportMode) -> &'static str {
    if transport.is_tls() {
        "tls_srtp_b2bua_1003_received.wav"
    } else {
        "b2bua_2003_received.wav"
    }
}

fn g729_caller_wav(transport: TransportMode) -> &'static str {
    if transport.is_tls() {
        "tls_srtp_g729_1001_received.wav"
    } else {
        "g729_2001_received.wav"
    }
}

fn g729_callee_wav(transport: TransportMode) -> &'static str {
    if transport.is_tls() {
        "tls_srtp_g729_1002_received.wav"
    } else {
        "g729_2002_received.wav"
    }
}

fn hold_resume_caller_wav(transport: TransportMode) -> &'static str {
    if transport.is_tls() {
        "tls_srtp_hold_resume_1001_received.wav"
    } else {
        "hold_resume_2001_received.wav"
    }
}

fn hold_resume_callee_wav(transport: TransportMode) -> &'static str {
    if transport.is_tls() {
        "tls_srtp_hold_resume_1002_received.wav"
    } else {
        "hold_resume_2002_received.wav"
    }
}

fn dtmf_caller_wav(transport: TransportMode) -> &'static str {
    if transport.is_tls() {
        "tls_srtp_dtmf_1001_received.wav"
    } else {
        "dtmf_2001_received.wav"
    }
}

fn dtmf_callee_wav(transport: TransportMode) -> &'static str {
    if transport.is_tls() {
        "tls_srtp_dtmf_1002_received.wav"
    } else {
        "dtmf_2002_received.wav"
    }
}

fn transferor_wav(transport: TransportMode) -> &'static str {
    if transport.is_tls() {
        "tls_srtp_blind_transfer_1001_received.wav"
    } else {
        "blind_transfer_2001_received.wav"
    }
}

fn transferee_wav(transport: TransportMode) -> &'static str {
    if transport.is_tls() {
        "tls_srtp_blind_transfer_1002_received.wav"
    } else {
        "blind_transfer_2002_received.wav"
    }
}

fn transfer_target_wav(transport: TransportMode) -> &'static str {
    if transport.is_tls() {
        "tls_srtp_blind_transfer_1003_received.wav"
    } else {
        "blind_transfer_2003_received.wav"
    }
}

fn leading_third(samples: &[i16]) -> &[i16] {
    &samples[..samples.len() / 3]
}

fn trailing_third(samples: &[i16]) -> &[i16] {
    &samples[(samples.len() * 2) / 3..]
}

fn stable_middle_half(samples: &[i16]) -> &[i16] {
    &samples[samples.len() / 4..(samples.len() * 3) / 4]
}

fn analysis_slice_for_window(samples: &[i16], window_samples: usize) -> &[i16] {
    let stable = stable_middle_half(samples);
    if stable.len() >= window_samples {
        stable
    } else {
        samples
    }
}

async fn stop_recv_task(task: JoinHandle<()>) {
    let _ = timeout(Duration::from_secs(2), async {
        loop {
            if task.is_finished() {
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    task.abort();
}

fn advertised_ip(provider: PbxProvider, local_ip: IpAddr) -> ExampleResult<IpAddr> {
    let value = match provider {
        PbxProvider::Asterisk => std::env::var("ADVERTISED_IP"),
        PbxProvider::FreeSwitch | PbxProvider::Kamailio | PbxProvider::OpenSips => {
            std::env::var("RVOIP_ADVERTISED_IP").or_else(|_| std::env::var("ADVERTISED_IP"))
        }
    };
    match value {
        Ok(value) => Ok(value.parse()?),
        Err(_) if !local_ip.is_unspecified() => Ok(local_ip),
        Err(_) => Err("advertised IP is required when local IP is unspecified".into()),
    }
}

fn media_advertised_ip(provider: PbxProvider, advertised_ip: IpAddr) -> ExampleResult<IpAddr> {
    let value = match provider {
        PbxProvider::Asterisk => std::env::var("MEDIA_ADVERTISED_IP"),
        PbxProvider::FreeSwitch | PbxProvider::Kamailio | PbxProvider::OpenSips => {
            std::env::var("RVOIP_MEDIA_ADVERTISED_IP")
                .or_else(|_| std::env::var("MEDIA_ADVERTISED_IP"))
        }
    };
    match value {
        Ok(value) if !value.trim().is_empty() => Ok(value.parse()?),
        _ => Ok(advertised_ip),
    }
}

fn auth_username_for(prefix: &str, username: &str) -> String {
    let endpoint_auth = non_empty_env(&format!("{}_AUTH_USERNAME", prefix));
    let sip_username = non_empty_env("SIP_USERNAME");
    let sip_auth_username = non_empty_env("SIP_AUTH_USERNAME");
    select_auth_username(
        username,
        endpoint_auth.as_deref(),
        sip_username.as_deref(),
        sip_auth_username.as_deref(),
    )
}

fn select_auth_username(
    username: &str,
    endpoint_auth: Option<&str>,
    sip_username: Option<&str>,
    sip_auth_username: Option<&str>,
) -> String {
    if let Some(value) = endpoint_auth {
        return value.trim().to_string();
    }
    match (sip_username, sip_auth_username) {
        (Some(sip_username), Some(auth_username)) if sip_username.trim() == username => {
            auth_username.trim().to_string()
        }
        (None, Some(auth_username)) => auth_username.trim().to_string(),
        _ => username.to_string(),
    }
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_string(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_u16(key: &str, default: u16) -> ExampleResult<u16> {
    Ok(std::env::var(key)
        .unwrap_or_else(|_| default.to_string())
        .parse()?)
}

fn env_bool(key: &str, default: bool) -> ExampleResult<bool> {
    let value = match std::env::var(key) {
        Ok(value) => value,
        Err(_) => return Ok(default),
    };
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(format!("{} must be a boolean value", key).into()),
    }
}

fn env_duration_secs(key: &str, default: u64) -> Duration {
    Duration::from_secs(
        std::env::var(key)
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(default),
    )
}

fn optional_env_duration_secs(key: &str) -> Option<Duration> {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse().ok())
        .map(Duration::from_secs)
}

fn split_host_port(value: &str) -> ExampleResult<(String, u16)> {
    if let Ok(addr) = value.parse::<SocketAddr>() {
        return Ok((addr.ip().to_string(), addr.port()));
    }
    let (host, port) = value
        .rsplit_once(':')
        .ok_or_else(|| format!("expected host:port PBX address, got '{}'", value))?;
    Ok((host.to_string(), port.parse()?))
}

fn deterministic_sip_instance(username: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in username.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!(
        "urn:uuid:00000000-0000-4000-8000-{:012x}",
        hash & 0xffff_ffff_ffff
    )
}

fn required_path(key: &str) -> ExampleResult<PathBuf> {
    let value =
        std::env::var(key).map_err(|_| format!("{} must be set for SIP_TRANSPORT=TLS", key))?;
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{} must not be empty", key).into());
    }
    Ok(PathBuf::from(value))
}

fn optional_path(key: &str) -> Option<PathBuf> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn default_pbx_port(transport: TransportMode) -> u16 {
    if transport.is_tls() {
        5061
    } else {
        5060
    }
}

fn transport_suffix(transport: TransportMode) -> &'static str {
    match transport {
        TransportMode::TlsSrtp => ";transport=tls",
        TransportMode::Udp => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone_samples(segments: &[(usize, f64, f64)]) -> Vec<i16> {
        let mut sample_offset = 0usize;
        let mut samples = Vec::new();
        for &(sample_count, expected_amplitude, expected_hz) in segments {
            samples.extend((0..sample_count).map(|index| {
                let absolute_index = sample_offset + index;
                let phase = 2.0 * std::f64::consts::PI * expected_hz * absolute_index as f64
                    / f64::from(SAMPLE_RATE);
                (expected_amplitude * phase.sin()).round() as i16
            }));
            sample_offset += sample_count;
        }
        samples
    }

    fn codec_like_near_bin_tone(sample_count: usize) -> Vec<i16> {
        (0..sample_count)
            .map(|index| {
                let time = index as f64 / f64::from(SAMPLE_RATE);
                let nominal = 8_000.0 * (2.0 * std::f64::consts::PI * 875.0 * time).sin();
                let rejected = 100.0 * (2.0 * std::f64::consts::PI * 440.0 * time).sin();
                (nominal + rejected).round() as i16
            })
            .collect()
    }

    #[test]
    fn g729_caller_capture_target_preserves_analyzer_floor_with_frame_margin() {
        assert!(G729_CALLER_CAPTURE_TARGET_SAMPLES > MIN_RECEIVED_SAMPLES);
        assert_eq!(G729_CALLER_CAPTURE_TARGET_SAMPLES % G729_FRAME_SIZE, 0);
        assert!(G729_CALLER_CAPTURE_TARGET_SAMPLES - MIN_RECEIVED_SAMPLES >= G729_FRAME_SIZE * 4);
    }

    /// The floor is a duration. Stated as a bare sample count it silently
    /// meant half as much exercise at 16 kHz, which is exactly how every
    /// wideband AMR run came out at 0.76 s while narrowband got 1.5 s.
    #[test]
    fn received_sample_floor_is_the_same_duration_at_either_rate() {
        assert_eq!(min_received_samples(8_000), 12_000);
        assert_eq!(min_received_samples(16_000), 24_000);
        for rate in [8_000u32, 16_000] {
            assert_eq!(
                min_received_samples(rate) * 1000 / rate as usize,
                MIN_RECEIVED_MS
            );
        }
    }

    /// The 8 kHz value is what every non-AMR scenario keys its thresholds to;
    /// changing the duration must not move it silently.
    #[test]
    fn narrowband_floor_is_unchanged_by_the_duration_restatement() {
        assert_eq!(MIN_RECEIVED_SAMPLES, 12_000);
        assert_eq!(MIN_RECEIVED_SAMPLES, min_received_samples(SAMPLE_RATE));
    }

    #[test]
    fn freeswitch_defaults_use_local_high_ports() {
        let udp = endpoint_defaults(PbxProvider::FreeSwitch, "2001", TransportMode::Udp);
        assert_eq!(udp.local_port, 15080);
        let tls = endpoint_defaults(PbxProvider::FreeSwitch, "1001", TransportMode::TlsSrtp);
        assert_eq!(tls.local_port, 15070);
        assert_eq!(tls.tls_local_port, Some(15071));
    }

    /// The proxy providers' port bases must clear the sip-proxy interop
    /// suite's 25xxx block (peer port 25070, egress 25071+): a 20_000 base
    /// would collide exactly, which is why these are 30k/40k.
    #[test]
    fn proxy_provider_ports_avoid_the_sip_proxy_interop_block() {
        for (provider, expected_udp) in [
            (PbxProvider::Kamailio, 35080),
            (PbxProvider::OpenSips, 45080),
        ] {
            let udp = endpoint_defaults(provider, "2001", TransportMode::Udp);
            assert_eq!(udp.local_port, expected_udp);
            assert!(
                !(25_000..26_000).contains(&udp.local_port),
                "{provider:?} collides with the sip-proxy interop port block"
            );
        }
    }

    /// Lab daemons and our own endpoints must not claim the same host port.
    ///
    /// They did. The Kamailio lab listened on 5072/5073 and OpenSIPS on 5074,
    /// which are the Asterisk suite's endpoint ports for users 1002 and 1003 —
    /// so with a proxy lab up, an Asterisk TLS cell could not bind its callee
    /// and every one failed with a 404 that named nothing about ports.
    /// Whichever started first won, which is why it looked intermittent.
    ///
    /// Two blocks, kept apart on purpose: daemons at 5060-5069, our endpoints
    /// from 5070 up with a per-provider base. This asserts the daemon side
    /// stays below the endpoint block rather than checking today's exact
    /// numbers, so moving a lab within its block does not fail the test but
    /// moving one back into ours does.
    #[test]
    fn lab_daemon_ports_stay_clear_of_the_endpoint_block() {
        // Every endpoint port this suite binds at base 0.
        let endpoint_ports: Vec<u16> = [
            ("1001", TransportMode::TlsSrtp),
            ("1002", TransportMode::TlsSrtp),
            ("1003", TransportMode::TlsSrtp),
            ("2001", TransportMode::Udp),
            ("2002", TransportMode::Udp),
            ("2003", TransportMode::Udp),
        ]
        .into_iter()
        .flat_map(|(user, transport)| {
            let defaults = endpoint_defaults(PbxProvider::Asterisk, user, transport);
            std::iter::once(defaults.local_port).chain(defaults.tls_local_port)
        })
        .collect();

        // The lab daemons' host ports, as `up.sh` defaults them.
        for (lab, port) in [
            ("asterisk", 5060u16),
            ("asterisk-tls", 5061),
            ("freeswitch", 5062),
            ("freeswitch-tls", 5063),
            ("kamailio", 5066),
            ("kamailio-tls", 5067),
            ("opensips", 5068),
        ] {
            assert!(
                !endpoint_ports.contains(&port),
                "the {lab} lab listens on {port}, which this suite also binds \
                 as an endpoint — one of them will fail to start, and which \
                 one depends on start order"
            );
            assert!(
                port < 5070,
                "the {lab} lab's {port} is inside the endpoint block; daemons \
                 belong below 5070"
            );
        }
    }

    #[test]
    fn asterisk_defaults_preserve_existing_lab_ports() {
        let udp = endpoint_defaults(PbxProvider::Asterisk, "2001", TransportMode::Udp);
        assert_eq!(udp.local_port, 5080);
        let tls = endpoint_defaults(PbxProvider::Asterisk, "1001", TransportMode::TlsSrtp);
        assert_eq!(tls.local_port, 5070);
        assert_eq!(tls.tls_local_port, Some(5071));
    }

    #[test]
    fn split_host_port_accepts_ipv4_socket_addr() {
        let (host, port) = split_host_port("127.0.0.1:5062").unwrap();
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, 5062);
    }

    #[test]
    fn canonical_user_sets_are_transport_specific() {
        let roles = [
            Role::Registration,
            Role::Caller,
            Role::Transferor,
            Role::Callee,
            Role::Transferee,
            Role::Target,
        ];
        let mut tls_users = roles
            .iter()
            .map(|role| username_for(TransportMode::TlsSrtp, *role))
            .collect::<Vec<_>>();
        tls_users.sort_unstable();
        tls_users.dedup();
        assert_eq!(tls_users, vec!["1001", "1002", "1003"]);

        let mut udp_users = roles
            .iter()
            .map(|role| username_for(TransportMode::Udp, *role))
            .collect::<Vec<_>>();
        udp_users.sort_unstable();
        udp_users.dedup();
        assert_eq!(udp_users, vec!["2001", "2002", "2003"]);
    }

    #[test]
    fn auth_username_ignores_global_for_other_endpoint_users() {
        assert_eq!(
            select_auth_username("2001", None, Some("1001"), Some("1001")),
            "2001"
        );
        assert_eq!(
            select_auth_username("1001", None, Some("1001"), Some("1001")),
            "1001"
        );
        assert_eq!(
            select_auth_username("2001", Some("auth2001"), Some("1001"), Some("1001")),
            "auth2001"
        );
    }

    #[test]
    fn hann_taper_recovers_near_bin_codec_tone_without_lowering_threshold() {
        let samples = codec_like_near_bin_tone(TONE_ANALYSIS_WINDOW_SAMPLES);
        let rectangular = analyze_samples(&samples, 880.0, 440.0).unwrap();
        let tapered = analyze_tapered_samples(&samples, SAMPLE_RATE, 880.0, 440.0);

        assert!(rectangular.ratio < DOMINANCE_RATIO);
        assert!(tapered.ratio >= DOMINANCE_RATIO);
    }

    #[test]
    fn hann_taper_rejects_true_rejected_tone() {
        let samples = tone_samples(&[(SAMPLE_RATE as usize, 8_000.0, 440.0)]);
        let scan = scan_tone_windows(
            &samples,
            SAMPLE_RATE,
            SAMPLE_RATE as usize,
            FRAME_SIZE,
            880.0,
            440.0,
            WindowGate::tone_only(),
        )
        .unwrap();

        assert_eq!(scan.longest_passing_run, 0);
        assert!(scan.longest_passing_run < scan.required_passing_run);
    }

    #[test]
    fn tone_scanner_rejects_silence() {
        let samples = vec![0; SAMPLE_RATE as usize];
        let scan = scan_tone_windows(
            &samples,
            SAMPLE_RATE,
            SAMPLE_RATE as usize,
            FRAME_SIZE,
            880.0,
            440.0,
            WindowGate::tone_only(),
        )
        .unwrap();

        assert_eq!(scan.passing_windows, 0);
        assert_eq!(scan.longest_passing_run, 0);
    }

    #[test]
    fn hann_taper_accepts_one_continuous_second_of_near_bin_tone() {
        let samples = codec_like_near_bin_tone(SAMPLE_RATE as usize);
        let scan = scan_tone_windows(
            &samples,
            SAMPLE_RATE,
            SAMPLE_RATE as usize,
            FRAME_SIZE,
            880.0,
            440.0,
            WindowGate::tone_only(),
        )
        .unwrap();

        assert!(scan.longest_passing_run >= scan.required_passing_run);
    }

    #[test]
    fn hann_taper_does_not_join_short_tone_islands() {
        let samples = tone_samples(&[
            (3_200, 8_000.0, 875.0),
            (1_600, 8_000.0, 440.0),
            (3_200, 8_000.0, 875.0),
        ]);
        let scan = scan_tone_windows(
            &samples,
            SAMPLE_RATE,
            SAMPLE_RATE as usize,
            FRAME_SIZE,
            880.0,
            440.0,
            WindowGate::tone_only(),
        )
        .unwrap();

        assert!(scan.passing_windows > 0);
        assert!(scan.longest_passing_run < scan.required_passing_run);
    }

    #[test]
    fn audio_path_scans_full_capture_for_one_continuous_second() {
        let samples = tone_samples(&[
            (SAMPLE_RATE as usize, 8_000.0, 875.0),
            (SAMPLE_RATE as usize, 8_000.0, 440.0),
        ]);
        assert!(
            assert_best_window_tone(
                "cropped-middle",
                stable_middle_half(&samples),
                SAMPLE_RATE,
                SAMPLE_RATE as usize,
                FRAME_SIZE,
                880.0,
                440.0,
            )
            .is_err(),
            "the middle-only slice intentionally lacks one continuous valid second"
        );

        let temp = tempfile::tempdir().unwrap();
        let path = save_wav(temp.path(), "full-capture.wav", &samples).unwrap();
        assert_audio_path(&path, 880.0, 440.0)
            .expect("the full capture contains one continuous valid second");
    }

    /// `secs` of `hz` at `rate`, produced by the *production* tone generator
    /// so every threshold test judges the exact signal the harness sends.
    fn amr_capture(rate: u32, hz: f32, secs: f32) -> Vec<i16> {
        let frame = frame_samples(rate);
        let frames = (secs * 50.0).round() as usize;
        (0..frames)
            .flat_map(|index| generate_tone_at_rate(hz, index, frame, rate))
            .collect()
    }

    /// Rewrite each 20 ms frame, the way a framing or relay defect does.
    fn per_frame(samples: &[i16], rate: u32, f: impl Fn(usize, &[i16]) -> Vec<i16>) -> Vec<i16> {
        let frame = frame_samples(rate);
        samples
            .chunks(frame)
            .enumerate()
            .flat_map(|(index, chunk)| f(index, chunk))
            .collect()
    }

    /// Additive white noise at a stated SNR relative to the signal's RMS.
    /// Deterministic LCG — this does not need to be a good generator, only a
    /// broadband and repeatable one.
    fn with_noise_at_snr(samples: &[i16], snr_db: f32, seed: u64) -> Vec<i16> {
        let energy: f64 = samples.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
        let rms = (energy / samples.len() as f64).sqrt();
        let noise_rms = rms / 10f64.powf(f64::from(snr_db) / 20.0);
        // A uniform variable on [-1, 1) has RMS 1/sqrt(3).
        let scale = noise_rms * 3f64.sqrt();
        let mut state = seed;
        samples
            .iter()
            .map(|&s| {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let uniform = ((state >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0;
                (f64::from(s) + uniform * scale).clamp(-32768.0, 32767.0) as i16
            })
            .collect()
    }

    #[test]
    fn amr_quality_accepts_a_clean_capture_at_either_rate() {
        let temp = tempfile::tempdir().unwrap();
        for rate in [8_000u32, 16_000] {
            let samples = amr_capture(rate, 880.0, 1.5);
            assert_amr_tone_quality_samples("clean", &samples, rate, 880.0, 440.0)
                .unwrap_or_else(|error| panic!("{rate} Hz clean capture should pass: {error}"));
            // And through the file path, so the header check admits a correct
            // header rather than only rejecting wrong ones.
            let path =
                save_wav_at_rate(temp.path(), &format!("{rate}.wav"), &samples, rate).unwrap();
            assert_amr_tone_quality(&path, rate, 880.0, 440.0)
                .unwrap_or_else(|error| panic!("{rate} Hz file capture should pass: {error}"));
            // The amplitude calibration: the fundamental of the production
            // tone reads back as what was sent, within 1%.
            let window = &samples[..tone_analysis_window_samples(rate)];
            let quality = tone_quality(window, rate, 880.0);
            let relative =
                (quality.fundamental_amplitude - TONE_PEAK_AMPLITUDE).abs() / TONE_PEAK_AMPLITUDE;
            assert!(
                relative < 0.01,
                "fundamental read {} of {} sent",
                quality.fundamental_amplitude,
                TONE_PEAK_AMPLITUDE
            );
        }
    }

    #[test]
    fn amr_quality_rejects_our_own_tone_coming_back() {
        let samples = amr_capture(16_000, 440.0, 1.5);
        assert!(
            assert_amr_tone_quality_samples("echo", &samples, 16_000, 880.0, 440.0).is_err(),
            "a recording of the tone we sent must not pass as the far end's"
        );
    }

    /// The wrong-rate case: 16 kHz samples read at 8 kHz are a real, clean
    /// tone — an octave low. The *analysis* has to catch it, not only the
    /// header check, which is why this goes through the samples entry point.
    #[test]
    fn amr_quality_rejects_a_wideband_capture_read_as_narrowband() {
        let samples = amr_capture(16_000, 880.0, 1.5);
        assert_amr_tone_quality_samples("wb-as-wb", &samples, 16_000, 880.0, 440.0)
            .expect("passes at the rate it was captured at");
        assert!(
            assert_amr_tone_quality_samples("wb-as-nb", &samples, 8_000, 880.0, 440.0).is_err(),
            "read at half the rate the 880Hz tone lands on 440Hz, which must not pass as 880Hz"
        );
    }

    /// And the header check catches the file whose label disagrees with the
    /// negotiated rate before any analysis runs.
    #[test]
    fn amr_quality_rejects_a_mislabeled_wav_header() {
        let temp = tempfile::tempdir().unwrap();
        let samples = amr_capture(16_000, 880.0, 1.5);
        let path = save_wav_at_rate(temp.path(), "wb.wav", &samples, 16_000).unwrap();
        let error = assert_amr_tone_quality(&path, 8_000, 880.0, 440.0)
            .expect_err("a 16 kHz header must not satisfy an 8 kHz leg");
        assert!(
            error.to_string().contains("WAV header"),
            "the failure should name the header mismatch, got: {error}"
        );
    }

    #[test]
    fn amr_quality_rejects_one_good_frame_then_silence() {
        let rate = 8_000u32;
        let frame = frame_samples(rate);
        let mut samples = amr_capture(rate, 880.0, 1.5);
        for sample in samples.iter_mut().skip(frame) {
            *sample = 0;
        }
        assert!(
            assert_amr_tone_quality_samples("one-frame", &samples, rate, 880.0, 440.0).is_err(),
            "one correct frame and 1.48s of silence passed the old check at ratio 44"
        );
    }

    /// Full level, right pitch, single-digit SNR: a square wave's fundamental
    /// holds 8/π² of its power (+6.3 dB), so only the SNR clause catches it.
    /// The same test pins that the *other* clauses do not: its frames are
    /// louder than the floor and its fundamental reads above what we sent.
    #[test]
    fn amr_quality_rejects_a_sign_only_square_wave() {
        let rate = 8_000u32;
        let clean = amr_capture(rate, 880.0, 1.5);
        let squared: Vec<i16> = clean
            .iter()
            .map(|&s| {
                if s > 0 {
                    TONE_PEAK_AMPLITUDE as i16
                } else {
                    -(TONE_PEAK_AMPLITUDE as i16)
                }
            })
            .collect();
        assert!(
            assert_amr_tone_quality_samples("square", &squared, rate, 880.0, 440.0).is_err(),
            "1-bit squaring passed the old check at ratio 441"
        );
        let window = &squared[..tone_analysis_window_samples(rate)];
        let quality = tone_quality(window, rate, 880.0);
        assert!(
            quality.snr_db < AMR_MIN_TONE_SNR_DB,
            "snr {}",
            quality.snr_db
        );
        assert!(
            quality.min_frame_rms > AMR_MIN_FRAME_RMS,
            "the square is loud; the level clause must not be what catches it"
        );
        assert!(
            quality.fundamental_amplitude > TONE_PEAK_AMPLITUDE,
            "a square's fundamental is 4A/π — an amplitude clause would wave it through"
        );
    }

    /// Perfect spectrum, no level: only the frame-RMS clause catches
    /// attenuation, which is why the level floor exists separately from SNR.
    #[test]
    fn amr_quality_rejects_a_hundredfold_attenuated_capture() {
        let rate = 8_000u32;
        let quiet: Vec<i16> = amr_capture(rate, 880.0, 1.5)
            .iter()
            .map(|&s| s / 100)
            .collect();
        assert!(
            assert_amr_tone_quality_samples("quiet", &quiet, rate, 880.0, 440.0).is_err(),
            "100x attenuation passed the old check at ratio 6820"
        );
        let window = &quiet[..tone_analysis_window_samples(rate)];
        let quality = tone_quality(window, rate, 880.0);
        assert!(
            quality.snr_db >= 30.0,
            "attenuation preserves spectral purity (snr {}); deleting the RMS floor must break this test",
            quality.snr_db
        );
        assert!(quality.min_frame_rms < AMR_MIN_FRAME_RMS);
    }

    #[test]
    fn amr_quality_rejects_every_other_frame_zeroed() {
        let rate = 8_000u32;
        let clean = amr_capture(rate, 880.0, 1.5);
        let gated = per_frame(&clean, rate, |index, chunk| {
            if index % 2 == 0 {
                chunk.to_vec()
            } else {
                vec![0; chunk.len()]
            }
        });
        assert!(
            assert_amr_tone_quality_samples("gated", &gated, rate, 880.0, 440.0).is_err(),
            "50% frame dropout passed the old check at ratio 922"
        );
    }

    /// The subtle one: full amplitude, no dropouts, energy on the right bin —
    /// but each frame's phase runs backwards, so the splatter at the frame
    /// rate wrecks the SNR and nothing else.
    #[test]
    fn amr_quality_rejects_per_frame_time_reversal() {
        let rate = 8_000u32;
        let clean = amr_capture(rate, 880.0, 1.5);
        let reversed = per_frame(&clean, rate, |_, chunk| {
            let mut frame = chunk.to_vec();
            frame.reverse();
            frame
        });
        assert!(
            assert_amr_tone_quality_samples("reversed", &reversed, rate, 880.0, 440.0).is_err(),
            "per-frame reversal passed the old check at ratio 23"
        );
        let window = &reversed[..tone_analysis_window_samples(rate)];
        let quality = tone_quality(window, rate, 880.0);
        assert!(
            quality.min_frame_rms > AMR_MIN_FRAME_RMS,
            "reversal conserves per-frame energy; the level clause must not be what catches it"
        );
    }

    /// The regression this whole gate exists for, pinned from both sides with
    /// the figures measured on real captures: a path measured at −12.6 dB
    /// passed the old check; the cleanest path's worst window measured
    /// +25.7 dB.
    #[test]
    fn amr_quality_brackets_the_measured_pbx_captures() {
        let rate = 8_000u32;
        let clean = amr_capture(rate, 880.0, 1.5);
        let degraded = with_noise_at_snr(&clean, -12.6, 7);
        assert!(
            assert_amr_tone_quality_samples("degraded", &degraded, rate, 880.0, 440.0).is_err(),
            "the real degraded capture measured -12.6 dB and passed the old check at 237x"
        );
        let clean_worst = with_noise_at_snr(&clean, 25.7, 11);
        assert_amr_tone_quality_samples("clean-worst", &clean_worst, rate, 880.0, 440.0)
            .expect("the cleanest real capture's worst window (+25.7 dB) must pass");
    }

    /// What makes the SNR threshold a physical quantity rather than a tuned
    /// index: noise injected at a stated SNR reads back at that SNR.
    #[test]
    fn tone_quality_reads_back_true_snr() {
        let rate = 8_000u32;
        let clean = amr_capture(rate, 880.0, 0.2);
        for (case, injected) in [(1u64, 30.0f32), (2, 20.0), (3, 15.0), (4, 10.0)] {
            let noisy = with_noise_at_snr(&clean, injected, case);
            let quality = tone_quality(&noisy[..tone_analysis_window_samples(rate)], rate, 880.0);
            assert!(
                (quality.snr_db - injected).abs() < 1.0,
                "injected {injected} dB, read {} dB",
                quality.snr_db
            );
        }
    }

    /// The 200 ms window is load-bearing: every harness tone must land on an
    /// exact Goertzel bin at both rates, or a clean tone's measurable SNR
    /// caps near 17 dB and the gate becomes unmeetable. Anyone adding a tone
    /// frequency adds it here.
    #[test]
    fn harness_tones_land_on_an_exact_goertzel_bin() {
        for rate in [8_000u32, 16_000] {
            let window = tone_analysis_window_samples(rate);
            for hz in [
                ENDPOINT_2001_TONE_HZ,
                ENDPOINT_2002_TONE_HZ,
                ENDPOINT_1003_TONE_HZ,
            ] {
                let exact = window as f32 * hz / rate as f32;
                assert!(
                    (exact - exact.round()).abs() < 1e-3,
                    "{hz} Hz falls {exact} bins into a {window}-sample window at {rate} Hz"
                );
            }
        }
    }

    /// Pins the continuity requirement — and documents why the wideband
    /// capture floor had to become a duration: a capture shorter than the
    /// required run cannot pass no matter how clean it is.
    #[test]
    fn amr_quality_rejects_a_clean_but_too_short_capture() {
        let rate = 8_000u32;
        let short = amr_capture(rate, 880.0, 0.8);
        assert!(
            assert_amr_tone_quality_samples("short", &short, rate, 880.0, 440.0).is_err(),
            "0.8s cannot contain the required 1.0s of continuous tone"
        );
        let enough = amr_capture(rate, 880.0, 1.2);
        assert_amr_tone_quality_samples("enough", &enough, rate, 880.0, 440.0)
            .expect("1.2s clean holds a full second");
    }

    /// The `SAMPLE_RATE`-literal bug that motivated threading the rate, in
    /// test form: a 16 kHz scan must measure 880 Hz as 880 Hz, not as its
    /// half or double.
    #[test]
    fn scan_at_sixteen_kilohertz_does_not_read_the_tone_an_octave_off() {
        let samples = amr_capture(16_000, 880.0, 1.5);
        assert_amr_tone_quality_samples("true-pitch", &samples, 16_000, 880.0, 440.0)
            .expect("880 Hz at 16 kHz is 880 Hz");
        assert!(
            assert_amr_tone_quality_samples("octave-up", &samples, 16_000, 1760.0, 880.0).is_err(),
            "the same capture must not read as 1760 Hz"
        );
    }

    const ALL_PAIRINGS: [CodecPairing; 5] = [
        CodecPairing::AmrNbPcmu,
        CodecPairing::AmrWbPcmu,
        CodecPairing::AmrNbBePcmu,
        CodecPairing::AmrWbBePcmu,
        CodecPairing::AmrNbAmrWb,
    ];

    /// The load-bearing property of the whole transcode scenario: the two
    /// legs' offers share nothing but telephone-event, so the PBX physically
    /// cannot native-bridge them — its own codecs must be in the path. This
    /// test, not the call passing, is what guards against a future edit
    /// quietly returning the scenario to a relayed call that still passes.
    #[test]
    fn transcode_pairings_put_disjoint_codecs_on_the_two_legs() {
        for pairing in ALL_PAIRINGS {
            let caller = pairing
                .profile_for(Role::Caller)
                .unwrap()
                .offered_codecs()
                .expect("a pairing leg always names its codecs");
            let callee = pairing
                .profile_for(Role::Callee)
                .unwrap()
                .offered_codecs()
                .expect("a pairing leg always names its codecs");
            let shared: Vec<u8> = caller
                .iter()
                .copied()
                .filter(|pt| callee.contains(pt))
                .collect();
            assert_eq!(
                shared,
                vec![101],
                "{}: the legs share {:?} beyond telephone-event, so a PBX could \
                 native-bridge and the scenario would prove nothing",
                pairing.env_value(),
                shared
            );
        }
    }

    #[test]
    fn pcmu_profile_offers_pcmu_alone() {
        assert_eq!(CodecProfile::Pcmu.offered_codecs(), Some(vec![0, 101]));
    }

    #[test]
    fn transcode_pairings_round_trip_their_names() {
        for pairing in ALL_PAIRINGS {
            assert_eq!(CodecPairing::parse(pairing.env_value()).unwrap(), pairing);
        }
        assert!(
            CodecPairing::parse("amrnb").is_err(),
            "a profile is not a pairing"
        );
    }

    #[test]
    fn transcode_pairings_have_no_profile_for_non_media_roles() {
        assert!(CodecPairing::AmrNbPcmu.profile_for(Role::Target).is_err());
    }

    /// The precedence ladder, pinned as a pure function so no test mutates
    /// the process environment.
    #[test]
    fn select_codec_profile_resolves_the_transcode_legs_per_role() {
        let caller = select_codec_profile(
            Scenario::AmrTranscodeCall,
            Some(Role::Caller),
            None,
            None,
            Some("amrwb_pcmu"),
        )
        .unwrap();
        let callee = select_codec_profile(
            Scenario::AmrTranscodeCall,
            Some(Role::Callee),
            None,
            None,
            Some("amrwb_pcmu"),
        )
        .unwrap();
        assert_eq!(caller, CodecProfile::AmrWb);
        assert_eq!(callee, CodecProfile::Pcmu);
        // No pairing set: the default pairing, not the default profile.
        assert_eq!(
            select_codec_profile(
                Scenario::AmrTranscodeCall,
                Some(Role::Caller),
                None,
                None,
                None
            )
            .unwrap(),
            CodecProfile::AmrNb
        );
    }

    /// The b2bua scenario defaults to AMR-WB — the exit criterion's codec.
    /// A `_` catch-all in `select_codec_profile` would silently default it to
    /// PCMU, so this pins the explicit arm.
    #[test]
    fn b2bua_scenario_defaults_to_wideband_and_honours_overrides() {
        assert_eq!(Scenario::parse("b2bua_call").unwrap(), Scenario::B2buaCall);
        assert_eq!(
            select_codec_profile(Scenario::B2buaCall, Some(Role::Caller), None, None, None)
                .unwrap(),
            CodecProfile::AmrWb
        );
        // A global override still wins outside the transcode scenario.
        assert_eq!(
            select_codec_profile(
                Scenario::B2buaCall,
                Some(Role::B2bua),
                None,
                Some("pcmu"),
                None
            )
            .unwrap(),
            CodecProfile::Pcmu
        );
    }

    /// The three b2bua roles map to three distinct users, and the b2bua is the
    /// user the caller dials — otherwise the inbound leg would never reach it.
    #[test]
    fn b2bua_roles_use_three_distinct_users() {
        for transport in [TransportMode::Udp, TransportMode::TlsSrtp] {
            let caller = username_for(transport, Role::Caller);
            let b2bua = username_for(transport, Role::B2bua);
            let target = username_for(transport, Role::Target);
            assert_ne!(caller, b2bua);
            assert_ne!(b2bua, target);
            assert_ne!(caller, target);
            assert_eq!(
                b2bua,
                target_user_for(transport),
                "the caller dials the b2bua, so they must share a user"
            );
        }
    }

    /// The tone the caller expects is the target's, and vice versa; the b2bua
    /// itself sends nothing, so neither side should ever expect its tone.
    #[test]
    fn b2bua_tone_mapping_is_asymmetric() {
        for transport in [TransportMode::Udp, TransportMode::TlsSrtp] {
            let caller_sends = tone_for_caller(transport);
            let far = tone_for_b2bua_far(transport);
            assert_ne!(caller_sends, far);
            assert_eq!(far, ENDPOINT_1003_TONE_HZ);
            // The b2bua's own (callee-slot) tone must not be what either end
            // listens for.
            assert_ne!(far, tone_for_callee(transport));
            assert_ne!(caller_sends, tone_for_callee(transport));
        }
    }

    #[test]
    fn select_codec_profile_rejects_a_single_profile_for_the_transcode_scenario() {
        let error = select_codec_profile(
            Scenario::AmrTranscodeCall,
            Some(Role::Caller),
            None,
            Some("amrnb"),
            None,
        )
        .expect_err("one profile cannot describe two legs");
        assert!(
            error.to_string().contains("PBX_CODEC_PAIRING"),
            "the refusal should say what to use instead: {error}"
        );
    }

    #[test]
    fn select_codec_profile_lets_an_endpoint_override_win() {
        // The per-endpoint channel outranks the pairing — it is the designed
        // escape hatch, including the deliberate same-codec vacuity check.
        let forced = select_codec_profile(
            Scenario::AmrTranscodeCall,
            Some(Role::Callee),
            Some("amrnb"),
            None,
            Some("amrnb_pcmu"),
        )
        .unwrap();
        assert_eq!(forced, CodecProfile::AmrNb);
        // And outside the transcode scenario nothing changed.
        assert_eq!(
            select_codec_profile(
                Scenario::AmrCall,
                Some(Role::Caller),
                None,
                Some("amrwb"),
                None
            )
            .unwrap(),
            CodecProfile::AmrWb
        );
        assert_eq!(
            select_codec_profile(Scenario::G729Call, None, None, None, None).unwrap(),
            CodecProfile::G729AB
        );
    }

    /// The pairing whose legs run at different rates exercises the floor as
    /// a duration: same milliseconds, different sample counts.
    #[test]
    fn transcode_legs_run_at_their_own_rate() {
        let caller = CodecPairing::AmrWbPcmu.profile_for(Role::Caller).unwrap();
        let callee = CodecPairing::AmrWbPcmu.profile_for(Role::Callee).unwrap();
        assert_eq!(amr_sample_rate(caller), 16_000);
        assert_eq!(amr_sample_rate(callee), 8_000);
        assert_eq!(
            min_received_samples(amr_sample_rate(caller)) * 1000 / 16_000,
            min_received_samples(amr_sample_rate(callee)) * 1000 / 8_000,
        );
    }
}
