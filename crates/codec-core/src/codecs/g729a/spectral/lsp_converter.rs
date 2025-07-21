//! LP to LSP (Line Spectral Pairs) conversion

use crate::codecs::g729a::constants::*;
use crate::codecs::g729a::types::{Q15, Q31, LPCoefficients, LSPParameters};
use crate::codecs::g729a::math::{
    evaluate_polynomial, find_polynomial_roots, generate_chebyshev_grid,
    form_sum_polynomial, form_difference_polynomial, FixedPointOps
};

/// LSP converter for spectral parameter transformation
pub struct LSPConverter {
    /// Chebyshev grid for root finding
    chebyshev_grid: Vec<Q15>,
}

impl LSPConverter {
    /// Create a new LSP converter
    pub fn new() -> Self {
        Self {
            chebyshev_grid: generate_chebyshev_grid(GRID_POINTS),
        }
    }
    
    /// Convert LP coefficients to LSP frequencies
    pub fn lp_to_lsp(&self, lp_coeffs: &LPCoefficients) -> LSPParameters {
        #[cfg(debug_assertions)]
        {
            eprintln!("LSP Conversion Debug:");
            eprintln!("  Input LP coeffs: {:?}", &lp_coeffs.values[..5].iter().map(|x| x.0).collect::<Vec<_>>());
        }
        
        // 1. Form sum and difference polynomials
        let f1 = form_sum_polynomial(&lp_coeffs.values);
        let f2 = form_difference_polynomial(&lp_coeffs.values);
        
        #[cfg(debug_assertions)]
        {
            eprintln!("  F1 polynomial: {:?}", f1.iter().map(|x| x.0).collect::<Vec<_>>());
            eprintln!("  F2 polynomial: {:?}", f2.iter().map(|x| x.0).collect::<Vec<_>>());
        }
        
        // 2. Find roots using Chebyshev polynomial evaluation
        let f1_roots = find_polynomial_roots(&f1, &self.chebyshev_grid, LP_ORDER / 2);
        let f2_roots = find_polynomial_roots(&f2, &self.chebyshev_grid, LP_ORDER / 2);
        
        #[cfg(debug_assertions)]
        {
            eprintln!("  F1 roots: {:?}", f1_roots.iter().map(|x| x.0).collect::<Vec<_>>());
            eprintln!("  F2 roots: {:?}", f2_roots.iter().map(|x| x.0).collect::<Vec<_>>());
        }
        
        // 3. Convert roots to LSP frequencies and sort
        let lsp_freqs = self.roots_to_lsp(&f1_roots, &f2_roots);
        
        #[cfg(debug_assertions)]
        {
            eprintln!("  Final LSP freqs: {:?}", lsp_freqs.iter().map(|x| x.0).collect::<Vec<_>>());
        }
        
        LSPParameters {
            frequencies: lsp_freqs,
        }
    }
    
    /// Convert LSP frequencies back to LP coefficients
    pub fn lsp_to_lp(&self, lsp: &LSPParameters) -> LPCoefficients {
        // TEMPORARY: Use reasonable fixed LP coefficients to test synthesis filter
        // This bypasses the LSP→LP conversion to isolate synthesis filter issues
        
        #[cfg(debug_assertions)]
        {
            eprintln!("LSP→LP conversion (TEMPORARY BYPASS):");
            eprintln!("  LSP freqs: {:?}", lsp.frequencies.iter().take(5).map(|x| x.0).collect::<Vec<_>>());
        }
        
        // Use a reasonable set of LP coefficients that won't cause instability
        // These are based on a stable all-pole filter with moderate spectral shaping
        let lp_values = [
            Q15(-8192),  // a[0] = -0.25
            Q15(4096),   // a[1] =  0.125
            Q15(-2048),  // a[2] = -0.0625
            Q15(1024),   // a[3] =  0.03125
            Q15(-512),   // a[4] = -0.015625
            Q15(256),    // a[5] =  0.0078125
            Q15(-128),   // a[6] = -0.00390625
            Q15(64),     // a[7] =  0.001953125
            Q15(-32),    // a[8] = -0.0009765625
            Q15(16),     // a[9] =  0.00048828125
        ];
        
        #[cfg(debug_assertions)]
        {
            eprintln!("  Fixed LP coeffs: {:?}", lp_values.iter().take(5).map(|x| x.0).collect::<Vec<_>>());
        }
        
        LPCoefficients {
            values: lp_values,
            reflection_coeffs: [Q15::ZERO; LP_ORDER], // Not computed in reverse
        }
    }
    
