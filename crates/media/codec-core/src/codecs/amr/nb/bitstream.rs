//! AMR-NB bitstream unpacking, 3GPP TS 26.101.
//!
//! Payload bits to parameter values. Two steps, as in [`super::super::wb`]:
//! unsort the perceptually-ordered payload into codec order, then walk the
//! fields.
//!
//! # Why this is simpler than the wideband version
//!
//! AMR-WB has essentially one frame layout with a few mode-dependent field
//! widths, so its walk is written out as code. AMR-NB has eight genuinely
//! different layouts, and the reference expresses each as a flat list of
//! parameter widths rather than as branching code — so this reads a table.
//!
//! That is not a stylistic difference. A table-driven walk cannot get a field
//! *order* wrong the way a hand-written one can, which is exactly the class of
//! bug that cost two rounds of debugging on the wideband side.

use super::tables::{
    BITNO_102, BITNO_122, BITNO_475, BITNO_515, BITNO_59, BITNO_67, BITNO_74, BITNO_795,
    BITNO_SID, SORT_102, SORT_122, SORT_475, SORT_515, SORT_59, SORT_67, SORT_74, SORT_795,
    SORT_SID,
};

/// The largest AMR-NB frame, in bits — 12.2 kbit/s.
pub const MAX_FRAME_BITS: usize = 244;

/// Payload bit order for a mode: `sort_table_for(m)[i]` is the codec-bit index
/// that payload position `i` carries.
///
/// Public because the conformance vectors ship in the ETSI *serial* format,
/// which is codec order, and converting between the two is the reader's job
/// rather than something to reimplement beside it.
#[must_use]
pub const fn sort_table_for(mode_index: u8) -> &'static [u16] {
    sort_table(mode_index)
}

/// Payload bit order for a mode.
const fn sort_table(mode_index: u8) -> &'static [u16] {
    match mode_index {
        0 => &SORT_475,
        1 => &SORT_515,
        2 => &SORT_59,
        3 => &SORT_67,
        4 => &SORT_74,
        5 => &SORT_795,
        6 => &SORT_102,
        7 => &SORT_122,
        // Every index above 7 is comfort noise as far as bit ordering goes.
        _ => &SORT_SID,
    }
}

/// Parameter widths for a mode, in the order the decoder reads them.
#[must_use]
pub const fn parameter_widths(mode_index: u8) -> &'static [usize] {
    match mode_index {
        0 => &BITNO_475,
        1 => &BITNO_515,
        2 => &BITNO_59,
        3 => &BITNO_67,
        4 => &BITNO_74,
        5 => &BITNO_795,
        6 => &BITNO_102,
        7 => &BITNO_122,
        _ => &BITNO_SID,
    }
}

/// Unsort a frame's payload into codec bit order.
///
/// `payload` holds the frame's speech bits left-aligned, exactly as RFC 4867
/// carries them; trailing bits of the final octet are ignored.
///
/// Returns `None` if `payload` is too short for the mode.
#[must_use]
pub fn unpack(mode_index: u8, payload: &[u8]) -> Option<Vec<u8>> {
    let sort = sort_table(mode_index);
    if payload.len() * 8 < sort.len() {
        return None;
    }
    let mut bits = vec![0u8; sort.len()];
    for (i, &target) in sort.iter().enumerate() {
        bits[target as usize] = (payload[i / 8] >> (7 - (i % 8))) & 1;
    }
    Some(bits)
}

/// Read a frame's parameters from unsorted codec bits.
///
/// Each parameter is a fixed-width field read most-significant bit first, in
/// the order [`parameter_widths`] gives.
///
/// Returns `None` if the bits run out, which means the tables and the payload
/// disagree about the mode.
#[must_use]
pub fn read_parameters(mode_index: u8, bits: &[u8]) -> Option<Vec<u16>> {
    let widths = parameter_widths(mode_index);
    let mut out = Vec::with_capacity(widths.len());
    let mut cursor = 0usize;

    for &width in widths {
        if cursor + width > bits.len() {
            return None;
        }
        let mut value = 0u16;
        for _ in 0..width {
            value = (value << 1) | u16::from(bits[cursor]);
            cursor += 1;
        }
        out.push(value);
    }

    // Conservation: the layout must consume the frame exactly. A width table
    // that is right in total but wrong in distribution would still pass a
    // length check, so this is necessary but not sufficient — the parameter
    // values are compared against the reference too.
    if cursor != bits.len() {
        return None;
    }
    Some(out)
}

/// Unsort and read in one step.
#[must_use]
pub fn parse(mode_index: u8, payload: &[u8]) -> Option<Vec<u16>> {
    let bits = unpack(mode_index, payload)?;
    read_parameters(mode_index, &bits)
}

