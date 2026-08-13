use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::identity::IdentityAssurance;

// =====================================================================
// Codec types
// =====================================================================

/// Legacy flat-fields codec entry — used internally by SIP/RTP adapters
/// that need the parsed `clock_rate_hz` / `channels` numbers directly.
/// Bridges to/from [`Codec`] (the spec wire shape) via `From`/`TryFrom`.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct CodecInfo {
    pub name: String,
    pub clock_rate_hz: u32,
    pub channels: u8,
    pub fmtp: Option<String>,
    /// The RTP payload type this codec negotiated, when the reporting
    /// transport knows it.
    ///
    /// `None` means "not reported", never "no payload type" — so a consumer
    /// that needs one must say what it does about the absence rather than
    /// substitute a guess. Substituting is the bug this field exists to
    /// close: the media pumps used to fall back to Opus's `111` for any
    /// codec they could not name, which put a wrong payload type on real
    /// datagrams.
    ///
    /// Static-payload codecs can be recovered from the name alone via
    /// `rvoip_core::bridge::codec_to_pt`; dynamic ones cannot,
    /// because the same codec takes different numbers on different calls —
    /// AMR routinely negotiates two at once, differing only in
    /// `octet-align`. That is why this is carried rather than derived.
    #[serde(default)]
    pub payload_type: Option<u8>,
}

impl Default for CodecInfo {
    fn default() -> Self {
        default_audio_codec()
    }
}

impl fmt::Debug for CodecInfo {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CodecInfo")
            .field("name_present", &!self.name.is_empty())
            .field("name_bytes", &self.name.len())
            .field("clock_rate_hz", &self.clock_rate_hz)
            .field("channels", &self.channels)
            .field("fmtp_present", &self.fmtp.is_some())
            .field(
                "fmtp_bytes",
                &self.fmtp.as_ref().map_or(0, std::string::String::len),
            )
            .finish()
    }
}

/// Reasonable default for adapter and orchestrator paths that need a
/// codec descriptor before negotiation has run (e.g. `Orchestrator::fanout_frame`
/// allocating a subscriber-side MediaStream before the publisher's
/// negotiated codec has propagated). Matches the codec the v0 default
/// CapabilityDescriptor advertises first.
pub fn default_audio_codec() -> CodecInfo {
    CodecInfo {
        name: "opus".into(),
        clock_rate_hz: 48_000,
        channels: 1,
        fmtp: None,
        // Pre-negotiation placeholder: nothing has been negotiated yet, so
        // there is no payload type to report.
        payload_type: None,
    }
}

impl CodecInfo {
    /// Build a `CodecInfo` from just the codec name, using
    /// standards-defined defaults for `clock_rate_hz` / `channels`.
    /// Used by the multi-party fanout path (plan B1) where the wire
    /// catalog only records the chosen codec name; richer params would
    /// require carrying the full negotiation result through more layers.
    /// Falls back to the `name`/48k/mono shape for codecs not in the
    /// table — fanout still works, the client just sees an audio stream
    /// it may or may not be able to decode (B2 codec-mismatch refusal
    /// is the right place to surface that).
    pub fn from_name_with_defaults(name: &str) -> Self {
        let (clock_rate_hz, channels) = match name {
            "opus" => (48_000, 1),
            "g.711-mu" | "PCMU" | "pcmu" => (8_000, 1),
            "g.711-a" | "PCMA" | "pcma" => (8_000, 1),
            "g.722" => (16_000, 1),
            "g.729" => (8_000, 1),
            "pcm_s16le" | "PCM_S16LE" => (16_000, 1),
            _ => (48_000, 1),
        };
        Self {
            name: name.to_string(),
            clock_rate_hz,
            channels,
            fmtp: None,
            // Derived from a name alone, so there is no negotiation result
            // to report. Callers that know the payload type should set it.
            payload_type: None,
        }
    }

    /// Record the payload type this codec negotiated.
    ///
    /// Chainable so the adapters that do know it can say so at the point
    /// they build the descriptor, rather than mutating it later.
    #[must_use]
    pub fn with_payload_type(mut self, payload_type: u8) -> Self {
        self.payload_type = Some(payload_type);
        self
    }
}

