//! Low-band synthesis, 3GPP TS 26.190 §6.6–§6.8, in fixed point.
//!
//! Excitation in, 12.8 kHz speech out. Three filters, each carrying state
//! across subframes:
//!
//! 1. **LP synthesis** — run the excitation through `1/A(z)`, which puts back
//!    the spectral envelope the encoder divided out.
//! 2. **De-emphasis** — undo the encoder's fixed pre-emphasis.
//! 3. **50 Hz high-pass** — remove the DC and rumble that the synthesis filter
//!    can introduce at very low frequencies, where the LP model is unreliable.
//!
//! # Double-precision synthesis
//!
//! The synthesis filter keeps its output as a `(high, low)` pair rather than a
//! single `Word16`, and feeds both halves back. That is not a rounding
//! refinement: `1/A(z)` is recursive and marginally stable by construction, so
//! rounding error at each step is fed back and accumulates. Sixteen bits of
//! state is not enough to keep a sharp formant from drifting.
//!
//! # Adaptive excitation scaling
//!
//! The excitation is carried with a per-subframe shift, `q_new`, chosen so the
//! loudest sample uses the available headroom. The filter compensates by
//! scaling `a[0]` rather than the signal, so no precision is lost undoing it.

use super::codebook::L_SUBFR;
use crate::fixed_point::arith::{extract_h, extract_l, round};
use crate::fixed_point::arith32::{l_deposit_h, l_mac, l_msu};
use crate::fixed_point::oper32::l_extract;
use crate::fixed_point::shift::{l_shl, l_shr, norm_s, shr};
use crate::fixed_point::types::{DspContext, Word16, Word32};

/// Order of the low-band LP filter.
pub const M: usize = 16;

/// Pre-emphasis factor, 0.68 in Q15. De-emphasis undoes exactly this.
pub const PREEMPH_FAC: Word16 = Word16(22282);

/// 50 Hz high-pass numerator, Q12.
const HP_B: [Word16; 3] = [Word16(4053), Word16(-8106), Word16(4053)];

/// 50 Hz high-pass denominator, Q12 and doubled.
const HP_A: [Word16; 3] = [Word16(8192), Word16(16211), Word16(-8021)];

/// The synthesis filter's carried state: `M` past outputs, in both halves.
#[derive(Debug, Clone)]
pub struct SynthesisFilter {
    high: [Word16; M],
    low: [Word16; M],
}

impl Default for SynthesisFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl SynthesisFilter {
    /// A filter with no history.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            high: [Word16(0); M],
            low: [Word16(0); M],
        }
    }

    /// Run one subframe of excitation through `1/A(z)`.
    ///
    /// `a` is `a[0..=M]` in Q12, `q_new` the excitation's scaling shift.
    /// Returns the synthesis as `(high, low)` halves, which
    /// [`deemphasis`] recombines.
    pub fn filter(
        &mut self,
        a: &[Word16; M + 1],
        excitation: &[Word16; L_SUBFR],
        q_new: i16,
    ) -> ([Word16; L_SUBFR], [Word16; L_SUBFR]) {
        let mut ctx = DspContext::default();

        // Headroom in a[0], less two bits for the accumulation.
        let s = norm_s(a[0]) - 2;
        // Fold the excitation's scaling into a[0] rather than shifting the
        // signal, so undoing it costs no precision.
        let a0 = shr(&mut ctx, a[0], 4 + q_new);

        // One buffer with the history in front, so negative indices are just
        // offsets and the recursion reads its own output naturally.
        let mut high = [Word16(0); M + L_SUBFR];
        let mut low = [Word16(0); M + L_SUBFR];
        high[..M].copy_from_slice(&self.high);
        low[..M].copy_from_slice(&self.low);

        for (i, &sample) in excitation.iter().enumerate() {
            let n = M + i;

            // The low half first, in its own scale, then shifted to join the
            // high half's. Splitting the two keeps the feedback exact.
            let mut acc = Word32(0);
            for j in 1..=M {
                acc = l_msu(&mut ctx, acc, low[n - j], a[j]);
            }
            acc = l_shr(&mut ctx, acc, 16 - 4);

            acc = l_mac(&mut ctx, acc, sample, a0);
            for j in 1..=M {
                acc = l_msu(&mut ctx, acc, high[n - j], a[j]);
            }

            let scaled = l_shl(&mut ctx, acc, 3 + s);
            high[n] = extract_h(scaled);
            // The low half is what the high half's 16 bits could not hold.
            let residue = l_shr(&mut ctx, scaled, 4);
            low[n] = extract_l(l_msu(&mut ctx, residue, high[n], Word16(2048)));
        }

        self.high.copy_from_slice(&high[L_SUBFR..]);
        self.low.copy_from_slice(&low[L_SUBFR..]);

        let mut out_hi = [Word16(0); L_SUBFR];
        let mut out_lo = [Word16(0); L_SUBFR];
        out_hi.copy_from_slice(&high[M..]);
        out_lo.copy_from_slice(&low[M..]);
        (out_hi, out_lo)
    }
}

