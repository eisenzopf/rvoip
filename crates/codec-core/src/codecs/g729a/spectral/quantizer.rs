//! LSP quantization using predictive two-stage vector quantization

use crate::codecs::g729a::constants::*;
use crate::codecs::g729a::types::{Q15, LSPParameters, QuantizedLSP};
use crate::codecs::g729a::math::FixedPointOps;
use crate::codecs::g729a::tables::{LSP_CB1, LSP_CB2, MEAN_LSP, LSP_PRED_COEF, q13_row_to_q15};

/// LSP codebook structure
pub struct LSPCodebooks {
    // TODO: Add actual codebook tables
    stage1_codebook: Vec<Vec<Q15>>,
    stage2_codebook_lower: Vec<Vec<Q15>>,
    stage2_codebook_upper: Vec<Vec<Q15>>,
}

/// LSP predictor for MA prediction
pub struct LSPPredictor {
    /// Previous quantized LSP values for MA prediction
    prev_lsp: [[Q15; LP_ORDER]; 4],
    /// Predictor coefficients
    ma_coeffs: [Q15; 4],
}

impl LSPPredictor {
    /// Create a new LSP predictor
    pub fn new() -> Self {
        // Initialize with default LSP values (from MEAN_LSP)
        let default_lsp = MEAN_LSP.iter()
            .map(|&val| Q15((val as i32 * 4) as i16)) // Q13 to Q15
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        
        // Use actual MA coefficients from tables (convert from Q13 to Q15)
        let ma_coeffs = LSP_PRED_COEF.iter()
            .map(|&val| Q15((val as i32 * 4) as i16))
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        
        Self {
            prev_lsp: [default_lsp; 4],
            ma_coeffs,
        }
    }
    
    /// Predict next LSP based on past values
    pub fn predict(&self) -> [Q15; LP_ORDER] {
        let mut prediction = [Q15::ZERO; LP_ORDER];
        
        // MA prediction: sum(ma_coeff[i] * prev_lsp[i])
        for i in 0..LP_ORDER {
            let mut sum = Q15::ZERO;
            for j in 0..4 {
                let prod = self.ma_coeffs[j].saturating_mul(self.prev_lsp[j][i]);
                sum = sum.saturating_add(prod);
            }
            prediction[i] = sum;
        }
        
        prediction
    }
    
    /// Update predictor state with new quantized LSP
    pub fn update(&mut self, quantized_lsp: &[Q15; LP_ORDER]) {
        // Shift previous values
        for i in (1..4).rev() {
            self.prev_lsp[i] = self.prev_lsp[i - 1];
        }
        self.prev_lsp[0] = *quantized_lsp;
    }
}

/// LSP quantizer
pub struct LSPQuantizer {
    codebooks: LSPCodebooks,
    predictor: LSPPredictor,
}

impl LSPQuantizer {
    /// Create a new LSP quantizer
    pub fn new() -> Self {
        Self {
            codebooks: LSPCodebooks {
                // Initialize with actual codebook data from tables
                stage1_codebook: LSP_CB1.iter()
                    .map(|row| q13_row_to_q15(row))
                    .collect(),
                stage2_codebook_lower: LSP_CB2[0..16].iter()
                    .map(|row| q13_row_to_q15(&row[0..5]))
                    .collect(),
                stage2_codebook_upper: LSP_CB2[16..32].iter()
                    .map(|row| q13_row_to_q15(&row[5..10]))
                    .collect(),
            },
            predictor: LSPPredictor::new(),
        }
    }
    
