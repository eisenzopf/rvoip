//! ITU-T G.729A LPC Analysis Module
//!
//! This module provides LPC (Linear Predictive Coding) analysis functions
//! based on the ITU-T G.729A reference implementation.

use crate::codecs::g729a::types::*;
use crate::codecs::g729a::basic_ops::*;

/// Hamming window for LPC analysis (L_WINDOW=240 samples)
/// From ITU reference TAB_LD8A.C: hamwindow[L_WINDOW]
pub const HAMWINDOW: [Word16; L_WINDOW] = [
    2621,  2623,  2629,  2638,  2651,  2668,  2689,  2713,  2741,  2772,
    2808,  2847,  2890,  2936,  2986,  3040,  3097,  3158,  3223,  3291,
    3363,  3438,  3517,  3599,  3685,  3774,  3867,  3963,  4063,  4166,
    4272,  4382,  4495,  4611,  4731,  4853,  4979,  5108,  5240,  5376,
    5514,  5655,  5800,  5947,  6097,  6250,  6406,  6565,  6726,  6890,
    7057,  7227,  7399,  7573,  7750,  7930,  8112,  8296,  8483,  8672,
    8863,  9057,  9252,  9450,  9650,  9852, 10055, 10261, 10468, 10677,
   10888, 11101, 11315, 11531, 11748, 11967, 12187, 12409, 12632, 12856,
   13082, 13308, 13536, 13764, 13994, 14225, 14456, 14688, 14921, 15155,
   15389, 15624, 15859, 16095, 16331, 16568, 16805, 17042, 17279, 17516,
   17754, 17991, 18228, 18465, 18702, 18939, 19175, 19411, 19647, 19882,
   20117, 20350, 20584, 20816, 21048, 21279, 21509, 21738, 21967, 22194,
   22420, 22644, 22868, 23090, 23311, 23531, 23749, 23965, 24181, 24394,
   24606, 24816, 25024, 25231, 25435, 25638, 25839, 26037, 26234, 26428,
   26621, 26811, 26999, 27184, 27368, 27548, 27727, 27903, 28076, 28247,
   28415, 28581, 28743, 28903, 29061, 29215, 29367, 29515, 29661, 29804,
   29944, 30081, 30214, 30345, 30472, 30597, 30718, 30836, 30950, 31062,
   31170, 31274, 31376, 31474, 31568, 31659, 31747, 31831, 31911, 31988,
   32062, 32132, 32198, 32261, 32320, 32376, 32428, 32476, 32521, 32561,
   32599, 32632, 32662, 32688, 32711, 32729, 32744, 32755, 32763, 32767,
   32767, 32763, 32755, 32744, 32729, 32711, 32688, 32662, 32632, 32599,
   32561, 32521, 32476, 32428, 32376, 32320, 32261, 32198, 32132, 32062,
   31988, 31911, 31831, 31747, 31659, 31568, 31474, 31376, 31274, 31170,
   31062, 30950, 30836, 30718, 30597, 30472, 30345, 30214, 30081, 29944,
];

/// Extract from a 32 bit integer two 16 bit DPF (Double Precision Format)
/// Based on ITU reference OPER_32B.C: L_Extract()
pub fn l_extract(l_32: Word32, hi: &mut Word16, lo: &mut Word16) {
    *hi = extract_h(l_32);
    *lo = extract_l(l_msu(l_shr(l_32, 1), *hi, 16384)); // lo = L_32>>1
}