/// One codec entry on the wire, matching CONVERSATION_PROTOCOL.md §8's
/// `{"name": "opus", "params": {"sample_rate": 48000, ...}}` shape.
/// Distinct from [`CodecInfo`] — the flat-fields shape can't represent
/// the spec wire format losslessly. Conversion helpers below.
#[derive(Clone, Serialize, Deserialize)]
pub struct Codec {
    pub name: String,
    #[serde(default)]
    pub params: BTreeMap<String, serde_json::Value>,
}

impl fmt::Debug for Codec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Codec")
            .field("name_present", &!self.name.is_empty())
            .field("name_bytes", &self.name.len())
            .field("parameter_count", &self.params.len())
            .finish()
    }
}

impl Codec {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            params: BTreeMap::new(),
        }
    }
}

impl From<CodecInfo> for Codec {
    fn from(c: CodecInfo) -> Self {
        let mut params = BTreeMap::new();
        params.insert("sample_rate".into(), serde_json::json!(c.clock_rate_hz));
        params.insert("channels".into(), serde_json::json!(c.channels));
        if let Some(fmtp) = c.fmtp {
            params.insert("fmtp".into(), serde_json::Value::String(fmtp));
        }
        // Carried so a round trip through the wire shape does not silently
        // drop it. Absent when not reported, which keeps the emitted params
        // identical to before for every producer that does not set one.
        if let Some(payload_type) = c.payload_type {
            params.insert("payload_type".into(), serde_json::json!(payload_type));
        }
        Self {
            name: c.name,
            params,
        }
    }
}

impl TryFrom<Codec> for CodecInfo {
    type Error = &'static str;
    fn try_from(c: Codec) -> Result<Self, Self::Error> {
        let clock_rate_hz = c
            .params
            .get("sample_rate")
            .and_then(|v| v.as_u64())
            .ok_or("missing or invalid sample_rate")? as u32;
        let channels = c
            .params
            .get("channels")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u8;
        let fmtp = c
            .params
            .get("fmtp")
            .and_then(|v| v.as_str())
            .map(String::from);
        // Out-of-range values are dropped rather than truncated: an RTP
        // payload type is 7 bits, and silently wrapping a bad one to a
        // valid-looking number is how a wrong PT reaches the wire.
        let payload_type = c
            .params
            .get("payload_type")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u8::try_from(value).ok())
            .filter(|value| *value < 128);
        Ok(Self {
            name: c.name,
            clock_rate_hz,
            channels,
            fmtp,
            payload_type,
        })
    }
}

// =====================================================================
// CapabilityDescriptor (expanded per CONVERSATION_PROTOCOL.md §8 +
// INTERFACE_DESIGN.md §9)
// =====================================================================

/// Capability descriptor that round-trips through CONVERSATION_PROTOCOL.md
/// §8's JSON shape. Field order matches the spec for readability.
///
/// `supports_dtmf_rfc4733` is a **method** (derived from `dtmf_modes`),
/// not a field — `dtmf_modes` is the single source of truth on the wire
/// and the boolean would silently desync from a custom serde round-trip.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct CapabilityDescriptor {
    #[serde(default)]
    pub audio_codecs: Vec<CodecInfo>,

    #[serde(default)]
    pub video_codecs: Vec<CodecInfo>,

    #[serde(default)]
    pub data_protocols: Vec<DataProtocol>,

    #[serde(default)]
    pub dtmf_modes: Vec<DtmfMode>,

    #[serde(default)]
    pub max_streams_per_connection: u16,

    #[serde(default)]
    pub transport_features: Vec<TransportFeature>,

    /// Gatewayable interop targets (`["sip", "webrtc"]`). Empty when the
    /// endpoint is UCTP-only.
    #[serde(default)]
    pub interop: Vec<InteropTarget>,

    /// IdentityAssurance the peer is offering. Defaults to
    /// `Anonymous` when not declared.
    #[serde(default = "default_assurance_offered")]
    pub identity_assurance_offered: AssuranceLevel,

    /// Minimum IdentityAssurance the peer requires from its counterpart.
    /// `None` means no constraint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_assurance_required: Option<IdentityAssuranceRequirement>,

    /// Legacy boolean retained from the original narrow `CapabilityDescriptor`
    /// for back-compat with consumers that check messaging support
    /// directly. Independent of `dtmf_modes` / `data_protocols`.
    #[serde(default)]
    pub supports_message_text: bool,

    /// Legacy boolean retained from the original narrow `CapabilityDescriptor`.
    /// Independent of `transport_features`.
    #[serde(default)]
    pub supports_srtp: bool,
}

