//! LSP quantization using predictive two-stage vector quantization - bcg729-exact implementation

use crate::codecs::g729a::constants::*;
use crate::codecs::g729a::types::{Q15, Q31, LSPParameters, QuantizedLSP};
use crate::codecs::g729a::math::FixedPointOps;
use crate::codecs::g729a::tables::{LSP_CB1, LSP_CB2, MEAN_LSP, LSP_PRED_COEF, q13_row_to_q15};

// Constants for cosine/sine polynomial approximation (Q15)
const KCOS1: i32 = 32768;
const KCOS2: i32 = -16384;
const KCOS3: i32 = 1365;
const KCOS4: i32 = -46;

const KSIN1: i32 = 32768;
const KSIN2: i32 = -5461;
const KSIN3: i32 = 273;
const KSIN4: i32 = -7;

const HALF_PI_Q13: i16 = 12868; // π/2 in Q13

// ITU-T G.729A constants from bcg729 (in Q13 format)
const OO4PIPLUS1_IN_Q13: i16 = 1032; // 0.4*Pi + 1 in Q13
const O92PIMINUS1_IN_Q13: i16 = 23715; // 0.92*Pi - 1 in Q13
const ONE_IN_Q13: i16 = 8192; // 1.0 in Q13
const ONE_IN_Q11: i16 = 2048; // 1.0 in Q11
const ONE_POINT_2_IN_Q14: i16 = 19661; // 1.2 in Q14
const GAP1: i16 = 10; // 0.0012 in Q13
const GAP2: i16 = 5; // 0.0006 in Q13
const MIN_qLSF_DISTANCE: i16 = 5; // Minimum distance between qLSF
const qLSF_MIN: i16 = 40; // Minimum qLSF value
const qLSF_MAX: i16 = 25681; // Maximum qLSF value

/// Multiply 16x16 and return result shifted right by 11 (Q11 scaling)
fn mult16_16_p11(a: i16, b: i16) -> i16 {
    ((1024 + (a as i32 * b as i32)) >> 11) as i16
}

/// Multiply 16x16 and return result shifted right by 13 (Q13 scaling)
fn mult16_16_p13(a: i16, b: i16) -> i16 {
    ((4096 + (a as i32 * b as i32)) >> 13) as i16
}

/// Multiply 16x16 and return result shifted right by 15 (Q15 scaling)
fn mult16_16_p15(a: i16, b: i16) -> i32 {
    (16384 + (a as i32 * b as i32)) >> 15
}

/// bcg729-exact cosine function: LSF (Q13) -> LSP (Q15)
pub fn g729_cos_q13q15(x: i16) -> i16 {
    // Based on bcg729 implementation with 4-segment polynomial approximation
    // Input: Q13 in range [0, π] where π ≈ 25736
    // Output: Q15 in range [-1, 1]
    
    let x2: i16;
    let x_scaled: i16;
    
    if x < 12868 { // x < π/2
        if x < 6434 { // x < π/4
            x2 = mult16_16_p11(x, x); // x² in Q15
            // cos(x) ≈ Kcos1 + x²(Kcos2 + x²(Kcos3 + x²·Kcos4))
            let inner = KCOS3 + mult16_16_p15(KCOS4 as i16, x2);
            let middle = KCOS2 + mult16_16_p15(inner as i16, x2);
            let result = KCOS1 + mult16_16_p15(middle as i16, x2);
            // bcg729 uses SATURATE which clamps to MAXINT16 (32767) not -32768
            return result.min(32767).max(-32768) as i16;
        } else { // π/4 ≤ x < π/2
            let x_adj = 12868 - x; // π/2 - x
            x2 = mult16_16_p11(x_adj, x_adj); // x² in Q15
            // cos(x) = sin(π/2 - x)
            let inner = KSIN3 + mult16_16_p15(KSIN4 as i16, x2);
            let middle = KSIN2 + mult16_16_p15(inner as i16, x2);
            let poly = KSIN1 + mult16_16_p15(middle as i16, x2);
            return mult16_16_p13(x_adj, poly as i16);
        }
    } else { // x ≥ π/2
        x_scaled = 25736 - x; // π - x
        if x < 19302 { // π/2 ≤ x < 3π/4
            let x_adj = 12868 - x_scaled; // x - π/2
            x2 = mult16_16_p11(x_adj, x_adj); // x² in Q15
            // cos(x) = -sin(x - π/2)
            let inner = KSIN3 + mult16_16_p15(KSIN4 as i16, x2);
            let middle = KSIN2 + mult16_16_p15(inner as i16, x2);
            let poly = KSIN1 + mult16_16_p15(middle as i16, x2);
            return -mult16_16_p13(x_adj, poly as i16);
        } else { // 3π/4 ≤ x < π
            x2 = mult16_16_p11(x_scaled, x_scaled); // x² in Q15
            // cos(x) = -cos(π - x)
            let inner = KCOS3 + mult16_16_p15(KCOS4 as i16, x2);
            let middle = KCOS2 + mult16_16_p15(inner as i16, x2);
            let result = -KCOS1 - mult16_16_p15(middle as i16, x2);
            return result.clamp(-32768, 32767) as i16;
        }
    }
}

