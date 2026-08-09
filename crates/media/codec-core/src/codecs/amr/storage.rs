//! RFC 4867 §5 AMR file storage format.
//!
//! This is the `.amr` container: a magic number identifying the variant,
//! followed by one octet-aligned record per 20 ms frame. It matters here
//! mainly because **it is the format the 3GPP conformance sequences ship in**,
//! so it is the gateway for every test vector the codec kernel will be
//! validated against.
//!
//! ```text
//! "#!AMR\n"  or  "#!AMR-WB\n"
//! | P FT(4) Q P P | speech octets (zero-padded) |   <- one per frame
//! | P FT(4) Q P P | speech octets (zero-padded) |
//! ...
//! ```
//!
//! The frame header is the octet-aligned RTP table-of-contents entry with the F
//! bit replaced by padding — there is no frame chaining, since the record
//! length is implied by FT.
//!
//! Only single-channel files are supported. The multi-channel magics
//! (`#!AMR_MC1.0\n`, `#!AMR-WB_MC1.0\n`) are recognised so they can be
//! rejected with a useful message rather than parsed as garbage.

use super::mode::{AmrFrameType, AmrVariant};
use super::payload::AmrPayloadFrame;
use crate::error::{CodecError, Result};

/// Multi-channel magic numbers, recognised only to reject them clearly.
const MULTICHANNEL_MAGICS: [&[u8]; 2] = [b"#!AMR_MC1.0\n", b"#!AMR-WB_MC1.0\n"];

/// Serialise frames as a single-channel `.amr` file.
///
/// # Errors
///
/// Returns an error when a frame's `data` length disagrees with its frame type,
/// or when a frame belongs to a different variant.
pub fn write(variant: AmrVariant, frames: &[AmrPayloadFrame]) -> Result<Vec<u8>> {
    let mut out = Vec::from(variant.storage_magic());
    for frame in frames {
        let expected = frame.frame_type.octet_aligned_bytes();
        if frame.data.len() != expected {
            return Err(CodecError::InvalidFrameSize {
                expected,
                actual: frame.data.len(),
            });
        }
        if let Some(frame_variant) = frame_variant(frame.frame_type) {
            if frame_variant != variant {
                return Err(CodecError::invalid_format(format!(
                    "cannot store a {frame_variant} frame in a {variant} file"
                )));
            }
        }
        // P | FT(4) | Q | P P  — the RTP ToC entry with F replaced by padding.
        out.push(frame.frame_type.frame_type_index() << 3 | u8::from(frame.quality_ok) << 2);
        out.extend_from_slice(&frame.data);
    }
    Ok(out)
}

/// Parse a single-channel `.amr` file, returning its variant and frames.
///
/// # Errors
///
/// Returns an error for an unrecognised or multi-channel magic number, a
/// reserved frame type, or a truncated final record.
pub fn read(bytes: &[u8]) -> Result<(AmrVariant, Vec<AmrPayloadFrame>)> {
    let mut reader = AmrStorageReader::new(bytes)?;
    let variant = reader.variant();
    let mut frames = Vec::new();
    while let Some(frame) = reader.next_frame()? {
        frames.push(frame);
    }
    Ok((variant, frames))
}

/// Incremental reader over a single-channel `.amr` file.
///
/// Preferred over [`read`] for the 3GPP conformance sequences, which are large
/// enough that materialising every frame at once is wasteful.
#[derive(Debug)]
pub struct AmrStorageReader<'a> {
    variant: AmrVariant,
    rest: &'a [u8],
}

impl<'a> AmrStorageReader<'a> {
    /// Parse the magic number and position at the first frame.
    ///
    /// # Errors
    ///
    /// Returns an error when the magic number is absent, unrecognised, or one
    /// of the multi-channel forms.
    pub fn new(bytes: &'a [u8]) -> Result<Self> {
        for magic in MULTICHANNEL_MAGICS {
            if bytes.starts_with(magic) {
                return Err(CodecError::invalid_format(format!(
                    "multi-channel AMR storage ({}) is not supported",
                    String::from_utf8_lossy(&magic[..magic.len() - 1])
                )));
            }
        }
        // Check wideband first: "#!AMR\n" is not a prefix of "#!AMR-WB\n" (the
        // newline differs), but ordering it this way keeps the intent obvious.
        for variant in [AmrVariant::WideBand, AmrVariant::NarrowBand] {
            let magic = variant.storage_magic();
            if bytes.starts_with(magic) {
                return Ok(Self {
                    variant,
                    rest: &bytes[magic.len()..],
                });
            }
        }
        Err(CodecError::invalid_format(
            "not an AMR storage file: expected a \"#!AMR\\n\" or \"#!AMR-WB\\n\" magic number",
        ))
    }

    /// The variant declared by the magic number.
    #[must_use]
    pub const fn variant(&self) -> AmrVariant {
        self.variant
    }

