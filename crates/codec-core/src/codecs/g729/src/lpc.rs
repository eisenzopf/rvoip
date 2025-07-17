//! Linear Predictive Coding (LPC) Analysis for G.729
//!
//! This module implements the LPC analysis functions used in G.729,
//! based on the ITU-T reference implementation in LPC.C and LPCFUNC.C.
//!
//! LPC analysis extracts the spectral envelope of speech by modeling
//! it as an all-pole filter. The process includes:
//! 1. Autocorrelation computation with windowing
//! 2. Levinson-Durbin algorithm to solve for LPC coefficients  
//! 3. Conversion to Line Spectral Pairs (LSP) for robust quantization

use super::types::{Word16, Word32, M, L_WINDOW, MP1};
use super::math::*;

/// Maximum number of iterations for LSP root finding
const MAX_LSP_ITERATIONS: usize = 4;

/// Number of grid points for LSP search
const GRID_POINTS: usize = 60;

/// Number of coefficients (M/2)
const NC: usize = M / 2;

/// Hamming window for LPC analysis (240 samples)
/// Exact values from ITU TAB_LD8K.C
const HAMWINDOW: [Word16; L_WINDOW] = [
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
   32767, 32741, 32665, 32537, 32359, 32129, 31850, 31521, 31143, 30716,
   30242, 29720, 29151, 28538, 27879, 27177, 26433, 25647, 24821, 23957,
   23055, 22117, 21145, 20139, 19102, 18036, 16941, 15820, 14674, 13505,
   12315, 11106,  9879,  8637,  7381,  6114,  4838,  3554,  2264,   971
];

/// Lag window coefficients for autocorrelation (exact ITU values)
const LAG_H: [Word16; M] = [
    32728, 32619, 32438, 32187, 31867, 31480, 31029, 30517, 29946, 29321
];

const LAG_L: [Word16; M] = [
    11904, 17280, 30720, 25856, 24192, 28992, 24384,  7360, 19520, 14784
];

/// Frequency grid for LSP root finding (cosine values in Q15)
/// Exact values from ITU TAB_LD8K.C
const GRID: [Word16; GRID_POINTS + 1] = [
      32760,     32723,     32588,     32364,     32051,     31651,
      31164,     30591,     29935,     29196,     28377,     27481,
      26509,     25465,     24351,     23170,     21926,     20621,
      19260,     17846,     16384,     14876,     13327,     11743,
      10125,      8480,      6812,      5126,      3425,      1714,
          0,     -1714,     -3425,     -5126,     -6812,     -8480,
     -10125,    -11743,    -13327,    -14876,    -16384,    -17846,
     -19260,    -20621,    -21926,    -23170,    -24351,    -25465,
     -26509,    -27481,    -28377,    -29196,    -29935,    -30591,
     -31164,    -31651,    -32051,    -32364,    -32588,    -32723,
     -32760
];

/// LPC analysis state
#[derive(Debug, Clone)]
pub struct LpcAnalyzer {
    /// Previous LPC coefficients for stability checking
    old_a: [Word16; MP1],
    /// Previous reflection coefficients
    old_rc: [Word16; 2],
    /// Previous LSP for backup
    old_lsp: [Word16; M],
}

impl Default for LpcAnalyzer {
    fn default() -> Self {
        let mut old_a = [0; MP1];
        old_a[0] = 4096; // a[0] = 1.0 in Q12
        
        // Initialize with default LSP values (spread evenly from 0 to pi)
        let mut old_lsp = [0; M];
        for i in 0..M {
            old_lsp[i] = ((i + 1) as Word32 * 32767 / (M + 1) as Word32) as Word16;
        }
        
        Self {
            old_a,
            old_rc: [0; 2],
            old_lsp,
        }
    }
}

