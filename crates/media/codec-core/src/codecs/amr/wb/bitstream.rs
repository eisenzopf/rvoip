//! AMR-WB bitstream unpacking, 3GPP TS 26.201.
//!
//! Turns a frame's payload bits into the parameter indices the decoder
//! consumes. Two steps, and they are easy to conflate:
//!
//! 1. **Unsort.** The payload carries codec bits ordered by subjective
//!    importance, not in the order the decoder reads them. That ordering is
//!    what makes unequal error protection possible — a channel codec can
//!    protect the front of a frame more heavily than the tail — and it is why
//!    the class A bit counts are simply prefix lengths.
//! 2. **Walk the fields.** Each parameter is a fixed-width field read
//!    most-significant bit first, in a per-mode layout.
//!
//! A permutation that is self-consistent but wrong will round-trip perfectly
//! against itself, so the tests decode real third-party bitstreams rather than
//! round-tripping synthetic ones.

use super::sort_tables::{
    SORT_1265, SORT_1425, SORT_1585, SORT_1825, SORT_1985, SORT_2305, SORT_2385, SORT_660,
    SORT_885, SORT_SID,
};
use crate::codecs::amr::mode::AmrMode;

/// The largest frame, in bits — 23.85 kbit/s.
pub const MAX_FRAME_BITS: usize = 477;

/// Payload bit order for a mode, and how many bits it has.
#[must_use]
const fn sort_table(mode: AmrMode) -> &'static [u16] {
    match mode.index() {
        0 => &SORT_660,
        1 => &SORT_885,
        2 => &SORT_1265,
        3 => &SORT_1425,
        4 => &SORT_1585,
        5 => &SORT_1825,
        6 => &SORT_1985,
        7 => &SORT_2305,
        8 => &SORT_2385,
        // Every AMR-WB mode index above 8 is comfort noise as far as bit
        // ordering goes; the caller has already rejected reserved types.
        _ => &SORT_SID,
    }
}

/// A frame's codec bits, in the order the decoder reads them.
///
/// Deliberately not a bitfield: the reference works one bit per slot, the
/// permutation is random-access, and 477 bytes per frame is not worth the
/// bookkeeping of packing them.
#[derive(Debug, Clone)]
pub struct CodecBits {
    bits: [u8; MAX_FRAME_BITS],
    len: usize,
    cursor: usize,
}

impl CodecBits {
    /// Unsort a frame's payload bits into codec order.
    ///
    /// `payload` holds the frame's speech bits left-aligned, exactly as RFC
    /// 4867 carries them; trailing bits of the final octet are ignored.
    ///
    /// Returns `None` if `payload` is too short for the mode.
    #[must_use]
    pub fn unpack(mode: AmrMode, payload: &[u8]) -> Option<Self> {
        let sort = sort_table(mode);
        let len = sort.len();
        if payload.len() * 8 < len {
            return None;
        }

        let mut bits = [0u8; MAX_FRAME_BITS];
        for (i, &target) in sort.iter().enumerate() {
            let bit = (payload[i / 8] >> (7 - (i % 8))) & 1;
            bits[target as usize] = bit;
        }

        Some(Self {
            bits,
            len,
            cursor: 0,
        })
    }

    /// How many codec bits this frame holds.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether the frame is empty. Never true for a real mode.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// How many bits remain unread.
    #[must_use]
    pub const fn remaining(&self) -> usize {
        self.len - self.cursor
    }

    /// Read the next `width`-bit field, most-significant bit first.
    ///
    /// Returns `None` if fewer than `width` bits remain, so a layout that
    /// overruns its frame fails loudly rather than reading zeros.
    #[must_use]
    pub fn take(&mut self, width: usize) -> Option<u16> {
        debug_assert!(width <= 16, "a parameter field is at most 16 bits");
        if self.remaining() < width {
            return None;
        }
        let mut value = 0u16;
        for _ in 0..width {
            value = (value << 1) | u16::from(self.bits[self.cursor]);
            self.cursor += 1;
        }
        Some(value)
    }