// Constants for sqrt approximation (Q14)
const C0: i32 = 3634;
const C1: i32 = 21173;
const C2: i32 = -12627;
const C3: i32 = 4204;

// Constants for atan approximation
const KPI6: i32 = 17157; // π/6 in Q15
const KTAN_PI6: i32 = 18919; // tan(π/6) in Q15
const KTAN_PI12: i32 = 8780; // tan(π/12) in Q15
const ATAN_B: i32 = 8453; // 0.257977658811405 in Q15
const ATAN_C: i32 = 19373; // 0.59120450521312 in Q15
const ONE_IN_Q15: i32 = 32768;
const ONE_IN_Q30: i64 = 1073741824; // 1.0 in Q30
const HALF_PI_Q15: i32 = 51472; // π/2 in Q15

/// Count leading zeros (excluding sign bit) for unsigned 32-bit
fn unsigned_count_leading_zeros(x: u32) -> u32 {
    if x == 0 { return 32; }
    x.leading_zeros()
}

/// Square root in Q0 -> Q7
pub fn g729_sqrt_q0q7(x: u32) -> i32 {
    if x == 0 { return 0; }
    
    // bcg729: k = (19-unsignedCountLeadingZeros(x))>>1;
    let clz = unsigned_count_leading_zeros(x);
    let k = ((19_i32 - clz as i32) >> 1).max(0);
    
    // x = VSHR32(x, k*2); /* x = x.2^-2k */
    let x_q14 = (x >> (k * 2)) as i32;
    
    // bcg729 uses nested MULT16_16_Q14 which is (a*b)>>14
    // rt = ADD16(C0, MULT16_16_Q14(x, ADD16(C1, MULT16_16_Q14(x, ADD16(C2, MULT16_16_Q14(x, (C3)))))));
    let term3 = C2 + ((x_q14 as i64 * C3 as i64) >> 14) as i32;
    let term2 = C1 + ((x_q14 as i64 * term3 as i64) >> 14) as i32;
    let rt = C0 + ((x_q14 as i64 * term2 as i64) >> 14) as i32;
    
    // rt = VSHR32(rt,-k); /* rt = sqrt(x).2^7 */
    if k >= 0 {
        rt << k
    } else {
        rt >> (-k)
    }
}

/// Arctangent in Q15 -> Q13
fn g729_atan_q15q13(x: i32) -> i16 {
    let mut x = x;
    let mut angle: i32;
    let mut high_segment = false;
    let mut sign = false;
    let mut complement = false;
    
    // Make argument positive
    if x < 0 {
        x = -x;
        sign = true;
    }
    
    // Limit argument to 0..1
    if x > ONE_IN_Q15 {
        complement = true;
        // bcg729: x = DIV32(ONE_IN_Q30, x); /* 1/x in Q15 */
        // For Q15 input, 1/x gives Q15 output
        x = ((ONE_IN_Q30 / x as i64) as i32).clamp(-32768, 32767);
    }
    
    // Determine segmentation
    if x > KTAN_PI12 {
        high_segment = true;
        // x = (x - k)/(1 + k*x)
        let numerator = (x - KTAN_PI6) as i64;
        let denominator = ONE_IN_Q15 as i64 + ((KTAN_PI6 as i64 * x as i64) >> 15);
        x = ((numerator << 15) / denominator) as i32;
    }
    
    // Approximate the function
    let x2 = ((x as i64 * x as i64) >> 15) as i32;
    let numerator = x as i64 * (ONE_IN_Q15 as i64 + ((ATAN_B as i64 * x2 as i64) >> 15));
    let denominator = ONE_IN_Q15 as i64 + ((ATAN_C as i64 * x2 as i64) >> 15);
    angle = (numerator / denominator) as i32;
    
    // Restore offset if needed
    if high_segment {
        angle += KPI6;
    }
    
    // Restore complement if needed
    if complement {
        angle = HALF_PI_Q15 - angle;
    }
    
    // Set result in Q13
    angle >>= 2;
    
    // Restore sign if needed
    if sign {
        (-angle) as i16
    } else {
        angle as i16
    }
}

