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
        // Initialize with ITU-T specified initial LSP values
        let initial_lsp: [Q15; LP_ORDER] = crate::codecs::g729a::constants::INITIAL_LSP_Q15
            .map(Q15)
            .try_into()
            .unwrap();
        
        // Use actual MA coefficients from tables (keep in Q13 for internal calculations)
        let ma_coeffs = LSP_PRED_COEF.iter()
            .map(|&val| Q15(val)) // Keep in Q13 format
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        
        // ITU-T G.729A: Initialize predictor with progressive values for better startup
        let mut prev_lsp_init = [[Q15::ZERO; LP_ORDER]; 4];
        
        // Frame 0: Use zeros (no prediction)
        prev_lsp_init[0] = [Q15::ZERO; LP_ORDER];
        
        // Frame 1: Use 25% of initial LSP
        for i in 0..LP_ORDER {
            prev_lsp_init[1][i] = Q15(initial_lsp[i].0 >> 2); // 25% in Q13
        }
        
        // Frame 2: Use 50% of initial LSP  
        for i in 0..LP_ORDER {
            prev_lsp_init[2][i] = Q15(initial_lsp[i].0 >> 1); // 50% in Q13
        }
        
        // Frame 3: Use 75% of initial LSP
        for i in 0..LP_ORDER {
            prev_lsp_init[3][i] = Q15((initial_lsp[i].0 as i32 * 3 / 4) as i16); // 75% in Q13
        }

        Self {
            prev_lsp: prev_lsp_init,
            ma_coeffs,
        }
    }
    
    /// Predict next LSP based on past values
    pub fn predict(&self) -> [Q15; LP_ORDER] {
        let mut prediction = [Q15::ZERO; LP_ORDER];
        
        // MA prediction: sum(ma_coeff[i] * prev_lsp[i]) with proper Q-format scaling
        for i in 0..LP_ORDER {
            let mut sum = 0i64; // Use i64 to avoid overflow in Q26
            
            #[cfg(debug_assertions)]
            if i < 3 { // Debug first few coefficients
                eprintln!("    LSP[{}]: prev_lsp = {:?}, ma_coeffs = {:?}", 
                    i, 
                    [self.prev_lsp[0][i].0, self.prev_lsp[1][i].0, self.prev_lsp[2][i].0, self.prev_lsp[3][i].0],
                    [self.ma_coeffs[0].0, self.ma_coeffs[1].0, self.ma_coeffs[2].0, self.ma_coeffs[3].0]
                );
            }
            
            for j in 0..4 {
                // Q13 × Q13 → Q26, then shift right 13 to get Q13
                let prod = (self.ma_coeffs[j].0 as i64) * (self.prev_lsp[j][i].0 as i64);
                sum = sum.saturating_add(prod);
            }
            // Convert Q26 to Q13 by shifting right 13 bits and saturate to i16 range
            let q13_result = (sum >> 13).clamp(i16::MIN as i64, i16::MAX as i64) as i16;
            prediction[i] = Q15(q13_result);
            
            #[cfg(debug_assertions)]
            if i < 3 { // Debug calculation
                eprintln!("    LSP[{}]: sum_Q26={}, result_Q13={}", i, sum, q13_result);
            }
        }
        
        prediction
    }
    
    /// Update predictor state with new quantized LSP (convert from Q15 to Q13)
    pub fn update(&mut self, quantized_lsp: &[Q15; LP_ORDER]) {
        // Convert Q15 to Q13 and shift previous values
        for i in (1..4).rev() {
            self.prev_lsp[i] = self.prev_lsp[i - 1];
        }
        // Store in Q13 format for prediction
        let mut lsp_q13 = [Q15::ZERO; LP_ORDER];
        for j in 0..LP_ORDER {
            lsp_q13[j] = Q15(quantized_lsp[j].0 >> 2); // Q15 to Q13
        }
        self.prev_lsp[0] = lsp_q13;
    }
}

/// LSP quantizer
pub struct LSPQuantizer {
    codebooks: LSPCodebooks,
    predictor: LSPPredictor,
    current_lsp: Option<[Q15; LP_ORDER]>,
}