    /// Quantize LSP parameters
    pub fn quantize(&mut self, lsp: &LSPParameters) -> QuantizedLSP {
        // 1. Compute residual from prediction
        let prediction = self.predictor.predict();
        let mut residual = [Q15::ZERO; LP_ORDER];
        
        for i in 0..LP_ORDER {
            residual[i] = lsp.frequencies[i].saturating_add(Q15(-prediction[i].0));
        }
        
        // 2. Apply mean removal
        let mean_lsp = self.get_mean_lsp();
        for i in 0..LP_ORDER {
            residual[i] = residual[i].saturating_add(Q15(-mean_lsp[i].0));
        }
        
        // 3. First stage: 7-bit VQ on first 5 elements
        let (stage1_idx, stage1_quant) = self.vq_stage1(&residual[0..5]);
        
        // 4. Compute second stage residual
        let mut stage2_residual = [Q15::ZERO; LP_ORDER];
        for i in 0..5 {
            stage2_residual[i] = residual[i].saturating_add(Q15(-stage1_quant[i].0));
        }
        for i in 5..10 {
            stage2_residual[i] = residual[i];
        }
        
        // 5. Second stage: Two 5-bit VQs
        let (stage2_idx_lower, stage2_quant_lower) = self.vq_stage2_lower(&stage2_residual[0..5]);
        let (stage2_idx_upper, stage2_quant_upper) = self.vq_stage2_upper(&stage2_residual[5..10]);
        
        // 6. Reconstruct quantized LSP
        let mut reconstructed = [Q15::ZERO; LP_ORDER];
        
        // Add all components back
        for i in 0..5 {
            reconstructed[i] = mean_lsp[i]
                .saturating_add(prediction[i])
                .saturating_add(stage1_quant[i])
                .saturating_add(stage2_quant_lower[i]);
        }
        for i in 5..10 {
            reconstructed[i] = mean_lsp[i]
                .saturating_add(prediction[i])
                .saturating_add(stage2_quant_upper[i-5]);
        }
        
        // 7. Check stability and adjust if needed
        self.ensure_lsp_stability(&mut reconstructed);
        
        // 8. Update predictor
        self.predictor.update(&reconstructed);
        
        QuantizedLSP {
            indices: [stage1_idx, stage2_idx_lower, stage2_idx_upper, 0],
            reconstructed: LSPParameters {
                frequencies: reconstructed,
            },
        }
    }
    
    /// First stage vector quantization
    fn vq_stage1(&self, residual: &[Q15]) -> (u8, Vec<Q15>) {
        let mut best_idx = 0u8;
        let mut best_dist = i32::MAX;
        let mut best_vector = vec![Q15::ZERO; 5];
        
        // Search all 128 codebook entries
        for (idx, codebook_entry) in self.codebooks.stage1_codebook.iter().enumerate() {
            let dist = self.weighted_distance(residual, codebook_entry, &[0, 1, 2, 3, 4]);
            
            if dist < best_dist {
                best_dist = dist;
                best_idx = idx as u8;
                best_vector = codebook_entry.clone();
            }
        }
        
        (best_idx, best_vector)
    }
    
    /// Second stage lower vector quantization
    fn vq_stage2_lower(&self, residual: &[Q15]) -> (u8, Vec<Q15>) {
        let mut best_idx = 0u8;
        let mut best_dist = i32::MAX;
        let mut best_vector = vec![Q15::ZERO; 5];
        
        for (idx, codebook_entry) in self.codebooks.stage2_codebook_lower.iter().enumerate() {
            let dist = self.weighted_distance(residual, codebook_entry, &[0, 1, 2, 3, 4]);
            
            if dist < best_dist {
                best_dist = dist;
                best_idx = idx as u8;
                best_vector = codebook_entry.clone();
            }
        }
        
        (best_idx, best_vector)
    }
    
    /// Second stage upper vector quantization
    fn vq_stage2_upper(&self, residual: &[Q15]) -> (u8, Vec<Q15>) {
        let mut best_idx = 0u8;
        let mut best_dist = i32::MAX;
        let mut best_vector = vec![Q15::ZERO; 5];
        
        for (idx, codebook_entry) in self.codebooks.stage2_codebook_upper.iter().enumerate() {
            let dist = self.weighted_distance(residual, codebook_entry, &[5, 6, 7, 8, 9]);
            
            if dist < best_dist {
                best_dist = dist;
                best_idx = idx as u8;
                best_vector = codebook_entry.clone();
            }
        }
        
        (best_idx, best_vector)
    }
    
