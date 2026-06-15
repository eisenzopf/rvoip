//! Provenance: Fixed codebook search/decode adapted from ITU G.729 ACELP algebraic codebook routines.
//! Q-format: Correlation buffers and pulse gains use Q13/Q15 with 32-bit fixed-point accumulators.

use crate::constants::{DIM_RR, L_SUBFR, NB_POS};

pub(crate) const ONE_HALF: i16 = 16384;
pub(crate) const ONE_FOURTH: i16 = 8192;
pub(crate) const ONE_EIGHTH: i16 = 4096;
pub(crate) const ONE_SIXTEENTH: i16 = 2048;

pub(crate) struct D4iWorkspace {
    pub(crate) sign_dn: [i16; L_SUBFR],
    pub(crate) sign_dn_inv: [i16; L_SUBFR],
    pub(crate) rr_mod: [i16; DIM_RR],
    pub(crate) tmp_vect: [i16; NB_POS],
    pub(crate) psk: i16,
    pub(crate) alpk: i16,
    pub(crate) ptr_rri0i3_i4: usize,
    pub(crate) ptr_rri1i3_i4: usize,
    pub(crate) ptr_rri2i3_i4: usize,
    pub(crate) ptr_rri3i3_i4: usize,
    pub(crate) ip0: i16,
    pub(crate) ip1: i16,
    pub(crate) ip2: i16,
    pub(crate) ip3: i16,
}

impl Default for D4iWorkspace {
    fn default() -> Self {
        Self {
            sign_dn: [0; L_SUBFR],
            sign_dn_inv: [0; L_SUBFR],
            rr_mod: [0; DIM_RR],
            tmp_vect: [0; NB_POS],
            psk: 0,
            alpk: 0,
            ptr_rri0i3_i4: 0,
            ptr_rri1i3_i4: 0,
            ptr_rri2i3_i4: 0,
            ptr_rri3i3_i4: 0,
            ip0: 0,
            ip1: 0,
            ip2: 0,
            ip3: 0,
        }
    }
}

pub(crate) struct Candidate {
    pub(crate) sq: i16,
    pub(crate) alp: i16,
    pub(crate) ip0: i16,
    pub(crate) ip1: i16,
    pub(crate) ip2: i16,
    pub(crate) ip3: i16,
}