/// Arcsine in Q15 -> Q13
fn g729_asin_q15q13(x: i16) -> i16 {
    // bcg729: g729Atan_Q15Q13(DIV32(SSHL(x,15), PSHR(g729Sqrt_Q0Q7(SUB32(ONE_IN_Q30, MULT16_16(x,x))),7)))
    
    // MULT16_16(x,x) gives Q30
    let x_sq = (x as i32) * (x as i32); // x² in Q30
    
    // SUB32(ONE_IN_Q30, x_sq)
    let one_minus_x_sq = (ONE_IN_Q30 as i32) - x_sq; // 1-x² in Q30
    
    if one_minus_x_sq <= 0 {
        // Handle edge cases
        if x >= 0 {
            return HALF_PI_Q13;
        } else {
            return -(HALF_PI_Q13 as i16);
        }
    }
    
    // g729Sqrt_Q0Q7 expects unsigned Q0 input, but one_minus_x_sq is Q30
    // We need to treat the Q30 value as if it were Q0 for the sqrt function
    let sqrt_val = g729_sqrt_q0q7(one_minus_x_sq as u32); // sqrt(1-x²) in Q7
    
    // PSHR(sqrt_val, 7) shifts right by 7 to get Q0
    // SSHL(x,15) shifts x left by 15 to get Q30
    // DIV32 divides Q30 by Q0 to get Q30/Q0 = Q30
    let numerator = (x as i32) << 15; // SSHL(x,15) -> Q30
    let denominator = sqrt_val >> 7;   // PSHR(sqrt_val,7) -> Q0
    
    if denominator <= 0 {
        return if x >= 0 { HALF_PI_Q13 } else { -(HALF_PI_Q13 as i16) };
    }
    
    // DIV32 gives Q15 result when dividing Q30 by Q0
    // But we need to ensure the result fits in 32-bit range for atan
    let ratio = (numerator / denominator).clamp(-2147483647, 2147483647);
    
    g729_atan_q15q13(ratio)
}

/// bcg729-exact arccos function: LSP (Q15) -> LSF (Q13)
pub fn g729_acos_q15q13(x: i16) -> i16 {
    // acos(x) = π/2 - asin(x)
    HALF_PI_Q13 - g729_asin_q15q13(x)
}

/// bcg729-exact rearrange coefficients function
fn rearrange_coefficients(qlsp: &mut [i16; LP_ORDER], j: i16) {
    for i in 1..LP_ORDER {
        let delta = (qlsp[i-1] - qlsp[i] + j) / 2;
        if delta > 0 {
            qlsp[i-1] -= delta;
            qlsp[i] += delta;
        }
    }
}

/// bcg729-exact insertion sort for stability check
fn insertion_sort(qlsf: &mut [i16; LP_ORDER]) {
    for i in 1..LP_ORDER {
        let key = qlsf[i];
        let mut j = i as i32 - 1;
        while j >= 0 && qlsf[j as usize] > key {
            qlsf[(j + 1) as usize] = qlsf[j as usize];
            j -= 1;
        }
        qlsf[(j + 1) as usize] = key;
    }
}

/// LSP predictor for MA prediction - bcg729-exact
pub struct LSPPredictor {
    /// Previous quantized LSF values for MA prediction (Q13 format like bcg729)
    prev_lsf: [[i16; LP_ORDER]; 4], // Changed to i16 to match bcg729
}