    /// Compute weighted Euclidean distance
    fn weighted_distance(&self, a: &[Q15], b: &[Q15], indices: &[usize]) -> i32 {
        let weights = self.get_lsp_weights(indices);
        let mut dist = 0i32;
        
        for (i, &idx) in indices.iter().enumerate().take(a.len().min(b.len())) {
            let diff = (a[i].0 as i32) - (b[i].0 as i32);
            let weighted_diff = ((diff * weights[idx].0 as i32) >> 15) as i16;
            dist += (weighted_diff as i32 * weighted_diff as i32) >> 15;
        }
        
        dist
    }
    
    /// Get LSP weighting factors based on distance between adjacent LSPs
    fn get_lsp_weights(&self, indices: &[usize]) -> [Q15; LP_ORDER] {
        // Simplified weights - in real G.729A these depend on LSP spacing
        [Q15::from_f32(1.0); LP_ORDER]
    }
    
    /// Get mean LSP values
    fn get_mean_lsp(&self) -> [Q15; LP_ORDER] {
        // Use actual mean LSP values from tables (convert from Q13 to Q15)
        let mut mean = [Q15::ZERO; LP_ORDER];
        for i in 0..LP_ORDER {
            mean[i] = Q15((MEAN_LSP[i] as i32 * 4) as i16);
        }
        mean
    }
    
    /// Ensure LSP stability constraints
    fn ensure_lsp_stability(&self, lsp: &mut [Q15; LP_ORDER]) {
        // Minimum separation in G.729A
        let min_gap = Q15((0.0391 * Q15_ONE as f32) as i16);
        
        // Sort if needed
        for i in 1..LP_ORDER {
            if lsp[i].0 < lsp[i-1].0 {
                lsp.sort_by_key(|x| x.0);
                break;
            }
        }
        
        // Enforce minimum gaps
        for i in 1..LP_ORDER {
            let gap = lsp[i].0.saturating_sub(lsp[i-1].0);
            if gap < min_gap.0 {
                lsp[i] = Q15(lsp[i-1].0.saturating_add(min_gap.0));
            }
        }
        
        // Ensure within valid range [0, π]
        if lsp[0].0 < 0 {
            lsp[0] = Q15(min_gap.0);
        }
        if lsp[LP_ORDER-1].0 > Q15_ONE - min_gap.0 {
            lsp[LP_ORDER-1] = Q15(Q15_ONE - min_gap.0);
        }
    }
}

/// LSP decoder (for decoder side)
pub struct LSPDecoder {
    codebooks: LSPCodebooks,
    predictor: LSPPredictor,
}

impl LSPDecoder {
    /// Create a new LSP decoder
    pub fn new() -> Self {
        Self {
            codebooks: LSPCodebooks {
                // TODO: Same codebooks as encoder
                stage1_codebook: vec![vec![Q15::ZERO; 5]; 128],
                stage2_codebook_lower: vec![vec![Q15::ZERO; 5]; 32],
                stage2_codebook_upper: vec![vec![Q15::ZERO; 5]; 32],
            },
            predictor: LSPPredictor::new(),
        }
    }
    
    /// Decode LSP from indices
    pub fn decode(&mut self, indices: &[u8; 4]) -> LSPParameters {
        // TODO: Implement actual decoding from codebook indices
        let frequencies = [Q15::ZERO; LP_ORDER];
        
        LSPParameters { frequencies }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lsp_predictor() {
        let predictor = LSPPredictor::new();
        let prediction = predictor.predict();
        
        // Should return non-zero prediction based on initialization
        let sum: i32 = prediction.iter().map(|&x| x.0 as i32).sum();
        assert!(sum != 0);
    }

    #[test]
    fn test_lsp_predictor_update() {
        let mut predictor = LSPPredictor::new();
        
        // Create new LSP values
        let new_lsp = [Q15::from_f32(0.1); LP_ORDER];
        
        // Update predictor
        predictor.update(&new_lsp);
        
        // Check that first previous LSP is updated
        assert_eq!(predictor.prev_lsp[0][0], new_lsp[0]);
    }

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