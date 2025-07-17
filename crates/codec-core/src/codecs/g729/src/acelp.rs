//! G.729 ACELP (Algebraic Code Excited Linear Prediction) Module
//!
//! This module implements ACELP analysis for the G.729 codec, including:
//! - Fixed codebook search (algebraic codebook)
//! - Adaptive codebook construction
//! - Gain quantization and prediction
//! - Innovation sequence generation
//! - Correlation functions for optimization
//!
//! Based on ITU-T G.729 reference implementation ACELP_CO.C (869 lines)

use super::types::*;
use super::math::*;
use super::dsp::*;

/// Number of tracks in the algebraic codebook
const NB_TRACK: usize = 4;

/// Number of positions per track  
const STEP: usize = 5;

/// Length of algebraic codeword
const L_CODE: usize = 40;

/// Number of bits for fixed codebook
const NB_BITS: usize = 17;

/// Maximum number of iterations for search
const MAX_ITER: usize = 4;

/// ACELP fixed codebook analyzer
#[derive(Debug, Clone)]
pub struct AcelpAnalyzer {
    /// Impulse response of weighted synthesis filter
    pub h: [Word16; L_SUBFR],
    /// Correlation matrix H^T * H
    pub h_h: [[Word16; L_CODE]; L_CODE],
    /// Target signal correlation
    pub target_corr: [Word32; L_CODE],
    /// Previous innovation gain
    pub prev_gain: Word16,
    /// Gain predictor state
    pub gain_pred: [Word16; 4],
}

impl AcelpAnalyzer {
    /// Create a new ACELP analyzer
    pub fn new() -> Self {
        Self {
            h: [0; L_SUBFR],
            h_h: [[0; L_CODE]; L_CODE],
            target_corr: [0; L_CODE],
            prev_gain: 1024,  // Q10 format, represents ~1.0
            gain_pred: [0; 4],
        }
    }

    /// Reset the ACELP analyzer state
    pub fn reset(&mut self) {
        self.h = [0; L_SUBFR];
        self.h_h = [[0; L_CODE]; L_CODE];
        self.target_corr = [0; L_CODE];
        self.prev_gain = 1024;
        self.gain_pred = [0; 4];
    }

    /// Set impulse response of weighted synthesis filter
    /// 
    /// This function sets the impulse response that will be used
    /// for correlation computations in the codebook search.
    /// 
    /// # Arguments
    /// * `impulse_response` - Impulse response of the filter [L_SUBFR]
    pub fn set_impulse_response(&mut self, impulse_response: &[Word16]) {
        assert_eq!(impulse_response.len(), L_SUBFR);
        self.h.copy_from_slice(impulse_response);
        
        // Precompute correlation matrix H^T * H for efficiency
        self.compute_correlation_matrix();
    }

    /// Compute correlation matrix H^T * H
    /// 
    /// This precomputes the correlation matrix to speed up the
    /// algebraic codebook search. The matrix is symmetric.
    fn compute_correlation_matrix(&mut self) {
        // Clear the matrix
        for i in 0..L_CODE {
            for j in 0..L_CODE {
                self.h_h[i][j] = 0;
            }
        }

        // Compute upper triangular part
        for i in 0..L_CODE {
            for j in i..L_CODE {
                let mut sum = 0i32;
                
                for k in 0..(L_SUBFR - j.max(i)) {
                    if i + k < L_SUBFR && j + k < L_SUBFR {
                        sum = l_add(sum, l_mult(self.h[i + k], self.h[j + k]));
                    }
                }
                
                self.h_h[i][j] = round_word32(sum);
                // Matrix is symmetric
                if i != j {
                    self.h_h[j][i] = self.h_h[i][j];
                }
            }
        }
    }