impl LSPPredictor {
    /// Create a new LSP predictor with bcg729-exact initialization
    pub fn new() -> Self {
        // bcg729 initialization: PI*(j+1)/(M+1) in Q2.13 format
        let initial_lsf: [i16; LP_ORDER] = [
            2339, 4679, 7018, 9358, 11698,
            14037, 16377, 18717, 21056, 23396
        ];
        
        let mut prev_lsf = [[0i16; LP_ORDER]; 4];
        for i in 0..4 {
            prev_lsf[i] = initial_lsf;
        }

        Self { prev_lsf }
    }
    
    /// Update predictor state with new quantized LSF (Q13)
    pub fn update(&mut self, quantized_lsf: &[i16; LP_ORDER]) {
        // Shift previous values
        for i in (1..4).rev() {
            self.prev_lsf[i] = self.prev_lsf[i - 1];
        }
        self.prev_lsf[0] = *quantized_lsf;
    }
}

/// LSP quantizer - bcg729-exact implementation
pub struct LSPQuantizer {
    predictor: LSPPredictor,
}

impl LSPQuantizer {
    /// Initialize LSPQuantizer with bcg729-exact algorithm
    pub fn new() -> Self {
        Self {
            predictor: LSPPredictor::new(),
        }
    }
    
    /// Quantize LSP parameters using bcg729-exact ITU-T G.729A algorithm
    pub fn quantize(&mut self, lsp: &LSPParameters) -> QuantizedLSP {
        // Step 1: Convert LSP to LSF using bcg729-exact arccos
        let mut lsf = [0i16; LP_ORDER];
        for i in 0..LP_ORDER {
            lsf[i] = g729_acos_q15q13(lsp.frequencies[i].0);
        }
        
        #[cfg(debug_assertions)]
        eprintln!("=== LSP QUANTIZER DEBUG ===");
        #[cfg(debug_assertions)]
        eprintln!("Input LSP: {:?}", lsp.frequencies.iter().take(5).map(|x| x.0).collect::<Vec<_>>());
        #[cfg(debug_assertions)]
        eprintln!("Converted LSF: {:?}", &lsf[0..5]);
        
        // Step 2: Compute weights using bcg729-exact algorithm
        let mut weights_threshold = [0i16; LP_ORDER];
        weights_threshold[0] = lsf[1] - OO4PIPLUS1_IN_Q13;
        for i in 1..LP_ORDER-1 {
            weights_threshold[i] = (lsf[i+1] - lsf[i-1]) - ONE_IN_Q13;
        }
        weights_threshold[LP_ORDER-1] = O92PIMINUS1_IN_Q13 - lsf[LP_ORDER-2];
        
        let mut weights = [0u16; LP_ORDER];
        for i in 0..LP_ORDER {
            if weights_threshold[i] > 0 {
                weights[i] = ONE_IN_Q11 as u16;
            } else {
                let square = ((weights_threshold[i] as i32) * (weights_threshold[i] as i32)) >> 13; // Q13*Q13 -> Q13
                let weighted = (square * 10) >> 2; // Apply factor and shift
                let result = weighted + (ONE_IN_Q11 as i32);
                weights[i] = result.clamp(0, 32767) as u16;
            }
        }
        // Apply special weights for coefficients 4 and 5
        weights[4] = ((weights[4] as i32 * ONE_POINT_2_IN_Q14 as i32) >> 14) as u16;
        weights[5] = ((weights[5] as i32 * ONE_POINT_2_IN_Q14 as i32) >> 14) as u16;
        
        // Step 3: Test both MA predictors (L0 = 0 and L0 = 1)
        let mut best_error = [u64::MAX; 2];
        let mut best_indices = [[0u8; 3]; 2]; // [L0][L1,L2,L3]
        
        for l0 in 0..2 {
            // Compute target vector for current predictor
            let mut target_vector = [0i16; LP_ORDER];
            for i in 0..LP_ORDER {
                let mut acc = (lsf[i] as i32) << 15; // Q13 -> Q28
                for j in 0..4 {
                    // Apply MA prediction: acc -= prev_lsf * ma_predictor (MSU operation)
                    let ma_coeff = self.get_ma_predictor(l0, j, i);
                    acc -= (self.predictor.prev_lsf[j][i] as i32) * (ma_coeff as i32);
                }
                // Convert back to Q13 and apply inverse MA predictor sum
                let temp_q13 = (acc >> 15) as i16;
                let inv_sum = self.get_inv_ma_predictor_sum(l0, i);
                target_vector[i] = ((temp_q13 as i32 * inv_sum as i32) >> 12) as i16; // Q13 * Q12 -> Q13
            }
            
            #[cfg(debug_assertions)]
            if l0 == 1 {
                eprintln!("L0={} target vector: {:?}", l0, &target_vector[0..5]);
            }
            
            // Find best L1 index
            let mut best_l1_error = i64::MAX;
            let mut best_l1 = 0u8;
            for l1 in 0..128 { // L1_RANGE = 128
                let mut error = 0i64;
                for j in 0..LP_ORDER {
                    let diff = (target_vector[j] as i32) - (self.get_l1_codebook(l1, j) as i32);
                    let ma_sum = self.get_ma_predictor_sum(l0, j);
                    let scaled_diff = ((diff as i32 * ma_sum as i32) >> 15) as i16;
                    let weighted_diff = ((scaled_diff as i32 * weights[j] as i32) >> 11) as i16;
                    error += (scaled_diff as i64) * (weighted_diff as i64);
                }
                if error < best_l1_error {
                    best_l1_error = error;
                    best_l1 = l1;
                }
            }
            
            #[cfg(debug_assertions)]
            if l0 == 1 {
                eprintln!("L0={} best L1={} (error={})", l0, best_l1, best_l1_error);
                eprintln!("  Expected L1=105, checking distance:");
                let mut error_105 = 0i64;
                for j in 0..LP_ORDER {
                    let diff = (target_vector[j] as i32) - (self.get_l1_codebook(105, j) as i32);
                    error_105 += (diff as i64) * (diff as i64);
                }
                eprintln!("  Distance to L1=105: {}", error_105);
            }
            
            // Find best L2 index (for first 5 coefficients)
            let mut best_l2_error = i64::MAX;
            let mut best_l2 = 0u8;
            for l2 in 0..32 { // L2_RANGE = 32
                let mut error = 0i64;
                for j in 0..5 {
                    let l1_val = self.get_l1_codebook(best_l1, j);
                    let l2_val = self.get_l2l3_codebook(l2, j);
                    let diff = (target_vector[j] as i32) - (l1_val as i32) - (l2_val as i32);
                    let ma_sum = self.get_ma_predictor_sum(l0, j);
                    let scaled_diff = ((diff as i32 * ma_sum as i32) >> 15) as i16;
                    let weighted_diff = ((scaled_diff as i32 * weights[j] as i32) >> 11) as i16;
                    error += (scaled_diff as i64) * (weighted_diff as i64);
                }
                if error < best_l2_error {
                    best_l2_error = error;
                    best_l2 = l2;
                }
            }
            
            // Find best L3 index (for last 5 coefficients)
            let mut best_l3_error = i64::MAX;
            let mut best_l3 = 0u8;
            for l3 in 0..32 { // L3_RANGE = 32
                let mut error = 0i64;
                for j in 5..LP_ORDER {
                    let l1_val = self.get_l1_codebook(best_l1, j);
                    let l3_val = self.get_l2l3_codebook(l3, j);
                    let diff = (target_vector[j] as i32) - (l1_val as i32) - (l3_val as i32);
                    let ma_sum = self.get_ma_predictor_sum(l0, j);
                    let scaled_diff = ((diff as i32 * ma_sum as i32) >> 15) as i16;
                    let weighted_diff = ((scaled_diff as i32 * weights[j] as i32) >> 11) as i16;
                    error += (scaled_diff as i64) * (weighted_diff as i64);
                }
                if error < best_l3_error {
                    best_l3_error = error;
                    best_l3 = l3;
                }
            }
            
            // Compute quantized vector and rearrange
            let mut quantized_vector = [0i16; LP_ORDER];
            for i in 0..5 {
                quantized_vector[i] = self.get_l1_codebook(best_l1, i) + self.get_l2l3_codebook(best_l2, i);
            }
            for i in 5..LP_ORDER {
                quantized_vector[i] = self.get_l1_codebook(best_l1, i) + self.get_l2l3_codebook(best_l3, i);
            }
            
            // Apply rearrangements as in bcg729
            rearrange_coefficients(&mut quantized_vector, GAP1);
            rearrange_coefficients(&mut quantized_vector, GAP2);
            
            // Compute final weighted error
            let mut final_error = 0u64;
            for i in 0..LP_ORDER {
                let diff = (target_vector[i] as i32) - (quantized_vector[i] as i32);
                let ma_sum = self.get_ma_predictor_sum(l0, i);
                let scaled_diff = (((diff as i32) * (ma_sum as i32)) >> 15).abs() as u16;
                let weighted_diff = ((scaled_diff as u32 * weights[i] as u32) >> 11) as u16;
                final_error += (scaled_diff as u64) * (weighted_diff as u64);
            }
            
            best_error[l0] = final_error;
            best_indices[l0] = [best_l1, best_l2, best_l3];
        }
        
        // Select best predictor
        let selected_l0 = if best_error[0] < best_error[1] { 0 } else { 1 };
        let selected_indices = best_indices[selected_l0];
        
        // Reconstruct final quantized LSF
        let mut quantizer_output = [0i16; LP_ORDER];
        for i in 0..5 {
            quantizer_output[i] = self.get_l1_codebook(selected_indices[0], i) + 
                                  self.get_l2l3_codebook(selected_indices[1], i);
        }
        for i in 5..LP_ORDER {
            quantizer_output[i] = self.get_l1_codebook(selected_indices[0], i) + 
                                  self.get_l2l3_codebook(selected_indices[2], i);
        }
        
        rearrange_coefficients(&mut quantizer_output, GAP1);
        rearrange_coefficients(&mut quantizer_output, GAP2);
        
        // Compute final qLSF using MA prediction
        let mut qlsf = [0i16; LP_ORDER];
        for i in 0..LP_ORDER {
            let ma_sum = self.get_ma_predictor_sum(selected_l0, i);
            let mut acc = (ma_sum as i32) * (quantizer_output[i] as i32); // Q15 * Q13 -> Q28
            for j in 0..4 {
                let ma_coeff = self.get_ma_predictor(selected_l0, j, i);
                acc += (ma_coeff as i32) * (self.predictor.prev_lsf[j][i] as i32);
            }
            qlsf[i] = (acc >> 15) as i16; // Q28 -> Q13
        }
        
        // Update predictor state
        self.predictor.update(&quantizer_output);
        
        // Apply stability check
        insertion_sort(&mut qlsf);
        
        if qlsf[0] < qLSF_MIN {
            qlsf[0] = qLSF_MIN;
        }
        
        for i in 0..LP_ORDER-1 {
            if qlsf[i+1] - qlsf[i] < MIN_qLSF_DISTANCE {
                qlsf[i+1] = qlsf[i] + MIN_qLSF_DISTANCE;
            }
        }
        
        if qlsf[LP_ORDER-1] > qLSF_MAX {
            qlsf[LP_ORDER-1] = qLSF_MAX;
        }
        
        // Convert qLSF back to qLSP
        let mut qlsp_frequencies = [Q15::ZERO; LP_ORDER];
        for i in 0..LP_ORDER {
            qlsp_frequencies[i] = Q15(g729_cos_q13q15(qlsf[i]));
        }
        
        QuantizedLSP {
            reconstructed: LSPParameters { frequencies: qlsp_frequencies },
            indices: [selected_l0 as u8, selected_indices[0], selected_indices[1], selected_indices[2]],
        }
    }
    
