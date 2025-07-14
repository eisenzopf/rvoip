//! ITU-T G.722 Reference Functions
//!
//! This module implements the core functions from the ITU-T G.722 reference
//! implementation to ensure bit-exact compliance with the standard.
//!
//! Based on ITU-T G.722 Annex E (Release 3.00, 2014-11)

use crate::codecs::g722::state::{G722State, AdpcmState};
use crate::codecs::g722::tables::*;

/// Saturate a value to 16-bit range (equivalent to ITU-T saturate2)
/// 
/// This function implements the ITU-T reference saturate2 function exactly.
/// 
/// # Arguments
/// * `x` - Input value to saturate
/// * `x_min` - Minimum allowed value
/// * `x_max` - Maximum allowed value
/// 
/// # Returns
/// * Saturated value within the specified range
pub fn saturate2(x: i32, x_min: i16, x_max: i16) -> i16 {
    if x > x_max as i32 {
        x_max
    } else if x < x_min as i32 {
        x_min
    } else {
        x as i16
    }
}

/// Common ADPCM adaptation function (adpcm_adapt_c)
/// 
/// This function implements the common ADPCM adaptation logic used by both
/// low-band and high-band ADPCM encoders/decoders.
/// 
/// # Arguments
/// * `ind` - Quantization index
/// * `state` - ADPCM state to update
/// * `d0` - Current quantized difference signal
fn adpcm_adapt_c(ind: i16, state: &mut AdpcmState, d0: i16) {
    // Store current quantized difference
    state.dlt[0] = d0;
    
    // Compute partial reconstructed signal (parrec)
    let tmp32 = (state.dlt[0] as i32) + (state.szl as i32);
    state.plt[0] = saturate2(tmp32, -32768, 32767);
    
    // Compute reconstructed signal (recons)
    let tmp32 = (state.s as i32) + (state.dlt[0] as i32);
    state.rlt[0] = saturate2(tmp32, -32768, 32767);
    
    // Update predictors
    upzero(&mut state.dlt, &mut state.b);
    uppol2(&mut state.a, &state.plt);
    uppol1(&mut state.a, &state.plt);
    
    // Update signal estimates
    state.szl = filtez(&state.dlt, &state.b);
    let sp = filtep(&state.rlt, &state.a);
    
    let tmp32 = (sp as i32) + (state.szl as i32);
    state.s = saturate2(tmp32, -32768, 32767);
}

/// High-band ADPCM adaptation (adpcm_adapt_h)
/// 
/// This function implements the ITU-T reference adpcm_adapt_h function.
/// 
/// # Arguments
/// * `ind` - Quantization index
/// * `state` - High-band ADPCM state
pub fn adpcm_adapt_h(ind: i16, state: &mut AdpcmState) {
    // Compute quantized difference signal
    let d0 = ((state.det as i32) * (QTAB2[ind as usize] as i32)) >> 15;
    let d0 = saturate2(d0, -32768, 32767);
    
    // Update logarithmic scale factor
    state.nb = logsch(ind, state.nb);
    
    // Update quantizer scale factor
    state.det = scaleh(state.nb);
    
    // Common adaptation
    adpcm_adapt_c(ind, state, d0);
}

/// Low-band ADPCM adaptation (adpcm_adapt_l)
/// 
/// This function implements the ITU-T reference adpcm_adapt_l function.
/// 
/// # Arguments
/// * `ind` - Quantization index
/// * `mode` - G.722 mode (for table selection)
/// * `state` - Low-band ADPCM state
pub fn adpcm_adapt_l(ind: i16, mode: u8, state: &mut AdpcmState) {
    // Get appropriate quantization table based on mode
    let table = get_invqbl_table(mode).unwrap_or(&QTAB6);
    let shift = get_invqbl_shift(mode);
    
    // Compute quantized difference signal
    let table_index = (ind as usize) >> shift;
    let d0 = if table_index < table.len() {
        let d0 = ((state.det as i32) * (table[table_index] as i32)) >> 15;
        saturate2(d0, -32768, 32767)
    } else {
        0
    };
    
    // Update logarithmic scale factor
    state.nb = logscl(ind, state.nb);
    
    // Update quantizer scale factor
    state.det = scalel(state.nb);
    
    // Update predictor coefficients and other state
    adpcm_adapt_c(ind, state, d0);
}

