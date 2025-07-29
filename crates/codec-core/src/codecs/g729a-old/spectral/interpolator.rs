//! LSP parameter interpolation for smooth transitions

use crate::codecs::g729a::constants::*;
use crate::codecs::g729a::types::{Q15, LSPParameters, QuantizedLSP};
use crate::codecs::g729a::math::FixedPointOps;

/// LSP interpolator for subframe parameter generation
pub struct LSPInterpolator;

impl LSPInterpolator {
    /// Interpolate LSP parameters between frames
    /// 
    /// For G.729A, interpolation weights are:
    /// - First subframe: 0.5 * previous + 0.5 * current
    /// - Second subframe: current (no interpolation)
    pub fn interpolate(
        prev_lsp: &LSPParameters,
        curr_lsp: &LSPParameters,
        subframe_idx: usize,
    ) -> LSPParameters {
        let mut interpolated = LSPParameters {
            frequencies: [Q15::ZERO; LP_ORDER],
        };
        
        match subframe_idx {
            0 => {
                // First subframe: 50% interpolation
                let weight_prev = Q15::HALF;
                let weight_curr = Q15::HALF;
                
                for i in 0..LP_ORDER {
                    let prev_weighted = prev_lsp.frequencies[i].saturating_mul(weight_prev);
                    let curr_weighted = curr_lsp.frequencies[i].saturating_mul(weight_curr);
                    interpolated.frequencies[i] = prev_weighted.saturating_add(curr_weighted);
                }
            }
            1 => {
                // Second subframe: use current LSP directly
                interpolated = curr_lsp.clone();
            }
            _ => {
                // Invalid subframe index, return current
                interpolated = curr_lsp.clone();
            }
        }
        
        // Ensure stability after interpolation
        Self::ensure_minimum_distance(&mut interpolated.frequencies);
        
        interpolated
    }
    
    /// Ensure minimum distance between adjacent LSP frequencies
    fn ensure_minimum_distance(lsp_freqs: &mut [Q15; LP_ORDER]) {
        // Minimum distance: 0.0391 (in normalized frequency)
        let min_dist = Q15((0.0391 * Q15_ONE as f32) as i16);
        
        // First, ensure ordering
        for i in 1..LP_ORDER {
            if lsp_freqs[i].0 < lsp_freqs[i-1].0 {
                // Swap if out of order
                let temp = lsp_freqs[i];
                lsp_freqs[i] = lsp_freqs[i-1];
                lsp_freqs[i-1] = temp;
            }
        }
        
        // Then ensure minimum distance
        for i in 1..LP_ORDER {
            let diff = lsp_freqs[i].0.saturating_sub(lsp_freqs[i-1].0);
            if diff < min_dist.0 {
                lsp_freqs[i] = Q15(lsp_freqs[i-1].0.saturating_add(min_dist.0));
            }
        }
        
        // Ensure last frequency doesn't exceed maximum
        let max_freq = Q15((0.98 * Q15_ONE as f32) as i16);
        if lsp_freqs[LP_ORDER-1].0 > max_freq.0 {
            lsp_freqs[LP_ORDER-1] = max_freq;
        }
    }
    
