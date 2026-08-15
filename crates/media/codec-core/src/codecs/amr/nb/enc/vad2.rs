//! Voice activity detection option 2 for AMR-NB, TS 26.073 `vad2.c`.
//!
//! # Narrowband only, by specification
//!
//! AMR-NB defines *two* detectors and ships both in its reference
//! (`vad1.c`, `vad2.c`, with `vadname.c` reporting which was compiled).
//! AMR-WB defines one (`wb_vad.c`), which lives in `wb/enc/vad.rs` and is
//! already bit-exact. There is no "VAD option 2" for wideband, so this module
//! has no wideband counterpart and inventing one would be inventing spec.
//!
//! # A completely different detector from VAD1
//!
//! VAD1 works from the encoder's own analysis — the LP residual, the open-loop
//! pitch lags, the tone flag. VAD2 does its own signal analysis from scratch:
//! it pre-emphasises the input, takes a 128-point real FFT, sums the energy
//! into sixteen non-uniform channels, and drives a SNR/hangover state machine
//! from those. The two share only their output type.
//!
//! That is why VAD1's port could not be parameterised into covering this one,
//! and why `build-amr-dtx-fixtures.sh` can tell them apart: on the committed
//! DTX fixture they choose different frame types on 21 of 150 frames.
//!
//! # Called twice per frame
//!
//! `vad2()` consumes 80 samples — *half* a 20 ms frame — and the frame's
//! decision is the OR of two successive calls. `cod_amr.c` does that pairing;
//! this module provides the half-frame call, exactly as the reference does.
//!
//! # Testing
//!
//! Same problem as VAD1, for the same reason: the decision appears nowhere in
//! the bitstream, so a wrong answer mid-spurt is invisible. The tests compare
//! the whole state against `tools/nb_vad2_probe.c` every half-frame — the
//! sixteen channel energies, the sixteen noise estimates, the long-term dB
//! array, and every counter — rather than the boolean alone.

use crate::codecs::amr::nb::math::{log2, pow2};
use crate::fixed_point::arith::{abs_s, add, extract_l, mult, mult_r, round, sub};
use crate::fixed_point::arith32::{l_add, l_deposit_h, l_mac, l_msu, l_mult, l_negate, l_sub};
use crate::fixed_point::div::div_s;
use crate::fixed_point::oper32::{l_extract, mpy_32_16};
use crate::fixed_point::shift::{l_shl, l_shr, l_shr_r, norm_s, shl, shr, shr_r};
use crate::fixed_point::types::{DspContext, Word16, Word32};

/// Samples `vad2` consumes per call: half a 20 ms frame.
pub const FRM_LEN: usize = 80;
/// Offset of the pre-emphasised data inside the FFT buffer.
const DELAY: usize = 24;
/// FFT size.
const FFT_LEN: usize = 128;
const NUM_CHAN: usize = 16;
const UPDATE_THLD: i16 = 35;
const HYSTER_CNT_THLD: i16 = 6;
const UPDATE_CNT_THLD: i16 = 50;

/// Channel energy scaled as 22,9.
const NOISE_FLOOR_CHAN_0: i16 = 512;
const MIN_CHAN_ENRG_0: i16 = 32;
const MIN_NOISE_ENRG_0: i32 = 32;
const INE_NOISE_0: i32 = 8192;
const FRACTIONAL_BITS_0: i16 = 9;

/// Channel energy scaled as 27,4.
const NOISE_FLOOR_CHAN_1: i16 = 16;
const MIN_CHAN_ENRG_1: i16 = 1;
const INE_NOISE_1: i32 = 256;
const FRACTIONAL_BITS_1: i16 = 4;

const STATE_1_TO_0_SHIFT_R: i16 = FRACTIONAL_BITS_1 - FRACTIONAL_BITS_0;
const STATE_0_TO_1_SHIFT_R: i16 = FRACTIONAL_BITS_0 - FRACTIONAL_BITS_1;

const HIGH_ALPHA: i16 = 29491;
const LOW_ALPHA: i16 = 22938;
const ALPHA_RANGE: i16 = HIGH_ALPHA - LOW_ALPHA;
const DEV_THLD: i16 = 7168;
const PRE_EMP_FAC: i16 = -26214;
const CEE_SM_FAC: i16 = 18022;
const ONE_MINUS_CEE_SM_FAC: i16 = 14746;
const CNE_SM_FAC: i16 = 3277;
const ONE_MINUS_CNE_SM_FAC: i16 = 29491;
const FFT_HEADROOM: i16 = 2;

/// Lower and upper FFT bin of each of the sixteen channels. Bins 0 (DC), 1 and
/// 64 (foldover) are deliberately excluded.
const CH_TBL: [[usize; 2]; NUM_CHAN] = [
    [2, 3],
    [4, 5],
    [6, 7],
    [8, 9],
    [10, 11],
    [12, 13],
    [14, 16],
    [17, 19],
    [20, 22],
    [23, 26],
    [27, 30],
    [31, 35],
    [36, 41],
    [42, 48],
    [49, 55],
    [56, 63],
];

/// Reciprocal of each channel's bin count, so the division is a multiply.
const CH_TBL_SH: [i16; NUM_CHAN] = [
    16384, 16384, 16384, 16384, 16384, 16384, 10923, 10923, 10923, 8192, 8192, 6554, 5461, 4681,
    4681, 4096,
];