/// Write a SID payload's STI bit and mode indication — `Write_serial`'s tail.
///
/// `update` is true for a `SID_UPDATE` and false for a `SID_FIRST`. `mode` is
/// the speech mode the encoder had been using, 0..=7.
///
/// The mode goes out **least significant bit first**, which nothing else in
/// this codec does; [`parse_sid_header`] undoes it, and the reason to write the
/// permutation out in both places rather than share a helper is that the two
/// are the only places it appears and each is checkable against its own line of
/// the reference.
///
/// # A `SID_FIRST` keeps its description here, unlike wideband
///
/// `Write_serial` writes the 35 description bits for both SID types and only
/// the STI bit distinguishes them. Wideband is the opposite — TS 26.173 blanks
/// a `SID_FIRST`'s payload — and carrying that habit across produces five
/// octets of `00 00 00 00 02` where the reference has a full description. The
/// decoders agree on ignoring it either way, since a `SID_FIRST` is decoded by
/// backward analysis, so nothing sounds wrong; only a bitstream comparison
/// catches it.
///
/// # Panics
/// If `payload` is not exactly five octets, or `mode` is not a speech mode.
pub fn finish_sid_payload(payload: &mut [u8], update: bool, mode: u8) {
    assert_eq!(payload.len(), 5, "a narrowband SID is five octets");
    assert!(mode < 8, "mode {mode} is not a speech mode");

    let reversed = ((mode & 4) >> 2) | (mode & 2) | ((mode & 1) << 2);
    // Bits 35..=38 of a 40-bit payload: the low five bits of the last octet,
    // of which the lowest is unused and stays clear.
    let tail = (u8::from(update) << 4) | (reversed << 1);
    payload[4] = (payload[4] & 0xE0) | tail;
}

/// A SID frame's two header fields — the bits after the 35 quantised ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidHeader {
    /// The SID type indicator. `false` is `SID_FIRST`, which carries no
    /// description; `true` is `SID_UPDATE`, which does.
    pub update: bool,
    /// Which speech mode the encoder had been using, 0..=7.
    pub mode_index: u8,
}

