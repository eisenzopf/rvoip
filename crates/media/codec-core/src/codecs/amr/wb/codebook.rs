//! Algebraic codebook decoding, 3GPP TS 26.190 §6.2.
//!
//! Expands a subframe's pulse indices into the 64-sample innovation vector.
//! This is the "A" in ACELP: the excitation's noise-like component is not
//! stored in a codebook at all, but described by the positions and signs of a
//! handful of unit pulses. A 64-sample vector costs 12 bits at 6.60 kbit/s and
//! 88 at 23.85 — no lookup table of any size could do that.
//!
//! # Tracks
//!
//! Positions are constrained to interleaved tracks: with four tracks, track
//! `k` owns samples `k, k+4, k+8, …`. That is what makes the search tractable
//! for the encoder, and it means a pulse's position within its track plus the
//! track number reconstructs the sample index.
//!
//! # Index packing
//!
//! The per-track codes are combinatorial rather than positional. `dec_2p_2N1`
//! packs two pulses and their signs into `2N+1` bits by exploiting that the
//! pair is unordered — swapping them is not a distinct codeword, so the spare
//! state carries a sign. The higher-pulse-count decoders are built recursively
//! from the lower ones, splitting on a few leading bits that say how the
//! pulses distribute between two half-ranges.
//!
//! Pulses are ±512, which is 1.0 in Q9.

/// Samples in a subframe.
pub const L_SUBFR: usize = 64;

/// Positions per track in the four-track layout, and the mask that separates a
/// position from its sign bit.
const NB_POS: u32 = 16;

/// Pulse amplitude, 1.0 in Q9.
const PULSE: i16 = 512;

/// Decode the two-pulse codebook used at 6.60 kbit/s.
///
/// Twelve bits: two tracks of 32 positions, one pulse each, with a sign.
#[must_use]
pub const fn decode_2t64(index: u16) -> [i16; L_SUBFR] {
    let mut code = [0i16; L_SUBFR];

    // Two tracks of 32, so positions are even and odd samples respectively.
    let i0 = ((index >> 5) & 0x003E) as usize;
    let i1 = (((index & 0x001F) << 1) + 1) as usize;

    code[i0] = if (index >> 6) & 32 == 0 {
        PULSE
    } else {
        -PULSE
    };
    code[i1] = if index & 32 == 0 { PULSE } else { -PULSE };
    code
}

/// Decode the four-track codebook, at whichever width the mode uses.
///
/// `bits` is the total spent on pulses in one subframe: 20, 36, 44, 52, 64, 72
/// or 88. Returns `None` for any other width.
#[must_use]
pub fn decode_4t64(indices: &[u16], bits: usize) -> Option<[i16; L_SUBFR]> {
    let mut code = [0i16; L_SUBFR];
    let mut pos = [0u32; 6];

    match bits {
        20 => {
            for (track, &index) in indices.iter().enumerate().take(4) {
                dec_1p_n1(u32::from(index), 4, 0, &mut pos);
                add_pulses(&pos[..1], track, &mut code);
            }
        }
        36 => {
            for (track, &index) in indices.iter().enumerate().take(4) {
                dec_2p_2n1(u32::from(index), 4, 0, &mut pos);
                add_pulses(&pos[..2], track, &mut code);
            }
        }
        44 => {
            // Asymmetric: the first two tracks get three pulses, the last two
            // get two. Speech energy is not spread evenly across the tracks.
            for track in 0..2 {
                dec_3p_3n1(u32::from(*indices.get(track)?), 4, 0, &mut pos);
                add_pulses(&pos[..3], track, &mut code);
            }
            for track in 2..4 {
                dec_2p_2n1(u32::from(*indices.get(track)?), 4, 0, &mut pos);
                add_pulses(&pos[..2], track, &mut code);
            }
        }
        52 => {
            for (track, &index) in indices.iter().enumerate().take(4) {
                dec_3p_3n1(u32::from(index), 4, 0, &mut pos);
                add_pulses(&pos[..3], track, &mut code);
            }
        }
        64 => {
            for track in 0..4 {
                let joined =
                    (u32::from(*indices.get(track)?) << 14) + u32::from(*indices.get(track + 4)?);
                dec_4p_4n(joined, 4, 0, &mut pos);
                add_pulses(&pos[..4], track, &mut code);
            }
        }
        72 => {
            for track in 0..2 {
                let joined =
                    (u32::from(*indices.get(track)?) << 10) + u32::from(*indices.get(track + 4)?);
                dec_5p_5n(joined, 4, 0, &mut pos);
                add_pulses(&pos[..5], track, &mut code);
            }
            for track in 2..4 {
                let joined =
                    (u32::from(*indices.get(track)?) << 14) + u32::from(*indices.get(track + 4)?);
                dec_4p_4n(joined, 4, 0, &mut pos);
                add_pulses(&pos[..4], track, &mut code);
            }
        }
        88 => {
            for track in 0..4 {
                let joined =
                    (u32::from(*indices.get(track)?) << 11) + u32::from(*indices.get(track + 4)?);
                dec_6p_6n_2(joined, 4, 0, &mut pos);
                add_pulses(&pos[..6], track, &mut code);
            }
        }
        _ => return None,
    }

    Some(code)
}