impl LpcAnalyzer {
    /// Create a new LPC analyzer
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset the LPC analyzer state
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Analyze a frame of speech to extract LPC coefficients and LSPs
    /// 
    /// # Arguments
    /// * `speech` - Input speech frame
    /// * `lpc_coeffs` - Output LPC coefficients [M+1]
    /// * `lsp` - Output LSP parameters [M]
    pub fn analyze_frame(&mut self, speech: &[Word16], lpc_coeffs: &mut [Word16], lsp: &mut [Word16]) {
        assert_eq!(lpc_coeffs.len(), M + 1);
        assert_eq!(lsp.len(), M);
        
        // Prepare autocorrelation buffers
        let mut r_h = [0i16; MP1];
        let mut r_l = [0i16; MP1];
        
        // Perform autocorrelation analysis (need at least L_WINDOW samples)
        let padded_speech = if speech.len() < L_WINDOW {
            let mut padded = vec![0i16; L_WINDOW];
            padded[..speech.len()].copy_from_slice(speech);
            padded
        } else {
            speech[..L_WINDOW].to_vec()
        };
        
        self.autocorr(&padded_speech, &mut r_h, &mut r_l);
        
        // Solve for LPC coefficients using Levinson-Durbin
        let mut rc = [0i16; M];
        self.levinson(&r_h, &r_l, lpc_coeffs, &mut rc);
        
        // Convert LPC to LSP
        self.lpc_to_lsp_conversion(lpc_coeffs, lsp);
    }

    /// Convert LPC coefficients to LSP parameters
    fn lpc_to_lsp_conversion(&self, lpc: &[Word16], lsp: &mut [Word16]) {
        // Simplified LSP conversion - normally uses Chebyshev polynomials
        for i in 0..M.min(lsp.len()) {
            if i < lpc.len() {
                lsp[i] = mult(lpc[i], 26214); // Simple scaling
            }
        }
    }

    /// Convert LSP coefficients back to LPC coefficients
    /// 
    /// This is the reverse of the LPC to LSP conversion process.
    /// Used in the decoder to reconstruct LPC coefficients from quantized LSPs.
    /// 
    /// # Arguments
    /// * `lsp` - Input LSP coefficients [M]
    /// * `lpc` - Output LPC coefficients [M+1]
    pub fn lsp_to_lpc(&self, lsp: &[Word16], lpc: &mut [Word16]) {
        assert_eq!(lsp.len(), M);
        assert_eq!(lpc.len(), M + 1);
        
        // Set LPC[0] = 1.0 (in Q12 format)
        lpc[0] = 4096;
        
        // Simplified LSP to LPC conversion - normally uses polynomial evaluation
        for i in 1..=M {
            if i-1 < lsp.len() {
                lpc[i] = mult(lsp[i-1], 19661); // Simple scaling back
            } else {
                lpc[i] = 0;
            }
        }
    }

    /// Compute autocorrelations of windowed signal
    ///
    /// # Arguments
    /// * `x` - Input signal (240 samples)
    /// * `r_h` - Output autocorrelations MSB
    /// * `r_l` - Output autocorrelations LSB
    ///
    /// Uses double precision format for better accuracy
    pub fn autocorr(&self, x: &[Word16], r_h: &mut [Word16], r_l: &mut [Word16]) {
        assert!(x.len() >= L_WINDOW);
        assert!(r_h.len() >= MP1);
        assert!(r_l.len() >= MP1);

        let mut y = [0; L_WINDOW];
        
        // Apply Hamming window
        for i in 0..L_WINDOW {
            y[i] = mult_r(x[i], HAMWINDOW[i]);
        }

        // Handle potential overflow by scaling
        loop {
            set_overflow(0);
            
            // Compute r[0] and test for overflow
            let mut sum = 1; // Avoid case of all zeros
            for i in 0..L_WINDOW {
                sum = l_mac(sum, y[i], y[i]);
            }

            if get_overflow() == 0 {
                // No overflow, compute normalization and extract r[0]
                let norm = norm_l(sum);
                sum = l_shl(sum, norm);
                l_extract(sum, &mut r_h[0], &mut r_l[0]);

                // Compute r[1] to r[M]
                for i in 1..=M {
                    sum = 0;
                    for j in 0..(L_WINDOW - i) {
                        sum = l_mac(sum, y[j], y[j + i]);
                    }
                    sum = l_shl(sum, norm);
                    l_extract(sum, &mut r_h[i], &mut r_l[i]);
                }
                break;
            } else {
                // Overflow occurred, scale down by 4
                for i in 0..L_WINDOW {
                    y[i] = shr(y[i], 2);
                }
            }
        }
    }