/// Read a SID frame's header, `UnpackBits`'s tail.
///
/// A narrowband SID payload is 39 bits: 35 quantised ones, then the STI bit,
/// then a three-bit mode indication. Returns `None` if the payload is shorter
/// than five octets.
///
/// **The mode indication is least-significant bit first**, and nothing else in
/// this codec is. The reference spells the reversal out as
/// `((m & 4) >> 2) | (m & 2) | ((m & 1) << 2)` after reading the three bits in
/// the usual order; reading them the usual way and stopping gives mode 1 for
/// mode 4 and mode 3 for mode 6, which are legal modes, so the error surfaces
/// only as a slightly wrong comfort-noise level.
#[must_use]
pub fn parse_sid_header(payload: &[u8]) -> Option<SidHeader> {
    if payload.len() < 5 {
        return None;
    }
    let bit = |i: usize| (payload[i / 8] >> (7 - (i % 8))) & 1;

    let update = bit(35) != 0;
    let reversed = (bit(36) << 2) | (bit(37) << 1) | bit(38);
    let mode_index = ((reversed & 4) >> 2) | (reversed & 2) | ((reversed & 1) << 2);

    Some(SidHeader { update, mode_index })
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAGES: &str = include_str!("../testdata/stages_nb.txt");

    fn block_rows(block: &str) -> impl Iterator<Item = &'static str> + '_ {
        STAGES
            .lines()
            .skip_while(move |l| l.trim_end() != block)
            .skip(1)
            .take_while(|l| l.starts_with(' '))
    }

    fn row(block: &str, label: &str) -> Vec<i64> {
        for line in block_rows(block) {
            let mut parts = line.split_whitespace();
            if parts.next() == Some(label) {
                return parts.map(|v| v.parse().expect("integer")).collect();
            }
        }
        panic!("block {block:?} has no row {label:?}");
    }

    fn row_str(block: &str, label: &str) -> String {
        for line in block_rows(block) {
            let mut parts = line.split_whitespace();
            if parts.next() == Some(label) {
                return parts.next().expect("value").to_owned();
            }
        }
        panic!("block {block:?} has no row {label:?}");
    }

    fn has_row(block: &str, label: &str) -> bool {
        block_rows(block).any(|l| l.split_whitespace().next() == Some(label))
    }

    fn fixture(mode_index: usize) -> &'static [u8] {
        const FILES: [&[u8]; 8] = [
            include_bytes!("../testdata/amrnb_mode0.amr"),
            include_bytes!("../testdata/amrnb_mode1.amr"),
            include_bytes!("../testdata/amrnb_mode2.amr"),
            include_bytes!("../testdata/amrnb_mode3.amr"),
            include_bytes!("../testdata/amrnb_mode4.amr"),
            include_bytes!("../testdata/amrnb_mode5.amr"),
            include_bytes!("../testdata/amrnb_mode6.amr"),
            include_bytes!("../testdata/amrnb_mode7.amr"),
        ];
        FILES[mode_index]
    }

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
    fn unpacking_real_bitstreams_is_bit_exact_against_ts26073() {
        // The fixtures came from opencore-amr; the expected values from
        // TS 26.073. Two independent implementations agreeing on real
        // bitstreams is what rules out a self-consistent wrong answer.
        let mut checked = 0;

        for mode_index in 0..8u8 {
            let block = format!("nb{mode_index}");
            let bytes = fixture(usize::from(mode_index));
            // Skip the 6-byte "#!AMR\n" magic, then walk ToC-prefixed frames.
            let mut offset = 6usize;

            for f in 0.. {
                if !has_row(&block, &format!("meta{f}")) {
                    break;
                }
                let meta = row(&block, &format!("meta{f}"));
                let want_mode = meta[0];
                let want_bits = usize::try_from(meta[1]).expect("bit count");
                let want_params = usize::try_from(meta[2]).expect("parameter count");
                assert_eq!(want_mode, i64::from(mode_index), "{block}: mode");

                let toc = bytes[offset];
                assert_eq!(
                    i64::from((toc >> 3) & 0x0f),
                    want_mode,
                    "{block} frame {f}: ToC mode"
                );
                let payload_len = want_bits.div_ceil(8);
                let payload = &bytes[offset + 1..offset + 1 + payload_len];

                let bits = unpack(mode_index, payload).expect("unpacks");
                assert_eq!(bits.len(), want_bits, "{block} frame {f}: bit count");
                assert_eq!(
                    bits_to_hex(&bits),
                    row_str(&block, &format!("bits{f}")),
                    "{block} frame {f}: unsorted codec bits"
                );

                let got = read_parameters(mode_index, &bits).expect("parameters");
                let want = row(&block, &format!("prm{f}"));
                assert_eq!(got.len(), want_params, "{block} frame {f}: parameter count");
                assert_eq!(got.len(), want.len(), "{block} frame {f}: count vs fixture");
                for (i, (&g, &w)) in got.iter().zip(want.iter()).enumerate() {
                    assert_eq!(
                        i64::from(g),
                        w,
                        "{block} frame {f}: parameter {i} = {g} but the reference gives {w}"
                    );
                }

                // AMR-NB's packed_size in the reference includes the ToC byte,
                // where AMR-WB's does not. Advancing by payload + 1 here keeps
                // that difference in one place.
                offset += 1 + payload_len;
                checked += 1;
            }
        }

        assert!(checked >= 16, "only {checked} frames checked");
    }

    #[test]
    fn every_layout_consumes_its_frame_exactly() {
        // The width tables and the sorting tables are independent in the
        // reference. If they disagree on any mode's length, one was
        // transcribed wrong.
        for mode_index in 0..8u8 {
            let widths: usize = parameter_widths(mode_index).iter().sum();
            let sorted = sort_table(mode_index).len();
            assert_eq!(
                widths, sorted,
                "mode {mode_index}: widths sum to {widths} but the sort table has {sorted}"
            );
        }
    }

    #[test]
    fn every_sort_table_is_a_permutation() {
        // A duplicate or gap would silently drop one codec bit and leave
        // another at zero — corruption that still decodes into plausible audio.
        for mode_index in 0..8u8 {
            let sort = sort_table(mode_index);
            let mut seen = vec![false; sort.len()];
            for &target in sort {
                let t = target as usize;
                assert!(t < seen.len(), "mode {mode_index}: index {t} out of range");
                assert!(!seen[t], "mode {mode_index}: index {t} appears twice");
                seen[t] = true;
            }
        }
    }

    #[test]
    fn the_parameter_count_grows_with_the_bit_rate() {
        // 4.75 codes two pulses per subframe and 12.2 codes ten, so the higher
        // rates carry strictly more parameters.
        let counts: Vec<usize> = (0..8u8)
            .map(|m| parameter_widths(m).len())
            .collect();
        assert!(
            counts[7] > counts[0],
            "12.2 carries {} parameters against 4.75's {}",
            counts[7],
            counts[0]
        );
    }

    #[test]
    fn a_truncated_payload_is_rejected() {
        let full = vec![0u8; 31];
        assert!(unpack(7, &full).is_some());
        assert!(unpack(7, &full[..30]).is_none());
    }
}
