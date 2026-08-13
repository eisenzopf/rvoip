//! Voice activity detection for AMR-NB, TS 26.073 `vad1.c`.
//!
//! # Why this is the hardest thing here to test
//!
//! The wideband detector's output is codec bit 0 of every frame, so a wrong
//! decision shows up immediately as a byte difference. **This one appears
//! nowhere in the bitstream.** Its only observable is which frames the encoder
//! turns into SID or `NO_DATA`, and that is filtered through a seven-frame
//! hangover — so a detector that is wrong in the middle of a talk spurt is
//! completely invisible in the output.
//!
//! The tests therefore compare the *whole state* against the reference every
//! frame: the decision registers, the counters and all nine noise estimates.
//! A test that compared only the boolean would agree with a constant roughly
//! 85% of the time.
//!
//! # VAD1, not VAD2
//!
//! The makefile's default and the detector the conformance vectors were made
//! with. The two are not interchangeable: under VAD2 the open-loop pitch stage
//! accumulates different quantities and the tone detector is replaced
//! entirely, so each is its own bit-exact module.
//! `tools/build-amr-dtx-fixtures.sh` asserts the committed fixture can tell
//! them apart — they choose different frame types on 21 of its 150 frames —
//! which is what makes this port qualifiable at all.
//!
//! VAD2 is no longer absent: it lives in [`super::vad2`], bit-exact against
//! the same reference. Neither replaces the other; the encoder selects one.
//!
//! # The three registers
//!
//! `vadreg`, `pitch` and `tone` each hold fifteen flags, newest in bit 14,
//! shifted right by one as each frame arrives. Most of the decision logic is
//! masks over those histories: "the last eight decisions were all zero" is
//! `vadreg & 0x7f80 == 0`, and so on. They are `i16` rather than a bitset
//! because the reference's shifts are arithmetic and the masks are its own.

use crate::fixed_point::arith::{abs_s, add, extract_h, mult, mult_r, round, sub};
use crate::fixed_point::arith32::{l_add, l_mac, l_msu, l_sub};
use crate::fixed_point::arith32::l_deposit_h;
use crate::fixed_point::div::div_s;
use crate::fixed_point::shift::{l_shl, norm_s, shl, shr};
use crate::fixed_point::types::{DspContext, Word16, Word32};

/// Samples per frame.
const FRAME_LEN: usize = 160;
/// Sub-bands the filter bank splits the frame into.
const COMPLEN: usize = 9;
/// `1/COMPLEN` in Q15.
const INV_COMPLEN: Word16 = Word16(3641);
/// Samples of history the power sum reaches back into.
pub const LOOKAHEAD: usize = 40;
/// `log2(MAX_16 / UNITY)`.
const UNIRSHFT: i16 = 6;

/// Pitch-gain threshold above which a tone is declared.
const TONE_THR: Word16 = Word16(21299);

/// Background-noise update coefficients, all `1 - x` in Q15.
const ALPHA_UP1: Word16 = Word16(1638);
const ALPHA_DOWN1: Word16 = Word16(2097);
const ALPHA_UP2: Word16 = Word16(491);
const ALPHA_DOWN2: Word16 = Word16(1867);
const ALPHA3: Word16 = Word16(1638);
const ALPHA4: Word16 = Word16(3276);
const ALPHA5: Word16 = Word16(16383);

/// Decision threshold at the quietest and loudest noise levels, and the slope
/// between them.
const VAD_THR_HIGH: Word16 = Word16(1260);
const VAD_THR_LOW: Word16 = Word16(720);
const VAD_P1: Word16 = Word16(0);
const VAD_SLOPE: Word16 = Word16(-2807);

/// Stationarity detection.
const STAT_COUNT: Word16 = Word16(20);
const CAD_MIN_STAT_COUNT: Word16 = Word16(5);
const STAT_THR_LEVEL: Word16 = Word16(184);
const STAT_THR: Word16 = Word16(1000);

/// Bounds and starting point for the per-band noise estimate.
const NOISE_MIN: Word16 = Word16(40);
const NOISE_MAX: Word16 = Word16(16000);
const NOISE_INIT: Word16 = Word16(150);

/// Above this noise level the hangover is shorter and triggers sooner.
const HANG_NOISE_THR: Word16 = Word16(100);
const BURST_LEN_HIGH_NOISE: Word16 = Word16(4);
const HANG_LEN_HIGH_NOISE: Word16 = Word16(7);
const BURST_LEN_LOW_NOISE: Word16 = Word16(5);
const HANG_LEN_LOW_NOISE: Word16 = Word16(4);

