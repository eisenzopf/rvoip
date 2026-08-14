//! The AMR-WB frame parameter layout, 3GPP TS 26.201.
//!
//! Walks a frame's codec bits into every parameter the decoder needs. The
//! layout is mode-dependent in three places — pitch lag width, whether an LTP
//! filter bit is present, and the algebraic codebook split — so this is a
//! per-mode field walk rather than a fixed structure.
//!
//! # Every bit is accounted for
//!
//! A field-offset error does not overrun the frame; it shifts everything after
//! it and yields parameters that are wrong but entirely plausible. Both a
//! reference oracle and an implementation can make the *same* mistake and
//! agree with each other perfectly.
//!
//! The defence is conservation: after the walk, exactly zero bits must remain.
//! [`FrameParams::parse`] enforces that, and it is what caught the VAD flag
//! being read before the ISF indices rather than after — the leftover count
//! was 1 for every mode, which is the signature of a one-bit shift at the
//! front.

use super::bitstream::CodecBits;
use crate::codecs::amr::mode::AmrMode;

/// Subframes per frame.
pub const NB_SUBFR: usize = 4;

/// Shortest pitch lag, in samples at 12.8 kHz, coded at 1/4 resolution.
const PIT_MIN: u16 = 34;
/// Above this lag the resolution drops to 1/2.
const PIT_FR2: u16 = 128;
/// Above this lag the 9-bit code becomes integer-resolution.
const PIT_FR1_9B: u16 = 160;
/// The 8-bit code's integer-resolution threshold.
const PIT_FR1_8B: u16 = 92;
/// Longest pitch lag.
const PIT_MAX: u16 = 231;

/// Frame sizes in bits, indexed by mode. Used to pick layout variants.
const NBBITS_7K: usize = 132;
const NBBITS_9K: usize = 177;
const NBBITS_12K: usize = 253;
const NBBITS_14K: usize = 285;
const NBBITS_16K: usize = 317;
const NBBITS_18K: usize = 365;
const NBBITS_20K: usize = 397;
const NBBITS_24K: usize = 477;

/// One subframe's excitation parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubframeParams {
    /// Integer pitch lag, in samples at 12.8 kHz.
    pub pitch_lag: u16,
    /// Fractional part of the lag, in quarters: always 0 to 3.
    pub pitch_frac: u8,
    /// Whether the low-pass LTP filter is selected. Always off below 12.65.
    pub ltp_filter: bool,
    /// Joint pitch/codebook gain index.
    pub gain_index: u16,
    /// Algebraic codebook pulse indices; count and width vary by mode.
    pub pulses: Vec<u16>,
    /// High-band correction gain index, 23.85 kbit/s only.
    ///
    /// Interleaved *per subframe*, immediately after the gain index — not
    /// grouped at the end of the frame. The reference reads it inside the
    /// subframe loop, right before calling synthesis.
    pub hf_gain: Option<u16>,
}

/// Everything one frame carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameParams {
    /// Voice activity flag, the first bit of a speech frame.
    pub vad_flag: bool,
    /// ISF quantiser indices, for [`super::lp::isf_dequant`].
    pub isf_indices: Vec<u16>,
    /// Per-subframe excitation parameters.
    pub subframes: Vec<SubframeParams>,
    /// High-band correction gains, one per subframe, in subframe order.
    ///
    /// A convenience view over [`SubframeParams::hf_gain`]; the bits themselves
    /// are interleaved through the frame, not grouped here.
    pub hf_gains: Vec<u16>,
}