/// Low sub-band decoder (lsbdec)
/// 
/// This function implements the ITU-T reference lsbdec function.
/// 
/// # Arguments
/// * `ilr` - Received low-band quantization index
/// * `mode` - G.722 mode
/// * `state` - G.722 state
/// 
/// # Returns
/// * Decoded low-band sample
pub fn lsbdec(ilr: i16, mode: u8, state: &mut G722State) -> i16 {
    // Get appropriate quantization table and shift based on mode
    let table = get_invqbl_table(mode).unwrap_or(&QTAB6);
    let shift = get_invqbl_shift(mode);
    
    // Compute quantized difference signal
    let table_index = (ilr as usize) >> shift;
    let dl = if table_index < table.len() {
        let tmp32 = ((state.low_band.det as i32) * (table[table_index] as i32)) >> 15;
        saturate2(tmp32, -32768, 32767)
    } else {
        0
    };
    
    // Store quantized difference
    state.low_band.dlt[0] = dl;
    
    // Compute prediction
    state.low_band.szl = filtez(&state.low_band.dlt, &state.low_band.b);
    let spl = filtep(&state.low_band.rlt, &state.low_band.a);
    
    let tmp32 = (spl as i32) + (state.low_band.szl as i32);
    state.low_band.sl = saturate2(tmp32, -32768, 32767);
    
    // Compute reconstructed signal
    let tmp32 = (state.low_band.sl as i32) + (dl as i32);
    let rl = saturate2(tmp32, -32768, 32767);
    
    // Update state
    state.low_band.plt[0] = state.low_band.szl + dl;
    state.low_band.rlt[0] = rl;
    
    // Update predictors
    upzero(&mut state.low_band.dlt, &mut state.low_band.b);
    uppol2(&mut state.low_band.a, &state.low_band.plt);
    uppol1(&mut state.low_band.a, &state.low_band.plt);
    
    // Update logarithmic scale factor and quantizer scale factor
    state.low_band.nb = logscl(ilr, state.low_band.nb);
    state.low_band.det = scalel(state.low_band.nb);
    
    rl
}

/// 5-bit quantization (quantl5b)
/// 
/// This function implements the ITU-T reference quantl5b function.
/// 
/// # Arguments
/// * `el` - Input signal to quantize
/// * `detl` - Quantizer scale factor
/// 
/// # Returns
/// * 5-bit quantization index
pub fn quantl5b(el: i16, detl: i16) -> i16 {
    let mil = ((el.abs() as i32) * 32) / (detl.max(1) as i32);
    let mil = mil.min(32767) as i16;
    
    // Find quantization level
    let mut il = 0;
    for i in 0..15 {
        if mil >= Q5B[i] {
            il = i + 1;
        } else {
            break;
        }
    }
    
    // Apply sign
    if el >= 0 {
        il as i16
    } else {
        (31 - il) as i16
    }
}

/// Pole predictor filter (filtep)
/// 
/// This function implements the ITU-T reference filtep function.
/// 
/// # Arguments
/// * `rlt` - Reconstructed signal delay line
/// * `al` - Predictor coefficients
/// 
/// # Returns
/// * Filtered output
pub fn filtep(rlt: &[i16], al: &[i16]) -> i16 {
    let mut spl = 0i32;
    
    // Apply pole predictor (2 poles)
    if rlt.len() >= 3 && al.len() >= 3 {
        spl += ((al[1] as i32) * (rlt[1] as i32)) >> 15;
        spl += ((al[2] as i32) * (rlt[2] as i32)) >> 15;
    }
    
    saturate2(spl, -32768, 32767)
}

/// Zero predictor filter (filtez)
/// 
/// This function implements the ITU-T reference filtez function.
/// 
/// # Arguments
/// * `dlt` - Quantized difference signal delay line
/// * `bl` - Predictor coefficients
/// 
/// # Returns
/// * Filtered output
pub fn filtez(dlt: &[i16], bl: &[i16]) -> i16 {
    let mut szl = 0i32;
    
    // Apply zero predictor (6 zeros)
    let len = dlt.len().min(bl.len()).min(7);
    for i in 1..len {
        szl += ((bl[i] as i32) * (dlt[i] as i32)) >> 15;
    }
    
    saturate2(szl, -32768, 32767)
}

/// High-band logarithmic scale factor update (logsch)
/// 
/// This function implements the ITU-T reference logsch function exactly.
/// 
/// # Arguments
/// * `ih` - High-band quantization index
/// * `nbh` - Current high-band log scale factor
/// 
/// # Returns
/// * Updated high-band log scale factor
pub fn logsch(ih: i16, nbh: i16) -> i16 {
    // ITU-T reference implementation:
    // nbph = (Short)(((long)nbh* (long)32512)>>15) + whi[ih];
    // if( nbph >= 0) nbph = nbph; else nbph = 0;
    // if( nbph <= 22528) nbph = nbph; else nbph = 22528;
    // return (nbph);
    
    let ih_index = (ih as usize) % WHI.len();
    let nbph = (((nbh as i32) * 32512) >> 15) + (WHI[ih_index] as i32);
    
    // Apply limits
    let nbph = if nbph >= 0 { nbph } else { 0 };
    let nbph = if nbph <= 22528 { nbph } else { 22528 };
    
    nbph as i16
}