    /// The raw codec bits, one per slot.
    #[must_use]
    pub fn bits(&self) -> &[u8] {
        &self.bits[..self.len]
    }
}

/// Field widths for the ISF quantiser indices, in read order.
///
/// The 6.60 kbit/s mode spends 36 bits on the spectrum and every other mode
/// spends 46. See [`super::lp::isf_dequant`].
#[must_use]
pub const fn isf_index_widths(mode: AmrMode) -> &'static [usize] {
    if mode.index() == 0 {
        &[8, 8, 7, 7, 6]
    } else {
        &[8, 8, 6, 7, 7, 5, 5]
    }
}

/// Read a frame's ISF quantiser indices.
///
/// They come first in every mode's layout, so this consumes the front of the
/// frame and leaves the cursor at the excitation parameters.
///
/// Returns `None` if the frame is too short.
#[must_use]
pub fn read_isf_indices(mode: AmrMode, bits: &mut CodecBits) -> Option<Vec<u16>> {
    isf_index_widths(mode)
        .iter()
        .map(|&w| bits.take(w))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::lp::isp_to_lp::tests_support::{block_has, block_row, has_block};
    use super::*;
    use crate::codecs::amr::mode::AmrVariant;
    use crate::codecs::amr::storage;

    /// The `.amr` fixtures, produced by the *other* oracles.
    fn fixture(mode_index: usize) -> &'static [u8] {
        const FILES: [&[u8]; 9] = [
            include_bytes!("../testdata/amrwb_mode0.amr"),
            include_bytes!("../testdata/amrwb_mode1.amr"),
            include_bytes!("../testdata/amrwb_mode2.amr"),
            include_bytes!("../testdata/amrwb_mode3.amr"),
            include_bytes!("../testdata/amrwb_mode4.amr"),
            include_bytes!("../testdata/amrwb_mode5.amr"),
            include_bytes!("../testdata/amrwb_mode6.amr"),
            include_bytes!("../testdata/amrwb_mode7.amr"),
            include_bytes!("../testdata/amrwb_mode8.amr"),
        ];
        FILES[mode_index]
    }

    /// Codec bits as the oracle prints them: hex, four bits per digit.
    fn bits_to_hex(bits: &[u8]) -> String {
        let mut out = String::new();
        for chunk in bits.chunks(4) {
            let mut nibble = 0u8;
            for i in 0..4 {
                nibble = (nibble << 1) | chunk.get(i).copied().unwrap_or(0);
            }
            out.push(char::from_digit(u32::from(nibble), 16).expect("nibble"));
        }
        out
    }

    #[test]
    fn unpacking_real_bitstreams_is_bit_exact_against_ts26173() {
        // The fixtures came from opencore-amr and vo-amrwbenc; the expected
        // values came from TS 26.173. Agreement across independent
        // implementations is what rules out a self-consistent wrong answer.
        let mut checked = 0;

        for mode_index in 0..9 {
            let block = format!("bitstream{mode_index}");
            assert!(has_block(&block), "fixture block {block} missing");

            let (_, frames) = storage::read(fixture(mode_index)).expect("fixture parses");

            for f in 0.. {
                if !block_has(&block, &format!("meta{f}")) {
                    break;
                }
                let meta = block_row(&block, &format!("meta{f}"));
                let want_mode = meta[0];
                let want_bits = usize::try_from(meta[1]).expect("bit count is positive");
                assert_eq!(
                    usize::try_from(want_mode).expect("mode"),
                    mode_index,
                    "{block} frame {f}: fixture is for a different mode"
                );

                let frame = frames.get(f).expect("fixture has this frame");
                let mode = AmrMode::new(AmrVariant::WideBand, u8::try_from(mode_index).expect("mode index")).expect("mode");
                let mut bits = CodecBits::unpack(mode, &frame.data).expect("unpacks");

                assert_eq!(bits.len(), want_bits, "{block} frame {f}: bit count");
                assert_eq!(
                    bits_to_hex(bits.bits()),
                    block_row_str(&block, &format!("bits{f}")),
                    "{block} frame {f}: unsorted codec bits"
                );

                let got = read_isf_indices(mode, &mut bits).expect("ISF indices");
                let want = block_row(&block, &format!("isfind{f}"));
                assert_eq!(got.len(), want.len(), "{block} frame {f}: index count");
                for (i, (&g, &w)) in got.iter().zip(want.iter()).enumerate() {
                    assert_eq!(
                        i64::from(g),
                        i64::from(w),
                        "{block} frame {f}: ISF index {i} = {g} but the reference gives {w}"
                    );
                }
                checked += 1;
            }
        }

        assert!(checked >= 18, "only {checked} frames checked");
    }

    /// The hex rows are strings, not integers, so they need their own reader.
    fn block_row_str(block: &str, label: &str) -> String {
        const LP_STAGES: &str = include_str!("../testdata/lp_stages_wb.txt");
        let mut in_block = false;
        for line in LP_STAGES.lines() {
            if line.trim_end() == block {
                in_block = true;
                continue;
            }
            if in_block {
                if !line.starts_with(' ') {
                    break;
                }
                let mut parts = line.split_whitespace();
                if parts.next() == Some(label) {
                    return parts.next().expect("value").to_owned();
                }
            }
        }
        panic!("block {block:?} has no row {label:?}");
    }

    #[test]
    fn every_sort_table_is_a_permutation() {
        // A duplicate or gap would silently drop a codec bit and leave another
        // at zero — a corruption that still decodes into plausible audio.
        for mode_index in 0..9 {
            let mode = AmrMode::new(AmrVariant::WideBand, u8::try_from(mode_index).expect("mode index")).expect("mode");
            let sort = sort_table(mode);
            let mut seen = vec![false; sort.len()];
            for &target in sort {
                let target = target as usize;
                assert!(target < seen.len(), "mode {mode_index}: index {target} out of range");
                assert!(!seen[target], "mode {mode_index}: index {target} appears twice");
                seen[target] = true;
            }
        }
    }

    #[test]
    fn a_short_payload_is_rejected_rather_than_padded() {
        let mode = AmrMode::new(AmrVariant::WideBand, 8).expect("mode");
        // 477 bits needs 60 octets; 59 must fail rather than read past the end.
        assert!(CodecBits::unpack(mode, &[0u8; 59]).is_none());
        assert!(CodecBits::unpack(mode, &[0u8; 60]).is_some());
    }

    #[test]
    fn reading_past_the_end_fails_rather_than_returning_zeros() {
        let mode = AmrMode::new(AmrVariant::WideBand, 0).expect("mode");
        let mut bits = CodecBits::unpack(mode, &[0xffu8; 17]).expect("unpacks");
        assert_eq!(bits.remaining(), 132);
        assert!(bits.take(16).is_some());
        // Drain the rest, then confirm the next read refuses.
        while bits.remaining() >= 8 {
            assert!(bits.take(8).is_some());
        }
        let left = bits.remaining();
        assert!(bits.take(left + 1).is_none(), "overrun was not rejected");
        assert!(bits.take(left).is_some());
    }

    #[test]
    fn the_payload_order_is_not_the_codec_order() {
        // Worth stating because it is easy to assume otherwise: the first
        // parameter field is *not* the leading octet of the payload. The
        // sorting scatters each field across the frame -- SORT_660 opens
        // `0, 5, 6, 7, ...`, so the eight bits of the first ISF index arrive
        // at payload positions 0, 31, 38, 32, 10, 1, 2, 3. An implementation
        // that skipped the permutation would still produce plausible-looking
        // indices, which is exactly why this needs a real-bitstream test
        // rather than a round trip.
        for mode_index in 0..9 {
            let mode = AmrMode::new(AmrVariant::WideBand, u8::try_from(mode_index).expect("mode index")).expect("mode");
            let sort = sort_table(mode);
            assert!(
                sort.iter().enumerate().any(|(i, &t)| usize::from(t) != i),
                "mode {mode_index}: sorting table is the identity"
            );
        }
    }
}
