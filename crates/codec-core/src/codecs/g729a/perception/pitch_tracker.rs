//! Open-loop pitch detection for G.729A

use crate::codecs::g729a::constants::*;
use crate::codecs::g729a::types::{Q15, Q31};
use crate::codecs::g729a::math::FixedPointOps;
use crate::codecs::g729a::math::dsp_operations::{add32, sub32, mult32_32_q23, g729_inv_sqrt_q0q31, shr32, shl32, mac16_16};
use crate::codecs::g729a::signal::{decimated_correlation, normalized_cross_correlation};
use std::ops::Range;

/// Pitch candidate with delay and correlation score
#[derive(Debug, Clone, Copy)]
pub struct PitchCandidate {
    pub delay: u16,
    pub correlation: Q15,
}

/// Low-pass filter for decimation
pub struct LowPassFilter {
    state: (Q15, Q15),
}

impl LowPassFilter {
    /// Create a new low-pass filter
    pub fn new() -> Self {
        Self {
            state: (Q15::ZERO, Q15::ZERO),
        }
    }
    
    /// Process signal through low-pass filter
    /// G.729A uses: H(z) = 1 / (1 - 0.7z^-1)
    /// which is equivalent to y[n] = x[n] + 0.7*y[n-1]
    pub fn process(&mut self, signal: &[Q15]) -> Vec<Q15> {
        let mut output = Vec::with_capacity(signal.len());
        let a1 = Q15::from_f32(0.7); // Positive for feedback
        
        for &sample in signal {
            // y[n] = x[n] + 0.7 * y[n-1]
            let y = sample.saturating_add(a1.saturating_mul(self.state.0));
            output.push(y);
            self.state.0 = y;
        }
        
        output
    }
}

/// Pitch tracker for open-loop pitch estimation
pub struct PitchTracker {
    decimation_filter: LowPassFilter,
}

impl PitchTracker {
    /// Create a new pitch tracker
    pub fn new() -> Self {
        Self {
            decimation_filter: LowPassFilter::new(),
        }
    }
    