    // Helper functions with bcg729 exact values
    pub fn get_ma_predictor(&self, l0: usize, frame: usize, coeff: usize) -> i16 {
        // Use the actual bcg729 MA predictor coefficients
        let ma_predictors = [
            // Predictor 0
            [
                [8421, 9109, 9175, 8965, 9034, 9057, 8765, 8775, 9106, 8673],
                [7018, 7189, 7638, 7307, 7444, 7379, 7038, 6956, 6930, 6868],
                [5472, 4990, 5134, 5177, 5246, 5141, 5206, 5095, 4830, 5147],
                [4056, 3031, 2614, 3024, 2916, 2713, 3309, 3237, 2857, 3473],
            ],
            // Predictor 1
            [
                [7733, 7880, 8188, 8175, 8247, 8490, 8637, 8601, 8359, 7569],
                [4210, 3031, 2552, 3473, 3876, 3853, 4184, 4154, 3909, 3968],
                [3214, 1930, 1313, 2143, 2493, 2385, 2755, 2706, 2542, 2919],
                [3024, 1592,  940, 1631, 1723, 1579, 2034, 2084, 1913, 2601],
            ],
        ];
        
        if l0 < 2 && frame < 4 && coeff < LP_ORDER {
            ma_predictors[l0][frame][coeff]
        } else {
            0
        }
    }
    