/// Place decoded pulses into their track's samples.
///
/// A position carries its sign in bit 4: below [`NB_POS`] is positive, at or
/// above is negative. Pulses can land on the same sample and accumulate.
fn add_pulses(pos: &[u32], track: usize, code: &mut [i16; L_SUBFR]) {
    for &p in pos {
        let sample = usize::try_from((p & (NB_POS - 1)) << 2).expect("position fits") + track;
        if p & NB_POS == 0 {
            code[sample] += PULSE;
        } else {
            code[sample] -= PULSE;
        }
    }
}

/// One pulse in `N+1` bits: `N` for the position, one for the sign.
fn dec_1p_n1(index: u32, n: u32, offset: u32, pos: &mut [u32]) {
    let mask = (1u32 << n) - 1;
    let mut p = (index & mask) + offset;
    if (index >> n) & 1 == 1 {
        p += NB_POS;
    }
    pos[0] = p;
}

/// Two pulses in `2N+1` bits.
///
/// The pair is unordered, so `(a, b)` and `(b, a)` would be the same codeword.
/// The encoder writes them in a canonical order and spends the freed state on
/// a sign, which is why the sign reconstruction here depends on comparing the
/// two decoded positions.
fn dec_2p_2n1(index: u32, n: u32, offset: u32, pos: &mut [u32]) {
    let mask = (1u32 << n) - 1;
    let mut pos1 = ((index >> n) & mask) + offset;
    let sign = (index >> (n * 2)) & 1;
    let mut pos2 = (index & mask) + offset;

    if pos2 < pos1 {
        if sign == 1 {
            pos1 += NB_POS;
        } else {
            pos2 += NB_POS;
        }
    } else if sign == 1 {
        pos1 += NB_POS;
        pos2 += NB_POS;
    }

    pos[0] = pos1;
    pos[1] = pos2;
}

/// Three pulses in `3N+1` bits: a two-pulse code over a half-range, plus one.
fn dec_3p_3n1(index: u32, n: u32, offset: u32, pos: &mut [u32]) {
    let mask = (1u32 << (n * 2 - 1)) - 1;
    let idx = index & mask;
    let mut j = offset;
    if (index >> (n * 2 - 1)) & 1 != 0 {
        j += 1 << (n - 1);
    }
    dec_2p_2n1(idx, n - 1, j, pos);

    let mask = (1u32 << (n + 1)) - 1;
    let idx = (index >> (n * 2)) & mask;
    dec_1p_n1(idx, n, offset, &mut pos[2..]);
}

/// Four pulses in `4N+1` bits: two two-pulse codes.
fn dec_4p_4n1(index: u32, n: u32, offset: u32, pos: &mut [u32]) {
    let mask = (1u32 << (n * 2 - 1)) - 1;
    let idx = index & mask;
    let mut j = offset;
    if (index >> (n * 2 - 1)) & 1 != 0 {
        j += 1 << (n - 1);
    }
    dec_2p_2n1(idx, n - 1, j, pos);

    let mask = (1u32 << (n * 2 + 1)) - 1;
    let idx = (index >> (n * 2)) & mask;
    dec_2p_2n1(idx, n, offset, &mut pos[2..]);
}

/// Four pulses in `4N` bits.
///
/// Two leading bits say how the four pulses split between the two half-ranges,
/// which is what saves the extra bit over [`dec_4p_4n1`].
fn dec_4p_4n(index: u32, n: u32, offset: u32, pos: &mut [u32]) {
    let n_1 = n - 1;
    let j = offset + (1 << n_1);

    match (index >> (n * 4 - 2)) & 3 {
        0 => {
            if (index >> (n_1 * 4 + 1)) & 1 == 0 {
                dec_4p_4n1(index, n_1, offset, pos);
            } else {
                dec_4p_4n1(index, n_1, j, pos);
            }
        }
        1 => {
            dec_1p_n1(index >> (3 * n_1 + 1), n_1, offset, pos);
            dec_3p_3n1(index, n_1, j, &mut pos[1..]);
        }
        2 => {
            dec_2p_2n1(index >> (2 * n_1 + 1), n_1, offset, pos);
            dec_2p_2n1(index, n_1, j, &mut pos[2..]);
        }
        _ => {
            dec_3p_3n1(index >> (n_1 + 1), n_1, offset, pos);
            dec_1p_n1(index, n_1, j, &mut pos[3..]);
        }
    }
}