/// Frame power below which the decision is forced to silence.
const VAD_POW_LOW: Word32 = Word32(15000);
/// Below this, the pitch flag is not believed.
const POW_PITCH_THR: Word32 = Word32(343_040);
/// Below this, the previous frame's complex flag is cleared.
const POW_COMPLEX_THR: Word32 = Word32(15000);

/// Filter-bank coefficients.
const COEFF3: Word16 = Word16(13363);
const COEFF5_1: Word16 = Word16(21955);
const COEFF5_2: Word16 = Word16(6390);

/// How close two open-loop lags must be to count as the same pitch, and how
/// many such agreements make a voiced decision.
const LTHRESH: Word16 = Word16(4);
const NTHRESH: Word16 = Word16(4);

/// Thresholds on the long-term high-band correlation.
const CVAD_THRESH_ADAPT_HIGH: Word16 = Word16(19660);
const CVAD_THRESH_ADAPT_LOW: Word16 = Word16(16383);
const CVAD_THRESH_IN_NOISE: Word16 = Word16(21299);
const CVAD_THRESH_HANG: Word16 = Word16(22937);
const CVAD_HANG_LIMIT: Word16 = Word16(100);
const CVAD_HANG_LENGTH: Word16 = Word16(250);
const CVAD_LOWPOW_RESET: Word16 = Word16(13107);
const CVAD_MIN_CORR: Word16 = Word16(13107);
const CVAD_ADAPT_SLOW: Word16 = Word16(655);
const CVAD_ADAPT_FAST: Word16 = Word16(2621);
const CVAD_ADAPT_REALLY_FAST: Word16 = Word16(6553);

/// Band, then start, end, stride, offset and scale, highest band first —
/// the order the reference writes them in.
const BANDS: [(usize, usize, usize, usize, usize, i16); COMPLEN] = [
    (8, FRAME_LEN / 4 - 8, FRAME_LEN / 4, 4, 1, 15),
    (7, FRAME_LEN / 8 - 4, FRAME_LEN / 8, 8, 7, 16),
    (6, FRAME_LEN / 8 - 4, FRAME_LEN / 8, 8, 3, 16),
    (5, FRAME_LEN / 8 - 4, FRAME_LEN / 8, 8, 2, 16),
    (4, FRAME_LEN / 8 - 4, FRAME_LEN / 8, 8, 6, 16),
    (3, FRAME_LEN / 16 - 2, FRAME_LEN / 16, 16, 4, 16),
    (2, FRAME_LEN / 16 - 2, FRAME_LEN / 16, 16, 12, 16),
    (1, FRAME_LEN / 16 - 2, FRAME_LEN / 16, 16, 8, 16),
    (0, FRAME_LEN / 16 - 2, FRAME_LEN / 16, 16, 0, 16),
];

/// The detector's carried state — the reference's `vadState1`.
#[derive(Debug, Clone)]
pub struct VoiceActivityDetector {
    /// Per-band background noise estimate.
    bckr_est: [Word16; COMPLEN],
    /// Slow average of the band levels, for stationarity.
    ave_level: [Word16; COMPLEN],
    /// The previous frame's band levels.
    old_level: [Word16; COMPLEN],
    /// Level of the samples past the frame proper, carried forward.
    sub_level: [Word16; COMPLEN],
    /// Filter-bank memories.
    a_data5: [[Word16; 2]; 3],
    a_data3: [Word16; 5],

    burst_count: Word16,
    hang_count: Word16,
    stat_count: Word16,

    /// Fifteen flags each, newest in bit 14.
    vadreg: Word16,
    pitch: Word16,
    tone: Word16,
    complex_high: Word16,
    complex_low: Word16,

    oldlag_count: Word16,
    oldlag: Word16,

    complex_hang_count: Word16,
    complex_hang_timer: Word16,

    /// The open-loop stage's high-pass-filtered best correlation.
    best_corr_hp: Word16,
    /// Its long-term average.
    corr_hp_fast: Word16,

    speech_vad_decision: bool,
    complex_warning: bool,
}