    /// Read the next frame, or `None` at end of file.
    ///
    /// # Errors
    ///
    /// Returns an error for a reserved frame type or a truncated record.
    pub fn next_frame(&mut self) -> Result<Option<AmrPayloadFrame>> {
        let Some((&header, rest)) = self.rest.split_first() else {
            return Ok(None);
        };

        let ft_index = header >> 3 & 0x0F;
        let quality_ok = header >> 2 & 1 == 1;
        let frame_type = AmrFrameType::from_index(self.variant, ft_index)?;

        let len = frame_type.octet_aligned_bytes();
        if rest.len() < len {
            return Err(CodecError::InvalidPayload {
                details: format!(
                    "truncated AMR storage record: frame type {ft_index} needs {len} octets, \
                     {} remain",
                    rest.len()
                ),
            });
        }
        let (data, tail) = rest.split_at(len);
        self.rest = tail;

        Ok(Some(AmrPayloadFrame {
            frame_type,
            quality_ok,
            data: data.to_vec(),
        }))
    }
}

/// The variant a frame type belongs to, if it implies one.
const fn frame_variant(frame_type: AmrFrameType) -> Option<AmrVariant> {
    match frame_type {
        AmrFrameType::Speech(mode) => Some(mode.variant()),
        AmrFrameType::Sid(variant) => Some(variant),
        AmrFrameType::NoData => None,
        AmrFrameType::SpeechLost => Some(AmrVariant::WideBand),
    }
}

#[cfg(test)]
// Binary literals are grouped by protocol field (P|FT|Q|P|P) to mirror the
// RFC 4867 storage-record diagram.
#[allow(clippy::unusual_byte_groupings)]
mod tests {
    use super::*;
    use crate::codecs::amr::mode::AmrMode;

    fn frame_data(bits: usize, seed: u8) -> Vec<u8> {
        let len = bits.div_ceil(8);
        let mut data: Vec<u8> = (0..len)
            .map(|i| u8::try_from(i).unwrap_or(0).wrapping_mul(29).wrapping_add(seed) | 0x21)
            .collect();
        let tail = bits % 8;
        if tail != 0 {
            let last = len - 1;
            data[last] &= 0xFFu8 << (8 - tail);
        }
        data
    }

    fn speech(variant: AmrVariant, index: u8) -> AmrPayloadFrame {
        let mode = AmrMode::new(variant, index).unwrap();
        AmrPayloadFrame::new(
            AmrFrameType::Speech(mode),
            true,
            frame_data(mode.bits(), index),
        )
        .unwrap()
    }

    #[test]
    fn round_trips_every_mode_for_both_variants() {
        for variant in [AmrVariant::NarrowBand, AmrVariant::WideBand] {
            let mut frames: Vec<_> = AmrMode::all(variant)
                .into_iter()
                .map(|mode| speech(variant, mode.index()))
                .collect();
            frames.push(
                AmrPayloadFrame::new(
                    AmrFrameType::Sid(variant),
                    true,
                    frame_data(variant.sid_bits(), 1),
                )
                .unwrap(),
            );
            frames.push(AmrPayloadFrame::no_data());
            if variant == AmrVariant::WideBand {
                frames.push(AmrPayloadFrame::speech_lost());
            }

            let bytes = write(variant, &frames).unwrap();
            let (got_variant, got_frames) = read(&bytes).unwrap();
            assert_eq!(got_variant, variant);
            assert_eq!(got_frames, frames);
        }
    }

    #[test]
    fn magic_numbers_match_rfc4867() {
        let nb = write(AmrVariant::NarrowBand, &[]).unwrap();
        assert_eq!(nb, b"#!AMR\n");
        let wb = write(AmrVariant::WideBand, &[]).unwrap();
        assert_eq!(wb, b"#!AMR-WB\n");
    }

    #[test]
    fn record_layout_matches_rfc4867() {
        // AMR-NB mode 7 is 244 bits -> 31 octets, plus a 1-octet header.
        let frame = speech(AmrVariant::NarrowBand, 7);
        let bytes = write(AmrVariant::NarrowBand, &[frame]).unwrap();
        assert_eq!(bytes.len(), b"#!AMR\n".len() + 1 + 31);
        // Header: P=0, FT=7, Q=1, PP=00 -> 0011_1100
        assert_eq!(bytes[b"#!AMR\n".len()], 0b0_0111_1_00);
    }

    #[test]
    fn quality_bit_round_trips() {
        let mut frame = speech(AmrVariant::WideBand, 4);
        frame.quality_ok = false;
        let bytes = write(AmrVariant::WideBand, &[frame.clone()]).unwrap();
        let (_, frames) = read(&bytes).unwrap();
        assert!(!frames[0].quality_ok);
        assert_eq!(frames[0], frame);
    }

    #[test]
    fn an_empty_file_is_a_magic_number_and_no_frames() {
        let bytes = write(AmrVariant::WideBand, &[]).unwrap();
        let (variant, frames) = read(&bytes).unwrap();
        assert_eq!(variant, AmrVariant::WideBand);
        assert!(frames.is_empty());
    }

