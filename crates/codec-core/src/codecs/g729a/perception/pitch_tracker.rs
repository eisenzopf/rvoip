//! Open-loop pitch detection for G.729A

use crate::codecs::g729a::constants::*;
use crate::codecs::g729a::types::{Q15, Q31};
use crate::codecs::g729a::math::{FixedPointOps, energy, iir_filter_1st_order};
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
    
    /// Estimate open-loop pitch from weighted speech
    /// Returns the best pitch candidate
    pub fn estimate_open_loop_pitch(&mut self, weighted_speech: &[Q15]) -> PitchCandidate {
        // G.729A uses specific window for pitch detection
        // Only use 80 samples (2 subframes) for correlation
        let pitch_window = &weighted_speech[..80];
        
        // 1. Apply low-pass filter for decimation
        let filtered = self.decimation_filter.process(pitch_window);
        
        // 2. Search in three delay ranges per ITU-T G.729A
        let mut candidates = vec![];
        
        // Range 1: 20-39 (no decimation)
        let best1 = self.search_range_729a(&filtered, 20, 39, 1);
        candidates.push(best1);
        
        // Range 2: 40-79 (no decimation) 
        let best2 = self.search_range_729a(&filtered, 40, 79, 1);
        candidates.push(best2);
        
        // Range 3: 80-143 (decimation by 2)
        let best3 = self.search_range_729a(&filtered, 80, 143, 2);
        candidates.push(best3);
        
        #[cfg(debug_assertions)]
        {
            eprintln!("Pitch candidates: [{}, {}, {}] with corr: [{}, {}, {}]",
                candidates[0].delay, candidates[1].delay, candidates[2].delay,
                candidates[0].correlation.0, candidates[1].correlation.0, candidates[2].correlation.0);
        }
        
        // 3. Select best with pitch doubling/halving checks
        let best = self.select_best_pitch_729a(&candidates);
        
        #[cfg(debug_assertions)]
        {
            eprintln!("Selected pitch: {} (corr: {})", best.delay, best.correlation.0);
        }
        
        best
    }
    
    /// Search for best pitch in a given range (ITU-T G.729A algorithm)
    fn search_range_729a(&self, signal: &[Q15], min: u16, max: u16, decimation: usize) -> PitchCandidate {
        let mut best_delay = min;
        let mut best_corr = Q15(i16::MIN);
        
        // For range 3, only test even delays first
        let step = if min >= 80 { 2 } else { 1 };
        
        for delay in (min..=max).step_by(step) {
            // Compute correlation R(k) = sum(sw[n] * sw[n-k])
            let corr = self.compute_normalized_correlation(signal, delay as usize, decimation);
            
            if corr.0 > best_corr.0 {
                best_corr = corr;
                best_delay = delay;
            }
        }
        
        // For range 3, refine around best delay
        if min >= 80 && best_delay > min && best_delay < max {
            // Test ±1 around best
            for offset in [-1i16, 1] {
                let test_delay = (best_delay as i16 + offset) as u16;
                if test_delay >= min && test_delay <= max {
                    let corr = self.compute_normalized_correlation(signal, test_delay as usize, decimation);
                    if corr.0 > best_corr.0 {
                        best_corr = corr;
                        best_delay = test_delay;
                    }
                }
            }
        }
        
        PitchCandidate {
            delay: best_delay,
            correlation: best_corr,
        }
    }
    
    /// Compute normalized correlation per ITU-T G.729A
    fn compute_normalized_correlation(&self, signal: &[Q15], delay: usize, decimation: usize) -> Q15 {
        // Use only 80 samples for correlation
        let len = 80usize.min(signal.len()).saturating_sub(delay);
        if len == 0 {
            return Q15::ZERO;
        }
        
        // Compute R(k) using decimation
        let r_k = if decimation == 2 {
            // Use every other sample for decimation by 2
            let mut sum = Q31::ZERO;
            for i in (0..len).step_by(2) {
                let prod = signal[i].to_q31().saturating_mul(signal[i + delay].to_q31());
                sum = sum.saturating_add(prod);
            }
            sum
        } else {
            // No decimation
            let mut sum = Q31::ZERO;
            for i in 0..len {
                let prod = signal[i].to_q31().saturating_mul(signal[i + delay].to_q31());
                sum = sum.saturating_add(prod);
            }
            sum
        };
        
        // Compute energy E(k) = sum(sw[n-k]^2)
        let e_k = if decimation == 2 {
            let mut sum = Q31::ZERO;
            for i in (0..len).step_by(2) {
                let val = signal[i + delay];
                let prod = val.to_q31().saturating_mul(val.to_q31());
                sum = sum.saturating_add(prod);
            }
            sum
        } else {
            let mut sum = Q31::ZERO;
            for i in 0..len {
                let val = signal[i + delay];
                let prod = val.to_q31().saturating_mul(val.to_q31());
                sum = sum.saturating_add(prod);
            }
            sum
        };
        
        // Normalize: R'(k) = R(k) / sqrt(E(k))
        if e_k.0 <= 0 {
            return Q15::ZERO;
        }
        
        // Approximate normalization
        // Use shifts to approximate division by sqrt
        let e_k_shifted = (e_k.0 >> 15).max(1);
        let normalized = (r_k.0 / e_k_shifted) as i16;
        Q15(normalized)
    }
    
    /// Select best pitch candidate with pitch doubling/halving checks (ITU-T G.729A)
    fn select_best_pitch_729a(&self, candidates: &[PitchCandidate]) -> PitchCandidate {
        let mut r_prime = [Q15::ZERO; 3];
        for i in 0..3 {
            r_prime[i] = candidates[i].correlation;
        }
        
        // Favor lower delays by weighting
        // Check for multiples between ranges
        
        // Check if range2 is multiple of range3
        let t2 = candidates[1].delay;
        let t3 = candidates[2].delay;
        
        if (t3 as i32 - 2 * t2 as i32).abs() <= 5 {
            r_prime[1] = Q15(r_prime[1].0.saturating_add((r_prime[2].0 >> 2))); // Add 0.25 * R'(t3)
        } else if (t3 as i32 - 3 * t2 as i32).abs() <= 7 {
            r_prime[1] = Q15(r_prime[1].0.saturating_add((r_prime[2].0 >> 2))); // Add 0.25 * R'(t3)
        }
        
        // Check if range1 is multiple of range2
        let t1 = candidates[0].delay;
        
        if (t2 as i32 - 2 * t1 as i32).abs() <= 5 {
            r_prime[0] = Q15(r_prime[0].0.saturating_add((r_prime[1].0 / 5))); // Add 0.2 * R'(t2)
        } else if (t2 as i32 - 3 * t1 as i32).abs() <= 7 {
            r_prime[0] = Q15(r_prime[0].0.saturating_add((r_prime[1].0 / 5))); // Add 0.2 * R'(t2)
        }
        
        // Find maximum
        let mut best_idx = 0;
        let mut best_corr = r_prime[0];
        
        for i in 1..3 {
            if r_prime[i].0 > best_corr.0 {
                best_corr = r_prime[i];
                best_idx = i;
            }
        }
        
        candidates[best_idx]
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
    fn test_search_range() {
        let tracker = PitchTracker::new();
        
        // Create a periodic signal
        let mut signal = vec![Q15::ZERO; 200];
        let period = 40;
        for i in 0..200 {
            if i % period == 0 {
                signal[i] = Q15::from_f32(0.8);
            }
        }
        
        // Search in medium range
        let candidate = tracker.search_range_729a(&signal, 35, 45, 1);
        
        // Should find delay close to 40
        assert!((candidate.delay as i32 - 40).abs() <= 2);
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

    #[test]
    fn test_select_best_pitch() {
        let tracker = PitchTracker::new();
        
        let candidates = [
            PitchCandidate { delay: 40, correlation: Q15::from_f32(0.7) },
            PitchCandidate { delay: 80, correlation: Q15::from_f32(0.75) }, // Double of first
            PitchCandidate { delay: 120, correlation: Q15::from_f32(0.6) },
        ];
        
        let best = tracker.select_best_pitch_729a(&candidates);
        
        // Should prefer the second one due to doubling relationship and good correlation
        // But first one might win due to short delay bonus
        assert!(best.delay == 40 || best.delay == 80);
    }

    #[test]
    fn test_open_loop_pitch_estimation() {
        let mut tracker = PitchTracker::new();
        
        // Create a signal with clear pitch
        let mut signal = vec![Q15::ZERO; FRAME_SIZE + LOOK_AHEAD];
        let pitch_period = 60;
        
        for i in 0..signal.len() {
            let phase = (i % pitch_period) as f32 / pitch_period as f32;
            signal[i] = Q15::from_f32((2.0 * std::f32::consts::PI * phase).sin() * 0.5);
        }
        
        let pitch = tracker.estimate_open_loop_pitch(&signal);
        
        // Should find pitch close to 60
        assert!((pitch.delay as i32 - pitch_period as i32).abs() <= 5);
        assert!(pitch.correlation.0 > 0); // Should have positive correlation
    }
} 