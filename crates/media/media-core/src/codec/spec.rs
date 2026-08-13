//! What a negotiated audio codec *is*, as one value.
//!
//! # Why a payload type is not enough
//!
//! Most of this crate identifies a codec by its RTP payload type, which works
//! for the static assignments RFC 3551 fixed: 0 is PCMU and nothing else ever
//! is. It does not work for a dynamic one. Payload type 96 is AMR on one call,
//! AMR-WB on the next and Opus on a third, decided entirely by SDP — so a
//! function that takes a `u8` and returns a codec is guessing, and the places
//! that needed to stop guessing each invented their own way out.
//!
//! Opus got a special case in the transcoder and another in
//! `rvoip-core::media_graph`. AMR cannot get one at all: both its variants are
//! dynamic, they differ in clock rate and frame size, and the fmtp decides the
//! payload's bit layout on top of that.
//!
//! [`AudioCodecSpec`] carries what a negotiation actually produced — name,
//! payload type, clock rate, channels and fmtp — so the codec can be built
//! from the answer rather than reconstructed from one field of it.

use crate::codec::audio::common::AudioCodec;
use crate::error::{Error, Result};
// Only the opus arm constructs a typed CodecError today; the import must not
// dangle in builds without that feature.
#[cfg(feature = "opus")]
use crate::error::CodecError;

/// A negotiated audio codec, complete enough to construct.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AudioCodecSpec {
    /// The SDP encoding name, e.g. `PCMU`, `opus`, `AMR-WB`. Compared
    /// case-insensitively everywhere it is used.
    pub name: String,
    /// The negotiated RTP payload type.
    pub payload_type: u8,
    /// The RTP clock rate in hertz, which for these codecs is also the PCM
    /// sample rate.
    pub clock_rate: u32,
    /// Interleaved PCM channels.
    pub channels: u8,
    /// The negotiated `a=fmtp` string, when the answer carried one.
    pub fmtp: Option<String>,
}

impl AudioCodecSpec {
    /// A spec for one of the statically assigned payload types.
    ///
    /// Returns `None` for anything dynamic, because there is nothing to infer:
    /// the name has to come from the negotiation.
    #[must_use]
    pub fn from_static_payload_type(payload_type: u8) -> Option<Self> {
        let (name, clock_rate, channels) = match payload_type {
            0 => ("PCMU", 8_000, 1),
            8 => ("PCMA", 8_000, 1),
            9 => ("G722", 8_000, 1),
            18 => ("G729", 8_000, 1),
            _ => return None,
        };
        Some(Self {
            name: name.to_string(),
            payload_type,
            clock_rate,
            channels,
            fmtp: None,
        })
    }

    /// A spec with an explicit name and shape.
    #[must_use]
    pub fn new(name: &str, payload_type: u8, clock_rate: u32, channels: u8) -> Self {
        Self {
            name: name.to_string(),
            payload_type,
            clock_rate,
            channels,
            fmtp: None,
        }
    }

    /// The same spec carrying an fmtp.
    #[must_use]
    pub fn with_fmtp(mut self, fmtp: Option<&str>) -> Self {
        self.fmtp = fmtp.map(ToString::to_string);
        self
    }

    /// Samples in one 20 ms frame at this codec's rate.
    #[must_use]
    pub const fn frame_samples_20ms(&self) -> usize {
        (self.clock_rate as usize / 50) * self.channels as usize
    }

    /// Samples this codec requires in exactly one `encode` call, or `None`
    /// when it accepts whatever length it is handed.
    ///
    /// The distinction is not cosmetic. G.711 encodes any buffer sample by
    /// sample and Opus accepts several frame durations, so a caller can pass
    /// a 10 ms or 30 ms buffer and get sensible output. AMR cannot: both
    /// variants are defined on exactly one 20 ms frame, and
    /// `AmrAdapter::encode` rejects anything else outright rather than
    /// truncating or padding it.
    ///
    /// Callers that re-frame — a transcoding graph fed by a source with a
    /// different packet time — need to know which of those two worlds a
    /// target codec lives in before they buffer anything.
    ///
    /// G.729 is deliberately absent: its own 10 ms framing is handled inside
    /// `G729Codec`, which accepts whole multiples and is fed 20 ms buffers
    /// today, so declaring a requirement here would change working behaviour
    /// to no benefit.
    #[must_use]
    pub fn required_frame_samples(&self) -> Option<usize> {
        if self.name.eq_ignore_ascii_case("AMR") || self.name.eq_ignore_ascii_case("AMR-WB") {
            return Some(self.frame_samples_20ms());
        }
        None
    }

