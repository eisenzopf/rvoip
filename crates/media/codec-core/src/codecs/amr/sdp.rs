//! RFC 4867 §8.1 SDP format parameters, and the §8.3.1 offer/answer rules.
//!
//! This module owns the `a=fmtp` line for `audio/AMR` and `audio/AMR-WB`. It
//! lives beside the payload format because they are the same specification —
//! the parameters here decide how [`super::payload`] frames the bits.
//!
//! # The negotiation rules are not what you would guess
//!
//! Three of them routinely surprise implementers, and all three cause silent
//! interop failures rather than clean errors:
//!
//! 1. **`mode-set` is not intersected.** RFC 4867 §8.3.1: "If a mode set was
//!    supplied in the offer, the answerer SHALL return the mode-set unmodified
//!    or reject the payload type." An answerer that narrows the set is
//!    non-compliant. Only when the offer omits `mode-set` may the answerer
//!    choose one — and that choice then binds both parties.
//! 2. **`mode-set` is bi-directional.** The negotiated set applies to media
//!    both sent *and* received by every party. It is not a per-direction
//!    declaration.
//! 3. **The transport parameters must be echoed verbatim.** "An SDP answerer
//!    MUST include, in the SDP answer for a payload type, the following
//!    parameters unmodified from the SDP offer (unless it removes the payload
//!    type): octet-align; crc; robust-sorting; interleaving; and channels."
//!    Each combination is a distinct bit pattern; changing one produces a
//!    stream the offerer cannot parse.
//!
//! Because each transport combination is incompatible with every other, an
//! endpoint that supports several should offer them as **separate payload
//! types** rather than trying to negotiate one down. Same for mode sets: "it is
//! RECOMMENDED to include each mode-set it can support as a separate payload
//! type within the offer."

use super::mode::{AmrModeSet, AmrVariant};
use super::payload::AmrPayloadConfig;
use crate::error::{CodecError, Result};
use std::fmt::Write as _;

/// The RFC 4867 §8.1 format parameters for one payload type.
///
/// Defaults match the RFC's defaults, so `AmrFmtp::default_for(variant)`
/// describes a payload type advertised with a bare `a=rtpmap` and no `a=fmtp`.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmrFmtp {
    /// Which codec family. Not itself an fmtp parameter — it comes from the
    /// `a=rtpmap` encoding name — but every other field is interpreted
    /// relative to it.
    pub variant: AmrVariant,
    /// `octet-align`. Default 0, meaning bandwidth-efficient.
    pub octet_align: bool,
    /// `mode-set`. `None` means the parameter is absent, i.e. all modes are
    /// allowed. An explicit set restricts both directions.
    pub mode_set: Option<AmrModeSet>,
    /// `mode-change-period`: 1 or 2. What this endpoint requires of streams it
    /// *receives*. Default 1 (changes allowed at any time).
    pub mode_change_period: u8,
    /// `mode-change-capability`: 1 or 2. What this endpoint can do when
    /// *transmitting*. Default 1 (cannot restrict).
    pub mode_change_capability: u8,
    /// `mode-change-neighbor`. Declarative: setting it says this endpoint wants
    /// to *receive* streams that only step between neighbouring modes.
    pub mode_change_neighbor: bool,
    /// `crc`. Implies `octet_align`.
    pub crc: bool,
    /// `robust-sorting`. Implies `octet_align`.
    pub robust_sorting: bool,
    /// `interleaving`: maximum frame-blocks in an interleaving group. Presence
    /// implies `octet_align`.
    pub interleaving: Option<u8>,
    /// `channels`. Default 1. Only mono is supported here.
    pub channels: u8,
    /// `max-red`: milliseconds between a primary transmission and the last
    /// redundant one. `None` means no limit was declared.
    pub max_red: Option<u16>,
}

impl AmrFmtp {
    /// The parameter set implied by a payload type with no `a=fmtp` line.
    #[must_use]
    pub const fn default_for(variant: AmrVariant) -> Self {
        Self {
            variant,
            octet_align: false,
            mode_set: None,
            mode_change_period: 1,
            mode_change_capability: 1,
            mode_change_neighbor: false,
            crc: false,
            robust_sorting: false,
            interleaving: None,
            channels: 1,
            max_red: None,
        }
    }