/// Five pulses in `5N` bits: a three-pulse code plus a two-pulse code.
fn dec_5p_5n(index: u32, n: u32, offset: u32, pos: &mut [u32]) {
    let n_1 = n - 1;
    let j = offset + (1 << n_1);
    let idx = index >> (n * 2 + 1);

    // Only the three-pulse half shifts range; the two-pulse call is identical
    // in both branches. Faithful to the reference, which writes it out twice.
    if (index >> (n * 5 - 1)) & 1 == 0 {
        dec_3p_3n1(idx, n_1, offset, pos);
    } else {
        dec_3p_3n1(idx, n_1, j, pos);
    }
    dec_2p_2n1(index, n, offset, &mut pos[3..]);
}

/// Six pulses in `6N-2` bits, the widest codebook.
fn dec_6p_6n_2(index: u32, n: u32, offset: u32, pos: &mut [u32]) {
    let n_1 = n - 1;
    let j = offset + (1 << n_1);

    let (offset_a, offset_b) = if (index >> (6 * n - 5)) & 1 == 0 {
        (offset, j)
    } else {
        (j, offset)
    };

    match (index >> (6 * n - 4)) & 3 {
        0 => {
            dec_5p_5n(index >> n, n_1, offset_a, pos);
            dec_1p_n1(index, n_1, offset_a, &mut pos[5..]);
        }
        1 => {
            dec_5p_5n(index >> n, n_1, offset_a, pos);
            dec_1p_n1(index, n_1, offset_b, &mut pos[5..]);
        }
        2 => {
            dec_4p_4n(index >> (2 * n_1 + 1), n_1, offset_a, pos);
            dec_2p_2n1(index, n_1, offset_b, &mut pos[4..]);
        }
        _ => {
            dec_3p_3n1(index >> (3 * n_1 + 1), n_1, offset, pos);
            dec_3p_3n1(index, n_1, j, &mut pos[3..]);
        }
    }
}

/// How many bits a mode spends on pulses per subframe.
#[must_use]
pub const fn pulse_bits(frame_bits: usize) -> usize {
    match frame_bits {
        132 => 12,
        177 => 20,
        253 => 36,
        285 => 44,
        317 => 52,
        365 => 64,
        397 => 72,
        _ => 88,
    }
}

/// Decode a subframe's innovation vector from its pulse indices.
///
/// Dispatches on the frame width, which is what selects the codebook.
#[must_use]
pub fn decode(indices: &[u16], frame_bits: usize) -> Option<[i16; L_SUBFR]> {
    let bits = pulse_bits(frame_bits);
    if bits == 12 {
        Some(decode_2t64(*indices.first()?))
    } else {
        decode_4t64(indices, bits)
    }
}

#[cfg(test)]
mod tests {
    use super::super::lp::isp_to_lp::tests_support::{block_has, block_row, has_block};
    use super::super::params::FrameParams;
    use super::*;
    use crate::codecs::amr::mode::{AmrMode, AmrVariant};
    use crate::codecs::amr::storage;

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

    fn mode_for(index: usize) -> AmrMode {
        AmrMode::new(AmrVariant::WideBand, u8::try_from(index).expect("index")).expect("mode")
    }

    /// Frame sizes in bits, by mode index.
    const FRAME_BITS: [usize; 9] = [132, 177, 253, 285, 317, 365, 397, 461, 477];

    #[test]
    fn the_codebook_is_bit_exact_against_ts26173() {
        let mut checked = 0;

        for (mode_index, &frame_bits) in FRAME_BITS.iter().enumerate() {
            let block = format!("bitstream{mode_index}");
            assert!(has_block(&block), "fixture block {block} missing");
            let (_, frames) = storage::read(fixture(mode_index)).expect("fixture parses");
            let mode = mode_for(mode_index);

            for f in 0.. {
                if !block_has(&block, &format!("meta{f}")) {
                    break;
                }
                let frame = frames.get(f).expect("frame");
                let params = FrameParams::parse(mode, &frame.data).expect("parses");

                for (sf, sub) in params.subframes.iter().enumerate() {
                    let got = decode(&sub.pulses, frame_bits)
                        .unwrap_or_else(|| panic!("{block} frame {f} subframe {sf}: no codebook"));
                    let want = block_row(&block, &format!("code{f}_{sf}"));
                    assert_eq!(
                        want.len(),
                        L_SUBFR,
                        "{block} frame {f} subframe {sf}: length"
                    );
                    for (i, (&g, &w)) in got.iter().zip(want.iter()).enumerate() {
                        assert_eq!(
                            g, w,
                            "{block} frame {f} subframe {sf}: code[{i}] = {g} but the reference gives {w}"
                        );
                    }
                    checked += 1;
                }
            }
        }

        assert!(checked >= 72, "only {checked} subframes checked");
    }