impl fmt::Debug for CapabilityDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CapabilityDescriptor")
            .field("audio_codec_count", &self.audio_codecs.len())
            .field("video_codec_count", &self.video_codecs.len())
            .field("data_protocols", &self.data_protocols)
            .field("dtmf_modes", &self.dtmf_modes)
            .field(
                "max_streams_per_connection",
                &self.max_streams_per_connection,
            )
            .field("transport_features", &self.transport_features)
            .field("interop", &self.interop)
            .field(
                "identity_assurance_offered",
                &self.identity_assurance_offered,
            )
            .field(
                "identity_assurance_required",
                &self.identity_assurance_required,
            )
            .field("supports_message_text", &self.supports_message_text)
            .field("supports_srtp", &self.supports_srtp)
            .finish()
    }
}

fn default_assurance_offered() -> AssuranceLevel {
    AssuranceLevel::Anonymous
}

impl CapabilityDescriptor {
    /// True when `dtmf_modes` includes `Rfc4733`. Defined as a method
    /// (not a field) so `dtmf_modes` is the single source of truth.
    pub fn supports_dtmf_rfc4733(&self) -> bool {
        self.dtmf_modes.contains(&DtmfMode::Rfc4733)
    }
}

// =====================================================================
// Capability catalog enums
// =====================================================================

/// `data_protocols` catalog per CONVERSATION_PROTOCOL.md §8.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DataProtocol {
    Text,
    Json,
    Binary,
}

/// `dtmf_modes` catalog per CONVERSATION_PROTOCOL.md §8.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DtmfMode {
    #[serde(rename = "rfc4733")]
    Rfc4733,
    #[serde(rename = "info")]
    Info,
}

/// `transport_features` catalog per CONVERSATION_PROTOCOL.md §8.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransportFeature {
    MediaDatagrams,
    ConnectionMigration,
    SessionResumption,
    #[serde(rename = "0rtt")]
    ZeroRtt,
    #[serde(rename = "transcode-g711-opus")]
    TranscodeG711Opus,
    /// Catch-all for future entries so the wire format stays forward-compat.
    #[serde(other)]
    Unknown,
}

/// `identity_assurance_required` levels per CONVERSATION_PROTOCOL.md §5.6 / §8.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IdentityAssuranceRequirement {
    None,
    Pseudonymous,
    Identified,
    TaskScoped,
    UserAuthorized,
}

/// Substrate name as it appears on the UCTP wire (CONVERSATION_PROTOCOL.md
/// §8 `interop`). Lowercase kebab-style. Distinct from
/// [`crate::connection::Transport`] (PascalCase Rust enum) because the
/// wire format uses lowercase and is the source of truth for
/// cross-language peers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InteropTarget {
    Sip,
    Webrtc,
    Quic,
    Webtransport,
    Websocket,
}

/// Wire form of `identity_assurance_offered`. Maps to the gradient
/// in [`IdentityAssurance`] but flattened to a single string because the
/// wire format does not carry the variant payload.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AssuranceLevel {
    #[default]
    Anonymous,
    Pseudonymous,
    Identified,
    TaskScoped,
    UserAuthorized,
}

impl AssuranceLevel {
    /// Map the wire-form level to its kebab-case label.
    pub fn to_core(self) -> Option<&'static str> {
        Some(match self {
            AssuranceLevel::Anonymous => "anonymous",
            AssuranceLevel::Pseudonymous => "pseudonymous",
            AssuranceLevel::Identified => "identified",
            AssuranceLevel::TaskScoped => "task-scoped",
            AssuranceLevel::UserAuthorized => "user-authorized",
        })
    }

    /// Derive the wire level from a full [`IdentityAssurance`].
    pub fn from_core(assurance: &IdentityAssurance) -> Self {
        match assurance {
            IdentityAssurance::Anonymous => AssuranceLevel::Anonymous,
            IdentityAssurance::Pseudonymous { .. } => AssuranceLevel::Pseudonymous,
            IdentityAssurance::Identified { .. } => AssuranceLevel::Identified,
            IdentityAssurance::TaskScoped { .. } => AssuranceLevel::TaskScoped,
            IdentityAssurance::UserAuthorized { .. } => AssuranceLevel::UserAuthorized,
            // D2 — DTLS fingerprint is key-binding without a real-world
            // identity, so the closest wire level is Pseudonymous.
            IdentityAssurance::DtlsFingerprint { .. } => AssuranceLevel::Pseudonymous,
        }
    }
}

