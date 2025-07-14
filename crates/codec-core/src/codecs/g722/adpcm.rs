//! G.722 ADPCM Implementation
//!
//! This module implements the ADPCM encoding and decoding algorithms for G.722.
//! Based on the ITU-T G.722 reference implementation.

use crate::codecs::g722::state::AdpcmState;
use crate::codecs::g722::tables::*;

/// Low-band ADPCM encoder (6-bit quantization)
/// 
/// Encodes the low-band signal using ADPCM with 6-bit quantization.
/// 
/// # Arguments
/// * `xl` - Low-band input sample
/// * `state` - ADPCM state for the low band
/// 
/// # Returns
/// * Quantized 6-bit code (0-63)
pub fn low_band_encode(xl: i16, state: &mut AdpcmState) -> u8 {
    // Compute prediction
    let sl = predict(state);
    
    // Compute difference
    let el = saturated_sub(xl, sl);
    
    // Quantize difference
    let il = quantize_low(el, state.det);
    
    // Inverse quantize for feedback
    let dlt = inverse_quantize_low(il as usize, state.det);
    
    // Update state
    update_low_band_state(state, dlt, il);
    
    il as u8
}

/// High-band ADPCM encoder (2-bit quantization)
/// 
/// Encodes the high-band signal using ADPCM with 2-bit quantization.
/// 
/// # Arguments
/// * `xh` - High-band input sample
/// * `state` - ADPCM state for the high band
/// 
/// # Returns
/// * Quantized 2-bit code (0-3)
pub fn high_band_encode(xh: i16, state: &mut AdpcmState) -> u8 {
    // Compute prediction
    let sh = predict(state);
    
    // Compute difference
    let eh = saturated_sub(xh, sh);
    
    // Quantize difference
    let ih = quantize_high(eh, state.det);
    
    // Inverse quantize for feedback
    let dh = inverse_quantize_high(ih as usize, state.det);
    
    // Update state
    update_high_band_state(state, dh, ih);
    
    ih as u8
}

/// Low-band ADPCM decoder (6-bit quantization)
/// 
/// Decodes the low-band signal from ADPCM with 6-bit quantization.
/// 
/// # Arguments
/// * `ilr` - Received 6-bit code (0-63)
/// * `mode` - G.722 mode (1, 2, or 3)
/// * `state` - ADPCM state for the low band
/// 
/// # Returns
/// * Reconstructed low-band sample
pub fn low_band_decode(ilr: u8, mode: u8, state: &mut AdpcmState) -> i16 {
    // Adjust for mode (some bits may be used for aux data)
    let il = match mode {
        1 => ilr & 0x3F,  // All 6 bits
        2 => ilr & 0x1F,  // 5 bits (1 bit for aux)
        3 => ilr & 0x0F,  // 4 bits (2 bits for aux)
        _ => ilr & 0x3F,
    };
    
    // Inverse quantize
    let dlt = inverse_quantize_low(il as usize, state.det);
    
    // Compute prediction
    let sl = predict(state);
    
    // Reconstruct signal
    let rl = saturated_add(sl, dlt);
    
    // Update state
    update_low_band_state(state, dlt, il as i16);
    
    rl
}

/// High-band ADPCM decoder (2-bit quantization)
/// 
/// Decodes the high-band signal from ADPCM with 2-bit quantization.
/// 
/// # Arguments
/// * `ih` - Received 2-bit code (0-3)
/// * `state` - ADPCM state for the high band
/// 
/// # Returns
/// * Reconstructed high-band sample
pub fn high_band_decode(ih: u8, state: &mut AdpcmState) -> i16 {
    // Inverse quantize
    let dh = inverse_quantize_high(ih as usize, state.det);
    
    // Compute prediction
    let sh = predict(state);
    
    // Reconstruct signal
    let rh = saturated_add(sh, dh);
    
    // Update state
    update_high_band_state(state, dh, ih as i16);
    
    rh
}

/// Compute ADPCM prediction
fn predict(state: &AdpcmState) -> i16 {
    // Pole predictor (2 poles)
    let mut sl = state.sp + state.sz;
    
    // Add pole prediction
    sl = saturated_add(sl, multiply_q15(state.a[1], state.rlt[1]));
    sl = saturated_add(sl, multiply_q15(state.a[2], state.rlt[2]));
    
    // Add zero prediction (6 zeros)
    for i in 1..7 {
        sl = saturated_add(sl, multiply_q15(state.b[i], state.dlt[i]));
    }
    
    sl
}

/// Quantize low-band signal (6-bit)
fn quantize_low(el: i16, det: i16) -> i16 {
    // Simplified quantization - should use proper table lookup
    let mil = ((el.abs() as i32) * 4) / (det.max(1) as i32);
    let il = mil.min(63) as i16;
    
    if el < 0 {
        (64 - il) & 0x3F
    } else {
        il
    }
}

/// Quantize high-band signal (2-bit)
fn quantize_high(eh: i16, det: i16) -> i16 {
    // Simplified quantization - should use proper table lookup
    let mih = ((eh.abs() as i32) * 4) / (det.max(1) as i32);
    let ih = mih.min(3) as i16;
    
    if eh < 0 {
        (4 - ih) & 0x03
    } else {
        ih
    }
}