impl Default for VoiceActivityDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl VoiceActivityDetector {
    /// A detector in its reset state, as `vad1_reset` leaves it.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            bckr_est: [NOISE_INIT; COMPLEN],
            ave_level: [NOISE_INIT; COMPLEN],
            old_level: [NOISE_INIT; COMPLEN],
            sub_level: [Word16(0); COMPLEN],
            a_data5: [[Word16(0); 2]; 3],
            a_data3: [Word16(0); 5],
            burst_count: Word16(0),
            hang_count: Word16(0),
            stat_count: Word16(0),
            vadreg: Word16(0),
            pitch: Word16(0),
            tone: Word16(0),
            complex_high: Word16(0),
            complex_low: Word16(0),
            oldlag_count: Word16(0),
            oldlag: Word16(0),
            complex_hang_count: Word16(0),
            complex_hang_timer: Word16(0),
            best_corr_hp: CVAD_LOWPOW_RESET,
            corr_hp_fast: CVAD_LOWPOW_RESET,
            speech_vad_decision: false,
            complex_warning: false,
        }
    }

    /// Decide whether one frame contains speech — `vad1`.
    ///
    /// `frame` is the 200 samples ending at this frame: forty of history then
    /// the 160 of the frame itself. The power sum reaches back into the
    /// history while the filter bank sees only the frame, which is why one
    /// buffer carries both rather than two arguments carrying one each.
    ///
    /// # Panics
    /// If `frame` is not exactly `LOOKAHEAD + FRAME_LEN` samples.
    pub fn process(&mut self, ctx: &mut DspContext, frame: &[Word16]) -> bool {
        assert_eq!(
            frame.len(),
            LOOKAHEAD + FRAME_LEN,
            "the detector needs forty samples of history before the frame"
        );
        let (history, current) = frame.split_at(LOOKAHEAD);

        // The power of the frame *offset by the lookahead*: the reference
        // indexes `in_buf[i - LOOKAHEAD]` for a whole frame, so this window
        // starts in the history and ends before the frame does.
        let mut power = Word32(0);
        for i in 0..FRAME_LEN {
            let sample = if i < LOOKAHEAD {
                history[i]
            } else {
                current[i - LOOKAHEAD]
            };
            power = l_mac(ctx, power, sample, sample);
        }

        // Too quiet to believe the pitch or the previous complex flag.
        if l_sub(ctx, power, POW_PITCH_THR).0 < 0 {
            self.pitch = Word16(self.pitch.0 & 0x3fff);
        }
        if l_sub(ctx, power, POW_COMPLEX_THR).0 < 0 {
            self.complex_low = Word16(self.complex_low.0 & 0x3fff);
        }

        let level = self.filter_bank(ctx, current);
        self.decide(ctx, &level, power)
    }

    /// Record the open-loop stage's high-pass correlation —
    /// `vad_complex_detection_update`.
    pub const fn observe_correlation(&mut self, best_corr_hp: Word16) {
        self.best_corr_hp = best_corr_hp;
    }

    /// Shift the tone register — `vad_tone_detection_update`.
    ///
    /// Called once per `Pitch_ol`, *before* the detections that follow it. At
    /// 4.75 and 5.15 kbit/s only one open-loop lag is computed per frame, so
    /// the register is shifted twice and the missing flag is assumed set.
    pub fn shift_tone_register(&mut self, ctx: &mut DspContext, one_lag_per_frame: bool) {
        self.tone = shr(ctx, self.tone, 1);
        if one_lag_per_frame {
            self.tone = shr(ctx, self.tone, 1);
            self.tone = Word16(self.tone.0 | 0x2000);
        }
    }

    /// Set the tone flag when the pitch gain is high — `vad_tone_detection`.
    ///
    /// `peak` and `energy` are the *raw* section maximum and energy from the
    /// open-loop search, before normalisation: the comparison is against the
    /// unnormalised pair.
    pub fn observe_tone(&mut self, ctx: &mut DspContext, peak: Word32, energy: Word32) {
        let scaled = round(ctx, energy);
        if scaled.0 > 0 && l_msu(ctx, peak, scaled, TONE_THR).0 > 0 {
            self.tone = Word16(self.tone.0 | 0x4000);
        }
    }

    /// Decide whether the frame is periodic — `vad_pitch_detection`.
    ///
    /// Takes both open-loop lags of the frame, and counts how many are close
    /// to the previous one. Four agreements across two frames make it voiced.
    pub fn observe_pitch(&mut self, ctx: &mut DspContext, lags: [Word16; 2]) {
        let mut lagcount = Word16(0);
        for lag in lags {
            let delta = sub(ctx, self.oldlag, lag);
            let difference = abs_s(ctx, delta);
            if sub(ctx, difference, LTHRESH).0 < 0 {
                lagcount = add(ctx, lagcount, Word16(1));
            }
            self.oldlag = lag;
        }
        self.pitch = shr(ctx, self.pitch, 1);
        let total = add(ctx, self.oldlag_count, lagcount);
        if sub(ctx, total, NTHRESH).0 >= 0 {
            self.pitch = Word16(self.pitch.0 | 0x4000);
        }
        self.oldlag_count = lagcount;
    }

    /// The decision this detector last made.
    #[must_use]
    pub const fn decision(&self) -> bool {
        self.speech_vad_decision
    }

    // ------------------------------------------------------------ internals --

    /// Split the frame into nine bands and measure each — `filter_bank`.
    fn filter_bank(&mut self, ctx: &mut DspContext, input: &[Word16]) -> [Word16; COMPLEN] {
        let mut buf = [Word16(0); FRAME_LEN];
        self.first_filter_stage(ctx, input, &mut buf);

        for i in 0..FRAME_LEN / 4 {
            let mut a = self.a_data5[1];
            filter5(ctx, &mut buf, 4 * i, 4 * i + 2, &mut a);
            self.a_data5[1] = a;
            let mut b = self.a_data5[2];
            filter5(ctx, &mut buf, 4 * i + 1, 4 * i + 3, &mut b);
            self.a_data5[2] = b;
        }
        for i in 0..FRAME_LEN / 8 {
            let mut m = self.a_data3[0];
            filter3(ctx, &mut buf, 8 * i, 8 * i + 4, &mut m);
            self.a_data3[0] = m;
            let mut m = self.a_data3[1];
            filter3(ctx, &mut buf, 8 * i + 2, 8 * i + 6, &mut m);
            self.a_data3[1] = m;
            let mut m = self.a_data3[4];
            filter3(ctx, &mut buf, 8 * i + 3, 8 * i + 7, &mut m);
            self.a_data3[4] = m;
        }
        for i in 0..FRAME_LEN / 16 {
            let mut m = self.a_data3[2];
            filter3(ctx, &mut buf, 16 * i, 16 * i + 8, &mut m);
            self.a_data3[2] = m;
            let mut m = self.a_data3[3];
            filter3(ctx, &mut buf, 16 * i + 4, 16 * i + 12, &mut m);
            self.a_data3[3] = m;
        }

        // Band, then start, end, stride, offset and scale. Highest band first,
        // as the reference writes them.
        let mut level = [Word16(0); COMPLEN];
        for (band, count1, count2, stride, offset, scale) in BANDS {
            let mut carried = self.sub_level[band];
            level[band] = level_calculation(
                ctx, &buf, &mut carried, count1, count2, stride, offset, scale,
            );
            self.sub_level[band] = carried;
        }
        level
    }

    /// The fifth-order pair that opens the bank — `first_filter_stage`.
    fn first_filter_stage(
        &mut self,
        ctx: &mut DspContext,
        input: &[Word16],
        out: &mut [Word16; FRAME_LEN],
    ) {
        let mut data0 = self.a_data5[0][0];
        let mut data1 = self.a_data5[0][1];
        for i in 0..FRAME_LEN / 4 {
            let scaled = shr(ctx, input[4 * i], 2);
            let tap = mult(ctx, COEFF5_1, data0);
            let temp0 = sub(ctx, scaled, tap);
            let tap = mult(ctx, COEFF5_1, temp0);
            let temp1 = add(ctx, data0, tap);

            let scaled = shr(ctx, input[4 * i + 1], 2);
            let tap = mult(ctx, COEFF5_2, data1);
            let temp3 = sub(ctx, scaled, tap);
            let tap = mult(ctx, COEFF5_2, temp3);
            let temp2 = add(ctx, data1, tap);

            out[4 * i] = add(ctx, temp1, temp2);
            out[4 * i + 1] = sub(ctx, temp1, temp2);

            let scaled = shr(ctx, input[4 * i + 2], 2);
            let tap = mult(ctx, COEFF5_1, temp0);
            data0 = sub(ctx, scaled, tap);
            let tap = mult(ctx, COEFF5_1, data0);
            let temp1 = add(ctx, temp0, tap);

            let scaled = shr(ctx, input[4 * i + 3], 2);
            let tap = mult(ctx, COEFF5_2, temp3);
            data1 = sub(ctx, scaled, tap);
            let tap = mult(ctx, COEFF5_2, data1);
            let temp2 = add(ctx, temp3, tap);

            out[4 * i + 2] = add(ctx, temp1, temp2);
            out[4 * i + 3] = sub(ctx, temp1, temp2);
        }
        self.a_data5[0][0] = data0;
        self.a_data5[0][1] = data1;
    }

    /// `vad_decision`: everything downstream of the filter bank.
    fn decide(&mut self, ctx: &mut DspContext, level: &[Word16; COMPLEN], power: Word32) -> bool {
        // Squared sum of level-over-noise, per band.
        let mut acc = Word32(0);
        for (&estimate, &band) in self.bckr_est.iter().zip(level) {
            let exp = norm_s(estimate);
            let denom = shl(ctx, estimate, exp);
            let ratio = div_s(shr(ctx, band, 1), denom);
            let ratio = shl(ctx, ratio, exp - (UNIRSHFT - 1));
            acc = l_mac(ctx, acc, ratio, ratio);
        }
        let snr_sum = extract_h(l_shl(ctx, acc, 6));
        let snr_sum = mult(ctx, snr_sum, INV_COMPLEN);

        let mut total = Word32(0);
        for &estimate in &self.bckr_est {
            total = l_add(ctx, total, Word32(i32::from(estimate.0)));
        }
        let noise_level = extract_h(l_shl(ctx, total, 13));

        // The threshold slides down as the noise floor rises.
        let offset = sub(ctx, noise_level, VAD_P1);
        let slid = mult(ctx, VAD_SLOPE, offset);
        let mut threshold = add(ctx, slid, VAD_THR_HIGH);
        if sub(ctx, threshold, VAD_THR_LOW).0 < 0 {
            threshold = VAD_THR_LOW;
        }

        self.vadreg = shr(ctx, self.vadreg, 1);
        if sub(ctx, snr_sum, threshold).0 > 0 {
            self.vadreg = Word16(self.vadreg.0 | 0x4000);
        }

        let low_power = l_sub(ctx, power, VAD_POW_LOW).0 < 0;
        self.adapt_complex_estimate(ctx, low_power);
        self.complex_warning = self.complex_decision(ctx, low_power);
        self.update_noise_estimate(ctx, level);
        self.speech_vad_decision = self.add_hangover(ctx, noise_level, low_power);
        self.speech_vad_decision
    }

    /// `complex_estimate_adapt`: track the high-band correlation, quickly
    /// downward and slowly upward once it is already high.
    fn adapt_complex_estimate(&mut self, ctx: &mut DspContext, low_power: bool) {
        let high = sub(ctx, self.corr_hp_fast, CVAD_THRESH_ADAPT_HIGH).0 >= 0;
        let decreasing = sub(ctx, self.best_corr_hp, self.corr_hp_fast).0 < 0;
        // Below the high threshold the speed is the same either way; above
        // it, falling is tracked much faster than rising.
        let alpha = if high {
            if decreasing {
                CVAD_ADAPT_REALLY_FAST
            } else {
                CVAD_ADAPT_SLOW
            }
        } else {
            CVAD_ADAPT_FAST
        };

        let mut acc = l_deposit_h(self.corr_hp_fast);
        acc = l_msu(ctx, acc, alpha, self.corr_hp_fast);
        acc = l_mac(ctx, acc, alpha, self.best_corr_hp);
        self.corr_hp_fast = round(ctx, acc);

        if sub(ctx, self.corr_hp_fast, CVAD_MIN_CORR).0 < 0 || low_power {
            self.corr_hp_fast = CVAD_MIN_CORR;
        }
    }

    /// `complex_vad`: is the background a complex signal rather than noise?
    fn complex_decision(&mut self, ctx: &mut DspContext, low_power: bool) -> bool {
        self.complex_high = shr(ctx, self.complex_high, 1);
        self.complex_low = shr(ctx, self.complex_low, 1);

        if !low_power {
            if sub(ctx, self.corr_hp_fast, CVAD_THRESH_ADAPT_HIGH).0 > 0 {
                self.complex_high = Word16(self.complex_high.0 | 0x4000);
            }
            if sub(ctx, self.corr_hp_fast, CVAD_THRESH_ADAPT_LOW).0 > 0 {
                self.complex_low = Word16(self.complex_low.0 | 0x4000);
            }
        }

        if sub(ctx, self.corr_hp_fast, CVAD_THRESH_HANG).0 > 0 {
            self.complex_hang_timer = add(ctx, self.complex_hang_timer, Word16(1));
        } else {
            self.complex_hang_timer = Word16(0);
        }

        (self.complex_high.0 & 0x7f80) == 0x7f80 || (self.complex_low.0 & 0x7fff) == 0x7fff
    }

    /// `update_cntrl`: how fast the noise estimate is allowed to move.
    fn update_control(&mut self, ctx: &mut DspContext, level: &[Word16; COMPLEN]) {
        // A complex background holds the update speed down for a while.
        if self.complex_warning && sub(ctx, self.stat_count, CAD_MIN_STAT_COUNT).0 < 0 {
            self.stat_count = CAD_MIN_STAT_COUNT;
        }

        // Two separate reasons to restart the stationarity counter, kept
        // apart because the reference keeps them apart: a sustained pitch or
        // tone, or eight consecutive silent decisions.
        let sustained = (self.pitch.0 & 0x6000) == 0x6000 || (self.tone.0 & 0x7c00) == 0x7c00;
        let eight_silent = (self.vadreg.0 & 0x7f80) == 0;
        if sustained || eight_silent {
            self.stat_count = STAT_COUNT;
        } else {
            // How far each band has moved from its own slow average.
            let mut stat_rat = Word16(0);
            for (&band, &average) in level.iter().zip(&self.ave_level) {
                let (mut num, mut denom) = if sub(ctx, band, average).0 > 0 {
                    (band, average)
                } else {
                    (average, band)
                };
                if sub(ctx, num, STAT_THR_LEVEL).0 < 0 {
                    num = STAT_THR_LEVEL;
                }
                if sub(ctx, denom, STAT_THR_LEVEL).0 < 0 {
                    denom = STAT_THR_LEVEL;
                }
                let exp = norm_s(denom);
                let denom = shl(ctx, denom, exp);
                let ratio = div_s(shr(ctx, num, 1), denom);
                let contribution = shr(ctx, ratio, 8 - exp);
                stat_rat = add(ctx, stat_rat, contribution);
            }

            if sub(ctx, stat_rat, STAT_THR).0 > 0 {
                self.stat_count = STAT_COUNT;
            } else if (self.vadreg.0 & 0x4000) != 0 && self.stat_count.0 != 0 {
                self.stat_count = sub(ctx, self.stat_count, Word16(1));
            }
        }

        // The averaging speed itself depends on what was just decided.
        let alpha = if sub(ctx, self.stat_count, STAT_COUNT).0 == 0 {
            Word16(32767)
        } else if (self.vadreg.0 & 0x4000) == 0 {
            ALPHA5
        } else {
            ALPHA4
        };
        for (average, &band) in self.ave_level.iter_mut().zip(level) {
            let difference = sub(ctx, band, *average);
            let step = mult_r(ctx, alpha, difference);
            *average = add(ctx, *average, step);
        }
    }

    /// `noise_estimate_update`.
    fn update_noise_estimate(&mut self, ctx: &mut DspContext, level: &[Word16; COMPLEN]) {
        self.update_control(ctx, level);

        let quiet_and_unpitched = (self.vadreg.0 & 0x7800) == 0
            && (self.pitch.0 & 0x7800) == 0
            && self.complex_hang_count.0 == 0;
        let (alpha_up, alpha_down, bckr_add) = if quiet_and_unpitched {
            (ALPHA_UP1, ALPHA_DOWN1, Word16(2))
        } else if self.stat_count.0 == 0 && self.complex_hang_count.0 == 0 {
            (ALPHA_UP2, ALPHA_DOWN2, Word16(2))
        } else {
            (Word16(0), ALPHA3, Word16(0))
        };

        for (estimate, &previous) in self.bckr_est.iter_mut().zip(&self.old_level) {
            let difference = sub(ctx, previous, *estimate);
            if difference.0 < 0 {
                let step = mult_r(ctx, alpha_down, difference);
                let moved = add(ctx, *estimate, step);
                *estimate = add(ctx, Word16(-2), moved);
                if sub(ctx, *estimate, NOISE_MIN).0 < 0 {
                    *estimate = NOISE_MIN;
                }
            } else {
                let step = mult_r(ctx, alpha_up, difference);
                let moved = add(ctx, *estimate, step);
                *estimate = add(ctx, bckr_add, moved);
                if sub(ctx, *estimate, NOISE_MAX).0 > 0 {
                    *estimate = NOISE_MAX;
                }
            }
        }
        self.old_level = *level;
    }

    /// `hangover_addition`: hold the decision on past the end of a burst.
    fn add_hangover(
        &mut self,
        ctx: &mut DspContext,
        noise_level: Word16,
        low_power: bool,
    ) -> bool {
        let (burst_len, hang_len) = if sub(ctx, noise_level, HANG_NOISE_THR).0 > 0 {
            (BURST_LEN_HIGH_NOISE, HANG_LEN_HIGH_NOISE)
        } else {
            (BURST_LEN_LOW_NOISE, HANG_LEN_LOW_NOISE)
        };

        if low_power {
            self.burst_count = Word16(0);
            self.hang_count = Word16(0);
            self.complex_hang_count = Word16(0);
            self.complex_hang_timer = Word16(0);
            return false;
        }

        if sub(ctx, self.complex_hang_timer, CVAD_HANG_LIMIT).0 > 0
            && sub(ctx, self.complex_hang_count, CVAD_HANG_LENGTH).0 < 0
        {
            self.complex_hang_count = CVAD_HANG_LENGTH;
        }

        if self.complex_hang_count.0 != 0 {
            // A long complex background overrides the decision entirely.
            self.burst_count = BURST_LEN_HIGH_NOISE;
            self.complex_hang_count = sub(ctx, self.complex_hang_count, Word16(1));
            return true;
        }
        if (self.vadreg.0 & 0x3ff0) == 0
            && sub(ctx, self.corr_hp_fast, CVAD_THRESH_IN_NOISE).0 > 0
        {
            return true;
        }

        if (self.vadreg.0 & 0x4000) != 0 {
            self.burst_count = add(ctx, self.burst_count, Word16(1));
            if sub(ctx, self.burst_count, burst_len).0 >= 0 {
                self.hang_count = hang_len;
            }
            return true;
        }

        self.burst_count = Word16(0);
        if self.hang_count.0 > 0 {
            self.hang_count = sub(ctx, self.hang_count, Word16(1));
            return true;
        }
        false
    }
}