    fn get_inv_ma_predictor_sum(&self, l0: usize, coeff: usize) -> i16 {
        // bcg729 inverse MA predictor sums in Q12 - CORRECTED VALUES
        let inv_sums = [
            [17210, 15888, 16357, 16183, 16516, 15833, 15888, 15421, 14840, 15597], // Predictor 0
            [9202, 7320, 6788, 7738, 8170, 8154, 8856, 8818, 8366, 8544], // Predictor 1
        ];
        
        if l0 < 2 && coeff < LP_ORDER {
            inv_sums[l0][coeff]
        } else {
            4096 // Default Q12 value
        }
    }
    
    fn get_ma_predictor_sum(&self, l0: usize, coeff: usize) -> i16 {
        // bcg729 MA predictor sums in Q15 - CORRECTED VALUES
        let sums = [
            [7798, 8447, 8205, 8293, 8126, 8477, 8447, 8703, 9043, 8604], // Predictor 0
            [14585, 18333, 19772, 17344, 16426, 16459, 15155, 15220, 16043, 15708], // Predictor 1
        ];
        
        if l0 < 2 && coeff < LP_ORDER {
            sums[l0][coeff]
        } else {
            16384 // Default Q15 value
        }
    }
    
    pub fn get_l1_codebook(&self, index: u8, coeff: usize) -> i16 {
        if (index as usize) < LSP_CB1.len() && coeff < LSP_CB1[0].len() {
            LSP_CB1[index as usize][coeff]
        } else {
            0
        }
    }
    
