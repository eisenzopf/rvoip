//! Gain quantization and processing for G.729A

use crate::codecs::g729a::constants::*;
use crate::codecs::g729a::types::{Q15, Q31, QuantizedGains};
use crate::codecs::g729a::math::{FixedPointOps, energy, dot_product};
use crate::codecs::g729a::types::Q14;
use crate::codecs::g729a::tables::{
    GBK1, GBK2, MAP1, MAP2, IMAP1, IMAP2, THR1, THR2, 
    GAIN_PRED_COEF, COEF, L_COEF
};
use std::collections::VecDeque;

/// Moving average predictor for gain prediction
pub struct GainPredictor {
    /// Past quantized gains for MA prediction
    past_gains: VecDeque<Q15>,
    /// Predictor coefficients
    ma_coeffs: [Q15; 4],
}

impl GainPredictor {
    /// Create a new gain predictor
    pub fn new() -> Self {
        // Convert predictor coefficients from Q13 to Q15
        let ma_coeffs = GAIN_PRED_COEF.iter()
            .map(|&val| Q15((val as i32 * 4) as i16))
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        
        // Initialize with past quantized energies
        // ITU-T G.729A uses initial energy of -14 dB in Q14
        // which is approximately 0.2 linear, or 6554 in Q15
        let initial_gain = Q15(6554);
            
        Self {
            past_gains: VecDeque::from(vec![initial_gain; 4]),
            ma_coeffs,
        }
    }
    
    /// Predict gain based on past quantized gains
    pub fn predict(&self) -> Q15 {
        let mut sum = Q31::ZERO;
        
        for (i, &past_gain) in self.past_gains.iter().enumerate() {
            let prod = self.ma_coeffs[i].to_q31().saturating_mul(past_gain.to_q31());
            sum = sum.saturating_add(prod);
        }
        
        sum.to_q15()
    }
    
    /// Update predictor with new quantized gain
    pub fn update(&mut self, quantized_gain: Q15) {
        self.past_gains.pop_back();
        self.past_gains.push_front(quantized_gain);
    }
}

/// Gain quantizer for adaptive and fixed codebook gains
pub struct GainQuantizer {
    predictor: GainPredictor,
}

impl GainQuantizer {
    /// Create a new gain quantizer
    pub fn new() -> Self {
        Self {
            predictor: GainPredictor::new(),
        }
    }
    
    /// Quantize adaptive and fixed codebook gains
    /// For G.729A: 7-bit scalar+vector quantization
    pub fn quantize(
        &mut self,
        adaptive_gain: Q15,
        fixed_gain: Q15,
        adaptive_vector: &[Q15],
        fixed_vector: &[Q15],
        target: &[Q15],
    ) -> QuantizedGains {
        #[cfg(debug_assertions)]
        {
            eprintln!("Gain Quantization Debug:");
            eprintln!("  Target gains: adaptive={}, fixed={}", adaptive_gain.0, fixed_gain.0);
        }
        
        // 1. Compute correlations needed for quantization
        let corr_xh = dot_product(target, adaptive_vector);
        let corr_xf = dot_product(target, fixed_vector);
        let corr_hf = dot_product(adaptive_vector, fixed_vector);
        let energy_h = energy(adaptive_vector);
        let energy_f = energy(fixed_vector);
        
        #[cfg(debug_assertions)]
        {
            eprintln!("  Correlations: xh={}, xf={}, hf={}", corr_xh.0, corr_xf.0, corr_hf.0);
            eprintln!("  Energies: h={}, f={}", energy_h.0, energy_f.0);
        }
        
        // 2. Get prediction
        let predicted_gain = self.predictor.predict();
        
        #[cfg(debug_assertions)]
        {
            eprintln!("  Predicted gain: {}", predicted_gain.0);
        }
        
        // 3. Search codebook (simplified - using dummy quantization)
        // In real G.729A, this would search a 7-bit MA-predictive VQ table
        let (ga_index, gc_index) = self.search_gain_codebook(
            adaptive_gain,
            fixed_gain,
            predicted_gain,
            corr_xh,
            corr_xf,
            corr_hf,
            energy_h,
            energy_f,
        );
        
        #[cfg(debug_assertions)]
        {
            eprintln!("  Selected indices: ga={}, gc={}", ga_index, gc_index);
            let decoded_ga = self.decode_adaptive_gain(ga_index);
            let decoded_gc = self.decode_fixed_gain(gc_index, predicted_gain);
            eprintln!("  Decoded gains: ga={}, gc={} (targets: ga={}, gc={})", 
                decoded_ga.0, decoded_gc.0, adaptive_gain.0, fixed_gain.0);
            
            // Show what index 0 would give for comparison
            let alt_gc = self.decode_fixed_gain(0, predicted_gain);
            eprintln!("  Alternative gc=0 would decode to: {}", alt_gc.0);
        }
        
        // 4. Reconstruct quantized gains
        let quantized_adaptive = self.decode_adaptive_gain(ga_index);
        let quantized_fixed = self.decode_fixed_gain(gc_index, predicted_gain);
        
        // 5. Update predictor
        self.predictor.update(quantized_fixed);
        
        QuantizedGains {
            adaptive_gain: quantized_adaptive,
            fixed_gain: quantized_fixed,
            gain_indices: [ga_index, gc_index],
        }
    }
    