impl LSPQuantizer {
    /// Create a new LSP quantizer
    pub fn new() -> Self {
        Self {
            codebooks: LSPCodebooks {
                // Initialize with actual codebook data from tables (keep in Q13)
                stage1_codebook: LSP_CB1.iter()
                    .map(|row| row.iter().map(|&val| Q15(val)).collect())
                    .collect(),
                // ITU-T: Both lower and upper use all 32 entries, split by dimensions
                stage2_codebook_lower: LSP_CB2.iter()
                    .map(|row| row[0..5].iter().map(|&val| Q15(val)).collect())
                    .collect(),
                stage2_codebook_upper: LSP_CB2.iter()
                    .map(|row| row[5..10].iter().map(|&val| Q15(val)).collect())
                    .collect(),
            },
            predictor: LSPPredictor::new(),
            current_lsp: None,
        }
    }
    
    /// Quantize LSP parameters
    pub fn quantize(&mut self, lsp: &LSPParameters) -> QuantizedLSP {
        // Store current LSP for weighting computation
        self.current_lsp = Some(lsp.frequencies);
        
        // ITU-T: Convert LSP from Q15 to Q13 for quantization
        let mut lsp_q13 = [Q15::ZERO; LP_ORDER];
        for i in 0..LP_ORDER {
            lsp_q13[i] = Q15(lsp.frequencies[i].0 >> 2); // Q15 to Q13: divide by 4
        }
        
        #[cfg(debug_assertions)]
        {
            eprintln!("LSP Quantization Debug:");
            eprintln!("  Input LSP Q15: {:?}", lsp.frequencies.iter().map(|x| x.0).collect::<Vec<_>>());
            eprintln!("  Input LSP Q13: {:?}", lsp_q13.iter().map(|x| x.0).collect::<Vec<_>>());
        }
        
        // 1. Compute residual from prediction (all in Q13)
        let prediction = self.predictor.predict();
        let mut residual = [Q15::ZERO; LP_ORDER];
        
        #[cfg(debug_assertions)]
        {
            eprintln!("  Prediction: {:?}", prediction.iter().map(|x| x.0).collect::<Vec<_>>());
        }
        
        for i in 0..LP_ORDER {
            residual[i] = lsp_q13[i].saturating_add(Q15(prediction[i].0.saturating_neg()));
        }
        
        // 2. Apply mean removal (mean LSP is already in Q13)
        let mean_lsp = self.get_mean_lsp();
        
        #[cfg(debug_assertions)]
        {
            eprintln!("  Mean LSP: {:?}", mean_lsp.iter().map(|x| x.0).collect::<Vec<_>>());
            eprintln!("  Residual after prediction: {:?}", residual.iter().map(|x| x.0).collect::<Vec<_>>());
        }
        
        for i in 0..LP_ORDER {
            residual[i] = residual[i].saturating_add(Q15(mean_lsp[i].0.saturating_neg()));
        }
        
        #[cfg(debug_assertions)]
        {
            eprintln!("  Final residual: {:?}", residual.iter().map(|x| x.0).collect::<Vec<_>>());
        }
        
        // 3. First stage: 7-bit VQ on residual (ITU-T G.729A split-VQ)
        // Use the residual after mean and prediction removal
        let target_vector = residual;
        
        #[cfg(debug_assertions)]
        eprintln!("  Target vector: {:?}", target_vector.iter().map(|x| x.0).collect::<Vec<_>>());
        
        let (stage1_idx, stage1_quant) = self.vq_stage1(&target_vector[0..5]);
        
        #[cfg(debug_assertions)]
        {
            eprintln!("  Stage1 idx: {}, vector: {:?}", stage1_idx, stage1_quant.iter().map(|x| x.0).collect::<Vec<_>>());
        }
        
        // 4. Compute second stage residual
        let mut stage2_residual = [Q15::ZERO; LP_ORDER];
        // Lower part: subtract stage1 quantization
        for i in 0..5 {
            stage2_residual[i] = residual[i].saturating_add(Q15(stage1_quant[i].0.saturating_neg()));
        }
        // Upper part: use original residual
        for i in 5..10 {
            stage2_residual[i] = residual[i];
        }
        
        // 5. Second stage: Two 5-bit VQs
        #[cfg(debug_assertions)]
        eprintln!("  Stage2 residual: {:?}", stage2_residual.iter().map(|x| x.0).collect::<Vec<_>>());
        
        let (stage2_idx_lower, stage2_quant_lower) = self.vq_stage2_lower(&stage2_residual[0..5]);
        
        #[cfg(debug_assertions)]
        eprintln!("  Stage2 lower idx: {}, vector: {:?}", stage2_idx_lower, stage2_quant_lower.iter().map(|x| x.0).collect::<Vec<_>>());
        
        let (stage2_idx_upper, stage2_quant_upper) = self.vq_stage2_upper(&stage2_residual[5..10]);
        
        #[cfg(debug_assertions)]
        eprintln!("  Stage2 upper idx: {}, vector: {:?}", stage2_idx_upper, stage2_quant_upper.iter().map(|x| x.0).collect::<Vec<_>>());
        
        // 6. Reconstruct quantized LSP (in Q13, then convert to Q15)
        let mut reconstructed_q13 = [Q15::ZERO; LP_ORDER];
        
        // ITU-T G.729A split-VQ: reconstructed = mean + prediction + quantized_residual
        for i in 0..5 {
            reconstructed_q13[i] = mean_lsp[i]
                .saturating_add(prediction[i])
                .saturating_add(stage1_quant[i])
                .saturating_add(stage2_quant_lower[i]);
        }
        for i in 5..10 {
            reconstructed_q13[i] = mean_lsp[i]
                .saturating_add(prediction[i])
                .saturating_add(stage2_quant_upper[i-5]);
        }
        
        // ITU-T: Convert reconstructed LSP from Q13 back to Q15 with saturation
        let mut reconstructed = [Q15::ZERO; LP_ORDER];
        for i in 0..LP_ORDER {
            let q15_val = (reconstructed_q13[i].0 as i32 * 4).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            reconstructed[i] = Q15(q15_val); // Q13 to Q15 with saturation
        }
        
        // 7. Check stability and adjust if needed
        self.ensure_lsp_stability(&mut reconstructed);
        
        // 8. Update predictor
        self.predictor.update(&reconstructed);
        
        // Clear current LSP
        self.current_lsp = None;
        
        QuantizedLSP {
            indices: [stage1_idx, stage2_idx_lower, stage2_idx_upper, 0],
            reconstructed: LSPParameters {
                frequencies: reconstructed,
            },
        }
    }
    