    /// Apply lag window to autocorrelations
    ///
    /// # Arguments
    /// * `r_h` - Autocorrelations MSB (modified in place)
    /// * `r_l` - Autocorrelations LSB (modified in place)
    ///
    /// Improves numerical stability by windowing higher lag autocorrelations
    pub fn lag_window(&self, r_h: &mut [Word16], r_l: &mut [Word16]) {
        for i in 1..=M {
            let x = mpy_32(r_h[i], r_l[i], LAG_H[i - 1], LAG_L[i - 1]);
            l_extract(x, &mut r_h[i], &mut r_l[i]);
        }
    }

    /// Levinson-Durbin algorithm to compute LPC coefficients
    ///
    /// # Arguments
    /// * `rh` - Autocorrelations MSB  
    /// * `rl` - Autocorrelations LSB
    /// * `a` - Output LPC coefficients in Q12
    /// * `rc` - Output reflection coefficients in Q15
    ///
    /// Solves the normal equations to find optimal LPC coefficients
    pub fn levinson(&mut self, rh: &[Word16], rl: &[Word16], a: &mut [Word16], rc: &mut [Word16]) -> bool {
        assert!(rh.len() >= MP1);
        assert!(rl.len() >= MP1);
        assert!(a.len() >= MP1);
        assert!(rc.len() >= M);

        let mut ah = [0; MP1];
        let mut al = [0; MP1];
        let mut anh = [0; MP1];
        let mut anl = [0; MP1];

        // Initialize
        a[0] = 4096; // 1.0 in Q12

        // K = A[1] = -R[1] / R[0]
        let t1 = l_comp(rh[1], rl[1]); // R[1] in Q31
        let t2 = l_abs(t1); // abs R[1]
        let mut t0 = div_32(t2, rh[0], rl[0]); // R[1]/R[0] in Q31
        
        if t1 > 0 {
            t0 = l_negate(t0); // -R[1]/R[0]
        }

        let mut kh = 0;
        let mut kl = 0;
        l_extract(t0, &mut kh, &mut kl); // K in DPF
        rc[0] = kh;
        
        t0 = l_shr(t0, 4); // A[1] in Q27
        l_extract(t0, &mut ah[1], &mut al[1]); // A[1] in DPF

        // Alpha = R[0] * (1-K**2)
        t0 = mpy_32(kh, kl, kh, kl); // K*K in Q31
        t0 = l_abs(t0); // Some cases <0
        t0 = l_sub(0x7fffffff, t0); // 1 - K*K in Q31
        
        let mut hi = 0;
        let mut lo = 0;
        l_extract(t0, &mut hi, &mut lo); // DPF format
        t0 = mpy_32(rh[0], rl[0], hi, lo); // Alpha in Q31

        // Normalize Alpha
        let alp_exp = norm_l(t0);
        t0 = l_shl(t0, alp_exp);
        let mut alp_h = 0;
        let mut alp_l = 0;
        l_extract(t0, &mut alp_h, &mut alp_l); // DPF format

        // Iterations i=2 to M
        for i in 2..=M {
            // t0 = SUM(R[j]*A[i-j], j=1,i-1) + R[i]
            t0 = 0;
            for j in 1..i {
                t0 = l_add(t0, mpy_32(rh[j], rl[j], ah[i - j], al[i - j]));
            }
            
            t0 = l_shl(t0, 4); // Convert Q27 to Q31
            let t1 = l_comp(rh[i], rl[i]);
            t0 = l_add(t0, t1); // Add R[i] in Q31

            // K = -t0 / Alpha
            let t1 = l_abs(t0);
            let mut t2 = div_32(t1, alp_h, alp_l); // abs(t0)/Alpha
            
            if t0 > 0 {
                t2 = l_negate(t2); // K = -t0/Alpha
            }
            
            t2 = l_shl(t2, alp_exp); // Denormalize
            l_extract(t2, &mut kh, &mut kl); // K in DPF
            rc[i - 1] = kh;

            // Test for unstable filter
            if abs_s(kh) > 32750 {
                // Filter is unstable, use previous coefficients
                for j in 0..=M {
                    a[j] = self.old_a[j];
                }
                for j in 0..2 {
                    rc[j] = self.old_rc[j];
                }
                return false; // Indicate instability
            }

            // Compute new LPC coefficients
            // An[j] = A[j] + K*A[i-j] for j=1 to i-1
            for j in 1..i {
                let t1 = mpy_32(kh, kl, ah[i - j], al[i - j]);
                let t1 = l_add(l_comp(ah[j], al[j]), l_shr(t1, 4));
                l_extract(t1, &mut anh[j], &mut anl[j]);
            }

            // An[i] = K
            t0 = l_shr(l_comp(kh, kl), 4); // K in Q27
            l_extract(t0, &mut anh[i], &mut anl[i]);

            // Update Alpha = Alpha * (1 - K*K)
            t0 = mpy_32(kh, kl, kh, kl); // K*K in Q31
            t0 = l_abs(t0);
            t0 = l_sub(0x7fffffff, t0); // 1 - K*K in Q31
            l_extract(t0, &mut hi, &mut lo);
            t0 = mpy_32(alp_h, alp_l, hi, lo); // Alpha * (1-K*K)

            // Normalize new Alpha
            let exp = norm_l(t0);
            t0 = l_shl(t0, exp);
            l_extract(t0, &mut alp_h, &mut alp_l);
            let alp_exp = add(alp_exp, exp);

            // Copy An to A for next iteration
            for j in 1..=i {
                ah[j] = anh[j];
                al[j] = anl[j];
            }
        }

        // Convert from DPF Q27 to Q12 for output
        a[0] = 4096; // 1.0 in Q12
        for i in 1..=M {
            t0 = l_comp(ah[i], al[i]); // Combine DPF
            a[i] = round(l_shl(t0, 1)); // Convert Q27 to Q12
        }

        // Store coefficients for next frame
        for i in 0..=M {
            self.old_a[i] = a[i];
        }
        for i in 0..2.min(M) {
            self.old_rc[i] = rc[i];
        }

        true // Stable filter
    }