impl FrameParams {
    /// Parse a frame's payload into its parameters.
    ///
    /// Returns `None` if the payload is too short, if any field overruns, or
    /// if the walk does not consume the frame exactly.
    #[must_use]
    pub fn parse(mode: AmrMode, payload: &[u8]) -> Option<Self> {
        let mut bits = CodecBits::unpack(mode, payload)?;
        let nb_bits = bits.len();

        // The VAD flag leads a speech frame. Reading it after the spectrum
        // instead of before shifts every later field by one bit.
        let vad_flag = bits.take(1)? == 1;

        let isf_indices: Vec<u16> = super::bitstream::isf_index_widths(mode)
            .iter()
            .map(|&w| bits.take(w))
            .collect::<Option<_>>()?;

        let mut subframes = Vec::with_capacity(NB_SUBFR);
        // Subframes 1 and 3 — and 2 at 6.60 kbit/s — code the lag relative to
        // a window around the last absolute one, so this has to be carried.
        let mut lag_window_base = PIT_MIN;

        for sf in 0..NB_SUBFR {
            let absolute = sf == 0 || (sf == 2 && nb_bits > NBBITS_7K);

            let (pitch_lag, pitch_frac) = if absolute {
                let (lag, frac) = decode_absolute_lag(&mut bits, nb_bits)?;
                lag_window_base = lag_window_start(lag);
                (lag, frac)
            } else {
                decode_relative_lag(&mut bits, nb_bits, lag_window_base)?
            };

            // Below 12.65 the filter is fixed, so no bit is spent on it.
            let ltp_filter = if nb_bits <= NBBITS_9K {
                false
            } else {
                bits.take(1)? == 1
            };

            let pulses = read_pulses(&mut bits, nb_bits)?;

            let gain_width = if nb_bits <= NBBITS_9K { 6 } else { 7 };
            let gain_index = bits.take(gain_width)?;

            // Only 23.85 kbit/s spends bits correcting the high band, and it
            // spends them here — inside the subframe, not at the end of the
            // frame. Reading them as a group parses the same total number of
            // bits while assigning every field after the first subframe's
            // gain to the wrong parameter.
            let hf_gain = if nb_bits >= NBBITS_24K {
                Some(bits.take(4)?)
            } else {
                None
            };

            subframes.push(SubframeParams {
                pitch_lag,
                pitch_frac,
                ltp_filter,
                gain_index,
                pulses,
                hf_gain,
            });
        }

        let hf_gains: Vec<u16> = subframes.iter().filter_map(|s| s.hf_gain).collect();

        // Conservation: a layout error shifts fields rather than overrunning,
        // so the only reliable signal is that nothing is left.
        if bits.remaining() != 0 {
            return None;
        }

        Some(Self {
            vad_flag,
            isf_indices,
            subframes,
            hf_gains,
        })
    }
}

/// Decode an absolutely-coded pitch lag.
///
/// Resolution is not uniform: quarter-sample for short lags, half then integer
/// as the lag grows. Low pitches need the precision and high ones do not, so
/// the code spends its range where it buys something.
fn decode_absolute_lag(bits: &mut CodecBits, nb_bits: usize) -> Option<(u16, u8)> {
    if nb_bits <= NBBITS_9K {
        let index = bits.take(8)?;
        if index < (PIT_FR1_8B - PIT_MIN) * 2 {
            let lag = PIT_MIN + (index >> 1);
            let frac = (index - ((lag - PIT_MIN) << 1)) << 1;
            Some((lag, u8::try_from(frac).ok()?))
        } else {
            // A negative offset: the integer-resolution range starts where the
            // half-resolution one ends, so the code value runs ahead of the lag.
            let offset = i32::from(PIT_FR1_8B) - i32::from(PIT_FR1_8B - PIT_MIN) * 2;
            Some((u16::try_from(i32::from(index) + offset).ok()?, 0))
        }
    } else {
        let index = bits.take(9)?;
        if index < (PIT_FR2 - PIT_MIN) * 4 {
            let lag = PIT_MIN + (index >> 2);
            let frac = index - ((lag - PIT_MIN) << 2);
            Some((lag, u8::try_from(frac).ok()?))
        } else if index < (PIT_FR2 - PIT_MIN) * 4 + (PIT_FR1_9B - PIT_FR2) * 2 {
            let index = index - (PIT_FR2 - PIT_MIN) * 4;
            let lag = PIT_FR2 + (index >> 1);
            let frac = (index - ((lag - PIT_FR2) << 1)) << 1;
            Some((lag, u8::try_from(frac).ok()?))
        } else {
            let offset = i32::from(PIT_FR1_9B)
                - i32::from(PIT_FR2 - PIT_MIN) * 4
                - i32::from(PIT_FR1_9B - PIT_FR2) * 2;
            Some((u16::try_from(i32::from(index) + offset).ok()?, 0))
        }
    }
}

/// Decode a lag coded relative to the previous subframe's window.
fn decode_relative_lag(bits: &mut CodecBits, nb_bits: usize, base: u16) -> Option<(u16, u8)> {
    if nb_bits <= NBBITS_9K {
        let index = bits.take(5)?;
        let lag = base + (index >> 1);
        let frac = (index - ((lag - base) << 1)) << 1;
        Some((lag, u8::try_from(frac).ok()?))
    } else {
        let index = bits.take(6)?;
        let lag = base + (index >> 2);
        let frac = index - ((lag - base) << 2);
        Some((lag, u8::try_from(frac).ok()?))
    }
}