    /// Parse an `a=fmtp` parameter string, e.g. `octet-align=1; mode-set=0,2,4`.
    ///
    /// Unknown parameters are ignored rather than rejected: SDP is extensible,
    /// and a peer offering something we do not recognise should not fail the
    /// whole payload type.
    ///
    /// # Errors
    ///
    /// Returns an error for a parameter whose value is malformed or outside the
    /// range the RFC allows — a `mode-set` naming mode 9 for narrowband, an
    /// `octet-align` that is neither 0 nor 1, a `channels` other than 1.
    pub fn parse(variant: AmrVariant, parameters: &str) -> Result<Self> {
        let mut fmtp = Self::default_for(variant);
        // Track explicit octet-align=0 so a contradiction with crc=1 can be
        // reported rather than silently upgraded.
        let mut explicit_octet_align = None;

        for part in parameters.split(';') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let (name, value) = part.split_once('=').unwrap_or((part, ""));
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim().trim_matches('"');

            match name.as_str() {
                "octet-align" => explicit_octet_align = Some(parse_flag(&name, value)?),
                "crc" => fmtp.crc = parse_flag(&name, value)?,
                "robust-sorting" => fmtp.robust_sorting = parse_flag(&name, value)?,
                "mode-change-neighbor" => fmtp.mode_change_neighbor = parse_flag(&name, value)?,
                "mode-set" => fmtp.mode_set = Some(parse_mode_set(variant, value)?),
                "mode-change-period" => {
                    fmtp.mode_change_period = parse_one_or_two(&name, value)?;
                }
                "mode-change-capability" => {
                    fmtp.mode_change_capability = parse_one_or_two(&name, value)?;
                }
                "interleaving" => {
                    fmtp.interleaving = Some(value.parse::<u8>().map_err(|_| {
                        CodecError::invalid_format(format!(
                            "AMR interleaving must be a frame-block count, got {value:?}"
                        ))
                    })?);
                }
                "channels" => {
                    let channels = value.parse::<u8>().map_err(|_| {
                        CodecError::invalid_format(format!(
                            "AMR channels must be 1-6, got {value:?}"
                        ))
                    })?;
                    if channels != 1 {
                        return Err(CodecError::invalid_format(format!(
                            "only single-channel AMR is supported, offer requested {channels}"
                        )));
                    }
                    fmtp.channels = channels;
                }
                "max-red" => {
                    fmtp.max_red = Some(value.parse::<u16>().map_err(|_| {
                        CodecError::invalid_format(format!(
                            "AMR max-red must be 0-65535 ms, got {value:?}"
                        ))
                    })?);
                }
                // Unknown parameter: ignore. ptime and maxptime are separate
                // SDP attributes, not fmtp parameters, and are handled by the
                // SDP layer.
                _ => {}
            }
        }

        // crc, robust-sorting and interleaving each imply octet-aligned
        // operation. An offer that sets one alongside octet-align=0 is
        // self-contradictory; say so rather than picking a winner.
        let implied = fmtp.crc || fmtp.robust_sorting || fmtp.interleaving.is_some();
        if implied && explicit_octet_align == Some(false) {
            return Err(CodecError::invalid_format(
                "AMR crc, robust-sorting and interleaving imply octet-align=1, \
                 but octet-align=0 was given",
            ));
        }
        fmtp.octet_align = explicit_octet_align.unwrap_or(false) || implied;