    /// Search gain codebook (simplified)
    fn search_gain_codebook(
        &self,
        ga_target: Q15,
        gc_target: Q15,
        gc_pred: Q15,
        corr_xh: Q31,
        corr_xf: Q31,
        corr_hf: Q31,
        energy_h: Q31,
        energy_f: Q31,
    ) -> (u8, u8) {
        // G.729A uses 3+4 bit joint scalar/predictive VQ
        // We search for optimal (gp, gc) that minimizes:
        // E = ||x - gp*y - gc*z||^2
        
        let mut best_ga_idx = 0u8;
        let mut best_gc_idx = 0u8;
        let mut best_criterion = Q31(i32::MIN);
        
        #[cfg(debug_assertions)]
        {
            eprintln!("    Searching {} x {} gain combinations", 8, 16);
            eprintln!("    Target gains: ga={}, gc={}, pred={}", ga_target.0, gc_target.0, gc_pred.0);
        }
        
        // Precompute some terms for efficiency
        let energy_x = corr_xh.saturating_add(corr_xf); // Simplified
        
        // Search adaptive gain (3 bits = 8 values)
        for ga_idx in 0..8 {
            let ga = self.decode_adaptive_gain(ga_idx);
            
            // Search fixed gain correction (4 bits = 16 values)
            for gc_idx in 0..16 {
                let gc = self.decode_fixed_gain(gc_idx, gc_pred);
                
                // Compute selection criterion
                // C = (gp*Rxy + gc*Rxz)^2 / (gp^2*Ryy + gc^2*Rzz + 2*gp*gc*Ryz)
                
                // Numerator
                let term1 = (ga.to_q31().0 as i64 * corr_xh.0 as i64 >> 31) as i32;
                let term2 = (gc.to_q31().0 as i64 * corr_xf.0 as i64 >> 31) as i32;
                let numerator = Q31(term1.saturating_add(term2));
                
                // Denominator
                let ga_sq = (ga.0 as i32 * ga.0 as i32) >> 15;
                let gc_sq = (gc.0 as i32 * gc.0 as i32) >> 15;
                let ga_gc = (ga.0 as i32 * gc.0 as i32) >> 14; // *2
                
                let denom_term1 = (ga_sq as i64 * energy_h.0 as i64 >> 31) as i32;
                let denom_term2 = (gc_sq as i64 * energy_f.0 as i64 >> 31) as i32;
                let denom_term3 = (ga_gc as i64 * corr_hf.0 as i64 >> 31) as i32;
                let denominator = Q31(denom_term1.saturating_add(denom_term2).saturating_add(denom_term3));
                
                // Compute criterion (avoid division)
                if denominator.0 > 0 {
                    // Approximate num^2/den by comparing num^2 * 1/den
                    let num_sq = (numerator.0 as i64 * numerator.0 as i64 >> 31) as i32;
                    let criterion = Q31(num_sq / (denominator.0 >> 16).max(1));
                    
                    if criterion.0 > best_criterion.0 {
                        best_criterion = criterion;
                        best_ga_idx = ga_idx;
                        best_gc_idx = gc_idx;
                    }
                }
            }
        }
        
        (best_ga_idx, best_gc_idx)
    }
    
    /// Decode adaptive gain from index
    fn decode_adaptive_gain(&self, index: u8) -> Q15 {
        // In G.729A, adaptive gain is NOT predicted
        // The table directly contains the gain values
        // GBK1[i][0] is the adaptive gain in Q14 format
        let mapped_idx = IMAP1[(index & 0x7) as usize] as usize;
        
        // Get gain value from table and convert Q14 to Q15
        let gain_q14 = GBK1[mapped_idx][0];
        Q15((gain_q14 as i32 * 2) as i16)
    }
    