    fn get_l2l3_codebook(&self, index: u8, coeff: usize) -> i16 {
        if (index as usize) < LSP_CB2.len() && coeff < LSP_CB2[0].len() {
            LSP_CB2[index as usize][coeff]
        } else {
            0
        }
    }
}

/// LSP decoder (for decoder side) - bcg729-exact implementation
pub struct LSPDecoder {
    predictor: LSPPredictor,
}

impl LSPDecoder {
    /// Create a new LSP decoder
    pub fn new() -> Self {
        Self {
            predictor: LSPPredictor::new(),
        }
    }
    
    /// Decode LSP from indices using bcg729-exact algorithm
    pub fn decode(&mut self, indices: &[u8; 4]) -> LSPParameters {
        let l0 = indices[0] as usize;
        let l1 = indices[1] as usize;
        let l2 = indices[2] as usize;
        let l3 = indices[3] as usize;
        
        // Reconstruct quantizer output from codebooks
        let mut quantizer_output = [0i16; LP_ORDER];
        for i in 0..5 {
            quantizer_output[i] = LSP_CB1[l1 as usize][i] + 
                                  self.get_l2l3_codebook(l2 as u8, i);
        }
        for i in 5..LP_ORDER {
            quantizer_output[i] = LSP_CB1[l1 as usize][i] + 
                                  self.get_l2l3_codebook(l3 as u8, i);
        }
        
        // Apply rearrangements
        rearrange_coefficients(&mut quantizer_output, GAP1);
        rearrange_coefficients(&mut quantizer_output, GAP2);
        
        // Compute qLSF using MA prediction
        let mut qlsf = [0i16; LP_ORDER];
        for i in 0..LP_ORDER {
            let ma_sum = self.get_ma_predictor_sum(l0, i);
            let mut acc = (ma_sum as i32) * (quantizer_output[i] as i32); // Q15 * Q13 -> Q28
            for j in 0..4 {
                let ma_coeff = self.get_ma_predictor(l0, j, i);
                acc += (ma_coeff as i32) * (self.predictor.prev_lsf[j][i] as i32);
            }
            qlsf[i] = (acc >> 15) as i16; // Q28 -> Q13
        }
        
        // Update predictor state
        self.predictor.update(&quantizer_output);
        
        // Apply stability check
        insertion_sort(&mut qlsf);
        
        if qlsf[0] < qLSF_MIN {
            qlsf[0] = qLSF_MIN;
        }
        
        for i in 0..LP_ORDER-1 {
            if qlsf[i+1] - qlsf[i] < MIN_qLSF_DISTANCE {
                qlsf[i+1] = qlsf[i] + MIN_qLSF_DISTANCE;
            }
        }
        
        if qlsf[LP_ORDER-1] > qLSF_MAX {
            qlsf[LP_ORDER-1] = qLSF_MAX;
        }
        
        // Convert qLSF back to qLSP
        let mut frequencies = [Q15::ZERO; LP_ORDER];
        for i in 0..LP_ORDER {
            frequencies[i] = Q15(g729_cos_q13q15(qlsf[i]));
        }
        
        LSPParameters { frequencies }
    }
    