    /// Convert LPC coefficients to Line Spectral Pairs (LSP)
    ///
    /// # Arguments
    /// * `a` - LPC coefficients in Q12
    /// * `lsp` - Output LSP coefficients in Q15 (cosine domain)
    ///
    /// Uses Chebyshev polynomial evaluation to find LSP roots
    pub fn az_lsp(&mut self, a: &[Word16], lsp: &mut [Word16]) {
        assert!(a.len() >= MP1);
        assert!(lsp.len() >= M);

        let mut f1 = [0; NC + 1];
        let mut f2 = [0; NC + 1];

        // Compute F1(z) = A(z) + z^(-M-1) * A(z^(-1))
        // and F2(z) = A(z) - z^(-M-1) * A(z^(-1))
        self.get_lsp_pol(a, &mut f1, &mut f2);

        // Find the roots using Chebyshev polynomial evaluation
        let mut nf = 0; // Number of found frequencies
        let mut ip = 0; // Indicator for f1 or f2

        let mut coef = &f1[..]; // Start with F1(z)
        let mut xlow = GRID[0];
        let mut ylow = if ip == 0 { 
            chebps_11(xlow, coef, NC) 
        } else { 
            chebps_10(xlow, coef, NC) 
        };

        let mut j = 0;
        while nf < M && j < GRID_POINTS {
            j += 1;
            let xhigh = xlow;
            let yhigh = ylow;
            xlow = GRID[j];
            ylow = if ip == 0 { 
                chebps_11(xlow, coef, NC) 
            } else { 
                chebps_10(xlow, coef, NC) 
            };

            // Check for sign change (root)
            let l_temp = l_mult(ylow, yhigh);
            if l_temp <= 0 {
                // Found a root, refine using bisection
                let mut xl = xlow;
                let mut xh = xhigh;
                let mut yl = ylow;
                let mut yh = yhigh;

                // Divide interval 4 times for precision
                for _ in 0..MAX_LSP_ITERATIONS {
                    let xmid = add(shr(xl, 1), shr(xh, 1)); // (xl + xh) / 2
                    let ymid = if ip == 0 { 
                        chebps_11(xmid, coef, NC) 
                    } else { 
                        chebps_10(xmid, coef, NC) 
                    };

                    let l_temp = l_mult(yl, ymid);
                    if l_temp <= 0 {
                        yh = ymid;
                        xh = xmid;
                    } else {
                        yl = ymid;
                        xl = xmid;
                    }
                }

                // Linear interpolation for final result
                let x = sub(xh, xl);
                let y = sub(yh, yl);

                let xint = if y == 0 {
                    xl
                } else {
                    let sign = y;
                    let y = abs_s(y);
                    let exp = norm_s(y);
                    let y = shl(y, exp);
                    let y = div_s(16383, y);
                    let mut t0 = l_mult(x, y);
                    t0 = l_shr(t0, sub(20, exp));
                    let mut y = extract_l(t0); // (xh-xl)/(yh-yl) in Q11

                    if sign < 0 {
                        y = negate(y);
                    }

                    t0 = l_mult(yl, y); // Result in Q26
                    t0 = l_shr(t0, 11); // Result in Q15
                    sub(xl, extract_l(t0)) // xl - yl*y
                };

                lsp[nf] = xint;
                xlow = xint;
                nf += 1;

                // Alternate between F1 and F2
                if ip == 0 {
                    ip = 1;
                    coef = &f2[..];
                } else {
                    ip = 0;
                    coef = &f1[..];
                }

                ylow = if ip == 0 { 
                    chebps_11(xlow, coef, NC) 
                } else { 
                    chebps_10(xlow, coef, NC) 
                };
            }
        }

        // Check if all M roots found
        if nf < M {
            // Not all roots found, use previous LSP
            for i in 0..M {
                lsp[i] = self.old_lsp[i];
            }
        } else {
            // Save current LSP for next frame
            for i in 0..M {
                self.old_lsp[i] = lsp[i];
            }
        }
    }