// =====================================================================
// Existing intersection / negotiation types (retained from the narrow
// CapabilityDescriptor era — used by rvoip-sip and other adapters)
// =====================================================================

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct CapabilityIntersection {
    pub audio: Option<CodecInfo>,
    pub video: Option<CodecInfo>,
    pub dtmf_method: Option<DtmfMethod>,
    pub messaging_enabled: bool,
}

impl fmt::Debug for CapabilityIntersection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CapabilityIntersection")
            .field("audio_present", &self.audio.is_some())
            .field("video_present", &self.video.is_some())
            .field("dtmf_method", &self.dtmf_method)
            .field("messaging_enabled", &self.messaging_enabled)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DtmfMethod {
    Rfc4733,
    SipInfo,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct NegotiatedCodecs {
    pub audio: Option<CodecInfo>,
    pub video: Option<CodecInfo>,
}

impl fmt::Debug for NegotiatedCodecs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NegotiatedCodecs")
            .field("audio_present", &self.audio.is_some())
            .field("video_present", &self.video.is_some())
            .finish()
    }
}

// =====================================================================
// §8.1 negotiation algorithm (relocated from rvoip-uctp)
// =====================================================================

/// Outcome of running [`negotiate_streams`] over an offer/answer pair.
#[derive(Clone)]
pub enum NegotiationOutcome {
    /// Per-Stream chosen codecs. Order matches the input `streams_offered`.
    Ok(Vec<NegotiatedStream>),
    /// Spec §11.2 488: no codecs overlapped on any stream.
    NotAcceptable488,
}

impl fmt::Debug for NegotiationOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ok(streams) => formatter
                .debug_struct("Ok")
                .field("stream_count", &streams.len())
                .finish(),
            Self::NotAcceptable488 => formatter.write_str("NotAcceptable488"),
        }
    }
}

/// One stream's negotiation result.
#[derive(Clone)]
pub struct NegotiatedStream {
    pub stream_id: String,
    pub kind: String,
    pub direction: String,
    /// `Some(codec_name)` when at least one of the offerer's preferences
    /// matched the answerer's capability; `None` when this individual
    /// stream had no overlap.
    pub chosen_codec: Option<String>,
}

impl fmt::Debug for NegotiatedStream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NegotiatedStream")
            .field("stream_id_present", &!self.stream_id.is_empty())
            .field("stream_id_bytes", &self.stream_id.len())
            .field("kind_present", &!self.kind.is_empty())
            .field("kind_bytes", &self.kind.len())
            .field("direction_present", &!self.direction.is_empty())
            .field("direction_bytes", &self.direction.len())
            .field("chosen_codec_present", &self.chosen_codec.is_some())
            .finish()
    }
}

/// Input shape mirroring `connection.offer.streams_offered`.
#[derive(Clone)]
pub struct StreamOffer<'a> {
    pub id: &'a str,
    pub kind: &'a str,
    pub direction: &'a str,
    pub codec_preferences: &'a [String],
}