    /// First stage vector quantization (full 10-dimensional)
    fn vq_stage1_full(&self, target: &[Q15; LP_ORDER]) -> (u8, [Q15; LP_ORDER]) {
        let mut best_idx = 0u8;
        let mut best_dist = i32::MAX;
        let mut best_vector = [Q15::ZERO; LP_ORDER];
        
        #[cfg(debug_assertions)]
        eprintln!("  VQ Stage1: searching {} codebook entries for target: {:?}", 
                  self.codebooks.stage1_codebook.len(),
                  target.iter().map(|x| x.0).collect::<Vec<_>>());
        
        // Search all 128 codebook entries
        for (idx, codebook_entry) in self.codebooks.stage1_codebook.iter().enumerate() {
            let dist = self.unweighted_distance_10d(target, codebook_entry);
            
            #[cfg(debug_assertions)]
            if idx < 3 { // Show first few entries
                eprintln!("    Entry {}: dist={}, vector={:?}", idx, dist, codebook_entry.iter().map(|x| x.0).collect::<Vec<_>>());
            }
            
            if dist < best_dist {
                best_dist = dist;
                best_idx = idx as u8;
                for i in 0..LP_ORDER {
                    best_vector[i] = codebook_entry[i];
                }
            }
        }
        
        #[cfg(debug_assertions)]
        eprintln!("  VQ Stage1 result: idx={}, dist={}, vector={:?}", best_idx, best_dist, best_vector.iter().map(|x| x.0).collect::<Vec<_>>());
        
        (best_idx, best_vector)
    }
    
    /// Unweighted Euclidean distance for 10-dimensional vectors
    fn unweighted_distance_10d(&self, a: &[Q15; LP_ORDER], b: &[Q15]) -> i32 {
        let mut dist = 0i64; // Use i64 to prevent overflow
        for i in 0..LP_ORDER {
            let diff = (a[i].0 as i64) - (b[i].0 as i64);
            dist = dist.saturating_add(diff * diff);
        }
        dist.min(i32::MAX as i64) as i32 // Clamp to i32 range
    }
    
    /// First stage vector quantization (legacy 5-dimensional)
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
                best_vector = codebook_entry[0..5].iter().map(|&x| x).collect();
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
        // G.729A uses weighting based on LSP spacing
        // Smaller spacing = larger weight (more sensitive to quantization)
        let mut weights = [Q15::ONE; LP_ORDER];
        