/// One fifth-order half-band pair with decimation — `filter5`.
fn filter5(ctx: &mut DspContext, buf: &mut [Word16], lo: usize, hi: usize, data: &mut [Word16; 2]) {
    let tap = mult(ctx, COEFF5_1, data[0]);
    let temp0 = sub(ctx, buf[lo], tap);
    let tap = mult(ctx, COEFF5_1, temp0);
    let temp1 = add(ctx, data[0], tap);
    data[0] = temp0;

    let tap = mult(ctx, COEFF5_2, data[1]);
    let temp0 = sub(ctx, buf[hi], tap);
    let tap = mult(ctx, COEFF5_2, temp0);
    let temp2 = add(ctx, data[1], tap);
    data[1] = temp0;

    let sum = add(ctx, temp1, temp2);
    let diff = sub(ctx, temp1, temp2);
    buf[lo] = shr(ctx, sum, 1);
    buf[hi] = shr(ctx, diff, 1);
}

/// One third-order half-band pair with decimation — `filter3`.
///
/// Note the outputs are written in the opposite order from `filter5`: the
/// high-pass result lands in the *second* slot and is computed from the
/// pre-update `in0`.
fn filter3(ctx: &mut DspContext, buf: &mut [Word16], lo: usize, hi: usize, data: &mut Word16) {
    let tap = mult(ctx, COEFF3, *data);
    let temp1 = sub(ctx, buf[hi], tap);
    let tap = mult(ctx, COEFF3, temp1);
    let temp2 = add(ctx, *data, tap);
    *data = temp1;

    let high = sub(ctx, buf[lo], temp2);
    let low = add(ctx, buf[lo], temp2);
    buf[hi] = shr(ctx, high, 1);
    buf[lo] = shr(ctx, low, 1);
}