/// Voice metric as a function of quantised channel SNR. Non-linear, with a
/// deadband near zero.
const VM_TBL: [i16; 90] = [
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 4, 4, 4, 5, 5, 5, 6, 6, 7, 7, 7, 8, 8, 9, 9,
    10, 10, 11, 12, 12, 13, 13, 14, 15, 15, 16, 17, 17, 18, 19, 20, 20, 21, 22, 23, 24, 24, 25, 26,
    27, 28, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48,
    49, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50,
];

/// Hangover length as a function of peak SNR, in 3 dB steps.
const HANGOVER_TABLE: [i16; 20] = [
    30, 30, 30, 30, 30, 30, 28, 26, 24, 22, 20, 18, 16, 14, 12, 10, 8, 8, 8, 8,
];

/// Burst sensitivity as a function of peak SNR, in 3 dB steps.
const BURSTCOUNT_TABLE: [i16; 20] = [8, 8, 8, 8, 8, 8, 8, 8, 7, 6, 5, 4, 4, 4, 4, 4, 4, 4, 4, 4];

/// Voice-metric threshold as a function of peak SNR, in 3 dB steps.
const VM_THRESHOLD_TABLE: [i16; 20] = [
    34, 34, 34, 34, 34, 34, 34, 34, 34, 34, 34, 40, 51, 71, 100, 139, 191, 257, 337, 432,
];

const NOISE_FLOOR_CHAN: [i16; 2] = [NOISE_FLOOR_CHAN_0, NOISE_FLOOR_CHAN_1];
const MIN_CHAN_ENRG: [i16; 2] = [MIN_CHAN_ENRG_0, MIN_CHAN_ENRG_1];
const INE_NOISE: [i32; 2] = [INE_NOISE_0, INE_NOISE_1];
const FBITS: [i16; 2] = [FRACTIONAL_BITS_0, FRACTIONAL_BITS_1];
const STATE_CHANGE_SHIFT_R: [i16; 2] = [STATE_1_TO_0_SHIFT_R, STATE_0_TO_1_SHIFT_R];
/// Energy scale given 30,1 input scaling, allowing for the -6 dB input shift.
const ENRG_NORM_SHIFT: [i16; 2] = [FRACTIONAL_BITS_0 - 1 + 2, FRACTIONAL_BITS_1 - 1 + 2];

/// Twiddle factors: cos/sin pairs for the 128-point FFT.
const PHS_TBL: [i16; 128] = [
    32767, 0, 32729, -1608, 32610, -3212, 32413, -4808, 32138, -6393, 31786, -7962, 31357, -9512,
    30853, -11039, 30274, -12540, 29622, -14010, 28899, -15447, 28106, -16846, 27246, -18205,
    26320, -19520, 25330, -20788, 24279, -22006, 23170, -23170, 22006, -24279, 20788, -25330,
    19520, -26320, 18205, -27246, 16846, -28106, 15447, -28899, 14010, -29622, 12540, -30274,
    11039, -30853, 9512, -31357, 7962, -31786, 6393, -32138, 4808, -32413, 3212, -32610, 1608,
    -32729, 0, -32768, -1608, -32729, -3212, -32610, -4808, -32413, -6393, -32138, -7962, -31786,
    -9512, -31357, -11039, -30853, -12540, -30274, -14010, -29622, -15447, -28899, -16846, -28106,
    -18205, -27246, -19520, -26320, -20788, -25330, -22006, -24279, -23170, -23170, -24279, -22006,
    -25330, -20788, -26320, -19520, -27246, -18205, -28106, -16846, -28899, -15447, -29622, -14010,
    -30274, -12540, -30853, -11039, -31357, -9512, -31786, -7962, -32138, -6393, -32413, -4808,
    -32610, -3212, -32729, -1608,
];

/// `10*log10(x)/128`, scaled 7,8. TS 26.073 `fn10Log10`.
fn fn10_log10(ctx: &mut DspContext, l_input: Word32, fbits: i16) -> Word16 {
    let (integer, fraction) = log2(ctx, l_input);
    let integer = sub(ctx, integer, Word16(fbits));
    // 24660 = 10*log10(2)/4 scaled 0,15.
    let ltmp = mpy_32_16(integer, fraction, Word16(24660));
    // The extra shift is the 30,1 => 15,0 extract correction.
    let ltmp = l_shr_r(ctx, ltmp, 5 + 1);
    extract_l(ltmp)
}

/// Block-normalise `input` into `out`, returning the left shift applied.
///
/// An all-zero sequence returns the maximum shift rather than `norm_s(0)`,
/// deliberately: the point is to associate silence with low energy.
fn block_norm(ctx: &mut DspContext, input: &[Word16], out: &mut [Word16], headroom: i16) -> i16 {
    let mut max = abs_s(ctx, input[0]);
    for &sample in &input[1..] {
        let adata = abs_s(ctx, sample);
        if sub(ctx, adata, max).0 > 0 {
            max = adata;
        }
    }
    if max.0 != 0 {
        let scnt = norm_s(max) - headroom;
        for (slot, &sample) in out.iter_mut().zip(input.iter()) {
            *slot = shl(ctx, sample, scnt);
        }
        scnt
    } else {
        for slot in out.iter_mut() {
            *slot = Word16(0);
        }
        16 - headroom
    }
}