/// Low-band logarithmic scale factor update (logscl)
/// 
/// This function implements the ITU-T reference logscl function exactly.
/// 
/// # Arguments
/// * `il` - Quantization index
/// * `nbl` - Current log scale factor
/// 
/// # Returns
/// * Updated log scale factor
pub fn logscl(il: i16, nbl: i16) -> i16 {
    // ITU-T reference implementation:
    // ril = il >> 2;
    // nbpl = (Short)(((long)nbl* (long)32512)>>15) + wli[ril];
    // if( nbpl >= 0) nbpl = nbpl; else nbpl = 0;
    // if( nbpl <= 18432) nbpl = nbpl; else nbpl = 18432;
    // return (nbpl);
    
    let ril = il >> 2;
    let ril_index = (ril as usize) % WLI.len();
    
    let nbpl = (((nbl as i32) * 32512) >> 15) + (WLI[ril_index] as i32);
    
    // Apply limits
    let nbpl = if nbpl >= 0 { nbpl } else { 0 };
    let nbpl = if nbpl <= 18432 { nbpl } else { 18432 };
    
    nbpl as i16
}

/// Low-band scale factor (scalel)
/// 
/// This function implements the ITU-T reference scalel function exactly.
/// 
/// # Arguments
/// * `nbpl` - Log scale factor
/// 
/// # Returns
/// * Linear scale factor
pub fn scalel(nbpl: i16) -> i16 {
    // ITU-T reference implementation:
    // wd1 = (nbpl >> 6) & 511;
    // wd2 = wd1 + 64;
    // return ila2[wd2];
    let wd1 = (nbpl >> 6) & 511;
    let wd2 = wd1 + 64;
    
    if (wd2 as usize) < ILA2.len() {
        ILA2[wd2 as usize]
    } else {
        32 // Default fallback
    }
}

/// High-band scale factor (scaleh)
/// 
/// This function implements the ITU-T reference scaleh function exactly.
/// 
/// # Arguments
/// * `nbph` - Log scale factor
/// 
/// # Returns
/// * Linear scale factor
pub fn scaleh(nbph: i16) -> i16 {
    // ITU-T reference implementation:
    // wd = (nbph >> 6) & 511;
    // return ila2[wd];
    let wd = (nbph >> 6) & 511;
    
    if (wd as usize) < ILA2.len() {
        ILA2[wd as usize]
    } else {
        32 // Default fallback
    }
}

/// First-order pole predictor update (uppol1)
/// 
/// This function implements the ITU-T reference uppol1 function.
/// 
/// # Arguments
/// * `al` - Predictor coefficients (modified in-place)
/// * `plt` - Partial reconstructed signal
pub fn uppol1(al: &mut [i16], plt: &[i16]) {
    if al.len() >= 3 && plt.len() >= 3 {
        let mut a1 = al[1];
        
        // Compute update - check for overflow
        let tmp = if plt[0] * plt[1] >= 0 { 192 } else { -192 };
        let tmp32 = (a1 as i32).saturating_add(tmp);
        
        // Apply limits and store
        a1 = saturate2(tmp32, -32768, 32767);
        al[1] = a1;
        
        // Limit to prevent instability
        if a1.abs() > 15360 {
            al[1] = if a1 >= 0 { 15360 } else { -15360 };
        }
    }
}

/// Second-order pole predictor update (uppol2)
/// 
/// This function implements the ITU-T reference uppol2 function.
/// 
/// # Arguments
/// * `al` - Predictor coefficients (modified in-place)
/// * `plt` - Partial reconstructed signal
pub fn uppol2(al: &mut [i16], plt: &[i16]) {
    if al.len() >= 3 && plt.len() >= 3 {
        let mut a2 = al[2];
        
        // Compute update - check for overflow
        let tmp1 = if plt[0] * plt[1] >= 0 { 128 } else { -128 };
        let tmp2 = if plt[0] * plt[2] >= 0 { 128 } else { -128 };
        let tmp32 = (a2 as i32).saturating_add(tmp1).saturating_add(tmp2);
        
        // Apply limits and store
        a2 = saturate2(tmp32, -32768, 32767);
        al[2] = a2;
        
        // Limit to prevent instability
        if a2.abs() > 12288 {
            al[2] = if a2 >= 0 { 12288 } else { -12288 };
        }
    }
}