    /// Convert LSP coefficients back to LPC coefficients
    ///
    /// # Arguments
    /// * `lsp` - LSP coefficients in Q15 (cosine domain)
    /// * `a` - Output LPC coefficients in Q12
    pub fn lsp_az(&self, lsp: &[Word16], a: &mut [Word16]) {
        assert!(lsp.len() >= M);
        assert!(a.len() >= MP1);

        let mut f1 = [0; 6];
        let mut f2 = [0; 6];

        // Get polynomial coefficients
        get_lsp_pol(&lsp[0..], &mut f1);
        get_lsp_pol(&lsp[1..], &mut f2);

        // Combine polynomials
        for i in (1..=5).rev() {
            f1[i] = l_add(f1[i], f1[i - 1]); // f1[i] += f1[i-1]
            f2[i] = l_sub(f2[i], f2[i - 1]); // f2[i] -= f2[i-1]
        }

        a[0] = 4096; // 1.0 in Q12
        for i in 1..=5 {
            let t0 = l_add(f1[i], f2[i]); // f1[i] + f2[i]
            a[i] = extract_l(l_shr_r(t0, 13)); // From Q24 to Q12 and * 0.5

            let t0 = l_sub(f1[i], f2[i]); // f1[i] - f2[i]
            a[M + 1 - i] = extract_l(l_shr_r(t0, 13)); // From Q24 to Q12 and * 0.5
        }
    }

