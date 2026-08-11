//! Simplified Media Adapter for rvoip-sip
//!
//! Thin translation layer between media-core and state machine.
//! Focuses only on essential media operations and events.

use crate::adapters::srtp_negotiator::{
    into_public_negotiation_error, SrtpDetailedResult, SrtpNegotiator, SrtpPair,
};
use crate::api::events::{Event, MediaSecurityKeying, MediaSecurityProfile, MediaSecurityState};
use crate::api::lifecycle::{LifecycleIndex, SessionEventPublisher};
use crate::api::unified::{MediaMode, SdesBase64Mode};
use crate::cleanup_diag::{self, CleanupStage};
use crate::errors::{Result, SessionError};
use crate::session_lifecycle::{
    ManagedResourceReleaseError, ManagedSessionResource, OwnedOperation, OwnedOperationCompletion,
    ResourceDescriptor, ResourceInstallationSink, ResourceSpec, SessionOperationKind,
};
use crate::session_registry::{SessionRegistryError, SessionRegistryHandle};
use crate::session_store::{SessionState, SessionStateSnapshot, SessionStore};
use crate::state_table::types::SessionId;
use dashmap::DashMap;
use rvoip_media_core::types::AudioFrame;
use rvoip_media_core::{
    relay::controller::{
        AudioSource, BridgeError, BridgeHandle, MediaConfig, MediaSessionController,
        MediaSessionInfo,
    },
    DialogId,
};
use rvoip_sip_core::sdp::SdpBuilder;
use rvoip_sip_core::types::sdp::{CryptoAttribute, CryptoSuite, ParsedAttribute, SdpSession};
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
#[cfg(test)]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;
use tokio::sync::mpsc;

const DIAG_OFF: u8 = 1;
const DIAG_ON: u8 = 2;
const AUDIO_RECEIVER_CHANNEL_FRAMES: usize = 128;
const MEDIA_CREATE_ALLOCATION_TIMEOUT: Duration = Duration::from_secs(15);
const MEDIA_CREATE_OWNED_OPERATION_TIMEOUT: Duration = Duration::from_secs(30);
const MEDIA_AUDIO_OWNED_OPERATION_TIMEOUT: Duration = Duration::from_secs(30);
const MEDIA_RESOURCE_RELEASE_TIMEOUT: Duration = Duration::from_secs(12);

static SRTP_DIAGNOSTICS: AtomicU8 = AtomicU8::new(DIAG_OFF);
static MEDIA_SDP_DIAGNOSTICS: AtomicU8 = AtomicU8::new(DIAG_OFF);

fn bounded_sdp_failure(stage: &'static str, class: &'static str) -> SessionError {
    SessionError::SDPNegotiationFailed(format!(
        "SDP negotiation failed (stage={stage}, class={class})"
    ))
}

#[cfg(feature = "perf-infra-memory-diagnostics")]
fn spawn_memory_tracked<F>(kind: &'static str, future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    rvoip_infra_common::memory_diagnostics::spawn_tracked(kind, future)
}

#[cfg(not(feature = "perf-infra-memory-diagnostics"))]
fn spawn_memory_tracked<F>(_: &'static str, future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(future)
}

/// NEXT_STEPS B.1 diag — process-global cleanup counter so external
/// observers (the `perf_listener` example, integration tests) can read
/// how many sessions have been fully cleaned up without subscribing to
/// the full tracing log. This is wire-light: an `AtomicU64` per
/// process, incremented once per `cleanup_session` exit. The pair to
/// `perf_listener`'s `accepted_total` lets us see at a glance whether
/// the cleanup path is keeping pace with the accept path.
///
/// The instrumentation is unconditional (no env-gate) because the cost
/// is one atomic add per call hangup. Strip the public getter after
/// the 100-CPS knee is resolved if it turns out to be only diagnostic.
pub mod cleanup_session_diag {
    use std::sync::atomic::{AtomicU64, Ordering};

    static CLEANED: AtomicU64 = AtomicU64::new(0);

    /// Increment the cleanup counter. Called from
    /// [`super::MediaAdapter::cleanup_session`].
    pub fn record_cleanup() {
        CLEANED.fetch_add(1, Ordering::Relaxed);
    }

    /// Read the cumulative count of completed cleanups.
    pub fn cleaned_total() -> u64 {
        CLEANED.load(Ordering::Relaxed)
    }
}

fn sdp_origin_session_id(raw_id: &str) -> String {
    let candidate = raw_id
        .strip_prefix("media-session-")
        .or_else(|| raw_id.strip_prefix("session-"))
        .unwrap_or(raw_id);

    if !candidate.is_empty() && candidate.bytes().all(|b| b.is_ascii_digit()) {
        return candidate.to_string();
    }

    if let Ok(uuid) = uuid::Uuid::parse_str(candidate) {
        let bytes = uuid.as_u128().to_be_bytes();
        let low = u64::from_be_bytes(bytes[8..16].try_into().expect("uuid low bytes"));
        return low.max(1).to_string();
    }

    let mut hash = 14_695_981_039_346_656_037u64;
    for byte in raw_id.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    hash.max(1).to_string()
}

fn advance_sdp_origin(session: &mut SessionState) -> (String, u64) {
    if session.sdp_origin_session_id.is_empty() {
        session.sdp_origin_session_id = sdp_origin_session_id(&session.session_id.0);
    }
    session.sdp_origin_version = session.sdp_origin_version.saturating_add(1);
    (
        session.sdp_origin_session_id.clone(),
        session.sdp_origin_version,
    )
}

fn direction_attribute(direction: crate::types::MediaDirection) -> &'static str {
    match direction {
        crate::types::MediaDirection::SendRecv => "sendrecv",
        crate::types::MediaDirection::SendOnly => "sendonly",
        crate::types::MediaDirection::RecvOnly => "recvonly",
        crate::types::MediaDirection::Inactive => "inactive",
    }
}

fn audio_direction(session: &SdpSession) -> Option<rvoip_sip_core::MediaDirection> {
    session
        .media_descriptions
        .iter()
        .find(|m| m.media == "audio")
        .and_then(|m| m.direction)
        .or(session.direction)
}

fn sip_direction_to_session(
    direction: rvoip_sip_core::MediaDirection,
) -> crate::types::MediaDirection {
    match direction {
        rvoip_sip_core::MediaDirection::SendRecv => crate::types::MediaDirection::SendRecv,
        rvoip_sip_core::MediaDirection::SendOnly => crate::types::MediaDirection::SendOnly,
        rvoip_sip_core::MediaDirection::RecvOnly => crate::types::MediaDirection::RecvOnly,
        rvoip_sip_core::MediaDirection::Inactive => crate::types::MediaDirection::Inactive,
    }
}

fn answer_direction_for_offer(
    offer_direction: &Option<rvoip_sip_core::MediaDirection>,
) -> crate::types::MediaDirection {
    match offer_direction.unwrap_or(rvoip_sip_core::MediaDirection::SendRecv) {
        rvoip_sip_core::MediaDirection::SendRecv => crate::types::MediaDirection::SendRecv,
        rvoip_sip_core::MediaDirection::SendOnly => crate::types::MediaDirection::RecvOnly,
        rvoip_sip_core::MediaDirection::RecvOnly => crate::types::MediaDirection::SendOnly,
        rvoip_sip_core::MediaDirection::Inactive => crate::types::MediaDirection::Inactive,
    }
}

fn local_direction_from_remote_answer(
    answer_direction: &Option<rvoip_sip_core::MediaDirection>,
) -> crate::types::MediaDirection {
    match answer_direction.unwrap_or(rvoip_sip_core::MediaDirection::SendRecv) {
        rvoip_sip_core::MediaDirection::SendRecv => crate::types::MediaDirection::SendRecv,
        rvoip_sip_core::MediaDirection::SendOnly => crate::types::MediaDirection::RecvOnly,
        rvoip_sip_core::MediaDirection::RecvOnly => crate::types::MediaDirection::SendOnly,
        rvoip_sip_core::MediaDirection::Inactive => crate::types::MediaDirection::Inactive,
    }
}

fn srtp_diagnostics_enabled() -> bool {
    SRTP_DIAGNOSTICS.load(Ordering::Relaxed) == DIAG_ON
}

fn media_diagnostics_enabled() -> bool {
    MEDIA_SDP_DIAGNOSTICS.load(Ordering::Relaxed) == DIAG_ON
}

fn sdp_diagnostics_enabled() -> bool {
    srtp_diagnostics_enabled() || media_diagnostics_enabled()
}

pub(crate) fn set_sdp_diagnostics(srtp_enabled: bool, media_enabled: bool) {
    SRTP_DIAGNOSTICS.store(
        if srtp_enabled { DIAG_ON } else { DIAG_OFF },
        Ordering::Relaxed,
    );
    MEDIA_SDP_DIAGNOSTICS.store(
        if media_enabled { DIAG_ON } else { DIAG_OFF },
        Ordering::Relaxed,
    );
}

fn emit_srtp_diag(line: String) {
    eprintln!("SRTP_DIAG {}", line);
    tracing::info!("SRTP_DIAG {}", line);
}

fn emit_media_diag(line: String) {
    eprintln!("MEDIA_DIAG {}", line);
    tracing::info!("MEDIA_DIAG {}", line);
}

fn emit_sdp_diag(line: String) {
    if srtp_diagnostics_enabled() {
        emit_srtp_diag(line.clone());
    }
    if media_diagnostics_enabled() {
        emit_media_diag(line);
    }
}

fn crypto_attribute_diag(count: usize) -> String {
    if count > 0 {
        format!("crypto_attrs={} sdp_attribute=a=crypto", count)
    } else {
        "crypto_attrs=0".to_string()
    }
}

fn audio_transport(session: &SdpSession) -> Option<&str> {
    session
        .media_descriptions
        .iter()
        .find(|m| m.media == "audio")
        .map(|m| m.protocol.as_str())
}

/// NEXT_STEPS C2 — lookup helper from RTP payload type to the
/// `a=rtpmap:` value (without the PT prefix). Returns `None` for
/// payload types we don't know how to advertise; callers should
/// skip emitting an rtpmap for those (legal per RFC 4566 for
/// static PTs 0/8 etc., but a builder convention here is to emit
/// rtpmap for every PT for explicitness).
pub(crate) fn rtpmap_for_pt(pt: u8) -> Option<&'static str> {
    match pt {
        0 => Some("PCMU/8000"),
        8 => Some("PCMA/8000"),
        9 => Some("G722/8000"),
        13 => Some("CN/8000"),
        18 => Some("G729/8000"),
        101 => Some("telephone-event/8000"),
        // RFC 4867 transport configurations are mutually incompatible bit
        // patterns, so each is offered as its own payload type rather than
        // negotiated down. Wideband first: it is the HD-voice codec.
        AMR_WB_BE_PT => Some("AMR-WB/16000"),
        AMR_WB_OA_PT => Some("AMR-WB/16000"),
        AMR_NB_BE_PT => Some("AMR/8000"),
        AMR_NB_OA_PT => Some("AMR/8000"),
        111 => Some("opus/48000/2"),
        _ => None,
    }
}

/// AMR-WB, bandwidth-efficient framing (the RFC 4867 default).
pub(crate) const AMR_WB_BE_PT: u8 = 104;
/// AMR-WB, octet-aligned framing.
pub(crate) const AMR_WB_OA_PT: u8 = 105;
/// AMR-NB, bandwidth-efficient framing.
pub(crate) const AMR_NB_BE_PT: u8 = 106;
/// AMR-NB, octet-aligned framing.
pub(crate) const AMR_NB_OA_PT: u8 = 107;


/// NEXT_STEPS C2 — `a=fmtp:` value for payload types that require
/// one. Returns `None` for codecs that work fine without an fmtp.
#[cfg(test)]
pub(crate) fn fmtp_for_pt(pt: u8) -> Option<&'static str> {
    fmtp_for_pt_with_g729_annex_b(pt, true)
}

/// The `a=fmtp` to put in an answer for `pt`.
///
/// For every codec but AMR this is the fixed per-payload-type string
/// [`fmtp_for_pt_with_g729_annex_b`] returns.
///
/// # Why AMR cannot use that table
///
/// RFC 4867 §8.3.1 makes the transport-format parameters — `octet-align`,
/// `crc`, `robust-sorting`, `interleaving`, `channels` — a mutually
/// incompatible set that an answerer must echo rather than renegotiate. The
/// table keys on *our* payload-type constants, which says nothing about what
/// the peer actually offered on that number:
///
/// - a peer offering `octet-align=1` on PT 104, our bandwidth-efficient
///   number, was answered with no fmtp at all — so we advertised
///   bandwidth-efficient and then transmitted octet-aligned, because the codec
///   is configured from the offer. Unparseable audio, no error;
/// - a peer using its own dynamic number gets nothing from a table keyed on
///   ours.
///
/// So the answer echoes the offer's transport parameters for that payload
/// type. `mode-set` and the rest are deliberately not echoed: they constrain
/// which modes may be used rather than how a frame is laid out, and an
/// answerer states its own.
fn answer_fmtp_for_pt(offer: &SdpSession, pt: u8, g729_annex_b: bool) -> Option<String> {
    if !sdp_payload_is_amr(offer, pt) {
        return fmtp_for_pt_with_g729_annex_b(pt, g729_annex_b).map(ToString::to_string);
    }

    let offered = audio_fmtp_params(offer, pt).unwrap_or_default();
    let mut echoed: Vec<&str> = Vec::new();
    for part in offered.split(';') {
        let trimmed = part.trim();
        let Some((name, value)) = trimmed.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        // Only the parameters that decide the bit layout, and only when set:
        // an explicit `octet-align=0` means the default, which is stated by
        // omission.
        let carries = matches!(
            name.trim().to_ascii_lowercase().as_str(),
            "octet-align" | "crc" | "robust-sorting"
        ) && value == "1";
        if carries {
            echoed.push(trimmed);
        }
    }
    (!echoed.is_empty()).then(|| echoed.join("; "))
}

/// Whether `pt` in this offer is mapped to one of the AMR encodings.
fn sdp_payload_is_amr(offer: &SdpSession, pt: u8) -> bool {
    audio_rtpmap(offer, pt).is_some_and(|mapping| {
        mapping.encoding_name.eq_ignore_ascii_case("AMR")
            || mapping.encoding_name.eq_ignore_ascii_case("AMR-WB")
    })
}

pub(crate) fn fmtp_for_pt_with_g729_annex_b(pt: u8, g729_annex_b: bool) -> Option<&'static str> {
    match pt {
        18 if g729_annex_b => Some("annexb=yes"),
        18 => Some("annexb=no"),
        101 => Some("0-15"),
        // Bandwidth-efficient is the RFC 4867 default, so those payload types
        // need no fmtp at all. Omitting it also leaves mode-set absent, which
        // is what an endpoint supporting every mode should do.
        AMR_WB_OA_PT | AMR_NB_OA_PT => Some("octet-align=1"),
        // Opus (PT 111) defaults are fine for VoIP without fmtp; a
        // production deployment may want `useinbandfec=1; minptime=10`.
        _ => None,
    }
}

fn parse_annex_b_param(parameters: &str) -> Option<bool> {
    parameters.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        if !name.trim().eq_ignore_ascii_case("annexb") {
            return None;
        }
        match value.trim().trim_matches('"').to_ascii_lowercase().as_str() {
            "yes" | "true" | "1" => Some(true),
            "no" | "false" | "0" => Some(false),
            _ => None,
        }
    })
}

/// The raw `a=fmtp` parameter string for `payload_type` in the audio stream.
///
/// Returns `None` when the payload type has no `a=fmtp` line, which is
/// meaningful rather than missing data: for AMR it selects every RFC 4867
/// default (bandwidth-efficient framing, all modes).
fn audio_fmtp_params(session: &SdpSession, payload_type: u8) -> Option<String> {
    let format = payload_type.to_string();
    session
        .media_descriptions
        .iter()
        .find(|m| m.media.eq_ignore_ascii_case("audio"))
        .and_then(|m| m.get_fmtp(&format))
        .map(|fmtp| fmtp.parameters.clone())
}

fn audio_fmtp_annex_b(session: &SdpSession, payload_type: u8) -> Option<bool> {
    let format = payload_type.to_string();
    session
        .media_descriptions
        .iter()
        .find(|m| m.media == "audio")
        .and_then(|m| m.get_fmtp(&format))
        .and_then(|fmtp| parse_annex_b_param(&fmtp.parameters))
}

fn negotiated_g729_annex_b(session: &SdpSession, local_annex_b: bool) -> bool {
    local_annex_b && audio_fmtp_annex_b(session, 18).unwrap_or(true)
}

fn select_primary_audio_payload(formats: &[String]) -> Option<u8> {
    formats
        .iter()
        .filter_map(|format| format.parse::<u8>().ok())
        .find(|payload_type| !matches!(*payload_type, 13 | 101))
}

#[cfg(test)]
fn select_primary_audio_payload_from_session(session: &SdpSession) -> Option<u8> {
    session
        .media_descriptions
        .iter()
        .find(|m| m.media == "audio")
        .and_then(|m| select_primary_audio_payload(&m.formats))
}

#[cfg(test)]
fn codec_name_for_payload(payload_type: u8, g729_annex_b: bool) -> String {
    match payload_type {
        0 => "PCMU",
        8 => "PCMA",
        9 => "G722",
        13 => "CN",
        18 if g729_annex_b => "G729BA",
        18 => "G729A",
        101 => "telephone-event",
        AMR_WB_BE_PT | AMR_WB_OA_PT => "AMR-WB",
        AMR_NB_BE_PT | AMR_NB_OA_PT => "AMR",
        111 => "opus",
        _ => return format!("PT{}", payload_type),
    }
    .to_string()
}

fn payload_codec_available(payload_type: u8) -> bool {
    match payload_type {
        0 | 8 | 13 | 101 => true,
        18 => cfg!(feature = "g729"),
        111 => cfg!(feature = "opus"),
        AMR_WB_BE_PT | AMR_WB_OA_PT => cfg!(feature = "amr-wb"),
        AMR_NB_BE_PT | AMR_NB_OA_PT => cfg!(feature = "amr-nb"),
        // G.722 remains wire-parseable but has no encoder/decoder.
        9 => false,
        _ => false,
    }
}

fn sdp_payload_codec_available(session: &SdpSession, payload_type: u8) -> bool {
    let mapping = audio_rtpmap(session, payload_type);
    let mapping_matches = |name: &str, rate: u32, channels: &[u8]| {
        mapping.is_some_and(|mapping| {
            let mapped_channels = mapping
                .encoding_params
                .as_deref()
                .unwrap_or("1")
                .parse::<u8>()
                .ok();
            mapping.encoding_name.eq_ignore_ascii_case(name)
                && mapping.clock_rate == rate
                && mapped_channels.is_some_and(|value| channels.contains(&value))
        })
    };
    match payload_type {
        0 => mapping.is_none() || mapping_matches("PCMU", 8_000, &[1]),
        8 => mapping.is_none() || mapping_matches("PCMA", 8_000, &[1]),
        13 => mapping.is_none() || mapping_matches("CN", 8_000, &[1]),
        18 => cfg!(feature = "g729") && (mapping.is_none() || mapping_matches("G729", 8_000, &[1])),
        // Telephone-event uses a dynamic PT in this stack and therefore
        // always requires an explicit RFC 4733 mapping.
        101 => mapping_matches("telephone-event", 8_000, &[1]),
        // Dynamic payload types carry no fixed meaning, so dispatch on the
        // encoding name the peer declared rather than assuming one codec owns
        // the whole range. This is what lets AMR and Opus coexist above 96.
        96..=127 => {
            mapping_matches("opus", 48_000, &[1, 2]) && cfg!(feature = "opus")
                || mapping_matches("AMR-WB", 16_000, &[1]) && cfg!(feature = "amr-wb")
                || mapping_matches("AMR", 8_000, &[1]) && cfg!(feature = "amr-nb")
        }
        _ => false,
    }
}

fn audio_rtpmap(
    session: &SdpSession,
    payload_type: u8,
) -> Option<&rvoip_sip_core::types::sdp::RtpMapAttribute> {
    session
        .media_descriptions
        .iter()
        .find(|media| media.media.eq_ignore_ascii_case("audio"))?
        .generic_attributes
        .iter()
        .find_map(|attribute| match attribute {
            ParsedAttribute::RtpMap(mapping) if mapping.payload_type == payload_type => {
                Some(mapping)
            }
            _ => None,
        })
}

fn negotiated_audio_shape_from_sdp(
    session: &SdpSession,
    payload_type: u8,
    g729_annex_b: bool,
) -> Result<(String, u32, u8)> {
    let mapping = audio_rtpmap(session, payload_type);
    let (wire_name, clock_rate, channels) = if let Some(mapping) = mapping {
        let channels = mapping
            .encoding_params
            .as_deref()
            .unwrap_or("1")
            .parse::<u8>()
            .map_err(|_| bounded_sdp_failure("codec", "invalid-channels"))?;
        (mapping.encoding_name.as_str(), mapping.clock_rate, channels)
    } else {
        match payload_type {
            0 => ("PCMU", 8_000, 1),
            8 => ("PCMA", 8_000, 1),
            9 => ("G722", 8_000, 1),
            18 => ("G729", 8_000, 1),
            _ => return Err(bounded_sdp_failure("codec", "missing-rtpmap")),
        }
    };

    let canonical = if wire_name.eq_ignore_ascii_case("PCMU") {
        "PCMU"
    } else if wire_name.eq_ignore_ascii_case("PCMA") {
        "PCMA"
    } else if wire_name.eq_ignore_ascii_case("G729") {
        if !cfg!(feature = "g729") {
            return Err(bounded_sdp_failure("codec", "g729-disabled"));
        }
        if g729_annex_b {
            "G729BA"
        } else {
            "G729A"
        }
    } else if wire_name.eq_ignore_ascii_case("opus") {
        if !cfg!(feature = "opus") {
            return Err(bounded_sdp_failure("codec", "opus-disabled"));
        }
        "opus"
    } else if wire_name.eq_ignore_ascii_case("AMR-WB") {
        if !cfg!(feature = "amr-wb") {
            return Err(bounded_sdp_failure("codec", "amr-wb-disabled"));
        }
        "AMR-WB"
    } else if wire_name.eq_ignore_ascii_case("AMR") {
        if !cfg!(feature = "amr-nb") {
            return Err(bounded_sdp_failure("codec", "amr-nb-disabled"));
        }
        "AMR"
    } else if wire_name.eq_ignore_ascii_case("G722") {
        return Err(bounded_sdp_failure("codec", "g722-unsupported"));
    } else {
        return Err(bounded_sdp_failure("codec", "unsupported"));
    };

    let valid_shape = if canonical.eq_ignore_ascii_case("opus") {
        clock_rate == 48_000 && matches!(channels, 1 | 2)
    } else if canonical.eq_ignore_ascii_case("AMR-WB") {
        // AMR-WB is the one 16 kHz codec here. Its clock rate is what
        // distinguishes it from AMR on the wire when both are offered.
        clock_rate == 16_000 && channels == 1
    } else {
        clock_rate == 8_000 && channels == 1
    };
    if !valid_shape {
        return Err(bounded_sdp_failure("codec", "invalid-shape"));
    }
    let valid_payload_identity = match payload_type {
        0 => canonical == "PCMU",
        8 => canonical == "PCMA",
        18 => canonical.starts_with("G729"),
        // Any dynamic payload type may carry any of these; the rtpmap decided
        // which, and the shape check above already validated the clock rate.
        96..=127 if payload_type != 101 => {
            canonical.eq_ignore_ascii_case("opus")
                || canonical.eq_ignore_ascii_case("AMR-WB")
                || canonical.eq_ignore_ascii_case("AMR")
        }
        _ => false,
    };
    if !valid_payload_identity {
        return Err(bounded_sdp_failure("codec", "payload-identity"));
    }

    Ok((canonical.to_string(), clock_rate, channels))
}

fn validate_uac_audio_answer(
    offer: &SdpSession,
    answer: &SdpSession,
    g729_annex_b: bool,
) -> Result<(u8, String, u32, u8)> {
    let offered_audio = offer
        .media_descriptions
        .iter()
        .find(|media| media.media.eq_ignore_ascii_case("audio"))
        .ok_or_else(|| bounded_sdp_failure("remote-answer", "missing-local-audio-offer"))?;
    let answered_audio = answer
        .media_descriptions
        .iter()
        .find(|media| media.media.eq_ignore_ascii_case("audio"))
        .ok_or_else(|| bounded_sdp_failure("remote-answer", "missing-audio"))?;

    let mut primary_payloads = Vec::new();
    for format in &answered_audio.formats {
        if !offered_audio
            .formats
            .iter()
            .any(|offered| offered == format)
        {
            return Err(bounded_sdp_failure("remote-answer", "unoffered-payload"));
        }
        let payload_type = format
            .parse::<u8>()
            .map_err(|_| bounded_sdp_failure("remote-answer", "invalid-payload"))?;
        if !sdp_payload_codec_available(answer, payload_type) {
            return Err(bounded_sdp_failure("remote-answer", "unsupported-payload"));
        }
        if !matches!(payload_type, 13 | 101) {
            // RFC 3264 permits an answer to retain multiple formats from the
            // offer. Validate every dynamic primary payload before choosing
            // the answerer's first (preferred) format for this media session.
            if payload_type >= 96 {
                let offer_map = audio_rtpmap(offer, payload_type)
                    .ok_or_else(|| bounded_sdp_failure("remote-answer", "missing-offer-rtpmap"))?;
                let answer_map = audio_rtpmap(answer, payload_type)
                    .ok_or_else(|| bounded_sdp_failure("remote-answer", "missing-answer-rtpmap"))?;
                if !offer_map
                    .encoding_name
                    .eq_ignore_ascii_case(&answer_map.encoding_name)
                    || offer_map.clock_rate != answer_map.clock_rate
                    || offer_map.encoding_params != answer_map.encoding_params
                {
                    return Err(bounded_sdp_failure(
                        "remote-answer",
                        "changed-dynamic-payload",
                    ));
                }
            }
            primary_payloads.push(payload_type);
        }
    }
    let payload_type = primary_payloads
        .first()
        .copied()
        .ok_or_else(|| bounded_sdp_failure("remote-answer", "missing-primary-payload"))?;
    if !sdp_payload_codec_available(answer, payload_type) {
        return Err(bounded_sdp_failure("remote-answer", "unsupported-payload"));
    }
    let negotiated_annex_b = payload_type == 18 && negotiated_g729_annex_b(answer, g729_annex_b);
    let (codec, clock_rate, channels) =
        negotiated_audio_shape_from_sdp(answer, payload_type, negotiated_annex_b)?;

    Ok((payload_type, codec, clock_rate, channels))
}

fn exact_initial_uac_offer(session: &SessionState) -> Option<&str> {
    session
        .initial_invite_offer_sdp
        .as_deref()
        .or(session.local_sdp.as_deref())
}

/// Build the SDP answer that declines an offered audio m-line per
/// RFC 3264 §6 / RFC 4568 §7.3. Port=0 signals refusal; the proto is
/// echoed from the offer so the peer can distinguish a policy
/// rejection from a parse error. A single dummy `0` format is
/// included because some peers (and some validators) reject m-lines
/// with an empty `<fmt>` list, even though RFC 3264 allows it.
pub(crate) fn build_port_zero_rejection_sdp(
    origin_session_id: &str,
    origin_version: u64,
    local_ip: &str,
    offered_transport: &str,
) -> Result<String> {
    let version_str = origin_version.to_string();
    let session = SdpBuilder::new("Session")
        .origin("-", origin_session_id, &version_str, "IN", "IP4", local_ip)
        .connection("IN", "IP4", local_ip)
        .time("0", "0")
        .media_audio(0, offered_transport)
        .formats(&["0"])
        .done()
        .build()
        .map_err(|_| bounded_sdp_failure("answer-build", "builder"))?;
    Ok(session.to_string())
}

fn unsupported_media_facade(operation: &str) -> SessionError {
    SessionError::InvalidTransition(format!(
        "{operation} is not backed by a media-core implementation; refusing to report fabricated success"
    ))
}

/// Audio format for recording
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum AudioFormat {
    Wav,
    Raw,
    Mp3,
}

/// Recording configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RecordingConfig {
    /// Path where the recording should be saved
    pub file_path: String,

    /// Audio format for the recording
    pub format: AudioFormat,

    /// Sample rate in Hz (e.g., 8000, 16000, 48000)
    pub sample_rate: u32,

    /// Number of channels (1 = mono, 2 = stereo)
    pub channels: u16,

    /// Include mixed audio from both legs (for conference recording)
    pub include_mixed: bool,

    /// Save separate tracks for each leg
    pub separate_tracks: bool,
}

impl Default for RecordingConfig {
    fn default() -> Self {
        Self {
            file_path: "/tmp/recording.wav".to_string(),
            format: AudioFormat::Wav,
            sample_rate: 8000,
            channels: 1,
            include_mixed: false,
            separate_tracks: false,
        }
    }
}

/// Recording status information
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RecordingStatus {
    pub is_recording: bool,
    pub is_paused: bool,
    pub duration_seconds: f64,
    pub file_size_bytes: u64,
}

/// Negotiated media configuration
const fn default_negotiated_clock_rate() -> u32 {
    8_000
}

const fn default_negotiated_channels() -> u8 {
    1
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NegotiatedConfig {
    pub local_addr: SocketAddr,
    pub remote_addr: SocketAddr,
    pub codec: String,
    #[serde(default)]
    pub payload_type: u8,
    #[serde(default = "default_negotiated_clock_rate")]
    pub clock_rate: u32,
    #[serde(default = "default_negotiated_channels")]
    pub channels: u8,
    /// Raw `a=fmtp` parameters agreed for `payload_type`, if the negotiated
    /// SDP carried any.
    ///
    /// Deliberately unparsed. Interpreting format parameters is the codec
    /// layer's job — for AMR they select the wire framing itself
    /// (`octet-align`) and the permitted bit rates (`mode-set`), and a relay
    /// that ignores them frames packets the peer cannot parse. Carrying the
    /// string keeps that knowledge out of the signalling layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub negotiated_fmtp: Option<String>,
    pub local_direction: crate::types::MediaDirection,
    pub remote_direction: crate::types::MediaDirection,
}

/// Validated offer/answer result waiting for its SIP commit boundary.
#[derive(Debug, Clone)]
struct StagedMediaNegotiation {
    config: NegotiatedConfig,
    stable_local_direction: crate::types::MediaDirection,
    srtp_negotiated: bool,
}

/// Exact key for pre-commit media artifacts. Production always uses the
/// generation-qualified registry handle; the raw-ID variant exists only for
/// isolated unit tests that exercise lane-owned helpers without a registry.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum MediaNegotiationKey {
    Exact(SessionRegistryHandle),
    #[cfg(test)]
    Unit(SessionId),
}

/// Reversible lower-media application held across one SIP response or ACK
/// write. Dropping the embedded SRTP token commits its context replacement.
pub(crate) struct PreparedMediaNegotiation {
    session_id: SessionId,
    negotiation_key: MediaNegotiationKey,
    remote_addr: SocketAddr,
    previous_media_security: Option<MediaSecurityState>,
    lower: Option<PreparedLowerMediaNegotiation>,
}

struct PreparedLowerMediaNegotiation {
    exact_media: ExactMediaSession,
    previous_config: MediaConfig,
    srtp_rollback: Option<rvoip_rtp_core::transport::SrtpContextRollback>,
    stable_local_direction: crate::types::MediaDirection,
}

#[derive(Clone)]
struct MediaSessionBinding {
    handle: SessionRegistryHandle,
    dialog_id: DialogId,
    resource: Weak<MediaSessionResource>,
}

/// One generation-qualified live media allocation resolved from the resource
/// owner and canonical registry association. Retaining the strong resource
/// reference prevents a weak binding from disappearing during a lower-layer
/// call; callers revalidate the same exact owner after any await that can race
/// cleanup.
struct ExactMediaSession {
    handle: SessionRegistryHandle,
    dialog_id: DialogId,
    _resource: Arc<MediaSessionResource>,
}

impl MediaSessionBinding {
    fn matches(&self, handle: &SessionRegistryHandle, dialog_id: &DialogId) -> bool {
        self.handle == *handle && self.dialog_id == *dialog_id
    }
}

struct MediaCreateReservationGuard {
    reservations: Arc<DashMap<SessionId, SessionRegistryHandle>>,
    handle: SessionRegistryHandle,
}

impl MediaCreateReservationGuard {
    fn new(
        reservations: Arc<DashMap<SessionId, SessionRegistryHandle>>,
        handle: SessionRegistryHandle,
    ) -> Self {
        Self {
            reservations,
            handle,
        }
    }
}

impl Drop for MediaCreateReservationGuard {
    fn drop(&mut self) {
        self.reservations
            .remove_if(self.handle.session_id(), |_, owner| owner == &self.handle);
    }
}

/// Exact ownership of one media-core allocation and its canonical association.
///
/// The lifecycle authority retains this resource across caller cancellation.
/// Explicit state-machine cleanup and authority teardown both enter the same
/// `OnceCell`, so the lower dialog is stopped exactly once even when terminal
/// events race each other.
#[derive(Clone)]
struct MediaSessionResource {
    controller: Arc<MediaSessionController>,
    store: Weak<SessionStore>,
    handle: SessionRegistryHandle,
    dialog_id: DialogId,
    core_media_allocated: bool,
    create_reservations: Arc<DashMap<SessionId, SessionRegistryHandle>>,
    bindings: Arc<DashMap<SessionId, MediaSessionBinding>>,
    media_sessions: Arc<DashMap<SessionId, MediaSessionInfo>>,
    audio_receivers: Arc<DashMap<SessionId, mpsc::Sender<AudioFrame>>>,
    cleanup_attempt_total: Arc<AtomicU64>,
    cleanup_mapped_total: Arc<AtomicU64>,
    cleanup_media_session_removed_total: Arc<AtomicU64>,
    cleanup_audio_receiver_removed_total: Arc<AtomicU64>,
    released: Arc<tokio::sync::OnceCell<()>>,
}