    /// Algebraic codebook search - main entry point
    /// 
    /// This function performs the algebraic codebook search to find
    /// the best fixed codebook contribution for the current subframe.
    /// 
    /// # Arguments
    /// * `target` - Target signal for the search [L_SUBFR]
    /// * `res2` - Residual signal after adaptive codebook [L_SUBFR]
    /// * `code` - Output: selected codeword [L_SUBFR]
    /// * `y` - Output: filtered codeword [L_SUBFR]
    /// 
    /// # Returns
    /// (position_indices, signs, gain_index)
    pub fn acelp_codebook_search(
        &mut self,
        target: &[Word16],
        res2: &[Word16],
        code: &mut [Word16],
        y: &mut [Word16],
    ) -> ([usize; 4], [i8; 4], usize) {
        assert_eq!(target.len(), L_SUBFR);
        assert_eq!(res2.len(), L_SUBFR);
        assert_eq!(code.len(), L_SUBFR);
        assert_eq!(y.len(), L_SUBFR);

        // Compute correlation between target and impulse response
        self.compute_target_correlation(target);

        // Search for best 4-pulse combination
        let (positions, signs) = self.d4i40_17();

        // Build the codeword from positions and signs
        code.fill(0);
        for i in 0..4 {
            if positions[i] < L_SUBFR {
                code[positions[i]] = if signs[i] > 0 { 4096 } else { -4096 }; // Q12 format
            }
        }

        // Filter the codeword through impulse response
        self.filter_codeword(code, y);

        // Compute gain index (simplified for now)
        let gain_index = self.compute_gain_index(target, y);

        (positions, signs, gain_index)
    }

    /// Compute correlation between target and impulse response
    /// 
    /// This computes the correlation d[n] = sum(target[k] * h[n-k])
    /// which is used to guide the codebook search.
    fn compute_target_correlation(&mut self, target: &[Word16]) {
        for i in 0..L_CODE.min(L_SUBFR) {
            let mut sum = 0i32;
            
            for k in 0..(L_SUBFR - i) {
                if k < target.len() && i + k < self.h.len() {
                    sum = l_add(sum, l_mult(target[k], self.h[i + k]));
                }
            }
            
            self.target_corr[i] = sum;
        }
        
        // Fill remaining positions with zeros
        for i in L_SUBFR..L_CODE {
            self.target_corr[i] = 0;
        }
    }

    /// D4i40_17: 4-pulse algebraic codebook search with 17 bits
    /// 
    /// This is the core of the ACELP search algorithm. It finds the
    /// optimal positions and signs for 4 pulses in 40 positions.
    /// 
    /// Track 0: positions 0, 5, 10, 15, 20, 25, 30, 35
    /// Track 1: positions 1, 6, 11, 16, 21, 26, 31, 36
    /// Track 2: positions 2, 7, 12, 17, 22, 27, 32, 37
    /// Track 3: positions 3, 8, 13, 18, 23, 28, 33, 38
    /// 
    /// # Returns
    /// (positions[4], signs[4]) - Best 4-pulse configuration
    fn d4i40_17(&self) -> ([usize; 4], [i8; 4]) {
        let mut best_positions = [0; 4];
        let mut best_signs = [1i8; 4];
        let mut max_correlation = 0i32;

        // Search all 4 tracks
        for track in 0..NB_TRACK {
            let (pos, sign, corr) = self.search_track(track);
            
            if track == 0 || corr > max_correlation {
                max_correlation = corr;
                best_positions[track] = pos;
                best_signs[track] = sign;
            }
        }

        // Refine search with multiple iterations
        for _iter in 0..MAX_ITER {
            let mut improved = false;
            
            for track in 0..NB_TRACK {
                let (new_pos, new_sign, new_corr) = self.search_track_refined(
                    track, &best_positions, &best_signs
                );
                
                if new_corr > max_correlation {
                    max_correlation = new_corr;
                    best_positions[track] = new_pos;
                    best_signs[track] = new_sign;
                    improved = true;
                }
            }
            
            if !improved {
                break;
            }
        }

        (best_positions, best_signs)
    }

    /// Search a single track for the best pulse position
    fn search_track(&self, track: usize) -> (usize, i8, Word32) {
        let mut best_pos = track;
        let mut best_sign = 1i8;
        let mut max_corr = 0i32;

        // Search all positions in this track
        for pos in (track..L_SUBFR).step_by(STEP) {
            if pos < self.target_corr.len() {
                let corr = self.target_corr[pos].abs();
                
                if corr > max_corr {
                    max_corr = corr;
                    best_pos = pos;
                    best_sign = if self.target_corr[pos] >= 0 { 1 } else { -1 };
                }
            }
        }

        (best_pos, best_sign, max_corr)
    }