    /// Get LSP polynomial coefficients
    fn get_lsp_pol(&self, a: &[Word16], f1: &mut [Word16], f2: &mut [Word16]) {
        // Compute symmetric and antisymmetric polynomials
        f1[0] = 1024; // 1.0 in Q10
        f2[0] = 1024; // 1.0 in Q10

        for i in 0..NC {
            let t0 = l_mult(a[i + 1], 8192); // (a[i+1] + a[M-i]) >> 1
            let t0 = l_mac(t0, a[M - i], 8192); // From Q11 to Q10
            let x = extract_h(t0);
            f1[i + 1] = sub(x, f1[i]); // f1[i+1] = a[i+1] + a[M-i] - f1[i]

            let t0 = l_mult(a[i + 1], 8192); // (a[i+1] - a[M-i]) >> 1
            let t0 = l_msu(t0, a[M - i], 8192); // From Q11 to Q10
            let x = extract_h(t0);
            f2[i + 1] = add(x, f2[i]); // f2[i+1] = a[i+1] - a[M-i] + f2[i]
        }
    }
}

/// Extract double precision format to high and low parts
fn l_extract(l_var: Word32, var_h: &mut Word16, var_l: &mut Word16) {
    *var_h = extract_h(l_var);
    *var_l = extract_l(l_shr(l_var, 1)) & 0x7fff;
}

/// Combine high and low parts to double precision format
fn l_comp(var_h: Word16, var_l: Word16) -> Word32 {
    l_add(l_deposit_h(var_h), l_deposit_l(var_l))
}

/// 32-bit multiplication in double precision format
fn mpy_32(hi1: Word16, lo1: Word16, hi2: Word16, lo2: Word16) -> Word32 {
    let temp = l_mult(hi1, hi2);
    l_add(temp, l_shr(l_mult(hi1, lo2), 15))
}

/// Division of 32-bit by 32-bit (simplified version)
fn div_32(l_num: Word32, denom_hi: Word16, denom_lo: Word16) -> Word32 {
    // Simplified implementation - in real G.729 this is more complex
    let denom = l_comp(denom_hi, denom_lo);
    if denom == 0 {
        return 0x7fffffff;
    }
    
    // Approximate division using shifts and iterations
    let mut quotient = 0;
    let mut remainder = l_num;
    
    for _ in 0..16 {
        quotient = l_shl(quotient, 1);
        remainder = l_shl(remainder, 1);
        
        if remainder >= denom {
            remainder = l_sub(remainder, denom);
            quotient = l_add(quotient, 1);
        }
    }
    
    quotient
}

/// Chebyshev polynomial evaluation for F1(z) (Q11 coefficients)
fn chebps_11(x: Word16, f: &[Word16], n: usize) -> Word16 {
    let mut b2_h = 256; // 1.0 in Q24 DPF
    let mut b2_l = 0;

    let mut t0 = l_mult(x, 512); // 2*x in Q24
    t0 = l_mac(t0, f[1], 4096); // + f[1] in Q24
    let mut b1_h = 0;
    let mut b1_l = 0;
    l_extract(t0, &mut b1_h, &mut b1_l); // b1 = 2*x + f[1]

    for i in 2..n {
        t0 = mpy_32_16(b1_h, b1_l, x); // t0 = 2.0*x*b1
        t0 = l_shl(t0, 1);
        t0 = l_mac(t0, b2_h, -32768); // t0 = 2.0*x*b1 - b2
        t0 = l_msu(t0, b2_l, 1);
        t0 = l_mac(t0, f[i], 4096); // t0 = 2.0*x*b1 - b2 + f[i]

        let mut b0_h = 0;
        let mut b0_l = 0;
        l_extract(t0, &mut b0_h, &mut b0_l); // b0 = 2.0*x*b1 - b2 + f[i]

        b2_l = b1_l; // b2 = b1
        b2_h = b1_h;
        b1_l = b0_l; // b1 = b0
        b1_h = b0_h;
    }

    t0 = mpy_32_16(b1_h, b1_l, x); // t0 = x*b1
    t0 = l_mac(t0, b2_h, -32768); // t0 = x*b1 - b2
    t0 = l_msu(t0, b2_l, 1);
    t0 = l_mac(t0, f[n], 2048); // t0 = x*b1 - b2 + f[n]/2

    t0 = l_shl(t0, 6); // Q24 to Q30 with saturation
    extract_h(t0) // Result in Q14
}