        Ok(fmtp)
    }

    /// Render as an `a=fmtp` parameter string.
    ///
    /// Only parameters that differ from their RFC defaults are emitted, which
    /// keeps offers small and avoids asserting defaults that a peer might
    /// otherwise treat as deliberate. Returns an empty string when everything
    /// is default — the caller should then omit the `a=fmtp` line entirely.
    #[must_use]
    pub fn to_fmtp_value(&self) -> String {
        let mut out = String::new();
        let mut push = |name: &str, value: &str| {
            if !out.is_empty() {
                out.push_str("; ");
            }
            // Writing to a String is infallible.
            let _ = write!(out, "{name}={value}");
        };

        if self.octet_align {
            push("octet-align", "1");
        }
        if let Some(mode_set) = &self.mode_set {
            push("mode-set", &mode_set.to_sdp_value());
        }
        if self.mode_change_period != 1 {
            push("mode-change-period", &self.mode_change_period.to_string());
        }
        if self.mode_change_capability != 1 {
            push(
                "mode-change-capability",
                &self.mode_change_capability.to_string(),
            );
        }
        if self.mode_change_neighbor {
            push("mode-change-neighbor", "1");
        }
        if self.crc {
            push("crc", "1");
        }
        if self.robust_sorting {
            push("robust-sorting", "1");
        }
        if let Some(interleaving) = self.interleaving {
            push("interleaving", &interleaving.to_string());
        }
        if let Some(max_red) = self.max_red {
            push("max-red", &max_red.to_string());
        }
        out
    }

    /// The payload framing these parameters select.
    #[must_use]
    pub const fn payload_config(&self) -> AmrPayloadConfig {
        AmrPayloadConfig {
            variant: self.variant,
            octet_aligned: self.octet_align,
            crc: self.crc,
            robust_sorting: self.robust_sorting,
            interleaving: self.interleaving.is_some(),
        }
    }

    /// The modes this payload type may use. An absent `mode-set` means all.
    #[must_use]
    pub fn active_modes(&self) -> AmrModeSet {
        self.mode_set
            .clone()
            .unwrap_or_else(|| AmrModeSet::all(self.variant))
    }

    /// Whether two parameter sets describe the same wire format.
    ///
    /// RFC 4867: "Each combination of the RTP payload transport format
    /// configuration parameters (octet-align, crc, robust-sorting,
    /// interleaving, and channels) is unique in its bit-pattern and not
    /// compatible with any other combination."
    #[must_use]
    pub const fn same_transport_as(&self, other: &Self) -> bool {
        self.octet_align == other.octet_align
            && self.crc == other.crc
            && self.robust_sorting == other.robust_sorting
            && self.interleaving.is_some() == other.interleaving.is_some()
            && self.channels == other.channels
    }
}

/// What this endpoint can do, used to answer an offer.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmrCapabilities {
    /// Codec family these capabilities describe.
    pub variant: AmrVariant,
    /// Every mode this endpoint can encode and decode.
    pub supported_modes: AmrModeSet,
    /// Whether octet-aligned framing can be used.
    pub supports_octet_align: bool,
    /// Whether bandwidth-efficient framing can be used.
    pub supports_bandwidth_efficient: bool,
    /// Whether the per-frame CRC can be produced and checked.
    pub supports_crc: bool,
    /// Whether robust sorting can be produced and undone.
    pub supports_robust_sorting: bool,
    /// Whether interleaving can be honoured. Producing the ILL/ILP fields is
    /// not enough — accepting means committing to reorder on receive.
    pub supports_interleaving: bool,
    /// `mode-change-capability` to declare: 1 or 2.
    pub mode_change_capability: u8,
    /// `mode-change-period` to require of the peer: 1 or 2.
    pub mode_change_period: u8,
    /// Whether to ask the peer to restrict itself to neighbouring mode changes.
    pub mode_change_neighbor: bool,
    /// `max-red` to declare, if any.
    pub max_red: Option<u16>,
}

impl AmrCapabilities {
    /// Capabilities matching what this crate currently implements: every mode,
    /// both framings, CRC and robust sorting, but **not** interleaving —
    /// the payload format carries ILL/ILP without reordering frame-blocks, and
    /// accepting the parameter would promise behaviour we do not provide.
    #[must_use]
    pub const fn new(variant: AmrVariant) -> Self {
        Self {
            variant,
            supported_modes: AmrModeSet::all(variant),
            supports_octet_align: true,
            supports_bandwidth_efficient: true,
            supports_crc: true,
            supports_robust_sorting: true,
            supports_interleaving: false,
            mode_change_capability: 1,
            mode_change_period: 1,
            mode_change_neighbor: false,
            max_red: None,
        }
    }

    /// Restrict the modes this endpoint will use.
    #[must_use]
    pub const fn with_modes(mut self, modes: AmrModeSet) -> Self {
        self.supported_modes = modes;
        self
    }

    /// Declare the ability to transmit with `mode-change-period=2`, which
    /// RFC 4867 recommends for interoperating with circuit-switched gateways.
    #[must_use]
    pub const fn with_restricted_mode_changes(mut self) -> Self {
        self.mode_change_capability = 2;
        self
    }

    /// Build an offer advertising these capabilities.
    ///
    /// Per RFC 4867, `mode-set` is omitted when every mode is supported —
    /// "an offerer supporting all modes and subsets SHOULD NOT include the
    /// mode-set parameter" — since including it would bind the answerer
    /// unnecessarily.
    #[must_use]
    pub fn to_offer(&self, octet_align: bool) -> AmrFmtp {
        let all = AmrModeSet::all(self.variant);
        AmrFmtp {
            variant: self.variant,
            octet_align,
            mode_set: (self.supported_modes != all).then(|| self.supported_modes.clone()),
            mode_change_period: self.mode_change_period,
            mode_change_capability: self.mode_change_capability,
            mode_change_neighbor: self.mode_change_neighbor,
            crc: false,
            robust_sorting: false,
            interleaving: None,
            channels: 1,
            max_red: self.max_red,
        }
    }

