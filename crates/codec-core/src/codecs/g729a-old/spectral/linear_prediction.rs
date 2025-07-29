//! Linear prediction analysis using Levinson-Durbin algorithm

use crate::codecs::g729a::constants::*;
use crate::codecs::g729a::types::{Q15, Q31, LPCoefficients};
use crate::codecs::g729a::math::{FixedPointOps};
use crate::codecs::g729a::math::dsp_operations::{
    mult16_16_p15, autocorrelation_bcg729, apply_lag_window_bcg729
};
use crate::codecs::g729a::math::fixed_point::{
    div32_32_q27, div32_32_q31, mult32_32_q31, mult32_32_q23, 
    mac32_32_q31, add32, sub32, sshl32
};
use crate::codecs::g729a::tables::window_tables::{get_wlp_window, get_wlag_window};

/// Linear predictor for spectral analysis with bcg729-exact implementation
pub struct LinearPredictor {
    /// Window for LP analysis (wlp table)
    wlp_window: &'static [Q15; 240],
    /// Lag window for autocorrelation (wlag table)
    wlag_window: &'static [Q15; 13],
}

impl LinearPredictor {
    /// Create a new linear predictor with exact bcg729 windows
    pub fn new() -> Self {
        Self {
            wlp_window: get_wlp_window(),
            wlag_window: get_wlag_window(),
        }
    }
    
    /// Analyze signal and extract LP coefficients - exact bcg729 implementation
    pub fn analyze(&self, signal: &[Q15]) -> LPCoefficients {
        // bcg729: Windowing per spec 3.2.1 eq4 using MULT16_16_P15
        let windowed_signal = self.apply_wlp_window(signal);
        
        #[cfg(debug_assertions)]
        {
            let windowed_energy: i64 = windowed_signal.iter()
                .map(|&s| (s.0 as i64) * (s.0 as i64))
                .sum();
            eprintln!("bcg729 windowed signal energy: {}", windowed_energy);
            eprintln!("First 10 windowed samples: {:?}", &windowed_signal[0..10].iter().map(|x| x.0).collect::<Vec<_>>());
        }
        
        // bcg729: Autocorrelation per spec 3.2.1 eq5 with 64-bit accumulation and dynamic scaling
        let mut correlations = autocorrelation_bcg729(&windowed_signal, LP_ORDER);
        
        // bcg729: Apply lag window per spec 3.2.1 eq7 using MULT16_32_P15
        apply_lag_window_bcg729(&mut correlations, self.wlag_window);
        
        #[cfg(debug_assertions)]
        {
            eprintln!("After bcg729 lag windowing:");
            eprintln!("  R[0]: {}, R[1]: {}", correlations[0].0, correlations[1].0);
        }
        
        // bcg729: Levinson-Durbin recursion per spec 3.2.2
        let (coefficients, reflection_coeffs) = self.levinson_durbin_bcg729(&correlations);
        
        LPCoefficients {
            values: coefficients,
            reflection_coeffs,
        }
    }
    
    /// Apply bcg729-exact wlp window per ITU-T spec 3.2.1 eq4
    /// windowedSignal[i] = MULT16_16_P15(signal[i], wlp[i])
    fn apply_wlp_window(&self, signal: &[Q15]) -> Vec<Q15> {
        assert_eq!(signal.len(), WINDOW_SIZE, "Signal must be {} samples", WINDOW_SIZE);
        
        let mut windowed = Vec::with_capacity(WINDOW_SIZE);
        
        for i in 0..WINDOW_SIZE {
            // bcg729: windowedSignal[i] = MULT16_16_P15(signal[i], wlp[i])
            // signal in Q0, wlp in Q0.15, windowedSignal in Q0
            let windowed_sample = mult16_16_p15(signal[i].0, self.wlp_window[i].0);
            windowed.push(Q15(windowed_sample));
        }
        
        #[cfg(debug_assertions)]
        {
            eprintln!("bcg729 wlp window values [120..130]: {:?}", &self.wlp_window[120..130].iter().map(|x| x.0).collect::<Vec<_>>());
            eprintln!("bcg729 windowed samples [120..130]: {:?}", &windowed[120..130].iter().map(|x| x.0).collect::<Vec<_>>());
        }
        
        windowed
    }
    
    /// Public interface for Levinson-Durbin (for backward compatibility)
    pub fn levinson_durbin(&self, r: &[Q31]) -> ([Q15; LP_ORDER], [Q15; LP_ORDER]) {
        self.levinson_durbin_bcg729(r)
    }
    