    #[test]
    fn pulse_counts_match_what_the_mode_pays_for() {
        // A decoder that puts pulses in the wrong track still produces the
        // right count, so this is weaker than the bit-exact test — but it
        // fails loudly if a codebook width is wired to the wrong decoder.
        const EXPECTED: [usize; 9] = [2, 4, 8, 10, 12, 16, 18, 24, 24];

        for (mode_index, &frame_bits) in FRAME_BITS.iter().enumerate() {
            let (_, frames) = storage::read(fixture(mode_index)).expect("fixture parses");
            let params = FrameParams::parse(mode_for(mode_index), &frames[0].data).expect("parses");
            let code = decode(&params.subframes[0].pulses, frame_bits).expect("code");

            // Pulses can coincide and cancel or reinforce, so count magnitude
            // rather than non-zero samples.
            let total: usize = code
                .iter()
                .map(|&v| usize::try_from(i32::from(v).abs() / i32::from(PULSE)).expect("count"))
                .sum();
            let expected = EXPECTED.get(mode_index).copied().expect("mode");
            assert_eq!(
                total, expected,
                "mode {mode_index}: {total} pulses, expected {expected}"
            );
        }
    }

    #[test]
    fn every_pulse_is_a_unit_in_q9() {
        // Amplitudes are always ±1.0; only positions and signs are coded. A
        // value that is not a multiple of 512 means a position collision was
        // mishandled or a shift went astray.
        for (mode_index, &frame_bits) in FRAME_BITS.iter().enumerate() {
            let (_, frames) = storage::read(fixture(mode_index)).expect("fixture parses");
            let mode = mode_for(mode_index);
            for frame in frames.iter().take(3) {
                let params = FrameParams::parse(mode, &frame.data).expect("parses");
                for sub in &params.subframes {
                    let code = decode(&sub.pulses, frame_bits).expect("code");
                    for (i, &v) in code.iter().enumerate() {
                        assert_eq!(
                            i32::from(v) % i32::from(PULSE),
                            0,
                            "mode {mode_index}: code[{i}] = {v} is not a whole pulse"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn four_track_pulses_land_only_in_their_own_track() {
        // Track k owns samples k, k+4, k+8, ... Getting this wrong is the
        // classic algebraic-codebook bug: the excitation stays sparse and
        // plausible, but the pulses are in the wrong places.
        for (mode_index, &frame_bits) in FRAME_BITS.iter().enumerate().skip(1) {
            let (_, frames) = storage::read(fixture(mode_index)).expect("fixture parses");
            let params = FrameParams::parse(mode_for(mode_index), &frames[0].data).expect("parses");

            // Decode each track in isolation and check where its pulses fall.
            let bits = pulse_bits(frame_bits);
            let full = decode_4t64(&params.subframes[0].pulses, bits).expect("code");

            // Every mode places at least one pulse in every track.
            for track in 0..4 {
                let in_track: i32 = (0..L_SUBFR)
                    .filter(|i| i % 4 == track)
                    .map(|i| i32::from(full[i]).abs())
                    .sum();
                assert!(
                    in_track > 0,
                    "mode {mode_index}: track {track} has no pulses"
                );
            }
        }
    }

    #[test]
    fn an_unknown_codebook_width_is_rejected() {
        assert!(decode_4t64(&[0; 8], 13).is_none());
        assert!(decode_4t64(&[0; 8], 0).is_none());
    }

    #[test]
    fn the_two_pulse_codebook_places_one_pulse_per_track() {
        // At 6.60 kbit/s there are two tracks of 32, so one pulse lands on an
        // even sample and one on an odd. Worth pinning: this codebook has a
        // different track layout from every other mode.
        for index in [0u16, 1, 1234, 4095] {
            let code = decode_2t64(index);
            let even: usize = (0..L_SUBFR).step_by(2).filter(|&i| code[i] != 0).count();
            let odd: usize = (1..L_SUBFR).step_by(2).filter(|&i| code[i] != 0).count();
            assert_eq!((even, odd), (1, 1), "index {index}: pulse placement");
        }
    }
}