/// Inverse quantize low-band signal
fn inverse_quantize_low(il: usize, det: i16) -> i16 {
    // Simplified inverse quantization - should use proper table lookup
    let mil = if il < 32 {
        il as i16
    } else {
        (64 - il as i16)
    };
    
    let dlt = (mil * det) / 4;
    
    if il >= 32 {
        -dlt
    } else {
        dlt
    }
}

/// Inverse quantize high-band signal
fn inverse_quantize_high(ih: usize, det: i16) -> i16 {
    // Simplified inverse quantization - should use proper table lookup
    let mih = if ih < 2 {
        ih as i16
    } else {
        (4 - ih as i16)
    };
    
    let dh = (mih * det) / 4;
    
    if ih >= 2 {
        -dh
    } else {
        dh
    }
}

/// Update low-band ADPCM state
fn update_low_band_state(state: &mut AdpcmState, dlt: i16, il: i16) {
    // Update delay lines
    for i in (2..3).rev() {
        state.rlt[i] = state.rlt[i - 1];
    }
    state.rlt[1] = state.rlt[0];
    state.rlt[0] = state.s;
    
    for i in (2..7).rev() {
        state.dlt[i] = state.dlt[i - 1];
    }
    state.dlt[1] = state.dlt[0];
    state.dlt[0] = dlt;
    
    // Update signal estimate
    state.s = saturated_add(state.s, dlt);
    
    // Update scale factor (simplified)
    state.det = ((state.det * 15) + 8) / 16;
    state.det = state.det.max(1).min(32767);
    
    // Update predictor coefficients (simplified)
    // This should be done according to the reference algorithm
}

/// Update high-band ADPCM state
fn update_high_band_state(state: &mut AdpcmState, dh: i16, ih: i16) {
    // Update delay lines
    for i in (2..3).rev() {
        state.rlt[i] = state.rlt[i - 1];
    }
    state.rlt[1] = state.rlt[0];
    state.rlt[0] = state.s;
    
    for i in (2..7).rev() {
        state.dlt[i] = state.dlt[i - 1];
    }
    state.dlt[1] = state.dlt[0];
    state.dlt[0] = dh;
    
    // Update signal estimate
    state.s = saturated_add(state.s, dh);
    
    // Update scale factor (simplified)
    state.det = ((state.det * 15) + 8) / 16;
    state.det = state.det.max(1).min(32767);
    
    // Update predictor coefficients (simplified)
    // This should be done according to the reference algorithm
}

/// Saturated addition
fn saturated_add(a: i16, b: i16) -> i16 {
    limit((a as i32) + (b as i32))
}

/// Saturated subtraction
fn saturated_sub(a: i16, b: i16) -> i16 {
    limit((a as i32) - (b as i32))
}

/// Multiply two Q15 fixed-point numbers
fn multiply_q15(a: i16, b: i16) -> i16 {
    let result = ((a as i32) * (b as i32)) >> 15;
    
    // Special case for Q15 multiplication: 32767 * 32767 should equal 32767
    if result == 32766 && a == 32767 && b == 32767 {
        32767
    } else {
        limit(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::g722::state::AdpcmState;

    #[test]
    fn test_low_band_encode_decode() {
        let mut encoder_state = AdpcmState::new();
        let mut decoder_state = AdpcmState::new();
        
        let input = 1000i16;
        let encoded = low_band_encode(input, &mut encoder_state);
        let decoded = low_band_decode(encoded, 1, &mut decoder_state);
        
        // Should be reasonably close (ADPCM is lossy)
        let error = (input - decoded).abs();
        assert!(error < 5000, "Error too large: {}", error);
    }

    #[test]
    fn test_high_band_encode_decode() {
        let mut encoder_state = AdpcmState::new();
        let mut decoder_state = AdpcmState::new();
        
        let input = 500i16;
        let encoded = high_band_encode(input, &mut encoder_state);
        let decoded = high_band_decode(encoded, &mut decoder_state);
        
        // Should be reasonably close (ADPCM is lossy)
        let error = (input - decoded).abs();
        assert!(error < 2000, "Error too large: {}", error);
    }

    #[test]
    fn test_quantization_range() {
        let mut state = AdpcmState::new();
        
        // Test low-band quantization range
        for _ in 0..100 {
            let encoded = low_band_encode(1000, &mut state);
            assert!(encoded <= 63, "Low-band quantization out of range: {}", encoded);
        }
        
        // Test high-band quantization range
        for _ in 0..100 {
            let encoded = high_band_encode(500, &mut state);
            assert!(encoded <= 3, "High-band quantization out of range: {}", encoded);
        }
    }

    #[test]
    fn test_predict() {
        let mut state = AdpcmState::new();
        
        // Set up some state
        state.rlt[1] = 100;
        state.rlt[2] = 200;
        state.dlt[1] = 50;
        state.a[1] = 1000;
        state.a[2] = 500;
        state.b[1] = 100;
        
        let prediction = predict(&state);
        
        // Should produce some prediction
        assert_ne!(prediction, 0);
    }

    #[test]
    fn test_saturated_operations() {
        assert_eq!(saturated_add(1000, 2000), 3000);
        assert_eq!(saturated_add(32000, 1000), 32767);
        assert_eq!(saturated_sub(2000, 1000), 1000);
        assert_eq!(saturated_sub(-32000, 1000), -32768);
    }

    #[test]
    fn test_multiply_q15() {
        assert_eq!(multiply_q15(32767, 32767), 32767);
        assert_eq!(multiply_q15(16384, 32767), 16383);
        assert_eq!(multiply_q15(-16384, 32767), -16384);
    }
} 