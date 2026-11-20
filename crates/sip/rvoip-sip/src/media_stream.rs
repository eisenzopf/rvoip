//! D4 — `MediaStream` impl for SIP sessions, the wrapper that closes the
//! `SipAdapter::streams()` gap.
//!
//! Wraps the existing PCM-level audio API ([`UnifiedCoordinator::subscribe_to_audio`]
//! / [`UnifiedCoordinator::send_audio`]) so the orchestrator-level
//! [`Orchestrator::bridge_connections`](rvoip_core::orchestrator::Orchestrator::bridge_connections)
//! can talk to the SIP leg in the same vocabulary it uses for WebRTC:
//! `MediaFrame { payload: Bytes }` channels driven by `frames_in()` /
//! `frames_out()`.
//!
//! **Payload contract — important.** `MediaFrame.payload` contains codec
//! payload bytes only, never an RTP wire header. Both this SIP stream and the
//! WebRTC inbound pump follow that contract; the orchestrator's `Transcoder`
//! consumes the same representation. RTP timestamps and payload types travel
//! in their dedicated `MediaFrame` fields, and each transport adapter creates
//! its own outbound RTP packet at the network boundary.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock, Weak};

use async_trait::async_trait;
use bytes::Bytes;
use chrono::Utc;
use std::sync::Mutex;
use tokio::sync::{mpsc, watch, Mutex as AsyncMutex};
use tokio::task::AbortHandle;

use rvoip_core::capability::CodecInfo;
use rvoip_core::connection::Direction;
use rvoip_core::error::{Result as RvoipResult, RvoipError};
use rvoip_core::ids::StreamId;
use rvoip_core::stream::{
    MediaFrame, MediaReceiverReservation, MediaStream, QualitySnapshot, StreamKind,
};
use rvoip_core_traits::peer_media::PeerMediaFrame;

use crate::api::unified::UnifiedCoordinator;
use crate::SessionId;

#[cfg(feature = "amr-wb")]
use rvoip_media_core::codec::audio::amr::AmrAdapter;
use rvoip_media_core::codec::audio::common::AudioCodec;
use rvoip_media_core::codec::audio::g711::G711Codec;
#[cfg(feature = "g729")]
use rvoip_media_core::codec::audio::g729::{G729Annexes, G729Codec, G729Config};
#[cfg(feature = "opus")]
use rvoip_media_core::codec::audio::opus::{OpusCodec, OpusConfig};
#[cfg(any(feature = "g729", feature = "opus"))]
use rvoip_media_core::types::SampleRate;

/// SIP G.711 PCMU sample rate (8 kHz / 20 ms / 160 samples per frame).
const G711_SAMPLE_RATE: u32 = 8_000;

enum SipPayloadCodec {
    G711(G711Codec),
    #[cfg(feature = "g729")]
    G729(Box<G729Codec>),
    #[cfg(feature = "opus")]
    Opus(OpusCodec),
    #[cfg(feature = "amr-wb")]
    AmrWb(Box<AmrAdapter>),
}

impl SipPayloadCodec {
    fn from_negotiated(
        config: &crate::session_store::state::NegotiatedConfig,
        payload_type: u8,
    ) -> Result<Self, &'static str> {
        // Keep codec construction and the descriptor published to rvoip-core
        // on one validation boundary. Otherwise a malformed shape can build a
        // hard-coded codec here while `codec_descriptor` correctly rejects it.
        codec_descriptor(config, payload_type)?;
        if matches!(
            config.codec.to_ascii_lowercase().as_str(),
            "pcmu" | "g.711-mu" | "g711-mu" | "g711-u"
        ) {
            return G711Codec::mu_law(G711_SAMPLE_RATE, 1)
                .map(Self::G711)
                .map_err(|_| "pcmu-codec-init");
        }
        if matches!(
            config.codec.to_ascii_lowercase().as_str(),
            "pcma" | "g.711-a" | "g711-a"
        ) {
            return G711Codec::a_law(G711_SAMPLE_RATE, 1)
                .map(Self::G711)
                .map_err(|_| "pcma-codec-init");
        }
        if is_g729_codec(&config.codec) {
            #[cfg(feature = "g729")]
            {
                let annex_b = negotiated_g729_annex_b(config);
                return G729Codec::new(
                    SampleRate::Rate8000,
                    1,
                    G729Config {
                        annexes: G729Annexes {
                            annex_a: true,
                            annex_b,
                        },
                        frame_size_ms: 10.0,
                        enable_vad: annex_b,
                        enable_cng: annex_b,
                    },
                )
                .map(Box::new)
                .map(Self::G729)
                .map_err(|_| "g729-codec-init");
            }
            #[cfg(not(feature = "g729"))]
            {
                return Err("g729-feature-disabled");
            }
        }
        if config.codec.eq_ignore_ascii_case("opus") {
            #[cfg(feature = "opus")]
            {
                let sample_rate =
                    SampleRate::from_hz(config.sample_rate).ok_or("opus-sample-rate")?;
                return OpusCodec::new(sample_rate, config.channels, OpusConfig::default())
                    .map(Self::Opus)
                    .map_err(|_| "opus-codec-init");
            }
            #[cfg(not(feature = "opus"))]
            {
                return Err("opus-feature-disabled");
            }
        }
        if is_amr_wb_codec(&config.codec) {
            #[cfg(feature = "amr-wb")]
            {
                return AmrAdapter::new(payload_type, "AMR-WB", config.fmtp.as_deref())
                    .map(Box::new)
                    .map(Self::AmrWb)
                    .map_err(|_| "amr-wb-codec-init");
            }
            #[cfg(not(feature = "amr-wb"))]
            {
                return Err("amr-wb-feature-disabled");
            }
        }
        Err("unsupported-negotiated-codec")
    }

    fn encode(
        &mut self,
        frame: &rvoip_media_core::types::AudioFrame,
    ) -> rvoip_media_core::error::Result<Vec<u8>> {
        match self {
            Self::G711(codec) => codec.encode(frame),
            #[cfg(feature = "g729")]
            Self::G729(codec) => codec.encode(frame),
            #[cfg(feature = "opus")]
            Self::Opus(codec) => codec.encode(frame),
            #[cfg(feature = "amr-wb")]
            Self::AmrWb(codec) => codec.encode(frame),
        }
    }

    /// Encode PCM into graph payloads without hiding a codec's fixed frame
    /// boundary inside one opaque payload.
    ///
    /// The coordinator normally supplies 20 ms PCM. G.729's codec primitive
    /// consumes 10 ms at a time, while AMR-WB consumes 20 ms. Splitting here
    /// keeps every graph frame independently decodable and gives each one the
    /// correct RTP timestamp. It also handles a peer that bundles more than
    /// one AMR frame in one RTP packet: the transport decoder returns all PCM
    /// samples, and this boundary emits one valid RFC 4867 payload per frame.
    fn encode_graph_frames(
        &mut self,
        frame: &rvoip_media_core::types::AudioFrame,
    ) -> rvoip_media_core::error::Result<Vec<(Vec<u8>, u32)>> {
        #[cfg(feature = "g729")]
        if let Self::G729(codec) = self {
            return codec.encode_payload_frames(frame).map(|packets| {
                packets
                    .into_iter()
                    .map(|packet| (packet.payload, packet.timestamp))
                    .collect()
            });
        }
        #[cfg(feature = "amr-wb")]
        if let Self::AmrWb(codec) = self {
            return encode_fixed_graph_frames(codec.as_mut(), frame, 320);
        }
        Ok(vec![(self.encode(frame)?, frame.timestamp)])
    }

    fn decode(
        &mut self,
        payload: &[u8],
    ) -> rvoip_media_core::error::Result<rvoip_media_core::types::AudioFrame> {
        match self {
            Self::G711(codec) => codec.decode(payload),
            #[cfg(feature = "g729")]
            Self::G729(codec) => codec.decode_rtp_payload(payload),
            #[cfg(feature = "opus")]
            Self::Opus(codec) => codec.decode(payload),
            #[cfg(feature = "amr-wb")]
            Self::AmrWb(codec) => codec.decode(payload),
        }
    }
}

#[cfg(feature = "amr-wb")]
fn encode_fixed_graph_frames<C: AudioCodec>(
    codec: &mut C,
    frame: &rvoip_media_core::types::AudioFrame,
    samples_per_channel: usize,
) -> rvoip_media_core::error::Result<Vec<(Vec<u8>, u32)>> {
    let channels = usize::from(frame.channels.max(1));
    let frame_samples = samples_per_channel * channels;
    if frame.samples.is_empty() || !frame.samples.len().is_multiple_of(frame_samples) {
        return Err(rvoip_media_core::error::CodecError::InvalidFrameSize {
            expected: frame_samples,
            actual: frame.samples.len(),
        }
        .into());
    }
    let mut encoded = Vec::with_capacity(frame.samples.len() / frame_samples);
    for (index, samples) in frame.samples.chunks_exact(frame_samples).enumerate() {
        let timestamp = frame
            .timestamp
            .wrapping_add((index * samples_per_channel) as u32);
        let payload = codec.encode(&rvoip_media_core::types::AudioFrame::new(
            samples.to_vec(),
            frame.sample_rate,
            frame.channels,
            timestamp,
        ))?;
        encoded.push((payload, timestamp));
    }
    Ok(encoded)
}

fn normalized_codec_name(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_uppercase)
        .collect()
}

fn is_g729_codec(name: &str) -> bool {
    matches!(
        normalized_codec_name(name).as_str(),
        "G729" | "G729A" | "G729AB" | "G729BA"
    )
}

fn is_amr_wb_codec(name: &str) -> bool {
    normalized_codec_name(name) == "AMRWB"
}

fn negotiated_g729_annex_b(config: &crate::session_store::state::NegotiatedConfig) -> bool {
    match normalized_codec_name(&config.codec).as_str() {
        "G729A" => false,
        "G729AB" | "G729BA" => true,
        // RFC 4855 keeps Annex B enabled when the parameter is absent. The
        // SDP negotiation layer normally canonicalizes this to G729A/G729BA,
        // but retaining the rule here makes direct NegotiatedConfig users
        // behave identically.
        _ => config
            .fmtp
            .as_deref()
            .and_then(|fmtp| {
                fmtp.split(';').find_map(|parameter| {
                    let (name, value) = parameter.trim().split_once('=')?;
                    if !name.trim().eq_ignore_ascii_case("annexb") {
                        return None;
                    }
                    match value.trim().trim_matches('"').to_ascii_lowercase().as_str() {
                        "yes" | "true" | "1" => Some(true),
                        "no" | "false" | "0" => Some(false),
                        _ => None,
                    }
                })
            })
            .unwrap_or(true),
    }
}

fn materialized_g729_fmtp(config: &crate::session_store::state::NegotiatedConfig) -> String {
    let mut parameters = config
        .fmtp
        .as_deref()
        .into_iter()
        .flat_map(|fmtp| fmtp.split(';'))
        .map(str::trim)
        .filter(|parameter| !parameter.is_empty())
        .filter(|parameter| {
            parameter
                .split_once('=')
                .is_none_or(|(name, _)| !name.trim().eq_ignore_ascii_case("annexb"))
        })
        .map(str::to_string)
        .collect::<Vec<_>>();
    parameters.push(format!(
        "annexb={}",
        if negotiated_g729_annex_b(config) {
            "yes"
        } else {
            "no"
        }
    ));
    parameters.join(";")
}