    /// Estimate open-loop pitch from weighted speech (ITU-T Algorithm per spec A3.4)
    /// Input: weighted_speech should be 240 samples [history(120) + current(80) + lookahead(40)]
    /// Returns the best pitch candidate
    pub fn estimate_open_loop_pitch(&mut self, weighted_speech: &[Q15]) -> PitchCandidate {
        // Ensure we have enough signal for pitch analysis
        if weighted_speech.len() < 223 {
            // Return default pitch if not enough signal
            return PitchCandidate { delay: 40, correlation: Q15::ZERO };
        }
        
        // For ITU-T algorithm, we need to extract the appropriate region
        // Current frame starts at sample 120 in the 240-sample buffer
        // We need 143 past samples + 80 current = 223 total
        let signal_start: i32 = 120 - 143; // Start 143 samples before current frame (can be negative)
        let signal_end: i32 = signal_start + 223; // 223 samples total
        
        // Handle boundary case for first frame
        let actual_start = signal_start.max(0) as usize;
        let actual_end = signal_end.min(weighted_speech.len() as i32) as usize;
        
        // Create signal buffer matching ITU-T expectations
        let mut itu_signal = vec![Q15::ZERO; 223];
        let copy_start = actual_start;
        let copy_len = actual_end - actual_start;
        let buffer_offset = if signal_start < 0 { (-signal_start) as usize } else { 0 };
        
        itu_signal[buffer_offset..buffer_offset + copy_len].copy_from_slice(&weighted_speech[copy_start..actual_end]);
        
        // Check if scaling is needed to prevent overflow
        let scaled_signal = self.scale_signal_if_needed(&itu_signal);
        
        // Search in three ranges exactly as ITU-T does
        let (corr_max_1, index_1) = self.get_correlation_max_itu_t(&scaled_signal, 20, 39, 1);
        let (corr_max_2, index_2) = self.get_correlation_max_itu_t(&scaled_signal, 40, 79, 1);
        let (mut corr_max_3, mut index_3) = self.get_correlation_max_itu_t(&scaled_signal, 80, 143, 2);
        
        // ITU-T: For range 3, test ±1 around the even maximum
        if index_3 > 80 {
            let corr_odd_minus = self.get_correlation_itu_t(&scaled_signal, index_3 - 1);
            if corr_odd_minus > corr_max_3 {
                corr_max_3 = corr_odd_minus;
                index_3 = index_3 - 1;
            }
        }
        let corr_odd_plus = self.get_correlation_itu_t(&scaled_signal, index_3 + 1);
        if corr_odd_plus > corr_max_3 {
            corr_max_3 = corr_odd_plus;
            index_3 = index_3 + 1;
        }
        
        // ITU-T: Normalize correlations using autocorrelation at the lag
        let auto_corr_1 = self.get_autocorrelation_at_lag(&scaled_signal, index_1);
        let auto_corr_2 = self.get_autocorrelation_at_lag(&scaled_signal, index_2);
        let auto_corr_3 = self.get_autocorrelation_at_lag(&scaled_signal, index_3);
        
        let norm_corr_1 = self.normalize_correlation_itu_t(corr_max_1, auto_corr_1);
        let mut norm_corr_2 = self.normalize_correlation_itu_t(corr_max_2, auto_corr_2);
        let norm_corr_3 = self.normalize_correlation_itu_t(corr_max_3, auto_corr_3);
        
        // ITU-T: Favor lower delays with multiples check
        let candidates = [
            PitchCandidate { delay: index_1, correlation: Q15((norm_corr_1 >> 8) as i16) },
            PitchCandidate { delay: index_2, correlation: Q15((norm_corr_2 >> 8) as i16) },
            PitchCandidate { delay: index_3, correlation: Q15((norm_corr_3 >> 8) as i16) },
        ];
        
        let best = self.select_best_pitch_itu_t(&candidates, norm_corr_1, norm_corr_2, norm_corr_3);
        
        #[cfg(debug_assertions)]
        {
            eprintln!("ITU-T Pitch candidates: [{}, {}, {}] with corr: [{}, {}, {}]",
                candidates[0].delay, candidates[1].delay, candidates[2].delay,
                candidates[0].correlation.0, candidates[1].correlation.0, candidates[2].correlation.0);
            eprintln!("Selected pitch: {} (corr: {})", best.delay, best.correlation.0);
        }
        
        best
    }
    
    /// ITU-T: Scale signal if autocorrelation would overflow (per reference implementation)
    fn scale_signal_if_needed(&self, signal: &[Q15]) -> Vec<Q15> {
        // Compute autocorrelation on 64 bits
        let mut autocorr_64 = 0i64;
        for i in 0..223 { // Full buffer: -143 to +80
            let sample = signal[i].0 as i64;
            autocorr_64 += sample * sample;
        }
        
        if autocorr_64 > i32::MAX as i64 {
            // Scale signal to prevent overflow
            let overflow_scale = ((31 - self.count_leading_zeros_32((autocorr_64 >> 31) as u32)) >> 1).max(1);
            signal.iter().map(|&s| Q15(s.0 >> overflow_scale)).collect()
        } else {
            signal.to_vec()
        }
    }
    
    /// ITU-T: Count leading zeros (31-bit version for autocorrelation scaling)
    fn count_leading_zeros_32(&self, x: u32) -> i32 {
        if x == 0 { return 31; }
        let mut count = 0;
        let mut val = x;
        while val < 0x40000000 { // Until bit 30 is set
            count += 1;
            val <<= 1;
        }
        count
    }
    
    /// ITU-T: getCorrelationMax - find max correlation in range with step
    fn get_correlation_max_itu_t(&self, signal: &[Q15], range_open: u16, range_close: u16, step: u16) -> (i32, u16) {
        let mut correlation_max = i32::MIN;
        let mut best_index = range_open;
        
        let mut i = range_open;
        while i <= range_close {
            let correlation = self.get_correlation_itu_t(signal, i);
            if correlation > correlation_max {
                best_index = i;
                correlation_max = correlation;
            }
            i += step;
        }
        
        (correlation_max, best_index)
    }
    