impl MediaSessionResource {
    fn new(
        adapter: &MediaAdapter,
        handle: SessionRegistryHandle,
        dialog_id: DialogId,
    ) -> Arc<Self> {
        Arc::new(Self {
            controller: Arc::clone(&adapter.controller),
            store: Arc::downgrade(&adapter.store),
            handle,
            dialog_id,
            core_media_allocated: adapter.signaling_only_local_port().is_none(),
            create_reservations: Arc::clone(&adapter.media_create_reservations),
            bindings: Arc::clone(&adapter.media_resources),
            media_sessions: Arc::clone(&adapter.media_sessions),
            audio_receivers: Arc::clone(&adapter.audio_receivers),
            cleanup_attempt_total: Arc::clone(&adapter.cleanup_attempt_total),
            cleanup_mapped_total: Arc::clone(&adapter.cleanup_mapped_total),
            cleanup_media_session_removed_total: Arc::clone(
                &adapter.cleanup_media_session_removed_total,
            ),
            cleanup_audio_receiver_removed_total: Arc::clone(
                &adapter.cleanup_audio_receiver_removed_total,
            ),
            released: Arc::new(tokio::sync::OnceCell::new()),
        })
    }

    /// Release lower-layer media exactly once without publishing session state.
    ///
    /// State-machine actions call this while they own the exact session lane;
    /// their working `SessionState` remains the sole source of the subsequent
    /// canonical store commit. Retained lifecycle teardown calls
    /// [`Self::release_once`], which additionally reconciles store and registry
    /// ownership after the lower resource is gone.
    async fn release_lower_once(&self) -> std::result::Result<(), ManagedResourceReleaseError> {
        self.released
            .get_or_try_init(|| async {
                let session_id = self.handle.session_id();
                let guard = cleanup_diag::stage_guard(CleanupStage::MediaCleanup, &session_id.0);
                self.cleanup_attempt_total.fetch_add(1, Ordering::Relaxed);

                let binding_matches = self
                    .bindings
                    .get(session_id)
                    .is_some_and(|binding| binding.matches(&self.handle, &self.dialog_id));
                if self.core_media_allocated || binding_matches {
                    self.cleanup_mapped_total.fetch_add(1, Ordering::Relaxed);
                }

                if self.core_media_allocated {
                    let _ = self
                        .controller
                        .remove_audio_frame_callback(&self.dialog_id)
                        .await;
                    self.controller
                        .stop_media(&self.dialog_id)
                        .await
                        .map_err(|_| ManagedResourceReleaseError::new("media-stop-failed"))?;
                }

                if self
                    .media_sessions
                    .remove_if(session_id, |_, info| info.dialog_id == self.dialog_id)
                    .is_some()
                {
                    self.cleanup_media_session_removed_total
                        .fetch_add(1, Ordering::Relaxed);
                }
                if binding_matches && self.audio_receivers.remove(session_id).is_some() {
                    self.cleanup_audio_receiver_removed_total
                        .fetch_add(1, Ordering::Relaxed);
                }
                self.bindings.remove_if(session_id, |_, binding| {
                    binding.matches(&self.handle, &self.dialog_id)
                });
                self.create_reservations
                    .remove_if(session_id, |_, owner| owner == &self.handle);

                cleanup_session_diag::record_cleanup();
                tracing::debug!(
                    session_id = %session_id,
                    dialog_id = %self.dialog_id,
                    "released exact media session resource"
                );
                guard.finish_success();
                Ok(())
            })
            .await
            .map(|_| ())
    }

    /// Reconcile retained state only for rollback or quiesced lifecycle
    /// teardown. This deliberately sits outside the lower-release `OnceCell`:
    /// an active lane may already have released the lower allocation and left
    /// the state mutation for its canonical executor commit.
    fn release_retained_ownership(&self) -> std::result::Result<(), ManagedResourceReleaseError> {
        let Some(store) = self.store.upgrade() else {
            return Ok(());
        };
        store
            .clear_media_session_retained_exact(&self.handle, &self.dialog_id)
            .map_err(|_| ManagedResourceReleaseError::new("media-state-release-failed"))?;
        match store
            .registry()
            .clear_media_handle_retained(&self.handle, &self.dialog_id)
        {
            Ok(_)
            | Err(SessionRegistryError::SlotMissing)
            | Err(SessionRegistryError::RevisionMismatch) => Ok(()),
            Err(_) => Err(ManagedResourceReleaseError::new(
                "media-registry-release-failed",
            )),
        }
    }

    async fn release_once(&self) -> std::result::Result<(), ManagedResourceReleaseError> {
        self.release_lower_once().await?;
        self.release_retained_ownership()
    }
}

impl ManagedSessionResource for MediaSessionResource {
    fn descriptor(&self) -> ResourceDescriptor {
        ResourceDescriptor::new("sip-media-session", self.dialog_id.to_string())
    }

    fn cancel(&self) {
        // The retained owned operation observes the lifecycle cancellation
        // signal. Async socket/controller cleanup belongs to `release`.
    }

    fn release(
        &self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = std::result::Result<(), ManagedResourceReleaseError>>
                + Send
                + 'static,
        >,
    > {
        let resource = self.clone();
        Box::pin(async move { resource.release_once().await })
    }
}

async fn rollback_owned_media<T>(
    operation: OwnedOperation,
    value: T,
) -> OwnedOperationCompletion<T> {
    operation
        .rollback(value)
        .await
        .unwrap_or_else(|_| panic!("media allocation exact rollback failed"))
}

async fn rollback_uncommitted_media<T>(
    operation: OwnedOperation,
    installation_sink: ResourceInstallationSink,
    resource: Arc<MediaSessionResource>,
    value: T,
) -> OwnedOperationCompletion<T> {
    if resource.release_once().await.is_ok() {
        installation_sink
            .confirm_unused()
            .unwrap_or_else(|_| panic!("media allocation unused confirmation failed"));
    } else {
        installation_sink
            .capture_at_install(resource as Arc<dyn ManagedSessionResource>)
            .unwrap_or_else(|_| panic!("failed media rollback could not retain exact ownership"));
    }
    rollback_owned_media(operation, value).await
}

/// Minimal media adapter - just translates between media-core and state machine
pub struct MediaAdapter {
    /// Media-core controller
    pub(crate) controller: Arc<MediaSessionController>,

    /// Session store for updating IDs
    pub(crate) store: Arc<SessionStore>,

    /// Private admission fence for media creation. This is not a published
    /// routing map; it only prevents concurrent creates for one exact session
    /// from allocating or rolling back each other's lower dialog.
    media_create_reservations: Arc<DashMap<SessionId, SessionRegistryHandle>>,

    /// Exact lifecycle binding for the managed media resource. Values retain
    /// a weak reference because the authority is the resource owner.
    media_resources: Arc<DashMap<SessionId, MediaSessionBinding>>,

    /// Store media session info for SDP generation
    media_sessions: Arc<DashMap<SessionId, MediaSessionInfo>>,

    /// Audio frame channels for receiving decoded audio from media-core
    audio_receivers: Arc<DashMap<SessionId, mpsc::Sender<AudioFrame>>>,

    /// Perf diagnostics for media lifecycle and active audio churn.
    session_create_attempt_total: Arc<AtomicU64>,
    session_create_success_total: Arc<AtomicU64>,
    session_create_failed_total: Arc<AtomicU64>,
    cleanup_attempt_total: Arc<AtomicU64>,
    cleanup_mapped_total: Arc<AtomicU64>,
    cleanup_fallback_total: Arc<AtomicU64>,
    cleanup_media_session_removed_total: Arc<AtomicU64>,
    cleanup_audio_receiver_removed_total: Arc<AtomicU64>,
    audio_subscriber_created_total: Arc<AtomicU64>,
    audio_subscriber_disconnected_total: Arc<AtomicU64>,
    audio_send_frames_total: Arc<AtomicU64>,
    audio_send_samples_total: Arc<AtomicU64>,

    /// Local IP for SDP generation
    local_ip: IpAddr,

    /// Port range for media
    media_port_start: u16,
    media_port_end: u16,

    /// Whether to allocate media-core sessions or generate SDP only.
    media_mode: MediaMode,

    // ==== RFC 4568 SDES-SRTP state (Step 2B) ====
    /// Whether to attach `a=crypto:` lines to outgoing offers and to
    /// answer with `RTP/SAVP` when peer offers SRTP. When `false`,
    /// the adapter behaves like the pre-2B baseline (plain RTP/AVP).
    offer_srtp: bool,

    /// When `true`, refuse to fall back to plaintext RTP. UAC: a
    /// remote SDP without acceptable `a=crypto:` causes the
    /// negotiation function to return `Err`. UAS: an offer without
    /// `a=crypto:` is rejected with the same `Err`, which the state
    /// machine surfaces as `488 Not Acceptable Here`.
    srtp_required: bool,

    /// Crypto suites to offer in preference order when `offer_srtp`
    /// is set. Default: AES-CM-128 + HMAC-SHA1-80 then -32 per
    /// RFC 4568 §6.2.1 MTI plus low-bandwidth fallback.
    srtp_offered_suites: Vec<CryptoSuite>,

    /// Inbound RFC 4568 key-material Base64 validation policy. This is a
    /// compact immutable adapter setting and is read only during SDP work.
    sdes_base64_mode: SdesBase64Mode,

    /// UAC-side state held between `generate_sdp_offer` and
    /// `negotiate_sdp_as_uac`. The offerer-role `SrtpNegotiator`
    /// holds our locally-generated keys keyed by tag.
    pending_srtp_offerers: Arc<DashMap<MediaNegotiationKey, SrtpNegotiator>>,

    /// Negotiated SRTP context pairs keyed by session. Phase 2B.2
    /// will read these out and hand them to media-core's
    /// `start_secure_media`.
    negotiated_srtp: Arc<DashMap<MediaNegotiationKey, SrtpPair>>,

    /// SDP results that passed validation but have not crossed their exact
    /// SIP offer/answer commit boundary.
    staged_media_negotiations: Arc<DashMap<MediaNegotiationKey, StagedMediaNegotiation>>,

    /// Global event coordinator for publishing RFC 4733 DTMF events
    /// onto the rvoip-sip API event bus. Populated at boot via
    /// [`Self::set_global_coordinator`]; `None` in tests that bypass
    /// the full wiring.
    pub(crate) global_coordinator: Arc<
        tokio::sync::RwLock<
            Option<Arc<rvoip_infra_common::events::coordinator::GlobalEventCoordinator>>,
        >,
    >,

    /// App-level event publisher that updates lifecycle before bus delivery.
    pub(crate) app_event_publisher: Arc<tokio::sync::RwLock<Option<SessionEventPublisher>>>,

    /// Sprint 3 A6 — public RTP-side address advertised in SDP `c=` /
    /// `o=` / `m=audio` lines. Set at coordinator boot from either
    /// `Config::media_public_addr` (static override) or a successful
    /// `Config::stun_server` probe. `None` falls back to `local_ip` +
    /// the per-session local RTP port (today's behaviour).
    public_rtp_addr: std::sync::RwLock<Option<SocketAddr>>,

    /// Sprint 3 C1 — when `true`, generated offers and answers
    /// advertise PT 13 (RFC 3389 Comfort Noise) alongside the
    /// PCMU + PCMA + telephone-event format set. Set at coordinator
    /// boot from `Config::comfort_noise_enabled`.
    comfort_noise_enabled: bool,

    /// Sprint 3.5 C2 swap — when `true` (default), the answer's
    /// format list is the strict RFC 3264 §6 intersection of the
    /// offered formats and our supported set, in offerer-preference
    /// order. When `false`, the answer always advertises our full
    /// supported set (legacy pre-Sprint-3.5 behaviour). Set at
    /// coordinator boot from `Config::strict_codec_matching`.
    strict_codec_matching: bool,

    /// RTP payload types to advertise in outgoing offers and accept on inbound
    /// matching. Default `[0, 8, 101]` (PCMU + PCMA + telephone-event) is the
    /// beta full-media set. Comfort Noise (PT 13) is independently controlled
    /// by `comfort_noise_enabled` for back-compat with existing callers and is
    /// inserted into the list automatically when enabled.
    ///
    /// This low-level adapter can still render SDP metadata for additional
    /// payload types in tests, but `Config::validate` rejects codec sets that
    /// media-core cannot encode/decode for beta full-media operation.
    offered_codecs: Vec<u8>,

    /// G.729 Annex B VAD/DTX/CNG SDP policy. When PT 18 is offered, we emit
    /// `a=fmtp:18 annexb=yes` when this is true and `annexb=no` when false.
    /// Answers disable Annex B when either side advertises `annexb=no`.
    g729_annex_b: bool,

    #[cfg(test)]
    pause_media_create_after_allocation: Arc<AtomicBool>,
    #[cfg(test)]
    media_create_allocated: Arc<tokio::sync::Notify>,
    #[cfg(test)]
    resume_media_create: Arc<tokio::sync::Notify>,
    #[cfg(test)]
    fail_media_commit_after_srtp_swap: Arc<AtomicBool>,
    #[cfg(test)]
    fail_media_rollback: Arc<AtomicBool>,
    #[cfg(test)]
    fail_staged_media_commit: Arc<AtomicBool>,
}

impl MediaAdapter {
    fn media_negotiation_key(session: &SessionState) -> Result<MediaNegotiationKey> {
        if let Some(handle) = session.lifecycle_handle.clone() {
            return Ok(MediaNegotiationKey::Exact(handle));
        }
        #[cfg(test)]
        let missing_exact_authority = Ok(MediaNegotiationKey::Unit(session.session_id.clone()));
        #[cfg(not(test))]
        let missing_exact_authority = Err(SessionError::InvalidTransition(
            "media negotiation requires exact session authority".to_string(),
        ));
        missing_exact_authority
    }

    fn exact_media_negotiation_key(handle: &SessionRegistryHandle) -> MediaNegotiationKey {
        MediaNegotiationKey::Exact(handle.clone())
    }

    pub(crate) fn discard_pending_srtp_offer_for_session(&self, session: &SessionState) {
        if let Ok(key) = Self::media_negotiation_key(session) {
            self.pending_srtp_offerers.remove(&key);
        }
    }

    fn discard_pending_srtp_offer_exact(&self, handle: &SessionRegistryHandle) {
        self.pending_srtp_offerers
            .remove(&Self::exact_media_negotiation_key(handle));
    }