pub(crate) fn codec_descriptor(
    config: &crate::session_store::state::NegotiatedConfig,
    payload_type: u8,
) -> Result<(CodecInfo, u8), &'static str> {
    let mut descriptor_fmtp = config.fmtp.clone();
    let name = if matches!(
        config.codec.to_ascii_lowercase().as_str(),
        "pcmu" | "g.711-mu" | "g711-mu" | "g711-u"
    ) {
        if config.sample_rate != 8_000 || config.channels != 1 {
            return Err("invalid-pcmu-shape");
        }
        "g.711-mu"
    } else if matches!(
        config.codec.to_ascii_lowercase().as_str(),
        "pcma" | "g.711-a" | "g711-a"
    ) {
        if config.sample_rate != 8_000 || config.channels != 1 {
            return Err("invalid-pcma-shape");
        }
        "g.711-a"
    } else if is_g729_codec(&config.codec) {
        if !cfg!(feature = "g729") {
            return Err("g729-feature-disabled");
        }
        if config.sample_rate != 8_000 || config.channels != 1 {
            return Err("invalid-g729-shape");
        }
        descriptor_fmtp = Some(materialized_g729_fmtp(config));
        "g729"
    } else if config.codec.eq_ignore_ascii_case("opus") {
        if !cfg!(feature = "opus") {
            return Err("opus-feature-disabled");
        }
        if config.sample_rate != 48_000 || !matches!(config.channels, 1 | 2) {
            return Err("invalid-opus-shape");
        }
        "opus"
    } else if is_amr_wb_codec(&config.codec) {
        if !cfg!(feature = "amr-wb") {
            return Err("amr-wb-feature-disabled");
        }
        if config.sample_rate != 16_000 || config.channels != 1 {
            return Err("invalid-amr-wb-shape");
        }
        if !(96..=127).contains(&payload_type) || payload_type == 101 {
            return Err("invalid-amr-wb-payload-type");
        }
        "AMR-WB"
    } else {
        return Err("unsupported-negotiated-codec");
    };
    Ok((
        CodecInfo {
            name: name.to_string(),
            clock_rate_hz: config.sample_rate,
            channels: config.channels,
            // Carried, not dropped. `rvoip-core` keys its transcoding codec
            // groups on this, so a hard-coded `None` puts every SIP leg in one
            // group and silently discards whatever the peer negotiated.
            fmtp: descriptor_fmtp,
            // The SIP leg is the one place that unambiguously knows this: it
            // is the payload type the SDP answer settled on, already this
            // function's own argument. Reporting it is what lets consumers
            // downstream stop deriving a payload type from the codec name.
            payload_type: Some(payload_type),
        },
        payload_type,
    ))
}

/// Frame channel depth. Same default as `rvoip-webrtc` (see
/// `crates/webrtc/rvoip-webrtc/src/media/pump.rs::FRAME_CHANNEL_CAP`).
const FRAME_CHANNEL_CAP: usize = 64;

/// Epoch-normalized clock for PCM handed to the negotiated SIP media runtime.
///
/// The media graph has already translated `MediaFrame::timestamp_rtp` into the
/// sink codec's RTP clock. We choose a local epoch for the first frame, then
/// advance by emitted samples plus any forward gap in that input timeline.
/// This avoids copying a remote random epoch while preserving Annex-B DTX or
/// packet-loss intervals that must remain visible to the RTP packetizer.
#[derive(Default)]
struct OutboundRtpClock {
    next_output_timestamp: u32,
    expected_input_timestamp: Option<u32>,
}

fn advance_outbound_timestamp(
    clock: &mut OutboundRtpClock,
    samples_emitted: usize,
    upstream_rtp_ts: u32,
) -> u32 {
    let forward_gap = clock
        .expected_input_timestamp
        .map(|expected| upstream_rtp_ts.wrapping_sub(expected))
        // A delta in the older half of the modular timestamp space means an
        // out-of-order packet or a restarted sender, not a centuries-long gap.
        .filter(|gap| *gap <= i32::MAX as u32)
        .unwrap_or(0);
    let ts = clock.next_output_timestamp.wrapping_add(forward_gap);
    let samples_emitted = samples_emitted as u32;
    clock.next_output_timestamp = ts.wrapping_add(samples_emitted);
    clock.expected_input_timestamp = Some(upstream_rtp_ts.wrapping_add(samples_emitted));
    ts
}

/// One-take wrapper for the inbound `MediaFrame` receiver — mirrors the
/// `WebRtcMediaStream` shape so consumers calling `frames_in()` twice get
/// a closed channel on the second call instead of a panic.
struct SipMediaStreamInner {
    stream_id: StreamId,
    codec: Arc<RwLock<CodecInfo>>,
    direction: Direction,
    frames_in_rx: Mutex<Option<mpsc::Receiver<MediaFrame>>>,
    frames_in_tx: Mutex<Option<mpsc::Sender<MediaFrame>>>,
    frames_out_tx: mpsc::Sender<MediaFrame>,
    frames_out_rx: Mutex<Option<mpsc::Receiver<MediaFrame>>>,
    peer_out_tx: mpsc::Sender<PeerMediaFrame>,
    peer_out_rx: Mutex<Option<mpsc::Receiver<PeerMediaFrame>>>,
    bind_target: Mutex<Option<SipMediaBindTarget>>,
    driver_abort: Mutex<Option<AbortHandle>>,
    lifecycle_gate: AsyncMutex<()>,
    lifecycle: Arc<SipMediaLifecycleState>,
    outbound_writes_activated: AtomicBool,
    cancel: watch::Sender<bool>,
    codec_updates: watch::Sender<Option<SipMediaCodecRuntime>>,
    /// Last quality the media layer reported for this stream's connection.
    ///
    /// Retained here because quality arrives by *push* — media-core distills
    /// RTCP RR/XR and the adapter routes it per connection — while
    /// `quality_snapshot` is a *pull*. Without somewhere to land, a poller
    /// would read defaults and report perfect quality for every call, which
    /// is worse than reporting none because it looks like evidence.
    ///
    /// `None` until the first report, so a caller can tell "no measurement
    /// yet" from "measured, and it is fine".
    last_quality: Mutex<Option<QualitySnapshot>>,
}

#[derive(Clone)]
struct SipMediaCodecRuntime {
    negotiated: crate::session_store::state::NegotiatedConfig,
    payload_type: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SipMediaLifecycle {
    Dormant,
    Binding,
    Bound,
    Closing,
    Closed,
    Failed,
}

struct SipMediaBindTarget {
    coordinator: Weak<UnifiedCoordinator>,
    session_id: SessionId,
}

impl SipMediaBindTarget {
    fn matches(&self, coordinator: &Arc<UnifiedCoordinator>, session_id: &SessionId) -> bool {
        self.session_id == *session_id && self.coordinator.ptr_eq(&Arc::downgrade(coordinator))
    }
}

struct SipMediaLifecycleState {
    state: Mutex<SipMediaLifecycle>,
    updates: watch::Sender<SipMediaLifecycle>,
}

impl SipMediaLifecycleState {
    fn new() -> Self {
        let (updates, _) = watch::channel(SipMediaLifecycle::Dormant);
        Self {
            state: Mutex::new(SipMediaLifecycle::Dormant),
            updates,
        }
    }

    fn current(&self) -> SipMediaLifecycle {
        *self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn subscribe(&self) -> watch::Receiver<SipMediaLifecycle> {
        self.updates.subscribe()
    }

    fn transition(
        &self,
        allowed: impl FnOnce(SipMediaLifecycle) -> bool,
        next: SipMediaLifecycle,
    ) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !allowed(*state) {
            return false;
        }
        *state = next;
        self.updates.send_replace(next);
        true
    }

    fn begin_binding(&self) -> bool {
        self.transition(
            |state| state == SipMediaLifecycle::Dormant,
            SipMediaLifecycle::Binding,
        )
    }

    fn mark_bound(&self) -> bool {
        self.transition(
            |state| state == SipMediaLifecycle::Binding,
            SipMediaLifecycle::Bound,
        )
    }

    fn mark_failed(&self) -> bool {
        self.transition(
            |state| {
                matches!(
                    state,
                    SipMediaLifecycle::Dormant
                        | SipMediaLifecycle::Binding
                        | SipMediaLifecycle::Bound
                )
            },
            SipMediaLifecycle::Failed,
        )
    }

    fn begin_closing(&self) -> bool {
        self.transition(
            |state| {
                !matches!(
                    state,
                    SipMediaLifecycle::Closing | SipMediaLifecycle::Closed
                )
            },
            SipMediaLifecycle::Closing,
        )
    }

    fn mark_closed(&self) {
        self.transition(
            |state| state != SipMediaLifecycle::Closed,
            SipMediaLifecycle::Closed,
        );
    }
}

/// Concrete `MediaStream` for the SIP transport.
///
/// The adapter allocates it in a dormant, local-only state before exposing a
/// connection. Binding to coordinator audio is retained, single-flight work
/// that happens only when the corresponding signaling route is activated.
pub struct SipMediaStream {
    inner: Arc<SipMediaStreamInner>,
}

impl SipMediaStream {
    #[cfg(feature = "test-hooks")]
    pub(crate) fn inject_failure_for_test(&self) -> bool {
        self.inner.lifecycle.mark_failed()
    }

    /// Record the media layer's latest quality report for this stream.
    pub(crate) fn record_quality(&self, snapshot: QualitySnapshot) {
        *self
            .inner
            .last_quality
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(snapshot);
    }

    /// Allocate a local-only media stream without touching a SIP session.
    ///
    /// This constructor allocates only bounded channels and a stable stream
    /// identifier. It does not subscribe to coordinator audio, create media,
    /// start a task, allocate a socket, or emit a packet. For compatibility,
    /// a publicly constructed outbound stream becomes writable when binding
    /// completes; staged adapters use the private deferred constructor.
    pub fn dormant(direction: Direction) -> Arc<Self> {
        Self::allocate_dormant(direction, true)
    }

    pub(crate) fn dormant_deferred(direction: Direction) -> Arc<Self> {
        Self::allocate_dormant(direction, direction == Direction::Inbound)
    }

    fn allocate_dormant(direction: Direction, outbound_writes_activated: bool) -> Arc<Self> {
        let stream_id = StreamId::new();
        let codec = CodecInfo {
            name: "g.711-mu".to_string(),
            clock_rate_hz: G711_SAMPLE_RATE,
            channels: 1,
            fmtp: None,
            // A dormant stream has negotiated nothing, and this descriptor is
            // a placeholder replaced once it has. Reporting PCMU's 0 here
            // would be reporting a negotiation that has not happened.
            payload_type: None,
        };
        let (frames_in_tx, frames_in_rx) = mpsc::channel::<MediaFrame>(FRAME_CHANNEL_CAP);
        let (frames_out_tx, frames_out_rx) = mpsc::channel::<MediaFrame>(FRAME_CHANNEL_CAP);
        let (peer_out_tx, peer_out_rx) = mpsc::channel(FRAME_CHANNEL_CAP);
        let (cancel, _) = watch::channel(false);
        let (codec_updates, _) = watch::channel(None);

        Arc::new(Self {
            inner: Arc::new(SipMediaStreamInner {
                stream_id,
                codec: Arc::new(RwLock::new(codec)),
                direction,
                frames_in_rx: Mutex::new(Some(frames_in_rx)),
                frames_in_tx: Mutex::new(Some(frames_in_tx)),
                frames_out_tx,
                frames_out_rx: Mutex::new(Some(frames_out_rx)),
                peer_out_tx,
                peer_out_rx: Mutex::new(Some(peer_out_rx)),
                bind_target: Mutex::new(None),
                driver_abort: Mutex::new(None),
                lifecycle_gate: AsyncMutex::new(()),
                lifecycle: Arc::new(SipMediaLifecycleState::new()),
                outbound_writes_activated: AtomicBool::new(outbound_writes_activated),
                last_quality: Mutex::new(None),
                cancel,
                codec_updates,
            }),
        })
    }

