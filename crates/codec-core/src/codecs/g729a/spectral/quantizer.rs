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
    /// Two MA predictor sets (switched by L0 bit)
    ma_predictors: [[[i16; LP_ORDER]; 4]; 2], // [predictor_set][frame][coeff]
}

impl LSPPredictor {
    /// Create a new LSP predictor with ITU-T G.729A dual predictors
    pub fn new() -> Self {
        // ITU-T G.729A uses two different MA predictor sets
        // Each predictor has 4 previous frames × 10 coefficients
        let ma_predictors = [
            // Predictor 0 (default)
            [
                [5571, 4751, 2785, 1556, 1268, 1021, 819, 657, 527, 423], // Frame 0
                [4751, 2785, 1556, 1268, 1021, 819, 657, 527, 423, 339],  // Frame 1  
                [2785, 1556, 1268, 1021, 819, 657, 527, 423, 339, 272],   // Frame 2
                [1556, 1268, 1021, 819, 657, 527, 423, 339, 272, 218],    // Frame 3
            ],
            // Predictor 1 (alternative)
            [
                [5016, 4231, 2478, 1384, 1097, 872, 690, 546, 432, 342],  // Frame 0
                [4231, 2478, 1384, 1097, 872, 690, 546, 432, 342, 271],   // Frame 1
                [2478, 1384, 1097, 872, 690, 546, 432, 342, 271, 214],    // Frame 2
                [1384, 1097, 872, 690, 546, 432, 342, 271, 214, 169],     // Frame 3
            ],
        ];

        // ITU-T G.729A: Initialize predictor with INITIAL_LSP values for realistic startup
        // Convert INITIAL_LSP_Q15 to Q13 for predictor state
        let initial_lsp_q13: [Q15; LP_ORDER] = crate::codecs::g729a::constants::INITIAL_LSP_Q15
            .map(|val| Q15(val >> 2)); // Q15 to Q13: divide by 4
        
        let mut prev_lsp_init = [[Q15::ZERO; LP_ORDER]; 4];
        
        // Initialize all 4 previous frames with the same initial LSP values
        // This provides a reasonable startup state for MA prediction
        for i in 0..4 {
            prev_lsp_init[i] = initial_lsp_q13;
        }

        Self {
            prev_lsp: prev_lsp_init,
            ma_predictors,
        }
    }
    
    /// Predict next LSP using specified predictor (0 or 1)
    pub fn predict_with_predictor(&self, predictor_idx: usize) -> [Q15; LP_ORDER] {
        let predictor_idx = predictor_idx.min(1); // Clamp to [0,1]
        let mut prediction = [Q15::ZERO; LP_ORDER];
        
        #[cfg(debug_assertions)]
        {
            eprintln!("      🔮 PREDICTOR DEBUG ({}): ", predictor_idx);
            eprintln!("        Prev LSP state: {:?}", 
                self.prev_lsp.iter().take(2).map(|frame| frame.iter().take(3).map(|x| x.0).collect::<Vec<_>>()).collect::<Vec<_>>());
            eprintln!("        MA coeffs[{}]: {:?}", predictor_idx,
                self.ma_predictors[predictor_idx].iter().take(2).map(|frame| frame.iter().take(3).collect::<Vec<_>>()).collect::<Vec<_>>());
        }
        
        // ITU-T G.729A MA prediction using selected predictor
        for i in 0..LP_ORDER {
            let mut sum = 0i64;
            
            for j in 0..4 {
                // Use the selected MA predictor coefficients (Q13 format)
                let coeff = self.ma_predictors[predictor_idx][j][i];
                let prev_val = self.prev_lsp[j][i].0;
                sum += (coeff as i64) * (prev_val as i64);
                
                #[cfg(debug_assertions)]
                if i < 3 && j < 2 {
                    eprintln!("        LSP[{}] frame[{}]: coeff={} * prev={} = {}", 
                        i, j, coeff, prev_val, (coeff as i64) * (prev_val as i64));
                }
            }
            
            // Convert Q26 to Q13 and saturate
            let q13_result = (sum >> 13).clamp(-4096, 4095) as i16;
            prediction[i] = Q15(q13_result);
            
            #[cfg(debug_assertions)]
            if i < 3 {
                eprintln!("        LSP[{}]: sum_Q26={} -> Q13={}", i, sum, q13_result);
            }
        }
        
        #[cfg(debug_assertions)]
        {
            eprintln!("        Final prediction: {:?}", prediction.iter().take(5).map(|x| x.0).collect::<Vec<_>>());
        }
        
        prediction
    }
    
    /// Legacy predict method (uses predictor 0)
    pub fn predict(&self) -> [Q15; LP_ORDER] {
        self.predict_with_predictor(0)
    }
    