/// Undo the encoder's pre-emphasis, recombining the two synthesis halves.
///
/// `memory` is the previous output sample, carried across subframes.
#[must_use]
pub fn deemphasis(
    high: &[Word16; L_SUBFR],
    low: &[Word16; L_SUBFR],
    mu: Word16,
    memory: &mut Word16,
) -> [Word16; L_SUBFR] {
    let mut ctx = DspContext::default();
    let mut out = [Word16(0); L_SUBFR];

    // Q15 to Q14: the recursion's own output is fed back at half weight to
    // leave room for the gain the filter applies at low frequencies.
    let fac = shr(&mut ctx, mu, 1);

    for i in 0..L_SUBFR {
        let mut acc = l_deposit_h(high[i]);
        acc = l_mac(&mut ctx, acc, low[i], Word16(8));
        acc = l_shl(&mut ctx, acc, 3);
        let previous = if i == 0 { *memory } else { out[i - 1] };
        acc = l_mac(&mut ctx, acc, previous, fac);
        // Saturation here is expected: de-emphasis has gain, and clipping is
        // preferable to wrapping.
        let acc = l_shl(&mut ctx, acc, 1);
        out[i] = round(&mut ctx, acc);
    }

    *memory = out[L_SUBFR - 1];
    out
}

/// State of the 50 Hz high-pass: two past outputs in double precision, and two
/// past inputs.
#[derive(Debug, Clone, Default)]
pub struct HighPass50 {
    y1_hi: Word16,
    y1_lo: Word16,
    y2_hi: Word16,
    y2_lo: Word16,
    x0: Word16,
    x1: Word16,
}