    /// Construct an SDP answer for `offer`, or explain why the payload type
    /// must be rejected.
    ///
    /// Implements RFC 4867 §8.3.1. In particular the answer echoes
    /// `octet-align`, `crc`, `robust-sorting`, `interleaving` and `channels`
    /// unmodified, and returns an offered `mode-set` unmodified rather than
    /// narrowing it.
    ///
    /// # Errors
    ///
    /// Returns an error when the payload type cannot be accepted: an unsupported
    /// transport combination, a `mode-set` containing modes this endpoint does
    /// not have, or a `mode-change-period=2` requirement neither side can meet.
    /// The caller should drop the payload type from the answer, not fail the
    /// whole session — other payload types may still be acceptable.
    pub fn answer(&self, offer: &AmrFmtp) -> Result<AmrFmtp> {
        if offer.variant != self.variant {
            return Err(CodecError::invalid_config(format!(
                "cannot answer a {} offer with {} capabilities",
                offer.variant, self.variant
            )));
        }
        if offer.channels != 1 {
            return Err(CodecError::invalid_format(format!(
                "only single-channel AMR is supported, offer requested {}",
                offer.channels
            )));
        }

        // Transport parameters are echoed unmodified or the payload type goes.
        // Check we can actually honour each before echoing it.
        let framing_ok = if offer.octet_align {
            self.supports_octet_align
        } else {
            self.supports_bandwidth_efficient
        };
        if !framing_ok {
            return Err(CodecError::invalid_format(format!(
                "{} framing is not supported",
                if offer.octet_align {
                    "octet-aligned"
                } else {
                    "bandwidth-efficient"
                }
            )));
        }
        if offer.crc && !self.supports_crc {
            return Err(CodecError::invalid_format("AMR crc=1 is not supported"));
        }
        if offer.robust_sorting && !self.supports_robust_sorting {
            return Err(CodecError::invalid_format(
                "AMR robust-sorting=1 is not supported",
            ));
        }
        if offer.interleaving.is_some() && !self.supports_interleaving {
            return Err(CodecError::invalid_format(
                "AMR interleaving is not supported",
            ));
        }

        // mode-set: return unmodified or reject. Narrowing it is a protocol
        // violation, and the peer would keep sending modes we said were fine.
        let mode_set = if let Some(offered) = &offer.mode_set {
            if !self.supported_modes.is_superset_of(offered) {
                return Err(CodecError::invalid_format(format!(
                    "offered mode-set {} includes modes this endpoint does not support ({})",
                    offered.to_sdp_value(),
                    self.supported_modes.to_sdp_value()
                )));
            }
            Some(offered.clone())
        } else {
            // The offer left it open, so we may bind it — and the choice
            // applies to both directions. Omit when we support everything.
            let all = AmrModeSet::all(self.variant);
            (self.supported_modes != all).then(|| self.supported_modes.clone())
        };

        // mode-change-period=2 is a requirement on what the *peer* transmits.
        // We may only demand it if the offerer said it can comply.
        let offerer_can_restrict =
            offer.mode_change_capability == 2 || offer.mode_change_period == 2;
        if self.mode_change_period == 2 && !offerer_can_restrict {
            return Err(CodecError::invalid_format(
                "this endpoint requires mode-change-period=2 but the offerer declared \
                 neither mode-change-capability=2 nor mode-change-period=2",
            ));
        }
        // Conversely, if the offerer requires period 2 of us, we must be able
        // to transmit that way.
        if offer.mode_change_period == 2 && self.mode_change_capability != 2 {
            return Err(CodecError::invalid_format(
                "the offer requires mode-change-period=2 but this endpoint cannot \
                 restrict its mode changes",
            ));
        }

        Ok(AmrFmtp {
            variant: self.variant,
            // Echoed verbatim.
            octet_align: offer.octet_align,
            crc: offer.crc,
            robust_sorting: offer.robust_sorting,
            interleaving: offer.interleaving,
            channels: offer.channels,
            mode_set,
            // Declarative, so we state our own.
            mode_change_period: self.mode_change_period,
            mode_change_capability: self.mode_change_capability,
            mode_change_neighbor: self.mode_change_neighbor,
            max_red: self.max_red,
        })
    }
}

