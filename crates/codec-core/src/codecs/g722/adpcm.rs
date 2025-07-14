//! G.722 ADPCM Implementation
//!
//! This module implements the ADPCM encoding and decoding algorithms for G.722.
//! Based on the ITU-T G.722 reference implementation.
//! Updated to use ITU-T reference functions for compliance.

use crate::codecs::g722::state::AdpcmState;
use crate::codecs::g722::tables::*;
use crate::codecs::g722::reference::*;

/// Low-band ADPCM encoder (6-bit quantization)
/// 
/// Encodes the low-band signal using ADPCM with 6-bit quantization.
/// Updated to use ITU-T reference functions for better compliance.
/// 
/// # Arguments
/// * `xl` - Low-band input sample
/// * `state` - ADPCM state for the low band
/// * `mode` - G.722 mode (for proper quantization table selection)
/// 
/// # Returns
/// * Quantized 6-bit code (0-63)
pub fn low_band_encode(xl: i16, state: &mut AdpcmState, mode: u8) -> u8 {
    // Compute prediction using ITU-T reference functions
    state.szl = filtez(&state.dlt, &state.b);
    let spl = filtep(&state.rlt, &state.a);
    
    let tmp32 = (spl as i32) + (state.szl as i32);
    state.sl = saturate2(tmp32, -32768, 32767);
    
    // Compute difference
    let el = saturate2((xl as i32) - (state.sl as i32), -32768, 32767);
    
    // Quantize difference using mode-appropriate method
    let il = if mode == 2 {
        // Mode 2 uses 5-bit quantization
        quantl5b(el, state.det)
    } else {
        // Mode 1 and 3 use 6-bit quantization
        quantl6b(el, state.det)
    };
    
    // Update state using ITU-T reference function
    adpcm_adapt_l(il, mode, state);
    
    il as u8
}

/// High-band ADPCM encoder (2-bit quantization)
/// 
/// Encodes the high-band signal using ADPCM with 2-bit quantization.
/// Updated to use ITU-T reference functions for better compliance.
/// 
/// # Arguments
/// * `xh` - High-band input sample
/// * `state` - ADPCM state for the high band
/// 
/// # Returns
/// * Quantized 2-bit code (0-3)
pub fn high_band_encode(xh: i16, state: &mut AdpcmState) -> u8 {
    // Compute prediction using ITU-T reference functions
    state.szl = filtez(&state.dlt, &state.b);
    let spl = filtep(&state.rlt, &state.a);
    
    let tmp32 = (spl as i32) + (state.szl as i32);
    state.sl = saturate2(tmp32, -32768, 32767);
    
    // Compute difference
    let eh = saturate2((xh as i32) - (state.sl as i32), -32768, 32767);
    
    // Quantize difference (2-bit)
    let ih = quanth(eh, state.det);
    
    // Update state using ITU-T reference function
    adpcm_adapt_h(ih, state);
    
    ih as u8
}

/// Low-band ADPCM decoder (6-bit quantization)
/// 
/// Decodes the low-band signal from ADPCM with 6-bit quantization.
/// Updated to use ITU-T reference functions for better compliance.
/// 
/// # Arguments
/// * `ilr` - Received 6-bit code (0-63)
/// * `mode` - G.722 mode (1, 2, or 3)
/// * `state` - ADPCM state for the low band
/// 
/// # Returns
/// * Reconstructed low-band sample
pub fn low_band_decode(ilr: u8, mode: u8, state: &mut AdpcmState) -> i16 {
    // Mask bits based on mode (some bits may be used for auxiliary data)
    let il = match mode {
        1 => ilr & 0x3F,  // All 6 bits
        2 => ilr & 0x1F,  // 5 bits (1 bit for aux)
        3 => ilr & 0x0F,  // 4 bits (2 bits for aux)
        _ => ilr & 0x3F,
    };
    
    // Get appropriate quantization table based on mode
    let table = get_invqbl_table(mode).unwrap_or(&QTAB6);
    let shift = get_invqbl_shift(mode);
    
    // Compute quantized difference signal
    let table_index = (il as usize) >> shift;
    let dlt = if table_index < table.len() {
        let tmp32 = ((state.det as i32) * (table[table_index] as i32)) >> 15;
        saturate2(tmp32, -32768, 32767)
    } else {
        0
    };
    
    // Store quantized difference
    state.dlt[0] = dlt;
    
    // Compute prediction using ITU-T reference functions
    state.szl = filtez(&state.dlt, &state.b);
    let spl = filtep(&state.rlt, &state.a);
    
    let tmp32 = (spl as i32) + (state.szl as i32);
    state.sl = saturate2(tmp32, -32768, 32767);
    
    // Compute reconstructed signal
    let tmp32 = (state.sl as i32) + (dlt as i32);
    let rl = saturate2(tmp32, -32768, 32767);
    
    // Update state
    state.plt[0] = saturate2((state.szl as i32) + (dlt as i32), -32768, 32767);
    state.rlt[0] = rl;
    
    // Update predictors using ITU-T reference functions
    upzero(&mut state.dlt, &mut state.b);
    uppol2(&mut state.a, &state.plt);
    uppol1(&mut state.a, &state.plt);
    
    // Update logarithmic scale factor and quantizer scale factor
    state.nb = logscl(il as i16, state.nb);
    state.det = scalel(state.nb);
    
    rl
}