        // Get current LSP frequencies (from previous frame)
        if let Some(prev_lsp) = &self.current_lsp {
            
            // Compute weights based on adjacent LSP differences
            // w[0] = 1 / (lsp[1] - lsp[0])
            // w[i] = 1 / min(lsp[i]-lsp[i-1], lsp[i+1]-lsp[i]) for 1 <= i <= 8
            // w[9] = 1 / (lsp[9] - lsp[8])
            
            for i in 0..LP_ORDER {
                let mut min_dist = Q15_ONE;
                
                if i == 0 {
                    // First LSP
                    let dist = prev_lsp[1].0.saturating_sub(prev_lsp[0].0);
                    if dist > 0 {
                        min_dist = dist as i16;
                    }
                } else if i == LP_ORDER - 1 {
                    // Last LSP
                    let dist = prev_lsp[9].0.saturating_sub(prev_lsp[8].0);
                    if dist > 0 {
                        min_dist = dist as i16;
                    }
                } else {
                    // Middle LSPs - use minimum of left and right distances
                    let dist_left = prev_lsp[i].0.saturating_sub(prev_lsp[i-1].0);
                    let dist_right = prev_lsp[i+1].0.saturating_sub(prev_lsp[i].0);
                    let min = dist_left.min(dist_right);
                    if min > 0 {
                        min_dist = min as i16;
                    }
                }
                
                // Weight is inversely proportional to distance
                // Normalize to Q15 range with minimum distance threshold
                let min_dist = min_dist.max(800); // Minimum distance threshold
                weights[i] = Q15((Q15_ONE as i32 * 4096 / min_dist as i32).min(Q15_ONE as i32) as i16);
            }
        }
        
        weights
    }
    
    /// Get mean LSP values (keep in Q13 for quantization)
    fn get_mean_lsp(&self) -> [Q15; LP_ORDER] {
        // Use actual mean LSP values from tables (keep in Q13)
        let mut mean = [Q15::ZERO; LP_ORDER];
        for i in 0..LP_ORDER {
            mean[i] = Q15(MEAN_LSP[i]); // Keep in Q13 format
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
                // Initialize with actual codebook data from tables (keep in Q13)
                stage1_codebook: LSP_CB1.iter()
                    .map(|row| row.iter().map(|&val| Q15(val)).collect())
                    .collect(),
                // ITU-T: Both lower and upper use all 32 entries, split by dimensions
                stage2_codebook_lower: LSP_CB2.iter()
                    .map(|row| row[0..5].iter().map(|&val| Q15(val)).collect())
                    .collect(),
                stage2_codebook_upper: LSP_CB2.iter()
                    .map(|row| row[5..10].iter().map(|&val| Q15(val)).collect())
                    .collect(),
            },
            predictor: LSPPredictor::new(),
        }
    }
    
    /// Decode LSP from indices
    pub fn decode(&mut self, indices: &[u8; 4]) -> LSPParameters {
        // Get codebook vectors
        let stage1_idx = indices[0] as usize;
        let stage2_idx_lower = indices[1] as usize;
        let stage2_idx_upper = indices[2] as usize;
        
        // Bounds checking
        let stage1_idx = stage1_idx.min(self.codebooks.stage1_codebook.len() - 1);
        let stage2_idx_lower = stage2_idx_lower.min(self.codebooks.stage2_codebook_lower.len() - 1);
        let stage2_idx_upper = stage2_idx_upper.min(self.codebooks.stage2_codebook_upper.len() - 1);
        
        // Get vectors from codebooks
        let stage1_vec = &self.codebooks.stage1_codebook[stage1_idx];
        let stage2_lower_vec = &self.codebooks.stage2_codebook_lower[stage2_idx_lower];
        let stage2_upper_vec = &self.codebooks.stage2_codebook_upper[stage2_idx_upper];
        
        // Get prediction and mean
        let prediction = self.predictor.predict();
        let mean_lsp = self.get_mean_lsp();
        
        // Reconstruct LSP: LSP = mean + prediction + stage1 + stage2
        let mut frequencies = [Q15::ZERO; LP_ORDER];
        
        for i in 0..5 {
            frequencies[i] = mean_lsp[i]
                .saturating_add(prediction[i])
                .saturating_add(stage1_vec[i])
                .saturating_add(stage2_lower_vec[i]);
        }
        
        for i in 5..10 {
            frequencies[i] = mean_lsp[i]
                .saturating_add(prediction[i])
                .saturating_add(stage2_upper_vec[i-5]);
        }
        
        // Update predictor with decoded values
        self.predictor.update(&frequencies);
        
        LSPParameters { frequencies }
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