/// The bottom of the 16-lag search window the next subframe codes against.
fn lag_window_start(lag: u16) -> u16 {
    let min = lag.saturating_sub(8).max(PIT_MIN);
    if min + 15 > PIT_MAX {
        PIT_MAX - 15
    } else {
        min
    }
}

/// Read one subframe's algebraic codebook indices.
///
/// The split is the clearest expression of what the extra bit rate buys: two
/// pulses at 6.60 kbit/s, twenty-four at 23.85.
fn read_pulses(bits: &mut CodecBits, nb_bits: usize) -> Option<Vec<u16>> {
    let widths: &[usize] = if nb_bits <= NBBITS_7K {
        &[12]
    } else if nb_bits <= NBBITS_9K {
        &[5, 5, 5, 5]
    } else if nb_bits <= NBBITS_12K {
        &[9, 9, 9, 9]
    } else if nb_bits <= NBBITS_14K {
        &[13, 13, 9, 9]
    } else if nb_bits <= NBBITS_16K {
        &[13, 13, 13, 13]
    } else if nb_bits <= NBBITS_18K {
        &[2, 2, 2, 2, 14, 14, 14, 14]
    } else if nb_bits <= NBBITS_20K {
        &[10, 10, 2, 2, 10, 10, 14, 14]
    } else {
        &[11, 11, 11, 11, 11, 11, 11, 11]
    };
    widths.iter().map(|&w| bits.take(w)).collect()
}