    /// Construct the codec this spec describes.
    ///
    /// # Errors
    ///
    /// When the name is unknown, when its cargo feature is not enabled, or
    /// when the shape or fmtp is one the codec cannot be built for.
    pub fn build(&self) -> Result<Box<dyn AudioCodec>> {
        let name = self.name.as_str();

        if name.eq_ignore_ascii_case("PCMU") || name.eq_ignore_ascii_case("G711U") {
            return Ok(Box::new(crate::codec::audio::G711Codec::mu_law(
                self.clock_rate,
                u16::from(self.channels),
            )?));
        }
        if name.eq_ignore_ascii_case("PCMA") || name.eq_ignore_ascii_case("G711A") {
            return Ok(Box::new(crate::codec::audio::G711Codec::a_law(
                self.clock_rate,
                u16::from(self.channels),
            )?));
        }
        if name.eq_ignore_ascii_case("PCM_S16LE") || name.eq_ignore_ascii_case("L16") {
            return Ok(Box::new(crate::codec::audio::PcmS16LeCodec::new(
                self.clock_rate,
                self.channels,
            )?));
        }

        #[cfg(feature = "g729")]
        if name.to_ascii_uppercase().starts_with("G729") {
            return Ok(Box::new(crate::codec::audio::G729Codec::new(
                crate::types::SampleRate::Rate8000,
                1,
                crate::codec::audio::G729Config::default(),
            )?));
        }

        #[cfg(feature = "opus")]
        if name.eq_ignore_ascii_case("opus") {
            let rate = crate::types::SampleRate::from_hz(self.clock_rate).ok_or_else(|| {
                Error::Codec(CodecError::InvalidParameters {
                    details: format!("invalid Opus sample rate: {}", self.clock_rate),
                })
            })?;
            return Ok(Box::new(crate::codec::audio::OpusCodec::new(
                rate,
                self.channels,
                crate::codec::audio::OpusConfig::default(),
            )?));
        }

        #[cfg(any(feature = "amr-nb", feature = "amr-wb"))]
        if name.eq_ignore_ascii_case("AMR") || name.eq_ignore_ascii_case("AMR-WB") {
            return Ok(Box::new(crate::codec::audio::amr::AmrAdapter::new(
                self.payload_type,
                name,
                self.fmtp.as_deref(),
            )?));
        }

        Err(Error::unsupported_codec(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_payload_types_map_to_their_codecs_and_dynamic_ones_do_not() {
        assert_eq!(
            AudioCodecSpec::from_static_payload_type(0).map(|s| s.name),
            Some("PCMU".to_string())
        );
        assert_eq!(
            AudioCodecSpec::from_static_payload_type(8).map(|s| s.name),
            Some("PCMA".to_string())
        );
        assert_eq!(
            AudioCodecSpec::from_static_payload_type(18).map(|s| s.name),
            Some("G729".to_string())
        );
        // Nothing to infer: 96 and 111 are whatever SDP said they were.
        for dynamic in [96u8, 97, 100, 111, 127] {
            assert!(
                AudioCodecSpec::from_static_payload_type(dynamic).is_none(),
                "{dynamic} is dynamic and must not be guessed"
            );
        }
    }

    #[test]
    fn a_frame_is_twenty_milliseconds_of_the_negotiated_rate() {
        assert_eq!(AudioCodecSpec::new("PCMU", 0, 8_000, 1).frame_samples_20ms(), 160);
        assert_eq!(AudioCodecSpec::new("AMR-WB", 97, 16_000, 1).frame_samples_20ms(), 320);
        assert_eq!(AudioCodecSpec::new("opus", 111, 48_000, 2).frame_samples_20ms(), 1920);
    }

    #[test]
    fn the_static_codecs_build() {
        for payload_type in [0u8, 8] {
            let spec = AudioCodecSpec::from_static_payload_type(payload_type).expect("static");
            assert!(spec.build().is_ok(), "PT {payload_type} should build");
        }
    }

    /// Two specs differing only in fmtp are different keys.
    ///
    /// The property a transcoding-session cache depends on: two AMR legs with
    /// different framing must not share one session, and they share a payload
    /// type.
    #[test]
    fn the_fmtp_is_part_of_a_specs_identity() {
        let plain = AudioCodecSpec::new("AMR", 96, 8_000, 1);
        let aligned = plain.clone().with_fmtp(Some("octet-align=1"));
        assert_ne!(plain, aligned);

        use std::collections::HashSet;
        let mut seen = HashSet::new();
        seen.insert(plain.clone());
        assert!(seen.insert(aligned), "the two must hash apart");
        assert!(!seen.insert(plain), "an identical spec must not");
    }

    #[test]
    fn an_unknown_name_does_not_build() {
        for name in ["not-a-codec", "", "AMR-NB"] {
            assert!(AudioCodecSpec::new(name, 96, 8_000, 1).build().is_err());
        }
    }

    #[test]
    #[cfg(feature = "amr-nb")]
    fn an_amr_spec_builds_at_its_negotiated_framing() {
        let spec = AudioCodecSpec::new("AMR", 96, 8_000, 1).with_fmtp(Some("octet-align=1"));
        let codec = spec.build().expect("builds");
        assert_eq!(codec.get_info().sample_rate, 8_000);
        assert_eq!(codec.get_info().name, "AMR");
    }
}
