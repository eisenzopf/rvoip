//! Annex B comfort-noise excitation synthesis.
//! Provenance: Annex B VAD/DTX/CNG behavior adapted from ITU G.729 Annex B reference routines.
//! Q-format: VAD features, SID parameters, and CNG gains use Q0/Q13/Q15 fixed-point domains.

use super::excitation_helpers::update_exc_err_l;
use super::excitation_subframe::synth_subframe;
use crate::constants::{L_FRAME, L_SUBFR};

/// Public function `calc_exc_rand`.
pub fn calc_exc_rand(
    cur_gain: i16,
    old_exc: &mut [i16],
    exc_index: usize,
    seed: &mut i16,
    flag_cod: bool,
    mut enc_taming: Option<&mut [i32; 4]>,
) {
    if cur_gain == 0 {
        for i in 0..L_FRAME {
            old_exc[exc_index + i] = 0;
        }
        if flag_cod {
            if let Some(l_exc_err) = enc_taming.as_deref_mut() {
                let gp = 0;
                let t0 = (L_SUBFR + 1) as i16;
                for _ in (0..L_FRAME).step_by(L_SUBFR) {
                    update_exc_err_l(l_exc_err, gp, t0);
                }
            }
        }
        return;
    }

    for cur_exc in (0..L_FRAME).step_by(L_SUBFR) {
        synth_subframe(
            cur_gain,
            old_exc,
            exc_index,
            cur_exc,
            seed,
            flag_cod,
            &mut enc_taming,
        );
    }
}