    /// Create a new media adapter (no SRTP — equivalent to the
    /// pre-Step-2B behaviour).
    pub fn new(
        controller: Arc<MediaSessionController>,
        store: Arc<SessionStore>,
        local_ip: IpAddr,
        port_start: u16,
        port_end: u16,
    ) -> Self {
        Self {
            controller,
            store,
            media_create_reservations: Arc::new(DashMap::new()),
            media_resources: Arc::new(DashMap::new()),
            media_sessions: Arc::new(DashMap::new()),
            audio_receivers: Arc::new(DashMap::new()),
            session_create_attempt_total: Arc::new(AtomicU64::new(0)),
            session_create_success_total: Arc::new(AtomicU64::new(0)),
            session_create_failed_total: Arc::new(AtomicU64::new(0)),
            cleanup_attempt_total: Arc::new(AtomicU64::new(0)),
            cleanup_mapped_total: Arc::new(AtomicU64::new(0)),
            cleanup_fallback_total: Arc::new(AtomicU64::new(0)),
            cleanup_media_session_removed_total: Arc::new(AtomicU64::new(0)),
            cleanup_audio_receiver_removed_total: Arc::new(AtomicU64::new(0)),
            audio_subscriber_created_total: Arc::new(AtomicU64::new(0)),
            audio_subscriber_disconnected_total: Arc::new(AtomicU64::new(0)),
            audio_send_frames_total: Arc::new(AtomicU64::new(0)),
            audio_send_samples_total: Arc::new(AtomicU64::new(0)),
            local_ip,
            media_port_start: port_start,
            media_port_end: port_end,
            media_mode: MediaMode::Enabled,
            offer_srtp: false,
            srtp_required: false,
            srtp_offered_suites: vec![
                CryptoSuite::AesCm128HmacSha1_80,
                CryptoSuite::AesCm128HmacSha1_32,
            ],
            sdes_base64_mode: SdesBase64Mode::default(),
            pending_srtp_offerers: Arc::new(DashMap::new()),
            negotiated_srtp: Arc::new(DashMap::new()),
            staged_media_negotiations: Arc::new(DashMap::new()),
            global_coordinator: Arc::new(tokio::sync::RwLock::new(None)),
            app_event_publisher: Arc::new(tokio::sync::RwLock::new(None)),
            public_rtp_addr: std::sync::RwLock::new(None),
            comfort_noise_enabled: false,
            strict_codec_matching: true,
            offered_codecs: vec![0, 8, 101],
            g729_annex_b: true,
            #[cfg(test)]
            pause_media_create_after_allocation: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            media_create_allocated: Arc::new(tokio::sync::Notify::new()),
            #[cfg(test)]
            resume_media_create: Arc::new(tokio::sync::Notify::new()),
            #[cfg(test)]
            fail_media_commit_after_srtp_swap: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_media_rollback: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_staged_media_commit: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Sprint 3 C1 — enable RFC 3389 Comfort Noise advertisement on
    /// outgoing offers and answers. Wired from
    /// `Config::comfort_noise_enabled` at coordinator boot. Mutates
    /// in place, mirroring the `set_srtp_policy` shape.
    ///
    /// Sprint 3.6 follow-up: also propagates the toggle into the
    /// underlying `MediaSessionController` so the VAD-driven
    /// `CnGate` activates on the audio TX path. With this wired,
    /// outbound PCM frames that VAD classifies as silence stop
    /// becoming G.711 packets and are replaced by periodic CN
    /// (PT 13) packets per RFC 3389 §4.1.
    pub fn set_comfort_noise(&mut self, enabled: bool) {
        self.comfort_noise_enabled = enabled;
        self.controller.set_comfort_noise_enabled(enabled);
    }

    /// Set media allocation behavior.
    pub fn set_media_mode(&mut self, mode: MediaMode) {
        self.media_mode = mode;
    }

    /// Resolve the live lower media allocation owned by one exact session
    /// lifetime. Both the resource binding and SessionRegistry must name the
    /// same media identifier; any partial publication or stale raw-ID entry
    /// fails closed.
    fn media_for_handle_exact(&self, handle: &SessionRegistryHandle) -> Option<ExactMediaSession> {
        let registry_media_id = self.store.registry().get_media_handle_exact(handle)?;
        let binding = self
            .media_resources
            .get(handle.session_id())
            .map(|entry| entry.value().clone())?;
        if binding.handle != *handle || binding.dialog_id != registry_media_id {
            return None;
        }
        let resource = binding.resource.upgrade()?;
        if resource.handle != *handle || resource.dialog_id != registry_media_id {
            return None;
        }
        Some(ExactMediaSession {
            handle: handle.clone(),
            dialog_id: registry_media_id,
            _resource: resource,
        })
    }

    /// Resolve only the currently admitted generation for a public raw
    /// SessionId compatibility call.
    fn current_media(&self, session_id: &SessionId) -> Option<ExactMediaSession> {
        let handle = self.store.lifecycle_handle(session_id)?;
        self.media_for_handle_exact(&handle)
    }

    /// Resolve a public raw media identifier only while it remains attached
    /// to one admitted exact session lifetime.
    fn current_media_by_dialog_id(
        &self,
        dialog_id: &crate::types::MediaSessionId,
    ) -> Option<ExactMediaSession> {
        let handle = self.media_resources.iter().find_map(|entry| {
            (&entry.value().dialog_id == dialog_id).then(|| entry.value().handle.clone())
        })?;
        self.media_for_handle_exact(&handle)
    }

    /// Resolve media while the state-machine executor owns this session's
    /// exact lane. The working state's media identity must agree with both
    /// canonical owners before any lower operation can proceed.
    fn lane_owned_media(&self, session: &SessionState) -> Result<ExactMediaSession> {
        let handle = session.lifecycle_handle.as_ref().ok_or_else(|| {
            SessionError::InvalidTransition(
                "media session has no exact lifecycle authority".to_string(),
            )
        })?;
        if handle.session_id() != &session.session_id {
            return Err(SessionError::InvalidTransition(
                "media lifecycle owner does not match its session".to_string(),
            ));
        }
        let exact = self.media_for_handle_exact(handle).ok_or_else(|| {
            SessionError::SessionNotFound(format!(
                "No exact media resource for session {}",
                session.session_id.0
            ))
        })?;
        if session
            .media_session_id
            .as_ref()
            .is_some_and(|media_id| media_id != &exact.dialog_id)
        {
            return Err(SessionError::InvalidTransition(
                "media resource no longer owns the lane state".to_string(),
            ));
        }
        Ok(exact)
    }

    fn media_is_still_exact(&self, expected: &ExactMediaSession) -> bool {
        self.media_for_handle_exact(&expected.handle)
            .is_some_and(|current| {
                current.dialog_id == expected.dialog_id
                    && Arc::ptr_eq(&current._resource, &expected._resource)
            })
    }

    /// Return the current RTP receive packet count for a SIP session, when a
    /// media-core RTP session exists for it.
    pub(crate) async fn rtp_packets_received(&self, session_id: &SessionId) -> Option<u64> {
        let exact = self.current_media(session_id)?;
        let packets = self
            .controller
            .get_session_info(&exact.dialog_id)
            .await
            .and_then(|info| info.rtp_stats.map(|stats| stats.packets_received));
        self.media_is_still_exact(&exact)
            .then_some(packets)
            .flatten()
    }

    /// Sprint 3.5 C2 swap — enable strict RFC 3264 §6 SDP-answer
    /// matching. Wired from `Config::strict_codec_matching` at
    /// coordinator boot.
    pub fn set_strict_codec_matching(&mut self, strict: bool) {
        self.strict_codec_matching = strict;
    }

    /// Set the list of RTP payload types this UA advertises in offers and
    /// accepts in answers. The adapter assumes the caller already ran
    /// `Config::validate`; tests may still use this setter directly for
    /// parser/SDP coverage outside the beta full-media contract.
    pub fn set_offered_codecs(&mut self, codecs: Vec<u8>) {
        self.offered_codecs = codecs;
    }

    /// Set local G.729 Annex B preference for PT 18 SDP offer/answer.
    pub fn set_g729_annex_b(&mut self, enabled: bool) {
        self.g729_annex_b = enabled;
    }

    /// Feature-gated retained-object counts for perf leak investigations.
    #[cfg(feature = "perf-tests")]
    pub(crate) fn perf_diagnostic_counts(&self) -> serde_json::Value {
        let audio_receiver_queue_frames: usize = self
            .audio_receivers
            .iter()
            .map(|entry| {
                let sender = entry.value();
                sender.max_capacity().saturating_sub(sender.capacity())
            })
            .sum();
        let audio_receiver_capacity_frames: usize = self
            .audio_receivers
            .iter()
            .map(|entry| entry.value().max_capacity())
            .sum();
        #[cfg(feature = "perf-media-diagnostics")]
        let controller_diagnostics = self.controller.diagnostic_counts();
        #[cfg(not(feature = "perf-media-diagnostics"))]
        let controller_diagnostics = serde_json::json!({
            "enabled": false,
            "compiled": false,
            "feature": "perf-media-diagnostics",
        });

        serde_json::json!({
            // Preserve compatibility keys at zero after deleting both adapter
            // routing projections. Canonical ownership is reported separately.
            "session_to_dialog": 0,
            "dialog_to_session": 0,
            "registry_media_bindings": self.store.registry().media_mapping_count(),
            "media_create_reservations": self.media_create_reservations.len(),
            "media_resources": self.media_resources.len(),
            "media_sessions": self.media_sessions.len(),
            "audio_receivers": self.audio_receivers.len(),
            "audio_mixers": 0,
            "audio_receiver_queue_frames": audio_receiver_queue_frames,
            "audio_receiver_capacity_frames": audio_receiver_capacity_frames,
            "pending_srtp_offerers": self.pending_srtp_offerers.len(),
            "negotiated_srtp": self.negotiated_srtp.len(),
            "controller": controller_diagnostics,
            "lifecycle": {
                "session_create_attempt_total": self.session_create_attempt_total.load(Ordering::Relaxed),
                "session_create_success_total": self.session_create_success_total.load(Ordering::Relaxed),
                "session_create_failed_total": self.session_create_failed_total.load(Ordering::Relaxed),
                "cleanup_attempt_total": self.cleanup_attempt_total.load(Ordering::Relaxed),
                "cleanup_mapped_total": self.cleanup_mapped_total.load(Ordering::Relaxed),
                "cleanup_fallback_total": self.cleanup_fallback_total.load(Ordering::Relaxed),
                "cleanup_media_session_removed_total": self.cleanup_media_session_removed_total.load(Ordering::Relaxed),
                "cleanup_audio_receiver_removed_total": self.cleanup_audio_receiver_removed_total.load(Ordering::Relaxed),
                "audio_subscriber_created_total": self.audio_subscriber_created_total.load(Ordering::Relaxed),
                "audio_subscriber_disconnected_total": self.audio_subscriber_disconnected_total.load(Ordering::Relaxed),
            },
            "audio_send": {
                "frames_total": self.audio_send_frames_total.load(Ordering::Relaxed),
                "samples_total": self.audio_send_samples_total.load(Ordering::Relaxed),
            },
        })
    }

    /// Compose the effective offer-format list: configured
    /// `offered_codecs` with comfort-noise (PT 13) inserted in front
    /// of DTMF (PT 101) when enabled, preserving the legacy ordering
    /// the byte-fixture tests pin.
    fn effective_offered_formats(&self) -> Vec<u8> {
        let offered: Vec<u8> = self
            .offered_codecs
            .iter()
            .copied()
            .filter(|payload_type| payload_codec_available(*payload_type))
            .collect();
        if !self.comfort_noise_enabled {
            return offered;
        }
        let mut out = Vec::with_capacity(offered.len() + 1);
        for pt in &offered {
            if *pt == 101 {
                out.push(13);
            }
            out.push(*pt);
        }
        if !out.contains(&13) {
            // No DTMF in the list — append CN at the end.
            out.push(13);
        }
        out
    }

    /// Commit one negotiated media generation to the media layer.
    ///
    /// `negotiated_fmtp` is the peer's `a=fmtp` parameter string for the
    /// primary audio payload type, carried verbatim and uninterpreted — the
    /// signalling layer has no business parsing it, but losing it is not
    /// neutral either. For AMR those parameters select the wire framing
    /// itself (RFC 4867 §8.3.1), so a relay that never sees them will forward
    /// bytes the far end cannot parse. `None` means the peer sent no usable
    /// `a=fmtp` line, and is passed through as such rather than flattened to
    /// an empty string: the configuration below is seeded from the previous
    /// generation, so `None` has to clear what a previous negotiation left.
    async fn apply_negotiated_media_config(
        &self,
        dialog_id: &DialogId,
        remote_addr: SocketAddr,
        codec: &str,
        payload_type: u8,
        clock_rate: u32,
        channels: u8,
        negotiated_fmtp: Option<&str>,
    ) -> Result<()> {
        let mut config = self
            .controller
            .get_session_info(dialog_id)
            .await
            .ok_or_else(|| {
                SessionError::MediaError(format!("No media session for dialog {}", dialog_id))
            })?
            .config;
        config.remote_addr = Some(remote_addr);
        config = config
            .with_negotiated_audio_codec(codec.to_string(), payload_type, clock_rate, channels)
            .with_negotiated_fmtp(negotiated_fmtp);

        self.controller
            .update_media(dialog_id.clone(), config)
            .await
            .map_err(|e| {
                SessionError::MediaError(format!("Failed to apply negotiated media config: {}", e))
            })
    }

    /// Set the public RTP address advertised in SDP. Called at
    /// coordinator boot from `Config::media_public_addr` (static
    /// override) or a successful STUN probe. Idempotent — subsequent
    /// calls overwrite. The IP address goes into `c=`/`o=` lines and
    /// the port (when set) replaces `info.rtp_port` in `m=audio`.
    pub fn set_public_rtp_addr(&self, addr: Option<SocketAddr>) {
        if let Ok(mut guard) = self.public_rtp_addr.write() {
            *guard = addr;
        }
    }

    /// Read the current public RTP address override (used by tests
    /// and by SDP generation).
    pub(crate) fn public_rtp_addr(&self) -> Option<SocketAddr> {
        self.public_rtp_addr.read().ok().and_then(|g| *g)
    }

    /// Local IP address bound by the adapter. Used by the Sprint 3
    /// A6 STUN probe to bind its temp socket on the same interface.
    pub fn local_ip(&self) -> IpAddr {
        self.local_ip
    }

    /// Install the global event coordinator so the adapter can publish
    /// RFC 4733 DTMF events onto the rvoip-sip API event stream.
    /// Idempotent — a later call replaces any prior coordinator.
    pub async fn set_global_coordinator(
        &self,
        coordinator: Arc<rvoip_infra_common::events::coordinator::GlobalEventCoordinator>,
    ) {
        *self.global_coordinator.write().await = Some(coordinator.clone());
        let mut publisher = self.app_event_publisher.write().await;
        if publisher.is_none() {
            *publisher = Some(SessionEventPublisher::new(
                coordinator,
                LifecycleIndex::new(),
            ));
        }
    }

    /// Install the app-level event publisher. This is preferred over direct
    /// global-coordinator publication because it updates lifecycle first.
    pub(crate) async fn set_app_event_publisher(&self, publisher: SessionEventPublisher) {
        *self.app_event_publisher.write().await = Some(publisher);
    }

    /// Acquire the exact session execution lane used by public MediaAdapter
    /// compatibility methods and reload state only after the wait. Production
    /// state-machine actions already own this lane and call the corresponding
    /// `*_lane_owned` methods instead.
    async fn lock_and_load_exact_media_session(
        &self,
        session_id: &SessionId,
    ) -> Result<(
        tokio::sync::OwnedMutexGuard<()>,
        Arc<SessionStateSnapshot>,
        SessionState,
    )> {
        let (handle, lane) = self.store.state_machine_lane(session_id).ok_or_else(|| {
            SessionError::SessionNotFound(format!("No session state for media {}", session_id.0))
        })?;
        let guard = lane.lock_owned().await;
        let snapshot = self
            .store
            .get_session_snapshot_exact(&handle)
            .map_err(|_| {
                SessionError::SessionNotFound(format!(
                    "Media session lifetime is no longer current for {}",
                    session_id.0
                ))
            })?;
        let session = snapshot.state().clone();
        Ok((guard, snapshot, session))
    }

    async fn commit_media_lane_state(
        &self,
        snapshot: &SessionStateSnapshot,
        session: &SessionState,
    ) -> Result<()> {
        let session_id = session.session_id.clone();
        let origin_session_id = session.sdp_origin_session_id.clone();
        let origin_version = session.sdp_origin_version;
        let media_security = session.media_security.clone();
        let committed = self
            .store
            .update_session_snapshot_with(snapshot, move |current| {
                current.sdp_origin_session_id = origin_session_id;
                current.sdp_origin_version = origin_version;
                current.media_security = media_security;
                true
            })
            .await
            .map_err(|_| {
                SessionError::InvalidTransition(
                    "exact media session state could not be committed".to_string(),
                )
            })?;
        committed.then_some(()).ok_or_else(|| {
            SessionError::InvalidTransition(format!(
                "exact media session lifetime changed before commit for {}",
                session_id.0
            ))
        })
    }

    /// Configure the SRTP offer policy. Called by `UnifiedCoordinator`
    /// when constructing the adapter from a [`Config`](crate::api::unified::Config) that has
    /// `offer_srtp` / `srtp_required` / `srtp_offered_suites` set.
    /// Mutates in place rather than returning a new adapter so the
    /// existing constructor signature stays unchanged.
    pub fn set_srtp_policy(
        &mut self,
        offer_srtp: bool,
        srtp_required: bool,
        suites: Vec<CryptoSuite>,
    ) {
        self.offer_srtp = offer_srtp;
        self.srtp_required = srtp_required;
        if !suites.is_empty() {
            self.srtp_offered_suites = suites;
        }
    }

    /// Configure how inbound SDES inline keys handle trailing Base64 padding.
    pub fn set_sdes_base64_mode(&mut self, mode: SdesBase64Mode) {
        self.sdes_base64_mode = mode;
    }

    // ===== Outbound Actions (from state machine) =====

    /// Start a media session
    pub async fn start_session(&self, session_id: &SessionId) -> Result<()> {
        if let Some(exact) = self.current_media(session_id) {
            if self.signaling_only_local_port().is_some() {
                tracing::debug!("Media session already started for session {}", session_id.0);
                return Ok(());
            }
            let started = self
                .controller
                .get_session_info(&exact.dialog_id)
                .await
                .is_some();
            if !self.media_is_still_exact(&exact) {
                return Err(SessionError::SessionNotFound(format!(
                    "Media session lifetime changed for {}",
                    session_id.0
                )));
            }
            if started {
                tracing::debug!("Media session already started for session {}", session_id.0);
                return Ok(());
            }
            return Err(SessionError::MediaError(
                "Exact media resource has no lower media-core allocation".to_string(),
            ));
        }
        if self.media_resources.contains_key(session_id) {
            return Err(SessionError::InvalidTransition(
                "A stale or partially published media resource blocks creation".to_string(),
            ));
        }

        let _media_id = self.create_session(session_id).await?;
        Ok(())
    }

    /// Generate SDP offer (for UAC).
    ///
    /// Built via `sip-core`'s typed `SdpBuilder` (RFC 8866). The
    /// previous format-string implementation produced byte-identical
    /// output to this version when the `offer_srtp` knob is not set —
    /// Extract the `a=crypto:` attributes from the audio m-section of
    /// a parsed SDP. Empty result means the peer offered no SRTP.
    fn extract_audio_crypto(session: &SdpSession) -> Vec<CryptoAttribute> {
        session
            .media_descriptions
            .iter()
            .find(|m| m.media == "audio")
            .map(|m| {
                m.generic_attributes
                    .iter()
                    .filter_map(|a| match a {
                        ParsedAttribute::Crypto(c) => Some(c.clone()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Process SDP answer and negotiate (for UAC)
    pub async fn negotiate_sdp_as_uac(
        &self,
        session_id: &SessionId,
        remote_sdp: &str,
    ) -> Result<NegotiatedConfig> {
        // Preserve bounded syntax/connection diagnostics before resolving the
        // session, then serialize every state mutation with the exact lane.
        self.parse_sdp_connection(remote_sdp)?;
        let (lane, snapshot, mut session) =
            self.lock_and_load_exact_media_session(session_id).await?;
        let previous_security = session.media_security.clone();
        let result = match self
            .negotiate_sdp_as_uac_lane_owned(&mut session, remote_sdp)
            .await
        {
            Ok(config) => self
                .commit_staged_media_negotiation_lane_owned(&mut session)
                .await
                .map(|()| config),
            Err(error) => Err(into_public_negotiation_error(error)),
        };
        if result.is_err() {
            self.discard_staged_media_negotiation_for_session(&session);
        }
        let security_changed = result.is_ok() && session.media_security != previous_security;
        let security_observation = security_changed
            .then(|| {
                session
                    .lifecycle_handle
                    .clone()
                    .zip(session.media_security.clone())
            })
            .flatten();
        if security_changed {
            // The public compatibility commit acquires the same non-reentrant
            // lane and revision-checks this snapshot. Release our exact guard
            // first; any intervening signaling event makes the commit fail
            // stale instead of merging two writers.
            drop(lane);
            self.commit_media_lane_state(&snapshot, &session).await?;
            if let Some((lifecycle_handle, state)) = security_observation {
                self.publish_media_security_observation(lifecycle_handle, state);
            }
        }
        result
    }

    pub(crate) async fn negotiate_sdp_as_uac_lane_owned(
        &self,
        session: &mut SessionState,
        remote_sdp: &str,
    ) -> SrtpDetailedResult<NegotiatedConfig> {
        let session_id = session.session_id.clone();
        let negotiation_key = Self::media_negotiation_key(session)?;
        // Parse remote SDP to extract IP and port
        let (remote_ip, remote_port) = self.parse_sdp_connection(remote_sdp)?;
        let parsed_answer = SdpSession::from_str(remote_sdp)
            .map_err(|_| bounded_sdp_failure("remote-answer", "syntax"))?;
        let parsed_offer = exact_initial_uac_offer(session)
            .ok_or_else(|| bounded_sdp_failure("remote-answer", "missing-local-offer"))
            .and_then(|offer| {
                SdpSession::from_str(offer)
                    .map_err(|_| bounded_sdp_failure("remote-answer", "invalid-local-offer"))
            })?;
        let (payload_type, negotiated_codec, clock_rate, channels) =
            validate_uac_audio_answer(&parsed_offer, &parsed_answer, self.g729_annex_b)?;
        let answer_direction = audio_direction(&parsed_answer);
        let srtp_diagnostics = srtp_diagnostics_enabled();
        if sdp_diagnostics_enabled() {
            emit_sdp_diag(format!(
                "remote_sdp_answer session={} media={}:{} transport={} {}",
                session_id.0,
                remote_ip,
                remote_port,
                audio_transport(&parsed_answer).unwrap_or("unknown"),
                crypto_attribute_diag(Self::extract_audio_crypto(&parsed_answer).len())
            ));
        }

        // Validate the exact lower owner while leaving address, codec,
        // direction and SRTP contexts at their last stable values. The answer
        // has not crossed its ACK/response commit boundary yet.
        let signaling_only_port = self.signaling_only_local_port();
        if signaling_only_port.is_none() {
            let exact_media = self.lane_owned_media(session)?;
            if self
                .controller
                .get_session_info(&exact_media.dialog_id)
                .await
                .is_none()
                || !self.media_is_still_exact(&exact_media)
            {
                return Err(SessionError::InvalidTransition(
                    "media resource changed during UAC negotiation validation".to_string(),
                )
                .into());
            }
        }

        // SDES answer-side handling (RFC 4568 §7.5). Keep the offerer's
        // generated key state available until the SIP/media commit boundary:
        // malformed answers and lower-media races must be retryable without
        // silently falling back to plaintext.
        let offered_crypto = Self::extract_audio_crypto(&parsed_offer);
        let answer_crypto = Self::extract_audio_crypto(&parsed_answer);
        let negotiated_srtp_pair = {
            let offerer_state = self.pending_srtp_offerers.get(&negotiation_key);
            match (offered_crypto.is_empty(), offerer_state.as_ref()) {
                (false, None) => {
                    return Err(SessionError::SDPNegotiationFailed(
                        "the local SDP offered SRTP but its pending SDES key state is unavailable"
                            .into(),
                    )
                    .into());
                }
                (true, Some(_)) => {
                    return Err(SessionError::SDPNegotiationFailed(
                        "pending SDES key state does not match the local SDP offer".into(),
                    )
                    .into());
                }
                (true, None) => {
                    if !answer_crypto.is_empty() {
                        return Err(SessionError::SDPNegotiationFailed(
                            "the SDP answer selected SRTP that was not offered".into(),
                        )
                        .into());
                    }
                    if self.srtp_required {
                        return Err(SessionError::SDPNegotiationFailed(
                            "srtp_required is set but the local SDP did not offer SRTP".into(),
                        )
                        .into());
                    }
                    None
                }
                (false, Some(offerer_state)) => {
                    if answer_crypto.len() > 1 {
                        return Err(SessionError::SDPNegotiationFailed(
                            "the SDP answer selected more than one a=crypto attribute".into(),
                        )
                        .into());
                    }
                    if let Some(chosen) = answer_crypto.first() {
                        let pair = offerer_state.accept_answer_detailed(chosen)?;
                        tracing::info!(
                            "SDES answer accepted for session {}: tag {} suite {:?}",
                            session_id.0,
                            chosen.tag,
                            chosen.suite
                        );
                        if srtp_diagnostics {
                            emit_srtp_diag(format!(
                                "sdes_answer_accepted session={} suite={:?}",
                                session_id.0, chosen.suite
                            ));
                        }
                        Some(pair)
                    } else if self.srtp_required {
                        return Err(SessionError::SDPNegotiationFailed(
                            "srtp_required is set but the SDP answer carries no a=crypto: line"
                                .into(),
                        )
                        .into());
                    } else {
                        tracing::warn!(
                            "Session {} offered SRTP but the answer didn't accept it; \
                             proceeding plaintext (Config::srtp_required = false)",
                            session_id.0
                        );
                        None
                    }
                }
            }
        };

        let local_port = match signaling_only_port {
            Some(port) => port,
            None => self.get_local_port(&session_id)?,
        };
        let config = NegotiatedConfig {
            local_addr: SocketAddr::new(self.local_ip, local_port),
            remote_addr: SocketAddr::new(remote_ip, remote_port),
            codec: negotiated_codec,
            payload_type,
            clock_rate,
            channels,
            negotiated_fmtp: audio_fmtp_params(&parsed_answer, payload_type),
            local_direction: local_direction_from_remote_answer(&answer_direction),
            remote_direction: answer_direction
                .map(sip_direction_to_session)
                .unwrap_or(crate::types::MediaDirection::SendRecv),
        };

        let srtp_negotiated = negotiated_srtp_pair.is_some();
        if let Some(pair) = negotiated_srtp_pair {
            self.negotiated_srtp.insert(negotiation_key.clone(), pair);
        }
        self.staged_media_negotiations.insert(
            negotiation_key,
            StagedMediaNegotiation {
                config: config.clone(),
                stable_local_direction: session.local_media_direction,
                srtp_negotiated,
            },
        );

        // Event publishing will be handled by SessionCrossCrateEventHandler

        Ok(config)
    }

    /// Validate a locally supplied offer before an in-dialog request reaches
    /// the wire. Codec agreement is still determined by the remote answer.
    pub(crate) fn validate_local_sdp_offer(&self, local_sdp: &str) -> Result<()> {
        let parsed_offer = SdpSession::from_str(local_sdp)
            .map_err(|_| bounded_sdp_failure("local-offer", "syntax"))?;
        self.parse_sdp_connection(local_sdp)?;
        if !parsed_offer
            .media_descriptions
            .iter()
            .any(|media| media.media == "audio" && !media.formats.is_empty())
        {
            return Err(SessionError::SDPNegotiationFailed(
                "local SDP offer has no usable audio media description".to_string(),
            ));
        }
        Ok(())
    }

    /// Validate an inbound re-INVITE/UPDATE offer without mutating media or
    /// session state, allowing malformed offers to receive an exact 488.
    pub(crate) fn validate_inbound_sdp_offer(&self, remote_sdp: &str) -> SrtpDetailedResult<()> {
        let parsed_offer = SdpSession::from_str(remote_sdp)
            .map_err(|_| bounded_sdp_failure("remote-offer", "syntax"))?;
        self.parse_sdp_connection(remote_sdp)?;

        let offered_crypto = Self::extract_audio_crypto(&parsed_offer);
        if offered_crypto.is_empty() && self.srtp_required {
            return Err(SessionError::SDPNegotiationFailed(
                "srtp_required is set but the SDP offer carries no a=crypto: line".into(),
            )
            .into());
        }
        if !offered_crypto.is_empty() && !self.offer_srtp {
            return Ok(());
        }
        if !offered_crypto.is_empty() {
            SrtpNegotiator::new_answerer_with_base64_mode(self.sdes_base64_mode)
                .validate_offer_detailed(&offered_crypto)?;
        }
        compute_answer_formats(
            &parsed_offer,
            &self.effective_offered_formats(),
            self.strict_codec_matching,
            self.offer_srtp,
            self.srtp_required,
        )?;
        Ok(())
    }

    /// Generate SDP answer and negotiate (for UAS)
    pub async fn negotiate_sdp_as_uas(
        &self,
        session_id: &SessionId,
        remote_sdp: &str,
    ) -> Result<(String, NegotiatedConfig)> {
        // Keep malformed-offer failures independent of session lookup, as the
        // historical public facade did, then enter the exact execution lane.
        SdpSession::from_str(remote_sdp)
            .map_err(|_| bounded_sdp_failure("remote-offer", "syntax"))?;
        let (lane, snapshot, mut session) =
            self.lock_and_load_exact_media_session(session_id).await?;
        let previous_origin = (
            session.sdp_origin_session_id.clone(),
            session.sdp_origin_version,
        );
        let previous_security = session.media_security.clone();
        let result = match self
            .negotiate_sdp_as_uas_lane_owned(&mut session, remote_sdp)
            .await
        {
            Ok((answer, config)) => self
                .commit_staged_media_negotiation_lane_owned(&mut session)
                .await
                .map(|()| (answer, config)),
            Err(error) => Err(into_public_negotiation_error(error)),
        };
        if result.is_err() {
            self.discard_staged_media_negotiation_for_session(&session);
        }
        let state_changed = result.is_ok()
            && (previous_origin
                != (
                    session.sdp_origin_session_id.clone(),
                    session.sdp_origin_version,
                )
                || session.media_security != previous_security);
        let security_observation = (result.is_ok() && session.media_security != previous_security)
            .then(|| {
                session
                    .lifecycle_handle
                    .clone()
                    .zip(session.media_security.clone())
            })
            .flatten();
        if state_changed {
            drop(lane);
            self.commit_media_lane_state(&snapshot, &session).await?;
            if let Some((lifecycle_handle, state)) = security_observation {
                self.publish_media_security_observation(lifecycle_handle, state);
            }
        }
        result
    }

    pub(crate) async fn negotiate_sdp_as_uas_lane_owned(
        &self,
        session: &mut SessionState,
        remote_sdp: &str,
    ) -> SrtpDetailedResult<(String, NegotiatedConfig)> {
        let session_id = session.session_id.clone();
        let negotiation_key = Self::media_negotiation_key(session)?;
        let stable_local_direction = session.local_media_direction;
        // Parse remote SDP — typed parse for both connection extraction
        // and SDES handling.
        let parsed_offer = SdpSession::from_str(remote_sdp)
            .map_err(|_| bounded_sdp_failure("remote-offer", "syntax"))?;
        let (remote_ip, remote_port) = self.parse_sdp_connection(remote_sdp)?;
        let srtp_diagnostics = srtp_diagnostics_enabled();
        if sdp_diagnostics_enabled() {
            emit_sdp_diag(format!(
                "remote_sdp_offer session={} media={}:{} transport={} {}",
                session_id.0,
                remote_ip,
                remote_port,
                audio_transport(&parsed_offer).unwrap_or("unknown"),
                crypto_attribute_diag(Self::extract_audio_crypto(&parsed_offer).len())
            ));
        }

        // SDES UAS-side handling. Per RFC 4568 §7.3, if we require
        // SRTP and the offer doesn't include any `a=crypto:` lines we
        // must reject — the state-machine path turns the
        // `SDPNegotiationFailed` into a `488 Not Acceptable Here`
        // (decision D10).
        let offered_crypto = Self::extract_audio_crypto(&parsed_offer);
        let offered_transport = audio_transport(&parsed_offer)
            .unwrap_or("RTP/AVP")
            .to_string();
        let (answer_attr, negotiated_srtp_pair, reject_with_port_zero) =
            if !offered_crypto.is_empty() && self.offer_srtp {
                // Both sides want SRTP — negotiate.
                let answerer = SrtpNegotiator::new_answerer_with_base64_mode(self.sdes_base64_mode);
                let (chosen, pair) = answerer.process_offer_detailed(&offered_crypto)?;
                tracing::info!(
                    "SDES offer provisionally accepted for session {}: tag {} suite {:?}",
                    session_id.0,
                    chosen.tag,
                    chosen.suite
                );
                if srtp_diagnostics {
                    emit_srtp_diag(format!(
                        "sdes_offer_accepted session={} suite={:?}",
                        session_id.0, chosen.suite
                    ));
                }
                (Some(chosen), Some(pair), false)
            } else if offered_crypto.is_empty() && self.srtp_required {
                return Err(SessionError::SDPNegotiationFailed(
                    "srtp_required is set but the SDP offer carries no a=crypto: line".into(),
                )
                .into());
            } else if !offered_crypto.is_empty() && !self.offer_srtp {
                // RFC 3264 §6 + RFC 4568 §7.3: peer offered SRTP but our
                // policy is plaintext. Reject the m-line by setting port=0
                // in the answer, preserving the offered proto so the peer
                // can distinguish a rejection from a parse error.
                tracing::info!(
                    "Session {} received SRTP offer but local policy is offer_srtp=false; \
                 rejecting m-line with port=0 per RFC 3264 §6",
                    session_id.0
                );
                if srtp_diagnostics {
                    emit_srtp_diag(format!(
                        "sdes_offer_rejected session={} reason=local_policy",
                        session_id.0
                    ));
                }
                (None, None, true)
            } else {
                (None, None, false)
            };

        // Port-zero rejection short-circuit: build a minimal RFC 3264
        // §6 declined m-line answer and return without setting up any
        // media flow. The peer is responsible for either re-offering
        // (with plaintext) or terminating the dialog.
        if reject_with_port_zero {
            let (origin_session_id, origin_version) = advance_sdp_origin(session);
            let advertised_ip = self
                .public_rtp_addr()
                .map(|sa| sa.ip())
                .unwrap_or(self.local_ip);
            let sdp_answer = build_port_zero_rejection_sdp(
                &origin_session_id,
                origin_version,
                &advertised_ip.to_string(),
                offered_transport.as_str(),
            )?;
            let config = NegotiatedConfig {
                local_addr: SocketAddr::new(advertised_ip, 0),
                remote_addr: SocketAddr::new(remote_ip, remote_port),
                codec: "PCMU".to_string(),
                payload_type: 0,
                clock_rate: 8_000,
                channels: 1,
                // A port-zero rejection carries no media, so no format
                // parameters apply.
                negotiated_fmtp: None,
                local_direction: crate::types::MediaDirection::Inactive,
                remote_direction: crate::types::MediaDirection::Inactive,
            };
            self.staged_media_negotiations.insert(
                negotiation_key,
                StagedMediaNegotiation {
                    config: config.clone(),
                    stable_local_direction,
                    srtp_negotiated: false,
                },
            );
            return Ok((sdp_answer, config));
        }

        let signaling_only_port = self.signaling_only_local_port();
        let local_port = match signaling_only_port {
            Some(port) => port,
            None => self.get_local_port(&session_id)?,
        };

        let formats = compute_answer_formats(
            &parsed_offer,
            &self.effective_offered_formats(),
            self.strict_codec_matching,
            self.offer_srtp,
            self.srtp_required,
        )?;
        let negotiated_payload_type = select_primary_audio_payload(&formats)
            .ok_or_else(|| bounded_sdp_failure("remote-offer", "missing-primary-payload"))?;
        let negotiated_annex_b = if negotiated_payload_type == 18 {
            negotiated_g729_annex_b(&parsed_offer, self.g729_annex_b)
        } else {
            false
        };
        let (negotiated_codec, clock_rate, channels) = negotiated_audio_shape_from_sdp(
            &parsed_offer,
            negotiated_payload_type,
            negotiated_annex_b,
        )?;
        let offered_direction = audio_direction(&parsed_offer);
        let answer_direction = answer_direction_for_offer(&offered_direction);

        // Validate the exact lower owner without mutating it. The generated
        // answer is still pre-wire state and must be retryable after a
        // definite zero-wire response failure.
        let exact_media = if signaling_only_port.is_some() {
            None
        } else {
            Some(self.lane_owned_media(session)?)
        };
        if let Some(exact_media) = exact_media {
            if self
                .controller
                .get_session_info(&exact_media.dialog_id)
                .await
                .is_none()
                || !self.media_is_still_exact(&exact_media)
            {
                return Err(SessionError::InvalidTransition(
                    "media resource changed during UAS negotiation validation".to_string(),
                )
                .into());
            }
        }

        // Generate the SDP answer.
        //
        // Sprint 3.5 — `negotiate_sdp_as_uas` now consumes the
        // generic RFC 3264 §6 matcher in
        // `rvoip_sip_dialog::sdp::match_offer`. The strict path
        // (default) additionally applies transport policy; both modes
        // intersect the offer with our implemented codecs and retain one
        // primary codec plus auxiliary CN/telephone-event payloads.
        let (origin_session_id, origin_version) = advance_sdp_origin(session);
        let origin_version = origin_version.to_string();
        // Sprint 3 A6 — same public-address override as the offer
        // path, so answers carry the discovered/configured public
        // mapping when one is set.
        let public = self.public_rtp_addr();
        let advertised_ip = public.map(|sa| sa.ip()).unwrap_or(self.local_ip);
        let local_ip_str = advertised_ip.to_string();
        let advertised_port = public
            .filter(|sa| sa.port() != 0)
            .map(|sa| sa.port())
            .unwrap_or(local_port);
        let answer_transport = if answer_attr.is_some() {
            "RTP/SAVP"
        } else {
            "RTP/AVP"
        };

        if sdp_diagnostics_enabled() {
            emit_sdp_diag(format!(
                "local_sdp_answer session={} media={}:{} transport={} {} direction={}",
                session_id.0,
                advertised_ip,
                advertised_port,
                answer_transport,
                crypto_attribute_diag(usize::from(answer_attr.is_some())),
                direction_attribute(answer_direction)
            ));
        }

        let formats_str: Vec<&str> = formats.iter().map(|s| s.as_str()).collect();
        let mut media_builder = SdpBuilder::new("Session")
            .origin(
                "-",
                &origin_session_id,
                &origin_version,
                "IN",
                "IP4",
                &local_ip_str,
            )
            .connection("IN", "IP4", &local_ip_str)
            .time("0", "0")
            .media_audio(advertised_port, answer_transport)
            .formats(&formats_str);
        // Emit rtpmap/fmtp ONLY for the one primary and any auxiliary
        // formats retained by the validated intersection.
        // NEXT_STEPS C2 — routed through `rtpmap_for_pt` so adding a
        // new codec only requires extending the helper.
        for fmt in &formats {
            let Ok(pt) = fmt.parse::<u8>() else {
                continue;
            };
            if pt == negotiated_payload_type && negotiated_codec.eq_ignore_ascii_case("opus") {
                let rtpmap = format!("opus/{clock_rate}/{channels}");
                media_builder = media_builder.rtpmap(fmt.as_str(), rtpmap.as_str());
            } else if let Some(rtpmap) = rtpmap_for_pt(pt) {
                media_builder = media_builder.rtpmap(fmt.as_str(), rtpmap);
            }
            if let Some(fmtp) = answer_fmtp_for_pt(&parsed_offer, pt, negotiated_annex_b) {
                media_builder = media_builder.fmtp(fmt.as_str(), fmtp.as_str());
            }
        }
        if let Some(attr) = answer_attr {
            media_builder = media_builder.crypto_attribute(attr);
        }
        let session = media_builder
            .attribute(direction_attribute(answer_direction), None::<String>)
            .done()
            .build()
            .map_err(|_| bounded_sdp_failure("answer-build", "builder"))?;
        let sdp_answer = session.to_string();

        let config = NegotiatedConfig {
            local_addr: SocketAddr::new(self.local_ip, local_port),
            remote_addr: SocketAddr::new(remote_ip, remote_port),
            codec: negotiated_codec,
            payload_type: negotiated_payload_type,
            clock_rate,
            channels,
            // RFC 4867 §8.3.1 requires the answerer to echo the transport
            // parameters unmodified, so the offer is the authority here.
            // Reading back our own answer would be circular.
            negotiated_fmtp: audio_fmtp_params(&parsed_offer, negotiated_payload_type),
            local_direction: answer_direction,
            remote_direction: offered_direction
                .map(sip_direction_to_session)
                .unwrap_or(crate::types::MediaDirection::SendRecv),
        };

        let srtp_negotiated = negotiated_srtp_pair.is_some();
        if let Some(pair) = negotiated_srtp_pair {
            self.negotiated_srtp.insert(negotiation_key.clone(), pair);
        }
        self.staged_media_negotiations.insert(
            negotiation_key,
            StagedMediaNegotiation {
                config: config.clone(),
                stable_local_direction,
                srtp_negotiated,
            },
        );

        // Event publishing will be handled by SessionCrossCrateEventHandler

        // Media flow is already represented by MediaStreamStarted above

        Ok((sdp_answer, config))
    }

    /// Drop every pre-commit media artifact for an offer/answer exchange.
    pub(crate) fn discard_staged_media_negotiation_for_session(&self, session: &SessionState) {
        if let Ok(key) = Self::media_negotiation_key(session) {
            self.staged_media_negotiations.remove(&key);
            self.negotiated_srtp.remove(&key);
        }
    }

    fn discard_staged_media_negotiation_exact(&self, handle: &SessionRegistryHandle) {
        let key = Self::exact_media_negotiation_key(handle);
        self.staged_media_negotiations.remove(&key);
        self.negotiated_srtp.remove(&key);
    }

    pub(crate) fn has_staged_media_negotiation(&self, session: &SessionState) -> bool {
        Self::media_negotiation_key(session)
            .is_ok_and(|key| self.staged_media_negotiations.contains_key(&key))
    }

    #[cfg(test)]
    pub(crate) fn fail_next_staged_media_commit_for_test(&self) {
        self.fail_staged_media_commit.store(true, Ordering::Release);
    }

    /// Apply one validated negotiation while retaining exact rollback
    /// authority until its SIP response or ACK crosses the wire boundary.
    pub(crate) async fn prepare_staged_media_negotiation_lane_owned(
        &self,
        session: &mut SessionState,
    ) -> Result<PreparedMediaNegotiation> {
        let session_id = session.session_id.clone();
        let negotiation_key = Self::media_negotiation_key(session)?;
        let staged = self
            .staged_media_negotiations
            .get(&negotiation_key)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| {
                SessionError::InvalidTransition(
                    "no validated media negotiation is staged for commit".to_string(),
                )
            })?;
        let previous_media_security = session.media_security.clone();

        if previous_media_security.is_some() && !staged.srtp_negotiated {
            return Err(SessionError::SDPNegotiationFailed(
                "an established secure media session cannot be downgraded to plaintext".to_string(),
            ));
        }

        if self.signaling_only_local_port().is_some() {
            if let Some((_, pair)) = self.negotiated_srtp.remove(&negotiation_key) {
                Self::record_media_security_negotiated_lane_owned(session, pair.suite, false);
            }
            return Ok(PreparedMediaNegotiation {
                session_id,
                negotiation_key,
                remote_addr: staged.config.remote_addr,
                previous_media_security,
                lower: None,
            });
        }

        let exact_media = self.lane_owned_media(session)?;
        let dialog_id = exact_media.dialog_id.clone();
        let previous_config = self
            .controller
            .get_session_info(&dialog_id)
            .await
            .ok_or_else(|| {
                SessionError::MediaError(format!("No media session for dialog {dialog_id}"))
            })?
            .config;
        if !self.media_is_still_exact(&exact_media) {
            return Err(SessionError::InvalidTransition(
                "media resource changed before negotiated media commit".to_string(),
            ));
        }

        let remote_addr = staged.config.remote_addr;
        let mut lower = PreparedLowerMediaNegotiation {
            exact_media,
            previous_config,
            srtp_rollback: None,
            stable_local_direction: staged.stable_local_direction,
        };
        let apply_result = async {
            self.apply_negotiated_media_config(
                &dialog_id,
                remote_addr,
                &staged.config.codec,
                staged.config.payload_type,
                staged.config.clock_rate,
                staged.config.channels,
                staged.config.negotiated_fmtp.as_deref(),
            )
            .await?;

            if let Some((_, pair)) = self.negotiated_srtp.remove(&negotiation_key) {
                let suite = pair.suite;
                lower.srtp_rollback = Some(
                    self.controller
                        .prepare_srtp_context_swap(&dialog_id, pair.send_ctx, pair.recv_ctx)
                        .await
                        .map_err(|error| {
                            SessionError::MediaError(format!(
                                "Failed to install SRTP contexts: {error}"
                            ))
                        })?,
                );
                Self::record_media_security_negotiated_lane_owned(session, suite, true);
                if srtp_diagnostics_enabled() {
                    emit_srtp_diag(format!(
                        "srtp_contexts_installed session={} role=commit suite={suite:?}",
                        session_id.0
                    ));
                }
            }

            #[cfg(test)]
            if lower.srtp_rollback.is_some()
                && self
                    .fail_media_commit_after_srtp_swap
                    .swap(false, Ordering::AcqRel)
            {
                return Err(SessionError::MediaError(
                    "injected failure after SRTP context replacement".to_string(),
                ));
            }

            self.controller
                .establish_media_flow(&dialog_id, remote_addr)
                .await
                .map_err(|error| {
                    SessionError::MediaError(format!(
                        "Failed to establish negotiated media flow: {error}"
                    ))
                })?;
            let media_direction = match staged.config.local_direction {
                crate::types::MediaDirection::SendRecv => {
                    rvoip_media_core::types::MediaDirection::SendRecv
                }
                crate::types::MediaDirection::SendOnly => {
                    rvoip_media_core::types::MediaDirection::SendOnly
                }
                crate::types::MediaDirection::RecvOnly => {
                    rvoip_media_core::types::MediaDirection::RecvOnly
                }
                crate::types::MediaDirection::Inactive => {
                    rvoip_media_core::types::MediaDirection::Inactive
                }
            };
            self.controller
                .set_media_direction(&dialog_id, media_direction)
                .await
                .map_err(|error| {
                    SessionError::MediaError(format!(
                        "Failed to apply negotiated media direction: {error}"
                    ))
                })?;
            if !self.media_is_still_exact(&lower.exact_media) {
                return Err(SessionError::InvalidTransition(
                    "media resource changed during negotiated media commit".to_string(),
                ));
            }
            Ok(())
        }
        .await;

        let prepared = PreparedMediaNegotiation {
            session_id,
            negotiation_key,
            remote_addr,
            previous_media_security,
            lower: Some(lower),
        };
        if let Err(error) = apply_result {
            return match self
                .rollback_prepared_media_negotiation_lane_owned(session, prepared)
                .await
            {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(rollback_error),
            };
        }
        Ok(prepared)
    }

    pub(crate) async fn finalize_prepared_media_negotiation_lane_owned(
        &self,
        session: &mut SessionState,
        prepared: PreparedMediaNegotiation,
    ) -> Result<()> {
        if prepared.session_id != session.session_id {
            return Err(SessionError::InvalidTransition(
                "prepared media negotiation belongs to another session".to_string(),
            ));
        }
        let current_key = Self::media_negotiation_key(session)?;
        if current_key != prepared.negotiation_key {
            self.staged_media_negotiations
                .remove(&prepared.negotiation_key);
            self.negotiated_srtp.remove(&prepared.negotiation_key);
            self.pending_srtp_offerers.remove(&prepared.negotiation_key);
            if let Some(lower) = prepared.lower.as_ref() {
                let _ = lower.exact_media._resource.release_lower_once().await;
            }
            return Err(SessionError::InvalidTransition(
                "prepared media negotiation belongs to a stale session generation".to_string(),
            ));
        }
        if prepared
            .lower
            .as_ref()
            .is_some_and(|lower| !self.media_is_still_exact(&lower.exact_media))
        {
            if let Some(lower) = prepared.lower.as_ref() {
                let _ = lower.exact_media._resource.release_lower_once().await;
            }
            session.media_session_id = None;
            session.media_session_ready = false;
            session.media_security = None;
            self.staged_media_negotiations
                .remove(&prepared.negotiation_key);
            self.negotiated_srtp.remove(&prepared.negotiation_key);
            return Err(SessionError::InvalidTransition(
                "media resource changed before negotiated media finalization".to_string(),
            ));
        }
        self.staged_media_negotiations
            .remove(&prepared.negotiation_key);
        self.negotiated_srtp.remove(&prepared.negotiation_key);
        self.pending_srtp_offerers.remove(&prepared.negotiation_key);
        tracing::info!(
            "Committed negotiated media for session {} at {}",
            session.session_id.0,
            prepared.remote_addr
        );
        Ok(())
    }

    pub(crate) async fn rollback_prepared_media_negotiation_lane_owned(
        &self,
        session: &mut SessionState,
        mut prepared: PreparedMediaNegotiation,
    ) -> Result<()> {
        if prepared.session_id != session.session_id {
            return Err(SessionError::InvalidTransition(
                "prepared media rollback belongs to another session".to_string(),
            ));
        }
        let same_generation = Self::media_negotiation_key(session)? == prepared.negotiation_key;
        self.staged_media_negotiations
            .remove(&prepared.negotiation_key);
        self.negotiated_srtp.remove(&prepared.negotiation_key);

        let Some(mut lower) = prepared.lower.take() else {
            if same_generation {
                session.media_security = prepared.previous_media_security;
                return Ok(());
            }
            return Err(SessionError::InvalidTransition(
                "prepared media rollback retired a stale session generation".to_string(),
            ));
        };
        let dialog_id = lower.exact_media.dialog_id.clone();
        let mut rollback_failures = Vec::new();
        if let Some(rollback) = lower.srtp_rollback.take() {
            if self
                .controller
                .rollback_srtp_context_swap(&dialog_id, rollback)
                .await
                .is_err()
            {
                rollback_failures.push("srtp");
            }
        }
        if self
            .controller
            .update_media(dialog_id.clone(), lower.previous_config)
            .await
            .is_err()
        {
            rollback_failures.push("configuration");
        }
        let stable_direction = match lower.stable_local_direction {
            crate::types::MediaDirection::SendRecv => {
                rvoip_media_core::types::MediaDirection::SendRecv
            }
            crate::types::MediaDirection::SendOnly => {
                rvoip_media_core::types::MediaDirection::SendOnly
            }
            crate::types::MediaDirection::RecvOnly => {
                rvoip_media_core::types::MediaDirection::RecvOnly
            }
            crate::types::MediaDirection::Inactive => {
                rvoip_media_core::types::MediaDirection::Inactive
            }
        };
        if self
            .controller
            .set_media_direction(&dialog_id, stable_direction)
            .await
            .is_err()
        {
            rollback_failures.push("direction");
        }
        if !self.media_is_still_exact(&lower.exact_media) {
            rollback_failures.push("ownership");
        }
        #[cfg(test)]
        if self.fail_media_rollback.swap(false, Ordering::AcqRel) {
            rollback_failures.push("injected");
        }

        if rollback_failures.is_empty() {
            if same_generation {
                session.media_security = prepared.previous_media_security;
                return Ok(());
            }
            return Err(SessionError::InvalidTransition(
                "prepared media rollback retired a stale session generation".to_string(),
            ));
        }
        let _ = lower.exact_media._resource.release_lower_once().await;
        if same_generation {
            session.media_session_id = None;
            session.media_session_ready = false;
            session.media_security = None;
        }
        Err(SessionError::MediaError(format!(
            "negotiated media rollback failed at {}; media was quarantined",
            rollback_failures.join(",")
        )))
    }

    pub(crate) async fn commit_staged_media_negotiation_lane_owned(
        &self,
        session: &mut SessionState,
    ) -> Result<()> {
        #[cfg(test)]
        if self.fail_staged_media_commit.swap(false, Ordering::AcqRel) {
            return Err(SessionError::MediaError(
                "injected staged media commit failure".to_string(),
            ));
        }
        let prepared = self
            .prepare_staged_media_negotiation_lane_owned(session)
            .await?;
        self.finalize_prepared_media_negotiation_lane_owned(session, prepared)
            .await
    }

    fn record_media_security_negotiated_lane_owned(
        session: &mut SessionState,
        suite: CryptoSuite,
        contexts_installed: bool,
    ) {
        let state = MediaSecurityState {
            keying: MediaSecurityKeying::Sdes,
            suite,
            profile: MediaSecurityProfile::RtpSavp,
            contexts_installed,
        };
        session.media_security = Some(state);
    }

    /// Queue the application observation only after the lane owner has
    /// committed the negotiated security state. The task is observational:
    /// event pressure cannot delay signaling or change media correctness.
    pub(crate) fn publish_media_security_observation(
        &self,
        lifecycle_handle: SessionRegistryHandle,
        state: MediaSecurityState,
    ) {
        let adapter = self.clone();
        spawn_memory_tracked(
            "sip.media_adapter.media_security_publish_task",
            async move {
                adapter
                    .publish_media_security_observation_inner(lifecycle_handle, state)
                    .await;
            },
        );
    }

    async fn publish_media_security_observation_inner(
        &self,
        lifecycle_handle: SessionRegistryHandle,
        state: MediaSecurityState,
    ) {
        let session_id = lifecycle_handle.session_id().clone();
        let api_event = Event::MediaSecurityNegotiated {
            call_id: session_id.clone(),
            keying: state.keying,
            suite: state.suite,
            profile: state.profile,
            contexts_installed: state.contexts_installed,
        };

        if let Some(publisher) = self.app_event_publisher.read().await.clone() {
            publisher.publish_exact(&lifecycle_handle, api_event);
        } else if let Some(coordinator) = self.global_coordinator.read().await.clone() {
            let public = crate::adapters::SessionApiCrossCrateEvent::new(
                crate::adapters::sanitize_session_api_observation(&api_event),
            );
            if let Err(e) = coordinator.publish_observational(public).await {
                tracing::warn!("Failed to publish MediaSecurityNegotiated event: {}", e);
            }
        } else {
            tracing::debug!(
                "MediaSecurityNegotiated publish skipped for session {}: no event publisher yet",
                session_id.0
            );
        }
    }

    /// Play an audio file to the remote party
    pub async fn play_audio_file(&self, session_id: &SessionId, _file_path: &str) -> Result<()> {
        self.current_media(session_id).ok_or_else(|| {
            SessionError::MediaError(format!("No media session for {}", session_id.0))
        })?;
        Err(unsupported_media_facade("play_audio_file"))
    }

    /// Start recording the media session
    pub async fn start_recording_old(&self, session_id: &SessionId) -> Result<String> {
        self.current_media(session_id).ok_or_else(|| {
            SessionError::MediaError(format!("No media session for {}", session_id.0))
        })?;
        Err(unsupported_media_facade("start_recording_old"))
    }

    /// Create a media bridge between two sessions (for peer-to-peer conferencing)
    pub async fn create_bridge(&self, _session1: &SessionId, _session2: &SessionId) -> Result<()> {
        Err(SessionError::InvalidTransition(
            "create_bridge cannot own an RTP bridge lifetime; use bridge_rtp_sessions and retain its BridgeHandle"
                .to_string(),
        ))
    }

    /// Swap the audio source on the running transmitter for a session.
    /// Used by early-media flows to replace silence with a ringback tone,
    /// hold announcement, or custom samples during `EarlyMedia`.
    ///
    /// The media session must already have an active transmitter — callers
    /// typically invoke this right after `send_early_media` (which has
    /// `PrepareEarlyMediaSDP` + `establish_media_flow` set one up).
    pub async fn set_audio_source(
        &self,
        session_id: &SessionId,
        source: AudioSource,
    ) -> Result<()> {
        let exact = self.current_media(session_id).ok_or_else(|| {
            SessionError::MediaError(format!("No media session for {}", session_id.0))
        })?;

        let result = self
            .controller
            .set_audio_source(&exact.dialog_id, source)
            .await
            .map_err(|e| SessionError::MediaError(format!("Failed to set audio source: {}", e)));
        if !self.media_is_still_exact(&exact) {
            return Err(SessionError::InvalidTransition(
                "media resource changed while setting its audio source".to_string(),
            ));
        }
        result
    }

    /// Set a transmitter source only for the exact session lifetime captured
    /// by the caller. The retained operation prevents teardown or raw-ID reuse
    /// while media-core is applying the change.
    pub(crate) async fn set_audio_source_exact(
        &self,
        handle: &SessionRegistryHandle,
        source: AudioSource,
    ) -> Result<()> {
        let adapter = self.clone();
        let operation_key = handle.key().clone();
        let exact_handle = handle.clone();
        let waiter = self
            .store
            .authority()
            .spawn_owned_exact(
                &operation_key,
                SessionOperationKind::Media,
                MEDIA_AUDIO_OWNED_OPERATION_TIMEOUT,
                move |operation| async move {
                    let Some(exact) = adapter.media_for_handle_exact(&exact_handle) else {
                        return rollback_owned_media(
                            operation,
                            Err(SessionError::SessionNotFound(format!(
                                "No exact media resource for session {}",
                                exact_handle.session_id().0
                            ))),
                        )
                        .await;
                    };
                    let committed = match operation.commit() {
                        Ok(committed) => committed,
                        Err(failure) => {
                            return rollback_owned_media(
                                failure.into_operation(),
                                Err(SessionError::InvalidTransition(format!(
                                    "Session {} retired before audio-source dispatch",
                                    exact_handle.session_id().0
                                ))),
                            )
                            .await;
                        }
                    };
                    let result = adapter
                        .controller
                        .set_audio_source(&exact.dialog_id, source)
                        .await
                        .map_err(|error| {
                            SessionError::MediaError(format!("Failed to set audio source: {error}"))
                        });
                    committed.complete(result)
                },
            )
            .map_err(|error| {
                SessionError::SessionNotFound(format!(
                    "Exact audio-source operation was not admitted: {error}"
                ))
            })?;
        waiter.await.map_err(|error| {
            SessionError::MediaError(format!(
                "Exact audio-source operation did not complete: {error}"
            ))
        })?
    }

    /// Bridge the RTP streams of two sessions at the media-core layer.
    ///
    /// Resolves each `SessionId` to its underlying `DialogId` and delegates
    /// to `MediaSessionController::bridge_sessions`. Transparent packet-level
    /// relay — both legs must have negotiated the same codec and reached the
    /// `Active` state (remote RTP address known).
    ///
    /// Dropping the returned [`BridgeHandle`] tears the bridge down.
    pub async fn bridge_rtp_sessions(
        &self,
        session_a: &SessionId,
        session_b: &SessionId,
    ) -> std::result::Result<BridgeHandle, BridgeError> {
        let exact_a = self
            .current_media(session_a)
            .ok_or_else(|| BridgeError::SessionNotFound(session_a.0.clone()))?;
        let exact_b = self
            .current_media(session_b)
            .ok_or_else(|| BridgeError::SessionNotFound(session_b.0.clone()))?;

        let handle = self
            .controller
            .bridge_sessions(exact_a.dialog_id.clone(), exact_b.dialog_id.clone())
            .await?;
        if !self.media_is_still_exact(&exact_a) || !self.media_is_still_exact(&exact_b) {
            drop(handle);
            return Err(BridgeError::SessionNotFound(
                "media lifetime changed during bridge creation".to_string(),
            ));
        }

        Ok(handle)
    }

    /// Compatibility facade retained for source stability. RTP bridge
    /// lifetime belongs to the [`BridgeHandle`] returned by
    /// [`Self::bridge_rtp_sessions`]; dropping that handle is the only exact
    /// destroy operation.
    pub async fn destroy_bridge(&self, _session_id: &SessionId) -> Result<()> {
        Err(SessionError::InvalidTransition(
            "destroy_bridge has no exact BridgeHandle owner; drop the handle returned by bridge_rtp_sessions"
                .to_string(),
        ))
    }

    /// Stop recording the media session
    pub async fn stop_recording_old(&self, session_id: &SessionId) -> Result<()> {
        self.current_media(session_id).ok_or_else(|| {
            SessionError::MediaError(format!("No media session for {}", session_id.0))
        })?;
        Err(unsupported_media_facade("stop_recording_old"))
    }

    // ===== AUDIO FRAME API - The Missing Core Functionality =====

    /// Send an audio frame for encoding and transmission
    /// This is the equivalent of the legacy MediaControl::send_audio_frame() API.
    pub async fn send_audio_frame(
        &self,
        session_id: &SessionId,
        audio_frame: AudioFrame,
    ) -> Result<()> {
        let exact = self.current_media(session_id).ok_or_else(|| {
            SessionError::MediaError(format!("No media session for {}", session_id.0))
        })?;

        tracing::trace!(
            "📤 Sending audio frame for session {} ({} samples) via RTP",
            session_id.0,
            audio_frame.samples.len()
        );

        self.audio_send_frames_total.fetch_add(1, Ordering::Relaxed);
        self.audio_send_samples_total
            .fetch_add(audio_frame.samples.len() as u64, Ordering::Relaxed);

        self.controller
            .encode_and_send_audio(&exact.dialog_id, audio_frame)
            .await
            .map_err(|e| {
                SessionError::MediaError(format!("Failed to send audio frame via RTP: {}", e))
            })?;

        if !self.media_is_still_exact(&exact) {
            return Err(SessionError::InvalidTransition(
                "media resource changed during audio dispatch".to_string(),
            ));
        }

        tracing::trace!(
            "✅ Audio frame sent successfully via RTP for session {}",
            session_id.0
        );
        Ok(())
    }

    /// Send one frame through the exact media lifetime retained by a
    /// `SessionHandle`. No raw SessionId lookup or post-await TOCTOU check is
    /// involved.
    pub(crate) async fn send_audio_frame_exact(
        &self,
        handle: &SessionRegistryHandle,
        audio_frame: AudioFrame,
    ) -> Result<()> {
        let exact = self.media_for_handle_exact(handle).ok_or_else(|| {
            SessionError::SessionNotFound(format!(
                "No exact media resource for session {}",
                handle.session_id().0
            ))
        })?;
        self.audio_send_frames_total.fetch_add(1, Ordering::Relaxed);
        self.audio_send_samples_total
            .fetch_add(audio_frame.samples.len() as u64, Ordering::Relaxed);

        // `dialog_id` is generation-qualified and the retained resource is a
        // strong exact-lifetime binding. Teardown may make this A operation
        // fail, but it can never redirect it to a later B allocation. Keeping
        // this hot path allocation-free avoids one supervisor task per 20 ms
        // audio frame.
        self.controller
            .encode_and_send_audio(&exact.dialog_id, audio_frame)
            .await
            .map_err(|error| {
                SessionError::MediaError(format!("Failed to send audio frame via RTP: {error}"))
            })
    }

    /// Create a new media session
    pub async fn create_session(
        &self,
        session_id: &SessionId,
    ) -> Result<crate::types::MediaSessionId> {
        self.session_create_attempt_total
            .fetch_add(1, Ordering::Relaxed);
        let handle = self.store.lifecycle_handle(session_id).ok_or_else(|| {
            SessionError::SessionNotFound(format!("Session {} is not live", session_id.0))
        })?;
        if let Some(existing) = self.media_for_handle_exact(&handle) {
            self.session_create_success_total
                .fetch_add(1, Ordering::Relaxed);
            return Ok(existing.dialog_id);
        }
        if self.media_resources.contains_key(session_id)
            || self
                .store
                .registry()
                .get_media_handle_exact(&handle)
                .is_some()
        {
            self.session_create_failed_total
                .fetch_add(1, Ordering::Relaxed);
            return Err(SessionError::InvalidTransition(
                "A stale or partially published media owner blocks creation".to_string(),
            ));
        }
        match self.media_create_reservations.entry(session_id.clone()) {
            dashmap::mapref::entry::Entry::Vacant(entry) => {
                entry.insert(handle.clone());
            }
            dashmap::mapref::entry::Entry::Occupied(_) => {
                self.session_create_failed_total
                    .fetch_add(1, Ordering::Relaxed);
                return Err(SessionError::InternalError(
                    "Media creation is already in progress for this session".to_string(),
                ));
            }
        }
        // The lower-layer identity is generation-qualified. A delayed cleanup
        // can therefore stop only the allocation it created, even after the
        // application reuses the raw SessionId beyond its anti-reuse horizon.
        let dialog_id = DialogId::new(format!(
            "media-{}-{}",
            session_id.0,
            handle.key().resource_generation_suffix()
        ));
        let resource = MediaSessionResource::new(self, handle.clone(), dialog_id.clone());
        let authority = Arc::clone(self.store.authority());
        let adapter = self.clone();
        let operation_key = handle.key().clone();
        let admission_reservations = Arc::clone(&self.media_create_reservations);
        let admission_handle = handle.clone();
        let owned_reservation_guard = MediaCreateReservationGuard::new(
            Arc::clone(&self.media_create_reservations),
            handle.clone(),
        );

        tracing::info!(
            "🚀 Creating media session for session {} with dialog ID {}",
            session_id.0,
            dialog_id
        );

        let waiter = authority
            .spawn_owned_exact(
                &operation_key,
                SessionOperationKind::Media,
                MEDIA_CREATE_OWNED_OPERATION_TIMEOUT,
                move |mut operation| async move {
                    // Constructed before supervisor admission so even a task
                    // that is dropped before its first poll owns cleanup of
                    // the private raw-id creation fence.
                    let _reservation_guard = owned_reservation_guard;
                    let spec = match ResourceSpec::new(
                        resource.descriptor(),
                        Vec::new(),
                        MEDIA_RESOURCE_RELEASE_TIMEOUT,
                    ) {
                        Ok(spec) => spec,
                        Err(_) => {
                            return rollback_owned_media(
                                operation,
                                Err(SessionError::InternalError(
                                    "Media resource specification failed (class=lifecycle)"
                                        .to_string(),
                                )),
                            )
                            .await;
                        }
                    };
                    let install_attempt = match operation.reserve_resource(spec) {
                        Ok(attempt) => attempt,
                        Err(_) => {
                            return rollback_owned_media(
                                operation,
                                Err(SessionError::InternalError(
                                    "Media resource admission failed (class=lifecycle)"
                                        .to_string(),
                                )),
                            )
                            .await;
                        }
                    };
                    let installation_sink = match install_attempt.dispatch() {
                        Ok(permit) => permit.into_installation_sink(),
                        Err(_) => {
                            return rollback_owned_media(
                                operation,
                                Err(SessionError::InternalError(
                                    "Media resource dispatch failed (class=lifecycle)"
                                        .to_string(),
                                )),
                            )
                            .await;
                        }
                    };

                    let info = if resource.core_media_allocated {
                        let media_config = MediaConfig {
                            local_addr: SocketAddr::new(adapter.local_ip, 0),
                            remote_addr: None,
                            preferred_codec: Some("PCMU".to_string()),
                            parameters: std::collections::HashMap::new(),
                        };
                        let allocation = tokio::time::timeout(
                            MEDIA_CREATE_ALLOCATION_TIMEOUT,
                            async {
                                adapter
                                    .controller
                                    .start_media(dialog_id.clone(), media_config)
                                    .await
                                    .map_err(|_| {
                                        SessionError::MediaError(
                                            "Failed to start media session (class=media-core)"
                                                .to_string(),
                                        )
                                    })?;

                                #[cfg(test)]
                                if adapter
                                    .pause_media_create_after_allocation
                                    .load(Ordering::Acquire)
                                {
                                    adapter.media_create_allocated.notify_one();
                                    adapter.resume_media_create.notified().await;
                                }

                                adapter
                                    .controller
                                    .get_session_info(&dialog_id)
                                    .await
                                    .ok_or_else(|| {
                                        SessionError::MediaError(
                                            "Media session disappeared during allocation"
                                                .to_string(),
                                        )
                                    })
                            },
                        );
                        tokio::pin!(allocation);
                        let mut cancellation = operation.cancellation();
                        let allocation_result = if let Some(cancel) = cancellation.as_mut() {
                            let cancelled = async {
                                if *cancel.borrow() {
                                    return;
                                }
                                let _ = cancel.changed().await;
                            };
                            tokio::pin!(cancelled);
                            tokio::select! {
                                result = &mut allocation => Some(result),
                                () = &mut cancelled => None,
                            }
                        } else {
                            Some(allocation.await)
                        };

                        match allocation_result {
                            Some(Ok(Ok(info))) => Some(info),
                            Some(Ok(Err(error))) => {
                                return rollback_uncommitted_media(
                                    operation,
                                    installation_sink,
                                    resource,
                                    Err(error),
                                )
                                .await;
                            }
                            Some(Err(_)) => {
                                return rollback_uncommitted_media(
                                    operation,
                                    installation_sink,
                                    resource,
                                    Err(SessionError::MediaError(
                                        "Media allocation timed out".to_string(),
                                    )),
                                )
                                .await;
                            }
                            None => {
                                return rollback_uncommitted_media(
                                    operation,
                                    installation_sink,
                                    resource,
                                    Err(SessionError::SessionNotFound(format!(
                                        "Session {} retired during media allocation",
                                        handle.session_id().0
                                    ))),
                                )
                                .await;
                            }
                        }
                    } else {
                        tracing::info!(
                            "signaling-only media mode: skipped media-core allocation for session {}",
                            handle.session_id().0
                        );
                        None
                    };

                    if installation_sink
                        .capture_at_install(
                            Arc::clone(&resource) as Arc<dyn ManagedSessionResource>
                        )
                        .is_err()
                    {
                        let _ = resource.release_once().await;
                        return rollback_owned_media(
                            operation,
                            Err(SessionError::InternalError(
                                "Media lifecycle capture failed (class=lifecycle)".to_string(),
                            )),
                        )
                        .await;
                    }

                    if adapter
                        .store
                        .registry()
                        .install_media_handle(&handle, dialog_id.clone())
                        .is_err()
                    {
                        return rollback_owned_media(
                            operation,
                            Err(SessionError::InternalError(
                                "Media registry installation failed (class=lifecycle)".to_string(),
                            )),
                        )
                        .await;
                    }

                    let committed = match operation.commit() {
                        Ok(committed) => committed,
                        Err(failure) => {
                            return rollback_owned_media(
                                failure.into_operation(),
                                Err(SessionError::SessionNotFound(format!(
                                    "Session {} retired before media commit",
                                    handle.session_id().0
                                ))),
                            )
                            .await;
                        }
                    };

                    // Publish resource views only after the exact operation and
                    // canonical registry association have committed. The owned
                    // operation remains registered until this block completes,
                    // so teardown cannot release halfway through publication.
                    if let Some(info) = info {
                        adapter
                            .media_sessions
                            .insert(handle.session_id().clone(), info);
                        adapter.controller.store_session_mapping(
                            handle.session_id().0.clone(),
                            rvoip_media_core::MediaSessionId::from_dialog(&dialog_id),
                        );
                    }
                    match adapter.media_resources.entry(handle.session_id().clone()) {
                        dashmap::mapref::entry::Entry::Vacant(entry) => {
                            entry.insert(MediaSessionBinding {
                                handle: handle.clone(),
                                dialog_id: dialog_id.clone(),
                                resource: Arc::downgrade(&resource),
                            });
                        }
                        dashmap::mapref::entry::Entry::Occupied(_) => {
                            let _ = resource.release_once().await;
                            return committed.complete(Err(SessionError::InternalError(
                                "Media resource publication collided (class=lifecycle)"
                                    .to_string(),
                            )));
                        }
                    }

                    if resource.core_media_allocated {
                        adapter
                            .install_dtmf_callback(handle.clone(), dialog_id.clone())
                            .await;
                    }

                    tracing::info!(
                        "✅ Media session created successfully for dialog {}",
                        dialog_id
                    );
                    committed.complete(Ok(dialog_id))
                },
            )
            .map_err(|_| {
                admission_reservations.remove_if(admission_handle.session_id(), |_, owner| {
                    owner == &admission_handle
                });
                self.session_create_failed_total
                    .fetch_add(1, Ordering::Relaxed);
                SessionError::InternalError(
                    "Media owned operation admission failed (class=lifecycle)".to_string(),
                )
            })?;

        let result = waiter.await.map_err(|_| {
            // `DeadlineExceeded` deliberately does not cancel retained owned
            // work. Removing the fence here would let a second creator race
            // that still-running operation. The closure-owned reservation
            // guard clears it on every actual completion, panic, or abort;
            // the managed resource also clears it on rollback/release.
            self.session_create_failed_total
                .fetch_add(1, Ordering::Relaxed);
            SessionError::InternalError(
                "Media owned operation failed (class=lifecycle)".to_string(),
            )
        })?;
        match result {
            Ok(media_id) => {
                self.session_create_success_total
                    .fetch_add(1, Ordering::Relaxed);
                Ok(media_id)
            }
            Err(error) => {
                self.session_create_failed_total
                    .fetch_add(1, Ordering::Relaxed);
                Err(error)
            }
        }
    }

    /// Generate the local SDP offer. **Sole** SDP-offer generator — this
    /// is the only entry point used by the state-machine's
    /// `Action::GenerateLocalSDP`. Always uses `SdpBuilder`, always
    /// advertises PCMU + PCMA + RFC 4733 telephone-event, and conditionally
    /// attaches `a=crypto:` lines when [`Config::offer_srtp`](crate::api::unified::Config) is set.
    ///
    /// Profile selection per RFC 4568 §3.1.4: `RTP/SAVP` when offering
    /// SDES, `RTP/AVP` otherwise. SRTP master keys are generated via
    /// [`SrtpNegotiator::new_offerer`] and stashed in
    /// `pending_srtp_offerers` keyed by `session_id` so
    /// [`Self::negotiate_sdp_as_uac`] can drive `accept_answer` against
    /// the matching answer.
    ///
    /// **DTMF advertisement (P2 fix).** Pre-Sprint 2.5 the non-SRTP path
    /// silently omitted PT 101 / `a=fmtp:101 0-15`, leaving plaintext
    /// callers unable to negotiate DTMF. The unified path always emits
    /// the telephone-event rtpmap + fmtp regardless of profile —
    /// `offer_advertises_telephone_event_on_plaintext` in the test
    /// module locks this in.
    pub async fn generate_local_sdp(&self, session_id: &SessionId) -> Result<String> {
        self.generate_local_sdp_offer(session_id, crate::types::MediaDirection::SendRecv)
            .await
    }

    pub(crate) async fn generate_local_sdp_lane_owned(
        &self,
        session: &mut SessionState,
    ) -> Result<String> {
        self.generate_local_sdp_offer_lane_owned(session, crate::types::MediaDirection::SendRecv)
            .await
    }

    /// Generate a local SDP offer for the requested media direction.
    pub async fn generate_local_sdp_offer(
        &self,
        session_id: &SessionId,
        direction: crate::types::MediaDirection,
    ) -> Result<String> {
        let (lane, snapshot, mut session) =
            self.lock_and_load_exact_media_session(session_id).await?;
        let previous_origin = (
            session.sdp_origin_session_id.clone(),
            session.sdp_origin_version,
        );
        let result = self
            .generate_local_sdp_offer_lane_owned(&mut session, direction)
            .await;
        if previous_origin
            != (
                session.sdp_origin_session_id.clone(),
                session.sdp_origin_version,
            )
        {
            drop(lane);
            self.commit_media_lane_state(&snapshot, &session).await?;
        }
        result
    }

    pub(crate) async fn generate_local_sdp_offer_lane_owned(
        &self,
        session: &mut SessionState,
        direction: crate::types::MediaDirection,
    ) -> Result<String> {
        let session_id = session.session_id.clone();
        let negotiation_key = Self::media_negotiation_key(session)?;
        if self.signaling_only_local_port().is_some() {
            return self
                .generate_signaling_only_sdp_offer_lane_owned(session, direction)
                .await;
        }
        // Resolve the exact managed resource once and prime the cached session
        // info. Both SRTP and plaintext paths share this canonical owner.
        let exact_media = self.lane_owned_media(session)?;
        let dialog_id = &exact_media.dialog_id;
        let info = self
            .controller
            .get_session_info(dialog_id)
            .await
            .ok_or_else(|| {
                SessionError::MediaError(format!(
                    "Failed to get session info for dialog {}",
                    dialog_id
                ))
            })?;
        self.media_sessions.insert(session_id.clone(), info.clone());
        if !self.media_is_still_exact(&exact_media) {
            return Err(SessionError::InvalidTransition(
                "media resource changed during SDP offer generation".to_string(),
            ));
        }

        // Sprint 3 A6 — when a public RTP address has been configured
        // (static override or STUN-discovered), advertise that in the
        // SDP `c=` / `o=` / `m=audio` lines instead of the bind-address.
        // The static override's port wins when set; otherwise we keep
        // the per-session local RTP port (most NATs don't preserve
        // ports across the binding, but absent better info the local
        // port is our best guess and symmetric-RTP latching covers
        // the rest).
        let public = self.public_rtp_addr();
        let port = public
            .filter(|sa| sa.port() != 0)
            .map(|sa| sa.port())
            .unwrap_or_else(|| info.rtp_port.unwrap_or(info.config.local_addr.port()));
        let (origin_session_id, origin_version) = advance_sdp_origin(session);
        let origin_version = origin_version.to_string();
        let advertised_ip = public.map(|sa| sa.ip()).unwrap_or(self.local_ip);
        let local_ip_str = advertised_ip.to_string();

        // Profile + crypto. RFC 4568 §3.1.4 — `RTP/SAVP` is mandatory
        // when offering SDES.
        let (transport, crypto_attrs) = if self.offer_srtp {
            let (negotiator, attrs) = SrtpNegotiator::new_offerer_with_base64_mode(
                &self.srtp_offered_suites,
                self.sdes_base64_mode,
            )?;
            self.pending_srtp_offerers
                .insert(negotiation_key, negotiator);
            ("RTP/SAVP", attrs)
        } else {
            ("RTP/AVP", Vec::new())
        };
        let crypto_attr_count = crypto_attrs.len();
        if sdp_diagnostics_enabled() {
            emit_sdp_diag(format!(
                "local_sdp_offer session={} media={}:{} transport={} {} direction={}",
                session_id.0,
                advertised_ip,
                port,
                transport,
                crypto_attribute_diag(crypto_attr_count),
                direction_attribute(direction)
            ));
        }

        // NEXT_STEPS C2 — iterate the configured `offered_codecs` and
        // emit one rtpmap (+ fmtp where required) per PT, instead of
        // hard-coding PCMU/PCMA/DTMF. Comfort Noise (PT 13) is folded
        // in by `effective_offered_formats` so existing callers that
        // only flip the `comfort_noise_enabled` switch keep working.
        // Crypto attrs follow rtpmap/fmtp so ordering matches what
        // carriers expect; sendrecv goes last so the byte-fixture
        // tests stay stable.
        let format_pts = self.effective_offered_formats();
        let format_strings: Vec<String> = format_pts.iter().map(|pt| pt.to_string()).collect();
        let formats_ref: Vec<&str> = format_strings.iter().map(|s| s.as_str()).collect();
        let mut media_builder = SdpBuilder::new("Session")
            .origin(
                "-",
                &origin_session_id,
                &origin_version,
                "IN",
                "IP4",
                &local_ip_str,
            )
            .connection("IN", "IP4", &local_ip_str)
            .time("0", "0")
            .media_audio(port, transport)
            .formats(&formats_ref);
        for (pt, pt_str) in format_pts.iter().zip(format_strings.iter()) {
            if let Some(rtpmap) = rtpmap_for_pt(*pt) {
                media_builder = media_builder.rtpmap(pt_str.as_str(), rtpmap);
            }
            if let Some(fmtp) = fmtp_for_pt_with_g729_annex_b(*pt, self.g729_annex_b) {
                media_builder = media_builder.fmtp(pt_str.as_str(), fmtp);
            }
        }
        for attr in crypto_attrs {
            media_builder = media_builder.crypto_attribute(attr);
        }
        let session = media_builder
            .attribute(direction_attribute(direction), None::<String>)
            .done()
            .build()
            .map_err(|_| bounded_sdp_failure("offer-build", "builder"))?;

        let sdp = session.to_string();
        tracing::info!(
            "✅ Generated SDP for session {} with local port {} direction {}",
            session_id.0,
            port,
            direction_attribute(direction)
        );
        Ok(sdp)
    }

    /// Install the RFC 4733 DTMF bridge: registers a callback with
    /// media-core so PT 101 packets (already deduped to one-per-digit
    /// on the first end-of-event frame) are published as
    /// `Event::DtmfReceived { call_id, digit }` on the rvoip-sip
    /// public API event stream. No-op if the app event publisher/global
    /// coordinator has not been installed yet (e.g. isolated unit tests).
    async fn install_dtmf_callback(
        &self,
        lifecycle_handle: SessionRegistryHandle,
        dialog_id: DialogId,
    ) {
        let session_id = lifecycle_handle.session_id().clone();
        let publisher = self.app_event_publisher.read().await.clone();
        let coordinator = self.global_coordinator.read().await.clone();
        if publisher.is_none() && coordinator.is_none() {
            tracing::debug!(
                "DTMF callback install skipped for session {}: no event publisher yet",
                session_id.0
            );
            return;
        };

        let (tx, mut rx) = mpsc::channel::<rvoip_media_core::DtmfNotification>(32);
        if let Err(e) = self
            .controller
            .set_dtmf_callback(dialog_id.clone(), tx)
            .await
        {
            tracing::warn!(
                "Failed to register DTMF callback for session {} (dialog {}): {}",
                session_id.0,
                dialog_id,
                e
            );
            return;
        }

        // Consumer task: forwards each DTMF notification from media-core
        // onto the session-core API event bus. Exits cleanly when the
        // sender end of the channel is dropped (media session stopped).
        let sid = session_id.clone();
        let did = dialog_id.clone();
        spawn_memory_tracked("sip.media_adapter.dtmf_bridge_task", async move {
            while let Some(notification) = rx.recv().await {
                let api_event = crate::api::events::Event::DtmfReceived {
                    call_id: sid.clone(),
                    digit: notification.digit,
                };
                if let Some(publisher) = publisher.as_ref() {
                    publisher.publish_exact(&lifecycle_handle, api_event);
                } else if let Some(coordinator) = coordinator.as_ref() {
                    let public = crate::adapters::SessionApiCrossCrateEvent::new(
                        crate::adapters::sanitize_session_api_observation(&api_event),
                    );
                    if let Err(e) = coordinator.publish_observational(public).await {
                        tracing::warn!(
                            "Failed to publish DtmfReceived for session {}: {}",
                            sid.0,
                            e
                        );
                    }
                }
                tracing::info!(
                    "📢 Published DtmfReceived digit='{}' for session {}",
                    notification.digit,
                    sid.0
                );
            }
            tracing::debug!(
                "DTMF bridge task exited for session {} (dialog {})",
                sid.0,
                did
            );
        });
    }

    /// Subscribe to receive decoded audio frames from RTP
    /// This is the equivalent of the legacy MediaControl::subscribe_to_audio_frames() API.
    pub async fn subscribe_to_audio_frames(
        &self,
        session_id: &SessionId,
    ) -> Result<crate::types::AudioFrameSubscriber> {
        let (_lane, _snapshot, session) =
            self.lock_and_load_exact_media_session(session_id).await?;
        let exact = self.lane_owned_media(&session)?;
        let dialog_id = &exact.dialog_id;

        tracing::info!(
            "🎧 Setting up audio subscription for session {} (dialog: {})",
            session_id.0,
            dialog_id
        );

        // Keep decoded-audio buffering bounded for real-time media. At 50 fps,
        // 128 frames is ~2.5 seconds; the old 1000-frame buffer retained up to
        // 20 seconds of stale audio per active call.
        let (tx, rx) = mpsc::channel(AUDIO_RECEIVER_CHANNEL_FRAMES);

        // Register the callback with MediaSessionController to receive audio frames
        self.controller
            .set_audio_frame_callback(dialog_id.clone(), tx.clone())
            .await
            .map_err(|e| {
                SessionError::MediaError(format!("Failed to set audio callback: {}", e))
            })?;

        if !self.media_is_still_exact(&exact) {
            let _ = self.controller.remove_audio_frame_callback(dialog_id).await;
            return Err(SessionError::InvalidTransition(
                "media resource changed during audio subscription".to_string(),
            ));
        }

        // Store the sender for this session for cleanup
        self.audio_receivers.insert(session_id.clone(), tx);
        self.audio_subscriber_created_total
            .fetch_add(1, Ordering::Relaxed);

        tracing::info!(
            "🎧 Created audio frame subscriber for session {} with dialog {}",
            session_id.0,
            dialog_id
        );

        // Return our types::AudioFrameSubscriber
        Ok(crate::types::AudioFrameSubscriber::new(
            session_id.clone(),
            rx,
        ))
    }

    /// Install an audio callback for one exact registry lifetime. The owned
    /// operation retains that lifetime until callback publication completes.
    pub(crate) async fn subscribe_to_audio_frames_exact(
        &self,
        handle: &SessionRegistryHandle,
    ) -> Result<crate::types::AudioFrameSubscriber> {
        let adapter = self.clone();
        let operation_key = handle.key().clone();
        let exact_handle = handle.clone();
        let waiter = self
            .store
            .authority()
            .spawn_owned_exact(
                &operation_key,
                SessionOperationKind::Media,
                MEDIA_AUDIO_OWNED_OPERATION_TIMEOUT,
                move |operation| async move {
                    let Some(exact) = adapter.media_for_handle_exact(&exact_handle) else {
                        return rollback_owned_media(
                            operation,
                            Err(SessionError::SessionNotFound(format!(
                                "No exact media resource for session {}",
                                exact_handle.session_id().0
                            ))),
                        )
                        .await;
                    };
                    let (tx, rx) = mpsc::channel(AUDIO_RECEIVER_CHANNEL_FRAMES);
                    let committed = match operation.commit() {
                        Ok(committed) => committed,
                        Err(failure) => {
                            return rollback_owned_media(
                                failure.into_operation(),
                                Err(SessionError::InvalidTransition(format!(
                                    "Session {} retired before audio subscription",
                                    exact_handle.session_id().0
                                ))),
                            )
                            .await;
                        }
                    };
                    let result = match adapter
                        .controller
                        .set_audio_frame_callback(exact.dialog_id.clone(), tx.clone())
                        .await
                    {
                        Ok(()) => {
                            adapter
                                .audio_receivers
                                .insert(exact_handle.session_id().clone(), tx);
                            adapter
                                .audio_subscriber_created_total
                                .fetch_add(1, Ordering::Relaxed);
                            Ok(crate::types::AudioFrameSubscriber::new(
                                exact_handle.session_id().clone(),
                                rx,
                            ))
                        }
                        Err(error) => Err(SessionError::MediaError(format!(
                            "Failed to set audio callback: {error}"
                        ))),
                    };
                    committed.complete(result)
                },
            )
            .map_err(|error| {
                SessionError::SessionNotFound(format!(
                    "Exact audio subscription was not admitted: {error}"
                ))
            })?;
        waiter.await.map_err(|error| {
            SessionError::MediaError(format!(
                "Exact audio subscription did not complete: {error}"
            ))
        })?
    }

    // ===== New Methods for CallController and ConferenceManager =====

    /// Create a media session
    pub async fn create_media_session(&self) -> Result<crate::types::MediaSessionId> {
        Err(unsupported_media_facade("create_media_session"))
    }

    /// Stop a media session
    pub async fn stop_media_session(&self, _media_id: crate::types::MediaSessionId) -> Result<()> {
        Err(unsupported_media_facade("stop_media_session"))
    }

    /// Set media direction (for hold/resume)
    pub async fn set_media_direction(
        &self,
        media_id: crate::types::MediaSessionId,
        direction: crate::types::MediaDirection,
    ) -> Result<()> {
        let exact = self.current_media_by_dialog_id(&media_id).ok_or_else(|| {
            SessionError::SessionNotFound(format!(
                "No exact session owns media allocation {}",
                media_id
            ))
        })?;
        if !exact._resource.core_media_allocated {
            return Ok(());
        }
        let media_direction = match direction {
            crate::types::MediaDirection::SendRecv => {
                rvoip_media_core::types::MediaDirection::SendRecv
            }
            crate::types::MediaDirection::SendOnly => {
                rvoip_media_core::types::MediaDirection::SendOnly
            }
            crate::types::MediaDirection::RecvOnly => {
                rvoip_media_core::types::MediaDirection::RecvOnly
            }
            crate::types::MediaDirection::Inactive => {
                rvoip_media_core::types::MediaDirection::Inactive
            }
        };
        self.controller
            .set_media_direction(&exact.dialog_id, media_direction)
            .await
            .map_err(|_| SessionError::MediaError("failed to set media direction".to_string()))?;
        if !self.media_is_still_exact(&exact) {
            return Err(SessionError::InvalidTransition(
                "media resource changed while updating its direction".to_string(),
            ));
        }
        Ok(())
    }

    /// Create hold SDP for an established session.
    pub async fn create_hold_sdp_for_session(&self, session_id: &SessionId) -> Result<String> {
        self.generate_local_sdp_offer(session_id, crate::types::MediaDirection::SendOnly)
            .await
    }

    pub(crate) async fn create_hold_sdp_for_session_lane_owned(
        &self,
        session: &mut SessionState,
    ) -> Result<String> {
        self.generate_local_sdp_offer_lane_owned(session, crate::types::MediaDirection::SendOnly)
            .await
    }

    /// Create active SDP for an established session.
    pub async fn create_active_sdp_for_session(&self, session_id: &SessionId) -> Result<String> {
        self.generate_local_sdp_offer(session_id, crate::types::MediaDirection::SendRecv)
            .await
    }

    pub(crate) async fn create_active_sdp_for_session_lane_owned(
        &self,
        session: &mut SessionState,
    ) -> Result<String> {
        self.generate_local_sdp_offer_lane_owned(session, crate::types::MediaDirection::SendRecv)
            .await
    }

    /// Create hold SDP without a session context.
    ///
    /// Prefer [`Self::create_hold_sdp_for_session`]. This fallback exists for
    /// older internal call sites and tests; it advertises the configured media
    /// start port rather than disabling the m-line.
    pub async fn create_hold_sdp(&self) -> Result<String> {
        self.create_directional_fallback_sdp(crate::types::MediaDirection::SendOnly)
    }

    /// Create active SDP without a session context.
    ///
    /// Prefer [`Self::create_active_sdp_for_session`].
    pub async fn create_active_sdp(&self) -> Result<String> {
        self.create_directional_fallback_sdp(crate::types::MediaDirection::SendRecv)
    }

    fn create_directional_fallback_sdp(
        &self,
        direction: crate::types::MediaDirection,
    ) -> Result<String> {
        let local_ip = self.local_ip.to_string();
        let formats: &[&str] = if self.comfort_noise_enabled {
            &["0", "8", "13", "101"]
        } else {
            &["0", "8", "101"]
        };
        let mut media_builder = SdpBuilder::new("Session")
            .origin("-", "0", "0", "IN", "IP4", &local_ip)
            .connection("IN", "IP4", &local_ip)
            .time("0", "0")
            .media_audio(self.media_port_start, "RTP/AVP")
            .formats(formats)
            .rtpmap("0", "PCMU/8000")
            .rtpmap("8", "PCMA/8000");
        if self.comfort_noise_enabled {
            media_builder = media_builder.rtpmap("13", "CN/8000");
        }
        let session = media_builder
            .rtpmap("101", "telephone-event/8000")
            .fmtp("101", "0-15")
            .attribute(direction_attribute(direction), None::<String>)
            .done()
            .build()
            .map_err(|_| bounded_sdp_failure("directional-build", "builder"))?;
        Ok(session.to_string())
    }

    /// Send DTMF digit (legacy `media_id` signature used by the state
    /// machine's CallController path). Delegates to
    /// [`Self::send_dtmf_rfc4733`] with a 100 ms duration.
    pub async fn send_dtmf(
        &self,
        media_id: crate::types::MediaSessionId,
        digit: char,
    ) -> Result<()> {
        // `MediaSessionId` is now a type alias for media-core's
        // `DialogId` (P5), so the value passed in IS the dialog id we
        // need — no reconstruction.
        let dialog_id = media_id;
        self.controller
            .send_dtmf_packet(&dialog_id, digit, 100)
            .await
            .map_err(|e| SessionError::MediaError(format!("DTMF send failed: {}", e)))?;
        tracing::debug!("☎️  Queued DTMF '{}' for media_id {:?}", digit, dialog_id);
        Ok(())
    }

    /// Send RFC 4733 DTMF by session id — preferred public API, used
    /// by [`UnifiedCoordinator::send_dtmf`](crate::api::unified::UnifiedCoordinator::send_dtmf).
    pub async fn send_dtmf_rfc4733(
        &self,
        session_id: &SessionId,
        digit: char,
        duration_ms: u32,
    ) -> Result<()> {
        let handle = self.store.lifecycle_handle(session_id).ok_or_else(|| {
            SessionError::SessionNotFound(format!("No media session for {}", session_id.0))
        })?;
        self.send_dtmf_rfc4733_exact(&handle, digit, duration_ms)
            .await
    }

    /// Send RFC 4733 only through media owned by the captured lifetime.
    pub(crate) async fn send_dtmf_rfc4733_exact(
        &self,
        handle: &SessionRegistryHandle,
        digit: char,
        duration_ms: u32,
    ) -> Result<()> {
        let exact = self.media_for_handle_exact(handle).ok_or_else(|| {
            SessionError::SessionNotFound(format!(
                "No exact media session for {}",
                handle.session_id().0
            ))
        })?;

        self.controller
            .send_dtmf_packet(&exact.dialog_id, digit, duration_ms)
            .await
            .map_err(|e| SessionError::MediaError(format!("DTMF send failed: {}", e)))?;
        if !self.media_is_still_exact(&exact) {
            return Err(SessionError::InvalidTransition(
                "media resource changed during DTMF dispatch".to_string(),
            ));
        }

        tracing::info!(
            "☎️  Queued DTMF '{}' for session {} (dialog {}, duration={}ms)",
            digit,
            handle.session_id().0,
            exact.dialog_id,
            duration_ms
        );
        Ok(())
    }

    /// Send a fully validated RFC 4733 digit sequence with explicit timing.
    ///
    /// The media controller validates every digit, the duration, the sequence
    /// count, and the total schedule before emitting the first RTP packet.
    pub async fn send_dtmf_sequence_rfc4733(
        &self,
        session_id: &SessionId,
        digits: &str,
        duration_ms: u32,
        inter_digit_ms: u32,
    ) -> Result<()> {
        let exact = self.current_media(session_id).ok_or_else(|| {
            SessionError::SessionNotFound(format!("No media session for {}", session_id.0))
        })?;

        self.controller
            .send_dtmf_sequence_packets(&exact.dialog_id, digits, duration_ms, inter_digit_ms)
            .await
            .map_err(|_error| {
                SessionError::MediaError("RFC 4733 DTMF sequence dispatch failed".to_string())
            })?;
        if !self.media_is_still_exact(&exact) {
            return Err(SessionError::InvalidTransition(
                "media resource changed during DTMF sequence dispatch".to_string(),
            ));
        }

        tracing::info!(
            digit_count = digits.chars().count(),
            duration_ms,
            inter_digit_ms,
            "Queued bounded RFC 4733 DTMF sequence"
        );
        Ok(())
    }

    /// Set mute state
    pub async fn set_mute(
        &self,
        media_id: crate::types::MediaSessionId,
        muted: bool,
    ) -> Result<()> {
        let exact = self.current_media_by_dialog_id(&media_id).ok_or_else(|| {
            SessionError::SessionNotFound(format!(
                "No exact session owns media allocation {}",
                media_id
            ))
        })?;
        self.controller
            .set_audio_muted(&exact.dialog_id, muted)
            .await
            .map_err(|_| SessionError::MediaError("failed to update media mute state".into()))?;
        if !self.media_is_still_exact(&exact) {
            return Err(SessionError::InvalidTransition(
                "media resource changed while updating mute state".to_string(),
            ));
        }
        Ok(())
    }

    /// Start recording for media session
    pub async fn start_recording_media(
        &self,
        _media_id: crate::types::MediaSessionId,
    ) -> Result<()> {
        Err(unsupported_media_facade("start_recording_media"))
    }

    /// Stop recording for media session
    pub async fn stop_recording_media(
        &self,
        _media_id: crate::types::MediaSessionId,
    ) -> Result<()> {
        Err(unsupported_media_facade("stop_recording_media"))
    }

    // ===== Conference Methods =====

    /// Create an audio mixer for a conference
    pub async fn create_audio_mixer(&self) -> Result<crate::types::MediaSessionId> {
        Err(unsupported_media_facade("create_audio_mixer"))
    }

    /// Redirect audio to a mixer
    pub async fn redirect_to_mixer(
        &self,
        _media_id: crate::types::MediaSessionId,
        _mixer_id: crate::types::MediaSessionId,
    ) -> Result<()> {
        Err(unsupported_media_facade("redirect_to_mixer"))
    }

    /// Remove audio from a mixer
    pub async fn remove_from_mixer(
        &self,
        _media_id: crate::types::MediaSessionId,
        _mixer_id: crate::types::MediaSessionId,
    ) -> Result<()> {
        Err(unsupported_media_facade("remove_from_mixer"))
    }

    /// Destroy an audio mixer
    pub async fn destroy_mixer(&self, _mixer_id: crate::types::MediaSessionId) -> Result<()> {
        Err(unsupported_media_facade("destroy_mixer"))
    }

    /// Clean up lower media while the state-machine executor owns the exact
    /// session lane.
    ///
    /// This path must not publish `SessionStore`: the caller is holding the
    /// event-local working state and will commit it once after the complete
    /// ordered action list. The exact registry association is retired here so
    /// a later action in the same transition can create replacement media.
    pub(crate) async fn cleanup_session_lane_owned(&self, session: &SessionState) -> Result<()> {
        let handle = session.lifecycle_handle.as_ref().ok_or_else(|| {
            SessionError::InvalidTransition(
                "media cleanup requires exact session authority".to_string(),
            )
        })?;
        if handle.session_id() != &session.session_id {
            return Err(SessionError::InvalidTransition(
                "media cleanup lifecycle owner does not match its session".to_string(),
            ));
        }

        let session_id = handle.session_id();
        self.discard_pending_srtp_offer_exact(handle);
        self.discard_staged_media_negotiation_exact(handle);
        let retained_binding = self
            .media_resources
            .get(session_id)
            .filter(|binding| binding.handle == *handle)
            .map(|binding| binding.value().clone());
        let managed_resource = retained_binding
            .as_ref()
            .and_then(|binding| binding.resource.upgrade());
        let registry_media = self.store.registry().get_media_handle_exact(handle);
        let binding_media = retained_binding
            .as_ref()
            .map(|binding| binding.dialog_id.clone());
        let mut exact_media = None;
        for candidate in [
            registry_media,
            session.media_session_id.clone(),
            binding_media,
        ]
        .into_iter()
        .flatten()
        {
            if exact_media
                .as_ref()
                .is_some_and(|existing| existing != &candidate)
            {
                return Err(SessionError::InvalidTransition(
                    "lane-owned media owners disagree during cleanup".to_string(),
                ));
            }
            exact_media = Some(candidate);
        }

        let Some(dialog_id) = exact_media else {
            self.media_resources
                .remove_if(session_id, |_, binding| binding.handle == *handle);
            self.media_create_reservations
                .remove_if(session_id, |_, owner| owner == handle);
            tracing::debug!(
                session_id = %session_id,
                "lane-owned media cleanup was already complete"
            );
            return Ok(());
        };

        if let Some(resource) = managed_resource {
            resource.release_lower_once().await.map_err(|_| {
                SessionError::MediaError(
                    "Failed to release lane-owned media resource (class=media-core)".to_string(),
                )
            })?;
        } else {
            let guard = cleanup_diag::stage_guard(CleanupStage::MediaCleanup, &session_id.0);
            self.cleanup_attempt_total.fetch_add(1, Ordering::Relaxed);
            self.cleanup_fallback_total.fetch_add(1, Ordering::Relaxed);
            self.cleanup_mapped_total.fetch_add(1, Ordering::Relaxed);
            if self.signaling_only_local_port().is_none() {
                let _ = self
                    .controller
                    .remove_audio_frame_callback(&dialog_id)
                    .await;
                // media-core may already have released this exact allocation.
                let _ = self.controller.stop_media(&dialog_id).await;
            }
            if self
                .media_sessions
                .remove_if(session_id, |_, info| info.dialog_id == dialog_id)
                .is_some()
            {
                self.cleanup_media_session_removed_total
                    .fetch_add(1, Ordering::Relaxed);
            }
            if retained_binding.is_some() && self.audio_receivers.remove(session_id).is_some() {
                self.cleanup_audio_receiver_removed_total
                    .fetch_add(1, Ordering::Relaxed);
            }
            self.media_resources
                .remove_if(session_id, |_, binding| binding.handle == *handle);
            self.media_create_reservations
                .remove_if(session_id, |_, owner| owner == handle);
            cleanup_session_diag::record_cleanup();
            guard.finish_success();
        }

        match self
            .store
            .registry()
            .clear_media_handle_retained(handle, &dialog_id)
        {
            Ok(_)
            | Err(SessionRegistryError::SlotMissing)
            | Err(SessionRegistryError::RevisionMismatch) => {}
            Err(_) => {
                return Err(SessionError::MediaError(
                    "Failed to clear lane-owned media registry owner".to_string(),
                ));
            }
        }

        tracing::debug!(
            session_id = %session_id,
            %dialog_id,
            "cleaned lane-owned media without publishing session state"
        );
        Ok(())
    }

    /// Clean up all mappings and resources for a session.
    ///
    /// Idempotent — safe to call multiple times. Always removes the audio
    /// frame callback from media-core (so subscriber `rx.recv()` calls can
    /// return `None` and exit their loops) as long as the dialog mapping is
    /// still present.
    pub async fn cleanup_session(&self, session_id: &SessionId) -> Result<()> {
        let handle = self.store.lifecycle_handle(session_id).ok_or_else(|| {
            SessionError::SessionNotFound(format!(
                "Session {} has no current lifecycle handle",
                session_id.0
            ))
        })?;
        self.cleanup_session_exact(&handle).await
    }

    /// Clean up only media owned by one retained session lifetime. This path
    /// never synthesizes a deterministic raw-ID fallback; it reconciles only
    /// exact retained registry, state, and resource ownership for this
    /// generation.
    pub(crate) async fn cleanup_session_exact(&self, handle: &SessionRegistryHandle) -> Result<()> {
        let retained = self
            .store
            .get_session_retained_exact(handle)
            .await
            .map_err(|_| {
                SessionError::SessionNotFound(format!(
                    "Session {} exact lifetime is unavailable",
                    handle.session_id().0
                ))
            })?;
        let session_id = handle.session_id();
        self.discard_pending_srtp_offer_exact(handle);
        self.discard_staged_media_negotiation_exact(handle);
        let managed_resource = self
            .media_resources
            .get(session_id)
            .filter(|binding| binding.handle == *handle)
            .and_then(|binding| binding.resource.upgrade());
        if let Some(resource) = managed_resource {
            resource.release_once().await.map_err(|_| {
                SessionError::MediaError(
                    "Failed to release exact media resource (class=media-core)".to_string(),
                )
            })?;
            return Ok(());
        }
        let retained_binding = self
            .media_resources
            .get(session_id)
            .filter(|binding| binding.handle == *handle)
            .map(|binding| binding.value().clone());
        let registry_media = self.store.registry().get_media_retained_exact(handle);
        let state_media = retained.media_session_id.clone();
        let binding_media = retained_binding
            .as_ref()
            .map(|binding| binding.dialog_id.clone());
        let mut exact_media = None;
        for candidate in [registry_media, state_media, binding_media]
            .into_iter()
            .flatten()
        {
            if exact_media
                .as_ref()
                .is_some_and(|existing| existing != &candidate)
            {
                return Err(SessionError::InvalidTransition(
                    "retained media owners disagree during exact cleanup".to_string(),
                ));
            }
            exact_media = Some(candidate);
        }

        if exact_media.is_none() {
            self.media_resources
                .remove_if(session_id, |_, binding| binding.handle == *handle);
            tracing::debug!(
                session_id = %session_id,
                "exact media cleanup was already completed for this session lifetime"
            );
            return Ok(());
        }
        let guard = cleanup_diag::stage_guard(CleanupStage::MediaCleanup, &session_id.0);
        self.cleanup_attempt_total.fetch_add(1, Ordering::Relaxed);
        self.cleanup_fallback_total.fetch_add(1, Ordering::Relaxed);
        self.cleanup_mapped_total.fetch_add(1, Ordering::Relaxed);
        let dialog_id = exact_media.expect("exact retained media was checked above");
        if self.signaling_only_local_port().is_none() {
            let _ = self
                .controller
                .remove_audio_frame_callback(&dialog_id)
                .await;
            // media-core may already have released this exact allocation.
            let _ = self.controller.stop_media(&dialog_id).await;
        }

        self.store
            .clear_media_session_retained_exact(handle, &dialog_id)
            .map_err(|_| {
                SessionError::MediaError("Failed to clear exact retained media state".to_string())
            })?;
        match self
            .store
            .registry()
            .clear_media_handle_retained(handle, &dialog_id)
        {
            Ok(_)
            | Err(SessionRegistryError::SlotMissing)
            | Err(SessionRegistryError::RevisionMismatch) => {}
            Err(_) => {
                return Err(SessionError::MediaError(
                    "Failed to clear exact retained media registry owner".to_string(),
                ));
            }
        }

        if self
            .media_sessions
            .remove_if(session_id, |_, info| info.dialog_id == dialog_id)
            .is_some()
        {
            self.cleanup_media_session_removed_total
                .fetch_add(1, Ordering::Relaxed);
        }

        if retained_binding.is_some() && self.audio_receivers.remove(session_id).is_some() {
            self.cleanup_audio_receiver_removed_total
                .fetch_add(1, Ordering::Relaxed);
        }
        self.media_resources
            .remove_if(session_id, |_, binding| binding.handle == *handle);

        // NEXT_STEPS B diag — bump a process-global counter so the
        // perf_listener example can poll cleanup throughput without
        // grepping logs at 100+ msg/sec. Cleared & checked in the
        // dockerized sipp comparison harness.
        cleanup_session_diag::record_cleanup();

        tracing::debug!(
            "Cleaned up media adapter resources for session {}",
            session_id.0
        );
        guard.finish_success();
        Ok(())
    }

    // ===== Helper Methods =====

    /// Get local RTP port for a session
    fn get_local_port(&self, session_id: &SessionId) -> Result<u16> {
        self.media_sessions
            .get(session_id)
            .and_then(|info| info.rtp_port)
            .ok_or_else(|| {
                SessionError::SessionNotFound(format!("No local port for session {}", session_id.0))
            })
    }

    async fn generate_signaling_only_sdp_offer_lane_owned(
        &self,
        session: &mut SessionState,
        direction: crate::types::MediaDirection,
    ) -> Result<String> {
        let session_id = session.session_id.clone();
        let negotiation_key = Self::media_negotiation_key(session)?;
        let (origin_session_id, origin_version) = advance_sdp_origin(session);
        let origin_version = origin_version.to_string();
        let advertised_ip = self
            .public_rtp_addr()
            .map(|sa| sa.ip())
            .unwrap_or(self.local_ip);
        let local_ip_str = advertised_ip.to_string();
        let port = self.signaling_only_local_port().ok_or_else(|| {
            SessionError::MediaError(
                "signaling-only SDP requested while media mode is enabled".to_string(),
            )
        })?;
        let (transport, crypto_attrs) = if self.offer_srtp {
            let (negotiator, attrs) = SrtpNegotiator::new_offerer_with_base64_mode(
                &self.srtp_offered_suites,
                self.sdes_base64_mode,
            )?;
            self.pending_srtp_offerers
                .insert(negotiation_key, negotiator);
            ("RTP/SAVP", attrs)
        } else {
            ("RTP/AVP", Vec::new())
        };

        let format_pts = self.effective_offered_formats();
        let format_strings: Vec<String> = format_pts.iter().map(|pt| pt.to_string()).collect();
        let formats_ref: Vec<&str> = format_strings.iter().map(|s| s.as_str()).collect();
        let mut media_builder = SdpBuilder::new("Session")
            .origin(
                "-",
                &origin_session_id,
                &origin_version,
                "IN",
                "IP4",
                &local_ip_str,
            )
            .connection("IN", "IP4", &local_ip_str)
            .time("0", "0")
            .media_audio(port, transport)
            .formats(&formats_ref);
        for (pt, pt_str) in format_pts.iter().zip(format_strings.iter()) {
            if let Some(rtpmap) = rtpmap_for_pt(*pt) {
                media_builder = media_builder.rtpmap(pt_str.as_str(), rtpmap);
            }
            if let Some(fmtp) = fmtp_for_pt_with_g729_annex_b(*pt, self.g729_annex_b) {
                media_builder = media_builder.fmtp(pt_str.as_str(), fmtp);
            }
        }
        for attr in crypto_attrs {
            media_builder = media_builder.crypto_attribute(attr);
        }
        let session = media_builder
            .attribute(direction_attribute(direction), None::<String>)
            .done()
            .build()
            .map_err(|_| bounded_sdp_failure("signaling-offer-build", "builder"))?;

        tracing::info!(
            "signaling-only media mode: generated SDP for session {} with advertised port {}",
            session_id.0,
            port
        );
        Ok(session.to_string())
    }

    fn signaling_only_local_port(&self) -> Option<u16> {
        match self.media_mode {
            MediaMode::Enabled => None,
            MediaMode::SignalingOnly { sdp_rtp_port } => Some(
                self.public_rtp_addr()
                    .filter(|addr| addr.port() != 0)
                    .map(|addr| addr.port())
                    .unwrap_or(sdp_rtp_port),
            ),
        }
    }

    /// Parse SDP to extract connection info from the audio m= section.
    ///
    /// Uses sip-core's typed `SdpSession::from_str` parser instead of
    /// the previous bespoke line-scanner so that future SDP work
    /// (`a=crypto:` for SDES, `a=fingerprint:`/`a=setup:` for
    /// DTLS-SRTP, video m= sections, RFC 8866 conformance) gets
    /// validation for free.
    ///
    /// Per RFC 8866 §5.7 the m-section's own `c=` line (if present)
    /// overrides the session-level `c=`. We honour that.
    fn parse_sdp_connection(&self, sdp: &str) -> Result<(IpAddr, u16)> {
        let session = SdpSession::from_str(sdp)
            .map_err(|_| bounded_sdp_failure("connection-extract", "syntax"))?;

        let media = session
            .media_descriptions
            .iter()
            .find(|m| m.media == "audio")
            .ok_or_else(|| {
                SessionError::SDPNegotiationFailed("SDP has no audio m= section".into())
            })?;

        let port = media.port;

        // Prefer the per-media c= line; fall back to session-level.
        let conn = media
            .connection_info
            .as_ref()
            .or(session.connection_info.as_ref())
            .ok_or_else(|| {
                SessionError::SDPNegotiationFailed(
                    "SDP has no c= line at session or audio level".into(),
                )
            })?;

        let ip = conn
            .connection_address
            .parse::<IpAddr>()
            .map_err(|_| bounded_sdp_failure("connection-extract", "address"))?;

        Ok((ip, port))
    }

    // ===== Event handling removed - now centralized in SessionCrossCrateEventHandler ====="

    // ===== Recording Management =====

    /// Start recording for a session (simple version for backward compatibility)
    pub async fn start_recording(&self, session_id: &SessionId) -> Result<String> {
        // Use default config for backward compatibility
        let config = RecordingConfig::default();
        self.start_recording_with_config(session_id, config).await
    }

    /// Start recording for a session with specific config
    pub async fn start_recording_with_config(
        &self,
        session_id: &SessionId,
        _config: RecordingConfig,
    ) -> Result<String> {
        self.current_media(session_id).ok_or_else(|| {
            SessionError::MediaError(format!("No dialog for session {}", session_id.0))
        })?;
        Err(unsupported_media_facade("start_recording_with_config"))
    }

    /// Stop recording for a session (simple version for backward compatibility)
    pub async fn stop_recording(&self, session_id: &SessionId) -> Result<()> {
        self.current_media(session_id).ok_or_else(|| {
            SessionError::MediaError(format!("No dialog for session {}", session_id.0))
        })?;
        Err(unsupported_media_facade("stop_recording"))
    }

    /// Stop recording for a session with specific recording ID
    pub async fn stop_recording_with_id(
        &self,
        session_id: &SessionId,
        _recording_id: &str,
    ) -> Result<()> {
        self.current_media(session_id).ok_or_else(|| {
            SessionError::MediaError(format!("No dialog for session {}", session_id.0))
        })?;
        Err(unsupported_media_facade("stop_recording_with_id"))
    }

    /// Pause recording for a session
    pub async fn pause_recording(&self, session_id: &SessionId, _recording_id: &str) -> Result<()> {
        self.current_media(session_id).ok_or_else(|| {
            SessionError::MediaError(format!("No dialog for session {}", session_id.0))
        })?;
        Err(unsupported_media_facade("pause_recording"))
    }

    /// Resume a paused recording
    pub async fn resume_recording(
        &self,
        session_id: &SessionId,
        _recording_id: &str,
    ) -> Result<()> {
        self.current_media(session_id).ok_or_else(|| {
            SessionError::MediaError(format!("No dialog for session {}", session_id.0))
        })?;
        Err(unsupported_media_facade("resume_recording"))
    }

    /// Get recording status
    pub async fn get_recording_status(
        &self,
        session_id: &SessionId,
        _recording_id: &str,
    ) -> Result<RecordingStatus> {
        self.current_media(session_id).ok_or_else(|| {
            SessionError::MediaError(format!("No dialog for session {}", session_id.0))
        })?;
        Err(unsupported_media_facade("get_recording_status"))
    }

    /// Start recording for a bridged session pair
    pub async fn start_bridge_recording(
        &self,
        session1: &SessionId,
        _session2: &SessionId,
        _config: RecordingConfig,
    ) -> Result<String> {
        self.current_media(session1).ok_or_else(|| {
            SessionError::MediaError(format!("No dialog for session {}", session1.0))
        })?;
        Err(unsupported_media_facade("start_bridge_recording"))
    }

    /// Enable/disable recording for all conference sessions
    pub async fn set_conference_recording_enabled(&self, _enabled: bool) -> Result<()> {
        Err(unsupported_media_facade("set_conference_recording_enabled"))
    }
}

impl Clone for MediaAdapter {
    fn clone(&self) -> Self {
        Self {
            controller: self.controller.clone(),
            store: self.store.clone(),
            media_create_reservations: self.media_create_reservations.clone(),
            media_resources: self.media_resources.clone(),
            media_sessions: self.media_sessions.clone(),
            audio_receivers: self.audio_receivers.clone(),
            session_create_attempt_total: self.session_create_attempt_total.clone(),
            session_create_success_total: self.session_create_success_total.clone(),
            session_create_failed_total: self.session_create_failed_total.clone(),
            cleanup_attempt_total: self.cleanup_attempt_total.clone(),
            cleanup_mapped_total: self.cleanup_mapped_total.clone(),
            cleanup_fallback_total: self.cleanup_fallback_total.clone(),
            cleanup_media_session_removed_total: self.cleanup_media_session_removed_total.clone(),
            cleanup_audio_receiver_removed_total: self.cleanup_audio_receiver_removed_total.clone(),
            audio_subscriber_created_total: self.audio_subscriber_created_total.clone(),
            audio_subscriber_disconnected_total: self.audio_subscriber_disconnected_total.clone(),
            audio_send_frames_total: self.audio_send_frames_total.clone(),
            audio_send_samples_total: self.audio_send_samples_total.clone(),
            local_ip: self.local_ip,
            media_port_start: self.media_port_start,
            media_port_end: self.media_port_end,
            media_mode: self.media_mode,
            offer_srtp: self.offer_srtp,
            srtp_required: self.srtp_required,
            srtp_offered_suites: self.srtp_offered_suites.clone(),
            sdes_base64_mode: self.sdes_base64_mode,
            pending_srtp_offerers: self.pending_srtp_offerers.clone(),
            negotiated_srtp: self.negotiated_srtp.clone(),
            staged_media_negotiations: self.staged_media_negotiations.clone(),
            global_coordinator: self.global_coordinator.clone(),
            app_event_publisher: self.app_event_publisher.clone(),
            public_rtp_addr: std::sync::RwLock::new(self.public_rtp_addr()),
            comfort_noise_enabled: self.comfort_noise_enabled,
            strict_codec_matching: self.strict_codec_matching,
            offered_codecs: self.offered_codecs.clone(),
            g729_annex_b: self.g729_annex_b,
            #[cfg(test)]
            pause_media_create_after_allocation: self.pause_media_create_after_allocation.clone(),
            #[cfg(test)]
            media_create_allocated: self.media_create_allocated.clone(),
            #[cfg(test)]
            resume_media_create: self.resume_media_create.clone(),
            #[cfg(test)]
            fail_media_commit_after_srtp_swap: self.fail_media_commit_after_srtp_swap.clone(),
            #[cfg(test)]
            fail_media_rollback: self.fail_media_rollback.clone(),
            #[cfg(test)]
            fail_staged_media_commit: self.fail_staged_media_commit.clone(),
        }
    }
}

/// Sprint 3.5 — compute the answer's `m=audio` format list from the
/// offer + our policy flags. Pure (no `MediaAdapter` state) so unit
/// tests can exercise the strict-vs-permissive logic without standing
/// up a coordinator.
///
/// Returns the formats in the order they should appear on the wire.
/// Caller is responsible for emitting the matching `a=rtpmap:` /
/// `a=fmtp:` lines.
///
/// `Err(SDPNegotiationFailed)` when:
/// - Strict mode + offer carries no overlap with our supported set
///   → state machine surfaces this as `488 Not Acceptable Here`.
/// - Strict mode + matcher rejects on SRTP policy (e.g. `require_srtp`
///   set + offer is plain RTP/AVP).
pub(crate) fn compute_answer_formats(
    offer: &SdpSession,
    offered_codecs: &[u8],
    strict: bool,
    offer_srtp: bool,
    srtp_required: bool,
) -> Result<Vec<String>> {
    let mut supported: Vec<String> = offered_codecs.iter().map(|pt| pt.to_string()).collect();

    // Dynamic payload numbers belong to the offer, not to a codec. Our
    // capability list uses fixed numbers to *represent* support; when the peer
    // maps the same codec to a different dynamic PT, match and answer using
    // that exact number.
    //
    // AMR needs this at least as much as Opus does: both its variants are
    // dynamic, and Asterisk and most handsets pick their own numbers. Without
    // it a peer offering AMR-WB on 96 got a 488 for a codec we support.
    let amr_wb_offered = offered_codecs.contains(&AMR_WB_BE_PT)
        || offered_codecs.contains(&AMR_WB_OA_PT);
    let amr_nb_offered = offered_codecs.contains(&AMR_NB_BE_PT)
        || offered_codecs.contains(&AMR_NB_OA_PT);
    if (cfg!(feature = "amr-wb") && amr_wb_offered) || (cfg!(feature = "amr-nb") && amr_nb_offered)
    {
        if let Some(audio) = offer
            .media_descriptions
            .iter()
            .find(|media| media.media.eq_ignore_ascii_case("audio"))
        {
            for format in &audio.formats {
                let Ok(payload_type) = format.parse::<u8>() else {
                    continue;
                };
                if !(96..=127).contains(&payload_type) || payload_type == 101 {
                    continue;
                }
                let Some(mapping) = audio_rtpmap(offer, payload_type) else {
                    continue;
                };
                let wanted = (mapping.encoding_name.eq_ignore_ascii_case("AMR-WB")
                    && cfg!(feature = "amr-wb")
                    && amr_wb_offered)
                    || (mapping.encoding_name.eq_ignore_ascii_case("AMR")
                        && cfg!(feature = "amr-nb")
                        && amr_nb_offered);
                if wanted && !supported.contains(format) {
                    supported.push(format.clone());
                }
            }
        }
    }

    if cfg!(feature = "opus") && offered_codecs.contains(&111) {
        if let Some(audio) = offer
            .media_descriptions
            .iter()
            .find(|media| media.media.eq_ignore_ascii_case("audio"))
        {
            for format in &audio.formats {
                let Ok(payload_type) = format.parse::<u8>() else {
                    continue;
                };
                if (96..=127).contains(&payload_type)
                    && payload_type != 101
                    && audio_rtpmap(offer, payload_type)
                        .is_some_and(|mapping| mapping.encoding_name.eq_ignore_ascii_case("opus"))
                    && !supported.contains(format)
                {
                    supported.push(format.clone());
                }
            }
        }
    }

    let candidates = if strict {
        let caps = rvoip_sip_dialog::sdp::AnswerCapabilities {
            supported_formats: supported.clone(),
            accept_srtp: offer_srtp,
            require_srtp: srtp_required,
        };
        let matched = rvoip_sip_dialog::sdp::match_offer(offer, &caps)
            .map_err(|_| bounded_sdp_failure("format-match", "policy"))?;
        let line = matched
            .media_lines
            .iter()
            .find(|line| line.media == "audio")
            .ok_or_else(|| {
                SessionError::SDPNegotiationFailed("matcher returned no audio media line".into())
            })?;
        if !line.accepted {
            return Err(SessionError::SDPNegotiationFailed(
                "no codec overlap with offer".into(),
            ));
        }
        line.negotiated_formats.clone()
    } else {
        // Even compatibility mode must obey RFC 3264: an answer cannot add
        // payloads the offer did not contain. It only bypasses the stricter
        // transport-policy matcher.
        let audio = offer
            .media_descriptions
            .iter()
            .find(|media| media.media.eq_ignore_ascii_case("audio"))
            .ok_or_else(|| bounded_sdp_failure("format-match", "missing-audio"))?;
        audio
            .formats
            .iter()
            .filter(|format| supported.contains(format))
            .cloned()
            .collect()
    };

    let mut primary = None;
    let mut auxiliary = Vec::new();
    for format in candidates {
        let payload_type = format
            .parse::<u8>()
            .map_err(|_| bounded_sdp_failure("format-match", "invalid-payload"))?;
        if !sdp_payload_codec_available(offer, payload_type) {
            continue;
        }
        if matches!(payload_type, 13 | 101) {
            auxiliary.push(format);
        } else if primary.is_none() {
            primary = Some(format);
        }
    }

    let primary = primary.ok_or_else(|| bounded_sdp_failure("format-match", "no-primary"))?;
    let mut answer = Vec::with_capacity(1 + auxiliary.len());
    answer.push(primary);
    answer.extend(auxiliary);
    Ok(answer)
}

#[cfg(test)]
mod sdp_format_tests {
    //! Byte-fixture regression tests for the format-strings →
    //! `SdpBuilder` refactor (Step 2B.1, decision D11). Builds the same
    //! SDP via the typed builder and asserts byte-identical output to
    //! what the previous `format!` block would have produced.
    //!
    //! When the SRTP offer landing in 2B.2 changes the m= transport to
    //! `RTP/SAVP` and adds `a=crypto:` lines, these tests will need a
    //! second fixture for that case.

    use super::*;

    fn build_srtp_answer(attr: Option<CryptoAttribute>) -> String {
        let mut media = SdpBuilder::new("Session")
            .origin("-", "answer", "0", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(
                18_000,
                if attr.is_some() {
                    "RTP/SAVP"
                } else {
                    "RTP/AVP"
                },
            )
            .formats(&["0"])
            .rtpmap("0", "PCMU/8000");
        if let Some(attr) = attr {
            media = media.crypto_attribute(attr);
        }
        media.done().build().expect("answer builds").to_string()
    }

    #[tokio::test]
    async fn legacy_bridge_facades_never_report_false_success() {
        use crate::session_store::SessionStore;
        use rvoip_media_core::relay::controller::MediaSessionController;
        use std::net::{IpAddr, Ipv4Addr};

        let adapter = MediaAdapter::new(
            Arc::new(MediaSessionController::new()),
            Arc::new(SessionStore::new()),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            16_000,
            16_100,
        );
        let first = SessionId::new();
        let second = SessionId::new();

        assert!(matches!(
            adapter.create_bridge(&first, &second).await,
            Err(SessionError::InvalidTransition(_))
        ));
        assert!(matches!(
            adapter.destroy_bridge(&first).await,
            Err(SessionError::InvalidTransition(_))
        ));
    }

    #[tokio::test]
    async fn unsupported_media_facades_never_report_false_success() {
        use crate::session_store::SessionStore;
        use rvoip_media_core::relay::controller::MediaSessionController;
        use std::net::{IpAddr, Ipv4Addr};

        let adapter = MediaAdapter::new(
            Arc::new(MediaSessionController::new()),
            Arc::new(SessionStore::new()),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            16_000,
            16_100,
        );
        let media_id = crate::types::MediaSessionId::new_v4();

        assert!(adapter.create_media_session().await.is_err());
        assert!(adapter.stop_media_session(media_id.clone()).await.is_err());
        assert!(adapter
            .start_recording_media(media_id.clone())
            .await
            .is_err());
        assert!(adapter
            .stop_recording_media(media_id.clone())
            .await
            .is_err());
        assert!(adapter.create_audio_mixer().await.is_err());
        assert!(adapter
            .redirect_to_mixer(media_id.clone(), media_id.clone())
            .await
            .is_err());
        assert!(adapter
            .remove_from_mixer(media_id.clone(), media_id.clone())
            .await
            .is_err());
        assert!(adapter.destroy_mixer(media_id).await.is_err());
        assert!(adapter
            .set_conference_recording_enabled(true)
            .await
            .is_err());

        let source = include_str!("media_adapter.rs");
        for (left, right) in [
            ("Recording started", " at:"),
            ("Recording", " stopped"),
            ("For now, we'll generate", " a simple recording ID"),
            ("For now, return", " a mock status"),
            ("For now, just return", " Ok"),
            ("audio_mixers: Arc", "<DashMap"),
        ] {
            let retired = format!("{left}{right}");
            assert!(
                !source.contains(&retired),
                "retired facade remains: {retired}"
            );
        }
    }

    #[tokio::test]
    async fn signaling_only_hold_and_resume_sdp_need_no_media_allocation() {
        use crate::session_store::SessionStore;
        use crate::state_table::types::Role;
        use rvoip_media_core::relay::controller::MediaSessionController;
        use std::net::{IpAddr, Ipv4Addr};

        let store = Arc::new(SessionStore::new());
        let session_id = SessionId("signaling-only-hold-resume".to_string());
        store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create signaling-only session");

        let mut adapter = MediaAdapter::new(
            Arc::new(MediaSessionController::new()),
            store,
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            16_000,
            16_100,
        );
        adapter.set_media_mode(MediaMode::SignalingOnly { sdp_rtp_port: 9 });

        let hold = adapter
            .create_hold_sdp_for_session(&session_id)
            .await
            .expect("signaling-only hold SDP");
        let resume = adapter
            .create_active_sdp_for_session(&session_id)
            .await
            .expect("signaling-only resume SDP");

        assert!(hold.contains("m=audio 9 RTP/AVP"), "{hold}");
        assert!(hold.contains("a=sendonly"), "{hold}");
        assert!(resume.contains("m=audio 9 RTP/AVP"), "{resume}");
        assert!(resume.contains("a=sendrecv"), "{resume}");
        assert!(adapter.media_sessions.is_empty());
        assert!(adapter.media_resources.is_empty());
    }

    #[tokio::test]
    async fn malformed_sdes_answer_preserves_offer_state_for_valid_retry() {
        use crate::state_table::types::Role;
        use rvoip_media_core::relay::controller::MediaSessionController;
        use std::net::{IpAddr, Ipv4Addr};

        let mut adapter = MediaAdapter::new(
            Arc::new(MediaSessionController::new()),
            Arc::new(SessionStore::new()),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            16_000,
            16_100,
        );
        adapter.set_media_mode(MediaMode::SignalingOnly { sdp_rtp_port: 9 });
        adapter.set_srtp_policy(true, true, vec![CryptoSuite::AesCm128HmacSha1_80]);
        let mut session = SessionState::new(SessionId("sdes-answer-retry".into()), Role::UAC);
        let offer = adapter
            .generate_local_sdp_offer_lane_owned(
                &mut session,
                crate::types::MediaDirection::SendRecv,
            )
            .await
            .expect("generate SRTP offer");
        session.local_sdp = Some(offer.clone());
        let negotiation_key = MediaAdapter::media_negotiation_key(&session).unwrap();
        let offered_crypto =
            MediaAdapter::extract_audio_crypto(&SdpSession::from_str(&offer).unwrap());
        let mut malformed_attr = offered_crypto[0].clone();
        malformed_attr.key_inline = "not-base64".to_string();

        assert!(
            adapter
                .negotiate_sdp_as_uac_lane_owned(
                    &mut session,
                    &build_srtp_answer(Some(malformed_attr)),
                )
                .await
                .is_err()
        );
        assert!(
            adapter.pending_srtp_offerers.contains_key(&negotiation_key),
            "a rejected answer must not consume the offerer's key state"
        );
        assert!(!adapter
            .staged_media_negotiations
            .contains_key(&negotiation_key));
        assert!(!adapter.negotiated_srtp.contains_key(&negotiation_key));

        let answerer = SrtpNegotiator::new_answerer();
        let (answer_attr, _) = answerer
            .process_offer(&offered_crypto)
            .expect("build valid answer keys");
        adapter
            .negotiate_sdp_as_uac_lane_owned(&mut session, &build_srtp_answer(Some(answer_attr)))
            .await
            .expect("the same offer accepts a corrected answer");
        assert!(
            adapter.pending_srtp_offerers.contains_key(&negotiation_key),
            "provisional negotiation must retain key state until commit"
        );

        adapter
            .commit_staged_media_negotiation_lane_owned(&mut session)
            .await
            .expect("commit corrected answer");
        assert!(
            !adapter.pending_srtp_offerers.contains_key(&negotiation_key),
            "successful finalization consumes the offerer's key state"
        );
        assert!(session.media_security.is_some());
    }

    #[tokio::test]
    async fn missing_sdes_offer_state_fails_closed_without_staging_plaintext() {
        use crate::state_table::types::Role;
        use rvoip_media_core::relay::controller::MediaSessionController;
        use std::net::{IpAddr, Ipv4Addr};

        let mut adapter = MediaAdapter::new(
            Arc::new(MediaSessionController::new()),
            Arc::new(SessionStore::new()),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            16_000,
            16_100,
        );
        adapter.set_media_mode(MediaMode::SignalingOnly { sdp_rtp_port: 9 });
        adapter.set_srtp_policy(true, true, vec![CryptoSuite::AesCm128HmacSha1_80]);
        let mut session = SessionState::new(SessionId("missing-sdes-state".into()), Role::UAC);
        let offer = adapter
            .generate_local_sdp_offer_lane_owned(
                &mut session,
                crate::types::MediaDirection::SendRecv,
            )
            .await
            .expect("generate SRTP offer");
        session.local_sdp = Some(offer);
        let negotiation_key = MediaAdapter::media_negotiation_key(&session).unwrap();
        adapter.pending_srtp_offerers.remove(&negotiation_key);

        let error = adapter
            .negotiate_sdp_as_uac_lane_owned(&mut session, &build_srtp_answer(None))
            .await
            .expect_err("lost offer state must not downgrade to plaintext");
        assert!(
            matches!(
                error.downcast_ref::<SessionError>(),
                Some(SessionError::SDPNegotiationFailed(detail))
                    if detail.contains("pending SDES key state is unavailable")
            ),
            "lost SDES state returned the wrong failure class"
        );
        assert!(!adapter
            .staged_media_negotiations
            .contains_key(&negotiation_key));
        assert!(!adapter.negotiated_srtp.contains_key(&negotiation_key));
        assert!(session.media_security.is_none());
    }

    #[test]
    fn inbound_sdes_preflight_rejects_malformed_key_material_before_state_mutation() {
        use rvoip_media_core::relay::controller::MediaSessionController;
        use std::net::{IpAddr, Ipv4Addr};

        let mut adapter = MediaAdapter::new(
            Arc::new(MediaSessionController::new()),
            Arc::new(SessionStore::new()),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            16_000,
            16_100,
        );
        adapter.set_srtp_policy(true, true, vec![CryptoSuite::AesCm128HmacSha1_80]);
        let malformed = CryptoAttribute::new(1, CryptoSuite::AesCm128HmacSha1_80, "not-base64");

        let error = adapter
            .validate_inbound_sdp_offer(&build_srtp_answer(Some(malformed)))
            .expect_err("malformed SDES offer must fail preflight");
        let diagnostic = error
            .downcast_ref::<crate::adapters::srtp_negotiator::SdesNegotiationFailure>()
            .expect("preflight preserves the structured SDES diagnostic")
            .diagnostic();
        assert_eq!(
            diagnostic.stage,
            crate::errors::SdesNegotiationStage::RemoteOffer
        );
        assert_eq!(
            diagnostic.failure_class,
            crate::errors::SdesNegotiationFailureClass::InvalidBase64
        );
    }

    #[cfg(feature = "perf-tests")]
    #[test]
    fn deleted_adapter_mappings_keep_zero_compatibility_diagnostics() {
        use crate::session_store::SessionStore;
        use rvoip_media_core::relay::controller::MediaSessionController;
        use std::net::{IpAddr, Ipv4Addr};

        let adapter = MediaAdapter::new(
            Arc::new(MediaSessionController::new()),
            Arc::new(SessionStore::new()),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            16_000,
            16_100,
        );

        assert_eq!(
            adapter.perf_diagnostic_counts()["session_to_dialog"].as_u64(),
            Some(0),
            "the removed forward map must remain a stable zero-valued diagnostic"
        );
        assert_eq!(
            adapter.perf_diagnostic_counts()["dialog_to_session"].as_u64(),
            Some(0),
            "the removed reverse map must remain a stable zero-valued diagnostic"
        );
        assert_eq!(
            adapter.perf_diagnostic_counts()["registry_media_bindings"].as_u64(),
            Some(0)
        );
        assert_eq!(
            adapter.perf_diagnostic_counts()["media_resources"].as_u64(),
            Some(0)
        );
        assert_eq!(
            adapter.perf_diagnostic_counts()["audio_mixers"].as_u64(),
            Some(0),
            "the removed metadata-only mixer projection remains a stable zero diagnostic"
        );
    }

    #[tokio::test]
    async fn remote_offer_parse_failure_is_bounded_before_state_machine_logging() {
        use crate::session_store::SessionStore;
        use rvoip_media_core::relay::controller::MediaSessionController;
        use std::net::Ipv4Addr;

        let adapter = MediaAdapter::new(
            Arc::new(MediaSessionController::new()),
            Arc::new(SessionStore::new()),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            16000,
            16100,
        );
        let canary = "FAST_AUTOACCEPT_SDP_CANARY_5ed91b";
        let malformed_offer = format!(
            "v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\ns=x\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio {canary} RTP/AVP 0\r\n"
        );

        let error = adapter
            .negotiate_sdp_as_uas(&SessionId("bounded-sdp-error".into()), &malformed_offer)
            .await
            .unwrap_err();
        let display = error.to_string();
        let debug = format!("{error:?}");
        assert!(matches!(
            &error,
            SessionError::SDPNegotiationFailed(detail)
                if detail == "SDP negotiation failed (stage=remote-offer, class=syntax)"
        ));
        assert!(!display.contains(canary));
        assert!(!debug.contains(canary));
    }

    #[test]
    fn offer_direction_maps_to_correct_answer_direction() {
        use rvoip_sip_core::MediaDirection as SipDirection;

        assert_eq!(
            answer_direction_for_offer(&Some(SipDirection::SendRecv)),
            crate::types::MediaDirection::SendRecv
        );
        assert_eq!(
            answer_direction_for_offer(&Some(SipDirection::SendOnly)),
            crate::types::MediaDirection::RecvOnly
        );
        assert_eq!(
            answer_direction_for_offer(&Some(SipDirection::RecvOnly)),
            crate::types::MediaDirection::SendOnly
        );
        assert_eq!(
            answer_direction_for_offer(&Some(SipDirection::Inactive)),
            crate::types::MediaDirection::Inactive
        );
        assert_eq!(
            answer_direction_for_offer(&None),
            crate::types::MediaDirection::SendRecv
        );
    }

    #[test]
    fn remote_answer_direction_maps_to_local_direction() {
        use rvoip_sip_core::MediaDirection as SipDirection;

        assert_eq!(
            local_direction_from_remote_answer(&Some(SipDirection::SendRecv)),
            crate::types::MediaDirection::SendRecv
        );
        assert_eq!(
            local_direction_from_remote_answer(&Some(SipDirection::SendOnly)),
            crate::types::MediaDirection::RecvOnly
        );
        assert_eq!(
            local_direction_from_remote_answer(&Some(SipDirection::RecvOnly)),
            crate::types::MediaDirection::SendOnly
        );
        assert_eq!(
            local_direction_from_remote_answer(&Some(SipDirection::Inactive)),
            crate::types::MediaDirection::Inactive
        );
        assert_eq!(
            local_direction_from_remote_answer(&None),
            crate::types::MediaDirection::SendRecv
        );
    }

    #[test]
    fn g729_fmtp_reflects_annex_b_flag() {
        assert_eq!(fmtp_for_pt_with_g729_annex_b(18, true), Some("annexb=yes"));
        assert_eq!(fmtp_for_pt_with_g729_annex_b(18, false), Some("annexb=no"));
        assert_eq!(fmtp_for_pt(101), Some("0-15"));
    }

    #[test]
    fn g729_annex_b_negotiation_honors_remote_no() {
        let sdp = "v=0\r\n\
o=- 1 1 IN IP4 127.0.0.1\r\n\
s=Session\r\n\
c=IN IP4 127.0.0.1\r\n\
t=0 0\r\n\
m=audio 17000 RTP/AVP 18 101\r\n\
a=rtpmap:18 G729/8000\r\n\
a=fmtp:18 annexb=no\r\n\
a=rtpmap:101 telephone-event/8000\r\n\
a=fmtp:101 0-15\r\n";
        let session = SdpSession::from_str(sdp).expect("sdp parses");

        assert!(!negotiated_g729_annex_b(&session, true));
        assert!(!negotiated_g729_annex_b(&session, false));
        assert_eq!(
            select_primary_audio_payload_from_session(&session),
            Some(18)
        );
        assert_eq!(codec_name_for_payload(18, false), "G729A");
        assert_eq!(codec_name_for_payload(AMR_WB_BE_PT, false), "AMR-WB");
        assert_eq!(codec_name_for_payload(AMR_WB_OA_PT, false), "AMR-WB");
        assert_eq!(codec_name_for_payload(AMR_NB_BE_PT, false), "AMR");
        assert_eq!(codec_name_for_payload(AMR_NB_OA_PT, false), "AMR");
    }

    #[test]
    fn audio_direction_prefers_media_level_direction() {
        use rvoip_sip_core::MediaDirection as SipDirection;

        let sdp = SdpBuilder::new("Session")
            .origin("-", "1", "0", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .direction(SipDirection::SendRecv)
            .media_audio(16000, "RTP/AVP")
            .formats(&["0"])
            .rtpmap("0", "PCMU/8000")
            .direction(SipDirection::SendOnly)
            .done()
            .build()
            .expect("offer builds");

        assert_eq!(audio_direction(&sdp), Some(SipDirection::SendOnly));
    }

    /// Build the offer the same way `generate_local_sdp` does, but with
    /// fixed inputs so the output is deterministic. Mirrors the
    /// production shape: PCMU + PCMA + telephone-event, RTP/AVP profile.
    fn build_offer(dialog_id: &str, elapsed_secs: u64, ip: &str, port: u16) -> String {
        let elapsed = elapsed_secs.to_string();
        let origin_session_id = sdp_origin_session_id(dialog_id);
        SdpBuilder::new("Session")
            .origin("-", &origin_session_id, &elapsed, "IN", "IP4", ip)
            .connection("IN", "IP4", ip)
            .time("0", "0")
            .media_audio(port, "RTP/AVP")
            .formats(&["0", "8", "101"])
            .rtpmap("0", "PCMU/8000")
            .rtpmap("8", "PCMA/8000")
            .rtpmap("101", "telephone-event/8000")
            .fmtp("101", "0-15")
            .attribute("sendrecv", None::<String>)
            .done()
            .build()
            .expect("offer builds")
            .to_string()
    }

    /// Same as `build_offer` but for the answer (different sess_version).
    fn build_answer(sess_id: &str, ip: &str, port: u16) -> String {
        SdpBuilder::new("Session")
            .origin("-", sess_id, "0", "IN", "IP4", ip)
            .connection("IN", "IP4", ip)
            .time("0", "0")
            .media_audio(port, "RTP/AVP")
            .formats(&["0", "8", "101"])
            .rtpmap("0", "PCMU/8000")
            .rtpmap("8", "PCMA/8000")
            .rtpmap("101", "telephone-event/8000")
            .fmtp("101", "0-15")
            .attribute("sendrecv", None::<String>)
            .done()
            .build()
            .expect("answer builds")
            .to_string()
    }

    /// Reference output for the unified offer (Sprint 2.5 P2): PCMU +
    /// PCMA + telephone-event on every offer regardless of SRTP. Pre-P2
    /// the non-SRTP path emitted only `0 8` (no DTMF) and the SRTP path
    /// only `0 101` (no PCMA); both have been merged into the unified
    /// shape below.
    fn legacy_offer(dialog_id: &str, elapsed_secs: u64, ip: &str, port: u16) -> String {
        let origin_session_id = sdp_origin_session_id(dialog_id);
        format!(
            "v=0\r\n\
             o=- {} {} IN IP4 {}\r\n\
             s=Session\r\n\
             c=IN IP4 {}\r\n\
             t=0 0\r\n\
             m=audio {} RTP/AVP 0 8 101\r\n\
             a=rtpmap:0 PCMU/8000\r\n\
             a=rtpmap:8 PCMA/8000\r\n\
             a=rtpmap:101 telephone-event/8000\r\n\
             a=fmtp:101 0-15\r\n\
             a=sendrecv\r\n",
            origin_session_id, elapsed_secs, ip, ip, port,
        )
    }

    fn legacy_answer(sess_id: &str, ip: &str, port: u16) -> String {
        format!(
            "v=0\r\n\
             o=- {} {} IN IP4 {}\r\n\
             s=Session\r\n\
             c=IN IP4 {}\r\n\
             t=0 0\r\n\
             m=audio {} RTP/AVP 0 8 101\r\n\
             a=rtpmap:0 PCMU/8000\r\n\
             a=rtpmap:8 PCMA/8000\r\n\
             a=rtpmap:101 telephone-event/8000\r\n\
             a=fmtp:101 0-15\r\n\
             a=sendrecv\r\n",
            sess_id, 0u64, ip, ip, port,
        )
    }

    #[test]
    fn offer_matches_legacy_format_byte_for_byte() {
        let dialog_id = "test-dialog-uuid";
        let elapsed = 42u64;
        let ip = "127.0.0.1";
        let port = 16000;
        let new = build_offer(dialog_id, elapsed, ip, port);
        let old = legacy_offer(dialog_id, elapsed, ip, port);
        assert_eq!(
            new, old,
            "SdpBuilder offer drifted from legacy format-string output"
        );
    }

    #[test]
    fn answer_matches_legacy_format_byte_for_byte() {
        let sess_id = "1234567890";
        let ip = "192.168.1.42";
        let port = 16002;
        let new = build_answer(sess_id, ip, port);
        let old = legacy_answer(sess_id, ip, port);
        assert_eq!(
            new, old,
            "SdpBuilder answer drifted from legacy format-string output"
        );
    }

    #[test]
    fn offer_round_trips_through_typed_parser() {
        // Build → parse → assert key fields. Catches CRLF / spacing
        // issues that would also break peer interop.
        let sdp_str = build_offer("d", 0, "10.0.0.1", 5004);
        let parsed = SdpSession::from_str(&sdp_str).expect("parses back");
        assert_eq!(parsed.session_name, "Session");
        assert_eq!(parsed.media_descriptions.len(), 1);
        let m = &parsed.media_descriptions[0];
        assert_eq!(m.media, "audio");
        assert_eq!(m.port, 5004);
        assert_eq!(m.protocol, "RTP/AVP");
        assert_eq!(m.formats, vec!["0", "8", "101"]);
    }

    #[test]
    fn offer_origin_uses_numeric_session_id() {
        let sdp = build_offer(
            "media-session-3e071bea-bda5-4758-bf05-8bce16c690e6",
            0,
            "10.0.0.1",
            5004,
        );
        let origin = sdp
            .lines()
            .find(|line| line.starts_with("o="))
            .expect("origin line");
        let fields: Vec<&str> = origin.trim_start_matches("o=").split_whitespace().collect();

        assert_eq!(fields.len(), 6, "origin line was: {}", origin);
        assert!(
            fields[1].bytes().all(|b| b.is_ascii_digit()),
            "SDP sess-id must be numeric for PJMEDIA/Asterisk interop: {}",
            origin
        );
        assert!(
            fields[1].len() <= 20,
            "SDP sess-id should fit common 64-bit parser limits: {}",
            origin
        );
    }

    /// Sprint 2.5 P2 regression: every plaintext (RTP/AVP) offer must
    /// advertise PT 101 telephone-event + the RFC 4733 fmtp param
    /// range. Pre-P2 the non-SRTP code path emitted `m=audio … RTP/AVP
    /// 0 8` with no DTMF rtpmap, which silently broke DTMF negotiation
    /// for any plaintext call. The unified `generate_local_sdp` emits
    /// the full PCMU + PCMA + 101 set on every offer.
    #[test]
    fn offer_advertises_telephone_event_on_plaintext() {
        let sdp = build_offer("d", 0, "127.0.0.1", 16000);
        assert!(
            sdp.contains("m=audio 16000 RTP/AVP 0 8 101\r\n"),
            "plaintext offer must advertise PT 101 alongside PCMU + PCMA:\n{}",
            sdp
        );
        assert!(
            sdp.contains("a=rtpmap:101 telephone-event/8000\r\n"),
            "plaintext offer must carry the RFC 4733 telephone-event rtpmap:\n{}",
            sdp
        );
        assert!(
            sdp.contains("a=fmtp:101 0-15\r\n"),
            "plaintext offer must carry the RFC 4733 fmtp 0-15 range:\n{}",
            sdp
        );
    }

    /// Build an SRTP-flavoured offer (RFC 4568 §3.1.4: m= profile is
    /// `RTP/SAVP`) directly via the builder so we can assert the
    /// shape without standing up a full MediaAdapter. Mirrors the
    /// unified `generate_local_sdp` shape — PCMU + PCMA + telephone-event
    /// — with crypto lines appended.
    fn build_srtp_offer(ip: &str, port: u16, attrs: Vec<CryptoAttribute>) -> String {
        let mut media_builder = SdpBuilder::new("Session")
            .origin("-", "1", "0", "IN", "IP4", ip)
            .connection("IN", "IP4", ip)
            .time("0", "0")
            .media_audio(port, "RTP/SAVP")
            .formats(&["0", "8", "101"])
            .rtpmap("0", "PCMU/8000")
            .rtpmap("8", "PCMA/8000")
            .rtpmap("101", "telephone-event/8000")
            .fmtp("101", "0-15");
        for attr in attrs {
            media_builder = media_builder.crypto_attribute(attr);
        }
        media_builder
            .attribute("sendrecv", None::<String>)
            .done()
            .build()
            .expect("srtp offer builds")
            .to_string()
    }

    #[test]
    fn srtp_offer_uses_savp_profile_and_carries_crypto_lines() {
        // RFC 4568 §3.1.4 — m= line MUST be RTP/SAVP when offering SDES.
        use crate::adapters::srtp_negotiator::SrtpNegotiator;
        let suites = vec![
            CryptoSuite::AesCm128HmacSha1_80,
            CryptoSuite::AesCm128HmacSha1_32,
        ];
        let (_, attrs) = SrtpNegotiator::new_offerer(&suites).unwrap();
        let sdp = build_srtp_offer("127.0.0.1", 16000, attrs);

        // Wire-level checks.
        assert!(
            sdp.contains("m=audio 16000 RTP/SAVP 0 8 101\r\n"),
            "SRTP offer should use RTP/SAVP profile per RFC 4568 §3.1.4 \
             with the unified PCMU+PCMA+telephone-event format set:\n{}",
            sdp
        );
        assert!(
            sdp.contains("a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:"),
            "SRTP offer should carry tag-1 _80 crypto line:\n{}",
            sdp
        );
        assert!(
            sdp.contains("a=crypto:2 AES_CM_128_HMAC_SHA1_32 inline:"),
            "SRTP offer should carry tag-2 _32 crypto line:\n{}",
            sdp
        );

        // Round-trip: parse back via the typed parser, assert the
        // crypto attributes survive both directions.
        let parsed = SdpSession::from_str(&sdp).expect("parses");
        let m = &parsed.media_descriptions[0];
        assert_eq!(m.protocol, "RTP/SAVP");
        let crypto_count = m
            .generic_attributes
            .iter()
            .filter(|a| matches!(a, ParsedAttribute::Crypto(_)))
            .count();
        assert_eq!(crypto_count, 2);
    }

    #[test]
    fn extract_audio_crypto_finds_both_offered_lines() {
        use crate::adapters::srtp_negotiator::SrtpNegotiator;
        let (_, attrs) = SrtpNegotiator::new_offerer(&[
            CryptoSuite::AesCm128HmacSha1_80,
            CryptoSuite::AesCm128HmacSha1_32,
        ])
        .unwrap();
        let sdp = build_srtp_offer("127.0.0.1", 16000, attrs);
        let parsed = SdpSession::from_str(&sdp).expect("parses");
        let extracted = MediaAdapter::extract_audio_crypto(&parsed);
        assert_eq!(extracted.len(), 2);
        assert_eq!(extracted[0].tag, 1);
        assert_eq!(extracted[1].tag, 2);
    }

    #[test]
    fn extract_audio_crypto_ignores_unknown_crypto_suite_and_keeps_supported_lines() {
        let sdp = concat!(
            "v=0\r\n",
            "o=- 1 0 IN IP4 127.0.0.1\r\n",
            "s=Session\r\n",
            "c=IN IP4 127.0.0.1\r\n",
            "t=0 0\r\n",
            "m=audio 16000 RTP/SAVP 0 8 101\r\n",
            "a=rtpmap:0 PCMU/8000\r\n",
            "a=crypto:99 AEAD_AES_128_GCM inline:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==\r\n",
            "a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\r\n",
            "a=sendrecv\r\n",
        );
        let parsed = SdpSession::from_str(sdp).expect("unknown crypto suite should not fail SDP");
        let extracted = MediaAdapter::extract_audio_crypto(&parsed);
        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].tag, 1);
        assert_eq!(extracted[0].suite, CryptoSuite::AesCm128HmacSha1_80);
    }

    #[test]
    fn extract_audio_crypto_parses_asterisk_default_aes256_name() {
        let sdp = concat!(
            "v=0\r\n",
            "o=- 1 0 IN IP4 127.0.0.1\r\n",
            "s=Session\r\n",
            "c=IN IP4 127.0.0.1\r\n",
            "t=0 0\r\n",
            "m=audio 16000 RTP/SAVP 0 8 101\r\n",
            "a=rtpmap:0 PCMU/8000\r\n",
            "a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\r\n",
            "a=crypto:2 AES_256_CM_HMAC_SHA1_80 inline:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==\r\n",
            "a=sendrecv\r\n",
        );
        let parsed = SdpSession::from_str(sdp).expect("Asterisk AES-256 crypto name parses");
        let extracted = MediaAdapter::extract_audio_crypto(&parsed);
        assert_eq!(extracted.len(), 2);
        assert_eq!(extracted[0].suite, CryptoSuite::AesCm128HmacSha1_80);
        assert_eq!(extracted[1].suite, CryptoSuite::AesCm256HmacSha1_80);
    }

    /// Sprint 3 A6 — when a public RTP address is configured (static
    /// or STUN-discovered), the offer's c=/o=/m= lines must advertise
    /// it instead of the local interface IP/port. Mirrors what the
    /// generate_local_sdp body does — `local_ip_str` resolves to
    /// `public.ip()` when set, and `port` to `public.port()` when
    /// non-zero, else falls back to the per-session local port.
    #[test]
    fn public_rtp_addr_override_replaces_local_ip_and_port_in_offer() {
        let public: SocketAddr = "203.0.113.42:30000".parse().unwrap();
        let local_fallback: std::net::IpAddr = "192.168.1.10".parse().unwrap();
        let local_port_fallback: u16 = 16000;

        // Replicate the override branch the way generate_local_sdp does it.
        let public_opt = Some(public);
        let advertised_ip = public_opt.map(|sa| sa.ip()).unwrap_or(local_fallback);
        let port = public_opt
            .filter(|sa| sa.port() != 0)
            .map(|sa| sa.port())
            .unwrap_or(local_port_fallback);

        let dialog_id = "dlg";
        let origin_session_id = sdp_origin_session_id(dialog_id);
        let sdp = build_offer(dialog_id, 0, &advertised_ip.to_string(), port);
        assert!(
            sdp.contains("c=IN IP4 203.0.113.42\r\n"),
            "c= must carry public IP when override set:\n{}",
            sdp
        );
        assert!(
            sdp.contains(&format!(
                "o=- {} 0 IN IP4 203.0.113.42\r\n",
                origin_session_id
            )),
            "o= must carry public IP when override set:\n{}",
            sdp
        );
        assert!(
            sdp.contains("m=audio 30000 RTP/AVP"),
            "m=audio must carry public port when override set:\n{}",
            sdp
        );
    }

    #[test]
    fn public_rtp_addr_unset_falls_back_to_local_ip_and_local_port() {
        let public_opt: Option<SocketAddr> = None;
        let local_fallback: std::net::IpAddr = "192.168.1.10".parse().unwrap();
        let local_port_fallback: u16 = 16000;

        let advertised_ip = public_opt.map(|sa| sa.ip()).unwrap_or(local_fallback);
        let port = public_opt
            .filter(|sa| sa.port() != 0)
            .map(|sa| sa.port())
            .unwrap_or(local_port_fallback);

        let sdp = build_offer("dlg", 0, &advertised_ip.to_string(), port);
        assert!(
            sdp.contains("c=IN IP4 192.168.1.10\r\n"),
            "c= falls back to local_ip when no override:\n{}",
            sdp
        );
        assert!(
            sdp.contains("m=audio 16000 RTP/AVP"),
            "m=audio falls back to local_port when no override:\n{}",
            sdp
        );
    }

    /// Sprint 3 C1 — when `comfort_noise_enabled` is set, the SDP
    /// offer's `m=audio` line lists `13` and the body carries an
    /// `a=rtpmap:13 CN/8000` line. The order must be `0 8 13 101` so
    /// telephone-event remains last (Sprint 2.5 P2 fixture stability).
    #[test]
    fn cn_enabled_offer_advertises_pt13_and_rtpmap() {
        let ip = "127.0.0.1";
        let port = 16000;
        let sdp = SdpBuilder::new("Session")
            .origin("-", "dlg", "0", "IN", "IP4", ip)
            .connection("IN", "IP4", ip)
            .time("0", "0")
            .media_audio(port, "RTP/AVP")
            .formats(&["0", "8", "13", "101"])
            .rtpmap("0", "PCMU/8000")
            .rtpmap("8", "PCMA/8000")
            .rtpmap("13", "CN/8000")
            .rtpmap("101", "telephone-event/8000")
            .fmtp("101", "0-15")
            .attribute("sendrecv", None::<String>)
            .done()
            .build()
            .expect("offer builds")
            .to_string();

        assert!(
            sdp.contains("m=audio 16000 RTP/AVP 0 8 13 101\r\n"),
            "format list must include 13 between PCMA and telephone-event:\n{}",
            sdp
        );
        assert!(
            sdp.contains("a=rtpmap:13 CN/8000\r\n"),
            "RFC 3389 CN rtpmap must appear:\n{}",
            sdp
        );
        // Sanity: existing PTs still present in the right shape.
        assert!(sdp.contains("a=rtpmap:0 PCMU/8000\r\n"));
        assert!(sdp.contains("a=rtpmap:8 PCMA/8000\r\n"));
        assert!(sdp.contains("a=rtpmap:101 telephone-event/8000\r\n"));
    }

    /// Sprint 3.5 — strict matching answers with the intersection
    /// only. Offer = `0 101` (no PCMA); answer must carry `0 101`,
    /// not the legacy full `0 8 101` set.
    #[test]
    fn strict_default_answers_with_intersection_only() {
        let offer_sdp = SdpBuilder::new("Session")
            .origin("-", "1", "0", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16000, "RTP/AVP")
            .formats(&["0", "101"])
            .rtpmap("0", "PCMU/8000")
            .rtpmap("101", "telephone-event/8000")
            .fmtp("101", "0-15")
            .attribute("sendrecv", None::<String>)
            .done()
            .build()
            .expect("offer builds")
            .to_string();
        let offer = SdpSession::from_str(&offer_sdp).expect("offer parses");

        let formats = compute_answer_formats(
            &offer,
            &[0, 8, 101], /*strict*/
            true,         /*offer_srtp*/
            false,        /*srtp_required*/
            false,
        )
        .expect("strict-mode match succeeds");
        assert_eq!(
            formats,
            vec!["0".to_string(), "101".to_string()],
            "strict answer must drop PCMA when the offer didn't list it"
        );
    }

    /// Compatibility mode still obeys RFC 3264 and cannot add an unoffered
    /// payload to the answer.
    #[test]
    fn compatibility_mode_still_answers_with_the_offered_intersection() {
        let offer_sdp = SdpBuilder::new("Session")
            .origin("-", "1", "0", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16000, "RTP/AVP")
            .formats(&["0", "101"])
            .rtpmap("0", "PCMU/8000")
            .rtpmap("101", "telephone-event/8000")
            .fmtp("101", "0-15")
            .attribute("sendrecv", None::<String>)
            .done()
            .build()
            .expect("offer builds")
            .to_string();
        let offer = SdpSession::from_str(&offer_sdp).expect("offer parses");

        let formats = compute_answer_formats(
            &offer,
            &[0, 8, 101], /*strict*/
            false,        /*offer_srtp*/
            false,        /*srtp_required*/
            false,
        )
        .expect("compatibility mode accepts the offered overlap");
        assert_eq!(
            formats,
            vec!["0".to_string(), "101".to_string()],
            "an answer must not add unoffered PCMA"
        );
    }

    #[test]
    fn uas_selects_one_primary_codec_and_keeps_auxiliary_payloads() {
        let offer = SdpBuilder::new("Session")
            .origin("-", "1", "0", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16_000, "RTP/AVP")
            .formats(&["8", "0", "13", "101"])
            .rtpmap("8", "PCMA/8000")
            .rtpmap("0", "PCMU/8000")
            .rtpmap("13", "CN/8000")
            .rtpmap("101", "telephone-event/8000")
            .done()
            .build()
            .unwrap();

        let formats = compute_answer_formats(&offer, &[0, 8, 13, 101], true, false, false)
            .expect("valid mixed offer");
        assert_eq!(formats, ["8", "13", "101"]);
    }

    #[test]
    fn uas_skips_invalid_mappings_and_never_answers_unmapped_dynamic_payloads() {
        let offer = SdpBuilder::new("Session")
            .origin("-", "1", "0", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16_000, "RTP/AVP")
            .formats(&["96", "0", "101"])
            .rtpmap("0", "PCMU/16000")
            .rtpmap("101", "telephone-event/8000")
            .done()
            .build()
            .unwrap();

        assert!(compute_answer_formats(&offer, &[0, 8, 111, 101], true, false, false).is_err());
    }

    #[test]
    fn uac_answer_accepts_multiple_offered_primaries_and_validates_dynamic_mappings() {
        let offer = SdpBuilder::new("Session")
            .origin("-", "1", "0", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16_000, "RTP/AVP")
            .formats(&["0", "8", "101"])
            .rtpmap("0", "PCMU/8000")
            .rtpmap("8", "PCMA/8000")
            .rtpmap("101", "telephone-event/8000")
            .done()
            .build()
            .unwrap();
        let multiple_primary = offer.clone();
        let negotiated = validate_uac_audio_answer(&offer, &multiple_primary, true)
            .expect("an answer may retain multiple offered formats");
        assert_eq!(negotiated.0, 0, "answer order selects the preferred codec");

        let missing_dynamic_map = SdpBuilder::new("Session")
            .origin("-", "2", "0", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16_002, "RTP/AVP")
            .formats(&["0", "101"])
            .rtpmap("0", "PCMU/8000")
            .done()
            .build()
            .unwrap();
        assert!(validate_uac_audio_answer(&offer, &missing_dynamic_map, true).is_err());
    }

    #[test]
    fn initial_uac_answer_uses_the_exact_retained_wire_offer() {
        let mut session = SessionState::new(
            SessionId("retained-initial-offer".to_string()),
            crate::state_table::Role::UAC,
        );
        session.local_sdp = Some("v=0\r\na=x-later-working-description\r\n".to_string());
        session.initial_invite_offer_sdp = Some("v=0\r\nm=audio 16000 RTP/AVP 0\r\n".to_string());

        assert_eq!(
            exact_initial_uac_offer(&session),
            Some("v=0\r\nm=audio 16000 RTP/AVP 0\r\n")
        );
    }

    #[tokio::test]
    async fn uas_answer_and_runtime_choose_the_same_single_primary_codec() {
        use crate::session_store::SessionStore;
        use crate::state_table::types::Role;
        use rvoip_media_core::relay::controller::MediaSessionController;
        use std::net::Ipv4Addr;

        let mut adapter = MediaAdapter::new(
            Arc::new(MediaSessionController::new()),
            Arc::new(SessionStore::new()),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            16_000,
            16_100,
        );
        adapter.set_media_mode(MediaMode::SignalingOnly { sdp_rtp_port: 9 });
        adapter.set_offered_codecs(vec![0, 8, 101]);
        let mut session = SessionState::new(SessionId("single-primary".into()), Role::UAS);
        let offer = SdpBuilder::new("Session")
            .origin("-", "1", "0", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(18_000, "RTP/AVP")
            .formats(&["8", "0", "101"])
            .rtpmap("8", "PCMA/8000")
            .rtpmap("0", "PCMU/8000")
            .rtpmap("101", "telephone-event/8000")
            .done()
            .build()
            .unwrap()
            .to_string();

        let (answer, config) = adapter
            .negotiate_sdp_as_uas_lane_owned(&mut session, &offer)
            .await
            .unwrap();
        assert!(answer.contains("m=audio 9 RTP/AVP 8 101\r\n"));
        assert!(!answer.contains("a=rtpmap:0 "));
        assert_eq!(config.codec, "PCMA");
        assert_eq!(config.payload_type, 8);
    }

    #[tokio::test]
    async fn invalid_uas_codec_offer_does_not_commit_provisional_srtp_state() {
        use crate::session_store::SessionStore;
        use crate::state_table::types::Role;
        use rvoip_media_core::relay::controller::MediaSessionController;
        use std::net::Ipv4Addr;

        let mut adapter = MediaAdapter::new(
            Arc::new(MediaSessionController::new()),
            Arc::new(SessionStore::new()),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            16_000,
            16_100,
        );
        adapter.set_media_mode(MediaMode::SignalingOnly { sdp_rtp_port: 9 });
        adapter.set_srtp_policy(true, true, vec![CryptoSuite::AesCm128HmacSha1_80]);
        let (_, attrs) =
            SrtpNegotiator::new_offerer(&[CryptoSuite::AesCm128HmacSha1_80]).expect("offerer keys");
        let mut media = SdpBuilder::new("Session")
            .origin("-", "1", "0", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(18_000, "RTP/SAVP")
            .formats(&["0"])
            .rtpmap("0", "PCMU/16000");
        for attr in attrs {
            media = media.crypto_attribute(attr);
        }
        let offer = media.done().build().unwrap().to_string();
        let session_id = SessionId("srtp-codec-rollback".into());
        let mut session = SessionState::new(session_id.clone(), Role::UAS);

        assert!(adapter
            .negotiate_sdp_as_uas_lane_owned(&mut session, &offer)
            .await
            .is_err());
        assert!(session.media_security.is_none());
        assert!(adapter.negotiated_srtp.is_empty());
    }

    /// Sprint 3.5 — strict mode + zero overlap returns
    /// `SDPNegotiationFailed`. The state machine turns this into
    /// `488 Not Acceptable Here` (the same path `srtp_required`
    /// already uses on a plain offer).
    #[test]
    fn strict_default_no_overlap_returns_negotiation_failed() {
        let offer_sdp = SdpBuilder::new("Session")
            .origin("-", "1", "0", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16000, "RTP/AVP")
            .formats(&["97", "98"])
            .rtpmap("97", "VP8/90000")
            .rtpmap("98", "VP9/90000")
            .attribute("sendrecv", None::<String>)
            .done()
            .build()
            .expect("offer builds")
            .to_string();
        let offer = SdpSession::from_str(&offer_sdp).expect("offer parses");

        let err = compute_answer_formats(
            &offer,
            &[0, 8, 101], /*strict*/
            true,         /*offer_srtp*/
            false,        /*srtp_required*/
            false,
        )
        .unwrap_err();
        assert!(
            matches!(err, SessionError::SDPNegotiationFailed(_)),
            "no overlap must surface as SDPNegotiationFailed → 488 NAH; got {:?}",
            err
        );
    }

    /// Sprint 3.5 — strict matching preserves CN advertisement
    /// when both peers offer it. Offer `0 13 101`, our caps include
    /// `13` (comfort_noise_enabled=true), answer carries `0 13 101`.
    #[test]
    fn strict_with_cn_enabled_keeps_cn_in_intersection() {
        let offer_sdp = SdpBuilder::new("Session")
            .origin("-", "1", "0", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16000, "RTP/AVP")
            .formats(&["0", "13", "101"])
            .rtpmap("0", "PCMU/8000")
            .rtpmap("13", "CN/8000")
            .rtpmap("101", "telephone-event/8000")
            .fmtp("101", "0-15")
            .attribute("sendrecv", None::<String>)
            .done()
            .build()
            .expect("offer builds")
            .to_string();
        let offer = SdpSession::from_str(&offer_sdp).expect("offer parses");

        let formats = compute_answer_formats(
            &offer,
            &[0, 8, 13, 101], /*strict*/
            true,             /*offer_srtp*/
            false,            /*srtp_required*/
            false,
        )
        .expect("CN-on-both-sides match succeeds");
        assert_eq!(
            formats,
            vec!["0".to_string(), "13".to_string(), "101".to_string()],
            "intersection must include 13 when both sides advertise it"
        );
    }

    #[test]
    fn cn_disabled_offer_omits_pt13_and_rtpmap() {
        // The pre-Sprint-3 baseline shape — no `13`, no CN rtpmap.
        let sdp = build_offer("dlg", 0, "127.0.0.1", 16000);
        assert!(
            sdp.contains("m=audio 16000 RTP/AVP 0 8 101\r\n"),
            "default offer must keep the pre-Sprint-3 format set:\n{}",
            sdp
        );
        assert!(
            !sdp.contains("CN/8000"),
            "default offer must not advertise CN: \n{}",
            sdp
        );
    }

    #[cfg(feature = "opus")]
    #[tokio::test]
    async fn opus_offered_codec_appears_in_generated_offer() {
        // NEXT_STEPS C2 — when Opus (PT 111) is in the configured
        // offered_codecs, the generated SDP offer MUST carry
        // `a=rtpmap:111 opus/48000/2` and list `111` in the m-line
        // format set. The legacy PCMU/PCMA/DTMF still go out alongside
        // when included in the list.
        use crate::session_store::SessionStore;
        use crate::state_table::types::Role;
        use rvoip_media_core::relay::controller::MediaSessionController;
        use std::net::Ipv4Addr;

        let controller = Arc::new(MediaSessionController::new());
        let store = Arc::new(SessionStore::new());
        let session_id = SessionId("opus-offer-test".to_string());
        store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create session");

        let mut adapter = MediaAdapter::new(
            controller,
            store.clone(),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            16000,
            16100,
        );
        adapter.set_offered_codecs(vec![0, 8, 111, 101]);
        adapter
            .start_session(&session_id)
            .await
            .expect("start session");

        let sdp = adapter
            .generate_local_sdp_offer(&session_id, crate::types::MediaDirection::SendRecv)
            .await
            .expect("offer builds");

        assert!(
            sdp.contains("a=rtpmap:111 opus/48000/2"),
            "Opus rtpmap must appear in the offer when 111 is in offered_codecs:\n{}",
            sdp
        );
        assert!(
            sdp.contains("a=rtpmap:0 PCMU/8000"),
            "legacy PCMU rtpmap must still appear:\n{}",
            sdp
        );
        assert!(
            sdp.contains(" 111 ") || sdp.contains(" 111\r\n"),
            "m-line format list must include PT 111:\n{}",
            sdp
        );
    }

    #[cfg(feature = "g729")]
    #[tokio::test]
    async fn g729_offer_advertises_annex_b_yes() {
        use crate::session_store::SessionStore;
        use crate::state_table::types::Role;
        use rvoip_media_core::relay::controller::MediaSessionController;
        use std::net::Ipv4Addr;

        let controller = Arc::new(MediaSessionController::new());
        let store = Arc::new(SessionStore::new());
        let session_id = SessionId("g729-annexb-yes-offer-test".to_string());
        store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create session");

        let mut adapter = MediaAdapter::new(
            controller,
            store.clone(),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            16000,
            16100,
        );
        adapter.set_offered_codecs(vec![18, 101]);
        adapter.set_g729_annex_b(true);
        adapter
            .start_session(&session_id)
            .await
            .expect("start session");

        let sdp = adapter
            .generate_local_sdp_offer(&session_id, crate::types::MediaDirection::SendRecv)
            .await
            .expect("offer builds");

        assert!(
            sdp.contains(" RTP/AVP 18 101\r\n"),
            "m-line must advertise PT18:\n{}",
            sdp
        );
        assert!(
            sdp.contains("a=rtpmap:18 G729/8000"),
            "G.729 rtpmap must appear:\n{}",
            sdp
        );
        assert!(
            sdp.contains("a=fmtp:18 annexb=yes"),
            "Annex B enabled offer must advertise annexb=yes:\n{}",
            sdp
        );
    }

    #[cfg(feature = "g729")]
    #[tokio::test]
    async fn g729_offer_advertises_annex_b_no() {
        use crate::session_store::SessionStore;
        use crate::state_table::types::Role;
        use rvoip_media_core::relay::controller::MediaSessionController;
        use std::net::Ipv4Addr;

        let controller = Arc::new(MediaSessionController::new());
        let store = Arc::new(SessionStore::new());
        let session_id = SessionId("g729-annexb-no-offer-test".to_string());
        store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create session");

        let mut adapter = MediaAdapter::new(
            controller,
            store.clone(),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            16000,
            16100,
        );
        adapter.set_offered_codecs(vec![18, 101]);
        adapter.set_g729_annex_b(false);
        adapter
            .start_session(&session_id)
            .await
            .expect("start session");

        let sdp = adapter
            .generate_local_sdp_offer(&session_id, crate::types::MediaDirection::SendRecv)
            .await
            .expect("offer builds");

        assert!(
            sdp.contains("a=fmtp:18 annexb=no"),
            "Annex B disabled offer must advertise annexb=no:\n{}",
            sdp
        );
    }

    #[cfg(feature = "g729")]
    #[tokio::test]
    async fn g729_uas_negotiation_updates_media_core_codec() {
        use crate::session_store::SessionStore;
        use crate::state_table::types::Role;
        use rvoip_media_core::relay::controller::MediaSessionController;
        use std::net::Ipv4Addr;

        let controller = Arc::new(MediaSessionController::new());
        let store = Arc::new(SessionStore::new());
        let session_id = SessionId("g729-negotiated-media-config-test".to_string());
        store
            .create_session(session_id.clone(), Role::UAS, false)
            .await
            .expect("create session");

        let mut adapter = MediaAdapter::new(
            controller.clone(),
            store.clone(),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            16000,
            16100,
        );
        adapter.set_offered_codecs(vec![18, 101]);
        adapter.set_g729_annex_b(false);
        adapter
            .start_session(&session_id)
            .await
            .expect("start session");

        let offer_sdp = SdpBuilder::new("Session")
            .origin("-", "1", "0", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(35000, "RTP/AVP")
            .formats(&["18", "101"])
            .rtpmap("18", "G729/8000")
            .fmtp("18", "annexb=no")
            .rtpmap("101", "telephone-event/8000")
            .fmtp("101", "0-15")
            .attribute("sendrecv", None::<String>)
            .done()
            .build()
            .expect("offer builds")
            .to_string();

        let (answer_sdp, config) = adapter
            .negotiate_sdp_as_uas(&session_id, &offer_sdp)
            .await
            .expect("G.729 offer negotiates");

        assert!(
            answer_sdp.contains("a=fmtp:18 annexb=no"),
            "G.729A answer must keep annexb=no:\n{}",
            answer_sdp
        );
        assert_eq!(config.codec, "G729A");
        assert_eq!(config.payload_type, 18);

        let dialog_id = adapter
            .current_media(&session_id)
            .expect("exact media resource exists")
            .dialog_id;
        let info = controller
            .get_session_info(&dialog_id)
            .await
            .expect("media session exists");
        assert_eq!(info.config.preferred_codec, Some("G729A".to_string()));
        assert_eq!(
            info.config.remote_addr,
            Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 35000))
        );

        adapter
            .cleanup_session(&session_id)
            .await
            .expect("cleanup media session");
    }

    /// A peer's AMR-WB offer negotiates into a session that actually codes.
    ///
    /// The locally-testable half of interop. A live FreeSWITCH or Asterisk
    /// call exercises the same path plus the wire; this exercises everything
    /// up to it — SDP in, negotiated payload type, clock rate and fmtp out,
    /// then a codec built from exactly those and a frame put through it.
    ///
    /// Four things have to survive together and each was separately broken on
    /// this branch: the codec name, the dynamic payload type, the 16 kHz clock
    /// rate (the shape check refused it), and the `octet-align` that decides
    /// the payload's bit layout.
    #[tokio::test]
    #[cfg(feature = "amr-wb")]
    async fn an_amr_wideband_offer_negotiates_into_a_working_codec() {
        use crate::session_store::SessionStore;
        use crate::state_table::types::Role;
        use rvoip_media_core::relay::controller::{
            MediaSessionController, NEGOTIATED_FMTP_PARAMETER,
        };
        use std::net::Ipv4Addr;

        let controller = Arc::new(MediaSessionController::new());
        let store = Arc::new(SessionStore::new());
        let session_id = SessionId("amr-wb-offer".to_string());
        store
            .create_session(session_id.clone(), Role::UAS, false)
            .await
            .expect("create session");

        let mut adapter = MediaAdapter::new(
            controller.clone(),
            store.clone(),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            16400,
            16500,
        );
        adapter.set_offered_codecs(vec![AMR_WB_OA_PT, 101]);
        adapter
            .start_session(&session_id)
            .await
            .expect("start session");

        let pt = AMR_WB_OA_PT.to_string();
        let offer = SdpBuilder::new("Session")
            .origin("-", "1", "0", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(35200, "RTP/AVP")
            .formats(&[pt.as_str()])
            .rtpmap(pt.as_str(), "AMR-WB/16000")
            .fmtp(pt.as_str(), "octet-align=1; mode-set=0,2,4")
            .attribute("sendrecv", None::<String>)
            .done()
            .build()
            .expect("offer builds")
            .to_string();

        adapter
            .negotiate_sdp_as_uas(&session_id, &offer)
            .await
            .expect("an AMR-WB offer negotiates");

        let dialog_id = adapter
            .current_media(&session_id)
            .expect("media resource exists")
            .dialog_id;
        let config = controller
            .get_session_info(&dialog_id)
            .await
            .expect("media session exists")
            .config;

        assert_eq!(
            config.preferred_codec.as_deref(),
            Some("AMR-WB"),
            "the negotiated codec name did not reach media-core"
        );
        assert_eq!(
            config.parameters.get(NEGOTIATED_FMTP_PARAMETER).map(String::as_str),
            Some("octet-align=1; mode-set=0,2,4"),
            "the framing did not reach media-core"
        );

        // And the thing that actually matters: a codec built from exactly this
        // negotiation codes a frame. 320 samples, because AMR-WB is 16 kHz --
        // a path that assumed 8 kHz would refuse this and pass a narrowband
        // test.
        use rvoip_media_core::codec::spec::AudioCodecSpec;
        let spec = AudioCodecSpec::new("AMR-WB", AMR_WB_OA_PT, 16_000, 1)
            .with_fmtp(config.parameters.get(NEGOTIATED_FMTP_PARAMETER).map(String::as_str));
        let mut codec = spec.build().expect("the negotiated codec builds");
        let pcm: Vec<i16> = (0..320)
            .map(|i| ((f64::from(i) * 0.05).sin() * 6000.0) as i16)
            .collect();
        let frame = rvoip_media_core::types::AudioFrame::new(pcm, 16_000, 1, 0);
        let payload = codec.encode(&frame).expect("encodes");
        assert!(!payload.is_empty());
        let decoded = codec.decode(&payload).expect("decodes");
        assert_eq!(decoded.samples.len(), 320);
        assert!(decoded.samples.iter().any(|&s| s != 0), "round trip was silent");

        adapter
            .cleanup_session(&session_id)
            .await
            .expect("cleanup media session");
    }

    /// The negotiated `a=fmtp` string must survive from the peer's SDP into
    /// the media layer's own configuration.
    ///
    /// It did not, for the whole life of the parameter: `NegotiatedConfig`
    /// carried it, `apply_negotiated_media_config` did not take it, and
    /// `MediaConfig::with_negotiated_fmtp` had zero callers -- so the bridge's
    /// AMR framing guard read `None`, compared `""` against `""`, and could
    /// never fire. Every test that looked like coverage stopped one call short
    /// of the boundary.
    ///
    /// G.729 rather than AMR, still: the parameter is codec-agnostic and what
    /// is under test here is carriage rather than interpretation. The AMR
    /// case is `an_amr_wideband_offer_negotiates_into_a_working_codec` above,
    /// which is a different claim -- that the value is not merely carried but
    /// acted on.
    #[cfg(feature = "g729")]
    #[tokio::test]
    async fn negotiated_fmtp_reaches_the_media_layer() {
        use crate::session_store::SessionStore;
        use crate::state_table::types::Role;
        use rvoip_media_core::relay::controller::{
            MediaSessionController, NEGOTIATED_FMTP_PARAMETER,
        };
        use std::net::Ipv4Addr;

        let controller = Arc::new(MediaSessionController::new());
        let store = Arc::new(SessionStore::new());
        let session_id = SessionId("fmtp-reaches-media-layer".to_string());
        store
            .create_session(session_id.clone(), Role::UAS, false)
            .await
            .expect("create session");

        let mut adapter = MediaAdapter::new(
            controller.clone(),
            store.clone(),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            16200,
            16300,
        );
        adapter.set_offered_codecs(vec![18, 101]);
        adapter.set_g729_annex_b(false);
        adapter
            .start_session(&session_id)
            .await
            .expect("start session");

        let offer = |fmtp: Option<&str>| {
            let mut media = SdpBuilder::new("Session")
                .origin("-", "1", "0", "IN", "IP4", "127.0.0.1")
                .connection("IN", "IP4", "127.0.0.1")
                .time("0", "0")
                .media_audio(35100, "RTP/AVP")
                .formats(&["18"])
                .rtpmap("18", "G729/8000");
            if let Some(fmtp) = fmtp {
                media = media.fmtp("18", fmtp);
            }
            media
                .attribute("sendrecv", None::<String>)
                .done()
                .build()
                .expect("offer builds")
                .to_string()
        };

        adapter
            .negotiate_sdp_as_uas(&session_id, &offer(Some("annexb=no")))
            .await
            .expect("G.729 offer negotiates");

        let dialog_id = adapter
            .current_media(&session_id)
            .expect("exact media resource exists")
            .dialog_id;
        let carried = controller
            .get_session_info(&dialog_id)
            .await
            .expect("media session exists")
            .config
            .parameters
            .get(NEGOTIATED_FMTP_PARAMETER)
            .cloned();
        assert_eq!(
            carried.as_deref(),
            Some("annexb=no"),
            "the peer's fmtp did not reach media-core"
        );

        // And a renegotiation carrying no fmtp must CLEAR it rather than leave
        // the previous generation's string behind. This is the half an
        // insert-only builder gets wrong: the next configuration is seeded
        // from this one, so a stale `octet-align=1` would outlive the
        // negotiation that agreed it and make the bridge refuse a compatible
        // pair.
        adapter
            .negotiate_sdp_as_uas(&session_id, &offer(None))
            .await
            .expect("fmtp-less offer negotiates");
        let after = controller
            .get_session_info(&dialog_id)
            .await
            .expect("media session exists")
            .config
            .parameters
            .get(NEGOTIATED_FMTP_PARAMETER)
            .cloned();
        assert_eq!(
            after, None,
            "a negotiation without fmtp left the previous value in place"
        );

        adapter
            .cleanup_session(&session_id)
            .await
            .expect("cleanup media session");
    }

    #[tokio::test]
    async fn default_offered_codecs_omit_opus() {
        // Regression guard: the C2 default (PCMU + PCMA + DTMF) must
        // not silently start advertising Opus. Adding Opus support
        // when media-core has no `opus` feature would negotiate a
        // codec the encoder can't produce.
        use crate::session_store::SessionStore;
        use crate::state_table::types::Role;
        use rvoip_media_core::relay::controller::MediaSessionController;
        use std::net::Ipv4Addr;

        let controller = Arc::new(MediaSessionController::new());
        let store = Arc::new(SessionStore::new());
        let session_id = SessionId("default-codecs-test".to_string());
        store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create session");

        let adapter = MediaAdapter::new(
            controller,
            store.clone(),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            16000,
            16100,
        );
        adapter
            .start_session(&session_id)
            .await
            .expect("start session");

        let sdp = adapter
            .generate_local_sdp_offer(&session_id, crate::types::MediaDirection::SendRecv)
            .await
            .expect("offer builds");

        assert!(
            !sdp.contains("opus"),
            "default offer must not advertise Opus:\n{}",
            sdp
        );
    }

    #[test]
    fn configured_formats_filter_unavailable_codecs() {
        use crate::session_store::SessionStore;
        use rvoip_media_core::relay::controller::MediaSessionController;
        use std::net::Ipv4Addr;

        let mut adapter = MediaAdapter::new(
            Arc::new(MediaSessionController::new()),
            Arc::new(SessionStore::new()),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            16_000,
            16_100,
        );
        adapter.set_offered_codecs(vec![9, 111, 0, 8, 101]);
        let formats = adapter.effective_offered_formats();
        assert!(!formats.contains(&9), "G.722 must never be advertised");
        #[cfg(feature = "opus")]
        assert!(formats.contains(&111));
        #[cfg(not(feature = "opus"))]
        assert!(!formats.contains(&111));
        assert!(formats.contains(&0));
        assert!(formats.contains(&8));
    }

    #[test]
    fn auxiliary_only_audio_formats_do_not_fall_back_to_pcmu() {
        let formats = vec!["13".to_string(), "101".to_string()];
        assert_eq!(select_primary_audio_payload(&formats), None);
    }

    /// A minimal audio offer with one dynamic payload type and its rtpmap.
    fn dynamic_audio_offer(pt: &str, rtpmap: &str) -> SdpSession {
        SdpBuilder::new("Session")
            .origin("-", "1", "1", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16_000, "RTP/AVP")
            .formats(&[pt])
            .rtpmap(pt, rtpmap)
            .done()
            .build()
            .unwrap()
    }

    #[test]
    #[cfg(feature = "amr-wb")]
    fn dynamic_payload_types_are_resolved_from_the_rtpmap_not_assumed_to_be_opus() {
        // The whole dynamic range used to be hardcoded to Opus, so an AMR-WB
        // offer on any PT above 95 was rejected. Which codec a dynamic PT
        // carries is decided by the encoding name the peer declared.
        let offer = dynamic_audio_offer("100", "AMR-WB/16000");
        assert!(sdp_payload_codec_available(&offer, 100));

        let (codec, clock_rate, channels) =
            negotiated_audio_shape_from_sdp(&offer, 100, false).unwrap();
        assert_eq!(codec, "AMR-WB");
        assert_eq!(clock_rate, 16_000);
        assert_eq!(channels, 1);
    }

    #[test]
    #[cfg(feature = "amr-nb")]
    fn amr_narrowband_is_resolved_on_a_dynamic_payload_type() {
        let offer = dynamic_audio_offer("98", "AMR/8000");
        assert!(sdp_payload_codec_available(&offer, 98));
        let (codec, clock_rate, _) = negotiated_audio_shape_from_sdp(&offer, 98, false).unwrap();
        assert_eq!(codec, "AMR");
        assert_eq!(clock_rate, 8_000);
    }

    #[test]
    #[cfg(all(feature = "amr-wb", feature = "opus"))]
    fn amr_and_opus_coexist_in_the_dynamic_range() {
        // Both must be resolvable on arbitrary dynamic payload types, which is
        // the point of dispatching on the encoding name.
        let amr = dynamic_audio_offer("111", "AMR-WB/16000");
        assert_eq!(
            negotiated_audio_shape_from_sdp(&amr, 111, false).unwrap().0,
            "AMR-WB"
        );
        let opus = dynamic_audio_offer("104", "opus/48000/2");
        assert_eq!(
            negotiated_audio_shape_from_sdp(&opus, 104, false).unwrap().0,
            "opus"
        );
    }

    #[test]
    #[cfg(feature = "amr-wb")]
    fn amr_wideband_with_the_wrong_clock_rate_is_rejected() {
        // AMR-WB is 16 kHz. An offer claiming 8 kHz is malformed, and its
        // clock rate is what distinguishes it from AMR when both are present.
        let offer = dynamic_audio_offer("100", "AMR-WB/8000");
        assert!(negotiated_audio_shape_from_sdp(&offer, 100, false).is_err());
        assert!(!sdp_payload_codec_available(&offer, 100));
    }

    #[test]
    fn unknown_dynamic_encodings_are_still_rejected() {
        // Widening the dynamic range must not turn it into a wildcard.
        let offer = dynamic_audio_offer("100", "SPEEX/16000");
        assert!(!sdp_payload_codec_available(&offer, 100));
        assert!(negotiated_audio_shape_from_sdp(&offer, 100, false).is_err());
    }

    #[test]
    #[cfg(feature = "amr-wb")]
    fn amr_fmtp_is_read_out_of_the_offer_verbatim() {
        // Scope: this asserts the SDP parser hands back the parameter string
        // unchanged. It says nothing about whether the value survives into
        // media-core -- for a long time it did not, and this test's previous
        // name claimed otherwise while the body never crossed the boundary.
        // `negotiated_fmtp_reaches_the_media_layer` below is the one that
        // crosses it.
        let offer = SdpBuilder::new("Session")
            .origin("-", "1", "1", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16_000, "RTP/AVP")
            .formats(&["100"])
            .rtpmap("100", "AMR-WB/16000")
            .fmtp("100", "octet-align=1; mode-set=0,1,2")
            .done()
            .build()
            .unwrap();

        let params = audio_fmtp_params(&offer, 100).expect("fmtp must be carried");
        assert!(params.contains("octet-align=1"), "{params}");
        assert!(params.contains("mode-set=0,1,2"), "{params}");
    }

    #[test]
    fn a_payload_type_without_fmtp_reports_none_rather_than_empty() {
        // Absent is meaningful for AMR — it selects every RFC 4867 default —
        // so it must be distinguishable from an empty parameter string.
        let offer = dynamic_audio_offer("100", "AMR-WB/16000");
        assert_eq!(audio_fmtp_params(&offer, 100), None);
        // And a payload type that is not present at all.
        assert_eq!(audio_fmtp_params(&offer, 99), None);
    }

    #[test]
    #[cfg(feature = "amr-wb")]
    fn amr_matches_a_peers_own_dynamic_payload_type() {
        // Asterisk and most handsets pick their own number for AMR. Opus
        // already has this remap; AMR did not, so a peer offering AMR-WB on
        // 96 got a 488 even though we support exactly that codec. Our own
        // call test passed only because both ends used our private constants.
        let offer = SdpBuilder::new("Session")
            .origin("-", "1", "1", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16_000, "RTP/AVP")
            .formats(&["96", "101"])
            .rtpmap("96", "AMR-WB/16000")
            .fmtp("96", "octet-align=1")
            .rtpmap("101", "telephone-event/8000")
            .done()
            .build()
            .unwrap();

        let answer = compute_answer_formats(&offer, &[AMR_WB_OA_PT, 101], true, false, false)
            .expect("a peer's own dynamic payload type for AMR must negotiate");
        assert_eq!(
            answer.first().map(String::as_str),
            Some("96"),
            "the answer must use the peer's number, not ours: {answer:?}"
        );
    }

    /// What we advertise and what we transmit must be the same framing.
    ///
    /// The answer's fmtp came from a fixed per-PT table while the codec was
    /// configured from the *offer's* fmtp. A peer offering `octet-align=1` on
    /// PT 104 — our bandwidth-efficient number — was accepted, answered with
    /// no fmtp at all, and then sent octet-aligned frames. Unparseable audio
    /// with no error anywhere, which is the exact failure the bridge's framing
    /// guard exists to catch one layer up.
    #[test]
    fn the_answers_framing_matches_what_the_codec_is_given() {
        for (offer_pt, offered_fmtp) in [
            (AMR_WB_BE_PT, Some("octet-align=1")),
            (AMR_WB_OA_PT, Some("octet-align=1")),
            (AMR_WB_BE_PT, None),
            (AMR_NB_BE_PT, Some("octet-align=1")),
        ] {
            let pt = offer_pt.to_string();
            let mut media = SdpBuilder::new("Session")
                .origin("-", "1", "1", "IN", "IP4", "127.0.0.1")
                .connection("IN", "IP4", "127.0.0.1")
                .time("0", "0")
                .media_audio(16_000, "RTP/AVP")
                .formats(&[pt.as_str()])
                .rtpmap(
                    pt.as_str(),
                    if offer_pt >= AMR_NB_BE_PT { "AMR/8000" } else { "AMR-WB/16000" },
                );
            if let Some(fmtp) = offered_fmtp {
                media = media.fmtp(pt.as_str(), fmtp);
            }
            let offer = media.done().build().unwrap();

            // What the codec will be configured with.
            let to_codec = audio_fmtp_params(&offer, offer_pt).unwrap_or_default();
            // What we would put in the answer.
            let advertised = answer_fmtp_for_pt(&offer, offer_pt, false).unwrap_or_default();

            let transmits_octet_aligned = to_codec.contains("octet-align=1");
            let advertises_octet_aligned = advertised.contains("octet-align=1");
            assert_eq!(
                transmits_octet_aligned, advertises_octet_aligned,
                "PT {offer_pt} offered {offered_fmtp:?}: we would transmit \
                 octet-aligned={transmits_octet_aligned} while advertising \
                 octet-aligned={advertises_octet_aligned} ({advertised:?})"
            );
        }
    }

    #[test]
    fn amr_offers_advertise_each_framing_as_its_own_payload_type() {
        // RFC 4867 §8.3.1: transport configurations are mutually incompatible
        // bit patterns, so they are separate payload types rather than
        // negotiated down. Bandwidth-efficient is the default and needs no
        // fmtp; octet-aligned must say so.
        assert_eq!(rtpmap_for_pt(AMR_WB_BE_PT), Some("AMR-WB/16000"));
        assert_eq!(rtpmap_for_pt(AMR_WB_OA_PT), Some("AMR-WB/16000"));
        assert_eq!(fmtp_for_pt_with_g729_annex_b(AMR_WB_BE_PT, false), None);
        assert_eq!(
            fmtp_for_pt_with_g729_annex_b(AMR_WB_OA_PT, false),
            Some("octet-align=1")
        );
        assert!(payload_codec_available(AMR_WB_BE_PT));
        assert!(payload_codec_available(AMR_WB_OA_PT));
    }

    #[test]
    fn uac_answer_rejects_unoffered_unsupported_and_changed_payloads() {
        let offer = SdpBuilder::new("Session")
            .origin("-", "1", "1", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16_000, "RTP/AVP")
            .formats(&["0", "111"])
            .rtpmap("0", "PCMU/8000")
            .rtpmap("111", "opus/48000/2")
            .done()
            .build()
            .unwrap();
        let unoffered = SdpBuilder::new("Session")
            .origin("-", "2", "2", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16_002, "RTP/AVP")
            .formats(&["96"])
            .rtpmap("96", "opus/48000/2")
            .done()
            .build()
            .unwrap();
        assert!(validate_uac_audio_answer(&offer, &unoffered, true).is_err());

        let changed = SdpBuilder::new("Session")
            .origin("-", "3", "3", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16_002, "RTP/AVP")
            .formats(&["111"])
            .rtpmap("111", "PCMU/8000")
            .done()
            .build()
            .unwrap();
        assert!(validate_uac_audio_answer(&offer, &changed, true).is_err());

        let unsupported_offer = SdpBuilder::new("Session")
            .origin("-", "4", "4", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16_000, "RTP/AVP")
            .formats(&["9"])
            .rtpmap("9", "G722/8000")
            .done()
            .build()
            .unwrap();
        let unsupported_answer = SdpBuilder::new("Session")
            .origin("-", "5", "5", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16_002, "RTP/AVP")
            .formats(&["9"])
            .rtpmap("9", "G722/8000")
            .done()
            .build()
            .unwrap();
        assert!(validate_uac_audio_answer(&unsupported_offer, &unsupported_answer, true).is_err());
    }

    #[cfg(feature = "opus")]
    #[test]
    fn dynamic_opus_payload_is_matched_and_preserved() {
        let offer = SdpBuilder::new("Session")
            .origin("-", "1", "1", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16_000, "RTP/AVP")
            .formats(&["96"])
            .rtpmap("96", "OpUs/48000/1")
            .done()
            .build()
            .unwrap();
        let formats = compute_answer_formats(&offer, &[0, 8, 111], true, false, false).unwrap();
        assert_eq!(formats, vec!["96"]);
        let (codec, clock_rate, channels) =
            negotiated_audio_shape_from_sdp(&offer, 96, false).unwrap();
        assert_eq!(codec, "opus");
        assert_eq!(clock_rate, 48_000);
        assert_eq!(channels, 1);
    }

    #[test]
    fn lane_owned_sdp_origin_increments_version_across_calls() {
        // RFC 3264 §8 — every fresh local SDP body on a session must
        // carry a strictly greater `o=` version. The pure mutation runs on
        // the executor's working state; the canonical lane commit publishes
        // it with the rest of the transition.
        use crate::state_table::types::Role;

        let session_id = SessionId("test-version-bump".to_string());
        let mut session = SessionState::new(session_id, Role::UAC);
        let (sess1, v_initial) = advance_sdp_origin(&mut session);
        let (sess2, v_hold) = advance_sdp_origin(&mut session);
        let (sess3, v_resume) = advance_sdp_origin(&mut session);

        // o= session-id must stay stable across re-INVITEs on the same
        // dialog (RFC 3264 §6 + §8) — only the version advances.
        assert_eq!(
            sess1, sess2,
            "session-id must not change between offer and hold re-INVITE"
        );
        assert_eq!(
            sess1, sess3,
            "session-id must not change between offer and resume re-INVITE"
        );

        // Strict version monotonicity, RFC 3264 §8.
        assert!(
            v_hold > v_initial,
            "hold re-INVITE version ({}) must exceed initial offer ({})",
            v_hold,
            v_initial
        );
        assert!(
            v_resume > v_hold,
            "resume re-INVITE version ({}) must exceed hold ({})",
            v_resume,
            v_hold
        );
    }

    #[test]
    fn port_zero_rejection_emits_m_audio_zero_with_offered_proto() {
        // RFC 3264 §6 / RFC 4568 §7.3: when we decline an offered
        // m-line (here: peer offered SRTP, our policy is plaintext),
        // the answer's m-line port must be 0 and the proto must echo
        // the offer's proto so the peer distinguishes a policy
        // rejection from a parse error.
        let sdp = build_port_zero_rejection_sdp("1234", 7, "192.0.2.10", "RTP/SAVP")
            .expect("rejection SDP builds");

        assert!(
            sdp.contains("m=audio 0 RTP/SAVP"),
            "rejection answer must carry port=0 and the offered proto:\n{}",
            sdp
        );
        assert!(
            sdp.contains("o=- 1234 7 IN IP4 192.0.2.10"),
            "rejection answer must carry the supplied origin line:\n{}",
            sdp
        );
        assert!(
            sdp.contains("c=IN IP4 192.0.2.10"),
            "rejection answer must include a c= line:\n{}",
            sdp
        );
        assert!(
            !sdp.contains("a=crypto"),
            "rejection answer must not advertise any crypto attributes:\n{}",
            sdp
        );
    }

    #[test]
    fn port_zero_rejection_echoes_avp_proto_when_offered() {
        // If peer ever offers plaintext but we still want to decline
        // (e.g. require_srtp branch wired through this helper later),
        // the proto echoed must be `RTP/AVP`, not the SAVP default.
        let sdp = build_port_zero_rejection_sdp("1", 0, "10.0.0.1", "RTP/AVP")
            .expect("rejection SDP builds");

        assert!(
            sdp.contains("m=audio 0 RTP/AVP"),
            "rejection must echo the offered proto verbatim:\n{}",
            sdp
        );
    }

    #[test]
    fn public_rtp_addr_with_zero_port_keeps_local_port() {
        // The override semantics: when `media_public_addr` carries an
        // IP-only mapping (port 0), advertise the public IP but keep
        // the per-session local port. Useful for SBC-fronted setups
        // where the port doesn't change but the IP does.
        let public: SocketAddr = "203.0.113.42:0".parse().unwrap();
        let public_opt = Some(public);
        let local_fallback: std::net::IpAddr = "192.168.1.10".parse().unwrap();
        let local_port_fallback: u16 = 16000;

        let advertised_ip = public_opt.map(|sa| sa.ip()).unwrap_or(local_fallback);
        let port = public_opt
            .filter(|sa| sa.port() != 0)
            .map(|sa| sa.port())
            .unwrap_or(local_port_fallback);

        assert_eq!(advertised_ip.to_string(), "203.0.113.42");
        assert_eq!(port, 16000, "zero port must defer to local_port_fallback");
    }

    #[tokio::test]
    async fn uas_plain_avp_answer_uses_allocated_rtp_port() {
        use crate::session_store::SessionStore;
        use crate::state_table::types::Role;
        use rvoip_media_core::relay::controller::MediaSessionController;
        use std::net::Ipv4Addr;

        let controller = Arc::new(MediaSessionController::new());
        let store = Arc::new(SessionStore::new());
        let session_id = SessionId("uas-plain-avp-answer-test".to_string());
        store
            .create_session(session_id.clone(), Role::UAS, false)
            .await
            .expect("create session");

        let adapter = MediaAdapter::new(
            controller,
            store,
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            16000,
            16100,
        );
        adapter
            .start_session(&session_id)
            .await
            .expect("start media session");

        let offer_sdp = SdpBuilder::new("Session")
            .origin("-", "1", "0", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(35000, "RTP/AVP")
            .formats(&["0"])
            .rtpmap("0", "PCMU/8000")
            .attribute("sendrecv", None::<String>)
            .done()
            .build()
            .expect("offer builds")
            .to_string();

        let (answer_sdp, config) = adapter
            .negotiate_sdp_as_uas(&session_id, &offer_sdp)
            .await
            .expect("plain RTP offer negotiates");
        let parsed = SdpSession::from_str(&answer_sdp).expect("answer parses");
        let media = parsed
            .media_descriptions
            .iter()
            .find(|media| media.media == "audio")
            .expect("audio media line");

        assert_eq!(media.protocol, "RTP/AVP");
        assert_eq!(media.formats, vec!["0".to_string()]);
        assert_eq!(media.port, config.local_addr.port());
        assert_ne!(
            media.port, 5060,
            "SDP answer must advertise the allocated RTP port, not the SIP listener port"
        );

        adapter
            .cleanup_session(&session_id)
            .await
            .expect("cleanup media session");
    }

    #[tokio::test]
    async fn teardown_during_media_create_rolls_back_exact_allocated_dialog() {
        use crate::session_store::SessionStore;
        use crate::state_table::types::Role;
        use rvoip_media_core::relay::controller::MediaSessionController;
        use std::net::Ipv4Addr;

        let controller = Arc::new(MediaSessionController::new());
        let store = Arc::new(SessionStore::new());
        let session_id = SessionId("teardown-during-media-create".to_string());
        store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create exact session");
        let handle = store
            .lifecycle_handle(&session_id)
            .expect("exact lifecycle handle");
        let expected_dialog = DialogId::new(format!(
            "media-{}-{}",
            session_id.0,
            handle.key().resource_generation_suffix()
        ));
        let adapter = MediaAdapter::new(
            Arc::clone(&controller),
            Arc::clone(&store),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            16_000,
            16_100,
        );
        adapter
            .pause_media_create_after_allocation
            .store(true, Ordering::Release);

        let create_adapter = adapter.clone();
        let create_session_id = session_id.clone();
        let create =
            tokio::spawn(async move { create_adapter.create_session(&create_session_id).await });
        tokio::time::timeout(
            Duration::from_secs(3),
            adapter.media_create_allocated.notified(),
        )
        .await
        .expect("media allocation pause was reached");
        assert!(controller
            .get_session_info(&expected_dialog)
            .await
            .is_some());
        assert!(
            adapter.media_resources.is_empty(),
            "exact resource binding must remain unpublished before owned commit"
        );
        assert!(
            store.registry().get_media_handle_exact(&handle).is_none(),
            "canonical registry association must remain unpublished before owned commit"
        );
        assert!(
            controller.get_media_id(&session_id.0).is_none(),
            "media-core compatibility mapping must remain unpublished"
        );

        let remove_store = Arc::clone(&store);
        let remove_session_id = session_id.clone();
        let remove =
            tokio::spawn(async move { remove_store.remove_session(&remove_session_id).await });
        let create_result = tokio::time::timeout(Duration::from_secs(3), create)
            .await
            .expect("retained media creator should observe teardown")
            .expect("media creator task");
        assert!(create_result.is_err());
        tokio::time::timeout(Duration::from_secs(3), remove)
            .await
            .expect("exact teardown should await and finish rollback")
            .expect("remove task")
            .expect("remove exact session");

        assert!(controller
            .get_session_info(&expected_dialog)
            .await
            .is_none());
        assert!(adapter.media_create_reservations.is_empty());
        assert!(adapter.media_resources.is_empty());
        assert!(store.registry().get_media_retained_exact(&handle).is_none());
        assert_eq!(
            adapter.cleanup_attempt_total.load(Ordering::Relaxed),
            1,
            "allocation rollback must stop the exact dialog once"
        );
    }

    #[tokio::test]
    async fn stale_exact_audio_authority_cannot_target_reused_media_session() {
        use crate::session_store::SessionStore;
        use crate::state_table::types::Role;
        use rvoip_media_core::relay::controller::MediaSessionController;
        use std::net::Ipv4Addr;

        let controller = Arc::new(MediaSessionController::new());
        let store = Arc::new(SessionStore::new());
        let session_id = SessionId("exact-audio-reuse".to_string());
        store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create media generation A");
        let generation_a = store
            .lifecycle_handle(&session_id)
            .expect("generation A exact handle");
        let adapter = MediaAdapter::new(
            controller,
            Arc::clone(&store),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            16_200,
            16_300,
        );
        adapter
            .start_session(&session_id)
            .await
            .expect("allocate generation A media");

        store
            .remove_session_exact(&generation_a)
            .await
            .expect("retire generation A and release exact media");
        assert!(
            store.authority().elapse_reuse_horizon_for_test(&session_id),
            "expire media anti-reuse horizon"
        );
        store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create media generation B");
        let generation_b = store
            .lifecycle_handle(&session_id)
            .expect("generation B exact handle");
        assert_ne!(generation_a, generation_b);
        adapter
            .start_session(&session_id)
            .await
            .expect("allocate generation B media");

        let stale_frame = AudioFrame::new(vec![0; 160], 8_000, 1, 0);
        assert!(adapter
            .send_audio_frame_exact(&generation_a, stale_frame)
            .await
            .is_err());
        assert!(adapter
            .subscribe_to_audio_frames_exact(&generation_a)
            .await
            .is_err());
        assert!(adapter
            .set_audio_source_exact(&generation_a, AudioSource::PassThrough)
            .await
            .is_err());
        assert!(
            adapter.media_for_handle_exact(&generation_b).is_some(),
            "stale generation-A audio work must leave generation B media intact"
        );

        store
            .remove_session_exact(&generation_b)
            .await
            .expect("cleanup generation B");
    }

    #[tokio::test]
    async fn stale_media_negotiation_cleanup_cannot_cross_session_generation_reuse() {
        use crate::session_store::SessionStore;
        use crate::state_table::types::Role;
        use rvoip_media_core::relay::controller::MediaSessionController;
        use std::net::Ipv4Addr;

        let store = Arc::new(SessionStore::new());
        let session_id = SessionId("exact-negotiation-reuse".to_string());
        store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create generation A");
        let generation_a = store
            .lifecycle_handle(&session_id)
            .expect("generation A handle");
        store
            .remove_session_exact(&generation_a)
            .await
            .expect("retire generation A");
        assert!(store.authority().elapse_reuse_horizon_for_test(&session_id));
        store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create generation B");
        let generation_b = store
            .lifecycle_handle(&session_id)
            .expect("generation B handle");

        let adapter = MediaAdapter::new(
            Arc::new(MediaSessionController::new()),
            store,
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            16_400,
            16_500,
        );
        let staged = StagedMediaNegotiation {
            config: NegotiatedConfig {
                local_addr: "127.0.0.1:16400".parse().unwrap(),
                remote_addr: "127.0.0.1:16402".parse().unwrap(),
                codec: "PCMU".to_string(),
                payload_type: 0,
                clock_rate: 8_000,
                channels: 1,
                negotiated_fmtp: None,
                local_direction: crate::types::MediaDirection::SendRecv,
                remote_direction: crate::types::MediaDirection::SendRecv,
            },
            stable_local_direction: crate::types::MediaDirection::SendRecv,
            srtp_negotiated: false,
        };
        adapter.staged_media_negotiations.insert(
            MediaNegotiationKey::Exact(generation_a.clone()),
            staged.clone(),
        );
        adapter
            .staged_media_negotiations
            .insert(MediaNegotiationKey::Exact(generation_b.clone()), staged);

        adapter.discard_staged_media_negotiation_exact(&generation_a);

        assert!(!adapter
            .staged_media_negotiations
            .contains_key(&MediaNegotiationKey::Exact(generation_a)));
        assert!(adapter
            .staged_media_negotiations
            .contains_key(&MediaNegotiationKey::Exact(generation_b)));
    }

    #[tokio::test]
    async fn stale_prepared_media_finalization_cannot_cross_session_generation_reuse() {
        use crate::session_store::SessionStore;
        use crate::state_table::types::Role;
        use rvoip_media_core::relay::controller::MediaSessionController;
        use std::net::Ipv4Addr;

        let store = Arc::new(SessionStore::new());
        let session_id = SessionId("exact-prepared-negotiation-reuse".to_string());
        store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create generation A");
        let generation_a = store
            .lifecycle_handle(&session_id)
            .expect("generation A handle");
        store
            .remove_session_exact(&generation_a)
            .await
            .expect("retire generation A");
        assert!(store.authority().elapse_reuse_horizon_for_test(&session_id));
        store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create generation B");
        let generation_b = store
            .lifecycle_handle(&session_id)
            .expect("generation B handle");

        let mut adapter = MediaAdapter::new(
            Arc::new(MediaSessionController::new()),
            store,
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            16_400,
            16_500,
        );
        adapter.set_media_mode(MediaMode::SignalingOnly { sdp_rtp_port: 9 });
        let staged = StagedMediaNegotiation {
            config: NegotiatedConfig {
                local_addr: "127.0.0.1:16400".parse().unwrap(),
                remote_addr: "127.0.0.1:16402".parse().unwrap(),
                codec: "PCMU".to_string(),
                payload_type: 0,
                clock_rate: 8_000,
                channels: 1,
                negotiated_fmtp: None,
                local_direction: crate::types::MediaDirection::SendRecv,
                remote_direction: crate::types::MediaDirection::SendRecv,
            },
            stable_local_direction: crate::types::MediaDirection::SendRecv,
            srtp_negotiated: false,
        };
        adapter.staged_media_negotiations.insert(
            MediaNegotiationKey::Exact(generation_a.clone()),
            staged.clone(),
        );
        adapter
            .staged_media_negotiations
            .insert(MediaNegotiationKey::Exact(generation_b.clone()), staged);
        let mut working = SessionState::new(session_id, Role::UAC);
        working.lifecycle_handle = Some(generation_a.clone());
        let prepared = adapter
            .prepare_staged_media_negotiation_lane_owned(&mut working)
            .await
            .expect("prepare generation A");

        working.lifecycle_handle = Some(generation_b.clone());
        adapter
            .finalize_prepared_media_negotiation_lane_owned(&mut working, prepared)
            .await
            .expect_err("stale prepared authority must fail closed");

        assert!(!adapter
            .staged_media_negotiations
            .contains_key(&MediaNegotiationKey::Exact(generation_a)));
        assert!(adapter
            .staged_media_negotiations
            .contains_key(&MediaNegotiationKey::Exact(generation_b)));
    }

    #[tokio::test]
    async fn failed_commit_after_srtp_swap_restores_stable_media() {
        use crate::session_store::SessionStore;
        use crate::state_table::types::Role;
        use rvoip_media_core::relay::controller::MediaSessionController;
        use rvoip_rtp_core::srtp::{SrtpContext, SrtpCryptoKey, SRTP_AES128_CM_SHA1_80};
        use std::net::Ipv4Addr;

        let controller = Arc::new(MediaSessionController::new());
        let store = Arc::new(SessionStore::new());
        let session_id = SessionId("post-srtp-swap-rollback".to_string());
        store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create exact session");
        let adapter = MediaAdapter::new(
            Arc::clone(&controller),
            Arc::clone(&store),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            16_400,
            16_500,
        );
        let dialog_id = adapter
            .create_session(&session_id)
            .await
            .expect("create managed media");
        let handle = store
            .lifecycle_handle(&session_id)
            .expect("exact lifecycle handle");
        let mut working = store
            .get_session_exact(&handle)
            .await
            .expect("load exact session");
        working.media_session_id = Some(dialog_id.clone());
        working.media_session_ready = true;
        working.local_media_direction = crate::types::MediaDirection::SendRecv;

        let before = controller
            .get_session_info(&dialog_id)
            .await
            .expect("stable lower media");
        let make_context = |key_byte, salt_byte| {
            SrtpContext::new(
                SRTP_AES128_CM_SHA1_80,
                SrtpCryptoKey::new(vec![key_byte; 16], vec![salt_byte; 14]),
            )
            .expect("test SRTP context")
        };
        let key = MediaNegotiationKey::Exact(handle.clone());
        adapter.negotiated_srtp.insert(
            key.clone(),
            SrtpPair {
                send_ctx: make_context(0x11, 0x22),
                recv_ctx: make_context(0x33, 0x44),
                suite: CryptoSuite::AesCm128HmacSha1_80,
            },
        );
        adapter.staged_media_negotiations.insert(
            key,
            StagedMediaNegotiation {
                config: NegotiatedConfig {
                    local_addr: before.config.local_addr,
                    remote_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 35_000),
                    codec: "PCMA".to_string(),
                    payload_type: 8,
                    clock_rate: 8_000,
                    channels: 1,
                    negotiated_fmtp: None,
                    local_direction: crate::types::MediaDirection::Inactive,
                    remote_direction: crate::types::MediaDirection::SendRecv,
                },
                stable_local_direction: crate::types::MediaDirection::SendRecv,
                srtp_negotiated: true,
            },
        );
        adapter
            .fail_media_commit_after_srtp_swap
            .store(true, Ordering::Release);

        let error = adapter
            .commit_staged_media_negotiation_lane_owned(&mut working)
            .await
            .expect_err("post-swap failure must abort the media commit");
        assert!(matches!(
            &error,
            SessionError::MediaError(detail)
                if detail == "injected failure after SRTP context replacement"
        ));
        assert_eq!(working.media_security, None);

        let after = controller
            .get_session_info(&dialog_id)
            .await
            .expect("restored lower media");
        assert_eq!(after.config.local_addr, before.config.local_addr);
        assert_eq!(after.config.remote_addr, before.config.remote_addr);
        assert_eq!(after.config.preferred_codec, before.config.preferred_codec);
        assert_eq!(after.config.parameters, before.config.parameters);

        adapter
            .cleanup_session(&session_id)
            .await
            .expect("cleanup managed media");
    }

    #[tokio::test]
    async fn failed_public_uas_commit_does_not_publish_advanced_sdp_origin() {
        use crate::session_store::SessionStore;
        use crate::state_table::types::Role;
        use rvoip_media_core::relay::controller::MediaSessionController;
        use std::net::Ipv4Addr;

        let controller = Arc::new(MediaSessionController::new());
        let store = Arc::new(SessionStore::new());
        let session_id = SessionId("failed-public-uas-origin-rollback".to_string());
        store
            .create_session(session_id.clone(), Role::UAS, false)
            .await
            .expect("create exact UAS session");
        let mut adapter = MediaAdapter::new(
            controller,
            Arc::clone(&store),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            17_200,
            17_300,
        );
        adapter.set_srtp_policy(true, true, vec![CryptoSuite::AesCm128HmacSha1_80]);
        adapter
            .start_session(&session_id)
            .await
            .expect("start exact media");

        let handle = store
            .lifecycle_handle(&session_id)
            .expect("capture exact UAS lifetime");
        let before = store
            .get_session_exact(&handle)
            .await
            .expect("load stable UAS state");
        let (_, offered_crypto) = SrtpNegotiator::new_offerer(&[CryptoSuite::AesCm128HmacSha1_80])
            .expect("build SRTP offer");
        let mut offer = SdpBuilder::new("Session")
            .origin("-", "1", "0", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(35_010, "RTP/SAVP")
            .formats(&["0"])
            .rtpmap("0", "PCMU/8000");
        for crypto in offered_crypto {
            offer = offer.crypto_attribute(crypto);
        }
        let offer = offer
            .attribute("sendrecv", None::<String>)
            .done()
            .build()
            .expect("build SRTP SDP offer")
            .to_string();

        adapter
            .fail_media_commit_after_srtp_swap
            .store(true, Ordering::Release);
        adapter
            .negotiate_sdp_as_uas(&session_id, &offer)
            .await
            .expect_err("injected media commit failure must reject the answer");

        let after = store
            .get_session_exact(&handle)
            .await
            .expect("reload stable UAS state");
        assert_eq!(after.sdp_origin_session_id, before.sdp_origin_session_id);
        assert_eq!(after.sdp_origin_version, before.sdp_origin_version);
        assert_eq!(after.media_security, before.media_security);
        assert!(adapter.staged_media_negotiations.is_empty());
        assert!(adapter.negotiated_srtp.is_empty());

        adapter
            .cleanup_session(&session_id)
            .await
            .expect("cleanup managed media");
    }

    #[tokio::test]
    async fn failed_prepared_media_rollback_quarantines_exact_allocation() {
        use crate::api::events::{MediaSecurityKeying, MediaSecurityProfile, MediaSecurityState};
        use crate::session_store::SessionStore;
        use crate::state_table::types::Role;
        use rvoip_media_core::relay::controller::MediaSessionController;
        use rvoip_rtp_core::srtp::{SrtpContext, SrtpCryptoKey, SRTP_AES128_CM_SHA1_80};
        use std::net::Ipv4Addr;

        let controller = Arc::new(MediaSessionController::new());
        let store = Arc::new(SessionStore::new());
        let session_id = SessionId("failed-media-rollback-quarantine".to_string());
        store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create exact session");
        let adapter = MediaAdapter::new(
            Arc::clone(&controller),
            Arc::clone(&store),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            16_600,
            16_700,
        );
        let dialog_id = adapter
            .create_session(&session_id)
            .await
            .expect("create managed media");
        let handle = store
            .lifecycle_handle(&session_id)
            .expect("exact lifecycle handle");
        let mut working = store
            .get_session_exact(&handle)
            .await
            .expect("load exact session");
        working.media_session_id = Some(dialog_id.clone());
        working.media_session_ready = true;
        working.local_media_direction = crate::types::MediaDirection::SendRecv;
        working.media_security = Some(MediaSecurityState {
            keying: MediaSecurityKeying::Sdes,
            suite: CryptoSuite::AesCm128HmacSha1_80,
            profile: MediaSecurityProfile::RtpSavp,
            contexts_installed: true,
        });

        let make_context = |key_byte, salt_byte| {
            SrtpContext::new(
                SRTP_AES128_CM_SHA1_80,
                SrtpCryptoKey::new(vec![key_byte; 16], vec![salt_byte; 14]),
            )
            .expect("test SRTP context")
        };
        controller
            .install_srtp_contexts(
                &dialog_id,
                make_context(0x01, 0x02),
                make_context(0x03, 0x04),
            )
            .await
            .expect("install stable SRTP contexts");
        let before = controller
            .get_session_info(&dialog_id)
            .await
            .expect("stable lower media");
        let key = MediaNegotiationKey::Exact(handle);
        adapter.negotiated_srtp.insert(
            key.clone(),
            SrtpPair {
                send_ctx: make_context(0x11, 0x12),
                recv_ctx: make_context(0x13, 0x14),
                suite: CryptoSuite::AesCm128HmacSha1_80,
            },
        );
        adapter.staged_media_negotiations.insert(
            key,
            StagedMediaNegotiation {
                config: NegotiatedConfig {
                    local_addr: before.config.local_addr,
                    remote_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 35_200),
                    codec: "PCMA".to_string(),
                    payload_type: 8,
                    clock_rate: 8_000,
                    channels: 1,
                    negotiated_fmtp: None,
                    local_direction: crate::types::MediaDirection::Inactive,
                    remote_direction: crate::types::MediaDirection::SendRecv,
                },
                stable_local_direction: crate::types::MediaDirection::SendRecv,
                srtp_negotiated: true,
            },
        );

        let prepared = adapter
            .prepare_staged_media_negotiation_lane_owned(&mut working)
            .await
            .expect("reversibly apply new media");
        adapter.fail_media_rollback.store(true, Ordering::Release);
        let error = adapter
            .rollback_prepared_media_negotiation_lane_owned(&mut working, prepared)
            .await
            .expect_err("rollback failure must quarantine media");
        assert!(matches!(
            error,
            SessionError::MediaError(detail)
                if detail.contains("media was quarantined") && detail.contains("injected")
        ));
        assert_eq!(working.media_session_id, None);
        assert!(!working.media_session_ready);
        assert_eq!(working.media_security, None);
        assert!(
            controller.get_session_info(&dialog_id).await.is_none(),
            "the failed rollback must release the exact lower allocation"
        );
    }

    #[tokio::test]
    async fn lane_owned_cleanup_leaves_store_unpublished_for_executor_commit() {
        use crate::session_store::SessionStore;
        use crate::state_table::types::Role;
        use rvoip_media_core::relay::controller::MediaSessionController;
        use std::net::Ipv4Addr;

        let controller = Arc::new(MediaSessionController::new());
        let store = Arc::new(SessionStore::new());
        let session_id = SessionId("lane-owned-media-cleanup".to_string());
        store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create exact session");
        let handle = store
            .lifecycle_handle(&session_id)
            .expect("exact lifecycle handle");
        let adapter = MediaAdapter::new(
            Arc::clone(&controller),
            Arc::clone(&store),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            16_000,
            16_100,
        );
        let media_id = adapter
            .create_session(&session_id)
            .await
            .expect("create managed media");
        let mut initial = store
            .get_session_exact(&handle)
            .await
            .expect("load initial exact state");
        initial.media_session_id = Some(media_id.clone());
        initial.media_session_ready = true;
        initial.sdp_negotiated = true;
        initial.local_sdp = Some("v=0\r\n".to_string());
        store
            .update_state_machine_session_and_snapshot(initial)
            .expect("publish initial media identity");

        let before = store
            .get_session_snapshot_exact(&handle)
            .expect("pre-cleanup snapshot");
        let mut working = before.state().clone();
        adapter
            .cleanup_session_lane_owned(&working)
            .await
            .expect("clean lower media in exact lane");

        let during = store
            .get_session_snapshot_exact(&handle)
            .expect("snapshot during lane-owned cleanup");
        assert_eq!(
            during.revision(),
            before.revision(),
            "lower cleanup must not publish an intermediate session revision"
        );
        assert_eq!(
            during.media_session_id.as_ref(),
            Some(&media_id),
            "the store changes only when the executor commits its working state"
        );
        assert!(controller.get_session_info(&media_id).await.is_none());
        assert!(adapter.media_resources.is_empty());
        assert!(adapter.media_sessions.is_empty());
        assert!(store.registry().get_media_handle_exact(&handle).is_none());

        working.media_session_id = None;
        working.media_session_ready = false;
        working.sdp_negotiated = false;
        working.local_sdp = None;
        working.negotiated_config = None;
        let committed = store
            .update_state_machine_session_and_snapshot(working)
            .expect("executor canonical media cleanup commit");
        assert_eq!(committed.revision(), before.revision() + 1);
        assert!(committed.media_session_id.is_none());
        assert!(!committed.media_session_ready);

        store
            .remove_session(&session_id)
            .await
            .expect("retire exact session");
        assert_eq!(
            adapter.cleanup_attempt_total.load(Ordering::Relaxed),
            1,
            "quiesced teardown must reuse the completed lower release"
        );
    }

    #[tokio::test]
    async fn repeated_exact_media_cleanup_releases_managed_resource_once() {
        use crate::session_store::SessionStore;
        use crate::state_table::types::Role;
        use rvoip_media_core::relay::controller::MediaSessionController;
        use std::net::Ipv4Addr;

        let controller = Arc::new(MediaSessionController::new());
        let store = Arc::new(SessionStore::new());
        let session_id = SessionId("managed-media-double-cleanup".to_string());
        store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create exact session");
        let adapter = MediaAdapter::new(
            Arc::clone(&controller),
            Arc::clone(&store),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            16_000,
            16_100,
        );
        let media_id = adapter
            .create_session(&session_id)
            .await
            .expect("create managed media");
        let mut state = store.get_session(&session_id).await.expect("session state");
        state.media_session_id = Some(media_id.clone());
        state.media_session_ready = true;
        store
            .update_session(state)
            .await
            .expect("publish media identity to exact session state");

        adapter
            .cleanup_session(&session_id)
            .await
            .expect("first exact cleanup");
        adapter
            .cleanup_session(&session_id)
            .await
            .expect("second exact cleanup is a no-op");

        assert_eq!(
            adapter.cleanup_attempt_total.load(Ordering::Relaxed),
            1,
            "explicit cleanup and its retry must share one release cell"
        );
        assert!(controller.get_session_info(&media_id).await.is_none());
        assert!(adapter.media_create_reservations.is_empty());
        assert!(adapter.media_resources.is_empty());
        assert!(adapter.media_sessions.is_empty());
        assert!(store
            .registry()
            .get_media_by_session(&session_id)
            .await
            .is_none());
        assert_eq!(
            store
                .get_session(&session_id)
                .await
                .expect("retained state")
                .media_session_id,
            None,
            "managed release must clear the exact retained media identity"
        );

        store
            .remove_session(&session_id)
            .await
            .expect("retire exact session");
        assert_eq!(
            adapter.cleanup_attempt_total.load(Ordering::Relaxed),
            1,
            "authority teardown must reuse the completed release cell"
        );
    }
}
