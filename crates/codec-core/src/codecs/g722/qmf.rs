//! G.722 QMF (Quadrature Mirror Filter) Implementation
//!
//! This module implements the QMF analysis and synthesis filters used in G.722.
//! Based on the ITU-T G.722 reference implementation.

use crate::codecs::g722::tables::{QMF_COEFFS, limit};
use crate::codecs::g722::state::G722State;

/// QMF analysis filter (encoder)
/// 
/// Splits the input signal into low and high frequency bands.
/// Processes two input samples and produces one low-band and one high-band sample.
/// 
/// # Arguments
/// * `xin0` - First input sample
/// * `xin1` - Second input sample  
/// * `state` - G.722 state containing the QMF delay line
/// 
/// # Returns
/// * `(xl, xh)` - Low-band and high-band samples
pub fn qmf_analysis(xin0: i16, xin1: i16, state: &mut G722State) -> (i16, i16) {
    let delay = state.qmf_tx_delay_mut();
    
    // Shift delay line first (move older samples towards the end)
    for i in 0..22 {
        delay[23 - i] = delay[21 - i];
    }
    
    // Insert new samples at the beginning
    delay[0] = xin0;
    delay[1] = xin1;
    
    // QMF filtering
    let mut accum_a = 0i64;
    let mut accum_b = 0i64;
    
    for i in 0..12 {
        accum_a += (delay[i * 2] as i64) * (QMF_COEFFS[i * 2] as i64);
        accum_b += (delay[i * 2 + 1] as i64) * (QMF_COEFFS[i * 2 + 1] as i64);
    }
    
    // Compute low and high band outputs
    let comp_low = (accum_a + accum_b) * 2;
    let comp_high = (accum_a - accum_b) * 2;
    
    let xl = limit((comp_low >> 16) as i32);
    let xh = limit((comp_high >> 16) as i32);
    
    (xl, xh)
}

/// QMF synthesis filter (decoder)
/// 
/// Reconstructs the time-domain signal from low and high frequency bands.
/// Processes one low-band and one high-band sample and produces two output samples.
/// 
/// # Arguments
/// * `rl` - Low-band sample
/// * `rh` - High-band sample
/// * `state` - G.722 state containing the QMF delay line
/// 
/// # Returns
/// * `(xout1, xout2)` - Two reconstructed output samples
pub fn qmf_synthesis(rl: i16, rh: i16, state: &mut G722State) -> (i16, i16) {
    let delay = state.qmf_rx_delay_mut();
    
    // Shift delay line first (move older samples towards the end)
    for i in 0..22 {
        delay[23 - i] = delay[21 - i];
    }
    
    // Compute sum and difference from lower-band (rl) and higher-band (rh) signals
    delay[0] = saturated_sub(rl, rh);
    delay[1] = saturated_add(rl, rh);
    
    // QMF filtering
    let mut accum_a = 0i64;
    let mut accum_b = 0i64;
    
    for i in 0..12 {
        accum_a += (delay[i * 2] as i64) * (QMF_COEFFS[i * 2] as i64);
        accum_b += (delay[i * 2 + 1] as i64) * (QMF_COEFFS[i * 2 + 1] as i64);
    }
    
    // Compute output samples with right shift by 10 for proper scaling
    let comp_low = accum_a >> 10;
    let comp_high = accum_b >> 10;
    
    let xout1 = limit(comp_low as i32);
    let xout2 = limit(comp_high as i32);
    
    (xout1, xout2)
}

/// QMF synthesis filter optimized version (decoder)
/// 
/// Alternative implementation that doesn't shift the delay line in memory.
/// Used for performance optimization in some cases.
/// 
/// # Arguments
/// * `rl` - Low-band sample
/// * `rh` - High-band sample
/// * `delay` - QMF delay line (external management)
/// * `output` - Output buffer for the two samples
pub fn qmf_synthesis_buf(rl: i16, rh: i16, delay: &mut [i16], output: &mut [i16]) {
    if delay.len() < 24 || output.len() < 2 {
        return;
    }
    
    // Shift delay line first (move older samples towards the end)
    for i in 0..22 {
        delay[23 - i] = delay[21 - i];
    }
    
    // Compute sum and difference from lower-band (rl) and higher-band (rh) signals
    delay[0] = saturated_sub(rl, rh);
    delay[1] = saturated_add(rl, rh);
    
    // QMF filtering
    let mut accum_a = 0i64;
    let mut accum_b = 0i64;
    
    for i in 0..12 {
        accum_a += (delay[i * 2] as i64) * (QMF_COEFFS[i * 2] as i64);
        accum_b += (delay[i * 2 + 1] as i64) * (QMF_COEFFS[i * 2 + 1] as i64);
    }
    
    // Compute output samples with right shift by 10 for proper scaling
    let comp_low = accum_a >> 10;
    let comp_high = accum_b >> 10;
    
    output[0] = limit(comp_low as i32);
    output[1] = limit(comp_high as i32);
}

/// Saturated addition of two i16 values
fn saturated_add(a: i16, b: i16) -> i16 {
    limit((a as i32) + (b as i32))
}

/// Saturated subtraction of two i16 values
fn saturated_sub(a: i16, b: i16) -> i16 {
    limit((a as i32) - (b as i32))
}