    /// Build a stream backed by an active SIP session.
    ///
    /// Kept as the compatibility surface for inbound and legacy callers. The
    /// implementation is the same dormant allocation followed by one retained
    /// bind, so every path shares the same lifecycle and cleanup behavior.
    pub async fn new(
        coordinator: Arc<UnifiedCoordinator>,
        session_id: SessionId,
        direction: Direction,
    ) -> crate::errors::Result<Arc<Self>> {
        let stream = Self::dormant(direction);
        stream.bind(coordinator, session_id).await?;
        Ok(stream)
    }

    /// Bind this dormant stream to one SIP session exactly once.
    ///
    /// The first caller starts a retained driver. Dropping that caller does not
    /// cancel the driver, and concurrent callers observe the same terminal
    /// outcome without creating another subscription or another pump pair.
    pub async fn bind(
        self: &Arc<Self>,
        coordinator: Arc<UnifiedCoordinator>,
        session_id: SessionId,
    ) -> crate::errors::Result<()> {
        let mut lifecycle = self.inner.lifecycle.subscribe();
        self.start_bind(coordinator, session_id).await?;

        loop {
            match *lifecycle.borrow_and_update() {
                SipMediaLifecycle::Bound => return Ok(()),
                SipMediaLifecycle::Failed => {
                    return Err(crate::errors::SessionError::Other(
                        "SIP media bind failed".to_string(),
                    ));
                }
                SipMediaLifecycle::Closing | SipMediaLifecycle::Closed => {
                    return Err(crate::errors::SessionError::Other(
                        "SIP media stream is closed".to_string(),
                    ));
                }
                SipMediaLifecycle::Dormant | SipMediaLifecycle::Binding => {}
            }
            if lifecycle.changed().await.is_err() {
                return Err(crate::errors::SessionError::Other(
                    "SIP media lifecycle ended".to_string(),
                ));
            }
        }
    }