    /// Update predictor state with new quantized LSP
    pub fn update(&mut self, quantized_lsp: &[Q15; LP_ORDER]) {
        // Shift previous values
        for i in (1..4).rev() {
            self.prev_lsp[i] = self.prev_lsp[i - 1];
        }
        
        // Store new values in Q13 format for prediction
        let mut lsp_q13 = [Q15::ZERO; LP_ORDER];
        for j in 0..LP_ORDER {
            let q13_val = (quantized_lsp[j].0 as i32 >> 2).clamp(-4096, 4095) as i16;
            lsp_q13[j] = Q15(q13_val);
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
    
    /// Quantize LSP parameters using ITU-T G.729A algorithm
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
            eprintln!("ITU-T G.729A LSP Quantization:");
            eprintln!("  Input LSP Q15: {:?}", lsp.frequencies.iter().map(|x| x.0).collect::<Vec<_>>());
            eprintln!("  Input LSP Q13: {:?}", lsp_q13.iter().map(|x| x.0).collect::<Vec<_>>());
        }
        
        // Try both MA predictors and select the best one (L0 bit)
        let mut best_error = f64::MAX;
        let mut best_l0 = 0u8;
        let mut best_indices = [0u8; 4];
        let mut best_reconstructed = [Q15::ZERO; LP_ORDER];
        
        for l0 in 0..2 {
            let (indices, reconstructed, error) = self.quantize_with_predictor(&lsp_q13, l0);
            
            #[cfg(debug_assertions)]
            eprintln!("  Predictor {}: error={:.2}, indices={:?}", l0, error, indices);
            
            if error < best_error {
                best_error = error;
                best_l0 = l0 as u8;
                best_indices = indices;
                best_reconstructed = reconstructed;
            }
        }
        
        #[cfg(debug_assertions)]
        {
            eprintln!("  Selected predictor: L0={}, error={:.2}", best_l0, best_error);
            eprintln!("  Final indices: {:?}", best_indices);
        }
        
        // Convert reconstructed from Q13 back to Q15
        let mut reconstructed_q15 = [Q15::ZERO; LP_ORDER];
        for i in 0..LP_ORDER {
            let q13_val = best_reconstructed[i].0 as i32;
            if q13_val > i16::MAX as i32 / 4 {
                reconstructed_q15[i] = Q15(i16::MAX);
            } else if q13_val < i16::MIN as i32 / 4 {
                reconstructed_q15[i] = Q15(i16::MIN);
            } else {
                reconstructed_q15[i] = Q15((q13_val * 4) as i16);
            }
        }
        
        // Apply stability constraints
        self.ensure_lsp_stability(&mut reconstructed_q15);
        
        // Update predictor with final result
        self.predictor.update(&reconstructed_q15);
        
        // Clear current LSP
        self.current_lsp = None;
        
        // Set L0 bit in indices[3] (using bit 0)
        let mut final_indices = best_indices;
        final_indices[3] = best_l0;
        
        QuantizedLSP {
            indices: final_indices,
            reconstructed: LSPParameters {
                frequencies: reconstructed_q15,
            },
        }
    }
    