/// Autocorrelation function
/// Based on ITU reference LPC.C: Autocorr()
pub fn autocorr(
    x: &[Word16],      // Input signal
    m: Word16,         // LPC order
    r_h: &mut [Word16], // Autocorrelations (msb)
    r_l: &mut [Word16], // Autocorrelations (lsb)
) {
    let mut y = [0i16; L_WINDOW];
    let mut sum: Word32;
    let mut norm: Word16;
    let mut overflow: bool;
    
    // Windowing of signal
    for i in 0..L_WINDOW {
        y[i] = mult_r(x[i], HAMWINDOW[i]);
    }
    
    // Compute r[0] and test for overflow
    loop {
        set_overflow(false);
        sum = 1; // Avoid case of all zeros
        
        for i in 0..L_WINDOW {
            sum = l_mac(sum, y[i], y[i]);
        }
        
        // If overflow divide y[] by 4
        overflow = get_overflow();
        if !overflow {
            break;
        }
        
        for i in 0..L_WINDOW {
            y[i] = shr(y[i], 2);
        }
    }
    
    // Normalization of r[0]
    norm = norm_l(sum);
    sum = l_shl(sum, norm);
    l_extract(sum, &mut r_h[0], &mut r_l[0]); // Put in DPF format
    
    // r[1] to r[m]
    for i in 1..=(m as usize) {
        sum = 0;
        for j in 0..(L_WINDOW - i) {
            sum = l_mac(sum, y[j], y[j + i]);
        }
        
        sum = l_shl(sum, norm);
        l_extract(sum, &mut r_h[i], &mut r_l[i]);
    }
}

/// Lag windowing on autocorrelations
/// Based on ITU reference LPC.C: Lag_window()
pub fn lag_window(
    m: Word16,         // LPC order
    r_h: &mut [Word16], // Autocorrelations (msb)
    r_l: &mut [Word16], // Autocorrelations (lsb)
) {
    // Lag window table (from ITU reference TAB_LD8A.C)
    const LAG_H: [Word16; 11] = [
        32767, 32729, 32610, 32413, 32138, 31786, 31357, 30852, 30274, 29622, 28898
    ];
    const LAG_L: [Word16; 11] = [
        0, 32677, 32238, 31446, 30247, 28639, 26622, 24196, 21362, 18120, 14470
    ];
    
    for i in 1..=(m as usize) {
        let x = mpy_32(r_h[i], r_l[i], LAG_H[i], LAG_L[i]);
        l_extract(l_shl(x, 4), &mut r_h[i], &mut r_l[i]);
    }
}

/// Multiply two 32-bit numbers in DPF format
/// Based on ITU reference OPER_32B.C: Mpy_32()
pub fn mpy_32(hi1: Word16, lo1: Word16, hi2: Word16, lo2: Word16) -> Word32 {
    let temp = l_mult(hi1, hi2);
    let temp = l_mac(temp, mult(hi1, lo2), 1);
    l_mac(temp, mult(lo1, hi2), 1)
}

