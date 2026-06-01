use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::identity::IdentityAssurance;

// =====================================================================
// Codec types
// =====================================================================

/// Legacy flat-fields codec entry — used internally by SIP/RTP adapters
/// that need the parsed `clock_rate_hz` / `channels` numbers directly.
/// Bridges to/from [`Codec`] (the spec wire shape) via `From`/`TryFrom`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CodecInfo {
    pub name: String,
    pub clock_rate_hz: u32,
    pub channels: u8,
    pub fmtp: Option<String>,
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
            _ => (48_000, 1),
        };
        Self {
            name: name.to_string(),
            clock_rate_hz,
            channels,
            fmtp: None,
        }
    }
}

/// One codec entry on the wire, matching CONVERSATION_PROTOCOL.md §8's
/// `{"name": "opus", "params": {"sample_rate": 48000, ...}}` shape.
/// Distinct from [`CodecInfo`] — the flat-fields shape can't represent
/// the spec wire format losslessly. Conversion helpers below.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Codec {
    pub name: String,
    #[serde(default)]
    pub params: BTreeMap<String, serde_json::Value>,
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
        Self { name: c.name, params }
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
        Ok(Self {
            name: c.name,
            clock_rate_hz,
            channels,
            fmtp,
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
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CapabilityIntersection {
    pub audio: Option<CodecInfo>,
    pub video: Option<CodecInfo>,
    pub dtmf_method: Option<DtmfMethod>,
    pub messaging_enabled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DtmfMethod {
    Rfc4733,
    SipInfo,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NegotiatedCodecs {
    pub audio: Option<CodecInfo>,
    pub video: Option<CodecInfo>,
}

// =====================================================================
// §8.1 negotiation algorithm (relocated from rvoip-uctp)
// =====================================================================

/// Outcome of running [`negotiate_streams`] over an offer/answer pair.
#[derive(Clone, Debug)]
pub enum NegotiationOutcome {
    /// Per-Stream chosen codecs. Order matches the input `streams_offered`.
    Ok(Vec<NegotiatedStream>),
    /// Spec §11.2 488: no codecs overlapped on any stream.
    NotAcceptable488,
}

/// One stream's negotiation result.
#[derive(Clone, Debug)]
pub struct NegotiatedStream {
    pub stream_id: String,
    pub kind: String,
    pub direction: String,
    /// `Some(codec_name)` when at least one of the offerer's preferences
    /// matched the answerer's capability; `None` when this individual
    /// stream had no overlap.
    pub chosen_codec: Option<String>,
}

/// Input shape mirroring `connection.offer.streams_offered`.
#[derive(Clone, Debug)]
pub struct StreamOffer<'a> {
    pub id: &'a str,
    pub kind: &'a str,
    pub direction: &'a str,
    pub codec_preferences: &'a [String],
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
