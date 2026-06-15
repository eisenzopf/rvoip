//! Annex B bitstream-to-parameter unpack helpers.
//! Provenance: Frame encode/decode pipeline derived from ITU G.729 Annex A/B reference flow.
//! Q-format: Speech, excitation, and LPC paths follow Q0/Q12/Q13/Q15 fixed-point stages.

use crate::bitstream::itu_params::{BITSNO, BITSNO2};
use crate::constants::{BIT_0, BIT_1, PRM_SIZE};

pub(super) fn bin2int(no_of_bits: i16, bits: &[u16], bit_offset: &mut usize) -> i16 {
    let mut value = 0i16;
    for _ in 0..no_of_bits {
        value <<= 1;
        let bit = bits.get(*bit_offset).copied().unwrap_or(BIT_0 as u16);
        if bit as i16 == BIT_1 {
            value = value.wrapping_add(1);
        }
        *bit_offset += 1;
    }
    value
}

pub(super) fn bits2prm_ld8k(bits: &[u16; 80]) -> [i16; PRM_SIZE] {
    let mut prm = [0i16; PRM_SIZE];
    let mut off = 0usize;
    for i in 0..PRM_SIZE {
        prm[i] = bin2int(BITSNO[i] as i16, bits, &mut off);
    }
    prm
}

pub(super) fn bits2prm_sid(bits: &[u16]) -> [i16; 4] {
    let mut prm = [0i16; 4];
    let mut off = 0usize;
    for i in 0..4 {
        prm[i] = bin2int(BITSNO2[i] as i16, bits, &mut off);
    }
    prm
}