/// Levinson-Durbin algorithm
/// Based on ITU reference LPC.C: Levinson()
pub fn levinson(
    rh: &[Word16],     // Autocorrelations (msb)
    rl: &[Word16],     // Autocorrelations (lsb)
    a: &mut [Word16],  // LPC coefficients
    rc: &mut [Word16], // Reflection coefficients
) {
    let mut a_hi = [0i16; MP1];
    let mut a_lo = [0i16; MP1];
    let mut k_hi: Word16 = 0;
    let mut k_lo: Word16 = 0;
    let mut at_hi: Word16 = 0;
    let mut at_lo: Word16 = 0;
    let mut alp_hi: Word16;
    let mut alp_lo: Word16;
    let mut temp: Word32;
    
    // Initialize
    a_hi[0] = 4096; // 1.0 in Q12
    a_lo[0] = 0;
    alp_hi = rh[0];
    alp_lo = rl[0];
    
    for i in 1..=M {
        // Compute correlation
        temp = 0;
        for j in 1..i {
            let mpy_result = mpy_32(a_hi[j], a_lo[j], rh[i - j], rl[i - j]);
            temp = l_mac(temp, extract_h(mpy_result), 1);
        }
        
        temp = l_shl(temp, 4);
        l_extract(temp, &mut at_hi, &mut at_lo);
        
        // Compute reflection coefficient
        let alp_exp = norm_l(mpy_32(alp_hi, alp_lo, alp_hi, alp_lo));
        let alp_norm = l_shl(mpy_32(alp_hi, alp_lo, alp_hi, alp_lo), alp_exp);
        l_extract(alp_norm, &mut alp_hi, &mut alp_lo);
        
        let cor = l_sub(mpy_32(rh[i], rl[i], 32767, 0), mpy_32(at_hi, at_lo, 32767, 0));
        l_extract(cor, &mut at_hi, &mut at_lo);
        
        // Simplified reflection coefficient computation (would need div_32 for exact ITU compliance)
        if alp_hi != 0 {
            k_hi = divide_s(at_hi, alp_hi);
            k_lo = 0;
        } else {
            k_hi = 0;
            k_lo = 0;
        }
        
        rc[i - 1] = k_hi;
        
        // Update LPC coefficients
        for j in 1..(i / 2 + 1) {
            let temp1 = mpy_32(a_hi[j], a_lo[j], 32767, 0);
            let temp2 = mpy_32(k_hi, k_lo, a_hi[i - j], a_lo[i - j]);
            let temp = l_sub(temp1, temp2);
            l_extract(temp, &mut a_hi[j], &mut a_lo[j]);
            
            let temp1 = mpy_32(a_hi[i - j], a_lo[i - j], 32767, 0);
            let temp2 = mpy_32(k_hi, k_lo, a_hi[j], a_lo[j]);
            let temp = l_sub(temp1, temp2);
            l_extract(temp, &mut a_hi[i - j], &mut a_lo[i - j]);
        }
        
        a_hi[i] = k_hi;
        a_lo[i] = k_lo;
        
        // Update alpha
        let temp = mpy_32(k_hi, k_lo, k_hi, k_lo);
        let temp = l_sub(mpy_32(alp_hi, alp_lo, 32767, 0), temp);
        l_extract(temp, &mut alp_hi, &mut alp_lo);
    }
    
    // Convert to output format (Q12)
    a[0] = 4096; // a[0] is always 1.0 in Q12
    for i in 1..=M {
        a[i] = round(l_shl(l_deposit_h(a_hi[i]), 4));
    }
}

/// Simple division for reflection coefficient computation (placeholder)
fn divide_s(num: Word16, den: Word16) -> Word16 {
    if den == 0 {
        return 0;
    }
    
    // Simplified division - in a full implementation this would use div_s from basic ops
    let result = (num as i32 * 32767) / (den as i32);
    saturate(result)
}