/// Decimation-in-time complex FFT, in place. Real and imaginary parts
/// interleaved, so the counters step by two.
//
// The index arithmetic deliberately mirrors the reference's own `Word16`
// counters -- butterfly tops, bottoms and the phase-table stride are computed
// with `add`/`shl`/`shr` exactly as `c_fft` does, because that is what makes
// the port checkable line against line. Every value is bounded by FFT_LEN
// (128), so the narrowing casts cannot lose anything.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::needless_range_loop
)]
fn c_fft(ctx: &mut DspContext, farray: &mut [Word16; FFT_LEN]) {
    const SIZE: i16 = FFT_LEN as i16;
    const SIZE_BY_TWO: i16 = 64;
    const NUM_STAGE: usize = 6;
    const II_TABLE: [i16; NUM_STAGE] = [64, 32, 16, 8, 4, 2];

    // Bit-reversed reordering.
    let mut j: i16 = 0;
    let mut i: i16 = 0;
    while i < SIZE - 2 {
        if sub(ctx, Word16(j), Word16(i)).0 > 0 {
            farray.swap(i as usize, j as usize);
            farray.swap(i as usize + 1, j as usize + 1);
        }
        let mut k = SIZE_BY_TWO;
        while sub(ctx, Word16(j), Word16(k)).0 >= 0 {
            j = sub(ctx, Word16(j), Word16(k)).0;
            k = shr(ctx, Word16(k), 1).0;
        }
        j = add(ctx, Word16(j), Word16(k)).0;
        i += 2;
    }

    for stage in 0..NUM_STAGE {
        let jj = shl(ctx, Word16(2), stage as i16).0; // FFT size
        let kk = shl(ctx, Word16(jj), 1).0; // twice it
        let ii = II_TABLE[stage];
        let ii2 = shl(ctx, Word16(ii), 1).0;
        let mut ji: i16 = 0; // phase table index

        let mut j = 0i16;
        while j < jj {
            let mut k = j;
            while k < SIZE {
                let kj = add(ctx, Word16(k), Word16(jj)).0; // butterfly bottom
                let (ku, kju) = (k as usize, kj as usize);

                let mut ftmp_real = l_mult(ctx, farray[kju], Word16(PHS_TBL[ji as usize]));
                ftmp_real = l_msu(
                    ctx,
                    ftmp_real,
                    farray[kju + 1],
                    Word16(PHS_TBL[ji as usize + 1]),
                );

                let mut ftmp_imag = l_mult(ctx, farray[kju + 1], Word16(PHS_TBL[ji as usize]));
                ftmp_imag = l_mac(
                    ctx,
                    ftmp_imag,
                    farray[kju],
                    Word16(PHS_TBL[ji as usize + 1]),
                );

                let tmp1 = round(ctx, ftmp_real);
                let tmp2 = round(ctx, ftmp_imag);

                let tmp = sub(ctx, farray[ku], tmp1);
                farray[kju] = shr(ctx, tmp, 1);
                let tmp = sub(ctx, farray[ku + 1], tmp2);
                farray[kju + 1] = shr(ctx, tmp, 1);
                let tmp = add(ctx, farray[ku], tmp1);
                farray[ku] = shr(ctx, tmp, 1);
                let tmp = add(ctx, farray[ku + 1], tmp2);
                farray[ku + 1] = shr(ctx, tmp, 1);

                k += kk;
            }
            ji = add(ctx, Word16(ji), Word16(ii2)).0;
            j += 2;
        }
    }
}

/// Real-input FFT, in place: the complex FFT plus the real-sequence fold.
fn r_fft(ctx: &mut DspContext, farray: &mut [Word16; FFT_LEN]) {
    const SIZE: usize = FFT_LEN;
    const SIZE_BY_TWO: usize = 64;

    c_fft(ctx, farray);

    // DC and foldover first.
    let ftmp1_real = farray[0];
    let ftmp2_real = farray[1];
    farray[0] = add(ctx, ftmp1_real, ftmp2_real);
    farray[1] = sub(ctx, ftmp1_real, ftmp2_real);

    let mut i = 2usize;
    while i <= SIZE_BY_TWO {
        let j = SIZE - i;
        let ftmp1_real = add(ctx, farray[i], farray[j]);
        let ftmp1_imag = sub(ctx, farray[i + 1], farray[j + 1]);
        let ftmp2_real = add(ctx, farray[i + 1], farray[j + 1]);
        let ftmp2_imag = sub(ctx, farray[j], farray[i]);

        let lftmp1_real = l_deposit_h(ftmp1_real);
        let lftmp1_imag = l_deposit_h(ftmp1_imag);

        let mut ltmp1 = l_mac(ctx, lftmp1_real, ftmp2_real, Word16(PHS_TBL[i]));
        ltmp1 = l_msu(ctx, ltmp1, ftmp2_imag, Word16(PHS_TBL[i + 1]));
        let shifted = l_shr(ctx, ltmp1, 1);
        farray[i] = round(ctx, shifted);

        let mut ltmp1 = l_mac(ctx, lftmp1_imag, ftmp2_imag, Word16(PHS_TBL[i]));
        ltmp1 = l_mac(ctx, ltmp1, ftmp2_real, Word16(PHS_TBL[i + 1]));
        let shifted = l_shr(ctx, ltmp1, 1);
        farray[i + 1] = round(ctx, shifted);

        let mut ltmp1 = l_mac(ctx, lftmp1_real, ftmp2_real, Word16(PHS_TBL[j]));
        ltmp1 = l_mac(ctx, ltmp1, ftmp2_imag, Word16(PHS_TBL[j + 1]));
        let shifted = l_shr(ctx, ltmp1, 1);
        farray[j] = round(ctx, shifted);

        let mut ltmp1 = l_negate(ctx, lftmp1_imag);
        ltmp1 = l_msu(ctx, ltmp1, ftmp2_imag, Word16(PHS_TBL[j]));
        ltmp1 = l_mac(ctx, ltmp1, ftmp2_real, Word16(PHS_TBL[j + 1]));
        let shifted = l_shr(ctx, ltmp1, 1);
        farray[j + 1] = round(ctx, shifted);

        i += 2;
    }
}