/// Extract high 16 bits from 32-bit value (equivalent to right shift by 16)
fn extract_high(value: i64) -> i16 {
    // Cast to i32 first to handle 32-bit values properly, then extract high 16 bits
    let val32 = value as i32;
    (val32 >> 16) as i16
}

/// Reset QMF delay lines
pub fn reset_qmf_delays(state: &mut G722State) {
    state.qmf_tx_delay = [0; 24];
    state.qmf_rx_delay = [0; 24];
}

/// Get frequency response of QMF filter (for testing/validation)
pub fn get_qmf_response(frequency: f32, sample_rate: f32) -> f32 {
    let omega = 2.0 * std::f32::consts::PI * frequency / sample_rate;
    let mut response = 0.0f32;
    
    for (i, &coeff) in QMF_COEFFS.iter().enumerate() {
        response += (coeff as f32) * (omega * (i as f32)).cos();
    }
    
    response.abs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::g722::state::G722State;

    #[test]
    fn test_qmf_analysis_basic() {
        let mut state = G722State::new();
        
        // Test with simple input
        let (xl, xh) = qmf_analysis(1000, 2000, &mut state);
        
        // QMF should produce some output (exact values depend on filter)
        assert_ne!(xl, 0);
        assert_ne!(xh, 0);
    }

    #[test]
    fn test_qmf_synthesis_basic() {
        let mut state = G722State::new();
        
        // Prime the filter with some samples first
        let _ = qmf_synthesis(500, 300, &mut state);
        let _ = qmf_synthesis(400, 200, &mut state);
        let _ = qmf_synthesis(300, 100, &mut state);
        
        // Now test with simple input - should produce some output
        let (xout1, xout2) = qmf_synthesis(500, 300, &mut state);
        
        // QMF should produce some output after priming
        assert_ne!(xout1, 0);
        assert_ne!(xout2, 0);
    }

    #[test]
    fn test_qmf_delay_line_shift() {
        let mut state = G722State::new();
        
        // Fill delay line with test pattern
        for i in 0..24 {
            state.qmf_tx_delay[i] = i as i16;
        }
        
        // Process samples
        let _ = qmf_analysis(100, 200, &mut state);
        
        // Check that delay line was shifted correctly
        assert_eq!(state.qmf_tx_delay[0], 100);
        assert_eq!(state.qmf_tx_delay[1], 200);
        assert_eq!(state.qmf_tx_delay[2], 0);  // Should be shifted from position 0
        assert_eq!(state.qmf_tx_delay[3], 1);  // Should be shifted from position 1
    }

    #[test]
    fn test_qmf_synthesis_buf() {
        let mut delay = [0i16; 24];
        let mut output = [0i16; 2];
        
        // Prime the filter with some samples first
        qmf_synthesis_buf(500, 300, &mut delay, &mut output);
        qmf_synthesis_buf(400, 200, &mut delay, &mut output);
        qmf_synthesis_buf(300, 100, &mut delay, &mut output);
        
        // Now test with simple input - should produce some output
        qmf_synthesis_buf(500, 300, &mut delay, &mut output);
        
        // Should produce non-zero output after priming
        assert_ne!(output[0], 0);
        assert_ne!(output[1], 0);
    }

    #[test]
    fn test_saturated_add() {
        assert_eq!(saturated_add(1000, 2000), 3000);
        assert_eq!(saturated_add(32000, 1000), 32767);  // Should saturate
        assert_eq!(saturated_add(-32000, -1000), -32768);  // Should saturate
    }

    #[test]
    fn test_saturated_sub() {
        assert_eq!(saturated_sub(2000, 1000), 1000);
        assert_eq!(saturated_sub(-32000, 1000), -32768);  // Should saturate
        assert_eq!(saturated_sub(32000, -1000), 32767);   // Should saturate
    }

    #[test]
    fn test_extract_high() {
        assert_eq!(extract_high(0x12345678), 0x1234);
        assert_eq!(extract_high(0x0000FFFF), 0x0000);
        assert_eq!(extract_high(0xFFFF0000), -1);  // Sign extension
    }

    #[test]
    fn test_reset_qmf_delays() {
        let mut state = G722State::new();
        
        // Fill delays with non-zero values
        state.qmf_tx_delay[0] = 100;
        state.qmf_rx_delay[0] = 200;
        
        reset_qmf_delays(&mut state);
        
        // Should be reset to zero
        assert_eq!(state.qmf_tx_delay[0], 0);
        assert_eq!(state.qmf_rx_delay[0], 0);
    }

    #[test]
    fn test_qmf_coefficients() {
        // Test that QMF coefficients are symmetric (property of QMF filters)
        let len = QMF_COEFFS.len();
        for i in 0..len/2 {
            assert_eq!(QMF_COEFFS[i], QMF_COEFFS[len - 1 - i]);
        }
    }

    #[test]
    fn test_qmf_frequency_response() {
        // Test frequency response at DC (should be non-zero)
        let dc_response = get_qmf_response(0.0, 16000.0);
        assert!(dc_response > 0.0);
        
        // Test frequency response at 1/4 Nyquist (should be non-zero)
        let quarter_nyquist_response = get_qmf_response(2000.0, 16000.0);
        assert!(quarter_nyquist_response > 0.0);
        
        // Test frequency response at 1/2 Nyquist (should be non-zero)
        let half_nyquist_response = get_qmf_response(4000.0, 16000.0);
        assert!(half_nyquist_response > 0.0);
    }
} 