/// Convert LPC coefficients to Line Spectral Pairs (LSP)
/// 
/// Based on ITU-T G.729A Az_lsp function from LPC.C
/// 
/// # Arguments
/// * `a` - LPC predictor coefficients (Q12)
/// * `lsp` - Output line spectral pairs (Q15) 
/// * `old_lsp` - Previous LSP values for fallback (Q15)
pub fn az_lsp(a: &[Word16], lsp: &mut [Word16], old_lsp: &[Word16]) {
    assert_eq!(a.len(), MP1);
    assert_eq!(lsp.len(), M);
    assert_eq!(old_lsp.len(), M);
    
    let mut f1 = [0i16; NC + 1];
    let mut f2 = [0i16; NC + 1];
    let mut ovf_coef = false;
    
    // Find the sum and diff polynomials F1(z) and F2(z)
    // F1(z) <--- F1(z)/(1+z**-1) & F2(z) <--- F2(z)/(1-z**-1)
    
    f1[0] = 2048; // f1[0] = 1.0 in Q11
    f2[0] = 2048; // f2[0] = 1.0 in Q11
    
    for i in 0..NC {
        // Check for overflow
        let overflow_check = || {
            let t0 = l_mult(a[i + 1], 16384);
            let t0 = l_mac(t0, a[M - i], 16384);
            (t0, extract_h(t0))
        };
        
        let (t0, x) = overflow_check();
        if t0 > 0x7FFFFFFF || t0 < -0x80000000 {
            ovf_coef = true;
        }
        
        f1[i + 1] = sub(x, f1[i]); // f1[i+1] = a[i+1] + a[M-i] - f1[i]
        
        let t0 = l_mult(a[i + 1], 16384);
        let t0 = l_msu(t0, a[M - i], 16384);
        let x = extract_h(t0);
        
        f2[i + 1] = add(x, f2[i]); // f2[i+1] = a[i+1] - a[M-i] + f2[i]
    }
    
    // Handle overflow by using Q10 instead of Q11
    if ovf_coef {
        f1[0] = 1024; // f1[0] = 1.0 in Q10
        f2[0] = 1024; // f2[0] = 1.0 in Q10
        
        for i in 0..NC {
            let t0 = l_mult(a[i + 1], 8192);
            let t0 = l_mac(t0, a[M - i], 8192);
            let x = extract_h(t0);
            f1[i + 1] = sub(x, f1[i]);
            
            let t0 = l_mult(a[i + 1], 8192);
            let t0 = l_msu(t0, a[M - i], 8192);
            let x = extract_h(t0);
            f2[i + 1] = add(x, f2[i]);
        }
    }
    
    // Find the LSPs using Chebyshev polynomial evaluation
    let mut nf = 0; // number of found frequencies
    let mut ip = 0; // indicator for f1 or f2
    
    let mut coef = &f1;
    
    let mut xlow = LSP_GRID[0];
    let mut ylow = if ovf_coef { chebps_10(xlow, coef, NC) } else { chebps_11(xlow, coef, NC) };
    
    let mut j = 0;
    while nf < M && j < GRID_POINTS {
        j += 1;
        let xhigh = xlow;
        let yhigh = ylow;
        xlow = LSP_GRID[j];
        ylow = if ovf_coef { chebps_10(xlow, coef, NC) } else { chebps_11(xlow, coef, NC) };
        
        let l_temp = l_mult(ylow, yhigh);
        if l_temp <= 0 {
            // Divide 2 times the interval
            let mut xmid = xlow;
            let mut ymid = ylow;
            let mut xhigh_temp = xhigh;
            let mut yhigh_temp = yhigh;
            let mut xlow_temp = xlow;
            let mut ylow_temp = ylow;
            
            for _ in 0..2 {
                xmid = add(shr(xlow_temp, 1), shr(xhigh_temp, 1)); // xmid = (xlow + xhigh)/2
                ymid = if ovf_coef { chebps_10(xmid, coef, NC) } else { chebps_11(xmid, coef, NC) };
                
                let l_temp = l_mult(ylow_temp, ymid);
                if l_temp <= 0 {
                    yhigh_temp = ymid;
                    xhigh_temp = xmid;
                } else {
                    ylow_temp = ymid;
                    xlow_temp = xmid;
                }
            }
            
            // Linear interpolation
            // xint = xlow - ylow*(xhigh-xlow)/(yhigh-ylow);
            let x = sub(xhigh_temp, xlow_temp);
            let y = sub(yhigh_temp, ylow_temp);
            
            let xint = if y == 0 {
                xlow_temp
            } else {
                let sign = y;
                let y_abs = abs_s(y);
                let exp = norm_s(y_abs);
                let y_norm = shl(y_abs, exp);
                let y_div = div_s(16383, y_norm);
                let mut t0 = l_mult(x, y_div);
                t0 = l_shr(t0, sub(20, exp));
                let mut y_result = extract_l(t0);
                
                if sign < 0 {
                    y_result = negate(y_result);
                }
                
                let t0 = l_mult(ylow_temp, y_result);
                let t0 = l_shr(t0, 11);
                sub(xlow_temp, extract_l(t0))
            };
            
            lsp[nf] = xint;
            xlow = xint;
            nf += 1;
            
            if ip == 0 {
                ip = 1;
                coef = &f2;
            } else {
                ip = 0;
                coef = &f1;
            }
            ylow = if ovf_coef { chebps_10(xlow, coef, NC) } else { chebps_11(xlow, coef, NC) };
        }
    }
    
    // Check if M roots found
    if nf < M {
        // Use old LSP values as fallback
        for i in 0..M {
            lsp[i] = old_lsp[i];
        }
    }
}

