//! Linear prediction analysis using Levinson-Durbin algorithm

use crate::codecs::g729a::constants::*;
use crate::codecs::g729a::types::{Q15, Q31, LPCoefficients};
use crate::codecs::g729a::math::{autocorrelation, FixedPointOps, apply_lag_window, mult16_16_p15};
use crate::codecs::g729a::math::fixed_point::{
    div32_32_q27, div32_32_q31, mult32_32_q31, mult32_32_q23, 
    mac32_32_q31, add32, sub32, sshl32
};
use crate::codecs::g729a::signal::{HammingWindow, LagWindow};
use crate::codecs::g729a::tables::{get_hamming_window, get_lag_window};

/// Linear predictor for spectral analysis
pub struct LinearPredictor {
    /// Window for LP analysis
    hamming_window: Vec<Q15>,
    /// Lag window for autocorrelation
    lag_window: Vec<Q15>,
}

impl LinearPredictor {
    /// Create a new linear predictor with exact ITU-T windows
    pub fn new() -> Self {
        Self {
            hamming_window: get_hamming_window(),
            lag_window: get_lag_window(),
        }
    }
    
    /// Analyze signal and extract LP coefficients - exact ITU-T implementation
    pub fn analyze(&self, signal: &[Q15]) -> LPCoefficients {
        // ITU-T: Windowing per spec 3.2.1 eq4
        let windowed_signal = self.apply_hamming_window(signal);
        
        #[cfg(debug_assertions)]
        {
            let windowed_energy: i64 = windowed_signal.iter()
                .map(|&s| (s.0 as i64) * (s.0 as i64))
                .sum();
            eprintln!("Windowed signal energy: {}", windowed_energy);
            eprintln!("First 10 windowed samples: {:?}", &windowed_signal[0..10].iter().map(|x| x.0).collect::<Vec<_>>());
        }
        
        // ITU-T: Autocorrelation per spec 3.2.1 eq5 with dynamic scaling
        let mut correlations = autocorrelation(&windowed_signal, LP_ORDER);
        
        // ITU-T: Apply lag window per spec 3.2.1 eq7
        apply_lag_window(&mut correlations, &self.lag_window);
        
        #[cfg(debug_assertions)]
        {
            eprintln!("After lag windowing:");
            eprintln!("  R[0]: {}, R[1]: {}", correlations[0].0, correlations[1].0);
        }
        
        // ITU-T: Levinson-Durbin recursion per spec 3.2.2
        let (coefficients, reflection_coeffs) = self.levinson_durbin(&correlations);
        
        LPCoefficients {
            values: coefficients,
            reflection_coeffs,
        }
    }
    
    /// Apply Hamming window per ITU-T spec 3.2.1 eq4
    fn apply_hamming_window(&self, signal: &[Q15]) -> Vec<Q15> {
        assert_eq!(signal.len(), WINDOW_SIZE, "Signal must be {} samples", WINDOW_SIZE);
        assert_eq!(self.hamming_window.len(), WINDOW_SIZE, "Window must be {} samples", WINDOW_SIZE);
        
        let mut windowed = Vec::with_capacity(WINDOW_SIZE);
        
        for i in 0..WINDOW_SIZE {
            // ITU-T: windowedSignal[i] = MULT16_16_P15(signal[i], wlp[i])
            let windowed_sample = mult16_16_p15(signal[i].0, self.hamming_window[i].0);
            windowed.push(Q15(windowed_sample));
        }
        
        #[cfg(debug_assertions)]
        {
            eprintln!("Window values [120..130]: {:?}", &self.hamming_window[120..130].iter().map(|x| x.0).collect::<Vec<_>>());
            eprintln!("Windowed samples [120..130]: {:?}", &windowed[120..130].iter().map(|x| x.0).collect::<Vec<_>>());
        }
        
        windowed
    }
    