    /// Linear interpolation between two sets of LSP parameters
    /// with arbitrary weight
    pub fn interpolate_weighted(
        lsp1: &LSPParameters,
        lsp2: &LSPParameters,
        weight: Q15, // Weight for lsp2 (0 to 1)
    ) -> LSPParameters {
        let mut interpolated = LSPParameters {
            frequencies: [Q15::ZERO; LP_ORDER],
        };
        
        let weight1 = Q15::ONE.saturating_add(Q15(-weight.0));
        
        for i in 0..LP_ORDER {
            let val1 = lsp1.frequencies[i].saturating_mul(weight1);
            let val2 = lsp2.frequencies[i].saturating_mul(weight);
            interpolated.frequencies[i] = val1.saturating_add(val2);
        }
        
        Self::ensure_minimum_distance(&mut interpolated.frequencies);
        
        interpolated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interpolation_first_subframe() {
        let prev_lsp = LSPParameters {
            frequencies: [Q15::from_f32(0.1); LP_ORDER],
        };
        
        let curr_lsp = LSPParameters {
            frequencies: [Q15::from_f32(0.2); LP_ORDER],
        };
        
        let interpolated = LSPInterpolator::interpolate(&prev_lsp, &curr_lsp, 0);
        
        // Should be average of 0.1 and 0.2 = 0.15
        for freq in &interpolated.frequencies {
            assert!((freq.to_f32() - 0.15).abs() < 0.01);
        }
    }

    #[test]
    fn test_interpolation_second_subframe() {
        let prev_lsp = LSPParameters {
            frequencies: [Q15::from_f32(0.1); LP_ORDER],
        };
        
        let curr_lsp = LSPParameters {
            frequencies: [Q15::from_f32(0.2); LP_ORDER],
        };
        
        let interpolated = LSPInterpolator::interpolate(&prev_lsp, &curr_lsp, 1);
        
        // Should be exactly current LSP
        for i in 0..LP_ORDER {
            assert_eq!(interpolated.frequencies[i].0, curr_lsp.frequencies[i].0);
        }
    }

    #[test]
    fn test_minimum_distance_enforcement() {
        let mut lsp_freqs = [Q15::ZERO; LP_ORDER];
        
        // Create LSPs that are too close
        for i in 0..LP_ORDER {
            lsp_freqs[i] = Q15::from_f32(0.1 + 0.01 * i as f32);
        }
        
        LSPInterpolator::ensure_minimum_distance(&mut lsp_freqs);
        
        // Check minimum distance
        let min_dist = Q15((0.0391 * Q15_ONE as f32) as i16);
        for i in 1..LP_ORDER {
            let diff = lsp_freqs[i].0 - lsp_freqs[i-1].0;
            assert!(diff >= min_dist.0);
        }
    }

    #[test]
    fn test_ordering_correction() {
        let mut lsp_freqs = [Q15::ZERO; LP_ORDER];
        
        // Create out-of-order LSPs
        for i in 0..LP_ORDER {
            lsp_freqs[i] = Q15::from_f32(0.9 - 0.08 * i as f32);
        }
        
        LSPInterpolator::ensure_minimum_distance(&mut lsp_freqs);
        
        // Check ordering
        for i in 1..LP_ORDER {
            assert!(lsp_freqs[i].0 > lsp_freqs[i-1].0);
        }
    }

    #[test]
    fn test_weighted_interpolation() {
        let lsp1 = LSPParameters {
            frequencies: [Q15::from_f32(0.1); LP_ORDER],
        };
        
        let lsp2 = LSPParameters {
            frequencies: [Q15::from_f32(0.3); LP_ORDER],
        };
        
        // Test with 25% weight for lsp2
        let weight = Q15::from_f32(0.25);
        let interpolated = LSPInterpolator::interpolate_weighted(&lsp1, &lsp2, weight);
        
        // Should be 0.75 * 0.1 + 0.25 * 0.3 = 0.15
        for freq in &interpolated.frequencies {
            assert!((freq.to_f32() - 0.15).abs() < 0.02);
        }
    }

    #[test]
    fn test_maximum_frequency_limit() {
        let mut lsp_freqs = [Q15::ZERO; LP_ORDER];
        
        // Create LSPs with last one exceeding limit
        for i in 0..LP_ORDER {
            lsp_freqs[i] = Q15::from_f32(0.1 + 0.1 * i as f32);
        }
        
        LSPInterpolator::ensure_minimum_distance(&mut lsp_freqs);
        
        // Check that last frequency is capped
        let max_freq = Q15((0.98 * Q15_ONE as f32) as i16);
        assert!(lsp_freqs[LP_ORDER-1].0 <= max_freq.0);
    }
} 