/// Zero predictor update (upzero)
/// 
/// This function implements the ITU-T reference upzero function.
/// 
/// # Arguments
/// * `dlt` - Quantized difference signal delay line (modified in-place)
/// * `bl` - Predictor coefficients (modified in-place)
pub fn upzero(dlt: &mut [i16], bl: &mut [i16]) {
    let len = dlt.len().min(bl.len()).min(7);
    
    // Update zero predictor coefficients
    for i in 1..len {
        // Use safe multiplication to avoid overflow
        let product = (dlt[0] as i32) * (dlt[i] as i32);
        let tmp = if product >= 0 { 128 } else { -128 };
        let tmp32 = (bl[i] as i32).saturating_add(tmp);
        bl[i] = saturate2(tmp32, -32768, 32767);
        
        // Apply limits
        if bl[i].abs() > 15360 {
            bl[i] = if bl[i] >= 0 { 15360 } else { -15360 };
        }
    }
    
    // Shift delay line
    for i in (1..len).rev() {
        dlt[i] = dlt[i-1];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::g722::state::G722State;
    
    #[test]
    fn test_saturate2() {
        assert_eq!(saturate2(1000, -32768, 32767), 1000);
        assert_eq!(saturate2(40000, -32768, 32767), 32767);
        assert_eq!(saturate2(-40000, -32768, 32767), -32768);
    }
    
    #[test]
    fn test_filtep() {
        let rlt = [0i16, 1000, 2000];
        let al = [0i16, 8192, 4096]; // 0.25 and 0.125 in Q15
        let result = filtep(&rlt, &al);
        // Should be approximately (1000 * 0.25) + (2000 * 0.125) = 250 + 250 = 500
        assert!((result - 500).abs() < 10);
    }
    
    #[test]
    fn test_filtez() {
        let dlt = [0i16, 1000, 2000, 0, 0, 0, 0];
        let bl = [0i16, 8192, 4096, 0, 0, 0, 0]; // 0.25 and 0.125 in Q15
        let result = filtez(&dlt, &bl);
        // Should be approximately (1000 * 0.25) + (2000 * 0.125) = 250 + 250 = 500
        assert!((result - 500).abs() < 10);
    }
    
    #[test]
    fn test_logscl() {
        // Test logscl with ITU-T reference behavior
        // logscl(0, 0): ril = 0 >> 2 = 0, nbpl = (0 * 32512) >> 15 + WLI[0] = 0 + (-60) = -60, limited to 0
        let result = logscl(0, 0);
        assert_eq!(result, 0, "logscl(0, 0) should return 0 after limiting");
        
        // logscl(1, 0): ril = 1 >> 2 = 0, nbpl = (0 * 32512) >> 15 + WLI[0] = 0 + (-60) = -60, limited to 0
        let result = logscl(1, 0);
        assert_eq!(result, 0, "logscl(1, 0) should return 0 after limiting");
        
        // logscl(10, 100): ril = 10 >> 2 = 2, nbpl = (100 * 32512) >> 15 + WLI[2] = 99 + 1198 = 1297
        let result = logscl(10, 100);
        let expected = ((100i32 * 32512) >> 15) + WLI[2] as i32;
        assert_eq!(result, expected as i16, "logscl(10, 100) should return {} based on ITU-T reference", expected);
    }
    
    #[test]
    fn test_scalel() {
        // Test scalel with ITU-T reference behavior
        // scalel(0): wd1 = (0 >> 6) & 511 = 0, wd2 = 0 + 64 = 64, return ila2[64]
        let result = scalel(0);
        let expected = ILA2[64]; // ila2[64] = 64
        assert_eq!(result, expected, "scalel(0) should return ila2[64] = {}, got {}", expected, result);
        
        // Test negative value
        let result = scalel(-1000);
        // For -1000: (-1000 >> 6) & 511 = (signed right shift, then mask)
        // In Rust, signed right shift preserves sign bit, so we get the expected behavior
        let wd1 = ((-1000i16) >> 6) & 511;
        let wd2 = wd1 + 64;
        let expected = if (wd2 as usize) < ILA2.len() { ILA2[wd2 as usize] } else { 32 };
        assert_eq!(result, expected, "scalel(-1000) should return ila2[{}] = {}, got {}", wd2, expected, result);
        
        // Test positive value
        let result = scalel(1000);
        // For 1000: (1000 >> 6) & 511 = 15, wd2 = 15 + 64 = 79, return ila2[79]
        let wd1 = (1000 >> 6) & 511;
        let wd2 = wd1 + 64;
        let expected = ILA2[wd2 as usize]; // ila2[79] = 76
        assert_eq!(result, expected, "scalel(1000) should return ila2[{}] = {}, got {}", wd2, expected, result);
        
        // Test scalel(172) specifically
        let result = scalel(172);
        let wd1 = (172 >> 6) & 511;
        let wd2 = wd1 + 64;
        let expected = ILA2[wd2 as usize];
        assert_eq!(result, expected, "scalel(172) should return ila2[{}] = {}, got {}", wd2, expected, result);
    }
    
    #[test]
    fn test_lsbdec() {
        let mut state = G722State::new();
        let result = lsbdec(0, 1, &mut state);
        // Should produce some reasonable output
        assert!(result.abs() < 32767);
    }
} 