    /// Levinson-Durbin algorithm - exact ITU-T implementation
    pub fn levinson_durbin(&self, r: &[Q31]) -> ([Q15; LP_ORDER], [Q15; LP_ORDER]) {
        // ITU-T: Check for zero energy
        if r[0].0 <= 0 {
            return ([Q15::ZERO; LP_ORDER], [Q15::ZERO; LP_ORDER]);
        }
        
        // ITU-T: Initialize arrays exactly as in autoCorrelation2LP()
        let mut previous_a = [0i32; LP_ORDER + 1]; // for previous iteration
        let mut lp_coeffs = [0i32; LP_ORDER + 1];  // in Q27 (Q4.27)
        
        // ITU-T constants
        const ONE_IN_Q27: i32 = 1 << 27;
        const ONE_IN_Q31: i32 = i32::MAX;  // 0x7FFFFFFF
        
        #[cfg(debug_assertions)]
        {
            eprintln!("ITU-T Levinson-Durbin debug:");
            eprintln!("  R[0] = {}, R[1] = {}", r[0].0, r[1].0);
        }
        
        // ITU-T: init - iteration i=1 setup
        lp_coeffs[0] = ONE_IN_Q27;  // a[0] = 1 in Q27
        
        // ITU-T: a[1] = -r1/r0 (result in Q27)
        lp_coeffs[1] = -div32_32_q27(r[1].0, r[0].0);
        let mut reflection_coeffs = [0i32; LP_ORDER];
        reflection_coeffs[0] = sshl32(lp_coeffs[1], 4); // k[0] = a[1] in Q31
        
        // ITU-T: E = r0(1 - a[1]^2) in Q31
        // LPCoefficient[1] is in Q27, using a Q23 operation will result in a Q31 variable
        let mut e = mult32_32_q31(
            r[0].0, 
            sub32(ONE_IN_Q31, mult32_32_q23(lp_coeffs[1], lp_coeffs[1]))
        );
        
        #[cfg(debug_assertions)]
        {
            eprintln!("  After iteration 1:");
            eprintln!("    a[1] = {} (Q27)", lp_coeffs[1]);
            eprintln!("    k[0] = {} (Q31)", reflection_coeffs[0]);
            eprintln!("    E = {} (Q31)", e);
        }
        
        // ITU-T: Main iterations i = 2..10
        for i in 2..=LP_ORDER {
            // ITU-T: update the previousIterationLPCoefficients needed for this one
            for j in 1..i {
                previous_a[j] = lp_coeffs[j];
            }
            
            // ITU-T: sum = r[i] + ∑ a[j]*r[i-j] with j = 1..i-1 (a[0] is always 1)
            let mut sum = 0i32; // in Q27
            for j in 1..i {
                // LPCoefficients in Q27, autoCorrelation in Q31 -> result in Q27 -> sum in Q27
                sum = mac32_32_q31(sum, lp_coeffs[j], r[i - j].0);
            }
            // set sum in Q31 and add r[i]
            sum = add32(sshl32(sum, 4), r[i].0);
            
            // ITU-T: a[i] = -sum/E
            // LPCoefficient of current iteration is in Q31 for now, it will be set to Q27 at the end of this iteration
            lp_coeffs[i] = -div32_32_q31(sum, e);
            reflection_coeffs[i - 1] = lp_coeffs[i]; // k[i-1] in Q31
            
            #[cfg(debug_assertions)]
            {
                eprintln!("  Iteration {}:", i);
                eprintln!("    sum = {} (Q31)", sum);
                eprintln!("    a[{}] = {} (Q31)", i, lp_coeffs[i]);
                eprintln!("    k[{}] = {} (Q31)", i-1, reflection_coeffs[i-1]);
            }
            
            // ITU-T: iterations j = 1..i-1
            //        a[j] += a[i]*a[i-j]
            for j in 1..i {
                // LPCoefficients in Q27 except for LPCoefficients[i] in Q31
                lp_coeffs[j] = mac32_32_q31(lp_coeffs[j], lp_coeffs[i], previous_a[i - j]);
            }
            
            // ITU-T: E *= (1 - a[i]^2) - all in Q31
            e = mult32_32_q31(e, sub32(ONE_IN_Q31, mult32_32_q31(lp_coeffs[i], lp_coeffs[i])));
            
            // ITU-T: set LPCoefficients[i] from Q31 to Q27
            lp_coeffs[i] = lp_coeffs[i] >> 4;
            
            #[cfg(debug_assertions)]
            {
                eprintln!("    After update: a[{}] = {} (Q27), E = {}", i, lp_coeffs[i], e);
            }
            
            // Check for instability
            if e <= 0 {
                eprintln!("Warning: Levinson-Durbin became unstable at iteration {}", i);
                break;
            }
        }
        
        // ITU-T: convert with rounding the LP Coefficients from Q27 to Q12, ignore first coefficient which is always 1
        let mut lp_output = [Q15::ZERO; LP_ORDER];
        let mut reflection_output = [Q15::ZERO; LP_ORDER];
        
        for i in 0..LP_ORDER {
            // ITU-T: PSHR(LPCoefficients[i+1], 15) with saturation - Q27 to Q12 conversion
            let q12_val = ((lp_coeffs[i + 1] + 0x4000) >> 15).clamp(-32768, 32767) as i16;
            lp_output[i] = Q15(q12_val << 3); // Convert Q12 to Q15 for our internal format
            
            // Convert reflection coefficients from Q31 to Q15
            reflection_output[i] = Q15((reflection_coeffs[i] >> 16).clamp(-32768, 32767) as i16);
        }
        
        #[cfg(debug_assertions)]
        {
            eprintln!("  Final LP coefficients (Q12->Q15): {:?}", lp_output.iter().map(|x| x.0).collect::<Vec<_>>());
            eprintln!("  Final reflection coefficients (Q15): {:?}", reflection_output.iter().map(|x| x.0).collect::<Vec<_>>());
        }
        
        (lp_output, reflection_output)
    }
    