    /// Start the retained bind driver without waiting for SDP negotiation.
    ///
    /// Outbound SIP activation must return its signaling receipt after the
    /// INVITE is dispatched and staged events are installed, even though the
    /// media codec cannot become known until a later answer. This operation
    /// commits the immutable coordinator/session target and retained driver;
    /// [`Self::bind`] remains the compatibility API that additionally waits
    /// for the driver to publish `Bound`.
    pub(crate) async fn start_bind(
        self: &Arc<Self>,
        coordinator: Arc<UnifiedCoordinator>,
        session_id: SessionId,
    ) -> crate::errors::Result<()> {
        {
            let _gate = self.inner.lifecycle_gate.lock().await;
            let state = self.inner.lifecycle.current();
            if state != SipMediaLifecycle::Dormant {
                let matches = self
                    .inner
                    .bind_target
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .as_ref()
                    .is_some_and(|target| target.matches(&coordinator, &session_id));
                if !matches {
                    return Err(crate::errors::SessionError::Other(
                        "SIP media stream is bound to a different coordinator or session"
                            .to_string(),
                    ));
                }
            }
            match state {
                SipMediaLifecycle::Dormant => {
                    *self
                        .inner
                        .bind_target
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                        Some(SipMediaBindTarget {
                            coordinator: Arc::downgrade(&coordinator),
                            session_id: session_id.clone(),
                        });
                    let frames_in_tx = self
                        .inner
                        .frames_in_tx
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .take();
                    let frames_out_rx = self
                        .inner
                        .frames_out_rx
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .take();
                    let peer_out_rx = self
                        .inner
                        .peer_out_rx
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .take();
                    let (Some(frames_in_tx), Some(frames_out_rx), Some(peer_out_rx)) =
                        (frames_in_tx, frames_out_rx, peer_out_rx)
                    else {
                        self.inner.lifecycle.mark_failed();
                        return Err(crate::errors::SessionError::Other(
                            "SIP media channels are unavailable".to_string(),
                        ));
                    };
                    if !self.inner.lifecycle.begin_binding() {
                        return Err(crate::errors::SessionError::Other(
                            "SIP media lifecycle changed during bind".to_string(),
                        ));
                    }
                    let driver = tokio::spawn(run_media_driver(
                        Arc::clone(&self.inner.lifecycle),
                        self.inner.cancel.clone(),
                        self.inner.cancel.subscribe(),
                        coordinator,
                        session_id,
                        self.inner.stream_id.clone(),
                        Arc::clone(&self.inner.codec),
                        self.inner.codec_updates.clone(),
                        frames_in_tx,
                        frames_out_rx,
                        peer_out_rx,
                    ));
                    *self
                        .inner
                        .driver_abort
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                        Some(driver.abort_handle());
                    drop(driver);
                }
                SipMediaLifecycle::Binding | SipMediaLifecycle::Bound => {}
                SipMediaLifecycle::Failed => {
                    return Err(crate::errors::SessionError::Other(
                        "SIP media bind failed".to_string(),
                    ));
                }
                SipMediaLifecycle::Closing | SipMediaLifecycle::Closed => {
                    return Err(crate::errors::SessionError::Other(
                        "SIP media stream is closed".to_string(),
                    ));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn subscribe_lifecycle(&self) -> watch::Receiver<SipMediaLifecycle> {
        self.inner.lifecycle.subscribe()
    }

    pub(crate) fn is_bound_to(
        &self,
        coordinator: &Arc<UnifiedCoordinator>,
        session_id: &SessionId,
    ) -> bool {
        self.inner
            .bind_target
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .is_some_and(|target| target.matches(coordinator, session_id))
    }

    /// Linearize deferred outbound write availability with activation success
    /// committed for publication.
    ///
    /// Binding starts the transport pumps, but an adapter-owned outbound
    /// stream remains unwritable until the adapter has committed successful
    /// activation. Inbound and legacy streams are writable when binding
    /// completes.
    pub(crate) fn activate_outbound_writes(&self) {
        if self.inner.direction == Direction::Outbound {
            self.inner
                .outbound_writes_activated
                .store(true, Ordering::Release);
        }
    }

    /// Commit a signaling-approved codec generation to this stable stream.
    ///
    /// The stream object and its application channels stay intact while both
    /// pumps rebuild their codec state from this watch value. This method is
    /// called only after a final re-INVITE answer has committed.
    pub(crate) fn apply_negotiated_media(
        &self,
        negotiated: &crate::session_store::state::NegotiatedConfig,
        payload_type: u8,
    ) -> Result<CodecInfo, &'static str> {
        let (descriptor, payload_type) = codec_descriptor(negotiated, payload_type)?;
        *self
            .inner
            .codec
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = descriptor.clone();
        self.inner
            .codec_updates
            .send_replace(Some(SipMediaCodecRuntime {
                negotiated: negotiated.clone(),
                payload_type,
            }));
        Ok(descriptor)
    }

    fn close_local_channels(&self) {
        self.inner
            .frames_in_tx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        self.inner
            .frames_out_rx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        self.inner
            .peer_out_rx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
    }

    /// Make cancellation sticky without requiring an async runtime join.
    ///
    /// Adapter teardown calls this before dropping its last registry handle so
    /// a retained bind waiting in coordinator media cannot form a task/stream
    /// cycle. [`MediaStream::close`] performs the subsequent bounded joins.
    pub(crate) fn request_close(&self) {
        self.inner.lifecycle.begin_closing();
        self.inner.cancel.send_replace(true);
        self.close_local_channels();
    }

    async fn close_retained(self: &Arc<Self>) -> RvoipResult<()> {
        let _gate = self.inner.lifecycle_gate.lock().await;
        if self.inner.lifecycle.current() == SipMediaLifecycle::Closed {
            return Ok(());
        }
        self.request_close();

        let driver_abort = self
            .inner
            .driver_abort
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(abort) = driver_abort {
            if tokio::time::timeout(std::time::Duration::from_secs(1), async {
                while !abort.is_finished() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .is_err()
            {
                abort.abort();
                if tokio::time::timeout(std::time::Duration::from_secs(1), async {
                    while !abort.is_finished() {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .is_err()
                {
                    return Err(RvoipError::Adapter(
                        "SIP media driver did not terminate after abort".to_string(),
                    ));
                }
            }
        }
        self.inner.lifecycle.mark_closed();
        Ok(())
    }
}

impl Drop for SipMediaStream {
    fn drop(&mut self) {
        // The driver deliberately does not retain `SipMediaStreamInner`, so
        // dropping the final public stream owner is the authoritative signal
        // that a cancelled constructor/bind has no owner left to close it.
        // Wake a cooperative subscription first, then abort as a synchronous
        // fail-safe for an uncooperative coordinator future.
        self.inner.lifecycle.begin_closing();
        self.inner.cancel.send_replace(true);
        self.inner
            .frames_in_tx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        self.inner
            .frames_out_rx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        self.inner
            .peer_out_rx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(driver) = self
            .inner
            .driver_abort
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            driver.abort();
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MediaOwnerProbe {
    Ready,
    Pending,
    Terminal,
    Missing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MediaOwnerWaitResult {
    Ready,
    Terminal,
    Missing,
    Cancelled,
    TimedOut,
}

async fn wait_for_media_owner<Probe, ProbeFuture>(
    mut probe: Probe,
    cancel_tx: &watch::Sender<bool>,
    cancel_rx: &mut watch::Receiver<bool>,
    deadline: tokio::time::Instant,
) -> MediaOwnerWaitResult
where
    Probe: FnMut() -> ProbeFuture,
    ProbeFuture: std::future::Future<Output = MediaOwnerProbe>,
{
    const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(10);

    loop {
        if *cancel_tx.borrow() {
            return MediaOwnerWaitResult::Cancelled;
        }
        match probe().await {
            MediaOwnerProbe::Ready => return MediaOwnerWaitResult::Ready,
            MediaOwnerProbe::Terminal => return MediaOwnerWaitResult::Terminal,
            MediaOwnerProbe::Missing => return MediaOwnerWaitResult::Missing,
            MediaOwnerProbe::Pending => {}
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return MediaOwnerWaitResult::TimedOut;
        }
        tokio::select! {
            _ = wait_for_media_cancel(cancel_rx) => {
                return MediaOwnerWaitResult::Cancelled;
            }
            _ = tokio::time::sleep_until((now + POLL_INTERVAL).min(deadline)) => {}
        }
    }
}

use rvoip_media_core::processing::audio::playout::{PlayoutBuffer, PlayoutConfig};
use rvoip_media_core::types::AudioFrame;

enum InboundPumpEvent {
    CodecChanged,
    Audio(Option<AudioFrame>),
    PlayoutTick,
}

async fn wait_for_playout(deadline: Option<std::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
        None => std::future::pending::<()>().await,
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_media_driver(
    lifecycle: Arc<SipMediaLifecycleState>,
    cancel_tx: watch::Sender<bool>,
    mut cancel_rx: watch::Receiver<bool>,
    coordinator: Arc<UnifiedCoordinator>,
    session_id: SessionId,
    stream_id: StreamId,
    codec_descriptor_slot: Arc<RwLock<CodecInfo>>,
    codec_updates: watch::Sender<Option<SipMediaCodecRuntime>>,
    frames_in_tx: mpsc::Sender<MediaFrame>,
    frames_out_rx: mpsc::Receiver<MediaFrame>,
    peer_out_rx: mpsc::Receiver<PeerMediaFrame>,
) {
    let playout = coordinator.playout_policy();
    let setup_deadline =
        tokio::time::Instant::now() + coordinator.setup_teardown_timeout_duration();
    // Inbound `IncomingCall` publication may win a narrow race with the
    // transition's later `CreateMediaSession` commit. Treat a live session
    // without its media owner as pending, while still failing closed for a
    // missing/terminal session and bounding the wait by the shared setup
    // deadline. This preserves eager stream publication without retiring a
    // valid inbound route before the application can answer it.
    let owner_coordinator = Arc::clone(&coordinator);
    let owner_session_id = session_id.clone();
    let owner = wait_for_media_owner(
        move || {
            let coordinator = Arc::clone(&owner_coordinator);
            let session_id = owner_session_id.clone();
            async move {
                match coordinator.session_state(&session_id).await {
                    Err(_) => MediaOwnerProbe::Missing,
                    Ok(session)
                        if session.call_state.is_final()
                            || session.call_state == crate::types::CallState::Terminating =>
                    {
                        MediaOwnerProbe::Terminal
                    }
                    Ok(session) if session.media_session_id.is_some() => MediaOwnerProbe::Ready,
                    Ok(_) => MediaOwnerProbe::Pending,
                }
            }
        },
        &cancel_tx,
        &mut cancel_rx,
        setup_deadline,
    )
    .await;
    match owner {
        MediaOwnerWaitResult::Ready => {}
        MediaOwnerWaitResult::Cancelled => return,
        MediaOwnerWaitResult::Terminal => {
            tracing::warn!(target: "rvoip_sip", "SipMediaStream session became terminal before media ownership");
            lifecycle.mark_failed();
            return;
        }
        MediaOwnerWaitResult::Missing => {
            tracing::warn!(target: "rvoip_sip", "SipMediaStream session disappeared before media ownership");
            lifecycle.mark_failed();
            return;
        }
        MediaOwnerWaitResult::TimedOut => {
            tracing::warn!(target: "rvoip_sip", "SipMediaStream media ownership timed out");
            lifecycle.mark_failed();
            return;
        }
    }

    // A UAC has no established media controller callback until its answer has
    // been negotiated. Waiting for that exact negotiated configuration first
    // keeps the retained driver dormant across the INVITE/answer gap instead
    // of treating a normal pre-answer subscription miss as terminal failure.
    let negotiation_deadline = setup_deadline;
    let (negotiated, payload_type) = loop {
        if *cancel_tx.borrow() {
            return;
        }
        match coordinator.negotiated_media_config(&session_id).await {
            Ok(Some(config)) => break config,
            Ok(None) => {
                tokio::select! {
                    _ = wait_for_media_cancel(&mut cancel_rx) => return,
                    _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {}
                    _ = tokio::time::sleep_until(negotiation_deadline) => {
                        tracing::warn!(
                            target: "rvoip_sip",
                            "SipMediaStream SDP negotiation timed out"
                        );
                        lifecycle.mark_failed();
                        return;
                    }
                }
            }
            Err(error) => {
                tracing::warn!(
                    target: "rvoip_sip",
                    error = %error,
                    "SipMediaStream negotiated media lookup failed"
                );
                lifecycle.mark_failed();
                return;
            }
        }
    };
    let (resolved_descriptor, payload_type) = match codec_descriptor(&negotiated, payload_type) {
        Ok(resolved) => resolved,
        Err(reason) => {
            tracing::warn!(
                target: "rvoip_sip",
                reason,
                "SipMediaStream rejected negotiated media format"
            );
            lifecycle.mark_failed();
            return;
        }
    };
    *codec_descriptor_slot
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = resolved_descriptor;
    let runtime = SipMediaCodecRuntime {
        negotiated,
        payload_type,
    };
    codec_updates.send_replace(Some(runtime.clone()));
    let subscription = coordinator.subscribe_to_audio(&session_id);
    tokio::pin!(subscription);
    let subscriber = tokio::select! {
        _ = wait_for_media_cancel(&mut cancel_rx) => return,
        result = &mut subscription => match result {
            Ok(subscriber) => subscriber,
            Err(error) => {
                tracing::warn!(
                    target: "rvoip_sip",
                    error = %error,
                    "SipMediaStream audio subscription failed"
                );
                lifecycle.mark_failed();
                return;
            }
        }
    };
    if !lifecycle.mark_bound() {
        return;
    }

    let inbound = run_inbound_pump(
        subscriber,
        runtime.clone(),
        codec_updates.subscribe(),
        stream_id,
        frames_in_tx,
        playout,
    );
    let outbound = run_outbound_pump(
        Arc::clone(&coordinator),
        session_id.clone(),
        runtime,
        codec_updates.subscribe(),
        frames_out_rx,
        peer_out_rx,
    );
    tokio::pin!(inbound, outbound);
    let failed_pump = tokio::select! {
        _ = wait_for_media_cancel(&mut cancel_rx) => None,
        failure = &mut inbound => Some(failure),
        failure = &mut outbound => Some(failure),
    };
    if let Some(failure) = failed_pump {
        tracing::warn!(target: "rvoip_sip", failure, "SipMediaStream pump stopped unexpectedly");
        if lifecycle.mark_failed() {
            cancel_tx.send_replace(true);
        }
    }
}

/// Converts decoded packets to the fixed frame size of the configured encoder.
/// Buffering is per pump/generation and never joins samples across an RTP gap.
struct InboundPcmFramer {
    samples_per_frame: Option<usize>,
    sample_rate: u32,
    channels: u8,
    pending: Vec<i16>,
    timestamp: u32,
    expected_input: Option<u32>,
}

impl InboundPcmFramer {
    fn new(config: &crate::session_store::state::NegotiatedConfig) -> Self {
        let spec = rvoip_media_core::codec::spec::AudioCodecSpec::new(
            &config.codec,
            0,
            config.sample_rate,
            config.channels,
        );
        Self {
            samples_per_frame: config
                .codec
                .eq_ignore_ascii_case("opus")
                .then(|| spec.frame_samples_20ms()),
            sample_rate: config.sample_rate,
            channels: config.channels,
            pending: Vec::new(),
            timestamp: 0,
            expected_input: None,
        }
    }

    fn push(&mut self, frame: AudioFrame) -> Vec<AudioFrame> {
        let Some(required) = self.samples_per_frame else {
            return vec![frame];
        };
        let channels = usize::from(self.channels);
        // Opus allows up to 120 ms in one decoded packet. Reject malformed
        // shapes before growing the buffer; retain less than one 20 ms frame.
        if required == 0
            || channels == 0
            || frame.sample_rate != self.sample_rate
            || frame.channels != self.channels
            || frame.samples.is_empty()
            || !frame.samples.len().is_multiple_of(channels)
            || frame.samples.len() > required * 6
        {
            self.pending.clear();
            self.expected_input = None;
            return Vec::new();
        }
        if self.expected_input != Some(frame.timestamp) {
            self.pending.clear();
        }
        if self.pending.is_empty() {
            self.timestamp = frame.timestamp;
        }
        self.expected_input = Some(
            frame
                .timestamp
                .wrapping_add((frame.samples.len() / channels) as u32),
        );
        self.pending.extend_from_slice(&frame.samples);
        let mut result = Vec::new();
        while self.pending.len() >= required {
            let rest = self.pending.split_off(required);
            let samples = std::mem::replace(&mut self.pending, rest);
            result.push(AudioFrame::new(
                samples,
                self.sample_rate,
                self.channels,
                self.timestamp,
            ));
            self.timestamp = self.timestamp.wrapping_add((required / channels) as u32);
        }
        result
    }
}

async fn run_inbound_pump(
    mut subscriber: crate::types::AudioFrameSubscriber,
    mut runtime: SipMediaCodecRuntime,
    mut codec_updates: watch::Receiver<Option<SipMediaCodecRuntime>>,
    stream_id: StreamId,
    frames_in_tx: mpsc::Sender<MediaFrame>,
    playout: Option<PlayoutConfig>,
) -> &'static str {
    // Without a playout buffer this pump forwards whatever arrives, in
    // arrival order, with a gap wherever a packet was lost — the behaviour
    // every release before this had. With one, frames are reordered onto the
    // media clock and losses are concealed rather than heard as clicks.
    let playout_policy = playout;
    let mut playout = playout_policy.map(PlayoutBuffer::new);
    let mut framer = InboundPcmFramer::new(&runtime.negotiated);
    let mut encoder =
        match SipPayloadCodec::from_negotiated(&runtime.negotiated, runtime.payload_type) {
            Ok(codec) => codec,
            Err(_) => return "sip-codec-reconfigure-failed",
        };
    let mut reported = std::time::Instant::now();

    loop {
        let event = tokio::select! {
            biased;
            changed = codec_updates.changed() => {
                if changed.is_err() {
                    return "sip-codec-update-channel-closed";
                }
                InboundPumpEvent::CodecChanged
            }
            frame = subscriber.receiver.recv() => InboundPumpEvent::Audio(frame),
            () = wait_for_playout(playout.as_ref().and_then(PlayoutBuffer::next_deadline)) => {
                InboundPumpEvent::PlayoutTick
            }
        };

        let mut ready = Vec::new();
        match event {
            InboundPumpEvent::CodecChanged => {
                let Some(updated) = codec_updates.borrow_and_update().clone() else {
                    continue;
                };
                encoder = match SipPayloadCodec::from_negotiated(
                    &updated.negotiated,
                    updated.payload_type,
                ) {
                    Ok(codec) => codec,
                    Err(_) => return "sip-codec-reconfigure-failed",
                };
                runtime = updated;
                playout = playout_policy.map(PlayoutBuffer::new);
                framer = InboundPcmFramer::new(&runtime.negotiated);
                continue;
            }
            InboundPumpEvent::Audio(Some(audio_frame)) => match playout.as_mut() {
                Some(buffer) => buffer.push(audio_frame, std::time::Instant::now()),
                None => ready.push(audio_frame),
            },
            InboundPumpEvent::Audio(None) => return "sip-audio-source-closed",
            InboundPumpEvent::PlayoutTick => {}
        }

        if let Some(buffer) = playout.as_mut() {
            // One timer tick emits one ordinary frame. A bounded number of
            // additional frames may be released only when the queue is above
            // target or the task woke late, which reconverges latency without
            // letting a network arrival run the media clock.
            while let Some(frame) = buffer.pop_due(std::time::Instant::now()) {
                ready.push(frame);
                if ready.len() >= 8 {
                    break;
                }
            }
        }

        if let Some(buffer) = playout.as_ref() {
            // Periodic, not per frame: this is a hot path and the numbers
            // are only useful as a trend.
            if reported.elapsed() >= std::time::Duration::from_secs(10) {
                reported = std::time::Instant::now();
                let stats = buffer.stats();
                if stats.frames_concealed > 0 || stats.frames_late > 0 || stats.frames_catch_up > 0
                {
                    tracing::debug!(
                        target: "rvoip_sip",
                        emitted = stats.frames_emitted,
                        concealed = stats.frames_concealed,
                        late = stats.frames_late,
                        catch_up = stats.frames_catch_up,
                        depth = stats.depth,
                        "SipMediaStream playout quality"
                    );
                }
            }
        }

        for frame in ready.into_iter().flat_map(|frame| framer.push(frame)) {
            let encoded_frames = match encoder.encode_graph_frames(&frame) {
                Ok(frames) => frames,
                Err(error) => {
                    tracing::trace!(target: "rvoip_sip", error = %error, "SipMediaStream: audio encode failed");
                    continue;
                }
            };
            for (encoded, timestamp_rtp) in encoded_frames {
                let media_frame = MediaFrame {
                    stream_id: stream_id.clone(),
                    kind: StreamKind::Audio,
                    payload: Bytes::from(encoded),
                    timestamp_rtp,
                    captured_at: Utc::now(),
                    payload_type: Some(runtime.payload_type),
                };
                if frames_in_tx.send(media_frame).await.is_err() {
                    return "inbound-consumer-closed";
                }
            }
        }
    }
}

async fn run_outbound_pump(
    coordinator: Arc<UnifiedCoordinator>,
    session_id: SessionId,
    mut runtime: SipMediaCodecRuntime,
    mut codec_updates: watch::Receiver<Option<SipMediaCodecRuntime>>,
    mut frames_out_rx: mpsc::Receiver<MediaFrame>,
    mut peer_out_rx: mpsc::Receiver<PeerMediaFrame>,
) -> &'static str {
    let mut decoder =
        match SipPayloadCodec::from_negotiated(&runtime.negotiated, runtime.payload_type) {
            Ok(codec) => codec,
            Err(_) => return "sip-codec-reconfigure-failed",
        };
    let mut channels = runtime.negotiated.channels.max(1);
    let mut outbound_clock = OutboundRtpClock::default();
    let mut telephone_events = TelephoneEventTracker::default();
    loop {
        let (media_frame, mut delivery_guard) = tokio::select! {
            changed = codec_updates.changed() => {
                if changed.is_err() {
                    return "sip-codec-update-channel-closed";
                }
                let Some(updated) = codec_updates.borrow_and_update().clone() else {
                    continue;
                };
                decoder = match SipPayloadCodec::from_negotiated(
                    &updated.negotiated,
                    updated.payload_type,
                ) {
                    Ok(codec) => codec,
                    Err(_) => return "sip-codec-reconfigure-failed",
                };
                channels = updated.negotiated.channels.max(1);
                runtime = updated;
                continue;
            }
            frame = frames_out_rx.recv() => match frame {
                Some(frame) => (frame, None),
                None => return "outbound-producer-closed",
            },
            frame = peer_out_rx.recv() => match frame {
                Some(frame) => match frame.into_delivery() {
                    Some((frame, guard)) => (frame, Some(guard)),
                    None => continue,
                },
                None => return "peer-outbound-producer-closed",
            }
        };
        const TELEPHONE_EVENT_PT: u8 = 101;
        if media_frame.payload_type == Some(TELEPHONE_EVENT_PT) {
            if let Some(digit) = telephone_events.digit(
                media_frame.stream_id.as_str(),
                media_frame.timestamp_rtp,
                &media_frame.payload,
            ) {
                // Await the complete tone schedule. The single-digit API
                // returns after spawning a background sender, which lets tone
                // packets escape a later output-pump cutoff acknowledgment.
                let mut encoded = [0u8; 4];
                let digits = digit.encode_utf8(&mut encoded);
                if let Some(guard) = delivery_guard.take() {
                    // The sequence itself spawns tone work. Retain the fence
                    // in an owner that survives cancellation of this pump.
                    let coordinator = Arc::clone(&coordinator);
                    let session_id = session_id.clone();
                    let digits = digits.to_owned();
                    let send = tokio::spawn(async move {
                        let _guard = guard;
                        coordinator
                            .send_dtmf_sequence(&session_id, &digits, 100, 0)
                            .await
                    });
                    if !matches!(send.await, Ok(Ok(()))) {
                        return "sip-dtmf-send-failed";
                    }
                } else if coordinator
                    .send_dtmf_sequence(&session_id, digits, 100, 0)
                    .await
                    .is_err()
                {
                    return "sip-dtmf-send-failed";
                }
            }
            continue;
        }
        if media_frame
            .payload_type
            .is_some_and(|actual| actual != runtime.payload_type)
        {
            tracing::trace!(
                target: "rvoip_sip",
                actual = ?media_frame.payload_type,
                expected = runtime.payload_type,
                "SipMediaStream: dropping unnegotiated payload type"
            );
            continue;
        }
        let mut audio_frame = match decoder.decode(&media_frame.payload) {
            Ok(frame) => frame,
            Err(error) => {
                tracing::trace!(
                    target: "rvoip_sip",
                    error = %error,
                    bytes = media_frame.payload.len(),
                    "SipMediaStream: audio decode failed; dropping frame"
                );
                continue;
            }
        };
        let samples_emitted = audio_frame.samples.len() / usize::from(channels.max(1));
        audio_frame.timestamp = advance_outbound_timestamp(
            &mut outbound_clock,
            samples_emitted,
            media_frame.timestamp_rtp,
        );
        if coordinator
            .send_audio(&session_id, audio_frame)
            .await
            .is_err()
        {
            return "sip-audio-send-failed";
        }
    }
}

async fn wait_for_media_cancel(cancel: &mut watch::Receiver<bool>) {
    loop {
        if *cancel.borrow_and_update() {
            return;
        }
        if cancel.changed().await.is_err() {
            return;
        }
    }
}

#[async_trait]
impl MediaStream for SipMediaStream {
    fn id(&self) -> StreamId {
        self.inner.stream_id.clone()
    }

    fn kind(&self) -> StreamKind {
        StreamKind::Audio
    }

    fn codec(&self) -> CodecInfo {
        self.inner
            .codec
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn direction(&self) -> Direction {
        self.inner.direction
    }

    fn source_ready(&self) -> bool {
        self.inner.lifecycle.current() == SipMediaLifecycle::Bound
    }

    fn frames_in(&self) -> mpsc::Receiver<MediaFrame> {
        self.try_frames_in().unwrap_or_else(|_| mpsc::channel(1).1)
    }

    fn try_frames_in(&self) -> RvoipResult<mpsc::Receiver<MediaFrame>> {
        Ok(self.reserve_frames_in()?.commit())
    }

    fn reserve_frames_in(&self) -> RvoipResult<MediaReceiverReservation> {
        let receiver = self
            .inner
            .frames_in_rx
            .lock()
            .map_err(|_| RvoipError::InvalidState("SIP media receiver lock is poisoned"))?
            .take()
            .ok_or(RvoipError::InvalidState(
                "SIP media receiver has already been acquired",
            ))?;
        let inner = Arc::clone(&self.inner);
        Ok(MediaReceiverReservation::new(receiver, move |receiver| {
            let mut slot = inner
                .frames_in_rx
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            debug_assert!(slot.is_none(), "reserved SIP receiver slot was replaced");
            if slot.is_none() {
                *slot = Some(receiver);
            }
        }))
    }

    fn frames_out(&self) -> mpsc::Sender<MediaFrame> {
        self.try_frames_out().unwrap_or_else(|_| mpsc::channel(1).0)
    }

    fn try_frames_out(&self) -> RvoipResult<mpsc::Sender<MediaFrame>> {
        match self.inner.lifecycle.current() {
            SipMediaLifecycle::Bound
                if self.inner.outbound_writes_activated.load(Ordering::Acquire) =>
            {
                Ok(self.inner.frames_out_tx.clone())
            }
            SipMediaLifecycle::Bound => Err(RvoipError::InvalidState(
                "SIP media stream is not activated",
            )),
            SipMediaLifecycle::Dormant | SipMediaLifecycle::Binding => Err(
                RvoipError::InvalidState("SIP media stream is not activated"),
            ),
            SipMediaLifecycle::Failed | SipMediaLifecycle::Closing | SipMediaLifecycle::Closed => {
                Err(RvoipError::InvalidState("SIP media stream is not writable"))
            }
        }
    }

    fn try_peer_frames_out(&self) -> RvoipResult<mpsc::Sender<PeerMediaFrame>> {
        self.try_frames_out()?; // Same activation/lifecycle admission as legacy writes.
        Ok(self.inner.peer_out_tx.clone())
    }

    fn quality_snapshot(&self) -> QualitySnapshot {
        // The last report the media layer pushed for this connection.
        //
        // Quality originates as RTCP receiver reports and XR, which
        // media-core distills and the adapter routes here per connection.
        // Before the first report there is no measurement, and the trait
        // has no way to say so — `default()` is zeros, which reads as
        // flawless. `has_quality_measurement` distinguishes the two, and a
        // poller should consult it rather than averaging in a call that has
        // not reported yet.
        self.inner
            .last_quality
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .unwrap_or_default()
    }

    /// False until the media layer reports quality for this connection, so
    /// an aggregator does not average in a call it has never measured.
    fn has_quality_measurement(&self) -> bool {
        self.inner
            .last_quality
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
    }

    async fn close(self: Arc<Self>) -> RvoipResult<()> {
        self.close_retained().await
    }
}

#[cfg(test)]
mod media_owner_wait_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[tokio::test]
    async fn inbound_publication_waits_for_delayed_media_owner_commit() {
        let owner_ready = Arc::new(AtomicBool::new(false));
        let delayed_owner = Arc::clone(&owner_ready);
        let commit = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            delayed_owner.store(true, Ordering::Release);
        });
        let (cancel_tx, mut cancel_rx) = watch::channel(false);

        let result = wait_for_media_owner(
            move || {
                let ready = owner_ready.load(Ordering::Acquire);
                std::future::ready(if ready {
                    MediaOwnerProbe::Ready
                } else {
                    MediaOwnerProbe::Pending
                })
            },
            &cancel_tx,
            &mut cancel_rx,
            tokio::time::Instant::now() + std::time::Duration::from_secs(1),
        )
        .await;

        commit.await.expect("delayed media-owner commit task");
        assert_eq!(result, MediaOwnerWaitResult::Ready);
    }

    #[tokio::test]
    async fn terminal_and_missing_sessions_fail_without_waiting_for_setup_deadline() {
        for (probe, expected) in [
            (MediaOwnerProbe::Terminal, MediaOwnerWaitResult::Terminal),
            (MediaOwnerProbe::Missing, MediaOwnerWaitResult::Missing),
        ] {
            let (cancel_tx, mut cancel_rx) = watch::channel(false);
            let result = tokio::time::timeout(
                std::time::Duration::from_millis(100),
                wait_for_media_owner(
                    || std::future::ready(probe),
                    &cancel_tx,
                    &mut cancel_rx,
                    tokio::time::Instant::now() + std::time::Duration::from_secs(5),
                ),
            )
            .await
            .expect("terminal ownership probe returned promptly");
            assert_eq!(result, expected);
        }
    }

    #[tokio::test]
    async fn pending_media_owner_wait_observes_route_cancellation() {
        let (cancel_tx, mut cancel_rx) = watch::channel(false);
        let cancelling = cancel_tx.clone();
        let cancel = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            cancelling.send_replace(true);
        });

        let result = wait_for_media_owner(
            || std::future::ready(MediaOwnerProbe::Pending),
            &cancel_tx,
            &mut cancel_rx,
            tokio::time::Instant::now() + std::time::Duration::from_secs(1),
        )
        .await;

        cancel.await.expect("media-owner cancellation task");
        assert_eq!(result, MediaOwnerWaitResult::Cancelled);
    }
}

/// Parse a non-terminal RFC 4733 telephone event. The initial duration may
/// already be nonzero; timestamp-based event tracking suppresses repetitions.
/// Payload layout (§2.3 of RFC 4733):
/// ```text
///  0                   1                   2                   3
///  0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |     event     |E|R| volume    |          duration             |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// ```
fn parse_rfc4733_digit(payload: &[u8]) -> Option<char> {
    if payload.len() < 4 {
        return None;
    }
    let event = payload[0];
    if payload[1] & 0x80 != 0 {
        // End-only arrivals must not start another synthesized tone.
        return None;
    }
    // Event codes 0–9 → '0'..'9', 10 → '*', 11 → '#', 12–15 → 'A'..'D'.
    match event {
        0..=9 => Some((b'0' + event) as char),
        10 => Some('*'),
        11 => Some('#'),
        12 => Some('A'),
        13 => Some('B'),
        14 => Some('C'),
        15 => Some('D'),
        _ => None,
    }
}

#[derive(Default)]
struct TelephoneEventTracker {
    source: Option<String>,
    last_timestamp: Option<u32>,
}

impl TelephoneEventTracker {
    fn digit(&mut self, source: &str, timestamp: u32, payload: &[u8]) -> Option<char> {
        let digit = parse_rfc4733_digit(payload)?;
        if self.source.as_deref() != Some(source) {
            self.source = Some(source.to_owned());
            self.last_timestamp = None;
        }
        if self
            .last_timestamp
            .is_some_and(|last| timestamp.wrapping_sub(last) as i32 <= 0)
        {
            return None;
        }
        self.last_timestamp = Some(timestamp);
        Some(digit)
    }
}

#[cfg(test)]
mod negotiated_codec_tests {
    use super::*;
    use std::net::SocketAddr;

    fn negotiated(
        codec: &str,
        sample_rate: u32,
        channels: u8,
    ) -> crate::session_store::state::NegotiatedConfig {
        crate::session_store::state::NegotiatedConfig {
            local_addr: SocketAddr::from(([127, 0, 0, 1], 10_000)),
            remote_addr: SocketAddr::from(([127, 0, 0, 1], 20_000)),
            codec: codec.to_string(),
            sample_rate,
            channels,
            fmtp: None,
        }
    }

    /// The negotiated fmtp reaches `CodecInfo` rather than being dropped.
    ///
    /// It was hard-coded to `None` here, which is the second of two
    /// independent drop points on the same parameter. `rvoip-core` keys its
    /// transcoding codec groups on this field, so every SIP leg landed in the
    /// one `fmtp: None` group and whatever the peer negotiated — Opus's
    /// `maxaveragebitrate`, and AMR's framing the moment AMR reaches here —
    /// was silently discarded.
    #[test]
    fn the_negotiated_fmtp_reaches_the_codec_descriptor() {
        let mut config = negotiated("PCMU", 8_000, 1);
        config.fmtp = Some("annexb=no".to_string());
        let (codec, _) = codec_descriptor(&config, 0).unwrap();
        assert_eq!(codec.fmtp.as_deref(), Some("annexb=no"));

        // Absent stays absent rather than becoming an empty string, because
        // the two are different keys downstream.
        let (plain, _) = codec_descriptor(&negotiated("PCMU", 8_000, 1), 0).unwrap();
        assert_eq!(plain.fmtp, None);

        // And two legs differing only in fmtp are different descriptors --
        // the property the transcoding grouping depends on.
        assert_ne!(codec, plain);
    }

    #[test]
    fn descriptor_uses_exact_negotiated_g711_variant() {
        let (pcmu, pcmu_pt) = codec_descriptor(&negotiated("PCMU", 8_000, 1), 0).unwrap();
        let (pcma, pcma_pt) = codec_descriptor(&negotiated("PCMA", 8_000, 1), 8).unwrap();

        assert_eq!(pcmu.name, "g.711-mu");
        assert_eq!(pcmu_pt, 0);
        assert_eq!(pcma.name, "g.711-a");
        assert_eq!(pcma_pt, 8);
        assert_ne!(pcmu, pcma);
    }

    #[tokio::test]
    async fn inbound_pump_applies_a_codec_generation_before_the_next_frame() {
        let session_id = SessionId::new();
        let (audio_tx, audio_rx) = mpsc::channel(4);
        let subscriber = crate::types::AudioFrameSubscriber::new(session_id, audio_rx);
        let (frames_tx, mut frames_rx) = mpsc::channel(4);
        let pcmu = SipMediaCodecRuntime {
            negotiated: negotiated("PCMU", 8_000, 1),
            payload_type: 0,
        };
        let pcma = SipMediaCodecRuntime {
            negotiated: negotiated("PCMA", 8_000, 1),
            payload_type: 8,
        };
        let (codec_tx, codec_rx) = watch::channel(Some(pcmu.clone()));
        let pump = tokio::spawn(run_inbound_pump(
            subscriber,
            pcmu,
            codec_rx,
            StreamId::new(),
            frames_tx,
            None,
        ));

        audio_tx
            .send(AudioFrame::new(vec![1_000; 160], 8_000, 1, 0))
            .await
            .expect("PCMU source frame");
        let first = frames_rx.recv().await.expect("PCMU graph frame");
        assert_eq!(first.payload_type, Some(0));

        codec_tx.send_replace(Some(pcma));
        audio_tx
            .send(AudioFrame::new(vec![1_000; 160], 8_000, 1, 160))
            .await
            .expect("PCMA source frame");
        let second = frames_rx.recv().await.expect("PCMA graph frame");
        assert_eq!(second.payload_type, Some(8));
        assert_ne!(first.payload, second.payload);

        drop(audio_tx);
        assert_eq!(pump.await.expect("pump task"), "sip-audio-source-closed");
    }

    #[tokio::test]
    async fn inbound_pump_uses_a_clocked_playout_deadline_and_plc() {
        let session_id = SessionId::new();
        let (audio_tx, audio_rx) = mpsc::channel(4);
        let subscriber = crate::types::AudioFrameSubscriber::new(session_id, audio_rx);
        let (frames_tx, mut frames_rx) = mpsc::channel(4);
        let pcmu = SipMediaCodecRuntime {
            negotiated: negotiated("PCMU", 8_000, 1),
            payload_type: 0,
        };
        let (_codec_tx, codec_rx) = watch::channel(Some(pcmu.clone()));
        let pump = tokio::spawn(run_inbound_pump(
            subscriber,
            pcmu,
            codec_rx,
            StreamId::new(),
            frames_tx,
            Some(PlayoutConfig {
                adaptive: false,
                ..PlayoutConfig::default()
            }),
        ));

        audio_tx
            .send(AudioFrame::new(vec![1_000; 160], 8_000, 1, 0))
            .await
            .expect("first source frame");
        audio_tx
            .send(AudioFrame::new(vec![1_000; 160], 8_000, 1, 320))
            .await
            .expect("post-loss source frame");

        let first = tokio::time::timeout(std::time::Duration::from_millis(100), frames_rx.recv())
            .await
            .expect("first playout deadline")
            .expect("first graph frame");
        assert_eq!(first.timestamp_rtp, 0);

        let concealed =
            tokio::time::timeout(std::time::Duration::from_millis(100), frames_rx.recv())
                .await
                .expect("loss playout deadline")
                .expect("concealed graph frame");
        assert_eq!(concealed.timestamp_rtp, 160);

        let resumed = tokio::time::timeout(std::time::Duration::from_millis(100), frames_rx.recv())
            .await
            .expect("resume playout deadline")
            .expect("resumed graph frame");
        assert_eq!(resumed.timestamp_rtp, 320);

        drop(audio_tx);
        assert_eq!(pump.await.expect("pump task"), "sip-audio-source-closed");
    }

    #[test]
    fn inbound_pcm_framing_preserves_samples_and_wrapping_channel_timestamps() {
        for channels in [1, 2] {
            for samples_per_channel in [120, 240, 480, 960, 1920, 2880, 5760] {
                let mut framer = InboundPcmFramer::new(&negotiated("opus", 48_000, channels));
                let start = u32::MAX - 300;
                let mut input = Vec::new();
                let mut output = Vec::new();
                let count = if samples_per_channel < 960 {
                    960 / samples_per_channel
                } else {
                    1
                };
                for packet in 0..count {
                    let samples = (0..samples_per_channel * usize::from(channels))
                        .map(|i| (i + packet * 31) as i16)
                        .collect::<Vec<_>>();
                    input.extend_from_slice(&samples);
                    output.extend(framer.push(AudioFrame::new(
                        samples,
                        48_000,
                        channels,
                        start.wrapping_add((packet * samples_per_channel) as u32),
                    )));
                }
                assert_eq!(output.len(), count * samples_per_channel / 960);
                let actual = output
                    .iter()
                    .flat_map(|frame| frame.samples.iter().copied())
                    .collect::<Vec<_>>();
                assert_eq!(actual, input);
                for (index, frame) in output.iter().enumerate() {
                    assert_eq!(frame.timestamp, start.wrapping_add(index as u32 * 960));
                    assert_eq!(frame.samples.len(), 960 * usize::from(channels));
                }
                assert!(framer.pending.is_empty());
            }
        }
    }

    #[test]
    fn inbound_pcm_framing_drops_partial_audio_across_gaps_and_invalid_shapes() {
        let mut framer = InboundPcmFramer::new(&negotiated("opus", 48_000, 2));
        assert!(framer
            .push(AudioFrame::new(vec![1; 960], 48_000, 2, 0))
            .is_empty());
        assert!(framer
            .push(AudioFrame::new(vec![2; 960], 48_000, 2, 48_000))
            .is_empty());
        let emitted = framer.push(AudioFrame::new(vec![3; 960], 48_000, 2, 48_480));
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].timestamp, 48_000);
        assert_eq!(&emitted[0].samples[..960], &[2; 960]);
        assert_eq!(&emitted[0].samples[960..], &[3; 960]);
        for (samples, rate, channels) in [
            (vec![1; 11], 48_000, 2),
            (vec![1; 960], 16_000, 2),
            (vec![1; 960], 48_000, 1),
            (vec![1; 1920 * 7], 48_000, 2),
            (Vec::new(), 48_000, 2),
        ] {
            assert!(framer
                .push(AudioFrame::new(vec![1; 960], 48_000, 2, 0))
                .is_empty());
            assert!(framer
                .push(AudioFrame::new(samples, rate, channels, 480))
                .is_empty());
            assert!(framer.pending.is_empty());
            assert_eq!(framer.expected_input, None);
        }
    }

    #[cfg(feature = "opus")]
    #[tokio::test]
    async fn inbound_pump_encodes_variable_opus_packets_as_audible_twenty_ms_frames() {
        for (packet_samples, clocked) in [120, 240, 480, 960, 1920, 2880, 5760]
            .into_iter()
            .flat_map(|samples| [(samples, false), (samples, true)])
        {
            let config = negotiated("opus", 48_000, 2);
            let runtime = SipMediaCodecRuntime {
                negotiated: config.clone(),
                payload_type: 102,
            };
            let (_codec_tx, codec_rx) = watch::channel(Some(runtime.clone()));
            let (audio_tx, audio_rx) = mpsc::channel(16);
            let (frames_tx, mut frames_rx) = mpsc::channel(16);
            let pump = tokio::spawn(run_inbound_pump(
                crate::types::AudioFrameSubscriber::new(SessionId::new(), audio_rx),
                runtime,
                codec_rx,
                StreamId::new(),
                frames_tx,
                clocked.then_some(PlayoutConfig {
                    adaptive: false,
                    ..PlayoutConfig::default()
                }),
            ));
            // Clocked playout needs at least two input packets to prime.
            let count = (960 / packet_samples).max(1) * 2;
            for index in 0..count {
                let samples = (0..packet_samples)
                    .flat_map(|i| {
                        let value = ((i + index * packet_samples) as f32 * 0.0576).sin() * 10_000.0;
                        [value as i16; 2]
                    })
                    .collect();
                audio_tx
                    .send(AudioFrame::new(
                        samples,
                        48_000,
                        2,
                        (index * packet_samples) as u32,
                    ))
                    .await
                    .unwrap();
            }
            let mut decoder = SipPayloadCodec::from_negotiated(&config, 102).unwrap();
            for index in 0..(count * packet_samples / 960) {
                let frame =
                    tokio::time::timeout(std::time::Duration::from_secs(2), frames_rx.recv())
                        .await
                        .unwrap_or_else(|error| {
                            panic!(
                        "packet_samples={packet_samples} clocked={clocked} index={index}: {error}"
                    )
                        })
                        .unwrap();
                assert_eq!(frame.timestamp_rtp, index as u32 * 960);
                assert_eq!(frame.payload_type, Some(102));
                let decoded = decoder.decode(&frame.payload).unwrap();
                assert_eq!(decoded.samples.len(), 1920);
                let energy: f64 = decoded.samples.iter().map(|v| f64::from(*v).powi(2)).sum();
                assert!(energy / decoded.samples.len() as f64 > 100_000.0);
            }
            drop(audio_tx);
            assert_eq!(pump.await.unwrap(), "sip-audio-source-closed");
            assert!(frames_rx.recv().await.is_none());
        }
    }

    #[test]
    fn pcmu_and_pcma_encode_with_different_wire_laws() {
        let frame = rvoip_media_core::types::AudioFrame::new(vec![0; 160], 8_000, 1, 0);
        let mut pcmu = SipPayloadCodec::from_negotiated(&negotiated("PCMU", 8_000, 1), 0).unwrap();
        let mut pcma = SipPayloadCodec::from_negotiated(&negotiated("PCMA", 8_000, 1), 8).unwrap();

        let pcmu_payload = pcmu.encode(&frame).unwrap();
        let pcma_payload = pcma.encode(&frame).unwrap();
        assert_eq!(pcmu_payload.len(), 160);
        assert_eq!(pcma_payload.len(), 160);
        assert_ne!(pcmu_payload, pcma_payload);
    }

    #[cfg(feature = "opus")]
    #[test]
    fn opus_descriptor_and_codec_follow_sdp_clock_and_channels() {
        let config = negotiated("opus", 48_000, 2);
        let (descriptor, payload_type) = codec_descriptor(&config, 96).unwrap();
        assert_eq!(descriptor.name, "opus");
        assert_eq!(descriptor.clock_rate_hz, 48_000);
        assert_eq!(descriptor.channels, 2);
        assert_eq!(payload_type, 96);
        assert!(matches!(
            SipPayloadCodec::from_negotiated(&config, 96),
            Ok(SipPayloadCodec::Opus(_))
        ));

        let mut encoder = SipPayloadCodec::from_negotiated(&config, 96).unwrap();
        let mut decoder = SipPayloadCodec::from_negotiated(&config, 96).unwrap();
        let frame = rvoip_media_core::types::AudioFrame::new(vec![0; 960 * 2], 48_000, 2, 960);
        let payload = encoder.encode(&frame).unwrap();
        let decoded = decoder.decode(&payload).unwrap();
        assert_eq!(decoded.sample_rate, 48_000);
        assert_eq!(decoded.channels, 2);
        assert_eq!(decoded.samples.len(), 960 * 2);
    }

    #[cfg(feature = "g729")]
    #[test]
    fn g729_descriptor_and_codec_preserve_annex_and_twenty_ms_packetization() {
        let mut config = negotiated("G729A", 8_000, 1);
        config.fmtp = Some("annexb=no".to_string());
        let (descriptor, payload_type) = codec_descriptor(&config, 18).unwrap();
        assert_eq!(descriptor.name, "g729");
        assert_eq!(descriptor.clock_rate_hz, 8_000);
        assert_eq!(descriptor.channels, 1);
        assert_eq!(descriptor.fmtp.as_deref(), Some("annexb=no"));
        assert_eq!(descriptor.payload_type, Some(18));
        assert_eq!(payload_type, 18);
        assert!(!negotiated_g729_annex_b(&config));

        let mut encoder = SipPayloadCodec::from_negotiated(&config, 18).unwrap();
        let mut decoder = SipPayloadCodec::from_negotiated(&config, 18).unwrap();
        let samples: Vec<i16> = (0..160)
            .map(|index| {
                let phase = f64::from(index) * 2.0 * std::f64::consts::PI * 440.0 / 8_000.0;
                (phase.sin() * 6_000.0) as i16
            })
            .collect();
        let payloads = encoder
            .encode_graph_frames(&rvoip_media_core::types::AudioFrame::new(
                samples, 8_000, 1, 1_000,
            ))
            .unwrap();
        assert_eq!(payloads.len(), 2, "20 ms becomes two G.729 graph frames");
        assert_eq!(payloads[0].0.len(), 10);
        assert_eq!(payloads[0].1, 1_000);
        assert_eq!(payloads[1].0.len(), 10);
        assert_eq!(payloads[1].1, 1_080);
        for (payload, _) in payloads {
            let decoded = decoder.decode(&payload).unwrap();
            assert_eq!(decoded.sample_rate, 8_000);
            assert_eq!(decoded.channels, 1);
            assert_eq!(decoded.samples.len(), 80);
        }

        let mut annex_b = negotiated("G729BA", 8_000, 1);
        annex_b.fmtp = Some("annexb=yes".to_string());
        assert!(negotiated_g729_annex_b(&annex_b));
        assert!(matches!(
            SipPayloadCodec::from_negotiated(&annex_b, 18),
            Ok(SipPayloadCodec::G729(_))
        ));
    }

    #[cfg(feature = "g729")]
    #[test]
    fn g729_descriptor_materializes_the_effective_annex_b_policy() {
        for (name, expected) in [
            ("G729A", "annexb=no"),
            ("G729BA", "annexb=yes"),
            ("G729", "annexb=yes"),
        ] {
            let (descriptor, _) = codec_descriptor(&negotiated(name, 8_000, 1), 18).unwrap();
            assert_eq!(descriptor.fmtp.as_deref(), Some(expected), "{name}");
        }

        let mut with_unrelated = negotiated("G729", 8_000, 1);
        with_unrelated.fmtp = Some("foo=bar; ANNEXB=no; mode=x".to_string());
        let (descriptor, _) = codec_descriptor(&with_unrelated, 18).unwrap();
        assert_eq!(descriptor.fmtp.as_deref(), Some("foo=bar;mode=x;annexb=no"));
    }

    #[cfg(feature = "g729")]
    fn g729_test_codec(annex_b: bool) -> G729Codec {
        G729Codec::new(
            SampleRate::Rate8000,
            1,
            G729Config {
                annexes: G729Annexes {
                    annex_a: true,
                    annex_b,
                },
                frame_size_ms: 10.0,
                enable_vad: annex_b,
                enable_cng: annex_b,
            },
        )
        .expect("G.729 test codec")
    }

    #[cfg(feature = "g729")]
    fn g729_speech_frames(count: usize) -> Vec<u8> {
        let mut encoder = g729_test_codec(false);
        let mut payload = Vec::with_capacity(count * 10);
        for frame_index in 0..count {
            let samples = (0..80)
                .map(|sample_index| {
                    let index = frame_index * 80 + sample_index;
                    let phase = index as f64 * 2.0 * std::f64::consts::PI * 440.0 / 8_000.0;
                    (phase.sin() * 6_000.0) as i16
                })
                .collect();
            let encoded = encoder
                .encode(&rvoip_media_core::types::AudioFrame::new(
                    samples, 8_000, 1, 0,
                ))
                .expect("G.729 speech encode");
            assert_eq!(encoded.len(), 10, "Annex-A encoder must emit speech");
            payload.extend(encoded);
        }
        payload
    }

    #[cfg(feature = "g729")]
    #[test]
    fn g729_decoder_accepts_speech_frames_followed_by_annex_b_sid() {
        for (speech_frames, expected_samples) in [(1, 160), (2, 240)] {
            let mut payload = g729_speech_frames(speech_frames);
            payload.extend([0, 0]);
            assert_eq!(payload.len(), speech_frames * 10 + 2);

            let decoded = g729_test_codec(true)
                .decode_rtp_payload(&payload)
                .expect("valid G.729 Annex-B RTP payload");
            assert_eq!(decoded.sample_rate, 8_000);
            assert_eq!(decoded.channels, 1);
            assert_eq!(decoded.timestamp, 0);
            assert_eq!(decoded.samples.len(), expected_samples);
        }
    }

    #[cfg(feature = "g729")]
    #[test]
    fn g729_decoder_delegates_malformed_rtp_payload_lengths_to_codec_validation() {
        for payload_len in [1, 3, 4, 8, 11, 13, 21, 23] {
            let error = g729_test_codec(true)
                .decode_rtp_payload(&vec![0; payload_len])
                .expect_err("malformed G.729 RTP payload must be rejected");
            assert!(
                error
                    .to_string()
                    .contains("invalid G.729 RTP payload length"),
                "unexpected error for {payload_len}-byte payload: {error}"
            );
        }
    }

    /// A carrier may answer AMR-WB in either RFC 4867 framing. The
    /// descriptor follows the SDP clock and carries the fmtp that selects the
    /// framing, and the codec builds for both: bandwidth-efficient when no
    /// fmtp is given, octet-aligned when `octet-align=1` is. A stream shape
    /// other than 16 kHz mono is refused before a codec is built. Ported from
    /// Thelve's vendored SIP crate.
    #[cfg(feature = "amr-wb")]
    #[test]
    fn amr_wb_descriptor_and_codec_follow_sdp_clock_and_fmtp_in_both_framings() {
        for (payload_type, fmtp) in [(104u8, None), (105u8, Some("octet-align=1"))] {
            let mut config = negotiated("AMR-WB", 16_000, 1);
            config.fmtp = fmtp.map(str::to_string);
            let (descriptor, resolved) = codec_descriptor(&config, payload_type)
                .unwrap_or_else(|error| panic!("descriptor for pt {payload_type}: {error}"));
            assert_eq!(descriptor.name, "AMR-WB");
            assert_eq!(descriptor.clock_rate_hz, 16_000);
            assert_eq!(descriptor.channels, 1);
            assert_eq!(descriptor.fmtp.as_deref(), fmtp);
            assert_eq!(resolved, payload_type);
            assert!(
                SipPayloadCodec::from_negotiated(&config, payload_type).is_ok(),
                "AMR-WB codec must build for pt {payload_type} with fmtp {fmtp:?}"
            );
        }
        let wrong_shape = negotiated("AMR-WB", 8_000, 1);
        assert_eq!(
            codec_descriptor(&wrong_shape, 104),
            Err("invalid-amr-wb-shape")
        );
    }

    #[cfg(feature = "amr-wb")]
    #[test]
    fn amr_wb_descriptor_and_codec_use_negotiated_payload_and_framing() {
        let mut config = negotiated("AMR-WB", 16_000, 1);
        config.fmtp = Some("octet-align=1; mode-set=2".to_string());
        let (descriptor, payload_type) = codec_descriptor(&config, 105).unwrap();
        assert_eq!(descriptor.name, "AMR-WB");
        assert_eq!(descriptor.clock_rate_hz, 16_000);
        assert_eq!(descriptor.channels, 1);
        assert_eq!(
            descriptor.fmtp.as_deref(),
            Some("octet-align=1; mode-set=2")
        );
        assert_eq!(descriptor.payload_type, Some(105));
        assert_eq!(payload_type, 105);

        let mut encoder = SipPayloadCodec::from_negotiated(&config, 105).unwrap();
        let mut decoder = SipPayloadCodec::from_negotiated(&config, 105).unwrap();
        let samples: Vec<i16> = (0..640)
            .map(|index| {
                let phase = f64::from(index) * 2.0 * std::f64::consts::PI * 440.0 / 16_000.0;
                (phase.sin() * 6_000.0) as i16
            })
            .collect();
        let payloads = encoder
            .encode_graph_frames(&rvoip_media_core::types::AudioFrame::new(
                samples, 16_000, 1, 4_000,
            ))
            .unwrap();
        assert_eq!(
            payloads.len(),
            2,
            "bundled PCM becomes independent AMR payloads"
        );
        assert_eq!(payloads[0].1, 4_000);
        assert_eq!(payloads[1].1, 4_320);
        assert!(!payloads[0].0.is_empty());
        for (payload, _) in payloads {
            let decoded = decoder.decode(&payload).unwrap();
            assert_eq!(decoded.sample_rate, 16_000);
            assert_eq!(decoded.channels, 1);
            assert_eq!(decoded.samples.len(), 320);
        }

        assert_eq!(
            codec_descriptor(&config, 18),
            Err("invalid-amr-wb-payload-type")
        );
    }

    #[test]
    fn unsupported_negotiated_codec_fails_closed() {
        let config = negotiated("peer-controlled-unknown", 8_000, 1);
        assert!(codec_descriptor(&config, 96).is_err());
        assert!(SipPayloadCodec::from_negotiated(&config, 96).is_err());

        let internal_pcm = negotiated("pcm_s16le", 16_000, 1);
        assert!(codec_descriptor(&internal_pcm, 96).is_err());
        assert!(SipPayloadCodec::from_negotiated(&internal_pcm, 96).is_err());
    }
}

#[cfg(test)]
mod rfc4733_tests {
    use super::{parse_rfc4733_digit, TelephoneEventTracker};

    #[test]
    fn start_packet_returns_digit() {
        // event=5, end=0, volume=10, duration=0
        let packet = [0x05, 0x0A, 0x00, 0x00];
        assert_eq!(parse_rfc4733_digit(&packet), Some('5'));
    }

    #[test]
    fn initial_nonzero_duration_is_valid() {
        // event=5, end=0, volume=10, duration=160
        let packet = [0x05, 0x0A, 0x00, 0xA0];
        assert_eq!(parse_rfc4733_digit(&packet), Some('5'));
    }

    #[test]
    fn tracker_suppresses_repetitions_and_accepts_timestamp_wrap() {
        let mut tracker = TelephoneEventTracker::default();
        let start = [5, 10, 0, 160];
        assert_eq!(tracker.digit("source-a", u32::MAX - 100, &start), Some('5'));
        assert_eq!(
            tracker.digit("source-a", u32::MAX - 100, &[5, 10, 1, 64]),
            None
        );
        assert_eq!(
            tracker.digit("source-a", u32::MAX - 100, &[5, 0x8a, 3, 32]),
            None
        );
        assert_eq!(tracker.digit("source-a", 100, &start), Some('5'));
        assert_eq!(tracker.digit("source-a", u32::MAX - 100, &start), None);
        assert_eq!(tracker.digit("source-a", 200, &[5, 0x8a, 3, 32]), None);
        assert_eq!(tracker.digit("source-b", 0, &start), Some('5'));
    }

    #[test]
    fn star_hash_letters_map_correctly() {
        assert_eq!(parse_rfc4733_digit(&[10, 0, 0, 0]), Some('*'));
        assert_eq!(parse_rfc4733_digit(&[11, 0, 0, 0]), Some('#'));
        assert_eq!(parse_rfc4733_digit(&[12, 0, 0, 0]), Some('A'));
        assert_eq!(parse_rfc4733_digit(&[15, 0, 0, 0]), Some('D'));
    }

    #[test]
    fn unknown_events_return_none() {
        assert_eq!(parse_rfc4733_digit(&[99, 0, 0, 0]), None);
        assert_eq!(parse_rfc4733_digit(&[0xFF, 0, 0, 0]), None);
    }

    #[test]
    fn short_payload_returns_none() {
        assert_eq!(parse_rfc4733_digit(&[5, 0, 0]), None);
        assert_eq!(parse_rfc4733_digit(&[]), None);
    }
}

#[cfg(test)]
mod outbound_timestamp_tests {
    use super::{advance_outbound_timestamp, OutboundRtpClock};

    /// A full 20 ms G.711 frame at 8 kHz mono.
    const G711_FRAME_SAMPLES: usize = 160;

    /// The graph supplies timestamps in the G.711 sink's 8 kHz clock. The SIP
    /// media runtime chooses a local epoch while retaining that cadence.
    #[test]
    fn normalizes_remote_epoch_and_advances_in_the_sink_clock() {
        let mut clock = OutboundRtpClock::default();
        let upstream = [1_000_000u32, 1_000_160, 1_000_320, 1_000_480];
        let out: Vec<u32> = upstream
            .iter()
            .map(|&u| advance_outbound_timestamp(&mut clock, G711_FRAME_SAMPLES, u))
            .collect();
        assert_eq!(out, vec![0, 160, 320, 480]);
    }

    #[test]
    fn preserves_a_suppressed_dtx_interval() {
        let mut clock = OutboundRtpClock::default();
        assert_eq!(advance_outbound_timestamp(&mut clock, 80, 4_000), 0);
        assert_eq!(advance_outbound_timestamp(&mut clock, 80, 4_080), 80);
        // Two 10 ms frames were suppressed. Resumed audio retains the +160 gap.
        assert_eq!(advance_outbound_timestamp(&mut clock, 80, 4_320), 320);
        assert_eq!(clock.next_output_timestamp, 400);
    }

    #[test]
    fn local_clock_and_remote_input_wrap_at_u32_boundary() {
        let mut clock = OutboundRtpClock {
            next_output_timestamp: u32::MAX - 100,
            expected_input_timestamp: None,
        };
        let first = advance_outbound_timestamp(&mut clock, 160, u32::MAX - 100);
        assert_eq!(first, u32::MAX - 100);
        assert_eq!(clock.next_output_timestamp, 59);
        assert_eq!(advance_outbound_timestamp(&mut clock, 160, 59), 59);
        assert_eq!(clock.next_output_timestamp, 219);
    }
}

#[cfg(test)]
mod receiver_ownership_tests {
    use super::*;
    use crate::api::unified::Config as ApiConfig;

    #[test]
    fn second_receiver_acquisition_is_a_typed_error() {
        let stream = SipMediaStream::dormant(Direction::Inbound);

        let reservation = stream.reserve_frames_in().expect("reserve receiver");
        assert!(matches!(
            stream.try_frames_in(),
            Err(RvoipError::InvalidState(_))
        ));
        drop(reservation);
        assert!(stream.try_frames_in().is_ok());
        assert!(matches!(
            stream.try_frames_in(),
            Err(RvoipError::InvalidState(_))
        ));
    }

    #[tokio::test]
    async fn dormant_stream_allocates_no_task_and_close_is_sticky() {
        let stream = SipMediaStream::dormant(Direction::Outbound);
        assert_eq!(stream.inner.lifecycle.current(), SipMediaLifecycle::Dormant);
        assert!(stream
            .inner
            .driver_abort
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_none());
        assert!(stream
            .inner
            .bind_target
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_none());

        Arc::clone(&stream).close().await.unwrap();
        assert_eq!(stream.inner.lifecycle.current(), SipMediaLifecycle::Closed);
        Arc::clone(&stream).close().await.unwrap();
        assert_eq!(stream.inner.lifecycle.current(), SipMediaLifecycle::Closed);
    }

    #[test]
    fn dormant_outbound_stream_rejects_writes_with_typed_state_error() {
        let stream = SipMediaStream::dormant(Direction::Outbound);

        assert!(matches!(
            stream.try_frames_out(),
            Err(RvoipError::InvalidState(
                "SIP media stream is not activated"
            ))
        ));
        assert!(
            stream.frames_out().is_closed(),
            "the legacy sender must fail closed instead of buffering pre-activation media"
        );
        assert!(stream
            .inner
            .driver_abort
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_none());
        assert!(stream
            .inner
            .bind_target
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_none());
    }

    #[test]
    fn deferred_bound_outbound_stream_remains_unwritable_until_activation_commits() {
        let stream = SipMediaStream::dormant_deferred(Direction::Outbound);
        assert!(!stream.source_ready());
        assert!(stream.inner.lifecycle.begin_binding());
        assert!(!stream.source_ready());
        assert!(stream.inner.lifecycle.mark_bound());
        assert!(stream.source_ready());

        assert!(matches!(
            stream.try_frames_out(),
            Err(RvoipError::InvalidState(
                "SIP media stream is not activated"
            ))
        ));
        stream.activate_outbound_writes();
        assert!(stream.try_frames_out().is_ok());
    }

    #[test]
    fn legacy_bound_outbound_stream_remains_writable_without_adapter_commit() {
        let stream = SipMediaStream::dormant(Direction::Outbound);
        assert!(stream.inner.lifecycle.begin_binding());
        assert!(stream.inner.lifecycle.mark_bound());
        assert!(stream.try_frames_out().is_ok());
    }

    #[tokio::test]
    async fn dropping_final_owner_aborts_every_inflight_driver() {
        for _ in 0..100 {
            let stream = SipMediaStream::dormant(Direction::Outbound);
            assert!(stream.inner.lifecycle.begin_binding());
            let driver = tokio::spawn(std::future::pending::<()>());
            let abort = driver.abort_handle();
            *stream
                .inner
                .driver_abort
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(abort.clone());
            drop(driver);

            drop(stream);
            tokio::time::timeout(std::time::Duration::from_secs(1), async {
                while !abort.is_finished() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("driver aborted when its final stream owner disappeared");
        }
    }

    #[tokio::test]
    async fn one_hundred_bind_callers_share_one_immutable_target() {
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("media-bind-singleflight", 0))
            .await
            .expect("coordinator");
        let other = UnifiedCoordinator::new(ApiConfig::local("media-bind-mismatch", 0))
            .await
            .expect("second coordinator");
        let stream = SipMediaStream::dormant(Direction::Outbound);
        let session_id = SessionId::new();
        let gate = Arc::new(tokio::sync::Barrier::new(101));
        let mut callers = Vec::new();
        for _ in 0..100 {
            let caller_stream = Arc::clone(&stream);
            let caller_coordinator = Arc::clone(&coordinator);
            let caller_session = session_id.clone();
            let caller_gate = Arc::clone(&gate);
            callers.push(tokio::spawn(async move {
                caller_gate.wait().await;
                caller_stream.bind(caller_coordinator, caller_session).await
            }));
        }
        gate.wait().await;
        for caller in callers {
            assert!(caller.await.expect("bind caller").is_err());
        }
        {
            let target = stream
                .inner
                .bind_target
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            assert!(target
                .as_ref()
                .is_some_and(|target| target.matches(&coordinator, &session_id)));
        }
        assert_eq!(stream.inner.lifecycle.current(), SipMediaLifecycle::Failed);

        let _mismatch = stream
            .bind(Arc::clone(&other), session_id.clone())
            .await
            .expect_err("coordinator identity is immutable");
        let _mismatch = stream
            .bind(Arc::clone(&coordinator), SessionId::new())
            .await
            .expect_err("session identity is immutable");
        assert!(stream.is_bound_to(&coordinator, &session_id));
        assert!(!stream.is_bound_to(&other, &session_id));

        Arc::clone(&stream).close().await.unwrap();
        drop(stream);
        coordinator
            .shutdown_gracefully(Some(std::time::Duration::from_secs(1)))
            .await
            .expect("shutdown");
        other
            .shutdown_gracefully(Some(std::time::Duration::from_secs(1)))
            .await
            .expect("shutdown");
    }

    #[tokio::test]
    async fn closing_is_monotonic_against_bound_and_failed_races() {
        for _ in 0..100 {
            let lifecycle = Arc::new(SipMediaLifecycleState::new());
            assert!(lifecycle.begin_binding());
            let close_lifecycle = Arc::clone(&lifecycle);
            let close = tokio::spawn(async move { close_lifecycle.begin_closing() });
            let bind_lifecycle = Arc::clone(&lifecycle);
            let bind = tokio::spawn(async move { bind_lifecycle.mark_bound() });
            let _ = tokio::join!(close, bind);
            lifecycle.begin_closing();
            assert_eq!(lifecycle.current(), SipMediaLifecycle::Closing);
            assert!(!lifecycle.mark_bound());
            assert!(!lifecycle.mark_failed());
            lifecycle.mark_closed();
            assert_eq!(lifecycle.current(), SipMediaLifecycle::Closed);
            assert!(!lifecycle.begin_closing());
        }
    }

    #[tokio::test]
    async fn closed_is_published_only_after_driver_termination() {
        let stream = SipMediaStream::dormant(Direction::Outbound);
        assert!(stream.inner.lifecycle.begin_binding());
        let driver = tokio::spawn(std::future::pending::<()>());
        let abort = driver.abort_handle();
        *stream
            .inner
            .driver_abort
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(abort.clone());
        drop(driver);

        let closing_stream = Arc::clone(&stream);
        let close = tokio::spawn(async move { closing_stream.close().await });
        tokio::task::yield_now().await;
        assert_eq!(stream.inner.lifecycle.current(), SipMediaLifecycle::Closing);
        assert!(!abort.is_finished());
        abort.abort();
        close.await.expect("close task").expect("stream close");
        assert!(abort.is_finished());
        assert_eq!(stream.inner.lifecycle.current(), SipMediaLifecycle::Closed);
    }
}