    /// Refined search considering interaction with other pulses
    fn search_track_refined(
        &self,
        track: usize,
        positions: &[usize; 4],
        signs: &[i8; 4],
    ) -> (usize, i8, Word32) {
        let mut best_pos = positions[track];
        let mut best_sign = signs[track];
        let mut max_metric = self.compute_search_metric(positions, signs);

        // Try all positions in this track
        for pos in (track..L_SUBFR).step_by(STEP) {
            for &sign in &[1i8, -1i8] {
                let mut test_positions = *positions;
                let mut test_signs = *signs;
                test_positions[track] = pos;
                test_signs[track] = sign;

                let metric = self.compute_search_metric(&test_positions, &test_signs);
                
                if metric > max_metric {
                    max_metric = metric;
                    best_pos = pos;
                    best_sign = sign;
                }
            }
        }

        (best_pos, best_sign, max_metric)
    }

    /// Compute search metric for a 4-pulse configuration
    /// 
    /// This computes the correlation metric used to evaluate
    /// different pulse configurations during the search.
    fn compute_search_metric(&self, positions: &[usize; 4], signs: &[i8; 4]) -> Word32 {
        let mut correlation = 0i32;
        let mut energy = 0i32;

        // Compute correlation sum(d[i] * sign[i])
        for i in 0..4 {
            let pos = positions[i];
            if pos < self.target_corr.len() {
                let contrib = mult(self.target_corr[pos] as Word16, signs[i] as Word16);
                correlation = l_add(correlation, contrib as Word32);
            }
        }

        // Compute energy sum(h[i,j] * sign[i] * sign[j])
        for i in 0..4 {
            for j in 0..4 {
                let pos_i = positions[i];
                let pos_j = positions[j];
                
                if pos_i < L_CODE && pos_j < L_CODE {
                    let h_val = self.h_h[pos_i][pos_j];
                    let contrib = mult(h_val, (signs[i] * signs[j]) as Word16);
                    energy = l_add(energy, contrib as Word32);
                }
            }
        }

        // Return correlation^2 / energy (simplified metric)
        if energy > 0 {
            let corr_sq = l_mult(correlation as Word16, correlation as Word16);
            corr_sq / energy.max(1)
        } else {
            0
        }
    }

    /// Filter codeword through impulse response
    /// 
    /// This computes the filtered codeword y[n] = sum(code[k] * h[n-k])
    /// which represents the contribution of the fixed codebook to the
    /// synthesis filter output.
    fn filter_codeword(&self, code: &[Word16], y: &mut [Word16]) {
        y.fill(0);
        
        for n in 0..L_SUBFR {
            let mut sum = 0i32;
            
            for k in 0..=n {
                if k < code.len() && n - k < self.h.len() {
                    sum = l_add(sum, l_mult(code[k], self.h[n - k]));
                }
            }
            
            y[n] = round_word32(sum);
        }
    }

    /// Compute gain index for quantization
    /// 
    /// This computes the optimal gain for the fixed codebook contribution
    /// and returns the quantization index.
    fn compute_gain_index(&mut self, target: &[Word16], filtered_code: &[Word16]) -> usize {
        // Compute correlation between target and filtered code
        let mut num = 0i32;
        let mut den = 0i32;
        
        for i in 0..L_SUBFR {
            num = l_add(num, l_mult(target[i], filtered_code[i]));
            den = l_add(den, l_mult(filtered_code[i], filtered_code[i]));
        }
        
        // Compute optimal gain
        let gain = if den > 0 {
            (num / den.max(1)).max(0).min(32767) as Word16
        } else {
            0
        };
        
        // Update gain predictor state
        self.prev_gain = gain;
        
        // Simplified gain quantization (normally uses MA prediction)
        // Map gain to 7-bit index (0-127)
        ((gain >> 8) as usize).min(127)
    }

