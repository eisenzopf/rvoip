//! QMF (Quadrature Mirror Filter) Implementation
//!
//! This module implements the QMF analysis and synthesis filters for G.722.
//! Updated to use exact ITU-T reference implementation functions.

use crate::codecs::g722::state::G722State;
use crate::codecs::g722::reference::{limit, add, sub, l_mult, l_mac, l_shr, l_shl, l_add, l_sub, extract_h};

/// QMF filter coefficients for both transmission and reception
/// 
/// Exact values from ITU-T G.722 reference implementation g722_tables.c
/// Original: coef_qmf[24] = {3*2, -11*2, -11*2, 53*2, 12*2, -156*2, ...}
const COEF_QMF: [i16; 24] = [
    6, -22, -22, 106, 24, -312,
    64, 724, -420, -1610, 1902, 7752,
    7752, 1902, -1610, -420, 724, 64,
    -312, 24, 106, -22, -22, 6
];

/// ITU-T qmf_tx_buf function - QMF analysis (encoder) filter
/// 
/// Exact implementation from ITU-T G.722 reference funcg722.c
/// 
/// # Arguments
/// * `xin0` - First input sample
/// * `xin1` - Second input sample  
/// * `xl` - Output low-band sample
/// * `xh` - Output high-band sample
/// * `state` - G.722 state containing delay line
pub fn qmf_tx_buf(xin0: i16, xin1: i16, xl: &mut i16, xh: &mut i16, state: &mut G722State) {
    // ITU-T reference algorithm:
    
    // Saving past samples in delay line (shift and insert new samples)
    // *--(*delayx) = *(*xin)++; *--(*delayx) = *(*xin)++;
    for i in (2..24).rev() {
        state.qmf_tx_delay[i] = state.qmf_tx_delay[i - 2];
    }
    state.qmf_tx_delay[1] = xin0;
    state.qmf_tx_delay[0] = xin1;
    
    // QMF filtering
    let mut accuma = 0i32;
    let mut accumb = 0i32;
    
    // ITU-T exact multiply-accumulate operations
    accuma = l_mult(COEF_QMF[0], state.qmf_tx_delay[0]);
    accumb = l_mult(COEF_QMF[1], state.qmf_tx_delay[1]);
    
    // FOR (i = 1; i < 12; i++) - ITU-T exact MAC operations
    for i in 1..12 {
        let coef_idx = i * 2;
        let delay_idx = i * 2;
        accuma = l_mac(accuma, COEF_QMF[coef_idx], state.qmf_tx_delay[delay_idx]);
        accumb = l_mac(accumb, COEF_QMF[coef_idx + 1], state.qmf_tx_delay[delay_idx + 1]);
    }
    
    // ITU-T exact 32-bit add/sub operations with doubling
    let mut comp_low = l_add(accuma, accumb);
    comp_low = l_add(comp_low, comp_low);  // Double the result
    
    // ITU-T exact 32-bit sub and add operations  
    let mut comp_high = l_sub(accuma, accumb);
    comp_high = l_add(comp_high, comp_high);  // Double the result
    
    // ITU-T exact right shift and limit operations
    *xl = limit(l_shr(comp_low, 16));
    *xh = limit(l_shr(comp_high, 16));
}