/// Sum the absolute values of one band — `level_calculation`.
///
/// The split into two runs is what carries a band's level across the frame
/// boundary: the samples past `count1` are measured now and *also* added to
/// the next frame's total, through `carried`.
#[allow(clippy::too_many_arguments)]
fn level_calculation(
    ctx: &mut DspContext,
    data: &[Word16],
    carried: &mut Word16,
    count1: usize,
    count2: usize,
    stride: usize,
    offset: usize,
    scale: i16,
) -> Word16 {
    let mut tail = Word32(0);
    for i in count1..count2 {
        let magnitude = abs_s(ctx, data[stride * i + offset]);
        tail = l_mac(ctx, tail, Word16(1), magnitude);
    }

    let previous = l_shl(ctx, Word32(i32::from(carried.0)), 16 - scale);
    let mut total = l_add(ctx, tail, previous);
    *carried = extract_h(l_shl(ctx, tail, scale));

    for i in 0..count1 {
        let magnitude = abs_s(ctx, data[stride * i + offset]);
        total = l_mac(ctx, total, Word16(1), magnitude);
    }
    extract_h(l_shl(ctx, total, scale))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Against TS 26.073's own `vad1`, frame by frame, state and all.
    ///
    /// The decision alone would be nearly vacuous — a detector wired to a
    /// constant agrees with this signal about 85% of the time — so every
    /// register, every counter and all nine noise estimates are compared on
    /// every frame. That is also the only way a divergence can be localised at
    /// all, since none of this reaches the bitstream.
    #[test]
    fn the_whole_state_matches_the_reference_every_frame() {
        let text = include_str!("../../testdata/nb_vad1_vectors.txt");
        let pcm: &[u8] = include_bytes!("../../testdata/amrnb_dtx_input.pcm");
        let samples: Vec<i16> = pcm
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();

        let mut ctx = DspContext::default();
        let mut vad = VoiceActivityDetector::new();
        let mut buffer = [Word16(0); LOOKAHEAD + FRAME_LEN];
        let mut rows = 0usize;
        let mut compared = 0usize;
        let mut decisions = (0usize, 0usize);

        for line in text.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
            let fields: Vec<&str> = line.split('|').collect();
            assert_eq!(fields.len(), 3, "malformed row `{line}`");
            let head: Vec<i32> = fields[0]
                .split_whitespace()
                .map(|v| v.parse().expect("number"))
                .collect();
            let frame = usize::try_from(head[0]).expect("frame index");
            let want_decision = head[1] == 1;
            let want_state: Vec<i16> = fields[1]
                .split_whitespace()
                .map(|v| v.parse().expect("state"))
                .collect();
            let want_noise: Vec<i16> = fields[2]
                .split_whitespace()
                .map(|v| v.parse().expect("estimate"))
                .collect();
            assert_eq!(frame, rows, "the fixture skipped a frame");

            // Slide the previous frame's tail into the history, exactly as the
            // encoder's own buffer does.
            for i in 0..LOOKAHEAD {
                buffer[i] = buffer[FRAME_LEN + i];
            }
            for (slot, &sample) in buffer[LOOKAHEAD..]
                .iter_mut()
                .zip(&samples[frame * FRAME_LEN..])
            {
                *slot = Word16(sample);
            }

            // The hooks the encoder's open-loop stage drives, with the same
            // deterministic inputs the probe used. Without them the pitch,
            // tone and complex registers never leave zero and half the state
            // comparison below is vacuous.
            let f = i32::try_from(frame).expect("short fixture");
            vad.shift_tone_register(&mut ctx, frame % 3 == 0);
            vad.observe_tone(
                &mut ctx,
                Word32(200_000 + (f * 37013).rem_euclid(900_000)),
                Word32(150_000 + (f * 11317).rem_euclid(300_000)),
            );
            vad.observe_correlation(Word16(
                i16::try_from(8000 + (f * 613).rem_euclid(20000)).expect("in range"),
            ));
            let base = i16::try_from(60 + (f / 12) * 23).expect("in range");
            let near = frame % 12 < 8;
            vad.observe_pitch(
                &mut ctx,
                [
                    Word16(base + if near { 0 } else { 17 }),
                    Word16(base + if near { 1 } else { 30 }),
                ],
            );

            let got = vad.process(&mut ctx, &buffer);
            assert_eq!(got, want_decision, "frame {frame} decision");
            if got {
                decisions.0 += 1;
            } else {
                decisions.1 += 1;
            }

            let state = [
                vad.vadreg.0,
                vad.pitch.0,
                vad.tone.0,
                vad.complex_high.0,
                vad.complex_low.0,
                vad.stat_count.0,
                vad.burst_count.0,
                vad.hang_count.0,
            ];
            for (i, (&mine, &theirs)) in state.iter().zip(&want_state).enumerate() {
                assert_eq!(mine, theirs, "frame {frame} state field {i}");
                compared += 1;
            }
            for (i, (&mine, &theirs)) in vad.bckr_est.iter().zip(&want_noise).enumerate() {
                assert_eq!(mine.0, theirs, "frame {frame} noise estimate {i}");
                compared += 1;
            }
            rows += 1;
        }

        assert_eq!(rows, 150, "the fixture lost rows");
        assert_eq!(compared, rows * (8 + COMPLEN));
        // And every field has to move, or the comparison is decorative. The
        // four register fields in particular are only driven by the hooks
        // above: without them they stay at zero for all 150 frames and agree
        // with anything.
        assert_eq!(decisions, (62, 88), "the fixture stopped discriminating");
        assert!(vad.pitch.0 != 0 || vad.tone.0 != 0, "the registers never moved");
    }
}