/// Convert Line Spectral Pairs (LSP) to LPC coefficients
/// 
/// Based on ITU-T G.729A Lsp_Az function from LPCFUNC.C
/// 
/// # Arguments
/// * `lsp` - Line spectral frequencies (Q15)
/// * `a` - Output predictor coefficients (Q12)
pub fn lsp_az(lsp: &[Word16], a: &mut [Word16]) {
    assert_eq!(lsp.len(), M);
    assert_eq!(a.len(), MP1);
    
    let mut f1 = [0i32; 6];
    let mut f2 = [0i32; 6];
    
    get_lsp_pol(&lsp[0..M], &mut f1, 0);
    get_lsp_pol(&lsp[0..M], &mut f2, 1);
    
    for i in (1..6).rev() {
        f1[i] = l_add(f1[i], f1[i - 1]); // f1[i] += f1[i-1];
        f2[i] = l_sub(f2[i], f2[i - 1]); // f2[i] -= f2[i-1];
    }
    
    a[0] = 4096; // 1.0 in Q12
    
    for i in 1..=5 {
        let j = 11 - i;
        let t0 = l_add(f1[i], f2[i]); // f1[i] + f2[i]
        a[i] = extract_l(l_shr_r(t0, 13)); // from Q24 to Q12 and * 0.5
        
        let t0 = l_sub(f1[i], f2[i]); // f1[i] - f2[i]
        a[j] = extract_l(l_shr_r(t0, 13)); // from Q24 to Q12 and * 0.5
    }
}

/// Interpolate quantized LSP between subframes
/// 
/// Based on ITU-T G.729A Int_qlpc function from LPCFUNC.C
/// 
/// # Arguments
/// * `lsp_old` - LSP vector of past frame (Q15)
/// * `lsp_new` - LSP vector of present frame (Q15)  
/// * `az` - Output interpolated Az() for the 2 subframes (Q12)
pub fn int_qlpc(lsp_old: &[Word16], lsp_new: &[Word16], az: &mut [Word16]) {
    assert_eq!(lsp_old.len(), M);
    assert_eq!(lsp_new.len(), M);
    assert_eq!(az.len(), 2 * MP1);
    
    let mut lsp = [0i16; M];
    
    // lsp[i] = lsp_new[i] * 0.5 + lsp_old[i] * 0.5
    for i in 0..M {
        lsp[i] = add(shr(lsp_new[i], 1), shr(lsp_old[i], 1));
    }
    
    // Subframe 1 - interpolated LSP
    lsp_az(&lsp, &mut az[0..MP1]);
    
    // Subframe 2 - current LSP
    lsp_az(lsp_new, &mut az[MP1..2*MP1]);
}

/// Get LSP polynomial F1(z) or F2(z) from the LSPs
/// 
/// Based on ITU-T G.729A Get_lsp_pol function
/// 
/// # Arguments
/// * `lsp` - Line spectral freq. (cosine domain) in Q15
/// * `f` - Coefficients of F1 or F2 in Q24
/// * `offset` - 0 for F1, 1 for F2
fn get_lsp_pol(lsp: &[Word16], f: &mut [i32], offset: usize) {
    // All computation in Q24
    f[0] = l_mult(4096, 2048); // f[0] = 1.0 in Q24
    f[1] = l_msu(0, lsp[offset], 512); // f[1] = -2.0 * lsp[0] in Q24
    
    let mut lsp_idx = offset + 2;
    
    for i in 2..=5 {
        f[i] = f[i - 2];
        
        // Bounds check for LSP array access
        if lsp_idx < lsp.len() {
            for j in (1..i).rev() {
                if j > 0 && j < f.len() && j >= 2 && (j - 1) < f.len() && (j - 2) < f.len() {
                    let (hi, lo) = l_extract_tuple(f[j - 1]);
                    let t0 = mpy_32_16(hi, lo, lsp[lsp_idx]);
                    let t0 = l_shl(t0, 1);
                    f[j] = l_add(f[j], f[j - 2]);
                    f[j] = l_sub(f[j], t0);
                }
            }
            if f.len() > 1 {
                f[1] = l_msu(f[1], lsp[lsp_idx], 512);
            }
        }
        
        lsp_idx += 2;
    }
}