    /// ITU-T: getCorrelation - compute correlation per spec A3.4 eq A.4
    /// correlation = ∑(i=0..39) inputSignal[2*i] * inputSignal[2*i-index]
    /// NOTE: Uses decimated samples (i+=2) for ALL computations!
    fn get_correlation_itu_t(&self, signal: &[Q15], index: u16) -> i32 {
        let mut correlation = 0i32;
        
        // ITU-T: Current frame starts at index 143 in our buffer
        // The algorithm correlates current frame with past samples at given lag
        let current_frame_start = 143;
        
        #[cfg(debug_assertions)]
        let mut debug_samples = 0;
        
        // ITU-T: for (i=0; i<L_FRAME; i+=2, j+=2) where j=-index
        // This means i goes: 0, 2, 4, ..., 78 (40 iterations)
        // And j goes: -index, -index+2, -index+4, ..., -index+78
        for i_offset in (0..80).step_by(2) {
            let i = current_frame_start + i_offset; // Current frame sample
            let j = current_frame_start + i_offset - (index as usize); // Past frame sample
            
            if j < signal.len() && i < signal.len() {
                let curr_val = signal[i].0;
                let past_val = signal[j].0;
                correlation = mac16_16(correlation, curr_val, past_val);
                
                #[cfg(debug_assertions)]
                {
                    debug_samples += 1;
                    if debug_samples <= 5 && index == 20 { // Debug first few samples for index 20
                        eprintln!("  Corr debug index={}: i={}, j={}, curr={}, past={}, running_corr={}", 
                            index, i, j, curr_val, past_val, correlation);
                    }
                }
            }
        }
        
        #[cfg(debug_assertions)]
        if index == 20 {
            eprintln!("  Correlation for index {}: {} (using {} samples)", index, correlation, debug_samples);
        }
        
        correlation
    }
    
    /// ITU-T: Compute autocorrelation at lag (for normalization)
    fn get_autocorrelation_at_lag(&self, signal: &[Q15], index: u16) -> i32 {
        // ITU-T: getCorrelation(&(scaledWeightedInputSignal[-index]), 0)
        // This computes autocorrelation of the past signal at the lag
        let past_signal_start = 143 - (index as usize); // Start position for past signal
        self.get_correlation_at_zero_lag(signal, past_signal_start)
    }
    
    /// Compute correlation at zero lag (autocorrelation) starting from given position
    fn get_correlation_at_zero_lag(&self, signal: &[Q15], start_pos: usize) -> i32 {
        let mut correlation = 0i32;
        
        // ITU-T: Same decimated pattern as getCorrelation but with zero lag
        // for (i=0; i<L_FRAME; i+=2) correlation += signal[start_pos + i] * signal[start_pos + i]
        for i_offset in (0..80).step_by(2) {
            let pos = start_pos + i_offset;
            if pos < signal.len() {
                correlation = mac16_16(correlation, signal[pos].0, signal[pos].0);
            }
        }
        
        correlation.max(1) // Avoid division by zero
    }
    
    /// ITU-T: Normalize correlation using inverse square root
    fn normalize_correlation_itu_t(&self, correlation: i32, autocorr: i32) -> i32 {
        // ITU-T: MULT32_32_Q23(correlationMax, g729InvSqrt_Q0Q31(autoCorrelation))
        let inv_sqrt = g729_inv_sqrt_q0q31(autocorr);
        mult32_32_q23(correlation, inv_sqrt)
    }
    