    /// Build innovation sequence from codebook parameters
    /// 
    /// This function reconstructs the innovation sequence from the
    /// quantized codebook parameters (positions, signs, gain).
    /// 
    /// # Arguments
    /// * `positions` - Pulse positions [4]
    /// * `signs` - Pulse signs [4]
    /// * `gain_index` - Quantized gain index
    /// * `innovation` - Output innovation sequence [L_SUBFR]
    pub fn build_innovation(
        &self,
        positions: &[usize; 4],
        signs: &[i8; 4],
        gain_index: usize,
        innovation: &mut [Word16],
    ) {
        innovation.fill(0);
        
        // Reconstruct gain from index (simplified)
        let gain = ((gain_index << 8) as Word16).min(32767);
        
        // Place pulses at specified positions with signs and gain
        for i in 0..4 {
            let pos = positions[i];
            if pos < innovation.len() {
                let amplitude = mult(gain, if signs[i] > 0 { 4096 } else { -4096 });
                innovation[pos] = amplitude;
            }
        }
    }

    /// Adaptive codebook filtering
    /// 
    /// This function filters the adaptive codebook contribution
    /// through the weighted synthesis filter.
    /// 
    /// # Arguments
    /// * `adaptive_exc` - Adaptive codebook excitation [L_SUBFR]
    /// * `filtered_adaptive` - Output filtered signal [L_SUBFR]
    pub fn filter_adaptive_codebook(
        &self,
        adaptive_exc: &[Word16],
        filtered_adaptive: &mut [Word16],
    ) {
        // This is similar to filter_codeword but for adaptive codebook
        filtered_adaptive.fill(0);
        
        for n in 0..L_SUBFR {
            let mut sum = 0i32;
            
            for k in 0..=n {
                if k < adaptive_exc.len() && n - k < self.h.len() {
                    sum = l_add(sum, l_mult(adaptive_exc[k], self.h[n - k]));
                }
            }
            
            filtered_adaptive[n] = round_word32(sum);
        }
    }
}

/// Correlation utilities for ACELP
pub mod correlation {
    use super::*;

    /// Compute correlation between two signals
    /// 
    /// # Arguments
    /// * `x` - First signal
    /// * `y` - Second signal
    /// * `length` - Length to correlate
    /// 
    /// # Returns
    /// Correlation value
    pub fn cor_h_x(x: &[Word16], y: &[Word16], length: usize) -> Word32 {
        let mut sum = 0i32;
        
        for i in 0..length.min(x.len().min(y.len())) {
            sum = l_add(sum, l_mult(x[i], y[i]));
        }
        
        sum
    }

    /// Compute auto-correlation of a signal
    /// 
    /// # Arguments
    /// * `x` - Input signal
    /// * `length` - Length to correlate
    /// 
    /// # Returns
    /// Auto-correlation value
    pub fn auto_correlation(x: &[Word16], length: usize) -> Word32 {
        cor_h_x(x, x, length)
    }