impl fmt::Debug for StreamOffer<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StreamOffer")
            .field("id_present", &!self.id.is_empty())
            .field("id_bytes", &self.id.len())
            .field("kind_present", &!self.kind.is_empty())
            .field("kind_bytes", &self.kind.len())
            .field("direction_present", &!self.direction.is_empty())
            .field("direction_bytes", &self.direction.len())
            .field("codec_preference_count", &self.codec_preferences.len())
            .finish()
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    #[test]
    fn internal_pcm_codec_uses_wideband_mono_defaults() {
        let codec = CodecInfo::from_name_with_defaults("pcm_s16le");
        assert_eq!(codec.clock_rate_hz, 16_000);
        assert_eq!(codec.channels, 1);
        assert!(codec.fmtp.is_none());
    }

    /// A payload type that survives the trip out but not back would be worse
    /// than not carrying one: consumers would see `None` and fall back to
    /// deriving from the name, which is the behaviour this field replaces.
    #[test]
    fn the_negotiated_payload_type_survives_the_wire_shape() {
        let original = CodecInfo::from_name_with_defaults("opus").with_payload_type(96);
        let restored = CodecInfo::try_from(Codec::from(original.clone()))
            .expect("a descriptor this crate produced must parse back");
        assert_eq!(restored.payload_type, Some(96));
        assert_eq!(restored, original);
    }

    /// Producers that report nothing must emit exactly what they emitted
    /// before, so adding this field cannot change an existing peer's wire.
    #[test]
    fn an_unreported_payload_type_adds_nothing_to_the_wire() {
        let codec = CodecInfo::from_name_with_defaults("opus");
        assert!(!Codec::from(codec).params.contains_key("payload_type"));
    }

    /// An RTP payload type is seven bits. Truncating an out-of-range value
    /// would manufacture a plausible-looking number from a corrupt one.
    #[test]
    fn an_out_of_range_payload_type_is_dropped_not_truncated() {
        for bogus in [128_u64, 256, 300, u64::from(u32::MAX)] {
            let mut wire = Codec::from(CodecInfo::from_name_with_defaults("opus"));
            wire.params
                .insert("payload_type".into(), serde_json::json!(bogus));
            let restored = CodecInfo::try_from(wire).expect("the rest still parses");
            assert_eq!(
                restored.payload_type, None,
                "{bogus} is not a payload type and must not become one"
            );
        }
    }

    #[test]
    fn capability_diagnostics_never_render_peer_strings() {
        const CANARY: &str = "capability-canary\r\nAuthorization: exposed";
        let codec = CodecInfo {
            name: CANARY.into(),
            clock_rate_hz: 48_000,
            channels: 1,
            fmtp: Some(CANARY.into()),
            payload_type: None,
        };
        let descriptor = CapabilityDescriptor {
            audio_codecs: vec![codec.clone()],
            ..CapabilityDescriptor::default()
        };
        let negotiated = NegotiatedStream {
            stream_id: CANARY.into(),
            kind: CANARY.into(),
            direction: CANARY.into(),
            chosen_codec: Some(CANARY.into()),
        };
        for debug in [
            format!("{codec:?}"),
            format!("{:?}", Codec::new(CANARY)),
            format!("{descriptor:?}"),
            format!("{negotiated:?}"),
            format!("{:?}", NegotiationOutcome::Ok(vec![negotiated])),
        ] {
            assert!(!debug.contains(CANARY));
        }
    }
}

/// Run the §8.1 negotiation algorithm on a single offer/answer pair.
///
/// 1. Walks the offerer's `codec_preferences` in order.
/// 2. Picks the first codec the answerer advertises (audio or video).
/// 3. If **no** stream gets a codec, returns
///    [`NegotiationOutcome::NotAcceptable488`].
pub fn negotiate_streams<'a, I>(
    streams_offered: I,
    answerer: &CapabilityDescriptor,
) -> NegotiationOutcome
where
    I: IntoIterator<Item = StreamOffer<'a>>,
{
    let answerer_codecs: std::collections::HashSet<&str> = answerer
        .audio_codecs
        .iter()
        .chain(answerer.video_codecs.iter())
        .map(|c| c.name.as_str())
        .collect();

    let mut results = Vec::new();
    let mut any_match = false;

    for offer in streams_offered {
        let chosen = offer
            .codec_preferences
            .iter()
            .find(|c| answerer_codecs.contains(c.as_str()))
            .cloned();
        if chosen.is_some() {
            any_match = true;
        }
        results.push(NegotiatedStream {
            stream_id: offer.id.to_string(),
            kind: offer.kind.to_string(),
            direction: offer.direction.to_string(),
            chosen_codec: chosen,
        });
    }

    if any_match {
        NegotiationOutcome::Ok(results)
    } else {
        NegotiationOutcome::NotAcceptable488
    }
}