/// ITU-T qmf_rx_buf function - QMF synthesis (decoder) filter
/// 
/// Exact implementation from ITU-T G.722 reference funcg722.c
/// 
/// # Arguments
/// * `rl` - Low-band input sample
/// * `rh` - High-band input sample
/// * `xout0` - Output first reconstructed sample
/// * `xout1` - Output second reconstructed sample
/// * `state` - G.722 state containing delay line
pub fn qmf_rx_buf(rl: i16, rh: i16, xout0: &mut i16, xout1: &mut i16, state: &mut G722State) {
    // ITU-T reference algorithm:
    
    // compute sum and difference from lower-band (rl) and higher-band (rh) signals
    // update delay line
    // *--(*delayx) = add (rl, rh); *--(*delayx) = sub (rl, rh);
    for i in (2..24).rev() {
        state.qmf_rx_delay[i] = state.qmf_rx_delay[i - 2];
    }
    state.qmf_rx_delay[1] = add(rl, rh);  // ITU-T saturated add
    state.qmf_rx_delay[0] = sub(rl, rh);  // ITU-T saturated sub
    
    // qmf_rx filtering
    let mut accuma = 0i32;
    let mut accumb = 0i32;
    
    // ITU-T exact multiply-accumulate operations
    accuma = l_mult(COEF_QMF[0], state.qmf_rx_delay[0]);
    accumb = l_mult(COEF_QMF[1], state.qmf_rx_delay[1]);
    
    // FOR (i = 1; i < 12; i++) - ITU-T exact MAC operations
    for i in 1..12 {
        let coef_idx = i * 2;
        let delay_idx = i * 2;
        accuma = l_mac(accuma, COEF_QMF[coef_idx], state.qmf_rx_delay[delay_idx]);
        accumb = l_mac(accumb, COEF_QMF[coef_idx + 1], state.qmf_rx_delay[delay_idx + 1]);
    }
    
    // ITU-T exact left shift operations
    let comp_low = l_shl(accuma, 4);
    let comp_high = l_shl(accumb, 4);
    
    // ITU-T exact high word extraction
    *xout0 = extract_h(comp_low);
    *xout1 = extract_h(comp_high);
}

/// QMF analysis for encoding (wrapper for ITU-T function)
/// 
/// # Arguments
/// * `sample0` - First input sample
/// * `sample1` - Second input sample  
/// * `state` - G.722 state
/// 
/// # Returns
/// * Tuple of (low_band, high_band) samples
pub fn qmf_analysis(sample0: i16, sample1: i16, state: &mut G722State) -> (i16, i16) {
    let mut xl = 0i16;
    let mut xh = 0i16;
    qmf_tx_buf(sample0, sample1, &mut xl, &mut xh, state);
    (xl, xh)
}

/// QMF synthesis for decoding (wrapper for ITU-T function)
/// 
/// # Arguments
/// * `xl` - Low-band sample
/// * `xh` - High-band sample
/// * `state` - G.722 state
/// 
/// # Returns
/// * Tuple of (sample0, sample1) reconstructed samples
pub fn qmf_synthesis(xl: i16, xh: i16, state: &mut G722State) -> (i16, i16) {
    let mut xout0 = 0i16;
    let mut xout1 = 0i16;
    qmf_rx_buf(xl, xh, &mut xout0, &mut xout1, state);
    (xout0, xout1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::g722::state::G722State;

    #[test]
    fn test_qmf_coefficients() {
        // Test that coefficients match ITU-T reference
        assert_eq!(COEF_QMF.len(), 24);
        assert_eq!(COEF_QMF[0], 6);    // 3*2
        assert_eq!(COEF_QMF[1], -22);  // -11*2
        assert_eq!(COEF_QMF[11], 7752); // 3876*2
        assert_eq!(COEF_QMF[12], 7752); // 3876*2
    }

    #[test]
    fn test_qmf_analysis_synthesis() {
        let mut state = G722State::new();
        
        // Test with some sample values
        let (xl, xh) = qmf_analysis(1000, 2000, &mut state);
        let (out0, out1) = qmf_synthesis(xl, xh, &mut state);
        
        // The values should be reasonable (QMF is not perfect reconstruction)
        assert!(xl.abs() < 32767);
        assert!(xh.abs() < 32767);
        assert!(out0.abs() < 32767);
        assert!(out1.abs() < 32767);
    }

    #[test]
    fn test_qmf_delay_line_management() {
        let mut state = G722State::new();
        
        // Fill delay line with known values
        qmf_analysis(100, 200, &mut state);
        qmf_analysis(300, 400, &mut state);
        
        // Check that delay line is properly shifted
        assert_eq!(state.qmf_tx_delay[0], 400);
        assert_eq!(state.qmf_tx_delay[1], 300);
        assert_eq!(state.qmf_tx_delay[2], 200);
        assert_eq!(state.qmf_tx_delay[3], 100);
    }
} 