    /// Apply bandwidth expansion to LP coefficients
    pub fn expand_bandwidth(coeffs: &[Q15], gamma: Q15) -> Vec<Q15> {
        let mut expanded = Vec::with_capacity(coeffs.len());
        let mut gamma_power = Q15::ONE;
        
        for &coeff in coeffs {
            expanded.push(coeff.saturating_mul(gamma_power));
            gamma_power = gamma_power.saturating_mul(gamma);
        }
        
        expanded
    }
}

impl Default for LinearPredictor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn test_linear_predictor_creation() {
        let predictor = LinearPredictor::new();
        // Just ensure it creates without panic
        assert!(true);
    }

    #[test]
    fn test_levinson_durbin_impulse() {
        let predictor = LinearPredictor::new();
        
        // Test with impulse autocorrelation (white noise)
        let r = vec![
            Q31::from_f32(0.5),
            Q31::ZERO,
            Q31::ZERO,
            Q31::ZERO,
        ];
        
        let (coeffs, k) = predictor.levinson_durbin(&r);
        
        // For white noise, all LP coefficients should be near zero
        for coeff in &coeffs[0..3] {
            assert!(coeff.0.abs() < 1000);
        }
        
        // All reflection coefficients should be near zero
        for reflection in &k[0..3] {
            assert!(reflection.0.abs() < 1000);
        }
    }

    #[test]
    fn test_levinson_durbin_correlated() {
        let predictor = LinearPredictor::new();
        
        // Test with correlated signal (AR process)
        let r = vec![
            Q31::from_f32(0.5),
            Q31::from_f32(0.4),
            Q31::from_f32(0.3),
            Q31::from_f32(0.2),
            Q31::from_f32(0.1),
        ];
        
        let (coeffs, k) = predictor.levinson_durbin(&r);
        
        // First coefficient should be negative (due to positive correlation)
        assert!(coeffs[0].0 < 0);
        
        // Reflection coefficients should be stable (|k| < 1)
        for reflection in &k[0..4] {
            assert!(reflection.0.abs() < Q15_ONE);
        }
    }

    #[test]
    fn test_bandwidth_expansion() {
        let coeffs = vec![
            Q15::from_f32(0.8),
            Q15::from_f32(0.4),
            Q15::from_f32(0.2),
        ];
        
        let gamma = Q15::from_f32(0.9);
        let expanded = LinearPredictor::expand_bandwidth(&coeffs, gamma);
        
        assert_eq!(expanded.len(), coeffs.len());
        
        // Check that magnitudes decrease
        assert!(expanded[0].0.abs() <= coeffs[0].0.abs());
        assert!(expanded[1].0.abs() <= coeffs[1].0.abs());
        assert!(expanded[2].0.abs() <= coeffs[2].0.abs());
        
        // Check approximate values
        assert!((expanded[0].to_f32() - 0.8).abs() < 0.01);
        assert!((expanded[1].to_f32() - 0.36).abs() < 0.02); // 0.4 * 0.9
        assert!((expanded[2].to_f32() - 0.162).abs() < 0.02); // 0.2 * 0.9^2
    }

    #[test]
    fn test_full_analysis() {
        let predictor = LinearPredictor::new();
        
        // Create a simple test signal
        let mut signal = vec![Q15::ZERO; WINDOW_SIZE];
        for i in 0..WINDOW_SIZE {
            let value = ((i as f32 / 10.0).sin() * 0.5) * (1.0 - i as f32 / WINDOW_SIZE as f32);
            signal[i] = Q15::from_f32(value);
        }
        
        // Apply window
        let windowed = predictor.apply_hamming_window(&signal);
        
        // Analyze
        let lp_coeffs = predictor.analyze(&windowed);
        
        // Check that we got LP_ORDER coefficients
        assert_eq!(lp_coeffs.values.len(), LP_ORDER);
        assert_eq!(lp_coeffs.reflection_coeffs.len(), LP_ORDER);
        
        // All reflection coefficients should be stable
        for k in &lp_coeffs.reflection_coeffs {
            assert!(k.0.abs() < Q15_ONE);
        }
    }

    #[test]
    fn test_zero_input() {
        let predictor = LinearPredictor::new();
        
        // Test with zero input
        let r = vec![Q31::ZERO; LP_ORDER + 1];
        let (coeffs, k) = predictor.levinson_durbin(&r);
        
        // All coefficients should be zero
        for coeff in &coeffs {
            assert_eq!(coeff.0, 0);
        }
        for reflection in &k {
            assert_eq!(reflection.0, 0);
        }
    }
} 