    /// Decode fixed gain from index and prediction
    fn decode_fixed_gain(&self, index: u8, prediction: Q15) -> Q15 {
        // Fixed gain uses predictive quantization
        // gc = gbk2_correction * predicted_gain
        let mapped_idx = IMAP2[(index & 0xF) as usize] as usize;
        let correction_q14 = GBK2[mapped_idx][0];
        
        // Apply correction to prediction: gc = correction * prediction
        // Both are in similar Q format, result in Q15
        let result = ((correction_q14 as i64 * prediction.0 as i64) >> 14) as i32;
        Q15(result.clamp(i16::MIN as i32, i16::MAX as i32) as i16)
    }
    
    /// Decode gains from indices (for decoder)
    pub fn decode(&mut self, indices: &[u8; 2]) -> QuantizedGains {
        let ga_index = indices[0];
        let gc_index = indices[1];
        
        let predicted_gain = self.predictor.predict();
        
        let adaptive_gain = self.decode_adaptive_gain(ga_index);
        let fixed_gain = self.decode_fixed_gain(gc_index, predicted_gain);
        
        self.predictor.update(fixed_gain);
        
        QuantizedGains {
            adaptive_gain,
            fixed_gain,
            gain_indices: *indices,
        }
    }
}

/// Apply gains to excitation vectors
pub fn apply_gains(
    adaptive_vector: &[Q15],
    fixed_vector: &[Q15],
    adaptive_gain: Q15,
    fixed_gain: Q15,
) -> Vec<Q15> {
    let mut excitation = Vec::with_capacity(adaptive_vector.len());
    
    for i in 0..adaptive_vector.len() {
        let adaptive_contrib = adaptive_vector[i].saturating_mul(adaptive_gain);
        let fixed_contrib = fixed_vector[i].saturating_mul(fixed_gain);
        let total = adaptive_contrib.saturating_add(fixed_contrib);
        excitation.push(total);
    }
    
    excitation
}

impl Default for GainQuantizer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gain_predictor() {
        let predictor = GainPredictor::new();
        
        // Initial prediction should be near zero
        let pred = predictor.predict();
        assert_eq!(pred.0, 0);
    }

    #[test]
    fn test_gain_predictor_update() {
        let mut predictor = GainPredictor::new();
        
        // Update with some gains
        predictor.update(Q15::from_f32(0.5));
        predictor.update(Q15::from_f32(0.6));
        
        // Prediction should be non-zero now
        let pred = predictor.predict();
        assert!(pred.0 > 0);
    }

    #[test]
    fn test_gain_quantizer_creation() {
        let quantizer = GainQuantizer::new();
        // Just ensure it creates without panic
        assert!(true);
    }

    #[test]
    fn test_decode_gains() {
        let mut quantizer = GainQuantizer::new();
        
        let indices = [4u8, 8u8]; // Mid-range values
        let gains = quantizer.decode(&indices);
        
        // Check we get reasonable gains
        assert!(gains.adaptive_gain.0 > 0);
        assert!(gains.fixed_gain.0 >= 0);
        assert_eq!(gains.gain_indices, indices);
    }

    #[test]
    fn test_apply_gains() {
        let adaptive = vec![Q15::from_f32(0.5); 10];
        let fixed = vec![Q15::from_f32(0.3); 10];
        let ga = Q15::from_f32(0.8);
        let gc = Q15::from_f32(0.6);
        
        let excitation = apply_gains(&adaptive, &fixed, ga, gc);
        
        assert_eq!(excitation.len(), 10);
        
        // Check approximate result: 0.5 * 0.8 + 0.3 * 0.6 = 0.58
        for sample in &excitation {
            assert!((sample.to_f32() - 0.58).abs() < 0.05);
        }
    }

    #[test]
    fn test_quantize_gains() {
        let mut quantizer = GainQuantizer::new();
        
        let adaptive_vector = vec![Q15::from_f32(0.5); SUBFRAME_SIZE];
        let fixed_vector = vec![Q15::from_f32(0.3); SUBFRAME_SIZE];
        let target = vec![Q15::from_f32(0.4); SUBFRAME_SIZE];
        
        let ga_target = Q15::from_f32(0.7);
        let gc_target = Q15::from_f32(0.5);
        
        let result = quantizer.quantize(
            ga_target,
            gc_target,
            &adaptive_vector,
            &fixed_vector,
            &target,
        );
        
        // Check indices are in valid range
        assert!(result.gain_indices[0] < 8);  // 3 bits
        assert!(result.gain_indices[1] < 16); // 4 bits
        
        // Check gains are reasonable
        assert!(result.adaptive_gain.0 >= 0);
        assert!(result.fixed_gain.0 >= 0);
    }
} 