/// `vadState2`: everything VAD2 carries between half-frames.
///
/// Zeroed at reset, exactly as `vad2_reset` does by memsetting the struct —
/// which is why every field's useful initial value is zero and none is
/// special-cased here.
#[derive(Debug, Clone)]
pub struct Vad2State {
    pre_emp_mem: Word16,
    update_cnt: Word16,
    hyster_cnt: Word16,
    last_update_cnt: Word16,
    /// Long-term channel energy in dB, scaled 7,8.
    ch_enrg_long_db: [Word16; NUM_CHAN],
    frame_cnt: Word32,
    /// Channel energy, scaled 22,9 or 27,4 depending on `shift_state`.
    ch_enrg: [Word32; NUM_CHAN],
    /// Channel noise estimate, always scaled 22,9.
    ch_noise: [Word32; NUM_CHAN],
    last_normb_shift: Word16,
    /// Total signal-to-noise ratio in dB, scaled 7,8.
    tsnr: Word16,
    hangover: Word16,
    burstcount: Word16,
    /// Whether the previous frame forced a noise update.
    fupdate_flag: bool,
    /// Negative-SNR variance, scaled 7,8.
    neg_snr_var: Word16,
    /// Sensitivity bias derived from it, scaled 15,0.
    neg_snr_bias: Word16,
    /// 0 selects 22,9 scaling for `ch_enrg`, 1 selects 27,4.
    shift_state: usize,
    l_r0: Word32,
    l_rmax: Word32,
    /// Set when the LTP gain exceeds the mode's threshold.
    ltp_flag: bool,
}

impl Default for Vad2State {
    fn default() -> Self {
        Self::new()
    }
}

impl Vad2State {
    /// A reset detector: every field zero, per `vad2_reset`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pre_emp_mem: Word16(0),
            update_cnt: Word16(0),
            hyster_cnt: Word16(0),
            last_update_cnt: Word16(0),
            ch_enrg_long_db: [Word16(0); NUM_CHAN],
            frame_cnt: Word32(0),
            ch_enrg: [Word32(0); NUM_CHAN],
            ch_noise: [Word32(0); NUM_CHAN],
            last_normb_shift: Word16(0),
            tsnr: Word16(0),
            hangover: Word16(0),
            burstcount: Word16(0),
            fupdate_flag: false,
            neg_snr_var: Word16(0),
            neg_snr_bias: Word16(0),
            shift_state: 0,
            l_r0: Word32(0),
            l_rmax: Word32(0),
            ltp_flag: false,
        }
    }

    /// Feed the open-loop pitch analysis results this frame produced.
    ///
    /// `cod_amr.c` writes these before calling [`Self::ltp_flag_update`]; they
    /// are the only inputs VAD2 takes from the rest of the encoder.
    pub const fn set_ltp_energies(&mut self, l_r0: Word32, l_rmax: Word32) {
        self.l_r0 = l_r0;
        self.l_rmax = l_rmax;
    }

    /// TS 26.073 `LTP_flag_update`: set the LTP flag when the gain exceeds a
    /// mode-dependent threshold.
    ///
    /// `mode_index` is the mode's own index (0 = 4.75 … 7 = 12.2), matching
    /// the reference's `MR475`…`MR122` enumeration.
    pub fn ltp_flag_update(&mut self, ctx: &mut DspContext, mode_index: u8) {
        let thresh = match mode_index {
            // 0.55, 0.60 and 0.65 scaled 0,15.
            0 | 1 => Word16(18022),
            5 => Word16(19661),
            _ => Word16(21299),
        };
        let (hi1, lo1) = l_extract(self.l_r0);
        let ltmp = mpy_32_16(hi1, lo1, thresh);
        self.ltp_flag = l_sub(ctx, self.l_rmax, ltmp).0 > 0;
    }

    /// The detector's channel energies, for tests and diagnostics.
    #[must_use]
    pub const fn channel_energies(&self) -> &[Word32; NUM_CHAN] {
        &self.ch_enrg
    }

    /// The detector's channel noise estimates.
    #[must_use]
    pub const fn channel_noise(&self) -> &[Word32; NUM_CHAN] {
        &self.ch_noise
    }

    /// The long-term per-channel energy in dB.
    #[must_use]
    pub const fn channel_energy_long_db(&self) -> &[Word16; NUM_CHAN] {
        &self.ch_enrg_long_db
    }

    /// Counters and SNR state, in the order `nb_vad2_probe.c` prints them:
    /// `(tsnr, hangover, burstcount, update_cnt, hyster_cnt, negSNRvar,
    /// negSNRbias, shift_state)`.
    #[must_use]
    pub const fn counters(&self) -> (i16, i16, i16, i16, i16, i16, i16, i16) {
        (
            self.tsnr.0,
            self.hangover.0,
            self.burstcount.0,
            self.update_cnt.0,
            self.hyster_cnt.0,
            self.neg_snr_var.0,
            self.neg_snr_bias.0,
            if self.shift_state == 1 { 1 } else { 0 },
        )
    }
}