/// Parse an RFC 4867 boolean parameter, which is spelled `0` or `1`.
fn parse_flag(name: &str, value: &str) -> Result<bool> {
    match value {
        "0" => Ok(false),
        "1" => Ok(true),
        // Be liberal about what has been seen in the field, but not silent:
        // anything else is a genuine syntax error.
        other => Err(CodecError::invalid_format(format!(
            "AMR {name} must be 0 or 1, got {other:?}"
        ))),
    }
}

/// Parse a parameter whose only legal values are 1 and 2.
fn parse_one_or_two(name: &str, value: &str) -> Result<u8> {
    match value {
        "1" => Ok(1),
        "2" => Ok(2),
        other => Err(CodecError::invalid_format(format!(
            "AMR {name} must be 1 or 2, got {other:?}"
        ))),
    }
}

/// Parse a `mode-set` value such as `0,2,4`.
fn parse_mode_set(variant: AmrVariant, value: &str) -> Result<AmrModeSet> {
    let mut modes = Vec::new();
    for token in value.split(',') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        modes.push(token.parse::<u8>().map_err(|_| {
            CodecError::invalid_format(format!("AMR mode-set entry {token:?} is not a number"))
        })?);
    }
    // from_indices rejects an empty list and out-of-range modes, which is what
    // the RFC requires for a mode-set naming a nonexistent mode.
    AmrModeSet::from_indices(variant, &modes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NB: AmrVariant = AmrVariant::NarrowBand;
    const WB: AmrVariant = AmrVariant::WideBand;

    #[test]
    fn absent_fmtp_means_rfc_defaults() {
        let fmtp = AmrFmtp::parse(WB, "").unwrap();
        assert_eq!(fmtp, AmrFmtp::default_for(WB));
        assert!(!fmtp.octet_align, "RFC default is bandwidth-efficient");
        assert!(fmtp.mode_set.is_none(), "absent mode-set means all modes");
        assert_eq!(fmtp.mode_change_period, 1);
        assert_eq!(fmtp.mode_change_capability, 1);
        assert_eq!(fmtp.channels, 1);
        // A fully default parameter set emits nothing, so no a=fmtp line.
        assert!(fmtp.to_fmtp_value().is_empty());
    }

    #[test]
    fn parses_a_realistic_volte_fmtp_line() {
        let fmtp = AmrFmtp::parse(
            WB,
            "octet-align=1; mode-set=0,1,2; mode-change-capability=2",
        )
        .unwrap();
        assert!(fmtp.octet_align);
        assert_eq!(fmtp.mode_set.as_ref().unwrap().to_sdp_value(), "0,1,2");
        assert_eq!(fmtp.mode_change_capability, 2);
    }

    #[test]
    fn round_trips_through_the_fmtp_string() {
        let original = AmrFmtp {
            variant: WB,
            octet_align: true,
            mode_set: Some(AmrModeSet::from_indices(WB, &[0, 2, 8]).unwrap()),
            mode_change_period: 2,
            mode_change_capability: 2,
            mode_change_neighbor: true,
            crc: true,
            robust_sorting: true,
            interleaving: Some(4),
            channels: 1,
            max_red: Some(220),
        };
        let text = original.to_fmtp_value();
        assert_eq!(AmrFmtp::parse(WB, &text).unwrap(), original, "{text}");
    }

    #[test]
    fn parsing_is_whitespace_and_case_insensitive() {
        // Real-world SDP varies in spacing and case.
        for text in [
            "octet-align=1;mode-set=0,1",
            "octet-align=1; mode-set=0,1",
            "  OCTET-ALIGN=1 ;  Mode-Set = 0,1  ",
            "octet-align=\"1\"; mode-set=\"0,1\"",
        ] {
            let fmtp = AmrFmtp::parse(NB, text).unwrap();
            assert!(fmtp.octet_align, "{text}");
            assert_eq!(
                fmtp.mode_set.as_ref().unwrap().to_sdp_value(),
                "0,1",
                "{text}"
            );
        }
    }

    #[test]
    fn unknown_parameters_are_ignored_not_rejected() {
        // SDP is extensible; a peer's private parameter must not fail the
        // payload type.
        let fmtp = AmrFmtp::parse(WB, "octet-align=1; vendor-magic=42; ptime=20").unwrap();
        assert!(fmtp.octet_align);
    }

    #[test]
    fn rejects_malformed_values() {
        assert!(AmrFmtp::parse(WB, "octet-align=2").is_err());
        assert!(AmrFmtp::parse(WB, "octet-align=yes").is_err());
        assert!(AmrFmtp::parse(WB, "mode-change-period=3").is_err());
        assert!(AmrFmtp::parse(WB, "mode-set=0,abc").is_err());
        assert!(AmrFmtp::parse(WB, "max-red=99999").is_err());
        // Mode 8 exists for wideband but not narrowband.
        assert!(AmrFmtp::parse(WB, "mode-set=8").is_ok());
        assert!(AmrFmtp::parse(NB, "mode-set=8").is_err());
        // Multi-channel is out of scope and must be refused explicitly.
        assert!(AmrFmtp::parse(WB, "channels=2").is_err());
    }

    #[test]
    fn crc_and_friends_imply_octet_alignment() {
        for text in ["crc=1", "robust-sorting=1", "interleaving=3"] {
            let fmtp = AmrFmtp::parse(WB, text).unwrap();
            assert!(fmtp.octet_align, "{text} must imply octet-align=1");
            assert!(fmtp.payload_config().octet_aligned);
        }
        // An offer that states the contradiction outright is an error rather
        // than something to silently correct.
        assert!(AmrFmtp::parse(WB, "octet-align=0; crc=1").is_err());
    }

    // ---- RFC 4867 §8.3.1 offer/answer ----

    #[test]
    fn answer_echoes_transport_parameters_unmodified() {
        // "An SDP answerer MUST include ... the following parameters unmodified
        // from the SDP offer: octet-align; crc; robust-sorting; interleaving;
        // and channels."
        let caps = AmrCapabilities::new(WB);
        for offer_text in ["", "octet-align=1", "crc=1", "robust-sorting=1"] {
            let offer = AmrFmtp::parse(WB, offer_text).unwrap();
            let answer = caps.answer(&offer).unwrap();
            assert!(
                answer.same_transport_as(&offer),
                "{offer_text}: transport parameters were modified"
            );
        }
    }

    #[test]
    fn answer_returns_an_offered_mode_set_unmodified_rather_than_narrowing_it() {
        // The rule people get wrong: the answerer must NOT intersect. Our
        // capabilities here are a strict superset of the offer.
        let caps = AmrCapabilities::new(WB).with_modes(AmrModeSet::all(WB));
        let offer = AmrFmtp::parse(WB, "mode-set=0,2,4").unwrap();
        let answer = caps.answer(&offer).unwrap();
        assert_eq!(answer.mode_set.as_ref().unwrap().to_sdp_value(), "0,2,4");

        // And when our own set is narrower than the offer, the payload type is
        // rejected outright — not answered with the smaller set.
        let narrow =
            AmrCapabilities::new(WB).with_modes(AmrModeSet::from_indices(WB, &[0, 2]).unwrap());
        let err = narrow.answer(&offer).unwrap_err();
        assert!(err.to_string().contains("mode-set"), "{err}");
    }

    #[test]
    fn answer_may_choose_a_mode_set_only_when_the_offer_omitted_one() {
        let caps =
            AmrCapabilities::new(WB).with_modes(AmrModeSet::from_indices(WB, &[0, 1, 2]).unwrap());

        // Offer omits mode-set -> we may bind it, and it binds both parties.
        let offer = AmrFmtp::parse(WB, "octet-align=1").unwrap();
        let answer = caps.answer(&offer).unwrap();
        assert_eq!(answer.mode_set.as_ref().unwrap().to_sdp_value(), "0,1,2");

        // An endpoint supporting everything omits mode-set entirely, per
        // "an offerer supporting all modes and subsets SHOULD NOT include the
        // mode-set parameter".
        let all = AmrCapabilities::new(WB);
        assert!(all.answer(&offer).unwrap().mode_set.is_none());
        assert!(all.to_offer(false).mode_set.is_none());
    }

    #[test]
    fn answer_rejects_transport_options_it_cannot_honour() {
        let mut caps = AmrCapabilities::new(WB);
        caps.supports_crc = false;
        caps.supports_robust_sorting = false;
        assert!(caps.answer(&AmrFmtp::parse(WB, "crc=1").unwrap()).is_err());
        assert!(caps
            .answer(&AmrFmtp::parse(WB, "robust-sorting=1").unwrap())
            .is_err());

        // Interleaving is off by default: the payload format carries ILL/ILP
        // but does not reorder, so accepting it would promise behaviour we do
        // not provide.
        let default_caps = AmrCapabilities::new(WB);
        assert!(!default_caps.supports_interleaving);
        assert!(default_caps
            .answer(&AmrFmtp::parse(WB, "interleaving=2").unwrap())
            .is_err());

        // An endpoint that only does octet-aligned must reject a
        // bandwidth-efficient offer rather than answer with the wrong framing.
        let mut oa_only = AmrCapabilities::new(WB);
        oa_only.supports_bandwidth_efficient = false;
        assert!(oa_only.answer(&AmrFmtp::default_for(WB)).is_err());
        assert!(oa_only
            .answer(&AmrFmtp::parse(WB, "octet-align=1").unwrap())
            .is_ok());
    }

    #[test]
    fn mode_change_period_two_requires_the_peer_to_have_declared_capability() {
        // We require period=2 of what we receive.
        let mut strict = AmrCapabilities::new(WB);
        strict.mode_change_period = 2;
        strict.mode_change_capability = 2;

        // Offerer says nothing: it cannot comply, so reject.
        assert!(strict.answer(&AmrFmtp::default_for(WB)).is_err());

        // Offerer declares capability=2: acceptable.
        let offer = AmrFmtp::parse(WB, "mode-change-capability=2").unwrap();
        assert_eq!(strict.answer(&offer).unwrap().mode_change_period, 2);

        // Offerer itself requires period=2, which also implies it can comply.
        let offer = AmrFmtp::parse(WB, "mode-change-period=2").unwrap();
        assert!(strict.answer(&offer).is_ok());
    }

    #[test]
    fn an_offer_requiring_restricted_mode_changes_needs_matching_capability() {
        let offer = AmrFmtp::parse(WB, "mode-change-period=2").unwrap();

        // We cannot restrict our transmissions -> reject the payload type.
        let plain = AmrCapabilities::new(WB);
        assert_eq!(plain.mode_change_capability, 1);
        assert!(plain.answer(&offer).is_err());

        // With the capability, we accept and advertise it.
        let capable = AmrCapabilities::new(WB).with_restricted_mode_changes();
        let answer = capable.answer(&offer).unwrap();
        assert_eq!(answer.mode_change_capability, 2);
    }

    #[test]
    fn answering_the_wrong_variant_is_a_local_error() {
        let caps = AmrCapabilities::new(WB);
        assert!(caps.answer(&AmrFmtp::default_for(NB)).is_err());
    }

    #[test]
    fn payload_config_follows_the_negotiated_parameters() {
        let fmtp = AmrFmtp::parse(WB, "octet-align=1; crc=1").unwrap();
        let config = fmtp.payload_config();
        assert_eq!(config.variant, WB);
        assert!(config.octet_aligned);
        assert!(config.crc);
        assert!(!config.robust_sorting);

        // And the config actually builds a working codec.
        assert!(super::super::payload::AmrPayloadCodec::new(config).is_ok());
    }

    #[test]
    fn active_modes_defaults_to_every_mode() {
        let fmtp = AmrFmtp::default_for(WB);
        assert_eq!(fmtp.active_modes(), AmrModeSet::all(WB));
        let fmtp = AmrFmtp::parse(WB, "mode-set=1,3").unwrap();
        assert_eq!(fmtp.active_modes().to_sdp_value(), "1,3");
    }

    #[test]
    fn transport_combinations_are_mutually_incompatible() {
        // RFC 4867: each combination "is unique in its bit-pattern and not
        // compatible with any other combination". This is why they are offered
        // as separate payload types rather than negotiated down.
        let be = AmrFmtp::default_for(WB);
        let oa = AmrFmtp::parse(WB, "octet-align=1").unwrap();
        let oa_crc = AmrFmtp::parse(WB, "octet-align=1; crc=1").unwrap();
        assert!(!be.same_transport_as(&oa));
        assert!(!oa.same_transport_as(&oa_crc));
        assert!(oa.same_transport_as(&AmrFmtp::parse(WB, "octet-align=1").unwrap()));
        // mode-set is not a transport parameter — it does not change framing.
        let oa_modes = AmrFmtp::parse(WB, "octet-align=1; mode-set=0,1").unwrap();
        assert!(oa.same_transport_as(&oa_modes));
    }

    #[test]
    fn a_full_offer_answer_exchange_agrees_on_one_wire_format() {
        // What the relay path depends on: after negotiation both ends must
        // build the same AmrPayloadConfig.
        let offerer = AmrCapabilities::new(WB)
            .with_modes(AmrModeSet::from_indices(WB, &[0, 1, 2, 3]).unwrap());
        let answerer = AmrCapabilities::new(WB);

        let offer = offerer.to_offer(true);
        let answer = answerer.answer(&offer).unwrap();

        assert_eq!(offer.payload_config(), answer.payload_config());
        assert_eq!(offer.active_modes(), answer.active_modes());
        assert_eq!(answer.active_modes().to_sdp_value(), "0,1,2,3");
    }

    // ---- captured from a real implementation ----

    /// The `a=fmtp` line `FreeSWITCH` 1.10.12 (`mod_amrwb`, `opencore-amrwb` +
    /// `vo-amrwbenc`) actually emits when originating an AMR-WB call.
    ///
    /// Captured from a live container rather than written by hand, so it pins
    /// this parser against a production stack instead of against our own
    /// reading of the RFC. The offer arrived on `a=rtpmap:102 AMR-WB/16000` —
    /// note the payload type is neither of the ones we offer, which is exactly
    /// why dynamic payload types must be resolved from the rtpmap.
    const FREESWITCH_AMRWB_OFFER: &str =
        "octet-align=0; mode-set=8; max-red=0; mode-change-capability=2";

    #[test]
    fn parses_a_real_freeswitch_amr_wb_offer() {
        let fmtp = AmrFmtp::parse(WB, FREESWITCH_AMRWB_OFFER).unwrap();

        // Bandwidth-efficient, stated explicitly rather than by omission.
        assert!(!fmtp.octet_align);
        assert!(!fmtp.payload_config().octet_aligned);

        // A single permitted mode: 23.85 kbit/s, matching mod_amrwb's
        // `default-bitrate` of 8.
        assert_eq!(fmtp.mode_set.as_ref().unwrap().to_sdp_value(), "8");
        assert_eq!(fmtp.active_modes().highest().unwrap().index(), 8);

        // FreeSWITCH declares it can restrict its mode changes, which is what
        // lets a peer require mode-change-period=2 of it.
        assert_eq!(fmtp.mode_change_capability, 2);
        assert_eq!(fmtp.max_red, Some(0));
    }

    #[test]
    fn answers_a_real_freeswitch_offer_compliantly() {
        let offer = AmrFmtp::parse(WB, FREESWITCH_AMRWB_OFFER).unwrap();
        let answer = AmrCapabilities::new(WB).answer(&offer).unwrap();

        // The transport parameters come back untouched, so both ends frame
        // identically — the property the relay depends on.
        assert!(answer.same_transport_as(&offer));
        assert!(!answer.octet_align);

        // mode-set is returned unmodified. We support all nine modes, but
        // narrowing to our own set here would be a protocol violation and
        // FreeSWITCH would keep sending mode 8 regardless.
        assert_eq!(answer.mode_set.as_ref().unwrap().to_sdp_value(), "8");
        assert_eq!(answer.active_modes().highest().unwrap().index(), 8);

        // And the negotiated framing is what the payload codec will use.
        assert_eq!(answer.payload_config(), offer.payload_config());
    }

    #[test]
    fn a_real_offer_survives_a_reparse_of_our_answer() {
        // What actually goes on the wire is the rendered string, so the round
        // trip through it has to preserve the negotiated meaning.
        let offer = AmrFmtp::parse(WB, FREESWITCH_AMRWB_OFFER).unwrap();
        let answer = AmrCapabilities::new(WB).answer(&offer).unwrap();

        let rendered = answer.to_fmtp_value();
        let reparsed = AmrFmtp::parse(WB, &rendered).unwrap();
        assert_eq!(reparsed, answer, "rendered as {rendered:?}");
        assert!(reparsed.same_transport_as(&offer));
    }

    #[test]
    fn a_peer_restricted_to_one_mode_still_negotiates() {
        // mode-set=8 alone is a legitimate and common gateway configuration.
        // It must not be mistaken for an empty or invalid set.
        let offer = AmrFmtp::parse(WB, FREESWITCH_AMRWB_OFFER).unwrap();
        let modes = offer.active_modes();
        assert_eq!(modes.modes().len(), 1);
        assert_eq!(modes.lowest(), modes.highest());

        // An endpoint that cannot do mode 8 must reject rather than negotiate
        // down to something the peer will not send.
        let limited =
            AmrCapabilities::new(WB).with_modes(AmrModeSet::from_indices(WB, &[0, 1]).unwrap());
        assert!(limited.answer(&offer).is_err());
    }
}