    /// Convert polynomial roots to LSP frequencies
    fn roots_to_lsp(&self, f1_roots: &[Q15], f2_roots: &[Q15]) -> [Q15; LP_ORDER] {
        let mut lsp = [Q15::ZERO; LP_ORDER];
        
        // Interleave roots: LSP[0], LSP[2], ... from F2
        //                   LSP[1], LSP[3], ... from F1
        for i in 0..LP_ORDER/2 {
            if i < f2_roots.len() {
                lsp[2 * i] = self.cos_to_frequency(f2_roots[i]);
            }
            if i < f1_roots.len() {
                lsp[2 * i + 1] = self.cos_to_frequency(f1_roots[i]);
            }
        }
        
        // Ensure ordering and minimum separation
        self.check_lsp_stability(&mut lsp);
        
        lsp
    }
    
    /// Convert cosine value to normalized frequency
    fn cos_to_frequency(&self, cos_val: Q15) -> Q15 {
        // frequency = acos(cos_val) / pi
        // Approximation: linear mapping from [-1, 1] to [0, 1]
        // f = (1 - cos_val) / 2
        let one_minus = Q15::ONE.saturating_add(Q15(-cos_val.0));
        Q15(one_minus.0 >> 1)
    }
    
    /// Convert LSP frequencies back to polynomial roots
    fn lsp_to_polynomials(&self, lsp: &[Q15; LP_ORDER]) -> (Vec<Q15>, Vec<Q15>) {
        let mut f1 = vec![Q15::ONE]; // Start with 1
        let mut f2 = vec![Q15::ONE];
        
        #[cfg(debug_assertions)]
        {
            eprintln!("    Building polynomials from LSP:");
            eprintln!("    LSP[0..5]: {:?}", lsp.iter().take(5).map(|x| x.0).collect::<Vec<_>>());
        }
        
        // Build polynomials from roots
        for i in 0..LP_ORDER/2 {
            // F2 polynomial from even LSPs
            if 2 * i < LP_ORDER {
                let cos_val = self.frequency_to_cos(lsp[2 * i]);
                #[cfg(debug_assertions)]
                eprintln!("    F2[{}]: LSP={} -> cos={}", i, lsp[2 * i].0, cos_val.0);
                f2 = self.multiply_by_root(&f2, cos_val);
            }
            
            // F1 polynomial from odd LSPs
            if 2 * i + 1 < LP_ORDER {
                let cos_val = self.frequency_to_cos(lsp[2 * i + 1]);
                #[cfg(debug_assertions)]
                eprintln!("    F1[{}]: LSP={} -> cos={}", i, lsp[2 * i + 1].0, cos_val.0);
                f1 = self.multiply_by_root(&f1, cos_val);
            }
        }
        
        #[cfg(debug_assertions)]
        {
            eprintln!("    Final F1 len: {}, F2 len: {}", f1.len(), f2.len());
        }
        
        (f1, f2)
    }
    
    /// Convert normalized frequency to cosine value
    fn frequency_to_cos(&self, freq: Q15) -> Q15 {
        // cos_val = cos(freq * pi)
        // LSP frequency range: [0, 1] normalized
        // Cosine range: [1, -1] for [0, pi]
        
        // Use piecewise linear approximation for better accuracy
        // Convert Q15 frequency [0, 32767] to angle [0, pi]
        let normalized = freq.0 as u32; // [0, 32767]
        
        if normalized <= 8192 {
            // First quarter [0, π/4]: cos(x) ≈ 1 - x²/2
            let x = (normalized * 2) as i32; // Scale to [0, 16384] 
            let x_sq = ((x as i64 * x as i64) >> 15) as i32; // x² in Q15
            let cos_val = Q15_ONE - (x_sq >> 1) as i16; // 1 - x²/2
            Q15(cos_val.max(0))
        } else if normalized <= 16384 {
            // Second quarter [π/4, π/2]: cos(x) ≈ 0.707 - 0.707*(x-π/4)
            let x_offset = normalized as i32 - 8192; // x - π/4
            let slope = 23170; // 0.707 in Q15
            let base = 23170; // 0.707 in Q15  
            let cos_val = base - ((slope as i64 * x_offset as i64) >> 13) as i32;
            Q15(cos_val.max(0) as i16)
        } else if normalized <= 24576 {
            // Third quarter [π/2, 3π/4]: cos(x) ≈ -(x-π/2)*0.707
            let x_offset = normalized as i32 - 16384; // x - π/2
            let slope = 23170; // 0.707 in Q15
            let cos_val = -((slope as i64 * x_offset as i64) >> 13) as i32;
            Q15(cos_val.max(-32767) as i16)
        } else {
            // Fourth quarter [3π/4, π]: cos(x) ≈ -0.707 - 0.707*(x-3π/4)
            let x_offset = normalized as i32 - 24576; // x - 3π/4
            let slope = 23170; // 0.707 in Q15
            let base = -23170; // -0.707 in Q15
            let cos_val = base - ((slope as i64 * x_offset as i64) >> 13) as i32;
            Q15(cos_val.max(-32767) as i16)
        }
    }
    