impl HighPass50 {
    /// A filter with no history.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            y1_hi: Word16(0),
            y1_lo: Word16(0),
            y2_hi: Word16(0),
            y2_lo: Word16(0),
            x0: Word16(0),
            x1: Word16(0),
        }
    }

    /// Filter one subframe in place.
    ///
    /// A biquad, but with the recursive part kept in double precision for the
    /// same reason as the synthesis filter: at 50 Hz against 12.8 kHz the poles
    /// sit very close to the unit circle, and single-precision feedback would
    /// drift.
    pub fn filter(&mut self, signal: &mut [Word16]) {
        let mut ctx = DspContext::default();

        for sample in signal.iter_mut() {
            let x2 = self.x1;
            self.x1 = self.x0;
            self.x0 = *sample;

            // Start at 0.5 in the low half's scale, so the shift below rounds
            // instead of truncating.
            let mut acc = Word32(16384);
            acc = l_mac(&mut ctx, acc, self.y1_lo, HP_A[1]);
            acc = l_mac(&mut ctx, acc, self.y2_lo, HP_A[2]);
            acc = l_shr(&mut ctx, acc, 15);
            acc = l_mac(&mut ctx, acc, self.y1_hi, HP_A[1]);
            acc = l_mac(&mut ctx, acc, self.y2_hi, HP_A[2]);
            acc = l_mac(&mut ctx, acc, self.x0, HP_B[0]);
            acc = l_mac(&mut ctx, acc, self.x1, HP_B[1]);
            acc = l_mac(&mut ctx, acc, x2, HP_B[2]);

            // Q12 coefficients, so lift the result to Q14.
            let acc = l_shl(&mut ctx, acc, 2);

            self.y2_hi = self.y1_hi;
            self.y2_lo = self.y1_lo;
            let (hi, lo) = l_extract(acc);
            self.y1_hi = hi;
            self.y1_lo = lo;

            let scaled = l_shl(&mut ctx, acc, 1);
            *sample = round(&mut ctx, scaled);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::lp::isp_to_lp::tests_support::{block_row, has_block};
    use super::*;

    /// The predictor the oracle filters with.
    const A_REAL: [i16; M + 1] = [
        4096, -3559, 1097, -175, -313, 292, -73, -119, 158, -83, -20, 72, -60, 12, 25, -31, 12,
    ];

    /// The oracle's deterministic excitation for one block.
    fn excitation(block: usize) -> [Word16; L_SUBFR] {
        let mut exc = [Word16(0); L_SUBFR];
        for (n, slot) in exc.iter_mut().enumerate() {
            let t = block * L_SUBFR + n;
            #[allow(clippy::cast_precision_loss)]
            let mut v = if t.is_multiple_of(53) { 6000.0 } else { 0.0 };
            #[allow(clippy::cast_precision_loss)]
            {
                v += 900.0 * (2.0 * std::f64::consts::PI * t as f64 / 41.0).sin();
            }
            #[allow(clippy::cast_possible_truncation)]
            {
                *slot = Word16(v as i16);
            }
        }
        exc
    }

    fn expect(label: &str, got: &[Word16], block: usize) {
        let want = block_row("synth", &format!("{label}{block}"));
        assert_eq!(want.len(), got.len(), "{label}{block}: length");
        for (i, (&g, &w)) in got.iter().zip(want.iter()).enumerate() {
            assert_eq!(
                g.0, w,
                "{label}{block}: sample {i} = {} but the reference gives {w}",
                g.0
            );
        }
    }

    #[test]
    fn the_low_band_chain_is_bit_exact_against_ts26173() {
        assert!(has_block("synth"), "fixture block synth missing");
        let a: [Word16; M + 1] = A_REAL.map(Word16);

        // Every stage carries state, so replay the sequence from reset.
        let mut filter = SynthesisFilter::new();
        let mut deemph_mem = Word16(0);
        let mut hp = HighPass50::new();

        for block in 0..4 {
            let exc = excitation(block);
            expect("exc", &exc, block);

            let (high, low) = filter.filter(&a, &exc, 0);
            expect("synhi", &high, block);
            expect("synlo", &low, block);

            let mut speech = deemphasis(&high, &low, PREEMPH_FAC, &mut deemph_mem);
            expect("deemph", &speech, block);

            hp.filter(&mut speech);
            expect("hp50_", &speech, block);
        }
    }

    #[test]
    fn the_synthesis_filter_carries_state_across_subframes() {
        // Filtering two blocks in sequence must differ from filtering the
        // second in isolation -- otherwise the recursion has no memory and
        // every subframe would start from silence, which clicks.
        let a: [Word16; M + 1] = A_REAL.map(Word16);

        let mut continuous = SynthesisFilter::new();
        continuous.filter(&a, &excitation(0), 0);
        let (carried, _) = continuous.filter(&a, &excitation(1), 0);

        let mut fresh = SynthesisFilter::new();
        let (isolated, _) = fresh.filter(&a, &excitation(1), 0);

        assert_ne!(
            carried, isolated,
            "the filter produced the same output with and without history"
        );
    }

    #[test]
    fn the_high_pass_removes_a_constant() {
        // A DC input must decay to nothing; that is what the filter is for.
        let mut hp = HighPass50::new();
        let mut last = 0i32;
        for _ in 0..20 {
            let mut block = [Word16(8000); L_SUBFR];
            hp.filter(&mut block);
            last = i32::from(block[L_SUBFR - 1].0);
        }
        assert!(
            last.abs() < 100,
            "DC survived the high-pass at {last} after 20 blocks"
        );
    }

    #[test]
    fn the_high_pass_passes_speech_frequencies() {
        // 500 Hz is well inside the passband and must come through largely
        // intact, or the filter is attenuating far more than 50 Hz worth.
        let mut hp = HighPass50::new();
        let mut peak = 0i32;
        for block in 0..8 {
            let mut samples = [Word16(0); L_SUBFR];
            for (n, slot) in samples.iter_mut().enumerate() {
                #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
                let t = (block * L_SUBFR + n) as f64;
                #[allow(clippy::cast_possible_truncation)]
                {
                    *slot = Word16(
                        (8000.0 * (2.0 * std::f64::consts::PI * 500.0 * t / 12800.0).sin()) as i16,
                    );
                }
            }
            hp.filter(&mut samples);
            // Skip the first blocks while the filter settles.
            if block >= 4 {
                peak = peak.max(samples.iter().map(|s| i32::from(s.0).abs()).max().unwrap_or(0));
            }
        }
        assert!(
            peak > 6000,
            "a 500 Hz tone came through at only {peak} of 8000"
        );
    }

    #[test]
    fn deemphasis_boosts_low_frequencies() {
        // It undoes a pre-emphasis, so it must have gain at DC. Feeding a
        // constant through should grow, not shrink.
        let mut memory = Word16(0);
        let high = [Word16(1000); L_SUBFR];
        let low = [Word16(0); L_SUBFR];
        let out = deemphasis(&high, &low, PREEMPH_FAC, &mut memory);
        assert!(
            i32::from(out[L_SUBFR - 1].0) > i32::from(out[0].0),
            "de-emphasis did not accumulate a constant: {} then {}",
            out[0].0,
            out[L_SUBFR - 1].0
        );
    }
}