    /// Compute normalized correlation
    /// 
    /// # Arguments
    /// * `x` - First signal
    /// * `y` - Second signal
    /// * `length` - Length to correlate
    /// 
    /// # Returns
    /// Normalized correlation (-1.0 to 1.0 in Q15)
    pub fn normalized_correlation(x: &[Word16], y: &[Word16], length: usize) -> Word16 {
        let xy = cor_h_x(x, y, length);
        let xx = auto_correlation(x, length);
        let yy = auto_correlation(y, length);
        
        if xx > 0 && yy > 0 {
            // Simplified normalization
            let denom = (xx.max(yy) >> 16).max(1);
            (xy / denom).max(-32768).min(32767) as Word16
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_acelp_analyzer_creation() {
        let analyzer = AcelpAnalyzer::new();
        assert_eq!(analyzer.prev_gain, 1024);
        assert_eq!(analyzer.h[0], 0);
    }

    #[test]
    fn test_acelp_analyzer_reset() {
        let mut analyzer = AcelpAnalyzer::new();
        analyzer.prev_gain = 2048;
        analyzer.h[0] = 100;
        
        analyzer.reset();
        
        assert_eq!(analyzer.prev_gain, 1024);
        assert_eq!(analyzer.h[0], 0);
    }

    #[test]
    fn test_impulse_response_setting() {
        let mut analyzer = AcelpAnalyzer::new();
        let impulse = vec![1000i16; L_SUBFR];
        
        analyzer.set_impulse_response(&impulse);
        
        assert_eq!(analyzer.h[0], 1000);
        assert_eq!(analyzer.h[L_SUBFR - 1], 1000);
    }

    #[test]
    fn test_target_correlation_computation() {
        let mut analyzer = AcelpAnalyzer::new();
        let impulse = vec![1000i16; L_SUBFR];
        let target = vec![500i16; L_SUBFR];
        
        analyzer.set_impulse_response(&impulse);
        analyzer.compute_target_correlation(&target);
        
        // First correlation should be non-zero
        assert!(analyzer.target_corr[0] != 0);
    }

    #[test]
    fn test_codebook_search() {
        let mut analyzer = AcelpAnalyzer::new();
        let impulse = vec![1000i16; L_SUBFR];
        let target = vec![500i16; L_SUBFR];
        let res2 = vec![100i16; L_SUBFR];
        let mut code = vec![0i16; L_SUBFR];
        let mut y = vec![0i16; L_SUBFR];
        
        analyzer.set_impulse_response(&impulse);
        
        let (positions, signs, gain_idx) = analyzer.acelp_codebook_search(
            &target, &res2, &mut code, &mut y
        );
        
        // Should find 4 pulse positions
        assert_eq!(positions.len(), 4);
        assert_eq!(signs.len(), 4);
        assert!(gain_idx <= 127);
        
        // Positions should be valid
        for &pos in &positions {
            assert!(pos < L_SUBFR);
        }
        
        // Signs should be ±1
        for &sign in &signs {
            assert!(sign == 1 || sign == -1);
        }
    }

    #[test]
    fn test_track_search() {
        let mut analyzer = AcelpAnalyzer::new();
        analyzer.target_corr[0] = 1000;
        analyzer.target_corr[5] = 2000;
        analyzer.target_corr[10] = 1500;
        
        let (pos, sign, corr) = analyzer.search_track(0);
        
        // Should find position 5 (highest correlation in track 0)
        assert_eq!(pos, 5);
        assert_eq!(sign, 1);
        assert_eq!(corr, 2000);
    }

    #[test]
    fn test_innovation_building() {
        let analyzer = AcelpAnalyzer::new();
        let positions = [0, 11, 22, 33];
        let signs = [1, -1, 1, -1];
        let gain_index = 64;
        let mut innovation = vec![0i16; L_SUBFR];
        
        analyzer.build_innovation(&positions, &signs, gain_index, &mut innovation);
        
        // Check pulse positions
        assert!(innovation[0] != 0);    // Position 0, positive
        assert!(innovation[11] != 0);   // Position 11, negative
        assert!(innovation[22] != 0);   // Position 22, positive
        assert!(innovation[33] != 0);   // Position 33, negative
        
        // Check signs
        assert!(innovation[0] > 0);
        assert!(innovation[11] < 0);
        assert!(innovation[22] > 0);
        assert!(innovation[33] < 0);
    }

    #[test]
    fn test_correlation_utilities() {
        let x = vec![1000i16; 10];
        let y = vec![800i16; 10];
        
        let corr = correlation::cor_h_x(&x, &y, 10);
        assert!(corr > 0);
        
        let auto_corr = correlation::auto_correlation(&x, 10);
        assert!(auto_corr > corr);
        
        let norm_corr = correlation::normalized_correlation(&x, &y, 10);
        assert!(norm_corr > 0);
        assert!(norm_corr <= 32767);
    }

    #[test]
    fn test_filter_codeword() {
        let mut analyzer = AcelpAnalyzer::new();
        let impulse = vec![1000i16; L_SUBFR];
        let mut code = vec![0i16; L_SUBFR];
        let mut y = vec![0i16; L_SUBFR];
        
        // Set impulse response
        analyzer.set_impulse_response(&impulse);
        
        // Create simple codeword
        code[0] = 4096;
        code[10] = -2048;
        
        analyzer.filter_codeword(&code, &mut y);
        
        // Output should be non-zero
        assert!(y[0] != 0);
        assert!(y[10] != 0);
    }

    #[test]
    fn test_gain_computation() {
        let mut analyzer = AcelpAnalyzer::new();
        let target = vec![1000i16; L_SUBFR];
        let filtered_code = vec![800i16; L_SUBFR];
        
        let gain_index = analyzer.compute_gain_index(&target, &filtered_code);
        
        assert!(gain_index <= 127);
        assert!(analyzer.prev_gain > 0);
    }
} 