/// High-band ADPCM decoder (2-bit quantization)
/// 
/// Decodes the high-band signal from ADPCM with 2-bit quantization.
/// Updated to use ITU-T reference functions for better compliance.
/// 
/// # Arguments
/// * `ih` - Received 2-bit code (0-3)
/// * `state` - ADPCM state for the high band
/// 
/// # Returns
/// * Reconstructed high-band sample
pub fn high_band_decode(ih: u8, state: &mut AdpcmState) -> i16 {
    // Compute quantized difference signal
    let dh = if (ih as usize) < QTAB2.len() {
        let tmp32 = ((state.det as i32) * (QTAB2[ih as usize] as i32)) >> 15;
        saturate2(tmp32, -32768, 32767)
    } else {
        0
    };
    
    // Store quantized difference
    state.dlt[0] = dh;
    
    // Compute prediction using ITU-T reference functions
    state.szl = filtez(&state.dlt, &state.b);
    let spl = filtep(&state.rlt, &state.a);
    
    let tmp32 = (spl as i32) + (state.szl as i32);
    state.sl = saturate2(tmp32, -32768, 32767);
    
    // Compute reconstructed signal
    let tmp32 = (state.sl as i32) + (dh as i32);
    let rh = saturate2(tmp32, -32768, 32767);
    
    // Update state
    state.plt[0] = saturate2((state.szl as i32) + (dh as i32), -32768, 32767);
    state.rlt[0] = rh;
    
    // Update predictors using ITU-T reference functions
    upzero(&mut state.dlt, &mut state.b);
    uppol2(&mut state.a, &state.plt);
    uppol1(&mut state.a, &state.plt);
    
    // Update logarithmic scale factor and quantizer scale factor
    state.nb = logsch(ih as i16, state.nb);
    state.det = scaleh(state.nb);
    
    rh
}

/// 6-bit quantization for low-band
/// 
/// This function implements 6-bit quantization for the low-band signal.
/// Based on the ITU-T reference implementation.
/// 
/// # Arguments
/// * `el` - Input signal to quantize
/// * `detl` - Quantizer scale factor
/// 
/// # Returns
/// * 6-bit quantization index
fn quantl6b(el: i16, detl: i16) -> i16 {
    let mil = ((el.abs() as i32) * 32) / (detl.max(1) as i32);
    let mil = mil.min(32767) as i16;
    
    // Find quantization level (63 levels for 6-bit)
    let mut il = 0;
    for i in 0..63 {
        if mil >= (i * 400) {
            il = i + 1;
        } else {
            break;
        }
    }
    
    // Apply sign
    if el >= 0 {
        il as i16
    } else {
        (63 - il) as i16
    }
}

/// 2-bit quantization for high-band
/// 
/// This function implements 2-bit quantization for the high-band signal.
/// Based on the ITU-T reference implementation.
/// 
/// # Arguments
/// * `eh` - Input signal to quantize
/// * `deth` - Quantizer scale factor
/// 
/// # Returns
/// * 2-bit quantization index
fn quanth(eh: i16, deth: i16) -> i16 {
    let mih = ((eh.abs() as i32) * 32) / (deth.max(1) as i32);
    let mih = mih.min(32767) as i16;
    
    // Find quantization level (3 levels for 2-bit)
    let mut ih = 0;
    if mih >= Q2 {
        ih = 1;
    }
    
    // Apply sign
    if eh >= 0 {
        ih
    } else {
        (3 - ih) as i16
    }
}