    #[test]
    fn the_variants_are_distinguished_by_magic() {
        // The narrowband magic must not be read as a wideband prefix or vice
        // versa; the mode tables differ, so this would silently mis-size every
        // frame.
        let nb_bytes = write(AmrVariant::NarrowBand, &[speech(AmrVariant::NarrowBand, 0)]).unwrap();
        assert_eq!(read(&nb_bytes).unwrap().0, AmrVariant::NarrowBand);

        let wb_bytes = write(AmrVariant::WideBand, &[speech(AmrVariant::WideBand, 0)]).unwrap();
        assert_eq!(read(&wb_bytes).unwrap().0, AmrVariant::WideBand);
    }

    #[test]
    fn rejects_unknown_and_multichannel_magics() {
        assert!(read(b"").is_err());
        assert!(read(b"#!AMR").is_err());
        assert!(read(b"#!OPUS\n").is_err());
        assert!(read(b"not an amr file at all").is_err());

        for magic in MULTICHANNEL_MAGICS {
            let err = read(magic).unwrap_err();
            let text = err.to_string();
            assert!(text.contains("multi-channel"), "{text}");
        }
    }

    #[test]
    fn rejects_a_truncated_final_record() {
        let frame = speech(AmrVariant::WideBand, 8);
        let full = write(AmrVariant::WideBand, &[frame]).unwrap();
        // Every proper prefix past the magic must fail rather than silently
        // yielding a short frame.
        for cut in b"#!AMR-WB\n".len() + 1..full.len() {
            assert!(read(&full[..cut]).is_err(), "{cut}-byte prefix decoded");
        }
        assert!(read(&full).is_ok());
    }

    #[test]
    fn rejects_reserved_frame_types() {
        // NB reserves FT 9-14.
        for ft in 9..=14u8 {
            let mut bytes = Vec::from(&b"#!AMR\n"[..]);
            bytes.push(ft << 3 | 0b100);
            assert!(read(&bytes).is_err(), "NB FT {ft} should be rejected");
        }
        // WB reserves 10-13; 14 is SPEECH_LOST and is valid.
        for ft in 10..=13u8 {
            let mut bytes = Vec::from(&b"#!AMR-WB\n"[..]);
            bytes.push(ft << 3 | 0b100);
            assert!(read(&bytes).is_err(), "WB FT {ft} should be rejected");
        }
        let mut bytes = Vec::from(&b"#!AMR-WB\n"[..]);
        bytes.push(14 << 3 | 0b100);
        assert!(read(&bytes).is_ok());
    }

    #[test]
    fn rejects_cross_variant_frames_on_write() {
        let wb_frame = speech(AmrVariant::WideBand, 2);
        assert!(write(AmrVariant::NarrowBand, &[wb_frame]).is_err());
    }

    #[test]
    fn incremental_reader_matches_the_whole_file_reader() {
        let variant = AmrVariant::WideBand;
        let frames: Vec<_> = (0..9).map(|i| speech(variant, i)).collect();
        let bytes = write(variant, &frames).unwrap();

        let mut reader = AmrStorageReader::new(&bytes).unwrap();
        assert_eq!(reader.variant(), variant);
        let mut streamed = Vec::new();
        while let Some(frame) = reader.next_frame().unwrap() {
            streamed.push(frame);
        }
        assert_eq!(streamed, frames);
        // Exhausted readers keep returning None rather than erroring.
        assert!(reader.next_frame().unwrap().is_none());
    }

    #[test]
    fn storage_and_rtp_frames_share_a_representation() {
        // A frame read from a .amr file must be packable into an RTP payload
        // without conversion — this is what lets conformance vectors drive the
        // payload tests directly.
        use crate::codecs::amr::payload::{AmrPacket, AmrPayloadCodec, AmrPayloadConfig};

        let variant = AmrVariant::WideBand;
        let frames: Vec<_> = (0..9).map(|i| speech(variant, i)).collect();
        let file = write(variant, &frames).unwrap();
        let (_, decoded) = read(&file).unwrap();

        let codec =
            AmrPayloadCodec::new(AmrPayloadConfig::bandwidth_efficient(variant)).unwrap();
        for frame in decoded {
            let packet = AmrPacket::single(frame);
            let bytes = codec.pack(&packet).unwrap();
            assert_eq!(codec.unpack(&bytes).unwrap(), packet);
        }
    }

    #[test]
    fn never_panics_on_arbitrary_input() {
        for byte in 0u16..=u16::from(u8::MAX) {
            let b = u8::try_from(byte).unwrap_or(0);
            for len in 0..6usize {
                let mut bytes = Vec::from(&b"#!AMR-WB\n"[..]);
                bytes.extend(std::iter::repeat_n(b, len));
                let _ = read(&bytes);
                let mut bytes = Vec::from(&b"#!AMR\n"[..]);
                bytes.extend(std::iter::repeat_n(b, len));
                let _ = read(&bytes);
            }
        }
    }
}