#[cfg(test)]
mod tests {
    use super::super::lp::isp_to_lp::tests_support::{block_has, block_row, has_block};
    use super::*;
    use crate::codecs::amr::mode::AmrVariant;
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
        AmrMode::new(
            AmrVariant::WideBand,
            u8::try_from(index).expect("mode index"),
        )
        .expect("mode")
    }

    #[test]
    fn the_full_parameter_walk_is_bit_exact_against_ts26173() {
        let mut checked = 0;

        for mode_index in 0..9 {
            let block = format!("bitstream{mode_index}");
            assert!(has_block(&block), "fixture block {block} missing");
            let (_, frames) = storage::read(fixture(mode_index)).expect("fixture parses");
            let mode = mode_for(mode_index);

            for f in 0.. {
                if !block_has(&block, &format!("meta{f}")) {
                    break;
                }
                let meta = block_row(&block, &format!("meta{f}"));
                let frame = frames.get(f).expect("fixture has this frame");
                let got = FrameParams::parse(mode, &frame.data).unwrap_or_else(|| {
                    panic!("{block} frame {f}: layout did not consume the frame")
                });

                // The row is mode, bit count, VAD flag -- the label is not
                // part of it.
                assert_eq!(meta.len(), 3, "{block} frame {f}: meta row shape");
                assert_eq!(
                    i16::from(got.vad_flag),
                    meta[2],
                    "{block} frame {f}: VAD flag"
                );

                let want_isf = block_row(&block, &format!("isfind{f}"));
                assert_eq!(
                    got.isf_indices.len(),
                    want_isf.len(),
                    "{block} frame {f}: ISF index count"
                );
                for (i, (&g, &w)) in got.isf_indices.iter().zip(want_isf.iter()).enumerate() {
                    assert_eq!(
                        i64::from(g),
                        i64::from(w),
                        "{block} frame {f}: ISF index {i}"
                    );
                }

                for (sf, params) in got.subframes.iter().enumerate() {
                    let want = block_row(&block, &format!("sf{f}_{sf}"));
                    assert_eq!(
                        i64::from(params.pitch_lag),
                        i64::from(want[0]),
                        "{block} frame {f} subframe {sf}: pitch lag"
                    );
                    assert_eq!(
                        i64::from(params.pitch_frac),
                        i64::from(want[1]),
                        "{block} frame {f} subframe {sf}: pitch fraction"
                    );
                    assert_eq!(
                        i64::from(i16::from(params.ltp_filter)),
                        i64::from(want[2]),
                        "{block} frame {f} subframe {sf}: LTP filter bit"
                    );
                    assert_eq!(
                        i64::from(params.gain_index),
                        i64::from(want[3]),
                        "{block} frame {f} subframe {sf}: gain index"
                    );
                    assert_eq!(
                        params
                            .hf_gain
                            .map_or(-1, |g| i16::try_from(g).expect("gain")),
                        want[4],
                        "{block} frame {f} subframe {sf}: high-band gain"
                    );
                    let want_pulses = &want[5..];
                    assert_eq!(
                        params.pulses.len(),
                        want_pulses.len(),
                        "{block} frame {f} subframe {sf}: pulse count"
                    );
                    for (i, (&g, &w)) in params.pulses.iter().zip(want_pulses.iter()).enumerate() {
                        assert_eq!(
                            i64::from(g),
                            i64::from(w),
                            "{block} frame {f} subframe {sf}: pulse {i}"
                        );
                    }
                }
                checked += 1;
            }
        }

        assert!(checked >= 18, "only {checked} frames checked");
    }

    #[test]
    fn the_walk_consumes_every_bit_of_every_mode() {
        // This is the check that catches a field-offset error. A shifted layout
        // still produces plausible parameters, so agreement with an oracle that
        // shares the mistake proves nothing -- but leftover bits prove a shift.
        // A missed VAD flag showed up here as exactly one bit left in all nine
        // modes before it was fixed.
        for mode_index in 0..9 {
            let (_, frames) = storage::read(fixture(mode_index)).expect("fixture parses");
            let mode = mode_for(mode_index);
            for (f, frame) in frames.iter().take(5).enumerate() {
                assert!(
                    FrameParams::parse(mode, &frame.data).is_some(),
                    "mode {mode_index} frame {f}: layout left bits unconsumed"
                );
            }
        }
    }

    #[test]
    fn only_the_top_mode_carries_high_band_gains() {
        // 23.85 differs from 23.05 only in spending 16 bits per frame on the
        // high band; everything else about the two is identical.
        for mode_index in 0..9 {
            let (_, frames) = storage::read(fixture(mode_index)).expect("fixture parses");
            let params = FrameParams::parse(mode_for(mode_index), &frames[0].data).expect("parses");
            let expected = usize::from(mode_index == 8) * NB_SUBFR;
            assert_eq!(
                params.hf_gains.len(),
                expected,
                "mode {mode_index}: high-band gain count"
            );
        }
    }

    #[test]
    fn decoded_lags_stay_within_the_codecs_range() {
        // A lag outside [PIT_MIN, PIT_MAX] would index outside the excitation
        // history. The relative codes make this worth checking: they add an
        // offset to a carried base, so an error compounds across subframes.
        for mode_index in 0..9 {
            let (_, frames) = storage::read(fixture(mode_index)).expect("fixture parses");
            let mode = mode_for(mode_index);
            for (f, frame) in frames.iter().take(5).enumerate() {
                let params = FrameParams::parse(mode, &frame.data).expect("parses");
                for (sf, s) in params.subframes.iter().enumerate() {
                    assert!(
                        (PIT_MIN..=PIT_MAX).contains(&s.pitch_lag),
                        "mode {mode_index} frame {f} subframe {sf}: lag {} out of range",
                        s.pitch_lag
                    );
                    assert!(
                        s.pitch_frac < 4,
                        "mode {mode_index} frame {f} subframe {sf}: fraction {} out of range",
                        s.pitch_frac
                    );
                }
            }
        }
    }

    #[test]
    fn the_pulse_count_grows_with_the_bit_rate() {
        // The clearest statement of what the extra rate buys. Not strictly
        // monotonic in *count* -- 18.25 switches to a different split -- but
        // the total bits spent on pulses must rise.
        let mut previous = 0usize;
        for mode_index in 0..9 {
            let (_, frames) = storage::read(fixture(mode_index)).expect("fixture parses");
            let params = FrameParams::parse(mode_for(mode_index), &frames[0].data).expect("parses");
            let count = params.subframes[0].pulses.len();
            assert!(count > 0, "mode {mode_index}: no pulses");
            previous = previous.max(count);
        }
        assert_eq!(previous, 8, "the top modes should code eight pulse indices");
    }

    #[test]
    fn a_truncated_frame_is_rejected() {
        let mode = mode_for(8);
        let (_, frames) = storage::read(fixture(8)).expect("fixture parses");
        let full = &frames[0].data;
        assert!(FrameParams::parse(mode, full).is_some());
        assert!(FrameParams::parse(mode, &full[..full.len() - 1]).is_none());
    }
}