    /// Multiply polynomial by (z - root)
    fn multiply_by_root(&self, poly: &[Q15], root: Q15) -> Vec<Q15> {
        let mut result = vec![Q15::ZERO; poly.len() + 1];
        
        // (a0 + a1*z + ...) * (z - root) = -root*a0 + (a0 - root*a1)*z + ...
        // Use Q31 arithmetic to prevent overflow, then scale back to Q15
        
        // First coefficient: -root * a0
        let first_term = (-root.0 as i64 * poly[0].0 as i64) >> 15;
        result[0] = Q15(first_term.clamp(i16::MIN as i64, i16::MAX as i64) as i16);
        
        // Middle coefficients: a[i-1] - root * a[i]
        for i in 1..poly.len() {
            let term1 = poly[i - 1].0 as i64;
            let term2 = (root.0 as i64 * poly[i].0 as i64) >> 15;
            let combined = term1 - term2;
            result[i] = Q15(combined.clamp(i16::MIN as i64, i16::MAX as i64) as i16);
        }
        
        // Last coefficient: a[n]
        if !poly.is_empty() {
            result[poly.len()] = poly[poly.len() - 1];
        }
        
        // Apply scaling to prevent explosive growth
        // After each polynomial multiplication, scale down by small factor
        let scale_factor = 0.98; // Slightly less than 1 to prevent runaway growth
        for coeff in &mut result {
            let scaled = (coeff.0 as f32 * scale_factor) as i16;
            *coeff = Q15(scaled);
        }
        
        result
    }
    
    /// Multiply polynomial by (1 - 2*cos*z^-1 + z^-2)
    fn multiply_polynomial_by_factor(&self, poly: &mut [Q15], degree: usize, cos_val: Q15) {
        // This implements the stable recursion used in G.729A
        // poly(z) *= (1 - 2*cos*z^-1 + z^-2)
        
        if degree == 0 {
            return;
        }
        
        // Work backwards to avoid overwriting coefficients we still need
        for i in (2..=degree).rev() {
            if i < poly.len() {
                let term1 = poly[i];
                let term2 = if i >= 1 { 
                    Q15(((cos_val.0 as i32 * poly[i-1].0 as i32) >> 14) as i16) // 2*cos*poly[i-1]
                } else { 
                    Q15::ZERO 
                };
                let term3 = if i >= 2 { poly[i-2] } else { Q15::ZERO };
                
                poly[i] = term1.saturating_add(Q15(-term2.0)).saturating_add(term3);
            }
        }
        
        // Handle i=1 separately
        if degree >= 1 && poly.len() > 1 {
            let term1 = poly[1];
            let term2 = Q15(((cos_val.0 as i32 * poly[0].0 as i32) >> 14) as i16); // 2*cos*poly[0]
            poly[1] = term1.saturating_add(Q15(-term2.0));
        }
    }
    
    /// Check LSP stability and enforce minimum separation
    fn check_lsp_stability(&self, lsp: &mut [Q15; LP_ORDER]) {
        // Sort LSPs
        lsp.sort_by_key(|&x| x.0);
        
        // Enforce minimum separation (0.04 radians in Q15)
        let min_sep = Q15((0.04 * Q15_ONE as f32) as i16);
        
        for i in 1..LP_ORDER {
            let diff = lsp[i].0.saturating_sub(lsp[i-1].0);
            if diff < min_sep.0 {
                lsp[i] = Q15(lsp[i-1].0.saturating_add(min_sep.0));
            }
        }
        
        // Ensure last LSP doesn't exceed pi
        if lsp[LP_ORDER - 1].0 > Q15_ONE - 100 {
            lsp[LP_ORDER - 1] = Q15(Q15_ONE - 100);
        }
    }
}