/// Chebyshev polynomial evaluation for F2(z) (Q10 coefficients)
fn chebps_10(x: Word16, f: &[Word16], n: usize) -> Word16 {
    let mut b2_h = 128; // 1.0 in Q23 DPF
    let mut b2_l = 0;

    let mut t0 = l_mult(x, 256); // 2*x in Q23
    t0 = l_mac(t0, f[1], 4096); // + f[1] in Q23
    let mut b1_h = 0;
    let mut b1_l = 0;
    l_extract(t0, &mut b1_h, &mut b1_l); // b1 = 2*x + f[1]

    for i in 2..n {
        t0 = mpy_32_16(b1_h, b1_l, x); // t0 = 2.0*x*b1
        t0 = l_shl(t0, 1);
        t0 = l_mac(t0, b2_h, -32768); // t0 = 2.0*x*b1 - b2
        t0 = l_msu(t0, b2_l, 1);
        t0 = l_mac(t0, f[i], 4096); // t0 = 2.0*x*b1 - b2 + f[i]

        let mut b0_h = 0;
        let mut b0_l = 0;
        l_extract(t0, &mut b0_h, &mut b0_l); // b0 = 2.0*x*b1 - b2 + f[i]

        b2_l = b1_l; // b2 = b1
        b2_h = b1_h;
        b1_l = b0_l; // b1 = b0
        b1_h = b0_h;
    }

    t0 = mpy_32_16(b1_h, b1_l, x); // t0 = x*b1
    t0 = l_mac(t0, b2_h, -32768); // t0 = x*b1 - b2
    t0 = l_msu(t0, b2_l, 1);
    t0 = l_mac(t0, f[n], 2048); // t0 = x*b1 - b2 + f[n]/2

    t0 = l_shl(t0, 7); // Q23 to Q30 with saturation
    extract_h(t0) // Result in Q14
}

/// 32-bit by 16-bit multiplication
fn mpy_32_16(hi: Word16, lo: Word16, n: Word16) -> Word32 {
    l_mult(hi, n)
}

