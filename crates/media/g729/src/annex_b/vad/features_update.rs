//! Provenance: Annex B VAD/DTX/CNG behavior adapted from ITU G.729 Annex B reference routines.
//! Q-format: VAD features, SID parameters, and CNG gains use Q0/Q13/Q15 fixed-point domains.

use super::VadState;
use crate::constants::M;
use crate::dsp::arith::{abs_s, add, extract_h, sub};
use crate::dsp::arith32::{l_mac, l_mult};
use crate::dsp::types::{DspContext, Word16};
use crate::tables::vad::{INIT_COUNT, INIT_FRAME, NOISE, VOICE};

#[allow(clippy::too_many_arguments)]
pub(super) fn refine_marker(
    state: &mut VadState,
    ctx: &mut DspContext,
    rc: i16,
    lsf: &[i16; M],
    frm_count: i16,
    prev_marker: i16,
    pprev_marker: i16,
    energy: i16,
    energy_low: i16,
    sd: i16,
    zc: i16,
    marker_in: i16,
) -> i16 {
    let mut marker = marker_in;

    if sub(ctx, Word16(frm_count), Word16(INIT_FRAME)).0 >= 0 {
        if sub(ctx, Word16(frm_count), Word16(INIT_FRAME)).0 == 0 {
            let li = state.less_count as usize;
            let mut acc0 = l_mult(
                ctx,
                Word16(state.mean_e),
                Word16(crate::tables::sid::FACTOR_FX[li]),
            );
            acc0 = crate::dsp::shift::l_shl(ctx, acc0, crate::tables::sid::SHIFT_FX[li]);
            state.mean_e = extract_h(acc0).0;

            acc0 = l_mult(
                ctx,
                Word16(state.mean_szc),
                Word16(crate::tables::sid::FACTOR_FX[li]),
            );
            acc0 = crate::dsp::shift::l_shl(ctx, acc0, crate::tables::sid::SHIFT_FX[li]);
            state.mean_szc = extract_h(acc0).0;

            for idx in 0..M {
                acc0 = l_mult(
                    ctx,
                    Word16(state.mean_lsf[idx]),
                    Word16(crate::tables::sid::FACTOR_FX[li]),
                );
                acc0 = crate::dsp::shift::l_shl(ctx, acc0, crate::tables::sid::SHIFT_FX[li]);
                state.mean_lsf[idx] = extract_h(acc0).0;
            }

            state.mean_se = sub(ctx, Word16(state.mean_e), Word16(2048)).0;
            state.mean_sle = sub(ctx, Word16(state.mean_e), Word16(2458)).0;
        }

        let dse = sub(ctx, Word16(state.mean_se), Word16(energy)).0;
        let dsle = sub(ctx, Word16(state.mean_sle), Word16(energy_low)).0;
        let dszc = sub(ctx, Word16(state.mean_szc), Word16(zc)).0;

        if sub(ctx, Word16(energy), Word16(3072)).0 < 0 {
            marker = NOISE;
        } else {
            marker = super::decision::make_dec_impl(dsle, dse, sd, dszc);
        }

        state.v_flag = 0;
        if prev_marker == VOICE
            && marker == NOISE
            && add(ctx, Word16(dse), Word16(410)).0 < 0
            && sub(ctx, Word16(energy), Word16(3072)).0 > 0
        {
            marker = VOICE;
            state.v_flag = 1;
        }

        if state.flag == 1 {
            let dprev = sub(ctx, Word16(state.prev_energy), Word16(energy)).0;
            let abs_dprev = abs_s(ctx, Word16(dprev)).0;
            let smooth_ok = sub(ctx, Word16(abs_dprev), Word16(614)).0 <= 0;
            if pprev_marker == VOICE && prev_marker == VOICE && marker == NOISE && smooth_ok {
                state.count_ext = add(ctx, Word16(state.count_ext), Word16(1)).0;
                marker = VOICE;
                state.v_flag = 1;
                if sub(ctx, Word16(state.count_ext), Word16(4)).0 <= 0 {
                    state.flag = 1;
                } else {
                    state.count_ext = 0;
                    state.flag = 0;
                }
            }
        } else {
            state.flag = 1;
        }

        if marker == NOISE {
            state.count_sil = add(ctx, Word16(state.count_sil), Word16(1)).0;
        }

        if marker == VOICE && sub(ctx, Word16(state.count_sil), Word16(10)).0 > 0 && {
            let e_delta = sub(ctx, Word16(energy), Word16(state.prev_energy)).0;
            sub(ctx, Word16(e_delta), Word16(614)).0 <= 0
        } {
            marker = NOISE;
            state.count_sil = 0;
        }

        if marker == VOICE {
            state.count_sil = 0;
        }

        let e_minus_614 = sub(ctx, Word16(energy), Word16(614)).0;
        let below_mean_se = sub(ctx, Word16(e_minus_614), Word16(state.mean_se)).0 < 0;

        if below_mean_se
            && sub(ctx, Word16(frm_count), Word16(128)).0 > 0
            && state.v_flag == 0
            && sub(ctx, Word16(rc), Word16(19661)).0 < 0
        {
            marker = NOISE;
        }

        if below_mean_se
            && sub(ctx, Word16(rc), Word16(24576)).0 < 0
            && sub(ctx, Word16(sd), Word16(83)).0 < 0
        {
            state.count_update = add(ctx, Word16(state.count_update), Word16(1)).0;

            let (coef, c_coef, coefzc, c_coefzc, coefsd, c_coefsd) =
                if sub(ctx, Word16(state.count_update), Word16(INIT_COUNT)).0 < 0 {
                    (24576, 8192, 26214, 6554, 19661, 13017)
                } else if sub(ctx, Word16(state.count_update), Word16(INIT_COUNT + 10)).0 < 0 {
                    (31130, 1638, 30147, 2621, 21299, 11469)
                } else if sub(ctx, Word16(state.count_update), Word16(INIT_COUNT + 20)).0 < 0 {
                    (31785, 983, 30802, 1966, 22938, 9830)
                } else if sub(ctx, Word16(state.count_update), Word16(INIT_COUNT + 30)).0 < 0 {
                    (32440, 328, 31457, 1311, 24576, 8192)
                } else if sub(ctx, Word16(state.count_update), Word16(INIT_COUNT + 40)).0 < 0 {
                    (32604, 164, 32440, 328, 24576, 8192)
                } else {
                    (32604, 164, 32702, 66, 24576, 8192)
                };

            let mut acc0 = l_mult(ctx, Word16(coef), Word16(state.mean_se));
            acc0 = l_mac(ctx, acc0, Word16(c_coef), Word16(energy));
            state.mean_se = extract_h(acc0).0;

            acc0 = l_mult(ctx, Word16(coef), Word16(state.mean_sle));
            acc0 = l_mac(ctx, acc0, Word16(c_coef), Word16(energy_low));
            state.mean_sle = extract_h(acc0).0;

            acc0 = l_mult(ctx, Word16(coefzc), Word16(state.mean_szc));
            acc0 = l_mac(ctx, acc0, Word16(c_coefzc), Word16(zc));
            state.mean_szc = extract_h(acc0).0;

            for idx in 0..M {
                acc0 = l_mult(ctx, Word16(coefsd), Word16(state.mean_lsf[idx]));
                acc0 = l_mac(ctx, acc0, Word16(c_coefsd), Word16(lsf[idx]));
                state.mean_lsf[idx] = extract_h(acc0).0;
            }
        }

        if sub(ctx, Word16(frm_count), Word16(128)).0 > 0
            && ((sub(ctx, Word16(state.mean_se), Word16(state.min)).0 < 0
                && sub(ctx, Word16(sd), Word16(83)).0 < 0)
                || sub(ctx, Word16(state.mean_se), Word16(state.min)).0 > 2048)
        {
            state.mean_se = state.min;
            state.count_update = 0;
        }
    }

    marker
}
