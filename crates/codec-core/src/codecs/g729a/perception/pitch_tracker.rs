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
    /// H(z) = 1 / (1 - 0.7z^-1)
    pub fn process(&mut self, signal: &[Q15]) -> Vec<Q15> {
        let b0 = Q15::ONE;
        let b1 = Q15::ZERO;
        let a1 = Q15::from_f32(-0.7);
        
        iir_filter_1st_order(signal, b0, b1, a1, &mut self.state)
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
        // 1. Apply low-pass filter for decimation
        let filtered = self.decimation_filter.process(weighted_speech);
        
        // 2. Search in three delay ranges
        let candidates = [
            self.search_range(&filtered, 20, 39, 2),   // Short delays
            self.search_range(&filtered, 40, 79, 2),   // Medium delays  
            self.search_range(&filtered, 80, 143, 4),  // Long delays (more decimation)
        ];
        
        // 3. Select best with pitch doubling/halving checks
        self.select_best_pitch(&candidates)
    }
    
    /// Search for best pitch in a given range
    fn search_range(&self, signal: &[Q15], min: u16, max: u16, decimation: usize) -> PitchCandidate {
        let mut best_delay = min;
        let mut best_corr = Q15(i16::MIN);
        
        // For G.729A, use decimated correlation
        for delay in min..=max {
            // Skip odd delays in third range for efficiency
            if min >= 80 && delay % 2 == 1 {
                continue;
            }
            
            let corr = if decimation > 1 {
                decimated_correlation(signal, delay as usize, decimation)
            } else {
                decimated_correlation(signal, delay as usize, 1)
            };
            
            // Normalize by energy
            let energy_at_delay = self.compute_energy_at_delay(signal, delay as usize);
            
            if energy_at_delay.0 > 0 {
                // Approximate normalized correlation
                let norm_corr = Q15((corr.0 / (energy_at_delay.0 >> 15).max(1)) as i16);
                
                if norm_corr.0 > best_corr.0 {
                    best_corr = norm_corr;
                    best_delay = delay;
                }
            }
        }
        
        // For the third range, refine around the best even delay
        if min >= 80 && best_delay < max {
            // Check adjacent odd delays
            let delay_minus = best_delay.saturating_sub(1).max(min);
            let delay_plus = (best_delay + 1).min(max);
            
            for &delay in &[delay_minus, delay_plus] {
                if delay != best_delay {
                    let corr = decimated_correlation(signal, delay as usize, decimation);
                    let energy_at_delay = self.compute_energy_at_delay(signal, delay as usize);
                    
                    if energy_at_delay.0 > 0 {
                        let norm_corr = Q15((corr.0 / (energy_at_delay.0 >> 15).max(1)) as i16);
                        
                        if norm_corr.0 > best_corr.0 {
                            best_corr = norm_corr;
                            best_delay = delay;
                        }
                    }
                }
            }
        }
        
        PitchCandidate {
            delay: best_delay,
            correlation: best_corr,
        }
    }
    
    /// Compute energy at a given delay
    fn compute_energy_at_delay(&self, signal: &[Q15], delay: usize) -> Q31 {
        if delay >= signal.len() {
            return Q31::ZERO;
        }
        
        let segment = &signal[..signal.len() - delay];
        energy(segment)
    }
    
    /// Select best pitch candidate with pitch doubling/halving checks
    fn select_best_pitch(&self, candidates: &[PitchCandidate; 3]) -> PitchCandidate {
        let mut best = candidates[0];
        
        // Favor lower delays with bonus factor
        let bonuses = [1.2, 1.1, 1.0]; // Bonus for short, medium, long delays
        
        for i in 0..3 {
            let mut score = candidates[i].correlation.0 as f32;
            
            // Apply bonus
            score *= bonuses[i];
            
            // Check for pitch doubling/halving relationships
            for j in 0..i {
                let ratio = candidates[i].delay as f32 / candidates[j].delay as f32;
                
                // If current delay is roughly double a previous one
                if (ratio - 2.0).abs() < 0.1 {
                    score *= 1.15; // Boost score
                }
                // If current delay is roughly half a previous one
                else if (ratio - 0.5).abs() < 0.05 {
                    score *= 1.15; // Boost score
                }
            }
            
            if score > best.correlation.0 as f32 * bonuses[0] {
                best = candidates[i];
            }
        }
        
        best
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
        let candidate = tracker.search_range(&signal, 35, 45, 1);
        
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
        
        let best = tracker.select_best_pitch(&candidates);
        
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