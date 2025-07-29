use crate::common::basic_operators::*;
use crate::common::tab_ld8a::{M, LSPCB1, LSPCB2, NC0, NC1, MA_NP};

/// LSP parameter decoder
/// Based on LSPDEC.C from G.729A reference implementation
pub struct LspDecoder {
    freq_prev: [[Word16; M]; MA_NP],  // Previous quantized LSP frequencies Q13
}

impl LspDecoder {
    pub fn new() -> Self {
        let mut freq_prev = [[0; M]; MA_NP];
        let freq_prev_reset = [
            2339, 4679, 7018, 9358, 11698, 14037, 16377, 18717, 21056, 23396,
        ];
        
        // Initialize all MA stages with default frequencies
        for i in 0..MA_NP {
            freq_prev[i].copy_from_slice(&freq_prev_reset);
        }
        
        Self { freq_prev }
    }
    
    /// Decode LSP parameters from quantization indices
    /// Based on D_lsp function in LSPDEC.C
    pub fn decode_lsp(&mut self, indices: &[Word16], lsp_q: &mut [Word16]) {
        assert_eq!(indices.len(), 2, "LSP indices must be length 2");
        assert_eq!(lsp_q.len(), M, "LSP output must be length {}", M);
        
        let index1 = indices[0]; // First stage index (10 bits)
        let index2 = indices[1]; // Second stage index (8 bits)
        
        // Ensure indices are within valid range
        let index1 = if index1 >= NC0 as Word16 { 0 } else { index1 };
        let index2 = if index2 >= NC1 as Word16 { 0 } else { index2 };
        
        // Step 1: First stage vector quantization
        let mut lsf_q = [0i16; M];
        for i in 0..M {
            lsf_q[i] = LSPCB1[index1 as usize][i];
        }
        
        // Step 2: Second stage vector quantization (residual)
        for i in 0..M {
            lsf_q[i] = add(lsf_q[i], LSPCB2[index2 as usize][i]);
        }
        
        // Step 3: Add moving average prediction
        for i in 0..M {
            let mut sum = 0i32;
            
            // MA prediction using previous quantized frequencies
            // Prediction coefficients are built into the process
            for j in 0..MA_NP {
                if j == 0 {
                    sum = l_mac(sum, self.freq_prev[j][i], 20861); // 0.636 in Q15
                } else if j == 1 {
                    sum = l_mac(sum, self.freq_prev[j][i], 12509); // 0.381 in Q15  
                } else if j == 2 {
                    sum = l_mac(sum, self.freq_prev[j][i], 7300);  // 0.223 in Q15
                } else {
                    sum = l_mac(sum, self.freq_prev[j][i], 3932);  // 0.120 in Q15
                }
            }
            
            // Add prediction to quantized LSF
            lsf_q[i] = add(lsf_q[i], round(l_shl(sum, 1))); // Q13
        }
        
        // Step 4: Ensure LSF ordering and stability
        self.reorder_lsf(&mut lsf_q);
        
        // Step 5: Convert LSF to LSP
        self.lsf_to_lsp(&lsf_q, lsp_q);
        
        // Step 6: Update MA predictor memory
        self.update_freq_prev(&lsf_q);
    }
    
    /// Ensure LSF parameters are properly ordered and stable
    /// Based on Reorder_lsf function in LSPDEC.C
    fn reorder_lsf(&self, lsf: &mut [Word16]) {
        const GAP: Word16 = 205; // Minimum gap between LSFs (50 Hz in Q13)
        
        // Ensure minimum spacing between consecutive LSFs
        for i in 1..M {
            let diff = sub(lsf[i], lsf[i-1]);
            if diff < GAP {
                let correction = shr(sub(GAP, diff), 1);
                lsf[i-1] = sub(lsf[i-1], correction);
                lsf[i] = add(lsf[i], correction);
            }
        }
        
        // Ensure LSFs are within valid frequency range
        const MIN_LSF: Word16 = 205;   // 50 Hz in Q13
        const MAX_LSF: Word16 = 25681; // 3900 Hz in Q13
        
        if lsf[0] < MIN_LSF {
            lsf[0] = MIN_LSF;
        }
        
        if lsf[M-1] > MAX_LSF {
            lsf[M-1] = MAX_LSF;
        }
        
        // Final pass to ensure ordering
        for i in 1..M {
            if lsf[i] <= lsf[i-1] {
                lsf[i] = add(lsf[i-1], GAP);
            }
        }
    }
    
    /// Convert Line Spectral Frequencies to Line Spectral Pairs
    /// Based on Lsf_lsp function
    fn lsf_to_lsp(&self, lsf: &[Word16], lsp: &mut [Word16]) {
        // For G.729A, LSF and LSP are essentially the same in the frequency domain
        // The conversion is: lsp[i] = cos(2*pi*lsf[i]/fs) where fs = 8000 Hz
        // But in practice, for the decoder, we can use a direct mapping
        
        for i in 0..M {
            // Convert from LSF (Q13, normalized frequency) to LSP (Q15, cosine domain)
            // This is a simplified conversion - the exact conversion uses cosine tables
            let normalized_freq = shl(lsf[i], 2); // Q13 -> Q15
            
            // Approximate cosine mapping (simplified for now)
            // In a full implementation, this would use a cosine lookup table
            lsp[i] = sub(32767, shr(normalized_freq, 1)); // Approximate mapping
        }
    }
    
    /// Update moving average predictor memory
    fn update_freq_prev(&mut self, lsf_q: &[Word16]) {
        // Shift previous frequencies
        for i in (1..MA_NP).rev() {
            self.freq_prev[i] = self.freq_prev[i-1];
        }
        
        // Store current quantized LSF
        self.freq_prev[0].copy_from_slice(lsf_q);
    }
}