    // Helper functions
    fn get_ma_predictor(&self, l0: usize, frame: usize, coeff: usize) -> i16 {
        // Use the actual bcg729 MA predictor coefficients
        let ma_predictors = [
            // Predictor 0
            [
                [8421, 9109, 9175, 8965, 9034, 9057, 8765, 8775, 9106, 8673],
                [7018, 7189, 7638, 7307, 7444, 7379, 7038, 6956, 6930, 6868],
                [5472, 4990, 5134, 5177, 5246, 5141, 5206, 5095, 4830, 5147],
                [4056, 3031, 2614, 3024, 2916, 2713, 3309, 3237, 2857, 3473],
            ],
            // Predictor 1
            [
                [7733, 7880, 8188, 8175, 8247, 8490, 8637, 8601, 8359, 7569],
                [4210, 3031, 2552, 3473, 3876, 3853, 4184, 4154, 3909, 3968],
                [3214, 1930, 1313, 2143, 2493, 2385, 2755, 2706, 2542, 2919],
                [3024, 1592,  940, 1631, 1723, 1579, 2034, 2084, 1913, 2601],
            ],
        ];
        
        if l0 < 2 && frame < 4 && coeff < LP_ORDER {
            ma_predictors[l0][frame][coeff]
        } else {
            0
        }
    }
    
    fn get_inv_ma_predictor_sum(&self, l0: usize, coeff: usize) -> i16 {
        // bcg729 inverse MA predictor sums in Q12 - CORRECTED VALUES
        let inv_sums = [
            [17210, 15888, 16357, 16183, 16516, 15833, 15888, 15421, 14840, 15597], // Predictor 0
            [9202, 7320, 6788, 7738, 8170, 8154, 8856, 8818, 8366, 8544], // Predictor 1
        ];
        
        if l0 < 2 && coeff < LP_ORDER {
            inv_sums[l0][coeff]
        } else {
            4096 // Default Q12 value
        }
    }
    
    fn get_ma_predictor_sum(&self, l0: usize, coeff: usize) -> i16 {
        // bcg729 MA predictor sums in Q15 - CORRECTED VALUES
        let sums = [
            [7798, 8447, 8205, 8293, 8126, 8477, 8447, 8703, 9043, 8604], // Predictor 0
            [14585, 18333, 19772, 17344, 16426, 16459, 15155, 15220, 16043, 15708], // Predictor 1
        ];
        
        if l0 < 2 && coeff < LP_ORDER {
            sums[l0][coeff]
        } else {
            16384 // Default Q15 value
        }
    }
    
    fn get_l2l3_codebook(&self, index: u8, coeff: usize) -> i16 {
        if (index as usize) < LSP_CB2.len() && coeff < LSP_CB2[0].len() {
            LSP_CB2[index as usize][coeff]
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;



    #[test]
    fn test_lsp_quantizer_basic() {
        let mut quantizer = LSPQuantizer::new();
        
        // Create test LSP
        let mut frequencies = [Q15::ZERO; LP_ORDER];
        for i in 0..LP_ORDER {
            frequencies[i] = Q15::from_f32((i + 1) as f32 / (LP_ORDER + 1) as f32);
        }
        
        let lsp = LSPParameters { frequencies };
        
        // Quantize
        let quantized = quantizer.quantize(&lsp);
        
        // Check that we get indices
        assert_eq!(quantized.indices.len(), 4);
        
        // Check that we get reconstructed LSP
        assert_eq!(quantized.reconstructed.frequencies.len(), LP_ORDER);
    }
} 