/// One half-frame of voice activity detection, TS 26.073 `vad2()`.
///
/// `farray` is 80 samples. The frame's decision is the OR of two successive
/// calls, which the encoder does — this returns the intermediate decision for
/// one half, exactly as the reference does.
// Same reasoning as `c_fft` for the casts; the length is the reference's own
// single function and splitting it would obscure the correspondence.
#[allow(
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::similar_names,
    clippy::needless_range_loop
)]
pub fn vad2(ctx: &mut DspContext, farray: &[Word16; FRM_LEN], st: &mut Vad2State) -> bool {
    let mut input_buffer = [Word16(0); FRM_LEN];
    let mut data_buffer = [Word16(0); FFT_LEN];

    st.frame_cnt = l_add(ctx, st.frame_cnt, Word32(1));

    let normb_shift = block_norm(ctx, farray, &mut input_buffer, FFT_HEADROOM);

    // Pre-emphasise into the FFT buffer at its DELAY offset.
    let shift = sub(ctx, st.last_normb_shift, Word16(normb_shift));
    st.pre_emp_mem = shr_r(ctx, st.pre_emp_mem, shift.0);
    st.last_normb_shift = Word16(normb_shift);

    let pre = mult(ctx, Word16(PRE_EMP_FAC), st.pre_emp_mem);
    data_buffer[DELAY] = add(ctx, input_buffer[0], pre);
    for j in 1..FRM_LEN {
        let pre = mult(ctx, Word16(PRE_EMP_FAC), input_buffer[j - 1]);
        data_buffer[DELAY + j] = add(ctx, input_buffer[j], pre);
    }
    st.pre_emp_mem = input_buffer[FRM_LEN - 1];

    r_fft(ctx, &mut data_buffer);

    // The block-norm shift decides which energy scaling the state uses.
    let mut state_change = false;
    if st.shift_state == 0 {
        if normb_shift <= -FFT_HEADROOM + 2 {
            state_change = true;
            st.shift_state = 1;
        }
    } else if normb_shift >= -FFT_HEADROOM + 5 {
        state_change = true;
        st.shift_state = 0;
    }
    if state_change {
        for i in 0..NUM_CHAN {
            st.ch_enrg[i] = l_shr(ctx, st.ch_enrg[i], STATE_CHANGE_SHIFT_R[st.shift_state]);
        }
    }

    // Channel energies, smoothed over time.
    let (alpha, one_m_alpha) = if l_sub(ctx, st.frame_cnt, Word32(1)).0 == 0 {
        (Word16(32767), Word16(0))
    } else {
        (Word16(CEE_SM_FAC), Word16(ONE_MINUS_CEE_SM_FAC))
    };

    for i in 0..NUM_CHAN {
        let mut lenrg = Word32(0);
        let (j1, j2) = (CH_TBL[i][0], CH_TBL[i][1]);
        for j in j1..=j2 {
            lenrg = l_mac(ctx, lenrg, data_buffer[2 * j], data_buffer[2 * j]);
            lenrg = l_mac(ctx, lenrg, data_buffer[2 * j + 1], data_buffer[2 * j + 1]);
        }
        // Denormalise and rescale 30,1 to the state's scaling.
        let shift = shl(ctx, Word16(normb_shift), 1);
        let shift = sub(ctx, shift, Word16(ENRG_NORM_SHIFT[st.shift_state]));
        let lenrg = l_shr_r(ctx, lenrg, shift.0);

        let tmp = mult(ctx, alpha, Word16(CH_TBL_SH[i]));
        let (hi1, lo1) = l_extract(lenrg);
        let ltmp = mpy_32_16(hi1, lo1, tmp);

        let (hi1, lo1) = l_extract(st.ch_enrg[i]);
        st.ch_enrg[i] = l_add(ctx, ltmp, mpy_32_16(hi1, lo1, one_m_alpha));
        if l_sub(
            ctx,
            st.ch_enrg[i],
            Word32(i32::from(MIN_CHAN_ENRG[st.shift_state])),
        )
        .0 < 0
        {
            st.ch_enrg[i] = Word32(i32::from(MIN_CHAN_ENRG[st.shift_state]));
        }
    }

    let mut ltce = Word32(0);
    for i in 0..NUM_CHAN {
        ltce = l_add(ctx, ltce, st.ch_enrg[i]);
    }

    // Spectral peak-to-average: sine waves are not valid at low frequencies,
    // so the search starts two channels up.
    let mut lpeak = Word32(0);
    for i in 2..NUM_CHAN {
        if l_sub(ctx, st.ch_enrg[i], lpeak).0 > 0 {
            lpeak = st.ch_enrg[i];
        }
    }
    // p2a > 10 dB is Lpeak > (10/16)*Ltce.
    let (hi1, lo1) = l_extract(ltce);
    let ltmp = mpy_32_16(hi1, lo1, Word16(20480));
    let p2a_flag = l_sub(ctx, lpeak, ltmp).0 > 0;

    // Seed the noise estimate from the first few frames.
    if l_sub(ctx, st.frame_cnt, Word32(4)).0 <= 0 {
        if p2a_flag {
            for i in 0..NUM_CHAN {
                st.ch_noise[i] = Word32(INE_NOISE_0);
            }
        } else {
            for i in 0..NUM_CHAN {
                if l_sub(ctx, st.ch_enrg[i], Word32(INE_NOISE[st.shift_state])).0 < 0 {
                    st.ch_noise[i] = Word32(INE_NOISE_0);
                } else if st.shift_state == 1 {
                    st.ch_noise[i] = l_shr(ctx, st.ch_enrg[i], STATE_CHANGE_SHIFT_R[0]);
                } else {
                    st.ch_noise[i] = st.ch_enrg[i];
                }
            }
        }
    }

    // Channel energies in dB, channel SNRs, and the voice metric sum.
    let mut ch_enrg_db = [Word16(0); NUM_CHAN];
    let mut ch_snr = [Word16(0); NUM_CHAN];
    let mut vm_sum = Word16(0);
    for i in 0..NUM_CHAN {
        ch_enrg_db[i] = fn10_log10(ctx, st.ch_enrg[i], FBITS[st.shift_state]);
        let ch_noise_db = fn10_log10(ctx, st.ch_noise[i], FRACTIONAL_BITS_0);
        ch_snr[i] = sub(ctx, ch_enrg_db[i], ch_noise_db);

        // Quantise the channel SNR in 3/8 dB steps, 7,8 => 15,0.
        let scaled = mult(ctx, Word16(21845), ch_snr[i]);
        let ch_snrq = shr_r(ctx, scaled, 6);
        let j = if sub(ctx, ch_snrq, Word16(89)).0 < 0 {
            if ch_snrq.0 > 0 {
                ch_snrq.0
            } else {
                0
            }
        } else {
            89
        };
        vm_sum = add(ctx, vm_sum, Word16(VM_TBL[j as usize]));
    }

    // Instantaneous frame SNR.
    let xt;
    if l_sub(ctx, st.frame_cnt, Word32(4)).0 <= 0 || st.fupdate_flag {
        // 96 - 22 - 10*log10(64), scaled 7,8.
        let tce_db = Word16(14320);
        st.neg_snr_var = Word16(0);
        st.neg_snr_bias = Word16(0);

        let mut ltne = Word32(0);
        for i in 0..NUM_CHAN {
            ltne = l_add(ctx, ltne, st.ch_noise[i]);
        }
        let tne_db = fn10_log10(ctx, ltne, FRACTIONAL_BITS_0);
        xt = sub(ctx, tce_db, tne_db);
        st.tsnr = xt;
    } else {
        // xt = 10*log10( sum(2.^(ch_snr*0.1*log2(10)))/length(ch_snr) )
        let mut ltmp1 = Word32(0);
        for &snr in &ch_snr {
            let prod = l_mult(ctx, snr, Word16(10885));
            let ltmp2 = l_shr(ctx, prod, 8);
            let (hi1, lo1) = l_extract(ltmp2);
            // 2^3 compensates for negative SNR.
            let hi1 = add(ctx, hi1, Word16(3));
            let term = pow2(ctx, hi1, lo1);
            ltmp1 = l_add(ctx, ltmp1, term);
        }
        // Average by 16, then undo the 2^3 compensation.
        xt = fn10_log10(ctx, ltmp1, 4 + 3);

        if sub(ctx, xt, st.tsnr).0 > 0 {
            // tsnr = 0.9*tsnr + 0.1*xt
            let a = l_mult(ctx, Word16(29491), st.tsnr);
            let b = l_mult(ctx, Word16(3277), xt);
            let sum = l_add(ctx, a, b);
            st.tsnr = round(ctx, sum);
        } else {
            let threshold = mult(ctx, Word16(20480), st.tsnr);
            if sub(ctx, xt, threshold).0 > 0 {
                // tsnr = 0.998*tsnr + 0.002*xt
                let a = l_mult(ctx, Word16(32702), st.tsnr);
                let b = l_mult(ctx, Word16(66), xt);
                let sum = l_add(ctx, a, b);
                st.tsnr = round(ctx, sum);
            }
        }
    }

    // Quantise the long-term SNR in 3 dB steps, clamped to 0..=19.
    let scaled = mult(ctx, st.tsnr, Word16(10923));
    // The reference writes this as two guarded assignments; the result is a
    // clamp to 0..=19 and the branch order carries no other effect.
    let tsnrq = shr(ctx, scaled, 8).0.clamp(0, 19);

    // Negative-SNR sensitivity bias.
    if xt.0 < 0 {
        // negSNRvar = 0.99*negSNRvar + 0.01*xt*xt, xt*xt is 14,17 so shift to 7,8.
        let sq = l_mult(ctx, xt, xt);
        let shifted = l_shl(ctx, sq, 7);
        let tmp = round(ctx, shifted);
        let a = l_mult(ctx, Word16(32440), st.neg_snr_var);
        let b = l_mult(ctx, Word16(328), tmp);
        let sum = l_add(ctx, a, b);
        st.neg_snr_var = round(ctx, sum);
        if sub(ctx, st.neg_snr_var, Word16(1024)).0 > 0 {
            st.neg_snr_var = Word16(1024);
        }
        // negSNRbias = max(12.0*(negSNRvar - 0.65), 0.0)
        let diff = sub(ctx, st.neg_snr_var, Word16(166));
        let scaled = shl(ctx, diff, 4);
        let tmp = mult_r(ctx, scaled, Word16(24576));
        st.neg_snr_bias = if tmp.0 < 0 {
            Word16(0)
        } else {
            shr(ctx, tmp, 8)
        };
    }

    // The decision itself: voice metric sum against an SNR-dependent threshold.
    let tmp = add(
        ctx,
        Word16(VM_THRESHOLD_TABLE[tsnrq as usize]),
        st.neg_snr_bias,
    );
    let ivad;
    if sub(ctx, vm_sum, tmp).0 > 0 {
        ivad = true;
        st.burstcount = add(ctx, st.burstcount, Word16(1));
        if sub(ctx, st.burstcount, Word16(BURSTCOUNT_TABLE[tsnrq as usize])).0 > 0 {
            st.hangover = Word16(HANGOVER_TABLE[tsnrq as usize]);
        }
    } else {
        st.burstcount = Word16(0);
        st.hangover = sub(ctx, st.hangover, Word16(1));
        if st.hangover.0 <= 0 {
            ivad = false;
            st.hangover = Word16(0);
        } else {
            ivad = true;
        }
    }

    // Log spectral deviation.
    let mut ch_enrg_dev = Word16(0);
    if l_sub(ctx, st.frame_cnt, Word32(1)).0 == 0 {
        st.ch_enrg_long_db = ch_enrg_db;
    } else {
        for i in 0..NUM_CHAN {
            let diff = sub(ctx, st.ch_enrg_long_db[i], ch_enrg_db[i]);
            let tmp = abs_s(ctx, diff);
            ch_enrg_dev = add(ctx, ch_enrg_dev, tmp);
        }
    }

    // Integration constant from instantaneous SNR: high SNR integrates slower.
    let tmp = sub(ctx, st.tsnr, xt);
    let (alpha, one_m_alpha) = if tmp.0 <= 0 || st.tsnr.0 <= 0 {
        (
            Word16(HIGH_ALPHA),
            Word16((32768i32 - i32::from(HIGH_ALPHA)) as i16),
        )
    } else if sub(ctx, tmp, st.tsnr).0 > 0 {
        (
            Word16(LOW_ALPHA),
            Word16((32768i32 - i32::from(LOW_ALPHA)) as i16),
        )
    } else {
        let ratio = div_s(tmp, st.tsnr);
        let scaled = mult(ctx, Word16(ALPHA_RANGE), ratio);
        let alpha = sub(ctx, Word16(HIGH_ALPHA), scaled);
        let one_m_alpha = sub(ctx, Word16(32767), alpha);
        (alpha, one_m_alpha)
    };

    for i in 0..NUM_CHAN {
        let ltmp1 = l_mult(ctx, one_m_alpha, ch_enrg_db[i]);
        let ltmp2 = l_mult(ctx, alpha, st.ch_enrg_long_db[i]);
        let sum = l_add(ctx, ltmp1, ltmp2);
        st.ch_enrg_long_db[i] = round(ctx, sum);
    }

    // Noise update flags.
    let mut update_flag = false;
    st.fupdate_flag = false;
    if sub(ctx, vm_sum, Word16(UPDATE_THLD)).0 <= 0 {
        if st.burstcount.0 == 0 {
            update_flag = true;
            st.update_cnt = Word16(0);
        }
    } else if l_sub(
        ctx,
        ltce,
        Word32(i32::from(NOISE_FLOOR_CHAN[st.shift_state])),
    )
    .0 > 0
        && sub(ctx, ch_enrg_dev, Word16(DEV_THLD)).0 < 0
        && !p2a_flag
        && !st.ltp_flag
    {
        st.update_cnt = add(ctx, st.update_cnt, Word16(1));
        if sub(ctx, st.update_cnt, Word16(UPDATE_CNT_THLD)).0 >= 0 {
            update_flag = true;
            st.fupdate_flag = true;
        }
    }

    if sub(ctx, st.update_cnt, st.last_update_cnt).0 == 0 {
        st.hyster_cnt = add(ctx, st.hyster_cnt, Word16(1));
    } else {
        st.hyster_cnt = Word16(0);
    }
    st.last_update_cnt = st.update_cnt;
    if sub(ctx, st.hyster_cnt, Word16(HYSTER_CNT_THLD)).0 > 0 {
        st.update_cnt = Word16(0);
    }

    // Conditionally update the channel noise estimates.
    if update_flag {
        // Noise is always state 0, so shift the energy down when in state 1.
        let tmp = if st.shift_state == 1 {
            STATE_CHANGE_SHIFT_R[0]
        } else {
            0
        };
        for i in 0..NUM_CHAN {
            let shifted = l_shr(ctx, st.ch_enrg[i], tmp);
            let (hi1, lo1) = l_extract(shifted);
            let ltmp = mpy_32_16(hi1, lo1, Word16(CNE_SM_FAC));

            let (hi1, lo1) = l_extract(st.ch_noise[i]);
            st.ch_noise[i] = l_add(ctx, ltmp, mpy_32_16(hi1, lo1, Word16(ONE_MINUS_CNE_SM_FAC)));
            if l_sub(ctx, st.ch_noise[i], Word32(MIN_NOISE_ENRG_0)).0 < 0 {
                st.ch_noise[i] = Word32(MIN_NOISE_ENRG_0);
            }
        }
    }

    ivad
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reference's own per-half-frame state over the committed DTX input,
    /// produced by `tools/nb_vad2_probe.c`. Regenerate with:
    ///
    /// ```text
    /// cc -O1 -I<ref>/c-code -o nb_vad2_probe tools/nb_vad2_probe.c \
    ///     <ref>/c-code/{vad2,r_fft,basicop2,oper_32b,log2,pow2,count}.c
    /// ./nb_vad2_probe src/codecs/amr/testdata/amrnb_dtx_input.pcm
    /// ```
    const REFERENCE_TRACE: &str = include_str!("../../testdata/amrnb_vad2_trace.txt");
    const INPUT_PCM: &[u8] = include_bytes!("../../testdata/amrnb_dtx_input.pcm");

    struct Expected {
        vad: bool,
        counters: (i16, i16, i16, i16, i16, i16, i16, i16),
        ch_enrg: [i32; NUM_CHAN],
        ch_noise: [i32; NUM_CHAN],
        ch_enrg_long_db: [i16; NUM_CHAN],
    }

    fn parse_trace() -> Vec<Expected> {
        REFERENCE_TRACE
            .lines()
            .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
            .map(|line| {
                let mut sections = line.split('|');
                let head: Vec<&str> = sections.next().expect("head").split_whitespace().collect();
                let counters: Vec<i16> = sections
                    .next()
                    .expect("counters")
                    .split_whitespace()
                    .map(|value| value.parse().expect("counter"))
                    .collect();
                let mut arrays = sections.map(|section| {
                    section
                        .split_whitespace()
                        .map(|value| value.parse::<i32>().expect("array entry"))
                        .collect::<Vec<i32>>()
                });
                let enrg = arrays.next().expect("ch_enrg");
                let noise = arrays.next().expect("ch_noise");
                let long_db = arrays.next().expect("ch_enrg_long_db");

                let mut ch_enrg = [0i32; NUM_CHAN];
                let mut ch_noise = [0i32; NUM_CHAN];
                let mut ch_enrg_long_db = [0i16; NUM_CHAN];
                ch_enrg.copy_from_slice(&enrg);
                ch_noise.copy_from_slice(&noise);
                for (slot, value) in ch_enrg_long_db.iter_mut().zip(long_db) {
                    *slot = i16::try_from(value).expect("dB fits a Word16");
                }

                Expected {
                    vad: head[1] == "1",
                    counters: (
                        counters[0],
                        counters[1],
                        counters[2],
                        counters[3],
                        counters[4],
                        counters[5],
                        counters[6],
                        counters[7],
                    ),
                    ch_enrg,
                    ch_noise,
                    ch_enrg_long_db,
                }
            })
            .collect()
    }

    /// Bit-exactness against TS 26.073's own `vad2()`, every half-frame, on
    /// the whole state rather than the decision.
    ///
    /// The decision alone is nearly vacuous — it is one bit, it appears
    /// nowhere in the bitstream, and VAD2's is the OR of two calls per frame,
    /// so a wrong half can be masked by its partner. A divergence in the
    /// channel energies localises to the FFT or the smoothing; one in the
    /// noise estimates localises to the update logic; one in the counters
    /// localises to the hangover machine.
    #[test]
    fn matches_the_reference_state_every_half_frame() {
        let expected = parse_trace();
        assert!(
            expected.len() > 250,
            "the committed trace should cover the whole fixture, got {}",
            expected.len()
        );

        let mut ctx = DspContext::default();
        let mut state = Vad2State::new();

        for (half, want) in expected.iter().enumerate() {
            let mut frame = [Word16(0); FRM_LEN];
            for (index, slot) in frame.iter_mut().enumerate() {
                let offset = (half * FRM_LEN + index) * 2;
                *slot = Word16(i16::from_le_bytes([
                    INPUT_PCM[offset],
                    INPUT_PCM[offset + 1],
                ]));
            }

            let got = vad2(&mut ctx, &frame, &mut state);

            assert_eq!(got, want.vad, "half-frame {half}: decision");
            assert_eq!(
                state.counters(),
                want.counters,
                "half-frame {half}: counters"
            );
            for i in 0..NUM_CHAN {
                assert_eq!(
                    state.channel_energies()[i].0,
                    want.ch_enrg[i],
                    "half-frame {half}: channel {i} energy"
                );
                assert_eq!(
                    state.channel_noise()[i].0,
                    want.ch_noise[i],
                    "half-frame {half}: channel {i} noise"
                );
                assert_eq!(
                    state.channel_energy_long_db()[i].0,
                    want.ch_enrg_long_db[i],
                    "half-frame {half}: channel {i} long-term dB"
                );
            }
        }
    }

    /// The trace has to contain both decisions, or the test above proves only
    /// that a constant matches a constant.
    #[test]
    fn the_reference_trace_is_not_degenerate() {
        let expected = parse_trace();
        let active = expected.iter().filter(|entry| entry.vad).count();
        assert!(
            active > 10 && active < expected.len() - 10,
            "the fixture must exercise both decisions: {active} active of {}",
            expected.len()
        );
    }
}