    /// ITU-T: Select best pitch with favor-lower-delays algorithm
    fn select_best_pitch_itu_t(&self, candidates: &[PitchCandidate], mut norm_corr_1: i32, mut norm_corr_2: i32, norm_corr_3: i32) -> PitchCandidate {
        let index_1 = candidates[0].delay;
        let index_2 = candidates[1].delay;
        let index_3 = candidates[2].delay;
        
        // ITU-T: Favor lower delays with multiples check
        let index_multiple = index_2 << 1; // 2 * indexRange2
        if (index_multiple as i32 - index_3 as i32).abs() < 5 {
            norm_corr_2 = add32(norm_corr_2, shr32(norm_corr_3, 2)); // Max2 += Max3*0.25
        }
        if ((index_multiple + index_2) as i32 - index_3 as i32).abs() < 7 { // 3*indexRange2 - indexRange3
            norm_corr_2 = add32(norm_corr_2, shr32(norm_corr_3, 2)); // Max2 += Max3*0.25
        }
        
        let index_multiple = index_1 << 1; // 2 * indexRange1
        if (index_multiple as i32 - index_2 as i32).abs() < 5 {
            // Max1 += Max2*0.2 using MAC16_32_P15 with O2_IN_Q15 (0.2 in Q15)
            norm_corr_1 = add32(norm_corr_1, mult32_32_q23(norm_corr_2, 0.2f32.to_bits() as i32 >> 8));
        }
        if ((index_multiple + index_1) as i32 - index_2 as i32).abs() < 7 { // 3*indexRange1 - indexRange2
            norm_corr_1 = add32(norm_corr_1, mult32_32_q23(norm_corr_2, 0.2f32.to_bits() as i32 >> 8));
        }
        
        // Return index with greatest normalized correlation
        if norm_corr_1 < norm_corr_2 {
            if norm_corr_2 < norm_corr_3 {
                candidates[2]
            } else {
                candidates[1]
            }
        } else if norm_corr_1 < norm_corr_3 {
            candidates[2]
        } else {
            candidates[0]
        }
    }
    
    /// Get pitch search range for closed-loop search
    /// Returns (min, max) delay range centered around open-loop estimate
    pub fn get_pitch_search_range(&self, open_loop_pitch: u16, subframe_idx: usize) -> Range<f32> {
        match subframe_idx {
            0 => {
                // First subframe: ±5 around open-loop pitch
                let min = (open_loop_pitch.saturating_sub(5)).max(PIT_MIN) as f32;
                let max = (open_loop_pitch + 5).min(PIT_MAX) as f32;
                min..max
            }
            1 => {
                // Second subframe: use previous subframe pitch (will be set by encoder)
                // For now, return same as first subframe
                let min = (open_loop_pitch.saturating_sub(5)).max(PIT_MIN) as f32;
                let max = (open_loop_pitch + 5).min(PIT_MAX) as f32;
                min..max
            }
            _ => {
                // Invalid subframe, return full range
                PIT_MIN as f32..PIT_MAX as f32
            }
        }
    }
}

impl Default for PitchTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pitch_tracker_creation() {
        let tracker = PitchTracker::new();
        // Just ensure it creates without panic
        assert!(true);
    }

    #[test]
    fn test_low_pass_filter() {
        let mut filter = LowPassFilter::new();
        
        // Test with impulse
        let signal = vec![Q15::ONE, Q15::ZERO, Q15::ZERO, Q15::ZERO];
        let output = filter.process(&signal);
        
        // First output should be 1
        assert_eq!(output[0], Q15::ONE);
        
        // Should have decaying response
        assert!(output[1].0 > 0);
        assert!(output[2].0 > 0);
        assert!(output[2].0 < output[1].0);
    }

    #[test]
    fn test_pitch_candidate_creation() {
        let candidate = PitchCandidate {
            delay: 50,
            correlation: Q15::from_f32(0.8),
        };
        
        assert_eq!(candidate.delay, 50);
        assert!((candidate.correlation.to_f32() - 0.8).abs() < 0.01);
    }

    #[test]
    fn test_pitch_search_range() {
        let tracker = PitchTracker::new();
        
        // Test first subframe
        let range = tracker.get_pitch_search_range(50, 0);
        assert_eq!(range.start, 45.0);
        assert_eq!(range.end, 55.0);
        
        // Test with boundary conditions
        let range = tracker.get_pitch_search_range(PIT_MIN + 2, 0);
        assert_eq!(range.start, PIT_MIN as f32);
        
        let range = tracker.get_pitch_search_range(PIT_MAX - 2, 0);
        assert_eq!(range.end, PIT_MAX as f32);
    }
} 