    /// bcg729-exact Levinson-Durbin algorithm per ITU-T spec 3.2.2
    fn levinson_durbin_bcg729(&self, r: &[Q31]) -> ([Q15; LP_ORDER], [Q15; LP_ORDER]) {
        // Implementation based on bcg729 autoCorrelation2LP function
        let mut previous_iteration_lp_coefficients = [0i32; LP_ORDER + 1];
        let mut lp_coefficients = [0i32; LP_ORDER + 1]; // in Q4.27
        let mut reflection_coefficients = [0i32; LP_ORDER]; // in Q31
        
        let mut e = 0i32; // in Q31
        let mut sum = 0i32; // in Q27
        
        // Initialize
        lp_coefficients[0] = 134217728; // ONE_IN_Q27 = 1.0 in Q27
        lp_coefficients[1] = -div32_32_q27(r[1].0, r[0].0); // result in Q27(but<1)
        reflection_coefficients[0] = sshl32(lp_coefficients[1], 4); // k[0] is -r1/r0 in Q31
        
        // E = r0(1 - a[1]^2) in Q31
        e = mult32_32_q31(r[0].0, sub32(2147483647, mult32_32_q23(lp_coefficients[1], lp_coefficients[1]))); // ONE_IN_Q31, LPCoefficient[1] is in Q27, using a Q23 operation will result in a Q31 variable
        
        #[cfg(debug_assertions)]
        eprintln!("bcg729 Levinson-Durbin debug:");
        eprintln!("  R[0] = {}, R[1] = {}", r[0].0, r[1].0);
        eprintln!("  After iteration 1:");
        eprintln!("    a[1] = {} (Q27)", lp_coefficients[1]);
        eprintln!("    k[0] = {} (Q31)", reflection_coefficients[0]);
        eprintln!("    E = {} (Q31)", e);
        
        for i in 2..=LP_ORDER {
            // Update the previous iteration LP coefficients needed for this one
            for j in 1..i {
                previous_iteration_lp_coefficients[j] = lp_coefficients[j];
            }
            
            // sum = r[i] + ∑ a[j]*r[i-j] with j = 1..i-1 (a[0] is always 1)
            sum = 0;
            for j in 1..i {
                sum = mac32_32_q31(sum, lp_coefficients[j], r[i-j].0); // LPCoefficients in Q27, autoCorrelation in Q31 -> result in Q27 -> sum in Q27
            }
            sum = add32(sshl32(sum, 4), r[i].0); // set sum in Q31 and add r[i]
            
            // a[i] = -sum/E
            lp_coefficients[i] = -div32_32_q31(sum, e); // LPCoefficient of current iteration is in Q31 for now, it will be set to Q27 at the end of this iteration
            reflection_coefficients[i-1] = lp_coefficients[i]; // k[i-1] is needed by VAD others by RFC3389 RTP payload for Comfort Noise spectral information encoding
            
            #[cfg(debug_assertions)]
            eprintln!("  Iteration {}:", i);
            eprintln!("    sum = {} (Q31)", sum);
            eprintln!("    a[{}] = {} (Q31)", i, lp_coefficients[i]);
            eprintln!("    k[{}] = {} (Q31)", i-1, reflection_coefficients[i-1]);
            
            // iterations j = 1..i-1
            // a[j] += a[i]*a[i-j]
            for j in 1..i {
                lp_coefficients[j] = mac32_32_q31(lp_coefficients[j], lp_coefficients[i], previous_iteration_lp_coefficients[i-j]); // LPCoefficients in Q27 except for LPCoefficients[i] in Q31
            }
            
            // E *= (1-a[i]^2)
            e = mult32_32_q31(e, sub32(2147483647, mult32_32_q31(lp_coefficients[i], lp_coefficients[i]))); // all in Q31, ONE_IN_Q31
            
            // Set LPCoefficients[i] from Q31 to Q27
            lp_coefficients[i] = lp_coefficients[i] >> 4;
            
            #[cfg(debug_assertions)]
            eprintln!("    After update: a[{}] = {} (Q27), E = {}", i, lp_coefficients[i], e);
        }
        
        // Convert with rounding the LP Coefficients from Q27 to Q15, ignore first coefficient which is always 1
        let mut coeffs_q15 = [Q15::ZERO; LP_ORDER];
        let mut reflections_q15 = [Q15::ZERO; LP_ORDER];
        
        for i in 0..LP_ORDER {
            // SATURATE(PSHR(LPCoefficients[i+1], 15), MAXINT16)
            let q15_val = ((lp_coefficients[i+1] + 0x4000) >> 15).clamp(-32768, 32767) as i16;
            coeffs_q15[i] = Q15(q15_val);
            
            let refl_q15_val = (reflection_coefficients[i] >> 16).clamp(-32768, 32767) as i16;
            reflections_q15[i] = Q15(refl_q15_val);
        }
        
        #[cfg(debug_assertions)]
        {
            eprintln!("  Final LP coefficients (Q12->Q15): {:?}", coeffs_q15.iter().map(|x| x.0).collect::<Vec<_>>());
            eprintln!("  Final reflection coefficients (Q15): {:?}", reflections_q15.iter().map(|x| x.0).collect::<Vec<_>>());
        }
        
        (coeffs_q15, reflections_q15)
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
        
        let (coeffs, k) = predictor.levinson_durbin_bcg729(&r);
        
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
        
        let (coeffs, k) = predictor.levinson_durbin_bcg729(&r);
        
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
        let windowed = predictor.apply_wlp_window(&signal);
        
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
        let (coeffs, k) = predictor.levinson_durbin_bcg729(&r);
        
        // All coefficients should be zero
        for coeff in &coeffs {
            assert_eq!(coeff.0, 0);
        }
        for reflection in &k {
            assert_eq!(reflection.0, 0);
        }
    }
} 