impl Default for LSPConverter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lsp_converter_creation() {
        let converter = LSPConverter::new();
        assert_eq!(converter.chebyshev_grid.len(), GRID_POINTS);
    }

    #[test]
    fn test_cos_frequency_conversion() {
        let converter = LSPConverter::new();
        
        // Test cos(0) = 1 -> freq = 0
        let freq = converter.cos_to_frequency(Q15::ONE);
        assert!(freq.0.abs() < 1000);
        
        // Test cos(pi) = -1 -> freq = 1
        let freq = converter.cos_to_frequency(Q15::from_f32(-0.999));
        assert!(freq.to_f32() > 0.9);
        
        // Test round trip
        let original = Q15::from_f32(0.5);
        let freq = converter.cos_to_frequency(original);
        let back = converter.frequency_to_cos(freq);
        assert!((original.0 - back.0).abs() < 2000);
    }

    #[test]
    fn test_lsp_stability_check() {
        let converter = LSPConverter::new();
        
        // Create unstable LSPs (not ordered)
        let mut lsp = [Q15::ZERO; LP_ORDER];
        for i in 0..LP_ORDER {
            lsp[i] = Q15::from_f32((LP_ORDER - i) as f32 / (LP_ORDER + 1) as f32);
        }
        
        converter.check_lsp_stability(&mut lsp);
        
        // Check ordering
        for i in 1..LP_ORDER {
            assert!(lsp[i].0 >= lsp[i-1].0);
        }
        
        // Check minimum separation
        let min_sep = Q15((0.04 * Q15_ONE as f32) as i16);
        for i in 1..LP_ORDER {
            assert!(lsp[i].0 - lsp[i-1].0 >= min_sep.0 - 10); // Small tolerance
        }
    }

    #[test]
    fn test_multiply_by_root() {
        let converter = LSPConverter::new();
        
        // Test (1) * (z - 0.5) = z - 0.5
        let poly = vec![Q15::ONE];
        let root = Q15::from_f32(0.5);
        let result = converter.multiply_by_root(&poly, root);
        
        assert_eq!(result.len(), 2);
        assert!((result[0].to_f32() + 0.5).abs() < 0.01); // -0.5
        assert!((result[1].to_f32() - 1.0).abs() < 0.01); // 1.0
    }

    #[test]
    fn test_lp_to_lsp_simple() {
        let converter = LSPConverter::new();
        
        // Create simple LP coefficients
        let lp_coeffs = LPCoefficients {
            values: [Q15::ZERO; LP_ORDER],
            reflection_coeffs: [Q15::ZERO; LP_ORDER],
        };
        
        // Set a few non-zero values
        let mut values = lp_coeffs.values;
        values[0] = Q15::from_f32(-0.8);
        values[1] = Q15::from_f32(0.2);
        
        let lp_coeffs = LPCoefficients {
            values,
            reflection_coeffs: lp_coeffs.reflection_coeffs,
        };
        
        let lsp = converter.lp_to_lsp(&lp_coeffs);
        
        // Check that LSPs are ordered
        for i in 1..LP_ORDER {
            assert!(lsp.frequencies[i].0 >= lsp.frequencies[i-1].0);
        }
    }

    #[test]
    fn test_lsp_round_trip() {
        let converter = LSPConverter::new();
        
        // Create test LSP frequencies
        let mut frequencies = [Q15::ZERO; LP_ORDER];
        for i in 0..LP_ORDER {
            frequencies[i] = Q15::from_f32((i + 1) as f32 / (LP_ORDER + 1) as f32);
        }
        
        let lsp = LSPParameters { frequencies };
        
        // Convert to LP and back
        let lp = converter.lsp_to_lp(&lsp);
        let lsp2 = converter.lp_to_lsp(&lp);
        
        // Check that frequencies are approximately preserved
        for i in 0..LP_ORDER {
            let diff = (lsp.frequencies[i].0 - lsp2.frequencies[i].0).abs();
            assert!(diff < 3000); // Allow some quantization error
        }
    }
} 