    /// Quantize with a specific predictor and return error
    fn quantize_with_predictor(&self, lsp_q13: &[Q15; LP_ORDER], predictor_idx: usize) -> ([u8; 4], [Q15; LP_ORDER], f64) {
        // 1. Get prediction from selected predictor
        let prediction = self.predictor.predict_with_predictor(predictor_idx);
        
        // 2. Apply mean removal and prediction
        let mean_lsp = self.get_mean_lsp();
        let mut residual = [Q15::ZERO; LP_ORDER];
        
        #[cfg(debug_assertions)]
        {
            eprintln!("    📊 RESIDUAL COMPUTATION DEBUG (Predictor {}):", predictor_idx);
            eprintln!("      Input LSP Q13: {:?}", lsp_q13.iter().take(5).map(|x| x.0).collect::<Vec<_>>());
            eprintln!("      Mean LSP Q13:  {:?}", mean_lsp.iter().take(5).map(|x| x.0).collect::<Vec<_>>());
            eprintln!("      Prediction Q13: {:?}", prediction.iter().take(5).map(|x| x.0).collect::<Vec<_>>());
        }
        
        for i in 0..LP_ORDER {
            let mut val = lsp_q13[i].0 as i32;
            val -= mean_lsp[i].0 as i32;
            val -= prediction[i].0 as i32;
            residual[i] = Q15(val.clamp(-4096, 4095) as i16);
            
            #[cfg(debug_assertions)]
            if i < 5 {
                eprintln!("      LSP[{}]: {} - {} - {} = {} -> {}", 
                    i, lsp_q13[i].0, mean_lsp[i].0, prediction[i].0, 
                    lsp_q13[i].0 as i32 - mean_lsp[i].0 as i32 - prediction[i].0 as i32,
                    residual[i].0);
            }
        }
        
        #[cfg(debug_assertions)]
        {
            eprintln!("      Final residual: {:?}", residual.iter().take(5).map(|x| x.0).collect::<Vec<_>>());
            let residual_energy: i64 = residual.iter().map(|x| (x.0 as i64) * (x.0 as i64)).sum();
            eprintln!("      Residual energy: {}", residual_energy);
        }
        
        // 3. Stage 1: 7-bit VQ (128 entries, full 10D)
        let (stage1_idx, stage1_quant) = self.vq_stage1_weighted(&residual);
        
        // 4. Stage 2: Split VQ (two 5-bit, 32 entries each)
        let mut stage2_residual = [Q15::ZERO; LP_ORDER];
        for i in 0..5 {
            stage2_residual[i] = Q15((residual[i].0 as i32 - stage1_quant[i].0 as i32).clamp(-4096, 4095) as i16);
        }
        for i in 5..10 {
            stage2_residual[i] = residual[i]; // Upper part doesn't use stage1
        }
        
        let (stage2_idx_lower, stage2_quant_lower) = self.vq_stage2_weighted(&stage2_residual[0..5], &[0,1,2,3,4]);
        let (stage2_idx_upper, stage2_quant_upper) = self.vq_stage2_weighted(&stage2_residual[5..10], &[5,6,7,8,9]);
        
        // 5. Reconstruct in Q13
        let mut reconstructed = [Q15::ZERO; LP_ORDER];
        for i in 0..5 {
            let mut sum = mean_lsp[i].0 as i32;
            sum += prediction[i].0 as i32;
            sum += stage1_quant[i].0 as i32;
            sum += stage2_quant_lower[i].0 as i32;
            reconstructed[i] = Q15(sum.clamp(-4096, 4095) as i16);
        }
        for i in 5..10 {
            let mut sum = mean_lsp[i].0 as i32;
            sum += prediction[i].0 as i32;
            sum += stage2_quant_upper[i-5].0 as i32;
            reconstructed[i] = Q15(sum.clamp(-4096, 4095) as i16);
        }
        
        // Apply rearrangement for stability (twice as per ITU-T)
        self.rearrange_lsp(&mut reconstructed, 10); // J = 0.0012 in Q13 ≈ 10
        self.rearrange_lsp(&mut reconstructed, 5);  // J = 0.0006 in Q13 ≈ 5
        
        // 6. Compute weighted MSE for this predictor
        let error = self.compute_weighted_mse(&lsp_q13, &reconstructed);
        
        let indices = [stage1_idx, stage2_idx_lower, stage2_idx_upper, 0];
        (indices, reconstructed, error)
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
        
        #[cfg(debug_assertions)]
        {
            eprintln!("  VQ Stage1 (5D): searching {} codebook entries", self.codebooks.stage1_codebook.len());
            eprintln!("  Input residual: {:?}", residual.iter().take(5).map(|x| x.0).collect::<Vec<_>>());
        }
        
        // Search all 128 codebook entries
        for (idx, codebook_entry) in self.codebooks.stage1_codebook.iter().enumerate() {
            let dist = self.weighted_distance(residual, codebook_entry, &[0, 1, 2, 3, 4]);
            
            #[cfg(debug_assertions)]
            if idx < 5 || idx == 11 || dist < best_dist { // Show first few entries, idx 11, and best
                eprintln!("    Entry {}: dist={}, vector={:?}", 
                    idx, dist, codebook_entry.iter().take(5).map(|x| x.0).collect::<Vec<_>>());
            }
            
            if dist < best_dist {
                best_dist = dist;
                best_idx = idx as u8;
                best_vector = codebook_entry[0..5].iter().map(|&x| x).collect();
                
                #[cfg(debug_assertions)]
                eprintln!("    NEW BEST: idx={}, dist={}", best_idx, best_dist);
            }
        }
        
        #[cfg(debug_assertions)]
        {
            eprintln!("  Final Stage1: idx={}, dist={}, vector={:?}", 
                best_idx, best_dist, best_vector.iter().map(|x| x.0).collect::<Vec<_>>());
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
    
    /// Weighted first stage vector quantization (full 10D)
    fn vq_stage1_weighted(&self, residual: &[Q15; LP_ORDER]) -> (u8, [Q15; LP_ORDER]) {
        let mut best_idx = 0u8;
        let mut best_dist = f64::MAX;
        let mut best_vector = [Q15::ZERO; LP_ORDER];
        
        // Get adaptive weights
        let weights = self.get_lsp_weights(&(0..LP_ORDER).collect::<Vec<_>>());
        
        #[cfg(debug_assertions)]
        {
            eprintln!("    🔍 VQ Stage1 DEBUG:");
            eprintln!("      Residual: {:?}", residual.iter().take(5).map(|x| x.0).collect::<Vec<_>>());
            eprintln!("      Weights: {:?}", weights.iter().take(5).map(|x| x.0).collect::<Vec<_>>());
            eprintln!("      Searching {} codebook entries...", self.codebooks.stage1_codebook.len());
        }
        
        // Search all 128 codebook entries
        for (idx, codebook_entry) in self.codebooks.stage1_codebook.iter().enumerate() {
            let mut dist = 0.0;
            
            // Weighted Euclidean distance
            for i in 0..LP_ORDER {
                let diff = (residual[i].0 as f64) - (codebook_entry[i].0 as f64);
                dist += (diff * diff) * (weights[i].0 as f64);
            }
            
            #[cfg(debug_assertions)]
            {
                // Log details for first 5 entries, index 71, and any new best
                if idx < 5 || idx == 71 || dist < best_dist {
                    eprintln!("      Entry {}: dist={:.0}, vector: {:?}", 
                        idx, dist, codebook_entry.iter().take(3).map(|x| x.0).collect::<Vec<_>>());
                    
                    if idx < 5 {
                        // Show detailed calculation for first few entries
                        eprintln!("        Detail: diff²*weight = {:?}", 
                            residual.iter().zip(codebook_entry.iter()).zip(weights.iter())
                                .take(3)
                                .map(|((r,c),w)| {
                                    let diff = r.0 as f64 - c.0 as f64;
                                    diff * diff * w.0 as f64
                                })
                                .collect::<Vec<_>>());
                    }
                }
            }
            
            if dist < best_dist {
                best_dist = dist;
                best_idx = idx as u8;
                for i in 0..LP_ORDER {
                    best_vector[i] = codebook_entry[i];
                }
                
                #[cfg(debug_assertions)]
                eprintln!("        ⭐ NEW BEST: idx={}, dist={:.0}", best_idx, best_dist);
            }
        }
        
        #[cfg(debug_assertions)]
        {
            eprintln!("    🎯 Final Stage1: idx={}, dist={:.0}", best_idx, best_dist);
            eprintln!("      Best vector: {:?}", best_vector.iter().take(5).map(|x| x.0).collect::<Vec<_>>());
        }
        
        (best_idx, best_vector)
    }
    
    /// Weighted stage2 vector quantization
    fn vq_stage2_weighted(&self, residual: &[Q15], weight_indices: &[usize]) -> (u8, Vec<Q15>) {
        let mut best_idx = 0u8;
        let mut best_dist = f64::MAX;
        let mut best_vector = vec![Q15::ZERO; residual.len()];
        
        // Get adaptive weights for specified indices
        let weights = self.get_lsp_weights(&(0..LP_ORDER).collect::<Vec<_>>());
        
        // Select appropriate codebook based on weight_indices
        let codebook = if weight_indices[0] < 5 {
            &self.codebooks.stage2_codebook_lower
        } else {
            &self.codebooks.stage2_codebook_upper
        };
        
        // Search all 32 codebook entries
        for (idx, codebook_entry) in codebook.iter().enumerate() {
            let mut dist = 0.0;
            
            // Weighted Euclidean distance
            for (i, &weight_idx) in weight_indices.iter().enumerate().take(residual.len()) {
                if i < codebook_entry.len() && i < residual.len() {
                    let diff = (residual[i].0 as f64) - (codebook_entry[i].0 as f64);
                    let weight = weights[weight_idx.min(LP_ORDER-1)].0 as f64;
                    dist += (diff * diff) * weight;
                }
            }
            
            if dist < best_dist {
                best_dist = dist;
                best_idx = idx as u8;
                best_vector = codebook_entry.clone();
            }
        }
        
        (best_idx, best_vector)
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

    /// Rearrange LSP to enforce stability constraints
    fn rearrange_lsp(&self, lsp: &mut [Q15; LP_ORDER], j: i16) {
        let mut i = 0;
        while i < LP_ORDER - 1 {
            let gap = lsp[i+1].0.saturating_sub(lsp[i].0);
            if gap < j {
                lsp[i] = Q15(lsp[i].0.saturating_add(j));
            }
            i += 1;
        }
    }

    /// Compute weighted MSE for a given LSP and reconstructed LSP
    fn compute_weighted_mse(&self, original: &[Q15; LP_ORDER], reconstructed: &[Q15; LP_ORDER]) -> f64 {
        let weights = self.get_lsp_weights(&(0..LP_ORDER).collect::<Vec<_>>());
        let mut mse = 0.0;
        for i in 0..LP_ORDER {
            let diff = (original[i].0 as f64) - (reconstructed[i].0 as f64);
            mse += (diff * diff) * (weights[i].0 as f64);
        }
        mse
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