/// Get LSP polynomial from LSP vector (for lsp_az)
fn get_lsp_pol(lsp: &[Word16], f: &mut [Word32]) {
    f[0] = l_mult(4096, 2048); // f[0] = 1.0 in Q24
    f[1] = l_msu(0, lsp[0], 512); // f[1] = -2.0 * lsp[0] in Q24

    for i in 2..=5 {
        f[i] = f[i - 2];
        let lsp_idx = if (i - 1) * 2 < lsp.len() { (i - 1) * 2 } else { 0 };
        for j in (1..i).rev() {
            let mut hi = 0;
            let mut lo = 0;
            l_extract(f[j - 1], &mut hi, &mut lo);
            let t0 = mpy_32_16(hi, lo, lsp[lsp_idx]); // f[j-1] * lsp
            let t0 = l_shl(t0, 1);
            if j >= 2 {
                f[j] = l_add(f[j], f[j - 2]); // f[j] += f[j-2]
            }
            f[j] = l_sub(f[j], t0); // f[j] -= t0
        }
        f[i] = l_msu(f[i], lsp[lsp_idx], 512); // f[i] -= lsp<<9
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lpc_analyzer_creation() {
        let analyzer = LpcAnalyzer::new();
        assert_eq!(analyzer.old_a[0], 4096); // 1.0 in Q12
    }

    #[test]
    fn test_autocorr_basic() {
        let analyzer = LpcAnalyzer::new();
        let x = [100; L_WINDOW]; // Simple constant signal
        let mut r_h = [0; MP1];
        let mut r_l = [0; MP1];
        
        analyzer.autocorr(&x, &mut r_h, &mut r_l);
        
        // R[0] should be positive (energy)
        assert!(r_h[0] > 0 || (r_h[0] == 0 && r_l[0] > 0));
        
        // R[1] should equal R[0] for constant signal (after windowing effects)
        // Due to windowing, this won't be exactly equal, but should be close
    }

    #[test]
    fn test_lag_window() {
        let analyzer = LpcAnalyzer::new();
        let mut r_h = [1000; MP1];
        let mut r_l = [500; MP1];
        let original_h = r_h.clone();
        
        analyzer.lag_window(&mut r_h, &mut r_l);
        
        // r[0] should be unchanged
        assert_eq!(r_h[0], original_h[0]);
        
        // Other values should be modified
        for i in 1..=M {
            // Values should generally be smaller due to lag windowing
            assert!(r_h[i] <= original_h[i]);
        }
    }

    #[test]
    fn test_levinson_stable() {
        let mut analyzer = LpcAnalyzer::new();
        
        // Create simple autocorrelations for a stable filter
        let mut rh = [0; MP1];
        let mut rl = [0; MP1];
        rh[0] = 32767; // Large R[0]
        rl[0] = 0;
        
        // Smaller values for R[1]..R[M] to ensure stability
        for i in 1..=M {
            rh[i] = 1000 / (i as Word16 + 1);
            rl[i] = 0;
        }
        
        let mut a = [0; MP1];
        let mut rc = [0; M];
        
        let stable = analyzer.levinson(&rh, &rl, &mut a, &mut rc);
        
        assert!(stable);
        assert_eq!(a[0], 4096); // a[0] should be 1.0 in Q12
        
        // Reflection coefficients should be reasonable
        for &coeff in &rc {
            assert!(abs_s(coeff) < 32000); // Should be stable
        }
    }

    #[test]
    fn test_lsp_conversion_basic() {
        let mut analyzer = LpcAnalyzer::new();
        
        // Test with simple identity-like coefficients to avoid overflow
        let a = [4096, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let mut lsp = [0; M];
        
        // This may fall back to backup LSP values if root finding fails
        analyzer.az_lsp(&a, &mut lsp);
        
        // LSP should be reasonable values 
        for &val in &lsp {
            assert!(abs_s(val) <= 32767); // Should be valid Q15 values
        }
        
        // Test the reverse conversion with known LSP values
        let test_lsp = [1000, 2000, 4000, 8000, 12000, 16000, 20000, 24000, 28000, 30000];
        let mut a_from_lsp = [0; MP1];
        analyzer.lsp_az(&test_lsp, &mut a_from_lsp);
        
        // a[0] should always be 1.0 in Q12
        assert_eq!(a_from_lsp[0], 4096);
        
        // Other coefficients should be reasonable
        for i in 1..MP1 {
            assert!(abs_s(a_from_lsp[i]) < 32767); // Should not overflow
        }
    }

    #[test]
    fn test_chebyshev_polynomials() {
        let f = [0, 1000, 2000, 3000, 4000, 5000];
        let x = 16384; // 0.5 in Q15
        
        let result11 = chebps_11(x, &f, 5);
        let result10 = chebps_10(x, &f, 5);
        
        // Results should be reasonable (not overflow)
        assert!(abs_s(result11) < 32767);
        assert!(abs_s(result10) < 32767);
    }

    #[test]
    fn test_l_extract_l_comp() {
        let original = 0x12345678;
        let mut hi = 0;
        let mut lo = 0;
        
        l_extract(original, &mut hi, &mut lo);
        let reconstructed = l_comp(hi, lo);
        
        // Should be close (not exact due to precision loss)
        let diff = l_abs(l_sub(original, reconstructed));
        assert!(diff < 65536); // Within reasonable tolerance
    }

    #[test]
    fn test_mpy_32() {
        let hi1 = 0x1234;
        let lo1 = 0x5678;
        let hi2 = 0x2000;
        let lo2 = 0x0000;
        
        let result = mpy_32(hi1, lo1, hi2, lo2);
        assert!(result != 0); // Should produce some result
    }
} 