/// Evaluate Chebyshev polynomial series (Q11 version)
/// 
/// Based on ITU-T G.729A Chebps_11 function
/// 
/// # Arguments
/// * `x` - Value to evaluate polynomial at
/// * `f` - Polynomial coefficients
/// * `n` - Order of polynomial
/// 
/// # Returns
/// Polynomial value
fn chebps_11(x: Word16, f: &[Word16], n: usize) -> Word16 {
    let mut b0 = 0i16;
    let mut b1 = 0i16;
    
    let x2 = shl(x, 1); // 2*x in Q15
    
    for i in (0..=n).rev() {
        let t0 = mult_r(x2, b1);
        let t0 = sub(t0, b0);
        let t0 = add(t0, f[i]);
        b0 = b1;
        b1 = t0;
    }
    
    let t0 = mult_r(x, b1);
    sub(t0, b0)
}

/// Evaluate Chebyshev polynomial series (Q10 version) 
/// 
/// Based on ITU-T G.729A Chebps_10 function
/// 
/// # Arguments
/// * `x` - Value to evaluate polynomial at
/// * `f` - Polynomial coefficients  
/// * `n` - Order of polynomial
/// 
/// # Returns
/// Polynomial value
fn chebps_10(x: Word16, f: &[Word16], n: usize) -> Word16 {
    let mut b0 = 0i16;
    let mut b1 = 0i16;
    
    let x2 = shl(x, 1); // 2*x in Q15
    
    for i in (0..=n).rev() {
        let t0 = mult_r(x2, b1);
        let t0 = sub(t0, b0);
        let t0 = add(t0, f[i]);
        b0 = b1;
        b1 = t0;
    }
    
    let t0 = mult_r(x, b1);
    sub(t0, b0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_l_extract() {
        let mut hi = 0i16;
        let mut lo = 0i16;
        let test_val = 0x12345678i32;
        
        l_extract(test_val, &mut hi, &mut lo);
        
        // Verify the extraction follows ITU format
        assert_eq!(hi, extract_h(test_val));
    }

    #[test]
    fn test_autocorr_basic() {
        let signal = [100i16; L_WINDOW];
        let mut r_h = [0i16; MP1];
        let mut r_l = [0i16; MP1];
        
        autocorr(&signal, M as Word16, &mut r_h, &mut r_l);
        
        // Should not crash and should produce some correlation
        assert!(r_h[0] != 0 || r_l[0] != 0);
    }

    #[test]
    fn test_lag_window() {
        let mut r_h = [1000i16; MP1];
        let mut r_l = [500i16; MP1];
        
        lag_window(M as Word16, &mut r_h, &mut r_l);
        
        // Lag windowing should modify the values
        // r[0] should remain unchanged, others should be windowed
        assert_eq!(r_h[0], 1000);
    }

    #[test]
    fn test_mpy_32() {
        let result = mpy_32(1000, 500, 2000, 1000);
        assert!(result != 0); // Should produce some result
    }

    #[test]
    fn test_levinson_basic() {
        let mut r_h = [1000i16; MP1];
        let mut r_l = [0i16; MP1];
        let mut a = [0i16; MP1];
        let mut rc = [0i16; M];
        
        // Set up a basic autocorrelation
        r_h[0] = 32767;
        for i in 1..MP1 {
            r_h[i] = 1000;
        }
        
        levinson(&r_h, &r_l, &mut a, &mut rc);
        
        // Should produce LPC coefficients - a[0] should be 1.0 in Q12 (4096)
        assert_eq!(a[0], 4096);
    }
} 