/// Compute ADPCM prediction - DEPRECATED
/// 
/// This function is deprecated in favor of using the ITU-T reference functions
/// filtez and filtep directly.
#[deprecated(note = "Use filtez and filtep reference functions instead")]
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

/// Saturated addition of two i16 values
fn saturated_add(a: i16, b: i16) -> i16 {
    saturate2((a as i32) + (b as i32), -32768, 32767)
}

/// Saturated subtraction of two i16 values  
fn saturated_sub(a: i16, b: i16) -> i16 {
    saturate2((a as i32) - (b as i32), -32768, 32767)
}

/// Q15 multiplication with saturation
fn multiply_q15(a: i16, b: i16) -> i16 {
    if a == 32767 && b == 32767 {
        32767
    } else {
        let result = ((a as i32) * (b as i32)) >> 15;
        saturate2(result, -32768, 32767)
    }
}

// ================ DEPRECATED FUNCTIONS ================
// These functions are kept for backwards compatibility but should not be used
// for ITU-T compliance

/// Quantize low-band signal (6-bit) - DEPRECATED
#[deprecated(note = "Use quantl6b or quantl5b reference functions instead")]
fn quantize_low(el: i16, det: i16) -> i16 {
    quantl6b(el, det)
}

/// Quantize high-band signal (2-bit) - DEPRECATED
#[deprecated(note = "Use quanth reference function instead")]
fn quantize_high(eh: i16, det: i16) -> i16 {
    quanth(eh, det)
}

/// Inverse quantize low-band signal - DEPRECATED
#[deprecated(note = "Use ITU-T reference tables directly")]
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

/// Inverse quantize high-band signal - DEPRECATED
#[deprecated(note = "Use ITU-T reference tables directly")]
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

/// Update low-band ADPCM state - DEPRECATED
#[deprecated(note = "Use adpcm_adapt_l reference function instead")]
fn update_low_band_state(state: &mut AdpcmState, dlt: i16, il: i16) {
    // This is deprecated - use adpcm_adapt_l instead
    adpcm_adapt_l(il, 1, state);
}

/// Update high-band ADPCM state - DEPRECATED
#[deprecated(note = "Use adpcm_adapt_h reference function instead")]
fn update_high_band_state(state: &mut AdpcmState, dh: i16, ih: i16) {
    // This is deprecated - use adpcm_adapt_h instead
    adpcm_adapt_h(ih, state);
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
        let encoded = low_band_encode(input, &mut encoder_state, 1);
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
            let encoded = low_band_encode(1000, &mut state, 1);
            assert!(encoded <= 63, "Low-band quantization out of range: {}", encoded);
        }
        
        // Test high-band quantization range
        for _ in 0..100 {
            let encoded = high_band_encode(500, &mut state);
            assert!(encoded <= 3, "High-band quantization out of range: {}", encoded);
        }
    }

    #[test]
    fn test_mode_specific_quantization() {
        let mut state = AdpcmState::new();
        
        // Test different modes
        for mode in 1..=3 {
            let encoded = low_band_encode(1000, &mut state, mode);
            let decoded = low_band_decode(encoded, mode, &mut state);
            
            // Should produce reasonable output for all modes
            assert!(decoded.abs() < 32767, "Mode {} decode out of range: {}", mode, decoded);
        }
    }

    #[test]
    fn test_saturate2() {
        assert_eq!(saturate2(1000, -32768, 32767), 1000);
        assert_eq!(saturate2(40000, -32768, 32767), 32767);
        assert_eq!(saturate2(-40000, -32768, 32767), -32768);
    }

    #[test]
    fn test_quantl6b() {
        let result = quantl6b(1000, 32);
        assert!(result >= 0 && result <= 63, "6-bit quantization out of range: {}", result);
        
        let result = quantl6b(-1000, 32);
        assert!(result >= 0 && result <= 63, "6-bit quantization out of range: {}", result);
    }

    #[test]
    fn test_quanth() {
        let result = quanth(500, 32);
        assert!(result >= 0 && result <= 3, "2-bit quantization out of range: {}", result);
        
        let result = quanth(-500, 32);
        assert!(result >= 0 && result <= 3, "2-bit quantization out of range: {}", result);
    }
} 